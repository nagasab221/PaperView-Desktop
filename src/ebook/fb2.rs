//! FictionBook (FB2), and FB2 zipped (FBZ / .fb2.zip): one XML file with the
//! text in `<body>` and images as base64 `<binary>` elements.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use base64::Engine;

use super::markup::{tokenize, Builder, Token};
use super::{Book, Content};

/// FB2 is often Windows-1251; the XML declaration says which.
fn decode(bytes: &[u8]) -> String {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).to_ascii_lowercase();
    if head.contains("windows-1251") || head.contains("cp1251") {
        super::decode_cp1251(bytes)
    } else if head.contains("windows-1252") || head.contains("iso-8859-1") {
        super::decode_cp1252(bytes)
    } else {
        super::decode_text(bytes)
    }
}

fn first_fb2_in_zip(path: &Path) -> Result<Vec<u8>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("Couldn't open the book: {e}"))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .find(|n| n.to_ascii_lowercase().ends_with(".fb2"))
        .ok_or("There's no .fb2 file inside this archive")?;
    let mut buf = Vec::new();
    zip.by_name(&name).map_err(|e| e.to_string())?.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

pub fn open_file(path: &Path) -> Result<Book, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't open the book: {e}"))?;
    parse(&decode(&bytes))
}

pub fn open_zip(path: &Path) -> Result<Book, String> {
    parse(&decode(&first_fb2_in_zip(path)?))
}

fn binaries(tokens: &[Token]) -> HashMap<String, Vec<u8>> {
    let mut out = HashMap::new();
    let mut current: Option<String> = None;
    for t in tokens {
        match t {
            Token::Start { name, attrs, .. } if name == "binary" => {
                current = attrs.iter().find(|(k, _)| k == "id").map(|(_, v)| v.clone());
            }
            Token::Text(text) if current.is_some() => {
                let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(clean) {
                    out.insert(current.take().unwrap(), bytes);
                }
            }
            Token::End { name } if name == "binary" => current = None,
            _ => {}
        }
    }
    out
}

/// Book title, author and the cover's binary id from `<title-info>`.
fn description(tokens: &[Token]) -> (String, Option<String>, Option<String>) {
    let (mut title, mut first, mut last, mut cover) = (String::new(), String::new(), String::new(), None);
    let mut in_title_info = false;
    let mut field: Option<String> = None;
    let mut author_done = false;
    for t in tokens {
        match t {
            Token::Start { name, attrs, .. } => match name.as_str() {
                "title-info" => in_title_info = true,
                "book-title" | "first-name" | "last-name" if in_title_info => field = Some(name.clone()),
                "image" if in_title_info => {
                    cover = attrs
                        .iter()
                        .find(|(k, _)| k.ends_with("href"))
                        .map(|(_, v)| v.trim_start_matches('#').to_string());
                }
                _ => {}
            },
            Token::End { name } => {
                if name == "title-info" {
                    break;
                }
                if name == "author" && (!first.is_empty() || !last.is_empty()) {
                    author_done = true;
                }
                field = None;
            }
            Token::Text(text) => match field.as_deref() {
                Some("book-title") => title.push_str(text.trim()),
                Some("first-name") if !author_done => first.push_str(text.trim()),
                Some("last-name") if !author_done => last.push_str(text.trim()),
                _ => {}
            },
        }
    }
    let author = format!("{first} {last}").trim().to_string();
    (title, (!author.is_empty()).then_some(author), cover)
}

fn parse(src: &str) -> Result<Book, String> {
    let tokens = tokenize(src);
    let bins = binaries(&tokens);
    let (title, author, _) = description(&tokens);
    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut resolve = |r: &str| -> Option<usize> {
        let key = r.trim_start_matches('#').to_string();
        if let Some(&i) = ids.get(&key) {
            return Some(i);
        }
        images.push(bins.get(&key)?.clone());
        ids.insert(key, images.len() - 1);
        Some(images.len() - 1)
    };
    let mut b = Builder::new(true, &mut resolve);
    b.feed(&tokens);
    let (blocks, _) = b.finish();
    if blocks.is_empty() {
        return Err("Couldn't find any text in this FB2 book".into());
    }
    Ok(Book { title, author, content: Content::Text { blocks, images }, chapters: Vec::new() })
}

fn cover_of(src: &str) -> Option<Vec<u8>> {
    let tokens = tokenize(src);
    let (_, _, cover) = description(&tokens);
    binaries(&tokens).remove(&cover?)
}

pub fn cover_file(path: &Path) -> Option<Vec<u8>> {
    cover_of(&decode(&std::fs::read(path).ok()?))
}

pub fn cover_zip(path: &Path) -> Option<Vec<u8>> {
    cover_of(&decode(&first_fb2_in_zip(path).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_small_book() {
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNGfake");
        let src = format!(
            r##"<?xml version="1.0" encoding="utf-8"?><FictionBook xmlns:l="http://www.w3.org/1999/xlink">
            <description><title-info><author><first-name>Leo</first-name><last-name>Tolstoy</last-name></author>
            <book-title>Kreutzer</book-title><coverpage><image l:href="#c.png"/></coverpage></title-info></description>
            <body><section><title><p>I</p></title><p>It was early spring.</p><image l:href="#c.png"/></section></body>
            <binary id="c.png" content-type="image/png">{png}</binary></FictionBook>"##
        );
        let book = parse(&src).unwrap();
        assert_eq!(book.title, "Kreutzer");
        assert_eq!(book.author.as_deref(), Some("Leo Tolstoy"));
        let Content::Text { blocks, images } = &book.content else { panic!() };
        assert_eq!(blocks.len(), 3);
        assert_eq!(images, &vec![b"\x89PNGfake".to_vec()]);
        assert_eq!(cover_of(&src).as_deref(), Some(&b"\x89PNGfake"[..]));
    }
}
