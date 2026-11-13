//! First-page covers for the Library shelf, rendered off the UI thread.
//!
//! A worker opens each requested document, renders page one at cover width
//! and hands the pixels back; the UI turns them into textures. Keyed by path
//! *and* file size + modification time, so a document changed on disk gets a
//! fresh cover rather than a stale one.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;

use crate::pdf::PdfEngine;

/// Cover width in physical pixels; tiles are drawn at about half this.
pub const COVER_PX: i32 = 360;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Key {
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
}

impl Key {
    fn of(path: &Path) -> Option<Key> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Key {
            path: path.to_path_buf(),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

enum Done {
    Ok(Key, usize, usize, Vec<u8>),
    Failed(Key),
}

pub enum Cover {
    Ready(egui::TextureHandle),
    /// Unreadable, password-protected, or missing — the tile shows its name.
    Unavailable,
}

pub struct Covers {
    engine: Option<PdfEngine>,
    map: HashMap<Key, Cover>,
    pending: HashSet<Key>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
}

impl Covers {
    pub fn new(engine: Option<PdfEngine>) -> Self {
        let (tx, rx) = channel();
        Self {
            engine,
            map: HashMap::new(),
            pending: HashSet::new(),
            tx,
            rx,
        }
    }

    /// The cover for `path`, if rendered; asks for it otherwise.
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> Option<&Cover> {
        let key = Key::of(path)?;
        if self.map.contains_key(&key) {
            return self.map.get(&key);
        }
        if self.pending.insert(key.clone()) {
            if let Some(engine) = self.engine {
                let tx = self.tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let done = match engine
                        .open(&key.path, None)
                        .ok()
                        .and_then(|doc| doc.render_page(0, COVER_PX, 0).ok())
                    {
                        Some(r) => Done::Ok(key, r.width, r.height, r.rgba),
                        None => Done::Failed(key),
                    };
                    let _ = tx.send(done);
                    ctx.request_repaint();
                });
            }
        }
        None
    }

    /// Turns finished renders into textures. Call once per frame.
    pub fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::Ok(key, w, h, rgba) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    let tex = ctx.load_texture(
                        format!("cover-{}", key.path.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.pending.remove(&key);
                    // A newer version of the same file supersedes the old cover.
                    self.map.retain(|k, _| k.path != key.path);
                    self.map.insert(key, Cover::Ready(tex));
                }
                Done::Failed(key) => {
                    self.pending.remove(&key);
                    self.map.insert(key, Cover::Unavailable);
                }
            }
        }
    }
}
