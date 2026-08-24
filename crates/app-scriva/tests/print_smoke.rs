//! The GDI print path, held in the hand.
//!
//! Ignored in the ordinary run: it drives the real *Microsoft Print to PDF*
//! driver, which means a spooler, a driver and a file appearing on its own
//! schedule — none of which belongs in a unit-test gate. Run it deliberately:
//!
//! ```text
//! cargo test -p scriva --test print_smoke -- --ignored
//! ```

#[cfg(windows)]
#[test]
#[ignore = "spools through the Microsoft Print to PDF driver; run by hand"]
fn a_corpus_document_prints_through_the_pdf_driver() {
    use ui_kit::egui;

    let ctx = egui::Context::default();
    ui_kit::fonts::register(&ctx, &[]);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let mut shaper = scriva::shaper::Egui::new(&ctx);

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/docx/table-spanning-pages.docx");
    let (document, package) = wp_docx::open(&path).expect("the corpus document opens");
    let parts = wp_docx::DocumentParts::locate_in(&package).expect("its parts");

    let mut view = scriva::view::View::default();
    view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
    assert!(view.pages().len() >= 2, "a document that spans pages");

    let images = scriva::publish::rasters(
        Some(&package),
        Some(&parts),
        &Default::default(),
        view.pages(),
    );
    let plots = scriva::publish::plots(Some(&package), Some(&parts), view.pages());
    let output = std::env::temp_dir().join("scriva-print-smoke.pdf");
    let _ = std::fs::remove_file(&output);

    wp_print::win::print_to_file(
        "Microsoft Print to PDF",
        &output,
        view.pages(),
        &images,
        Some(&mut wp_print::ops::Charts {
            plots: &plots,
            shaper: &mut shaper,
        }),
        "scriva print smoke",
        &ui_kit::fonts::gdi_family,
    )
    .expect("the driver accepts the job");

    // The spooler writes the file on its own time, shortly after EndDoc.
    let mut bytes = Vec::new();
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(read) = std::fs::read(&output) {
            if read.starts_with(b"%PDF") {
                bytes = read;
                break;
            }
        }
    }
    assert!(
        bytes.starts_with(b"%PDF"),
        "the driver produced a PDF at {}",
        output.display()
    );
    assert!(bytes.len() > 10_000, "with pages in it, not a stub");
}
