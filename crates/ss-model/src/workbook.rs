//! Sheets and workbooks.

use std::collections::BTreeMap;

use crate::cell::{Cell, CellRef, FormulaId};
use crate::formula::Formula;
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

#[derive(Debug, Clone, Default)]
pub struct Sheet {
    pub name: String,
    pub kind: SheetKind,
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
}

#[derive(Debug, Clone)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellValue;

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
