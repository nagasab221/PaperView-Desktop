//! Window chrome in the PaperView style: a top bar with the document tabs and
//! the actions, an optional search row under it, and — while reading — a
//! bottom bar with the page, the peek button and zoom, like the phone's.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Align2, Layout, RichText, Sense, Stroke};

use super::{
    open_file_location, truncate_title, HomeScreen, ScrollTarget, SidebarMode, ViewMode, ViewerApp,
    Zoom,
};
use crate::ui::{theme, widgets as w};

const TOAST_FOR: Duration = Duration::from_millis(4500);
const ACTIONS_W: f32 = 90.0;
/// Height of a document tab (the tab strip's row).
const TAB_H: f32 = 44.0;
/// Height of PaperView's own title bar, and the width of its window buttons.
const TITLE_H: f32 = 34.0;
const WIN_BTN_W: f32 = 44.0;
/// Height of the bottom bar's row.
const BAR_H: f32 = 42.0;

impl ViewerApp {
    pub(super) fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub(super) fn cloud_active(&self) -> bool {
        self.cloud_enabled && self.cloud_cfg.ready()
    }

    // ── full screen ──────────────────────────────────────────────────────────

    /// Full screen with nothing but the pages: no title bar, tabs, bars or
    /// sidebar. Esc or F11 leaves.
    pub(super) fn set_immersive(&mut self, ctx: &egui::Context, on: bool) {
        if self.immersive == on {
            return;
        }
        self.immersive = on;
        self.immersive_stir = Some(Instant::now());
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    /// In full screen, a small floating bar with the page and the way out.
    /// It shows while the pointer moves (or rests near the bottom) and fades
    /// away while you read; the pointer hides too.
    pub(super) fn immersive_controls(&mut self, ctx: &egui::Context) {
        const LINGER: Duration = Duration::from_millis(2200);
        let p = w::pal_ctx(ctx);
        let (moved, pointer) = ctx.input(|i| {
            (
                i.pointer.delta() != egui::Vec2::ZERO || i.pointer.any_pressed(),
                i.pointer.hover_pos(),
            )
        });
        if moved {
            self.immersive_stir = Some(Instant::now());
        }
        let screen = ctx.content_rect();
        let near_bottom = pointer.is_some_and(|pos| pos.y > screen.bottom() - 110.0);
        let busy = ctx.memory(|m| m.has_focus(egui::Id::new("page-field")))
            || egui::Popup::is_id_open(ctx, egui::Id::new("reading-popover"));
        let recent = self
            .immersive_stir
            .map(|t| t.elapsed())
            .filter(|e| *e < LINGER);
        let visible = recent.is_some() || near_bottom || busy;
        if let Some(e) = recent {
            ctx.request_repaint_after(LINGER - e);
        }
        let t = ctx.animate_bool_with_time(egui::Id::new("immersive-pill"), visible, 0.25);
        if !visible && pointer.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        if t <= 0.0 {
            return;
        }
        egui::Area::new(egui::Id::new("immersive-pill"))
            .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -22.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::NONE
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.hairline))
                    .corner_radius(22)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 6],
                        blur: 26,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(if p.dark { 130 } else { 46 }),
                    })
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if self.book.is_some() {
                                self.book_page_controls(ui);
                                ui.add_space(8.0);
                                self.reading_button(ui);
                            } else {
                                self.page_controls(ui);
                            }
                            ui.add_space(4.0);
                            if w::icon_button(ui, "⛶", "Leave full screen (Esc, F11)", true)
                                .clicked()
                            {
                                self.set_immersive(ctx, false);
                            }
                        });
                    });
            });
    }

    // ── title bar ────────────────────────────────────────────────────────────

    /// PaperView draws its own title bar, in the app's colours: the open
    /// document's name in the middle, window buttons on the right. Drag it to
    /// move the window, double-click to maximise. The window edges resize.
    pub(super) fn title_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let p = w::pal(ui);
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let title = match &self.book {
            Some(v) => Some(v.title.clone()),
            None => self
                .active_tab
                .and_then(|i| self.tabs.get(i))
                .map(|t| t.title.clone()),
        };
        let fill = if self.tabs.is_empty() {
            p.background
        } else {
            p.tray()
        };
        egui::Panel::top("titlebar")
            .exact_size(TITLE_H)
            .frame(egui::Frame::NONE.fill(fill))
            .show_separator_line(false)
            .show(ui, |ui| {
                let bar = ui.max_rect();
                // The whole bar drags the window; buttons drawn on top win.
                let drag =
                    ui.interact(bar, egui::Id::new("titlebar-drag"), Sense::click_and_drag());
                if drag.double_clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                } else if drag.drag_started_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                // The app menu lives in the left corner, as in Telegram.
                let corner = egui::Rect::from_min_size(
                    bar.min + egui::vec2(6.0, 1.0),
                    egui::vec2(40.0, TITLE_H - 2.0),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(corner)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| self.overflow_menu(ui, &ctx),
                );
                if self.tabs.is_empty() {
                    ui.painter().hline(
                        bar.x_range(),
                        bar.bottom() - 0.5,
                        Stroke::new(1.0, p.hairline),
                    );
                }

                // Title, centred on the whole window rather than the free space.
                let (name, app) = match &title {
                    Some(t) => (t.clone(), "  —  PaperView"),
                    None => ("PaperView".to_string(), ""),
                };
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    &name,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::new(13.5, theme::medium()),
                        color: p.ink,
                        ..Default::default()
                    },
                );
                job.append(
                    app,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::new(13.5, egui::FontFamily::Proportional),
                        color: p.muted,
                        ..Default::default()
                    },
                );
                let room = (bar.width() - 3.0 * WIN_BTN_W * 2.0 - 24.0).max(60.0);
                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let pos = bar.center() - galley.size() * 0.5;
                ui.painter().galley(pos, galley, p.ink);

                // Window buttons, right to left.
                let mut x = bar.right();
                for which in [WinBtn::Close, WinBtn::Maximize, WinBtn::Minimize] {
                    x -= WIN_BTN_W;
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(x, bar.top()),
                        egui::vec2(WIN_BTN_W, TITLE_H),
                    );
                    let resp =
                        ui.interact(rect, egui::Id::new(("winbtn", which as u8)), Sense::click());
                    let t = w::hover_t(ui, &resp, true);
                    let close = which == WinBtn::Close;
                    let red = egui::Color32::from_rgb(0xD9, 0x3B, 0x30);
                    let fill = if close {
                        red.gamma_multiply(t)
                    } else {
                        p.hover().gamma_multiply(t)
                    };
                    ui.painter().rect_filled(rect, 0, fill);
                    let ink = if close {
                        p.ink.lerp_to_gamma(egui::Color32::WHITE, t)
                    } else {
                        p.ink
                    };
                    let behind = fill.lerp_to_gamma(p.hover(), t);
                    paint_win_glyph(ui.painter(), rect.center(), which, maximized, ink, behind);
                    if resp.clicked() {
                        ctx.send_viewport_cmd(match which {
                            WinBtn::Close => egui::ViewportCommand::Close,
                            WinBtn::Maximize => egui::ViewportCommand::Maximized(!maximized),
                            WinBtn::Minimize => egui::ViewportCommand::Minimized(true),
                        });
                    }
                    resp.on_hover_text(match which {
                        WinBtn::Close => "Close",
                        WinBtn::Maximize if maximized => "Restore",
                        WinBtn::Maximize => "Maximise",
                        WinBtn::Minimize => "Minimise",
                    });
                }
            });
    }

    /// Without the system frame, the edges have to resize the window themselves.
    pub(super) fn window_edges(&mut self, ctx: &egui::Context) {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if maximized || fullscreen {
            return;
        }
        let screen = ctx.content_rect();
        let p = theme::palette(self.skin, self.theme.is_dark(ctx));
        // A hairline frame, so the window has an edge on any wallpaper.
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("window-edge"),
        ))
        .rect_stroke(
            screen,
            0,
            Stroke::new(1.0, p.hairline.lerp_to_gamma(p.muted, 0.35)),
            egui::StrokeKind::Inside,
        );

        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        const GRIP: f32 = 6.0;
        const CORNER: f32 = 14.0;
        let (l, r, t, b) = (
            pos.x - screen.left() < GRIP,
            screen.right() - pos.x < GRIP,
            pos.y - screen.top() < GRIP,
            screen.bottom() - pos.y < GRIP,
        );
        // Corners are a little more generous than the straight edges.
        let near = |d: f32| d < CORNER;
        let (cl, cr, ct, cb) = (
            near(pos.x - screen.left()),
            near(screen.right() - pos.x),
            near(pos.y - screen.top()),
            near(screen.bottom() - pos.y),
        );
        use egui::viewport::ResizeDirection as D;
        let dir = match () {
            _ if (t && cl) || (l && ct) => Some(D::NorthWest),
            _ if (t && cr) || (r && ct) => Some(D::NorthEast),
            _ if (b && cl) || (l && cb) => Some(D::SouthWest),
            _ if (b && cr) || (r && cb) => Some(D::SouthEast),
            _ if l => Some(D::West),
            _ if r => Some(D::East),
            _ if t => Some(D::North),
            _ if b => Some(D::South),
            _ => None,
        };
        let Some(dir) = dir else { return };
        ctx.set_cursor_icon(match dir {
            D::North | D::South => egui::CursorIcon::ResizeVertical,
            D::East | D::West => egui::CursorIcon::ResizeHorizontal,
            D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe,
            D::NorthEast | D::SouthWest => egui::CursorIcon::ResizeNeSw,
        });
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }

    // ── top bar ──────────────────────────────────────────────────────────────

    pub(super) fn top_bar(&mut self, ui: &mut egui::Ui) {
        // With nothing open, a strip holding only "Library" is just noise.
        if self.tabs.is_empty() {
            return;
        }
        let ctx = ui.ctx().clone();
        let p = w::pal(ui);
        self.active_tab_span = None;
        egui::Panel::top("topbar")
            .frame(egui::Frame::NONE.fill(p.tray()))
            .show_separator_line(false)
            .show(ui, |ui| {
                // The active tab's underline sits right on the rule below.
                ui.spacing_mut().item_spacing.y = 0.0;
                let full = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(full, TAB_H),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_min_height(TAB_H);
                        // Tabs fill exactly this band, wherever egui lays them out.
                        let band = ui.max_rect().y_range();
                        ui.add_space(4.0);
                        let tabs_w = (ui.available_width() - ACTIONS_W).max(80.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(tabs_w, TAB_H),
                            Layout::left_to_right(Align::Center),
                            |ui| self.tab_strip(ui, &ctx, band),
                        );
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(8.0);
                            if self.book.is_some() {
                                let open = self.sidebar != SidebarMode::Hidden;
                                if w::icon_button(ui, "📖", "Contents", open).clicked() {
                                    self.sidebar = if open {
                                        SidebarMode::Hidden
                                    } else {
                                        SidebarMode::Toc
                                    };
                                }
                            }
                            if self.doc.is_some() {
                                if w::icon_button(
                                    ui,
                                    "🔍",
                                    "Find in document (Ctrl+F)",
                                    self.search_open,
                                )
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
                                if w::icon_button(ui, "📖", "Pages and contents", open).clicked()
                                {
                                    self.sidebar = if open {
                                        SidebarMode::Hidden
                                    } else if self.toc.is_empty() {
                                        SidebarMode::Thumbs
                                    } else {
                                        SidebarMode::Toc
                                    };
                                }
                                // Leftmost of the icons: right-to-left, added last.
                                let doc_path = self.doc.as_ref().map(|d| d.path.clone());
                                if let Some(path) = doc_path.filter(|p| self.can_upload(p)) {
                                    if w::icon_button(ui, "☁", "Save to cloud…", false).clicked()
                                    {
                                        self.ask_upload(&ctx, path);
                                    }
                                }
                            }
                        });
                    },
                );
                // The rule under the tabs, broken where the active card joins
                // the page below.
                let (rule, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
                let stroke = Stroke::new(1.0, p.hairline);
                match self.active_tab_span {
                    Some(span) => {
                        ui.painter()
                            .hline(rule.left()..=span.min, rule.center().y, stroke);
                        ui.painter()
                            .hline(span.max..=rule.right(), rule.center().y, stroke);
                    }
                    None => {
                        ui.painter().hline(rule.x_range(), rule.center().y, stroke);
                    }
                }
                if self.search_open && self.doc.is_some() {
                    egui::Frame::NONE.fill(p.background).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.search_row(ui);
                    });
                    w::hairline(ui);
                }
            });
    }

    /// "Library" first, then one text tab per open document.
    fn tab_strip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, band: egui::Rangef) {
        let p = w::pal(ui);
        let mut activate: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut home = false;
        let mut span: Option<egui::Rect> = None;
        let clip = ui.clip_rect();
        egui::ScrollArea::horizontal()
            .id_salt("tab-strip")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(TAB_H);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let lib = tab_item(ui, &p, band, "Library", None, self.active_tab.is_none());
                    if lib.0 {
                        home = true;
                    }
                    if self.active_tab.is_none() {
                        span = Some(lib.2);
                    }
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let (clicked, closed, rect) = tab_item(
                            ui,
                            &p,
                            band,
                            &truncate_title(&tab.title),
                            Some(&tab.path.display().to_string()),
                            self.active_tab == Some(i),
                        );
                        if self.active_tab == Some(i) {
                            span = Some(rect);
                        }
                        if clicked {
                            activate = Some(i);
                        }
                        if closed {
                            close = Some(i);
                        }
                    }
                });
            });
        // Where the active card meets the rule, clipped to what's visible.
        self.active_tab_span = span
            .map(|r| r.x_range())
            .filter(|r| r.max > clip.left() && r.min < clip.right())
            .map(|r| egui::Rangef::new(r.min.max(clip.left()), r.max.min(clip.right())));
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
                w::button(
                    ui,
                    RichText::new(glyph).size(15.0),
                    w::Btn::new(w::Kind::Quiet).enabled(enabled).min(32.0, 32.0),
                )
            };
            if arrow(ui, "⏶")
                .on_hover_text("Previous match (Shift+F3)")
                .clicked()
            {
                self.search_step(-1);
            }
            if arrow(ui, "⏷")
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
                if ui
                    .add(item(
                        if single {
                            "Continuous scroll"
                        } else {
                            "Single page"
                        },
                        "",
                    ))
                    .clicked()
                {
                    self.mode = if single {
                        ViewMode::Continuous
                    } else {
                        ViewMode::Single
                    };
                    self.scroll_to = Some(ScrollTarget::PageTop(self.page_index));
                    ui.close();
                }
                if ui.add(item("Full screen", "F11")).clicked() {
                    ui.close();
                    self.set_immersive(ctx, true);
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
                let label = if has_spot {
                    "Move peek spot here"
                } else {
                    "Set peek spot here"
                };
                if ui.add(item(label, "Shift+Q")).clicked() {
                    self.save_peek_spot();
                    ui.close();
                }
                if has_spot && ui.add(item("Remove peek spot", "")).clicked() {
                    self.remove_peek_spot();
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(!self.printing, item("Print…", "Ctrl+P"))
                    .clicked()
                {
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
                let doc_path = self.doc.as_ref().map(|d| d.path.clone());
                if let Some(path) = doc_path.filter(|p| self.can_upload(p)) {
                    if ui.add(item("Save to cloud…", "")).clicked() {
                        ui.close();
                        self.ask_upload(ctx, path);
                    }
                }
                ui.separator();
                if ui.add(item("Close document", "Ctrl+W")).clicked() {
                    ui.close();
                    self.close_doc(ctx);
                }
            } else if self.book.is_some() {
                if ui
                    .add(item(
                        if self.book_spread {
                            "One page at a time"
                        } else {
                            "Two pages"
                        },
                        "",
                    ))
                    .clicked()
                {
                    self.set_book_spread(!self.book_spread);
                    ui.close();
                }
                if ui.add(item("Full screen", "F11")).clicked() {
                    ui.close();
                    self.set_immersive(ctx, true);
                }
                if ui.add(item("Go to page…", "Ctrl+G")).clicked() {
                    ui.close();
                    self.book_page_field.clear();
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new("page-field")));
                }
                if ui.add(item("Typeface and spacing…", "")).clicked() {
                    ui.close();
                    egui::Popup::open_id(ctx, egui::Id::new("reading-popover"));
                }
                if ui.add(item("Larger text", "Ctrl+=")).clicked() {
                    self.bump_book_size(1.0);
                    ui.close();
                }
                if ui.add(item("Smaller text", "Ctrl+-")).clicked() {
                    self.bump_book_size(-1.0);
                    ui.close();
                }
                ui.separator();
                if ui.add(item("Show in folder", "")).clicked() {
                    if let Some(path) = self.book.as_ref().map(|v| v.path.clone()) {
                        open_file_location(&path);
                    }
                    ui.close();
                }
                if ui.add(item("Close book", "Ctrl+W")).clicked() {
                    ui.close();
                    self.close_doc(ctx);
                }
                ui.separator();
                if ui.add(item("Open a PDF or book…", "Ctrl+O")).clicked() {
                    ui.close();
                    self.open_dialog(ctx);
                }
            } else if ui.add(item("Open a PDF or book…", "Ctrl+O")).clicked() {
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
                ui.allocate_ui_with_layout(
                    egui::vec2(full, BAR_H),
                    Layout::left_to_right(Align::Center),
                    |ui| {
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
                    },
                );
            });
    }

    pub(super) fn page_controls(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        let count = self.page_count();
        if step_button(ui, "‹", self.page_index > 0)
            .on_hover_text("Previous page (PgUp)")
            .clicked()
        {
            self.go_to_page(self.page_index - 1);
        }
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.page_field)
                .id(egui::Id::new("page-field"))
                .desired_width(34.0)
                .horizontal_align(Align::Center)
                .background_color(p.surface),
        );
        w::select_all_on_focus(ui, &field, &self.page_field);
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
        if step_button(ui, "›", self.page_index + 1 < count)
            .on_hover_text("Next page (PgDn)")
            .clicked()
        {
            self.go_to_page(self.page_index + 1);
        }
    }

    /// Click: jump to the peek spot and back. Hold: peek while held.
    fn peek_eye(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = w::pal(ui);
        let away = self.peek.away();
        let peeking = self.peek.peeking();
        let tint = if away || peeking { p.accent } else { p.ink };
        let kind = if peeking {
            w::Kind::Selected
        } else {
            w::Kind::Quiet
        };
        let resp = w::button(
            ui,
            RichText::new(if away { "↩" } else { "👁" })
                .size(16.0)
                .color(tint),
            w::Btn::new(kind)
                .min(32.0, 32.0)
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
        // Right-to-left: the first added sits furthest right. Every control is
        // the same height, the glyph buttons square, so the row lines up.
        if step_button(ui, "+", true)
            .on_hover_text("Zoom in (Ctrl+=)")
            .clicked()
        {
            self.change_zoom(Zoom::Custom(self.effective_zoom * 1.25), None);
        }
        let pct = RichText::new(format!("{:.0}%", self.effective_zoom * 100.0)).color(p.muted);
        if w::button(ui, pct, w::Btn::new(w::Kind::Quiet).min(56.0, 32.0))
            .on_hover_text("Actual size (Ctrl+0)")
            .clicked()
        {
            self.change_zoom(Zoom::Custom(1.0), None);
        }
        if step_button(ui, "−", true)
            .on_hover_text("Zoom out (Ctrl+-)")
            .clicked()
        {
            self.change_zoom(Zoom::Custom(self.effective_zoom / 1.25), None);
        }
        ui.add_space(14.0);
        let fit = |ui: &mut egui::Ui, label: &str, on: bool| {
            let kind = if on {
                w::Kind::Selected
            } else {
                w::Kind::Quiet
            };
            let rt = RichText::new(label);
            let rt = if on { rt.family(theme::medium()) } else { rt };
            w::button(ui, rt, w::Btn::new(kind).min(60.0, 32.0))
        };
        if fit(ui, "Width", self.zoom == Zoom::FitWidth)
            .on_hover_text("Fit the page width")
            .clicked()
        {
            self.change_zoom(Zoom::FitWidth, None);
        }
        if fit(ui, "Fit", self.zoom == Zoom::FitPage)
            .on_hover_text("Fit the whole page")
            .clicked()
        {
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
        let shown = if open {
            self.sidebar
        } else {
            self.sidebar_last
        };
        egui::Panel::left("sidebar")
            .exact_size(236.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(p.background))
            .show_separator_line(false)
            .show_collapsible(ui, &mut open, |ui| {
                // Vertical hairline on the panel's right edge.
                let r = ui.max_rect();
                ui.painter()
                    .vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, p.hairline));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        let labels: &[&str] = if self.toc.is_empty() {
                            &["Pages"]
                        } else {
                            &["Pages", "Contents"]
                        };
                        let sel = usize::from(shown == SidebarMode::Toc);
                        if let Some(i) = w::text_tabs(ui, labels, sel) {
                            self.sidebar = if i == 1 {
                                SidebarMode::Toc
                            } else {
                                SidebarMode::Thumbs
                            };
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
                                ui.add(
                                    egui::Label::new(RichText::new(msg).color(p.background)).wrap(),
                                );
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

/// One tab in the strip: (clicked, close clicked, its rect). `path` makes it
/// closable.
///
/// Index cards: the active tab is a card in the page's own colour, with
/// hairline edges, rounded top corners and a thin accent line along its top,
/// open at the bottom where it joins the page. Other tabs are plain text in
/// the tray, showing a faint card outline on hover.
fn tab_item(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    band: egui::Rangef,
    title: &str,
    path: Option<&str>,
    active: bool,
) -> (bool, bool, egui::Rect) {
    const PAD: f32 = 14.0;
    const CLOSE: f32 = 20.0;
    const LIFT: f32 = 7.0;
    let rt = RichText::new(title);
    let rt = if active {
        rt.family(theme::medium())
    } else {
        rt
    };
    let galley = egui::WidgetText::from(rt).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let closable = path.is_some();
    let width = PAD + galley.size().x + if closable { 6.0 + CLOSE + 6.0 } else { PAD };
    // Take the width in the layout, but draw and click across the bar's band.
    let (slot, _) = ui.allocate_exact_size(egui::vec2(width, 20.0), Sense::hover());
    let rect = egui::Rect::from_x_y_ranges(slot.x_range(), band);
    let resp = ui.interact(rect, ui.id().with(("tab", title, path)), Sense::click());

    let hover = w::hover_t(ui, &resp, !active);
    let on = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("on"), active, w::HOVER_FADE);
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("tab-cards"),
        ))
        .with_clip_rect(ui.clip_rect().expand2(egui::vec2(0.0, 2.0)));

    // The card: from a little below the top of the band down over the rule.
    let card = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + LIFT),
        egui::pos2(rect.right(), rect.bottom() + 1.0),
    );
    let corners = egui::CornerRadius {
        nw: 6,
        ne: 6,
        sw: 0,
        se: 0,
    };
    if on > 0.0 {
        painter.rect(
            card,
            corners,
            p.tray().lerp_to_gamma(p.background, on),
            Stroke::new(1.0, p.hairline.gamma_multiply(on)),
            egui::StrokeKind::Inside,
        );
        // Open at the bottom: the card runs on into the page.
        painter.hline(
            card.left() + 1.0..=card.right() - 1.0,
            card.bottom() - 0.5,
            Stroke::new(1.5, p.tray().lerp_to_gamma(p.background, on)),
        );
        painter.hline(
            card.left() + 5.0..=card.right() - 5.0,
            card.top() + 1.0,
            Stroke::new(2.0, p.accent.gamma_multiply(on)),
        );
    } else if hover > 0.0 {
        painter.rect(
            card.with_max_y(rect.bottom() - 1.0),
            corners,
            p.hover().gamma_multiply(hover * 0.7),
            Stroke::new(1.0, p.hairline.gamma_multiply(hover * 0.8)),
            egui::StrokeKind::Inside,
        );
    }

    let text_color = p
        .muted
        .lerp_to_gamma(p.ink, hover.max(on).max(if closable { 0.7 } else { 0.0 }));
    let text_pos = egui::pos2(rect.left() + PAD, card.center().y - galley.size().y * 0.5);
    painter.galley(text_pos, galley, text_color);

    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = match path {
        Some(full) => resp.on_hover_text(full),
        None => resp,
    };
    let mut closed = resp.middle_clicked();
    if closable {
        // The close button sits inside the tab and takes its own clicks.
        let x_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 6.0 - CLOSE * 0.5, card.center().y),
            egui::vec2(CLOSE, CLOSE),
        );
        let x = ui.interact(x_rect, resp.id.with("close"), Sense::click());
        let xt = w::hover_t(ui, &x, true);
        if xt > 0.0 {
            painter.circle_filled(x_rect.center(), CLOSE * 0.5, p.pressed().gamma_multiply(xt));
        }
        let c = x_rect.center();
        let k = 3.5;
        let stroke = Stroke::new(1.3, p.muted.lerp_to_gamma(p.ink, xt.max(on * 0.6)));
        painter.line_segment([c + egui::vec2(-k, -k), c + egui::vec2(k, k)], stroke);
        painter.line_segment([c + egui::vec2(-k, k), c + egui::vec2(k, -k)], stroke);
        if x.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if x.on_hover_text("Close (Ctrl+W)").clicked() {
            closed = true;
        }
    }
    (resp.clicked() && !closed, closed, card)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WinBtn {
    Minimize,
    Maximize,
    Close,
}

/// Window-button glyphs, drawn with lines so they're crisp at any scale.
fn paint_win_glyph(
    painter: &egui::Painter,
    c: egui::Pos2,
    which: WinBtn,
    maximized: bool,
    ink: egui::Color32,
    behind: egui::Color32,
) {
    let s = Stroke::new(1.2, ink);
    let h = 5.0;
    match which {
        WinBtn::Minimize => {
            painter.hline(c.x - h..=c.x + h, c.y, s);
        }
        WinBtn::Maximize if maximized => {
            // Restore: two overlapping squares.
            let back =
                egui::Rect::from_center_size(c + egui::vec2(1.5, -1.5), egui::vec2(8.0, 8.0));
            let front =
                egui::Rect::from_center_size(c + egui::vec2(-1.5, 1.5), egui::vec2(8.0, 8.0));
            painter.rect_stroke(back, 1, s, egui::StrokeKind::Middle);
            painter.rect_filled(front, 1, behind);
            painter.rect_stroke(front, 1, s, egui::StrokeKind::Middle);
        }
        WinBtn::Maximize => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)),
                1,
                s,
                egui::StrokeKind::Middle,
            );
        }
        WinBtn::Close => {
            painter.line_segment([c + egui::vec2(-h, -h), c + egui::vec2(h, h)], s);
            painter.line_segment([c + egui::vec2(-h, h), c + egui::vec2(h, -h)], s);
        }
    }
}

/// A square glyph button for steppers (page, zoom).
pub(super) fn step_button(ui: &mut egui::Ui, glyph: &str, enabled: bool) -> egui::Response {
    w::button(
        ui,
        RichText::new(glyph).size(17.0),
        w::Btn::new(w::Kind::Quiet).enabled(enabled).min(32.0, 32.0),
    )
}
