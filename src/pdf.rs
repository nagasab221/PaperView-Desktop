//! PDFium binding + document handling, kept UI-free.

use std::path::{Path, PathBuf};

use pdfium_render::prelude::*;

/// PDFium is single-threaded at heart: every public operation takes this
/// process-wide gate so the print worker and the UI thread never interleave
/// FFI sessions (interleaving corrupts results — see the test suite notes).
fn gate() -> std::sync::MutexGuard<'static, ()> {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Wraps the process-wide PDFium instance.
///
/// PDFium is initialised once and intentionally leaked to `'static`: documents
/// borrow from `Pdfium`, and a `&'static` reference sidesteps a self-referential
/// struct. One instance per process is exactly what PDFium wants anyway.
#[derive(Clone, Copy)]
pub struct PdfEngine {
    pdfium: &'static Pdfium,
}

impl PdfEngine {
    /// Locates pdfium.dll and initialises PDFium.
    ///
    /// Search order: next to the executable, then the working directory, then
    /// the system loader path.
    pub fn new() -> Result<Self, String> {
        let bindings = Self::bind()?;
        let pdfium: &'static Pdfium = Box::leak(Box::new(Pdfium::new(bindings)));
        Ok(Self { pdfium })
    }

    fn bind() -> Result<Box<dyn PdfiumLibraryBindings>, String> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                dirs.push(dir.to_path_buf());
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            dirs.push(cwd);
        }

        for dir in &dirs {
            if let Ok(bindings) =
                Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(dir))
            {
                return Ok(bindings);
            }
        }
        Pdfium::bind_to_system_library().map_err(|e| {
            format!(
                "Could not load pdfium.dll.\n\n\
                 Searched next to the executable, in the working directory, and on the system path.\n\
                 Download `pdfium-win-x64.tgz` from\n\
                 https://github.com/bblanchon/pdfium-binaries/releases\n\
                 and place `pdfium.dll` next to paperview.exe.\n\n({e:?})"
            )
        })
    }

    /// Opens a document from disk, optionally with a password.
    pub fn open(&self, path: &Path, password: Option<&str>) -> Result<Document, OpenError> {
        let _gate = gate();
        let doc = match self.pdfium.load_pdf_from_file(path, password) {
            Ok(doc) => doc,
            Err(PdfiumError::PdfiumLibraryInternalError(
                PdfiumInternalError::PasswordError,
            )) => return Err(OpenError::PasswordRequired),
            Err(e) => {
                return Err(OpenError::Message(format!(
                    "Could not open {}: {e:?}",
                    path.display()
                )))
            }
        };

        let pages = doc.pages();
        let count = pages.len();
        let mut page_sizes_pt = Vec::with_capacity(count as usize);
        let mut page_origins_pt = Vec::with_capacity(count as usize);
        for index in 0..count {
            let page = pages
                .get(index)
                .map_err(|e| OpenError::Message(format!("Could not read page {}: {e:?}", index + 1)))?;
            let (w, h) = (page.width().value, page.height().value);
            page_sizes_pt.push((w, h));
            // Text/search coordinates are in raw page space; the *displayed*
            // area is the bounding box (CropBox ∩ MediaBox), whose origin is
            // not always (0, 0). Remember (left, top) per page so all
            // coordinates can be reported relative to the displayed box.
            let (left, top) = page
                .boundaries()
                .bounding()
                .map(|b| (b.bounds.left().value, b.bounds.top().value))
                .unwrap_or((0.0, h));
            page_origins_pt.push((left, top));
        }

        Ok(Document {
            doc,
            path: path.to_path_buf(),
            page_sizes_pt,
            page_origins_pt,
        })
    }
}

/// Why a document failed to open.
#[derive(Debug)]
pub enum OpenError {
    /// The file is encrypted and needs a (different) password.
    PasswordRequired,
    Message(String),
}

/// The target of a clickable link region on a page.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    /// Jump to a page (0-based).
    Page(i32),
    /// Open an external URL.
    Uri(String),
}

/// One entry of the document outline (table of contents).
#[derive(Clone, Debug)]
pub struct TocEntry {
    pub title: String,
    pub page: Option<i32>,
    pub depth: u8,
}

/// An open PDF document plus cached per-page metadata.
pub struct Document {
    doc: PdfDocument<'static>,
    pub path: PathBuf,
    /// (width, height) of every page's displayed box, PDF points (1/72 in).
    pub page_sizes_pt: Vec<(f32, f32)>,
    /// (left, top) of every page's displayed box in raw page coordinates.
    page_origins_pt: Vec<(f32, f32)>,
}

/// One page rendered to raw RGBA8 pixels.
pub struct RenderedPage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// A search-hit rectangle on a page, in PDF points with a **top-left** origin
/// (already flipped from PDF's bottom-left convention so it maps straight to
/// screen space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One search match. A match that wraps across lines spans several segments,
/// hence several rectangles.
#[derive(Clone, Debug)]
pub struct PageHit {
    pub rects: Vec<HitRect>,
}

impl Document {
    pub fn page_count(&self) -> i32 {
        self.page_sizes_pt.len() as i32
    }

    /// Renders a page to RGBA at the given pixel width of the FINAL image
    /// (aspect preserved), rotated by `quarter_turns` × 90° clockwise.
    pub fn render_page(
        &self,
        index: i32,
        target_width_px: i32,
        quarter_turns: u8,
    ) -> Result<RenderedPage, String> {
        let _gate = gate();
        let page = self
            .doc
            .pages()
            .get(index)
            .map_err(|e| format!("Could not read page {}: {e:?}", index + 1))?;
        // Scale explicitly from the FINAL (post-rotation) width — target-width
        // constraints apply pre-rotation and would swap meaning at 90/270°.
        let (w_pt, h_pt) = self.page_sizes_pt[index as usize];
        let final_w_pt = if quarter_turns % 2 == 1 { h_pt } else { w_pt };
        let scale = target_width_px as f32 / final_w_pt.max(1.0);
        let mut config = PdfRenderConfig::new()
            .scale_page_by_factor(scale)
            .render_annotations(true)
            .render_form_data(true);
        let rotation = match quarter_turns % 4 {
            1 => Some(PdfPageRenderRotation::Degrees90),
            2 => Some(PdfPageRenderRotation::Degrees180),
            3 => Some(PdfPageRenderRotation::Degrees270),
            _ => None,
        };
        if let Some(r) = rotation {
            config = config.rotate(r, true);
        }
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| format!("Could not render page {}: {e:?}", index + 1))?;
        Ok(RenderedPage {
            width: bitmap.width() as usize,
            height: bitmap.height() as usize,
            rgba: bitmap.as_rgba_bytes(),
        })
    }

    /// The link (if any) under a point on a page, in displayed-box top-left
    /// coordinates (PDF points).
    pub fn link_at(&self, index: i32, x: f32, y: f32) -> Result<Option<LinkTarget>, String> {
        let _gate = gate();
        let page = self
            .doc
            .pages()
            .get(index)
            .map_err(|e| format!("Could not read page {}: {e:?}", index + 1))?;
        let (left, top) = self.page_origins_pt[index as usize];
        let links = page.links();
        let Some(link) = links.link_at_point(PdfPoints::new(x + left), PdfPoints::new(top - y))
        else {
            return Ok(None);
        };
        // Internal destination first, then the action (URI etc.).
        if let Some(dest) = link.destination() {
            if let Ok(p) = dest.page_index() {
                return Ok(Some(LinkTarget::Page(p as i32)));
            }
        }
        if let Some(action) = link.action() {
            match action {
                PdfAction::Uri(uri_action) => {
                    if let Ok(uri) = uri_action.uri() {
                        return Ok(Some(LinkTarget::Uri(uri)));
                    }
                }
                PdfAction::LocalDestination(local) => {
                    if let Ok(dest) = local.destination() {
                        if let Ok(p) = dest.page_index() {
                            return Ok(Some(LinkTarget::Page(p as i32)));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// The document outline (table of contents), flattened with depths.
    pub fn toc(&self) -> Vec<TocEntry> {
        let _gate = gate();
        fn resolve_page(bookmark: &PdfBookmark<'_>) -> Option<i32> {
            if let Some(dest) = bookmark.destination() {
                if let Ok(p) = dest.page_index() {
                    return Some(p as i32);
                }
            }
            if let Some(PdfAction::LocalDestination(local)) = bookmark.action() {
                if let Ok(dest) = local.destination() {
                    if let Ok(p) = dest.page_index() {
                        return Some(p as i32);
                    }
                }
            }
            None
        }
        fn walk(out: &mut Vec<TocEntry>, bookmark: &PdfBookmark<'_>, depth: u8) {
            if depth > 4 || out.len() >= 2000 {
                return;
            }
            out.push(TocEntry {
                title: bookmark.title().unwrap_or_default(),
                page: resolve_page(bookmark),
                depth,
            });
            for child in bookmark.iter_direct_children() {
                walk(out, &child, depth + 1);
            }
        }
        let mut out = Vec::new();
        // `bookmarks().iter()` is a depth-first walk of the whole tree, which
        // would duplicate children under our own recursion — walk the top
        // level (root + its siblings) explicitly instead.
        if let Some(root) = self.doc.bookmarks().root() {
            walk(&mut out, &root, 0);
            for sibling in root.iter_siblings() {
                walk(&mut out, &sibling, 0);
            }
        }
        out
    }

    /// Number of extractable text characters on a page.
    pub fn char_count(&self, index: i32) -> Result<i32, String> {
        let _gate = gate();
        let page = self
            .doc
            .pages()
            .get(index)
            .map_err(|e| format!("Could not read page {}: {e:?}", index + 1))?;
        let text = page
            .text()
            .map_err(|e| format!("Could not extract text from page {}: {e:?}", index + 1))?;
        Ok(text.chars().len() as i32)
    }

    /// Index of the character nearest to a point on a page. Point and
    /// tolerances are in PDF points with a top-left origin.
    ///
    /// With `snap_vertical`, a point above the first / below the last text
    /// line resolves to the first / last character (so drag-selection past
    /// the ends of the text block behaves like other viewers) — PDFium's own
    /// tolerance search does not snap across large distances.
    pub fn char_index_at(
        &self,
        index: i32,
        x: f32,
        y: f32,
        tol_x: f32,
        tol_y: f32,
        snap_vertical: bool,
    ) -> Result<Option<i32>, String> {
        let _gate = gate();
        let page = self
            .doc
            .pages()
            .get(index)
            .map_err(|e| format!("Could not read page {}: {e:?}", index + 1))?;
        let (left, top) = self.page_origins_pt[index as usize];
        let text = page
            .text()
            .map_err(|e| format!("Could not extract text from page {}: {e:?}", index + 1))?;
        let chars = text.chars();
        let x_pdf = x + left;
        let y_pdf = top - y; // flip to PDF's bottom-left origin (displayed box)
        if let Some(c) = chars.get_char_near_point(
            PdfPoints::new(x_pdf),
            PdfPoints::new(tol_x),
            PdfPoints::new(y_pdf),
            PdfPoints::new(tol_y),
        ) {
            return Ok(Some(c.index() as i32));
        }
        if snap_vertical {
            let count = chars.len() as i32;
            if count > 0 {
                // Reading order is top-down for ordinary documents: below the
                // last char's line → last char; above the first char's line
                // → first char. (PDF y grows upward.)
                let first = chars.get(0).ok().and_then(|c| c.loose_bounds().ok());
                let last = chars
                    .get((count - 1) as _)
                    .ok()
                    .and_then(|c| c.loose_bounds().ok());
                if let Some(b) = last {
                    if y_pdf < b.bottom().value {
                        return Ok(Some(count - 1));
                    }
                }
                if let Some(b) = first {
                    if y_pdf > b.top().value {
                        return Ok(Some(0));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Merged per-line rectangles for a character range (top-left origin).
    pub fn range_rects(&self, index: i32, start: i32, count: i32) -> Result<Vec<HitRect>, String> {
        let _gate = gate();
        let page = self
            .doc
            .pages()
            .get(index)
            .map_err(|e| format!("Could not read page {}: {e:?}", index + 1))?;
        let (left, top) = self.page_origins_pt[index as usize];
        let text = page
            .text()
            .map_err(|e| format!("Could not extract text from page {}: {e:?}", index + 1))?;
        let mut rects = Vec::new();
        for segment in text.segments_subset(start as _, count as _).iter() {
            let b = segment.bounds();
            rects.push(HitRect {
                x: b.left().value - left,
                y: top - b.top().value,
                w: b.width().value,
                h: b.height().value,
            });
        }
        Ok(rects)
    }

    /// Extracted text for a character range (exact, index-aligned).
    pub fn range_text(&self, index: i32, start: i32, count: i32) -> Result<String, String> {
        let _gate = gate();
        let page = self
            .doc
            .pages()
            .get(index)
            .map_err(|e| format!("Could not read page {}: {e:?}", index + 1))?;
        let text = page
            .text()
            .map_err(|e| format!("Could not extract text from page {}: {e:?}", index + 1))?;
        let chars = text.chars();
        let mut out = String::with_capacity(count.max(0) as usize);
        for i in start..start.saturating_add(count) {
            if let Some(ch) = chars.get(i as _).ok().and_then(|c| c.unicode_char()) {
                out.push(ch);
            }
        }
        Ok(out)
    }

    /// (start, count) of the word around a character — for double-click select.
    pub fn word_range_at(&self, index: i32, char_idx: i32) -> Result<(i32, i32), String> {
        let _gate = gate();
        let page = self
            .doc
            .pages()
            .get(index)
            .map_err(|e| format!("Could not read page {}: {e:?}", index + 1))?;
        let text = page
            .text()
            .map_err(|e| format!("Could not extract text from page {}: {e:?}", index + 1))?;
        let chars = text.chars();
        let total = chars.len() as i32;
        let ch_at = |i: i32| -> Option<char> {
            if i < 0 || i >= total {
                return None;
            }
            chars.get(i as _).ok().and_then(|c| c.unicode_char())
        };
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !ch_at(char_idx).map(&is_word).unwrap_or(false) {
            return Ok((char_idx, 1));
        }
        let mut s = char_idx;
        let mut e = char_idx;
        for _ in 0..200 {
            match ch_at(s - 1) {
                Some(c) if is_word(c) => s -= 1,
                _ => break,
            }
        }
        for _ in 0..200 {
            match ch_at(e + 1) {
                Some(c) if is_word(c) => e += 1,
                _ => break,
            }
        }
        Ok((s, e - s + 1))
    }

    /// Finds all case-insensitive matches of `query` on one page, using
    /// PDFium's native text search.
    pub fn page_hits(&self, index: i32, query: &str) -> Result<Vec<PageHit>, String> {
        let _gate = gate();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let page = self
            .doc
            .pages()
            .get(index)
            .map_err(|e| format!("Could not read page {}: {e:?}", index + 1))?;
        let (left, top) = self.page_origins_pt[index as usize];
        let text = page
            .text()
            .map_err(|e| format!("Could not extract text from page {}: {e:?}", index + 1))?;

        let options = PdfSearchOptions::new()
            .match_case(false)
            .match_whole_word(false);
        let search = text
            .search(query, &options)
            .map_err(|e| format!("Search failed on page {}: {e:?}", index + 1))?;

        let mut hits = Vec::new();
        while let Some(segments) = search.find_next() {
            let mut rects = Vec::new();
            for segment in segments.iter() {
                let b = segment.bounds();
                rects.push(HitRect {
                    x: b.left().value - left,
                    y: top - b.top().value, // flip to top-left origin of the displayed box
                    w: b.width().value,
                    h: b.height().value,
                });
            }
            if !rects.is_empty() {
                hits.push(PageHit { rects });
            }
        }
        Ok(hits)
    }
}
