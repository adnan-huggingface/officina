//! Driving an application by keystroke, without a window.
//!
//! ADR 0002 records what a person at a keyboard finds that a green suite does
//! not: a crash and two silent losses in one afternoon, every one of them in
//! the seam between a key and the model. The afternoons since have been spent
//! on a hidden display, with a screenshot read as a picture after every key —
//! and the bug found that way was then pinned by a test that ran the same
//! frames in eighty milliseconds, once somebody had written the frame helper
//! by hand, for the third time, in the third place.
//!
//! This is that helper, once. A [`Driver`] runs the frame the window runs —
//! [`crate::shell::frame`], dialogs then menus then the document, in that
//! order and no other — with the events a keyboard would have put into it: a
//! key with its modifiers, the text a printable key also sends, a menu opened
//! by its Alt letter and an item chosen by its own. What the application does
//! with them is read back from the application, which is the point: a test
//! asks the model where the caret is, rather than a person asking a picture.
//! The display is still needed for two things — the file choosers, which are
//! another program's windows, and how a page *looks* — and for nothing else.
//!
//! **Pace is part of the input.** [`Driver::press`] is one frame, and the next
//! call is the next frame, which is faster than any hand types: a real window
//! paints a frame for the key's release and more for a caret's blink. A
//! sequence that fails here and not on the window is not therefore a false
//! alarm — the Tab race in Calx's grid failed at zero and one idle frames and
//! passed at two, and a quick typist gets the first two. Test a sequence at
//! every pace that could matter, with [`Driver::settle`] between the keys,
//! and believe the window only for the slowest.

use std::cell::Cell;

use eframe::egui;

use crate::shell::{self, DocumentApp};

mod painted;
pub use painted::{Painted, PaintedRect, PaintedText};

/// The window a driven application is given. Wide enough for every toolbar
/// to fit without folding and tall enough for a page, since a frame that
/// hides a control is a frame in which its shortcut cannot be tested.
pub const WINDOW: egui::Vec2 = egui::vec2(1600.0, 1000.0);

/// A window without a screen: the context, the fonts and the theme the shell
/// would give the application, and a way to run frames through it.
pub struct Driver {
    ctx: egui::Context,
    window: Cell<egui::Vec2>,
    /// The size the window grows to, and how many frames it has still to be
    /// drawn at its opening size first.
    growing: Cell<Option<(egui::Vec2, u32)>>,
    /// The time the next frame is drawn at, in seconds.
    clock: Cell<f64>,
    /// The pointer shape the last frame asked for.
    cursor: Cell<egui::CursorIcon>,
}

/// What a driver's frame runs.
///
/// An application runs in the shell's frame — dialogs, toolbar, status bar,
/// document — as the window runs it. A widget with tests of its own runs
/// bare, filling the window, by implementing this itself: its tests put
/// the pointer on the widget's own coordinates, and a toolbar above it
/// would move every one of them.
pub trait Driven {
    fn drive(&mut self, ui: &mut egui::Ui);
}

impl<A: DocumentApp> Driven for A {
    fn drive(&mut self, ui: &mut egui::Ui) {
        shell::frame(self, ui);
    }
}

impl Default for Driver {
    fn default() -> Self {
        Self::new()
    }
}

/// How long a frame lasts: a sixtieth of a second, the pace egui assumes
/// when nobody says otherwise, and the step its animations take per frame.
pub const FRAME: f64 = 1.0 / 60.0;

/// How many frames an [`Driver::opening`] window is drawn at the shell's
/// first size before it is maximized. The real window takes a few frames to
/// be placed before the maximize can land; three is enough to be seen.
pub const OPENING_FRAMES: u32 = 3;

impl Driver {
    /// A fresh context, with no system font loaded: the layout in a test is
    /// measured in whatever egui ships, which is the same on every machine,
    /// where a machine's own fonts are not.
    pub fn new() -> Driver {
        Driver::with_faces(&[])
    }

    /// The same, with the generic face of each shape given here set in
    /// place of egui's own — see [`crate::fonts::register_with`].
    pub fn with_faces(faces: &[(crate::fonts::Family, Vec<u8>)]) -> Driver {
        crate::headless::enter();
        let ctx = egui::Context::default();
        crate::fonts::register_with(&ctx, &[], faces);
        shell::theme(&ctx);
        Driver {
            ctx,
            window: Cell::new(WINDOW),
            growing: Cell::new(None),
            clock: Cell::new(0.0),
            cursor: Cell::new(egui::CursorIcon::Default),
        }
    }

    /// A driver whose sans-serif type is Hack, the monospaced face egui
    /// carries: taller in its descent than egui's proportional face and a
    /// good deal wider, so code that silently assumed egui's default metrics
    /// fails in it. Free to carry, being egui's own.
    pub fn in_hack() -> Driver {
        let hack = egui::FontDefinitions::default()
            .font_data
            .get("Hack")
            .map(|data| data.font.to_vec())
            .expect("egui carries Hack");
        Driver::with_faces(&[(crate::fonts::Family::Sans, hack)])
    }

    /// The same, in a window of another size — for what a toolbar does when
    /// it does not fit.
    pub fn sized(window: egui::Vec2) -> Driver {
        let driver = Driver::new();
        driver.window.set(window);
        driver
    }

    /// A window that opens the way the shell's does on a first run: at
    /// [`shell::FIRST_SIZE`] for [`OPENING_FRAMES`] frames, and then at the
    /// full [`WINDOW`], as the maximize lands. A fit or a layout taken on the
    /// first frames and never again is seen here and nowhere else.
    pub fn opening() -> Driver {
        let driver = Driver::sized(shell::FIRST_SIZE);
        driver.growing.set(Some((WINDOW, OPENING_FRAMES)));
        driver
    }

    /// The window at another size from the next frame on — what the
    /// desktop does when it maximizes the window a few frames after it
    /// opened, or the user pulls its corner.
    pub fn resize(&mut self, window: egui::Vec2) {
        self.growing.set(None);
        self.window.set(window);
    }

    /// The size the next frame is drawn at.
    pub fn window(&self) -> egui::Vec2 {
        self.window.get()
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// The time the next frame will be drawn at, in seconds.
    pub fn now(&self) -> f64 {
        self.clock.get()
    }

    /// Idle frames, a [`FRAME`] apart, until `seconds` have passed — the
    /// only way to see the end of what egui moves by a frame's time per
    /// frame however far the clock jumps: an animated value, and a scroll
    /// area's glide.
    pub fn wait<A: Driven + ?Sized>(&self, app: &mut A, seconds: f64) {
        let frames = (seconds / FRAME).ceil().max(1.0) as usize;
        for _ in 0..frames {
            self.settle(app);
        }
    }

    /// The input a frame starts from: the window as it is now, the clock,
    /// and the step a frame takes. The window grows here, and the clock
    /// moves on, so that every frame the driver runs keeps the same time.
    fn input(&self, time: Option<f64>) -> egui::RawInput {
        if let Some(time) = time {
            self.clock.set(time);
        }
        if let Some((to, left)) = self.growing.get() {
            match left {
                0 => {
                    self.window.set(to);
                    self.growing.set(None);
                }
                _ => self.growing.set(Some((to, left - 1))),
            }
        }
        let now = self.clock.get();
        self.clock.set(now + FRAME);
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                self.window.get(),
            )),
            time: Some(now),
            predicted_dt: FRAME as f32,
            ..Default::default()
        }
    }

    fn run<A: Driven + ?Sized>(&self, app: &mut A, input: egui::RawInput) -> egui::FullOutput {
        let mut out = self.ctx.run_ui(input, |ui| app.drive(ui));
        out.textures_delta.clear();
        self.cursor.set(out.platform_output.cursor_icon);
        out
    }

    /// The pointer's shape as the last frame asked for it — what a widget
    /// says about what a press there would do.
    pub fn cursor(&self) -> egui::CursorIcon {
        self.cursor.get()
    }

    /// One whole frame, with `events` as everything the keyboard did in it.
    pub fn frame<A: Driven + ?Sized>(&self, app: &mut A, events: Vec<egui::Event>) {
        self.frame_at(app, events, None);
    }

    /// A frame with its clock set, and everything it painted.
    ///
    /// The clock is what a caret's blink and a notice's fading read. A frame
    /// without one is drawn a [`FRAME`] after the last; with one, the clock
    /// is set to it, and the frames after go on from there. The shapes come
    /// back for the same reason a test reads the model back: what was
    /// painted is the only evidence that a caret, a shadow or a strike is on
    /// the screen at all. [`Driver::paint`] reads them.
    pub fn frame_at<A: Driven + ?Sized>(
        &self,
        app: &mut A,
        events: Vec<egui::Event>,
        time: Option<f64>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let input = egui::RawInput {
            events,
            ..self.input(time)
        };
        self.run(app, input).shapes
    }

    /// One frame, and what it painted, ready to be asked about.
    pub fn paint<A: Driven + ?Sized>(&self, app: &mut A, events: Vec<egui::Event>) -> Painted {
        Painted::new(&self.frame_at(app, events, None))
    }

    /// An idle frame with its clock set, and what it painted — how a test
    /// looks at the screen a blink or a fade later.
    pub fn paint_at<A: Driven + ?Sized>(&self, app: &mut A, time: Option<f64>) -> Painted {
        Painted::new(&self.frame_at(app, Vec::new(), time))
    }

    /// One frame, and whether it asked for the next one at once — what a
    /// frame that changed something on the screen after painting it must
    /// do, or the change waits for the next event or the caret's blink.
    pub fn frame_wants_repaint<A: Driven + ?Sized>(
        &self,
        app: &mut A,
        events: Vec<egui::Event>,
    ) -> bool {
        let input = egui::RawInput {
            events,
            ..self.input(None)
        };
        self.run(app, input)
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|viewport| viewport.repaint_delay.is_zero())
    }

    /// Files dropped on the window, in a frame of their own — what the
    /// desktop sends when a document is dragged out of a file manager.
    pub fn drop_files<A: Driven + ?Sized>(&self, app: &mut A, paths: &[std::path::PathBuf]) {
        let input = egui::RawInput {
            dropped_files: paths
                .iter()
                .map(|path| {
                    let file: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(path.clone()));
                    file
                })
                .collect(),
            ..self.input(None)
        };
        self.run(app, input);
    }

    /// A frame with nothing in it: egui has no fonts until a frame has run,
    /// and a shaper that asks before then panics rather than measuring. For
    /// a test that lays type without a window to lay it in.
    pub fn warm(&self) {
        self.settle(&mut Nothing);
    }

    /// A frame in which nothing is pressed — what a window does between keys,
    /// and what a menu or a dialog opened last frame needs in order to appear.
    pub fn settle<A: Driven + ?Sized>(&self, app: &mut A) {
        self.frame(app, Vec::new());
    }

    /// One key, pressed with `modifiers` held, in a frame of its own.
    pub fn key<A: Driven + ?Sized>(&self, app: &mut A, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(app, key_events(key, modifiers));
    }

    /// One key by name — `Enter`, `ctrl+shift+S`, `alt+F` — as [`key_spec`]
    /// reads it. A name it does not know is a fault in the test, and says so.
    pub fn press<A: Driven + ?Sized>(&self, app: &mut A, spec: &str) {
        let (key, modifiers) = key_spec(spec).unwrap_or_else(|why| panic!("{why}"));
        self.key(app, key, modifiers);
    }

    /// The primary button goes down at `at`, the pointer having moved there
    /// in the same frame.
    pub fn press_at<A: Driven + ?Sized>(&self, app: &mut A, at: egui::Pos2) {
        self.press_button(app, at, egui::PointerButton::Primary);
    }

    /// Any button goes down at `at`, the pointer having moved there in the
    /// same frame.
    pub fn press_button<A: Driven + ?Sized>(
        &self,
        app: &mut A,
        at: egui::Pos2,
        which: egui::PointerButton,
    ) {
        let events = vec![egui::Event::PointerMoved(at), button(at, which, true)];
        self.frame(app, events);
    }

    /// Any button comes up at `at`.
    pub fn release_button<A: Driven + ?Sized>(
        &self,
        app: &mut A,
        at: egui::Pos2,
        which: egui::PointerButton,
    ) {
        let events = vec![egui::Event::PointerMoved(at), button(at, which, false)];
        self.frame(app, events);
    }

    /// A click of the secondary button — what opens a context menu.
    pub fn right_click<A: Driven + ?Sized>(&self, app: &mut A, at: egui::Pos2) {
        self.press_button(app, at, egui::PointerButton::Secondary);
        self.release_button(app, at, egui::PointerButton::Secondary);
        self.settle(app);
    }

    /// The pointer moves to `at`, whatever the button is doing, in a frame
    /// of its own.
    pub fn move_to<A: Driven + ?Sized>(&self, app: &mut A, at: egui::Pos2) {
        self.frame(app, vec![egui::Event::PointerMoved(at)]);
    }

    /// The primary button comes up at `at`.
    pub fn release_at<A: Driven + ?Sized>(&self, app: &mut A, at: egui::Pos2) {
        self.release_button(app, at, egui::PointerButton::Primary);
    }

    /// A click of the pointer at a place in the window: the press in one
    /// frame and the release in the next, which is the least a click is.
    pub fn click<A: Driven + ?Sized>(&self, app: &mut A, at: egui::Pos2) {
        self.press_at(app, at);
        self.frame(app, vec![button(at, egui::PointerButton::Primary, false)]);
        self.settle(app);
    }

    /// Two clicks at one place, a frame apart, and a frame for what the pair
    /// did: well inside any double-click time, as a hand makes one.
    pub fn double_click<A: Driven + ?Sized>(&self, app: &mut A, at: egui::Pos2) {
        for _ in 0..2 {
            self.press_at(app, at);
            self.release_at(app, at);
        }
        self.settle(app);
    }

    /// A drag from `from` to `to`: pressed, moved there twice — egui calls a
    /// press a drag only once the pointer has moved in a frame after it —
    /// released, and a frame for what the release did.
    pub fn drag<A: Driven + ?Sized>(&self, app: &mut A, from: egui::Pos2, to: egui::Pos2) {
        self.drag_via(app, &[from, to]);
    }

    /// A drag that passes through every point of `path` in turn, a frame
    /// at each, and is released at the last.
    pub fn drag_via<A: Driven + ?Sized>(&self, app: &mut A, path: &[egui::Pos2]) {
        let (Some(&first), Some(&last)) = (path.first(), path.last()) else {
            panic!("a drag wants at least a point to start from");
        };
        self.press_at(app, first);
        for &at in &path[1..] {
            self.move_to(app, at);
        }
        self.move_to(app, last);
        self.release_at(app, last);
        self.settle(app);
    }

    /// The button held and the pointer resting at `at` for `frames` frames —
    /// what a sweep does at the edge of a view that scrolls under it. The
    /// button stays down; [`Driver::release_at`] ends it.
    pub fn hold<A: Driven + ?Sized>(&self, app: &mut A, at: egui::Pos2, frames: usize) {
        for _ in 0..frames {
            self.move_to(app, at);
        }
    }

    /// Text, as typed: one frame carrying it, the way a paste or a burst of
    /// keys arrives.
    pub fn type_text<A: Driven + ?Sized>(&self, app: &mut A, text: &str) {
        self.frame(app, vec![egui::Event::Text(text.to_owned())]);
    }

    /// A menu command by mnemonic: Alt and the title's letter, then the
    /// item's, with a frame between for the menu to open and one after for
    /// what it chose to happen.
    pub fn menu<A: Driven + ?Sized>(&self, app: &mut A, title: char, item: char) {
        let letter = |c: char| {
            egui::Key::from_name(&c.to_ascii_uppercase().to_string())
                .unwrap_or_else(|| panic!("`{c}` is not a letter a menu can be opened by"))
        };
        self.key(app, letter(title), egui::Modifiers::ALT);
        self.settle(app);
        self.key(app, letter(item), egui::Modifiers::NONE);
        self.settle(app);
    }
}

/// A few lines of drawing, run bare in the driver's window: for a test of a
/// piece of chrome — a message box, a menu row, a chart — that has no widget
/// type of its own to implement [`Driven`] on. The closure names its
/// argument's type, `|ui: &mut egui::Ui|`, for the compiler to take it.
pub struct Bare<F>(pub F);

impl<F: FnMut(&mut egui::Ui)> Driven for Bare<F> {
    fn drive(&mut self, ui: &mut egui::Ui) {
        (self.0)(ui);
    }
}

/// What [`Driver::warm`] runs.
struct Nothing;

impl Driven for Nothing {
    fn drive(&mut self, _ui: &mut egui::Ui) {}
}

/// A button, going down or coming up at a point.
fn button(at: egui::Pos2, which: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: which,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A file as the desktop drops one: its path, and its bytes read from it.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|why| why.to_string())
    }
}

impl Driver {
    /// Opens every menu under `titles` — the bar's letters — and every
    /// submenu under those, by keyboard, and returns each menu's rows by the
    /// path of letters that opened it. What the rows claimed is then in
    /// [`crate::menu::clashes`]. A menu that opens a dialog or runs a
    /// command is never chosen: only rows that open submenus are pressed,
    /// and only those that are enabled.
    pub fn every_menu<A: Driven + ?Sized>(
        &self,
        app: &mut A,
        titles: &str,
    ) -> Vec<(String, Vec<crate::menu::Row>)> {
        let mut found = Vec::new();
        let mut paths: Vec<String> = titles.chars().map(String::from).collect();
        while let Some(path) = paths.pop() {
            self.close_menus(app);
            let mut letters = path.chars();
            let title = letters.next().expect("a path has its title");
            let letter = |c: char| {
                egui::Key::from_name(&c.to_ascii_uppercase().to_string())
                    .unwrap_or_else(|| panic!("`{c}` is not a letter"))
            };
            self.key(app, letter(title), egui::Modifiers::ALT);
            self.settle(app);
            for c in letters {
                self.key(app, letter(c), egui::Modifiers::NONE);
                self.settle(app);
            }
            // A popup measures itself on its first frame and is drawn on the
            // next; the rows are read from the one that is drawn.
            self.settle(app);
            // The menu the path opens is as deep as the path is long; if the
            // innermost menu drawn is shallower, the last letter opened
            // nothing, and its parent's rows must not be walked again.
            let (rows, open) = crate::menu::innermost_rows(&self.ctx);
            if open != path.chars().count() {
                found.push((path, Vec::new()));
                continue;
            }
            for row in &rows {
                if let (true, true, Some(c)) = (row.sub, row.enabled, row.letter) {
                    paths.push(format!("{path}{c}"));
                }
            }
            found.push((path, rows));
        }
        self.close_menus(app);
        found
    }

    fn close_menus<A: Driven + ?Sized>(&self, app: &mut A) {
        for _ in 0..3 {
            self.key(app, egui::Key::Escape, egui::Modifiers::NONE);
        }
        self.settle(app);
    }
}

/// The menus [`Driver::every_menu`] walked, as a table of the keys that reach
/// each row — what `MAP.md` prints, so that "which letter opens what" is
/// looked up rather than found by trying.
pub fn menus_markdown(menus: &[(String, Vec<crate::menu::Row>)]) -> String {
    let mut menus: Vec<&(String, Vec<crate::menu::Row>)> = menus.iter().collect();
    menus.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::from("| keys | row |\n|---|---|\n");
    for (path, rows) in menus {
        let mut letters = path.chars();
        let Some(title) = letters.next() else {
            continue;
        };
        let mut keys = format!("Alt+{}", title.to_ascii_uppercase());
        for c in letters {
            keys.push_str(&format!(", {}", c.to_ascii_uppercase()));
        }
        for row in rows {
            let key = match row.letter {
                Some(c) => format!("{keys}, {}", c.to_ascii_uppercase()),
                None => format!("{keys}, —"),
            };
            let sub = if row.sub { " ▸" } else { "" };
            let off = if row.enabled {
                ""
            } else {
                " *(disabled here)*"
            };
            out.push_str(&format!(
                "| `{key}` | {}{sub}{off} |\n",
                row.label.replace('|', "\\|")
            ));
        }
    }
    out
}

/// What a keyboard puts into a frame for one press: the key with its
/// modifiers, and — for a printable key with neither Ctrl nor Alt held — the
/// text it types as well, because that is what a real keyboard sends and the
/// difference is where a bug lived: a menu letter that was consumed as a key
/// and still typed as text.
pub fn key_events(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    let mut events = Vec::new();
    // The platform turns Ctrl+C and Ctrl+X into events of their own, and
    // sends the key press as well: a grid reads the one and a shortcut table
    // the other, and both arrive.
    if modifiers.matches_exact(egui::Modifiers::COMMAND) {
        match key {
            egui::Key::C => events.push(egui::Event::Copy),
            egui::Key::X => events.push(egui::Event::Cut),
            _ => {}
        }
    }
    events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    if !modifiers.command && !modifiers.ctrl && !modifiers.alt {
        let typed = match key {
            egui::Key::Space => Some(" ".to_owned()),
            _ => {
                let name = key.name();
                (name.chars().count() == 1).then(|| match modifiers.shift {
                    true => name.to_uppercase(),
                    false => name.to_lowercase(),
                })
            }
        };
        if let Some(text) = typed {
            events.push(egui::Event::Text(text));
        }
    }
    events
}

/// `ctrl+shift+Home`: modifiers in front, egui's own key names last. The same
/// words a test and a script use, so that a key means one thing in both.
pub fn key_spec(spec: &str) -> Result<(egui::Key, egui::Modifiers), String> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut parts = spec.split('+').peekable();
    let mut name = None;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            name = Some(part);
            break;
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" => modifiers = modifiers.plus(egui::Modifiers::COMMAND),
            "shift" => modifiers = modifiers.plus(egui::Modifiers::SHIFT),
            "alt" => modifiers = modifiers.plus(egui::Modifiers::ALT),
            other => return Err(format!("`{other}` is not a modifier (ctrl, shift, alt)")),
        }
    }
    let name = name.filter(|n| !n.is_empty()).ok_or("a key wants a name")?;
    let key = egui::Key::from_name(name).ok_or_else(|| {
        format!("`{name}` is not a key name egui knows (Enter, Tab, ArrowDown, Home, A)")
    })?;
    Ok((key, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu;

    /// The smallest application with a menu and a place to type: what it
    /// records is what the frame let through to it.
    #[derive(Default)]
    struct Recorder {
        chosen: Vec<&'static str>,
        typed: String,
        frames: usize,
        keys_in_body: usize,
    }

    impl DocumentApp for Recorder {
        fn id(&self) -> crate::AppId {
            crate::SCRIVA
        }

        fn toolbar(&mut self, ui: &mut egui::Ui) {
            menu::bar(ui, |ui| {
                menu::top(ui, "&File", |ui| {
                    if menu::item(ui, "&New", "Ctrl+N").clicked() {
                        self.chosen.push("New");
                    }
                    if menu::item(ui, "&Open…", "Ctrl+O").clicked() {
                        self.chosen.push("Open");
                    }
                });
                menu::top(ui, "&Edit", |ui| {
                    if menu::item(ui, "Select &All", "Ctrl+A").clicked() {
                        self.chosen.push("Select All");
                    }
                });
                menu::top(ui, "&Insert", |ui| {
                    if menu::item(ui, "&Picture…", "").clicked() {
                        self.chosen.push("Picture");
                    }
                    menu::sub(ui, "Page &Number", |ui| {
                        if menu::item(ui, "&Plain Number", "").clicked() {
                            self.chosen.push("Plain Number");
                        }
                    });
                });
            });
        }

        fn ui(&mut self, ui: &mut egui::Ui) {
            self.frames += 1;
            ui.input(|input| {
                for event in &input.events {
                    match event {
                        egui::Event::Text(text) => self.typed.push_str(text),
                        egui::Event::Key { pressed: true, .. } => self.keys_in_body += 1,
                        _ => {}
                    }
                }
            });
        }
    }

    #[test]
    fn a_menu_is_opened_by_its_letter_and_the_letter_is_not_typed() {
        let drive = Driver::new();
        let mut app = Recorder::default();
        drive.settle(&mut app);
        drive.menu(&mut app, 'F', 'N');
        assert_eq!(app.chosen, vec!["New"]);
        assert_eq!(app.typed, "", "the N chose the item and was not typed");
        drive.menu(&mut app, 'E', 'A');
        assert_eq!(app.chosen, vec!["New", "Select All"]);
        assert_eq!(app.typed, "");
    }

    /// Insert, Page Number, P: the P is the submenu's "Plain Number", not
    /// the parent's "Picture…" one row up, which took it first and opened a
    /// file chooser — the keyboard belongs to the innermost open menu.
    #[test]
    fn a_letter_goes_to_the_open_submenu_and_not_to_its_parent() {
        let drive = Driver::new();
        let mut app = Recorder::default();
        drive.settle(&mut app);
        drive.menu(&mut app, 'I', 'N');
        drive.press(&mut app, "P");
        drive.settle(&mut app);
        assert_eq!(app.chosen, vec!["Plain Number"]);
        assert_eq!(app.typed, "");
        // And with no submenu open the parent's letter is its own.
        drive.menu(&mut app, 'I', 'P');
        assert_eq!(app.chosen, vec!["Plain Number", "Picture"]);
    }

    /// Every menu walked by keyboard, submenus included, and the clash
    /// between two rows of one menu that claim one letter is reported.
    #[test]
    fn every_menu_is_walked_and_a_shared_letter_is_reported() {
        #[derive(Default)]
        struct Clashing;
        impl DocumentApp for Clashing {
            fn id(&self) -> crate::AppId {
                crate::SCRIVA
            }
            fn toolbar(&mut self, ui: &mut egui::Ui) {
                menu::bar(ui, |ui| {
                    menu::top(ui, "&Table", |ui| {
                        menu::item(ui, "Cell &Margins…", "");
                        menu::item(ui, "&Merge Cells", "");
                        menu::sub(ui, "&Borders", |ui| {
                            menu::item(ui, "&All", "");
                            menu::item(ui, "&None", "");
                        });
                    });
                });
            }
            fn ui(&mut self, _ui: &mut egui::Ui) {}
        }
        let drive = Driver::new();
        let mut app = Clashing;
        drive.settle(&mut app);
        let menus = drive.every_menu(&mut app, "T");
        let paths: Vec<&str> = menus.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, vec!["T", "Tb"], "the submenu was opened too");
        assert_eq!(menus[1].1.len(), 2, "with its two rows");
        assert_eq!(
            crate::menu::clashes(drive.ctx()),
            vec!["`Cell Margins…` and `Merge Cells` both take m".to_owned()]
        );
    }

    /// The arrows walk a menu: Down lights the next row and wraps, Right
    /// opens a lit submenu, Enter chooses — so a row with no letter is still
    /// a row the keyboard can reach.
    #[test]
    fn the_arrows_light_a_row_and_enter_chooses_it() {
        let drive = Driver::new();
        let mut app = Recorder::default();
        drive.settle(&mut app);
        drive.key(&mut app, egui::Key::I, egui::Modifiers::ALT);
        drive.settle(&mut app);
        drive.settle(&mut app);
        drive.press(&mut app, "ArrowDown"); // Picture…
        drive.press(&mut app, "ArrowDown"); // Page Number ▸
        drive.press(&mut app, "ArrowRight");
        drive.settle(&mut app);
        drive.settle(&mut app);
        drive.press(&mut app, "ArrowDown"); // Plain Number
        drive.press(&mut app, "Enter");
        drive.settle(&mut app);
        assert_eq!(app.chosen, vec!["Plain Number"]);

        drive.key(&mut app, egui::Key::I, egui::Modifiers::ALT);
        drive.settle(&mut app);
        drive.settle(&mut app);
        drive.press(&mut app, "ArrowUp"); // wraps to the last row, Page Number
        drive.press(&mut app, "ArrowUp"); // Picture…
        drive.press(&mut app, "Enter");
        drive.settle(&mut app);
        assert_eq!(app.chosen, vec!["Plain Number", "Picture"]);
        assert_eq!(app.typed, "", "no key reached the document");
    }

    #[test]
    fn typing_reaches_the_document_and_a_plain_key_types_its_letter() {
        let drive = Driver::new();
        let mut app = Recorder::default();
        drive.type_text(&mut app, "hello");
        drive.press(&mut app, "shift+A");
        drive.press(&mut app, "Space");
        drive.press(&mut app, "ctrl+S");
        assert_eq!(app.typed, "helloA ", "Ctrl+S types nothing");
        assert_eq!(app.keys_in_body, 3);
        assert_eq!(app.frames, 4, "one frame a call");
    }

    #[test]
    fn a_key_is_named_the_way_a_script_names_it() {
        assert_eq!(
            key_spec("ctrl+shift+S").unwrap(),
            (
                egui::Key::S,
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT)
            )
        );
        assert_eq!(
            key_spec("ArrowDown").unwrap(),
            (egui::Key::ArrowDown, egui::Modifiers::NONE)
        );
        assert!(key_spec("super+X").unwrap_err().contains("super"));
        assert!(key_spec("ctrl+Whatever").unwrap_err().contains("Whatever"));
        assert!(key_spec("ctrl+").is_err());
        assert_eq!(key_events(egui::Key::Enter, egui::Modifiers::NONE).len(), 1);
        assert_eq!(
            key_events(egui::Key::C, egui::Modifiers::COMMAND)[0],
            egui::Event::Copy,
            "Ctrl+C arrives as a copy as well as a key, as it does from the platform"
        );
        assert_eq!(key_events(egui::Key::T, egui::Modifiers::ALT).len(), 1);
        assert_eq!(key_events(egui::Key::T, egui::Modifiers::NONE).len(), 2);
    }

    /// An application that paints one of everything a test asks about.
    struct Painter;

    const RED: egui::Color32 = egui::Color32::from_rgb(200, 0, 0);
    const BLUE: egui::Color32 = egui::Color32::from_rgb(0, 0, 200);
    const GREEN: egui::Color32 = egui::Color32::from_rgb(0, 150, 0);
    const GOLD: egui::Color32 = egui::Color32::from_rgb(200, 160, 0);

    impl DocumentApp for Painter {
        fn id(&self) -> crate::AppId {
            crate::SCRIVA
        }
        fn toolbar(&mut self, _ui: &mut egui::Ui) {}
        fn ui(&mut self, ui: &mut egui::Ui) {
            let painter = ui.painter().clone();
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(300.0, 200.0)),
                0.0,
                GOLD,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(30.0, 20.0)),
                0.0,
                GOLD,
            );
            painter.hline(40.0..=90.0, 300.0, egui::Stroke::new(1.0, GREEN));
            painter.line_segment(
                [egui::pos2(5.0, 5.0), egui::pos2(50.0, 60.0)],
                egui::Stroke::new(1.0, GREEN),
            );
            let font = egui::FontId::proportional(14.0);
            painter.text(
                egui::pos2(400.0, 20.0),
                egui::Align2::LEFT_TOP,
                "all red",
                font.clone(),
                RED,
            );
            // Two runs of one line, painted as two shapes, the way a page
            // paints its runs.
            painter.text(
                egui::pos2(400.0, 60.0),
                egui::Align2::LEFT_TOP,
                "plain ",
                font.clone(),
                ui.visuals().text_color(),
            );
            painter.text(
                egui::pos2(440.0, 60.0),
                egui::Align2::LEFT_TOP,
                "blue",
                font.clone(),
                BLUE,
            );
            // A colour left to the shape's fallback, and one overridden.
            let job = egui::text::LayoutJob::simple_singleline(
                "fallen back".into(),
                font.clone(),
                egui::Color32::PLACEHOLDER,
            );
            painter.galley(egui::pos2(400.0, 100.0), painter.layout_job(job), GREEN);
            let galley = painter.layout_no_wrap("overridden".into(), font, RED);
            painter.add(
                egui::epaint::TextShape::new(egui::pos2(400.0, 140.0), galley, RED)
                    .with_override_text_color(BLUE),
            );
        }
    }

    #[test]
    fn painted_reads_rects_texts_and_the_colour_each_letter_is_painted_in() {
        let drive = Driver::new();
        let mut app = Painter;
        drive.settle(&mut app);
        let painted = drive.paint(&mut app, Vec::new());
        assert_eq!(painted.filled(GOLD).len(), 2);
        assert_eq!(
            painted.largest(GOLD),
            Some(egui::Rect::from_min_size(
                egui::pos2(10.0, 10.0),
                egui::vec2(300.0, 200.0)
            )),
            "the larger of the two, whatever the order"
        );
        assert_eq!(
            painted.hlines(GREEN),
            vec![(300.0, 40.0, 90.0)],
            "the slanted line is not a rule"
        );
        let red = painted.text("all red").expect("the text is painted");
        assert!(red.rect.min.x >= 400.0 && red.rect.min.y >= 20.0);
        assert_eq!(
            red.letters.iter().filter(|(_, c)| c.is_none()).count(),
            1,
            "the blank has no colour"
        );
        assert_eq!(painted.colour_of("red"), Some(RED));
        assert_eq!(painted.colour_of("blue"), Some(BLUE));
        let across = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            painted.colour_of("plain blue")
        }));
        assert!(
            across.is_err(),
            "a word across two colours is not answered quietly: {across:?}"
        );
        assert_eq!(painted.colour_of("fallen"), Some(GREEN), "the fallback");
        assert_eq!(painted.colour_of("overridden"), Some(BLUE), "the override");
        assert_eq!(painted.colour_of("nowhere"), None);
        assert_eq!(painted.strings_starting("all"), vec!["all red".to_owned()]);
    }

    /// An application that records what the pointer did to one area.
    #[derive(Default)]
    struct Pointer {
        started: Option<egui::Pos2>,
        stopped: Option<egui::Pos2>,
        seen: Vec<egui::Pos2>,
        held_frames: usize,
        clicks: usize,
        doubles: usize,
        secondary: usize,
    }

    impl DocumentApp for Pointer {
        fn id(&self) -> crate::AppId {
            crate::SCRIVA
        }
        fn toolbar(&mut self, _ui: &mut egui::Ui) {}
        fn ui(&mut self, ui: &mut egui::Ui) {
            let (_, response) =
                ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
            if response.drag_started() {
                self.started = ui.input(|i| i.pointer.press_origin());
            }
            if response.dragged() {
                if let Some(at) = response.interact_pointer_pos() {
                    self.seen.push(at);
                }
            }
            if response.drag_stopped() {
                self.stopped = response.interact_pointer_pos();
            }
            if response.clicked() {
                self.clicks += 1;
            }
            if response.double_clicked() {
                self.doubles += 1;
            }
            if response.secondary_clicked() {
                self.secondary += 1;
            }
            if ui.input(|i| i.pointer.primary_down()) {
                self.held_frames += 1;
            }
        }
    }

    #[test]
    fn a_drag_is_a_press_moves_and_a_release_and_a_hold_keeps_the_button_down() {
        let drive = Driver::new();
        let mut app = Pointer::default();
        drive.settle(&mut app);
        let (from, via, to) = (
            egui::pos2(200.0, 300.0),
            egui::pos2(260.0, 320.0),
            egui::pos2(400.0, 500.0),
        );
        drive.drag_via(&mut app, &[from, via, to]);
        assert_eq!(
            app.started,
            Some(from),
            "the drag began where it was pressed"
        );
        assert!(
            app.seen.contains(&via),
            "and passed through {via:?}: {:?}",
            app.seen
        );
        assert_eq!(app.stopped, Some(to), "and ended where it was let go");
        assert_eq!(app.clicks, 0, "a drag is not a click");

        let mut app = Pointer::default();
        drive.press_at(&mut app, from);
        drive.hold(&mut app, to, 30);
        assert!(
            app.held_frames >= 30,
            "the button stayed down: {}",
            app.held_frames
        );
        drive.release_at(&mut app, to);
        drive.settle(&mut app);
        let held = app.held_frames;
        drive.settle(&mut app);
        assert_eq!(app.held_frames, held, "and came up at the release");
        assert_eq!(app.stopped, Some(to));

        let mut app = Pointer::default();
        drive.click(&mut app, from);
        assert_eq!(app.clicks, 1);
        assert_eq!(app.started, None);
        let mut app = Pointer::default();
        drive.right_click(&mut app, from);
        assert_eq!(
            (app.clicks, app.secondary),
            (0, 1),
            "a right click is its own"
        );
        // A pause first, as a hand makes one: egui times a double click
        // from the last click of any button, the right one included.
        let mut app = Pointer::default();
        drive.wait(&mut app, 1.0);
        drive.double_click(&mut app, from);
        assert_eq!((app.clicks, app.doubles), (2, 1), "two clicks, one pair");
    }

    /// An application that writes down the size of every frame.
    #[derive(Default)]
    struct Sizes(Vec<egui::Vec2>);

    impl DocumentApp for Sizes {
        fn id(&self) -> crate::AppId {
            crate::SCRIVA
        }
        fn toolbar(&mut self, _ui: &mut egui::Ui) {}
        fn ui(&mut self, ui: &mut egui::Ui) {
            self.0.push(ui.ctx().viewport_rect().size());
        }
    }

    #[test]
    fn an_opening_window_grows_to_its_full_size_a_few_frames_in() {
        let drive = Driver::opening();
        let mut app = Sizes::default();
        for _ in 0..OPENING_FRAMES + 2 {
            drive.settle(&mut app);
        }
        let opening = OPENING_FRAMES as usize;
        assert!(
            app.0[..opening]
                .iter()
                .all(|size| *size == shell::FIRST_SIZE),
            "the first frames at the first-run size: {:?}",
            app.0
        );
        assert!(
            app.0[opening..].iter().all(|size| *size == WINDOW),
            "and the rest at the full window: {:?}",
            app.0
        );
        assert_eq!(drive.window(), WINDOW);

        // A resize is the test's own, and stops a growth under way.
        let mut drive = Driver::opening();
        let mut app = Sizes::default();
        drive.resize(egui::vec2(900.0, 600.0));
        for _ in 0..OPENING_FRAMES + 2 {
            drive.settle(&mut app);
        }
        assert!(app.0.iter().all(|size| *size == egui::vec2(900.0, 600.0)));
    }

    /// An application that animates one value towards one, once it is
    /// told to: egui starts an animation seen for the first time at its end.
    #[derive(Default)]
    struct Fading {
        on: bool,
        value: f32,
    }

    impl DocumentApp for Fading {
        fn id(&self) -> crate::AppId {
            crate::SCRIVA
        }
        fn toolbar(&mut self, _ui: &mut egui::Ui) {}
        fn ui(&mut self, ui: &mut egui::Ui) {
            // A value, not a switch: egui moves a value by at most a frame's
            // time per frame, as it moves a scroll area, where a switch is
            // timed from the moment it was thrown.
            let target = if self.on { 1.0 } else { 0.0 };
            self.value = ui
                .ctx()
                .animate_value_with_time(egui::Id::new("fade"), target, 0.5);
        }
    }

    #[test]
    fn waiting_runs_frames_until_an_animation_has_finished() {
        let drive = Driver::new();
        let mut app = Fading::default();
        assert_eq!(drive.now(), 0.0);
        drive.settle(&mut app);
        assert!((drive.now() - FRAME).abs() < 1e-9, "a frame's time passed");
        app.on = true;
        drive.wait(&mut app, 0.25);
        assert!(
            app.value > 0.2 && app.value < 0.8,
            "half way there after a quarter of a second: {}",
            app.value
        );
        drive.wait(&mut app, 0.5);
        assert_eq!(app.value, 1.0, "and there once the time has passed");

        // One frame whose clock leaps a second does not finish it: egui
        // steps a value by a frame's time, which is why `wait` runs every
        // frame in between.
        let drive = Driver::new();
        let mut app = Fading::default();
        drive.settle(&mut app);
        app.on = true;
        drive.frame_at(&mut app, Vec::new(), Some(1.0));
        assert!(app.value < 0.2, "a leap is a step: {}", app.value);
        assert!(
            (drive.now() - (1.0 + FRAME)).abs() < 1e-9,
            "and the clock goes on from where it was set"
        );
    }

    /// An application that measures a line of type in the generic sans face.
    #[derive(Default)]
    struct Measure(Option<(f32, f32, f32)>);

    impl DocumentApp for Measure {
        fn id(&self) -> crate::AppId {
            crate::SCRIVA
        }
        fn toolbar(&mut self, _ui: &mut egui::Ui) {}
        fn ui(&mut self, ui: &mut egui::Ui) {
            let font = egui::FontId::new(
                20.0,
                crate::fonts::face(crate::fonts::Family::Sans, false, false),
            );
            let width = |text: &str| {
                ui.painter()
                    .layout_no_wrap(text.into(), font.clone(), egui::Color32::BLACK)
                    .size()
                    .x
            };
            let height = ui.fonts_mut(|fonts| fonts.row_height(&font));
            self.0 = Some((width("iiii"), width("mmmm"), height));
        }
    }

    #[test]
    fn a_generic_face_given_in_memory_is_the_one_text_is_set_in() {
        let measure = |drive: Driver| {
            let mut app = Measure::default();
            drive.settle(&mut app);
            app.0.expect("measured")
        };
        let (narrow, wide, egui_height) = measure(Driver::new());
        assert!(
            wide > narrow * 1.5,
            "egui's own face is proportional: {narrow} and {wide}"
        );
        let (narrow, wide, hack_height) = measure(Driver::in_hack());
        assert!(
            (wide - narrow).abs() < 0.01,
            "Hack is monospaced, so the sans face is Hack: {narrow} and {wide}"
        );
        assert!(
            (hack_height - egui_height).abs() > 0.1,
            "and its line is another height: {hack_height} against {egui_height}"
        );
    }
}
