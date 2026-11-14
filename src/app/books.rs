//! Books on the Library's Books tab, and opening one into a document tab.
//! The tab only exists once a book has been added.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke};

use super::reader::{BookView, Mark};
use super::{now_ts, same_path, SidebarMode, Tab, ViewerApp};
use crate::ui::{theme, widgets as w};

/// A book in the Library, with where its reader left off.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BookEntry {
    pub path: PathBuf,
    pub title: String,
    pub author: Option<String>,
    pub mark: Mark,
    /// 0..1, for "41%".
    pub progress: f32,
    /// Last opened, unix seconds.
    pub ts: i64,
}

pub(super) fn encode(books: &[BookEntry]) -> String {
    books
        .iter()
        .map(|b| {
            format!(
                "{}\t{}\t{}\t{:.4}\t{}\t{}\t{}",
                b.ts,
                b.mark.block,
                b.mark.chr,
                b.progress,
                b.title.replace(['\t', '\n'], " "),
                b.author.as_deref().unwrap_or("").replace(['\t', '\n'], " "),
                b.path.display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn decode(raw: &str) -> Vec<BookEntry> {
    raw.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(7, '\t').collect();
            let [ts, block, chr, progress, title, author, path] = f.as_slice() else {
                return None;
            };
            if path.is_empty() {
                return None;
            }
            Some(BookEntry {
                path: PathBuf::from(path),
                title: title.to_string(),
                author: (!author.is_empty()).then(|| author.to_string()),
                mark: Mark {
                    block: block.parse().ok()?,
                    chr: chr.parse().ok()?,
                },
                progress: progress.parse().unwrap_or(0.0),
                ts: ts.parse().unwrap_or(0),
            })
        })
        .collect()
}

impl ViewerApp {
    /// Opens a book in a document tab (or switches to it if it's open).
    pub(super) fn open_book(&mut self, ctx: &egui::Context, path: &Path) {
        if let Some(i) = self.tabs.iter().position(|t| same_path(&t.path, path)) {
            self.activate_tab(ctx, i);
            return;
        }
        self.stash_active_tab();
        let mark = self
            .books
            .iter()
            .find(|b| same_path(&b.path, path))
            .map(|b| b.mark)
            .unwrap_or_default();
        let view = BookView::open(path, mark, self.book_spread, self.book_font);
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{name} — PaperView")));
        self.tabs.push(Tab {
            title: name,
            path: path.to_path_buf(),
            state: None,
            book: None,
        });
        self.active_tab = Some(self.tabs.len() - 1);
        self.book = Some(view);
        self.add_book(path);
    }

    /// Puts a book on the Books shelf (front), without opening it.
    pub(super) fn add_book(&mut self, path: &Path) {
        let existing = self.books.iter().position(|b| same_path(&b.path, path));
        let mut entry = match existing {
            Some(i) => self.books.remove(i),
            None => BookEntry {
                path: path.to_path_buf(),
                title: path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                author: None,
                mark: Mark::default(),
                progress: 0.0,
                ts: 0,
            },
        };
        entry.ts = now_ts();
        self.books.insert(0, entry);
    }

    /// Writes the open book's position (and what it learnt about its title
    /// and author) into its shelf entry.
    pub(super) fn save_book_progress(&mut self) {
        let Some(view) = &self.book else { return };
        let (path, mark, progress) = (view.path.clone(), view.mark, view.progress());
        let meta = view.book().map(|b| (b.title.clone(), b.author.clone()));
        if let Some(entry) = self.books.iter_mut().find(|b| same_path(&b.path, &path)) {
            entry.mark = mark;
            entry.progress = progress;
            if let Some((title, author)) = meta {
                if !title.trim().is_empty() {
                    entry.title = title;
                }
                if author.is_some() {
                    entry.author = author;
                }
            }
        }
    }

    /// The "Add books" picker on the Books tab.
    pub(super) fn pick_books(&mut self) {
        let exts: Vec<&str> = crate::ebook::EXTENSIONS.to_vec();
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Books", &exts)
            .pick_files()
        {
            let n = paths.len();
            for p in paths.iter().rev() {
                self.add_book(p);
            }
            self.toast(format!("Added {n} book{}", if n == 1 { "" } else { "s" }));
        }
    }
}

impl ViewerApp {
    pub(super) fn set_book_font(&mut self, size: f32) {
        self.book_font = size.clamp(super::reader::FONT_MIN, super::reader::FONT_MAX);
        if let Some(v) = &mut self.book {
            v.set_font(self.book_font);
        }
    }

    pub(super) fn set_book_spread(&mut self, spread: bool) {
        self.book_spread = spread;
        if let Some(v) = &mut self.book {
            v.set_spread(spread);
        }
    }

    /// The reader's bottom bar: pages and chapter on the left; two pages or
    /// one, and the type size, on the right.
    pub(super) fn book_bar(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        egui::Panel::bottom("bookbar")
            .frame(egui::Frame::NONE.fill(p.background))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                w::hairline(ui);
                let full = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(full, 42.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_min_height(42.0);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.add_space(10.0);
                        let glyph = |ui: &mut egui::Ui, g: &str| {
                            w::button(
                                ui,
                                RichText::new(g).size(17.0),
                                w::Btn::new(w::Kind::Quiet).min(32.0, 32.0),
                            )
                        };
                        let (label, chapter, comic) = match &self.book {
                            Some(v) => (v.page_label(), v.chapter_title(), v.is_comic()),
                            None => (String::new(), None, false),
                        };
                        if glyph(ui, "‹")
                            .on_hover_text("Previous page (←, PgUp)")
                            .clicked()
                        {
                            if let Some(v) = &mut self.book {
                                v.prev();
                            }
                        }
                        ui.label(RichText::new(label).color(p.ink));
                        if glyph(ui, "›")
                            .on_hover_text("Next page (→, PgDn, Space)")
                            .clicked()
                        {
                            if let Some(v) = &mut self.book {
                                v.next();
                            }
                        }
                        if let Some(ch) = chapter {
                            ui.add_space(12.0);
                            ui.add(egui::Label::new(RichText::new(ch).color(p.muted)).truncate());
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(10.0);
                            if !comic {
                                if glyph(ui, "A+")
                                    .on_hover_text("Larger text (Ctrl+=)")
                                    .clicked()
                                {
                                    self.set_book_font(self.book_font + 1.0);
                                }
                                if glyph(ui, "A−")
                                    .on_hover_text("Smaller text (Ctrl+-)")
                                    .clicked()
                                {
                                    self.set_book_font(self.book_font - 1.0);
                                }
                                ui.add_space(14.0);
                            }
                            let one = w::tool_button(ui, "One page", !self.book_spread);
                            let two = w::tool_button(ui, "Two pages", self.book_spread);
                            if one.clicked() {
                                self.set_book_spread(false);
                            }
                            if two.clicked() {
                                self.set_book_spread(true);
                            }
                        });
                    },
                );
            });
        self.save_book_progress();
    }

    /// Contents for the open book, sliding in like the PDF sidebar.
    pub(super) fn book_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        let mut open = self.sidebar != SidebarMode::Hidden;
        let chapters = self.book.as_ref().map(|v| v.chapters()).unwrap_or_default();
        let mut jump: Option<usize> = None;
        egui::Panel::left("book-sidebar")
            .exact_size(260.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(p.background))
            .show_separator_line(false)
            .show_collapsible(ui, &mut open, |ui| {
                let r = ui.max_rect();
                ui.painter()
                    .vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, p.hairline));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("CONTENTS")
                            .text_style(theme::label())
                            .extra_letter_spacing(1.2)
                            .color(p.muted),
                    );
                });
                ui.add_space(6.0);
                if chapters.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new("This book has no chapter list.")
                                .small()
                                .color(p.muted),
                        );
                    });
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (title, block, current) in &chapters {
                            let width = ui.available_width();
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(width - 6.0, 30.0), Sense::click());
                            let t = w::hover_t(ui, &resp, true);
                            if *current {
                                ui.painter().rect_filled(rect, 4, p.accent_wash());
                            } else if t > 0.0 {
                                ui.painter()
                                    .rect_filled(rect, 4, p.hover().gamma_multiply(t));
                            }
                            let font = egui::FontId::new(
                                13.5,
                                if *current {
                                    theme::medium()
                                } else {
                                    egui::FontFamily::Proportional
                                },
                            );
                            let color = if *current { p.accent } else { p.ink };
                            let galley =
                                ui.painter()
                                    .layout(title.clone(), font, color, width - 40.0);
                            let pos = egui::pos2(
                                rect.left() + 16.0,
                                rect.center().y - galley.size().y.min(18.0) / 2.0,
                            );
                            ui.painter().with_clip_rect(rect).galley(pos, galley, color);
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if resp.on_hover_text(title).clicked() {
                                jump = Some(*block);
                            }
                        }
                    });
            });
        if let (Some(block), Some(v)) = (jump, &mut self.book) {
            v.goto_block(block);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_round_trips() {
        let books = vec![BookEntry {
            path: PathBuf::from("/b/Pride and Prejudice.epub"),
            title: "Pride and Prejudice".into(),
            author: Some("Jane Austen".into()),
            mark: Mark {
                block: 812,
                chr: 40,
            },
            progress: 0.3214,
            ts: 1_763_000_000,
        }];
        assert_eq!(decode(&encode(&books)), books);
    }
}
