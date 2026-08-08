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

    /// What the open document is called, and whether it has unsaved changes.
    ///
    /// The shell turns this into the window title. `None` means no document.
    fn document(&self) -> Option<(String, bool)> {
        None
    }

    /// Called when the user asks to close the window.
    ///
    /// Returning `false` cancels the close, and the app is expected to be
    /// showing whatever it wants an answer to. The shell asks once per close
    /// request, so an app that returns `true` closes immediately.
    fn close_requested(&mut self) -> bool {
        true
    }
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
        Box::new(move |_cc| {
            Ok(Box::new(Host {
                app,
                title: id.display.to_string(),
            }))
        }),
    )
}

struct Host<A: DocumentApp> {
    app: A,
    /// The last title sent to the window manager.
    ///
    /// Remembered because the title is pushed with a viewport command rather
    /// than returned, and sending the same one every frame would ask the window
    /// manager to relabel the window sixty times a second.
    title: String,
}

impl<A: DocumentApp> eframe::App for Host<A> {
    // eframe 0.36 hands the app a `Ui` covering the whole window rather than a
    // `Context` to open panels on, so the host has little to set up here yet.
    // The ribbon and status bar will attach around this call.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested()) && !self.app.close_requested() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }

        let id = self.app.id();
        let title = match self.app.document() {
            // The dot is the convention every editor uses for "not saved", and
            // it goes first so it is visible in a truncated taskbar entry.
            Some((name, true)) => format!("• {name} — {}", id.display),
            Some((name, false)) => format!("{name} — {}", id.display),
            None => id.display.to_string(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        self.app.ui(ui);
    }
}
