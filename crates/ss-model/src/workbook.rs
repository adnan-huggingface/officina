//! Sheets and workbooks.

use std::collections::BTreeMap;

use crate::cell::{Cell, CellRef, FormulaId};
use crate::formula::Formula;
use crate::shift::{Axis, Shift};
use crate::store::CellStore;
use crate::strings::StringTable;

/// A range of cells, inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRange {
    pub start: CellRef,
    pub end: CellRef,
}

impl CellRange {
    /// Builds a range, normalizing so `start` is the top-left corner.
    ///
    /// Users drag selections in all four directions; every consumer downstream
    /// would otherwise have to re-derive which corner is which.
    pub fn new(a: CellRef, b: CellRef) -> Self {
        CellRange {
            start: CellRef::new(a.row.min(b.row), a.col.min(b.col)),
            end: CellRef::new(a.row.max(b.row), a.col.max(b.col)),
        }
    }

    pub fn contains(&self, at: CellRef) -> bool {
        at.row >= self.start.row
            && at.row <= self.end.row
            && at.col >= self.start.col
            && at.col <= self.end.col
    }

    pub fn rows(&self) -> u32 {
        self.end.row - self.start.row + 1
    }

    pub fn cols(&self) -> u32 {
        self.end.col - self.start.col + 1
    }
}

/// How a sheet was last being looked at.
///
/// Part of the document rather than of the application: it is stored in the
/// file, and a workbook reopened at a different sheet, a different zoom, and
/// scrolled back to A1 is not the document that was closed.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetView {
    /// `zoomScale` as a fraction. Excel stores 90 for ninety percent.
    pub zoom: f64,
    /// `showGridLines`. A sheet used as a form or a report turns them off, and
    /// drawing them anyway is the single most visible way to get it wrong.
    pub gridlines: bool,
    pub headings: bool,
    /// The cell that was selected, and the top-left cell that was showing.
    pub selection: Option<CellRef>,
    pub top_left: Option<CellRef>,
    /// The colour of the sheet's tab, which is how a workbook of thirty sheets
    /// is navigated by people who made it.
    pub tab_color: Option<crate::Color>,
}

impl Default for SheetView {
    fn default() -> Self {
        SheetView {
            zoom: 1.0,
            gridlines: true,
            headings: true,
            selection: None,
            top_left: None,
            tab_color: None,
        }
    }
}

/// What kind of sheet a workbook tab holds.
///
/// Chart and dialog sheets have no cell grid, but they *do* occupy a position in
/// the workbook's sheet list — and `localSheetId` on a defined name is an index
/// into that list. Dropping them because they have no cells would silently
/// re-point every sheet-scoped name after them at the wrong sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SheetKind {
    #[default]
    Worksheet,
    Chart,
    Dialog,
    /// An Excel 4.0 macro sheet. Preserved, never executed.
    Macro,
}

impl SheetKind {
    /// True when this sheet has a cell grid to edit.
    pub const fn has_grid(self) -> bool {
        matches!(self, SheetKind::Worksheet | SheetKind::Macro)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub kind: SheetKind,
    /// The package part this sheet was read from, for a sheet that came out of
    /// a file.
    ///
    /// `None` means the sheet has never been written — a new tab, or a whole
    /// workbook that has never been saved — and it is what tells the writer to
    /// *author* a part rather than edit one. It is also the only durable
    /// identity a sheet has: the model reorders and deletes sheets, so a
    /// position in the file's list goes stale the moment anyone drags a tab.
    pub part: Option<String>,
    pub cells: CellStore,
    /// Formula arena. `Cell::formula` indexes this, one-based via [`FormulaId`].
    ///
    /// Per sheet rather than per workbook because shared-formula group indices
    /// (`si`) are only unique within a sheet.
    pub formulas: Vec<Formula>,
    /// Merged regions. The top-left cell holds the value; the rest are covered.
    pub merges: Vec<CellRange>,
    /// Column widths in Excel's character units; absent means the sheet default.
    pub column_widths: BTreeMap<u32, f64>,
    /// Row heights in points; absent means auto.
    pub row_heights: BTreeMap<u32, f64>,
    /// Rows above and columns left of this stay pinned when scrolling.
    pub frozen: Option<CellRef>,
    pub hidden: bool,
    /// A column's style, applied to every cell in it that has none of its own.
    /// This is `<col style="..">`, and it is how a whole column gets shaded
    /// without the file storing a million cells.
    pub column_styles: BTreeMap<u32, crate::StyleId>,
    /// The same for `<row s=".." customFormat="1">`.
    pub row_styles: BTreeMap<u32, crate::StyleId>,
    pub conditional_formats: Vec<crate::cond::ConditionalFormat>,
    pub validations: Vec<crate::cond::DataValidation>,
    /// Charts anchored to this sheet. A view over parts kept verbatim, never a
    /// replacement for them.
    pub charts: Vec<crate::chart::Chart>,
    /// Pictures anchored to this sheet — logos, diagrams, screenshots.
    ///
    /// Kept apart from charts because they are drawn rather than plotted, and
    /// because a sheet with no chart very often still has a masthead.
    pub pictures: Vec<crate::picture::Picture>,
    /// Tables (`ListObject`s) on this sheet.
    ///
    /// Read because a table style is the one piece of a cell's appearance that
    /// is not in `styles.xml`: the cells of a formatted table are very often
    /// unstyled, and everything visible about them comes from a style name.
    pub tables: Vec<crate::table::Table>,
    /// Pivot tables anchored to this sheet, read so that an editor can leave
    /// their region alone. Preserved verbatim, never written.
    pub pivots: Vec<crate::pivot::PivotTable>,
    /// How the sheet was last being *looked at*, which is part of the document.
    ///
    /// A workbook is saved with a sheet showing, at a zoom, scrolled somewhere,
    /// with a cell selected. Opening it anywhere else is opening a different
    /// document than the one that was closed — on the workbook this was built
    /// against, the sheet that matters is the twelfth and it is 90% zoom.
    pub view: crate::SheetView,
    /// The sheet's autofilter, if it has one.
    pub filter: Option<crate::filter::AutoFilter>,
    /// Where each dynamic-array formula last spilled to, by its anchor.
    ///
    /// Derived at recalculation and never read from a file: it exists so that a
    /// result which *shrinks* clears the cells it used to occupy. Without it,
    /// a `FILTER` that goes from five matches to two leaves three stale rows
    /// behind, which is worse than showing nothing.
    pub spills: BTreeMap<CellRef, CellRange>,
}

impl Sheet {
    pub fn new(name: impl Into<String>) -> Self {
        Sheet {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn get(&self, at: CellRef) -> Option<&Cell> {
        self.cells.get(at)
    }

    pub fn set(&mut self, at: CellRef, cell: Cell) -> bool {
        self.cells.set(at, cell)
    }

    /// The merge covering `at`, if any.
    pub fn merge_at(&self, at: CellRef) -> Option<&CellRange> {
        self.merges.iter().find(|m| m.contains(at))
    }

    /// The style that governs a cell, whether or not the cell exists.
    ///
    /// Excel shades a whole column by putting a style on `<col>`, not by storing
    /// a million cells — so a grid that only reads `Cell::style` draws an empty
    /// shaded column as unshaded, and a value typed into it loses its formatting
    /// the moment the row is written. The row wins over the column, matching
    /// Excel's own precedence.
    pub fn style_at(&self, at: CellRef) -> crate::StyleId {
        if let Some(cell) = self.get(at) {
            if cell.style != crate::StyleId::DEFAULT {
                return cell.style;
            }
        }
        if let Some(style) = self.row_styles.get(&at.row) {
            return *style;
        }
        self.column_styles
            .get(&at.col)
            .copied()
            .unwrap_or(crate::StyleId::DEFAULT)
    }

    /// The pivot table covering a cell, if any.
    ///
    /// Typing into one leaves the file self-contradictory — the cells say one
    /// thing and the definition another, and Excel discards the edit at the
    /// next refresh — so the application asks before it writes.
    pub fn pivot_at(&self, at: CellRef) -> Option<&crate::pivot::PivotTable> {
        self.pivots.iter().find(|p| p.covers(at))
    }

    /// The validation rule covering a cell, if any.
    pub fn validation_at(&self, at: CellRef) -> Option<&crate::cond::DataValidation> {
        self.validations.iter().find(|v| v.covers(at))
    }

    /// Adds a formula to the arena and returns its handle.
    pub fn push_formula(&mut self, formula: Formula) -> FormulaId {
        let id = FormulaId::from_index(self.formulas.len() as u32);
        self.formulas.push(formula);
        id
    }

    pub fn formula(&self, id: FormulaId) -> Option<&Formula> {
        self.formulas.get(id.index() as usize)
    }

    /// The formula attached to `at`, if it has one.
    pub fn formula_at(&self, at: CellRef) -> Option<&Formula> {
        self.formula(self.get(at)?.formula?)
    }

    /// Moves everything the sheet holds by position, returning the cells the
    /// shift destroyed.
    ///
    /// Formula *text* is deliberately not touched here. Rewriting `A1` to `A2`
    /// means lexing the formula, which is `ss-formula`'s job and would invert
    /// the dependency between these two crates. The caller has to do both —
    /// [`ss_formula::edit`] is the one that does.
    pub fn shift(&mut self, shift: Shift) -> Vec<(CellRef, Cell)> {
        let removed = self.cells.shift(shift);

        // A merge that loses all but one of its cells stops being a merge.
        self.merges = self
            .merges
            .iter()
            .filter_map(|m| shift.range(*m))
            .filter(|m| m.rows() > 1 || m.cols() > 1)
            .collect();

        for formula in &mut self.formulas {
            match &mut formula.kind {
                crate::formula::FormulaKind::Array { range } => {
                    if let Some(moved) = shift.range(*range) {
                        *range = moved;
                    }
                }
                crate::formula::FormulaKind::Shared { range, .. } => {
                    if let Some(r) = range {
                        *range = shift.range(*r);
                    }
                }
                _ => {}
            }
        }

        let sizes = match shift.axis {
            Axis::Rows => &mut self.row_heights,
            Axis::Columns => &mut self.column_widths,
        };
        *sizes = sizes
            .iter()
            .filter_map(|(index, size)| shift.point(*index).map(|moved| (moved, *size)))
            .collect();

        let styles = match shift.axis {
            Axis::Rows => &mut self.row_styles,
            Axis::Columns => &mut self.column_styles,
        };
        *styles = styles
            .iter()
            .filter_map(|(index, style)| shift.point(*index).map(|moved| (moved, *style)))
            .collect();

        // A conditional format or a validation whose whole region is deleted
        // goes with it; one that survives moves with the cells it was applied to.
        for cf in &mut self.conditional_formats {
            cf.ranges = cf.ranges.iter().filter_map(|r| shift.range(*r)).collect();
        }
        self.conditional_formats.retain(|cf| !cf.ranges.is_empty());
        for dv in &mut self.validations {
            dv.ranges = dv.ranges.iter().filter_map(|r| shift.range(*r)).collect();
        }
        self.validations.retain(|dv| !dv.ranges.is_empty());

        // The freeze is a boundary line, not content: deleting the rows it sits
        // in pulls it up to where they were rather than removing it.
        if let Some(frozen) = self.frozen {
            let index = shift.axis.index(frozen);
            let moved = shift.point(index).unwrap_or(shift.at);
            self.frozen = Some(shift.axis.with(frozen, moved));
        }

        removed
    }

    pub fn insert_rows(&mut self, at: u32, count: u32) -> Vec<(CellRef, Cell)> {
        self.shift(Shift::insert(Axis::Rows, at, count))
    }

    pub fn delete_rows(&mut self, at: u32, count: u32) -> Vec<(CellRef, Cell)> {
        self.shift(Shift::delete(Axis::Rows, at, count))
    }

    pub fn insert_columns(&mut self, at: u32, count: u32) -> Vec<(CellRef, Cell)> {
        self.shift(Shift::insert(Axis::Columns, at, count))
    }

    pub fn delete_columns(&mut self, at: u32, count: u32) -> Vec<(CellRef, Cell)> {
        self.shift(Shift::delete(Axis::Columns, at, count))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    /// Shared across all sheets, mirroring xlsx's single sharedStrings part.
    pub strings: StringTable,
    /// Workbook-scoped names. Sheet-scoped names carry their sheet index.
    pub defined_names: Vec<DefinedName>,
    /// What a cell's [`StyleId`](crate::StyleId) resolves to.
    pub styles: crate::style::StyleTable,
    /// The sheet that was showing when the workbook was saved.
    pub active_sheet: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DefinedName {
    pub name: String,
    /// The formula text the name expands to, stored unparsed.
    ///
    /// Kept as written until the formula engine exists, so a name we cannot yet
    /// evaluate still survives a round trip intact.
    pub refers_to: String,
    /// `None` for workbook scope, else the owning sheet's index.
    pub scope: Option<usize>,
}

impl Workbook {
    pub fn new() -> Self {
        Self::default()
    }

    /// A workbook with one empty sheet, as a new file would be.
    pub fn blank() -> Self {
        let mut wb = Workbook::new();
        wb.sheets.push(Sheet::new("Sheet1"));
        // One style, General, at index 0 — the same table a new file from Excel
        // has. Without it the first format anyone asks for would be allocated
        // index 0, and every unstyled cell in the workbook points there.
        wb.styles = crate::style::StyleTable::build(&std::collections::BTreeMap::new(), &[0]);
        wb
    }

    pub fn sheet(&self, index: usize) -> Option<&Sheet> {
        self.sheets.get(index)
    }

    pub fn sheet_mut(&mut self, index: usize) -> Option<&mut Sheet> {
        self.sheets.get_mut(index)
    }

    /// Finds a sheet by name, case-insensitively — Excel treats sheet names that
    /// way, and refuses to create two that differ only by case.
    pub fn sheet_by_name(&self, name: &str) -> Option<(usize, &Sheet)> {
        self.sheets
            .iter()
            .enumerate()
            .find(|(_, s)| s.name.eq_ignore_ascii_case(name))
    }

    /// Why `name` is not a usable sheet name here, in words a user can act on.
    ///
    /// Both halves matter and they are different kinds of rule. The character
    /// and length rules are Excel's, and a file breaking them is a file Excel
    /// will not open. The uniqueness rule is this workbook's, and it is checked
    /// case-insensitively because a formula saying `data!A1` has to resolve to
    /// exactly one sheet.
    pub fn sheet_name_refusal(&self, name: &str, allowing: Option<usize>) -> Option<String> {
        if let Some(problem) = sheet_name_problem(name) {
            return Some(problem.to_string());
        }
        let taken = self
            .sheets
            .iter()
            .enumerate()
            .any(|(i, s)| Some(i) != allowing && s.name.eq_ignore_ascii_case(name));
        taken.then(|| format!("There is already a sheet called {name}"))
    }

    /// `base`, or `base (2)`, or whichever suffix is free.
    pub fn unique_sheet_name(&self, base: &str) -> String {
        if self.sheet_by_name(base).is_none() {
            return base.to_string();
        }
        for n in 2.. {
            let candidate = format!("{base} ({n})");
            if self.sheet_by_name(&candidate).is_none() {
                return candidate;
            }
        }
        unreachable!("the loop is unbounded")
    }

    /// What Excel would call the next new sheet: `Sheet4` when `Sheet1` to
    /// `Sheet3` are taken, regardless of what the sheets are actually called.
    pub fn next_sheet_name(&self) -> String {
        for n in 1.. {
            let candidate = format!("Sheet{n}");
            if self.sheet_by_name(&candidate).is_none() {
                return candidate;
            }
        }
        unreachable!("the loop is unbounded")
    }

    /// Resolves a name in `scope`, falling back to workbook scope.
    ///
    /// Sheet-scoped names shadow workbook-scoped ones of the same name, which is
    /// how Excel resolves them.
    pub fn resolve_name(&self, name: &str, scope: Option<usize>) -> Option<&DefinedName> {
        if let Some(sheet) = scope {
            if let Some(found) = self
                .defined_names
                .iter()
                .find(|d| d.scope == Some(sheet) && d.name.eq_ignore_ascii_case(name))
            {
                return Some(found);
            }
        }
        self.defined_names
            .iter()
            .find(|d| d.scope.is_none() && d.name.eq_ignore_ascii_case(name))
    }
}

/// The characters Excel refuses in a sheet name.
///
/// Every one of them means something in a formula — `:` builds a range, `\` and
/// `/` are path separators in an external reference, `[` and `]` bracket the
/// workbook, `?` and `*` are wildcards — so a name containing one could not be
/// written into a reference and read back as itself.
const FORBIDDEN: [char; 7] = [':', '\\', '/', '?', '*', '[', ']'];

/// Excel's own limit. Thirty-one characters, and the file records it as such.
pub const MAX_SHEET_NAME: usize = 31;

fn sheet_name_problem(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        return Some("A sheet name cannot be empty");
    }
    if name.chars().count() > MAX_SHEET_NAME {
        return Some("A sheet name is at most 31 characters");
    }
    if name.contains(FORBIDDEN) {
        return Some("A sheet name cannot contain : \\ / ? * [ or ]");
    }
    // A leading or trailing apostrophe collides with the quoting a qualified
    // reference uses, so Excel refuses it even though the character is legal
    // in the middle of a name.
    if name.starts_with('\'') || name.ends_with('\'') {
        return Some("A sheet name cannot start or end with an apostrophe");
    }
    // Reserved by Excel for the change-tracking sheet it writes itself.
    if name.eq_ignore_ascii_case("History") {
        return Some("\"History\" is reserved by Excel");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellValue;

    fn num(v: f64) -> Cell {
        Cell {
            value: CellValue::Number(v),
            ..Default::default()
        }
    }

    #[test]
    fn ranges_normalize_whichever_way_the_user_dragged() {
        let a = CellRange::new(CellRef::new(5, 5), CellRef::new(1, 2));
        assert_eq!(a.start, CellRef::new(1, 2));
        assert_eq!(a.end, CellRef::new(5, 5));

        let b = CellRange::new(CellRef::new(1, 2), CellRef::new(5, 5));
        assert_eq!(a, b, "drag direction must not change the range");
    }

    #[test]
    fn range_geometry() {
        let r = CellRange::new(CellRef::new(2, 3), CellRef::new(4, 3));
        assert_eq!(r.rows(), 3);
        assert_eq!(r.cols(), 1);
        assert!(r.contains(CellRef::new(3, 3)));
        assert!(!r.contains(CellRef::new(3, 4)));
    }

    #[test]
    fn sheet_names_compare_case_insensitively() {
        let mut wb = Workbook::new();
        wb.sheets.push(Sheet::new("Summary"));
        assert!(wb.sheet_by_name("summary").is_some());
        assert!(wb.sheet_by_name("SUMMARY").is_some());
        assert!(wb.sheet_by_name("Summar").is_none());
    }

    #[test]
    fn a_sheet_name_is_refused_for_the_reason_a_user_can_act_on() {
        let mut wb = Workbook::blank();
        wb.sheets.push(Sheet::new("Data"));

        assert!(wb.sheet_name_refusal("Fine", None).is_none());
        assert!(wb.sheet_name_refusal("", None).is_some(), "empty");
        assert!(wb.sheet_name_refusal("   ", None).is_some(), "just spaces");
        assert!(wb.sheet_name_refusal(&"x".repeat(32), None).is_some());
        assert!(wb.sheet_name_refusal(&"x".repeat(31), None).is_none());
        for bad in ["a:b", "a\\b", "a/b", "a?b", "a*b", "a[b", "a]b"] {
            assert!(wb.sheet_name_refusal(bad, None).is_some(), "{bad}");
        }
        assert!(wb.sheet_name_refusal("'quoted'", None).is_some());
        assert!(
            wb.sheet_name_refusal("Bob's Data", None).is_none(),
            "an apostrophe inside is fine; only the ends collide with quoting"
        );
        assert!(wb.sheet_name_refusal("history", None).is_some());

        // Uniqueness is case-insensitive, because `data!A1` has to resolve to
        // exactly one sheet.
        assert!(wb.sheet_name_refusal("DATA", None).is_some());
        assert!(
            wb.sheet_name_refusal("DATA", Some(1)).is_none(),
            "renaming a sheet to what it is already called is not a clash"
        );
    }

    #[test]
    fn a_new_sheet_takes_the_first_free_number() {
        let mut wb = Workbook::blank();
        assert_eq!(wb.next_sheet_name(), "Sheet2", "Sheet1 exists");
        wb.sheets.push(Sheet::new("Sheet2"));
        wb.sheets.push(Sheet::new("Sheet4"));
        assert_eq!(wb.next_sheet_name(), "Sheet3", "the gap is used");

        assert_eq!(wb.unique_sheet_name("Data"), "Data");
        wb.sheets.push(Sheet::new("Data"));
        assert_eq!(wb.unique_sheet_name("Data"), "Data (2)");
        wb.sheets.push(Sheet::new("Data (2)"));
        assert_eq!(wb.unique_sheet_name("Data"), "Data (3)");
    }

    #[test]
    fn sheet_scoped_names_shadow_workbook_scoped_ones() {
        let mut wb = Workbook::blank();
        wb.defined_names.push(DefinedName {
            name: "Rate".into(),
            refers_to: "0.1".into(),
            scope: None,
        });
        wb.defined_names.push(DefinedName {
            name: "Rate".into(),
            refers_to: "0.2".into(),
            scope: Some(0),
        });

        assert_eq!(wb.resolve_name("Rate", Some(0)).unwrap().refers_to, "0.2");
        assert_eq!(wb.resolve_name("Rate", None).unwrap().refers_to, "0.1");
        // A sheet with no local definition falls back to workbook scope.
        assert_eq!(wb.resolve_name("Rate", Some(1)).unwrap().refers_to, "0.1");
    }

    #[test]
    fn merges_are_found_by_any_covered_cell() {
        let mut sheet = Sheet::new("S");
        sheet
            .merges
            .push(CellRange::new(CellRef::new(1, 1), CellRef::new(3, 4)));

        assert!(sheet.merge_at(CellRef::new(1, 1)).is_some(), "anchor");
        assert!(sheet.merge_at(CellRef::new(2, 3)).is_some(), "interior");
        assert!(sheet.merge_at(CellRef::new(3, 4)).is_some(), "far corner");
        assert!(sheet.merge_at(CellRef::new(0, 1)).is_none());
        assert!(sheet.merge_at(CellRef::new(4, 4)).is_none());
    }

    #[test]
    fn inserting_rows_moves_cells_merges_sizes_and_the_freeze() {
        let mut sheet = Sheet::new("S");
        sheet.set(CellRef::new(0, 0), num(1.0));
        sheet.set(CellRef::new(5, 0), num(2.0));
        sheet
            .merges
            .push(CellRange::new(CellRef::new(5, 0), CellRef::new(5, 3)));
        sheet.row_heights.insert(5, 33.0);
        sheet.frozen = Some(CellRef::new(3, 0));

        assert!(
            sheet.insert_rows(2, 2).is_empty(),
            "nothing fell off the grid"
        );

        assert_eq!(
            sheet.get(CellRef::new(0, 0)).map(|c| c.value),
            Some(CellValue::Number(1.0))
        );
        assert_eq!(
            sheet.get(CellRef::new(7, 0)).map(|c| c.value),
            Some(CellValue::Number(2.0))
        );
        assert_eq!(sheet.get(CellRef::new(5, 0)), None);
        assert_eq!(sheet.merges[0].start, CellRef::new(7, 0));
        assert_eq!(sheet.row_heights.get(&7), Some(&33.0));
        assert_eq!(sheet.frozen, Some(CellRef::new(5, 0)));
    }

    #[test]
    fn deleting_rows_hands_back_what_it_destroyed() {
        let mut sheet = Sheet::new("S");
        sheet.set(CellRef::new(1, 0), num(10.0));
        sheet.set(CellRef::new(2, 0), num(20.0));
        sheet.set(CellRef::new(9, 0), num(90.0));

        let removed = sheet.delete_rows(1, 2);
        assert_eq!(removed.len(), 2, "undo has to be able to put them back");
        assert_eq!(removed[0].0, CellRef::new(1, 0));
        assert_eq!(
            sheet.get(CellRef::new(7, 0)).map(|c| c.value),
            Some(CellValue::Number(90.0))
        );
        assert_eq!(sheet.cells.len(), 1);
    }

    #[test]
    fn a_merge_that_loses_all_but_one_cell_stops_being_a_merge() {
        let mut sheet = Sheet::new("S");
        sheet
            .merges
            .push(CellRange::new(CellRef::new(0, 0), CellRef::new(0, 1)));
        sheet.delete_columns(1, 1);
        assert!(sheet.merges.is_empty());
    }

    #[test]
    fn content_pushed_off_the_bottom_is_reported_not_silently_dropped() {
        let mut sheet = Sheet::new("S");
        let last = CellRef::new(crate::cell::MAX_ROWS - 1, 0);
        sheet.set(last, num(1.0));
        let removed = sheet.insert_rows(0, 1);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, last);
    }

    #[test]
    fn blank_workbook_has_one_sheet() {
        let wb = Workbook::blank();
        assert_eq!(wb.sheets.len(), 1);
        assert_eq!(wb.sheets[0].name, "Sheet1");
        assert!(wb.sheets[0].cells.is_empty());
    }

    #[test]
    fn strings_are_shared_across_sheets() {
        // Mirrors xlsx: one sharedStrings part for the whole workbook.
        let mut wb = Workbook::blank();
        wb.sheets.push(Sheet::new("Sheet2"));
        let id = wb.strings.intern("Active");

        wb.sheets[0].set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Text(id),
                ..Default::default()
            },
        );
        wb.sheets[1].set(
            CellRef::new(9, 9),
            Cell {
                value: CellValue::Text(id),
                ..Default::default()
            },
        );

        assert_eq!(wb.strings.len(), 2, "empty string plus one value");
        assert_eq!(wb.strings.resolve(id), "Active");
    }
}
