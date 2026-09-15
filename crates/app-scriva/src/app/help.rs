//! The Help menu's boxes: every key the application answers to, generated
//! from the command table so that it cannot drift; the user guide, opened
//! beside the executable; and About.

use super::*;
use crate::commands::TABLE;

impl Scriva {
    pub(super) fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        egui::Modal::new(egui::Id::new("scriva-shortcuts"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(380.0);
                    ui.label(
                        egui::RichText::new("Keyboard Shortcuts").font(dialog::heading_font(16.0)),
                    );
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .show(ui, |ui| {
                            egui::Grid::new("scriva-shortcuts-grid")
                                .num_columns(2)
                                .spacing([24.0, 4.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    for entry in
                                        TABLE.iter().filter(|entry| !entry.shown.is_empty())
                                    {
                                        ui.label(
                                            egui::RichText::new(entry.shown)
                                                .family(egui::FontFamily::Monospace)
                                                .size(ui_kit::theme::TEXT_SMALL),
                                        );
                                        ui.label(entry.name);
                                        ui.end_row();
                                    }
                                });
                        });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "F6 moves the keyboard round the window; Esc brings it back.",
                        )
                        .size(ui_kit::theme::TEXT_SMALL)
                        .color(ui_kit::theme::INK_SOFT),
                    );
                    if dialog::row(ui, |ui| dialog::button(ui, "Close", true).clicked()) {
                        close = true;
                    }
                    close |= dialog::answered(ui).is_some();
                });
            });
        if close {
            self.shortcuts_up = false;
        }
    }

    pub(super) fn about_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        egui::Modal::new(egui::Id::new("scriva-about"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(360.0);
                    ui.label(
                        egui::RichText::new(format!("Scriva {}", env!("CARGO_PKG_VERSION")))
                            .font(dialog::heading_font(16.0)),
                    );
                    ui.add_space(8.0);
                    dialog::paragraph(
                        ui,
                        "A word processor that keeps your document as it was.\n\
                         \n\
                         Licensed under the MIT licence or the Apache licence, version 2.0, at your option.\n\
                         \n\
                         Fonts come from your system; nothing is bundled. A face a document asks for that is not installed is shown in a stand-in, and the status bar says so.",
                    );
                    if dialog::row(ui, |ui| dialog::button(ui, "Close", true).clicked()) {
                        close = true;
                    }
                    close |= dialog::answered(ui).is_some();
                });
            });
        if close {
            self.about_up = false;
        }
    }

    /// Opens `GUIDE.md` beside the executable with the system's opener, or
    /// says where it is when it is not there to open.
    pub(super) fn open_user_guide(&mut self) {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("GUIDE.md")))
            .filter(|path| path.is_file());
        match beside {
            Some(path) => {
                if let Err(why) = open_with_system(&path) {
                    self.message = Some((
                        "Cannot open the guide".to_owned(),
                        format!("The guide is at {}.\n\n{why}", path.display()),
                    ));
                }
            }
            None => {
                self.message = Some((
                    "The guide is not beside the program".to_owned(),
                    "GUIDE.md is kept beside the executable, and is in the source tree at the top. \
                     Put a copy next to the program to open it from here."
                        .to_owned(),
                ));
            }
        }
    }
}

/// The desktop's own opener for a file, by platform.
fn open_with_system(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|why| format!("The system's opener could not be started: {why}"))
}
