//! Calx — spreadsheet.

#![forbid(unsafe_code)]
// No console window on Windows for release builds; keep it in debug so `dbg!` lands
// somewhere visible.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::io::BufRead;
use std::path::{Path, PathBuf};

use ss_formula::clip::{self, Clip};
use ss_formula::cond;
use ss_formula::edit::{self, Change, Patch};
use ss_model::style::Underline;
use ss_model::{Axis, CellRange, Color, Fill, HAlign, Shift, Workbook};
use ui_kit::{egui, paths, AppId, DocumentApp, CALX};

use calx::grid::{self, Action, BorderPreset, Editor, Format, GridView, Mode};

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
        }
    }

    fn open(&mut self, path: &Path) {
        if is_delimited(path) {
            self.open_delimited(path);
            return;
        }
        match ss_xlsx::XlsxDocument::open(path) {
            Ok(doc) => {
                self.doc = doc;
                let cells: usize = self.doc.workbook.sheets.iter().map(|s| s.cells.len()).sum();
                self.status = format!("{} sheet(s), {cells} cells", self.doc.workbook.sheets.len());
                self.path = Some(path.to_path_buf());
                self.reset();
            }
            Err(e) => self.status = format!("could not open {}: {e}", path.display()),
        }
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
            .add_filter("Spreadsheets", &["xlsx", "xlsm", "csv", "tsv", "txt"])
            .add_filter("Excel workbook", &["xlsx", "xlsm"])
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
            Action::Resized(before) => {
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

    /// The toolbar. Everything on it is also a keystroke; the buttons are for
    /// finding out that the keystroke exists.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let mut requested = None;
        let mut title_change = None;
        let mut file = None;
        let mut save = false;
        let mut save_as = false;
        ui.horizontal(|ui| {
            if ui.button("New").on_hover_text("Ctrl+N").clicked() {
                file = Some(Pending::New);
            }
            if ui.button("Open").on_hover_text("Ctrl+O").clicked() {
                file = Some(Pending::Browse);
            }
            save = ui
                .add_enabled(
                    self.edited || self.path.is_none(),
                    egui::Button::new("Save"),
                )
                .on_hover_text("Ctrl+S")
                .clicked();
            save_as = ui
                .button("Save as…")
                .on_hover_text("Ctrl+Shift+S")
                .clicked();
            ui.separator();
            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("↶"))
                .on_hover_text("Undo (Ctrl+Z)")
                .clicked()
            {
                requested = Some(Action::Undo);
            }
            if ui
                .add_enabled(!self.redo.is_empty(), egui::Button::new("↷"))
                .on_hover_text("Redo (Ctrl+Y)")
                .clicked()
            {
                requested = Some(Action::Redo);
            }
            ui.separator();
            for (label, hover, action) in [
                ("Insert row", "Ctrl++", Action::Insert(Axis::Rows)),
                ("Delete row", "Ctrl+-", Action::Delete(Axis::Rows)),
                ("Insert col", "", Action::Insert(Axis::Columns)),
                ("Delete col", "", Action::Delete(Axis::Columns)),
            ] {
                let button = ui.button(label);
                let button = if hover.is_empty() {
                    button
                } else {
                    button.on_hover_text(hover)
                };
                if button.clicked() {
                    requested = Some(action);
                }
            }
        });
        // The formatting row. Second line rather than the first because the
        // first is about the *document* and this is about the selection.
        ui.horizontal(|ui| {
            let look = self.cursor_look();
            for (label, hover, on, command) in [
                ("B", "Bold (Ctrl+B)", look.font.bold, Format::Bold),
                ("I", "Italic (Ctrl+I)", look.font.italic, Format::Italic),
                (
                    "U",
                    "Underline (Ctrl+U)",
                    !look.font.underline.is_none(),
                    Format::Underline,
                ),
                (
                    "S",
                    "Strikethrough (Ctrl+5)",
                    look.font.strike,
                    Format::Strike,
                ),
            ] {
                if ui
                    .selectable_label(on, label)
                    .on_hover_text(hover)
                    .clicked()
                {
                    requested = Some(Action::Format(command));
                }
            }
            ui.separator();
            for (label, hover, align) in [
                ("⏴", "Align left", HAlign::Left),
                ("⏷", "Centre", HAlign::Center),
                ("⏵", "Align right", HAlign::Right),
            ] {
                let on = look.alignment.horizontal == align;
                if ui
                    .selectable_label(on, label)
                    .on_hover_text(hover)
                    .clicked()
                {
                    requested = Some(Action::Format(Format::Align(align)));
                }
            }
            if ui
                .selectable_label(look.alignment.wrap, "⏎")
                .on_hover_text("Wrap text")
                .clicked()
            {
                requested = Some(Action::Format(Format::Wrap));
            }
            for (label, hover, by) in [("→", "Increase indent", 1), ("←", "Decrease indent", -1)]
            {
                if ui.button(label).on_hover_text(hover).clicked() {
                    requested = Some(Action::Format(Format::Indent(by)));
                }
            }
            ui.separator();

            let mut text_rgb = look
                .font
                .color
                .resolve(self.doc.workbook.styles.theme())
                .unwrap_or([0, 0, 0]);
            if ui
                .color_edit_button_srgb(&mut text_rgb)
                .on_hover_text("Text colour")
                .changed()
            {
                let [r, g, b] = text_rgb;
                requested = Some(Action::Format(Format::TextColor(Some(Color::rgb(r, g, b)))));
            }
            let mut fill_rgb = look
                .fill
                .shade(self.doc.workbook.styles.theme())
                .unwrap_or([255, 255, 255]);
            if ui
                .color_edit_button_srgb(&mut fill_rgb)
                .on_hover_text("Fill colour")
                .changed()
            {
                let [r, g, b] = fill_rgb;
                requested = Some(Action::Format(Format::Fill(Some(Color::rgb(r, g, b)))));
            }
            if ui.button("No fill").clicked() {
                requested = Some(Action::Format(Format::Fill(None)));
            }
            ui.separator();

            egui::ComboBox::from_id_salt("calx-borders")
                .selected_text("Borders")
                .width(90.0)
                .show_ui(ui, |ui| {
                    for (label, preset) in [
                        ("All", BorderPreset::All),
                        ("Outline", BorderPreset::Outline),
                        ("Thick outline", BorderPreset::Thick),
                        ("Bottom", BorderPreset::Bottom),
                        ("Top", BorderPreset::Top),
                        ("Left", BorderPreset::Left),
                        ("Right", BorderPreset::Right),
                        ("None", BorderPreset::None),
                    ] {
                        if ui.selectable_label(false, label).clicked() {
                            requested = Some(Action::Format(Format::Border(preset)));
                        }
                    }
                });

            let mut size = look.font.size;
            if ui
                .add(
                    egui::DragValue::new(&mut size)
                        .speed(0.5)
                        .range(1.0..=409.0),
                )
                .on_hover_text("Font size")
                .changed()
            {
                requested = Some(Action::Format(Format::FontSize(size)));
            }

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
            if ui
                .button("Clear")
                .on_hover_text("Clear formatting")
                .clicked()
            {
                requested = Some(Action::Format(Format::Clear));
            }
        });

        // The chart row appears only when a chart is selected, because that is
        // the only time it can do anything.
        if let Some(index) = self.grid.selected_chart {
            let sheet = self.grid.sheet_index;
            let existing = self
                .doc
                .workbook
                .sheet(sheet)
                .and_then(|s| s.charts.get(index))
                .map(|c| c.title.clone().unwrap_or_default());
            if let Some(existing) = existing {
                ui.horizontal(|ui| {
                    ui.label("Chart title");
                    if self.chart_title.is_none() {
                        self.chart_title = Some(existing.clone());
                    }
                    let buffer = self.chart_title.get_or_insert_with(String::new);
                    let response = ui.add(
                        egui::TextEdit::singleline(buffer)
                            .desired_width(220.0)
                            .hint_text("(none)"),
                    );
                    // On losing focus rather than on every keystroke: one undo
                    // entry per title, not one per letter.
                    if response.lost_focus() && *buffer != existing {
                        title_change = Some((index, buffer.clone()));
                    }
                });
            }
        } else {
            self.chart_title = None;
        }
        if let Some(action) = requested {
            let ui = &*ui;
            self.act(ui, action);
        }
        if let Some((chart, title)) = title_change {
            let change = edit::chart_title(self.grid.sheet_index, chart, &title);
            self.perform(change);
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

    /// The name box and the formula bar.
    fn formula_bar(&mut self, ui: &mut egui::Ui) {
        let cursor = self.grid.selection.cursor();
        let mut open_editor = false;
        let mut commit = false;

        ui.horizontal(|ui| {
            ui.add_sized(
                [90.0, 20.0],
                egui::Label::new(egui::RichText::new(cursor.to_a1()).monospace()),
            );
            ui.separator();

            let width = ui.available_width();
            match &mut self.grid.editor {
                Some(editing) => {
                    let font = egui::FontId::monospace(13.0);
                    let plain = ui.visuals().text_color();
                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                        let mut job = grid::editor::highlight(text.as_str(), font.clone(), plain);
                        job.wrap.max_width = wrap;
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    let response = ui.add_sized(
                        [width, 20.0],
                        egui::TextEdit::singleline(&mut editing.text)
                            .id(egui::Id::new("calx-formula-bar"))
                            .layouter(&mut layouter),
                    );
                    // Typing here rather than in the cell is still an edit, and
                    // the arrow keys have to stop committing once it starts.
                    if response.changed() {
                        editing.mode = Mode::Edit;
                    }
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                }
                None => {
                    let source =
                        grid::source_text(&self.doc.workbook, self.grid.sheet_index, cursor);
                    let response = ui.add_sized(
                        [width, 20.0],
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
    }
}

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

    fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.unsaved_prompt(&ctx);
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

        self.toolbar(ui);
        self.formula_bar(ui);
        ui.separator();

        // The tabs and status line sit below the grid, so the grid is given
        // what is left rather than all of it.
        let bottom = 48.0;
        let available = ui.available_size();
        let grid_size = egui::vec2(available.x, (available.y - bottom).max(0.0));
        let (_, grid_rect) = ui.allocate_space(grid_size);
        let mut grid_ui = ui.new_child(egui::UiBuilder::new().max_rect(grid_rect));
        self.grid.show(&mut grid_ui, &mut self.doc.workbook);

        for action in self.grid.take_actions() {
            self.act(ui, action);
        }

        ui.separator();
        ui.horizontal(|ui| {
            for index in 0..self.doc.workbook.sheets.len() {
                let (name, visible) = {
                    let sheet = &self.doc.workbook.sheets[index];
                    (sheet.name.clone(), sheet.kind.has_grid() && !sheet.hidden)
                };
                if !visible {
                    continue;
                }
                let selected = index == self.grid.sheet_index;
                if ui.selectable_label(selected, name).clicked() && !selected {
                    self.grid.editor = None;
                    self.grid.sheet_index = index;
                    self.grid.scroll = grid::Scroll::default();
                    self.grid.selection = grid::Selection::default();
                    self.grid.invalidate();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.small(&self.status);
            if self.edited {
                ui.small("• edited");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.small(format!("{:.0}%", self.grid.zoom * 100.0));
                if let Err(e) = &self.config_dir {
                    ui.colored_label(egui::Color32::RED, format!("config unavailable: {e}"));
                }
            });
        });
    }
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

/// What to call a delimiter in a status line.
fn delimiter_name(byte: u8) -> &'static str {
    match byte {
        b'\t' => "tab-separated",
        b';' => "semicolon-separated",
        b'|' => "pipe-separated",
        _ => "comma-separated",
    }
}
