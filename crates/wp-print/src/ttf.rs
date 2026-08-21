//! The little of TrueType a PDF embedder has to read.
//!
//! Embedding a font in a PDF does not require understanding it — the file goes
//! in whole, as `FontFile2`, and the *viewer* rasterises it. What the embedder
//! must supply alongside are the numbers PDF wants stated up front: which glyph
//! each character is (the text operators name glyphs, not characters), how wide
//! each glyph is (viewers are told the widths so text selection works without
//! opening the font), and the descriptor's vertical metrics.
//!
//! So this reads exactly four questions' worth of tables — `cmap`, `hmtx`,
//! `head`/`hhea`, `OS/2`/`post` — and nothing else. No glyf, no hinting, no
//! kerning: layout already happened, and the placement of every fragment
//! arrives from the layout engine to the point.

/// One parsed face, holding the tables it answers from.
pub struct Face<'a> {
    data: &'a [u8],
    units_per_em: u16,
    cmap: Option<Cmap<'a>>,
    hmtx: &'a [u8],
    number_of_h_metrics: u16,
    /// From `hhea`, in font units.
    pub ascent: i16,
    pub descent: i16,
    pub line_gap: i16,
    /// From `OS/2` when it says, else the ascent — a resume viewer never
    /// notices, only a text-selection rectangle does.
    pub cap_height: i16,
    /// From `post`, whole degrees are enough.
    pub italic_angle: f64,
    /// From `head`: the glyph bounding box, in font units.
    pub bbox: [i16; 4],
    /// From `OS/2`: the licence bits, kept raw.
    fs_type: u16,
}

/// The character-to-glyph table, restricted to the two formats every Windows
/// font actually carries.
enum Cmap<'a> {
    Format4(&'a [u8]),
    Format12(&'a [u8]),
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]))
}

fn i16_at(data: &[u8], at: usize) -> Option<i16> {
    u16_at(data, at).map(|v| v as i16)
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *data.get(at)?,
        *data.get(at + 1)?,
        *data.get(at + 2)?,
        *data.get(at + 3)?,
    ]))
}

impl<'a> Face<'a> {
    /// `None` for anything that is not a single TrueType face — a collection,
    /// a CFF OpenType, a WOFF. The caller falls back to a base-14 font rather
    /// than embedding something a viewer would reject.
    pub fn parse(data: &'a [u8]) -> Option<Face<'a>> {
        let tag = u32_at(data, 0)?;
        // 'true' and 1.0 are TrueType outlines; 'OTTO' is CFF and its glyph
        // widths live elsewhere.
        if tag != 0x0001_0000 && tag != u32::from_be_bytes(*b"true") {
            return None;
        }
        let count = u16_at(data, 4)? as usize;
        let table = |name: &[u8; 4]| -> Option<&'a [u8]> {
            for index in 0..count {
                let entry = 12 + index * 16;
                if data.get(entry..entry + 4)? == name {
                    let offset = u32_at(data, entry + 8)? as usize;
                    let length = u32_at(data, entry + 12)? as usize;
                    return data.get(offset..offset.checked_add(length)?);
                }
            }
            None
        };

        let head = table(b"head")?;
        let hhea = table(b"hhea")?;
        let hmtx = table(b"hmtx")?;
        let units_per_em = u16_at(head, 18)?;
        let ascent = i16_at(hhea, 4)?;
        let descent = i16_at(hhea, 6)?;
        let line_gap = i16_at(hhea, 8)?;
        let number_of_h_metrics = u16_at(hhea, 34)?;
        let bbox = [
            i16_at(head, 36)?,
            i16_at(head, 38)?,
            i16_at(head, 40)?,
            i16_at(head, 42)?,
        ];

        let cap_height = table(b"OS/2")
            .filter(|os2| u16_at(os2, 0).is_some_and(|version| version >= 2))
            .and_then(|os2| i16_at(os2, 88))
            .filter(|height| *height > 0)
            .unwrap_or(ascent);
        // A face with no `OS/2` table predates the licence bits; zero is the
        // spec's own reading of that — installable, nothing withheld.
        let fs_type = table(b"OS/2").and_then(|os2| u16_at(os2, 8)).unwrap_or(0);
        let italic_angle = table(b"post")
            .and_then(|post| u32_at(post, 4))
            .map(|fixed| fixed as i32 as f64 / 65536.0)
            .unwrap_or(0.0);

        let cmap = table(b"cmap").and_then(pick_subtable);

        Some(Face {
            data,
            units_per_em,
            cmap,
            hmtx,
            number_of_h_metrics,
            ascent,
            descent,
            line_gap,
            cap_height,
            italic_angle,
            bbox,
            fs_type,
        })
    }

    /// Whether the face's licence lets it travel inside a document.
    ///
    /// `fsType` is a licence statement, not a capability: 0x0002 alone means
    /// the font must not leave the machine it is installed on. When a font
    /// says several things at once the spec resolves to the least restrictive,
    /// so preview-and-print (0x0004) or editable (0x0008) embedding overrides
    /// restricted — which is also how Word reads it when it embeds fonts. The
    /// no-subsetting bit needs no answer here: the file is embedded whole,
    /// never subset.
    pub fn embeddable(&self) -> bool {
        self.fs_type & 0x000C != 0 || self.fs_type & 0x0002 == 0
    }

    pub fn units_per_em(&self) -> f64 {
        // Zero would divide; no real font says it, but a truncated one might.
        f64::from(self.units_per_em.max(1))
    }

    /// The whole file, for `FontFile2`.
    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }

    /// The glyph a character maps to — 0, the missing-glyph box, when the face
    /// has no idea.
    pub fn glyph(&self, c: char) -> u16 {
        let point = c as u32;
        match &self.cmap {
            Some(Cmap::Format4(sub)) => glyph_format4(sub, point)
                // Symbol-encoded faces (Symbol, Wingdings) map their glyphs at
                // 0xF000-0xF0FF; a document's bullet arrives as U+2022 or as
                // the bare F0B7, and the F000 window answers for the latter.
                .or_else(|| glyph_format4(sub, 0xF000 + (point & 0xFF)))
                .unwrap_or(0),
            Some(Cmap::Format12(sub)) => glyph_format12(sub, point).unwrap_or(0),
            None => 0,
        }
    }

    /// The advance of a glyph, in font units.
    pub fn advance(&self, glyph: u16) -> u16 {
        let count = self.number_of_h_metrics.max(1);
        // Past the last entry, every glyph repeats the last stated advance —
        // that is what the table's own compression scheme means.
        let index = glyph.min(count - 1) as usize;
        u16_at(self.hmtx, index * 4).unwrap_or(0)
    }
}

/// Chooses the subtable the way Windows does: Unicode full, then Unicode BMP,
/// then the symbol encoding.
fn pick_subtable(cmap: &[u8]) -> Option<Cmap<'_>> {
    let count = u16_at(cmap, 2)? as usize;
    let mut best: Option<(u8, usize)> = None;
    for index in 0..count {
        let entry = 4 + index * 8;
        let platform = u16_at(cmap, entry)?;
        let encoding = u16_at(cmap, entry + 2)?;
        let offset = u32_at(cmap, entry + 4)? as usize;
        let rank = match (platform, encoding) {
            (3, 10) | (0, 4) | (0, 6) => 3,
            (3, 1) | (0, 3) | (0, 2) | (0, 1) | (0, 0) => 2,
            (3, 0) => 1,
            _ => 0,
        };
        if rank > 0 && best.map(|(r, _)| rank > r).unwrap_or(true) {
            best = Some((rank, offset));
        }
    }
    let (_, offset) = best?;
    let sub = cmap.get(offset..)?;
    match u16_at(sub, 0)? {
        4 => Some(Cmap::Format4(sub)),
        12 => Some(Cmap::Format12(sub)),
        _ => None,
    }
}

fn glyph_format4(sub: &[u8], point: u32) -> Option<u16> {
    if point > 0xFFFF {
        return None;
    }
    let point = point as u16;
    let seg_count_x2 = u16_at(sub, 6)? as usize;
    let ends = 14;
    let starts = ends + seg_count_x2 + 2;
    let deltas = starts + seg_count_x2;
    let ranges = deltas + seg_count_x2;
    for segment in (0..seg_count_x2).step_by(2) {
        let end = u16_at(sub, ends + segment)?;
        if point > end {
            continue;
        }
        let start = u16_at(sub, starts + segment)?;
        if point < start {
            return None;
        }
        let delta = u16_at(sub, deltas + segment)?;
        let range_offset = u16_at(sub, ranges + segment)?;
        if range_offset == 0 {
            return Some(point.wrapping_add(delta));
        }
        // The famous self-relative pointer: the offset counts from its own
        // position in the file.
        let at = ranges + segment + range_offset as usize + 2 * (point - start) as usize;
        let glyph = u16_at(sub, at)?;
        if glyph == 0 {
            return None;
        }
        return Some(glyph.wrapping_add(delta));
    }
    None
}

fn glyph_format12(sub: &[u8], point: u32) -> Option<u16> {
    let groups = u32_at(sub, 12)? as usize;
    let mut low = 0usize;
    let mut high = groups;
    while low < high {
        let middle = (low + high) / 2;
        let entry = 16 + middle * 12;
        let start = u32_at(sub, entry)?;
        let end = u32_at(sub, entry + 4)?;
        if point < start {
            high = middle;
        } else if point > end {
            low = middle + 1;
        } else {
            let first_glyph = u32_at(sub, entry + 8)?;
            return u16::try_from(first_glyph + (point - start)).ok();
        }
    }
    None
}

/// The least TrueType `parse` accepts, with the licence field under the
/// caller's control — what the embedding-permission tests are made of.
#[cfg(test)]
pub(crate) mod fixture {
    pub fn face(fs_type: u16) -> Vec<u8> {
        let mut os2 = vec![0u8; 78];
        os2[8..10].copy_from_slice(&fs_type.to_be_bytes());

        let mut head = vec![0u8; 54];
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[38..40].copy_from_slice(&(-200i16).to_be_bytes());
        head[40..42].copy_from_slice(&500i16.to_be_bytes());
        head[42..44].copy_from_slice(&800i16.to_be_bytes());

        let mut hhea = vec![0u8; 36];
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[34..36].copy_from_slice(&1u16.to_be_bytes());

        let hmtx = vec![0x01, 0xF4, 0, 0];

        let tables: [(&[u8; 4], &[u8]); 4] = [
            (b"OS/2", &os2),
            (b"head", &head),
            (b"hhea", &hhea),
            (b"hmtx", &hmtx),
        ];
        let mut data = Vec::new();
        data.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        data.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        data.extend_from_slice(&[0, 64, 0, 2, 0, 0]);
        let mut offset = 12 + tables.len() * 16;
        for (tag, table) in &tables {
            data.extend_from_slice(*tag);
            data.extend_from_slice(&[0; 4]);
            data.extend_from_slice(&(offset as u32).to_be_bytes());
            data.extend_from_slice(&(table.len() as u32).to_be_bytes());
            offset += table.len();
        }
        for (_, table) in &tables {
            data.extend_from_slice(table);
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A face this machine certainly has, or the test has nothing to say.
    fn arial() -> Option<Vec<u8>> {
        let windir = std::env::var_os("SystemRoot")?;
        std::fs::read(std::path::PathBuf::from(windir).join("Fonts/arial.ttf")).ok()
    }

    #[test]
    fn a_real_face_answers_the_four_questions() {
        let Some(bytes) = arial() else { return };
        let face = Face::parse(&bytes).expect("Arial is a TrueType face");
        assert!(face.units_per_em() >= 1000.0);
        assert!(face.ascent > 0);
        assert!(face.descent < 0, "hhea descent is negative");
        let a = face.glyph('A');
        assert_ne!(a, 0, "Arial knows the letter A");
        assert_ne!(face.glyph('B'), a, "and B is a different glyph");
        assert!(face.advance(a) > 0);
        // A space is narrower than an M, in any face anyone ships.
        assert!(face.advance(face.glyph(' ')) < face.advance(face.glyph('M')));
    }

    #[test]
    fn a_character_no_face_has_maps_to_the_missing_glyph() {
        let Some(bytes) = arial() else { return };
        let face = Face::parse(&bytes).expect("Arial parses");
        assert_eq!(face.glyph('\u{E837}'), 0, "private use area is empty");
    }

    #[test]
    fn the_licence_bits_decide_embedding() {
        let says = |fs_type: u16| {
            let bytes = fixture::face(fs_type);
            Face::parse(&bytes)
                .expect("the fixture parses")
                .embeddable()
        };
        assert!(says(0x0000), "installable");
        assert!(!says(0x0002), "restricted");
        assert!(says(0x0004), "preview and print");
        assert!(says(0x0008), "editable");
        // Conflicting bits resolve to the least restrictive, per the spec.
        assert!(says(0x0006), "restricted, but preview granted");
        // No-subsetting binds subsetters; this embedder never subsets.
        assert!(says(0x0100), "whole-file only");
    }

    #[test]
    fn garbage_is_refused_rather_than_read() {
        assert!(Face::parse(b"not a font at all").is_none());
        assert!(Face::parse(b"OTTO\x00\x04rest").is_none(), "CFF is refused");
        assert!(
            Face::parse(b"ttcf\x00\x02more").is_none(),
            "so is a collection"
        );
        assert!(Face::parse(&[]).is_none());
    }
}
