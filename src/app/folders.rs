//! Folders: directories the reader adds to the Library, shown as shelves of
//! the PDFs inside them (subfolders included). Nothing is copied or moved;
//! the folder is read again whenever its tab is shown, so files added or
//! deleted in the file manager appear without doing anything.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::ViewerApp;

/// Deep enough for "Semester 3/Pharmacology/Week 2", not a whole disk.
const MAX_DEPTH: usize = 6;
/// A folder with more than this is listed only up to it.
const MAX_FILES: usize = 3000;
/// While the Folders tab is open, re-read the folders this often.
pub(super) const RESCAN_EVERY: Duration = Duration::from_secs(8);

pub(super) struct FolderDoc {
    pub path: PathBuf,
    /// Subfolder relative to the added folder, "" for files at its top.
    pub subdir: String,
    pub modified: SystemTime,
    pub len: u64,
}

pub(super) struct Listing {
    pub root: PathBuf,
    pub docs: Vec<FolderDoc>,
    pub missing: bool,
    pub truncated: bool,
}

/// All PDFs under `root`, sorted by subfolder then name.
pub(super) fn scan(root: &Path) -> Listing {
    let mut docs = Vec::new();
    let mut truncated = false;
    let missing = !root.is_dir();
    if !missing {
        walk(root, root, 0, &mut docs, &mut truncated);
    }
    docs.sort_by_cached_key(|d: &FolderDoc| {
        (
            d.subdir.to_lowercase(),
            d.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(),
        )
    });
    Listing { root: root.to_path_buf(), docs, missing, truncated }
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<FolderDoc>, truncated: &mut bool) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            *truncated = true;
            return;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue; // hidden files and folders
        }
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                subdirs.push(path);
            }
        } else if name.to_ascii_lowercase().ends_with(".pdf") {
            let meta = entry.metadata().ok();
            if meta.as_ref().is_some_and(|m| m.len() == 0) {
                continue;
            }
            let subdir = dir
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(FolderDoc {
                path,
                subdir,
                len: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                modified: meta.and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    for sub in subdirs {
        walk(root, &sub, depth + 1, out, truncated);
    }
}

pub(super) fn encode(folders: &[PathBuf]) -> String {
    folders.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")
}

pub(super) fn decode(raw: &str) -> Vec<PathBuf> {
    raw.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect()
}

impl ViewerApp {
    pub(super) fn add_folder(&mut self, path: PathBuf) {
        if self.folders.iter().any(|f| f == &path) {
            self.toast("That folder is already in your Library");
        } else {
            let name = folder_name(&path);
            self.folders.push(path);
            self.toast(format!("Added {name}"));
        }
        self.rescan_folders();
        self.landing_tab = super::LandingTab::Folders;
        self.home = super::HomeScreen::Library;
    }

    pub(super) fn remove_folder(&mut self, path: &Path) {
        self.folders.retain(|f| f != path);
        self.rescan_folders();
        self.toast(format!("Removed {} from your Library", folder_name(path)));
    }

    pub(super) fn pick_folder(&mut self) {
        if let Some(path) = rfd::FileDialog::new().set_title("Add a folder of PDFs").pick_folder() {
            self.add_folder(path);
        }
    }

    /// Re-reads the folders on a background thread; a big tree must never
    /// stall a frame. Results land via [`Self::poll_folder_scan`].
    pub(super) fn rescan_folders(&mut self) {
        self.folders_scanned = Some(Instant::now());
        if self.folder_scan.is_some() {
            return; // one at a time
        }
        let folders = self.folders.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(folders.iter().map(|f| scan(f)).collect::<Vec<_>>());
        });
        self.folder_scan = Some(rx);
    }

    pub(super) fn poll_folder_scan(&mut self) {
        let Some(rx) = &self.folder_scan else { return };
        match rx.try_recv() {
            Ok(listings) => {
                // The scan already read every file's size and date: hand them
                // to the covers so the UI thread never has to ask the disk.
                for doc in listings.iter().flat_map(|l| &l.docs) {
                    let secs = doc
                        .modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    self.covers.seed(&doc.path, doc.len, secs);
                }
                // Folders removed while scanning don't come back.
                self.folder_listings =
                    listings.into_iter().filter(|l| self.folders.contains(&l.root)).collect();
                self.folder_scan = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.folder_scan = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    pub(super) fn scanning_folders(&self) -> bool {
        self.folder_scan.is_some()
    }
}

/// The folder's own name, for headings.
pub(super) fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_pdfs_in_subfolders_and_skips_the_rest() {
        let root = std::env::temp_dir().join(format!("pv-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Week 2")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("b.pdf"), b"%PDF-1.4").unwrap();
        std::fs::write(root.join("A.PDF"), b"%PDF-1.4").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        std::fs::write(root.join("empty.pdf"), b"").unwrap();
        std::fs::write(root.join("Week 2/c.pdf"), b"%PDF-1.4").unwrap();
        std::fs::write(root.join(".hidden/d.pdf"), b"%PDF-1.4").unwrap();

        let listing = scan(&root);
        let names: Vec<String> = listing
            .docs
            .iter()
            .map(|d| format!("{}|{}", d.subdir, d.path.file_name().unwrap().to_string_lossy()))
            .collect();
        assert_eq!(names, ["|A.PDF", "|b.pdf", "Week 2|c.pdf"]);
        assert!(!listing.missing);
        let _ = std::fs::remove_dir_all(&root);

        assert!(scan(&root).missing);
    }

    #[test]
    fn folder_list_round_trips() {
        let list = vec![PathBuf::from("/home/me/Med school"), PathBuf::from("/tmp/x")];
        assert_eq!(decode(&encode(&list)), list);
    }
}
