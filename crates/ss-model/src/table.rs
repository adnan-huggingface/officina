//! Tables — `xl/tables/tableN.xml` — and the look a table style gives a cell.
//!
//! A table is the one place where a cell's appearance is not in `styles.xml` at
//! all. `<tableStyleInfo name="TableStyleMedium15"/>` names a style that lives
//! *in Excel*, not in the package, and every cell in the table can be — and on
//! a real sheet usually is — completely unstyled. Read the cells alone and a
//! formatted table comes out as bare text on white, which is what the CRC
//! calculator sheet of the reference workbook did: a black header row with
//! white headings and a grey data row, rendered as neither.
//!
//! What is read from the file is exact: the range, how many rows are header and
//! totals, which of the four emphases are switched on, and the `dxf` overrides.
//! What is *not* in the file is the built-in style's palette. Excel's own
//! definitions are not published in the package and could not be measured here,
//! so the colours below are our rendering of the gallery rather than a copy of
//! it — the same kind of approximation as drawing a 3-D chart flat. The one
//! style that has been checked against Excel pixel for pixel is
//! `TableStyleMedium15`: a solid black header with white bold text, and body
//! stripes in `#D9D9D9`, which is black lightened by 0.85.
//!
//! Nothing here is ever written. A table style is a view over a name.

use crate::style::{Border, BorderStyle, Dxf, Edge, Fill};
use crate::{CellRange, CellRef, Color};

/// The theme slot a built-in style is built from: text 1, or one of the six
/// accents. The gallery is seven columns wide and this is the column.
const SLOTS: [u32; 7] = [1, 4, 5, 6, 7, 8, 9];

/// How much a body stripe is lightened from the style's own colour.
///
/// Measured: `TableStyleMedium15` is built from text 1 (black) and its stripe
/// is `#D9D9D9`, which is exactly this tint of it.
const STRIPE_TINT: f64 = 0.85;

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The package part, kept so the writer can find what it came from.
    pub part: String,
    /// `displayName`, which is what a structured reference names.
    pub name: String,
    pub range: CellRange,
    /// `headerRowCount`, which is 1 unless the file says otherwise.
    pub header_rows: u32,
    pub totals_rows: u32,
    pub style: TableStyle,
    /// `headerRowDxfId` and friends: overrides the file *does* carry, applied
    /// over whatever the built-in style says.
    pub header_dxf: Option<u32>,
    pub data_dxf: Option<u32>,
    pub totals_dxf: Option<u32>,
}

/// `<tableStyleInfo>`: which style, and which of its emphases are on.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableStyle {
    /// `None` for a table shown with no style at all.
    pub name: Option<String>,
    pub row_stripes: bool,
    pub column_stripes: bool,
    pub first_column: bool,
    pub last_column: bool,
}

/// Which horizontal band of a table a cell falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Header,
    Totals,
    /// `stripe` is true on the rows a striped style fills.
    Body {
        stripe: bool,
    },
}

/// The three families of the built-in gallery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Light,
    Medium,
    Dark,
}

/// A built-in style name taken apart: which family, and which of the seven
/// theme colours the gallery column uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Builtin {
    family: Family,
    slot: u32,
}

impl Table {
    pub fn contains(&self, at: CellRef) -> bool {
        self.range.contains(at)
    }

    /// Which band a cell is in, or `None` when it is not in the table.
    pub fn band_at(&self, at: CellRef) -> Option<Band> {
        if !self.range.contains(at) {
            return None;
        }
        let top = self.range.start.row;
        let bottom = self.range.end.row;
        if at.row < top.saturating_add(self.header_rows) {
            return Some(Band::Header);
        }
        if self.totals_rows > 0 && at.row + self.totals_rows > bottom {
            return Some(Band::Totals);
        }
        // Excel counts stripes from the first body row, and fills that one.
        let body = at.row - (top + self.header_rows);
        Some(Band::Body {
            stripe: self.style.row_stripes && body.is_multiple_of(2),
        })
    }

    /// What the table style makes a cell look like, before the cell's own
    /// style is laid over it.
    ///
    /// `None` when the table has no style, or names one we do not recognise —
    /// a custom style defined in the workbook, which lives in `styles.xml` as
    /// a `<tableStyle>` we do not read.
    ///
    /// The colours come back as *theme* colours rather than resolved RGB, so
    /// they follow the workbook's own scheme the way every other colour in a
    /// file does. Resolving is the caller's, against the theme it is painting
    /// with.
    pub fn look(&self, at: CellRef) -> Option<Dxf> {
        let band = self.band_at(at)?;
        let style = Builtin::parse(self.style.name.as_deref()?)?;
        let base = Color::Theme {
            index: SLOTS[style.slot as usize],
            tint: 0.0,
        };
        let mut dxf = Dxf::default();

        match (style.family, band) {
            // A Light table has no header fill: it is a rule under bold text.
            (Family::Light, Band::Header) => {
                dxf.bold = Some(true);
            }
            (_, Band::Header) => {
                dxf.fill = Some(Fill::solid(base));
                dxf.color = Some(Color::Theme {
                    index: 0,
                    tint: 0.0,
                });
                dxf.bold = Some(true);
            }
            (_, Band::Totals) => {
                dxf.bold = Some(true);
            }
            (Family::Dark, Band::Body { stripe: true }) => {
                dxf.fill = Some(Fill::solid(tinted(base, 0.4)));
            }
            (_, Band::Body { stripe: true }) => {
                dxf.fill = Some(Fill::solid(tinted(base, STRIPE_TINT)));
            }
            (_, Band::Body { stripe: false }) => {}
        }

        // Emphasised first and last columns are bold in every family.
        if (self.style.first_column && at.col == self.range.start.col)
            || (self.style.last_column && at.col == self.range.end.col)
        {
            dxf.bold = Some(true);
        }

        // The outline. Excel draws it round the table rather than round each
        // cell, so only the cells on an edge get one — which is why this needs
        // the address and not just the band.
        let edge = Edge {
            style: BorderStyle::Thin,
            color: base,
        };
        let mut border = Border::default();
        let mut outlined = false;
        if at.row == self.range.start.row {
            border.top = edge;
            outlined = true;
        }
        if at.row == self.range.end.row {
            border.bottom = edge;
            outlined = true;
        }
        if at.col == self.range.start.col {
            border.left = edge;
            outlined = true;
        }
        if at.col == self.range.end.col {
            border.right = edge;
            outlined = true;
        }
        if outlined {
            dxf.border = Some(border);
        }

        (!dxf.is_empty()).then_some(dxf)
    }
}

fn tinted(color: Color, tint: f64) -> Color {
    match color {
        Color::Theme { index, .. } => Color::Theme { index, tint },
        other => other,
    }
}

impl Builtin {
    /// `TableStyleMedium15` -> the Medium family, gallery column 0.
    ///
    /// The gallery is seven columns wide — text 1, then the six accents — and
    /// the number runs along the rows. Everything else about a style is which
    /// row it is in, which is what the families stand in for here.
    fn parse(name: &str) -> Option<Builtin> {
        let (family, rest) = [
            (Family::Light, "TableStyleLight"),
            (Family::Medium, "TableStyleMedium"),
            (Family::Dark, "TableStyleDark"),
        ]
        .into_iter()
        .find_map(|(family, prefix)| Some((family, name.strip_prefix(prefix)?)))?;
        let n: u32 = rest.parse().ok()?;
        // The gallery is numbered from one; a `TableStyleMedium0` is not a
        // style Excel can have written.
        let index = n.checked_sub(1)?;
        Some(Builtin {
            family,
            slot: index % 7,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn at(a1: &str) -> CellRef {
        CellRef::from_a1(a1).expect("an address")
    }

    /// The CRC calculator sheet's table, as the file gives it.
    fn crc_table() -> Table {
        Table {
            part: "/xl/tables/table1.xml".into(),
            name: "Table1".into(),
            range: CellRange::new(at("E7"), at("M8")),
            header_rows: 1,
            totals_rows: 0,
            style: TableStyle {
                name: Some("TableStyleMedium15".into()),
                row_stripes: true,
                column_stripes: false,
                first_column: false,
                last_column: false,
            },
            header_dxf: Some(1),
            data_dxf: None,
            totals_dxf: None,
        }
    }

    #[test]
    fn the_header_row_is_solid_black_with_white_bold_text() {
        // Checked against Excel's own rendering of this table.
        let table = crc_table();
        let theme = Theme::default();
        let look = table.look(at("E7")).expect("a header look");
        assert_eq!(
            look.fill.and_then(|f| f.shade(&theme)),
            Some([0x00, 0x00, 0x00])
        );
        assert_eq!(
            look.color.and_then(|c| c.resolve(&theme)),
            Some([0xFF, 0xFF, 0xFF])
        );
        assert_eq!(look.bold, Some(true));
    }

    #[test]
    fn the_first_body_row_is_the_grey_excel_draws() {
        let table = crc_table();
        let theme = Theme::default();
        let look = table.look(at("E8")).expect("a body look");
        assert_eq!(
            look.fill.and_then(|f| f.shade(&theme)),
            Some([0xD9, 0xD9, 0xD9]),
            "black lightened by 0.85, which is what Excel draws"
        );
    }

    #[test]
    fn stripes_alternate_from_the_first_body_row() {
        let mut table = crc_table();
        table.range = CellRange::new(at("E7"), at("M11"));
        let bands: Vec<Band> = ["E8", "E9", "E10", "E11"]
            .iter()
            .map(|a| table.band_at(at(a)).expect("in the table"))
            .collect();
        assert_eq!(
            bands,
            vec![
                Band::Body { stripe: true },
                Band::Body { stripe: false },
                Band::Body { stripe: true },
                Band::Body { stripe: false },
            ]
        );
    }

    #[test]
    fn a_style_with_stripes_switched_off_fills_nothing() {
        let mut table = crc_table();
        table.style.row_stripes = false;
        let look = table.look(at("E8"));
        assert!(look.is_none_or(|l| l.fill.is_none()));
    }

    #[test]
    fn the_outline_is_drawn_round_the_table_and_not_round_every_cell() {
        let table = crc_table();
        let corner = table.look(at("E7")).expect("a look");
        let border = corner.border.expect("the corner has edges");
        assert_eq!(border.top.style, BorderStyle::Thin);
        assert_eq!(border.left.style, BorderStyle::Thin);
        assert_eq!(border.right.style, BorderStyle::None, "E is not the last");

        let middle = table.look(at("G8")).expect("a look");
        let border = middle.border.expect("still on the bottom row");
        assert_eq!(border.left.style, BorderStyle::None);
        assert_eq!(border.right.style, BorderStyle::None);
        assert_eq!(border.bottom.style, BorderStyle::Thin);
    }

    #[test]
    fn the_gallery_is_seven_columns_wide() {
        // 1 and 8 and 15 are the same colour in different rows of the gallery;
        // 2 is the next one along.
        let slot = |n: &str| Builtin::parse(n).expect("a built-in name").slot;
        assert_eq!(slot("TableStyleMedium1"), 0);
        assert_eq!(slot("TableStyleMedium8"), 0);
        assert_eq!(slot("TableStyleMedium15"), 0);
        assert_eq!(slot("TableStyleMedium16"), 1);
        assert_eq!(slot("TableStyleLight21"), 6);
    }

    #[test]
    fn a_style_we_do_not_know_is_left_alone_rather_than_guessed_at() {
        let mut table = crc_table();
        table.style.name = Some("MyCompanyTableStyle".into());
        assert!(table.look(at("E7")).is_none());
    }

    #[test]
    fn a_cell_outside_the_table_gets_nothing() {
        let table = crc_table();
        assert!(table.band_at(at("D7")).is_none());
        assert!(table.look(at("E20")).is_none());
    }
}
