//! The Library: Recents as a shelf of first-page covers and, once a cloud
//! account is set up and switched on, a Cloud page beside it — the same two
//! tabs as the phone. The Cloud tab doesn't exist until an account does.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke};

use super::density::LibraryLayout;
use super::{
    fmt_speed, fmt_time, fmt_ts, folders, open_file_location, same_path, CloudFile, CloudOp,
    HomeScreen, LandingTab, ViewerApp, FREE_TIER_BYTES,
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

/// A document downloaded from the current cloud folder.
pub(super) struct LocalDoc {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
}

impl ViewerApp {
    pub(super) fn refresh_local_docs(&mut self) {
        self.cloud_local = cloud::local_docs(&self.cloud_current, &self.cloud_dir)
            .into_iter()
            .map(|(path, name, size)| LocalDoc { path, name, size })
            .collect();
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
    /// added, Cloud once an account is set up and switched on.
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
            LandingTab::Saved => self
                .current_account()
                .map(|a| a.summary())
                .unwrap_or_default(),
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
                "Open a PDF or an ebook with Ctrl+O, or drop files or a folder anywhere in this window"
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
            if w::primary_button(ui, "Open a file", true)
                .on_hover_text("Open a PDF or an ebook (Ctrl+O)")
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
                "Open a PDF or an ebook and it appears on this shelf, with its cover and the page you left off on.",
                Some(("Open a file", &mut open)),
            );
            if open {
                self.open_dialog(ctx);
            }
            return;
        }
        ui.add_space(22.0);
        // Ask about every entry in the background: deleted or renamed files
        // drop off the shelf by themselves (see ViewerApp::ui).
        for i in 0..self.recents.len() {
            let path = self.recents[i].path.clone();
            self.covers.check(ctx, &path);
        }
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
        if let Some(p) = open.filter(|p| self.available(ctx, p)) {
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
        for i in 0..self.books.len() {
            let path = self.books[i].path.clone();
            self.covers.check(ctx, &path);
        }
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
        if let Some(path) = open.filter(|p| self.available(ctx, p)) {
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
    /// Whether a shelf file can be opened; says why not if it can't.
    fn available(&mut self, ctx: &egui::Context, path: &std::path::Path) -> bool {
        if self.covers.exists(ctx, path) {
            return true;
        }
        self.toast("That file isn't available. Is its drive connected?");
        false
    }

    fn caption(&self, tile: &Tile, exists: bool) -> String {
        if !exists {
            // Deleted or renamed files leave the shelf on their own; what's
            // left is usually on a drive that isn't connected.
            return "Not available — is its drive connected?".to_string();
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
            let (resp, thumb) =
                w::row_lead(ui, true, egui::vec2(31.0, 40.0), &name, Some(&meta), |_| {});
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
            let upload_ok = exists && self.can_upload(path);
            let mut upload = false;
            resp.on_hover_text(path.display().to_string())
                .context_menu(|ui| {
                    if exists && ui.button("Open").clicked() {
                        open_req = Some(tile.path.clone());
                        ui.close();
                    }
                    if upload_ok && ui.button("Save to cloud…").clicked() {
                        upload = true;
                        ui.close();
                    }
                    if exists && ui.button("Show in folder").clicked() {
                        open_file_location(path);
                        ui.close();
                    }
                    if tile.removable && ui.button("Remove from library").clicked() {
                        remove_req = Some(i);
                        ui.close();
                    }
                });
            if upload {
                self.ask_upload(ctx, tile.path.clone());
            }
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
            // Clickable even when unavailable: the right-click menu still
            // offers to remove it.
            let (slot, resp) = ui.allocate_exact_size(cover_size, Sense::click());
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
                let cover = exists.then(|| self.covers.get(ctx, path)).flatten();
                // Still being drawn in the background (big PDFs take a moment
                // the first time; after that the cover comes from disk).
                let loading = exists && cover.is_none();
                match cover {
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
                    _ if loading => {
                        // A slow, calm pulse while the cover is on its way.
                        let t = ui.input(|i| i.time) as f32;
                        let pulse = (t * 2.4).sin() * 0.5 + 0.5;
                        painter.rect_filled(
                            rect,
                            0,
                            theme::Palette::mix(p.surface, p.hover(), 0.35 + 0.65 * pulse),
                        );
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(33));
                    }
                    _ => {
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            if exists { "PDF" } else { "Unavailable" },
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
            let upload_ok = exists && self.can_upload(path);
            let mut upload = false;
            resp.on_hover_text(path.display().to_string())
                .context_menu(|ui| {
                    if exists && ui.button("Open").clicked() {
                        open = true;
                        ui.close();
                    }
                    if upload_ok && ui.button("Save to cloud…").clicked() {
                        upload = true;
                        ui.close();
                    }
                    if exists && ui.button("Show in folder").clicked() {
                        open_file_location(path);
                        ui.close();
                    }
                    if tile.removable && ui.button("Remove from library").clicked() {
                        remove = true;
                        ui.close();
                    }
                });
            if upload {
                self.ask_upload(ctx, tile.path.clone());
            }

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
        let Some(acct) = self.current_account().cloned() else {
            return;
        };
        // Auto-list once (an error stops retries until Refresh).
        if !self.cloud_listed && !self.cloud_busy && self.cloud_status.is_none() {
            self.start_cloud_op(ctx, CloudOp::List(self.cloud_dir.clone()));
        }

        let mut refresh = false;
        let mut upload = false;
        let mut upload_dir = false;
        let mut new_folder = false;
        let busy = self.cloud_busy;
        w::section_heading(ui, "Saved to cloud", |ui| {
            if w::inline_button(ui, "Refresh", !busy).clicked() {
                refresh = true;
            }
            if w::inline_button(ui, "New folder…", true).clicked() {
                new_folder = true;
            }
            if w::inline_button(ui, "Upload folder…", true)
                .on_hover_text(
                    "Every PDF and ebook in a folder and its subfolders, keeping the subfolders",
                )
                .clicked()
            {
                upload_dir = true;
            }
            if w::inline_button(ui, "Upload files…", true).clicked() {
                upload = true;
            }
        });
        if upload_dir {
            if let Some(dir) = rfd::FileDialog::new()
                .set_title("Upload a folder")
                .pick_folder()
            {
                self.upload_folder(ctx, dir);
            }
        }
        self.cloud_path_bar(ui, &acct);
        if refresh {
            self.cloud_listed = false;
            self.cloud_status = None;
            self.start_cloud_op(ctx, CloudOp::List(self.cloud_dir.clone()));
        }
        if new_folder {
            self.cloud_dialog = Some(CloudDialog::NewFolder {
                draft: String::new(),
                focus: true,
            });
        }
        if upload {
            let mut exts = vec!["pdf"];
            exts.extend_from_slice(crate::ebook::EXTENSIONS);
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter("Documents", &exts)
                .pick_files()
            {
                let n = paths.len();
                if n == 1 {
                    // One file: let it be named first.
                    self.ask_upload(ctx, paths[0].clone());
                } else {
                    for path in paths {
                        let op = self.upload_here(path);
                        self.start_cloud_op(ctx, op);
                    }
                    self.toast(format!(
                        "Uploading {n} document{}",
                        if n == 1 { "" } else { "s" }
                    ));
                }
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
            let secs = self
                .cloud_op_started
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            let mut text = self
                .cloud_status
                .clone()
                .unwrap_or_else(|| "Connecting…".into());
            if secs >= 2 {
                text.push_str(&format!("  {secs} s"));
            }
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(text).color(p.muted));
            });
        }

        // Selecting turns every row's "…" into a checkbox.
        let selecting = !self.cloud_selected.is_empty();
        if selecting {
            self.selection_bar(ui, ctx);
        }

        // Folders, then documents split by where they actually are.
        let dirs: Vec<CloudFile> = self
            .cloud_files
            .iter()
            .filter(|f| f.is_dir)
            .cloned()
            .collect();
        let downloaded: Vec<CloudEntry> = self
            .cloud_local
            .iter()
            .map(|doc| CloudEntry {
                name: doc.name.clone(),
                remote: self
                    .cloud_files
                    .iter()
                    .find(|f| !f.is_dir && cloud::local_name(&f.name) == doc.name)
                    .cloned(),
                local: Some((doc.path.clone(), doc.size)),
            })
            .collect();
        let local_names: std::collections::HashSet<&str> =
            self.cloud_local.iter().map(|d| d.name.as_str()).collect();
        let remote_only: Vec<CloudEntry> = self
            .cloud_files
            .iter()
            .filter(|f| !f.is_dir && !local_names.contains(cloud::local_name(&f.name).as_str()))
            .map(|f| CloudEntry {
                name: f.name.clone(),
                remote: Some(f.clone()),
                local: None,
            })
            .collect();

        let mut acts: Vec<CloudAct> = Vec::new();
        if !dirs.is_empty() {
            group_label(ui, "Folders", dirs.len());
            for d in &dirs {
                let resp = w::row(ui, true, Some("🗀"), &d.name, None, |_| {});
                if resp.clicked() && !selecting {
                    acts.push(CloudAct::Enter(d.path.clone()));
                }
                w::hairline(ui);
            }
        }
        if !downloaded.is_empty() {
            let label = if acct.kind == cloud::Kind::Folder {
                "Copies kept offline"
            } else {
                "On this computer"
            };
            group_label(ui, label, downloaded.len());
            for e in &downloaded {
                self.cloud_row(ui, &acct, e, selecting, &mut acts);
            }
        }
        if !remote_only.is_empty() {
            group_label(ui, &format!("In {}", acct.name), remote_only.len());
            for e in &remote_only {
                self.cloud_row(ui, &acct, e, selecting, &mut acts);
            }
        }
        for act in acts {
            self.cloud_act(ctx, &acct, act);
        }

        if self.cloud_listed && self.cloud_files.is_empty() && self.cloud_local.is_empty() && !busy
        {
            if self.cloud_dir.is_empty() {
                w::empty_state(
                    ui,
                    &format!("Nothing in {} yet", acct.name),
                    "Open a document and choose ☰ > Save to cloud, drop documents onto this page, or upload above.",
                    None,
                );
            } else {
                w::empty_state(
                    ui,
                    "This folder is empty",
                    "Drop documents onto this page or upload above to put them here.",
                    None,
                );
            }
        }

        if let Some(msg) = self.cloud_status.clone().filter(|_| !busy) {
            ui.add_space(12.0);
            if self.cloud_files.is_empty() && !self.cloud_listed {
                // Nothing listed at all: this is the reason, so say it plainly.
                let lower = msg.to_ascii_lowercase();
                let auth = ["authoriz", "accept", "refused", "401", "403", "password"]
                    .iter()
                    .any(|k| lower.contains(k));
                let headline = if auth {
                    format!("{} didn't let PaperView in", acct.name)
                } else {
                    format!("Couldn't reach {}", acct.name)
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
                    if w::inline_button(ui, "Check the account in Settings", true).clicked() {
                        settings = true;
                    }
                    if w::inline_button(ui, "Try again", true).clicked() {
                        self.cloud_status = None;
                        self.start_cloud_op(ctx, CloudOp::List(self.cloud_dir.clone()));
                    }
                });
                if settings {
                    self.open_settings(ctx);
                }
            } else {
                ui.label(RichText::new(msg).small().color(p.muted));
            }
        }

        let files: Vec<&CloudFile> = self.cloud_files.iter().filter(|f| !f.is_dir).collect();
        if !files.is_empty() {
            ui.add_space(16.0);
            let used: u64 = files.iter().map(|f| f.size).sum();
            let mut line = format!(
                "{} file{} · {}",
                files.len(),
                if files.len() == 1 { "" } else { "s" },
                cloud::fmt_size(used),
            );
            if acct.kind == cloud::Kind::B2 && self.cloud_dir.is_empty() {
                line.push_str(&format!(
                    " of {} free tier",
                    cloud::fmt_size(FREE_TIER_BYTES)
                ));
            }
            if let Some(ts) = self.cloud_refreshed {
                line.push_str(&format!(" · refreshed {}", fmt_time(ts)));
            }
            if let Some((bps, _)) = self.cloud_last_dl {
                line.push_str(&format!(" · last download {}", fmt_speed(bps)));
            }
            ui.label(RichText::new(line).small().color(p.muted));
            ui.label(
                RichText::new(
                    "Tip: drop documents or whole folders anywhere on this page to upload them here.",
                )
                    .small()
                    .color(p.muted),
            );
        }
    }

    /// Where on the account we are — "NAS › Math › Calculus", each step
    /// clickable — and, with more than one account, a switch between them.
    fn cloud_path_bar(&mut self, ui: &mut egui::Ui, acct: &cloud::Account) {
        let p = w::pal(ui);
        let mut go: Option<String> = None;
        let mut switch: Option<String> = None;
        let others: Vec<(String, String)> = self
            .cloud_accounts
            .iter()
            .filter(|a| a.ready())
            .map(|a| (a.id.clone(), format!("{}  {}", a.kind.glyph(), a.name)))
            .collect();
        // Connected (it listed), not connected (it failed), or still trying.
        let state = if self.cloud_listed {
            Some(("✔ Connected", p.accent))
        } else if !self.cloud_busy && self.cloud_status.is_some() {
            Some(("Not connected", ui.visuals().error_fg_color))
        } else {
            None
        };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.label(RichText::new(acct.kind.glyph()).color(p.muted));
            ui.add_space(4.0);
            let parts: Vec<&str> = self
                .cloud_dir
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();
            let crumb = |ui: &mut egui::Ui, text: &str, last: bool| -> bool {
                if last {
                    ui.label(RichText::new(text).family(theme::medium()).color(p.ink));
                    false
                } else {
                    w::inline_button(ui, text, true).clicked()
                }
            };
            if crumb(ui, &acct.name, parts.is_empty()) {
                go = Some(String::new());
            }
            for (i, part) in parts.iter().enumerate() {
                ui.label(RichText::new("›").color(p.muted));
                if crumb(ui, part, i + 1 == parts.len()) {
                    go = Some(parts[..=i].join("/"));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if others.len() > 1 {
                    egui::ComboBox::from_id_salt("cloud-account")
                        .selected_text(RichText::new("Switch account").small())
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (id, label) in &others {
                                if ui.selectable_label(*id == acct.id, label).clicked() {
                                    switch = Some(id.clone());
                                }
                            }
                        });
                    ui.add_space(12.0);
                }
                if let Some((text, color)) = state {
                    ui.label(RichText::new(text).small().color(color));
                }
            });
        });
        ui.add_space(2.0);
        if let Some(id) = switch.filter(|id| *id != acct.id) {
            self.switch_account(id);
        } else if let Some(dir) = go {
            self.enter_cloud_dir(dir);
        }
    }

    /// One document. Click: open it (downloading first if needed, straight
    /// from the folder for Folder accounts) — or, while selecting, tick it.
    /// The "…" menu holds everything else.
    fn cloud_row(
        &self,
        ui: &mut egui::Ui,
        acct: &cloud::Account,
        e: &CloudEntry,
        selecting: bool,
        acts: &mut Vec<CloudAct>,
    ) {
        let p = w::pal(ui);
        let in_place = acct.local_root();
        let picked = e
            .remote
            .as_ref()
            .is_some_and(|f| self.cloud_selected.contains(&f.path));
        let when = |f: &CloudFile| {
            if f.modified_ms > 0 {
                format!(" · {}", fmt_ts(f.modified_ms / 1000))
            } else {
                String::new()
            }
        };
        let (glyph, sub) = match (&e.local, &e.remote) {
            (Some((_, size)), Some(_)) => (
                "✔",
                format!("On this computer · {}", cloud::fmt_size(*size)),
            ),
            (Some((_, size)), None) => (
                "✔",
                format!("On this computer only · {}", cloud::fmt_size(*size)),
            ),
            (None, Some(f)) if in_place.is_some() => (
                "🗋",
                format!(
                    "Opens from {} · {}{}",
                    acct.name,
                    cloud::fmt_size(f.size),
                    when(f)
                ),
            ),
            (None, Some(f)) => (
                "☁",
                if selecting {
                    format!("In {} · {}{}", acct.name, cloud::fmt_size(f.size), when(f))
                } else {
                    format!("Click to download · {}{}", cloud::fmt_size(f.size), when(f))
                },
            ),
            (None, None) => ("☁", String::new()),
        };
        let in_place_path = |f: &CloudFile| {
            in_place
                .as_ref()
                .map(|root| root.join(f.path.replace('/', std::path::MAIN_SEPARATOR_STR)))
        };
        let resp = w::row(ui, true, Some(glyph), &e.name, Some(&sub), |ui| {
            if selecting {
                if let Some(f) = &e.remote {
                    if w::check(ui, picked).clicked() {
                        acts.push(CloudAct::Toggle(f.path.clone()));
                    }
                }
                return;
            }
            ui.menu_button(RichText::new("…").size(16.0).color(p.muted), |ui| {
                ui.set_min_width(200.0);
                if let Some((path, _)) = &e.local {
                    if ui.button("Open").clicked() {
                        acts.push(CloudAct::Open(path.clone()));
                        ui.close();
                    }
                } else if let Some(f) = &e.remote {
                    if let Some(path) = in_place_path(f) {
                        if ui.button("Open").clicked() {
                            acts.push(CloudAct::Open(path.clone()));
                            ui.close();
                        }
                        if ui.button("Show in folder").clicked() {
                            acts.push(CloudAct::ShowInFolder(path));
                            ui.close();
                        }
                        if ui
                            .button("Keep a copy offline")
                            .on_hover_text(
                                "Copies it to this computer, for when the share isn't there",
                            )
                            .clicked()
                        {
                            acts.push(CloudAct::Download(f.path.clone(), f.size, false));
                            ui.close();
                        }
                    } else {
                        if ui.button("Download").clicked() {
                            acts.push(CloudAct::Download(f.path.clone(), f.size, false));
                            ui.close();
                        }
                        if ui.button("Download and open").clicked() {
                            acts.push(CloudAct::Download(f.path.clone(), f.size, true));
                            ui.close();
                        }
                    }
                }
                if let Some(f) = &e.remote {
                    if ui.button("Rename…").clicked() {
                        acts.push(CloudAct::Rename(f.path.clone(), f.id.clone()));
                        ui.close();
                    }
                }
                if let Some((path, _)) = &e.local {
                    let label = if in_place.is_some() {
                        "Remove offline copy"
                    } else {
                        "Remove download"
                    };
                    if ui
                        .button(label)
                        .on_hover_text(format!("Keeps it in {}", acct.name))
                        .clicked()
                    {
                        acts.push(CloudAct::RemoveLocal(path.clone()));
                        ui.close();
                    }
                }
                if let Some(f) = &e.remote {
                    if ui.button(format!("Delete from {}…", acct.name)).clicked() {
                        acts.push(CloudAct::Delete(vec![(f.path.clone(), f.id.clone())]));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Select").clicked() {
                        acts.push(CloudAct::Toggle(f.path.clone()));
                        ui.close();
                    }
                }
            });
        });
        if resp.clicked() {
            match (selecting, &e.local, &e.remote) {
                (true, _, Some(f)) => acts.push(CloudAct::Toggle(f.path.clone())),
                (true, _, None) => {}
                (false, Some((path, _)), _) => acts.push(CloudAct::Open(path.clone())),
                (false, None, Some(f)) => match in_place_path(f) {
                    Some(path) => acts.push(CloudAct::Open(path)),
                    None => acts.push(CloudAct::Download(f.path.clone(), f.size, true)),
                },
                (false, None, None) => {}
            }
        }
        w::hairline(ui);
    }

    fn cloud_act(&mut self, ctx: &egui::Context, acct: &cloud::Account, act: CloudAct) {
        match act {
            CloudAct::Open(path) => self.open_path(ctx, &path),
            CloudAct::ShowInFolder(path) => open_file_location(&path),
            CloudAct::Enter(dir) => self.enter_cloud_dir(dir),
            CloudAct::Download(path, size, open) => {
                self.toast(format!("Downloading {}", cloud::last_part(&path)));
                self.start_cloud_op(ctx, CloudOp::Download { path, size, open });
            }
            CloudAct::RemoveLocal(path) => {
                let _ = std::fs::remove_file(&path);
                self.refresh_local_docs();
                self.toast(format!(
                    "Removed from this computer; it's still in {}",
                    acct.name
                ));
            }
            CloudAct::Rename(path, id) => {
                let draft = cloud::last_part(&path).to_string();
                self.cloud_dialog = Some(CloudDialog::Rename {
                    path,
                    id,
                    draft,
                    focus: true,
                });
            }
            CloudAct::Delete(items) => self.cloud_dialog = Some(CloudDialog::Delete { items }),
            CloudAct::Toggle(path) => {
                if !self.cloud_selected.remove(&path) {
                    self.cloud_selected.insert(path);
                }
            }
        }
    }

    /// Shown while selecting: what to do with the ticked files.
    fn selection_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        let n = self.cloud_selected.len();
        let all = self.cloud_files.iter().filter(|f| !f.is_dir).count();
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
            self.cloud_selected = self
                .cloud_files
                .iter()
                .filter(|f| !f.is_dir)
                .map(|f| f.path.clone())
                .collect();
        }
        if cancel {
            self.cloud_selected.clear();
        }
        if download {
            let acct = self.cloud_current.clone();
            let items: Vec<(String, u64)> = self
                .cloud_files
                .iter()
                .filter(|f| !f.is_dir && self.cloud_selected.contains(&f.path))
                .filter(|f| !cloud::local_path_for(&acct, &f.path).exists())
                .map(|f| (f.path.clone(), f.size))
                .collect();
            let n = items.len();
            for (path, size) in items {
                self.start_cloud_op(
                    ctx,
                    CloudOp::Download {
                        path,
                        size,
                        open: false,
                    },
                );
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
                .filter(|f| !f.is_dir && self.cloud_selected.contains(&f.path))
                .map(|f| (f.path.clone(), f.id.clone()))
                .collect();
            if !items.is_empty() {
                self.cloud_dialog = Some(CloudDialog::Delete { items });
            }
        }
        ui.add_space(4.0);
    }

    /// Whether a local document can be offered for upload: an account is set
    /// up, and the file didn't come from it in the first place (a download,
    /// or a file in a Folder account's own folder).
    pub(super) fn can_upload(&self, path: &Path) -> bool {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.cloud_active()
            && cloud::is_document(&name)
            && !path.starts_with(cloud::cache_dir())
            && !self
                .current_account()
                .is_some_and(|a| a.contains_local(path))
    }

    /// Asks what to call a document in the cloud, then uploads it into the
    /// folder being shown.
    pub(super) fn ask_upload(&mut self, ctx: &egui::Context, path: PathBuf) {
        if !self.cloud_active() {
            return;
        }
        let draft = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.cloud_dialog = Some(CloudDialog::Upload {
            path,
            draft,
            focus: true,
        });
        // The folder's listing, to warn before replacing a file.
        if !self.cloud_listed && !self.cloud_busy {
            self.start_cloud_op(ctx, CloudOp::List(self.cloud_dir.clone()));
        }
    }

    /// Rename, upload, new-folder and delete dialogs.
    pub(super) fn cloud_dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.cloud_dialog.take() else {
            return;
        };
        let p = theme::palette(self.skin, self.theme.is_dark(ctx));
        let place = self.account_name();
        let names_here: Vec<String> = self.cloud_files.iter().map(|f| f.name.clone()).collect();
        let error = |ui: &mut egui::Ui, text: &str| {
            ui.label(
                RichText::new(text)
                    .small()
                    .color(ui.visuals().error_fg_color),
            );
        };
        match dialog {
            CloudDialog::Rename {
                path,
                id,
                mut draft,
                focus,
            } => {
                let mut done = false;
                let mut go = false;
                let old = cloud::last_part(&path).to_string();
                let wanted = with_extension(draft.trim(), extension(&old));
                let clash = wanted != old && names_here.contains(&wanted);
                let valid =
                    !draft.trim().is_empty() && !draft.contains('/') && wanted != old && !clash;
                let modal = w::modal("cloud-rename", ctx).show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.label(
                        RichText::new("Rename")
                            .text_style(theme::headline())
                            .color(p.ink),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "It's renamed in {place}, so your other devices see the new name too."
                        ))
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
                        error(ui, "There's already something with that name here.");
                    } else if draft.contains('/') {
                        error(ui, "Names can't contain “/”.");
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
                    let to = cloud::join(cloud::parent(&path), &wanted);
                    self.toast(format!("Renaming to {wanted}…"));
                    self.start_cloud_op(ctx, CloudOp::Rename { from: path, id, to });
                } else if !done && !modal.should_close() {
                    self.cloud_dialog = Some(CloudDialog::Rename {
                        path,
                        id,
                        draft,
                        focus: false,
                    });
                }
            }
            CloudDialog::Upload {
                path,
                mut draft,
                focus,
            } => {
                let mut done = false;
                let mut go = false;
                let local_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let wanted = with_extension(draft.trim(), extension(&local_name));
                let clash = names_here.contains(&wanted);
                let valid = !draft.trim().is_empty() && !draft.contains('/');
                let where_ = match self.cloud_dir.as_str() {
                    "" => place.clone(),
                    dir => format!("{place} › {}", dir.replace('/', " › ")),
                };
                let modal = w::modal("cloud-upload", ctx).show(ctx, |ui| {
                    ui.set_width(440.0);
                    ui.label(
                        RichText::new("Save to cloud")
                            .text_style(theme::headline())
                            .color(p.ink),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "It's saved in {where_} under this name, and your other devices see \
                             the same name. Change it, or just upload."
                        ))
                        .color(p.muted),
                    );
                    ui.add_space(12.0);
                    let edit = w::field(ui, &mut draft, "Name", false, 400.0);
                    if focus {
                        edit.request_focus();
                    }
                    w::select_all_on_focus(ui, &edit, &draft);
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid {
                        go = true;
                    }
                    ui.add_space(2.0);
                    if draft.contains('/') {
                        error(ui, "Names can't contain “/”.");
                    } else if clash {
                        ui.label(
                            RichText::new(format!(
                                "There's already a “{wanted}” here. Uploading replaces it."
                            ))
                            .small()
                            .color(p.accent),
                        );
                    } else if valid {
                        ui.label(
                            RichText::new(format!("Saved as “{wanted}”"))
                                .small()
                                .color(p.muted),
                        );
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let label = if clash { "Replace" } else { "Upload" };
                            if w::primary_button(ui, label, valid).clicked() {
                                go = true;
                            }
                            if w::outlined_button(ui, "Cancel", true).clicked() {
                                done = true;
                            }
                        });
                    });
                });
                if go {
                    self.toast(format!("Uploading {wanted} to {place}…"));
                    let target = cloud::join(&self.cloud_dir, &wanted);
                    self.start_cloud_op(ctx, CloudOp::Upload(path, target));
                } else if !done && !modal.should_close() {
                    self.cloud_dialog = Some(CloudDialog::Upload {
                        path,
                        draft,
                        focus: false,
                    });
                }
            }
            CloudDialog::NewFolder { mut draft, focus } => {
                let mut done = false;
                let mut go = false;
                let name = draft.trim().to_string();
                let clash = names_here.contains(&name);
                let valid =
                    !name.is_empty() && !name.contains('/') && !name.starts_with('.') && !clash;
                let modal = w::modal("cloud-new-folder", ctx).show(ctx, |ui| {
                    ui.set_width(400.0);
                    ui.label(
                        RichText::new("New folder")
                            .text_style(theme::headline())
                            .color(p.ink),
                    );
                    ui.add_space(12.0);
                    let edit = w::field(ui, &mut draft, "Folder name", false, 360.0);
                    if focus {
                        edit.request_focus();
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid {
                        go = true;
                    }
                    if clash {
                        error(ui, "There's already something with that name here.");
                    } else if name.contains('/') {
                        error(ui, "Names can't contain “/”.");
                    } else if name.starts_with('.') {
                        error(ui, "Names starting with “.” would be hidden.");
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if w::primary_button(ui, "Make folder", valid).clicked() {
                                go = true;
                            }
                            if w::outlined_button(ui, "Cancel", true).clicked() {
                                done = true;
                            }
                        });
                    });
                });
                if go {
                    let path = cloud::join(&self.cloud_dir, &name);
                    self.start_cloud_op(ctx, CloudOp::MakeFolder(path));
                } else if !done && !modal.should_close() {
                    self.cloud_dialog = Some(CloudDialog::NewFolder {
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
                    for (path, _) in items.iter().take(5) {
                        ui.label(
                            RichText::new(format!("·  {}", cloud::last_part(path))).color(p.ink),
                        );
                    }
                    if n > 5 {
                        ui.label(RichText::new(format!("…and {} more", n - 5)).color(p.muted));
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "They're removed from {place} for good — your other devices won't \
                             see them either. Copies downloaded to this computer are removed too."
                        ))
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
                    for (path, _) in &items {
                        let _ =
                            std::fs::remove_file(cloud::local_path_for(&self.cloud_current, path));
                    }
                    self.refresh_local_docs();
                    self.cloud_selected.clear();
                    self.start_cloud_op(ctx, CloudOp::Delete(items));
                } else if !done && !modal.should_close() {
                    self.cloud_dialog = Some(CloudDialog::Delete { items });
                }
            }
            CloudDialog::RemoveAccount { id } => {
                let mut done = false;
                let mut go = false;
                let name = self
                    .cloud_accounts
                    .iter()
                    .find(|a| a.id == id)
                    .map(|a| a.name.clone())
                    .unwrap_or_default();
                let modal = w::modal("cloud-remove-account", ctx).show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.label(
                        RichText::new(format!("Remove {name}?"))
                            .text_style(theme::headline())
                            .color(p.ink),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "PaperView forgets this account and its saved password, and removes \
                             what it downloaded from it. Nothing is deleted from the account itself.",
                        )
                        .color(p.muted),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let remove = w::button(
                                ui,
                                RichText::new("Remove").family(theme::medium()),
                                w::Btn::new(w::Kind::Danger).min(0.0, 32.0),
                            );
                            if remove.clicked() {
                                go = true;
                            }
                            if w::outlined_button(ui, "Cancel", true).clicked() {
                                done = true;
                            }
                        });
                    });
                });
                if go {
                    self.remove_account(&id);
                } else if !done && !modal.should_close() {
                    self.cloud_dialog = Some(CloudDialog::RemoveAccount { id });
                }
            }
        }
    }
}

/// A document as the Cloud page shows it: in the account, on this
/// computer, or both.
struct CloudEntry {
    name: String,
    remote: Option<CloudFile>,
    /// (downloaded copy, size)
    local: Option<(PathBuf, u64)>,
}

enum CloudAct {
    Open(PathBuf),
    ShowInFolder(PathBuf),
    Enter(String),
    /// (path, size, open after)
    Download(String, u64, bool),
    RemoveLocal(PathBuf),
    /// (path, file id)
    Rename(String, String),
    Delete(Vec<(String, String)>),
    Toggle(String),
}

pub(super) enum CloudDialog {
    Rename {
        path: String,
        id: String,
        draft: String,
        focus: bool,
    },
    Delete {
        items: Vec<(String, String)>,
    },
    /// Name a local document before it's uploaded.
    Upload {
        path: PathBuf,
        draft: String,
        focus: bool,
    },
    NewFolder {
        draft: String,
        focus: bool,
    },
    RemoveAccount {
        id: String,
    },
}

/// A file name's extension, without the dot ("" when there is none).
fn extension(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext,
        _ => "",
    }
}

/// Keeps the document's extension on a new name, adding it if the reader
/// left it off.
fn with_extension(name: &str, ext: &str) -> String {
    if ext.is_empty()
        || name
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", ext.to_ascii_lowercase()))
    {
        name.to_string()
    } else {
        format!("{name}.{ext}")
    }
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
