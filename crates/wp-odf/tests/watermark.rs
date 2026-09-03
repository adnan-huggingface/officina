//! The watermark of a real document, read out of the file Word exported.
//!
//! A unit test over hand-written XML says the reader understood the element it
//! was shown. This says the element in the corpus is the element it was shown —
//! which is the only version of the question worth asking, because a watermark
//! is four nested things and the export writes it in a header of `styles.xml`
//! rather than anywhere a body reader would look.

use std::path::{Path, PathBuf};

use wp_model::doc::{Block, Drawing, Inline, Piece};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/odt")
        .canonicalize()
        .expect("the corpus is in the repository")
}

/// Every drawing in every band of a document, headers and footers alike.
fn drawings_in_bands(document: &wp_model::Document) -> Vec<&Drawing> {
    let mut out = Vec::new();
    for band in &document.headers {
        for block in &band.content {
            if let Block::Paragraph(paragraph) = block {
                for inline in &paragraph.content {
                    if let Inline::Run(run) = inline {
                        for piece in &run.content {
                            if let Piece::Drawing(drawing) = piece {
                                out.push(&**drawing);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// The document is a rubbing of a real one, so its watermark reads as nonsense
/// — but it is a `<draw:custom-shape>` on a text path in a header, exactly as
/// Word's ODF export writes one, and that is what is being asserted.
#[test]
fn the_custom_shape_in_a_real_export_is_read_as_the_watermark_it_draws() {
    let (document, _, _) =
        wp_odf::open(corpus().join("word-odf-export.odt")).expect("the package opens");
    let shapes: Vec<_> = drawings_in_bands(&document)
        .into_iter()
        .filter(|drawing| drawing.text.is_some())
        .collect();
    assert_eq!(shapes.len(), 1, "one watermark, in one band");
    let shape = shapes[0];
    let words = shape.text.as_ref().expect("it carries words");
    assert!(!words.text.trim().is_empty(), "and the words are not empty");
    assert!(shape.behind_text, "a watermark is under the body");
    assert!(shape.anchored, "and floats rather than sitting in the line");
    assert!(
        shape.extent.0.points() > 400.0,
        "across most of the page: {:?}",
        shape.extent
    );
    assert!(
        !shape.source.is_empty(),
        "kept as the bytes it arrived as, so a save does not restate it"
    );
}
