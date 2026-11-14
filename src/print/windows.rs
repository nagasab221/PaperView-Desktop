//! Raster printing: the classic Win32 print dialog (comdlg32) + GDI, with
//! pages rendered by PDFium at printer resolution (capped at 300 dpi).
//!
//! The dialog runs on the UI thread (it is modal anyway); the print job runs
//! on a worker thread with its own `Document`, serialized against the UI's
//! PDFium use by the global gate inside `pdf.rs`.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use super::{layout_on_paper, PrintEvent};
use crate::pdf::{OpenError, PdfEngine};

// ── minimal Win32 FFI (hand-declared: import libs ship with the toolchain) ──

#[repr(C)]
#[allow(non_snake_case)]
struct PRINTDLGW {
    lStructSize: u32,
    hwndOwner: isize,
    hDevMode: isize,
    hDevNames: isize,
    hDC: isize,
    Flags: u32,
    nFromPage: u16,
    nToPage: u16,
    nMinPage: u16,
    nMaxPage: u16,
    nCopies: u16,
    hInstance: isize,
    lCustData: isize,
    lpfnPrintHook: isize,
    lpfnSetupHook: isize,
    lpPrintTemplateName: *const u16,
    lpSetupTemplateName: *const u16,
    hPrintTemplate: isize,
    hSetupTemplate: isize,
}

#[repr(C)]
#[allow(non_snake_case)]
struct DOCINFOW {
    cbSize: i32,
    lpszDocName: *const u16,
    lpszOutput: *const u16,
    lpszDatatype: *const u16,
    fwType: u32,
}

#[repr(C)]
#[allow(non_snake_case)]
#[derive(Clone, Copy, Default)]
struct BITMAPINFOHEADER {
    biSize: u32,
    biWidth: i32,
    biHeight: i32,
    biPlanes: u16,
    biBitCount: u16,
    biCompression: u32,
    biSizeImage: u32,
    biXPelsPerMeter: i32,
    biYPelsPerMeter: i32,
    biClrUsed: u32,
    biClrImportant: u32,
}

const PD_PAGENUMS: u32 = 0x0000_0002;
const PD_NOSELECTION: u32 = 0x0000_0004;
const PD_RETURNDC: u32 = 0x0000_0100;
const PD_USEDEVMODECOPIESANDCOLLATE: u32 = 0x0004_0000;
const PD_HIDEPRINTTOFILE: u32 = 0x0010_0000;

const LOGPIXELSX: i32 = 88;
const HORZRES: i32 = 8;
const VERTRES: i32 = 10;
const DIB_RGB_COLORS: u32 = 0;
const SRCCOPY: u32 = 0x00CC_0020;

#[link(name = "comdlg32")]
unsafe extern "system" {
    fn PrintDlgW(pd: *mut PRINTDLGW) -> i32;
    fn CommDlgExtendedError() -> u32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn StartDocW(hdc: isize, di: *const DOCINFOW) -> i32;
    fn StartPage(hdc: isize) -> i32;
    fn EndPage(hdc: isize) -> i32;
    fn EndDoc(hdc: isize) -> i32;
    fn AbortDoc(hdc: isize) -> i32;
    fn DeleteDC(hdc: isize) -> i32;
    fn GetDeviceCaps(hdc: isize, index: i32) -> i32;
    #[allow(clippy::too_many_arguments)]
    fn StretchDIBits(
        hdc: isize,
        x_dest: i32,
        y_dest: i32,
        dest_w: i32,
        dest_h: i32,
        x_src: i32,
        y_src: i32,
        src_w: i32,
        src_h: i32,
        bits: *const core::ffi::c_void,
        bmi: *const BITMAPINFOHEADER,
        usage: u32,
        rop: u32,
    ) -> i32;
}

/// A printer DC, the chosen page range (0-based, inclusive) and the device
/// metrics — enough to lay pages out exactly as they will print.
pub struct PrintTicket {
    hdc: isize,
    pub from: i32,
    pub to: i32,
    /// Printable area in device pixels.
    pub dev_w: i32,
    pub dev_h: i32,
    /// Device resolution, dots per inch.
    pub dpi: i32,
}

// The DC is only ever used by the print worker thread after the dialog.
unsafe impl Send for PrintTicket {}

// The ticket owns the DC: every exit path (incl. preview cancel) releases it.
impl Drop for PrintTicket {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.hdc);
        }
    }
}

/// Shows the system print dialog. `Ok(None)` = user cancelled.
pub fn show_dialog(owner_hwnd: isize, page_count: i32) -> Result<Option<PrintTicket>, String> {
    unsafe {
        let mut pd: PRINTDLGW = std::mem::zeroed();
        pd.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
        pd.hwndOwner = owner_hwnd;
        pd.Flags =
            PD_RETURNDC | PD_USEDEVMODECOPIESANDCOLLATE | PD_NOSELECTION | PD_HIDEPRINTTOFILE;
        let max = page_count.clamp(1, u16::MAX as i32) as u16;
        pd.nFromPage = 1;
        pd.nToPage = max;
        pd.nMinPage = 1;
        pd.nMaxPage = max;

        if PrintDlgW(&mut pd) == 0 {
            let err = CommDlgExtendedError();
            return if err == 0 {
                Ok(None) // cancelled
            } else {
                Err(format!("The print dialog failed (error 0x{err:X})."))
            };
        }
        if pd.hDC == 0 {
            return Err("The printer did not provide a device context.".into());
        }
        let (from, to) = if pd.Flags & PD_PAGENUMS != 0 {
            (
                (pd.nFromPage as i32 - 1).clamp(0, page_count - 1),
                (pd.nToPage as i32 - 1).clamp(0, page_count - 1),
            )
        } else {
            (0, page_count - 1)
        };
        let dev_w = GetDeviceCaps(pd.hDC, HORZRES).max(1);
        let dev_h = GetDeviceCaps(pd.hDC, VERTRES).max(1);
        let dpi = GetDeviceCaps(pd.hDC, LOGPIXELSX).max(96);
        Ok(Some(PrintTicket {
            hdc: pd.hDC,
            from: from.min(to),
            to: from.max(to),
            dev_w,
            dev_h,
            dpi,
        }))
    }
}

/// Prints on a worker thread; progress arrives as [`PrintEvent`]s.
pub fn spawn_print(
    engine: PdfEngine,
    path: PathBuf,
    password: Option<String>,
    ticket: PrintTicket,
    doc_name: String,
    tx: Sender<PrintEvent>,
    repaint: eframe::egui::Context,
) {
    std::thread::spawn(move || {
        let send = |e: PrintEvent| {
            let _ = tx.send(e);
            repaint.request_repaint();
        };
        match run(
            engine,
            &path,
            password.as_deref(),
            &ticket,
            &doc_name,
            &send,
        ) {
            Ok(pages) => send(PrintEvent::Done(pages)),
            Err(e) => send(PrintEvent::Error(e)),
        }
    });
}

fn run(
    engine: PdfEngine,
    path: &std::path::Path,
    password: Option<&str>,
    ticket: &PrintTicket,
    doc_name: &str,
    send: &dyn Fn(PrintEvent),
) -> Result<usize, String> {
    let doc = engine.open(path, password).map_err(|e| match e {
        OpenError::PasswordRequired => "The document needs a password.".to_string(),
        OpenError::Message(m) => m,
    })?;

    unsafe {
        let hdc = ticket.hdc;
        let dpi = ticket.dpi;
        let dev_w = ticket.dev_w;
        let dev_h = ticket.dev_h;

        let name_w: Vec<u16> = doc_name.encode_utf16().chain([0]).collect();
        let di = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: name_w.as_ptr(),
            lpszOutput: std::ptr::null(),
            lpszDatatype: std::ptr::null(),
            fwType: 0,
        };
        if StartDocW(hdc, &di) <= 0 {
            return Err("Could not start the print job (is the printer available?).".into());
        }

        let total = ticket.to - ticket.from + 1;
        let mut printed = 0usize;
        for page in ticket.from..=ticket.to {
            send(PrintEvent::Status(format!(
                "Printing page {} of {total}…",
                printed + 1
            )));
            let (w_pt, h_pt) = doc.page_sizes_pt[page as usize];
            let layout = layout_on_paper(w_pt, h_pt, dev_w, dev_h);
            let final_w_pt = if layout.rotate == 1 { h_pt } else { w_pt };
            // Render at printer resolution, capped to keep buffers sane.
            let eff_dpi = dpi.min(300) as f32;
            let target_px = ((final_w_pt / 72.0) * eff_dpi).round() as i32;
            let target_px = target_px.clamp(16, 10_000);
            let rendered = match doc.render_page(page, target_px, layout.rotate) {
                Ok(r) => r,
                Err(e) => {
                    AbortDoc(hdc);
                    return Err(e);
                }
            };
            // RGBA → BGRA for GDI.
            let mut bits = rendered.rgba;
            for px in bits.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            let (src_w, src_h) = (rendered.width as i32, rendered.height as i32);
            let dest_w = ((layout.w * dev_w as f32) as i32).max(1);
            let dest_h = ((layout.h * dev_h as f32) as i32).max(1);
            let dest_x = (layout.x * dev_w as f32) as i32;
            let dest_y = (layout.y * dev_h as f32) as i32;

            if StartPage(hdc) <= 0 {
                AbortDoc(hdc);
                return Err("The printer rejected the page (StartPage failed).".into());
            }
            let bmi = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: src_w,
                biHeight: -src_h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            };
            StretchDIBits(
                hdc,
                dest_x,
                dest_y,
                dest_w,
                dest_h,
                0,
                0,
                src_w,
                src_h,
                bits.as_ptr() as *const _,
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            if EndPage(hdc) <= 0 {
                AbortDoc(hdc);
                return Err("The printer rejected the page (EndPage failed).".into());
            }
            printed += 1;
        }
        EndDoc(hdc);
        Ok(printed)
    }
}
