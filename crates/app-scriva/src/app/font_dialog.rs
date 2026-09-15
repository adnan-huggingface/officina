//! Format ▸ Font… (Ctrl+D): family, style, size, colour, highlight and the
//! effects the model writes, with a line of preview, applied to the
//! selection as one undo step.
//!
//! Only what changed since the box opened is applied — a box opened on a
//! word and applied to a paragraph of mixed faces states one thing about
//! them and leaves the rest of each run as it was, which is what Word's box
//! does with its blank fields.

use super::*;
use crate::toolbar::{HIGHLIGHTS, PALETTE, SIZES};
use ui_kit::menu;
use wp_model::prop::{Toggle, VertAlign};

/// The box as it stands: strings for what is typed, choices for the rest.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FontDraft {
    pub family: String,
    /// Regular, Bold, Italic, Bold Italic.
    pub style: usize,
    pub size: String,
    /// `None` is Automatic.
    pub colour: Option<[u8; 3]>,
    pub highlight: wp_model::Highlight,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub small_caps: bool,
    pub all_caps: bool,
}

const STYLES: [&str; 4] = ["Regular", "Bold", "Italic", "Bold Italic"];

impl Scriva {
    /// Opens the box on the selection, or the word at the caret, showing
    /// what it is: a face and size where they agree, blank where they do
    /// not.
    pub(super) fn open_font_dialog(&mut self) {
        let (bold, italic, _) = self.emphasis();
        let style = match (bold, italic) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        };
        let colour = match self.colour_at() {
            Some(wp_model::Color::Rgb(rgb)) => Some(rgb),
            _ => None,
        };
        let draft = FontDraft {
            family: self.face_at().unwrap_or_default(),
            style,
            size: self
                .size_at()
                .map(|half| crate::toolbar::size_label(half.0))
                .unwrap_or_default(),
            colour,
            highlight: self.highlight_at().unwrap_or(wp_model::Highlight::None),
            strike: self.struck(),
            superscript: self.probe_runs(|p| p.vert_align == Some(VertAlign::Superscript)),
            subscript: self.probe_runs(|p| p.vert_align == Some(VertAlign::Subscript)),
            small_caps: self.probe_runs(|p| p.toggles.is_on(Toggle::SmallCaps)),
            all_caps: self.probe_runs(|p| p.toggles.is_on(Toggle::Caps)),
        };
        self.font_draft = Some((draft.clone(), draft));
    }

    pub(super) fn font_dialog(&mut self, ctx: &egui::Context) {
        let Some((opened, mut draft)) = self.font_draft.clone() else {
            return;
        };
        let document_faces = crate::app::font_names(&self.document);
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-font"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(400.0);
                    ui.label(egui::RichText::new("Font").font(dialog::heading_font(16.0)));
                    ui.add_space(6.0);
                    dialog::labelled(ui, "Family:", |ui| {
                        let shown = match draft.family.is_empty() {
                            true => "(mixed)".to_owned(),
                            false => draft.family.clone(),
                        };
                        egui::ComboBox::from_id_salt("scriva-font-family")
                            .selected_text(shown)
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(300.0)
                                    .show(ui, |ui| {
                                        let families = ui_kit::catalogue::families();
                                        for name in document_faces.iter().chain(
                                            families.iter().filter(|f| !document_faces.contains(f)),
                                        ) {
                                            if ui
                                                .selectable_label(draft.family == *name, name)
                                                .clicked()
                                            {
                                                draft.family = name.clone();
                                            }
                                        }
                                    });
                            });
                    });
                    dialog::labelled(ui, "Style:", |ui| {
                        egui::ComboBox::from_id_salt("scriva-font-style")
                            .selected_text(STYLES[draft.style.min(3)])
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for (index, name) in STYLES.iter().enumerate() {
                                    if ui.selectable_label(draft.style == index, *name).clicked() {
                                        draft.style = index;
                                    }
                                }
                            });
                    });
                    dialog::labelled(ui, "Size:", |ui| {
                        dialog::first_unit_field(ui, "scriva-font", &mut draft.size, "pt", 64.0);
                        let picked = egui::ComboBox::from_id_salt("scriva-font-sizes")
                            .selected_text("")
                            .width(24.0)
                            .show_ui(ui, |ui| {
                                let mut picked = None;
                                for half in SIZES {
                                    let label = crate::toolbar::size_label(half);
                                    if ui.selectable_label(draft.size == label, &label).clicked() {
                                        picked = Some(label);
                                    }
                                }
                                picked
                            })
                            .inner
                            .flatten();
                        if let Some(label) = picked {
                            draft.size = label;
                        }
                    });
                    dialog::labelled(ui, "Colour:", |ui| {
                        let name = draft
                            .colour
                            .and_then(|rgb| PALETTE.iter().find(|(_, other)| *other == rgb))
                            .map(|(name, _)| *name)
                            .unwrap_or(match draft.colour {
                                Some(_) => "Custom",
                                None => "Automatic",
                            });
                        let response = swatch_button(ui, draft.colour, name);
                        let colours: Vec<(&str, egui::Color32)> = PALETTE
                            .iter()
                            .map(|(name, [r, g, b])| (*name, egui::Color32::from_rgb(*r, *g, *b)))
                            .collect();
                        let current = draft
                            .colour
                            .and_then(|rgb| PALETTE.iter().position(|(_, other)| *other == rgb));
                        if let Some(Some(pick)) = menu::under(&response, |ui| {
                            menu::swatches(ui, "&Automatic", &colours, current, None)
                        }) {
                            draft.colour = match pick {
                                menu::Swatch::First => None,
                                menu::Swatch::Index(index) => Some(PALETTE[index].1),
                                menu::Swatch::More => draft.colour,
                            };
                        }
                    });
                    dialog::labelled(ui, "Highlight:", |ui| {
                        let found = HIGHLIGHTS
                            .iter()
                            .find(|(_, value, _)| *value == draft.highlight);
                        let name = found.map(|(name, _, _)| *name).unwrap_or("None");
                        let response = swatch_button(ui, found.map(|(_, _, rgb)| *rgb), name);
                        let colours: Vec<(&str, egui::Color32)> = HIGHLIGHTS
                            .iter()
                            .map(|(name, _, [r, g, b])| {
                                (*name, egui::Color32::from_rgb(*r, *g, *b))
                            })
                            .collect();
                        let current = HIGHLIGHTS
                            .iter()
                            .position(|(_, value, _)| *value == draft.highlight);
                        if let Some(Some(pick)) = menu::under(&response, |ui| {
                            menu::swatches(ui, "&None", &colours, current, None)
                        }) {
                            draft.highlight = match pick {
                                menu::Swatch::First => wp_model::Highlight::None,
                                menu::Swatch::Index(index) => HIGHLIGHTS[index].1,
                                menu::Swatch::More => draft.highlight,
                            };
                        }
                    });
                    dialog::section(ui, "Effects");
                    dialog::labelled(ui, "", |ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.x = 16.0;
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut draft.strike, "Strikethrough");
                                ui.checkbox(&mut draft.small_caps, "Small caps");
                            });
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut draft.superscript, "Superscript").changed()
                                    && draft.superscript
                                {
                                    draft.subscript = false;
                                }
                                if ui.checkbox(&mut draft.subscript, "Subscript").changed()
                                    && draft.subscript
                                {
                                    draft.superscript = false;
                                }
                                ui.checkbox(&mut draft.all_caps, "All caps");
                            });
                        });
                    });
                    dialog::section(ui, "Preview");
                    preview(ui, &draft);
                    ui.add_space(4.0);
                    if let Some(answer) = dialog::submit(ui, "Apply") {
                        done = Some(answer);
                    }
                });
            });
        self.font_draft = Some((opened.clone(), draft.clone()));
        match done {
            Some(true) => {
                self.font_draft = None;
                self.apply_font(&opened, &draft);
            }
            Some(false) => self.font_draft = None,
            None => {}
        }
    }

    /// Applies what changed since the box opened, as one undo step.
    pub(super) fn apply_font(&mut self, opened: &FontDraft, draft: &FontDraft) {
        if opened == draft {
            return;
        }
        let family: Option<&str> =
            (opened.family != draft.family && !draft.family.is_empty()).then_some(&draft.family);
        let style = (opened.style != draft.style).then_some(draft.style);
        let size = (opened.size != draft.size)
            .then(|| crate::toolbar::parse_size(&draft.size))
            .flatten();
        let colour = (opened.colour != draft.colour).then_some(draft.colour);
        let highlight = (opened.highlight != draft.highlight).then_some(draft.highlight);
        let strike = (opened.strike != draft.strike).then_some(draft.strike);
        let vertical = (opened.superscript != draft.superscript
            || opened.subscript != draft.subscript)
            .then_some(match (draft.superscript, draft.subscript) {
                (true, _) => VertAlign::Superscript,
                (_, true) => VertAlign::Subscript,
                _ => VertAlign::Baseline,
            });
        let small_caps = (opened.small_caps != draft.small_caps).then_some(draft.small_caps);
        let all_caps = (opened.all_caps != draft.all_caps).then_some(draft.all_caps);
        self.format_runs(move |props| {
            if let Some(name) = family {
                props.fonts.ascii = Some(name.into());
                props.fonts.high_ansi = Some(name.into());
                props.fonts.ascii_theme = None;
                props.fonts.high_ansi_theme = None;
            }
            if let Some(style) = style {
                props.toggles.set(Toggle::Bold, style == 1 || style == 3);
                props.toggles.set(Toggle::Italic, style == 2 || style == 3);
            }
            if let Some(half) = size {
                props.size = Some(HalfPoint(half));
            }
            if let Some(colour) = colour {
                props.color = Some(match colour {
                    Some(rgb) => wp_model::Color::Rgb(rgb),
                    None => wp_model::Color::Auto,
                });
            }
            if let Some(highlight) = highlight {
                props.highlight = match highlight {
                    wp_model::Highlight::None => None,
                    chosen => Some(chosen),
                };
            }
            if let Some(strike) = strike {
                props.toggles.set(Toggle::Strike, strike);
            }
            if let Some(vertical) = vertical {
                props.vert_align = match vertical {
                    VertAlign::Baseline => None,
                    other => Some(other),
                };
            }
            if let Some(on) = small_caps {
                props.toggles.set(Toggle::SmallCaps, on);
            }
            if let Some(on) = all_caps {
                props.toggles.set(Toggle::Caps, on);
            }
        });
    }
}

/// A button showing a colour and its name, which opens the swatches.
fn swatch_button(ui: &mut egui::Ui, colour: Option<[u8; 3]>, name: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(150.0, 24.0), egui::Sense::click());
    ui.painter().rect(
        rect,
        ui_kit::theme::RADIUS_CONTROL as f32,
        ui_kit::theme::FIELD,
        egui::Stroke::new(
            1.0,
            match response.hovered() {
                true => ui_kit::theme::FIELD_EDGE_HOT,
                false => ui_kit::theme::FIELD_EDGE,
            },
        ),
        egui::StrokeKind::Inside,
    );
    let well = egui::Rect::from_min_size(rect.min + egui::vec2(5.0, 5.0), egui::vec2(14.0, 14.0));
    match colour {
        Some([r, g, b]) => {
            ui.painter()
                .rect_filled(well, 2.0, egui::Color32::from_rgb(r, g, b));
        }
        None => {
            ui.painter().rect_stroke(
                well,
                2.0,
                egui::Stroke::new(1.0, ui_kit::theme::INK_SOFT),
                egui::StrokeKind::Inside,
            );
        }
    }
    ui.painter().text(
        egui::pos2(well.right() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(ui_kit::theme::TEXT),
        ui_kit::theme::INK,
    );
    crate::icons::draw(
        ui.painter(),
        crate::icons::Icon::ChevronDown,
        egui::pos2(rect.right() - 10.0, rect.center().y),
        ui_kit::theme::INK_SOFT,
    );
    response
}

/// The pangram in the face, size, colour and effects the box describes,
/// where the face is on this machine.
fn preview(ui: &mut egui::Ui, draft: &FontDraft) {
    let bold = draft.style == 1 || draft.style == 3;
    let italic = draft.style == 2 || draft.style == 3;
    let family = ui_kit::fonts::named_face(&draft.family, bold, italic)
        .filter(|face| ui_kit::fonts::bound(ui.ctx(), face))
        .unwrap_or_else(|| {
            ui_kit::fonts::face(ui_kit::fonts::Family::of(&draft.family), bold, italic)
        });
    let size = crate::toolbar::parse_size(&draft.size)
        .map(|half| half as f32 / 2.0)
        .unwrap_or(12.0)
        .clamp(6.0, 40.0);
    let mut text = "The quick brown fox jumps over the lazy dog".to_owned();
    if draft.all_caps {
        text = text.to_uppercase();
    }
    let colour = match draft.colour {
        Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
        None => ui_kit::theme::INK,
    };
    let mut rich = egui::RichText::new(text)
        .font(egui::FontId::new(size, family))
        .color(colour);
    if draft.strike {
        rich = rich.strikethrough();
    }
    if let Some((_, _, [r, g, b])) = HIGHLIGHTS
        .iter()
        .find(|(_, value, _)| *value == draft.highlight)
    {
        rich = rich.background_color(egui::Color32::from_rgb(*r, *g, *b));
    }
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        ui_kit::theme::RADIUS_CONTROL as f32,
        egui::Color32::WHITE,
        egui::Stroke::new(1.0, ui_kit::theme::FIELD_EDGE),
        egui::StrokeKind::Inside,
    );
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)));
    inner.centered_and_justified(|ui| {
        ui.add(egui::Label::new(rich).truncate());
    });
}
