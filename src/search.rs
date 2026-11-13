//! Incremental document-wide text search.
//!
//! Pages are scanned a batch per frame (starting at the page that was visible
//! when the search was submitted, wrapping around), so huge documents never
//! freeze the UI; the result counter grows as the scan fills in.

use std::collections::HashMap;

use crate::pdf::{Document, PageHit};

/// Pages scanned per `pump` call (one call per frame while incomplete).
const PAGES_PER_PUMP: i32 = 24;

#[derive(Default)]
pub struct SearchState {
    /// Text currently in the search box (not yet necessarily submitted).
    pub input: String,
    /// The submitted query the results below belong to.
    query: String,
    /// Whether highlights/results are live.
    pub active: bool,
    /// Per-page hits, filled in as the scan progresses.
    hits: HashMap<i32, Vec<PageHit>>,
    /// (page, hit index) in document order, rebuilt as pages come in.
    order: Vec<(i32, usize)>,
    /// Page the search was submitted from (auto-select prefers hits ≥ here).
    start_page: i32,
    scanned: i32,
    total_pages: i32,
    /// The focused hit, as (page, hit index on that page).
    current: Option<(i32, usize)>,
}

impl SearchState {
    /// Submit the box contents. A re-submit of the same query advances to the
    /// next hit instead of rescanning.
    pub fn submit(&mut self, doc: &Document, start_page: i32) -> Option<(i32, usize)> {
        let q = self.input.trim().to_string();
        if q.is_empty() {
            self.deactivate();
            return None;
        }
        if self.active && q == self.query {
            return self.step(doc, 1);
        }
        self.query = q;
        self.active = true;
        self.hits.clear();
        self.order.clear();
        self.current = None;
        self.start_page = start_page;
        self.scanned = 0;
        self.total_pages = doc.page_count();
        None
    }

    pub fn deactivate(&mut self) {
        self.active = false;
        self.hits.clear();
        self.order.clear();
        self.current = None;
        self.scanned = 0;
        self.query.clear();
    }

    pub fn done(&self) -> bool {
        !self.active || self.scanned >= self.total_pages
    }

    /// Scans the next batch of pages. Returns true if the scan advanced (the
    /// caller should request a repaint).
    pub fn pump(&mut self, doc: &Document) -> bool {
        if self.done() {
            return false;
        }
        for _ in 0..PAGES_PER_PUMP {
            if self.scanned >= self.total_pages {
                break;
            }
            let page = (self.start_page + self.scanned).rem_euclid(self.total_pages);
            self.scanned += 1;
            if let Ok(page_hits) = doc.page_hits(page, &self.query) {
                if !page_hits.is_empty() {
                    self.hits.insert(page, page_hits);
                }
            }
        }
        self.rebuild_order();
        true
    }

    fn rebuild_order(&mut self) {
        self.order.clear();
        let mut pages: Vec<i32> = self.hits.keys().copied().collect();
        pages.sort_unstable();
        for page in pages {
            for i in 0..self.hits[&page].len() {
                self.order.push((page, i));
            }
        }
    }

    /// Picks the first hit at/after the page the search started on, once one
    /// exists and nothing is selected yet. Returns it so the caller can jump.
    pub fn autoselect(&mut self) -> Option<(i32, usize)> {
        if !self.active || self.current.is_some() || self.order.is_empty() {
            return None;
        }
        let pick = self
            .order
            .iter()
            .find(|(p, _)| *p >= self.start_page)
            .or_else(|| self.order.first())
            .copied();
        self.current = pick;
        pick
    }

    /// Moves to the next (+1) or previous (-1) hit, wrapping. Returns the new
    /// focused hit. Makes sure at least the batch containing more results is
    /// scanned first via `pump` calls happening each frame anyway.
    pub fn step(&mut self, _doc: &Document, dir: i32) -> Option<(i32, usize)> {
        if self.order.is_empty() {
            return None;
        }
        let cur_pos = self
            .current
            .and_then(|c| self.order.iter().position(|&o| o == c));
        let next_pos = match cur_pos {
            Some(p) => {
                (p as i64 + dir as i64).rem_euclid(self.order.len() as i64) as usize
            }
            None => {
                if dir >= 0 {
                    0
                } else {
                    self.order.len() - 1
                }
            }
        };
        self.current = Some(self.order[next_pos]);
        self.current
    }

    pub fn hits_for_page(&self, page: i32) -> Option<&Vec<PageHit>> {
        if !self.active {
            return None;
        }
        self.hits.get(&page)
    }

    pub fn is_current(&self, page: i32, hit: usize) -> bool {
        self.current == Some((page, hit))
    }

    pub fn current_hit(&self) -> Option<(i32, usize)> {
        self.current
    }

    pub fn hit_rects(&self, page: i32, hit: usize) -> Option<&crate::pdf::PageHit> {
        self.hits.get(&page).and_then(|v| v.get(hit))
    }

    /// (position of current among all found so far, total found so far).
    pub fn counter(&self) -> (usize, usize) {
        let pos = self
            .current
            .and_then(|c| self.order.iter().position(|&o| o == c))
            .map(|p| p + 1)
            .unwrap_or(0);
        (pos, self.order.len())
    }
}
