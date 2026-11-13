//! First-page covers for the Library shelves.
//!
//! Three things keep a shelf of hundreds of PDFs smooth:
//!
//! * **A disk cache.** A cover is rendered once and saved as a small PNG under
//!   the user cache directory, keyed by the file's path, size and modification
//!   time; later launches just read it back. A changed file gets a new key.
//! * **A background worker with two queues.** Covers for tiles on screen
//!   jump the queue; *preparing* a whole folder (making sure every cover is
//!   on disk) runs behind them. The UI thread never touches PDFium for covers.
//! * **Gentle GPU use.** Finished covers are uploaded a few per frame, not all
//!   at once, and textures not drawn for a while are dropped (they come back
//!   from disk in a few milliseconds when scrolled to again).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use eframe::egui;
use sha1::{Digest, Sha1};

use crate::pdf::PdfEngine;

/// Cover width in pixels. Tiles are 110–180 logical px wide.
pub const COVER_PX: i32 = 240;
/// Uploads to the GPU per frame at most, and the time they may take; the rest
/// wait for the next frame. Keeps even a 165 Hz frame (6 ms) comfortable.
const UPLOADS_PER_FRAME: usize = 6;
const UPLOAD_BUDGET: std::time::Duration = std::time::Duration::from_micros(1500);
/// Covers waiting to load; older requests (tiles scrolled past) are dropped.
const MAX_QUEUED_LOADS: usize = 48;
/// Textures kept in GPU memory before the least recently drawn are dropped.
const MAX_TEXTURES: usize = 240;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Key {
    path: PathBuf,
    len: u64,
    modified: u64,
}

impl Key {
    fn of(path: &Path) -> Option<Key> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(Key { path: path.to_path_buf(), len: meta.len(), modified })
    }

    fn cache_file(&self) -> PathBuf {
        let mut h = Sha1::new();
        h.update(self.path.to_string_lossy().as_bytes());
        h.update(self.len.to_le_bytes());
        h.update(self.modified.to_le_bytes());
        let digest = h.finalize();
        let name: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        cache_dir().join(format!("{name}.png"))
    }
}

/// Where covers are kept: `$XDG_CACHE_HOME/paperview/covers` on Linux,
/// `%LOCALAPPDATA%\PaperView\covers` on Windows.
pub fn cache_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(|b| PathBuf::from(b).join("PaperView"))
        .unwrap_or_else(|| std::env::temp_dir().join("PaperView"));
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("paperview");
    base.join("covers")
}

/// (files, bytes) in the cover cache.
pub fn cache_stats() -> (usize, u64) {
    std::fs::read_dir(cache_dir())
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .fold((0, 0), |(n, b), m| (n + 1, b + m.len()))
        })
        .unwrap_or((0, 0))
}

pub fn clear_cache() {
    if let Ok(dir) = std::fs::read_dir(cache_dir()) {
        for e in dir.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

// ── PNG ──────────────────────────────────────────────────────────────────────

fn save_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Write beside, then rename: a half-written cover must never be read.
    let tmp = path.with_extension("tmp");
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn load_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width as usize, info.height as usize);
    match info.color_type {
        png::ColorType::Rgba => Some((w, h, buf)),
        png::ColorType::Rgb => {
            let rgba = buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
            Some((w, h, rgba))
        }
        _ => None,
    }
}

// ── Worker ───────────────────────────────────────────────────────────────────

enum Job {
    /// A tile on screen wants pixels.
    Load(Key),
    /// Make sure this cover is on disk; no pixels needed now.
    Prepare(Key),
}

enum Done {
    /// Ready for the GPU: converted to egui's format on the worker thread.
    Pixels(Key, egui::ColorImage),
    /// On disk (or known to be unreadable); `ok` says which.
    Prepared(Key, bool),
    Failed(Key),
    /// A background stat: the file's key, or None if it's gone.
    Stat(PathBuf, Option<Key>),
}

#[derive(Default)]
struct Queues {
    /// Most recent first: the tile just scrolled to matters most.
    load: VecDeque<Key>,
    prepare: VecDeque<Key>,
}

struct Shared {
    queues: Mutex<Queues>,
    wake: Condvar,
}

/// Renders page one, or reads it back from the cache.
fn produce(engine: PdfEngine, key: &Key, want_pixels: bool) -> Option<(usize, usize, Vec<u8>)> {
    let file = key.cache_file();
    if file.exists() {
        if !want_pixels {
            return Some((0, 0, Vec::new()));
        }
        if let Some(px) = load_png(&file) {
            return Some(px);
        }
    }
    // Books: their cover image (or a comic's first page), scaled to size.
    if crate::ebook::is_ebook(&key.path) {
        let bytes = crate::ebook::cover(&key.path)?;
        let img = image::load_from_memory(&bytes).ok()?;
        let h = (img.height() as u64 * COVER_PX as u64 / img.width().max(1) as u64).max(1) as u32;
        let rgba = img.resize_exact(COVER_PX as u32, h, image::imageops::FilterType::Triangle).to_rgba8();
        let (w, h) = (rgba.width() as usize, rgba.height() as usize);
        let px = rgba.into_raw();
        let _ = save_png(&file, w, h, &px);
        return Some((w, h, px));
    }
    let doc = engine.open(&key.path, None).ok()?;
    let r = doc.render_page(0, COVER_PX, 0).ok()?;
    let _ = save_png(&file, r.width, r.height, &r.rgba);
    Some((r.width, r.height, r.rgba))
}

fn worker(engine: PdfEngine, shared: Arc<Shared>, done: std::sync::mpsc::Sender<Done>, ctx: egui::Context) {
    loop {
        let job = {
            let mut q = shared.queues.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(k) = q.load.pop_front() {
                    break Job::Load(k);
                }
                if let Some(k) = q.prepare.pop_front() {
                    break Job::Prepare(k);
                }
                q = shared.wake.wait(q).unwrap_or_else(|p| p.into_inner());
            }
        };
        let out = match job {
            Job::Load(key) => match produce(engine, &key, true) {
                Some((w, h, px)) => Done::Pixels(key, egui::ColorImage::from_rgba_unmultiplied([w, h], &px)),
                None => Done::Failed(key),
            },
            Job::Prepare(key) => {
                let ok = produce(engine, &key, false).is_some();
                Done::Prepared(key, ok)
            }
        };
        if done.send(out).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

// ── The UI side ──────────────────────────────────────────────────────────────

pub enum Cover {
    Ready(egui::TextureHandle),
    /// Unreadable, password-protected, or missing — the tile shows its name.
    Unavailable,
}

struct Entry {
    cover: Cover,
    last_used: u64,
}

pub struct Covers {
    engine: Option<PdfEngine>,
    shared: Option<Arc<Shared>>,
    done_tx: std::sync::mpsc::Sender<Done>,
    done_rx: std::sync::mpsc::Receiver<Done>,
    textures: HashMap<Key, Entry>,
    /// Waiting for pixels.
    loading: HashSet<Key>,
    /// Finished renders not yet uploaded to the GPU.
    pending_upload: VecDeque<(Key, egui::ColorImage)>,
    /// Paths known to be on disk (or unreadable), for preparing folders.
    prepared: HashSet<PathBuf>,
    preparing: HashSet<PathBuf>,
    /// What's known about each file (present with its key, or missing) and
    /// when it was learnt. Only ever filled from other threads — the folder
    /// scan, or the stat thread — because on a busy USB hard disk a single
    /// stat on the UI thread can freeze the window for a tenth of a second.
    facts: HashMap<PathBuf, (Option<Key>, std::time::Instant)>,
    stat_pending: HashSet<PathBuf>,
    stat_tx: Option<std::sync::mpsc::Sender<PathBuf>>,
    frame: u64,
}

/// How long a file fact is trusted before it's quietly checked again.
const FACT_TTL: std::time::Duration = std::time::Duration::from_secs(10);

impl Covers {
    pub fn new(engine: Option<PdfEngine>) -> Self {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        Self {
            engine,
            shared: None,
            done_tx,
            done_rx,
            textures: HashMap::new(),
            loading: HashSet::new(),
            pending_upload: VecDeque::new(),
            prepared: HashSet::new(),
            preparing: HashSet::new(),
            facts: HashMap::new(),
            stat_pending: HashSet::new(),
            stat_tx: None,
            frame: 0,
        }
    }

    fn shared(&mut self, ctx: &egui::Context) -> Option<Arc<Shared>> {
        if self.shared.is_none() {
            let engine = self.engine?;
            let shared = Arc::new(Shared { queues: Mutex::default(), wake: Condvar::new() });
            let (s, tx, c) = (shared.clone(), self.done_tx.clone(), ctx.clone());
            std::thread::Builder::new()
                .name("covers".into())
                .spawn(move || worker(engine, s, tx, c))
                .ok()?;
            self.shared = Some(shared);
        }
        self.shared.clone()
    }

    /// Records what a folder scan already learnt, so nothing has to stat it.
    pub fn seed(&mut self, path: &Path, len: u64, modified: u64) {
        let key = Key { path: path.to_path_buf(), len, modified };
        self.facts.insert(path.to_path_buf(), (Some(key), std::time::Instant::now()));
    }

    /// Asks the stat thread about a file.
    fn stat(&mut self, ctx: &egui::Context, path: &Path) {
        if !self.stat_pending.insert(path.to_path_buf()) {
            return;
        }
        if self.stat_tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
            let done = self.done_tx.clone();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new().name("stat".into()).spawn(move || {
                for path in rx {
                    let key = Key::of(&path);
                    if done.send(Done::Stat(path, key)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
            if spawned.is_err() {
                return;
            }
            self.stat_tx = Some(tx);
        }
        if let Some(tx) = &self.stat_tx {
            let _ = tx.send(path.to_path_buf());
        }
    }

    /// The file's key from what's known; checks again in the background when
    /// unknown or stale. Never touches the disk itself.
    fn key(&mut self, ctx: &egui::Context, path: &Path) -> Option<Key> {
        match self.facts.get(path) {
            Some((fact, at)) => {
                let fact = fact.clone();
                if at.elapsed() > FACT_TTL {
                    self.stat(ctx, path);
                }
                fact
            }
            None => {
                self.stat(ctx, path);
                None
            }
        }
    }

    /// Whether the file exists, as far as is known. Unknown counts as yes —
    /// the answer arrives from the background a moment later.
    pub fn exists(&mut self, ctx: &egui::Context, path: &Path) -> bool {
        match self.facts.get(path) {
            Some((fact, at)) => {
                let present = fact.is_some();
                if at.elapsed() > FACT_TTL {
                    self.stat(ctx, path);
                }
                present
            }
            None => {
                self.stat(ctx, path);
                true
            }
        }
    }

    /// The cover for a tile that is on screen; asks for it if it isn't loaded.
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> Option<&Cover> {
        let key = self.key(ctx, path)?;
        if self.textures.contains_key(&key) {
            let frame = self.frame;
            let e = self.textures.get_mut(&key)?;
            e.last_used = frame;
            return Some(&e.cover);
        }
        if self.loading.insert(key.clone()) {
            if let Some(shared) = self.shared(ctx) {
                let mut q = shared.queues.lock().unwrap_or_else(|p| p.into_inner());
                q.load.push_front(key);
                // Scrolling fast queues tiles that are long gone; keep only
                // the newest few and let the rest ask again if they return.
                while q.load.len() > MAX_QUEUED_LOADS {
                    if let Some(old) = q.load.pop_back() {
                        self.loading.remove(&old);
                    }
                }
                shared.wake.notify_one();
            }
        }
        None
    }

    /// Loads a cover that is about to scroll into view, so it's on the GPU by
    /// the time it's drawn instead of arriving in a burst with its neighbours.
    pub fn warm(&mut self, ctx: &egui::Context, path: &Path) {
        let _ = self.get(ctx, path);
    }

    /// Makes sure every cover for `paths` is on disk, in the background.
    /// Returns how many are done so far.
    pub fn prepare(&mut self, ctx: &egui::Context, paths: &[PathBuf]) -> usize {
        let mut fresh = Vec::new();
        let mut done = 0;
        for path in paths {
            if self.prepared.contains(path) {
                done += 1;
            } else if !self.preparing.contains(path) {
                match self.facts.get(path).map(|(k, _)| k.clone()) {
                    Some(Some(k)) => {
                        fresh.push(k);
                        self.preparing.insert(path.clone());
                    }
                    // Gone since the folder was read: nothing to wait for.
                    Some(None) => {
                        self.prepared.insert(path.clone());
                        done += 1;
                    }
                    // Not known yet: ask, and count it next time.
                    None => self.stat(ctx, path),
                }
            }
        }
        if !fresh.is_empty() {
            if let Some(shared) = self.shared(ctx) {
                let mut q = shared.queues.lock().unwrap_or_else(|p| p.into_inner());
                q.prepare.extend(fresh);
                shared.wake.notify_one();
            }
        }
        done
    }

    /// After the disk cache is cleared: covers must be made again.
    pub fn forget_disk(&mut self) {
        self.prepared.clear();
    }

    /// Call once per frame: uploads a few finished covers and drops old ones.
    pub fn pump(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        while let Ok(done) = self.done_rx.try_recv() {
            match done {
                Done::Pixels(key, image) => self.pending_upload.push_back((key, image)),
                Done::Prepared(key, _ok) => {
                    self.preparing.remove(&key.path);
                    self.prepared.insert(key.path);
                }
                Done::Stat(path, key) => {
                    self.stat_pending.remove(&path);
                    self.facts.insert(path, (key, std::time::Instant::now()));
                }
                Done::Failed(key) => {
                    self.loading.remove(&key);
                    self.prepared.insert(key.path.clone());
                    self.textures.insert(key, Entry { cover: Cover::Unavailable, last_used: self.frame });
                }
            }
        }
        let started = std::time::Instant::now();
        for _ in 0..UPLOADS_PER_FRAME {
            if started.elapsed() > UPLOAD_BUDGET {
                break;
            }
            let Some((key, image)) = self.pending_upload.pop_front() else { break };
            let tex = ctx.load_texture(
                format!("cover-{}", key.path.display()),
                image,
                egui::TextureOptions::LINEAR,
            );
            self.loading.remove(&key);
            self.prepared.insert(key.path.clone());
            // A newer version of the same file supersedes the old cover.
            self.textures.retain(|k, _| k.path != key.path);
            self.textures.insert(key, Entry { cover: Cover::Ready(tex), last_used: self.frame });
        }
        if !self.pending_upload.is_empty() {
            ctx.request_repaint();
        }
        // Keep GPU memory bounded: drop what hasn't been drawn lately.
        if self.textures.len() > MAX_TEXTURES {
            let mut by_age: Vec<(u64, Key)> =
                self.textures.iter().map(|(k, e)| (e.last_used, k.clone())).collect();
            by_age.sort_by_key(|(t, _)| *t);
            let excess = self.textures.len() - MAX_TEXTURES;
            for (_, k) in by_age.into_iter().take(excess) {
                self.textures.remove(&k);
            }
        }
    }
}
