//! The Enhanced Metafile player.
//!
//! An EMF is a list of records, each one a GDI call: select this pen, multiply
//! the world transform by this matrix, walk this path, write these characters
//! at this point. Playing it back is keeping the same state a device context
//! keeps and turning the drawing calls into [`Prim`]s as they arrive.
//!
//! **Two things about it are not obvious and are worth stating.**
//!
//! The first is where the picture is. A record's coordinates are in *logical*
//! units, which are device pixels of the machine that recorded it; the header
//! says how large that machine's screen was in millimetres, and that ratio —
//! not the record bounds — is what turns a coordinate into a length. A player
//! that instead stretches the ink's bounding box to fill the box it was given
//! loses the drawing's own margins and draws every diagram at a slightly
//! different scale from the one Word draws.
//!
//! The second is that Word records geometry in sixteenths. It multiplies the
//! world transform by 1/16, emits a shape in whole units of that finer grid,
//! and multiplies by 16 again — so a coordinate of 2975 is 185.9 and, more
//! importantly, a *pen* five units wide is five sixteenths of a unit wide,
//! because a geometric pen is measured in the logical units in force when it
//! draws. Reading the width without the transform draws every line in the
//! document sixteen times too thick.
//!
//! Record numbers and field offsets are [MS-EMF] section 2.3.

use std::collections::HashMap;

use crate::{Picture, Prim};

/// Points per millimetre.
const POINTS_PER_MM: f64 = 72.0 / 25.4;

/// How many segments a cubic curve is cut into.
///
/// A diagram's curves are the corners of rounded rectangles and the elbows of
/// connectors, none of them larger than a few points on the page, so this is
/// generous rather than tight.
const CURVE_STEPS: usize = 12;

/// A 2x3 affine transform in GDI's own order and convention: a row vector
/// times a matrix, so that `x' = m11*x + m21*y + dx`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Xform {
    m11: f64,
    m12: f64,
    m21: f64,
    m22: f64,
    dx: f64,
    dy: f64,
}

impl Xform {
    const IDENTITY: Xform = Xform {
        m11: 1.0,
        m12: 0.0,
        m21: 0.0,
        m22: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.m11 * x + self.m21 * y + self.dx,
            self.m12 * x + self.m22 * y + self.dy,
        )
    }

    /// `self` first, then `then` — which is the product `self * then` under
    /// GDI's row-vector convention.
    fn then(&self, then: &Xform) -> Xform {
        Xform {
            m11: self.m11 * then.m11 + self.m12 * then.m21,
            m12: self.m11 * then.m12 + self.m12 * then.m22,
            m21: self.m21 * then.m11 + self.m22 * then.m21,
            m22: self.m21 * then.m12 + self.m22 * then.m22,
            dx: self.dx * then.m11 + self.dy * then.m21 + then.dx,
            dy: self.dx * then.m12 + self.dy * then.m22 + then.dy,
        }
    }

    /// How much this transform scales a length, ignoring which way it points.
    fn magnitude(&self) -> f64 {
        (self.m11 * self.m22 - self.m12 * self.m21).abs().sqrt()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Pen {
    rgb: [u8; 3],
    /// In logical units, before the world transform. Zero for a pen that
    /// draws nothing.
    width: f64,
    draws: bool,
    /// A cosmetic pen is one device unit wide however the world is scaled,
    /// which is the one width the transform must not touch.
    cosmetic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Brush {
    rgb: [u8; 3],
    fills: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Font {
    family: String,
    /// Logical units, always positive: a negative `lfHeight` is the height of
    /// the characters and a positive one the height of the cell they sit in,
    /// and the difference is internal leading this crate cannot measure.
    height: f64,
    bold: bool,
    italic: bool,
    /// Tenths of a degree counter-clockwise, as `lfEscapement` states it.
    escapement: f64,
}

impl Default for Font {
    fn default() -> Font {
        Font {
            family: "Arial".to_owned(),
            height: 12.0,
            bold: false,
            italic: false,
            escapement: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(Font),
}

/// Everything a device context remembers, and therefore everything `SaveDC`
/// puts on the stack.
#[derive(Debug, Clone, PartialEq)]
struct Dc {
    xform: Xform,
    pen: Pen,
    brush: Brush,
    font: Font,
    text_rgb: [u8; 3],
}

impl Default for Dc {
    fn default() -> Dc {
        Dc {
            xform: Xform::IDENTITY,
            pen: Pen {
                rgb: [0, 0, 0],
                width: 1.0,
                draws: true,
                cosmetic: true,
            },
            brush: Brush {
                rgb: [255, 255, 255],
                fills: true,
            },
            font: Font::default(),
            text_rgb: [0, 0, 0],
        }
    }
}

struct Player {
    dc: Dc,
    saved: Vec<Dc>,
    objects: HashMap<u32, Object>,
    /// Logical units to points, one factor per axis: a recording machine's
    /// pixels were not necessarily square.
    scale: (f64, f64),
    /// The run of segments being drawn outside a path, in points.
    open: Vec<(f64, f64)>,
    /// Where the next `LineTo` starts, in logical units before the transform
    /// — the position is remembered in the space the records speak.
    at: (f64, f64),
    /// The figures of a path under construction, in points.
    figures: Vec<Vec<(f64, f64)>>,
    in_path: bool,
    prims: Vec<Prim>,
}

/// Plays an EMF, or gives nothing back when the bytes are not one.
pub fn play(bytes: &[u8]) -> Option<Picture> {
    let header = record_at(bytes, 0).filter(|record| record.kind == 1)?;
    if header.bytes.get(40..44) != Some(b" EMF") {
        return None;
    }
    // `rclFrame` is the picture itself, in hundredths of a millimetre;
    // `szlDevice` and `szlMillimeters` are the recording screen, and their
    // ratio is what one logical unit is worth.
    let frame = rect(header.bytes, 24)?;
    let device = (i32(header.bytes, 72)?, i32(header.bytes, 76)?);
    let millimetres = (i32(header.bytes, 80)?, i32(header.bytes, 84)?);
    let size = (
        f64::from(frame.2 - frame.0) / 100.0 * POINTS_PER_MM,
        f64::from(frame.3 - frame.1) / 100.0 * POINTS_PER_MM,
    );
    if !(size.0 > 0.0 && size.1 > 0.0) {
        return None;
    }
    // A recording that states no screen size still states its own bounds, and
    // fitting the frame to those is the same ratio by another route.
    let bounds = rect(header.bytes, 8)?;
    let square = device.0 > 0 && millimetres.0 > 0 && device.1 > 0 && millimetres.1 > 0;
    let scale = match square {
        true => (
            f64::from(millimetres.0) / f64::from(device.0) * POINTS_PER_MM,
            f64::from(millimetres.1) / f64::from(device.1) * POINTS_PER_MM,
        ),
        false => (
            size.0 / f64::from(bounds.2 - bounds.0).max(1.0),
            size.1 / f64::from(bounds.3 - bounds.1).max(1.0),
        ),
    };

    let mut player = Player {
        dc: Dc::default(),
        saved: Vec::new(),
        objects: HashMap::new(),
        scale,
        open: Vec::new(),
        at: (0.0, 0.0),
        figures: Vec::new(),
        in_path: false,
        prims: Vec::new(),
    };
    let mut at = 0usize;
    // A record that states a length of nothing would otherwise be walked for
    // ever; the walk stops where it stops believing the file.
    while let Some(record) = record_at(bytes, at) {
        if record.kind == 14 {
            break;
        }
        at += record.bytes.len();
        player.record(&record);
    }
    player.flush();
    Some(Picture {
        size,
        prims: player.prims,
    })
}

struct Record<'a> {
    kind: u32,
    bytes: &'a [u8],
}

fn record_at(bytes: &[u8], at: usize) -> Option<Record<'_>> {
    let kind = u32(bytes, at)?;
    let size = u32(bytes, at + 4)? as usize;
    // Every record is a whole number of words and no shorter than its header.
    if size < 8 || !size.is_multiple_of(4) {
        return None;
    }
    Some(Record {
        kind,
        bytes: bytes.get(at..at + size)?,
    })
}

impl Player {
    fn record(&mut self, record: &Record<'_>) {
        let data = record.bytes;
        // Everything but the calls that continue a line ends the run being
        // drawn, so that a pen or a transform never changes underneath one.
        if !matches!(record.kind, 5 | 6 | 54 | 70 | 88 | 89) {
            self.flush();
        }
        match record.kind {
            // MoveToEx
            27 => {
                self.flush();
                self.at = point32(data, 8);
                if self.in_path {
                    let start = self.place(self.at);
                    self.figures.push(vec![start]);
                }
            }
            // LineTo
            54 => {
                let to = point32(data, 8);
                self.line_to(&[to]);
            }
            // PolyBezier, Polygon, Polyline
            2..=4 => {
                let points = points32(data, 28, count(data, 24));
                self.figure(record.kind, &points);
            }
            // PolyBezier16, Polygon16, Polyline16
            85..=87 => {
                let points = points16(data, 28, count(data, 24));
                self.figure(record.kind - 83, &points);
            }
            // PolyBezierTo, PolyLineTo
            5 | 6 => {
                let points = points32(data, 28, count(data, 24));
                match record.kind {
                    5 => self.curve_to(&points),
                    _ => self.line_to(&points),
                }
            }
            // PolyBezierTo16, PolyLineTo16
            88 | 89 => {
                let points = points16(data, 28, count(data, 24));
                match record.kind {
                    88 => self.curve_to(&points),
                    _ => self.line_to(&points),
                }
            }
            // PolyPolyline, PolyPolygon, and their sixteen-bit forms.
            7 | 8 | 90 | 91 => self.poly_poly(record.kind, data),
            // BeginPath
            59 => {
                self.in_path = true;
                self.figures.clear();
            }
            // EndPath
            60 => self.in_path = false,
            // CloseFigure
            61 => {
                if let Some(figure) = self.figures.last_mut() {
                    if let Some(&first) = figure.first() {
                        figure.push(first);
                    }
                }
            }
            // FillPath, StrokeAndFillPath, StrokePath
            62..=64 => {
                let fills = record.kind != 64;
                let strokes = record.kind != 62;
                for figure in std::mem::take(&mut self.figures) {
                    if fills {
                        self.fill(&figure);
                    }
                    if strokes {
                        self.stroke(&figure);
                    }
                }
            }
            // SetWorldTransform
            35 => {
                if let Some(xform) = xform(data, 8) {
                    self.dc.xform = xform;
                }
            }
            // ModifyWorldTransform
            36 => {
                let Some(by) = xform(data, 8) else { return };
                self.dc.xform = match u32(data, 32) {
                    Some(1) => Xform::IDENTITY,
                    // Left-multiplied means the given transform happens first.
                    Some(2) => by.then(&self.dc.xform),
                    Some(3) => self.dc.xform.then(&by),
                    _ => by,
                };
            }
            // SaveDC
            33 => self.saved.push(self.dc.clone()),
            // RestoreDC
            34 => {
                if let Some(dc) = self.saved.pop() {
                    self.dc = dc;
                }
            }
            // SelectObject
            37 => self.select(u32(data, 8).unwrap_or(0)),
            // DeleteObject
            40 => {
                if let Some(handle) = u32(data, 8) {
                    self.objects.remove(&handle);
                }
            }
            // CreatePen
            38 => {
                let (Some(handle), Some(style), Some(width), Some(colour)) =
                    (u32(data, 8), u32(data, 12), i32(data, 16), u32(data, 24))
                else {
                    return;
                };
                self.objects.insert(
                    handle,
                    Object::Pen(Pen {
                        rgb: colour_ref(colour),
                        width: f64::from(width).max(0.0),
                        draws: style & 0x0F != 5,
                        cosmetic: true,
                    }),
                );
            }
            // ExtCreatePen
            95 => {
                let (Some(handle), Some(style), Some(width), Some(colour)) =
                    (u32(data, 8), u32(data, 28), u32(data, 32), u32(data, 40))
                else {
                    return;
                };
                // A brush style of `BS_NULL` is a pen that measures a shape
                // without marking it.
                let hollow = u32(data, 36) == Some(1);
                self.objects.insert(
                    handle,
                    Object::Pen(Pen {
                        rgb: colour_ref(colour),
                        width: f64::from(width),
                        draws: style & 0x0F != 5 && !hollow,
                        cosmetic: style & 0x0001_0000 == 0,
                    }),
                );
            }
            // CreateBrushIndirect
            39 => {
                let (Some(handle), Some(style), Some(colour)) =
                    (u32(data, 8), u32(data, 12), u32(data, 16))
                else {
                    return;
                };
                self.objects.insert(
                    handle,
                    Object::Brush(Brush {
                        rgb: colour_ref(colour),
                        // Hatched and patterned brushes are not drawn: filling
                        // them flat would be a shape the drawing has not got.
                        fills: style == 0,
                    }),
                );
            }
            // ExtCreateFontIndirectW
            82 => {
                let Some(handle) = u32(data, 8) else { return };
                let Some(font) = log_font(data, 12) else {
                    return;
                };
                self.objects.insert(handle, Object::Font(font));
            }
            // SetTextColor
            24 => {
                if let Some(colour) = u32(data, 8) {
                    self.dc.text_rgb = colour_ref(colour);
                }
            }
            // ExtTextOutW
            84 => self.text(data),
            _ => {}
        }
    }

    /// A logical point, transformed and turned into points.
    fn place(&self, point: (f64, f64)) -> (f64, f64) {
        let (x, y) = self.dc.xform.apply(point.0, point.1);
        (x * self.scale.0, y * self.scale.1)
    }

    /// How much of a point one logical unit is worth right now, which is what
    /// a pen width and a type size are both measured in.
    fn along(&self) -> f64 {
        // The average of the two axes: a line is one width, and a recording
        // machine with unsquare pixels does not give it two.
        self.dc.xform.magnitude() * (self.scale.0 + self.scale.1) / 2.0
    }

    /// How wide the current pen draws, in points.
    fn pen_width(&self) -> f64 {
        match self.dc.pen.cosmetic {
            // One device unit, whatever the world has been scaled to.
            true => self.dc.pen.width.max(1.0) * (self.scale.0 + self.scale.1) / 2.0,
            false => self.dc.pen.width * self.along(),
        }
    }

    fn stroke(&mut self, points: &[(f64, f64)]) {
        if points.len() < 2 || !self.dc.pen.draws {
            return;
        }
        self.prims.push(Prim::Stroke {
            points: points.to_vec(),
            rgb: self.dc.pen.rgb,
            width: self.pen_width(),
        });
    }

    fn fill(&mut self, points: &[(f64, f64)]) {
        if points.len() < 3 || !self.dc.brush.fills {
            return;
        }
        self.prims.push(Prim::Fill {
            points: points.to_vec(),
            rgb: self.dc.brush.rgb,
        });
    }

    /// Draws whatever run of segments was still open.
    fn flush(&mut self) {
        let open = std::mem::take(&mut self.open);
        self.stroke(&open);
    }

    /// `LineTo` and its plural: outside a path they draw, inside one they
    /// build the figure.
    fn line_to(&mut self, points: &[(f64, f64)]) {
        for &point in points {
            let placed = self.place(point);
            match self.in_path {
                true => match self.figures.last_mut() {
                    Some(figure) => figure.push(placed),
                    None => {
                        let start = self.place(self.at);
                        self.figures.push(vec![start, placed]);
                    }
                },
                false => {
                    if self.open.is_empty() {
                        self.open.push(self.place(self.at));
                    }
                    self.open.push(placed);
                }
            }
            self.at = point;
        }
    }

    /// The same for a run of cubics, each three points long, flattened.
    fn curve_to(&mut self, points: &[(f64, f64)]) {
        for triple in points.chunks_exact(3) {
            let from = self.at;
            let cut: Vec<(f64, f64)> = (1..=CURVE_STEPS)
                .map(|step| {
                    let t = step as f64 / CURVE_STEPS as f64;
                    cubic(from, triple[0], triple[1], triple[2], t)
                })
                .collect();
            self.line_to(&cut);
            self.at = triple[2];
        }
    }

    /// One whole shape given in a single record: a polygon fills and strokes,
    /// a polyline strokes, a bezier strokes what it flattens to.
    fn figure(&mut self, kind: u32, points: &[(f64, f64)]) {
        let Some(&first) = points.first() else { return };
        if kind == 2 {
            self.at = first;
            let rest = points[1..].to_vec();
            self.curve_to(&rest);
            self.flush();
            return;
        }
        let placed: Vec<(f64, f64)> = points.iter().map(|&point| self.place(point)).collect();
        if kind == 3 {
            let mut closed = placed.clone();
            if closed.first() != closed.last() {
                closed.push(placed[0]);
            }
            self.fill(&closed);
            self.stroke(&closed);
        } else {
            self.stroke(&placed);
        }
        if let Some(&last) = points.last() {
            self.at = last;
        }
    }

    /// `PolyPolyline` and `PolyPolygon`: a count of shapes, the length of
    /// each, and then all their points end to end.
    fn poly_poly(&mut self, kind: u32, data: &[u8]) {
        let shapes = count(data, 24);
        let Some(lengths): Option<Vec<usize>> = (0..shapes)
            .map(|i| u32(data, 32 + i * 4).map(|n| n as usize))
            .collect()
        else {
            return;
        };
        let at = 32 + lengths.len() * 4;
        let total: usize = lengths.iter().sum();
        let points = match kind {
            7 | 8 => points32(data, at, total.min(data.len())),
            _ => points16(data, at, total.min(data.len())),
        };
        let polygons = matches!(kind, 8 | 91);
        let mut from = 0;
        for length in lengths {
            let Some(shape) = points.get(from..from + length) else {
                break;
            };
            from += length;
            self.figure(if polygons { 3 } else { 4 }, shape);
        }
    }

    fn select(&mut self, handle: u32) {
        // The stock objects, which a metafile names rather than creates.
        if handle & 0x8000_0000 != 0 {
            let grey = |value: u8| Brush {
                rgb: [value; 3],
                fills: true,
            };
            match handle & 0x7FFF_FFFF {
                0 => self.dc.brush = grey(255),
                1 => self.dc.brush = grey(192),
                2 => self.dc.brush = grey(128),
                3 => self.dc.brush = grey(64),
                4 => self.dc.brush = grey(0),
                5 => self.dc.brush.fills = false,
                6 | 7 => {
                    self.dc.pen = Pen {
                        rgb: [if handle & 0x7FFF_FFFF == 6 { 255 } else { 0 }; 3],
                        width: 1.0,
                        draws: true,
                        cosmetic: true,
                    }
                }
                8 => self.dc.pen.draws = false,
                _ => {}
            }
            return;
        }
        match self.objects.get(&handle) {
            Some(Object::Pen(pen)) => self.dc.pen = *pen,
            Some(Object::Brush(brush)) => self.dc.brush = *brush,
            Some(Object::Font(font)) => self.dc.font = font.clone(),
            None => {}
        }
    }

    fn text(&mut self, data: &[u8]) {
        let reference = point32(data, 36);
        let chars = count(data, 44);
        let (Some(string_at), Some(dx_at)) = (u32(data, 48), u32(data, 72)) else {
            return;
        };
        let Some(bytes) = data.get(string_at as usize..string_at as usize + chars * 2) else {
            return;
        };
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        if text.trim().is_empty() {
            return;
        }
        // The advances are one per code unit and the string is one per
        // character: outside the basic plane the two disagree, and folding a
        // pair's advances together keeps the run as wide as it was recorded.
        let along = self.along();
        let stated: Vec<f64> = (0..chars)
            .map(|i| f64::from(i32(data, dx_at as usize + i * 4).unwrap_or(0)) * along)
            .collect();
        let mut advances = Vec::with_capacity(text.chars().count());
        let mut unit = 0;
        for character in text.chars() {
            let width: f64 = stated
                .get(unit..unit + character.len_utf16())
                .unwrap_or_default()
                .iter()
                .sum();
            advances.push(width);
            unit += character.len_utf16();
        }
        let (x, baseline) = self.place(reference);
        self.prims.push(Prim::Text {
            x,
            baseline,
            text,
            advances,
            family: self.dc.font.family.clone(),
            size: self.dc.font.height * along,
            bold: self.dc.font.bold,
            italic: self.dc.font.italic,
            rgb: self.dc.text_rgb,
            // `lfEscapement` turns text the way mathematics does, and a page
            // turns it the other way.
            rotation: -self.dc.font.escapement / 10.0,
        });
    }
}

fn cubic(from: (f64, f64), a: (f64, f64), b: (f64, f64), to: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        w0 * from.0 + w1 * a.0 + w2 * b.0 + w3 * to.0,
        w0 * from.1 + w1 * a.1 + w2 * b.1 + w3 * to.1,
    )
}

fn log_font(data: &[u8], at: usize) -> Option<Font> {
    let height = i32(data, at)?;
    let escapement = i32(data, at + 8)?;
    let weight = i32(data, at + 16)?;
    let italic = *data.get(at + 20)? != 0;
    let name = data.get(at + 28..at + 92)?;
    let family: String = String::from_utf16_lossy(
        &name
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect::<Vec<u16>>(),
    );
    Some(Font {
        family: match family.is_empty() {
            true => "Arial".to_owned(),
            false => family,
        },
        height: f64::from(height.abs()),
        bold: weight >= 600,
        italic,
        escapement: f64::from(escapement),
    })
}

fn xform(data: &[u8], at: usize) -> Option<Xform> {
    let value = |offset: usize| -> Option<f64> {
        let bytes = data.get(at + offset..at + offset + 4)?;
        Some(f64::from(f32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ])))
    };
    Some(Xform {
        m11: value(0)?,
        m12: value(4)?,
        m21: value(8)?,
        m22: value(12)?,
        dx: value(16)?,
        dy: value(20)?,
    })
}

/// `COLORREF` is blue, green and red with a byte of nothing on the end.
fn colour_ref(value: u32) -> [u8; 3] {
    [value as u8, (value >> 8) as u8, (value >> 16) as u8]
}

fn rect(data: &[u8], at: usize) -> Option<(i32, i32, i32, i32)> {
    Some((
        i32(data, at)?,
        i32(data, at + 4)?,
        i32(data, at + 8)?,
        i32(data, at + 12)?,
    ))
}

/// A count of things, capped at what the record could possibly hold.
fn count(data: &[u8], at: usize) -> usize {
    u32(data, at).unwrap_or(0).min(data.len() as u32) as usize
}

fn point32(data: &[u8], at: usize) -> (f64, f64) {
    (
        f64::from(i32(data, at).unwrap_or(0)),
        f64::from(i32(data, at + 4).unwrap_or(0)),
    )
}

fn points32(data: &[u8], at: usize, count: usize) -> Vec<(f64, f64)> {
    (0..count)
        .map_while(|i| {
            let base = at + i * 8;
            Some((f64::from(i32(data, base)?), f64::from(i32(data, base + 4)?)))
        })
        .collect()
}

fn points16(data: &[u8], at: usize, count: usize) -> Vec<(f64, f64)> {
    let short = |at: usize| -> Option<f64> {
        let bytes = data.get(at..at + 2)?;
        Some(f64::from(i16::from_le_bytes([bytes[0], bytes[1]])))
    };
    (0..count)
        .map_while(|i| {
            let base = at + i * 4;
            Some((short(base)?, short(base + 2)?))
        })
        .collect()
}

fn u32(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn i32(data: &[u8], at: usize) -> Option<i32> {
    u32(data, at).map(|value| value as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole metafile, built record by record, so that a test can say what
    /// it means rather than a hexadecimal string.
    #[derive(Default)]
    struct Emf(Vec<u8>);

    impl Emf {
        fn record(&mut self, kind: u32, body: &[u8]) -> &mut Emf {
            self.0.extend((kind).to_le_bytes());
            self.0.extend(((body.len() + 8) as u32).to_le_bytes());
            self.0.extend(body);
            self
        }

        /// A screen of 1000 units across 254 millimetres, so that one logical
        /// unit is exactly 0.72 of a point and the arithmetic in a test is
        /// arithmetic rather than rounding.
        fn header(width: i32, height: i32) -> Emf {
            let mut body = Vec::new();
            for value in [0i32, 0, width, height] {
                body.extend(value.to_le_bytes());
            }
            // rclFrame, in hundredths of a millimetre.
            for value in [0i32, 0, width * 254 * 100 / 1000, height * 254 * 100 / 1000] {
                body.extend(value.to_le_bytes());
            }
            body.extend(b" EMF");
            body.extend([0u8; 28]);
            for value in [1000i32, 1000, 254, 254] {
                body.extend(value.to_le_bytes());
            }
            let mut emf = Emf::default();
            emf.record(1, &body);
            emf
        }

        fn done(&mut self) -> Vec<u8> {
            self.record(14, &[0u8; 12]);
            self.0.clone()
        }
    }

    fn points(values: &[(i16, i16)]) -> Vec<u8> {
        let mut body = vec![0u8; 16];
        body.extend((values.len() as u32).to_le_bytes());
        for (x, y) in values {
            body.extend(x.to_le_bytes());
            body.extend(y.to_le_bytes());
        }
        body
    }

    #[test]
    fn a_picture_is_as_large_as_its_frame_says_and_not_as_large_as_its_ink() {
        // Ink in one corner only: a player that fits the bounding box to the
        // page would draw this diagram at four times its size.
        let mut emf = Emf::header(1000, 500);
        let picture = play(&emf.record(87, &points(&[(0, 0), (10, 10)])).done()).expect("plays");
        assert!((picture.size.0 - 720.0).abs() < 0.01, "{:?}", picture.size);
        assert!((picture.size.1 - 360.0).abs() < 0.01, "{:?}", picture.size);
        let Some(Prim::Stroke { points, .. }) = picture.prims.first() else {
            panic!("a polyline strokes: {:?}", picture.prims);
        };
        assert!((points[1].0 - 7.2).abs() < 0.01, "{points:?}");
    }

    #[test]
    fn a_geometric_pen_is_as_wide_as_the_transform_in_force_makes_it() {
        // Word's own idiom: scale the world down by sixteen, draw in the finer
        // grid, scale back. A pen created inside it is a sixteenth as wide as
        // its stated width, and reading the width alone draws it far too
        // heavily.
        let mut body = Vec::new();
        for value in [0.0625f32, 0.0, 0.0, 0.0625, 0.0, 0.0] {
            body.extend(value.to_le_bytes());
        }
        body.extend(2u32.to_le_bytes());
        let mut pen = Vec::new();
        pen.extend(1u32.to_le_bytes());
        pen.extend([0u8; 16]);
        pen.extend(0x0001_0000u32.to_le_bytes());
        pen.extend(16u32.to_le_bytes());
        pen.extend(0u32.to_le_bytes());
        pen.extend(0u32.to_le_bytes());
        pen.extend([0u8; 8]);

        let mut emf = Emf::header(1000, 500);
        let bytes = emf
            .record(36, &body)
            .record(95, &pen)
            .record(37, &1u32.to_le_bytes())
            .record(87, &points(&[(0, 0), (160, 0)]))
            .done();
        let picture = play(&bytes).expect("plays");
        let Some(Prim::Stroke { width, points, .. }) = picture.prims.first() else {
            panic!("{:?}", picture.prims);
        };
        // Sixteen logical units wide, a sixteenth of that after the transform,
        // and 0.72 of a point to the unit.
        assert!((width - 0.72).abs() < 0.001, "{width}");
        // And the same transform moves the geometry: 160 sixteenths is ten.
        assert!((points[1].0 - 7.2).abs() < 0.01, "{points:?}");
    }

    #[test]
    fn a_path_is_filled_with_the_brush_and_stroked_with_the_pen_it_asks_for() {
        let mut brush = Vec::new();
        brush.extend(1u32.to_le_bytes());
        brush.extend(0u32.to_le_bytes());
        // COLORREF, which is blue-green-red rather than red-green-blue.
        brush.extend(0x00FF_8000u32.to_le_bytes());
        brush.extend(0u32.to_le_bytes());

        let mut emf = Emf::header(1000, 500);
        let mut start = vec![0u8; 0];
        start.extend(0i32.to_le_bytes());
        start.extend(0i32.to_le_bytes());
        let mut corner = Vec::new();
        corner.extend(100i32.to_le_bytes());
        corner.extend(0i32.to_le_bytes());
        let bytes = emf
            .record(39, &brush)
            .record(37, &1u32.to_le_bytes())
            .record(59, &[])
            .record(27, &start)
            .record(54, &corner)
            .record(89, &points(&[(100, 100)]))
            .record(61, &[])
            .record(60, &[])
            .record(62, &[0u8; 16])
            .done();
        let picture = play(&bytes).expect("plays");
        let Some(Prim::Fill { points, rgb }) = picture.prims.first() else {
            panic!("{:?}", picture.prims);
        };
        assert_eq!(*rgb, [0x00, 0x80, 0xFF]);
        // Four corners: the closed figure comes back to where it began.
        assert_eq!(points.len(), 4, "{points:?}");
    }

    #[test]
    fn text_is_placed_on_its_baseline_with_the_advances_the_drawing_recorded() {
        let mut font = Vec::new();
        font.extend(1u32.to_le_bytes());
        font.extend((-20i32).to_le_bytes());
        font.extend(0i32.to_le_bytes());
        font.extend(0i32.to_le_bytes());
        font.extend(0i32.to_le_bytes());
        font.extend(700i32.to_le_bytes());
        font.extend([0u8; 8]);
        let mut name = Vec::new();
        for unit in "Arial".encode_utf16() {
            name.extend(unit.to_le_bytes());
        }
        name.resize(64, 0);
        font.extend(&name);

        let mut text = vec![0u8; 28];
        text.extend(100i32.to_le_bytes());
        text.extend(200i32.to_le_bytes());
        text.extend(2u32.to_le_bytes());
        // The string and the advances both live past the fixed fields, at
        // offsets counted from the front of the record.
        text.extend(84u32.to_le_bytes());
        text.extend(0u32.to_le_bytes());
        text.extend([0u8; 16]);
        text.extend(88u32.to_le_bytes());
        // 8 + 28 + 8 + 4 + 4 + 4 + 16 + 4 = 76; pad to the string at 84.
        text.extend([0u8; 8]);
        text.extend(u16::from(b'h').to_le_bytes());
        text.extend(u16::from(b'i').to_le_bytes());
        text.extend(10i32.to_le_bytes());
        text.extend(6i32.to_le_bytes());

        let mut emf = Emf::header(1000, 500);
        let bytes = emf
            .record(82, &font)
            .record(37, &1u32.to_le_bytes())
            .record(84, &text)
            .done();
        let picture = play(&bytes).expect("plays");
        let Some(Prim::Text {
            x,
            baseline,
            text,
            advances,
            size,
            bold,
            family,
            ..
        }) = picture.prims.first()
        else {
            panic!("{:?}", picture.prims);
        };
        assert_eq!(text, "hi");
        assert_eq!(family, "Arial");
        assert!(*bold);
        assert!((x - 72.0).abs() < 0.01, "{x}");
        assert!((baseline - 144.0).abs() < 0.01, "{baseline}");
        assert!((size - 14.4).abs() < 0.01, "{size}");
        assert!((advances[0] - 7.2).abs() < 0.01, "{advances:?}");
    }

    #[test]
    fn a_record_that_states_no_length_stops_the_walk_rather_than_repeating_it() {
        let mut bytes = Emf::header(100, 100).0;
        bytes.extend(87u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        // Reaching this at all means the walk ended.
        assert!(play(&bytes).is_some());
    }
}
