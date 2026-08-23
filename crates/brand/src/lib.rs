//! The applications' icons, drawn in code.
//!
//! Nothing in the suite is bundled — not a font, not a bitmap — so the icon
//! is rasterised: a rounded tile in the application's colour with a white
//! glyph on it. The two applications are told apart first by colour and then
//! by glyph, because a taskbar button is a thumbnail-sized thing seen at a
//! glance, and at that size two identical tiles with different lines on them
//! are the same tile.
//!
//! The same picture serves twice: the window hands it to the windowing system
//! at start-up, which is what the taskbar and the title bar show while the
//! application runs, and the build script writes it into the executable as
//! an icon resource, which is what Explorer, a shortcut and "Open with" show
//! when it is not. This crate has no dependencies so that the build script can
//! afford it.

/// Which application's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    Calx,
    Scriva,
}

impl App {
    /// The application by the slug its binary is named with.
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "calx" => Some(App::Calx),
            "scriva" => Some(App::Scriva),
            _ => None,
        }
    }

    /// The tile colour behind the glyph.
    ///
    /// Calx takes the suite's green. Scriva takes a blue of the same weight
    /// so that the two sit together as one family and apart as two
    /// applications.
    pub fn tile(self) -> [u8; 3] {
        match self {
            App::Calx => [0x1E, 0x6F, 0x5C],
            App::Scriva => [0x2F, 0x5D, 0x8A],
        }
    }
}

/// The icon at `size` pixels square, as RGBA rows top-down.
pub fn rgba(app: App, size: usize) -> Vec<u8> {
    // Supersampled, or the tile's corners and the glyph's edges are
    // staircases at the sizes a taskbar and a file listing show them.
    const SUB: usize = 4;
    let tile = app.tile();
    let mut out = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0u32; 4];
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let px = (x as f32 + (sx as f32 + 0.5) / SUB as f32) / size as f32;
                    let py = (y as f32 + (sy as f32 + 0.5) / SUB as f32) / size as f32;
                    for (acc, v) in sum.iter_mut().zip(shade(app, tile, px, py)) {
                        *acc += u32::from(v);
                    }
                }
            }
            for v in sum {
                out.push((v / (SUB * SUB) as u32) as u8);
            }
        }
    }
    out
}

/// The sizes an executable's icon is asked for: the small and large icons of
/// every scale Windows runs at, and the big one a jump list or a tile shows.
pub const ICO_SIZES: [usize; 6] = [16, 24, 32, 48, 64, 256];

/// The icon as a Windows `.ico` file holding every size in [`ICO_SIZES`].
///
/// Each image is a 32-bit bitmap with its alpha in the pixels, which is what
/// every Windows since XP reads; the AND mask that older readers want is
/// written empty, as it has to be present to be skipped.
pub fn ico(app: App) -> Vec<u8> {
    let images: Vec<(usize, Vec<u8>)> = ICO_SIZES
        .iter()
        .map(|&size| (size, bmp_entry(&rgba(app, size), size)))
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // icon, not cursor
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, data) in &images {
        // A 256-pixel image states its size as 0: the field is a byte.
        let side = if *size >= 256 { 0u8 } else { *size as u8 };
        out.push(side);
        out.push(side);
        out.push(0); // no palette
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

/// One icon image as the bitmap an `.ico` embeds: a header that states twice
/// the height (the colour rows and the mask rows are one picture to it), the
/// colour rows bottom-up in BGRA, then the empty mask.
fn bmp_entry(rgba: &[u8], size: usize) -> Vec<u8> {
    let mask_row = (size.div_ceil(32)) * 4;
    let mut out = Vec::with_capacity(40 + size * size * 4 + mask_row * size);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // no compression
    out.extend_from_slice(&((size * size * 4 + mask_row * size) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // resolution, palette: unused
    for y in (0..size).rev() {
        for x in 0..size {
            let p = &rgba[(y * size + x) * 4..][..4];
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    out.resize(out.len() + mask_row * size, 0);
    out
}

/// The colour at a point of the unit square: transparent outside the tile,
/// the tile colour inside it, white where the glyph is.
fn shade(app: App, tile: [u8; 3], x: f32, y: f32) -> [u8; 4] {
    if !rounded(x, y, 0.03, 0.97, 0.2) {
        return [0, 0, 0, 0];
    }
    let on_glyph = match app {
        App::Calx => grid(x, y),
        App::Scriva => page(x, y),
    };
    if on_glyph {
        [0xFF, 0xFF, 0xFF, 0xFF]
    } else {
        [tile[0], tile[1], tile[2], 0xFF]
    }
}

/// Inside the square from `lo` to `hi` with corners of radius `r`.
fn rounded(x: f32, y: f32, lo: f32, hi: f32, r: f32) -> bool {
    if !(lo..=hi).contains(&x) || !(lo..=hi).contains(&y) {
        return false;
    }
    let cx = x.clamp(lo + r, hi - r);
    let cy = y.clamp(lo + r, hi - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// Calx: a sheet of cells, three by three, as lines rather than boxes.
fn grid(x: f32, y: f32) -> bool {
    const LO: f32 = 0.24;
    const HI: f32 = 0.76;
    const LINE: f32 = 0.05;
    if !(LO..=HI).contains(&x) || !(LO..=HI).contains(&y) {
        return false;
    }
    let near = |v: f32| {
        [LO, (2.0 * LO + HI) / 3.0, (LO + 2.0 * HI) / 3.0, HI]
            .iter()
            .any(|edge| (v - edge).abs() <= LINE / 2.0)
    };
    near(x) || near(y)
}

/// Scriva: a page with its top corner turned, and three lines of text.
fn page(x: f32, y: f32) -> bool {
    const LEFT: f32 = 0.30;
    const RIGHT: f32 = 0.70;
    const TOP: f32 = 0.20;
    const BOTTOM: f32 = 0.80;
    const FOLD: f32 = 0.14;
    if !(LEFT..=RIGHT).contains(&x) || !(TOP..=BOTTOM).contains(&y) {
        return false;
    }
    // The corner that is turned down is cut off the page.
    if (x - (RIGHT - FOLD)) + (TOP + FOLD - y) > FOLD {
        return false;
    }
    // Text lines are the page showing through in the tile's colour.
    let lines = [0.44, 0.55, 0.66];
    let on_line =
        lines.iter().any(|ly| (y - ly).abs() <= 0.025) && (LEFT + 0.08..=RIGHT - 0.08).contains(&x);
    !on_line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(app: App, size: usize) -> Vec<[u8; 4]> {
        rgba(app, size)
            .chunks(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect()
    }

    #[test]
    fn each_application_has_its_own_icon_with_a_tile_and_a_glyph() {
        let calx = pixels(App::Calx, 128);
        let scriva = pixels(App::Scriva, 128);
        assert_ne!(calx, scriva, "one icon for two applications");
        for (app, px) in [(App::Calx, &calx), (App::Scriva, &scriva)] {
            let tile = app.tile();
            let tiled = px.iter().filter(|p| p[..3] == tile && p[3] == 0xFF).count();
            let white = px.iter().filter(|p| **p == [0xFF; 4]).count();
            let clear = px.iter().filter(|p| p[3] == 0).count();
            assert!(tiled > px.len() / 3, "{app:?}: the tile is most of it");
            assert!(white > px.len() / 40, "{app:?}: the glyph is seen");
            assert!(clear > 0, "{app:?}: the corners are rounded");
        }
        // Told apart by colour before glyph: the tiles differ.
        assert_ne!(calx[64 * 128 + 8], scriva[64 * 128 + 8]);
    }

    #[test]
    fn the_glyph_survives_the_smallest_size() {
        for app in [App::Calx, App::Scriva] {
            let px = pixels(app, 16);
            let white = px.iter().filter(|p| p[0] > 0xC0 && p[3] == 0xFF).count();
            assert!(white >= 8, "{app:?} at 16px: {white} light pixels");
        }
    }

    #[test]
    fn the_ico_is_laid_out_as_windows_reads_it() {
        let ico = ico(App::Calx);
        let u16_at = |i: usize| u16::from_le_bytes([ico[i], ico[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([ico[i], ico[i + 1], ico[i + 2], ico[i + 3]]);
        assert_eq!(u16_at(2), 1, "type: icon");
        assert_eq!(usize::from(u16_at(4)), ICO_SIZES.len());
        let mut expected_offset = 6 + 16 * ICO_SIZES.len();
        for (n, size) in ICO_SIZES.iter().enumerate() {
            let entry = 6 + 16 * n;
            let side = if *size >= 256 { 0 } else { *size as u8 };
            assert_eq!((ico[entry], ico[entry + 1]), (side, side));
            assert_eq!(u16_at(entry + 6), 32, "bits per pixel");
            let bytes = u32_at(entry + 8) as usize;
            let offset = u32_at(entry + 12) as usize;
            assert_eq!(offset, expected_offset, "image {n} follows the last");
            assert_eq!(u32_at(offset), 40, "a bitmap header where the image starts");
            assert_eq!(u32_at(offset + 8) as usize, size * 2, "twice the height");
            expected_offset += bytes;
        }
        assert_eq!(ico.len(), expected_offset, "nothing after the last image");
    }
}
