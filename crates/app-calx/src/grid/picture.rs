//! Anchored pictures: decoding them, drawing them, and handling them.
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
//!
//! Geometry is done in **sheet space** — pixels from the top-left of A1, before
//! any scrolling — because that is the only frame of reference a drag can be
//! measured in that does not move underneath it.

use std::collections::HashMap;

use ss_model::chart::{Anchor, AnchorPoint, EMU_PER_POINT};
use ss_model::Picture;
use ui_kit::egui;

use super::Layout;

/// How big a selection handle is drawn, and how far around it a press counts.
const HANDLE: f32 = 4.5;
const GRAB: f32 = 7.0;
/// A picture may not be dragged smaller than this, in sheet pixels. Zero would
/// make it invisible and therefore unrecoverable without an undo.
const MIN_SIZE: f32 = 8.0;

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
            let key = key(picture);
            if self.loaded.contains_key(&key) {
                continue;
            }
            let handle = decode(&picture.data)
                .map(|image| ctx.load_texture(&key, image, egui::TextureOptions::LINEAR));
            self.loaded.insert(key, handle);
        }
    }

    pub fn get(&self, part: &str) -> Option<&egui::TextureHandle> {
        self.loaded.get(part).and_then(|slot| slot.as_ref())
    }
}

/// What a picture's texture is cached under.
///
/// The package part, which is where two anchors sharing one image — a logo
/// repeated on every sheet — decode once. A picture the user has just inserted
/// has no part until the workbook is saved, so it is keyed on the address of
/// its bytes instead: two new pictures would otherwise share the empty name
/// and the second would be drawn as the first.
fn key(picture: &Picture) -> String {
    if picture.part.is_empty() {
        format!("unsaved:{:p}", std::sync::Arc::as_ptr(&picture.data))
    } else {
        picture.part.clone()
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
/// The image fills its anchor rather than being fitted inside it, which is what
/// Excel does and what the handles promise: drag the middle handle out and the
/// picture must stretch, not float at its own proportions in a wider box.
pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    picture: &Picture,
    textures: &Textures,
    outline: egui::Color32,
) {
    let Some(texture) = textures.get(&key(picture)) else {
        // Something is anchored here and we cannot show it. A box says so;
        // nothing at all would read as an empty sheet.
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
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

/// Which part of a selected picture a press landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

impl Handle {
    /// Clockwise from the top left, which is the order Excel draws them in.
    pub const ALL: [Handle; 8] = [
        Handle::NorthWest,
        Handle::North,
        Handle::NorthEast,
        Handle::East,
        Handle::SouthEast,
        Handle::South,
        Handle::SouthWest,
        Handle::West,
    ];

    pub fn at(self, rect: egui::Rect) -> egui::Pos2 {
        let mid = rect.center();
        match self {
            Handle::NorthWest => rect.left_top(),
            Handle::North => egui::pos2(mid.x, rect.top()),
            Handle::NorthEast => rect.right_top(),
            Handle::East => egui::pos2(rect.right(), mid.y),
            Handle::SouthEast => rect.right_bottom(),
            Handle::South => egui::pos2(mid.x, rect.bottom()),
            Handle::SouthWest => rect.left_bottom(),
            Handle::West => egui::pos2(rect.left(), mid.y),
        }
    }

    /// The rectangle that results from dragging this handle to `to`.
    ///
    /// Free in both directions, because that is what the user asked for: an
    /// edge handle moves one edge and a corner moves two, with no proportion
    /// kept. The opposite edges do not move at all.
    pub fn resize(self, start: egui::Rect, to: egui::Pos2) -> egui::Rect {
        let (mut left, mut top, mut right, mut bottom) =
            (start.left(), start.top(), start.right(), start.bottom());
        if matches!(self, Handle::NorthWest | Handle::West | Handle::SouthWest) {
            left = to.x.min(right - MIN_SIZE);
        }
        if matches!(self, Handle::NorthEast | Handle::East | Handle::SouthEast) {
            right = to.x.max(left + MIN_SIZE);
        }
        if matches!(self, Handle::NorthWest | Handle::North | Handle::NorthEast) {
            top = to.y.min(bottom - MIN_SIZE);
        }
        if matches!(self, Handle::SouthWest | Handle::South | Handle::SouthEast) {
            bottom = to.y.max(top + MIN_SIZE);
        }
        egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom))
    }

    pub fn cursor(self) -> egui::CursorIcon {
        match self {
            Handle::NorthWest | Handle::SouthEast => egui::CursorIcon::ResizeNwSe,
            Handle::NorthEast | Handle::SouthWest => egui::CursorIcon::ResizeNeSw,
            Handle::North | Handle::South => egui::CursorIcon::ResizeVertical,
            Handle::East | Handle::West => egui::CursorIcon::ResizeHorizontal,
        }
    }
}

/// The handle under `pos`, if any. `rect` is on screen, as is `pos`.
pub fn handle_at(rect: egui::Rect, pos: egui::Pos2) -> Option<Handle> {
    Handle::ALL.into_iter().find(|h| {
        egui::Rect::from_center_size(h.at(rect), egui::Vec2::splat(GRAB * 2.0)).contains(pos)
    })
}

/// Excel's selection chrome: a thin outline and eight round handles.
///
/// Handles are drawn *outside* the outline's corners rather than centred on the
/// picture's own edge pixels, so a logo on a white background does not swallow
/// them.
pub fn draw_selection(painter: &egui::Painter, rect: egui::Rect) {
    let edge = egui::Color32::from_gray(0x80);
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, edge),
        egui::StrokeKind::Outside,
    );
    for handle in Handle::ALL {
        painter.circle(
            handle.at(rect),
            HANDLE,
            egui::Color32::WHITE,
            egui::Stroke::new(1.0, edge),
        );
    }
}

/// Where an anchor sits in sheet space: pixels from the top-left of A1, before
/// any scrolling and before the headers are allowed for.
pub fn sheet_rect(layout: &Layout, anchor: &Anchor) -> egui::Rect {
    let corner = |col: u32, col_off: i64, row: u32, row_off: i64| {
        egui::pos2(
            (layout.cols.offset(col) + emu_to_pixels(col_off, layout.zoom)) as f32,
            (layout.rows.offset(row) + emu_to_pixels(row_off, layout.zoom)) as f32,
        )
    };
    match anchor {
        Anchor::TwoCell { from, to } => egui::Rect::from_two_pos(
            corner(from.col, from.col_offset, from.row, from.row_offset),
            corner(to.col, to.col_offset, to.row, to.row_offset),
        ),
        Anchor::OneCell {
            from,
            width,
            height,
        } => egui::Rect::from_min_size(
            corner(from.col, from.col_offset, from.row, from.row_offset),
            egui::vec2(
                emu_to_pixels(*width, layout.zoom) as f32,
                emu_to_pixels(*height, layout.zoom) as f32,
            ),
        ),
        Anchor::Absolute {
            x,
            y,
            width,
            height,
        } => egui::Rect::from_min_size(
            egui::pos2(
                emu_to_pixels(*x, layout.zoom) as f32,
                emu_to_pixels(*y, layout.zoom) as f32,
            ),
            egui::vec2(
                emu_to_pixels(*width, layout.zoom) as f32,
                emu_to_pixels(*height, layout.zoom) as f32,
            ),
        ),
    }
}

/// Turns a sheet-space rectangle back into an anchor of the same kind as `like`.
///
/// The kind is kept rather than normalised, because it is the file's own
/// statement about how the picture should behave when rows and columns change
/// size, and a drag says nothing about that.
pub fn anchor_of(layout: &Layout, rect: egui::Rect, like: &Anchor) -> Anchor {
    match like {
        Anchor::TwoCell { .. } => Anchor::TwoCell {
            from: point_at(layout, rect.min),
            to: point_at(layout, rect.max),
        },
        Anchor::OneCell { .. } => Anchor::OneCell {
            from: point_at(layout, rect.min),
            width: pixels_to_emu(f64::from(rect.width()), layout.zoom),
            height: pixels_to_emu(f64::from(rect.height()), layout.zoom),
        },
        Anchor::Absolute { .. } => Anchor::Absolute {
            x: pixels_to_emu(f64::from(rect.min.x), layout.zoom),
            y: pixels_to_emu(f64::from(rect.min.y), layout.zoom),
            width: pixels_to_emu(f64::from(rect.width()), layout.zoom),
            height: pixels_to_emu(f64::from(rect.height()), layout.zoom),
        },
    }
}

/// The cell a sheet-space position falls in, plus how far into it.
fn point_at(layout: &Layout, pos: egui::Pos2) -> AnchorPoint {
    let x = f64::from(pos.x).max(0.0);
    let y = f64::from(pos.y).max(0.0);
    let col = layout.cols.index_at(x);
    let row = layout.rows.index_at(y);
    AnchorPoint {
        col,
        col_offset: pixels_to_emu(x - layout.cols.offset(col), layout.zoom),
        row,
        row_offset: pixels_to_emu(y - layout.rows.offset(row), layout.zoom),
    }
}

/// EMUs are Office's internal unit: 12,700 to the point. The grid works in
/// points at 100% zoom, so this is the whole conversion.
fn emu_to_pixels(emu: i64, zoom: f64) -> f64 {
    emu as f64 / EMU_PER_POINT * zoom
}

fn pixels_to_emu(pixels: f64, zoom: f64) -> i64 {
    if zoom <= 0.0 {
        return 0;
    }
    (pixels / zoom * EMU_PER_POINT).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::Workbook;

    fn layout() -> Layout {
        let book = Workbook::blank();
        let sheet = book.sheet(0).expect("a sheet");
        Layout::for_sheet(&book, sheet, 1.0)
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

    #[test]
    fn a_rectangle_survives_the_trip_through_an_anchor_and_back() {
        let layout = layout();
        let original = egui::Rect::from_min_size(egui::pos2(137.0, 84.0), egui::vec2(210.0, 55.0));
        let anchor = anchor_of(
            &layout,
            original,
            &Anchor::TwoCell {
                from: AnchorPoint::default(),
                to: AnchorPoint::default(),
            },
        );
        let back = sheet_rect(&layout, &anchor);
        // EMUs are finer than pixels, so this is exact to well within one.
        assert!((back.min.x - original.min.x).abs() < 0.01, "{back:?}");
        assert!((back.min.y - original.min.y).abs() < 0.01, "{back:?}");
        assert!((back.max.x - original.max.x).abs() < 0.01, "{back:?}");
        assert!((back.max.y - original.max.y).abs() < 0.01, "{back:?}");
    }

    #[test]
    fn a_one_cell_anchor_keeps_its_kind_when_it_is_resized() {
        let layout = layout();
        let like = Anchor::OneCell {
            from: AnchorPoint::default(),
            width: 10,
            height: 10,
        };
        let anchor = anchor_of(
            &layout,
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 50.0)),
            &like,
        );
        match anchor {
            Anchor::OneCell { width, height, .. } => {
                assert_eq!(width, 100 * EMU_PER_POINT as i64);
                assert_eq!(height, 50 * EMU_PER_POINT as i64);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn dragging_an_edge_handle_moves_only_that_edge() {
        let start = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));
        let wider = Handle::East.resize(start, egui::pos2(200.0, 999.0));
        assert_eq!(wider.left(), 10.0);
        assert_eq!(wider.top(), 20.0);
        assert_eq!(wider.bottom(), 70.0);
        assert_eq!(wider.right(), 200.0);
    }

    #[test]
    fn a_corner_stretches_freely_rather_than_keeping_the_shape() {
        let start = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        let stretched = Handle::SouthEast.resize(start, egui::pos2(400.0, 110.0));
        assert_eq!(stretched.size(), egui::vec2(400.0, 110.0));
    }

    #[test]
    fn a_handle_dragged_past_the_opposite_edge_stops_rather_than_inverting() {
        let start = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        let squashed = Handle::East.resize(start, egui::pos2(-500.0, 0.0));
        assert!(squashed.width() >= MIN_SIZE);
        assert!(squashed.right() > squashed.left());
    }

    #[test]
    fn the_handles_are_where_the_pointer_expects_them() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 40.0));
        assert_eq!(
            handle_at(rect, egui::pos2(0.0, 0.0)),
            Some(Handle::NorthWest)
        );
        assert_eq!(handle_at(rect, egui::pos2(50.0, 40.0)), Some(Handle::South));
        assert_eq!(handle_at(rect, egui::pos2(50.0, 20.0)), None, "the middle");
    }
}
