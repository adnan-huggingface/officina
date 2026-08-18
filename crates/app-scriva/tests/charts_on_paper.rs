//! A document's chart, all the way to the page a renderer draws.
//!
//! The screen grew charts first, and print kept skipping them for a while
//! afterwards — an `Op::Image` needs bytes and a chart has none, so the box
//! came out empty. This walks the real corpus document that carries one, from
//! the package to the drawing operations, and insists there is ink in the box.

use std::collections::HashMap;

use ui_kit::egui;

fn corpus(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../corpus/docx/{name}"))
}

#[test]
fn the_chart_a_document_carries_is_drawn_on_its_page() {
    let ctx = egui::Context::default();
    // The machine's own type, not just the family names: a chart sets its
    // labels in Calibri, which is an exactly-named face, and a context given
    // no directories to read has none of those — every name falls back to the
    // generic sans, and the page this test leaves behind is set in the wrong
    // face while every assertion in it still passes.
    ui_kit::fonts::install(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let mut shaper = scriva::shaper::Egui::new(&ctx);

    let path = corpus("file-sample_500kB.docx");
    let (document, package) = wp_docx::open(&path).expect("the corpus document opens");
    let parts = wp_docx::DocumentParts::locate_in(&package).expect("its parts");
    let mut view = scriva::view::View::default();
    view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);

    // The drawing on page one is a clustered bar chart, not a picture: the
    // page names a chart part and no image bytes at all.
    let rels = wp_print::ops::chart_rels(view.pages().iter());
    assert_eq!(rels.len(), 1, "one chart in the document: {rels:?}");
    let plots = scriva::publish::plots(Some(&package), Some(&parts), view.pages());
    let plot = plots.get(&rels[0]).expect("the chart part parses");
    assert!(!plot.series.is_empty(), "a chart with numbers in it");

    let charted = view
        .pages()
        .iter()
        .find(|page| {
            wp_print::ops::flatten(page)
                .iter()
                .any(|op| matches!(op, wp_print::ops::Op::Chart { .. }))
        })
        .expect("a page holding the chart");

    let ops = wp_print::ops::draw_charts(
        wp_print::ops::flatten(charted),
        &mut wp_print::ops::Charts {
            plots: &plots,
            shaper: &mut shaper,
        },
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, wp_print::ops::Op::Chart { .. })),
        "no chart is left for a backend to skip"
    );
    // One filled rectangle per point per series, in that series' own colour.
    let series = chart::draw::cached_series(plot);
    let bars = ops
        .iter()
        .filter(|op| {
            matches!(op, wp_print::ops::Op::Fill { rgb, .. }
                if series.iter().any(|s| s.rgb == *rgb))
        })
        .count();
    let points: usize = series.iter().map(|s| s.values.len()).sum();
    let swatches = if plot.legend.is_some() {
        series.len()
    } else {
        0
    };
    assert_eq!(
        bars,
        points + swatches,
        "a bar for every point, and a legend swatch for every series"
    );

    // And the whole page still exports — the ops a chart adds are ones both
    // backends already knew.
    let images = scriva::publish::rasters(Some(&package), Some(&parts), view.pages());
    let mut faces = scriva::publish::SystemFaces::new();
    let pdf = wp_print::pdf::export(
        std::slice::from_ref(charted),
        &mut faces,
        &images,
        Some(&mut wp_print::ops::Charts {
            plots: &plots,
            shaper: &mut shaper,
        }),
        Some("chart"),
    );
    assert!(pdf.starts_with(b"%PDF-1.5") && pdf.ends_with(b"%%EOF\n"));

    let bare = wp_print::pdf::export(
        std::slice::from_ref(charted),
        &mut faces,
        &images,
        None,
        Some("chart"),
    );
    assert!(
        pdf.len() > bare.len(),
        "the drawn chart is bytes the skipped one was not: {} vs {}",
        pdf.len(),
        bare.len()
    );

    // Left behind for a human to look at, which is the only way to see that
    // the picture is the screen's picture.
    let _ = std::fs::write(std::env::temp_dir().join("scriva-chart.pdf"), &pdf);
    let _: HashMap<String, wp_print::Raster> = images;
}
