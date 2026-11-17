//! Settings: appearance, reading, cloud accounts, storage and system integration.

use eframe::egui::{self, RichText, Stroke};

use std::sync::mpsc::Receiver;
use std::time::Instant;

use super::{now_ts, CloudOp, HomeScreen, ViewerApp};
use crate::cloud::{self, Account, CloudEvent, CloudFile, Kind};
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
        // One grid, four to a row (two when narrow), Paper first.
        let dark = self.theme.is_dark(&ctx);
        let gap = 12.0;
        let cols = if ui.available_width() >= 760.0 {
            4.0
        } else {
            2.0
        };
        let card_w = ((ui.available_width().min(1100.0) - gap * (cols - 1.0)) / cols).floor();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            for skin in Skin::ALL {
                if skin_card(ui, skin, dark, skin == self.skin, card_w).clicked() {
                    self.skin = skin;
                }
            }
        });
        self.density_settings(ui);

        // ── Reading ──────────────────────────────────────────────────────────
        w::section_heading(ui, "Reading", |_| {});
        self.reading_settings(ui);

        // ── Cloud accounts ───────────────────────────────────────────────────
        self.cloud_settings(ui, &ctx);

        // ── Storage ──────────────────────────────────────────────────────────
        w::section_heading(ui, "Storage", |_| {});
        let (n, bytes) = self.cloud_cache;
        let sub = format!(
            "{n} file{} · {} — downloaded from your cloud accounts",
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
                    let path = entry.path();
                    let _ = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
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
            RichText::new(
                "A free reader for PDFs and ebooks. No ads, no accounts, nothing tracked. \
                 Pages are drawn by PDFium.",
            )
            .color(p.muted),
        );
        ui.hyperlink_to(
            RichText::new("Project page and updates").color(p.accent),
            "https://github.com/nagasab221/PaperView-Desktop",
        );
        ui.add_space(14.0);
        w::field_label(ui, "Keyboard shortcuts");
        ui.add_space(4.0);
        egui::Grid::new("shortcuts")
            .num_columns(4)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                for (i, (keys, what)) in SHORTCUTS.iter().enumerate() {
                    ui.label(RichText::new(*keys).family(theme::medium()).color(p.ink));
                    ui.label(RichText::new(*what).color(p.muted));
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(24.0);
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

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+O", "Open a PDF or ebook"),
    ("Ctrl+F", "Find in the document"),
    ("Ctrl+G", "Go to a page"),
    ("F11", "Full screen (Esc leaves)"),
    ("Hold Q", "Peek at your saved spot"),
    ("Tap Q", "Jump to it and back"),
    ("Shift+Q", "Set the peek spot"),
    ("Ctrl+= / Ctrl+-", "Zoom, or text size in books"),
    ("PgUp / PgDn", "Previous / next page"),
    ("Home / End", "First / last page"),
    ("Alt+Left", "Back after a link"),
    ("Ctrl+P", "Print"),
    ("Ctrl+Tab", "Next document"),
    ("Ctrl+W", "Close the document"),
    ("Ctrl+Shift+T", "Reopen a closed document"),
    ("Ctrl+,", "Settings"),
];

/// A small preview of a skin: its paper, a rule, a line of ink, its accent.
fn skin_card(
    ui: &mut egui::Ui,
    skin: Skin,
    dark: bool,
    selected: bool,
    width: f32,
) -> egui::Response {
    let current = w::pal(ui);
    let sp = theme::palette(skin, dark);
    let size = egui::vec2(width, 92.0);
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
    // The description wraps inside the card (two lines at most).
    let blurb = painter.layout(
        skin.blurb().to_string(),
        egui::FontId::new(10.5, egui::FontFamily::Proportional),
        sp.muted,
        rect.width() - 28.0,
    );
    painter.with_clip_rect(rect.shrink(1.0)).galley(
        egui::pos2(x, rect.top() + 50.0),
        blurb,
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

/// The account form in Settings: a new account, or changes to one.
pub(super) struct AccountEditor {
    /// The account being changed (None: a new one).
    editing: Option<String>,
    draft: Account,
    /// A new password / secret key; empty keeps the saved one.
    secret: String,
    show_secret: bool,
    /// Why the last connection check failed (nothing was saved).
    failed: Option<String>,
}

/// A connection check running in the background: lists the account's top
/// folder, the same thing the Cloud tab does first.
pub(super) struct CloudTest {
    /// The account as it will be saved.
    account: Account,
    /// A newly typed secret; kept only if the account is saved.
    secret: Option<String>,
    rx: Receiver<CloudEvent>,
    started: Instant,
    /// What the worker said it's doing.
    stage: String,
    /// From the account form: save the account once it connects.
    from_form: bool,
}

/// What an account last said: connected (with what's there) or why not.
pub(super) struct Health {
    ok: bool,
    text: String,
    ts: i64,
}

/// "3 documents and 2 folders", "empty so far".
pub(super) fn listing_summary(files: &[CloudFile]) -> String {
    let dirs = files.iter().filter(|f| f.is_dir).count();
    let docs = files.len() - dirs;
    let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
    match (docs, dirs) {
        (0, 0) => "empty so far".to_string(),
        (d, 0) => plural(d, "document"),
        (0, f) => plural(f, "folder"),
        (d, f) => format!("{} and {}", plural(d, "document"), plural(f, "folder")),
    }
}

impl AccountEditor {
    fn new(kind: Kind) -> Self {
        AccountEditor {
            editing: None,
            draft: Account {
                kind,
                ..Default::default()
            },
            secret: String::new(),
            show_secret: false,
            failed: None,
        }
    }
}

impl ViewerApp {
    fn cloud_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        let mut add = false;
        w::section_heading(ui, "Cloud", |ui| {
            if self.cloud_editor.is_none() && w::inline_button(ui, "Add account", true).clicked() {
                add = true;
            }
        });
        if add {
            self.cloud_editor = Some(AccountEditor::new(Kind::Ssh));
        }
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "Keep documents somewhere besides this computer: a NAS, another computer over \
                 SSH, or a cloud bucket. Nothing uploads unless you choose Save to cloud.",
            )
            .color(p.muted),
        );
        ui.add_space(6.0);

        let mut use_id: Option<String> = None;
        let mut edit_id: Option<String> = None;
        let mut remove_id: Option<String> = None;
        let mut test_id: Option<String> = None;
        let error = ui.visuals().error_fg_color;
        for acct in &self.cloud_accounts {
            let current = acct.id == self.cloud_current;
            let mut sub = acct.summary();
            if current && self.cloud_accounts.len() > 1 {
                sub.push_str(" · shown in the Cloud tab");
            }
            let testing = self
                .cloud_test
                .as_ref()
                .filter(|t| !t.from_form && t.account.id == acct.id);
            w::row(
                ui,
                false,
                Some(acct.kind.glyph()),
                &acct.name,
                Some(&sub),
                |ui| {
                    if w::inline_button(ui, "Remove", true).clicked() {
                        remove_id = Some(acct.id.clone());
                    }
                    if w::inline_button(ui, "Edit", self.cloud_editor.is_none()).clicked() {
                        edit_id = Some(acct.id.clone());
                    }
                    if w::inline_button(ui, "Test", self.cloud_test.is_none() && acct.ready())
                        .on_hover_text("Connect now and say what happened")
                        .clicked()
                    {
                        test_id = Some(acct.id.clone());
                    }
                    if !current && w::inline_button(ui, "Show", acct.ready()).clicked() {
                        use_id = Some(acct.id.clone());
                    }
                },
            );
            // What the account last said: connecting now, connected, or why not.
            let line = match (testing, self.cloud_health.get(&acct.id)) {
                (Some(t), _) => Some((
                    format!("{}  {} s", t.stage, t.started.elapsed().as_secs()),
                    p.muted,
                    true,
                )),
                (None, Some(h)) if h.ok => Some((
                    format!("✔ {} · checked {}", h.text, super::fmt_time(h.ts)),
                    p.accent,
                    false,
                )),
                (None, Some(h)) => Some((format!("Couldn't connect: {}", h.text), error, false)),
                (None, None) => None,
            };
            if let Some((text, color, busy)) = line {
                ui.horizontal(|ui| {
                    ui.add_space(38.0);
                    if busy {
                        ui.spinner();
                    }
                    ui.add(egui::Label::new(RichText::new(text).small().color(color)).wrap());
                });
                ui.add_space(6.0);
            }
            w::hairline(ui);
        }
        if let Some(id) = test_id {
            if let Some(acct) = self.cloud_accounts.iter().find(|a| a.id == id).cloned() {
                self.start_test(ctx, acct, None, false);
            }
        }
        if self.cloud_accounts.is_empty() && self.cloud_editor.is_none() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("No accounts yet.").color(p.muted));
                if w::inline_button(ui, "Add one", true).clicked() {
                    self.cloud_editor = Some(AccountEditor::new(Kind::Ssh));
                }
            });
        }
        if let Some(id) = use_id {
            self.switch_account(id);
            self.cloud_enabled = true;
        }
        if let Some(id) = edit_id {
            if let Some(acct) = self.cloud_accounts.iter().find(|a| a.id == id) {
                self.cloud_editor = Some(AccountEditor {
                    editing: Some(id),
                    draft: acct.clone(),
                    secret: String::new(),
                    show_secret: false,
                    failed: None,
                });
            }
        }
        if let Some(id) = remove_id {
            self.cloud_dialog = Some(super::library::CloudDialog::RemoveAccount { id });
        }

        if self.cloud_editor.is_some() {
            self.account_form(ui, ctx);
        }

        ui.add_space(10.0);
        let ready = self.current_account().is_some_and(|a| a.ready());
        let sub = if !ready {
            "Add an account above to turn this on"
        } else if self.cloud_enabled {
            "On — the Library shows a Cloud tab"
        } else {
            "Off — the Library shows no Cloud tab"
        };
        let mut on = self.cloud_enabled && ready;
        w::row(ui, false, Some("☁"), "Cloud tab", Some(sub), |ui| {
            ui.add_space(4.0);
            if w::switch(ui, &mut on, ready).changed() {
                self.cloud_enabled = on;
            }
        });
        w::hairline(ui);

        let active = self.cloud_active();
        let sub = format!(
            "Upload every document you open to {}, up to {} MB",
            self.account_name(),
            self.auto_save_mb
        );
        let mut auto = self.auto_save && active;
        w::row(
            ui,
            false,
            Some("⬆"),
            "Auto-save opened documents",
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
        ui.add_space(4.0);
        let plain: Vec<&str> = self
            .cloud_accounts
            .iter()
            .filter(|a| a.plain_secret.is_some())
            .map(|a| a.name.as_str())
            .collect();
        let note = if plain.is_empty() {
            "Passwords and keys are kept in your system keyring, never in PaperView's settings."
                .to_string()
        } else {
            format!(
                "No system keyring was available, so the password for {} is kept in \
                 PaperView's settings file on this computer.",
                plain.join(", ")
            )
        };
        ui.label(RichText::new(note).small().color(p.muted));
    }

    /// The add / edit form for one account.
    fn account_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        let Some(mut ed) = self.cloud_editor.take() else {
            return;
        };
        let mut cancel = false;
        let has_saved_secret = ed.editing.as_ref().is_some_and(|id| {
            self.cloud_accounts
                .iter()
                .any(|a| a.id == *id && a.kind == ed.draft.kind)
        });
        let mut test = false;
        let mut save_anyway = false;
        let mut stop = false;
        let testing = self
            .cloud_test
            .as_ref()
            .filter(|t| t.from_form)
            .map(|t| (t.stage.clone(), t.started.elapsed().as_secs()));
        ui.add_space(12.0);
        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.hairline))
            .corner_radius(6)
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.add_enabled_ui(testing.is_none(), |ui| {
                    let title = if ed.editing.is_some() {
                        format!("Edit {}", ed.draft.name)
                    } else {
                        "Add an account".to_string()
                    };
                    ui.label(RichText::new(title).text_style(theme::title()).color(p.ink));
                    ui.add_space(8.0);
                    if ed.editing.is_none() {
                        let labels: Vec<&str> = Kind::ALL.iter().map(|k| k.label()).collect();
                        let sel = Kind::ALL
                            .iter()
                            .position(|k| *k == ed.draft.kind)
                            .unwrap_or(0);
                        if let Some(i) = w::choice_chips(ui, &labels, sel) {
                            ed.draft.kind = Kind::ALL[i];
                        }
                        ui.add_space(8.0);
                    }
                    let (blurb, link) = kind_blurb(ed.draft.kind);
                    ui.label(RichText::new(blurb).color(p.muted));
                    if let Some((text, url)) = link {
                        ui.hyperlink_to(RichText::new(text).color(p.accent), url);
                    }
                    ui.add_space(10.0);

                    const W: f32 = 380.0;
                    let field = |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str| {
                        w::field_label(ui, label);
                        w::field(ui, value, hint, false, W);
                        ui.add_space(6.0);
                    };
                    let d = &mut ed.draft;
                    field(ui, "Name", &mut d.name, name_hint(d.kind));
                    match d.kind {
                        Kind::Folder => {
                            w::field_label(ui, "Folder");
                            ui.horizontal(|ui| {
                                w::field(ui, &mut d.address, "e.g. /mnt/nas/Documents", false, W);
                                if w::inline_button(ui, "Choose…", true).clicked() {
                                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                        d.address = dir.to_string_lossy().into_owned();
                                    }
                                }
                            });
                            ui.add_space(6.0);
                        }
                        Kind::WebDav => {
                            field(
                                ui,
                                "Server address",
                                &mut d.address,
                                "https://nas.local:5006/PaperView",
                            );
                            field(ui, "User name", &mut d.user, "");
                        }
                        Kind::S3 => {
                            field(
                                ui,
                                "Endpoint",
                                &mut d.address,
                                "https://<account id>.r2.cloudflarestorage.com",
                            );
                            field(
                                ui,
                                "Region",
                                &mut d.region,
                                "“auto” for R2, e.g. eu-central-1 — empty means us-east-1",
                            );
                            field(ui, "Bucket", &mut d.bucket, "");
                            field(ui, "Access key ID", &mut d.user, "");
                        }
                        Kind::B2 => {
                            field(ui, "Key ID", &mut d.user, "e.g. 005a1b2c3d4e5f60000000001");
                            field(
                                ui,
                                "Bucket name",
                                &mut d.bucket,
                                "Exactly as it appears in Backblaze",
                            );
                        }
                        Kind::Ssh => {
                            field(
                                ui,
                                "Host",
                                &mut d.address,
                                "A host name or an alias from ~/.ssh/config",
                            );
                            field(
                                ui,
                                "User (optional)",
                                &mut d.user,
                                "When not set in ~/.ssh/config",
                            );
                            field(ui, "Port (optional)", &mut d.port, "22");
                        }
                    }
                    if d.kind.has_secret() {
                        let label = match d.kind {
                            Kind::S3 => "Secret access key",
                            Kind::B2 => "Application key",
                            _ => "Password",
                        };
                        let hint = if has_saved_secret {
                            "Saved — leave empty to keep it"
                        } else if d.kind == Kind::B2 {
                            "Starts with K — shown once, when the key is created"
                        } else {
                            ""
                        };
                        w::field_label(ui, label);
                        ui.horizontal(|ui| {
                            w::field(ui, &mut ed.secret, hint, !ed.show_secret, W);
                            if w::inline_button(
                                ui,
                                if ed.show_secret { "Hide" } else { "Show" },
                                true,
                            )
                            .clicked()
                            {
                                ed.show_secret = !ed.show_secret;
                            }
                        });
                        let key = ed.secret.trim();
                        if ed.draft.kind == Kind::B2 && !key.is_empty() && !key.starts_with('K') {
                            ui.label(
                                RichText::new(
                                    "Application keys start with “K” — this looks like the Key ID.",
                                )
                                .small()
                                .color(ui.visuals().error_fg_color),
                            );
                        }
                        ui.add_space(6.0);
                    }
                    let d = &mut ed.draft;
                    match d.kind {
                        Kind::S3 | Kind::B2 => field(
                            ui,
                            "Folder in the bucket (optional)",
                            &mut d.folder,
                            "Empty uses the whole bucket",
                        ),
                        Kind::Ssh => field(
                            ui,
                            "Folder on the server",
                            &mut d.folder,
                            "PaperView (in your home folder) — made if it's missing",
                        ),
                        _ => {}
                    }
                    if matches!(d.kind, Kind::WebDav | Kind::S3) {
                        ui.checkbox(
                            &mut d.insecure_tls,
                            RichText::new(
                                "Trust a self-signed certificate (a NAS on your own network)",
                            )
                            .color(p.ink),
                        );
                        ui.add_space(6.0);
                    }
                    if d.kind == Kind::Ssh {
                        ui.label(
                            RichText::new(
                                "Signs in with your SSH keys and ~/.ssh/config, like ssh in a \
                             terminal; no password is stored. A new server's key is accepted \
                             the first time.",
                            )
                            .small()
                            .color(p.muted),
                        );
                        ui.add_space(6.0);
                    }
                });

                let needs_secret = ed.draft.kind.has_secret()
                    && ed.draft.kind != Kind::WebDav
                    && !has_saved_secret
                    && ed.secret.trim().is_empty();
                let ok = ed.draft.ready() && !needs_secret;
                ui.add_space(6.0);
                if let Some((stage, secs)) = &testing {
                    // Checking: what's happening, and for how long.
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(format!("{stage}  {secs} s")).color(p.ink));
                    });
                    ui.label(
                        RichText::new(
                            "PaperView saves the account once it answers. If nothing answers, \
                             it gives up within about 30 seconds.",
                        )
                        .small()
                        .color(p.muted),
                    );
                    ui.add_space(6.0);
                    if w::outlined_button(ui, "Stop", true).clicked() {
                        stop = true;
                    }
                    return;
                }
                if let Some(why) = &ed.failed {
                    let error = ui.visuals().error_fg_color;
                    ui.label(
                        RichText::new("Couldn't connect — nothing was saved")
                            .family(theme::medium())
                            .color(error),
                    );
                    ui.add(egui::Label::new(RichText::new(why).small().color(error)).wrap());
                    ui.label(
                        RichText::new(
                            "Fix the details and try again, or save it anyway to connect later \
                             (say, when the NAS is on).",
                        )
                        .small()
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                }
                ui.horizontal(|ui| {
                    let label = if ed.failed.is_some() {
                        "Try again"
                    } else {
                        "Connect and save"
                    };
                    if w::primary_button(ui, label, ok).clicked() {
                        test = true;
                    }
                    if ed.failed.is_some() && w::outlined_button(ui, "Save anyway", ok).clicked() {
                        save_anyway = true;
                    }
                    if w::outlined_button(ui, "Cancel", true).clicked() {
                        cancel = true;
                    }
                });
            });
        if test {
            let (acct, secret) = prepare_account(&ed);
            ed.failed = None;
            self.start_test(ctx, acct, secret, true);
            self.cloud_editor = Some(ed);
        } else if save_anyway {
            let (acct, secret) = prepare_account(&ed);
            self.commit_account(ctx, acct, secret, None);
        } else if stop {
            self.cloud_test = None;
            self.cloud_editor = Some(ed);
        } else if cancel {
            if self.cloud_test.as_ref().is_some_and(|t| t.from_form) {
                self.cloud_test = None;
            }
        } else {
            self.cloud_editor = Some(ed);
        }
    }

    /// Starts a connection check of `account` (with a newly typed secret,
    /// if any). One at a time.
    fn start_test(
        &mut self,
        ctx: &egui::Context,
        account: Account,
        secret: Option<String>,
        from_form: bool,
    ) {
        let mut probe = account.clone();
        if let Some(s) = &secret {
            probe.plain_secret = Some(s.clone());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        cloud::spawn(probe, CloudOp::List(String::new()), tx, ctx.clone());
        self.cloud_test = Some(CloudTest {
            account,
            secret,
            rx,
            started: Instant::now(),
            stage: "Starting…".into(),
            from_form,
        });
    }

    /// Follows a running connection check; when it ends, records the result
    /// and (from the form) saves the account if it connected.
    pub(super) fn poll_cloud_test(&mut self, ctx: &egui::Context) {
        let Some(t) = self.cloud_test.as_mut() else {
            return;
        };
        let mut done = None;
        while let Ok(event) = t.rx.try_recv() {
            match event {
                CloudEvent::Status(s) => t.stage = s,
                CloudEvent::Listed { files, .. } => done = Some(Ok(files)),
                CloudEvent::Error(e) => done = Some(Err(e)),
                _ => {}
            }
        }
        // Keep the seconds counting.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let Some(result) = done else {
            return;
        };
        let Some(t) = self.cloud_test.take() else {
            return;
        };
        let name = t.account.name.clone();
        let id = t.account.id.clone();
        match result {
            Ok(files) => {
                let summary = listing_summary(&files);
                self.note_health(&id, true, format!("Connected · {summary}"));
                self.toast(format!("Connected to {name} — {summary}"));
                if t.from_form {
                    self.cloud_editor = None;
                    self.commit_account(ctx, t.account, t.secret, Some(files));
                }
            }
            Err(why) => {
                self.note_health(&id, false, why.clone());
                if t.from_form {
                    if let Some(ed) = self.cloud_editor.as_mut() {
                        ed.failed = Some(why);
                    }
                } else {
                    self.toast(format!("Couldn't connect to {name}"));
                }
            }
        }
    }

    pub(super) fn note_health(&mut self, id: &str, ok: bool, text: String) {
        self.cloud_health.insert(
            id.to_string(),
            Health {
                ok,
                text,
                ts: now_ts(),
            },
        );
    }

    /// Saves an account (and its new secret) and shows it on the Cloud tab;
    /// `listing` is its top folder when a check just read it.
    fn commit_account(
        &mut self,
        ctx: &egui::Context,
        mut acct: Account,
        secret: Option<String>,
        listing: Option<Vec<CloudFile>>,
    ) {
        if let Some(secret) = secret {
            // A new secret goes to the keyring (the settings-file copy, if
            // there was one, is dropped; it comes back if the keyring fails).
            acct.plain_secret = None;
            self.persist_secret(ctx, acct.id.clone(), secret);
        }
        let id = acct.id.clone();
        match self.cloud_accounts.iter_mut().find(|a| a.id == id) {
            Some(slot) => *slot = acct,
            None => self.cloud_accounts.push(acct),
        }
        self.switch_account(id);
        self.cloud_enabled = true;
        match listing {
            Some(files) => {
                self.cloud_top = files
                    .iter()
                    .filter(|f| !f.is_dir)
                    .map(|f| (f.name.clone(), f.size))
                    .collect();
                self.cloud_files = files;
                self.cloud_listed = true;
                self.cloud_refreshed = Some(now_ts());
                self.refresh_local_docs();
            }
            None => self.start_cloud_op(ctx, CloudOp::List(String::new())),
        }
    }

    /// Forgets an account, its saved secret and its downloads.
    pub(super) fn remove_account(&mut self, id: &str) {
        let Some(i) = self.cloud_accounts.iter().position(|a| a.id == id) else {
            return;
        };
        let acct = self.cloud_accounts.remove(i);
        let gone = acct.id.clone();
        std::thread::spawn(move || cloud::secrets::forget(&gone));
        let _ = std::fs::remove_dir_all(cloud::account_cache(&acct.id));
        self.cloud_queue.retain(|(a, _)| a.id != acct.id);
        if self.cloud_current == acct.id {
            let next = self
                .cloud_accounts
                .first()
                .map(|a| a.id.clone())
                .unwrap_or_default();
            self.switch_account(next);
        }
        self.refresh_local_docs();
        self.toast(format!("Removed {}", acct.name));
    }
}

/// The account as the form describes it — named, with an id — and the
/// newly typed secret, if any.
fn prepare_account(ed: &AccountEditor) -> (Account, Option<String>) {
    let mut acct = ed.draft.clone().trimmed();
    if acct.name.is_empty() {
        acct.name = match acct.kind {
            Kind::Ssh | Kind::WebDav if !acct.address.is_empty() => acct
                .address
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split(['/', ':'])
                .next()
                .unwrap_or("")
                .to_string(),
            Kind::Folder => cloud::expand_home(&acct.address)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Folder".into()),
            k => k.label().to_string(),
        };
    }
    acct.id = ed.editing.clone().unwrap_or_else(cloud::new_id);
    let secret = ed.secret.trim().to_string();
    (acct, (!secret.is_empty()).then_some(secret))
}

fn kind_blurb(kind: Kind) -> (&'static str, Option<(&'static str, &'static str)>) {
    match kind {
        Kind::Folder => (
            "A folder on this computer: a mounted NAS share (SMB, NFS), or a Syncthing or \
             Nextcloud folder. Documents open straight from it.",
            None,
        ),
        Kind::WebDav => (
            "Nextcloud, ownCloud, Synology, QNAP, TrueNAS and most NAS boxes. Use the full \
             address of the folder, starting with http:// or https://.",
            None,
        ),
        Kind::S3 => (
            "Cloudflare R2, Wasabi, AWS S3, MinIO, iDrive e2 — anything that speaks S3.",
            None,
        ),
        Kind::B2 => (
            "Your own private Backblaze bucket — 10 GB free. The phone app can use the same bucket.",
            Some((
                "Create a free Backblaze account",
                "https://www.backblaze.com/sign-up/cloud-storage",
            )),
        ),
        Kind::Ssh => (
            "Another computer or a NAS you can reach with ssh. Documents go in a folder there.",
            None,
        ),
    }
}

fn name_hint(kind: Kind) -> &'static str {
    match kind {
        Kind::Folder => "e.g. NAS share",
        Kind::WebDav => "e.g. Nextcloud",
        Kind::S3 => "e.g. R2",
        Kind::B2 => "Backblaze B2",
        Kind::Ssh => "e.g. Laptop",
    }
}
