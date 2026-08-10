//! Adding, removing, renaming and reordering sheets.
//!
//! Every one of these is a workbook-wide edit wearing a local disguise. A sheet
//! is named by *its name* in every formula that mentions it and by *its index*
//! in every sheet-scoped defined name, and both of those change under an
//! operation that looks like it only touches a tab.
//!
//! - Rename: every qualifier spelling the old name has to be respelled, or the
//!   formula stops resolving and comes back `#NAME?`.
//! - Delete: every reference into it becomes `#REF!`, and every scoped name
//!   after it shifts down one.
//! - Insert and move: no formula changes at all, and every scoped name after
//!   the insertion point shifts.
//!
//! Doing any of them without the others leaves a workbook that opens and is
//! quietly wrong, which is the failure mode this whole crate exists to avoid.

use ss_model::{DefinedName, Sheet, Workbook};

use crate::edit::{Change, Patch};
use crate::translate::{drop_sheet, rename_sheet};

/// A new empty sheet at `at`, named `name`.
pub fn insert(book: &Workbook, at: usize, name: &str) -> Change {
    let at = at.min(book.sheets.len());
    let mut sheet = Sheet::new(name);
    // No part: this sheet has never been in a file, and that is what tells the
    // writer to author one rather than look for it.
    sheet.part = None;
    let mut patches = vec![Patch::SheetInsert {
        index: at,
        sheet: Box::new(sheet),
    }];
    if let Some(names) = rescoped(book, |scope| Some(scope + usize::from(scope >= at))) {
        patches.push(names);
    }
    Change::new("Insert sheet", patches)
}

/// A copy of `index` placed at `at`, named `name` — Excel's Move or Copy with
/// the box ticked.
pub fn duplicate(book: &Workbook, index: usize, at: usize, name: &str) -> Change {
    let Some(source) = book.sheet(index) else {
        return Change::default();
    };
    let mut copy = source.clone();
    copy.name = name.to_string();
    // The copy is not the part the original came from, and saying it was would
    // have the writer rewrite one part with two sheets' contents.
    copy.part = None;
    let at = at.min(book.sheets.len());

    let mut patches = vec![Patch::SheetInsert {
        index: at,
        sheet: Box::new(copy),
    }];
    if let Some(names) = rescoped(book, |scope| Some(scope + usize::from(scope >= at))) {
        patches.push(names);
    }
    Change::new("Copy sheet", patches)
}

/// Removes a sheet, turning every reference into it into `#REF!`.
///
/// Refuses to remove the last sheet that has a grid: a workbook with no
/// worksheet is not something Excel will open, and it is not something a user
/// can undo their way out of if the application has nothing left to draw.
pub fn remove(book: &Workbook, index: usize) -> Change {
    let Some(going) = book.sheet(index) else {
        return Change::default();
    };
    let name = going.name.clone();
    let mut patches = vec![Patch::SheetRemove { index }];

    for (other, sheet) in book.sheets.iter().enumerate() {
        if other == index {
            continue;
        }
        let texts: Vec<(ss_model::FormulaId, String)> = sheet
            .formulas
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.text.is_empty())
            .filter_map(|(i, f)| {
                let rewritten = drop_sheet(&f.text, &name)?;
                Some((ss_model::FormulaId::from_index(i as u32), rewritten))
            })
            .collect();
        if !texts.is_empty() {
            // Indices are as they are *after* the removal, because that is when
            // this patch runs.
            patches.push(Patch::Formulas {
                sheet: other - usize::from(other > index),
                texts,
            });
        }
    }

    // Names scoped to the sheet go with it; names after it shift down one; and
    // any name whose target mentioned it loses that target.
    let names: Vec<DefinedName> = book
        .defined_names
        .iter()
        .filter(|d| d.scope != Some(index))
        .map(|d| DefinedName {
            name: d.name.clone(),
            refers_to: drop_sheet(&d.refers_to, &name).unwrap_or_else(|| d.refers_to.clone()),
            scope: d.scope.map(|s| s - usize::from(s > index)),
        })
        .collect();
    if names != book.defined_names {
        patches.push(Patch::DefinedNames { names });
    }

    Change::new("Delete sheet", patches)
}

/// Renames a sheet and respells every qualifier that named it.
pub fn rename(book: &Workbook, index: usize, to: &str) -> Change {
    let Some(target) = book.sheet(index) else {
        return Change::default();
    };
    let from = target.name.clone();
    if from == to {
        return Change::default();
    }

    let mut patches = vec![Patch::SheetName {
        index,
        name: to.to_string(),
    }];
    for (other, sheet) in book.sheets.iter().enumerate() {
        let texts: Vec<(ss_model::FormulaId, String)> = sheet
            .formulas
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.text.is_empty())
            .filter_map(|(i, f)| {
                let rewritten = rename_sheet(&f.text, &from, to)?;
                Some((ss_model::FormulaId::from_index(i as u32), rewritten))
            })
            .collect();
        if !texts.is_empty() {
            patches.push(Patch::Formulas {
                sheet: other,
                texts,
            });
        }
    }

    let names: Vec<DefinedName> = book
        .defined_names
        .iter()
        .map(|d| DefinedName {
            name: d.name.clone(),
            refers_to: rename_sheet(&d.refers_to, &from, to).unwrap_or_else(|| d.refers_to.clone()),
            scope: d.scope,
        })
        .collect();
    if names != book.defined_names {
        patches.push(Patch::DefinedNames { names });
    }

    Change::new("Rename sheet", patches)
}

/// Moves a tab. No formula changes: a qualifier names a sheet, not a position.
pub fn reorder(book: &Workbook, from: usize, to: usize) -> Change {
    if from == to || from >= book.sheets.len() || to >= book.sheets.len() {
        return Change::default();
    }
    let mut patches = vec![Patch::SheetMove { from, to }];
    if let Some(names) = rescoped(book, |scope| Some(moved_index(scope, from, to))) {
        patches.push(names);
    }
    Change::new("Move sheet", patches)
}

pub fn set_hidden(index: usize, hidden: bool) -> Change {
    Change::new(
        if hidden { "Hide sheet" } else { "Unhide sheet" },
        vec![Patch::SheetHidden { index, hidden }],
    )
}

pub fn set_tab_color(index: usize, color: Option<ss_model::Color>) -> Change {
    Change::new("Tab colour", vec![Patch::TabColor { index, color }])
}

/// Where an index lands when the item at `from` is taken out and put back at
/// `to`.
fn moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        return to;
    }
    match (from < to, index) {
        (true, i) if i > from && i <= to => i - 1,
        (false, i) if i >= to && i < from => i + 1,
        (_, i) => i,
    }
}

/// The defined-names patch for a change of sheet *positions*, or `None` if no
/// scoped name moved.
fn rescoped(book: &Workbook, remap: impl Fn(usize) -> Option<usize>) -> Option<Patch> {
    let names: Vec<DefinedName> = book
        .defined_names
        .iter()
        .map(|d| DefinedName {
            scope: d.scope.and_then(&remap),
            ..d.clone()
        })
        .collect();
    (names != book.defined_names).then_some(Patch::DefinedNames { names })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::apply;
    use ss_model::{Cell, CellRef, Formula, FormulaId, StyleId};

    fn book() -> Workbook {
        let mut book = Workbook::blank();
        book.sheets.push(Sheet::new("Data"));
        book.sheets.push(Sheet::new("Notes"));
        book
    }

    /// Puts a formula on a sheet and returns its handle.
    fn formula(book: &mut Workbook, sheet: usize, at: &str, text: &str) -> FormulaId {
        let id = book.sheets[sheet].push_formula(Formula::normal(text));
        let at = CellRef::from_a1(at).expect("a1");
        book.sheets[sheet].set(
            at,
            Cell {
                value: ss_model::CellValue::Blank,
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );
        id
    }

    fn text(book: &Workbook, sheet: usize, id: FormulaId) -> &str {
        &book.sheets[sheet].formulas[id.index() as usize].text
    }

    #[test]
    fn renaming_a_sheet_reaches_every_formula_and_every_defined_name() {
        let mut book = book();
        let here = formula(&mut book, 0, "A1", "Data!B2*2");
        let there = formula(&mut book, 2, "A1", "SUM(Data!A1:A9)");
        book.defined_names.push(DefinedName {
            name: "Total".into(),
            refers_to: "Data!$A$1:$A$9".into(),
            scope: None,
        });

        let change = rename(&book, 1, "Sales");
        let undo = apply(&mut book, change);

        assert_eq!(book.sheets[1].name, "Sales");
        assert_eq!(text(&book, 0, here), "Sales!B2*2");
        assert_eq!(text(&book, 2, there), "SUM(Sales!A1:A9)");
        assert_eq!(book.defined_names[0].refers_to, "Sales!$A$1:$A$9");

        apply(&mut book, undo);
        assert_eq!(book.sheets[1].name, "Data");
        assert_eq!(text(&book, 0, here), "Data!B2*2");
        assert_eq!(book.defined_names[0].refers_to, "Data!$A$1:$A$9");
    }

    #[test]
    fn deleting_a_sheet_leaves_ref_errors_and_gives_the_sheet_back_on_undo() {
        let mut book = book();
        formula(&mut book, 1, "A1", "42");
        book.sheets[1].set(
            CellRef::from_a1("B1").expect("a1"),
            Cell {
                value: ss_model::CellValue::Number(7.0),
                ..Default::default()
            },
        );
        let pointing = formula(&mut book, 0, "A1", "Data!B1+1");

        let change = remove(&book, 1);
        let undo = apply(&mut book, change);

        assert_eq!(book.sheets.len(), 2);
        assert_eq!(book.sheets[1].name, "Notes");
        assert_eq!(
            text(&book, 0, pointing),
            "#REF!+1",
            "a formula into a sheet that is gone says so"
        );

        apply(&mut book, undo);
        assert_eq!(book.sheets.len(), 3);
        assert_eq!(book.sheets[1].name, "Data");
        assert_eq!(
            book.sheets[1]
                .get(CellRef::from_a1("B1").expect("a1"))
                .map(|c| c.value),
            Some(ss_model::CellValue::Number(7.0)),
            "with its cells"
        );
        assert_eq!(text(&book, 0, pointing), "Data!B1+1");
    }

    #[test]
    fn a_scoped_name_follows_its_sheet_through_every_rearrangement() {
        // `localSheetId` is a position in the sheet list, so anything that moves
        // a sheet re-points every name after it. Getting this wrong scopes a
        // name to the wrong sheet, which is invisible until someone uses it.
        let mut book = book();
        book.defined_names.push(DefinedName {
            name: "Local".into(),
            refers_to: "$A$1".into(),
            scope: Some(2),
        });

        let change = insert(&book, 0, "New");
        let undo = apply(&mut book, change);
        assert_eq!(
            book.defined_names[0].scope,
            Some(3),
            "everything moved down"
        );
        apply(&mut book, undo);
        assert_eq!(book.defined_names[0].scope, Some(2));

        let change = reorder(&book, 2, 0);
        let undo = apply(&mut book, change);
        assert_eq!(book.sheets[0].name, "Notes");
        assert_eq!(book.defined_names[0].scope, Some(0), "it went with the tab");
        apply(&mut book, undo);
        assert_eq!(book.defined_names[0].scope, Some(2));
        assert_eq!(book.sheets[2].name, "Notes", "and the tab came back");

        // A name scoped to the sheet that is deleted goes with it.
        let change = remove(&book, 2);
        let undo = apply(&mut book, change);
        assert!(book.defined_names.is_empty());
        apply(&mut book, undo);
        assert_eq!(book.defined_names.len(), 1);
        assert_eq!(book.defined_names[0].scope, Some(2));
    }

    #[test]
    fn moving_a_tab_is_the_same_permutation_either_direction() {
        for (from, to) in [(0usize, 2usize), (2, 0), (1, 2), (0, 1)] {
            let mut order: Vec<usize> = (0..4).collect();
            let sheet = order.remove(from);
            order.insert(to, sheet);
            for (was, is) in (0..4).map(|i| (i, moved_index(i, from, to))) {
                assert_eq!(
                    order[is], was,
                    "index {was} after moving {from} to {to} should be {is}"
                );
            }
        }
    }

    #[test]
    fn a_copied_sheet_carries_the_cells_and_not_the_part() {
        let mut book = book();
        book.sheets[1].part = Some("/xl/worksheets/sheet2.xml".into());
        book.sheets[1].set(
            CellRef::from_a1("A1").expect("a1"),
            Cell {
                value: ss_model::CellValue::Number(3.0),
                ..Default::default()
            },
        );

        let change = duplicate(&book, 1, 2, "Data (2)");
        apply(&mut book, change);

        assert_eq!(book.sheets[2].name, "Data (2)");
        assert_eq!(
            book.sheets[2]
                .get(CellRef::from_a1("A1").expect("a1"))
                .map(|c| c.value),
            Some(ss_model::CellValue::Number(3.0))
        );
        assert_eq!(
            book.sheets[2].part, None,
            "two sheets pointing at one part would have the writer put both in it"
        );
        assert_eq!(
            book.sheets[1].part.as_deref(),
            Some("/xl/worksheets/sheet2.xml")
        );
    }
}
