//! Window chrome in the PaperView style: a top bar with the document tabs and
//! the actions, an optional search row under it, and — while reading — a
//! bottom bar with the page, the peek button and zoom, like the phone's.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Align2, Layout, RichText, Sense, Stroke};

use super::{
    open_file_location, truncate_title, CloudOp, HomeScreen, ScrollTarget, SidebarMode, ViewMode,
    ViewerApp, Zoom,
};
use crate::ui::{theme, widgets as w};

const TOAST_FOR: Duration = Duration::from_millis(4500);
const ACTIONS_W: f32 = 132.0;

impl ViewerApp {
    pub(super) fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub(super) fn cloud_active(&self) -> bool {
        self.cloud_enabled && self.cloud_cfg.ready()
    }

    // ── top bar ──────────────────────────────────────────────────────────────

    pub(super) fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let p = w::pal(ui);
        egui::Panel::top("topbar")
            .frame(egui::Frame::NONE.fill(p.background))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(40.0);
                    ui.add_space(12.0);
                    let tabs_w = (ui.available_width() - ACTIONS_W).max(80.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(tabs_w, 40.0),
                        Layout::left_to_right(Align::Center),
                        |ui| self.tab_strip(ui, &ctx),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
                        self.overflow_menu(ui, &ctx);
                        if self.doc.is_some() {
                            if w::icon_button(ui, "🔍", "Find in document (Ctrl+F)", self.search_open)
                                .clicked()
                            {
                                if self.search_open {
                                    self.close_search();
                                } else {
                                    self.search_open = true;
                                    self.focus_search = true;
                                }
                            }
                            let open = self.sidebar != SidebarMode::Hidden;
                            if w::icon_button(ui, "📖", "Pages and contents", open).clicked() {
                                self.sidebar = if open {
                                    SidebarMode::Hidden
                                } else if self.toc.is_empty() {
                                    SidebarMode::Thumbs
                                } else {
                                    SidebarMode::Toc
                                };
                            }
                        }
                    });
                });
                w::hairline(ui);
                if self.search_open && self.doc.is_some() {
                    self.search_row(ui);
                    w::hairline(ui);
                }
            });
    }

    /// "Library" first, then one text tab per open document.
    fn tab_strip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        let mut activate: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut home = false;
        egui::ScrollArea::horizontal()
            .id_salt("tab-strip")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if tab_item(ui, &p, "Library", None, self.active_tab.is_none()).0 {
                        home = true;
                    }
                    for (i, tab) in self.tabs.iter().enumerate() {
                        ui.add_space(10.0);
                        let (clicked, closed) = tab_item(
                            ui,
                            &p,
                            &truncate_title(&tab.title),
                            Some(&tab.path.display().to_string()),
                            self.active_tab == Some(i),
                        );
                        if clicked {
                            activate = Some(i);
                        }
                        if closed {
                            close = Some(i);
                        }
                    }
                });
            });
        if home {
            self.home = HomeScreen::Library;
            self.go_home(ctx);
        }
        if let Some(i) = activate {
            self.activate_tab(ctx, i);
        }
        if let Some(i) = close {
            self.close_tab(ctx, i);
        }
    }

    fn search_row(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let edit = w::field(ui, &mut self.search.input, "Find in document", false, 260.0);
            if std::mem::take(&mut self.focus_search) {
                edit.request_focus();
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                // Enter on the same query moves to the next match.
                if self.search.active && ui.input(|i| i.modifiers.shift) {
                    self.search_step(-1);
                } else {
                    self.submit_search();
                }
                edit.request_focus();
            }
            let enabled = self.search.active;
            if ui
                .add_enabled(enabled, egui::Button::new("⏶"))
                .on_hover_text("Previous match (Shift+F3)")
                .clicked()
            {
                self.search_step(-1);
            }
            if ui
                .add_enabled(enabled, egui::Button::new("⏷"))
                .on_hover_text("Next match (F3, Enter)")
                .clicked()
            {
                self.search_step(1);
            }
            if self.search.active {
                let (pos, total) = self.search.counter();
                let text = if !self.search.done() {
                    format!("{pos} of {total}…")
                } else if total == 0 {
                    "No matches".to_string()
                } else {
                    format!("{pos} of {total}")
                };
                ui.label(RichText::new(text).color(p.muted));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if w::inline_button(ui, "Close", true).clicked() {
                    self.close_search();
                }
            });
        });
        ui.add_space(6.0);
    }

    pub(super) fn close_search(&mut self) {
        self.search_open = false;
        self.search.deactivate();
    }

    // ── overflow menu ────────────────────────────────────────────────────────

    fn overflow_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        ui.menu_button(RichText::new("☰").size(17.0).color(p.ink), |ui| {
            ui.set_min_width(230.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let item = |text: &str, keys: &str| {
                egui::Button::new(text).shortcut_text(RichText::new(keys).color(p.muted))
            };
            if self.doc.is_some() {
                let single = self.mode == ViewMode::Single;
                if ui.add(item(if single { "Continuous scroll" } else { "Single page" }, ""))
                    .clicked()
                {
                    self.mode = if single { ViewMode::Continuous } else { ViewMode::Single };
                    self.scroll_to = Some(ScrollTarget::PageTop(self.page_index));
                    ui.close();
                }
                if ui.add(item("Rotate left", "")).clicked() {
                    self.rotate_by(-1);
                    ui.close();
                }
                if ui.add(item("Rotate right", "")).clicked() {
                    self.rotate_by(1);
                    ui.close();
                }
                ui.separator();
                let has_spot = self.peek_spot().is_some();
                let label = if has_spot { "Move peek spot here" } else { "Set peek spot here" };
                if ui.add(item(label, "Shift+Q")).clicked() {
                    self.save_peek_spot();
                    ui.close();
                }
                if has_spot && ui.add(item("Remove peek spot", "")).clicked() {
                    self.remove_peek_spot();
                    ui.close();
                }
                ui.separator();
                if ui.add_enabled(!self.printing, item("Print…", "Ctrl+P")).clicked() {
                    ui.close();
                    self.start_print(ctx);
                }
                if ui.add(item("Save a copy…", "Ctrl+S")).clicked() {
                    ui.close();
                    self.save_as();
                }
                if ui.add(item("Show in folder", "")).clicked() {
                    if let Some(path) = self.doc.as_ref().map(|d| d.path.clone()) {
                        open_file_location(&path);
                    }
                    ui.close();
                }
                if self.cloud_active() && ui.add(item("Save to cloud", "")).clicked() {
                    if let Some(path) = self.doc.as_ref().map(|d| d.path.clone()) {
                        self.start_cloud_op(ctx, CloudOp::Upload(path));
                        self.toast(format!("Uploading to {}", self.cloud_cfg.bucket.trim()));
                    }
                    ui.close();
                }
                ui.separator();
                if ui.add(item("Close document", "Ctrl+W")).clicked() {
                    ui.close();
                    self.close_doc(ctx);
                }
            } else if ui.add(item("Open a PDF…", "Ctrl+O")).clicked() {
                ui.close();
                self.open_dialog(ctx);
            }
            if ui.add(item("Settings", "Ctrl+,")).clicked() {
                ui.close();
                self.open_settings(ctx);
            }
        })
        .response
        .on_hover_text("More");
    }

    pub(super) fn open_settings(&mut self, ctx: &egui::Context) {
        self.home = HomeScreen::Settings;
        self.b2_draft = self.cloud_cfg.clone();
        self.go_home(ctx);
    }

    // ── bottom bar ───────────────────────────────────────────────────────────

    pub(super) fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let p = w::pal(ui);
        egui::Panel::bottom("bottombar")
            .frame(egui::Frame::NONE.fill(p.background))
            .show_separator_line(false)
            .show(ui, |ui| {
                w::hairline(ui);
                ui.horizontal(|ui| {
                    ui.set_min_height(38.0);
                    ui.add_space(10.0);
                    self.page_controls(ui);
                    if self.peek_spot().is_some() {
                        ui.add_space(6.0);
                        self.peek_eye(ui, &ctx);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(10.0);
                        self.zoom_controls(ui);
                    });
                });
            });
    }

    fn page_controls(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        let count = self.page_count();
        if ui.add(egui::Button::new("‹")).on_hover_text("Previous page (PgUp)").clicked() {
            self.go_to_page(self.page_index - 1);
        }
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.page_field)
                .desired_width(34.0)
                .horizontal_align(Align::Center)
                .background_color(p.surface),
        );
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Ok(n) = self.page_field.trim().parse::<i32>() {
                if (1..=count).contains(&n) {
                    self.jump_with_history(n - 1);
                }
            }
        }
        if !field.has_focus() {
            self.page_field = (self.page_index + 1).to_string();
        }
        ui.label(RichText::new(format!("of {count}")).color(p.muted));
        if ui.add(egui::Button::new("›")).on_hover_text("Next page (PgDn)").clicked() {
            self.go_to_page(self.page_index + 1);
        }
    }

    /// Click: jump to the peek spot and back. Hold: peek while held.
    fn peek_eye(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        let away = self.peek.away();
        let peeking = self.peek.peeking();
        let tint = if away || peeking { p.accent } else { p.ink };
        let resp = ui
            .add(
                egui::Button::new(RichText::new(if away { "↩" } else { "👁" }).size(16.0).color(tint))
                    .fill(if peeking { p.accent_wash() } else { egui::Color32::TRANSPARENT })
                    .min_size(egui::Vec2::splat(30.0))
                    .sense(Sense::click_and_drag()),
            )
            .on_hover_text(if away {
                "Back to where you were (Q)"
            } else {
                "Peek — hold to look, click to jump there (hold Q / tap Q)"
            });
        let down = resp.is_pointer_button_down_on();
        self.peek_button(ctx, down);
    }

    fn zoom_controls(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        // Right-to-left: the first added sits furthest right.
        if ui.add(egui::Button::new("+")).on_hover_text("Zoom in (Ctrl+=)").clicked() {
            self.change_zoom(Zoom::Custom(self.effective_zoom * 1.25), None);
        }
        if ui
            .add(
                egui::Button::new(
                    RichText::new(format!("{:.0}%", self.effective_zoom * 100.0)).color(p.muted),
                )
                .min_size(egui::vec2(52.0, 0.0)),
            )
            .on_hover_text("Actual size (Ctrl+0)")
            .clicked()
        {
            self.change_zoom(Zoom::Custom(1.0), None);
        }
        if ui.add(egui::Button::new("−")).on_hover_text("Zoom out (Ctrl+-)").clicked() {
            self.change_zoom(Zoom::Custom(self.effective_zoom / 1.25), None);
        }
        ui.add_space(8.0);
        if w::tool_button(ui, "Width", self.zoom == Zoom::FitWidth).clicked() {
            self.change_zoom(Zoom::FitWidth, None);
        }
        if w::tool_button(ui, "Fit", self.zoom == Zoom::FitPage).clicked() {
            self.change_zoom(Zoom::FitPage, None);
        }
    }

    // ── sidebar ──────────────────────────────────────────────────────────────

    /// Pages and Contents in one panel, as on the phone's drawer.
    pub(super) fn sidebar_panel(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        egui::Panel::left("sidebar")
            .exact_size(236.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(p.background))
            .show_separator_line(false)
            .show(ui, |ui| {
                // Vertical hairline on the panel's right edge.
                let r = ui.max_rect();
                ui.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, p.hairline));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        let labels: &[&str] =
                            if self.toc.is_empty() { &["Pages"] } else { &["Pages", "Contents"] };
                        let sel = usize::from(self.sidebar == SidebarMode::Toc);
                        if let Some(i) = w::text_tabs(ui, labels, sel) {
                            self.sidebar = if i == 1 { SidebarMode::Toc } else { SidebarMode::Thumbs };
                        }
                    });
                });
                match self.sidebar {
                    SidebarMode::Toc => self.toc_body(ui),
                    _ => self.thumbs_body(ui),
                }
            });
    }

    // ── overlays ─────────────────────────────────────────────────────────────

    /// The transient toast and any error banner, floating above the bottom bar.
    pub(super) fn overlays(&mut self, ctx: &egui::Context) {
        let p = theme::palette(self.skin, self.theme.is_dark(ctx));
        let lift = if self.doc.is_some() { -58.0 } else { -24.0 };

        if let Some(msg) = self.status.clone() {
            let mut dismiss = false;
            egui::Area::new(egui::Id::new("status-banner"))
                .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, lift))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.hairline))
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.set_max_width(560.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&msg).color(ui.visuals().error_fg_color),
                                )
                                .wrap(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                if w::inline_button(ui, "Dismiss", true).clicked() {
                                    dismiss = true;
                                }
                            });
                        });
                });
            if dismiss {
                self.status = None;
            }
        }

        if let Some((msg, at)) = &self.toast {
            let left = TOAST_FOR.saturating_sub(at.elapsed());
            if left.is_zero() {
                self.toast = None;
            } else {
                let shift = if self.status.is_some() { -46.0 } else { 0.0 };
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, lift + shift))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ctx, |ui| {
                        egui::Frame::NONE
                            .fill(p.ink)
                            .corner_radius(6)
                            .inner_margin(egui::Margin::symmetric(16, 9))
                            .show(ui, |ui| {
                                ui.set_max_width(520.0);
                                ui.add(egui::Label::new(RichText::new(msg).color(p.background)).wrap());
                            });
                    });
                ctx.request_repaint_after(left);
            }
        }
    }
}

/// One tab in the strip: (clicked, close clicked). `path` makes it closable.
fn tab_item(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    title: &str,
    path: Option<&str>,
    active: bool,
) -> (bool, bool) {
    let color = if active { p.ink } else { p.muted };
    let text = RichText::new(title).color(color);
    let text = if active { text.family(theme::medium()) } else { text };
    let resp = ui.add(egui::Label::new(text).sense(Sense::click()).selectable(false));
    let resp = match path {
        Some(p) => resp.on_hover_text(p),
        None => resp,
    };
    if active {
        let r = resp.rect;
        let bar_bottom = ui.max_rect().bottom();
        ui.painter().hline(r.x_range(), bar_bottom - 1.0, Stroke::new(2.0, p.accent));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let mut closed = resp.middle_clicked();
    if path.is_some() {
        let x = ui.add(
            egui::Button::new(RichText::new("×").color(p.muted))
                .frame(false)
                .min_size(egui::vec2(16.0, 16.0)),
        );
        if x.on_hover_text("Close (Ctrl+W)").clicked() {
            closed = true;
        }
    }
    (resp.clicked(), closed)
}
