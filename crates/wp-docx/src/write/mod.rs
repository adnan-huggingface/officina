//! Writing a document back to its package.
//!
//! **The writer edits `document.xml`; it does not reprint it.** The part is
//! walked with a [`splice::Splicer`], and each `<w:p>` and `<w:tbl>` is compared
//! against the model by *re-reading it* and asking whether the result differs.
//! One that does not is copied byte for byte — its `w:rsid*` attributes, its
//! `<w:proofErr>` marks, its `<mc:AlternateContent>` fallbacks and everything
//! else this crate does not model included. One that does is emitted from the
//! model.
//!
//! Comparing by re-reading rather than by remembering is deliberate. It means
//! the writer never has to trust what a reader believed some time earlier, and
//! it means the definition of "changed" is exactly "would read back differently"
//! — which is the only definition that cannot drift away from the reader.
//!
//! `document.xml` is one part *and* the whole document, so the stakes here are
//! higher than for a worksheet and so is the win: settings, rsids, content
//! controls, equations and every vendor extension ride through an edit three
//! paragraphs away.

pub mod blank;
mod drawing;
mod emit;
mod splice;

use std::fmt::Write as _;
use std::path::Path;

use ooxml::Package;
use quick_xml::events::Event;
use wp_model::doc::{Block, Document};
use wp_model::section::SectionProps;

use crate::ctx::{Ctx, HeaderIndex};
use crate::error::{Error, Result};
use crate::parts;
use crate::xml::local_name;
use splice::{escape_attr, Splicer};

/// Rewrites the document part of `package` from `document`, and saves it.
///
/// Beside the target first, then renamed over it — `ooxml::Package::save` does
/// that, and it is what stops a refusal or a crash halfway through from leaving
/// the user with neither the old document nor the new one.
pub fn save(document: &Document, package: &mut Package, path: impl AsRef<Path>) -> Result<()> {
    flush(document, package)?;
    package.save(path)?;
    Ok(())
}

/// Puts the rewritten document part back into the package without saving.
pub fn flush(document: &Document, package: &mut Package) -> Result<()> {
    let located = parts::locate(package)?;
    let part = package
        .part(&located.document)
        .ok_or_else(|| Error::MissingPart {
            referenced_by: "/_rels/.rels".to_owned(),
            rel_id: "officeDocument".to_owned(),
        })?;
    let content_type = part.content_type.clone();
    let rewritten = document_out(part.data(), document);
    package.put_part(located.document, &content_type, rewritten);
    Ok(())
}

/// One item of the model's walk, pairing positionally with a `<w:p>` or a
/// `<w:tbl>` met in the file.
enum Item<'a> {
    Paragraph(&'a wp_model::Paragraph),
    Table(&'a wp_model::table::Table),
}

/// The body's paragraphs and tables in document order — the model's side of the
/// pairing.
///
/// The walk descends into exactly the containers [`Document::paragraphs`]
/// descends into, so that a document whose tables are untouched pairs in the
/// same order it always did — with one difference of altitude: a table is **one
/// item**, not a hole its cell paragraphs fall through. The writer pairs a
/// `<w:tbl>` whole, so the paragraphs inside it are the table's business.
fn items(document: &Document) -> Vec<Item<'_>> {
    fn walk<'a>(blocks: &'a [Block], into: &mut Vec<Item<'a>>) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => into.push(Item::Paragraph(paragraph)),
                Block::Table(table) => into.push(Item::Table(table)),
                Block::Structured(sdt) => walk(&sdt.content, into),
                Block::Anchor(_) | Block::AltChunk { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&document.body, &mut out);
    out
}

/// Rewrites `document.xml`, splicing in only what changed.
///
/// Paragraphs and tables are paired **by document order, at any depth the
/// splicer walks**. A `<w:p>` inside a content control is a paragraph like any
/// other, and one of the corpus documents is nothing *but* a content control —
/// a writer that only walked the body's own children could not save an edit to
/// it at all. A `<w:tbl>` is consumed whole and paired as one unit against the
/// model's table at the same position: copied byte for byte when it reads back
/// equal, and re-emitted from the model — cells, scaffolding and all — when it
/// does not. Everything else — the `<w:sdt>` and its properties,
/// `<w:proofErr>`, `<mc:AlternateContent>` — passes through byte for byte, so
/// the structure around an edited paragraph is untouched.
pub fn document_out(original: &[u8], document: &Document) -> Vec<u8> {
    // The comparison reads paragraphs back out of the file, which interns style
    // ids. Every style the document names is already in the table, so nothing is
    // added — but the reader needs a mutable one, and the document's must not be
    // touched by a save.
    let mut styles = document.styles.clone();
    let mut headers = HeaderIndex::default();

    let model = items(document);
    let mut next = 0usize;

    let mut out = Vec::with_capacity(original.len());
    let mut splicer = Splicer::new(original);
    out.extend_from_slice(splicer.preamble());
    let mut in_body = false;

    while let Some((event, span)) = splicer.next() {
        match &event {
            Event::Start(e) if local_name(e) == b"body" => {
                in_body = true;
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::Start(e) if in_body && local_name(e) == b"p" => {
                let whole = splicer.element(b"p", span);
                let bytes = splicer.bytes(whole).to_vec();
                paragraph_out(
                    &mut out,
                    &bytes,
                    &model,
                    &mut next,
                    document,
                    &mut styles,
                    &mut headers,
                );
            }
            Event::Empty(e) if in_body && local_name(e) == b"p" => {
                let bytes = splicer.bytes(span).to_vec();
                paragraph_out(
                    &mut out,
                    &bytes,
                    &model,
                    &mut next,
                    document,
                    &mut styles,
                    &mut headers,
                );
            }
            // A table met anywhere the splicer walks — the body's own child, or
            // inside a content control — except inside another table, whose
            // whole element was already consumed below.
            Event::Start(e) if in_body && local_name(e) == b"tbl" => {
                let whole = splicer.element(b"tbl", span);
                let bytes = splicer.bytes(whole).to_vec();
                table_out(
                    &mut out,
                    &bytes,
                    &model,
                    &mut next,
                    document,
                    &mut styles,
                    &mut headers,
                );
            }
            // The body's final `<w:sectPr>` is its last child, so anything the
            // model still has to say goes in before it. The one inside a
            // `<w:pPr>` never reaches here — its paragraph was consumed whole.
            Event::Start(e) if in_body && local_name(e) == b"sectPr" => {
                append_rest(&mut out, &model, &mut next, document);
                let whole = splicer.element(b"sectPr", span);
                let mut section = Vec::new();
                section_bytes(&mut section, &document.section, splicer.bytes(whole));
                out.extend_from_slice(&section);
            }
            Event::End(e) if in_body && crate::xml::end_local_name(e) == b"body" => {
                append_rest(&mut out, &model, &mut next, document);
                in_body = false;
                out.extend_from_slice(splicer.bytes(span));
            }
            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }
    out
}

/// One `<w:p>` from the file, paired with the model's paragraph at the same
/// position.
#[allow(clippy::too_many_arguments)]
fn paragraph_out(
    out: &mut Vec<u8>,
    bytes: &[u8],
    model: &[Item<'_>],
    next: &mut usize,
    document: &Document,
    styles: &mut wp_model::StyleTable,
    headers: &mut HeaderIndex,
) {
    // A table the model holds before this paragraph is one the file has not got:
    // it was inserted, and goes in ahead of the paragraph it now precedes.
    while let Some(Item::Table(table)) = model.get(*next) {
        *next += 1;
        let mut text = String::new();
        emit::table(&mut text, table, &document.styles);
        out.extend_from_slice(text.as_bytes());
    }
    // No paragraph here means the model has fewer than the file: this one was
    // deleted, and its bytes go with it.
    if let Some(Item::Paragraph(paragraph)) = model.get(*next) {
        *next += 1;
        if same_paragraph(bytes, paragraph, styles, headers) {
            // Unchanged: the producer's own bytes, exactly.
            out.extend_from_slice(bytes);
        } else {
            let mut text = String::new();
            emit::paragraph(&mut text, paragraph, &document.styles);
            out.extend_from_slice(text.as_bytes());
        }
    }
}

/// One whole `<w:tbl>` from the file, paired with the model's table at the same
/// position.
#[allow(clippy::too_many_arguments)]
fn table_out(
    out: &mut Vec<u8>,
    bytes: &[u8],
    model: &[Item<'_>],
    next: &mut usize,
    document: &Document,
    styles: &mut wp_model::StyleTable,
    headers: &mut HeaderIndex,
) {
    // A file table pairs only against a model table. A paragraph at this
    // position, or nothing at all, means the table was deleted from the model —
    // its bytes go with it, and the pairing does not advance, because whatever
    // the model holds here still has a file counterpart ahead.
    if let Some(Item::Table(table)) = model.get(*next) {
        *next += 1;
        if same_table(bytes, table, styles, headers) {
            // Unchanged: the producer's own bytes, exactly — rsids, `tblPrEx`,
            // sdt-wrapped rows and everything else this crate does not model.
            out.extend_from_slice(bytes);
        } else {
            // Any change inside the table — one cell paragraph included —
            // rewrites the whole element from the model. That is the accepted
            // cost of pairing tables as units.
            let mut text = String::new();
            emit::table(&mut text, table, &document.styles);
            out.extend_from_slice(text.as_bytes());
        }
    }
}

/// Emits the paragraphs and tables the model has and the file does not.
fn append_rest(out: &mut Vec<u8>, model: &[Item<'_>], next: &mut usize, document: &Document) {
    while let Some(item) = model.get(*next) {
        *next += 1;
        let mut text = String::new();
        match item {
            Item::Paragraph(paragraph) => emit::paragraph(&mut text, paragraph, &document.styles),
            Item::Table(table) => emit::table(&mut text, table, &document.styles),
        }
        out.extend_from_slice(text.as_bytes());
    }
}

/// Whether the model's paragraph reads back the same as the file's bytes.
///
/// Comparing by re-reading rather than by remembering means the writer never
/// trusts what a reader believed some time earlier, and it makes "changed" mean
/// exactly "would read back differently" — the only definition that cannot drift
/// away from the reader.
fn same_paragraph(
    bytes: &[u8],
    paragraph: &wp_model::Paragraph,
    styles: &mut wp_model::StyleTable,
    headers: &mut HeaderIndex,
) -> bool {
    let mut ctx = Ctx::of_part(styles, headers, bytes);
    let mut reader = quick_xml::Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let (read, _) = crate::body::read_blocks_for_writer(&mut reader, &mut ctx, b"p");
    matches!(read.first(), Some(Block::Paragraph(read)) if read == paragraph)
}

/// Whether the model's table reads back the same as the file's bytes — the same
/// definition of "changed" as [`same_paragraph`], applied to the whole
/// `<w:tbl>` at once.
fn same_table(
    bytes: &[u8],
    table: &wp_model::table::Table,
    styles: &mut wp_model::StyleTable,
    headers: &mut HeaderIndex,
) -> bool {
    let mut ctx = Ctx::of_part(styles, headers, bytes);
    let mut reader = quick_xml::Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let (read, _) = crate::body::read_blocks_for_writer(&mut reader, &mut ctx, b"tbl");
    matches!(read.first(), Some(Block::Table(read)) if read == table)
}

/// The final `<w:sectPr>`.
///
/// Copied verbatim unless the model's section differs from what the file says.
/// `<w:sectPr>` holds printer paper-source codes, footnote placement, and an
/// `extLst`; re-emitting it from the model would drop all three for a document
/// whose page setup nobody touched.
fn section_bytes(out: &mut Vec<u8>, section: &SectionProps, original: &[u8]) {
    let mut styles = wp_model::StyleTable::new();
    let mut headers = HeaderIndex::default();
    let read = read_section(original, &mut styles, &mut headers);
    if read
        .as_ref()
        .is_some_and(|read| same_section(read, section))
    {
        out.extend_from_slice(original);
        return;
    }
    let mut text = String::new();
    self::section(&mut text, section);
    out.extend_from_slice(text.as_bytes());
}

/// Whether two sections say the same thing.
///
/// Not `==`: a [`HeaderId`] is handed out in the order references were read, and
/// the comparison re-reads one section on its own — so its ids start again from
/// zero and would never match. What identifies a header is the relationship it
/// names.
///
/// [`HeaderId`]: wp_model::HeaderId
fn same_section(a: &SectionProps, b: &SectionProps) -> bool {
    let refs_match = |left: &[wp_model::HeaderRef], right: &[wp_model::HeaderRef]| {
        left.len() == right.len() && left.iter().zip(right).all(|(left, right)| left.same(right))
    };
    refs_match(&a.headers, &b.headers)
        && refs_match(&a.footers, &b.footers)
        && SectionProps {
            headers: Vec::new(),
            footers: Vec::new(),
            ..a.clone()
        } == SectionProps {
            headers: Vec::new(),
            footers: Vec::new(),
            ..b.clone()
        }
}

fn read_section(
    bytes: &[u8],
    styles: &mut wp_model::StyleTable,
    headers: &mut HeaderIndex,
) -> Option<SectionProps> {
    let mut reader = quick_xml::Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut ctx = Ctx::of_part(styles, headers, bytes);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if local_name(&e) == b"sectPr" => {
                let owned = e.into_owned();
                return Some(crate::props::section_props(&mut reader, owned, &mut ctx));
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// Writes a `<w:sectPr>` from the model.
pub(crate) fn section(out: &mut String, section: &SectionProps) {
    out.push_str("<w:sectPr>");
    if section.start != wp_model::section::SectionStart::NextPage {
        let _ = write!(out, r#"<w:type w:val="{}"/>"#, section.start.name());
    }
    for (element, references) in [
        ("headerReference", &section.headers),
        ("footerReference", &section.footers),
    ] {
        for reference in references {
            // The relationship id, not the body index: the index is an artefact
            // of the order the document was read in, and writing it would point
            // the section at whatever relationship happened to have that number.
            let Some(rel) = &reference.rel else {
                continue;
            };
            let _ = write!(
                out,
                r#"<w:{element} w:type="{}" r:id="{}"/>"#,
                reference.kind.name(),
                escape_attr(rel)
            );
        }
    }
    let page = &section.page;
    let _ = write!(
        out,
        r#"<w:pgSz w:w="{}" w:h="{}""#,
        page.width.0, page.height.0
    );
    if page.orientation == wp_model::Orientation::Landscape {
        out.push_str(r#" w:orient="landscape""#);
    }
    if let Some(code) = page.code {
        let _ = write!(out, r#" w:code="{code}""#);
    }
    out.push_str("/>");
    let margins = &section.margins;
    let _ = write!(
        out,
        r#"<w:pgMar w:top="{}" w:right="{}" w:bottom="{}" w:left="{}" w:header="{}" w:footer="{}" w:gutter="{}"/>"#,
        margins.top.0,
        margins.end.0,
        margins.bottom.0,
        margins.start.0,
        margins.header.0,
        margins.footer.0,
        margins.gutter.0
    );
    if section.title_page {
        out.push_str("<w:titlePg/>");
    }
    let numbering = &section.page_numbering;
    if numbering.start.is_some() || numbering.format.is_some() {
        out.push_str("<w:pgNumType");
        if let Some(start) = numbering.start {
            let _ = write!(out, r#" w:start="{start}""#);
        }
        if let Some(format) = &numbering.format {
            let _ = write!(out, r#" w:fmt="{}""#, escape_attr(format));
        }
        out.push_str("/>");
    }
    let columns = &section.columns;
    let _ = write!(out, r#"<w:cols w:num="{}""#, columns.count());
    let _ = write!(out, r#" w:space="{}""#, columns.space.0);
    if !columns.equal_width {
        out.push_str(r#" w:equalWidth="0""#);
    }
    if columns.separator {
        out.push_str(r#" w:sep="1""#);
    }
    if columns.columns.is_empty() {
        out.push_str("/>");
    } else {
        out.push('>');
        for column in &columns.columns {
            let _ = write!(
                out,
                r#"<w:col w:w="{}" w:space="{}"/>"#,
                column.width.0, column.space.0
            );
        }
        out.push_str("</w:cols>");
    }
    if let Some(grid) = section.doc_grid {
        let _ = write!(out, r#"<w:docGrid w:linePitch="{}"/>"#, grid.line_pitch.0);
    }
    let _ = escape_attr("");
    out.push_str("</w:sectPr>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Inline, Paragraph, Run};
    use wp_model::table::{Cell, Row, RowHeight, RowProps, Table, TableLayout, VMerge, Width};
    use wp_model::units::Twips;

    /// A body with two paragraphs and the attributes Word decorates them with.
    const BODY: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="x" xmlns:w14="y"><w:body><w:p w14:paraId="760D8500" w:rsidR="002A5EF5" w:rsidRDefault="002A5EF5"><w:proofErr w:type="spellStart"/><w:r><w:t>first</w:t></w:r></w:p><w:p w14:paraId="4C057B28"><w:r><w:t>second</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720"/></w:sectPr></w:body></w:document>"#;

    /// A body with a table between two paragraphs, decorated the way Word
    /// decorates one — rsids on the row, ids on the cell paragraphs.
    const TABLE_BODY: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="x" xmlns:w14="y"><w:body><w:p w14:paraId="760D8500" w:rsidR="002A5EF5"><w:r><w:t>before</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="6240" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="3120"/><w:gridCol w:w="3120"/></w:tblGrid><w:tr w:rsidR="00B15C10"><w:tc><w:tcPr><w:tcW w:w="3120" w:type="dxa"/></w:tcPr><w:p w14:paraId="11111111"><w:r><w:t>alpha</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3120" w:type="dxa"/></w:tcPr><w:p w14:paraId="22222222"><w:r><w:t>beta</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p w14:paraId="4C057B28"><w:r><w:t>after</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720"/></w:sectPr></w:body></w:document>"#;

    fn read(bytes: &[u8]) -> Document {
        let package = crate::tests_support::package_with(bytes);
        crate::read(&package).expect("it reads")
    }

    #[test]
    fn a_save_with_no_edits_reproduces_the_bytes_exactly() {
        // The guarantee the whole design exists for. Not "equivalent XML" —
        // identical bytes, `w:rsidRDefault` and `<w:proofErr>` and all.
        let document = read(BODY);
        let out = document_out(BODY, &document);
        assert_eq!(String::from_utf8_lossy(&out), String::from_utf8_lossy(BODY));
    }

    #[test]
    fn an_edited_paragraph_is_rewritten_and_its_neighbour_is_not() {
        let mut document = read(BODY);
        let Block::Paragraph(paragraph) = &mut document.body[0] else {
            panic!("the first block is a paragraph");
        };
        paragraph.content = vec![Inline::Run(Run::of("changed"))];

        let out = document_out(BODY, &document);
        let text = String::from_utf8(out).expect("utf-8");
        assert!(text.contains("<w:t>changed</w:t>"), "{text}");
        assert!(!text.contains("<w:t>first</w:t>"), "{text}");
        // The paragraph nobody touched keeps every byte it had, `w14:paraId`
        // included, and so does the section.
        assert!(
            text.contains(r#"<w:p w14:paraId="4C057B28"><w:r><w:t>second</w:t></w:r></w:p>"#),
            "{text}"
        );
        assert!(text.contains(r#"<w:cols w:space="720"/>"#), "{text}");
        // And the rewritten one keeps its identity.
        assert!(text.contains(r#"w14:paraId="760D8500""#), "{text}");
    }

    #[test]
    fn a_paragraph_added_at_the_end_lands_before_the_section() {
        let mut document = read(BODY);
        document
            .body
            .push(Block::Paragraph(Paragraph::of("appended")));
        let text = String::from_utf8(document_out(BODY, &document)).expect("utf-8");
        let added = text.find("appended").expect("the new paragraph is there");
        let section = text.find("<w:sectPr>").expect("the section is still there");
        assert!(added < section, "a section is the body's last child");
    }

    #[test]
    fn a_deleted_paragraph_takes_its_bytes_with_it() {
        let mut document = read(BODY);
        document.body.remove(0);
        let text = String::from_utf8(document_out(BODY, &document)).expect("utf-8");
        assert!(!text.contains("<w:t>first</w:t>"), "{text}");
        assert!(text.contains("<w:t>second</w:t>"), "{text}");
        // And the survivor is still byte for byte what it was.
        assert!(
            text.contains(r#"<w:p w14:paraId="4C057B28"><w:r><w:t>second</w:t></w:r></w:p>"#),
            "{text}"
        );
    }

    #[test]
    fn an_edit_survives_being_read_back() {
        let mut document = read(BODY);
        let Block::Paragraph(paragraph) = &mut document.body[1] else {
            panic!("a paragraph");
        };
        paragraph.content = vec![Inline::Run(Run::of("rewritten"))];

        let out = document_out(BODY, &document);
        let reopened = read(&out);
        assert_eq!(reopened.body.len(), 2);
        assert_eq!(reopened.text(), "first\nrewritten");
    }

    #[test]
    fn a_changed_page_setup_is_written_and_an_unchanged_one_is_copied() {
        let document = read(BODY);
        let untouched = String::from_utf8(document_out(BODY, &document)).expect("utf-8");
        assert!(
            untouched.contains(r#"<w:pgMar w:top="1440" w:right="1440""#),
            "copied verbatim: {untouched}"
        );

        let mut turned = read(BODY);
        turned.section.page = turned.section.page.rotated();
        let text = String::from_utf8(document_out(BODY, &turned)).expect("utf-8");
        assert!(text.contains(r#"w:orient="landscape""#), "{text}");
        assert!(text.contains(r#"w:w="15840""#), "{text}");
    }

    fn cell_of(text: &str) -> Cell {
        Cell {
            content: vec![Block::Paragraph(Paragraph::of(text))],
            ..Cell::new()
        }
    }

    /// A table exercising the vocabulary the reader models: fixed widths, a
    /// span, a merge, borders, shading, a header row with a stated height.
    fn built_table(styles: &wp_model::StyleTable) -> Table {
        use wp_model::prop::{Border, BorderStyle, Shading, ShadingPattern};
        use wp_model::table::{CellMargins, TableBorders, TableLook};
        use wp_model::units::Eighth;

        let edge = Border {
            style: BorderStyle::Single,
            size: Some(Eighth(4)),
            space: Some(0),
            color: Some(wp_model::Color::Rgb([0, 0, 0])),
            shadow: false,
        };
        let mut table = Table::new();
        table.props.style = styles.lookup("TableGrid");
        table.props.width = Width::Fixed(Twips(9360));
        table.props.layout = TableLayout::Fixed;
        table.props.borders = TableBorders {
            top: Some(edge),
            start: Some(edge),
            bottom: Some(edge),
            end: Some(edge),
            inside_h: Some(edge),
            inside_v: Some(edge),
        };
        table.props.look = TableLook {
            first_row: true,
            no_v_band: true,
            ..TableLook::default()
        };
        table.props.cell_margins = CellMargins::word_default();
        table.grid = vec![Twips(3120), Twips(3120), Twips(3120)];

        let mut wide = cell_of("head");
        wide.props.grid_span = 2;
        wide.props.width = Width::Fixed(Twips(6240));
        wide.props.shading = Some(Shading {
            pattern: ShadingPattern::Clear,
            fill: Some(wp_model::Color::Rgb([0xD9, 0xD9, 0xD9])),
            color: None,
        });
        let mut merged = cell_of("tall");
        merged.props.v_merge = Some(VMerge::Restart);
        merged.props.width = Width::Fixed(Twips(3120));
        table.rows.push(Row {
            props: RowProps {
                header: true,
                height: Some(RowHeight::AtLeast(Twips(400))),
                ..RowProps::default()
            },
            cells: vec![wide, merged],
        });

        let mut continued = cell_of("");
        continued.props.v_merge = Some(VMerge::Continue);
        continued.props.width = Width::Fixed(Twips(3120));
        table.rows.push(Row {
            props: RowProps::default(),
            cells: vec![cell_of("a"), cell_of("b"), continued],
        });
        table
    }

    #[test]
    fn an_emitted_table_reads_back_equal() {
        // The contract `same_table` stands on: "changed" means "would read back
        // differently", so what `emit::table` writes has to read back as
        // exactly the table it was given.
        let mut styles = wp_model::StyleTable::new();
        styles.insert(wp_model::style::Style::new(
            "TableGrid",
            wp_model::style::StyleKind::Table,
        ));
        let table = built_table(&styles);

        let mut xml = String::new();
        emit::table(&mut xml, &table, &styles);
        let bytes = xml.as_bytes();
        let mut headers = HeaderIndex::default();
        let mut ctx = Ctx::of_part(&mut styles, &mut headers, bytes);
        let mut reader = quick_xml::Reader::from_reader(bytes);
        reader.config_mut().trim_text(false);
        let (read, _) = crate::body::read_blocks_for_writer(&mut reader, &mut ctx, b"tbl");
        let Some(Block::Table(read)) = read.into_iter().next() else {
            panic!("a table reads back from {xml}");
        };
        assert_eq!(read, table, "{xml}");
    }

    #[test]
    fn a_save_with_no_table_edits_reproduces_the_bytes_exactly() {
        // The same invariant as the paragraph one, with a table in the middle:
        // the row's rsid and the producer's whole `<w:tbl>` survive untouched.
        let document = read(TABLE_BODY);
        let out = document_out(TABLE_BODY, &document);
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(TABLE_BODY)
        );
    }

    #[test]
    fn an_inserted_table_lands_between_the_paragraphs_it_was_put_between() {
        let mut document = read(BODY);
        let mut table = Table::new();
        table.grid = vec![Twips(4680), Twips(4680)];
        table.rows.push(Row {
            props: RowProps::default(),
            cells: vec![cell_of("left"), cell_of("right")],
        });
        document.body.insert(1, Block::Table(table));

        let out = document_out(BODY, &document);
        let text = String::from_utf8(out).expect("utf-8");
        // Both neighbours keep every byte they had, decorations included.
        assert!(
            text.contains(r#"<w:p w14:paraId="760D8500" w:rsidR="002A5EF5" w:rsidRDefault="002A5EF5"><w:proofErr w:type="spellStart"/><w:r><w:t>first</w:t></w:r></w:p>"#),
            "{text}"
        );
        assert!(
            text.contains(r#"<w:p w14:paraId="4C057B28"><w:r><w:t>second</w:t></w:r></w:p>"#),
            "{text}"
        );
        let first = text.find("<w:t>first</w:t>").expect("first");
        let tbl = text.find("<w:tbl>").expect("the new table");
        let second = text.find("<w:t>second</w:t>").expect("second");
        assert!(first < tbl && tbl < second, "{text}");

        let reopened = read(text.as_bytes());
        assert_eq!(reopened.body.len(), 3);
        let Block::Table(table) = &reopened.body[1] else {
            panic!("the middle block is the table");
        };
        assert_eq!(table.text(), "left\tright\n");
    }

    #[test]
    fn an_edited_cell_rewrites_the_whole_table_and_only_the_table() {
        let mut document = read(TABLE_BODY);
        let Block::Table(table) = &mut document.body[1] else {
            panic!("the middle block is a table");
        };
        let Block::Paragraph(paragraph) = &mut table.rows[0].cells[1].content[0] else {
            panic!("a paragraph in the cell");
        };
        paragraph.content = vec![Inline::Run(Run::of("gamma"))];

        let out = document_out(TABLE_BODY, &document);
        let text = String::from_utf8(out).expect("utf-8");
        assert!(text.contains("<w:t>gamma</w:t>"), "{text}");
        assert!(!text.contains("<w:t>beta</w:t>"), "{text}");
        assert_eq!(text.matches("<w:tbl>").count(), 1, "{text}");
        // The whole table is re-emitted, which drops the row's unmodelled rsid
        // — the accepted cost of pairing tables as units. The paragraphs
        // outside it keep every byte.
        assert!(
            text.contains(r#"<w:p w14:paraId="760D8500" w:rsidR="002A5EF5"><w:r><w:t>before</w:t></w:r></w:p>"#),
            "{text}"
        );
        assert!(
            text.contains(r#"<w:p w14:paraId="4C057B28"><w:r><w:t>after</w:t></w:r></w:p>"#),
            "{text}"
        );

        let reopened = read(text.as_bytes());
        assert_eq!(reopened.body, document.body, "{text}");
    }

    #[test]
    fn a_deleted_table_takes_its_bytes_with_it() {
        let mut document = read(TABLE_BODY);
        assert!(matches!(document.body[1], Block::Table(_)));
        document.body.remove(1);

        let out = document_out(TABLE_BODY, &document);
        let text = String::from_utf8(out).expect("utf-8");
        assert!(!text.contains("<w:tbl>"), "{text}");
        assert!(!text.contains("alpha"), "{text}");
        // And the neighbours are still byte for byte what they were.
        assert!(
            text.contains(r#"<w:p w14:paraId="760D8500" w:rsidR="002A5EF5"><w:r><w:t>before</w:t></w:r></w:p>"#),
            "{text}"
        );
        assert!(
            text.contains(r#"<w:p w14:paraId="4C057B28"><w:r><w:t>after</w:t></w:r></w:p>"#),
            "{text}"
        );
    }

    #[test]
    fn a_table_appended_at_the_end_lands_before_the_section() {
        let mut document = read(BODY);
        let mut table = Table::new();
        table.rows.push(Row {
            props: RowProps::default(),
            cells: vec![cell_of("appended")],
        });
        document.body.push(Block::Table(table));

        let text = String::from_utf8(document_out(BODY, &document)).expect("utf-8");
        let added = text.find("<w:tbl>").expect("the new table is there");
        let section = text.find("<w:sectPr>").expect("the section is still there");
        assert!(added < section, "a section is the body's last child");

        let reopened = read(text.as_bytes());
        let Some(Block::Table(table)) = reopened.body.last() else {
            panic!("the table is the last block");
        };
        assert_eq!(table.text(), "appended\n");
    }
}
