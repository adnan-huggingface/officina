//! Calx — spreadsheet.

#![forbid(unsafe_code)]
// No console window on Windows for release builds; keep it in debug so `dbg!` lands
// somewhere visible.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use ui_kit::{egui, paths, AppId, DocumentApp, CALX};

fn main() -> ui_kit::eframe::Result<()> {
    ui_kit::run(Calx::new())
}

struct Calx {
    /// Resolved on startup so a permissions problem surfaces immediately rather
    /// than the first time the user changes a setting.
    config_dir: Result<std::path::PathBuf, String>,
}

impl Calx {
    fn new() -> Self {
        Self {
            config_dir: paths::config_dir(CALX).map_err(|e| e.to_string()),
        }
    }
}

impl DocumentApp for Calx {
    fn id(&self) -> AppId {
        CALX
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Calx");
        ui.label(concat!("version ", env!("CARGO_PKG_VERSION")));
        ui.separator();
        match &self.config_dir {
            Ok(p) => {
                ui.label(format!("config: {}", p.display()));
            }
            Err(e) => {
                ui.colored_label(egui::Color32::RED, format!("config unavailable: {e}"));
            }
        }
        ui.separator();
        ui.label("Scaffold only — the grid lands in chunk C8.");
    }
}
