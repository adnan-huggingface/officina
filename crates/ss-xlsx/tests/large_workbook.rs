//! The C4 exit criterion: a large workbook opens fast enough to feel instant.
//!
//! Ignored by default — it synthesizes tens of megabytes of XML and is meaningless
//! in a debug build, where the parser runs an order of magnitude slower than what
//! ships. Run it the way it is meant to be run:
//!
//! ```text
//! cargo test --release -p ss-xlsx -- --ignored --nocapture
//! ```
//!
//! The sheet is synthesized rather than taken from `corpus/` on purpose. Parse
//! cost is dominated by the number of `<c>` elements, not by who produced them,
//! and a performance check that silently passes because the corpus happens to be
//! empty would be worse than none. Fidelity against real Excel output is the
//! separate job of `cargo xtask fidelity`.

use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use ss_xlsx::XlsxDocument;

/// Target size of the generated sheet XML.
///
/// "A 50 MB Excel file" is ambiguous and the two readings are far apart. An xlsx
/// is deflated at roughly 10:1, so 50 MB *on disk* is around 500 MB of XML —
/// which even quick-xml's raw event scan cannot walk in 2 seconds on this
/// hardware, let alone build a model from (see `xml_scan_floor`). This test
/// takes the reachable reading: 50 MB of sheet XML, about 5 MB on disk and
/// upwards of a million cells, which is already a workbook far beyond what most
/// people ever open. The harsher reading is recorded in `PROGRESS.md` as not met.
const TARGET_SHEET_BYTES: usize = 50 * 1024 * 1024;

const COLS: usize = 10;

/// The budget from `PROGRESS.md` C4.
const BUDGET: Duration = Duration::from_secs(2);

#[test]
#[ignore = "performance check; run with --release and --ignored"]
fn a_fifty_megabyte_workbook_opens_within_the_budget() {
    let rows = rows_for_target();
    let sheet_xml = synth_sheet(rows, COLS);
    let uncompressed = sheet_xml.len();

    let package = build_package(&sheet_xml);
    let compressed = package.len();

    // Read from memory so the number measures the parser rather than the disk.
    let started = Instant::now();
    let doc = XlsxDocument::read(Cursor::new(package)).expect("large workbook opens");
    let elapsed = started.elapsed();

    let sheet = &doc.workbook.sheets[0];
    let cells = sheet.cells.len();

    println!();
    println!("  sheet XML     {:>9.1} MB", mb(uncompressed));
    println!("  package       {:>9.1} MB", mb(compressed));
    println!("  cells         {cells:>9}");
    println!("  formulas      {:>9}", sheet.formulas.len());
    println!("  shared strings{:>9}", doc.workbook.strings.len());
    println!("  parse         {:>9.3} s", elapsed.as_secs_f64());
    println!(
        "  throughput    {:>9.1} MB/s   ({:.1}M cells/s)",
        mb(uncompressed) / elapsed.as_secs_f64(),
        cells as f64 / elapsed.as_secs_f64() / 1e6
    );
    println!();

    // Correctness alongside speed: a parser that dropped most of the sheet would
    // otherwise look like a very fast one.
    assert_eq!(
        cells,
        rows * COLS,
        "every generated cell should have been read"
    );

    assert!(
        elapsed < BUDGET,
        "opening {:.1} MB of sheet XML took {:.3}s, over the {:.0}s budget",
        mb(uncompressed),
        elapsed.as_secs_f64(),
        BUDGET.as_secs_f64()
    );
}

/// Where the time goes when a real workbook is opened, edited and saved.
///
/// Point it at a file: `CALX_BIG=path cargo test --release -p ss-xlsx --
/// --ignored --nocapture save_a_real_workbook`. It is skipped with a note when
/// the variable is unset, because the file it is meant for is a user's own
/// workbook and not something the repository carries.
#[test]
#[ignore = "diagnostic; needs CALX_BIG and --release"]
fn save_a_real_workbook() {
    let Ok(path) = std::env::var("CALX_BIG") else {
        println!("CALX_BIG is not set; nothing to measure");
        return;
    };
    let bytes = std::fs::read(&path).expect("the workbook is readable");
    println!("\n  file          {:>9.1} MB", mb(bytes.len()));

    let started = Instant::now();
    let mut doc = XlsxDocument::read(Cursor::new(bytes)).expect("opens");
    println!("  read          {:>9.3} s", started.elapsed().as_secs_f64());

    let cells: usize = doc.workbook.sheets.iter().map(|s| s.cells.len()).sum();
    println!("  cells         {cells:>9}");

    // One edit, the way a user makes one: a word typed into the first cell.
    let text = doc.workbook.strings.intern("CALX-EDITED");
    doc.workbook.sheet_mut(0).expect("a sheet").set(
        ss_model::CellRef::new(0, 0),
        ss_model::Cell {
            value: ss_model::CellValue::Text(text),
            ..Default::default()
        },
    );

    let started = Instant::now();
    doc.flush().expect("flushes");
    println!("  flush         {:>9.3} s", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let mut out = Cursor::new(Vec::new());
    doc.write_to(&mut out).expect("writes");
    println!(
        "  flush + zip   {:>9.3} s   ({:.1} MB out)\n",
        started.elapsed().as_secs_f64(),
        mb(out.into_inner().len())
    );
}

/// What the XML scan alone costs, with no model built.
///
/// This is the floor: no amount of tuning in `sheet.rs` can beat walking the
/// events. Kept as a permanent diagnostic so a future slowdown can be attributed
/// to our code or to the parser underneath it without guessing.
#[test]
#[ignore = "diagnostic; run with --release and --ignored"]
fn xml_scan_floor() {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let sheet_xml = synth_sheet(rows_for_target(), COLS);
    let started = Instant::now();

    let mut reader = Reader::from_reader(sheet_xml.as_bytes());
    let mut buf = Vec::new();
    let mut events = 0u64;
    loop {
        match reader.read_event_into(&mut buf).expect("scans") {
            Event::Eof => break,
            _ => events += 1,
        }
        buf.clear();
    }
    let elapsed = started.elapsed();

    println!();
    println!("  events        {events:>9}");
    println!("  scan          {:>9.3} s", elapsed.as_secs_f64());
    println!(
        "  ceiling       {:>9.1} MB/s",
        mb(sheet_xml.len()) / elapsed.as_secs_f64()
    );
    println!();
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Row count that lands the generated sheet on [`TARGET_SHEET_BYTES`].
///
/// Measured from a small sample rather than hardcoded, so the fixture stays the
/// stated size if the generated markup ever changes shape.
fn rows_for_target() -> usize {
    const SAMPLE: usize = 1_000;
    let per_row = synth_sheet(SAMPLE, COLS).len() as f64 / SAMPLE as f64;
    (TARGET_SHEET_BYTES as f64 / per_row).round() as usize
}

/// Builds a sheet with the mix of cell kinds a real workbook has: numbers,
/// shared strings, an inline string, and a shared-formula group per row.
fn synth_sheet(rows: usize, cols: usize) -> String {
    let mut xml = String::with_capacity(rows * cols * 34 + 4096);
    xml.push_str(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );

    const NAMES: [&str; COLS] = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"];
    assert!(cols <= NAMES.len());

    for r in 1..=rows {
        xml.push_str("<row r=\"");
        push_usize(&mut xml, r);
        xml.push_str("\">");
        for (c, name) in NAMES.iter().enumerate().take(cols) {
            xml.push_str("<c r=\"");
            xml.push_str(name);
            push_usize(&mut xml, r);
            xml.push('"');
            match c % 5 {
                // A shared string, the commonest cell in a real export.
                0 => {
                    xml.push_str(" t=\"s\"><v>");
                    push_usize(&mut xml, r % 4);
                    xml.push_str("</v></c>");
                }
                // A styled number.
                1 => {
                    xml.push_str(" s=\"3\"><v>");
                    push_usize(&mut xml, r * 7 % 100_000);
                    xml.push_str(".25</v></c>");
                }
                // A shared-formula master on the first row, followers after it.
                2 => {
                    if r == 1 {
                        xml.push_str("><f t=\"shared\" ref=\"C1:C");
                        push_usize(&mut xml, rows);
                        xml.push_str("\" si=\"0\">B1*2</f><v>0</v></c>");
                    } else {
                        xml.push_str("><f t=\"shared\" si=\"0\"/><v>");
                        push_usize(&mut xml, r);
                        xml.push_str("</v></c>");
                    }
                }
                3 => {
                    xml.push_str("><v>");
                    push_usize(&mut xml, r);
                    xml.push_str("</v></c>");
                }
                _ => {
                    xml.push_str(" t=\"inlineStr\"><is><t>row ");
                    push_usize(&mut xml, r);
                    xml.push_str("</t></is></c>");
                }
            }
        }
        xml.push_str("</row>");
    }

    xml.push_str("</sheetData></worksheet>");
    xml
}

/// Appends an integer without going through `format!`.
///
/// `format!` here would allocate once per cell — several million allocations
/// spent building the fixture rather than measuring the parser.
fn push_usize(out: &mut String, mut n: usize) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    out.push_str(std::str::from_utf8(&digits[i..]).expect("ASCII digits"));
}

fn build_package(sheet_xml: &str) -> Vec<u8> {
    const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
</Types>"#;

    const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

    const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>"#;

    const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Big" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;

    const SHARED_STRINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">
  <si><t>Active</t></si><si><t>Pending</t></si><si><t>Closed</t></si><si><t>Escalated</t></si>
</sst>"#;

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for (name, body) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
        ("xl/sharedStrings.xml", SHARED_STRINGS),
        ("xl/worksheets/sheet1.xml", sheet_xml),
    ] {
        zip.start_file(name, opts).expect("zip entry starts");
        zip.write_all(body.as_bytes()).expect("zip entry writes");
    }
    zip.finish().expect("zip finishes").into_inner()
}
