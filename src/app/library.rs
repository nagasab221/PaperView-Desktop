//! The Library: Recents as a shelf of first-page covers and, once a Backblaze
//! bucket is set up and switched on, a Cloud page beside it — the same two
//! tabs as the phone. The Cloud tab doesn't exist until the bucket does.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke};

use super::{
    folders, fmt_speed, fmt_time, fmt_ts, open_file_location, CloudOp, HomeScreen, LandingTab,
    ViewerApp, FREE_TIER_BYTES,
};
use crate::cloud;
use crate::covers::Cover;
use crate::ui::{theme, widgets as w};

const TILE_W: f32 = 150.0;
const TILE_GAP: f32 = 24.0;
/// Cover aspect (width / height), as on the phone.
const COVER_ASPECT: f32 = 0.77;
const COLUMN_MAX: f32 = 1080.0;

/// One document on a shelf.
struct Tile {
    path: PathBuf,
    meta: String,
    /// Recents can be removed from the Library; folder files can't.
    removable: bool,
}

/// A date for shelf captions, e.g. "13 Nov 2026".
fn fmt_date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|utc| utc.with_timezone(&chrono::Local).format("%-d %b %Y").to_string())
        .unwrap_or_default()
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
                        LocalDoc { path: e.path(), name, size: meta.len() },
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
            .id_salt(("home", self.home, self.landing_tab))
            .show(ui, |ui| {
                // Switching screens or tabs fades the new one in.
                w::fade_in(ui, (self.home, self.landing_tab));
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
                "Open a PDF with Ctrl+O, or drop a PDF or a folder anywhere in this window".to_string()
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
            if w::primary_button(ui, "Open a PDF", true).on_hover_text("Ctrl+O").clicked() {
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
            let sel = tabs.iter().position(|(_, t)| *t == self.landing_tab).unwrap_or(0);
            if let Some(i) = w::text_tabs(ui, &labels, sel) {
                self.landing_tab = tabs[i].1;
                if self.landing_tab == LandingTab::Folders {
                    self.rescan_folders();
                }
            }
        } else {
            ui.add_space(10.0);
        }

        match self.landing_tab {
            LandingTab::Saved => self.cloud_page(ui, &ctx),
            LandingTab::Folders => self.folders_page(ui, &ctx),
            LandingTab::Recent => self.recents_shelf(ui, &ctx),
        }
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
                meta: format!("page {} · {}", r.page + 1, fmt_date(r.ts)),
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

    // ── Folders ──────────────────────────────────────────────────────────────

    fn folders_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        // Folders change behind our back; re-read them now and then while shown.
        if self.folders_scanned.is_none_or(|t| t.elapsed() >= folders::RESCAN_EVERY) {
            self.rescan_folders();
        }
        ctx.request_repaint_after(folders::RESCAN_EVERY);

        let mut open: Option<PathBuf> = None;
        let mut remove: Option<PathBuf> = None;
        let mut refresh = false;
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
            ui.label(RichText::new(root.display().to_string()).small().color(p.muted));
            if listing.missing {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("This folder can't be found. Was it moved, or is its drive unplugged?")
                        .color(ui.visuals().error_fg_color),
                );
                continue;
            }
            if listing.docs.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("No PDFs in this folder yet.").color(p.muted));
                continue;
            }
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
                        meta: self.reading_note(&d.path, d.modified),
                        path: d.path.clone(),
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
                    RichText::new("Only the first 3000 PDFs are shown.").small().color(p.muted),
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

    /// "page 12" if the document has been read, otherwise its file date.
    fn reading_note(&self, path: &std::path::Path, modified: std::time::SystemTime) -> String {
        if let Some(r) = self.recents.iter().find(|r| r.path == path) {
            return format!("page {} · {}", r.page + 1, fmt_date(r.ts));
        }
        let ts = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        fmt_date(ts)
    }

    // ── Shelf ────────────────────────────────────────────────────────────────

    /// Tiles in rows. Returns the tile clicked, and the index of one removed.
    fn shelf(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tiles: &[Tile],
    ) -> (Option<PathBuf>, Option<usize>) {
        let width = ui.available_width();
        let cols = (((width + TILE_GAP) / (TILE_W + TILE_GAP)).floor() as usize).max(1);
        let tile_w = ((width - TILE_GAP * (cols as f32 - 1.0)) / cols as f32).min(TILE_W + 30.0);
        let mut open_req = None;
        let mut remove_req = None;
        for (row, chunk) in tiles.chunks(cols).enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = TILE_GAP;
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
        let exists = path.exists();
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut open = false;
        let mut remove = false;

        ui.vertical(|ui| {
            ui.set_width(tile_w);
            let cover_size = egui::vec2(tile_w, tile_w / COVER_ASPECT);
            let (slot, resp) = ui.allocate_exact_size(
                cover_size,
                if exists { Sense::click() } else { Sense::hover() },
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
                        // Fill the tile, cropping from the top like the phone does.
                        let [tw, th] = tex.size().map(|v| v as f32);
                        let tex_aspect = tw / th.max(1.0);
                        let uv = if tex_aspect < COVER_ASPECT {
                            egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(1.0, tex_aspect / COVER_ASPECT),
                            )
                        } else {
                            let f = COVER_ASPECT / tex_aspect;
                            egui::Rect::from_min_max(
                                egui::pos2((1.0 - f) * 0.5, 0.0),
                                egui::pos2((1.0 + f) * 0.5, 1.0),
                            )
                        };
                        painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
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
            resp.on_hover_text(path.display().to_string()).context_menu(|ui| {
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
            ui.add(
                egui::Label::new(
                    RichText::new(&name)
                        .family(theme::medium())
                        .size(14.0)
                        .color(if exists { p.ink } else { p.muted }),
                )
                .wrap()
                .selectable(false),
            );
            let meta = if exists { tile.meta.clone() } else { "File moved or deleted".to_string() };
            ui.label(RichText::new(meta).small().color(p.muted));
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
                self.toast(format!("Uploading {n} document{}", if n == 1 { "" } else { "s" }));
            }
        }

        // Live transfer readout.
        if let Some((done, total, bps)) = self.cloud_progress {
            ui.add_space(8.0);
            let label = self.cloud_status.clone().unwrap_or_else(|| "Transferring…".into());
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
                    RichText::new(self.cloud_status.clone().unwrap_or_else(|| "Connecting…".into()))
                        .color(p.muted),
                );
            });
        }

        self.selection_bar(ui, ctx);

        // Split by where each file actually is.
        let local_names: std::collections::HashSet<String> =
            self.cloud_local.iter().map(|d| d.name.clone()).collect();
        let in_bucket: Vec<(String, u64, i64)> = self
            .cloud_files
            .iter()
            .filter(|f| !local_names.contains(&local_name(&f.name)))
            .map(|f| (f.name.clone(), f.size, f.uploaded_ms))
            .collect();

        if !self.cloud_local.is_empty() {
            group_label(ui, "On this device", self.cloud_local.len());
            let mut open: Option<PathBuf> = None;
            let mut remove: Option<PathBuf> = None;
            for doc in &self.cloud_local {
                let sub = format!("On this device · {}", cloud::fmt_size(doc.size));
                let resp = w::row(ui, true, Some("✔"), &doc.name, Some(&sub), |ui| {
                    ui.menu_button(RichText::new("…").color(p.muted), |ui| {
                        if ui.button("Open").clicked() {
                            open = Some(doc.path.clone());
                            ui.close();
                        }
                        if ui.button("Remove download").clicked() {
                            remove = Some(doc.path.clone());
                            ui.close();
                        }
                    });
                });
                if resp.clicked() {
                    open = Some(doc.path.clone());
                }
                w::hairline(ui);
            }
            if let Some(path) = remove {
                let _ = std::fs::remove_file(&path);
                self.refresh_local_docs();
            }
            if let Some(path) = open {
                self.open_path(ctx, &path);
            }
        }

        if !in_bucket.is_empty() {
            group_label(ui, "In your bucket", in_bucket.len());
            let mut download: Option<(String, u64, bool)> = None;
            let mut toggle: Option<String> = None;
            for (name, size, uploaded_ms) in &in_bucket {
                let picked = self.cloud_selected.contains(name);
                let sub = format!(
                    "Click to download · {} · {}",
                    cloud::fmt_size(*size),
                    fmt_ts(uploaded_ms / 1000)
                );
                let glyph = if picked { "☑" } else { "☁" };
                let resp = w::row(ui, true, Some(glyph), name, Some(&sub), |ui| {
                    ui.menu_button(
                        RichText::new("…").color(if picked { p.accent } else { p.muted }),
                        |ui| {
                            if ui.button("Download").clicked() {
                                download = Some((name.clone(), *size, false));
                                ui.close();
                            }
                            if ui.button(if picked { "Deselect" } else { "Select" }).clicked() {
                                toggle = Some(name.clone());
                                ui.close();
                            }
                        },
                    );
                });
                if resp.clicked() {
                    download = Some((name.clone(), *size, true));
                }
                w::hairline(ui);
            }
            if let Some(name) = toggle {
                if !self.cloud_selected.remove(&name) {
                    self.cloud_selected.insert(name);
                }
                self.confirm_delete_until = None;
            }
            if let Some((name, size, open)) = download {
                self.toast(format!("Downloading {name}"));
                self.start_cloud_op(ctx, CloudOp::Download(name, size, open));
            }
        }

        if self.cloud_listed && self.cloud_files.is_empty() && self.cloud_local.is_empty() && !busy {
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
                ui.label(RichText::new(headline).text_style(theme::title()).color(p.ink));
                ui.label(RichText::new(&msg).small().color(ui.visuals().error_fg_color));
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

    /// Download / delete for the files ticked with … → Select.
    fn selection_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let n = self.cloud_selected.len();
        if n == 0 {
            return;
        }
        let p = w::pal(ui);
        ui.add_space(10.0);
        let arming = self.confirm_delete_until.is_some_and(|t| Instant::now() < t);
        if arming {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        let mut download = false;
        let mut delete = false;
        let mut clear = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{n} selected")).color(p.ink));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::inline_button(ui, "Clear", true).clicked() {
                    clear = true;
                }
                // Two clicks: these files may be the only copy anywhere.
                let label = if arming { "Confirm delete" } else { "Delete" };
                if w::inline_button(ui, label, !self.cloud_busy).clicked() {
                    delete = true;
                }
                if w::inline_button(ui, "Download", !self.cloud_busy).clicked() {
                    download = true;
                }
            });
        });
        w::hairline(ui);
        if clear {
            self.cloud_selected.clear();
            self.confirm_delete_until = None;
        }
        if download {
            let items: Vec<(String, u64)> = self
                .cloud_files
                .iter()
                .filter(|f| self.cloud_selected.contains(&f.name))
                .map(|f| (f.name.clone(), f.size))
                .collect();
            for (name, size) in items {
                self.start_cloud_op(ctx, CloudOp::Download(name, size, false));
            }
            self.cloud_selected.clear();
        }
        if delete {
            if arming {
                let items: Vec<(String, String)> = self
                    .cloud_files
                    .iter()
                    .filter(|f| self.cloud_selected.contains(&f.name))
                    .map(|f| (f.name.clone(), f.file_id.clone()))
                    .collect();
                if !items.is_empty() {
                    self.start_cloud_op(ctx, CloudOp::Delete(items));
                }
                self.confirm_delete_until = None;
            } else {
                self.confirm_delete_until = Some(Instant::now() + Duration::from_secs(4));
            }
        }
    }
}

/// The on-disk name a bucket file downloads to.
fn local_name(bucket_name: &str) -> String {
    cloud::local_path_for(bucket_name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
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
        ui.label(RichText::new(count.to_string()).text_style(theme::label()).color(p.muted));
    });
    ui.add_space(2.0);
    w::hairline(ui);
}

