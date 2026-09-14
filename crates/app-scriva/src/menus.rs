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

use wp_model::prop::Justify;
use wp_model::units::{HalfPoint, Line240, Twips};

use crate::app::{Command, Scriva};
use crate::commands::shortcut;
use crate::toolbar::{HIGHLIGHTS, PALETTE, SIZES};

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
        let in_band = self.editing_band();
        let (has_header, has_footer) = self.has_bands();
        let face = self.face_at();
        let size = self.size_at();

        menu::bar(ui, |ui| {
            let mut chosen = None;

            menu::top(ui, "&File", |ui| {
                if menu::item(ui, "&New", shortcut(&Command::New)).clicked() {
                    chosen = Some(Command::New);
                }
                if menu::item(ui, "&Open…", shortcut(&Command::Open)).clicked() {
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
                if menu::item(ui, "&Save", shortcut(&Command::Save)).clicked() {
                    chosen = Some(Command::Save);
                }
                if menu::item(ui, "Save &As…", shortcut(&Command::SaveAs)).clicked() {
                    chosen = Some(Command::SaveAs);
                }
                menu::sep(ui);
                if menu::item(ui, "&Print…", shortcut(&Command::Print)).clicked() {
                    chosen = Some(Command::Print);
                }
                if menu::item(ui, "Export as P&DF…", "").clicked() {
                    chosen = Some(Command::ExportPdf);
                }
                menu::sep(ui);
                if menu::item(ui, "&Close", shortcut(&Command::Close)).clicked() {
                    chosen = Some(Command::Close);
                }
                if menu::item(ui, "E&xit", shortcut(&Command::Exit)).clicked() {
                    chosen = Some(Command::Exit);
                }
            });

            menu::top(ui, "&Edit", |ui| {
                ui.add_enabled_ui(undo, |ui| {
                    if menu::item(ui, "&Undo", shortcut(&Command::Undo)).clicked() {
                        chosen = Some(Command::Undo);
                    }
                });
                ui.add_enabled_ui(redo, |ui| {
                    if menu::item(ui, "&Redo", shortcut(&Command::Redo)).clicked() {
                        chosen = Some(Command::Redo);
                    }
                });
                menu::sep(ui);
                ui.add_enabled_ui(selected, |ui| {
                    if menu::item(ui, "Cu&t", shortcut(&Command::Cut)).clicked() {
                        chosen = Some(Command::Cut);
                    }
                    if menu::item(ui, "&Copy", shortcut(&Command::Copy)).clicked() {
                        chosen = Some(Command::Copy);
                    }
                });
                if menu::item(ui, "&Paste", shortcut(&Command::Paste)).clicked() {
                    chosen = Some(Command::Paste);
                }
                menu::sep(ui);
                if menu::item(ui, "&Find…", shortcut(&Command::Find)).clicked() {
                    chosen = Some(Command::Find);
                }
                if menu::item(ui, "R&eplace…", shortcut(&Command::Replace)).clicked() {
                    chosen = Some(Command::Replace);
                }
                menu::sep(ui);
                if menu::item(ui, "Select &All", shortcut(&Command::SelectAll)).clicked() {
                    chosen = Some(Command::SelectAll);
                }
            });

            menu::top(ui, "&View", |ui| {
                menu::sub(ui, "&Zoom", |ui| {
                    // Six levels and five digits among them, so one goes
                    // without a letter: two rows that shared the 1 left
                    // 125% and 150% unreachable from the keyboard.
                    for (percent, label) in [
                        (50, "&50%"),
                        (75, "&75%"),
                        (100, "&100%"),
                        (125, "1&25%"),
                        (150, "150%"),
                        (200, "20&0%"),
                    ] {
                        let on = (zoom * 100.0).round() as i32 == percent;
                        if menu::check(ui, label, "", on).clicked() {
                            chosen = Some(Command::Zoom(percent as f64 / 100.0));
                        }
                    }
                });
                menu::sep(ui);
                // Word's own menu bar kept this here, and it is the right
                // place: opening the header is a change of what is being
                // looked at and edited, not something inserted.
                if menu::check(ui, "&Header and Footer", "", in_band).clicked() {
                    chosen = Some(match in_band {
                        true => Command::CloseChrome,
                        false => Command::EditHeader,
                    });
                }
                menu::sep(ui);
                if menu::check(
                    ui,
                    "Formatting &Marks",
                    shortcut(&Command::ShowMarks),
                    marks,
                )
                .clicked()
                {
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
                if menu::item(ui, "&Bold", shortcut(&Command::Bold)).clicked() {
                    chosen = Some(Command::Bold);
                }
                if menu::item(ui, "&Italic", shortcut(&Command::Italic)).clicked() {
                    chosen = Some(Command::Italic);
                }
                if menu::item(ui, "&Underline", shortcut(&Command::Underline)).clicked() {
                    chosen = Some(Command::Underline);
                }
                if menu::item(ui, "Strike&through", "").clicked() {
                    chosen = Some(Command::Strike);
                }
                // Word's menu bar had this at Format ▸ Background ▸ Printed
                // Watermark; there is no background submenu here to hang it
                // under, and a watermark is a thing the page is formatted
                // with rather than a thing inserted into the text.
                menu::sep(ui);
                if menu::item(ui, "&Watermark…", "").clicked() {
                    chosen = Some(Command::Watermark);
                }
                menu::sep(ui);
                if menu::item(ui, "Su&perscript", shortcut(&Command::Superscript)).clicked() {
                    chosen = Some(Command::Superscript);
                }
                if menu::item(ui, "Subsc&ript", shortcut(&Command::Subscript)).clicked() {
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
                                let on = face.as_deref() == Some(name);
                                if menu::check(ui, name, "", on).clicked() {
                                    chosen = Some(Command::Font(name.to_owned()));
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
                        let on = size == Some(HalfPoint(half));
                        if menu::check(ui, &label, "", on).clicked() {
                            chosen = Some(Command::Size(HalfPoint(half)));
                        }
                    }
                });
                if menu::item(ui, "&Grow", shortcut(&Command::Grow)).clicked() {
                    chosen = Some(Command::Grow);
                }
                if menu::item(ui, "S&hrink", shortcut(&Command::Shrink)).clicked() {
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
                menu::sub(ui, "High&light", |ui| {
                    if menu::item(ui, "&None", "").clicked() {
                        chosen = Some(Command::Highlight(wp_model::Highlight::None));
                    }
                    for (name, value, _) in HIGHLIGHTS {
                        if menu::item(ui, name, "").clicked() {
                            chosen = Some(Command::Highlight(value));
                        }
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "&Clear Formatting", shortcut(&Command::ClearFormatting))
                    .clicked()
                {
                    chosen = Some(Command::ClearFormatting);
                }
                menu::sep(ui);
                // For the selected picture or chart. Dragging a handle is the
                // fast way; this is the one with numbers in it.
                if menu::item(ui, "Picture Si&ze…", "").clicked() {
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
                if menu::item(ui, "Align &Left", shortcut(&Command::Align(Justify::Start)))
                    .clicked()
                {
                    chosen = Some(Command::Align(Justify::Start));
                }
                if menu::item(ui, "&Centre", shortcut(&Command::Align(Justify::Center))).clicked() {
                    chosen = Some(Command::Align(Justify::Center));
                }
                if menu::item(ui, "Align &Right", shortcut(&Command::Align(Justify::End))).clicked()
                {
                    chosen = Some(Command::Align(Justify::End));
                }
                if menu::item(ui, "&Justify", shortcut(&Command::Align(Justify::Both))).clicked() {
                    chosen = Some(Command::Align(Justify::Both));
                }
                menu::sep(ui);
                menu::sub(ui, "Line &Spacing", |ui| {
                    if menu::item(
                        ui,
                        "&Single",
                        shortcut(&Command::LineSpacing(Line240::SINGLE)),
                    )
                    .clicked()
                    {
                        chosen = Some(Command::LineSpacing(Line240::SINGLE));
                    }
                    if menu::item(
                        ui,
                        "&1.5 Lines",
                        shortcut(&Command::LineSpacing(Line240::ONE_AND_A_HALF)),
                    )
                    .clicked()
                    {
                        chosen = Some(Command::LineSpacing(Line240::ONE_AND_A_HALF));
                    }
                    if menu::item(
                        ui,
                        "&Double",
                        shortcut(&Command::LineSpacing(Line240::DOUBLE)),
                    )
                    .clicked()
                    {
                        chosen = Some(Command::LineSpacing(Line240::DOUBLE));
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "&Increase Indent", shortcut(&Command::Indent(1))).clicked() {
                    chosen = Some(Command::Indent(1));
                }
                if menu::item(ui, "&Decrease Indent", shortcut(&Command::Indent(-1))).clicked() {
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
                if menu::item(ui, "Page &Break", shortcut(&Command::PageBreak)).clicked() {
                    chosen = Some(Command::PageBreak);
                }
            });

            menu::top(ui, "&Review", |ui| {
                if menu::check(
                    ui,
                    "&Track Changes",
                    shortcut(&Command::TrackChanges),
                    tracking,
                )
                .clicked()
                {
                    chosen = Some(Command::TrackChanges);
                }
                menu::sep(ui);
                if menu::item(ui, "&Next Change", shortcut(&Command::NextChange)).clicked() {
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
                if menu::item(ui, "New &Comment", shortcut(&Command::AddComment)).clicked() {
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
                if menu::item(
                    ui,
                    "&Update Table of Contents",
                    shortcut(&Command::UpdateToc),
                )
                .clicked()
                {
                    chosen = Some(Command::UpdateToc);
                }
                menu::sep(ui);
                // Edit makes the band if the document has none and puts the
                // caret in it. There is nothing to fill in first: what goes in
                // a header is typed into the header.
                menu::sub(ui, "&Header", |ui| {
                    if menu::item(ui, "&Edit Header", "").clicked() {
                        chosen = Some(Command::EditHeader);
                    }
                    ui.add_enabled_ui(has_header, |ui| {
                        if menu::item(ui, "&Remove Header", "").clicked() {
                            chosen = Some(Command::RemoveChrome { footer: false });
                        }
                    });
                });
                menu::sub(ui, "&Footer", |ui| {
                    if menu::item(ui, "&Edit Footer", "").clicked() {
                        chosen = Some(Command::EditFooter);
                    }
                    ui.add_enabled_ui(has_footer, |ui| {
                        if menu::item(ui, "&Remove Footer", "").clicked() {
                            chosen = Some(Command::RemoveChrome { footer: true });
                        }
                    });
                });
                menu::sub(ui, "Page &Number", |ui| {
                    if menu::item(ui, "&Plain Number", "").clicked() {
                        chosen = Some(Command::InsertPageNumber { of_pages: false });
                    }
                    if menu::item(ui, "Page &X of Y", "").clicked() {
                        chosen = Some(Command::InsertPageNumber { of_pages: true });
                    }
                });
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
                if menu::item(ui, "Cell &Margins…", "").clicked() {
                    chosen = Some(Command::CellMargins);
                }
                if menu::item(ui, "Mer&ge Cells", "").clicked() {
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
                                chosen =
                                    Some(Command::GoTo(wp_model::Scope::Body, heading.paragraph));
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
                                chosen =
                                    Some(Command::GoTo(wp_model::Scope::Body, bookmark.paragraph));
                            }
                        }
                    }
                });
            });
        chosen
    }
}

/// Which flow something is in, for a list that shows more than one of them.
/// `None` for the text, which needs no saying.
fn flow_name(document: &wp_model::Document, scope: wp_model::Scope) -> Option<&'static str> {
    let wp_model::Scope::Chrome(id) = scope else {
        return None;
    };
    Some(match document.header(id)?.footer {
        true => "footer",
        false => "header",
    })
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
                                    "{} — {}{}",
                                    change.mark.author,
                                    change.what,
                                    // Which flow, when it is not the text: two
                                    // entries that read the same and settle
                                    // different pages are two entries nobody
                                    // can act on.
                                    match flow_name(self.document_ref(), change.scope) {
                                        Some(name) => format!(", in the {name}"),
                                        None => String::new(),
                                    }
                                ))
                                .strong(),
                            );
                            if !change.text.is_empty() {
                                ui.label(&change.text);
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button("Go to").clicked() {
                                    chosen = Some(Command::GoTo(change.scope, change.paragraph));
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
                                    if let Some((scope, at)) =
                                        crate::revise::comment_at(self.document_ref(), *id)
                                    {
                                        chosen = Some(Command::GoTo(scope, at.paragraph));
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
