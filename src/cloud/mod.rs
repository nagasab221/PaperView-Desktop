//! Cloud accounts: places PaperView keeps documents besides this computer.
//!
//! Five kinds share one shape — list a folder, download, upload, delete,
//! rename, make a folder:
//!
//! * [`Kind::Folder`] — any folder: a mounted NAS share (SMB, NFS, sshfs),
//!   a Syncthing or Nextcloud-client folder. Files open in place.
//! * [`Kind::WebDav`] — Nextcloud, ownCloud, Synology, QNAP, TrueNAS…
//! * [`Kind::S3`] — anything S3-compatible: Cloudflare R2, Wasabi, AWS,
//!   MinIO, iDrive e2, Backblaze's S3 endpoint.
//! * [`Kind::B2`] — Backblaze B2's native API (what PaperView started with).
//! * [`Kind::Ssh`] — SFTP through the system's `ssh`, so ~/.ssh/config
//!   aliases and keys just work and no password is stored.
//!
//! Each operation runs on its own thread: connect → do the op → send
//! [`CloudEvent`]s back over a channel and poke egui awake. Paths are
//! relative to the account's top folder and use `/`, e.g. `Math/Calc.pdf`.

mod b2;
mod folder;
mod http;
mod s3;
pub mod secrets;
mod sftp;
mod webdav;
mod xml;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Folder,
    WebDav,
    S3,
    B2,
    Ssh,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Folder, Kind::WebDav, Kind::S3, Kind::B2, Kind::Ssh];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Folder => "Folder",
            Kind::WebDav => "WebDAV",
            Kind::S3 => "S3",
            Kind::B2 => "Backblaze B2",
            Kind::Ssh => "SSH",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Kind::Folder => "🖴",
            Kind::WebDav => "🌐",
            Kind::S3 => "🗄",
            Kind::B2 => "☁",
            Kind::Ssh => "💻",
        }
    }

    /// Whether the account needs a password or secret key.
    pub fn has_secret(self) -> bool {
        matches!(self, Kind::WebDav | Kind::S3 | Kind::B2)
    }
}

/// One saved account. Which fields matter depends on the kind; the secret
/// (password / secret key / application key) lives in the system keyring,
/// not here — see [`secrets`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Stable id: the keyring entry and the download folder are named by it.
    pub id: String,
    /// What the reader calls it ("NAS", "Laptop").
    pub name: String,
    pub kind: Kind,
    /// Folder: the folder. WebDAV: the server URL, including its path.
    /// S3: the endpoint URL. SSH: a host name or ~/.ssh/config alias.
    #[serde(default)]
    pub address: String,
    /// WebDAV user name (optional), S3 access key ID, B2 key ID, SSH user
    /// (optional).
    #[serde(default)]
    pub user: String,
    /// S3 / B2 bucket.
    #[serde(default)]
    pub bucket: String,
    /// S3 region ("auto" for R2; empty means us-east-1).
    #[serde(default)]
    pub region: String,
    /// Folder inside the bucket (S3, B2) or on the server (SSH).
    #[serde(default)]
    pub folder: String,
    /// SSH port, when not 22 and not set in ~/.ssh/config.
    #[serde(default)]
    pub port: String,
    /// Accept a self-signed HTTPS certificate (WebDAV / S3 on a NAS).
    #[serde(default)]
    pub insecure_tls: bool,
    /// The secret, kept here only when no system keyring was available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plain_secret: Option<String>,
}

impl Account {
    /// Enough filled in to try connecting (the secret is checked on connect).
    pub fn ready(&self) -> bool {
        let filled = |s: &str| !s.trim().is_empty();
        match self.kind {
            Kind::Folder => filled(&self.address),
            Kind::WebDav => filled(&self.address),
            Kind::S3 => filled(&self.address) && filled(&self.user) && filled(&self.bucket),
            Kind::B2 => filled(&self.user) && filled(&self.bucket),
            Kind::Ssh => filled(&self.address),
        }
    }

    /// A short description for lists: "SSH · nagasab:PaperView".
    pub fn summary(&self) -> String {
        let where_ = match self.kind {
            Kind::Folder => self.address.trim().to_string(),
            Kind::WebDav => self.address.trim().to_string(),
            Kind::S3 => {
                let host = self
                    .address
                    .trim()
                    .trim_start_matches("https://")
                    .trim_start_matches("http://");
                join_display(&format!("{host}/{}", self.bucket.trim()), &self.folder)
            }
            Kind::B2 => join_display(self.bucket.trim(), &self.folder),
            Kind::Ssh => {
                let host = match self.user.trim() {
                    "" => self.address.trim().to_string(),
                    u => format!("{u}@{}", self.address.trim()),
                };
                format!("{host}:{}", ssh_folder(&self.folder))
            }
        };
        format!("{} · {where_}", self.kind.label())
    }

    /// For a Folder account, the folder itself: files there open in place.
    pub fn local_root(&self) -> Option<PathBuf> {
        (self.kind == Kind::Folder).then(|| expand_home(self.address.trim()))
    }

    /// Whether `path` is a file inside this account's own folder (Folder
    /// accounts only) — such a file is already "in the cloud".
    pub fn contains_local(&self, path: &Path) -> bool {
        self.local_root()
            .is_some_and(|root| !root.as_os_str().is_empty() && path.starts_with(root))
    }

    /// Trims stray whitespace pasted around the details.
    pub fn trimmed(mut self) -> Account {
        for f in [
            &mut self.name,
            &mut self.address,
            &mut self.user,
            &mut self.bucket,
            &mut self.region,
            &mut self.folder,
            &mut self.port,
        ] {
            *f = f.trim().to_string();
        }
        self.folder = self.folder.trim_matches('/').to_string();
        if self.kind == Kind::Ssh && self.folder.is_empty() {
            self.folder = DEFAULT_SSH_FOLDER.to_string();
        }
        self
    }
}

/// Where an SSH account keeps documents when no folder is given.
pub const DEFAULT_SSH_FOLDER: &str = "PaperView";

fn ssh_folder(folder: &str) -> String {
    match folder.trim().trim_matches('/') {
        "" => DEFAULT_SSH_FOLDER.to_string(),
        f => f.to_string(),
    }
}

fn join_display(a: &str, b: &str) -> String {
    match b.trim().trim_matches('/') {
        "" => a.to_string(),
        b => format!("{a}/{b}"),
    }
}

/// `~` and `~/…` mean the home folder.
pub fn expand_home(s: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if s == "~" {
        return home().unwrap_or_default();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(h) = home() {
            return h.join(rest);
        }
    }
    PathBuf::from(s)
}

/// A fresh account id: time-based, unique enough for one person's accounts.
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{:x}", t & 0xffff_ffff_ffff, N.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Debug)]
pub struct CloudFile {
    /// Path from the account's top folder, e.g. `Math/Calc.pdf`.
    pub path: String,
    /// The last part of the path.
    pub name: String,
    pub size: u64,
    /// Upload / modification time, unix milliseconds (0 when unknown).
    pub modified_ms: i64,
    /// B2 file id (needed to delete or copy that version); empty elsewhere.
    pub id: String,
    pub is_dir: bool,
}

impl CloudFile {
    fn file(path: String, size: u64, modified_ms: i64, id: String) -> CloudFile {
        CloudFile {
            name: last_part(&path).to_string(),
            path,
            size,
            modified_ms,
            id,
            is_dir: false,
        }
    }

    fn dir(path: String) -> CloudFile {
        let path = path.trim_end_matches('/').to_string();
        CloudFile {
            name: last_part(&path).to_string(),
            path,
            size: 0,
            modified_ms: 0,
            id: String::new(),
            is_dir: true,
        }
    }
}

pub enum CloudEvent {
    /// Progress note (op still running).
    Status(String),
    /// Terminal failure.
    Error(String),
    /// Terminal: a folder listing of `account`.
    Listed {
        account: String,
        dir: String,
        files: Vec<CloudFile>,
    },
    /// Live transfer progress.
    Progress { done: u64, total: u64, bps: f64 },
    /// Terminal: file fetched to this local path.
    Downloaded {
        path: PathBuf,
        bytes: u64,
        secs: f64,
        open: bool,
    },
    /// Terminal: upload finished.
    UploadDone { name: String, bytes: u64, secs: f64 },
    /// Terminal: files deleted.
    Deleted(usize),
    /// Terminal: a file now goes by a new name; its downloaded copy (if any)
    /// moved from `from_local` to `to_local`.
    Renamed {
        to: String,
        from_local: PathBuf,
        to_local: PathBuf,
    },
    /// Terminal: a folder was made.
    FolderMade(String),
    /// Not an op: the keyring couldn't take this account's secret, so the
    /// app should keep it in its settings file instead.
    SecretNotStored {
        account: String,
        secret: String,
        why: String,
    },
}

impl CloudEvent {
    /// Whether this event ends the running op.
    pub fn is_terminal(&self) -> bool {
        !matches!(
            self,
            CloudEvent::Status(_) | CloudEvent::Progress { .. } | CloudEvent::SecretNotStored { .. }
        )
    }
}

pub enum CloudOp {
    /// List a folder ("" is the top).
    List(String),
    /// A local file, and the path to save it under.
    Upload(PathBuf, String),
    Download {
        path: String,
        size: u64,
        open: bool,
    },
    /// (path, file id) pairs to delete.
    Delete(Vec<(String, String)>),
    /// Rename a file (B2 and S3 copy it server-side, then delete the old one).
    Rename {
        from: String,
        id: String,
        to: String,
    },
    MakeFolder(String),
}

/// What every kind of account can do. Paths are relative to the account's
/// top folder.
trait Backend {
    fn list(&mut self, dir: &str) -> Result<Vec<CloudFile>, String>;
    /// Streams a file into `out`; `progress` gets the bytes so far.
    fn get(
        &mut self,
        path: &str,
        size: u64,
        out: &mut dyn Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, String>;
    fn put(&mut self, path: &str, data: &[u8], progress: &mut dyn FnMut(u64))
        -> Result<(), String>;
    fn delete(&mut self, path: &str, id: &str) -> Result<(), String>;
    fn rename(&mut self, from: &str, id: &str, to: &str) -> Result<(), String>;
    fn mkdir(&mut self, path: &str) -> Result<(), String>;
}

fn connect(acct: &Account, secret: &str) -> Result<Box<dyn Backend>, String> {
    Ok(match acct.kind {
        Kind::Folder => Box::new(folder::Folder::connect(acct)?),
        Kind::WebDav => Box::new(webdav::WebDav::connect(acct, secret)?),
        Kind::S3 => Box::new(s3::S3::connect(acct, secret)?),
        Kind::B2 => Box::new(b2::B2::connect(acct, secret)?),
        Kind::Ssh => Box::new(sftp::Sftp::connect(acct)?),
    })
}

/// Where downloaded documents live locally: one folder per account, under
/// the XDG data directory (`~/.local/share/paperview/cloud`) — not the cache
/// or `/tmp`, because a downloaded document is something the reader expects
/// to still be there. Windows: `%LOCALAPPDATA%\PaperView\cloud`.
pub fn cache_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join("PaperView").join("cloud")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir);
        base.join("paperview").join("cloud")
    }
}

/// An account's download folder.
pub fn account_cache(account: &str) -> PathBuf {
    cache_dir().join(safe_part(account))
}

/// Local path a file of `account` downloads to; subfolders are kept.
pub fn local_path_for(account: &str, path: &str) -> PathBuf {
    let mut out = account_cache(account);
    for part in path.split('/').filter(|p| !p.is_empty()) {
        out.push(safe_part(part));
    }
    out
}

/// The file name a cloud file of this name downloads to.
pub fn local_name(name: &str) -> String {
    safe_part(name)
}

/// One path component, safe on any filesystem: no separators, no `..`.
fn safe_part(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    match flat.as_str() {
        "" | "." | ".." => "_".to_string(),
        _ => flat,
    }
}

/// Joins a folder and a name with `/` ("" is the top folder).
pub fn join(dir: &str, name: &str) -> String {
    let dir = dir.trim_matches('/');
    let name = name.trim_matches('/');
    match (dir.is_empty(), name.is_empty()) {
        (true, _) => name.to_string(),
        (_, true) => dir.to_string(),
        _ => format!("{dir}/{name}"),
    }
}

/// The folder part of a path ("" for the top).
pub fn parent(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("")
}

pub fn last_part(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    p.rsplit_once('/').map(|(_, n)| n).unwrap_or(p)
}

/// Documents PaperView can open, by extension.
pub fn is_document(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some((_, ext)) = lower.rsplit_once('.') else {
        return false;
    };
    ext == "pdf" || crate::ebook::EXTENSIONS.contains(&ext)
}

/// What the listing shows: folders and documents, no hidden files.
fn keep(f: &CloudFile) -> bool {
    !f.name.is_empty() && !f.name.starts_with('.') && (f.is_dir || is_document(&f.name))
}

/// Folders first, then files, each by name ignoring case.
fn sort(files: &mut [CloudFile]) {
    files.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Fire-and-forget worker for `acct`. Events arrive on `tx`; `repaint` is
/// poked after each send so egui wakes up.
pub fn spawn(acct: Account, op: CloudOp, tx: Sender<CloudEvent>, repaint: eframe::egui::Context) {
    std::thread::spawn(move || {
        let send = |e: CloudEvent| {
            let _ = tx.send(e);
            repaint.request_repaint();
        };
        if acct.kind != Kind::Folder {
            send(CloudEvent::Status("Connecting…".into()));
        }
        let secret = if acct.kind.has_secret() {
            match secrets::get(&acct) {
                Ok(s) => s,
                Err(e) => return send(CloudEvent::Error(e)),
            }
        } else {
            String::new()
        };
        let mut backend = match connect(&acct, &secret) {
            Ok(b) => b,
            Err(e) => return send(CloudEvent::Error(e)),
        };
        send(run(&mut *backend, &acct, op, &send));
    });
}

/// Throttled progress: at most every 150 ms, with the speed since the last.
struct Meter<'a> {
    send: &'a dyn Fn(CloudEvent),
    total: u64,
    started: std::time::Instant,
    last: std::time::Instant,
    last_bytes: u64,
}

impl<'a> Meter<'a> {
    fn new(send: &'a dyn Fn(CloudEvent), total: u64) -> Self {
        let now = std::time::Instant::now();
        Meter {
            send,
            total,
            started: now,
            last: now,
            last_bytes: 0,
        }
    }

    fn tick(&mut self, done: u64) {
        let since = self.last.elapsed();
        if since.as_millis() < 150 {
            return;
        }
        let bps = done.saturating_sub(self.last_bytes) as f64 / since.as_secs_f64().max(1e-6);
        (self.send)(CloudEvent::Progress {
            done,
            total: self.total.max(done),
            bps,
        });
        self.last = std::time::Instant::now();
        self.last_bytes = done;
    }

    fn secs(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

fn run(b: &mut dyn Backend, acct: &Account, op: CloudOp, send: &dyn Fn(CloudEvent)) -> CloudEvent {
    let result = match op {
        CloudOp::List(dir) => b.list(&dir).map(|mut files| {
            files.retain(keep);
            sort(&mut files);
            CloudEvent::Listed {
                account: acct.id.clone(),
                dir,
                files,
            }
        }),
        CloudOp::Upload(local, path) => {
            send(CloudEvent::Status(format!("Uploading {}…", last_part(&path))));
            upload(b, &local, &path, send)
        }
        CloudOp::Download { path, size, open } => {
            send(CloudEvent::Status(format!("Downloading {}…", last_part(&path))));
            download(b, acct, &path, size, send).map(|(dest, bytes, secs)| CloudEvent::Downloaded {
                path: dest,
                bytes,
                secs,
                open,
            })
        }
        CloudOp::Delete(items) => {
            send(CloudEvent::Status("Deleting…".into()));
            items
                .iter()
                .try_for_each(|(path, id)| b.delete(path, id))
                .map(|()| CloudEvent::Deleted(items.len()))
        }
        CloudOp::Rename { from, id, to } => {
            send(CloudEvent::Status(format!("Renaming to {}…", last_part(&to))));
            b.rename(&from, &id, &to).map(|()| {
                // A downloaded copy follows the new name.
                let from_local = local_path_for(&acct.id, &from);
                let to_local = local_path_for(&acct.id, &to);
                if from_local.exists() {
                    if let Some(dir) = to_local.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::rename(&from_local, &to_local);
                }
                CloudEvent::Renamed {
                    to,
                    from_local,
                    to_local,
                }
            })
        }
        CloudOp::MakeFolder(path) => b.mkdir(&path).map(|()| CloudEvent::FolderMade(path)),
    };
    result.unwrap_or_else(CloudEvent::Error)
}

fn upload(
    b: &mut dyn Backend,
    local: &Path,
    path: &str,
    send: &dyn Fn(CloudEvent),
) -> Result<CloudEvent, String> {
    if last_part(path).trim().is_empty() {
        return Err("File has no name".into());
    }
    let bytes =
        std::fs::read(local).map_err(|e| format!("Could not read {}: {e}", local.display()))?;
    let mut meter = Meter::new(send, bytes.len() as u64);
    b.put(path, &bytes, &mut |done| meter.tick(done))?;
    Ok(CloudEvent::UploadDone {
        name: last_part(path).to_string(),
        bytes: bytes.len() as u64,
        secs: meter.secs(),
    })
}

/// Downloads into a hidden part-file first, so a broken transfer never
/// shows up as a downloaded document.
fn download(
    b: &mut dyn Backend,
    acct: &Account,
    path: &str,
    size: u64,
    send: &dyn Fn(CloudEvent),
) -> Result<(PathBuf, u64, f64), String> {
    let dest = local_path_for(&acct.id, path);
    let dir = dest.parent().map(Path::to_path_buf).unwrap_or_else(cache_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let part = dir.join(format!(
        ".{}.part",
        dest.file_name().unwrap_or_default().to_string_lossy()
    ));
    let file = std::fs::File::create(&part).map_err(|e| format!("Could not save download: {e}"))?;
    let mut out = std::io::BufWriter::with_capacity(256 * 1024, file);
    let mut meter = Meter::new(send, size);
    let got = b.get(path, size, &mut out, &mut |done| meter.tick(done));
    let flushed = out
        .flush()
        .map_err(|e| format!("Could not save download: {e}"));
    match got.and_then(|n| flushed.map(|()| n)) {
        Ok(n) => {
            std::fs::rename(&part, &dest).map_err(|e| format!("Could not save download: {e}"))?;
            Ok((dest, n, meter.secs()))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Downloaded documents of `account` in `dir`: (path, name, size), newest
/// first. Read straight off the disk, so they stay listed with no network.
pub fn local_docs(account: &str, dir: &str) -> Vec<(PathBuf, String, u64)> {
    let mut docs: Vec<((PathBuf, String, u64), std::time::SystemTime)> =
        std::fs::read_dir(local_path_for(account, dir))
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let meta = e.metadata().ok()?;
                        let name = e.file_name().to_string_lossy().into_owned();
                        if !meta.is_file() || meta.len() == 0 || name.starts_with('.') {
                            return None;
                        }
                        Some((
                            (e.path(), name, meta.len()),
                            meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
    docs.sort_by_key(|d| std::cmp::Reverse(d.1));
    docs.into_iter().map(|(d, _)| d).collect()
}

/// (file count, total bytes) of every download, all accounts.
pub fn cache_stats() -> (usize, u64) {
    fn walk(dir: &Path, acc: &mut (usize, u64)) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                walk(&entry.path(), acc);
            } else if meta.is_file() {
                acc.0 += 1;
                acc.1 += meta.len();
            }
        }
    }
    let mut acc = (0, 0);
    walk(&cache_dir(), &mut acc);
    acc
}

/// Moves downloads that predate accounts (loose files in the cache folder)
/// into `account`'s folder; returns (old, new) paths so references follow.
pub fn adopt_loose_downloads(account: &str) -> Vec<(PathBuf, PathBuf)> {
    let mut moved = Vec::new();
    let Ok(rd) = std::fs::read_dir(cache_dir()) else {
        return moved;
    };
    let target = account_cache(account);
    for entry in rd.flatten() {
        if !entry.metadata().is_ok_and(|m| m.is_file()) {
            continue;
        }
        if std::fs::create_dir_all(&target).is_err() {
            break;
        }
        let to = target.join(entry.file_name());
        if std::fs::rename(entry.path(), &to).is_ok() {
            moved.push((entry.path(), to));
        }
    }
    moved
}

/// Percent-encodes UTF-8 (unreserved characters kept; `/` kept only when
/// `keep_slash`).
pub(crate) fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && b == b'/');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// "12.3 MB"-style size formatting.
pub fn fmt_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.2} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode("plain-name_1.pdf", false), "plain-name_1.pdf");
        assert_eq!(percent_encode("a b.pdf", false), "a%20b.pdf");
        assert_eq!(percent_encode("a/b.pdf", true), "a/b.pdf");
        assert_eq!(percent_encode("a/b.pdf", false), "a%2Fb.pdf");
        assert_eq!(percent_encode("ü.pdf", false), "%C3%BC.pdf");
        assert_eq!(percent_decode("a%20b%C3%BC.pdf"), "a bü.pdf");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%2"), "%2");
    }

    #[test]
    fn size_formatting() {
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(2048), "2 KB");
        assert_eq!(fmt_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn local_paths_keep_folders_but_stay_inside() {
        let p = local_path_for("acct", "Math/sub\\file:x.pdf");
        assert!(p.ends_with("acct/Math/sub_file_x.pdf"));
        let p = local_path_for("acct", "../../etc/passwd");
        assert!(p.starts_with(account_cache("acct")));
        assert!(p.ends_with("_/_/etc/passwd"));
    }

    #[test]
    fn path_helpers() {
        assert_eq!(join("", "a.pdf"), "a.pdf");
        assert_eq!(join("Math/", "a.pdf"), "Math/a.pdf");
        assert_eq!(parent("Math/Calc/a.pdf"), "Math/Calc");
        assert_eq!(parent("a.pdf"), "");
        assert_eq!(last_part("Math/Calc/"), "Calc");
        assert!(is_document("x.PDF") && is_document("b.epub") && !is_document("a.mp4"));
    }

    #[test]
    fn listing_order_and_filter() {
        let mut v = vec![
            CloudFile::file("b.pdf".into(), 1, 0, String::new()),
            CloudFile::file("A.pdf".into(), 1, 0, String::new()),
            CloudFile::file("notes.mp4".into(), 1, 0, String::new()),
            CloudFile::file(".bzEmpty".into(), 0, 0, String::new()),
            CloudFile::dir("zeta/".into()),
        ];
        v.retain(keep);
        sort(&mut v);
        let names: Vec<_> = v.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["zeta", "A.pdf", "b.pdf"]);
    }
}
