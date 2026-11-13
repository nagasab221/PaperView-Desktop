//! First-page covers for the Library shelves, rendered off the UI thread.
//!
//! One worker thread takes requests in order, opens each document, renders
//! page one at cover width and hands the pixels back; the UI turns them into
//! textures. Callers only ask for covers that are on screen, so a folder of a
//! thousand PDFs costs what the visible tiles cost. Keyed by path *and* file
//! size + modification time, so a document changed on disk gets a fresh cover.

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
    /// Requests to the worker; started on first use.
    work: Option<Sender<Key>>,
    done_tx: Sender<Done>,
    done_rx: Receiver<Done>,
}

impl Covers {
    pub fn new(engine: Option<PdfEngine>) -> Self {
        let (done_tx, done_rx) = channel();
        Self {
            engine,
            map: HashMap::new(),
            pending: HashSet::new(),
            work: None,
            done_tx,
            done_rx,
        }
    }

    fn worker(&mut self, ctx: &egui::Context) -> Option<&Sender<Key>> {
        if self.work.is_none() {
            let engine = self.engine?;
            let (tx, rx) = channel::<Key>();
            let done = self.done_tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("covers".into())
                .spawn(move || {
                    for key in rx {
                        let out = match engine
                            .open(&key.path, None)
                            .ok()
                            .and_then(|doc| doc.render_page(0, COVER_PX, 0).ok())
                        {
                            Some(r) => Done::Ok(key, r.width, r.height, r.rgba),
                            None => Done::Failed(key),
                        };
                        if done.send(out).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                })
                .ok()?;
            self.work = Some(tx);
        }
        self.work.as_ref()
    }

    /// The cover for `path`, if rendered; queues it otherwise. Only call this
    /// for tiles that are actually visible.
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> Option<&Cover> {
        let key = Key::of(path)?;
        if self.map.contains_key(&key) {
            return self.map.get(&key);
        }
        if !self.pending.contains(&key) {
            if let Some(tx) = self.worker(ctx) {
                if tx.send(key.clone()).is_ok() {
                    self.pending.insert(key);
                }
            }
        }
        None
    }

    /// Turns finished renders into textures. Call once per frame.
    pub fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.done_rx.try_recv() {
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
