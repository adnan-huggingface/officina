//! Headers, footers, watermarks and sections: the bands a page wears.

use super::*;

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
    assert!(app.notice.is_some(), "and it said why");
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
