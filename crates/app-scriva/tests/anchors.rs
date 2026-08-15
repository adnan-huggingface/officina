//! Where did this line land? — the question every fidelity round asks.
//!
//! Word answers it to a hundredth of a point through COM
//! (`Range.Information(6)`); this prints Scriva's answer for the same
//! document, so the two can be laid side by side. Ignored in the ordinary
//! run because it reads a document named by the environment:
//!
//! ```text
//! SCRIVA_ANCHORS_DOC=C:\path\doc.docx SCRIVA_ANCHORS="one;two" \
//!   cargo test -p scriva --test anchors -- --ignored --nocapture
//! ```

use ui_kit::egui;

#[test]
#[ignore = "reads the document named by SCRIVA_ANCHORS_DOC; run by hand"]
fn where_each_anchor_landed() {
    let path = std::env::var("SCRIVA_ANCHORS_DOC").expect("SCRIVA_ANCHORS_DOC names a document");
    let anchors = std::env::var("SCRIVA_ANCHORS").expect("SCRIVA_ANCHORS is semicolon-separated");
    let anchors: Vec<&str> = anchors.split(';').filter(|a| !a.is_empty()).collect();

    let ctx = egui::Context::default();
    // The machine's real faces: an anchor's position is a statement about
    // metrics, and the arithmetic shaper would answer for a different font.
    ui_kit::fonts::install(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let mut shaper = scriva::shaper::Egui::new(&ctx);

    let (document, _package) = wp_docx::open(&path).expect("the document opens");
    let mut view = scriva::view::View::default();
    view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);

    for anchor in anchors {
        let mut found = false;
        'pages: for page in view.pages() {
            for placement in page.everything() {
                let wp_layout::block::Placed::Line { line, .. } = &placement.kind else {
                    continue;
                };
                let text: String = line
                    .fragments
                    .iter()
                    .filter_map(|fragment| match &fragment.content {
                        wp_layout::Content::Text { text, .. }
                        | wp_layout::Content::Label { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                if text.contains(anchor) {
                    println!(
                        "{anchor:<28} page {}  top={:.2}  baseline={:.2}",
                        page.number,
                        placement.y,
                        placement.y + line.baseline
                    );
                    found = true;
                    break 'pages;
                }
            }
        }
        if !found {
            println!("{anchor:<28} NOT FOUND on one line (wrapped or absent)");
        }
    }
}
