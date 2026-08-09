//! Worksheet parts: the cell grid, and the geometry around it.
//!
//! This is the hot path. A 50 MB workbook is 50 MB of `<c>` elements, so the
//! inner loop avoids allocating: addresses and style indices are parsed straight
//! out of the attribute bytes, and only actual text reaches a `String`.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use ss_model::cell::{CellError, MAX_COLS, MAX_ROWS};
use ss_model::cond::{
    CfKind, CfOperator, CfRule, CfValue, CfValueKind, ConditionalFormat, DataValidation, DvKind,
    DvOperator, DvSeverity, TextOp,
};
use ss_model::formula::{Formula, FormulaKind};
use ss_model::{Cell, CellRange, CellRef, CellValue, Sheet, StrId, StringTable, StyleId};

use crate::error::{xml_err, Result};
use crate::xml::{
    attr_f64, attr_raw, attr_text, attr_u32, attributes, end_local_name, local_name, parse_bool,
    parse_f64, parse_u32, push_text, strip_prefix,
};

/// What `t=` said the cell holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Number,
    SharedString,
    /// A formula whose result is text, stored inline in `<v>`.
    FormulaString,
    InlineString,
    Bool,
    Error,
    /// ISO-8601 date text. Only ever written by the Strict profile.
    Date,
}

impl ValueKind {
    fn from_attr(raw: Option<&[u8]>) -> Self {
        match raw {
            None | Some(b"n") => ValueKind::Number,
            Some(b"s") => ValueKind::SharedString,
            Some(b"str") => ValueKind::FormulaString,
            Some(b"inlineStr") => ValueKind::InlineString,
            Some(b"b") => ValueKind::Bool,
            Some(b"e") => ValueKind::Error,
            Some(b"d") => ValueKind::Date,
            // An unrecognized type is treated as a number, which is the schema
            // default. The part is retained verbatim either way, so a wrong guess
            // costs display fidelity, not data.
            Some(_) => ValueKind::Number,
        }
    }
}

/// Parses a worksheet into `sheet`.
///
/// `sst` maps shared-string indices to interned ids; `strings` receives any text
/// stored inline in the sheet rather than in the shared table.
pub(crate) fn parse(
    part: &str,
    data: &[u8],
    sheet: &mut Sheet,
    sst: &[StrId],
    strings: &mut StringTable,
) -> Result<Extras> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = true;

    let mut buf = Vec::new();

    // Row and column trackers. `r=` is optional on both `<row>` and `<c>`; when
    // absent the position is implied by document order, and several generators
    // (notably server-side exporters) omit it to save bytes.
    let mut row: u32 = 0;
    let mut col: u32 = 0;

    let mut cell = Cell::default();
    let mut kind = ValueKind::Number;
    let mut text = String::new();

    // Where accumulated text belongs. `<f>` and `<v>` are siblings inside `<c>`
    // and both contain character data, so the target has to be explicit.
    let mut sink = Sink::None;

    let mut formula_text = String::new();
    let mut formula_kind: Option<FormulaKind> = None;
    let mut in_cell = false;
    let mut in_phonetic = 0usize;

    // Conditional formatting and data validation are regions rather than cells,
    // and both are built up across several child elements.
    let mut extras = Extras::default();
    // Which pane of a frozen sheet was the active one. A frozen sheet writes a
    // `<selection>` per pane, and taking any but the active one puts the cursor
    // in the wrong half of the split — usually in the frozen headings.
    let mut active_pane: Vec<u8> = Vec::new();
    let mut block: Option<ConditionalFormat> = None;
    let mut rule: Option<PartialRule> = None;
    let mut validation: Option<DataValidation> = None;
    let mut side = String::new();

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;

        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                match local_name(e) {
                    b"row" => {
                        let attrs = read_row_attrs(e);
                        row = match attrs.number {
                            Some(one_based) if one_based >= 1 => one_based - 1,
                            // Absent or zero: continue after the previous row.
                            _ if sheet.cells.is_empty() && row == 0 => 0,
                            _ => row + 1,
                        };
                        col = 0;
                        apply_row_geometry(&attrs, sheet, row);
                    }
                    b"c" => {
                        // One pass over the attributes rather than three lookups.
                        // `<c>` is the hot element — a large sheet is millions of
                        // them — and each lookup re-walks the whole tag.
                        let mut style = 0u32;
                        kind = ValueKind::Number;
                        for a in attributes(e) {
                            match strip_prefix(a.key.as_ref()) {
                                b"r" => {
                                    if let Some(at) = parse_a1_bytes(&a.value) {
                                        row = at.row;
                                        col = at.col;
                                    }
                                }
                                b"s" => style = parse_u32(&a.value).unwrap_or(0),
                                b"t" => kind = ValueKind::from_attr(Some(&a.value)),
                                _ => {}
                            }
                        }
                        cell = Cell {
                            value: CellValue::Blank,
                            style: StyleId(style),
                            formula: None,
                        };
                        text.clear();
                        formula_text.clear();
                        formula_kind = None;
                        in_cell = true;

                        if empty {
                            // `<c r="B4" s="3"/>` — a styled but valueless cell.
                            // Real content: it carries borders and fills.
                            finish_cell(sheet, CellRef::new(row, col), cell, None);
                            in_cell = false;
                            col = col.saturating_add(1);
                        }
                    }
                    b"v" => sink = Sink::Value,
                    b"t" if in_cell && in_phonetic == 0 => sink = Sink::Value,
                    b"f" => {
                        formula_kind = Some(read_formula_kind(e));
                        formula_text.clear();
                        sink = Sink::Formula;
                        if empty {
                            // `<f t="shared" si="4"/>` — a follower, all attribute.
                            sink = Sink::None;
                        }
                    }
                    b"rPh" => in_phonetic += 1,
                    b"mergeCell" => {
                        if let Some(r) = attr_raw(e, b"ref") {
                            if let Some(range) = parse_range_bytes(&r) {
                                sheet.merges.push(range);
                            }
                        }
                    }
                    b"col" => read_col_geometry(e, sheet),
                    b"pane" => {
                        active_pane = attr_raw(e, b"activePane")
                            .map(|v| v.into_owned())
                            .unwrap_or_default();
                        read_pane(e, sheet);
                    }
                    b"sheetView" => read_sheet_view(e, sheet),
                    b"selection" => {
                        let pane = attr_raw(e, b"pane").unwrap_or_default();
                        // An unfrozen sheet has one selection and no pane on it.
                        let mine = pane.as_ref() == active_pane.as_slice();
                        if mine || sheet.view.selection.is_none() {
                            if let Some(at) = attr_raw(e, b"activeCell")
                                .as_deref()
                                .and_then(parse_a1_bytes)
                            {
                                sheet.view.selection = Some(at);
                            }
                        }
                    }
                    b"tabColor" => {
                        let color = crate::styles::read_color(e);
                        if color != ss_model::Color::Auto {
                            sheet.view.tab_color = Some(color);
                        }
                    }
                    // A worksheet does not name a chart. It names a *drawing*,
                    // by relationship id, and the drawing names the chart.
                    b"drawing" => {
                        extras.drawing =
                            attr_raw(e, b"id").map(|id| String::from_utf8_lossy(&id).into_owned());
                    }

                    b"conditionalFormatting" => {
                        let ranges = attr_raw(e, b"sqref")
                            .map(|raw| parse_sqref(&raw))
                            .unwrap_or_default();
                        block = Some(ConditionalFormat {
                            ranges,
                            rules: Vec::new(),
                        });
                    }
                    b"cfRule" => {
                        let started = read_cf_rule(e);
                        if empty {
                            if let (Some(b), Some(r)) = (block.as_mut(), started.finish()) {
                                b.rules.push(r);
                            }
                        } else {
                            rule = Some(started);
                        }
                    }
                    b"cfvo" => {
                        if let Some(r) = rule.as_mut() {
                            r.stops.push(CfValue {
                                kind: attr_text(e, b"type")
                                    .map_or(CfValueKind::Number, |t| CfValueKind::from_xml(&t)),
                                value: attr_text(e, b"val").unwrap_or_default(),
                            });
                        }
                    }
                    b"color" => {
                        if let Some(r) = rule.as_mut() {
                            r.colors.push(crate::styles::read_color(e));
                        }
                    }
                    b"dataBar" => {
                        if let Some(r) = rule.as_mut() {
                            r.show_value = attr_raw(e, b"showValue")
                                .and_then(|v| parse_bool(&v))
                                .unwrap_or(true);
                        }
                    }
                    b"iconSet" => {
                        if let Some(r) = rule.as_mut() {
                            r.icon_set = attr_text(e, b"iconSet").unwrap_or_default();
                            r.show_value = attr_raw(e, b"showValue")
                                .and_then(|v| parse_bool(&v))
                                .unwrap_or(true);
                            r.reverse = attr_raw(e, b"reverse")
                                .and_then(|v| parse_bool(&v))
                                .unwrap_or(false);
                        }
                    }
                    b"formula" if rule.is_some() => {
                        side.clear();
                        sink = Sink::Side;
                    }

                    b"dataValidation" => {
                        let dv = read_validation(e);
                        if empty {
                            push_validation(sheet, dv);
                        } else {
                            validation = Some(dv);
                        }
                    }
                    b"formula1" | b"formula2" if validation.is_some() => {
                        side.clear();
                        sink = Sink::Side;
                    }

                    _ => {}
                }
            }

            Event::End(ref e) => match end_local_name(e) {
                b"c" => {
                    if in_cell {
                        cell.value = build_value(kind, &text, sst, strings);
                        let formula = formula_kind
                            .take()
                            .map(|k| build_formula(k, std::mem::take(&mut formula_text)));
                        finish_cell(sheet, CellRef::new(row, col), cell, formula);
                        in_cell = false;
                        col = col.saturating_add(1);
                    }
                    sink = Sink::None;
                }
                b"v" | b"t" | b"f" => sink = Sink::None,
                b"rPh" => in_phonetic = in_phonetic.saturating_sub(1),

                b"formula" => {
                    if let Some(r) = rule.as_mut() {
                        r.formulas.push(std::mem::take(&mut side));
                    }
                    sink = Sink::None;
                }
                b"cfRule" => {
                    if let (Some(b), Some(r)) =
                        (block.as_mut(), rule.take().and_then(|r| r.finish()))
                    {
                        b.rules.push(r);
                    }
                    sink = Sink::None;
                }
                b"conditionalFormatting" => {
                    if let Some(b) = block.take() {
                        // A block with no ranges applies to nothing; one with no
                        // rules formats nothing. Neither is worth carrying.
                        if !b.ranges.is_empty() && !b.rules.is_empty() {
                            sheet.conditional_formats.push(b);
                        }
                    }
                }

                b"formula1" => {
                    if let Some(dv) = validation.as_mut() {
                        dv.formula1 = std::mem::take(&mut side);
                    }
                    sink = Sink::None;
                }
                b"formula2" => {
                    if let Some(dv) = validation.as_mut() {
                        dv.formula2 = std::mem::take(&mut side);
                    }
                    sink = Sink::None;
                }
                b"dataValidation" => {
                    if let Some(dv) = validation.take() {
                        push_validation(sheet, dv);
                    }
                }
                _ => {}
            },

            Event::Eof => break,

            ref other => match sink {
                Sink::Value if in_phonetic == 0 => {
                    push_text(&mut text, other)?;
                }
                Sink::Formula => {
                    push_text(&mut formula_text, other)?;
                }
                Sink::Side => {
                    push_text(&mut side, other)?;
                }
                _ => {}
            },
        }
        buf.clear();
    }

    Ok(extras)
}

/// What the worksheet says about parts other than itself.
#[derive(Debug, Clone, Default)]
pub(crate) struct Extras {
    /// The relationship id of `<drawing r:id="rId1"/>`.
    pub drawing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sink {
    None,
    Value,
    Formula,
    /// A `<formula>` of a conditional format, or a `<formula1>`/`<formula2>` of
    /// a validation. All three are text beside the grid rather than in it.
    Side,
}

/// A `<cfRule>` while its children are still arriving.
///
/// The rule's *type* is an attribute but its operands are elements, so nothing
/// can be decided until the element closes.
struct PartialRule {
    kind: String,
    operator: Option<CfOperator>,
    dxf: Option<u32>,
    priority: i32,
    stop_if_true: bool,
    rank: u32,
    percent: bool,
    bottom: bool,
    equal_average: bool,
    above: bool,
    std_dev: Option<i32>,
    text: String,
    time_period: String,
    icon_set: String,
    show_value: bool,
    reverse: bool,
    formulas: Vec<String>,
    stops: Vec<CfValue>,
    colors: Vec<ss_model::color::Color>,
}

impl PartialRule {
    fn finish(self) -> Option<CfRule> {
        let kind = match self.kind.as_str() {
            "cellIs" => CfKind::CellIs {
                operator: self.operator?,
                formulas: self.formulas,
            },
            "expression" => CfKind::Expression {
                formula: self.formulas.into_iter().next()?,
            },
            "colorScale" => CfKind::ColorScale {
                stops: self.stops,
                colors: self.colors,
            },
            "dataBar" => CfKind::DataBar {
                min: self.stops.first().cloned()?,
                max: self.stops.get(1).cloned()?,
                color: self.colors.first().copied().unwrap_or_default(),
                show_value: self.show_value,
            },
            "iconSet" => CfKind::IconSet {
                set: self.icon_set,
                stops: self.stops,
                show_value: self.show_value,
                reverse: self.reverse,
            },
            "top10" => CfKind::Top10 {
                rank: self.rank,
                percent: self.percent,
                bottom: self.bottom,
            },
            "aboveAverage" => CfKind::AboveAverage {
                above: self.above,
                equal_average: self.equal_average,
                std_dev: self.std_dev,
            },
            "containsText" => CfKind::Text {
                op: TextOp::Contains,
                text: self.text,
            },
            "notContainsText" => CfKind::Text {
                op: TextOp::NotContains,
                text: self.text,
            },
            "beginsWith" => CfKind::Text {
                op: TextOp::BeginsWith,
                text: self.text,
            },
            "endsWith" => CfKind::Text {
                op: TextOp::EndsWith,
                text: self.text,
            },
            "timePeriod" => CfKind::TimePeriod {
                period: self.time_period,
            },
            "duplicateValues" => CfKind::Duplicates { unique: false },
            "uniqueValues" => CfKind::Duplicates { unique: true },
            "containsBlanks" => CfKind::Presence {
                blanks: true,
                negated: false,
            },
            "notContainsBlanks" => CfKind::Presence {
                blanks: true,
                negated: true,
            },
            "containsErrors" => CfKind::Presence {
                blanks: false,
                negated: false,
            },
            "notContainsErrors" => CfKind::Presence {
                blanks: false,
                negated: true,
            },
            other => CfKind::Other(other.to_string()),
        };
        Some(CfRule {
            kind,
            dxf: self.dxf,
            priority: self.priority,
            stop_if_true: self.stop_if_true,
        })
    }
}

fn read_cf_rule(e: &BytesStart<'_>) -> PartialRule {
    let mut out = PartialRule {
        kind: String::new(),
        operator: None,
        dxf: None,
        // Excel writes a priority on every rule; a missing one is last rather
        // than first, so that a rule we could not read cannot shadow one we could.
        priority: i32::MAX,
        stop_if_true: false,
        rank: 10,
        percent: false,
        bottom: false,
        equal_average: false,
        above: true,
        std_dev: None,
        text: String::new(),
        time_period: String::new(),
        icon_set: String::new(),
        show_value: true,
        reverse: false,
        formulas: Vec::new(),
        stops: Vec::new(),
        colors: Vec::new(),
    };
    for a in attributes(e) {
        let text = || String::from_utf8_lossy(&a.value).into_owned();
        match strip_prefix(a.key.as_ref()) {
            b"type" => out.kind = text(),
            b"operator" => out.operator = CfOperator::from_xml(&text()),
            b"dxfId" => out.dxf = parse_u32(&a.value),
            b"priority" => {
                out.priority = text().parse().unwrap_or(i32::MAX);
            }
            b"stopIfTrue" => out.stop_if_true = parse_bool(&a.value).unwrap_or(false),
            b"rank" => out.rank = parse_u32(&a.value).unwrap_or(10),
            b"percent" => out.percent = parse_bool(&a.value).unwrap_or(false),
            b"bottom" => out.bottom = parse_bool(&a.value).unwrap_or(false),
            b"aboveAverage" => out.above = parse_bool(&a.value).unwrap_or(true),
            b"equalAverage" => out.equal_average = parse_bool(&a.value).unwrap_or(false),
            b"stdDev" => out.std_dev = text().parse().ok(),
            b"text" => out.text = text(),
            b"timePeriod" => out.time_period = text(),
            _ => {}
        }
    }
    out
}

fn read_validation(e: &BytesStart<'_>) -> DataValidation {
    let mut out = DataValidation::default();
    for a in attributes(e) {
        let text = || String::from_utf8_lossy(&a.value).into_owned();
        match strip_prefix(a.key.as_ref()) {
            b"sqref" => out.ranges = parse_sqref(&a.value),
            b"type" => out.kind = DvKind::from_xml(&text()),
            b"operator" => out.operator = DvOperator::from_xml(&text()),
            b"allowBlank" => out.allow_blank = parse_bool(&a.value).unwrap_or(false),
            // Inverted in the file: `showDropDown="1"` *hides* the arrow. Read
            // literally, every list in the workbook loses its dropdown.
            b"showDropDown" => out.show_dropdown = !parse_bool(&a.value).unwrap_or(false),
            b"errorStyle" => {
                out.severity = match a.value.as_ref() {
                    b"warning" => DvSeverity::Warning,
                    b"information" => DvSeverity::Information,
                    _ => DvSeverity::Stop,
                }
            }
            b"errorTitle" => out.error_title = text(),
            b"error" => out.error_message = text(),
            b"promptTitle" => out.prompt_title = text(),
            b"prompt" => out.prompt_message = text(),
            _ => {}
        }
    }
    out
}

fn push_validation(sheet: &mut Sheet, dv: DataValidation) {
    if !dv.ranges.is_empty() && dv.kind != DvKind::None {
        sheet.validations.push(dv);
    }
}

/// `sqref="A1:A5 C1 E2:G9"` — a space-separated list of ranges.
fn parse_sqref(raw: &[u8]) -> Vec<CellRange> {
    raw.split(|b| b.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(parse_range_bytes)
        .collect()
}

/// Stores a cell, unless it carries nothing at all.
fn finish_cell(sheet: &mut Sheet, at: CellRef, mut cell: Cell, formula: Option<Formula>) {
    if !at.is_valid() {
        return;
    }
    if let Some(f) = formula {
        cell.formula = Some(sheet.push_formula(f));
    }
    // A vacant cell is one the file could have omitted. Storing it would inflate
    // the sparse grid for no gain — but note that a *styled* empty cell is not
    // vacant, so this does not silently strip formatting.
    if cell.is_vacant() {
        return;
    }
    sheet.cells.set(at, cell);
}

fn build_value(kind: ValueKind, text: &str, sst: &[StrId], strings: &mut StringTable) -> CellValue {
    match kind {
        ValueKind::Number => match text.trim() {
            "" => CellValue::Blank,
            n => n.parse::<f64>().map(CellValue::Number).unwrap_or_else(|_| {
                // Not a number despite claiming to be. Keeping the characters is
                // better than a silent zero, which would look like real data.
                CellValue::Text(strings.intern(n))
            }),
        },
        ValueKind::SharedString => match parse_u32(text.trim().as_bytes()) {
            Some(i) => sst
                .get(i as usize)
                .copied()
                .map(CellValue::Text)
                // An index past the table means the shared strings and the sheet
                // disagree. Blank is the honest answer; inventing text is not.
                .unwrap_or(CellValue::Blank),
            None => CellValue::Blank,
        },
        ValueKind::FormulaString | ValueKind::InlineString | ValueKind::Date => {
            if text.is_empty() {
                CellValue::Blank
            } else {
                CellValue::Text(strings.intern(text))
            }
        }
        ValueKind::Bool => match text.trim() {
            "1" | "true" | "TRUE" => CellValue::Bool(true),
            "0" | "false" | "FALSE" => CellValue::Bool(false),
            "" => CellValue::Blank,
            other => CellValue::Text(strings.intern(other)),
        },
        ValueKind::Error => CellError::from_code(text.trim())
            .map(CellValue::Error)
            .unwrap_or_else(|| CellValue::Text(strings.intern(text.trim()))),
    }
}

fn read_formula_kind(e: &BytesStart<'_>) -> FormulaKind {
    // One pass: `<f>` appears on every formula cell, of which a calculation-heavy
    // sheet is almost entirely composed.
    let mut range = None;
    let mut t: Option<Vec<u8>> = None;
    let mut si = 0u32;
    for a in attributes(e) {
        match strip_prefix(a.key.as_ref()) {
            b"ref" => range = parse_range_bytes(&a.value),
            b"t" => t = Some(a.value.into_owned()),
            b"si" => si = parse_u32(&a.value).unwrap_or(0),
            _ => {}
        }
    }
    match t.as_deref() {
        Some(b"array") => match range {
            Some(range) => FormulaKind::Array { range },
            // An array formula must declare its range; without one it covers
            // only its own cell, which is what Excel assumes too.
            None => FormulaKind::Normal,
        },
        // The master is the one carrying a range. Followers have only `si`, and
        // their text has to be translated from the master's.
        Some(b"shared") => match range {
            Some(range) => FormulaKind::Shared {
                index: si,
                range: Some(range),
            },
            None => FormulaKind::SharedFollower { index: si },
        },
        Some(b"dataTable") => FormulaKind::DataTable,
        _ => FormulaKind::Normal,
    }
}

fn build_formula(kind: FormulaKind, text: String) -> Formula {
    // A shared master written without a range is indistinguishable from a
    // follower by attributes alone, so text is the tiebreaker: only the master
    // has any.
    let kind = match kind {
        FormulaKind::SharedFollower { index } if !text.is_empty() => {
            FormulaKind::Shared { index, range: None }
        }
        other => other,
    };
    Formula { text, kind }
}

/// Everything `<row>` carries, read in one pass.
struct RowAttrs {
    number: Option<u32>,
    height: Option<f64>,
    custom_height: bool,
    hidden: bool,
    /// `s` plus `customFormat="1"`: a style for every cell in the row that has
    /// none of its own. Without `customFormat` the `s` is advisory and Excel
    /// ignores it.
    style: Option<StyleId>,
}

fn read_row_attrs(e: &BytesStart<'_>) -> RowAttrs {
    let mut out = RowAttrs {
        number: None,
        height: None,
        custom_height: false,
        hidden: false,
        style: None,
    };
    let mut style = None;
    let mut custom_format = false;
    for a in attributes(e) {
        match strip_prefix(a.key.as_ref()) {
            b"r" => out.number = parse_u32(&a.value),
            b"ht" => out.height = parse_f64(&a.value),
            b"customHeight" => out.custom_height = parse_bool(&a.value).unwrap_or(false),
            b"hidden" => out.hidden = parse_bool(&a.value).unwrap_or(false),
            b"s" => style = parse_u32(&a.value).map(StyleId),
            b"customFormat" => custom_format = parse_bool(&a.value).unwrap_or(false),
            _ => {}
        }
    }
    if custom_format {
        out.style = style.filter(|s| *s != StyleId::DEFAULT);
    }
    out
}

fn apply_row_geometry(attrs: &RowAttrs, sheet: &mut Sheet, row: u32) {
    // Only a *custom* height is stored. Recording auto-fit heights would freeze
    // every row at whatever the producing version of Excel measured.
    if attrs.custom_height {
        if let Some(h) = attrs.height {
            sheet.row_heights.insert(row, h);
        }
    }
    if attrs.hidden {
        sheet.row_heights.insert(row, 0.0);
    }
    if let Some(style) = attrs.style {
        sheet.row_styles.insert(row, style);
    }
}

fn read_col_geometry(e: &BytesStart<'_>, sheet: &mut Sheet) {
    // `<col min="2" max="9" width="12.5"/>` sets a whole span at once.
    let (Some(min), Some(max)) = (attr_u32(e, b"min"), attr_u32(e, b"max")) else {
        return;
    };
    // A `<col>` may carry a style without a width — that is how a whole column
    // gets shaded, and it is the only record of it anywhere in the file.
    let width = attr_f64(e, b"width");
    let style = attr_u32(e, b"style")
        .map(StyleId)
        .filter(|s| *s != StyleId::DEFAULT);
    if width.is_none() && style.is_none() {
        return;
    }
    if min == 0 || min > max {
        return;
    }
    // `max` is routinely 16384 to mean "the rest of the sheet". Materializing a
    // width for every one of those columns would allocate 16k entries per file.
    let last = max
        .min(min.saturating_add(MAX_COLS_SPAN_LIMIT))
        .min(MAX_COLS);
    for c in min..=last {
        if let Some(width) = width {
            sheet.column_widths.insert(c - 1, width);
        }
        if let Some(style) = style {
            sheet.column_styles.insert(c - 1, style);
        }
    }
}

/// How many columns a single `<col>` span may materialize.
///
/// Excel writes `max="16384"` for a sheet-wide default; storing that as 16k
/// explicit widths costs more than the entire rest of a small workbook.
const MAX_COLS_SPAN_LIMIT: u32 = 1024;

/// `<sheetView>`: the zoom, and what the sheet chooses not to draw.
///
/// `showGridLines="0"` is not decoration. A sheet laid out as an invoice or a
/// dashboard turns the lines off and fills the cells instead; drawing them
/// anyway puts a grid through the middle of somebody's letterhead.
fn read_sheet_view(e: &BytesStart<'_>, sheet: &mut Sheet) {
    // `zoomScaleNormal` is the zoom of the *normal* view, which is the one
    // being opened; `zoomScale` may belong to page-break preview.
    if let Some(scale) = attr_u32(e, b"zoomScaleNormal").or_else(|| attr_u32(e, b"zoomScale")) {
        if (10..=400).contains(&scale) {
            sheet.view.zoom = f64::from(scale) / 100.0;
        }
    }
    for (name, slot) in [
        (&b"showGridLines"[..], &mut sheet.view.gridlines),
        (&b"showRowColHeaders"[..], &mut sheet.view.headings),
    ] {
        if let Some(raw) = attr_raw(e, name) {
            if let Some(on) = parse_bool(&raw) {
                *slot = on;
            }
        }
    }
    if let Some(at) = attr_raw(e, b"topLeftCell")
        .as_deref()
        .and_then(parse_a1_bytes)
    {
        sheet.view.top_left = Some(at);
    }
}

fn read_pane(e: &BytesStart<'_>, sheet: &mut Sheet) {
    // Only frozen panes pin content. A `split` pane is a scrolling convenience
    // and its position is in twips, not cells.
    let frozen = matches!(
        attr_raw(e, b"state").as_deref(),
        Some(b"frozen") | Some(b"frozenSplit")
    );
    if !frozen {
        return;
    }
    // On a frozen sheet this is the first cell of the *scrolling* pane, which
    // is what the scroll offset means here too — the frozen bands are not part
    // of it. It sits on `<pane>` rather than on `<sheetView>` whenever there is
    // a pane at all.
    if let Some(at) = attr_raw(e, b"topLeftCell")
        .as_deref()
        .and_then(parse_a1_bytes)
    {
        sheet.view.top_left = Some(at);
    }
    let x = attr_u32(e, b"xSplit").unwrap_or(0);
    let y = attr_u32(e, b"ySplit").unwrap_or(0);
    if x == 0 && y == 0 {
        return;
    }
    sheet.frozen = Some(CellRef::new(y.min(MAX_ROWS - 1), x.min(MAX_COLS - 1)));
}

/// Parses `A1` straight from attribute bytes, with no intermediate `String`.
fn parse_a1_bytes(bytes: &[u8]) -> Option<CellRef> {
    let mut i = 0;
    if bytes.first() == Some(&b'$') {
        i += 1;
    }
    let mut col: u32 = 0;
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        let digit = (bytes[i].to_ascii_uppercase() - b'A') as u32 + 1;
        col = col.checked_mul(26)?.checked_add(digit)?;
        if col > MAX_COLS {
            return None;
        }
        i += 1;
    }
    if i == start {
        return None;
    }
    if bytes.get(i) == Some(&b'$') {
        i += 1;
    }
    let row_one_based = parse_u32(bytes.get(i..)?)?;
    if row_one_based == 0 || row_one_based > MAX_ROWS {
        return None;
    }
    Some(CellRef::new(row_one_based - 1, col - 1))
}

/// Parses `A1:D9`, and a bare `A1` as a one-cell range.
pub(crate) fn parse_range_bytes(bytes: &[u8]) -> Option<CellRange> {
    match bytes.iter().position(|&b| b == b':') {
        Some(i) => {
            let a = parse_a1_bytes(&bytes[..i])?;
            let b = parse_a1_bytes(&bytes[i + 1..])?;
            Some(CellRange::new(a, b))
        }
        None => {
            let a = parse_a1_bytes(bytes)?;
            Some(CellRange::new(a, a))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(xml: &str) -> (Sheet, StringTable) {
        read_with(xml, &[])
    }

    fn read_with(xml: &str, shared: &[&str]) -> (Sheet, StringTable) {
        let mut strings = StringTable::new();
        let sst: Vec<StrId> = shared.iter().map(|s| strings.intern(s)).collect();
        let mut sheet = Sheet::new("S");
        parse("sheet1.xml", xml.as_bytes(), &mut sheet, &sst, &mut strings).expect("parses");
        (sheet, strings)
    }

    fn value_at(sheet: &Sheet, a1: &str) -> CellValue {
        sheet
            .get(CellRef::from_a1(a1).expect("valid address"))
            .map(|c| c.value)
            .unwrap_or(CellValue::Blank)
    }

    #[test]
    fn conditional_formatting_comes_back_with_its_regions_and_its_order() {
        let (sheet, _) = read(
            r#"<worksheet><sheetData/>
              <conditionalFormatting sqref="B2:B10 D2:D10">
                <cfRule type="cellIs" dxfId="0" priority="2" operator="greaterThan">
                  <formula>100</formula></cfRule>
                <cfRule type="expression" dxfId="1" priority="1" stopIfTrue="1">
                  <formula>MOD(ROW(),2)=0</formula></cfRule>
              </conditionalFormatting>
              <conditionalFormatting sqref="F1:F9">
                <cfRule type="colorScale" priority="3">
                  <colorScale><cfvo type="min"/><cfvo type="max"/>
                    <color rgb="FFF8696B"/><color rgb="FF63BE7B"/></colorScale></cfRule>
              </conditionalFormatting>
            </worksheet>"#,
        );

        assert_eq!(sheet.conditional_formats.len(), 2);
        let first = &sheet.conditional_formats[0];
        assert_eq!(first.ranges.len(), 2, "sqref is a space-separated list");
        assert!(first.covers(CellRef::from_a1("D5").expect("a1")));
        assert!(!first.covers(CellRef::from_a1("C5").expect("a1")));
        assert_eq!(first.rules.len(), 2);
        assert_eq!(
            first.rules[0].kind,
            CfKind::CellIs {
                operator: CfOperator::GreaterThan,
                formulas: vec!["100".to_string()],
            }
        );
        assert!(first.rules[1].stop_if_true);
        assert_eq!(first.rules[1].dxf, Some(1));

        match &sheet.conditional_formats[1].rules[0].kind {
            CfKind::ColorScale { stops, colors } => {
                assert_eq!(stops.len(), 2);
                assert_eq!(colors.len(), 2);
                assert_eq!(stops[0].kind, CfValueKind::Min);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_data_validation_list_keeps_its_choices_and_its_dropdown() {
        let (sheet, _) = read(
            r#"<worksheet><sheetData/>
              <dataValidation type="list" allowBlank="1" showInputMessage="1"
                errorTitle="No" error="Pick one" sqref="A1:A20">
                <formula1>"North,South,East,West"</formula1></dataValidation>
              <dataValidation type="whole" operator="between" showDropDown="1" sqref="C1">
                <formula1>1</formula1><formula2>10</formula2></dataValidation>
            </worksheet>"#,
        );

        assert_eq!(sheet.validations.len(), 2);
        let list = sheet
            .validation_at(CellRef::from_a1("A7").expect("a1"))
            .expect("covered");
        assert_eq!(list.kind, DvKind::List);
        assert!(list.allow_blank);
        assert_eq!(list.error_message, "Pick one");
        assert_eq!(
            list.inline_choices(),
            Some(vec![
                "North".to_string(),
                "South".to_string(),
                "East".to_string(),
                "West".to_string()
            ])
        );
        assert!(list.show_dropdown, "absent means shown");

        let whole = sheet
            .validation_at(CellRef::from_a1("C1").expect("a1"))
            .expect("covered");
        assert_eq!(whole.formula1, "1");
        assert_eq!(whole.formula2, "10");
        assert!(
            !whole.show_dropdown,
            "showDropDown=\"1\" hides it; the attribute is inverted in the file"
        );
    }

    #[test]
    fn a_column_style_reaches_cells_the_file_never_stored() {
        // Shading a whole column puts a style on `<col>` and stores no cells at
        // all. A grid reading only `Cell::style` draws it unshaded.
        let (sheet, _) = read(
            r#"<worksheet>
              <cols><col min="2" max="3" style="4"/><col min="5" max="5" width="20" customWidth="1"/></cols>
              <sheetData>
                <row r="1" s="7" customFormat="1"><c r="A1"><v>1</v></c><c r="B1" s="9"><v>2</v></c></row>
              </sheetData></worksheet>"#,
        );

        assert_eq!(
            sheet.style_at(CellRef::from_a1("B4").expect("a1")),
            StyleId(4)
        );
        assert_eq!(
            sheet.style_at(CellRef::from_a1("D4").expect("a1")),
            StyleId::DEFAULT
        );
        assert_eq!(
            sheet.style_at(CellRef::from_a1("A1").expect("a1")),
            StyleId(7),
            "the row's style, since the cell has none of its own"
        );
        assert_eq!(
            sheet.style_at(CellRef::from_a1("B1").expect("a1")),
            StyleId(9),
            "the cell's own style wins over both"
        );
        assert!(
            sheet.column_widths.contains_key(&4),
            "a width-only col still sets a width"
        );
        assert!(!sheet.column_styles.contains_key(&4));
    }

    #[test]
    fn a1_parses_from_bytes_at_the_corners() {
        assert_eq!(parse_a1_bytes(b"A1"), Some(CellRef::new(0, 0)));
        assert_eq!(parse_a1_bytes(b"$A$1"), Some(CellRef::new(0, 0)));
        assert_eq!(
            parse_a1_bytes(b"XFD1048576"),
            Some(CellRef::new(MAX_ROWS - 1, MAX_COLS - 1))
        );
        assert_eq!(parse_a1_bytes(b"XFE1"), None, "past the last column");
        assert_eq!(parse_a1_bytes(b"A1048577"), None, "past the last row");
        assert_eq!(parse_a1_bytes(b"A0"), None);
        assert_eq!(parse_a1_bytes(b"1A"), None);
        assert_eq!(parse_a1_bytes(b""), None);
    }

    #[test]
    fn numbers_booleans_and_errors_read_as_themselves() {
        let (sheet, _) = read(
            r#"<worksheet><sheetData><row r="1">
                 <c r="A1"><v>42</v></c>
                 <c r="B1"><v>-3.5e2</v></c>
                 <c r="C1" t="b"><v>1</v></c>
                 <c r="D1" t="b"><v>0</v></c>
                 <c r="E1" t="e"><v>#DIV/0!</v></c>
                 <c r="F1" t="e"><v>#N/A</v></c>
               </row></sheetData></worksheet>"#,
        );
        assert_eq!(value_at(&sheet, "A1"), CellValue::Number(42.0));
        assert_eq!(value_at(&sheet, "B1"), CellValue::Number(-350.0));
        assert_eq!(value_at(&sheet, "C1"), CellValue::Bool(true));
        assert_eq!(value_at(&sheet, "D1"), CellValue::Bool(false));
        assert_eq!(value_at(&sheet, "E1"), CellValue::Error(CellError::Div0));
        assert_eq!(
            value_at(&sheet, "F1"),
            CellValue::Error(CellError::NotAvailable)
        );
    }

    #[test]
    fn shared_strings_resolve_through_the_index_table() {
        let (sheet, strings) = read_with(
            r#"<worksheet><sheetData><row r="1">
                 <c r="A1" t="s"><v>1</v></c>
                 <c r="B1" t="s"><v>0</v></c>
               </row></sheetData></worksheet>"#,
            &["Alpha", "Beta"],
        );
        match value_at(&sheet, "A1") {
            CellValue::Text(id) => assert_eq!(strings.resolve(id), "Beta"),
            other => panic!("expected text, got {other:?}"),
        }
        match value_at(&sheet, "B1") {
            CellValue::Text(id) => assert_eq!(strings.resolve(id), "Alpha"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_shared_string_index_past_the_table_does_not_invent_text() {
        let (sheet, _) = read_with(
            r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>99</v></c></row></sheetData></worksheet>"#,
            &["only"],
        );
        assert_eq!(value_at(&sheet, "A1"), CellValue::Blank);
    }

    #[test]
    fn inline_strings_read_from_the_is_element() {
        let (sheet, strings) = read(
            r#"<worksheet><sheetData><row r="1">
                 <c r="A1" t="inlineStr"><is><t>inline text</t></is></c>
                 <c r="B1" t="inlineStr"><is><r><t>two </t></r><r><t>runs</t></r></is></c>
               </row></sheetData></worksheet>"#,
        );
        match value_at(&sheet, "A1") {
            CellValue::Text(id) => assert_eq!(strings.resolve(id), "inline text"),
            other => panic!("expected text, got {other:?}"),
        }
        match value_at(&sheet, "B1") {
            CellValue::Text(id) => assert_eq!(strings.resolve(id), "two runs"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn formula_text_and_result_are_both_kept() {
        let (sheet, _) = read(
            r#"<worksheet><sheetData><row r="1">
                 <c r="A1"><f>SUM(B1:B9)</f><v>45</v></c>
               </row></sheetData></worksheet>"#,
        );
        assert_eq!(value_at(&sheet, "A1"), CellValue::Number(45.0));
        let f = sheet
            .formula_at(CellRef::from_a1("A1").unwrap())
            .expect("formula stored");
        assert_eq!(f.text, "SUM(B1:B9)");
        assert_eq!(f.kind, FormulaKind::Normal);
    }

    #[test]
    fn formula_text_does_not_leak_into_the_value() {
        // <f> and <v> are siblings; a reader with one text accumulator produces
        // "SUM(B1:B9)45" and calls it a number.
        let (sheet, _) = read(
            r#"<worksheet><sheetData><row r="1">
                 <c r="A1" t="str"><f>CONCAT("a")</f><v>a</v></c>
               </row></sheetData></worksheet>"#,
        );
        match value_at(&sheet, "A1") {
            CellValue::Text(_) => {}
            other => panic!("expected text, got {other:?}"),
        }
        let f = sheet.formula_at(CellRef::from_a1("A1").unwrap()).unwrap();
        assert_eq!(f.text, r#"CONCAT("a")"#);
    }

    #[test]
    fn shared_formula_masters_and_followers_are_distinguished() {
        let (sheet, _) = read(
            r#"<worksheet><sheetData>
                 <row r="1"><c r="A1"><f t="shared" ref="A1:A3" si="0">B1*2</f><v>2</v></c></row>
                 <row r="2"><c r="A2"><f t="shared" si="0"/><v>4</v></c></row>
                 <row r="3"><c r="A3"><f t="shared" si="0"/><v>6</v></c></row>
               </sheetData></worksheet>"#,
        );
        let master = sheet.formula_at(CellRef::from_a1("A1").unwrap()).unwrap();
        assert_eq!(master.text, "B1*2");
        assert!(matches!(master.kind, FormulaKind::Shared { index: 0, .. }));

        for a1 in ["A2", "A3"] {
            let f = sheet
                .formula_at(CellRef::from_a1(a1).unwrap())
                .unwrap_or_else(|| panic!("{a1} should keep its group membership"));
            assert_eq!(f.kind, FormulaKind::SharedFollower { index: 0 });
            assert!(f.borrows_text());
        }
    }

    #[test]
    fn array_formulas_keep_their_range() {
        let (sheet, _) = read(
            r#"<worksheet><sheetData><row r="1">
                 <c r="D1"><f t="array" ref="D1:D10">SUM(A1:A10*B1:B10)</f><v>385</v></c>
               </row></sheetData></worksheet>"#,
        );
        let f = sheet.formula_at(CellRef::from_a1("D1").unwrap()).unwrap();
        match &f.kind {
            FormulaKind::Array { range } => {
                assert_eq!(range.start, CellRef::new(0, 3));
                assert_eq!(range.end, CellRef::new(9, 3));
            }
            other => panic!("expected an array formula, got {other:?}"),
        }
    }

    #[test]
    fn a_styled_empty_cell_survives() {
        // Losing these strips borders and fills from otherwise-blank cells.
        let (sheet, _) = read(
            r#"<worksheet><sheetData><row r="1">
                 <c r="A1" s="7"/>
                 <c r="B1"/>
               </row></sheetData></worksheet>"#,
        );
        let styled = sheet.get(CellRef::new(0, 0)).expect("styled cell kept");
        assert_eq!(styled.style, StyleId(7));
        assert!(styled.value.is_blank());
        assert!(
            sheet.get(CellRef::new(0, 1)).is_none(),
            "a cell with nothing at all need not be stored"
        );
    }

    #[test]
    fn cells_without_addresses_fall_into_document_order() {
        // Some exporters omit r= entirely to save bytes.
        let (sheet, _) = read(
            r#"<worksheet><sheetData>
                 <row r="1"><c><v>1</v></c><c><v>2</v></c><c><v>3</v></c></row>
                 <row r="2"><c><v>4</v></c></row>
               </sheetData></worksheet>"#,
        );
        assert_eq!(value_at(&sheet, "A1"), CellValue::Number(1.0));
        assert_eq!(value_at(&sheet, "B1"), CellValue::Number(2.0));
        assert_eq!(value_at(&sheet, "C1"), CellValue::Number(3.0));
        assert_eq!(value_at(&sheet, "A2"), CellValue::Number(4.0));
    }

    #[test]
    fn merges_read_and_normalize() {
        let (sheet, _) = read(
            r#"<worksheet><mergeCells count="2">
                 <mergeCell ref="A1:D1"/>
                 <mergeCell ref="B5:B9"/>
               </mergeCells></worksheet>"#,
        );
        assert_eq!(sheet.merges.len(), 2);
        assert_eq!(sheet.merges[0].start, CellRef::new(0, 0));
        assert_eq!(sheet.merges[0].end, CellRef::new(0, 3));
        assert!(sheet.merge_at(CellRef::new(6, 1)).is_some());
    }

    #[test]
    fn the_view_is_read_from_the_active_pane_and_not_from_the_first_one() {
        // A frozen sheet writes one `<selection>` per pane. Taking the first
        // puts the cursor in the frozen headings rather than where the user
        // left it, which is what this file does: G1 is the top-right pane's
        // idea of a selection, and B5 is the real one.
        let (sheet, _) = read(
            r#"<worksheet><sheetViews><sheetView zoomScale="90" zoomScaleNormal="90">
                 <pane xSplit="5" ySplit="3" topLeftCell="F4" activePane="bottomRight" state="frozenSplit"/>
                 <selection pane="topRight" activeCell="G1" sqref="G1"/>
                 <selection pane="bottomLeft" activeCell="A9" sqref="A9"/>
                 <selection pane="bottomRight" activeCell="B5" sqref="B5"/>
               </sheetView></sheetViews></worksheet>"#,
        );
        assert_eq!(sheet.view.selection, Some(CellRef::from_a1("B5").unwrap()));
        assert_eq!(sheet.view.zoom, 0.9);
        assert_eq!(sheet.view.top_left, Some(CellRef::from_a1("F4").unwrap()));
        assert_eq!(sheet.frozen, Some(CellRef::new(3, 5)));
    }

    #[test]
    fn a_sheet_that_hides_its_gridlines_says_so() {
        let (sheet, _) = read(
            r#"<worksheet><sheetViews>
                 <sheetView showGridLines="0" workbookViewId="0"/>
               </sheetViews></worksheet>"#,
        );
        assert!(!sheet.view.gridlines);
        assert!(sheet.view.headings, "headings were not turned off");
    }

    #[test]
    fn frozen_panes_read_but_split_panes_do_not() {
        let (frozen, _) = read(
            r#"<worksheet><sheetViews><sheetView>
                 <pane xSplit="1" ySplit="2" topLeftCell="B3" state="frozen"/>
               </sheetView></sheetViews></worksheet>"#,
        );
        assert_eq!(frozen.frozen, Some(CellRef::new(2, 1)));

        let (split, _) = read(
            r#"<worksheet><sheetViews><sheetView>
                 <pane xSplit="1440" ySplit="720" topLeftCell="B3" state="split"/>
               </sheetView></sheetViews></worksheet>"#,
        );
        assert_eq!(split.frozen, None, "a split pane pins nothing");
    }

    #[test]
    fn column_widths_apply_across_their_span() {
        let (sheet, _) = read(
            r#"<worksheet><cols>
                 <col min="2" max="4" width="12.5" customWidth="1"/>
               </cols></worksheet>"#,
        );
        assert_eq!(sheet.column_widths.get(&1), Some(&12.5));
        assert_eq!(sheet.column_widths.get(&3), Some(&12.5));
        assert_eq!(sheet.column_widths.get(&0), None);
        assert_eq!(sheet.column_widths.get(&4), None);
    }

    #[test]
    fn a_sheet_wide_column_span_does_not_materialize_16k_entries() {
        let (sheet, _) = read(
            r#"<worksheet><cols><col min="1" max="16384" width="9.140625"/></cols></worksheet>"#,
        );
        assert!(
            sheet.column_widths.len() <= 1025,
            "span was materialized in full: {} entries",
            sheet.column_widths.len()
        );
    }

    #[test]
    fn phonetic_runs_inside_cells_are_not_cell_text() {
        let xml = "<worksheet><sheetData><row r=\"1\">\
             <c r=\"A1\" t=\"inlineStr\"><is>\
             <t>\u{5C71}\u{7530}</t>\
             <rPh sb=\"0\" eb=\"2\"><t>\u{3084}\u{307E}\u{3060}</t></rPh>\
             </is></c></row></sheetData></worksheet>";
        let (sheet, strings) = read(xml);
        match value_at(&sheet, "A1") {
            CellValue::Text(id) => assert_eq!(strings.resolve(id), "\u{5C71}\u{7530}"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_number_that_is_not_a_number_keeps_its_characters() {
        // Silently becoming 0.0 would be indistinguishable from real data.
        let (sheet, strings) = read(
            r#"<worksheet><sheetData><row r="1"><c r="A1"><v>not-a-number</v></c></row></sheetData></worksheet>"#,
        );
        match value_at(&sheet, "A1") {
            CellValue::Text(id) => assert_eq!(strings.resolve(id), "not-a-number"),
            other => panic!("expected the text preserved, got {other:?}"),
        }
    }

    #[test]
    fn rows_and_cells_out_of_range_are_dropped_rather_than_wrapped() {
        let (sheet, _) = read(
            r#"<worksheet><sheetData><row r="1">
                 <c r="A1"><v>1</v></c>
               </row></sheetData></worksheet>"#,
        );
        assert_eq!(sheet.cells.len(), 1);
    }

    #[test]
    fn prefixed_worksheets_parse_the_same() {
        let (sheet, _) = read(
            r#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
                 <x:sheetData><x:row r="1"><x:c r="A1"><x:v>7</x:v></x:c></x:row></x:sheetData>
               </x:worksheet>"#,
        );
        assert_eq!(value_at(&sheet, "A1"), CellValue::Number(7.0));
    }
}
