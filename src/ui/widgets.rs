//! The small set of components every screen is built from — the desktop
//! counterpart of the Android app's `ui/components/Paper.kt`. Screens should
//! compose these rather than reach for egui's defaults; that is what keeps
//! the app looking like one thing.

use eframe::egui::{self, Align, Color32, Layout, Response, RichText, Sense, Stroke, Ui, Vec2};

use super::theme::{self, Palette};

/// The palette currently in force, stashed in egui's memory by the app.
pub fn pal(ui: &Ui) -> Palette {
    ui.ctx()
        .data(|d| d.get_temp::<Palette>(egui::Id::new("paper-palette")))
        .unwrap_or_else(|| theme::palette(theme::Skin::Paper, ui.visuals().dark_mode))
}

pub fn set_pal(ctx: &egui::Context, p: Palette) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("paper-palette"), p));
}

/// The single hairline used wherever a boundary is needed.
pub fn hairline(ui: &mut Ui) {
    let p = pal(ui);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.hairline));
}

/// A section label: small, letterspaced, uppercase, over a rule.
pub fn section_heading(ui: &mut Ui, text: &str, trailing: impl FnOnce(&mut Ui)) {
    let p = pal(ui);
    ui.add_space(26.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(text.to_uppercase())
                .text_style(theme::label())
                .extra_letter_spacing(1.2)
                .color(p.muted),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), trailing);
    });
    ui.add_space(4.0);
    hairline(ui);
}

/// Large editorial page heading.
pub fn masthead(ui: &mut Ui, title: &str, subtitle: Option<&str>, actions: impl FnOnce(&mut Ui)) {
    let p = pal(ui);
    ui.add_space(22.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).text_style(theme::display()).color(p.ink));
            if let Some(sub) = subtitle {
                ui.label(RichText::new(sub).small().color(p.muted));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), actions);
    });
}

/// A frameless accent-coloured text button.
pub fn inline_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let p = pal(ui);
    let color = if enabled { p.accent } else { p.muted };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).family(theme::medium()).color(color)).frame(false),
    )
}

/// A quiet text button for toolbars; `on` marks the current choice.
pub fn tool_button(ui: &mut Ui, text: &str, on: bool) -> Response {
    let p = pal(ui);
    let rt = RichText::new(text).color(if on { p.accent } else { p.ink });
    let rt = if on { rt.family(theme::medium()) } else { rt };
    ui.add(egui::Button::new(rt).fill(if on { p.accent_wash() } else { Color32::TRANSPARENT }))
}

/// The primary action: solid accent.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let p = pal(ui);
    let (fill, fg) = if enabled { (p.accent, p.on_accent) } else { (p.hover(), p.muted) };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).family(theme::medium()).color(fg))
            .fill(fill)
            .min_size(egui::vec2(0.0, 32.0)),
    )
}

/// A secondary action: hairline outline.
pub fn outlined_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let p = pal(ui);
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(if enabled { p.ink } else { p.muted }))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, p.hairline))
            .min_size(egui::vec2(0.0, 32.0)),
    )
}

/// A single-line text field on a hairline frame.
pub fn field(ui: &mut Ui, value: &mut String, hint: &str, password: bool, width: f32) -> Response {
    let p = pal(ui);
    egui::Frame::NONE
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.hairline))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .frame(egui::Frame::NONE)
                    .password(password)
                    .hint_text(RichText::new(hint).color(p.muted))
                    .desired_width(width),
            )
        })
        .inner
}

/// A small caption above a field.
pub fn field_label(ui: &mut Ui, text: &str) {
    let p = pal(ui);
    ui.label(RichText::new(text).small().color(p.muted));
}

/// A row: optional glyph, title, description, trailing content. A
/// `clickable` row highlights on hover and reports clicks.
pub fn row(
    ui: &mut Ui,
    clickable: bool,
    glyph: Option<&str>,
    title: &str,
    subtitle: Option<&str>,
    trailing: impl FnOnce(&mut Ui),
) -> Response {
    let p = pal(ui);
    let width = ui.available_width();
    // Reserve a slot under the content for the hover wash, so it is painted
    // behind the text rather than over it.
    let wash = ui.painter().add(egui::Shape::Noop);
    let inner = ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_height(48.0);
            ui.add_space(4.0);
            if let Some(g) = glyph {
                ui.add_sized(
                    [24.0, 24.0],
                    egui::Label::new(RichText::new(g).size(17.0).color(p.muted)).selectable(false),
                );
                ui.add_space(6.0);
            }
            ui.vertical(|ui| {
                ui.add_space(7.0);
                ui.add(
                    egui::Label::new(RichText::new(title).color(p.ink))
                        .truncate()
                        .selectable(false),
                );
                if let Some(sub) = subtitle {
                    ui.add(
                        egui::Label::new(RichText::new(sub).small().color(p.muted))
                            .truncate()
                            .selectable(false),
                    );
                }
                ui.add_space(7.0);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), trailing);
        },
    );
    let rect = inner.response.rect;
    let sense = if clickable { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, ui.id().with(("row", title)), sense);
    if clickable && resp.hovered() {
        ui.painter()
            .set(wash, egui::epaint::RectShape::filled(rect, 4, p.hover()));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Text tabs over a hairline, the selected one in accent with an underline.
pub fn text_tabs(ui: &mut Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let p = pal(ui);
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;
        for (i, label) in labels.iter().enumerate() {
            let on = i == selected;
            let text = RichText::new(*label)
                .family(theme::medium())
                .size(14.5)
                .color(if on { p.accent } else { p.muted });
            let resp = ui.add(egui::Label::new(text).sense(Sense::click()).selectable(false));
            if on {
                let r = resp.rect;
                ui.painter().hline(
                    r.x_range(),
                    r.bottom() + 7.0,
                    Stroke::new(2.0, p.accent),
                );
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                clicked = Some(i);
            }
        }
    });
    ui.add_space(6.0);
    hairline(ui);
    clicked
}

/// A pill switch in the accent colour.
pub fn switch(ui: &mut Ui, on: &mut bool, enabled: bool) -> Response {
    let p = pal(ui);
    let size = egui::vec2(38.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let track = if !enabled {
        p.hover()
    } else if *on {
        p.accent
    } else {
        p.background
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        11,
        track,
        Stroke::new(1.0, if *on && enabled { p.accent } else { p.hairline }),
        egui::StrokeKind::Inside,
    );
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    let knob = if *on && enabled { p.on_accent } else { p.muted };
    painter.circle_filled(egui::pos2(x, rect.center().y), 7.0, knob);
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// A choice among a few options, as text chips; returns the new index.
pub fn choice_chips(ui: &mut Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let p = pal(ui);
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, label) in labels.iter().enumerate() {
            let on = i == selected;
            let resp = ui.add(
                egui::Button::new(
                    RichText::new(*label).color(if on { p.on_accent } else { p.ink }),
                )
                .fill(if on { p.accent } else { Color32::TRANSPARENT })
                .stroke(Stroke::new(1.0, if on { p.accent } else { p.hairline }))
                .corner_radius(14)
                .min_size(egui::vec2(64.0, 28.0)),
            );
            if resp.clicked() && !on {
                out = Some(i);
            }
        }
    });
    out
}

/// Quiet empty state: a line of serif and a line of explanation.
pub fn empty_state(ui: &mut Ui, title: &str, body: &str, action: Option<(&str, &mut bool)>) {
    let p = pal(ui);
    ui.add_space(56.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).text_style(theme::headline()).color(p.ink));
        ui.add_space(6.0);
        ui.label(RichText::new(body).color(p.muted));
        if let Some((label, pressed)) = action {
            ui.add_space(16.0);
            if outlined_button(ui, label, true).clicked() {
                *pressed = true;
            }
        }
    });
    ui.add_space(24.0);
}

/// A glyph button with a tooltip; `on` tints it with the accent.
pub fn icon_button(ui: &mut Ui, glyph: &str, tip: &str, on: bool) -> Response {
    let p = pal(ui);
    ui.add(
        egui::Button::new(RichText::new(glyph).size(16.0).color(if on { p.accent } else { p.ink }))
            .fill(if on { p.accent_wash() } else { Color32::TRANSPARENT })
            .min_size(Vec2::splat(30.0)),
    )
    .on_hover_text(tip)
}
