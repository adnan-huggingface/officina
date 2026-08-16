//! Editing a workbook, and being able to take it back.
//!
//! Every change here is a value, not a method call, and applying one returns the
//! change that undoes it. Undo is then not a second implementation of every
//! operation — the thing that got applied *is* the thing that gets un-applied,
//! so the two cannot drift apart. Redo falls out for free: applying the undo
//! returns the redo.
//!
//! Structural changes are why this lives in `ss-formula` rather than `ss-model`.
//! Inserting a row moves cells, which the model can do, and rewrites every
//! formula that pointed at them, which needs the lexer. Splitting those two
//! across a chunk boundary would leave a state where the data moved and the
//! formulas did not.
//!
//! What an undo entry costs is bounded by what actually changed: the cells a
//! deletion destroyed, the formulas whose text was rewritten, and one small
//! snapshot of merges, sizes, and the freeze. Nothing here clones a sheet.

use std::borrow::Cow;
use std::collections::BTreeMap;

use ss_model::{
    Axis, Cell, CellRange, CellRef, CellValue, Formula, FormulaId, Shift, StyleId, Workbook,
};

use crate::translate::translate;
use crate::value::Value;

/// Merges, sizes, and the pane division — everything positional a sheet
/// holds that is not a cell.
///
/// Snapshotted whole rather than patched, because it is small and because
/// getting a partial merge restore wrong is silent.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Geometry {
    pub merges: Vec<CellRange>,
    pub column_widths: BTreeMap<u32, f64>,
    pub row_heights: BTreeMap<u32, f64>,
    pub panes: Option<ss_model::Panes>,
    pub row_outlines: BTreeMap<u32, u8>,
    pub column_outlines: BTreeMap<u32, u8>,
    pub row_collapsed: std::collections::BTreeSet<u32>,
    pub column_collapsed: std::collections::BTreeSet<u32>,
}

impl Geometry {
    pub fn of(sheet: &ss_model::Sheet) -> Self {
        Geometry {
            merges: sheet.merges.clone(),
            column_widths: sheet.column_widths.clone(),
            row_heights: sheet.row_heights.clone(),
            panes: sheet.panes,
            row_outlines: sheet.row_outlines.clone(),
            column_outlines: sheet.column_outlines.clone(),
            row_collapsed: sheet.row_collapsed.clone(),
            column_collapsed: sheet.column_collapsed.clone(),
        }
    }

    fn restore(self, sheet: &mut ss_model::Sheet) -> Geometry {
        let previous = Geometry::of(sheet);
        sheet.merges = self.merges;
        sheet.column_widths = self.column_widths;
        sheet.row_heights = self.row_heights;
        sheet.panes = self.panes;
        sheet.row_outlines = self.row_outlines;
        sheet.column_outlines = self.column_outlines;
        sheet.row_collapsed = self.row_collapsed;
        sheet.column_collapsed = self.column_collapsed;
        previous
    }
}

/// One indivisible piece of a change.
#[derive(Debug, Clone, PartialEq)]
pub enum Patch {
    /// Cells written to the given contents. `None` means "make it empty".
    Cells {
        sheet: usize,
        cells: Vec<(CellRef, Option<Cell>)>,
    },
    /// Formula text replaced in a sheet's arena.
    ///
    /// Text only: a structural shift moves an array formula's *range* itself,
    /// and writing back a whole `Formula` here would undo that.
    Formulas {
        sheet: usize,
        texts: Vec<(FormulaId, String)>,
    },
    Geometry {
        sheet: usize,
        geometry: Geometry,
    },
    /// Rows or columns inserted or deleted, moving everything positional.
    Shift {
        sheet: usize,
        shift: Shift,
    },
    /// A band of rows or columns picked up and put down elsewhere.
    ///
    /// Its own patch rather than a delete and an insert, because it is not
    /// one: nothing is destroyed, so nothing has to be carried in the undo but
    /// the opposite move.
    Rearrange {
        sheet: usize,
        rearrange: ss_model::Move,
    },
    /// Rows of a column band reordered: the row landing at `first + i` is the
    /// one `order[i]` holds now.
    ///
    /// What a sort is, when the rows it moves carry no formulas. A sort is a
    /// permutation — every row that leaves a position is a row arriving at
    /// another one — so the undo is the inverse permutation rather than a copy
    /// of everything that moved. On a hundred thousand rows that is the
    /// difference between half a megabyte of undo and a hundred megabytes of
    /// it. Rows whose formulas have to be rewritten as they travel are not a
    /// permutation of the cells they were, and go through `Cells` instead.
    Permute {
        sheet: usize,
        first: u32,
        cols: (u32, u32),
        order: Vec<u32>,
    },
    /// A chart's title. The rest of a chart is preserved verbatim, so this is
    /// the only thing in one the model can disagree with the file about.
    ChartTitle {
        sheet: usize,
        chart: usize,
        title: Option<String>,
    },
    /// Every picture on a sheet, replaced wholesale.
    ///
    /// Wholesale because a sheet holds a handful of them, not a million, and
    /// because moving one and deleting another are then the same operation
    /// with the same inverse — the list as it was. A per-picture patch would
    /// need indices that stay valid across a deletion, which is exactly the
    /// bookkeeping this avoids.
    Pictures {
        sheet: usize,
        pictures: Vec<ss_model::Picture>,
    },
    /// Every chart on a sheet, replaced wholesale — the same shape as
    /// `Pictures`, for the same reasons.
    Charts {
        sheet: usize,
        charts: Vec<ss_model::Chart>,
    },
    /// Every data-validation rule on a sheet, replaced wholesale. A sheet
    /// holds a handful, and add/edit/delete then share one inverse: the list
    /// as it was.
    Validations {
        sheet: usize,
        validations: Vec<ss_model::cond::DataValidation>,
    },
    /// Every conditional-formatting block on a sheet, wholesale — and here
    /// wholesale is also the *honest* choice: priorities are workbook-wide
    /// and blocks merge and split as regions change, so no per-rule index
    /// would survive an edit.
    ConditionalFormats {
        sheet: usize,
        formats: Vec<ss_model::cond::ConditionalFormat>,
    },
    /// A style put on whole rows or columns.
    ///
    /// Shading a column is not shading a million cells: Excel stores it once on
    /// `<col>`, and so do we. Without this, formatting a column selection would
    /// materialize the entire sheet.
    AxisStyles {
        sheet: usize,
        axis: Axis,
        entries: Vec<(u32, Option<StyleId>)>,
    },
    /// A whole sheet put into the workbook's list at `index`.
    ///
    /// Boxed because a `Sheet` is the largest thing in this enum by a wide
    /// margin, and every other patch would otherwise pay for it.
    SheetInsert {
        index: usize,
        sheet: Box<ss_model::Sheet>,
    },
    /// A sheet taken out. Its inverse is a `SheetInsert` carrying it, which is
    /// why deleting a sheet is the one undo entry that costs what the sheet
    /// costs — there is nowhere smaller to keep a sheet than in a sheet.
    SheetRemove {
        index: usize,
    },
    SheetName {
        index: usize,
        name: String,
    },
    /// A tab dragged from one position to another.
    SheetMove {
        from: usize,
        to: usize,
    },
    SheetHidden {
        index: usize,
        hidden: bool,
    },
    TabColor {
        index: usize,
        color: Option<ss_model::Color>,
    },
    /// The workbook's defined names, replaced wholesale.
    ///
    /// Wholesale because a sheet-scoped name carries an *index* into the sheet
    /// list, so inserting or removing a sheet re-points every name after it.
    /// There are tens of these, not millions.
    DefinedNames {
        names: Vec<ss_model::DefinedName>,
    },
    /// A sheet's autofilter — the rule, not the hidden rows it produces.
    /// Every note on a sheet, replaced wholesale — the same shape as pictures
    /// and for the same reason: a sheet holds a handful, and adding, editing
    /// and deleting one then share an inverse, which is the list as it was.
    Comments {
        sheet: usize,
        comments: Vec<ss_model::Comment>,
    },
    /// A sheet protected, unprotected, or protected differently.
    Protection {
        sheet: usize,
        protection: Option<ss_model::Protection>,
    },
    Filter {
        sheet: usize,
        filter: Option<ss_model::AutoFilter>,
    },
}

/// One user-visible action, as a list of patches applied in order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Change {
    pub label: String,
    pub patches: Vec<Patch>,
}

impl Change {
    pub fn new(label: impl Into<String>, patches: Vec<Patch>) -> Self {
        Change {
            label: label.into(),
            patches,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }
}

/// Applies `change` and returns the change that undoes it.
pub fn apply(book: &mut Workbook, change: Change) -> Change {
    let mut inverse: Vec<Patch> = Vec::new();
    for patch in change.patches {
        let undo = apply_patch(book, patch);
        // Each patch's undo goes in front of everything already collected, so
        // undo runs the patches backwards while keeping each group's own order.
        inverse.splice(0..0, undo);
    }
    Change {
        label: change.label,
        patches: inverse,
    }
}

fn apply_patch(book: &mut Workbook, patch: Patch) -> Vec<Patch> {
    match patch {
        Patch::Cells { sheet, cells } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let mut before = Vec::with_capacity(cells.len());
            for (at, cell) in cells {
                before.push((at, target.get(at).copied()));
                target.set(at, cell.unwrap_or_default());
            }
            vec![Patch::Cells {
                sheet,
                cells: before,
            }]
        }

        Patch::Permute {
            sheet,
            first,
            cols,
            order,
        } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            // Where each row came from, read backwards: the row that came from
            // `order[i]` has to go back to `order[i]`, so the inverse asks for
            // whatever is now at `first + i` when it is that row's turn.
            //
            // Built before anything moves, because it is also the check that
            // this *is* a permutation. A list that names a row twice, or one
            // outside the band, has no inverse, and applying it would scatter
            // cells that undo could never gather up again.
            let mut back = vec![u32::MAX; order.len()];
            for (i, &from) in order.iter().enumerate() {
                let Some(slot) = from
                    .checked_sub(first)
                    .and_then(|k| back.get_mut(k as usize))
                    .filter(|slot| **slot == u32::MAX)
                else {
                    return Vec::new();
                };
                *slot = first + i as u32;
            }
            target.cells.permute_rows(first, cols, &order);
            vec![Patch::Permute {
                sheet,
                first,
                cols,
                order: back,
            }]
        }

        Patch::ChartTitle {
            sheet,
            chart,
            title,
        } => {
            let Some(target) = book.sheet_mut(sheet).and_then(|s| s.charts.get_mut(chart)) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.plot.title, title);
            vec![Patch::ChartTitle {
                sheet,
                chart,
                title: before,
            }]
        }

        Patch::Pictures { sheet, pictures } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.pictures, pictures);
            vec![Patch::Pictures {
                sheet,
                pictures: before,
            }]
        }

        Patch::Charts { sheet, charts } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.charts, charts);
            vec![Patch::Charts {
                sheet,
                charts: before,
            }]
        }

        Patch::Validations { sheet, validations } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.validations, validations);
            vec![Patch::Validations {
                sheet,
                validations: before,
            }]
        }

        Patch::ConditionalFormats { sheet, formats } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.conditional_formats, formats);
            vec![Patch::ConditionalFormats {
                sheet,
                formats: before,
            }]
        }

        Patch::AxisStyles {
            sheet,
            axis,
            entries,
        } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let styles = match axis {
                Axis::Rows => &mut target.row_styles,
                Axis::Columns => &mut target.column_styles,
            };
            let mut before = Vec::with_capacity(entries.len());
            for (index, style) in entries {
                before.push((index, styles.get(&index).copied()));
                match style {
                    Some(style) => styles.insert(index, style),
                    None => styles.remove(&index),
                };
            }
            vec![Patch::AxisStyles {
                sheet,
                axis,
                entries: before,
            }]
        }

        Patch::Formulas { sheet, texts } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let mut before = Vec::with_capacity(texts.len());
            for (id, text) in texts {
                let Some(formula) = target.formulas.get_mut(id.index() as usize) else {
                    continue;
                };
                before.push((id, std::mem::replace(&mut formula.text, text)));
            }
            vec![Patch::Formulas {
                sheet,
                texts: before,
            }]
        }

        Patch::Geometry { sheet, geometry } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            vec![Patch::Geometry {
                sheet,
                geometry: geometry.restore(target),
            }]
        }

        Patch::SheetInsert { index, sheet } => {
            let index = index.min(book.sheets.len());
            book.sheets.insert(index, *sheet);
            vec![Patch::SheetRemove { index }]
        }

        Patch::SheetRemove { index } => {
            if index >= book.sheets.len() {
                return Vec::new();
            }
            let removed = book.sheets.remove(index);
            // The active sheet is an index, and the sheet it named may have
            // just gone or moved down one.
            book.active_sheet = book.active_sheet.min(book.sheets.len().saturating_sub(1));
            vec![Patch::SheetInsert {
                index,
                sheet: Box::new(removed),
            }]
        }

        Patch::SheetName { index, name } => {
            let Some(target) = book.sheet_mut(index) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.name, name);
            vec![Patch::SheetName {
                index,
                name: before,
            }]
        }

        Patch::SheetMove { from, to } => {
            if from >= book.sheets.len() || to >= book.sheets.len() {
                return Vec::new();
            }
            let sheet = book.sheets.remove(from);
            book.sheets.insert(to, sheet);
            vec![Patch::SheetMove { from: to, to: from }]
        }

        Patch::SheetHidden { index, hidden } => {
            let Some(target) = book.sheet_mut(index) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.hidden, hidden);
            vec![Patch::SheetHidden {
                index,
                hidden: before,
            }]
        }

        Patch::TabColor { index, color } => {
            let Some(target) = book.sheet_mut(index) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.view.tab_color, color);
            vec![Patch::TabColor {
                index,
                color: before,
            }]
        }

        Patch::DefinedNames { names } => {
            let before = std::mem::replace(&mut book.defined_names, names);
            vec![Patch::DefinedNames { names: before }]
        }

        Patch::Comments { sheet, comments } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.comments, comments);
            vec![Patch::Comments {
                sheet,
                comments: before,
            }]
        }

        Patch::Protection { sheet, protection } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.protection, protection);
            vec![Patch::Protection {
                sheet,
                protection: before,
            }]
        }

        Patch::Filter { sheet, filter } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let before = std::mem::replace(&mut target.filter, filter);
            vec![Patch::Filter {
                sheet,
                filter: before,
            }]
        }

        Patch::Rearrange { sheet, rearrange } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            target.rearrange(rearrange);
            // The band is somewhere else now, so the undo picks it up from
            // where it landed and puts it back where it started.
            let (first, last) = rearrange.landing();
            let before = if rearrange.before > rearrange.last {
                rearrange.first
            } else {
                // Moving left: the band ends up in front of what used to
                // follow it, which has itself moved along by the band's width.
                rearrange.last + 1
            };
            vec![Patch::Rearrange {
                sheet,
                rearrange: ss_model::Move::new(rearrange.axis, first, last, before),
            }]
        }

        Patch::Shift { sheet, shift } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let geometry = Geometry::of(target);
            let destroyed = target.shift(shift);
            // Undoing: move the grid back, put the destroyed cells where they
            // were, then restore the geometry exactly rather than trusting the
            // reverse shift to have reconstructed it.
            vec![
                Patch::Shift {
                    sheet,
                    shift: shift.inverse(),
                },
                Patch::Cells {
                    sheet,
                    cells: destroyed
                        .into_iter()
                        .map(|(at, cell)| (at, Some(cell)))
                        .collect(),
                },
                Patch::Geometry { sheet, geometry },
            ]
        }
    }
}

/// Inserting or deleting rows or columns, with every formula in the workbook
/// rewritten to keep meaning what it meant.
pub fn structural(book: &Workbook, sheet: usize, shift: Shift) -> Change {
    let Some(moved) = book.sheet(sheet) else {
        return Change::default();
    };
    let mut patches = vec![Patch::Shift { sheet, shift }];

    // Every sheet, not just the one that moved: a formula on Sheet2 can name
    // Sheet1!A1, and it is just as wrong afterwards.
    for (index, other) in book.sheets.iter().enumerate() {
        let texts: Vec<(FormulaId, String)> = other
            .formulas
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.text.is_empty())
            .filter_map(|(i, f)| {
                let rewritten = translate(&f.text, &other.name, &moved.name, shift)?;
                Some((FormulaId::from_index(i as u32), rewritten))
            })
            .collect();
        if !texts.is_empty() {
            patches.push(Patch::Formulas {
                sheet: index,
                texts,
            });
        }
    }

    Change::new(label_for(shift), patches)
}

/// A band of rows or columns moved, with every formula in the workbook
/// rewritten to keep meaning what it meant.
///
/// The counterpart of [`structural`], and it differs in what it cannot do:
/// nothing is destroyed, so no reference can end up `#REF!` and no cells have
/// to be carried in the undo. What a formula says changes; what it means does
/// not, which is the whole promise of moving a column rather than retyping it.
pub fn move_band(book: &Workbook, sheet: usize, mv: ss_model::Move) -> Change {
    let Some(moved) = book.sheet(sheet) else {
        return Change::default();
    };
    if mv.is_noop() {
        return Change::default();
    }
    let mut patches = vec![Patch::Rearrange {
        sheet,
        rearrange: mv,
    }];

    // Every sheet, for the same reason a shift touches every sheet: a formula
    // on Sheet2 can name Sheet1!A1, and A1 is somewhere else now.
    for (index, other) in book.sheets.iter().enumerate() {
        let texts: Vec<(FormulaId, String)> = other
            .formulas
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.text.is_empty())
            .filter_map(|(i, f)| {
                let rewritten = crate::translate::move_band(&f.text, &other.name, &moved.name, mv)?;
                Some((FormulaId::from_index(i as u32), rewritten))
            })
            .collect();
        if !texts.is_empty() {
            patches.push(Patch::Formulas {
                sheet: index,
                texts,
            });
        }
    }

    let label = match mv.axis {
        Axis::Rows => "Move rows",
        Axis::Columns => "Move columns",
    };
    Change::new(label, patches)
}

fn label_for(shift: Shift) -> &'static str {
    match (shift.axis, shift.delete) {
        (Axis::Rows, false) => "Insert rows",
        (Axis::Rows, true) => "Delete rows",
        (Axis::Columns, false) => "Insert columns",
        (Axis::Columns, true) => "Delete columns",
    }
}

/// Emptying cells, keeping their style — Excel's Delete key.
pub fn clear_contents(book: &Workbook, sheet: usize, ranges: &[CellRange]) -> Change {
    let Some(target) = book.sheet(sheet) else {
        return Change::default();
    };
    // Only cells that exist can be cleared, so a range is worth no more than
    // its overlap with the data. Without the trim, Delete over a whole-sheet
    // selection asks about seventeen billion cells one at a time.
    let Some(used) = target.used_range() else {
        return Change::default();
    };
    let mut cells = Vec::new();
    for range in ranges.iter().filter_map(|r| r.intersect(used)) {
        for row in range.start.row..=range.end.row {
            for col in range.start.col..=range.end.col {
                let at = CellRef::new(row, col);
                let Some(existing) = target.get(at) else {
                    continue;
                };
                cells.push((
                    at,
                    Some(Cell {
                        value: CellValue::Blank,
                        style: existing.style,
                        formula: None,
                    }),
                ));
            }
        }
    }
    Change::new("Clear", vec![Patch::Cells { sheet, cells }])
}

/// What the user typed, turned into what the cell should hold.
///
/// Takes the workbook mutably because a formula needs an arena slot and text
/// needs interning, both of which only ever grow. The cell write itself is in
/// the returned change, so it is still undo that decides when it happens.
pub fn input(book: &mut Workbook, sheet: usize, at: CellRef, typed: &str) -> Change {
    let style = book
        .sheet(sheet)
        .and_then(|s| s.get(at))
        .map_or(StyleId::DEFAULT, |c| c.style);
    let cell = typed_cell(book, sheet, style, typed);
    Change::new(
        "Edit cell",
        vec![Patch::Cells {
            sheet,
            cells: vec![(at, Some(cell))],
        }],
    )
}

/// A block of cells picked up and put down with its top-left corner at `to`
/// — the drag of a selection border, and the honest half of cut-and-paste.
///
/// A move is not a copy, and the difference is everything here. The moved
/// cells keep their formulas *as written* — their references were not asked
/// to change, only carried (references pointing into the moved block travel
/// with it, so `=B1+C1` moved wholesale still adds the same two numbers).
/// And every formula anywhere in the workbook that pointed into the block
/// follows the data to its new address, dollars notwithstanding: `$` guards
/// against copies, not against the data itself being taken elsewhere.
///
/// Merges wholly inside the block move with it. A destination that would
/// fall off the sheet refuses by returning an empty change.
pub fn move_range(book: &mut Workbook, sheet: usize, from: CellRange, to: CellRef) -> Change {
    let rows = i64::from(to.row) - i64::from(from.start.row);
    let cols = i64::from(to.col) - i64::from(from.start.col);
    if rows == 0 && cols == 0 {
        return Change::default();
    }
    let end_row = i64::from(from.end.row) + rows;
    let end_col = i64::from(from.end.col) + cols;
    if end_row >= i64::from(ss_model::cell::MAX_ROWS)
        || end_col >= i64::from(ss_model::cell::MAX_COLS)
    {
        return Change::default();
    }
    let Some(moved_name) = book.sheet(sheet).map(|s| s.name.clone()) else {
        return Change::default();
    };

    // The final state of every touched position, keyed so that a source cell
    // that is also a destination appears exactly once — a position written
    // twice in one patch would leave the undo restoring the wrong layer.
    let mut outcome: std::collections::BTreeMap<CellRef, Option<Cell>> =
        std::collections::BTreeMap::new();
    if let Some(s) = book.sheet(sheet) {
        for row in from.start.row..=from.end.row {
            for col in from.start.col..=from.end.col {
                let at = CellRef::new(row, col);
                outcome.entry(at).or_insert(None);
                let landing = CellRef::new(
                    (i64::from(row) + rows) as u32,
                    (i64::from(col) + cols) as u32,
                );
                outcome.insert(landing, s.get(at).cloned());
            }
        }
    }
    let mut patches = vec![Patch::Cells {
        sheet,
        cells: outcome.into_iter().collect(),
    }];

    // Merges wholly inside the block travel with it.
    if let Some(s) = book.sheet(sheet) {
        let inside = |m: &CellRange| {
            m.start.row >= from.start.row
                && m.end.row <= from.end.row
                && m.start.col >= from.start.col
                && m.end.col <= from.end.col
        };
        if s.merges.iter().any(inside) {
            let mut geometry = Geometry::of(s);
            for merge in &mut geometry.merges {
                if inside(merge) {
                    merge.start.row = (i64::from(merge.start.row) + rows) as u32;
                    merge.start.col = (i64::from(merge.start.col) + cols) as u32;
                    merge.end.row = (i64::from(merge.end.row) + rows) as u32;
                    merge.end.col = (i64::from(merge.end.col) + cols) as u32;
                }
            }
            patches.push(Patch::Geometry { sheet, geometry });
        }
    }

    // Every formula in the workbook follows the data it pointed at.
    for (index, other) in book.sheets.iter().enumerate() {
        let texts: Vec<(FormulaId, String)> = other
            .formulas
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.text.is_empty())
            .filter_map(|(i, f)| {
                let moved = crate::translate::follow_move(
                    &f.text,
                    &other.name,
                    &moved_name,
                    from,
                    rows,
                    cols,
                )?;
                Some((FormulaId::from_index(i as u32), moved))
            })
            .collect();
        if !texts.is_empty() {
            patches.push(Patch::Formulas {
                sheet: index,
                texts,
            });
        }
    }

    Change::new("Move cells", patches)
}

/// Ctrl+Enter: one entry, written to every target at once.
///
/// Each cell receives the text as if it had been typed there: a formula's
/// relative references travel by the cell's distance from `at`, exactly as a
/// copy would carry them, and `$` pins what it always pins. One change, one
/// undo.
pub fn input_many(
    book: &mut Workbook,
    sheet: usize,
    at: CellRef,
    targets: &[CellRef],
    typed: &str,
) -> Change {
    let mut cells = Vec::with_capacity(targets.len());
    for &target in targets {
        let text = match typed.strip_prefix('=') {
            Some(body) => {
                let rows = i64::from(target.row) - i64::from(at.row);
                let cols = i64::from(target.col) - i64::from(at.col);
                match crate::translate::offset(body, rows, cols) {
                    Some(moved) => format!("={moved}"),
                    None => typed.to_string(),
                }
            }
            None => typed.to_string(),
        };
        let style = book
            .sheet(sheet)
            .and_then(|s| s.get(target))
            .map_or(StyleId::DEFAULT, |c| c.style);
        let cell = typed_cell(book, sheet, style, &text);
        cells.push((target, Some(cell)));
    }
    Change::new("Fill selection", vec![Patch::Cells { sheet, cells }])
}

/// The cell that `typed` produces, keeping `style` unless the input calls for
/// one of its own.
///
/// Shared with paste, because text arriving from another program has to be read
/// exactly the way text arriving from the keyboard is.
/// What typing `typed` into a cell of `style` produces, ready to store.
///
/// Public because importing a csv is the same interpretation as typing: `5` is
/// a number, `2024-01-15` is a date with a date format, `=A1` is a formula, and
/// `007` is text. An importer that skipped this would turn every column into
/// strings.
pub fn typed_cell(book: &mut Workbook, sheet: usize, style: StyleId, typed: &str) -> Cell {
    match classify(typed) {
        Typed::Empty => Cell {
            value: CellValue::Blank,
            style,
            formula: None,
        },
        Typed::Formula(text) => {
            let id = book
                .sheet_mut(sheet)
                .map(|s| s.push_formula(Formula::normal(text)));
            Cell {
                // A placeholder until recalculation runs.
                value: CellValue::Blank,
                style,
                formula: id,
            }
        }
        Typed::Number(n, format) => Cell {
            value: CellValue::Number(n),
            // A typed date is a number plus the format that makes it read as a
            // date. Without the second half the user sees 45306.
            style: match format {
                Some(code) if style == StyleId::DEFAULT => book.styles.style_for_format(&code),
                _ => style,
            },
            formula: None,
        },
        Typed::Bool(b) => Cell {
            value: CellValue::Bool(b),
            style,
            formula: None,
        },
        Typed::Error(e) => Cell {
            value: CellValue::Error(e),
            style,
            formula: None,
        },
        Typed::Text(text) => {
            let id = book.strings.intern(&text);
            Cell {
                value: CellValue::Text(id),
                style,
                formula: None,
            }
        }
    }
}

enum Typed {
    Empty,
    Formula(String),
    /// A number, and the format code it should be displayed through if the cell
    /// does not already have one of its own.
    ///
    /// Owned rather than `&'static str` because a typed percentage carries as
    /// many decimal places as the user typed, and there is no fixed set of them.
    Number(f64, Option<Cow<'static, str>>),
    Bool(bool),
    Error(ss_model::CellError),
    Text(String),
}

/// Excel's order of interpretation for typed input.
///
/// The order is the whole content of this function. `TRUE` is a boolean before
/// it is text; `1/2/2024` is a date before it is a division; `+5` is a number
/// even though `+` starts a formula in some spreadsheets.
fn classify(typed: &str) -> Typed {
    let trimmed = typed.trim();
    if trimmed.is_empty() {
        return Typed::Empty;
    }
    if let Some(rest) = trimmed.strip_prefix('=') {
        return Typed::Formula(rest.to_string());
    }
    // A leading apostrophe forces text, and is not part of the value.
    if let Some(rest) = typed.strip_prefix('\'') {
        return Typed::Text(rest.to_string());
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Typed::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Typed::Bool(false);
    }
    if let Some(error) = ss_model::CellError::from_code(trimmed) {
        return Typed::Error(error);
    }
    if let Some(number) = parse_number(trimmed) {
        return number;
    }
    if let Some((serial, format)) = crate::functions::date::typed_datetime(trimmed) {
        return Typed::Number(serial, Some(Cow::Borrowed(format)));
    }
    Typed::Text(typed.to_string())
}

/// Plain numbers, percentages, and grouped thousands.
fn parse_number(text: &str) -> Option<Typed> {
    if let Some(body) = text.strip_suffix('%') {
        let body = body.trim();
        let value: f64 = degroup(body)?.parse().ok()?;
        // The format has to keep the digits that were typed. `42.5%` under `0%`
        // displays as `43%`, which is the cell disagreeing with what the user
        // just put in it.
        let decimals = body.split_once('.').map_or(0, |(_, d)| d.len());
        let format = match decimals {
            0 => "0%".to_string(),
            n => format!("0.{}%", "0".repeat(n)),
        };
        return Some(Typed::Number(value / 100.0, Some(Cow::Owned(format))));
    }
    let value: f64 = degroup(text)?.parse().ok()?;
    // `#,##0` here would be a guess about the user's intent; Excel does apply a
    // grouped format when the typing was grouped, and so do we.
    let grouped = text.contains(',');
    Some(Typed::Number(
        value,
        grouped.then(|| {
            Cow::Borrowed(if text.contains('.') {
                "#,##0.00"
            } else {
                "#,##0"
            })
        }),
    ))
}

/// Removes thousands separators, refusing anything that is not a number shape.
///
/// `f64::from_str` would accept `inf` and `NaN`, which are not values a
/// spreadsheet cell can hold, so the character set is checked first.
fn degroup(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    if !text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b',' | b'-' | b'+' | b'e' | b'E'))
    {
        return None;
    }
    Some(text.replace(',', ""))
}

/// The most cells a formatting command will materialize.
///
/// A rectangle bigger than this is almost certainly a mis-drag, and writing a
/// styled cell for each one would turn a sparse sheet into a dense one.
pub const MAX_FORMAT_CELLS: usize = 1 << 18;

/// Applies a formatting change to a selection.
///
/// Three shapes, because Excel stores them three ways and so must we: a whole
/// column goes on `<col>`, a whole row on `<row>`, and anything else on the
/// cells themselves. Formatting a column selection as cells would materialize a
/// million of them — the file has one attribute for exactly this reason.
///
/// The style table is append-only, so nothing here mutates a style: each target
/// gets the id of a style that *has* the new look, which may be one that
/// already existed.
pub fn format(
    book: &mut Workbook,
    sheet: usize,
    ranges: &[CellRange],
    label: impl Into<String>,
    edit: impl Fn(&mut ss_model::Look),
) -> Change {
    let Some(model) = book.sheet(sheet) else {
        return Change::default();
    };

    // Everything is read before anything is changed: `restyle` borrows the
    // style table mutably and the sheet is where the current styles come from.
    let mut cells: Vec<(CellRef, StyleId)> = Vec::new();
    let mut columns: Vec<(u32, StyleId)> = Vec::new();
    let mut rows: Vec<(u32, StyleId)> = Vec::new();
    for range in ranges {
        if range.rows() == ss_model::cell::MAX_ROWS {
            for col in range.start.col..=range.end.col {
                columns.push((
                    col,
                    model.column_styles.get(&col).copied().unwrap_or_default(),
                ));
            }
        } else if range.cols() == ss_model::cell::MAX_COLS {
            for row in range.start.row..=range.end.row {
                rows.push((row, model.row_styles.get(&row).copied().unwrap_or_default()));
            }
        } else {
            for row in range.start.row..=range.end.row {
                for col in range.start.col..=range.end.col {
                    if cells.len() >= MAX_FORMAT_CELLS {
                        break;
                    }
                    let at = CellRef::new(row, col);
                    cells.push((at, model.style_at(at)));
                }
            }
        }
    }

    let mut patches = Vec::new();
    if !cells.is_empty() {
        let written: Vec<(CellRef, Option<Cell>)> = cells
            .into_iter()
            .map(|(at, style)| {
                let style = book.styles.restyle(style, &edit);
                let existing = book.sheet(sheet).and_then(|s| s.get(at)).copied();
                let cell = Cell {
                    style,
                    ..existing.unwrap_or_default()
                };
                (at, Some(cell))
            })
            .collect();
        patches.push(Patch::Cells {
            sheet,
            cells: written,
        });
    }
    for (axis, targets) in [(Axis::Columns, columns), (Axis::Rows, rows)] {
        if targets.is_empty() {
            continue;
        }
        let entries = targets
            .into_iter()
            .map(|(index, style)| {
                let style = book.styles.restyle(style, &edit);
                // The workbook default is the absence of a style, not a style
                // of zero: writing `s="0"` on every column would be noise.
                (index, (style != StyleId::DEFAULT).then_some(style))
            })
            .collect();
        patches.push(Patch::AxisStyles {
            sheet,
            axis,
            entries,
        });
    }

    Change::new(label, patches)
}

/// What typing `text` into a cell would store, without storing it.
///
/// Data validation is a rule about the *value*, not about the characters: a
/// "whole number between 1 and 10" rule has to see 5 rather than "5", and a
/// date rule has to see the serial. So the same interpretation the editor uses
/// runs first, and only then does the rule get asked.
///
/// A formula is not interpreted here — it would have to be evaluated, and a
/// validation that refused a formula because it could not read it would be
/// worse than one that let it through.
pub fn typed_value(text: &str) -> Value {
    match classify(text) {
        Typed::Empty => Value::Blank,
        // Not interpreted: it would have to be evaluated, and a validation that
        // refused a formula because it could not read it would be worse than
        // one that let it through.
        Typed::Formula(_) => Value::Blank,
        Typed::Number(n, _) => Value::Number(n),
        Typed::Bool(b) => Value::Bool(b),
        Typed::Error(e) => Value::Error(e),
        Typed::Text(t) => Value::Text(t),
    }
}

/// Retitles a chart, or clears its title when `title` is empty.
pub fn chart_title(sheet: usize, chart: usize, title: &str) -> Change {
    let trimmed = title.trim();
    Change::new(
        "Chart title",
        vec![Patch::ChartTitle {
            sheet,
            chart,
            title: (!trimmed.is_empty()).then(|| trimmed.to_string()),
        }],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(a1: &str) -> CellRef {
        CellRef::from_a1(a1).expect("valid address")
    }

    #[test]
    fn a_move_carries_formulas_as_written_and_references_follow() {
        let mut book = Workbook::blank();
        book.sheets[0].set(
            at("B1"),
            Cell {
                value: CellValue::Number(3.0),
                ..Default::default()
            },
        );
        let inside = book.sheets[0].push_formula(Formula::normal("B1*2"));
        book.sheets[0].set(
            at("B2"),
            Cell {
                value: CellValue::Number(6.0),
                style: StyleId::DEFAULT,
                formula: Some(inside),
            },
        );
        let watcher = book.sheets[0].push_formula(Formula::normal("$B$1+B2"));
        book.sheets[0].set(
            at("D1"),
            Cell {
                value: CellValue::Number(9.0),
                style: StyleId::DEFAULT,
                formula: Some(watcher),
            },
        );

        let from = CellRange::new(at("B1"), at("B2"));
        let change = move_range(&mut book, 0, from, at("C5"));
        let undo = apply(&mut book, change);

        let text =
            |book: &Workbook, a: &str| book.sheets[0].formula_at(at(a)).map(|f| f.text.clone());
        assert!(book.sheets[0].get(at("B1")).is_none_or(|c| c.is_vacant()));
        assert_eq!(
            book.sheets[0].get(at("C5")).map(|c| c.value),
            Some(CellValue::Number(3.0))
        );
        assert_eq!(
            text(&book, "C6").as_deref(),
            Some("C5*2"),
            "a reference inside the block travelled with it"
        );
        assert_eq!(
            text(&book, "D1").as_deref(),
            Some("$C$5+C6"),
            "watchers follow the data, dollars notwithstanding"
        );

        apply(&mut book, undo);
        assert_eq!(
            book.sheets[0].get(at("B1")).map(|c| c.value),
            Some(CellValue::Number(3.0))
        );
        assert_eq!(
            text(&book, "D1").as_deref(),
            Some("$B$1+B2"),
            "undo restores the text"
        );
        assert_eq!(text(&book, "B2").as_deref(), Some("B1*2"));
    }

    #[test]
    fn one_entry_fills_many_cells_with_travelling_references() {
        let mut book = Workbook::blank();
        let targets = [at("B2"), at("B3"), at("C4")];
        let change = input_many(&mut book, 0, at("B2"), &targets, "=A1*2");
        assert_eq!(change.label, "Fill selection");
        apply(&mut book, change);
        let formula = |a: &str| {
            let sheet = &book.sheets[0];
            sheet.formula_at(at(a)).expect("a formula").text.clone()
        };
        assert_eq!(formula("B2"), "A1*2", "home cell keeps the entry as typed");
        assert_eq!(formula("B3"), "A2*2", "one row down, references follow");
        assert_eq!(formula("C4"), "B3*2", "and across");
        // Plain values are simply repeated.
        let change = input_many(&mut book, 0, at("D1"), &[at("D1"), at("D2")], "7");
        apply(&mut book, change);
        assert_eq!(
            book.sheets[0].get(at("D2")).map(|c| c.value),
            Some(CellValue::Number(7.0))
        );
    }

    fn a_picture(col: u32) -> ss_model::Picture {
        use ss_model::chart::{Anchor, AnchorPoint};
        ss_model::Picture {
            part: "/xl/media/image1.png".into(),
            drawing_part: "/xl/drawings/drawing1.xml".into(),
            anchor_index: 0,
            name: "Picture 3".into(),
            anchor: Anchor::OneCell {
                from: AnchorPoint {
                    col,
                    col_offset: 0,
                    row: 0,
                    row_offset: 0,
                },
                width: 100,
                height: 100,
            },
            data: std::sync::Arc::from(Vec::new().into_boxed_slice()),
            content_type: "image/png".into(),
        }
    }

    /// A sheet whose first row spells out which column each cell is in, and
    /// whose second row is a formula pointing at the cell above it.
    fn lettered() -> Workbook {
        let mut book = Workbook::blank();
        for col in 0..5u32 {
            let letter = ss_model::column_name(col);
            let change = input(&mut book, 0, CellRef::new(0, col), &letter);
            apply(&mut book, change);
            let change = input(&mut book, 0, CellRef::new(1, col), &format!("={letter}1"));
            apply(&mut book, change);
        }
        book
    }

    fn shown(book: &Workbook, a1: &str) -> String {
        match book.sheets[0].get(at(a1)).map(|c| c.value) {
            Some(CellValue::Text(id)) => book.strings.resolve(id).to_string(),
            Some(CellValue::Number(n)) => ss_model::format_general(n),
            _ => String::new(),
        }
    }

    fn formula(book: &Workbook, a1: &str) -> String {
        book.sheets[0]
            .formula_at(at(a1))
            .map(|f| f.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn a_moved_column_takes_its_cells_and_its_formulas_with_it() {
        let mut book = lettered();
        // B moved to the far side of D: A C D B E.
        let change = move_band(&book, 0, ss_model::Move::new(Axis::Columns, 1, 1, 4));
        let undo = apply(&mut book, change);

        assert_eq!(
            (0..5)
                .map(|c| shown(&book, &format!("{}1", ss_model::column_name(c))))
                .collect::<Vec<_>>(),
            ["A", "C", "D", "B", "E"]
        );
        // Each formula still points at the cell above it, wherever that went.
        for col in 0..5u32 {
            let letter = ss_model::column_name(col);
            assert_eq!(
                formula(&book, &format!("{letter}2")),
                format!("{letter}1"),
                "the formula under {letter}1"
            );
        }

        apply(&mut book, undo);
        assert_eq!(
            (0..5)
                .map(|c| shown(&book, &format!("{}1", ss_model::column_name(c))))
                .collect::<Vec<_>>(),
            ["A", "B", "C", "D", "E"],
            "and back"
        );
        assert_eq!(formula(&book, "B2"), "B1");
    }

    #[test]
    fn a_range_over_a_moved_column_still_covers_the_same_cells() {
        // The trap: mapping the two corners says A1:C1 and quietly drops a
        // column from the sum. The range still spans the same four.
        let mut book = lettered();
        let change = input(&mut book, 0, at("A4"), "=SUM(A1:D1)");
        apply(&mut book, change);
        let change = input(&mut book, 0, at("A5"), "=B1");
        apply(&mut book, change);

        let change = move_band(&book, 0, ss_model::Move::new(Axis::Columns, 1, 1, 4));
        apply(&mut book, change);
        assert_eq!(formula(&book, "A4"), "SUM(A1:D1)");
        assert_eq!(formula(&book, "A5"), "D1", "B is at D now");
    }

    #[test]
    fn moving_rows_carries_their_heights_and_leaves_the_freeze_alone() {
        let mut book = Workbook::blank();
        {
            let sheet = &mut book.sheets[0];
            sheet.row_heights.insert(1, 40.0);
            sheet.panes = Some(ss_model::Panes::frozen(CellRef::new(2, 0)));
        }
        let change = move_band(&book, 0, ss_model::Move::new(Axis::Rows, 1, 1, 5));
        apply(&mut book, change);
        let sheet = &book.sheets[0];
        assert_eq!(
            sheet.row_heights.get(&4),
            Some(&40.0),
            "the height went too"
        );
        assert_eq!(sheet.row_heights.get(&1), None);
        assert_eq!(
            sheet.panes,
            Some(ss_model::Panes::frozen(CellRef::new(2, 0))),
            "a freeze is a line on the sheet, not a property of the rows beside it"
        );
    }

    #[test]
    fn a_move_that_lands_where_it_started_is_not_a_change_at_all() {
        let book = lettered();
        for before in 1..=2u32 {
            let change = move_band(&book, 0, ss_model::Move::new(Axis::Columns, 1, 1, before));
            assert!(change.patches.is_empty(), "before {before}");
        }
    }

    #[test]
    fn moving_and_deleting_a_picture_undo_the_same_way() {
        let mut book = Workbook::blank();
        book.sheets[0].pictures = vec![a_picture(1)];

        // Moved.
        let moved = Change::new(
            "Move picture",
            vec![Patch::Pictures {
                sheet: 0,
                pictures: book.sheets[0].pictures.clone(),
            }],
        );
        book.sheets[0].pictures = vec![a_picture(7)];
        let redo = apply(&mut book, moved);
        assert_eq!(book.sheets[0].pictures, vec![a_picture(1)], "undone");
        apply(&mut book, redo);
        assert_eq!(book.sheets[0].pictures, vec![a_picture(7)], "redone");

        // Deleted. The same patch, because the list is what is replaced.
        let deleted = Change::new(
            "Delete picture",
            vec![Patch::Pictures {
                sheet: 0,
                pictures: book.sheets[0].pictures.clone(),
            }],
        );
        book.sheets[0].pictures.clear();
        apply(&mut book, deleted);
        assert_eq!(book.sheets[0].pictures.len(), 1, "it came back");
    }

    #[test]
    fn formatting_a_selection_is_undoable_like_anything_else() {
        let mut book = Workbook::blank();
        let change = input(&mut book, 0, at("A1"), "7");
        apply(&mut book, change);

        let range = CellRange::new(at("A1"), at("B2"));
        let bold = format(&mut book, 0, &[range], "Bold", |look| look.font.bold = true);
        let undo = apply(&mut book, bold);

        let style = |book: &Workbook, a1: &str| book.sheets[0].style_at(at(a1));
        assert!(book.styles.font(style(&book, "A1")).bold);
        assert!(
            book.styles.font(style(&book, "B2")).bold,
            "an empty cell in the selection is formatted too"
        );
        assert_eq!(
            book.sheets[0].get(at("A1")).map(|c| c.value),
            Some(CellValue::Number(7.0)),
            "the value came along unchanged"
        );

        apply(&mut book, undo);
        assert!(!book.styles.font(style(&book, "A1")).bold);
        assert!(!book.styles.font(style(&book, "B2")).bold);
    }

    #[test]
    fn formatting_a_whole_column_does_not_materialize_a_million_cells() {
        // Excel stores this once on `<col>`. Writing a styled cell per row would
        // turn a sparse sheet into a dense one and make the file enormous.
        let mut book = Workbook::blank();
        let whole = CellRange::new(
            CellRef::new(0, 2),
            CellRef::new(ss_model::cell::MAX_ROWS - 1, 2),
        );
        let change = format(&mut book, 0, &[whole], "Fill", |look| {
            look.fill = ss_model::Fill::solid(ss_model::Color::rgb(0xFF, 0xFF, 0))
        });
        let undo = apply(&mut book, change);

        assert!(book.sheets[0].cells.is_empty(), "no cells were created");
        let style = book.sheets[0].style_at(CellRef::new(500_000, 2));
        assert_ne!(style, StyleId::DEFAULT);
        assert!(!book.styles.fill(style).is_none());

        apply(&mut book, undo);
        assert_eq!(
            book.sheets[0].style_at(CellRef::new(500_000, 2)),
            StyleId::DEFAULT
        );
    }

    #[test]
    fn what_typing_would_store_is_decided_before_it_is_stored() {
        // Data validation is a rule about the value, so it has to see the
        // number rather than the characters.
        assert_eq!(typed_value("5"), Value::Number(5.0));
        assert_eq!(typed_value("50%"), Value::Number(0.5));
        assert_eq!(typed_value("TRUE"), Value::Bool(true));
        assert_eq!(typed_value(""), Value::Blank);
        assert_eq!(typed_value("north"), Value::Text("north".to_string()));
        assert_eq!(
            typed_value("=SUM(A1:A9)"),
            Value::Blank,
            "a formula is not judged by a rule that cannot evaluate it"
        );
    }

    use ss_model::{CellValue, Sheet};

    fn book() -> Workbook {
        let mut book = Workbook::blank();
        book.sheets.push(Sheet::new("Data"));
        book
    }

    fn number(book: &Workbook, sheet: usize, a1: &str) -> Option<f64> {
        let at = CellRef::from_a1(a1)?;
        match book.sheet(sheet)?.get(at)?.value {
            CellValue::Number(n) => Some(n),
            _ => None,
        }
    }

    fn text_of(book: &mut Workbook, sheet: usize, a1: &str, typed: &str) -> Change {
        let at = CellRef::from_a1(a1).expect("valid address");
        input(book, sheet, at, typed)
    }

    #[test]
    fn undo_puts_back_exactly_what_was_there() {
        let mut book = book();
        let change = text_of(&mut book, 0, "A1", "5");
        let undo = apply(&mut book, change);
        assert_eq!(number(&book, 0, "A1"), Some(5.0));

        let redo = apply(&mut book, undo);
        assert_eq!(book.sheet(0).unwrap().get(CellRef::new(0, 0)), None);

        apply(&mut book, redo);
        assert_eq!(
            number(&book, 0, "A1"),
            Some(5.0),
            "redo is the undo of undo"
        );
    }

    #[test]
    fn a_percentage_keeps_the_decimals_it_was_typed_with() {
        // Under a plain `0%` the cell would answer `43%` to someone who just
        // typed `42.5%` — the one number they are certain about.
        // A cell each: typing into one that already has a format keeps that
        // format, which is a different rule and is tested by its own name.
        let mut book = book();
        for (row, (typed, shown)) in [("42.5%", "42.5%"), ("50%", "50%"), ("1.25%", "1.25%")]
            .into_iter()
            .enumerate()
        {
            let cell = format!("A{}", row + 1);
            let change = text_of(&mut book, 0, &cell, typed);
            apply(&mut book, change);
            let at = CellRef::new(row as u32, 0);
            let style = book.sheet(0).unwrap().get(at).unwrap().style;
            let value = number(&book, 0, &cell).expect("a number");
            assert_eq!(
                book.styles
                    .number_format(style)
                    .format(ss_model::numfmt::FormatValue::Number(value))
                    .text,
                shown,
                "typing {typed}"
            );
        }
    }

    #[test]
    fn typing_is_read_the_way_excel_reads_it() {
        let mut book = book();
        for (typed, expected) in [("5", 5.0), ("-2.5", -2.5), ("50%", 0.5), ("1,234", 1234.0)] {
            let change = text_of(&mut book, 0, "A1", typed);
            apply(&mut book, change);
            assert_eq!(number(&book, 0, "A1"), Some(expected), "typing {typed}");
        }

        let change = text_of(&mut book, 0, "A1", "TRUE");
        apply(&mut book, change);
        assert_eq!(
            book.sheet(0)
                .unwrap()
                .get(CellRef::new(0, 0))
                .unwrap()
                .value,
            CellValue::Bool(true)
        );

        // A leading apostrophe is Excel's escape hatch, and is not stored.
        let change = text_of(&mut book, 0, "A1", "'5");
        apply(&mut book, change);
        let cell = book.sheet(0).unwrap().get(CellRef::new(0, 0)).unwrap();
        match cell.value {
            CellValue::Text(id) => assert_eq!(book.strings.resolve(id), "5"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_typed_date_gets_a_format_that_makes_it_read_as_one() {
        let mut book = book();
        let change = text_of(&mut book, 0, "A1", "2024-01-15");
        apply(&mut book, change);

        let cell = *book.sheet(0).unwrap().get(CellRef::new(0, 0)).unwrap();
        assert_eq!(cell.value, CellValue::Number(45306.0));
        assert_eq!(
            book.styles
                .number_format(cell.style)
                .format(ss_model::FormatValue::Number(45306.0))
                .text,
            "01-15-24",
            "the serial alone would show as 45306"
        );
    }

    #[test]
    fn inserting_rows_rewrites_formulas_on_every_sheet() {
        let mut book = book();
        let id = book.sheets[0].push_formula(Formula::normal("SUM(A1:A10)"));
        book.sheets[0].set(
            CellRef::new(20, 0),
            Cell {
                value: CellValue::Blank,
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );
        let elsewhere = book.sheets[1].push_formula(Formula::normal("Sheet1!A5*2"));
        book.sheets[1].set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Blank,
                style: StyleId::DEFAULT,
                formula: Some(elsewhere),
            },
        );

        let change = structural(&book, 0, Shift::insert(Axis::Rows, 0, 2));
        let undo = apply(&mut book, change);

        assert_eq!(book.sheets[0].formulas[0].text, "SUM(A3:A12)");
        assert_eq!(
            book.sheets[1].formulas[0].text, "Sheet1!A7*2",
            "a formula on another sheet pointed at the rows that moved"
        );

        apply(&mut book, undo);
        assert_eq!(book.sheets[0].formulas[0].text, "SUM(A1:A10)");
        assert_eq!(book.sheets[1].formulas[0].text, "Sheet1!A5*2");
    }

    #[test]
    fn undoing_a_delete_brings_the_data_back() {
        let mut book = book();
        for row in 0..5u32 {
            book.sheets[0].set(
                CellRef::new(row, 0),
                Cell {
                    value: CellValue::Number(f64::from(row)),
                    ..Default::default()
                },
            );
        }
        book.sheets[0]
            .merges
            .push(CellRange::new(CellRef::new(1, 0), CellRef::new(1, 3)));
        book.sheets[0].row_heights.insert(1, 40.0);

        let change = structural(&book, 0, Shift::delete(Axis::Rows, 1, 2));
        let undo = apply(&mut book, change);
        assert_eq!(number(&book, 0, "A2"), Some(3.0));
        assert!(book.sheets[0].merges.is_empty());

        apply(&mut book, undo);
        for row in 0..5u32 {
            assert_eq!(
                number(&book, 0, &format!("A{}", row + 1)),
                Some(f64::from(row)),
                "row {row} came back"
            );
        }
        assert_eq!(book.sheets[0].merges.len(), 1, "the merge came back too");
        assert_eq!(book.sheets[0].row_heights.get(&1), Some(&40.0));
    }

    #[test]
    fn clearing_keeps_the_formatting_and_removes_the_content() {
        let mut book = book();
        book.sheets[0].set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Number(7.0),
                style: StyleId(3),
                formula: None,
            },
        );
        let range = CellRange::new(CellRef::new(0, 0), CellRef::new(0, 0));
        let change = clear_contents(&book, 0, &[range]);
        apply(&mut book, change);

        let cell = book.sheets[0].get(CellRef::new(0, 0)).expect("still there");
        assert_eq!(cell.value, CellValue::Blank);
        assert_eq!(cell.style, StyleId(3), "Delete clears contents, not format");
    }

    #[test]
    fn deleting_over_the_whole_sheet_is_the_cells_that_exist() {
        // Delete after a Ctrl+A is an everyday thing to do, and asking every
        // address on the sheet whether it holds something is seventeen billion
        // questions with two answers in them.
        let mut book = book();
        for (row, col) in [(0, 0), (3, 2)] {
            book.sheets[0].set(
                CellRef::new(row, col),
                Cell {
                    value: CellValue::Number(1.0),
                    ..Default::default()
                },
            );
        }
        let everything = CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(ss_model::cell::MAX_ROWS - 1, ss_model::cell::MAX_COLS - 1),
        );
        let change = clear_contents(&book, 0, &[everything]);
        match change.patches.as_slice() {
            [Patch::Cells { cells, .. }] => assert_eq!(cells.len(), 2),
            other => panic!("expected two cells, got {other:?}"),
        }
        apply(&mut book, change);
        assert!(
            book.sheets[0]
                .get(CellRef::new(3, 2))
                .is_none_or(|c| c.value.is_blank()),
            "and the sheet came out empty"
        );
    }
}
