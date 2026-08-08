//! Cell styles.
//!
//! Excel stores formatting centrally and references it per cell, which is why a
//! workbook with uniform formatting costs almost nothing. We mirror that rather
//! than inlining formatting into cells.
//!
//! Only the number format is resolved at this chunk — it is what the grid needs
//! to decide whether `45352` is a date. Fonts, fills, borders, and alignment
//! land in C11; until then the table carries the indices and nothing reads them,
//! so the file still round-trips through the Preservation Vault untouched.

use std::collections::BTreeMap;

use crate::numfmt::NumberFormat;

/// Index into the workbook's style table.
///
/// This is the `s` attribute on a cell, which indexes `cellXfs` in styles.xml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StyleId(pub u32);

impl StyleId {
    /// The workbook default, always index 0. A cell with this style is unformatted.
    pub const DEFAULT: StyleId = StyleId(0);
}

/// The formatting a workbook's cells can refer to.
#[derive(Debug, Clone, Default)]
pub struct StyleTable {
    /// Distinct parsed formats. Several styles usually share one.
    formats: Vec<NumberFormat>,
    /// Which entry of `formats` each [`StyleId`] uses.
    by_style: Vec<u32>,
    /// General, returned for any style the table does not cover.
    fallback: NumberFormat,
}

impl StyleTable {
    /// Builds the table from what styles.xml said.
    ///
    /// `codes` holds the custom format codes by id; ids below 164 are Excel's
    /// built-ins and are *not* written into the file, so they are resolved from
    /// [`NumberFormat::builtin`]. A workbook whose dates are all `numFmtId="14"`
    /// contains no format code anywhere — get that lookup wrong and every date
    /// in the document displays as a five-digit number.
    pub fn build(codes: &BTreeMap<u32, String>, style_format_ids: &[u32]) -> Self {
        let mut formats = Vec::new();
        let mut seen: BTreeMap<u32, u32> = BTreeMap::new();
        let mut by_style = Vec::with_capacity(style_format_ids.len());

        for id in style_format_ids {
            let slot = *seen.entry(*id).or_insert_with(|| {
                let code = codes
                    .get(id)
                    .map(String::as_str)
                    .or_else(|| NumberFormat::builtin(*id))
                    .unwrap_or("General");
                formats.push(NumberFormat::parse(code));
                formats.len() as u32 - 1
            });
            by_style.push(slot);
        }

        StyleTable {
            formats,
            by_style,
            fallback: NumberFormat::general(),
        }
    }

    /// The number format a cell should display through.
    ///
    /// A style we have never heard of gets General rather than an error — a cell
    /// must always show something, and a file with a dangling style index is
    /// still a file the user wants to read.
    pub fn number_format(&self, style: StyleId) -> &NumberFormat {
        self.by_style
            .get(style.0 as usize)
            .and_then(|slot| self.formats.get(*slot as usize))
            .unwrap_or(&self.fallback)
    }

    pub fn len(&self) -> usize {
        self.by_style.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_style.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numfmt::FormatValue;

    #[test]
    fn default_style_is_index_zero() {
        assert_eq!(StyleId::default(), StyleId::DEFAULT);
        assert_eq!(StyleId::DEFAULT.0, 0);
    }

    #[test]
    fn builtin_ids_resolve_without_the_file_saying_anything() {
        // Style 1 is `numFmtId="14"`, a date, and the file carries no code for it.
        let table = StyleTable::build(&BTreeMap::new(), &[0, 14]);
        let shown = |style: u32| {
            table
                .number_format(StyleId(style))
                .format(FormatValue::Number(45352.0))
                .text
        };
        assert_eq!(shown(0), "45352", "General shows the serial");
        assert_eq!(shown(1), "03-01-24", "id 14 is a date");
    }

    #[test]
    fn custom_codes_win_over_the_builtin_table() {
        let codes = BTreeMap::from([(164, "0.000".to_string())]);
        let table = StyleTable::build(&codes, &[164]);
        assert_eq!(
            table
                .number_format(StyleId(0))
                .format(FormatValue::Number(1.5))
                .text,
            "1.500"
        );
    }

    #[test]
    fn styles_sharing_a_format_share_one_parse() {
        let table = StyleTable::build(&BTreeMap::new(), &[14, 14, 14, 0]);
        assert_eq!(table.len(), 4);
        assert_eq!(table.formats.len(), 2, "one parse per distinct format");
    }

    #[test]
    fn a_dangling_style_index_falls_back_to_general() {
        // Files do contain these. Showing the value is better than showing an
        // error the user cannot act on.
        let table = StyleTable::build(&BTreeMap::new(), &[0]);
        assert_eq!(
            table
                .number_format(StyleId(99))
                .format(FormatValue::Number(1.5))
                .text,
            "1.5"
        );
    }
}
