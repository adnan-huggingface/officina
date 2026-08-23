//! Reading `<w:rPr>`, `<w:pPr>` and `<w:sectPr>`.
//!
//! The three appear in four different places each — in the document, in a style,
//! in a numbering level, and inside a `<w:*PrChange>` recording what the
//! formatting used to be — so they are read here once and called from all of
//! them. Anything that special-cased "properties in a style" would have four
//! copies of the toggle rules to keep in step.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::color::{Color, Highlight, ThemeSlot};
use wp_model::prop::{
    Border, BorderStyle, Fonts, Indent, Justify, LineSpacing, NumRef, ParaBorders, ParaProps,
    RunProps, Script, Shading, ShadingPattern, Spacing, TabKind, TabLeader, TabStop, TextAlign,
    ThemeFont, Toggle, Underline, UnderlineKind, VertAlign,
};
use wp_model::revision::{Mark, PreviousProps, PropChange, Revision};
use wp_model::section::{
    Column, Columns, DocGrid, DocGridKind, HeaderKind, HeaderRef, LineNumberRestart, LineNumbers,
    Orientation, PageBorderDisplay, PageBorders, PageMargins, PageNumbering, PageSize, PageVAlign,
    SectionProps, SectionStart,
};
use wp_model::style::StyleKind;
use wp_model::units::{Eighth, HalfPoint, Line240, Twips};
use wp_model::Lang;

use crate::ctx::Ctx;
use crate::xml::{attr, attr_i32, attr_twips, attr_u32, end_local_name, local_name, on_off, val};

/// What a `<w:rPr>` yielded.
#[derive(Debug, Default)]
pub(crate) struct RunPropsRead {
    pub props: RunProps,
    pub change: Option<Box<PropChange>>,
    /// `<w:ins>` or `<w:del>` inside a *paragraph mark's* `<w:rPr>` — the
    /// paragraph break itself being a tracked change. In a run's `<w:rPr>` this
    /// does not occur.
    pub mark_revision: Option<Revision>,
}

/// What a `<w:pPr>` yielded.
#[derive(Debug, Default)]
pub(crate) struct ParaPropsRead {
    pub props: ParaProps,
    pub section: Option<Box<SectionProps>>,
    pub change: Option<Box<PropChange>>,
    pub mark_revision: Option<Revision>,
}

/// A colour attribute set: `w:val` plus the theme trio beside it.
///
/// The theme reference wins where both are present, because `w:val` is a *cache*
/// of what the theme currently resolves to and goes stale the moment the theme
/// changes.
pub(crate) fn color(e: &BytesStart<'_>) -> Option<Color> {
    color_in(e, b"val")
}

/// A colour held in a named attribute of some other element.
///
/// `<w:color>` says its colour in `w:val`, but a border or a shading says its
/// own thing there — "single", "clear" — and keeps the colour in `w:color`.
/// Reading `w:val` for those parses the *style* as a colour, fails, and turns
/// every white table rule black.
fn color_in(e: &BytesStart<'_>, name: &[u8]) -> Option<Color> {
    if let Some(slot) = attr(e, b"themeColor")
        .as_deref()
        .and_then(ThemeSlot::from_name)
    {
        return Some(Color::Theme {
            slot,
            tint: attr(e, b"themeTint")
                .as_deref()
                .and_then(|t| u8::from_str_radix(t.trim(), 16).ok()),
            shade: attr(e, b"themeShade")
                .as_deref()
                .and_then(|t| u8::from_str_radix(t.trim(), 16).ok()),
        });
    }
    Color::from_val(&attr(e, name)?)
}

/// `<w:rFonts>`.
pub(crate) fn fonts(e: &BytesStart<'_>) -> Fonts {
    Fonts {
        ascii: attr(e, b"ascii").map(Into::into),
        high_ansi: attr(e, b"hAnsi").map(Into::into),
        east_asian: attr(e, b"eastAsia").map(Into::into),
        complex: attr(e, b"cs").map(Into::into),
        ascii_theme: attr(e, b"asciiTheme")
            .as_deref()
            .and_then(ThemeFont::from_val),
        high_ansi_theme: attr(e, b"hAnsiTheme")
            .as_deref()
            .and_then(ThemeFont::from_val),
        east_asian_theme: attr(e, b"eastAsiaTheme")
            .as_deref()
            .and_then(ThemeFont::from_val),
        complex_theme: attr(e, b"cstheme").as_deref().and_then(ThemeFont::from_val),
        hint: attr(e, b"hint").as_deref().and_then(Script::from_val),
    }
}

/// One edge of any border element.
pub(crate) fn border(e: &BytesStart<'_>) -> Border {
    Border {
        style: val(e)
            .as_deref()
            .map(BorderStyle::from_val)
            .unwrap_or_default(),
        size: attr_i32(e, b"sz").map(Eighth),
        space: attr_u32(e, b"space").map(|s| s.min(255) as u8),
        color: color_in(e, b"color"),
        shadow: attr(e, b"shadow")
            .as_deref()
            .map(|v| wp_model::prop::on_off(Some(v)))
            .unwrap_or(false),
    }
}

/// `<w:shd>`.
pub(crate) fn shading(e: &BytesStart<'_>) -> Shading {
    Shading {
        pattern: val(e)
            .as_deref()
            .map(ShadingPattern::from_val)
            .unwrap_or_default(),
        fill: attr(e, b"fill")
            .as_deref()
            .and_then(Color::from_val)
            .or_else(|| {
                attr(e, b"themeFill")
                    .as_deref()
                    .and_then(ThemeSlot::from_name)
                    .map(|slot| Color::Theme {
                        slot,
                        tint: attr(e, b"themeFillTint")
                            .as_deref()
                            .and_then(|t| u8::from_str_radix(t.trim(), 16).ok()),
                        shade: attr(e, b"themeFillShade")
                            .as_deref()
                            .and_then(|t| u8::from_str_radix(t.trim(), 16).ok()),
                    })
            }),
        color: color_in(e, b"color"),
    }
}

/// `<w:u>`.
fn underline(e: &BytesStart<'_>) -> Underline {
    Underline {
        kind: val(e)
            .as_deref()
            .and_then(UnderlineKind::from_val)
            .unwrap_or(UnderlineKind::Single),
        color: color(e),
    }
}

/// `<w:ind>`.
fn indent(e: &BytesStart<'_>) -> Indent {
    Indent {
        // `start`/`end` are the 2010 names and `left`/`right` the 2007 ones.
        // Both occur, sometimes in the same document.
        start: attr_twips(e, b"start").or_else(|| attr_twips(e, b"left")),
        end: attr_twips(e, b"end").or_else(|| attr_twips(e, b"right")),
        first_line: attr_twips(e, b"firstLine"),
        hanging: attr_twips(e, b"hanging"),
    }
}

/// `<w:spacing>` on a paragraph.
fn spacing(e: &BytesStart<'_>) -> Spacing {
    let line = attr_i32(e, b"line").map(|value| match attr(e, b"lineRule").as_deref() {
        Some("exact") => LineSpacing::Exact(Twips(value)),
        Some("atLeast") => LineSpacing::AtLeast(Twips(value)),
        // `auto`, absent, and anything unrecognised. The unit changes with the
        // rule, which is why the three are one enum rather than a number and a
        // flag that can drift apart.
        _ => LineSpacing::Multiple(Line240(value)),
    });
    Spacing {
        before: attr_twips(e, b"before"),
        after: attr_twips(e, b"after"),
        before_auto: attr(e, b"beforeAutospacing")
            .as_deref()
            .map(|v| wp_model::prop::on_off(Some(v))),
        after_auto: attr(e, b"afterAutospacing")
            .as_deref()
            .map(|v| wp_model::prop::on_off(Some(v))),
        line,
    }
}

fn tab_stop(e: &BytesStart<'_>) -> Option<TabStop> {
    Some(TabStop {
        position: attr_twips(e, b"pos")?,
        kind: val(e).as_deref().and_then(TabKind::from_val)?,
        leader: attr(e, b"leader")
            .as_deref()
            .and_then(TabLeader::from_val)
            .unwrap_or_default(),
    })
}

/// A tracked-change mark's attributes: `w:id`, `w:author`, `w:date`.
fn mark(e: &BytesStart<'_>) -> Mark {
    Mark {
        id: attr_u32(e, b"id").unwrap_or(0),
        author: attr(e, b"author").unwrap_or_default().into(),
        date: attr(e, b"date").map(Into::into),
    }
}

/// `<w:ins>`, `<w:del>`, `<w:moveFrom>`, `<w:moveTo>` as a revision.
pub(crate) fn revision(name: &[u8], e: &BytesStart<'_>) -> Option<Revision> {
    let mark = mark(e);
    Some(match name {
        b"ins" => Revision::Inserted(mark),
        b"del" => Revision::Deleted(mark),
        b"moveFrom" => Revision::MovedFrom {
            mark,
            name: attr(e, b"name").unwrap_or_default().into(),
        },
        b"moveTo" => Revision::MovedTo {
            mark,
            name: attr(e, b"name").unwrap_or_default().into(),
        },
        _ => return None,
    })
}

/// Reads a `<w:rPr>` whose start tag has already been consumed.
pub(crate) fn run_props(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> RunPropsRead {
    let mut out = RunPropsRead::default();
    let props = &mut out.props;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let empty = matches!(event, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = event else {
                    unreachable!()
                };
                let name = local_name(&e).to_vec();
                run_prop(&name, &e, props, ctx);
                match name.as_slice() {
                    // Only descend into an element that actually has children.
                    // An empty `<w:rPrChange/>` is legal, and a child reader let
                    // loose on one runs past its parent's end tag and eats the
                    // rest of the paragraph.
                    b"rPrChange" if !empty => {
                        let previous = read_nested_run_props(reader, ctx);
                        out.change = Some(Box::new(PropChange {
                            mark: mark(&e),
                            previous: PreviousProps::Run(Box::new(previous)),
                        }));
                    }
                    b"ins" | b"del" | b"moveFrom" | b"moveTo" => {
                        out.mark_revision = revision(&name, &e);
                    }
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"rPr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// The `<w:rPr>` inside an `<w:rPrChange>`, which is a whole nested property set.
fn read_nested_run_props(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> RunProps {
    let mut props = RunProps::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if local_name(&e) == b"rPr" => {
                props = run_props(reader, ctx).props;
            }
            Ok(Event::End(e)) if end_local_name(&e) == b"rPrChange" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    props
}

/// One child of `<w:rPr>`.
fn run_prop(name: &[u8], e: &BytesStart<'_>, props: &mut RunProps, ctx: &mut Ctx<'_>) {
    if let Some(toggle) = Toggle::from_element(std::str::from_utf8(name).unwrap_or("")) {
        props.toggles.set(toggle, on_off(e));
        return;
    }
    match name {
        b"rStyle" => {
            if let Some(id) = val(e) {
                props.style = Some(ctx.styles.intern(&id, StyleKind::Character));
            }
        }
        b"rFonts" => props.fonts = fonts(e),
        b"sz" => props.size = attr_i32(e, b"val").map(HalfPoint),
        b"szCs" => props.size_complex = attr_i32(e, b"val").map(HalfPoint),
        b"color" => props.color = color(e),
        b"u" => props.underline = Some(underline(e)),
        b"highlight" => props.highlight = val(e).as_deref().and_then(Highlight::from_name),
        b"vertAlign" => props.vert_align = val(e).as_deref().and_then(VertAlign::from_val),
        b"spacing" => props.letter_spacing = attr_twips(e, b"val"),
        b"w" => props.scale = attr_u32(e, b"val").map(|v| v.min(u16::MAX as u32) as u16),
        b"position" => props.raise = attr_i32(e, b"val").map(HalfPoint),
        b"kern" => props.kern = attr_i32(e, b"val").map(HalfPoint),
        b"shd" => props.shading = Some(shading(e)),
        b"bdr" => props.border = Some(border(e)),
        b"rtl" => props.rtl = Some(on_off(e)),
        b"noProof" => props.no_proof = Some(on_off(e)),
        b"lang" => {
            props.lang = Some(Lang {
                value: attr(e, b"val").map(Into::into),
                east_asian: attr(e, b"eastAsia").map(Into::into),
                complex: attr(e, b"bidi").map(Into::into),
            })
        }
        _ => {}
    }
}

/// Reads a `<w:pPr>` whose start tag has already been consumed.
pub(crate) fn para_props(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> ParaPropsRead {
    let mut out = ParaPropsRead::default();
    let mut num_id: Option<u32> = None;
    let mut level: Option<u8> = None;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let empty = matches!(event, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = event else {
                    unreachable!()
                };
                let name = local_name(&e).to_vec();
                // Same rule as in `run_props`: a child reader may only be
                // started for an element that has children to read.
                match name.as_slice() {
                    b"rPr" if !empty => {
                        let read = run_props(reader, ctx);
                        out.props.mark = Some(Box::new(read.props));
                        out.mark_revision = read.mark_revision;
                    }
                    b"sectPr" if !empty => {
                        out.section = Some(Box::new(section_props(reader, e.clone(), ctx)))
                    }
                    b"tabs" if !empty => out.props.tabs = Some(read_tabs(reader)),
                    b"framePr" => {
                        out.props.frame = Some(Box::new(wp_model::prop::FrameProps {
                            drop_cap: attr(&e, b"dropCap")
                                .as_deref()
                                .and_then(wp_model::prop::DropCap::from_val)
                                .unwrap_or_default(),
                            lines: attr_u32(&e, b"lines").unwrap_or(0),
                        }));
                    }
                    b"pBdr" if !empty => {
                        out.props.borders = Some(Box::new(read_para_borders(reader)))
                    }
                    b"numPr" if !empty => {
                        let (n, l) = read_num_pr(reader);
                        num_id = n;
                        level = l;
                    }
                    b"pPrChange" if !empty => {
                        let previous = read_nested_para_props(reader, ctx);
                        out.change = Some(Box::new(PropChange {
                            mark: mark(&e),
                            previous: PreviousProps::Paragraph(Box::new(previous)),
                        }));
                    }
                    other => para_prop(other, &e, &mut out.props, ctx),
                }
            }
            Event::End(e) if end_local_name(&e) == b"pPr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    // A `<w:numPr>` may carry only a level — which is a paragraph moving deeper
    // in the list its *style* named — so the reference is built from whatever
    // arrived rather than requiring both.
    if num_id.is_some() || level.is_some() {
        out.props.numbering = Some(NumRef {
            num_id: num_id.unwrap_or(0),
            level: level.unwrap_or(0),
        });
    }
    out
}

fn read_nested_para_props(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> ParaProps {
    let mut props = ParaProps::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if local_name(&e) == b"pPr" => {
                props = para_props(reader, ctx).props;
            }
            Ok(Event::End(e)) if end_local_name(&e) == b"pPrChange" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    props
}

fn para_prop(name: &[u8], e: &BytesStart<'_>, props: &mut ParaProps, ctx: &mut Ctx<'_>) {
    match name {
        b"pStyle" => {
            if let Some(id) = val(e) {
                props.style = Some(ctx.styles.intern(&id, StyleKind::Paragraph));
            }
        }
        b"jc" => props.justify = val(e).as_deref().and_then(Justify::from_val),
        b"ind" => props.indent = indent(e),
        b"spacing" => props.spacing = spacing(e),
        b"keepNext" => props.keep_next = Some(on_off(e)),
        b"keepLines" => props.keep_lines = Some(on_off(e)),
        b"pageBreakBefore" => props.page_break_before = Some(on_off(e)),
        b"widowControl" => props.widow_control = Some(on_off(e)),
        b"contextualSpacing" => props.contextual_spacing = Some(on_off(e)),
        b"suppressLineNumbers" => props.suppress_line_numbers = Some(on_off(e)),
        b"bidi" => props.bidi = Some(on_off(e)),
        b"outlineLvl" => props.outline_level = attr_u32(e, b"val").map(|v| v.min(8) as u8),
        b"shd" => props.shading = Some(shading(e)),
        b"textAlignment" => props.text_align = val(e).as_deref().and_then(TextAlign::from_val),
        _ => {}
    }
}

fn read_num_pr(reader: &mut Reader<&[u8]>) -> (Option<u32>, Option<u8>) {
    let (mut num_id, mut level) = (None, None);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local_name(&e) {
                b"numId" => num_id = attr_u32(&e, b"val"),
                b"ilvl" => level = attr_u32(&e, b"val").map(|v| v.min(8) as u8),
                _ => {}
            },
            Ok(Event::End(e)) if end_local_name(&e) == b"numPr" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (num_id, level)
}

pub(crate) fn read_tabs(reader: &mut Reader<&[u8]>) -> Vec<TabStop> {
    let mut stops = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local_name(&e) == b"tab" => {
                if let Some(stop) = tab_stop(&e) {
                    stops.push(stop);
                }
            }
            Ok(Event::End(e)) if end_local_name(&e) == b"tabs" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    // A tab list is a set of positions rather than a sequence. Word writes them
    // in order and files that do not are still perfectly meaningful, so they are
    // sorted here — which is also the invariant `ParaProps::layer` maintains, so
    // a paragraph's own list and a resolved one are the same shape.
    stops.sort_by_key(|stop| stop.position);
    stops
}

pub(crate) fn read_para_borders(reader: &mut Reader<&[u8]>) -> ParaBorders {
    let mut borders = ParaBorders::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let edge = border(&e);
                match local_name(&e) {
                    b"top" => borders.top = Some(edge),
                    b"left" | b"start" => borders.start = Some(edge),
                    b"bottom" => borders.bottom = Some(edge),
                    b"right" | b"end" => borders.end = Some(edge),
                    b"between" => borders.between = Some(edge),
                    b"bar" => borders.bar = Some(edge),
                    _ => {}
                }
            }
            Ok(Event::End(e)) if end_local_name(&e) == b"pBdr" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    borders
}

/// Reads a `<w:sectPr>` whose start tag has already been consumed.
pub(crate) fn section_props(
    reader: &mut Reader<&[u8]>,
    start: BytesStart<'_>,
    ctx: &mut Ctx<'_>,
) -> SectionProps {
    let mut section = SectionProps::new();
    // `w:rsidSect` and friends live on the start tag and are not modelled.
    let _ = start;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let empty = matches!(event, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = event else {
                    unreachable!()
                };
                match local_name(&e) {
                    b"type" => {
                        section.start = val(&e)
                            .as_deref()
                            .and_then(SectionStart::from_val)
                            .unwrap_or_default()
                    }
                    b"pgSz" => {
                        section.page = PageSize {
                            width: attr_twips(&e, b"w").unwrap_or(Twips::LETTER_WIDTH),
                            height: attr_twips(&e, b"h").unwrap_or(Twips::LETTER_HEIGHT),
                            // The measurements above are already the printed ones —
                            // Word writes them swapped for a landscape page. This is
                            // read for the page setup dialog and for the writer, and
                            // is deliberately not applied to them.
                            orientation: match attr(&e, b"orient").as_deref() {
                                Some("landscape") => Orientation::Landscape,
                                _ => Orientation::Portrait,
                            },
                            code: attr_u32(&e, b"code"),
                        }
                    }
                    b"pgMar" => {
                        section.margins = PageMargins {
                            top: attr_twips(&e, b"top").unwrap_or(Twips::INCH),
                            bottom: attr_twips(&e, b"bottom").unwrap_or(Twips::INCH),
                            start: attr_twips(&e, b"left")
                                .or_else(|| attr_twips(&e, b"start"))
                                .unwrap_or(Twips::INCH),
                            end: attr_twips(&e, b"right")
                                .or_else(|| attr_twips(&e, b"end"))
                                .unwrap_or(Twips::INCH),
                            header: attr_twips(&e, b"header").unwrap_or(Twips(720)),
                            footer: attr_twips(&e, b"footer").unwrap_or(Twips(720)),
                            gutter: attr_twips(&e, b"gutter").unwrap_or(Twips(0)),
                        }
                    }
                    // `<w:cols w:space="720"/>` is an empty element, and reading
                    // children for it consumed `</w:sectPr>` — which left the
                    // section reader running to the end of the document, taking two
                    // paragraphs and a second section with it. The whole class of
                    // bug is why every descent below is guarded.
                    b"cols" => section.columns = read_columns(reader, &e, empty),
                    b"titlePg" => section.title_page = on_off(&e),
                    b"vAlign" => {
                        section.v_align = val(&e)
                            .as_deref()
                            .and_then(PageVAlign::from_val)
                            .unwrap_or_default()
                    }
                    b"bidi" => section.bidi = on_off(&e),
                    b"rtlGutter" => section.rtl_gutter = on_off(&e),
                    b"gutterAtTop" => section.gutter_at_top = on_off(&e),
                    b"docGrid" => {
                        section.doc_grid = Some(DocGrid {
                            kind: attr(&e, b"type")
                                .as_deref()
                                .and_then(DocGridKind::from_val)
                                .unwrap_or_default(),
                            line_pitch: attr_twips(&e, b"linePitch").unwrap_or(Twips(360)),
                            char_space: attr_twips(&e, b"charSpace").unwrap_or(Twips(0)),
                        })
                    }
                    b"lnNumType" => {
                        section.line_numbers = Some(LineNumbers {
                            count_by: attr_u32(&e, b"countBy").unwrap_or(1),
                            start: attr_u32(&e, b"start").unwrap_or(1),
                            distance: attr_twips(&e, b"distance"),
                            restart: match attr(&e, b"restart").as_deref() {
                                Some("newSection") => LineNumberRestart::NewSection,
                                Some("continuous") => LineNumberRestart::Continuous,
                                _ => LineNumberRestart::NewPage,
                            },
                        })
                    }
                    b"pgNumType" => {
                        section.page_numbering = PageNumbering {
                            start: attr_u32(&e, b"start"),
                            format: attr(&e, b"fmt").map(Into::into),
                            chapter_style: attr_u32(&e, b"chapStyle").map(|v| v.min(8) as u8),
                            chapter_separator: attr(&e, b"chapSep").and_then(|s| s.chars().next()),
                        }
                    }
                    b"pgBorders" if !empty => {
                        section.borders = Some(Box::new(read_page_borders(reader, &e)))
                    }
                    b"headerReference" | b"footerReference" => {
                        let kind = attr(&e, b"type")
                            .as_deref()
                            .and_then(HeaderKind::from_val)
                            .unwrap_or(HeaderKind::Default);
                        if let Some(rel) = attr(&e, b"id") {
                            let footer = local_name(&e) == b"footerReference";
                            let body = ctx.header_id(&rel, footer);
                            let reference = HeaderRef {
                                kind,
                                body,
                                rel: Some(rel.as_str().into()),
                            };
                            if footer {
                                section.footers.push(reference);
                            } else {
                                section.headers.push(reference);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"sectPr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    section
}

fn read_columns(reader: &mut Reader<&[u8]>, start: &BytesStart<'_>, empty: bool) -> Columns {
    let mut columns = Columns {
        num: attr_u32(start, b"num").unwrap_or(1),
        space: attr_twips(start, b"space").unwrap_or(Twips(720)),
        // The default is *true*, so `<w:cols w:num="3"/>` is three equal columns
        // rather than three columns of nothing.
        equal_width: attr(start, b"equalWidth")
            .as_deref()
            .map(|v| wp_model::prop::on_off(Some(v)))
            .unwrap_or(true),
        separator: attr(start, b"sep")
            .as_deref()
            .map(|v| wp_model::prop::on_off(Some(v)))
            .unwrap_or(false),
        columns: Vec::new(),
    };
    if empty {
        return columns;
    }
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local_name(&e) == b"col" => {
                columns.columns.push(Column {
                    width: attr_twips(&e, b"w").unwrap_or(Twips(0)),
                    space: attr_twips(&e, b"space").unwrap_or(Twips(0)),
                });
            }
            Ok(Event::End(e)) if end_local_name(&e) == b"cols" => break,
            Ok(Event::End(e)) if end_local_name(&e) == b"sectPr" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    columns
}

fn read_page_borders(reader: &mut Reader<&[u8]>, start: &BytesStart<'_>) -> PageBorders {
    let mut borders = PageBorders {
        from_text: attr(start, b"offsetFrom").as_deref() == Some("text"),
        display: match attr(start, b"display").as_deref() {
            Some("firstPage") => PageBorderDisplay::FirstPage,
            Some("notFirstPage") => PageBorderDisplay::NotFirstPage,
            _ => PageBorderDisplay::AllPages,
        },
        behind_text: attr(start, b"zOrder").as_deref() == Some("back"),
        ..PageBorders::default()
    };
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let edge = border(&e);
                match local_name(&e) {
                    b"top" => borders.top = Some(edge),
                    b"left" | b"start" => borders.start = Some(edge),
                    b"bottom" => borders.bottom = Some(edge),
                    b"right" | b"end" => borders.end = Some(edge),
                    _ => {}
                }
            }
            Ok(Event::End(e)) if end_local_name(&e) == b"pgBorders" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    borders
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::test_ctx;

    fn run(xml: &str) -> RunProps {
        let mut reader = Reader::from_str(xml);
        let (mut styles, mut headers) = test_ctx();
        let mut ctx = Ctx::new(&mut styles, &mut headers);
        loop {
            match reader.read_event().unwrap() {
                Event::Start(e) if local_name(&e) == b"rPr" => {
                    return run_props(&mut reader, &mut ctx).props
                }
                Event::Eof => panic!("no rPr in {xml}"),
                _ => {}
            }
        }
    }

    fn para(xml: &str) -> ParaPropsRead {
        let mut reader = Reader::from_str(xml);
        let (mut styles, mut headers) = test_ctx();
        let mut ctx = Ctx::new(&mut styles, &mut headers);
        loop {
            match reader.read_event().unwrap() {
                Event::Start(e) if local_name(&e) == b"pPr" => {
                    return para_props(&mut reader, &mut ctx)
                }
                Event::Eof => panic!("no pPr in {xml}"),
                _ => {}
            }
        }
    }

    #[test]
    fn a_bare_toggle_element_is_on_and_val_zero_is_off() {
        let props = run("<w:rPr><w:b/><w:i w:val=\"0\"/><w:caps w:val=\"true\"/></w:rPr>");
        assert_eq!(props.toggles.get(Toggle::Bold), Some(true));
        assert_eq!(props.toggles.get(Toggle::Italic), Some(false));
        assert_eq!(props.toggles.get(Toggle::Caps), Some(true));
        assert_eq!(
            props.toggles.get(Toggle::Strike),
            None,
            "unstated stays unstated"
        );
    }

    #[test]
    fn a_font_size_is_half_points() {
        let props = run("<w:rPr><w:sz w:val=\"22\"/><w:szCs w:val=\"28\"/></w:rPr>");
        assert_eq!(props.size, Some(HalfPoint(22)));
        assert_eq!(props.size.unwrap().points(), 11.0);
        assert_eq!(props.size_complex, Some(HalfPoint(28)));
    }

    #[test]
    fn a_theme_colour_beats_the_cached_value_beside_it() {
        // `w:val` is what the theme resolved to when the file was written, and
        // it goes stale the moment the theme changes.
        let props = run(
            r#"<w:rPr><w:color w:val="2F5496" w:themeColor="accent1" w:themeShade="BF"/></w:rPr>"#,
        );
        assert_eq!(
            props.color,
            Some(Color::Theme {
                slot: ThemeSlot::Accent1,
                tint: None,
                shade: Some(0xBF),
            })
        );

        let plain = run(r#"<w:rPr><w:color w:val="FF0000"/></w:rPr>"#);
        assert_eq!(plain.color, Some(Color::Rgb([0xFF, 0, 0])));
    }

    #[test]
    fn all_four_font_faces_are_read() {
        let props = run(
            r#"<w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="MS Mincho" w:cs="Arial" w:hint="eastAsia"/></w:rPr>"#,
        );
        assert_eq!(props.fonts.ascii.as_deref(), Some("Calibri"));
        assert_eq!(props.fonts.east_asian.as_deref(), Some("MS Mincho"));
        assert_eq!(props.fonts.complex.as_deref(), Some("Arial"));
        assert_eq!(props.fonts.hint, Some(Script::EastAsian));
    }

    #[test]
    fn a_theme_font_reference_is_read_beside_the_cached_name() {
        let props =
            run(r#"<w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:ascii="Calibri"/></w:rPr>"#);
        assert_eq!(props.fonts.ascii_theme, Some(ThemeFont::MinorHighAnsi));
        assert_eq!(props.fonts.ascii.as_deref(), Some("Calibri"));
    }

    #[test]
    fn an_underline_with_no_val_is_a_single_line() {
        let props = run("<w:rPr><w:u/></w:rPr>");
        assert_eq!(props.underline.unwrap().kind, UnderlineKind::Single);
        let none = run(r#"<w:rPr><w:u w:val="none"/></w:rPr>"#);
        assert_eq!(none.underline.unwrap().kind, UnderlineKind::None);
    }

    #[test]
    fn the_line_rule_decides_what_the_line_attribute_measures() {
        let auto = para(r#"<w:pPr><w:spacing w:line="360" w:lineRule="auto"/></w:pPr>"#);
        assert_eq!(
            auto.props.spacing.line,
            Some(LineSpacing::Multiple(Line240(360)))
        );
        let exact = para(r#"<w:pPr><w:spacing w:line="360" w:lineRule="exact"/></w:pPr>"#);
        assert_eq!(
            exact.props.spacing.line,
            Some(LineSpacing::Exact(Twips(360)))
        );
        // No rule at all means auto, and 360 is then one-and-a-half lines rather
        // than eighteen points.
        let bare = para(r#"<w:pPr><w:spacing w:line="360"/></w:pPr>"#);
        assert_eq!(
            bare.props.spacing.line,
            Some(LineSpacing::Multiple(Line240(360)))
        );
    }

    #[test]
    fn both_spellings_of_the_indent_attributes_are_read() {
        let old = para(r#"<w:pPr><w:ind w:left="720" w:right="360" w:hanging="360"/></w:pPr>"#);
        assert_eq!(old.props.indent.start, Some(Twips(720)));
        assert_eq!(old.props.indent.end, Some(Twips(360)));
        assert_eq!(old.props.indent.first_line_offset(), Twips(-360));

        let new = para(r#"<w:pPr><w:ind w:start="720" w:end="360"/></w:pPr>"#);
        assert_eq!(new.props.indent.start, Some(Twips(720)));
        assert_eq!(new.props.indent.end, Some(Twips(360)));
    }

    #[test]
    fn a_num_pr_with_only_a_level_still_makes_a_reference() {
        // A paragraph moving deeper into the list its style named writes only
        // `<w:ilvl>`, and requiring both would drop it out of the list.
        let read = para(r#"<w:pPr><w:numPr><w:ilvl w:val="2"/></w:numPr></w:pPr>"#);
        let reference = read.props.numbering.unwrap();
        assert_eq!(reference.level, 2);
        assert_eq!(reference.num_id, 0);

        let both =
            para(r#"<w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="3"/></w:numPr></w:pPr>"#);
        let reference = both.props.numbering.unwrap();
        assert_eq!((reference.num_id, reference.level), (3, 1));
    }

    #[test]
    fn a_paragraph_mark_carries_its_own_run_properties_and_its_own_revision() {
        let read = para(
            r#"<w:pPr><w:rPr><w:ins w:id="7" w:author="Adnan Khan" w:date="2026-08-14T00:00:00Z"/><w:b/></w:rPr></w:pPr>"#,
        );
        let mark = read.props.mark.expect("the mark has properties");
        assert!(mark.bold());
        match read.mark_revision {
            Some(Revision::Inserted(mark)) => {
                assert_eq!(mark.id, 7);
                assert_eq!(mark.author.as_ref(), "Adnan Khan");
            }
            other => panic!("the paragraph break is a tracked insertion: {other:?}"),
        }
    }

    #[test]
    fn a_property_change_holds_the_previous_formatting() {
        let props = run(
            r#"<w:rPr><w:b/><w:rPrChange w:id="1" w:author="A" w:date="2026-01-01T00:00:00Z"><w:rPr><w:i/></w:rPr></w:rPrChange></w:rPr>"#,
        );
        // The *current* formatting is bold; the change holds the italic it was.
        assert!(props.bold());
        assert!(!props.italic());
    }

    #[test]
    fn tabs_accumulate_and_keep_their_leader() {
        let read = para(
            r#"<w:pPr><w:tabs><w:tab w:val="right" w:leader="dot" w:pos="9026"/><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr>"#,
        );
        let tabs = read.props.tabs.unwrap();
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].position, Twips(720), "sorted by position");
        assert_eq!(tabs[1].kind, TabKind::End);
        assert_eq!(tabs[1].leader, TabLeader::Dot);
    }

    #[test]
    fn a_section_reads_its_page_and_its_margins() {
        let mut reader = Reader::from_str(
            r#"<w:sectPr><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720"/></w:sectPr>"#,
        );
        let (mut styles, mut headers) = test_ctx();
        let mut ctx = Ctx::new(&mut styles, &mut headers);
        let start = match reader.read_event().unwrap() {
            Event::Start(e) => e.into_owned(),
            other => panic!("{other:?}"),
        };
        let section = section_props(&mut reader, start, &mut ctx);
        assert_eq!(section.page.orientation, Orientation::Landscape);
        assert_eq!(section.page.width, Twips(15840));
        assert!(
            section.page.width > section.page.height,
            "already the printed width"
        );
        assert_eq!(section.text_width(), Twips(15840 - 2880));
        assert_eq!(section.columns.count(), 1);
    }

    #[test]
    fn an_empty_cols_element_is_three_equal_columns() {
        let mut reader =
            Reader::from_str(r#"<w:sectPr><w:cols w:num="3" w:space="432"/></w:sectPr>"#);
        let (mut styles, mut headers) = test_ctx();
        let mut ctx = Ctx::new(&mut styles, &mut headers);
        let start = match reader.read_event().unwrap() {
            Event::Start(e) => e.into_owned(),
            other => panic!("{other:?}"),
        };
        let section = section_props(&mut reader, start, &mut ctx);
        assert_eq!(section.columns.count(), 3);
        assert!(section.columns.equal_width);
        assert_eq!(section.columns.space, Twips(432));
    }

    #[test]
    fn solid_shading_keeps_both_colours_so_the_right_one_can_be_painted() {
        let props = run(r#"<w:rPr><w:shd w:val="clear" w:color="auto" w:fill="D9E2F3"/></w:rPr>"#);
        let shading = props.shading.unwrap();
        assert_eq!(shading.pattern, ShadingPattern::Clear);
        assert_eq!(shading.background(), Some(Color::Rgb([0xD9, 0xE2, 0xF3])));
    }

    #[test]
    fn a_borders_colour_is_in_w_color_because_w_val_is_its_style() {
        // resume.docx rules its footer with white borders — invisible on paper.
        // Reading `w:val` here parses "single" as a colour, fails, and the
        // painter's black fallback draws a box Word never shows.
        let mut reader =
            Reader::from_str(r#"<w:top w:color="ffffff" w:space="0" w:sz="4" w:val="single"/>"#);
        let e = match reader.read_event().unwrap() {
            Event::Empty(e) => e.into_owned(),
            other => panic!("{other:?}"),
        };
        let edge = border(&e);
        assert_eq!(edge.style, BorderStyle::Single);
        assert_eq!(edge.color, Some(Color::Rgb([0xff, 0xff, 0xff])));
    }

    #[test]
    fn solid_shading_finds_its_fill_in_w_color() {
        // `solid` inverts the attributes: `w:color` is what gets painted.
        let props = run(r#"<w:rPr><w:shd w:val="solid" w:color="1F4E79" w:fill="auto"/></w:rPr>"#);
        let shading = props.shading.unwrap();
        assert_eq!(shading.background(), Some(Color::Rgb([0x1F, 0x4E, 0x79])));
    }
}
