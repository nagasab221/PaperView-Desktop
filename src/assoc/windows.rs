//! Per-user `.pdf` "Open with" registration — HKCU only, no admin rights.
//!
//! Registers a `PaperView.Document` ProgID (icon = the exe's embedded icon,
//! open command = this executable) and lists it under the `.pdf` extension's
//! `OpenWithProgIds`. Windows deliberately does not let programs seize the
//! *default* handler — pick PaperView once via "Open with → Always" and it
//! sticks. Everything is written with `reg.exe`, so it is transparent and
//! trivially undoable.

use std::os::windows::process::CommandExt;
use std::process::Command;

const PROGID: &str = r"HKCU\Software\Classes\PaperView.Document";
const OPENWITH: &str = r"HKCU\Software\Classes\.pdf\OpenWithProgIds";
/// CREATE_NO_WINDOW — no console flash from a GUI app.
const NO_WINDOW: u32 = 0x0800_0000;

fn reg(args: &[&str]) -> Result<(), String> {
    let status = Command::new("reg")
        .args(args)
        .creation_flags(NO_WINDOW)
        .status()
        .map_err(|e| format!("Could not run reg.exe: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "reg.exe {} failed (exit {:?})",
            args.first().copied().unwrap_or(""),
            status.code()
        ))
    }
}

/// Registers this executable (from its current location) as a `.pdf` handler.
pub fn register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.display().to_string();
    let icon = format!("\"{exe}\",0");
    let open = format!("\"{exe}\" \"%1\"");
    reg(&[
        "add",
        PROGID,
        "/ve",
        "/d",
        "PDF Document (PaperView)",
        "/f",
    ])?;
    let icon_key = format!(r"{PROGID}\DefaultIcon");
    reg(&["add", &icon_key, "/ve", "/d", &icon, "/f"])?;
    let cmd_key = format!(r"{PROGID}\shell\open\command");
    reg(&["add", &cmd_key, "/ve", "/d", &open, "/f"])?;
    reg(&["add", OPENWITH, "/v", "PaperView.Document", "/f"])?;
    notify_shell();
    Ok(())
}

/// Removes the registration (safe to call when not registered).
pub fn unregister() -> Result<(), String> {
    let _ = reg(&["delete", PROGID, "/f"]);
    let _ = reg(&["delete", OPENWITH, "/v", "PaperView.Document", "/f"]);
    notify_shell();
    Ok(())
}

/// Tells Explorer that file associations changed (refreshes icons).
fn notify_shell() {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHChangeNotify(
            event: i32,
            flags: u32,
            item1: *const core::ffi::c_void,
            item2: *const core::ffi::c_void,
        );
    }
    const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null());
    }
}

/// Windows doesn't let programs make themselves the default: register, then
/// open Settings > Default apps so the reader can pick PaperView there.
pub fn make_default() -> Result<(), String> {
    register()?;
    Command::new("cmd")
        .args(["/C", "start", "ms-settings:defaultapps"])
        .creation_flags(NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Couldn't open Default apps: {e}"))?;
    Ok(())
}

/// Not reliably knowable on Windows (the choice is hashed); assume not.
pub fn is_default() -> bool {
    false
}
