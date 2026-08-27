//! A throwaway measuring stick: lays a document out exactly as the window
//! would and writes what it drew, so a rendering can be compared against
//! Word's own without a screen in the way. Not part of the gate.
//!
//! ```text
//! PROBE_DOCX=... PROBE_OUT=... PROBE_PDF=... cargo test -p scriva --test zz_probe -- --ignored --nocapture
//! ```

#[test]
#[ignore = "a measuring tool, driven by environment variables"]
fn probe() {
    use std::fmt::Write as _;
    use ui_kit::egui;

    let legacy = std::env::var("PROBE_DOC").ok();
    let Some(source) = legacy.clone().or_else(|| std::env::var("PROBE_DOCX").ok()) else {
        return;
    };

    let ctx = egui::Context::default();
    ui_kit::fonts::install(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let mut shaper = scriva::shaper::Egui::new(&ctx);

    let path = std::path::PathBuf::from(&source);
    let mut loose = std::collections::HashMap::new();
    let (document, package) = match &legacy {
        Some(_) => {
            let (document, media) = wp_doc::open(&path).expect("the document opens");
            for picture in media {
                loose.insert(picture.rel, picture.data);
            }
            (document, None)
        }
        None => {
            let (document, package) = wp_docx::open(&path).expect("the document opens");
            (document, Some(package))
        }
    };
    let parts = package
        .as_ref()
        .map(|package| wp_docx::DocumentParts::locate_in(package).expect("its parts"));

    // The same order the window uses: the document's own faces are registered
    // before anything is measured, and the shaper is built after them.
    let embedded: Vec<(String, bool, bool, Vec<u8>)> = package
        .as_ref()
        .zip(parts.as_ref())
        .map(|(package, parts)| wp_docx::embedded(package, parts))
        .unwrap_or_default()
        .into_iter()
        .map(|face| (face.family, face.bold, face.italic, face.bytes))
        .collect();
    let named = scriva::app::font_names(&document);
    if !embedded.is_empty() || !named.is_empty() {
        println!(
            "embedded faces: {} | names: {}",
            embedded.len(),
            named.len()
        );
        ui_kit::fonts::embed_document(&ctx, &embedded, &named);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        shaper = scriva::shaper::Egui::new(&ctx);
    }

    let mut view = scriva::view::View::default();
    let fields = wp_layout::FieldValues::new();
    view.refresh(&document, &fields, 1, &mut shaper);

    let mut text = String::new();
    let _ = writeln!(text, "pages {}", view.pages().len());
    for (n, page) in view.pages().iter().enumerate() {
        let _ = writeln!(
            text,
            "PAGE {} {:.2}x{:.2} notes={}",
            n + 1,
            page.geometry.width,
            page.geometry.height,
            page.footnotes.len()
        );
        for op in wp_print::ops::flatten(page) {
            match op {
                wp_print::ops::Op::Text {
                    x,
                    baseline,
                    text: run,
                    advances,
                    font,
                    rgb,
                    rotation,
                    stretch,
                } => {
                    let width: f64 = advances.iter().sum();
                    let turn = match rotation == 0.0 && (stretch - 1.0).abs() < 1e-9 {
                        true => String::new(),
                        false => format!(" rot={rotation} stretch={stretch}"),
                    };
                    let _ = writeln!(
                        text,
                        "T {x:.2} {baseline:.2} {width:.2} {}pt{}{} {} #{:02x}{:02x}{:02x} {run:?}{turn}",
                        font.size,
                        if font.bold { " b" } else { "" },
                        if font.italic { " i" } else { "" },
                        font.family,
                        rgb[0],
                        rgb[1],
                        rgb[2]
                    );
                }
                wp_print::ops::Op::Fill {
                    x,
                    y,
                    width,
                    height,
                    rgb,
                } => {
                    let _ = writeln!(
                        text,
                        "F {x:.2} {y:.2} {width:.2} {height:.2} #{:02x}{:02x}{:02x}",
                        rgb[0], rgb[1], rgb[2]
                    );
                }
                wp_print::ops::Op::Rule {
                    from,
                    to,
                    thickness,
                    rgb,
                } => {
                    let _ = writeln!(
                        text,
                        "R {:.2} {:.2} {:.2} {:.2} {thickness:.2} #{:02x}{:02x}{:02x}",
                        from.0, from.1, to.0, to.1, rgb[0], rgb[1], rgb[2]
                    );
                }
                wp_print::ops::Op::Image {
                    x,
                    y,
                    width,
                    height,
                    rel,
                } => {
                    let _ = writeln!(text, "I {x:.2} {y:.2} {width:.2} {height:.2} {rel}");
                }
                wp_print::ops::Op::Chart {
                    x,
                    y,
                    width,
                    height,
                    rel,
                } => {
                    let _ = writeln!(text, "C {x:.2} {y:.2} {width:.2} {height:.2} {rel}");
                }
                wp_print::ops::Op::Poly { points, rgb } => {
                    let _ = writeln!(
                        text,
                        "P {} #{:02x}{:02x}{:02x}",
                        points.len(),
                        rgb[0],
                        rgb[1],
                        rgb[2]
                    );
                }
            }
        }
    }

    if let Ok(out) = std::env::var("PROBE_OUT") {
        std::fs::write(&out, &text).expect("the dump is written");
        println!("wrote {out}");
    } else {
        println!("{text}");
    }

    if let Ok(target) = std::env::var("PROBE_PDF") {
        let images =
            scriva::publish::rasters(package.as_ref(), parts.as_ref(), &loose, view.pages());
        let metafiles =
            scriva::publish::metafiles(package.as_ref(), parts.as_ref(), &loose, view.pages());
        let plots = scriva::publish::plots(package.as_ref(), parts.as_ref(), view.pages());
        let mut faces = scriva::publish::SystemFaces::new();
        let mut charts = wp_print::ops::Charts {
            plots: &plots,
            shaper: &mut shaper,
        };
        let pdf = wp_print::pdf::export(
            view.pages(),
            &mut faces,
            &images,
            &metafiles,
            Some(&mut charts),
            Some("probe"),
        );
        std::fs::write(&target, pdf).expect("the pdf is written");
        println!("wrote {target}");
    }
}
