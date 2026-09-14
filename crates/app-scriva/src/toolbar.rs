//! The toolbar: one row, on which the caret's state can be read at a glance.
//!
//! Style, face and size are combos showing what the caret is in; the toggles
//! show what it has; colour and highlight are split buttons carrying the
//! colour they will apply. One row rather than two, because the page loses
//! thirty-six points on every window for a second row of controls that are
//! all in the menus as well — so what does not fit folds, from the right,
//! into an overflow menu whose rows carry their keys. Nothing is ever
//! unreachable, and the test at 800 points wide says so.
//!
//! Every control returns a [`Command`] and does nothing itself, for the reason
//! `menus.rs` gives: the menu, the toolbar and the keyboard arrive at one
//! dispatcher and cannot answer the same command differently.

use ui_kit::{egui, menu, theme};
use wp_model::prop::Justify;
use wp_model::units::{HalfPoint, Line240};

use crate::app::{Command, Scriva};
use crate::commands::{self, tooltip, tooltip_named};
use crate::icons::{self, Icon};

/// Font sizes the size box offers — Word's own list, in half-points because
/// the list has 10.5 in it and a whole-point list cannot say so.
pub(crate) const SIZES: [i32; 17] = [
    16, 18, 20, 21, 22, 24, 28, 32, 36, 40, 44, 48, 56, 64, 72, 96, 144,
];

/// Word's standard-colours row, under Word's own names.
pub(crate) const PALETTE: [(&str, [u8; 3]); 13] = [
    ("Black", [0x00, 0x00, 0x00]),
    ("Dark Red", [0xC0, 0x00, 0x00]),
    ("Red", [0xFF, 0x00, 0x00]),
    ("Orange", [0xFF, 0xC0, 0x00]),
    ("Yellow", [0xFF, 0xFF, 0x00]),
    ("Light Green", [0x92, 0xD0, 0x50]),
    ("Green", [0x00, 0xB0, 0x50]),
    ("Light Blue", [0x00, 0xB0, 0xF0]),
    ("Blue", [0x00, 0x70, 0xC0]),
    ("Dark Blue", [0x00, 0x20, 0x60]),
    ("Purple", [0x70, 0x30, 0xA0]),
    ("Gray", [0x80, 0x80, 0x80]),
    ("White", [0xFF, 0xFF, 0xFF]),
];

/// The marker-pen palette in Word's gallery order, under the names Word's
/// tooltips use — which are not the names the attribute values use — and
/// the colour each one draws.
pub(crate) const HIGHLIGHTS: [(&str, wp_model::Highlight, [u8; 3]); 15] = [
    ("Yellow", wp_model::Highlight::Yellow, [0xFF, 0xFF, 0x00]),
    (
        "Bright Green",
        wp_model::Highlight::Green,
        [0x00, 0xFF, 0x00],
    ),
    ("Turquoise", wp_model::Highlight::Cyan, [0x00, 0xFF, 0xFF]),
    ("Pink", wp_model::Highlight::Magenta, [0xFF, 0x00, 0xFF]),
    ("Blue", wp_model::Highlight::Blue, [0x00, 0x00, 0xFF]),
    ("Red", wp_model::Highlight::Red, [0xFF, 0x00, 0x00]),
    (
        "Dark Blue",
        wp_model::Highlight::DarkBlue,
        [0x00, 0x00, 0x80],
    ),
    ("Teal", wp_model::Highlight::DarkCyan, [0x00, 0x80, 0x80]),
    ("Green", wp_model::Highlight::DarkGreen, [0x00, 0x80, 0x00]),
    (
        "Violet",
        wp_model::Highlight::DarkMagenta,
        [0x80, 0x00, 0x80],
    ),
    ("Dark Red", wp_model::Highlight::DarkRed, [0x80, 0x00, 0x00]),
    (
        "Dark Yellow",
        wp_model::Highlight::DarkYellow,
        [0x80, 0x80, 0x00],
    ),
    (
        "Gray 50%",
        wp_model::Highlight::DarkGray,
        [0x80, 0x80, 0x80],
    ),
    (
        "Gray 25%",
        wp_model::Highlight::LightGray,
        [0xC0, 0xC0, 0xC0],
    ),
    ("Black", wp_model::Highlight::Black, [0x00, 0x00, 0x00]),
];

/// One control of the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Control {
    Undo,
    Redo,
    Style,
    Font,
    Size,
    Bold,
    Italic,
    Underline,
    Strike,
    Colour,
    Highlight,
    AlignLeft,
    AlignCentre,
    AlignRight,
    Justify,
    Bullets,
    Numbers,
    IndentOut,
    IndentIn,
    LineSpacing,
    Table,
    Picture,
    Find,
    Comment,
    Track,
    Navigate,
    Review,
    /// The overflow button itself, drawn only when something folded.
    More,
}

/// The row, left to right, in the groups a rule stands between.
pub(crate) const ROW: &[&[Control]] = &[
    &[Control::Undo, Control::Redo],
    &[Control::Style, Control::Font, Control::Size],
    &[
        Control::Bold,
        Control::Italic,
        Control::Underline,
        Control::Strike,
        Control::Colour,
        Control::Highlight,
    ],
    &[
        Control::AlignLeft,
        Control::AlignCentre,
        Control::AlignRight,
        Control::Justify,
    ],
    &[
        Control::Bullets,
        Control::Numbers,
        Control::IndentOut,
        Control::IndentIn,
        Control::LineSpacing,
    ],
    &[Control::Table, Control::Picture],
];

/// The group at the right edge: the modes and the panes. Never folded.
pub(crate) const RIGHT: &[Control] = &[
    Control::Find,
    Control::Comment,
    Control::Track,
    Control::Navigate,
    Control::Review,
];

/// The gap between controls, and the width a rule between groups takes.
const GAP: f32 = 4.0;
const RULE: f32 = 13.0;
/// The chevron half of a split button or a dropdown.
const CHEVRON: f32 = 14.0;

impl Control {
    /// Every control there is, for a walk.
    #[cfg(test)]
    pub(crate) fn all() -> impl Iterator<Item = Control> {
        ROW.iter()
            .flat_map(|group| group.iter().copied())
            .chain(RIGHT.iter().copied())
    }

    /// How wide the control is on the row.
    fn width(self) -> f32 {
        match self {
            Control::Style | Control::Font => 150.0,
            Control::Size => 56.0 + CHEVRON,
            Control::Colour | Control::Highlight | Control::Table | Control::LineSpacing => {
                theme::TARGET.x + CHEVRON
            }
            _ => theme::TARGET.x,
        }
    }

    /// The command a simple control runs, where it is one command.
    fn command(self) -> Option<Command> {
        Some(match self {
            Control::Undo => Command::Undo,
            Control::Redo => Command::Redo,
            Control::Bold => Command::Bold,
            Control::Italic => Command::Italic,
            Control::Underline => Command::Underline,
            Control::Strike => Command::Strike,
            Control::AlignLeft => Command::Align(Justify::Start),
            Control::AlignCentre => Command::Align(Justify::Center),
            Control::AlignRight => Command::Align(Justify::End),
            Control::Justify => Command::Align(Justify::Both),
            Control::Bullets => Command::Bullets,
            Control::Numbers => Command::Numbers,
            Control::IndentOut => Command::Indent(-1),
            Control::IndentIn => Command::Indent(1),
            Control::Picture => Command::InsertPicture,
            Control::Find => Command::Find,
            Control::Comment => Command::AddComment,
            Control::Track => Command::TrackChanges,
            Control::Navigate => Command::Navigator,
            Control::Review => Command::Reviewer,
            Control::Table => Command::InsertTable,
            Control::Style
            | Control::Font
            | Control::Size
            | Control::Colour
            | Control::Highlight
            | Control::LineSpacing
            | Control::More => return None,
        })
    }

    /// What the control is called: in its tooltip, and in the overflow.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Control::Style => "Style",
            Control::Font => "Font",
            Control::Size => "Size",
            Control::Colour => "Text Colour",
            Control::Highlight => "Highlight",
            Control::LineSpacing => "Line Spacing",
            Control::Table => "Table",
            Control::Track => "Track Changes",
            Control::Navigate => "Navigate",
            Control::Review => "Review",
            Control::Comment => "New Comment",
            Control::More => "More",
            other => commands::name(&other.command().expect("a simple control has a command")),
        }
    }

    fn icon(self) -> Option<Icon> {
        Some(match self {
            Control::Undo => Icon::Undo,
            Control::Redo => Icon::Redo,
            Control::AlignLeft => Icon::AlignLeft,
            Control::AlignCentre => Icon::AlignCenter,
            Control::AlignRight => Icon::AlignRight,
            Control::Justify => Icon::Justify,
            Control::Bullets => Icon::Bullets,
            Control::Numbers => Icon::Numbering,
            Control::IndentOut => Icon::IndentOut,
            Control::IndentIn => Icon::IndentIn,
            Control::LineSpacing => Icon::LineSpacing,
            Control::Colour => Icon::TextColour,
            Control::Highlight => Icon::Highlight,
            Control::Table => Icon::Table,
            Control::Picture => Icon::Picture,
            Control::Find => Icon::Find,
            Control::Comment => Icon::Comment,
            Control::Track => Icon::TrackChanges,
            Control::Navigate => Icon::Navigate,
            Control::Review => Icon::Review,
            _ => return None,
        })
    }
}

/// Which controls stand on the row at this width, group by group, and
/// which fold into the overflow — from the right, whole controls, in order.
///
/// Decided from the controls' stated widths rather than measured as they are
/// drawn, so that the answer is the same on the frame the window opens and
/// can be asked by a test without a window at all.
pub(crate) fn plan(available: f32) -> (Vec<Vec<Control>>, Vec<Control>) {
    let right: f32 = RIGHT.iter().map(|c| c.width() + GAP).sum::<f32>() + RULE;
    let overflow = theme::TARGET.x + GAP;
    let budget = available - right - overflow - 2.0 * GAP;
    let mut used = 0.0;
    let mut shown: Vec<Vec<Control>> = Vec::new();
    let mut folded = Vec::new();
    let mut fits = true;
    for (index, group) in ROW.iter().enumerate() {
        let mut row_group = Vec::new();
        for control in group.iter().copied() {
            let need = control.width()
                + GAP
                + if row_group.is_empty() && index > 0 {
                    RULE
                } else {
                    0.0
                };
            if fits && used + need <= budget {
                used += need;
                row_group.push(control);
            } else {
                fits = false;
                folded.push(control);
            }
        }
        if !row_group.is_empty() {
            shown.push(row_group);
        }
    }
    (shown, folded)
}

/// One control as it was drawn this frame: its name, its tooltip, and where.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Drawn {
    pub control: Control,
    pub tip: String,
    pub rect: egui::Rect,
}

fn drawn_id() -> egui::Id {
    egui::Id::new("scriva-toolbar-drawn")
}

/// The controls the last frame drew on the row, in order — what a test walks
/// instead of a screenshot.
#[cfg(test)]
pub(crate) fn drawn(ctx: &egui::Context) -> Vec<Drawn> {
    ctx.data(|d| d.get_temp::<Vec<Drawn>>(drawn_id()))
        .unwrap_or_default()
}

fn note(ui: &egui::Ui, control: Control, tip: &str, rect: egui::Rect) {
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<Drawn>>(drawn_id())
            .push(Drawn {
                control,
                tip: tip.to_owned(),
                rect,
            })
    });
}

/// Everything the row reads from the document before it draws, gathered
/// once: a closure drawing a popup cannot borrow the application while the
/// application is drawing it.
struct State {
    undo: bool,
    redo: bool,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    alignment: Option<Justify>,
    bullets: bool,
    numbers: bool,
    spacing: Option<Line240>,
    styles: Vec<(wp_model::StyleId, String)>,
    style: Option<wp_model::StyleId>,
    face: Option<String>,
    document_faces: Vec<String>,
    size: Option<HalfPoint>,
    tracking: bool,
    navigator: bool,
    reviewer: bool,
    colour: wp_model::Color,
    highlight: wp_model::Highlight,
}

impl Scriva {
    fn toolbar_state(&self) -> State {
        let (undo, redo) = self.can_undo_redo();
        let (bold, italic, underline) = self.emphasis();
        let (bullets, numbers) = self.list_state();
        let (tracking, reviewer) = self.reviewing();
        State {
            undo,
            redo,
            bold,
            italic,
            underline,
            strike: self.struck(),
            alignment: self.alignment(),
            bullets,
            numbers,
            spacing: self.line_spacing_at(),
            styles: self.quick_styles(),
            style: self.style_at(),
            face: self.face_at(),
            document_faces: crate::app::font_names(self.document_ref()),
            size: self.size_at(),
            tracking,
            navigator: self.showing_navigator(),
            reviewer,
            colour: self.last_colour,
            highlight: self.last_highlight,
        }
    }

    /// The row of controls under the menu bar.
    pub(crate) fn toolbar_row(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        ui.ctx()
            .data_mut(|d| d.insert_temp::<Vec<Drawn>>(drawn_id(), Vec::new()));
        let state = self.toolbar_state();
        let (shown, folded) = plan(ui.available_width());
        let mut chosen = None;
        let mut size_text = self.size_text.take();
        let mut field_held = false;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            ui.set_min_height(theme::TARGET.y);
            for (index, group) in shown.iter().enumerate() {
                if index > 0 {
                    rule(ui);
                }
                for control in group.iter().copied() {
                    let (command, held) = draw(ui, control, &state, &mut size_text);
                    chosen = chosen.take().or(command);
                    field_held |= held;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                if !folded.is_empty() {
                    let (rect, response) =
                        ui.allocate_exact_size(theme::TARGET, egui::Sense::click());
                    icons::paint_state(ui, rect, &response, false);
                    icons::draw(ui.painter(), Icon::Overflow, rect.center(), theme::INK);
                    note(ui, Control::More, "More", rect);
                    let response = response.on_hover_text("More");
                    if let Some(Some(command)) =
                        menu::under(&response, |ui| overflow_menu(ui, &folded, &state))
                    {
                        chosen = chosen.take().or(Some(command));
                    }
                }
                for control in RIGHT.iter().rev().copied() {
                    let (command, held) = draw(ui, control, &state, &mut size_text);
                    chosen = chosen.take().or(command);
                    field_held |= held;
                }
                rule(ui);
            });
        });
        self.size_text = size_text;
        self.field_held = field_held;
        if let Some(Command::Color(colour)) = &chosen {
            self.last_colour = *colour;
        }
        if let Some(Command::Highlight(highlight)) = &chosen {
            self.last_highlight = *highlight;
        }
        chosen
    }
}

/// A vertical rule between two groups.
fn rule(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(RULE - GAP, theme::TARGET.y),
        egui::Sense::hover(),
    );
    let x = rect.center().x.round() + 0.5;
    ui.painter().vline(
        x,
        (rect.top() + 4.0)..=(rect.bottom() - 4.0),
        egui::Stroke::new(1.0, theme::CHROME_RULE),
    );
}

/// The rows of the overflow menu: every folded control, with its key.
fn overflow_menu(ui: &mut egui::Ui, folded: &[Control], state: &State) -> Option<Command> {
    let mut chosen = None;
    for control in folded.iter().copied() {
        match control {
            Control::Style => {
                menu::sub(ui, "Style", |ui| {
                    chosen = chosen.take().or(style_rows(ui, state));
                });
            }
            Control::Font => {
                menu::sub(ui, "Font", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(340.0)
                        .show(ui, |ui| {
                            chosen = chosen.take().or(font_rows(ui, state, ""));
                        });
                });
            }
            Control::Size => {
                menu::sub(ui, "Size", |ui| {
                    chosen = chosen.take().or(size_rows(ui, state));
                });
            }
            Control::Colour => {
                menu::sub(ui, "Text Colour", |ui| {
                    chosen = chosen.take().or(colour_rows(ui, state));
                });
            }
            Control::Highlight => {
                menu::sub(ui, "Highlight", |ui| {
                    chosen = chosen.take().or(highlight_rows(ui, state));
                });
            }
            Control::LineSpacing => {
                menu::sub(ui, "Line Spacing", |ui| {
                    chosen = chosen.take().or(spacing_rows(ui, state));
                });
            }
            Control::Table => {
                menu::sub(ui, "Table", |ui| {
                    chosen = chosen.take().or(table_rows(ui));
                });
            }
            other => {
                let command = other.command().expect("a simple control");
                if menu::item(ui, other.name(), commands::shortcut(&command)).clicked() {
                    chosen = Some(command);
                }
            }
        }
    }
    chosen
}

/// Draws one control, and says what it chose and whether a field of its
/// holds the keyboard.
fn draw(
    ui: &mut egui::Ui,
    control: Control,
    state: &State,
    size_text: &mut Option<String>,
) -> (Option<Command>, bool) {
    let mut held = false;
    let command = match control {
        Control::Bold | Control::Italic | Control::Underline | Control::Strike => {
            let (letter, on) = match control {
                Control::Bold => ("B", state.bold),
                Control::Italic => ("I", state.italic),
                Control::Underline => ("U", state.underline),
                _ => ("S", state.strike),
            };
            let command = control.command().expect("a letter has a command");
            let tip = tooltip(&command);
            let response = icons::emphasis(ui, letter, on, &tip);
            note(ui, control, &tip, response.rect);
            response.clicked().then_some(command)
        }
        Control::Style => combo(ui, control, 150.0, state.style_name(), "Style", |ui| {
            style_rows(ui, state)
        }),
        Control::Font => {
            let filter_id = egui::Id::new("scriva-font-filter");
            let filter: String = ui
                .ctx()
                .data(|d| d.get_temp::<String>(filter_id))
                .unwrap_or_default();
            let mut filter_now = filter.clone();
            let chosen = combo(
                ui,
                control,
                150.0,
                state.face.clone().unwrap_or_default(),
                "Font",
                |ui| {
                    // Typed-to-filter: the field takes the keyboard when the
                    // list opens, and the rows under it are the ones whose
                    // names contain what was typed.
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut filter_now)
                            .hint_text("Type to filter")
                            .desired_width(f32::INFINITY),
                    );
                    if ui.ctx().memory(|m| m.focused()).is_none() {
                        field.request_focus();
                    }
                    held = field.has_focus();
                    menu::sep(ui);
                    let mut chosen = None;
                    egui::ScrollArea::vertical()
                        .max_height(340.0)
                        .show(ui, |ui| {
                            chosen = font_rows(ui, state, &filter_now);
                        });
                    chosen
                },
            );
            // Asked before the lock is taken: a question to the context from
            // inside its own data lock is a deadlock, not an answer.
            let list_open = egui::Popup::is_any_open(ui.ctx());
            ui.ctx().data_mut(|d| {
                if chosen.is_some() || !list_open {
                    d.remove_temp::<String>(filter_id);
                } else {
                    d.insert_temp(filter_id, filter_now);
                }
            });
            chosen
        }
        Control::Size => {
            let (command, holds) = size_field(ui, state, size_text);
            held = holds;
            command
        }
        Control::Colour => {
            let rgb = match state.colour {
                wp_model::Color::Rgb(rgb) => Some(rgb),
                _ => None,
            };
            split(
                ui,
                control,
                Icon::TextColour,
                rgb,
                Command::Color(state.colour),
                |ui| colour_rows(ui, state),
            )
        }
        Control::Highlight => {
            let rgb = HIGHLIGHTS
                .iter()
                .find(|(_, value, _)| *value == state.highlight)
                .map(|(_, _, rgb)| *rgb);
            split(
                ui,
                control,
                Icon::Highlight,
                rgb,
                Command::Highlight(state.highlight),
                |ui| highlight_rows(ui, state),
            )
        }
        Control::LineSpacing => dropdown(ui, control, Icon::LineSpacing, "Line Spacing", |ui| {
            spacing_rows(ui, state)
        }),
        Control::Table => dropdown(ui, control, Icon::Table, "Table", table_rows),
        other => {
            let command = other.command().expect("a simple control has a command");
            let icon = other.icon().expect("and an icon");
            let on = match other {
                Control::AlignLeft => state.alignment == Some(Justify::Start),
                Control::AlignCentre => state.alignment == Some(Justify::Center),
                Control::AlignRight => state.alignment == Some(Justify::End),
                Control::Justify => state.alignment == Some(Justify::Both),
                Control::Bullets => state.bullets,
                Control::Numbers => state.numbers,
                Control::Track => state.tracking,
                Control::Navigate => state.navigator,
                Control::Review => state.reviewer,
                _ => false,
            };
            let enabled = match other {
                Control::Undo => state.undo,
                Control::Redo => state.redo,
                _ => true,
            };
            let tip = tooltip_named(other.name(), &command);
            let response = ui
                .add_enabled_ui(enabled, |ui| icons::button(ui, icon, on, &tip))
                .inner;
            note(ui, control, &tip, response.rect);
            response.clicked().then_some(command)
        }
    };
    (command, held)
}

impl State {
    fn style_name(&self) -> String {
        self.style
            .and_then(|id| self.styles.iter().find(|(other, _)| *other == id))
            .map(|(_, name)| name.clone())
            .unwrap_or_default()
    }
}

/// A combo in the field style: the current value, a chevron, and a menu of
/// the values under it.
fn combo(
    ui: &mut egui::Ui,
    control: Control,
    width: f32,
    value: String,
    tip: &str,
    rows: impl FnOnce(&mut egui::Ui) -> Option<Command>,
) -> Option<Command> {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, theme::TARGET.y), egui::Sense::click());
    let edge = if response.hovered()
        || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response))
    {
        theme::FIELD_EDGE_HOT
    } else {
        theme::FIELD_EDGE
    };
    ui.painter().rect(
        rect,
        theme::RADIUS_CONTROL as f32,
        theme::FIELD,
        egui::Stroke::new(1.0, edge),
        egui::StrokeKind::Inside,
    );
    let text = egui::Rect::from_min_max(
        rect.min + egui::vec2(7.0, 0.0),
        egui::pos2(rect.right() - CHEVRON, rect.bottom()),
    );
    let galley =
        ui.painter()
            .layout_no_wrap(value, egui::FontId::proportional(theme::TEXT), theme::INK);
    ui.painter().with_clip_rect(text).galley(
        egui::pos2(text.left(), text.center().y - galley.size().y / 2.0),
        galley,
        theme::INK,
    );
    icons::draw(
        ui.painter(),
        Icon::ChevronDown,
        egui::pos2(rect.right() - CHEVRON / 2.0 - 2.0, rect.center().y),
        theme::INK_SOFT,
    );
    note(ui, control, tip, rect);
    let response = response.on_hover_text(tip);
    menu::under(&response, rows).flatten()
}

/// A split button: the glyph applies the last colour, the chevron opens the
/// swatches, and a bar under the glyph shows which colour that is.
fn split(
    ui: &mut egui::Ui,
    control: Control,
    icon: Icon,
    bar: Option<[u8; 3]>,
    apply: Command,
    rows: impl FnOnce(&mut egui::Ui) -> Option<Command>,
) -> Option<Command> {
    let tip = control.name();
    let (rect, response) = ui.allocate_exact_size(theme::TARGET, egui::Sense::click());
    icons::paint_state(ui, rect, &response, false);
    icons::draw(
        ui.painter(),
        icon,
        rect.center() + egui::vec2(0.0, -2.0),
        theme::INK,
    );
    let swatch = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 6.0, rect.bottom() - 5.0),
        egui::pos2(rect.right() - 6.0, rect.bottom() - 2.0),
    );
    let fill = match bar {
        Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
        None => theme::INK,
    };
    ui.painter().rect_filled(swatch, 1.0, fill);
    let (arrow_rect, arrow) =
        ui.allocate_exact_size(egui::vec2(CHEVRON, theme::TARGET.y), egui::Sense::click());
    icons::paint_state(ui, arrow_rect, &arrow, false);
    icons::draw(
        ui.painter(),
        Icon::ChevronDown,
        arrow_rect.center(),
        theme::INK_SOFT,
    );
    note(ui, control, tip, rect.union(arrow_rect));
    let pressed = response.on_hover_text(tip).clicked();
    let picked = menu::under(&arrow.on_hover_text(format!("{tip} — choose")), rows).flatten();
    picked.or(pressed.then_some(apply))
}

/// An icon with a chevron beside it, and a menu under both.
fn dropdown(
    ui: &mut egui::Ui,
    control: Control,
    icon: Icon,
    tip: &str,
    rows: impl FnOnce(&mut egui::Ui) -> Option<Command>,
) -> Option<Command> {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(theme::TARGET.x + CHEVRON, theme::TARGET.y),
        egui::Sense::click(),
    );
    icons::paint_state(ui, rect, &response, false);
    icons::draw(
        ui.painter(),
        icon,
        egui::pos2(rect.left() + theme::TARGET.x / 2.0, rect.center().y),
        theme::INK,
    );
    icons::draw(
        ui.painter(),
        Icon::ChevronDown,
        egui::pos2(rect.right() - CHEVRON / 2.0, rect.center().y),
        theme::INK_SOFT,
    );
    note(ui, control, tip, rect);
    menu::under(&response.on_hover_text(tip), rows).flatten()
}

/// The size box: a field the size can be typed into, Enter applies, and a
/// chevron with Word's list under it.
fn size_field(
    ui: &mut egui::Ui,
    state: &State,
    size_text: &mut Option<String>,
) -> (Option<Command>, bool) {
    let id = egui::Id::new("scriva-size-field");
    let shown = state
        .size
        .map(|HalfPoint(half)| size_label(half))
        .unwrap_or_default();
    let mut text = size_text.clone().unwrap_or_else(|| shown.clone());
    let field = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(56.0 - 14.0)
            .font(egui::FontId::proportional(theme::TEXT))
            .horizontal_align(egui::Align::Center),
    );
    let mut chosen = None;
    let held = field.has_focus();
    if held {
        *size_text = Some(text.clone());
    } else {
        *size_text = None;
    }
    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some(half) = parse_size(&text) {
            chosen = Some(Command::Size(HalfPoint(half)));
        }
        *size_text = None;
    }
    let (arrow_rect, arrow) =
        ui.allocate_exact_size(egui::vec2(CHEVRON, theme::TARGET.y), egui::Sense::click());
    icons::paint_state(ui, arrow_rect, &arrow, false);
    icons::draw(
        ui.painter(),
        Icon::ChevronDown,
        arrow_rect.center(),
        theme::INK_SOFT,
    );
    note(ui, Control::Size, "Size", field.rect.union(arrow_rect));
    let picked = menu::under(&arrow.on_hover_text("Size"), |ui| size_rows(ui, state)).flatten();
    (picked.or(chosen), held)
}

/// A typed size, in half-points: `11`, `10.5`, `12pt` — anything from one
/// point to a thousand.
pub(crate) fn parse_size(text: &str) -> Option<i32> {
    let number: f64 = text.trim().trim_end_matches("pt").trim().parse().ok()?;
    if !(1.0..=1000.0).contains(&number) {
        return None;
    }
    Some((number * 2.0).round() as i32)
}

fn size_label(half: i32) -> String {
    if half % 2 == 0 {
        format!("{}", half / 2)
    } else {
        format!("{}.5", half / 2)
    }
}

fn style_rows(ui: &mut egui::Ui, state: &State) -> Option<Command> {
    let mut chosen = None;
    for (id, name) in &state.styles {
        if menu::check(ui, name, "", state.style == Some(*id)).clicked() {
            chosen = Some(Command::Style(*id));
        }
    }
    if state.styles.is_empty() {
        ui.add_enabled(false, egui::Button::new("No styles in this document"));
    }
    chosen
}

/// The document's own faces first, a rule, then every family the machine
/// has — one row each, sorted, filtered by what was typed. No per-row face
/// previews: previewing means registering every family with egui's atlas,
/// and the atlas is rebuilt on every registration.
fn font_rows(ui: &mut egui::Ui, state: &State, filter: &str) -> Option<Command> {
    let mut chosen = None;
    let wanted = filter.trim().to_ascii_lowercase();
    let matches = |name: &str| wanted.is_empty() || name.to_ascii_lowercase().contains(&wanted);
    let mut any_document = false;
    for name in state.document_faces.iter().filter(|name| matches(name)) {
        any_document = true;
        if menu::check(ui, name, "", state.face.as_deref() == Some(name)).clicked() {
            chosen = Some(Command::Font(name.clone()));
        }
    }
    let families = ui_kit::catalogue::families();
    let rest: Vec<&String> = families
        .iter()
        .filter(|name| matches(name) && !state.document_faces.contains(name))
        .collect();
    if any_document && !rest.is_empty() {
        menu::sep(ui);
    }
    for name in rest {
        if menu::check(ui, name, "", state.face.as_deref() == Some(name)).clicked() {
            chosen = Some(Command::Font(name.clone()));
        }
    }
    chosen
}

fn size_rows(ui: &mut egui::Ui, state: &State) -> Option<Command> {
    let mut chosen = None;
    for half in SIZES {
        let on = state.size == Some(HalfPoint(half));
        if menu::check(ui, &size_label(half), "", on).clicked() {
            chosen = Some(Command::Size(HalfPoint(half)));
        }
    }
    chosen
}

fn colour_rows(ui: &mut egui::Ui, state: &State) -> Option<Command> {
    let colours: Vec<(&str, egui::Color32)> = PALETTE
        .iter()
        .map(|(name, [r, g, b])| (*name, egui::Color32::from_rgb(*r, *g, *b)))
        .collect();
    let current = match state.colour {
        wp_model::Color::Rgb(rgb) => PALETTE.iter().position(|(_, other)| *other == rgb),
        _ => None,
    };
    match menu::swatches(ui, "&Automatic", &colours, current, Some("&More Colours…"))? {
        menu::Swatch::First => Some(Command::Color(wp_model::Color::Auto)),
        menu::Swatch::Index(index) => Some(Command::Color(wp_model::Color::Rgb(PALETTE[index].1))),
        menu::Swatch::More => Some(Command::CustomColor),
    }
}

fn highlight_rows(ui: &mut egui::Ui, state: &State) -> Option<Command> {
    let colours: Vec<(&str, egui::Color32)> = HIGHLIGHTS
        .iter()
        .map(|(name, _, [r, g, b])| (*name, egui::Color32::from_rgb(*r, *g, *b)))
        .collect();
    let current = HIGHLIGHTS
        .iter()
        .position(|(_, value, _)| *value == state.highlight);
    match menu::swatches(ui, "&None", &colours, current, None)? {
        menu::Swatch::First => Some(Command::Highlight(wp_model::Highlight::None)),
        menu::Swatch::Index(index) => Some(Command::Highlight(HIGHLIGHTS[index].1)),
        menu::Swatch::More => None,
    }
}

fn spacing_rows(ui: &mut egui::Ui, state: &State) -> Option<Command> {
    let mut chosen = None;
    for (label, value) in [
        ("&Single", Line240::SINGLE),
        ("&1.5 Lines", Line240::ONE_AND_A_HALF),
        ("&Double", Line240::DOUBLE),
    ] {
        let command = Command::LineSpacing(value);
        if menu::check(
            ui,
            label,
            commands::shortcut(&command),
            state.spacing == Some(value),
        )
        .clicked()
        {
            chosen = Some(command);
        }
    }
    chosen
}

/// The grid picker: eight by eight squares, the ones under and to the left
/// of the pointer lit, and the size they make in words above them. A click
/// inserts that table; the row under the grid opens the dialog for numbers
/// the grid does not reach.
fn table_rows(ui: &mut egui::Ui) -> Option<Command> {
    const SIDE: f32 = 18.0;
    const CELLS: usize = 8;
    let mut chosen = None;
    let hovered_id = ui.id().with("table-grid-hover");
    let hovered: Option<(usize, usize)> = ui.ctx().data(|d| d.get_temp(hovered_id));
    ui.label(
        egui::RichText::new(match hovered {
            Some((rows, columns)) => format!("{rows} × {columns} table"),
            None => "Insert table".to_owned(),
        })
        .color(theme::INK_SOFT),
    );
    let (grid, response) = ui.allocate_exact_size(
        egui::vec2(CELLS as f32 * SIDE + 8.0, CELLS as f32 * SIDE + 8.0),
        egui::Sense::click(),
    );
    let at = response.hover_pos().map(|pointer| {
        let column =
            (((pointer.x - grid.left() - 4.0) / SIDE).floor() as usize).clamp(0, CELLS - 1);
        let row = (((pointer.y - grid.top() - 4.0) / SIDE).floor() as usize).clamp(0, CELLS - 1);
        (row + 1, column + 1)
    });
    ui.ctx().data_mut(|d| match at {
        Some(at) => {
            d.insert_temp(hovered_id, at);
        }
        None => {
            d.remove_temp::<(usize, usize)>(hovered_id);
        }
    });
    for row in 0..CELLS {
        for column in 0..CELLS {
            let cell = egui::Rect::from_min_size(
                grid.min + egui::vec2(4.0 + column as f32 * SIDE, 4.0 + row as f32 * SIDE),
                egui::vec2(SIDE - 2.0, SIDE - 2.0),
            );
            let lit = at.is_some_and(|(rows, columns)| row < rows && column < columns);
            ui.painter().rect(
                cell,
                1.0,
                if lit { theme::TINT_ON } else { theme::FIELD },
                egui::Stroke::new(
                    1.0,
                    if lit {
                        theme::ACCENT
                    } else {
                        theme::FIELD_EDGE
                    },
                ),
                egui::StrokeKind::Inside,
            );
        }
    }
    if response.clicked() {
        if let Some((rows, columns)) = at {
            chosen = Some(Command::InsertTableOf(rows, columns));
            ui.close();
        }
    }
    menu::sep(ui);
    if menu::item(ui, "&Insert Table…", "").clicked() {
        chosen = Some(Command::InsertTable);
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_window_shows_the_whole_row_and_a_narrow_one_folds_from_the_right() {
        let (shown, folded) = plan(1600.0);
        assert!(folded.is_empty(), "nothing folds at 1600: {folded:?}");
        assert_eq!(shown.len(), ROW.len());

        let (shown, folded) = plan(800.0);
        assert!(!folded.is_empty(), "something folds at 800");
        // Order kept: what folds is the tail of the row.
        let all: Vec<Control> = ROW.iter().flat_map(|g| g.iter().copied()).collect();
        let kept: Vec<Control> = shown.into_iter().flatten().collect();
        assert_eq!(all[..kept.len()], kept[..]);
        assert_eq!(all[kept.len()..], folded[..]);
    }

    #[test]
    fn a_typed_size_is_read_in_half_points() {
        assert_eq!(parse_size("11"), Some(22));
        assert_eq!(parse_size("10.5"), Some(21));
        assert_eq!(parse_size(" 12pt "), Some(24));
        assert_eq!(parse_size("0"), None);
        assert_eq!(parse_size("big"), None);
    }
}
