//! A menu bar, and menus that look like menus.
//!
//! egui supplies the mechanism — a bar, buttons, popups, submenus — and leaves
//! the grammar to the caller: what a command row is made of, where its shortcut
//! sits, how a ticked item is marked, what separates one group of commands from
//! the next. Left to each call site that grammar drifts within a single menu,
//! never mind between two of them, and a menu whose rows do not agree with each
//! other is the thing that makes an application look homemade.
//!
//! So it lives here, and every menu in both applications is the same menu.

use eframe::egui;

/// The narrowest a menu may be.
///
/// A menu is a column of commands, and a column two words wide is a tooltip.
/// The width also has to leave room for the longest shortcut without the
/// shortcuts of shorter rows sliding left, which is what `shortcut_text` and a
/// single width together guarantee.
const MIN_WIDTH: f32 = 208.0;

/// The tick, drawn in every row whether or not the row is ever ticked.
///
/// Windows reserves the gutter always. A menu whose labels shift sideways the
/// moment one of them is checked is a menu that moves under the pointer, and
/// the fix is to spend the space unconditionally: the mark is *there*, in the
/// colour of the background, in every row that has no mark to make.
const TICK: &str = "✔";

/// Excel's green, which is what a ticked item is marked in.
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x21, 0x73, 0x46);

/// The line between groups of commands.
const RULE: egui::Color32 = egui::Color32::from_rgb(0xDF, 0xDF, 0xDF);

/// Which menu is open, and which one the pointer has moved onto.
///
/// A switch is carried a frame rather than acted on where it is noticed,
/// because the menu being left may already have been drawn by the time the
/// pointer is found over the next title, and closing it then would leave one
/// frame showing two open menus. See [`bar`].
type Switch = (egui::Id, egui::Id);

fn switch_id() -> egui::Id {
    egui::Id::new("ui-kit-menu-switch")
}

fn open_id() -> egui::Id {
    egui::Id::new("ui-kit-menu-open")
}

/// The menu bar across the top of the window.
///
/// Add one [`top`] per menu inside `add`.
pub fn bar<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    // The switch the previous frame asked for, performed before any menu is
    // drawn: sliding along an open menu bar changes menus, which is behaviour
    // every menu bar has had since there were menu bars, and egui's own bar
    // opens only on a click.
    let switch: Option<Switch> = ctx.data_mut(|d| {
        let asked = d.get_temp::<Switch>(switch_id());
        d.remove::<Switch>(switch_id());
        asked
    });
    if let Some((from, to)) = switch {
        egui::Popup::close_id(&ctx, from);
        egui::Popup::open_id(&ctx, to);
        ctx.data_mut(|d| d.insert_temp(open_id(), to));
    }
    egui::MenuBar::new()
        .style(bar_style as fn(&mut egui::Style))
        .ui(ui, add)
        .inner
}

/// One title on the menu bar, and the menu that drops from it.
pub fn top<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let response = ui.add(egui::Button::new(label));
    let ctx = ui.ctx().clone();
    let id = egui::Popup::default_response_id(&response);

    if egui::Popup::is_id_open(&ctx, id) {
        ctx.data_mut(|d| d.insert_temp(open_id(), id));
    } else if response.hovered() {
        // Another title has the floor and the pointer has arrived here: take
        // it, next frame.
        let held: Option<egui::Id> = ctx.data(|d| d.get_temp(open_id()));
        if let Some(open) = held.filter(|open| *open != id && egui::Popup::is_id_open(&ctx, *open))
        {
            ctx.data_mut(|d| d.insert_temp(switch_id(), (open, id)));
            ctx.request_repaint();
        }
    }

    under(&response, add)
}

/// A menu hung under a control that is not on the menu bar: a toolbar
/// dropdown, or the chevron half of a split button.
///
/// Same rows, same card, same rule — because a dropdown that looks unlike the
/// menus is a second menu system, and two menu systems in one window is the
/// thing that reads as unfinished.
pub fn under<R>(response: &egui::Response, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let config = egui::containers::menu::MenuConfig::new()
        .style(menu_style as fn(&mut egui::Style))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    egui::Popup::menu(response)
        .style(menu_style as fn(&mut egui::Style))
        .info(
            egui::UiStackInfo::new(egui::UiKind::Menu)
                .with_tag_value(egui::containers::menu::MenuConfig::MENU_CONFIG_TAG, config),
        )
        .show(|ui| {
            ui.set_min_width(MIN_WIDTH);
            add(ui)
        })
        .map(|inner| inner.inner)
}

/// A command: a label, and the keystroke that does the same thing.
///
/// Pass an empty `shortcut` for a command that has no key. Clicking closes the
/// menu, because a menu that stays open after its command has run is a menu
/// covering the thing the command just changed.
pub fn item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> egui::Response {
    entry(ui, label, shortcut, None)
}

/// A command that is either on or off, marked with a tick when it is on.
pub fn check(ui: &mut egui::Ui, label: &str, shortcut: &str, on: bool) -> egui::Response {
    entry(ui, label, shortcut, Some(on))
}

fn entry(ui: &mut egui::Ui, label: &str, shortcut: &str, checked: Option<bool>) -> egui::Response {
    let mark = egui::RichText::new(TICK).size(12.0).color(match checked {
        Some(true) => ACCENT,
        // The gutter is still spent, so that every label in the menu starts at
        // the same x whatever any of them is doing.
        _ => egui::Color32::TRANSPARENT,
    });
    let mut button = egui::Button::new((mark, label)).gap(8.0);
    if !shortcut.is_empty() {
        button = button.shortcut_text(shortcut);
    }
    let response = ui.add(button);
    if response.clicked() {
        ui.close();
    }
    response
}

/// A submenu, opened by resting on its row.
pub fn sub<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let mark = egui::RichText::new(TICK)
        .size(12.0)
        .color(egui::Color32::TRANSPARENT);
    let button = egui::Button::new((mark, label))
        .gap(8.0)
        .right_text(egui::containers::menu::SubMenuButton::RIGHT_ARROW);
    egui::containers::menu::SubMenuButton::from_button(button)
        .ui(ui, |ui| {
            ui.set_min_width(MIN_WIDTH);
            add(ui)
        })
        .1
        .map(|inner| inner.inner)
}

/// The rule between one group of commands and the next.
///
/// Inset from both edges rather than run wall to wall: a full-width rule cuts
/// the menu into two menus, and these are groups within one.
pub fn sep(ui: &mut egui::Ui) {
    // Nothing during the pass that measures the menu. A popup sizes itself from
    // what its contents ask for, and a rule that asks for "however much room is
    // left" answers that question with the width of the screen — which is what
    // the menu then becomes.
    let width = if ui.is_sizing_pass() {
        0.0
    } else {
        ui.available_width()
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 9.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let y = rect.center().y.round() + 0.5;
        ui.painter().hline(
            (rect.left() + 9.0)..=(rect.right() - 9.0),
            y,
            egui::Stroke::new(1.0, RULE),
        );
    }
}

/// The titles on the bar: flat until the pointer is on them, and roomy enough
/// to be a target rather than a word that happens to be clickable.
fn bar_style(style: &mut egui::Style) {
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.item_spacing.x = 1.0;
    let v = &mut style.visuals;
    v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    // A title with a menu down is *held*, and looks held for as long as it is.
    v.widgets.open.weak_bg_fill = egui::Color32::from_rgb(0xDC, 0xE8, 0xE1);
    v.widgets.open.bg_stroke = egui::Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(0xE6, 0xEE, 0xEA);
    v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    v.widgets.active.weak_bg_fill = egui::Color32::from_rgb(0xDC, 0xE8, 0xE1);
    v.widgets.active.bg_stroke = egui::Stroke::NONE;
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = egui::CornerRadius::same(4);
    }
}

/// Inside a menu: rows the full width of the box, no outlines on any of them,
/// and a card that reads as floating above the window rather than cut into it.
fn menu_style(style: &mut egui::Style) {
    style.spacing.button_padding = egui::vec2(10.0, 4.0);
    style.spacing.item_spacing = egui::vec2(6.0, 1.0);
    style.spacing.menu_margin = egui::Margin::symmetric(5, 6);
    style.spacing.interact_size.y = 22.0;

    let v = &mut style.visuals;
    v.menu_corner_radius = egui::CornerRadius::same(7);
    v.window_fill = egui::Color32::from_gray(0xFD);
    v.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0xC6));
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: egui::Color32::from_black_alpha(38),
    };
    // A hovered row in a menu is a *band* across the menu, not a button with a
    // border: the border is what makes an egui menu look like a list of tiny
    // buttons somebody stacked up.
    v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(0xE6, 0xEE, 0xEA);
    v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    v.widgets.active.weak_bg_fill = egui::Color32::from_rgb(0xD6, 0xE5, 0xDC);
    v.widgets.active.bg_stroke = egui::Stroke::NONE;
    v.widgets.open.weak_bg_fill = egui::Color32::from_rgb(0xE6, 0xEE, 0xEA);
    v.widgets.open.bg_stroke = egui::Stroke::NONE;
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = egui::CornerRadius::same(4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout a real menu lays its rows out in: every row the full width of
    /// the box, so that `shortcut_text` has somewhere to push the shortcut to.
    fn in_a_menu(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
        menu_style(ui.style_mut());
        ui.allocate_ui_with_layout(
            egui::vec2(MIN_WIDTH, 200.0),
            egui::Layout::top_down_justified(egui::Align::Min),
            add,
        );
    }

    /// Runs one frame of `add` and hands back every shape it painted.
    fn painted(add: impl FnOnce(&mut egui::Ui)) -> Vec<egui::epaint::Shape> {
        let ctx = egui::Context::default();
        crate::fonts::register(&ctx, &[]);
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            ..Default::default()
        };
        let mut add = Some(add);
        // Two passes: the first sizes the bar, the second paints it.
        let mut out = ctx.run_ui(input(), |ui| {
            bar(ui, |ui| {
                top(ui, "File", |_| {});
            });
            let _ = &mut add;
        });
        out.textures_delta.clear();
        let mut out = ctx.run_ui(input(), |ui| {
            if let Some(add) = add.take() {
                add(ui);
            }
        });
        out.textures_delta.clear();
        let mut flat = Vec::new();
        fn walk(shape: egui::epaint::Shape, into: &mut Vec<egui::epaint::Shape>) {
            match shape {
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, into);
                    }
                }
                other => into.push(other),
            }
        }
        for clipped in out.shapes {
            walk(clipped.shape, &mut flat);
        }
        flat
    }

    /// A menu is as wide as its widest command, and a rule is not a command.
    ///
    /// The bug this catches: a popup measures itself in a pass of its own, and
    /// during that pass "however much width is left" is the width of the
    /// screen. A rule that asked for it made every menu containing one three
    /// times wider than its longest label.
    #[test]
    fn a_rule_does_not_get_a_vote_on_how_wide_the_menu_is() {
        let ctx = egui::Context::default();
        crate::fonts::register(&ctx, &[]);
        let mut asked = f32::NAN;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                ui.scope_builder(egui::UiBuilder::new().sizing_pass(), |ui| {
                    let before = ui.min_rect().width();
                    sep(ui);
                    asked = ui.min_rect().width() - before;
                });
            },
        );
        out.textures_delta.clear();
        assert_eq!(asked, 0.0, "the rule claimed {asked} points of width");
    }

    /// The gutter is what keeps a menu's labels in a column. If a ticked row
    /// and a plain row lay their text out differently, the labels step sideways
    /// as things are switched on and off.
    #[test]
    fn a_ticked_row_and_a_plain_row_start_their_label_in_the_same_place() {
        fn label_x(checked: Option<bool>) -> f32 {
            let shapes = painted(|ui| {
                in_a_menu(ui, |ui| {
                    entry(ui, "Freeze Panes", "", checked);
                });
            });
            let text: Vec<_> = shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::epaint::Shape::Text(text) => Some(text),
                    _ => None,
                })
                .collect();
            let row = text
                .iter()
                .find(|text| text.galley.job.text.contains("Freeze"))
                .expect("the label was never painted");
            row.pos.x
        }
        assert_eq!(label_x(Some(true)), label_x(Some(false)));
        assert_eq!(label_x(Some(true)), label_x(None));
    }

    /// A shortcut is only useful if it is legible as a *shortcut*: right of the
    /// label, not run on after it.
    #[test]
    fn a_shortcut_sits_at_the_right_hand_edge_of_the_menu() {
        let shapes = painted(|ui| {
            in_a_menu(ui, |ui| {
                item(ui, "Save", "Ctrl+S");
            });
        });
        let mut label = None;
        let mut shortcut = None;
        for shape in &shapes {
            if let egui::epaint::Shape::Text(text) = shape {
                match text.galley.job.text.as_str() {
                    "Save" => label = Some(text.pos.x),
                    "Ctrl+S" => shortcut = Some(text.pos.x),
                    _ => {}
                }
            }
        }
        let label = label.expect("no label");
        let shortcut = shortcut.expect("no shortcut");
        assert!(
            shortcut > label + 100.0,
            "the shortcut ({shortcut}) crowds the label ({label})"
        );
    }
}
