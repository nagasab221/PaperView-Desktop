//! The book reader: an open book laid out as pages, two facing pages by
//! default (one at a time on request), turned with the keys, the wheel or a
//! click on either side.
//!
//! Text books are typeset to the window: each block becomes an egui text
//! layout at the column width, and its rows are dealt onto pages. Laying out a
//! whole novel at once would stall the window, so pagination runs a few
//! milliseconds per frame until done; the page you're on is laid out first.
//! The reading position is kept as (block, character), so it survives a new
//! window size or font size. Comics show one image per page.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, FontFamily, FontId, Pos2, Rect, Stroke, Vec2};

use crate::ebook::{self, Block, Book, Content, Run};
use crate::ui::theme::{self, Palette};

/// Where the reader is: a block and a character within it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Mark {
    pub block: usize,
    pub chr: usize,
}

#[derive(Clone, Copy, PartialEq)]
struct LayoutKey {
    col: (i32, i32),
    font: i32,
    spread: bool,
}

enum Item {
    /// Rows `rows` of block `block`'s layout, drawn with their top at `y`.
    Text {
        block: usize,
        rows: std::ops::Range<usize>,
        y: f32,
    },
    Image {
        id: usize,
        y: f32,
        size: Vec2,
    },
    Rule {
        y: f32,
    },
}

struct Page {
    items: Vec<Item>,
    start: Mark,
}

enum Loaded {
    Loading(Receiver<Result<Book, String>>),
    Ready(Arc<Book>),
    Failed(String),
}

/// Images decoded off the UI thread.
type Decoded = (usize, Option<egui::ColorImage>);

pub(super) struct BookView {
    pub path: PathBuf,
    pub title: String,
    loaded: Loaded,
    pub mark: Mark,
    pub spread: bool,
    pub font: f32,
    /// Laid-out pages for `key`; more are added each frame until done.
    pages: Vec<Page>,
    key: Option<LayoutKey>,
    /// Pagination progress: the next block to place, and the open page.
    next_block: usize,
    open_page: Option<Page>,
    used: f32,
    done: bool,
    /// The page shown (the left one in a spread).
    pub cur: usize,
    /// A size change waits until the window stops moving.
    pending: Option<(LayoutKey, Instant)>,
    image_sizes: HashMap<usize, Vec2>,
    textures: HashMap<usize, Option<egui::TextureHandle>>,
    decode_tx: Sender<Decoded>,
    decode_rx: Receiver<Decoded>,
    wheel: f32,
    last_turn: Option<Instant>,
}

/// Font size range, logical points.
pub(super) const FONT_MIN: f32 = 13.0;
pub(super) const FONT_MAX: f32 = 30.0;

impl BookView {
    pub fn open(path: &Path, mark: Mark, spread: bool, font: f32) -> BookView {
        let (tx, rx) = channel();
        let p = path.to_path_buf();
        std::thread::spawn(move || {
            let _ = tx.send(ebook::open(&p));
        });
        let (decode_tx, decode_rx) = channel();
        BookView {
            path: path.to_path_buf(),
            title: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            loaded: Loaded::Loading(rx),
            mark,
            spread,
            font: font.clamp(FONT_MIN, FONT_MAX),
            pages: Vec::new(),
            key: None,
            next_block: 0,
            open_page: None,
            used: 0.0,
            done: false,
            cur: 0,
            pending: None,
            image_sizes: HashMap::new(),
            textures: HashMap::new(),
            decode_tx,
            decode_rx,
            wheel: 0.0,
            last_turn: None,
        }
    }

    pub fn book(&self) -> Option<&Arc<Book>> {
        match &self.loaded {
            Loaded::Ready(b) => Some(b),
            _ => None,
        }
    }

    /// Fraction of the book before the current page.
    pub fn progress(&self) -> f32 {
        let Some(book) = self.book() else { return 0.0 };
        match &book.content {
            Content::Comic { pages } => self.cur as f32 / pages.len().max(1) as f32,
            Content::Text { blocks, .. } => self.mark.block as f32 / blocks.len().max(1) as f32,
        }
    }

    fn step(&self) -> usize {
        if self.spread {
            2
        } else {
            1
        }
    }

    fn page_count(&self) -> usize {
        match self.book().map(|b| &b.content) {
            Some(Content::Comic { pages }) => pages.len(),
            _ => self.pages.len(),
        }
    }

    /// "12–13 of 341", "12 of 341…" while pages are still being laid out.
    pub fn page_label(&self) -> String {
        let n = self.page_count();
        if n == 0 {
            return String::new();
        }
        let a = self.cur + 1;
        let b = (self.cur + self.step()).min(n);
        let more = if self.done || self.book().is_some_and(|b| b.is_comic()) {
            ""
        } else {
            "+"
        };
        if b > a {
            format!("{a}–{b} of {n}{more}")
        } else {
            format!("{a} of {n}{more}")
        }
    }

    /// The chapter the current page is in.
    pub fn chapter_title(&self) -> Option<String> {
        let book = self.book()?;
        book.chapters
            .iter()
            .rev()
            .find(|c| c.block <= self.mark.block)
            .map(|c| one_line(&c.title))
    }

    pub fn next(&mut self) {
        let n = self.page_count();
        if self.cur + self.step() < n {
            self.cur += self.step();
        }
        self.sync_mark();
    }

    pub fn prev(&mut self) {
        self.cur = self.cur.saturating_sub(self.step());
        self.sync_mark();
    }

    pub fn first(&mut self) {
        self.cur = 0;
        self.sync_mark();
    }

    pub fn last(&mut self) {
        let n = self.page_count();
        if n > 0 {
            self.cur = (n - 1) / self.step() * self.step();
        }
        self.sync_mark();
    }

    /// Jumps to a chapter (a block in the text).
    pub fn goto_block(&mut self, block: usize) {
        self.mark = Mark { block, chr: 0 };
        self.seek_mark();
    }

    pub fn set_spread(&mut self, spread: bool) {
        if self.spread != spread {
            self.spread = spread;
            self.key = None; // column width changes: lay out again
        }
    }

    pub fn set_font(&mut self, font: f32) {
        let font = font.clamp(FONT_MIN, FONT_MAX);
        if (font - self.font).abs() > 0.01 {
            self.font = font;
            self.key = None;
        }
    }

    fn sync_mark(&mut self) {
        if let Some(page) = self.pages.get(self.cur) {
            self.mark = page.start;
        } else if self.book().is_some_and(|b| b.is_comic()) {
            self.mark = Mark {
                block: self.cur,
                chr: 0,
            };
        }
    }

    /// Finds the page holding `mark`, laying out more pages if need be.
    fn seek_mark(&mut self) {
        if self.book().is_some_and(|b| b.is_comic()) {
            self.cur = self.mark.block.min(self.page_count().saturating_sub(1));
            self.cur -= self.cur % self.step();
            return;
        }
        let target = self.mark;
        let found = self.pages.iter().rposition(|p| {
            p.start.block < target.block
                || (p.start.block == target.block && p.start.chr <= target.chr)
        });
        let page = found.unwrap_or(0);
        self.cur = page - page % self.step();
    }

    // ── Layout ───────────────────────────────────────────────────────────────

    fn reset_layout(&mut self, key: LayoutKey) {
        self.key = Some(key);
        self.pages.clear();
        self.open_page = None;
        self.next_block = 0;
        self.used = 0.0;
        self.done = false;
    }

    fn job(&self, block: &Block, width: f32, p: &Palette) -> Option<egui::text::LayoutJob> {
        let (runs, size, family_base, centred, extra) = match block {
            Block::Para { runs } => (runs, self.font, "serif", false, 1.55),
            Block::Heading { level, runs } => {
                let scale = match level {
                    1 => 1.65,
                    2 => 1.4,
                    3 => 1.2,
                    _ => 1.08,
                };
                (runs, self.font * scale, "serif", true, 1.3)
            }
            _ => return None,
        };
        let _ = family_base;
        let mut job = egui::text::LayoutJob::default();
        for Run { text, style } in runs {
            let family = match (
                style.bold || matches!(block, Block::Heading { .. }),
                style.italic,
            ) {
                (true, true) => FontFamily::Name("serif-bolditalic".into()),
                (true, false) => FontFamily::Name("serif-bold".into()),
                (false, true) => FontFamily::Name("serif-italic".into()),
                (false, false) => theme::serif(),
            };
            job.append(
                text,
                0.0,
                egui::TextFormat {
                    font_id: FontId::new(size, family),
                    color: p.ink,
                    line_height: Some(size * extra),
                    ..Default::default()
                },
            );
        }
        job.wrap.max_width = width;
        // Left-aligned: egui's justification spaces out letters as well as
        // words, which reads worse than a ragged right edge.
        job.halign = if centred { Align::Center } else { Align::LEFT };
        Some(job)
    }

    fn image_size(&mut self, id: usize, col: Vec2) -> Vec2 {
        let natural = match self.image_sizes.get(&id) {
            Some(n) => *n,
            None => {
                let book = self.book().cloned();
                let n = match book.as_deref().map(|b| &b.content) {
                    Some(Content::Text { images, .. }) => images
                        .get(id)
                        .and_then(|bytes| {
                            image::ImageReader::new(std::io::Cursor::new(bytes))
                                .with_guessed_format()
                                .ok()?
                                .into_dimensions()
                                .ok()
                        })
                        .map(|(w, h)| Vec2::new(w as f32, h as f32))
                        .unwrap_or(Vec2::ZERO),
                    _ => Vec2::ZERO,
                };
                self.image_sizes.insert(id, n);
                n
            }
        };
        if natural.x <= 0.0 || natural.y <= 0.0 {
            return Vec2::ZERO;
        }
        // Fit the column, no taller than most of a page, never enlarged much.
        let scale = (col.x / natural.x).min(col.y * 0.85 / natural.y).min(2.0);
        natural * scale
    }

    /// Lays out more pages, for up to `budget`, or until `until` is placed.
    fn paginate(
        &mut self,
        ctx: &egui::Context,
        col: Vec2,
        p: &Palette,
        budget: Duration,
        until: Option<usize>,
    ) {
        let Some(book) = self.book().cloned() else {
            return;
        };
        let Content::Text { blocks, .. } = &book.content else {
            self.done = true;
            return;
        };
        let started = Instant::now();
        let gap = self.font * 0.55;
        while self.next_block < blocks.len() {
            let over_budget = started.elapsed() > budget;
            let reached = until.is_none_or(|u| self.next_block > u);
            if over_budget && reached {
                return;
            }
            let i = self.next_block;
            self.next_block += 1;
            let page = self.open_page.get_or_insert_with(|| Page {
                items: Vec::new(),
                start: Mark { block: i, chr: 0 },
            });
            let _ = page;
            match &blocks[i] {
                Block::PageBreak => {
                    if self.open_page.as_ref().is_some_and(|p| !p.items.is_empty()) {
                        self.close_page(Mark {
                            block: i + 1,
                            chr: 0,
                        });
                    }
                }
                Block::Rule => {
                    if self.used + gap * 2.0 > col.y {
                        self.close_page(Mark { block: i, chr: 0 });
                    }
                    let y = self.used + gap;
                    self.open_page
                        .get_or_insert_with(|| Page {
                            items: Vec::new(),
                            start: Mark { block: i, chr: 0 },
                        })
                        .items
                        .push(Item::Rule { y });
                    self.used += gap * 2.0;
                }
                Block::Image { id } => {
                    let size = self.image_size(*id, col);
                    if size == Vec2::ZERO {
                        continue;
                    }
                    if self.used > 0.0 && self.used + size.y > col.y {
                        self.close_page(Mark { block: i, chr: 0 });
                    }
                    let y = self.used;
                    self.open_page
                        .get_or_insert_with(|| Page {
                            items: Vec::new(),
                            start: Mark { block: i, chr: 0 },
                        })
                        .items
                        .push(Item::Image { id: *id, y, size });
                    self.used += size.y + gap;
                }
                block @ (Block::Para { .. } | Block::Heading { .. }) => {
                    let Some(job) = self.job(block, col.x, p) else {
                        continue;
                    };
                    let galley = ctx.fonts_mut(|f| f.layout_job(job));
                    let heading = matches!(block, Block::Heading { .. });
                    let before = if heading && self.used > 0.0 {
                        self.font * 1.2
                    } else {
                        0.0
                    };
                    // Keep a heading with at least two lines of what follows.
                    if heading
                        && self.used > 0.0
                        && self.used + before + galley.size().y + self.font * 3.2 > col.y
                    {
                        self.close_page(Mark { block: i, chr: 0 });
                    } else {
                        self.used += before;
                    }
                    let mut chr = 0usize;
                    let mut run_start: Option<(usize, f32, f32)> = None; // (row, y on page, row top in galley)
                    for (r, row) in galley.rows.iter().enumerate() {
                        let h = row.rect().height();
                        if self.used + h > col.y && self.used > 0.0 {
                            if let Some((start, y, _)) = run_start.take() {
                                self.push_text(i, start..r, y);
                            }
                            self.close_page(Mark { block: i, chr });
                        }
                        if run_start.is_none() {
                            run_start = Some((r, self.used, row.min_y()));
                            if self.open_page.is_none() {
                                self.open_page = Some(Page {
                                    items: Vec::new(),
                                    start: Mark { block: i, chr },
                                });
                            }
                        }
                        self.used += h;
                        chr += row.char_count_including_newline().0;
                    }
                    if let Some((start, y, _)) = run_start {
                        self.push_text(i, start..galley.rows.len(), y);
                    }
                    self.used += gap;
                }
            }
        }
        // All placed.
        if self.open_page.as_ref().is_some_and(|p| !p.items.is_empty()) {
            let page = self.open_page.take().unwrap();
            self.pages.push(page);
        }
        self.done = true;
    }

    fn push_text(&mut self, block: usize, rows: std::ops::Range<usize>, y: f32) {
        if rows.is_empty() {
            return;
        }
        let page = self.open_page.get_or_insert_with(|| Page {
            items: Vec::new(),
            start: Mark { block, chr: 0 },
        });
        page.items.push(Item::Text { block, rows, y });
    }

    fn close_page(&mut self, next_start: Mark) {
        if let Some(page) = self.open_page.take() {
            if !page.items.is_empty() {
                self.pages.push(page);
            }
        }
        self.open_page = Some(Page {
            items: Vec::new(),
            start: next_start,
        });
        self.used = 0.0;
    }

    // ── Drawing ──────────────────────────────────────────────────────────────

    /// Draws the book into `ui` (the whole central area). Returns true when
    /// the position changed (worth saving).
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette) -> bool {
        let ctx = ui.ctx().clone();
        let before = (self.cur, self.mark);

        // Finish loading.
        if let Loaded::Loading(rx) = &self.loaded {
            match rx.try_recv() {
                Ok(Ok(book)) => {
                    if !book.title.trim().is_empty() {
                        self.title = book.title.clone();
                    }
                    self.loaded = Loaded::Ready(Arc::new(book));
                    self.key = None;
                    if self.book().is_some_and(|b| b.is_comic()) {
                        self.seek_mark();
                    }
                }
                Ok(Err(e)) => self.loaded = Loaded::Failed(e),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => self.loaded = Loaded::Failed("The book couldn't be read".into()),
            }
        }
        let area = ui.max_rect();
        ui.painter().rect_filled(area, 0, p.viewport);
        match &self.loaded {
            Loaded::Loading(_) => {
                centred_note(ui, area, p, "Opening the book…", true);
                return false;
            }
            Loaded::Failed(msg) => {
                let msg = msg.clone();
                centred_note(ui, area, p, &msg, false);
                return false;
            }
            Loaded::Ready(_) => {}
        }

        // Page geometry: one or two paper pages centred in the area.
        let margin = Vec2::new(46.0, 54.0);
        let gutter = 18.0;
        let max_page_w = if self.spread { 600.0 } else { 700.0 };
        let avail = area.shrink2(Vec2::new(24.0, 18.0));
        let page_h = avail.height();
        let page_w = if self.spread {
            ((avail.width() - gutter) / 2.0)
                .min(max_page_w)
                .min(page_h * 0.78)
        } else {
            avail.width().min(max_page_w).min(page_h * 0.85)
        };
        let col = Vec2::new(
            (page_w - margin.x * 2.0).max(120.0),
            (page_h - margin.y * 2.0).max(120.0),
        );
        let key = LayoutKey {
            col: (col.x.round() as i32, col.y.round() as i32),
            font: (self.font * 10.0).round() as i32,
            spread: self.spread,
        };

        // Re-lay out after a change, but let a window resize settle first.
        let is_comic = self.book().is_some_and(|b| b.is_comic());
        if !is_comic && self.key != Some(key) {
            let settle = match self.pending {
                Some((k, at)) if k == key => {
                    at.elapsed() > Duration::from_millis(140) || self.key.is_none()
                }
                _ => {
                    self.pending = Some((key, Instant::now()));
                    self.key.is_none()
                }
            };
            if settle {
                let mark = self.mark;
                self.reset_layout(key);
                self.pending = None;
                // Lay out as far as the reading position before showing it.
                self.paginate(&ctx, col, p, Duration::from_millis(4), Some(mark.block));
                self.mark = mark;
                self.seek_mark();
            } else {
                ctx.request_repaint_after(Duration::from_millis(150));
            }
        }
        if !is_comic && !self.done {
            self.paginate(&ctx, col, p, Duration::from_millis(4), None);
            ctx.request_repaint();
        }

        // Images decoded in the background.
        while let Ok((id, image)) = self.decode_rx.try_recv() {
            let tex = image.map(|img| {
                ctx.load_texture(format!("book-img-{id}"), img, egui::TextureOptions::LINEAR)
            });
            self.textures.insert(id, tex);
        }

        let n_pages = if self.spread { 2 } else { 1 };
        let total_w = page_w * n_pages as f32 + gutter * (n_pages - 1) as f32;
        let left = area.center().x - total_w / 2.0;
        let top = area.center().y - page_h / 2.0;

        for k in 0..n_pages {
            let index = self.cur + k;
            let rect = Rect::from_min_size(
                Pos2::new(left + k as f32 * (page_w + gutter), top),
                Vec2::new(page_w, page_h),
            );
            if index >= self.page_count() {
                continue;
            }
            ui.painter().rect(
                rect,
                3,
                p.surface,
                Stroke::new(1.0, p.hairline),
                egui::StrokeKind::Outside,
            );
            if is_comic {
                self.draw_comic_page(ui, index, rect.shrink(10.0), p);
            } else {
                self.draw_text_page(ui, &ctx, index, rect, margin, col, p);
            }
        }

        // Clicking the left or right part of the spread turns the page.
        let resp = ui.interact(area, ui.id().with("book-turn"), egui::Sense::click());
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let third = area.width() / 3.0;
                if pos.x < area.left() + third {
                    self.prev();
                } else if pos.x > area.right() - third {
                    self.next();
                }
            }
        }
        // The wheel turns pages too, one notch at a time.
        if resp.hovered() {
            self.wheel += ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
            let cooled = self
                .last_turn
                .is_none_or(|t| t.elapsed() > Duration::from_millis(160));
            if self.wheel.abs() > 40.0 && cooled {
                if self.wheel < 0.0 {
                    self.next();
                } else {
                    self.prev();
                }
                self.wheel = 0.0;
                self.last_turn = Some(Instant::now());
            }
        }
        (self.cur, self.mark) != before
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text_page(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        index: usize,
        rect: Rect,
        margin: Vec2,
        col: Vec2,
        p: &Palette,
    ) {
        let Some(book) = self.book().cloned() else {
            return;
        };
        let Content::Text { blocks, images } = &book.content else {
            return;
        };
        let origin = rect.min + margin;
        let painter = ui.painter_at(rect);

        // Running head: the book on the left page, the chapter on the right.
        let head = if index.is_multiple_of(2) || !self.spread {
            self.title.clone()
        } else {
            self.chapter_title().unwrap_or_else(|| self.title.clone())
        };
        painter.text(
            Pos2::new(rect.center().x, rect.top() + margin.y * 0.45),
            egui::Align2::CENTER_CENTER,
            truncate(&head, 60),
            FontId::new(11.0, theme::medium()),
            p.muted,
        );
        painter.text(
            Pos2::new(rect.center().x, rect.bottom() - margin.y * 0.45),
            egui::Align2::CENTER_CENTER,
            (index + 1).to_string(),
            FontId::new(11.0, FontFamily::Proportional),
            p.muted,
        );

        let Some(page) = self.pages.get(index) else {
            return;
        };
        // Collect what to draw first (the layout borrows self immutably).
        let mut texts = Vec::new();
        let mut pics = Vec::new();
        let mut rules = Vec::new();
        for item in &page.items {
            match item {
                Item::Text { block, rows, y } => texts.push((*block, rows.clone(), *y)),
                Item::Image { id, y, size } => pics.push((*id, *y, *size)),
                Item::Rule { y } => rules.push(*y),
            }
        }
        for (block, rows, y) in texts {
            let Some(job) = self.job(&blocks[block], col.x, p) else {
                continue;
            };
            let galley = ctx.fonts_mut(|f| f.layout_job(job));
            let (Some(first), Some(last)) =
                (galley.rows.get(rows.start), galley.rows.get(rows.end - 1))
            else {
                continue;
            };
            let clip = Rect::from_min_max(
                Pos2::new(rect.left(), origin.y + y - 1.0),
                Pos2::new(
                    rect.right(),
                    origin.y + y + (last.max_y() - first.min_y()) + 1.0,
                ),
            );
            // A centred layout is centred on its origin: start it mid-column.
            let x = if matches!(blocks[block], Block::Heading { .. }) {
                origin.x + col.x / 2.0
            } else {
                origin.x
            };
            let pos = Pos2::new(x, origin.y + y - first.min_y());
            ui.painter()
                .with_clip_rect(clip.intersect(rect))
                .galley(pos, galley, p.ink);
        }
        for y in rules {
            let yy = origin.y + y;
            painter.hline(
                origin.x + col.x * 0.35..=origin.x + col.x * 0.65,
                yy,
                Stroke::new(1.0, p.hairline),
            );
        }
        for (id, y, size) in pics {
            let r = Rect::from_min_size(
                Pos2::new(origin.x + (col.x - size.x) / 2.0, origin.y + y),
                size,
            );
            match self.textures.get(&id) {
                Some(Some(tex)) => {
                    painter.image(
                        tex.id(),
                        r,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                Some(None) => {}
                None => {
                    painter.rect_filled(r, 2, p.hover());
                    self.textures.insert(id, None);
                    if let Some(bytes) = images.get(id).cloned() {
                        let tx = self.decode_tx.clone();
                        let ctx = ctx.clone();
                        let max_w = (size.x * 2.0) as u32;
                        std::thread::spawn(move || {
                            let _ = tx.send((id, decode_image(&bytes, max_w)));
                            ctx.request_repaint();
                        });
                    }
                }
            }
        }
    }

    fn draw_comic_page(&mut self, ui: &mut egui::Ui, index: usize, rect: Rect, p: &Palette) {
        let Some(book) = self.book().cloned() else {
            return;
        };
        let Content::Comic { pages } = &book.content else {
            return;
        };
        let Some(name) = pages.get(index).cloned() else {
            return;
        };
        match self.textures.get(&index) {
            Some(Some(tex)) => {
                let size = tex.size_vec2();
                let scale = (rect.width() / size.x).min(rect.height() / size.y);
                let r = Rect::from_center_size(rect.center(), size * scale);
                ui.painter().image(
                    tex.id(),
                    r,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            Some(None) => {}
            None => {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "…",
                    FontId::proportional(18.0),
                    p.muted,
                );
                self.textures.insert(index, None);
                // This page and the next spread's, ahead of time.
                for i in [index, index + 2, index + 3] {
                    if i != index && self.textures.contains_key(&i) {
                        continue;
                    }
                    let Some(name) = (if i == index {
                        Some(name.clone())
                    } else {
                        pages.get(i).cloned()
                    }) else {
                        continue;
                    };
                    if i != index {
                        self.textures.insert(i, None);
                    }
                    let (tx, ctx, path) =
                        (self.decode_tx.clone(), ui.ctx().clone(), self.path.clone());
                    let max_w = (rect.width() * ui.ctx().pixels_per_point() * 1.5) as u32;
                    std::thread::spawn(move || {
                        let img =
                            ebook::comic_page(&path, &name).and_then(|b| decode_image(&b, max_w));
                        let _ = tx.send((i, img));
                        ctx.request_repaint();
                    });
                }
            }
        }
    }

    /// The chapter list, for the sidebar.
    pub fn chapters(&self) -> Vec<(String, usize, bool)> {
        let Some(book) = self.book() else {
            return Vec::new();
        };
        let current = book
            .chapters
            .iter()
            .rposition(|c| c.block <= self.mark.block);
        book.chapters
            .iter()
            .enumerate()
            .map(|(i, c)| (one_line(&c.title), c.block, Some(i) == current))
            .collect()
    }

    pub fn is_comic(&self) -> bool {
        self.book().is_some_and(|b| b.is_comic())
    }
}

/// Decodes and, if wider than `max_w`, scales down an image.
fn decode_image(bytes: &[u8], max_w: u32) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if max_w > 0 && img.width() > max_w {
        let h = (img.height() as u64 * max_w as u64 / img.width().max(1) as u64) as u32;
        img.resize(max_w, h.max(1), image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

/// Titles can carry line breaks ("PRIDE.\nand\nPREJUDICE"): one line please.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn centred_note(ui: &mut egui::Ui, area: Rect, p: &Palette, text: &str, spinning: bool) {
    let c = area.center();
    if spinning {
        let t = ui.input(|i| i.time) as f32 * 5.0;
        let pts: Vec<Pos2> = (0..=24)
            .map(|i| {
                let a = t + i as f32 / 24.0 * std::f32::consts::PI * 1.4;
                c + Vec2::new(a.cos(), a.sin() - 3.0) * 10.0
            })
            .collect();
        ui.painter()
            .add(egui::Shape::line(pts, Stroke::new(2.0, p.accent)));
        ui.ctx().request_repaint();
    }
    ui.painter().text(
        c + Vec2::new(0.0, 14.0),
        egui::Align2::CENTER_TOP,
        text,
        FontId::proportional(14.5),
        p.muted,
    );
}
