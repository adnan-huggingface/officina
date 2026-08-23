//! A table style is a scheme, not a set of properties.
//!
//! `<w:tblStyle>` names one entry in `styles.xml`, but what that entry
//! contributes to a given cell depends on where the cell sits: the header row
//! is white on green, the last row is doubled above, every other row alternates
//! between two shades. Word keeps each of those as a `<w:tblStylePr>` of a
//! named type, and the table says which of them it wants through `<w:tblLook>`.
//!
//! A reader that takes only the style's base properties draws every one of
//! these tables as a bare grid — which is what the demonstration document's
//! five tables looked like before this existed.
//!
//! The order the parts apply in is ECMA-376 Part 1 §17.7.6, and it is not the
//! order they are written in: the whole table first, then the column bands,
//! then the row bands, then the first and last column, then the first and last
//! row, and the four corner cells last of all. A cell in the header row of a
//! banded table therefore takes the header's shading and the band's borders,
//! not one or the other.

use std::collections::BTreeMap;

use crate::prop::{ParaProps, RunProps, Shading};
use crate::table::{CellMargins, CellVAlign, TableBorders, TableLook, Width};

/// Which part of a table a conditional format applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Band {
    FirstRow,
    LastRow,
    FirstColumn,
    LastColumn,
    Band1Horizontal,
    Band2Horizontal,
    Band1Vertical,
    Band2Vertical,
    NorthWest,
    NorthEast,
    SouthWest,
    SouthEast,
}

impl Band {
    pub fn from_val(text: &str) -> Option<Band> {
        Some(match text {
            "firstRow" => Band::FirstRow,
            "lastRow" => Band::LastRow,
            "firstCol" => Band::FirstColumn,
            "lastCol" => Band::LastColumn,
            "band1Horz" => Band::Band1Horizontal,
            "band2Horz" => Band::Band2Horizontal,
            "band1Vert" => Band::Band1Vertical,
            "band2Vert" => Band::Band2Vertical,
            "nwCell" => Band::NorthWest,
            "neCell" => Band::NorthEast,
            "swCell" => Band::SouthWest,
            "seCell" => Band::SouthEast,
            _ => return None,
        })
    }

    pub fn to_val(self) -> &'static str {
        match self {
            Band::FirstRow => "firstRow",
            Band::LastRow => "lastRow",
            Band::FirstColumn => "firstCol",
            Band::LastColumn => "lastCol",
            Band::Band1Horizontal => "band1Horz",
            Band::Band2Horizontal => "band2Horz",
            Band::Band1Vertical => "band1Vert",
            Band::Band2Vertical => "band2Vert",
            Band::NorthWest => "nwCell",
            Band::NorthEast => "neCell",
            Band::SouthWest => "swCell",
            Band::SouthEast => "seCell",
        }
    }
}

/// One conditional part of a table style: what it says about the table, its
/// rows, its cells and the text in them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TablePart {
    pub para: ParaProps,
    pub run: RunProps,
    /// `<w:tblPr><w:tblBorders>` — the grid this part draws.
    pub borders: TableBorders,
    pub cell_margins: CellMargins,
    /// `<w:tblInd>` — where the table starts. Stated at all, it is measured to
    /// the first cell's *text*; absent, the table's own edge sits on the
    /// margin. See [`crate::style::StyleTable::resolve_table_indent`].
    pub indent: Option<Width>,
    /// `<w:tcBorders>` — the edges of a cell this part covers, which
    /// outrank the grid above.
    pub cell_borders: TableBorders,
    pub cell_shading: Option<Shading>,
    pub cell_v_align: Option<CellVAlign>,
    /// `<w:tblStyleRowBandSize>` and its column twin, which only the base part
    /// carries. Kept here so one reader serves the base and the bands.
    pub row_band: Option<u32>,
    pub column_band: Option<u32>,
    /// Whether the table named a style of its own, rather than falling to the
    /// document's default table style. Nothing about a cell's *appearance*
    /// turns on it — it is here because the size Word's own Normal states
    /// stops at the edge of such a table, and only such a table. See
    /// [`crate::style::StyleTable::size_normal_does_not_carry_in`].
    pub named: bool,
}

impl TablePart {
    pub fn is_empty(&self) -> bool {
        *self == TablePart::default()
    }
}

/// A table style's whole conditional scheme.
#[derive(Debug, Clone, PartialEq)]
pub struct TableScheme {
    /// `<w:tblPr>` and the `pPr`/`rPr` beside it — what applies everywhere.
    pub whole: TablePart,
    pub parts: BTreeMap<Band, TablePart>,
    /// `<w:tblStyleRowBandSize>` — how many rows make one stripe. Never zero.
    pub row_band: u32,
    pub column_band: u32,
}

impl Default for TableScheme {
    fn default() -> TableScheme {
        TableScheme {
            whole: TablePart::default(),
            parts: BTreeMap::new(),
            // A stripe is one row unless the style says otherwise.
            row_band: 1,
            column_band: 1,
        }
    }
}

impl TableScheme {
    pub fn is_empty(&self) -> bool {
        self.whole.is_empty() && self.parts.is_empty()
    }
}

/// Where one cell sits in its table, which is all the banding rules need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellAt {
    pub row: usize,
    pub rows: usize,
    pub column: usize,
    pub columns: usize,
}

/// The bands that cover a cell, lowest precedence first.
///
/// The corner cells come last because they are the most specific: a style that
/// gives `neCell` a border is describing exactly one cell and must beat both
/// the row and the column it lies in.
pub fn bands(at: CellAt, look: TableLook, scheme: &TableScheme) -> Vec<Band> {
    let mut out = Vec::new();

    // Banding skips the rows and columns that have a part of their own: a
    // striped table's first stripe is the row *after* the header.
    let stripe = |index: usize, count: usize, first: bool, last: bool, size: u32| -> Option<bool> {
        let start = usize::from(first);
        let end = count.saturating_sub(usize::from(last));
        if index < start || index >= end {
            return None;
        }
        let size = size.max(1) as usize;
        Some(((index - start) / size).is_multiple_of(2))
    };

    if !look.no_v_band {
        if let Some(odd) = stripe(
            at.column,
            at.columns,
            look.first_column,
            look.last_column,
            scheme.column_band,
        ) {
            out.push(if odd {
                Band::Band1Vertical
            } else {
                Band::Band2Vertical
            });
        }
    }
    if !look.no_h_band {
        if let Some(odd) = stripe(
            at.row,
            at.rows,
            look.first_row,
            look.last_row,
            scheme.row_band,
        ) {
            out.push(if odd {
                Band::Band1Horizontal
            } else {
                Band::Band2Horizontal
            });
        }
    }

    let first_column = look.first_column && at.column == 0;
    let last_column = look.last_column && at.column + 1 == at.columns;
    let first_row = look.first_row && at.row == 0;
    let last_row = look.last_row && at.row + 1 == at.rows;

    if first_column {
        out.push(Band::FirstColumn);
    }
    if last_column {
        out.push(Band::LastColumn);
    }
    if first_row {
        out.push(Band::FirstRow);
    }
    if last_row {
        out.push(Band::LastRow);
    }
    match (first_row, last_row, first_column, last_column) {
        (true, _, true, _) => out.push(Band::NorthWest),
        (true, _, _, true) => out.push(Band::NorthEast),
        (_, true, true, _) => out.push(Band::SouthWest),
        (_, true, _, true) => out.push(Band::SouthEast),
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> TableLook {
        TableLook {
            first_row: true,
            last_row: false,
            first_column: false,
            last_column: false,
            no_h_band: false,
            no_v_band: true,
        }
    }

    fn at(row: usize, rows: usize) -> CellAt {
        CellAt {
            row,
            rows,
            column: 0,
            columns: 3,
        }
    }

    #[test]
    fn a_header_row_takes_the_header_part_and_no_stripe() {
        let found = bands(at(0, 5), look(), &TableScheme::default());
        assert_eq!(found, vec![Band::FirstRow]);
    }

    #[test]
    fn the_stripes_begin_below_the_header_rather_than_at_the_top() {
        let scheme = TableScheme::default();
        // Row 1 is the first body row, so it is the *first* stripe.
        assert_eq!(
            bands(at(1, 5), look(), &scheme),
            vec![Band::Band1Horizontal]
        );
        assert_eq!(
            bands(at(2, 5), look(), &scheme),
            vec![Band::Band2Horizontal]
        );
        assert_eq!(
            bands(at(3, 5), look(), &scheme),
            vec![Band::Band1Horizontal]
        );
    }

    #[test]
    fn a_wider_stripe_covers_more_rows_before_it_alternates() {
        let scheme = TableScheme {
            row_band: 2,
            ..TableScheme::default()
        };
        let kinds: Vec<Band> = (1..5)
            .map(|row| bands(at(row, 6), look(), &scheme)[0])
            .collect();
        assert_eq!(
            kinds,
            vec![
                Band::Band1Horizontal,
                Band::Band1Horizontal,
                Band::Band2Horizontal,
                Band::Band2Horizontal
            ]
        );
    }

    #[test]
    fn a_table_that_asks_for_no_banding_gets_none() {
        let look = TableLook {
            no_h_band: true,
            ..look()
        };
        assert!(bands(at(2, 5), look, &TableScheme::default()).is_empty());
    }

    #[test]
    fn a_corner_is_more_specific_than_the_row_and_column_that_meet_there() {
        let look = TableLook {
            first_row: true,
            first_column: true,
            no_h_band: true,
            no_v_band: true,
            ..TableLook::default()
        };
        let found = bands(
            CellAt {
                row: 0,
                rows: 4,
                column: 0,
                columns: 4,
            },
            look,
            &TableScheme::default(),
        );
        assert_eq!(
            found,
            vec![Band::FirstColumn, Band::FirstRow, Band::NorthWest],
            "the corner applies last, so it wins"
        );
    }

    #[test]
    fn every_conditional_type_word_writes_is_recognised() {
        for name in [
            "firstRow",
            "lastRow",
            "firstCol",
            "lastCol",
            "band1Horz",
            "band2Horz",
            "band1Vert",
            "band2Vert",
            "nwCell",
            "neCell",
            "swCell",
            "seCell",
        ] {
            let band = Band::from_val(name).expect("a type Word writes");
            assert_eq!(band.to_val(), name, "and it writes back the same way");
        }
        assert_eq!(Band::from_val("nonsense"), None);
    }
}
