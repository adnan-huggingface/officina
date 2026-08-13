//! Copy, paste, and the fill handle.
//!
//! All three are the same operation seen from different angles: take a
//! rectangle of cells, decide what it becomes somewhere else, and write it
//! there. What differs is only how the destination values are derived — paste
//! repeats the source, fill extrapolates it.
//!
//! A clip holds *resolved* values rather than the model's interned string ids
//! and formula-arena handles, because the destination may be a different sheet
//! and, once there is a system clipboard involved, a different program.

use ss_model::{Cell, CellRange, CellRef, CellValue, Formula, StyleId, Workbook};

use crate::edit::{Change, Patch};
use crate::translate::offset;

#[derive(Debug, Clone, PartialEq)]
pub enum ClipValue {
    Blank,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(ss_model::CellError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipCell {
    pub value: ClipValue,
    pub style: StyleId,
    /// Formula text with no leading `=`, re-anchored on paste.
    pub formula: Option<String>,
}

impl Default for ClipCell {
    fn default() -> Self {
        ClipCell {
            value: ClipValue::Blank,
            style: StyleId::DEFAULT,
            formula: None,
        }
    }
}

/// A rectangle lifted out of a sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    /// Where it came from, so a paste knows how far its formulas travelled.
    pub origin: CellRef,
    pub rows: u32,
    pub cols: u32,
    /// Row-major, `rows * cols` entries.
    pub cells: Vec<ClipCell>,
    /// True when the text arrived from outside and has to be read the way typed
    /// input is read — `5` as a number, `1/2/2024` as a date, `=A1` as a
    /// formula. A clip taken from our own sheet already knows its types.
    pub reinterpret: bool,
}

impl Clip {
    pub fn get(&self, row: u32, col: u32) -> &ClipCell {
        &self.cells[(row * self.cols + col) as usize]
    }

    /// The clip as tab-separated text, which is what every other spreadsheet —
    /// Excel included — reads from the system clipboard.
    pub fn to_tsv(&self) -> String {
        let mut out = String::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                if col > 0 {
                    out.push('\t');
                }
                let cell = self.get(row, col);
                match &cell.value {
                    ClipValue::Blank => {}
                    ClipValue::Number(n) => out.push_str(&ss_model::format_general(*n)),
                    ClipValue::Text(t) => out.push_str(t),
                    ClipValue::Bool(b) => out.push_str(if *b { "TRUE" } else { "FALSE" }),
                    ClipValue::Error(e) => out.push_str(e.as_str()),
                }
            }
            out.push('\n');
        }
        out
    }

    /// Reads tab-separated text, as pasted from another program.
    ///
    /// Everything arrives as text: what the source program meant by `5` is not
    /// recoverable from the clipboard, so the paste re-reads each field exactly
    /// as if the user had typed it.
    pub fn from_tsv(text: &str, at: CellRef) -> Clip {
        let lines: Vec<&str> = text
            .strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        let cols = lines
            .iter()
            .map(|l| l.split('\t').count())
            .max()
            .unwrap_or(1) as u32;
        let mut cells = Vec::with_capacity(lines.len() * cols as usize);
        for line in &lines {
            let mut fields = line.split('\t');
            for _ in 0..cols {
                let field = fields.next().unwrap_or("");
                cells.push(ClipCell {
                    value: ClipValue::Text(field.to_string()),
                    style: StyleId::DEFAULT,
                    formula: None,
                });
            }
        }
        Clip {
            origin: at,
            rows: lines.len() as u32,
            cols,
            cells,
            reinterpret: true,
        }
    }
}

/// Lifts a rectangle out of a sheet.
pub fn copy(book: &Workbook, sheet: usize, range: CellRange) -> Option<Clip> {
    let source = book.sheet(sheet)?;
    // A clip is dense — every cell of the rectangle, blanks included, because
    // pasting one has to clear what it lands on. That makes a whole-sheet
    // selection a seventeen-billion-entry allocation, so a range covering an
    // axis end to end is taken as what it describes: the data.
    let range = source.drawn_range(range)?;
    let mut cells = Vec::with_capacity((range.rows() as usize) * (range.cols() as usize));
    for row in range.start.row..=range.end.row {
        for col in range.start.col..=range.end.col {
            let at = CellRef::new(row, col);
            let Some(cell) = source.get(at) else {
                cells.push(ClipCell::default());
                continue;
            };
            cells.push(ClipCell {
                value: match cell.value {
                    CellValue::Blank => ClipValue::Blank,
                    CellValue::Number(n) => ClipValue::Number(n),
                    CellValue::Text(id) => ClipValue::Text(book.strings.resolve(id).to_string()),
                    CellValue::Bool(b) => ClipValue::Bool(b),
                    CellValue::Error(e) => ClipValue::Error(e),
                },
                style: cell.style,
                formula: source
                    .formula_at(at)
                    .filter(|f| !f.text.is_empty())
                    .map(|f| f.text.clone()),
            });
        }
    }
    Some(Clip {
        origin: range.start,
        rows: range.rows(),
        cols: range.cols(),
        cells,
        reinterpret: false,
    })
}

/// Which parts of a clip a paste carries across.
///
/// Excel's Paste Special list, cut down to the entries that mean something
/// different from each other. `Values` is the one worth naming: it lands the
/// *result* a formula produced rather than the formula, which is how a sheet
/// full of computations gets frozen into the numbers it worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PasteKind {
    #[default]
    All,
    /// Contents, formulas included, without disturbing the target's look.
    Formulas,
    /// The result, as a literal. No formulas travel.
    Values,
    /// The look and nothing else: what is in the cells stays.
    Formats,
    ValuesAndFormats,
}

impl PasteKind {
    fn contents(self) -> bool {
        !matches!(self, PasteKind::Formats)
    }
    fn formulas(self) -> bool {
        matches!(self, PasteKind::All | PasteKind::Formulas)
    }
    fn formats(self) -> bool {
        matches!(
            self,
            PasteKind::All | PasteKind::Formats | PasteKind::ValuesAndFormats
        )
    }
    pub fn label(self) -> &'static str {
        match self {
            PasteKind::All => "All",
            PasteKind::Formulas => "Formulas",
            PasteKind::Values => "Values",
            PasteKind::Formats => "Formats",
            PasteKind::ValuesAndFormats => "Values and formats",
        }
    }
    pub const ALL: [PasteKind; 5] = [
        PasteKind::All,
        PasteKind::Formulas,
        PasteKind::Values,
        PasteKind::Formats,
        PasteKind::ValuesAndFormats,
    ];
}

/// The whole of a Paste Special answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PasteSpecial {
    pub kind: PasteKind,
    /// Rows become columns.
    pub transpose: bool,
    /// A blank in the clip leaves whatever is already in the target cell,
    /// rather than emptying it.
    pub skip_blanks: bool,
}

/// Writes a clip at `to`, tiling it if the target is a whole multiple of it.
///
/// Excel tiles: copy one cell, select ten, paste, and all ten are filled. The
/// rule is the same for blocks, and a target that is not a multiple gets one
/// copy anchored at its top-left.
pub fn paste(book: &mut Workbook, sheet: usize, to: CellRange, clip: &Clip) -> Change {
    paste_special(book, sheet, to, clip, PasteSpecial::default())
}

/// A paste that carries only some of what it was given.
pub fn paste_special(
    book: &mut Workbook,
    sheet: usize,
    to: CellRange,
    clip: &Clip,
    how: PasteSpecial,
) -> Change {
    if clip.rows == 0 || clip.cols == 0 {
        return Change::default();
    }
    // Transposing swaps the shape of the block before anything is measured
    // against the target, so tiling counts the same way either way.
    let (rows, cols) = if how.transpose {
        (clip.cols, clip.rows)
    } else {
        (clip.rows, clip.cols)
    };
    let down = if to.rows().is_multiple_of(rows) {
        to.rows() / rows
    } else {
        1
    };
    let across = if to.cols().is_multiple_of(cols) {
        to.cols() / cols
    } else {
        1
    };

    let mut cells = Vec::new();
    for tile_row in 0..down {
        for tile_col in 0..across {
            let anchor = CellRef::new(
                to.start.row + tile_row * rows,
                to.start.col + tile_col * cols,
            );
            for row in 0..rows {
                for col in 0..cols {
                    let at = CellRef::new(anchor.row + row, anchor.col + col);
                    if !at.is_valid() {
                        continue;
                    }
                    // Under a transpose the cell at (row, col) of the result
                    // came from (col, row) of the clip, and so did the address
                    // its formulas were written against.
                    let (from_row, from_col) = if how.transpose {
                        (col, row)
                    } else {
                        (row, col)
                    };
                    let entry = clip.get(from_row, from_col);
                    if how.skip_blanks && entry.value == ClipValue::Blank && entry.formula.is_none()
                    {
                        continue;
                    }
                    let source =
                        CellRef::new(clip.origin.row + from_row, clip.origin.col + from_col);
                    let existing = book.sheet(sheet).and_then(|s| s.get(at)).copied();
                    let cell = materialize(
                        book,
                        sheet,
                        entry,
                        source,
                        at,
                        clip.reinterpret,
                        how.kind,
                        existing,
                    );
                    cells.push((at, Some(cell)));
                }
            }
        }
    }
    Change::new("Paste", vec![Patch::Cells { sheet, cells }])
}

/// Turns one clip cell into a cell of this workbook at `at`.
///
/// `existing` is what is already there, which the partial pastes need: Formats
/// keeps the value and takes the look, and everything else keeps the look
/// unless it was asked for.
#[allow(clippy::too_many_arguments)]
fn materialize(
    book: &mut Workbook,
    sheet: usize,
    clip: &ClipCell,
    from: CellRef,
    at: CellRef,
    reinterpret: bool,
    kind: PasteKind,
    existing: Option<Cell>,
) -> Cell {
    let existing = existing.unwrap_or_default();
    let style = if kind.formats() {
        clip.style
    } else {
        existing.style
    };
    if !kind.contents() {
        // Formats only: what the cell holds is none of this paste's business,
        // formula and all.
        return Cell { style, ..existing };
    }
    if reinterpret {
        if let ClipValue::Text(typed) = &clip.value {
            return crate::edit::typed_cell(book, sheet, style, typed);
        }
    }
    if kind.formulas() {
        if let Some(text) = &clip.formula {
            let rows = i64::from(at.row) - i64::from(from.row);
            let cols = i64::from(at.col) - i64::from(from.col);
            let moved = offset(text, rows, cols).unwrap_or_else(|| text.clone());
            let id = book
                .sheet_mut(sheet)
                .map(|s| s.push_formula(Formula::normal(moved)));
            return Cell {
                value: CellValue::Blank,
                style,
                formula: id,
            };
        }
    }
    Cell {
        value: match &clip.value {
            ClipValue::Blank => CellValue::Blank,
            ClipValue::Number(n) => CellValue::Number(*n),
            ClipValue::Bool(b) => CellValue::Bool(*b),
            ClipValue::Error(e) => CellValue::Error(*e),
            ClipValue::Text(t) => CellValue::Text(book.strings.intern(t)),
        },
        style,
        formula: None,
    }
}

/// Extends `from` to cover `to`, the way dragging the fill handle does.
///
/// The direction is whichever way `to` grew past `from`. Numbers extrapolate,
/// formulas are re-anchored, and anything else repeats.
pub fn fill(book: &mut Workbook, sheet: usize, from: CellRange, to: CellRange) -> Change {
    fill_series(book, sheet, from, to, false)
}

/// [`fill`], with Excel's Ctrl held: whatever the seed would have done, it
/// does the other thing — a lone number counts instead of copying, a run of
/// numbers copies instead of counting, a weekday repeats instead of walking
/// the week.
pub fn fill_series(
    book: &mut Workbook,
    sheet: usize,
    from: CellRange,
    to: CellRange,
    toggle: bool,
) -> Change {
    let Some(source) = copy(book, sheet, from) else {
        return Change::default();
    };
    let vertical = to.rows() > from.rows() || to.cols() == from.cols();
    let mut cells = Vec::new();

    // Along the fill direction each lane — one column when filling down, one
    // row when filling across — is its own series.
    let lanes = if vertical { from.cols() } else { from.rows() };
    let length = if vertical { from.rows() } else { from.cols() };
    let (start, end) = if vertical {
        (to.start.row, to.end.row)
    } else {
        (to.start.col, to.end.col)
    };
    let origin = if vertical {
        from.start.row
    } else {
        from.start.col
    };

    for lane in 0..lanes {
        let seed: Vec<&ClipCell> = (0..length)
            .map(|i| {
                if vertical {
                    source.get(i, lane)
                } else {
                    source.get(lane, i)
                }
            })
            .collect();
        // A lone date counts in days where a lone number would copy — the
        // format is what tells 45870 apart from the 1st of August.
        let dated = matches!(seed[..], [only] if matches!(only.value, ClipValue::Number(_))
            && book.styles.number_format(only.style).is_datetime());
        let series = Series::of(&seed, dated, toggle);

        for index in start..=end {
            let step = index as i64 - origin as i64;
            if (0..length as i64).contains(&step) {
                continue; // the seed itself stays as it is
            }
            let at = if vertical {
                CellRef::new(index, from.start.col + lane)
            } else {
                CellRef::new(from.start.row + lane, index)
            };
            if !at.is_valid() {
                continue;
            }
            let position = step.rem_euclid(length as i64) as u32;
            let template = seed[position as usize];
            let source_at = if vertical {
                CellRef::new(origin + position, from.start.col + lane)
            } else {
                CellRef::new(from.start.row + lane, origin + position)
            };
            let cell = match series.at(step, position) {
                Some(value) => {
                    let filled = ClipCell {
                        value,
                        ..template.clone()
                    };
                    materialize(
                        book,
                        sheet,
                        &filled,
                        source_at,
                        at,
                        false,
                        PasteKind::All,
                        None,
                    )
                }
                None => materialize(
                    book,
                    sheet,
                    template,
                    source_at,
                    at,
                    false,
                    PasteKind::All,
                    None,
                ),
            };
            cells.push((at, Some(cell)));
        }
    }

    Change::new("Fill", vec![Patch::Cells { sheet, cells }])
}

/// What a run of seed cells extrapolates to.
enum Series {
    /// A straight line fitted through the seed values.
    Linear { intercept: f64, slope: f64 },
    /// Text ending in a number, or a member of one of Excel's built-in lists.
    Counted { prefix: String, first: i64 },
    /// A member of one of Excel's built-in lists: weekdays and month names.
    Ring { list: usize, entry: usize },
    /// Nothing extrapolates; the seed just repeats.
    Repeat,
}

impl Series {
    /// `dated` says a lone numeric seed wears a date format; `toggle` is
    /// Ctrl, which flips whatever the seed would otherwise have done.
    fn of(seed: &[&ClipCell], dated: bool, toggle: bool) -> Series {
        if seed.iter().any(|c| c.formula.is_some()) {
            return Series::Repeat;
        }

        let numbers: Option<Vec<f64>> = seed
            .iter()
            .map(|c| match c.value {
                ClipValue::Number(n) => Some(n),
                _ => None,
            })
            .collect();
        if let Some(numbers) = numbers {
            if let [only] = numbers[..] {
                // Excel copies a lone number but *counts* a lone date, one
                // day at a time; Ctrl asks for the other one.
                return if dated != toggle {
                    Series::Linear {
                        intercept: only,
                        slope: 1.0,
                    }
                } else {
                    Series::Repeat
                };
            }
            return if toggle {
                Series::Repeat
            } else {
                Series::fit(&numbers)
            };
        }

        // A single text cell: "Item 4" counts, and so does "Mon" — unless
        // Ctrl asks for plain copies.
        if let [only] = seed {
            if toggle {
                return Series::Repeat;
            }
            if let ClipValue::Text(text) = &only.value {
                if let Some((list, entry)) = list_index(text) {
                    return Series::Ring { list, entry };
                }
                if let Some((prefix, number)) = split_trailing_number(text) {
                    return Series::Counted {
                        prefix,
                        first: number,
                    };
                }
            }
        }
        Series::Repeat
    }

    /// Least squares through the seed, which is what Excel fits.
    ///
    /// For the evenly-spaced seeds people actually type this is the same as
    /// "the difference between the first two", and for `1, 2, 4` it gives
    /// Excel's answer rather than a naive one.
    fn fit(values: &[f64]) -> Series {
        let n = values.len() as f64;
        let mean_x = (n - 1.0) / 2.0;
        let mean_y = values.iter().sum::<f64>() / n;
        let mut num = 0.0;
        let mut den = 0.0;
        for (i, y) in values.iter().enumerate() {
            let dx = i as f64 - mean_x;
            num += dx * (y - mean_y);
            den += dx * dx;
        }
        let slope = if den == 0.0 { 0.0 } else { num / den };
        Series::Linear {
            intercept: mean_y - slope * mean_x,
            slope,
        }
    }

    /// The value `step` places past the start of the seed, where `position` is
    /// which seed cell it lines up with when the series merely repeats.
    fn at(&self, step: i64, position: u32) -> Option<ClipValue> {
        match self {
            Series::Repeat => None,
            Series::Linear { intercept, slope } => {
                Some(ClipValue::Number(intercept + slope * step as f64))
            }
            Series::Counted { prefix, first } => Some(ClipValue::Text(format!(
                "{prefix}{}",
                first + step - i64::from(position)
            ))),
            Series::Ring { list, entry } => {
                let entries = LISTS[*list];
                let slot = (*entry as i64 + step).rem_euclid(entries.len() as i64);
                Some(ClipValue::Text(entries[slot as usize].to_string()))
            }
        }
    }
}

/// Excel's built-in fill lists, flattened: dragging `Mon` gives `Tue`.
const LISTS: [&[&str]; 4] = [
    &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    &[
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    &[
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    &[
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
];

fn list_index(text: &str) -> Option<(usize, usize)> {
    LISTS.iter().enumerate().find_map(|(list, entries)| {
        let entry = entries.iter().position(|e| e.eq_ignore_ascii_case(text))?;
        Some((list, entry))
    })
}

/// Splits `"Item 4"` into `("Item ", 4)`.
fn split_trailing_number(text: &str) -> Option<(String, i64)> {
    let digits = text.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits == text.len() {
        return None;
    }
    let cut = text.len() - digits;
    Some((text[..cut].to_string(), text[cut..].parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::apply;

    fn book_with(values: &[(&str, f64)]) -> Workbook {
        let mut book = Workbook::blank();
        for (a1, value) in values {
            let at = CellRef::from_a1(a1).expect("valid address");
            book.sheets[0].set(
                at,
                Cell {
                    value: CellValue::Number(*value),
                    ..Default::default()
                },
            );
        }
        book
    }

    fn at(a1: &str) -> CellRef {
        CellRef::from_a1(a1).expect("valid address")
    }

    fn range(a: &str, b: &str) -> CellRange {
        CellRange::new(at(a), at(b))
    }

    fn shown(book: &Workbook, a1: &str) -> String {
        match book.sheets[0].get(at(a1)).map(|c| c.value) {
            Some(CellValue::Number(n)) => ss_model::format_general(n),
            Some(CellValue::Text(id)) => book.strings.resolve(id).to_string(),
            Some(CellValue::Bool(b)) => if b { "TRUE" } else { "FALSE" }.to_string(),
            _ => String::new(),
        }
    }

    fn formula(book: &Workbook, a1: &str) -> Option<String> {
        Some(book.sheets[0].formula_at(at(a1))?.text.clone())
    }

    #[test]
    fn copying_a_selection_that_covers_the_sheet_copies_the_data_in_it() {
        // Not an optimization: a clip holds every cell of its rectangle, and
        // the corner box selects seventeen billion of them.
        let book = book_with(&[("B2", 1.0), ("C3", 2.0)]);
        let everything = CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(ss_model::cell::MAX_ROWS - 1, ss_model::cell::MAX_COLS - 1),
        );
        let clip = copy(&book, 0, everything).expect("copies");
        assert_eq!(clip.origin, at("B2"));
        assert_eq!((clip.rows, clip.cols), (2, 2));
        assert_eq!(clip.cells.len(), 4);
    }

    #[test]
    fn copying_a_column_keeps_the_column_and_trims_the_rows() {
        let book = book_with(&[("B2", 1.0), ("B4", 2.0)]);
        let whole_column = CellRange::new(
            CellRef::new(0, 1),
            CellRef::new(ss_model::cell::MAX_ROWS - 1, 1),
        );
        let clip = copy(&book, 0, whole_column).expect("copies");
        assert_eq!(clip.origin, at("B2"));
        assert_eq!((clip.rows, clip.cols), (3, 1), "B2:B4, blank row and all");
    }

    #[test]
    fn a_range_drawn_around_blank_cells_still_carries_them() {
        // The trim is only for ranges nobody drew cell by cell. Copying ten
        // rows of which one holds anything and pasting them elsewhere is meant
        // to clear the other nine, and a clip that forgot them would not.
        let book = book_with(&[("A1", 1.0)]);
        let clip = copy(&book, 0, range("A1", "B5")).expect("copies");
        assert_eq!((clip.rows, clip.cols), (5, 2));
        assert_eq!(clip.cells.len(), 10);
    }

    /// A1:A2 hold 3 and 4; A3 is `=A1*A2` with its answer cached.
    fn with_a_formula() -> Workbook {
        let mut book = book_with(&[("A1", 3.0), ("A2", 4.0)]);
        let id = book.sheets[0].push_formula(Formula::normal("A1*A2"));
        book.sheets[0].set(
            at("A3"),
            Cell {
                value: CellValue::Number(12.0),
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );
        book
    }

    #[test]
    fn ctrl_flips_what_the_fill_handle_would_have_done() {
        // A lone number copies; Ctrl makes it count.
        let mut book = book_with(&[("A1", 5.0)]);
        let change = fill_series(&mut book, 0, range("A1", "A1"), range("A1", "A3"), true);
        apply(&mut book, change);
        assert_eq!(
            (shown(&book, "A2"), shown(&book, "A3")),
            ("6".into(), "7".into())
        );

        // Two numbers count; Ctrl makes them tile as copies.
        let mut book = book_with(&[("B1", 1.0), ("B2", 2.0)]);
        let change = fill_series(&mut book, 0, range("B1", "B2"), range("B1", "B4"), true);
        apply(&mut book, change);
        assert_eq!(
            (shown(&book, "B3"), shown(&book, "B4")),
            ("1".into(), "2".into())
        );

        // And a weekday holds still under Ctrl instead of walking the week.
        let mut book = Workbook::blank();
        let id = book.strings.intern("Mon");
        book.sheets[0].set(
            at("C1"),
            Cell {
                value: CellValue::Text(id),
                ..Default::default()
            },
        );
        let change = fill_series(&mut book, 0, range("C1", "C1"), range("C1", "C2"), true);
        apply(&mut book, change);
        assert_eq!(shown(&book, "C2"), "Mon");
    }

    #[test]
    fn a_lone_date_counts_in_days_where_a_number_copies() {
        let mut book = book_with(&[("A1", 45_000.0)]);
        let dated = book.styles.restyle(StyleId::DEFAULT, |l| {
            l.number_format = "d-mmm-yy".to_string();
        });
        book.sheets[0].set(
            at("A1"),
            Cell {
                value: CellValue::Number(45_000.0),
                style: dated,
                formula: None,
            },
        );
        let change = fill(&mut book, 0, range("A1", "A1"), range("A1", "A3"));
        apply(&mut book, change);
        let serial = |book: &Workbook, a1: &str| match book.sheets[0].get(at(a1)).map(|c| c.value) {
            Some(CellValue::Number(n)) => n,
            other => panic!("expected a number, got {other:?}"),
        };
        assert_eq!(serial(&book, "A2"), 45_001.0, "one day later");
        assert_eq!(serial(&book, "A3"), 45_002.0);
        assert_eq!(
            book.sheets[0].get(at("A2")).map(|c| c.style),
            Some(dated),
            "the format travels with the fill"
        );
        // Ctrl+drag on the same date copies it instead.
        let change = fill_series(&mut book, 0, range("A1", "A1"), range("A1", "A2"), true);
        apply(&mut book, change);
        assert_eq!(serial(&book, "A2"), 45_000.0);
    }

    #[test]
    fn pasting_values_lands_the_answer_and_not_the_sum_that_found_it() {
        // The whole point of Paste Values: a sheet of computations frozen into
        // the numbers it worked out, with nothing left pointing at the cells
        // that are about to be deleted.
        let mut book = with_a_formula();
        let clip = copy(&book, 0, range("A3", "A3")).expect("copied");
        let change = paste_special(
            &mut book,
            0,
            range("C1", "C1"),
            &clip,
            PasteSpecial {
                kind: PasteKind::Values,
                ..Default::default()
            },
        );
        apply(&mut book, change);
        assert_eq!(formula(&book, "C1"), None, "no formula travelled");
        assert_eq!(shown(&book, "C1"), "12");

        // And the ordinary paste still carries the formula, re-anchored.
        let change = paste(&mut book, 0, range("D3", "D3"), &clip);
        apply(&mut book, change);
        assert_eq!(formula(&book, "D3").as_deref(), Some("D1*D2"));
    }

    #[test]
    fn pasting_formats_leaves_the_contents_where_they_are() {
        let mut book = book_with(&[("A1", 1.0), ("B1", 2.0)]);
        // Give A1 a style nobody else has.
        let look = book.styles.look(StyleId::DEFAULT);
        let styled = book.styles.restyle(StyleId::DEFAULT, |l| {
            l.font.bold = true;
        });
        assert_ne!(styled, StyleId::DEFAULT, "{look:?}");
        book.sheets[0].set(
            at("A1"),
            Cell {
                value: CellValue::Number(1.0),
                style: styled,
                formula: None,
            },
        );

        let clip = copy(&book, 0, range("A1", "A1")).expect("copied");
        let change = paste_special(
            &mut book,
            0,
            range("B1", "B1"),
            &clip,
            PasteSpecial {
                kind: PasteKind::Formats,
                ..Default::default()
            },
        );
        apply(&mut book, change);
        assert_eq!(shown(&book, "B1"), "2", "what was there stayed");
        assert_eq!(book.sheets[0].get(at("B1")).map(|c| c.style), Some(styled));
    }

    #[test]
    fn transposing_turns_the_rows_into_columns_and_moves_the_formulas_with_them() {
        let mut book = book_with(&[("A1", 1.0), ("B1", 2.0), ("C1", 3.0)]);
        let clip = copy(&book, 0, range("A1", "C1")).expect("copied");
        let change = paste_special(
            &mut book,
            0,
            range("A3", "A5"),
            &clip,
            PasteSpecial {
                transpose: true,
                ..Default::default()
            },
        );
        apply(&mut book, change);
        assert_eq!(
            [shown(&book, "A3"), shown(&book, "A4"), shown(&book, "A5")],
            ["1", "2", "3"]
        );
    }

    #[test]
    fn skipping_blanks_leaves_what_was_already_there() {
        let mut book = book_with(&[
            ("A1", 1.0),
            ("A3", 3.0),
            ("C1", 9.0),
            ("C2", 9.0),
            ("C3", 9.0),
        ]);
        let clip = copy(&book, 0, range("A1", "A3")).expect("copied");
        let change = paste_special(
            &mut book,
            0,
            range("C1", "C3"),
            &clip,
            PasteSpecial {
                skip_blanks: true,
                ..Default::default()
            },
        );
        apply(&mut book, change);
        assert_eq!(
            [shown(&book, "C1"), shown(&book, "C2"), shown(&book, "C3")],
            ["1", "9", "3"],
            "A2 was blank, so C2 was left alone"
        );
    }

    #[test]
    fn pasting_a_formula_re_anchors_its_relative_references() {
        let mut book = book_with(&[("A1", 1.0), ("A2", 2.0)]);
        let id = book.sheets[0].push_formula(Formula::normal("A1+A2"));
        book.sheets[0].set(
            at("A3"),
            Cell {
                value: CellValue::Number(3.0),
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );

        let clip = copy(&book, 0, range("A3", "A3")).expect("copied");
        let change = paste(&mut book, 0, range("C5", "C5"), &clip);
        apply(&mut book, change);

        assert_eq!(formula(&book, "C5").as_deref(), Some("C3+C4"));
    }

    #[test]
    fn pasting_one_cell_into_a_block_tiles_it() {
        let mut book = book_with(&[("A1", 7.0)]);
        let clip = copy(&book, 0, range("A1", "A1")).expect("copied");
        let change = paste(&mut book, 0, range("B1", "C2"), &clip);
        apply(&mut book, change);

        for a1 in ["B1", "B2", "C1", "C2"] {
            assert_eq!(shown(&book, a1), "7", "{a1}");
        }
    }

    #[test]
    fn filling_two_numbers_continues_the_series() {
        let mut book = book_with(&[("A1", 1.0), ("A2", 3.0)]);
        let change = fill(&mut book, 0, range("A1", "A2"), range("A1", "A5"));
        apply(&mut book, change);

        assert_eq!(shown(&book, "A3"), "5");
        assert_eq!(shown(&book, "A4"), "7");
        assert_eq!(shown(&book, "A5"), "9");
    }

    #[test]
    fn filling_one_number_repeats_it_the_way_excel_does() {
        // Dragging a lone `5` gives five fives, not 5, 6, 7.
        let mut book = book_with(&[("A1", 5.0)]);
        let change = fill(&mut book, 0, range("A1", "A1"), range("A1", "A3"));
        apply(&mut book, change);
        assert_eq!(shown(&book, "A2"), "5");
        assert_eq!(shown(&book, "A3"), "5");
    }

    #[test]
    fn filling_a_formula_moves_its_references_down_with_it() {
        let mut book = book_with(&[("A1", 1.0), ("A2", 2.0), ("A3", 3.0)]);
        let id = book.sheets[0].push_formula(Formula::normal("A1*2"));
        book.sheets[0].set(
            at("B1"),
            Cell {
                value: CellValue::Blank,
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );

        let change = fill(&mut book, 0, range("B1", "B1"), range("B1", "B3"));
        apply(&mut book, change);
        assert_eq!(formula(&book, "B2").as_deref(), Some("A2*2"));
        assert_eq!(formula(&book, "B3").as_deref(), Some("A3*2"));
    }

    #[test]
    fn filling_text_counts_when_it_ends_in_a_number() {
        let mut book = Workbook::blank();
        let id = book.strings.intern("Item 4");
        book.sheets[0].set(
            at("A1"),
            Cell {
                value: CellValue::Text(id),
                ..Default::default()
            },
        );
        let change = fill(&mut book, 0, range("A1", "A1"), range("A1", "A3"));
        apply(&mut book, change);
        assert_eq!(shown(&book, "A2"), "Item 5");
        assert_eq!(shown(&book, "A3"), "Item 6");
    }

    #[test]
    fn filling_a_weekday_walks_the_built_in_list() {
        let mut book = Workbook::blank();
        let id = book.strings.intern("Fri");
        book.sheets[0].set(
            at("A1"),
            Cell {
                value: CellValue::Text(id),
                ..Default::default()
            },
        );
        let change = fill(&mut book, 0, range("A1", "A1"), range("A1", "A4"));
        apply(&mut book, change);
        assert_eq!(shown(&book, "A2"), "Sat");
        assert_eq!(shown(&book, "A3"), "Sun", "the list wraps");
        assert_eq!(shown(&book, "A4"), "Mon");
    }

    #[test]
    fn filling_across_works_the_same_as_filling_down() {
        let mut book = book_with(&[("A1", 10.0), ("B1", 20.0)]);
        let change = fill(&mut book, 0, range("A1", "B1"), range("A1", "D1"));
        apply(&mut book, change);
        assert_eq!(shown(&book, "C1"), "30");
        assert_eq!(shown(&book, "D1"), "40");
    }

    #[test]
    fn tab_separated_text_round_trips() {
        let book = book_with(&[("A1", 1.0), ("B1", 2.0), ("A2", 3.0), ("B2", 4.0)]);
        let clip = copy(&book, 0, range("A1", "B2")).expect("copied");
        assert_eq!(clip.to_tsv(), "1\t2\n3\t4\n");

        let read = Clip::from_tsv("1\t2\n3\t4\n", at("A1"));
        assert_eq!(read.rows, 2);
        assert_eq!(read.cols, 2);
        assert_eq!(read.get(1, 1).value, ClipValue::Text("4".into()));
    }

    #[test]
    fn ragged_pasted_text_is_padded_rather_than_misaligned() {
        // A row with fewer tabs must not shift the next row's columns left.
        let clip = Clip::from_tsv("a\tb\tc\nd\n", at("A1"));
        assert_eq!(clip.cols, 3);
        assert_eq!(clip.get(1, 0).value, ClipValue::Text("d".into()));
        assert_eq!(clip.get(1, 2).value, ClipValue::Text(String::new()));
    }
}
