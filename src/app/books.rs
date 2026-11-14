//! Books on the Library's Books tab, and opening one into a document tab.
//! The tab only exists once a book has been added.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, Rect, RichText, Sense, Stroke};

use super::reader::{BookView, Mark, ReadingStyle, FONT_MAX, FONT_MIN};
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
        // A user-chosen file, so one quick check is fine here: a book that
        // isn't there shouldn't land on the shelf.
        if !path.is_file() {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.toast(format!("Couldn't find {name}"));
            return;
        }
        self.stash_active_tab();
        let mark = self
            .books
            .iter()
            .find(|b| same_path(&b.path, path))
            .map(|b| b.mark)
            .unwrap_or_default();
        let view = BookView::open(path, mark, self.book_spread, self.reading);
        // The title the shelf learnt last time, until the book is read again.
        let name = self
            .books
            .iter()
            .find(|b| same_path(&b.path, path))
            .map(|b| b.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            });
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
    /// Applies a reading style to the shelf's default and the open book.
    pub(super) fn set_reading(&mut self, style: ReadingStyle) {
        self.reading = ReadingStyle {
            size: style.size.clamp(FONT_MIN, FONT_MAX),
            ..style
        };
        if let Some(v) = &mut self.book {
            v.set_style(self.reading);
        }
    }

    pub(super) fn bump_book_size(&mut self, delta: f32) {
        self.set_reading(ReadingStyle {
            size: self.reading.size + delta,
            ..self.reading
        });
    }

    pub(super) fn set_book_spread(&mut self, spread: bool) {
        self.book_spread = spread;
        if let Some(v) = &mut self.book {
            v.set_spread(spread);
        }
    }

    /// The reader's bottom bar: the progress line along its top edge; the
    /// page box and the chapter on the left; reading options, full screen and
    /// one or two pages on the right.
    pub(super) fn book_bar(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        egui::Panel::bottom("bookbar")
            .frame(egui::Frame::NONE.fill(p.background))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.progress_rail(ui);
                let full = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(full, 42.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_min_height(42.0);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.add_space(10.0);
                        self.book_page_controls(ui);
                        ui.add_space(14.0);
                        // Right side first, so the chapter gets what's left.
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(10.0);
                            if w::icon_button(ui, "⛶", "Full screen (F11)", self.immersive)
                                .clicked()
                            {
                                let on = !self.immersive;
                                self.set_immersive(ui.ctx(), on);
                            }
                            self.reading_button(ui);
                            ui.add_space(12.0);
                            let one = w::tool_button(ui, "One page", !self.book_spread);
                            let two = w::tool_button(ui, "Two pages", self.book_spread);
                            if one.clicked() {
                                self.set_book_spread(false);
                            }
                            if two.clicked() {
                                self.set_book_spread(true);
                            }
                            ui.add_space(12.0);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                self.chapter_note(ui);
                            });
                        });
                    },
                );
            });
        self.save_book_progress();
    }

    /// "‹ [ 12 ] of 1,532 ›": type a page and press Enter to go there.
    pub(super) fn book_page_controls(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        let Some(v) = &self.book else { return };
        let (now, (total, exact)) = (v.page_now(), v.page_total());
        if super::chrome::step_button(ui, "‹", now > 1)
            .on_hover_text("Previous page (←, PgUp)")
            .clicked()
        {
            if let Some(v) = &mut self.book {
                v.prev();
            }
        }
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.book_page_field)
                .id(egui::Id::new("page-field"))
                .desired_width(44.0)
                .horizontal_align(Align::Center)
                .background_color(p.surface),
        );
        w::select_all_on_focus(ui, &field, &self.book_page_field);
        let field = field.on_hover_text("Type a page number and press Enter (Ctrl+G)");
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let typed = self.book_page_field.trim().replace([',', '.', ' '], "");
            if let (Ok(n), Some(v)) = (typed.parse::<usize>(), &mut self.book) {
                v.goto_page(n.max(1) - 1);
            }
        }
        if !field.has_focus() {
            self.book_page_field = now.to_string();
        }
        ui.add_space(4.0);
        let of = if exact {
            format!("of {}", thousands(total))
        } else {
            format!("of about {}", thousands(total))
        };
        ui.label(RichText::new(of).color(p.muted))
            .on_hover_text(if exact {
                "Pages at this window size and text size"
            } else {
                "Still counting pages…"
            });
        ui.add_space(2.0);
        if super::chrome::step_button(ui, "›", !exact || now < total)
            .on_hover_text("Next page (→, PgDn, Space)")
            .clicked()
        {
            if let Some(v) = &mut self.book {
                v.next();
            }
        }
    }

    /// The chapter, and how far it is to the next one, or the page a jump is
    /// waiting for.
    fn chapter_note(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        let Some(v) = &self.book else { return };
        let mut job = egui::text::LayoutJob::default();
        let fmt = |color, family| egui::TextFormat {
            font_id: egui::FontId::new(13.5, family),
            color,
            ..Default::default()
        };
        if let Some(n) = v.seeking_page() {
            let text = if n == usize::MAX {
                "Finding the last page…".to_string()
            } else {
                format!("Finding page {}…", thousands(n))
            };
            job.append(&text, 0.0, fmt(p.accent, theme::medium()));
        } else {
            let chapter = v.chapter_title();
            if let Some(ch) = &chapter {
                job.append(ch, 0.0, fmt(p.ink, egui::FontFamily::Proportional));
            }
            if let Some(left) = v.pages_left_in_chapter().filter(|_| chapter.is_some()) {
                let text = match left {
                    0 => "last page of the chapter".to_string(),
                    1 => "1 page left in the chapter".to_string(),
                    n => format!("{n} pages left in the chapter"),
                };
                let sep = if job.is_empty() { "" } else { "  ·  " };
                job.append(
                    &format!("{sep}{text}"),
                    0.0,
                    fmt(p.muted, egui::FontFamily::Proportional),
                );
            }
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width().max(0.0));
        ui.label(job);
    }

    /// The thin line along the top of the bar: how far through the book you
    /// are. Click or drag along it to go anywhere.
    fn progress_rail(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        let Some(v) = &self.book else {
            w::hairline(ui);
            return;
        };
        let progress = v.progress();
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 7.0), Sense::click_and_drag());
        let t = ui
            .ctx()
            .animate_bool_with_time(resp.id, resp.hovered() || resp.dragged(), 0.12);
        let line_y = rect.top() + 0.5;
        ui.painter()
            .hline(rect.x_range(), line_y, Stroke::new(1.0, p.hairline));
        let thick = 2.0 + 3.0 * t;
        let x = rect.left() + width * progress;
        ui.painter().rect_filled(
            Rect::from_min_max(
                egui::pos2(rect.left(), rect.top()),
                egui::pos2(x, rect.top() + thick),
            ),
            0,
            p.accent,
        );
        if t > 0.0 {
            ui.painter()
                .circle_filled(egui::pos2(x, rect.top() + thick / 2.0), 5.0 * t, p.accent);
        }
        let pointer_f = resp
            .hover_pos()
            .or(resp.interact_pointer_pos())
            .map(|pos| ((pos.x - rect.left()) / width).clamp(0.0, 1.0));
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if let Some(f) = pointer_f {
            resp.clone().on_hover_text_at_pointer(format!(
                "{:.0}% through · now at {:.0}%",
                f * 100.0,
                progress * 100.0
            ));
        }
        if resp.clicked() || resp.dragged() {
            if let (Some(pos), Some(v)) = (resp.interact_pointer_pos(), &mut self.book) {
                v.goto_fraction(((pos.x - rect.left()) / width).clamp(0.0, 1.0));
            }
        }
    }

    /// "Aa": the reading options in a popover.
    pub(super) fn reading_button(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        let id = egui::Id::new("reading-popover");
        let open = egui::Popup::is_id_open(ui.ctx(), id);
        let resp = w::button(
            ui,
            RichText::new("Aa")
                .family(theme::serif())
                .size(16.0)
                .color(if open { p.accent } else { p.ink }),
            w::Btn::new(if open {
                w::Kind::Selected
            } else {
                w::Kind::Quiet
            })
            .min(36.0, 32.0),
        )
        .on_hover_text("Typeface, size and spacing");
        egui::Popup::from_toggle_button_response(&resp)
            .id(id)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(
                egui::Frame::NONE
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.hairline))
                    .corner_radius(10)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 6],
                        blur: 24,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(if p.dark { 120 } else { 40 }),
                    })
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(|ui| {
                ui.set_width(300.0);
                self.reading_controls(ui, true);
            });
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
                            // One line, cut with "…": a fixed-height row
                            // can't take a second line (the full title shows
                            // on hover).
                            let mut job = egui::text::LayoutJob::simple_singleline(
                                title.clone(),
                                font,
                                color,
                            );
                            job.wrap = egui::text::TextWrapping::truncate_at_width(width - 40.0);
                            let galley = ui.painter().layout_job(job);
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

/// 1532 -> "1,532".
pub(super) fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(700), "700");
        assert_eq!(thousands(1532), "1,532");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

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
