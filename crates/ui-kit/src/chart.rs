//! Drawing a chart on the screen.
//!
//! Shared, because a chart in a document and a chart on a sheet are the same
//! picture of the same numbers — only the rectangle differs, and the caller
//! knows that.
//!
//! Where the ink goes is not decided here: [`chart::draw`] works that out in
//! plain numbers, so the screen, a PDF and a printer cannot come to disagree
//! about it. What is left for this module is the translation — one primitive
//! into one egui shape — and answering the question the geometry cannot ask
//! itself, which is how wide a string is in the faces this context loaded.

use chart::draw::{Measure, Prim, Rect};
use chart::Plot;

use crate::egui;
use crate::fonts::{self, Family};

/// The series, the style and the palette are the geometry's own types: a
/// second copy of them here would be a second thing to keep in step.
pub use chart::draw::{Plotted, Style, SERIES_COLORS};

/// A colour as the geometry states them.
pub fn rgb(color: egui::Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}

/// The face a chart sets its own text in, which is Word's rather than the
/// document's: a chart part that names none gets the minor latin font of
/// Office's built-in theme. `wp_print::ops` names the same one, so a label
/// centred on the screen is centred on the page.
const FACE: &str = "Calibri";

/// Measures in that face — or in the application's own sans, on a machine
/// that has no Calibri to give.
struct Fonts<'a>(&'a egui::Painter);

impl Measure for Fonts<'_> {
    fn size(&mut self, text: &str, size: f64) -> (f64, f64) {
        let galley =
            self.0
                .layout_no_wrap(text.to_string(), font(size), egui::Color32::PLACEHOLDER);
        (f64::from(galley.size().x), f64::from(galley.size().y))
    }
}

fn font(size: f64) -> egui::FontId {
    let family = fonts::exact_face(FACE, false, false)
        .unwrap_or_else(|| fonts::face(Family::Sans, false, false));
    egui::FontId::new(size as f32, family)
}

pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    plot: &Plot,
    series: &[Plotted],
    style: &Style,
) {
    let area = Rect::new(
        f64::from(rect.left()),
        f64::from(rect.top()),
        f64::from(rect.width()),
        f64::from(rect.height()),
    );
    for prim in chart::draw::primitives(area, plot, series, style, &mut Fonts(painter)) {
        paint(painter, &prim);
    }
}

fn paint(painter: &egui::Painter, prim: &Prim) {
    match prim {
        Prim::Fill { rect, rgb, round } => {
            painter.rect_filled(egui_rect(*rect), *round as f32, color(*rgb));
        }
        Prim::Frame {
            rect,
            rgb,
            thickness,
            round,
        } => {
            painter.rect_stroke(
                egui_rect(*rect),
                *round as f32,
                egui::Stroke::new(*thickness as f32, color(*rgb)),
                egui::StrokeKind::Inside,
            );
        }
        Prim::Line {
            points,
            thickness,
            rgb,
        } => {
            painter.add(egui::Shape::line(
                points.iter().map(|&(x, y)| pos(x, y)).collect(),
                egui::Stroke::new(*thickness as f32, color(*rgb)),
            ));
        }
        Prim::Poly {
            points,
            rgb: fill,
            edge,
        } => {
            // Not `Shape::convex_polygon`: its fan from the first corner
            // fills over every dip of a concave shape, and an area chart is
            // one. A mesh of the polygon's own triangles has no feathering,
            // so the outline is stroked as well — in the edge's ink where
            // there is one, else a hairline of the fill, which is what an
            // anti-aliased edge looks like.
            let corners: Vec<egui::Pos2> = points.iter().map(|&(x, y)| pos(x, y)).collect();
            let mut mesh = egui::Mesh::default();
            for corner in &corners {
                mesh.colored_vertex(*corner, color(*fill));
            }
            for [a, b, c] in chart::draw::triangles(points) {
                mesh.add_triangle(a as u32, b as u32, c as u32);
            }
            painter.add(egui::Shape::mesh(mesh));
            let stroke = match edge {
                Some((rgb, thickness)) => egui::Stroke::new(*thickness as f32, color(*rgb)),
                None => egui::Stroke::new(1.0, color(*fill)),
            };
            painter.add(egui::Shape::closed_line(corners, stroke));
        }
        Prim::Dot { at, radius, rgb } => {
            painter.circle_filled(pos(at.0, at.1), *radius as f32, color(*rgb));
        }
        Prim::Text {
            at,
            size,
            text,
            rgb,
        } => {
            let ink = color(*rgb);
            let galley = painter.layout_no_wrap(text.clone(), font(*size), ink);
            painter.galley(pos(at.0, at.1), galley, ink);
        }
    }
}

fn pos(x: f64, y: f64) -> egui::Pos2 {
    egui::pos2(x as f32, y as f32)
}

/// Both corners narrowed from the geometry's own numbers.
///
/// Not a corner and a size: egui would add those back together in `f32`, and
/// a bar whose base was worked out in `f64` as exactly the axis line comes
/// back an ulp above or below it depending on how tall the bar is. Three
/// columns then stand on three different baselines, by a distance no one can
/// see and every equality test can.
fn egui_rect(rect: Rect) -> egui::Rect {
    egui::Rect::from_min_max(pos(rect.x, rect.y), pos(rect.right(), rect.bottom()))
}

fn color(rgb: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chart::{ChartKind, Grouping, Series};

    /// One series in its own colour throughout, as an Excel-made part
    /// states with `varyColors="0"`.
    fn plot(kind: ChartKind) -> Plot {
        Plot {
            kind,
            grouping: Grouping::Clustered,
            vary_colors: false,
            series: vec![Series {
                name: Some("Sales".to_string()),
                values: vec![Some(1.0), Some(2.0), Some(3.0)],
                ..Default::default()
            }],
            ..Plot::default()
        }
    }

    /// Every shape a chart draws into a 400x300 box, so a test can look for
    /// the ones that matter rather than at a screenshot.
    fn painted(plot: &Plot, series: &[Plotted]) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        crate::fonts::register(&ctx, &[]);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 400.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
            let style = Style {
                background: [255, 255, 255],
                outline: [128, 128, 128],
                text: [0, 0, 0],
                grid: [128, 128, 128],
                zoom: 1.0,
                label: chart::draw::plain_label,
            };
            draw(ui.painter(), rect, plot, series, &style);
        });
        out.textures_delta.clear();
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    #[test]
    fn a_bar_chart_draws_a_bar_for_every_point() {
        // It did not, for as long as bar charts have been drawn: the bar was
        // built from a y range written low to high, which is upside down on a
        // screen, and egui fills an upside-down rectangle with nothing. The
        // axes and the gridlines were all anyone ever saw.
        let series = vec![Plotted {
            name: "Sales".to_string(),
            values: vec![Some(1.0), Some(2.0), Some(3.0)],
            rgb: [255, 0, 0],
            ..Default::default()
        }];
        let bars: Vec<egui::Rect> = painted(&plot(ChartKind::Bar), &series)
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Rect(r) if r.fill == egui::Color32::RED => Some(r.rect),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 3, "one bar per point");
        for bar in &bars {
            assert!(
                bar.height() > 1.0 && bar.width() > 1.0,
                "a bar nobody can see is not a bar: {bar:?}"
            );
        }
        // Taller values, taller bars — and all of them stand on one baseline.
        assert!(bars[0].height() < bars[1].height() && bars[1].height() < bars[2].height());
        assert!(bars.windows(2).all(|w| w[0].bottom() == w[1].bottom()));
    }

    #[test]
    fn an_area_chart_is_filled_as_its_own_triangles_and_keeps_its_dips() {
        // Drawn as a convex polygon, the gallery's area chart had its valley
        // at Feb filled in, where Excel showed the dip.
        let mut area = plot(ChartKind::Area);
        area.grouping = Grouping::Standard;
        area.series[0].values = vec![Some(10.0), Some(6.0), Some(15.0), Some(12.0)];
        let series = chart::draw::cached_series(&area);
        let shapes = painted(&area, &series);
        let mesh = shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Mesh(mesh) if mesh.vertices[0].color == color(SERIES_COLORS[0]) => {
                    Some(mesh)
                }
                _ => None,
            })
            .expect("the area as a mesh in its own colour");
        assert_eq!(mesh.vertices.len(), 6, "four tops and two feet");
        assert_eq!(mesh.indices.len(), 4 * 3);
        // A point just above the dip at Feb — between the first and third
        // tops, above the second — is inside no triangle.
        let feb = mesh.vertices[1].pos;
        let probe = egui::pos2(feb.x, feb.y - 2.0);
        let side = |a: egui::Pos2, b: egui::Pos2| {
            (b.x - a.x) * (probe.y - a.y) - (b.y - a.y) * (probe.x - a.x)
        };
        for tri in mesh.indices.chunks(3) {
            let (a, b, c) = (
                mesh.vertices[tri[0] as usize].pos,
                mesh.vertices[tri[1] as usize].pos,
                mesh.vertices[tri[2] as usize].pos,
            );
            let (u, v, w) = (side(a, b), side(b, c), side(c, a));
            assert!(
                !((u > 0.0 && v > 0.0 && w > 0.0) || (u < 0.0 && v < 0.0 && w < 0.0)),
                "the dip at Feb is filled over by {tri:?}"
            );
        }
    }

    #[test]
    fn a_document_chart_draws_from_its_cache_alone() {
        // A chart in a .docx has no cells behind it: whatever the producing
        // application wrote into `<c:numCache>` is the whole of the picture.
        let mut plot = plot(ChartKind::Bar);
        plot.series[0].values_ref = None;
        let series = chart::draw::cached_series(&plot);
        let bars = painted(&plot, &series)
            .iter()
            .filter(
                |shape| matches!(shape, egui::Shape::Rect(r) if r.fill == color(SERIES_COLORS[0])),
            )
            .count();
        assert_eq!(bars, 3);
    }

    #[test]
    fn a_label_is_placed_by_the_width_this_contexts_fonts_give_it() {
        // The geometry asks how wide a string is and centres it on the answer;
        // a painter that measured with one font and drew with another would
        // hang every title off the side of its chart.
        let mut with_title = plot(ChartKind::Bar);
        with_title.title = Some("Quarterly".to_string());
        let series = chart::draw::cached_series(&with_title);
        let title = painted(&with_title, &series)
            .into_iter()
            .find_map(|shape| match shape {
                egui::Shape::Text(text) if text.galley.text() == "Quarterly" => Some(text),
                _ => None,
            })
            .expect("the title is drawn");
        let width = title.galley.size().x;
        assert!(
            (title.pos.x - (200.0 - width / 2.0)).abs() < 0.5,
            "centred on the chart, not on a guess: {width} wide at {}",
            title.pos.x
        );
    }
}
