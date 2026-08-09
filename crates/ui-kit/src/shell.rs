//! The window shell both apps boot into.

use eframe::egui;

use crate::AppId;

/// A document application hosted by the shell.
///
/// The surface is split into three because a document window is three things:
/// commands above, the document itself, and the state of the document below.
/// The shell gives each its own panel, and that is not a stylistic choice —
/// laying them out by hand means subtracting the height of the other two from
/// the window, and getting that subtraction wrong hides the bottom one
/// off-screen with no scrollbar and no way to reach it. A panel cannot be
/// wrong: the centre is *what is left*, however much that is.
pub trait DocumentApp {
    /// Identity of this app, used for the window title and config directory.
    fn id(&self) -> AppId;

    /// Draw one frame of the document surface.
    fn ui(&mut self, ui: &mut egui::Ui);

    /// Commands, above the document. Laid out top-down; may be several rows.
    fn toolbar(&mut self, _ui: &mut egui::Ui) {}

    /// State, below the document. Laid out top-down; may be several rows.
    fn status(&mut self, _ui: &mut egui::Ui) {}

    /// Anything drawn over the whole window: modals, and nothing else.
    ///
    /// Separate from `ui` because a modal opened from inside a panel is
    /// clipped to that panel, which is not what a modal is.
    fn overlay(&mut self, _ctx: &egui::Context) {}

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
            // Modest, and *logical*: at 150% scaling a "1440 x 900" window is
            // 2160 x 1350 physical pixels, which is taller than the screen it
            // was meant to fit on — and a window taller than the screen hides
            // its own bottom edge, which is where a spreadsheet keeps its
            // sheet tabs.
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0])
            // A document window opens filling the screen, because a spreadsheet
            // that shows twelve columns is a spreadsheet you have to scroll to
            // read at all.
            .with_maximized(true),
        ..Default::default()
    };

    eframe::run_native(
        id.slug,
        options,
        Box::new(move |cc| {
            crate::fonts::install(&cc.egui_ctx);
            theme(&cc.egui_ctx);
            Ok(Box::new(Host {
                app,
                maximize_for: 4,
                title: id.display.to_string(),
            }))
        }),
    )
}

/// Height of the strip below the document: one row of tabs and one of state.
///
/// Fixed rather than measured. It never varies — it is two rows of the same
/// controls all day — and a panel that measures itself has a first frame where
/// it does not know its own size yet.
const STATUS_HEIGHT: f32 = 56.0;

/// The colours and metrics both apps share.
///
/// Light rather than following the system: these are documents, and a document
/// is paper. A dark chrome around a white page is a defensible design, but a
/// *grey* page with black text is not the workbook the user formatted, and the
/// whole point of reading styles.xml is to show them what they made.
pub fn theme(ctx: &egui::Context) {
    // Both themes are set to the same thing rather than only the current one:
    // egui keeps a style per theme and switches between them when the system
    // does, so styling one leaves the other as the default.
    ctx.all_styles_mut(paint_style);
    ctx.set_theme(egui::Theme::Light);
}

fn paint_style(style: &mut egui::Style) {
    style.visuals = egui::Visuals::light();

    let v = &mut style.visuals;
    v.panel_fill = egui::Color32::from_rgb(0xF3, 0xF3, 0xF3);
    v.window_fill = egui::Color32::from_rgb(0xFB, 0xFB, 0xFB);
    v.extreme_bg_color = egui::Color32::WHITE;
    v.faint_bg_color = egui::Color32::from_rgb(0xE9, 0xE9, 0xE9);
    // Excel's green, which is what the selection and the active tab are.
    let accent = egui::Color32::from_rgb(0x21, 0x73, 0x46);
    v.selection.bg_fill = accent.gamma_multiply(0.25);
    v.selection.stroke = egui::Stroke::new(1.0, accent);
    v.hyperlink_color = egui::Color32::from_rgb(0x05, 0x63, 0xC1);

    // Flat controls with a visible edge only where one is needed. A toolbar of
    // forty buttons each drawing its own raised frame is a wall of boxes.
    v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(0xE1, 0xEC, 0xE6);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0xC0));
    v.widgets.active.weak_bg_fill = egui::Color32::from_rgb(0xCB, 0xE1, 0xD4);
    v.widgets.open.weak_bg_fill = egui::Color32::from_rgb(0xE1, 0xEC, 0xE6);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0xCF));

    let r = egui::CornerRadius::same(3);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }

    style.spacing.button_padding = egui::vec2(6.0, 3.0);
    style.spacing.item_spacing = egui::vec2(5.0, 4.0);
    style.spacing.interact_size.y = 22.0;
}

struct Host<A: DocumentApp> {
    app: A,
    /// Frames left in which to insist the window fills the screen.
    ///
    /// `ViewportBuilder::with_maximized` is a *request* made before the window
    /// exists, and it is quietly dropped when an explicit inner size is given
    /// alongside it. Sending the command afterwards works — but not on the
    /// very first frame, before the window manager has finished placing the
    /// window, so it is sent for a few frames and then never again.
    maximize_for: u8,
    /// The last title sent to the window manager.
    ///
    /// Remembered because the title is pushed with a viewport command rather
    /// than returned, and sending the same one every frame would ask the window
    /// manager to relabel the window sixty times a second.
    title: String,
}

impl<A: DocumentApp> eframe::App for Host<A> {
    // eframe 0.36 hands the app a `Ui` covering the whole window rather than a
    // `Context` to open panels on, so the panels are opened *inside* that `Ui`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.maximize_for > 0 {
            self.maximize_for -= 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }

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

        self.app.overlay(&ctx);

        // The bottom panel is declared before the centre so that it keeps its
        // height when the window is short: panels are allotted space in the
        // order they are added, and the centre takes what is left. The status
        // of the document is never the thing to drop.
        let chrome = ui.visuals().panel_fill;
        egui::Panel::top("shell-toolbar")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(chrome)
                    .inner_margin(egui::Margin::symmetric(6, 4)),
            )
            .show(ui, |ui| self.app.toolbar(ui));

        egui::Panel::bottom("shell-status")
            .resizable(false)
            .exact_size(STATUS_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(chrome)
                    .inner_margin(egui::Margin::symmetric(6, 3)),
            )
            .show(ui, |ui| self.app.status(ui));

        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(egui::Color32::WHITE))
            .show(ui, |ui| self.app.ui(ui));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this whole arrangement exists to prevent: a document surface
    /// that takes the whole window and leaves the sheet tabs and the status
    /// line drawn off the bottom of the screen, unreachable and invisible.
    #[test]
    fn the_centre_leaves_room_for_the_panels_around_it() {
        let ctx = egui::Context::default();
        let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
        let input = egui::RawInput {
            screen_rect: Some(window),
            ..Default::default()
        };

        let mut centre = egui::Rect::NOTHING;
        let mut status = egui::Rect::NOTHING;
        let mut out = ctx.run_ui(input, |ui| {
            egui::Panel::top("t")
                .resizable(false)
                .show(ui, |ui| ui.label("toolbar"));
            egui::Panel::bottom("b").resizable(false).show(ui, |ui| {
                // The real thing: a scrolling tab strip and a status line with
                // a right-aligned zoom control.
                egui::ScrollArea::horizontal()
                    .id_salt("tabs")
                    .max_height(26.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for name in ["Sheet1", "Sheet2", "Sheet3"] {
                                let _ = ui.button(name);
                            }
                        });
                    });
                ui.horizontal(|ui| {
                    ui.small("Ready");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut zoom = 100.0;
                        ui.add(egui::Slider::new(&mut zoom, 25.0..=400.0).show_value(false));
                        ui.small("Sum 42");
                    });
                });
                status = ui.min_rect();
            });
            egui::CentralPanel::default().show(ui, |ui| {
                centre = ui.available_rect_before_wrap();
            });
        });
        out.textures_delta.clear();

        assert!(status.height() > 0.0, "the status panel drew nothing");
        assert!(
            centre.bottom() <= status.top() + 1.0,
            "the document surface ({centre:?}) overlaps the status panel ({status:?})"
        );
        assert!(centre.height() > 400.0, "the centre got squeezed out");
    }
}
