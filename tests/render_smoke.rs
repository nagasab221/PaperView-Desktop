//! Headless pipeline test: bind PDFium, open the fixture, render a page,
//! and sanity-check the pixels. Requires pdfium.dll in the project root
//! (see README) — the same copy the app uses during `cargo run`.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

use paperview::pdf::PdfEngine;

/// PDFium wants exactly one initialisation per process; tests run in parallel
/// threads, so they share one engine (double-init races trip PDFium CHECKs).
fn engine() -> &'static PdfEngine {
    static ENGINE: OnceLock<PdfEngine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        PdfEngine::new()
            .expect("pdfium.dll must be locatable (project root / exe dir) — see README")
    })
}

/// PDFium is single-threaded at heart: the `thread_safe` feature makes each
/// FFI call atomic, but interleaving page/text/search *sessions* from several
/// threads still corrupts results. Every test takes this lock first.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn renders_fixture_pages() {
    let _serial = serial();
    let doc = engine()
        .open(Path::new("tests/fixtures/sample.pdf"), None)
        .expect("fixture should open");

    assert_eq!(doc.page_count(), 3, "fixture has 3 pages");
    assert_eq!(doc.page_sizes_pt[0], (612.0, 792.0), "US Letter in points");

    let page = doc.render_page(0, 600, 0).expect("page 0 should render");
    assert_eq!(page.width, 600);
    // 792/612 * 600 ≈ 776 — allow rounding slack.
    assert!(
        (770..=782).contains(&page.height),
        "unexpected height {}",
        page.height
    );
    assert_eq!(page.rgba.len(), page.width * page.height * 4);

    // Page 1 carries black text and a *red* rectangle on white. Counting
    // pixels per class also catches BGRA/RGBA channel swaps.
    let (mut white, mut dark, mut red) = (0usize, 0usize, 0usize);
    for px in page.rgba.as_chunks::<4>().0 {
        let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
        assert_eq!(a, 255, "opaque page expected");
        if r > 240 && g > 240 && b > 240 {
            white += 1;
        } else if r < 100 && g < 100 && b < 100 {
            dark += 1;
        } else if r > 150 && g < 120 && b < 120 {
            red += 1;
        }
    }
    let total = page.width * page.height;
    assert!(white > total / 2, "mostly white page, got {white}/{total}");
    assert!(dark > 500, "text pixels expected, got {dark}");
    assert!(
        red > 5_000,
        "red rectangle expected, got {red} (channel swap?)"
    );

    // Last page renders too (bounds are respected).
    let last = doc.render_page(2, 300, 0).expect("page 3 should render");
    assert_eq!(last.width, 300);

    // Rotation: 90 deg swaps the aspect — the FINAL image is 300 wide and
    // correspondingly short (US Letter sideways: 300 * 612/792 ≈ 232).
    let rotated = doc.render_page(0, 300, 1).expect("rotated render");
    assert_eq!(
        rotated.width, 300,
        "target width applies to the final image"
    );
    assert!(
        (220..=245).contains(&rotated.height),
        "rotated height {} (expected ≈232)",
        rotated.height
    );
}

#[test]
fn finds_text_hits() {
    let _serial = serial();
    let doc = engine()
        .open(Path::new("tests/fixtures/sample.pdf"), None)
        .expect("fixture should open");

    // Case-insensitive: "paperview" matches "PaperView render-test fixture",
    // which appears exactly once per page.
    let hits = doc.page_hits(1, "paperview").expect("search should run");
    assert_eq!(hits.len(), 1, "one match on page 2");
    let r = hits[0].rects[0];
    // The footer line is drawn at x=72 pt, baseline y=520 pt (bottom-origin);
    // in top-left-origin coordinates its top sits around 792-534 ≈ 258 pt.
    assert!(r.x > 55.0 && r.x < 90.0, "x = {}", r.x);
    assert!(r.y > 230.0 && r.y < 300.0, "y = {}", r.y);
    assert!(r.w > 40.0, "w = {}", r.w);
    assert!(r.h > 5.0 && r.h < 40.0, "h = {}", r.h);

    // Headline only exists on its own page.
    let p3 = doc.page_hits(2, "Page 3 of 3").expect("search should run");
    assert_eq!(p3.len(), 1);
    let not_here = doc.page_hits(0, "Page 3 of 3").expect("search should run");
    assert!(not_here.is_empty());

    // No false positives.
    let none = doc.page_hits(0, "zzzzqqqq").expect("search should run");
    assert!(none.is_empty());
}

#[test]
fn text_selection_geometry() {
    let _serial = serial();
    let doc = engine()
        .open(Path::new("tests/fixtures/sample.pdf"), None)
        .expect("fixture should open");

    // Page 1's text starts with the headline.
    let n = doc.char_count(0).expect("count chars");
    assert!(n >= 40, "expected headline + footer chars, got {n}");
    assert_eq!(doc.range_text(0, 0, 11).unwrap(), "Page 1 of 3");

    // Hit-testing into the headline (top-origin coords: headline glyphs sit
    // around y ≈ 60–100 pt) returns a headline char index.
    let idx = doc
        .char_index_at(0, 100.0, 85.0, 20.0, 20.0, false)
        .expect("hit test runs")
        .expect("headline should be hit");
    assert!(idx <= 11, "expected headline char, got {idx}");

    // Double-click word selection around char 1 ('a' in "Page") → "Page".
    let (ws, wc) = doc.word_range_at(0, 1).expect("word range");
    assert_eq!(doc.range_text(0, ws, wc).unwrap(), "Page");

    // The headline is a single line — one merged rect of plausible size.
    let rects = doc.range_rects(0, 0, 11).expect("range rects");
    assert_eq!(rects.len(), 1, "one line expected");
    let r = rects[0];
    assert!(r.y > 40.0 && r.y < 120.0, "headline y = {}", r.y);
    assert!(r.w > 150.0, "headline width = {}", r.w);
    assert!(r.h > 20.0 && r.h < 80.0, "headline height = {}", r.h);

    // Vertical snapping (drag-continuation): below all text → last char,
    // above all text → first char.
    let below = doc
        .char_index_at(0, 500.0, 760.0, 20.0, 20.0, true)
        .expect("hit test runs")
        .expect("below-text point should snap to the last char");
    assert_eq!(below, n - 1);
    let above = doc
        .char_index_at(0, 500.0, 10.0, 20.0, 20.0, true)
        .expect("hit test runs")
        .expect("above-text point should snap to the first char");
    assert_eq!(above, 0);
    // Without snapping, the same far point resolves to nothing.
    let no_snap = doc
        .char_index_at(0, 500.0, 760.0, 20.0, 20.0, false)
        .expect("hit test runs");
    assert!(no_snap.is_none());
}

#[test]
fn cropbox_pages_use_displayed_coordinates() {
    let _serial = serial();
    let doc = engine()
        .open(Path::new("tests/fixtures/sample.pdf"), None)
        .expect("fixture should open");

    // Page 3 has CropBox [30 40 582 752]: the displayed box is 552×712 and
    // its origin is offset — all text coordinates must be reported relative
    // to the displayed box, not the raw MediaBox.
    let (w, h) = doc.page_sizes_pt[2];
    assert!((w - 552.0).abs() < 0.5, "cropped width {w}");
    assert!((h - 712.0).abs() < 0.5, "cropped height {h}");

    // Headline is drawn at x=72 pt baseline y=690 in RAW page space; in
    // displayed-box space that's x = 72-30 = 42, top ≈ 752-738 = 14…40.
    let rects = doc.range_rects(2, 0, 11).expect("headline rects");
    assert_eq!(rects.len(), 1);
    let r = rects[0];
    assert!(r.x > 35.0 && r.x < 50.0, "cropped headline x = {}", r.x);
    assert!(r.y > 5.0 && r.y < 60.0, "cropped headline y = {}", r.y);

    // Hit-testing round-trips through the same offset.
    let idx = doc
        .char_index_at(2, r.x + 5.0, r.y + r.h * 0.5, 15.0, 15.0, false)
        .expect("hit test runs")
        .expect("headline should be hit on cropped page");
    assert!(idx <= 11, "expected headline char, got {idx}");

    // Search hits share the coordinate fix.
    let hits = doc.page_hits(2, "Page 3 of 3").expect("search");
    assert_eq!(hits.len(), 1);
    let hr = hits[0].rects[0];
    assert!((hr.x - r.x).abs() < 2.0 && (hr.y - r.y).abs() < 2.0);
}

#[test]
fn outline_and_links() {
    let _serial = serial();
    let doc = engine()
        .open(Path::new("tests/fixtures/sample.pdf"), None)
        .expect("fixture should open");

    // Nested outline: "Page 2" (depth 0) with a child pointing at page 3.
    let toc = doc.toc();
    assert_eq!(toc.len(), 2, "two outline entries");
    assert_eq!(toc[0].title, "Page 2");
    assert_eq!(toc[0].depth, 0);
    assert_eq!(toc[0].page, Some(1));
    assert_eq!(toc[1].title, "Page 3 section");
    assert_eq!(toc[1].depth, 1);
    assert_eq!(toc[1].page, Some(2));

    // Internal link annotation: raw rect [72 510 300 540] → top-origin y
    // 252..282; probing its middle resolves to page 3 (index 2).
    let internal = doc
        .link_at(0, 150.0, 267.0)
        .expect("link probe runs")
        .expect("internal link expected");
    assert_eq!(internal, paperview::pdf::LinkTarget::Page(2));

    // URI link annotation: raw rect [72 470 300 500] → top-origin y 292..322.
    let uri = doc
        .link_at(0, 150.0, 307.0)
        .expect("link probe runs")
        .expect("uri link expected");
    assert_eq!(
        uri,
        paperview::pdf::LinkTarget::Uri("https://example.com/paperview".into())
    );

    // Empty area → no link.
    let none = doc.link_at(0, 400.0, 700.0).expect("link probe runs");
    assert!(none.is_none());
}
