//! What leaves the editor: the fonts and pictures a page renderer needs.
//!
//! `wp-print` deliberately knows nothing about packages, egui or the disk.
//! This module is the adapter: font names resolve to the same files the
//! screen's shaper resolved them to, and image relationships resolve through
//! the package to decoded pixels — with the original bytes kept alongside when
//! they are a JPEG a PDF can embed without recompressing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use wp_print::Raster;

/// Font files as the screen resolves them, cached per face and shared per
/// file — Arial asked for under three names is read once and embedded once.
#[derive(Default)]
pub struct SystemFaces {
    #[allow(clippy::type_complexity)]
    by_request: HashMap<(String, bool, bool), Option<Arc<[u8]>>>,
    by_path: HashMap<PathBuf, Arc<[u8]>>,
}

impl SystemFaces {
    pub fn new() -> SystemFaces {
        SystemFaces::default()
    }
}

impl wp_print::Faces for SystemFaces {
    fn face(&mut self, family: &str, bold: bool, italic: bool) -> Option<Arc<[u8]>> {
        let key = (family.to_ascii_lowercase(), bold, italic);
        if let Some(known) = self.by_request.get(&key) {
            return known.clone();
        }
        let resolved = ui_kit::fonts::face_file(family, bold, italic).map(|(path, bytes)| {
            self.by_path
                .entry(path)
                .or_insert_with(|| Arc::from(bytes.into_boxed_slice()))
                .clone()
        });
        self.by_request.insert(key, resolved.clone());
        resolved
    }
}

/// Every image the pages draw, decoded once, by relationship id.
pub fn rasters(
    package: Option<&ooxml::Package>,
    parts: Option<&wp_docx::DocumentParts>,
    pages: &[wp_layout::block::Page],
) -> HashMap<String, Raster> {
    let mut out = HashMap::new();
    let (Some(package), Some(parts)) = (package, parts) else {
        return out;
    };
    for rel in wp_print::ops::image_rels(pages.iter()) {
        let Some(name) = parts.target(&rel) else {
            continue;
        };
        let Some(bytes) = package.part(name).map(|part| part.data()) else {
            continue;
        };
        let Ok(image) = image::load_from_memory(bytes) else {
            continue;
        };
        let rgba = image.to_rgba8();
        let jpeg = bytes.starts_with(&[0xFF, 0xD8]).then(|| bytes.to_vec());
        out.insert(
            rel,
            Raster {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
                jpeg,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_kit::egui;
    use wp_model::doc::{Block, Paragraph};
    use wp_model::Document;

    #[test]
    fn a_document_with_no_package_has_no_rasters_rather_than_a_panic() {
        assert!(rasters(None, None, &[]).is_empty());
    }

    #[test]
    fn the_pages_the_screen_laid_out_export_to_a_wellformed_pdf() {
        // The whole path the Export command takes, minus the file dialog: the
        // screen's own layout, through the screen's own font resolution, into
        // a file that ends the way a PDF must.
        let ctx = egui::Context::default();
        ui_kit::fonts::register(&ctx, &[]);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let mut shaper = crate::shaper::Egui::new(&ctx);
        let mut view = crate::view::View::default();
        let document = Document {
            body: vec![
                Block::Paragraph(Paragraph::of("What the screen shows")),
                Block::Paragraph(Paragraph::of("is what the paper says.")),
            ],
            ..Document::new()
        };
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        assert!(!view.pages().is_empty());
        let mut faces = SystemFaces::new();
        let pdf = wp_print::pdf::export(view.pages(), &mut faces, &HashMap::new(), Some("proof"));
        assert!(pdf.starts_with(b"%PDF-1.5"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        assert!(pdf.len() > 500, "a page of text is not {} bytes", pdf.len());
    }
}
