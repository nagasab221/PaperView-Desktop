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
/// Height of the bottom bar's row.
const BAR_H: f32 = 42.0;

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
                // The active tab's underline sits right on the rule below.
                ui.spacing_mut().item_spacing.y = 0.0;
                let full = ui.available_width();
                ui.allocate_ui_with_layout(egui::vec2(full, 44.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_height(44.0);
                    ui.add_space(12.0);
                    let tabs_w = (ui.available_width() - ACTIONS_W).max(80.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(tabs_w, 40.0),
                        Layout::left_to_right(Align::Center),
                        |ui| self.tab_strip(ui, &ctx),
                    );
                    ui.spacing_mut().item_spacing.x = 4.0;
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
            let arrow = |ui: &mut egui::Ui, glyph: &str| {
                w::button(ui, RichText::new(glyph).size(15.0), w::Btn::new(w::Kind::Quiet).enabled(enabled).min(32.0, 32.0))
            };
            if arrow(ui, "⏶").on_hover_text("Previous match (Shift+F3)").clicked() {
                self.search_step(-1);
            }
            if arrow(ui, "⏷").on_hover_text("Next match (F3, Enter)").clicked() {
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
                // No gap between the rule and the row, or the row sits low.
                ui.spacing_mut().item_spacing.y = 0.0;
                w::hairline(ui);
                // Fixed height with every control centred on it: a plain
                // horizontal row only centres against what came before.
                let full = ui.available_width();
                ui.allocate_ui_with_layout(egui::vec2(full, BAR_H), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_height(BAR_H);
                    ui.spacing_mut().item_spacing.x = 2.0;
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
        if step_button(ui, "‹", self.page_index > 0).on_hover_text("Previous page (PgUp)").clicked() {
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
        ui.add_space(4.0);
        ui.label(RichText::new(format!("of {count}")).color(p.muted));
        ui.add_space(2.0);
        if step_button(ui, "›", self.page_index + 1 < count).on_hover_text("Next page (PgDn)").clicked() {
            self.go_to_page(self.page_index + 1);
        }
    }

    /// Click: jump to the peek spot and back. Hold: peek while held.
    fn peek_eye(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        let away = self.peek.away();
        let peeking = self.peek.peeking();
        let tint = if away || peeking { p.accent } else { p.ink };
        let kind = if peeking { w::Kind::Selected } else { w::Kind::Quiet };
        let resp = w::button(
            ui,
            RichText::new(if away { "↩" } else { "👁" }).size(16.0).color(tint),
            w::Btn::new(kind).min(32.0, 32.0).sense(Sense::click_and_drag()),
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
        // Right-to-left: the first added sits furthest right. Every control is
        // the same height, the glyph buttons square, so the row lines up.
        if step_button(ui, "+", true).on_hover_text("Zoom in (Ctrl+=)").clicked() {
            self.change_zoom(Zoom::Custom(self.effective_zoom * 1.25), None);
        }
        let pct = RichText::new(format!("{:.0}%", self.effective_zoom * 100.0)).color(p.muted);
        if w::button(ui, pct, w::Btn::new(w::Kind::Quiet).min(56.0, 32.0))
            .on_hover_text("Actual size (Ctrl+0)")
            .clicked()
        {
            self.change_zoom(Zoom::Custom(1.0), None);
        }
        if step_button(ui, "−", true).on_hover_text("Zoom out (Ctrl+-)").clicked() {
            self.change_zoom(Zoom::Custom(self.effective_zoom / 1.25), None);
        }
        ui.add_space(14.0);
        let fit = |ui: &mut egui::Ui, label: &str, on: bool| {
            let kind = if on { w::Kind::Selected } else { w::Kind::Quiet };
            let rt = RichText::new(label);
            let rt = if on { rt.family(theme::medium()) } else { rt };
            w::button(ui, rt, w::Btn::new(kind).min(60.0, 32.0))
        };
        if fit(ui, "Width", self.zoom == Zoom::FitWidth).on_hover_text("Fit the page width").clicked() {
            self.change_zoom(Zoom::FitWidth, None);
        }
        if fit(ui, "Fit", self.zoom == Zoom::FitPage).on_hover_text("Fit the whole page").clicked() {
            self.change_zoom(Zoom::FitPage, None);
        }
    }

    // ── sidebar ──────────────────────────────────────────────────────────────

    /// Pages and Contents in one panel, as on the phone's drawer.
    pub(super) fn sidebar_panel(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        let mut open = self.sidebar != SidebarMode::Hidden;
        if open {
            self.sidebar_last = self.sidebar;
        }
        let shown = if open { self.sidebar } else { self.sidebar_last };
        egui::Panel::left("sidebar")
            .exact_size(236.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(p.background))
            .show_separator_line(false)
            .show_collapsible(ui, &mut open, |ui| {
                // Vertical hairline on the panel's right edge.
                let r = ui.max_rect();
                ui.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, p.hairline));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        let labels: &[&str] =
                            if self.toc.is_empty() { &["Pages"] } else { &["Pages", "Contents"] };
                        let sel = usize::from(shown == SidebarMode::Toc);
                        if let Some(i) = w::text_tabs(ui, labels, sel) {
                            self.sidebar = if i == 1 { SidebarMode::Toc } else { SidebarMode::Thumbs };
                        }
                    });
                });
                // While sliding shut, keep showing what was open.
                match shown {
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
            // Fade in over 150 ms, out over the last 350 ms.
            let alpha = (at.elapsed().as_secs_f32() / 0.15)
                .min(left.as_secs_f32() / 0.35)
                .clamp(0.0, 1.0);
            if left.is_zero() {
                self.toast = None;
            } else {
                let shift = if self.status.is_some() { -46.0 } else { 0.0 };
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, lift + shift))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ctx, |ui| {
                        ui.multiply_opacity(alpha);
                        egui::Frame::NONE
                            .fill(p.ink)
                            .corner_radius(6)
                            .inner_margin(egui::Margin::symmetric(16, 9))
                            .show(ui, |ui| {
                                ui.set_max_width(520.0);
                                ui.add(egui::Label::new(RichText::new(msg).color(p.background)).wrap());
                            });
                    });
                if alpha < 1.0 {
                    ctx.request_repaint();
                } else {
                    ctx.request_repaint_after(left.saturating_sub(Duration::from_millis(350)));
                }
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
    let rt = RichText::new(title);
    let rt = if active { rt.family(theme::medium()) } else { rt };
    let galley = egui::WidgetText::from(rt).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let hover = w::hover_t(ui, &resp, !active);
    let on = ui.ctx().animate_bool_with_time(resp.id.with("on"), active, w::HOVER_FADE);
    let color = p.muted.lerp_to_gamma(p.ink, hover.max(on));
    ui.painter().galley(rect.min, galley, color);
    if on > 0.0 {
        let bar_bottom = ui.max_rect().bottom();
        ui.painter().hline(
            rect.x_range(),
            bar_bottom - 1.0,
            Stroke::new(2.0, p.accent.gamma_multiply(on)),
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = match path {
        Some(full) => resp.on_hover_text(full),
        None => resp,
    };
    let mut closed = resp.middle_clicked();
    if path.is_some() {
        let x = w::button(
            ui,
            RichText::new("×").color(p.muted),
            w::Btn::new(w::Kind::Quiet).min(20.0, 20.0).radius(10),
        );
        if x.on_hover_text("Close (Ctrl+W)").clicked() {
            closed = true;
        }
    }
    (resp.clicked(), closed)
}

/// A square glyph button for steppers (page, zoom).
fn step_button(ui: &mut egui::Ui, glyph: &str, enabled: bool) -> egui::Response {
    w::button(
        ui,
        RichText::new(glyph).size(17.0),
        w::Btn::new(w::Kind::Quiet).enabled(enabled).min(32.0, 32.0),
    )
}
