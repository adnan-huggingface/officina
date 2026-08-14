//! Scriva — word processor.

#![forbid(unsafe_code)]
// No console window on Windows for release builds; keep it in debug so `dbg!`
// lands somewhere visible.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

fn main() -> ui_kit::eframe::Result<()> {
    // A path on the command line opens that document, which is what a double
    // click in the file manager becomes.
    let app = match std::env::args_os().nth(1) {
        Some(path) => scriva::app::Scriva::opening(std::path::PathBuf::from(path)),
        None => scriva::app::Scriva::new(),
    };
    ui_kit::run(app)
}
