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

/// The first `numFmtId` a file is allowed to define. Everything below it is an
/// Excel built-in and is never written out.
pub const FIRST_CUSTOM_FORMAT_ID: u32 = 164;

/// The formatting a workbook's cells can refer to.
#[derive(Debug, Clone, Default)]
pub struct StyleTable {
    /// Distinct parsed formats. Several styles usually share one.
    formats: Vec<NumberFormat>,
    /// Which entry of `formats` each [`StyleId`] uses.
    by_style: Vec<u32>,
    /// The `numFmtId` each style names, kept as the file wrote it so a save can
    /// put it back rather than inventing an equivalent one.
    format_ids: Vec<u32>,
    /// Custom format codes by id, as read and as added since.
    codes: BTreeMap<u32, String>,
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
            format_ids: style_format_ids.to_vec(),
            codes: codes.clone(),
            fallback: NumberFormat::general(),
        }
    }

    /// The `numFmtId` a style names, for a writer putting styles.xml back.
    pub fn format_id(&self, style: StyleId) -> u32 {
        self.format_ids.get(style.0 as usize).copied().unwrap_or(0)
    }

    /// Custom format codes by id — everything a writer has to emit as `<numFmt>`.
    pub fn codes(&self) -> &BTreeMap<u32, String> {
        &self.codes
    }

    /// A style that displays through `code`, reusing an existing one if there is
    /// a match and adding one otherwise.
    ///
    /// This is what typing a date into an empty cell needs. Excel does the same
    /// thing: the value it stores is the serial `45306`, and what makes the cell
    /// read as a date is a style pointing at a date format.
    pub fn style_for_format(&mut self, code: &str) -> StyleId {
        if let Some(index) =
            (0..self.by_style.len()).find(|i| self.code_of(StyleId(*i as u32)) == code)
        {
            return StyleId(index as u32);
        }

        let id = self
            .codes
            .iter()
            .find(|(_, existing)| existing.as_str() == code)
            .map(|(id, _)| *id)
            .or_else(|| {
                (0..FIRST_CUSTOM_FORMAT_ID).find(|id| NumberFormat::builtin(*id) == Some(code))
            })
            .unwrap_or_else(|| {
                let next = self
                    .codes
                    .keys()
                    .copied()
                    .filter(|id| *id >= FIRST_CUSTOM_FORMAT_ID)
                    .max()
                    .map_or(FIRST_CUSTOM_FORMAT_ID, |max| max + 1);
                self.codes.insert(next, code.to_string());
                next
            });

        self.formats.push(NumberFormat::parse(code));
        self.by_style.push(self.formats.len() as u32 - 1);
        self.format_ids.push(id);
        StyleId(self.by_style.len() as u32 - 1)
    }

    /// The format code behind a style, resolving built-ins.
    fn code_of(&self, style: StyleId) -> &str {
        let id = self.format_id(style);
        self.codes
            .get(&id)
            .map(String::as_str)
            .or_else(|| NumberFormat::builtin(id))
            .unwrap_or("General")
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
    fn asking_for_a_format_reuses_a_style_that_already_has_it() {
        let mut table = StyleTable::build(&BTreeMap::new(), &[0, 14]);
        let before = table.len();
        assert_eq!(table.style_for_format("General"), StyleId(0));
        assert_eq!(table.len(), before, "no new style for one that exists");
    }

    #[test]
    fn a_new_format_gets_a_custom_id_a_writer_can_emit() {
        let mut table = StyleTable::build(&BTreeMap::new(), &[0]);
        let style = table.style_for_format("0.000");
        assert_eq!(
            table
                .number_format(style)
                .format(FormatValue::Number(1.5))
                .text,
            "1.500"
        );
        assert_eq!(table.format_id(style), FIRST_CUSTOM_FORMAT_ID);
        assert_eq!(
            table
                .codes()
                .get(&FIRST_CUSTOM_FORMAT_ID)
                .map(String::as_str),
            Some("0.000")
        );

        // A second style for the same code shares the id rather than inventing one.
        let again = table.style_for_format("0.000");
        assert_eq!(again, style);
    }

    #[test]
    fn a_builtin_code_is_recognized_rather_than_redefined() {
        // `mm-dd-yy` is built-in id 14. Writing it as a custom format would be
        // legal but would make our files differ from Excel's for no reason.
        let mut table = StyleTable::build(&BTreeMap::new(), &[0]);
        let style = table.style_for_format("mm-dd-yy");
        assert_eq!(table.format_id(style), 14);
        assert!(table.codes().is_empty());
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
