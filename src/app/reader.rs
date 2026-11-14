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

use eframe::egui::{self, Align, FontId, Pos2, Rect, Stroke, Vec2};

use crate::ebook::{self, Block, Book, Content, Run};
use crate::ui::reading::BookFont;
use crate::ui::theme::{self, Palette};

/// Where the reader is: a block and a character within it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Mark {
    pub block: usize,
    pub chr: usize,
}

impl Mark {
    fn key(self) -> (usize, usize) {
        (self.block, self.chr)
    }
}

/// Space between lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Spacing {
    Compact,
    Normal,
    Relaxed,
}

impl Spacing {
    pub const ALL: [Spacing; 3] = [Spacing::Compact, Spacing::Normal, Spacing::Relaxed];
    pub fn label(self) -> &'static str {
        match self {
            Spacing::Compact => "Compact",
            Spacing::Normal => "Normal",
            Spacing::Relaxed => "Relaxed",
        }
    }
    fn line(self) -> f32 {
        match self {
            Spacing::Compact => 1.36,
            Spacing::Normal => 1.55,
            Spacing::Relaxed => 1.8,
        }
    }
}

/// White space around the text on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Margins {
    Narrow,
    Normal,
    Wide,
}

impl Margins {
    pub const ALL: [Margins; 3] = [Margins::Narrow, Margins::Normal, Margins::Wide];
    pub fn label(self) -> &'static str {
        match self {
            Margins::Narrow => "Narrow",
            Margins::Normal => "Normal",
            Margins::Wide => "Wide",
        }
    }
    pub fn factor(self) -> f32 {
        match self {
            Margins::Narrow => 0.55,
            Margins::Normal => 1.0,
            Margins::Wide => 1.55,
        }
    }
}

/// How a book is typeset: the face, its size, and the page's rhythm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ReadingStyle {
    pub font: BookFont,
    /// Logical points, before the face's own size correction.
    pub size: f32,
    pub spacing: Spacing,
    pub margins: Margins,
    /// Paragraphs as in a printed book (first line indented, no gap between
    /// them) rather than spaced apart as on the web.
    pub indent: bool,
}

impl Default for ReadingStyle {
    fn default() -> Self {
        ReadingStyle {
            font: BookFont::NotoSerif,
            size: 18.0,
            spacing: Spacing::Normal,
            margins: Margins::Normal,
            indent: true,
        }
    }
}

impl ReadingStyle {
    pub fn encode(&self) -> String {
        format!(
            "font={};size={:.1};spacing={};margins={};para={}",
            self.font.key(),
            self.size,
            self.spacing.label().to_ascii_lowercase(),
            self.margins.label().to_ascii_lowercase(),
            if self.indent { "indent" } else { "spaced" }
        )
    }

    pub fn decode(raw: &str) -> ReadingStyle {
        let mut s = ReadingStyle::default();
        for pair in raw.split(';') {
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            match k {
                "font" => s.font = BookFont::from_key(v),
                "size" => {
                    if let Ok(n) = v.parse::<f32>() {
                        s.size = n.clamp(FONT_MIN, FONT_MAX);
                    }
                }
                "spacing" => {
                    if let Some(x) = Spacing::ALL
                        .into_iter()
                        .find(|x| x.label().eq_ignore_ascii_case(v))
                    {
                        s.spacing = x;
                    }
                }
                "margins" => {
                    if let Some(x) = Margins::ALL
                        .into_iter()
                        .find(|x| x.label().eq_ignore_ascii_case(v))
                    {
                        s.margins = x;
                    }
                }
                "para" => s.indent = v != "spaced",
                _ => {}
            }
        }
        s
    }

    /// The size text is actually set at.
    fn px(&self) -> f32 {
        self.size * self.font.scale()
    }

    /// Book typography: a paragraph that follows another is indented, with
    /// no gap between them (unless the reader prefers spaced paragraphs).
    fn indented(&self, blocks: &[Block], i: usize) -> bool {
        self.indent
            && i > 0
            && matches!(blocks[i], Block::Para { .. })
            && matches!(blocks[i - 1], Block::Para { .. })
    }

    /// Space after a block.
    pub fn gap_after(&self, block: &Block) -> f32 {
        match block {
            Block::Para { .. } if self.indent => self.px() * 0.18,
            _ => self.px() * 0.55,
        }
    }
}

/// Lays out block `i` of a book as text, in `style`.
pub(super) fn block_job(
    style: &ReadingStyle,
    blocks: &[Block],
    i: usize,
    width: f32,
    ink: egui::Color32,
) -> Option<egui::text::LayoutJob> {
    let block = &blocks[i];
    let px = style.px();
    let (runs, size, centred, line) = match block {
        Block::Para { runs } => (runs, px, false, style.spacing.line()),
        Block::Heading { level, runs } => {
            let scale = match level {
                1 => 1.65,
                2 => 1.4,
                3 => 1.2,
                _ => 1.08,
            };
            (runs, px * scale, true, 1.3)
        }
        _ => return None,
    };
    let mut job = egui::text::LayoutJob::default();
    let mut indent = if style.indented(blocks, i) {
        px * 1.5
    } else {
        0.0
    };
    let heading = matches!(block, Block::Heading { .. });
    for Run { text, style: run } in runs {
        let family = style.font.family(run.bold || heading, run.italic);
        job.append(
            text,
            std::mem::take(&mut indent),
            egui::TextFormat {
                font_id: FontId::new(size, family),
                color: ink,
                line_height: Some(size * line),
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

/// A jump waiting for pagination to reach it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Seek {
    /// A page number (0-based), typed into the page box.
    Page(usize),
    /// A place in the text: a chapter, or a spot on the progress bar. The
    /// current pages stay up meanwhile.
    Mark(Mark),
    /// The reading position, found again after the type or window changed.
    /// Nothing is shown until it's found.
    Place(Mark),
}

#[derive(Clone, Copy, PartialEq)]
struct LayoutKey {
    col: (i32, i32),
    style: ReadingStyle,
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
    pub style: ReadingStyle,
    /// A jump waiting for pagination to get there.
    seek: Option<Seek>,
    /// Characters before each block (and the total, last), for progress and
    /// for estimating the page count while pages are still being laid out.
    chars: Vec<usize>,
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
    /// When the current layout began (reported in debug builds).
    #[cfg(debug_assertions)]
    layout_started: Option<Instant>,
}

/// Font size range, logical points.
pub(super) const FONT_MIN: f32 = 13.0;
pub(super) const FONT_MAX: f32 = 30.0;

impl BookView {
    pub fn open(path: &Path, mark: Mark, spread: bool, style: ReadingStyle) -> BookView {
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
            style: ReadingStyle {
                size: style.size.clamp(FONT_MIN, FONT_MAX),
                ..style
            },
            seek: None,
            chars: Vec::new(),
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
            #[cfg(debug_assertions)]
            layout_started: None,
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
            Content::Text { blocks, .. } => match self.chars.last() {
                Some(&total) if total > 0 => {
                    let before = self.chars.get(self.mark.block).copied().unwrap_or(total);
                    ((before + self.mark.chr) as f32 / total as f32).min(1.0)
                }
                _ => self.mark.block as f32 / blocks.len().max(1) as f32,
            },
        }
    }

    /// The page shown first (1-based).
    pub fn page_now(&self) -> usize {
        self.cur + 1
    }

    /// How many pages the book has, and whether that's final. While pages
    /// are still being laid out it's an estimate from how much text is done.
    pub fn page_total(&self) -> (usize, bool) {
        let n = self.page_count();
        if self.done || self.is_comic() {
            return (n, true);
        }
        let total = self.chars.last().copied().unwrap_or(0);
        let placed = self.chars.get(self.next_block).copied().unwrap_or(total);
        if placed == 0 || n < 3 {
            return (n, false);
        }
        let estimate = (n as f64 * total as f64 / placed as f64).round() as usize;
        (estimate.max(n), false)
    }

    /// Re-finding the reading position after a layout change: page turns
    /// wait for it, or they'd start from the wrong page.
    fn finding_place(&self) -> bool {
        matches!(self.seek, Some(Seek::Place(_))) && self.key.is_some() && !self.is_comic()
    }

    /// The page a typed jump is waiting for (1-based), while it's laid out.
    pub fn seeking_page(&self) -> Option<usize> {
        match self.seek {
            Some(Seek::Page(n)) => Some(n.saturating_add(1)),
            _ => None,
        }
    }

    /// Jumps to page `page` (0-based), laying pages out first if need be.
    pub fn goto_page(&mut self, page: usize) {
        if self.is_comic() || page < self.pages.len() || self.done {
            let n = self.page_count();
            if n == 0 {
                return;
            }
            let page = page.min(n - 1);
            self.cur = page - page % self.step();
            self.seek = None;
            self.sync_mark();
        } else {
            self.seek = Some(Seek::Page(page));
        }
    }

    /// Jumps to a fraction of the way through the book (the progress bar).
    pub fn goto_fraction(&mut self, f: f32) {
        let f = f.clamp(0.0, 1.0);
        if self.is_comic() {
            let n = self.page_count();
            self.goto_page(((n as f32 * f) as usize).min(n.saturating_sub(1)));
            return;
        }
        let Some(&total) = self.chars.last() else {
            return;
        };
        let target = (total as f64 * f as f64) as usize;
        // The block holding that character.
        let block = self
            .chars
            .partition_point(|&c| c <= target)
            .saturating_sub(1);
        let block = block.min(self.chars.len().saturating_sub(2));
        let chr = target.saturating_sub(self.chars[block]);
        self.seek = Some(Seek::Mark(Mark { block, chr }));
    }

    /// Pages after the ones showing until the next chapter starts, once
    /// that far has been laid out.
    pub fn pages_left_in_chapter(&self) -> Option<usize> {
        let book = self.book()?;
        if book.is_comic() || self.pages.is_empty() {
            return None;
        }
        let next = book
            .chapters
            .iter()
            .map(|c| c.block)
            .find(|&b| b > self.mark.block);
        let shown_end = (self.cur + self.step()).min(self.pages.len());
        let end = match next {
            Some(block) => {
                let i = self.pages.iter().position(|p| p.start.block >= block);
                match i {
                    Some(i) => i,
                    None if self.done => self.pages.len(),
                    None => return None,
                }
            }
            None if self.done => self.pages.len(),
            None => return None,
        };
        Some(end.saturating_sub(shown_end))
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
        if self.finding_place() {
            return;
        }
        let n = self.page_count();
        if self.cur + self.step() < n {
            self.cur += self.step();
        }
        self.seek = None;
        self.sync_mark();
    }

    pub fn prev(&mut self) {
        if self.finding_place() {
            return;
        }
        self.cur = self.cur.saturating_sub(self.step());
        self.seek = None;
        self.sync_mark();
    }

    pub fn first(&mut self) {
        if self.finding_place() {
            return;
        }
        self.cur = 0;
        self.seek = None;
        self.sync_mark();
    }

    /// The last page; a book still being laid out gets there when it's done.
    pub fn last(&mut self) {
        if !self.done && !self.is_comic() {
            self.seek = Some(Seek::Page(usize::MAX));
            return;
        }
        let n = self.page_count();
        if n > 0 {
            self.cur = (n - 1) / self.step() * self.step();
        }
        self.seek = None;
        self.sync_mark();
    }

    /// Jumps to a chapter (a block in the text).
    pub fn goto_block(&mut self, block: usize) {
        if self.is_comic() {
            self.goto_page(block);
            return;
        }
        self.seek = Some(Seek::Mark(Mark { block, chr: 0 }));
    }

    pub fn set_spread(&mut self, spread: bool) {
        if self.spread != spread {
            self.spread = spread;
            self.key = None; // column width changes: lay out again
        }
    }

    pub fn set_style(&mut self, style: ReadingStyle) {
        let style = ReadingStyle {
            size: style.size.clamp(FONT_MIN, FONT_MAX),
            ..style
        };
        if style != self.style {
            self.style = style;
            self.key = None;
        }
    }

    /// Works on a pending jump: lays out more pages, and lands once the
    /// target is reached. Returns whether one is still pending.
    fn resolve_seek(&mut self, ctx: &egui::Context, col: Vec2, p: &Palette, budget: Duration) {
        let Some(seek) = self.seek else { return };
        match seek {
            Seek::Mark(m) | Seek::Place(m) => {
                // The page holding `m` is final once a later page has begun.
                let reached = |s: &BookView| {
                    s.done || s.pages.last().is_some_and(|pg| pg.start.key() > m.key())
                };
                if !reached(self) {
                    self.paginate(ctx, col, p, budget, None);
                }
                if reached(self) {
                    #[cfg(debug_assertions)]
                    if let Some(t) = self.layout_started {
                        eprintln!(
                            "paperview: found the place after {} ms",
                            t.elapsed().as_millis()
                        );
                    }
                    self.mark = m;
                    self.seek_mark();
                    self.seek = None;
                }
            }
            Seek::Page(n) => {
                if n >= self.pages.len() && !self.done {
                    self.paginate(ctx, col, p, budget, None);
                }
                if n < self.pages.len() || self.done {
                    self.seek = None;
                    self.goto_page(n);
                }
            }
        }
        if self.seek.is_some() {
            ctx.request_repaint();
        }
    }

    /// Counts characters per block, once the book is in.
    fn count_chars(&mut self) {
        let Some(book) = self.book().cloned() else {
            return;
        };
        let Content::Text { blocks, .. } = &book.content else {
            return;
        };
        let mut total = 0usize;
        self.chars = Vec::with_capacity(blocks.len() + 1);
        for b in blocks {
            self.chars.push(total);
            // An image takes about as long to look at as a paragraph.
            total += if matches!(b, Block::Image { .. }) {
                400
            } else {
                b.text_len()
            };
        }
        self.chars.push(total);
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
        #[cfg(debug_assertions)]
        {
            self.layout_started = Some(Instant::now());
        }
        self.key = Some(key);
        self.pages.clear();
        self.open_page = None;
        self.next_block = 0;
        self.used = 0.0;
        self.done = false;
    }

    fn job(
        &self,
        blocks: &[Block],
        i: usize,
        width: f32,
        p: &Palette,
    ) -> Option<egui::text::LayoutJob> {
        block_job(&self.style, blocks, i, width, p.ink)
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
        let px = self.style.px();
        let gap = px * 0.55;
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
                    let Some(job) = self.job(blocks, i, col.x, p) else {
                        continue;
                    };
                    let galley = ctx.fonts_mut(|f| f.layout_job(job));
                    let heading = matches!(block, Block::Heading { .. });
                    let before = if heading && self.used > 0.0 {
                        px * 1.2
                    } else {
                        0.0
                    };
                    // Keep a heading with at least two lines of what follows.
                    if heading
                        && self.used > 0.0
                        && self.used + before + galley.size().y + px * 3.2 > col.y
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
                    self.used += self.style.gap_after(block);
                }
            }
        }
        // All placed.
        if self.open_page.as_ref().is_some_and(|p| !p.items.is_empty()) {
            let page = self.open_page.take().unwrap();
            self.pages.push(page);
        }
        self.done = true;
        #[cfg(debug_assertions)]
        if let Some(t) = self.layout_started.take() {
            eprintln!(
                "paperview: laid out {} pages in {} ms",
                self.pages.len(),
                t.elapsed().as_millis()
            );
        }
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

        // Page geometry: one page, or two facing pages that meet at a spine
        // like an open book, centred in the area.
        let margin = Vec2::new(46.0 * self.style.margins.factor(), 56.0);
        let max_page_w = if self.spread { 620.0 } else { 720.0 };
        let avail = area.shrink2(Vec2::new(28.0, 20.0));
        let page_h = avail.height();
        let page_w = if self.spread {
            (avail.width() / 2.0).min(max_page_w).min(page_h * 0.78)
        } else {
            avail.width().min(max_page_w).min(page_h * 0.85)
        };
        let col = Vec2::new(
            (page_w - margin.x * 2.0).max(120.0),
            (page_h - margin.y * 2.0).max(120.0),
        );
        let key = LayoutKey {
            col: (col.x.round() as i32, col.y.round() as i32),
            style: self.style,
            spread: self.spread,
        };
        if self.chars.is_empty() {
            self.count_chars();
        }

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
                // Keep the reading position: find it again in the new layout
                // (a jump already on its way keeps going).
                let target = match self.seek {
                    Some(Seek::Page(_)) | None => Seek::Place(self.mark),
                    Some(seek) => seek,
                };
                self.reset_layout(key);
                self.pending = None;
                self.seek = Some(target);
            } else {
                ctx.request_repaint_after(Duration::from_millis(150));
            }
        }
        // A jump lays out in bigger slices, so it lands quickly.
        if !is_comic && self.key == Some(key) {
            // Nothing else is on screen while the place is found: use more of
            // the frame.
            let budget = if self.finding_place() { 30 } else { 14 };
            self.resolve_seek(&ctx, col, p, Duration::from_millis(budget));
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

        // Finding the reading position again after the type changed.
        if self.finding_place() {
            centred_note(ui, area, p, "Finding your place…", true);
            return self.after_input(ui, area, before);
        }

        let n_pages = if self.spread { 2 } else { 1 };
        let sheet =
            Rect::from_center_size(area.center(), Vec2::new(page_w * n_pages as f32, page_h));
        if self.cur < self.page_count() {
            paint_sheet(ui.painter(), sheet, self.spread && !is_comic, p);
        }
        for k in 0..n_pages {
            let index = self.cur + k;
            let rect = Rect::from_min_size(
                Pos2::new(sheet.left() + k as f32 * page_w, sheet.top()),
                Vec2::new(page_w, page_h),
            );
            if index >= self.page_count() {
                continue;
            }
            if is_comic {
                self.draw_comic_page(ui, index, rect.shrink(10.0), p);
            } else {
                let side = match (self.spread, k) {
                    (false, _) => Side::Single,
                    (true, 0) => Side::Left,
                    _ => Side::Right,
                };
                self.draw_text_page(ui, &ctx, index, rect, side, margin, col, p);
            }
        }
        self.after_input(ui, area, before)
    }

    /// Page turns from clicks and the wheel; true when the position moved.
    fn after_input(&mut self, ui: &mut egui::Ui, area: Rect, before: (usize, Mark)) -> bool {
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
        side: Side,
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

        // Running head, in small letterspaced capitals: the book on the left
        // page, the chapter on the right.
        let head = if side == Side::Right {
            self.chapter_title().unwrap_or_else(|| self.title.clone())
        } else {
            self.title.clone()
        };
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &head.to_uppercase(),
            0.0,
            egui::TextFormat {
                font_id: FontId::new(10.0, theme::medium()),
                color: p.muted,
                extra_letter_spacing: 1.4,
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(col.x);
        let galley = ctx.fonts_mut(|f| f.layout_job(job));
        painter.galley(
            Pos2::new(
                rect.center().x - galley.size().x / 2.0,
                rect.top() + margin.y * 0.45 - galley.size().y / 2.0,
            ),
            galley,
            p.muted,
        );
        // Folio: at the outer corner of a spread, centred on a single page.
        let (anchor, x) = match side {
            Side::Left => (egui::Align2::LEFT_CENTER, origin.x),
            Side::Right => (egui::Align2::RIGHT_CENTER, origin.x + col.x),
            Side::Single => (egui::Align2::CENTER_CENTER, rect.center().x),
        };
        painter.text(
            Pos2::new(x, rect.bottom() - margin.y * 0.45),
            anchor,
            (index + 1).to_string(),
            FontId::new(12.0, self.style.font.family(false, false)),
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
            let Some(job) = self.job(blocks, block, col.x, p) else {
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

/// Which page of the view a page is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Single,
    Left,
    Right,
}

/// The paper: a soft shadow under it and, for two pages, the fold of an
/// open book, shaded towards the spine.
fn paint_sheet(painter: &egui::Painter, sheet: Rect, spine: bool, p: &Palette) {
    let shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 24,
        spread: 0,
        color: egui::Color32::from_black_alpha(if p.dark { 110 } else { 34 }),
    };
    painter.add(shadow.as_shape(sheet, 4));
    painter.rect(
        sheet,
        4,
        p.surface,
        Stroke::new(1.0, p.hairline.gamma_multiply(0.7)),
        egui::StrokeKind::Outside,
    );
    if !spine {
        return;
    }
    let x = sheet.center().x;
    let shade = egui::Color32::from_black_alpha(if p.dark { 60 } else { 20 });
    let clear = egui::Color32::TRANSPARENT;
    let mut mesh = egui::Mesh::default();
    let mut band = |left: f32, right: f32, a: egui::Color32, b: egui::Color32| {
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(Pos2::new(left, sheet.top()), a);
        mesh.colored_vertex(Pos2::new(right, sheet.top()), b);
        mesh.colored_vertex(Pos2::new(right, sheet.bottom()), b);
        mesh.colored_vertex(Pos2::new(left, sheet.bottom()), a);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    band(x - 44.0, x, clear, shade);
    band(x, x + 26.0, shade, clear);
    painter.add(mesh);
    painter.vline(
        x,
        sheet.y_range(),
        Stroke::new(1.0, p.hairline.gamma_multiply(0.8)),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_style_round_trips() {
        let style = ReadingStyle {
            font: BookFont::Garamond,
            size: 21.0,
            spacing: Spacing::Relaxed,
            margins: Margins::Wide,
            indent: false,
        };
        assert_eq!(ReadingStyle::decode(&style.encode()), style);
    }

    #[test]
    fn reading_style_survives_junk() {
        let s = ReadingStyle::decode("font=nope;size=999;spacing=x;stray");
        assert_eq!(s.font, BookFont::NotoSerif);
        assert_eq!(s.size, FONT_MAX);
        assert_eq!(s.spacing, Spacing::Normal);
        assert_eq!(ReadingStyle::decode(""), ReadingStyle::default());
    }
}
