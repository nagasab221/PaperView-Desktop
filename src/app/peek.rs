//! Peek: a saved spot in a document — typically the answer key — that can be
//! glanced at without losing your place.
//!
//! * **Hold Q** (or hold the eye button): the view shows the spot for as long
//!   as it is held, then snaps back exactly where it was.
//! * **Tap Q** (or click the eye): jump to the spot; tap again to come back.
//! * **Shift+Q** (or the ☰ menu): save the current view as the
//!   spot; pressing it again moves the spot.
//!
//! One spot per document, remembered between sessions. A spot is stored
//! relative to its page — how far down it, and the zoom mode — so it lands on
//! the same place whatever the window size.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

use super::{zoom_from_str, zoom_to_str, ScrollTarget, ViewMode, ViewerApp, Zoom};
use crate::view::{self, PAGE_GAP};

/// Held longer than this, the button or key peeks instead of jumping.
pub(super) const HOLD_TO_PEEK: Duration = Duration::from_millis(220);
/// Jumping there and straight back within this is a peek done the slow way.
const QUICK_RETURN: Duration = Duration::from_secs(15);
/// The "hold to peek" tip is shown at most this many times.
const TIP_LIMIT: u32 = 3;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct PeekSpot {
    pub page: i32,
    /// Top of the view as a fraction of the page's height (may be slightly
    /// negative when the gap above the page was showing).
    pub y_frac: f32,
    /// Horizontal scroll as a fraction of the content width.
    pub x_frac: f32,
    pub zoom: Zoom,
}

/// Exactly where the view was, so a peek can put it back to the pixel.
#[derive(Clone, Copy, Debug)]
pub(super) struct ViewSnap {
    page: i32,
    zoom: Zoom,
    offset: egui::Vec2,
}

/// Peek state for the active document plus the per-document spots.
#[derive(Default)]
pub(super) struct Peek {
    pub spots: HashMap<PathBuf, PeekSpot>,
    /// Where a tap jumped from; set while "away" at the spot.
    away_from: Option<ViewSnap>,
    /// Where a hold started; restored on release.
    hold_from: Option<ViewSnap>,
    /// When the Q key / eye button went down, while it is down.
    key_down_at: Option<Instant>,
    button_down_at: Option<Instant>,
    jumped_at: Option<Instant>,
    pub tips_shown: u32,
}

impl Peek {
    pub fn peeking(&self) -> bool {
        self.hold_from.is_some()
    }

    pub fn away(&self) -> bool {
        self.away_from.is_some()
    }

    /// Whether the Q key is currently being tracked as held.
    pub fn key_held(&self) -> bool {
        self.key_down_at.is_some()
    }

    /// Where the reader really is while a hold is showing the spot.
    pub fn held_view(&self) -> Option<(i32, Zoom, egui::Vec2)> {
        self.hold_from.map(|s| (s.page, s.zoom, s.offset))
    }
}

pub(super) fn encode_spots(spots: &HashMap<PathBuf, PeekSpot>) -> String {
    let mut lines: Vec<String> = spots
        .iter()
        .map(|(path, s)| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                s.page,
                s.y_frac,
                s.x_frac,
                zoom_to_str(s.zoom),
                path.display()
            )
        })
        .collect();
    lines.sort();
    lines.join("\n")
}

pub(super) fn decode_spots(raw: &str) -> HashMap<PathBuf, PeekSpot> {
    raw.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(5, '\t').collect();
            let [page, y, x, zoom, path] = f.as_slice() else {
                return None;
            };
            if path.is_empty() {
                return None;
            }
            Some((
                PathBuf::from(path),
                PeekSpot {
                    page: page.parse().ok()?,
                    y_frac: y.parse().ok()?,
                    x_frac: x.parse().ok()?,
                    zoom: zoom_from_str(zoom),
                },
            ))
        })
        .collect()
}

impl ViewerApp {
    fn doc_path(&self) -> Option<PathBuf> {
        self.doc.as_ref().map(|d| d.path.clone())
    }

    /// This document's peek spot, if it has one.
    pub(super) fn peek_spot(&self) -> Option<PeekSpot> {
        let path = self.doc.as_ref()?.path.as_path();
        self.peek.spots.get(path).copied()
    }

    /// What is on screen now, as a page-relative spot.
    fn current_spot(&self) -> Option<PeekSpot> {
        let count = self.page_count();
        if count == 0 {
            return None;
        }
        let avail = self.central_rect.size();
        let zoom = self.effective_zoom;
        let off = self.last_offset;
        let (page, rect, content_w) = match self.mode {
            ViewMode::Continuous => {
                let sizes: Vec<(f32, f32)> = (0..count).map(|i| self.display_size(i)).collect();
                let layout = view::layout_pages(&sizes, zoom, avail.x);
                // The page under the top edge of the view anchors the spot.
                let page = layout
                    .rects
                    .iter()
                    .rposition(|r| r.min.y - PAGE_GAP <= off.y)
                    .unwrap_or(0);
                (page as i32, layout.rects[page], layout.content.x)
            }
            ViewMode::Single => {
                let page = self.page_index.clamp(0, count - 1);
                let layout = view::layout_single(self.display_size(page), zoom, avail);
                (page, layout.rects[0], layout.content.x)
            }
        };
        Some(PeekSpot {
            page,
            y_frac: (off.y - rect.min.y) / rect.height().max(1.0),
            x_frac: if content_w > 0.0 {
                off.x / content_w
            } else {
                0.0
            },
            zoom: self.zoom,
        })
    }

    fn snap(&self) -> ViewSnap {
        ViewSnap {
            page: self.page_index,
            zoom: self.zoom,
            offset: self.last_offset,
        }
    }

    fn restore(&mut self, s: ViewSnap) {
        self.zoom = s.zoom;
        self.zoom_prev_effective = None;
        self.zoom_anchor = None;
        self.pending_scroll = 0.0;
        self.page_index = s.page.clamp(0, (self.page_count() - 1).max(0));
        self.scroll_to = Some(ScrollTarget::Offset(s.offset));
    }

    fn show_spot(&mut self, spot: PeekSpot) {
        self.zoom = spot.zoom;
        self.zoom_prev_effective = None;
        self.zoom_anchor = None;
        self.pending_scroll = 0.0;
        self.page_index = spot.page.clamp(0, (self.page_count() - 1).max(0));
        self.scroll_to = Some(ScrollTarget::Spot {
            page: self.page_index,
            y_frac: spot.y_frac,
            x_frac: spot.x_frac,
        });
    }

    /// Shift+Q / menu: save what is on screen as this document's spot.
    pub(super) fn save_peek_spot(&mut self) {
        let (Some(path), Some(spot)) = (self.doc_path(), self.current_spot()) else {
            return;
        };
        let first = !self.peek.spots.contains_key(&path);
        self.peek.spots.insert(path, spot);
        // Saving here makes "here" the spot: there is nothing to go back to.
        self.peek.away_from = None;
        self.toast(if first {
            "Peek spot saved. Hold Q (or the eye) to peek, tap Q to jump there and back."
        } else {
            "Peek spot moved here"
        });
    }

    pub(super) fn remove_peek_spot(&mut self) {
        if let Some(path) = self.doc_path() {
            self.peek.spots.remove(&path);
        }
        self.peek.away_from = None;
        self.toast("Peek spot removed");
    }

    /// A tap: go to the spot, or come back if a tap already went there.
    pub(super) fn peek_tap(&mut self) {
        let Some(spot) = self.peek_spot() else {
            self.toast("No peek spot yet. Press Shift+Q to save one here.");
            return;
        };
        if let Some(back) = self.peek.away_from.take() {
            self.restore(back);
            let quick = self
                .peek
                .jumped_at
                .is_some_and(|t| t.elapsed() < QUICK_RETURN);
            if quick && self.peek.tips_shown < TIP_LIMIT {
                self.peek.tips_shown += 1;
                self.toast("Tip: hold Q (or the eye) to peek — let go and you're right back.");
            }
        } else {
            self.peek.away_from = Some(self.snap());
            self.peek.jumped_at = Some(Instant::now());
            self.show_spot(spot);
        }
    }

    fn begin_peek(&mut self) {
        let Some(spot) = self.peek_spot() else { return };
        if self.peek.hold_from.is_none() {
            self.peek.hold_from = Some(self.snap());
            self.show_spot(spot);
        }
    }

    pub(super) fn end_peek(&mut self) {
        if let Some(back) = self.peek.hold_from.take() {
            self.restore(back);
        }
    }

    /// Leaving the document (tab switch, home, close): put a held peek back
    /// first, so the stashed position is the reader's, and forget "away".
    pub(super) fn peek_leave_doc(&mut self) {
        // Write the real position back directly: the document is being
        // stashed now, before a scroll could carry it there next frame.
        if let Some(s) = self.peek.hold_from.take() {
            self.page_index = s.page;
            self.zoom = s.zoom;
            self.last_offset = s.offset;
        }
        self.peek.away_from = None;
        self.peek.key_down_at = None;
        self.peek.button_down_at = None;
    }

    /// Q held or tapped. `down` is the key's state this frame.
    pub(super) fn peek_key(&mut self, ctx: &egui::Context, down: bool) {
        self.peek_press(ctx, down, |p| &mut p.key_down_at);
    }

    /// The eye button, same rules as the key.
    pub(super) fn peek_button(&mut self, ctx: &egui::Context, down: bool) {
        self.peek_press(ctx, down, |p| &mut p.button_down_at);
    }

    fn peek_press(
        &mut self,
        ctx: &egui::Context,
        down: bool,
        slot: fn(&mut Peek) -> &mut Option<Instant>,
    ) {
        let started = *slot(&mut self.peek);
        match (started, down) {
            (None, true) => {
                *slot(&mut self.peek) = Some(Instant::now());
                ctx.request_repaint_after(HOLD_TO_PEEK);
            }
            (Some(t), true) => {
                if t.elapsed() >= HOLD_TO_PEEK {
                    self.begin_peek();
                } else {
                    ctx.request_repaint_after(HOLD_TO_PEEK - t.elapsed());
                }
            }
            (Some(_), false) => {
                *slot(&mut self.peek) = None;
                if self.peek.peeking() {
                    self.end_peek();
                } else {
                    self.peek_tap();
                }
            }
            (None, false) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn spots_round_trip() {
        let mut spots = HashMap::new();
        spots.insert(
            PathBuf::from("/home/me/Answers & keys.pdf"),
            PeekSpot {
                page: 41,
                y_frac: 0.62,
                x_frac: 0.0,
                zoom: Zoom::FitWidth,
            },
        );
        spots.insert(
            PathBuf::from("/tmp/b.pdf"),
            PeekSpot {
                page: 0,
                y_frac: -0.01,
                x_frac: 0.25,
                zoom: Zoom::Custom(2.5),
            },
        );
        assert_eq!(decode_spots(&encode_spots(&spots)), spots);
    }

    #[test]
    fn broken_lines_are_skipped() {
        let raw = "junk\n1\tx\t0\tfitp\t/a.pdf\n2\t0.5\t0\tfitp\t/good.pdf\n3\t0.5\t0\tfitp\t";
        let spots = decode_spots(raw);
        assert_eq!(spots.len(), 1);
        assert!(spots.contains_key(Path::new("/good.pdf")));
    }
}
