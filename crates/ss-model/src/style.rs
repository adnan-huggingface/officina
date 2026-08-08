//! Cell styles.
//!
//! Only the identity layer exists at this chunk — enough for the cell store to
//! carry style references and for the round-trip guarantee to hold. Fonts,
//! borders, fills, and number formats land in C11.

/// Index into the workbook's style table.
///
/// Excel stores styles centrally and references them per cell, which is why a
/// workbook with uniform formatting costs almost nothing. We mirror that rather
/// than inlining formatting into cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StyleId(pub u32);

impl StyleId {
    /// The workbook default, always index 0. A cell with this style is unformatted.
    pub const DEFAULT: StyleId = StyleId(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_style_is_index_zero() {
        assert_eq!(StyleId::default(), StyleId::DEFAULT);
        assert_eq!(StyleId::DEFAULT.0, 0);
    }
}
