//! Comic book archives (CBZ): a zip of page images, read in name order.

use std::io::Read;
use std::path::Path;

use super::{Book, Content};

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];

fn archive(path: &Path) -> Option<zip::ZipArchive<std::io::BufReader<std::fs::File>>> {
    let f = std::fs::File::open(path).ok()?;
    zip::ZipArchive::new(std::io::BufReader::new(f)).ok()
}

/// "page 2" before "page 10".
fn natural_key(s: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut num = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            if !num.is_empty() {
                out.push((std::mem::take(&mut text), num.parse().unwrap_or(0)));
                num.clear();
            }
            text.push(c);
        }
    }
    out.push((text, num.parse().unwrap_or(0)));
    out
}

fn pages(path: &Path) -> Option<Vec<String>> {
    let mut zip = archive(path)?;
    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| {
            let lower = n.to_ascii_lowercase();
            !lower.contains("__macosx")
                && IMAGE_EXTS.iter().any(|e| lower.ends_with(&format!(".{e}")))
        })
        .collect();
    names.sort_by_cached_key(|n| natural_key(n));
    Some(names)
}

pub fn open(path: &Path) -> Result<Book, String> {
    let pages = pages(path).ok_or("This doesn't look like a comic archive")?;
    if pages.is_empty() {
        return Err("There are no pages in this comic".into());
    }
    Ok(Book {
        title: String::new(),
        author: None,
        content: Content::Comic { pages },
        chapters: Vec::new(),
    })
}

pub fn page(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut zip = archive(path)?;
    let mut f = zip.by_name(name).ok()?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

pub fn cover(path: &Path) -> Option<Vec<u8>> {
    let first = pages(path)?.into_iter().next()?;
    page(path, &first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["p10.jpg", "p2.jpg", "p1.jpg"];
        v.sort_by_cached_key(|n| natural_key(n));
        assert_eq!(v, ["p1.jpg", "p2.jpg", "p10.jpg"]);
    }
}
