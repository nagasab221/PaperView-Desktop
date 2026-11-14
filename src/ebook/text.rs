//! Plain text and single HTML files.

use std::path::Path;

use super::markup::{tokenize, Builder};
use super::{Block, Book, Content, Run, Style};

/// Paragraphs are separated by blank lines; lines inside one are joined (most
/// text books are hard-wrapped). Short lines that look like "CHAPTER IV" or
/// are all capitals become headings.
pub fn open_txt(path: &Path) -> Result<Book, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't open the book: {e}"))?;
    let text = super::decode_text(&bytes).replace("\r\n", "\n");
    let mut blocks = Vec::new();
    for para in text.split("\n\n") {
        let lines: Vec<&str> = para
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let joined = lines.join(" ");
        let words = joined.split_whitespace().count();
        let has_letters = joined.chars().any(|c| c.is_alphabetic());
        let shouty = has_letters && !joined.chars().any(|c| c.is_lowercase());
        let lower = joined.to_lowercase();
        let chapterish = lower.starts_with("chapter ")
            || lower.starts_with("book ")
            || lower.starts_with("part ");
        let run = vec![Run {
            text: joined,
            style: Style::default(),
        }];
        blocks.push(
            if lines.len() == 1 && words <= 8 && (shouty || chapterish) {
                Block::Heading {
                    level: 2,
                    runs: run,
                }
            } else {
                Block::Para { runs: run }
            },
        );
    }
    if blocks.is_empty() {
        return Err("This text file is empty".into());
    }
    Ok(Book {
        title: String::new(),
        author: None,
        content: Content::Text {
            blocks,
            images: Vec::new(),
        },
        chapters: Vec::new(),
    })
}

pub fn open_html(path: &Path) -> Result<Book, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't open the book: {e}"))?;
    let src = super::decode_text(&bytes);
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut resolve = |r: &str| -> Option<usize> {
        if r.contains("://") {
            return None; // never fetch from the network
        }
        let bytes = std::fs::read(dir.join(r.split('#').next()?)).ok()?;
        images.push(bytes);
        Some(images.len() - 1)
    };
    let mut b = Builder::new(false, &mut resolve);
    let tokens = tokenize(&src);
    // The document's <title>, before the builder skips it.
    let mut title = String::new();
    let mut in_title = false;
    for t in &tokens {
        match t {
            super::markup::Token::Start { name, .. } if name == "title" => in_title = true,
            super::markup::Token::End { name } if name == "title" => break,
            super::markup::Token::Text(s) if in_title => title.push_str(s.trim()),
            _ => {}
        }
    }
    b.feed(&tokens);
    let (blocks, _) = b.finish();
    if blocks.is_empty() {
        return Err("Couldn't find any text in this file".into());
    }
    Ok(Book {
        title,
        author: None,
        content: Content::Text { blocks, images },
        chapters: Vec::new(),
    })
}
