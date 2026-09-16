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

use eframe::egui;

use crate::shell::{self, DocumentApp};

/// The window a driven application is given. Wide enough for every toolbar
/// to fit without folding and tall enough for a page, since a frame that
/// hides a control is a frame in which its shortcut cannot be tested.
pub const WINDOW: egui::Vec2 = egui::vec2(1600.0, 1000.0);

/// A window without a screen: the context, the fonts and the theme the shell
/// would give the application, and a way to run frames through it.
pub struct Driver {
    ctx: egui::Context,
    window: egui::Vec2,
}

impl Default for Driver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver {
    /// A fresh context, with no system font loaded: the layout in a test is
    /// measured in whatever egui ships, which is the same on every machine,
    /// where a machine's own fonts are not.
    pub fn new() -> Driver {
        crate::headless::enter();
        let ctx = egui::Context::default();
        crate::fonts::register(&ctx, &[]);
        shell::theme(&ctx);
        Driver {
            ctx,
            window: WINDOW,
        }
    }

    /// The same, in a window of another size — for what a toolbar does when
    /// it does not fit.
    pub fn sized(window: egui::Vec2) -> Driver {
        Driver {
            window,
            ..Driver::new()
        }
    }

    /// The window at another size from the next frame on — what the
    /// desktop does when it maximizes the window a few frames after it
    /// opened, or the user pulls its corner.
    pub fn resize(&mut self, window: egui::Vec2) {
        self.window = window;
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// One whole frame, with `events` as everything the keyboard did in it.
    pub fn frame<A: DocumentApp>(&self, app: &mut A, events: Vec<egui::Event>) {
        self.frame_at(app, events, None);
    }

    /// A frame with its clock set, and everything it painted.
    ///
    /// The clock is what a caret's blink and a notice's fading read, and a
    /// frame without one is a sixtieth of a second after the last — which is
    /// the right pace for typing and no way to ask what the screen shows
    /// four seconds on. The shapes come back for the same reason a test
    /// reads the model back: what was painted is the only evidence that a
    /// caret, a shadow or a strike is on the screen at all.
    pub fn frame_at<A: DocumentApp>(
        &self,
        app: &mut A,
        events: Vec<egui::Event>,
        time: Option<f64>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.window)),
            events,
            time,
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| shell::frame(app, ui));
        out.textures_delta.clear();
        out.shapes
    }

    /// Files dropped on the window, in a frame of their own — what the
    /// desktop sends when a document is dragged out of a file manager.
    pub fn drop_files<A: DocumentApp>(&self, app: &mut A, paths: &[std::path::PathBuf]) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.window)),
            dropped_files: paths
                .iter()
                .map(|path| {
                    let file: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(path.clone()));
                    file
                })
                .collect(),
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| shell::frame(app, ui));
        out.textures_delta.clear();
    }

    /// A frame in which nothing is pressed — what a window does between keys,
    /// and what a menu or a dialog opened last frame needs in order to appear.
    pub fn settle<A: DocumentApp>(&self, app: &mut A) {
        self.frame(app, Vec::new());
    }

    /// One key, pressed with `modifiers` held, in a frame of its own.
    pub fn key<A: DocumentApp>(&self, app: &mut A, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(app, key_events(key, modifiers));
    }

    /// One key by name — `Enter`, `ctrl+shift+S`, `alt+F` — as [`key_spec`]
    /// reads it. A name it does not know is a fault in the test, and says so.
    pub fn press<A: DocumentApp>(&self, app: &mut A, spec: &str) {
        let (key, modifiers) = key_spec(spec).unwrap_or_else(|why| panic!("{why}"));
        self.key(app, key, modifiers);
    }

    /// A click of the pointer at a place in the window: the press in one
    /// frame and the release in the next, which is the least a click is.
    pub fn click<A: DocumentApp>(&self, app: &mut A, at: egui::Pos2) {
        let button = |pressed: bool| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(app, vec![egui::Event::PointerMoved(at), button(true)]);
        self.frame(app, vec![button(false)]);
        self.settle(app);
    }

    /// Text, as typed: one frame carrying it, the way a paste or a burst of
    /// keys arrives.
    pub fn type_text<A: DocumentApp>(&self, app: &mut A, text: &str) {
        self.frame(app, vec![egui::Event::Text(text.to_owned())]);
    }

    /// A menu command by mnemonic: Alt and the title's letter, then the
    /// item's, with a frame between for the menu to open and one after for
    /// what it chose to happen.
    pub fn menu<A: DocumentApp>(&self, app: &mut A, title: char, item: char) {
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
    pub fn every_menu<A: DocumentApp>(
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

    fn close_menus<A: DocumentApp>(&self, app: &mut A) {
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
}
