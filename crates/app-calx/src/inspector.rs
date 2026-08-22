//! The chart inspector: a panel beside the grid for as long as a chart is
//! selected.
//!
//! There is no dialog and no Apply. Every control changes the chart the
//! moment it is touched, so the chart on the sheet is the preview, and a
//! change is undone the way it was made: one entry per *gesture* — a slider
//! dragged, a colour chosen, a kind picked — rather than one per frame the
//! slider moved through. The plot as it stood when the gesture began is kept
//! here until the gesture ends, and is what the undo puts back.
//!
//! What the panel offers is exactly what the writer can carry into the file:
//! a chart from Excel keeps everything the inspector does not show, byte for
//! byte (`ss_xlsx::write::chart_out`).

use ss_formula::edit::{self, Change, Patch};
use ss_model::chart::{excel_defaults, ChartKind, Grouping, LegendPosition, Plot, Symbol};
use ui_kit::chart::SERIES_COLORS;
use ui_kit::{dialog, egui};

use crate::Calx;

/// What the inspector holds between frames.
#[derive(Default)]
pub(crate) struct State {
    /// The title box's buffer. Kept out of the model so that typing does not
    /// push an undo entry per keystroke: the text is a title when the box is
    /// left, not before.
    pub(crate) title: Option<String>,
    /// The plot as it was when the current gesture began — sheet, chart, and
    /// everything it plotted — pushed to the undo stack when the gesture
    /// ends.
    pub(crate) before: Option<(usize, usize, Plot)>,
}

/// The kinds a chart can be turned into, as the Insert menu names them.
///
/// Scatter is not among them: a scatter takes its first column as numbers,
/// and a chart whose categories are months has no numbers to take.
const KINDS: &[(&str, ChartKind, Grouping, bool)] = &[
    (
        "Clustered column",
        ChartKind::Bar,
        Grouping::Clustered,
        false,
    ),
    ("Stacked column", ChartKind::Bar, Grouping::Stacked, false),
    (
        "100% stacked column",
        ChartKind::Bar,
        Grouping::PercentStacked,
        false,
    ),
    ("Clustered bar", ChartKind::Bar, Grouping::Clustered, true),
    ("Stacked bar", ChartKind::Bar, Grouping::Stacked, true),
    (
        "100% stacked bar",
        ChartKind::Bar,
        Grouping::PercentStacked,
        true,
    ),
    ("Line", ChartKind::Line, Grouping::Standard, false),
    ("Area", ChartKind::Area, Grouping::Standard, false),
    ("Stacked area", ChartKind::Area, Grouping::Stacked, false),
    (
        "100% stacked area",
        ChartKind::Area,
        Grouping::PercentStacked,
        false,
    ),
    ("Pie", ChartKind::Pie, Grouping::Standard, false),
    ("Doughnut", ChartKind::Doughnut, Grouping::Standard, false),
    ("Radar", ChartKind::Radar, Grouping::Standard, false),
];

const LEGENDS: &[(&str, Option<LegendPosition>)] = &[
    ("None", None),
    ("Right", Some(LegendPosition::Right)),
    ("Left", Some(LegendPosition::Left)),
    ("Top", Some(LegendPosition::Top)),
    ("Bottom", Some(LegendPosition::Bottom)),
    ("Top right", Some(LegendPosition::TopRight)),
];

const SYMBOLS: &[(&str, Symbol)] = &[
    ("Automatic", Symbol::Auto),
    ("None", Symbol::None),
    ("Circle", Symbol::Circle),
    ("Square", Symbol::Square),
    ("Diamond", Symbol::Diamond),
    ("Triangle", Symbol::Triangle),
    ("X", Symbol::X),
    ("Star", Symbol::Star),
    ("Plus", Symbol::Plus),
    ("Dash", Symbol::Dash),
    ("Dot", Symbol::Dot),
];

/// The name the Insert menu gives a chart's kind, or none for a kind the
/// menu does not offer.
///
/// A pie, a doughnut or a radar has no grouping element in its part and no
/// direction, so the reader leaves both at their defaults; they are matched
/// on the kind alone, or every pie would come up "Unsupported type".
fn kind_name(plot: &Plot) -> Option<&'static str> {
    let grouped = matches!(
        plot.kind,
        ChartKind::Bar | ChartKind::Line | ChartKind::Area
    );
    KINDS
        .iter()
        .find(|(_, kind, grouping, horizontal)| {
            *kind == plot.kind
                && (!grouped || *grouping == plot.grouping)
                && (plot.kind != ChartKind::Bar || *horizontal == plot.horizontal)
        })
        .map(|(name, ..)| *name)
}

impl Calx {
    /// The panel, drawn before the grid so that the grid takes what is left.
    pub(crate) fn chart_panel(&mut self, ui: &mut egui::Ui) {
        let sheet = self.grid.sheet_index;
        let Some(index) = self.grid.selected_chart else {
            self.inspector.title = None;
            // A chart deselected mid-gesture — by a click on the sheet,
            // say — still owes its undo entry.
            self.settle_chart_gesture(true);
            return;
        };
        if self
            .doc
            .workbook
            .sheet(sheet)
            .and_then(|s| s.charts.get(index))
            .is_none()
        {
            return;
        }

        egui::Panel::right("calx-chart-inspector")
            .resizable(false)
            .exact_size(272.0)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 14,
                        top: 10,
                        bottom: 12,
                    }),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Chart").font(dialog::heading_font(16.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("×").size(18.0)).frame(false),
                            )
                            .on_hover_text("Deselect (Esc)")
                            .clicked()
                        {
                            self.grid.selected_chart = None;
                        }
                    });
                });
                ui.add_space(2.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.chart_controls(ui, sheet, index));
            });

        // The gesture ends when nothing is being dragged, typed into, or
        // picked from: a slider released, a colour popup closed.
        let busy = ui.input(|i| i.pointer.any_down())
            || egui::Popup::is_any_open(ui.ctx())
            || ui
                .ctx()
                .memory(|m| m.focused())
                .is_some_and(|id| id != egui::Id::new("calx-chart-title"));
        self.settle_chart_gesture(!busy);
    }

    fn chart_controls(&mut self, ui: &mut egui::Ui, sheet: usize, index: usize) {
        let plot = self
            .doc
            .workbook
            .sheet(sheet)
            .and_then(|s| s.charts.get(index))
            .map(|c| c.plot.clone());
        let Some(plot) = plot else { return };

        egui::Grid::new("calx-chart-form")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("Type");
                let current = kind_name(&plot);
                let mut chosen = None;
                ui.add_enabled_ui(current.is_some() || plot.kind != ChartKind::Scatter, |ui| {
                    egui::ComboBox::from_id_salt("calx-chart-kind")
                        .selected_text(current.unwrap_or(match plot.kind {
                            ChartKind::Scatter => "Scatter",
                            _ => "Unsupported type",
                        }))
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for (name, kind, grouping, horizontal) in KINDS {
                                if ui.selectable_label(current == Some(*name), *name).clicked() {
                                    chosen = Some((kind.clone(), *grouping, *horizontal));
                                }
                            }
                        });
                });
                if let Some((kind, grouping, horizontal)) = chosen {
                    self.chart_edit(|plot| {
                        plot.kind = kind;
                        plot.grouping = grouping;
                        plot.horizontal = horizontal;
                        // A doughnut gets its hole, a stack its overlap, a
                        // line its straight segments: what Excel's own
                        // Insert would have given the new kind.
                        excel_defaults(plot);
                    });
                }
                ui.end_row();

                ui.label("Title");
                self.chart_title_box(ui, sheet, index, &plot);
                ui.end_row();

                ui.label("Legend");
                let legend = LEGENDS
                    .iter()
                    .find(|(_, p)| *p == plot.legend)
                    .map_or("Right", |(name, _)| *name);
                let mut picked = None;
                egui::ComboBox::from_id_salt("calx-chart-legend")
                    .selected_text(legend)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for (name, position) in LEGENDS {
                            if ui.selectable_label(legend == *name, *name).clicked() {
                                picked = Some(*position);
                            }
                        }
                    });
                if let Some(position) = picked {
                    self.chart_edit(|plot| plot.legend = position);
                }
                ui.end_row();
            });

        self.series_rows(ui, &plot);

        match plot.kind {
            ChartKind::Bar => {
                section(ui, "Bars");
                let mut gap = plot.gap;
                if slider(ui, "Gap width", &mut gap, 0.0..=500.0) {
                    self.chart_edit(|plot| plot.gap = gap);
                }
                let mut overlap = plot.overlap;
                if slider(ui, "Overlap", &mut overlap, -100.0..=100.0) {
                    self.chart_edit(|plot| plot.overlap = overlap);
                }
            }
            ChartKind::Doughnut => {
                section(ui, "Doughnut");
                let mut hole = plot.hole;
                if slider(ui, "Hole size", &mut hole, 10.0..=90.0) {
                    self.chart_edit(|plot| plot.hole = hole);
                }
            }
            ChartKind::Line | ChartKind::Scatter | ChartKind::Radar => {
                section(ui, "Lines");
                ui.horizontal(|ui| {
                    let mut markers = plot.markers;
                    if ui.checkbox(&mut markers, "Markers").changed() {
                        self.chart_edit(|plot| plot.markers = markers);
                    }
                    if plot.kind != ChartKind::Radar {
                        let mut smooth = plot.series.iter().all(|s| s.smooth != Some(false));
                        if ui.checkbox(&mut smooth, "Smooth").changed() {
                            self.chart_edit(|plot| {
                                for series in &mut plot.series {
                                    series.smooth = Some(smooth);
                                }
                            });
                        }
                    }
                    if plot.kind == ChartKind::Scatter {
                        let mut lines = plot.scatter_lines;
                        if ui.checkbox(&mut lines, "Lines").changed() {
                            self.chart_edit(|plot| plot.scatter_lines = lines);
                        }
                    }
                });
            }
            _ => {}
        }

        if plot.kind.has_axes() {
            section(ui, "Axes");
            for (label, value) in [("Value", true), ("Category", false)] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    let axis = if value { plot.val_axis } else { plot.cat_axis };
                    let mut shown = !axis.deleted;
                    if ui.checkbox(&mut shown, "Shown").changed() {
                        self.chart_edit(|plot| {
                            let axis = if value {
                                &mut plot.val_axis
                            } else {
                                &mut plot.cat_axis
                            };
                            axis.deleted = !shown;
                        });
                    }
                    let mut gridlines = axis.gridlines;
                    if ui.checkbox(&mut gridlines, "Gridlines").changed() {
                        self.chart_edit(|plot| {
                            let axis = if value {
                                &mut plot.val_axis
                            } else {
                                &mut plot.cat_axis
                            };
                            axis.gridlines = gridlines;
                        });
                    }
                });
            }
        }

        ui.add_space(8.0);
        let mut vary = plot.vary_colors;
        if ui
            .checkbox(&mut vary, "Vary colours by point")
            .on_hover_text("Each bar, slice or marker in its own colour")
            .changed()
        {
            self.chart_edit(|plot| plot.vary_colors = vary);
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(4.0);
        if ui.button("Delete chart").clicked() {
            self.delete_chart(sheet, index);
        }
    }

    fn chart_title_box(&mut self, ui: &mut egui::Ui, sheet: usize, index: usize, plot: &Plot) {
        let existing = plot.title.clone().unwrap_or_default();
        if self.inspector.title.is_none() {
            self.inspector.title = Some(existing.clone());
        }
        let buffer = self.inspector.title.get_or_insert_with(String::new);
        let response = ui.add(
            egui::TextEdit::singleline(buffer)
                .id(egui::Id::new("calx-chart-title"))
                .desired_width(ui.available_width())
                .hint_text("(none)"),
        );
        // On losing focus rather than on every keystroke: one undo entry
        // per title, not one per letter.
        if response.lost_focus() && *buffer != existing {
            let change = edit::chart_title(sheet, index, buffer);
            self.perform(change);
            self.inspector.title = None;
        }
    }

    fn series_rows(&mut self, ui: &mut egui::Ui, plot: &Plot) {
        section(ui, "Series");
        let marks = matches!(
            plot.kind,
            ChartKind::Line | ChartKind::Scatter | ChartKind::Radar
        );
        egui::Grid::new("calx-chart-series")
            .num_columns(if marks { 4 } else { 3 })
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for (i, series) in plot.series.iter().enumerate() {
                    let mut rgb = series
                        .color
                        .unwrap_or(SERIES_COLORS[i % SERIES_COLORS.len()]);
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        self.chart_edit(|plot| plot.series[i].color = Some(rgb));
                    }
                    let name = series
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Series {}", i + 1));
                    ui.add(egui::Label::new(name).truncate());
                    if marks {
                        let current = SYMBOLS
                            .iter()
                            .find(|(_, s)| *s == series.symbol)
                            .map_or("Automatic", |(name, _)| *name);
                        let mut picked = None;
                        egui::ComboBox::from_id_salt(("calx-chart-symbol", i))
                            .selected_text(current)
                            .width(92.0)
                            .show_ui(ui, |ui| {
                                for (name, symbol) in SYMBOLS {
                                    if ui.selectable_label(current == *name, *name).clicked() {
                                        picked = Some(*symbol);
                                    }
                                }
                            });
                        if let Some(symbol) = picked {
                            self.chart_edit(|plot| plot.series[i].symbol = symbol);
                        }
                    }
                    // A way back to the theme's colour, which no picker can
                    // express.
                    if series.color.is_some()
                        && ui
                            .small_button("Auto")
                            .on_hover_text("Back to the automatic colour")
                            .clicked()
                    {
                        self.chart_edit(|plot| plot.series[i].color = None);
                    }
                    ui.end_row();
                }
            });
    }

    /// Changes the selected chart's plot in place, opening a gesture if none
    /// is open. The undo entry is pushed when the gesture settles.
    pub(crate) fn chart_edit(&mut self, apply: impl FnOnce(&mut Plot)) {
        let sheet = self.grid.sheet_index;
        let Some(index) = self.grid.selected_chart else {
            return;
        };
        let Some(chart) = self
            .doc
            .workbook
            .sheet_mut(sheet)
            .and_then(|s| s.charts.get_mut(index))
        else {
            return;
        };
        if self.inspector.before.is_none() {
            self.inspector.before = Some((sheet, index, chart.plot.clone()));
        }
        apply(&mut chart.plot);
        self.edited = true;
    }

    /// Ends the open gesture, if `finished`: one undo entry for all it
    /// changed, or none if it changed nothing after all.
    pub(crate) fn settle_chart_gesture(&mut self, finished: bool) {
        if !finished || self.inspector.before.is_none() {
            return;
        }
        let Some((sheet, index, before)) = self.inspector.before.take() else {
            return;
        };
        let now = self
            .doc
            .workbook
            .sheet(sheet)
            .and_then(|s| s.charts.get(index))
            .map(|c| &c.plot);
        if now.is_none_or(|now| *now == before) {
            return;
        }
        self.undo.push(Change::new(
            "Format chart",
            vec![Patch::ChartPlot {
                sheet,
                chart: index,
                plot: Box::new(before),
            }],
        ));
        self.redo.clear();
        self.status = "Chart formatted".to_string();
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).font(dialog::heading_font(12.5)));
}

/// A labelled percentage slider; true when it moved this frame.
fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized([70.0, 18.0], egui::Label::new(label));
        ui.spacing_mut().slider_width = (ui.available_width() - 56.0).max(60.0);
        ui.add(
            egui::Slider::new(value, range)
                .suffix("%")
                .fixed_decimals(0)
                .trailing_fill(true),
        )
        .changed()
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use calx::grid;
    use ss_model::CellRef;

    /// An app with a two-column chart selected.
    fn with_chart() -> Calx {
        let mut app = Calx::new();
        for (at, text) in [
            ("A1", "Jan"),
            ("B1", "10"),
            ("A2", "Feb"),
            ("B2", "6"),
            ("A3", "Mar"),
            ("B3", "15"),
        ] {
            let at = CellRef::from_a1(at).expect("valid");
            let change = edit::input(&mut app.doc.workbook, 0, at, text);
            app.perform(change);
        }
        app.grid.selection = grid::Selection::at(CellRef::from_a1("A1").expect("valid"));
        app.grid.selection.extend_to(
            CellRef::from_a1("B3").expect("valid"),
            app.doc.workbook.sheet(0).expect("sheet 0"),
        );
        app.insert_chart(ChartKind::Bar, Grouping::Clustered, false);
        assert_eq!(app.grid.selected_chart, Some(0));
        app
    }

    fn plot(app: &Calx) -> &Plot {
        &app.doc.workbook.sheet(0).expect("sheet 0").charts[0].plot
    }

    #[test]
    fn a_gesture_of_many_frames_is_one_undo_entry_and_undo_puts_the_plot_back() {
        let mut app = with_chart();
        let entries = app.undo.len();
        // A slider dragged through three frames.
        app.chart_edit(|p| p.gap = 120.0);
        app.settle_chart_gesture(false);
        app.chart_edit(|p| p.gap = 90.0);
        app.settle_chart_gesture(false);
        app.chart_edit(|p| p.gap = 60.0);
        assert_eq!(
            app.undo.len(),
            entries,
            "nothing on the stack while the drag is on"
        );
        app.settle_chart_gesture(true);
        assert_eq!(app.undo.len(), entries + 1, "one entry when it ends");
        assert_eq!(plot(&app).gap, 60.0);
        app.undo();
        assert_eq!(plot(&app).gap, 150.0, "back to where the drag began");
        app.redo();
        assert_eq!(plot(&app).gap, 60.0);
    }

    /// One egui frame of the whole app, with these events.
    fn frame(app: &mut Calx, ctx: &egui::Context, events: Vec<egui::Event>) {
        use ui_kit::DocumentApp;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    #[test]
    fn a_click_in_the_title_box_focuses_it_and_keeps_the_chart_selected() {
        // Found at the keyboard: the click that should have put the caret in
        // the title box deselected the chart, the panel vanished under the
        // pointer, and the title went into the cell that had been behind it.
        let ctx = egui::Context::default();
        ui_kit::fonts::register(&ctx, &[]);
        let mut app = with_chart();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let title = ctx
            .read_response(egui::Id::new("calx-chart-title"))
            .expect("the title box is on screen")
            .rect;
        let inside = title.center();
        // As the platform delivers a click: the move and the press in one
        // frame, and the release hard on their heels.
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(inside), press(inside, true)],
        );
        frame(&mut app, &ctx, vec![press(inside, false)]);
        assert_eq!(
            app.grid.selected_chart,
            Some(0),
            "the chart is still selected"
        );
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("calx-chart-title")),
            "and the box has the caret"
        );
    }

    #[test]
    fn enter_in_the_title_box_sets_the_title_and_leaves_the_chart_selected() {
        // Found at the keyboard: the box gave its focus up on Enter while the
        // panel was being drawn, the grid found nothing focused, took the
        // Enter as its own, and deselected the chart.
        let ctx = egui::Context::default();
        ui_kit::fonts::register(&ctx, &[]);
        let mut app = with_chart();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let inside = ctx
            .read_response(egui::Id::new("calx-chart-title"))
            .expect("the title box is on screen")
            .rect
            .center();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(inside), press(inside, true)],
        );
        frame(&mut app, &ctx, vec![press(inside, false)]);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("Sales by month".to_string())],
        );
        let enter = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, &ctx, vec![enter(true)]);
        frame(&mut app, &ctx, vec![enter(false)]);
        assert_eq!(
            app.grid.selected_chart,
            Some(0),
            "the chart is still selected"
        );
        assert_eq!(plot(&app).title.as_deref(), Some("Sales by month"));
        assert!(
            matches!(
                app.doc
                    .workbook
                    .sheet(0)
                    .expect("sheet 0")
                    .get(CellRef::new(0, 0))
                    .map(|c| c.value),
                Some(ss_model::CellValue::Text(_))
            ),
            "and nothing was typed into the sheet"
        );
    }

    #[test]
    fn a_gesture_that_changed_nothing_leaves_no_entry() {
        let mut app = with_chart();
        let entries = app.undo.len();
        app.chart_edit(|p| p.series[0].color = Some([1, 2, 3]));
        app.chart_edit(|p| p.series[0].color = None);
        app.settle_chart_gesture(true);
        assert_eq!(app.undo.len(), entries);
    }

    #[test]
    fn a_new_kind_brings_what_excels_insert_would_give_it() {
        let mut app = with_chart();
        app.chart_edit(|p| {
            p.kind = ChartKind::Doughnut;
            p.grouping = Grouping::Standard;
            excel_defaults(p);
        });
        app.settle_chart_gesture(true);
        assert_eq!(plot(&app).hole, 75.0, "a doughnut's hole");
        assert!(plot(&app).vary_colors, "and a slice per colour");
        assert_eq!(kind_name(plot(&app)), Some("Doughnut"));
        // As read from a file, where a pie states no grouping at all.
        let mut read = plot(&app).clone();
        read.grouping = Grouping::Clustered;
        assert_eq!(kind_name(&read), Some("Doughnut"));
        read.kind = ChartKind::Radar;
        assert_eq!(kind_name(&read), Some("Radar"));
    }
}
