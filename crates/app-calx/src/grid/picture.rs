//! Drawing an anchored picture, and the textures that make it possible.
//!
//! A picture arrives as an encoded file — a PNG, usually — and the renderer
//! wants premultiplied RGBA on the GPU. That conversion is expensive enough
//! that doing it per frame would be visible, so it happens once per part and
//! the handle is kept. The cache is keyed on the *part name* rather than on the
//! anchor: a logo repeated across fifteen sheets is one upload, not fifteen.
//!
//! A picture that fails to decode is remembered as a failure. Retrying every
//! frame would spend a decoder's worth of work, sixty times a second, to reach
//! the same answer; and something has to be drawn in its place either way.

use std::collections::HashMap;

use ss_model::Picture;
use ui_kit::egui;

/// Decoded images, by package part.
#[derive(Default)]
pub struct Textures {
    /// `None` marks a part we tried and could not decode, so we do not try
    /// again. The distinction from "not yet seen" is the whole point.
    loaded: HashMap<String, Option<egui::TextureHandle>>,
}

impl Textures {
    /// Decodes anything on this sheet that has not been decoded yet.
    ///
    /// Called once per frame from `show`, where `&mut self` is available;
    /// painting itself only reads.
    pub fn ensure(&mut self, ctx: &egui::Context, pictures: &[Picture]) {
        for picture in pictures {
            if self.loaded.contains_key(&picture.part) {
                continue;
            }
            let handle = decode(&picture.data)
                .map(|image| ctx.load_texture(&picture.part, image, egui::TextureOptions::LINEAR));
            self.loaded.insert(picture.part.clone(), handle);
        }
    }

    pub fn get(&self, part: &str) -> Option<&egui::TextureHandle> {
        self.loaded.get(part).and_then(|slot| slot.as_ref())
    }
}

/// Turns an image file into pixels egui can upload.
///
/// The formats enabled are the ones Excel actually embeds. A TIFF or an EMF —
/// both legal in a package, both rare — decodes to `None` and is drawn as a
/// placeholder, which is honest about there being something there.
fn decode(data: &[u8]) -> Option<egui::ColorImage> {
    let decoded = image::load_from_memory(data).ok()?;
    let rgba = decoded.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    if size[0] == 0 || size[1] == 0 {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

/// Draws one picture, or a marker where it would have been.
///
/// The image is *fitted* into the anchor rather than stretched to fill it.
/// Excel's own anchors are written from the picture's real size, so the two
/// agree to within a pixel on a file Excel wrote — but a two-cell anchor is
/// tied to the columns, and a column we measure a little differently would
/// otherwise stretch a logo into something the company would not recognise.
pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    picture: &Picture,
    textures: &Textures,
    outline: egui::Color32,
) {
    let Some(texture) = textures.get(&picture.part) else {
        // Something is anchored here and we cannot show it. A dashed box says
        // so; nothing at all would read as an empty sheet.
        painter.rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, outline),
            egui::StrokeKind::Inside,
        );
        return;
    };
    painter.image(
        texture.id(),
        fit(rect, texture.size_vec2()),
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

/// The largest rectangle of the image's own proportions that fits in `into`,
/// centred. Never larger than the anchor, so a picture cannot bleed over cells
/// Excel would not have covered.
fn fit(into: egui::Rect, natural: egui::Vec2) -> egui::Rect {
    if natural.x <= 0.0 || natural.y <= 0.0 || !into.is_positive() {
        return into;
    }
    let scale = (into.width() / natural.x).min(into.height() / natural.y);
    let size = natural * scale;
    egui::Rect::from_center_size(into.center(), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_keeps_its_proportions_inside_a_wider_anchor() {
        let anchor = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        let placed = fit(anchor, egui::vec2(100.0, 100.0));
        assert_eq!(placed.size(), egui::vec2(100.0, 100.0));
        // Centred, so the spare width is split evenly.
        assert_eq!(placed.center(), anchor.center());
    }

    #[test]
    fn a_picture_never_grows_past_its_anchor() {
        let anchor = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(50.0, 50.0));
        let placed = fit(anchor, egui::vec2(4000.0, 1000.0));
        assert!(placed.width() <= anchor.width() + 0.01);
        assert!(placed.height() <= anchor.height() + 0.01);
    }

    #[test]
    fn bytes_that_are_not_an_image_decode_to_nothing_rather_than_panicking() {
        assert!(decode(b"this is not a png").is_none());
    }

    #[test]
    fn a_one_pixel_png_decodes() {
        // A 1x1 opaque red PNG, written out by hand so the test needs no file.
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let image = decode(PNG).expect("decodes");
        assert_eq!(image.size, [1, 1]);
    }
}
