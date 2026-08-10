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
use ss_model::style::Underline;
use ss_model::{Axis, CellRange, CellRef, Color, Fill, HAlign, Shift, Workbook};
use ui_kit::{egui, paths, AppId, DocumentApp, CALX};

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

/// One row of the sort dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SortLevel {
    /// `None` for "none", else an absolute sheet column.
    col: Option<u32>,
    descending: bool,
}

/// An action held up by the "you have unsaved changes" prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Close,
    New,
    Open(PathBuf),
    /// Open, but the file still has to be chosen — asked for *after* the
    /// current document is dealt with, so the user is not made to pick a file
    /// and only then told they might lose the one they have.
    Browse,
}

impl Calx {
    fn new() -> Self {
        Calx {
            config_dir: paths::config_dir(CALX).map_err(|e| e.to_string()),
            doc: blank(),
            path: None,
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
                self.reset();
                // The sheet the workbook was saved on, at the zoom it was saved
                // at. Opening every file on sheet one at 100% is opening a
                // different document than the one that was closed.
                let active = self.doc.workbook.active_sheet;
                self.grid.open_sheet(&self.doc.workbook, active);
            }
            Err(e) => self.status = format!("could not open {}: {e}", path.display()),
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
            Err(e) => self.status = format!("could not save {}: {e}", path.display()),
        }
    }

    fn new_document(&mut self) {
        self.doc = blank();
        self.path = None;
        self.status = "New workbook".to_string();
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
        if let Some(current) = self.path.as_ref().and_then(|p| p.parent()) {
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
                self.edited = false;
                self.status = format!("Saved {}", name_of(path));
            }
            // Deliberately not clearing `edited`: the document is still
            // unsaved, and the next Ctrl+S should try again.
            Err(e) => self.status = format!("could not save {}: {e}", path.display()),
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
        if let Some(current) = self.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(current);
        }
        if let Some(path) = dialog.pick_file() {
            self.guard(Pending::Open(path));
        }
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
            Pending::Close => {}
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
        let (save, save_as, open, new) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::S,
                ),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::O),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::N),
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
    }

    /// The prompt shown when something would discard unsaved changes.
    fn unsaved_prompt(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("calx-unsaved")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(match &self.path {
                Some(path) => format!("{} has changes that are not saved.", name_of(path)),
                None => "This workbook has never been saved.".to_string(),
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });

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
        if pending == Pending::Close {
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
        let undo = edit::apply(&mut self.doc.workbook, change);
        self.undo.push(undo);
        self.redo.clear();
        self.edited = true;
        self.recalculate();
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
            Action::Fill { from, to } => {
                let change = clip::fill(&mut self.doc.workbook, sheet, from, to);
                self.perform(change);
                self.grid.selection = grid::Selection::at(to.start);
                if let Some(s) = self.doc.workbook.sheet(sheet) {
                    self.grid.selection.extend_to(to.end, s);
                }
            }
            Action::Format(command) => self.format(command),
            Action::StepSheet(step) => self.step_sheet(step),
            Action::Merge(join) => self.merge(join),
            Action::Freeze(on) => self.freeze(on),
            Action::Visibility { axis, hide } => self.set_visibility(axis, hide),
            Action::AutoFit(axis) => self.autofit(axis),
            Action::AutoFitAt { axis, index } => self.autofit_span(axis, index, index),
            Action::PicturesMoved(before) => self.pictures_moved(sheet, before),
            Action::DeletePicture(index) => self.delete_picture(sheet, index),
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
    fn data_range(&self) -> Option<CellRange> {
        let sheet = self.doc.workbook.sheet(self.grid.sheet_index)?;
        let selected = self.grid.selection.active_range();
        if selected.rows() > 1 || selected.cols() > 1 {
            return Some(selected);
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

    /// Freezes the rows above and the columns left of the cursor.
    fn freeze(&mut self, on: bool) {
        let sheet = self.grid.sheet_index;
        let Some(current) = self.doc.workbook.sheet(sheet) else {
            return;
        };
        let mut geometry = Geometry::of(current);
        let cursor = self.grid.selection.cursor();
        geometry.frozen = on.then_some(cursor).filter(|at| at.row > 0 || at.col > 0);
        if geometry.frozen == current.frozen {
            return;
        }
        self.perform(Change::new(
            if on { "Freeze panes" } else { "Unfreeze panes" },
            vec![Patch::Geometry { sheet, geometry }],
        ));
        self.grid.invalidate();
    }

    /// Hides or shows the selected rows or columns.
    ///
    /// Hiding is a size of zero, which is exactly how the file stores it — a
    /// hidden row is `hidden="1"` with no height, and a height of zero is what
    /// that means on screen.
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
        self.status = if cut { "Cut" } else { "Copied" }.to_string();
    }

    fn paste(&mut self, text: String) {
        let sheet = self.grid.sheet_index;
        let target = self.grid.selection.active_range();

        // Our own clip carries formulas and styles; the system clipboard carries
        // only text. Prefer ours when the clipboard is still what we put there.
        let source = match &self.clip {
            Some(held) if text == self.clip_text => held.clone(),
            _ => Clip::from_tsv(&text, target.start),
        };

        let mut change = clip::paste(&mut self.doc.workbook, sheet, target, &source);
        // A cut is a paste that also empties where it came from, and the two
        // have to be one undo step or Ctrl-Z leaves the data in both places.
        if let Some((from_sheet, from_range)) = self.cut_from.take() {
            let cleared = edit::clear_contents(&self.doc.workbook, from_sheet, &[from_range]);
            change.patches.splice(0..0, cleared.patches);
            change.label = "Move".to_string();
        }
        self.perform(change);
    }

    /// The command surface, in three rows: the document, the selection's
    /// formatting, and what the cursor is sitting on.
    ///
    /// Three rows rather than one because they answer different questions and
    /// change at different rates. The first row is about the file and is the
    /// same all day; the second changes with every click; the third is the
    /// cell. A single row of forty controls makes the user re-scan all forty
    /// to find the one that moved.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let mut requested: Option<Action> = None;
        let mut file: Option<Pending> = None;
        let mut save = false;
        let mut save_as = false;
        let mut sort: Option<bool> = None;
        let mut filter: Option<FilterCommand> = None;
        let mut size: Option<Axis> = None;

        ui.horizontal(|ui| {
            if icons::button(ui, Icon::New, false, "New workbook (Ctrl+N)").clicked() {
                file = Some(Pending::New);
            }
            if icons::button(ui, Icon::Open, false, "Open (Ctrl+O)").clicked() {
                file = Some(Pending::Browse);
            }
            ui.add_enabled_ui(self.edited || self.path.is_none(), |ui| {
                save = icons::button(ui, Icon::Save, false, "Save (Ctrl+S)").clicked();
            });
            if ui
                .button("Save as…")
                .on_hover_text("Ctrl+Shift+S")
                .clicked()
            {
                save_as = true;
            }
            separate(ui);

            ui.add_enabled_ui(!self.undo.is_empty(), |ui| {
                let tip = match self.undo.last() {
                    Some(change) => format!("Undo {} (Ctrl+Z)", change.label),
                    None => "Undo (Ctrl+Z)".to_string(),
                };
                if icons::button(ui, Icon::Undo, false, &tip).clicked() {
                    requested = Some(Action::Undo);
                }
            });
            ui.add_enabled_ui(!self.redo.is_empty(), |ui| {
                let tip = match self.redo.last() {
                    Some(change) => format!("Redo {} (Ctrl+Y)", change.label),
                    None => "Redo (Ctrl+Y)".to_string(),
                };
                if icons::button(ui, Icon::Redo, false, &tip).clicked() {
                    requested = Some(Action::Redo);
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
                    requested = Some(action);
                }
            }
            separate(ui);

            // Excel's Home ▸ Cells ▸ Format, which is where anybody who has
            // not found the draggable boundary goes looking.
            ui.menu_button("Format", |ui| {
                for (label, action) in [
                    ("Fit column to contents", Action::AutoFit(Axis::Columns)),
                    ("Fit row to contents", Action::AutoFit(Axis::Rows)),
                    (
                        "Hide columns",
                        Action::Visibility {
                            axis: Axis::Columns,
                            hide: true,
                        },
                    ),
                    (
                        "Hide rows",
                        Action::Visibility {
                            axis: Axis::Rows,
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
                        "Unhide rows",
                        Action::Visibility {
                            axis: Axis::Rows,
                            hide: false,
                        },
                    ),
                ] {
                    if ui.button(label).clicked() {
                        requested = Some(action);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Column width…").clicked() {
                    size = Some(Axis::Columns);
                    ui.close();
                }
                if ui.button("Row height…").clicked() {
                    size = Some(Axis::Rows);
                    ui.close();
                }
            });
            separate(ui);

            let merged = self
                .doc
                .workbook
                .sheet(self.grid.sheet_index)
                .is_some_and(|s| s.merge_at(self.grid.selection.cursor()).is_some());
            if icons::button(ui, Icon::Merge, merged, "Merge cells").clicked() {
                requested = Some(Action::Merge(!merged));
            }
            let frozen = self
                .doc
                .workbook
                .sheet(self.grid.sheet_index)
                .is_some_and(|s| s.frozen.is_some());
            let tip = if frozen {
                "Unfreeze panes"
            } else {
                "Freeze rows above and columns left of the cursor"
            };
            if icons::button(ui, Icon::Freeze, frozen, tip).clicked() {
                requested = Some(Action::Freeze(!frozen));
            }
            if icons::button(ui, Icon::Sum, false, "Sum the cells above").clicked() {
                self.autosum();
            }
            separate(ui);

            // Sort & Filter. `sort` and `filter` are deferred rather than run
            // inside the closure because both need `self` mutably and the
            // toolbar has it borrowed for the layout.
            for (icon, tip, descending) in [
                (Icon::SortAscending, "Sort A to Z (smallest first)", false),
                (Icon::SortDescending, "Sort Z to A (largest first)", true),
            ] {
                if icons::button(ui, icon, false, tip).clicked() {
                    sort = Some(descending);
                }
            }
            if ui
                .button("Sort…")
                .on_hover_text("Sort by up to three columns")
                .clicked()
            {
                filter = Some(FilterCommand::SortDialog);
            }
            let filtering = self
                .doc
                .workbook
                .sheet(self.grid.sheet_index)
                .and_then(|s| s.filter.as_ref());
            let has_filter = filtering.is_some();
            let constrained = filtering.is_some_and(|f| f.is_filtering());
            if icons::button(
                ui,
                Icon::Filter,
                has_filter,
                if has_filter {
                    "Remove the filter arrows"
                } else {
                    "Filter (arrows on the heading row)"
                },
            )
            .clicked()
            {
                filter = Some(FilterCommand::Toggle);
            }
            ui.add_enabled_ui(constrained, |ui| {
                if ui
                    .button("Clear")
                    .on_hover_text("Show every row, keeping the arrows")
                    .on_disabled_hover_text("Nothing is being filtered out")
                    .clicked()
                {
                    filter = Some(FilterCommand::Clear);
                }
            });
            ui.add_enabled_ui(has_filter, |ui| {
                if ui
                    .button("Reapply")
                    .on_hover_text("Re-run the filter over the data as it is now")
                    .clicked()
                {
                    filter = Some(FilterCommand::Reapply);
                }
            });
        });

        // The formatting row.
        ui.horizontal(|ui| {
            let look = self.cursor_look();

            let mut name = look.font.name.clone();
            egui::ComboBox::from_id_salt("calx-font")
                .selected_text(&name)
                .width(130.0)
                .show_ui(ui, |ui| {
                    // The workbook's own font first, so a document set in a
                    // face nobody offers can still be got back to.
                    let mut offered: Vec<&str> = FONT_NAMES.to_vec();
                    if !offered.contains(&name.as_str()) {
                        offered.insert(0, name.as_str());
                    }
                    for choice in offered {
                        if ui.selectable_label(choice == name, choice).clicked() {
                            requested = Some(Action::Format(Format::FontName(choice.to_string())));
                        }
                    }
                });
            name.clear();

            let mut size = look.font.size;
            if ui
                .add(
                    egui::DragValue::new(&mut size)
                        .speed(0.5)
                        .range(1.0..=409.0)
                        .fixed_decimals(0),
                )
                .on_hover_text("Font size")
                .changed()
            {
                requested = Some(Action::Format(Format::FontSize(size)));
            }
            separate(ui);

            for (letter, command) in [
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
                    requested = Some(Action::Format(command));
                }
            }
            separate(ui);

            let theme = self.doc.workbook.styles.theme();
            let mut text_rgb = look.font.color.resolve(theme).unwrap_or([0, 0, 0]);
            ui.horizontal(|ui| {
                icons::button(ui, Icon::TextColor, false, "Text colour");
                if ui
                    .color_edit_button_srgb(&mut text_rgb)
                    .on_hover_text("Text colour")
                    .changed()
                {
                    let [r, g, b] = text_rgb;
                    requested = Some(Action::Format(Format::TextColor(Some(Color::rgb(r, g, b)))));
                }
            });
            let mut fill_rgb = look.fill.shade(theme).unwrap_or([255, 255, 255]);
            ui.horizontal(|ui| {
                icons::button(ui, Icon::FillColor, false, "Fill colour");
                if ui
                    .color_edit_button_srgb(&mut fill_rgb)
                    .on_hover_text("Fill colour")
                    .changed()
                {
                    let [r, g, b] = fill_rgb;
                    requested = Some(Action::Format(Format::Fill(Some(Color::rgb(r, g, b)))));
                }
            });
            if ui.small_button("×").on_hover_text("No fill").clicked() {
                requested = Some(Action::Format(Format::Fill(None)));
            }
            separate(ui);

            for (icon, align, tip) in [
                (Icon::AlignLeft, HAlign::Left, "Align left"),
                (Icon::AlignCenter, HAlign::Center, "Centre"),
                (Icon::AlignRight, HAlign::Right, "Align right"),
            ] {
                let on = look.alignment.horizontal == align;
                if icons::button(ui, icon, on, tip).clicked() {
                    requested = Some(Action::Format(Format::Align(align)));
                }
            }
            if icons::button(ui, Icon::Wrap, look.alignment.wrap, "Wrap text").clicked() {
                requested = Some(Action::Format(Format::Wrap));
            }
            for (icon, by, tip) in [
                (Icon::IndentLess, -1, "Decrease indent"),
                (Icon::IndentMore, 1, "Increase indent"),
            ] {
                if icons::button(ui, icon, false, tip).clicked() {
                    requested = Some(Action::Format(Format::Indent(by)));
                }
            }
            separate(ui);

            ui.menu_button("Borders", |ui| {
                for (label, preset) in [
                    ("All borders", BorderPreset::All),
                    ("Outline", BorderPreset::Outline),
                    ("Thick outline", BorderPreset::Thick),
                    ("Bottom", BorderPreset::Bottom),
                    ("Top", BorderPreset::Top),
                    ("Left", BorderPreset::Left),
                    ("Right", BorderPreset::Right),
                    ("No border", BorderPreset::None),
                ] {
                    if ui.button(label).clicked() {
                        requested = Some(Action::Format(Format::Border(preset)));
                        ui.close();
                    }
                }
            });

            egui::ComboBox::from_id_salt("calx-number-format")
                .selected_text(short_format_name(&look.number_format))
                .width(120.0)
                .show_ui(ui, |ui| {
                    for (label, code) in NUMBER_FORMATS {
                        if ui
                            .selectable_label(look.number_format == *code, *label)
                            .clicked()
                        {
                            requested =
                                Some(Action::Format(Format::NumberFormat(code.to_string())));
                        }
                    }
                });
            for (label, tip, code) in [
                ("$", "Currency", "\"$\"#,##0.00"),
                ("%", "Percent", "0.00%"),
                (",", "Thousands", "#,##0.00"),
            ] {
                if ui.small_button(label).on_hover_text(tip).clicked() {
                    requested = Some(Action::Format(Format::NumberFormat(code.to_string())));
                }
            }
            if ui
                .button("Clear")
                .on_hover_text("Clear formatting")
                .clicked()
            {
                requested = Some(Action::Format(Format::Clear));
            }
        });

        self.formula_bar(ui);
        self.chart_bar(ui);

        if let Some(action) = requested {
            let ui = &*ui;
            self.act(ui, action);
        }
        if let Some(axis) = size {
            self.open_size_dialog(axis);
        }
        if let Some(descending) = sort {
            self.sort_quick(descending);
        }
        match filter {
            Some(FilterCommand::SortDialog) => self.open_sort_dialog(),
            Some(FilterCommand::Toggle) => self.toggle_filter(),
            Some(FilterCommand::Clear) => self.clear_filter(),
            Some(FilterCommand::Reapply) => self.reapply_filter(),
            None => {}
        }
        if save {
            self.save();
        }
        if save_as {
            self.save_as();
        }
        if let Some(what) = file {
            self.guard(what);
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
                    let response = ui.add_sized(
                        [width, 22.0],
                        egui::Label::new(egui::RichText::new(source).monospace())
                            .truncate()
                            .sense(egui::Sense::click()),
                    );
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
                        if response.clicked() && !selected {
                            switch = Some(index);
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

        ui.horizontal(|ui| {
            ui.small(&self.status);
            if self.edited {
                ui.small("• edited");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The zoom control, which belongs here rather than in the
                // toolbar because it is about the view and not the document.
                let mut zoom = self.grid.zoom * 100.0;
                if ui
                    .add(
                        egui::Slider::new(&mut zoom, 25.0..=400.0)
                            .show_value(false)
                            .trailing_fill(false),
                    )
                    .on_hover_text("Zoom (Ctrl+scroll)")
                    .changed()
                {
                    self.grid.set_zoom(zoom / 100.0);
                }
                if ui
                    .small_button(format!("{:.0}%", self.grid.zoom * 100.0))
                    .on_hover_text("Back to 100%")
                    .clicked()
                {
                    self.grid.set_zoom(1.0);
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
                    ui.horizontal(|ui| {
                        accept |= ui.button("Rename").clicked();
                        keep &= !ui.button("Cancel").clicked();
                    });
                });
                if accept {
                    let name = text.clone();
                    self.rename_sheet(index, &name);
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
                    ui.horizontal(|ui| {
                        go = ui.button("OK").clicked();
                        keep &= !ui.button("Cancel").clicked();
                    });
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
                    ui.horizontal(|ui| {
                        accept |= ui.button("OK").clicked();
                        keep &= !ui.button("Cancel").clicked();
                    });
                });
                if accept {
                    if let Some(size) = parse_size(text, axis) {
                        self.resize_selection(axis, size);
                        keep = false;
                    }
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
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        go = ui.button("Sort").clicked();
                        keep &= !ui.button("Cancel").clicked();
                    });
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
                    ui.separator();
                    ui.horizontal(|ui| {
                        go = ui.button("OK").clicked();
                        clear = ui.button("Clear this column").clicked();
                        keep &= !ui.button("Cancel").clicked();
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
            requested = Some(Action::Paste(self.clip_text.clone()));
            ui.close();
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
        if ui.button("Clear formatting").clicked() {
            requested = Some(Action::Format(Format::Clear));
            ui.close();
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

/// A modal window: centred, not resizable, and blocking the rest of the app.
fn modal(ctx: &egui::Context, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Modal::new(egui::Id::new(("calx-modal", title))).show(ctx, |ui| {
        ui.set_min_width(320.0);
        ui.heading(title);
        ui.separator();
        add(ui);
    });
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
        self.pending = Some(Pending::Close);
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
        self.last_body = ui.available_size();
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
