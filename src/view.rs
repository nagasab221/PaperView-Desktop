//! Page layout math and the texture cache shared by the page view and the
//! thumbnail sidebar.

use std::collections::HashMap;

use eframe::egui;

use crate::pdf::Document;

/// Logical pixels per PDF point at 100 % zoom (96 dpi over 72 pt/inch).
pub const PX_PER_PT: f32 = 96.0 / 72.0;
/// Vertical gap between pages in continuous mode, logical px.
pub const PAGE_GAP: f32 = 12.0;
/// Largest texture edge we'll ask PDFium / the GPU for.
pub const MAX_TEXTURE_DIM: i32 = 8192;

/// Positions of pages at a given zoom, origin (0, 0) at the top-left of the
/// scrollable content.
pub struct Layout {
    pub rects: Vec<egui::Rect>,
    pub content: egui::Vec2,
}

/// All pages stacked vertically, centered horizontally.
pub fn layout_pages(sizes_pt: &[(f32, f32)], zoom: f32, min_width: f32) -> Layout {
    let scale = PX_PER_PT * zoom;
    let widest = sizes_pt.iter().map(|s| s.0 * scale).fold(0.0f32, f32::max);
    let content_w = (widest + PAGE_GAP * 2.0).max(min_width);
    let mut rects = Vec::with_capacity(sizes_pt.len());
    let mut y = PAGE_GAP;
    for &(w_pt, h_pt) in sizes_pt {
        let size = egui::vec2(w_pt * scale, h_pt * scale);
        let x = (content_w - size.x) * 0.5;
        rects.push(egui::Rect::from_min_size(egui::pos2(x, y), size));
        y += size.y + PAGE_GAP;
    }
    Layout {
        rects,
        content: egui::vec2(content_w, y),
    }
}

/// One page centered in (at least) the viewport — single-page mode.
pub fn layout_single(size_pt: (f32, f32), zoom: f32, viewport: egui::Vec2) -> Layout {
    let scale = PX_PER_PT * zoom;
    let size = egui::vec2(size_pt.0 * scale, size_pt.1 * scale);
    let content = egui::vec2(
        (size.x + PAGE_GAP * 2.0).max(viewport.x),
        (size.y + PAGE_GAP * 2.0).max(viewport.y),
    );
    let rect = egui::Rect::from_min_size(((content - size) * 0.5).to_pos2(), size);
    Layout {
        rects: vec![rect],
        content,
    }
}

/// Rendered-page textures keyed by (page index, pixel width, rotation).
#[derive(Default)]
pub struct TexCache {
    map: HashMap<(i32, i32, u8), egui::TextureHandle>,
}

impl TexCache {
    pub fn get(&self, page: i32, px_w: i32, rot: u8) -> Option<(egui::TextureId, egui::Vec2)> {
        self.map
            .get(&(page, px_w, rot))
            .map(|t| (t.id(), t.size_vec2()))
    }

    /// Any texture for this page at this rotation, regardless of resolution —
    /// used to stretch a stale texture while a zoom gesture is in motion.
    pub fn any_for_page(&self, page: i32, rot: u8) -> Option<(egui::TextureId, egui::Vec2)> {
        self.map
            .iter()
            .find(|((p, _, r), _)| *p == page && *r == rot)
            .map(|(_, t)| (t.id(), t.size_vec2()))
    }

    /// Returns the texture for (page, px_w, rot), rendering it now if missing.
    pub fn render_into(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        page: i32,
        px_w: i32,
        rot: u8,
        name_prefix: &str,
    ) -> Result<(egui::TextureId, egui::Vec2), String> {
        use std::collections::hash_map::Entry;
        match self.map.entry((page, px_w, rot)) {
            Entry::Occupied(e) => Ok((e.get().id(), e.get().size_vec2())),
            Entry::Vacant(v) => {
                let rendered = doc.render_page(page, px_w, rot)?;
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [rendered.width, rendered.height],
                    &rendered.rgba,
                );
                let tex = ctx.load_texture(
                    format!("{name_prefix}-{page}-{px_w}-{rot}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                let out = (tex.id(), tex.size_vec2());
                v.insert(tex);
                Ok(out)
            }
        }
    }

    /// Keeps only entries for which `keep` returns true.
    pub fn retain(&mut self, keep: impl Fn(i32, i32, u8) -> bool) {
        self.map.retain(|(page, px, rot), _| keep(*page, *px, *rot));
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacks_and_centers_pages() {
        // Three US-Letter pages at 100 %: 612 pt × 4/3 = 816 px, 792 pt → 1056 px.
        let sizes = vec![(612.0, 792.0); 3];
        let l = layout_pages(&sizes, 1.0, 500.0);
        assert_eq!(l.rects.len(), 3);
        assert!((l.content.x - (816.0 + 2.0 * PAGE_GAP)).abs() < 0.5);
        assert!((l.content.y - (3.0 * 1056.0 + 4.0 * PAGE_GAP)).abs() < 0.5);
        // Second page sits one page + gap below the first.
        assert!((l.rects[1].min.y - (PAGE_GAP + 1056.0 + PAGE_GAP)).abs() < 0.5);
        // Centered: equal margins.
        let left = l.rects[0].min.x;
        let right = l.content.x - l.rects[0].max.x;
        assert!((left - right).abs() < 0.5);
    }

    #[test]
    fn single_page_centers_in_viewport() {
        let l = layout_single((612.0, 792.0), 0.5, egui::vec2(1000.0, 900.0));
        // 612 × 4/3 × 0.5 = 408 wide, 528 tall — smaller than viewport, so
        // content matches the viewport and the page is centered.
        assert_eq!(l.content, egui::vec2(1000.0, 900.0));
        let r = l.rects[0];
        assert!((r.center().x - 500.0).abs() < 0.5);
        assert!((r.center().y - 450.0).abs() < 0.5);
    }
}
