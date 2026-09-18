use super::*;
use assist::ToolCall;
use serde_json::json;

/// A workbook of three columns and four rows with a header, as a person's
/// sheet begins: A1..C1 named, A2..C4 numbers.
fn sheet_of_numbers() -> Workbook {
    let mut book = Workbook::blank();
    let rows: [(&str, &str); 12] = [
        ("A1", "North"),
        ("B1", "South"),
        ("C1", "East"),
        ("A2", "1"),
        ("B2", "2"),
        ("C2", "3"),
        ("A3", "4"),
        ("B3", "5"),
        ("C3", "6"),
        ("A4", "7"),
        ("B4", "8"),
        ("C4", "9"),
    ];
    for (at, typed) in rows {
        let change = edit::input(&mut book, 0, cell(at), typed);
        edit::apply(&mut book, change);
    }
    ss_formula::recalculate(&mut book);
    book
}

fn cell(a1: &str) -> CellRef {
    CellRef::from_a1(a1).expect("an address")
}

fn range(a1: &str) -> CellRange {
    range_of(a1).expect("a range")
}

fn call(name: &str, input: Value) -> ToolCall {
    ToolCall {
        id: "call_1".into(),
        name: name.into(),
        input,
    }
}

/// Nothing is protected in these tests: the window's own check is what says
/// no, and it has its own tests.
fn open(_book: &Workbook, _change: &Change) -> Option<String> {
    None
}

fn ran(book: &mut Workbook, call: &ToolCall) -> Done {
    run(book, 0, call, &open)
}

fn text(book: &Workbook, a1: &str) -> String {
    text_of(book, 0, cell(a1))
}

/// A request says which sheets there are, which one is showing, what is
/// selected, the cells around it with their formulas, and the workbook's
/// names — and nothing else leaves the machine.
#[test]
fn a_request_carries_the_sheets_the_selection_the_headers_and_the_names() {
    let mut book = sheet_of_numbers();
    book.sheets.push(ss_model::Sheet::new("Notes"));
    let change = edit::input(&mut book, 0, cell("D2"), "=SUM(A2:C2)");
    edit::apply(&mut book, change);
    ss_formula::recalculate(&mut book);
    book.defined_names.push(ss_model::DefinedName {
        name: "Total".into(),
        refers_to: "Sheet1!$D$2".into(),
        scope: None,
    });

    let sent = request(
        &book,
        0,
        range("B2:B3"),
        About::Selection,
        "Add a total row.",
    );
    assert!(sent.contains("2 sheets: Sheet1, Notes"), "{sent}");
    assert!(
        sent.contains("about the cells B2:B3 of \u{201c}Sheet1\u{201d}"),
        "{sent}"
    );
    assert!(sent.contains("Its cells run to A1:D4"), "{sent}");
    // The header row and the rows under it, tab-separated, with the formula
    // beside the value it gives.
    assert!(sent.contains("1\tNorth\tSouth\tEast\t"), "{sent}");
    assert!(sent.contains("2\t1\t2\t3\t6 [=SUM(A2:C2)]"), "{sent}");
    assert!(
        sent.contains("The workbook's names: Total = Sheet1!$D$2"),
        "{sent}"
    );
    assert!(sent.ends_with("The request: Add a total row."), "{sent}");

    // The scope is named in the person's words, whichever chip it is.
    for (about, said) in [
        (About::Cell, "cell B2"),
        (About::Sheet, "the whole sheet"),
        (About::Workbook, "the whole workbook"),
    ] {
        let sent = request(&book, 0, range("B2:B3"), about, "Look.");
        assert!(sent.contains(said), "{about:?}: {sent}");
    }
}

/// A sheet of ten thousand cells does not go over the wire: the request shows
/// what it can, whole rows only, and says what it left out and how to read it.
#[test]
fn the_sample_sent_is_capped_and_says_what_it_left_out() {
    let mut book = Workbook::blank();
    for row in 0..400u32 {
        for col in 0..4u32 {
            let change = edit::input(
                &mut book,
                0,
                CellRef::new(row, col),
                &format!("{}", row * 10 + col),
            );
            edit::apply(&mut book, change);
        }
    }
    ss_formula::recalculate(&mut book);
    let sent = request(&book, 0, range("A1:D6"), About::Sheet, "Sum it.");
    let shown = sent
        .lines()
        .filter(|line| line.starts_with(char::is_numeric))
        .count();
    assert!(shown <= MOST_SHOWN / 4 + 1, "{shown} rows of four");
    assert!(
        sent.contains("Rows 7 to 400 are not shown: read_range shows them."),
        "{sent}"
    );

    // A selection of the whole sheet — one Ctrl+A — is inside the same cap:
    // the request says it cannot show them rather than sending ten thousand
    // cells with a floor under the budget.
    let sent = request(&book, 0, range("A1:Z400"), About::Selection, "Sum it.");
    let rows_sent = sent
        .lines()
        .filter(|line| line.starts_with(|c: char| c.is_ascii_digit()))
        .count();
    assert!(rows_sent * 4 <= MOST_SHOWN + 26, "{rows_sent} rows sent");
    assert!(sent.len() < 6_000, "{} bytes", sent.len());
    assert!(
        sent.contains("A1:Z400"),
        "and it says what it was about: {sent}"
    );

    // And a reading of the whole sheet at once is refused with its size, so
    // that the helper asks for a part instead.
    let done = ran(
        &mut book,
        &call("read_range", json!({"sheet": "Sheet1", "range": "A1:D400"})),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("1600 cells"),
        "{}",
        done.result.content
    );
}

/// Each tool takes what its schema says and nothing else; reading answers
/// with the cells as the grid shows them, and a sheet or a range that is not
/// there is said rather than guessed at.
#[test]
fn the_six_tools_are_strict_and_reading_answers_with_the_cells_as_shown() {
    let named: Vec<String> = tools().iter().map(|tool| tool.name.clone()).collect();
    assert_eq!(
        named,
        [
            "read_range",
            "write_cells",
            "fill",
            "insert",
            "delete",
            "add_sheet"
        ]
    );
    for tool in tools() {
        assert!(
            tool.is_strict(),
            "{} takes what it says and no more",
            tool.name
        );
        assert!(!tool.description.is_empty());
    }

    let mut book = sheet_of_numbers();
    let change = edit::input(&mut book, 0, cell("D2"), "=SUM(A2:C2)");
    edit::apply(&mut book, change);
    ss_formula::recalculate(&mut book);

    let done = ran(
        &mut book,
        &call("read_range", json!({"sheet": "Sheet1", "range": "A1:D2"})),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    assert!(
        done.result.content.contains("1\tNorth\tSouth\tEast"),
        "{}",
        done.result.content
    );
    assert!(
        done.result.content.contains("2\t1\t2\t3\t6 [=SUM(A2:C2)]"),
        "{}",
        done.result.content
    );
    assert_eq!(done.line.as_deref(), Some("read A1:D2 of Sheet1"));
    assert!(done.undo.is_none(), "reading changes nothing");

    // A sheet that is not there, a range that is not one, a tool that is not.
    let done = ran(
        &mut book,
        &call("read_range", json!({"sheet": "Ledger", "range": "A1"})),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("no sheet called"),
        "{}",
        done.result.content
    );
    assert!(done.result.content.contains("Sheet1"), "the ones there are");
    let done = ran(
        &mut book,
        &call(
            "read_range",
            json!({"sheet": "Sheet1", "range": "over there"}),
        ),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("A1 notation"),
        "{}",
        done.result.content
    );
    let done = ran(&mut book, &call("sort_it", json!({})));
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("no tool called"),
        "{}",
        done.result.content
    );

    // A call with no sheet named is about the one the person is looking at.
    let done = ran(&mut book, &call("read_range", json!({"range": "A1"})));
    assert!(!done.result.is_error, "{}", done.result.content);
}

/// A cell that holds something is not written over by accident: the refusal
/// names the cells and changes nothing, and the call passes overwrite to mean
/// it.
#[test]
fn write_cells_refuses_a_cell_that_holds_something_unless_told_to_overwrite() {
    let mut book = sheet_of_numbers();
    let before = text(&book, "B2");
    let over = |overwrite: Option<bool>| {
        let mut input = json!({"sheet": "Sheet1",
                               "cells": [{"at": "B2", "typed": "99"},
                                         {"at": "D1", "typed": "Total"}]});
        if let Some(overwrite) = overwrite {
            input["overwrite"] = json!(overwrite);
        }
        call("write_cells", input)
    };

    let done = ran(&mut book, &over(None));
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("B2 already holds something"),
        "{}",
        done.result.content
    );
    assert!(
        !done.result.content.contains("D1"),
        "not the empty one: {}",
        done.result.content
    );
    assert!(
        done.result.content.contains("overwrite"),
        "and how to mean it"
    );
    assert_eq!(text(&book, "B2"), before, "nothing was written");
    assert_eq!(text(&book, "D1"), "", "not even the empty one");
    assert!(done.undo.is_none());

    let done = ran(&mut book, &over(Some(true)));
    assert!(!done.result.is_error, "{}", done.result.content);
    assert_eq!(text(&book, "B2"), "99");
    assert_eq!(text(&book, "D1"), "Total");
    assert_eq!(done.overwrote, vec![cell("B2")], "the card lists it");
    assert_eq!(done.wrote, vec![cell("B2"), cell("D1")]);

    // And the undo it gives back puts both cells as they were.
    let undo = done.undo.expect("an entry");
    edit::apply(&mut book, undo);
    ss_formula::recalculate(&mut book);
    assert_eq!(text(&book, "B2"), before);
    assert_eq!(text(&book, "D1"), "");
}

/// The engine is the oracle: a formula that comes back an error is in front
/// of the helper, named, before it answers — and a good one comes back as the
/// number the grid shows.
#[test]
fn a_formula_that_evaluates_to_an_error_is_shown_to_the_assistant_before_it_answers() {
    let mut book = sheet_of_numbers();
    let done = ran(
        &mut book,
        &call(
            "write_cells",
            json!({"sheet": "Sheet1", "cells": [{"at": "D2", "typed": "=SUM(A2:C2)"},
                                                {"at": "D3", "typed": "=TOTAL(A3:C3)"}]}),
        ),
    );
    assert!(
        !done.result.is_error,
        "the writing worked: {}",
        done.result.content
    );
    assert!(
        done.result.content.contains("D2: 6 [=SUM(A2:C2)]"),
        "{}",
        done.result.content
    );
    assert!(
        done.result.content.contains("D3: #NAME?"),
        "{}",
        done.result.content
    );
    assert!(
        done.result
            .content
            .contains("put it right before you answer"),
        "{}",
        done.result.content
    );
}

/// Rows and columns put in or taken out, and a sheet added, are changes like
/// any other: each gives back what takes it back, and gathered into one entry
/// they undo to exactly the workbook that was there.
#[test]
fn rows_columns_and_a_sheet_the_assistant_added_are_part_of_the_same_entry() {
    let mut book = sheet_of_numbers();
    let before: Vec<String> = ["A1", "B2", "C4"]
        .iter()
        .map(|at| text(&book, at))
        .collect();
    let mut entry = Change::new("Assistant", Vec::new());

    for (name, input) in [
        (
            "insert",
            json!({"sheet": "Sheet1", "axis": "rows", "at": "1", "count": 2}),
        ),
        (
            "write_cells",
            json!({"sheet": "Sheet1", "cells": [{"at": "A1", "typed": "Report"}]}),
        ),
        (
            "delete",
            json!({"sheet": "Sheet1", "axis": "columns", "at": "C", "count": 1}),
        ),
        ("add_sheet", json!({"name": "Working"})),
    ] {
        let done = ran(&mut book, &call(name, input));
        assert!(!done.result.is_error, "{name}: {}", done.result.content);
        gather(&mut entry, done.undo.expect("an entry"));
    }
    assert_eq!(book.sheets.len(), 2, "the sheet was added");
    assert_eq!(text(&book, "A1"), "Report");
    assert_eq!(text(&book, "A3"), "North", "the rows moved down");

    // One entry, one undo: the workbook is what it was, to the cell.
    edit::apply(&mut book, entry);
    ss_formula::recalculate(&mut book);
    assert_eq!(book.sheets.len(), 1, "and the sheet is gone again");
    let after: Vec<String> = ["A1", "B2", "C4"]
        .iter()
        .map(|at| text(&book, at))
        .collect();
    assert_eq!(after, before);

    // The axis and the position are read in the person's terms, and anything
    // else is said.
    let done = ran(
        &mut book,
        &call(
            "insert",
            json!({"sheet": "Sheet1", "axis": "diagonal", "at": "1"}),
        ),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("rows"),
        "{}",
        done.result.content
    );
    let done = ran(
        &mut book,
        &call(
            "insert",
            json!({"sheet": "Sheet1", "axis": "columns", "at": "4"}),
        ),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("column letter"),
        "{}",
        done.result.content
    );
    let done = ran(&mut book, &call("add_sheet", json!({"name": "Sheet1"})));
    assert!(done.result.is_error, "a name already taken");
}

/// A formula written once and filled down is what the helper is told to do,
/// and what the fill leaves is the formula moved a row at a time.
#[test]
fn a_formula_filled_down_moves_a_row_at_a_time() {
    let mut book = sheet_of_numbers();
    let done = ran(
        &mut book,
        &call(
            "write_cells",
            json!({"sheet": "Sheet1", "cells": [{"at": "D1", "typed": "Total"},
                                                {"at": "D2", "typed": "=SUM(A2:C2)"}]}),
        ),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    let done = ran(
        &mut book,
        &call(
            "fill",
            json!({"sheet": "Sheet1", "from": "D2", "to": "D2:D4"}),
        ),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    assert_eq!(done.wrote.len(), 2, "the cells under the one copied");
    assert_eq!(
        ["D2", "D3", "D4"].map(|at| text(&book, at)).to_vec(),
        ["6", "15", "24"]
    );
    assert!(
        done.result.content.contains("D4: 24 [=SUM(A4:C4)]"),
        "{}",
        done.result.content
    );

    // A fill that does not start where the cells it copies are is refused.
    let done = ran(
        &mut book,
        &call(
            "fill",
            json!({"sheet": "Sheet1", "from": "D2", "to": "E5:E9"}),
        ),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("must begin at"),
        "{}",
        done.result.content
    );
}

/// A fill writes over what is there only when it is told to, covers its
/// source's own rows and columns and no others, and says which cells held
/// something before.
#[test]
fn a_fill_writes_over_nothing_it_was_not_told_to_and_covers_its_own_lanes() {
    let mut book = sheet_of_numbers();
    let before: Vec<String> = ["A3", "A4", "B3"]
        .iter()
        .map(|at| text(&book, at))
        .collect();

    // A3 and A4 hold the person's figures: the fill is refused and nothing
    // moves.
    let done = ran(
        &mut book,
        &call(
            "fill",
            json!({"sheet": "Sheet1", "from": "A2", "to": "A2:A4"}),
        ),
    );
    assert!(done.result.is_error);
    assert!(
        done.result
            .content
            .contains("A3, A4 already hold something"),
        "{}",
        done.result.content
    );
    assert!(
        done.result.content.contains("overwrite"),
        "and how to mean it"
    );
    assert!(done.undo.is_none());
    assert_eq!(
        ["A3", "A4", "B3"].map(|at| text(&book, at)).to_vec(),
        before
    );

    // Told to, it fills and names what it wrote over.
    let done = ran(
        &mut book,
        &call(
            "fill",
            json!({"sheet": "Sheet1", "from": "A2", "to": "A2:A4", "overwrite": true}),
        ),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    assert_eq!(done.overwrote, vec![cell("A3"), cell("A4")]);
    assert_eq!(done.wrote, vec![cell("A3"), cell("A4")]);
    assert_eq!(text(&book, "A3"), "1");

    // A `to` wider than `from` writes only `from`'s own column: what it says
    // it wrote is what the grid shows changed.
    let mut book = sheet_of_numbers();
    let change = edit::input(&mut book, 0, cell("D2"), "=SUM(A2:C2)");
    edit::apply(&mut book, change);
    ss_formula::recalculate(&mut book);
    let done = ran(
        &mut book,
        &call(
            "fill",
            json!({"sheet": "Sheet1", "from": "D2", "to": "D2:E4", "overwrite": true}),
        ),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    assert_eq!(
        done.wrote,
        vec![cell("D3"), cell("D4")],
        "its own column only"
    );

    // And a fill that does not start where its source is says so, naming it.
    let done = ran(
        &mut book,
        &call(
            "fill",
            json!({"sheet": "Sheet1", "from": "D2", "to": "A2:D4"}),
        ),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("must begin at D2"),
        "{}",
        done.result.content
    );
}

/// What a person's own typing cannot do, the assistant cannot do either: a
/// cell inside a pivot table, or one a validation rule stops.
#[test]
fn a_write_passes_the_guards_a_persons_typing_passes() {
    use ss_model::cond::{DataValidation, DvKind, DvOperator, DvSeverity};

    let mut book = sheet_of_numbers();
    book.sheets[0].validations.push(DataValidation {
        ranges: vec![range("B2:B4")],
        kind: DvKind::Whole,
        operator: DvOperator::Between,
        formula1: "1".into(),
        formula2: "10".into(),
        severity: DvSeverity::Stop,
        allow_blank: true,
        ..Default::default()
    });
    let done = ran(
        &mut book,
        &call(
            "write_cells",
            json!({"sheet": "Sheet1", "overwrite": true,
                   "cells": [{"at": "B2", "typed": "500"}]}),
        ),
    );
    assert!(done.result.is_error, "{}", done.result.content);
    assert!(
        done.result.content.contains("does not take"),
        "{}",
        done.result.content
    );
    assert_eq!(text(&book, "B2"), "2", "nothing was written");

    // A value the rule takes goes in as usual.
    let done = ran(
        &mut book,
        &call(
            "write_cells",
            json!({"sheet": "Sheet1", "overwrite": true,
                   "cells": [{"at": "B2", "typed": "5"}]}),
        ),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    assert_eq!(text(&book, "B2"), "5");

    // A pivot table's cells come from its own definition.
    let mut book = sheet_of_numbers();
    book.sheets[0].pivots.push(ss_model::pivot::PivotTable {
        part: "pivotTable1.xml".into(),
        name: "PivotTable1".into(),
        location: range("A1:C4"),
        source: None,
        fields: Vec::new(),
    });
    let done = ran(
        &mut book,
        &call(
            "write_cells",
            json!({"sheet": "Sheet1", "overwrite": true,
                   "cells": [{"at": "B2", "typed": "rewritten"}]}),
        ),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("pivot table"),
        "{}",
        done.result.content
    );
    assert_eq!(text(&book, "B2"), "2");
}

/// A count that is not a count of rows is said rather than taken for one, and
/// a delete beyond the sheet's own cells takes nothing out and says so.
#[test]
fn a_count_of_none_puts_nothing_in_and_a_delete_past_the_end_takes_nothing_out() {
    let mut book = sheet_of_numbers();
    for count in [json!(0), json!(-3), json!("two")] {
        let done = ran(
            &mut book,
            &call(
                "insert",
                json!({"sheet": "Sheet1", "axis": "rows", "at": "1", "count": count}),
            ),
        );
        assert!(done.result.is_error, "{count}: {}", done.result.content);
        assert!(
            done.result.content.contains("one or more"),
            "{}",
            done.result.content
        );
        assert_eq!(text(&book, "A1"), "North", "and the sheet is as it was");
    }

    let done = ran(
        &mut book,
        &call(
            "delete",
            json!({"sheet": "Sheet1", "axis": "rows", "at": "500", "count": 1}),
        ),
    );
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("do not reach row 500"),
        "{}",
        done.result.content
    );
    assert!(done.undo.is_none(), "nothing to undo either");

    // A row that is there comes out.
    let done = ran(
        &mut book,
        &call(
            "delete",
            json!({"sheet": "Sheet1", "axis": "rows", "at": "1", "count": 1}),
        ),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    assert_eq!(text(&book, "A1"), "1");
}

/// A call stopped partway names only the cells it actually wrote over, and
/// says what landed before it stopped.
#[test]
fn a_call_stopped_partway_names_only_what_it_wrote() {
    let mut book = sheet_of_numbers();
    // A guard that refuses everything about C2, as a protected cell would.
    let guard = |_: &Workbook, change: &Change| {
        change
            .patches
            .iter()
            .any(|patch| format!("{patch:?}").contains("col: 2"))
            .then(|| "C2 is locked, and the sheet is protected".to_owned())
    };
    let asked = call(
        "write_cells",
        json!({"sheet": "Sheet1", "overwrite": true,
               "cells": [{"at": "B2", "typed": "20"},
                         {"at": "C2", "typed": "30"},
                         {"at": "A3", "typed": "40"}]}),
    );
    let done = run(&mut book, 0, &asked, &guard);
    assert!(done.result.is_error);
    assert_eq!(done.wrote, vec![cell("B2")]);
    assert_eq!(
        done.overwrote,
        vec![cell("B2")],
        "and not the ones it never reached"
    );
    assert_eq!(done.line.as_deref(), Some("wrote 1 cell before stopping"));
    assert_eq!(text(&book, "C2"), "3", "which are as they were");
    assert_eq!(text(&book, "A3"), "4");
}
