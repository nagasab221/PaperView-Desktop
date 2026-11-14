//! The Library: Recents as a shelf of first-page covers and, once a Backblaze
//! bucket is set up and switched on, a Cloud page beside it — the same two
//! tabs as the phone. The Cloud tab doesn't exist until the bucket does.

use std::path::PathBuf;

use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke};

use super::density::LibraryLayout;
use super::{
    fmt_speed, fmt_time, fmt_ts, folders, open_file_location, same_path, CloudOp, HomeScreen,
    LandingTab, ViewerApp, FREE_TIER_BYTES,
};
use crate::cloud;
use crate::covers::Cover;
use crate::ui::{theme, widgets as w};

const TILE_W: f32 = 150.0;
const TILE_GAP: f32 = 24.0;
const SMALL_TILE_W: f32 = 110.0;
/// Room under a cover: gap, two title lines, the caption.
const TILE_TEXT_H: f32 = 8.0 + 38.0 + 22.0;
/// A list row and its rule.
const LIST_ROW_H: f32 = 49.0;
const SMALL_TILE_GAP: f32 = 18.0;
/// Cover aspect (width / height), as on the phone.
const COVER_ASPECT: f32 = 0.77;
const COLUMN_MAX: f32 = 1080.0;

/// Texture coordinates that fill a frame of `aspect` (width / height) with
/// a page, cropping from the top like the phone does.
fn cover_uv(tex_size: [usize; 2], aspect: f32) -> egui::Rect {
    let tex_aspect = tex_size[0] as f32 / tex_size[1].max(1) as f32;
    if tex_aspect < aspect {
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, tex_aspect / aspect))
    } else {
        let f = aspect / tex_aspect;
        egui::Rect::from_min_max(
            egui::pos2((1.0 - f) * 0.5, 0.0),
            egui::pos2((1.0 + f) * 0.5, 1.0),
        )
    }
}

/// One document on a shelf. Its caption is only formatted when it's drawn.
struct Tile {
    path: PathBuf,
    /// Last opened (Recents) or modified (folders), unix seconds.
    when: i64,
    /// Recents can be removed from the Library; folder files can't.
    removable: bool,
}

/// A date for shelf captions, e.g. "13 Nov 2026". Remembered per day:
/// local-time conversion isn't free, and a shelf asks for the same few dates
/// over and over.
fn fmt_date(ts: i64) -> String {
    thread_local! {
        static DAYS: std::cell::RefCell<std::collections::HashMap<i64, String>> = Default::default();
    }
    // Keyed by the UTC hour so a timezone's day boundary is still respected.
    let hour = ts.div_euclid(3600);
    DAYS.with(|d| {
        d.borrow_mut()
            .entry(hour)
            .or_insert_with(|| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .map(|utc| {
                        utc.with_timezone(&chrono::Local)
                            .format("%-d %b %Y")
                            .to_string()
                    })
                    .unwrap_or_default()
            })
            .clone()
    })
}

fn unix(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A document already downloaded from the bucket.
pub(super) struct LocalDoc {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
}

/// Downloaded documents, read straight off the disk so they stay listed
/// (and open) with no network at all.
pub(super) fn read_local_docs() -> Vec<LocalDoc> {
    let mut docs: Vec<(LocalDoc, std::time::SystemTime)> = std::fs::read_dir(cloud::cache_dir())
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let meta = e.metadata().ok()?;
                    if !meta.is_file() || meta.len() == 0 {
                        return None;
                    }
                    let name = e.file_name().to_string_lossy().into_owned();
                    Some((
                        LocalDoc {
                            path: e.path(),
                            name,
                            size: meta.len(),
                        },
                        meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    docs.sort_by_key(|d| std::cmp::Reverse(d.1));
    docs.into_iter().map(|(d, _)| d).collect()
}

impl ViewerApp {
    pub(super) fn refresh_local_docs(&mut self) {
        self.cloud_local = read_local_docs();
        self.cloud_cache = cloud::cache_stats();
    }

    /// The home view: Library or Settings, on the paper background.
    pub(super) fn home_view(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_width();
        let col = COLUMN_MAX.min(avail - 48.0).max(280.0);
        let pad = ((avail - col) * 0.5).max(24.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt(("home", self.home))
            .show(ui, |ui| {
                // Library <-> Settings is a page change: fade the page.
                w::fade_in(ui, "home", self.home);
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        match self.home {
                            HomeScreen::Library => self.library_page(ui),
                            HomeScreen::Settings => self.settings_page(ui),
                        }
                        ui.add_space(40.0);
                    });
                });
            });
    }

    /// The Library's tabs right now: Recents always, Folders once one is
    /// added, Cloud once a bucket is set up and switched on.
    fn library_tabs(&self) -> Vec<(&'static str, LandingTab)> {
        let mut tabs = vec![("Recents", LandingTab::Recent)];
        if !self.books.is_empty() {
            tabs.push(("Books", LandingTab::Books));
        }
        if !self.folders.is_empty() {
            tabs.push(("Folders", LandingTab::Folders));
        }
        if self.cloud_active() {
            tabs.push(("Cloud", LandingTab::Saved));
        }
        tabs
    }

    fn library_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let tabs = self.library_tabs();
        if !tabs.iter().any(|(_, t)| *t == self.landing_tab) {
            self.landing_tab = LandingTab::Recent;
        }
        let subtitle = match self.landing_tab {
            LandingTab::Saved => self.cloud_cfg.bucket.trim().to_string(),
            LandingTab::Books => {
                let n = self.books.len();
                format!("{n} book{}", if n == 1 { "" } else { "s" })
            }
            LandingTab::Folders => {
                let n = self.folders.len();
                let docs: usize = self.folder_listings.iter().map(|l| l.docs.len()).sum();
                format!(
                    "{n} folder{} · {docs} PDF{}",
                    if n == 1 { "" } else { "s" },
                    if docs == 1 { "" } else { "s" }
                )
            }
            LandingTab::Recent if self.recents.is_empty() => {
                "Open a PDF with Ctrl+O, or drop a PDF or a folder anywhere in this window"
                    .to_string()
            }
            LandingTab::Recent => {
                let n = self.recents.len();
                format!("{n} document{}", if n == 1 { "" } else { "s" })
            }
        };

        let mut open = false;
        let mut settings = false;
        let mut add_folder = false;
        w::masthead(ui, "Library", Some(&subtitle), |ui| {
            if w::icon_button(ui, "⚙", "Settings (Ctrl+,)", false).clicked() {
                settings = true;
            }
            if w::primary_button(ui, "Open a PDF", true)
                .on_hover_text("Ctrl+O")
                .clicked()
            {
                open = true;
            }
            if w::outlined_button(ui, "Add folder", true)
                .on_hover_text("Show every PDF in a folder (and its subfolders) here")
                .clicked()
            {
                add_folder = true;
            }
        });
        if open {
            self.open_dialog(&ctx);
        }
        if add_folder {
            self.pick_folder();
        }
        if settings {
            self.open_settings(&ctx);
            return;
        }

        if tabs.len() > 1 {
            ui.add_space(14.0);
            let labels: Vec<&str> = tabs.iter().map(|(l, _)| *l).collect();
            let sel = tabs
                .iter()
                .position(|(_, t)| *t == self.landing_tab)
                .unwrap_or(0);
            if let Some(i) = w::text_tabs(ui, &labels, sel) {
                self.landing_tab = tabs[i].1;
                if self.landing_tab == LandingTab::Folders {
                    self.rescan_folders();
                }
            }
        } else {
            ui.add_space(10.0);
        }

        // Only what's under the tabs changes when they're switched; the
        // heading and the tabs themselves stay put.
        let tab = self.landing_tab;
        ui.scope(|ui| {
            w::fade_in(ui, "library-tab", tab);
            match tab {
                LandingTab::Saved => self.cloud_page(ui, &ctx),
                LandingTab::Books => self.books_shelf(ui, &ctx),
                LandingTab::Folders => self.folders_page(ui, &ctx),
                LandingTab::Recent => self.recents_shelf(ui, &ctx),
            }
        });
    }

    // ── Recents ──────────────────────────────────────────────────────────────

    fn recents_shelf(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.recents.is_empty() {
            let mut open = false;
            w::empty_state(
                ui,
                "Nothing here yet",
                "Open a PDF and it appears on this shelf, showing its first page and the page you left off on.",
                Some(("Open a PDF", &mut open)),
            );
            if open {
                self.open_dialog(ctx);
            }
            return;
        }
        ui.add_space(22.0);
        let tiles: Vec<Tile> = self
            .recents
            .iter()
            .map(|r| Tile {
                path: r.path.clone(),
                when: r.ts,
                removable: true,
            })
            .collect();
        let (open, remove) = self.shelf(ui, ctx, &tiles);
        if let Some(i) = remove {
            self.recents.remove(i);
        }
        if let Some(p) = open {
            self.open_path(ctx, &p);
        }
    }

    // ── Books ────────────────────────────────────────────────────────────────

    fn books_shelf(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut add = false;
        w::section_heading(ui, "Your books", |ui| {
            if w::inline_button(ui, "Add books…", true).clicked() {
                add = true;
            }
        });
        if add {
            self.pick_books();
        }
        ui.add_space(14.0);
        let tiles: Vec<Tile> = self
            .books
            .iter()
            .map(|b| Tile {
                path: b.path.clone(),
                when: b.ts,
                removable: true,
            })
            .collect();
        let (open, remove) = self.shelf(ui, ctx, &tiles);
        if let Some(i) = remove {
            if i < self.books.len() {
                self.books.remove(i);
            }
        }
        if let Some(path) = open {
            self.open_book(ctx, &path);
        }
    }

    // ── Folders ──────────────────────────────────────────────────────────────

    fn folders_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        // Folders change behind our back; re-read them now and then while shown.
        if self
            .folders_scanned
            .is_none_or(|t| t.elapsed() >= folders::RESCAN_EVERY)
        {
            self.rescan_folders();
        }
        ctx.request_repaint_after(folders::RESCAN_EVERY);

        let mut open: Option<PathBuf> = None;
        let mut remove: Option<PathBuf> = None;
        let mut refresh = false;
        if self.folder_listings.is_empty() && self.scanning_folders() {
            loading_block(ui, "Reading your folders…", None);
            ctx.request_repaint();
            return;
        }
        let listings = std::mem::take(&mut self.folder_listings);
        for listing in &listings {
            let root = listing.root.clone();
            w::section_heading(ui, &folders::folder_name(&root), |ui| {
                if w::inline_button(ui, "Remove", true)
                    .on_hover_text("Stop showing this folder (the files stay where they are)")
                    .clicked()
                {
                    remove = Some(root.clone());
                }
                if w::inline_button(ui, "Open folder", !listing.missing).clicked() {
                    let _ = std::process::Command::new(if cfg!(target_os = "windows") {
                        "explorer"
                    } else {
                        "xdg-open"
                    })
                    .arg(&root)
                    .spawn();
                }
                if w::inline_button(ui, "Refresh", true).clicked() {
                    refresh = true;
                }
            });
            ui.label(
                RichText::new(root.display().to_string())
                    .small()
                    .color(p.muted),
            );
            if listing.missing {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "This folder can't be found. Was it moved, or is its drive unplugged?",
                    )
                    .color(ui.visuals().error_fg_color),
                );
                continue;
            }
            if listing.docs.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("No PDFs in this folder yet.").color(p.muted));
                continue;
            }
            // Covers are made in the background; show the shelf once they're
            // all ready rather than letting it fill in tile by tile.
            let paths: Vec<PathBuf> = listing.docs.iter().map(|d| d.path.clone()).collect();
            let ready = self.covers.prepare(ctx, &paths);
            if ready < paths.len() {
                ui.add_space(12.0);
                loading_block(ui, "Preparing covers…", Some((ready, paths.len())));
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                continue;
            }
            w::fade_in(ui, &format!("folder-{}", root.display()), true);
            // One shelf per subfolder, the folder's own files first.
            let mut start = 0;
            while start < listing.docs.len() {
                let sub = listing.docs[start].subdir.clone();
                let end = listing.docs[start..]
                    .iter()
                    .position(|d| d.subdir != sub)
                    .map_or(listing.docs.len(), |n| start + n);
                if !sub.is_empty() {
                    group_label(ui, &sub, end - start);
                }
                ui.add_space(14.0);
                let tiles: Vec<Tile> = listing.docs[start..end]
                    .iter()
                    .map(|d| Tile {
                        path: d.path.clone(),
                        when: unix(d.modified),
                        removable: false,
                    })
                    .collect();
                if let (Some(path), _) = self.shelf(ui, ctx, &tiles) {
                    open = Some(path);
                }
                start = end;
            }
            if listing.truncated {
                ui.label(
                    RichText::new("Only the first 3000 PDFs are shown.")
                        .small()
                        .color(p.muted),
                );
            }
        }
        self.folder_listings = listings;
        ui.add_space(18.0);
        let mut add = false;
        ui.horizontal(|ui| {
            if w::outlined_button(ui, "Add another folder", true).clicked() {
                add = true;
            }
            ui.label(
                RichText::new("Tip: you can also drop a folder onto this window.")
                    .small()
                    .color(p.muted),
            );
        });
        if add {
            self.pick_folder();
        }
        if refresh {
            self.rescan_folders();
        }
        if let Some(path) = remove {
            self.remove_folder(&path);
        }
        if let Some(path) = open {
            self.open_path(ctx, &path);
        }
    }

    /// The caption under a tile: where you are, if you've read it, else its date.
    fn caption(&self, tile: &Tile, exists: bool) -> String {
        if !exists {
            return "File moved or deleted".to_string();
        }
        if let Some(b) = self.books.iter().find(|b| same_path(&b.path, &tile.path)) {
            let progress = if b.progress > 0.005 {
                format!("{:.0}%", (b.progress * 100.0).max(1.0))
            } else {
                "Not started".to_string()
            };
            return match &b.author {
                Some(a) => format!("{progress} · {a}"),
                None => progress,
            };
        }
        match self.recents.iter().find(|r| same_path(&r.path, &tile.path)) {
            Some(r) => format!("page {} · {}", r.page + 1, fmt_date(r.ts)),
            None => fmt_date(tile.when),
        }
    }

    // ── Shelf ────────────────────────────────────────────────────────────────

    /// Tiles in rows. Returns the tile clicked, and the index of one removed.
    fn shelf(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tiles: &[Tile],
    ) -> (Option<PathBuf>, Option<usize>) {
        if self.library_layout == LibraryLayout::List {
            return self.shelf_list(ui, ctx, tiles);
        }
        let (tile, gap) = if self.library_layout == LibraryLayout::SmallCovers {
            (SMALL_TILE_W, SMALL_TILE_GAP)
        } else {
            (TILE_W, TILE_GAP)
        };
        let width = ui.available_width();
        let cols = (((width + gap) / (tile + gap)).floor() as usize).max(1);
        let tile_w = ((width - gap * (cols as f32 - 1.0)) / cols as f32).min(tile + 30.0);
        let mut open_req = None;
        let mut remove_req = None;
        // Rows off screen are skipped, only their space reserved: a shelf of
        // hundreds then costs what the visible rows cost.
        let row_h = tile_w / COVER_ASPECT + TILE_TEXT_H;
        // A screen above and below the view: covers there load ahead of time.
        let clip = ui.clip_rect();
        let near = clip.expand2(egui::vec2(0.0, clip.height()));
        for (row, chunk) in tiles.chunks(cols).enumerate() {
            let probe = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, row_h));
            if !ui.is_rect_visible(probe) {
                if near.intersects(probe) {
                    for tile in chunk {
                        self.covers.warm(ctx, &tile.path);
                    }
                }
                ui.allocate_space(egui::vec2(width, row_h));
                ui.add_space(18.0);
                continue;
            }
            ui.horizontal_top(|ui| {
                ui.set_height(row_h);
                ui.spacing_mut().item_spacing.x = gap;
                for (k, tile) in chunk.iter().enumerate() {
                    let (open, remove) = self.cover_tile(ui, ctx, tile, tile_w);
                    if open {
                        open_req = Some(tile.path.clone());
                    }
                    if remove {
                        remove_req = Some(row * cols + k);
                    }
                }
            });
            ui.add_space(18.0);
        }
        (open_req, remove_req)
    }

    /// The shelf as a compact list: a small cover, the title, the line about it.
    fn shelf_list(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tiles: &[Tile],
    ) -> (Option<PathBuf>, Option<usize>) {
        let p = w::pal(ui);
        let mut open_req = None;
        let mut remove_req = None;
        let clip = ui.clip_rect();
        let near = clip.expand2(egui::vec2(0.0, clip.height()));
        for (i, tile) in tiles.iter().enumerate() {
            let probe = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), LIST_ROW_H),
            );
            if !ui.is_rect_visible(probe) {
                if near.intersects(probe) {
                    self.covers.warm(ctx, &tile.path);
                }
                ui.allocate_space(egui::vec2(ui.available_width(), LIST_ROW_H));
                continue;
            }
            let path = tile.path.as_path();
            let exists = self.covers.exists(ctx, path);
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let meta = self.caption(tile, exists);
            let (resp, thumb) = w::row_lead(
                ui,
                exists,
                egui::vec2(31.0, 40.0),
                &name,
                Some(&meta),
                |_| {},
            );
            // A small first page in the row's leading space.
            if ui.is_rect_visible(thumb) {
                ui.painter().rect_filled(thumb, 0, p.surface);
                if let Some(Cover::Ready(tex)) =
                    exists.then(|| self.covers.get(ctx, path)).flatten()
                {
                    let uv = cover_uv(tex.size(), thumb.width() / thumb.height());
                    ui.painter()
                        .image(tex.id(), thumb, uv, egui::Color32::WHITE);
                }
                ui.painter().rect_stroke(
                    thumb,
                    0,
                    Stroke::new(1.0, p.hairline),
                    egui::StrokeKind::Outside,
                );
            }
            if resp.clicked() {
                open_req = Some(tile.path.clone());
            }
            resp.on_hover_text(path.display().to_string())
                .context_menu(|ui| {
                    if exists && ui.button("Open").clicked() {
                        open_req = Some(tile.path.clone());
                        ui.close();
                    }
                    if ui.button("Show in folder").clicked() {
                        open_file_location(path);
                        ui.close();
                    }
                    if tile.removable && ui.button("Remove from library").clicked() {
                        remove_req = Some(i);
                        ui.close();
                    }
                });
            w::hairline(ui);
        }
        ui.add_space(12.0);
        (open_req, remove_req)
    }

    /// One document on a shelf: cover, title, and a line about it. The cover
    /// lifts a little under the pointer.
    fn cover_tile(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tile: &Tile,
        tile_w: f32,
    ) -> (bool, bool) {
        let p = w::pal(ui);
        let path = tile.path.as_path();
        let exists = self.covers.exists(ctx, path);
        let name = self
            .books
            .iter()
            .find(|b| same_path(&b.path, path))
            .map(|b| b.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            });
        let mut open = false;
        let mut remove = false;

        ui.vertical(|ui| {
            ui.set_width(tile_w);
            let cover_size = egui::vec2(tile_w, tile_w / COVER_ASPECT);
            let (slot, resp) = ui.allocate_exact_size(
                cover_size,
                if exists {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            let hover = w::hover_t(ui, &resp, exists);
            let rect = slot.translate(egui::vec2(0.0, -3.0 * hover));
            if ui.is_rect_visible(slot) {
                let painter = ui.painter_at(slot.expand(6.0));
                // A soft shadow grows under the lifted cover.
                if hover > 0.0 {
                    painter.rect_filled(
                        rect.translate(egui::vec2(0.0, 3.0)).expand(1.0),
                        2,
                        p.ink.gamma_multiply(0.10 * hover),
                    );
                }
                painter.rect_filled(rect, 0, p.surface);
                // Only tiles on screen ask for their cover.
                match exists.then(|| self.covers.get(ctx, path)).flatten() {
                    Some(Cover::Ready(tex)) => {
                        let uv = cover_uv(tex.size(), COVER_ASPECT);
                        painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
                    }
                    _ if exists && crate::ebook::is_ebook(path) => {
                        // A typeset cover: the title in serif on tinted paper.
                        painter.rect_filled(rect, 0, p.accent_wash());
                        let inner = rect.shrink(10.0);
                        painter.rect_stroke(
                            inner,
                            0,
                            Stroke::new(1.0, p.accent.gamma_multiply(0.5)),
                            egui::StrokeKind::Inside,
                        );
                        let galley = painter.layout(
                            name.clone(),
                            egui::FontId::new((tile_w / 9.0).clamp(12.0, 19.0), theme::serif()),
                            p.ink,
                            inner.width() - 16.0,
                        );
                        let pos = egui::pos2(
                            inner.center().x - galley.size().x / 2.0,
                            inner.top() + inner.height() * 0.28,
                        );
                        painter.galley(pos, galley, p.ink);
                    }
                    _ => {
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            if exists { "PDF" } else { "Missing" },
                            egui::FontId::new(11.0, theme::medium()),
                            p.muted,
                        );
                    }
                }
                let edge = p.hairline.lerp_to_gamma(p.muted, hover);
                painter.rect_stroke(rect, 0, Stroke::new(1.0, edge), egui::StrokeKind::Outside);
            }
            if resp.hovered() && exists {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                open = true;
            }
            resp.on_hover_text(path.display().to_string())
                .context_menu(|ui| {
                    if exists && ui.button("Open").clicked() {
                        open = true;
                        ui.close();
                    }
                    if ui.button("Show in folder").clicked() {
                        open_file_location(path);
                        ui.close();
                    }
                    if tile.removable && ui.button("Remove from library").clicked() {
                        remove = true;
                        ui.close();
                    }
                });

            ui.add_space(8.0);
            // At most two lines, always two lines' room: every row is the
            // same height, which is what lets off-screen rows be skipped.
            let mut job = egui::text::LayoutJob::single_section(
                name.clone(),
                egui::TextFormat {
                    font_id: egui::FontId::new(14.0, theme::medium()),
                    color: if exists { p.ink } else { p.muted },
                    ..Default::default()
                },
            );
            job.wrap = egui::text::TextWrapping {
                max_width: tile_w,
                max_rows: 2,
                break_anywhere: false,
                overflow_character: Some('…'),
            };
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let (title_rect, _) = ui.allocate_exact_size(egui::vec2(tile_w, 38.0), Sense::hover());
            ui.painter().galley(title_rect.min, galley, p.ink);
            let meta = self.caption(tile, exists);
            ui.add(egui::Label::new(RichText::new(meta).small().color(p.muted)).truncate());
        });
        (open, remove)
    }

    // ── Cloud ────────────────────────────────────────────────────────────────

    fn cloud_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        // Auto-list once (an error stops retries until Refresh).
        if !self.cloud_listed && !self.cloud_busy && self.cloud_status.is_none() {
            self.start_cloud_op(ctx, CloudOp::List);
        }

        let mut refresh = false;
        let mut upload = false;
        let busy = self.cloud_busy;
        w::section_heading(ui, "Saved to cloud", |ui| {
            if w::inline_button(ui, "Refresh", !busy).clicked() {
                refresh = true;
            }
            if w::inline_button(ui, "Upload a PDF…", true).clicked() {
                upload = true;
            }
        });
        if refresh {
            self.cloud_listed = false;
            self.cloud_status = None;
            self.start_cloud_op(ctx, CloudOp::List);
        }
        if upload {
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter("PDF documents", &["pdf"])
                .pick_files()
            {
                let n = paths.len();
                for path in paths {
                    self.start_cloud_op(ctx, CloudOp::Upload(path));
                }
                self.toast(format!(
                    "Uploading {n} document{}",
                    if n == 1 { "" } else { "s" }
                ));
            }
        }

        // Live transfer readout.
        if let Some((done, total, bps)) = self.cloud_progress {
            ui.add_space(8.0);
            let label = self
                .cloud_status
                .clone()
                .unwrap_or_else(|| "Transferring…".into());
            ui.label(
                RichText::new(format!(
                    "{label}  {}%  ·  {}",
                    (done as f32 / total.max(1) as f32 * 100.0) as u32,
                    fmt_speed(bps)
                ))
                .small()
                .color(p.muted),
            );
            ui.add(
                egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                    .desired_height(2.0)
                    .fill(p.accent),
            );
        } else if busy {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(
                        self.cloud_status
                            .clone()
                            .unwrap_or_else(|| "Connecting…".into()),
                    )
                    .color(p.muted),
                );
            });
        }

        // Selecting turns every row's "…" into a checkbox.
        let selecting = !self.cloud_selected.is_empty();
        if selecting {
            self.selection_bar(ui, ctx);
        }

        // Split by where each file actually is.
        let downloaded: Vec<CloudEntry> = self
            .cloud_local
            .iter()
            .map(|doc| CloudEntry {
                name: doc.name.clone(),
                bucket: self
                    .cloud_files
                    .iter()
                    .find(|f| local_name(&f.name) == doc.name)
                    .map(|f| (f.name.clone(), f.file_id.clone(), f.size, f.uploaded_ms)),
                local: Some((doc.path.clone(), doc.size)),
            })
            .collect();
        let local_names: std::collections::HashSet<&str> =
            self.cloud_local.iter().map(|d| d.name.as_str()).collect();
        let in_bucket: Vec<CloudEntry> = self
            .cloud_files
            .iter()
            .filter(|f| !local_names.contains(local_name(&f.name).as_str()))
            .map(|f| CloudEntry {
                name: f.name.clone(),
                bucket: Some((f.name.clone(), f.file_id.clone(), f.size, f.uploaded_ms)),
                local: None,
            })
            .collect();

        let mut acts: Vec<CloudAct> = Vec::new();
        if !downloaded.is_empty() {
            group_label(ui, "On this device", downloaded.len());
            for e in &downloaded {
                self.cloud_row(ui, e, selecting, &mut acts);
            }
        }
        if !in_bucket.is_empty() {
            group_label(ui, "In your bucket", in_bucket.len());
            for e in &in_bucket {
                self.cloud_row(ui, e, selecting, &mut acts);
            }
        }
        for act in acts {
            self.cloud_act(ctx, act);
        }

        if self.cloud_listed && self.cloud_files.is_empty() && self.cloud_local.is_empty() && !busy
        {
            w::empty_state(
                ui,
                "Your bucket is empty",
                "Open a document and choose ☰ > Save to cloud, drop PDFs onto this page, or upload above.",
                None,
            );
        }

        if let Some(msg) = self.cloud_status.clone().filter(|_| !busy) {
            ui.add_space(12.0);
            if self.cloud_files.is_empty() && !self.cloud_listed {
                // Nothing listed at all: this is the reason, so say it plainly.
                let auth = msg.to_ascii_lowercase().contains("authoriz");
                let headline = if auth {
                    "Backblaze didn't accept these details"
                } else {
                    "Couldn't reach your bucket"
                };
                ui.label(
                    RichText::new(headline)
                        .text_style(theme::title())
                        .color(p.ink),
                );
                ui.label(
                    RichText::new(&msg)
                        .small()
                        .color(ui.visuals().error_fg_color),
                );
                ui.add_space(6.0);
                let mut settings = false;
                ui.horizontal(|ui| {
                    if w::inline_button(ui, "Check the details in Settings", true).clicked() {
                        settings = true;
                    }
                    if w::inline_button(ui, "Try again", true).clicked() {
                        self.cloud_status = None;
                        self.start_cloud_op(ctx, CloudOp::List);
                    }
                });
                if settings {
                    self.open_settings(ctx);
                }
            } else {
                ui.label(RichText::new(msg).small().color(p.muted));
            }
        }

        if !self.cloud_files.is_empty() {
            ui.add_space(16.0);
            let used: u64 = self.cloud_files.iter().map(|f| f.size).sum();
            let mut line = format!(
                "{} file{} · {} of {} free tier",
                self.cloud_files.len(),
                if self.cloud_files.len() == 1 { "" } else { "s" },
                cloud::fmt_size(used),
                cloud::fmt_size(FREE_TIER_BYTES),
            );
            if let Some(ts) = self.cloud_refreshed {
                line.push_str(&format!(" · refreshed {}", fmt_time(ts)));
            }
            if let Some((bps, _)) = self.cloud_last_dl {
                line.push_str(&format!(" · last download {}", fmt_speed(bps)));
            }
            ui.label(RichText::new(line).small().color(p.muted));
            ui.label(
                RichText::new("Tip: drop PDFs anywhere on this page to upload them.")
                    .small()
                    .color(p.muted),
            );
        }
    }

    /// One file. Click: open it (downloading first if needed) — or, while
    /// selecting, tick it. The "…" menu holds everything else.
    fn cloud_row(
        &self,
        ui: &mut egui::Ui,
        e: &CloudEntry,
        selecting: bool,
        acts: &mut Vec<CloudAct>,
    ) {
        let p = w::pal(ui);
        let picked = e
            .bucket
            .as_ref()
            .is_some_and(|b| self.cloud_selected.contains(&b.0));
        let (glyph, sub) = match (&e.local, &e.bucket) {
            (Some((_, size)), Some(_)) => (
                "✔",
                format!("On this computer · {}", cloud::fmt_size(*size)),
            ),
            (Some((_, size)), None) => (
                "✔",
                format!("On this computer only · {}", cloud::fmt_size(*size)),
            ),
            (None, Some((_, _, size, uploaded))) => (
                "☁",
                if selecting {
                    format!(
                        "In your bucket · {} · {}",
                        cloud::fmt_size(*size),
                        fmt_ts(uploaded / 1000)
                    )
                } else {
                    format!(
                        "Click to download · {} · {}",
                        cloud::fmt_size(*size),
                        fmt_ts(uploaded / 1000)
                    )
                },
            ),
            (None, None) => ("☁", String::new()),
        };
        let resp = w::row(ui, true, Some(glyph), &e.name, Some(&sub), |ui| {
            if selecting {
                if let Some(b) = &e.bucket {
                    if w::check(ui, picked).clicked() {
                        acts.push(CloudAct::Toggle(b.0.clone()));
                    }
                }
                return;
            }
            ui.menu_button(RichText::new("…").size(16.0).color(p.muted), |ui| {
                ui.set_min_width(190.0);
                if let Some((path, _)) = &e.local {
                    if ui.button("Open").clicked() {
                        acts.push(CloudAct::Open(path.clone()));
                        ui.close();
                    }
                } else if let Some((name, _, size, _)) = &e.bucket {
                    if ui.button("Download").clicked() {
                        acts.push(CloudAct::Download(name.clone(), *size, false));
                        ui.close();
                    }
                    if ui.button("Download and open").clicked() {
                        acts.push(CloudAct::Download(name.clone(), *size, true));
                        ui.close();
                    }
                }
                if let Some((name, id, _, _)) = &e.bucket {
                    if ui.button("Rename…").clicked() {
                        acts.push(CloudAct::Rename(name.clone(), id.clone()));
                        ui.close();
                    }
                }
                if let Some((path, _)) = &e.local {
                    if ui
                        .button("Remove download")
                        .on_hover_text("Keeps it in your bucket")
                        .clicked()
                    {
                        acts.push(CloudAct::RemoveLocal(path.clone()));
                        ui.close();
                    }
                }
                if let Some((name, id, _, _)) = &e.bucket {
                    if ui.button("Delete from cloud…").clicked() {
                        acts.push(CloudAct::Delete(vec![(name.clone(), id.clone())]));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Select").clicked() {
                        acts.push(CloudAct::Toggle(name.clone()));
                        ui.close();
                    }
                }
            });
        });
        if resp.clicked() {
            match (selecting, &e.local, &e.bucket) {
                (true, _, Some(b)) => acts.push(CloudAct::Toggle(b.0.clone())),
                (true, _, None) => {}
                (false, Some((path, _)), _) => acts.push(CloudAct::Open(path.clone())),
                (false, None, Some((name, _, size, _))) => {
                    acts.push(CloudAct::Download(name.clone(), *size, true))
                }
                (false, None, None) => {}
            }
        }
        w::hairline(ui);
    }

    fn cloud_act(&mut self, ctx: &egui::Context, act: CloudAct) {
        match act {
            CloudAct::Open(path) => self.open_path(ctx, &path),
            CloudAct::Download(name, size, open) => {
                self.toast(format!("Downloading {name}"));
                self.start_cloud_op(ctx, CloudOp::Download(name, size, open));
            }
            CloudAct::RemoveLocal(path) => {
                let _ = std::fs::remove_file(&path);
                self.refresh_local_docs();
                self.toast("Removed from this computer; it's still in your bucket");
            }
            CloudAct::Rename(name, id) => {
                let draft = name.clone();
                self.cloud_dialog = Some(CloudDialog::Rename {
                    name,
                    id,
                    draft,
                    focus: true,
                });
            }
            CloudAct::Delete(items) => self.cloud_dialog = Some(CloudDialog::Delete { items }),
            CloudAct::Toggle(name) => {
                if !self.cloud_selected.remove(&name) {
                    self.cloud_selected.insert(name);
                }
            }
        }
    }

    /// Shown while selecting: what to do with the ticked files.
    fn selection_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        let n = self.cloud_selected.len();
        let all = self.cloud_files.len();
        ui.add_space(10.0);
        let mut select_all = false;
        let mut download = false;
        let mut delete = false;
        let mut cancel = false;
        egui::Frame::NONE
            .fill(p.accent_wash())
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(12, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{n} selected"))
                            .family(theme::medium())
                            .color(p.ink),
                    );
                    if n < all && w::inline_button(ui, "Select all", true).clicked() {
                        select_all = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::inline_button(ui, "Cancel", true).clicked() {
                            cancel = true;
                        }
                        if w::inline_button(ui, "Delete…", !self.cloud_busy).clicked() {
                            delete = true;
                        }
                        if w::inline_button(ui, "Download", true).clicked() {
                            download = true;
                        }
                    });
                });
            });
        if select_all {
            self.cloud_selected = self.cloud_files.iter().map(|f| f.name.clone()).collect();
        }
        if cancel {
            self.cloud_selected.clear();
        }
        if download {
            let items: Vec<(String, u64)> = self
                .cloud_files
                .iter()
                .filter(|f| self.cloud_selected.contains(&f.name))
                .filter(|f| !cloud::local_path_for(&f.name).exists())
                .map(|f| (f.name.clone(), f.size))
                .collect();
            let n = items.len();
            for (name, size) in items {
                self.start_cloud_op(ctx, CloudOp::Download(name, size, false));
            }
            self.toast(if n == 0 {
                "Those are all on this computer already".to_string()
            } else {
                format!("Downloading {n} document{}", if n == 1 { "" } else { "s" })
            });
            self.cloud_selected.clear();
        }
        if delete {
            let items: Vec<(String, String)> = self
                .cloud_files
                .iter()
                .filter(|f| self.cloud_selected.contains(&f.name))
                .map(|f| (f.name.clone(), f.file_id.clone()))
                .collect();
            if !items.is_empty() {
                self.cloud_dialog = Some(CloudDialog::Delete { items });
            }
        }
        ui.add_space(4.0);
    }

    /// Rename and delete confirmations.
    pub(super) fn cloud_dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.cloud_dialog.take() else {
            return;
        };
        let p = theme::palette(self.skin, self.theme.is_dark(ctx));
        match dialog {
            CloudDialog::Rename {
                name,
                id,
                mut draft,
                focus,
            } => {
                let mut done = false;
                let mut go = false;
                let wanted = with_pdf_extension(draft.trim());
                let clash = wanted != name && self.cloud_files.iter().any(|f| f.name == wanted);
                let valid =
                    !draft.trim().is_empty() && !draft.contains('/') && wanted != name && !clash;
                let modal = w::modal("cloud-rename", ctx).show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.label(
                        RichText::new("Rename")
                            .text_style(theme::headline())
                            .color(p.ink),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "It's renamed in your bucket, so your phone sees the new name too.",
                        )
                        .color(p.muted),
                    );
                    ui.add_space(12.0);
                    let edit = w::field(ui, &mut draft, "New name", false, 380.0);
                    if focus {
                        edit.request_focus();
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid {
                        go = true;
                    }
                    if clash {
                        ui.label(
                            RichText::new("There's already a file with that name.")
                                .small()
                                .color(ui.visuals().error_fg_color),
                        );
                    } else if draft.contains('/') {
                        ui.label(
                            RichText::new("Names can't contain “/”.")
                                .small()
                                .color(ui.visuals().error_fg_color),
                        );
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if w::primary_button(ui, "Rename", valid).clicked() {
                                go = true;
                            }
                            if w::outlined_button(ui, "Cancel", true).clicked() {
                                done = true;
                            }
                        });
                    });
                });
                if go {
                    self.toast(format!("Renaming to {wanted}…"));
                    self.start_cloud_op(ctx, CloudOp::Rename(name, id, wanted));
                } else if !done && !modal.should_close() {
                    self.cloud_dialog = Some(CloudDialog::Rename {
                        name,
                        id,
                        draft,
                        focus: false,
                    });
                }
            }
            CloudDialog::Delete { items } => {
                let mut done = false;
                let mut go = false;
                let n = items.len();
                let modal = w::modal("cloud-delete", ctx).show(ctx, |ui| {
                    ui.set_width(440.0);
                    let title = if n == 1 {
                        "Delete this file?".to_string()
                    } else {
                        format!("Delete {n} files?")
                    };
                    ui.label(
                        RichText::new(title)
                            .text_style(theme::headline())
                            .color(p.ink),
                    );
                    ui.add_space(6.0);
                    for (name, _) in items.iter().take(5) {
                        ui.label(RichText::new(format!("·  {name}")).color(p.ink));
                    }
                    if n > 5 {
                        ui.label(RichText::new(format!("…and {} more", n - 5)).color(p.muted));
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            "They're removed from your bucket for good — your phone won't see them \
                             either. Copies downloaded to this computer are removed too.",
                        )
                        .color(p.muted),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let delete = w::button(
                                ui,
                                RichText::new("Delete").family(theme::medium()),
                                w::Btn::new(w::Kind::Danger).min(0.0, 32.0),
                            );
                            if delete.clicked() {
                                go = true;
                            }
                            if w::outlined_button(ui, "Cancel", true).clicked() {
                                done = true;
                            }
                        });
                    });
                });
                if go {
                    for (name, _) in &items {
                        let _ = std::fs::remove_file(cloud::local_path_for(name));
                    }
                    self.refresh_local_docs();
                    self.cloud_selected.clear();
                    self.start_cloud_op(ctx, CloudOp::Delete(items));
                } else if !done && !modal.should_close() {
                    self.cloud_dialog = Some(CloudDialog::Delete { items });
                }
            }
        }
    }
}

/// A file as the Cloud page shows it: in the bucket, on this computer, or both.
struct CloudEntry {
    name: String,
    /// (bucket name, file id, size, uploaded ms)
    bucket: Option<(String, String, u64, i64)>,
    /// (downloaded copy, size)
    local: Option<(PathBuf, u64)>,
}

enum CloudAct {
    Open(PathBuf),
    Download(String, u64, bool),
    RemoveLocal(PathBuf),
    Rename(String, String),
    Delete(Vec<(String, String)>),
    Toggle(String),
}

pub(super) enum CloudDialog {
    Rename {
        name: String,
        id: String,
        draft: String,
        focus: bool,
    },
    Delete {
        items: Vec<(String, String)>,
    },
}

/// Keeps ".pdf" on a new name, adding it if the reader left it off.
fn with_pdf_extension(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".pdf") {
        name.to_string()
    } else {
        format!("{name}.pdf")
    }
}

/// The on-disk name a bucket file downloads to.
fn local_name(bucket_name: &str) -> String {
    cloud::local_path_for(bucket_name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A spinner, a line of text and, when known, progress — while covers or a
/// folder are being read in the background.
fn loading_block(ui: &mut egui::Ui, text: &str, progress: Option<(usize, usize)>) {
    let p = w::pal(ui);
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        spinner(ui, &p);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            let line = match progress {
                Some((done, total)) => format!("{text} {done} of {total}"),
                None => text.to_string(),
            };
            ui.label(RichText::new(line).color(p.ink));
            if let Some((done, total)) = progress {
                let frac = done as f32 / total.max(1) as f32;
                let shown = ui
                    .ctx()
                    .animate_value_with_time(ui.id().with("progress"), frac, 0.25);
                let (bar, _) = ui.allocate_exact_size(egui::vec2(260.0, 3.0), Sense::hover());
                ui.painter().rect_filled(bar, 2, p.hairline);
                let mut fill = bar;
                fill.set_width(bar.width() * shown);
                ui.painter().rect_filled(fill, 2, p.accent);
                ui.label(
                    RichText::new("Covers are saved, so this only happens the first time.")
                        .small()
                        .color(p.muted),
                );
            }
        });
    });
    ui.add_space(18.0);
}

/// A small rotating arc in the accent colour.
fn spinner(ui: &mut egui::Ui, p: &crate::ui::theme::Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    let start = t * 5.0;
    let points: Vec<egui::Pos2> = (0..=24)
        .map(|i| {
            let a = start + i as f32 / 24.0 * std::f32::consts::PI * 1.4;
            rect.center() + egui::vec2(a.cos(), a.sin()) * 8.0
        })
        .collect();
    ui.painter()
        .circle_stroke(rect.center(), 8.0, Stroke::new(2.0, p.hairline));
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(2.0, p.accent)));
    ui.ctx().request_repaint();
}

/// Small caption dividing the cloud list into on-device and bucket-only.
fn group_label(ui: &mut egui::Ui, text: &str, count: usize) {
    let p = w::pal(ui);
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(text.to_uppercase())
                .text_style(theme::label())
                .extra_letter_spacing(1.0)
                .color(p.muted),
        );
        ui.label(
            RichText::new(count.to_string())
                .text_style(theme::label())
                .color(p.muted),
        );
    });
    ui.add_space(2.0);
    w::hairline(ui);
}
