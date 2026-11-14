//! Reading options: the typeface, size, line spacing, margins and paragraph
//! style books are set in. The same controls appear in the reader's "Aa"
//! panel (compact) and in Settings (with a live sample page).

use eframe::egui::{self, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::reader::{self, Margins, ReadingStyle, Spacing, FONT_MAX, FONT_MIN};
use super::ViewerApp;
use crate::ebook::{Block, Run, Style as RunStyle};
use crate::ui::reading::BookFont;
use crate::ui::theme::{self, Palette};
use crate::ui::widgets as w;

impl ViewerApp {
    /// All the reading options; `compact` for the reader's popover.
    pub(super) fn reading_controls(&mut self, ui: &mut egui::Ui, compact: bool) {
        let p = w::pal(ui);
        let mut style = self.reading;

        w::field_label(ui, "Typeface");
        ui.add_space(4.0);
        if compact {
            for font in BookFont::ALL {
                if font_row(ui, font, font == style.font, &p).clicked() {
                    style.font = font;
                }
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                for font in BookFont::ALL {
                    if font_card(ui, font, font == style.font, &p).clicked() {
                        style.font = font;
                    }
                }
            });
        }

        ui.add_space(14.0);
        w::field_label(ui, "Text size");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let smaller = w::button(
                ui,
                RichText::new("A−").size(13.0),
                w::Btn::new(w::Kind::Outlined)
                    .enabled(style.size > FONT_MIN)
                    .radius(14)
                    .min(44.0, 28.0),
            );
            if smaller.on_hover_text("Smaller (Ctrl+-)").clicked() {
                style.size = (style.size - 1.0).max(FONT_MIN);
            }
            ui.add_sized(
                [46.0, 28.0],
                egui::Label::new(
                    RichText::new(format!("{:.0}", style.size))
                        .family(theme::medium())
                        .color(p.ink),
                ),
            );
            let larger = w::button(
                ui,
                RichText::new("A+").size(16.0),
                w::Btn::new(w::Kind::Outlined)
                    .enabled(style.size < FONT_MAX)
                    .radius(14)
                    .min(44.0, 28.0),
            );
            if larger.on_hover_text("Larger (Ctrl+=)").clicked() {
                style.size = (style.size + 1.0).min(FONT_MAX);
            }
            if !compact && (style.size - 18.0).abs() > 0.01 {
                ui.add_space(6.0);
                if w::inline_button(ui, "Reset", true).clicked() {
                    style.size = 18.0;
                }
            }
        });

        ui.add_space(12.0);
        w::field_label(ui, "Line spacing");
        let labels = Spacing::ALL.map(|s| s.label());
        let i = Spacing::ALL
            .iter()
            .position(|s| *s == style.spacing)
            .unwrap_or(1);
        if let Some(i) = w::choice_chips(ui, &labels, i) {
            style.spacing = Spacing::ALL[i];
        }

        ui.add_space(12.0);
        w::field_label(ui, "Margins");
        let labels = Margins::ALL.map(|m| m.label());
        let i = Margins::ALL
            .iter()
            .position(|m| *m == style.margins)
            .unwrap_or(1);
        if let Some(i) = w::choice_chips(ui, &labels, i) {
            style.margins = Margins::ALL[i];
        }

        ui.add_space(12.0);
        w::field_label(ui, "Paragraphs");
        if let Some(i) = w::choice_chips(ui, &["Indented", "Spaced"], usize::from(!style.indent)) {
            style.indent = i == 0;
        }

        if style != self.reading {
            self.set_reading(style);
        }
    }

    /// Settings' Reading section: the options beside a sample page.
    pub(super) fn reading_settings(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        ui.add_space(10.0);
        ui.label(
            RichText::new("How books are set. Changes apply to every book, and you can also change them while reading, from Aa in the bottom bar.")
                .color(p.muted),
        );
        ui.add_space(12.0);
        let wide = ui.available_width() > 980.0;
        if wide {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - 400.0);
                    self.reading_controls(ui, false);
                });
                ui.add_space(24.0);
                sample_page(ui, &self.reading, &p, Vec2::new(360.0, 470.0));
            });
        } else {
            self.reading_controls(ui, false);
            ui.add_space(16.0);
            sample_page(ui, &self.reading, &p, Vec2::new(420.0, 340.0));
        }
    }
}

/// A typeface in the popover: its name set in itself, a check when chosen.
fn font_row(ui: &mut egui::Ui, font: BookFont, on: bool, p: &Palette) -> egui::Response {
    let width = ui.available_width().max(260.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 34.0), Sense::click());
    let t = w::hover_t(ui, &resp, true);
    if on {
        ui.painter().rect_filled(rect, 6, p.accent_wash());
    } else if t > 0.0 {
        ui.painter()
            .rect_filled(rect, 6, p.hover().gamma_multiply(t));
    }
    ui.painter().text(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        font.label(),
        egui::FontId::new(16.0 * font.scale(), font.family(false, false)),
        if on { p.accent } else { p.ink },
    );
    if on {
        ui.painter().text(
            Pos2::new(rect.right() - 12.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "✔",
            egui::FontId::proportional(13.0),
            p.accent,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(font.blurb())
}

/// A typeface in Settings: a card with its name and what it's like.
fn font_card(ui: &mut egui::Ui, font: BookFont, on: bool, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(196.0, 70.0), Sense::click());
    let t = w::hover_t(ui, &resp, true);
    let painter = ui.painter();
    let fill = if on {
        p.accent_wash()
    } else {
        Palette::mix(p.surface, p.hover(), t)
    };
    painter.rect_filled(rect, 8, fill);
    painter.rect_stroke(
        rect,
        8,
        Stroke::new(
            if on { 2.0 } else { 1.0 },
            if on { p.accent } else { p.hairline },
        ),
        egui::StrokeKind::Inside,
    );
    // The name in its own face, shrunk to fit the card if it's long.
    let room = rect.width() - 28.0;
    let mut size = 19.0 * font.scale();
    let mut name = painter.layout_no_wrap(
        font.label().into(),
        egui::FontId::new(size, font.family(false, false)),
        p.ink,
    );
    if name.size().x > room {
        size *= room / name.size().x;
        name = painter.layout_no_wrap(
            font.label().into(),
            egui::FontId::new(size, font.family(false, false)),
            p.ink,
        );
    }
    painter.galley(rect.left_top() + Vec2::new(14.0, 12.0), name, p.ink);
    painter.text(
        rect.left_bottom() + Vec2::new(14.0, -12.0),
        egui::Align2::LEFT_BOTTOM,
        font.blurb(),
        egui::FontId::new(10.5, egui::FontFamily::Proportional),
        p.muted,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// A page of a book (public domain), set exactly as the reader would.
fn sample_page(ui: &mut egui::Ui, style: &ReadingStyle, p: &Palette, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect.expand(30.0));
    let shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 22,
        spread: 0,
        color: egui::Color32::from_black_alpha(if p.dark { 110 } else { 30 }),
    };
    painter.add(shadow.as_shape(rect, 4));
    painter.rect(
        rect,
        4,
        p.surface,
        Stroke::new(1.0, p.hairline.gamma_multiply(0.7)),
        egui::StrokeKind::Outside,
    );
    let plain = |t: &str| Run {
        text: t.into(),
        style: RunStyle::default(),
    };
    let blocks = [
        Block::Heading {
            level: 2,
            runs: vec![plain("Chapter I")],
        },
        Block::Para {
            runs: vec![plain(
                "It was the best of times, it was the worst of times, it was the age of \
                 wisdom, it was the age of foolishness, it was the epoch of belief, it was \
                 the epoch of incredulity.",
            )],
        },
        Block::Para {
            runs: vec![
                plain("There were a king with a large jaw and a queen with a plain face, on the throne of England; there were a king with a large jaw and a queen with a fair face, on the throne of France. In both countries it was clearer than crystal to the lords of the State preserves of loaves and fishes, that things in "),
                Run {
                    text: "general".into(),
                    style: RunStyle {
                        italic: true,
                        ..Default::default()
                    },
                },
                plain(" were settled for ever."),
            ],
        },
    ];
    let margin = Vec2::new(34.0 * style.margins.factor(), 30.0);
    let col = rect.shrink2(margin);
    let mut y = col.top();
    for (i, block) in blocks.iter().enumerate() {
        let Some(job) = reader::block_job(style, &blocks, i, col.width(), p.ink) else {
            continue;
        };
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let x = if matches!(block, Block::Heading { .. }) {
            col.center().x
        } else {
            col.left()
        };
        let h = galley.size().y;
        painter
            .with_clip_rect(Rect::from_min_max(col.min, col.max).intersect(rect))
            .galley(Pos2::new(x, y), galley, p.ink);
        y += h + style.gap_after(block);
        if y > col.bottom() {
            break;
        }
    }
    // A soft fade where the sample runs off the page.
    let fade = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 40.0), rect.max);
    let mut mesh = egui::Mesh::default();
    let clear = egui::Color32::TRANSPARENT;
    mesh.colored_vertex(fade.left_top(), clear);
    mesh.colored_vertex(fade.right_top(), clear);
    mesh.colored_vertex(fade.right_bottom(), p.surface);
    mesh.colored_vertex(fade.left_bottom(), p.surface);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}
