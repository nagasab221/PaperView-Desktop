//! How dense the app is: an interface size (everything scales) and how the
//! Library lays documents out. While the size slider is dragged — or a layout
//! is hovered — a small floating card shows a *mock* of the Library at that
//! setting; the real app only changes when the slider is let go.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};

use super::ViewerApp;
use crate::ui::theme::{self, Palette};
use crate::ui::widgets as w;

pub(super) const SCALE_MIN: f32 = 0.8;
pub(super) const SCALE_MAX: f32 = 1.3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LibraryLayout {
    Covers,
    SmallCovers,
    List,
}

impl LibraryLayout {
    pub const ALL: [LibraryLayout; 3] = [
        LibraryLayout::Covers,
        LibraryLayout::SmallCovers,
        LibraryLayout::List,
    ];

    pub fn key(self) -> &'static str {
        match self {
            LibraryLayout::Covers => "covers",
            LibraryLayout::SmallCovers => "small",
            LibraryLayout::List => "list",
        }
    }

    pub fn from_key(key: &str) -> LibraryLayout {
        match key {
            "small" => LibraryLayout::SmallCovers,
            "list" => LibraryLayout::List,
            _ => LibraryLayout::Covers,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LibraryLayout::Covers => "Large covers",
            LibraryLayout::SmallCovers => "Small covers",
            LibraryLayout::List => "List",
        }
    }
}

/// What the floating preview is showing (kept while it fades out).
#[derive(Clone, Copy)]
pub(super) struct Preview {
    scale: f32,
    layout: LibraryLayout,
    anchor: Pos2,
}

impl ViewerApp {
    /// The "Size" slider and the Library layout choice, for Settings.
    pub(super) fn density_settings(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);

        ui.add_space(16.0);
        w::field_label(ui, "Size");
        let mut showing: Option<Preview> = None;
        let mut sliding = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("A").size(11.0).color(p.muted));
            ui.spacing_mut().slider_width = 280.0;
            let slider = ui.add(
                egui::Slider::new(&mut self.ui_scale_draft, SCALE_MIN..=SCALE_MAX)
                    .step_by(0.05)
                    .show_value(false)
                    .trailing_fill(true)
                    .handle_shape(egui::style::HandleShape::Circle),
            );
            ui.label(RichText::new("A").size(18.0).color(p.muted));
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("{:.0}%", self.ui_scale_draft * 100.0))
                    .family(theme::medium())
                    .color(p.ink),
            );
            if (self.ui_scale_draft - 1.0).abs() > 0.001
                && w::inline_button(ui, "Reset", true).clicked()
            {
                self.ui_scale_draft = 1.0;
            }
            if slider.dragged() {
                sliding = true;
                showing = Some(Preview {
                    scale: self.ui_scale_draft,
                    layout: self.library_layout,
                    // Clear of the percentage and Reset beside the slider.
                    anchor: slider.rect.right_center() + Vec2::new(150.0, 0.0),
                });
            }
        });
        ui.label(
            RichText::new("Smaller fits more on screen; larger is easier to read.")
                .small()
                .color(p.muted),
        );

        ui.add_space(16.0);
        w::field_label(ui, "Library layout");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for layout in LibraryLayout::ALL {
                let on = layout == self.library_layout;
                let kind = if on {
                    w::Kind::Primary
                } else {
                    w::Kind::Outlined
                };
                let resp = w::button(
                    ui,
                    layout.label(),
                    w::Btn::new(kind).radius(14).min(96.0, 28.0),
                );
                if resp.clicked() {
                    self.library_layout = layout;
                }
                if resp.hovered() && showing.is_none() {
                    showing = Some(Preview {
                        scale: self.ui_scale_draft,
                        layout,
                        anchor: resp.rect.right_center(),
                    });
                }
            }
        });

        // Let go of the slider (or moved it with the keyboard): apply for real.
        if !sliding && (self.ui_scale_draft - self.ui_scale).abs() > 0.001 {
            self.ui_scale = self.ui_scale_draft;
        }

        if let Some(s) = showing {
            self.preview = Some(s);
        }
        self.preview_card(ui.ctx(), showing.is_some(), &p);
    }

    /// The floating mock-up, fading in while something is being previewed.
    fn preview_card(&mut self, ctx: &egui::Context, visible: bool, p: &Palette) {
        let t = ctx.animate_bool_with_time(egui::Id::new("density-preview"), visible, 0.15);
        let Some(pv) = self.preview else { return };
        if t <= 0.0 {
            self.preview = None;
            return;
        }
        // Drawn in the *current* scale, showing the candidate one by ratio.
        let ratio = pv.scale / self.ui_scale.max(0.1);
        let screen = ctx.content_rect();
        let card = Vec2::new(300.0, 214.0);
        let mut pos = pv.anchor + Vec2::new(28.0, -card.y * 0.5);
        pos.x = pos.x.min(screen.right() - card.x - 12.0);
        pos.y = pos
            .y
            .clamp(screen.top() + 12.0, screen.bottom() - card.y - 12.0);

        egui::Area::new(egui::Id::new("density-preview-card"))
            .fixed_pos(pos)
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(t);
                egui::Frame::NONE
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.muted.gamma_multiply(0.5)))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("PREVIEW")
                                    .text_style(theme::label())
                                    .extra_letter_spacing(1.0)
                                    .color(p.muted),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{:.0}% · {}",
                                    pv.scale * 100.0,
                                    pv.layout.label()
                                ))
                                .small()
                                .color(p.muted),
                            );
                        });
                        ui.add_space(4.0);
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(280.0, 172.0), egui::Sense::hover());
                        paint_mock(&ui.painter_at(rect), rect, ratio, pv.layout, p);
                    });
            });
        if t < 1.0 {
            ctx.request_repaint();
        }
    }
}

/// A miniature Library — title bar, heading, tabs and shelf — drawn with
/// shapes at `ratio` times today's size. Not the real thing: a sketch that
/// shows how much fits and how big the type is.
fn paint_mock(painter: &egui::Painter, rect: Rect, ratio: f32, layout: LibraryLayout, p: &Palette) {
    // The mock is a 0.5-scale sketch of the real window.
    let k = 0.5 * ratio;
    painter.rect_filled(rect, 4, p.background);
    let o = rect.min;
    let at = |x: f32, y: f32| o + Vec2::new(x * k, y * k);

    // Title bar.
    let title_h = 34.0 * k;
    painter.hline(rect.x_range(), o.y + title_h, Stroke::new(1.0, p.hairline));
    painter.text(
        Pos2::new(rect.center().x, o.y + title_h * 0.5),
        Align2::CENTER_CENTER,
        "PaperView",
        FontId::new(13.5 * k, theme::medium()),
        p.ink,
    );
    for i in 0..3 {
        let c = Pos2::new(
            rect.right() - (22.0 + 44.0 * i as f32) * k,
            o.y + title_h * 0.5,
        );
        painter.circle_filled(c, 2.2 * k.max(0.5), p.muted);
    }

    // Heading, subtitle and the primary button.
    painter.text(
        at(24.0, 56.0),
        Align2::LEFT_TOP,
        "Library",
        FontId::new(30.0 * k, theme::serif()),
        p.ink,
    );
    painter.text(
        at(24.0, 96.0),
        Align2::LEFT_TOP,
        "12 documents",
        FontId::new(12.0 * k, egui::FontFamily::Proportional),
        p.muted,
    );
    let btn = Rect::from_min_size(
        Pos2::new(rect.right() - 130.0 * k, o.y + 60.0 * k),
        Vec2::new(100.0 * k, 32.0 * k),
    );
    if btn.left() > o.x + 150.0 * k {
        painter.rect_filled(btn, 4.0 * k, p.accent);
    }

    // Tabs.
    let tabs_y = 128.0;
    painter.text(
        at(24.0, tabs_y),
        Align2::LEFT_TOP,
        "Recents",
        FontId::new(14.5 * k, theme::medium()),
        p.accent,
    );
    painter.text(
        at(100.0, tabs_y),
        Align2::LEFT_TOP,
        "Folders",
        FontId::new(14.5 * k, theme::medium()),
        p.muted,
    );
    let rule_y = o.y + (tabs_y + 26.0) * k;
    painter.hline(
        o.x + 24.0 * k..=o.x + 80.0 * k,
        rule_y - 1.0,
        Stroke::new(2.0 * k.max(0.5), p.accent),
    );
    painter.hline(
        o.x + 24.0 * k..=rect.right() - 24.0 * k,
        rule_y,
        Stroke::new(1.0, p.hairline),
    );

    // The shelf.
    let top = tabs_y + 48.0;
    let left = 24.0;
    let usable = rect.width() / k - 48.0;
    let line = |x: f32, y: f32, w: f32, c: Color32| {
        painter.hline(
            o.x + x * k..=o.x + (x + w) * k,
            o.y + y * k,
            Stroke::new((3.0 * k).max(1.0), c),
        );
    };
    match layout {
        LibraryLayout::List => {
            let row_h = 56.0;
            for i in 0..8 {
                let y = top + i as f32 * row_h;
                if o.y + y * k > rect.bottom() {
                    break;
                }
                let thumb = Rect::from_min_size(at(left, y + 6.0), Vec2::new(34.0 * k, 44.0 * k));
                painter.rect_filled(thumb, 1, Color32::WHITE);
                painter.rect_stroke(
                    thumb,
                    1,
                    Stroke::new(1.0, p.hairline),
                    egui::StrokeKind::Outside,
                );
                painter.text(
                    at(left + 48.0, y + 12.0),
                    Align2::LEFT_TOP,
                    [
                        "Anatomy notes",
                        "Pharmacology",
                        "Answer key",
                        "Biochem week 2",
                    ][i % 4],
                    FontId::new(14.5 * k, egui::FontFamily::Proportional),
                    p.ink,
                );
                line(left + 48.0, y + 38.0, 90.0, p.hairline);
                painter.hline(
                    o.x + left * k..=rect.right() - 24.0 * k,
                    o.y + (y + row_h - 2.0) * k,
                    Stroke::new(1.0, p.hairline),
                );
            }
        }
        _ => {
            let (tile_w, gap) = if layout == LibraryLayout::SmallCovers {
                (110.0, 18.0)
            } else {
                (150.0, 24.0)
            };
            let cols = (((usable + gap) / (tile_w + gap)).floor() as usize).max(1);
            let tile_h = tile_w / 0.77;
            for row in 0..3 {
                for col in 0..cols {
                    let x = left + col as f32 * (tile_w + gap);
                    let y = top + row as f32 * (tile_h + 60.0);
                    if o.y + y * k > rect.bottom() {
                        continue;
                    }
                    let cover = Rect::from_min_size(at(x, y), Vec2::new(tile_w * k, tile_h * k));
                    painter.rect_filled(cover, 0, Color32::WHITE);
                    painter.rect_stroke(
                        cover,
                        0,
                        Stroke::new(1.0, p.hairline),
                        egui::StrokeKind::Outside,
                    );
                    // A suggestion of a page: a heading bar and lines of text.
                    line(
                        x + 14.0,
                        y + 22.0,
                        tile_w * 0.55,
                        p.muted.gamma_multiply(0.7),
                    );
                    for l in 0..5 {
                        line(
                            x + 14.0,
                            y + 42.0 + l as f32 * 12.0,
                            tile_w - 28.0,
                            p.hairline,
                        );
                    }
                    painter.text(
                        at(x, y + tile_h + 8.0),
                        Align2::LEFT_TOP,
                        "Notes",
                        FontId::new(14.0 * k, theme::medium()),
                        p.ink,
                    );
                }
            }
        }
    }
}
