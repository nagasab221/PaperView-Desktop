//! Registering PaperView as an "Open with" handler for PDFs, per user and
//! without admin rights. Windows uses the registry; Linux a desktop entry.

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{is_default, make_default, register, unregister};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{is_default, make_default, register, unregister};

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn register() -> Result<(), String> {
    Err("Registering as a PDF handler isn't supported on this platform".into())
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn unregister() -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn make_default() -> Result<(), String> {
    Err("Setting the default app isn't supported on this platform".into())
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn is_default() -> bool {
    false
}
