//! The find bar and the panes beside the page: navigation, review and comments.

use super::*;

/// The most ordinary find and replace there is — type the word, Enter,
/// Tab to the other field, type its replacement — edited the document
/// instead: the Tab left the bar for an arrow button, the bar said it no
/// longer held the keyboard, and the same Tab was typed over the match.
#[test]
fn tab_in_the_find_bar_goes_to_the_replace_field_and_not_into_the_document() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["the quick fox and the quick dog"]);
    drive.settle(&mut app);
    app.run(Command::Replace);
    drive.settle(&mut app);
    drive.type_text(&mut app, "quick");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert_eq!(
        app.selected_text().as_deref(),
        Some("quick"),
        "Enter selects the first match"
    );
    drive.press(&mut app, "Tab");
    drive.settle(&mut app);
    drive.type_text(&mut app, "slow");
    drive.settle(&mut app);

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
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["the quick fox and the quick dog"]);
    drive.settle(&mut app);
    app.run(Command::Replace);
    drive.settle(&mut app);
    drive.type_text(&mut app, "quick");
    drive.settle(&mut app);
    let field = egui::Id::new("scriva-find-replacement");
    let drawn = |drive: &ui_kit::drive::Driver| drive.ctx().read_response(field).map(|r| r.rect);
    let before = drawn(&drive).expect("the field is drawn");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.selected_text().is_some(), "the count now says 1 of 2");
    let after = drawn(&drive).expect("still drawn");
    assert_eq!(before, after, "and Replace with did not move");
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

/// A card clicked on its words went nowhere: the words were labels, egui's
/// labels are selectable, and a selectable label takes the click from the
/// card it sits on. Only a click in a card's margin went to its place; Word's
/// reviewing pane goes to the change wherever its entry is clicked.
#[test]
fn a_card_clicked_on_its_words_goes_to_its_place() {
    use crate::panes::review::{drawn, CardDrawn, CardKey};
    let drive = ui_kit::drive::Driver::new();
    // The painted text of `words` on `card`, found afresh after `app` drew.
    let words_on = |app: &mut Scriva, words: &str, card: &dyn Fn(&CardDrawn) -> bool| {
        let painted = drive.paint(app, Vec::new());
        let card = drawn(drive.ctx())
            .into_iter()
            .find(|drawn| card(drawn))
            .expect("the card");
        painted
            .texts()
            .into_iter()
            .find(|text| text.text == words && card.rect.contains(text.shown().center()))
            .unwrap_or_else(|| panic!("{words:?} on the card"))
            .shown()
            .center()
    };

    let mut app = with_two_changes();
    app.run(Command::Reviewer);
    drive.settle(&mut app);
    let deletion = |card: &CardDrawn| matches!(&card.key, CardKey::Change(mark) if mark.id == 2);
    for words in ["Adnan Khan", "deleted", "gone"] {
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 0,
        });
        drive.settle(&mut app);
        let at = words_on(&mut app, words, &deletion);
        drive.click(&mut app, at);
        drive.settle(&mut app);
        assert_eq!(
            app.caret().paragraph,
            1,
            "a click on {words:?} went to the deletion"
        );
    }

    let mut app = app_with(&["a word to comment on", "and more"]);
    drive.settle(&mut app);
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
    drive.press(&mut app, "ctrl+alt+M");
    drive.settle(&mut app);
    drive.type_text(&mut app, "kept");
    drive.press(&mut app, "ctrl+Enter");
    drive.settle(&mut app);
    app.selection = Selection::at(Caret {
        paragraph: 1,
        offset: 3,
    });
    drive.settle(&mut app);
    let comment = |card: &CardDrawn| matches!(card.key, CardKey::Comment(_));
    let at = words_on(&mut app, "kept", &comment);
    drive.click(&mut app, at);
    drive.settle(&mut app);
    assert_eq!(
        app.selected_text().as_deref(),
        Some("a word"),
        "a click on the comment's words selected what it is about"
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

/// Ctrl+H pressed in the find bar opens Replace and puts the keyboard in
/// its field, and Ctrl+F there goes back to the find field. The keyboard
/// was the bar's, so the document's Ctrl+H never ran and the key did
/// nothing — the one place Replace is most wanted was the one place its
/// key was dead.
#[test]
fn ctrl_h_in_the_find_bar_opens_replace() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["some text"]);
    drive.settle(&mut app);
    drive.press(&mut app, "ctrl+F");
    drive.settle(&mut app);
    let finder = app.finder.as_ref().expect("the bar is open");
    assert!(!finder.with_replace, "without Replace");
    drive.press(&mut app, "ctrl+H");
    drive.settle(&mut app);
    let finder = app.finder.as_ref().expect("still open");
    assert!(finder.with_replace, "Ctrl+H in the bar opened Replace");
    let focused = drive.ctx().memory(|m| m.focused());
    assert_eq!(
        focused,
        Some(egui::Id::new("scriva-find-replacement")),
        "with the keyboard in its field"
    );
    drive.press(&mut app, "ctrl+F");
    drive.settle(&mut app);
    let focused = drive.ctx().memory(|m| m.focused());
    assert_eq!(
        focused,
        Some(egui::Id::new("scriva-find-query")),
        "and Ctrl+F there goes back to the find field"
    );
    assert!(
        app.finder.as_ref().is_some_and(|f| f.with_replace),
        "leaving Replace open"
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

/// The find bar is painted where it can be seen: its `Aa` switch reaches
/// the screen inside a clip rectangle with height. It was not — a panel
/// nested in the toolbar's panel was given no height and clipped the whole
/// bar to nothing, while every key still reached its field, so no test
/// that typed into it could tell.
#[test]
fn the_find_bar_is_drawn_where_it_can_be_seen() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["the cat"]);
    drive.settle(&mut app);
    app.run(Command::Replace);
    drive.settle(&mut app);
    drive.settle(&mut app);
    let painted = drive.paint(&mut app, Vec::new());
    for label in ["Aa", "ab", "Replace All"] {
        let text = painted
            .text(label)
            .unwrap_or_else(|| panic!("{label} is painted; painted: {:?}", painted.strings()));
        let shown = text.shown();
        assert!(
            shown.height() > 8.0 && shown.width() > 8.0,
            "{label} at {:?} is visible inside its clip {:?}",
            text.rect,
            text.clip
        );
    }
}

/// Saving works from wherever the keyboard is. A pane or the find bar holds
/// the keys a person types — that is what stops a search for "bug" from also
/// typing "bug" into the document — but Ctrl+S is the application's, not the
/// document's, and Word saves from anywhere.
#[test]
fn the_application_keys_work_while_a_pane_has_the_keyboard() {
    let drive = Driver::new();
    let dir = scratch("keys-from-a-pane");
    let path = dir.join("held.docx");
    let mut app = app_with(&["Title", "text"]);
    app.path = Some(path.clone());
    drive.settle(&mut app);

    // The find bar, then the reviewing pane: the keyboard is theirs, and
    // Ctrl+S still saves.
    for (what, open, kept) in [
        (
            "the find bar",
            Command::Find,
            (|app: &Scriva| app.keyboard == Keyboard::Find) as fn(&Scriva) -> bool,
        ),
        (
            "the reviewing pane",
            Command::AddComment,
            (|app: &Scriva| app.draft.is_some()) as fn(&Scriva) -> bool,
        ),
    ] {
        app.notice = None;
        app.document
            .body
            .push(Block::Paragraph(Paragraph::of("more")));
        app.run(open);
        drive.settle(&mut app);
        assert_ne!(app.keyboard, Keyboard::Document, "{what} has the keyboard");
        drive.press(&mut app, "ctrl+S");
        drive.settle(&mut app);
        assert_eq!(
            app.notice.as_ref().map(|(said, _)| said.as_str()),
            Some("Saved held.docx"),
            "{what}"
        );
        assert!(kept(&app), "{what} keeps what was being written");
        // What belongs to the document is still the holder's: a letter typed
        // while it has the keyboard is not typed into the text.
        let before = app.document.paragraphs()[1].text();
        drive.type_text(&mut app, "z");
        drive.settle(&mut app);
        assert_eq!(app.document.paragraphs()[1].text(), before, "{what}");
    }

    // The reviewing pane holding the keyboard with no field in it — a card
    // list, walked by the arrows — is the case no text field covers: a letter
    // typed there is the pane's, and the page keeps its hands off it.
    drive.type_text(&mut app, "a comment");
    drive.press(&mut app, "ctrl+Return");
    drive.settle(&mut app);
    assert_eq!(app.document.comments.len(), 1, "posted");
    app.keyboard = Keyboard::Review;
    app.pane_held = true;
    let before = app.document.paragraphs()[1].text();
    drive.type_text(&mut app, "z");
    drive.settle(&mut app);
    assert_eq!(app.document.paragraphs()[1].text(), before, "the card list");
    let _ = std::fs::remove_dir_all(dir);
}
