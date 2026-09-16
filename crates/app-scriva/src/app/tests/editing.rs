//! Typing, keys in the text, lists, the clipboard, formatting and undo.

use super::*;

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

/// A formatting change with nothing selected and the caret at the end of
/// a word — or its start — is for what is typed next, not for the word:
/// "Hello" typed in red, Automatic chosen at its end, stays red, and the
/// letters typed after it are black. It recoloured the word, taking a
/// caret at a word's edge to be in the word — Word's rule only for a caret
/// between a word's letters. The choice holds while the caret stays; a
/// caret that moves before typing lets it go.
#[test]
fn a_format_chosen_at_a_words_edge_is_for_the_typing_that_follows() {
    use wp_model::Color;
    let red = Color::Rgb([0xFF, 0x00, 0x00]);
    let mut app = app_with(&["Hello"]);
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 0,
        },
        head: Caret {
            paragraph: 0,
            offset: 5,
        },
    };
    app.run(Command::Color(red));
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 5,
    });
    app.run(Command::Color(Color::Auto));
    {
        let paragraphs = app.document.paragraphs();
        let runs = paragraphs[0].runs();
        assert!(
            runs.iter().all(|run| run.props.color == Some(red)),
            "the word keeps its red: {:?}",
            runs.iter().map(|r| r.props.color).collect::<Vec<_>>()
        );
    }
    app.type_text(" world");
    {
        let paragraphs = app.document.paragraphs();
        let runs = paragraphs[0].runs();
        assert_eq!(paragraphs[0].text(), "Hello world");
        assert_eq!(runs.len(), 2, "{:?}", runs);
        assert_eq!(runs[0].props.color, Some(red), "the word is still red");
        assert_eq!(
            runs[1].props.color,
            Some(Color::Auto),
            "and the typing is automatic"
        );
    }
    // Bold chosen at the end, shown on the toolbar before a letter is
    // typed, and the letter is bold.
    app.run(Command::Bold);
    assert!(app.emphasis().0, "the button is lit for the typing to come");
    assert!(
        !app.document.paragraphs()[0].runs()[1].props.bold(),
        "and \"world\" is not bold"
    );
    app.type_text("s");
    {
        let paragraphs = app.document.paragraphs();
        let runs = paragraphs[0].runs();
        assert_eq!(paragraphs[0].text(), "Hello worlds");
        assert!(
            runs.last().unwrap().props.bold(),
            "the s is bold: {:?}",
            runs
        );
        assert!(!runs[1].props.bold(), "\"world\" is not");
    }
    // Chosen, then the caret moved: let go.
    app.run(Command::Italic);
    app.key(egui::Key::ArrowLeft, egui::Modifiers::NONE);
    app.type_text("!");
    let paragraphs = app.document.paragraphs();
    assert_eq!(paragraphs[0].text(), "Hello world!s");
    assert!(
        !paragraphs[0].runs().iter().any(|run| run.props.italic()),
        "nothing is italic: {:?}",
        paragraphs[0].runs()
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
    assert!(app.notice.is_some());
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
    assert!(app.notice.is_some());
    assert!(!app.dirty, "and nothing was changed");
}

/// Shift+Up at the end of the first line selects it back to its start, and
/// Shift+Down at the start of the last line selects it on to its end: Up on
/// the first line of the document goes to its start and Down on the last
/// to its end, as Word's do. They went nowhere — there was no line above
/// or below to step to, so the step was refused and nothing was selected.
#[test]
fn up_on_the_first_line_goes_to_its_start_and_down_on_the_last_to_its_end() {
    let drive = ui_kit::drive::Driver::new();
    let first = "Quarterly report";
    let last = "The first quarter went well.";
    let mut app = app_with(&[first, last]);
    drive.settle(&mut app);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: first.len(),
    });
    drive.press(&mut app, "shift+Up");
    drive.settle(&mut app);
    assert_eq!(
        app.selected_text().as_deref(),
        Some(first),
        "Shift+Up on the first line: {:?}",
        app.selection
    );
    drive.press(&mut app, "Up");
    drive.settle(&mut app);
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 0,
            offset: 0
        },
        "Up alone goes to the start and drops the selection"
    );

    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 0,
    });
    drive.press(&mut app, "shift+Down");
    drive.settle(&mut app);
    assert_eq!(
        app.selected_text().as_deref(),
        Some(last),
        "Shift+Down on the last line: {:?}",
        app.selection
    );
    drive.press(&mut app, "Down");
    drive.settle(&mut app);
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 1,
            offset: last.len()
        },
        "Down alone goes to the end"
    );
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

/// Layout ▸ Page Setup… with A4 and Landscape chosen and applied: the
/// section's paper is A4 on its side — width and height stored swapped,
/// as the model stores a landscape page — and one undo puts Letter upright
/// back, because the box is one decision however many fields it has.
#[test]
fn page_setup_a4_landscape_is_one_undo_step() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["a paragraph"]);
    drive.settle(&mut app);
    let was = app.document.section.page;
    assert_eq!(
        (was.width, was.height),
        (Twips(12240), Twips(15840)),
        "Letter to start"
    );
    drive.menu(&mut app, 'L', 'P');
    drive.settle(&mut app);
    let mut draft = app.page_setup.clone().expect("Alt+L, P opened Page Setup");
    assert_eq!(draft.paper, 0, "and it shows Letter");
    assert!(!draft.landscape);
    // The A4 row of the combo, and the Landscape toggle, as the box would
    // set them; then Enter applies.
    draft.paper = 2;
    draft.width = "8.27".to_owned();
    draft.height = "11.69".to_owned();
    draft.landscape = true;
    app.page_setup = Some(draft);
    drive.settle(&mut app);
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.page_setup.is_none(), "Enter applied and closed it");
    let page = &app.document.section.page;
    assert_eq!(page.orientation, wp_model::Orientation::Landscape);
    assert_eq!(
        (page.width, page.height),
        (Twips(16838), Twips(11906)),
        "A4 on its side"
    );
    app.run(Command::Undo);
    let page = &app.document.section.page;
    assert_eq!(
        page.orientation,
        wp_model::Orientation::Portrait,
        "one undo"
    );
    assert_eq!(
        (page.width, page.height),
        (Twips(12240), Twips(15840)),
        "Letter upright again"
    );
    app.run(Command::Redo);
    assert_eq!(
        app.document.section.page.orientation,
        wp_model::Orientation::Landscape
    );
}
