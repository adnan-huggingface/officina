//! Pictures and charts: in by paste or menu, sized, moved and taken out.

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
    let (text, _) = app.notice.as_ref().expect("it says why, in the status bar");
    assert!(text.starts_with("Nothing selected"), "{text}");
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
