use super::*;

#[test]
fn a_file_that_can_be_written_is_held_by_nobody() {
    let path = std::env::temp_dir().join(format!("calx-lock-{}.xlsx", std::process::id()));
    std::fs::write(&path, b"not really a workbook").expect("writes");
    assert_eq!(locked_by(&path), None);
    // Nor is a file that does not exist yet — that is a Save As, not a lock.
    let missing = path.with_file_name("calx-nothing-here.xlsx");
    assert_eq!(locked_by(&missing), None);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_refused_save_says_what_to_do_about_it() {
    // A directory cannot be opened for writing, which is the same answer
    // Windows gives for a file another program is holding.
    let dir = std::env::temp_dir().join(format!("calx-refused-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory");
    let message = save_trouble(&dir);

    let name = name_of(&dir);
    assert!(message.contains(&name), "{message}");
    assert!(message.contains("open in"), "{message}");
    assert!(
        message.contains("Save As"),
        "the way out is on the box: {message}"
    );
    assert!(
        message.contains("still here"),
        "and it says the work is not lost: {message}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_a_workbook_leaves_nothing_of_it_behind() {
    let mut app = Calx::new();
    app.path = Some(PathBuf::from(r"C:\books\budget.xlsx"));
    type_into(&mut app, "A1", "42");
    app.dialog = Some(Dialog::GoTo {
        text: String::new(),
    });
    assert!(app.edited && !app.undo.is_empty());

    app.close_document();

    assert_eq!(app.path, None, "the file has been let go of");
    assert_eq!(value_at(&app, "A1"), None, "and so has what was in it");
    assert!(!app.edited, "an empty workbook has nothing to save");
    assert!(
        app.undo.is_empty(),
        "an undo naming cells in the old workbook would write into the new one"
    );
    assert!(app.dialog.is_none(), "the boxes went with their subject");
    assert!(
        app.status.contains("budget.xlsx"),
        "the status names what was closed: {}",
        app.status
    );
}

#[test]
fn closing_an_edited_workbook_asks_first() {
    let mut app = Calx::new();
    type_into(&mut app, "A1", "42");

    app.guard(Pending::Close);
    assert_eq!(app.pending, Some(Pending::Close), "the prompt is up");
    assert_eq!(
        value_at(&app, "A1"),
        Some(ss_model::CellValue::Number(42.0)),
        "and nothing has been thrown away while it is"
    );

    // Whereas a workbook with nothing in it to lose closes on the spot.
    let mut untouched = Calx::new();
    untouched.guard(Pending::Close);
    assert_eq!(untouched.pending, None);
    assert_eq!(untouched.status, "Closed");
}

/// A workbook with one protected sheet, and B1 unlocked in it.
fn protected(allow: ss_model::Protection) -> Calx {
    let mut app = Calx::new();
    let book = &mut app.doc.workbook;
    let open = {
        let mut look = book.styles.look(ss_model::StyleId::DEFAULT);
        look.locked = false;
        book.styles.style_for(&look)
    };
    let sheet = book.sheet_mut(0).expect("sheet 0");
    sheet.set(
        CellRef::new(0, 1),
        ss_model::Cell {
            style: open,
            ..Default::default()
        },
    );
    sheet.protection = Some(allow);
    app
}

fn type_into(app: &mut Calx, at: &str, text: &str) {
    let at = CellRef::from_a1(at).expect("valid");
    let change = edit::input(&mut app.doc.workbook, 0, at, text);
    app.perform(change);
}

fn value_at(app: &Calx, at: &str) -> Option<ss_model::CellValue> {
    let at = CellRef::from_a1(at).expect("valid");
    app.doc.workbook.sheet(0)?.get(at).map(|c| c.value)
}

#[test]
fn a_copied_chart_travels_as_the_part_scriva_pastes() {
    // The clipboard payload is the whole `<c:chartSpace>` plus a size in
    // EMUs — everything the other application needs, because a document
    // renders a chart from its caches and has no cells to anchor it to.
    let mut app = Calx::new();
    type_into(&mut app, "A1", "3");
    type_into(&mut app, "A2", "1");
    type_into(&mut app, "A3", "4");
    app.grid.selection = grid::Selection::at(CellRef::from_a1("A1").expect("valid"));
    app.grid.selection.extend_to(
        CellRef::from_a1("A3").expect("valid"),
        app.doc.workbook.sheet(0).expect("sheet 0"),
    );
    app.insert_chart(
        ss_model::ChartKind::Bar,
        ss_model::chart::Grouping::Clustered,
        false,
    );
    assert_eq!(app.grid.selected_chart, Some(0), "the new chart is held");

    let payload = app.chart_payload(0, 0).expect("a payload");
    let (cx, cy, chart_space) =
        ss_model::chart::clipboard::unpack(&payload).expect("our own format");
    assert!(cx > 0 && cy > 0, "a size the drawing can state: {cx}x{cy}");
    let plot = ss_model::chart::read::plot(chart_space).expect("a part the reader draws");
    assert_eq!(plot.series.len(), 1);
    assert_eq!(
        plot.series[0].values,
        vec![Some(3.0), Some(1.0), Some(4.0)],
        "the numbers ride in the caches"
    );
}

/// Frames of `overlay` — the dialogs — each with one key pressed.
fn press_in_dialogs(app: &mut Calx, keys: &[egui::Key]) {
    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    let mut warm = ctx.run_ui(egui::RawInput::default(), |ui| app.overlay(ui.ctx()));
    warm.textures_delta.clear();
    for &key in keys {
        let mut out = ctx.run_ui(input_of(key), |ui| app.overlay(ui.ctx()));
        out.textures_delta.clear();
    }
}

/// One whole frame — the dialogs, then the body with the grid in it —
/// with one key pressed, in the order the shell runs them.
fn whole_frame(app: &mut Calx, ctx: &egui::Context, key: egui::Key) {
    let mut out = ctx.run_ui(input_of(key), |ui| {
        app.overlay(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
}

fn input_of(key: egui::Key) -> egui::RawInput {
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    input
}

/// Driven on the rig: Column Width…, a number, Enter — and the box stayed
/// up, as did Format Cells, Protect Sheet, Data Validation and the rest.
/// They answered only to the pointer. Escape was the one key they knew.
#[test]
fn enter_answers_a_dialog_and_escape_closes_it_unanswered() {
    let mut app = Calx::new();
    app.open_size_dialog(Axis::Columns);
    if let Some(Dialog::Size { text, .. }) = &mut app.dialog {
        *text = "20".to_owned();
    }
    press_in_dialogs(&mut app, &[egui::Key::Enter]);
    assert!(app.dialog.is_none(), "Enter closes the box");
    let width = app
        .doc
        .workbook
        .sheet(0)
        .and_then(|s| s.column_widths.get(&0).copied());
    assert_eq!(width, Some(20.0), "and applies the number");

    app.open_size_dialog(Axis::Columns);
    if let Some(Dialog::Size { text, .. }) = &mut app.dialog {
        *text = "40".to_owned();
    }
    press_in_dialogs(&mut app, &[egui::Key::Escape]);
    assert!(app.dialog.is_none(), "Escape closes it");
    let width = app
        .doc
        .workbook
        .sheet(0)
        .and_then(|s| s.column_widths.get(&0).copied());
    assert_eq!(width, Some(20.0), "and applies nothing");
}

/// The same drive by menu, through the frame the window runs: Alt+E, G
/// opens Go To, the reference is typed into its box and not into the
/// grid, Enter answers it, and the cursor is on C5 with nothing typed.
#[test]
fn go_to_by_menu_letters_takes_the_typed_reference_and_nothing_else_does() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    drive.settle(&mut app);
    drive.menu(&mut app, 'E', 'G');
    assert!(
        matches!(app.dialog, Some(Dialog::GoTo { .. })),
        "Alt+E, G opened Go To"
    );
    drive.type_text(&mut app, "C5");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.dialog.is_none(), "Enter answered it");
    assert_eq!(
        app.grid.selection.cursor(),
        CellRef::from_a1("C5").expect("valid")
    );
    let a1 = CellRef::from_a1("A1").expect("valid");
    assert!(
        app.doc.workbook.sheet(0).and_then(|s| s.get(a1)).is_none(),
        "nothing of the sequence was typed into the grid"
    );
}

fn texts(app: &Calx, cells: &[&str]) -> Vec<String> {
    cells
        .iter()
        .map(|at| app.display_text(0, CellRef::from_a1(at).expect("valid")))
        .collect()
}

/// Data ▸ Sort Ascending by its letters, on a typed column.
#[test]
fn sort_ascending_by_menu_letters_orders_the_typed_column() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    for (at, text) in [
        ("A1", "pear"),
        ("A2", "apple"),
        ("A3", "fig"),
        ("A4", "banana"),
    ] {
        type_into(&mut app, at, text);
    }
    drive.settle(&mut app);
    drive.menu(&mut app, 'D', 'A');
    assert_eq!(
        texts(&app, &["A1", "A2", "A3", "A4"]),
        vec!["apple", "banana", "fig", "pear"]
    );
    drive.menu(&mut app, 'D', 'D');
    assert_eq!(
        texts(&app, &["A1", "A2", "A3", "A4"]),
        vec!["pear", "fig", "banana", "apple"]
    );
}

/// Data ▸ Sort… by its letters, answered with Enter as it stands: the
/// cursor's column, A to Z.
#[test]
fn sort_dialog_by_menu_letters_and_enter_sorts_by_the_cursors_column() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    for (at, text) in [
        ("A1", "3"),
        ("B1", "c"),
        ("A2", "1"),
        ("B2", "a"),
        ("A3", "2"),
        ("B3", "b"),
    ] {
        type_into(&mut app, at, text);
    }
    drive.settle(&mut app);
    drive.menu(&mut app, 'D', 'S');
    assert!(
        matches!(app.dialog, Some(Dialog::Sort { .. })),
        "Alt+D, S opened Sort"
    );
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.dialog.is_none(), "Enter sorted and closed");
    assert_eq!(texts(&app, &["A1", "A2", "A3"]), vec!["1", "2", "3"]);
    assert_eq!(
        texts(&app, &["B1", "B2", "B3"]),
        vec!["a", "b", "c"],
        "rows moved whole"
    );
}

/// Edit ▸ Find… by its letters, the word typed, Enter: the cursor lands
/// on the cell that holds it, and nothing is typed into the grid.
#[test]
fn find_by_menu_letters_takes_the_typed_word_and_lands_on_it() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    for (at, text) in [("A1", "pear"), ("A2", "apple"), ("B3", "fig")] {
        type_into(&mut app, at, text);
    }
    drive.settle(&mut app);
    drive.menu(&mut app, 'E', 'F');
    assert!(
        matches!(app.dialog, Some(Dialog::Find { .. })),
        "Alt+E, F opened Find"
    );
    drive.type_text(&mut app, "fig");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert_eq!(
        app.grid.selection.cursor(),
        CellRef::from_a1("B3").expect("valid"),
        "Enter found it"
    );
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.dialog.is_none(), "Escape closed Find");
    assert_eq!(
        texts(&app, &["A1", "A2", "B3"]),
        vec!["pear", "apple", "fig"]
    );
}

/// Edit ▸ Replace… by its letters: the word, Tab, its replacement, and
/// Replace all reached by Tab and Enter, as a keyboard user would.
#[test]
fn replace_all_by_menu_letters_and_keys_replaces_every_match() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    for (at, text) in [("A1", "fig"), ("A2", "apple"), ("B3", "fig")] {
        type_into(&mut app, at, text);
    }
    drive.settle(&mut app);
    drive.menu(&mut app, 'E', 'E');
    assert!(
        matches!(
            app.dialog,
            Some(Dialog::Find {
                replacing: true,
                ..
            })
        ),
        "Alt+E, E opened Replace"
    );
    drive.type_text(&mut app, "fig");
    drive.press(&mut app, "Tab");
    drive.type_text(&mut app, "kiwi");
    if let Some(Dialog::Find { query, with, .. }) = &app.dialog {
        assert_eq!(
            (query.needle.as_str(), with.as_str()),
            ("fig", "kiwi"),
            "both fields took their text"
        );
    }
    // Backwards along the focus order from the second field: past the
    // first field, round to Find next, Find previous, Replace — and
    // Replace all, which Enter then presses.
    for _ in 0..5 {
        drive.press(&mut app, "shift+Tab");
    }
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert_eq!(
        texts(&app, &["A1", "A2", "B3"]),
        vec!["kiwi", "apple", "kiwi"],
        "Replace all was reached and pressed from the keyboard"
    );
}

/// Edit ▸ Paste Special… by its letters opens the box and Enter answers
/// it. What it pastes comes off the system clipboard, which a test has
/// none of, so the paste itself is Calx's own clipboard tests' business.
#[test]
fn paste_special_by_menu_letters_opens_and_enter_answers() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    type_into(&mut app, "A1", "=1+1");
    drive.settle(&mut app);
    drive.press(&mut app, "ctrl+C");
    assert!(app.clip.is_some(), "Ctrl+C copied the cell");
    drive.press(&mut app, "ArrowRight");
    assert_eq!(
        app.grid.selection.cursor(),
        CellRef::from_a1("B1").expect("valid")
    );
    drive.menu(&mut app, 'E', 'S');
    assert!(
        matches!(app.dialog, Some(Dialog::PasteSpecial { .. })),
        "Alt+E, S opened Paste Special"
    );
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.dialog.is_none(), "Enter answered it");
    assert_eq!(
        texts(&app, &["A1"]),
        vec!["2"],
        "and the copy is still there"
    );
}

/// Data ▸ Text to Columns… and Remove Duplicates… by their letters,
/// answered with Enter as they stand.
#[test]
fn text_to_columns_and_remove_duplicates_by_menu_letters_and_enter() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    type_into(&mut app, "A1", "a,b,c");
    drive.settle(&mut app);
    drive.menu(&mut app, 'D', 'T');
    assert!(
        matches!(app.dialog, Some(Dialog::TextToColumns { .. })),
        "Alt+D, T opened Text to Columns"
    );
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.dialog.is_none());
    assert_eq!(texts(&app, &["A1", "B1", "C1"]), vec!["a", "b", "c"]);

    let mut app = Calx::new();
    for (at, text) in [("A1", "x"), ("A2", "y"), ("A3", "x"), ("A4", "z")] {
        type_into(&mut app, at, text);
    }
    drive.settle(&mut app);
    drive.menu(&mut app, 'D', 'U');
    assert!(
        matches!(app.dialog, Some(Dialog::RemoveDuplicates { .. })),
        "Alt+D, U opened Remove Duplicates"
    );
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.dialog.is_none());
    assert_eq!(
        texts(&app, &["A1", "A2", "A3", "A4"]),
        vec!["x", "y", "z", ""]
    );
}

/// Format ▸ Column ▸ Width… by its letters, a number typed over the one
/// offered, Enter: the number typed is the width, not the two run
/// together.
#[test]
fn column_width_by_menu_letters_takes_the_typed_number_in_place_of_the_offered_one() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    drive.settle(&mut app);
    drive.menu(&mut app, 'O', 'N');
    drive.press(&mut app, "W");
    drive.settle(&mut app);
    assert!(
        matches!(
            app.dialog,
            Some(Dialog::Size {
                axis: Axis::Columns,
                ..
            })
        ),
        "Alt+O, N, W opened Column Width"
    );
    drive.type_text(&mut app, "20");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.dialog.is_none(), "Enter answered it");
    let width = app
        .doc
        .workbook
        .sheet(0)
        .and_then(|s| s.column_widths.get(&0).copied());
    assert_eq!(width, Some(20.0));
}

/// Every menu and submenu, walked by keyboard: no two rows of one menu
/// claim one letter, since the second of them could never be chosen by it.
#[test]
fn no_two_rows_of_a_menu_share_a_letter() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    drive.settle(&mut app);
    let menus = drive.every_menu(&mut app, "FEVIODT");
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
    let clashes = ui_kit::menu::clashes(drive.ctx());
    assert!(clashes.is_empty(), "{}", clashes.join("\n"));
    assert!(unopened.is_empty(), "these did not open: {unopened:?}");
}

/// With Go To up, Alt+D, A sorts nothing behind it: the menus are the
/// box's business while it is open.
#[test]
fn a_menu_letter_does_nothing_behind_an_open_box() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    for (at, text) in [("A1", "pear"), ("A2", "apple"), ("A3", "fig")] {
        type_into(&mut app, at, text);
    }
    drive.settle(&mut app);
    drive.menu(&mut app, 'E', 'G');
    assert!(matches!(app.dialog, Some(Dialog::GoTo { .. })));
    drive.menu(&mut app, 'D', 'A');
    assert_eq!(
        texts(&app, &["A1", "A2", "A3"]),
        vec!["pear", "apple", "fig"],
        "Sort Ascending ran behind the box"
    );
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.dialog.is_none());
}

/// Tools ▸ Define Names… by its letters opens the list, and Escape
/// closes it; a new name is added from its own row.
#[test]
fn names_by_menu_letters_opens_and_escape_closes() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    drive.settle(&mut app);
    drive.menu(&mut app, 'T', 'N');
    assert!(
        matches!(app.dialog, Some(Dialog::Names { .. })),
        "Alt+T, N opened Names"
    );
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.dialog.is_none(), "Escape closed it");
}

/// Driven on the rig: Ctrl+G, C5, Enter landed on C6 — the Enter that
/// answered the box reached the grid too and walked the cursor. With a
/// copy pending it pasted the clipboard at the target instead.
#[test]
fn the_enter_that_answers_a_dialog_does_not_reach_the_grid() {
    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    let mut app = Calx::new();
    app.dialog = Some(Dialog::GoTo {
        text: "C5".to_owned(),
    });
    whole_frame(&mut app, &ctx, egui::Key::Enter);
    assert!(app.dialog.is_none());
    assert_eq!(
        app.grid.selection.cursor(),
        CellRef::from_a1("C5").expect("valid"),
        "the cursor is where Go To put it, not a row further"
    );
    // A frame later the grid has its keys back.
    whole_frame(&mut app, &ctx, egui::Key::Enter);
    assert_eq!(
        app.grid.selection.cursor(),
        CellRef::from_a1("C6").expect("valid")
    );
}

#[test]
fn a_focused_text_box_outside_the_grid_owns_the_keys_and_the_cells_editor_does_not() {
    let ctx = egui::Context::default();
    let focus = |id: &str| {
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(id)));
        keys_belong_elsewhere(&ctx)
    };
    assert!(
        !keys_belong_elsewhere(&ctx),
        "nothing focused, the grid listens"
    );
    assert!(
        focus("calx-chart-title"),
        "a title being typed is not a cell"
    );
    assert!(
        !focus("calx-cell-editor"),
        "the cell's own editor is the grid's"
    );
    assert!(!focus("calx-formula-bar"));
}

#[test]
fn a_scatter_takes_its_first_column_as_x_even_when_it_holds_numbers() {
    // Every other kind only surrenders the first column to labels when it
    // holds text; a scatter's first column being numbers is the point.
    let mut app = Calx::new();
    for (at, value) in [
        ("A1", "1"),
        ("B1", "10"),
        ("A2", "2"),
        ("B2", "20"),
        ("A3", "4"),
        ("B3", "40"),
    ] {
        type_into(&mut app, at, value);
    }
    app.grid.selection = grid::Selection::at(CellRef::from_a1("A1").expect("valid"));
    app.grid.selection.extend_to(
        CellRef::from_a1("B3").expect("valid"),
        app.doc.workbook.sheet(0).expect("sheet 0"),
    );
    app.insert_chart(
        ss_model::ChartKind::Scatter,
        ss_model::chart::Grouping::Standard,
        false,
    );
    let chart = &app.doc.workbook.sheet(0).expect("sheet 0").charts[0];
    assert_eq!(chart.plot.series.len(), 1, "one Y series, not two");
    assert_eq!(chart.plot.series[0].categories, ["1", "2", "4"]);
    assert_eq!(
        chart.plot.series[0].values,
        [Some(10.0), Some(20.0), Some(40.0)]
    );
}

#[test]
fn a_protected_sheet_takes_typing_only_where_it_is_unlocked() {
    let mut app = protected(ss_model::Protection::as_excel_protects());

    type_into(&mut app, "A1", "42");
    assert_eq!(value_at(&app, "A1"), None, "A1 is locked");
    assert!(app.status.contains("A1 is locked"), "{}", app.status);
    assert!(app.undo.is_empty(), "a refused edit is not an undo entry");

    type_into(&mut app, "B1", "42");
    assert_eq!(
        value_at(&app, "B1"),
        Some(ss_model::CellValue::Number(42.0)),
        "B1 was unlocked before the sheet was protected"
    );
}

#[test]
fn an_unprotected_sheet_takes_typing_into_locked_cells() {
    // Every cell in a workbook is locked. Locking means nothing until the
    // sheet is protected, and a guard that forgot this would make a fresh
    // workbook read-only.
    let mut app = Calx::new();
    type_into(&mut app, "A1", "42");
    assert_eq!(
        value_at(&app, "A1"),
        Some(ss_model::CellValue::Number(42.0))
    );
}

#[test]
fn what_a_protected_sheet_allows_is_what_it_allows() {
    let mut app = protected(ss_model::Protection {
        insert_rows: true,
        ..ss_model::Protection::as_excel_protects()
    });
    let rows = |app: &Calx| app.doc.workbook.sheet(0).expect("sheet 0").cells.len();

    app.status.clear();
    app.structural(Axis::Rows, false);
    assert_eq!(app.status, "", "inserting rows was allowed");
    app.structural(Axis::Columns, false);
    assert!(
        app.status.contains("protected sheet"),
        "inserting columns was not: {}",
        app.status
    );
    let _ = rows;
}

#[test]
fn taking_protection_off_is_never_refused_by_the_protection() {
    let mut app = protected(ss_model::Protection::as_excel_protects());
    app.toggle_protection();
    assert!(app
        .doc
        .workbook
        .sheet(0)
        .expect("sheet 0")
        .protection
        .is_none());
    type_into(&mut app, "A1", "42");
    assert_eq!(
        value_at(&app, "A1"),
        Some(ss_model::CellValue::Number(42.0))
    );
}

#[test]
fn a_password_nobody_can_check_is_a_sheet_nobody_can_unprotect() {
    let mut app = protected(ss_model::Protection {
        password: vec![("password".to_string(), "CC3D".to_string())],
        ..ss_model::Protection::as_excel_protects()
    });
    app.toggle_protection();
    assert!(
        app.doc
            .workbook
            .sheet(0)
            .expect("sheet 0")
            .protection
            .is_some(),
        "the sheet stays protected"
    );
    assert!(app.status.contains("password"), "{}", app.status);
}

#[test]
fn a_note_is_written_signed_and_taken_off_again() {
    let mut app = Calx::new();
    let at = CellRef::from_a1("B2").expect("valid");
    app.set_note(at, "Ada", "check the ledger");

    let notes = &app.doc.workbook.sheets[0].comments;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].at, at);
    assert_eq!(notes[0].author, "Ada");
    assert_eq!(
        notes[0].text,
        "Ada:
check the ledger",
        "the author's name goes into the body too, as Excel writes it"
    );
    assert_eq!(notes[0].body(), "check the ledger");

    // Editing replaces rather than adds.
    app.set_note(at, "Ada", "checked");
    assert_eq!(app.doc.workbook.sheets[0].comments.len(), 1);

    // And an empty note is no note.
    app.set_note(at, "Ada", "   ");
    assert!(app.doc.workbook.sheets[0].comments.is_empty());

    app.undo();
    assert_eq!(
        app.doc.workbook.sheets[0].comments.len(),
        1,
        "deleting a note is undoable"
    );
}

#[test]
fn notes_are_kept_in_the_order_the_cells_come_in() {
    // The file lists them in reading order and so should the model: a
    // reader that renumbers author ids by position would otherwise write a
    // different file every time a note was added above another one.
    let mut app = Calx::new();
    for a1 in ["C3", "A1", "B2"] {
        let at = CellRef::from_a1(a1).expect("valid");
        app.set_note(at, "Ada", a1);
    }
    let order: Vec<String> = app.doc.workbook.sheets[0]
        .comments
        .iter()
        .map(|note| note.at.to_a1())
        .collect();
    assert_eq!(order, ["A1", "B2", "C3"]);
}

#[test]
fn a_typed_size_is_taken_only_where_the_file_could_hold_it() {
    assert_eq!(parse_size("12.5", Axis::Columns), Some(12.5));
    assert_eq!(parse_size("  30 ", Axis::Rows), Some(30.0));
    // Zero is a size, and it is how the file spells "hidden".
    assert_eq!(parse_size("0", Axis::Rows), Some(0.0));
    assert_eq!(parse_size("-1", Axis::Rows), None);
    assert_eq!(parse_size("wide", Axis::Columns), None);
    assert_eq!(parse_size("", Axis::Columns), None);
    // Excel's own ceilings, and they differ by axis.
    assert_eq!(parse_size("409", Axis::Rows), Some(409.0));
    assert_eq!(parse_size("409", Axis::Columns), None);
    assert_eq!(parse_size("255", Axis::Columns), Some(255.0));
    assert_eq!(parse_size("inf", Axis::Rows), None);
}

/// Format ▸ Sheet by its letters, and Shift+F11: a sheet added, renamed,
/// hidden and shown again, and never the pointer. The tab's right-click menu
/// and a double-click were the only ways to any of it.
#[test]
fn sheets_are_added_renamed_hidden_and_shown_from_the_keyboard() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = Calx::new();
    drive.settle(&mut app);
    let names = |app: &Calx| -> Vec<String> {
        app.doc
            .workbook
            .sheets
            .iter()
            .map(|s| s.name.clone())
            .collect()
    };
    let before = names(&app).len();

    drive.press(&mut app, "shift+F11");
    drive.settle(&mut app);
    assert_eq!(names(&app).len(), before + 1, "Shift+F11 added a sheet");
    assert_eq!(app.grid.sheet_index, before, "and shows it");

    drive.menu(&mut app, 'O', 'S');
    drive.press(&mut app, "R");
    drive.settle(&mut app);
    assert!(
        matches!(app.dialog, Some(Dialog::RenameSheet { .. })),
        "Alt+O, S, R opened Rename"
    );
    drive.type_text(&mut app, "Budget");
    drive.press(&mut app, "Enter");
    drive.settle(&mut app);
    assert!(app.dialog.is_none(), "Enter answered it");
    assert_eq!(
        names(&app)[before],
        "Budget",
        "the name typed replaced the old one"
    );

    drive.menu(&mut app, 'O', 'S');
    drive.press(&mut app, "H");
    drive.settle(&mut app);
    assert!(app.doc.workbook.sheets[before].hidden, "Alt+O, S, H hid it");
    assert_ne!(app.grid.sheet_index, before, "and shows another");

    drive.menu(&mut app, 'O', 'S');
    drive.press(&mut app, "U");
    drive.settle(&mut app);
    assert!(
        app.doc.workbook.sheets.iter().all(|s| !s.hidden),
        "Alt+O, S, U showed it again"
    );

    drive.menu(&mut app, 'O', 'S');
    drive.press(&mut app, "M");
    drive.settle(&mut app);
    assert!(
        matches!(app.dialog, Some(Dialog::MoveSheet { .. })),
        "Alt+O, S, M opened Move or Copy"
    );
    drive.press(&mut app, "Escape");
    drive.settle(&mut app);
    assert!(app.dialog.is_none());
}
