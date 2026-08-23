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
    let remembered = Placement::load(id);
    let placement = remembered.unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: {
            let mut viewport = egui::ViewportBuilder::default()
                .with_title(id.display)
                .with_icon(std::sync::Arc::new(crate::brand::icon(id)))
                .with_inner_size(placement.size)
                .with_min_inner_size([640.0, 400.0]);
            // Only where the window was left, and only when it was left
            // somewhere: a window with no remembered position is placed by the
            // window manager, which knows about the monitors and the taskbar.
            if let Some(pos) = placement.pos {
                viewport = viewport.with_position(pos);
            }
            viewport
        },
        // Nothing here asks for a maximized window, and that is deliberate.
        // Alongside an explicit inner size, `with_maximized(true)` gives a
        // window that is *marked* maximized and *sized* as asked: 1280 by 800
        // on a screen half as big again. Which would merely be a smaller
        // window than intended, except that the flag is already set, so the
        // command to maximize it afterwards is a no-op — winit sees nothing to
        // change — and the window stays small with its resize border, and the
        // desktop behind it, showing down the side of the page. The shell asks
        // after the window exists instead.
        ..Default::default()
    };

    eframe::run_native(
        id.slug,
        options,
        Box::new(move |cc| {
            crate::fonts::install(&cc.egui_ctx);
            theme(&cc.egui_ctx);
            // Windows: a window dragged between monitors of different scales
            // is resized mid-drag until the drag jams. The guard defers the
            // scale change until the user lets go. See dpi-guard's own story.
            dpi_guard::install(cc);
            Ok(Box::new(Host {
                app,
                // A document window fills the screen the *first* time it opens,
                // because a spreadsheet showing twelve columns is one you have
                // to scroll to read at all. After that it opens how it was
                // left: a window that reclaims the whole screen every morning
                // is not being helpful, it is overruling a decision the user
                // already made.
                maximize_for: if placement.maximized { 60 } else { 0 },
                id,
                placement,
                title: id.display.to_string(),
            }))
        }),
    )
}

/// Where the window was when it was last closed.
///
/// Kept by the shell because eframe's own window persistence is part of a
/// feature this workspace does not build, and it is four numbers in a file.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    maximized: bool,
    /// The size to open at, and to return to when un-maximized — never the
    /// size of a maximized window, which is the screen's and not a choice
    /// anybody made.
    size: [f32; 2],
    pos: Option<[f32; 2]>,
}

impl Default for Placement {
    /// A first run: modest, *logical* pixels, and maximized.
    ///
    /// At 150% scaling a "1440 x 900" window is 2160 x 1350 physical pixels,
    /// which is taller than the screen it was meant to fit on — and a window
    /// taller than the screen hides its own bottom edge, which is where a
    /// spreadsheet keeps its sheet tabs. This is the size the window returns
    /// to when the user un-maximizes it.
    fn default() -> Self {
        Placement {
            maximized: true,
            size: [1280.0, 800.0],
            pos: None,
        }
    }
}

impl Placement {
    fn file(app: AppId) -> Option<std::path::PathBuf> {
        crate::paths::config_dir_path(app)
            .ok()
            .map(|dir| dir.join("window"))
    }

    fn load(app: AppId) -> Option<Self> {
        Self::parse(&std::fs::read_to_string(Self::file(app)?).ok()?)
    }

    /// Best-effort, and silent about it: a window that will not remember where
    /// it was is a small annoyance, and a dialog about it on startup is a
    /// larger one.
    fn store(&self, app: AppId) {
        let Some(path) = Self::file(app) else { return };
        if crate::paths::config_dir(app).is_err() {
            return;
        }
        let _ = std::fs::write(path, self.text());
    }

    fn text(&self) -> String {
        let mut out = format!(
            "maximized {}\nsize {} {}\n",
            u8::from(self.maximized),
            self.size[0],
            self.size[1]
        );
        if let Some([x, y]) = self.pos {
            out.push_str(&format!("pos {x} {y}\n"));
        }
        out
    }

    /// Anything unreadable is treated as nothing remembered, which lands on the
    /// defaults. A half-written file after a power cut should not be able to
    /// open the window somewhere it cannot be seen.
    fn parse(text: &str) -> Option<Self> {
        let mut placement = Placement {
            maximized: false,
            ..Placement::default()
        };
        let mut sized = false;
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let (key, a, b) = (words.next()?, words.next(), words.next());
            let pair = || Some([a?.parse::<f32>().ok()?, b?.parse::<f32>().ok()?]);
            match key {
                "maximized" => placement.maximized = a? == "1",
                "size" => {
                    let size = pair()?;
                    // A window narrower than the minimum, or wider than any
                    // real screen, is a corrupt file rather than a preference.
                    if !(640.0..=32_768.0).contains(&size[0])
                        || !(400.0..=32_768.0).contains(&size[1])
                    {
                        return None;
                    }
                    placement.size = size;
                    sized = true;
                }
                "pos" => placement.pos = Some(pair()?),
                _ => {}
            }
        }
        sized.then_some(placement)
    }

    /// The window as it is now, keeping the un-maximized size when it is
    /// maximized: that is the size it will be restored to, and the screen's
    /// size is not a preference.
    fn of(ctx: &egui::Context, restore: Placement) -> Self {
        ctx.input(|i| {
            let view = i.viewport();
            let maximized = view.maximized.unwrap_or(false);
            Placement {
                maximized,
                size: match (maximized, view.inner_rect) {
                    (false, Some(rect)) if rect.width() >= 640.0 && rect.height() >= 400.0 => {
                        [rect.width(), rect.height()]
                    }
                    _ => restore.size,
                },
                pos: match (maximized, view.outer_rect) {
                    (false, Some(rect)) => Some([rect.min.x, rect.min.y]),
                    _ => restore.pos,
                },
            }
        })
    }
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
    // The suite's accent green, which is what the selection and the active tab are.
    let accent = egui::Color32::from_rgb(0x1E, 0x6F, 0x5C);
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
    /// first frames, before the window manager has finished placing the
    /// window, and a fixed handful of frames was too few: the window opened at
    /// its fallback size instead, which is where the black edge came from,
    /// because an unmaximized window has a resize border and eframe clears
    /// what it does not paint to near-black.
    ///
    /// So it asks until the window agrees that it is maximized, and gives up
    /// after about a second in case it never will — a window that opened
    /// smaller than intended is a shame, and one that fights the user for a
    /// minute over the size they chose is a fault.
    maximize_for: u8,
    /// Which app's config directory the window geometry belongs in.
    id: AppId,
    /// Where the window is, kept up to date so that it can be written down
    /// when the window closes — and standing in, while the window is
    /// maximized, for the size and place it will be restored to.
    placement: Placement,
    /// The last title sent to the window manager.
    ///
    /// Remembered because the title is pushed with a viewport command rather
    /// than returned, and sending the same one every frame would ask the window
    /// manager to relabel the window sixty times a second.
    title: String,
}

impl<A: DocumentApp> eframe::App for Host<A> {
    /// What the window holds before anything is drawn over it.
    ///
    /// eframe clears to a near-black translucent grey by default, which suits
    /// a demo floating over a desktop and not a document. Anything the panels
    /// do not reach shows it: the resize border of a window that is not
    /// maximized, and the moment between a resize and the frame that answers
    /// it, both read as a black bar down the side of the page. Cleared to the
    /// chrome colour instead, so the worst case is a seam nobody can see.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    // eframe 0.36 hands the app a `Ui` covering the whole window rather than a
    // `Context` to open panels on, so the panels are opened *inside* that `Ui`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.maximize_for > 0 {
            self.maximize_for -= 1;
            if ctx.input(|i| i.viewport().maximized) == Some(true) {
                self.maximize_for = 0;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                // Frames only arrive while something asks for them, and until
                // the window is placed nothing else is asking.
                ctx.request_repaint();
            }
        }

        // Watched every frame rather than read at the end: by the time the
        // window is closing it may already have been unmapped, and a window
        // that is gone reports nothing about where it was.
        if self.maximize_for == 0 {
            self.placement = Placement::of(&ctx, self.placement);
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            if self.app.close_requested() {
                self.placement.store(self.id);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
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

    #[test]
    fn a_window_is_written_down_and_read_back_the_same() {
        let left = Placement {
            maximized: false,
            size: [1024.0, 768.0],
            pos: Some([120.0, -8.0]),
        };
        assert_eq!(Placement::parse(&left.text()), Some(left));

        let full = Placement {
            maximized: true,
            size: [1280.0, 800.0],
            pos: None,
        };
        assert_eq!(Placement::parse(&full.text()), Some(full));
    }

    #[test]
    fn a_damaged_file_is_no_memory_rather_than_a_window_nobody_can_see() {
        // Anything unparseable lands on the defaults, which are a window the
        // window manager places. The sizes are the ones that would open a
        // window too small to use or larger than any screen.
        for text in [
            "",
            "maximized 1\n",
            "size 40 30\n",
            "size 99999 99999\n",
            "size wide tall\n",
            "size 1024\n",
        ] {
            assert_eq!(Placement::parse(text), None, "{text:?} should not parse");
        }
    }

    #[test]
    fn the_size_kept_while_maximized_is_the_one_to_come_back_to() {
        // The screen's size is not a preference. A window maximized at close
        // has to reopen maximized *and* remember what un-maximizing means.
        let ctx = egui::Context::default();
        let restore = Placement {
            maximized: false,
            size: [1024.0, 768.0],
            pos: Some([64.0, 64.0]),
        };
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1920.0, 1080.0),
                )),
                ..Default::default()
            },
            |_ui| {},
        );
        out.textures_delta.clear();
        // No window manager here, so the viewport reports nothing about being
        // maximized: the answer must still be the size handed in, never a
        // guess made from the screen.
        let now = Placement::of(&ctx, restore);
        assert_eq!(now.size, restore.size);
        assert_eq!(now.pos, restore.pos);
    }

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
