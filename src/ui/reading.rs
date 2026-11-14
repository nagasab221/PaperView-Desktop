//! Typefaces for reading books: popular open-licence book faces, each with
//! real bold and italic cuts (egui can't synthesise them). They're trimmed to
//! Latin, Greek and Cyrillic by `scripts/make_reading_fonts.py`; anything
//! else falls back to Noto.

use eframe::egui::{FontData, FontDefinitions, FontFamily};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum BookFont {
    #[default]
    NotoSerif,
    Literata,
    Merriweather,
    Garamond,
    Lora,
    SourceSerif,
    NotoSans,
    Atkinson,
    OpenDyslexic,
}

impl BookFont {
    pub const ALL: [BookFont; 9] = [
        BookFont::NotoSerif,
        BookFont::Literata,
        BookFont::Merriweather,
        BookFont::Garamond,
        BookFont::Lora,
        BookFont::SourceSerif,
        BookFont::NotoSans,
        BookFont::Atkinson,
        BookFont::OpenDyslexic,
    ];

    pub fn key(self) -> &'static str {
        match self {
            BookFont::NotoSerif => "noto-serif",
            BookFont::Literata => "literata",
            BookFont::Merriweather => "merriweather",
            BookFont::Garamond => "garamond",
            BookFont::Lora => "lora",
            BookFont::SourceSerif => "source-serif",
            BookFont::NotoSans => "noto-sans",
            BookFont::Atkinson => "atkinson",
            BookFont::OpenDyslexic => "opendyslexic",
        }
    }

    pub fn from_key(key: &str) -> BookFont {
        BookFont::ALL
            .into_iter()
            .find(|f| f.key() == key)
            .unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            BookFont::NotoSerif => "Noto Serif",
            BookFont::Literata => "Literata",
            BookFont::Merriweather => "Merriweather",
            BookFont::Garamond => "EB Garamond",
            BookFont::Lora => "Lora",
            BookFont::SourceSerif => "Source Serif",
            BookFont::NotoSans => "Noto Sans",
            BookFont::Atkinson => "Atkinson Hyperlegible",
            BookFont::OpenDyslexic => "OpenDyslexic",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            BookFont::NotoSerif => "PaperView's serif: calm and even",
            BookFont::Literata => "Drawn for long reading on screens",
            BookFont::Merriweather => "Sturdy and open, clear when small",
            BookFont::Garamond => "The classic printed-book face",
            BookFont::Lora => "Warm, with a calligraphic touch",
            BookFont::SourceSerif => "Crisp and modern",
            BookFont::NotoSans => "Plain sans-serif",
            BookFont::Atkinson => "Letters made hard to confuse",
            BookFont::OpenDyslexic => "Weighted letters, for dyslexia",
        }
    }

    pub fn is_sans(self) -> bool {
        matches!(
            self,
            BookFont::NotoSans | BookFont::Atkinson | BookFont::OpenDyslexic
        )
    }

    /// Faces differ in how big they look at one size (EB Garamond is small,
    /// OpenDyslexic large); this evens them out so 18 looks like 18.
    pub fn scale(self) -> f32 {
        match self {
            BookFont::Garamond => 1.13,
            BookFont::Merriweather => 0.94,
            BookFont::OpenDyslexic => 0.88,
            BookFont::Atkinson => 1.02,
            _ => 1.0,
        }
    }

    /// The family for a run of text in this face.
    pub fn family(self, bold: bool, italic: bool) -> FontFamily {
        let style = style_suffix(bold, italic);
        if self == BookFont::NotoSerif {
            // The serif PaperView's own titles use, registered by the theme.
            return FontFamily::Name(if style.is_empty() {
                "serif".into()
            } else {
                format!("serif-{style}").into()
            });
        }
        FontFamily::Name(family_name(self, style).into())
    }

    /// The four cuts: regular, bold, italic, bold italic.
    fn faces(self) -> Option<[&'static [u8]; 4]> {
        macro_rules! cuts {
            ($name:literal, $ext:literal) => {
                [
                    include_bytes!(concat!(
                        "../../assets/fonts/reading/",
                        $name,
                        "-Regular.",
                        $ext
                    )),
                    include_bytes!(concat!(
                        "../../assets/fonts/reading/",
                        $name,
                        "-Bold.",
                        $ext
                    )),
                    include_bytes!(concat!(
                        "../../assets/fonts/reading/",
                        $name,
                        "-Italic.",
                        $ext
                    )),
                    include_bytes!(concat!(
                        "../../assets/fonts/reading/",
                        $name,
                        "-BoldItalic.",
                        $ext
                    )),
                ]
            };
        }
        Some(match self {
            BookFont::NotoSerif => return None,
            BookFont::Literata => cuts!("Literata", "ttf"),
            BookFont::Merriweather => cuts!("Merriweather", "ttf"),
            BookFont::Garamond => cuts!("EBGaramond", "ttf"),
            BookFont::Lora => cuts!("Lora", "ttf"),
            BookFont::SourceSerif => cuts!("SourceSerif4", "ttf"),
            BookFont::NotoSans => [
                include_bytes!("../../assets/fonts/NotoSans-Regular.ttf"),
                include_bytes!("../../assets/fonts/reading/NotoSans-Bold.ttf"),
                include_bytes!("../../assets/fonts/reading/NotoSans-Italic.ttf"),
                include_bytes!("../../assets/fonts/reading/NotoSans-BoldItalic.ttf"),
            ],
            BookFont::Atkinson => cuts!("AtkinsonHyperlegible", "ttf"),
            BookFont::OpenDyslexic => cuts!("OpenDyslexic", "otf"),
        })
    }
}

fn style_suffix(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (false, false) => "",
        (true, false) => "bold",
        (false, true) => "italic",
        (true, true) => "bolditalic",
    }
}

fn family_name(font: BookFont, style: &str) -> String {
    if style.is_empty() {
        format!("book-{}", font.key())
    } else {
        format!("book-{}-{style}", font.key())
    }
}

/// Registers every reading face as its own family. A missing glyph falls back
/// to the same face's regular cut, then Noto, then egui's symbol fonts.
pub fn register(fonts: &mut FontDefinitions, fallbacks: &[String]) {
    for font in BookFont::ALL {
        let Some(faces) = font.faces() else { continue };
        let styles = [(false, false), (true, false), (false, true), (true, true)];
        let regular = family_name(font, "");
        for (bytes, (bold, italic)) in faces.into_iter().zip(styles) {
            let name = family_name(font, style_suffix(bold, italic));
            fonts
                .font_data
                .insert(name.clone(), FontData::from_static(bytes).into());
            let mut list = vec![name.clone()];
            if name != regular {
                list.push(regular.clone());
            }
            let noto = if font.is_sans() {
                ["noto-sans", "noto-serif"]
            } else {
                ["noto-serif", "noto-sans"]
            };
            list.extend(noto.iter().map(|s| s.to_string()));
            list.extend(fallbacks.iter().cloned());
            fonts.families.insert(FontFamily::Name(name.into()), list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        for f in BookFont::ALL {
            assert_eq!(BookFont::from_key(f.key()), f);
        }
        assert_eq!(BookFont::from_key("nonsense"), BookFont::NotoSerif);
    }
}
