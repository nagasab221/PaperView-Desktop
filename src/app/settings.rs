//! Settings: appearance, the Backblaze bucket, storage and system integration.

use eframe::egui::{self, RichText, Stroke};

use super::{CloudOp, HomeScreen, ViewerApp};
use crate::cloud;
use crate::ui::theme::{self, Skin, ThemeChoice};
use crate::ui::widgets as w;

impl ViewerApp {
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let p = w::pal(ui);
        let mut back = false;
        w::masthead(ui, "Settings", None, |ui| {
            if w::inline_button(ui, "Back to Library", true).clicked() {
                back = true;
            }
        });
        if back {
            self.home = HomeScreen::Library;
        }

        // ── Appearance ───────────────────────────────────────────────────────
        w::section_heading(ui, "Appearance", |_| {});
        ui.add_space(12.0);
        w::field_label(ui, "Theme");
        let choices = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];
        let current = choices.iter().position(|c| *c == self.theme).unwrap_or(0);
        if let Some(i) = w::choice_chips(ui, &["System", "Light", "Dark"], current) {
            self.theme = choices[i];
        }
        ui.add_space(16.0);
        w::field_label(ui, "Style");
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
            let dark = self.theme.is_dark(&ctx);
            for skin in Skin::ALL {
                if skin_card(ui, skin, dark, skin == self.skin).clicked() {
                    self.skin = skin;
                }
            }
        });
        self.density_settings(ui);

        // ── Reading ──────────────────────────────────────────────────────────
        w::section_heading(ui, "Reading", |_| {});
        self.reading_settings(ui);

        // ── Backblaze B2 ─────────────────────────────────────────────────────
        w::section_heading(ui, "Backblaze B2", |_| {});
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "Keep documents in your own private bucket — Backblaze gives you 10 GB free. \
                 It's the same bucket your phone uses. Nothing uploads unless you choose \
                 Save to cloud.",
            )
            .color(p.muted),
        );
        ui.hyperlink_to(
            RichText::new("Create a free Backblaze account").color(p.accent),
            "https://www.backblaze.com/sign-up/cloud-storage",
        );
        ui.add_space(6.0);

        let ready = self.cloud_cfg.ready();
        let sub = if !ready {
            "Save your bucket details below to turn this on"
        } else if self.cloud_enabled {
            "On — the Library shows Recents and Cloud"
        } else {
            "Off — the Library shows Recents only"
        };
        let mut on = self.cloud_enabled && ready;
        w::row(ui, false, Some("☁"), "Cloud tab", Some(sub), |ui| {
            ui.add_space(4.0);
            if w::switch(ui, &mut on, ready).changed() {
                self.cloud_enabled = on;
            }
        });
        w::hairline(ui);
        ui.add_space(14.0);

        w::field_label(ui, "Key ID");
        w::field(
            ui,
            &mut self.b2_draft.key_id,
            "e.g. 005a1b2c3d4e5f60000000001",
            false,
            380.0,
        );
        ui.add_space(8.0);
        w::field_label(ui, "Application key");
        ui.horizontal(|ui| {
            w::field(
                ui,
                &mut self.b2_draft.app_key,
                "Starts with K — shown once, when the key is created",
                !self.show_app_key,
                380.0,
            );
            if w::inline_button(ui, if self.show_app_key { "Hide" } else { "Show" }, true).clicked()
            {
                self.show_app_key = !self.show_app_key;
            }
        });
        let key = self.b2_draft.app_key.trim();
        if !key.is_empty() && !key.starts_with('K') {
            ui.label(
                RichText::new("Application keys start with “K” — this looks like the Key ID.")
                    .small()
                    .color(ui.visuals().error_fg_color),
            );
        }
        ui.add_space(8.0);
        w::field_label(ui, "Bucket name");
        w::field(
            ui,
            &mut self.b2_draft.bucket,
            "Exactly as it appears in Backblaze",
            false,
            380.0,
        );
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let changed = self.b2_draft != self.cloud_cfg;
            let label = if changed || !ready {
                "Save and connect"
            } else {
                "Saved"
            };
            if w::primary_button(ui, label, self.b2_draft.ready() && (changed || !ready)).clicked()
            {
                self.cloud_cfg = B2Trimmed::trim(self.b2_draft.clone());
                self.b2_draft = self.cloud_cfg.clone();
                self.cloud_files.clear();
                self.cloud_listed = false;
                self.cloud_status = None;
                self.cloud_enabled = true;
                self.start_cloud_op(&ctx, CloudOp::List);
                self.toast("Connecting to your bucket…");
            }
            if ready && w::inline_button(ui, "Forget these details", true).clicked() {
                self.cloud_cfg = Default::default();
                self.b2_draft = Default::default();
                self.cloud_files.clear();
                self.cloud_listed = false;
                self.cloud_status = None;
            }
        });
        if let Some(msg) = self.cloud_status.clone().filter(|_| !self.cloud_busy) {
            ui.add_space(6.0);
            ui.label(RichText::new(msg).small().color(p.muted));
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new("Stored on this computer only, in PaperView's settings file.")
                .small()
                .color(p.muted),
        );

        ui.add_space(10.0);
        let active = self.cloud_active();
        let sub = format!("Upload every PDF you open, up to {} MB", self.auto_save_mb);
        let mut auto = self.auto_save && active;
        w::row(
            ui,
            false,
            Some("⬆"),
            "Auto-save opened PDFs",
            Some(&sub),
            |ui| {
                ui.add_space(4.0);
                if w::switch(ui, &mut auto, active).changed() {
                    self.auto_save = auto;
                }
                ui.add_enabled(
                    active && self.auto_save,
                    egui::DragValue::new(&mut self.auto_save_mb)
                        .range(1..=500)
                        .suffix(" MB"),
                );
            },
        );
        w::hairline(ui);

        // ── Storage ──────────────────────────────────────────────────────────
        w::section_heading(ui, "Storage", |_| {});
        let (n, bytes) = self.cloud_cache;
        let sub = format!(
            "{n} file{} · {} — downloaded from your bucket",
            if n == 1 { "" } else { "s" },
            cloud::fmt_size(bytes)
        );
        let mut clear = false;
        w::row(
            ui,
            false,
            Some("⬇"),
            "Downloaded documents",
            Some(&sub),
            |ui| {
                if w::inline_button(ui, "Clear", n > 0).clicked() {
                    clear = true;
                }
            },
        );
        w::hairline(ui);
        if clear {
            if let Ok(dir) = std::fs::read_dir(cloud::cache_dir()) {
                for entry in dir.flatten() {
                    if entry.path().is_file() {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
            self.refresh_local_docs();
            self.toast("Downloaded documents cleared");
        }

        let (cn, cbytes) = crate::covers::cache_stats();
        let sub = format!(
            "{cn} saved · {} — so the Library opens without re-rendering",
            cloud::fmt_size(cbytes)
        );
        let mut clear_covers = false;
        w::row(
            ui,
            false,
            Some("🖼"),
            "Cover thumbnails",
            Some(&sub),
            |ui| {
                if w::inline_button(ui, "Clear", cn > 0).clicked() {
                    clear_covers = true;
                }
            },
        );
        w::hairline(ui);
        if clear_covers {
            crate::covers::clear_cache();
            self.covers.forget_disk();
            self.toast("Saved covers cleared; they'll be made again as needed");
        }

        // ── System ───────────────────────────────────────────────────────────
        w::section_heading(ui, "System", |_| {});
        let is_default = *self
            .default_status
            .get_or_insert_with(crate::assoc::is_default);
        let sub = if is_default {
            "PDFs and ebooks open with PaperView"
        } else if cfg!(target_os = "windows") {
            "Opens Windows' Default apps, where you can choose PaperView"
        } else {
            "Open PDFs and ebooks (EPUB, Kindle, FB2, comics) with PaperView"
        };
        let mut make = false;
        let mut remove = false;
        w::row(
            ui,
            false,
            Some("📄"),
            "Default app for PDFs and books",
            Some(sub),
            |ui| {
                if is_default {
                    if w::inline_button(ui, "Undo", true)
                        .on_hover_text("Remove PaperView from the Open with list")
                        .clicked()
                    {
                        remove = true;
                    }
                } else if w::primary_button(ui, "Make default", true).clicked() {
                    make = true;
                }
            },
        );
        w::hairline(ui);
        if make {
            self.make_default_app();
        }
        if remove {
            let _ = crate::assoc::unregister();
            self.default_status = None;
            self.toast("PaperView is no longer the default; your system picks another app");
        }

        // ── About ────────────────────────────────────────────────────────────
        w::section_heading(ui, "About", |_| {});
        ui.add_space(10.0);
        ui.label(
            RichText::new(format!("PaperView {}", env!("CARGO_PKG_VERSION")))
                .text_style(theme::title())
                .color(p.ink),
        );
        ui.label(
            RichText::new("A free PDF reader. No ads, no accounts. Pages are drawn by PDFium.")
                .color(p.muted),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(SHORTCUTS).small().color(p.muted));
    }
}

impl ViewerApp {
    pub(super) fn make_default_app(&mut self) {
        match crate::assoc::make_default() {
            Ok(()) if cfg!(target_os = "windows") => {
                self.toast("In Default apps, choose PaperView for .pdf and .epub files")
            }
            Ok(()) => self.toast("PaperView now opens your PDFs and ebooks"),
            Err(e) => self.status = Some(format!("Couldn't make PaperView the default: {e}")),
        }
        self.default_status = None;
    }

    /// Asked once, on first launch: open PDFs and ebooks with PaperView?
    pub(super) fn first_run_prompt(&mut self, ctx: &egui::Context) {
        if self.default_asked {
            return;
        }
        let p = theme::palette(self.skin, self.theme.is_dark(ctx));
        let mut answer: Option<bool> = None;
        let modal = w::modal("first-run-default", ctx).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new("Open your PDFs and books with PaperView?").text_style(theme::headline()).color(p.ink));
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "PaperView can be the app that opens PDFs and ebooks — EPUB, Kindle (MOBI, AZW3), \
                     FB2 and comic books — when you double-click them. You can change this any time in \
                     Settings.",
                )
                .color(p.muted),
            );
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::primary_button(ui, "Make PaperView the default", true).clicked() {
                        answer = Some(true);
                    }
                    if w::outlined_button(ui, "Not now", true).clicked() {
                        answer = Some(false);
                    }
                });
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        if let Some(yes) = answer {
            self.default_asked = true;
            if yes {
                self.make_default_app();
            }
        }
    }
}

const SHORTCUTS: &str = "Shortcuts — Ctrl+O open · Ctrl+F find · Ctrl+P print · \
    Ctrl+W close · Ctrl+Tab next document · Hold Q peek · Tap Q jump to peek spot · \
    Shift+Q set peek spot · Ctrl+= / Ctrl+- zoom · PgUp / PgDn pages · Ctrl+G go to page · \
    F11 full screen · Alt+Left back";

/// Whitespace pasted around credentials is the usual reason they're rejected.
struct B2Trimmed;
impl B2Trimmed {
    fn trim(mut c: cloud::B2Config) -> cloud::B2Config {
        c.key_id = c.key_id.trim().to_string();
        c.app_key = c.app_key.trim().to_string();
        c.bucket = c.bucket.trim().to_string();
        c
    }
}

/// A small preview of a skin: its paper, a rule, a line of ink, its accent.
fn skin_card(ui: &mut egui::Ui, skin: Skin, dark: bool, selected: bool) -> egui::Response {
    let current = w::pal(ui);
    let sp = theme::palette(skin, dark);
    let size = egui::vec2(206.0, 92.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 6, sp.background);
    let border = if selected {
        current.accent
    } else {
        current.hairline
    };
    painter.rect_stroke(
        rect,
        6,
        Stroke::new(if selected { 2.0 } else { 1.0 }, border),
        egui::StrokeKind::Inside,
    );
    let x = rect.left() + 14.0;
    painter.text(
        egui::pos2(x, rect.top() + 12.0),
        egui::Align2::LEFT_TOP,
        skin.label(),
        egui::FontId::new(18.0, theme::serif()),
        sp.ink,
    );
    painter.hline(
        x..=rect.right() - 14.0,
        rect.top() + 42.0,
        Stroke::new(1.0, sp.hairline),
    );
    painter.text(
        egui::pos2(x, rect.top() + 50.0),
        egui::Align2::LEFT_TOP,
        skin.blurb(),
        egui::FontId::new(10.5, egui::FontFamily::Proportional),
        sp.muted,
    );
    painter.circle_filled(
        egui::pos2(rect.right() - 20.0, rect.top() + 22.0),
        7.0,
        sp.accent,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}
