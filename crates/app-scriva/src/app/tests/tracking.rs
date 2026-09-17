//! Track Changes, driven by the keys a person presses. Backspace, Delete,
//! Enter and pasting once went around it: with Track Changes on, they changed
//! the text and recorded nothing. The shapes are Word's, measured (the story
//! workspace's `bugs/evidence/word/track-keys.ps1`).

use super::*;

/// "Title words" as a heading and "Body words" after it, Track Changes on.
fn tracking() -> Scriva {
    let mut app = app_with(&["Title words", "Body words"]);
    let heading = app.document.styles.lookup("Heading1").expect("Heading 1");
    if let Block::Paragraph(paragraph) = &mut app.document.body[0] {
        paragraph.props.style = Some(heading);
    }
    app.document.settings.track_changes = true;
    app
}

fn heading(app: &Scriva) -> Option<wp_model::StyleId> {
    app.document.styles.lookup("Heading1")
}

fn put(app: &mut Scriva, paragraph: usize, offset: usize) {
    app.selection = Selection::at(Caret { paragraph, offset });
}

fn listed(app: &Scriva) -> Vec<(usize, &'static str, String)> {
    crate::revise::tracked(&app.document)
        .into_iter()
        .map(|change| (change.paragraph, change.what, change.text))
        .collect()
}

fn texts(app: &Scriva) -> Vec<(Option<wp_model::StyleId>, String)> {
    app.document
        .paragraphs()
        .iter()
        .map(|paragraph| (paragraph.props.style, paragraph.text()))
        .collect()
}

#[test]
fn backspace_and_delete_mark_the_text_deleted_with_track_changes_on() {
    let drive = Driver::new();
    let mut app = tracking();
    let before = app.document.body.clone();
    drive.settle(&mut app);
    put(&mut app, 1, 10);
    drive.press(&mut app, "Backspace");
    drive.press(&mut app, "Backspace");
    assert_eq!(app.document.paragraphs()[1].text(), "Body wor");
    assert_eq!(
        app.document.paragraphs()[1].shown_text(),
        "Body words",
        "still there, struck"
    );
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 1,
            offset: 8
        }
    );
    assert_eq!(
        listed(&app),
        [(1, "deleted", "ds".to_owned())],
        "one change"
    );

    put(&mut app, 1, 0);
    drive.press(&mut app, "Delete");
    assert_eq!(app.document.paragraphs()[1].text(), "ody wor");
    put(&mut app, 1, 7);
    drive.press(&mut app, "ctrl+Backspace");
    assert_eq!(app.document.paragraphs()[1].text(), "ody ");
    assert_eq!(
        listed(&app),
        [
            (1, "deleted", "B".to_owned()),
            (1, "deleted", "words".to_owned())
        ],
        "the word joins the letters deleted after it"
    );

    // Taken back one step at a time, and settled.
    let tracked = app.document.body.clone();
    app.run(Command::RejectAll);
    assert_eq!(
        app.document.body, before,
        "rejected, the text is back as it was"
    );
    app.run(Command::Undo);
    assert_eq!(app.document.body, tracked);
    app.run(Command::AcceptAll);
    assert_eq!(texts(&app)[1].1, "ody ");
}

#[test]
fn backspace_at_a_paragraphs_start_and_enter_are_tracked_as_word_tracks_them() {
    let drive = Driver::new();
    let mut app = tracking();
    drive.settle(&mut app);
    put(&mut app, 1, 0);
    drive.press(&mut app, "Backspace");
    assert_eq!(
        app.document.paragraphs().len(),
        2,
        "the paragraphs stand until the deletion is accepted"
    );
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 0,
            offset: 11
        }
    );
    assert_eq!(
        listed(&app),
        [
            (0, "paragraph break deleted", String::new()),
            (1, "paragraph formatting changed", "Body words".to_owned()),
        ]
    );
    app.run(Command::AcceptAll);
    assert_eq!(
        texts(&app),
        [(heading(&app), "Title wordsBody words".to_owned())],
        "accepted, the heading's words keep their look"
    );

    // Enter inside the heading, then at the end of what follows, and words.
    let mut app = tracking();
    let before = app.document.body.clone();
    drive.settle(&mut app);
    put(&mut app, 0, 5);
    drive.press(&mut app, "Enter");
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 1,
            offset: 0
        }
    );
    assert_eq!(
        listed(&app),
        [(0, "paragraph break inserted", String::new())]
    );
    put(&mut app, 1, 6);
    drive.press(&mut app, "Enter");
    drive.type_text(&mut app, "New");
    let normal = app.document.styles.lookup("Normal");
    assert_eq!(
        texts(&app),
        [
            (heading(&app), "Title".to_owned()),
            (heading(&app), " words".to_owned()),
            (normal, "New".to_owned()),
            (None, "Body words".to_owned()),
        ]
    );
    assert_eq!(
        listed(&app)[2..],
        [
            (2, "paragraph formatting changed", "New".to_owned()),
            (2, "inserted", "New".to_owned()),
        ]
    );
    app.run(Command::RejectAll);
    assert_eq!(app.document.body, before);

    // Enter and Backspace straight after leave nothing to review.
    let mut app = tracking();
    let before = app.document.body.clone();
    drive.settle(&mut app);
    put(&mut app, 0, 11);
    drive.press(&mut app, "Enter");
    drive.press(&mut app, "Backspace");
    assert_eq!(app.document.body, before);
    assert!(listed(&app).is_empty());
}

#[test]
fn a_selection_deleted_typed_over_or_pasted_over_is_tracked_marks_and_all() {
    let drive = Driver::new();
    let mut app = tracking();
    drive.settle(&mut app);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 6,
        },
        head: Caret {
            paragraph: 1,
            offset: 5,
        },
    };
    drive.press(&mut app, "Delete");
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 0,
            offset: 6
        }
    );
    assert_eq!(
        listed(&app),
        [
            (0, "deleted", "words".to_owned()),
            (0, "paragraph break deleted", String::new()),
            (1, "paragraph formatting changed", "words".to_owned()),
            (1, "deleted", "Body".to_owned()),
        ]
    );
    app.run(Command::AcceptAll);
    assert_eq!(texts(&app), [(heading(&app), "Title words".to_owned())]);

    // Cut, or replaced with nothing: the same.
    let mut app = tracking();
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 6,
        },
        head: Caret {
            paragraph: 1,
            offset: 5,
        },
    };
    app.replace_selection("");
    assert_eq!(app.document.paragraphs().len(), 2);
    assert_eq!(listed(&app).len(), 4);

    // Typed over: the selection deleted, the words inserted.
    let mut app = tracking();
    app.selection = Selection {
        anchor: Caret {
            paragraph: 1,
            offset: 0,
        },
        head: Caret {
            paragraph: 1,
            offset: 4,
        },
    };
    app.type_text("Some");
    assert_eq!(
        listed(&app),
        [
            (1, "deleted", "Body".to_owned()),
            (1, "inserted", "Some".to_owned())
        ],
        "the old words struck, then the new, as Word types them (`type-over`)"
    );

    // Lines pasted: the words inserted, and the break between them.
    let mut app = tracking();
    put(&mut app, 1, 0);
    app.paste_text("one\ntwo ");
    assert_eq!(
        listed(&app),
        [
            (1, "inserted", "one".to_owned()),
            (1, "paragraph break inserted", String::new()),
            (2, "inserted", "two".to_owned()),
        ]
    );
    assert_eq!(app.document.text(), "Title words\none\ntwo Body words");
    app.run(Command::RejectAll);
    assert_eq!(app.document.text(), "Title words\nBody words");

    // Paragraphs copied here and pasted: inserted, breaks and all.
    let mut app = tracking();
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 6,
        },
        head: Caret {
            paragraph: 1,
            offset: 4,
        },
    };
    let text = app.selected_text().expect("something to copy");
    app.clipboard = Some(Clip {
        text: text.clone(),
        paragraphs: edit::copy_range(&app.document, app.scope, app.selection),
    });
    put(&mut app, 1, 10);
    app.paste_matching(&text);
    assert_eq!(
        listed(&app),
        [
            (1, "inserted", "words".to_owned()),
            (1, "paragraph break inserted", String::new()),
            (2, "inserted", "Body".to_owned()),
        ]
    );

    // A tab is typed like anything else.
    let mut app = tracking();
    drive.settle(&mut app);
    put(&mut app, 1, 0);
    drive.press(&mut app, "Tab");
    assert_eq!(listed(&app), [(1, "inserted", String::new())]);
}

/// What Track Changes cannot record it says, and changes nothing: a half
/// recorded change is worse than none.
#[test]
fn what_track_changes_cannot_record_is_said_and_left_alone() {
    let drive = Driver::new();
    let mut app = tracking();
    app.document.body[1] = Block::Paragraph(Paragraph {
        content: vec![
            wp_model::doc::Inline::Run(wp_model::doc::Run::of("see ")),
            wp_model::doc::Inline::Hyperlink(Box::new(wp_model::Hyperlink {
                rel: None,
                anchor: Some("x".into()),
                tooltip: None,
                history: true,
                content: vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of("linked"))],
            })),
        ],
        ..Paragraph::default()
    });
    app.changed();
    let before = app.document.body.clone();
    drive.settle(&mut app);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 1,
            offset: 2,
        },
        head: Caret {
            paragraph: 1,
            offset: 7,
        },
    };
    drive.press(&mut app, "Delete");
    assert_eq!(app.document.body, before);
    assert_eq!(
        app.notice.as_ref().map(|(said, _)| said.as_str()),
        Some(crate::revise::CANNOT_RECORD)
    );
    // Nor is what was typed over it typed.
    app.notice = None;
    app.type_text("x");
    assert_eq!(app.document.body, before);
    assert_eq!(
        app.notice.as_ref().map(|(said, _)| said.as_str()),
        Some(crate::revise::CANNOT_RECORD)
    );
    // Nor is a picture: nothing goes into the package, and the refusal is
    // the answer rather than "nowhere to keep a picture".
    app.notice = None;
    put(&mut app, 1, 7);
    assert!(app.insert_picture(b"not read", "image/png", 10, 10));
    assert!(app.package.is_none(), "no part was made for it");
    assert_eq!(app.message, None);
    assert_eq!(
        app.notice.as_ref().map(|(said, _)| said.as_str()),
        Some(crate::revise::CANNOT_RECORD)
    );
    // Nor is any line of a paste put in, inside the link.
    app.notice = None;
    put(&mut app, 1, 7);
    app.paste_text("one\ntwo");
    assert_eq!(app.document.body, before);
    assert_eq!(
        app.notice.as_ref().map(|(said, _)| said.as_str()),
        Some(crate::revise::CANNOT_RECORD)
    );
}

/// Ctrl+Enter over a selection, and Delete on a picked picture, changed the
/// document with Track Changes on and recorded nothing.
#[test]
fn ctrl_enter_and_deleting_a_picture_are_tracked() {
    let drive = Driver::new();
    let mut app = tracking();
    let before = app.document.body.clone();
    drive.settle(&mut app);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 1,
            offset: 0,
        },
        head: Caret {
            paragraph: 1,
            offset: 5,
        },
    };
    drive.press(&mut app, "ctrl+Enter");
    assert_eq!(
        listed(&app),
        [
            (1, "inserted", String::new()),
            (1, "paragraph break inserted", String::new()),
            (2, "deleted", "Body".to_owned()),
        ],
        "the break before what it replaced, as Word's `page-break-over`"
    );
    app.run(Command::RejectAll);
    assert_eq!(app.document.body, before);

    let mut app = tracking();
    app.document.body[1] =
        Block::Paragraph(picture_paragraph("rId9", &app.document.section, 10, 10));
    app.changed();
    drive.settle(&mut app);
    app.picked = Some(crate::drawings::Picked {
        paragraph: 1,
        nth: 0,
    });
    drive.press(&mut app, "Delete");
    assert_eq!(listed(&app), [(1, "deleted", String::new())]);
    assert_eq!(
        app.document.paragraphs()[1].drawings().len(),
        1,
        "struck, and still there to see"
    );
    app.run(Command::RejectAll);
    assert!(listed(&app).is_empty());
    assert_eq!(app.document.paragraphs()[1].drawings().len(), 1);
}

/// At a cell's edge, Backspace and Delete do nothing, as Word's do, and say
/// nothing: they once said a hyperlink was in the way.
#[test]
fn backspace_and_delete_at_a_cells_edge_do_nothing() {
    let drive = Driver::new();
    let mut app = tracking();
    app.document.body.insert(
        1,
        Block::Table(wp_model::table::Table {
            rows: vec![wp_model::table::Row {
                cells: vec![wp_model::table::Cell {
                    props: wp_model::table::CellProps::new(),
                    content: vec![Block::Paragraph(Paragraph::of("cell"))],
                }],
                ..wp_model::table::Row::new()
            }],
            ..wp_model::table::Table::new()
        }),
    );
    app.changed();
    let before = app.document.body.clone();
    drive.settle(&mut app);
    for (caret, key) in [
        (
            Caret {
                paragraph: 1,
                offset: 0,
            },
            "Backspace",
        ),
        (
            Caret {
                paragraph: 1,
                offset: 4,
            },
            "Delete",
        ),
    ] {
        app.selection = Selection::at(caret);
        drive.press(&mut app, key);
        assert_eq!(app.document.body, before, "{key}");
        assert_eq!(app.notice, None, "{key}");
    }
    // Ctrl+Enter in a cell splits the table, which is not recorded: said,
    // before the selection it would replace is touched.
    app.selection = Selection {
        anchor: Caret {
            paragraph: 1,
            offset: 1,
        },
        head: Caret {
            paragraph: 1,
            offset: 3,
        },
    };
    drive.press(&mut app, "ctrl+Enter");
    assert_eq!(app.document.body, before);
    assert_eq!(
        app.notice.as_ref().map(|(said, _)| said.as_str()),
        Some(crate::revise::TABLE_UNTRACKED)
    );
    // A picture, or a chart, over a selection around the table: refused
    // before any part is made, in words about the table.
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 2,
        },
        head: Caret {
            paragraph: 2,
            offset: 2,
        },
    };
    app.notice = None;
    assert!(app.insert_picture(b"not read", "image/png", 10, 10));
    assert!(app.insert_chart_part(b"<c:chartSpace/>", 10, 10));
    assert!(app.package.is_none(), "no part was made");
    assert_eq!(app.document.body, before);
    assert_eq!(
        app.notice.as_ref().map(|(said, _)| said.as_str()),
        Some(crate::revise::ACROSS_BLOCKS)
    );
}

/// The keys counted a paragraph's text as `Paragraph::text` does, which
/// leaves out a tab inside a tracked deletion and puts in an equation's
/// text; the layout and the edits count the other way. With either in a
/// paragraph, a caret the keys placed landed in the wrong place — inside a
/// character, where a tracked Backspace panicked, or past the end.
#[test]
fn keys_count_a_paragraph_as_its_layout_does_with_a_deleted_tab_and_an_equation() {
    use wp_model::doc::{Inline, Piece, Run};
    let drive = Driver::new();
    let mut app = tracking();
    app.document.body[1] = Block::Paragraph(Paragraph {
        content: vec![
            Inline::Run(Run::of("Name")),
            Inline::Revised {
                revision: wp_model::Revision::Deleted(wp_model::Mark::new(1, "Someone")),
                content: vec![Inline::Run(Run {
                    content: vec![Piece::Tab],
                    ..Run::default()
                })],
            },
            Inline::Run(Run::of("don\u{2019}t")),
        ],
        ..Paragraph::default()
    });
    app.changed();
    drive.settle(&mut app);
    put(&mut app, 1, 0);
    for _ in 0..8 {
        drive.press(&mut app, "ArrowRight");
    }
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 1,
            offset: 8
        },
        "after the n"
    );
    drive.press(&mut app, "Backspace");
    assert_eq!(app.document.paragraphs()[1].text(), "Namedo\u{2019}t");
    drive.press(&mut app, "End");
    drive.press(&mut app, "ctrl+Backspace");
    assert_eq!(
        app.document.paragraphs()[1].text(),
        "Name",
        "the word, apostrophe and all"
    );

    // An equation holds no place for the caret.
    let mut app = tracking();
    app.document.body[1] = Block::Paragraph(Paragraph {
        content: vec![
            Inline::Run(Run::of("where x = ")),
            Inline::Math(Box::new(wp_model::doc::MathBlob {
                source: std::sync::Arc::from(&b"<m:oMath/>"[..]),
                text: "y\u{2032}".into(),
            })),
        ],
        ..Paragraph::default()
    });
    app.changed();
    drive.settle(&mut app);
    put(&mut app, 1, 0);
    drive.press(&mut app, "ctrl+End");
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 1,
            offset: 10
        }
    );
    drive.press(&mut app, "ctrl+Backspace");
    // Delete at the end, with only the equation after: nothing to delete.
    drive.press(&mut app, "Delete");
    assert_eq!(
        crate::text::content(app.document.paragraphs()[1]),
        "where x "
    );
    assert_eq!(listed(&app), [(1, "deleted", "=".to_owned())]);
}

/// Update Table of Contents on a list whose field fits in one paragraph put
/// the entries over the paragraphs after it. They go after it, and one undo
/// takes them away.
#[test]
fn a_one_paragraph_contents_list_gains_its_entries_after_it() {
    use wp_model::doc::Inline;
    let mut app = app_with(&["", "Intro", "words"]);
    let heading = app.document.styles.lookup("Heading1").expect("Heading 1");
    app.document.body[0] = Block::Paragraph(Paragraph {
        content: vec![Inline::SimpleField {
            instruction: "TOC \\o \"1-3\"".into(),
            content: vec![Inline::Run(wp_model::doc::Run::of(
                "No table of contents entries found.",
            ))],
        }],
        ..Paragraph::default()
    });
    if let Block::Paragraph(paragraph) = &mut app.document.body[1] {
        paragraph.props.style = Some(heading);
    }
    app.changed();
    let before = app.document.body.clone();
    app.run(Command::UpdateToc);
    let texts: Vec<String> = app.document.paragraphs().iter().map(|p| p.text()).collect();
    assert_eq!(
        texts,
        [
            "No table of contents entries found.",
            "Intro",
            "Intro",
            "words"
        ]
    );
    app.run(Command::Undo);
    assert_eq!(app.document.body, before);
}

/// Update Table of Contents where a bookmark's end stands between the
/// entries, and the new list has another length, which would lose it: said,
/// and nothing changed. It wrote the new entries over the old one for one,
/// and left the rest. With as many headings as entries, the list is written
/// in place, and the bookmark's end stays.
#[test]
fn a_contents_list_with_a_bookmark_among_its_entries_is_left_as_it_was() {
    use wp_model::doc::{Inline, Piece, Run};
    let mut app = app_with(&["", "old one", "old two", "", "Intro", "words"]);
    let heading = app.document.styles.lookup("Heading1").expect("Heading 1");
    let field = |content: Vec<Piece>| {
        Inline::Run(Run {
            content,
            ..Run::default()
        })
    };
    app.document.body[0] = Block::Paragraph(Paragraph {
        content: vec![field(vec![
            Piece::FieldStart {
                dirty: false,
                lock: false,
            },
            Piece::Instruction(" TOC \\o \"1-3\" ".into()),
            Piece::FieldSeparate,
        ])],
        ..Paragraph::default()
    });
    if let Block::Paragraph(paragraph) = &mut app.document.body[3] {
        paragraph.content.push(field(vec![Piece::FieldEnd]));
    }
    if let Block::Paragraph(paragraph) = &mut app.document.body[4] {
        paragraph.props.style = Some(heading);
    }
    app.document
        .body
        .insert(2, Block::Anchor(wp_model::Anchor::BookmarkEnd { id: 9 }));
    app.changed();
    let before = app.document.body.clone();
    app.run(Command::UpdateToc);
    assert_eq!(app.document.body, before);
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|(said, _)| said.contains("not updated")),
        "{:?}",
        app.notice
    );

    if let Block::Paragraph(paragraph) = &mut app.document.body[6] {
        paragraph.props.style = Some(heading);
    }
    app.changed();
    app.run(Command::UpdateToc);
    let texts: Vec<String> = app.document.paragraphs().iter().map(|p| p.text()).collect();
    assert_eq!(texts[1..3], ["Intro", "words"]);
    assert_eq!(
        app.document.body[2],
        Block::Anchor(wp_model::Anchor::BookmarkEnd { id: 9 })
    );
}

/// A second line pasted at a heading's end takes the style after the
/// heading, as Enter gives it untracked.
#[test]
fn lines_pasted_after_a_headings_end_take_the_style_after_it() {
    let mut app = tracking();
    put(&mut app, 0, 11);
    app.paste_text(" more\nnext line");
    let normal = app.document.styles.lookup("Normal");
    assert_eq!(texts(&app)[1], (normal, "next line".to_owned()));
    // In a heading's middle they stay headings, as Enter leaves them there.
    let mut app = tracking();
    put(&mut app, 0, 5);
    app.paste_text(" more\nnext line");
    assert_eq!(
        texts(&app)[1],
        (heading(&app), "next line words".to_owned())
    );
}

/// A link after an equation was found three places along.
#[test]
fn a_link_after_an_equation_is_found_where_the_caret_is() {
    use wp_model::doc::{Inline, Run};
    let paragraph = Paragraph {
        content: vec![
            Inline::Math(Box::new(wp_model::doc::MathBlob {
                source: std::sync::Arc::from(&b"<m:oMath/>"[..]),
                text: "x+y".into(),
            })),
            Inline::Run(Run::of("see ")),
            Inline::Hyperlink(Box::new(wp_model::Hyperlink {
                rel: None,
                anchor: Some("x".into()),
                tooltip: None,
                history: true,
                content: vec![Inline::Run(Run::of("here"))],
            })),
        ],
        ..Paragraph::default()
    };
    assert!(paragraph.link_at(3).is_none(), "\"see \" is 0..4");
    assert!(paragraph.link_at(4).is_some(), "\"here\" is 4..8");
    assert!(paragraph.link_at(7).is_some());
    assert!(paragraph.link_at(8).is_none(), "its end is not in it");
}
