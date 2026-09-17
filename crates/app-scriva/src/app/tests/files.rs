//! Opening, saving, the formats between them, and what the window says about a file.

use super::*;

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

#[test]
fn a_document_with_no_path_is_still_called_something() {
    let app = Scriva::new();
    let (name, dirty) = app.document().expect("a title");
    assert_eq!(name, "Document1");
    assert!(!dirty);
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

    let drive = ui_kit::drive::Driver::new();
    for _ in 0..200 {
        if app.asking.is_none() {
            break;
        }
        drive.settle(&mut app);
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

/// File ▸ Print… by its letters, on a platform without a print path: it says
/// so and points at Export as PDF, and Enter puts the saying away.
#[cfg(not(windows))]
#[test]
fn print_by_menu_letters_says_where_printing_is() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["text"]);
    drive.settle(&mut app);
    drive.menu(&mut app, 'F', 'P');
    drive.settle(&mut app);
    assert!(
        app.message.is_none(),
        "no box: a fact about this platform is not a question"
    );
    let notice = app.notices.first().expect("the notice bar says so");
    assert!(notice.text.contains("Export a PDF"), "{}", notice.text);
    assert_eq!(
        notice.action.as_ref().map(|(_, command)| command),
        Some(&Command::ExportPdf),
        "with the way out on it"
    );
    // The same notice twice is one notice.
    drive.menu(&mut app, 'F', 'P');
    drive.settle(&mut app);
    assert_eq!(app.notices.len(), 1);
}

/// A save says `Saved <name>` at the left of the status bar for four
/// seconds — read off the driver's clock — and not after.
#[test]
fn a_save_says_so_in_the_status_for_four_seconds_and_not_after() {
    let drive = ui_kit::drive::Driver::new();
    let dir = scratch("status-notice");
    let path = dir.join("said.docx");
    let mut app = app_with(&["words"]);
    app.path = Some(path.clone());
    drive.settle(&mut app);
    drive.frame_at(&mut app, Vec::new(), Some(100.0));
    app.run(Command::Save);
    assert!(path.is_file(), "the save wrote the file");
    let shown = |app: &Scriva| app.notice.as_ref().map(|(text, _)| text.clone());
    assert_eq!(shown(&app), Some("Saved said.docx".to_owned()));
    // Stamped on its first frame, still there at 3.9 seconds, gone at 4.1.
    drive.frame_at(&mut app, Vec::new(), Some(100.5));
    assert_eq!(shown(&app), Some("Saved said.docx".to_owned()), "shown");
    drive.frame_at(&mut app, Vec::new(), Some(104.4));
    assert_eq!(
        shown(&app),
        Some("Saved said.docx".to_owned()),
        "still shown at 3.9 s"
    );
    drive.frame_at(&mut app, Vec::new(), Some(104.6));
    assert_eq!(shown(&app), None, "gone after four seconds");
    // A new notice replaces the old: a queue of one.
    app.say("first");
    app.say("second");
    drive.frame_at(&mut app, Vec::new(), Some(105.0));
    assert_eq!(shown(&app), Some("second".to_owned()));
}

/// A document dropped on the window opens through the same guard Open
/// takes: a clean document opens it at once, and a dirty one asks first.
#[test]
fn a_dropped_document_opens_through_the_unsaved_guard() {
    let drive = ui_kit::drive::Driver::new();
    let document = corpus_docx("comments.docx");
    // Clean: it opens.
    let mut app = app_with(&["fresh"]);
    app.dirty = false;
    drive.settle(&mut app);
    drive.drop_files(&mut app, std::slice::from_ref(&document));
    drive.settle(&mut app);
    assert_eq!(
        app.path.as_deref(),
        Some(document.as_path()),
        "the dropped document opened"
    );
    assert!(app.pending.is_none());
    // Dirty: it asks, and Cancel keeps what was there.
    let mut app = app_with(&["unsaved words"]);
    app.dirty = true;
    drive.settle(&mut app);
    drive.drop_files(&mut app, std::slice::from_ref(&document));
    drive.settle(&mut app);
    assert!(
        matches!(&app.pending, Some(Pending::Unsaved(command)) if **command == Command::Reopen(document.clone())),
        "the guard asks first: {:?}",
        app.pending
    );
    assert_eq!(app.document.paragraphs()[0].text(), "unsaved words");
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.pending.is_none(), "Cancel put the question away");
    assert_eq!(
        app.document.paragraphs()[0].text(),
        "unsaved words",
        "and kept the document"
    );
    assert!(app.path.is_none());
}

/// A `.doc` opened shows the copy notice in the bar under the toolbar,
/// with no box to dismiss first, and its `×` takes it away.
#[test]
fn a_doc_opened_shows_the_notice_bar_and_no_box() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Scriva::new();
    drive.settle(&mut app);
    app.open_path(&corpus_doc("plain-paragraphs.doc"));
    drive.settle(&mut app);
    assert!(app.message.is_none(), "no box: {:?}", app.message);
    let notice = app.notices.first().expect("the notice bar has the fact");
    assert!(
        notice
            .text
            .starts_with("Opened as a copy of plain-paragraphs.doc"),
        "{}",
        notice.text
    );
    assert!(
        notice.text.contains("plain-paragraphs.docx"),
        "{}",
        notice.text
    );
    // Opening another document clears the bar: the fact was about that one.
    app.open_path(&corpus_docx("comments.docx"));
    drive.settle(&mut app);
    assert!(
        app.notices.is_empty(),
        "the notice was about the other document"
    );
}

/// A Word 97-2003 document from the corpus, by name.
fn corpus_doc(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/doc")
        .join(name)
}
