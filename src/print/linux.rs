//! Linux printing — placeholder until the CUPS backend lands.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use super::PrintEvent;
use crate::pdf::PdfEngine;

/// What the print dialog chose.
pub struct PrintTicket {
    pub from: i32,
    pub to: i32,
    /// Printable area in device pixels.
    pub dev_w: i32,
    pub dev_h: i32,
    /// Device resolution, dots per inch.
    pub dpi: i32,
}

pub fn show_dialog(_owner: isize, _page_count: i32) -> Result<Option<PrintTicket>, String> {
    Err("Printing isn't available on Linux yet.".into())
}

pub fn spawn_print(
    _engine: PdfEngine,
    _path: PathBuf,
    _password: Option<String>,
    _ticket: PrintTicket,
    _doc_name: String,
    tx: Sender<PrintEvent>,
    repaint: eframe::egui::Context,
) {
    let _ = tx.send(PrintEvent::Error("Printing isn't available on Linux yet.".into()));
    repaint.request_repaint();
}
