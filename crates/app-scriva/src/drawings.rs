//! A picture as an object: selected, moved, resized, deleted.
//!
//! **The edits are the two numbers the file can hold.** A drawing's bytes are
//! kept whole (`Drawing::source`), and only its size and its position are
//! spliced back into them by `wp_docx::write::drawing`. So this changes those
//! two things and nothing else: there is no crop, no rotate and no recolour,
//! because there is no honest way to write one back yet. Calx's rule — never
//! show an edit that the writer will silently throw away.
//!
//! **Moving is for anchored drawings only.** An inline drawing sits in the text
//! like a letter: its position is decided by the words around it, and dragging
//! one in Word converts it to a floating drawing, which is a different element
//! and a different `<w:drawing>`. Resizing and deleting work on both.

use wp_layout::block::{anchor_base, anchor_position};
use wp_model::doc::{Alignment, Drawing, DrawingPosition, Offset, RelativeTo};
use wp_model::{Emu, PageBox};

/// Which drawing is selected: a paragraph, and which of its drawings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picked {
    pub paragraph: usize,
    pub nth: usize,
}

/// The eight handles of a selected drawing, and the body between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    Body,
    Corner { right: bool, bottom: bool },
    Edge { horizontal: bool, far: bool },
}

/// How near a handle the pointer has to be, in points.
pub const GRIP: f64 = 4.0;

/// Which part of a selected drawing the pointer is over.
///
/// Corners before edges, because the corner grips overlap the edge grips and a
/// user who aims at a corner means the corner.
pub fn grip_at(rect: (f64, f64, f64, f64), x: f64, y: f64) -> Option<Grip> {
    let (left, top, width, height) = rect;
    let (right, bottom) = (left + width, top + height);
    if x < left - GRIP || x > right + GRIP || y < top - GRIP || y > bottom + GRIP {
        return None;
    }
    let near = |a: f64, b: f64| (a - b).abs() <= GRIP;
    let (near_left, near_right) = (near(x, left), near(x, right));
    let (near_top, near_bottom) = (near(y, top), near(y, bottom));
    if (near_left || near_right) && (near_top || near_bottom) {
        return Some(Grip::Corner {
            right: near_right,
            bottom: near_bottom,
        });
    }
    if near_left || near_right {
        return Some(Grip::Edge {
            horizontal: true,
            far: near_right,
        });
    }
    if near_top || near_bottom {
        return Some(Grip::Edge {
            horizontal: false,
            far: near_bottom,
        });
    }
    Some(Grip::Body)
}

/// Moves an anchored drawing by a distance in points.
///
/// The drawing is moved on the page and the file has to say the same thing in
/// the drawing's own frame of reference, so the new position is resolved back
/// through the base it is measured from. An alignment becomes an offset: once a
/// user has put a picture somewhere by hand, "centred" is no longer true.
pub fn moved(drawing: &mut Drawing, page: &PageBox, line_top: f64, dx: f64, dy: f64) -> bool {
    if !drawing.anchored {
        return false;
    }
    let (x, y) = anchor_position(drawing, page, line_top);
    let (base_x, base_y) = anchor_base(drawing, page, line_top);
    let position = drawing.position.get_or_insert_with(|| {
        Box::new(DrawingPosition {
            horizontal: axis(RelativeTo::Column),
            vertical: axis(RelativeTo::Paragraph),
        })
    });
    position.horizontal.align = None;
    position.horizontal.offset = Some(Emu::from_points(x + dx - base_x));
    position.vertical.align = None;
    position.vertical.offset = Some(Emu::from_points(y + dy - base_y));
    true
}

fn axis(relative_to: RelativeTo) -> Offset {
    Offset {
        relative_to,
        offset: None,
        align: None,
    }
}

/// The smallest a drawing may be dragged to, in points. Below this the handles
/// are on top of each other and it cannot be dragged back.
const FLOOR: f64 = 6.0;

/// Resizes a drawing by dragging one grip.
///
/// A corner keeps the aspect ratio, as Word's corners do — a picture stretched
/// by a corner is almost always an accident. An edge does not.
pub fn resized(drawing: &mut Drawing, grip: Grip, dx: f64, dy: f64) -> bool {
    let width = drawing.extent.0.points();
    let height = drawing.extent.1.points();
    let (new_width, new_height) = match grip {
        Grip::Body => return false,
        Grip::Corner { right, bottom } => {
            let by_width = width + if right { dx } else { -dx };
            let by_height = height + if bottom { dy } else { -dy };
            // Whichever axis the user pulled further decides, so the picture
            // follows the pointer rather than fighting it.
            if (by_width - width).abs() >= (by_height - height).abs() {
                let scale = (by_width / width).max(0.01);
                (by_width, height * scale)
            } else {
                let scale = (by_height / height).max(0.01);
                (width * scale, by_height)
            }
        }
        Grip::Edge { horizontal, far } => {
            let delta = if horizontal { dx } else { dy };
            let delta = if far { delta } else { -delta };
            match horizontal {
                true => (width + delta, height),
                false => (width, height + delta),
            }
        }
    };
    if new_width < FLOOR || new_height < FLOOR {
        return false;
    }
    drawing.extent = (Emu::from_points(new_width), Emu::from_points(new_height));
    true
}

/// Sets a drawing's size from typed numbers, in points.
pub fn set_size(drawing: &mut Drawing, width: f64, height: f64) -> bool {
    if width < FLOOR || height < FLOOR {
        return false;
    }
    drawing.extent = (Emu::from_points(width), Emu::from_points(height));
    true
}

/// Puts an anchored drawing where an alignment says, rather than at an offset.
pub fn align(drawing: &mut Drawing, horizontal: Option<Alignment>, vertical: Option<Alignment>) {
    let position = drawing.position.get_or_insert_with(|| {
        Box::new(DrawingPosition {
            horizontal: axis(RelativeTo::Margin),
            vertical: axis(RelativeTo::Paragraph),
        })
    });
    if let Some(alignment) = horizontal {
        position.horizontal.align = Some(alignment);
        position.horizontal.offset = None;
    }
    if let Some(alignment) = vertical {
        position.vertical.align = Some(alignment);
        position.vertical.offset = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn drawing(anchored: bool) -> Drawing {
        Drawing {
            source: b"<w:drawing/>".to_vec().into(),
            anchored,
            extent: (Emu::from_points(100.0), Emu::from_points(50.0)),
            rel: Some("rId5".into()),
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::Square,
            distance: Default::default(),
            position: None,
            behind_text: false,
        }
    }

    #[test]
    fn a_corner_keeps_the_shape_of_the_picture() {
        // A picture stretched by a corner is almost always an accident.
        let mut model = drawing(true);
        assert!(resized(
            &mut model,
            Grip::Corner {
                right: true,
                bottom: true,
            },
            50.0,
            0.0,
        ));
        assert_eq!(model.extent.0.points(), 150.0);
        assert_eq!(model.extent.1.points(), 75.0, "the height followed");
    }

    #[test]
    fn an_edge_stretches_one_axis_only() {
        let mut model = drawing(true);
        assert!(resized(
            &mut model,
            Grip::Edge {
                horizontal: false,
                far: true,
            },
            0.0,
            25.0,
        ));
        assert_eq!(model.extent.0.points(), 100.0);
        assert_eq!(model.extent.1.points(), 75.0);
    }

    #[test]
    fn a_drawing_cannot_be_dragged_smaller_than_its_own_handles() {
        // Otherwise it collapses to nothing and there is no way to get it back.
        let mut model = drawing(true);
        assert!(!resized(
            &mut model,
            Grip::Edge {
                horizontal: true,
                far: true,
            },
            -99.0,
            0.0,
        ));
        assert_eq!(model.extent.0.points(), 100.0, "and nothing changed");
    }

    #[test]
    fn moving_a_drawing_states_the_new_place_in_the_files_own_terms() {
        let mut model = drawing(true);
        // No position at all: it sits at the column, at the line.
        assert!(moved(&mut model, &page(), 300.0, 20.0, -10.0));
        let position = model.position.as_ref().expect("a position now");
        assert_eq!(position.horizontal.offset, Some(Emu::from_points(20.0)));
        assert_eq!(position.vertical.offset, Some(Emu::from_points(-10.0)));
    }

    #[test]
    fn moving_a_centred_drawing_stops_it_being_centred() {
        // "Centred" was true until the user put it somewhere by hand.
        let mut model = drawing(true);
        align(&mut model, Some(Alignment::Center), None);
        assert!(moved(&mut model, &page(), 300.0, 10.0, 0.0));
        let position = model.position.as_ref().expect("a position");
        assert_eq!(position.horizontal.align, None);
        // Centred on a 72..540 column put it at 256; ten points on is 266, and
        // the column measures from 72.
        assert_eq!(position.horizontal.offset, Some(Emu::from_points(194.0)));
    }

    #[test]
    fn an_inline_drawing_is_not_dragged_around_the_page() {
        // It sits in the text like a letter. Word converts it to a floating
        // drawing instead, which is a different element.
        let mut model = drawing(false);
        assert!(!moved(&mut model, &page(), 300.0, 20.0, 20.0));
        assert!(model.position.is_none());
    }

    #[test]
    fn a_corner_is_preferred_to_the_edge_it_sits_on() {
        let rect = (100.0, 100.0, 80.0, 40.0);
        assert_eq!(
            grip_at(rect, 180.0, 140.0),
            Some(Grip::Corner {
                right: true,
                bottom: true,
            })
        );
        assert_eq!(
            grip_at(rect, 180.0, 120.0),
            Some(Grip::Edge {
                horizontal: true,
                far: true,
            })
        );
        assert_eq!(grip_at(rect, 140.0, 120.0), Some(Grip::Body));
        assert_eq!(grip_at(rect, 300.0, 120.0), None);
    }
}
