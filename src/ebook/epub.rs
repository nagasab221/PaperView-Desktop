//! EPUB 2 and 3: a zip holding XHTML chapters, a package file (OPF) with
//! the reading order, and a table of contents (EPUB 3 nav or EPUB 2 NCX).

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use super::markup::{tokenize, Builder, Token};
use super::{Book, Chapter, Content};

type Archive = zip::ZipArchive<std::io::BufReader<std::fs::File>>;

fn archive(path: &Path) -> Result<Archive, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("Couldn't open the book: {e}"))?;
    zip::ZipArchive::new(std::io::BufReader::new(f))
        .map_err(|e| format!("This doesn't look like an EPUB ({e})"))
}

fn read(zip: &mut Archive, name: &str) -> Option<Vec<u8>> {
    let mut f = zip.by_name(name).ok()?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn read_text(zip: &mut Archive, name: &str) -> Option<String> {
    read(zip, name).map(|b| super::decode_text(&b))
}

/// `dir/../x.html#a` relative to `base` (a file inside the zip), normalised.
fn resolve(base: &str, href: &str) -> String {
    let href = percent_decode(href.split('#').next().unwrap_or(""));
    if href.is_empty() {
        return base.to_string();
    }
    let mut parts: Vec<&str> = base.rsplit_once('/').map_or(vec![], |(d, _)| d.split('/').collect());
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn fragment(href: &str) -> Option<&str> {
    href.split_once('#').map(|(_, f)| f).filter(|f| !f.is_empty())
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

struct Package {
    dir_of_opf: String,
    title: String,
    author: Option<String>,
    /// id → (href resolved inside the zip, media type, properties)
    manifest: HashMap<String, (String, String, String)>,
    spine: Vec<String>,
    ncx: Option<String>,
    cover_id: Option<String>,
}

fn package(zip: &mut Archive) -> Result<Package, String> {
    let container = read_text(zip, "META-INF/container.xml")
        .ok_or("This EPUB is missing its container file")?;
    let opf_path = tokenize(&container)
        .into_iter()
        .find_map(|t| match t {
            Token::Start { name, attrs, .. } if name == "rootfile" => {
                attr(&attrs, "full-path").map(|s| s.to_string())
            }
            _ => None,
        })
        .ok_or("This EPUB doesn't say where its contents are")?;
    let opf = read_text(zip, &opf_path).ok_or("This EPUB's package file is missing")?;

    let mut p = Package {
        dir_of_opf: opf_path.clone(),
        title: String::new(),
        author: None,
        manifest: HashMap::new(),
        spine: Vec::new(),
        ncx: None,
        cover_id: None,
    };
    let tokens = tokenize(&opf);
    let mut in_tag: Option<&str> = None;
    for t in &tokens {
        match t {
            Token::Start { name, attrs, .. } => match name.as_str() {
                "title" if p.title.is_empty() => in_tag = Some("title"),
                "creator" if p.author.is_none() => in_tag = Some("creator"),
                "item" => {
                    if let (Some(id), Some(href)) = (attr(attrs, "id"), attr(attrs, "href")) {
                        let media = attr(attrs, "media-type").unwrap_or("").to_string();
                        let props = attr(attrs, "properties").unwrap_or("").to_string();
                        if props.split_whitespace().any(|x| x == "cover-image") {
                            p.cover_id = Some(id.to_string());
                        }
                        p.manifest.insert(id.to_string(), (resolve(&opf_path, href), media, props));
                    }
                }
                "itemref" => {
                    if attr(attrs, "linear") != Some("no") {
                        if let Some(id) = attr(attrs, "idref") {
                            p.spine.push(id.to_string());
                        }
                    }
                }
                "spine" => p.ncx = attr(attrs, "toc").map(|s| s.to_string()),
                "meta"
                    if attr(attrs, "name") == Some("cover") && p.cover_id.is_none() => {
                        p.cover_id = attr(attrs, "content").map(|s| s.to_string());
                    }
                _ => {}
            },
            Token::Text(text) => match in_tag {
                Some("title") => p.title.push_str(text.trim()),
                Some("creator") => p.author = Some(text.trim().to_string()),
                _ => {}
            },
            Token::End { .. } => in_tag = None,
        }
    }
    if p.spine.is_empty() {
        return Err("This EPUB has no chapters".into());
    }
    Ok(p)
}

/// Table of contents entries: (title, href inside the zip incl. #fragment).
fn toc(zip: &mut Archive, p: &Package) -> Vec<(String, String)> {
    // EPUB 3: the nav document.
    if let Some((href, _, _)) = p.manifest.values().find(|(_, _, props)| props.contains("nav")) {
        if let Some(src) = read_text(zip, href) {
            let mut out = Vec::new();
            let mut in_toc = false;
            let mut depth = 0;
            let mut current: Option<(String, String)> = None;
            for t in tokenize(&src) {
                match t {
                    Token::Start { name, attrs, closed } if name == "nav" && !closed => {
                        let kind = attr(&attrs, "epub:type").or(attr(&attrs, "type")).unwrap_or("");
                        if kind.contains("toc") || !in_toc && out.is_empty() {
                            in_toc = true;
                            depth = 1;
                        } else if in_toc {
                            depth += 1;
                        }
                    }
                    Token::End { name } if name == "nav" && in_toc => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Token::Start { name, attrs, .. } if in_toc && name == "a" => {
                        if let Some(h) = attr(&attrs, "href") {
                            let frag = fragment(h).map(|f| format!("#{f}")).unwrap_or_default();
                            current = Some((String::new(), format!("{}{frag}", resolve(href, h))));
                        }
                    }
                    Token::Text(text) if current.is_some() => {
                        current.as_mut().unwrap().0.push_str(&text);
                    }
                    Token::End { name } if name == "a" => {
                        if let Some((title, h)) = current.take() {
                            let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
                            if !title.is_empty() {
                                out.push((title, h));
                            }
                        }
                    }
                    _ => {}
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
    }
    // EPUB 2: the NCX.
    let ncx_href = p
        .ncx
        .as_ref()
        .and_then(|id| p.manifest.get(id))
        .or_else(|| p.manifest.values().find(|(_, m, _)| m == "application/x-dtbncx+xml"))
        .map(|(h, _, _)| h.clone());
    let Some(href) = ncx_href else { return Vec::new() };
    let Some(src) = read_text(zip, &href) else { return Vec::new() };
    let mut out = Vec::new();
    let mut label = String::new();
    let mut in_text = false;
    for t in tokenize(&src) {
        match t {
            Token::Start { name, .. } if name == "navlabel" => label.clear(),
            Token::Start { name, .. } if name == "text" => in_text = true,
            Token::End { name } if name == "text" => in_text = false,
            Token::Text(text) if in_text => label.push_str(&text),
            Token::Start { name, attrs, .. } if name == "content" => {
                if let Some(src) = attr(&attrs, "src") {
                    let frag = fragment(src).map(|f| format!("#{f}")).unwrap_or_default();
                    let title = label.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !title.is_empty() {
                        out.push((title, format!("{}{frag}", resolve(&href, src))));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub fn open(path: &Path) -> Result<Book, String> {
    let mut zip = archive(path)?;
    let pkg = package(&mut zip)?;
    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut image_ids: HashMap<String, usize> = HashMap::new();

    let mut all_blocks = Vec::new();
    let mut anchors: HashMap<String, usize> = HashMap::new();
    for idref in &pkg.spine {
        let Some((href, media, _)) = pkg.manifest.get(idref).cloned() else { continue };
        if !(media.contains("html") || media.contains("xml") || href.ends_with("htm") || href.ends_with("html")) {
            continue;
        }
        let Some(src) = read_text(&mut zip, &href) else { continue };
        let chapter_href = href.clone();
        let mut resolve_image = |r: &str| -> Option<usize> {
            let target = resolve(&chapter_href, r);
            if let Some(&id) = image_ids.get(&target) {
                return Some(id);
            }
            let bytes = read(&mut zip, &target)?;
            images.push(bytes);
            image_ids.insert(target, images.len() - 1);
            Some(images.len() - 1)
        };
        let mut b = Builder::new(false, &mut resolve_image);
        b.anchor(&href);
        b.feed(&tokenize(&src));
        let (blocks, local) = b.finish();
        let offset = all_blocks.len();
        for (id, at) in local {
            let key = if id == href { href.clone() } else { format!("{href}#{id}") };
            anchors.entry(key).or_insert(offset + at);
        }
        all_blocks.extend(blocks);
    }

    let chapters = toc(&mut zip, &pkg)
        .into_iter()
        .filter_map(|(title, href)| {
            let block = anchors
                .get(&href)
                .or_else(|| anchors.get(href.split('#').next().unwrap_or("")))?;
            Some(Chapter { title, block: *block })
        })
        .collect();

    Ok(Book {
        title: pkg.title,
        author: pkg.author,
        content: Content::Text { blocks: all_blocks, images },
        chapters,
    })
}

pub fn cover(path: &Path) -> Option<Vec<u8>> {
    let mut zip = archive(path).ok()?;
    let pkg = package(&mut zip).ok()?;
    if let Some((href, _, _)) = pkg.cover_id.as_ref().and_then(|id| pkg.manifest.get(id)) {
        if let Some(b) = read(&mut zip, href) {
            return Some(b);
        }
    }
    // No declared cover: an image whose name says "cover", or the first one.
    let images: Vec<String> = pkg
        .manifest
        .values()
        .filter(|(_, m, _)| m.starts_with("image/"))
        .map(|(h, _, _)| h.clone())
        .collect();
    let pick = images
        .iter()
        .find(|h| h.to_ascii_lowercase().contains("cover"))
        .or_else(|| images.first())?
        .clone();
    let _ = &pkg.dir_of_opf;
    read(&mut zip, &pick)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hrefs_resolve_inside_the_zip() {
        assert_eq!(resolve("OEBPS/content.opf", "Text/ch1.xhtml"), "OEBPS/Text/ch1.xhtml");
        assert_eq!(resolve("OEBPS/Text/ch1.xhtml", "../Images/a%20b.jpg"), "OEBPS/Images/a b.jpg");
        assert_eq!(resolve("ch1.html", "ch2.html#x"), "ch2.html");
        assert_eq!(fragment("ch2.html#x"), Some("x"));
    }
}
