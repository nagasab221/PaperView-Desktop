//! Minimal Backblaze B2 client (native JSON API — no OAuth, no request
//! signing) plus background workers so the UI never blocks on the network.
//!
//! Each operation runs on its own thread: authorize → resolve bucket → do the
//! op → send [`CloudEvent`]s back over an mpsc channel and poke egui awake.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use base64::Engine as _;
use serde::Deserialize;
use sha1::{Digest, Sha1};

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct B2Config {
    pub key_id: String,
    pub app_key: String,
    pub bucket: String,
}

impl B2Config {
    pub fn ready(&self) -> bool {
        !self.key_id.trim().is_empty()
            && !self.app_key.trim().is_empty()
            && !self.bucket.trim().is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct CloudFile {
    pub name: String,
    pub size: u64,
    /// Upload time, unix milliseconds.
    pub uploaded_ms: i64,
    /// B2 file id (needed to delete this version).
    pub file_id: String,
}

pub enum CloudEvent {
    /// Progress note (op still running).
    Status(String),
    /// Terminal failure.
    Error(String),
    /// Terminal: listing finished.
    Files(Vec<CloudFile>),
    /// Live transfer progress (downloads).
    Progress { done: u64, total: u64, bps: f64 },
    /// Terminal: file fetched to this local path.
    Downloaded { path: PathBuf, bytes: u64, secs: f64, open: bool },
    /// Terminal: upload finished.
    UploadDone { name: String, bytes: u64, secs: f64 },
    /// Terminal: files deleted.
    Deleted(usize),
    /// Terminal: a file now goes by a new name.
    Renamed { from: String, to: String },
}

pub enum CloudOp {
    List,
    Upload(PathBuf),
    /// (file name, expected size, open after download)
    Download(String, u64, bool),
    /// (file name, file id) pairs to delete.
    Delete(Vec<(String, String)>),
    /// (current name, file id, new name). B2 has no rename: the file is
    /// copied server-side under the new name, then the old version deleted.
    Rename(String, String, String),
}

/// Where downloaded PDFs live locally.
///
/// Windows: `%LOCALAPPDATA%\PaperView\cloud`. Linux: the XDG data directory
/// (`~/.local/share/paperview/cloud`) — not the cache or `/tmp`, because a
/// downloaded document is something the reader expects to still be there.
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

/// Local path a cloud file would download to.
pub fn local_path_for(name: &str) -> PathBuf {
    // We upload flat names, but sanitize anything else defensively.
    let flat: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':') { '_' } else { c })
        .collect();
    cache_dir().join(flat)
}

/// Fire-and-forget worker. Events arrive on `tx`; `repaint` is poked after
/// each send so egui wakes up.
pub fn spawn(cfg: B2Config, op: CloudOp, tx: Sender<CloudEvent>, repaint: eframe::egui::Context) {
    std::thread::spawn(move || {
        let send = |e: CloudEvent| {
            let _ = tx.send(e);
            repaint.request_repaint();
        };
        #[cfg(debug_assertions)]
        if let Some(dir) = std::env::var_os("PAPERVIEW_FAKE_CLOUD").filter(|d| !d.is_empty()) {
            return fake::run(std::path::Path::new(&dir), op, &send);
        }
        send(CloudEvent::Status("Connecting…".into()));
        let session = match connect(&cfg) {
            Ok(s) => s,
            Err(e) => return send(CloudEvent::Error(e)),
        };
        match op {
            CloudOp::List => match list(&session) {
                Ok(files) => send(CloudEvent::Files(files)),
                Err(e) => send(CloudEvent::Error(e)),
            },
            CloudOp::Upload(path) => {
                send(CloudEvent::Status("Uploading…".into()));
                match upload(&session, &path) {
                    Ok((name, bytes, secs)) => send(CloudEvent::UploadDone { name, bytes, secs }),
                    Err(e) => send(CloudEvent::Error(e)),
                }
            }
            CloudOp::Download(name, expected, open) => {
                send(CloudEvent::Status("Downloading…".into()));
                match download(&session, &name, expected, &send) {
                    Ok((path, bytes, secs)) => {
                        send(CloudEvent::Downloaded { path, bytes, secs, open })
                    }
                    Err(e) => send(CloudEvent::Error(e)),
                }
            }
            CloudOp::Delete(items) => {
                send(CloudEvent::Status("Deleting…".into()));
                match delete(&session, &items) {
                    Ok(n) => send(CloudEvent::Deleted(n)),
                    Err(e) => send(CloudEvent::Error(e)),
                }
            }
            CloudOp::Rename(from, id, to) => {
                send(CloudEvent::Status(format!("Renaming to {to}…")));
                match rename(&session, &from, &id, &to) {
                    Ok(()) => send(CloudEvent::Renamed { from, to }),
                    Err(e) => send(CloudEvent::Error(e)),
                }
            }
        }
    });
}

// ── B2 protocol ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AuthResp {
    #[serde(rename = "accountId")]
    account_id: String,
    #[serde(rename = "apiUrl")]
    api_url: String,
    #[serde(rename = "downloadUrl")]
    download_url: String,
    #[serde(rename = "authorizationToken")]
    token: String,
}

#[derive(Deserialize)]
struct BucketsResp {
    buckets: Vec<Bucket>,
}

#[derive(Deserialize)]
struct Bucket {
    #[serde(rename = "bucketId")]
    id: String,
    #[serde(rename = "bucketName")]
    name: String,
}

#[derive(Deserialize)]
struct ListResp {
    files: Vec<FileInfo>,
    #[serde(rename = "nextFileName", default)]
    next_file_name: Option<String>,
}

#[derive(Deserialize)]
struct FileInfo {
    #[serde(rename = "fileName")]
    name: String,
    #[serde(rename = "fileId", default)]
    file_id: String,
    #[serde(rename = "contentLength")]
    size: u64,
    #[serde(rename = "uploadTimestamp")]
    ts: i64,
    #[serde(default)]
    action: String,
}

#[derive(Deserialize)]
struct UploadUrlResp {
    #[serde(rename = "uploadUrl")]
    url: String,
    #[serde(rename = "authorizationToken")]
    token: String,
}

struct Session {
    api_url: String,
    download_url: String,
    token: String,
    bucket_id: String,
    bucket_name: String,
}

/// HTTP agent using the OS TLS stack (Schannel) — ureq's default provider is
/// rustls, which isn't compiled in (its `ring` dep needs a C compiler).
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .into()
}

fn http_err(what: &str, e: ureq::Error) -> String {
    format!("{what}: {e}")
}

fn read_json<T: serde::de::DeserializeOwned>(
    what: &str,
    resp: &mut ureq::http::Response<ureq::Body>,
) -> Result<T, String> {
    let mut text = String::new();
    resp.body_mut()
        .as_reader()
        .read_to_string(&mut text)
        .map_err(|e| format!("{what}: read failed: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{what}: unexpected response: {e}"))
}

fn connect(cfg: &B2Config) -> Result<Session, String> {
    let creds = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:{}", cfg.key_id.trim(), cfg.app_key.trim()));
    let mut resp = agent().get("https://api.backblazeb2.com/b2api/v2/b2_authorize_account")
        .header("Authorization", format!("Basic {creds}"))
        .call()
        .map_err(|e| http_err("Authorization failed (check Key ID / Application Key)", e))?;
    let auth: AuthResp = read_json("Authorize", &mut resp)?;

    let body = serde_json::json!({
        "accountId": auth.account_id,
        "bucketName": cfg.bucket.trim(),
    });
    let mut resp = agent().post(format!("{}/b2api/v2/b2_list_buckets", auth.api_url))
        .header("Authorization", &auth.token)
        .header("Content-Type", "application/json").send(body.to_string())
        .map_err(|e| http_err("Could not list buckets", e))?;
    let buckets: BucketsResp = read_json("List buckets", &mut resp)?;
    let bucket = buckets
        .buckets
        .into_iter()
        .find(|b| b.name == cfg.bucket.trim())
        .ok_or_else(|| {
            format!(
                "Bucket \"{}\" not found (is the application key allowed to access it?)",
                cfg.bucket.trim()
            )
        })?;

    Ok(Session {
        api_url: auth.api_url,
        download_url: auth.download_url,
        token: auth.token,
        bucket_id: bucket.id,
        bucket_name: bucket.name,
    })
}

fn list(s: &Session) -> Result<Vec<CloudFile>, String> {
    // Paginate the full listing so stats (total size, count) are accurate.
    let mut out = Vec::new();
    let mut start: Option<String> = None;
    loop {
        let mut body = serde_json::json!({
            "bucketId": s.bucket_id,
            "maxFileCount": 1000,
        });
        if let Some(sfn) = &start {
            body["startFileName"] = serde_json::json!(sfn);
        }
        let mut resp = agent().post(format!("{}/b2api/v2/b2_list_file_names", s.api_url))
            .header("Authorization", &s.token)
            .header("Content-Type", "application/json").send(body.to_string())
            .map_err(|e| http_err("Could not list files", e))?;
        let listing: ListResp = read_json("List files", &mut resp)?;
        out.extend(
            listing
                .files
                .into_iter()
                .filter(|f| f.action.is_empty() || f.action == "upload")
                .map(|f| CloudFile {
                    name: f.name,
                    size: f.size,
                    uploaded_ms: f.ts,
                    file_id: f.file_id,
                }),
        );
        match listing.next_file_name {
            Some(next) if out.len() < 10_000 => start = Some(next),
            _ => break,
        }
    }
    Ok(out)
}

/// Uploads a file; returns (name, bytes, seconds the transfer itself took).
fn upload(s: &Session, path: &Path) -> Result<(String, u64, f64), String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let sha = hex(&Sha1::digest(&bytes));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("File has no name")?;

    let body = serde_json::json!({ "bucketId": s.bucket_id });
    let mut resp = agent().post(format!("{}/b2api/v2/b2_get_upload_url", s.api_url))
        .header("Authorization", &s.token)
        .header("Content-Type", "application/json").send(body.to_string())
        .map_err(|e| http_err("Could not get upload URL", e))?;
    let up: UploadUrlResp = read_json("Get upload URL", &mut resp)?;

    let started = std::time::Instant::now();
    agent().post(&up.url)
        .header("Authorization", &up.token)
        .header("X-Bz-File-Name", percent_encode(&name, false))
        .header("Content-Type", "application/pdf")
        .header("X-Bz-Content-Sha1", &sha)
        .send(&bytes[..])
        .map_err(|e| http_err("Upload failed", e))?;
    let secs = started.elapsed().as_secs_f64();
    let len = bytes.len() as u64;
    Ok((name, len, secs))
}

/// Downloads a file with live progress via `emit`; returns
/// (local path, bytes, seconds).
fn download(
    s: &Session,
    name: &str,
    expected: u64,
    emit: &dyn Fn(CloudEvent),
) -> Result<(PathBuf, u64, f64), String> {
    let url = format!(
        "{}/file/{}/{}",
        s.download_url,
        s.bucket_name,
        percent_encode(name, true)
    );
    let mut resp = agent().get(&url)
        .header("Authorization", &s.token)
        .call()
        .map_err(|e| http_err("Download failed", e))?;

    let started = std::time::Instant::now();
    let mut last_emit = started;
    let mut last_bytes = 0u64;
    let mut bytes: Vec<u8> = Vec::with_capacity(expected as usize);
    let mut reader = resp.body_mut().as_reader();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download read failed: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if last_emit.elapsed().as_millis() >= 150 {
            let done = bytes.len() as u64;
            let bps =
                (done - last_bytes) as f64 / last_emit.elapsed().as_secs_f64().max(1e-6);
            emit(CloudEvent::Progress {
                done,
                total: expected.max(done),
                bps,
            });
            last_emit = std::time::Instant::now();
            last_bytes = done;
        }
    }
    let secs = started.elapsed().as_secs_f64();

    let dest = local_path_for(name);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not create cache dir: {e}"))?;
    }
    std::fs::write(&dest, &bytes).map_err(|e| format!("Could not save download: {e}"))?;
    let len = bytes.len() as u64;
    Ok((dest, len, secs))
}

/// Permanently deletes the given (name, id) file versions.
fn delete(s: &Session, items: &[(String, String)]) -> Result<usize, String> {
    for (name, id) in items {
        let body = serde_json::json!({ "fileName": name, "fileId": id });
        agent().post(format!("{}/b2api/v2/b2_delete_file_version", s.api_url))
            .header("Authorization", &s.token)
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| http_err(&format!("Could not delete {name}"), e))?;
    }
    Ok(items.len())
}

/// Copies a file to `to` on the server, then deletes the original version.
fn rename(s: &Session, from: &str, id: &str, to: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "sourceFileId": id,
        "fileName": to,
        "metadataDirective": "COPY",
    });
    agent()
        .post(format!("{}/b2api/v2/b2_copy_file", s.api_url))
        .header("Authorization", &s.token)
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| http_err(&format!("Could not copy {from} to {to}"), e))?;
    delete(s, &[(from.to_string(), id.to_string())])?;
    // Keep a downloaded copy under its new name too.
    let old_local = local_path_for(from);
    if old_local.exists() {
        let _ = std::fs::rename(&old_local, local_path_for(to));
    }
    Ok(())
}

/// (file count, total bytes) of the local download cache.
pub fn cache_stats() -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    if let Ok(rd) = std::fs::read_dir(cache_dir()) {
        for entry in rd.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    count += 1;
                    bytes += meta.len();
                }
            }
        }
    }
    (count, bytes)
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Percent-encodes a file name for B2 headers/URLs (UTF-8, unreserved chars
/// kept; `/` kept only when `keep_slash`).
fn percent_encode(s: &str, keep_slash: bool) -> String {
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

/// "12.3 MB"-style size formatting for the Saved tab.
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
        // UTF-8 bytes get encoded individually.
        assert_eq!(percent_encode("ü.pdf", false), "%C3%BC.pdf");
    }

    #[test]
    fn size_formatting() {
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(2048), "2 KB");
        assert_eq!(fmt_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn local_paths_are_flattened() {
        let p = local_path_for("dir/sub\\file:x.pdf");
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, "dir_sub_file_x.pdf");
    }
}

/// A folder standing in for the bucket, for UI tests (debug builds only,
/// `PAPERVIEW_FAKE_CLOUD=<dir>`), so tests never touch a real bucket.
#[cfg(debug_assertions)]
mod fake {
    use super::*;

    pub fn run(dir: &Path, op: CloudOp, send: &dyn Fn(CloudEvent)) {
        let _ = std::fs::create_dir_all(dir);
        let list = || -> Vec<CloudFile> {
            let mut files: Vec<CloudFile> = std::fs::read_dir(dir)
                .map(|d| {
                    d.flatten()
                        .filter_map(|e| {
                            let meta = e.metadata().ok()?;
                            let name = e.file_name().to_string_lossy().into_owned();
                            Some(CloudFile {
                                file_id: format!("id-{name}"),
                                size: meta.len(),
                                uploaded_ms: 1_763_000_000_000,
                                name,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            files.sort_by(|a, b| a.name.cmp(&b.name));
            files
        };
        match op {
            CloudOp::List => send(CloudEvent::Files(list())),
            CloudOp::Upload(path) => {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let n = std::fs::copy(&path, dir.join(&name)).unwrap_or(0);
                send(CloudEvent::UploadDone { name, bytes: n, secs: 0.1 })
            }
            CloudOp::Download(name, _, open) => {
                let dest = local_path_for(&name);
                let _ = std::fs::create_dir_all(cache_dir());
                match std::fs::copy(dir.join(&name), &dest) {
                    Ok(n) => send(CloudEvent::Downloaded { path: dest, bytes: n, secs: 0.1, open }),
                    Err(e) => send(CloudEvent::Error(format!("fake download: {e}"))),
                }
            }
            CloudOp::Delete(items) => {
                for (name, _) in &items {
                    let _ = std::fs::remove_file(dir.join(name));
                }
                send(CloudEvent::Deleted(items.len()))
            }
            CloudOp::Rename(from, _, to) => {
                let _ = std::fs::rename(dir.join(&from), dir.join(&to));
                let old_local = local_path_for(&from);
                if old_local.exists() {
                    let _ = std::fs::rename(&old_local, local_path_for(&to));
                }
                send(CloudEvent::Renamed { from, to })
            }
        }
    }
}
