//! The desk as it is painted: pages, zoom, scrolling, the caret and the pointer.

use super::*;
use ui_kit::drive::{Driver, Painted};

/// Whatever the chrome has to say — the caret is in a table, a header is
/// being edited, the find bar is open, with or without Replace, there is
/// news about the document — it says in the one row under the toolbar,
/// which is always there, and the desk does not move. It did: the strip
/// stood in the toolbar's panel only while there was one, so a click in a
/// table pushed the page down a row and a click out of it pulled the page
/// back up, and Ctrl+F did the same.
#[test]
fn the_desk_stays_put_whatever_the_row_under_the_toolbar_says() {
    let drive = Driver::new();
    let mut app = app_with(&["before"]);
    drive.settle(&mut app);
    let looked = |app: &mut Scriva| -> (f32, Vec<String>) {
        let painted = drive.paint(app, Vec::new());
        let desk = painted
            .largest(ui_kit::theme::DESK)
            .expect("the desk is painted");
        (desk.top(), painted.strings())
    };
    let (top, texts) = looked(&mut app);
    assert!(!texts.iter().any(|t| t == "Row above"), "no strip yet");

    // In a table, and out of it again.
    app.insert_table(2, 2);
    drive.settle(&mut app);
    let (now, texts) = looked(&mut app);
    assert!(
        texts.iter().any(|t| t == "Row above"),
        "the table strip is on: {texts:?}"
    );
    assert_eq!(
        now, top,
        "the desk did not move when the caret entered a table"
    );
    // The table went in front of the paragraph; the paragraph is now last.
    app.selection = Selection::at(Caret {
        paragraph: app.document.paragraphs().len() - 1,
        offset: 0,
    });
    drive.settle(&mut app);
    let (now, texts) = looked(&mut app);
    assert!(
        !texts.iter().any(|t| t == "Row above"),
        "the strip is off again"
    );
    assert_eq!(now, top, "and the desk did not move when it left");

    // Editing the header.
    app.run(Command::EditHeader);
    drive.settle(&mut app);
    let (now, texts) = looked(&mut app);
    assert!(
        texts.iter().any(|t| t == "Different first page"),
        "the header's row: {texts:?}"
    );
    assert_eq!(now, top, "the desk did not move for the header");
    app.run(Command::CloseChrome);
    drive.settle(&mut app);
    assert_eq!(looked(&mut app).0, top, "nor when the header closed");

    // The find bar, then Replace with it.
    app.run(Command::Find);
    drive.settle(&mut app);
    let (now, texts) = looked(&mut app);
    assert!(texts.iter().any(|t| t == "Aa"), "the find bar: {texts:?}");
    assert_eq!(now, top, "the desk did not move for the find bar");
    app.run(Command::Replace);
    drive.settle(&mut app);
    let (now, texts) = looked(&mut app);
    assert!(
        texts.iter().any(|t| t == "Replace All"),
        "with Replace: {texts:?}"
    );
    assert_eq!(now, top, "nor for Replace");
    app.finder = None;
    drive.settle(&mut app);
    assert_eq!(looked(&mut app).0, top, "nor when it closed");

    // News about the document.
    app.post_notice(
        "Something about this document",
        Some(("Export…", Command::ExportPdf)),
    );
    drive.settle(&mut app);
    let (now, texts) = looked(&mut app);
    assert!(
        texts.iter().any(|t| t == "Something about this document"),
        "the notice: {texts:?}"
    );
    assert_eq!(now, top, "the desk did not move for the notice");
    app.notices.clear();
    drive.settle(&mut app);
    assert_eq!(looked(&mut app).0, top, "nor when it was dismissed");
}

/// A press at the end of the first line and a drag up into the top margin
/// selects the line back to its start, and a press at the start of the last
/// line dragged down into the bottom margin selects it to its end — where
/// Word puts a pointer above the first line or below the last. They
/// selected nothing: the point in the margin was given to the nearest line
/// at the same x, which was where the press already was.
#[test]
fn dragging_out_of_the_text_above_or_below_takes_the_line_to_its_edge() {
    let drive = Driver::new();
    let first = "Quarterly report";
    let last = "The first quarter went well.";
    let mut app = app_with(&[first, last]);
    drive.settle(&mut app);
    let paper = drive
        .paint(&mut app, Vec::new())
        .largest(egui::Color32::WHITE)
        .expect("a page is painted");

    // Up from the end of the first line, into the top margin.
    let end = app
        .on_screen(Caret {
            paragraph: 0,
            offset: first.len(),
        })
        .expect("drawn");
    drive.drag(&mut app, end, egui::pos2(end.x, paper.min.y + 12.0));
    assert_eq!(
        app.selected_text().as_deref(),
        Some(first),
        "the first line, back to its start; caret {:?}",
        app.selection
    );

    // Down from the start of the last line, into the bottom margin.
    let start = app
        .on_screen(Caret {
            paragraph: 1,
            offset: 0,
        })
        .expect("drawn");
    // The page runs off the bottom of the window; the desk's foot is
    // still far below the last line.
    drive.drag(&mut app, start, egui::pos2(start.x, 940.0));
    assert_eq!(
        app.selected_text().as_deref(),
        Some(last),
        "the last line, on to its end; caret {:?}",
        app.selection
    );
}

/// The desk's scroll bar is painted in the chrome's palette — a soft grey
/// handle on a chrome track at the desk's right edge — and nothing dark
/// stands there. egui's own bar floated over the desk in the widget
/// foreground colour, which under this theme is the ink: a black stripe.
#[test]
fn the_desks_scroll_bar_is_grey_on_chrome_and_not_black() {
    let drive = Driver::new();
    let mut app = six_pages(&drive);
    // With the pointer on the desk, so that a bar that only shows itself
    // while hovered would be showing.
    let painted = drive.paint(
        &mut app,
        vec![egui::Event::PointerMoved(egui::pos2(800.0, 500.0))],
    );
    let at_edge: Vec<_> = painted
        .rects()
        .into_iter()
        .filter(|r| r.rect.right() > 1580.0 && r.rect.width() < 20.0 && r.rect.height() > 20.0)
        .collect();
    assert!(
        at_edge.iter().any(|r| r.fill == ui_kit::theme::INK_FAINT),
        "a grey handle at the edge: {at_edge:?}"
    );
    assert!(
        at_edge
            .iter()
            .any(|r| r.fill == ui_kit::theme::CHROME && r.rect.height() > 400.0),
        "on a chrome track: {at_edge:?}"
    );
    let dark = |c: &egui::Color32| c.r() < 0x70 && c.g() < 0x70 && c.b() < 0x70 && c.a() > 0;
    assert!(
        !at_edge.iter().any(|r| dark(&r.fill)),
        "and nothing black there: {at_edge:?}"
    );
    // And no bar along the foot: the page fits the desk's width, and the
    // vertical bar's own width must not put the desk over by that much.
    let along_foot: Vec<egui::Rect> = painted
        .filled(ui_kit::theme::CHROME)
        .into_iter()
        .filter(|rect| rect.height() <= 12.0 && rect.width() > 500.0)
        .collect();
    assert!(along_foot.is_empty(), "no horizontal bar: {along_foot:?}");
}

/// Six pages, each with a word on it, laid out and settled.
fn six_pages(drive: &Driver) -> Scriva {
    let mut app = app_with(&["one", "two", "three", "four", "five", "six"]);
    drive.settle(&mut app);
    for paragraph in (1..6).rev() {
        app.selection = Selection::at(Caret {
            paragraph,
            offset: 0,
        });
        app.run(Command::PageBreak);
    }
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert!(app.view.pages().len() >= 6, "six pages");
    app
}

/// A document opens at the zoom that shows a whole page, as Word opens one;
/// so does a new document, and the one the window starts with — and the
/// fit follows the desk until a zoom is chosen, because the window opens
/// at its modest size and is maximized a few frames later, and a fit
/// taken on the first frames was a page a third of the screen.
#[test]
fn a_document_opens_at_whole_page_zoom() {
    let mut drive = Driver::sized(egui::vec2(1000.0, 700.0));
    let mut app = Scriva::new();
    drive.settle(&mut app);
    drive.settle(&mut app);
    let small = app.fit_percent(false).expect("a page to fit") as f64 / 100.0;
    assert!(
        (app.view.zoom - small).abs() < 0.001,
        "the window starts at the whole-page zoom: {} against {small}",
        app.view.zoom
    );
    drive.resize(egui::vec2(1600.0, 1000.0));
    drive.settle(&mut app);
    drive.settle(&mut app);
    let fit = app.fit_percent(false).expect("a page to fit") as f64 / 100.0;
    assert!(
        fit > small + 0.1,
        "a taller desk fits a bigger page: {fit} over {small}"
    );
    assert!(
        fit < 0.95,
        "whole page in a 1000-tall window is under 100%: {fit}"
    );
    assert!(
        (app.view.zoom - fit).abs() < 0.001,
        "and the zoom followed the desk when the window grew: {} against {fit}",
        app.view.zoom
    );
    // Whole means whole: the page's foot is on the desk, with room under
    // it, and its head too. It was not: the fit took the paper alone, and
    // at a zoom past 100% the gap above the page grew and pushed its foot
    // under the status bar.
    let painted = drive.paint(&mut app, Vec::new());
    let desk = painted.largest(ui_kit::theme::DESK).expect("the desk");
    let paper = painted.largest(egui::Color32::WHITE).expect("the page");
    assert!(
        paper.top() > desk.top() + 8.0 && paper.bottom() < desk.bottom() - 8.0,
        "the whole page is on the desk: page {paper:?} on desk {desk:?}"
    );
    // A zoom chosen holds, whatever the window does after.
    app.run(Command::Zoom(1.0));
    drive.resize(egui::vec2(1200.0, 800.0));
    drive.settle(&mut app);
    drive.settle(&mut app);
    assert_eq!(app.view.zoom, 1.0, "a chosen zoom is kept through a resize");
    drive.resize(egui::vec2(1600.0, 1000.0));
    drive.settle(&mut app);
    app.run(Command::Zoom(1.5));
    drive.settle(&mut app);
    app.open_path(&corpus_docx("comments.docx"));
    drive.settle(&mut app);
    drive.settle(&mut app);
    let fit = app.fit_percent(false).expect("a page to fit") as f64 / 100.0;
    assert!(
        (app.view.zoom - fit).abs() < 0.001,
        "an opened document is shown whole: {} against {fit}",
        app.view.zoom
    );
    app.run(Command::Zoom(1.5));
    drive.settle(&mut app);
    app.dirty = false;
    app.run(Command::New);
    drive.settle(&mut app);
    drive.settle(&mut app);
    let fit = app.fit_percent(false).expect("a page to fit") as f64 / 100.0;
    assert!(
        (app.view.zoom - fit).abs() < 0.001,
        "and so is a new one: {} against {fit}",
        app.view.zoom
    );
}

/// At a zoom that shows a whole page, Page Down shows the next page whole
/// and Page Up the one before, with the caret on the page shown; at a
/// zoom where a page is taller than the desk, each moves the desk by one
/// screen. The caret keeps its place on the screen either way — Word's
/// Page Down moves the view, not the caret to the view's edge. It moved
/// the caret a screen and scrolled only as far as showed it, so the view
/// went half a screen and the caret sat at its edge; and with less than a
/// screen left there was no line a screen away, so the key did nothing.
#[test]
fn page_down_at_whole_page_zoom_shows_the_next_page_whole() {
    let drive = Driver::new();
    let mut app = six_pages(&drive);
    let fit = app.fit_percent(false).expect("a fit") as f64 / 100.0;
    app.run(Command::Zoom(fit));
    drive.settle(&mut app);
    drive.settle(&mut app);
    let zoom = (app.view.zoom * view::SCALE) as f32;
    let pitch = (app.view.pages()[0].geometry.height + view::GAP as f64) as f32 * zoom;
    assert!(pitch <= app.viewport.y, "a page and its gap fit the desk");
    let caret_page = |app: &Scriva| {
        view::caret_rect(&app.view, wp_model::Scope::Body, app.caret())
            .map(|(page, _)| page)
            .expect("a caret")
    };
    let start = app.scroll;
    drive.press(&mut app, "PageDown");
    drive.settle(&mut app);
    assert!(
        (app.scroll - start - pitch).abs() < 1.0,
        "the desk moved one page: {} from {start} against {pitch}",
        app.scroll
    );
    assert_eq!(caret_page(&app), 1, "and the caret is on the page shown");
    drive.press(&mut app, "PageDown");
    drive.settle(&mut app);
    assert!(
        (app.scroll - start - 2.0 * pitch).abs() < 1.0,
        "two pages: {}",
        app.scroll
    );
    assert_eq!(caret_page(&app), 2);
    drive.press(&mut app, "PageUp");
    drive.settle(&mut app);
    assert!(
        (app.scroll - start - pitch).abs() < 1.0,
        "back one: {}",
        app.scroll
    );
    assert_eq!(caret_page(&app), 1);

    // At 100% a page is taller than the desk: one screen at a time, and the
    // caret keeps its place on the screen.
    app.run(Command::Zoom(1.0));
    drive.settle(&mut app);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 0,
    });
    app.reveal = Some(app.caret());
    drive.settle(&mut app);
    drive.settle(&mut app);
    let before = app.scroll;
    let screen = app.viewport.y;
    let zoom = (app.view.zoom * view::SCALE) as f32;
    let on_screen = |app: &Scriva| {
        let (page, rect) = view::caret_rect(&app.view, wp_model::Scope::Body, app.caret()).unwrap();
        (app.view.page_origin(page).1 as f32 + rect.min.y) * zoom - app.scroll
    };
    let was = on_screen(&app);
    drive.press(&mut app, "PageDown");
    drive.settle(&mut app);
    assert!(
        (app.scroll - before - screen).abs() < 1.0,
        "one screen down: {} from {before}, screen {screen}",
        app.scroll
    );
    // On the screen still — on the nearest line to where it was, which in
    // a document of one word a page is that page's only line.
    let now = on_screen(&app);
    assert!(
        (0.0..screen).contains(&now),
        "the caret is on the screen shown: {now} (was {was}, screen {screen})"
    );

    // With less than a screen left, Page Down goes to the end and the last
    // screen, rather than nowhere.
    for _ in 0..12 {
        drive.press(&mut app, "PageDown");
        drive.settle(&mut app);
    }
    let last = app.document.paragraphs().len() - 1;
    assert_eq!(
        app.caret().paragraph,
        last,
        "the caret reached the last paragraph"
    );
    assert_eq!(app.caret().offset, "six".len(), "at its end");
}

/// The frame a pointer move arrives in, mid-sweep, paints the selection
/// up to that move — not up to the move before it. The desk painted
/// first and read the pointer after, so the highlight trailed the mouse
/// by a frame however fast the frames came, and on a display that shows
/// each frame a little late that read as a laggy selection.
#[test]
fn a_sweep_paints_the_selection_up_to_the_pointer_in_the_same_frame() {
    let drive = Driver::new();
    let mut app = app_with(&["one two three", "four five six", "seven eight nine"]);
    drive.settle(&mut app);
    app.run(Command::Zoom(1.0));
    drive.settle(&mut app);
    drive.settle(&mut app);
    let at = |app: &Scriva, paragraph: usize, offset: usize| {
        app.on_screen(Caret { paragraph, offset }).expect("drawn")
    };
    let (first, second, third) = (at(&app, 0, 0), at(&app, 1, 4), at(&app, 2, 10));
    drive.press_at(&mut app, first);
    drive.move_to(&mut app, second);
    drive.move_to(&mut app, second);
    // The move to the third line, and what that very frame painted.
    let painted = drive.paint(&mut app, vec![egui::Event::PointerMoved(third)]);
    let highlight = painted.filled(ui_kit::theme::SELECTION);
    let reach = highlight.iter().map(|r| r.bottom()).fold(0.0f32, f32::max);
    assert!(
        reach >= third.y,
        "the highlight reaches the third line in the frame the pointer got there: {reach} against {}; {highlight:?}",
        third.y
    );
}

/// Scrolled to the end, the last page stands a gap above the desk's foot,
/// with its bottom edge and its shadow in view. The desk was one gap
/// short: a gap above the first page and one after each page counted a
/// gap fewer than the stack needs, so the last page's foot lay on the
/// desk's very end and its border was cut off.
#[test]
fn the_last_page_has_a_gap_under_it_at_the_end_of_the_desk() {
    let drive = Driver::new();
    let mut app = six_pages(&drive);
    app.run(Command::Zoom(1.0));
    drive.settle(&mut app);
    // To the end, and past it: the desk stops at its foot.
    drive.press(&mut app, "ctrl+End");
    drive.settle(&mut app);
    for _ in 0..4 {
        drive.press(&mut app, "PageDown");
        drive.settle(&mut app);
    }
    let painted = drive.paint(&mut app, Vec::new());
    let desk = painted.largest(ui_kit::theme::DESK).expect("the desk");
    let last = painted
        .filled(egui::Color32::WHITE)
        .into_iter()
        .filter(|rect| rect.width() > 500.0)
        .map(|rect| rect.bottom())
        .fold(f32::MIN, f32::max);
    let gap = view::GAP * (app.view.zoom * view::SCALE) as f32;
    assert!(
        last <= desk.bottom() - gap + 1.0,
        "the last page's foot is a gap above the desk's: {last} against {} (gap {gap})",
        desk.bottom()
    );
}

/// A sweep pulled past the desk's foot scrolls the desk on, a step a
/// frame, and the selection grows to what comes into view — Word's
/// autoscroll, and the only way a mouse selects more than a screen. It
/// stopped at the edge: the desk stood still and the selection with it.
#[test]
fn a_sweep_past_the_desks_edge_scrolls_the_desk_and_grows_the_selection() {
    let drive = Driver::new();
    let mut app = six_pages(&drive);
    app.run(Command::Zoom(1.0));
    drive.settle(&mut app);
    let desk = drive
        .paint(&mut app, Vec::new())
        .largest(ui_kit::theme::DESK)
        .expect("the desk");
    let start = app.on_screen(Caret::default()).expect("drawn") + egui::vec2(4.0, 0.0);
    drive.press_at(&mut app, start);
    // Down past the desk's foot, into the status bar, and held there.
    let below = egui::pos2(start.x, desk.bottom() + 20.0);
    drive.move_to(&mut app, below);
    let before = app.scroll;
    drive.hold(&mut app, below, 30);
    assert!(
        app.scroll > before + 100.0,
        "the desk scrolled on while the pointer rested past its foot: {} from {before}",
        app.scroll
    );
    let (anchor, head) = (app.selection.anchor, app.selection.head);
    assert_eq!(
        anchor,
        Caret {
            paragraph: 0,
            offset: 0
        },
        "the anchor stayed"
    );
    assert!(
        head.paragraph > 0,
        "and the selection grew into what came into view: {head:?}"
    );
    // And back up past the top: the desk comes back.
    let above = egui::pos2(start.x, desk.top() - 20.0);
    let scrolled = app.scroll;
    drive.hold(&mut app, above, 30);
    assert!(
        app.scroll < scrolled - 100.0,
        "and back up again: {} from {scrolled}",
        app.scroll
    );
}

#[test]
fn a_page_sits_on_the_light_desk_with_a_shadow_and_no_fade() {
    let drive = Driver::new();
    let mut app = Scriva::new();
    drive.settle(&mut app);
    let painted = drive.paint(&mut app, Vec::new());

    assert_eq!(view::desk(), ui_kit::theme::DESK);
    let rects = painted.rects();
    let desk = rects
        .iter()
        .position(|r| r.fill == ui_kit::theme::DESK && r.rect.width() > 1000.0)
        .expect("the desk is painted");
    let paper = rects
        .iter()
        .position(|r| r.fill == egui::Color32::WHITE && r.rect.width() > 500.0)
        .expect("a page is painted");
    let shadow = rects
        .iter()
        .position(|r| r.blur > 0.0 && r.rect.width() > 500.0)
        .expect("a shadow is painted");
    assert!(
        desk < shadow && shadow < paper,
        "desk, then shadow, then paper"
    );
    // At whatever zoom the window opened at — the whole-page fit.
    let glass = (app.view.zoom * view::SCALE) as f32;
    assert!(
        (rects[paper].rect.top() - rects[desk].rect.top() - view::GAP * glass).abs() < 1.0,
        "the first page stands one gap below the toolbar"
    );

    // The blur along the bottom of the desk was egui's scroll-area fade: a
    // four-cornered mesh from clear to half-grey over the last twenty points.
    // Nothing of that shape is painted on the desk now.
    let desk_rect = rects[desk].rect;
    let gradients = painted.shapes().iter().filter(|shape| match shape {
        egui::Shape::Mesh(mesh) => {
            let bounds = shape.visual_bounding_rect();
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

/// The caret's stroke, if the frame painted one.
fn caret(painted: &Painted) -> Option<egui::Rect> {
    painted
        .filled(view::CARET)
        .into_iter()
        .find(|rect| rect.width() == view::CARET_WIDTH)
}

/// The caret is the height of the type on its line, as Word's is, and not
/// the line's whole pitch: a 12-point line spaced at 1.15 wears a 12-point
/// caret. It wore the pitch, half again too tall for its word.
#[test]
fn the_caret_is_the_height_of_the_type_and_not_the_line() {
    let drive = Driver::new();
    let mut app = app_with(&["fds"]);
    drive.settle(&mut app);
    app.run(Command::Zoom(1.0));
    drive.frame_at(&mut app, vec![egui::Event::Text("x".into())], Some(10.0));
    let caret = caret(&drive.paint_at(&mut app, Some(10.0))).expect("the caret is painted");
    let (line, pitch) = app.view.pages()[0]
        .content
        .iter()
        .find_map(|p| match &p.kind {
            wp_layout::block::Placed::Line { line, .. } => Some((line.clone(), p.height)),
            _ => None,
        })
        .expect("the line");
    let glass = (app.view.zoom * view::SCALE) as f32;
    let type_height = (line.ascent + line.descent) as f32 * glass;
    assert!(
        (caret.height() - type_height).abs() < 1.0,
        "the caret is the type's height: {} against {type_height}",
        caret.height()
    );
    assert!(
        caret.height() < pitch as f32 * glass - 1.0,
        "and shorter than the line's pitch {}",
        pitch as f32 * glass
    );
}

/// The frame a letter is typed in paints the letter and the caret after
/// it. It painted the page laid out before the letter, with a caret whose
/// offset no line of that page reached — so the caret fell back to the
/// line's left edge for one frame, a flash at the start of the line on
/// every keystroke.
#[test]
fn a_typed_letter_is_painted_with_its_caret_after_it_in_the_same_frame() {
    let drive = Driver::new();
    let mut app = app_with(&["fds"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 3,
    });
    drive.settle(&mut app);
    app.run(Command::Zoom(1.0));
    drive.frame_at(&mut app, Vec::new(), Some(10.0));
    let before = caret(&drive.paint_at(&mut app, Some(10.0)))
        .expect("the caret is painted")
        .min
        .x;
    let painted =
        Painted::new(&drive.frame_at(&mut app, vec![egui::Event::Text("x".into())], Some(10.0)));
    let after = caret(&painted).expect("the caret is painted").min.x;
    assert!(
        after > before + 4.0,
        "the caret stands after the new letter in the frame it was typed: {after} was {before}"
    );
    assert!(
        painted.text("fdsx").is_some(),
        "and the letter is on the page in that frame"
    );
}

#[test]
fn the_caret_blinks_and_stands_solid_after_a_key() {
    let drive = Driver::new();
    let mut app = Scriva::new();
    drive.frame_at(&mut app, Vec::new(), Some(10.0));
    drive.frame_at(&mut app, vec![egui::Event::Text("a".into())], Some(10.0));

    // Solid in the frame after the key, and for the whole first beat.
    let beat = ui_kit::theme::BLINK;
    let shapes = drive.paint_at(&mut app, Some(10.0));
    assert!(caret(&shapes).is_some(), "shown at the key");
    let shapes = drive.paint_at(&mut app, Some(10.0 + beat * 0.9));
    assert!(
        caret(&shapes).is_some(),
        "still shown before the first beat ends"
    );
    // Gone for the second beat, back for the third.
    let shapes = drive.paint_at(&mut app, Some(10.0 + beat * 1.5));
    assert!(caret(&shapes).is_none(), "hidden in the second beat");
    let shapes = drive.paint_at(&mut app, Some(10.0 + beat * 2.5));
    assert!(caret(&shapes).is_some(), "shown again in the third");

    // A key in the hidden beat brings it straight back.
    drive.frame_at(
        &mut app,
        vec![egui::Event::Text("b".into())],
        Some(10.0 + beat * 3.5),
    );
    let shapes = drive.paint_at(&mut app, Some(10.0 + beat * 3.6));
    assert!(caret(&shapes).is_some(), "solid again from the key");

    // With a selection showing there is nothing to blink: the selection
    // says where the caret is.
    drive.press(&mut app, "shift+Home");
    assert!(!app.selection.is_empty());
    for beats in [0.5, 1.5, 2.5] {
        let shapes = drive.paint_at(&mut app, Some(20.0 + beat * beats));
        assert!(
            caret(&shapes).is_some(),
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
fn a_deletion_is_struck_and_an_insertion_underlined_on_the_page() {
    use wp_model::doc::{inserted_by, Inline, Piece, Run};
    use wp_model::{Mark, Revision};
    let drive = Driver::new();
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
    let painted = drive.paint(&mut app, Vec::new());

    let colour = ui_kit::theme::author(0);
    let rules = painted.hlines(colour);
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
    let bars: Vec<egui::Rect> = painted
        .filled(colour)
        .into_iter()
        .filter(|rect| rect.width() <= 3.0)
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
    let painted = drive.paint(&mut app, Vec::new());
    assert!(painted.hlines(colour).is_empty());
    assert!(painted.rects().iter().all(|r| r.fill != colour));
}

#[test]
fn a_comment_washes_its_range_and_marks_the_margin() {
    let drive = Driver::new();
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
    let painted = drive.paint(&mut app, Vec::new());

    let wash = view::wash_colour(0);
    let washed = painted.filled(wash);
    assert_eq!(washed.len(), 1, "one band under the one commented word");
    assert!(
        washed[0].width() > 10.0 && washed[0].width() < 80.0,
        "the width of a word: {washed:?}"
    );

    let colour = ui_kit::theme::author(0);
    let marker = painted
        .shapes()
        .iter()
        .find_map(|shape| match shape {
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
    let painted = drive.paint(&mut app, Vec::new());
    assert!(painted.rects().iter().all(|r| r.fill != wash));
}

/// A wheel over the desk brings the page badge up — `Page 2 of 6`, painted
/// at the right of the desk — and eight hundred milliseconds after the last
/// tick it has faded away; the caret moving the desk brings no badge, since
/// the status bar already says where the caret is.
#[test]
fn the_page_badge_shows_while_the_desk_scrolls_and_fades_after() {
    let drive = Driver::new();
    let mut app = six_pages(&drive);
    drive.frame_at(&mut app, Vec::new(), Some(10.0));
    drive.frame_at(&mut app, Vec::new(), Some(10.1));
    let badge_texts = |painted: Painted| painted.strings_starting("Page ");
    // Nothing before the wheel but the status bar's own count.
    let before = badge_texts(drive.paint_at(&mut app, Some(10.2)));
    assert_eq!(before, ["Page 1 of 6"], "the status bar alone: {before:?}");
    let wheel = |delta: f32| {
        vec![
            egui::Event::PointerMoved(egui::pos2(800.0, 500.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, delta),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    drive.frame_at(&mut app, wheel(-1600.0), Some(10.3));
    drive.frame_at(&mut app, Vec::new(), Some(10.4));
    drive.frame_at(&mut app, Vec::new(), Some(10.5));
    let during = badge_texts(drive.paint_at(&mut app, Some(10.6)));
    assert_eq!(
        during.len(),
        2,
        "the badge beside the status count: {during:?}"
    );
    assert!(
        during.iter().any(|text| text != "Page 1 of 6"),
        "and it names the page the desk shows: {during:?}"
    );
    let after = badge_texts(drive.paint_at(&mut app, Some(11.5)));
    assert_eq!(after.len(), 1, "faded a second later: {after:?}");
    // The caret moving the desk: no badge.
    app.run(Command::GoToPage);
    app.goto = None;
    app.go_to_page(1);
    drive.frame_at(&mut app, Vec::new(), Some(12.0));
    let moved = badge_texts(drive.paint_at(&mut app, Some(12.1)));
    assert_eq!(
        moved,
        ["Page 1 of 6"],
        "no badge for a caret move: {moved:?}"
    );
}

/// The user's case, read off the screen: "Hello" in red, Automatic chosen
/// after its last letter, " world" typed. The page paints Hello red and
/// world in the automatic ink. The model said so before; nothing could ask
/// the glass until the driver could read the colour a letter was painted in.
#[test]
fn the_colour_chosen_for_typing_is_the_colour_painted() {
    use wp_model::Color;
    let drive = Driver::new();
    let mut app = app_with(&["Hello"]);
    drive.settle(&mut app);
    app.selection = Selection {
        anchor: Caret::default(),
        head: Caret {
            paragraph: 0,
            offset: 5,
        },
    };
    app.run(Command::Color(Color::Rgb([0xC0, 0x10, 0x20])));
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 5,
    });
    drive.settle(&mut app);
    app.run(Command::Color(Color::Auto));
    drive.type_text(&mut app, " world");
    drive.settle(&mut app);
    let painted = drive.paint(&mut app, Vec::new());
    let red = egui::Color32::from_rgb(0xC0, 0x10, 0x20);
    assert_eq!(
        painted.colour_of("Hello"),
        Some(red),
        "the word keeps its red on the page: {:?}",
        painted.strings()
    );
    let world = painted.colour_of("world").expect("the typing is painted");
    assert_ne!(world, red, "and the typing is not red");
    assert_eq!(
        world,
        egui::Color32::BLACK,
        "it is in the automatic ink on white paper"
    );
}

/// `on_screen` finds a caret on any page, with the desk scrolled away from
/// the first, and a click there puts the caret back on it: the point is
/// where the window draws the caret, not where the first page's corner and
/// the zoom say it would be if nothing had moved.
#[test]
fn a_caret_is_found_on_screen_on_any_page_however_far_the_desk_has_scrolled() {
    let drive = Driver::new();
    let mut app = six_pages(&drive);
    app.run(Command::Zoom(1.0));
    drive.settle(&mut app);
    let six = Caret {
        paragraph: app.document.paragraphs().len() - 1,
        offset: 2,
    };
    app.selection = Selection::at(six);
    app.reveal = Some(six);
    drive.settle(&mut app);
    drive.settle(&mut app);
    let (page, _) = view::caret_rect(&app.view, wp_model::Scope::Body, six).expect("laid");
    assert_eq!(page, 5, "the caret is on the sixth page");
    assert!(
        app.scroll > 4000.0,
        "and the desk scrolled there: {}",
        app.scroll
    );
    // Asked after a frame painted at the new scroll: the answer is the
    // screen as last painted, and the reveal scrolled at that frame's end.
    let painted = drive.paint(&mut app, Vec::new());
    let at = app.on_screen(six).expect("page six is drawn");
    let desk = painted.largest(ui_kit::theme::DESK).expect("the desk");
    assert!(desk.contains(at), "{at:?} is on the desk {desk:?}");
    let caret = caret(&painted).expect("the caret is painted");
    assert!(
        (caret.min.x - at.x).abs() < 1.0 && caret.y_range().contains(at.y),
        "and on the caret's stroke: {at:?} against {caret:?}"
    );
    app.selection = Selection::at(Caret::default());
    drive.click(&mut app, at);
    assert_eq!(app.caret(), six, "a click there lands there");
}

/// The window opens at the shell's first-run size and is maximized a few
/// frames later; the page is then shown whole at the size the window ended
/// at. This is the case every test missed while the driver's window was
/// full size from its first frame.
#[test]
fn a_window_that_opens_small_and_grows_shows_the_whole_page_at_its_full_size() {
    let drive = Driver::opening();
    let mut app = Scriva::new();
    for _ in 0..ui_kit::drive::OPENING_FRAMES + 3 {
        drive.settle(&mut app);
    }
    assert_eq!(drive.window(), ui_kit::drive::WINDOW);
    let fit = app.fit_percent(false).expect("a page to fit") as f64 / 100.0;
    assert!(
        (app.view.zoom - fit).abs() < 0.001,
        "the zoom is the full window's fit: {} against {fit}",
        app.view.zoom
    );
    let painted = drive.paint(&mut app, Vec::new());
    let desk = painted.largest(ui_kit::theme::DESK).expect("the desk");
    let paper = painted.largest(egui::Color32::WHITE).expect("the page");
    assert!(
        paper.top() > desk.top() && paper.bottom() < desk.bottom(),
        "the whole page is on the desk: {paper:?} on {desk:?}"
    );
    assert!(
        paper.height() > desk.height() * 0.8,
        "and fills it, rather than the first frames' third: {paper:?} on {desk:?}"
    );
}

/// In Hack, whose metrics are not egui's own face's, the caret is still the
/// height of the type, stands after a letter typed, and a click at each
/// place between the letters lands there. Code that took a height or a
/// width from the face egui ships rather than the face on the page passes
/// every other test and fails this one.
#[test]
fn in_a_face_other_than_eguis_own_the_caret_and_the_click_follow_the_face() {
    let ascent = |drive: &Driver| {
        let mut app = app_with(&["fds"]);
        drive.settle(&mut app);
        drive.settle(&mut app);
        match &app.view.pages()[0].content[0].kind {
            wp_layout::block::Placed::Line { line, .. } => (line.ascent, line.descent, line.width),
            other => panic!("a line, not {other:?}"),
        }
    };
    let drive = Driver::in_hack();
    let (hack_ascent, hack_descent, hack_width) = ascent(&drive);
    let (egui_ascent, egui_descent, egui_width) = ascent(&Driver::new());
    assert!(
        (hack_ascent + hack_descent - egui_ascent - egui_descent).abs() > 0.1
            && (hack_width - egui_width).abs() > 1.0,
        "the document is laid in Hack: {hack_ascent}+{hack_descent} wide {hack_width} \
         against {egui_ascent}+{egui_descent} wide {egui_width}"
    );

    let mut app = app_with(&["fds"]);
    app.selection = Selection::at(Caret {
        paragraph: 0,
        offset: 3,
    });
    drive.settle(&mut app);
    app.run(Command::Zoom(1.0));
    drive.frame_at(&mut app, Vec::new(), Some(10.0));
    let before = caret(&drive.paint_at(&mut app, Some(10.0))).expect("the caret");
    let painted =
        Painted::new(&drive.frame_at(&mut app, vec![egui::Event::Text("x".into())], Some(10.0)));
    let after = caret(&painted).expect("the caret");
    let glass = (app.view.zoom * view::SCALE) as f32;
    assert!(
        (after.min.x - before.min.x - hack_width as f32 / 3.0 * glass).abs() < 1.0,
        "the caret moved one Hack letter on: {} to {}",
        before.min.x,
        after.min.x
    );
    assert!(
        (after.height() - (hack_ascent + hack_descent) as f32 * glass).abs() < 1.0,
        "and is Hack's height: {}",
        after.height()
    );
    assert!(painted.text("fdsx").is_some(), "the letter is painted");

    for offset in 0..=4 {
        let place = Caret {
            paragraph: 0,
            offset,
        };
        let at = app.on_screen(place).expect("drawn") + egui::vec2(0.5, 0.0);
        app.selection = Selection::at(Caret::default());
        drive.click(&mut app, at);
        assert_eq!(app.caret(), place, "a click at {at:?}");
    }
}
