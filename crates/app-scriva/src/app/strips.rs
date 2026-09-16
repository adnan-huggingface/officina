//! The row under the toolbar, and the mode strips drawn on it.
//!
//! The row is always there and always the same height. It says one thing
//! at a time — the find bar while it is open, news about the document
//! while there is some, else the strip for the thing the caret is in: a
//! header being edited, a picked picture, a table — and nothing when there
//! is nothing to say. It used to appear only when it had something to say,
//! and the page moved down a row every time the caret entered a table and
//! back up when it left; a chrome that changes height under the pointer is
//! a page that will not hold still.
//!
//! The commands in a strip are the Table menu's and the picture's, in the
//! same words, so that a strip is a shortcut to a menu the user has already
//! seen and not a second vocabulary. Every control is a flat text chip with
//! the menu's own tooltip; the strip's first word says what it is about, in
//! bold, and the second — `3 × 4`, `3.25 × 2.10 in` — says the one number a
//! glance wants.

use super::*;
use crate::commands::tooltip;
use crate::icons;
use crate::toolbar::PALETTE;
use ui_kit::{menu, theme};

/// Space between chips, and the padding inside one.
const GAP: f32 = 4.0;
const PAD: f32 = 8.0;

/// What the row has to say this frame, first wins: the find bar is open
/// because the user opened it, news outranks a mode because it is new,
/// and the way out of a band matters more than a table in it.
enum Says {
    Find,
    Notice,
    Band,
    Picture(f64, f64),
    Table(TableAt),
    Nothing,
}

impl Scriva {
    /// The row under the toolbar: one row of `theme::STRIP`, whatever it
    /// says, so that the desk below never moves. Returns the command a
    /// control on it chose, and leaves in `find_held` whether the find bar
    /// held the keyboard.
    pub(super) fn context_row(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let says = if self.finder.is_some() {
            Says::Find
        } else if !self.notices.is_empty() {
            Says::Notice
        } else if matches!(self.scope, wp_model::Scope::Chrome(_)) {
            Says::Band
        } else if let Some((width, height)) = self.picked_size() {
            Says::Picture(width, height)
        } else if let Some(table) = self.table_at_caret() {
            Says::Table(table)
        } else {
            Says::Nothing
        };
        // The hairline's space is always taken, and the line is drawn only
        // over a row with something on it: an empty row under a rule reads
        // as an empty bar, and under none as the toolbar's own margin.
        let width = ui.available_width();
        let (line, _) = ui.allocate_exact_size(egui::vec2(width, 5.0), egui::Sense::hover());
        if !matches!(says, Says::Nothing) {
            let y = line.center().y.round() + 0.5;
            ui.painter().hline(
                line.x_range(),
                y,
                egui::Stroke::new(1.0, theme::CHROME_RULE),
            );
        }
        self.find_held = false;
        let mut chosen = None;
        ui.allocate_ui_with_layout(
            egui::vec2(width, theme::STRIP),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(width, theme::STRIP));
                ui.spacing_mut().item_spacing.x = GAP;
                match says {
                    Says::Find => self.find_held = self.find_bar(ui),
                    Says::Notice => chosen = self.notice_bar(ui),
                    Says::Band => chosen = self.band_bar(ui),
                    Says::Picture(width, height) => chosen = picture_strip(ui, width, height),
                    Says::Table(table) => chosen = table_strip(ui, table),
                    Says::Nothing => {}
                }
            },
        );
        chosen
    }
}

/// `Table` · `3 × 4` · the row and column inserts · Delete ▾ · Merge ·
/// Borders ▾ · Shading ▾ · Width… · Margins….
fn table_strip(ui: &mut egui::Ui, table: TableAt) -> Option<Command> {
    let mut chosen = None;
    {
        ui.add_space(PAD);
        title(ui, "Table", &format!("{} × {}", table.rows, table.columns));
        divider(ui);
        for (label, command) in [
            ("Row above", Command::InsertRow { below: false }),
            ("Row below", Command::InsertRow { below: true }),
            ("Column left", Command::InsertColumn { after: false }),
            ("Column right", Command::InsertColumn { after: true }),
        ] {
            if chip(ui, label, &tooltip(&command)).clicked() {
                chosen = Some(command);
            }
        }
        divider(ui);
        chosen = chosen.take().or(chip_menu(ui, "Delete", |ui| {
            let mut picked = None;
            for (label, command) in [
                ("&Row", Command::DeleteRow),
                ("&Column", Command::DeleteColumn),
                ("&Table", Command::DeleteTable),
            ] {
                if menu::item(ui, label, "").clicked() {
                    picked = Some(command);
                }
            }
            picked
        }));
        if chip(ui, "Merge", &tooltip(&Command::MergeCells)).clicked() {
            chosen = Some(Command::MergeCells);
        }
        chosen = chosen.take().or(chip_menu(ui, "Borders", |ui| {
            let mut picked = None;
            if menu::item(ui, "&All", "").clicked() {
                picked = Some(Command::TableBorders(true));
            }
            if menu::item(ui, "&None", "").clicked() {
                picked = Some(Command::TableBorders(false));
            }
            picked
        }));
        chosen = chosen.take().or(chip_menu(ui, "Shading", |ui| {
            shading_rows(ui, table.shading)
        }));
        divider(ui);
        if chip(ui, "Width…", &tooltip(&Command::ColumnWidth)).clicked() {
            chosen = Some(Command::ColumnWidth);
        }
        if chip(ui, "Margins…", &tooltip(&Command::CellMargins)).clicked() {
            chosen = Some(Command::CellMargins);
        }
    }
    chosen
}

/// `Picture` · `3.25 × 2.10 in` · Size… · Align ▾ · Original size · Delete.
/// No wrap: the layout does not wrap text round a picture, and a strip that
/// offered it would be promising what the page cannot show.
fn picture_strip(ui: &mut egui::Ui, width: f64, height: f64) -> Option<Command> {
    let mut chosen = None;
    {
        ui.add_space(PAD);
        title(
            ui,
            "Picture",
            &format!("{:.2} × {:.2} in", width / 72.0, height / 72.0),
        );
        divider(ui);
        if chip(ui, "Size…", &tooltip(&Command::PictureSize)).clicked() {
            chosen = Some(Command::PictureSize);
        }
        chosen = chosen.take().or(chip_menu(ui, "Align", |ui| {
            use wp_model::doc::Alignment;
            let mut picked = None;
            for (label, alignment) in [
                ("&Left", Alignment::Left),
                ("&Centre", Alignment::Center),
                ("&Right", Alignment::Right),
            ] {
                if menu::item(ui, label, "").clicked() {
                    picked = Some(Command::AlignPicture(alignment));
                }
            }
            picked
        }));
        if chip(ui, "Original size", &tooltip(&Command::PictureOriginalSize)).clicked() {
            chosen = Some(Command::PictureOriginalSize);
        }
        divider(ui);
        if chip(ui, "Delete", "Delete Picture  Del").clicked() {
            chosen = Some(Command::DeletePicture);
        }
    }
    chosen
}

/// The swatch rows the Shading menus share: No Fill, the palette with the
/// caret cell's own fill marked, no dialog row.
pub(crate) fn shading_rows(ui: &mut egui::Ui, current: Option<[u8; 3]>) -> Option<Command> {
    let colours: Vec<(&str, egui::Color32)> = PALETTE
        .iter()
        .map(|(name, [r, g, b])| (*name, egui::Color32::from_rgb(*r, *g, *b)))
        .collect();
    let current = current.and_then(|rgb| PALETTE.iter().position(|(_, other)| *other == rgb));
    match menu::swatches(ui, "&No Fill", &colours, current, None)? {
        menu::Swatch::First => Some(Command::TableShading(None)),
        menu::Swatch::Index(index) => Some(Command::TableShading(Some(PALETTE[index].1))),
        menu::Swatch::More => None,
    }
}

/// The strip's first two words: what it is about, and its one number.
fn title(ui: &mut egui::Ui, what: &str, figure: &str) {
    ui.label(
        egui::RichText::new(what)
            .strong()
            .size(theme::TEXT)
            .color(theme::INK),
    );
    ui.add_space(GAP);
    ui.label(
        egui::RichText::new(figure)
            .size(theme::TEXT)
            .color(theme::INK_SOFT),
    );
    ui.add_space(GAP);
}

/// A hairline between the row's groups.
pub(super) fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(PAD * 2.0 + 1.0, theme::TARGET.y),
        egui::Sense::hover(),
    );
    let x = rect.center().x.round() + 0.5;
    ui.painter().vline(
        x,
        (rect.top() + 4.0)..=(rect.bottom() - 4.0),
        egui::Stroke::new(1.0, theme::CHROME_RULE),
    );
}

/// A flat text button: the running size, padded, tinted for its state.
fn chip(ui: &mut egui::Ui, label: &str, tip: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(theme::TEXT),
        theme::INK,
    );
    let size = egui::vec2(galley.size().x + PAD * 2.0, theme::TARGET.y);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    icons::paint_state(ui, rect, &response, false);
    let at = egui::pos2(rect.left() + PAD, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, theme::INK);
    response.on_hover_text(tip)
}

/// A chip with a chevron that opens a menu of rows under it. The chevron is
/// drawn, as the toolbar's are: the chrome face has no glyph for it, and a
/// box where an arrow should be is what a strip would wear otherwise.
fn chip_menu(
    ui: &mut egui::Ui,
    label: &str,
    rows: impl FnOnce(&mut egui::Ui) -> Option<Command>,
) -> Option<Command> {
    const CHEVRON: f32 = 12.0;
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(theme::TEXT),
        theme::INK,
    );
    let size = egui::vec2(galley.size().x + PAD * 2.0 + CHEVRON, theme::TARGET.y);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    icons::paint_state(ui, rect, &response, false);
    let at = egui::pos2(rect.left() + PAD, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, theme::INK);
    icons::draw(
        ui.painter(),
        icons::Icon::ChevronDown,
        egui::pos2(rect.right() - PAD / 2.0 - CHEVRON / 2.0, rect.center().y),
        theme::INK,
    );
    let response = response.on_hover_text(label);
    menu::under(&response, rows).flatten()
}
