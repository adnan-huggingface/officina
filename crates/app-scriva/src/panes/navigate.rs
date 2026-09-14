//! The Navigate pane: the document's headings as a tree, and its bookmarks.
//!
//! Word calls it the navigation pane, and on a document longer than a screen
//! it is the only way to reach a heading without scrolling for it. The old
//! pane was a flat list of frameless buttons with a paragraph of grey text
//! when empty: no collapse, no sign of where the caret was, no close, and
//! no way in from the keyboard. This one is a tree with a chevron per heading
//! that has children, a filter above it, the heading containing the caret lit
//! and kept in view, the bookmarks folded away at the end, and rows the
//! arrows walk once F6 has brought the keyboard here.

use ui_kit::{egui, menu, theme};
use wp_model::outline::{self, Heading};

use crate::app::{Command, Keyboard, Scriva};

/// One row as it was drawn: the paragraph it goes to, its label, and whether
/// it is lit — what a test reads instead of a screenshot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowDrawn {
    pub paragraph: Option<usize>,
    pub label: String,
    pub lit: bool,
    pub rect: egui::Rect,
}

fn drawn_id() -> egui::Id {
    egui::Id::new("scriva-navigate-drawn")
}

/// The rows the last frame drew, in order.
#[cfg(test)]
pub(crate) fn drawn(ctx: &egui::Context) -> Vec<RowDrawn> {
    ctx.data(|d| d.get_temp::<Vec<RowDrawn>>(drawn_id()))
        .unwrap_or_default()
}

fn note(ui: &egui::Ui, row: RowDrawn) {
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<RowDrawn>>(drawn_id())
            .push(row)
    });
}

/// One row of the tree, once the filter and the collapsed headings have had
/// their say.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Heading {
        /// Index into the document's headings.
        index: usize,
        children: bool,
        collapsed: bool,
    },
    /// The `Bookmarks (n)` section row.
    Bookmarks {
        count: usize,
        open: bool,
    },
    Bookmark {
        name: String,
        paragraph: usize,
    },
}

/// The rows the pane shows for `headings`: a typed filter flattens the tree
/// to what matches; otherwise a collapsed heading hides every heading under
/// it until the next of its level or higher.
fn rows(
    headings: &[Heading],
    bookmarks: &[(String, usize)],
    filter: &str,
    collapsed: &std::collections::BTreeSet<usize>,
    bookmarks_open: bool,
) -> Vec<Row> {
    let mut out = Vec::new();
    let wanted = filter.trim().to_ascii_lowercase();
    let mut hidden_below: Option<u8> = None;
    for (index, heading) in headings.iter().enumerate() {
        if !wanted.is_empty() {
            if heading.text.to_ascii_lowercase().contains(&wanted) {
                out.push(Row::Heading {
                    index,
                    children: false,
                    collapsed: false,
                });
            }
            continue;
        }
        if let Some(level) = hidden_below {
            if heading.level > level {
                continue;
            }
            hidden_below = None;
        }
        let children = headings
            .get(index + 1)
            .is_some_and(|next| next.level > heading.level);
        let is_collapsed = children && collapsed.contains(&heading.paragraph);
        if is_collapsed {
            hidden_below = Some(heading.level);
        }
        out.push(Row::Heading {
            index,
            children,
            collapsed: is_collapsed,
        });
    }
    if !bookmarks.is_empty() && wanted.is_empty() {
        out.push(Row::Bookmarks {
            count: bookmarks.len(),
            open: bookmarks_open,
        });
        if bookmarks_open {
            for (name, paragraph) in bookmarks {
                out.push(Row::Bookmark {
                    name: name.clone(),
                    paragraph: *paragraph,
                });
            }
        }
    }
    out
}

impl Scriva {
    /// The pane down the left, and what was chosen on it.
    pub(crate) fn navigate_pane(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        ui.ctx()
            .data_mut(|d| d.insert_temp::<Vec<RowDrawn>>(drawn_id(), Vec::new()));
        let headings = outline::headings(self.document_ref());
        let bookmarks: Vec<(String, usize)> = outline::bookmarks(self.document_ref())
            .into_iter()
            .filter(|bookmark| !bookmark.is_internal())
            .map(|bookmark| (bookmark.name.to_string(), bookmark.paragraph))
            .collect();
        let paragraphs = self.document_ref().paragraphs().len();
        let mut filter = self.nav_filter.clone();
        let mut collapsed = self.nav_collapsed.clone();
        let mut bookmarks_open = self.nav_bookmarks_open;
        let mut chosen: Option<Command> = None;
        let mut held = false;

        // The row the caret is under, when the text is what is being edited.
        let caret_row = match self.scope {
            wp_model::Scope::Body => outline::heading_containing(&headings, self.caret().paragraph),
            _ => None,
        };
        let keyboard_here = self.keyboard == Keyboard::Navigate;
        let visible = rows(&headings, &bookmarks, &filter, &collapsed, bookmarks_open);
        let mut row_at = self.nav_row.min(visible.len().saturating_sub(1));

        // The keys, before the rows are drawn, so this frame shows the result.
        if keyboard_here && !visible.is_empty() {
            let (up, down, enter, right, left) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft),
                )
            });
            if up {
                row_at = row_at.saturating_sub(1);
            }
            if down {
                row_at = (row_at + 1).min(visible.len() - 1);
            }
            match visible[row_at].clone() {
                Row::Heading {
                    index, children, ..
                } => {
                    let paragraph = headings[index].paragraph;
                    if right && children {
                        collapsed.remove(&paragraph);
                    }
                    if left && children {
                        collapsed.insert(paragraph);
                    }
                    if enter {
                        chosen = Some(Command::GoTo(wp_model::Scope::Body, paragraph));
                    }
                }
                Row::Bookmarks { .. } => {
                    if right || enter {
                        bookmarks_open = !(enter && bookmarks_open) && (right || enter);
                    }
                    if left {
                        bookmarks_open = false;
                    }
                }
                Row::Bookmark { paragraph, .. } => {
                    if enter {
                        chosen = Some(Command::GoTo(wp_model::Scope::Body, paragraph));
                    }
                }
            }
        }
        // A row lit by the keyboard, or the caret's heading otherwise.
        let lit = if keyboard_here {
            Some(row_at)
        } else {
            caret_row.and_then(|index| {
                visible
                    .iter()
                    .position(|row| matches!(row, Row::Heading { index: at, .. } if *at == index))
            })
        };
        let mut scrolled = self.nav_scrolled;

        egui::Panel::left("scriva-navigator")
            .default_size(260.0)
            .size_range(200.0..=420.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::CHROME))
            .show(ui, |ui| {
                let header = egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.set_min_height(theme::PANE_HEADER - 10.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Navigate").strong().size(theme::TEXT));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(egui::Button::new("×").frame(false))
                                        .on_hover_text("Close the pane")
                                        .clicked()
                                    {
                                        chosen = Some(Command::Navigator);
                                    }
                                },
                            );
                        });
                        // The filter: a magnifier and a field.
                        ui.horizontal(|ui| {
                            let (glyph, _) = ui.allocate_exact_size(
                                egui::vec2(18.0, theme::TARGET.y),
                                egui::Sense::hover(),
                            );
                            crate::icons::draw(
                                ui.painter(),
                                crate::icons::Icon::Find,
                                glyph.center(),
                                theme::INK_SOFT,
                            );
                            let field = ui
                                .scope(|ui| {
                                    ui_kit::dialog::form_style(ui.style_mut());
                                    ui.add(
                                        egui::TextEdit::singleline(&mut filter)
                                            .id(egui::Id::new("scriva-navigate-filter"))
                                            .hint_text("Filter headings")
                                            .desired_width(f32::INFINITY),
                                    )
                                })
                                .inner;
                            held |= field.has_focus();
                        });
                    });
                ui.painter().hline(
                    header.response.rect.x_range(),
                    header.response.rect.bottom() + 0.5,
                    egui::Stroke::new(1.0, theme::CHROME_RULE),
                );

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        if headings.is_empty() && filter.trim().is_empty() {
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.vertical(|ui| {
                                    ui.label("No headings yet.");
                                    ui.label(
                                        egui::RichText::new(
                                            "Give a paragraph a heading style and it appears here.",
                                        )
                                        .color(theme::INK_SOFT)
                                        .size(theme::TEXT_SMALL),
                                    );
                                });
                            });
                        }
                        for (at, row) in visible.iter().enumerate() {
                            let is_lit = lit == Some(at);
                            let (label, indent, chevron, paragraph) = match row {
                                Row::Heading {
                                    index,
                                    children,
                                    collapsed,
                                } => {
                                    let heading = &headings[*index];
                                    (
                                        heading.text.clone(),
                                        14.0 * (heading.level.saturating_sub(1) as f32),
                                        children.then_some(!*collapsed),
                                        Some(heading.paragraph),
                                    )
                                }
                                Row::Bookmarks { count, open } => {
                                    (format!("Bookmarks ({count})"), 0.0, Some(*open), None)
                                }
                                Row::Bookmark { name, paragraph } => {
                                    (name.clone(), 14.0, None, Some(*paragraph))
                                }
                            };
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 24.0),
                                egui::Sense::click(),
                            );
                            if is_lit {
                                ui.painter().rect_filled(rect, 0.0, theme::TINT_ON);
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_max(
                                        rect.min,
                                        egui::pos2(rect.left() + 3.0, rect.bottom()),
                                    ),
                                    0.0,
                                    theme::ACCENT,
                                );
                                if scrolled != Some(at) {
                                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                    scrolled = Some(at);
                                }
                            } else if response.hovered() {
                                ui.painter().rect_filled(rect, 0.0, theme::TINT_HOVER);
                            }
                            let x = rect.left() + 10.0 + indent;
                            // The chevron: a small triangle, down when open,
                            // right when folded; its own click target.
                            let chevron_rect = egui::Rect::from_min_size(
                                egui::pos2(x, rect.top()),
                                egui::vec2(16.0, rect.height()),
                            );
                            if let Some(open) = chevron {
                                let c = chevron_rect.center();
                                let points = if open {
                                    vec![
                                        c + egui::vec2(-4.0, -2.0),
                                        c + egui::vec2(4.0, -2.0),
                                        c + egui::vec2(0.0, 3.0),
                                    ]
                                } else {
                                    vec![
                                        c + egui::vec2(-2.0, -4.0),
                                        c + egui::vec2(3.0, 0.0),
                                        c + egui::vec2(-2.0, 4.0),
                                    ]
                                };
                                ui.painter().add(egui::Shape::convex_polygon(
                                    points,
                                    theme::INK_SOFT,
                                    egui::Stroke::NONE,
                                ));
                            }
                            let text_rect = egui::Rect::from_min_max(
                                egui::pos2(chevron_rect.right() + 2.0, rect.top()),
                                egui::pos2(rect.right() - 6.0, rect.bottom()),
                            );
                            let galley = ui.painter().layout_no_wrap(
                                label.clone(),
                                egui::FontId::proportional(theme::TEXT),
                                if matches!(row, Row::Bookmarks { .. }) {
                                    theme::INK_SOFT
                                } else {
                                    theme::INK
                                },
                            );
                            ui.painter().with_clip_rect(text_rect).galley(
                                egui::pos2(
                                    text_rect.left(),
                                    text_rect.center().y - galley.size().y / 2.0,
                                ),
                                galley,
                                theme::INK,
                            );
                            note(
                                ui,
                                RowDrawn {
                                    paragraph,
                                    label: label.clone(),
                                    lit: is_lit,
                                    rect,
                                },
                            );
                            // A click on the chevron folds; anywhere else goes.
                            let on_chevron = response
                                .interact_pointer_pos()
                                .is_some_and(|p| chevron_rect.contains(p));
                            if response.clicked() {
                                match (row, on_chevron && chevron.is_some()) {
                                    (Row::Heading { index, .. }, true) => {
                                        let paragraph = headings[*index].paragraph;
                                        if !collapsed.remove(&paragraph) {
                                            collapsed.insert(paragraph);
                                        }
                                    }
                                    (Row::Bookmarks { open, .. }, _) => bookmarks_open = !open,
                                    (_, _) => {
                                        if let Some(paragraph) = paragraph {
                                            chosen = Some(Command::GoTo(
                                                wp_model::Scope::Body,
                                                paragraph,
                                            ));
                                        }
                                    }
                                }
                            }
                            if let Row::Heading { index, .. } = row {
                                let heading_index = *index;
                                let paragraph = headings[heading_index].paragraph;
                                let extent =
                                    outline::heading_extent(&headings, heading_index, paragraphs);
                                menu::context(&response, |ui| {
                                    if menu::item(ui, "&Go to", "").clicked() {
                                        chosen =
                                            Some(Command::GoTo(wp_model::Scope::Body, paragraph));
                                    }
                                    if menu::item(ui, "&Select heading and content", "").clicked() {
                                        chosen = Some(Command::SelectParagraphs(
                                            extent.start,
                                            extent.end,
                                        ));
                                    }
                                });
                            }
                        }
                        ui.add_space(6.0);
                    });
            });

        self.nav_filter = filter;
        self.nav_collapsed = collapsed;
        self.nav_bookmarks_open = bookmarks_open;
        self.nav_row = row_at;
        self.nav_scrolled = scrolled;
        self.pane_held |= held;
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heading(paragraph: usize, level: u8, text: &str) -> Heading {
        Heading {
            paragraph,
            level,
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_collapsed_heading_hides_what_is_under_it_and_a_filter_flattens_the_tree() {
        let headings = [
            heading(0, 1, "Intro"),
            heading(2, 2, "Aims"),
            heading(4, 2, "Scope"),
            heading(6, 1, "Method"),
        ];
        let none = std::collections::BTreeSet::new();
        let all = rows(&headings, &[], "", &none, false);
        assert_eq!(all.len(), 4);
        assert!(matches!(all[0], Row::Heading { children: true, .. }));
        assert!(matches!(
            all[1],
            Row::Heading {
                children: false,
                ..
            }
        ));

        let folded: std::collections::BTreeSet<usize> = [0].into_iter().collect();
        let some = rows(&headings, &[], "", &folded, false);
        let shown: Vec<usize> = some
            .iter()
            .filter_map(|row| match row {
                Row::Heading { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(shown, vec![0, 3], "Aims and Scope are folded under Intro");

        let filtered = rows(&headings, &[], "sco", &folded, false);
        assert_eq!(filtered.len(), 1, "the filter finds Scope through the fold");

        let with_marks = rows(&headings, &[("here".to_owned(), 5)], "", &none, false);
        assert!(matches!(
            with_marks.last(),
            Some(Row::Bookmarks {
                count: 1,
                open: false
            })
        ));
        let opened = rows(&headings, &[("here".to_owned(), 5)], "", &none, true);
        assert!(matches!(
            opened.last(),
            Some(Row::Bookmark { paragraph: 5, .. })
        ));
    }
}
