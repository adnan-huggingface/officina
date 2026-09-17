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
        let comments = self.showing_comments();
        let zoom = self.zoom();
        let style = self.style_at();
        let navigator = self.showing_navigator();
        let (tracking, reviewer) = self.reviewing();
        let assisting = self.assisting;
        let (orientation, paper, margins) = self.page_setup();
        let in_band = self.editing_band();
        let (has_header, has_footer) = self.has_bands();
        let face = self.face_at();
        let size = self.size_at();
        let table = self.table_at_caret();
        let colour = self.colour_at();
        let highlight = self.highlight_at();
        let spacing = self.line_spacing_at();
        let picked = self.has_picked();
        let fits = (self.fit_percent(true), self.fit_percent(false));
        let style_faces = self.style_faces(ui.ctx());

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
                    // Ten, with the folder after the name in the soft
                    // column, which is how two files of one name are told
                    // apart; the tenth's letter is its 0.
                    for (index, path) in recent.iter().enumerate().take(10) {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        let folder = path
                            .parent()
                            .and_then(|dir| dir.file_name())
                            .map(|dir| dir.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let label = match index {
                            9 => format!("1&0 {name}"),
                            _ => format!("&{} {name}", index + 1),
                        };
                        if menu::item(ui, &label, &folder).clicked() {
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
                if menu::item(
                    ui,
                    "Paste U&nformatted",
                    shortcut(&Command::PasteUnformatted),
                )
                .clicked()
                {
                    chosen = Some(Command::PasteUnformatted);
                }
                menu::sep(ui);
                if menu::item(ui, "&Find…", shortcut(&Command::Find)).clicked() {
                    chosen = Some(Command::Find);
                }
                if menu::item(ui, "R&eplace…", shortcut(&Command::Replace)).clicked() {
                    chosen = Some(Command::Replace);
                }
                if menu::item(ui, "&Go To…", shortcut(&Command::GoToPage)).clicked() {
                    chosen = Some(Command::GoToPage);
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
                    menu::sep(ui);
                    // The two fits the Zoom box offers, for the keyboard.
                    if let Some(percent) = fits.0 {
                        if menu::item(ui, "Page &Width", "").clicked() {
                            chosen = Some(Command::Zoom(percent as f64 / 100.0));
                        }
                    }
                    if let Some(percent) = fits.1 {
                        if menu::item(ui, "W&hole Page", "").clicked() {
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
                if menu::check(ui, "C&omments", shortcut(&Command::ShowComments), comments)
                    .clicked()
                {
                    chosen = Some(Command::ShowComments);
                }
                if menu::check(ui, "&Navigation Pane", "", navigator).clicked() {
                    chosen = Some(Command::Navigator);
                }
                // A view, so it lives here and not under Review.
                if menu::check(
                    ui,
                    "Re&viewing Pane",
                    shortcut(&Command::Reviewer),
                    reviewer,
                )
                .clicked()
                {
                    chosen = Some(Command::Reviewer);
                }
                // Beside Review, whose place on the right it shares.
                if menu::check(ui, "&Assist", shortcut(&Command::Assist), assisting).clicked() {
                    chosen = Some(match assisting {
                        true => Command::HideAssist,
                        false => Command::Assist,
                    });
                }
            });

            menu::top(ui, "&Insert", |ui| {
                if menu::item(ui, "&Picture…", "").clicked() {
                    chosen = Some(Command::InsertPicture);
                }
                // The grid picker first, as the toolbar's, and the box with
                // numbers under it for a table bigger than eight by eight.
                menu::sub(ui, "&Table", |ui| {
                    if let Some(command) = crate::toolbar::table_rows(ui) {
                        chosen = Some(command);
                    }
                });
                // Inserted, so it is here rather than under Layout.
                if menu::item(ui, "Page &Break", shortcut(&Command::PageBreak)).clicked() {
                    chosen = Some(Command::PageBreak);
                }
                menu::sub(ui, "Page &Number", |ui| {
                    if menu::item(ui, "&Plain Number", "").clicked() {
                        chosen = Some(Command::InsertPageNumber { of_pages: false });
                    }
                    if menu::item(ui, "Page &X of Y", "").clicked() {
                        chosen = Some(Command::InsertPageNumber { of_pages: true });
                    }
                });
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
                menu::sep(ui);
                if menu::item(ui, "&Comment", shortcut(&Command::AddComment)).clicked() {
                    chosen = Some(Command::AddComment);
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
            });

            menu::top(ui, "F&ormat", |ui| {
                if menu::item(ui, "Fo&nt…", shortcut(&Command::FontDialog)).clicked() {
                    chosen = Some(Command::FontDialog);
                }
                menu::sep(ui);
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
                if menu::item(ui, "Su&perscript", shortcut(&Command::Superscript)).clicked() {
                    chosen = Some(Command::Superscript);
                }
                if menu::item(ui, "Subsc&ript", shortcut(&Command::Subscript)).clicked() {
                    chosen = Some(Command::Subscript);
                }
                menu::sep(ui);
                menu::sub(ui, "Text C&olour", |ui| {
                    let colours: Vec<(&str, egui::Color32)> = PALETTE
                        .iter()
                        .map(|(name, [r, g, b])| (*name, egui::Color32::from_rgb(*r, *g, *b)))
                        .collect();
                    let current = match colour {
                        Some(wp_model::Color::Rgb(rgb)) => {
                            PALETTE.iter().position(|(_, other)| *other == rgb)
                        }
                        _ => None,
                    };
                    let picked = match menu::swatches(
                        ui,
                        "&Automatic",
                        &colours,
                        current,
                        Some("&More Colours…"),
                    ) {
                        Some(menu::Swatch::First) => Some(Command::Color(wp_model::Color::Auto)),
                        Some(menu::Swatch::Index(index)) => {
                            Some(Command::Color(wp_model::Color::Rgb(PALETTE[index].1)))
                        }
                        Some(menu::Swatch::More) => Some(Command::CustomColor),
                        None => None,
                    };
                    if picked.is_some() {
                        chosen = picked;
                    }
                });
                menu::sub(ui, "High&light", |ui| {
                    let colours: Vec<(&str, egui::Color32)> = HIGHLIGHTS
                        .iter()
                        .map(|(name, _, [r, g, b])| (*name, egui::Color32::from_rgb(*r, *g, *b)))
                        .collect();
                    let current = highlight
                        .and_then(|h| HIGHLIGHTS.iter().position(|(_, value, _)| *value == h));
                    let picked = match menu::swatches(ui, "&None", &colours, current, None) {
                        Some(menu::Swatch::First) => {
                            Some(Command::Highlight(wp_model::Highlight::None))
                        }
                        Some(menu::Swatch::Index(index)) => {
                            Some(Command::Highlight(HIGHLIGHTS[index].1))
                        }
                        _ => None,
                    };
                    if picked.is_some() {
                        chosen = picked;
                    }
                });
                menu::sep(ui);
                menu::sub(ui, "&Font", |ui| {
                    // Twenty-seven faces do not fit a laptop's window, and a
                    // popup taller than the screen loses its tail — Verdana
                    // was unreachable until this scrolled.
                    ui_kit::scroll::show(
                        ui,
                        egui::ScrollArea::vertical().max_height(340.0),
                        |ui| {
                            for name in FAMILIES {
                                let on = face.as_deref() == Some(name);
                                if menu::check(ui, name, "", on).clicked() {
                                    chosen = Some(Command::Font(name.to_owned()));
                                }
                            }
                        },
                    );
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
                if menu::item(ui, "&Clear Formatting", shortcut(&Command::ClearFormatting))
                    .clicked()
                {
                    chosen = Some(Command::ClearFormatting);
                }
                menu::sep(ui);
                // Word's menu bar had this at Format ▸ Background ▸ Printed
                // Watermark; there is no background submenu here to hang it
                // under, and a watermark is a thing the page is formatted
                // with rather than a thing inserted into the text.
                if menu::item(ui, "&Watermark…", "").clicked() {
                    chosen = Some(Command::Watermark);
                }
                // For the picked picture or chart. Dragging a handle is the
                // fast way; this is the one with numbers in it, and it is
                // disabled until there is a picture for it to be about.
                ui.add_enabled_ui(picked, |ui| {
                    if menu::item(ui, "Picture Si&ze…", "")
                        .on_disabled_hover_text("Click a picture first")
                        .clicked()
                    {
                        chosen = Some(Command::PictureSize);
                    }
                });
            });

            menu::top(ui, "&Paragraph", |ui| {
                if menu::item(ui, "&Bullets", shortcut(&Command::Bullets)).clicked() {
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
                    for (label, value) in [
                        ("&Single", Line240::SINGLE),
                        ("&1.5 Lines", Line240::ONE_AND_A_HALF),
                        ("&Double", Line240::DOUBLE),
                    ] {
                        let command = Command::LineSpacing(value);
                        if menu::check(ui, label, shortcut(&command), spacing == Some(value))
                            .clicked()
                        {
                            chosen = Some(command);
                        }
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
                if menu::item(ui, "&Page Setup…", "").clicked() {
                    chosen = Some(Command::PageSetup);
                }
                menu::sep(ui);
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
            });

            // Everything here acts on the table the caret is in, and says so
            // when it is not in one: every row is disabled, and resting on
            // one says why.
            menu::top(ui, "T&able", |ui| {
                let why = "Put the caret in a table cell first";
                ui.add_enabled_ui(table.is_some(), |ui| {
                    let row = |ui: &mut egui::Ui, label: &str, command: Command| {
                        let item = menu::item(ui, label, shortcut(&command));
                        let chosen = item.clicked();
                        item.response.on_disabled_hover_text(why);
                        chosen.then_some(command)
                    };
                    menu::sub(ui, "&Insert", |ui| {
                        if let Some(command) =
                            row(ui, "Row &Above", Command::InsertRow { below: false })
                        {
                            chosen = Some(command);
                        }
                        if let Some(command) =
                            row(ui, "Row &Below", Command::InsertRow { below: true })
                        {
                            chosen = Some(command);
                        }
                        if let Some(command) =
                            row(ui, "Column &Left", Command::InsertColumn { after: false })
                        {
                            chosen = Some(command);
                        }
                        if let Some(command) =
                            row(ui, "Column &Right", Command::InsertColumn { after: true })
                        {
                            chosen = Some(command);
                        }
                    });
                    menu::sub(ui, "&Delete", |ui| {
                        if let Some(command) = row(ui, "&Row", Command::DeleteRow) {
                            chosen = Some(command);
                        }
                        if let Some(command) = row(ui, "&Column", Command::DeleteColumn) {
                            chosen = Some(command);
                        }
                        if let Some(command) = row(ui, "&Table", Command::DeleteTable) {
                            chosen = Some(command);
                        }
                    });
                    menu::sep(ui);
                    if let Some(command) = row(ui, "Mer&ge Cells", Command::MergeCells) {
                        chosen = Some(command);
                    }
                    menu::sub(ui, "&Borders", |ui| {
                        if let Some(command) = row(ui, "&All", Command::TableBorders(true)) {
                            chosen = Some(command);
                        }
                        if let Some(command) = row(ui, "&None", Command::TableBorders(false)) {
                            chosen = Some(command);
                        }
                    });
                    menu::sub(ui, "Border &Colour", |ui| {
                        let colours: Vec<(&str, egui::Color32)> = PALETTE
                            .iter()
                            .map(|(name, [r, g, b])| (*name, egui::Color32::from_rgb(*r, *g, *b)))
                            .collect();
                        let picked =
                            match menu::swatches(ui, "&Automatic", &colours, None, Some("&Other…"))
                            {
                                Some(menu::Swatch::First) => {
                                    Some(Command::BorderColor(wp_model::Color::Auto))
                                }
                                Some(menu::Swatch::Index(index)) => Some(Command::BorderColor(
                                    wp_model::Color::Rgb(PALETTE[index].1),
                                )),
                                Some(menu::Swatch::More) => Some(Command::CustomBorderColor),
                                None => None,
                            };
                        if picked.is_some() {
                            chosen = picked;
                        }
                    });
                    menu::sub(ui, "&Shading", |ui| {
                        let picked =
                            crate::app::shading_rows(ui, table.and_then(|table| table.shading));
                        if picked.is_some() {
                            chosen = picked;
                        }
                    });
                    menu::sep(ui);
                    if let Some(command) = row(ui, "Column &Width…", Command::ColumnWidth) {
                        chosen = Some(command);
                    }
                    if let Some(command) = row(ui, "Cell &Margins…", Command::CellMargins) {
                        chosen = Some(command);
                    }
                });
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
                if menu::item(ui, "&Accept", shortcut(&Command::AcceptOne)).clicked() {
                    chosen = Some(Command::AcceptOne);
                }
                if menu::item(ui, "&Reject", shortcut(&Command::RejectOne)).clicked() {
                    chosen = Some(Command::RejectOne);
                }
                if menu::item(ui, "Accept A&ll", shortcut(&Command::AcceptAll)).clicked() {
                    chosen = Some(Command::AcceptAll);
                }
                if menu::item(ui, "Re&ject All", shortcut(&Command::RejectAll)).clicked() {
                    chosen = Some(Command::RejectAll);
                }
                if menu::item(ui, "&Next Change", shortcut(&Command::NextChange)).clicked() {
                    chosen = Some(Command::NextChange);
                }
                if menu::item(ui, "&Previous Change", shortcut(&Command::PreviousChange)).clicked()
                {
                    chosen = Some(Command::PreviousChange);
                }
                menu::sep(ui);
                if menu::item(ui, "New &Comment", shortcut(&Command::AddComment)).clicked() {
                    chosen = Some(Command::AddComment);
                }
                if menu::item(ui, "Repl&y to Comment", shortcut(&Command::ReplyHere)).clicked() {
                    chosen = Some(Command::ReplyHere);
                }
                if menu::item(ui, "Re&solve Comment", shortcut(&Command::ResolveHere)).clicked() {
                    chosen = Some(Command::ResolveHere);
                }
                if menu::item(ui, "&Delete Comment", shortcut(&Command::DeleteComment)).clicked() {
                    chosen = Some(Command::DeleteComment);
                }
            });

            // Each row in its style's own face where the machine has it, at
            // the menu's size — the face and not the size: a 26-point title
            // row in a menu is a wall. The style the caret is in is ticked.
            menu::top(ui, "&Styles", |ui| {
                for (id, name, face) in &style_faces {
                    if menu::check_in_face(ui, name, style == Some(*id), face.clone()).clicked() {
                        chosen = Some(Command::Style(*id));
                    }
                }
                if style_faces.is_empty() {
                    ui.add_enabled(false, egui::Button::new("No styles in this document"));
                }
            });

            menu::top(ui, "&Help", |ui| {
                if menu::item(ui, "&Keyboard Shortcuts…", "").clicked() {
                    chosen = Some(Command::KeyboardShortcuts);
                }
                if menu::item(ui, "&User Guide", "").clicked() {
                    chosen = Some(Command::UserGuide);
                }
                menu::sep(ui);
                if menu::item(ui, "&About Scriva", "").clicked() {
                    chosen = Some(Command::About);
                }
            });

            chosen
        })
    }
}
