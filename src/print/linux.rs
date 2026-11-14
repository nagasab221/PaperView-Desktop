//! Linux printing through CUPS.
//!
//! There is no system print dialog to borrow, so the choices — printer,
//! copies, page range — live in PaperView's own print preview, and the job is
//! handed to `lp`. CUPS prints PDF natively, so the document goes as it is:
//! sharper than rasterising it ourselves, and the printer driver does the
//! paper handling.

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Sender;

use eframe::egui;

use super::PrintEvent;
use crate::pdf::PdfEngine;

/// What the print preview chose.
pub struct PrintTicket {
    /// Page range, 0-based inclusive.
    pub from: i32,
    pub to: i32,
    /// Paper in device pixels at `dpi` (A4 or Letter), for the preview.
    pub dev_w: i32,
    pub dev_h: i32,
    pub dpi: i32,
    pub printers: Vec<String>,
    pub printer: usize,
    pub copies: u32,
    pub page_count: i32,
}

const DPI: i32 = 300;

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn have_cups() -> bool {
    Command::new("lp").arg("--help").output().is_ok()
}

/// Paper size of a printer's default media: (width, height) in device px.
fn paper_for(printer: &str) -> (i32, i32) {
    let opts = run("lpoptions", &["-p", printer]).unwrap_or_default();
    let letter = opts.split_whitespace().any(|o| {
        (o.starts_with("PageSize=") || o.starts_with("media="))
            && o.to_ascii_lowercase().contains("letter")
    });
    if letter {
        (2550, 3300) // 8.5 × 11 in
    } else {
        (2480, 3508) // A4
    }
}

/// Gathers the printers; `Ok(None)` is never returned (there's no dialog to
/// cancel — the preview is where the choice happens).
pub fn show_dialog(_owner: isize, page_count: i32) -> Result<Option<PrintTicket>, String> {
    if !have_cups() {
        return Err(
            "Printing needs CUPS, the Linux printing system. Install it (on Arch/CachyOS: \
                    sudo pacman -S cups, then sudo systemctl enable --now cups) and add your \
                    printer in System Settings > Printers."
                .into(),
        );
    }
    let printers: Vec<String> = run("lpstat", &["-e"])
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if printers.is_empty() {
        return Err("No printers are set up yet. Add one in System Settings > Printers.".into());
    }
    let default = run("lpstat", &["-d"])
        .and_then(|s| s.rsplit_once(':').map(|(_, n)| n.trim().to_string()))
        .and_then(|name| printers.iter().position(|p| *p == name))
        .unwrap_or(0);
    let (dev_w, dev_h) = paper_for(&printers[default]);
    Ok(Some(PrintTicket {
        from: 0,
        to: (page_count - 1).max(0),
        dev_w,
        dev_h,
        dpi: DPI,
        printers,
        printer: default,
        copies: 1,
        page_count,
    }))
}

/// Printer, copies and range — shown inside the print preview.
pub fn options_ui(ui: &mut egui::Ui, t: &mut PrintTicket) {
    egui::Grid::new("print-options")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Printer");
            let before = t.printer;
            egui::ComboBox::from_id_salt("printer")
                .selected_text(t.printers[t.printer].as_str())
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (i, name) in t.printers.iter().enumerate() {
                        ui.selectable_value(&mut t.printer, i, name);
                    }
                });
            if t.printer != before {
                let (w, h) = paper_for(&t.printers[t.printer]);
                t.dev_w = w;
                t.dev_h = h;
            }
            ui.end_row();

            ui.label("Copies");
            boxed(ui, |ui| {
                ui.add(egui::DragValue::new(&mut t.copies).range(1..=99))
            });
            ui.end_row();

            ui.label("Pages");
            ui.horizontal(|ui| {
                let last = t.page_count;
                let mut from = t.from + 1;
                let mut to = t.to + 1;
                boxed(ui, |ui| {
                    ui.add(egui::DragValue::new(&mut from).range(1..=last))
                });
                ui.label("to");
                boxed(ui, |ui| {
                    ui.add(egui::DragValue::new(&mut to).range(1..=last))
                });
                if ui.small_button("All").clicked() {
                    from = 1;
                    to = last;
                }
                t.from = (from.min(to) - 1).max(0);
                t.to = (from.max(to) - 1).max(0);
            });
            ui.end_row();
        });
}

/// A number box on a hairline frame, so it reads as editable.
fn boxed(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> egui::Response) {
    let p = crate::ui::widgets::pal(ui);
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0, p.hairline))
        .fill(p.surface)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| add(ui));
}

/// Sends the document to CUPS on a worker thread.
pub fn spawn_print(
    _engine: PdfEngine,
    path: PathBuf,
    password: Option<String>,
    ticket: PrintTicket,
    doc_name: String,
    tx: Sender<PrintEvent>,
    repaint: egui::Context,
) {
    std::thread::spawn(move || {
        let send = |e: PrintEvent| {
            let _ = tx.send(e);
            repaint.request_repaint();
        };
        if password.is_some() {
            send(PrintEvent::Error(
                "Password-protected PDFs can't be sent to the printer directly. Save an \
                 unlocked copy first, or print from another app."
                    .into(),
            ));
            return;
        }
        let printer = ticket.printers[ticket.printer].clone();
        send(PrintEvent::Status(format!("Sending to {printer}…")));
        let range = format!("{}-{}", ticket.from + 1, ticket.to + 1);
        let out = Command::new("lp")
            .args(["-d", &printer])
            .args(["-n", &ticket.copies.to_string()])
            .args(["-P", &range])
            .args(["-t", &doc_name])
            .args(["-o", "fit-to-page"])
            .arg("--")
            .arg(&path)
            .output();
        match out {
            Ok(o) if o.status.success() => {
                send(PrintEvent::Done((ticket.to - ticket.from + 1) as usize))
            }
            Ok(o) => send(PrintEvent::Error(format!(
                "The printer refused the job: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ))),
            Err(e) => send(PrintEvent::Error(format!("Could not run lp: {e}"))),
        }
    });
}
