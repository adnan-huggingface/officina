//! Scriva's window, driven: one file per area, and here what more than one
//! of them uses.

use super::*;

mod assisting;
mod bands;
mod desk;
mod editing;
mod files;
mod menus;
mod panes;
mod pictures;
mod tables;
mod tracking;

use ui_kit::drive::{Driven, Driver};

/// One key press, through `keys`, in a frame of its own.
///
/// egui's `consume_key` ignores an extra Shift or Alt, and every plain entry
/// in `keys` was asked before its shifted sibling, so Ctrl+Shift+S saved
/// without a dialog and Ctrl+Shift+M indented further.
fn pressed(app: &mut Scriva, key: egui::Key, modifiers: egui::Modifiers) -> Option<Command> {
    /// The key table alone, and what it answered on the last frame.
    struct Keys<'a> {
        app: &'a mut Scriva,
        found: Option<Command>,
    }
    impl Driven for Keys<'_> {
        fn drive(&mut self, ui: &mut egui::Ui) {
            self.found = self.app.keys(ui);
        }
    }
    let drive = Driver::new();
    let mut keys = Keys { app, found: None };
    drive.settle(&mut keys);
    drive.key(&mut keys, key, modifiers);
    keys.found
}

/// What every paragraph and every run of it resolves to through the
/// styles — faces by name, so this too is the same on any machine. The
/// style's own id is left out: it is a name, and names are allowed to
/// change on the way into a `.docx`.
fn resolved(
    document: &Document,
) -> Vec<(wp_model::prop::ParaProps, Vec<wp_model::prop::RunProps>)> {
    document
        .paragraphs()
        .iter()
        .map(|paragraph| {
            let layers = document.styles.resolve_paragraph(&paragraph.props, None);
            let runs = paragraph
                .runs()
                .iter()
                .map(|run| wp_model::prop::RunProps {
                    style: None,
                    ..document.styles.resolve_run(&layers, &run.props)
                })
                .collect();
            let para = wp_model::prop::ParaProps {
                style: None,
                ..layers.para
            };
            (para, runs)
        })
        .collect()
}

fn app_with(texts: &[&str]) -> Scriva {
    let mut app = Scriva::new();
    app.document.body = texts
        .iter()
        .map(|text| Block::Paragraph(Paragraph::of(text)))
        .collect();
    app.stamp += 1;
    app
}

fn watermark_draft(text: &str) -> WatermarkDraft {
    WatermarkDraft {
        text: text.to_owned(),
        font: String::new(),
        color: String::new(),
        diagonal: true,
        existing: false,
    }
}

/// Types `input` where the caret is, in whichever flow it is in.
fn typed(app: &mut Scriva, input: &str) {
    let caret = edit::type_text(
        &mut app.document,
        app.scope,
        &mut app.history,
        app.selection,
        input,
    );
    app.selection = Selection::at(caret);
    app.changed();
}

/// An app whose view has really been laid out, for keys that ask the
/// layout where the caret is — Home, End and the arrows.
fn laid_app(text: &str, text_width: f64) -> Scriva {
    let drive = Driver::new();
    drive.warm();
    let mut app = app_with(&[text]);
    let margins =
        app.document.section.margins.start.points() + app.document.section.margins.end.points();
    app.document.section.page.width = Twips::from_points(text_width + margins);
    let mut shaper = Egui::new(drive.ctx());
    app.view.refresh(
        &app.document,
        &wp_layout::FieldValues::new(),
        app.stamp,
        &mut shaper,
    );
    app.shaper = Some(shaper);
    app
}

/// Selects `range` of the first paragraph and copies it, without going near
/// the machine's real clipboard — a test that wrote to it would throw away
/// whatever the user had on it.
fn copied(app: &mut Scriva, range: std::ops::Range<usize>) {
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: range.start,
        },
        head: Caret {
            paragraph: 0,
            offset: range.end,
        },
    };
    let text = app.selected_text().expect("something to copy");
    app.clipboard = Some(Clip {
        text,
        paragraphs: edit::copy_range(&app.document, app.scope, app.selection),
    });
}

/// One transparent pixel, as a real PNG.
const PIXEL: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn text_of(app: &Scriva, index: usize) -> String {
    app.document.paragraphs()[index].text()
}

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/odt")
        .join(name)
}

/// A directory of this test's own, so that two of these running at once
/// cannot save over one another's document.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scriva-odt-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Hands the application the answer a file chooser would have given, the way
/// the chooser's own thread does, and runs frames until it is taken.
fn chooser_answers(drive: &ui_kit::drive::Driver, app: &mut Scriva, path: PathBuf, what: Chosen) {
    app.asking = Some(ui_kit::chooser::Asking::new(move || Some(path), what));
    for _ in 0..200 {
        if app.asking.is_none() {
            break;
        }
        drive.settle(app);
    }
    assert!(app.asking.is_none(), "the answer was taken");
}

/// A `.docx` from the corpus, by name.
fn corpus_docx(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/docx")
        .join(name)
}
