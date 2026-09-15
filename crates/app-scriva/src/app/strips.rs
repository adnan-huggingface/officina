//! The mode strips: one row under the toolbar that appears for the thing the
//! caret is in — a table, a picked picture — and goes when it is left.
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

impl Scriva {
    /// The strip for the body's caret: the picked picture's, else the caret
    /// table's, else none — one at a time, and the picture first because it
    /// is the thing most recently clicked.
    pub(super) fn strip(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        if let Some((width, height)) = self.picked_size() {
            rule(ui);
            return picture_strip(ui, width, height);
        }
        if let Some(table) = self.table_at_caret() {
            rule(ui);
            return table_strip(ui, table);
        }
        None
    }
}

/// `Table` · `3 × 4` · the row and column inserts · Delete ▾ · Merge ·
/// Borders ▾ · Shading ▾ · Width… · Margins….
fn table_strip(ui: &mut egui::Ui, table: TableAt) -> Option<Command> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        ui.set_min_height(theme::TARGET.y);
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
    });
    chosen
}

/// `Picture` · `3.25 × 2.10 in` · Size… · Align ▾ · Original size · Delete.
/// No wrap: the layout does not wrap text round a picture, and a strip that
/// offered it would be promising what the page cannot show.
fn picture_strip(ui: &mut egui::Ui, width: f64, height: f64) -> Option<Command> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        ui.set_min_height(theme::TARGET.y);
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
    });
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

/// A hairline between the strip's groups.
fn divider(ui: &mut egui::Ui) {
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
