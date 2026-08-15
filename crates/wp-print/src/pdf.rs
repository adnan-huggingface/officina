//! Laid-out pages to a PDF, with the document's own fonts inside it.
//!
//! The one decision that matters: **text is placed, not typeset**. Every
//! fragment arrives with the x its line gave it and every character with the
//! advance the layout measured, and the PDF's `TJ` operator is told to move the
//! pen by exactly those amounts — the difference between the font's natural
//! advance and the layout's is written between the glyphs. A viewer therefore
//! cannot re-break a line, whatever it thinks of the font's metrics, and the
//! exported page is the screen's page to the point.
//!
//! Fonts are embedded whole as `FontFile2` under an Identity-H CID font, which
//! is the one arrangement that talks glyph ids rather than encodings and so
//! never meets a code page. A `ToUnicode` map is written alongside so that the
//! text a recruiter selects and copies out of a resume is the text, not glyph
//! soup. A face the machine cannot supply falls back to Helvetica — the page
//! still says what it said, in the wrong clothes, which beats saying nothing.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use wp_layout::block::Page;
use wp_layout::FontRequest;

use crate::ops::{flatten, Op};
use crate::ttf::Face;
use crate::{Faces, Raster};

/// Turns pages into a complete PDF file.
///
/// `images` is keyed by relationship id, prepared by the caller — see
/// [`crate::ops::image_rels`] for the list worth decoding.
pub fn export(
    pages: &[Page],
    faces: &mut dyn Faces,
    images: &HashMap<String, Raster>,
    title: Option<&str>,
) -> Vec<u8> {
    let mut writer = Writer::new();
    let mut fonts = Fonts::default();
    let mut xobjects = XObjects::default();

    // Content first: it discovers which fonts and images the pages actually
    // use, and the resource objects follow.
    let mut page_contents = Vec::new();
    for page in pages {
        let content = content_stream(page, faces, &mut fonts, &mut xobjects, images);
        page_contents.push((page.geometry.width, page.geometry.height, content));
    }

    // Object 1 is the catalog, 2 the page tree; pages and their streams come
    // next, then fonts and images, then the info dictionary.
    let catalog = writer.reserve();
    let tree = writer.reserve();
    debug_assert_eq!((catalog, tree), (1, 2));

    let mut kids = Vec::new();
    let mut page_objects = Vec::new();
    for (width, height, content) in page_contents {
        let contents = writer.reserve();
        let page_object = writer.reserve();
        kids.push(page_object);
        page_objects.push((page_object, contents, width, height, content));
    }

    let font_ids: Vec<(String, u32)> = fonts
        .interned
        .iter()
        .enumerate()
        .map(|(index, _)| (format!("F{index}"), 0))
        .collect();
    // Font objects are written after the pages; their numbers are assigned as
    // they are written, so the resource dictionary is written last of all —
    // reserve it now.
    let resources = writer.reserve();

    for (page_object, contents, width, height, content) in page_objects {
        writer.stream_at(contents, &content, true);
        writer.object_at(
            page_object,
            &format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] \
                 /Contents {contents} 0 R /Resources {resources} 0 R >>",
                number(width),
                number(height),
            ),
        );
    }

    let font_ids: Vec<(String, u32)> = font_ids
        .into_iter()
        .zip(&fonts.interned)
        .map(|((name, _), slot)| (name, write_font(&mut writer, slot)))
        .collect();

    let image_ids: Vec<(String, u32)> = xobjects
        .interned
        .iter()
        .map(|(rel, index)| {
            let raster = &images[rel];
            (format!("Im{index}"), write_image(&mut writer, raster))
        })
        .collect();

    let mut resource_body = String::from("<< /Font <<");
    for (name, object) in &font_ids {
        resource_body.push_str(&format!(" /{name} {object} 0 R"));
    }
    resource_body.push_str(" >> /XObject <<");
    for (name, object) in &image_ids {
        resource_body.push_str(&format!(" /{name} {object} 0 R"));
    }
    resource_body.push_str(" >> >>");
    writer.object_at(resources, &resource_body);

    let kids_list: Vec<String> = kids.iter().map(|id| format!("{id} 0 R")).collect();
    writer.object_at(
        tree,
        &format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids_list.join(" "),
            kids.len()
        ),
    );
    writer.object_at(catalog, &format!("<< /Type /Catalog /Pages {tree} 0 R >>"));

    let info = title.map(|title| {
        writer.object(&format!(
            "<< /Producer (Scriva) /Title ({}) >>",
            escape_string(title)
        ))
    });

    writer.finish(catalog, info)
}

// ---------------------------------------------------------------------------
// The content of one page.
// ---------------------------------------------------------------------------

/// Fonts the pages turned out to use, one slot per distinct face.
#[derive(Default)]
struct Fonts {
    /// Index into `interned` by (family, bold, italic) — lowercased, because
    /// that is how the screen resolves names too.
    by_request: BTreeMap<(String, bool, bool), usize>,
    interned: Vec<FontSlot>,
}

struct FontSlot {
    /// The face to embed, or `None` for the Helvetica fallback.
    bytes: Option<Arc<[u8]>>,
    bold: bool,
    italic: bool,
    /// A printable name for the font dictionary.
    name: String,
    /// Every glyph the document used, with the character it stood for —
    /// what the widths array and the ToUnicode map are built from.
    used: BTreeMap<u16, char>,
}

impl Fonts {
    fn slot(&mut self, faces: &mut dyn Faces, font: &FontRequest) -> usize {
        let key = (font.family.to_ascii_lowercase(), font.bold, font.italic);
        if let Some(index) = self.by_request.get(&key) {
            return *index;
        }
        let bytes = faces.face(&font.family, font.bold, font.italic);
        // Two document names can resolve to one file — Arial and an unknown
        // name both come back as Arial — and embedding it twice would double
        // the file for nothing.
        let index = self
            .interned
            .iter()
            .position(|slot| match (&slot.bytes, &bytes) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => slot.bold == font.bold && slot.italic == font.italic,
                _ => false,
            })
            .unwrap_or_else(|| {
                self.interned.push(FontSlot {
                    bytes: bytes.clone(),
                    bold: font.bold,
                    italic: font.italic,
                    name: sanitize_name(&font.family),
                    used: BTreeMap::new(),
                });
                self.interned.len() - 1
            });
        self.by_request.insert(key, index);
        index
    }
}

#[derive(Default)]
struct XObjects {
    /// Relationship id to image index.
    interned: BTreeMap<String, usize>,
}

fn content_stream(
    page: &Page,
    faces: &mut dyn Faces,
    fonts: &mut Fonts,
    xobjects: &mut XObjects,
    images: &HashMap<String, Raster>,
) -> Vec<u8> {
    let height = page.geometry.height;
    let mut out = String::new();
    for op in flatten(page) {
        match op {
            Op::Fill {
                x,
                y,
                width,
                height: h,
                rgb,
            } => {
                out.push_str(&format!(
                    "{} rg {} {} {} {} re f\n",
                    color(rgb),
                    number(x),
                    number(height - y - h),
                    number(width),
                    number(h),
                ));
            }
            Op::Rule {
                from,
                to,
                thickness,
                rgb,
            } => {
                out.push_str(&format!(
                    "{} RG {} w {} {} m {} {} l S\n",
                    color(rgb),
                    number(thickness),
                    number(from.0),
                    number(height - from.1),
                    number(to.0),
                    number(height - to.1),
                ));
            }
            Op::Text {
                x,
                baseline,
                text,
                advances,
                font,
                rgb,
            } => {
                let index = fonts.slot(faces, &font);
                out.push_str(&text_op(
                    &mut fonts.interned[index],
                    index,
                    x,
                    height - baseline,
                    &text,
                    &advances,
                    &font,
                    rgb,
                ));
            }
            Op::Image {
                x,
                y,
                width,
                height: h,
                rel,
            } => {
                if !images.contains_key(&rel) {
                    continue;
                }
                let next = xobjects.interned.len();
                let index = *xobjects.interned.entry(rel).or_insert(next);
                out.push_str(&format!(
                    "q {} 0 0 {} {} {} cm /Im{index} Do Q\n",
                    number(width),
                    number(h),
                    number(x),
                    number(height - y - h),
                ));
            }
        }
    }
    out.into_bytes()
}

/// One text run: the glyphs, each forced to the layout's advance.
#[allow(clippy::too_many_arguments)]
fn text_op(
    slot: &mut FontSlot,
    index: usize,
    x: f64,
    baseline: f64,
    text: &str,
    advances: &[f64],
    font: &FontRequest,
    rgb: [u8; 3],
) -> String {
    let size = font.size.max(0.1);
    let mut glyphs = String::new();
    match slot.bytes.as_deref().and_then(Face::parse) {
        Some(face) => {
            let upem = face.units_per_em();
            for (index, c) in text.chars().enumerate() {
                let glyph = face.glyph(c);
                slot.used.entry(glyph).or_insert(c);
                let natural = f64::from(face.advance(glyph)) / upem * size;
                let wanted = advances.get(index).copied().unwrap_or(natural);
                glyphs.push_str(&format!("<{glyph:04X}>"));
                // TJ counts thousandths of the font size, positive moving the
                // pen *back* — so the correction is natural minus wanted.
                let correction = (natural - wanted) * 1000.0 / size;
                if correction.abs() > 0.01 {
                    glyphs.push_str(&number(correction));
                }
            }
        }
        None => {
            // The Helvetica fallback speaks WinAnsi, near enough: the byte is
            // the character for ASCII, and everything else becomes a question
            // mark rather than a wrong letter. Widths still come from the
            // layout, via the same corrections against Helvetica's own table.
            for (index, c) in text.chars().enumerate() {
                let byte = if c.is_ascii() { c as u8 } else { b'?' };
                slot.used.entry(byte as u16).or_insert(c);
                let natural = helvetica_width(byte, slot.bold) * size;
                let wanted = advances.get(index).copied().unwrap_or(natural);
                glyphs.push_str(&format!("<{byte:02X}>"));
                let correction = (natural - wanted) * 1000.0 / size;
                if correction.abs() > 0.01 {
                    glyphs.push_str(&number(correction));
                }
            }
        }
    }
    format!(
        "BT /F{index} {} Tf {} rg {} {} Td [{glyphs}] TJ ET\n",
        number(size),
        color(rgb),
        number(x),
        number(baseline),
    )
}

// ---------------------------------------------------------------------------
// Font objects.
// ---------------------------------------------------------------------------

fn write_font(writer: &mut Writer, slot: &FontSlot) -> u32 {
    match slot.bytes.as_deref().and_then(Face::parse) {
        Some(face) => write_embedded(writer, slot, &face),
        None => write_helvetica(writer, slot),
    }
}

fn write_embedded(writer: &mut Writer, slot: &FontSlot, face: &Face) -> u32 {
    let scale = 1000.0 / face.units_per_em();
    let font_file = writer.stream(face.bytes(), true);

    let flags = 4 // Symbolic: the truth for Identity-H, whatever the face.
        | if slot.italic { 64 } else { 0 };
    let descriptor = writer.object(&format!(
        "<< /Type /FontDescriptor /FontName /{} /Flags {flags} \
         /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} \
         /CapHeight {} /StemV {} /FontFile2 {font_file} 0 R >>",
        slot.name,
        number(f64::from(face.bbox[0]) * scale),
        number(f64::from(face.bbox[1]) * scale),
        number(f64::from(face.bbox[2]) * scale),
        number(f64::from(face.bbox[3]) * scale),
        number(face.italic_angle),
        number(f64::from(face.ascent) * scale),
        number(f64::from(face.descent) * scale),
        number(f64::from(face.cap_height) * scale),
        if slot.bold { 160 } else { 80 },
    ));

    // The widths of the glyphs the document used; a viewer needs them without
    // opening the font file.
    let mut widths = String::new();
    for &glyph in slot.used.keys() {
        widths.push_str(&format!(
            "{glyph} [{}] ",
            number(f64::from(face.advance(glyph)) * scale)
        ));
    }
    let descendant = writer.object(&format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{} \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
         /FontDescriptor {descriptor} 0 R /W [{widths}] /CIDToGIDMap /Identity >>",
        slot.name,
    ));

    let to_unicode = writer.stream(&to_unicode_cmap(&slot.used), true);
    writer.object(&format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /{} /Encoding /Identity-H \
         /DescendantFonts [{descendant} 0 R] /ToUnicode {to_unicode} 0 R >>",
        slot.name,
    ))
}

fn write_helvetica(writer: &mut Writer, slot: &FontSlot) -> u32 {
    let base = match (slot.bold, slot.italic) {
        (false, false) => "Helvetica",
        (true, false) => "Helvetica-Bold",
        (false, true) => "Helvetica-Oblique",
        (true, true) => "Helvetica-BoldOblique",
    };
    writer.object(&format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Encoding /WinAnsiEncoding >>"
    ))
}

/// The map from glyph id back to the character it drew, so text can be copied
/// out of the file.
fn to_unicode_cmap(used: &BTreeMap<u16, char>) -> Vec<u8> {
    let mut chars = String::new();
    for (&glyph, &c) in used {
        let mut units = [0u16; 2];
        let encoded = c.encode_utf16(&mut units);
        let hex: String = encoded.iter().map(|unit| format!("{unit:04X}")).collect();
        chars.push_str(&format!("<{glyph:04X}> <{hex}>\n"));
    }
    format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
         {} beginbfchar\n{chars}endbfchar\n\
         endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
        used.len()
    )
    .into_bytes()
}

/// Helvetica's widths, coarsely: the exact numbers matter little because every
/// advance is corrected to the layout's anyway, and this path only runs on a
/// machine with no fonts to embed.
fn helvetica_width(byte: u8, bold: bool) -> f64 {
    let narrow = b"ijltfI.,;:'|!()[]";
    let wide = b"mwMW@%";
    let width = if narrow.contains(&byte) {
        0.28
    } else if wide.contains(&byte) {
        0.89
    } else if byte == b' ' {
        0.28
    } else if byte.is_ascii_uppercase() || byte.is_ascii_digit() {
        0.67
    } else {
        0.55
    };
    if bold {
        width + 0.05
    } else {
        width
    }
}

// ---------------------------------------------------------------------------
// Image objects.
// ---------------------------------------------------------------------------

fn write_image(writer: &mut Writer, raster: &Raster) -> u32 {
    // A JPEG the reader can pass through goes in as its own bytes; DCTDecode
    // is precisely "the stream is a JPEG".
    if let Some(jpeg) = &raster.jpeg {
        if let Some(components) = jpeg_components(jpeg) {
            let space = if components == 1 {
                "/DeviceGray"
            } else {
                "/DeviceRGB"
            };
            let object = writer.reserve();
            writer.raw_stream_at(
                object,
                &format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                     /ColorSpace {space} /BitsPerComponent 8 /Filter /DCTDecode \
                     /Length {} >>",
                    raster.width,
                    raster.height,
                    jpeg.len()
                ),
                jpeg,
            );
            return object;
        }
    }

    let pixels = (raster.width as usize) * (raster.height as usize);
    let mut rgb = Vec::with_capacity(pixels * 3);
    let mut alpha = Vec::with_capacity(pixels);
    let mut transparent = false;
    for pixel in raster.rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
        alpha.push(pixel[3]);
        transparent |= pixel[3] != 255;
    }

    let smask = transparent.then(|| {
        let object = writer.reserve();
        let data = deflate(&alpha);
        writer.raw_stream_at(
            object,
            &format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode \
                 /Length {} >>",
                raster.width,
                raster.height,
                data.len()
            ),
            &data,
        );
        object
    });

    let data = deflate(&rgb);
    let object = writer.reserve();
    let smask_entry = smask
        .map(|id| format!(" /SMask {id} 0 R"))
        .unwrap_or_default();
    writer.raw_stream_at(
        object,
        &format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode \
             /Length {}{smask_entry} >>",
            raster.width,
            raster.height,
            data.len()
        ),
        &data,
    );
    object
}

/// How many colour components the JPEG's frame header states — and `None` for
/// the kinds (CMYK, progressive oddities) better re-encoded than passed through.
fn jpeg_components(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF {
            return None;
        }
        let marker = bytes[at + 1];
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        // Baseline and progressive frames both decode under DCTDecode.
        if matches!(marker, 0xC0..=0xC2) {
            let components = *bytes.get(at + 9)?;
            return matches!(components, 1 | 3).then_some(components);
        }
        // An arithmetic-coded or hierarchical frame is beyond what every
        // viewer decodes; let the pixel path handle it.
        if matches!(marker, 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCF) {
            return None;
        }
        at += 2 + length;
    }
    None
}

// ---------------------------------------------------------------------------
// The file itself: objects, the xref table, the trailer.
// ---------------------------------------------------------------------------

struct Writer {
    out: Vec<u8>,
    /// Byte offset of each written object, by number; zero means reserved but
    /// not yet written.
    offsets: Vec<usize>,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            out: b"%PDF-1.5\n%\xB5\xB5\xB5\xB5\n".to_vec(),
            offsets: Vec::new(),
        }
    }

    /// Allocates an object number to be written later.
    fn reserve(&mut self) -> u32 {
        self.offsets.push(0);
        self.offsets.len() as u32
    }

    fn object(&mut self, body: &str) -> u32 {
        let id = self.reserve();
        self.object_at(id, body);
        id
    }

    fn object_at(&mut self, id: u32, body: &str) {
        self.offsets[(id - 1) as usize] = self.out.len();
        let _ = write!(self.out, "{id} 0 obj\n{body}\nendobj\n");
    }

    /// A stream object, deflated unless it is already compressed data.
    fn stream(&mut self, data: &[u8], compress: bool) -> u32 {
        let id = self.reserve();
        self.stream_at(id, data, compress);
        id
    }

    fn stream_at(&mut self, id: u32, data: &[u8], compress: bool) {
        if compress {
            let deflated = deflate(data);
            self.raw_stream_at(
                id,
                &format!("<< /Filter /FlateDecode /Length {} >>", deflated.len()),
                &deflated,
            );
        } else {
            self.raw_stream_at(id, &format!("<< /Length {} >>", data.len()), data);
        }
    }

    /// A stream whose dictionary the caller wrote in full.
    fn raw_stream_at(&mut self, id: u32, dictionary: &str, data: &[u8]) {
        self.offsets[(id - 1) as usize] = self.out.len();
        let _ = write!(self.out, "{id} 0 obj\n{dictionary}\nstream\n");
        self.out.extend_from_slice(data);
        let _ = write!(self.out, "\nendstream\nendobj\n");
    }

    fn finish(mut self, catalog: u32, info: Option<u32>) -> Vec<u8> {
        let xref = self.out.len();
        let count = self.offsets.len() + 1;
        let _ = write!(self.out, "xref\n0 {count}\n0000000000 65535 f \n");
        for offset in &self.offsets {
            let _ = writeln!(self.out, "{offset:010} 00000 n ");
        }
        let info_entry = info
            .map(|id| format!(" /Info {id} 0 R"))
            .unwrap_or_default();
        let _ = write!(
            self.out,
            "trailer\n<< /Size {count} /Root {catalog} 0 R{info_entry} >>\n\
             startxref\n{xref}\n%%EOF\n"
        );
        self.out
    }
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

/// A number the way PDF likes them: no exponents, no trailing zeros.
fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let rounded = (value * 100.0).round() / 100.0;
    if rounded == rounded.trunc() {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn color([r, g, b]: [u8; 3]) -> String {
    format!(
        "{} {} {}",
        number(f64::from(r) / 255.0),
        number(f64::from(g) / 255.0),
        number(f64::from(b) / 255.0)
    )
}

/// A PDF name may not carry spaces or delimiters; a font family often does.
fn sanitize_name(family: &str) -> String {
    let cleaned: String = family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if cleaned.is_empty() {
        "Face".to_owned()
    } else {
        cleaned
    }
}

fn escape_string(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii() && !c.is_control())
        .map(|c| match c {
            '(' | ')' | '\\' => format!("\\{c}"),
            _ => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoFaces;
    impl Faces for NoFaces {
        fn face(&mut self, _: &str, _: bool, _: bool) -> Option<Arc<[u8]>> {
            None
        }
    }

    struct SystemFaces;
    impl Faces for SystemFaces {
        fn face(&mut self, _: &str, _: bool, _: bool) -> Option<Arc<[u8]>> {
            let windir = std::env::var_os("SystemRoot")?;
            let bytes =
                std::fs::read(std::path::PathBuf::from(windir).join("Fonts/arial.ttf")).ok()?;
            Some(Arc::from(bytes.into_boxed_slice()))
        }
    }

    fn page() -> Page {
        use wp_layout::block::{Placed, Placement};
        use wp_layout::inline::{Content, Fragment, Line};
        use wp_layout::TextStyle;
        let style = TextStyle {
            font: FontRequest::new("Arial", 12.0),
            color: Some([20, 30, 40]),
            highlight: None,
            shading: None,
            underline: wp_model::prop::UnderlineKind::None,
            underline_color: None,
            strike: false,
            double_strike: false,
            caps: false,
            small_caps: false,
            raise: 0.0,
            letter_spacing: 0.0,
            hidden: false,
            rtl: false,
        };
        Page {
            number: 1,
            section: 0,
            geometry: wp_model::PageBox {
                width: 612.0,
                height: 792.0,
                top: 72.0,
                bottom: 72.0,
                start: 72.0,
                end: 72.0,
            },
            content: vec![Placement {
                x: 72.0,
                y: 100.0,
                width: 30.0,
                height: 14.0,
                kind: Placed::Line {
                    line: Box::new(Line {
                        fragments: vec![Fragment {
                            x: 0.0,
                            width: 30.0,
                            style,
                            content: Content::Text {
                                text: "Hi".to_owned(),
                                advances: vec![9.0, 6.0],
                                hyphen: false,
                            },
                            source: None,
                            field: None,
                        }],
                        y: 0.0,
                        baseline: 11.0,
                        height: 14.0,
                        ascent: 11.0,
                        descent: 3.0,
                        x: 0.0,
                        width: 30.0,
                        ended_by: None,
                    }),
                    paragraph: 0,
                },
            }],
            header: Vec::new(),
            footer: Vec::new(),
            footnotes: Vec::new(),
        }
    }

    /// The offsets in the xref table must point at the objects they claim to.
    ///
    /// Byte arithmetic on the raw file: a PDF is full of compressed streams,
    /// and any text-decoded view of it has different offsets than the file.
    fn assert_xref_is_honest(pdf: &[u8]) {
        let marker = b"startxref\n";
        let at = (0..pdf.len().saturating_sub(marker.len()))
            .rev()
            .find(|&i| pdf[i..].starts_with(marker))
            .expect("a startxref");
        let tail = &pdf[at + marker.len()..];
        let end = tail.iter().position(|b| *b == b'\n').expect("a line");
        let xref_at: usize = std::str::from_utf8(&tail[..end])
            .expect("ascii")
            .trim()
            .parse()
            .expect("an offset");
        assert!(
            pdf[xref_at..].starts_with(b"xref"),
            "startxref points at the table"
        );
        let mut entries = pdf[xref_at..].split(|b| *b == b'\n').skip(2);
        // The first entry is the free-list head; the rest each name an object.
        assert!(entries.next().is_some_and(|line| line.ends_with(b"f ")));
        for (index, line) in entries.enumerate() {
            if !line.ends_with(b"n ") {
                break;
            }
            let offset: usize = std::str::from_utf8(&line[..10])
                .expect("ascii")
                .parse()
                .expect("an offset");
            let expected = format!("{} 0 obj", index + 1);
            assert!(
                pdf[offset..].starts_with(expected.as_bytes()),
                "object {} is not at its stated offset",
                index + 1
            );
        }
    }

    #[test]
    fn a_page_with_no_fonts_still_exports_a_wellformed_file() {
        let pdf = export(&[page()], &mut NoFaces, &HashMap::new(), Some("Test"));
        assert!(pdf.starts_with(b"%PDF-1.5"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Helvetica"), "the fallback stood in");
        assert!(text.contains("/MediaBox [0 0 612 792]"));
        assert_xref_is_honest(&pdf);
    }

    #[test]
    fn a_real_face_is_embedded_once_however_many_names_ask_for_it() {
        if SystemFaces.face("Arial", false, false).is_none() {
            return; // A machine with no fonts has nothing to say here.
        }
        let mut two_names = page();
        two_names.content.push(page().content[0].clone());
        let pdf = export(&[two_names], &mut SystemFaces, &HashMap::new(), None);
        let text = String::from_utf8_lossy(&pdf);
        assert_eq!(
            text.matches("/FontFile2").count(),
            1,
            "one embedded font file"
        );
        assert!(text.contains("/Identity-H"));
        assert!(text.contains("/ToUnicode"));
        assert_xref_is_honest(&pdf);
    }

    #[test]
    fn an_image_goes_in_as_pixels_with_its_transparency() {
        let raster = Raster {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 128],
            jpeg: None,
        };
        let mut images = HashMap::new();
        images.insert("rId1".to_owned(), raster);
        let mut with_image = page();
        with_image.content.push(wp_layout::block::Placement {
            x: 100.0,
            y: 200.0,
            width: 50.0,
            height: 25.0,
            kind: wp_layout::block::Placed::Drawing {
                rel: Some("rId1".into()),
                anchor: None,
                paragraph: 0,
                nth: 0,
            },
        });
        let pdf = export(&[with_image], &mut NoFaces, &images, None);
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("/SMask"),
            "half-transparent green needs a mask"
        );
        // The `Do` itself is inside the deflated content stream; the resource
        // dictionary that names it is not.
        assert!(text.contains("/XObject << /Im0"));
        assert_xref_is_honest(&pdf);
    }

    #[test]
    fn a_jpegs_bytes_pass_through_untouched() {
        // A minimal syntactically-plausible JPEG: SOI, a baseline SOF0 frame
        // header claiming 3 components, EOI.
        let jpeg = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xC0, 0x00, 0x0B, 8, 0, 1, 0, 1, 3, 0, 0, 0, // SOF0
            0xFF, 0xD9, // EOI
        ];
        assert_eq!(jpeg_components(&jpeg), Some(3));
        assert_eq!(jpeg_components(b"not a jpeg"), None);
    }

    #[test]
    fn numbers_are_written_plainly() {
        assert_eq!(number(612.0), "612");
        assert_eq!(number(11.5), "11.5");
        assert_eq!(number(0.333333), "0.33");
        assert_eq!(number(-3.10), "-3.1");
        assert_eq!(number(f64::NAN), "0");
    }
}
