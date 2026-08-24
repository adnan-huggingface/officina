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

use wp_model::doc::{Block, HeaderFooter, Inline, Paragraph, Piece as ModelPiece, Run};
use wp_model::section::{HeaderId, HeaderKind, HeaderRef};
use wp_model::table::{Cell, Row, Table, TableBorders};
use wp_model::units::Twips;

use crate::{fkp, sprm, Doc, Part};

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
    /// Where a picture or an OLE object was. Read as nothing, and said so.
    pub const OBJECT: char = '\u{1}';
    pub const NOTE: char = '\u{2}';
}

/// Reads a `.doc` into a document.
pub fn document(doc: &Doc) -> wp_model::Document {
    let characters = fkp::characters(&doc.fib, &doc.table, &doc.stream);
    let paragraphs = fkp::paragraphs(&doc.fib, &doc.table, &doc.stream);
    let ranges = doc.fib.counts.ranges();
    let body = ranges
        .iter()
        .find(|(part, _, _)| *part == Part::Body)
        .map(|(_, from, to)| (*from, *to))
        .unwrap_or((0, 0));

    let (styles, by_istd) = crate::style::read(&doc.fib, &doc.table);
    let read = Reader {
        doc,
        characters,
        paragraphs,
        by_istd,
    };
    let blocks = read.blocks(body.0, body.1);

    let mut document = wp_model::Document::new();
    document.body = blocks;
    document.styles = styles;
    if let Some(section) = crate::section::read(&doc.fib, &doc.table, &doc.stream) {
        document.section = section;
    }
    let headers = ranges
        .iter()
        .find(|(part, _, _)| *part == Part::Headers)
        .map(|(_, from, to)| (*from, *to))
        .unwrap_or((0, 0));
    let (bodies, section_headers, section_footers) = read.header_footers(headers.0);
    document.settings.even_and_odd_headers = facing_pages(&doc.fib, &doc.table);
    document.headers = bodies;
    document.section.headers = section_headers;
    document.section.footers = section_footers;
    document
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
            run.content = pieces(&text);
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
fn pieces(text: &str) -> Vec<ModelPiece> {
    let mut out = Vec::new();
    let mut buffer = String::new();
    let flush = |buffer: &mut String, out: &mut Vec<ModelPiece>| {
        if !buffer.is_empty() {
            out.push(ModelPiece::Text(std::mem::take(buffer).into()));
        }
    };
    for character in text.chars() {
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
            // A picture, an OLE object or a note reference. There is nothing in
            // the text for these — the bytes are elsewhere, and this reader does
            // not go and get them.
            mark::OBJECT | mark::NOTE | mark::CELL => flush(&mut buffer, &mut out),
            '\u{0}' => {}
            other => buffer.push(other),
        }
    }
    flush(&mut buffer, &mut out);
    out
}

/// Groups the paragraphs that are in a table into rows and cells.
///
/// A `.doc` does not nest its tables in the file: every paragraph is at the top
/// level, and the ones inside a table say so. A cell ends at a cell mark and a
/// row ends at the paragraph whose properties say it is a row mark, so the shape
/// has to be put back from the marks alone.
fn assemble(read: Vec<Read>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut cells: Vec<Cell> = Vec::new();
    let mut cell: Vec<Block> = Vec::new();
    // A `.doc` states a table's grid and its row-level border once per row
    // (see `sprm::table_row`'s doc comment); real tables keep both constant
    // across rows, so the first row to state them wins and later rows are
    // trusted to agree.
    let mut grid: Option<Vec<Twips>> = None;
    let mut borders: Option<TableBorders> = None;

    for item in read {
        if !item.in_table {
            // Whatever table was open ends here.
            if !rows.is_empty() || !cells.is_empty() {
                flush_table(
                    &mut out,
                    &mut rows,
                    &mut cells,
                    &mut cell,
                    &mut grid,
                    &mut borders,
                );
            }
            out.push(Block::Paragraph(item.paragraph));
            continue;
        }
        if item.row_end {
            // The row mark's own paragraph is not content.
            if !cell.is_empty() {
                cells.push(cell_of(std::mem::take(&mut cell)));
            }
            if let Some(row_props) = item.row_props {
                if grid.is_none() {
                    grid = row_props.grid;
                }
                if row_props.borders.is_some() {
                    borders = row_props.borders;
                }
                for (cell, cell_borders) in cells.iter_mut().zip(row_props.cells) {
                    cell.props.borders = cell_borders;
                }
            }
            let mut row = Row::new();
            row.cells = std::mem::take(&mut cells);
            rows.push(row);
            continue;
        }
        cell.push(Block::Paragraph(item.paragraph));
        if item.cell_end {
            cells.push(cell_of(std::mem::take(&mut cell)));
        }
    }
    if !rows.is_empty() || !cells.is_empty() || !cell.is_empty() {
        flush_table(
            &mut out,
            &mut rows,
            &mut cells,
            &mut cell,
            &mut grid,
            &mut borders,
        );
    }
    out
}

fn flush_table(
    out: &mut Vec<Block>,
    rows: &mut Vec<Row>,
    cells: &mut Vec<Cell>,
    cell: &mut Vec<Block>,
    grid: &mut Option<Vec<Twips>>,
    borders: &mut Option<TableBorders>,
) {
    if !cell.is_empty() {
        cells.push(cell_of(std::mem::take(cell)));
    }
    if !cells.is_empty() {
        let mut row = Row::new();
        row.cells = std::mem::take(cells);
        rows.push(row);
    }
    let grid = grid.take();
    let borders = borders.take();
    if rows.is_empty() {
        return;
    }
    let mut table = Table::new();
    table.rows = std::mem::take(rows);
    if let Some(grid) = grid {
        table.grid = grid;
    }
    if let Some(borders) = borders {
        table.props.borders = borders;
    }
    out.push(Block::Table(table));
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

    #[test]
    fn control_characters_become_pieces_rather_than_letters() {
        // Otherwise a field code's braces appear in the middle of a sentence and
        // a tab is a space.
        let out = pieces("a\tb\u{b}c\u{13}d\u{15}");
        assert!(matches!(out[1], ModelPiece::Tab));
        assert!(matches!(
            out[3],
            ModelPiece::Break(wp_model::doc::Break::Line)
        ));
        assert!(matches!(out[5], ModelPiece::FieldStart { .. }));
        assert!(matches!(out.last(), Some(ModelPiece::FieldEnd)));
    }
}
