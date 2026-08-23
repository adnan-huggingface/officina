//! The menu bar and the formatting row.
//!
//! Every row returns a [`Command`] rather than doing anything, so the menu, the
//! toolbar and the keyboard all arrive at `Scriva::run` and cannot answer the
//! same command differently.
//!
//! **Nothing inside a menu may ask for `ui.available_width()`.** A popup
//! measures itself in a pass where that is the width of the screen, which is
//! what made Calx's first File menu seven hundred points wide. `menu::sep`
//! guards it; anything added here must too.

use ui_kit::{egui, menu};

use crate::icons::{self, Icon};
use wp_model::prop::Justify;
use wp_model::units::{HalfPoint, Line240, Twips};

use crate::app::{Command, Scriva};

/// Font sizes the size box offers — Word's own list, in half-points because
/// the list has 10.5 in it and a whole-point list cannot say so.
const SIZES: [i32; 17] = [
    16, 18, 20, 21, 22, 24, 28, 32, 36, 40, 44, 48, 56, 64, 72, 96, 144,
];

/// Faces the font menu offers: the classic trio the generic families always
/// resolve, and the names in `ui-kit`'s exact-face table — so what the menu
/// promises is a face the screen can actually draw, or at worst substitute
/// the way Word would. The symbol-encoded faces stay out; they are for list
/// bullets, not for prose.
const FAMILIES: [&str; 27] = [
    "Arial",
    "Arial Narrow",
    "Book Antiqua",
    "Bookman Old Style",
    "Calibri",
    "Candara",
    "Century Gothic",
    "Comic Sans MS",
    "Constantia",
    "Corbel",
    "Courier New",
    "DejaVu Sans",
    "Franklin Gothic Medium",
    "Garamond",
    "Georgia",
    "Impact",
    "Liberation Sans",
    "Liberation Serif",
    "Lucida Console",
    "Lucida Sans Unicode",
    "Open Sans",
    "Palatino Linotype",
    "Segoe UI",
    "Tahoma",
    "Times New Roman",
    "Trebuchet MS",
    "Verdana",
];

/// Word's standard-colours row, under Word's own names.
const PALETTE: [(&str, [u8; 3]); 13] = [
    ("Black", [0x00, 0x00, 0x00]),
    ("Dark Red", [0xC0, 0x00, 0x00]),
    ("Red", [0xFF, 0x00, 0x00]),
    ("Orange", [0xFF, 0xC0, 0x00]),
    ("Yellow", [0xFF, 0xFF, 0x00]),
    ("Light Green", [0x92, 0xD0, 0x50]),
    ("Green", [0x00, 0xB0, 0x50]),
    ("Light Blue", [0x00, 0xB0, 0xF0]),
    ("Blue", [0x00, 0x70, 0xC0]),
    ("Dark Blue", [0x00, 0x20, 0x60]),
    ("Purple", [0x70, 0x30, 0xA0]),
    ("Gray", [0x80, 0x80, 0x80]),
    ("White", [0xFF, 0xFF, 0xFF]),
];

/// The marker-pen palette in Word's gallery order, under the names Word's
/// tooltips use — which are not the names the attribute values use.
const HIGHLIGHTS: [(&str, wp_model::Highlight); 15] = [
    ("Yellow", wp_model::Highlight::Yellow),
    ("Bright Green", wp_model::Highlight::Green),
    ("Turquoise", wp_model::Highlight::Cyan),
    ("Pink", wp_model::Highlight::Magenta),
    ("Blue", wp_model::Highlight::Blue),
    ("Red", wp_model::Highlight::Red),
    ("Dark Blue", wp_model::Highlight::DarkBlue),
    ("Teal", wp_model::Highlight::DarkCyan),
    ("Green", wp_model::Highlight::DarkGreen),
    ("Violet", wp_model::Highlight::DarkMagenta),
    ("Dark Red", wp_model::Highlight::DarkRed),
    ("Dark Yellow", wp_model::Highlight::DarkYellow),
    ("Gray 50%", wp_model::Highlight::DarkGray),
    ("Gray 25%", wp_model::Highlight::LightGray),
    ("Black", wp_model::Highlight::Black),
];

impl Scriva {
    pub(crate) fn menus(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        // Everything the menus need, read before the bar is drawn: a menu
        // closure cannot borrow `self` while `self` is drawing it.
        let recent: Vec<std::path::PathBuf> = self.recent_paths();
        let (undo, redo) = self.can_undo_redo();
        let selected = self.has_selection();
        let marks = self.showing_marks();
        let revisions = self.showing_revisions();
        let zoom = self.zoom();
        let styles = self.quick_styles();
        let navigator = self.showing_navigator();
        let (tracking, reviewer) = self.reviewing();
        let (orientation, paper, margins) = self.page_setup();

        menu::bar(ui, |ui| {
            let mut chosen = None;

            menu::top(ui, "&File", |ui| {
                if menu::item(ui, "&New", "Ctrl+N").clicked() {
                    chosen = Some(Command::New);
                }
                if menu::item(ui, "&Open…", "Ctrl+O").clicked() {
                    chosen = Some(Command::Open);
                }
                menu::sub(ui, "&Recent", |ui| {
                    for (index, path) in recent.iter().enumerate().take(9) {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        if menu::item(ui, &format!("&{} {name}", index + 1), "").clicked() {
                            chosen = Some(Command::Reopen(path.clone()));
                        }
                    }
                    if recent.is_empty() {
                        ui.add_enabled(false, egui::Button::new("No recent documents"));
                    } else {
                        menu::sep(ui);
                        if menu::item(ui, "C&lear List", "").clicked() {
                            chosen = Some(Command::ForgetRecent);
                        }
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "&Save", "Ctrl+S").clicked() {
                    chosen = Some(Command::Save);
                }
                if menu::item(ui, "Save &As…", "Ctrl+Shift+S").clicked() {
                    chosen = Some(Command::SaveAs);
                }
                menu::sep(ui);
                if menu::item(ui, "&Print…", "Ctrl+P").clicked() {
                    chosen = Some(Command::Print);
                }
                if menu::item(ui, "Export as P&DF…", "").clicked() {
                    chosen = Some(Command::ExportPdf);
                }
                menu::sep(ui);
                if menu::item(ui, "&Close", "Ctrl+W").clicked() {
                    chosen = Some(Command::Close);
                }
                if menu::item(ui, "E&xit", "Alt+F4").clicked() {
                    chosen = Some(Command::Exit);
                }
            });

            menu::top(ui, "&Edit", |ui| {
                ui.add_enabled_ui(undo, |ui| {
                    if menu::item(ui, "&Undo", "Ctrl+Z").clicked() {
                        chosen = Some(Command::Undo);
                    }
                });
                ui.add_enabled_ui(redo, |ui| {
                    if menu::item(ui, "&Redo", "Ctrl+Y").clicked() {
                        chosen = Some(Command::Redo);
                    }
                });
                menu::sep(ui);
                ui.add_enabled_ui(selected, |ui| {
                    if menu::item(ui, "Cu&t", "Ctrl+X").clicked() {
                        chosen = Some(Command::Cut);
                    }
                    if menu::item(ui, "&Copy", "Ctrl+C").clicked() {
                        chosen = Some(Command::Copy);
                    }
                });
                if menu::item(ui, "&Paste", "Ctrl+V").clicked() {
                    chosen = Some(Command::Paste);
                }
                menu::sep(ui);
                if menu::item(ui, "&Find…", "Ctrl+F").clicked() {
                    chosen = Some(Command::Find);
                }
                if menu::item(ui, "R&eplace…", "Ctrl+H").clicked() {
                    chosen = Some(Command::Replace);
                }
                menu::sep(ui);
                if menu::item(ui, "Select &All", "Ctrl+A").clicked() {
                    chosen = Some(Command::SelectAll);
                }
            });

            menu::top(ui, "&View", |ui| {
                menu::sub(ui, "&Zoom", |ui| {
                    for percent in [50, 75, 100, 125, 150, 200] {
                        let on = (zoom * 100.0).round() as i32 == percent;
                        if menu::check(ui, &format!("&{percent}%"), "", on).clicked() {
                            chosen = Some(Command::Zoom(percent as f64 / 100.0));
                        }
                    }
                });
                menu::sep(ui);
                if menu::check(ui, "Formatting &Marks", "Ctrl+Shift+8", marks).clicked() {
                    chosen = Some(Command::ShowMarks);
                }
                if menu::check(ui, "Tracked &Changes", "", revisions).clicked() {
                    chosen = Some(Command::ShowRevisions);
                }
                if menu::check(ui, "&Navigation Pane", "", navigator).clicked() {
                    chosen = Some(Command::Navigator);
                }
            });

            menu::top(ui, "F&ormat", |ui| {
                if menu::item(ui, "&Bold", "Ctrl+B").clicked() {
                    chosen = Some(Command::Bold);
                }
                if menu::item(ui, "&Italic", "Ctrl+I").clicked() {
                    chosen = Some(Command::Italic);
                }
                if menu::item(ui, "&Underline", "Ctrl+U").clicked() {
                    chosen = Some(Command::Underline);
                }
                if menu::item(ui, "Strike&through", "").clicked() {
                    chosen = Some(Command::Strike);
                }
                menu::sep(ui);
                if menu::item(ui, "Su&perscript", "Ctrl+Shift+=").clicked() {
                    chosen = Some(Command::Superscript);
                }
                if menu::item(ui, "Su&bscript", "Ctrl+=").clicked() {
                    chosen = Some(Command::Subscript);
                }
                menu::sep(ui);
                menu::sub(ui, "&Font", |ui| {
                    // Twenty-seven faces do not fit a laptop's window, and a
                    // popup taller than the screen loses its tail — Verdana
                    // was unreachable until this scrolled.
                    egui::ScrollArea::vertical()
                        .max_height(340.0)
                        .show(ui, |ui| {
                            for name in FAMILIES {
                                if menu::item(ui, name, "").clicked() {
                                    chosen = Some(Command::Font(name));
                                }
                            }
                        });
                });
                menu::sub(ui, "&Size", |ui| {
                    for half in SIZES {
                        let label = if half % 2 == 0 {
                            format!("{}", half / 2)
                        } else {
                            format!("{}.5", half / 2)
                        };
                        if menu::item(ui, &label, "").clicked() {
                            chosen = Some(Command::Size(HalfPoint(half)));
                        }
                    }
                });
                if menu::item(ui, "&Grow", "Ctrl+Shift+>").clicked() {
                    chosen = Some(Command::Grow);
                }
                if menu::item(ui, "S&hrink", "Ctrl+Shift+<").clicked() {
                    chosen = Some(Command::Shrink);
                }
                menu::sep(ui);
                menu::sub(ui, "Text C&olour", |ui| {
                    if menu::item(ui, "&Automatic", "").clicked() {
                        chosen = Some(Command::Color(wp_model::Color::Auto));
                    }
                    for (name, rgb) in PALETTE {
                        if menu::item(ui, name, "").clicked() {
                            chosen = Some(Command::Color(wp_model::Color::Rgb(rgb)));
                        }
                    }
                    menu::sep(ui);
                    if menu::item(ui, "&Other…", "").clicked() {
                        chosen = Some(Command::CustomColor);
                    }
                });
                menu::sub(ui, "Highlig&ht", |ui| {
                    if menu::item(ui, "&None", "").clicked() {
                        chosen = Some(Command::Highlight(wp_model::Highlight::None));
                    }
                    for (name, value) in HIGHLIGHTS {
                        if menu::item(ui, name, "").clicked() {
                            chosen = Some(Command::Highlight(value));
                        }
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "&Clear Formatting", "Ctrl+Space").clicked() {
                    chosen = Some(Command::ClearFormatting);
                }
                menu::sep(ui);
                // For the selected picture or chart. Dragging a handle is the
                // fast way; this is the one with numbers in it.
                if menu::item(ui, "P&icture Size…", "").clicked() {
                    chosen = Some(Command::PictureSize);
                }
            });

            menu::top(ui, "&Paragraph", |ui| {
                if menu::item(ui, "&Bullets", "").clicked() {
                    chosen = Some(Command::Bullets);
                }
                if menu::item(ui, "&Numbering", "").clicked() {
                    chosen = Some(Command::Numbers);
                }
                menu::sep(ui);
                if menu::item(ui, "Align &Left", "Ctrl+L").clicked() {
                    chosen = Some(Command::Align(Justify::Start));
                }
                if menu::item(ui, "&Centre", "Ctrl+E").clicked() {
                    chosen = Some(Command::Align(Justify::Center));
                }
                if menu::item(ui, "Align &Right", "Ctrl+R").clicked() {
                    chosen = Some(Command::Align(Justify::End));
                }
                if menu::item(ui, "&Justify", "Ctrl+J").clicked() {
                    chosen = Some(Command::Align(Justify::Both));
                }
                menu::sep(ui);
                menu::sub(ui, "Line &Spacing", |ui| {
                    if menu::item(ui, "&Single", "Ctrl+1").clicked() {
                        chosen = Some(Command::LineSpacing(Line240::SINGLE));
                    }
                    if menu::item(ui, "&1.5 Lines", "Ctrl+5").clicked() {
                        chosen = Some(Command::LineSpacing(Line240::ONE_AND_A_HALF));
                    }
                    if menu::item(ui, "&Double", "Ctrl+2").clicked() {
                        chosen = Some(Command::LineSpacing(Line240::DOUBLE));
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "&Increase Indent", "Ctrl+M").clicked() {
                    chosen = Some(Command::Indent(1));
                }
                if menu::item(ui, "&Decrease Indent", "Ctrl+Shift+M").clicked() {
                    chosen = Some(Command::Indent(-1));
                }
                menu::sep(ui);
                if menu::item(ui, "&Paragraph…", "").clicked() {
                    chosen = Some(Command::ParagraphDialog);
                }
            });

            menu::top(ui, "&Layout", |ui| {
                menu::sub(ui, "&Margins", |ui| {
                    for (name, top, bottom, side) in [
                        ("&Normal — 1\" all round", 1440, 1440, 1440),
                        ("N&arrow — ½\" all round", 720, 720, 720),
                        ("M&oderate — 1\" × ¾\"", 1440, 1440, 1080),
                        ("&Wide — 1\" × 2\"", 1440, 1440, 2880),
                    ] {
                        let ticked = margins.top == Twips(top)
                            && margins.bottom == Twips(bottom)
                            && margins.start == Twips(side)
                            && margins.end == Twips(side);
                        if menu::check(ui, name, "", ticked).clicked() {
                            chosen = Some(Command::Margins(wp_model::PageMargins {
                                top: Twips(top),
                                bottom: Twips(bottom),
                                start: Twips(side),
                                end: Twips(side),
                                ..margins
                            }));
                        }
                    }
                    menu::sep(ui);
                    if menu::item(ui, "&Custom Margins…", "").clicked() {
                        chosen = Some(Command::CustomMargins);
                    }
                });
                menu::sub(ui, "&Orientation", |ui| {
                    let portrait = orientation == wp_model::Orientation::Portrait;
                    if menu::check(ui, "&Portrait", "", portrait).clicked() {
                        chosen = Some(Command::Orient(wp_model::Orientation::Portrait));
                    }
                    if menu::check(ui, "&Landscape", "", !portrait).clicked() {
                        chosen = Some(Command::Orient(wp_model::Orientation::Landscape));
                    }
                });
                menu::sub(ui, "&Size", |ui| {
                    for (name, width, height) in [
                        ("&Letter — 8.5\" × 11\"", 12240, 15840),
                        ("Le&gal — 8.5\" × 14\"", 12240, 20160),
                        ("&A4 — 210 × 297 mm", 11906, 16838),
                    ] {
                        let ticked = paper == (Twips(width), Twips(height));
                        if menu::check(ui, name, "", ticked).clicked() {
                            chosen = Some(Command::Paper(Twips(width), Twips(height)));
                        }
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "Page &Break", "Ctrl+Enter").clicked() {
                    chosen = Some(Command::PageBreak);
                }
            });

            menu::top(ui, "&Review", |ui| {
                if menu::check(ui, "&Track Changes", "Ctrl+Shift+E", tracking).clicked() {
                    chosen = Some(Command::TrackChanges);
                }
                menu::sep(ui);
                if menu::item(ui, "&Next Change", "Alt+F7").clicked() {
                    chosen = Some(Command::NextChange);
                }
                if menu::item(ui, "&Accept", "").clicked() {
                    chosen = Some(Command::AcceptOne);
                }
                if menu::item(ui, "&Reject", "").clicked() {
                    chosen = Some(Command::RejectOne);
                }
                menu::sep(ui);
                if menu::item(ui, "Accept A&ll", "").clicked() {
                    chosen = Some(Command::AcceptAll);
                }
                if menu::item(ui, "Re&ject All", "").clicked() {
                    chosen = Some(Command::RejectAll);
                }
                menu::sep(ui);
                if menu::item(ui, "New &Comment", "Ctrl+Alt+M").clicked() {
                    chosen = Some(Command::AddComment);
                }
                if menu::item(ui, "&Delete Comment", "").clicked() {
                    chosen = Some(Command::DeleteComment);
                }
                if menu::check(ui, "Re&viewing Pane", "", reviewer).clicked() {
                    chosen = Some(Command::Reviewer);
                }
            });

            menu::top(ui, "&Insert", |ui| {
                if menu::item(ui, "&Picture…", "").clicked() {
                    chosen = Some(Command::InsertPicture);
                }
                if menu::item(ui, "&Table…", "").clicked() {
                    chosen = Some(Command::InsertTable);
                }
                if menu::item(ui, "&Update Table of Contents", "F9").clicked() {
                    chosen = Some(Command::UpdateToc);
                }
                menu::sep(ui);
                if menu::item(ui, "&Header…", "").clicked() {
                    chosen = Some(Command::EditHeader);
                }
                if menu::item(ui, "&Footer…", "").clicked() {
                    chosen = Some(Command::EditFooter);
                }
            });

            // Everything here acts on the table the caret is in, and says so
            // when it is not in one.
            menu::top(ui, "T&able", |ui| {
                menu::sub(ui, "&Borders", |ui| {
                    if menu::item(ui, "&All", "").clicked() {
                        chosen = Some(Command::TableBorders(true));
                    }
                    if menu::item(ui, "&None", "").clicked() {
                        chosen = Some(Command::TableBorders(false));
                    }
                });
                menu::sub(ui, "Border &Colour", |ui| {
                    if menu::item(ui, "&Automatic", "").clicked() {
                        chosen = Some(Command::BorderColor(wp_model::Color::Auto));
                    }
                    menu::sep(ui);
                    for (name, rgb) in PALETTE {
                        if menu::item(ui, name, "").clicked() {
                            chosen = Some(Command::BorderColor(wp_model::Color::Rgb(rgb)));
                        }
                    }
                    menu::sep(ui);
                    if menu::item(ui, "&Other…", "").clicked() {
                        chosen = Some(Command::CustomBorderColor);
                    }
                });
                menu::sub(ui, "&Shading", |ui| {
                    if menu::item(ui, "&No Fill", "").clicked() {
                        chosen = Some(Command::TableShading(None));
                    }
                    menu::sep(ui);
                    for (name, rgb) in PALETTE {
                        if menu::item(ui, name, "").clicked() {
                            chosen = Some(Command::TableShading(Some(rgb)));
                        }
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "Column &Width…", "").clicked() {
                    chosen = Some(Command::ColumnWidth);
                }
                if menu::item(ui, "&Merge Cells", "").clicked() {
                    chosen = Some(Command::MergeCells);
                }
            });

            menu::top(ui, "&Styles", |ui| {
                for (id, name) in &styles {
                    if menu::item(ui, name, "").clicked() {
                        chosen = Some(Command::Style(*id));
                    }
                }
                if styles.is_empty() {
                    ui.add_enabled(false, egui::Button::new("No styles in this document"));
                }
            });

            chosen
        })
    }

    /// The row of formatting controls under the menu bar.
    ///
    /// Icons rather than words, and *drawn* icons rather than typed ones — see
    /// `crate::icons`.
    pub(crate) fn format_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut chosen = None;
        let (undo, redo) = self.can_undo_redo();
        let (bold, italic, underline) = self.emphasis();
        let alignment = self.alignment();

        ui.horizontal(|ui| {
            ui.add_enabled_ui(undo, |ui| {
                if icons::button(ui, Icon::Undo, false, "Undo (Ctrl+Z)") {
                    chosen = Some(Command::Undo);
                }
            });
            ui.add_enabled_ui(redo, |ui| {
                if icons::button(ui, Icon::Redo, false, "Redo (Ctrl+Y)") {
                    chosen = Some(Command::Redo);
                }
            });
            ui.separator();

            if icons::emphasis(ui, "B", bold, "Bold (Ctrl+B)") {
                chosen = Some(Command::Bold);
            }
            if icons::emphasis(ui, "I", italic, "Italic (Ctrl+I)") {
                chosen = Some(Command::Italic);
            }
            if icons::emphasis(ui, "U", underline, "Underline (Ctrl+U)") {
                chosen = Some(Command::Underline);
            }
            ui.separator();

            for (icon, justify, tip) in [
                (Icon::AlignLeft, Justify::Start, "Align left (Ctrl+L)"),
                (Icon::AlignCenter, Justify::Center, "Centre (Ctrl+E)"),
                (Icon::AlignRight, Justify::End, "Align right (Ctrl+R)"),
                (Icon::Justify, Justify::Both, "Justify (Ctrl+J)"),
            ] {
                if icons::button(ui, icon, alignment == Some(justify), tip) {
                    chosen = Some(Command::Align(justify));
                }
            }
            ui.separator();

            if icons::button(ui, Icon::Shrink, false, "Shrink (Ctrl+Shift+<)") {
                chosen = Some(Command::Shrink);
            }
            if icons::button(ui, Icon::Grow, false, "Grow (Ctrl+Shift+>)") {
                chosen = Some(Command::Grow);
            }
        });
        chosen
    }
}

impl Scriva {
    /// The pane down the left: the document's headings and its bookmarks.
    ///
    /// Word calls it the navigation pane, and on a document longer than a screen
    /// it is the only way to reach a heading without scrolling for it.
    pub(crate) fn navigation_pane(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let headings = wp_model::outline::headings(self.document_ref());
        let bookmarks: Vec<_> = wp_model::outline::bookmarks(self.document_ref())
            .into_iter()
            .filter(|bookmark| !bookmark.is_internal())
            .collect();
        let mut chosen = None;

        egui::Panel::left("scriva-navigator")
            .default_size(230.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Navigation").strong());
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if headings.is_empty() {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(
                                "No headings. A paragraph becomes one by taking a \
                                 heading style.",
                            )
                            .weak(),
                        );
                    }
                    for heading in &headings {
                        // Indented by level, so the pane reads as an outline
                        // rather than as a list.
                        ui.horizontal(|ui| {
                            ui.add_space((heading.level.saturating_sub(1) as f32) * 12.0);
                            if ui
                                .add(
                                    egui::Button::new(&heading.text)
                                        .frame(false)
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                )
                                .clicked()
                            {
                                chosen = Some(Command::GoTo(heading.paragraph));
                            }
                        });
                    }
                    if !bookmarks.is_empty() {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new("Bookmarks").strong());
                        ui.separator();
                        for bookmark in &bookmarks {
                            if ui
                                .add(egui::Button::new(bookmark.name.as_ref()).frame(false))
                                .clicked()
                            {
                                chosen = Some(Command::GoTo(bookmark.paragraph));
                            }
                        }
                    }
                });
            });
        chosen
    }
}

impl Scriva {
    /// The pane down the right: what has been changed, and what has been said
    /// about it.
    ///
    /// A tracked change the user cannot find is a tracked change they will not
    /// settle, and Word's own reviewing pane exists for exactly that reason.
    pub(crate) fn reviewing_pane(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let changes = crate::revise::tracked(self.document_ref());
        let comments: Vec<(u32, String, String, String)> = self
            .document_ref()
            .comments
            .iter()
            .map(|comment| {
                (
                    comment.id,
                    comment.author.to_string(),
                    comment.text(),
                    if comment.done { "resolved" } else { "" }.to_owned(),
                )
            })
            .collect();
        let mut chosen = None;

        egui::Panel::right("scriva-reviewer")
            .default_size(280.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Reviewing").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Reject all").clicked() {
                            chosen = Some(Command::RejectAll);
                        }
                        if ui.small_button("Accept all").clicked() {
                            chosen = Some(Command::AcceptAll);
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if changes.is_empty() && comments.is_empty() {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Nothing to review.").weak());
                    }
                    for change in &changes {
                        ui.group(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} — {}",
                                    change.mark.author, change.what
                                ))
                                .strong(),
                            );
                            if !change.text.is_empty() {
                                ui.label(&change.text);
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button("Go to").clicked() {
                                    chosen = Some(Command::GoTo(change.paragraph));
                                }
                            });
                        });
                    }
                    for (id, author, text, state) in &comments {
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(author).strong());
                            ui.label(text);
                            if !state.is_empty() {
                                ui.label(egui::RichText::new(state.as_str()).weak());
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button("Go to").clicked() {
                                    if let Some(at) =
                                        crate::revise::comment_at(self.document_ref(), *id)
                                    {
                                        chosen = Some(Command::GoTo(at.paragraph));
                                    }
                                }
                            });
                        });
                    }
                });
            });
        chosen
    }
}
