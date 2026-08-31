//! `<style:*-properties>` — everything a style says, in the vocabulary it says
//! it in.
//!
//! ODF states formatting in the words of XSL-FO wherever XSL-FO had one —
//! `fo:font-size`, `fo:margin-left`, `fo:text-align` — and in its own only
//! where it did not. That is why a run's weight is `fo:font-weight="bold"`
//! rather than a `<w:b/>` that is on by being present, and it is the single
//! largest difference between reading this format and reading the other one:
//! **nothing here is a toggle that means true by existing.** Every property
//! carries its value, so a property that is absent is genuinely absent, and a
//! style that turns bold *off* has said so rather than merely gone quiet.
//!
//! What is not done here is inheritance. A style says what it changes and names
//! its parent; the model resolves the chain later and this reader flattens
//! nothing, which is the rule `wp-model` states at the top of its own crate and
//! the reason a paragraph in a document whose default face is Calibri does not
//! come out with Calibri written on every run of it.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::prop::{
    Border, BorderStyle, Justify, Lang, LineSpacing, ParaBorders, ParaProps, RunProps, Shading,
    ShadingPattern, TabKind, TabLeader, TabStop, Toggle, Underline, UnderlineKind, VertAlign,
};
use wp_model::table::{
    CellMargins, CellProps, CellVAlign, RowHeight, RowProps, TableBorders, TableProps, Width,
};
use wp_model::units::{Eighth, HalfPoint, Line240, Twips};

use crate::fonts::FontFaces;
use crate::xml::{
    attr, attr_bool, attr_in, attr_length, boolean, color, end_local_name, length, local_name,
    percent,
};

/// Everything one `<style:style>` states, before anything is resolved.
///
/// One type for every style family, because ODF puts them all in the same
/// element and tells them apart by an attribute. A paragraph style leaves the
/// table fields empty and a table-cell style leaves the paragraph ones empty,
/// which costs a few words of memory and saves a family-shaped enum that every
/// caller would have to take apart.
#[derive(Debug, Default, Clone)]
pub struct Props {
    pub para: ParaProps,
    pub run: RunProps,
    pub table: Option<TableProps>,
    pub row: Option<RowProps>,
    pub cell: Option<CellProps>,
    /// A column's width, which is a style rather than a property of the column
    /// element — ODF puts every measurement in a style.
    pub column: Option<Width>,
    /// `style:parent-style-name`, unresolved: a style may name a parent that
    /// stands later in the file.
    pub parent: Option<String>,
    /// `style:next-style-name`.
    pub next: Option<String>,
    /// `style:list-style-name` — a paragraph style may carry the list it is
    /// numbered by, which is how a heading gets its outline number.
    pub list_style: Option<String>,
    /// `style:master-page-name`. A section break, spelled as a property of the
    /// first paragraph after it.
    pub master_page: Option<String>,
    /// How text goes round a frame in this style.
    pub wrap: Option<wp_model::doc::Wrap>,
}

/// Reads the `<style:*-properties>` children of whatever element is open, up to
/// its end tag.
///
/// The same handful of elements appear under a named style, an automatic style,
/// a list level, a page layout and a default style, so this is written once and
/// called from all of them.
pub fn properties(reader: &mut Reader<&[u8]>, end: &[u8], faces: &FontFaces, props: &mut Props) {
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return,
        };
        // Only a start tag has a body to read past. An element written empty —
        // `<style:paragraph-properties/>` — has no end tag, and a reader that
        // went looking for one would swallow everything up to the next
        // unrelated one.
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"text-properties" => text_properties(&e, faces, &mut props.run),
                    b"paragraph-properties" => {
                        paragraph_properties(&e, &mut props.para);
                        if !empty {
                            para_children(reader, &mut props.para);
                        }
                    }
                    b"table-properties" => {
                        table_properties(&e, props.table.get_or_insert_with(TableProps::default))
                    }
                    b"table-row-properties" => {
                        row_properties(&e, props.row.get_or_insert_with(RowProps::default))
                    }
                    b"table-column-properties" => props.column = column_width(&e),
                    b"table-cell-properties" => {
                        cell_properties(&e, props.cell.get_or_insert_with(CellProps::default))
                    }
                    b"graphic-properties" => props.wrap = wrap(&e),
                    _ => {}
                }
                if !empty && name != b"paragraph-properties" {
                    crate::xml::skip_element(reader, &name);
                }
            }
            Event::End(e) if end_local_name(&e) == end => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

/// The children of `<style:paragraph-properties>`, of which the tab stops are
/// the only ones that reach the model.
fn para_children(reader: &mut Reader<&[u8]>, para: &mut ParaProps) {
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                if name == b"tab-stops" {
                    if !empty {
                        let stops = tab_stops(reader);
                        if !stops.is_empty() {
                            para.tabs = Some(stops);
                        }
                    }
                } else if !empty {
                    crate::xml::skip_element(reader, &name);
                }
            }
            Event::End(e) if end_local_name(&e) == b"paragraph-properties" => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

/// `<style:text-properties>` — everything about a run.
pub fn text_properties(e: &BytesStart<'_>, faces: &FontFaces, run: &mut RunProps) {
    // A face may be named twice over: `style:font-name` points into
    // `<office:font-face-decls>`, and `fo:font-family` states the family
    // outright. The declaration wins where there is one, because that is what
    // also carries the pitch and the generic family a substitution needs.
    if let Some(family) = attr_in(e, b"style", b"font-name").and_then(|n| faces.family(&n)) {
        run.fonts.ascii = Some(family.clone());
        run.fonts.high_ansi = Some(family);
    } else if let Some(family) = attr_in(e, b"fo", b"font-family") {
        let family: std::sync::Arc<str> = unquoted(&family).into();
        run.fonts.ascii = Some(family.clone());
        run.fonts.high_ansi = Some(family);
    }
    if let Some(name) = attr_in(e, b"style", b"font-name-asian") {
        run.fonts.east_asian = faces.family(&name);
    }
    if let Some(name) = attr_in(e, b"style", b"font-name-complex") {
        run.fonts.complex = faces.family(&name);
    }

    if let Some(size) = attr_in(e, b"fo", b"font-size")
        .as_deref()
        .and_then(font_size)
    {
        run.size = Some(size);
    }
    if let Some(size) = attr_in(e, b"style", b"font-size-complex")
        .as_deref()
        .and_then(font_size)
    {
        run.size_complex = Some(size);
    }
    if let Some(weight) = attr_in(e, b"fo", b"font-weight") {
        run.toggles.set(Toggle::Bold, bold(&weight));
    }
    if let Some(weight) = attr_in(e, b"style", b"font-weight-complex") {
        run.toggles.set(Toggle::BoldCs, bold(&weight));
    }
    if let Some(style) = attr_in(e, b"fo", b"font-style") {
        run.toggles.set(Toggle::Italic, italic(&style));
    }
    if let Some(style) = attr_in(e, b"style", b"font-style-complex") {
        run.toggles.set(Toggle::ItalicCs, italic(&style));
    }
    if let Some(transform) = attr_in(e, b"fo", b"text-transform") {
        run.toggles.set(Toggle::Caps, transform == "uppercase");
    }
    if let Some(variant) = attr_in(e, b"fo", b"font-variant") {
        run.toggles.set(Toggle::SmallCaps, variant == "small-caps");
    }
    if let Some(style) = attr_in(e, b"style", b"text-line-through-style") {
        let struck = style != "none";
        let double = attr_in(e, b"style", b"text-line-through-type").as_deref() == Some("double");
        run.toggles.set(Toggle::Strike, struck && !double);
        run.toggles.set(Toggle::DoubleStrike, struck && double);
    }
    if let Some(relief) = attr_in(e, b"style", b"font-relief") {
        run.toggles.set(Toggle::Emboss, relief == "embossed");
        run.toggles.set(Toggle::Imprint, relief == "engraved");
    }
    if let Some(outline) = attr_in(e, b"style", b"text-outline").and_then(|v| boolean(&v)) {
        run.toggles.set(Toggle::Outline, outline);
    }
    if let Some(shadow) = attr_in(e, b"fo", b"text-shadow") {
        run.toggles.set(Toggle::Shadow, shadow != "none");
    }
    // `text:display="none"` is ODF's hidden text. It is spelled as a display
    // property rather than as a character one, and a reader that passed over it
    // would lay out text that no application shows.
    if let Some(display) = attr_in(e, b"text", b"display") {
        run.toggles.set(Toggle::Vanish, display == "none");
    }

    if let Some(underline) = attr_in(e, b"style", b"text-underline-style") {
        run.underline = Some(Underline {
            kind: underline_kind(
                &underline,
                attr_in(e, b"style", b"text-underline-type").as_deref(),
                attr_in(e, b"style", b"text-underline-width").as_deref(),
            ),
            color: attr_in(e, b"style", b"text-underline-color")
                .as_deref()
                .and_then(color),
        });
    }
    if let Some(value) = attr_in(e, b"fo", b"color").as_deref().and_then(color) {
        run.color = Some(value);
    }
    if let Some(fill) = attr_in(e, b"fo", b"background-color") {
        run.shading = Some(Shading {
            pattern: ShadingPattern::Clear,
            fill: color(&fill),
            color: None,
        });
    }
    if let Some(position) = attr_in(e, b"style", b"text-position") {
        run.vert_align = vertical(&position);
    }
    if let Some(spacing) = attr_in(e, b"fo", b"letter-spacing")
        .as_deref()
        .and_then(length)
    {
        run.letter_spacing = Some(spacing);
    }
    if let Some(scale) = attr_in(e, b"style", b"text-scale")
        .as_deref()
        .and_then(percent)
    {
        run.scale = Some(scale.round().clamp(1.0, 600.0) as u16);
    }
    let language = attr_in(e, b"fo", b"language");
    let country = attr_in(e, b"fo", b"country");
    let asian = (
        attr_in(e, b"style", b"language-asian"),
        attr_in(e, b"style", b"country-asian"),
    );
    let complex = (
        attr_in(e, b"style", b"language-complex"),
        attr_in(e, b"style", b"country-complex"),
    );
    // ODF states the language and the country as two attributes where the model
    // keeps one tag, and it has three of them because a run may be in one
    // language for its Latin text and another for its Arabic.
    if language.is_some() || country.is_some() || asian.0.is_some() || complex.0.is_some() {
        let lang = run.lang.get_or_insert_with(Lang::default);
        if language.is_some() || country.is_some() {
            lang.value = Some(tag(language.as_deref(), country.as_deref()));
        }
        if asian.0.is_some() || asian.1.is_some() {
            lang.east_asian = Some(tag(asian.0.as_deref(), asian.1.as_deref()));
        }
        if complex.0.is_some() || complex.1.is_some() {
            lang.complex = Some(tag(complex.0.as_deref(), complex.1.as_deref()));
        }
    }
}

/// `<style:paragraph-properties>`, but not its children: tab stops are an
/// element of their own and the caller reads them.
pub fn paragraph_properties(e: &BytesStart<'_>, para: &mut ParaProps) {
    if let Some(align) = attr_in(e, b"fo", b"text-align") {
        para.justify = Some(justify(&align));
    }
    if let Some(start) = attr_in(e, b"fo", b"margin-left")
        .as_deref()
        .and_then(length)
    {
        para.indent.start = Some(start);
    }
    if let Some(end) = attr_in(e, b"fo", b"margin-right")
        .as_deref()
        .and_then(length)
    {
        para.indent.end = Some(end);
    }
    // ODF has one attribute where WordprocessingML has two: a negative
    // `fo:text-indent` is a hanging indent. The model keeps the two apart
    // because the format it usually reads does.
    if let Some(first) = attr_in(e, b"fo", b"text-indent")
        .as_deref()
        .and_then(length)
    {
        match first.0 < 0 {
            true => para.indent.hanging = Some(Twips(-first.0)),
            false => para.indent.first_line = Some(first),
        }
    }
    if let Some(before) = attr_in(e, b"fo", b"margin-top").as_deref().and_then(length) {
        para.spacing.before = Some(before);
    }
    if let Some(after) = attr_in(e, b"fo", b"margin-bottom")
        .as_deref()
        .and_then(length)
    {
        para.spacing.after = Some(after);
    }
    if let Some(height) = attr_in(e, b"fo", b"line-height") {
        para.spacing.line = line_height(&height);
    }
    if let Some(least) = attr_in(e, b"style", b"line-height-at-least")
        .as_deref()
        .and_then(length)
    {
        para.spacing.line = Some(LineSpacing::AtLeast(least));
    }
    if let Some(keep) = attr_in(e, b"fo", b"keep-with-next") {
        para.keep_next = Some(keep != "auto");
    }
    if let Some(keep) = attr_in(e, b"fo", b"keep-together") {
        para.keep_lines = Some(keep != "auto");
    }
    if let Some(brk) = attr_in(e, b"fo", b"break-before") {
        para.page_break_before = Some(brk == "page" || brk == "column");
    }
    // Word has one switch for widow and orphan control together; ODF states the
    // two counts separately, and either being present at all is the switch.
    if attr_in(e, b"fo", b"orphans").is_some() || attr_in(e, b"fo", b"widows").is_some() {
        para.widow_control = Some(true);
    }
    if let Some(fill) = attr_in(e, b"fo", b"background-color") {
        para.shading = Some(Shading {
            pattern: ShadingPattern::Clear,
            fill: color(&fill),
            color: None,
        });
    }
    if let Some(mode) = attr_in(e, b"style", b"writing-mode") {
        para.bidi = Some(mode.starts_with("rl"));
    }
    if let Some(numbered) = attr_in(e, b"text", b"number-lines").and_then(|v| boolean(&v)) {
        para.suppress_line_numbers = Some(!numbered);
    }
    if let Some(borders) = para_borders(e) {
        para.borders = Some(Box::new(borders));
    }
}

fn para_borders(e: &BytesStart<'_>) -> Option<ParaBorders> {
    let all = attr_in(e, b"fo", b"border").as_deref().and_then(border);
    // `fo:padding` is the clear space between the border and the text, which is
    // what `w:space` is on the other side and what the model keeps. A paragraph
    // with padding and no border has nowhere to put it — the model has no field
    // for a gap that nothing is drawn at the end of — and loses it, which costs
    // the page nothing because nothing is drawn there.
    let space = |name: &[u8]| {
        attr_in(e, b"fo", name)
            .as_deref()
            .or(attr_in(e, b"fo", b"padding").as_deref())
            .and_then(length)
            .map(|pad| pad.points().round().clamp(0.0, 31.0) as u8)
    };
    let side = |name: &[u8], pad: &[u8]| {
        let mut border = attr_in(e, b"fo", name)
            .as_deref()
            .and_then(border)
            .or(all)?;
        border.space = space(pad);
        Some(border)
    };
    let (top, start, bottom, end) = (
        side(b"border-top", b"padding-top"),
        side(b"border-left", b"padding-left"),
        side(b"border-bottom", b"padding-bottom"),
        side(b"border-right", b"padding-right"),
    );
    if top.is_none() && start.is_none() && bottom.is_none() && end.is_none() {
        return None;
    }
    Some(ParaBorders {
        top,
        start,
        bottom,
        end,
        between: None,
        bar: None,
    })
}

/// `<style:table-properties>`.
pub fn table_properties(e: &BytesStart<'_>, table: &mut TableProps) {
    // **A relative width outranks the absolute one beside it, and is not
    // allowed past the column.** A producer writes both — `style:width` as the
    // measurement and `style:rel-width` as the intent — and where they
    // disagree the intent is what the reference draws: measured on a table
    // stating 7.31in and 112.8%, which came out at the column's own 6.5in and
    // not at the inch and a quarter over. A grid that then does not fit is
    // scaled to what does.
    if let Some(share) = attr_in(e, b"style", b"rel-width")
        .as_deref()
        .and_then(percent)
    {
        table.width = Width::Percent(wp_model::Pct50::from_percent(share.min(100.0)));
    } else if let Some(width) = attr_in(e, b"style", b"width").as_deref().and_then(length) {
        table.width = Width::Fixed(width);
    }
    // Every column of an ODF table states its own width, so the grid is the
    // layout rather than a hint the widest cell may overrule.
    table.layout = wp_model::table::TableLayout::Fixed;
    if let Some(align) = attr_in(e, b"table", b"align") {
        table.justify = Some(match align.as_str() {
            "center" => Justify::Center,
            "right" => Justify::End,
            _ => Justify::Start,
        });
    }
    if let Some(indent) = attr_in(e, b"fo", b"margin-left")
        .as_deref()
        .and_then(length)
    {
        table.indent = Some(Width::Fixed(indent));
    }
    if let Some(fill) = attr_in(e, b"fo", b"background-color") {
        table.shading = Some(Shading {
            pattern: ShadingPattern::Clear,
            fill: color(&fill),
            color: None,
        });
    }
}

/// `<style:table-row-properties>`.
pub fn row_properties(e: &BytesStart<'_>, row: &mut RowProps) {
    // Two attributes, and which one is present is the difference between a row
    // that may grow and one that may not.
    if let Some(least) = attr_in(e, b"style", b"min-row-height")
        .as_deref()
        .and_then(length)
    {
        row.height = Some(RowHeight::AtLeast(least));
    } else if let Some(exact) = attr_in(e, b"style", b"row-height")
        .as_deref()
        .and_then(length)
    {
        row.height = Some(RowHeight::Exact(exact));
    }
    if let Some(broken) = attr_bool(e, b"keep-together") {
        row.cant_split = !broken;
    }
}

fn column_width(e: &BytesStart<'_>) -> Option<Width> {
    if let Some(width) = attr_in(e, b"style", b"column-width")
        .as_deref()
        .and_then(length)
    {
        return Some(Width::Fixed(width));
    }
    attr_in(e, b"style", b"rel-column-width")
        .as_deref()
        .and_then(relative_width)
        .map(Width::Percent)
}

/// `style:rel-column-width` is `1234*` — a share of the table, in units nobody
/// declares. The caller turns the shares into a grid; here it is kept as the
/// proportion it is.
fn relative_width(text: &str) -> Option<wp_model::Pct50> {
    let share: f64 = text.trim().strip_suffix('*')?.trim().parse().ok()?;
    Some(wp_model::Pct50(share.round() as i32))
}

/// `<style:table-cell-properties>`.
pub fn cell_properties(e: &BytesStart<'_>, cell: &mut CellProps) {
    if let Some(fill) = attr_in(e, b"fo", b"background-color") {
        cell.shading = Some(Shading {
            pattern: ShadingPattern::Clear,
            fill: color(&fill),
            color: None,
        });
    }
    if let Some(align) = attr_in(e, b"style", b"vertical-align") {
        cell.v_align = match align.as_str() {
            "middle" => CellVAlign::Center,
            "bottom" => CellVAlign::Bottom,
            _ => CellVAlign::Top,
        };
    }
    let all = attr_in(e, b"fo", b"border").as_deref().and_then(border);
    let side = |name: &[u8]| attr_in(e, b"fo", name).as_deref().and_then(border).or(all);
    cell.borders = TableBorders {
        top: side(b"border-top"),
        start: side(b"border-left"),
        bottom: side(b"border-bottom"),
        end: side(b"border-right"),
        ..TableBorders::default()
    };
    let padding = attr_in(e, b"fo", b"padding").as_deref().and_then(length);
    let pad = |name: &[u8]| {
        attr_in(e, b"fo", name)
            .as_deref()
            .and_then(length)
            .or(padding)
            .map(Width::Fixed)
    };
    cell.margins = CellMargins {
        top: pad(b"padding-top"),
        start: pad(b"padding-left"),
        bottom: pad(b"padding-bottom"),
        end: pad(b"padding-right"),
    };
}

/// `<style:graphic-properties style:wrap="...">` — how the text goes round a
/// frame.
///
/// `run-through` is a frame the text ignores, which is what the model calls no
/// wrap at all; `none` is the opposite and means the text is pushed clear above
/// and below. The two names read backwards from the model's, and reading one
/// for the other puts a floating picture on top of the paragraph it should
/// have moved.
fn wrap(e: &BytesStart<'_>) -> Option<wp_model::doc::Wrap> {
    use wp_model::doc::Wrap;
    Some(match attr_in(e, b"style", b"wrap")?.as_str() {
        "none" => Wrap::TopAndBottom,
        "run-through" => Wrap::None,
        "left" | "right" | "parallel" | "dynamic" => Wrap::Square,
        "biggest" => Wrap::Square,
        _ => return None,
    })
}

/// `fo:border` is one string: a width, a style and a colour, in any order.
fn border(text: &str) -> Option<Border> {
    let text = text.trim();
    if text.is_empty() || text == "none" || text == "hidden" {
        return None;
    }
    let mut size = None;
    let mut style = BorderStyle::Single;
    let mut shade = None;
    for word in text.split_whitespace() {
        if let Some(width) = length(word) {
            size = Some(Eighth::from_points(width.points()));
        } else if let Some(value) = color(word) {
            shade = Some(value);
        } else {
            style = match word {
                "none" | "hidden" => return None,
                "dotted" => BorderStyle::Dotted,
                "dashed" => BorderStyle::Dashed,
                "double" => BorderStyle::Double,
                "groove" | "ridge" | "inset" | "outset" => BorderStyle::Thick,
                _ => BorderStyle::Single,
            };
        }
    }
    Some(Border {
        style,
        size,
        space: None,
        color: shade,
        shadow: false,
    })
}

/// `<style:tab-stops>`, which is an element rather than an attribute.
pub fn tab_stops(reader: &mut Reader<&[u8]>) -> Vec<TabStop> {
    let mut stops = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local_name(&e) == b"tab-stop" => {
                if let Some(position) = attr_length(&e, b"position") {
                    stops.push(TabStop {
                        position,
                        kind: match attr(&e, b"type").as_deref() {
                            Some("center") => TabKind::Center,
                            Some("right") => TabKind::End,
                            Some("char") => TabKind::Decimal,
                            _ => TabKind::Start,
                        },
                        leader: leader(&e),
                    });
                }
            }
            Ok(Event::End(e)) if end_local_name(&e) == b"tab-stops" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    stops
}

fn leader(e: &BytesStart<'_>) -> TabLeader {
    match attr_in(e, b"style", b"leader-style").as_deref() {
        Some("none") | None => TabLeader::None,
        Some("dotted") => TabLeader::Dot,
        Some("dash") | Some("long-dash") => TabLeader::Hyphen,
        Some("solid") => match attr_in(e, b"style", b"leader-text")
            .as_deref()
            .map(str::trim)
        {
            Some("\u{b7}") => TabLeader::MiddleDot,
            _ => TabLeader::Underscore,
        },
        Some(_) => TabLeader::Dot,
    }
}

/// A font size, which ODF allows to be a length or a percentage of the parent's.
///
/// A percentage is not something the model can hold — it keeps a size, not a
/// relation — and resolving one here would need the parent, which is exactly
/// what this reader has decided not to do. So it is left unsaid and the style
/// inherits the size it would have inherited anyway. The error is confined to a
/// style that scales its parent rather than restating a size, which is what a
/// `%` in a heading style usually is, and it shows up as a heading at body size
/// rather than as anything worse.
fn font_size(text: &str) -> Option<HalfPoint> {
    length(text).map(|size| HalfPoint::from_points(size.points()))
}

fn bold(weight: &str) -> bool {
    match weight {
        "normal" => false,
        "bold" => true,
        // The numeric scale, where anything from six hundred up is bold.
        other => other.parse::<u32>().is_ok_and(|n| n >= 600),
    }
}

fn italic(style: &str) -> bool {
    style == "italic" || style == "oblique"
}

fn underline_kind(style: &str, kind: Option<&str>, width: Option<&str>) -> UnderlineKind {
    let heavy = matches!(width, Some("bold") | Some("thick"));
    let double = kind == Some("double");
    match (style, double, heavy) {
        ("none", _, _) => UnderlineKind::None,
        ("dotted", _, true) => UnderlineKind::DottedHeavy,
        ("dotted", _, false) => UnderlineKind::Dotted,
        ("dash", _, true) => UnderlineKind::DashedHeavy,
        ("dash", _, false) => UnderlineKind::Dash,
        ("long-dash", _, _) => UnderlineKind::DashLong,
        ("dot-dash", _, _) => UnderlineKind::DotDash,
        ("dot-dot-dash", _, _) => UnderlineKind::DotDotDash,
        ("wave", true, _) => UnderlineKind::WavyDouble,
        ("wave", _, true) => UnderlineKind::WavyHeavy,
        ("wave", _, false) => UnderlineKind::Wave,
        (_, true, _) => UnderlineKind::Double,
        (_, _, true) => UnderlineKind::Thick,
        _ => UnderlineKind::Single,
    }
}

/// `style:text-position` is a raise and a size, both as percentages of the
/// font: `super 58%`, `sub 58%`, `-33% 58%`.
fn vertical(text: &str) -> Option<VertAlign> {
    match text.split_whitespace().next().unwrap_or_default() {
        "super" => Some(VertAlign::Superscript),
        "sub" => Some(VertAlign::Subscript),
        first => match percent(first) {
            Some(raise) if raise > 0.0 => Some(VertAlign::Superscript),
            Some(raise) if raise < 0.0 => Some(VertAlign::Subscript),
            Some(_) => Some(VertAlign::Baseline),
            None => None,
        },
    }
}

fn justify(align: &str) -> Justify {
    match align {
        "center" => Justify::Center,
        "end" | "right" => Justify::End,
        "justify" => Justify::Both,
        _ => Justify::Start,
    }
}

/// `fo:line-height` is `normal`, a length, or a percentage of the font's own.
fn line_height(text: &str) -> Option<LineSpacing> {
    if text == "normal" {
        return None;
    }
    if let Some(ratio) = percent(text) {
        return Some(LineSpacing::Multiple(Line240::from_multiple(ratio / 100.0)));
    }
    length(text).map(LineSpacing::Exact)
}

/// `'Times New Roman', serif` names one family, and the quotes are not part of
/// the name.
fn unquoted(family: &str) -> String {
    family
        .split(',')
        .next()
        .unwrap_or(family)
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .trim()
        .to_string()
}

fn tag(language: Option<&str>, country: Option<&str>) -> std::sync::Arc<str> {
    match (language, country) {
        (Some(l), Some(c)) if !c.is_empty() && c != "none" => format!("{l}-{c}").into(),
        (Some(l), _) => l.into(),
        (None, Some(c)) => c.into(),
        (None, None) => "".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_of(xml: &str) -> RunProps {
        let mut reader = Reader::from_str(xml);
        let mut run = RunProps::default();
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                text_properties(&e, &FontFaces::default(), &mut run)
            }
            other => panic!("the test xml is one element: {other:?}"),
        }
        run
    }

    fn para_of(xml: &str) -> ParaProps {
        let mut reader = Reader::from_str(xml);
        let mut para = ParaProps::default();
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => paragraph_properties(&e, &mut para),
            other => panic!("the test xml is one element: {other:?}"),
        }
        para
    }

    /// The difference from the other format, and the reason a translation that
    /// took a property for a flag would read every run as bold.
    #[test]
    fn a_weight_carries_its_value_and_normal_means_not_bold() {
        let bold = run_of(r#"<style:text-properties fo:font-weight="bold"/>"#);
        assert_eq!(bold.toggles.get(Toggle::Bold), Some(true));
        let plain = run_of(r#"<style:text-properties fo:font-weight="normal"/>"#);
        assert_eq!(
            plain.toggles.get(Toggle::Bold),
            Some(false),
            "a style that turns bold off has said so, and must not merely go quiet"
        );
        let quiet = run_of(r#"<style:text-properties fo:font-size="11pt"/>"#);
        assert_eq!(quiet.toggles.get(Toggle::Bold), None);
        assert_eq!(quiet.size, Some(HalfPoint(22)));
        // The numeric scale, which is what a semi-bold face is written as.
        assert_eq!(
            run_of(r#"<style:text-properties fo:font-weight="600"/>"#)
                .toggles
                .get(Toggle::Bold),
            Some(true)
        );
    }

    #[test]
    fn a_negative_first_line_indent_is_a_hanging_one() {
        let hanging = para_of(r#"<style:paragraph-properties fo:text-indent="-0.25in"/>"#);
        assert_eq!(hanging.indent.hanging, Some(Twips(360)));
        assert_eq!(hanging.indent.first_line, None);
        let first = para_of(r#"<style:paragraph-properties fo:text-indent="0.5in"/>"#);
        assert_eq!(first.indent.first_line, Some(Twips(720)));
        assert_eq!(first.indent.hanging, None);
    }

    #[test]
    fn line_height_is_a_ratio_or_a_length_and_normal_is_neither() {
        assert_eq!(
            para_of(r#"<style:paragraph-properties fo:line-height="150%"/>"#)
                .spacing
                .line,
            Some(LineSpacing::Multiple(Line240(360)))
        );
        assert_eq!(
            para_of(r#"<style:paragraph-properties fo:line-height="14pt"/>"#)
                .spacing
                .line,
            Some(LineSpacing::Exact(Twips(280)))
        );
        assert_eq!(
            para_of(r#"<style:paragraph-properties fo:line-height="normal"/>"#)
                .spacing
                .line,
            None
        );
    }

    #[test]
    fn a_family_keeps_its_first_name_without_the_quotes_around_it() {
        let run = run_of(
            r#"<style:text-properties fo:font-family="&apos;Times New Roman&apos;, serif"/>"#,
        );
        assert_eq!(run.fonts.ascii.as_deref(), Some("Times New Roman"));
    }

    #[test]
    fn hidden_text_is_a_display_property_here_rather_than_a_character_one() {
        let run = run_of(r#"<style:text-properties text:display="none"/>"#);
        assert_eq!(run.toggles.get(Toggle::Vanish), Some(true));
    }

    #[test]
    fn a_border_is_one_string_of_width_style_and_colour() {
        let para = para_of(r##"<style:paragraph-properties fo:border="0.75pt solid #1e6f5c"/>"##);
        let borders = para.borders.expect("a border was stated");
        let top = borders.top.expect("on every side");
        assert_eq!(top.style, BorderStyle::Single);
        assert_eq!(top.color, Some(wp_model::Color::Rgb([30, 111, 92])));
        assert_eq!(top.size, Some(Eighth(6)));
        assert_eq!(borders.end.map(|b| b.style), Some(BorderStyle::Single));
        assert!(para_of(r#"<style:paragraph-properties fo:border="none"/>"#)
            .borders
            .is_none());
    }

    #[test]
    fn a_tab_stop_carries_its_own_leader() {
        let xml = concat!(
            r#"<style:tab-stops>"#,
            r#"<style:tab-stop style:position="6.5in" style:type="right" style:leader-style="dotted"/>"#,
            r#"<style:tab-stop style:position="1in"/>"#,
            r#"</style:tab-stops>"#
        );
        let mut reader = Reader::from_str(xml);
        assert!(matches!(reader.read_event(), Ok(Event::Start(_))));
        let stops = tab_stops(&mut reader);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].position, Twips(9360));
        assert_eq!(stops[0].kind, TabKind::End);
        assert_eq!(stops[0].leader, TabLeader::Dot);
        assert_eq!(stops[1].kind, TabKind::Start);
        assert_eq!(stops[1].leader, TabLeader::None);
    }
}
