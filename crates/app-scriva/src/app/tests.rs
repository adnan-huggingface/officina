use super::*;

#[test]
fn a_chart_from_the_board_arrives_as_a_part_and_a_drawing_that_renders() {
    // What the paste hands over is exactly what Calx's copy packs: the
    // chartSpace bytes and a size. The document must end up holding all
    // three pieces — part, relationship, drawing — and the part must be
    // one our own reader draws, or the paste has produced a hole.
    let mut app = Scriva::new();
    let chart_space = ss_xlsx_free_chart();
    assert!(
        app.insert_chart_part(&chart_space, 10_000_000, 3_000_000),
        "the paste lands"
    );

    let drawings: Vec<_> = app
        .document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.drawings().into_iter().cloned())
        .collect();
    let [drawing] = &drawings[..] else {
        panic!("one chart, not {}", drawings.len());
    };
    let rel = drawing.chart.as_deref().expect("the drawing names a part");
    assert!(drawing.rel.is_none(), "a chart is not a picture");

    // Wider than the text column, so it shrank to fit, proportions kept.
    let (wp_model::Emu(cx), wp_model::Emu(cy)) = drawing.extent;
    assert!(cx < 10_000_000, "shrunk: {cx}");
    // Within one EMU of true proportion: the shrink divides integers.
    assert!(
        (cy * 10_000_000 - cx * 3_000_000).abs() <= 10_000_000,
        "in proportion: {cx}x{cy}"
    );

    let parts = app.parts.as_ref().expect("the part index was rebuilt");
    let name = parts.target(rel).expect("the relationship resolves");
    let package = app.package.as_ref().expect("a package was authored");
    let part = package.part(name).expect("the part is there");
    let plot = chart::read::plot(part.data()).expect("and the reader draws it");
    assert_eq!(plot.series.len(), 1);
}

/// A chartSpace as Calx would put one on the board — hand-written rather
/// than imported, because this crate must not depend on the spreadsheet
/// stack to test a paste.
fn ss_xlsx_free_chart() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/><c:val><c:numRef><c:f>Sheet1!$A$1:$A$2</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>5</c:v></c:pt><c:pt idx="1"><c:v>7</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx></c:plotArea><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart></c:chartSpace>"#.to_vec()
}

#[test]
fn a_link_to_a_bookmark_takes_the_caret_there() {
    // The demonstration document's "paragraph level formatting" points at
    // a heading further up. Following it is a caret move and a scroll —
    // there is nothing to open and nowhere to go.
    let mut app = Scriva::new();
    let mut heading = Paragraph::of("Paragraph level formatting");
    heading.content.insert(
        0,
        wp_model::doc::Inline::Anchor(wp_model::revision::Anchor::BookmarkStart {
            id: 1,
            name: "_Paragraph_level_formatting".into(),
        }),
    );
    let mut linking = Paragraph::of("back to ");
    linking
        .content
        .push(wp_model::doc::Inline::Hyperlink(Box::new(
            wp_model::doc::Hyperlink {
                rel: None,
                anchor: Some("_Paragraph_level_formatting".into()),
                tooltip: None,
                history: true,
                content: vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of(
                    "that section",
                ))],
            },
        )));
    app.document.body = vec![
        Block::Paragraph(heading),
        Block::Paragraph(Paragraph::of("in between")),
        Block::Paragraph(linking),
    ];

    let inside = Caret {
        paragraph: 2,
        offset: 10,
    };
    let found = app.link_at(inside).expect("the caret stands in the link");
    assert_eq!(
        found,
        crate::links::Destination::Here("_Paragraph_level_formatting".to_owned())
    );
    app.follow_link(found);
    assert_eq!(app.caret().paragraph, 0, "the caret went to the bookmark");
    assert!(app.reveal.is_some(), "and the view was asked to follow");
    assert!(app.message.is_none(), "with nothing to report");

    // A link to a mark that is not in the document says so rather than
    // appearing to do nothing.
    app.follow_link(crate::links::Destination::Here("_gone".to_owned()));
    assert!(app.message.is_some(), "a dangling link is reported");
}

/// One key press, through `keys`, in a frame of its own.
///
/// egui's `consume_key` ignores an extra Shift or Alt, and every plain entry
/// in `keys` was asked before its shifted sibling, so Ctrl+Shift+S saved
/// without a dialog and Ctrl+Shift+M indented further.
fn pressed(app: &mut Scriva, key: egui::Key, modifiers: egui::Modifiers) -> Option<Command> {
    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    let mut warm = ctx.run_ui(egui::RawInput::default(), |_| {});
    warm.textures_delta.clear();
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    let mut found = None;
    let mut out = ctx.run_ui(input, |ui| {
        found = app.keys(ui);
    });
    out.textures_delta.clear();
    found
}

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

/// Frames of `overlay` — the dialogs — each with one key pressed.
fn press_in_dialogs(app: &mut Scriva, keys: &[egui::Key]) {
    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    let mut warm = ctx.run_ui(egui::RawInput::default(), |ui| app.overlay(ui.ctx()));
    warm.textures_delta.clear();
    for &key in keys {
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let mut out = ctx.run_ui(input, |ui| app.overlay(ui.ctx()));
        out.textures_delta.clear();
    }
}

/// Insert ▸ Table…, Enter. The form answered only to the pointer, so the
/// Enter did nothing and what was typed next went into its fields.
#[test]
fn enter_inserts_the_table_the_dialog_describes() {
    let mut app = app_with(&["before"]);
    app.run(Command::InsertTable);
    app.table_draft = Some(["3".to_owned(), "2".to_owned()]);
    press_in_dialogs(&mut app, &[egui::Key::Enter]);
    assert!(app.table_draft.is_none(), "Enter closes the dialog");
    let table = app
        .document
        .body
        .iter()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("and inserts the table");
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells.len(), 3);

    // And Escape is still the way out, with nothing inserted.
    let mut app = app_with(&["before"]);
    app.run(Command::InsertTable);
    press_in_dialogs(&mut app, &[egui::Key::Escape]);
    assert!(app.table_draft.is_none());
    assert!(
        !app.document
            .body
            .iter()
            .any(|b| matches!(b, Block::Table(_))),
        "Escape inserts nothing"
    );
}

/// The rig's own sequence — Alt+I, T, Enter — as a test, through the
/// frame the window runs: the menu opens on its letter, the item on its
/// own, the dialog answers Enter in the overlay, and neither letter is
/// typed into the document on the way past.
#[test]
fn insert_table_by_menu_letters_and_enter_puts_a_table_in_the_document() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["before"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'I', 'T');
    assert!(app.table_draft.is_some(), "Alt+I, T opened Insert Table");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.table_draft.is_none(), "Enter answered it");
    let table = app
        .document
        .body
        .iter()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("and the table is in the document");
    assert_eq!((table.rows.len(), table.rows[0].cells.len()), (2, 2));
    let text: String = app
        .document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .collect();
    assert_eq!(text, "before", "no letter of the sequence was typed");
}

/// Format ▸ Watermark… by its letters, the text typed, Enter: what a
/// keyboard user does, and never driven before.
#[test]
fn watermark_by_menu_letters_takes_the_typed_text() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["body text"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'O', 'W');
    assert!(app.watermark_draft.is_some(), "Alt+O, W opened Watermark");
    drive.type_text(&mut app, "DRAFT");
    assert_eq!(
        app.document.paragraphs()[0].text(),
        "body text",
        "nothing typed at the box reached the document"
    );
    assert_eq!(
        app.watermark_draft.as_ref().map(|d| d.text.as_str()),
        Some("DRAFT"),
        "typing after opening the box goes into its text field"
    );
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.watermark_draft.is_none(), "Enter applied it");
    assert!(
        !app.document.headers.is_empty(),
        "a header carries the watermark"
    );
    assert_eq!(app.document.paragraphs()[0].text(), "body text");
}

/// File ▸ Save As a Markdown file asks first, and Enter is "Save".
#[test]
fn saving_as_markdown_asks_and_enter_writes_the_file() {
    let drive = ui_kit::drive::Driver::new();
    let dir = scratch("markdown-by-key");
    let target = dir.join("note.md");
    let mut app = app_with(&["A heading", "Some words."]);
    drive.settle(&mut app);
    assert!(
        !app.save_to(target.clone()),
        "not written before the question"
    );
    assert!(matches!(app.pending, Some(Pending::Lossy(..))));
    drive.settle(&mut app);
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.pending.is_none(), "Enter answered the question");
    let text = std::fs::read_to_string(&target).expect("the file was written");
    assert!(text.contains("Some words."), "{text}");
}

/// Every corpus document, opened as the command line opens one, laid out
/// and drawn in a frame. The readers have their own tests; this is the
/// application around them, which is where a document that reads fine
/// has panicked before.
#[test]
fn every_corpus_document_opens_and_draws_in_a_frame() {
    let drive = ui_kit::drive::Driver::new();
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let mut seen = 0;
    for kind in ["docx", "doc", "odt"] {
        let Ok(entries) = std::fs::read_dir(corpus.join(kind)) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let mut app = Scriva::opening(path.clone());
            drive.settle(&mut app);
            drive.press(&mut app, "ctrl+End");
            drive.settle(&mut app);
            // A `.doc` says it was opened as a copy, which is a notice
            // and not a fault; anything that could not be done is.
            if let Some((title, why)) = &app.message {
                assert!(!title.starts_with("Cannot"), "{name}: {title}: {why}");
            }
            assert!(!app.view.pages().is_empty(), "{name}: no page was laid");
            seen += 1;
        }
    }
    assert!(seen >= 25, "only {seen} documents");
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
    assert!(
        app.margins_draft.is_some(),
        "Alt+L, M, C opened Custom Margins"
    );
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.margins_draft.is_none(), "Enter closed Custom Margins");
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
    check("Custom Margins", Command::CustomMargins, "2", &|app| {
        app.margins_draft.as_ref().map(|d| d[0].clone())
    });
    check("Column Width", Command::ColumnWidth, "3", &|app| {
        app.column_draft.clone()
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Insert ▸ Footer ▸ Edit Footer by its letters, words typed, Insert ▸
/// Page Number ▸ Plain Number by its letters, Escape: the footer holds
/// the words and the field, and the caret is back in the text.
#[test]
fn a_footer_with_a_page_number_by_menu_letters() {
    let drive = ui_kit::drive::Driver::new();
    let refused_before = ui_kit::headless::choosers_refused();
    let mut app = app_with(&["body text"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'I', 'F');
    drive.press(&mut app, "E");
    drive.settle(&mut app);
    assert!(
        matches!(app.scope, wp_model::Scope::Chrome(_)),
        "Alt+I, F, E opened the footer"
    );
    drive.type_text(&mut app, "Page ");
    drive.menu(&mut app, 'I', 'N');
    drive.press(&mut app, "P");
    drive.settle(&mut app);
    // The field's cached result reads as text, so the footer says "Page 1".
    assert_eq!(
        app.paragraph_text(0),
        "Page 1",
        "the words are in the footer"
    );
    assert!(app.asking.is_none(), "and no chooser was opened by the P");
    assert_eq!(ui_kit::headless::choosers_refused(), refused_before);
    let footer = app
        .document
        .paragraphs_in(app.scope)
        .first()
        .cloned()
        .expect("the footer has a paragraph");
    assert!(
        footer
            .runs()
            .iter()
            .flat_map(|run| run.content.iter())
            .any(|piece| matches!(piece, wp_model::doc::Piece::FieldStart { .. })),
        "and a page field"
    );
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert_eq!(app.scope, wp_model::Scope::Body, "Escape left the footer");
    assert_eq!(app.document.paragraphs()[0].text(), "body text");
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
    assert!(app.margins_draft.is_none());
    assert_eq!(
        app.document.section.margins.top,
        Twips(2880),
        "a two-inch top margin"
    );
}

/// The Table menu by its letters on an inserted table: Borders ▸ None
/// and ▸ All, Shading ▸ No Fill, Merge Cells on one cell, Cell Margins…
/// answered as it stands.
#[test]
fn the_table_menu_by_its_letters() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["before"]);
    app.insert_table(2, 2);
    drive.settle(&mut app);
    let ruled =
        |app: &Scriva| {
            app.document
                .body
                .iter()
                .find_map(|block| match block {
                    // Borders ▸ None writes a border of style None on the
                    // table and clears the cells' own, so "ruled" is a top
                    // border with a style.
                    Block::Table(table) => Some(
                        table.props.borders.top.is_some_and(|border| {
                            border.style != wp_model::prop::BorderStyle::None
                        }) || table.rows[0].cells[0].props.borders.top.is_some(),
                    ),
                    _ => None,
                })
                .expect("a table")
        };
    assert!(ruled(&app), "inserted ruled");
    drive.menu(&mut app, 'A', 'B');
    drive.press(&mut app, "N");
    drive.settle(&mut app);
    assert!(!ruled(&app), "Borders ▸ None took the rules off");
    drive.menu(&mut app, 'A', 'B');
    drive.press(&mut app, "A");
    drive.settle(&mut app);
    assert!(ruled(&app), "Borders ▸ All put them back");
    drive.menu(&mut app, 'A', 'S');
    drive.press(&mut app, "N");
    drive.settle(&mut app);
    // Merge Cells with the caret in one cell says there is nothing to
    // merge, and Enter dismisses the saying.
    drive.menu(&mut app, 'A', 'G');
    drive.settle(&mut app);
    assert_eq!(
        app.message.as_ref().map(|(title, _)| title.as_str()),
        Some("Nothing to merge")
    );
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.message.is_none(), "Enter dismissed it");
    drive.menu(&mut app, 'A', 'M');
    assert!(
        app.cell_margin_draft.is_some(),
        "Alt+A, M opened Cell Margins"
    );
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.cell_margin_draft.is_none());
    assert!(app.message.is_none(), "{:?}", app.message);
    let text: String = app
        .document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .collect();
    assert!(text.contains("before"), "{text:?}");
}

/// Save As a plain text file asks first, and Enter writes it.
#[test]
fn saving_as_plain_text_asks_and_enter_writes_the_file() {
    let drive = ui_kit::drive::Driver::new();
    let dir = scratch("text-by-key");
    let target = dir.join("note.txt");
    let mut app = app_with(&["One line.", "Another."]);
    drive.settle(&mut app);
    assert!(!app.save_to(target.clone()));
    drive.settle(&mut app);
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.pending.is_none());
    let text = std::fs::read_to_string(&target).expect("the file was written");
    assert!(
        text.contains("One line.") && text.contains("Another."),
        "{text}"
    );
}

/// Insert ▸ Picture… by its letters in a test: the chooser is refused and
/// counted, never put on the developer's screen, and the document is as
/// it was. On Linux the chooser is the desktop portal's window, and a
/// test that reached one put it on the screen of whoever ran the tests.
#[test]
fn a_test_that_reaches_a_file_chooser_does_not_open_one() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["text"]);
    drive.settle(&mut app);
    let before = ui_kit::headless::choosers_refused();
    drive.menu(&mut app, 'I', 'P');
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert!(
        ui_kit::headless::choosers_refused() > before,
        "the chooser was asked for"
    );
    assert!(app.asking.is_none(), "and answered cancelled at once");
    assert_eq!(app.document.paragraphs()[0].text(), "text");
    let config = ui_kit::paths::config_dir_path(SCRIVA).expect("a directory");
    assert!(
        config.starts_with(std::env::temp_dir()),
        "a test's recent files are not the user's: {}",
        config.display()
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
    let menus = drive.every_menu(&mut app, "FEVOPLRIAS");
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

/// One whole frame of the window's body, with `events` as its input.
fn frame_of(app: &mut Scriva, ctx: &egui::Context, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        egui::Panel::top("test-toolbar").show(ui, |ui| app.toolbar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| app.ui(ui));
    });
    out.textures_delta.clear();
}

fn key_event(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The most ordinary find and replace there is — type the word, Enter,
/// Tab to the other field, type its replacement — edited the document
/// instead: the Tab left the bar for an arrow button, the bar said it no
/// longer held the keyboard, and the same Tab was typed over the match.
#[test]
fn tab_in_the_find_bar_goes_to_the_replace_field_and_not_into_the_document() {
    let mut app = app_with(&["the quick fox and the quick dog"]);
    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    frame_of(&mut app, &ctx, vec![]);
    app.run(Command::Replace);
    frame_of(&mut app, &ctx, vec![]);
    frame_of(&mut app, &ctx, vec![egui::Event::Text("quick".into())]);
    frame_of(&mut app, &ctx, vec![key_event(egui::Key::Enter)]);
    frame_of(&mut app, &ctx, vec![]);
    assert_eq!(
        app.selected_text().as_deref(),
        Some("quick"),
        "Enter selects the first match"
    );
    frame_of(&mut app, &ctx, vec![key_event(egui::Key::Tab)]);
    frame_of(&mut app, &ctx, vec![]);
    frame_of(&mut app, &ctx, vec![egui::Event::Text("slow".into())]);
    frame_of(&mut app, &ctx, vec![]);

    assert_eq!(
        app.document.text(),
        "the quick fox and the quick dog",
        "neither the Tab nor the typing reached the document"
    );
    let finder = app.finder.as_ref().expect("the bar is still open");
    assert_eq!(finder.query, "quick");
    assert_eq!(finder.replacement, "slow", "the Tab went to Replace with");
}

/// Replace All with an empty field was reported as doing nothing. It
/// deletes every match, as Word's does; what was seen doing nothing was a
/// click that the moving count had put on Replace, below.
#[test]
fn a_replace_all_with_nothing_to_put_back_deletes_every_match() {
    let mut app = app_with(&["a quick, quick fox"]);
    app.finder = Some(Finder {
        query: "quick".into(),
        with_replace: true,
        ..Finder::default()
    });
    app.replace_all();
    assert_eq!(app.document.text(), "a ,  fox");
    assert_eq!(
        app.finder.as_ref().unwrap().note.as_deref(),
        Some("Replaced 2")
    );
}

/// "Replace All does nothing": the click had landed on Replace. The count
/// sat in a slot as wide as its words, and its words change with every
/// step, so each control after it moved along the bar — the pointer aimed
/// at Replace All was over Replace by the time it pressed, Replace only
/// found the first match, and the shorter count that left slid Replace
/// All back under the pointer. Nothing after the count moves now.
#[test]
fn the_find_bar_controls_stay_put_while_the_count_changes() {
    let mut app = app_with(&["the quick fox and the quick dog"]);
    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    frame_of(&mut app, &ctx, vec![]);
    app.run(Command::Replace);
    frame_of(&mut app, &ctx, vec![]);
    frame_of(&mut app, &ctx, vec![egui::Event::Text("quick".into())]);
    frame_of(&mut app, &ctx, vec![]);
    let field = egui::Id::new("scriva-find-replacement");
    let before = ctx.read_response(field).expect("the field is drawn").rect;
    frame_of(&mut app, &ctx, vec![key_event(egui::Key::Enter)]);
    frame_of(&mut app, &ctx, vec![]);
    assert!(app.selected_text().is_some(), "the count now says 1 of 2");
    let after = ctx.read_response(field).expect("still drawn").rect;
    assert_eq!(before, after, "and Replace with did not move");
}

/// One whole frame of the window as the shell lays it out — dialogs, the
/// menu bar and toolbar, and the page — with `events` as its input.
fn window_frame(app: &mut Scriva, ctx: &egui::Context, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        app.overlay(ui.ctx());
        egui::Panel::top("test-toolbar").show(ui, |ui| app.toolbar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| app.ui(ui));
    });
    out.textures_delta.clear();
}

/// Observed once on a keystroke drive and then reproduced: letters typed
/// after a table were eaten, the rest struck through, and a menu title
/// sat highlighted with no menu open. Nothing held the keyboard, so each
/// Tab also walked egui's focus along the menu bar; the title it stopped
/// on took the next Enter as a click and opened its menu, and the menu
/// took the typing as its commands.
#[test]
fn tab_and_enter_stay_in_the_document_when_nothing_else_has_the_keyboard() {
    let mut app = app_with(&["text"]);
    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    window_frame(&mut app, &ctx, vec![]);
    window_frame(&mut app, &ctx, vec![]);
    for _ in 0..5 {
        window_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    window_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    window_frame(&mut app, &ctx, vec![]);
    assert!(
        !egui::Popup::is_any_open(&ctx),
        "Enter opened a menu instead of ending the paragraph"
    );
    assert_eq!(
        app.document.paragraphs().len(),
        2,
        "Enter split the paragraph"
    );
    assert_eq!(
        ctx.memory(|m| m.focused()),
        app.surface_id,
        "and the page holds the keyboard"
    );
}

/// The keystroke drive's own sequence: A1, Tab, B1, Tab, A2, Tab, B2, Tab,
/// A3. Every Tab was a tab character, and all of it landed in the first
/// cell — a table could only be filled in by clicking every cell.
#[test]
fn tab_fills_a_table_in_cell_by_cell_and_adds_a_row_at_the_end() {
    let mut app = app_with(&["after"]);
    app.insert_table(2, 2);
    let cells = |app: &Scriva| -> Vec<Vec<String>> {
        let Some(Block::Table(table)) = app
            .document
            .body
            .iter()
            .find(|block| matches!(block, Block::Table(_)))
        else {
            panic!("the table is there");
        };
        table
            .rows
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.text()).collect())
            .collect()
    };
    for (at, word) in ["A1", "B1", "A2", "B2", "A3"].into_iter().enumerate() {
        if at > 0 {
            app.key(egui::Key::Tab, egui::Modifiers::NONE);
        }
        app.type_text(word);
    }
    assert_eq!(
        cells(&app),
        [["A1", "B1"], ["A2", "B2"], ["A3", ""]],
        "one cell each, and a third row from the Tab in the last cell"
    );

    // Shift+Tab goes back and selects the cell's text, so typing replaces it.
    app.key(egui::Key::Tab, egui::Modifiers::SHIFT);
    assert_eq!(app.selected_text().as_deref(), Some("B2"));
    app.type_text("X");
    // And Ctrl+Tab is how a tab gets into a cell.
    app.key(egui::Key::Tab, egui::Modifiers::COMMAND);
    assert_eq!(cells(&app)[1], ["A2", "X\t"]);

    // The row the Tab added comes out with an undo of its own.
    for _ in 0..4 {
        app.run(Command::Undo);
    }
    assert_eq!(cells(&app), [["A1", "B1"], ["A2", "B2"]]);
}

/// File ▸ New, three lines, Save As, open what was saved. The window drew
/// the lines single-spaced with nothing after them; the file said eight
/// points after and a line of 1.08, because the package authored for a new
/// document wrote Word 2013's defaults whatever the document said, and the
/// document came back a third taller from its own first save.
#[test]
fn a_new_document_reopens_spaced_as_it_was_drawn() {
    let dir = scratch("new-document-spacing");
    let target = dir.join("new.docx");
    let mut app = Scriva::new();
    app.type_text("one");
    app.key(egui::Key::Enter, egui::Modifiers::NONE);
    app.type_text("two café");
    app.key(egui::Key::Enter, egui::Modifiers::NONE);
    app.type_text("three");
    let drawn = fixed_lines(&app.document);
    assert!(app.save_to(target.clone()), "the save reports success");

    let mut reopened = Scriva::new();
    reopened.open_path(&target);
    assert_eq!(reopened.document.paragraphs().len(), 3);
    assert_eq!(
        fixed_lines(&reopened.document),
        drawn,
        "every line stands where it stood before the save"
    );
}

/// Every line of every page, as (page, paragraph, y), laid out with the
/// fixed-width shaper.
///
/// Fixed rather than the window's shaper, because the window's depends on
/// the machine: a face this one does not have is drawn in its substitute
/// on both sides of a save, and a substitute hides exactly what a round
/// trip can lose. Every glyph half its point size cannot hide a size, a
/// space, an indent, a tab stop or a number.
fn fixed_lines(document: &Document) -> Vec<(usize, usize, i64)> {
    let theme = document.theme.clone();
    let notes = wp_layout::NoteMarks::of(document);
    let contents = wp_layout::field::Contents::of(document);
    let ctx = wp_layout::inline::Context {
        theme: &theme,
        styles: &document.styles,
        notes: &notes,
        contents: &contents,
        default_tab: document.settings.default_tab_stop,
        no_leading: document.settings.no_leading,
        close_up_justified: document.settings.compatibility_mode >= 15,
        no_tab_for_hanging_indent: document.settings.no_tab_for_hanging_indent,
        ..Default::default()
    };
    wp_layout::block::layout(document, &ctx, &mut wp_layout::Fixed)
        .iter()
        .enumerate()
        .flat_map(|(page, laid)| {
            laid.content
                .iter()
                .filter_map(move |placement| match &placement.kind {
                    wp_layout::block::Placed::Line { paragraph, .. } => {
                        Some((page, *paragraph, (placement.y * 100.0).round() as i64))
                    }
                    _ => None,
                })
        })
        .collect()
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

/// A `.doc` is saved as `.docx` so that it can be written at all, and the
/// user is told so: the copy is meant to be the same document. It was not.
/// A sixteen-page specification came back as twenty-six, its headings
/// without their numbers and its contents without their leaders — Word
/// 2013's defaults over a Word 97 document, and every style written as its
/// chain and a face. Every corpus `.doc` now resolves and lays out the
/// same before its first save and after its reopening, and its styles go
/// by the ids Word would give them.
#[test]
fn a_doc_saved_as_docx_reopens_as_the_same_document() {
    let dir = scratch("doc-as-docx");
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/doc");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&folder)
        .expect("the corpus has .doc files")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "doc"))
        .collect();
    names.sort();
    assert!(!names.is_empty());
    for source in names {
        let name = source.file_stem().unwrap().to_string_lossy().into_owned();
        let mut app = Scriva::new();
        app.open_path(&source);
        let lines = fixed_lines(&app.document);
        let properties = resolved(&app.document);
        let target = dir.join(format!("{name}.docx"));
        assert!(
            app.save_to(target.clone()),
            "{name}: the save reports success"
        );

        let mut reopened = Scriva::new();
        reopened.open_path(&target);
        let again = resolved(&reopened.document);
        assert_eq!(again.len(), properties.len(), "{name}: every paragraph");
        for (at, (came, went)) in again.iter().zip(&properties).enumerate() {
            assert_eq!(came, went, "{name}: paragraph {at} resolves as it did");
        }
        assert_eq!(
            fixed_lines(&reopened.document),
            lines,
            "{name}: every line where it was, on the page it was on"
        );
        for (_, style) in reopened.document.styles.iter() {
            assert!(
                !style.id.contains([' ', ',']),
                "{name}: {:?} is not an id Word would make",
                style.id
            );
        }
    }
}

/// The keystroke drive's repro: New, Insert ▸ Table…, Insert, Save As
/// `.odt`, open it again. The table came back as unruled text with no
/// widths, and the page break was gone, with nothing said.
#[test]
fn a_table_and_a_page_break_survive_a_save_as_odt() {
    let dir = scratch("table-and-break-as-odt");
    let target = dir.join("t.odt");
    let mut app = Scriva::new();
    app.type_text("Before.");
    app.run(Command::PageBreak);
    app.type_text("After the break.");
    app.insert_table(2, 2);
    app.type_text("A1");
    let grid = match app
        .document
        .body
        .iter()
        .find(|b| matches!(b, Block::Table(_)))
    {
        Some(Block::Table(table)) => table.grid.clone(),
        _ => panic!("the table is in"),
    };
    assert!(app.save_to(target.clone()), "the save reports success");

    let mut reopened = Scriva::new();
    reopened.open_path(&target);
    let Some(Block::Table(table)) = reopened
        .document
        .body
        .iter()
        .find(|b| matches!(b, Block::Table(_)))
    else {
        panic!("the table came back as a table");
    };
    assert_eq!(table.grid, grid, "as wide as it was");
    assert!(
        table.rows[0].cells[0].props.borders.top.is_some(),
        "and ruled"
    );
    assert_eq!(table.rows[0].cells[0].text(), "A1");
    let broken = reopened.document.paragraphs().iter().any(|paragraph| {
        paragraph
            .runs()
            .iter()
            .flat_map(|run| run.content.iter())
            .any(|piece| {
                matches!(
                    piece,
                    wp_model::doc::Piece::Break(wp_model::doc::Break::Page)
                )
            })
    });
    assert!(broken, "the page break is still there");
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

#[test]
fn a_watermark_is_put_in_a_header_the_document_did_not_have() {
    let mut app = app_with(&["body text"]);
    assert!(app.document.headers.is_empty(), "nothing to start with");
    app.apply_watermark(&watermark_draft("CONFIDENTIAL"));

    let shape = watermark_in(&app.document).expect("the watermark is there");
    assert_eq!(&*shape.text, "CONFIDENTIAL");
    assert_eq!(shape.rotation, 315.0, "diagonal, as Word's own is");
    // A header body with no part and no relationship: the writer gives it
    // both, and the section now names it.
    let header = app.document.headers.first().expect("a header was made");
    assert!(header.part.is_none() && header.rel.is_none());
    assert!(!header.footer);
    assert_eq!(app.document.section.headers.len(), 1);
}

#[test]
fn the_watermark_is_stamped_only_on_the_headers_a_page_will_show() {
    // A document commonly names three headers while the settings that
    // would show two of them are off. Word stamps the one that is drawn
    // and leaves the other parts empty — measured against a watermark
    // Word wrote itself.
    let mut app = app_with(&["body text"]);
    for (index, kind) in [
        wp_model::HeaderKind::Default,
        wp_model::HeaderKind::First,
        wp_model::HeaderKind::Even,
    ]
    .into_iter()
    .enumerate()
    {
        let id = wp_model::HeaderId(index as u32);
        app.document.headers.push(wp_model::doc::HeaderFooter {
            id,
            part: None,
            rel: None,
            footer: false,
            content: Vec::new(),
        });
        app.document.section.headers.push(wp_model::HeaderRef {
            kind,
            body: id,
            rel: None,
        });
    }
    app.apply_watermark(&watermark_draft("DRAFT"));
    let carrying = |app: &Scriva| -> Vec<u32> {
        app.document
            .headers
            .iter()
            .filter(|header| shape_words_in(&header.content).is_some())
            .map(|header| header.id.0)
            .collect()
    };
    assert_eq!(carrying(&app), vec![0], "the default header alone");

    // Turn on the two settings that put the others on a page, and they
    // are stamped too — a title page without the watermark is the page
    // that most needed it.
    app.document.section.title_page = true;
    app.document.settings.even_and_odd_headers = true;
    app.apply_watermark(&watermark_draft("DRAFT"));
    assert_eq!(carrying(&app), vec![0, 1, 2], "all three, once each");
}

#[test]
fn a_second_watermark_replaces_the_first_rather_than_joining_it() {
    let mut app = app_with(&["body text"]);
    app.apply_watermark(&watermark_draft("DRAFT"));
    app.apply_watermark(&watermark_draft("FINAL"));
    let shapes: Vec<String> = app
        .document
        .headers
        .iter()
        .flat_map(|header| &header.content)
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            _ => None,
        })
        .flat_map(|paragraph| paragraph.drawings())
        .filter_map(|drawing| drawing.text.as_deref())
        .map(|text| text.text.to_string())
        .collect();
    assert_eq!(shapes, vec!["FINAL".to_owned()]);
}

#[test]
fn removing_a_watermark_takes_its_paragraph_with_it_and_undoes_in_one_step() {
    let mut app = app_with(&["body text"]);
    app.apply_watermark(&watermark_draft("CONFIDENTIAL"));
    let with = app.document.headers[0].content.len();
    assert_eq!(with, 1, "one paragraph, holding the shape");

    app.apply_watermark(&watermark_draft(""));
    assert!(watermark_in(&app.document).is_none(), "it is gone");
    assert!(
        app.document.headers[0].content.is_empty(),
        "and so is the empty line it would have left behind"
    );

    app.run(Command::Undo);
    assert_eq!(
        watermark_in(&app.document).map(|shape| shape.text.to_string()),
        Some("CONFIDENTIAL".to_owned()),
        "one undo brings it back"
    );
}

#[test]
fn a_watermark_that_was_read_out_of_a_header_is_offered_back_to_be_edited() {
    let mut app = app_with(&["body text"]);
    app.apply_watermark(&WatermarkDraft {
        text: "SAMPLE".to_owned(),
        font: "Verdana".to_owned(),
        color: "1E6F5C".to_owned(),
        diagonal: false,
        existing: false,
    });
    app.open_watermark_dialog();
    let draft = app.watermark_draft.clone().expect("the box is open");
    assert_eq!(draft.text, "SAMPLE");
    assert_eq!(draft.font, "Verdana");
    assert_eq!(draft.color, "1E6F5C");
    assert!(!draft.diagonal, "an unturned shape reads back as flat");
    assert!(draft.existing, "so the box offers to remove it");
}

#[test]
fn a_watermarks_size_is_taken_from_the_page_it_will_be_stamped_on() {
    let mut app = app_with(&["body text"]);
    app.apply_watermark(&watermark_draft("CONFIDENTIAL"));
    let drawing = app.document.headers[0]
        .content
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => paragraph.drawings().first().copied(),
            _ => None,
        })
        .next()
        .expect("the shape");
    // Turned through forty-five degrees, a shape's bounding box is
    // `(w + h) / root two` each way, so the widest that fits the text area
    // is what this comes to. Word's own is 527.75 by 131.95 on the same
    // page; without a shaper here the fallback proportion decides, and
    // both numbers land within a couple of points of Word's.
    assert!(
        (drawing.extent.0.points() - 529.5).abs() < 2.0,
        "width {} is not the width that fits",
        drawing.extent.0.points()
    );
    assert!(
        (drawing.extent.1.points() - 132.4).abs() < 2.0,
        "height {} does not keep the proportion",
        drawing.extent.1.points()
    );
}

#[test]
fn naming_a_font_speaks_for_the_latin_slots_and_silences_the_theme() {
    let mut app = app_with(&["hello"]);
    app.run(Command::SelectAll);
    // The theme reference modern Word puts on nearly every run. It
    // outranks a cached name, so the command must take it away too or
    // the choice would silently lose to the theme.
    app.format_runs(|props| {
        props.fonts.ascii_theme = Some(wp_model::prop::ThemeFont::MinorHighAnsi)
    });
    app.run(Command::Font("Verdana".to_owned()));
    assert!(app.probe_runs(|props| {
        props.fonts.ascii.as_deref() == Some("Verdana")
            && props.fonts.high_ansi.as_deref() == Some("Verdana")
            && props.fonts.ascii_theme.is_none()
    }));
}

#[test]
fn the_palette_and_the_hex_box_agree_on_what_a_colour_is() {
    let mut app = app_with(&["hello"]);
    app.run(Command::SelectAll);
    app.run(Command::Color(wp_model::Color::Rgb([0x33, 0x33, 0x99])));
    assert!(app.probe_runs(|props| props.color == Some(wp_model::Color::Rgb([0x33, 0x33, 0x99]))));
    // The dialog parses with the same reader a file's `w:val` gets, so
    // `#333399` and `333399` and `auto` all mean what they mean there.
    assert_eq!(
        wp_model::Color::from_val("#333399"),
        Some(wp_model::Color::Rgb([0x33, 0x33, 0x99]))
    );
}

#[test]
fn the_highlighter_erases_by_removing_rather_than_writing_none() {
    let mut app = app_with(&["hello"]);
    app.run(Command::SelectAll);
    app.run(Command::Highlight(wp_model::Highlight::Yellow));
    assert!(app.probe_runs(|props| props.highlight == Some(wp_model::Highlight::Yellow)));
    app.run(Command::Highlight(wp_model::Highlight::None));
    assert!(app.probe_runs(|props| props.highlight.is_none()));
}

#[test]
fn merging_cells_spans_their_columns_and_keeps_their_paragraphs_in_order() {
    let mut app = app_with(&["text"]);
    app.insert_table(2, 3);
    // The caret is in the first cell; the cells' paragraphs are 0, 1, 2
    // across the first row. Fill the first and the third, leave the
    // middle blank.
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    app.type_text("left");
    app.selection = Selection::at(Caret {
        paragraph: 2,
        offset: 0,
    });
    app.type_text("right");
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 0,
        },
        head: Caret {
            paragraph: 2,
            offset: 0,
        },
    };
    app.run(Command::MergeCells);
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is the first block");
    };
    let row = &table.rows[0];
    assert_eq!(row.cells.len(), 1, "one cell where three were");
    let cell = &row.cells[0];
    assert_eq!(cell.props.grid_span, 3);
    let whole: i32 = table.grid.iter().map(|t| t.0).sum();
    assert_eq!(
        cell.props.width,
        wp_model::table::Width::Fixed(Twips(whole))
    );
    // The blank middle cell left no blank line behind it.
    assert_eq!(wp_model::doc::text_of(&cell.content), "left\nright");
    assert_eq!(table.rows[1].cells.len(), 3, "the other row is untouched");
    assert_eq!(app.caret().paragraph, 0);

    app.run(Command::Undo);
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is still the first block");
    };
    assert_eq!(table.rows[0].cells.len(), 3);
}

#[test]
fn merging_needs_a_selection_across_cells_of_one_row() {
    let mut app = app_with(&["text"]);
    app.insert_table(2, 2);
    // One cell: nothing to merge.
    app.run(Command::MergeCells);
    assert!(app.message.is_some(), "a caret in one cell is told why");
    app.message = None;
    // Two cells in different rows: not a merge either.
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 0,
        },
        head: Caret {
            paragraph: 2,
            offset: 0,
        },
    };
    app.run(Command::MergeCells);
    assert!(app.message.is_some());
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is the first block");
    };
    assert!(table.rows.iter().all(|row| row.cells.len() == 2));
}

#[test]
fn a_border_colour_reaches_every_rule_and_draws_rules_where_there_were_none() {
    let grey = wp_model::Color::Rgb([0xC0, 0xC0, 0xC0]);
    let mut app = app_with(&["text"]);
    app.insert_table(2, 2);
    app.run(Command::BorderColor(grey));
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is the first block");
    };
    let b = &table.props.borders;
    for edge in [b.top, b.start, b.bottom, b.end, b.inside_h, b.inside_v] {
        let edge = edge.expect("every rule is stated");
        assert_eq!(edge.color, Some(grey));
        assert_eq!(edge.style, wp_model::prop::BorderStyle::Single);
    }

    // With the rules off, a colour brings them back in that colour: a
    // colour on no line would be a menu choice that did nothing.
    app.run(Command::TableBorders(false));
    app.run(Command::BorderColor(grey));
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is still the first block");
    };
    let top = table.props.borders.top.expect("ruled again");
    assert_eq!(top.style, wp_model::prop::BorderStyle::Single);
    assert_eq!(top.color, Some(grey));
}

#[test]
fn the_paragraph_box_applies_only_the_fields_that_were_changed() {
    let mut app = app_with(&["one", "two"]);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 0,
        },
        head: Caret {
            paragraph: 1,
            offset: 0,
        },
    };
    app.format_paragraphs(|props| props.spacing.before = Some(Twips(120)));
    app.open_paragraph_dialog();
    let (opened, _) = app.paragraph_draft.clone().expect("the box is open");
    assert_eq!(opened.before, "6", "six points, not 6.00");
    let mut draft = opened.clone();
    draft.after = "3".into();
    draft.left = "0.17".into();
    draft.hanging = "0.17".into();
    app.apply_paragraph(&opened, &draft);
    for paragraph in app.document.paragraphs().iter() {
        assert_eq!(
            paragraph.props.spacing.before,
            Some(Twips(120)),
            "untouched"
        );
        assert_eq!(paragraph.props.spacing.after, Some(Twips(60)));
        assert_eq!(paragraph.props.indent.start, Some(Twips(245)));
        assert_eq!(paragraph.props.indent.hanging, Some(Twips(245)));
    }
    // A field blanked takes the value away rather than writing zero.
    let opened = draft.clone();
    let mut draft = opened.clone();
    draft.before = String::new();
    app.apply_paragraph(&opened, &draft);
    assert!(app
        .document
        .paragraphs()
        .iter()
        .all(|p| p.props.spacing.before.is_none()));
}

#[test]
fn turning_borders_off_writes_none_rather_than_nothing() {
    let mut app = app_with(&["text"]);
    app.insert_table(2, 2);
    // The caret landed in the first cell; the command must find the table
    // from there.
    app.run(Command::TableBorders(false));
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is the first block");
    };
    // Explicit `none`, because an absent border is an inherited one: a
    // table style could rule the edges right back.
    let edge = table.props.borders.top.expect("the edge is stated");
    assert_eq!(edge.style, wp_model::prop::BorderStyle::None);
    assert!(table
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .all(|cell| cell.props.borders == wp_model::table::TableBorders::default()));

    app.run(Command::Undo);
    let Block::Table(table) = &app.document.body[0] else {
        panic!("still the first block");
    };
    let edge = table.props.borders.top.expect("the rule is back");
    assert_eq!(edge.style, wp_model::prop::BorderStyle::Single, "one undo");
}

#[test]
fn shading_lands_on_the_cell_the_caret_is_in() {
    let mut app = app_with(&["text"]);
    app.insert_table(2, 2);
    app.run(Command::TableShading(Some([0x92, 0xD0, 0x50])));
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is the first block");
    };
    let shading = table.rows[0].cells[0]
        .props
        .shading
        .as_ref()
        .expect("the caret's cell is filled");
    assert_eq!(
        shading.background(),
        Some(wp_model::Color::Rgb([0x92, 0xD0, 0x50]))
    );
    assert!(
        table.rows[0].cells[1].props.shading.is_none(),
        "and its neighbour is not"
    );
}

#[test]
fn a_column_width_moves_the_grid_and_the_cells_together() {
    let mut app = app_with(&["text"]);
    app.insert_table(2, 2);
    app.apply_column_width(Twips(1440));
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is the first block");
    };
    assert_eq!(table.grid[0], Twips(1440), "the caret's column");
    for row in &table.rows {
        assert_eq!(
            row.cells[0].props.width,
            wp_model::table::Width::Fixed(Twips(1440)),
            "every cell in it restates the width"
        );
    }
}

#[test]
fn cell_margins_state_the_sides_given_and_leave_a_blank_one_to_the_style() {
    use wp_model::table::Width;
    let mut app = app_with(&["text"]);
    app.insert_table(2, 2);
    // The resume's own padding: none above or below, 5.75pt either side.
    app.apply_cell_margins(&[
        "0".to_owned(),
        "5.75".to_owned(),
        "0".to_owned(),
        String::new(),
    ]);
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is the first block");
    };
    let margins = table.props.cell_margins;
    assert_eq!(margins.top, Some(Width::Fixed(Twips(0))));
    assert_eq!(margins.start, Some(Width::Fixed(Twips(115))));
    assert_eq!(margins.bottom, Some(Width::Fixed(Twips(0))));
    assert_eq!(margins.end, None, "a blank side states nothing at all");
}

#[test]
fn the_bullet_press_makes_a_list_and_the_second_press_unmakes_it() {
    let mut app = app_with(&["one", "two"]);
    app.run(Command::SelectAll);
    app.run(Command::Bullets);
    {
        let paragraphs = app.document.paragraphs();
        for paragraph in &paragraphs {
            let reference = paragraph.props.numbering.expect("in a list");
            let level = app
                .document
                .numbering
                .level(reference.num_id, 0)
                .expect("that resolves");
            assert!(matches!(level.format, wp_model::NumFormat::Bullet));
            // The glyph is Symbol's dot, meaningless in any other face —
            // the level must carry the face with it.
            assert_eq!(level.run.fonts.ascii.as_deref(), Some("Symbol"));
        }
    }
    app.run(Command::Bullets);
    let paragraphs = app.document.paragraphs();
    assert!(
        paragraphs.iter().all(|p| p.props.numbering.is_none()),
        "the second press takes them out"
    );
}

#[test]
fn two_presses_are_two_lists_of_one_definition() {
    let mut app = app_with(&["one", "two"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    app.run(Command::Numbers);
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 0,
    });
    app.run(Command::Numbers);
    let paragraphs = app.document.paragraphs();
    let first = paragraphs[0].props.numbering.expect("the first is listed");
    let second = paragraphs[1].props.numbering.expect("and the second");
    assert_ne!(first.num_id, second.num_id, "instances count separately");
    let of = |id| {
        app.document
            .numbering
            .num(id)
            .expect("resolves")
            .abstract_id
    };
    assert_eq!(
        of(first.num_id),
        of(second.num_id),
        "but the definition is shared, as Word's button shares its gallery entry"
    );
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

#[test]
fn opening_a_header_a_document_does_not_have_makes_one_and_puts_the_caret_in_it() {
    let mut app = app_with(&["text"]);
    app.run(Command::EditHeader);
    let wp_model::Scope::Chrome(id) = app.scope else {
        panic!("the caret is in the header");
    };
    let header = app.document.header(id).expect("which the document has");
    assert!(!header.footer);
    let reference = &app.document.section.headers[0];
    assert_eq!(reference.kind, wp_model::HeaderKind::Default);
    assert_eq!(reference.body, id);
    assert!(
        reference.rel.is_none(),
        "no relationship until a save assigns one"
    );

    // Word's Header style, which is what a bare paragraph would miss: the
    // body's space-after would otherwise push the page's own text down to
    // make room under a one-line header, and Tab would walk nowhere.
    let Block::Paragraph(paragraph) = &header.content[0] else {
        panic!("a paragraph");
    };
    assert_eq!(paragraph.props.spacing.before, Some(Twips(0)));
    assert_eq!(paragraph.props.spacing.after, Some(Twips(0)));
    assert_eq!(
        paragraph.props.spacing.line,
        Some(LineSpacing::Multiple(Line240::SINGLE))
    );
    let tabs = paragraph.props.tabs.as_ref().expect("a centre and a right");
    assert_eq!(tabs[0].kind, wp_model::prop::TabKind::Center);
    assert_eq!(tabs[1].kind, wp_model::prop::TabKind::End);
    assert_eq!(tabs[1].position, app.document.section.text_width());
}

#[test]
fn typing_in_an_open_header_leaves_the_body_alone_and_undoes_with_it_open() {
    let mut app = app_with(&["the body"]);
    app.run(Command::EditHeader);
    typed(&mut app, "RESUME / CV");
    let wp_model::Scope::Chrome(id) = app.scope else {
        panic!("still in the header");
    };
    assert_eq!(
        wp_model::doc::text_of(&app.document.header(id).expect("there").content),
        "RESUME / CV"
    );
    assert_eq!(app.document.text(), "the body", "the text is untouched");

    // A paragraph index means one thing in the header and another in the
    // body, so undo has to know which flow made the change — and put the
    // caret back in it.
    app.run(Command::Undo);
    assert_eq!(app.scope, wp_model::Scope::Chrome(id));
    assert_eq!(
        wp_model::doc::text_of(&app.document.header(id).expect("there").content),
        ""
    );
    assert_eq!(app.document.text(), "the body");
}

#[test]
fn a_header_that_holds_a_table_is_still_a_table_after_it_has_been_edited() {
    use wp_model::table::{Cell, Row, Table};
    let mut app = app_with(&["the body"]);
    app.run(Command::EditHeader);
    let wp_model::Scope::Chrome(id) = app.scope else {
        panic!("in the header");
    };
    // A real header: a table of revisions, which is what the box this
    // replaced would have flattened into one line of text.
    app.document.header_mut(id).expect("there").content.insert(
        0,
        Block::Table(Table {
            rows: vec![Row {
                props: Default::default(),
                cells: vec![Cell::new(), Cell::new()],
            }],
            ..Table::new()
        }),
    );
    app.changed();

    // The caret is in the first cell, because the flow walks a table's
    // paragraphs in document order exactly as the body's does.
    app.selection = Selection::at(Caret::default());
    typed(&mut app, "ECN#");
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 0,
    });
    typed(&mut app, "DATE");

    let header = app.document.header(id).expect("there");
    let Block::Table(table) = &header.content[0] else {
        panic!("the table is still a table");
    };
    assert_eq!(
        wp_model::doc::text_of(&table.rows[0].cells[0].content),
        "ECN#"
    );
    assert_eq!(
        wp_model::doc::text_of(&table.rows[0].cells[1].content),
        "DATE"
    );
}

#[test]
fn closing_a_band_puts_the_caret_back_where_it_stood_in_the_text() {
    let mut app = app_with(&["one", "two"]);
    let was = Caret {
        paragraph: 1,
        offset: 2,
    };
    app.selection = Selection::at(was);
    app.run(Command::EditFooter);
    assert!(app.editing_band());
    assert!(
        app.document.headers.iter().any(|header| header.footer),
        "Insert ▸ Footer makes one"
    );
    app.run(Command::CloseChrome);
    assert_eq!(app.scope, wp_model::Scope::Body);
    assert_eq!(app.caret(), was, "and not at the top of the page");
}

#[test]
fn the_band_bar_switches_between_the_two_without_making_a_third() {
    let mut app = app_with(&["text"]);
    app.run(Command::EditHeader);
    app.run(Command::SwitchBand);
    assert!(app.in_footer());
    app.run(Command::SwitchBand);
    assert!(!app.in_footer());
    assert_eq!(app.document.headers.len(), 2, "one header and one footer");
}

#[test]
fn a_page_number_is_a_field_and_not_a_typed_digit() {
    use wp_model::doc::{Inline, Piece};
    let mut app = app_with(&["text"]);
    app.run(Command::EditFooter);
    app.run(Command::InsertPageNumber { of_pages: true });
    let wp_model::Scope::Chrome(id) = app.scope else {
        panic!("in the footer");
    };
    let footer = app.document.header(id).expect("there");
    let Block::Paragraph(paragraph) = &footer.content[0] else {
        panic!("a paragraph");
    };
    let codes: Vec<String> = paragraph
        .content
        .iter()
        .filter_map(|inline| match inline {
            Inline::Run(run) => Some(run),
            _ => None,
        })
        .flat_map(|run| &run.content)
        .filter_map(|piece| match piece {
            Piece::Instruction(code) => Some(code.trim().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(codes, ["PAGE", "NUMPAGES"]);
    assert!(paragraph.text().starts_with("Page "));
}

#[test]
fn removing_a_header_takes_its_references_with_it_and_undoes_in_one_step() {
    let mut app = app_with(&["text"]);
    app.run(Command::EditHeader);
    typed(&mut app, "RESUME / CV");
    app.run(Command::CloseChrome);

    app.run(Command::RemoveChrome { footer: false });
    assert!(app.document.headers.is_empty());
    assert!(
        app.document.section.headers.is_empty(),
        "a reference pointing at nothing is a damaged document"
    );

    app.run(Command::Undo);
    assert_eq!(
        app.document.headers.len(),
        1,
        "bodies and references, together"
    );
    assert_eq!(
        wp_model::doc::text_of(&app.document.headers[0].content),
        "RESUME / CV"
    );
}

#[test]
fn a_click_in_the_margin_is_not_a_place_to_put_the_caret_in_the_text() {
    // While the body is what is being edited, the margins are not part of
    // it; while a band is open, the page is not. A click on the wrong one
    // has to do nothing, which is what the wash over it promises.
    let mut app = laid_app("the body", 400.0);
    let page = &app.view.pages()[0];
    let (top, middle, bottom) = (
        view::Spot {
            page: 0,
            x: page.geometry.start + 1.0,
            y: 4.0,
        },
        view::Spot {
            page: 0,
            x: page.geometry.start + 1.0,
            y: page.geometry.top + 4.0,
        },
        view::Spot {
            page: 0,
            x: page.geometry.start + 1.0,
            y: page.geometry.height - 4.0,
        },
    );
    assert_eq!(app.band_at(top), Some(false));
    assert_eq!(app.band_at(bottom), Some(true));
    assert_eq!(app.band_at(middle), None);
    assert!(!app.click_lands_here(top));
    assert!(app.click_lands_here(middle));

    app.run(Command::EditHeader);
    assert!(
        app.click_lands_here(top),
        "the header is what is being edited"
    );
    assert!(!app.click_lands_here(middle), "and the text is not");
}

#[test]
fn a_comment_asked_for_in_a_header_is_refused_rather_than_put_somewhere_else() {
    // Word, over COM, against a header's own range: "Comments, endnotes
    // and footnotes can only be added to the main story." The trap this
    // closes is the other answer — `revise` used to speak in body
    // positions, so a caret standing in a header wore a number that meant
    // something else there and the comment wrapped whatever body paragraph
    // shared it.
    let mut app = app_with(&["the body"]);
    app.run(Command::EditHeader);
    typed(&mut app, "RESUME / CV");
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 0,
        },
        head: Caret {
            paragraph: 0,
            offset: 6,
        },
    };
    app.run(Command::AddComment);

    assert!(app.draft.is_none(), "no draft opened");
    assert!(app.message.is_some(), "and it said why");
    assert!(app.document.comments.is_empty());
    assert!(app.editing_band(), "the band is left as it was");
}

#[test]
fn a_comment_a_file_carries_in_a_header_is_still_found_and_still_removable() {
    // Nothing in Word writes one, but the schema allows it and a second
    // producer may; a reviewer that cannot see it is a reviewer that lies.
    let mut app = app_with(&["the body"]);
    app.run(Command::EditHeader);
    typed(&mut app, "RESUME / CV");
    let wp_model::Scope::Chrome(id) = app.scope else {
        panic!("in the header");
    };
    let comment = crate::revise::add_comment(
        &mut app.document,
        &mut app.history,
        app.scope,
        Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 0,
                offset: 6,
            },
        },
        "A",
        "A",
        "written by something else",
    );

    assert_eq!(
        crate::revise::comment_at(&app.document, comment).map(|(scope, _)| scope),
        Some(wp_model::Scope::Chrome(id)),
        "found in the flow it is anchored in"
    );
    assert_eq!(
        app.document.text(),
        "the body",
        "and the text carries no anchor of it"
    );
    assert!(crate::revise::delete_comment(
        &mut app.document,
        &mut app.history,
        comment
    ));
    assert_eq!(crate::revise::comment_at(&app.document, comment), None);
}

#[test]
fn a_tracked_change_in_a_header_is_listed_and_settled_where_it_stands() {
    let mut app = app_with(&["the body"]);
    app.run(Command::EditHeader);
    let wp_model::Scope::Chrome(id) = app.scope else {
        panic!("in the header");
    };
    app.run(Command::TrackChanges);
    app.type_text("DRAFT");

    let changes = crate::revise::tracked(&app.document);
    assert_eq!(changes.len(), 1, "one insertion, in the header");
    assert_eq!(changes[0].scope, wp_model::Scope::Chrome(id));

    app.run(Command::AcceptAll);
    assert!(
        crate::revise::tracked(&app.document).is_empty(),
        "and accepting reaches it"
    );
    assert_eq!(
        wp_model::doc::text_of(&app.document.header(id).expect("still there").content),
        "DRAFT",
        "the words survive being accepted"
    );
}

/// Two sections: the first names a header, the second names nothing —
/// which is exactly what Word writes for a section linked to the previous
/// one. The caret lands in the second, because with no layout to ask, the
/// last section is the one a page belongs to.
fn two_sections_with_a_linked_header() -> Scriva {
    let mut app = app_with(&["section one", "section two"]);
    let mut first = wp_model::SectionProps::new();
    first.headers.push(wp_model::HeaderRef {
        kind: wp_model::HeaderKind::Default,
        body: wp_model::HeaderId(0),
        rel: None,
    });
    if let Block::Paragraph(paragraph) = &mut app.document.body[0] {
        paragraph.section = Some(Box::new(first));
    }
    app.document.headers.push(wp_model::doc::HeaderFooter {
        id: wp_model::HeaderId(0),
        part: None,
        rel: None,
        footer: false,
        content: vec![Block::Paragraph(Paragraph::of("INHERITED HEAD"))],
    });
    app.stamp += 1;
    app
}

#[test]
fn a_linked_section_edits_the_band_it_inherits_rather_than_making_a_second() {
    let mut app = two_sections_with_a_linked_header();
    assert_eq!(app.linked_to_previous(false), Some(true));

    app.run(Command::EditHeader);
    assert_eq!(
        app.scope,
        wp_model::Scope::Chrome(wp_model::HeaderId(0)),
        "the band it shows is the band it opens"
    );
    assert_eq!(app.document.headers.len(), 1, "and no second one was made");
}

#[test]
fn breaking_the_link_takes_a_copy_the_section_can_change_on_its_own() {
    // Word's own answer, over COM: unlink a second section's header and
    // the words stay on the page, while the section before it keeps a copy
    // of its own — so the two can then be changed apart.
    let mut app = two_sections_with_a_linked_header();
    app.run(Command::EditHeader);
    app.set_link_to_previous(false);

    assert_eq!(app.linked_to_previous(false), Some(false));
    assert_eq!(app.document.headers.len(), 2, "a copy, not a move");
    let wp_model::Scope::Chrome(own) = app.scope else {
        panic!("in the new band");
    };
    assert_ne!(own, wp_model::HeaderId(0));
    assert_eq!(
        wp_model::doc::text_of(&app.document.header(own).expect("made").content),
        "INHERITED HEAD",
        "the words are still on the page"
    );

    typed(&mut app, "!");
    assert_eq!(
        wp_model::doc::text_of(
            &app.document
                .header(wp_model::HeaderId(0))
                .expect("still there")
                .content
        ),
        "INHERITED HEAD",
        "and the section before it is untouched"
    );

    // Linking again shows the previous section's band and leaves this
    // one's body behind, so flicking the switch back costs no words.
    app.set_link_to_previous(true);
    assert_eq!(app.linked_to_previous(false), Some(true));
    assert_eq!(app.scope, wp_model::Scope::Chrome(wp_model::HeaderId(0)));
}

#[test]
fn the_first_section_is_never_offered_a_link_to_previous() {
    let mut app = app_with(&["only section"]);
    app.run(Command::EditHeader);
    assert_eq!(
        app.linked_to_previous(false),
        None,
        "there is nothing before it"
    );
}

#[test]
fn turning_on_a_first_page_band_moves_the_caret_into_the_band_that_page_now_wants() {
    let mut app = app_with(&["text"]);
    app.run(Command::EditHeader);
    typed(&mut app, "EVERY PAGE");
    let wp_model::Scope::Chrome(default) = app.scope else {
        panic!("in the default header");
    };

    app.set_band_kinds(true, false);
    assert!(app.document.section.title_page);
    let wp_model::Scope::Chrome(first) = app.scope else {
        panic!("still in a header");
    };
    assert_ne!(first, default, "page one wants a band of its own now");
    assert_eq!(
        wp_model::doc::text_of(&app.document.header(first).expect("made").content),
        "",
        "and Word leaves the new one empty"
    );

    // A switch is not a delete: the band it stopped using is still there,
    // and flicking it back finds it rather than making a third.
    app.set_band_kinds(false, false);
    assert_eq!(app.scope, wp_model::Scope::Chrome(default));
    assert_eq!(app.document.headers.len(), 2);

    app.run(Command::Undo);
    assert!(app.document.section.title_page, "one undo per flick");
}

#[test]
fn the_even_page_switch_is_a_document_setting_that_undo_puts_back() {
    let mut app = app_with(&["text"]);
    app.run(Command::EditHeader);
    app.set_band_kinds(false, true);
    assert!(app.document.settings.even_and_odd_headers);
    app.run(Command::Undo);
    assert!(
        !app.document.settings.even_and_odd_headers,
        "the setting rides with the bands it decides the use of"
    );
}

#[test]
fn find_looks_through_the_headers_and_opens_the_one_it_lands_in() {
    let mut app = app_with(&["a spec in the body"]);
    app.run(Command::EditHeader);
    typed(&mut app, "spec no 190A1430");
    app.run(Command::CloseChrome);

    app.finder = Some(crate::find::Finder::new(false));
    if let Some(finder) = &mut app.finder {
        finder.query = "spec".into();
    }
    app.selection = Selection::at(Caret::default());
    app.refresh_matches();
    assert_eq!(app.find_matches.len(), 2, "the text and the header");

    // The first is in the text; the second is in the header, and stepping
    // on to it opens the band the way a double-click on it would.
    app.run(Command::FindNext);
    assert_eq!(app.scope, wp_model::Scope::Body);
    app.run(Command::FindNext);
    assert!(app.editing_band(), "Find Next opened the header");
    assert_eq!(app.selected_text().as_deref(), Some("spec"));

    // And Close still puts the caret back where it stood in the text.
    app.run(Command::CloseChrome);
    assert_eq!(app.scope, wp_model::Scope::Body);
}

#[test]
fn replace_all_reaches_the_headers_too() {
    let mut app = app_with(&["draft copy"]);
    app.run(Command::EditHeader);
    typed(&mut app, "draft");
    app.run(Command::CloseChrome);

    app.finder = Some(crate::find::Finder::new(true));
    if let Some(finder) = &mut app.finder {
        finder.query = "draft".into();
        finder.replacement = "final".into();
    }
    app.replace_all();
    assert_eq!(app.document.text(), "final copy");
    let header = app.document.headers.first().expect("there");
    assert_eq!(wp_model::doc::text_of(&header.content), "final");
}

#[test]
fn the_table_menu_says_so_when_the_caret_is_not_in_a_table() {
    let mut app = app_with(&["just a paragraph"]);
    app.run(Command::TableBorders(false));
    let (title, _) = app.message.as_ref().expect("it says why");
    assert_eq!(title, "Not in a table");
    assert!(!app.history.can_undo(), "and nothing was recorded to undo");
}

#[test]
fn the_layout_commands_change_the_page_and_undo_back() {
    use wp_model::{Orientation, PageMargins};
    let mut app = app_with(&["hello"]);
    let was = app.document.section.page;

    app.run(Command::Orient(Orientation::Landscape));
    assert_eq!(
        app.document.section.page.orientation,
        Orientation::Landscape
    );
    assert_eq!(
        app.document.section.page.width, was.height,
        "the paper turned"
    );
    app.run(Command::Undo);
    assert_eq!(app.document.section.page, was, "and undo turns it back");

    let narrow = PageMargins {
        top: Twips(720),
        bottom: Twips(720),
        start: Twips(720),
        end: Twips(720),
        ..app.document.section.margins
    };
    app.run(Command::Margins(narrow));
    assert_eq!(app.document.section.margins.start, Twips(720));

    app.run(Command::PageBreak);
    let has_break = app.document.paragraphs()[0]
        .runs()
        .iter()
        .flat_map(|run| run.content.iter())
        .any(|piece| {
            matches!(
                piece,
                wp_model::doc::Piece::Break(wp_model::doc::Break::Page)
            )
        });
    assert!(has_break, "Ctrl+Enter left a page break at the caret");
}

#[test]
fn the_word_count_counts_the_way_word_does() {
    use wp_model::doc::{Break, Inline, Piece, Run};
    // A slash splits — Word counts "TCP/IP" as two — but a hyphen does not.
    let mut app = app_with(&["TCP/IP real-time networks"]);
    assert_eq!(app.word_count(), 4);
    // A page break separates the words around it, even though `text()`
    // has nothing to show for it.
    let mut run = Run::of("before");
    run.content.push(Piece::Break(Break::Page));
    run.content.push(Piece::Text("after".into()));
    app.document.body.push(Block::Paragraph(Paragraph {
        content: vec![Inline::Run(run)],
        ..Paragraph::default()
    }));
    assert_eq!(app.word_count(), 6);
}

#[test]
fn a_new_document_is_one_empty_paragraph_with_a_default_style() {
    let app = Scriva::new();
    assert_eq!(app.paragraph_count(), 1);
    assert_eq!(app.word_count(), 0);
    let normal = app
        .document
        .styles
        .default_style(wp_model::StyleKind::Paragraph)
        .expect("a default paragraph style");
    assert_eq!(
        app.document.styles.get(normal).unwrap().id.as_ref(),
        "Normal"
    );
    assert_eq!(
        app.document.settings.compatibility_mode, 15,
        "a new document is a Word 2013 document, which is how it is laid out"
    );
    // Word's own defaults for a new document, which are also what Word
    // lays a file that states none with.
    let defaults = app.document.styles.doc_defaults();
    assert_eq!(defaults.run.size, Some(HalfPoint(24)));
    assert_eq!(defaults.para.spacing.after, Some(Twips(160)));
    assert_eq!(
        defaults.para.spacing.line,
        Some(LineSpacing::Multiple(Line240(278)))
    );
}

/// An app whose view has really been laid out, for keys that ask the
/// layout where the caret is — Home, End and the arrows.
fn laid_app(text: &str, text_width: f64) -> Scriva {
    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let mut app = app_with(&[text]);
    let margins =
        app.document.section.margins.start.points() + app.document.section.margins.end.points();
    app.document.section.page.width = Twips::from_points(text_width + margins);
    let mut shaper = Egui::new(&ctx);
    app.view.refresh(
        &app.document,
        &wp_layout::FieldValues::new(),
        app.stamp,
        &mut shaper,
    );
    app.shaper = Some(shaper);
    app
}

/// The keystroke drive's own sequence: type, Ctrl+Enter, type. The break
/// went in and the caret stayed at its offset — the near side of it — so
/// the second text was typed onto the first page and the second was empty.
#[test]
fn what_is_typed_after_a_page_break_is_on_the_next_page() {
    let mut app = laid_app("", 200.0);
    app.type_text("First.");
    app.run(Command::PageBreak);
    app.type_text("On page two.");
    let shaper = app.shaper.as_mut().expect("laid out");
    app.view.refresh(
        &app.document,
        &wp_layout::FieldValues::new(),
        app.stamp,
        shaper,
    );
    let texts: Vec<String> = app.document.paragraphs().iter().map(|p| p.text()).collect();
    assert_eq!(texts, ["First.", "On page two."]);
    assert_eq!(app.view.pages().len(), 2, "one break, one new page");
    assert_eq!(app.caret_page(), 1, "and the caret is on it");

    app.run(Command::Undo);
    app.run(Command::Undo);
    let texts: Vec<String> = app.document.paragraphs().iter().map(|p| p.text()).collect();
    assert_eq!(texts, ["First."], "the typing, then the break, undo away");
}

/// The keystroke drive's sequence in a table: fill row one, Tab to row two,
/// Ctrl+Enter, type. The break landed in the cell, the layout rightly
/// ignored it (Word does), and the key looked dead. Word splits the table.
#[test]
fn ctrl_enter_in_a_cell_splits_the_table_and_the_rest_is_on_the_next_page() {
    let mut app = laid_app("", 200.0);
    app.insert_table(2, 1);
    app.type_text("first row");
    app.key(egui::Key::Tab, egui::Modifiers::NONE);
    app.type_text("second row");
    app.run(Command::PageBreak);
    app.type_text(" continued");
    let tables: Vec<Vec<String>> = app
        .document
        .body
        .iter()
        .filter_map(|block| match block {
            Block::Table(table) => Some(table.rows.iter().map(|row| row.cells[0].text()).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(
        tables,
        [vec!["first row"], vec!["second row continued"]],
        "two tables, and the typing went on in the caret's cell"
    );
    let shaper = app.shaper.as_mut().expect("laid out");
    app.view.refresh(
        &app.document,
        &wp_layout::FieldValues::new(),
        app.stamp,
        shaper,
    );
    assert_eq!(
        app.view.pages().len(),
        2,
        "the break between the halves takes"
    );
    assert_eq!(app.caret_page(), 1, "and the caret is on the second page");

    app.run(Command::Undo);
    app.run(Command::Undo);
    let tables = app
        .document
        .body
        .iter()
        .filter(|block| matches!(block, Block::Table(_)))
        .count();
    assert_eq!(tables, 1, "the typing, then the split, undo away");
}

/// Measured against Word: an inserted table whose cells stated no width
/// was laid to its content — 28pt wide for a 468pt grid, the words of
/// the second column 219pt from where Scriva had them.
#[test]
fn an_inserted_table_states_every_cells_width_so_word_lays_it_to_the_grid() {
    let mut app = app_with(&["after"]);
    app.insert_table(2, 3);
    let Some(Block::Table(table)) = app
        .document
        .body
        .iter()
        .find(|block| matches!(block, Block::Table(_)))
    else {
        panic!("the table is there");
    };
    let each = table.grid[0];
    assert!(each.0 > 0);
    for cell in table.rows.iter().flat_map(|row| row.cells.iter()) {
        assert_eq!(
            cell.props.width,
            wp_model::table::Width::Fixed(each),
            "every cell states the grid's width"
        );
    }
}

#[test]
fn home_and_end_move_on_the_visual_line_not_the_paragraph() {
    let text = "aa bb cc dd ee ff gg hh";
    // 60 points of text: a few words per line, so the paragraph wraps.
    let mut app = laid_app(text, 60.0);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    app.key(egui::Key::End, egui::Modifiers::NONE);
    let end = app.caret();
    assert_eq!(end.paragraph, 0);
    assert!(
        end.offset > 0 && end.offset < text.len(),
        "End stops at the end of the first visual line, not at {:?}",
        text.len()
    );
    assert_ne!(
        &text[end.offset - 1..end.offset],
        " ",
        "and not beyond the space the wrap ate"
    );

    // Down one visual line, still inside the same paragraph.
    app.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    let down = app.caret();
    assert_eq!(down.paragraph, 0);
    assert!(
        down.offset >= end.offset,
        "the line below, not the line above"
    );

    app.key(egui::Key::Home, egui::Modifiers::NONE);
    let home = app.caret();
    assert_eq!(home.paragraph, 0);
    assert!(
        home.offset >= end.offset,
        "Home goes to this line's start, not the paragraph's"
    );

    // Ctrl+End still means the end of the document.
    app.key(egui::Key::End, egui::Modifiers::COMMAND);
    assert_eq!(app.caret().offset, text.len());
}

fn bulleted(app: &mut Scriva, paragraph: usize) {
    let mut paragraphs = app.document.paragraphs_mut();
    paragraphs[paragraph].props.numbering = Some(wp_model::prop::NumRef {
        num_id: 1,
        level: 0,
    });
}

#[test]
fn enter_on_an_empty_list_item_ends_the_list() {
    let mut app = app_with(&["item", ""]);
    bulleted(&mut app, 0);
    bulleted(&mut app, 1);
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 0,
    });
    app.key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(app.paragraph_count(), 2, "no new paragraph was added");
    assert_eq!(
        app.document.paragraphs()[1].props.numbering,
        None,
        "the bullet is gone instead"
    );
    app.run(Command::Undo);
    assert!(
        app.document.paragraphs()[1].props.numbering.is_some(),
        "and undo puts it back"
    );
}

#[test]
fn backspace_at_the_start_of_a_list_item_takes_the_bullet_first() {
    let mut app = app_with(&["first", "second"]);
    bulleted(&mut app, 1);
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 0,
    });
    app.key(egui::Key::Backspace, egui::Modifiers::NONE);
    assert_eq!(app.paragraph_count(), 2, "nothing joined yet");
    assert_eq!(app.document.paragraphs()[1].props.numbering, None);
    app.key(egui::Key::Backspace, egui::Modifiers::NONE);
    assert_eq!(app.document.text(), "firstsecond", "the second one joins");
}

#[test]
fn tab_at_the_start_of_a_list_item_changes_its_depth() {
    let mut app = app_with(&["item", "plain"]);
    bulleted(&mut app, 0);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    app.key(egui::Key::Tab, egui::Modifiers::NONE);
    assert_eq!(
        app.document.paragraphs()[0].props.numbering.unwrap().level,
        1
    );
    app.key(egui::Key::Tab, egui::Modifiers::SHIFT);
    assert_eq!(
        app.document.paragraphs()[0].props.numbering.unwrap().level,
        0
    );
    // Anywhere else, Tab is still a tab.
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 5,
    });
    app.key(egui::Key::Tab, egui::Modifiers::NONE);
    assert_eq!(app.paragraph_text(1), "plain\t");
}

#[test]
fn the_selection_reads_out_as_text_across_paragraphs() {
    let mut app = app_with(&["first line", "second"]);
    assert_eq!(app.selected_text(), None, "an empty selection is nothing");
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 6,
        },
        head: Caret {
            paragraph: 1,
            offset: 3,
        },
    };
    assert_eq!(app.selected_text().as_deref(), Some("line\nsec"));
}

#[test]
fn pasting_types_over_the_selection_and_newlines_press_enter() {
    let mut app = app_with(&["first line", "second"]);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 6,
        },
        head: Caret {
            paragraph: 1,
            offset: 3,
        },
    };
    app.paste_text("X\r\nY");
    assert_eq!(app.document.text(), "first X\nYond");
    assert_eq!(app.caret().paragraph, 1);
    assert_eq!(app.caret().offset, 1);
}

#[test]
fn ctrl_backspace_deletes_the_word_before_the_caret() {
    let mut app = app_with(&["hello world"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 11,
    });
    app.key(egui::Key::Backspace, egui::Modifiers::COMMAND);
    assert_eq!(app.document.text(), "hello ");
    app.history.undo(&mut app.document);
    assert_eq!(app.document.text(), "hello world");
}

#[test]
fn ctrl_delete_deletes_the_word_after_the_caret() {
    let mut app = app_with(&["hello world"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    app.key(egui::Key::Delete, egui::Modifiers::COMMAND);
    assert_eq!(app.document.text(), "world");
}

#[test]
fn find_next_selects_the_match_and_moving_on_wraps() {
    let mut app = app_with(&["alpha beta alpha"]);
    app.finder = Some(Finder {
        query: "alpha".into(),
        ..Finder::default()
    });
    app.jump_match(true);
    assert_eq!(app.selection.ordered().0.offset, 0, "the first occurrence");
    assert_eq!(app.selection.ordered().1.offset, 5, "selected whole");
    app.jump_match(true);
    assert_eq!(app.selection.ordered().0.offset, 11);
    app.jump_match(true);
    assert_eq!(app.selection.ordered().0.offset, 0, "wrapped around");
    assert!(app.reveal.is_some(), "and the view was asked to follow");
}

#[test]
fn replace_all_replaces_every_match_and_says_how_many() {
    let mut app = app_with(&["one two one", "one more"]);
    app.finder = Some(Finder {
        query: "ONE".into(),
        replacement: "1".into(),
        ..Finder::default()
    });
    app.replace_all();
    assert_eq!(app.document.text(), "1 two 1\n1 more");
    assert_eq!(
        app.finder.as_ref().unwrap().note.as_deref(),
        Some("Replaced 3")
    );
    // A replace is a deletion and an insertion, so it comes back in two.
    app.history.undo(&mut app.document);
    app.history.undo(&mut app.document);
    assert_eq!(app.document.text(), "one two 1\n1 more", "undo, one by one");
}

#[test]
fn select_all_reaches_the_end_of_the_last_paragraph() {
    let mut app = app_with(&["one", "two", "three"]);
    app.run(Command::SelectAll);
    let (start, end) = app.selection.ordered();
    assert_eq!(
        start,
        Caret {
            paragraph: 0,
            offset: 0
        }
    );
    assert_eq!(
        end,
        Caret {
            paragraph: 2,
            offset: 5
        }
    );
}

#[test]
fn bold_with_no_selection_applies_to_the_word_the_caret_is_in() {
    // Otherwise Ctrl+B with the caret in a word appears to do nothing at all.
    let mut app = app_with(&["hello world"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 2,
    });
    app.run(Command::Bold);
    let paragraphs = app.document.paragraphs();
    let runs = paragraphs[0].runs();
    assert!(runs[0].props.bold(), "the word the caret was in");
    assert!(
        runs.last().is_some_and(|run| !run.props.bold()),
        "and not the rest of the line"
    );
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

#[test]
fn a_pasted_picture_lands_in_the_document_and_in_the_package() {
    // The three pieces a picture is. The board itself is the machine's, so
    // this hands the bytes over the way a paste would have.
    let mut app = app_with(&["before after"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 6,
    });
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");

    let paragraphs = app.document.paragraphs();
    let drawings = paragraphs[0].drawings();
    let [drawing] = &drawings[..] else {
        panic!("one picture, not {}", drawings.len());
    };
    // 96 pixels at 96 to the inch is an inch, which is 914400 EMU.
    assert_eq!(drawing.extent.0, wp_model::Emu(914_400), "an inch wide");
    assert_eq!(
        drawing.extent.1,
        wp_model::Emu(457_200),
        "half an inch tall"
    );
    // The picture is one character of the paragraph, where the caret was —
    // which is what lets the caret step over it and Backspace take it.
    assert_eq!(
        paragraphs[0].text(),
        format!("before{} after", wp_model::doc::OBJECT),
        "a picture is a character, and the words either side are untouched"
    );

    // The relationship the drawing names resolves to a part holding the
    // bytes — the half of it that lives outside the document.
    let package = app.package.as_ref().expect("a package was authored");
    let parts = app.parts.as_ref().expect("and located");
    let rel = drawing.rel.as_deref().expect("the drawing names one");
    let name = parts.target(rel).expect("which resolves");
    assert_eq!(package.part(name).expect("to a part").data(), PIXEL);

    // And it is one edit, so one undo takes it away again.
    app.run(Command::Undo);
    assert!(
        app.document.paragraphs()[0].drawings().is_empty(),
        "undo takes the picture out"
    );
}

#[test]
fn pressing_enter_beside_a_pasted_picture_does_not_make_a_second_one() {
    // The bug: Enter split the paragraph, and a picture that held none of
    // its text ended up in both halves.
    let mut app = app_with(&["before after"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 6,
    });
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");

    // What the Enter key does, at the caret the paste left behind.
    let caret = edit::split_paragraph(
        &mut app.document,
        app.scope,
        &mut app.history,
        app.selection,
    );
    app.selection = Selection::at(caret);

    let pictures: usize = app
        .document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.drawings().len())
        .sum();
    assert_eq!(pictures, 1, "still the one picture");
    assert_eq!(app.paragraph_count(), 2, "and the paragraph did split");
}

#[test]
fn a_picture_can_be_sized_by_the_numbers_and_one_undo_puts_it_back() {
    let mut app = app_with(&["ab"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 1,
    });
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");
    let picked = crate::drawings::Picked {
        paragraph: 0,
        nth: 0,
    };
    app.picked = Some(picked);

    // The box opens on the size the picture is: 96 by 48 pixels at 96 to
    // the inch is one inch by half of one.
    app.open_size_dialog();
    let draft = app.size_draft.as_ref().expect("the box opened");
    assert_eq!(
        (draft.width.as_str(), draft.height.as_str()),
        ("1.00", "0.50")
    );
    assert!(draft.locked, "the ratio is locked, as Word's box opens");
    assert!((draft.ratio - 2.0).abs() < 1e-9);

    // Two inches by one, which is what typing 2.00 with the lock on means.
    app.resize_drawing(picked, 144.0, 72.0);
    let drawing = app.picked_drawing().expect("still there");
    assert_eq!(drawing.extent.0, wp_model::Emu(1_828_800), "two inches");
    assert_eq!(drawing.extent.1, wp_model::Emu(914_400), "by one");

    app.run(Command::Undo);
    let drawing = app.picked_drawing().expect("and still there after");
    assert_eq!(drawing.extent.0, wp_model::Emu(914_400), "back an inch");
    assert_eq!(drawing.extent.1, wp_model::Emu(457_200));
}

#[test]
fn asking_to_size_nothing_says_so_rather_than_doing_nothing() {
    let mut app = app_with(&["ab"]);
    app.run(Command::PictureSize);
    assert!(app.size_draft.is_none(), "no box, because no picture");
    let (title, _) = app.message.as_ref().expect("it says why");
    assert_eq!(title, "Nothing selected");
}

#[test]
fn the_caret_steps_over_a_picture_and_backspace_takes_it() {
    let mut app = app_with(&["ab"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 1,
    });
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");
    assert_eq!(
        app.paragraph_text(0),
        format!("a{}b", wp_model::doc::OBJECT)
    );

    // One press of Right from in front of it lands behind it: a picture is
    // one character, not none.
    let text = app.paragraph_text(0);
    assert_eq!(text::next_char(&text, 1), 2);
    assert_eq!(text::previous_char(&text, 2), 1);

    // And Backspace from behind it takes the whole picture.
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 2,
    });
    let caret = edit::backspace(
        &mut app.document,
        app.scope,
        &mut app.history,
        app.selection,
    );
    assert_eq!(caret.offset, 1);
    assert_eq!(app.paragraph_text(0), "ab");
    assert!(app.document.paragraphs()[0].drawings().is_empty());
}

#[test]
fn a_copied_picture_pastes_as_the_same_part_shown_twice() {
    let mut app = app_with(&["ab"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 1,
    });
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");
    app.picked = Some(crate::drawings::Picked {
        paragraph: 0,
        nth: 0,
    });
    assert!(app.copy_drawing(), "there is a picture to copy");
    let copied = app.copied_drawing.as_ref().expect("and it was kept");
    assert!(
        copied.bytes.is_some(),
        "with its file bytes, for a document that has no such part"
    );

    app.picked = None;
    let text = app.paragraph_text(0);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: text.len(),
    });
    assert!(app.paste_copied_drawing(), "and it pastes back");
    let paragraphs = app.document.paragraphs();
    let drawings = paragraphs[0].drawings();
    assert_eq!(drawings.len(), 2, "the picture is now shown twice");
    // The same relationship: one part, no second copy of the bytes —
    // which is exactly what a duplicated picture is.
    assert_eq!(drawings[0].rel, drawings[1].rel);
}

#[test]
fn a_cut_picture_leaves_and_a_paste_puts_it_back() {
    let mut app = app_with(&["ab"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 1,
    });
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");
    app.picked = Some(crate::drawings::Picked {
        paragraph: 0,
        nth: 0,
    });

    app.run(Command::Cut);
    assert!(
        app.document.paragraphs()[0].drawings().is_empty(),
        "cut took the picture"
    );
    assert!(app.picked.is_none(), "and nothing is picked any more");

    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    assert!(app.paste_copied_drawing(), "the cut copy pastes back");
    assert_eq!(app.document.paragraphs()[0].drawings().len(), 1);
}

#[test]
fn a_chart_says_it_cannot_cross_into_another_document() {
    // A chart is a family of parts, and only its drawing was copied. In a
    // document where its relationship names nothing, the paste says so
    // rather than doing nothing.
    let mut app = app_with(&["ab"]);
    app.copied_drawing = Some(CopiedDrawing {
        drawing: wp_model::doc::Drawing {
            source: Vec::new().into(),
            source_format: wp_model::SourceFormat::Authored,
            anchored: false,
            extent: (wp_model::Emu(914_400), wp_model::Emu(457_200)),
            rel: None,
            chart: Some("rId99".into()),
            name: None,
            description: None,
            wrap: wp_model::doc::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: false,
            text: None,
            tone: None,
            outline: None,
        },
        bytes: None,
        png: None,
    });
    assert!(!app.paste_copied_drawing(), "nothing was pasted");
    let (title, _) = app.message.as_ref().expect("and it says why");
    assert_eq!(title, "Cannot paste");
    assert!(
        app.document.paragraphs()[0].drawings().is_empty(),
        "no half-pasted chart in the text"
    );
}

#[test]
fn a_file_word_can_embed_goes_in_as_it_is_and_anything_else_becomes_a_png() {
    // A JPEG re-encoded as a PNG is several times the size and no better;
    // a BMP kept as it is, is a photograph's worth of bytes for a picture
    // of a button.
    let (data, kind, width, height) = picture_bytes(PIXEL.to_vec()).expect("a png");
    assert_eq!(kind, "image/png");
    assert_eq!((width, height), (1, 1));
    assert_eq!(data, PIXEL, "the bytes were not touched");

    let mut bmp = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(4, 2))
        .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
        .expect("a bitmap");
    let (data, kind, width, height) = picture_bytes(bmp).expect("which is a picture");
    assert_eq!(
        kind, "image/png",
        "and arrives as one Word will not baulk at"
    );
    assert_eq!((width, height), (4, 2), "at the size it was");
    assert_eq!(
        image::guess_format(&data).expect("a format"),
        image::ImageFormat::Png
    );

    // A file that is not a picture at all is not offered to the document.
    assert!(picture_bytes(b"I am a text file, not a picture".to_vec()).is_none());
}

#[test]
fn a_picture_too_wide_for_the_page_is_brought_down_to_the_column() {
    // A snip of a whole screen is 1920 pixels, which is twenty inches.
    let app = app_with(&[""]);
    let paragraph = picture_paragraph("rId9", &app.document.section, 1920, 1080);
    let drawings = paragraph.drawings();
    let drawing = drawings.first().expect("a picture");
    let column = wp_model::PageBox::of(&app.document.section).text_width();
    let width = drawing.extent.0 .0 as f64 / 12_700.0;
    let height = drawing.extent.1 .0 as f64 / 12_700.0;
    assert!((width - column).abs() < 0.5, "{width} fills the column");
    assert!(
        (height / width - 1080.0 / 1920.0).abs() < 0.01,
        "and keeps its proportions"
    );
}

#[test]
fn the_cf_html_header_says_where_the_fragment_is() {
    // Every number in that header is a byte offset into the string it is
    // part of. One digit out and Word pastes the header, or half the
    // fragment, or nothing.
    fn offset(wrapped: &str, field: &str) -> usize {
        let at = wrapped.find(field).expect("the field") + field.len();
        wrapped[at..at + 10].parse().expect("ten digits")
    }
    let fragment = "<span style=\"font-weight:bold\">hello</span>";
    let wrapped = cf_html(fragment);
    assert_eq!(
        &wrapped[offset(&wrapped, "StartFragment:")..offset(&wrapped, "EndFragment:")],
        fragment
    );
    assert!(wrapped[offset(&wrapped, "StartHTML:")..].starts_with("<html>"));
    assert_eq!(offset(&wrapped, "EndHTML:"), wrapped.len());
}

#[test]
fn a_copy_offers_word_the_formatting_the_text_cannot_carry() {
    // What goes on the board beside the text. The board itself is the
    // machine's and a test must not touch it, so this asks the two writers
    // for what a copy would have handed it.
    let mut app = app_with(&["make this bold please"]);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 5,
        },
        head: Caret {
            paragraph: 0,
            offset: 14,
        },
    };
    app.run(Command::Bold);
    let paragraphs = edit::copy_range(&app.document, app.scope, app.selection);
    let html = clip::html(&app.document, &paragraphs);
    assert!(html.contains("font-weight:bold"), "{html}");
    assert!(html.contains("this bold"), "{html}");
    let rtf = clip::rtf(&app.document, &paragraphs);
    assert!(rtf.contains("\\b"), "{rtf}");
    assert!(rtf.contains("this bold"), "{rtf}");
}

#[test]
fn pasting_what_this_copied_keeps_its_formatting() {
    // The clipboard holds text and nothing else, so copy and paste went
    // through a `String` and everything the runs knew was thrown away on
    // the way: a bold phrase came back plain.
    let mut app = app_with(&["make this bold please"]);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 5,
        },
        head: Caret {
            paragraph: 0,
            offset: 14,
        },
    };
    app.run(Command::Bold);
    copied(&mut app, 5..14);

    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: text_of(&app, 0).len(),
    });
    app.paste_matching("this bold");

    assert_eq!(
        text_of(&app, 0),
        "make this bold please".to_string() + "this bold"
    );
    let paragraphs = app.document.paragraphs();
    let bold: String = paragraphs[0]
        .runs()
        .iter()
        .filter(|run| run.props.bold())
        .map(|run| run.text())
        .collect();
    assert_eq!(bold, "this boldthis bold", "the pasted copy is bold too");
}

#[test]
fn pasting_something_another_program_copied_arrives_as_text() {
    // The board no longer says what this copied, so whatever is on it came
    // from somewhere else and is text — it takes the formatting of wherever
    // the caret is, which is Word's rule.
    let mut app = app_with(&["make this bold please"]);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 5,
        },
        head: Caret {
            paragraph: 0,
            offset: 14,
        },
    };
    app.run(Command::Bold);
    copied(&mut app, 5..14);

    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    app.paste_matching("from elsewhere");

    assert!(text_of(&app, 0).starts_with("from elsewhere"));
    let paragraphs = app.document.paragraphs();
    let bold: String = paragraphs[0]
        .runs()
        .iter()
        .filter(|run| run.props.bold())
        .map(|run| run.text())
        .collect();
    assert_eq!(bold, "this bold", "only what was already bold");
}

#[test]
fn copying_across_paragraphs_pastes_them_back_as_paragraphs() {
    let mut app = app_with(&["first line", "second line", "third line"]);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 6,
        },
        head: Caret {
            paragraph: 2,
            offset: 5,
        },
    };
    let text = app.selected_text().expect("something to copy");
    app.clipboard = Some(Clip {
        text: text.clone(),
        paragraphs: edit::copy_range(&app.document, app.scope, app.selection),
    });
    let was = app.document.paragraphs().len();

    app.selection = Selection::at(Caret {
        paragraph: 2,
        offset: text_of(&app, 2).len(),
    });
    app.paste_matching(&text);

    assert_eq!(
        app.document.paragraphs().len(),
        was + 2,
        "three copied paragraphs join onto one and add two"
    );
    assert_eq!(text_of(&app, 2), "third lineline");
    assert_eq!(text_of(&app, 3), "second line");
    assert_eq!(text_of(&app, 4), "third");
}

fn text_of(app: &Scriva, index: usize) -> String {
    app.document.paragraphs()[index].text()
}

#[test]
fn bold_is_a_toggle_rather_than_a_one_way_switch() {
    let mut app = app_with(&["word"]);
    app.run(Command::SelectAll);
    app.run(Command::Bold);
    assert!(app.document.paragraphs()[0].runs()[0].props.bold());
    app.run(Command::Bold);
    assert!(!app.document.paragraphs()[0].runs()[0].props.bold());
}

#[test]
fn undo_puts_the_caret_back_in_the_document() {
    let mut app = app_with(&["first", "second"]);
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 0,
    });
    let caret = edit::backspace(
        &mut app.document,
        app.scope,
        &mut app.history,
        app.selection,
    );
    app.selection = Selection::at(caret);
    assert_eq!(app.paragraph_count(), 1);
    app.run(Command::Undo);
    assert_eq!(app.paragraph_count(), 2);
    assert!(
        app.selection.head.paragraph < app.paragraph_count(),
        "the caret is inside the document it names"
    );
}

#[test]
fn a_caret_past_the_end_is_brought_back_inside() {
    let document = Document {
        body: vec![Block::Paragraph(Paragraph::of("short"))],
        ..Document::new()
    };
    let clamped = clamp(
        &document,
        wp_model::Scope::Body,
        Caret {
            paragraph: 9,
            offset: 900,
        },
    );
    assert_eq!(
        clamped,
        Caret {
            paragraph: 0,
            offset: 5
        }
    );
}

#[test]
fn alignment_is_a_paragraph_command_and_needs_no_selection() {
    let mut app = app_with(&["one", "two"]);
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 0,
    });
    app.run(Command::Align(Justify::Center));
    assert_eq!(app.document.paragraphs()[0].props.justify, None);
    assert_eq!(
        app.document.paragraphs()[1].props.justify,
        Some(Justify::Center)
    );
}

#[test]
fn indenting_moves_by_half_an_inch_and_never_past_the_margin() {
    let mut app = app_with(&["text"]);
    app.run(Command::Indent(1));
    assert_eq!(
        app.document.paragraphs()[0].props.indent.start,
        Some(Twips(720))
    );
    app.run(Command::Indent(-1));
    assert_eq!(
        app.document.paragraphs()[0].props.indent.start,
        Some(Twips(0))
    );
    app.run(Command::Indent(-1));
    assert_eq!(
        app.document.paragraphs()[0].props.indent.start,
        Some(Twips(0)),
        "and no further"
    );
}

#[test]
fn a_document_with_no_path_is_still_called_something() {
    let app = Scriva::new();
    let (name, dirty) = app.document().expect("a title");
    assert_eq!(name, "Document1");
    assert!(!dirty);
}

#[test]
fn an_edit_marks_the_document_dirty_and_the_view_stale() {
    let mut app = app_with(&["text"]);
    let stamp = app.stamp;
    app.run(Command::SelectAll);
    app.run(Command::Bold);
    assert!(app.dirty);
    assert_ne!(app.stamp, stamp, "the view has to lay out again");
}

#[test]
fn typing_with_track_changes_on_records_rather_than_replaces() {
    let mut app = app_with(["hello"].as_slice());
    app.document.settings.track_changes = true;
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 5,
    });
    app.type_text(" there");
    assert_eq!(app.document.text(), "hello there");
    let changes = crate::revise::tracked(&app.document);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].what, "inserted");
}

#[test]
fn typing_with_track_changes_off_is_an_ordinary_edit() {
    let mut app = app_with(["hello"].as_slice());
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 5,
    });
    app.type_text("!");
    assert_eq!(app.document.text(), "hello!");
    assert!(crate::revise::tracked(&app.document).is_empty());
}

#[test]
fn accept_all_with_nothing_tracked_says_so_rather_than_doing_nothing() {
    let mut app = app_with(["plain"].as_slice());
    app.run(Command::AcceptAll);
    assert!(app.message.is_some());
    assert!(!app.dirty);
}

#[test]
fn next_change_walks_round_the_document() {
    let mut app = app_with(["one", "two", "three"].as_slice());
    let paragraphs = &mut app.document.body;
    if let Block::Paragraph(p) = &mut paragraphs[2] {
        p.content = vec![wp_model::doc::inserted_by(
            "A",
            1,
            vec![wp_model::doc::Run::of("added")]
                .into_iter()
                .map(wp_model::Inline::Run)
                .collect(),
        )];
    }
    app.run(Command::NextChange);
    assert_eq!(app.caret().paragraph, 2);
    // And round again from the end.
    app.run(Command::NextChange);
    assert_eq!(app.caret().paragraph, 2);
}

#[test]
fn go_to_puts_the_caret_at_a_paragraph_and_asks_to_be_shown_it() {
    let mut app = app_with(&["one", "two", "three"]);
    app.run(Command::GoTo(wp_model::Scope::Body, 2));
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 2,
            offset: 0
        }
    );
    assert!(app.reveal.is_some(), "a caret has no place on the page yet");
}

#[test]
fn go_to_a_paragraph_that_is_not_there_lands_inside_the_document() {
    let mut app = app_with(&["only"]);
    app.run(Command::GoTo(wp_model::Scope::Body, 99));
    assert_eq!(app.caret().paragraph, 0);
}

#[test]
fn updating_a_table_of_contents_with_no_toc_field_says_so() {
    // Silently doing nothing is the failure here: the user pressed a key and
    // has to be told why nothing happened.
    let mut app = app_with(&["body text"]);
    app.run(Command::UpdateToc);
    assert!(app.message.is_some());
    assert!(!app.dirty, "and nothing was changed");
}

#[test]
fn a_document_with_a_path_answers_the_filename_field() {
    let mut app = app_with(&["text"]);
    app.path = Some(PathBuf::from("C:/reports/Q3.docx"));
    app.refresh_fields();
    assert_eq!(app.fields.file_name.as_deref(), Some("Q3.docx"));
}

#[test]
fn the_extension_decides_the_format() {
    assert_eq!(Format::of(Path::new("a.docx")), Format::Docx);
    assert_eq!(Format::of(Path::new("a.DOCX")), Format::Docx);
    assert_eq!(Format::of(Path::new("a.md")), Format::Markdown);
    assert_eq!(Format::of(Path::new("a.txt")), Format::Text);
    assert!(Format::Markdown.is_lossy());
    assert!(!Format::Docx.is_lossy());
}

#[test]
fn a_markdown_file_opens_as_a_document_with_headings() {
    let dir = std::env::temp_dir().join("scriva-c26");
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let path = dir.join("note.md");
    std::fs::write(&path, "# Title\r\n\r\nSome **bold** text.\r\n").expect("written");

    let mut app = Scriva::new();
    app.open_path(&path);
    assert_eq!(app.paragraph_count(), 2);
    assert!(app.package.is_none(), "there is no package behind a .md");
    assert_eq!(app.ending, wp_text::LineEnding::Crlf, "kept for the save");
    let paragraphs = app.document.paragraphs();
    assert_eq!(
        wp_model::outline::heading_level(paragraphs[0], &app.document.styles),
        Some(1)
    );
}

#[test]
fn saving_as_text_keeps_the_line_endings_the_file_came_in_with() {
    let dir = std::env::temp_dir().join("scriva-c26");
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let path = dir.join("plain.txt");
    std::fs::write(&path, "one\r\ntwo\r\n").expect("written");

    let mut app = Scriva::new();
    app.open_path(&path);
    assert_eq!(app.paragraph_count(), 2);
    assert!(app.save_text(&path, Format::Text));
    let back = std::fs::read_to_string(&path).expect("read back");
    assert_eq!(back, "one\r\ntwo");
}

#[test]
fn saving_in_a_lossy_format_asks_before_it_writes() {
    // A user who did not mean it has no way back once the file is on disk.
    let mut app = app_with(["text"].as_slice());
    app.pending = Some(Pending::Lossy(PathBuf::from("x.md"), Format::Markdown));
    assert!(matches!(app.pending, Some(Pending::Lossy(_, _))));
}

#[test]
fn a_file_saved_without_an_extension_gets_one() {
    assert_eq!(
        with_extension(PathBuf::from("report")),
        PathBuf::from("report.docx")
    );
    assert_eq!(
        with_extension(PathBuf::from("report.docm")),
        PathBuf::from("report.docm")
    );
}

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/odt")
        .join(name)
}

/// The whole of A1: an OpenDocument file opened here is a document, not a
/// copy of one. It keeps its own name, its own package and its own format,
/// and a save writes back through all three.
#[test]
fn an_odt_is_saved_in_place_rather_than_turned_into_a_docx() {
    assert!(Format::Odt.is_writable());
    assert!(!Format::Odt.is_lossy());

    let source = corpus("second-producer.odt");
    let temp = std::env::temp_dir().join("scriva-odt-in-place.odt");
    std::fs::copy(&source, &temp).expect("the corpus document is there");

    let mut app = Scriva::new();
    app.open_odt(&temp);
    assert_eq!(
        app.path.as_deref(),
        Some(temp.as_path()),
        "the path is the file's own, not the same name with .docx on it"
    );
    assert!(
        app.container.is_some(),
        "the package it came out of is kept"
    );
    assert!(
        !app.dirty,
        "and it is a document rather than an unsaved copy"
    );

    {
        let mut paragraphs = app.document.paragraphs_mut();
        let first = paragraphs.first_mut().expect("a paragraph to edit");
        first.content = vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of(
            "edited in Scriva",
        ))];
    }
    assert!(app.save(), "the save reports success");

    let (reopened, _, saved) =
        wp_odf::open(&temp).expect("what came out is an OpenDocument package");
    assert!(
        reopened.text().contains("edited in Scriva"),
        "the edit came back"
    );

    // And the half that matters: an edit in the body moved `content.xml`
    // and nothing else in the package.
    let original = wp_odf::Container::open(&source).expect("the corpus document opens");
    for part in original.parts() {
        let name = part.name().as_str();
        if name.trim_start_matches('/') == "content.xml" {
            continue;
        }
        assert_eq!(
            saved.data(name),
            Some(part.data()),
            "{name} was rewritten though nothing in it was edited"
        );
    }
    let _ = std::fs::remove_file(&temp);
}

/// Save As `.odt` from a document that never had a package: the writer
/// authors one, and what lands on disk opens again.
#[test]
fn a_document_with_no_package_saves_as_an_odt() {
    let mut app = app_with(&["first line", "second line"]);
    let temp = std::env::temp_dir().join("scriva-odt-authored.odt");
    app.path = Some(temp.clone());
    assert!(app.save(), "the save reports success");

    let (read, _, _) = wp_odf::open(&temp).expect("it opens as OpenDocument text");
    assert_eq!(
        read.text(),
        "first line
second line"
    );
    let _ = std::fs::remove_file(&temp);
}

/// A directory of this test's own, so that two of these running at once
/// cannot save over one another's document.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scriva-odt-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A chooser answers frames after it was asked — on Linux from another
/// program's window, which used to hold this one's thread until it did —
/// so what was to follow a Save As rides along with the question. Here
/// that is the Close that asked for the save, and it happens once the
/// document is on disk and not before.
#[test]
fn a_save_as_answered_frames_later_saves_and_then_does_what_it_was_for() {
    let dir = scratch("chooser-answer");
    let target = dir.join("answered.docx");
    let mut app = app_with(&["kept"]);
    app.dirty = true;
    let chosen = target.clone();
    app.asking = Some(ui_kit::chooser::Asking::new(
        move || Some(chosen),
        Chosen::SaveAs(Some(Box::new(Command::Close))),
    ));

    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    for _ in 0..200 {
        if app.asking.is_none() {
            break;
        }
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.overlay(ui.ctx()));
        out.textures_delta.clear();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.asking.is_none(), "the answer was picked up");
    let saved = wp_docx::open(&target).expect("the document was saved where the chooser said");
    assert_eq!(saved.0.paragraphs()[0].text(), "kept");
    assert!(
        app.path.is_none() && !app.dirty,
        "and the Close the save was standing in front of went ahead"
    );
}

/// S1 — Save As to a different name. The new file holds the edit, the file
/// it was saved *from* is untouched, and the application goes on editing
/// the new one: a Save As that leaves the caret over the old path is how
/// the next Ctrl+S writes to the wrong file.
#[test]
fn save_as_odt_writes_a_new_file_and_leaves_the_old_one_where_it_was() {
    let dir = scratch("save-as");
    let source = dir.join("original.odt");
    std::fs::copy(corpus("second-producer.odt"), &source).expect("the corpus document is there");
    let before = std::fs::read(&source).expect("the copy is readable");

    let mut app = Scriva::new();
    app.open_odt(&source);
    app.type_text("Saved as. ");
    assert!(app.dirty, "typing is what makes a document need saving");

    let target = dir.join("under-another-name.odt");
    assert!(app.save_to(target.clone()), "the save reports success");
    assert_eq!(
        app.path.as_deref(),
        Some(target.as_path()),
        "the document being edited is now the new one"
    );
    assert!(!app.dirty, "and it is saved");

    assert_eq!(
        std::fs::read(&source).expect("it is still there"),
        before,
        "the file Save As was invoked from was written to"
    );
    let (reopened, _, _) = wp_odf::open(&target).expect("the new file is an OpenDocument one");
    assert!(
        reopened.text().starts_with("Saved as. "),
        "the edit is in the file that was written"
    );
}

/// S2 — both directions across the two package formats. The document lets
/// go of the container it arrived in and authors the other, which is what
/// `self.container` and `self.package` never being live together means.
#[test]
fn a_cross_format_save_lets_go_of_the_package_the_document_arrived_in() {
    let dir = scratch("cross-format");

    // An OpenDocument document saved as a Word one.
    let mut app = Scriva::new();
    app.open_odt(&corpus("second-producer.odt"));
    assert!(app.container.is_some() && app.package.is_none());
    app.type_text("Now a docx. ");
    let as_docx = dir.join("from-odt.docx");
    assert!(app.save_to(as_docx.clone()), "the save reports success");
    assert!(
        app.container.is_none(),
        "the OpenDocument package is let go with the format it belonged to"
    );
    assert!(app.package.is_some(), "and a Word one stands in its place");
    let (from_odt, _) =
        wp_docx::open(&as_docx).expect("it opens as the Word document it claims to be");
    assert!(from_odt.text().starts_with("Now a docx. "));

    // And a Word document saved as an OpenDocument one.
    let mut app = Scriva::new();
    app.open_docx(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/docx/minimal.docx"));
    assert!(app.package.is_some() && app.container.is_none());
    let was = app.document.text();
    app.type_text("Now an odt. ");
    let as_odt = dir.join("from-docx.odt");
    assert!(app.save_to(as_odt.clone()), "the save reports success");
    assert!(
        app.package.is_none() && app.parts.is_none(),
        "the Word package is let go rather than kept beside the container"
    );
    assert!(app.container.is_some());
    let (from_docx, _, _) =
        wp_odf::open(&as_odt).expect("it opens as the OpenDocument text it claims to be");
    assert_eq!(
        from_docx.text(),
        format!("Now an odt. {was}"),
        "the words came across whole"
    );
}

/// S3 — two saves in one session. The second writes through a container the
/// first already flushed, and the parts nobody edited are still the bytes
/// the corpus document arrived with.
#[test]
fn a_session_that_saves_twice_moves_only_what_was_edited_both_times() {
    let dir = scratch("twice");
    let source = corpus("second-producer.odt");
    let target = dir.join("twice.odt");
    std::fs::copy(&source, &target).expect("the corpus document is there");

    let mut app = Scriva::new();
    app.open_odt(&target);
    app.type_text("First save. ");
    assert!(app.save(), "the first save reports success");
    app.type_text("Second save. ");
    assert!(app.save(), "the second save reports success");

    let (reopened, _, saved) = wp_odf::open(&target).expect("what came out is a package");
    assert!(
        reopened.text().starts_with("First save. Second save. "),
        "both edits are in the file, in the order they were typed"
    );

    let original = wp_odf::Container::open(&source).expect("the corpus document opens");
    for part in original.parts() {
        let name = part.name().as_str();
        if name.trim_start_matches('/') == "content.xml" {
            continue;
        }
        assert_eq!(
            saved.data(name),
            Some(part.data()),
            "{name} was rewritten by the second save though nothing in it was edited"
        );
    }
}

/// S4 — a save that cannot be written. The message says so, the document is
/// still dirty and still knows its name, and what is on disk is what was
/// there before: a failed save that clears the dirty flag is a lost
/// document the next time the window closes.
#[test]
fn a_save_that_fails_says_so_and_loses_neither_the_edit_nor_the_file() {
    let dir = scratch("cannot-write");
    let target = dir.join("read-only.odt");
    std::fs::copy(corpus("second-producer.odt"), &target).expect("the corpus document is there");

    let mut app = Scriva::new();
    app.open_odt(&target);
    app.type_text("Never written. ");

    let mut readonly = std::fs::metadata(&target)
        .expect("it is there")
        .permissions();
    readonly.set_readonly(true);
    std::fs::set_permissions(&target, readonly).expect("the file can be made read-only");
    let before = std::fs::read(&target).expect("and read");

    assert!(
        !app.save(),
        "a save that did not happen does not report success"
    );
    let (title, said) = app.message.clone().expect("the user is told");
    assert_eq!(title, "Cannot save");
    assert!(
        said.contains("read-only.odt") && said.contains("another program"),
        "the message names the file and the likeliest reason: {said}"
    );
    assert!(
        app.dirty,
        "the edit is still unsaved rather than believed saved"
    );
    assert_eq!(
        app.path.as_deref(),
        Some(target.as_path()),
        "and the document still knows where it belongs"
    );
    assert_eq!(
        std::fs::read(&target).expect("still readable"),
        before,
        "the file that could not be written was written to anyway"
    );

    // The other way a save fails, which reaches the same arm through a
    // different error: nothing to write into. The temporary file a save
    // writes beside its target has nowhere to go either, so this is the
    // case where not one byte is created.
    let gone = dir.join("no-such-directory").join("elsewhere.odt");
    assert!(
        !app.save_to(gone.clone()),
        "and this one does not report success either"
    );
    assert!(app.dirty);
    assert!(!gone.exists());
    assert_eq!(
        app.path.as_deref(),
        Some(target.as_path()),
        "a Save As that failed does not rename the document to where it could not go"
    );

    let mut writable = std::fs::metadata(&target)
        .expect("it is there")
        .permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    writable.set_readonly(false);
    let _ = std::fs::set_permissions(&target, writable);
}

/// A Save As into the *other* format that fails, then Ctrl+S.
///
/// **The path was only half of what a failed Save As has to put back.** A
/// save into the other format lets go of the package the document arrived
/// in, because that package belongs to the format being left. If the write
/// then fails — the target open in another program is the everyday case —
/// the document goes back to the file it came from under its old name, and
/// the next Ctrl+S has no package to write it through. It authors one from
/// nothing, and everything the reader did not model goes with the old one:
/// the pictures of an `.odt`, the custom XML of a `.docx`. Nothing says so;
/// the save reports success.
#[test]
fn a_cross_format_save_that_fails_keeps_the_odt_package_the_document_came_in() {
    let dir = scratch("cross-format-odt");
    let source = dir.join("original.odt");
    std::fs::copy(corpus("second-producer.odt"), &source).expect("the corpus document is there");
    let original = wp_odf::Container::open(&source).expect("the copy opens");

    let mut app = Scriva::new();
    app.open_odt(&source);
    app.type_text("Kept. ");
    let named = |app: &mut Scriva| {
        let pictures: Vec<Option<String>> = app
            .document
            .drawings_mut()
            .iter()
            .map(|drawing| drawing.rel.as_deref().map(str::to_owned))
            .collect();
        (pictures, external_links(&mut app.document))
    };
    let before = named(&mut app);
    let nowhere = dir.join("no-such-directory").join("elsewhere.docx");
    assert!(!app.save_to(nowhere), "a save with nowhere to go fails");
    assert_eq!(
        named(&mut app),
        before,
        "a save that did not happen left the pictures and links named for a package              that was never written"
    );
    assert!(
        app.save(),
        "and the save back to the file it came from works"
    );

    let saved = wp_odf::Container::open(&source).expect("what was saved opens");
    for part in original.parts() {
        let name = part.name().as_str();
        if name.trim_start_matches('/') == "content.xml" {
            continue;
        }
        assert_eq!(
            saved.data(name),
            Some(part.data()),
            "{name} did not survive a failed Save As into .docx and the Ctrl+S after it"
        );
    }
}

/// The same, the other way: a `.docx` whose Save As into `.odt` failed.
#[test]
fn a_cross_format_save_that_fails_keeps_the_docx_package_the_document_came_in() {
    let dir = scratch("cross-format-docx");
    let source = dir.join("original.docx");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/docx/content-controls.docx"),
        &source,
    )
    .expect("the corpus document is there");
    let original = ooxml::Package::open(&source).expect("the copy opens");

    let mut app = Scriva::new();
    app.open_path(&source);
    app.type_text("Kept. ");
    let nowhere = dir.join("no-such-directory").join("elsewhere.odt");
    assert!(!app.save_to(nowhere), "a save with nowhere to go fails");
    assert!(
        app.save(),
        "and the save back to the file it came from works"
    );

    let saved = ooxml::Package::open(&source).expect("what was saved opens");
    for part in original.parts() {
        if part.name.as_str() == "/word/document.xml" {
            continue;
        }
        assert_eq!(
            saved.part(&part.name).map(|part| part.data()),
            Some(part.data()),
            "{} did not survive a failed Save As into .odt and the Ctrl+S after it",
            part.name.as_str()
        );
    }
}

/// The pictures of an `.odt` saved as a `.docx` are pictures in it.
///
/// The bytes were always carried across; what went wrong was the name.
/// Each reader mints a name for a picture it hands out loose — `.doc`
/// pictures are `doc-picture-N`, ODF ones `odf-picture-N` — and authoring a
/// package re-points the drawings from that name to the relationship it
/// embeds the picture under. It re-pointed only the names beginning `doc-`,
/// so a drawing out of an `.odt` kept naming `odf-picture-1`, which is no
/// relationship of the document at all, and Word reports that as a damaged
/// file rather than as a missing picture.
#[test]
fn a_cross_format_save_carries_the_odt_pictures_into_the_docx() {
    let dir = scratch("cross-format-pictures");
    let source = dir.join("with-pictures.odt");
    std::fs::copy(corpus("second-producer.odt"), &source).expect("the corpus document is there");
    let mut app = Scriva::new();
    app.open_odt(&source);
    // `drawings_mut` because it is the one walk that takes in the headers as
    // well as the body; nothing here is changed through it.
    let pictures = app
        .document
        .drawings_mut()
        .iter()
        .filter(|drawing| drawing.rel.is_some())
        .count();
    assert!(pictures > 0, "the corpus document has pictures to carry");

    let target = dir.join("as-word.docx");
    assert!(app.save_to(target.clone()), "the save reports success");

    let package = ooxml::Package::open(&target).expect("it opens as a package");
    let parts = wp_docx::DocumentParts::locate_in(&package).expect("it has a document part");
    let rels = package
        .relationships(&parts.document)
        .expect("the document part has relationships");
    let mut document = wp_docx::read(&package).expect("it reads as a document");
    let named: Vec<String> = document
        .drawings_mut()
        .into_iter()
        .filter_map(|drawing| drawing.rel.as_deref().map(str::to_owned))
        .collect();
    assert_eq!(named.len(), pictures, "every picture came across");
    let xml = String::from_utf8_lossy(
        package
            .part(&parts.document)
            .expect("the document part is there")
            .data(),
    )
    .into_owned();
    assert_eq!(
        unbound_prefixes(&xml),
        Vec::<String>::new(),
        "the document part uses prefixes it never declares"
    );
    for rel in named {
        assert!(
            rels.get(&rel).is_some(),
            "{rel} names no relationship of the document, which Word reports as a damaged file"
        );
    }
}

/// Authoring a package re-points every loose picture's drawing at the
/// relationship it now has, and the painter finds a relationship's bytes
/// through `parts`, the package's index. The save set the package and not
/// its index, so from the next edit — the first layout to paint the new
/// names — every picture in the window was an empty box, and a PDF made
/// then would have had none.
#[test]
fn the_pictures_still_paint_after_the_save_that_authored_their_package() {
    let dir = scratch("pictures-after-first-save");
    let source = dir.join("with-pictures.odt");
    std::fs::copy(corpus("second-producer.odt"), &source).expect("the corpus document is there");
    let mut app = Scriva::new();
    app.open_odt(&source);
    assert!(
        app.save_to(dir.join("as-word.docx")),
        "the save reports success"
    );

    let named: Vec<String> = app
        .document
        .drawings_mut()
        .into_iter()
        .filter_map(|drawing| drawing.rel.as_deref().map(str::to_owned))
        .collect();
    assert!(!named.is_empty(), "the corpus document has pictures");
    for rel in named {
        assert!(
            app.pictures
                .bytes(app.package.as_ref(), app.parts.as_ref(), &rel)
                .is_some(),
            "{rel} is a picture the window can no longer find"
        );
    }
}

/// The prefixes a part uses and has not declared where it uses them.
///
/// A part with one is not well-formed XML as far as a namespace-aware
/// reader is concerned, and both applications that own these formats are
/// namespace-aware: Word reports the file damaged and LibreOffice will not
/// open the part. This crate's own readers match local names and skip what
/// they do not know, so a reopen here passes over exactly this. Scoped, and
/// attributes included, because a first version of this check was neither:
/// it passed a `.docx` whose `r:id` had no `r:` declared anywhere above it,
/// and Word refused the file.
fn unbound_prefixes(xml: &str) -> Vec<String> {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;

    let mut reader = quick_xml::NsReader::from_str(xml);
    let mut out: Vec<String> = Vec::new();
    let note = |result: ResolveResult<'_>, out: &mut Vec<String>| {
        if let ResolveResult::Unknown(prefix) = result {
            let prefix = String::from_utf8_lossy(&prefix).into_owned();
            if !out.contains(&prefix) {
                out.push(prefix);
            }
        }
    };
    loop {
        match reader.read_resolved_event() {
            Ok((result, Event::Start(element) | Event::Empty(element))) => {
                note(result, &mut out);
                for attribute in element.attributes().flatten() {
                    let (result, _) = reader.resolver().resolve_attribute(attribute.key);
                    note(result, &mut out);
                }
            }
            Ok((_, Event::Eof)) => break,
            Ok(_) => {}
            Err(error) => {
                out.push(format!("(not XML at all: {error})"));
                break;
            }
        }
    }
    out
}

/// A `.docx` with pictures saved as `.odt` is an OpenDocument package, not
/// a WordprocessingML one wearing its name.
///
/// A drawing keeps the bytes it was read as so that a save can put them
/// back, and the ODF writer put back any bytes it was given — including a
/// `<w:drawing>`, whose `w:`, `wp:` and `a:` prefixes no ODF part declares.
/// Whatever else comes across, what must is a file LibreOffice will open.
#[test]
fn a_cross_format_save_writes_no_docx_markup_into_the_odt() {
    let dir = scratch("cross-format-markup");
    let source = dir.join("with-pictures.docx");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/docx/floating-image-wrap.docx"),
        &source,
    )
    .expect("the corpus document is there");
    let mut app = Scriva::new();
    app.open_path(&source);
    assert!(
        !app.document.drawings_mut().is_empty(),
        "the corpus document has pictures"
    );

    let target = dir.join("as-odf.odt");
    assert!(app.save_to(target.clone()), "the save reports success");

    let saved = wp_odf::Container::open(&target).expect("it opens as a package");
    for part in ["content.xml", "styles.xml"] {
        let xml =
            String::from_utf8_lossy(saved.data(part).expect("the part is there")).into_owned();
        assert_eq!(
            unbound_prefixes(&xml),
            Vec::<String>::new(),
            "{part} uses prefixes it never declares"
        );
    }
}

/// What every link to outside the document names, in document order.
fn external_links(document: &mut wp_model::Document) -> Vec<String> {
    document
        .hyperlinks_mut()
        .into_iter()
        .filter(|link| link.anchor.is_none())
        .filter_map(|link| link.rel.as_deref().map(str::to_owned))
        .collect()
}

/// The links of an `.odt` saved as a `.docx` go where they went.
///
/// ODF states a link's address on the link; WordprocessingML names a
/// relationship that holds it. The address was being written where the
/// name goes — `r:id="https://…"`, with no `r:` declared above it — and
/// Word refused the whole file rather than the link.
#[test]
fn a_cross_format_save_relates_the_odt_links_in_the_docx() {
    let dir = scratch("cross-format-links");
    let source = dir.join("with-links.odt");
    std::fs::copy(corpus("second-producer.odt"), &source).expect("the corpus document is there");
    let mut app = Scriva::new();
    app.open_odt(&source);
    let addresses = external_links(&mut app.document);
    assert!(
        !addresses.is_empty(),
        "the corpus document links outside itself"
    );

    let target = dir.join("as-word.docx");
    assert!(app.save_to(target.clone()), "the save reports success");

    let package = ooxml::Package::open(&target).expect("it opens as a package");
    let parts = wp_docx::DocumentParts::locate_in(&package).expect("it has a document part");
    let rels = package
        .relationships(&parts.document)
        .expect("the document part has relationships");
    let mut document = wp_docx::read(&package).expect("it reads as a document");
    let went: Vec<String> = external_links(&mut document)
        .iter()
        .map(|id| {
            let rel = rels
                .get(id)
                .unwrap_or_else(|| panic!("{id} names no relationship"));
            assert_eq!(
                rel.mode,
                ooxml::TargetMode::External,
                "{id} leaves the document"
            );
            rel.target.clone()
        })
        .collect();
    assert_eq!(went, addresses, "each link goes where it went in the .odt");
}

/// The other way: a `.docx` link names a relationship and an `.odt` has
/// none, so what goes there is the address the relationship held. Its name
/// would be a link to nowhere, and nothing would say so.
#[test]
fn a_cross_format_save_states_the_docx_links_in_the_odt() {
    const ADDRESS: &str = "https://example.invalid/from-word";
    let dir = scratch("cross-format-docx-links");
    let source = dir.join("with-a-link.docx");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/docx/minimal.docx"),
        &source,
    )
    .expect("the corpus document is there");
    let mut app = Scriva::new();
    app.open_path(&source);

    // No document in the corpus links outside itself, so this one is given
    // a link the way a file Word wrote carries one: a relationship in the
    // package, and a hyperlink in the text that names it.
    let package = app
        .package
        .as_mut()
        .expect("a .docx arrives in its package");
    let id = wp_docx::link::relate(package, ADDRESS).expect("related");
    app.parts = wp_docx::DocumentParts::locate_in(package).ok();
    {
        let mut paragraphs = app.document.paragraphs_mut();
        let first = paragraphs.first_mut().expect("a paragraph");
        first
            .content
            .push(wp_model::doc::Inline::Hyperlink(Box::new(
                wp_model::doc::Hyperlink {
                    rel: Some(id.as_str().into()),
                    anchor: None,
                    tooltip: None,
                    history: true,
                    content: vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of("a link"))],
                },
            )));
    }

    let target = dir.join("as-odf.odt");
    assert!(app.save_to(target.clone()), "the save reports success");

    let (mut reopened, _, _) = wp_odf::open(&target).expect("it opens as OpenDocument text");
    assert_eq!(
        external_links(&mut reopened),
        vec![ADDRESS.to_owned()],
        "the link goes to the address, not to the name of a relationship"
    );
}

/// The notes of an `.odt` saved as a `.docx` are in it.
///
/// The text named them and the package did not hold them: the writer had
/// no notes part to author, and a reference to a note that is not there is
/// what Word means by a corrupted file. It refused the whole document.
#[test]
fn a_cross_format_save_carries_the_odt_notes_into_the_docx() {
    let dir = scratch("cross-format-notes");
    let source = dir.join("with-notes.odt");
    std::fs::copy(corpus("second-producer.odt"), &source).expect("the corpus document is there");
    let mut app = Scriva::new();
    app.open_odt(&source);
    let target = dir.join("as-word.docx");
    assert!(app.save_to(target.clone()), "the save reports success");

    let package = ooxml::Package::open(&target).expect("it opens as a package");
    let document = wp_docx::read(&package).expect("it reads as a document");
    let named: Vec<i32> = document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.runs())
        .flat_map(|run| run.content.iter())
        .filter_map(|piece| match piece {
            wp_model::doc::Piece::FootnoteRef { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert!(!named.is_empty(), "the corpus document has a footnote");
    for id in named {
        assert!(
            document
                .footnotes
                .iter()
                .any(|note| note.id == id && !note.content.is_empty()),
            "footnote {id} is named in the text and not in the package"
        );
    }
}

/// The pictures of a `.docx` saved as an `.odt` are pictures in it.
///
/// A `.docx` picture is a relationship naming a part, and ODF has no
/// relationships: the frame names the picture's path in the package. So the
/// bytes have to be carried across and the drawing re-pointed at where they
/// went. Until they were, a Word document saved as OpenDocument lost every
/// picture it had, and the application said so rather than doing it.
#[test]
fn a_cross_format_save_carries_the_docx_pictures_into_the_odt() {
    let dir = scratch("cross-format-docx-pictures");
    let source = dir.join("with-pictures.docx");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/docx/floating-image-wrap.docx"),
        &source,
    )
    .expect("the corpus document is there");
    let mut app = Scriva::new();
    app.open_path(&source);
    let pictures = app
        .document
        .drawings_mut()
        .iter()
        .filter(|drawing| drawing.rel.is_some())
        .count();
    assert!(pictures > 0, "the corpus document has pictures to carry");

    let target = dir.join("as-odf.odt");
    assert!(app.save_to(target.clone()), "the save reports success");
    assert!(
        app.document
            .drawings_mut()
            .iter()
            .filter_map(|drawing| drawing.rel.as_deref())
            .all(|rel| app.pictures.loose().contains_key(rel)),
        "and every picture still has something to paint it, the .docx being gone"
    );

    let (mut reopened, media, _) = wp_odf::open(&target).expect("it opens as OpenDocument text");
    let named: Vec<String> = reopened
        .drawings_mut()
        .iter()
        .filter_map(|drawing| drawing.rel.as_deref().map(str::to_owned))
        .collect();
    assert_eq!(named.len(), pictures, "every picture came across");
    for rel in &named {
        assert!(
            media.iter().any(|picture| &*picture.rel == rel.as_str()),
            "{rel} names no picture in the package"
        );
    }
}

/// A picture added to an `.odt` is saved in it.
///
/// The application refused this outright, because a picture in a `.docx` is
/// three things and nothing authored them for ODF. In ODF it is one: a part
/// under `Pictures/`, named by its path from the frame.
#[test]
fn a_picture_added_to_an_odt_is_saved_in_it() {
    let dir = scratch("odt-insert-picture");
    let source = dir.join("gains-a-picture.odt");
    std::fs::copy(corpus("second-producer.odt"), &source).expect("the corpus document is there");
    let png =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/sample-image.png"))
            .expect("the sample picture is there");

    let mut app = Scriva::new();
    app.open_odt(&source);
    let before = app.document.drawings_mut().len();
    assert!(
        app.insert_picture(&png, "image/png", 160, 120),
        "the picture goes in"
    );
    assert!(app.message.is_none(), "and nothing refuses it");
    assert!(app.save(), "the save reports success");

    let (mut reopened, media, _) = wp_odf::open(&source).expect("it opens again");
    assert_eq!(
        reopened.drawings_mut().len(),
        before + 1,
        "one picture more than it had"
    );
    assert!(
        media.iter().any(|picture| picture.data == png),
        "and its bytes are in the package"
    );
}

/// Insert Table by its letters and Enter, then the cells filled with Tab
/// between them, at every pace: the next key on the very next frame, one
/// frame later, or two. The surface takes the keyboard back when the box
/// closes, and egui lets it lock Tab only from its second frame with it.
#[test]
fn cells_filled_with_tab_straight_after_insert_table_at_any_pace() {
    for idle in 0..=2 {
        let drive = ui_kit::drive::Driver::new();
        let mut app = app_with(&["before"]);
        drive.settle(&mut app);
        drive.menu(&mut app, 'I', 'T');
        drive.press(&mut app, "Enter");
        for text in ["one", "two", "three"] {
            for _ in 0..idle {
                drive.settle(&mut app);
            }
            drive.type_text(&mut app, text);
            for _ in 0..idle {
                drive.settle(&mut app);
            }
            drive.press(&mut app, "Tab");
        }
        let table = app
            .document
            .body
            .iter()
            .find_map(|block| match block {
                Block::Table(table) => Some(table),
                _ => None,
            })
            .expect("a table");
        let cells: Vec<String> = table
            .rows
            .iter()
            .flat_map(|row| row.cells.iter().map(|cell| cell.text()))
            .collect();
        assert_eq!(
            cells,
            vec!["one", "two", "three", ""],
            "{idle} idle frames between keys"
        );
    }
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

/// File ▸ Export as PDF… by its letters: the chooser is asked for (and, in a
/// test, refused), and given a path the export writes a PDF that holds the
/// document's words.
#[test]
fn export_as_pdf_by_menu_letters_writes_the_document() {
    let drive = ui_kit::drive::Driver::new();
    let dir = scratch("pdf-by-key");
    let target = dir.join("out.pdf");
    let mut app = app_with(&["Exported words."]);
    drive.settle(&mut app);
    let before = ui_kit::headless::choosers_refused();
    drive.menu(&mut app, 'F', 'D');
    drive.settle(&mut app);
    assert_eq!(
        ui_kit::headless::choosers_refused(),
        before + 1,
        "Alt+F, D asked for a chooser"
    );
    chooser_answers(&drive, &mut app, target.clone(), Chosen::ExportPdf);
    let pdf = std::fs::read(&target).expect("a PDF was written");
    assert!(pdf.starts_with(b"%PDF-"), "and it is a PDF");
    assert!(app.message.is_none(), "{:?}", app.message);
}

/// File ▸ Recent by its letters reopens what was saved, and View ▸ Zoom by
/// its letters sets the zoom.
#[test]
fn recent_and_zoom_by_menu_letters() {
    let drive = ui_kit::drive::Driver::new();
    let dir = scratch("recent-by-key");
    let saved = dir.join("kept.docx");
    let mut app = app_with(&["Kept for later."]);
    assert!(app.save_to(saved.clone()));
    let mut other = Scriva::new();
    other.recent.remember(SCRIVA, &saved);
    drive.settle(&mut other);
    drive.menu(&mut other, 'F', 'R');
    drive.press(&mut other, "1");
    drive.settle(&mut other);
    assert_eq!(
        other.path.as_deref(),
        Some(saved.as_path()),
        "Alt+F, R, 1 reopened it"
    );
    assert_eq!(other.document.paragraphs()[0].text(), "Kept for later.");

    drive.menu(&mut other, 'V', 'Z');
    drive.press(&mut other, "2");
    drive.settle(&mut other);
    assert!(
        (other.view.zoom - 1.25).abs() < 1e-9,
        "Alt+V, Z, 2 is 125%: {}",
        other.view.zoom
    );
    drive.menu(&mut other, 'V', 'Z');
    drive.press(&mut other, "0");
    drive.settle(&mut other);
    assert!(
        (other.view.zoom - 2.0).abs() < 1e-9,
        "Alt+V, Z, 0 is 200%: {}",
        other.view.zoom
    );
}

/// Insert ▸ Picture… by its letters, the chooser answered with a picture:
/// the picture is in the document, and Backspace takes it out again.
#[test]
fn a_picture_by_menu_letters_goes_in_and_backspace_takes_it_out() {
    let drive = ui_kit::drive::Driver::new();
    let picture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/sample-image.png");
    let mut app = app_with(&["before"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'I', 'P');
    chooser_answers(&drive, &mut app, picture, Chosen::Picture);
    let drawings = |app: &Scriva| {
        app.document
            .paragraphs()
            .iter()
            .flat_map(|p| p.runs())
            .flat_map(|run| run.content.iter())
            .filter(|piece| matches!(piece, wp_model::doc::Piece::Drawing(_)))
            .count()
    };
    assert_eq!(drawings(&app), 1, "the picture is in");
    assert!(app.message.is_none(), "{:?}", app.message);
    // The picture is left picked, as Word leaves one it has just put in, so
    // that the strip and the handles are there for it — and Backspace takes
    // it out as it would a character.
    assert!(app.picked.is_some(), "the picture just put in is picked");
    drive.press(&mut app, "Backspace");
    drive.settle(&mut app);
    assert_eq!(drawings(&app), 0, "Backspace took the picture out");
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
        .position(|(_, name)| name == "heading 1")
        .expect("heading 1 is a quick style");
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

/// File ▸ Print… by its letters, on a platform without a print path: it says
/// so and points at Export as PDF, and Enter puts the saying away.
#[cfg(not(windows))]
#[test]
fn print_by_menu_letters_says_where_printing_is() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["text"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'F', 'P');
    let said = app.message.clone().expect("a message");
    assert_eq!(said.0, "Cannot print");
    assert!(said.1.contains("PDF"), "{}", said.1);
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.message.is_none());
}

/// The rectangles a frame painted, with their fill, in the order they were
/// painted: the evidence for anything about how the page *looks*.
fn painted_rects(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Rect, egui::Color32, f32)> {
    shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => Some((rect.rect, rect.fill, rect.blur_width)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_page_sits_on_the_light_desk_with_a_shadow_and_no_fade() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Scriva::new();
    drive.settle(&mut app);
    let shapes = drive.frame_at(&mut app, Vec::new(), None);

    assert_eq!(view::desk(), ui_kit::theme::DESK);
    let rects = painted_rects(&shapes);
    let desk = rects
        .iter()
        .position(|(rect, fill, _)| *fill == ui_kit::theme::DESK && rect.width() > 1000.0)
        .expect("the desk is painted");
    let paper = rects
        .iter()
        .position(|(rect, fill, _)| *fill == egui::Color32::WHITE && rect.width() > 500.0)
        .expect("a page is painted");
    let shadow = rects
        .iter()
        .position(|(rect, _, blur)| *blur > 0.0 && rect.width() > 500.0)
        .expect("a shadow is painted");
    assert!(
        desk < shadow && shadow < paper,
        "desk, then shadow, then paper"
    );
    assert!(
        (rects[paper].0.top() - rects[desk].0.top() - view::GAP * view::SCALE as f32).abs() < 1.0,
        "the first page stands one gap below the toolbar"
    );

    // The blur along the bottom of the desk was egui's scroll-area fade: a
    // four-cornered mesh from clear to half-grey over the last twenty points.
    // Nothing of that shape is painted on the desk now.
    let desk_rect = rects[desk].0;
    let gradients = shapes.iter().filter(|clipped| match &clipped.shape {
        egui::Shape::Mesh(mesh) => {
            let bounds = clipped.shape.visual_bounding_rect();
            desk_rect.contains_rect(bounds)
                && bounds.width() > 500.0
                && mesh
                    .vertices
                    .iter()
                    .any(|v| v.color != mesh.vertices[0].color)
        }
        _ => false,
    });
    assert_eq!(gradients.count(), 0, "no gradient is painted over the desk");
}

/// The caret's stroke among the shapes a frame painted, if it was painted.
fn caret_in(shapes: &[egui::epaint::ClippedShape]) -> Option<egui::Rect> {
    painted_rects(shapes)
        .into_iter()
        .find(|(rect, fill, _)| *fill == view::CARET && rect.width() == view::CARET_WIDTH)
        .map(|(rect, _, _)| rect)
}

#[test]
fn the_caret_blinks_and_stands_solid_after_a_key() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Scriva::new();
    drive.frame_at(&mut app, Vec::new(), Some(10.0));
    drive.frame_at(&mut app, vec![egui::Event::Text("a".into())], Some(10.0));

    // Solid in the frame after the key, and for the whole first beat.
    let beat = ui_kit::theme::BLINK;
    let shapes = drive.frame_at(&mut app, Vec::new(), Some(10.0));
    assert!(caret_in(&shapes).is_some(), "shown at the key");
    let shapes = drive.frame_at(&mut app, Vec::new(), Some(10.0 + beat * 0.9));
    assert!(
        caret_in(&shapes).is_some(),
        "still shown before the first beat ends"
    );
    // Gone for the second beat, back for the third.
    let shapes = drive.frame_at(&mut app, Vec::new(), Some(10.0 + beat * 1.5));
    assert!(caret_in(&shapes).is_none(), "hidden in the second beat");
    let shapes = drive.frame_at(&mut app, Vec::new(), Some(10.0 + beat * 2.5));
    assert!(caret_in(&shapes).is_some(), "shown again in the third");

    // A key in the hidden beat brings it straight back.
    drive.frame_at(
        &mut app,
        vec![egui::Event::Text("b".into())],
        Some(10.0 + beat * 3.5),
    );
    let shapes = drive.frame_at(&mut app, Vec::new(), Some(10.0 + beat * 3.6));
    assert!(caret_in(&shapes).is_some(), "solid again from the key");

    // With a selection showing there is nothing to blink: the selection
    // says where the caret is.
    drive.press(&mut app, "shift+Home");
    assert!(!app.selection.is_empty());
    for beats in [0.5, 1.5, 2.5] {
        let shapes = drive.frame_at(&mut app, Vec::new(), Some(20.0 + beat * beats));
        assert!(
            caret_in(&shapes).is_some(),
            "solid with a selection at {beats}"
        );
    }
}

#[test]
fn the_carets_style_face_and_size_are_read_from_the_document() {
    use wp_model::units::HalfPoint;
    let mut app = app_with(&["plain words here", "second paragraph"]);
    let normal = app
        .document
        .styles
        .default_style(wp_model::StyleKind::Paragraph)
        .expect("a default paragraph style");
    assert_eq!(
        app.style_at(),
        Some(normal),
        "a fresh paragraph is in Normal"
    );
    let default_face = app
        .face_at()
        .expect("a face is resolved even when nothing names one");
    let default_size = app
        .size_at()
        .expect("a size is resolved even when nothing names one");
    assert_ne!(default_size, HalfPoint(28));

    // The word at the caret takes the change, and the caret reads it back.
    app.run(Command::Font("Verdana".to_owned()));
    app.run(Command::Size(HalfPoint(28)));
    assert_eq!(app.face_at().as_deref(), Some("Verdana"));
    assert_eq!(app.size_at(), Some(HalfPoint(28)));

    let heading = app
        .quick_styles()
        .into_iter()
        .find(|(_, name)| name.to_ascii_lowercase().starts_with("heading"))
        .map(|(id, _)| id)
        .expect("a heading style in a new document");
    app.run(Command::Style(heading));
    assert_eq!(app.style_at(), Some(heading));

    // A selection across the styled word and plain text mixes, and a mixed
    // selection is an empty box, not the first value.
    app.run(Command::SelectAll);
    assert_eq!(
        app.face_at(),
        None,
        "mixed faces: {default_face} and Verdana"
    );
    assert_eq!(app.size_at(), None, "mixed sizes");
    assert_eq!(app.style_at(), None, "mixed styles");
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

/// The horizontal line segments a frame painted in `colour`, as (y, x0, x1).
fn rules_in(shapes: &[egui::epaint::ClippedShape], colour: egui::Color32) -> Vec<(f32, f32, f32)> {
    shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == colour && (points[0].y - points[1].y).abs() < 0.01 =>
            {
                Some((
                    points[0].y,
                    points[0].x.min(points[1].x),
                    points[0].x.max(points[1].x),
                ))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_deletion_is_struck_and_an_insertion_underlined_on_the_page() {
    use wp_model::doc::{inserted_by, Inline, Piece, Run};
    use wp_model::{Mark, Revision};
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["kept "]);
    app.document.body[0] = Block::Paragraph(Paragraph {
        content: vec![
            Inline::Run(Run::of("kept ")),
            inserted_by("Adnan Khan", 1, vec![Inline::Run(Run::of("added "))]),
            Inline::Revised {
                revision: Revision::Deleted(Mark::new(2, "Adnan Khan")),
                content: vec![Inline::Run(Run {
                    content: vec![Piece::Deleted("gone".into())],
                    ..Run::default()
                })],
            },
        ],
        ..Paragraph::default()
    });
    app.changed();
    drive.settle(&mut app);
    let shapes = drive.frame_at(&mut app, Vec::new(), None);

    let colour = ui_kit::theme::author(0);
    let rules = rules_in(&shapes, colour);
    assert_eq!(
        rules.len(),
        2,
        "one underline and one strike, in the author's colour: {rules:?}"
    );
    let (underline, strike) = (rules[0], rules[1]);
    assert!(
        strike.0 < underline.0,
        "the strike crosses the word, the underline hangs under it: {rules:?}"
    );
    assert!(
        strike.1 >= underline.2 - 1.0,
        "the struck word comes after the inserted one: {rules:?}"
    );
    let bars: Vec<egui::Rect> = painted_rects(&shapes)
        .into_iter()
        .filter(|(rect, fill, _)| *fill == colour && rect.width() <= 3.0)
        .map(|(rect, _, _)| rect)
        .collect();
    assert_eq!(bars.len(), 1, "one change bar for the one marked line");
    assert!(
        bars[0].right() < underline.1,
        "and it stands in the left margin"
    );

    // With tracked changes hidden, the page is plain: no colour, no rules,
    // no bar.
    app.run(Command::ShowRevisions);
    drive.settle(&mut app);
    let shapes = drive.frame_at(&mut app, Vec::new(), None);
    assert!(rules_in(&shapes, colour).is_empty());
    assert!(painted_rects(&shapes)
        .iter()
        .all(|(_, fill, _)| *fill != colour));
}

#[test]
fn a_comment_washes_its_range_and_marks_the_margin() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["the quick fox"]);
    let quick = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 4,
        },
        head: Caret {
            paragraph: 0,
            offset: 9,
        },
    };
    crate::revise::add_comment(
        &mut app.document,
        &mut app.history,
        wp_model::Scope::Body,
        quick,
        "Reviewer",
        "R",
        "is it quick?",
    );
    app.changed();
    drive.settle(&mut app);
    let shapes = drive.frame_at(&mut app, Vec::new(), None);

    let wash = view::wash_colour(0);
    let washed: Vec<egui::Rect> = painted_rects(&shapes)
        .into_iter()
        .filter(|(_, fill, _)| *fill == wash)
        .map(|(rect, _, _)| rect)
        .collect();
    assert_eq!(washed.len(), 1, "one band under the one commented word");
    assert!(
        washed[0].width() > 10.0 && washed[0].width() < 80.0,
        "the width of a word: {washed:?}"
    );

    let colour = ui_kit::theme::author(0);
    let marker = shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Path(path) if path.fill == colour => Some(path.visual_bounding_rect()),
            _ => None,
        })
        .expect("a marker in the author's colour");
    assert!(
        marker.left() > washed[0].right() + 100.0,
        "in the right margin, past the text: {marker:?}"
    );
    assert!(
        (marker.center().y - washed[0].center().y).abs() < 20.0,
        "level with the word's line"
    );

    // View ▸ Comments off: neither.
    app.run(Command::ShowComments);
    drive.settle(&mut app);
    let shapes = drive.frame_at(&mut app, Vec::new(), None);
    assert!(painted_rects(&shapes)
        .iter()
        .all(|(_, fill, _)| *fill != wash));
}

#[test]
fn a_comment_with_no_selection_takes_the_word_at_the_caret() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["first word here"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 7,
    });
    drive.settle(&mut app);
    drive.press(&mut app, "ctrl+alt+M");
    drive.settle(&mut app);
    assert!(app.message.is_none(), "no refusal");
    assert!(app.reviewer, "the pane opened");
    let draft = app.draft.clone().expect("a draft in the pane");
    assert_eq!(
        draft.range.ordered(),
        (
            Caret {
                paragraph: 0,
                offset: 6
            },
            Caret {
                paragraph: 0,
                offset: 10
            }
        ),
        "the word at the caret"
    );
    drive.type_text(&mut app, "why this word?");
    drive.press(&mut app, "ctrl+Enter");
    drive.settle(&mut app);
    assert!(app.draft.is_none());
    assert_eq!(app.document.comments.len(), 1);
    assert_eq!(app.document.comments[0].text(), "why this word?");
    let ranges = crate::revise::comment_ranges(&app.document);
    assert_eq!(ranges[0].range.ordered().0.offset, 6);
    assert_eq!(ranges[0].range.ordered().1.offset, 10);
    assert_eq!(
        app.document.text(),
        "first word here",
        "nothing typed into the text"
    );
}

#[test]
fn a_draft_comment_is_posted_with_ctrl_enter_and_discarded_with_escape() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["a word to comment on"]);
    drive.settle(&mut app);
    drive.press(&mut app, "ctrl+A");
    drive.press(&mut app, "ctrl+alt+M");
    drive.settle(&mut app);
    drive.type_text(&mut app, "thrown away");
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.draft.is_none(), "Escape discarded the draft");
    assert!(
        app.document.comments.is_empty(),
        "and no comment was posted"
    );
    assert!(app.reviewer, "the pane stays open");

    drive.press(&mut app, "ctrl+alt+M");
    drive.settle(&mut app);
    drive.type_text(&mut app, "kept");
    drive.press(&mut app, "ctrl+Enter");
    drive.settle(&mut app);
    assert!(app.draft.is_none());
    assert_eq!(app.document.comments.len(), 1);
    assert_eq!(app.document.comments[0].text(), "kept");
    assert_eq!(app.document.text(), "a word to comment on");
}

/// A document with an insertion in one paragraph and a deletion in the
/// next, both by the same author.
fn with_two_changes() -> Scriva {
    use wp_model::doc::{inserted_by, Inline, Piece, Run};
    use wp_model::{Mark, Revision};
    let mut app = app_with(&["one", "two"]);
    app.document.body[0] = Block::Paragraph(Paragraph {
        content: vec![
            Inline::Run(Run::of("kept ")),
            inserted_by("Adnan Khan", 1, vec![Inline::Run(Run::of("added"))]),
        ],
        ..Paragraph::default()
    });
    app.document.body[1] = Block::Paragraph(Paragraph {
        content: vec![
            Inline::Run(Run::of("stays ")),
            Inline::Revised {
                revision: Revision::Deleted(Mark::new(2, "Adnan Khan")),
                content: vec![Inline::Run(Run {
                    content: vec![Piece::Deleted("gone".into())],
                    ..Run::default()
                })],
            },
        ],
        ..Paragraph::default()
    });
    app.changed();
    app
}

#[test]
fn one_change_is_settled_from_its_card_without_touching_the_others() {
    use crate::panes::review::{drawn, CardKey};
    let drive = ui_kit::drive::Driver::new();
    let mut app = with_two_changes();
    app.run(Command::Reviewer);
    drive.settle(&mut app);
    drive.settle(&mut app);
    let cards = drawn(drive.ctx());
    assert_eq!(cards.len(), 2, "one card per change: {cards:?}");
    let insertion = cards
        .iter()
        .find(|card| matches!(&card.key, CardKey::Change(mark) if mark.id == 1))
        .expect("the insertion's card");
    let reject = insertion
        .actions
        .iter()
        .find(|(name, _)| *name == "Reject")
        .expect("with a Reject button")
        .1;
    drive.click(&mut app, reject.center());
    let left = crate::revise::tracked(&app.document);
    assert_eq!(left.len(), 1, "one change settled, one left: {left:?}");
    assert_eq!(left[0].what, "deleted");
    assert_eq!(
        app.document.paragraphs()[0].text(),
        "kept ",
        "the insertion went"
    );
    assert_eq!(drawn(drive.ctx()).len(), 1, "and its card with it");
}

#[test]
fn the_card_at_the_caret_is_the_outlined_one() {
    use crate::panes::review::{drawn, CardKey};
    let drive = ui_kit::drive::Driver::new();
    let mut app = with_two_changes();
    app.run(Command::Reviewer);
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 0,
    });
    drive.settle(&mut app);
    drive.settle(&mut app);
    let outlined = |cards: &[crate::panes::review::CardDrawn]| -> Vec<u32> {
        cards
            .iter()
            .filter(|card| card.at_caret)
            .filter_map(|card| match &card.key {
                CardKey::Change(mark) => Some(mark.id),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        outlined(&drawn(drive.ctx())),
        vec![2],
        "the deletion, in the caret's paragraph"
    );
    drive.press(&mut app, "ctrl+Home");
    drive.settle(&mut app);
    assert_eq!(
        outlined(&drawn(drive.ctx())),
        vec![1],
        "the insertion, once the caret moved up"
    );
}

/// Four paragraphs, two of them headings, for the Navigate pane.
fn with_headings() -> Scriva {
    let mut app = app_with(&["Intro", "words under intro", "Method", "words under method"]);
    let heading = app
        .quick_styles()
        .into_iter()
        .find(|(_, name)| name.to_ascii_lowercase().starts_with("heading"))
        .map(|(id, _)| id)
        .expect("a heading style");
    for index in [0, 2] {
        if let Block::Paragraph(paragraph) = &mut app.document.body[index] {
            paragraph.props.style = Some(heading);
        }
    }
    app.changed();
    app
}

#[test]
fn f6_lands_in_the_navigate_pane_when_open_and_skips_it_when_closed() {
    use crate::app::Keyboard;
    let drive = ui_kit::drive::Driver::new();
    let mut app = with_headings();
    drive.settle(&mut app);
    assert_eq!(app.keyboard, Keyboard::Document);
    drive.press(&mut app, "F6");
    drive.settle(&mut app);
    assert_eq!(
        app.keyboard,
        Keyboard::Toolbar,
        "no pane open: the toolbar is next"
    );
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert_eq!(app.keyboard, Keyboard::Document, "Escape brings it home");

    app.run(Command::Navigator);
    drive.settle(&mut app);
    drive.press(&mut app, "F6");
    drive.settle(&mut app);
    assert_eq!(app.keyboard, Keyboard::Navigate);
    drive.press(&mut app, "F6");
    drive.settle(&mut app);
    assert_eq!(app.keyboard, Keyboard::Toolbar);
    drive.press(&mut app, "shift+F6");
    drive.settle(&mut app);
    assert_eq!(app.keyboard, Keyboard::Navigate, "and back the other way");
    drive.press(&mut app, "shift+F6");
    drive.settle(&mut app);
    assert_eq!(app.keyboard, Keyboard::Document);
}

#[test]
fn enter_on_a_heading_moves_the_caret_and_returns_the_keyboard_to_the_document() {
    use crate::app::Keyboard;
    let drive = ui_kit::drive::Driver::new();
    let mut app = with_headings();
    app.run(Command::Navigator);
    drive.settle(&mut app);
    drive.press(&mut app, "F6");
    drive.settle(&mut app);
    assert_eq!(app.keyboard, Keyboard::Navigate);
    drive.press(&mut app, "ArrowDown");
    drive.settle(&mut app);
    let lit: Vec<Option<usize>> = crate::panes::navigate::drawn(drive.ctx())
        .into_iter()
        .filter(|row| row.lit)
        .map(|row| row.paragraph)
        .collect();
    assert_eq!(lit, vec![Some(2)], "Down lit the second heading");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert_eq!(app.caret().paragraph, 2, "Enter went to it");
    assert_eq!(
        app.keyboard,
        Keyboard::Document,
        "and left the keyboard in the document"
    );
    drive.type_text(&mut app, "X");
    assert_eq!(
        app.document.paragraphs()[2].text(),
        "XMethod",
        "which takes the typing"
    );
}

#[test]
fn escape_closes_the_band_first_and_the_find_bar_second() {
    use crate::app::Keyboard;
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["some text"]);
    drive.settle(&mut app);
    drive.press(&mut app, "ctrl+F");
    drive.settle(&mut app);
    assert!(app.finder.is_some());
    assert_eq!(
        app.keyboard,
        Keyboard::Find,
        "Ctrl+F put the keyboard in the bar"
    );
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.finder.is_some(), "Escape in the bar leaves it open");
    assert_eq!(
        app.keyboard,
        Keyboard::Document,
        "and gives the keyboard back"
    );

    app.run(Command::EditHeader);
    drive.settle(&mut app);
    assert!(app.editing_band());
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(!app.editing_band(), "the first Escape closed the header");
    assert!(app.finder.is_some(), "and not the find bar");
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.finder.is_none(), "the second closed the find bar");
}

#[test]
fn the_heading_containing_the_caret_is_the_lit_row() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = with_headings();
    app.run(Command::Navigator);
    drive.settle(&mut app);
    drive.settle(&mut app);
    let lit = |ctx: &egui::Context| -> Vec<Option<usize>> {
        crate::panes::navigate::drawn(ctx)
            .into_iter()
            .filter(|row| row.lit)
            .map(|row| row.paragraph)
            .collect()
    };
    assert_eq!(
        lit(drive.ctx()),
        vec![Some(0)],
        "the caret starts under Intro"
    );
    drive.press(&mut app, "ctrl+End");
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert_eq!(
        lit(drive.ctx()),
        vec![Some(2)],
        "Ctrl+End puts it under Method"
    );
}

/// A picture put in beside text is picked as *that* picture: the second
/// drawing of the paragraph when one stood before it.
#[test]
fn a_picture_just_put_in_is_the_picked_one() {
    let mut app = app_with(&["ab"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 2,
    });
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48));
    assert_eq!(
        app.picked,
        Some(crate::drawings::Picked {
            paragraph: 0,
            nth: 0
        })
    );
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 1,
    });
    app.picked = None;
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48));
    assert_eq!(
        app.picked,
        Some(crate::drawings::Picked {
            paragraph: 0,
            nth: 0
        }),
        "put in before the first, it is the first"
    );
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: text::len(app.document.paragraphs()[0]),
    });
    assert!(app.insert_picture(PIXEL, "image/png", 96, 48));
    assert_eq!(
        app.picked.map(|picked| picked.nth),
        Some(2),
        "put in at the end, it is the third"
    );
}

/// Table ▸ Insert ▸ Row Below by its letters, from the last cell of the
/// first row: the new row is under the caret's, empty, and Tab — which goes
/// from the row's last cell to the next row's first — lands in it. One undo
/// takes the row away.
#[test]
fn a_row_inserted_below_by_menu_takes_tab_into_its_first_cell() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["after"]);
    drive.settle(&mut app);
    app.insert_table(2, 2);
    drive.type_text(&mut app, "a1");
    drive.press(&mut app, "Tab");
    drive.type_text(&mut app, "b1");
    drive.settle(&mut app);
    let table = |app: &Scriva| match &app.document.body[0] {
        Block::Table(table) => table.clone(),
        other => panic!("the table is the first block, not {other:?}"),
    };
    assert_eq!(table(&app).rows.len(), 2);
    drive.menu(&mut app, 'A', 'I');
    drive.press(&mut app, "B");
    drive.settle(&mut app);
    let now = table(&app);
    assert_eq!(now.rows.len(), 3, "a row went in");
    assert_eq!(
        now.rows[0].text(),
        "a1\tb1",
        "above the caret's row, untouched"
    );
    assert_eq!(now.rows[1].text(), "\t", "the new one, empty");
    assert_eq!(now.rows[1].cells.len(), 2);
    assert_eq!(
        edit::table_cell_at(&app.document, app.scope, app.caret()),
        Some((0, 0, 1)),
        "the caret stayed in its cell"
    );
    drive.press(&mut app, "Tab");
    drive.settle(&mut app);
    assert_eq!(
        edit::table_cell_at(&app.document, app.scope, app.caret()),
        Some((0, 1, 0)),
        "and Tab went into the new row's first cell"
    );
    let text: String = app
        .document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .collect();
    assert_eq!(text, "a1b1after", "no letter of the sequence was typed");
    app.run(Command::Undo);
    assert_eq!(table(&app).rows.len(), 2, "one undo takes the row away");
    assert_eq!(table(&app).rows[1].text(), "\t");
}

/// Delete ▸ Column on the middle of three: the grid loses that column's
/// width, the other two keep theirs, every row is a cell shorter, the caret
/// lands in the cell that took its place, and undo puts it all back.
#[test]
fn deleting_the_caret_column_narrows_the_grid_and_nothing_else() {
    let mut app = app_with(&["after"]);
    app.insert_table(2, 3);
    let widths = [Twips(2880), Twips(5760), Twips(1440)];
    if let Block::Table(table) = &mut app.document.body[0] {
        table.grid = widths.to_vec();
        for row in &mut table.rows {
            for (cell, width) in row.cells.iter_mut().zip(widths) {
                cell.props.width = wp_model::table::Width::Fixed(width);
                cell.content = vec![Block::Paragraph(Paragraph::of(&format!("{}", width.0)))];
            }
        }
    }
    app.stamp += 1;
    // Into the middle column of the second row.
    let range = edit::cell_paragraphs(&app.document, app.scope, 0, 1, 1).expect("cell (1, 1)");
    app.selection = Selection::at(Caret {
        paragraph: range.start,
        offset: 0,
    });
    app.run(Command::DeleteColumn);
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is still there");
    };
    assert_eq!(
        table.grid,
        vec![Twips(2880), Twips(1440)],
        "the grid lost the middle"
    );
    for row in &table.rows {
        assert_eq!(row.text(), "2880\t1440", "each row lost its middle cell");
        assert_eq!(
            row.cells[0].props.width,
            wp_model::table::Width::Fixed(Twips(2880)),
            "and the others keep their widths"
        );
        assert_eq!(
            row.cells[1].props.width,
            wp_model::table::Width::Fixed(Twips(1440))
        );
    }
    assert_eq!(
        edit::table_cell_at(&app.document, app.scope, app.caret()),
        Some((0, 1, 1)),
        "the caret is in the cell that took the place"
    );
    assert!(app.message.is_none(), "{:?}", app.message);
    app.run(Command::Undo);
    let Block::Table(table) = &app.document.body[0] else {
        panic!("the table is still there after undo");
    };
    assert_eq!(
        table.grid,
        widths.to_vec(),
        "one undo brings the column back"
    );
    assert_eq!(table.rows[1].text(), "2880\t5760\t1440");
}

/// Delete ▸ Table leaves an empty paragraph where the table stood — a
/// document is never left with nothing where the caret is — with the caret
/// on it, and one undo brings the table back whole.
#[test]
fn deleting_a_table_leaves_an_empty_paragraph_and_undo_brings_it_back() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["after"]);
    drive.settle(&mut app);
    app.insert_table(2, 2);
    drive.type_text(&mut app, "cell");
    drive.settle(&mut app);
    assert!(matches!(app.document.body[0], Block::Table(_)));
    app.run(Command::DeleteTable);
    assert_eq!(app.document.body.len(), 2);
    assert!(
        matches!(&app.document.body[0], Block::Paragraph(p) if p.text().is_empty()),
        "an empty paragraph where the table was"
    );
    assert_eq!(app.document.paragraphs()[1].text(), "after");
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 0,
            offset: 0
        },
        "the caret is on it"
    );
    assert!(app.message.is_none(), "{:?}", app.message);
    app.run(Command::Undo);
    let Block::Table(table) = &app.document.body[0] else {
        panic!("undo brings the table back");
    };
    assert_eq!(table.rows[0].text(), "cell\t");
    assert_eq!(
        app.document.paragraphs().last().map(|p| p.text()),
        Some("after".to_owned())
    );

    // Deleting the only row, or the only column, is deleting the table.
    let mut app = app_with(&["after"]);
    app.insert_table(1, 2);
    app.run(Command::DeleteRow);
    assert!(
        matches!(app.document.body[0], Block::Paragraph(_)),
        "the last row took the table"
    );
    let mut app = app_with(&["after"]);
    app.insert_table(2, 1);
    app.run(Command::DeleteColumn);
    assert!(
        matches!(app.document.body[0], Block::Paragraph(_)),
        "the last column took the table"
    );
}
