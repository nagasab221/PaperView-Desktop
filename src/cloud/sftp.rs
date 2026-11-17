//! SSH accounts: SFTP (protocol version 3) spoken over the system's own
//! `ssh -s host sftp`, so ~/.ssh/config aliases, keys and the agent all
//! work, and PaperView never sees or stores a password. Any machine or NAS
//! with OpenSSH serves this.
//!
//! Reads and writes are pipelined (many requests in flight), so a far-away
//! server is still quick.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Account, Backend, CloudFile};

const INIT: u8 = 1;
const VERSION: u8 = 2;
const OPEN: u8 = 3;
const CLOSE: u8 = 4;
const READ: u8 = 5;
const WRITE: u8 = 6;
const OPENDIR: u8 = 11;
const READDIR: u8 = 12;
const REMOVE: u8 = 13;
const MKDIR: u8 = 14;
const STAT: u8 = 17;
const RENAME: u8 = 18;
const STATUS: u8 = 101;
const HANDLE: u8 = 102;
const DATA: u8 = 103;
const NAME: u8 = 104;
const ATTRS: u8 = 105;

const FX_OK: u32 = 0;
const FX_EOF: u32 = 1;
const FX_NO_SUCH_FILE: u32 = 2;

const READ_FLAG: u32 = 0x01;
const WRITE_FLAG: u32 = 0x02;
const CREAT: u32 = 0x08;
const TRUNC: u32 = 0x10;

const CHUNK: u32 = 32 * 1024;
/// Requests in flight at once: 2 MB, enough to fill a long, fast link.
const WINDOW: usize = 64;

pub struct Sftp {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Result<Vec<u8>, String>>,
    stderr: Arc<Mutex<String>>,
    next_id: u32,
    /// The account's folder on the server: relative to the home folder, or
    /// absolute.
    base: String,
}

/// A reply being read.
struct Buf {
    data: Vec<u8>,
    pos: usize,
}

impl Buf {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        if self.pos + n > self.data.len() {
            return Err("SFTP: reply cut short".into());
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(((self.u32()? as u64) << 32) | self.u32()? as u64)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.bytes()?).into_owned())
    }
    fn attrs(&mut self) -> Result<Attrs, String> {
        let flags = self.u32()?;
        let mut a = Attrs::default();
        if flags & 0x1 != 0 {
            a.size = self.u64()?;
        }
        if flags & 0x2 != 0 {
            self.u32()?;
            self.u32()?;
        }
        if flags & 0x4 != 0 {
            a.perms = Some(self.u32()?);
        }
        if flags & 0x8 != 0 {
            self.u32()?;
            a.mtime = self.u32()? as i64;
        }
        if flags & 0x8000_0000 != 0 {
            for _ in 0..self.u32()? {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Ok(a)
    }
}

#[derive(Default)]
struct Attrs {
    size: u64,
    perms: Option<u32>,
    mtime: i64,
}

impl Attrs {
    fn kind(&self) -> u32 {
        self.perms.unwrap_or(0) & 0o170000
    }
    fn is_dir(&self) -> bool {
        self.kind() == 0o040000
    }
    fn is_link(&self) -> bool {
        self.kind() == 0o120000
    }
    fn is_file(&self) -> bool {
        self.kind() == 0o100000 || self.perms.is_none()
    }
}

fn put_u32(v: &mut Vec<u8>, n: u32) {
    v.extend_from_slice(&n.to_be_bytes());
}
fn put_u64(v: &mut Vec<u8>, n: u64) {
    v.extend_from_slice(&n.to_be_bytes());
}
fn put_str(v: &mut Vec<u8>, s: &[u8]) {
    put_u32(v, s.len() as u32);
    v.extend_from_slice(s);
}

/// Refuses values that `ssh` would read as an option.
fn safe_arg(what: &str, s: &str) -> Result<(), String> {
    if s.starts_with('-') || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Err(format!("The SSH {what} “{s}” isn't valid"))
    } else {
        Ok(())
    }
}

impl Sftp {
    pub fn connect(acct: &Account) -> Result<Sftp, String> {
        let host = acct.address.trim();
        let user = acct.user.trim();
        let port = acct.port.trim();
        safe_arg("host", host)?;
        if host.is_empty() {
            return Err("No SSH host given".into());
        }
        safe_arg("user", user)?;
        if !port.is_empty() && port.parse::<u16>().is_err() {
            return Err(format!("The SSH port “{port}” isn't a number"));
        }
        let mut cmd = Command::new("ssh");
        cmd.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "ServerAliveInterval=15",
            "-x",
            "-a",
        ]);
        if !port.is_empty() {
            cmd.args(["-p", port]);
        }
        if !user.is_empty() {
            cmd.args(["-l", user]);
        }
        cmd.args(["-s", host, "sftp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "SSH accounts need the ssh program (OpenSSH), which isn't installed".to_string()
            } else {
                format!("Could not start ssh: {e}")
            }
        })?;
        let stdin = child.stdin.take().ok_or("ssh: no stdin")?;
        let mut stdout = child.stdout.take().ok_or("ssh: no stdout")?;
        let mut err_pipe = child.stderr.take().ok_or("ssh: no stderr")?;

        let stderr = Arc::new(Mutex::new(String::new()));
        let sink = stderr.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = err_pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Ok(mut s) = sink.lock() {
                    s.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
        });
        // Replies are read on their own thread, so a full pipe in one
        // direction can never stall the other.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || loop {
            let mut len = [0u8; 4];
            if stdout.read_exact(&mut len).is_err() {
                let _ = tx.send(Err("closed".to_string()));
                return;
            }
            let len = u32::from_be_bytes(len) as usize;
            if len == 0 || len > 1 << 24 {
                let _ = tx.send(Err("SFTP: garbled reply".to_string()));
                return;
            }
            let mut pkt = vec![0u8; len];
            if stdout.read_exact(&mut pkt).is_err() {
                let _ = tx.send(Err("closed".to_string()));
                return;
            }
            if tx.send(Ok(pkt)).is_err() {
                return;
            }
        });

        let base = match acct.folder.trim().trim_end_matches('/') {
            "" | "~" => super::DEFAULT_SSH_FOLDER.to_string(),
            f => f.strip_prefix("~/").unwrap_or(f).to_string(),
        };
        let mut s = Sftp {
            child,
            stdin,
            rx,
            stderr,
            next_id: 1,
            base,
        };
        let mut init = Vec::new();
        put_u32(&mut init, 5);
        init.push(INIT);
        put_u32(&mut init, 3);
        s.write(&init)?;
        let mut reply = s.packet()?;
        if reply.u8()? != VERSION {
            return Err("SFTP: the server didn't answer as expected".into());
        }
        s.ensure_base()?;
        Ok(s)
    }

    /// The reason ssh gave up, in a sentence.
    fn ssh_error(&mut self) -> String {
        // Give ssh a moment to finish writing why it quit.
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let text = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        let last = text
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty() && !l.starts_with("Warning: Permanently added"))
            .unwrap_or("")
            .trim_start_matches("ssh: ")
            .to_string();
        if last.contains("Permission denied") {
            format!(
                "SSH refused the login ({last}). PaperView signs in with your SSH keys — \
                 set one up with ssh-copy-id, then try again."
            )
        } else if last.contains("Host key verification failed")
            || last.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
        {
            "The server's SSH host key changed, so PaperView won't connect. \
             Check it in a terminal with ssh first."
                .to_string()
        } else if last.is_empty() {
            "The SSH connection closed".to_string()
        } else if let Some(hint) = super::network_hint(&last) {
            format!("SSH: {last}. {hint}")
        } else {
            format!("SSH: {last}")
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self
            .stdin
            .write_all(bytes)
            .and_then(|()| self.stdin.flush())
            .is_err()
        {
            return Err(self.ssh_error());
        }
        Ok(())
    }

    fn packet(&mut self) -> Result<Buf, String> {
        match self.rx.recv_timeout(Duration::from_secs(60)) {
            Ok(Ok(data)) => Ok(Buf { data, pos: 0 }),
            Ok(Err(e)) if e == "closed" => Err(self.ssh_error()),
            Ok(Err(e)) => Err(e),
            Err(RecvTimeoutError::Timeout) => Err("The SSH server stopped answering".into()),
            Err(RecvTimeoutError::Disconnected) => Err(self.ssh_error()),
        }
    }

    /// Sends a request; returns its id.
    fn send(&mut self, kind: u8, payload: &[u8]) -> Result<u32, String> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let mut pkt = Vec::with_capacity(payload.len() + 9);
        put_u32(&mut pkt, (payload.len() + 5) as u32);
        pkt.push(kind);
        put_u32(&mut pkt, id);
        pkt.extend_from_slice(payload);
        self.write(&pkt)?;
        Ok(id)
    }

    /// The next reply: (type, request id, the rest).
    fn reply(&mut self) -> Result<(u8, u32, Buf), String> {
        let mut b = self.packet()?;
        let kind = b.u8()?;
        let id = b.u32()?;
        Ok((kind, id, b))
    }

    /// One request, one reply.
    fn call(&mut self, kind: u8, payload: &[u8]) -> Result<(u8, Buf), String> {
        let id = self.send(kind, payload)?;
        loop {
            let (k, rid, b) = self.reply()?;
            if rid == id {
                return Ok((k, b));
            }
        }
    }

    fn full(&self, rel: &str) -> String {
        match rel.trim_matches('/') {
            "" => self.base.clone(),
            r => format!("{}/{r}", self.base),
        }
    }

    fn path_payload(path: &str) -> Vec<u8> {
        let mut p = Vec::new();
        put_str(&mut p, path.as_bytes());
        p
    }

    fn status(what: &str, kind: u8, mut b: Buf) -> Result<(), String> {
        if kind != STATUS {
            return Err(format!("{what}: unexpected reply"));
        }
        let code = b.u32()?;
        if code == FX_OK {
            return Ok(());
        }
        let msg = b.string().unwrap_or_default();
        Err(match code {
            FX_NO_SUCH_FILE => format!("{what}: no such file or folder"),
            3 => format!("{what}: permission denied"),
            _ if !msg.is_empty() => format!("{what}: {msg}"),
            _ => format!("{what}: failed (code {code})"),
        })
    }

    fn handle(what: &str, kind: u8, b: Buf) -> Result<Vec<u8>, String> {
        let mut b = b;
        if kind == HANDLE {
            return b.bytes();
        }
        Self::status(what, kind, b)?;
        Err(format!("{what}: unexpected reply"))
    }

    fn stat(&mut self, path: &str) -> Result<Option<Attrs>, String> {
        let (k, mut b) = self.call(STAT, &Self::path_payload(path))?;
        if k == ATTRS {
            return Ok(Some(b.attrs()?));
        }
        let code = b.u32()?;
        if code == FX_NO_SUCH_FILE {
            Ok(None)
        } else {
            Err(format!("Could not look at {path} (code {code})"))
        }
    }

    fn mkdir_raw(&mut self, path: &str) -> Result<(), String> {
        let mut p = Self::path_payload(path);
        put_u32(&mut p, 0); // no attributes
        let (k, b) = self.call(MKDIR, &p)?;
        Self::status("Could not make the folder", k, b)
    }

    /// The account folder is made on first use (`mkdir -p`).
    fn ensure_base(&mut self) -> Result<(), String> {
        match self.stat(&self.base.clone())? {
            Some(a) if a.is_dir() || a.is_link() => return Ok(()),
            Some(_) => {
                return Err(format!(
                    "{} on the server is a file, not a folder",
                    self.base
                ))
            }
            None => {}
        }
        let absolute = self.base.starts_with('/');
        let mut so_far = String::new();
        for part in self.base.clone().split('/').filter(|p| !p.is_empty()) {
            if so_far.is_empty() {
                so_far = if absolute {
                    format!("/{part}")
                } else {
                    part.to_string()
                };
            } else {
                so_far = format!("{so_far}/{part}");
            }
            if self.stat(&so_far)?.is_none() {
                self.mkdir_raw(&so_far)?;
            }
        }
        Ok(())
    }

    fn open(&mut self, path: &str, flags: u32, what: &str) -> Result<Vec<u8>, String> {
        let mut p = Self::path_payload(path);
        put_u32(&mut p, flags);
        put_u32(&mut p, 0);
        let (k, b) = self.call(OPEN, &p)?;
        Self::handle(what, k, b)
    }

    fn close(&mut self, handle: &[u8]) -> Result<(), String> {
        let mut p = Vec::new();
        put_str(&mut p, handle);
        let (k, b) = self.call(CLOSE, &p)?;
        Self::status("Could not finish", k, b)
    }
}

impl Drop for Sftp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Backend for Sftp {
    fn list(&mut self, dir: &str) -> Result<Vec<CloudFile>, String> {
        let full = self.full(dir);
        let (k, b) = self.call(OPENDIR, &Self::path_payload(&full))?;
        let handle = Self::handle("Could not open the folder", k, b)?;
        let mut entries: Vec<(String, Attrs)> = Vec::new();
        loop {
            let mut p = Vec::new();
            put_str(&mut p, &handle);
            let (k, mut b) = self.call(READDIR, &p)?;
            if k == NAME {
                for _ in 0..b.u32()? {
                    let name = b.string()?;
                    let _long = b.bytes()?;
                    let attrs = b.attrs()?;
                    if name != "." && name != ".." {
                        entries.push((name, attrs));
                    }
                }
                continue;
            }
            if k == STATUS && b.u32()? == FX_EOF {
                break;
            }
            let _ = self.close(&handle);
            return Err("Could not read the folder".into());
        }
        self.close(&handle)?;
        let mut out = Vec::new();
        for (name, mut attrs) in entries {
            if name.starts_with('.') {
                continue;
            }
            let path = super::join(dir, &name);
            if attrs.is_link() {
                // Follow the link to see what it points at.
                match self.stat(&self.full(&path))? {
                    Some(a) => attrs = a,
                    None => continue,
                }
            }
            if attrs.is_dir() {
                out.push(CloudFile::dir(path));
            } else if attrs.is_file() {
                out.push(CloudFile::file(
                    path,
                    attrs.size,
                    attrs.mtime * 1000,
                    String::new(),
                ));
            }
        }
        Ok(out)
    }

    fn get(
        &mut self,
        path: &str,
        _size: u64,
        out: &mut dyn Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, String> {
        let full = self.full(path);
        let handle = self.open(&full, READ_FLAG, "Could not open the file")?;
        let read_req = |handle: &[u8], off: u64, len: u32| {
            let mut p = Vec::with_capacity(handle.len() + 16);
            put_str(&mut p, handle);
            put_u64(&mut p, off);
            put_u32(&mut p, len);
            p
        };
        let mut pending: HashMap<u32, (u64, u32)> = HashMap::new();
        let mut chunks: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
        let mut next = 0u64;
        let mut written = 0u64;
        let mut eof = false;
        let result = (|| -> Result<u64, String> {
            loop {
                while !eof && pending.len() < WINDOW {
                    let id = self.send(READ, &read_req(&handle, next, CHUNK))?;
                    pending.insert(id, (next, CHUNK));
                    next += CHUNK as u64;
                }
                if pending.is_empty() {
                    break;
                }
                let (k, id, mut b) = self.reply()?;
                let Some((off, len)) = pending.remove(&id) else {
                    continue;
                };
                match k {
                    DATA => {
                        let data = b.bytes()?;
                        let n = data.len() as u32;
                        if n == 0 {
                            eof = true;
                        } else {
                            chunks.insert(off, data);
                            if n < len {
                                // A short read: ask for the rest of that span.
                                let id =
                                    self.send(READ, &read_req(&handle, off + n as u64, len - n))?;
                                pending.insert(id, (off + n as u64, len - n));
                            }
                        }
                    }
                    STATUS => {
                        let code = b.u32()?;
                        if code == FX_EOF {
                            eof = true;
                        } else {
                            return Err(format!("Download failed (code {code})"));
                        }
                    }
                    _ => return Err("Download failed: unexpected reply".into()),
                }
                while let Some(data) = chunks.remove(&written) {
                    out.write_all(&data)
                        .map_err(|e| format!("Could not save download: {e}"))?;
                    written += data.len() as u64;
                    progress(written);
                }
            }
            if !chunks.is_empty() {
                return Err("Download failed: the file changed while it was read".into());
            }
            Ok(written)
        })();
        let closed = self.close(&handle);
        let n = result?;
        closed?;
        Ok(n)
    }

    fn put(
        &mut self,
        path: &str,
        data: &[u8],
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        let full = self.full(path);
        let handle = self.open(
            &full,
            WRITE_FLAG | CREAT | TRUNC,
            "Could not create the file",
        )?;
        let mut pending: HashMap<u32, usize> = HashMap::new();
        let mut chunks = data.chunks(CHUNK as usize).enumerate();
        let mut acked = 0u64;
        let result = (|| -> Result<(), String> {
            loop {
                while pending.len() < WINDOW {
                    let Some((i, chunk)) = chunks.next() else {
                        break;
                    };
                    let mut p = Vec::with_capacity(handle.len() + chunk.len() + 16);
                    put_str(&mut p, &handle);
                    put_u64(&mut p, (i * CHUNK as usize) as u64);
                    put_str(&mut p, chunk);
                    let id = self.send(WRITE, &p)?;
                    pending.insert(id, chunk.len());
                }
                if pending.is_empty() {
                    return Ok(());
                }
                let (k, id, b) = self.reply()?;
                if let Some(len) = pending.remove(&id) {
                    Self::status("Upload failed", k, b)?;
                    acked += len as u64;
                    progress(acked);
                }
            }
        })();
        let closed = self.close(&handle);
        result?;
        closed
    }

    fn delete(&mut self, path: &str, _id: &str) -> Result<(), String> {
        let (k, b) = self.call(REMOVE, &Self::path_payload(&self.full(path)))?;
        Self::status(&format!("Could not delete {path}"), k, b)
    }

    fn rename(&mut self, from: &str, _id: &str, to: &str) -> Result<(), String> {
        let mut p = Self::path_payload(&self.full(from));
        put_str(&mut p, self.full(to).as_bytes());
        let (k, b) = self.call(RENAME, &p)?;
        Self::status("Could not rename", k, b)
    }

    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        self.mkdir_raw(&self.full(path))
    }
}
