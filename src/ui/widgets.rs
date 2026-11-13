//! The small set of components every screen is built from — the desktop
//! counterpart of the Android app's `ui/components/Paper.kt`. Screens should
//! compose these rather than reach for egui's defaults; that is what keeps
//! the app looking like one thing.
//!
//! Interactive pieces animate: a hover fades its wash in and out, a press
//! darkens it, tab underlines slide. Each animation is a few frames of
//! `animate_*`, which only asks for repaints while something is moving — an
//! idle window still costs nothing.

use eframe::egui::{
    self, Align, Color32, CursorIcon, Layout, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2, WidgetText,
};

use super::theme::{self, Palette};

/// Hover fade, seconds.
pub const HOVER_FADE: f32 = 0.14;
/// Press darkening, seconds.
const PRESS_FADE: f32 = 0.06;

/// The palette currently in force, stashed in egui's memory by the app.
pub fn pal(ui: &Ui) -> Palette {
    ui.ctx()
        .data(|d| d.get_temp::<Palette>(egui::Id::new("paper-palette")))
        .unwrap_or_else(|| theme::palette(theme::Skin::Paper, ui.visuals().dark_mode))
}

pub fn set_pal(ctx: &egui::Context, p: Palette) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("paper-palette"), p));
}

/// How far a widget is into its hover (0..1), eased.
pub fn hover_t(ui: &Ui, resp: &Response, enabled: bool) -> f32 {
    ui.ctx().animate_bool_with_time(
        resp.id.with("hover"),
        enabled && (resp.hovered() || resp.has_focus()),
        HOVER_FADE,
    )
}

// ── Buttons ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// No frame until hovered: toolbars, text actions.
    Quiet,
    /// Accent text, no frame: inline actions inside content.
    Inline,
    /// Quiet, but marked as the current choice.
    Selected,
    /// Solid accent: the one main action.
    Primary,
    /// Hairline outline: secondary actions.
    Outlined,
    /// Solid red: an action that can't be undone.
    Danger,
}

/// Everything a button can vary in, with sensible defaults.
#[derive(Clone, Copy)]
pub struct Btn {
    pub kind: Kind,
    pub enabled: bool,
    pub min: Vec2,
    pub radius: u8,
    pub sense: Sense,
}

impl Btn {
    pub fn new(kind: Kind) -> Self {
        Btn {
            kind,
            enabled: true,
            min: Vec2::new(0.0, 30.0),
            radius: 4,
            sense: Sense::click(),
        }
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn min(mut self, w: f32, h: f32) -> Self {
        self.min = Vec2::new(w, h);
        self
    }
    pub fn radius(mut self, r: u8) -> Self {
        self.radius = r;
        self
    }
    pub fn sense(mut self, s: Sense) -> Self {
        self.sense = s;
        self
    }
}

/// The one button every control is drawn with.
pub fn button(ui: &mut Ui, text: impl Into<WidgetText>, b: Btn) -> Response {
    let p = pal(ui);
    let galley = text.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let pad = if b.kind == Kind::Inline { Vec2::new(6.0, 4.0) } else { Vec2::new(11.0, 5.0) };
    let size = (galley.size() + pad * 2.0).max(b.min);
    let sense = if b.enabled { b.sense } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);

    if ui.is_rect_visible(rect) {
        let hover = hover_t(ui, &resp, b.enabled);
        let press = ui.ctx().animate_bool_with_time(
            resp.id.with("press"),
            b.enabled && resp.is_pointer_button_down_on(),
            PRESS_FADE,
        );
        let (fill, stroke, text) = match (b.kind, b.enabled) {
            (_, false) => (Color32::TRANSPARENT, Stroke::NONE, p.muted),
            (Kind::Primary, true) => {
                let lift = Palette::mix(p.accent, p.ink, 0.12);
                (p.accent.lerp_to_gamma(lift, hover * 0.6 + press * 0.4), Stroke::NONE, p.on_accent)
            }
            (Kind::Danger, true) => {
                let red = ui.visuals().error_fg_color;
                let deep = Palette::mix(red, p.ink, 0.15);
                (red.lerp_to_gamma(deep, hover * 0.6 + press * 0.4), Stroke::NONE, p.surface)
            }
            (Kind::Outlined, true) => (
                p.hover().gamma_multiply(hover).lerp_to_gamma(p.pressed(), press),
                Stroke::new(1.0, p.hairline.lerp_to_gamma(p.muted, hover * 0.6)),
                p.ink,
            ),
            (Kind::Selected, true) => (
                p.accent_wash().lerp_to_gamma(p.pressed(), press * 0.5),
                Stroke::NONE,
                p.accent,
            ),
            (Kind::Inline, true) => (
                p.hover().gamma_multiply(hover * 0.8).lerp_to_gamma(p.pressed(), press),
                Stroke::NONE,
                p.accent,
            ),
            (Kind::Quiet, true) => (
                p.hover().gamma_multiply(hover).lerp_to_gamma(p.pressed(), press),
                Stroke::NONE,
                p.ink,
            ),
        };
        let painter = ui.painter();
        painter.rect(rect, b.radius, fill, stroke, StrokeKind::Inside);
        let pos = rect.center() - galley.size() * 0.5;
        painter.galley(pos, galley, text);
    }
    if b.enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// A frameless accent-coloured text action.
pub fn inline_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    button(
        ui,
        RichText::new(text).family(theme::medium()),
        Btn::new(Kind::Inline).enabled(enabled).min(0.0, 26.0),
    )
}

/// A quiet text button for toolbars; `on` marks the current choice.
pub fn tool_button(ui: &mut Ui, text: &str, on: bool) -> Response {
    let rt = RichText::new(text);
    let rt = if on { rt.family(theme::medium()) } else { rt };
    button(ui, rt, Btn::new(if on { Kind::Selected } else { Kind::Quiet }).min(0.0, 30.0))
}

/// The primary action: solid accent.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    button(
        ui,
        RichText::new(text).family(theme::medium()),
        Btn::new(Kind::Primary).enabled(enabled).min(0.0, 32.0),
    )
}

/// A secondary action: hairline outline.
pub fn outlined_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    button(ui, text, Btn::new(Kind::Outlined).enabled(enabled).min(0.0, 32.0))
}

/// A square glyph button with a tooltip; `on` tints it with the accent.
pub fn icon_button(ui: &mut Ui, glyph: &str, tip: &str, on: bool) -> Response {
    button(
        ui,
        RichText::new(glyph).size(16.0),
        Btn::new(if on { Kind::Selected } else { Kind::Quiet }).min(32.0, 32.0),
    )
    .on_hover_text(tip)
}

// ── Structure ────────────────────────────────────────────────────────────────

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

/// A single-line text field on a hairline frame; the frame warms on focus.
pub fn field(ui: &mut Ui, value: &mut String, hint: &str, password: bool, width: f32) -> Response {
    let p = pal(ui);
    let id = ui.next_auto_id();
    let focus = ui.ctx().animate_bool_with_time(
        id.with("focus"),
        ui.memory(|m| m.focused()).is_some_and(|f| f == id.with("edit")),
        HOVER_FADE,
    );
    egui::Frame::NONE
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.hairline.lerp_to_gamma(p.accent, focus)))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .id(id.with("edit"))
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
/// `clickable` row fades a wash in on hover and reports clicks.
///
/// The row's own click area is registered *before* its contents, so buttons
/// inside it (a menu, a checkbox) get their clicks instead of the row.
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
    // A slot under the content for the hover wash, painted behind the text.
    let wash = ui.painter().add(egui::Shape::Noop);
    let builder = egui::UiBuilder::new()
        .id_salt(("row", title))
        .layout(Layout::left_to_right(Align::Center))
        .sense(if clickable { Sense::click() } else { Sense::hover() });
    let resp = ui
        .scope_builder(builder, |ui| {
            ui.set_width(width);
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
        })
        .response;
    if clickable {
        let hovering = resp.hovered() || resp.contains_pointer();
        let t = ui.ctx().animate_bool_with_time(resp.id.with("hover"), hovering, HOVER_FADE);
        if t > 0.0 {
            ui.painter()
                .set(wash, egui::epaint::RectShape::filled(resp.rect, 4, p.hover().gamma_multiply(t)));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }
    resp
}

/// A square checkbox in the accent colour; the tick fades in.
pub fn check(ui: &mut Ui, checked: bool) -> Response {
    let p = pal(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id, checked, 0.12);
    let hover = hover_t(ui, &resp, true);
    let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(18.0));
    let painter = ui.painter();
    if hover > 0.0 {
        painter.circle_filled(rect.center(), 15.0, p.hover().gamma_multiply(hover));
    }
    painter.rect(
        r,
        4,
        Color32::TRANSPARENT.lerp_to_gamma(p.accent, t),
        Stroke::new(1.5, p.muted.lerp_to_gamma(p.accent, t)),
        StrokeKind::Inside,
    );
    if t > 0.0 {
        // The tick, drawn rather than taken from a font.
        let c = p.on_accent.gamma_multiply(t);
        let a = r.left_top() + egui::vec2(4.0, 9.5);
        let b = r.left_top() + egui::vec2(7.5, 13.0);
        let d = r.left_top() + egui::vec2(14.0, 5.5);
        painter.line_segment([a, b], Stroke::new(2.0, c));
        painter.line_segment([b, d], Stroke::new(2.0, c));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// Text tabs over a hairline. The selected one is in the accent, and its
/// underline slides across when the selection changes.
pub fn text_tabs(ui: &mut Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let p = pal(ui);
    let mut clicked = None;
    let mut spans: Vec<egui::Rangef> = Vec::with_capacity(labels.len());
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;
        for (i, label) in labels.iter().enumerate() {
            let on = i == selected;
            let galley = WidgetText::from(RichText::new(*label).family(theme::medium()).size(14.5))
                .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
            let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
            let t = hover_t(ui, &resp, !on);
            let on_t = ui.ctx().animate_bool_with_time(resp.id.with("on"), on, HOVER_FADE);
            let color = p.muted.lerp_to_gamma(p.ink, t * 0.8).lerp_to_gamma(p.accent, on_t);
            ui.painter().galley(rect.min, galley, color);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if resp.clicked() {
                clicked = Some(i);
            }
            spans.push(rect.x_range());
        }
    });
    // One underline, gliding between tabs.
    if let Some(target) = spans.get(selected) {
        let id = row.response.id.with("underline");
        let left = ui.ctx().animate_value_with_time(id.with("l"), target.min, 0.18);
        let right = ui.ctx().animate_value_with_time(id.with("r"), target.max, 0.18);
        let y = row.response.rect.bottom() + 7.0;
        ui.painter().hline(left..=right, y, Stroke::new(2.0, p.accent));
    }
    ui.add_space(6.0);
    hairline(ui);
    clicked
}

/// A pill switch in the accent colour; the knob slides.
pub fn switch(ui: &mut Ui, on: &mut bool, enabled: bool) -> Response {
    let p = pal(ui);
    let size = egui::vec2(38.0, 22.0);
    let (rect, mut resp) =
        ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.16);
    let track = if !enabled {
        p.hover()
    } else {
        p.background.lerp_to_gamma(p.accent, t)
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        11,
        track,
        Stroke::new(1.0, if enabled { p.hairline.lerp_to_gamma(p.accent, t) } else { p.hairline }),
        StrokeKind::Inside,
    );
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    let knob = if enabled { p.muted.lerp_to_gamma(p.on_accent, t) } else { p.muted };
    painter.circle_filled(egui::pos2(x, rect.center().y), 7.0, knob);
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// A choice among a few options, as rounded chips; returns the new index.
pub fn choice_chips(ui: &mut Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, label) in labels.iter().enumerate() {
            let on = i == selected;
            let kind = if on { Kind::Primary } else { Kind::Outlined };
            if button(ui, *label, Btn::new(kind).radius(14).min(64.0, 28.0)).clicked() && !on {
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

/// A dialog in the house style: paper surface, hairline, generous margins.
pub fn modal(id: &str, ctx: &egui::Context) -> egui::Modal {
    let p = ctx
        .data(|d| d.get_temp::<Palette>(egui::Id::new("paper-palette")))
        .unwrap_or_else(|| theme::palette(theme::Skin::Paper, false));
    egui::Modal::new(egui::Id::new(id)).frame(
        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.hairline))
            .corner_radius(8)
            .inner_margin(egui::Margin::same(24)),
    )
}

/// Fades a freshly shown view in: call at the top of the area that changes,
/// with a `key` that changes whenever its content does. Each `slot` tracks
/// its own area, so a page and a tab inside it can fade independently.
/// Starts from a faint version of the new view rather than from nothing, so
/// a switch reads as a soft change instead of a blink.
pub fn fade_in(ui: &mut Ui, slot: &str, key: impl std::hash::Hash + std::fmt::Debug) {
    let id = egui::Id::new(("fade-in", slot, key));
    let current = egui::Id::new(("fade-in-current", slot));
    let now = ui.input(|i| i.time);
    let previous = ui.ctx().data(|d| d.get_temp::<egui::Id>(current));
    let started = match previous {
        Some(prev) if prev == id => ui.ctx().data(|d| d.get_temp::<f64>(id)).unwrap_or(now - 1.0),
        Some(prev) => {
            // Switched: start this view's fade now.
            ui.ctx().data_mut(|d| {
                d.remove::<f64>(prev);
                d.insert_temp(current, id);
                d.insert_temp(id, now);
            });
            now
        }
        None => {
            // First view ever shown: no fade.
            ui.ctx().data_mut(|d| {
                d.insert_temp(current, id);
                d.insert_temp(id, now - 1.0);
            });
            now - 1.0
        }
    };
    let t = (((now - started) as f32) / 0.16).clamp(0.0, 1.0);
    if t < 1.0 {
        let eased = t * (2.0 - t);
        ui.multiply_opacity(0.45 + 0.55 * eased);
        ui.ctx().request_repaint();
    }
}
