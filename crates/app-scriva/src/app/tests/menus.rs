//! Menus, shortcuts, boxes and the toolbar, reached the way a keyboard reaches them.

use super::*;

#[test]
fn a_shifted_shortcut_is_not_taken_by_its_unshifted_sibling() {
    let mut app = app_with(&["text"]);
    let ctrl_shift = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
    let ctrl_alt = egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT);
    assert_eq!(
        pressed(&mut app, egui::Key::S, ctrl_shift),
        Some(Command::SaveAs)
    );
    assert_eq!(
        pressed(&mut app, egui::Key::M, ctrl_shift),
        Some(Command::Indent(-1))
    );
    assert_eq!(
        pressed(&mut app, egui::Key::Z, ctrl_shift),
        Some(Command::Redo)
    );
    assert_eq!(
        pressed(&mut app, egui::Key::E, ctrl_shift),
        Some(Command::TrackChanges)
    );
    assert_eq!(
        pressed(&mut app, egui::Key::M, ctrl_alt),
        Some(Command::AddComment)
    );
    assert_eq!(
        pressed(&mut app, egui::Key::Equals, ctrl_shift),
        Some(Command::Superscript)
    );
    // And the unshifted ones still answer for themselves.
    let ctrl = egui::Modifiers::COMMAND;
    assert_eq!(pressed(&mut app, egui::Key::S, ctrl), Some(Command::Save));
    assert_eq!(
        pressed(&mut app, egui::Key::M, ctrl),
        Some(Command::Indent(1))
    );
    assert_eq!(pressed(&mut app, egui::Key::Z, ctrl), Some(Command::Undo));
    assert_eq!(
        pressed(&mut app, egui::Key::E, ctrl),
        Some(Command::Align(Justify::Center))
    );
    assert_eq!(
        pressed(&mut app, egui::Key::Equals, ctrl),
        Some(Command::Subscript)
    );
}

/// Review ▸ Track Changes by its letters, words typed, Accept All: the
/// revisions are kept as revisions and then settled, by menu alone.
#[test]
fn tracking_and_accepting_by_menu_letters() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["before"]);
    drive.settle(&mut app);
    drive.press(&mut app, "ctrl+End");
    drive.menu(&mut app, 'R', 'T');
    assert!(
        app.document.settings.track_changes,
        "Alt+R, T switched tracking on"
    );
    drive.type_text(&mut app, " and after");
    assert_eq!(app.document.paragraphs()[0].text(), "before and after");
    assert!(
        !crate::revise::tracked(&app.document).is_empty(),
        "typed as a tracked insertion"
    );
    drive.menu(&mut app, 'R', 'L');
    assert_eq!(app.document.paragraphs()[0].text(), "before and after");
    assert!(
        crate::revise::tracked(&app.document).is_empty(),
        "Accept All settled it"
    );
}

/// The dialogs nobody had driven, opened by their letters and answered
/// with Enter as they stand: Paragraph…, Custom Margins…, Column Width…
/// each leaves the document as it was and closes.
#[test]
fn paragraph_margins_and_column_width_dialogs_answer_enter_unchanged() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["a paragraph"]);
    drive.settle(&mut app);
    let before = app.document.clone();

    drive.menu(&mut app, 'P', 'P');
    assert!(app.paragraph_draft.is_some(), "Alt+P, P opened Paragraph");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.paragraph_draft.is_none(), "Enter closed Paragraph");

    drive.menu(&mut app, 'L', 'M');
    drive.press(&mut app, "C");
    drive.settle(&mut app);
    assert!(app.page_setup.is_some(), "Alt+L, M, C opened Page Setup");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.page_setup.is_none(), "Enter closed Page Setup");
    assert_eq!(app.document.section.margins, before.section.margins);
    assert_eq!(
        app.document.paragraphs()[0].props,
        before.paragraphs()[0].props,
        "nothing about the paragraph changed"
    );

    app.insert_table(2, 2);
    drive.settle(&mut app);
    drive.menu(&mut app, 'A', 'W');
    assert!(app.column_draft.is_some(), "Alt+A, W opened Column Width");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.column_draft.is_none(), "Enter closed Column Width");
}

/// Every box with a text field, opened by its command and typed at
/// straight away, as a keyboard user does: the first field has the
/// keyboard, and what is typed lands in it and nowhere else.
#[test]
fn a_box_that_opens_puts_the_keyboard_in_its_first_field() {
    let drive = ui_kit::drive::Driver::new();
    let mut failures = Vec::new();
    let mut check =
        |name: &str, command: Command, typed: &str, read: &dyn Fn(&Scriva) -> Option<String>| {
            let mut app = app_with(&["body text"]);
            if matches!(command, Command::ColumnWidth | Command::CellMargins) {
                app.insert_table(2, 2);
            }
            drive.settle(&mut app);
            app.run(command);
            drive.settle(&mut app);
            drive.type_text(&mut app, typed);
            let got = read(&app);
            let body: String = app.document.paragraphs().iter().map(|p| p.text()).collect();
            if got.as_deref() != Some(typed) || !body.contains("body text") {
                failures.push(format!("{name}: field {got:?}, body {body:?}"));
            }
        };
    check("Insert Table", Command::InsertTable, "4", &|app| {
        app.table_draft.as_ref().map(|d| d[0].clone())
    });
    check("Watermark", Command::Watermark, "DRAFT", &|app| {
        app.watermark_draft.as_ref().map(|d| d.text.clone())
    });
    check("Page Setup", Command::PageSetup, "2", &|app| {
        app.page_setup.as_ref().map(|d| d.top.clone())
    });
    check("Font", Command::FontDialog, "14", &|app| {
        app.font_draft.as_ref().map(|(_, d)| d.size.clone())
    });
    check("Paragraph", Command::ParagraphDialog, "6", &|app| {
        app.paragraph_draft.as_ref().map(|(_, d)| d.before.clone())
    });
    check("Text Colour", Command::CustomColor, "FF0000", &|app| {
        app.color_draft.as_ref().map(|(_, d)| d.clone())
    });
    check("Column Width", Command::ColumnWidth, "3", &|app| {
        app.column_draft.clone()
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Review ▸ New Comment by its letters on a selection, the note typed,
/// Ctrl+Enter — Word's key for posting a comment: the comment is on the
/// document. Tab cannot leave a multi-line field, so the box needed the
/// mouse for its Add button before it had the key.
#[test]
fn a_comment_by_menu_letters_and_keys() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["a word to comment on"]);
    drive.settle(&mut app);
    drive.press(&mut app, "ctrl+A");
    assert!(app.has_selection(), "Ctrl+A selected the text");
    drive.menu(&mut app, 'R', 'C');
    drive.settle(&mut app);
    assert!(app.draft.is_some(), "Alt+R, C opened a draft in the pane");
    drive.type_text(&mut app, "a note");
    assert_eq!(
        app.draft.as_ref().map(|d| d.text.as_str()),
        Some("a note"),
        "typed into the draft"
    );
    drive.press(&mut app, "Enter");
    assert_eq!(
        app.draft.as_ref().map(|d| d.text.as_str()),
        Some("a note\n"),
        "Enter is a new line in the note"
    );
    drive.press(&mut app, "Backspace");
    drive.press(&mut app, "ctrl+Enter");
    drive.settle(&mut app);
    assert!(
        app.draft.is_none(),
        "Ctrl+Enter posted it and closed the draft"
    );
    assert_eq!(
        app.document.comments.len(),
        1,
        "and the comment is on the document"
    );
    let said: String = app.document.comments[0]
        .content
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph.text()),
            _ => None,
        })
        .collect();
    assert_eq!(said, "a note");
}

/// The boxes by their letters with a number typed over the one offered:
/// Insert Table 3 by 4, a paragraph's space before, a page's top margin.
/// What was typed is what the document gets.
#[test]
fn numbers_typed_into_boxes_by_menu_letters_reach_the_document() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["a paragraph"]);
    drive.settle(&mut app);

    drive.menu(&mut app, 'I', 'T');

    drive.press(&mut app, "I");

    drive.settle(&mut app);
    drive.type_text(&mut app, "3");
    drive.press(&mut app, "Tab");
    drive.type_text(&mut app, "4");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    let table = app
        .document
        .body
        .iter()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("a table");
    assert_eq!(
        (table.rows[0].cells.len(), table.rows.len()),
        (3, 4),
        "columns then rows, as the box asks"
    );

    let mut app = app_with(&["a paragraph"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'P', 'P');
    drive.type_text(&mut app, "12");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.paragraph_draft.is_none());
    assert_eq!(
        app.document.paragraphs()[0].props.spacing.before,
        Some(Twips(240)),
        "twelve points before"
    );

    let mut app = app_with(&["a paragraph"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'L', 'M');
    drive.press(&mut app, "C");
    drive.settle(&mut app);
    drive.type_text(&mut app, "2");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.page_setup.is_none());
    assert_eq!(
        app.document.section.margins.top,
        Twips(2880),
        "a two-inch top margin"
    );
}

/// With a box up the menu bar is the box's business: Alt and a letter
/// open no menu behind it, and the letter after runs no command there.
/// Found when Alt+A, G was pressed with Cell Margins already open — the
/// Table menu opened behind the box, and the box's Enter went to it.
#[test]
fn a_menu_does_not_open_behind_an_open_box() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["text"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'I', 'T');
    drive.press(&mut app, "I");
    drive.settle(&mut app);
    assert!(app.table_draft.is_some(), "Insert Table is up");
    drive.menu(&mut app, 'E', 'A');
    assert!(
        !app.has_selection(),
        "Alt+E, A behind the box selected the document"
    );
    assert!(
        !egui::Popup::is_any_open(drive.ctx()),
        "a menu is open behind the box"
    );
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.table_draft.is_none(), "Escape still closes the box");
}

/// Every menu and submenu, walked by keyboard: no two rows of one menu
/// claim one letter, since the second of them could never be chosen by
/// it. Walked with a table and a picture-free document, so the Table
/// menu's rows are live.
#[test]
fn no_two_rows_of_a_menu_share_a_letter() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["text"]);
    app.insert_table(2, 2);
    // A recent list with something on it, so File ▸ Recent has rows.
    app.recent
        .remember(SCRIVA, Path::new("/nowhere/walked.docx"));
    drive.settle(&mut app);
    let menus = drive.every_menu(&mut app, "FEVIOPLARSH");
    for (path, rows) in &menus {
        eprintln!(
            "MENU {path}: {}",
            rows.iter()
                .map(|r| format!("{}{}", r.label, if r.sub { " >" } else { "" }))
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    let unopened: Vec<&str> = menus
        .iter()
        .filter(|(_, rows)| rows.is_empty())
        .map(|(path, _)| path.as_str())
        .collect();
    // `cargo xtask map` asks for the walk, for the map's table of menus.
    if let Some(out) = std::env::var_os("OFFICINA_MENUS_OUT") {
        std::fs::write(out, ui_kit::drive::menus_markdown(&menus)).expect("the menus written");
    }
    let clashes = ui_kit::menu::clashes(drive.ctx());
    assert!(clashes.is_empty(), "{}", clashes.join("\n"));
    assert!(unopened.is_empty(), "these did not open: {unopened:?}");
}

/// Styles by the arrows: the Styles menu's rows have no letters, and until
/// the arrows walked a menu they could be chosen only with the pointer.
#[test]
fn a_style_is_chosen_from_the_styles_menu_by_the_arrows() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["a paragraph"]);
    drive.settle(&mut app);
    let styles = app.quick_styles();
    let heading = styles
        .iter()
        .position(|(_, name)| name == "Heading 1")
        .expect("Heading 1 is a quick style");
    drive.key(&mut app, egui::Key::S, egui::Modifiers::ALT);
    drive.settle(&mut app);
    drive.settle(&mut app);
    for _ in 0..=heading {
        drive.press(&mut app, "ArrowDown");
    }
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert_eq!(
        app.document.paragraphs()[0].props.style,
        Some(styles[heading].0),
        "the lit row's style was applied"
    );
    assert_eq!(app.document.paragraphs()[0].text(), "a paragraph");
}

#[test]
fn every_toolbar_command_is_reachable_at_eight_hundred_wide() {
    let drive = ui_kit::drive::Driver::sized(egui::vec2(800.0, 600.0));
    let mut app = app_with(&["text"]);
    drive.settle(&mut app);
    drive.settle(&mut app);
    let on_row = crate::toolbar::drawn(drive.ctx());
    let more = on_row
        .iter()
        .find(|drawn| drawn.control == crate::toolbar::Control::More)
        .expect("at 800 wide something folds, so the overflow is drawn")
        .rect;
    drive.click(&mut app, more.center());
    drive.settle(&mut app);
    let (rows, open) = ui_kit::menu::innermost_rows(drive.ctx());
    assert_eq!(open, 1, "the overflow menu is open");
    let mut reachable: Vec<String> = on_row.iter().map(|d| d.control.name().to_owned()).collect();
    reachable.extend(rows.iter().map(|row| row.label.clone()));
    let missing: Vec<&str> = crate::toolbar::Control::all()
        .map(|control| control.name())
        .filter(|name| !reachable.iter().any(|r| r == name))
        .collect();
    assert!(
        missing.is_empty(),
        "not on the row or in the overflow: {missing:?}"
    );
}

#[test]
fn every_toolbar_tooltip_ends_with_the_key_the_table_gives() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["text"]);
    drive.settle(&mut app);
    drive.settle(&mut app);
    let drawn = crate::toolbar::drawn(drive.ctx());
    assert!(
        drawn.len() > 20,
        "the whole row is drawn at 1600: {}",
        drawn.len()
    );
    for control in crate::toolbar::Control::all() {
        let drawn = drawn
            .iter()
            .find(|d| d.control == control)
            .unwrap_or_else(|| panic!("{control:?} is on the row"));
        assert!(
            drawn.tip.starts_with(control.name()),
            "{control:?}'s tooltip {:?} starts with its name",
            drawn.tip
        );
        let key = match control {
            crate::toolbar::Control::Bold => "Ctrl+B",
            crate::toolbar::Control::Undo => "Ctrl+Z",
            crate::toolbar::Control::AlignCentre => "Ctrl+E",
            crate::toolbar::Control::Find => "Ctrl+F",
            crate::toolbar::Control::Track => "Ctrl+Shift+E",
            crate::toolbar::Control::Comment => "Ctrl+Alt+M",
            _ => continue,
        };
        assert!(
            drawn.tip.ends_with(key),
            "{control:?}'s tooltip {:?} ends with {key}",
            drawn.tip
        );
    }
}

#[test]
fn a_click_on_bold_toggles_bold_through_the_command() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["word"]);
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert!(!app.emphasis().0);
    let bold = crate::toolbar::drawn(drive.ctx())
        .into_iter()
        .find(|d| d.control == crate::toolbar::Control::Bold)
        .expect("B is on the row")
        .rect;
    drive.click(&mut app, bold.center());
    assert!(
        app.emphasis().0,
        "the word at the caret is bold after the click"
    );
    // And the key does the same thing, through the same command.
    drive.press(&mut app, "ctrl+B");
    drive.settle(&mut app);
    assert!(!app.emphasis().0, "and Ctrl+B takes it off again");
}

/// Shift+F10 opens the page's menu at the caret, Escape closes it, and the
/// caret is where it was through both — a menu opened from the keyboard is
/// about the place the keyboard is, and closing it must not lose the place.
#[test]
fn shift_f10_opens_the_context_menu_and_escape_closes_it_without_moving_the_caret() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["the quick fox"]);
    let at = Caret {
        paragraph: 0,
        offset: 4,
    };
    app.selection = Selection::at(at);
    drive.settle(&mut app);
    drive.press(&mut app, "shift+F10");
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert!(
        egui::Popup::is_any_open(drive.ctx()),
        "Shift+F10 opened the menu"
    );
    let (rows, _) = ui_kit::menu::innermost_rows(drive.ctx());
    let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    assert!(
        labels.contains(&"Paste") && labels.contains(&"Select All"),
        "the text menu's rows: {labels:?}"
    );
    assert_eq!(app.caret(), at, "the caret stayed put while it opened");
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert!(
        !egui::Popup::is_any_open(drive.ctx()),
        "Escape closed the menu"
    );
    assert_eq!(app.caret(), at, "and the caret is still where it was");
    assert_eq!(app.document.paragraphs()[0].text(), "the quick fox");
}

/// The page's menu in each of the states it has rows for — text, a table
/// with the caret in it, a tracked change under the caret, a picked picture
/// — with every submenu opened by its letter: no two rows of any of them
/// share a letter, and every state has rows at all.
#[test]
fn no_two_rows_of_the_context_menu_share_a_letter() {
    let drive = ui_kit::drive::Driver::new();
    let open = |drive: &ui_kit::drive::Driver, app: &mut Scriva, subs: &[&str]| {
        drive.settle(app);
        drive.press(app, "shift+F10");
        drive.settle(app);
        drive.settle(app);
        assert!(egui::Popup::is_any_open(drive.ctx()), "the menu opened");
        let (rows, _) = ui_kit::menu::innermost_rows(drive.ctx());
        assert!(!rows.is_empty(), "it has rows");
        let labels: Vec<String> = rows.iter().map(|row| row.label.clone()).collect();
        for sub in subs {
            drive.press(app, sub);
            drive.settle(app);
            drive.settle(app);
            let (inner, depth) = ui_kit::menu::innermost_rows(drive.ctx());
            assert_eq!(depth, 2, "{sub} opened its submenu: {inner:?}");
            assert!(!inner.is_empty(), "{sub}'s submenu has rows");
            drive.press(app, "Escape");
            drive.settle(app);
        }
        let clashes = ui_kit::menu::clashes(drive.ctx());
        assert!(clashes.is_empty(), "{labels:?}: {}", clashes.join("\n"));
        for _ in 0..2 {
            drive.press(app, "Escape");
            drive.settle(app);
        }
        assert!(!egui::Popup::is_any_open(drive.ctx()), "closed again");
        labels
    };

    // Text.
    let mut app = app_with(&["plain text"]);
    let labels = open(&drive, &mut app, &["S"]);
    assert!(
        labels.contains(&"Paste Unformatted".to_owned()),
        "{labels:?}"
    );

    // In a table: the Table submenu in front.
    let mut app = app_with(&["after"]);
    app.insert_table(2, 2);
    let labels = open(&drive, &mut app, &["T", "S"]);
    assert_eq!(
        labels.first().map(String::as_str),
        Some("Table"),
        "{labels:?}"
    );

    // On a tracked change: Accept and Reject in front.
    let mut app = app_with(&["plain text"]);
    drive.settle(&mut app);
    app.run(Command::TrackChanges);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 5,
    });
    drive.type_text(&mut app, " new");
    drive.settle(&mut app);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 7,
    });
    let labels = open(&drive, &mut app, &["S"]);
    assert_eq!(
        labels.first().map(String::as_str),
        Some("Accept Change"),
        "{labels:?}"
    );

    // A picked picture: its own menu.
    let mut app = app_with(&["ab"]);
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48));
    assert!(app.picked.is_some());
    let labels = open(&drive, &mut app, &["A"]);
    assert_eq!(
        labels.first().map(String::as_str),
        Some("Cut"),
        "{labels:?}"
    );
    assert!(labels.contains(&"Original size".to_owned()), "{labels:?}");
    assert!(!labels.contains(&"Paste".to_owned()), "not the text's menu");
}

/// Ctrl+G, `5`, Enter: the caret is on page five's first line, the box is
/// gone, and nothing was typed into the document on the way.
#[test]
fn ctrl_g_five_enter_puts_the_caret_on_page_five() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["one", "two", "three", "four", "five", "six"]);
    drive.settle(&mut app);
    // A page break before each paragraph after the first: six pages.
    for paragraph in (1..6).rev() {
        app.selection = Selection::at(Caret {
            paragraph,
            offset: 0,
        });
        app.run(Command::PageBreak);
    }
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert!(
        app.view.pages().len() >= 6,
        "{} pages",
        app.view.pages().len()
    );
    app.selection = Selection::at(Caret::default());
    drive.press(&mut app, "ctrl+G");
    drive.settle(&mut app);
    assert!(app.goto.is_some(), "Ctrl+G opened Go To");
    drive.type_text(&mut app, "5");
    drive.settle(&mut app);
    assert_eq!(app.goto.as_deref(), Some("5"), "the field took the 5");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert!(app.goto.is_none(), "Enter closed it");
    let page = view::caret_rect(&app.view, app.scope, app.caret())
        .map(|(index, _)| index + 1)
        .expect("the caret is on a page");
    assert_eq!(page, 5, "the caret is on page five");
    assert_eq!(
        app.document.paragraphs()[app.caret().paragraph].text(),
        "five",
        "on its first line"
    );
    let text: String = app.document.paragraphs().iter().map(|p| p.text()).collect();
    assert!(
        !text.contains('5'),
        "nothing was typed into the document: {text:?}"
    );
    // And a step from here.
    drive.press(&mut app, "ctrl+G");
    drive.settle(&mut app);
    drive.type_text(&mut app, "-2");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    drive.settle(&mut app);
    let page = view::caret_rect(&app.view, app.scope, app.caret())
        .map(|(index, _)| index + 1)
        .expect("still on a page");
    assert_eq!(page, 3, "-2 from five is three");
}

/// The guide's key tables and the command table say the same keys: every
/// key the table reads is in the guide's Scriva section, and every
/// modifier key the guide names for Scriva is one the table reads — the
/// editing keys egui delivers itself, and the menu mnemonics, aside.
#[test]
fn the_guides_key_tables_are_the_command_tables() {
    let guide =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../GUIDE.md"))
            .expect("GUIDE.md is at the top of the tree");
    let start = guide.find("## Scriva").expect("a Scriva section");
    let end = guide[start + 1..]
        .find("\n## ")
        .map(|at| start + 1 + at)
        .unwrap_or(guide.len());
    let section = &guide[start..end];
    let mut in_guide: Vec<String> = Vec::new();
    for line in section.lines() {
        for (index, piece) in line.split('`').enumerate() {
            // Odd pieces are inside backticks.
            if index % 2 == 1 && !piece.contains(' ') {
                in_guide.push(piece.to_owned());
            }
        }
    }
    let in_table: Vec<&str> = crate::commands::TABLE
        .iter()
        .map(|entry| entry.shown)
        .filter(|shown| !shown.is_empty())
        .collect();
    let missing: Vec<&str> = in_table
        .iter()
        .copied()
        .filter(|key| !in_guide.iter().any(|g| g == key))
        .collect();
    assert!(
        missing.is_empty(),
        "keys the table reads and the guide does not name: {missing:?}"
    );
    // Keys the guide names that are not commands: the editor's own movement
    // and structure keys, which egui or the surface handle, and the one
    // menu mnemonic the guide points at.
    const NOT_COMMANDS: &[&str] = &[
        "Ctrl+Arrow",
        "Ctrl+Home",
        "Ctrl+End",
        "Ctrl+Tab",
        "Shift+Tab",
        "Alt+A",
        "F6",
        "Shift+F6",
        "Ctrl+click",
    ];
    let unread: Vec<&String> = in_guide
        .iter()
        .filter(|key| {
            (key.contains("Ctrl+")
                || key.contains("Alt+")
                || key.starts_with('F') && key[1..].chars().all(|c| c.is_ascii_digit())
                || key.starts_with("Shift+F"))
                && !in_table.contains(&key.as_str())
                && !NOT_COMMANDS.contains(&key.as_str())
        })
        .collect();
    assert!(
        unread.is_empty(),
        "keys the guide names and nothing reads: {unread:?}"
    );
}

/// Alt+H opens the Help menu, whose rows are the two boxes and the guide;
/// K opens Keyboard Shortcuts, which lists the table's keys.
#[test]
fn the_help_menu_opens_by_its_letter_and_lists_the_shortcuts() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["text"]);
    drive.settle(&mut app);
    drive.press(&mut app, "alt+H");
    drive.settle(&mut app);
    drive.settle(&mut app);
    let (rows, _) = ui_kit::menu::innermost_rows(drive.ctx());
    let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Keyboard Shortcuts…", "User Guide", "About Scriva"],
        "the Help menu's rows"
    );
    drive.press(&mut app, "K");
    drive.settle(&mut app);
    assert!(app.shortcuts_up, "K opened Keyboard Shortcuts");
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(!app.shortcuts_up, "and Escape closed it");
    assert_eq!(
        app.document.paragraphs()[0].text(),
        "text",
        "nothing was typed"
    );
}
