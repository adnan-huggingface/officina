//! The window shell both apps boot into.

use eframe::egui;

use crate::AppId;

/// A document application hosted by the shell.
///
/// Deliberately minimal for now: the shell owns the window, the frame loop, and
/// the chrome, and hands the app a `Ui` for its document surface. As the ribbon,
/// command palette, and keybinding engine land, they attach here rather than in
/// each app.
pub trait DocumentApp {
    /// Identity of this app, used for the window title and config directory.
    fn id(&self) -> AppId;

    /// Draw one frame of the document surface.
    fn ui(&mut self, ui: &mut egui::Ui);
}

/// Boots a window for `app` and runs until the user closes it.
pub fn run(app: impl DocumentApp + 'static) -> eframe::Result<()> {
    let id = app.id();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(id.display)
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        id.slug,
        options,
        Box::new(move |_cc| Ok(Box::new(Host { app }))),
    )
}

struct Host<A: DocumentApp> {
    app: A,
}

impl<A: DocumentApp> eframe::App for Host<A> {
    // eframe 0.36 hands the app a `Ui` covering the whole window rather than a
    // `Context` to open panels on, so the host has nothing to set up here yet.
    // The ribbon and status bar will attach around this call.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}
