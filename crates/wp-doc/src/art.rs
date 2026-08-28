//! OfficeArt: the drawing layer a `.doc` keeps beside its text.
//!
//! Every picture, shape and piece of WordArt in a `.doc` lives here rather than
//! in the text. The text holds only an anchor — `\x01` for something inline,
//! `\x08` for something floating — and the drawing itself is a tree of records
//! in the table stream, shared by the whole document.
//!
//! **The tree is uniform and that is the whole trick.** Every record, container
//! or leaf, starts with the same eight bytes: a version and instance packed
//! into one word, a type, and a length. A reader that knows nothing about a
//! record can still step over it exactly, which is what makes it safe to look
//! for the four record types this understands and ignore the several dozen it
//! does not.
//!
//! **Pictures are stored once and referred to by number.** The drawing group
//! holds a store of BLIPs — the raw JPEG, PNG or metafile bytes — and a shape
//! names one by index. A logo in a header that appears on forty pages is one
//! BLIP, which is also why the model gives every BLIP a name and lets the
//! drawings share it.
//!
//! What is read here is what can be drawn: the BLIP bytes, and per shape its
//! identifier, its picture, its WordArt text and the handful of properties that
//! decide how that text looks. Connectors, callouts, groups, and the ~500 other
//! shape properties are stepped over.

use crate::fib::{u16 as read_u16, u32 as read_u32, Fib};

/// Record types, by the names [MS-ODRAW] gives them.
mod kind {
    pub const DGG_CONTAINER: u16 = 0xF000;
    pub const BSTORE_CONTAINER: u16 = 0xF001;
    pub const DG_CONTAINER: u16 = 0xF002;
    pub const SPGR_CONTAINER: u16 = 0xF003;
    pub const SP_CONTAINER: u16 = 0xF004;
    pub const FBSE: u16 = 0xF007;
    pub const FSP: u16 = 0xF00A;
    pub const FOPT: u16 = 0xF00B;
    /// The other two property tables. They hold the same kind of entries and
    /// differ only in which properties were put in them, so all three are read
    /// the same way — and Word puts a shape's *position* in the third.
    pub const SECONDARY_FOPT: u16 = 0xF121;
    pub const TERTIARY_FOPT: u16 = 0xF122;
    /// The BLIP records, `OfficeArtBlipEMF` through `OfficeArtBlipJPEG`.
    pub const BLIP_FIRST: u16 = 0xF018;
    pub const BLIP_LAST: u16 = 0xF117;
}

/// One record: its instance field, which several record types overload, and
/// everything after the eight-byte header.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    pub instance: u16,
    pub kind: u16,
    pub data: &'a [u8],
}

/// The records directly inside one container, in file order.
///
/// A record whose length runs past the end of what it was given is dropped
/// rather than clamped: a truncated container is a damaged file, and reading
/// half a record produces confident nonsense.
pub fn records(data: &[u8]) -> Vec<Record<'_>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let head = read_u16(data, at);
        let kind = read_u16(data, at + 2);
        let length = read_u32(data, at + 4) as usize;
        let Some(body) = data.get(at + 8..at + 8 + length) else {
            break;
        };
        out.push(Record {
            instance: head >> 4,
            kind,
            data: body,
        });
        at += 8 + length;
    }
    out
}

/// The bytes of one picture, and what they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blip {
    pub data: Vec<u8>,
    /// The MIME type, for a decoder that is handed the bytes alone.
    pub content_type: &'static str,
}

/// One shape, reduced to what can be drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// What an anchor names this shape by.
    pub spid: u32,
    /// The shape's geometry, from the `MSOSPT` its `OfficeArtFSP` header
    /// carries. 1 is the rectangle; 136 and up are the WordArt shapes, which
    /// draw their words instead.
    pub kind: u16,
    /// `lineColor` and `lineWidth`, and whether `fLine` says the shape has one.
    /// Word states neither for an ordinary black hairline, so the defaults are
    /// the file's defaults and not zero.
    pub line: wp_model::Color,
    pub line_width: i32,
    pub lined: bool,
    /// `pib` — which picture of the store this shape shows, if it shows one.
    /// Stated one-based in the file and held that way here, because zero is
    /// how the file says "no picture".
    pub picture: Option<u32>,
    /// `gtextUNICODE` — the words of a piece of WordArt, which is how Word
    /// writes a watermark. A shape with this and no picture *is* its text.
    pub text: Option<String>,
    /// `gtextFont`, the face that text is set in.
    pub font: Option<String>,
    /// `fGtextFStretch` of `geometryTextBooleanProperties` — whether the words
    /// are pulled about until their ink fills the shape, or only set at the
    /// size that fills its width. See [`wp_model::doc::ShapeText::stretch`].
    pub stretch: bool,
    /// `wzName` — the name the selection pane shows. Word's own watermarks are
    /// called `PowerPlusWaterMarkObject` and nothing else is, which is the only
    /// way to tell one from an ordinary piece of WordArt.
    pub name: Option<String>,
    /// `fillColor`, and whether `fFilled` says it is used at all.
    pub fill: Option<wp_model::Color>,
    pub filled: bool,
    /// `fillOpacity`, one being solid. **Word's own watermark is half of
    /// one**, and a reader that ignores it stamps the page twice as dark as
    /// Word does — measured, Word's grey comes out at 223 over white paper
    /// where the stated `#C0C0C0` alone would give 192.
    pub opacity: f64,
    /// `rotation`, in degrees clockwise.
    pub rotation: f64,
    /// `posh` with `posrelh`, and `posv` with `posrelv`: how Word places the
    /// shape, and what it places it against.
    ///
    /// **These override the anchor's rectangle**, and a reader that trusts the
    /// rectangle alone puts a watermark meant for the middle of the page at
    /// the top of it, where the paragraph that anchors it happens to be.
    pub horizontal: Option<Placement>,
    pub vertical: Option<Placement>,
}

impl Default for Shape {
    /// A shape that states nothing is filled with white and drawn with a
    /// black hairline — OfficeArt's own defaults, not this language's.
    fn default() -> Shape {
        Shape {
            spid: 0,
            kind: 0,
            line: wp_model::Color::Rgb([0, 0, 0]),
            line_width: 9525,
            lined: true,
            picture: None,
            text: None,
            font: None,
            stretch: false,
            name: None,
            fill: None,
            filled: true,
            opacity: 1.0,
            rotation: 0.0,
            horizontal: None,
            vertical: None,
        }
    }
}

/// One axis of a shape's position: where it sits, and what it sits against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// `posh`/`posv`: 0 the rectangle decides, 1 leading, 2 centre, 3
    /// trailing, 4 inside, 5 outside.
    pub align: u8,
    /// `posrelh`: 0 the margin, 1 the page, 2 the text, 3 the character.
    /// `posrelv`: 0 the margin, 1 the page, 2 the text, 3 the line.
    pub relative_to: u8,
}

/// Every drawing in one document.
#[derive(Debug, Default)]
pub struct Drawings {
    /// The BLIP store, in `pib` order, with a gap for every picture whose
    /// format this cannot hand to a decoder.
    pub blips: Vec<Option<Blip>>,
    /// Shapes of the main document, by shape identifier.
    pub main: Vec<Shape>,
    /// Shapes of the header document. A watermark is one of these.
    pub header: Vec<Shape>,
}

impl Drawings {
    /// The shape one anchor names, in whichever of the two drawings it is in.
    pub fn shape(&self, spid: u32) -> Option<&Shape> {
        self.main
            .iter()
            .chain(&self.header)
            .find(|shape| shape.spid == spid)
    }

    /// The picture a shape shows.
    pub fn blip(&self, index: u32) -> Option<&Blip> {
        self.blips.get(index.checked_sub(1)? as usize)?.as_ref()
    }
}

/// Reads the whole drawing layer out of the table stream.
pub fn read(fib: &Fib, table: &[u8]) -> Drawings {
    let mut out = Drawings::default();
    let Some(content) = fib.slice(table, crate::fib::field::DGG_INFO) else {
        return out;
    };
    let mut at = 0usize;
    for record in records(content) {
        // The length of what has been consumed, so the drawings that follow
        // the group can be found: `OfficeArtContent` is one drawing-group
        // container and then an array of drawings, each preceded by a byte
        // that says which document it belongs to.
        at += 8 + record.data.len();
        if record.kind == kind::DGG_CONTAINER {
            out.blips = blip_store(record.data);
            break;
        }
    }
    while at + 1 < content.len() {
        let in_header = content[at] == 0x01;
        let Some(record) = records(&content[at + 1..]).into_iter().next() else {
            break;
        };
        at += 1 + 8 + record.data.len();
        if record.kind != kind::DG_CONTAINER {
            break;
        }
        let shapes = match in_header {
            true => &mut out.header,
            false => &mut out.main,
        };
        collect_shapes(record.data, shapes);
    }
    out
}

/// The pictures of the drawing group, in the order `pib` counts them.
fn blip_store(dgg: &[u8]) -> Vec<Option<Blip>> {
    let Some(store) = records(dgg)
        .into_iter()
        .find(|record| record.kind == kind::BSTORE_CONTAINER)
    else {
        return Vec::new();
    };
    records(store.data).into_iter().map(blip_block).collect()
}

/// One `OfficeArtBStoreContainerFileBlock`: either a picture, or a picture
/// wrapped in the bookkeeping that lets several shapes share it.
///
/// Both shapes appear in both places — the document's shared store and the
/// bytes an inline picture carries with it — so which one a caller is looking
/// at is not a thing the caller should have to know.
pub fn blip_block(record: Record<'_>) -> Option<Blip> {
    match record.kind {
        kind::FBSE => fbse(record.data),
        kind::BLIP_FIRST..=kind::BLIP_LAST => blip_of(record),
        _ => None,
    }
}

/// `OfficeArtFBSE` — a BLIP wrapped in the bookkeeping that lets several
/// shapes share it: two rendering hints, a digest, a tag, the size, a
/// reference count, an offset into the delay stream, three unused bytes and a
/// name, and only then the picture itself.
///
/// A file whose `foDelay` sends the reader to a stream this does not open has
/// no embedded BLIP at all, and answers `None` rather than a guess.
fn fbse(data: &[u8]) -> Option<Blip> {
    let name_length = *data.get(33)? as usize;
    let embedded = data.get(36 + name_length..)?;
    blip_of(records(embedded).into_iter().next()?)
}

/// One `OfficeArtBlip`. The header's *instance* says both which format the
/// bytes are in and — by being odd — whether there are one or two digests in
/// front of them.
pub fn blip_of(record: Record<'_>) -> Option<Blip> {
    // Two digests when the instance is the odd one of the format's pair.
    let digests = match record.instance & 1 {
        1 => 32,
        _ => 16,
    };
    let content_type = match record.kind {
        0xF01D | 0xF02A => "image/jpeg",
        0xF01E => "image/png",
        0xF029 => "image/tiff",
        // A metafile is not a picture but a recording of the calls that drew
        // one, and it is kept compressed with a header of its own in front.
        0xF01A => return metafile(record.data.get(digests..)?, "image/x-emf"),
        0xF01B => return metafile(record.data.get(digests..)?, "image/x-wmf"),
        // A `PICT` or a device-independent bitmap: counted, and left alone
        // rather than mangled by a decoder that does not take them.
        _ => return None,
    };
    // Then one byte of tag, and the picture.
    Some(Blip {
        data: record.data.get(digests + 1..)?.to_vec(),
        content_type,
    })
}

/// `OfficeArtMetafileHeader` and the metafile behind it.
///
/// The header is thirty-four bytes — the uncompressed size, the bounds, the
/// size in EMUs, the saved size, and how the bytes were squeezed — and the
/// recording itself is deflated far more often than not. A metafile that
/// cannot be inflated is dropped rather than handed on: half a recording
/// plays as a drawing that stops in the middle, which looks like a document
/// that was written that way.
fn metafile(data: &[u8], content_type: &'static str) -> Option<Blip> {
    /// `MSOBLIPCOMPRESSION_DEFLATE`. The other value that appears is 0xFE,
    /// which is no compression at all.
    const DEFLATE: u8 = 0x00;

    /// How much of the stated uncompressed size is believed in advance. The
    /// decoder still grows past it; this only keeps a damaged header from
    /// asking for four gigabytes before a single byte has been read.
    const TRUSTED: usize = 64 << 20;

    let header = data.get(..34)?;
    let saved = read_u32(header, 28) as usize;
    let bytes = data.get(34..34 + saved).unwrap_or(data.get(34..)?);
    let data = match header[32] {
        DEFLATE => {
            let mut out = Vec::with_capacity((read_u32(header, 0) as usize).min(TRUSTED));
            let mut reader = flate2::bufread::ZlibDecoder::new(bytes);
            std::io::Read::read_to_end(&mut reader, &mut out).ok()?;
            out
        }
        _ => bytes.to_vec(),
    };
    Some(Blip { data, content_type })
}

/// Walks a drawing for its shapes, through however many groups it nests them
/// in. A group is only a box drawn round shapes; what is drawn is the shapes.
fn collect_shapes(container: &[u8], out: &mut Vec<Shape>) {
    for record in records(container) {
        match record.kind {
            kind::SPGR_CONTAINER | kind::DG_CONTAINER => collect_shapes(record.data, out),
            kind::SP_CONTAINER => {
                if let Some(shape) = shape_of(record.data) {
                    out.push(shape);
                }
            }
            _ => {}
        }
    }
}

/// One `OfficeArtSpContainer`: the shape's identifier, then its properties.
pub fn shape_of(container: &[u8]) -> Option<Shape> {
    let mut shape = Shape::default();
    let mut found = false;
    for record in records(container) {
        match record.kind {
            kind::FSP => {
                shape.spid = read_u32(record.data, 0);
                // The header's instance field is the shape's geometry.
                shape.kind = record.instance;
                found = true;
            }
            kind::FOPT | kind::SECONDARY_FOPT | kind::TERTIARY_FOPT => {
                properties(record.instance, record.data, &mut shape)
            }
            _ => {}
        }
    }
    found.then_some(shape)
}

/// The property table: a run of six-byte entries, then the variable-length
/// data of whichever of them said they were too big to fit in four bytes.
///
/// **How many entries there are is in the record header and nowhere else.**
/// The overflow follows the entries with nothing between, so a reader that
/// walks to the end of the record reads the first six bytes of somebody's
/// string as a property and assigns whatever it decodes.
fn properties(count: u16, table: &[u8], shape: &mut Shape) {
    let mut entries = Vec::new();
    let mut at = 0usize;
    for _ in 0..count {
        if at + 6 > table.len() {
            break;
        }
        let opid = read_u16(table, at);
        entries.push((opid & 0x3FFF, opid & 0x8000 != 0, read_u32(table, at + 2)));
        at += 6;
    }
    let mut complex = at;
    for (id, is_complex, value) in entries {
        let bytes = match is_complex {
            false => None,
            true => {
                let from = complex;
                complex = complex.saturating_add(value as usize);
                table.get(from..complex)
            }
        };
        match id {
            // `rotation`, a signed 16.16 fixed-point number of degrees.
            0x0004 => shape.rotation = value as i32 as f64 / 65_536.0,
            // `gtextUNICODE` and `gtextFont`, both UTF-16 with a terminator.
            0x00C0 => shape.text = bytes.map(utf16),
            0x00C5 => shape.font = bytes.map(utf16),
            // `geometryTextBooleanProperties`. Only `fGtextFStretch`, bit 3 of
            // the second byte, is read, and the mask in the high half is not
            // consulted: Word's own WordArt sets the bit without marking it
            // used, and a reader that believed the mask would draw the one
            // kind of shape that *is* stretched as if it were not.
            0x00FF => shape.stretch = value & 0x0080 != 0,
            // `pib`. Zero is "no picture", which is not picture zero.
            0x0104 => shape.picture = (value != 0).then_some(value),
            0x0181 => shape.fill = colour(value),
            // `fillOpacity`, a 16.16 fixed-point fraction.
            0x0182 => shape.opacity = (value as f64 / 65_536.0).clamp(0.0, 1.0),
            0x01C0 => shape.line = colour(value).unwrap_or(shape.line),
            0x01CB => shape.line_width = value as i32,
            // `lineStyleBooleans`: `fLine` is bit 3, and the high half says
            // whether the shape stated it at all.
            0x01FF => {
                if value & 0x0008_0000 != 0 {
                    shape.lined = value & 0x0008 != 0;
                }
            }
            // `fillStyleBooleans`: the low half is the flags, the high half
            // says which of them the shape actually stated.
            0x01BF => {
                if value & 0x0010_0000 != 0 {
                    shape.filled = value & 0x0010 != 0;
                }
            }
            0x0380 => shape.name = bytes.map(utf16),
            0x038F => at_mut(&mut shape.horizontal).align = value as u8,
            0x0390 => at_mut(&mut shape.horizontal).relative_to = value as u8,
            0x0391 => at_mut(&mut shape.vertical).align = value as u8,
            0x0392 => at_mut(&mut shape.vertical).relative_to = value as u8,
            _ => {}
        }
    }
}

/// One axis of a position, made if the shape has not stated one yet. The four
/// properties that describe it arrive separately and in no fixed order.
fn at_mut(slot: &mut Option<Placement>) -> &mut Placement {
    slot.get_or_insert(Placement {
        align: 0,
        relative_to: 0,
    })
}

/// An `OfficeArtCOLORREF`. Only a literal colour is taken: the same four bytes
/// can be an index into a scheme or a palette this has no copy of, and drawing
/// those as if they were red-green-blue is worse than not drawing them.
fn colour(value: u32) -> Option<wp_model::Color> {
    let [red, green, blue, flags] = value.to_le_bytes();
    (flags == 0x00).then_some(wp_model::Color::Rgb([red, green, blue]))
}

/// A complex property's UTF-16 string, without the terminator the file writes
/// and Word does not show.
fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Where one floating shape sits, and which shape it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    /// The character position of the `\x08` in the text.
    pub cp: u32,
    /// `Spa.lid` — the shape's identifier.
    pub spid: u32,
    /// The rectangle the shape covers, in twips, measured from whatever
    /// `horizontal`/`vertical` name.
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    /// `bx`: 0 the page's leading margin, 1 the page's edge, 2 the column.
    pub horizontal: u8,
    /// `by`: 0 the top margin, 1 the top of the page, 2 the paragraph.
    pub vertical: u8,
    /// `wr` — 0 wrap round, 1 above and below, 2 square, 3 none at all,
    /// 4 tight, 5 through.
    pub wrap: u8,
    /// `fBelowText`, which only means anything when `wrap` is 3. A watermark
    /// is exactly this: no wrapping, and behind the words.
    pub below_text: bool,
}

/// The anchors of one part of the document: a `PlcfSpa`, which is *n*+1
/// character positions followed by *n* twenty-six byte entries.
pub fn anchors(fib: &Fib, table: &[u8], field: usize) -> Vec<Anchor> {
    let Some(plc) = fib.slice(table, field) else {
        return Vec::new();
    };
    if plc.len() < 4 + SPA {
        return Vec::new();
    }
    let count = (plc.len() - 4) / (4 + SPA);
    let base = (count + 1) * 4;
    (0..count)
        .map(|index| anchor(plc, index * 4, base + index * SPA))
        .collect()
}

/// One `Spa`, and the character position that goes with it.
const SPA: usize = 26;

fn anchor(plc: &[u8], cp_at: usize, at: usize) -> Anchor {
    let flags = read_u16(plc, at + 20);
    Anchor {
        cp: read_u32(plc, cp_at),
        spid: read_u32(plc, at),
        left: read_u32(plc, at + 4) as i32,
        top: read_u32(plc, at + 8) as i32,
        right: read_u32(plc, at + 12) as i32,
        bottom: read_u32(plc, at + 16) as i32,
        horizontal: ((flags >> 1) & 0x03) as u8,
        vertical: ((flags >> 3) & 0x03) as u8,
        wrap: ((flags >> 5) & 0x0F) as u8,
        below_text: flags & 0x4000 != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An OfficeArt record: the packed version and instance, the type, the
    /// length, and the body.
    fn record(instance: u16, kind: u16, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(((instance << 4) | 0x0F).to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn a_record_says_how_long_it_is_and_the_next_one_starts_after_it() {
        let mut data = record(0, 0xF00A, &[1, 2, 3, 4]);
        data.extend(record(0, 0xF00B, &[5, 6]));
        let found = records(&data);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].kind, 0xF00A);
        assert_eq!(found[1].data, &[5, 6]);
    }

    #[test]
    fn a_record_that_runs_past_the_end_is_dropped_rather_than_clamped() {
        // Half a record is a damaged file, and reading it produces confident
        // nonsense rather than an error.
        let mut data = record(0, 0xF00A, &[1, 2, 3, 4]);
        data.truncate(data.len() - 2);
        assert!(records(&data).is_empty());
    }

    #[test]
    fn a_png_blip_is_the_bytes_after_the_digest_and_the_tag() {
        let mut body = vec![0xAAu8; 16]; // rgbUid1
        body.push(0xFF); // tag
        body.extend(b"\x89PNG");
        let data = record(0x6E0, 0xF01E, &body);
        let found = blip_of(records(&data)[0]).expect("a decodable picture");
        assert_eq!(found.content_type, "image/png");
        assert_eq!(&found.data, b"\x89PNG");
    }

    #[test]
    fn a_metafile_blip_is_inflated_rather_than_handed_over_compressed() {
        // The bytes behind an `OfficeArtMetafileHeader` are deflated, and a
        // reader that hands them on as they are gives a player a recording it
        // cannot possibly play.
        let recording = b"a metafile, or near enough for a test";
        let mut body = vec![0xAAu8; 16]; // rgbUid1
        body.extend((recording.len() as u32).to_le_bytes()); // cbSize
        body.extend([0u8; 16]); // rcBounds
        body.extend([0u8; 8]); // ptSize
        let squeezed = {
            let mut out = Vec::new();
            let mut writer =
                flate2::write::ZlibEncoder::new(&mut out, flate2::Compression::default());
            std::io::Write::write_all(&mut writer, recording).expect("squeezed");
            writer.finish().expect("finished");
            out
        };
        body.extend((squeezed.len() as u32).to_le_bytes()); // cbSave
        body.push(0x00); // deflate
        body.push(0xFE); // no filter
        body.extend(&squeezed);

        let data = record(0x3D4, 0xF01A, &body);
        let found = blip_of(records(&data)[0]).expect("a metafile");
        assert_eq!(found.content_type, "image/x-emf");
        assert_eq!(found.data, recording);
    }

    #[test]
    fn an_odd_instance_means_two_digests_rather_than_one() {
        // Getting this wrong hands the decoder sixteen bytes of somebody's MD4
        // digest and calls it a JPEG.
        let mut body = vec![0xAAu8; 32];
        body.push(0xFF);
        body.extend([0xFF, 0xD8, 0xFF]);
        let data = record(0x46B, 0xF01D, &body);
        let found = blip_of(records(&data)[0]).expect("a decodable picture");
        assert_eq!(&found.data, &[0xFF, 0xD8, 0xFF]);
    }

    #[test]
    fn a_shapes_wordart_text_comes_out_of_the_property_tables_overflow() {
        // The entries are fixed width and their strings follow them in order,
        // so a reader that takes the first six bytes as the string gets an
        // opcode and a length instead of a word.
        let mut table = Vec::new();
        // `rotation`, plain, 315 degrees.
        table.extend(0x0004u16.to_le_bytes());
        table.extend((315i32 * 65_536).to_le_bytes());
        // `gtextUNICODE`, complex, eight bytes of it.
        table.extend((0x00C0u16 | 0x8000).to_le_bytes());
        table.extend(8u32.to_le_bytes());
        table.extend([0x44, 0x00, 0x52, 0x00, 0x41, 0x00, 0x00, 0x00]);

        let mut shape = Shape::default();
        properties(2, &table, &mut shape);
        assert_eq!(shape.text.as_deref(), Some("DRA"));
        assert_eq!(shape.rotation, 315.0);
    }

    #[test]
    fn a_half_transparent_fill_is_read_as_the_half_it_is() {
        // Word's own watermark states `fillOpacity` as a 16.16 fraction, and a
        // shape that states nothing is solid.
        let read = |value: u32| {
            let mut shape = Shape::default();
            let mut table = Vec::new();
            table.extend_from_slice(&0x0182u16.to_le_bytes());
            table.extend_from_slice(&value.to_le_bytes());
            properties(1, &table, &mut shape);
            shape.opacity
        };
        assert_eq!(read(0x0000_8000), 0.5);
        assert_eq!(read(0x0001_0000), 1.0);
        assert_eq!(Shape::default().opacity, 1.0);
    }

    #[test]
    fn a_watermark_says_its_words_are_not_to_be_stretched_and_word_art_says_they_are() {
        // The two values are the ones the two shapes actually carry, read off
        // a `.doc` Word wrote: its diagonal watermark and a piece of WordArt
        // inserted from the gallery. They differ in exactly one bit — bit 3 of
        // the second byte, `fGtextFStretch` — and Word draws the second with
        // its letters pulled about to fill the box and the first without.
        //
        // The high half is the mask of which booleans the shape claims to have
        // stated, and it is not consulted: the WordArt's mask leaves the
        // stretch bit out while its value sets it, so a reader that believed
        // the mask would draw the one shape that *is* stretched flat.
        let table = |value: u32| {
            let mut out = Vec::new();
            out.extend(0x00FFu16.to_le_bytes());
            out.extend(value.to_le_bytes());
            out
        };
        let read = |value: u32| {
            let mut shape = Shape::default();
            properties(1, &table(value), &mut shape);
            shape.stretch
        };
        assert!(!read(0xC086_0000), "the watermark's own value");
        assert!(read(0x5700_0080), "and the gallery WordArt's");
        assert!(!Shape::default().stretch, "a shape that says nothing");
    }

    #[test]
    fn an_anchor_names_a_shape_and_the_rectangle_it_covers() {
        let mut plc = Vec::new();
        plc.extend(100u32.to_le_bytes()); // the anchor's character position
        plc.extend(200u32.to_le_bytes()); // and the end of the range
        plc.extend(1025u32.to_le_bytes()); // lid
        for value in [10i32, 20, 1450, 900] {
            plc.extend(value.to_le_bytes());
        }
        // wr = 3 (no wrapping), fBelowText — a watermark's own flags.
        plc.extend((0x4000u16 | (3 << 5)).to_le_bytes());
        plc.extend(0u32.to_le_bytes());

        let anchor = anchor(&plc, 0, 8);
        assert_eq!(anchor.cp, 100);
        assert_eq!(anchor.spid, 1025);
        assert_eq!((anchor.left, anchor.bottom), (10, 900));
        assert_eq!(anchor.wrap, 3);
        assert!(anchor.below_text, "and it is behind the words");
    }
}
