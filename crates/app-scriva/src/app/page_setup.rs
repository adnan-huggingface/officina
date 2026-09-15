//! Layout ▸ Page Setup…: paper, orientation, margins and the bands' distance
//! from the edge in one box, with the page drawn from the numbers as they
//! are typed.
//!
//! One box rather than the old Custom Margins plus three preset submenus,
//! because the four are one decision — a landscape A4 with narrow margins
//! is chosen together — and Word's own box holds them together. Applied to
//! the section as one undo step, whatever changed.

use super::*;
use wp_model::Orientation;

/// The box as typed. Every number is text until Apply, so that a half-typed
/// field is a half-typed field and not a zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageSetupDraft {
    pub top: String,
    pub bottom: String,
    pub left: String,
    pub right: String,
    pub header: String,
    pub footer: String,
    /// Which of [`PAPERS`] the paper is, or its length for Custom.
    pub paper: usize,
    /// The paper's portrait width and height, in the field's inches.
    pub width: String,
    pub height: String,
    pub landscape: bool,
}

/// The papers the combo offers, in portrait twips.
pub(crate) const PAPERS: [(&str, i32, i32); 3] = [
    ("Letter", 12240, 15840),
    ("Legal", 12240, 20160),
    ("A4", 11906, 16838),
];

impl Scriva {
    /// Opens the box on the section the caret is in, showing what it has.
    pub(super) fn open_page_setup(&mut self) {
        let section = &self.document.section;
        let inch = |t: Twips| dialog::inches(t.0 as f64 / 1440.0);
        let m = section.margins;
        let (w, h) = match section.page.orientation {
            Orientation::Portrait => (section.page.width, section.page.height),
            Orientation::Landscape => (section.page.height, section.page.width),
        };
        let paper = PAPERS
            .iter()
            .position(|(_, pw, ph)| Twips(*pw) == w && Twips(*ph) == h)
            .unwrap_or(PAPERS.len());
        self.page_setup = Some(PageSetupDraft {
            top: inch(m.top),
            bottom: inch(m.bottom),
            left: inch(m.start),
            right: inch(m.end),
            header: inch(m.header),
            footer: inch(m.footer),
            paper,
            width: inch(w),
            height: inch(h),
            landscape: section.page.orientation == Orientation::Landscape,
        });
    }

    pub(super) fn page_setup_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.page_setup.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-page-setup"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(440.0);
                    ui.label(egui::RichText::new("Page Setup").font(dialog::heading_font(16.0)));
                    ui.add_space(6.0);
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            dialog::section(ui, "Margins");
                            dialog::labelled(ui, "Top:", |ui| {
                                dialog::first_unit_field(
                                    ui,
                                    "scriva-page-setup",
                                    &mut draft.top,
                                    "in",
                                    72.0,
                                );
                            });
                            for (label, field) in [
                                ("Bottom:", &mut draft.bottom),
                                ("Left:", &mut draft.left),
                                ("Right:", &mut draft.right),
                            ] {
                                dialog::labelled(ui, label, |ui| {
                                    dialog::unit_field(ui, field, "in", 72.0);
                                });
                            }
                            dialog::section(ui, "From edge");
                            for (label, field) in [
                                ("Header:", &mut draft.header),
                                ("Footer:", &mut draft.footer),
                            ] {
                                dialog::labelled(ui, label, |ui| {
                                    dialog::unit_field(ui, field, "in", 72.0);
                                });
                            }
                            dialog::section(ui, "Paper");
                            dialog::labelled(ui, "Size:", |ui| {
                                let name = PAPERS
                                    .get(draft.paper)
                                    .map(|(name, _, _)| *name)
                                    .unwrap_or("Custom");
                                egui::ComboBox::from_id_salt("scriva-page-setup-paper")
                                    .selected_text(name)
                                    .width(120.0)
                                    .show_ui(ui, |ui| {
                                        for (index, (name, w, h)) in PAPERS.iter().enumerate() {
                                            if ui
                                                .selectable_label(draft.paper == index, *name)
                                                .clicked()
                                            {
                                                draft.paper = index;
                                                draft.width = dialog::inches(*w as f64 / 1440.0);
                                                draft.height = dialog::inches(*h as f64 / 1440.0);
                                            }
                                        }
                                        if ui
                                            .selectable_label(draft.paper == PAPERS.len(), "Custom")
                                            .clicked()
                                        {
                                            draft.paper = PAPERS.len();
                                        }
                                    });
                            });
                            // Typing a size is choosing Custom: the combo says
                            // what the numbers are, not the other way round.
                            dialog::labelled(ui, "Width:", |ui| {
                                if dialog::unit_field(ui, &mut draft.width, "in", 72.0).changed() {
                                    draft.paper = PAPERS.len();
                                }
                            });
                            dialog::labelled(ui, "Height:", |ui| {
                                if dialog::unit_field(ui, &mut draft.height, "in", 72.0).changed() {
                                    draft.paper = PAPERS.len();
                                }
                            });
                            dialog::section(ui, "Orientation");
                            dialog::labelled(ui, "", |ui| {
                                for landscape in [false, true] {
                                    if orientation_button(
                                        ui,
                                        landscape,
                                        draft.landscape == landscape,
                                    )
                                    .clicked()
                                    {
                                        draft.landscape = landscape;
                                    }
                                }
                            });
                        });
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            dialog::section(ui, "Preview");
                            preview(ui, &draft);
                        });
                    });
                    ui.add_space(4.0);
                    if let Some(answer) = dialog::submit(ui, "Apply") {
                        done = Some(answer);
                    }
                });
            });
        self.page_setup = Some(draft.clone());
        match done {
            Some(true) => {
                self.page_setup = None;
                self.apply_page_setup(&draft);
            }
            Some(false) => self.page_setup = None,
            None => {}
        }
    }

    /// Everything the box says, on the section, as one undo step. A field
    /// that does not parse keeps what the section had.
    pub(super) fn apply_page_setup(&mut self, draft: &PageSetupDraft) {
        let mut section = self.document.section.clone();
        let twips = |text: &str, was: Twips, most: f64| {
            dialog::measure(text)
                .filter(|v| (0.0..=most).contains(v))
                .map(|v| Twips((v * 1440.0).round() as i32))
                .unwrap_or(was)
        };
        let m = section.margins;
        section.margins.top = twips(&draft.top, m.top, 5.0);
        section.margins.bottom = twips(&draft.bottom, m.bottom, 5.0);
        section.margins.start = twips(&draft.left, m.start, 5.0);
        section.margins.end = twips(&draft.right, m.end, 5.0);
        section.margins.header = twips(&draft.header, m.header, 5.0);
        section.margins.footer = twips(&draft.footer, m.footer, 5.0);
        let (was_w, was_h) = match section.page.orientation {
            Orientation::Portrait => (section.page.width, section.page.height),
            Orientation::Landscape => (section.page.height, section.page.width),
        };
        let (w, h) = match PAPERS.get(draft.paper) {
            Some((_, w, h)) => (Twips(*w), Twips(*h)),
            None => (
                twips(&draft.width, was_w, 22.0),
                twips(&draft.height, was_h, 22.0),
            ),
        };
        if (w, h) != (was_w, was_h) {
            // The old paper's printer-tray code would now be a lie.
            section.page.code = None;
        }
        let orientation = match draft.landscape {
            true => Orientation::Landscape,
            false => Orientation::Portrait,
        };
        // The model's width is the printed width: landscape stores them
        // swapped, with the orientation beside them.
        (section.page.width, section.page.height) = match orientation {
            Orientation::Portrait => (w, h),
            Orientation::Landscape => (h, w),
        };
        section.page.orientation = orientation;
        if section != self.document.section {
            self.set_section(section);
        }
    }
}

/// A toggle with a page glyph, portrait or landscape, lit when chosen.
fn orientation_button(ui: &mut egui::Ui, landscape: bool, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(72.0, 44.0), egui::Sense::click());
    crate::icons::paint_state(ui, rect, &response, on);
    let page = match landscape {
        true => egui::vec2(22.0, 16.0),
        false => egui::vec2(16.0, 22.0),
    };
    let paper = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 6.0), page);
    ui.painter().rect(
        paper,
        1.0,
        egui::Color32::WHITE,
        egui::Stroke::new(1.0, ui_kit::theme::INK),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 3.0),
        egui::Align2::CENTER_BOTTOM,
        match landscape {
            true => "Landscape",
            false => "Portrait",
        },
        egui::FontId::proportional(ui_kit::theme::TEXT_SMALL),
        ui_kit::theme::INK,
    );
    response.on_hover_text(match landscape {
        true => "The page on its side",
        false => "The page upright",
    })
}

/// The page as the numbers describe it, drawn to fit a 120-point square:
/// the paper, the text area inside the margins, and the bands' lines.
fn preview(ui: &mut egui::Ui, draft: &PageSetupDraft) {
    const BOX: f32 = 120.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(BOX, BOX), egui::Sense::hover());
    let (mut w, mut h) = match PAPERS.get(draft.paper) {
        Some((_, w, h)) => (*w as f64 / 1440.0, *h as f64 / 1440.0),
        None => (
            dialog::measure(&draft.width).unwrap_or(8.5).max(0.5),
            dialog::measure(&draft.height).unwrap_or(11.0).max(0.5),
        ),
    };
    if draft.landscape {
        std::mem::swap(&mut w, &mut h);
    }
    let scale = (BOX as f64 / w.max(h)) as f32;
    let paper = egui::Rect::from_center_size(
        rect.center(),
        egui::vec2(w as f32 * scale, h as f32 * scale),
    );
    ui.painter().rect(
        paper,
        0.0,
        egui::Color32::WHITE,
        egui::Stroke::new(1.0, ui_kit::theme::PAGE_EDGE),
        egui::StrokeKind::Inside,
    );
    let inch = |text: &str| dialog::measure(text).unwrap_or(1.0).clamp(0.0, 5.0) as f32 * scale;
    let text = egui::Rect::from_min_max(
        paper.min + egui::vec2(inch(&draft.left), inch(&draft.top)),
        paper.max - egui::vec2(inch(&draft.right), inch(&draft.bottom)),
    );
    if text.is_positive() {
        ui.painter().rect_stroke(
            text,
            0.0,
            egui::Stroke::new(1.0, ui_kit::theme::FIELD_EDGE_HOT),
            egui::StrokeKind::Inside,
        );
    }
    let soft = egui::Stroke::new(1.0, ui_kit::theme::INK_FAINT);
    for y in [
        paper.top() + inch(&draft.header),
        paper.bottom() - inch(&draft.footer),
    ] {
        if y > paper.top() && y < paper.bottom() {
            ui.painter()
                .hline(paper.left() + 3.0..=paper.right() - 3.0, y, soft);
        }
    }
}
