//! Reads real books from `$PAPERVIEW_TEST_BOOKS` (skipped when unset): every
//! book must open, have text or pages, and report what it found.

use paperview::ebook::{self, Block, Content};

#[test]
fn real_books_open() {
    let Some(dir) = std::env::var_os("PAPERVIEW_TEST_BOOKS") else {
        eprintln!("PAPERVIEW_TEST_BOOKS not set; skipping");
        return;
    };
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths.iter().filter(|p| ebook::is_ebook(p)) {
        let t = std::time::Instant::now();
        let book = ebook::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let ms = t.elapsed().as_millis();
        match &book.content {
            Content::Text { blocks, images } => {
                let words: usize = blocks
                    .iter()
                    .map(|b| match b {
                        Block::Para { runs } | Block::Heading { runs, .. } => {
                            runs.iter().map(|r| r.text.split_whitespace().count()).sum()
                        }
                        _ => 0,
                    })
                    .sum();
                let first = blocks.iter().find_map(|b| match b {
                    Block::Para { runs } => {
                        Some(runs.iter().map(|r| r.text.as_str()).collect::<String>())
                    }
                    _ => None,
                });
                eprintln!(
                    "{}: \"{}\" by {:?} — {} blocks, {} words, {} images, {} chapters, {} ms; first para: {:.80?}",
                    path.file_name().unwrap().to_string_lossy(),
                    book.title,
                    book.author,
                    blocks.len(),
                    words,
                    images.len(),
                    book.chapters.len(),
                    ms,
                    first.unwrap_or_default()
                );
                assert!(words > 1000, "{}: only {words} words", path.display());
            }
            Content::Comic { pages } => {
                eprintln!("{}: comic, {} pages", path.display(), pages.len());
                assert!(!pages.is_empty());
            }
        }
        let cover = ebook::cover(path).map(|c| c.len());
        eprintln!("    cover bytes: {cover:?}");
    }
}
