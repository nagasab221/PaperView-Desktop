//! Linux: a desktop entry in `~/.local/share/applications` lists PaperView in
//! the app menu and under "Open with" for PDFs; the icon goes into the user's
//! hicolor theme. Like the Windows side it doesn't seize the default handler —
//! choose PaperView once in the file manager's "Open with" and it sticks.

use std::path::PathBuf;
use std::process::Command;

const DESKTOP_FILE: &str = "paperview.desktop";
const ICON_NAME: &str = "paperview";
const ICON_PNG: &[u8] = include_bytes!("../../assets/icon_preview.png");

fn data_home() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/share"))
}

fn desktop_path() -> Result<PathBuf, String> {
    Ok(data_home()?.join("applications").join(DESKTOP_FILE))
}

fn icon_path() -> Result<PathBuf, String> {
    Ok(data_home()?
        .join("icons/hicolor/256x256/apps")
        .join(format!("{ICON_NAME}.png")))
}

/// Quotes a path for a desktop entry's Exec key.
fn exec_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Registers this executable (from its current location) as a PDF handler.
pub fn register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_str().ok_or("The program's path isn't valid UTF-8")?;

    let icon = icon_path()?;
    std::fs::create_dir_all(icon.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&icon, ICON_PNG).map_err(|e| format!("Could not write the icon: {e}"))?;

    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=PaperView\n\
         GenericName=PDF Reader\n\
         Comment=Read PDF documents\n\
         Exec={} %f\n\
         Icon={ICON_NAME}\n\
         Terminal=false\n\
         Categories=Office;Viewer;\n\
         MimeType=application/pdf;application/epub+zip;application/x-mobipocket-ebook;application/vnd.amazon.ebook;application/x-fictionbook+xml;application/vnd.comicbook+zip;\n\
         Keywords=pdf;document;reader;viewer;\n\
         StartupWMClass=paperview\n",
        exec_quote(exe),
    );
    let path = desktop_path()?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, entry).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    refresh();
    Ok(())
}

/// The file types PaperView opens: PDFs and ebooks.
pub const MIME_TYPES: &[&str] = &[
    "application/pdf",
    "application/epub+zip",
    "application/x-mobipocket-ebook",
    "application/vnd.amazon.ebook",
    "application/x-fictionbook+xml",
    "application/vnd.comicbook+zip",
];

/// Registers PaperView and makes it the default app for PDFs and ebooks
/// (written to `mimeapps.list`, which KDE, GNOME and others all follow).
pub fn make_default() -> Result<(), String> {
    register()?;
    let status = Command::new("xdg-mime")
        .arg("default")
        .arg(DESKTOP_FILE)
        .args(MIME_TYPES)
        .status()
        .map_err(|e| format!("Couldn't run xdg-mime: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("xdg-mime couldn't set the default apps".into())
    }
}

/// Whether PDFs open with PaperView by default.
pub fn is_default() -> bool {
    Command::new("xdg-mime")
        .args(["query", "default", "application/pdf"])
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == DESKTOP_FILE)
}

/// Removes the registration (safe to call when not registered).
pub fn unregister() -> Result<(), String> {
    for path in [desktop_path()?, icon_path()?] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Could not remove {}: {e}", path.display())),
        }
    }
    refresh();
    Ok(())
}

/// Best effort: lets menus and file managers notice the change straight away.
fn refresh() {
    if let Ok(dir) = data_home() {
        let _ = Command::new("update-desktop-database")
            .arg(dir.join("applications"))
            .status();
        let _ = Command::new("gtk-update-icon-cache")
            .args(["-f", "-t"])
            .arg(dir.join("icons/hicolor"))
            .status();
    }
}
