//! Ebooks. Every format is read into one simple model — headings,
//! paragraphs of styled runs, images, page breaks and a chapter list — which
//! the reader then lays out into pages. Comics (CBZ) are a list of page images.
//!
//! Supported: EPUB, MOBI / AZW / AZW3 / PRC (without DRM), FB2 and FBZ,
//! CBZ, TXT and single HTML files.

pub mod markup;

mod comic;
mod epub;
mod fb2;
mod mobi;
mod text;

use std::path::Path;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading { level: u8, runs: Vec<Run> },
    Para { runs: Vec<Run> },
    /// Index into [`Content::Text`]'s images.
    Image { id: usize },
    PageBreak,
    Rule,
}

impl Block {
    pub fn text_len(&self) -> usize {
        match self {
            Block::Heading { runs, .. } | Block::Para { runs } => {
                runs.iter().map(|r| r.text.chars().count()).sum()
            }
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    pub title: String,
    /// The block the chapter starts at.
    pub block: usize,
}

pub enum Content {
    /// Reflowable text, laid out into pages by the reader.
    Text { blocks: Vec<Block>, images: Vec<Vec<u8>> },
    /// One image per page (comics). Read on demand from the archive.
    Comic { pages: Vec<String> },
}

pub struct Book {
    pub title: String,
    pub author: Option<String>,
    pub content: Content,
    pub chapters: Vec<Chapter>,
}

impl Book {
    pub fn is_comic(&self) -> bool {
        matches!(self.content, Content::Comic { .. })
    }
}

/// Extensions PaperView reads as books.
pub const EXTENSIONS: &[&str] = &[
    "epub", "mobi", "azw", "azw3", "prc", "fb2", "fbz", "cbz", "txt", "html", "htm", "xhtml",
];

pub fn is_ebook(path: &Path) -> bool {
    let name = path.to_string_lossy().to_ascii_lowercase();
    name.ends_with(".fb2.zip")
        || path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}

fn kind(path: &Path) -> String {
    let name = path.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".fb2.zip") {
        return "fbz".into();
    }
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Reads a whole book. Slow for big ones: call it off the UI thread.
pub fn open(path: &Path) -> Result<Book, String> {
    let mut book = match kind(path).as_str() {
        "epub" => epub::open(path)?,
        "mobi" | "azw" | "azw3" | "prc" => mobi::open(path)?,
        "fb2" => fb2::open_file(path)?,
        "fbz" => fb2::open_zip(path)?,
        "cbz" => comic::open(path)?,
        "html" | "htm" | "xhtml" => text::open_html(path)?,
        _ => text::open_txt(path)?,
    };
    if book.title.trim().is_empty() {
        book.title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
    }
    if book.chapters.is_empty() {
        book.chapters = chapters_from_headings(&book.content);
    }
    Ok(book)
}

/// The cover image's bytes, if the book has one — cheaper than a full read
/// where the format allows it.
pub fn cover(path: &Path) -> Option<Vec<u8>> {
    match kind(path).as_str() {
        "epub" => epub::cover(path),
        "mobi" | "azw" | "azw3" | "prc" => mobi::cover(path),
        "fb2" => fb2::cover_file(path),
        "fbz" => fb2::cover_zip(path),
        "cbz" => comic::cover(path),
        _ => None,
    }
}

/// Bytes of one comic page.
pub fn comic_page(path: &Path, name: &str) -> Option<Vec<u8>> {
    comic::page(path, name)
}

/// When a book has no table of contents, its headings make one.
fn chapters_from_headings(content: &Content) -> Vec<Chapter> {
    let Content::Text { blocks, .. } = content else { return Vec::new() };
    blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            Block::Heading { level, runs } if *level <= 2 => {
                let title: String = runs.iter().map(|r| r.text.as_str()).collect();
                let title = title.trim().to_string();
                (!title.is_empty()).then_some(Chapter { title, block: i })
            }
            _ => None,
        })
        .collect()
}

/// Windows-1252 bytes to text (old Kindle books and stray files).
pub(crate) fn decode_cp1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => HIGH[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

/// Windows-1251 (Cyrillic) bytes to text — common in FB2 books.
pub(crate) fn decode_cp1251(bytes: &[u8]) -> String {
    const HIGH: [u16; 64] = [
        0x0402, 0x0403, 0x201A, 0x0453, 0x201E, 0x2026, 0x2020, 0x2021, 0x20AC, 0x2030, 0x0409,
        0x2039, 0x040A, 0x040C, 0x040B, 0x040F, 0x0452, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022,
        0x2013, 0x2014, 0x0098, 0x2122, 0x0459, 0x203A, 0x045A, 0x045C, 0x045B, 0x045F, 0x00A0,
        0x040E, 0x045E, 0x0408, 0x00A4, 0x0490, 0x00A6, 0x00A7, 0x0401, 0x00A9, 0x0404, 0x00AB,
        0x00AC, 0x00AD, 0x00AE, 0x0407, 0x00B0, 0x00B1, 0x0406, 0x0456, 0x0491, 0x00B5, 0x00B6,
        0x00B7, 0x0451, 0x2116, 0x0454, 0x00BB, 0x0458, 0x0405, 0x0455, 0x0457,
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0xBF => char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap_or('?'),
            0xC0..=0xFF => char::from_u32(0x0410 + (b - 0xC0) as u32).unwrap_or('?'),
            _ => b as char,
        })
        .collect()
}

/// UTF-8 if it is, otherwise Windows-1252.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => decode_cp1252(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_pages() {
        assert_eq!(decode_cp1252(b"\x93caf\xe9\x94"), "“café”");
        assert_eq!(decode_cp1251(b"\xcf\xf0\xe8\xe2\xe5\xf2"), "Привет");
        assert_eq!(decode_text("ok ✓".as_bytes()), "ok ✓");
    }

    #[test]
    fn recognises_book_files() {
        for name in ["a.epub", "B.MOBI", "c.azw3", "d.fb2", "e.fb2.zip", "f.cbz", "g.txt"] {
            assert!(is_ebook(Path::new(name)), "{name}");
        }
        assert!(!is_ebook(Path::new("x.pdf")));
    }
}
