//! Calx — spreadsheet.

#![forbid(unsafe_code)]
// No console window on Windows for release builds; keep it in debug so `dbg!` lands
// somewhere visible.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::io::BufRead;
use std::path::{Path, PathBuf};

use ss_formula::clip::{self, Clip};
use ss_formula::cond;
use ss_formula::edit::{self, Change, Geometry, Patch};
use ss_model::style::{BorderStyle, Pattern, Underline, VAlign};
use ss_model::{Axis, CellRange, CellRef, Color, Fill, HAlign, Shift, Workbook};
use ui_kit::{dialog, egui, menu, paths, AppId, DocumentApp, Recent, CALX};

use calx::grid::{self, Action, BorderPreset, Editor, Format, GridView, Mode};
use calx::icons::{self, Icon};

enum Choice {
    Save,
    Discard,
    Cancel,
}

/// A document with no file behind it yet.
///
/// The package is authored rather than left absent, so that saving a new
/// workbook is the same code path as saving one that came from Excel.
fn blank() -> ss_xlsx::XlsxDocument {
    ss_xlsx::XlsxDocument::new(Workbook::blank())
        .expect("a blank package's part names are compile-time constants")
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Who is holding a file open, if anyone, in words rather than in error codes.
///
/// Asked by *trying*, because that is the only answer Windows gives that is
/// worth anything: a file it will not open for writing now is a file it will
/// not let a save write later. Opening for write and closing again changes
/// nothing — no truncation, no timestamp.
///
/// The name comes from the owner file Office leaves beside an open workbook —
/// `~$book.xlsx` — which is what lets this say "Excel" instead of "another
/// program". `None` means the file can be written, or does not exist yet.
fn locked_by(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    if std::fs::OpenOptions::new().write(true).open(path).is_ok() {
        return None;
    }
    let owner = path.file_name().map(|n| {
        let mut name = std::ffi::OsString::from("~$");
        name.push(n);
        path.with_file_name(name)
    });
    match owner {
        Some(owner) if owner.exists() => Some("Excel".to_string()),
        _ => Some("another program".to_string()),
    }
}

/// What to say when a save was refused, in a sentence that names the way out.
///
/// The error itself is not in here: it goes in the box's `detail`, small and
/// grey, where an error code belongs.
fn save_trouble(path: &Path) -> String {
    let name = name_of(path);
    if let Some(holder) = locked_by(path) {
        return format!(
            "{name} was not saved: it is open in {holder}.\n\
             \n\
             Windows will not let one program write over a file another one is \
             holding open. Close it there and press Ctrl+S again — nothing has \
             been lost, the changes are still here — or use Save As to write a \
             different file."
        );
    }
    if path.exists() && std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
        return format!(
            "{name} was not saved: the file is marked read-only.\n\
             \n\
             Clear the read-only tick in its Windows properties, or use Save As \
             to write a different file. The changes are still here either way."
        );
    }
    format!(
        "{name} was not saved. The changes are still here — try Save As to write \
         a different file."
    )
}

fn main() -> ui_kit::eframe::Result<()> {
    let mut app = Calx::new();
    // A path on the command line opens straight into the document, which is what
    // a file association does.
    if let Some(path) = std::env::args_os().nth(1) {
        app.open(Path::new(&path));
    }
    ui_kit::run(app)
}

struct Calx {
    /// Resolved on startup so a permissions problem surfaces immediately rather
    /// than the first time the user changes a setting.
    config_dir: Result<PathBuf, String>,
    /// The workbook *and* the package it came from.
    ///
    /// One value rather than two, because a save writes the model back into the
    /// package it was read from — every part we never understood included — and
    /// keeping the two apart is an invitation for them to drift.
    doc: ss_xlsx::XlsxDocument,
    path: Option<PathBuf>,
    /// The files opened and saved before this one, most recent first.
    recent: Recent,
    grid: GridView,
    status: String,
    /// Changes that undo what has been done, most recent last.
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// Our own copy of the last thing copied, richer than the text that went to
    /// the system clipboard: it carries formulas and styles.
    clip: Option<Clip>,
    /// Exactly what we put on the system clipboard, so a paste can tell whether
    /// the clipboard is still ours or has been replaced by another program.
    clip_text: String,
    /// Where a cut came from, cleared when the paste lands.
    cut_from: Option<(usize, CellRange)>,
    edited: bool,
    /// The title box's buffer while a chart is selected. Kept out of the model
    /// so that typing does not push an undo entry per keystroke.
    chart_title: Option<String>,
    /// What the user asked for that unsaved changes are standing in the way of.
    pending: Option<Pending>,
    /// The name box's buffer. Kept out of the selection because it holds what
    /// is being *typed*, which is not an address until Enter says so.
    name_box: String,
    /// How big the grid was last frame, so that a jump from the name box can
    /// scroll the target into view without waiting for the next one.
    last_body: egui::Vec2,
    /// The one modal window that may be open.
    ///
    /// One rather than a flag each: they are mutually exclusive by construction
    /// — a modal is modal — and a single `Option` makes that true rather than
    /// merely intended.
    dialog: Option<Dialog>,
    /// The sheet tab being dragged along the strip, if one is.
    dragging_tab: Option<usize>,
}

/// A window that takes over until it is answered.
enum Dialog {
    RenameSheet {
        index: usize,
        text: String,
    },
    /// Excel's Move or Copy: where the tab goes, and whether the original stays.
    MoveSheet {
        index: usize,
        /// The tab it goes *before*, or the sheet count for "move to end".
        before: usize,
        copy: bool,
    },
    Sort {
        range: CellRange,
        header: bool,
        /// Three levels, as Excel's dialog offers. A level with no column is
        /// not a level.
        levels: [SortLevel; 3],
    },
    /// Excel's Column Width and Row Height boxes: an exact size, in the units
    /// the file itself stores, applied to whatever is selected.
    Size {
        axis: Axis,
        text: String,
    },
    /// Excel's Zoom box: preset magnifications and a percent you can type.
    Zoom {
        text: String,
        /// True until the percent field is first touched. While set, the
        /// number stays selected so the next keystroke replaces it.
        fresh: bool,
    },
    /// Excel's Name Manager: the workbook's defined names, all of them at once.
    ///
    /// A working copy rather than the workbook's own list, because the whole
    /// list is what one undo entry replaces — a sheet-scoped name carries an
    /// *index* into the sheet list, so the entries are not independent of each
    /// other and editing them one at a time would be a lie.
    Names {
        names: Vec<ss_model::DefinedName>,
        /// Which row is open for editing, if any.
        editing: Option<usize>,
    },
    /// Excel's Format Cells, all five tabs of it.
    ///
    /// The look is a working copy of the cursor's, edited in place and applied
    /// whole on OK — which is what the dialog is, and why pressing OK over a
    /// mixed selection makes it uniform. Excel does the same.
    FormatCells {
        look: Box<ss_model::style::Look>,
        tab: FormatTab,
    },
    /// A note on a cell: Excel's Insert Comment, which it now calls a note.
    Note {
        at: CellRef,
        author: String,
        text: String,
    },
    /// Excel's Text to Columns: how to cut one column of text into several.
    TextToColumns {
        how: ss_formula::tools::Split,
        /// The "Other" box, kept as typed so that clearing it is possible.
        other: String,
    },
    /// Excel's Remove Duplicates: which columns decide that two rows are one.
    RemoveDuplicates {
        /// The rows and columns that will move. Held here rather than read from
        /// the selection when the button is pressed, because it may have been
        /// widened to the block around it and the user is owed the range they
        /// are about to change.
        range: CellRange,
        /// The range's columns, and whether each is being compared.
        columns: Vec<(u32, bool)>,
        header: bool,
    },
    /// Excel's Protect Sheet: what a protected sheet will still allow.
    Protect {
        allow: Box<ss_model::Protection>,
    },
    /// Excel's Paste Special: which parts of the clipboard to bring across.
    PasteSpecial {
        how: ss_formula::clip::PasteSpecial,
    },
    /// Something went wrong and the user has to know.
    ///
    /// Every failure here used to go to the status line, where a save that did
    /// not happen looks exactly like a save that did. This is for the ones a
    /// person has to act on: the file could not be written, or could not be
    /// read. `offer_save_as` puts the way out on the box itself, because the
    /// answer to "that file is locked" is usually "then write a different one".
    Trouble {
        title: &'static str,
        /// What the icon says before the words are read. A file held open by
        /// Excel is news, not a failure, and drawing it with the same red cross
        /// as a workbook that would not open teaches the user to ignore both.
        severity: dialog::Severity,
        message: String,
        /// The machine's own words — an error code, a parser's complaint.
        /// Kept apart from the message because it is the one line in the box
        /// nobody asked for and nobody can act on.
        detail: String,
        offer_save_as: bool,
    },
    /// Excel's Find and Replace, which are one window with a row hidden.
    ///
    /// Not modal, in the sense that matters: it keeps its state across Find
    /// Next, and the selection moves under it. `replacing` is what Ctrl+H
    /// turns on and Ctrl+F leaves off.
    Find {
        query: ss_formula::find::Query,
        with: String,
        replacing: bool,
        /// Every sheet, or just this one.
        whole_workbook: bool,
        /// What the last button press did, in words, because "nothing
        /// happened" and "nothing matched" look identical otherwise.
        report: String,
    },
    /// Excel's Go To (Ctrl+G, F5): an address, a range, or a defined name.
    GoTo {
        text: String,
    },
    /// Excel's Data Validation: one rule, edited whole, applied to the
    /// selection. `existing` is where the working copy came from when the
    /// selection already had a rule.
    Validation {
        rule: ss_model::cond::DataValidation,
        existing: Option<usize>,
    },
    /// A manager for the sheet's conditional-formatting rules: the list is a
    /// working copy edited whole and applied as one patch, plus a builder
    /// for a new rule over the selection.
    CondFormat {
        formats: Vec<ss_model::cond::ConditionalFormat>,
        kind: usize,
        operator: ss_model::cond::CfOperator,
        value1: String,
        value2: String,
        bold: bool,
        italic: bool,
        use_text_color: bool,
        text_color: [u8; 3],
        use_fill: bool,
        fill_color: [u8; 3],
    },
    /// The checkbox list behind one filter arrow.
    Filter {
        /// An offset into the filter's range.
        col: u32,
        /// Every distinct value in the column, and whether it has blanks.
        offered: Vec<String>,
        has_blanks: bool,
        ticked: std::collections::BTreeSet<String>,
        blanks: bool,
        search: String,
    },
}

/// The tabs of Format Cells, in Excel's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FormatTab {
    #[default]
    Number,
    Alignment,
    Font,
    Border,
    Fill,
    Protection,
}

impl FormatTab {
    const ALL: [(FormatTab, &'static str); 6] = [
        (FormatTab::Number, "Number"),
        (FormatTab::Alignment, "Alignment"),
        (FormatTab::Font, "Font"),
        (FormatTab::Border, "Border"),
        (FormatTab::Fill, "Fill"),
        (FormatTab::Protection, "Protection"),
    ];
}

/// One row of the sort dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SortLevel {
    /// `None` for "none", else an absolute sheet column.
    col: Option<u32>,
    descending: bool,
}

/// Which of the Data menu's two tools was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataTool {
    TextToColumns,
    RemoveDuplicates,
}

/// An action held up by the "you have unsaved changes" prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    /// The window is closing and the application goes with it.
    Quit,
    /// The workbook is being put away and the window stays.
    Close,
    New,
    Open(PathBuf),
    /// Open, but the file still has to be chosen — asked for *after* the
    /// current document is dealt with, so the user is not made to pick a file
    /// and only then told they might lose the one they have.
    Browse,
}

/// Something the command surface asked for.
///
/// The menus and the toolbar gather one of these and hand it back; the caller
/// performs it once the layout is finished. Deferred rather than run where it
/// is clicked because a command that ran mid-layout would change the document
/// under the controls that have not been drawn yet — and because the layout
/// closures hold `self`, while half of these need it again.
///
/// It also means the menu bar and the toolbar cannot drift: "Freeze Panes" in
/// the View menu and the freeze button on the toolbar are the same `Command`,
/// not two call sites that were meant to stay in step.
#[derive(Debug, Clone, PartialEq)]
enum Command {
    /// Anything the grid already knows how to be asked for.
    Do(Action),
    /// A document-level move that has to clear the unsaved-changes prompt.
    Guard(Pending),
    Save,
    SaveAs,
    Exit,
    Reopen(PathBuf),
    ForgetRecent,
    Sort(bool),
    Filter(FilterCommand),
    Size(Axis),
    Find {
        replacing: bool,
    },
    GoTo,
    Names,
    FormatCells(FormatTab),
    Validation,
    CondFormat,
    Protect,
    Data(DataTool),
    Chart(ss_model::ChartKind),
    Picture,
    Note,
    /// Paste needs the system clipboard, which the layout has no business
    /// reading: the read happens where the command is performed.
    Paste,
    PasteSpecial,
    Autosum,
    Zoom(f64),
}

impl Calx {
    fn new() -> Self {
        Calx {
            config_dir: paths::config_dir(CALX).map_err(|e| e.to_string()),
            doc: blank(),
            path: None,
            recent: Recent::load(CALX),
            grid: GridView::default(),
            status: "Ready".to_string(),
            undo: Vec::new(),
            redo: Vec::new(),
            clip: None,
            clip_text: String::new(),
            cut_from: None,
            edited: false,
            chart_title: None,
            pending: None,
            name_box: "A1".to_string(),
            last_body: egui::vec2(800.0, 600.0),
            dialog: None,
            dragging_tab: None,
        }
    }

    fn open(&mut self, path: &Path) {
        if is_delimited(path) {
            self.open_delimited(path);
            return;
        }
        if is_legacy(path) {
            self.open_legacy(path);
            return;
        }
        match ss_xlsx::XlsxDocument::open(path) {
            Ok(doc) => {
                self.doc = doc;
                let cells: usize = self.doc.workbook.sheets.iter().map(|s| s.cells.len()).sum();
                self.status = format!("{} sheet(s), {cells} cells", self.doc.workbook.sheets.len());
                self.path = Some(path.to_path_buf());
                self.recent.remember(CALX, path);
                self.reset();
                // The sheet the workbook was saved on, at the zoom it was saved
                // at. Opening every file on sheet one at 100% is opening a
                // different document than the one that was closed.
                let active = self.doc.workbook.active_sheet;
                self.grid.open_sheet(&self.doc.workbook, active);
                // Excel says at the door that a workbook is open elsewhere,
                // and it is right to: a person who is not told now finds out
                // an hour later, when Ctrl+S is refused and the hour is
                // sitting in memory. This does not open the file read-only —
                // the edits are real and Save As will write them — it says
                // which of the two saves is available.
                if let Some(holder) = locked_by(path) {
                    self.dialog = Some(Dialog::Trouble {
                        title: "Open in another program",
                        // A warning rather than a notice: it is about a save
                        // that will be refused later, and later is when it
                        // costs something.
                        severity: dialog::Severity::Warning,
                        message: format!(
                            "{} is open in {holder}.\n\
                             \n\
                             You can read and edit it here, but Windows will not let \
                             Calx write over a file another program is holding, so \
                             Ctrl+S will be refused until it is closed there.\n\
                             Save As will write a copy at any time.",
                            name_of(path)
                        ),
                        detail: String::new(),
                        offer_save_as: false,
                    });
                }
            }
            Err(e) => {
                self.status = format!("could not open {}: {e}", path.display());
                self.dialog = Some(Dialog::Trouble {
                    title: "Not opened",
                    severity: dialog::Severity::Error,
                    message: format!("{} could not be opened.", name_of(path)),
                    detail: e.to_string(),
                    offer_save_as: false,
                });
            }
        }
    }

    /// Opens a legacy `.xls` workbook, which is read-only.
    ///
    /// Like a csv import, the path is deliberately not kept as the document's
    /// own. Nothing writes BIFF and nothing here ever will, so Ctrl+S has to
    /// become Save As — and saving as xlsx over the original path would be a
    /// silent format change with the old extension still on it.
    fn open_legacy(&mut self, path: &Path) {
        let doc = match ss_xls::open(path) {
            Ok(doc) => doc,
            Err(e) => {
                self.status = format!("could not open {}: {e}", path.display());
                return;
            }
        };
        let cells: usize = doc.workbook.sheets.iter().map(|s| s.cells.len()).sum();
        let sheets = doc.workbook.sheets.len();
        let active = doc.workbook.active_sheet;
        self.doc = match ss_xlsx::XlsxDocument::new(doc.workbook) {
            Ok(doc) => doc,
            Err(e) => {
                self.status = format!("could not open {}: {e}", path.display());
                return;
            }
        };
        self.path = None;
        // Listed even though it is not the document's own path: the list is of
        // files the user opened, and a read-only workbook is one they will want
        // to get back to as much as any other.
        self.recent.remember(CALX, path);
        self.reset();
        self.recalculate();
        // Opened but not saveable in place: the same state a csv import leaves,
        // so the unsaved-changes guard offers the way out.
        self.edited = true;
        self.grid.open_sheet(&self.doc.workbook, active);
        self.status = format!(
            "{} — {sheets} sheet(s), {cells} cells, read-only (save as .xlsx to edit)",
            name_of(path)
        );
    }

    /// Imports a delimited text file into a new workbook.
    ///
    /// The path is deliberately *not* remembered as the document's own. A csv
    /// holds one sheet of values and no formulas, formats, or second sheet, so
    /// Ctrl+S over the original would quietly throw away everything the user
    /// then added. Save-as is the way back out, and it defaults to xlsx.
    fn open_delimited(&mut self, path: &Path) {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) => {
                self.status = format!("could not open {}: {e}", path.display());
                return;
            }
        };
        let mut source = std::io::BufReader::new(file);
        // The sniffer needs a look at the start without consuming it.
        let sample = match source.fill_buf() {
            Ok(bytes) => bytes.to_vec(),
            Err(e) => {
                self.status = format!("could not read {}: {e}", path.display());
                return;
            }
        };
        let (encoding, mut dialect) = ss_csv::sniff(&sample);
        // The extension is a stronger signal than any guess when the file has
        // no delimiter on its first lines at all.
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            if sample.iter().all(|b| *b != dialect.delimiter) {
                dialect = ss_csv::Dialect::for_extension(extension);
            }
        }

        let mut book = Workbook::blank();
        book.sheets[0].name = sheet_name(path);
        let mut reader = ss_csv::Reader::new(source, encoding, dialect);
        let imported = ss_csv::read_into(&mut reader, |row, fields| {
            for (col, field) in fields.iter().enumerate() {
                if field.is_empty() {
                    continue;
                }
                // Two mutable borrows of the workbook, one after the other: the
                // interpretation may allocate a number format or a formula, and
                // only then is there a cell to store.
                let cell = edit::typed_cell(&mut book, 0, ss_model::StyleId::DEFAULT, field);
                book.sheets[0].set(ss_model::CellRef::new(row, col as u32), cell);
            }
        });

        match imported {
            Ok(stats) => {
                self.doc = match ss_xlsx::XlsxDocument::new(book) {
                    Ok(doc) => doc,
                    Err(e) => {
                        self.status = format!("could not import {}: {e}", path.display());
                        return;
                    }
                };
                self.path = None;
                self.recent.remember(CALX, path);
                self.reset();
                self.recalculate();
                // Imported, not opened: the user has an unsaved workbook.
                self.edited = true;
                let mut note = format!(
                    "imported {} rows x {} columns ({}, {:?})",
                    stats.rows,
                    stats.columns,
                    delimiter_name(dialect.delimiter),
                    encoding
                );
                if stats.truncated > 0 {
                    note.push_str(&format!(
                        ", {} rows past the sheet dropped",
                        stats.truncated
                    ));
                }
                self.status = note;
            }
            Err(e) => self.status = format!("could not read {}: {e}", path.display()),
        }
    }

    /// Exports the active sheet as delimited text.
    fn write_delimited(&mut self, path: &Path) {
        let dialect = ss_csv::Dialect::for_extension(
            path.extension().and_then(|e| e.to_str()).unwrap_or("csv"),
        );
        let book = &self.doc.workbook;
        let Some(sheet) = book.sheet(self.grid.sheet_index) else {
            self.status = "nothing to export".to_string();
            return;
        };
        let file = match std::fs::File::create(path) {
            Ok(file) => file,
            Err(e) => {
                self.status = format!("could not save {}: {e}", path.display());
                self.dialog = Some(Dialog::Trouble {
                    title: "Not saved",
                    severity: dialog::Severity::Error,
                    message: save_trouble(path),
                    detail: e.to_string(),
                    offer_save_as: true,
                });
                return;
            }
        };
        let mut out = std::io::BufWriter::new(file);
        let written = ss_csv::write_sheet(&mut out, sheet, dialect, |at| {
            // The *displayed* text, which is what a csv is for: a column of
            // dates has to come out as dates rather than as five-digit serials.
            let Some(cell) = sheet.get(at) else {
                return String::new();
            };
            let value = match cell.value {
                ss_model::CellValue::Blank => return String::new(),
                ss_model::CellValue::Number(n) => ss_model::FormatValue::Number(n),
                ss_model::CellValue::Bool(b) => ss_model::FormatValue::Bool(b),
                ss_model::CellValue::Error(e) => ss_model::FormatValue::Error(e),
                ss_model::CellValue::Text(id) => {
                    ss_model::FormatValue::Text(book.strings.resolve(id))
                }
            };
            book.styles
                .number_format(sheet.style_at(at))
                .format(value)
                .text
        });
        match written.and_then(|()| std::io::Write::flush(&mut out)) {
            Ok(()) => {
                // Exported rather than saved: the workbook still belongs to its
                // own file, and `edited` stays as it was.
                self.status = format!("exported {} to {}", sheet.name, name_of(path));
            }
            Err(e) => {
                self.status = format!("could not save {}: {e}", path.display());
                self.dialog = Some(Dialog::Trouble {
                    title: "Not saved",
                    severity: dialog::Severity::Error,
                    message: save_trouble(path),
                    detail: e.to_string(),
                    offer_save_as: true,
                });
            }
        }
    }

    fn new_document(&mut self) {
        self.blank_slate();
        self.status = "New workbook".to_string();
    }

    /// Excel's File ▸ Close: the workbook is put away, the window stays.
    ///
    /// What is left behind is an empty workbook rather than an empty window.
    /// A spreadsheet with no grid in it is a window with nothing to click, and
    /// the next thing anyone does after closing one workbook is start another
    /// or open another — both of which want a grid to land in.
    ///
    /// Which makes this New with a different sentence at the bottom, and that
    /// sentence is the point: "Closed budget.xlsx" says the file was let go of,
    /// where "New workbook" only says one arrived.
    fn close_document(&mut self) {
        let closed = self.path.as_deref().map(name_of);
        self.blank_slate();
        self.status = match closed {
            Some(name) => format!("Closed {name}"),
            None => "Closed".to_string(),
        };
    }

    /// A fresh empty document, with nothing of the old one left anywhere.
    ///
    /// The dialogs go too. Every one of them was opened about the document
    /// that is being let go — a Format Cells over a selection that no longer
    /// exists, a Find over a sheet that has gone — and a box that outlives its
    /// subject can only do harm when its buttons are pressed.
    fn blank_slate(&mut self) {
        self.doc = blank();
        self.path = None;
        self.dialog = None;
        self.reset();
    }

    /// Clears everything that belonged to the document being replaced.
    ///
    /// The undo stack most of all: its patches name sheet indices and formula
    /// ids in the old workbook, and applying one to a different document would
    /// write cells at addresses nobody chose.
    fn reset(&mut self) {
        self.grid = GridView::default();
        self.undo.clear();
        self.redo.clear();
        self.clip = None;
        self.clip_text.clear();
        self.cut_from = None;
        self.edited = false;
    }

    /// Records a finished picture drag, or drops it if nothing actually moved.
    ///
    /// A plain click on a picture selects it and ends a drag of zero distance.
    /// Left unchecked that would put an undo entry on the stack for every
    /// click, so the only thing that decides is whether the geometry actually
    /// differs from what it was.
    fn pictures_moved(&mut self, sheet: usize, before: Vec<ss_model::Picture>) {
        let unchanged = self
            .doc
            .workbook
            .sheet(sheet)
            .is_some_and(|s| s.pictures == before);
        if unchanged {
            return;
        }
        self.undo.push(Change::new(
            "Move picture",
            vec![Patch::Pictures {
                sheet,
                pictures: before,
            }],
        ));
        self.redo.clear();
        self.edited = true;
    }

    fn delete_picture(&mut self, sheet: usize, index: usize) {
        let Some(target) = self.doc.workbook.sheet_mut(sheet) else {
            return;
        };
        if index >= target.pictures.len() {
            return;
        }
        let before = target.pictures.clone();
        target.pictures.remove(index);
        self.grid.selected_picture = None;
        self.undo.push(Change::new(
            "Delete picture",
            vec![Patch::Pictures {
                sheet,
                pictures: before,
            }],
        ));
        self.redo.clear();
        self.edited = true;
    }

    /// Puts a chart of `kind` on the sheet, plotting whatever is selected.
    ///
    /// Excel's own reading of a selection: the first column labels the points
    /// when it holds text, the first row names the series when it looks like a
    /// heading, and every remaining column is a series. Getting this wrong in
    /// either direction is recoverable — the chart is there to be looked at and
    /// undone — and guessing is what makes the button worth pressing.
    fn insert_chart(&mut self, kind: ss_model::ChartKind) {
        let Some(range) = self.data_range() else {
            self.status = "Select the cells to plot first".to_string();
            return;
        };
        let index = self.grid.sheet_index;
        let Some(sheet) = self.doc.workbook.sheet(index) else {
            return;
        };
        let header = ss_formula::sort::looks_like_headers(sheet, range);
        let name = sheet.name.clone();
        let labelled = matches!(
            sheet
                .get(CellRef::new(
                    range.start.row + u32::from(header),
                    range.start.col
                ))
                .map(|c| c.value),
            Some(ss_model::CellValue::Text(_))
        ) && range.cols() > 1;

        let first_row = range.start.row + u32::from(header);
        let categories: Vec<String> = if labelled {
            (first_row..=range.end.row)
                .map(|row| self.display_text(index, CellRef::new(row, range.start.col)))
                .collect()
        } else {
            Vec::new()
        };
        let categories_ref = labelled.then(|| {
            reference(
                &name,
                range.start.col,
                first_row,
                range.start.col,
                range.end.row,
            )
        });

        let first_col = range.start.col + u32::from(labelled);
        let mut series = Vec::new();
        for col in first_col..=range.end.col {
            let values: Vec<Option<f64>> = (first_row..=range.end.row)
                .map(
                    |row| match sheet.get(CellRef::new(row, col)).map(|c| c.value) {
                        Some(ss_model::CellValue::Number(n)) => Some(n),
                        _ => None,
                    },
                )
                .collect();
            if values.iter().all(Option::is_none) {
                continue;
            }
            series.push(ss_model::chart::Series {
                name: header.then(|| self.display_text(index, CellRef::new(range.start.row, col))),
                name_ref: header
                    .then(|| reference(&name, col, range.start.row, col, range.start.row)),
                values_ref: Some(reference(&name, col, first_row, col, range.end.row)),
                values,
                categories_ref: categories_ref.clone(),
                categories: categories.clone(),
                color: None,
            });
        }
        if series.is_empty() {
            self.status = "There are no numbers in the selection to plot".to_string();
            return;
        }

        // Beside the data rather than over it, and about the size Excel makes
        // one: fifteen rows tall and seven columns wide.
        let corner = |col: u32, row: u32| ss_model::chart::AnchorPoint {
            col,
            row,
            ..Default::default()
        };
        let left = range
            .end
            .col
            .saturating_add(2)
            .min(ss_model::cell::MAX_COLS - 8);
        let chart = ss_model::Chart {
            part: String::new(),
            drawing_part: String::new(),
            anchor_index: 0,
            anchor: ss_model::chart::Anchor::TwoCell {
                from: corner(left, range.start.row),
                to: corner(left + 7, range.start.row.saturating_add(15)),
            },
            kind,
            grouping: ss_model::chart::Grouping::Clustered,
            horizontal: false,
            title: None,
            title_ref: None,
            legend: (series.len() > 1).then_some(ss_model::chart::LegendPosition::Right),
            series,
        };

        let Some(target) = self.doc.workbook.sheet_mut(index) else {
            return;
        };
        let before = target.charts.clone();
        target.charts.push(chart);
        self.grid.selected_chart = Some(target.charts.len() - 1);
        self.undo.push(Change::new(
            "Insert chart",
            vec![Patch::Charts {
                sheet: index,
                charts: before,
            }],
        ));
        self.redo.clear();
        self.edited = true;
        self.status = "Chart inserted".to_string();
    }

    /// Puts an image on the sheet at the cursor.
    ///
    /// Anchored to one cell at its natural size rather than stretched over the
    /// selection: a logo dropped onto a sheet should look like itself, and a
    /// picture is resized afterwards by dragging its corner.
    fn insert_picture(&mut self) {
        let mut dialog =
            rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp"]);
        if let Some(directory) = self.start_directory() {
            dialog = dialog.set_directory(directory);
        }
        let Some(path) = dialog.pick_file() else {
            return;
        };
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(e) => {
                self.status = format!("could not read {}: {e}", path.display());
                return;
            }
        };
        // Decoded here and thrown away: what is wanted is the size, and a file
        // that will not decode is one the grid could not draw either, which is
        // better said now than as a blank rectangle later.
        let Ok(decoded) = image::load_from_memory(&data) else {
            self.status = format!("{} is not an image Calx can read", name_of(&path));
            return;
        };
        let (pixels_wide, pixels_high) = image::GenericImageView::dimensions(&decoded);
        let emu = |pixels: u32| {
            // A pixel at 96 DPI is three quarters of a point, and a point is
            // 12,700 EMUs.
            (f64::from(pixels) * 0.75 * ss_model::chart::EMU_PER_POINT) as i64
        };

        let index = self.grid.sheet_index;
        let at = self.grid.selection.cursor();
        let Some(sheet) = self.doc.workbook.sheet(index) else {
            return;
        };
        let before = sheet.pictures.clone();
        let number = sheet.pictures.len() + 1;
        let picture = ss_model::Picture {
            part: String::new(),
            drawing_part: String::new(),
            anchor_index: 0,
            name: format!("Picture {number}"),
            anchor: ss_model::chart::Anchor::OneCell {
                from: ss_model::chart::AnchorPoint {
                    col: at.col,
                    row: at.row,
                    ..Default::default()
                },
                width: emu(pixels_wide),
                height: emu(pixels_high),
            },
            data: std::sync::Arc::from(data.into_boxed_slice()),
            content_type: content_type_of(&path).to_string(),
        };
        let Some(target) = self.doc.workbook.sheet_mut(index) else {
            return;
        };
        target.pictures.push(picture);
        self.grid.selected_picture = Some(target.pictures.len() - 1);
        self.grid.selected_chart = None;
        self.undo.push(Change::new(
            "Insert picture",
            vec![Patch::Pictures {
                sheet: index,
                pictures: before,
            }],
        ));
        self.redo.clear();
        self.edited = true;
        self.status = format!("{} inserted", name_of(&path));
    }

    /// A cell as the grid shows it, which is what a chart label should say.
    fn display_text(&self, sheet: usize, at: CellRef) -> String {
        let book = &self.doc.workbook;
        let Some(target) = book.sheet(sheet) else {
            return String::new();
        };
        let Some(cell) = target.get(at) else {
            return String::new();
        };
        let value = match cell.value {
            ss_model::CellValue::Blank => return String::new(),
            ss_model::CellValue::Number(n) => ss_model::FormatValue::Number(n),
            ss_model::CellValue::Bool(b) => ss_model::FormatValue::Bool(b),
            ss_model::CellValue::Error(e) => ss_model::FormatValue::Error(e),
            ss_model::CellValue::Text(id) => ss_model::FormatValue::Text(book.strings.resolve(id)),
        };
        book.styles
            .number_format(target.style_at(at))
            .format(value)
            .text
    }

    /// Records a finished chart drag, or drops it if nothing actually moved.
    fn charts_moved(&mut self, sheet: usize, before: Vec<ss_model::Chart>) {
        let unchanged = self
            .doc
            .workbook
            .sheet(sheet)
            .is_some_and(|s| s.charts == before);
        if unchanged {
            return;
        }
        self.undo.push(Change::new(
            "Move chart",
            vec![Patch::Charts {
                sheet,
                charts: before,
            }],
        ));
        self.redo.clear();
        self.edited = true;
    }

    fn delete_chart(&mut self, sheet: usize, index: usize) {
        let Some(target) = self.doc.workbook.sheet_mut(sheet) else {
            return;
        };
        if index >= target.charts.len() {
            return;
        }
        let before = target.charts.clone();
        target.charts.remove(index);
        self.grid.selected_chart = None;
        self.undo.push(Change::new(
            "Delete chart",
            vec![Patch::Charts {
                sheet,
                charts: before,
            }],
        ));
        self.redo.clear();
        self.edited = true;
    }

    /// Writes to the current path, asking for one if there is none.
    fn save(&mut self) {
        match self.path.clone() {
            Some(path) => self.write(&path),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Excel workbook", &["xlsx"])
            .add_filter("Comma-separated values", &["csv"])
            .add_filter("Tab-separated values", &["tsv", "txt"]);
        if let Some(current) = self.start_directory() {
            dialog = dialog.set_directory(current);
        }
        if let Some(name) = self.path.as_ref().and_then(|p| p.file_name()) {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
        if let Some(path) = dialog.save_file() {
            self.write(&path);
        }
    }

    fn write(&mut self, path: &Path) {
        // An open editor holds text that is not in the workbook yet, and a save
        // that dropped it would be the worst possible moment to do so.
        self.grid.commit(None);
        for action in self.grid.take_actions() {
            self.act_headless(action);
        }

        if is_delimited(path) {
            self.write_delimited(path);
            return;
        }

        match self.doc.save(path) {
            Ok(()) => {
                self.path = Some(path.to_path_buf());
                self.recent.remember(CALX, path);
                self.edited = false;
                self.status = format!("Saved {}", name_of(path));
            }
            // Deliberately not clearing `edited`: the document is still
            // unsaved, and the next Ctrl+S should try again.
            //
            // And said in a box rather than on the status line. A save that
            // did not happen is the one failure a person must not be allowed
            // to walk away from, and a grey line along the bottom of the
            // window is something anyone can miss — which is exactly what
            // happened: "the save feature does not work", when the save was
            // refused by Windows because Excel had the file open.
            Err(e) => {
                self.status = format!("could not save {}: {e}", path.display());
                self.dialog = Some(Dialog::Trouble {
                    title: "Not saved",
                    severity: dialog::Severity::Error,
                    message: save_trouble(path),
                    detail: e.to_string(),
                    offer_save_as: true,
                });
            }
        }
    }

    fn browse(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter(
                "Spreadsheets",
                &["xlsx", "xlsm", "xls", "xlt", "csv", "tsv", "txt"],
            )
            .add_filter("Excel workbook", &["xlsx", "xlsm"])
            .add_filter("Excel 97-2003 workbook", &["xls", "xlt"])
            .add_filter("Delimited text", &["csv", "tsv", "txt"]);
        if let Some(current) = self.start_directory() {
            dialog = dialog.set_directory(current);
        }
        if let Some(path) = dialog.pick_file() {
            self.guard(Pending::Open(path));
        }
    }

    /// Where a file dialog should start: this document's own directory, or —
    /// for a workbook that has no file yet — wherever the last one came from,
    /// which beats whatever the process happens to be running in.
    fn start_directory(&self) -> Option<&Path> {
        match self.path.as_deref().and_then(Path::parent) {
            Some(dir) => Some(dir),
            None => self.recent.directory(),
        }
    }

    /// Opens a file from the recent list, dropping the entry if it has since
    /// been moved or deleted — a menu that offers a file nobody has is worse
    /// than one that is a line shorter.
    fn open_recent(&mut self, path: PathBuf) {
        if !path.exists() {
            self.status = format!("{} is no longer there", path.display());
            self.recent.forget(CALX, &path);
            return;
        }
        self.guard(Pending::Open(path));
    }

    /// Runs `what` now, or holds it behind the unsaved-changes prompt.
    fn guard(&mut self, what: Pending) {
        if self.edited {
            self.pending = Some(what);
        } else {
            self.proceed(what);
        }
    }

    fn proceed(&mut self, what: Pending) {
        match what {
            Pending::Quit => {}
            Pending::Close => self.close_document(),
            Pending::New => self.new_document(),
            Pending::Open(path) => self.open(&path),
            Pending::Browse => self.browse(),
        }
    }

    /// Applies an action with no `Ui` to hand.
    ///
    /// Only the ones a save can provoke — committing an open editor — reach
    /// here. Anything needing the clipboard does not, and saying so is better
    /// than passing a `Ui` around that would only ever be used by mistake.
    fn act_headless(&mut self, action: Action) {
        if let Action::Commit { at, text, .. } = action {
            let sheet = self.grid.sheet_index;
            let change = edit::input(&mut self.doc.workbook, sheet, at, &text);
            self.perform(change);
        }
    }

    /// The file keystrokes, which belong to the window rather than the grid.
    fn file_keys(&mut self, ctx: &egui::Context) {
        let (save, save_as, open, new, close) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::S,
                ),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::O),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::N),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::W),
            )
        });
        // `consume_key` matches modifiers exactly, so Ctrl+Shift+S is not also a
        // Ctrl+S; the `else` is belt and braces against that changing.
        if save_as {
            self.save_as();
        } else if save {
            self.save();
        }
        if open {
            self.guard(Pending::Browse);
        }
        if new {
            self.guard(Pending::New);
        }
        // Ctrl+W puts the workbook away and leaves the window standing, even
        // when it is the last one. Excel closes the application with it; a
        // mistyped Ctrl+W is a slip, and a slip should not end the session.
        if close {
            self.guard(Pending::Close);
        }

        // Find and Replace are one window with a row hidden, so the two keys
        // open the same dialog and differ only in whether the row is there. A
        // Ctrl+H over an already-open Find turns it into a Replace rather than
        // throwing away what has been typed.
        let (find, replace) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::F),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::H),
            )
        });
        if find || replace {
            self.open_find(replace);
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::ALT,
                egui::Key::V,
            )
        }) {
            self.dialog = Some(Dialog::PasteSpecial {
                how: Default::default(),
            });
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num1)) {
            self.open_format_cells();
        }
        // Excel's Ctrl+F3, and it is worth having: a workbook full of names
        // nobody can see is a workbook full of formulas nobody can read.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F3)) {
            self.open_names();
        }
        // Go To, under both of Excel's keys.
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::G)
                | i.consume_key(egui::Modifiers::NONE, egui::Key::F5)
        }) {
            self.dialog = Some(Dialog::GoTo {
                text: String::new(),
            });
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::L,
            )
        }) {
            self.toggle_filter();
        }
        // Alt+= writes the SUM the toolbar button writes.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::Equals)) {
            self.autosum();
        }
        // Alt+F1 charts the selection where it stands. Excel's F11 puts the
        // same chart on a sheet of its own, which needs a chart sheet — a
        // sheet kind this workbook model keeps a slot for but cannot draw —
        // so only the embedded half of that pair is here.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::F1)) {
            self.insert_chart(ss_model::ChartKind::Bar);
        }
        // F9 recalculates. Everything here recalculates after every edit, so
        // the key is for the volatile functions — NOW, TODAY, RAND — whose
        // answers go stale on their own with nothing having been typed.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F9)) {
            self.recalculate();
            if !self.status.starts_with("circular") {
                self.status = "Recalculated".to_string();
            }
        }
    }

    /// Opens Data Validation on the rule covering the cursor, or on a fresh
    /// rule over the selection.
    fn open_validation(&mut self) {
        let sheet = self.grid.sheet_index;
        let cursor = self.grid.selection.cursor();
        let Some(s) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        let existing = s.validations.iter().position(|v| v.covers(cursor));
        let rule = match existing {
            Some(i) => s.validations[i].clone(),
            None => ss_model::cond::DataValidation {
                ranges: self.grid.selection.ranges().to_vec(),
                kind: ss_model::cond::DvKind::List,
                ..Default::default()
            },
        };
        self.dialog = Some(Dialog::Validation { rule, existing });
    }

    fn open_cond_format(&mut self) {
        let sheet = self.grid.sheet_index;
        let Some(s) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        self.dialog = Some(Dialog::CondFormat {
            formats: s.conditional_formats.clone(),
            kind: 0,
            operator: ss_model::cond::CfOperator::GreaterThan,
            value1: String::new(),
            value2: String::new(),
            bold: false,
            italic: false,
            use_text_color: false,
            text_color: [0x9C, 0x00, 0x06],
            use_fill: true,
            fill_color: [0xFF, 0xC7, 0xCE],
        });
    }

    fn open_names(&mut self) {
        self.dialog = Some(Dialog::Names {
            names: self.doc.workbook.defined_names.clone(),
            editing: None,
        });
    }

    /// Opens Format Cells on a working copy of the cursor's own formatting.
    ///
    /// The cursor's rather than the selection's, because a selection can be
    /// mixed and a dialog has to show one answer per field. Excel picks the
    /// same cell, and pressing OK is what makes the rest of the selection
    /// agree with it.
    fn open_format_cells(&mut self) {
        self.dialog = Some(Dialog::FormatCells {
            look: Box::new(self.cursor_look()),
            tab: FormatTab::default(),
        });
    }

    /// Opens the note on the cursor cell, or a blank one to write.
    fn open_note(&mut self) {
        let at = self.grid.selection.cursor();
        let existing = self
            .doc
            .workbook
            .sheet(self.grid.sheet_index)
            .and_then(|s| s.comments.iter().find(|c| c.at == at));
        self.dialog = Some(Dialog::Note {
            at,
            author: existing.map_or_else(author_name, |c| c.author.clone()),
            text: existing.map(|c| c.body().to_string()).unwrap_or_default(),
        });
    }

    /// Writes a note onto a cell, or takes one off when the text is empty.
    fn set_note(&mut self, at: CellRef, author: &str, text: &str) {
        let sheet = self.grid.sheet_index;
        let Some(target) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        let mut comments = target.comments.clone();
        comments.retain(|note| note.at != at);
        let text = text.trim();
        if !text.is_empty() {
            // The author's name goes into the body as well as into the author
            // list, which is what Excel writes and what every other reader
            // expects to find at the top of the box.
            comments.push(ss_model::Comment::new(
                at,
                author,
                format!(
                    "{author}:
{text}"
                ),
            ));
            comments.sort_by_key(|note| (note.at.row, note.at.col));
        }
        if comments == target.comments {
            return;
        }
        let label = if text.is_empty() {
            "Delete note"
        } else {
            "Edit note"
        };
        self.perform(Change::new(
            label,
            vec![Patch::Comments { sheet, comments }],
        ));
    }

    fn open_text_to_columns(&mut self) {
        self.dialog = Some(Dialog::TextToColumns {
            how: ss_formula::tools::Split::default(),
            other: String::new(),
        });
    }

    fn open_remove_duplicates(&mut self) {
        let range = self.grid.selection.active_range();
        let Some(sheet) = self.doc.workbook.sheet(self.grid.sheet_index) else {
            return;
        };
        // The selection trimmed to the data, so that a whole-column selection
        // offers the columns somebody has actually used rather than sixteen
        // thousand checkboxes.
        let Some(range) = ss_formula::sort::clamped(sheet, range) else {
            self.status = "There is nothing there to look through".to_string();
            return;
        };
        // Widened to the block it sits in, which is what Excel's "expand the
        // selection" prompt offers and always the right answer: removing rows
        // from column A alone, while B stays where it is, silently tears every
        // row of the table in half.
        let block = ss_formula::sort::region(sheet, range.start);
        let range = CellRange::new(
            CellRef::new(
                range.start.row.min(block.start.row),
                range.start.col.min(block.start.col),
            ),
            CellRef::new(
                range.end.row.max(block.end.row),
                range.end.col.max(block.end.col),
            ),
        );
        self.dialog = Some(Dialog::RemoveDuplicates {
            range,
            columns: (range.start.col..=range.end.col)
                .map(|c| (c, true))
                .collect(),
            // The same guess the sort dialog makes, and for the same reason:
            // a first row of text over columns of numbers is a heading row.
            header: ss_formula::sort::looks_like_headers(sheet, range),
        });
    }

    fn text_to_columns(&mut self, how: &ss_formula::tools::Split) {
        let sheet = self.grid.sheet_index;
        let range = self.grid.selection.active_range();
        match ss_formula::tools::text_to_columns(&mut self.doc.workbook, sheet, range, how) {
            Ok(change) => {
                if change.is_empty() {
                    self.status = "There is no text there to split".to_string();
                    return;
                }
                self.perform(change);
                self.grid.invalidate();
            }
            Err(why) => self.status = why,
        }
    }

    fn remove_duplicates(&mut self, range: CellRange, columns: &[u32], header: bool) {
        let sheet = self.grid.sheet_index;
        match ss_formula::tools::remove_duplicates(
            &mut self.doc.workbook,
            sheet,
            range,
            columns,
            header,
        ) {
            Ok((change, count)) => {
                self.perform(change);
                self.grid.invalidate();
                self.status = match count.removed {
                    0 => format!("No repeated rows found; {} rows kept", count.kept),
                    1 => format!("1 repeated row removed; {} rows kept", count.kept),
                    n => format!("{n} repeated rows removed; {} rows kept", count.kept),
                };
            }
            Err(why) => self.status = why,
        }
    }

    /// Opens Protect Sheet, or takes protection off a sheet that has it.
    ///
    /// Excel's own toggle: one control that asks what to allow on the way in
    /// and asks nothing on the way out.
    fn toggle_protection(&mut self) {
        let Some(sheet) = self.doc.workbook.sheet(self.grid.sheet_index) else {
            return;
        };
        match &sheet.protection {
            // Calx implements none of Excel's password hashes, so it cannot
            // tell the right password from the wrong one. Refusing is the only
            // honest answer: quietly unprotecting would throw away a decision
            // somebody deliberately made, and asking for a password we cannot
            // check would be theatre.
            Some(p) if p.has_password() => {
                self.status =
                    "This sheet is protected with a password, which Calx cannot check".to_string();
            }
            Some(_) => self.protect(None),
            None => {
                self.dialog = Some(Dialog::Protect {
                    allow: Box::new(ss_model::Protection::as_excel_protects()),
                });
            }
        }
    }

    fn protect(&mut self, protection: Option<ss_model::Protection>) {
        let sheet = self.grid.sheet_index;
        let label = if protection.is_some() {
            "Protect sheet"
        } else {
            "Unprotect sheet"
        };
        self.perform(Change::new(
            label,
            vec![Patch::Protection { sheet, protection }],
        ));
    }

    /// Opens Find, or turns an open one into Replace.
    fn open_find(&mut self, replacing: bool) {
        if let Some(Dialog::Find { replacing: on, .. }) = &mut self.dialog {
            *on |= replacing;
            return;
        }
        self.dialog = Some(Dialog::Find {
            query: ss_formula::find::Query::default(),
            with: String::new(),
            replacing,
            whole_workbook: false,
            report: String::new(),
        });
    }

    /// The prompt shown when something would discard unsaved changes.
    fn unsaved_prompt(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        // Named in the question rather than in the body: "Unsaved changes" over
        // "this workbook has changes" says the same thing twice and neither
        // time says which workbook. Office asks about the file by name, and the
        // buttons answer the question as asked.
        let heading = match &self.path {
            Some(path) => format!("Save changes to {}?", name_of(path)),
            None => "Save changes to this workbook?".to_string(),
        };
        let answer = dialog::message(
            ctx,
            "unsaved",
            dialog::Severity::Warning,
            &heading,
            "Your changes will be lost if you don't save them.",
            None,
            &[
                dialog::Choice::new("Save").primary(),
                dialog::Choice::new("Don't Save"),
                dialog::Choice::new("Cancel").escapes(),
            ],
        );
        let choice = match answer {
            Some(0) => Some(Choice::Save),
            Some(1) => Some(Choice::Discard),
            Some(_) => Some(Choice::Cancel),
            None => None,
        };

        match choice {
            Some(Choice::Save) => {
                self.save();
                // Still edited means the save failed or was cancelled, and the
                // thing it was standing in the way of must not happen.
                if !self.edited {
                    self.pending = None;
                    self.finish(pending, ctx);
                }
            }
            Some(Choice::Discard) => {
                self.edited = false;
                self.pending = None;
                self.finish(pending, ctx);
            }
            Some(Choice::Cancel) => self.pending = None,
            None => {}
        }
    }

    fn finish(&mut self, pending: Pending, ctx: &egui::Context) {
        if pending == Pending::Quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.proceed(pending);
    }

    /// Applies a change, records how to undo it, and recalculates.
    ///
    /// Doing anything new drops the redo stack — the history is a line, not a
    /// tree, which is what every editor a user has ever met does.
    fn perform(&mut self, change: Change) {
        if change.is_empty() {
            return;
        }
        if let Some(refusal) = self.protection_refuses(&change) {
            self.status = refusal;
            return;
        }
        // Any edit dismisses the marching ants, as Excel's does. A paste that
        // wants to keep them (copy can be pasted repeatedly) puts them back.
        self.grid.marquee = None;
        let undo = edit::apply(&mut self.doc.workbook, change);
        self.undo.push(undo);
        self.redo.clear();
        self.edited = true;
        self.recalculate();
    }

    /// Why a protected sheet will not take this change, or `None` if it will.
    ///
    /// One test at the one place every change passes through, rather than a
    /// check at each of the thirty commands that can make one. A guard per
    /// command is a guard somebody forgets to add, and the patches say exactly
    /// what is about to happen — which is more than the command's name does.
    ///
    /// Undo and redo deliberately do not come through here: they apply their
    /// changes directly. Protecting a sheet must stay undoable, and nothing can
    /// be undone that protection did not already allow to happen.
    fn protection_refuses(&self, change: &Change) -> Option<String> {
        for patch in &change.patches {
            let refusal = match patch {
                Patch::Cells { sheet, cells } => self.cells_refusal(*sheet, cells),
                // A formula's *text* rewritten in place: the cells that hold
                // it are what protection is about, and the sheet's own
                // permission to be edited at all is the closest question.
                Patch::Formulas { sheet, .. } => self.refuse(*sheet, |p| p.format_cells, "edited"),
                Patch::Permute { sheet, .. } => self.refuse(*sheet, |p| p.sort, "sorted"),
                Patch::Shift { sheet, shift } => {
                    let inserting = shift.count > 0;
                    match (shift.axis, inserting) {
                        (Axis::Rows, true) => self.refuse(*sheet, |p| p.insert_rows, "added"),
                        (Axis::Rows, false) => self.refuse(*sheet, |p| p.delete_rows, "deleted"),
                        (Axis::Columns, true) => self.refuse(*sheet, |p| p.insert_columns, "added"),
                        (Axis::Columns, false) => {
                            self.refuse(*sheet, |p| p.delete_columns, "deleted")
                        }
                    }
                }
                // A move is an insert and a delete at once, and needs both.
                Patch::Rearrange { sheet, rearrange } => match rearrange.axis {
                    Axis::Rows => self.refuse(*sheet, |p| p.insert_rows && p.delete_rows, "moved"),
                    Axis::Columns => {
                        self.refuse(*sheet, |p| p.insert_columns && p.delete_columns, "moved")
                    }
                },
                Patch::Geometry { sheet, geometry } => self.geometry_refusal(*sheet, geometry),
                Patch::AxisStyles { sheet, axis, .. } => match axis {
                    Axis::Rows => self.refuse(*sheet, |p| p.format_rows, "formatted"),
                    Axis::Columns => self.refuse(*sheet, |p| p.format_columns, "formatted"),
                },
                Patch::Validations { sheet, .. } | Patch::ConditionalFormats { sheet, .. } => {
                    self.refuse(*sheet, |p| p.format_cells, "formatted")
                }
                Patch::Pictures { sheet, .. }
                | Patch::Charts { sheet, .. }
                | Patch::ChartTitle { sheet, .. } => self.refuse(*sheet, |p| p.objects, "changed"),
                Patch::Filter { sheet, .. } => self.refuse(*sheet, |p| p.filter, "filtered"),
                // Protecting and unprotecting, and everything that belongs to
                // the workbook rather than to a sheet: a protected sheet can
                // still be renamed, hidden, or dragged to another position,
                // because sheet protection is about the cells in it.
                _ => None,
            };
            if refusal.is_some() {
                return refusal;
            }
        }
        None
    }

    /// Whether protection stands in the way — `Some` when it does.
    fn forbidden(
        &self,
        sheet: usize,
        allowed: impl Fn(&ss_model::Protection) -> bool,
    ) -> Option<()> {
        let protection = self.doc.workbook.sheet(sheet)?.protection.as_ref()?;
        (!allowed(protection)).then_some(())
    }

    fn refuse(
        &self,
        sheet: usize,
        allowed: impl Fn(&ss_model::Protection) -> bool,
        verb: &str,
    ) -> Option<String> {
        self.forbidden(sheet, allowed)
            .map(|()| format!("A protected sheet cannot have that {verb}"))
    }

    /// Whether a protected sheet takes these cells.
    ///
    /// A cell's *value* may only change when the cell is unlocked, and its
    /// *look* only when the sheet allows formatting — which are different
    /// permissions on the same patch, so the two are told apart by comparing
    /// with what is there now.
    fn cells_refusal(
        &self,
        sheet: usize,
        cells: &[(CellRef, Option<ss_model::Cell>)],
    ) -> Option<String> {
        let target = self.doc.workbook.sheet(sheet)?;
        let protection = target.protection.as_ref()?;
        for (at, after) in cells {
            let before = target.get(*at);
            let value_changed = match (before, after) {
                (Some(a), Some(b)) => a.value != b.value || a.formula != b.formula,
                (None, Some(b)) => !b.value.is_blank() || b.formula.is_some(),
                (Some(a), None) => !a.value.is_blank() || a.formula.is_some(),
                (None, None) => false,
            };
            if value_changed {
                if !self.doc.workbook.styles.locked(target.style_at(*at)) {
                    continue;
                }
                return Some(format!(
                    "{} is locked, and the sheet is protected",
                    at.to_a1()
                ));
            }
            if !protection.format_cells {
                return Some("A protected sheet cannot have its cells formatted".to_string());
            }
        }
        None
    }

    /// Whether a protected sheet takes this geometry.
    fn geometry_refusal(&self, sheet: usize, wanted: &Geometry) -> Option<String> {
        let target = self.doc.workbook.sheet(sheet)?;
        let now = Geometry::of(target);
        // A division is a way of looking at the sheet rather than a change to
        // it, and Excel lets a protected sheet be frozen and split freely.
        if now.row_heights != wanted.row_heights || now.row_outlines != wanted.row_outlines {
            self.refuse(sheet, |p| p.format_rows, "resized")?;
        }
        if now.column_widths != wanted.column_widths
            || now.column_outlines != wanted.column_outlines
        {
            self.refuse(sheet, |p| p.format_columns, "resized")?;
        }
        if now.merges != wanted.merges {
            self.refuse(sheet, |p| p.format_cells, "merged")?;
        }
        None
    }

    fn recalculate(&mut self) {
        let result = ss_formula::recalculate(&mut self.doc.workbook);
        if !result.circular.is_empty() {
            self.status = format!("circular reference in {} cells", result.circular.len());
        }
    }

    fn undo(&mut self) {
        if let Some(change) = self.undo.pop() {
            let label = change.label.clone();
            let redo = edit::apply(&mut self.doc.workbook, change);
            self.redo.push(redo);
            self.recalculate();
            self.status = format!("Undo {label}");
        }
    }

    fn redo(&mut self) {
        if let Some(change) = self.redo.pop() {
            let label = change.label.clone();
            let undo = edit::apply(&mut self.doc.workbook, change);
            self.undo.push(undo);
            self.recalculate();
            self.status = format!("Redo {label}");
        }
    }

    fn act(&mut self, ui: &egui::Ui, action: Action) {
        let sheet = self.grid.sheet_index;
        match action {
            Action::Commit { at, text, advance } => {
                // A pivot table's cells are written by its definition, and an
                // edit to one leaves the file self-contradictory: Excel throws
                // the edit away at the next refresh. Refusing is the only
                // answer that does not quietly lose the user's typing.
                if let Some(pivot) = self.doc.workbook.sheet(sheet).and_then(|s| s.pivot_at(at)) {
                    self.status = format!(
                        "{} is inside pivot table \"{}\" and cannot be edited here",
                        at.to_a1(),
                        pivot.name
                    );
                    return;
                }
                // Validation runs on what the entry *becomes*, not on the
                // characters typed: a rule about whole numbers has to see the
                // number, and a date rule has to see the serial.
                let typed = edit::typed_value(&text);
                if let Some(refusal) = cond::validate(&self.doc.workbook, sheet, at, &typed) {
                    let blocks = refusal.blocks();
                    self.status = refusal.message;
                    if blocks {
                        // The cell keeps what it had and the cursor stays put,
                        // so the user can see what was refused and fix it.
                        return;
                    }
                }
                let change = edit::input(&mut self.doc.workbook, sheet, at, &text);
                self.perform(change);
                // Re-borrowed after the mutation, not held across it.
                if let (Some(direction), Some(s)) = (advance, self.doc.workbook.sheet(sheet)) {
                    self.grid.selection.advance(direction, s);
                }
            }
            Action::CommitAll { at, text } => {
                // The same refusal a single commit gets, judged once at the
                // active cell.
                let typed = edit::typed_value(&text);
                if let Some(refusal) = cond::validate(&self.doc.workbook, sheet, at, &typed) {
                    let blocks = refusal.blocks();
                    self.status = refusal.message;
                    if blocks {
                        return;
                    }
                }
                // Excel fills a whole-column selection to the bottom of the
                // sheet; a bounded fill with a message beats an unbounded
                // freeze, and beats a silent one.
                const MOST: usize = 100_000;
                let mut targets = Vec::new();
                let mut seen = std::collections::HashSet::new();
                let mut clipped = false;
                'ranges: for range in self.grid.selection.ranges() {
                    for row in range.start.row..=range.end.row {
                        for col in range.start.col..=range.end.col {
                            if targets.len() >= MOST {
                                clipped = true;
                                break 'ranges;
                            }
                            let cell = CellRef::new(row, col);
                            let Some(s) = self.doc.workbook.sheet(sheet) else {
                                break 'ranges;
                            };
                            // Covered merge cells and pivot cells take no
                            // writes, the same as they refuse single edits.
                            if s.merge_at(cell).is_some_and(|m| m.start != cell)
                                || s.pivot_at(cell).is_some()
                            {
                                continue;
                            }
                            if seen.insert(cell) {
                                targets.push(cell);
                            }
                        }
                    }
                }
                let change = edit::input_many(&mut self.doc.workbook, sheet, at, &targets, &text);
                self.perform(change);
                if clipped {
                    self.status = format!("Filled the first {MOST} cells of the selection");
                }
            }
            Action::Clear => {
                let ranges = self.grid.selection.ranges().to_vec();
                let change = edit::clear_contents(&self.doc.workbook, sheet, &ranges);
                self.perform(change);
            }
            Action::Insert(axis) => self.structural(axis, false),
            Action::Delete(axis) => self.structural(axis, true),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Copy { cut } => self.copy(ui, cut),
            Action::Paste(text) => self.paste(text),
            Action::MoveRange { from, to, copy } => {
                let end = CellRef::new(to.row + from.rows() - 1, to.col + from.cols() - 1);
                if copy {
                    // Ctrl-drag: a copy, with copy semantics — relative
                    // references adjust by the distance travelled.
                    if let Some(source) = clip::copy(&self.doc.workbook, sheet, from) {
                        let mut change = clip::paste(
                            &mut self.doc.workbook,
                            sheet,
                            CellRange::new(to, to),
                            &source,
                        );
                        change.label = "Copy cells".to_string();
                        self.perform(change);
                    }
                } else {
                    let change = edit::move_range(&mut self.doc.workbook, sheet, from, to);
                    self.perform(change);
                }
                // The selection lands on the block where it came down.
                self.grid.selection = grid::Selection::at(to);
                if let Some(s) = self.doc.workbook.sheet(sheet) {
                    self.grid.selection.extend_to(end, s);
                }
            }
            // Enter over the marching ants: paste what the app itself holds.
            Action::PasteClip => {
                let text = self.clip_text.clone();
                self.paste(text);
            }
            Action::CancelClipboard => {
                // The grid has already dropped the ants; what must not
                // survive them is the pending cut, or a later paste would
                // still move cells nobody can see are marked.
                self.cut_from = None;
            }
            Action::Fill { from, to, toggle } => {
                let change = clip::fill_series(&mut self.doc.workbook, sheet, from, to, toggle);
                self.perform(change);
                self.grid.selection = grid::Selection::at(to.start);
                if let Some(s) = self.doc.workbook.sheet(sheet) {
                    self.grid.selection.extend_to(to.end, s);
                }
            }
            Action::Format(command) => self.format(command),
            Action::StepSheet(step) => self.step_sheet(step),
            Action::Merge(join) => self.merge(join),
            Action::EditNote => self.open_note(),
            Action::Refused(at) => {
                self.status = format!("{} is locked, and the sheet is protected", at.to_a1());
            }
            Action::Freeze(on) => self.divide(on, true),
            Action::Split(on) => self.divide(on, false),
            Action::Visibility { axis, hide } => self.set_visibility(axis, hide),
            Action::Group { axis, ungroup } => self.group_selection(axis, ungroup),
            Action::ToggleOutline { axis, index } => self.toggle_outline(axis, index),
            Action::AutoFit(axis) => self.autofit(axis),
            Action::AutoFitAt { axis, index } => self.autofit_span(axis, index, index),
            Action::MoveBand {
                axis,
                first,
                last,
                before,
            } => self.move_band(axis, first, last, before),
            Action::PicturesMoved(before) => self.pictures_moved(sheet, before),
            Action::DeletePicture(index) => self.delete_picture(sheet, index),
            Action::ChartsMoved(before) => self.charts_moved(sheet, before),
            Action::DeleteChart(index) => self.delete_chart(sheet, index),
            Action::FilterMenu(col) => self.open_filter_menu(col),
            Action::Resized(before) => {
                // A press on a boundary that never moved is not an edit. It
                // happens on every double-click to autofit, which presses
                // twice before the gesture is recognised, and an undo stack
                // with two do-nothing entries on top of the fit is worse than
                // useless.
                if self.doc.workbook.sheet(sheet).is_some_and(|s| {
                    s.column_widths == before.column_widths && s.row_heights == before.row_heights
                }) {
                    return;
                }
                // The sheet already has the new sizes; what goes on the stack is
                // how it looked before, which is the undo.
                self.undo.push(Change::new(
                    "Resize",
                    vec![Patch::Geometry {
                        sheet,
                        geometry: before,
                    }],
                ));
                self.redo.clear();
                self.edited = true;
            }
        }
    }

    /// The next or previous *visible* sheet, skipping hidden ones and the
    /// chart sheets that have no grid to show.
    fn step_sheet(&mut self, step: i32) {
        let showable: Vec<usize> = (0..self.doc.workbook.sheets.len())
            .filter(|i| {
                let sheet = &self.doc.workbook.sheets[*i];
                sheet.kind.has_grid() && !sheet.hidden
            })
            .collect();
        let Some(at) = showable.iter().position(|i| *i == self.grid.sheet_index) else {
            return;
        };
        let next = (at as i32 + step).clamp(0, showable.len() as i32 - 1) as usize;
        if showable[next] != self.grid.sheet_index {
            self.show_sheet(showable[next]);
        }
    }

    fn show_sheet(&mut self, index: usize) {
        self.grid.commit(None);
        for action in self.grid.take_actions() {
            self.act_headless(action);
        }
        self.grid.open_sheet(&self.doc.workbook, index);
        self.status = self.doc.workbook.sheets[index].name.clone();
    }

    // ---- Sheets -----------------------------------------------------------

    /// A new empty sheet after the one showing, and switches to it.
    fn add_sheet(&mut self) {
        let at = self.grid.sheet_index + 1;
        let name = self.doc.workbook.next_sheet_name();
        let change = ss_formula::sheets::insert(&self.doc.workbook, at, &name);
        self.perform(change);
        self.show_sheet(at);
        self.status = format!("Added {name}");
    }

    /// Deletes a sheet, refusing to leave the workbook with nothing to show.
    ///
    /// No confirmation, because there is a full undo behind it and the data
    /// comes back with the tab. Excel asks because *its* delete is permanent.
    fn delete_sheet(&mut self, index: usize) {
        if self.visible_sheets() <= 1 && !self.doc.workbook.sheets[index].hidden {
            self.status = "A workbook needs one visible sheet".to_string();
            return;
        }
        let name = self.doc.workbook.sheets[index].name.clone();
        let change = ss_formula::sheets::remove(&self.doc.workbook, index);
        self.perform(change);
        let showing = self
            .grid
            .sheet_index
            .min(self.doc.workbook.sheets.len() - 1);
        self.show_sheet(showing);
        self.status = format!("Deleted {name} — Ctrl+Z brings it back");
    }

    fn rename_sheet(&mut self, index: usize, to: &str) {
        let to = to.trim();
        if let Some(refusal) = self.doc.workbook.sheet_name_refusal(to, Some(index)) {
            self.status = refusal;
            return;
        }
        let change = ss_formula::sheets::rename(&self.doc.workbook, index, to);
        self.perform(change);
        self.status = format!("Renamed to {to}");
    }

    /// Moves a tab, or drops a copy of it, before position `before`.
    fn move_sheet(&mut self, index: usize, before: usize, copy: bool) {
        let landing = if copy || before <= index {
            before
        } else {
            // Removing the tab first shifts everything after it down one, so
            // "before the fifth" is position four once the fourth has gone.
            before - 1
        };
        let change = if copy {
            let base = self.doc.workbook.sheets[index].name.clone();
            let name = self.doc.workbook.unique_sheet_name(&base);
            ss_formula::sheets::duplicate(&self.doc.workbook, index, landing, &name)
        } else {
            ss_formula::sheets::reorder(&self.doc.workbook, index, landing)
        };
        if change.is_empty() {
            return;
        }
        self.perform(change);
        self.show_sheet(landing.min(self.doc.workbook.sheets.len() - 1));
    }

    fn set_tab_color(&mut self, index: usize, color: Option<Color>) {
        let change = ss_formula::sheets::set_tab_color(index, color);
        self.perform(change);
    }

    // ---- Sort and filter --------------------------------------------------

    /// The range a sort or a filter is about.
    ///
    /// A dragged selection means itself. A single cell means the block it is
    /// standing in, which is what Excel does and what makes A→Z a one-click
    /// operation on a table rather than a way to scramble one column against
    /// its neighbours.
    ///
    /// Either way the answer is trimmed to the data. Whole rows, whole columns
    /// and select-all are selections of a million cells that hold a hundred,
    /// and everything downstream — the dialog's range, its column list, the
    /// header guess, the status line — should be about the hundred.
    fn data_range(&self) -> Option<CellRange> {
        let sheet = self.doc.workbook.sheet(self.grid.sheet_index)?;
        let selected = self.grid.selection.active_range();
        if selected.rows() > 1 || selected.cols() > 1 {
            return ss_formula::sort::clamped(sheet, selected);
        }
        Some(ss_formula::sort::region(
            sheet,
            self.grid.selection.cursor(),
        ))
    }

    /// A→Z or Z→A on the cursor's column.
    fn sort_quick(&mut self, descending: bool) {
        let Some(range) = self.data_range() else {
            return;
        };
        let sheet_index = self.grid.sheet_index;
        let col = self
            .grid
            .selection
            .cursor()
            .col
            .clamp(range.start.col, range.end.col);
        let header = self
            .doc
            .workbook
            .sheet(sheet_index)
            .is_some_and(|s| ss_formula::sort::looks_like_headers(s, range));
        let keys = [ss_formula::sort::SortKey { col, descending }];
        match ss_formula::sort::sort(&mut self.doc.workbook, sheet_index, range, &keys, header) {
            Ok(change) if change.is_empty() => {
                self.status = "Already in that order".to_string();
            }
            Ok(change) => {
                self.perform(change);
                self.grid.invalidate();
                self.status = format!(
                    "Sorted {} by column {}{}",
                    range_label(range),
                    ss_model::column_name(col),
                    if header {
                        " (first row kept as headings)"
                    } else {
                        ""
                    }
                );
            }
            Err(why) => self.status = why,
        }
    }

    /// Opens the multi-level sort dialog over the range the cursor is in.
    fn open_sort_dialog(&mut self) {
        let Some(range) = self.data_range() else {
            return;
        };
        let header = self
            .doc
            .workbook
            .sheet(self.grid.sheet_index)
            .is_some_and(|s| ss_formula::sort::looks_like_headers(s, range));
        let mut levels = [SortLevel::default(); 3];
        levels[0].col = Some(
            self.grid
                .selection
                .cursor()
                .col
                .clamp(range.start.col, range.end.col),
        );
        self.dialog = Some(Dialog::Sort {
            range,
            header,
            levels,
        });
    }

    /// Runs what the sort dialog was left holding.
    fn sort_by(&mut self, range: CellRange, header: bool, levels: &[SortLevel]) {
        let keys: Vec<ss_formula::sort::SortKey> = levels
            .iter()
            .filter_map(|level| {
                Some(ss_formula::sort::SortKey {
                    col: level.col?,
                    descending: level.descending,
                })
            })
            .collect();
        if keys.is_empty() {
            self.status = "Choose a column to sort by".to_string();
            return;
        }
        let sheet = self.grid.sheet_index;
        match ss_formula::sort::sort(&mut self.doc.workbook, sheet, range, &keys, header) {
            Ok(change) if change.is_empty() => self.status = "Already in that order".to_string(),
            Ok(change) => {
                self.perform(change);
                self.grid.invalidate();
                self.status = format!("Sorted {} by {} column(s)", range_label(range), keys.len());
            }
            Err(why) => self.status = why,
        }
    }

    /// Puts arrows on the current block, or takes them off.
    fn toggle_filter(&mut self) {
        let sheet = self.grid.sheet_index;
        let has = self
            .doc
            .workbook
            .sheet(sheet)
            .is_some_and(|s| s.filter.is_some());
        if has {
            let change = ss_formula::filter::remove(&self.doc.workbook, sheet);
            self.perform(change);
            self.grid.invalidate();
            self.status = "Filter removed".to_string();
            return;
        }
        let Some(range) = self.data_range() else {
            return;
        };
        if range.rows() < 2 {
            self.status = "A filter needs a heading row and at least one row under it".to_string();
            return;
        }
        let change = Change::new(
            "Filter",
            vec![Patch::Filter {
                sheet,
                filter: Some(ss_model::AutoFilter::over(range)),
            }],
        );
        self.perform(change);
        self.grid.invalidate();
        self.status = format!("Filter over {}", range_label(range));
    }

    /// Opens the value list behind one arrow.
    fn open_filter_menu(&mut self, col: u32) {
        let sheet = self.grid.sheet_index;
        let (offered, has_blanks) = ss_formula::filter::distinct(&self.doc.workbook, sheet, col);
        let existing = self
            .doc
            .workbook
            .sheet(sheet)
            .and_then(|s| s.filter.as_ref())
            .and_then(|f| f.column(col))
            .map(|c| c.kind.clone());
        // A column with no constraint starts with everything ticked, which is
        // what "showing everything" looks like as a checkbox list.
        let (ticked, blanks) = match existing {
            Some(ss_model::FilterKind::Values { values, blanks }) => (values, blanks),
            _ => (offered.clone(), has_blanks),
        };
        self.dialog = Some(Dialog::Filter {
            col,
            offered: offered.into_iter().collect(),
            has_blanks,
            ticked,
            blanks,
            search: String::new(),
        });
    }

    /// Applies one column's tick list and re-runs the whole filter.
    fn set_filter_column(
        &mut self,
        col: u32,
        ticked: std::collections::BTreeSet<String>,
        blanks: bool,
        offered: &[String],
        has_blanks: bool,
    ) {
        let sheet = self.grid.sheet_index;
        let Some(mut filter) = self
            .doc
            .workbook
            .sheet(sheet)
            .and_then(|s| s.filter.clone())
        else {
            return;
        };
        // Everything ticked is no constraint at all, and writing one would
        // leave the column wearing a funnel that hides nothing.
        let everything = ticked.len() == offered.len() && blanks == has_blanks;
        filter.set(
            col,
            (!everything).then_some(ss_model::FilterKind::Values {
                values: ticked,
                blanks,
            }),
        );
        let change = Change::new(
            "Filter",
            vec![Patch::Filter {
                sheet,
                filter: Some(filter),
            }],
        );
        self.perform(change);
        self.reapply_filter();
    }

    /// Recomputes which rows the filter hides.
    fn reapply_filter(&mut self) {
        let sheet = self.grid.sheet_index;
        let change = ss_formula::filter::apply(&self.doc.workbook, sheet);
        if !change.is_empty() {
            self.perform(change);
        }
        self.grid.invalidate();
        let showing = self
            .doc
            .workbook
            .sheet(sheet)
            .and_then(|s| s.filter.as_ref())
            .map(|f| {
                let total = f.range.end.row - f.first_data_row() + 1;
                let hidden = self
                    .doc
                    .workbook
                    .sheet(sheet)
                    .map(|s| {
                        (f.first_data_row()..=f.range.end.row)
                            .filter(|r| s.row_heights.get(r) == Some(&0.0))
                            .count()
                    })
                    .unwrap_or(0);
                (total as usize - hidden, total as usize)
            });
        if let Some((shown, total)) = showing {
            self.status = format!("{shown} of {total} rows");
        }
    }

    /// Clears every column's criteria, keeping the arrows.
    fn clear_filter(&mut self) {
        let change = ss_formula::filter::clear(&self.doc.workbook, self.grid.sheet_index);
        if change.is_empty() {
            return;
        }
        self.perform(change);
        self.grid.invalidate();
        self.status = "Filter cleared".to_string();
    }

    /// Merges the selection into one cell, or takes apart the merges under it.
    ///
    /// Merging keeps only the top-left value, which is what Excel does and warns
    /// about. The values it drops go onto the undo stack with the merge itself,
    /// so one Ctrl+Z brings both the grid and the text back.
    fn merge(&mut self, join: bool) {
        let sheet = self.grid.sheet_index;
        let range = self.grid.selection.active_range();
        let Some(current) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        let mut geometry = Geometry::of(current);
        let mut cleared = Vec::new();

        if join {
            if range.rows() == 1 && range.cols() == 1 {
                self.status = "Select two or more cells to merge".to_string();
                return;
            }
            geometry.merges.retain(|m| !overlaps(*m, range));
            geometry.merges.push(range);
            for (at, _) in current.cells.iter() {
                if at != range.start && range.contains(at) {
                    cleared.push((at, None));
                }
            }
        } else {
            let before = geometry.merges.len();
            geometry.merges.retain(|m| !overlaps(*m, range));
            if geometry.merges.len() == before {
                self.status = "Nothing merged here".to_string();
                return;
            }
        }

        let mut patches = vec![Patch::Geometry { sheet, geometry }];
        if !cleared.is_empty() {
            patches.push(Patch::Cells {
                sheet,
                cells: cleared,
            });
        }
        let label = if join { "Merge" } else { "Unmerge" };
        self.perform(Change::new(label, patches));
        self.grid.invalidate();
    }

    /// Divides the sheet at the cursor, or puts it back together.
    ///
    /// Freezing and splitting are one operation with one difference — whether
    /// the bands before the division are pinned — and a sheet has one division,
    /// so asking for either replaces the other rather than stacking on it.
    fn divide(&mut self, on: bool, frozen: bool) {
        let sheet = self.grid.sheet_index;
        let Some(current) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        let mut geometry = Geometry::of(current);
        let cursor = self.grid.selection.cursor();
        geometry.panes = on
            .then_some(cursor)
            .filter(|at| at.row > 0 || at.col > 0)
            .map(|at| ss_model::Panes { at, frozen });
        if geometry.panes == current.panes {
            return;
        }
        let label = match (on, frozen) {
            (true, true) => "Freeze panes",
            (true, false) => "Split panes",
            (false, true) => "Unfreeze panes",
            (false, false) => "Remove split",
        };
        self.perform(Change::new(
            label,
            vec![Patch::Geometry { sheet, geometry }],
        ));
        self.grid.invalidate();
    }

    /// The sheets a search covers, starting at the one being looked at.
    ///
    /// Order matters: Find Next from a cell on sheet 3 should reach the rest of
    /// sheet 3 before it reaches sheet 1, and "reading order" across a workbook
    /// is therefore a rotation of the sheet list rather than the list.
    fn search_scope(&self, whole_workbook: bool) -> Vec<usize> {
        let here = self.grid.sheet_index;
        if !whole_workbook {
            return vec![here];
        }
        let count = self.doc.workbook.sheets.len();
        (0..count)
            .map(|n| (here + n) % count)
            .filter(|i| {
                self.doc.workbook.sheets[*i].kind.has_grid() && !self.doc.workbook.sheets[*i].hidden
            })
            .collect()
    }

    /// Moves the cursor to a hit, switching sheets if it is on another one.
    fn show_hit(&mut self, hit: ss_formula::find::Hit) {
        if hit.sheet != self.grid.sheet_index {
            self.show_sheet(hit.sheet);
        }
        self.grid.selection = grid::Selection::at(hit.at);
        let body = self.last_body;
        if let Some(sheet) = self.doc.workbook.sheet(hit.sheet) {
            self.grid
                .scroll_into_view(hit.at, body, &self.doc.workbook, sheet);
        }
    }

    /// One press of a Find window button, and what to say about it.
    fn find_command(
        &mut self,
        command: FindCommand,
        query: &ss_formula::find::Query,
        with: &str,
        whole_workbook: bool,
    ) -> String {
        use ss_formula::find;
        let scope = self.search_scope(whole_workbook);
        if query.needle.is_empty() {
            return "Type something to look for".to_string();
        }
        let here = find::Hit {
            sheet: self.grid.sheet_index,
            at: self.grid.selection.cursor(),
        };
        match command {
            FindCommand::Next | FindCommand::Previous => {
                let back = command == FindCommand::Previous;
                match find::next(&self.doc.workbook, &scope, here, query, back) {
                    Some(hit) => {
                        self.show_hit(hit);
                        let sheet = self
                            .doc
                            .workbook
                            .sheet(hit.sheet)
                            .map(|s| s.name.clone())
                            .unwrap_or_default();
                        format!("{} on {sheet}", hit.at.to_a1())
                    }
                    None => "No match".to_string(),
                }
            }
            FindCommand::ReplaceOne => {
                // The cell the cursor is on, if it is one of the matches —
                // Excel replaces where you are standing and then moves on, so
                // that Replace repeated is Replace All done by hand.
                let cell = self
                    .doc
                    .workbook
                    .sheet(here.sheet)
                    .and_then(|s| s.get(here.at))
                    .is_some();
                let hits = find::all(&self.doc.workbook, &scope, query);
                let standing_on = cell && hits.contains(&here);
                let (change, report) = if standing_on {
                    find::replace(&mut self.doc.workbook, &[here], query, with)
                } else {
                    (Change::default(), find::Replaced::default())
                };
                if !change.patches.is_empty() {
                    self.perform(change);
                }
                let moved = find::next(&self.doc.workbook, &scope, here, query, false);
                if let Some(hit) = moved {
                    self.show_hit(hit);
                }
                match (report.cells, report.skipped) {
                    (0, 0) if standing_on => "Nothing to replace here".to_string(),
                    (0, 0) => "Find first, then replace".to_string(),
                    (0, _) => "That cell is a formula; its value cannot be replaced".to_string(),
                    (n, _) => format!("Replaced {n}"),
                }
            }
            FindCommand::ReplaceAll => {
                let hits = find::all(&self.doc.workbook, &scope, query);
                let (change, report) = find::replace(&mut self.doc.workbook, &hits, query, with);
                if change.patches.is_empty() {
                    return "No match".to_string();
                }
                // One entry on the undo stack, whatever it touched: undoing a
                // Replace All in three hundred steps is not undoing it.
                self.perform(change);
                match report.skipped {
                    0 => format!("Replaced {} cells", report.cells),
                    n => format!(
                        "Replaced {} cells; {n} left alone, being formulas found by their value",
                        report.cells
                    ),
                }
            }
        }
    }

    /// Puts a dragged band of rows or columns down where it was dropped.
    fn move_band(&mut self, axis: Axis, first: u32, last: u32, before: u32) {
        let sheet = self.grid.sheet_index;
        let mv = ss_model::Move::new(axis, first, last, before);
        let change = edit::move_band(&self.doc.workbook, sheet, mv);
        if change.patches.is_empty() {
            return;
        }
        self.perform(change);
        // The selection follows the band. Watching it stay behind on the
        // columns the move pushed into its old place is disorienting, and it
        // also makes a second drag move the wrong ones.
        let (landed_first, landed_last) = mv.landing();
        match axis {
            Axis::Rows => self
                .grid
                .selection
                .select_rows(landed_first, landed_last, false),
            Axis::Columns => self
                .grid
                .selection
                .select_columns(landed_first, landed_last, false),
        }
        self.grid.invalidate();
        self.status = format!(
            "Moved {} {}",
            landed_last - landed_first + 1,
            match axis {
                Axis::Rows => "rows",
                Axis::Columns => "columns",
            }
        );
    }

    /// Hides or shows the selected rows or columns.
    ///
    /// Hiding is a size of zero, which is exactly how the file stores it — a
    /// hidden row is `hidden="1"` with no height, and a height of zero is what
    /// that means on screen.
    /// Shift+Alt+Right and its inverse: the selected rows or columns go one
    /// outline level deeper or shallower.
    fn group_selection(&mut self, axis: Axis, ungroup: bool) {
        let sheet = self.grid.sheet_index;
        let Some(current) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        let mut geometry = Geometry::of(current);
        let range = self.grid.selection.active_range();
        let (from, to) = match axis {
            Axis::Rows => (range.start.row, range.end.row),
            Axis::Columns => (range.start.col, range.end.col),
        };
        let to = to.min(from.saturating_add(4096));
        let outlines = match axis {
            Axis::Rows => &mut geometry.row_outlines,
            Axis::Columns => &mut geometry.column_outlines,
        };
        for index in from..=to {
            let level = outlines.get(&index).copied().unwrap_or(0);
            let next = if ungroup {
                level.saturating_sub(1)
            } else {
                (level + 1).min(7)
            };
            if next == 0 {
                outlines.remove(&index);
            } else {
                outlines.insert(index, next);
            }
        }
        self.perform(Change::new(
            if ungroup { "Ungroup" } else { "Group" },
            vec![Patch::Geometry { sheet, geometry }],
        ));
        self.grid.invalidate();
        let what = match axis {
            Axis::Rows => "rows",
            Axis::Columns => "columns",
        };
        self.status = format!(
            "{} {} {}–{}",
            if ungroup { "Ungrouped" } else { "Grouped" },
            what,
            from + 1,
            to + 1
        );
    }

    /// A collapse button: hides or reveals the contiguous group ending just
    /// before the summary line that carries the button.
    fn toggle_outline(&mut self, axis: Axis, index: u32) {
        let sheet = self.grid.sheet_index;
        let Some(current) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        let mut geometry = Geometry::of(current);
        let levels = match axis {
            Axis::Rows => geometry.row_outlines.clone(),
            Axis::Columns => geometry.column_outlines.clone(),
        };
        let Some(prev) = index.checked_sub(1) else {
            return;
        };
        let level = levels.get(&prev).copied().unwrap_or(0);
        if level == 0 {
            return;
        }
        let mut first = prev;
        while first > 0 && levels.get(&(first - 1)).copied().unwrap_or(0) >= level {
            first -= 1;
        }
        let (sizes, collapsed) = match axis {
            Axis::Rows => (&mut geometry.row_heights, &mut geometry.row_collapsed),
            Axis::Columns => (&mut geometry.column_widths, &mut geometry.column_collapsed),
        };
        let expanding = collapsed.contains(&index);
        for i in first..index {
            if expanding {
                sizes.remove(&i);
            } else {
                sizes.insert(i, 0.0);
            }
        }
        if expanding {
            collapsed.remove(&index);
        } else {
            collapsed.insert(index);
        }
        self.perform(Change::new(
            if expanding { "Expand" } else { "Collapse" },
            vec![Patch::Geometry { sheet, geometry }],
        ));
        self.grid.invalidate();
    }

    fn set_visibility(&mut self, axis: Axis, hide: bool) {
        let sheet = self.grid.sheet_index;
        let Some(current) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        let mut geometry = Geometry::of(current);
        let range = self.grid.selection.active_range();
        let (from, to) = match axis {
            Axis::Rows => (range.start.row, range.end.row),
            Axis::Columns => (range.start.col, range.end.col),
        };
        // Capped: "hide" over a select-all would otherwise store a million
        // zeroes, which is a bigger file than the sheet it came from.
        let to = to.min(from.saturating_add(4096));
        for index in from..=to {
            let sizes = match axis {
                Axis::Rows => &mut geometry.row_heights,
                Axis::Columns => &mut geometry.column_widths,
            };
            if hide {
                sizes.insert(index, 0.0);
            } else {
                sizes.remove(&index);
            }
        }
        self.perform(Change::new(
            if hide { "Hide" } else { "Unhide" },
            vec![Patch::Geometry { sheet, geometry }],
        ));
        self.grid.invalidate();
    }

    /// Sizes the selected columns or rows to their contents.
    fn autofit(&mut self, axis: Axis) {
        let range = self.grid.selection.active_range();
        let (from, to) = match axis {
            Axis::Rows => (range.start.row, range.end.row),
            Axis::Columns => (range.start.col, range.end.col),
        };
        self.autofit_span(axis, from, to);
    }

    /// Sizes one run of rows or columns to their contents.
    fn autofit_span(&mut self, axis: Axis, from: u32, to: u32) {
        let sheet_index = self.grid.sheet_index;
        let Some(sheet) = self.doc.workbook.sheet(sheet_index) else {
            return;
        };
        let mut geometry = Geometry::of(sheet);
        match axis {
            // Rows fit themselves already — every row with no stored height is
            // measured at layout time — so fitting one means forgetting
            // whatever height was stored over it.
            Axis::Rows => {
                for row in from..=to.min(from.saturating_add(4096)) {
                    geometry.row_heights.remove(&row);
                }
            }
            Axis::Columns => {
                for col in from..=to.min(from.saturating_add(256)) {
                    if let Some(chars) = fitted_width(&self.doc.workbook, sheet, col) {
                        geometry.column_widths.insert(col, chars);
                    }
                }
            }
        }
        self.perform(Change::new(
            "Autofit",
            vec![Patch::Geometry {
                sheet: sheet_index,
                geometry,
            }],
        ));
        self.grid.invalidate();
    }

    /// Opens the Column Width or Row Height box, filled in with what the
    /// selection is now — the cursor's own row or column, since a selection
    /// spanning several sizes has no one number to show.
    fn open_size_dialog(&mut self, axis: Axis) {
        let cursor = self.grid.selection.cursor();
        let sheet = self.doc.workbook.sheet(self.grid.sheet_index);
        let current = sheet.and_then(|s| match axis {
            Axis::Rows => s.row_heights.get(&cursor.row).copied(),
            Axis::Columns => s.column_widths.get(&cursor.col).copied(),
        });
        let size = current.unwrap_or(match axis {
            Axis::Rows => grid::axis::DEFAULT_ROW_POINTS,
            Axis::Columns => grid::axis::DEFAULT_COLUMN_CHARS,
        });
        self.dialog = Some(Dialog::Size {
            axis,
            // Trimmed, so a width of 8.43 does not appear as 8.4300000000001.
            text: format!("{:.2}", size)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string(),
        });
    }

    /// Applies an exact size, in the file's own units, to the selection.
    fn resize_selection(&mut self, axis: Axis, size: f64) {
        let sheet_index = self.grid.sheet_index;
        let Some(sheet) = self.doc.workbook.sheet(sheet_index) else {
            return;
        };
        let mut geometry = Geometry::of(sheet);
        let range = self.grid.selection.active_range();
        let (from, to) = match axis {
            Axis::Rows => (range.start.row, range.end.row),
            Axis::Columns => (range.start.col, range.end.col),
        };
        // Capped the same way hiding is: a size typed over a select-all would
        // otherwise store a million identical numbers.
        let to = to.min(from.saturating_add(4096));
        for index in from..=to {
            match axis {
                Axis::Rows => geometry.row_heights.insert(index, size),
                Axis::Columns => geometry.column_widths.insert(index, size),
            };
        }
        self.perform(Change::new(
            "Resize",
            vec![Patch::Geometry {
                sheet: sheet_index,
                geometry,
            }],
        ));
        self.grid.invalidate();
    }

    /// Applies a formatting command to the selection.
    ///
    /// A toggle is a toggle *of the selection*, and what it toggles to is
    /// decided by the cursor's own cell — pressing Ctrl+B over a mixed
    /// selection makes all of it bold, and pressing it again over an all-bold
    /// one takes it all off. Deciding per cell instead would make Ctrl+B
    /// invert a selection rather than set it, which is not what anyone means.
    fn format(&mut self, command: Format) {
        let sheet = self.grid.sheet_index;
        let ranges = self.grid.selection.ranges().to_vec();
        let cursor = self.grid.selection.cursor();
        let current = self
            .doc
            .workbook
            .sheet(sheet)
            .map(|s| self.doc.workbook.styles.look(s.style_at(cursor)))
            .unwrap_or_default();

        let label = format_label(&command);
        let change = match command {
            Format::Bold => {
                let on = !current.font.bold;
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.font.bold = on
                })
            }
            Format::Italic => {
                let on = !current.font.italic;
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.font.italic = on
                })
            }
            Format::Underline => {
                let on = current.font.underline.is_none();
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.font.underline = if on {
                        Underline::Single
                    } else {
                        Underline::None
                    }
                })
            }
            Format::Strike => {
                let on = !current.font.strike;
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.font.strike = on
                })
            }
            Format::Align(h) => {
                // Pressing the alignment a cell already has puts it back to
                // General, which is how Excel's buttons behave.
                let to = if current.alignment.horizontal == h {
                    HAlign::General
                } else {
                    h
                };
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.alignment.horizontal = to
                })
            }
            Format::Vertical(v) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.alignment.vertical = v
                })
            }
            Format::Wrap => {
                let on = !current.alignment.wrap;
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.alignment.wrap = on
                })
            }
            Format::Indent(by) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.alignment.indent = l.alignment.indent.saturating_add_signed(by).min(250)
                })
            }
            Format::FontName(name) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.font.name = name.clone()
                })
            }
            Format::FontSize(points) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.font.size = points
                })
            }
            Format::TextColor(color) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.font.color = color.unwrap_or(Color::Auto)
                })
            }
            Format::Fill(color) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.fill = match color {
                        Some(color) => Fill::solid(color),
                        None => Fill::default(),
                    }
                })
            }
            Format::Border(preset) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    preset.apply(&mut l.border)
                })
            }
            Format::NumberFormat(code) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    l.number_format = code.clone()
                })
            }
            Format::Whole(look) => {
                edit::format(&mut self.doc.workbook, sheet, &ranges, label, move |l| {
                    *l = (*look).clone()
                })
            }
            Format::Clear => edit::format(&mut self.doc.workbook, sheet, &ranges, label, |l| {
                *l = ss_model::Look::default()
            }),
        };
        self.perform(change);
        self.grid.invalidate();
    }

    /// The formatting under the cursor, which is what the toolbar shows as on
    /// or off. A selection can be mixed; the cursor's cell is the one the user
    /// can see the state of.
    fn cursor_look(&self) -> ss_model::Look {
        let sheet = self.grid.sheet_index;
        let cursor = self.grid.selection.cursor();
        self.doc
            .workbook
            .sheet(sheet)
            .map(|s| self.doc.workbook.styles.look(s.style_at(cursor)))
            .unwrap_or_default()
    }

    fn structural(&mut self, axis: Axis, delete: bool) {
        let sheet = self.grid.sheet_index;
        let range = self.grid.selection.active_range();
        let (at, count) = match axis {
            Axis::Rows => (range.start.row, range.rows()),
            Axis::Columns => (range.start.col, range.cols()),
        };
        let count = count.min(axis.limit() - at);
        let shift = if delete {
            Shift::delete(axis, at, count)
        } else {
            Shift::insert(axis, at, count)
        };
        let change = edit::structural(&self.doc.workbook, sheet, shift);
        self.perform(change);
        self.grid.invalidate();
    }

    fn copy(&mut self, ui: &egui::Ui, cut: bool) {
        let sheet = self.grid.sheet_index;
        let range = self.grid.selection.active_range();
        let Some(taken) = clip::copy(&self.doc.workbook, sheet, range) else {
            return;
        };
        self.clip_text = taken.to_tsv();
        ui.ctx().copy_text(self.clip_text.clone());
        self.clip = Some(taken);
        self.cut_from = cut.then_some((sheet, range));
        // The ants say what would move or be pasted; Escape dismisses them.
        self.grid.marquee = Some((sheet, range));
        self.status = if cut { "Cut" } else { "Copied" }.to_string();
    }

    fn paste(&mut self, text: String) {
        self.paste_how(text, clip::PasteSpecial::default());
    }

    /// What the OS clipboard holds right now.
    ///
    /// The menu paths need this: Ctrl+V arrives as an event carrying the
    /// text, but a click on "Paste" carries nothing, and pasting the text
    /// *we* remember would ignore whatever another program copied since.
    fn os_clipboard_text(&self) -> String {
        arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .unwrap_or_else(|_| self.clip_text.clone())
    }

    fn paste_how(&mut self, text: String, how: clip::PasteSpecial) {
        let sheet = self.grid.sheet_index;
        let target = self.grid.selection.active_range();

        // A cut pasted whole on its own sheet is a *move*, and a move is not
        // a copy: the formulas travel as written, and every reference in the
        // workbook that pointed into the block follows the data. Cross-sheet
        // cuts and partial pastes fall through to the clear-and-paste below.
        if let Some((from_sheet, from_range)) = self.cut_from {
            if from_sheet == sheet && text == self.clip_text && how == clip::PasteSpecial::default()
            {
                self.cut_from = None;
                let change =
                    edit::move_range(&mut self.doc.workbook, sheet, from_range, target.start);
                self.perform(change);
                return;
            }
        }

        // Our own clip carries formulas and styles; the system clipboard carries
        // only text. Prefer ours when the clipboard is still what we put there.
        let source = match &self.clip {
            Some(held) if text == self.clip_text => held.clone(),
            _ => Clip::from_tsv(&text, target.start),
        };

        let mut change = clip::paste_special(&mut self.doc.workbook, sheet, target, &source, how);
        // A cut is a paste that also empties where it came from, and the two
        // have to be one undo step or Ctrl-Z leaves the data in both places.
        let was_cut = self.cut_from.is_some();
        if let Some((from_sheet, from_range)) = self.cut_from.take() {
            let cleared = edit::clear_contents(&self.doc.workbook, from_sheet, &[from_range]);
            change.patches.splice(0..0, cleared.patches);
            change.label = "Move".to_string();
        }
        let ants = self.grid.marquee;
        self.perform(change);
        // A copy can be pasted again, so its ants keep marching; a cut is
        // spent the moment it lands.
        if !was_cut {
            self.grid.marquee = ants;
        }
    }

    /// The menu bar: every command the application has, in the places a
    /// spreadsheet has kept them for thirty years.
    ///
    /// A row of forty buttons is not a command surface, it is a wall, and the
    /// only way through a wall of unlabelled groups is to read all forty labels
    /// every time. Menus cost one click and give back three things a button row
    /// cannot: a name over each group of commands, room for the ones nobody
    /// needs weekly, and somewhere to print the keystroke beside the command —
    /// which is the only way anybody ever stops using the menu.
    fn menus(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut command: Option<Command> = None;

        // Everything the menus *read* is read first, into plain values. The
        // layout closures below would otherwise hold a borrow of the sheet for
        // as long as they run, and a menu that borrowed the sheet to ask
        // whether it is frozen could not then ask to unfreeze it.
        let cursor = self.grid.selection.cursor();
        let sheet = self.doc.workbook.sheet(self.grid.sheet_index);
        let merged = sheet.is_some_and(|s| s.merge_at(cursor).is_some());
        let panes = sheet.and_then(|s| s.panes);
        let frozen = panes.is_some_and(|p| p.frozen);
        let split = panes.is_some_and(|p| !p.frozen);
        let protected = sheet.is_some_and(|s| s.protection.is_some());
        let filtering = sheet.and_then(|s| s.filter.as_ref());
        let has_filter = filtering.is_some();
        let constrained = filtering.is_some_and(|f| f.is_filtering());
        let noted = sheet.is_some_and(|s| s.comments.iter().any(|note| note.at == cursor));
        let undo = self.undo.last().map(|change| change.label.clone());
        let redo = self.redo.last().map(|change| change.label.clone());
        let recent: Vec<PathBuf> = self.recent.paths().to_vec();
        let can_save = self.edited || self.path.is_none();
        let zoom = self.grid.zoom;

        menu::bar(ui, |ui| {
            menu::top(ui, "&File", |ui| {
                if menu::item(ui, "&New", "Ctrl+N").clicked() {
                    command = Some(Command::Guard(Pending::New));
                }
                if menu::item(ui, "&Open…", "Ctrl+O").clicked() {
                    command = Some(Command::Guard(Pending::Browse));
                }
                ui.add_enabled_ui(!recent.is_empty(), |ui| {
                    menu::sub(ui, "Open &Recent", |ui| {
                        for (n, path) in recent.iter().enumerate() {
                            // Numbered, so the list is a *place* rather than an
                            // order that shuffles under the pointer; and shown
                            // in full on hover, because two directories can
                            // both hold a "budget.xlsx" and only one is meant.
                            let entry =
                                menu::item(ui, &format!("&{}   {}", n + 1, name_of(path)), "")
                                    .on_hover_text(path.display().to_string());
                            if entry.clicked() {
                                command = Some(Command::Reopen(path.clone()));
                            }
                        }
                        menu::sep(ui);
                        if menu::item(ui, "Clear &List", "").clicked() {
                            command = Some(Command::ForgetRecent);
                        }
                    });
                });
                menu::sep(ui);
                ui.add_enabled_ui(can_save, |ui| {
                    if menu::item(ui, "&Save", "Ctrl+S").clicked() {
                        command = Some(Command::Save);
                    }
                });
                if menu::item(ui, "Save &As…", "Ctrl+Shift+S").clicked() {
                    command = Some(Command::SaveAs);
                }
                menu::sep(ui);
                if menu::item(ui, "&Close", "Ctrl+W").clicked() {
                    command = Some(Command::Guard(Pending::Close));
                }
                if menu::item(ui, "E&xit", "Alt+F4").clicked() {
                    command = Some(Command::Exit);
                }
            });

            menu::top(ui, "&Edit", |ui| {
                // The label carries what would be undone, which is the whole
                // value of an undo menu over an undo button: "Undo Sort" and
                // "Undo Paste" are different offers.
                ui.add_enabled_ui(undo.is_some(), |ui| {
                    let label = match &undo {
                        Some(what) => format!("&Undo {what}"),
                        None => "&Undo".to_string(),
                    };
                    if menu::item(ui, &label, "Ctrl+Z").clicked() {
                        command = Some(Command::Do(Action::Undo));
                    }
                });
                ui.add_enabled_ui(redo.is_some(), |ui| {
                    let label = match &redo {
                        Some(what) => format!("&Redo {what}"),
                        None => "&Redo".to_string(),
                    };
                    if menu::item(ui, &label, "Ctrl+Y").clicked() {
                        command = Some(Command::Do(Action::Redo));
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "Cu&t", "Ctrl+X").clicked() {
                    command = Some(Command::Do(Action::Copy { cut: true }));
                }
                if menu::item(ui, "&Copy", "Ctrl+C").clicked() {
                    command = Some(Command::Do(Action::Copy { cut: false }));
                }
                if menu::item(ui, "&Paste", "Ctrl+V").clicked() {
                    command = Some(Command::Paste);
                }
                if menu::item(ui, "Paste &Special…", "Ctrl+Alt+V").clicked() {
                    command = Some(Command::PasteSpecial);
                }
                menu::sep(ui);
                if menu::item(ui, "Clear Co&ntents", "Delete").clicked() {
                    command = Some(Command::Do(Action::Clear));
                }
                menu::sep(ui);
                if menu::item(ui, "&Find…", "Ctrl+F").clicked() {
                    command = Some(Command::Find { replacing: false });
                }
                if menu::item(ui, "R&eplace…", "Ctrl+H").clicked() {
                    command = Some(Command::Find { replacing: true });
                }
                if menu::item(ui, "&Go To…", "Ctrl+G").clicked() {
                    command = Some(Command::GoTo);
                }
            });

            menu::top(ui, "&View", |ui| {
                if menu::check(ui, "&Freeze Panes", "", frozen).clicked() {
                    command = Some(Command::Do(Action::Freeze(!frozen)));
                }
                if menu::check(ui, "S&plit", "", split).clicked() {
                    command = Some(Command::Do(Action::Split(!split)));
                }
                menu::sep(ui);
                menu::sub(ui, "&Zoom", |ui| {
                    for percent in [50.0, 75.0, 100.0, 125.0, 150.0, 200.0] {
                        let on = (zoom * 100.0 - percent).abs() < 0.5;
                        if menu::check(ui, &format!("{percent:.0}%"), "", on).clicked() {
                            command = Some(Command::Zoom(percent / 100.0));
                        }
                    }
                });
            });

            menu::top(ui, "&Insert", |ui| {
                if menu::item(ui, "&Rows", "Ctrl++").clicked() {
                    command = Some(Command::Do(Action::Insert(Axis::Rows)));
                }
                if menu::item(ui, "&Columns", "").clicked() {
                    command = Some(Command::Do(Action::Insert(Axis::Columns)));
                }
                menu::sep(ui);
                if menu::item(ui, "Delete Ro&ws", "Ctrl+-").clicked() {
                    command = Some(Command::Do(Action::Delete(Axis::Rows)));
                }
                if menu::item(ui, "Delete Colu&mns", "").clicked() {
                    command = Some(Command::Do(Action::Delete(Axis::Columns)));
                }
                menu::sep(ui);
                menu::sub(ui, "C&hart", |ui| {
                    for (label, key, kind) in [
                        ("C&olumn", "Alt+F1", ss_model::ChartKind::Bar),
                        ("&Line", "", ss_model::ChartKind::Line),
                        ("Pi&e", "", ss_model::ChartKind::Pie),
                        ("&Area", "", ss_model::ChartKind::Area),
                    ] {
                        if menu::item(ui, label, key).clicked() {
                            command = Some(Command::Chart(kind));
                        }
                    }
                });
                if menu::item(ui, "&Picture…", "").clicked() {
                    command = Some(Command::Picture);
                }
                menu::sep(ui);
                if menu::item(ui, "&Sum Above", "Alt+=").clicked() {
                    command = Some(Command::Autosum);
                }
                let note = if noted { "Edit &Note" } else { "&Note" };
                if menu::item(ui, note, "Shift+F2").clicked() {
                    command = Some(Command::Note);
                }
            });

            menu::top(ui, "F&ormat", |ui| {
                if menu::item(ui, "&Cells…", "Ctrl+1").clicked() {
                    command = Some(Command::FormatCells(FormatTab::default()));
                }
                menu::sep(ui);
                menu::sub(ui, "&Row", |ui| {
                    if menu::item(ui, "Heigh&t…", "").clicked() {
                        command = Some(Command::Size(Axis::Rows));
                    }
                    if menu::item(ui, "F&it to Contents", "").clicked() {
                        command = Some(Command::Do(Action::AutoFit(Axis::Rows)));
                    }
                    menu::sep(ui);
                    if menu::item(ui, "&Hide", "Ctrl+9").clicked() {
                        command = Some(Command::Do(Action::Visibility {
                            axis: Axis::Rows,
                            hide: true,
                        }));
                    }
                    if menu::item(ui, "&Unhide", "Ctrl+Shift+9").clicked() {
                        command = Some(Command::Do(Action::Visibility {
                            axis: Axis::Rows,
                            hide: false,
                        }));
                    }
                });
                menu::sub(ui, "Colum&n", |ui| {
                    if menu::item(ui, "&Width…", "").clicked() {
                        command = Some(Command::Size(Axis::Columns));
                    }
                    if menu::item(ui, "F&it to Contents", "").clicked() {
                        command = Some(Command::Do(Action::AutoFit(Axis::Columns)));
                    }
                    menu::sep(ui);
                    if menu::item(ui, "&Hide", "Ctrl+0").clicked() {
                        command = Some(Command::Do(Action::Visibility {
                            axis: Axis::Columns,
                            hide: true,
                        }));
                    }
                    if menu::item(ui, "&Unhide", "Ctrl+Shift+0").clicked() {
                        command = Some(Command::Do(Action::Visibility {
                            axis: Axis::Columns,
                            hide: false,
                        }));
                    }
                });
                menu::sep(ui);
                if menu::check(ui, "&Merge Cells", "", merged).clicked() {
                    command = Some(Command::Do(Action::Merge(!merged)));
                }
                if menu::item(ui, "Con&ditional Formatting…", "").clicked() {
                    command = Some(Command::CondFormat);
                }
                menu::sep(ui);
                if menu::item(ui, "Clear &Formatting", "").clicked() {
                    command = Some(Command::Do(Action::Format(Format::Clear)));
                }
            });

            menu::top(ui, "&Data", |ui| {
                if menu::item(ui, "Sort &Ascending", "").clicked() {
                    command = Some(Command::Sort(false));
                }
                if menu::item(ui, "Sort &Descending", "").clicked() {
                    command = Some(Command::Sort(true));
                }
                if menu::item(ui, "&Sort…", "").clicked() {
                    command = Some(Command::Filter(FilterCommand::SortDialog));
                }
                menu::sep(ui);
                if menu::check(ui, "&Filter", "Ctrl+Shift+L", has_filter).clicked() {
                    command = Some(Command::Filter(FilterCommand::Toggle));
                }
                ui.add_enabled_ui(constrained, |ui| {
                    if menu::item(ui, "&Clear Filter", "").clicked() {
                        command = Some(Command::Filter(FilterCommand::Clear));
                    }
                });
                ui.add_enabled_ui(has_filter, |ui| {
                    if menu::item(ui, "&Reapply Filter", "").clicked() {
                        command = Some(Command::Filter(FilterCommand::Reapply));
                    }
                });
                menu::sep(ui);
                if menu::item(ui, "&Text to Columns…", "").clicked() {
                    command = Some(Command::Data(DataTool::TextToColumns));
                }
                if menu::item(ui, "Remove D&uplicates…", "").clicked() {
                    command = Some(Command::Data(DataTool::RemoveDuplicates));
                }
            });

            menu::top(ui, "&Tools", |ui| {
                if menu::check(ui, "&Protect Sheet", "", protected).clicked() {
                    command = Some(Command::Protect);
                }
                menu::sep(ui);
                if menu::item(ui, "&Data Validation…", "").clicked() {
                    command = Some(Command::Validation);
                }
                if menu::item(ui, "Define &Names…", "Ctrl+F3").clicked() {
                    command = Some(Command::Names);
                }
            });
        });

        command
    }

    /// Performs one gathered [`Command`].
    ///
    /// One place, so that the toolbar and the menus cannot answer the same
    /// command differently — and so that adding a command anywhere on the
    /// surface is adding it in exactly two places, not five.
    fn run(&mut self, ui: &egui::Ui, command: Command) {
        match command {
            Command::Do(action) => self.act(ui, action),
            Command::Guard(what) => self.guard(what),
            Command::Save => self.save(),
            Command::SaveAs => self.save_as(),
            Command::Exit => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
            Command::Reopen(path) => self.open_recent(path),
            Command::ForgetRecent => {
                self.recent.clear(CALX);
                self.status = "Recent file list cleared".to_string();
            }
            Command::Sort(descending) => self.sort_quick(descending),
            Command::Filter(FilterCommand::SortDialog) => self.open_sort_dialog(),
            Command::Filter(FilterCommand::Toggle) => self.toggle_filter(),
            Command::Filter(FilterCommand::Clear) => self.clear_filter(),
            Command::Filter(FilterCommand::Reapply) => self.reapply_filter(),
            Command::Size(axis) => self.open_size_dialog(axis),
            Command::Find { replacing } => self.open_find(replacing),
            Command::GoTo => {
                self.dialog = Some(Dialog::GoTo {
                    text: String::new(),
                })
            }
            Command::Names => self.open_names(),
            Command::FormatCells(tab) => {
                self.dialog = Some(Dialog::FormatCells {
                    look: Box::new(self.cursor_look()),
                    tab,
                })
            }
            Command::Validation => self.open_validation(),
            Command::CondFormat => self.open_cond_format(),
            Command::Protect => self.toggle_protection(),
            Command::Data(DataTool::TextToColumns) => self.open_text_to_columns(),
            Command::Data(DataTool::RemoveDuplicates) => self.open_remove_duplicates(),
            Command::Chart(kind) => self.insert_chart(kind),
            Command::Picture => self.insert_picture(),
            Command::Note => self.open_note(),
            Command::Paste => {
                let text = self.os_clipboard_text();
                self.act(ui, Action::Paste(text));
            }
            Command::PasteSpecial => {
                self.dialog = Some(Dialog::PasteSpecial {
                    how: Default::default(),
                })
            }
            Command::Autosum => self.autosum(),
            Command::Zoom(factor) => self.grid.set_zoom(factor),
        }
    }

    /// The command surface: menus, the quick-access icons, the formatting of
    /// the selection, and what the cursor is sitting on.
    ///
    /// Four rows rather than one because they answer different questions and
    /// change at different rates. The menus are the same all day; the icons are
    /// the dozen commands used by the minute; the formatting row changes with
    /// every click; the formula bar is the cell. A single row of forty controls
    /// makes the user re-scan all forty to find the one that moved.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let mut command = self.menus(ui);
        rule(ui);
        ui.add_space(2.0);

        // Read before the row is laid out, for the same reason the menus read
        // their state up front.
        let cursor = self.grid.selection.cursor();
        let sheet = self.doc.workbook.sheet(self.grid.sheet_index);
        let merged = sheet.is_some_and(|s| s.merge_at(cursor).is_some());
        let panes = sheet.and_then(|s| s.panes);
        let frozen = panes.is_some_and(|p| p.frozen);
        let has_filter = sheet.is_some_and(|s| s.filter.is_some());
        let undo = self.undo.last().map(|change| change.label.clone());
        let redo = self.redo.last().map(|change| change.label.clone());
        let can_save = self.edited || self.path.is_none();

        // The quick-access row. Icons only: a toolbar that mixes drawn glyphs
        // with words reads as two toolbars that collided, and every word that
        // used to be here is a command already named — more legibly, with its
        // keystroke — one row up.
        ui.horizontal(|ui| {
            for (icon, tip, what) in [
                (
                    Icon::New,
                    "New workbook (Ctrl+N)",
                    Command::Guard(Pending::New),
                ),
                (Icon::Open, "Open (Ctrl+O)", Command::Guard(Pending::Browse)),
            ] {
                if icons::button(ui, icon, false, tip).clicked() {
                    command = Some(what);
                }
            }
            ui.add_enabled_ui(can_save, |ui| {
                if icons::button(ui, Icon::Save, false, "Save (Ctrl+S)")
                    .on_disabled_hover_text("Nothing has changed since the last save")
                    .clicked()
                {
                    command = Some(Command::Save);
                }
            });
            separate(ui);

            ui.add_enabled_ui(undo.is_some(), |ui| {
                let tip = match &undo {
                    Some(what) => format!("Undo {what} (Ctrl+Z)"),
                    None => "Undo (Ctrl+Z)".to_string(),
                };
                if icons::button(ui, Icon::Undo, false, &tip).clicked() {
                    command = Some(Command::Do(Action::Undo));
                }
            });
            ui.add_enabled_ui(redo.is_some(), |ui| {
                let tip = match &redo {
                    Some(what) => format!("Redo {what} (Ctrl+Y)"),
                    None => "Redo (Ctrl+Y)".to_string(),
                };
                if icons::button(ui, Icon::Redo, false, &tip).clicked() {
                    command = Some(Command::Do(Action::Redo));
                }
            });
            separate(ui);

            for (icon, tip, action) in [
                (
                    Icon::InsertRow,
                    "Insert rows (Ctrl++)",
                    Action::Insert(Axis::Rows),
                ),
                (
                    Icon::DeleteRow,
                    "Delete rows (Ctrl+-)",
                    Action::Delete(Axis::Rows),
                ),
                (
                    Icon::InsertColumn,
                    "Insert columns",
                    Action::Insert(Axis::Columns),
                ),
                (
                    Icon::DeleteColumn,
                    "Delete columns",
                    Action::Delete(Axis::Columns),
                ),
            ] {
                if icons::button(ui, icon, false, tip).clicked() {
                    command = Some(Command::Do(action));
                }
            }
            separate(ui);

            if icons::button(ui, Icon::Merge, merged, "Merge cells").clicked() {
                command = Some(Command::Do(Action::Merge(!merged)));
            }
            let tip = if frozen {
                "Unfreeze panes"
            } else {
                "Freeze rows above and columns left of the cursor"
            };
            if icons::button(ui, Icon::Freeze, frozen, tip).clicked() {
                command = Some(Command::Do(Action::Freeze(!frozen)));
            }
            if icons::button(ui, Icon::Sum, false, "Sum the cells above (Alt+=)").clicked() {
                command = Some(Command::Autosum);
            }
            separate(ui);

            for (icon, tip, descending) in [
                (Icon::SortAscending, "Sort A to Z (smallest first)", false),
                (Icon::SortDescending, "Sort Z to A (largest first)", true),
            ] {
                if icons::button(ui, icon, false, tip).clicked() {
                    command = Some(Command::Sort(descending));
                }
            }
            let tip = if has_filter {
                "Remove the filter arrows (Ctrl+Shift+L)"
            } else {
                "Filter — arrows on the heading row (Ctrl+Shift+L)"
            };
            if icons::button(ui, Icon::Filter, has_filter, tip).clicked() {
                command = Some(Command::Filter(FilterCommand::Toggle));
            }
        });

        // The formatting row: what the cursor's cell looks like, and every way
        // to change it that is worth a permanent place on the screen.
        ui.horizontal(|ui| {
            let look = self.cursor_look();
            let theme = self.doc.workbook.styles.theme();

            let name = look.font.name.clone();
            field(ui, |ui| {
                egui::ComboBox::from_id_salt("calx-font")
                    .selected_text(&name)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        // The workbook's own font first, so a document set in a
                        // face nobody offers can still be got back to.
                        let mut offered: Vec<&str> = FONT_NAMES.to_vec();
                        if !offered.contains(&name.as_str()) {
                            offered.insert(0, name.as_str());
                        }
                        for choice in offered {
                            if ui.selectable_label(choice == name, choice).clicked() {
                                command = Some(Command::Do(Action::Format(Format::FontName(
                                    choice.to_string(),
                                ))));
                            }
                        }
                    });
            });

            let mut size = look.font.size;
            let changed = field(ui, |ui| {
                ui.add(
                    egui::DragValue::new(&mut size)
                        .speed(0.5)
                        .range(1.0..=409.0)
                        .fixed_decimals(0),
                )
                .on_hover_text("Font size — drag, or click to type")
                .changed()
            });
            if changed {
                command = Some(Command::Do(Action::Format(Format::FontSize(size))));
            }
            separate(ui);

            for (letter, apply) in [
                (bold_letter("B", look.font.bold), Format::Bold),
                (italic_letter("I", look.font.italic), Format::Italic),
                (
                    icons::Letter {
                        text: "U",
                        underline: true,
                        on: !look.font.underline.is_none(),
                        tip: "Underline (Ctrl+U)",
                        ..icons::Letter::plain()
                    },
                    Format::Underline,
                ),
                (
                    icons::Letter {
                        text: "S",
                        strike: true,
                        on: look.font.strike,
                        tip: "Strikethrough (Ctrl+5)",
                        ..icons::Letter::plain()
                    },
                    Format::Strike,
                ),
            ] {
                if icons::letter(ui, letter).clicked() {
                    command = Some(Command::Do(Action::Format(apply)));
                }
            }
            separate(ui);

            // Excel's split colour controls: the glyph applies the colour shown
            // in the band beneath it, and the chevron is where a different one
            // is chosen. Two halves rather than one because applying the same
            // colour to a dozen ranges is the common case, and it should not
            // cost a trip through a palette every time.
            let text_rgb = look.font.color.resolve(theme).unwrap_or([0, 0, 0]);
            if icons::color_button(ui, Icon::TextColor, text_rgb, "Apply this text colour")
                .clicked()
            {
                let [r, g, b] = text_rgb;
                command = Some(Command::Do(Action::Format(Format::TextColor(Some(
                    Color::rgb(r, g, b),
                )))));
            }
            let arrow = icons::arrow(ui, "Choose a text colour");
            if let Some(chosen) = menu::under(&arrow, |ui| palette(ui, "Automatic")).flatten() {
                let color = chosen.map(|[r, g, b]| Color::rgb(r, g, b));
                command = Some(Command::Do(Action::Format(Format::TextColor(color))));
            }

            let fill_rgb = look.fill.shade(theme).unwrap_or([255, 255, 255]);
            if icons::color_button(ui, Icon::FillColor, fill_rgb, "Apply this fill colour")
                .clicked()
            {
                let [r, g, b] = fill_rgb;
                command = Some(Command::Do(Action::Format(Format::Fill(Some(Color::rgb(
                    r, g, b,
                ))))));
            }
            let arrow = icons::arrow(ui, "Choose a fill colour");
            if let Some(chosen) = menu::under(&arrow, |ui| palette(ui, "No Fill")).flatten() {
                let color = chosen.map(|[r, g, b]| Color::rgb(r, g, b));
                command = Some(Command::Do(Action::Format(Format::Fill(color))));
            }
            separate(ui);

            for (icon, align, tip) in [
                (Icon::AlignLeft, HAlign::Left, "Align left"),
                (Icon::AlignCenter, HAlign::Center, "Centre"),
                (Icon::AlignRight, HAlign::Right, "Align right"),
            ] {
                let on = look.alignment.horizontal == align;
                if icons::button(ui, icon, on, tip).clicked() {
                    command = Some(Command::Do(Action::Format(Format::Align(align))));
                }
            }
            if icons::button(ui, Icon::Wrap, look.alignment.wrap, "Wrap text").clicked() {
                command = Some(Command::Do(Action::Format(Format::Wrap)));
            }
            for (icon, by, tip) in [
                (Icon::IndentLess, -1, "Decrease indent"),
                (Icon::IndentMore, 1, "Increase indent"),
            ] {
                if icons::button(ui, icon, false, tip).clicked() {
                    command = Some(Command::Do(Action::Format(Format::Indent(by))));
                }
            }
            separate(ui);

            let borders = icons::menu_button(ui, Icon::Borders, "Borders");
            if let Some(Some(preset)) = menu::under(&borders, |ui| {
                let mut chosen = None;
                for (label, key, preset) in [
                    ("&All Borders", "", BorderPreset::All),
                    ("Ou&tline", "Ctrl+Shift+&", BorderPreset::Outline),
                    ("T&hick Outline", "", BorderPreset::Thick),
                    ("&Bottom", "", BorderPreset::Bottom),
                    ("To&p", "", BorderPreset::Top),
                    ("&Left", "", BorderPreset::Left),
                    ("&Right", "", BorderPreset::Right),
                ] {
                    if menu::item(ui, label, key).clicked() {
                        chosen = Some(preset);
                    }
                }
                menu::sep(ui);
                if menu::item(ui, "&No Border", "Ctrl+Shift+_").clicked() {
                    chosen = Some(BorderPreset::None);
                }
                chosen
            }) {
                command = Some(Command::Do(Action::Format(Format::Border(preset))));
            }
            separate(ui);

            let number = look.number_format.clone();
            field(ui, |ui| {
                egui::ComboBox::from_id_salt("calx-number-format")
                    .selected_text(short_format_name(&number))
                    .width(126.0)
                    .show_ui(ui, |ui| {
                        for (label, code) in NUMBER_FORMATS {
                            if ui.selectable_label(number == *code, *label).clicked() {
                                command = Some(Command::Do(Action::Format(Format::NumberFormat(
                                    code.to_string(),
                                ))));
                            }
                        }
                    });
            });
            for (label, tip, code) in [
                ("$", "Currency", "\"$\"#,##0.00"),
                ("%", "Percent", "0.00%"),
                (",", "Thousands separator", "#,##0.00"),
            ] {
                if icons::letter(
                    ui,
                    icons::Letter {
                        text: label,
                        tip,
                        ..icons::Letter::plain()
                    },
                )
                .clicked()
                {
                    command = Some(Command::Do(Action::Format(Format::NumberFormat(
                        code.to_string(),
                    ))));
                }
            }
            separate(ui);

            // Excel's dialog launcher: the whole of Format Cells, for the parts
            // of it a toolbar has no room to hold.
            if word_button(ui, "More…")
                .on_hover_text("Format Cells (Ctrl+1)")
                .clicked()
            {
                command = Some(Command::FormatCells(FormatTab::default()));
            }
        });

        self.formula_bar(ui);
        self.chart_bar(ui);

        if let Some(command) = command {
            let ui = &*ui;
            self.run(ui, command);
        }
    }

    /// The title box, shown only while a chart is selected.
    fn chart_bar(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.grid.selected_chart else {
            self.chart_title = None;
            return;
        };
        let sheet = self.grid.sheet_index;
        let Some(existing) = self
            .doc
            .workbook
            .sheet(sheet)
            .and_then(|s| s.charts.get(index))
            .map(|c| c.title.clone().unwrap_or_default())
        else {
            return;
        };
        let mut change = None;
        ui.horizontal(|ui| {
            ui.label("Chart title");
            if self.chart_title.is_none() {
                self.chart_title = Some(existing.clone());
            }
            let buffer = self.chart_title.get_or_insert_with(String::new);
            let response = ui.add(
                egui::TextEdit::singleline(buffer)
                    .desired_width(260.0)
                    .hint_text("(none)"),
            );
            // On losing focus rather than on every keystroke: one undo entry
            // per title, not one per letter.
            if response.lost_focus() && *buffer != existing {
                change = Some(buffer.clone());
            }
        });
        if let Some(title) = change {
            let change = edit::chart_title(sheet, index, &title);
            self.perform(change);
        }
    }

    /// The name box and the formula bar.
    ///
    /// The name box is an entry, not a label: typing `H400` into it and pressing
    /// Enter is how anybody reaches row four hundred of a wide sheet, and a
    /// spreadsheet without it is one that can only be navigated by scrolling.
    fn formula_bar(&mut self, ui: &mut egui::Ui) {
        let cursor = self.grid.selection.cursor();
        let mut open_editor = false;
        let mut commit = false;
        let mut cancel = false;
        let mut go_to = None;

        ui.horizontal(|ui| {
            // Reset to the cursor whenever the box is not being typed into, so
            // it tracks the selection rather than holding a stale address.
            let id = egui::Id::new("calx-name-box");
            let focused = ui.memory(|m| m.has_focus(id));
            if !focused {
                self.name_box = cursor.to_a1();
            }
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.name_box)
                    .id(id)
                    .desired_width(88.0)
                    .horizontal_align(egui::Align::Center),
            );
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go_to = Some(self.name_box.clone());
            }
            ui.separator();

            let editing = self.grid.editor.is_some();
            // The tick and cross that appear only while an edit is open, which
            // is the one part of Excel's formula bar that is not a keystroke.
            ui.add_enabled_ui(editing, |ui| {
                if ui.small_button("✔").on_hover_text("Enter").clicked() {
                    commit = true;
                }
                if ui.small_button("✖").on_hover_text("Cancel (Esc)").clicked() {
                    cancel = true;
                }
            });
            ui.label(egui::RichText::new("fx").italics().weak());
            ui.separator();

            let width = ui.available_width();
            match &mut self.grid.editor {
                Some(open) => {
                    let font = egui::FontId::monospace(13.0);
                    let plain = ui.visuals().text_color();
                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                        let mut job = grid::editor::highlight(text.as_str(), font.clone(), plain);
                        job.wrap.max_width = wrap;
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    let response = ui.add_sized(
                        [width, 22.0],
                        egui::TextEdit::singleline(&mut open.text)
                            .id(egui::Id::new("calx-formula-bar"))
                            .layouter(&mut layouter),
                    );
                    // Typing here rather than in the cell is still an edit, and
                    // the arrow keys have to stop committing once it starts.
                    if response.changed() {
                        open.mode = Mode::Edit;
                    }
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                }
                None => {
                    let source =
                        grid::source_text(&self.doc.workbook, self.grid.sheet_index, cursor);
                    // Laid out left to right rather than sized: `add_sized`
                    // *centres* what it is given, so a short formula in a wide
                    // bar drifted into the middle of the window, nowhere near
                    // the caret that appears the moment it is clicked.
                    let response = ui
                        .allocate_ui_with_layout(
                            egui::vec2(width, 22.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(source).monospace())
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                )
                            },
                        )
                        .inner;
                    if response.clicked() {
                        open_editor = true;
                    }
                }
            }
        });

        if open_editor {
            let text = grid::source_text(&self.doc.workbook, self.grid.sheet_index, cursor);
            self.grid.editor = Some(Editor::editing(cursor, text));
        }
        if commit {
            self.grid.commit(Some(grid::Direction::Down));
        }
        if cancel {
            self.grid.editor = None;
        }
        if let Some(target) = go_to {
            self.go_to(&target);
        }
    }

    /// Jumps to an address, a range, or a defined name.
    fn go_to(&mut self, target: &str) {
        let text = target.trim();
        if text.is_empty() {
            return;
        }
        // A defined name first: `Sales` is a name before it is an attempt at a
        // cell reference, and Excel resolves it the same way round.
        let resolved = self
            .doc
            .workbook
            .defined_names
            .iter()
            .find(|n| n.name.eq_ignore_ascii_case(text))
            .map(|n| n.refers_to.clone());
        let text = resolved.as_deref().unwrap_or(text);
        let cleaned = text.rsplit('!').next().unwrap_or(text).replace('$', "");

        let (from, to) = match cleaned.split_once(':') {
            Some((a, b)) => (CellRef::from_a1(a), CellRef::from_a1(b)),
            None => (CellRef::from_a1(&cleaned), None),
        };
        let Some(from) = from else {
            self.status = format!("{target} is not a cell or a name");
            return;
        };
        self.grid.selection = grid::Selection::at(from);
        if let Some(sheet) = self.doc.workbook.sheet(self.grid.sheet_index) {
            if let Some(to) = to {
                self.grid.selection.extend_to(to, sheet);
            }
            self.grid
                .scroll_into_view(from, self.last_body, &self.doc.workbook, sheet);
        }
        self.status = format!("Went to {}", from.to_a1());
    }

    /// Puts a SUM over the run of numbers above the cursor.
    fn autosum(&mut self) {
        let sheet_index = self.grid.sheet_index;
        let at = self.grid.selection.cursor();
        let Some(sheet) = self.doc.workbook.sheet(sheet_index) else {
            return;
        };
        // Upwards from the cell above, for as long as there are numbers. An
        // empty cell ends the run, which is what makes this land on the block
        // under a heading rather than on the whole column.
        let mut top = at.row;
        while top > 0 {
            let above = CellRef::new(top - 1, at.col);
            let numeric = sheet
                .get(above)
                .is_some_and(|c| matches!(c.value, ss_model::CellValue::Number(_)));
            if !numeric {
                break;
            }
            top -= 1;
        }
        if top == at.row {
            self.status = "Nothing to sum above this cell".to_string();
            return;
        }
        let formula = format!(
            "=SUM({}:{})",
            CellRef::new(top, at.col).to_a1(),
            CellRef::new(at.row - 1, at.col).to_a1()
        );
        let change = edit::input(&mut self.doc.workbook, sheet_index, at, &formula);
        self.perform(change);
    }

    /// The sheet tabs and the status line.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let mut switch = None;
        let mut context: Option<(usize, TabCommand)> = None;
        let hidden_count = self
            .doc
            .workbook
            .sheets
            .iter()
            .filter(|s| s.hidden && s.kind.has_grid())
            .count();

        let mut tab_rects: Vec<(usize, egui::Rect)> = Vec::new();
        egui::ScrollArea::horizontal()
            .id_salt("calx-tabs")
            .max_height(26.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    let theme = self.doc.workbook.styles.theme();
                    for index in 0..self.doc.workbook.sheets.len() {
                        let sheet = &self.doc.workbook.sheets[index];
                        if !sheet.kind.has_grid() || sheet.hidden {
                            continue;
                        }
                        let selected = index == self.grid.sheet_index;
                        let stripe = sheet.view.tab_color.and_then(|c| c.resolve(theme));
                        let response = tab(ui, &sheet.name, selected, stripe);
                        tab_rects.push((index, response.rect));
                        if response.clicked() && !selected {
                            switch = Some(index);
                        }
                        // Dragging a tab sideways picks it up; the drop is
                        // resolved below, once every tab's place is known.
                        if response.drag_started() {
                            self.dragging_tab = Some(index);
                        }
                        response.context_menu(|ui| {
                            let mut chose = |command| context = Some((index, command));
                            if ui.button("Insert…").clicked() {
                                chose(TabCommand::Insert);
                                ui.close();
                            }
                            if ui.button("Delete").clicked() {
                                chose(TabCommand::Delete);
                                ui.close();
                            }
                            if ui.button("Rename…").clicked() {
                                chose(TabCommand::Rename);
                                ui.close();
                            }
                            if ui.button("Move or Copy…").clicked() {
                                chose(TabCommand::MoveOrCopy);
                                ui.close();
                            }
                            ui.separator();
                            ui.menu_button("Tab Colour", |ui| {
                                // Excel's own standard row, plus the way back.
                                for (label, rgb) in TAB_COLORS {
                                    let [r, g, b] = *rgb;
                                    if color_swatch(ui, label, [r, g, b]).clicked() {
                                        chose(TabCommand::Color(Some(Color::rgb(r, g, b))));
                                        ui.close();
                                    }
                                }
                                if ui.button("No colour").clicked() {
                                    chose(TabCommand::Color(None));
                                    ui.close();
                                }
                            });
                            ui.separator();
                            if ui.button("Hide").clicked() {
                                chose(TabCommand::Hide);
                                ui.close();
                            }
                            ui.add_enabled_ui(hidden_count > 0, |ui| {
                                if ui
                                    .button("Unhide…")
                                    .on_disabled_hover_text("No sheets are hidden")
                                    .clicked()
                                {
                                    chose(TabCommand::UnhideAll);
                                    ui.close();
                                }
                            });
                            ui.separator();
                            if ui.button("Select All Sheets").clicked() {
                                chose(TabCommand::SelectAll);
                                ui.close();
                            }
                        });
                    }

                    // The `+` at the end of the strip, which is how a sheet
                    // gets added without anyone having to find a menu.
                    ui.add_space(4.0);
                    if ui
                        .add(egui::Button::new("+").min_size(egui::vec2(24.0, 22.0)))
                        .on_hover_text("New sheet")
                        .clicked()
                    {
                        context = Some((self.grid.sheet_index, TabCommand::Insert));
                    }
                    if hidden_count > 0 {
                        ui.add_space(6.0);
                        ui.weak(format!("({hidden_count} hidden)"));
                    }
                });
            });

        // A tab in flight: a closed hand for the pointer, a drop line where
        // it would land, and the reorder itself when the button comes up.
        if let Some(dragged) = self.dragging_tab {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let pos = ui.ctx().pointer_latest_pos();
            // Past the middle of a tab lands after it; the slot is named as
            // "before sheet N", the same language Move or Copy speaks.
            let before = pos.map(|pos| {
                tab_rects
                    .iter()
                    .find(|(_, rect)| pos.x < rect.center().x)
                    .map_or(self.doc.workbook.sheets.len(), |(index, _)| *index)
            });
            if let (Some(before), Some(pos)) = (before, pos) {
                let x = tab_rects
                    .iter()
                    .find(|(index, _)| *index == before)
                    .map_or_else(
                        || tab_rects.last().map_or(pos.x, |(_, r)| r.right()),
                        |(_, r)| r.left(),
                    );
                if let Some((_, sample)) = tab_rects.first() {
                    ui.painter().vline(
                        x,
                        sample.y_range(),
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(0x21, 0x73, 0x46)),
                    );
                }
            }
            if ui.ctx().input(|i| !i.pointer.any_down()) {
                self.dragging_tab = None;
                if let Some(before) = before {
                    if before != dragged && before != dragged + 1 {
                        self.move_sheet(dragged, before, false);
                    }
                }
            }
        }

        ui.horizontal(|ui| {
            // The mode cell, leftmost as Excel puts it: Ready, Enter while
            // typing, Edit under F2, Point while a reference is being aimed.
            let mode = match &self.grid.editor {
                Some(open) if open.is_formula() && open.can_point() => "Point",
                Some(open) if open.mode == grid::editor::Mode::Enter => "Enter",
                Some(_) => "Edit",
                None => "Ready",
            };
            ui.small(mode);
            ui.separator();
            ui.small(&self.status);
            if self.edited {
                ui.small("• edited");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The zoom control, which belongs here rather than in the
                // toolbar because it is about the view and not the document.
                // Excel's corner, right to left: the percentage (click it to
                // type an exact one), zoom in, the slider with its 100%
                // detent, zoom out. The buttons step to the next round ten.
                let shown = (self.grid.zoom * 100.0).round();
                let mut percent = shown;
                ui.spacing_mut().item_spacing.x = 4.0;
                let label = ui
                    .add(
                        egui::Button::new(format!("{}%", percent as i32))
                            .frame(false)
                            .min_size(egui::vec2(40.0, 0.0)),
                    )
                    .on_hover_text("Zoom level — click to set an exact zoom");
                if label.clicked() {
                    self.dialog = Some(Dialog::Zoom {
                        text: (percent as i32).to_string(),
                        fresh: true,
                    });
                }
                if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                    percent = (percent / 10.0).floor() * 10.0 + 10.0;
                }
                zoom_slider(ui, &mut percent);
                if ui.small_button("−").on_hover_text("Zoom out").clicked() {
                    percent = (percent / 10.0).ceil() * 10.0 - 10.0;
                }
                let percent = percent.clamp(10.0, 400.0);
                if percent != shown {
                    self.grid.set_zoom(percent / 100.0);
                }
                if let Err(e) = &self.config_dir {
                    ui.colored_label(egui::Color32::RED, format!("config unavailable: {e}"));
                }
                ui.separator();
                for text in summary_labels(&self.grid.summary) {
                    ui.small(text);
                    ui.add_space(6.0);
                }
            });
        });

        if let Some(index) = switch {
            self.show_sheet(index);
        }
        if let Some((index, command)) = context {
            self.tab_command(index, command);
        }
    }

    fn tab_command(&mut self, index: usize, command: TabCommand) {
        match command {
            TabCommand::Insert => self.add_sheet(),
            TabCommand::Delete => self.delete_sheet(index),
            TabCommand::Rename => {
                self.dialog = Some(Dialog::RenameSheet {
                    index,
                    text: self.doc.workbook.sheets[index].name.clone(),
                });
            }
            TabCommand::MoveOrCopy => {
                self.dialog = Some(Dialog::MoveSheet {
                    index,
                    before: index,
                    copy: false,
                });
            }
            TabCommand::Color(color) => self.set_tab_color(index, color),
            TabCommand::Hide => {
                if self.visible_sheets() > 1 {
                    self.perform(ss_formula::sheets::set_hidden(index, true));
                    if self.grid.sheet_index == index {
                        self.step_sheet(1);
                        if self.grid.sheet_index == index {
                            self.step_sheet(-1);
                        }
                    }
                } else {
                    self.status = "A workbook needs one visible sheet".to_string();
                }
            }
            TabCommand::UnhideAll => {
                // One change, not one per sheet, so a single Ctrl+Z puts them
                // all back the way they were.
                let patches: Vec<Patch> = self
                    .doc
                    .workbook
                    .sheets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.hidden)
                    .map(|(index, _)| Patch::SheetHidden {
                        index,
                        hidden: false,
                    })
                    .collect();
                if patches.is_empty() {
                    return;
                }
                self.perform(Change::new("Unhide sheets", patches));
            }
            TabCommand::SelectAll => {
                // Excel's "Select All Sheets" groups them so that typing lands
                // on every one at once. That is a mode with real consequences
                // — a stray keystroke edits fifteen sheets — and nothing else
                // in Calx has the concept yet, so this reports rather than
                // pretends.
                self.status = format!(
                    "{} sheets — grouped editing is not implemented, so this selects nothing",
                    self.visible_sheets()
                );
            }
        }
    }

    fn visible_sheets(&self) -> usize {
        self.doc
            .workbook
            .sheets
            .iter()
            .filter(|s| s.kind.has_grid() && !s.hidden)
            .count()
    }

    /// Whichever modal is open, drawn and answered.
    ///
    /// The dialog is taken out of `self` for the duration so that its own state
    /// can be edited while the application is borrowed to act on it, and put
    /// back unless something closed it.
    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        match &mut dialog {
            Dialog::RenameSheet { index, text } => {
                let index = *index;
                let mut accept = false;
                modal(ctx, "Rename sheet", |ui| {
                    let field = ui.text_edit_singleline(text);
                    field.request_focus();
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        accept = true;
                    }
                    ui.add_space(4.0);
                    // Told *before* pressing OK rather than after: the refusal
                    // is about the name being typed, and finding out on submit
                    // means retyping it.
                    if let Some(why) = self
                        .doc
                        .workbook
                        .sheet_name_refusal(text.trim(), Some(index))
                    {
                        ui.colored_label(egui::Color32::from_rgb(0xB0, 0x30, 0x20), why);
                    }
                    match dialog::confirm(ui, "Rename") {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if accept {
                    let name = text.clone();
                    self.rename_sheet(index, &name);
                    keep = false;
                }
            }

            Dialog::GoTo { text } => {
                let mut accept = false;
                modal(ctx, "Go to", |ui| {
                    let field = ui.text_edit_singleline(text);
                    if text.is_empty() {
                        field.request_focus();
                    }
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        accept = true;
                    }
                    ui.add_space(2.0);
                    ui.weak("A cell, a range, or a defined name — B12, A1:D9, Sales.");
                    ui.add_space(4.0);
                    match dialog::confirm(ui, "Go") {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if accept {
                    let target = text.clone();
                    self.go_to(&target);
                    keep = false;
                }
            }

            Dialog::Validation { rule, existing } => {
                use ss_model::cond::{DvKind, DvOperator, DvSeverity};
                let existing = *existing;
                let (mut accept, mut remove) = (false, false);
                modal(ctx, "Data validation", |ui| {
                    ui.label(
                        egui::RichText::new(format!("Applies to {}", ranges_label(&rule.ranges)))
                            .weak()
                            .small(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Allow:");
                        egui::ComboBox::from_id_salt("calx-dv-kind")
                            .selected_text(dv_kind_name(rule.kind))
                            .show_ui(ui, |ui| {
                                for kind in [
                                    DvKind::List,
                                    DvKind::Whole,
                                    DvKind::Decimal,
                                    DvKind::Date,
                                    DvKind::Time,
                                    DvKind::TextLength,
                                    DvKind::Custom,
                                ] {
                                    ui.selectable_value(&mut rule.kind, kind, dv_kind_name(kind));
                                }
                            });
                        ui.checkbox(&mut rule.allow_blank, "Ignore blank");
                    });
                    match rule.kind {
                        DvKind::List => {
                            ui.horizontal(|ui| {
                                ui.label("Source:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut rule.formula1)
                                        .hint_text("\"Yes,No\" or A1:A9")
                                        .desired_width(220.0),
                                );
                            });
                            ui.checkbox(&mut rule.show_dropdown, "In-cell dropdown");
                        }
                        DvKind::Custom => {
                            ui.horizontal(|ui| {
                                ui.label("Formula:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut rule.formula1)
                                        .hint_text("=ISNUMBER(A1)")
                                        .desired_width(220.0),
                                );
                            });
                        }
                        DvKind::None => {}
                        _ => {
                            ui.horizontal(|ui| {
                                ui.label("Data:");
                                egui::ComboBox::from_id_salt("calx-dv-op")
                                    .selected_text(dv_op_name(rule.operator))
                                    .show_ui(ui, |ui| {
                                        for op in [
                                            DvOperator::Between,
                                            DvOperator::NotBetween,
                                            DvOperator::Equal,
                                            DvOperator::NotEqual,
                                            DvOperator::GreaterThan,
                                            DvOperator::LessThan,
                                            DvOperator::GreaterThanOrEqual,
                                            DvOperator::LessThanOrEqual,
                                        ] {
                                            ui.selectable_value(
                                                &mut rule.operator,
                                                op,
                                                dv_op_name(op),
                                            );
                                        }
                                    });
                            });
                            let two = matches!(
                                rule.operator,
                                DvOperator::Between | DvOperator::NotBetween
                            );
                            ui.horizontal(|ui| {
                                ui.label(if two { "Minimum:" } else { "Value:" });
                                ui.add(
                                    egui::TextEdit::singleline(&mut rule.formula1)
                                        .desired_width(120.0),
                                );
                                if two {
                                    ui.label("Maximum:");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut rule.formula2)
                                            .desired_width(120.0),
                                    );
                                }
                            });
                        }
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("On error:");
                        egui::ComboBox::from_id_salt("calx-dv-severity")
                            .selected_text(match rule.severity {
                                DvSeverity::Stop => "Stop",
                                DvSeverity::Warning => "Warning",
                                DvSeverity::Information => "Information",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut rule.severity, DvSeverity::Stop, "Stop");
                                ui.selectable_value(
                                    &mut rule.severity,
                                    DvSeverity::Warning,
                                    "Warning",
                                );
                                ui.selectable_value(
                                    &mut rule.severity,
                                    DvSeverity::Information,
                                    "Information",
                                );
                            });
                        ui.add(
                            egui::TextEdit::singleline(&mut rule.error_title)
                                .hint_text("Error title")
                                .desired_width(140.0),
                        );
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut rule.error_message)
                            .hint_text("Error message")
                            .desired_width(320.0),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut rule.prompt_title)
                                .hint_text("Prompt title")
                                .desired_width(140.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut rule.prompt_message)
                                .hint_text("Prompt message")
                                .desired_width(172.0),
                        );
                    });
                    // Right to left, which is the order `dialog::row` lays a
                    // group out in: Cancel ends up on the right.
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Cancel", false).clicked();
                        ui.add_enabled_ui(existing.is_some(), |ui| {
                            remove = dialog::button(ui, "Remove", false)
                                .on_hover_text("Delete this rule from every cell it covers")
                                .clicked();
                        });
                        accept = dialog::button(ui, "OK", true).clicked();
                    });
                });
                let sheet = self.grid.sheet_index;
                if accept {
                    if let Some(s) = self.doc.workbook.sheet(sheet) {
                        let mut list = s.validations.clone();
                        match existing {
                            Some(i) if i < list.len() => list[i] = rule.clone(),
                            _ => list.push(rule.clone()),
                        }
                        self.perform(Change::new(
                            "Data validation",
                            vec![Patch::Validations {
                                sheet,
                                validations: list,
                            }],
                        ));
                        self.grid.invalidate();
                    }
                    keep = false;
                }
                if remove {
                    if let (Some(s), Some(i)) = (self.doc.workbook.sheet(sheet), existing) {
                        let mut list = s.validations.clone();
                        if i < list.len() {
                            list.remove(i);
                        }
                        self.perform(Change::new(
                            "Remove validation",
                            vec![Patch::Validations {
                                sheet,
                                validations: list,
                            }],
                        ));
                        self.grid.invalidate();
                    }
                    keep = false;
                }
            }

            Dialog::CondFormat {
                formats,
                kind,
                operator,
                value1,
                value2,
                bold,
                italic,
                use_text_color,
                text_color,
                use_fill,
                fill_color,
            } => {
                use ss_model::cond::{CfKind, CfOperator, CfRule, CfValue, CfValueKind};
                const KINDS: [&str; 9] = [
                    "Cell value",
                    "Text contains",
                    "Duplicate values",
                    "Unique values",
                    "Top 10",
                    "Above average",
                    "Formula is true",
                    "Colour scale",
                    "Data bar",
                ];
                let mut accept = false;
                let mut add = false;
                let selection = self.grid.selection.ranges().to_vec();
                modal(ctx, "Conditional formatting", |ui| {
                    // The sheet's rules, each with its own delete.
                    let mut delete: Option<(usize, usize)> = None;
                    if formats.is_empty() {
                        ui.weak("No rules on this sheet yet.");
                    }
                    for (b, block) in formats.iter().enumerate() {
                        for (r, rule) in block.rules.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("✕").on_hover_text("Delete rule").clicked() {
                                    delete = Some((b, r));
                                }
                                ui.small(format!(
                                    "{} — {}",
                                    ranges_label(&block.ranges),
                                    cf_rule_label(rule)
                                ));
                            });
                        }
                    }
                    if let Some((b, r)) = delete {
                        formats[b].rules.remove(r);
                        if formats[b].rules.is_empty() {
                            formats.remove(b);
                        }
                    }
                    ui.separator();
                    ui.label("New rule over the selection:");
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("calx-cf-kind")
                            .selected_text(KINDS[*kind])
                            .show_ui(ui, |ui| {
                                for (i, name) in KINDS.iter().enumerate() {
                                    ui.selectable_value(kind, i, *name);
                                }
                            });
                        match *kind {
                            0 => {
                                egui::ComboBox::from_id_salt("calx-cf-op")
                                    .selected_text(cf_op_name(*operator))
                                    .show_ui(ui, |ui| {
                                        for op in [
                                            CfOperator::GreaterThan,
                                            CfOperator::GreaterThanOrEqual,
                                            CfOperator::LessThan,
                                            CfOperator::LessThanOrEqual,
                                            CfOperator::Equal,
                                            CfOperator::NotEqual,
                                            CfOperator::Between,
                                            CfOperator::NotBetween,
                                        ] {
                                            ui.selectable_value(operator, op, cf_op_name(op));
                                        }
                                    });
                                ui.add(egui::TextEdit::singleline(value1).desired_width(80.0));
                                if matches!(operator, CfOperator::Between | CfOperator::NotBetween)
                                {
                                    ui.label("and");
                                    ui.add(egui::TextEdit::singleline(value2).desired_width(80.0));
                                }
                            }
                            1 => {
                                ui.add(
                                    egui::TextEdit::singleline(value1)
                                        .hint_text("text")
                                        .desired_width(140.0),
                                );
                            }
                            4 => {
                                ui.label("rank");
                                ui.add(
                                    egui::TextEdit::singleline(value1)
                                        .hint_text("10")
                                        .desired_width(50.0),
                                );
                            }
                            6 => {
                                ui.add(
                                    egui::TextEdit::singleline(value1)
                                        .hint_text("=A1>B1")
                                        .desired_width(180.0),
                                );
                            }
                            _ => {}
                        }
                    });
                    // The format the rule paints with, for the dxf kinds.
                    if *kind <= 6 {
                        ui.horizontal(|ui| {
                            ui.checkbox(bold, "Bold");
                            ui.checkbox(italic, "Italic");
                            ui.checkbox(use_text_color, "Text");
                            if *use_text_color {
                                ui.color_edit_button_srgb(text_color);
                            }
                            ui.checkbox(use_fill, "Fill");
                            if *use_fill {
                                ui.color_edit_button_srgb(fill_color);
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("Colour:");
                            ui.color_edit_button_srgb(fill_color);
                        });
                    }
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Cancel", false).clicked();
                        accept = dialog::button(ui, "OK", true).clicked();
                        // Set apart from the two answers: it adds a rule to the
                        // list rather than answering the dialog.
                        ui.add_space(12.0);
                        add = dialog::button(ui, "Add rule", false).clicked();
                    });
                });
                if add && !selection.is_empty() {
                    let wins = formats
                        .iter()
                        .flat_map(|f| f.rules.iter())
                        .map(|r| r.priority)
                        .min()
                        .unwrap_or(2)
                        - 1;
                    let dxf =
                        if *kind <= 6 && (*bold || *italic || *use_text_color || *use_fill) {
                            let [r, g, b] = *text_color;
                            let [fr, fg, fb] = *fill_color;
                            Some(self.doc.workbook.styles.add_dxf(ss_model::style::Dxf {
                                bold: bold.then_some(true),
                                italic: italic.then_some(true),
                                color: use_text_color.then_some(Color::rgb(r, g, b)),
                                fill: use_fill.then_some(ss_model::style::Fill::solid(Color::rgb(
                                    fr, fg, fb,
                                ))),
                                ..Default::default()
                            }))
                        } else {
                            None
                        };
                    let [fr, fg, fb] = *fill_color;
                    let visual_color = Color::rgb(fr, fg, fb);
                    let stop = |kind: CfValueKind| CfValue {
                        kind,
                        value: String::new(),
                    };
                    let rule_kind = match *kind {
                        0 => {
                            let mut formulas = vec![value1.clone()];
                            if matches!(operator, CfOperator::Between | CfOperator::NotBetween) {
                                formulas.push(value2.clone());
                            }
                            CfKind::CellIs {
                                operator: *operator,
                                formulas,
                            }
                        }
                        1 => CfKind::Text {
                            op: ss_model::cond::TextOp::Contains,
                            text: value1.clone(),
                        },
                        2 => CfKind::Duplicates { unique: false },
                        3 => CfKind::Duplicates { unique: true },
                        4 => CfKind::Top10 {
                            rank: value1.trim().parse().unwrap_or(10),
                            percent: false,
                            bottom: false,
                        },
                        5 => CfKind::AboveAverage {
                            above: true,
                            equal_average: false,
                            std_dev: None,
                        },
                        6 => CfKind::Expression {
                            formula: value1.trim_start_matches('=').to_string(),
                        },
                        7 => CfKind::ColorScale {
                            stops: vec![stop(CfValueKind::Min), stop(CfValueKind::Max)],
                            colors: vec![Color::rgb(0xFF, 0xFF, 0xFF), visual_color],
                        },
                        _ => CfKind::DataBar {
                            min: stop(CfValueKind::Min),
                            max: stop(CfValueKind::Max),
                            color: visual_color,
                            show_value: true,
                        },
                    };
                    formats.push(ss_model::cond::ConditionalFormat {
                        ranges: selection,
                        rules: vec![CfRule {
                            kind: rule_kind,
                            dxf,
                            priority: wins,
                            stop_if_true: false,
                        }],
                    });
                    value1.clear();
                    value2.clear();
                }
                if accept {
                    let sheet = self.grid.sheet_index;
                    self.perform(Change::new(
                        "Conditional formatting",
                        vec![Patch::ConditionalFormats {
                            sheet,
                            formats: formats.clone(),
                        }],
                    ));
                    self.grid.invalidate();
                    keep = false;
                }
            }

            Dialog::MoveSheet {
                index,
                before,
                copy,
            } => {
                let (index, mut go) = (*index, false);
                let names: Vec<String> = self
                    .doc
                    .workbook
                    .sheets
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                modal(ctx, "Move or copy sheet", |ui| {
                    ui.label("Before sheet:");
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for (i, name) in names.iter().enumerate() {
                                if ui.selectable_label(*before == i, name).clicked() {
                                    *before = i;
                                }
                            }
                            if ui
                                .selectable_label(*before >= names.len(), "(move to end)")
                                .clicked()
                            {
                                *before = names.len();
                            }
                        });
                    ui.checkbox(copy, "Create a copy");
                    match dialog::confirm(ui, "OK") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let (before, copy) = (*before, *copy);
                    self.move_sheet(index, before, copy);
                    keep = false;
                }
            }

            Dialog::Size { axis, text } => {
                let axis = *axis;
                let (title, unit, default, ceiling) = match axis {
                    Axis::Columns => (
                        "Column width",
                        "characters",
                        grid::axis::DEFAULT_COLUMN_CHARS,
                        255,
                    ),
                    Axis::Rows => ("Row height", "points", grid::axis::DEFAULT_ROW_POINTS, 409),
                };
                let mut accept = false;
                modal(ctx, title, |ui| {
                    ui.horizontal(|ui| {
                        let field = ui.add(egui::TextEdit::singleline(text).desired_width(90.0));
                        field.request_focus();
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            accept = true;
                        }
                        ui.label(unit);
                    });
                    // The units are the file's, not the screen's, so the number
                    // typed here is the number the next reader will see. Saying
                    // what the default is turns "8.43" from an odd number into
                    // the one everything else on the sheet already is.
                    ui.label(
                        egui::RichText::new(format!("Default {default}"))
                            .weak()
                            .small(),
                    );
                    if parse_size(text, axis).is_none() {
                        ui.colored_label(
                            egui::Color32::from_rgb(0xB0, 0x30, 0x20),
                            format!("A number from 0 to {ceiling}. Zero hides."),
                        );
                    }
                    match dialog::confirm(ui, "OK") {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if accept {
                    match parse_size(text, axis) {
                        Some(size) => {
                            self.resize_selection(axis, size);
                            keep = false;
                        }
                        // OK on an unreadable number: the dialog stays, and
                        // the status line says why — a button that silently
                        // does nothing reads as a broken button.
                        None => {
                            self.status =
                                format!("\"{}\" is not a number from 0 to {ceiling}", text.trim());
                        }
                    }
                }
            }

            Dialog::Zoom { text, fresh } => {
                let mut accept = false;
                modal(ctx, "Zoom", |ui| {
                    ui.label("Magnification");
                    let mut preset: Option<i32> = None;
                    ui.horizontal(|ui| {
                        for percent in [200, 100, 75, 50, 25] {
                            if ui.button(format!("{percent}%")).clicked() {
                                preset = Some(percent);
                            }
                        }
                    });
                    if let Some(percent) = preset {
                        *text = percent.to_string();
                        *fresh = true;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Custom:");
                        // While the field is untouched its number stays
                        // selected, re-seeded every frame — egui clears the
                        // selection on frames the field is not yet focused,
                        // and a keystroke can arrive on any frame. First
                        // touch ends it.
                        if *fresh {
                            select_zoom_percent(ui.ctx(), text);
                        }
                        let field = ui.add(
                            egui::TextEdit::singleline(text)
                                .id(egui::Id::new("calx-zoom-percent"))
                                .desired_width(56.0),
                        );
                        ui.label("%");
                        if field.changed() || field.clicked() || field.dragged() {
                            *fresh = false;
                        }
                        field.request_focus();
                    });
                    // The percent field is the only thing to type into, so
                    // Enter anywhere in the box means OK.
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        accept = true;
                    }
                    match dialog::confirm(ui, "OK") {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if accept {
                    // A percent that does not parse keeps the zoom it had.
                    if let Ok(percent) = text.trim().trim_end_matches('%').trim().parse::<f64>() {
                        self.grid.set_zoom(percent.clamp(10.0, 400.0) / 100.0);
                    }
                    keep = false;
                }
                if accept || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    // The box closes this frame, after the grid's key gate
                    // was already decided — without this, the very Enter
                    // that confirmed the zoom would also walk the cursor.
                    ctx.input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                    });
                    keep = false;
                }
            }

            Dialog::Names { names, editing } => {
                let sheet_names: Vec<String> = self
                    .doc
                    .workbook
                    .sheets
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                let here = self.grid.sheet_index;
                let selection = format!(
                    "{}!{}",
                    ss_formula::translate::quote_sheet(
                        &sheet_names[here.min(sheet_names.len() - 1)]
                    ),
                    absolute(self.grid.selection.active_range())
                );
                let mut save = false;
                let mut remove: Option<usize> = None;
                modal(ctx, "Names", |ui| {
                    ui.set_min_width(560.0);
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            egui::Grid::new("calx-names")
                                .num_columns(5)
                                .spacing([8.0, 6.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("Name").strong());
                                    ui.label(egui::RichText::new("Refers to").strong());
                                    ui.label(egui::RichText::new("Scope").strong());
                                    ui.label("");
                                    ui.end_row();
                                    for (index, entry) in names.iter_mut().enumerate() {
                                        let open = *editing == Some(index);
                                        if open {
                                            ui.add(
                                                egui::TextEdit::singleline(&mut entry.name)
                                                    .desired_width(120.0),
                                            );
                                            ui.add(
                                                egui::TextEdit::singleline(&mut entry.refers_to)
                                                    .desired_width(220.0)
                                                    .font(egui::TextStyle::Monospace),
                                            );
                                            egui::ComboBox::from_id_salt((
                                                "calx-name-scope",
                                                index,
                                            ))
                                            .selected_text(match entry.scope {
                                                None => "Workbook".to_string(),
                                                Some(i) => sheet_names
                                                    .get(i)
                                                    .cloned()
                                                    .unwrap_or_else(|| "?".into()),
                                            })
                                            .width(110.0)
                                            .show_ui(
                                                ui,
                                                |ui| {
                                                    ui.selectable_value(
                                                        &mut entry.scope,
                                                        None,
                                                        "Workbook",
                                                    );
                                                    for (i, name) in sheet_names.iter().enumerate()
                                                    {
                                                        ui.selectable_value(
                                                            &mut entry.scope,
                                                            Some(i),
                                                            name,
                                                        );
                                                    }
                                                },
                                            );
                                            if ui.button("Done").clicked() {
                                                *editing = None;
                                            }
                                        } else {
                                            ui.label(&entry.name);
                                            ui.label(
                                                egui::RichText::new(&entry.refers_to).monospace(),
                                            );
                                            ui.label(match entry.scope {
                                                None => "Workbook".to_string(),
                                                Some(i) => sheet_names
                                                    .get(i)
                                                    .cloned()
                                                    .unwrap_or_else(|| "?".into()),
                                            });
                                            ui.horizontal(|ui| {
                                                if ui.button("Edit").clicked() {
                                                    *editing = Some(index);
                                                }
                                                if ui.button("Delete").clicked() {
                                                    remove = Some(index);
                                                }
                                            });
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                    if names.is_empty() {
                        ui.label(
                            egui::RichText::new("This workbook has no names yet")
                                .weak()
                                .small(),
                        );
                    }
                    ui.add_space(6.0);
                    // Told about a clash while it is being typed rather than on
                    // submit, which is the only moment the answer is useful.
                    for (index, entry) in names.iter().enumerate() {
                        if let Some(why) = self.doc.workbook.defined_name_refusal(
                            entry.name.trim(),
                            entry.scope,
                            Some(index),
                        ) {
                            ui.colored_label(
                                egui::Color32::from_rgb(0xB0, 0x30, 0x20),
                                format!("{}: {why}", entry.name),
                            );
                        }
                    }
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Cancel", false).clicked();
                        save = dialog::button(ui, "Save", true).clicked();
                        ui.add_space(12.0);
                        if dialog::button(ui, "New", false)
                            .on_hover_text(format!("Refers to {selection}"))
                            .clicked()
                        {
                            names.push(ss_model::DefinedName {
                                name: unused_name(names),
                                refers_to: selection.clone(),
                                scope: None,
                            });
                            *editing = Some(names.len() - 1);
                        }
                    });
                });
                if let Some(index) = remove {
                    names.remove(index);
                    *editing = None;
                }
                if save {
                    let names: Vec<ss_model::DefinedName> = names
                        .iter()
                        .filter(|n| !n.name.trim().is_empty())
                        .cloned()
                        .collect();
                    self.perform(Change::new("Names", vec![Patch::DefinedNames { names }]));
                    keep = false;
                }
            }

            Dialog::FormatCells { look, tab } => {
                let mut apply = false;
                // Cloned out before the closure, which borrows `look`: a theme
                // is a dozen colours, and the alternative is to hand the whole
                // workbook to a function that wants three of them.
                let theme = self.doc.workbook.styles.theme().clone();
                modal(ctx, "Format cells", |ui| {
                    ui.set_min_width(460.0);
                    ui.horizontal(|ui| {
                        for (which, label) in FormatTab::ALL {
                            if ui.selectable_label(*tab == which, label).clicked() {
                                *tab = which;
                            }
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| match tab {
                            FormatTab::Number => number_tab(ui, look),
                            FormatTab::Alignment => alignment_tab(ui, look),
                            FormatTab::Font => font_tab(ui, &theme, look),
                            FormatTab::Border => border_tab(ui, &theme, look),
                            FormatTab::Fill => fill_tab(ui, &theme, look),
                            FormatTab::Protection => protection_tab(ui, look),
                        });
                    match dialog::confirm(ui, "OK") {
                        Some(true) => apply = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if apply {
                    let look = look.clone();
                    self.format(Format::Whole(look));
                    keep = false;
                }
            }

            Dialog::Note { at, author, text } => {
                let mut apply = false;
                let title = format!("Note on {}", at.to_a1());
                modal(ctx, &title, |ui| {
                    ui.set_width(360.0);
                    ui.horizontal(|ui| {
                        ui.label("Author");
                        ui.add(egui::TextEdit::singleline(author).desired_width(220.0));
                    });
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .desired_width(f32::INFINITY)
                            .desired_rows(5),
                    );
                    ui.add_space(4.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.small("An empty note is no note: clearing the text removes it.");
                    });
                    match dialog::confirm(ui, "OK") {
                        Some(true) => apply = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if apply {
                    let (at, author, text) = (*at, author.clone(), text.clone());
                    self.set_note(at, &author, &text);
                    keep = false;
                }
            }

            Dialog::TextToColumns { how, other } => {
                let mut go = false;
                modal(ctx, "Text to columns", |ui| {
                    ui.set_width(340.0);
                    ui.label("Split the selected column at:");
                    ui.add_space(4.0);
                    for (label, ch) in [
                        ("Tab", '\t'),
                        ("Semicolon", ';'),
                        ("Comma", ','),
                        ("Space", ' '),
                    ] {
                        let mut on = how.delimiters.contains(&ch);
                        if ui.checkbox(&mut on, label).changed() {
                            if on {
                                how.delimiters.push(ch);
                            } else {
                                how.delimiters.retain(|d| *d != ch);
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("Other");
                        // One character, because a delimiter is one: a box
                        // holding `, ` would quietly split on neither.
                        if ui
                            .add(egui::TextEdit::singleline(other).desired_width(40.0))
                            .changed()
                        {
                            other.truncate(other.chars().count().min(1));
                        }
                    });
                    ui.add_space(6.0);
                    ui.checkbox(&mut how.merge, "Treat consecutive delimiters as one");
                    let mut quoted = how.quote.is_some();
                    if ui
                        .checkbox(&mut quoted, "Text in quotes stays together")
                        .changed()
                    {
                        how.quote = quoted.then_some('"');
                    }
                    ui.add_space(6.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        for line in [
                            "The fields land in the column itself and the ones",
                            "to its right, over whatever is already there.",
                        ] {
                            ui.small(line);
                        }
                    });
                    match dialog::confirm(ui, "Split") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let mut how = how.clone();
                    if let Some(c) = other.chars().next() {
                        if !how.delimiters.contains(&c) {
                            how.delimiters.push(c);
                        }
                    }
                    self.text_to_columns(&how);
                    keep = false;
                }
            }

            Dialog::RemoveDuplicates {
                range,
                columns,
                header,
            } => {
                let mut go = false;
                let where_ = format!("{}:{}", range.start.to_a1(), range.end.to_a1());
                modal(ctx, "Remove duplicates", |ui| {
                    ui.set_width(300.0);
                    ui.label(format!("Looking through {where_}"));
                    ui.add_space(4.0);
                    ui.checkbox(header, "My data has headers");
                    ui.add_space(4.0);
                    ui.label("Rows repeat when these columns match:");
                    ui.horizontal(|ui| {
                        if ui.small_button("Select all").clicked() {
                            for (_, on) in columns.iter_mut() {
                                *on = true;
                            }
                        }
                        if ui.small_button("Select none").clicked() {
                            for (_, on) in columns.iter_mut() {
                                *on = false;
                            }
                        }
                    });
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for (col, on) in columns.iter_mut() {
                                ui.checkbox(on, ss_model::column_name(*col));
                            }
                        });
                    match dialog::confirm(ui, "Remove") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let chosen: Vec<u32> = columns
                        .iter()
                        .filter(|(_, on)| *on)
                        .map(|(c, _)| *c)
                        .collect();
                    let (range, header) = (*range, *header);
                    self.remove_duplicates(range, &chosen, header);
                    keep = false;
                }
            }

            Dialog::Protect { allow } => {
                let mut go = false;
                modal(ctx, "Protect sheet", |ui| {
                    ui.set_width(340.0);
                    ui.label("Allow everyone who uses this sheet to:");
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            for (label, field) in protection_fields(allow) {
                                ui.checkbox(field, label);
                            }
                        });
                    ui.add_space(6.0);
                    // Broken by hand and laid out left to right explicitly: a
                    // modal stretches its children and centres what is in them,
                    // which turns a paragraph into a monument.
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        for line in [
                            "Protection guards against accidents rather than",
                            "against anyone determined: Calx sets no password,",
                            "and a sheet protected here can be unprotected here.",
                        ] {
                            ui.small(line);
                        }
                    });
                    match dialog::confirm(ui, "Protect") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let allow = (**allow).clone();
                    self.protect(Some(allow));
                    keep = false;
                }
            }

            Dialog::PasteSpecial { how } => {
                let mut go = false;
                modal(ctx, "Paste special", |ui| {
                    for kind in ss_formula::clip::PasteKind::ALL {
                        ui.radio_value(&mut how.kind, kind, kind.label());
                    }
                    ui.add_space(6.0);
                    ui.checkbox(&mut how.transpose, "Transpose");
                    ui.checkbox(&mut how.skip_blanks, "Skip blanks")
                        .on_hover_text("A blank in the copy leaves what is already there");
                    match dialog::confirm(ui, "Paste") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let how = *how;
                    let text = self.os_clipboard_text();
                    self.paste_how(text, how);
                    keep = false;
                }
            }

            Dialog::Trouble {
                title,
                severity,
                message,
                detail,
                offer_save_as,
            } => {
                let offer = *offer_save_as;
                // Save As is the way out of a refused save, so it is the
                // default when it is offered at all: the box is not asking
                // whether the news was received, it is offering somewhere else
                // to put the work.
                let choices: &[dialog::Choice] = if offer {
                    &[
                        dialog::Choice::new("Save As…").primary(),
                        dialog::Choice::new("OK").escapes(),
                    ]
                } else {
                    &[dialog::Choice::new("OK").primary().escapes()]
                };
                let answer = dialog::message(
                    ctx,
                    "trouble",
                    *severity,
                    title,
                    message,
                    Some(detail.as_str()),
                    choices,
                );
                if answer.is_some() {
                    keep = false;
                }
                if offer && answer == Some(0) {
                    self.save_as();
                }
            }

            Dialog::Find {
                query,
                with,
                replacing,
                whole_workbook,
                report,
            } => {
                let mut command: Option<FindCommand> = None;
                let title = if *replacing { "Replace" } else { "Find" };
                modal(ctx, title, |ui| {
                    egui::Grid::new("calx-find-fields")
                        .num_columns(2)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Find what");
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut query.needle).desired_width(240.0),
                            );
                            if query.needle.is_empty() {
                                field.request_focus();
                            }
                            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                command = Some(FindCommand::Next);
                            }
                            ui.end_row();
                            if *replacing {
                                ui.label("Replace with");
                                ui.add(
                                    egui::TextEdit::singleline(with)
                                        .desired_width(240.0)
                                        .hint_text("(nothing)"),
                                );
                                ui.end_row();
                            }
                        });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut query.match_case, "Match case");
                        ui.checkbox(&mut query.whole_cell, "Whole cell");
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(whole_workbook, "All sheets");
                        // Excel calls this Look in, and the choice decides what
                        // a replacement can reach: only the source of a cell is
                        // ever rewritten.
                        ui.label("Look in");
                        egui::ComboBox::from_id_salt("calx-find-in")
                            .selected_text(if query.in_formulas {
                                "Formulas"
                            } else {
                                "Values"
                            })
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut query.in_formulas, true, "Formulas");
                                ui.selectable_value(&mut query.in_formulas, false, "Values");
                            });
                    });
                    ui.label(
                        egui::RichText::new("* matches any run, ? any one character")
                            .weak()
                            .small(),
                    );
                    if !report.is_empty() {
                        ui.label(egui::RichText::new(report.as_str()).small());
                    }
                    // Right to left, so this reads backwards: Close on the
                    // right, then the searches, with Find next — the one Enter
                    // in the field also does — filled and furthest left.
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Close", false).clicked();
                        ui.add_space(12.0);
                        if *replacing {
                            if dialog::button(ui, "Replace all", false).clicked() {
                                command = Some(FindCommand::ReplaceAll);
                            }
                            if dialog::button(ui, "Replace", false).clicked() {
                                command = Some(FindCommand::ReplaceOne);
                            }
                        }
                        if dialog::button(ui, "Find previous", false).clicked() {
                            command = Some(FindCommand::Previous);
                        }
                        if dialog::button(ui, "Find next", true).clicked() {
                            command = Some(FindCommand::Next);
                        }
                    });
                });
                if let Some(command) = command {
                    let outcome = self.find_command(command, query, with, *whole_workbook);
                    *report = outcome;
                }
            }

            Dialog::Sort {
                range,
                header,
                levels,
            } => {
                let (range, mut go) = (*range, false);
                let columns: Vec<u32> = (range.start.col..=range.end.col).collect();
                let names: Vec<String> = columns
                    .iter()
                    .map(|col| self.column_label(range, *header, *col))
                    .collect();
                modal(ctx, "Sort", |ui| {
                    ui.label(format!("Range {}", range_label(range)));
                    ui.checkbox(header, "My data has headers");
                    ui.add_space(6.0);
                    egui::Grid::new("calx-sort-levels")
                        .num_columns(3)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            for (n, level) in levels.iter_mut().enumerate() {
                                ui.label(if n == 0 { "Sort by" } else { "Then by" });
                                let selected = match level.col {
                                    Some(col) => columns
                                        .iter()
                                        .position(|c| *c == col)
                                        .map_or("(none)".to_string(), |i| names[i].clone()),
                                    None => "(none)".to_string(),
                                };
                                egui::ComboBox::from_id_salt(("calx-sort-col", n))
                                    .selected_text(selected)
                                    .width(180.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(level.col.is_none(), "(none)")
                                            .clicked()
                                        {
                                            level.col = None;
                                        }
                                        for (col, name) in columns.iter().zip(&names) {
                                            if ui
                                                .selectable_label(level.col == Some(*col), name)
                                                .clicked()
                                            {
                                                level.col = Some(*col);
                                            }
                                        }
                                    });
                                egui::ComboBox::from_id_salt(("calx-sort-dir", n))
                                    .selected_text(if level.descending {
                                        "Z to A"
                                    } else {
                                        "A to Z"
                                    })
                                    .width(90.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(!level.descending, "A to Z")
                                            .clicked()
                                        {
                                            level.descending = false;
                                        }
                                        if ui.selectable_label(level.descending, "Z to A").clicked()
                                        {
                                            level.descending = true;
                                        }
                                    });
                                ui.end_row();
                            }
                        });
                    match dialog::confirm(ui, "Sort") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let (header, levels) = (*header, *levels);
                    self.sort_by(range, header, &levels);
                    keep = false;
                }
            }

            Dialog::Filter {
                col,
                offered,
                has_blanks,
                ticked,
                blanks,
                search,
            } => {
                let (col, has_blanks) = (*col, *has_blanks);
                let mut go = false;
                let mut clear = false;
                modal(ctx, "Filter", |ui| {
                    ui.text_edit_singleline(search)
                        .on_hover_text("Narrows the list below; it does not filter by itself");
                    let needle = search.to_lowercase();
                    let visible: Vec<&String> = offered
                        .iter()
                        .filter(|v| needle.is_empty() || v.to_lowercase().contains(&needle))
                        .collect();

                    ui.horizontal(|ui| {
                        if ui.small_button("Select all").clicked() {
                            for value in &visible {
                                ticked.insert((*value).clone());
                            }
                            *blanks = has_blanks;
                        }
                        if ui.small_button("Select none").clicked() {
                            for value in &visible {
                                ticked.remove(*value);
                            }
                            *blanks = false;
                        }
                    });
                    ui.separator();

                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for value in visible {
                                let mut on = ticked.contains(value);
                                if ui.checkbox(&mut on, value).changed() {
                                    if on {
                                        ticked.insert(value.clone());
                                    } else {
                                        ticked.remove(value);
                                    }
                                }
                            }
                            if has_blanks && needle.is_empty() {
                                ui.checkbox(blanks, "(Blanks)");
                            }
                        });
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Cancel", false).clicked();
                        go = dialog::button(ui, "OK", true).clicked();
                        ui.add_space(12.0);
                        clear = dialog::button(ui, "Clear this column", false).clicked();
                    });
                });
                if go || clear {
                    let (ticked, blanks) = if clear {
                        (offered.iter().cloned().collect(), has_blanks)
                    } else {
                        (ticked.clone(), *blanks)
                    };
                    let offered = offered.clone();
                    self.set_filter_column(col, ticked, blanks, &offered, has_blanks);
                    keep = false;
                }
            }
        }
        // Escape cancels whichever dialog is up, the way Cancel would —
        // every one of them, because a window Escape cannot leave is a trap.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            keep = false;
        }
        if keep {
            self.dialog = Some(dialog);
        }
    }

    /// What to call a column in the sort dialog: its heading if the range has
    /// one, else the column letter.
    fn column_label(&self, range: CellRange, header: bool, col: u32) -> String {
        let letter = ss_model::column_name(col);
        if !header {
            return format!("Column {letter}");
        }
        let text = self
            .doc
            .workbook
            .sheet(self.grid.sheet_index)
            .and_then(|s| s.get(CellRef::new(range.start.row, col)))
            .and_then(|c| match c.value {
                ss_model::CellValue::Text(id) => {
                    Some(self.doc.workbook.strings.resolve(id).to_string())
                }
                ss_model::CellValue::Number(n) => Some(ss_model::format_general(n)),
                _ => None,
            })
            .filter(|t| !t.trim().is_empty());
        match text {
            Some(text) => format!("{text} ({letter})"),
            None => format!("Column {letter}"),
        }
    }

    /// The right-click menu over the grid.
    fn context_menu(&mut self, ui: &mut egui::Ui) {
        let mut requested = None;
        let cursor = self.grid.selection.cursor();
        ui.label(
            egui::RichText::new(self.grid.selection.active_range_label())
                .weak()
                .small(),
        );
        ui.separator();
        for (label, action) in [
            ("Cut", Action::Copy { cut: true }),
            ("Copy", Action::Copy { cut: false }),
        ] {
            if ui.button(label).clicked() {
                requested = Some(action);
                ui.close();
            }
        }
        if ui.button("Paste").clicked() {
            requested = Some(Action::Paste(self.os_clipboard_text()));
            ui.close();
        }
        // The two people reach for most, straight on the menu; the rest are
        // one more click away in the dialog.
        let mut special: Option<Option<ss_formula::clip::PasteKind>> = None;
        for (label, kind) in [
            ("Paste values", ss_formula::clip::PasteKind::Values),
            ("Paste formats", ss_formula::clip::PasteKind::Formats),
        ] {
            if ui.button(label).clicked() {
                special = Some(Some(kind));
                ui.close();
            }
        }
        if ui.button("Paste special…").clicked() {
            special = Some(None);
            ui.close();
        }
        match special {
            Some(Some(kind)) => {
                let text = self.os_clipboard_text();
                self.paste_how(
                    text,
                    ss_formula::clip::PasteSpecial {
                        kind,
                        ..Default::default()
                    },
                );
            }
            Some(None) => {
                self.dialog = Some(Dialog::PasteSpecial {
                    how: Default::default(),
                })
            }
            None => {}
        }
        ui.separator();
        let has_note = self
            .doc
            .workbook
            .sheet(self.grid.sheet_index)
            .is_some_and(|s| s.comments.iter().any(|note| note.at == cursor));
        let mut note = false;
        if ui
            .button(if has_note { "Edit note" } else { "Insert note" })
            .on_hover_text("Shift+F2")
            .clicked()
        {
            note = true;
            ui.close();
        }
        if note {
            self.open_note();
        }
        ui.separator();
        for (label, action) in [
            ("Insert rows", Action::Insert(Axis::Rows)),
            ("Delete rows", Action::Delete(Axis::Rows)),
            ("Insert columns", Action::Insert(Axis::Columns)),
            ("Delete columns", Action::Delete(Axis::Columns)),
        ] {
            if ui.button(label).clicked() {
                requested = Some(action);
                ui.close();
            }
        }
        ui.separator();
        for (label, action) in [
            (
                "Hide rows",
                Action::Visibility {
                    axis: Axis::Rows,
                    hide: true,
                },
            ),
            (
                "Unhide rows",
                Action::Visibility {
                    axis: Axis::Rows,
                    hide: false,
                },
            ),
            (
                "Hide columns",
                Action::Visibility {
                    axis: Axis::Columns,
                    hide: true,
                },
            ),
            (
                "Unhide columns",
                Action::Visibility {
                    axis: Axis::Columns,
                    hide: false,
                },
            ),
            (
                "Group rows",
                Action::Group {
                    axis: Axis::Rows,
                    ungroup: false,
                },
            ),
            (
                "Ungroup rows",
                Action::Group {
                    axis: Axis::Rows,
                    ungroup: true,
                },
            ),
            (
                "Group columns",
                Action::Group {
                    axis: Axis::Columns,
                    ungroup: false,
                },
            ),
            (
                "Ungroup columns",
                Action::Group {
                    axis: Axis::Columns,
                    ungroup: true,
                },
            ),
            ("Fit columns to contents", Action::AutoFit(Axis::Columns)),
            ("Fit rows to contents", Action::AutoFit(Axis::Rows)),
        ] {
            if ui.button(label).clicked() {
                requested = Some(action);
                ui.close();
            }
        }
        // Not an `Action`: the dialog belongs to the application, and nothing
        // has happened to the document yet for the grid to be told about.
        let mut size: Option<Axis> = None;
        if ui.button("Column width…").clicked() {
            size = Some(Axis::Columns);
            ui.close();
        }
        if ui.button("Row height…").clicked() {
            size = Some(Axis::Rows);
            ui.close();
        }
        if let Some(axis) = size {
            self.open_size_dialog(axis);
        }
        ui.separator();
        let merged = self
            .doc
            .workbook
            .sheet(self.grid.sheet_index)
            .is_some_and(|s| s.merge_at(cursor).is_some());
        if ui
            .button(if merged { "Unmerge" } else { "Merge cells" })
            .clicked()
        {
            requested = Some(Action::Merge(!merged));
            ui.close();
        }
        if ui.button("Clear contents").clicked() {
            requested = Some(Action::Clear);
            ui.close();
        }
        let mut find_dialog = false;
        let mut validation = false;
        let mut cond = false;
        if ui.button("Data validation…").clicked() {
            validation = true;
            ui.close();
        }
        if ui.button("Conditional formatting…").clicked() {
            cond = true;
            ui.close();
        }
        if ui.button("Find and replace…").clicked() {
            find_dialog = true;
            ui.close();
        }
        if find_dialog {
            self.open_find(true);
        }
        if validation {
            self.open_validation();
        }
        if cond {
            self.open_cond_format();
        }
        if ui.button("Clear formatting").clicked() {
            requested = Some(Action::Format(Format::Clear));
            ui.close();
        }
        let mut format_cells = false;
        if ui.button("Format cells…").on_hover_text("Ctrl+1").clicked() {
            format_cells = true;
            ui.close();
        }
        if format_cells {
            self.open_format_cells();
        }
        ui.separator();
        let mut deferred: Option<FilterCommand> = None;
        let mut sort: Option<bool> = None;
        ui.menu_button("Sort", |ui| {
            if ui.button("A to Z").clicked() {
                sort = Some(false);
                ui.close();
            }
            if ui.button("Z to A").clicked() {
                sort = Some(true);
                ui.close();
            }
            if ui.button("Custom sort…").clicked() {
                deferred = Some(FilterCommand::SortDialog);
                ui.close();
            }
        });
        let filtered = self
            .doc
            .workbook
            .sheet(self.grid.sheet_index)
            .is_some_and(|s| s.filter.is_some());
        if ui
            .button(if filtered { "Remove filter" } else { "Filter" })
            .clicked()
        {
            deferred = Some(FilterCommand::Toggle);
            ui.close();
        }

        if let Some(action) = requested {
            let ui = &*ui;
            self.act(ui, action);
        }
        if let Some(descending) = sort {
            self.sort_quick(descending);
        }
        match deferred {
            Some(FilterCommand::SortDialog) => self.open_sort_dialog(),
            Some(FilterCommand::Toggle) => self.toggle_filter(),
            Some(FilterCommand::Clear) => self.clear_filter(),
            Some(FilterCommand::Reapply) => self.reapply_filter(),
            None => {}
        }
    }
}

/// What the Sort & Filter group asked for, deferred out of the layout closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterCommand {
    SortDialog,
    Toggle,
    Clear,
    Reapply,
}

/// Which of the Find window's four buttons was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindCommand {
    Next,
    Previous,
    ReplaceOne,
    ReplaceAll,
}

/// What a tab's right-click menu asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TabCommand {
    Insert,
    Delete,
    Rename,
    MoveOrCopy,
    Color(Option<Color>),
    Hide,
    UnhideAll,
    SelectAll,
}

/// Excel's standard tab colours, which is the palette people recognise.
const TAB_COLORS: &[(&str, [u8; 3])] = &[
    ("Red", [0xC0, 0x00, 0x00]),
    ("Orange", [0xE3, 0x6C, 0x0A]),
    ("Yellow", [0xFF, 0xC0, 0x00]),
    ("Green", [0x00, 0xB0, 0x50]),
    ("Blue", [0x00, 0x70, 0xC0]),
    ("Purple", [0x70, 0x30, 0xA0]),
    ("Grey", [0x80, 0x80, 0x80]),
];

/// A menu entry showing the colour it names.
fn color_swatch(ui: &mut egui::Ui, label: &str, [r, g, b]: [u8; 3]) -> egui::Response {
    let response = ui.button(format!("      {label}"));
    let box_ = egui::Rect::from_min_size(
        response.rect.min + egui::vec2(6.0, 4.0),
        egui::vec2(12.0, response.rect.height() - 8.0),
    );
    ui.painter().rect(
        box_,
        egui::CornerRadius::same(2),
        egui::Color32::from_rgb(r, g, b),
        egui::Stroke::new(1.0, egui::Color32::from_gray(0x66)),
        egui::StrokeKind::Inside,
    );
    response
}

/// `A1:D9 F2:F9`, the way a dialog names where a rule applies.
fn ranges_label(ranges: &[CellRange]) -> String {
    ranges
        .iter()
        .map(|r| {
            if r.start == r.end {
                r.start.to_a1()
            } else {
                format!("{}:{}", r.start.to_a1(), r.end.to_a1())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn dv_kind_name(kind: ss_model::cond::DvKind) -> &'static str {
    use ss_model::cond::DvKind;
    match kind {
        DvKind::None => "Any value",
        DvKind::Whole => "Whole number",
        DvKind::Decimal => "Decimal",
        DvKind::List => "List",
        DvKind::Date => "Date",
        DvKind::Time => "Time",
        DvKind::TextLength => "Text length",
        DvKind::Custom => "Custom formula",
    }
}

fn dv_op_name(op: ss_model::cond::DvOperator) -> &'static str {
    use ss_model::cond::DvOperator;
    match op {
        DvOperator::Between => "between",
        DvOperator::NotBetween => "not between",
        DvOperator::Equal => "equal to",
        DvOperator::NotEqual => "not equal to",
        DvOperator::GreaterThan => "greater than",
        DvOperator::LessThan => "less than",
        DvOperator::GreaterThanOrEqual => "at least",
        DvOperator::LessThanOrEqual => "at most",
    }
}

fn cf_op_name(op: ss_model::cond::CfOperator) -> &'static str {
    use ss_model::cond::CfOperator;
    match op {
        CfOperator::GreaterThan => "greater than",
        CfOperator::GreaterThanOrEqual => "at least",
        CfOperator::LessThan => "less than",
        CfOperator::LessThanOrEqual => "at most",
        CfOperator::Equal => "equal to",
        CfOperator::NotEqual => "not equal to",
        CfOperator::Between => "between",
        CfOperator::NotBetween => "not between",
        CfOperator::ContainsText => "containing",
        CfOperator::NotContains => "not containing",
        CfOperator::BeginsWith => "beginning with",
        CfOperator::EndsWith => "ending with",
    }
}

/// One line of the rule manager: what a rule does, in words.
fn cf_rule_label(rule: &ss_model::cond::CfRule) -> String {
    use ss_model::cond::{CfKind, TextOp};
    match &rule.kind {
        CfKind::CellIs { operator, formulas } => format!(
            "cell is {} {}",
            cf_op_name(*operator),
            formulas.join(" and ")
        ),
        CfKind::Expression { formula } => format!("formula ={formula}"),
        CfKind::Text { op, text } => format!(
            "text {} \"{text}\"",
            match op {
                TextOp::Contains => "contains",
                TextOp::NotContains => "does not contain",
                TextOp::BeginsWith => "begins with",
                TextOp::EndsWith => "ends with",
            }
        ),
        CfKind::ColorScale { .. } => "colour scale".to_string(),
        CfKind::DataBar { .. } => "data bar".to_string(),
        CfKind::IconSet { set, .. } => format!("icon set {set}"),
        CfKind::Top10 {
            rank,
            percent,
            bottom,
        } => format!(
            "{} {rank}{}",
            if *bottom { "bottom" } else { "top" },
            if *percent { "%" } else { "" }
        ),
        CfKind::AboveAverage { above, .. } => if *above {
            "above average"
        } else {
            "below average"
        }
        .to_string(),
        CfKind::TimePeriod { period } => format!("time period {period}"),
        CfKind::Duplicates { unique } => if *unique {
            "unique values"
        } else {
            "duplicate values"
        }
        .to_string(),
        CfKind::Presence { blanks, negated } => match (blanks, negated) {
            (true, false) => "blanks",
            (true, true) => "no blanks",
            (false, false) => "errors",
            (false, true) => "no errors",
        }
        .to_string(),
        CfKind::Other(name) => name.clone(),
    }
}

/// A form dialog: centred, not resizable, and blocking the rest of the app.
///
/// The title bar, the gutter and the frame come from `ui_kit::dialog` so that a
/// form and a message box are recognisably the same furniture; what the form
/// puts inside is its own business, and its buttons go in `dialog::actions`.
fn modal(ctx: &egui::Context, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Modal::new(egui::Id::new(("calx-modal", title)))
        .frame(dialog::frame(ctx))
        .show(ctx, |ui| {
            ui.set_min_width(340.0);
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 22,
                    right: 22,
                    top: 18,
                    bottom: 18,
                })
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(title).font(dialog::heading_font(16.0)));
                    ui.add_space(12.0);
                    add(ui);
                });
        });
}

/// Excel's zoom slider: 10–400% with 100% at the centre of the track, a notch
/// marking it, and a detent that snaps the thumb onto it. Each half of the
/// track is linear in its own range, which is why 100% can sit in the middle.
fn zoom_slider(ui: &mut egui::Ui, percent: &mut f64) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(100.0, 16.0), egui::Sense::click_and_drag());
    if response.dragged() || response.clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let mut t = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
            if (t - 0.5).abs() < 0.04 {
                t = 0.5;
            }
            *percent = if t <= 0.5 {
                10.0 + t / 0.5 * 90.0
            } else {
                100.0 + (t - 0.5) / 0.5 * 300.0
            }
            .round();
        }
    }
    if ui.is_rect_visible(rect) {
        let line = egui::Stroke::new(1.0, ui.visuals().widgets.inactive.fg_stroke.color);
        let y = rect.center().y;
        let painter = ui.painter();
        painter.hline(rect.x_range(), y, line);
        painter.vline(rect.center().x, egui::Rangef::new(y - 4.0, y + 4.0), line);
        let t = if *percent <= 100.0 {
            (*percent - 10.0) / 90.0 * 0.5
        } else {
            0.5 + (*percent - 100.0) / 300.0 * 0.5
        };
        let x = rect.left() + t.clamp(0.0, 1.0) as f32 * rect.width();
        let visuals = ui.style().interact(&response);
        painter.rect(
            egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(8.0, 14.0)),
            2.0,
            visuals.bg_fill,
            visuals.fg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text("Zoom (Ctrl+scroll)");
}

/// Leaves the Zoom box's percent field with its whole number selected, so the
/// next keystroke replaces it — the way Excel's box behaves.
fn select_zoom_percent(ctx: &egui::Context, text: &str) {
    let mut state = egui::text_edit::TextEditState::default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(text.chars().count()),
        )));
    state.store(ctx, egui::Id::new("calx-zoom-percent"));
}

/// Format Cells ▸ Number. A category list and the code behind it.
///
/// The code box is not a nicety: `#,##0.00;[Red](#,##0.00)` is the only way to
/// say some of what a spreadsheet says, and a fixed list of a dozen entries
/// cannot cover a format language.
fn number_tab(ui: &mut egui::Ui, look: &mut ss_model::Look) {
    ui.label("Category");
    for (label, code) in NUMBER_FORMATS {
        if ui
            .selectable_label(look.number_format == *code, *label)
            .clicked()
        {
            look.number_format = code.to_string();
        }
    }
    ui.add_space(6.0);
    ui.label("Format code");
    ui.add(
        egui::TextEdit::singleline(&mut look.number_format)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace),
    );
    // What the code does to a number, before the dialog is closed over it.
    let sample = ss_model::numfmt::NumberFormat::parse(&look.number_format)
        .format(ss_model::numfmt::FormatValue::Number(-1234.567))
        .text;
    ui.label(
        egui::RichText::new(format!("−1234.567 shows as  {sample}"))
            .weak()
            .small(),
    );
}

/// Format Cells ▸ Alignment.
fn alignment_tab(ui: &mut egui::Ui, look: &mut ss_model::Look) {
    egui::Grid::new("calx-align")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Horizontal");
            egui::ComboBox::from_id_salt("calx-halign")
                .selected_text(halign_name(look.alignment.horizontal))
                .width(160.0)
                .show_ui(ui, |ui| {
                    for h in [
                        HAlign::General,
                        HAlign::Left,
                        HAlign::Center,
                        HAlign::Right,
                        HAlign::Fill,
                        HAlign::Justify,
                        HAlign::CenterContinuous,
                        HAlign::Distributed,
                    ] {
                        ui.selectable_value(&mut look.alignment.horizontal, h, halign_name(h));
                    }
                });
            ui.end_row();

            ui.label("Vertical");
            egui::ComboBox::from_id_salt("calx-valign")
                .selected_text(valign_name(look.alignment.vertical))
                .width(160.0)
                .show_ui(ui, |ui| {
                    for v in [
                        VAlign::Top,
                        VAlign::Center,
                        VAlign::Bottom,
                        VAlign::Justify,
                        VAlign::Distributed,
                    ] {
                        ui.selectable_value(&mut look.alignment.vertical, v, valign_name(v));
                    }
                });
            ui.end_row();

            ui.label("Indent");
            ui.add(egui::DragValue::new(&mut look.alignment.indent).range(0..=250));
            ui.end_row();

            // Excel stores 0–90 for anticlockwise and 91–180 for the clockwise
            // mirror, so the number in the file is not the angle. The dialog
            // shows the angle and converts, because −45 is what anybody means.
            ui.label("Rotation");
            let mut degrees = rotation_degrees(look.alignment.rotation);
            let stacked = look.alignment.rotation == 255;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!stacked, |ui| {
                    if ui
                        .add(
                            egui::DragValue::new(&mut degrees)
                                .range(-90..=90)
                                .suffix("°"),
                        )
                        .changed()
                    {
                        look.alignment.rotation = rotation_stored(degrees);
                    }
                });
                let mut on = stacked;
                if ui
                    .checkbox(&mut on, "Stacked")
                    .on_hover_text("One character above the next, which is rotation 255")
                    .changed()
                {
                    look.alignment.rotation = if on { 255 } else { 0 };
                }
            });
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.checkbox(&mut look.alignment.wrap, "Wrap text");
    ui.checkbox(&mut look.alignment.shrink, "Shrink to fit");
}

fn halign_name(h: HAlign) -> &'static str {
    match h {
        HAlign::General => "General",
        HAlign::Left => "Left",
        HAlign::Center => "Centre",
        HAlign::Right => "Right",
        HAlign::Fill => "Fill",
        HAlign::Justify => "Justify",
        HAlign::CenterContinuous => "Centre across selection",
        HAlign::Distributed => "Distributed",
    }
}

fn valign_name(v: VAlign) -> &'static str {
    match v {
        VAlign::Top => "Top",
        VAlign::Center => "Centre",
        VAlign::Bottom => "Bottom",
        VAlign::Justify => "Justify",
        VAlign::Distributed => "Distributed",
    }
}

/// The angle a stored rotation means. 91–180 is Excel's spelling of −1 to −90.
fn rotation_degrees(stored: u32) -> i32 {
    match stored {
        255 => 0,
        r if r > 90 => -((r as i32) - 90),
        r => r as i32,
    }
}

fn rotation_stored(degrees: i32) -> u32 {
    if degrees >= 0 {
        degrees.min(90) as u32
    } else {
        (90 + degrees.abs().min(90)) as u32
    }
}

/// Format Cells ▸ Font.
fn font_tab(ui: &mut egui::Ui, theme: &ss_model::color::Theme, look: &mut ss_model::Look) {
    egui::Grid::new("calx-font-tab")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Font");
            egui::ComboBox::from_id_salt("calx-font-tab-name")
                .selected_text(look.font.name.clone())
                .width(190.0)
                .show_ui(ui, |ui| {
                    // The workbook's own face first when nobody offers it, so
                    // a document set in something exotic can still be got back
                    // to after the list has been opened.
                    let mut offered: Vec<String> =
                        FONT_NAMES.iter().map(|f| f.to_string()).collect();
                    if !offered.contains(&look.font.name) {
                        offered.insert(0, look.font.name.clone());
                    }
                    for choice in offered {
                        if ui
                            .selectable_label(choice == look.font.name, &choice)
                            .clicked()
                        {
                            look.font.name = choice;
                        }
                    }
                });
            ui.end_row();

            ui.label("Size");
            ui.add(
                egui::DragValue::new(&mut look.font.size)
                    .range(1.0..=409.0)
                    .speed(0.5),
            );
            ui.end_row();

            ui.label("Underline");
            egui::ComboBox::from_id_salt("calx-underline")
                .selected_text(underline_name(look.font.underline))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for u in [
                        Underline::None,
                        Underline::Single,
                        Underline::Double,
                        Underline::SingleAccounting,
                        Underline::DoubleAccounting,
                    ] {
                        ui.selectable_value(&mut look.font.underline, u, underline_name(u));
                    }
                });
            ui.end_row();

            ui.label("Position");
            egui::ComboBox::from_id_salt("calx-vertalign")
                .selected_text(match look.font.vert_align {
                    None => "Normal",
                    Some(ss_model::style::VertAlign::Superscript) => "Superscript",
                    Some(ss_model::style::VertAlign::Subscript) => "Subscript",
                })
                .width(190.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut look.font.vert_align, None, "Normal");
                    ui.selectable_value(
                        &mut look.font.vert_align,
                        Some(ss_model::style::VertAlign::Superscript),
                        "Superscript",
                    );
                    ui.selectable_value(
                        &mut look.font.vert_align,
                        Some(ss_model::style::VertAlign::Subscript),
                        "Subscript",
                    );
                });
            ui.end_row();

            ui.label("Colour");
            color_row(ui, theme, &mut look.font.color);
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut look.font.bold, "Bold");
        ui.checkbox(&mut look.font.italic, "Italic");
        ui.checkbox(&mut look.font.strike, "Strikethrough");
    });
}

fn underline_name(u: Underline) -> &'static str {
    match u {
        Underline::None => "None",
        Underline::Single => "Single",
        Underline::Double => "Double",
        Underline::SingleAccounting => "Single, accounting",
        Underline::DoubleAccounting => "Double, accounting",
    }
}

/// A colour, with a way back to "automatic".
///
/// Automatic is not a colour and no picker can express it, which is why it
/// needs a checkbox of its own: without one, a border once given a colour
/// could never be handed back to the theme.
fn color_row(ui: &mut egui::Ui, theme: &ss_model::color::Theme, color: &mut Color) {
    ui.horizontal(|ui| {
        let mut rgb = color.resolve(theme).unwrap_or([0, 0, 0]);
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            let [r, g, b] = rgb;
            *color = Color::rgb(r, g, b);
        }
        let mut automatic = *color == Color::Auto;
        if ui.checkbox(&mut automatic, "Automatic").changed() {
            *color = if automatic {
                Color::Auto
            } else {
                let [r, g, b] = rgb;
                Color::rgb(r, g, b)
            };
        }
    });
}

/// Format Cells ▸ Border, one edge at a time.
///
/// Per edge rather than by preset, because the presets on the toolbar cannot
/// say "a thick red line under this and a hairline down the side", and that is
/// the whole reason to open a dialog rather than press a button.
fn border_tab(ui: &mut egui::Ui, theme: &ss_model::color::Theme, look: &mut ss_model::Look) {
    let presets = [
        ("All", BorderPreset::All),
        ("Outline", BorderPreset::Outline),
        ("None", BorderPreset::None),
    ];
    ui.horizontal(|ui| {
        ui.label("Quick");
        for (label, preset) in presets {
            if ui.button(label).clicked() {
                preset.apply(&mut look.border);
            }
        }
    });
    ui.add_space(6.0);
    egui::Grid::new("calx-border-edges")
        .num_columns(3)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            let edges: [BorderEdge; 5] = [
                ("Left", |b| &mut b.left),
                ("Right", |b| &mut b.right),
                ("Top", |b| &mut b.top),
                ("Bottom", |b| &mut b.bottom),
                ("Diagonal", |b| &mut b.diagonal),
            ];
            for (name, pick) in edges {
                let edge = pick(&mut look.border);
                ui.label(name);
                egui::ComboBox::from_id_salt(("calx-border", name))
                    .selected_text(border_style_name(edge.style))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for style in BORDER_STYLES {
                            ui.selectable_value(&mut edge.style, *style, border_style_name(*style));
                        }
                    });
                color_row(ui, theme, &mut edge.color);
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut look.border.diagonal_up, "Diagonal up");
        ui.checkbox(&mut look.border.diagonal_down, "Diagonal down");
    });
}

/// One row of the border tab: what to call the edge, and how to reach it.
type BorderEdge = (
    &'static str,
    fn(&mut ss_model::Border) -> &mut ss_model::style::Edge,
);

const BORDER_STYLES: &[BorderStyle] = &[
    BorderStyle::None,
    BorderStyle::Hair,
    BorderStyle::Thin,
    BorderStyle::Medium,
    BorderStyle::Thick,
    BorderStyle::Double,
    BorderStyle::Dotted,
    BorderStyle::Dashed,
    BorderStyle::DashDot,
    BorderStyle::DashDotDot,
    BorderStyle::MediumDashed,
    BorderStyle::MediumDashDot,
    BorderStyle::MediumDashDotDot,
    BorderStyle::SlantDashDot,
];

fn border_style_name(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::None => "None",
        BorderStyle::Hair => "Hair",
        BorderStyle::Thin => "Thin",
        BorderStyle::Medium => "Medium",
        BorderStyle::Thick => "Thick",
        BorderStyle::Double => "Double",
        BorderStyle::Dotted => "Dotted",
        BorderStyle::Dashed => "Dashed",
        BorderStyle::DashDot => "Dash-dot",
        BorderStyle::DashDotDot => "Dash-dot-dot",
        BorderStyle::MediumDashed => "Medium dashed",
        BorderStyle::MediumDashDot => "Medium dash-dot",
        BorderStyle::MediumDashDotDot => "Medium dash-dot-dot",
        BorderStyle::SlantDashDot => "Slant dash-dot",
    }
}

/// Format Cells ▸ Fill.
/// Who to sign a new note as.
///
/// The account name the machine already knows, because asking for it in the
/// note dialog is a question nobody wants to answer twice — and a note signed
/// "Unknown" is worse than one signed with the name on the login.
fn author_name() -> String {
    for key in ["USERNAME", "USER", "LOGNAME"] {
        if let Some(name) = std::env::var_os(key) {
            let name = name.to_string_lossy().trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    "Author".to_string()
}

/// What the package should call an image, from the name it came in under.
///
/// The extension rather than the bytes: the package's content type is what
/// Excel reads, an extension is what the user chose, and a mismatch between
/// them is a file that was renamed rather than converted.
fn content_type_of(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        _ => "image/png",
    }
}

/// A chart's idea of a range: `'Sales figures'!$B$2:$B$9`.
///
/// Absolute and sheet-qualified, always, because a chart lives outside the
/// grid: there is no cell for a relative reference to be relative to.
fn reference(sheet: &str, first_col: u32, first_row: u32, last_col: u32, last_row: u32) -> String {
    let name = if sheet
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        sheet.to_string()
    } else {
        // A name holding a space or a punctuation mark is quoted, and a quote
        // inside one is doubled.
        format!("'{}'", sheet.replace('\'', "''"))
    };
    let cell = |col: u32, row: u32| format!("${}${}", ss_model::column_name(col), row + 1);
    if first_col == last_col && first_row == last_row {
        format!("{name}!{}", cell(first_col, first_row))
    } else {
        format!(
            "{name}!{}:{}",
            cell(first_col, first_row),
            cell(last_col, last_row)
        )
    }
}

/// The Protect Sheet checkboxes, in the order Excel lists them.
///
/// Borrowed rather than copied so the dialog edits the model value directly:
/// fifteen `checkbox` calls with fifteen field names is fifteen chances to
/// wire one to the wrong flag.
fn protection_fields(p: &mut ss_model::Protection) -> Vec<(&'static str, &mut bool)> {
    vec![
        ("Select locked cells", &mut p.select_locked),
        ("Select unlocked cells", &mut p.select_unlocked),
        ("Format cells", &mut p.format_cells),
        ("Format columns", &mut p.format_columns),
        ("Format rows", &mut p.format_rows),
        ("Insert columns", &mut p.insert_columns),
        ("Insert rows", &mut p.insert_rows),
        ("Insert hyperlinks", &mut p.insert_hyperlinks),
        ("Delete columns", &mut p.delete_columns),
        ("Delete rows", &mut p.delete_rows),
        ("Sort", &mut p.sort),
        ("Use AutoFilter", &mut p.filter),
        ("Use PivotTable reports", &mut p.pivot_tables),
        ("Edit objects", &mut p.objects),
        ("Edit scenarios", &mut p.scenarios),
    ]
}

/// Format Cells ▸ Protection: the tab that does nothing until the sheet is
/// protected, which is the single most confusing thing about it in Excel too.
fn protection_tab(ui: &mut egui::Ui, look: &mut ss_model::Look) {
    ui.checkbox(&mut look.locked, "Locked");
    ui.add_space(6.0);
    // A line per widget rather than one label with newlines in it: a modal
    // centres a multi-line galley, and a centred paragraph is a monument.
    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
        for line in [
            "Locking a cell has no effect until the sheet is protected.",
            "Every cell starts locked, so protecting a sheet with nothing",
            "unlocked locks all of it — unlock the cells people are meant",
            "to type in first.",
        ] {
            ui.small(line);
        }
    });
}

fn fill_tab(ui: &mut egui::Ui, theme: &ss_model::color::Theme, look: &mut ss_model::Look) {
    // The hatches are kept by name and drawn as a blend, so the list here is
    // the handful anybody picks; a file's own `lightTrellis` survives being
    // opened and shown among them because the model never dropped it.
    let offered: Vec<Pattern> = [
        Pattern::None,
        Pattern::Solid,
        Pattern::Named("gray125".into()),
        Pattern::Named("gray0625".into()),
        Pattern::Named("lightGray".into()),
        Pattern::Named("mediumGray".into()),
        Pattern::Named("darkGray".into()),
    ]
    .into_iter()
    .chain(
        (!matches!(look.fill.pattern, Pattern::None | Pattern::Solid))
            .then(|| look.fill.pattern.clone()),
    )
    .collect();

    egui::Grid::new("calx-fill-tab")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Pattern");
            egui::ComboBox::from_id_salt("calx-pattern")
                .selected_text(pattern_name(&look.fill.pattern))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for pattern in &offered {
                        if ui
                            .selectable_label(look.fill.pattern == *pattern, pattern_name(pattern))
                            .clicked()
                        {
                            look.fill.pattern = pattern.clone();
                        }
                    }
                });
            ui.end_row();
            ui.label("Colour");
            color_row(ui, theme, &mut look.fill.fg);
            ui.end_row();
            ui.label("Pattern colour");
            color_row(ui, theme, &mut look.fill.bg);
            ui.end_row();
        });
    ui.label(
        egui::RichText::new("A solid fill uses the first colour; a hatch uses both")
            .weak()
            .small(),
    );
}

fn pattern_name(p: &Pattern) -> String {
    match p {
        Pattern::None => "None".to_string(),
        Pattern::Solid => "Solid".to_string(),
        Pattern::Named(name) => match name.as_str() {
            "gray125" => "12.5% grey".to_string(),
            "gray0625" => "6.25% grey".to_string(),
            "lightGray" => "25% grey".to_string(),
            "mediumGray" => "50% grey".to_string(),
            "darkGray" => "75% grey".to_string(),
            other => other.to_string(),
        },
    }
}

/// A range with dollars on it, which is how a defined name refers to one.
///
/// Anchored because a name is not copied and so has nothing to be relative
/// *to*: Excel writes `$A$1:$D$9` for every one it creates, and a relative
/// name means something quite different — it shifts with whatever cell is
/// looking at it.
fn absolute(range: CellRange) -> String {
    let cell = |at: CellRef| format!("${}${}", ss_model::column_name(at.col), at.row + 1);
    if range.start == range.end {
        cell(range.start)
    } else {
        format!("{}:{}", cell(range.start), cell(range.end))
    }
}

/// `Name1`, or the first number after it that nobody is using.
fn unused_name(names: &[ss_model::DefinedName]) -> String {
    for n in 1.. {
        let candidate = format!("Name{n}");
        if !names
            .iter()
            .any(|d| d.name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
    }
    unreachable!("the loop is unbounded")
}

/// A typed row height or column width, if it is one.
///
/// The ceilings are Excel's: 409 points of row, 255 characters of column. Zero
/// is allowed, and it hides — which is not a special case here but exactly what
/// a zero in the file has always meant.
fn parse_size(text: &str, axis: Axis) -> Option<f64> {
    let ceiling = match axis {
        Axis::Rows => 409.0,
        Axis::Columns => 255.0,
    };
    let size: f64 = text.trim().parse().ok()?;
    (size.is_finite() && (0.0..=ceiling).contains(&size)).then_some(size)
}

/// A range as a user would name it: `A1:D9`, or `A1` for one cell.
fn range_label(range: CellRange) -> String {
    if range.start == range.end {
        return range.start.to_a1();
    }
    format!("{}:{}", range.start.to_a1(), range.end.to_a1())
}

/// A vertical rule between groups of toolbar controls.
fn separate(ui: &mut egui::Ui) {
    ui.add_space(3.0);
    ui.separator();
    ui.add_space(3.0);
}

/// A toolbar control that is a word rather than a glyph.
///
/// Given an edge, because the shared theme draws inactive buttons flat and a
/// flat word on a flat bar is prose. The glyph buttons can do without one —
/// they are already obviously controls — but "More…" without a box round it is
/// just the word "more".
fn word_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(label)
            .min_size(egui::vec2(0.0, icons::SIZE + 4.0))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(0xBC))),
    )
}

/// A hairline across the whole width, under the menu bar.
///
/// The menus and the toolbar are two different things — one is every command
/// there is, the other is the dozen used by the minute — and without a line
/// between them they read as one crowded block of chrome.
fn rule(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 5.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().hline(
            rect.x_range(),
            rect.center().y.round() + 0.5,
            egui::Stroke::new(1.0, egui::Color32::from_gray(0xDC)),
        );
    }
}

/// Draws `add` with the look of a form field rather than a toolbar button.
///
/// The shared theme paints inactive controls flat and borderless, which is
/// right for forty buttons and wrong for the three things on the row that are
/// *inputs*. A combo box with no edge is a word floating on the chrome, and
/// nothing about it says it can be opened; a number with no box around it does
/// not look like something you are allowed to change.
fn field<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let v = &mut ui.style_mut().visuals;
        let edge = egui::Stroke::new(1.0, egui::Color32::from_gray(0xBC));
        let lit = egui::Stroke::new(1.0, egui::Color32::from_gray(0x8C));
        for (widget, stroke) in [
            (&mut v.widgets.inactive, edge),
            (&mut v.widgets.hovered, lit),
            (&mut v.widgets.active, lit),
            (&mut v.widgets.open, lit),
        ] {
            widget.weak_bg_fill = egui::Color32::WHITE;
            widget.bg_fill = egui::Color32::WHITE;
            widget.bg_stroke = stroke;
        }
        add(ui)
    })
    .inner
}

/// The colours a colour menu offers, as Excel arranges them: greys along the
/// top, the standard hues below, and a lighter tint of each under that.
const PALETTE: [[u32; 10]; 3] = [
    [
        0x000000, 0x262626, 0x404040, 0x595959, 0x808080, 0xA6A6A6, 0xBFBFBF, 0xD9D9D9, 0xF2F2F2,
        0xFFFFFF,
    ],
    [
        0xC00000, 0xFF0000, 0xFFC000, 0xFFFF00, 0x92D050, 0x00B050, 0x00B0F0, 0x0070C0, 0x002060,
        0x7030A0,
    ],
    [
        0xE6B8B7, 0xFF9999, 0xFFE699, 0xFFFF99, 0xC6E0B4, 0xA9D08E, 0x9BC2E6, 0x9DC3E6, 0x8EA9DB,
        0xB4A7D6,
    ],
];

/// A colour menu. `None` means the way *out* of colour — "Automatic" for text,
/// "No Fill" for a background — which is a real answer and not the absence of
/// one, so it goes at the top where it can be found.
///
/// Returns `Some(choice)` only on the frame something is picked.
fn palette(ui: &mut egui::Ui, none: &str) -> Option<Option<[u8; 3]>> {
    let mut picked = None;
    if menu::item(ui, none, "").clicked() {
        picked = Some(None);
    }
    menu::sep(ui);
    for row in PALETTE {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for packed in row {
                let rgb = [(packed >> 16) as u8, (packed >> 8) as u8, packed as u8];
                if swatch(ui, rgb).clicked() {
                    picked = Some(Some(rgb));
                    ui.close();
                }
            }
        });
    }
    picked
}

/// One colour in the palette.
///
/// Outlined whatever it holds, because white and near-white are colours a
/// spreadsheet uses constantly and an unoutlined white swatch is a gap in the
/// grid rather than a choice.
fn swatch(ui: &mut egui::Ui, rgb: [u8; 3]) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let [r, g, b] = rgb;
        let hovered = response.hovered();
        let inner = if hovered { rect.shrink(2.0) } else { rect };
        ui.painter()
            .rect_filled(inner, 2.0, egui::Color32::from_rgb(r, g, b));
        ui.painter().rect_stroke(
            inner,
            2.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(0x99)),
            egui::StrokeKind::Inside,
        );
        if hovered {
            ui.painter().rect_stroke(
                rect,
                3.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(0x21, 0x73, 0x46)),
                egui::StrokeKind::Inside,
            );
        }
    }
    response
}

/// One sheet tab.
///
/// Drawn rather than assembled from a `selectable_label` because the active tab
/// is not a highlighted label: it is a tab that has come *forward*, joined to
/// the sheet below it and separated from its neighbours. That is the whole
/// visual grammar of a tab strip, and it is what tells you at a glance which of
/// fifteen sheets you are looking at.
fn tab(ui: &mut egui::Ui, name: &str, selected: bool, stripe: Option<[u8; 3]>) -> egui::Response {
    let font = egui::FontId::new(
        13.0,
        ui_kit::fonts::face(ui_kit::Family::Sans, selected, false),
    );
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), font, egui::Color32::PLACEHOLDER);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(galley.size().x + 22.0, 24.0),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = match (selected, hovered) {
            (true, _) => egui::Color32::WHITE,
            (false, true) => egui::Color32::from_gray(0xE6),
            (false, false) => egui::Color32::from_gray(0xDC),
        };
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            egui::CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            },
            fill,
        );
        if !selected {
            painter.vline(
                rect.right(),
                rect.y_range(),
                egui::Stroke::new(1.0, egui::Color32::from_gray(0xC4)),
            );
        }
        // The colour the workbook gave this tab, as the band along its foot —
        // full height behind the label when the tab is not the active one,
        // which is how Excel shows it.
        if let Some([r, g, b]) = stripe {
            let color = egui::Color32::from_rgb(r, g, b);
            let band =
                egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 3.0), rect.max);
            painter.rect_filled(band, 0.0, color);
            if !selected {
                painter.rect_filled(
                    rect.shrink2(egui::vec2(0.0, 1.0)),
                    0.0,
                    color.gamma_multiply(0.35),
                );
            }
        }
        let color = if selected {
            egui::Color32::from_rgb(0x21, 0x73, 0x46)
        } else {
            egui::Color32::from_gray(0x33)
        };
        let font = egui::FontId::new(
            13.0,
            ui_kit::fonts::face(ui_kit::Family::Sans, selected, false),
        );
        painter.text(
            rect.center() - egui::vec2(0.0, 1.0),
            egui::Align2::CENTER_CENTER,
            name,
            font,
            color,
        );
    }
    response.on_hover_text(name)
}

/// What the status bar reports about the selection, Excel's own set.
///
/// Nothing at all for a single cell holding one value: the number is already on
/// screen, and "Sum: 42" beside a cell reading 42 is noise.
fn summary_labels(summary: &grid::Summary) -> Vec<String> {
    if summary.count <= 1 {
        return Vec::new();
    }
    let mut out = vec![format!("Count {}", summary.count)];
    if summary.numeric > 0 {
        if let Some(average) = summary.average() {
            out.insert(0, format!("Sum {}", ss_model::format_general(summary.sum)));
            out.insert(0, format!("Average {}", ss_model::format_general(average)));
        }
        out.push(format!("Min {}", ss_model::format_general(summary.min)));
        out.push(format!("Max {}", ss_model::format_general(summary.max)));
    }
    out
}

/// The font families offered, which are the ones a spreadsheet actually uses.
const FONT_NAMES: &[&str] = &[
    "Calibri",
    "Arial",
    "Times New Roman",
    "Courier New",
    "Consolas",
    "Georgia",
    "Verdana",
    "Segoe UI",
];

impl DocumentApp for Calx {
    fn id(&self) -> AppId {
        CALX
    }

    fn document(&self) -> Option<(String, bool)> {
        Some((
            self.path
                .as_deref()
                .map(name_of)
                .unwrap_or_else(|| "Untitled".to_string()),
            self.edited,
        ))
    }

    fn close_requested(&mut self) -> bool {
        if !self.edited {
            return true;
        }
        self.pending = Some(Pending::Quit);
        false
    }

    fn overlay(&mut self, ctx: &egui::Context) {
        self.unsaved_prompt(ctx);
        self.dialogs(ctx);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        Calx::toolbar(self, ui);
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        self.status_bar(ui);
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // While the prompt is up it owns the keyboard: Ctrl+S behind a modal
        // asking about Ctrl+S is not a question anyone can answer.
        if self.pending.is_none() {
            self.file_keys(&ctx);
        }

        // A file dropped on the window opens it, which is how a workbook arrives
        // when it is already in front of you in a file manager.
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(path) = dropped.first().map(|f| PathBuf::from(f.path())) {
            self.guard(Pending::Open(path));
        }

        // The grid fills the panel. It is the only thing in it, which is the
        // point of the panel: nothing here has to work out how much room the
        // tabs below need, so nothing here can get that wrong.
        //
        // It draws while a dialog is up and takes no keys: a modal stops the
        // pointer by itself, but the grid reads key events straight out of the
        // input rather than waiting to be focused, so it has to be told.
        //
        // An open menu counts, and so does an open combo box. Menu rows answer
        // to their own letter, and a grid that also saw that letter would put
        // it in a cell — press Alt+I, then W to delete a row, and "w" lands in
        // the cursor cell as well.
        self.last_body = ui.available_size();
        self.grid.blocked =
            self.dialog.is_some() || self.pending.is_some() || egui::Popup::is_any_open(ui.ctx());
        let response = self.grid.show(ui, &mut self.doc.workbook);
        response.context_menu(|ui| self.context_menu(ui));

        for action in self.grid.take_actions() {
            self.act(ui, action);
        }
    }
}

/// The bold and italic buttons, which wear the formatting they apply.
fn bold_letter(text: &'static str, on: bool) -> icons::Letter<'static> {
    icons::Letter {
        text,
        bold: true,
        on,
        tip: "Bold (Ctrl+B)",
        ..icons::Letter::plain()
    }
}

fn italic_letter(text: &'static str, on: bool) -> icons::Letter<'static> {
    icons::Letter {
        text,
        italic: true,
        on,
        tip: "Italic (Ctrl+I)",
        ..icons::Letter::plain()
    }
}

/// Whether two ranges touch at all.
fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.row <= b.end.row
        && b.start.row <= a.end.row
        && a.start.col <= b.end.col
        && b.start.col <= a.end.col
}

/// The width a column would need for its widest entry, in Excel's own units.
///
/// Estimated from the glyph count rather than measured, for the same reason the
/// row fitter estimates: this runs where there is no font atlas to ask. It is
/// generous by design — a column a little too wide is untidy, and one a little
/// too narrow shows `####` where a number used to be.
fn fitted_width(book: &Workbook, sheet: &ss_model::Sheet, col: u32) -> Option<f64> {
    let mut widest = 0.0f64;
    for (at, cell) in sheet.cells.iter() {
        if at.col != col || cell.value.is_blank() {
            continue;
        }
        let style = sheet.style_at(at);
        let value = match cell.value {
            ss_model::CellValue::Number(n) => ss_model::FormatValue::Number(n),
            ss_model::CellValue::Bool(b) => ss_model::FormatValue::Bool(b),
            ss_model::CellValue::Error(e) => ss_model::FormatValue::Error(e),
            ss_model::CellValue::Text(id) => ss_model::FormatValue::Text(book.strings.resolve(id)),
            ss_model::CellValue::Blank => continue,
        };
        let text = book.styles.number_format(style).format(value).text;
        let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        // Widths are counted in digits of the default font, so a bigger font
        // needs proportionally more of them.
        let scale = book.styles.font(style).size / ss_model::style::DEFAULT_FONT_SIZE;
        widest = widest.max(longest as f64 * scale);
    }
    (widest > 0.0).then(|| (widest + 1.0).clamp(2.0, 120.0))
}

/// What the undo stack calls a formatting command.
fn format_label(command: &Format) -> &'static str {
    match command {
        Format::Bold => "Bold",
        Format::Italic => "Italic",
        Format::Underline => "Underline",
        Format::Strike => "Strikethrough",
        Format::Align(_) => "Align",
        Format::Vertical(_) => "Vertical align",
        Format::Wrap => "Wrap text",
        Format::Indent(_) => "Indent",
        Format::FontSize(_) => "Font size",
        Format::FontName(_) => "Font",
        Format::TextColor(_) => "Text colour",
        Format::Fill(_) => "Fill colour",
        Format::Border(_) => "Borders",
        Format::NumberFormat(_) => "Number format",
        Format::Whole(_) => "Format cells",
        Format::Clear => "Clear formatting",
    }
}

/// The formats the toolbar offers, which are Excel's own menu.
const NUMBER_FORMATS: &[(&str, &str)] = &[
    ("General", "General"),
    ("Number", "0.00"),
    ("Thousands", "#,##0.00"),
    ("Currency", "\"$\"#,##0.00"),
    ("Percent", "0.00%"),
    ("Scientific", "0.00E+00"),
    ("Short date", "mm-dd-yy"),
    ("Long date", "d-mmm-yy"),
    ("Time", "h:mm:ss AM/PM"),
    ("Text", "@"),
];

/// The menu label for a format code, or the code itself for one we did not
/// offer — a workbook's own custom formats have to be visible as *something*.
fn short_format_name(code: &str) -> &str {
    NUMBER_FORMATS
        .iter()
        .find(|(_, known)| *known == code)
        .map_or(code, |(label, _)| label)
}

/// A sheet name from a file name, without the extension.
///
/// Excel names the sheet after the file, and a workbook saved from it keeps
/// that name — so it is worth getting right rather than calling it "Sheet1".
fn sheet_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Sheet1");
    // Excel's limit, and the characters it refuses in a sheet name.
    let cleaned: String = stem
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .take(31)
        .collect();
    if cleaned.trim().is_empty() {
        "Sheet1".to_string()
    } else {
        cleaned
    }
}

/// True for a path this application should treat as delimited text.
fn is_delimited(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("csv") | Some("tsv") | Some("tab") | Some("txt")
    )
}

/// The binary formats Excel 97 to 2003 wrote. Read-only: see `ss_xls`.
fn is_legacy(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("xls") | Some("xlt")
    )
}

/// What to call a delimiter in a status line.
fn delimiter_name(byte: u8) -> &'static str {
    match byte {
        b'\t' => "tab-separated",
        b';' => "semicolon-separated",
        b'|' => "pipe-separated",
        _ => "comma-separated",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_can_be_written_is_held_by_nobody() {
        let path = std::env::temp_dir().join(format!("calx-lock-{}.xlsx", std::process::id()));
        std::fs::write(&path, b"not really a workbook").expect("writes");
        assert_eq!(locked_by(&path), None);
        // Nor is a file that does not exist yet — that is a Save As, not a lock.
        let missing = path.with_file_name("calx-nothing-here.xlsx");
        assert_eq!(locked_by(&missing), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_refused_save_says_what_to_do_about_it() {
        // A directory cannot be opened for writing, which is the same answer
        // Windows gives for a file another program is holding.
        let dir = std::env::temp_dir().join(format!("calx-refused-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a directory");
        let message = save_trouble(&dir);

        let name = name_of(&dir);
        assert!(message.contains(&name), "{message}");
        assert!(message.contains("open in"), "{message}");
        assert!(
            message.contains("Save As"),
            "the way out is on the box: {message}"
        );
        assert!(
            message.contains("still here"),
            "and it says the work is not lost: {message}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn closing_a_workbook_leaves_nothing_of_it_behind() {
        let mut app = Calx::new();
        app.path = Some(PathBuf::from(r"C:\books\budget.xlsx"));
        type_into(&mut app, "A1", "42");
        app.dialog = Some(Dialog::GoTo {
            text: String::new(),
        });
        assert!(app.edited && !app.undo.is_empty());

        app.close_document();

        assert_eq!(app.path, None, "the file has been let go of");
        assert_eq!(value_at(&app, "A1"), None, "and so has what was in it");
        assert!(!app.edited, "an empty workbook has nothing to save");
        assert!(
            app.undo.is_empty(),
            "an undo naming cells in the old workbook would write into the new one"
        );
        assert!(app.dialog.is_none(), "the boxes went with their subject");
        assert!(
            app.status.contains("budget.xlsx"),
            "the status names what was closed: {}",
            app.status
        );
    }

    #[test]
    fn closing_an_edited_workbook_asks_first() {
        let mut app = Calx::new();
        type_into(&mut app, "A1", "42");

        app.guard(Pending::Close);
        assert_eq!(app.pending, Some(Pending::Close), "the prompt is up");
        assert_eq!(
            value_at(&app, "A1"),
            Some(ss_model::CellValue::Number(42.0)),
            "and nothing has been thrown away while it is"
        );

        // Whereas a workbook with nothing in it to lose closes on the spot.
        let mut untouched = Calx::new();
        untouched.guard(Pending::Close);
        assert_eq!(untouched.pending, None);
        assert_eq!(untouched.status, "Closed");
    }

    /// A workbook with one protected sheet, and B1 unlocked in it.
    fn protected(allow: ss_model::Protection) -> Calx {
        let mut app = Calx::new();
        let book = &mut app.doc.workbook;
        let open = {
            let mut look = book.styles.look(ss_model::StyleId::DEFAULT);
            look.locked = false;
            book.styles.style_for(&look)
        };
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            CellRef::new(0, 1),
            ss_model::Cell {
                style: open,
                ..Default::default()
            },
        );
        sheet.protection = Some(allow);
        app
    }

    fn type_into(app: &mut Calx, at: &str, text: &str) {
        let at = CellRef::from_a1(at).expect("valid");
        let change = edit::input(&mut app.doc.workbook, 0, at, text);
        app.perform(change);
    }

    fn value_at(app: &Calx, at: &str) -> Option<ss_model::CellValue> {
        let at = CellRef::from_a1(at).expect("valid");
        app.doc.workbook.sheet(0)?.get(at).map(|c| c.value)
    }

    #[test]
    fn a_protected_sheet_takes_typing_only_where_it_is_unlocked() {
        let mut app = protected(ss_model::Protection::as_excel_protects());

        type_into(&mut app, "A1", "42");
        assert_eq!(value_at(&app, "A1"), None, "A1 is locked");
        assert!(app.status.contains("A1 is locked"), "{}", app.status);
        assert!(app.undo.is_empty(), "a refused edit is not an undo entry");

        type_into(&mut app, "B1", "42");
        assert_eq!(
            value_at(&app, "B1"),
            Some(ss_model::CellValue::Number(42.0)),
            "B1 was unlocked before the sheet was protected"
        );
    }

    #[test]
    fn an_unprotected_sheet_takes_typing_into_locked_cells() {
        // Every cell in a workbook is locked. Locking means nothing until the
        // sheet is protected, and a guard that forgot this would make a fresh
        // workbook read-only.
        let mut app = Calx::new();
        type_into(&mut app, "A1", "42");
        assert_eq!(
            value_at(&app, "A1"),
            Some(ss_model::CellValue::Number(42.0))
        );
    }

    #[test]
    fn what_a_protected_sheet_allows_is_what_it_allows() {
        let mut app = protected(ss_model::Protection {
            insert_rows: true,
            ..ss_model::Protection::as_excel_protects()
        });
        let rows = |app: &Calx| app.doc.workbook.sheet(0).expect("sheet 0").cells.len();

        app.status.clear();
        app.structural(Axis::Rows, false);
        assert_eq!(app.status, "", "inserting rows was allowed");
        app.structural(Axis::Columns, false);
        assert!(
            app.status.contains("protected sheet"),
            "inserting columns was not: {}",
            app.status
        );
        let _ = rows;
    }

    #[test]
    fn taking_protection_off_is_never_refused_by_the_protection() {
        let mut app = protected(ss_model::Protection::as_excel_protects());
        app.toggle_protection();
        assert!(app
            .doc
            .workbook
            .sheet(0)
            .expect("sheet 0")
            .protection
            .is_none());
        type_into(&mut app, "A1", "42");
        assert_eq!(
            value_at(&app, "A1"),
            Some(ss_model::CellValue::Number(42.0))
        );
    }

    #[test]
    fn a_password_nobody_can_check_is_a_sheet_nobody_can_unprotect() {
        let mut app = protected(ss_model::Protection {
            password: vec![("password".to_string(), "CC3D".to_string())],
            ..ss_model::Protection::as_excel_protects()
        });
        app.toggle_protection();
        assert!(
            app.doc
                .workbook
                .sheet(0)
                .expect("sheet 0")
                .protection
                .is_some(),
            "the sheet stays protected"
        );
        assert!(app.status.contains("password"), "{}", app.status);
    }

    #[test]
    fn a_note_is_written_signed_and_taken_off_again() {
        let mut app = Calx::new();
        let at = CellRef::from_a1("B2").expect("valid");
        app.set_note(at, "Ada", "check the ledger");

        let notes = &app.doc.workbook.sheets[0].comments;
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].at, at);
        assert_eq!(notes[0].author, "Ada");
        assert_eq!(
            notes[0].text,
            "Ada:
check the ledger",
            "the author's name goes into the body too, as Excel writes it"
        );
        assert_eq!(notes[0].body(), "check the ledger");

        // Editing replaces rather than adds.
        app.set_note(at, "Ada", "checked");
        assert_eq!(app.doc.workbook.sheets[0].comments.len(), 1);

        // And an empty note is no note.
        app.set_note(at, "Ada", "   ");
        assert!(app.doc.workbook.sheets[0].comments.is_empty());

        app.undo();
        assert_eq!(
            app.doc.workbook.sheets[0].comments.len(),
            1,
            "deleting a note is undoable"
        );
    }

    #[test]
    fn notes_are_kept_in_the_order_the_cells_come_in() {
        // The file lists them in reading order and so should the model: a
        // reader that renumbers author ids by position would otherwise write a
        // different file every time a note was added above another one.
        let mut app = Calx::new();
        for a1 in ["C3", "A1", "B2"] {
            let at = CellRef::from_a1(a1).expect("valid");
            app.set_note(at, "Ada", a1);
        }
        let order: Vec<String> = app.doc.workbook.sheets[0]
            .comments
            .iter()
            .map(|note| note.at.to_a1())
            .collect();
        assert_eq!(order, ["A1", "B2", "C3"]);
    }

    #[test]
    fn a_typed_size_is_taken_only_where_the_file_could_hold_it() {
        assert_eq!(parse_size("12.5", Axis::Columns), Some(12.5));
        assert_eq!(parse_size("  30 ", Axis::Rows), Some(30.0));
        // Zero is a size, and it is how the file spells "hidden".
        assert_eq!(parse_size("0", Axis::Rows), Some(0.0));
        assert_eq!(parse_size("-1", Axis::Rows), None);
        assert_eq!(parse_size("wide", Axis::Columns), None);
        assert_eq!(parse_size("", Axis::Columns), None);
        // Excel's own ceilings, and they differ by axis.
        assert_eq!(parse_size("409", Axis::Rows), Some(409.0));
        assert_eq!(parse_size("409", Axis::Columns), None);
        assert_eq!(parse_size("255", Axis::Columns), Some(255.0));
        assert_eq!(parse_size("inf", Axis::Rows), None);
    }
}
