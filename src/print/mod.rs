//! Printing. The paper layout maths is shared; the dialog and the job itself
//! are platform backends behind the same three calls: [`show_dialog`],
//! [`layout_on_paper`] and [`spawn_print`].

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{show_dialog, spawn_print, PrintTicket};

#[cfg(not(target_os = "windows"))]
mod linux;
#[cfg(not(target_os = "windows"))]
pub use linux::{show_dialog, spawn_print, PrintTicket};

pub enum PrintEvent {
    Status(String),
    Done(usize),
    Error(String),
}

/// How a page lands on the paper: rotation plus the destination rectangle as
/// fractions of the printable area. Shared by the preview and the print job.
#[derive(Clone, Copy, Debug)]
pub struct PaperLayout {
    pub rotate: u8,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn layout_on_paper(page_w_pt: f32, page_h_pt: f32, dev_w: i32, dev_h: i32) -> PaperLayout {
    // Rotate sideways pages to match the paper orientation.
    let rotate = u8::from((page_w_pt > page_h_pt) != (dev_w > dev_h));
    let (w_pt, h_pt) = if rotate == 1 {
        (page_h_pt, page_w_pt)
    } else {
        (page_w_pt, page_h_pt)
    };
    let scale = (dev_w as f32 / w_pt.max(1.0)).min(dev_h as f32 / h_pt.max(1.0));
    let w = (w_pt * scale) / dev_w as f32;
    let h = (h_pt * scale) / dev_h as f32;
    PaperLayout {
        rotate,
        x: (1.0 - w) * 0.5,
        y: (1.0 - h) * 0.5,
        w,
        h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portrait_page_on_portrait_paper() {
        // US Letter page on a Letter-shaped printable area: no rotation,
        // fills the full height, centered horizontally.
        let l = layout_on_paper(612.0, 792.0, 2400, 3100);
        assert_eq!(l.rotate, 0);
        assert!((l.h - 1.0).abs() < 0.02 || (l.w - 1.0).abs() < 0.02);
        assert!((l.x - (1.0 - l.w) * 0.5).abs() < 1e-5);
        assert!((l.y - (1.0 - l.h) * 0.5).abs() < 1e-5);
        assert!(l.w <= 1.0 && l.h <= 1.0);
    }

    #[test]
    fn landscape_page_auto_rotates() {
        let l = layout_on_paper(792.0, 612.0, 2400, 3100);
        assert_eq!(l.rotate, 1, "landscape page on portrait paper rotates");
        assert!(l.w <= 1.0 && l.h <= 1.0);
    }
}
