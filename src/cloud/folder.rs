//! A folder standing in for the cloud: a mounted NAS share (SMB, NFS,
//! sshfs, KDE network folders), a Syncthing or Nextcloud-client folder.
//! The app opens these files in place; "download" copies one for offline.

use std::io::{Read, Write};
use std::path::PathBuf;

use super::{Account, Backend, CloudFile};

pub struct Folder {
    root: PathBuf,
}

impl Folder {
    pub fn connect(acct: &Account) -> Result<Folder, String> {
        let root = acct.local_root().unwrap_or_default();
        if !root.is_dir() {
            return Err(format!(
                "{} isn't there (is the share mounted?)",
                root.display()
            ));
        }
        Ok(Folder { root })
    }

    fn path(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel.split('/').filter(|s| !s.is_empty() && *s != "..") {
            p.push(part);
        }
        p
    }
}

fn io(what: &str, e: std::io::Error) -> String {
    format!("{what}: {e}")
}

impl Backend for Folder {
    fn list(&mut self, dir: &str) -> Result<Vec<CloudFile>, String> {
        let rd =
            std::fs::read_dir(self.path(dir)).map_err(|e| io("Could not read the folder", e))?;
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // metadata() follows symlinks, so a linked folder is a folder.
            let Ok(meta) = std::fs::metadata(entry.path()) else {
                continue;
            };
            let path = super::join(dir, &name);
            if meta.is_dir() {
                out.push(CloudFile::dir(path));
            } else if meta.is_file() {
                let ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                out.push(CloudFile::file(path, meta.len(), ms, String::new()));
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
        let mut file =
            std::fs::File::open(self.path(path)).map_err(|e| io("Could not open the file", e))?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            let n = file.read(&mut buf).map_err(|e| io("Read failed", e))?;
            if n == 0 {
                return Ok(done);
            }
            out.write_all(&buf[..n])
                .map_err(|e| io("Could not save the copy", e))?;
            done += n as u64;
            progress(done);
        }
    }

    fn put(
        &mut self,
        path: &str,
        data: &[u8],
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        let dest = self.path(path);
        let mut file =
            std::fs::File::create(&dest).map_err(|e| io("Could not create the file", e))?;
        for (i, chunk) in data.chunks(256 * 1024).enumerate() {
            file.write_all(chunk).map_err(|e| io("Write failed", e))?;
            progress(((i * 256 * 1024) + chunk.len()) as u64);
        }
        file.sync_all().map_err(|e| io("Write failed", e))
    }

    fn delete(&mut self, path: &str, _id: &str) -> Result<(), String> {
        std::fs::remove_file(self.path(path))
            .map_err(|e| io(&format!("Could not delete {path}"), e))
    }

    fn rename(&mut self, from: &str, _id: &str, to: &str) -> Result<(), String> {
        let dest = self.path(to);
        if dest.exists() {
            return Err(format!("There's already a file called {to}"));
        }
        std::fs::rename(self.path(from), dest).map_err(|e| io("Could not rename", e))
    }

    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        std::fs::create_dir(self.path(path)).map_err(|e| io("Could not make the folder", e))
    }
}
