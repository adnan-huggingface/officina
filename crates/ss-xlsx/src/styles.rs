//! The style table (`styles.xml`).
//!
//! Six lists, four of which are addressed by position from somewhere else:
//! `<fonts>`, `<fills>`, and `<borders>` are indexed by an `<xf>`, and
//! `<cellXfs>` is indexed by a cell's `s` attribute. Nothing here may reorder
//! any of them.
//!
//! Three traps, each of which produces a plausible and wrong picture rather
//! than an error:
//!
//! - **Most number formats are never written down.** `numFmtId="14"` is a date
//!   and the file says nothing more; the code lives in a table baked into every
//!   implementation. A reader that only honours the codes it can see shows every
//!   date in every document as a five-digit serial.
//! - **`<cellStyleXfs>` is a different list that also holds `<xf>` elements**,
//!   and a cell's `s` attribute does not index it. Reading both into one shifts
//!   every style by the size of the first.
//! - **A `<dxf>` spells its fill backwards.** In a regular solid fill the
//!   visible colour is `fgColor`; in a differential one Excel writes `bgColor`.
//!   Read the same way as a regular fill, every conditional format in every
//!   workbook comes out uncoloured.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use ss_model::color::{Color, Theme};
use ss_model::style::{
    Alignment, Border, BorderStyle, CellFormat, Dxf, Edge, Fill, Font, HAlign, NamedStyle, Parts,
    Pattern, StyleTable, Underline, VAlign, VertAlign,
};

use crate::error::{xml_err, Result};
use crate::xml::{
    attr_f64, attr_raw, attr_text, attr_u32, attributes, end_local_name, local_name, parse_bool,
    parse_f64, parse_u32, strip_prefix,
};

/// Which list the reader is inside. `<xf>`, `<font>`, `<fill>`, and `<border>`
/// all appear in more than one place, so the element name alone never says what
/// is being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Fonts,
    Fills,
    Borders,
    CellStyleXfs,
    CellXfs,
    CellStyles,
    Dxfs,
}

/// Where a `<color>` belongs, which the element itself never says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorSlot {
    None,
    Font,
    FillFg,
    FillBg,
    Edge(EdgeSlot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeSlot {
    Left,
    Right,
    Top,
    Bottom,
    Diagonal,
}

pub(crate) fn parse(part: &str, data: &[u8], theme: Theme) -> Result<StyleTable> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = true;

    let mut parts = Parts {
        theme,
        ..Default::default()
    };

    let mut section = Section::None;
    let mut slot = ColorSlot::None;
    let mut font = Font::default();
    let mut fill = Fill::default();
    let mut border = Border::default();
    let mut xf = CellFormat::default();
    let mut dxf = Dxf::default();
    // A `<dxf>` is a *partial* look, so its font attributes have to stay
    // `None` until one is actually written.
    let mut in_dxf_font = false;
    let mut buf = Vec::new();

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        let empty = matches!(ev, Event::Empty(_));

        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"numFmt" => {
                    if let (Some(id), Some(code)) =
                        (attr_u32(e, b"numFmtId"), attr_text(e, b"formatCode"))
                    {
                        if section == Section::Dxfs {
                            dxf.number_format = Some(code);
                        } else {
                            parts.codes.insert(id, code);
                        }
                    }
                }

                // An empty `<fonts count="0"/>` opens no section: there is
                // nothing inside it, and leaving the section set would file the
                // next list's contents under this one.
                b"fonts" if !empty => section = Section::Fonts,
                b"fills" if !empty => section = Section::Fills,
                b"borders" if !empty => section = Section::Borders,
                b"cellStyleXfs" if !empty => section = Section::CellStyleXfs,
                b"cellXfs" if !empty => section = Section::CellXfs,
                b"cellStyles" if !empty => section = Section::CellStyles,
                b"dxfs" if !empty => section = Section::Dxfs,

                b"dxf" => {
                    dxf = Dxf::default();
                    if empty {
                        parts.dxfs.push(std::mem::take(&mut dxf));
                    }
                }

                b"font" => {
                    font = Font::default();
                    slot = ColorSlot::Font;
                    in_dxf_font = section == Section::Dxfs;
                    if empty {
                        // `<font/>` is the default font, and a fonts table that
                        // skipped it would renumber every font after it.
                        if section == Section::Fonts {
                            parts.fonts.push(std::mem::take(&mut font));
                        }
                        slot = ColorSlot::None;
                        in_dxf_font = false;
                    }
                }
                b"b" if slot == ColorSlot::Font => {
                    let on = on_by_default(e);
                    if in_dxf_font {
                        dxf.bold = Some(on);
                    } else {
                        font.bold = on;
                    }
                }
                b"i" if slot == ColorSlot::Font => {
                    let on = on_by_default(e);
                    if in_dxf_font {
                        dxf.italic = Some(on);
                    } else {
                        font.italic = on;
                    }
                }
                b"strike" if slot == ColorSlot::Font => {
                    let on = on_by_default(e);
                    if in_dxf_font {
                        dxf.strike = Some(on);
                    } else {
                        font.strike = on;
                    }
                }
                b"u" if slot == ColorSlot::Font => {
                    // `<u/>` with no `val` is a single underline, not none.
                    let style =
                        attr_text(e, b"val").map_or(Underline::Single, |v| Underline::from_xml(&v));
                    if in_dxf_font {
                        dxf.underline = Some(style);
                    } else {
                        font.underline = style;
                    }
                }
                b"vertAlign" if slot == ColorSlot::Font => {
                    font.vert_align = match attr_raw(e, b"val").as_deref() {
                        Some(b"superscript") => Some(VertAlign::Superscript),
                        Some(b"subscript") => Some(VertAlign::Subscript),
                        _ => None,
                    };
                }
                b"sz" if slot == ColorSlot::Font => {
                    if let Some(size) = attr_f64(e, b"val") {
                        font.size = size;
                    }
                }
                b"name" | b"rFont" if slot == ColorSlot::Font => {
                    if let Some(name) = attr_text(e, b"val") {
                        font.name = name;
                    }
                }

                b"fill" => {
                    fill = Fill::default();
                    if empty && section == Section::Fills {
                        parts.fills.push(std::mem::take(&mut fill));
                    }
                }
                b"patternFill" => {
                    let declared = attr_raw(e, b"patternType");
                    fill.pattern = match declared.as_deref() {
                        None if section == Section::Dxfs => {
                            // A differential fill routinely omits the type and
                            // still paints. Excel means solid, and writes the
                            // colour in `bgColor`.
                            Pattern::Solid
                        }
                        None | Some(b"none") => Pattern::None,
                        Some(b"solid") => Pattern::Solid,
                        Some(other) => Pattern::Named(String::from_utf8_lossy(other).into_owned()),
                    };
                }
                b"fgColor" => {
                    let color = read_color(e);
                    if section == Section::Dxfs {
                        fill.bg = color;
                    } else {
                        fill.fg = color;
                    }
                    slot = ColorSlot::FillFg;
                }
                b"bgColor" => {
                    let color = read_color(e);
                    // Backwards on purpose. See the module note.
                    if section == Section::Dxfs {
                        fill.fg = color;
                    } else {
                        fill.bg = color;
                    }
                    slot = ColorSlot::FillBg;
                }

                b"left" | b"start" => slot = edge_start(e, &mut border, EdgeSlot::Left),
                b"right" | b"end" => slot = edge_start(e, &mut border, EdgeSlot::Right),
                b"top" => slot = edge_start(e, &mut border, EdgeSlot::Top),
                b"bottom" => slot = edge_start(e, &mut border, EdgeSlot::Bottom),
                b"diagonal" => slot = edge_start(e, &mut border, EdgeSlot::Diagonal),
                b"border" => {
                    border = Border::default();
                    border.diagonal_up = attr_raw(e, b"diagonalUp")
                        .and_then(|v| parse_bool(&v))
                        .unwrap_or(false);
                    border.diagonal_down = attr_raw(e, b"diagonalDown")
                        .and_then(|v| parse_bool(&v))
                        .unwrap_or(false);
                    if empty && section == Section::Borders {
                        parts.borders.push(border);
                    }
                }

                b"color" => {
                    let color = read_color(e);
                    match slot {
                        ColorSlot::Font => {
                            if in_dxf_font {
                                dxf.color = Some(color);
                            } else {
                                font.color = color;
                            }
                        }
                        ColorSlot::Edge(which) => edge_of(&mut border, which).color = color,
                        _ => {}
                    }
                }

                b"xf" => {
                    xf = read_xf(e);
                    if empty {
                        push_xf(&mut parts, section, std::mem::take(&mut xf));
                    }
                }
                b"alignment" => {
                    xf.alignment = read_alignment(e);
                }
                // Guarded by section because `<protection>` is also a child of
                // `<dxf>`, and a conditional format's rule must not decide
                // whether the cell it lands on can be typed in.
                b"protection" if matches!(section, Section::CellXfs | Section::CellStyleXfs) => {
                    if let Some(raw) = attr_raw(e, b"locked") {
                        xf.locked = parse_bool(&raw).unwrap_or(true);
                    }
                }

                b"cellStyle" if section == Section::CellStyles => {
                    parts.named.push(NamedStyle {
                        name: attr_text(e, b"name").unwrap_or_default(),
                        xf_id: attr_u32(e, b"xfId").unwrap_or(0),
                        builtin_id: attr_u32(e, b"builtinId"),
                        hidden: attr_raw(e, b"hidden")
                            .and_then(|v| parse_bool(&v))
                            .unwrap_or(false),
                    });
                }

                _ => {}
            },

            Event::End(ref e) => match end_local_name(e) {
                b"font" => {
                    if section == Section::Fonts {
                        parts.fonts.push(std::mem::take(&mut font));
                    }
                    in_dxf_font = false;
                    slot = ColorSlot::None;
                }
                b"fill" => {
                    let done = std::mem::take(&mut fill);
                    match section {
                        Section::Fills => parts.fills.push(done),
                        Section::Dxfs => dxf.fill = Some(done),
                        _ => {}
                    }
                    slot = ColorSlot::None;
                }
                b"border" => {
                    let done = std::mem::take(&mut border);
                    match section {
                        Section::Borders => parts.borders.push(done),
                        Section::Dxfs if !done.is_none() => dxf.border = Some(done),
                        _ => {}
                    }
                    slot = ColorSlot::None;
                }
                b"left" | b"right" | b"top" | b"bottom" | b"diagonal" | b"start" | b"end"
                | b"fgColor" | b"bgColor" => slot = ColorSlot::None,
                b"xf" => push_xf(&mut parts, section, std::mem::take(&mut xf)),
                b"dxf" => parts.dxfs.push(std::mem::take(&mut dxf)),
                b"fonts" | b"fills" | b"borders" | b"cellStyleXfs" | b"cellXfs" | b"cellStyles"
                | b"dxfs" => section = Section::None,
                _ => {}
            },

            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(StyleTable::from_parts(parts))
}

/// A toggle element: `<b/>` means on, `<b val="0"/>` means off.
fn on_by_default(e: &BytesStart<'_>) -> bool {
    attr_raw(e, b"val")
        .and_then(|v| parse_bool(&v))
        .unwrap_or(true)
}

fn edge_start(e: &BytesStart<'_>, border: &mut Border, which: EdgeSlot) -> ColorSlot {
    let style = attr_raw(e, b"style")
        .map(|v| BorderStyle::from_xml(&String::from_utf8_lossy(&v)))
        .unwrap_or(BorderStyle::None);
    edge_of(border, which).style = style;
    ColorSlot::Edge(which)
}

fn edge_of(border: &mut Border, which: EdgeSlot) -> &mut Edge {
    match which {
        EdgeSlot::Left => &mut border.left,
        EdgeSlot::Right => &mut border.right,
        EdgeSlot::Top => &mut border.top,
        EdgeSlot::Bottom => &mut border.bottom,
        EdgeSlot::Diagonal => &mut border.diagonal,
    }
}

fn push_xf(parts: &mut Parts, section: Section, xf: CellFormat) {
    match section {
        Section::CellXfs => parts.cell_xfs.push(xf),
        Section::CellStyleXfs => parts.cell_style_xfs.push(xf),
        _ => {}
    }
}

fn read_xf(e: &BytesStart<'_>) -> CellFormat {
    let mut xf = CellFormat::default();
    for a in attributes(e) {
        match strip_prefix(a.key.as_ref()) {
            b"numFmtId" => xf.num_fmt_id = parse_u32(&a.value).unwrap_or(0),
            b"fontId" => xf.font = parse_u32(&a.value).unwrap_or(0),
            b"fillId" => xf.fill = parse_u32(&a.value).unwrap_or(0),
            b"borderId" => xf.border = parse_u32(&a.value).unwrap_or(0),
            b"xfId" => xf.xf_id = parse_u32(&a.value).unwrap_or(0),
            b"quotePrefix" => xf.quote_prefix = parse_bool(&a.value).unwrap_or(false),
            _ => {}
        }
    }
    xf
}

fn read_alignment(e: &BytesStart<'_>) -> Alignment {
    let mut out = Alignment::default();
    for a in attributes(e) {
        match strip_prefix(a.key.as_ref()) {
            b"horizontal" => out.horizontal = HAlign::from_xml(&String::from_utf8_lossy(&a.value)),
            b"vertical" => out.vertical = VAlign::from_xml(&String::from_utf8_lossy(&a.value)),
            b"wrapText" => out.wrap = parse_bool(&a.value).unwrap_or(false),
            b"shrinkToFit" => out.shrink = parse_bool(&a.value).unwrap_or(false),
            b"indent" => out.indent = parse_u32(&a.value).unwrap_or(0),
            b"textRotation" => out.rotation = parse_u32(&a.value).unwrap_or(0),
            _ => {}
        }
    }
    out
}

/// Reads any of the five colour elements, all of which share one attribute set.
pub(crate) fn read_color(e: &BytesStart<'_>) -> Color {
    let mut tint = 0.0f64;
    let mut color = Color::Auto;
    for a in attributes(e) {
        match strip_prefix(a.key.as_ref()) {
            b"auto" => {
                if parse_bool(&a.value).unwrap_or(false) {
                    color = Color::Auto;
                }
            }
            b"rgb" => {
                if let Some(parsed) = Color::from_hex(&String::from_utf8_lossy(&a.value)) {
                    color = parsed;
                }
            }
            b"indexed" => {
                if let Some(i) = parse_u32(&a.value) {
                    color = Color::Indexed(i);
                }
            }
            b"theme" => {
                if let Some(i) = parse_u32(&a.value) {
                    color = Color::Theme {
                        index: i,
                        tint: 0.0,
                    };
                }
            }
            b"tint" => tint = parse_f64(&a.value).unwrap_or(0.0),
            _ => {}
        }
    }
    // `tint` may arrive before `theme`, so it is applied after the whole tag.
    match color {
        Color::Theme { index, .. } => Color::Theme { index, tint },
        Color::Rgb(rgb) if tint != 0.0 => {
            let [a, r, g, b] = rgb;
            let [r, g, b] = ss_model::color::apply_tint([r, g, b], tint);
            Color::Rgb([a, r, g, b])
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::numfmt::FormatValue;
    use ss_model::StyleId;

    const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="1"><numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0.00"/></numFmts>
  <fonts count="3">
    <font><sz val="11"/><color theme="1"/><name val="Calibri"/></font>
    <font><b/><sz val="14"/><color rgb="FFFF0000"/><name val="Cambria"/></font>
    <font><i/><u val="double"/><strike/><sz val="11"/><color indexed="10"/><name val="Calibri"/></font>
  </fonts>
  <fills count="3">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FFFFEB9C"/><bgColor indexed="64"/></patternFill></fill>
  </fills>
  <borders count="2">
    <border><left/><right/><top/><bottom/><diagonal/></border>
    <border><left style="thin"><color rgb="FF000000"/></left><right/><top/><bottom style="double"><color theme="4" tint="-0.25"/></bottom><diagonal/></border>
  </borders>
  <cellStyleXfs count="1"><xf numFmtId="9" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="4">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="164" fontId="1" fillId="2" borderId="1" xfId="0" applyNumberFormat="1" applyFont="1"/>
    <xf numFmtId="0" fontId="2" fillId="0" borderId="0" xfId="0" quotePrefix="1">
      <alignment horizontal="center" vertical="top" wrapText="1" indent="2" textRotation="135"/>
    </xf>
  </cellXfs>
  <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
  <dxfs count="1">
    <dxf><font><b/><color rgb="FF9C0006"/></font>
      <fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill></dxf>
  </dxfs>
</styleSheet>"#;

    fn table() -> StyleTable {
        parse("styles.xml", STYLES.as_bytes(), Theme::default()).expect("parses")
    }

    fn shown(table: &StyleTable, style: u32, value: f64) -> String {
        table
            .number_format(StyleId(style))
            .format(FormatValue::Number(value))
            .text
    }

    #[test]
    fn cell_formats_resolve_through_builtins_and_custom_codes() {
        let table = table();
        assert_eq!(table.len(), 4, "only the cellXfs entries");
        assert_eq!(shown(&table, 0, 45352.0), "45352");
        assert_eq!(shown(&table, 1, 45352.0), "03-01-24");
        assert_eq!(shown(&table, 2, 1234.5), "$1,234.50");
    }

    #[test]
    fn the_named_style_table_is_not_the_cell_style_table() {
        // `<cellStyleXfs>` also holds `<xf>` elements. Counting them here would
        // shift every cell's style index by one and format the whole sheet
        // wrongly — plausibly, since the values would still be numbers.
        let table = table();
        assert_eq!(
            shown(&table, 0, 0.5),
            "0.5",
            "style 0 is General, not the 0% from cellStyleXfs"
        );
        assert_eq!(table.original().cell_xfs, 4);
    }

    #[test]
    fn fonts_come_back_with_everything_that_changes_how_a_cell_reads() {
        let table = table();
        let plain = table.font(StyleId(0));
        assert_eq!(plain.name, "Calibri");
        assert_eq!(plain.size, 11.0);
        assert_eq!(
            plain.color,
            Color::Theme {
                index: 1,
                tint: 0.0
            }
        );

        let heading = table.font(StyleId(2));
        assert!(heading.bold);
        assert_eq!(heading.size, 14.0);
        assert_eq!(heading.name, "Cambria");
        assert_eq!(heading.color.resolve(table.theme()), Some([0xFF, 0, 0]));

        let fancy = table.font(StyleId(3));
        assert!(fancy.italic && fancy.strike);
        assert_eq!(fancy.underline, Underline::Double);
        assert_eq!(fancy.color, Color::Indexed(10));
    }

    #[test]
    fn a_solid_fill_is_read_from_its_foreground() {
        let table = table();
        assert_eq!(
            table.fill(StyleId(2)).shade(table.theme()),
            Some([0xFF, 0xEB, 0x9C])
        );
        assert!(table.fill(StyleId(0)).is_none());
    }

    #[test]
    fn borders_keep_their_style_and_their_colour_per_edge() {
        let table = table();
        let border = table.border(StyleId(2));
        assert_eq!(border.left.style, BorderStyle::Thin);
        assert_eq!(border.bottom.style, BorderStyle::Double);
        assert!(border.right.is_none());
        // A tinted theme colour is neither the raw theme colour nor black.
        let bottom = border
            .bottom
            .color
            .resolve(table.theme())
            .expect("resolves");
        assert_ne!(bottom, [0, 0, 0]);
        assert_ne!(bottom, table.theme().color(4).expect("accent1"));
    }

    #[test]
    fn alignment_and_quote_prefix_survive() {
        let table = table();
        let align = table.alignment(StyleId(3));
        assert_eq!(align.horizontal, HAlign::Center);
        assert_eq!(align.vertical, VAlign::Top);
        assert!(align.wrap);
        assert_eq!(align.indent, 2);
        assert_eq!(align.degrees(), Some(-45.0));
        assert!(table.format_of(StyleId(3)).expect("xf").quote_prefix);
    }

    #[test]
    fn a_dxf_spells_its_fill_backwards_and_is_read_that_way() {
        // Excel writes the *visible* colour of a differential fill in `bgColor`,
        // the opposite of a regular fill. Read the ordinary way, every "Light
        // Red Fill" conditional format in the world comes out uncoloured.
        let table = table();
        let dxf = table.dxf(0).expect("one dxf");
        assert_eq!(dxf.bold, Some(true));
        assert_eq!(
            dxf.color.and_then(|c| c.resolve(table.theme())),
            Some([0x9C, 0x00, 0x06])
        );
        assert_eq!(
            dxf.fill.as_ref().and_then(|f| f.shade(table.theme())),
            Some([0xFF, 0xC7, 0xCE]),
            "the colour is in bgColor and it is what gets painted"
        );
    }

    #[test]
    fn a_named_style_is_recorded_with_the_table_it_points_into() {
        let table = table();
        let named = table.named_styles();
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].name, "Normal");
        assert_eq!(named[0].builtin_id, Some(0));
    }

    #[test]
    fn a_workbook_with_no_styles_part_still_formats() {
        let table = parse("styles.xml", b"<styleSheet/>", Theme::default()).expect("parses");
        assert!(table.is_empty());
        assert_eq!(shown(&table, 0, 1.5), "1.5");
    }
}
