//! Calx — spreadsheet.

#![forbid(unsafe_code)]
// No console window on Windows for release builds; keep it in debug so `dbg!` lands
// somewhere visible.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use ss_formula::clip::{self, Clip};
use ss_formula::edit::{self, Change, Patch};
use ss_model::{Axis, CellRange, Shift, Workbook};
use ui_kit::{egui, paths, AppId, DocumentApp, CALX};

use calx::grid::{self, Action, Editor, GridView, Mode};

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
    book: Workbook,
    path: Option<PathBuf>,
    /// The package the workbook came from, held so a later save can write back
    /// every part we did not model. Nothing reads it until C10.
    source: Option<ss_xlsx::XlsxDocument>,
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
}

impl Calx {
    fn new() -> Self {
        Calx {
            config_dir: paths::config_dir(CALX).map_err(|e| e.to_string()),
            book: Workbook::blank(),
            path: None,
            source: None,
            grid: GridView::default(),
            status: "Ready".to_string(),
            undo: Vec::new(),
            redo: Vec::new(),
            clip: None,
            clip_text: String::new(),
            cut_from: None,
            edited: false,
        }
    }

    fn open(&mut self, path: &Path) {
        match ss_xlsx::XlsxDocument::open(path) {
            Ok(doc) => {
                self.book = doc.workbook.clone();
                let cells: usize = self.book.sheets.iter().map(|s| s.cells.len()).sum();
                self.status = format!(
                    "{} — {} sheet(s), {cells} cells",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    self.book.sheets.len()
                );
                self.path = Some(path.to_path_buf());
                self.source = Some(doc);
                self.grid = GridView::default();
                self.undo.clear();
                self.redo.clear();
                self.edited = false;
            }
            Err(e) => self.status = format!("could not open {}: {e}", path.display()),
        }
    }

    /// Applies a change, records how to undo it, and recalculates.
    ///
    /// Doing anything new drops the redo stack — the history is a line, not a
    /// tree, which is what every editor a user has ever met does.
    fn perform(&mut self, change: Change) {
        if change.is_empty() {
            return;
        }
        let undo = edit::apply(&mut self.book, change);
        self.undo.push(undo);
        self.redo.clear();
        self.edited = true;
        self.recalculate();
    }

    fn recalculate(&mut self) {
        let result = ss_formula::recalculate(&mut self.book);
        if !result.circular.is_empty() {
            self.status = format!("circular reference in {} cells", result.circular.len());
        }
    }

    fn undo(&mut self) {
        if let Some(change) = self.undo.pop() {
            let label = change.label.clone();
            let redo = edit::apply(&mut self.book, change);
            self.redo.push(redo);
            self.recalculate();
            self.status = format!("Undo {label}");
        }
    }

    fn redo(&mut self) {
        if let Some(change) = self.redo.pop() {
            let label = change.label.clone();
            let undo = edit::apply(&mut self.book, change);
            self.undo.push(undo);
            self.recalculate();
            self.status = format!("Redo {label}");
        }
    }

    fn act(&mut self, ui: &egui::Ui, action: Action) {
        let sheet = self.grid.sheet_index;
        match action {
            Action::Commit { at, text, advance } => {
                let change = edit::input(&mut self.book, sheet, at, &text);
                self.perform(change);
                // Re-borrowed after the mutation, not held across it.
                if let (Some(direction), Some(s)) = (advance, self.book.sheet(sheet)) {
                    self.grid.selection.advance(direction, s);
                }
            }
            Action::Clear => {
                let ranges = self.grid.selection.ranges().to_vec();
                let change = edit::clear_contents(&self.book, sheet, &ranges);
                self.perform(change);
            }
            Action::Insert(axis) => self.structural(axis, false),
            Action::Delete(axis) => self.structural(axis, true),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Copy { cut } => self.copy(ui, cut),
            Action::Paste(text) => self.paste(text),
            Action::Fill { from, to } => {
                let change = clip::fill(&mut self.book, sheet, from, to);
                self.perform(change);
                self.grid.selection = grid::Selection::at(to.start);
                if let Some(s) = self.book.sheet(sheet) {
                    self.grid.selection.extend_to(to.end, s);
                }
            }
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
        let change = edit::structural(&self.book, sheet, shift);
        self.perform(change);
        self.grid.invalidate();
    }

    fn copy(&mut self, ui: &egui::Ui, cut: bool) {
        let sheet = self.grid.sheet_index;
        let range = self.grid.selection.active_range();
        let Some(taken) = clip::copy(&self.book, sheet, range) else {
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

        let mut change = clip::paste(&mut self.book, sheet, target, &source);
        // A cut is a paste that also empties where it came from, and the two
        // have to be one undo step or Ctrl-Z leaves the data in both places.
        if let Some((from_sheet, from_range)) = self.cut_from.take() {
            let cleared = edit::clear_contents(&self.book, from_sheet, &[from_range]);
            change.patches.splice(0..0, cleared.patches);
            change.label = "Move".to_string();
        }
        self.perform(change);
    }

    /// The toolbar. Everything on it is also a keystroke; the buttons are for
    /// finding out that the keystroke exists.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let mut requested = None;
        ui.horizontal(|ui| {
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
        if let Some(action) = requested {
            let ui = &*ui;
            self.act(ui, action);
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
                    let source = grid::source_text(&self.book, self.grid.sheet_index, cursor);
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
            let text = grid::source_text(&self.book, self.grid.sheet_index, cursor);
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

    fn ui(&mut self, ui: &mut egui::Ui) {
        // A file dropped on the window opens it. Until C10 has a file dialog this
        // is the way in, alongside the command line.
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        if let Some(path) = dropped.first().map(|f| PathBuf::from(f.path())) {
            self.open(&path);
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
        self.grid.show(&mut grid_ui, &mut self.book);

        for action in self.grid.take_actions() {
            self.act(ui, action);
        }

        ui.separator();
        ui.horizontal(|ui| {
            for index in 0..self.book.sheets.len() {
                let (name, visible) = {
                    let sheet = &self.book.sheets[index];
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
