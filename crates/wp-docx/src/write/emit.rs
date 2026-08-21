//! Serializing the model back to WordprocessingML.
//!
//! Only for the paragraphs a save actually changed — everything else is copied
//! byte for byte by the splicer. That is what makes this file's one hard
//! constraint survivable:
//!
//! **The children of `<w:pPr>` and `<w:rPr>` have a fixed order, and Word calls
//! a document with them out of order damaged.** The schema is a sequence, not a
//! choice, so `<w:sz>` before `<w:color>` is not a formatting difference — it is
//! a file that will not open. The two `ORDER` tables below are that sequence,
//! and every property is emitted by walking them rather than by writing whatever
//! comes to hand.

use std::fmt::Write as _;

use wp_model::color::Color;
use wp_model::doc::{Block, Break, Inline, Paragraph, Piece, Run};
use wp_model::prop::{
    Border, Justify, LineSpacing, ParaProps, RunProps, Shading, ShadingPattern, TabKind, TabLeader,
    Toggle, UnderlineKind, VertAlign,
};
use wp_model::style::StyleTable;
use wp_model::table::{
    Cell, CellMargins, CellProps, CellVAlign, FloatAnchor, Row, RowHeight, RowProps, Table,
    TableBorders, TableLayout, TableLook, TableProps, TextDirection, VMerge, Width,
};

use super::splice::{escape_attr, escape_text, needs_space_preserve};

/// The order `<w:rPr>`'s children must appear in.
const RUN_ORDER: [&str; 24] = [
    "rStyle",
    "rFonts",
    "b",
    "bCs",
    "i",
    "iCs",
    "caps",
    "smallCaps",
    "strike",
    "dstrike",
    "outline",
    "shadow",
    "emboss",
    "imprint",
    "noProof",
    "vanish",
    "color",
    "spacing",
    "w",
    "kern",
    "position",
    "sz",
    "szCs",
    "highlight",
];

/// The rest of `<w:rPr>`, after `highlight`. Split only because Rust arrays are
/// sized and one list of thirty reads worse than two.
const RUN_ORDER_TAIL: [&str; 6] = ["u", "bdr", "shd", "vertAlign", "rtl", "lang"];

/// The order `<w:pPr>`'s children must appear in.
const PARA_ORDER: [&str; 16] = [
    "pStyle",
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "widowControl",
    "numPr",
    "suppressLineNumbers",
    "pBdr",
    "shd",
    "tabs",
    "bidi",
    "spacing",
    "ind",
    "contextualSpacing",
    "jc",
    "textAlignment",
];

/// Writes a `<w:p>` element.
pub(crate) fn paragraph(out: &mut String, paragraph: &Paragraph, styles: &StyleTable) {
    out.push_str("<w:p");
    if let Some(id) = paragraph.id {
        let _ = write!(out, r#" w14:paraId="{id:08X}""#);
    }
    if let Some(id) = paragraph.text_id {
        let _ = write!(out, r#" w14:textId="{id:08X}""#);
    }
    out.push('>');
    para_props(out, paragraph, styles);
    for inline in &paragraph.content {
        self::inline(out, inline, styles);
    }
    out.push_str("</w:p>");
}

fn para_props(out: &mut String, paragraph: &Paragraph, styles: &StyleTable) {
    let props = &paragraph.props;
    let has_mark = props.mark.as_ref().is_some_and(|mark| !mark.is_empty());
    if props.is_empty() && paragraph.section.is_none() && !has_mark {
        return;
    }
    out.push_str("<w:pPr>");
    for name in PARA_ORDER {
        para_prop(out, name, props, styles);
    }
    // `<w:rPr>` for the paragraph mark, then `<w:sectPr>`, and in that order:
    // they are the last two children of the sequence.
    if let Some(mark) = &props.mark {
        if !mark.is_empty() {
            run_props(out, mark, styles);
        }
    }
    if let Some(section) = &paragraph.section {
        super::section(out, section);
    }
    out.push_str("</w:pPr>");
}

fn on_off(out: &mut String, name: &str, value: Option<bool>) {
    match value {
        // A bare element is true, which is how Word writes it.
        Some(true) => {
            let _ = write!(out, "<w:{name}/>");
        }
        Some(false) => {
            let _ = write!(out, r#"<w:{name} w:val="0"/>"#);
        }
        // Absent means inherit, and writing nothing is the only way to say that.
        None => {}
    }
}

fn para_prop(out: &mut String, name: &str, props: &ParaProps, styles: &StyleTable) {
    match name {
        "pStyle" => {
            if let Some(id) = props.style.and_then(|id| styles.get(id)) {
                let _ = write!(out, r#"<w:pStyle w:val="{}"/>"#, escape_attr(&id.id));
            }
        }
        "keepNext" => on_off(out, "keepNext", props.keep_next),
        "keepLines" => on_off(out, "keepLines", props.keep_lines),
        "pageBreakBefore" => on_off(out, "pageBreakBefore", props.page_break_before),
        "widowControl" => on_off(out, "widowControl", props.widow_control),
        "numPr" => {
            if let Some(reference) = props.numbering {
                let _ = write!(
                    out,
                    r#"<w:numPr><w:ilvl w:val="{}"/><w:numId w:val="{}"/></w:numPr>"#,
                    reference.level, reference.num_id
                );
            }
        }
        "suppressLineNumbers" => on_off(out, "suppressLineNumbers", props.suppress_line_numbers),
        "pBdr" => {
            if let Some(borders) = &props.borders {
                out.push_str("<w:pBdr>");
                for (element, edge) in [
                    ("top", borders.top),
                    ("left", borders.start),
                    ("bottom", borders.bottom),
                    ("right", borders.end),
                    ("between", borders.between),
                    ("bar", borders.bar),
                ] {
                    if let Some(edge) = edge {
                        border(out, element, &edge);
                    }
                }
                out.push_str("</w:pBdr>");
            }
        }
        "shd" => {
            if let Some(value) = props.shading {
                shading(out, &value);
            }
        }
        "tabs" => {
            if let Some(stops) = &props.tabs {
                if !stops.is_empty() {
                    out.push_str("<w:tabs>");
                    for stop in stops {
                        let kind = match stop.kind {
                            TabKind::Start => "left",
                            TabKind::Center => "center",
                            TabKind::End => "right",
                            TabKind::Decimal => "decimal",
                            TabKind::Bar => "bar",
                            TabKind::Clear => "clear",
                        };
                        let _ = write!(out, r#"<w:tab w:val="{kind}""#);
                        if stop.leader != TabLeader::None {
                            let leader = match stop.leader {
                                TabLeader::Dot => "dot",
                                TabLeader::Hyphen => "hyphen",
                                TabLeader::Underscore => "underscore",
                                TabLeader::Heavy => "heavy",
                                TabLeader::MiddleDot => "middleDot",
                                TabLeader::None => unreachable!(),
                            };
                            let _ = write!(out, r#" w:leader="{leader}""#);
                        }
                        let _ = write!(out, r#" w:pos="{}"/>"#, stop.position.0);
                    }
                    out.push_str("</w:tabs>");
                }
            }
        }
        "bidi" => on_off(out, "bidi", props.bidi),
        "spacing" => {
            let spacing = &props.spacing;
            if spacing.is_empty() {
                return;
            }
            out.push_str("<w:spacing");
            if let Some(before) = spacing.before {
                let _ = write!(out, r#" w:before="{}""#, before.0);
            }
            if let Some(after) = spacing.after {
                let _ = write!(out, r#" w:after="{}""#, after.0);
            }
            if let Some(line) = spacing.line {
                let (value, rule) = match line {
                    LineSpacing::Multiple(n) => (n.0, "auto"),
                    LineSpacing::AtLeast(t) => (t.0, "atLeast"),
                    LineSpacing::Exact(t) => (t.0, "exact"),
                };
                let _ = write!(out, r#" w:line="{value}" w:lineRule="{rule}""#);
            }
            out.push_str("/>");
        }
        "ind" => {
            let indent = &props.indent;
            if indent.is_empty() {
                return;
            }
            out.push_str("<w:ind");
            if let Some(start) = indent.start {
                let _ = write!(out, r#" w:left="{}""#, start.0);
            }
            if let Some(end) = indent.end {
                let _ = write!(out, r#" w:right="{}""#, end.0);
            }
            if let Some(hanging) = indent.hanging {
                let _ = write!(out, r#" w:hanging="{}""#, hanging.0);
            } else if let Some(first) = indent.first_line {
                let _ = write!(out, r#" w:firstLine="{}""#, first.0);
            }
            out.push_str("/>");
        }
        "contextualSpacing" => on_off(out, "contextualSpacing", props.contextual_spacing),
        "jc" => jc(out, props.justify),
        "textAlignment" => {
            if let Some(align) = props.text_align {
                let value = match align {
                    wp_model::prop::TextAlign::Auto => "auto",
                    wp_model::prop::TextAlign::Top => "top",
                    wp_model::prop::TextAlign::Center => "center",
                    wp_model::prop::TextAlign::Baseline => "baseline",
                    wp_model::prop::TextAlign::Bottom => "bottom",
                };
                let _ = write!(out, r#"<w:textAlignment w:val="{value}"/>"#);
            }
        }
        _ => {}
    }
    // `outlineLvl` sits between `jc` and the paragraph mark's `rPr`; it is
    // handled here rather than in the table because it is the only property
    // whose position depends on nothing.
    if name == "textAlignment" {
        if let Some(level) = props.outline_level {
            let _ = write!(out, r#"<w:outlineLvl w:val="{level}"/>"#);
        }
    }
}

/// Writes a `<w:rPr>` element, or nothing when there is nothing to say.
pub(crate) fn run_props(out: &mut String, props: &RunProps, styles: &StyleTable) {
    if props.is_empty() {
        return;
    }
    out.push_str("<w:rPr>");
    for name in RUN_ORDER.iter().chain(RUN_ORDER_TAIL.iter()) {
        run_prop(out, name, props, styles);
    }
    out.push_str("</w:rPr>");
}

fn run_prop(out: &mut String, name: &str, props: &RunProps, styles: &StyleTable) {
    if let Some(toggle) = Toggle::from_element(name) {
        if let Some(value) = props.toggles.get(toggle) {
            on_off(out, name, Some(value));
        }
        return;
    }
    match name {
        "rStyle" => {
            if let Some(style) = props.style.and_then(|id| styles.get(id)) {
                let _ = write!(out, r#"<w:rStyle w:val="{}"/>"#, escape_attr(&style.id));
            }
        }
        "rFonts" => {
            let fonts = &props.fonts;
            if fonts.is_empty() {
                return;
            }
            out.push_str("<w:rFonts");
            for (attribute, value) in [
                ("w:ascii", &fonts.ascii),
                ("w:hAnsi", &fonts.high_ansi),
                ("w:eastAsia", &fonts.east_asian),
                ("w:cs", &fonts.complex),
            ] {
                if let Some(face) = value {
                    let _ = write!(out, r#" {attribute}="{}""#, escape_attr(face));
                }
            }
            for (attribute, value) in [
                ("w:asciiTheme", fonts.ascii_theme),
                ("w:hAnsiTheme", fonts.high_ansi_theme),
                ("w:eastAsiaTheme", fonts.east_asian_theme),
                ("w:cstheme", fonts.complex_theme),
            ] {
                if let Some(which) = value {
                    let _ = write!(out, r#" {attribute}="{}""#, theme_font(which));
                }
            }
            out.push_str("/>");
        }
        "noProof" => on_off(out, "noProof", props.no_proof),
        "color" => {
            if let Some(value) = props.color {
                out.push_str("<w:color");
                color_attrs(out, value);
                out.push_str("/>");
            }
        }
        "spacing" => {
            if let Some(spacing) = props.letter_spacing {
                let _ = write!(out, r#"<w:spacing w:val="{}"/>"#, spacing.0);
            }
        }
        "w" => {
            if let Some(scale) = props.scale {
                let _ = write!(out, r#"<w:w w:val="{scale}"/>"#);
            }
        }
        "kern" => {
            if let Some(kern) = props.kern {
                let _ = write!(out, r#"<w:kern w:val="{}"/>"#, kern.0);
            }
        }
        "position" => {
            if let Some(raise) = props.raise {
                let _ = write!(out, r#"<w:position w:val="{}"/>"#, raise.0);
            }
        }
        "sz" => {
            if let Some(size) = props.size {
                let _ = write!(out, r#"<w:sz w:val="{}"/>"#, size.0);
            }
        }
        "szCs" => {
            if let Some(size) = props.size_complex {
                let _ = write!(out, r#"<w:szCs w:val="{}"/>"#, size.0);
            }
        }
        "highlight" => {
            if let Some(highlight) = props.highlight {
                let _ = write!(out, r#"<w:highlight w:val="{}"/>"#, highlight.name());
            }
        }
        "u" => {
            if let Some(underline) = props.underline {
                let value = underline_name(underline.kind);
                let _ = write!(out, r#"<w:u w:val="{value}""#);
                // `w:val` is the underline's kind; its colour lives in
                // `w:color`, which is where the reader looks for it.
                if let Some(color) = underline.color {
                    color_attr(out, color);
                }
                out.push_str("/>");
            }
        }
        "bdr" => {
            if let Some(edge) = props.border {
                border(out, "bdr", &edge);
            }
        }
        "shd" => {
            if let Some(value) = props.shading {
                shading(out, &value);
            }
        }
        "vertAlign" => {
            if let Some(align) = props.vert_align {
                let value = match align {
                    VertAlign::Baseline => "baseline",
                    VertAlign::Superscript => "superscript",
                    VertAlign::Subscript => "subscript",
                };
                let _ = write!(out, r#"<w:vertAlign w:val="{value}"/>"#);
            }
        }
        "rtl" => on_off(out, "rtl", props.rtl),
        "lang" => {
            if let Some(lang) = &props.lang {
                out.push_str("<w:lang");
                for (attribute, value) in [
                    ("w:val", &lang.value),
                    ("w:eastAsia", &lang.east_asian),
                    ("w:bidi", &lang.complex),
                ] {
                    if let Some(tag) = value {
                        let _ = write!(out, r#" {attribute}="{}""#, escape_attr(tag));
                    }
                }
                out.push_str("/>");
            }
        }
        _ => {}
    }
}

fn theme_font(which: wp_model::prop::ThemeFont) -> &'static str {
    use wp_model::prop::ThemeFont::*;
    match which {
        MajorAscii => "majorAscii",
        MajorHighAnsi => "majorHAnsi",
        MajorEastAsian => "majorEastAsia",
        MajorComplex => "majorBidi",
        MinorAscii => "minorAscii",
        MinorHighAnsi => "minorHAnsi",
        MinorEastAsian => "minorEastAsia",
        MinorComplex => "minorBidi",
    }
}

fn underline_name(kind: UnderlineKind) -> &'static str {
    use UnderlineKind::*;
    match kind {
        None => "none",
        Single => "single",
        Double => "double",
        Thick => "thick",
        Dotted => "dotted",
        DottedHeavy => "dottedHeavy",
        Dash => "dash",
        DashedHeavy => "dashedHeavy",
        DashLong => "dashLong",
        DashLongHeavy => "dashLongHeavy",
        DotDash => "dotDash",
        DashDotHeavy => "dashDotHeavy",
        DotDotDash => "dotDotDash",
        DashDotDotHeavy => "dashDotDotHeavy",
        Wave => "wave",
        WavyHeavy => "wavyHeavy",
        WavyDouble => "wavyDouble",
        Words => "words",
    }
}

/// The `w:val` and, for a themed colour, the theme attributes beside it.
fn color_attrs(out: &mut String, color: Color) {
    match color {
        Color::Theme { slot, tint, shade } => {
            // `w:val` is a *cache* of what the theme resolves to, and Word writes
            // one. `auto` is the honest value for a writer that would otherwise
            // have to guess: the theme reference beside it is what Word reads.
            let _ = write!(out, r#" w:val="auto" w:themeColor="{}""#, slot.name());
            if let Some(tint) = tint {
                let _ = write!(out, r#" w:themeTint="{tint:02X}""#);
            }
            if let Some(shade) = shade {
                let _ = write!(out, r#" w:themeShade="{shade:02X}""#);
            }
        }
        other => {
            let _ = write!(out, r#" w:val="{}""#, other.to_val());
        }
    }
}

fn border(out: &mut String, element: &str, border: &Border) {
    let style = match border.style {
        wp_model::prop::BorderStyle::None => "none",
        wp_model::prop::BorderStyle::Single => "single",
        wp_model::prop::BorderStyle::Thick => "thick",
        wp_model::prop::BorderStyle::Double => "double",
        wp_model::prop::BorderStyle::Dotted => "dotted",
        wp_model::prop::BorderStyle::Dashed => "dashed",
        wp_model::prop::BorderStyle::DotDash => "dotDash",
        wp_model::prop::BorderStyle::DotDotDash => "dotDotDash",
        wp_model::prop::BorderStyle::Triple => "triple",
        wp_model::prop::BorderStyle::Wave => "wave",
        wp_model::prop::BorderStyle::DoubleWave => "doubleWave",
        // An unrecognised style was read as `Art` and its name was not kept, so
        // it is written as a plain line. Recorded as a limit: a clip-art border
        // survives untouched as long as its paragraph is not edited, and becomes
        // a single line if it is.
        wp_model::prop::BorderStyle::Art => "single",
    };
    let _ = write!(out, "<w:{element} w:val=\"{style}\"");
    if let Some(size) = border.size {
        let _ = write!(out, r#" w:sz="{}""#, size.0);
    }
    if let Some(space) = border.space {
        let _ = write!(out, r#" w:space="{space}""#);
    }
    // A border spells its colour in `w:color`, not `w:val` — that attribute
    // already holds the line style here.
    if let Some(color) = border.color {
        color_attr(out, color);
    }
    out.push_str("/>");
}

/// The `w:color` attribute — for the elements whose `w:val` already means
/// something else, such as a border's line style or an underline's kind.
fn color_attr(out: &mut String, color: Color) {
    match color {
        Color::Theme { slot, tint, shade } => {
            // As in `color_attrs`: the cached value beside the reference, and
            // `auto` because a writer without the theme would have to guess.
            let _ = write!(out, r#" w:color="auto" w:themeColor="{}""#, slot.name());
            if let Some(tint) = tint {
                let _ = write!(out, r#" w:themeTint="{tint:02X}""#);
            }
            if let Some(shade) = shade {
                let _ = write!(out, r#" w:themeShade="{shade:02X}""#);
            }
        }
        other => {
            let _ = write!(out, r#" w:color="{}""#, other.to_val());
        }
    }
}

fn shading(out: &mut String, shading: &Shading) {
    let pattern = match shading.pattern {
        ShadingPattern::Clear => "clear".to_string(),
        ShadingPattern::Solid => "solid".to_string(),
        ShadingPattern::Percent(percent) => format!("pct{percent}"),
        ShadingPattern::Hatch => "clear".to_string(),
    };
    let _ = write!(out, r#"<w:shd w:val="{pattern}""#);
    if let Some(color) = shading.color {
        let _ = write!(out, r#" w:color="{}""#, color.to_val());
    }
    if let Some(fill) = shading.fill {
        let _ = write!(out, r#" w:fill="{}""#, fill.to_val());
    }
    out.push_str("/>");
}

/// `<w:jc>`, or nothing when no justification is stated.
fn jc(out: &mut String, justify: Option<Justify>) {
    if let Some(justify) = justify {
        let value = match justify {
            Justify::Start => "left",
            Justify::Center => "center",
            Justify::End => "right",
            Justify::Both => "both",
            Justify::Distribute => "distribute",
        };
        let _ = write!(out, r#"<w:jc w:val="{value}"/>"#);
    }
}

// ---------------------------------------------------------------- tables

/// Writes a `<w:tbl>` element.
///
/// Only for tables a save actually changed — an untouched one is copied byte
/// for byte by the splicer. The contract is the one `paragraph` keeps: reading
/// the emitted XML back must produce an equal [`Table`], because that re-read
/// is exactly how `write::same_table` decides whether the file's bytes can
/// stand. Anything the reader treats as absent-by-default is therefore not
/// written at all, and every properties element follows the schema's child
/// order — a `<w:tblPr>` out of sequence is a file Word calls damaged.
pub(crate) fn table(out: &mut String, table: &Table, styles: &StyleTable) {
    out.push_str("<w:tbl>");
    table_props(out, &table.props, styles);
    // `<w:tblGrid>` is required even when there is nothing to say about the
    // columns; the reader treats the empty spelling and an absent one alike.
    if table.grid.is_empty() {
        out.push_str("<w:tblGrid/>");
    } else {
        out.push_str("<w:tblGrid>");
        for column in &table.grid {
            let _ = write!(out, r#"<w:gridCol w:w="{}"/>"#, column.0);
        }
        out.push_str("</w:tblGrid>");
    }
    for row in &table.rows {
        table_row(out, row, styles);
    }
    out.push_str("</w:tbl>");
}

/// A width element — `<w:tblW>`, `<w:tcW>`, `<w:tblInd>` and the rest all share
/// the `w:w`/`w:type` shape.
fn width_element(out: &mut String, name: &str, width: Width) {
    let (kind, value) = match width {
        Width::Auto => ("auto", 0),
        Width::Fixed(twips) => ("dxa", twips.0),
        Width::Percent(pct) => ("pct", pct.0),
        Width::Nil => ("nil", 0),
    };
    let _ = write!(out, r#"<w:{name} w:w="{value}" w:type="{kind}"/>"#);
}

/// The edges of a `<w:tblBorders>` or `<w:tcBorders>`, in the schema's order.
/// The diagonals exist only on a cell and come last.
fn borders_element(
    out: &mut String,
    name: &str,
    borders: &TableBorders,
    diagonals: (Option<Border>, Option<Border>),
) {
    let _ = write!(out, "<w:{name}>");
    for (element, edge) in [
        ("top", borders.top),
        ("left", borders.start),
        ("bottom", borders.bottom),
        ("right", borders.end),
        ("insideH", borders.inside_h),
        ("insideV", borders.inside_v),
        ("tl2br", diagonals.0),
        ("tr2bl", diagonals.1),
    ] {
        if let Some(edge) = edge {
            border(out, element, &edge);
        }
    }
    let _ = write!(out, "</w:{name}>");
}

/// `<w:tblCellMar>` or `<w:tcMar>` — the same four width children.
fn margins_element(out: &mut String, name: &str, margins: &CellMargins) {
    let _ = write!(out, "<w:{name}>");
    for (element, value) in [
        ("top", margins.top),
        ("left", margins.start),
        ("bottom", margins.bottom),
        ("right", margins.end),
    ] {
        if let Some(value) = value {
            width_element(out, element, value);
        }
    }
    let _ = write!(out, "</w:{name}>");
}

fn float_anchor(anchor: FloatAnchor) -> &'static str {
    match anchor {
        FloatAnchor::Text => "text",
        FloatAnchor::Margin => "margin",
        FloatAnchor::Page => "page",
    }
}

fn table_props(out: &mut String, props: &TableProps, styles: &StyleTable) {
    let mut inner = String::new();
    if let Some(style) = props.style.and_then(|id| styles.get(id)) {
        let _ = write!(inner, r#"<w:tblStyle w:val="{}"/>"#, escape_attr(&style.id));
    }
    if let Some(float) = &props.float {
        inner.push_str("<w:tblpPr");
        for (attribute, value) in [
            ("w:leftFromText", float.left_from_text),
            ("w:rightFromText", float.right_from_text),
            ("w:topFromText", float.top_from_text),
            ("w:bottomFromText", float.bottom_from_text),
        ] {
            if let Some(twips) = value {
                let _ = write!(inner, r#" {attribute}="{}""#, twips.0);
            }
        }
        let _ = write!(
            inner,
            r#" w:vertAnchor="{}" w:horzAnchor="{}""#,
            float_anchor(float.vertical_anchor),
            float_anchor(float.horizontal_anchor)
        );
        if let Some(x) = float.x {
            let _ = write!(inner, r#" w:tblpX="{}""#, x.0);
        }
        if let Some(y) = float.y {
            let _ = write!(inner, r#" w:tblpY="{}""#, y.0);
        }
        inner.push_str("/>");
    }
    if props.bidi_visual {
        inner.push_str("<w:bidiVisual/>");
    }
    if props.width != Width::Auto {
        width_element(&mut inner, "tblW", props.width);
    }
    jc(&mut inner, props.justify);
    if let Some(spacing) = props.cell_spacing {
        width_element(&mut inner, "tblCellSpacing", spacing);
    }
    if let Some(indent) = props.indent {
        width_element(&mut inner, "tblInd", indent);
    }
    if !props.borders.is_empty() {
        borders_element(&mut inner, "tblBorders", &props.borders, (None, None));
    }
    if let Some(value) = props.shading {
        shading(&mut inner, &value);
    }
    if props.layout == TableLayout::Fixed {
        inner.push_str(r#"<w:tblLayout w:type="fixed"/>"#);
    }
    if props.cell_margins != CellMargins::default() {
        margins_element(&mut inner, "tblCellMar", &props.cell_margins);
    }
    if props.look != TableLook::default() {
        // Both spellings, the way Word writes them: the legacy mask for old
        // consumers, and the six attributes the reader prefers.
        let look = props.look;
        let bit = |on: bool| if on { "1" } else { "0" };
        let _ = write!(
            inner,
            r#"<w:tblLook w:val="{:04X}" w:firstRow="{}" w:lastRow="{}" w:firstColumn="{}" w:lastColumn="{}" w:noHBand="{}" w:noVBand="{}"/>"#,
            look.to_mask(),
            bit(look.first_row),
            bit(look.last_row),
            bit(look.first_column),
            bit(look.last_column),
            bit(look.no_h_band),
            bit(look.no_v_band)
        );
    }
    if let Some(caption) = &props.caption {
        let _ = write!(inner, r#"<w:tblCaption w:val="{}"/>"#, escape_attr(caption));
    }
    if let Some(description) = &props.description {
        let _ = write!(
            inner,
            r#"<w:tblDescription w:val="{}"/>"#,
            escape_attr(description)
        );
    }
    // `<w:tblPr>` is written even with nothing to say, because Word always
    // writes one and a reader meeting the empty spelling reads it as default.
    if inner.is_empty() {
        out.push_str("<w:tblPr/>");
    } else {
        out.push_str("<w:tblPr>");
        out.push_str(&inner);
        out.push_str("</w:tblPr>");
    }
}

fn table_row(out: &mut String, row: &Row, styles: &StyleTable) {
    out.push_str("<w:tr>");
    row_props(out, &row.props);
    for cell in &row.cells {
        table_cell(out, cell, styles);
    }
    out.push_str("</w:tr>");
}

fn row_props(out: &mut String, props: &RowProps) {
    if *props == RowProps::default() {
        return;
    }
    out.push_str("<w:trPr>");
    if props.grid_before != 0 {
        let _ = write!(out, r#"<w:gridBefore w:val="{}"/>"#, props.grid_before);
    }
    if props.grid_after != 0 {
        let _ = write!(out, r#"<w:gridAfter w:val="{}"/>"#, props.grid_after);
    }
    if let Some(width) = props.width_before {
        width_element(out, "wBefore", width);
    }
    if let Some(width) = props.width_after {
        width_element(out, "wAfter", width);
    }
    if props.cant_split {
        out.push_str("<w:cantSplit/>");
    }
    if let Some(height) = props.height {
        match height {
            // The value beside `auto` is ignored by every consumer, so none is
            // written; an absent rule already means `atLeast`.
            RowHeight::Auto => out.push_str(r#"<w:trHeight w:hRule="auto"/>"#),
            RowHeight::AtLeast(twips) => {
                let _ = write!(out, r#"<w:trHeight w:val="{}"/>"#, twips.0);
            }
            RowHeight::Exact(twips) => {
                let _ = write!(out, r#"<w:trHeight w:val="{}" w:hRule="exact"/>"#, twips.0);
            }
        }
    }
    if props.header {
        out.push_str("<w:tblHeader/>");
    }
    if let Some(spacing) = props.cell_spacing {
        width_element(out, "tblCellSpacing", spacing);
    }
    jc(out, props.justify);
    if let Some(revision) = &props.revision {
        // Only an insertion or a deletion can be said on a row — `<w:trPr>` has
        // no move elements, and the reader never produces one here.
        let (element, mark) = match revision {
            wp_model::Revision::Inserted(mark) => (Some("ins"), mark),
            wp_model::Revision::Deleted(mark) => (Some("del"), mark),
            wp_model::Revision::MovedFrom { mark, .. }
            | wp_model::Revision::MovedTo { mark, .. } => (None, mark),
        };
        if let Some(element) = element {
            let _ = write!(
                out,
                r#"<w:{element} w:id="{}" w:author="{}""#,
                mark.id,
                escape_attr(&mark.author)
            );
            if let Some(date) = &mark.date {
                let _ = write!(out, r#" w:date="{}""#, escape_attr(date));
            }
            out.push_str("/>");
        }
    }
    out.push_str("</w:trPr>");
}

fn table_cell(out: &mut String, cell: &Cell, styles: &StyleTable) {
    out.push_str("<w:tc>");
    cell_props(out, &cell.props);
    // A `<w:tc>` with no `<w:p>` in it is a document Word calls damaged.
    // `Cell::new` guarantees the paragraph, but a cell built by hand may not.
    if cell.content.is_empty() {
        out.push_str("<w:p/>");
    }
    for item in &cell.content {
        block(out, item, styles);
    }
    out.push_str("</w:tc>");
}

fn cell_props(out: &mut String, props: &CellProps) {
    if *props == CellProps::new() {
        return;
    }
    out.push_str("<w:tcPr>");
    if props.width != Width::Auto {
        width_element(out, "tcW", props.width);
    }
    if props.grid_span != 1 {
        let _ = write!(out, r#"<w:gridSpan w:val="{}"/>"#, props.grid_span);
    }
    match props.v_merge {
        Some(VMerge::Restart) => out.push_str(r#"<w:vMerge w:val="restart"/>"#),
        // The bare element means *continue* — the format's own inversion, and
        // the spelling Word writes for the covered cells of a merge.
        Some(VMerge::Continue) => out.push_str("<w:vMerge/>"),
        None => {}
    }
    if !props.borders.is_empty() || props.diagonal_down.is_some() || props.diagonal_up.is_some() {
        borders_element(
            out,
            "tcBorders",
            &props.borders,
            (props.diagonal_down, props.diagonal_up),
        );
    }
    if let Some(value) = props.shading {
        shading(out, &value);
    }
    if props.no_wrap {
        out.push_str("<w:noWrap/>");
    }
    if props.margins != CellMargins::default() {
        margins_element(out, "tcMar", &props.margins);
    }
    match props.text_direction {
        TextDirection::Horizontal => {}
        TextDirection::RotatedDown => out.push_str(r#"<w:textDirection w:val="tbRl"/>"#),
        TextDirection::RotatedUp => out.push_str(r#"<w:textDirection w:val="btLr"/>"#),
    }
    if props.fit_text {
        out.push_str("<w:tcFitText/>");
    }
    match props.v_align {
        CellVAlign::Top => {}
        CellVAlign::Center => out.push_str(r#"<w:vAlign w:val="center"/>"#),
        CellVAlign::Bottom => out.push_str(r#"<w:vAlign w:val="bottom"/>"#),
    }
    if props.hide_mark {
        out.push_str("<w:hideMark/>");
    }
    out.push_str("</w:tcPr>");
}

/// One block of a cell's content. Paragraphs and nested tables are the ordinary
/// cases; the rest are carried so that rewriting a table does not drop what a
/// cell legally holds.
fn block(out: &mut String, block: &Block, styles: &StyleTable) {
    match block {
        Block::Paragraph(paragraph) => self::paragraph(out, paragraph, styles),
        Block::Table(table) => self::table(out, table, styles),
        // The same trade as the inline control in `inline`: the wrapper is
        // written with the properties this crate models, and the rest is the
        // stated cost of rewriting a table that holds one.
        Block::Structured(sdt) => {
            out.push_str("<w:sdt><w:sdtPr>");
            if let Some(alias) = &sdt.alias {
                let _ = write!(out, r#"<w:alias w:val="{}"/>"#, escape_attr(alias));
            }
            if let Some(tag) = &sdt.tag {
                let _ = write!(out, r#"<w:tag w:val="{}"/>"#, escape_attr(tag));
            }
            if let Some(id) = sdt.id {
                let _ = write!(out, r#"<w:id w:val="{id}"/>"#);
            }
            out.push_str("</w:sdtPr><w:sdtContent>");
            for inner in &sdt.content {
                self::block(out, inner, styles);
            }
            out.push_str("</w:sdtContent></w:sdt>");
        }
        Block::Anchor(anchor) => self::anchor(out, anchor),
        Block::AltChunk { rel } => {
            let _ = write!(out, r#"<w:altChunk r:id="{}"/>"#, escape_attr(rel));
        }
    }
}

fn inline(out: &mut String, inline: &Inline, styles: &StyleTable) {
    match inline {
        Inline::Run(run) => self::run(out, run, styles),
        Inline::Hyperlink(link) => {
            out.push_str("<w:hyperlink");
            if let Some(rel) = &link.rel {
                let _ = write!(out, r#" r:id="{}""#, escape_attr(rel));
            }
            if let Some(anchor) = &link.anchor {
                let _ = write!(out, r#" w:anchor="{}""#, escape_attr(anchor));
            }
            out.push('>');
            for inner in &link.content {
                self::inline(out, inner, styles);
            }
            out.push_str("</w:hyperlink>");
        }
        Inline::Revised { revision, content } => {
            let (element, mark, name) = match revision {
                wp_model::Revision::Inserted(mark) => ("ins", mark, None),
                wp_model::Revision::Deleted(mark) => ("del", mark, None),
                wp_model::Revision::MovedFrom { mark, name } => ("moveFrom", mark, Some(name)),
                wp_model::Revision::MovedTo { mark, name } => ("moveTo", mark, Some(name)),
            };
            let _ = write!(
                out,
                r#"<w:{element} w:id="{}" w:author="{}""#,
                mark.id,
                escape_attr(&mark.author)
            );
            if let Some(date) = &mark.date {
                let _ = write!(out, r#" w:date="{}""#, escape_attr(date));
            }
            if let Some(name) = name {
                let _ = write!(out, r#" w:name="{}""#, escape_attr(name));
            }
            out.push('>');
            for inner in content {
                self::inline(out, inner, styles);
            }
            let _ = write!(out, "</w:{element}>");
        }
        Inline::SimpleField {
            instruction,
            content,
        } => {
            let _ = write!(
                out,
                r#"<w:fldSimple w:instr="{}">"#,
                escape_attr(instruction)
            );
            for inner in content {
                self::inline(out, inner, styles);
            }
            out.push_str("</w:fldSimple>");
        }
        Inline::Anchor(anchor) => self::anchor(out, anchor),
        // A content control, a smart tag, an equation and a wrapper are only
        // ever emitted for a paragraph that changed, and a paragraph holding one
        // is never treated as changed — see `write::document_out`. Emitting the
        // contents without the wrapper would lose the control, so the wrapper is
        // written and its unmodelled properties are the stated cost.
        Inline::Structured(sdt) => {
            out.push_str("<w:sdt><w:sdtPr>");
            if let Some(alias) = &sdt.alias {
                let _ = write!(out, r#"<w:alias w:val="{}"/>"#, escape_attr(alias));
            }
            if let Some(tag) = &sdt.tag {
                let _ = write!(out, r#"<w:tag w:val="{}"/>"#, escape_attr(tag));
            }
            if let Some(id) = sdt.id {
                let _ = write!(out, r#"<w:id w:val="{id}"/>"#);
            }
            out.push_str("</w:sdtPr><w:sdtContent>");
            for inner in &sdt.content {
                self::inline(out, inner, styles);
            }
            out.push_str("</w:sdtContent></w:sdt>");
        }
        Inline::Wrapper { content, .. } => {
            for inner in content {
                self::inline(out, inner, styles);
            }
        }
        Inline::Math(_) => {}
    }
}

fn anchor(out: &mut String, anchor: &wp_model::Anchor) {
    match anchor {
        wp_model::Anchor::BookmarkStart { id, name } => {
            let _ = write!(
                out,
                r#"<w:bookmarkStart w:id="{id}" w:name="{}"/>"#,
                escape_attr(name)
            );
        }
        wp_model::Anchor::BookmarkEnd { id } => {
            let _ = write!(out, r#"<w:bookmarkEnd w:id="{id}"/>"#);
        }
        wp_model::Anchor::CommentStart { id } => {
            let _ = write!(out, r#"<w:commentRangeStart w:id="{id}"/>"#);
        }
        wp_model::Anchor::CommentEnd { id } => {
            let _ = write!(out, r#"<w:commentRangeEnd w:id="{id}"/>"#);
        }
        wp_model::Anchor::PermissionStart { id, editor } => {
            let _ = write!(out, r#"<w:permStart w:id="{}""#, escape_attr(id));
            if let Some(editor) = editor {
                let _ = write!(out, r#" w:ed="{}""#, escape_attr(editor));
            }
            out.push_str("/>");
        }
        wp_model::Anchor::PermissionEnd { id } => {
            let _ = write!(out, r#"<w:permEnd w:id="{}"/>"#, escape_attr(id));
        }
        // `<w:commentReference>` lives inside a run rather than between them —
        // the reader never produces one here, and emitting it at this level
        // would put it where the schema does not allow it.
        wp_model::Anchor::CommentReference { .. } => {}
    }
}

fn run(out: &mut String, run: &Run, styles: &StyleTable) {
    out.push_str("<w:r>");
    run_props(out, &run.props, styles);
    for piece in &run.content {
        self::piece(out, piece);
    }
    out.push_str("</w:r>");
}

fn piece(out: &mut String, piece: &Piece) {
    match piece {
        Piece::Text(text) => text_element(out, "t", text),
        Piece::Deleted(text) => text_element(out, "delText", text),
        Piece::Instruction(text) => text_element(out, "instrText", text),
        Piece::DeletedInstruction(text) => text_element(out, "delInstrText", text),
        Piece::Tab => out.push_str("<w:tab/>"),
        Piece::Break(kind) => match kind {
            Break::Line => out.push_str("<w:br/>"),
            Break::Page => out.push_str(r#"<w:br w:type="page"/>"#),
            Break::Column => out.push_str(r#"<w:br w:type="column"/>"#),
            Break::TextWrapping(clear) => {
                let value = match clear {
                    wp_model::doc::Clear::None => "none",
                    wp_model::doc::Clear::Left => "left",
                    wp_model::doc::Clear::Right => "right",
                    wp_model::doc::Clear::All => "all",
                };
                let _ = write!(out, r#"<w:br w:type="textWrapping" w:clear="{value}"/>"#);
            }
        },
        Piece::Hyphen { breaking: false } => out.push_str("<w:noBreakHyphen/>"),
        Piece::Hyphen { breaking: true } => out.push_str("<w:softHyphen/>"),
        Piece::Symbol { font, ch } => {
            let _ = write!(
                out,
                r#"<w:sym w:font="{}" w:char="{:04X}"/>"#,
                escape_attr(font),
                *ch as u32
            );
        }
        Piece::FootnoteRef { id, custom_mark } => {
            let _ = write!(out, r#"<w:footnoteReference"#);
            if *custom_mark {
                out.push_str(r#" w:customMarkFollows="1""#);
            }
            let _ = write!(out, r#" w:id="{id}"/>"#);
        }
        Piece::EndnoteRef { id, custom_mark } => {
            let _ = write!(out, r#"<w:endnoteReference"#);
            if *custom_mark {
                out.push_str(r#" w:customMarkFollows="1""#);
            }
            let _ = write!(out, r#" w:id="{id}"/>"#);
        }
        Piece::CommentRef(id) => {
            let _ = write!(out, r#"<w:commentReference w:id="{id}"/>"#);
        }
        Piece::FieldStart { dirty, lock } => {
            out.push_str(r#"<w:fldChar w:fldCharType="begin""#);
            if *dirty {
                out.push_str(r#" w:dirty="1""#);
            }
            if *lock {
                out.push_str(r#" w:fldLock="1""#);
            }
            out.push_str("/>");
        }
        Piece::FieldSeparate => out.push_str(r#"<w:fldChar w:fldCharType="separate"/>"#),
        Piece::FieldEnd => out.push_str(r#"<w:fldChar w:fldCharType="end"/>"#),
        Piece::PositionTab => out.push_str("<w:ptab/>"),
        // Word rewrites these itself on the next layout, and a stale one is
        // worse than none: it would tell C19's oracle where the pages used to
        // end rather than where they do.
        Piece::LastRenderedPageBreak => {}
        // A drawing and an embedded object are put back as the bytes they were
        // read as, so that editing the paragraph around a picture does not
        // destroy the picture. The drawing's size and position are the two
        // things the editor can change, and those are spliced into those bytes
        // rather than re-authored — see `write::drawing`.
        Piece::Drawing(drawing) => {
            out.push_str(&String::from_utf8_lossy(&super::drawing::patch(drawing)));
        }
        Piece::Embedded { source, .. } => {
            out.push_str(&String::from_utf8_lossy(source));
        }
    }
}

fn text_element(out: &mut String, element: &str, text: &str) {
    let _ = write!(out, "<w:{element}");
    if needs_space_preserve(text) {
        out.push_str(r#" xml:space="preserve""#);
    }
    let _ = write!(out, ">{}</w:{element}>", escape_text(text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::prop::{Indent, Spacing};
    use wp_model::style::{Style, StyleKind};
    use wp_model::units::{HalfPoint, Twips};

    fn styles() -> StyleTable {
        let mut styles = StyleTable::new();
        styles.insert(Style::new("Heading1", StyleKind::Paragraph));
        styles.insert(Style::new("Strong", StyleKind::Character));
        styles
    }

    fn emit(paragraph: &Paragraph) -> String {
        let mut out = String::new();
        super::paragraph(&mut out, paragraph, &styles());
        out
    }

    #[test]
    fn a_plain_paragraph_has_no_properties_element_at_all() {
        // An empty `<w:pPr/>` is legal and Word does not write one, and the
        // point of this writer is to look like Word.
        assert_eq!(
            emit(&Paragraph::of("hello")),
            "<w:p><w:r><w:t>hello</w:t></w:r></w:p>"
        );
    }

    #[test]
    fn run_properties_come_out_in_the_order_the_schema_demands() {
        // Not a formatting difference: a `<w:sz>` before a `<w:color>` is a file
        // Word calls damaged.
        let mut run = Run::of("x");
        run.props.size = Some(HalfPoint(24));
        run.props.color = Some(Color::Rgb([0xFF, 0, 0]));
        run.props.toggles.set(Toggle::Bold, true);
        run.props.toggles.set(Toggle::Italic, true);
        let paragraph = Paragraph {
            content: vec![Inline::Run(run)],
            ..Paragraph::new()
        };
        let xml = emit(&paragraph);
        let order = |needle: &str| {
            xml.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {xml}"))
        };
        assert!(order("<w:b/>") < order("<w:i/>"));
        assert!(order("<w:i/>") < order("<w:color"));
        assert!(order("<w:color") < order("<w:sz"));
    }

    #[test]
    fn paragraph_properties_come_out_in_order_too() {
        let mut paragraph = Paragraph::of("x");
        paragraph.props.justify = Some(Justify::Center);
        paragraph.props.indent = Indent {
            start: Some(Twips(720)),
            ..Indent::default()
        };
        paragraph.props.style = Some(styles().lookup("Heading1").unwrap());
        paragraph.props.keep_next = Some(true);
        let xml = emit(&paragraph);
        let order = |needle: &str| {
            xml.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {xml}"))
        };
        assert!(order("<w:pStyle") < order("<w:keepNext"));
        assert!(order("<w:keepNext") < order("<w:ind"));
        assert!(order("<w:ind") < order("<w:jc"));
    }

    #[test]
    fn a_bare_element_is_written_for_true_and_val_zero_for_false() {
        let mut paragraph = Paragraph::of("x");
        paragraph.props.keep_next = Some(true);
        paragraph.props.widow_control = Some(false);
        let xml = emit(&paragraph);
        assert!(xml.contains("<w:keepNext/>"), "{xml}");
        assert!(xml.contains(r#"<w:widowControl w:val="0"/>"#), "{xml}");
        // Absent stays absent: writing `w:val="0"` for a property nobody stated
        // would turn "inherit" into "off".
        assert!(!xml.contains("keepLines"), "{xml}");
    }

    #[test]
    fn space_preserve_is_written_where_the_text_needs_it() {
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run::of("word")), Inline::Run(Run::of(" and "))],
            ..Paragraph::new()
        };
        let xml = emit(&paragraph);
        assert!(xml.contains("<w:t>word</w:t>"), "{xml}");
        assert!(
            xml.contains(r#"<w:t xml:space="preserve"> and </w:t>"#),
            "{xml}"
        );
    }

    #[test]
    fn a_hanging_indent_is_written_as_hanging_rather_than_as_both() {
        let mut paragraph = Paragraph::of("x");
        paragraph.props.indent = Indent {
            start: Some(Twips(720)),
            first_line: Some(Twips(240)),
            hanging: Some(Twips(360)),
            ..Indent::default()
        };
        let xml = emit(&paragraph);
        assert!(xml.contains(r#"w:hanging="360""#), "{xml}");
        assert!(!xml.contains("firstLine"), "hanging wins: {xml}");
    }

    #[test]
    fn a_themed_colour_keeps_its_reference_rather_than_being_flattened() {
        let mut run = Run::of("x");
        run.props.color = Some(Color::Theme {
            slot: wp_model::ThemeSlot::Accent1,
            tint: None,
            shade: Some(0xBF),
        });
        let paragraph = Paragraph {
            content: vec![Inline::Run(run)],
            ..Paragraph::new()
        };
        let xml = emit(&paragraph);
        assert!(xml.contains(r#"w:themeColor="accent1""#), "{xml}");
        assert!(xml.contains(r#"w:themeShade="BF""#), "{xml}");
    }

    #[test]
    fn a_paragraph_id_is_written_as_the_hex_word_wrote() {
        let paragraph = Paragraph {
            id: Some(0x760D_8500),
            text_id: Some(0x19F8_FDBD),
            ..Paragraph::of("x")
        };
        let xml = emit(&paragraph);
        assert!(xml.contains(r#"w14:paraId="760D8500""#), "{xml}");
        assert!(xml.contains(r#"w14:textId="19F8FDBD""#), "{xml}");
    }

    #[test]
    fn a_tracked_insertion_wraps_the_runs_it_is_about() {
        let paragraph = Paragraph {
            content: vec![wp_model::doc::inserted_by(
                "Adnan Khan",
                3,
                vec![Inline::Run(Run::of("new"))],
            )],
            ..Paragraph::new()
        };
        let xml = emit(&paragraph);
        assert!(
            xml.contains(r#"<w:ins w:id="3" w:author="Adnan Khan">"#),
            "{xml}"
        );
        assert!(xml.contains("<w:t>new</w:t>"), "{xml}");
    }

    #[test]
    fn a_field_is_written_as_the_three_pieces_it_is() {
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::FieldStart {
                        dirty: true,
                        lock: false,
                    },
                    Piece::Instruction(" PAGE ".into()),
                    Piece::FieldSeparate,
                    Piece::Text("1".into()),
                    Piece::FieldEnd,
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let xml = emit(&paragraph);
        assert!(
            xml.contains(r#"w:fldCharType="begin" w:dirty="1""#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"<w:instrText xml:space="preserve"> PAGE </w:instrText>"#),
            "{xml}"
        );
        assert!(xml.contains(r#"w:fldCharType="end""#), "{xml}");
    }

    #[test]
    fn text_that_needs_escaping_is_escaped() {
        let paragraph = Paragraph::of("R&D <5>");
        assert!(emit(&paragraph).contains("R&amp;D &lt;5&gt;"));
    }

    #[test]
    fn the_spacing_rule_is_written_beside_the_number_it_governs() {
        let mut paragraph = Paragraph::of("x");
        paragraph.props.spacing = Spacing {
            before: Some(Twips(120)),
            line: Some(LineSpacing::Exact(Twips(360))),
            ..Spacing::default()
        };
        let xml = emit(&paragraph);
        assert!(
            xml.contains(r#"<w:spacing w:before="120" w:line="360" w:lineRule="exact"/>"#),
            "{xml}"
        );
    }
}
