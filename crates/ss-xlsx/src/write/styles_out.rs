//! Adding to `styles.xml` without rewriting it.
//!
//! Five tables live in this part and four of them are addressed by position:
//! `<fonts>`, `<fills>`, and `<borders>` by an `<xf>`, and `<cellXfs>` by a
//! cell's `s` attribute. So the file's own entries are copied through and ours
//! are appended. Appending is not a stylistic choice — inserting anywhere but
//! the end would silently reformat every cell in the workbook after the
//! insertion point, and the file would still be valid, so nothing would report
//! it.
//!
//! Everything the file already has is copied byte for byte, including the parts
//! nobody here reads: gradient fills, table styles, the `<colors>` palette,
//! `extLst`. Only entries we authored are ever written out, and those by
//! construction contain nothing we did not model.

use quick_xml::events::Event;

use ss_model::style::{Border, CellFormat, Edge, Fill, Font, Pattern, StyleTable};
use ss_model::Color;

use crate::error::Result;
use crate::write::splice::{close, open, prefix_of, raw_attr, retag, Set, Splicer};
use crate::xml::{end_local_name, local_name, parse_u32};

/// What the model has that the file does not.
pub(crate) struct Additions {
    /// `numFmtId` -> format code, for codes the file never declared.
    formats: Vec<(u32, String)>,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    /// The `<xf>` entries to append to `cellXfs`.
    styles: Vec<CellFormat>,
    /// Differential formats past the file's count — conditional-formatting
    /// rules authored in the app point at these.
    dxfs: Vec<ss_model::style::Dxf>,
}

/// Compares the model's style table against the file's.
///
/// The file is read again rather than remembered from open, so that this is a
/// statement about the bytes being written and not about what a reader believed
/// some time earlier. Where the two disagree the *file* wins: appending from an
/// index the file does not actually end at is how a style table gets shifted.
pub(crate) fn additions(part: &str, data: &[u8], styles: &StyleTable) -> Result<Additions> {
    let file = scan(part, data)?;
    Ok(Additions {
        formats: styles
            .codes()
            .iter()
            .filter(|(id, _)| !file.declared.contains(id))
            .map(|(id, code)| (*id, code.clone()))
            .collect(),
        fonts: after(styles.fonts(), file.fonts),
        fills: after(styles.fills(), file.fills),
        borders: after(styles.borders(), file.borders),
        styles: after(styles.entries(), file.cell_xfs),
        dxfs: after(styles.dxfs(), file.dxfs),
    })
}

fn after<T: Clone>(table: &[T], count: usize) -> Vec<T> {
    table.get(count..).unwrap_or_default().to_vec()
}

/// Rewrites `styles.xml` with the additions spliced into their five elements.
pub(crate) fn rewrite(part: &str, data: &[u8], add: &Additions) -> Result<Vec<u8>> {
    let file = scan(part, data)?;
    let mut out = Vec::with_capacity(data.len() + 256);
    let mut splicer = Splicer::new(part, data);
    let mut prefix = Vec::new();
    let mut done_formats = false;
    // Sections the file does not have but our additions need. `<numFmts>` is
    // handled separately because its schema position is *first child*, before
    // even `<fonts>`; the other three sit together just before the xf tables.
    let mut pending_tables = !file.has_fonts || !file.has_fills || !file.has_borders;
    // `<dxfs>` sits after the xf tables; when the file has none at all the
    // block is written before whatever schema-later element shows up first,
    // or before `</styleSheet>` if nothing does.
    let mut pending_dxfs = !file.has_dxfs && !add.dxfs.is_empty();

    while let Some((event, span)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"styleSheet" => {
                prefix = prefix_of(e);
                out.extend_from_slice(splicer.bytes(span));
                // A workbook with no custom formats has no `<numFmts>` at all,
                // and the schema fixes its position: first child, before
                // `<fonts>`. Nothing else can carry it.
                if !file.has_num_fmts && !add.formats.is_empty() {
                    write_num_fmts(&mut out, &prefix, &add.formats, add.formats.len());
                    done_formats = true;
                }
            }

            Event::Start(e) | Event::Empty(e) if local_name(e) == b"numFmts" => {
                let count = file.declared.len() + add.formats.len();
                let sets = [Set::to(b"count", count.to_string())];
                if matches!(event, Event::Empty(_)) {
                    out.extend_from_slice(&retag(e, &sets, add.formats.is_empty()));
                    if !add.formats.is_empty() {
                        push_formats(&mut out, &prefix, &add.formats);
                        close(&mut out, &prefix, b"numFmts");
                    }
                    done_formats = true;
                } else {
                    out.extend_from_slice(&retag(e, &sets, false));
                }
            }
            Event::End(e) if end_local_name(e) == b"numFmts" => {
                if !done_formats {
                    push_formats(&mut out, &prefix, &add.formats);
                    done_formats = true;
                }
                out.extend_from_slice(splicer.bytes(span));
            }

            Event::Start(e) | Event::Empty(e) if local_name(e) == b"fonts" => {
                open_table(
                    &mut out,
                    &mut splicer,
                    e,
                    span,
                    &event,
                    file.fonts,
                    add.fonts.len(),
                );
                if matches!(event, Event::Empty(_)) && !add.fonts.is_empty() {
                    for font in &add.fonts {
                        write_font(&mut out, &prefix, font);
                    }
                    close(&mut out, &prefix, b"fonts");
                }
            }
            Event::End(e) if end_local_name(e) == b"fonts" => {
                for font in &add.fonts {
                    write_font(&mut out, &prefix, font);
                }
                out.extend_from_slice(splicer.bytes(span));
            }

            Event::Start(e) | Event::Empty(e) if local_name(e) == b"fills" => {
                open_table(
                    &mut out,
                    &mut splicer,
                    e,
                    span,
                    &event,
                    file.fills,
                    add.fills.len(),
                );
                if matches!(event, Event::Empty(_)) && !add.fills.is_empty() {
                    for fill in &add.fills {
                        write_fill(&mut out, &prefix, fill);
                    }
                    close(&mut out, &prefix, b"fills");
                }
            }
            Event::End(e) if end_local_name(e) == b"fills" => {
                for fill in &add.fills {
                    write_fill(&mut out, &prefix, fill);
                }
                out.extend_from_slice(splicer.bytes(span));
            }

            Event::Start(e) | Event::Empty(e) if local_name(e) == b"borders" => {
                open_table(
                    &mut out,
                    &mut splicer,
                    e,
                    span,
                    &event,
                    file.borders,
                    add.borders.len(),
                );
                if matches!(event, Event::Empty(_)) && !add.borders.is_empty() {
                    for border in &add.borders {
                        write_border(&mut out, &prefix, border);
                    }
                    close(&mut out, &prefix, b"borders");
                }
            }
            Event::End(e) if end_local_name(e) == b"borders" => {
                for border in &add.borders {
                    write_border(&mut out, &prefix, border);
                }
                out.extend_from_slice(splicer.bytes(span));
            }

            Event::Start(e) | Event::Empty(e)
                if matches!(local_name(e), b"cellStyleXfs" | b"cellXfs") =>
            {
                // The last chance to add a table the file left out: this is the
                // schema position for all three, after `<numFmts>` and before
                // the xf tables.
                if pending_tables {
                    write_missing_tables(&mut out, &prefix, &file, add);
                    pending_tables = false;
                }
                if local_name(e) == b"cellXfs" {
                    let count = file.cell_xfs + add.styles.len();
                    let sets = [Set::to(b"count", count.to_string())];
                    if matches!(event, Event::Empty(_)) {
                        out.extend_from_slice(&retag(e, &sets, add.styles.is_empty()));
                        if !add.styles.is_empty() {
                            push_styles(&mut out, &prefix, &add.styles);
                            close(&mut out, &prefix, b"cellXfs");
                        }
                    } else {
                        out.extend_from_slice(&retag(e, &sets, false));
                    }
                } else {
                    out.extend_from_slice(splicer.bytes(span));
                }
            }
            Event::End(e) if end_local_name(e) == b"cellXfs" => {
                push_styles(&mut out, &prefix, &add.styles);
                out.extend_from_slice(splicer.bytes(span));
            }

            Event::Start(e) | Event::Empty(e) if local_name(e) == b"dxfs" => {
                open_table(
                    &mut out,
                    &mut splicer,
                    e,
                    span,
                    &event,
                    file.dxfs,
                    add.dxfs.len(),
                );
                if matches!(event, Event::Empty(_)) && !add.dxfs.is_empty() {
                    for dxf in &add.dxfs {
                        write_dxf(&mut out, &prefix, dxf);
                    }
                    close(&mut out, &prefix, b"dxfs");
                }
            }
            Event::End(e) if end_local_name(e) == b"dxfs" => {
                for dxf in &add.dxfs {
                    write_dxf(&mut out, &prefix, dxf);
                }
                out.extend_from_slice(splicer.bytes(span));
            }

            // A file with no `<dxfs>` gets one at its schema position: before
            // the first of these, or failing that, before the sheet closes.
            Event::Start(e) | Event::Empty(e)
                if pending_dxfs
                    && matches!(local_name(e), b"tableStyles" | b"colors" | b"extLst") =>
            {
                write_dxfs_block(&mut out, &prefix, &add.dxfs);
                pending_dxfs = false;
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::End(e) if pending_dxfs && end_local_name(e) == b"styleSheet" => {
                write_dxfs_block(&mut out, &prefix, &add.dxfs);
                pending_dxfs = false;
                out.extend_from_slice(splicer.bytes(span));
            }

            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }

    Ok(out)
}

fn write_dxfs_block(out: &mut Vec<u8>, prefix: &[u8], dxfs: &[ss_model::style::Dxf]) {
    open(
        out,
        prefix,
        b"dxfs",
        &[Set::to(b"count", dxfs.len().to_string())],
        false,
    );
    for dxf in dxfs {
        write_dxf(out, prefix, dxf);
    }
    close(out, prefix, b"dxfs");
}

/// A differential format: only what the rule changes is written, because a
/// `<dxf>` field *sets* — an all-fields dxf would repaint everything.
///
/// The fill goes out backwards on purpose: a differential solid fill carries
/// its visible colour in `bgColor` with the pattern type omitted, which is
/// how Excel spells it and exactly what our own reader inverts. A dxf read
/// from a file is never re-emitted (only additions past the file's count
/// are), so `number_format` — which would need a fresh `numFmtId` — never
/// reaches this function from anything the app can author.
fn write_dxf(out: &mut Vec<u8>, prefix: &[u8], dxf: &ss_model::style::Dxf) {
    open(out, prefix, b"dxf", &[], false);
    let has_font = dxf.bold.is_some()
        || dxf.italic.is_some()
        || dxf.strike.is_some()
        || dxf.underline.is_some()
        || dxf.color.is_some();
    if has_font {
        open(out, prefix, b"font", &[], false);
        if dxf.bold == Some(true) {
            open(out, prefix, b"b", &[], true);
        } else if dxf.bold == Some(false) {
            open(out, prefix, b"b", &[Set::to(b"val", "0")], true);
        }
        if dxf.italic == Some(true) {
            open(out, prefix, b"i", &[], true);
        } else if dxf.italic == Some(false) {
            open(out, prefix, b"i", &[Set::to(b"val", "0")], true);
        }
        if dxf.strike == Some(true) {
            open(out, prefix, b"strike", &[], true);
        } else if dxf.strike == Some(false) {
            open(out, prefix, b"strike", &[Set::to(b"val", "0")], true);
        }
        if let Some(u) = dxf.underline {
            open(out, prefix, b"u", &[Set::to(b"val", u.as_str())], true);
        }
        if let Some(color) = dxf.color {
            write_color(out, prefix, b"color", color);
        }
        close(out, prefix, b"font");
    }
    if let Some(fill) = &dxf.fill {
        open(out, prefix, b"fill", &[], false);
        let sets = match &fill.pattern {
            // Solid stays unsaid: an omitted patternType inside a dxf means
            // solid, and writing it out loud is not what Excel does.
            Pattern::Solid => Vec::new(),
            Pattern::None => vec![Set::to(b"patternType", "none")],
            Pattern::Named(name) => vec![Set::to(b"patternType", name.clone())],
        };
        let has_colors = !fill.fg.is_auto() || !fill.bg.is_auto();
        open(out, prefix, b"patternFill", &sets, !has_colors);
        if has_colors {
            // Swapped relative to a table fill: the visible colour rides in
            // `bgColor` here. The reader swaps them back.
            write_color(out, prefix, b"fgColor", fill.bg);
            write_color(out, prefix, b"bgColor", fill.fg);
            close(out, prefix, b"patternFill");
        }
        close(out, prefix, b"fill");
    }
    if let Some(border) = &dxf.border {
        write_border(out, prefix, border);
    }
    close(out, prefix, b"dxf");
}

/// Re-emits a table's start tag with its count corrected.
fn open_table(
    out: &mut Vec<u8>,
    splicer: &mut Splicer<'_>,
    e: &quick_xml::events::BytesStart<'_>,
    span: std::ops::Range<usize>,
    event: &Event<'_>,
    had: usize,
    adding: usize,
) {
    if adding == 0 {
        // Not even the count changes, so the file's own bytes go back untouched
        // — whitespace, attribute order, and all.
        out.extend_from_slice(splicer.bytes(span));
        return;
    }
    // An empty `<fonts count="0"/>` has to become a start tag: the entries are
    // going inside it.
    let _ = event;
    let sets = [Set::to(b"count", (had + adding).to_string())];
    out.extend_from_slice(&retag(e, &sets, false));
}

/// Writes the tables the file omitted entirely, in schema order.
fn write_missing_tables(out: &mut Vec<u8>, prefix: &[u8], file: &Scan, add: &Additions) {
    if !file.has_fonts && !add.fonts.is_empty() {
        open(
            out,
            prefix,
            b"fonts",
            &[Set::to(b"count", add.fonts.len().to_string())],
            false,
        );
        for font in &add.fonts {
            write_font(out, prefix, font);
        }
        close(out, prefix, b"fonts");
    }
    if !file.has_fills && !add.fills.is_empty() {
        open(
            out,
            prefix,
            b"fills",
            &[Set::to(b"count", add.fills.len().to_string())],
            false,
        );
        for fill in &add.fills {
            write_fill(out, prefix, fill);
        }
        close(out, prefix, b"fills");
    }
    if !file.has_borders && !add.borders.is_empty() {
        open(
            out,
            prefix,
            b"borders",
            &[Set::to(b"count", add.borders.len().to_string())],
            false,
        );
        for border in &add.borders {
            write_border(out, prefix, border);
        }
        close(out, prefix, b"borders");
    }
}

fn write_num_fmts(out: &mut Vec<u8>, prefix: &[u8], formats: &[(u32, String)], count: usize) {
    open(
        out,
        prefix,
        b"numFmts",
        &[Set::to(b"count", count.to_string())],
        false,
    );
    push_formats(out, prefix, formats);
    close(out, prefix, b"numFmts");
}

fn push_formats(out: &mut Vec<u8>, prefix: &[u8], formats: &[(u32, String)]) {
    for (id, code) in formats {
        open(
            out,
            prefix,
            b"numFmt",
            &[
                Set::to(b"numFmtId", id.to_string()),
                Set::to(b"formatCode", code.clone()),
            ],
            true,
        );
    }
}

/// A font, in the order Excel writes one.
///
/// `CT_Font` is a repeated choice rather than a sequence, so the order is not
/// forced by the schema — but matching Excel's keeps a diff against a file it
/// produced readable, which is the whole reason the fidelity harness can see
/// anything at all.
fn write_font(out: &mut Vec<u8>, prefix: &[u8], font: &Font) {
    open(out, prefix, b"font", &[], false);
    if font.bold {
        open(out, prefix, b"b", &[], true);
    }
    if font.italic {
        open(out, prefix, b"i", &[], true);
    }
    if font.strike {
        open(out, prefix, b"strike", &[], true);
    }
    if !font.underline.is_none() {
        open(
            out,
            prefix,
            b"u",
            &[Set::to(b"val", font.underline.as_str())],
            true,
        );
    }
    if let Some(vert) = font.vert_align {
        let val = match vert {
            ss_model::style::VertAlign::Superscript => "superscript",
            ss_model::style::VertAlign::Subscript => "subscript",
        };
        open(out, prefix, b"vertAlign", &[Set::to(b"val", val)], true);
    }
    open(
        out,
        prefix,
        b"sz",
        &[Set::to(b"val", ss_model::format_general(font.size))],
        true,
    );
    write_color(out, prefix, b"color", font.color);
    open(
        out,
        prefix,
        b"name",
        &[Set::to(b"val", font.name.clone())],
        true,
    );
    close(out, prefix, b"font");
}

fn write_fill(out: &mut Vec<u8>, prefix: &[u8], fill: &Fill) {
    open(out, prefix, b"fill", &[], false);
    let pattern = match &fill.pattern {
        Pattern::None => "none",
        Pattern::Solid => "solid",
        Pattern::Named(name) => name.as_str(),
    };
    let has_colors = !fill.fg.is_auto() || !fill.bg.is_auto();
    open(
        out,
        prefix,
        b"patternFill",
        &[Set::to(b"patternType", pattern)],
        !has_colors,
    );
    if has_colors {
        write_color(out, prefix, b"fgColor", fill.fg);
        write_color(out, prefix, b"bgColor", fill.bg);
        close(out, prefix, b"patternFill");
    }
    close(out, prefix, b"fill");
}

/// A border. All five edges are written even when empty, because `CT_Border`
/// *is* a sequence and a missing `<left>` moves every edge after it.
fn write_border(out: &mut Vec<u8>, prefix: &[u8], border: &Border) {
    let mut sets = Vec::new();
    if border.diagonal_up {
        sets.push(Set::to(b"diagonalUp", "1"));
    }
    if border.diagonal_down {
        sets.push(Set::to(b"diagonalDown", "1"));
    }
    open(out, prefix, b"border", &sets, false);
    for (name, edge) in [
        (b"left".as_slice(), border.left),
        (b"right".as_slice(), border.right),
        (b"top".as_slice(), border.top),
        (b"bottom".as_slice(), border.bottom),
        (b"diagonal".as_slice(), border.diagonal),
    ] {
        write_edge(out, prefix, name, edge);
    }
    close(out, prefix, b"border");
}

fn write_edge(out: &mut Vec<u8>, prefix: &[u8], name: &[u8], edge: Edge) {
    if edge.is_none() {
        open(out, prefix, name, &[], true);
        return;
    }
    let styled = [Set::to(b"style", edge.style.as_str())];
    if edge.color.is_auto() {
        open(out, prefix, name, &styled, true);
        return;
    }
    open(out, prefix, name, &styled, false);
    write_color(out, prefix, b"color", edge.color);
    close(out, prefix, name);
}

/// One of the colour elements, or nothing at all for `Auto`.
///
/// Writing `<color auto="1"/>` would be legal but is not what Excel does for an
/// unset colour, and the difference shows up in every diff.
fn write_color(out: &mut Vec<u8>, prefix: &[u8], name: &[u8], color: Color) {
    let sets = match color {
        Color::Auto => return,
        Color::Rgb(_) => vec![Set::to(b"rgb", color.to_hex().unwrap_or_default())],
        Color::Indexed(i) => vec![Set::to(b"indexed", i.to_string())],
        Color::Theme { index, tint } => {
            let mut sets = vec![Set::to(b"theme", index.to_string())];
            if tint != 0.0 {
                sets.push(Set::to(b"tint", ss_model::format_general(tint)));
            }
            sets
        }
    };
    open(out, prefix, name, &sets, true);
}

fn push_styles(out: &mut Vec<u8>, prefix: &[u8], styles: &[CellFormat]) {
    for xf in styles {
        let mut sets = vec![
            Set::to(b"numFmtId", xf.num_fmt_id.to_string()),
            Set::to(b"fontId", xf.font.to_string()),
            Set::to(b"fillId", xf.fill.to_string()),
            Set::to(b"borderId", xf.border.to_string()),
            Set::to(b"xfId", xf.xf_id.to_string()),
        ];
        // The `apply*` flags say "this xf's own value overrides the named style
        // it is based on". Everything we author is direct formatting, so each
        // one is set exactly when the field is not the default.
        if xf.num_fmt_id != 0 {
            sets.push(Set::to(b"applyNumberFormat", "1"));
        }
        if xf.font != 0 {
            sets.push(Set::to(b"applyFont", "1"));
        }
        if xf.fill != 0 {
            sets.push(Set::to(b"applyFill", "1"));
        }
        if xf.border != 0 {
            sets.push(Set::to(b"applyBorder", "1"));
        }
        let aligned = !xf.alignment.is_default();
        if aligned {
            sets.push(Set::to(b"applyAlignment", "1"));
        }
        if xf.quote_prefix {
            sets.push(Set::to(b"quotePrefix", "1"));
        }
        // Only an *unlocked* cell needs saying: locked is what an xf means
        // when it says nothing, and writing it out everywhere would put a
        // `<protection>` on every style in the workbook to no effect.
        let unlocked = !xf.locked;
        if unlocked {
            sets.push(Set::to(b"applyProtection", "1"));
        }
        let body = aligned || unlocked;
        open(out, prefix, b"xf", &sets, !body);
        if aligned {
            let a = xf.alignment;
            let mut align = Vec::new();
            if let Some(h) = a.horizontal.as_str() {
                align.push(Set::to(b"horizontal", h));
            }
            if let Some(v) = a.vertical.as_str() {
                align.push(Set::to(b"vertical", v));
            }
            if a.rotation != 0 {
                align.push(Set::to(b"textRotation", a.rotation.to_string()));
            }
            if a.wrap {
                align.push(Set::to(b"wrapText", "1"));
            }
            if a.indent != 0 {
                align.push(Set::to(b"indent", a.indent.to_string()));
            }
            if a.shrink {
                align.push(Set::to(b"shrinkToFit", "1"));
            }
            open(out, prefix, b"alignment", &align, true);
        }
        if unlocked {
            open(out, prefix, b"protection", &[Set::to(b"locked", "0")], true);
        }
        if body {
            close(out, prefix, b"xf");
        }
    }
}

/// What styles.xml already contains.
struct Scan {
    declared: std::collections::BTreeSet<u32>,
    has_num_fmts: bool,
    has_fonts: bool,
    has_fills: bool,
    has_borders: bool,
    has_dxfs: bool,
    fonts: usize,
    fills: usize,
    borders: usize,
    cell_xfs: usize,
    dxfs: usize,
}

fn scan(part: &str, data: &[u8]) -> Result<Scan> {
    let mut splicer = Splicer::new(part, data);
    let mut out = Scan {
        declared: Default::default(),
        has_num_fmts: false,
        has_fonts: false,
        has_fills: false,
        has_borders: false,
        has_dxfs: false,
        fonts: 0,
        fills: 0,
        borders: 0,
        cell_xfs: 0,
        dxfs: 0,
    };
    // `<font>`, `<fill>`, and `<border>` all appear inside `<dxfs>` too, where
    // they are differential formats and not table entries. Counting those would
    // make every appended font land past the end of the table.
    let mut section: &[u8] = b"";
    let mut depth_in_dxfs = false;

    while let Some((event, _)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) => match local_name(e) {
                b"numFmts" => out.has_num_fmts = true,
                b"numFmt" if !depth_in_dxfs => {
                    if let Some(id) = raw_attr(e, b"numFmtId").and_then(|a| parse_u32(&a.value)) {
                        out.declared.insert(id);
                    }
                }
                b"fonts" => {
                    out.has_fonts = true;
                    section = b"fonts";
                }
                b"fills" => {
                    out.has_fills = true;
                    section = b"fills";
                }
                b"borders" => {
                    out.has_borders = true;
                    section = b"borders";
                }
                b"cellXfs" => section = b"cellXfs",
                b"dxfs" => {
                    out.has_dxfs = true;
                    depth_in_dxfs = true;
                }
                b"dxf" if depth_in_dxfs => out.dxfs += 1,
                b"font" if section == b"fonts" => out.fonts += 1,
                b"fill" if section == b"fills" => out.fills += 1,
                b"border" if section == b"borders" => out.borders += 1,
                // `<cellStyleXfs>` holds `<xf>` elements too, and a cell's `s`
                // attribute does not index it.
                b"xf" if section == b"cellXfs" => out.cell_xfs += 1,
                _ => {}
            },
            Event::End(e) => match end_local_name(e) {
                b"fonts" | b"fills" | b"borders" | b"cellXfs" => section = b"",
                b"dxfs" => depth_in_dxfs = false,
                _ => {}
            },
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::style::{BorderStyle, HAlign, StyleId};
    use std::collections::BTreeMap;

    const STYLES: &str = concat!(
        r#"<?xml version="1.0"?><styleSheet xmlns="http://x">"#,
        r#"<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>"#,
        r#"<fills count="2"><fill><patternFill patternType="none"/></fill>"#,
        r#"<fill><patternFill patternType="gray125"/></fill></fills>"#,
        r#"<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>"#,
        r#"<cellStyleXfs count="1"><xf numFmtId="0"/></cellStyleXfs>"#,
        r#"<cellXfs count="1"><xf numFmtId="0" fontId="0"/></cellXfs>"#,
        r#"<dxfs count="1"><dxf><font><b/></font><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill></dxf></dxfs>"#,
        r#"</styleSheet>"#,
    );

    fn table() -> StyleTable {
        StyleTable::build(&BTreeMap::new(), &[0])
    }

    fn rewritten(styles: &StyleTable) -> String {
        let add = additions("styles.xml", STYLES.as_bytes(), styles).expect("scans");
        let out = rewrite("styles.xml", STYLES.as_bytes(), &add).expect("writes");
        String::from_utf8(out).expect("utf-8")
    }

    #[test]
    fn a_workbook_nobody_edited_comes_back_unchanged() {
        assert_eq!(rewritten(&table()), STYLES);
    }

    #[test]
    fn the_dxfs_tables_own_fonts_and_fills_are_not_counted_as_table_entries() {
        // `<dxf>` holds a `<font>` and a `<fill>` that are differential formats,
        // not entries. Counting them would place every appended font past the
        // end of the real table, and a cell pointing at it would get whatever
        // Excel decided a missing font looks like.
        let scan = scan("styles.xml", STYLES.as_bytes()).expect("scans");
        assert_eq!(scan.fonts, 1);
        assert_eq!(scan.fills, 2);
        assert_eq!(scan.borders, 1);
        assert_eq!(scan.cell_xfs, 1);
    }

    #[test]
    fn an_authored_dxf_is_appended_and_spelt_backwards() {
        let mut styles = table();
        // The fixture already carries one dxf; in the app the reader would
        // have put it in the table, so the test stands in for that first.
        let replica = ss_model::style::Dxf {
            bold: Some(true),
            fill: Some(Fill::solid(Color::rgb(0xFF, 0xC7, 0xCE))),
            ..Default::default()
        };
        styles.add_dxf(replica);
        let before = styles.dxfs().len();
        let dxf = ss_model::style::Dxf {
            italic: Some(true),
            fill: Some(Fill::solid(Color::rgb(0xC6, 0xEF, 0xCE))),
            ..Default::default()
        };
        let id = styles.add_dxf(dxf);
        assert_eq!(id as usize, before, "appended past the file's own");

        let out = rewritten(&styles);
        assert!(
            out.contains(&format!(r#"<dxfs count="{}""#, before + 1)),
            "{out}"
        );
        // The visible colour of a differential solid fill rides in bgColor
        // with the pattern type unsaid — Excel's spelling, and our reader's
        // exact inverse.
        assert!(
            out.contains(
                r#"<dxf><font><i/></font><fill><patternFill><bgColor rgb="FFC6EFCE"/></patternFill></fill></dxf>"#
            ),
            "{out}"
        );
    }

    #[test]
    fn a_typed_date_adds_a_style_that_points_at_a_builtin() {
        let mut styles = table();
        styles.style_for_format("mm-dd-yy");
        let out = rewritten(&styles);

        assert!(out.contains(r#"<cellXfs count="2">"#), "{out}");
        assert!(out.contains(r#"<xf numFmtId="14""#), "{out}");
        assert!(
            !out.contains("<numFmts"),
            "a built-in format is never declared: {out}"
        );
        assert_eq!(
            out.matches("<font>").count(),
            2,
            "one in the fonts table and the dxf's, and no new one: {out}"
        );
    }

    #[test]
    fn a_format_excel_has_no_id_for_gets_a_numfmts_element() {
        let mut styles = table();
        styles.style_for_format("0.000");
        let out = rewritten(&styles);

        // The schema fixes `<numFmts>` as the first child, before `<fonts>`.
        let fmts = out.find("<numFmts").expect("declared");
        let fonts = out.find("<fonts").expect("still there");
        assert!(fmts < fonts, "{out}");
        assert!(
            out.contains(r#"<numFmt numFmtId="164" formatCode="0.000"/>"#),
            "{out}"
        );
    }

    #[test]
    fn bolding_a_cell_appends_a_font_and_points_a_new_xf_at_it() {
        let mut styles = table();
        let bold = styles.restyle(StyleId::DEFAULT, |look| look.font.bold = true);
        assert_eq!(bold, StyleId(1));
        let out = rewritten(&styles);

        assert!(out.contains(r#"<fonts count="2">"#), "{out}");
        assert!(
            out.contains(r#"<font><b/><sz val="11"/><name val="Calibri"/></font>"#),
            "{out}"
        );
        assert!(
            out.contains(
                r#"<xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>"#
            ),
            "{out}"
        );
        assert!(
            out.contains(r#"<fills count="2">"#),
            "an untouched table keeps its own count: {out}"
        );
    }

    #[test]
    fn a_fill_a_border_and_an_alignment_all_land_in_their_own_tables() {
        let mut styles = table();
        styles.restyle(StyleId::DEFAULT, |look| {
            look.fill = Fill::solid(Color::rgb(0xFF, 0xEB, 0x9C));
            look.border = Border::all(BorderStyle::Thin);
            look.alignment.horizontal = HAlign::Center;
            look.alignment.wrap = true;
        });
        let out = rewritten(&styles);

        assert!(out.contains(r#"<fills count="3">"#), "{out}");
        assert!(
            out.contains(
                r#"<patternFill patternType="solid"><fgColor rgb="FFFFEB9C"/></patternFill>"#
            ),
            "{out}"
        );
        assert!(out.contains(r#"<borders count="2">"#), "{out}");
        assert!(
            out.contains(
                r#"<border><left style="thin"/><right style="thin"/><top style="thin"/><bottom style="thin"/><diagonal/></border>"#
            ),
            "every edge is written, in order, or the sequence shifts: {out}"
        );
        assert!(
            out.contains(r#"<alignment horizontal="center" wrapText="1"/>"#),
            "{out}"
        );
        assert!(out.contains(r#"applyAlignment="1""#), "{out}");
    }

    #[test]
    fn an_existing_numfmts_element_is_added_to_rather_than_replaced() {
        let file = concat!(
            r#"<styleSheet><numFmts count="1">"#,
            r#"<numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0"/></numFmts>"#,
            r#"<fonts count="1"><font/></fonts>"#,
            r#"<fills count="2"><fill/><fill/></fills>"#,
            r#"<borders count="1"><border/></borders>"#,
            r#"<cellXfs count="1"><xf numFmtId="164"/></cellXfs></styleSheet>"#,
        );
        let codes = BTreeMap::from([(164, "\"$\"#,##0".to_string())]);
        let mut styles = StyleTable::build(&codes, &[164]);
        // Not one of Excel's built-ins, so it has to be declared.
        styles.style_for_format("0.0000");

        let add = additions("styles.xml", file.as_bytes(), &styles).expect("scans");
        let out = String::from_utf8(rewrite("styles.xml", file.as_bytes(), &add).expect("writes"))
            .expect("utf-8");

        assert!(
            out.contains(r#"<numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0"/>"#),
            "the file's own escaping survives: {out}"
        );
        assert!(
            out.contains(r#"numFmtId="165" formatCode="0.0000""#),
            "{out}"
        );
        assert!(out.contains(r#"<numFmts count="2">"#), "{out}");
    }
}
