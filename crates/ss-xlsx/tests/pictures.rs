//! A picture anchored to a sheet, read and preserved.
//!
//! The workbook this was written for opens on a form whose heading — the
//! company's name, in the company's own type — is not a cell at all. It is a
//! PNG anchored over four rows of empty ones. A reader that follows only the
//! chart branch of the drawing shows that sheet with a blank space at the top,
//! which reads as a fault in the file rather than in the reader.
//!
//! Built in memory rather than taken from the corpus: the corpus is generated
//! through Excel's own object model and none of its files carry an image, and
//! the thing under test is three parts and two relationship hops, which is
//! exactly what a fixture can express.

use std::io::{Cursor, Write};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
</Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="REVISION" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;

const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

const SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData><row r="6"><c r="A6" t="inlineStr"><is><t>DATE</t></is></c></row></sheetData>
  <drawing r:id="rId1"/>
</worksheet>"#;

const SHEET_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/>
</Relationships>"#;

/// Anchored the way the real file anchors its masthead: corner to corner, in
/// column D, from partway down row 1 to partway into row 4.
const DRAWING: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
          xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <xdr:twoCellAnchor editAs="oneCell">
    <xdr:from><xdr:col>3</xdr:col><xdr:colOff>110612</xdr:colOff>
              <xdr:row>0</xdr:row><xdr:rowOff>159775</xdr:rowOff></xdr:from>
    <xdr:to><xdr:col>3</xdr:col><xdr:colOff>1926712</xdr:colOff>
            <xdr:row>3</xdr:row><xdr:rowOff>75381</xdr:rowOff></xdr:to>
    <xdr:pic>
      <xdr:nvPicPr><xdr:cNvPr id="4" name="Picture 3"/><xdr:cNvPicPr/></xdr:nvPicPr>
      <xdr:blipFill><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill>
    </xdr:pic>
    <xdr:clientData/>
  </xdr:twoCellAnchor>
</xdr:wsDr>"#;

const DRAWING_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#;

/// A 1x1 opaque red PNG. What it depicts does not matter; that the reader
/// hands back these exact bytes does.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn build_package() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for (name, body) in [
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", WORKBOOK.as_bytes()),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes()),
        ("xl/worksheets/sheet1.xml", SHEET.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", SHEET_RELS.as_bytes()),
        ("xl/drawings/drawing1.xml", DRAWING.as_bytes()),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            DRAWING_RELS.as_bytes(),
        ),
        ("xl/media/image1.png", PNG),
    ] {
        zip.start_file(name, opts).expect("zip entry starts");
        zip.write_all(body).expect("zip entry writes");
    }
    zip.finish().expect("zip finishes").into_inner()
}

#[test]
fn a_picture_anchored_to_a_sheet_is_read_with_its_bytes() {
    let doc = ss_xlsx::XlsxDocument::read(Cursor::new(build_package())).expect("opens");
    let sheet = &doc.workbook.sheets[0];

    assert_eq!(sheet.pictures.len(), 1, "the masthead");
    let picture = &sheet.pictures[0];
    assert_eq!(picture.part, "/xl/media/image1.png");
    assert_eq!(picture.name, "Picture 3");
    assert_eq!(picture.content_type, "image/png");
    assert_eq!(&*picture.data, PNG, "the file's own bytes, not a re-encode");
}

#[test]
fn the_anchor_keeps_the_offset_that_stops_it_snapping_to_a_column() {
    let doc = ss_xlsx::XlsxDocument::read(Cursor::new(build_package())).expect("opens");
    match &doc.workbook.sheets[0].pictures[0].anchor {
        ss_model::Anchor::TwoCell { from, to } => {
            assert_eq!((from.col, from.row), (3, 0));
            assert_eq!(from.col_offset, 110_612);
            assert_eq!(from.row_offset, 159_775);
            assert_eq!((to.col, to.row), (3, 3));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_picture_is_not_mistaken_for_a_chart() {
    let doc = ss_xlsx::XlsxDocument::read(Cursor::new(build_package())).expect("opens");
    assert!(
        doc.workbook.sheets[0].charts.is_empty(),
        "an image part is not a chartSpace, and drawing a placeholder chart \
         over the heading would be worse than drawing nothing"
    );
}

#[test]
fn saving_puts_the_image_part_back_untouched() {
    let mut doc = ss_xlsx::XlsxDocument::read(Cursor::new(build_package())).expect("opens");
    let change = ss_formula::edit::input(
        &mut doc.workbook,
        0,
        ss_model::CellRef::from_a1("A6").expect("A6"),
        "EDITED",
    );
    ss_formula::edit::apply(&mut doc.workbook, change);

    let mut bytes = Vec::new();
    doc.write_to(Cursor::new(&mut bytes)).expect("writes");
    let reopened = ss_xlsx::XlsxDocument::read(Cursor::new(bytes)).expect("reads back");

    let picture = &reopened.workbook.sheets[0].pictures[0];
    assert_eq!(&*picture.data, PNG);
    assert_eq!(picture.name, "Picture 3");
}
