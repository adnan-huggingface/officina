//! Calx — spreadsheet.

#![forbid(unsafe_code)]
// No console window on Windows for release builds; keep it in debug so `dbg!` lands
// somewhere visible.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use ss_model::{CellRef, CellValue, Workbook};
use ui_kit::{egui, paths, AppId, DocumentApp, CALX};

use calx::grid::{self, GridView};

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
            }
            Err(e) => self.status = format!("could not open {}: {e}", path.display()),
        }
    }

    /// What the formula bar shows: the formula if there is one, else the value.
    fn cell_source(&self, at: CellRef) -> String {
        let Some(sheet) = self.book.sheet(self.grid.sheet_index) else {
            return String::new();
        };
        if let Some(formula) = sheet.formula_at(at) {
            if !formula.text.is_empty() {
                return format!("={}", formula.text);
            }
        }
        match sheet.get(at).map(|c| c.value) {
            Some(CellValue::Text(id)) => self.book.strings.resolve(id).to_string(),
            Some(CellValue::Number(n)) => ss_model::format_general(n),
            Some(CellValue::Bool(b)) => if b { "TRUE" } else { "FALSE" }.to_string(),
            Some(CellValue::Error(e)) => e.as_str().to_string(),
            _ => String::new(),
        }
    }
}

impl DocumentApp for Calx {
    fn id(&self) -> AppId {
        CALX
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        // A file dropped on the window opens it. Until C9 has a file dialog this
        // is the way in, alongside the command line.
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        if let Some(path) = dropped.first().map(|f| PathBuf::from(f.path())) {
            self.open(&path);
        }

        let cursor = self.grid.selection.cursor();
        ui.horizontal(|ui| {
            ui.add_sized(
                [90.0, 20.0],
                egui::Label::new(egui::RichText::new(cursor.to_a1()).monospace()),
            );
            ui.separator();
            // Read-only for now; the editor and its reference highlighting are C9.
            let source = self.cell_source(cursor);
            ui.add_sized(
                [ui.available_width(), 20.0],
                egui::Label::new(egui::RichText::new(source).monospace()).truncate(),
            );
        });
        ui.separator();

        // The tabs and status line sit below the grid, so the grid is given
        // what is left rather than all of it.
        let bottom = 48.0;
        let available = ui.available_size();
        let grid_size = egui::vec2(available.x, (available.y - bottom).max(0.0));
        let (_, grid_rect) = ui.allocate_space(grid_size);
        let mut grid_ui = ui.new_child(egui::UiBuilder::new().max_rect(grid_rect));
        self.grid.show(&mut grid_ui, &mut self.book);

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
                    self.grid.sheet_index = index;
                    self.grid.scroll = grid::Scroll::default();
                    self.grid.selection = grid::Selection::default();
                    self.grid.invalidate();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.small(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.small(format!("{:.0}%", self.grid.zoom * 100.0));
                if let Err(e) = &self.config_dir {
                    ui.colored_label(egui::Color32::RED, format!("config unavailable: {e}"));
                }
            });
        });
    }
}
