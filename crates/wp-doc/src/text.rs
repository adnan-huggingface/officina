//! Turning a `.doc` into the document model the rest of the application uses.
//!
//! **A paragraph ends at a paragraph mark, and a `.doc` has four of them.**
//! `\r` ends an ordinary paragraph, `\x07` ends a table cell *and*, when the
//! paragraph's properties say so, a table row; `\x0C` is a page or section
//! break; `\x0B` is a line break inside a paragraph, not the end of one. Getting
//! this wrong is what makes a table come out as one long paragraph of run-on
//! words.
//!
//! **Formatting is looked up per character run, not per paragraph.** The bin
//! tables give byte ranges, and the piece table gives byte offsets, so a
//! paragraph is cut where either changes. That is why bold in the middle of a
//! sentence survives.

use std::cell::RefCell;
use std::collections::HashMap;

use wp_model::doc::{
    Block, Drawing, HeaderFooter, Inline, Paragraph, Piece as ModelPiece, Run, ShapeText, Wrap,
};
use wp_model::section::{HeaderId, HeaderKind, HeaderRef};
use wp_model::table::{Cell, Row, Table, TableBorders, Width};
use wp_model::units::Twips;

use crate::{art, fkp, picture, sprm, Doc, Media, Part};

/// Word's own control characters, by the name the format gives them.
mod mark {
    pub const CELL: char = '\u{7}';
    pub const PARAGRAPH: char = '\r';
    pub const LINE: char = '\u{b}';
    pub const PAGE: char = '\u{c}';
    pub const COLUMN: char = '\u{e}';
    pub const FIELD_START: char = '\u{13}';
    pub const FIELD_SEPARATOR: char = '\u{14}';
    pub const FIELD_END: char = '\u{15}';
    /// Where an inline picture or an OLE object is. What it stands for is in
    /// the `Data` stream, at the offset the character's own properties give.
    pub const OBJECT: char = '\u{1}';
    pub const NOTE: char = '\u{2}';
    /// Where a *floating* shape is anchored. The shape itself is in the
    /// drawing layer and may be drawn anywhere on the page; this is only the
    /// paragraph it travels with.
    pub const SHAPE: char = '\u{8}';
}

/// Reads a `.doc` into a document, and the pictures it draws.
///
/// The pictures come out beside the document rather than inside it: the model
/// names a picture by a relationship, the way a `.docx` does, and a `.doc` has
/// no package for one to point into. The caller gives them somewhere to live.
pub fn document(doc: &Doc) -> (wp_model::Document, Vec<Media>) {
    let characters = fkp::characters(&doc.fib, &doc.table, &doc.stream);
    let paragraphs = fkp::paragraphs(&doc.fib, &doc.table, &doc.stream);
    let ranges = doc.fib.counts.ranges();
    let part = |wanted: Part| {
        ranges
            .iter()
            .find(|(part, _, _)| *part == wanted)
            .map(|(_, from, to)| (*from, *to))
            .unwrap_or((0, 0))
    };
    let body = part(Part::Body);
    let headers = part(Part::Headers);

    let (styles, by_istd) = crate::style::read(&doc.fib, &doc.table);
    let read = Reader {
        doc,
        characters,
        paragraphs,
        by_istd,
        art: art::read(&doc.fib, &doc.table),
        anchors: anchors(doc, headers.0),
        media: RefCell::new(Vec::new()),
    };
    let blocks = read.blocks(body.0, body.1);

    let mut document = wp_model::Document::new();
    document.body = blocks;
    document.styles = styles;
    if let Some(section) = crate::section::read(&doc.fib, &doc.table, &doc.stream) {
        document.section = section;
    }
    let (bodies, section_headers, section_footers) = read.header_footers(headers.0);
    document.settings.even_and_odd_headers = facing_pages(&doc.fib, &doc.table);
    document.headers = bodies;
    document.section.headers = section_headers;
    document.section.footers = section_footers;
    (document, read.media.into_inner())
}

/// Every floating shape's anchor, by the character position of the `\x08` that
/// carries it.
///
/// The two tables are kept apart in the file and joined here, because the walk
/// that meets the anchors does not know which document it is in — the header's
/// positions are stated from the header document's own start, so they are
/// moved into the one coordinate space everything else is read in.
fn anchors(doc: &Doc, header_start: u32) -> HashMap<u32, art::Anchor> {
    let mut out = HashMap::new();
    for (field, base) in [
        (crate::fib::field::PLC_SPA_MOM, 0),
        (crate::fib::field::PLC_SPA_HDR, header_start),
    ] {
        for anchor in art::anchors(&doc.fib, &doc.table, field) {
            out.insert(anchor.cp + base, anchor);
        }
    }
    out
}

/// `DopBase.fFacingPages`, bit 0 of the document properties' first byte: a
/// *document*-wide setting, unlike `sprmSFTitlePage`'s per-section one, that
/// decides whether any section's even-page header is ever drawn.
fn facing_pages(fib: &crate::Fib, table: &[u8]) -> bool {
    fib.slice(table, crate::fib::field::DOP)
        .and_then(|dop| dop.first())
        .is_some_and(|byte| byte & 0x01 != 0)
}

/// The six story kinds a section's header document holds, in the order
/// [`Plcfhdd`](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/8f336b7e-66cb-4346-9fd4-88ede9a4a9db)
/// states them, following the six fixed footnote/endnote separator stories.
const STORY_KINDS: [(HeaderKind, bool); 6] = [
    (HeaderKind::Even, false),
    (HeaderKind::Default, false),
    (HeaderKind::Even, true),
    (HeaderKind::Default, true),
    (HeaderKind::First, false),
    (HeaderKind::First, true),
];

/// The header document's story boundaries: a `Plc` of bare CPs, relative to the
/// header document's own start, with a final undefined CP that is ignored.
fn plcfhdd(fib: &crate::Fib, table: &[u8]) -> Vec<u32> {
    let Some(bytes) = fib.slice(table, crate::fib::field::PLCFHDD) else {
        return Vec::new();
    };
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

struct Reader<'a> {
    doc: &'a Doc,
    characters: Vec<fkp::Exception>,
    paragraphs: Vec<fkp::Exception>,
    /// The file numbers its styles its own way; this maps its numbering onto
    /// the model'''s.
    by_istd: Vec<Option<wp_model::style::StyleId>>,
    /// Every shape and every shared picture in the document.
    art: art::Drawings,
    /// Where each floating shape is anchored, by character position.
    anchors: HashMap<u32, art::Anchor>,
    /// The pictures met so far. Held here rather than returned from every
    /// call because a picture is found in the middle of reading a run, and
    /// threading a sink through the whole walk to say so would be worse.
    media: RefCell<Vec<Media>>,
}

/// One paragraph, before it is decided whether it is a paragraph or a table row.
struct Read {
    paragraph: Paragraph,
    in_table: bool,
    /// The paragraph is the `\x07` that ends a row rather than a cell.
    row_end: bool,
    /// It ended with a cell mark, so it is the last paragraph of a cell.
    cell_end: bool,
    /// The row's geometry, read from the row mark's own `grpprl` — only ever
    /// set when `row_end` is, since that is the one paragraph a `.doc` states
    /// `sprmTDefTable`/`sprmTTableBorders80` on.
    row_props: Option<sprm::TableRow>,
}

impl Reader<'_> {
    /// Every block between two character positions.
    fn blocks(&self, from: u32, to: u32) -> Vec<Block> {
        let read = self.paragraphs_between(from, to);
        assemble(read)
    }

    /// Cuts a character range into paragraphs at the paragraph marks.
    fn paragraphs_between(&self, from: u32, to: u32) -> Vec<Read> {
        let text = self.doc.pieces.text(&self.doc.stream, from, to);
        let mut out = Vec::new();
        let mut start = from;
        let mut at = from;
        for character in text.chars() {
            at += 1;
            match character {
                mark::PARAGRAPH | mark::CELL => {
                    out.push(self.paragraph(start, at - 1, at - 1, character == mark::CELL));
                    start = at;
                }
                _ => {}
            }
        }
        // A last paragraph with no mark after it: Word does not write one, but a
        // damaged file can end anywhere and the text should not be dropped.
        if start < at {
            out.push(self.paragraph(start, at, at.saturating_sub(1), false));
        }
        out
    }

    /// One paragraph: its runs, and the properties of the mark that ends it.
    ///
    /// The properties are on the *mark*, not on the paragraph's first character.
    /// This is the part of the format that is easiest to get subtly wrong: a
    /// reader that looks at the start gets the previous paragraph's style.
    fn paragraph(&self, from: u32, to: u32, mark: u32, cell_end: bool) -> Read {
        let exception = self
            .doc
            .pieces
            .offset_of(mark)
            .and_then(|(offset, _)| fkp::at(&self.paragraphs, offset));
        let (istd, props) = fkp::para_props(exception);
        let (in_table, row_end, row_props) = exception
            .map(|exception| {
                let (_, rest) = fkp::split_istd(&exception.grpprl);
                let (in_table, row_end) = sprm::table_flags(rest);
                let row_props = row_end.then(|| sprm::table_row(rest));
                (in_table, row_end, row_props)
            })
            .unwrap_or((false, false, None));

        let mut paragraph = Paragraph::new();
        paragraph.props = props;
        paragraph.props.style = istd.and_then(|index| self.style_of(index));
        paragraph.content = self.runs(from, to);
        Read {
            paragraph,
            in_table,
            row_end,
            cell_end,
            row_props,
        }
    }

    /// The first section's headers and footers: the bodies to add to the
    /// document, and the references that name them.
    ///
    /// Only the first section's — the rest of this reader only models the
    /// first section's page setup too (see the crate's own doc comment), and a
    /// header document's per-section groups of six stories are laid out end to
    /// end, so a later section's would simply start right after this one's.
    fn header_footers(
        &self,
        header_doc_from: u32,
    ) -> (Vec<HeaderFooter>, Vec<HeaderRef>, Vec<HeaderRef>) {
        let acp = plcfhdd(&self.doc.fib, &self.doc.table);
        // Six fixed footnote/endnote separator stories, then this section's six
        // header/footer stories: seven boundary CPs, indices 6 through 12.
        if acp.len() < 13 {
            return (Vec::new(), Vec::new(), Vec::new());
        }
        let mut bodies = Vec::new();
        let mut headers = Vec::new();
        let mut footers = Vec::new();
        for (index, (kind, is_footer)) in STORY_KINDS.iter().enumerate() {
            let start = acp[6 + index];
            let end = acp[7 + index];
            if end <= start {
                continue; // An empty story: no header/footer of this kind.
            }
            // The story's own trailing paragraph mark is content; the guard
            // mark right after it, which separates it from the next story, is
            // not (see the header-document overview) — so the last character
            // of the span is dropped.
            let from = header_doc_from + start;
            let to = header_doc_from + end - 1;
            let id = HeaderId(bodies.len() as u32);
            bodies.push(HeaderFooter {
                id,
                part: None,
                rel: None,
                footer: *is_footer,
                content: self.blocks(from, to),
            });
            let reference = HeaderRef {
                kind: *kind,
                body: id,
                rel: None,
            };
            if *is_footer {
                footers.push(reference);
            } else {
                headers.push(reference);
            }
        }
        (bodies, headers, footers)
    }

    /// The model's id for one of the file's style indices.
    ///
    /// Index 0 is "Normal", which every paragraph is in by default and which
    /// says nothing; naming it on every paragraph would bury the ones that mean
    /// something.
    fn style_of(&self, istd: u16) -> Option<wp_model::style::StyleId> {
        match istd {
            0 => None,
            index => self.by_istd.get(index as usize).copied().flatten(),
        }
    }

    /// What one of the two drawing characters stands for, if anything.
    fn drawing(
        &self,
        character: char,
        cp: u32,
        exception: Option<&fkp::Exception>,
    ) -> Option<ModelPiece> {
        let drawing = match character {
            mark::OBJECT => self.inline_picture(exception?)?,
            mark::SHAPE => self.floating_shape(cp)?,
            _ => return None,
        };
        Some(ModelPiece::Drawing(Box::new(drawing)))
    }

    /// The picture a `\x01` stands for.
    ///
    /// **The same property points at something else entirely** when the
    /// character's `sprmCFData` is set: a form field's binary data, not a
    /// picture. Reading that as one gives a picture of whatever the field
    /// happens to contain, and a document of checkboxes becomes a document of
    /// broken image frames.
    fn inline_picture(&self, exception: &fkp::Exception) -> Option<Drawing> {
        let mut location = None;
        let mut is_form_data = false;
        for sprm in sprm::walk(&exception.grpprl) {
            match sprm.opcode {
                0x6A03 => {
                    location = sprm
                        .operand
                        .get(..4)
                        .map(|at| u32::from_le_bytes([at[0], at[1], at[2], at[3]]))
                }
                0x0806 => is_form_data = sprm.operand.first().copied().unwrap_or(0) != 0,
                _ => {}
            }
        }
        if is_form_data {
            return None;
        }
        let picture = picture::at(&self.doc.data, location? as usize)?;
        // The bytes are usually beside the picture; a picture that names one
        // of the document's shared BLIPs instead is the other half of it.
        let blip = picture.blip.or_else(|| {
            let index = picture.shape.as_ref()?.picture?;
            self.art.blip(index).cloned()
        });
        Some(Drawing {
            anchored: false,
            extent: (emu(picture.width), emu(picture.height)),
            rel: blip.map(|blip| self.remember(blip)),
            ..blank()
        })
    }

    /// The shape a `\x08` anchors, with everything the page needs to place it.
    ///
    /// A shape with neither a picture nor words of its own draws nothing —
    /// the invisible box a group hangs off is one of these — and answers
    /// `None` rather than an empty frame.
    fn floating_shape(&self, cp: u32) -> Option<Drawing> {
        use wp_model::doc::{DrawingPosition, Offset};

        let anchor = self.anchors.get(&cp)?;
        let shape = self.art.shape(anchor.spid)?;
        let rel = shape
            .picture
            .and_then(|index| self.art.blip(index))
            .map(|blip| self.remember(blip.clone()));
        let text = shape.text.as_deref().filter(|text| !text.is_empty());
        // A rectangle is drawn as itself. Every other geometry this does not
        // know how to trace, so it draws nothing rather than a wrong outline.
        let outline = (shape.kind == RECTANGLE)
            .then(|| wp_model::doc::ShapeOutline {
                fill: shape.fill.filter(|_| shape.filled),
                line: shape.lined.then_some(shape.line),
                line_width: wp_model::Emu(shape.line_width as i64),
            })
            .filter(|outline| outline.fill.is_some() || outline.line.is_some());
        if rel.is_none() && text.is_none() && outline.is_none() {
            return None;
        }
        // The anchor's rectangle says where the shape is; `posh`/`posv` say it
        // better when they are there, because they are what Word wrote when
        // the user said "centre this on the page".
        let axis = |offset: i32, frame: u8, stated: Option<art::Placement>, vertical: bool| {
            let (relative_to, align) = match stated.filter(|stated| stated.align != 0) {
                Some(stated) => (
                    relative_of(stated.relative_to, vertical),
                    align_of(stated.align, vertical),
                ),
                None => (frame_of(frame, vertical), None),
            };
            Offset {
                relative_to,
                offset: align.is_none().then(|| emu(offset)),
                align,
            }
        };
        Some(Drawing {
            anchored: true,
            extent: (
                emu(anchor.right - anchor.left),
                emu(anchor.bottom - anchor.top),
            ),
            rel,
            name: shape.name.as_deref().map(Into::into),
            wrap: match anchor.wrap {
                1 => Wrap::TopAndBottom,
                3 => Wrap::None,
                4 | 5 => Wrap::Tight,
                _ => Wrap::Square,
            },
            position: Some(Box::new(DrawingPosition {
                horizontal: axis(anchor.left, anchor.horizontal, shape.horizontal, false),
                vertical: axis(anchor.top, anchor.vertical, shape.vertical, true),
            })),
            behind_text: anchor.below_text,
            outline,
            text: text.map(|text| {
                Box::new(ShapeText {
                    text: text.into(),
                    font: shape.font.as_deref().map(Into::into),
                    color: shape.fill.filter(|_| shape.filled),
                    bold: false,
                    italic: false,
                    rotation: shape.rotation,
                })
            }),
            ..blank()
        })
    }

    /// Keeps a picture's bytes, and answers the name the model calls them by.
    ///
    /// The same picture met twice — a logo on a shape in every header — is
    /// kept once, because everything downstream decodes one relationship one
    /// time and a second name would cost a second decode of the same bytes.
    fn remember(&self, blip: crate::art::Blip) -> std::sync::Arc<str> {
        let mut media = self.media.borrow_mut();
        if let Some(found) = media.iter().find(|held| held.data == blip.data) {
            return found.rel.as_str().into();
        }
        let rel = format!("doc-picture-{}", media.len() + 1);
        media.push(Media {
            rel: rel.clone(),
            data: blip.data,
            content_type: blip.content_type,
        });
        rel.into()
    }

    /// Cuts a paragraph's text where its character formatting changes.
    fn runs(&self, from: u32, to: u32) -> Vec<Inline> {
        let mut out: Vec<Inline> = Vec::new();
        let mut at = from;
        while at < to {
            let Some((offset, compressed)) = self.doc.pieces.offset_of(at) else {
                break;
            };
            let exception = fkp::at(&self.characters, offset);
            // The run goes as far as the exception does, or as far as the
            // paragraph does, whichever comes first — and the exception is in
            // bytes, which are half as many as characters when a piece is not
            // compressed.
            let width = if compressed { 1 } else { 2 };
            let bytes = exception.map(|e| e.to - offset).unwrap_or(usize::MAX);
            let span = (bytes / width) as u32;
            let end = to.min(at.saturating_add(span.max(1)));

            let mut props = wp_model::prop::RunProps::default();
            if let Some(exception) = exception {
                sprm::apply_run(&mut props, &exception.grpprl);
                if let Some(index) = sprm::run_style(&exception.grpprl) {
                    props.style = self.style_of(index);
                }
            }
            let text = self.doc.pieces.text(&self.doc.stream, at, end);
            let mut run = Run::new();
            run.props = props;
            run.content = pieces(&text, at, &mut |character, cp| {
                self.drawing(character, cp, exception)
            });
            if !run.content.is_empty() {
                out.push(Inline::Run(run));
            }
            at = end;
        }
        out
    }
}

/// Word's control characters become the model's own pieces, so that nothing is
/// silently dropped and nothing pretends to be text.
///
/// Two of them stand for something the text does not hold: an inline picture
/// and a floating shape's anchor. `drawing` is asked what they are, and gets
/// the character position as well as the character because a shape is found by
/// where it is anchored and by nothing else.
fn pieces(
    text: &str,
    from: u32,
    drawing: &mut dyn FnMut(char, u32) -> Option<ModelPiece>,
) -> Vec<ModelPiece> {
    let mut out = Vec::new();
    let mut buffer = String::new();
    let flush = |buffer: &mut String, out: &mut Vec<ModelPiece>| {
        if !buffer.is_empty() {
            out.push(ModelPiece::Text(std::mem::take(buffer).into()));
        }
    };
    for (cp, character) in (from..).zip(text.chars()) {
        match character {
            '\t' => {
                flush(&mut buffer, &mut out);
                out.push(ModelPiece::Tab);
            }
            mark::LINE => {
                flush(&mut buffer, &mut out);
                out.push(ModelPiece::Break(wp_model::doc::Break::Line));
            }
            mark::PAGE => {
                flush(&mut buffer, &mut out);
                out.push(ModelPiece::Break(wp_model::doc::Break::Page));
            }
            mark::COLUMN => {
                flush(&mut buffer, &mut out);
                out.push(ModelPiece::Break(wp_model::doc::Break::Column));
            }
            mark::FIELD_START => {
                flush(&mut buffer, &mut out);
                out.push(ModelPiece::FieldStart {
                    dirty: false,
                    lock: false,
                });
            }
            mark::FIELD_SEPARATOR => {
                flush(&mut buffer, &mut out);
                out.push(ModelPiece::FieldSeparate);
            }
            mark::FIELD_END => {
                flush(&mut buffer, &mut out);
                out.push(ModelPiece::FieldEnd);
            }
            // A picture, a shape's anchor, a note reference or the mark that
            // ends a cell. The first two stand for something the drawing layer
            // holds; the last two for something that is not in the text at all.
            mark::OBJECT | mark::SHAPE => {
                flush(&mut buffer, &mut out);
                out.extend(drawing(character, cp));
            }
            mark::NOTE | mark::CELL => flush(&mut buffer, &mut out),
            '\u{0}' => {}
            other => buffer.push(other),
        }
    }
    flush(&mut buffer, &mut out);
    out
}

/// `msosptRectangle` — the one shape geometry this draws as a shape.
const RECTANGLE: u16 = 1;

/// What the *anchor* measures its rectangle from: `Spa.bx` and `Spa.by`.
fn frame_of(value: u8, vertical: bool) -> wp_model::doc::RelativeTo {
    use wp_model::doc::RelativeTo;
    match (value, vertical) {
        (0, false) => RelativeTo::Margin,
        (0, true) => RelativeTo::TopMargin,
        (1, _) => RelativeTo::Page,
        // The column, and — the vertical one — the paragraph the anchor is in,
        // which is what makes a picture travel with its text.
        (_, false) => RelativeTo::Column,
        (_, true) => RelativeTo::Paragraph,
    }
}

/// What `posrelh`/`posrelv` measure from, which is a different list.
fn relative_of(value: u8, vertical: bool) -> wp_model::doc::RelativeTo {
    use wp_model::doc::RelativeTo;
    match (value, vertical) {
        (1, _) => RelativeTo::Page,
        (2, false) => RelativeTo::Column,
        (2, true) => RelativeTo::Paragraph,
        (3, false) => RelativeTo::Character,
        (3, true) => RelativeTo::Line,
        (_, false) => RelativeTo::Margin,
        (_, true) => RelativeTo::TopMargin,
    }
}

/// `posh`/`posv`, which name a side rather than a distance.
fn align_of(value: u8, vertical: bool) -> Option<wp_model::doc::Alignment> {
    use wp_model::doc::Alignment;
    Some(match (value, vertical) {
        (1, false) => Alignment::Left,
        (1, true) => Alignment::Top,
        (2, _) => Alignment::Center,
        (3, false) => Alignment::Right,
        (3, true) => Alignment::Bottom,
        (4, _) => Alignment::Inside,
        (5, _) => Alignment::Outside,
        // Zero is "the rectangle decides", and is filtered out before this.
        _ => return None,
    })
}

/// Twips, as the drawing layer states every measurement, in the EMUs the
/// model holds a drawing's geometry in. There are 635 of them to the twip.
fn emu(twips: i32) -> wp_model::Emu {
    wp_model::Emu(twips as i64 * 635)
}

/// A drawing with nothing in it, for the two readers above to fill in. The
/// model has no `Default` for one, and spelling out nine empty fields twice
/// buries the three that carry the answer.
fn blank() -> Drawing {
    Drawing {
        source: Vec::new().into(),
        anchored: false,
        extent: (wp_model::Emu(0), wp_model::Emu(0)),
        rel: None,
        chart: None,
        name: None,
        description: None,
        wrap: Wrap::None,
        distance: (
            wp_model::Emu(0),
            wp_model::Emu(0),
            wp_model::Emu(0),
            wp_model::Emu(0),
        ),
        position: None,
        behind_text: false,
        text: None,
        outline: None,
    }
}

/// Groups the paragraphs that are in a table into rows and cells.
///
/// A `.doc` does not nest its tables in the file: every paragraph is at the top
/// level, and the ones inside a table say so. A cell ends at a cell mark and a
/// row ends at the paragraph whose properties say it is a row mark, so the shape
/// has to be put back from the marks alone.
fn assemble(read: Vec<Read>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut table = Building::default();
    let mut cells: Vec<Cell> = Vec::new();
    let mut cell: Vec<Block> = Vec::new();

    for item in read {
        if !item.in_table {
            // Whatever table was open ends here.
            if !table.is_empty() || !cells.is_empty() {
                table.flush(&mut out, &mut cells, &mut cell);
            }
            out.push(Block::Paragraph(item.paragraph));
            continue;
        }
        if item.row_end {
            // The row mark's own paragraph is not content.
            if !cell.is_empty() {
                cells.push(cell_of(std::mem::take(&mut cell)));
            }
            table.close_row(std::mem::take(&mut cells), item.row_props);
            continue;
        }
        cell.push(Block::Paragraph(item.paragraph));
        if item.cell_end {
            cells.push(cell_of(std::mem::take(&mut cell)));
        }
    }
    if !table.is_empty() || !cells.is_empty() || !cell.is_empty() {
        table.flush(&mut out, &mut cells, &mut cell);
    }
    out
}

/// The table currently being put back together, and the geometry its rows
/// stated as they went by.
///
/// The rows are held rather than emitted because **a `.doc` states the grid per
/// row and the rows do not have to agree**: a row whose cells span two of the
/// table's columns simply states one boundary fewer. The grid is therefore not
/// known until the last row is in, and neither is any cell's span.
#[derive(Default)]
struct Building {
    rows: Vec<Row>,
    /// One entry per row, in step with `rows`.
    geometry: Vec<Option<sprm::TableRow>>,
    /// A `.doc` states these once per row too; real tables keep them constant,
    /// so the first row to say anything wins and later rows are trusted.
    borders: Option<TableBorders>,
    padding: Option<wp_model::table::CellMargins>,
    gap_half: Option<Twips>,
}

impl Building {
    fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn close_row(&mut self, cells: Vec<Cell>, props: Option<sprm::TableRow>) {
        let mut row = Row::new();
        row.cells = cells;
        if let Some(props) = &props {
            row.props.height = props.height;
            if props.borders.is_some() {
                self.borders = props.borders;
            }
            if props.padding.is_some() {
                self.padding = props.padding;
            }
            if props.gap_half.is_some() {
                self.gap_half = props.gap_half;
            }
        }
        self.rows.push(row);
        self.geometry.push(props);
    }

    /// Emits the table, once every row's geometry is known.
    fn flush(&mut self, out: &mut Vec<Block>, cells: &mut Vec<Cell>, cell: &mut Vec<Block>) {
        if !cell.is_empty() {
            cells.push(cell_of(std::mem::take(cell)));
        }
        if !cells.is_empty() {
            self.close_row(std::mem::take(cells), None);
        }
        let built = std::mem::take(self);
        if built.rows.is_empty() {
            return;
        }
        out.push(Block::Table(built.into_table()));
    }

    fn into_table(mut self) -> Table {
        // The union of every row's boundaries, which is the grid Word draws
        // this table on. `<w:tblGrid>` states it directly; a `.doc` leaves it
        // to be worked out.
        let mut boundaries: Vec<i32> = self
            .geometry
            .iter()
            .flatten()
            .filter_map(|row| row.boundaries.as_ref())
            .flatten()
            .copied()
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();

        for (row, props) in self.rows.iter_mut().zip(&self.geometry) {
            let Some(edges) = props.as_ref().and_then(|row| row.boundaries.as_ref()) else {
                continue;
            };
            let column = |at: i32| boundaries.iter().position(|edge| *edge == at);
            row.props.grid_before = column(edges[0]).unwrap_or(0) as u32;
            let defs: &[sprm::CellDef] = props.as_ref().map_or(&[], |row| &row.cells);
            for (index, cell) in row.cells.iter_mut().enumerate() {
                let (Some(start), Some(end)) = (
                    edges.get(index).copied().and_then(column),
                    edges.get(index + 1).copied().and_then(column),
                ) else {
                    continue;
                };
                cell.props.grid_span = (end - start).max(1) as u32;
                cell.props.width = Width::Fixed(Twips(edges[index + 1] - edges[index]));
                if let Some(def) = defs.get(index) {
                    cell.props.borders = def.borders;
                    cell.props.v_merge = def.v_merge;
                    cell.props.v_align = def.v_align;
                }
            }
            fold_horizontal_merges(row, defs);
        }

        let mut table = Table::new();
        table.grid = boundaries
            .windows(2)
            .map(|pair| Twips(pair[1] - pair[0]))
            .collect();
        if let Some(borders) = self.borders {
            table.props.borders = borders;
        }
        if let Some(padding) = self.padding {
            table.props.cell_margins = padding;
        } else if let Some(Twips(half)) = self.gap_half {
            // A file old enough to state only `sprmTDxaGapHalf` says the same
            // thing the long way round: half a gap of padding on each side.
            table.props.cell_margins = wp_model::table::CellMargins {
                start: Some(Width::Fixed(Twips(half))),
                end: Some(Width::Fixed(Twips(half))),
                ..Default::default()
            };
        }
        // The first boundary is where the *gap* starts, half a gap to the left
        // of where the first cell's text does, so a table flush with the margin
        // states -108 and not zero.
        if let Some(left) = boundaries.first() {
            let half = self.gap_half.unwrap_or(Twips(0)).0;
            table.props.indent = Some(Width::Fixed(Twips(left + half)));
        }
        table.rows = std::mem::take(&mut self.rows);
        table
    }
}

/// Folds away Word 97's horizontal merge, where the merged cells stay in the
/// row and `fMerged` says to draw them as one.
///
/// The model has no such flag: a cell that covers three columns says so with
/// its span, once. So the run's cells become one cell, whose content is
/// everything they held — dropping the followers' paragraphs would lose text
/// that Word shows, since it is the run that is drawn as a single cell and not
/// the first cell of it.
fn fold_horizontal_merges(row: &mut Row, defs: &[sprm::CellDef]) {
    if !defs.iter().any(|def| def.merged_left) {
        return;
    }
    let mut kept: Vec<Cell> = Vec::with_capacity(row.cells.len());
    for (index, cell) in std::mem::take(&mut row.cells).into_iter().enumerate() {
        match (
            defs.get(index).is_some_and(|def| def.merged_left),
            kept.last_mut(),
        ) {
            (true, Some(first)) => {
                first.props.grid_span += cell.props.span();
                if let Some(Width::Fixed(Twips(width))) = Some(cell.props.width) {
                    if let Width::Fixed(Twips(so_far)) = first.props.width {
                        first.props.width = Width::Fixed(Twips(so_far + width));
                    }
                }
                first.content.extend(cell.content);
            }
            _ => kept.push(cell),
        }
    }
    row.cells = kept;
}

fn cell_of(content: Vec<Block>) -> Cell {
    let mut cell = Cell::new();
    cell.content = content;
    cell
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(text: &str, in_table: bool, row_end: bool, cell_end: bool) -> Read {
        let mut paragraph = Paragraph::new();
        let mut run = Run::new();
        run.content = vec![ModelPiece::Text(text.into())];
        paragraph.content = vec![Inline::Run(run)];
        Read {
            paragraph,
            in_table,
            row_end,
            cell_end,
            row_props: None,
        }
    }

    #[test]
    fn paragraphs_outside_a_table_stay_where_they_are() {
        let blocks = assemble(vec![
            paragraph("one", false, false, false),
            paragraph("two", false, false, false),
        ]);
        assert_eq!(blocks.len(), 2);
        assert!(blocks
            .iter()
            .all(|block| matches!(block, Block::Paragraph(_))));
    }

    #[test]
    fn cell_marks_and_row_marks_put_a_table_back_together() {
        // The file has no nesting at all: six paragraphs in a row become a
        // two-by-two table because of where the marks fall.
        let blocks = assemble(vec![
            paragraph("before", false, false, false),
            paragraph("a", true, false, true),
            paragraph("b", true, false, true),
            paragraph("", true, true, false),
            paragraph("c", true, false, true),
            paragraph("d", true, false, true),
            paragraph("", true, true, false),
            paragraph("after", false, false, false),
        ]);
        assert_eq!(blocks.len(), 3);
        let Block::Table(table) = &blocks[1] else {
            panic!("the middle block is a table, not {:?}", blocks[1]);
        };
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(table.rows[1].cells.len(), 2);
    }

    #[test]
    fn a_table_that_runs_to_the_end_of_the_document_is_still_a_table() {
        let blocks = assemble(vec![
            paragraph("a", true, false, true),
            paragraph("", true, true, false),
        ]);
        assert!(matches!(blocks.first(), Some(Block::Table(_))));
    }

    /// A row mark's geometry, stated the way `sprmTDefTable` states it.
    fn row_props(edges: &[i32], cells: Vec<sprm::CellDef>) -> Option<sprm::TableRow> {
        Some(sprm::TableRow {
            boundaries: Some(edges.to_vec()),
            cells,
            ..Default::default()
        })
    }

    fn merged(v_merge: Option<wp_model::table::VMerge>) -> sprm::CellDef {
        sprm::CellDef {
            v_merge,
            ..Default::default()
        }
    }

    #[test]
    fn a_row_with_fewer_columns_spans_the_grid_the_other_rows_state() {
        // The letterhead case: a four-column row above a three-column one whose
        // last cell covers the last two. Reading the grid off the first row and
        // stopping there leaves that cell one column short and the fourth
        // column empty, which is a table with a hole in the corner.
        let mut wide = paragraph("", true, true, false);
        wide.row_props = row_props(&[0, 1000, 2000, 3000], vec![sprm::CellDef::default(); 3]);
        let mut narrow = paragraph("", true, true, false);
        narrow.row_props = row_props(&[0, 1000, 3000], vec![sprm::CellDef::default(); 2]);

        let blocks = assemble(vec![
            paragraph("a", true, false, true),
            paragraph("b", true, false, true),
            paragraph("c", true, false, true),
            wide,
            paragraph("d", true, false, true),
            paragraph("e", true, false, true),
            narrow,
        ]);
        let Block::Table(table) = &blocks[0] else {
            panic!("a table, not {:?}", blocks[0]);
        };
        assert_eq!(
            table.grid,
            vec![Twips(1000), Twips(1000), Twips(1000)],
            "the grid is the union of both rows' boundaries"
        );
        let spans = |row: &wp_model::table::Row| -> Vec<u32> {
            row.cells.iter().map(|cell| cell.props.span()).collect()
        };
        assert_eq!(spans(&table.rows[0]), [1, 1, 1]);
        assert_eq!(spans(&table.rows[1]), [1, 2], "the wide cell says so");
    }

    #[test]
    fn a_row_that_starts_part_way_across_leaves_the_columns_before_it_empty() {
        let mut first = paragraph("", true, true, false);
        first.row_props = row_props(&[0, 1000, 2000], vec![sprm::CellDef::default(); 2]);
        let mut second = paragraph("", true, true, false);
        second.row_props = row_props(&[1000, 2000], vec![sprm::CellDef::default()]);

        let blocks = assemble(vec![
            paragraph("a", true, false, true),
            paragraph("b", true, false, true),
            first,
            paragraph("c", true, false, true),
            second,
        ]);
        let Block::Table(table) = &blocks[0] else {
            panic!("a table");
        };
        assert_eq!(table.rows[1].props.grid_before, 1);
    }

    #[test]
    fn a_vertically_merged_cell_continues_the_one_above_rather_than_starting_its_own() {
        use wp_model::table::VMerge;
        let mut first = paragraph("", true, true, false);
        first.row_props = row_props(&[0, 1000], vec![merged(Some(VMerge::Restart))]);
        let mut second = paragraph("", true, true, false);
        second.row_props = row_props(&[0, 1000], vec![merged(Some(VMerge::Continue))]);

        let blocks = assemble(vec![
            paragraph("letterhead", true, false, true),
            first,
            paragraph("", true, false, true),
            second,
        ]);
        let Block::Table(table) = &blocks[0] else {
            panic!("a table");
        };
        assert_eq!(table.rows[0].cells[0].props.v_merge, Some(VMerge::Restart));
        assert!(table.rows[1].cells[0].props.is_merged_up());
    }

    #[test]
    fn word_97s_horizontal_merge_becomes_one_cell_that_spans_rather_than_three() {
        // `fMerged` keeps the cells in the row and asks for them to be drawn as
        // one; the model has no such flag, so the run has to become a single
        // cell of the same width — with everything they held, since it is the
        // run and not its first cell that Word draws.
        let follower = sprm::CellDef {
            merged_left: true,
            ..Default::default()
        };
        let mut mark = paragraph("", true, true, false);
        mark.row_props = row_props(
            &[0, 1000, 2000, 3000],
            vec![sprm::CellDef::default(), follower, follower],
        );
        let blocks = assemble(vec![
            paragraph("one", true, false, true),
            paragraph("", true, false, true),
            paragraph("", true, false, true),
            mark,
        ]);
        let Block::Table(table) = &blocks[0] else {
            panic!("a table");
        };
        assert_eq!(table.rows[0].cells.len(), 1);
        assert_eq!(table.rows[0].cells[0].props.span(), 3);
    }

    #[test]
    fn control_characters_become_pieces_rather_than_letters() {
        // Otherwise a field code's braces appear in the middle of a sentence and
        // a tab is a space.
        let out = pieces("a\tb\u{b}c\u{13}d\u{15}", 0, &mut |_, _| None);
        assert!(matches!(out[1], ModelPiece::Tab));
        assert!(matches!(
            out[3],
            ModelPiece::Break(wp_model::doc::Break::Line)
        ));
        assert!(matches!(out[5], ModelPiece::FieldStart { .. }));
        assert!(matches!(out.last(), Some(ModelPiece::FieldEnd)));
    }
}
