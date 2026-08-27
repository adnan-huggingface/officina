//! Decoding the drawings a document carries, and holding them for the painter.
//!
//! Pictures and charts together, because a `<w:drawing>` is either one and the
//! painter meets them at the same place — a rectangle in a line, or a
//! rectangle on the page. A chart costs a parse rather than a decode, and is
//! held for the same reason: re-reading `chart1.xml` on every frame is the
//! same waste as decoding the JPEG on every frame.
//!
//! **The bytes are shared and decoded once per part.** A logo repeated in a
//! header on forty pages is one relationship, one part and one texture upload —
//! decoding it per placement would decode it forty times a frame. Calx learned
//! this with a fifteen-sheet workbook whose masthead was on every sheet.
//!
//! **A part that fails to decode is remembered as a failure.** Otherwise a
//! broken PNG is retried sixty times a second for as long as the document is
//! open, and the application is busy doing nothing.

use std::collections::HashMap;

use ui_kit::egui;

/// The drawings of one document, decoded on demand.
#[derive(Default)]
pub struct Pictures {
    /// By relationship id, because that is what a `<a:blip r:embed>` names.
    loaded: HashMap<String, Option<egui::TextureHandle>>,
    /// The same, for the `<c:chart r:id>` of a chart drawing.
    charts: HashMap<String, Option<chart::Plot>>,
    /// And the same for a picture that is a metafile — a recording of the
    /// calls that drew a diagram rather than pixels of one. Playing it costs
    /// what parsing a chart costs and is held for the same reason.
    played: HashMap<String, Option<metafile::Picture>>,
    /// Pictures that came without a package: a `.doc` keeps its own in a
    /// stream of its own, and there is no part for a relationship to name.
    /// Consulted first, because a document is either one or the other.
    loose: HashMap<String, Vec<u8>>,
}

impl Pictures {
    pub fn new() -> Pictures {
        Pictures::default()
    }

    /// Throws away every texture, for a document that has been closed.
    pub fn clear(&mut self) {
        self.loaded.clear();
        self.charts.clear();
        self.played.clear();
        self.loose.clear();
    }

    /// Takes up the pictures of a document that arrived without a package.
    pub fn adopt(&mut self, media: impl IntoIterator<Item = (String, Vec<u8>)>) {
        self.loose.extend(media);
    }

    /// The pictures of a document that arrived without a package, for a
    /// renderer that decodes its own.
    pub fn loose(&self) -> &HashMap<String, Vec<u8>> {
        &self.loose
    }

    /// The texture for a relationship, decoding it the first time.
    ///
    /// `None` for an image this cannot decode — and it is asked for only once,
    /// however many times it is drawn.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        package: Option<&ooxml::Package>,
        parts: Option<&wp_docx::DocumentParts>,
        rel: &str,
    ) -> Option<&egui::TextureHandle> {
        if !self.loaded.contains_key(rel) {
            let decoded = decode(ctx, package, parts, &self.loose, rel);
            self.loaded.insert(rel.to_owned(), decoded);
        }
        self.loaded.get(rel).and_then(|slot| slot.as_ref())
    }

    /// Decodes everything a laid-out view draws, before the frame is painted.
    ///
    /// The painter holds the pages by reference and cannot also hold this
    /// mutably, so the decoding happens first and the painting reads what it
    /// left. Parts already decoded are not decoded again.
    pub fn prepare(
        &mut self,
        ctx: &egui::Context,
        package: Option<&ooxml::Package>,
        parts: Option<&wp_docx::DocumentParts>,
        rels: impl Iterator<Item = String>,
    ) {
        for rel in rels {
            if self.loaded.contains_key(&rel) {
                continue;
            }
            // A metafile is played rather than decoded, and there is no
            // texture behind it at all: the ink is drawn straight onto the
            // page. Trying the raster decoder on one first would only fail.
            let picture = raw(package, parts, &self.loose, &rel).and_then(metafile::read);
            if picture.is_some() {
                self.played.insert(rel.clone(), picture);
                self.loaded.insert(rel, None);
                continue;
            }
            let decoded = decode(ctx, package, parts, &self.loose, &rel);
            self.loaded.insert(rel, decoded);
        }
    }

    /// The played metafile for a relationship that [`Pictures::prepare`] has
    /// seen, for a painter that draws its ink instead of a texture.
    pub fn played(&self, rel: &str) -> Option<&metafile::Picture> {
        self.played.get(rel).and_then(|slot| slot.as_ref())
    }

    /// The texture for a relationship that [`Pictures::prepare`] has seen.
    pub fn texture(&self, rel: &str) -> Option<&egui::TextureHandle> {
        self.loaded.get(rel).and_then(|slot| slot.as_ref())
    }

    /// Reads every chart part a laid-out view draws, before the frame is
    /// painted — the same contract as [`Pictures::prepare`], for the same
    /// borrow reason.
    pub fn prepare_charts(
        &mut self,
        package: Option<&ooxml::Package>,
        parts: Option<&wp_docx::DocumentParts>,
        rels: impl Iterator<Item = String>,
    ) {
        for rel in rels {
            if self.charts.contains_key(&rel) {
                continue;
            }
            let plot = (|| -> Option<chart::Plot> {
                let name = parts?.target(&rel)?;
                chart::read::plot(package?.part(name)?.data())
            })();
            self.charts.insert(rel, plot);
        }
    }

    /// The chart for a relationship that [`Pictures::prepare_charts`] has seen.
    pub fn chart(&self, rel: &str) -> Option<&chart::Plot> {
        self.charts.get(rel).and_then(|slot| slot.as_ref())
    }

    /// How many parts have been looked at, for a test.
    pub fn asked(&self) -> usize {
        self.loaded.len()
    }
}

/// The bytes behind one relationship, whichever way this document keeps them.
/// The bytes behind a picture name — the washout the name may carry is not
/// this function's business, only the relationship inside it.
fn raw<'a>(
    package: Option<&'a ooxml::Package>,
    parts: Option<&wp_docx::DocumentParts>,
    loose: &'a HashMap<String, Vec<u8>>,
    key: &str,
) -> Option<&'a [u8]> {
    let (rel, _) = wp_model::doc::picture_source(key);
    match loose.get(rel) {
        Some(bytes) => Some(bytes.as_slice()),
        None => Some(package?.part(parts?.target(rel)?)?.data()),
    }
}

fn decode(
    ctx: &egui::Context,
    package: Option<&ooxml::Package>,
    parts: Option<&wp_docx::DocumentParts>,
    loose: &HashMap<String, Vec<u8>>,
    key: &str,
) -> Option<egui::TextureHandle> {
    let bytes = raw(package, parts, loose, key)?;
    let image = image::load_from_memory(bytes).ok()?;
    let mut rgba = image.to_rgba8();
    // A washed-out watermark is the picture *recoloured*, not the picture
    // drawn faintly — Word bakes the same answer into its own PDF — so the
    // samples are put through the tone once, here, and the texture is what
    // gets drawn. See `wp_model::doc::Tone`.
    if let (_, Some(tone)) = wp_model::doc::picture_source(key) {
        tone.apply_rgba(&mut rgba);
    }
    let size = [rgba.width() as usize, rgba.height() as usize];
    let colour = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(
        format!("scriva-image-{key}"),
        colour,
        egui::TextureOptions::LINEAR,
    ))
}

/// Where an anchored drawing sits on the page.
///
/// This lives in the layout, not here: it is geometry, and a headless test of it
/// should not need a window. Re-exported so the painter reads as one thing.
pub use wp_layout::block::anchor_position;

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Alignment, DrawingPosition, Offset, RelativeTo};
    use wp_model::{Emu, PageBox};

    fn page() -> PageBox {
        PageBox {
            width: 612.0,
            height: 792.0,
            top: 72.0,
            bottom: 72.0,
            start: 72.0,
            end: 72.0,
        }
    }

    #[test]
    fn preparing_the_same_part_twice_decodes_it_once() {
        let ctx = egui::Context::default();
        let mut pictures = Pictures::new();
        let rels = ["rId3".to_owned(), "rId3".to_owned(), "rId4".to_owned()];
        pictures.prepare(&ctx, None, None, rels.into_iter());
        assert_eq!(pictures.asked(), 2);
        assert!(pictures.texture("rId3").is_none());
    }

    fn drawing(position: Option<DrawingPosition>) -> wp_model::Drawing {
        wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (Emu::from_points(100.0), Emu::from_points(50.0)),
            rel: Some("rId5".into()),
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::Square,
            distance: Default::default(),
            position: position.map(Box::new),
            behind_text: false,
            tone: None,
            text: None,
            outline: None,
        }
    }

    fn at(horizontal: Offset, vertical: Offset) -> DrawingPosition {
        DrawingPosition {
            horizontal,
            vertical,
        }
    }

    fn aligned(align: Alignment, relative_to: RelativeTo) -> Offset {
        Offset {
            relative_to,
            offset: None,
            align: Some(align),
        }
    }

    fn offset(points: f64, relative_to: RelativeTo) -> Offset {
        Offset {
            relative_to,
            offset: Some(Emu::from_points(points)),
            align: None,
        }
    }

    #[test]
    fn a_centred_drawing_is_centred_on_the_text_column() {
        let drawing = drawing(Some(at(
            aligned(Alignment::Center, RelativeTo::Margin),
            aligned(Alignment::Top, RelativeTo::Margin),
        )));
        let (x, y) = anchor_position(&drawing, &page(), (72.0, 200.0));
        // The column is 72..540, so its middle is 306 and a 100pt picture
        // starts at 256.
        assert_eq!(x, 256.0);
        assert_eq!(y, 72.0);
    }

    #[test]
    fn relative_to_the_page_measures_from_the_paper_rather_than_the_margin() {
        let drawing = drawing(Some(at(
            aligned(Alignment::Left, RelativeTo::Page),
            aligned(Alignment::Bottom, RelativeTo::Page),
        )));
        let (x, y) = anchor_position(&drawing, &page(), (72.0, 200.0));
        assert_eq!(x, 0.0);
        assert_eq!(y, 742.0, "the bottom of the paper, less its height");
    }

    #[test]
    fn an_offset_from_the_margin_is_measured_from_the_margin() {
        let drawing = drawing(Some(at(
            offset(36.0, RelativeTo::Margin),
            offset(18.0, RelativeTo::Margin),
        )));
        let (x, y) = anchor_position(&drawing, &page(), (72.0, 200.0));
        assert_eq!(x, 108.0);
        assert_eq!(y, 90.0);
    }

    #[test]
    fn an_offset_from_the_paragraph_is_measured_from_where_the_text_is() {
        // This is what makes a picture travel with the paragraph it belongs to
        // rather than staying where it was when the document was written.
        let drawing = drawing(Some(at(
            offset(0.0, RelativeTo::Column),
            offset(10.0, RelativeTo::Paragraph),
        )));
        let (_, y) = anchor_position(&drawing, &page(), (72.0, 400.0));
        assert_eq!(y, 410.0);
    }

    #[test]
    fn a_column_offset_is_measured_from_the_column_the_paragraph_is_set_in() {
        // A shape anchored inside a table cell measures from that cell's text,
        // not from the page's margin. This is how Word draws the page frame of
        // a document whose letterhead is a table: the frame states a negative
        // offset from a cell that begins further in, and reading the offset
        // from the margin instead puts the whole frame off centre.
        let drawing = drawing(Some(at(
            offset(-5.4, RelativeTo::Column),
            offset(0.0, RelativeTo::Paragraph),
        )));
        let (x, _) = anchor_position(&drawing, &page(), (113.4, 200.0));
        assert_eq!(x, 108.0);
    }

    #[test]
    fn a_drawing_that_states_no_position_goes_where_the_text_is() {
        let (x, y) = anchor_position(&drawing(None), &page(), (72.0, 300.0));
        assert_eq!((x, y), (72.0, 300.0));
    }

    #[test]
    fn an_image_that_cannot_be_decoded_is_asked_for_once() {
        // Otherwise a broken PNG is retried sixty times a second for as long as
        // the document is open.
        let ctx = egui::Context::default();
        let mut pictures = Pictures::new();
        assert!(pictures.get(&ctx, None, None, "rId9").is_none());
        assert_eq!(pictures.asked(), 1);
        assert!(pictures.get(&ctx, None, None, "rId9").is_none());
        assert_eq!(pictures.asked(), 1, "and not asked again");
    }
}
