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
use eframe::egui::AtomExt as _;

/// The gap between the tick gutter and the label.
///
/// Spent as a sized atom rather than as the button's atom spacing, because the
/// label is not one atom any more: it is split around the mnemonic so that one
/// character of it can be underlined, and a spacing that applied between *all*
/// the atoms would put a gap inside every word.
const GUTTER: f32 = 8.0;

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

fn marks_id() -> egui::Id {
    egui::Id::new("ui-kit-menu-marks")
}

/// Whether the mnemonics are showing.
///
/// Hidden until Alt is held down or a menu is open, which is the rule Windows
/// has used since the underlines stopped being permanent: a bar with a letter
/// underlined in every title all day is noise, and the underline is only
/// wanted at the moment somebody reaches for Alt.
fn showing_marks(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(marks_id())).unwrap_or(false)
}

/// A label with its mnemonic picked out, written the way Windows has written
/// it for thirty years: `"&File"`, `"Save &As…"`, `"E&xit"`.
struct Marked<'a> {
    before: &'a str,
    key: Option<char>,
    after: &'a str,
}

fn mark(label: &str) -> Marked<'_> {
    let Some(at) = label.find('&') else {
        return Marked {
            before: label,
            key: None,
            after: "",
        };
    };
    let rest = &label[at + 1..];
    match rest.chars().next() {
        Some(key) => Marked {
            before: &label[..at],
            key: Some(key),
            after: &rest[key.len_utf8()..],
        },
        // A trailing `&` marks nothing. Show the label as it was written rather
        // than swallowing a character that was probably a typo.
        None => Marked {
            before: label,
            key: None,
            after: "",
        },
    }
}

impl Marked<'_> {
    /// The key that reaches this command, if it has one.
    fn key(&self) -> Option<egui::Key> {
        egui::Key::from_name(&self.key?.to_ascii_uppercase().to_string())
    }

    /// Was this row's letter pressed, with `modifiers` held?
    ///
    /// Takes the character as well as the key. `consume_key` removes the key
    /// event and leaves the `Text` event beside it, and `Text` is what anything
    /// that accepts typing reads — so the letter that chose a menu command also
    /// arrived in the cell underneath the menu, which is exactly the bug this
    /// whole arrangement is supposed to prevent.
    fn taken(&self, ui: &egui::Ui, modifiers: egui::Modifiers) -> bool {
        let (Some(letter), Some(key)) = (self.key, self.key()) else {
            return false;
        };
        ui.input_mut(|i| {
            if !i.consume_key(modifiers, key) {
                return false;
            }
            i.events.retain(|event| match event {
                egui::Event::Text(text) => !text.eq_ignore_ascii_case(&letter.to_string()),
                _ => true,
            });
            true
        })
    }

    /// The label, as atoms.
    ///
    /// Three of them rather than one, so that the mnemonic can carry an
    /// underline while the rest of the label still takes its colour from the
    /// widget — which is what greys a disabled row out, and what a baked-in
    /// `LayoutJob` would have thrown away.
    fn atoms(&self, underline: bool, into: &mut egui::Atoms<'static>) {
        if !self.before.is_empty() {
            into.push_right(egui::RichText::new(self.before.to_string()));
        }
        if let Some(key) = self.key {
            let mut text = egui::RichText::new(key.to_string());
            if underline {
                text = text.underline();
            }
            into.push_right(text);
        }
        if !self.after.is_empty() {
            into.push_right(egui::RichText::new(self.after.to_string()));
        }
    }
}

/// One row of a menu, which can be reached by the pointer or by its letter.
///
/// A wrapper rather than a bare [`egui::Response`] because a key press is not
/// a click and egui has no way to say that it was — and every call site asks
/// the same question, "was this chosen", which should have one answer.
pub struct Item {
    /// The row itself: for a tooltip, or its rectangle.
    pub response: egui::Response,
    by_key: bool,
}

impl Item {
    /// Was this command chosen — by the pointer, or by its letter?
    pub fn clicked(&self) -> bool {
        self.by_key || self.response.clicked()
    }

    pub fn on_hover_text(self, text: impl Into<egui::WidgetText>) -> Self {
        Item {
            response: self.response.on_hover_text(text),
            by_key: self.by_key,
        }
    }
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

    let held = ui.input(|i| i.modifiers.alt);
    let down = ctx
        .data(|d| d.get_temp::<egui::Id>(open_id()))
        .is_some_and(|id| egui::Popup::is_id_open(&ctx, id));
    ctx.data_mut(|d| d.insert_temp(marks_id(), held || down));

    egui::MenuBar::new()
        .style(bar_style as fn(&mut egui::Style))
        .ui(ui, add)
        .inner
}

/// One title on the menu bar, and the menu that drops from it.
///
/// Mark the mnemonic with `&`: `"&File"` is opened by Alt+F.
pub fn top<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let marked = mark(label);
    let mut atoms = egui::Atoms::new("");
    marked.atoms(showing_marks(ui.ctx()), &mut atoms);

    // The title of an open menu is *held down*, and has to look it — including
    // when the menu was opened from the keyboard and the pointer is nowhere
    // near it. The id a widget will take is known before it is added, which is
    // the only way to ask whether its menu is open in time to draw it that way.
    let id = ui.next_auto_id().with("popup");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let mut button = egui::Button::new(atoms).gap(0.0);
    if open {
        let held = ui.visuals().widgets.open;
        button = button.fill(held.weak_bg_fill).stroke(held.bg_stroke);
    }
    let response = ui.add(button);
    let ctx = ui.ctx().clone();
    debug_assert_eq!(id, egui::Popup::default_response_id(&response));

    // Alt and the underlined letter, from anywhere in the window. Consumed, so
    // that the grid below — which reads key events straight out of the frame —
    // does not also see it.
    let by_key = marked.taken(ui, egui::Modifiers::ALT);
    if by_key {
        egui::Popup::open_id(&ctx, id);
        ctx.data_mut(|d| {
            d.insert_temp(open_id(), id);
            d.insert_temp(marks_id(), true);
        });
    }

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
/// Mark the mnemonic with `&`: inside an open menu, `"&New"` runs on N. Pass an
/// empty `shortcut` for a command that has no key. Choosing closes the menu,
/// because a menu that stays open after its command has run is a menu covering
/// the thing the command just changed.
pub fn item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> Item {
    entry(ui, label, shortcut, None)
}

/// A command that is either on or off, marked with a tick when it is on.
pub fn check(ui: &mut egui::Ui, label: &str, shortcut: &str, on: bool) -> Item {
    entry(ui, label, shortcut, Some(on))
}

fn entry(ui: &mut egui::Ui, label: &str, shortcut: &str, checked: Option<bool>) -> Item {
    let marked = mark(label);
    let tick = egui::RichText::new(TICK).size(12.0).color(match checked {
        Some(true) => ACCENT,
        // The gutter is still spent, so that every label in the menu starts at
        // the same x whatever any of them is doing.
        _ => egui::Color32::TRANSPARENT,
    });
    let mut atoms = egui::Atoms::new(tick);
    atoms.push_right("".atom_size(egui::vec2(GUTTER, 0.0)));
    marked.atoms(showing_marks(ui.ctx()), &mut atoms);
    let mut button = egui::Button::new(atoms).gap(0.0);
    if !shortcut.is_empty() {
        button = button.shortcut_text(shortcut);
    }
    let response = ui.add(button);

    // An open menu owns the keyboard, so the bare letter is enough. Consumed
    // rather than merely read, which is what stops the row above from also
    // answering to it — and what stops the letter reaching the grid, where it
    // would start typing into a cell.
    let by_key = ui.is_enabled() && marked.taken(ui, egui::Modifiers::NONE);
    if response.clicked() || by_key {
        ui.close();
    }
    Item { response, by_key }
}

/// A submenu, opened by resting on its row — or by its letter.
pub fn sub<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let marked = mark(label);
    let tick = egui::RichText::new(TICK)
        .size(12.0)
        .color(egui::Color32::TRANSPARENT);
    let mut atoms = egui::Atoms::new(tick);
    atoms.push_right("".atom_size(egui::vec2(GUTTER, 0.0)));
    marked.atoms(showing_marks(ui.ctx()), &mut atoms);
    let button = egui::Button::new(atoms)
        .gap(0.0)
        .right_text(egui::containers::menu::SubMenuButton::RIGHT_ARROW);
    let by_key = ui.is_enabled() && marked.taken(ui, egui::Modifiers::NONE);

    let (response, inner) =
        egui::containers::menu::SubMenuButton::from_button(button).ui(ui, |ui| {
            ui.set_min_width(MIN_WIDTH);
            add(ui)
        });

    // A submenu is opened by telling the menu it belongs to which of its rows
    // is open — the same thing resting on the row does, one frame later.
    //
    // `mark_shown` first, and it is not optional: a menu forgets an open row
    // whose submenu has not been drawn for a frame, which is a sound rule for
    // a submenu that has gone away and exactly wrong for one that has not been
    // drawn *yet*. Without it the row is opened and forgotten in the same
    // breath, and the letter appears to do nothing at all.
    if by_key {
        let child = egui::containers::menu::SubMenu::id_from_widget_id(response.id);
        egui::containers::menu::MenuState::mark_shown(ui.ctx(), child);
        egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item = Some(child));
        ui.ctx().request_repaint();
    }

    inner.map(|inner| inner.inner)
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
        painted_with_marks(false, add)
    }

    fn painted_with_marks(
        marks: bool,
        add: impl FnOnce(&mut egui::Ui),
    ) -> Vec<egui::epaint::Shape> {
        let ctx = egui::Context::default();
        crate::fonts::register(&ctx, &[]);
        ctx.data_mut(|d| d.insert_temp(marks_id(), marks));
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
        // The warm-up ran `bar`, which decides for itself whether the marks
        // show. Put the answer the test wants back before the pass it reads.
        ctx.data_mut(|d| d.insert_temp(marks_id(), marks));
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

    #[test]
    fn a_label_says_where_its_mnemonic_is_with_an_ampersand() {
        let cases = [
            ("&File", ("", Some('F'), "ile")),
            ("Save &As…", ("Save ", Some('A'), "s…")),
            ("E&xit", ("E", Some('x'), "it")),
            // Nothing marked, and a stray trailing marker, both come back as
            // the label written out — never as a swallowed character.
            ("Automatic", ("Automatic", None, "")),
            ("Rows &", ("Rows &", None, "")),
        ];
        for (label, want) in cases {
            let got = mark(label);
            assert_eq!((got.before, got.key, got.after), want, "{label:?}");
        }
        assert_eq!(mark("&File").key(), Some(egui::Key::F));
        assert_eq!(mark("Cu&t").key(), Some(egui::Key::T));
        assert_eq!(mark("Automatic").key(), None);
    }

    /// A letter that chooses a menu command has to be *taken*, not read.
    ///
    /// The bug: `consume_key` removes the key event and leaves the `Text` event
    /// beside it, and `Text` is what anything accepting typing reads. So Alt+V
    /// then P split the panes and typed "p" into the cell underneath.
    #[test]
    fn a_menu_letter_is_taken_from_the_keyboard_rather_than_merely_read() {
        let ctx = egui::Context::default();
        crate::fonts::register(&ctx, &[]);
        let mut left = Vec::new();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events: vec![
                    egui::Event::Key {
                        key: egui::Key::P,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Text("p".to_string()),
                ],
                ..Default::default()
            },
            |ui| {
                assert!(mark("S&plit").taken(ui, egui::Modifiers::NONE));
                left = ui.input(|i| i.events.clone());
            },
        );
        out.textures_delta.clear();
        assert!(left.is_empty(), "the keyboard still holds {left:?}");
    }

    /// The marker is notation, not text. If an `&` ever reaches the screen the
    /// menu reads as a debugging artefact — and it is the sort of thing that
    /// survives review because the code looks right.
    #[test]
    fn the_ampersand_never_reaches_the_screen() {
        for marks in [false, true] {
            let shapes = painted_with_marks(marks, |ui| {
                in_a_menu(ui, |ui| {
                    entry(ui, "Save &As…", "Ctrl+Shift+S", None);
                });
            });
            let mut said = String::new();
            for shape in &shapes {
                if let egui::epaint::Shape::Text(text) = shape {
                    said.push_str(&text.galley.job.text);
                }
            }
            assert!(!said.contains('&'), "marks={marks}: painted {said:?}");
            assert!(said.contains("Save "), "marks={marks}: painted {said:?}");
            assert!(said.contains('A'), "marks={marks}: painted {said:?}");
            assert!(said.contains("s…"), "marks={marks}: painted {said:?}");
        }
    }

    /// The underline is the whole point of the mnemonic, and it is meant to
    /// appear only when somebody reaches for Alt.
    #[test]
    fn the_mnemonic_is_underlined_only_while_the_marks_are_showing() {
        fn underlined(marks: bool) -> bool {
            let shapes = painted_with_marks(marks, |ui| {
                in_a_menu(ui, |ui| {
                    entry(ui, "&New", "Ctrl+N", None);
                });
            });
            shapes.iter().any(|shape| match shape {
                egui::epaint::Shape::Text(text) => {
                    text.galley.job.text == "N"
                        && text
                            .galley
                            .job
                            .sections
                            .iter()
                            .any(|section| section.format.underline.width > 0.0)
                }
                _ => false,
            })
        }
        assert!(underlined(true), "Alt was held and nothing was underlined");
        assert!(!underlined(false), "the underline showed unasked");
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
