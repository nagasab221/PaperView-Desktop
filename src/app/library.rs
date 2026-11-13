//! The Library: Recents as a shelf of first-page covers and, once a Backblaze
//! bucket is set up and switched on, a Cloud page beside it — the same two
//! tabs as the phone. The Cloud tab doesn't exist until the bucket does.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke};

use super::{
    fmt_speed, fmt_time, fmt_ts, open_file_location, CloudOp, HomeScreen, LandingTab, ViewerApp,
    FREE_TIER_BYTES,
};
use crate::cloud;
use crate::covers::Cover;
use crate::ui::{theme, widgets as w};

const TILE_W: f32 = 150.0;
const TILE_GAP: f32 = 24.0;
/// Cover aspect (width / height), as on the phone.
const COVER_ASPECT: f32 = 0.77;
const COLUMN_MAX: f32 = 1080.0;

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
            .id_salt(("home", self.home == HomeScreen::Settings))
            .show(ui, |ui| {
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

    fn library_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let show_cloud = self.cloud_active();
        if !show_cloud {
            self.landing_tab = LandingTab::Recent;
        }
        let on_cloud = show_cloud && self.landing_tab == LandingTab::Saved;
        let subtitle = if on_cloud {
            self.cloud_cfg.bucket.trim().to_string()
        } else if self.recents.is_empty() {
            "Open a PDF with Ctrl+O, or drop one anywhere in this window".to_string()
        } else {
            let n = self.recents.len();
            format!("{n} document{}", if n == 1 { "" } else { "s" })
        };

        let mut open = false;
        let mut settings = false;
        w::masthead(ui, "Library", Some(&subtitle), |ui| {
            if w::icon_button(ui, "⚙", "Settings (Ctrl+,)", false).clicked() {
                settings = true;
            }
            if w::primary_button(ui, "Open a PDF", true).on_hover_text("Ctrl+O").clicked() {
                open = true;
            }
        });
        if open {
            self.open_dialog(&ctx);
        }
        if settings {
            self.open_settings(&ctx);
            return;
        }

        if show_cloud {
            ui.add_space(14.0);
            let sel = usize::from(on_cloud);
            if let Some(i) = w::text_tabs(ui, &["Recents", "Cloud"], sel) {
                self.landing_tab = if i == 1 { LandingTab::Saved } else { LandingTab::Recent };
            }
        } else {
            ui.add_space(10.0);
        }

        if on_cloud {
            self.cloud_page(ui, &ctx);
        } else {
            self.recents_shelf(ui, &ctx);
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
        let width = ui.available_width();
        let cols = (((width + TILE_GAP) / (TILE_W + TILE_GAP)).floor() as usize).max(1);
        let tile_w = ((width - TILE_GAP * (cols as f32 - 1.0)) / cols as f32).min(TILE_W + 30.0);

        let mut open_req: Option<PathBuf> = None;
        let mut remove_req: Option<usize> = None;
        let entries: Vec<(usize, PathBuf, i32, i64)> = self
            .recents
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.path.clone(), r.page, r.ts))
            .collect();
        for chunk in entries.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = TILE_GAP;
                for (i, path, page, ts) in chunk {
                    let (open, remove) = self.cover_tile(ui, ctx, path, *page, *ts, tile_w);
                    if open {
                        open_req = Some(path.clone());
                    }
                    if remove {
                        remove_req = Some(*i);
                    }
                }
            });
            ui.add_space(18.0);
        }
        if let Some(i) = remove_req {
            self.recents.remove(i);
        }
        if let Some(p) = open_req {
            self.open_path(ctx, &p);
        }
    }

    /// One document on the shelf: cover, title, and where you were.
    fn cover_tile(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        path: &std::path::Path,
        page: i32,
        ts: i64,
        tile_w: f32,
    ) -> (bool, bool) {
        let p = w::pal(ui);
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
            let (rect, resp) = ui.allocate_exact_size(
                cover_size,
                if exists { Sense::click() } else { Sense::hover() },
            );
            let painter = ui.painter_at(rect.expand(2.0));
            painter.rect_filled(rect, 0, p.surface);
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
            let edge = if resp.hovered() { p.muted } else { p.hairline };
            painter.rect_stroke(rect, 0, Stroke::new(1.0, edge), egui::StrokeKind::Outside);
            if resp.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                open = true;
            }
            let path_for_menu = path;
            resp.on_hover_text(path.display().to_string()).context_menu(|ui| {
                if exists && ui.button("Open").clicked() {
                    open = true;
                    ui.close();
                }
                if ui.button("Show in folder").clicked() {
                    open_file_location(path_for_menu);
                    ui.close();
                }
                if ui.button("Remove from library").clicked() {
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
            let meta = if exists {
                format!("page {} · {}", page + 1, fmt_ts(ts))
            } else {
                "File moved or deleted".to_string()
            };
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

