//! Reading a body of blocks: `<w:body>`, a table cell, a header, a footnote, a
//! comment, or the contents of a content control.
//!
//! All six hold the same thing — paragraphs and tables — which is why one
//! function reads all six and takes the element it should stop at.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::doc::{
    Alignment, Block, Break, DataBinding, Drawing, DrawingPosition, Hyperlink, Inline, MathBlob,
    Offset, Paragraph, Piece, RelativeTo, Run, Sdt, SdtKind, Wrap,
};
use wp_model::prop::Justify;
use wp_model::revision::Anchor;
use wp_model::section::SectionProps;
use wp_model::style::StyleKind;
use wp_model::table::{
    Cell, CellMargins, CellProps, CellVAlign, FloatAnchor, Row, RowHeight, RowProps, Table,
    TableBorders, TableFloat, TableLayout, TableLook, TableProps, TextDirection, VMerge, Width,
};
use wp_model::units::{Emu, Twips};

use crate::ctx::Ctx;
use crate::props;
use crate::xml::{
    attr, attr_i32, attr_twips, attr_u32, end_local_name, local_name, on_off, prefix, push_text,
    skip_element, val,
};

/// Reads blocks until `until` closes, and the `<w:sectPr>` that may sit among
/// them.
///
/// The section properties are returned separately because in `<w:body>` they are
/// the *document's* final section rather than a block, and everywhere else they
/// do not occur at all.
pub(crate) fn read_blocks(
    reader: &mut Reader<&[u8]>,
    ctx: &mut Ctx<'_>,
    until: &[u8],
) -> (Vec<Block>, Option<SectionProps>) {
    let mut blocks = Vec::new();
    let mut section = None;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"p" => blocks.push(Block::Paragraph(read_paragraph(reader, ctx, &e))),
                    b"tbl" => blocks.push(Block::Table(read_table(reader, ctx))),
                    b"sdt" => blocks.push(read_block_sdt(reader, ctx)),
                    b"sectPr" => section = Some(props::section_props(reader, e.into_owned(), ctx)),
                    // A `<w:customXml>` or `<w:ins>` around whole blocks is a
                    // transparent wrapper at this level: its children are blocks
                    // and belong in the same list.
                    b"customXml" | b"ins" | b"del" | b"moveFrom" | b"moveTo" => {
                        let (inner, _) = read_blocks(reader, ctx, &name);
                        blocks.extend(inner);
                    }
                    other => {
                        let owned = other.to_vec();
                        let _ = skip_element(reader, &owned);
                    }
                }
            }
            Event::Empty(e) => {
                // `<w:p/>` is a real paragraph — a blank line — and Word writes
                // one as the entire body of a style-only template. Handling only
                // the start-tag spelling drops it, and the document opens empty.
                if local_name(&e) == b"p" {
                    blocks.push(Block::Paragraph(Paragraph {
                        id: hex_id(&e, b"paraId"),
                        text_id: hex_id(&e, b"textId"),
                        ..Paragraph::new()
                    }));
                } else if let Some(anchor) = read_anchor(&e) {
                    blocks.push(Block::Anchor(anchor));
                } else if local_name(&e) == b"altChunk" {
                    if let Some(rel) = attr(&e, b"id") {
                        blocks.push(Block::AltChunk { rel: rel.into() });
                    }
                }
            }
            Event::End(e) if end_local_name(&e) == until => break,
            Event::Eof => break,
            _ => {}
        }
    }
    (blocks, section)
}

/// `w14:paraId="760D8500"` — eight hex digits, not a decimal number.
fn hex_id(e: &BytesStart<'_>, want: &[u8]) -> Option<u32> {
    u32::from_str_radix(attr(e, want)?.trim(), 16).ok()
}

fn read_paragraph(
    reader: &mut Reader<&[u8]>,
    ctx: &mut Ctx<'_>,
    start: &BytesStart<'_>,
) -> Paragraph {
    let mut paragraph = Paragraph {
        id: hex_id(start, b"paraId"),
        text_id: hex_id(start, b"textId"),
        ..Paragraph::new()
    };
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => {
                let name = local_name(&e).to_vec();
                if name == b"pPr" {
                    let read = props::para_props(reader, ctx);
                    paragraph.props = read.props;
                    paragraph.section = read.section;
                    paragraph.prop_change = read.change;
                    paragraph.mark_revision = read.mark_revision;
                } else if let Some(inline) = read_inline(reader, ctx, &e, &name) {
                    paragraph.content.push(inline);
                } else {
                    let _ = skip_element(reader, &name);
                }
            }
            Event::Empty(e) => {
                // `<w:r/>` holds nothing and is still a run. So is `<w:p/>`
                // above; the empty spelling has to be handled everywhere the
                // start-tag one is.
                if local_name(&e) == b"r" {
                    paragraph.content.push(Inline::Run(Run::new()));
                } else if let Some(anchor) = read_anchor(&e) {
                    paragraph.content.push(Inline::Anchor(anchor));
                }
            }
            Event::End(e) if end_local_name(&e) == b"p" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    paragraph
}

/// One child of `<w:p>` that is a start tag, or `None` for one we do not model.
fn read_inline(
    reader: &mut Reader<&[u8]>,
    ctx: &mut Ctx<'_>,
    e: &BytesStart<'_>,
    name: &[u8],
) -> Option<Inline> {
    match name {
        b"r" => Some(Inline::Run(read_run(reader, ctx))),
        b"hyperlink" => Some(Inline::Hyperlink(Box::new(Hyperlink {
            rel: attr(e, b"id").map(Into::into),
            anchor: attr(e, b"anchor").map(Into::into),
            tooltip: attr(e, b"tooltip").map(Into::into),
            history: attr(e, b"history")
                .as_deref()
                .map(|v| wp_model::prop::on_off(Some(v)))
                .unwrap_or(true),
            content: read_inlines(reader, ctx, name),
        }))),
        b"ins" | b"del" | b"moveFrom" | b"moveTo" => {
            let revision = props::revision(name, e)?;
            Some(Inline::Revised {
                revision,
                content: read_inlines(reader, ctx, name),
            })
        }
        b"sdt" => Some(read_inline_sdt(reader, ctx)),
        b"smartTag" | b"customXml" => Some(Inline::Wrapper {
            name: attr(e, b"element")
                .or_else(|| attr(e, b"uri"))
                .unwrap_or_else(|| String::from_utf8_lossy(name).into_owned())
                .into(),
            content: read_inlines(reader, ctx, name),
        }),
        b"fldSimple" => Some(Inline::SimpleField {
            instruction: attr(e, b"instr").unwrap_or_default().into(),
            content: read_inlines(reader, ctx, name),
        }),
        // `<m:oMath>` and `<m:oMathPara>`. Held whole: the layout cannot draw
        // mathematics and a reader that dropped it would destroy an equation the
        // moment the document was saved.
        b"oMath" | b"oMathPara" if prefix(e.name().into_inner()) == b"m" => {
            Some(Inline::Math(Box::new(read_math(reader, name))))
        }
        b"bookmarkStart" | b"bookmarkEnd" | b"commentRangeStart" | b"commentRangeEnd"
        | b"permStart" | b"permEnd" => {
            let anchor = read_anchor(e)?;
            let owned = name.to_vec();
            let _ = skip_element(reader, &owned);
            Some(Inline::Anchor(anchor))
        }
        _ => None,
    }
}

fn read_inlines(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>, until: &[u8]) -> Vec<Inline> {
    let mut content = Vec::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => {
                let name = local_name(&e).to_vec();
                match read_inline(reader, ctx, &e, &name) {
                    Some(inline) => content.push(inline),
                    None => {
                        let _ = skip_element(reader, &name);
                    }
                }
            }
            Event::Empty(e) => {
                if local_name(&e) == b"r" {
                    content.push(Inline::Run(Run::new()));
                } else if let Some(anchor) = read_anchor(&e) {
                    content.push(Inline::Anchor(anchor));
                }
            }
            Event::End(e) if end_local_name(&e) == until => break,
            Event::Eof => break,
            _ => {}
        }
    }
    content
}

fn read_anchor(e: &BytesStart<'_>) -> Option<Anchor> {
    Some(match local_name(e) {
        b"bookmarkStart" => Anchor::BookmarkStart {
            id: attr_u32(e, b"id")?,
            name: attr(e, b"name").unwrap_or_default().into(),
        },
        b"bookmarkEnd" => Anchor::BookmarkEnd {
            id: attr_u32(e, b"id")?,
        },
        b"commentRangeStart" => Anchor::CommentStart {
            id: attr_u32(e, b"id")?,
        },
        b"commentRangeEnd" => Anchor::CommentEnd {
            id: attr_u32(e, b"id")?,
        },
        b"permStart" => Anchor::PermissionStart {
            id: attr(e, b"id")?.into(),
            editor: attr(e, b"ed").map(Into::into),
        },
        b"permEnd" => Anchor::PermissionEnd {
            id: attr(e, b"id")?.into(),
        },
        _ => return None,
    })
}

fn read_run(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> Run {
    let mut run = Run::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"rPr" => {
                        let read = props::run_props(reader, ctx);
                        run.props = read.props;
                        run.prop_change = read.change;
                    }
                    b"t" => run
                        .content
                        .push(Piece::Text(read_text(reader, b"t").into())),
                    b"delText" => run
                        .content
                        .push(Piece::Deleted(read_text(reader, b"delText").into())),
                    b"instrText" => run
                        .content
                        .push(Piece::Instruction(read_text(reader, b"instrText").into())),
                    b"delInstrText" => run.content.push(Piece::DeletedInstruction(
                        read_text(reader, b"delInstrText").into(),
                    )),
                    b"drawing" => {
                        if let Some(drawing) = read_drawing(reader) {
                            run.content.push(Piece::Drawing(Box::new(drawing)));
                        }
                    }
                    b"pict" | b"object" => {
                        let rel = find_embed(reader, &name).map(Into::into);
                        run.content.push(Piece::Embedded { rel });
                    }
                    other => {
                        let owned = other.to_vec();
                        let _ = skip_element(reader, &owned);
                    }
                }
            }
            Event::Empty(e) => {
                if let Some(piece) = read_piece(&e) {
                    run.content.push(piece);
                }
            }
            Event::End(e) if end_local_name(&e) == b"r" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    run
}

/// The empty elements inside a run.
fn read_piece(e: &BytesStart<'_>) -> Option<Piece> {
    Some(match local_name(e) {
        b"tab" => Piece::Tab,
        b"br" => Piece::Break(Break::from_val(
            attr(e, b"type").as_deref(),
            attr(e, b"clear").as_deref(),
        )),
        b"cr" => Piece::Break(Break::Line),
        b"noBreakHyphen" => Piece::Hyphen { breaking: false },
        b"softHyphen" => Piece::Hyphen { breaking: true },
        b"sym" => Piece::Symbol {
            font: attr(e, b"font").unwrap_or_default().into(),
            // `w:char="F0B7"` is hex, and in the private use area — the
            // character means nothing without the font beside it.
            ch: attr(e, b"char")
                .and_then(|hex| u32::from_str_radix(hex.trim(), 16).ok())
                .and_then(char::from_u32)?,
        },
        b"footnoteReference" => Piece::FootnoteRef {
            id: attr_i32(e, b"id")?,
            custom_mark: attr(e, b"customMarkFollows")
                .as_deref()
                .map(|v| wp_model::prop::on_off(Some(v)))
                .unwrap_or(false),
        },
        b"endnoteReference" => Piece::EndnoteRef {
            id: attr_i32(e, b"id")?,
            custom_mark: attr(e, b"customMarkFollows")
                .as_deref()
                .map(|v| wp_model::prop::on_off(Some(v)))
                .unwrap_or(false),
        },
        b"commentReference" => Piece::CommentRef(attr_u32(e, b"id")?),
        b"fldChar" => match attr(e, b"fldCharType").as_deref() {
            Some("begin") => Piece::FieldStart {
                dirty: attr(e, b"dirty")
                    .as_deref()
                    .map(|v| wp_model::prop::on_off(Some(v)))
                    .unwrap_or(false),
                lock: attr(e, b"fldLock")
                    .as_deref()
                    .map(|v| wp_model::prop::on_off(Some(v)))
                    .unwrap_or(false),
            },
            Some("separate") => Piece::FieldSeparate,
            Some("end") => Piece::FieldEnd,
            _ => return None,
        },
        b"lastRenderedPageBreak" => Piece::LastRenderedPageBreak,
        b"ptab" => Piece::PositionTab,
        // `<w:t/>` written as an empty element is an empty string, which is
        // real: it is what a run holding only formatting looks like.
        b"t" => Piece::Text("".into()),
        _ => return None,
    })
}

/// The text of an element, entities and all.
fn read_text(reader: &mut Reader<&[u8]>, until: &[u8]) -> String {
    let mut out = String::new();
    while let Ok(event) = reader.read_event() {
        match &event {
            Event::End(e) if end_local_name(e) == until => break,
            Event::Eof => break,
            other => {
                push_text(&mut out, other);
            }
        }
    }
    out
}

/// An equation, kept whole with its text pulled out beside it.
fn read_math(reader: &mut Reader<&[u8]>, until: &[u8]) -> MathBlob {
    let mut text = String::new();
    let mut depth = 1usize;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => {
                if local_name(&e) == until {
                    depth += 1;
                } else if local_name(&e) == b"t" {
                    text.push_str(&read_text(reader, b"t"));
                }
            }
            Event::End(e) if end_local_name(&e) == until => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    MathBlob {
        // The bytes are filled in by the writer's side of the vault; the reader
        // records what the equation says so search and word count are not blind.
        source: Vec::new().into(),
        text: text.into(),
    }
}

/// `<w:drawing>`: an inline or anchored picture, chart, shape or diagram.
fn read_drawing(reader: &mut Reader<&[u8]>) -> Option<Drawing> {
    let mut drawing = Drawing {
        anchored: false,
        extent: (Emu(0), Emu(0)),
        rel: None,
        name: None,
        description: None,
        wrap: Wrap::None,
        distance: (Emu(0), Emu(0), Emu(0), Emu(0)),
        position: None,
        behind_text: false,
    };
    let mut horizontal: Option<Offset> = None;
    let mut vertical: Option<Offset> = None;
    let mut axis_is_horizontal = true;
    let mut saw_anchor_or_inline = false;

    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let empty = matches!(event, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = event else {
                    unreachable!()
                };
                match local_name(&e) {
                    b"anchor" => {
                        saw_anchor_or_inline = true;
                        drawing.anchored = true;
                        drawing.behind_text = attr(&e, b"behindDoc")
                            .as_deref()
                            .map(|v| wp_model::prop::on_off(Some(v)))
                            .unwrap_or(false);
                        drawing.distance = (
                            Emu(attr_i32(&e, b"distT").unwrap_or(0) as i64),
                            Emu(attr_i32(&e, b"distR").unwrap_or(0) as i64),
                            Emu(attr_i32(&e, b"distB").unwrap_or(0) as i64),
                            Emu(attr_i32(&e, b"distL").unwrap_or(0) as i64),
                        );
                    }
                    b"inline" => saw_anchor_or_inline = true,
                    b"extent" => {
                        drawing.extent = (
                            Emu(attr(&e, b"cx")
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(0)),
                            Emu(attr(&e, b"cy")
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(0)),
                        )
                    }
                    b"docPr" => {
                        drawing.name = attr(&e, b"name").map(Into::into);
                        drawing.description = attr(&e, b"descr").map(Into::into);
                    }
                    b"blip" => {
                        drawing.rel = attr(&e, b"embed")
                            .or_else(|| attr(&e, b"link"))
                            .map(Into::into)
                    }
                    b"wrapNone" => drawing.wrap = Wrap::None,
                    b"wrapSquare" => drawing.wrap = Wrap::Square,
                    b"wrapTight" | b"wrapThrough" => drawing.wrap = Wrap::Tight,
                    b"wrapTopAndBottom" => drawing.wrap = Wrap::TopAndBottom,
                    b"positionH" => {
                        axis_is_horizontal = true;
                        horizontal = Some(Offset {
                            relative_to: relative_to(attr(&e, b"relativeFrom").as_deref()),
                            offset: None,
                            align: None,
                        });
                    }
                    b"positionV" => {
                        axis_is_horizontal = false;
                        vertical = Some(Offset {
                            relative_to: relative_to(attr(&e, b"relativeFrom").as_deref()),
                            offset: None,
                            align: None,
                        });
                    }
                    b"posOffset" if !empty => {
                        let value = read_text(reader, b"posOffset");
                        let emu = Emu(value.trim().parse().unwrap_or(0));
                        let slot = if axis_is_horizontal {
                            &mut horizontal
                        } else {
                            &mut vertical
                        };
                        if let Some(offset) = slot {
                            offset.offset = Some(emu);
                        }
                    }
                    b"align" if !empty => {
                        let value = read_text(reader, b"align");
                        let slot = if axis_is_horizontal {
                            &mut horizontal
                        } else {
                            &mut vertical
                        };
                        if let Some(offset) = slot {
                            offset.align = alignment(value.trim());
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"drawing" => break,
            Event::Eof => break,
            _ => {}
        }
    }

    if let (Some(horizontal), Some(vertical)) = (horizontal, vertical) {
        drawing.position = Some(Box::new(DrawingPosition {
            horizontal,
            vertical,
        }));
    }
    saw_anchor_or_inline.then_some(drawing)
}

fn relative_to(text: Option<&str>) -> RelativeTo {
    match text {
        Some("character") => RelativeTo::Character,
        Some("margin") => RelativeTo::Margin,
        Some("page") => RelativeTo::Page,
        Some("insideMargin") => RelativeTo::InsideMargin,
        Some("outsideMargin") => RelativeTo::OutsideMargin,
        Some("paragraph") => RelativeTo::Paragraph,
        Some("line") => RelativeTo::Line,
        Some("topMargin") => RelativeTo::TopMargin,
        Some("bottomMargin") => RelativeTo::BottomMargin,
        Some("leftMargin") => RelativeTo::LeftMargin,
        Some("rightMargin") => RelativeTo::RightMargin,
        _ => RelativeTo::Column,
    }
}

fn alignment(text: &str) -> Option<Alignment> {
    Some(match text {
        "left" => Alignment::Left,
        "center" => Alignment::Center,
        "right" => Alignment::Right,
        "top" => Alignment::Top,
        "bottom" => Alignment::Bottom,
        "inside" => Alignment::Inside,
        "outside" => Alignment::Outside,
        _ => return None,
    })
}

/// The first `r:id` inside a VML picture or an OLE object.
fn find_embed(reader: &mut Reader<&[u8]>, until: &[u8]) -> Option<String> {
    let mut rel = None;
    let mut depth = 1usize;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => {
                if rel.is_none() {
                    rel = attr(&e, b"id").filter(|id| id.starts_with("rId"));
                }
                if local_name(&e) == until {
                    depth += 1;
                }
            }
            Event::End(e) if end_local_name(&e) == until => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    rel
}

fn read_block_sdt(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> Block {
    let (sdt, content) = read_sdt(reader, ctx, true);
    let blocks = match content {
        SdtContent::Blocks(blocks) => blocks,
        SdtContent::Inlines(_) => Vec::new(),
    };
    Block::Structured(Box::new(Sdt {
        id: sdt.id,
        alias: sdt.alias,
        tag: sdt.tag,
        kind: sdt.kind,
        binding: sdt.binding,
        lock: sdt.lock,
        placeholder: sdt.placeholder,
        content: blocks,
    }))
}

fn read_inline_sdt(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> Inline {
    let (sdt, content) = read_sdt(reader, ctx, false);
    let inlines = match content {
        SdtContent::Inlines(inlines) => inlines,
        SdtContent::Blocks(_) => Vec::new(),
    };
    Inline::Structured(Box::new(Sdt {
        id: sdt.id,
        alias: sdt.alias,
        tag: sdt.tag,
        kind: sdt.kind,
        binding: sdt.binding,
        lock: sdt.lock,
        placeholder: sdt.placeholder,
        content: inlines,
    }))
}

enum SdtContent {
    Blocks(Vec<Block>),
    Inlines(Vec<Inline>),
}

/// The properties and the content of a `<w:sdt>`.
fn read_sdt(
    reader: &mut Reader<&[u8]>,
    ctx: &mut Ctx<'_>,
    block_level: bool,
) -> (Sdt<()>, SdtContent) {
    let mut sdt = Sdt {
        id: None,
        alias: None,
        tag: None,
        kind: SdtKind::RichText,
        binding: None,
        lock: None,
        placeholder: false,
        content: (),
    };
    let mut content = if block_level {
        SdtContent::Blocks(Vec::new())
    } else {
        SdtContent::Inlines(Vec::new())
    };
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let empty = matches!(event, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = event else {
                    unreachable!()
                };
                match local_name(&e) {
                    b"sdtContent" if !empty => {
                        content = if block_level {
                            SdtContent::Blocks(read_blocks(reader, ctx, b"sdtContent").0)
                        } else {
                            SdtContent::Inlines(read_inlines(reader, ctx, b"sdtContent"))
                        };
                    }
                    b"id" => sdt.id = attr_i32(&e, b"val"),
                    b"alias" => sdt.alias = val(&e).map(Into::into),
                    b"tag" => sdt.tag = val(&e).map(Into::into),
                    b"lock" => sdt.lock = val(&e).map(Into::into),
                    b"showingPlcHdr" => sdt.placeholder = on_off(&e),
                    b"dataBinding" => {
                        sdt.binding = Some(Box::new(DataBinding {
                            xpath: attr(&e, b"xpath").unwrap_or_default().into(),
                            prefix_mappings: attr(&e, b"prefixMappings").map(Into::into),
                            store_item_id: attr(&e, b"storeItemID").map(Into::into),
                        }))
                    }
                    other => {
                        if let Some(kind) = sdt_kind(other) {
                            sdt.kind = kind;
                        }
                    }
                }
            }
            Event::End(e) if end_local_name(&e) == b"sdt" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    (sdt, content)
}

fn sdt_kind(name: &[u8]) -> Option<SdtKind> {
    Some(match name {
        b"richText" => SdtKind::RichText,
        // `<w:text/>` is the *plain* text control. The rich one is `richText`,
        // and the two are one letter and a whole behaviour apart.
        b"text" => SdtKind::PlainText,
        b"picture" => SdtKind::Picture,
        b"comboBox" => SdtKind::ComboBox,
        b"dropDownList" => SdtKind::DropDownList,
        b"date" => SdtKind::Date,
        b"checkbox" => SdtKind::Checkbox,
        b"docPartObj" | b"docPartList" => SdtKind::DocPartObject,
        b"group" => SdtKind::Group,
        b"citation" => SdtKind::Citation,
        b"bibliography" => SdtKind::Bibliography,
        b"repeatingSection" => SdtKind::RepeatingSection,
        _ => return None,
    })
}

// ---------------------------------------------------------------- tables

fn read_table(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> Table {
    let mut table = Table::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => match local_name(&e) {
                b"tblPr" => table.props = read_table_props(reader, ctx),
                b"tblGrid" => table.grid = read_grid(reader),
                b"tr" => table.rows.push(read_row(reader, ctx)),
                other => {
                    let owned = other.to_vec();
                    let _ = skip_element(reader, &owned);
                }
            },
            Event::End(e) if end_local_name(&e) == b"tbl" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    table
}

fn width(e: &BytesStart<'_>) -> Width {
    Width::from_parts(
        attr(e, b"type").as_deref().unwrap_or("auto"),
        attr_i32(e, b"w").unwrap_or(0),
    )
}

fn read_table_props(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> TableProps {
    let mut props = TableProps::default();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let empty = matches!(event, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = event else {
                    unreachable!()
                };
                match local_name(&e) {
                    b"tblStyle" => {
                        if let Some(id) = val(&e) {
                            props.style = Some(ctx.styles.intern(&id, StyleKind::Table));
                        }
                    }
                    b"tblW" => props.width = width(&e),
                    b"jc" => props.justify = val(&e).as_deref().and_then(Justify::from_val),
                    b"tblInd" => props.indent = Some(width(&e)),
                    // Only descend where there are children. An empty
                    // `<w:tblBorders/>` sent a child reader past its parent's end
                    // tag and into the rows.
                    b"tblBorders" if !empty => {
                        props.borders = read_table_borders(reader, b"tblBorders").0
                    }
                    b"shd" => props.shading = Some(props::shading(&e)),
                    b"tblLayout" => {
                        props.layout = match attr(&e, b"type").as_deref() {
                            Some("fixed") => TableLayout::Fixed,
                            _ => TableLayout::Auto,
                        }
                    }
                    b"tblLook" => props.look = read_look(&e),
                    b"tblCellMar" if !empty => {
                        props.cell_margins = read_cell_margins(reader, b"tblCellMar")
                    }
                    b"tblCellSpacing" => props.cell_spacing = Some(width(&e)),
                    b"tblCaption" => props.caption = val(&e).map(Into::into),
                    b"tblDescription" => props.description = val(&e).map(Into::into),
                    b"bidiVisual" => props.bidi_visual = on_off(&e),
                    b"tblpPr" => {
                        props.float = Some(Box::new(TableFloat {
                            x: attr_twips(&e, b"tblpX"),
                            y: attr_twips(&e, b"tblpY"),
                            horizontal_anchor: float_anchor(attr(&e, b"horzAnchor").as_deref()),
                            vertical_anchor: float_anchor(attr(&e, b"vertAnchor").as_deref()),
                            left_from_text: attr_twips(&e, b"leftFromText"),
                            right_from_text: attr_twips(&e, b"rightFromText"),
                            top_from_text: attr_twips(&e, b"topFromText"),
                            bottom_from_text: attr_twips(&e, b"bottomFromText"),
                        }))
                    }
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"tblPr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    props
}

fn float_anchor(text: Option<&str>) -> FloatAnchor {
    match text {
        Some("margin") => FloatAnchor::Margin,
        Some("page") => FloatAnchor::Page,
        _ => FloatAnchor::Text,
    }
}

/// `<w:tblLook>`, from the six attributes if they are there and from the legacy
/// mask if they are not.
fn read_look(e: &BytesStart<'_>) -> TableLook {
    let explicit = |name: &[u8]| {
        attr(e, name)
            .as_deref()
            .map(|v| wp_model::prop::on_off(Some(v)))
    };
    let mask = val(e)
        .and_then(|hex| u32::from_str_radix(hex.trim(), 16).ok())
        .map(TableLook::from_mask)
        .unwrap_or_default();
    TableLook {
        first_row: explicit(b"firstRow").unwrap_or(mask.first_row),
        last_row: explicit(b"lastRow").unwrap_or(mask.last_row),
        first_column: explicit(b"firstColumn").unwrap_or(mask.first_column),
        last_column: explicit(b"lastColumn").unwrap_or(mask.last_column),
        no_h_band: explicit(b"noHBand").unwrap_or(mask.no_h_band),
        no_v_band: explicit(b"noVBand").unwrap_or(mask.no_v_band),
    }
}

/// Reads a borders element. The second half of the pair is the diagonals, which
/// only a cell has.
fn read_table_borders(
    reader: &mut Reader<&[u8]>,
    until: &[u8],
) -> (
    TableBorders,
    (Option<wp_model::Border>, Option<wp_model::Border>),
) {
    let mut borders = TableBorders::default();
    let mut diagonals = (None, None);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let edge = props::border(&e);
                match local_name(&e) {
                    b"top" => borders.top = Some(edge),
                    b"left" | b"start" => borders.start = Some(edge),
                    b"bottom" => borders.bottom = Some(edge),
                    b"right" | b"end" => borders.end = Some(edge),
                    b"insideH" => borders.inside_h = Some(edge),
                    b"insideV" => borders.inside_v = Some(edge),
                    b"tl2br" => diagonals.0 = Some(edge),
                    b"tr2bl" => diagonals.1 = Some(edge),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == until => break,
            Event::Eof => break,
            _ => {}
        }
    }
    (borders, diagonals)
}

fn read_cell_margins(reader: &mut Reader<&[u8]>, until: &[u8]) -> CellMargins {
    let mut margins = CellMargins::default();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let value = width(&e);
                match local_name(&e) {
                    b"top" => margins.top = Some(value),
                    b"left" | b"start" => margins.start = Some(value),
                    b"bottom" => margins.bottom = Some(value),
                    b"right" | b"end" => margins.end = Some(value),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == until => break,
            Event::Eof => break,
            _ => {}
        }
    }
    margins
}

fn read_grid(reader: &mut Reader<&[u8]>) -> Vec<Twips> {
    let mut grid = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local_name(&e) == b"gridCol" => {
                grid.push(attr_twips(&e, b"w").unwrap_or(Twips(0)));
            }
            Ok(Event::End(e)) if end_local_name(&e) == b"tblGrid" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    grid
}

fn read_row(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> Row {
    let mut row = Row::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => match local_name(&e) {
                b"trPr" => row.props = read_row_props(reader),
                b"tc" => row.cells.push(read_cell(reader, ctx)),
                // `<w:tblPrEx>` is a per-row override of the table's properties,
                // written by Word into every row of a table that has ever been
                // split. Not modelled; the element survives through the writer.
                other => {
                    let owned = other.to_vec();
                    let _ = skip_element(reader, &owned);
                }
            },
            Event::End(e) if end_local_name(&e) == b"tr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    row
}

fn read_row_props(reader: &mut Reader<&[u8]>) -> RowProps {
    let mut props = RowProps::default();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"trHeight" => {
                    let value = attr_twips(&e, b"val").unwrap_or(Twips(0));
                    props.height = Some(match attr(&e, b"hRule").as_deref() {
                        Some("exact") => RowHeight::Exact(value),
                        Some("auto") => RowHeight::Auto,
                        // `atLeast` is the default when the rule is absent, and
                        // a height with no rule is *not* an exact height.
                        _ => RowHeight::AtLeast(value),
                    })
                }
                b"cantSplit" => props.cant_split = on_off(&e),
                b"tblHeader" => props.header = on_off(&e),
                b"tblCellSpacing" => props.cell_spacing = Some(width(&e)),
                b"jc" => props.justify = val(&e).as_deref().and_then(Justify::from_val),
                b"gridBefore" => props.grid_before = attr_u32(&e, b"val").unwrap_or(0),
                b"gridAfter" => props.grid_after = attr_u32(&e, b"val").unwrap_or(0),
                b"wBefore" => props.width_before = Some(width(&e)),
                b"wAfter" => props.width_after = Some(width(&e)),
                name @ (b"ins" | b"del") => {
                    let owned = name.to_vec();
                    props.revision = props::revision(&owned, &e);
                }
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"trPr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    props
}

fn read_cell(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> Cell {
    let mut cell = Cell {
        props: CellProps::new(),
        content: Vec::new(),
    };
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"tcPr" => cell.props = read_cell_props(reader),
                    b"p" => cell
                        .content
                        .push(Block::Paragraph(read_paragraph(reader, ctx, &e))),
                    b"tbl" => cell.content.push(Block::Table(read_table(reader, ctx))),
                    b"sdt" => cell.content.push(read_block_sdt(reader, ctx)),
                    other => {
                        let owned = other.to_vec();
                        let _ = skip_element(reader, &owned);
                    }
                }
            }
            Event::Empty(e) => {
                if local_name(&e) == b"p" {
                    cell.content.push(Block::Paragraph(Paragraph::new()));
                } else if let Some(anchor) = read_anchor(&e) {
                    cell.content.push(Block::Anchor(anchor));
                }
            }
            Event::End(e) if end_local_name(&e) == b"tc" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    // The format requires a cell to end with a paragraph, and a document whose
    // cell holds only a table is one Word calls damaged. A reader that has been
    // handed one repairs it rather than passing the fault on to the writer.
    if !cell
        .content
        .last()
        .is_some_and(|block| matches!(block, Block::Paragraph(_)))
    {
        cell.content.push(Block::Paragraph(Paragraph::new()));
    }
    cell
}

fn read_cell_props(reader: &mut Reader<&[u8]>) -> CellProps {
    let mut props = CellProps::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(_) | Event::Empty(_) => {
                let empty = matches!(event, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = event else {
                    unreachable!()
                };
                match local_name(&e) {
                    b"tcW" => props.width = width(&e),
                    b"gridSpan" => props.grid_span = attr_u32(&e, b"val").unwrap_or(1),
                    // A bare `<w:vMerge/>` means *continue*, against the format's own
                    // convention everywhere else. Read as "true" it makes every
                    // merged cell the start of its own merge.
                    b"vMerge" => props.v_merge = Some(VMerge::from_val(val(&e).as_deref())),
                    b"tcBorders" if !empty => {
                        let (borders, diagonals) = read_table_borders(reader, b"tcBorders");
                        props.borders = borders;
                        props.diagonal_down = diagonals.0;
                        props.diagonal_up = diagonals.1;
                    }
                    b"shd" => props.shading = Some(props::shading(&e)),
                    b"tcMar" if !empty => props.margins = read_cell_margins(reader, b"tcMar"),
                    b"vAlign" => {
                        props.v_align = val(&e)
                            .as_deref()
                            .and_then(CellVAlign::from_val)
                            .unwrap_or_default()
                    }
                    b"textDirection" => {
                        props.text_direction = val(&e)
                            .as_deref()
                            .and_then(TextDirection::from_val)
                            .unwrap_or_default()
                    }
                    b"noWrap" => props.no_wrap = on_off(&e),
                    b"tcFitText" => props.fit_text = on_off(&e),
                    b"hideMark" => props.hide_mark = on_off(&e),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"tcPr" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    props
}
