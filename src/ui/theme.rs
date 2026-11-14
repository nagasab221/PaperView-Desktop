//! The PaperView look, shared with the Android app: paper-toned surfaces, a
//! serif for anything that announces itself, a sans body, hairline rules
//! instead of cards, and no drop shadows. Colour is kept for things that mean
//! something — the active tab, a primary action, the current search hit.
//!
//! Skins, each with a light and a dark rendition. Paper, Slate and Ink are
//! the Android app's `Theme.kt`, one for one. The colour skins (one per main
//! hue) are desktop-only: each is made from a single OKLCH hue at fixed
//! lightness steps, so they share Paper's quiet, tinted-paper character, and
//! every text colour clears 4.5:1 contrast against what it sits on.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke,
    TextStyle,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Skin {
    Paper,
    Slate,
    Ink,
    Cherry,
    Apricot,
    Saffron,
    Forest,
    Lagoon,
    Cobalt,
    Lavender,
    Blossom,
}

impl Skin {
    pub const ALL: [Skin; 11] = [
        Skin::Paper,
        Skin::Slate,
        Skin::Ink,
        Skin::Cherry,
        Skin::Apricot,
        Skin::Saffron,
        Skin::Forest,
        Skin::Lagoon,
        Skin::Cobalt,
        Skin::Lavender,
        Skin::Blossom,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Skin::Paper => "paper",
            Skin::Slate => "slate",
            Skin::Ink => "ink",
            Skin::Cherry => "cherry",
            Skin::Apricot => "apricot",
            Skin::Saffron => "saffron",
            Skin::Forest => "forest",
            Skin::Lagoon => "lagoon",
            Skin::Cobalt => "cobalt",
            Skin::Lavender => "lavender",
            Skin::Blossom => "blossom",
        }
    }

    pub fn from_key(key: &str) -> Skin {
        Skin::ALL
            .into_iter()
            .find(|s| s.key() == key)
            .unwrap_or(Skin::Paper)
    }

    pub fn label(self) -> &'static str {
        match self {
            Skin::Paper => "Paper",
            Skin::Slate => "Slate",
            Skin::Ink => "Ink",
            Skin::Cherry => "Cherry",
            Skin::Apricot => "Apricot",
            Skin::Saffron => "Saffron",
            Skin::Forest => "Forest",
            Skin::Lagoon => "Lagoon",
            Skin::Cobalt => "Cobalt",
            Skin::Lavender => "Lavender",
            Skin::Blossom => "Blossom",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Skin::Paper => "Warm off-white with a sepia accent",
            Skin::Slate => "Cool grey with a steel-blue accent",
            Skin::Ink => "High contrast, no colour at all",
            Skin::Cherry => "Rose-tinted paper with a cherry-red accent",
            Skin::Apricot => "Peach-tinted paper with an orange accent",
            Skin::Saffron => "Cream paper with a golden-yellow accent",
            Skin::Forest => "Pale sage with a deep green accent",
            Skin::Lagoon => "Cool mist with a teal accent",
            Skin::Cobalt => "Crisp blue-white with a cobalt accent",
            Skin::Lavender => "Lilac-tinted paper with a purple accent",
            Skin::Blossom => "Pink-tinted paper with a raspberry accent",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn from_key(key: &str) -> ThemeChoice {
        match key {
            "light" => ThemeChoice::Light,
            "dark" => ThemeChoice::Dark,
            _ => ThemeChoice::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }

    /// Whether this choice comes out dark right now.
    pub fn is_dark(self, ctx: &egui::Context) -> bool {
        match self {
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
            ThemeChoice::System => ctx.system_theme() == Some(egui::Theme::Dark),
        }
    }
}

/// One skin in one brightness.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Palette {
    pub dark: bool,
    /// The page behind everything.
    pub background: Color32,
    /// Raised things: menus, dialogs, text fields.
    pub surface: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub hairline: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    /// Behind the PDF pages, so white paper reads as paper.
    pub viewport: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub fn palette(skin: Skin, dark: bool) -> Palette {
    match (skin, dark) {
        (Skin::Paper, false) => Palette {
            dark,
            background: hex(0xFAF8F4),
            surface: hex(0xFFFDF9),
            ink: hex(0x1B1917),
            muted: hex(0x6B6459),
            hairline: hex(0xE3DDD2),
            accent: hex(0x8A5A2B),
            on_accent: hex(0xFFFDF9),
            viewport: hex(0xCFC9BE),
        },
        (Skin::Paper, true) => Palette {
            dark,
            background: hex(0x16140F),
            surface: hex(0x1D1A14),
            ink: hex(0xEFEAE0),
            muted: hex(0x9A9285),
            hairline: hex(0x2E2A22),
            accent: hex(0xC9915A),
            on_accent: hex(0x1B1610),
            viewport: hex(0x0F0D0A),
        },
        (Skin::Slate, false) => Palette {
            dark,
            background: hex(0xF4F6F8),
            surface: hex(0xFFFFFF),
            ink: hex(0x171A1C),
            muted: hex(0x5C666E),
            hairline: hex(0xDCE2E8),
            accent: hex(0x35678F),
            on_accent: hex(0xFFFFFF),
            viewport: hex(0xC4CCD3),
        },
        (Skin::Slate, true) => Palette {
            dark,
            background: hex(0x101416),
            surface: hex(0x171C20),
            ink: hex(0xE4E9ED),
            muted: hex(0x8D979F),
            hairline: hex(0x262D33),
            accent: hex(0x7FA9CC),
            on_accent: hex(0x0B1116),
            viewport: hex(0x0A0D0F),
        },
        (Skin::Ink, false) => Palette {
            dark,
            background: hex(0xFBFBF8),
            surface: hex(0xFFFFFF),
            ink: hex(0x0B0B0A),
            muted: hex(0x55554F),
            hairline: hex(0xDEDED6),
            accent: hex(0x0B0B0A),
            on_accent: hex(0xFBFBF8),
            viewport: hex(0xC8C8C2),
        },
        (Skin::Ink, true) => Palette {
            dark,
            background: hex(0x0A0A09),
            surface: hex(0x121211),
            ink: hex(0xF2F1EA),
            muted: hex(0x8B8B84),
            hairline: hex(0x1F1F1D),
            accent: hex(0xF2F1EA),
            on_accent: hex(0x0A0A09),
            viewport: hex(0x000000),
        },
        (Skin::Cherry, false) => Palette {
            dark,
            background: hex(0xFFF7F6),
            surface: hex(0xFFFDFC),
            ink: hex(0x1E1312),
            muted: hex(0x745D5B),
            hairline: hex(0xEBDBDA),
            accent: hex(0xAC3031),
            on_accent: hex(0xFFFDFC),
            viewport: hex(0xD7C1BF),
        },
        (Skin::Cherry, true) => Palette {
            dark,
            background: hex(0x130C0C),
            surface: hex(0x1C1313),
            ink: hex(0xF1E7E5),
            muted: hex(0xA99694),
            hairline: hex(0x2E2422),
            accent: hex(0xF28881),
            on_accent: hex(0x170C0B),
            viewport: hex(0x070404),
        },
        (Skin::Apricot, false) => Palette {
            dark,
            background: hex(0xFFF7F3),
            surface: hex(0xFFFDFB),
            ink: hex(0x1D140F),
            muted: hex(0x725F54),
            hairline: hex(0xEADDD5),
            accent: hex(0xA64B00),
            on_accent: hex(0xFFFDFB),
            viewport: hex(0xD6C3B9),
        },
        (Skin::Apricot, true) => Palette {
            dark,
            background: hex(0x130D0A),
            surface: hex(0x1C1410),
            ink: hex(0xF1E7E3),
            muted: hex(0xA7988F),
            hairline: hex(0x2D2420),
            accent: hex(0xF29D6C),
            on_accent: hex(0x170D08),
            viewport: hex(0x070403),
        },
        (Skin::Saffron, false) => Palette {
            dark,
            background: hex(0xFCF9F2),
            surface: hex(0xFFFEFA),
            ink: hex(0x19160D),
            muted: hex(0x6A6350),
            hairline: hex(0xE5DFD3),
            accent: hex(0x896600),
            on_accent: hex(0xFFFEFA),
            viewport: hex(0xCEC7B5),
        },
        (Skin::Saffron, true) => Palette {
            dark,
            background: hex(0x100E08),
            surface: hex(0x19160E),
            ink: hex(0xEDE9E1),
            muted: hex(0xA19B8C),
            hairline: hex(0x2A261E),
            accent: hex(0xEBC669),
            on_accent: hex(0x130F06),
            viewport: hex(0x060503),
        },
        (Skin::Forest, false) => Palette {
            dark,
            background: hex(0xF5FBF6),
            surface: hex(0xFBFFFC),
            ink: hex(0x101812),
            muted: hex(0x57685A),
            hairline: hex(0xD8E3D9),
            accent: hex(0x197037),
            on_accent: hex(0xFBFFFC),
            viewport: hex(0xBCCCBF),
        },
        (Skin::Forest, true) => Palette {
            dark,
            background: hex(0x0B100C),
            surface: hex(0x111812),
            ink: hex(0xE4ECE5),
            muted: hex(0x929F94),
            hairline: hex(0x212922),
            accent: hex(0x79CA8B),
            on_accent: hex(0x09120B),
            viewport: hex(0x040604),
        },
        (Skin::Lagoon, false) => Palette {
            dark,
            background: hex(0xF2FBFB),
            surface: hex(0xFAFFFF),
            ink: hex(0x0C1919),
            muted: hex(0x4F6968),
            hairline: hex(0xD3E4E3),
            accent: hex(0x007172),
            on_accent: hex(0xFAFFFF),
            viewport: hex(0xB6CDCC),
        },
        (Skin::Lagoon, true) => Palette {
            dark,
            background: hex(0x081010),
            surface: hex(0x0E1818),
            ink: hex(0xE1ECEC),
            muted: hex(0x8CA09F),
            hairline: hex(0x1D2929),
            accent: hex(0x5BC8C8),
            on_accent: hex(0x051212),
            viewport: hex(0x030606),
        },
        (Skin::Cobalt, false) => Palette {
            dark,
            background: hex(0xF5F9FF),
            surface: hex(0xFCFEFF),
            ink: hex(0x12161E),
            muted: hex(0x5A6475),
            hairline: hex(0xD9E0EC),
            accent: hex(0x1F53B8),
            on_accent: hex(0xFCFEFF),
            viewport: hex(0xBFC8D8),
        },
        (Skin::Cobalt, true) => Palette {
            dark,
            background: hex(0x0B0E14),
            surface: hex(0x12161D),
            ink: hex(0xE5EAF2),
            muted: hex(0x949CA9),
            hairline: hex(0x22272E),
            accent: hex(0x84AEFA),
            on_accent: hex(0x0B0F18),
            viewport: hex(0x040507),
        },
        (Skin::Lavender, false) => Palette {
            dark,
            background: hex(0xFAF8FF),
            surface: hex(0xFEFDFF),
            ink: hex(0x17141D),
            muted: hex(0x666072),
            hairline: hex(0xE1DDEA),
            accent: hex(0x6C44A4),
            on_accent: hex(0xFEFDFF),
            viewport: hex(0xC9C4D6),
        },
        (Skin::Lavender, true) => Palette {
            dark,
            background: hex(0x0F0D13),
            surface: hex(0x17151C),
            ink: hex(0xEAE8F1),
            muted: hex(0x9D99A7),
            hairline: hex(0x27252D),
            accent: hex(0xBD9FF2),
            on_accent: hex(0x110D17),
            viewport: hex(0x050507),
        },
        (Skin::Blossom, false) => Palette {
            dark,
            background: hex(0xFFF6FA),
            surface: hex(0xFFFCFE),
            ink: hex(0x1C1317),
            muted: hex(0x715C66),
            hairline: hex(0xEADBE1),
            accent: hex(0xA53C75),
            on_accent: hex(0xFFFCFE),
            viewport: hex(0xD5C1C9),
        },
        (Skin::Blossom, true) => Palette {
            dark,
            background: hex(0x120C0F),
            surface: hex(0x1B1317),
            ink: hex(0xF0E6EA),
            muted: hex(0xA7969D),
            hairline: hex(0x2D2327),
            accent: hex(0xF197C2),
            on_accent: hex(0x160C11),
            viewport: hex(0x070405),
        },
    }
}

impl Palette {
    /// `a` blended toward `b` by `t` (0..1).
    pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
        let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
    }

    /// The tray the document tabs sit in: a shade deeper than the page, so
    /// the active tab — a card in the page's own colour — stands forward.
    pub fn tray(&self) -> Color32 {
        Self::mix(
            self.background,
            self.ink,
            if self.dark { 0.045 } else { 0.035 },
        )
    }

    /// The faint wash behind a hovered row or button.
    pub fn hover(&self) -> Color32 {
        Self::mix(
            self.background,
            self.ink,
            if self.dark { 0.08 } else { 0.05 },
        )
    }

    pub fn pressed(&self) -> Color32 {
        Self::mix(
            self.background,
            self.ink,
            if self.dark { 0.14 } else { 0.10 },
        )
    }

    /// The accent as a soft tint, for selected-but-not-primary states.
    pub fn accent_wash(&self) -> Color32 {
        Self::mix(
            self.background,
            self.accent,
            if self.dark { 0.22 } else { 0.14 },
        )
    }
}

// ── Typography ───────────────────────────────────────────────────────────────

/// Serif face for mastheads and titles (Noto Serif — Android's own serif).
pub fn serif() -> FontFamily {
    FontFamily::Name("serif".into())
}

/// The sans at medium weight, for labels and emphasis.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn display() -> TextStyle {
    TextStyle::Name("display".into())
}
pub fn headline() -> TextStyle {
    TextStyle::Name("headline".into())
}
pub fn title() -> TextStyle {
    TextStyle::Name("title".into())
}
/// Small uppercase letterspaced section labels.
pub fn label() -> TextStyle {
    TextStyle::Name("label".into())
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "noto-sans".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/NotoSans-Regular.ttf")).into(),
    );
    fonts.font_data.insert(
        "noto-sans-medium".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/NotoSans-Medium.ttf")).into(),
    );
    fonts.font_data.insert(
        "noto-serif".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/NotoSerif-Regular.ttf")).into(),
    );

    // egui's bundled fonts stay behind ours as fallbacks: they carry the
    // emoji and symbol glyphs the icons use.
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let with = |first: &str| {
        let mut list = vec![first.to_string()];
        list.extend(fallbacks.iter().cloned());
        list
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with("noto-sans"));
    fonts.families.insert(medium(), with("noto-sans-medium"));
    let mut serif_list = vec!["noto-serif".to_string(), "noto-sans".to_string()];
    serif_list.extend(fallbacks.iter().cloned());
    fonts.families.insert(serif(), serif_list);
    // Book text needs real bold and italic faces, not synthesised ones.
    for (family, file) in [
        (
            "serif-bold",
            &include_bytes!("../../assets/fonts/NotoSerif-Bold.ttf")[..],
        ),
        (
            "serif-italic",
            &include_bytes!("../../assets/fonts/NotoSerif-Italic.ttf")[..],
        ),
        (
            "serif-bolditalic",
            &include_bytes!("../../assets/fonts/NotoSerif-BoldItalic.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(family.into(), FontData::from_static(file).into());
        let mut list = vec![
            family.to_string(),
            "noto-serif".to_string(),
            "noto-sans".to_string(),
        ];
        list.extend(fallbacks.iter().cloned());
        fonts.families.insert(FontFamily::Name(family.into()), list);
    }
    super::reading::register(&mut fonts, &fallbacks);
    ctx.set_fonts(fonts);
}

/// Applies the palette to every egui style: colours, type scale, spacing.
pub fn apply(ctx: &egui::Context, p: &Palette) {
    let theme = if p.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    // Pin egui's own light/dark to ours so System never flips one without
    // the other.
    ctx.set_theme(theme);
    // Type and spacing go on both of egui's styles: anything that asks for a
    // named text style must find it whichever one is current.
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (display(), FontId::new(30.0, serif())),
            (headline(), FontId::new(22.0, serif())),
            (title(), FontId::new(17.0, serif())),
            (TextStyle::Heading, FontId::new(19.0, serif())),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (label(), FontId::new(11.5, medium())),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 6.0);
        s.button_padding = egui::vec2(10.0, 5.0);
        s.menu_margin = egui::Margin::symmetric(6, 6);
        s.window_margin = egui::Margin::same(18);
        s.interact_size.y = 28.0;
    });
    ctx.style_mut_of(theme, |style| {
        let v = &mut style.visuals;
        v.dark_mode = p.dark;
        v.override_text_color = None;
        v.panel_fill = p.background;
        v.window_fill = p.surface;
        v.extreme_bg_color = p.surface;
        v.faint_bg_color = p.hover();
        v.code_bg_color = p.hover();
        v.window_stroke = Stroke::new(1.0, p.hairline);
        // No drop shadows anywhere — a hairline is the frame.
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow::NONE;
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(4);
        v.hyperlink_color = p.accent;
        v.weak_text_color = Some(p.muted);
        v.selection.bg_fill = p.accent_wash();
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.text_cursor.stroke = Stroke::new(1.5, p.accent);
        v.striped = false;
        v.indent_has_left_vline = false;
        v.button_frame = true;
        v.slider_trailing_fill = true;
        v.warn_fg_color = if p.dark { hex(0xE0B060) } else { hex(0x9A6A12) };
        v.error_fg_color = if p.dark { hex(0xE58A7C) } else { hex(0xB3261E) };

        let radius = CornerRadius::same(4);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.background;
        w.noninteractive.weak_bg_fill = p.background;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.hairline);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
        w.noninteractive.corner_radius = radius;

        // Buttons are quiet: no fill until touched, a hairline at most.
        w.inactive.bg_fill = p.surface;
        w.inactive.weak_bg_fill = Color32::TRANSPARENT;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, p.ink);
        w.inactive.corner_radius = radius;
        w.inactive.expansion = 0.0;

        w.hovered.bg_fill = p.hover();
        w.hovered.weak_bg_fill = p.hover();
        w.hovered.bg_stroke = Stroke::NONE;
        w.hovered.fg_stroke = Stroke::new(1.0, p.ink);
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = p.pressed();
        w.active.weak_bg_fill = p.pressed();
        w.active.bg_stroke = Stroke::NONE;
        w.active.fg_stroke = Stroke::new(1.0, p.ink);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;

        w.open.bg_fill = p.hover();
        w.open.weak_bg_fill = p.hover();
        w.open.bg_stroke = Stroke::NONE;
        w.open.fg_stroke = Stroke::new(1.0, p.ink);
        w.open.corner_radius = radius;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_skin_reads() {
        for skin in Skin::ALL {
            assert_eq!(Skin::from_key(skin.key()), skin);
            for dark in [false, true] {
                let p = palette(skin, dark);
                let what = format!("{} {}", skin.label(), if dark { "dark" } else { "light" });
                assert!(contrast(p.ink, p.background) >= 7.0, "{what}: ink");
                assert!(contrast(p.muted, p.background) >= 4.5, "{what}: muted");
                assert!(contrast(p.accent, p.background) >= 4.5, "{what}: accent");
                assert!(contrast(p.on_accent, p.accent) >= 4.5, "{what}: on accent");
            }
        }
    }
}
