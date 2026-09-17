//! Tables: inserting, filling, merging, borders, rows and columns.

use super::*;

/// Insert ▸ Table…, Enter. The form answered only to the pointer, so the
/// Enter did nothing and what was typed next went into its fields.
#[test]
fn enter_inserts_the_table_the_dialog_describes() {
    let drive = ui_kit::drive::Driver::new();
    let mut app = app_with(&["before"]);
    drive.settle(&mut app);
    app.run(Command::InsertTable);
    app.table_draft = Some(["3".to_owned(), "2".to_owned()]);
    drive.press(&mut app, "Enter");
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
    drive.settle(&mut app);
    app.run(Command::InsertTable);
    drive.press(&mut app, "Escape");
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
    drive.press(&mut app, "I");
    drive.settle(&mut app);
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
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|(text, _)| text.starts_with("Nothing to merge")),
        "the status bar says so: {:?}",
        app.notice
    );
    assert!(app.message.is_none(), "and no box stands in the way");
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
    assert!(app.notice.is_some(), "a caret in one cell is told why");
    app.notice = None;
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
    assert!(app.notice.is_some());
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
fn the_table_menu_says_so_when_the_caret_is_not_in_a_table() {
    let mut app = app_with(&["just a paragraph"]);
    app.run(Command::TableBorders(false));
    let (text, _) = app.notice.as_ref().expect("it says why, in the status bar");
    assert!(text.starts_with("Not in a table"), "{text}");
    assert!(app.message.is_none(), "and no box");
    assert!(!app.history.can_undo(), "and nothing was recorded to undo");
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
        drive.press(&mut app, "I");
        drive.settle(&mut app);
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

/// Text selected across the cells of a row and deleted is deleted from
/// every cell, and the cells stay: Word clears what the selection covers
/// in each cell and never joins cells. Across paragraphs the deletion
/// joined the first and the last into one and wrote that over the first
/// cell alone, so the middle cell kept its text and the last its whole.
#[test]
fn deleting_a_selection_across_cells_clears_every_cell_it_covers() {
    let mut app = app_with(&["after"]);
    app.insert_table(1, 3);
    app.type_text("A1");
    app.key(egui::Key::Tab, egui::Modifiers::NONE);
    app.type_text("B1");
    app.key(egui::Key::Tab, egui::Modifiers::NONE);
    app.type_text("C1");
    let texts = |app: &Scriva| -> Vec<String> {
        app.document
            .paragraphs()
            .iter()
            .take(3)
            .map(|p| p.text())
            .collect()
    };
    assert_eq!(texts(&app), ["A1", "B1", "C1"]);
    // From inside the first cell to inside the last.
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 1,
        },
        head: Caret {
            paragraph: 2,
            offset: 1,
        },
    };
    app.key(egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(
        texts(&app),
        ["A", "", "1"],
        "the covered text of every cell is gone and the cells stay"
    );
    let table = app
        .document
        .body
        .iter()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("the table is still there");
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.rows[0].cells.len(), 3, "three cells still");
    assert_eq!(
        app.caret(),
        Caret {
            paragraph: 0,
            offset: 1
        },
        "the caret is where the selection began"
    );
    // The whole row's text, from the start of the first cell to the end of
    // the last.
    app.selection = Selection {
        anchor: Caret {
            paragraph: 0,
            offset: 0,
        },
        head: Caret {
            paragraph: 2,
            offset: 1,
        },
    };
    app.key(egui::Key::Backspace, egui::Modifiers::NONE);
    assert_eq!(texts(&app), ["", "", ""], "every cell is empty");
    // And undo brings it all back.
    app.run(Command::Undo);
    assert_eq!(texts(&app), ["A", "", "1"], "undo restores the cells' text");
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
