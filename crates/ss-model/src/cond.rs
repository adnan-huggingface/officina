//! Conditional formatting and data validation.
//!
//! Both are rules attached to *regions* rather than to cells, and both are
//! stored on the worksheet rather than in `styles.xml`. They are modeled here
//! and evaluated in `ss-formula`, because half the rule kinds are just a formula
//! and the model may not depend on the engine.
//!
//! Neither is written back by the save path. They live outside `<sheetData>`,
//! so the splice writer carries them through byte for byte — reading them is
//! what the grid needs to *draw* the workbook the way its author sees it, and
//! preservation is already handled by not touching them.

use crate::workbook::CellRange;

/// How a rule decides. The `dxf` it carries is what gets applied when it does.
#[derive(Debug, Clone, PartialEq)]
pub enum CfKind {
    /// `cellIs`: the cell's value against one or two formulas.
    CellIs {
        operator: CfOperator,
        formulas: Vec<String>,
    },
    /// `expression`: an arbitrary formula, true meaning "format me". The formula
    /// is written relative to the *top-left cell of the region*, so applying it
    /// to any other cell means offsetting every relative reference in it.
    Expression { formula: String },
    /// Two or three colours interpolated across the region's value range.
    ColorScale {
        stops: Vec<CfValue>,
        colors: Vec<crate::color::Color>,
    },
    DataBar {
        min: CfValue,
        max: CfValue,
        color: crate::color::Color,
        show_value: bool,
    },
    IconSet {
        set: String,
        stops: Vec<CfValue>,
        show_value: bool,
        reverse: bool,
    },
    /// The top or bottom `rank` values, or percent of them.
    Top10 {
        rank: u32,
        percent: bool,
        bottom: bool,
    },
    AboveAverage {
        above: bool,
        equal_average: bool,
        std_dev: Option<i32>,
    },
    /// `containsText`, `notContainsText`, `beginsWith`, `endsWith`.
    Text { op: TextOp, text: String },
    /// `timePeriod`, e.g. `today`, `lastWeek`.
    TimePeriod { period: String },
    /// `duplicateValues` and `uniqueValues`.
    Duplicates { unique: bool },
    /// `containsBlanks`/`notContainsBlanks` and the error pair.
    Presence { blanks: bool, negated: bool },
    /// A rule type we do not evaluate. Kept so the count of rules stays honest
    /// and `stop_if_true` still means what the file says.
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfOperator {
    LessThan,
    LessThanOrEqual,
    Equal,
    NotEqual,
    GreaterThanOrEqual,
    GreaterThan,
    Between,
    NotBetween,
    ContainsText,
    NotContains,
    BeginsWith,
    EndsWith,
}

impl CfOperator {
    pub fn from_xml(text: &str) -> Option<CfOperator> {
        Some(match text {
            "lessThan" => CfOperator::LessThan,
            "lessThanOrEqual" => CfOperator::LessThanOrEqual,
            "equal" => CfOperator::Equal,
            "notEqual" => CfOperator::NotEqual,
            "greaterThanOrEqual" => CfOperator::GreaterThanOrEqual,
            "greaterThan" => CfOperator::GreaterThan,
            "between" => CfOperator::Between,
            "notBetween" => CfOperator::NotBetween,
            "containsText" => CfOperator::ContainsText,
            "notContains" => CfOperator::NotContains,
            "beginsWith" => CfOperator::BeginsWith,
            "endsWith" => CfOperator::EndsWith,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOp {
    Contains,
    NotContains,
    BeginsWith,
    EndsWith,
}

/// One end of a scale: `<cfvo type="min"/>`, `<cfvo type="num" val="0"/>`.
#[derive(Debug, Clone, PartialEq)]
pub struct CfValue {
    pub kind: CfValueKind,
    /// Unparsed, because `formula` is a formula and `num` is a number and the
    /// attribute is the same either way.
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfValueKind {
    Min,
    Max,
    Number,
    Percent,
    Percentile,
    Formula,
    AutoMin,
    AutoMax,
}

impl CfValueKind {
    pub fn from_xml(text: &str) -> CfValueKind {
        match text {
            "min" => CfValueKind::Min,
            "max" => CfValueKind::Max,
            "percent" => CfValueKind::Percent,
            "percentile" => CfValueKind::Percentile,
            "formula" => CfValueKind::Formula,
            "autoMin" => CfValueKind::AutoMin,
            "autoMax" => CfValueKind::AutoMax,
            _ => CfValueKind::Number,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CfRule {
    pub kind: CfKind,
    /// Index into the workbook's `dxfs`. Absent for the visual rule kinds, which
    /// carry their own colours.
    pub dxf: Option<u32>,
    /// Lower numbers win. Excel writes them workbook-unique but does not
    /// guarantee they start at 1 or are contiguous.
    pub priority: i32,
    /// When this rule matches, no lower-priority rule is even considered.
    pub stop_if_true: bool,
}

/// A `<conditionalFormatting>` block: one `sqref` and the rules over it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalFormat {
    pub ranges: Vec<CellRange>,
    pub rules: Vec<CfRule>,
}

impl ConditionalFormat {
    pub fn covers(&self, at: crate::CellRef) -> bool {
        self.ranges.iter().any(|r| r.contains(at))
    }

    /// The top-left corner of the whole region, which is what a rule's relative
    /// references are written against.
    pub fn anchor(&self) -> Option<crate::CellRef> {
        self.ranges
            .iter()
            .map(|r| r.start)
            .reduce(|a, b| crate::CellRef::new(a.row.min(b.row), a.col.min(b.col)))
    }
}

// --- Data validation ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DvKind {
    #[default]
    None,
    Whole,
    Decimal,
    List,
    Date,
    Time,
    TextLength,
    Custom,
}

impl DvKind {
    pub fn from_xml(text: &str) -> DvKind {
        match text {
            "whole" => DvKind::Whole,
            "decimal" => DvKind::Decimal,
            "list" => DvKind::List,
            "date" => DvKind::Date,
            "time" => DvKind::Time,
            "textLength" => DvKind::TextLength,
            "custom" => DvKind::Custom,
            _ => DvKind::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DvOperator {
    #[default]
    Between,
    NotBetween,
    Equal,
    NotEqual,
    GreaterThan,
    LessThan,
    GreaterThanOrEqual,
    LessThanOrEqual,
}

impl DvOperator {
    pub fn from_xml(text: &str) -> DvOperator {
        match text {
            "notBetween" => DvOperator::NotBetween,
            "equal" => DvOperator::Equal,
            "notEqual" => DvOperator::NotEqual,
            "greaterThan" => DvOperator::GreaterThan,
            "lessThan" => DvOperator::LessThan,
            "greaterThanOrEqual" => DvOperator::GreaterThanOrEqual,
            "lessThanOrEqual" => DvOperator::LessThanOrEqual,
            _ => DvOperator::Between,
        }
    }
}

/// What a rejected entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DvSeverity {
    /// Refuses the entry.
    #[default]
    Stop,
    /// Asks, and accepts if the user insists.
    Warning,
    /// Says so and accepts.
    Information,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataValidation {
    pub ranges: Vec<CellRange>,
    pub kind: DvKind,
    pub operator: DvOperator,
    pub formula1: String,
    pub formula2: String,
    pub allow_blank: bool,
    /// `showDropDown="1"` means *hide* it. The attribute is inverted in the
    /// file, and reading it literally hides the arrow on every list in the
    /// workbook and shows one on every list that asked to hide it.
    pub show_dropdown: bool,
    pub severity: DvSeverity,
    pub error_title: String,
    pub error_message: String,
    pub prompt_title: String,
    pub prompt_message: String,
}

impl Default for DataValidation {
    fn default() -> Self {
        DataValidation {
            ranges: Vec::new(),
            kind: DvKind::None,
            operator: DvOperator::Between,
            formula1: String::new(),
            formula2: String::new(),
            allow_blank: false,
            show_dropdown: true,
            severity: DvSeverity::Stop,
            error_title: String::new(),
            error_message: String::new(),
            prompt_title: String::new(),
            prompt_message: String::new(),
        }
    }
}

impl DataValidation {
    pub fn covers(&self, at: crate::CellRef) -> bool {
        self.ranges.iter().any(|r| r.contains(at))
    }

    /// The choices of an inline list — `formula1` of `"a,b,c"`, quotes included.
    ///
    /// A list whose source is a range comes back empty here; resolving that
    /// needs the workbook, which is the caller's.
    pub fn inline_choices(&self) -> Option<Vec<String>> {
        if self.kind != DvKind::List {
            return None;
        }
        let text = self.formula1.trim();
        let inner = text.strip_prefix('"')?.strip_suffix('"')?;
        // Excel escapes a literal quote inside the list by doubling it.
        Some(
            inner
                .split(',')
                .map(|item| item.replace("\"\"", "\"").trim().to_string())
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellRef;

    fn range(a: &str, b: &str) -> CellRange {
        CellRange::new(
            CellRef::from_a1(a).expect("a1"),
            CellRef::from_a1(b).expect("a1"),
        )
    }

    #[test]
    fn an_inline_list_splits_on_commas_and_unescapes_quotes() {
        let dv = DataValidation {
            kind: DvKind::List,
            formula1: r#""Yes,No,""Maybe""""#.to_string(),
            ..Default::default()
        };
        assert_eq!(
            dv.inline_choices(),
            Some(vec![
                "Yes".to_string(),
                "No".to_string(),
                "\"Maybe\"".to_string()
            ])
        );
    }

    #[test]
    fn a_list_that_names_a_range_is_not_an_inline_list() {
        let dv = DataValidation {
            kind: DvKind::List,
            formula1: "$H$1:$H$5".to_string(),
            ..Default::default()
        };
        assert_eq!(dv.inline_choices(), None);
    }

    #[test]
    fn a_regions_anchor_is_the_corner_of_all_of_it() {
        // A rule's relative references are written against the whole region's
        // top-left cell, not against each `sqref` fragment.
        let cf = ConditionalFormat {
            ranges: vec![range("D4", "D9"), range("B2", "B3")],
            rules: Vec::new(),
        };
        assert_eq!(cf.anchor(), CellRef::from_a1("B2"));
        assert!(cf.covers(CellRef::from_a1("D5").expect("a1")));
        assert!(!cf.covers(CellRef::from_a1("C5").expect("a1")));
    }
}
