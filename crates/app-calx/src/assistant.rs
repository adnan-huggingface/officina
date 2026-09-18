//! What Calx's helper is shown, and what its tools do to a workbook.
//!
//! The document side of Assist, with no window in it: the words a request
//! carries, the six tools' schemas, and each tool run against a
//! [`Workbook`]. `crate::assisting` — a module of the binary, where `Calx`
//! lives — is the other half: the pane, the keys, the cards, and the one
//! undo entry a whole request becomes.
//!
//! **A tool changes the workbook and says what the grid then shows.** Calx
//! has no tracked changes and grows none for this: an edit lands at once, as
//! a [`Change`] like any other, and what makes it the assistant's is that the
//! window gathers a request's changes into one entry with a label and a card
//! offering Undo. Every tool answers with the cells as they evaluate *after*
//! the edit, because the engine is the only thing that knows whether the
//! formula the helper wrote means anything: a `#NAME?` in front of the helper
//! is one it can still fix before it answers.
//!
//! **A cell that holds something is not overwritten** unless the call says so
//! and names nothing else: the request already showed which cells were empty,
//! so a helper that asks to write over one is guessing, and the refusal names
//! the cells it would have lost.

use assist::{Tool, ToolCall, ToolResult};
use serde_json::{json, Value};
use ss_formula::edit::{self, Change};
use ss_formula::{clip, sheets};
use ss_model::shift::{Axis, Shift};
use ss_model::{CellRange, CellRef, CellValue, FormatValue, Workbook};

/// How many cells of the sheet a request carries, before it says it has left
/// the rest out. Two hundred is a screenful of a real sheet, and a fifth of
/// the smallest helper's window.
pub const MOST_SHOWN: usize = 200;

/// How many rows under the header row a request shows, when the selection
/// does not reach further.
pub const ROWS_SHOWN: u32 = 5;

/// How many cells one tool call may read.
pub const MOST_READ: usize = 1_000;

/// How many cells one call may write.
pub const MOST_WRITTEN: usize = 500;

/// What a request is about, as the pane's scope chips name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum About {
    Cell,
    Selection,
    Sheet,
    Workbook,
}

impl About {
    /// The chips, in the order the pane shows them.
    pub const ALL: [About; 4] = [About::Cell, About::Selection, About::Sheet, About::Workbook];

    pub fn name(self) -> &'static str {
        match self {
            About::Cell => "Cell",
            About::Selection => "Selection",
            About::Sheet => "Sheet",
            About::Workbook => "Whole workbook",
        }
    }
}

/// The tools the helper may ask for, each with a schema it must keep to.
pub fn tools() -> Vec<Tool> {
    let sheet = || json!({"type": "string"});
    vec![
        Tool::new(
            "read_range",
            "Read the cells of a range, as rows of tab-separated values with a formula shown \
             as `=…` beside the value it gives. The sheet is named; the range is in A1 \
             notation, like `B2:D40`.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["sheet", "range"],
                   "properties": {"sheet": sheet(), "range": {"type": "string"}}}),
        ),
        Tool::new(
            "write_cells",
            "Put values or formulas into cells, each entry typed exactly as a person would \
             type it: `=SUM(A2:C2)`, `12`, `2026-01-31`, or plain text. A cell that already \
             holds something is refused unless `overwrite` is true.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["sheet", "cells"],
                   "properties": {
                       "sheet": sheet(),
                       "overwrite": {"type": "boolean"},
                       "cells": {"type": "array", "items": {
                           "type": "object", "additionalProperties": false,
                           "required": ["at", "typed"],
                           "properties": {"at": {"type": "string"},
                                          "typed": {"type": "string"}}}}}}),
        ),
        Tool::new(
            "fill",
            "Copy the cells of `from` over `to`, as dragging the fill handle does: formulas \
             move with the rows or columns they land in. `to` starts at `from` and runs down \
             or across, like `D2:D101`. Cells that already hold something are refused unless \
             `overwrite` is true.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["sheet", "from", "to"],
                   "properties": {"sheet": sheet(), "from": {"type": "string"},
                                  "to": {"type": "string"},
                                  "overwrite": {"type": "boolean"}}}),
        ),
        Tool::new(
            "insert",
            "Put in whole rows or columns at a position, moving what follows down or right. \
             `axis` is \"rows\" or \"columns\"; `at` is the row number or the column letter.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["sheet", "axis", "at", "count"],
                   "properties": {"sheet": sheet(), "axis": {"type": "string"},
                                  "at": {"type": "string"}, "count": {"type": "integer"}}}),
        ),
        Tool::new(
            "delete",
            "Take out whole rows or columns, moving what follows up or left. `axis` is \
             \"rows\" or \"columns\"; `at` is the row number or the column letter.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["sheet", "axis", "at", "count"],
                   "properties": {"sheet": sheet(), "axis": {"type": "string"},
                                  "at": {"type": "string"}, "count": {"type": "integer"}}}),
        ),
        Tool::new(
            "add_sheet",
            "Add a sheet at the end of the workbook, with a name of your choosing.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["name"],
                   "properties": {"name": {"type": "string"}}}),
        ),
    ]
}

/// What the pane is set up with.
pub fn setup() -> ui_kit::assist::Setup {
    ui_kit::assist::Setup {
        system: assist::prompt::calx(),
        tools: tools(),
    }
}

// ---------------------------------------------------------------- the words

/// A cell as the helper reads it: the text the grid shows, and the formula
/// behind it where there is one.
fn shown(book: &Workbook, sheet: usize, at: CellRef) -> String {
    let Some(target) = book.sheet(sheet) else {
        return String::new();
    };
    let text = text_of(book, sheet, at);
    match target.formula_at(at).map(|formula| formula.text.clone()) {
        Some(formula) if !formula.is_empty() => match text.is_empty() {
            true => format!("={formula}"),
            false => format!("{text} [={formula}]"),
        },
        _ => text,
    }
}

/// A cell's text as the grid shows it — the value through the cell's number
/// format, which is what a person reading the screen would tell the helper.
pub fn text_of(book: &Workbook, sheet: usize, at: CellRef) -> String {
    let Some(target) = book.sheet(sheet) else {
        return String::new();
    };
    let Some(cell) = target.get(at) else {
        return String::new();
    };
    let value = match cell.value {
        CellValue::Blank => return String::new(),
        CellValue::Number(n) => FormatValue::Number(n),
        CellValue::Bool(b) => FormatValue::Bool(b),
        CellValue::Error(e) => FormatValue::Error(e),
        CellValue::Text(id) => FormatValue::Text(book.strings.resolve(id)),
    };
    book.styles
        .number_format(target.style_at(at))
        .format(value)
        .text
}

/// The rows of `range` as tab-separated text, each row headed by its number,
/// stopping at `most` cells and saying what was left out.
fn rows(book: &Workbook, sheet: usize, range: CellRange, most: usize) -> String {
    let wide = (range.end.col - range.start.col + 1) as usize;
    let deep = (range.end.row - range.start.row + 1) as usize;
    // Whole rows or none: half a row read as a row is worse than a row fewer.
    let fits = match wide {
        0 => 0,
        _ => (most / wide).min(deep),
    };
    let mut out = String::new();
    for row in range.start.row..range.start.row + fits as u32 {
        let cells: Vec<String> = (range.start.col..=range.end.col)
            .map(|col| shown(book, sheet, CellRef::new(row, col)))
            .collect();
        out.push_str(&format!("{}\t{}\n", row + 1, cells.join("\t")));
    }
    if fits < deep {
        let left = deep - fits;
        let last = range.start.row + fits as u32;
        out.push_str(&format!(
            "({left} more rows, {} to {}, are not shown: read_range shows them.)\n",
            last + 1,
            range.end.row + 1
        ));
    }
    out
}

/// The columns a range covers, as a person names them: "A to D".
fn columns_of(range: CellRange) -> String {
    let first = ss_model::cell::column_name(range.start.col);
    match range.start.col == range.end.col {
        true => first,
        false => format!("{first} to {}", ss_model::cell::column_name(range.end.col)),
    }
}

/// A range in A1 notation, as the helper must write it back.
pub fn a1(range: CellRange) -> String {
    match range.start == range.end {
        true => range.start.to_a1(),
        false => format!("{}:{}", range.start.to_a1(), range.end.to_a1()),
    }
}

/// What the helper is sent: the workbook's sheets, the selection, the cells
/// around it, the workbook's names, and the request in the person's words.
pub fn request(
    book: &Workbook,
    sheet: usize,
    selection: CellRange,
    about: About,
    asked: &str,
) -> String {
    let mut out = String::new();
    let names: Vec<&str> = book
        .sheets
        .iter()
        .map(|target| target.name.as_str())
        .collect();
    let showing = names.get(sheet).copied().unwrap_or("Sheet1");
    out.push_str(&format!(
        "The workbook has {}: {}. The request is about {} of “{showing}”.\n",
        match names.len() {
            1 => "one sheet".to_owned(),
            n => format!("{n} sheets"),
        },
        names.join(", "),
        match about {
            About::Cell => format!("cell {}", selection.start.to_a1()),
            About::Selection => format!("the cells {}", a1(selection)),
            About::Sheet => "the whole sheet".to_owned(),
            About::Workbook => "the whole workbook".to_owned(),
        }
    ));
    let used = book
        .sheet(sheet)
        .and_then(|target| target.used_range())
        .unwrap_or(CellRange::new(CellRef::new(0, 0), CellRef::new(0, 0)));
    out.push_str(&format!(
        "Its cells run to {}; the selection is {}.\n",
        a1(used),
        a1(selection)
    ));

    // The first rows of the used range — the header and a few under it — and
    // the selection, which is what the request is about and may be anywhere.
    // Two hundred cells between them: a sheet of ten thousand is not sent,
    // and what was left out is said rather than trailing off.
    let head = CellRange::new(
        used.start,
        CellRef::new(used.end.row.min(used.start.row + ROWS_SHOWN), used.end.col),
    );
    let shown_head = rows(book, sheet, head, MOST_SHOWN);
    out.push_str(&format!(
        "\nThe first rows of columns {}:\n{shown_head}",
        columns_of(head)
    ));
    if used.end.row > head.end.row {
        out.push_str(&format!(
            "(Rows {} to {} are not shown: read_range shows them.)\n",
            head.end.row + 2,
            used.end.row + 1
        ));
    }
    let below = selection.start.row > head.end.row || selection.end.col > head.end.col;
    if below && about != About::Workbook {
        // What is left of the two hundred, and no floor under it: a selection
        // of the whole sheet is one row of ten thousand cells, and a cap with
        // a "show at least one row" exception is not a cap.
        let left = MOST_SHOWN.saturating_sub(cells_shown(&shown_head, wide_as(head)));
        let shown = rows(book, sheet, selection, left);
        out.push_str(&match shown.is_empty() {
            true => format!(
                "\nThe cells {} the request is about are too many to show here: read_range \
                 shows any part of them.\n",
                a1(selection)
            ),
            false => format!(
                "\nThe cells {} the request is about:\n{shown}",
                a1(selection)
            ),
        });
    }

    let defined: Vec<String> = book
        .defined_names
        .iter()
        .map(|name| format!("{} = {}", name.name, name.refers_to))
        .collect();
    if !defined.is_empty() {
        out.push_str(&format!("\nThe workbook's names: {}\n", defined.join("; ")));
    }
    out.push_str(&format!("\nThe request: {asked}"));
    out
}

/// How many cells a row of `range` is.
fn wide_as(range: CellRange) -> usize {
    (range.end.col - range.start.col + 1) as usize
}

/// How many cells a sample already spent: its rows, each `wide` cells. A note
/// about what was left out is not a row of cells.
fn cells_shown(sample: &str, wide: usize) -> usize {
    sample
        .lines()
        .filter(|line| line.starts_with(|c: char| c.is_ascii_digit()))
        .count()
        .saturating_mul(wide)
}

// ---------------------------------------------------------------- the tools

/// What running one tool call did.
pub struct Done {
    pub result: ToolResult,
    /// A line for the transcript: "wrote 3 cells in D".
    pub line: Option<String>,
    /// What takes this call back, for the window to gather into the one entry
    /// a whole request becomes.
    pub undo: Option<Change>,
    /// The cells the call wrote, for the wash and the card's count.
    pub wrote: Vec<CellRef>,
    /// Those of them that held something before, which the card lists.
    pub overwrote: Vec<CellRef>,
    /// A sheet the call added.
    pub added: Option<usize>,
}

impl Done {
    fn refusal(call: &ToolCall, why: impl Into<String>) -> Done {
        Done {
            result: ToolResult::error(call, why.into()),
            line: None,
            undo: None,
            wrote: Vec::new(),
            overwrote: Vec::new(),
            added: None,
        }
    }

    fn said(call: &ToolCall, words: impl Into<String>) -> Done {
        Done {
            result: ToolResult::ok(call, words.into()),
            line: None,
            undo: None,
            wrote: Vec::new(),
            overwrote: Vec::new(),
            added: None,
        }
    }
}

/// Puts `later`'s patches before `earlier`'s, so that undoing the entry
/// undoes the last call first — which is the only order that gives back what
/// was there, since each call was made against what the one before left.
pub fn gather(entry: &mut Change, later: Change) {
    let mut patches = later.patches;
    patches.append(&mut entry.patches);
    entry.patches = patches;
}

/// What a call is answered with when the person has edited the workbook
/// since the helper was shown it.
///
/// **The addresses the helper has are the ones it was given.** A person who
/// has typed, sorted or inserted a row since has moved the cells under them,
/// so a write that was right when it was decided would land somewhere else.
/// Nothing is done, and the helper is asked to say what it would have done.
pub fn changed_meanwhile(call: &ToolCall) -> Done {
    let mut done = Done::refusal(
        call,
        "Nothing was changed: the person edited the workbook while you were working, so the \
         cells you have in mind may have moved. Say in a sentence what you would have done, \
         and stop; the person can ask again.",
    );
    done.line = Some("changed nothing: the workbook was edited meanwhile".to_owned());
    done
}

/// Whether a tool call changes the workbook, as against reading it.
pub fn edits(call: &ToolCall) -> bool {
    call.name != "read_range"
}

/// Runs one tool call against `book`, with `sheet` the one the person is
/// looking at. `refuses` is the window's protection check, which sees the
/// change before it lands; `None` means it may.
pub fn run(
    book: &mut Workbook,
    sheet: usize,
    call: &ToolCall,
    refuses: &dyn Fn(&Workbook, &Change) -> Option<String>,
) -> Done {
    match call.name.as_str() {
        "read_range" => read_range(book, sheet, call),
        "write_cells" => write_cells(book, sheet, call, refuses),
        "fill" => fill(book, sheet, call, refuses),
        "insert" | "delete" => structural(book, sheet, call, refuses),
        "add_sheet" => add_sheet(book, call, refuses),
        other => Done::refusal(call, format!("There is no tool called `{other}`.")),
    }
}

/// The sheet a call names, or the one the person is looking at.
fn sheet_of(book: &Workbook, sheet: usize, call: &ToolCall) -> Result<usize, String> {
    let Some(named) = call.input.get("sheet").and_then(Value::as_str) else {
        return Ok(sheet);
    };
    let named = named.trim();
    if named.is_empty() {
        return Ok(sheet);
    }
    book.sheets
        .iter()
        .position(|target| target.name.eq_ignore_ascii_case(named))
        .ok_or_else(|| {
            let names: Vec<&str> = book
                .sheets
                .iter()
                .map(|target| target.name.as_str())
                .collect();
            format!(
                "There is no sheet called “{named}”. The workbook's sheets are: {}.",
                names.join(", ")
            )
        })
}

/// A range in A1 notation — `B2`, `B2:D40` — or why it is not one.
fn range_of(text: &str) -> Result<CellRange, String> {
    let text = text.trim();
    let (first, last) = match text.split_once(':') {
        Some((first, last)) => (first, last),
        None => (text, text),
    };
    match (CellRef::from_a1(first), CellRef::from_a1(last)) {
        (Some(start), Some(end)) => Ok(CellRange::new(start, end)),
        _ => Err(format!(
            "“{text}” is not a range: write it in A1 notation, like `B2` or `B2:D40`."
        )),
    }
}

fn words(call: &ToolCall, name: &str) -> Result<String, String> {
    call.input
        .get(name)
        .and_then(Value::as_str)
        .map(|text| text.to_owned())
        .ok_or_else(|| format!("`{name}` must be a string."))
}

fn cells_in(range: CellRange) -> usize {
    let wide = (range.end.col - range.start.col + 1) as usize;
    let deep = (range.end.row - range.start.row + 1) as usize;
    wide.saturating_mul(deep)
}

fn read_range(book: &Workbook, sheet: usize, call: &ToolCall) -> Done {
    let sheet = match sheet_of(book, sheet, call) {
        Ok(sheet) => sheet,
        Err(why) => return Done::refusal(call, why),
    };
    let range = match words(call, "range").and_then(|text| range_of(&text)) {
        Ok(range) => range,
        Err(why) => return Done::refusal(call, why),
    };
    if cells_in(range) > MOST_READ {
        return Done::refusal(
            call,
            format!(
                "That is {} cells; one reading is at most {MOST_READ}. Read it a part at a \
                 time.",
                cells_in(range)
            ),
        );
    }
    let name = book
        .sheet(sheet)
        .map(|target| target.name.clone())
        .unwrap_or_default();
    let mut done = Done::said(
        call,
        format!(
            "“{name}”, {}, a row to a line:\n{}",
            a1(range),
            rows(book, sheet, range, MOST_READ)
        ),
    );
    done.line = Some(format!("read {} of {name}", a1(range)));
    done
}

/// Whether a cell holds anything a person would mind losing.
fn holds_something(book: &Workbook, sheet: usize, at: CellRef) -> bool {
    book.sheet(sheet)
        .and_then(|target| target.get(at))
        .is_some_and(|cell| !cell.value.is_blank() || cell.formula.is_some())
}

/// Why a call that would write over `over` is refused, unless the call says
/// it means to. The request already showed which cells were empty, so a
/// helper writing over one is guessing, and the refusal names what it would
/// have lost.
fn would_lose(call: &ToolCall, over: &[CellRef]) -> Option<Done> {
    let allowed = call
        .input
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if over.is_empty() || allowed {
        return None;
    }
    let named: Vec<String> = over.iter().take(12).map(|at| at.to_a1()).collect();
    let more = match over.len() > named.len() {
        true => format!(" and {} more", over.len() - named.len()),
        false => String::new(),
    };
    Some(Done::refusal(
        call,
        format!(
            "Nothing was written: {}{more} already {}. Read {} first, and pass overwrite if \
             the new values should replace what is there.",
            named.join(", "),
            match over.len() {
                1 => "holds something",
                _ => "hold something",
            },
            match over.len() {
                1 => "it",
                _ => "them",
            }
        ),
    ))
}

/// The cells written, as the grid now shows them, and the errors among them.
fn written_back(book: &Workbook, sheet: usize, cells: &[CellRef]) -> String {
    let mut lines = Vec::new();
    let mut wrong = Vec::new();
    for at in cells.iter().take(MOST_SHOWN) {
        let text = shown(book, sheet, *at);
        if matches!(
            book.sheet(sheet)
                .and_then(|target| target.get(*at))
                .map(|cell| cell.value),
            Some(CellValue::Error(_))
        ) {
            wrong.push(at.to_a1());
        }
        lines.push(format!("{}: {text}", at.to_a1()));
    }
    if cells.len() > MOST_SHOWN {
        lines.push(format!("(and {} more)", cells.len() - MOST_SHOWN));
    }
    let mut out = format!("The grid now shows:\n{}", lines.join("\n"));
    if !wrong.is_empty() {
        out.push_str(&format!(
            "\n{} is an error: put it right before you answer.",
            match wrong.len() {
                1 => wrong[0].clone(),
                _ => format!("{} are errors", wrong.join(", ")),
            }
        ));
    }
    out
}

/// Applies a change the assistant made: the protection check the window
/// gives, then the edit and a recalculation, so that what the tool answers
/// with is what the grid shows.
fn land(
    book: &mut Workbook,
    change: Change,
    refuses: &dyn Fn(&Workbook, &Change) -> Option<String>,
) -> Result<Change, String> {
    if change.is_empty() {
        return Ok(Change::new(String::new(), Vec::new()));
    }
    if let Some(refusal) = refuses(book, &change) {
        return Err(refusal);
    }
    let undo = edit::apply(book, change);
    ss_formula::recalculate(book);
    Ok(undo)
}

fn write_cells(
    book: &mut Workbook,
    sheet: usize,
    call: &ToolCall,
    refuses: &dyn Fn(&Workbook, &Change) -> Option<String>,
) -> Done {
    let sheet = match sheet_of(book, sheet, call) {
        Ok(sheet) => sheet,
        Err(why) => return Done::refusal(call, why),
    };
    let Some(asked) = call.input.get("cells").and_then(Value::as_array) else {
        return Done::refusal(call, "`cells` must be a list of {at, typed}.");
    };
    if asked.is_empty() {
        return Done::refusal(call, "There is nothing to write: `cells` is empty.");
    }
    if asked.len() > MOST_WRITTEN {
        return Done::refusal(
            call,
            format!(
                "That is {} cells; one call writes at most {MOST_WRITTEN}. Write a formula \
                 once and fill it rather than writing each cell.",
                asked.len()
            ),
        );
    }
    let mut wanted: Vec<(CellRef, String)> = Vec::new();
    for entry in asked {
        let Some(at) = entry.get("at").and_then(Value::as_str) else {
            return Done::refusal(call, "Every cell needs `at`, an address like `D1`.");
        };
        let Some(at) = CellRef::from_a1(at) else {
            return Done::refusal(
                call,
                format!("“{at}” is not a cell: write it in A1 notation, like `D1`."),
            );
        };
        let Some(typed) = entry.get("typed").and_then(Value::as_str) else {
            return Done::refusal(call, "Every cell needs `typed`, what to put in it.");
        };
        wanted.push((at, typed.to_owned()));
    }
    // What is there already, before anything is written: a cell this call
    // writes twice is not a cell it overwrote.
    let over: Vec<CellRef> = wanted
        .iter()
        .map(|(at, _)| *at)
        .filter(|at| holds_something(book, sheet, *at))
        .collect();
    if let Some(refusal) = would_lose(call, &over) {
        return refusal;
    }
    // The guards a person's own typing passes. A pivot table's cells are
    // written by its definition, and an edit to one leaves the file
    // self-contradictory; a validation rule that stops a person stops the
    // assistant. Checked for every cell before any of them lands, so that a
    // refusal leaves the sheet as it was.
    for (at, typed) in &wanted {
        if let Some(pivot) = book.sheet(sheet).and_then(|target| target.pivot_at(*at)) {
            return Done::refusal(
                call,
                format!(
                    "Nothing was written: {} is inside the pivot table “{}”, whose cells \
                     come from its own definition.",
                    at.to_a1(),
                    pivot.name
                ),
            );
        }
        let value = edit::typed_value(typed);
        if let Some(refusal) = ss_formula::cond::validate(book, sheet, *at, &value) {
            if refusal.blocks() {
                return Done::refusal(
                    call,
                    format!(
                        "Nothing was written: {} does not take “{typed}”. {}",
                        at.to_a1(),
                        refusal.message
                    ),
                );
            }
        }
    }
    let mut entry = Change::new("Assistant", Vec::new());
    let mut wrote = Vec::new();
    for (at, typed) in &wanted {
        let change = edit::input(book, sheet, *at, typed);
        match land(book, change, refuses) {
            Ok(undo) => {
                gather(&mut entry, undo);
                wrote.push(*at);
            }
            Err(why) => {
                // What landed already stays: it is in the entry, and the
                // person's Undo takes it back with the rest. The card counts
                // what was written, not what was asked for, so the cells this
                // call never reached are not named as written over.
                let overwrote = over
                    .iter()
                    .copied()
                    .filter(|at| wrote.contains(at))
                    .collect();
                let line = (!wrote.is_empty()).then(|| {
                    format!(
                        "wrote {} before stopping",
                        match wrote.len() {
                            1 => "1 cell".to_owned(),
                            n => format!("{n} cells"),
                        }
                    )
                });
                return Done {
                    result: ToolResult::error(call, why),
                    line,
                    undo: Some(entry),
                    wrote,
                    overwrote,
                    added: None,
                };
            }
        }
    }
    let where_ = columns_of(CellRange::new(
        wrote.first().copied().unwrap_or(CellRef::new(0, 0)),
        wrote.last().copied().unwrap_or(CellRef::new(0, 0)),
    ));
    Done {
        result: ToolResult::ok(call, written_back(book, sheet, &wrote)),
        line: Some(format!(
            "wrote {} in {where_}",
            match wrote.len() {
                1 => "1 cell".to_owned(),
                n => format!("{n} cells"),
            }
        )),
        undo: Some(entry),
        wrote,
        overwrote: over,
        added: None,
    }
}

fn fill(
    book: &mut Workbook,
    sheet: usize,
    call: &ToolCall,
    refuses: &dyn Fn(&Workbook, &Change) -> Option<String>,
) -> Done {
    let sheet = match sheet_of(book, sheet, call) {
        Ok(sheet) => sheet,
        Err(why) => return Done::refusal(call, why),
    };
    let from = match words(call, "from").and_then(|text| range_of(&text)) {
        Ok(range) => range,
        Err(why) => return Done::refusal(call, why),
    };
    let to = match words(call, "to").and_then(|text| range_of(&text)) {
        Ok(range) => range,
        Err(why) => return Done::refusal(call, why),
    };
    if cells_in(to) > MOST_WRITTEN {
        return Done::refusal(
            call,
            format!(
                "That fill covers {} cells; one call fills at most {MOST_WRITTEN}.",
                cells_in(to)
            ),
        );
    }
    if to.start != from.start {
        return Done::refusal(
            call,
            format!(
                "A fill starts where the cells it copies are: `to` must begin at {}, \
                 `from`'s own corner, and run down or across from it.",
                from.start.to_a1()
            ),
        );
    }
    // What the fill will actually write. A fill runs one way — down when `to`
    // is deeper than `from`, across otherwise (`clip::fill_series`) — and
    // covers only the lanes `from` itself covers. A `to` that reaches wider
    // as well does not spread the source sideways, and saying it did would
    // put cells on the card, and under the wash, that were never touched.
    let down = to.rows() > from.rows() || to.cols() == from.cols();
    let wrote: Vec<CellRef> = match down {
        true => ((from.end.row + 1)..=to.end.row)
            .flat_map(|row| (from.start.col..=from.end.col).map(move |col| CellRef::new(row, col)))
            .collect(),
        false => (from.start.row..=from.end.row)
            .flat_map(|row| {
                ((from.end.col + 1)..=to.end.col).map(move |col| CellRef::new(row, col))
            })
            .collect(),
    };
    let over: Vec<CellRef> = wrote
        .iter()
        .copied()
        .filter(|at| holds_something(book, sheet, *at))
        .collect();
    if let Some(refusal) = would_lose(call, &over) {
        return refusal;
    }
    let change = clip::fill(book, sheet, from, to);
    let undo = match land(book, change, refuses) {
        Ok(undo) => undo,
        Err(why) => return Done::refusal(call, why),
    };
    Done {
        result: ToolResult::ok(call, written_back(book, sheet, &wrote)),
        line: Some(format!("filled {}", a1(to))),
        undo: Some(undo),
        wrote,
        overwrote: over,
        added: None,
    }
}

fn structural(
    book: &mut Workbook,
    sheet: usize,
    call: &ToolCall,
    refuses: &dyn Fn(&Workbook, &Change) -> Option<String>,
) -> Done {
    let sheet = match sheet_of(book, sheet, call) {
        Ok(sheet) => sheet,
        Err(why) => return Done::refusal(call, why),
    };
    let axis = match words(call, "axis") {
        Ok(axis) => match axis.trim().to_ascii_lowercase().as_str() {
            "rows" | "row" => Axis::Rows,
            "columns" | "column" | "cols" | "col" => Axis::Columns,
            other => {
                return Done::refusal(
                    call,
                    format!("`axis` is “rows” or “columns”, not “{other}”."),
                )
            }
        },
        Err(why) => return Done::refusal(call, why),
    };
    let at = match words(call, "at") {
        Ok(at) => {
            let at = at.trim().to_owned();
            let found = match axis {
                Axis::Rows => at
                    .parse::<u32>()
                    .ok()
                    .filter(|row| *row >= 1)
                    .map(|row| row - 1),
                Axis::Columns => ss_model::cell::column_index(&at),
            };
            match found {
                Some(found) => found,
                None => {
                    return Done::refusal(
                        call,
                        match axis {
                            Axis::Rows => format!("`at` is a row number, like 4, not “{at}”."),
                            Axis::Columns => {
                                format!("`at` is a column letter, like D, not “{at}”.")
                            }
                        },
                    )
                }
            }
        }
        Err(why) => return Done::refusal(call, why),
    };
    let asked_for = call.input.get("count");
    let count = match asked_for {
        None => 1,
        Some(Value::Number(n)) if n.as_u64().is_some_and(|n| n >= 1) => {
            n.as_u64().unwrap_or(1).min(1_000) as u32
        }
        Some(other) => {
            return Done::refusal(
                call,
                format!(
                    "`count` is how many rows or columns, one or more; {other} is not a \
                     number of them."
                ),
            )
        }
    };
    let taking = call.name == "delete";
    // Nothing is there to take out: an entry on the undo stack and a card
    // offering to take back a change nobody can see is worse than a sentence
    // saying the sheet ends before that.
    if taking {
        let used = book.sheet(sheet).and_then(|target| target.used_range());
        let ends = used.map(|used| match axis {
            Axis::Rows => used.end.row,
            Axis::Columns => used.end.col,
        });
        if ends.is_none_or(|ends| at > ends) {
            let where_ = match axis {
                Axis::Rows => format!("row {}", at + 1),
                Axis::Columns => format!("column {}", ss_model::cell::column_name(at)),
            };
            return Done::refusal(
                call,
                format!("Nothing was taken out: the sheet's cells do not reach {where_}."),
            );
        }
    }
    let shift = match taking {
        true => Shift::delete(axis, at, count),
        false => Shift::insert(axis, at, count),
    };
    let change = edit::structural(book, sheet, shift);
    let undo = match land(book, change, refuses) {
        Ok(undo) => undo,
        Err(why) => return Done::refusal(call, why),
    };
    let what = match (axis, taking) {
        (Axis::Rows, true) => "took out",
        (Axis::Rows, false) => "put in",
        (Axis::Columns, true) => "took out",
        (Axis::Columns, false) => "put in",
    };
    let named = match axis {
        Axis::Rows => format!("{count} row{}", if count == 1 { "" } else { "s" }),
        Axis::Columns => format!("{count} column{}", if count == 1 { "" } else { "s" }),
    };
    let where_ = match axis {
        Axis::Rows => format!("row {}", at + 1),
        Axis::Columns => format!("column {}", ss_model::cell::column_name(at)),
    };
    Done {
        result: ToolResult::ok(
            call,
            format!(
                "{what} {named} at {where_}. Everything after it has moved, and the formulas \
                 that pointed at it point at it still."
            ),
        ),
        line: Some(format!("{what} {named} at {where_}")),
        undo: Some(undo),
        wrote: Vec::new(),
        overwrote: Vec::new(),
        added: None,
    }
}

fn add_sheet(
    book: &mut Workbook,
    call: &ToolCall,
    refuses: &dyn Fn(&Workbook, &Change) -> Option<String>,
) -> Done {
    let name = match words(call, "name") {
        Ok(name) => name.trim().to_owned(),
        Err(why) => return Done::refusal(call, why),
    };
    if let Some(refusal) = book.sheet_name_refusal(&name, None) {
        return Done::refusal(call, refusal);
    }
    let at = book.sheets.len();
    let change = sheets::insert(book, at, &name);
    let undo = match land(book, change, refuses) {
        Ok(undo) => undo,
        Err(why) => return Done::refusal(call, why),
    };
    Done {
        result: ToolResult::ok(
            call,
            format!(
                "Added the sheet “{name}”. It is empty, and it is sheet {}.",
                at + 1
            ),
        ),
        line: Some(format!("added the sheet “{name}”")),
        undo: Some(undo),
        wrote: Vec::new(),
        overwrote: Vec::new(),
        added: Some(at),
    }
}

#[cfg(test)]
mod tests;
