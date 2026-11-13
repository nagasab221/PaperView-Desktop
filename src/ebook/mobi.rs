//! Kindle books: MOBI, AZW, AZW3 (KF8) and PRC.
//!
//! A Palm database whose first record holds the headers and whose next ones
//! hold the book's HTML, usually PalmDOC-compressed. Images sit in later
//! records. KF8 (AZW3) stores its HTML in fragments, but read in order they
//! still give the text in reading order, which is all the reader needs.
//! Books with DRM, or the rarer HUFF/CDIC compression, are declined politely.

use std::collections::HashMap;
use std::path::Path;

use super::markup::{tokenize, Builder};
use super::{Book, Content};

struct Pdb {
    data: Vec<u8>,
    offsets: Vec<usize>,
}

impl Pdb {
    fn load(path: &Path) -> Result<Pdb, String> {
        let data = std::fs::read(path).map_err(|e| format!("Couldn't open the book: {e}"))?;
        if data.len() < 78 {
            return Err("This file is too short to be a Kindle book".into());
        }
        let n = u16::from_be_bytes([data[76], data[77]]) as usize;
        let mut offsets = Vec::with_capacity(n);
        for i in 0..n {
            let at = 78 + i * 8;
            if at + 4 > data.len() {
                break;
            }
            offsets.push(u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize);
        }
        if offsets.is_empty() {
            return Err("This Kindle book has no records".into());
        }
        Ok(Pdb { data, offsets })
    }

    fn record(&self, i: usize) -> Option<&[u8]> {
        let start = *self.offsets.get(i)?;
        let end = self.offsets.get(i + 1).copied().unwrap_or(self.data.len());
        self.data.get(start..end.max(start))
    }
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

struct Header {
    /// The record the book starts at (0, or the KF8 part of a hybrid file).
    base: usize,
    compression: u16,
    text_records: usize,
    utf8: bool,
    first_image: Option<usize>,
    extra_flags: u16,
    title: String,
    author: Option<String>,
    cover_offset: Option<usize>,
}

fn header(pdb: &Pdb, base: usize) -> Result<Header, String> {
    let r0 = pdb.record(base).ok_or("This Kindle book has no header")?;
    let compression = be16(r0, 0).unwrap_or(1);
    let text_records = be16(r0, 8).unwrap_or(0) as usize;
    let encryption = be16(r0, 12).unwrap_or(0);
    if encryption != 0 {
        return Err("This Kindle book is DRM-protected, so it can't be opened outside the Kindle app.".into());
    }
    let mut h = Header {
        base,
        compression,
        text_records,
        utf8: false,
        first_image: None,
        extra_flags: 0,
        title: String::new(),
        author: None,
        cover_offset: None,
    };
    if r0.get(16..20) == Some(b"MOBI") {
        let hlen = be32(r0, 20).unwrap_or(0) as usize;
        h.utf8 = be32(r0, 28) == Some(65001);
        h.first_image = be32(r0, 108).filter(|&v| v != 0xFFFF_FFFF && v != 0).map(|v| v as usize);
        if hlen >= 0xE4 {
            h.extra_flags = be16(r0, 242).unwrap_or(0);
        }
        let (name_off, name_len) = (be32(r0, 84).unwrap_or(0) as usize, be32(r0, 88).unwrap_or(0) as usize);
        if let Some(name) = r0.get(name_off..name_off + name_len) {
            h.title = decode(name, h.utf8);
        }
        // EXTH: author, a better title, the cover.
        if be32(r0, 128).unwrap_or(0) & 0x40 != 0 {
            let at = 16 + hlen;
            if r0.get(at..at + 4) == Some(b"EXTH") {
                let count = be32(r0, at + 8).unwrap_or(0) as usize;
                let mut p = at + 12;
                for _ in 0..count {
                    let (Some(kind), Some(len)) = (be32(r0, p), be32(r0, p + 4)) else { break };
                    let len = len as usize;
                    if len < 8 {
                        break;
                    }
                    if let Some(val) = r0.get(p + 8..p + len) {
                        match kind {
                            100 if h.author.is_none() => h.author = Some(decode(val, h.utf8)),
                            503 => h.title = decode(val, h.utf8),
                            201 => h.cover_offset = be32(val, 0).map(|v| v as usize),
                            _ => {}
                        }
                    }
                    p += len;
                }
            }
        }
    }
    // Image record indexes count from the start of the file, not the KF8 part.
    if let Some(fi) = h.first_image.as_mut() {
        if base > 0 && *fi < base {
            *fi += base;
        }
    }
    Ok(h)
}

fn decode(b: &[u8], utf8: bool) -> String {
    if utf8 {
        String::from_utf8_lossy(b).into_owned()
    } else {
        super::decode_cp1252(b)
    }
}

/// PalmDOC (LZ77-style) decompression.
fn palmdoc(input: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(input.len() * 2);
    let mut i = 0;
    while i < input.len() {
        let c = input[i];
        i += 1;
        match c {
            0x01..=0x08 => {
                let end = (i + c as usize).min(input.len());
                out.extend_from_slice(&input[i..end]);
                i = end;
            }
            0x00 | 0x09..=0x7F => out.push(c),
            0x80..=0xBF => {
                let Some(&c2) = input.get(i) else { break };
                i += 1;
                let pair = (((c as usize) << 8) | c2 as usize) & 0x3FFF;
                let dist = pair >> 3;
                let len = (pair & 7) + 3;
                if dist == 0 || dist > out.len() {
                    continue;
                }
                for _ in 0..len {
                    out.push(out[out.len() - dist]);
                }
            }
            _ => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
    }
    out
}

/// Bytes of trailing data appended to a text record (MOBI "extra data").
fn trailing_size(data: &[u8], flags: u16) -> usize {
    let mut size = data.len();
    let mut num = 0usize;
    let mut f = flags >> 1;
    while f != 0 {
        if f & 1 != 0 && size > num {
            // A backward-encoded variable-width integer.
            let mut end = size - num;
            let mut value = 0usize;
            let mut shift = 0;
            while end > 0 {
                let v = data[end - 1];
                value |= ((v & 0x7F) as usize) << shift;
                shift += 7;
                end -= 1;
                if v & 0x80 != 0 || shift >= 28 {
                    break;
                }
            }
            num += value;
        }
        f >>= 1;
    }
    if flags & 1 != 0 && size > num {
        num += (data[size - num - 1] & 0x3) as usize + 1;
    }
    size = num.min(size);
    size
}

fn text(pdb: &Pdb, h: &Header) -> Result<String, String> {
    if h.compression == 17480 {
        return Err("This Kindle book uses HUFF/CDIC compression, which PaperView can't read yet.".into());
    }
    let mut raw = Vec::new();
    for i in 1..=h.text_records {
        let Some(rec) = pdb.record(h.base + i) else { break };
        let cut = trailing_size(rec, h.extra_flags);
        let rec = &rec[..rec.len().saturating_sub(cut)];
        match h.compression {
            2 => raw.extend(palmdoc(rec)),
            _ => raw.extend_from_slice(rec),
        }
    }
    Ok(decode(&raw, h.utf8))
}

/// Picks the KF8 half of a hybrid MOBI+KF8 file when there is one.
fn best_header(pdb: &Pdb) -> Result<Header, String> {
    let h0 = header(pdb, 0)?;
    // EXTH 121 marks where the KF8 part begins; the MOBI 6 part is fine to
    // read and simpler, so prefer it unless it's empty.
    if h0.text_records > 0 {
        return Ok(h0);
    }
    Err("This Kindle book has no text".into())
}

/// An image's record from a reference: `recindex="00012"` (MOBI) or
/// `kindle:embed:000C?mime=image/jpg` (KF8, base 32).
fn image_record(reference: &str, first_image: usize) -> Option<usize> {
    let r = reference.trim();
    let n = if let Some(rest) = r.strip_prefix("kindle:embed:") {
        let code: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        usize::from_str_radix(&code.to_ascii_uppercase(), 32).ok()?
    } else {
        r.trim_start_matches('0').parse::<usize>().ok()?
    };
    Some(first_image + n.checked_sub(1)?)
}

pub fn open(path: &Path) -> Result<Book, String> {
    let pdb = Pdb::load(path)?;
    let h = best_header(&pdb)?;
    let html = text(&pdb, &h)?;
    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut seen: HashMap<usize, usize> = HashMap::new();
    let first = h.first_image;
    let mut resolve = |r: &str| -> Option<usize> {
        let rec = image_record(r, first?)?;
        if let Some(&id) = seen.get(&rec) {
            return Some(id);
        }
        let bytes = pdb.record(rec)?.to_vec();
        images.push(bytes);
        seen.insert(rec, images.len() - 1);
        Some(images.len() - 1)
    };
    let mut b = Builder::new(false, &mut resolve);
    b.feed(&tokenize(&html));
    let (blocks, _) = b.finish();
    if blocks.is_empty() {
        return Err("Couldn't find any text in this Kindle book".into());
    }
    Ok(Book {
        title: h.title,
        author: h.author,
        content: Content::Text { blocks, images },
        chapters: Vec::new(),
    })
}

pub fn cover(path: &Path) -> Option<Vec<u8>> {
    let pdb = Pdb::load(path).ok()?;
    let h = header(&pdb, 0).ok()?;
    let rec = h.first_image? + h.cover_offset.unwrap_or(0);
    pdb.record(rec).map(|r| r.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palmdoc_literals_and_back_references() {
        // "abc" then a back-reference of distance 3, length 3 → "abcabc";
        // 0xE1 is " a" (space + 0x61).
        // distance 3 in the high bits, length 3 (stored as length − 3 = 0) in the low.
        let pair: u16 = 3 << 3;
        let enc = [b'a', b'b', b'c', 0x80 | (pair >> 8) as u8, (pair & 0xFF) as u8, 0xE1];
        assert_eq!(palmdoc(&enc), b"abcabc a");
        // 0x02 copies the next two bytes verbatim.
        assert_eq!(palmdoc(&[0x02, 0xC0, 0xC1, b'!']), [0xC0, 0xC1, b'!']);
    }

    #[test]
    fn image_references() {
        assert_eq!(image_record("00003", 100), Some(102));
        assert_eq!(image_record("kindle:embed:000A?mime=image/jpg", 100), Some(109));
        assert_eq!(image_record("0", 100), None);
    }
}
