//! Pivot tables, modeled for what a reader has to know about them.
//!
//! A pivot table is not stored as a formula. Excel writes its *rendered* cells
//! into the worksheet like any others and keeps the definition beside them in
//! two or three more parts — the table's layout, a cache definition naming the
//! fields, and usually a cache of the source records. So the grid already draws
//! one correctly without knowing anything at all.
//!
//! What it does need to know is the **rectangle**. Typing into a pivot table's
//! area leaves the file self-contradictory: the cells say one thing and the
//! definition another, and the next refresh in Excel silently discards the
//! edit. Recording the region is what lets the application say so instead.
//!
//! Nothing here is written back. The parts are preserved verbatim, and editing
//! a pivot table properly means recomputing the cache, which is a chunk of its
//! own and not one anybody has asked for.

use crate::workbook::CellRange;

/// Where a field sits in the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
    /// A `<dataField>` — the thing being summed.
    Data,
    /// A report filter, above the table.
    Filter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub axis: Axis,
    /// `sum`, `count`, `average`… for a data field, as the file spells it.
    pub function: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PivotTable {
    /// The part's name, so the region can be traced back to its definition.
    pub part: String,
    pub name: String,
    /// The whole table, headers and totals included.
    pub location: CellRange,
    /// Where the cache says its records came from, when it says.
    pub source: Option<String>,
    pub fields: Vec<Field>,
}

impl PivotTable {
    pub fn covers(&self, at: crate::CellRef) -> bool {
        self.location.contains(at)
    }

    pub fn fields_on(&self, axis: Axis) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(move |f| f.axis == axis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellRef;

    fn table() -> PivotTable {
        PivotTable {
            part: "/xl/pivotTables/pivotTable1.xml".to_string(),
            name: "PivotTable1".to_string(),
            location: CellRange::new(
                CellRef::from_a1("A3").expect("a1"),
                CellRef::from_a1("D12").expect("a1"),
            ),
            source: Some("Sheet1!$A$1:$C$20".to_string()),
            fields: vec![
                Field {
                    name: "Region".to_string(),
                    axis: Axis::Row,
                    function: None,
                },
                Field {
                    name: "Sales".to_string(),
                    axis: Axis::Data,
                    function: Some("sum".to_string()),
                },
            ],
        }
    }

    #[test]
    fn the_region_is_what_an_editor_has_to_respect() {
        let pivot = table();
        assert!(pivot.covers(CellRef::from_a1("B7").expect("a1")));
        assert!(!pivot.covers(CellRef::from_a1("A2").expect("a1")));
        assert!(!pivot.covers(CellRef::from_a1("E7").expect("a1")));
    }

    #[test]
    fn fields_are_found_by_where_they_sit() {
        let pivot = table();
        assert_eq!(pivot.fields_on(Axis::Row).count(), 1);
        assert_eq!(
            pivot.fields_on(Axis::Data).next().map(|f| f.name.as_str()),
            Some("Sales")
        );
        assert_eq!(pivot.fields_on(Axis::Column).count(), 0);
    }
}
