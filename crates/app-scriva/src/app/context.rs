//! The page's right-click menu: what a click or Shift+F10 on the document
//! offers, drawn on the same card as every other menu.
//!
//! One menu for text, with rows added in front of it for what the click
//! landed on — a link, a table, a tracked change — and a menu of its own for
//! a picked picture, which is not a stretch of text. Every row is a command
//! the menus already have, in the menus' words, so that the menu at the
//! pointer teaches the menu bar and not a second vocabulary. What the click
//! landed on is read once, when the menu opens, and kept: the menu outlives
//! the click, and the pointer has moved on by the time the rows are drawn.

use super::*;
use crate::commands::shortcut;
use ui_kit::menu;

/// What the menu was opened on, read when it opened.
#[derive(Debug, Clone, Default)]
pub(super) struct ContextState {
    /// A picked picture: its own menu, and none of the text's rows.
    pub picture: bool,
    pub selection: bool,
    /// The link under the click, if any.
    pub link: Option<crate::links::Destination>,
    pub in_table: bool,
    /// The tracked change the caret stands in, if any.
    pub change: Option<wp_model::Mark>,
    pub styles: Vec<(wp_model::StyleId, String)>,
    pub style: Option<wp_model::StyleId>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Scriva {
    /// Reads what the menu is about, at the caret — which a right-click has
    /// already moved to the click, and Shift+F10 leaves where it is.
    pub(super) fn context_state(&mut self) -> ContextState {
        let (bold, italic, underline) = self.emphasis();
        let caret = self.caret();
        let change = crate::revise::tracked(&self.document)
            .into_iter()
            .filter(|change| change.scope == self.scope && change.paragraph == caret.paragraph)
            .min_by_key(|change| change.offset.abs_diff(caret.offset))
            .filter(|change| change.offset.abs_diff(caret.offset) <= change.text.len())
            .map(|change| change.mark);
        ContextState {
            picture: self.picked.is_some(),
            selection: !self.selection.is_empty(),
            link: self.menu_link.clone(),
            in_table: edit::table_cell_at(&self.document, self.scope, caret).is_some(),
            change,
            styles: self.quick_styles(),
            style: self.style_at(),
            bold,
            italic,
            underline,
        }
    }
}

/// The rows, for the state the menu was opened in. Letters are unique down
/// the whole menu, prepended rows included, which a test holds them to.
pub(super) fn context_rows(ui: &mut egui::Ui, state: &ContextState) -> Option<Command> {
    let mut chosen = None;
    if state.picture {
        if let Some(command) = row(ui, "&Cut", Command::Cut) {
            chosen = Some(command);
        }
        if let Some(command) = row(ui, "Cop&y", Command::Copy) {
            chosen = Some(command);
        }
        menu::sep(ui);
        if let Some(command) = row(ui, "&Size…", Command::PictureSize) {
            chosen = Some(command);
        }
        menu::sub(ui, "&Align", |ui| {
            use wp_model::doc::Alignment;
            if let Some(command) = row(ui, "&Left", Command::AlignPicture(Alignment::Left)) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "&Centre", Command::AlignPicture(Alignment::Center)) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "&Right", Command::AlignPicture(Alignment::Right)) {
                chosen = Some(command);
            }
        });
        if let Some(command) = row(ui, "&Original size", Command::PictureOriginalSize) {
            chosen = Some(command);
        }
        menu::sep(ui);
        if let Some(command) = row(ui, "&Delete", Command::DeletePicture) {
            chosen = Some(command);
        }
        return chosen;
    }
    if state.link.is_some() {
        // Word offers this too, and it is how a reader who never hears
        // about the modifier follows a link.
        if let Some(command) = row(ui, "&Open Hyperlink", Command::OpenLink) {
            chosen = Some(command);
        }
        if let Some(command) = row(ui, "Copy Lin&k Address", Command::CopyLinkAddress) {
            chosen = Some(command);
        }
        menu::sep(ui);
    }
    if state.in_table {
        menu::sub(ui, "&Table", |ui| {
            if let Some(command) = row(ui, "Row &Above", Command::InsertRow { below: false }) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "Row &Below", Command::InsertRow { below: true }) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "Column &Left", Command::InsertColumn { after: false }) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "Column &Right", Command::InsertColumn { after: true }) {
                chosen = Some(command);
            }
            menu::sep(ui);
            if let Some(command) = row(ui, "&Delete Row", Command::DeleteRow) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "Delete &Column", Command::DeleteColumn) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "Delete &Table", Command::DeleteTable) {
                chosen = Some(command);
            }
            menu::sep(ui);
            if let Some(command) = row(ui, "&Merge Cells", Command::MergeCells) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "Column &Width…", Command::ColumnWidth) {
                chosen = Some(command);
            }
            if let Some(command) = row(ui, "Cell Mar&gins…", Command::CellMargins) {
                chosen = Some(command);
            }
        });
        menu::sep(ui);
    }
    if let Some(mark) = &state.change {
        if let Some(command) = row(ui, "Acc&ept Change", Command::AcceptChange(mark.clone())) {
            chosen = Some(command);
        }
        if let Some(command) = row(ui, "Re&ject Change", Command::RejectChange(mark.clone())) {
            chosen = Some(command);
        }
        menu::sep(ui);
    }
    ui.add_enabled_ui(state.selection, |ui| {
        if let Some(command) = row(ui, "&Cut", Command::Cut) {
            chosen = Some(command);
        }
        if let Some(command) = row(ui, "Cop&y", Command::Copy) {
            chosen = Some(command);
        }
    });
    if let Some(command) = row(ui, "&Paste", Command::Paste) {
        chosen = Some(command);
    }
    if let Some(command) = row(ui, "Paste &Unformatted", Command::PasteUnformatted) {
        chosen = Some(command);
    }
    menu::sep(ui);
    for (label, on, command) in [
        ("&Bold", state.bold, Command::Bold),
        ("&Italic", state.italic, Command::Italic),
        ("Under&line", state.underline, Command::Underline),
    ] {
        if menu::check(ui, label, shortcut(&command), on).clicked() {
            chosen = Some(command);
        }
    }
    menu::sep(ui);
    if let Some(command) = row(ui, "Pa&ragraph…", Command::ParagraphDialog) {
        chosen = Some(command);
    }
    menu::sub(ui, "&Styles", |ui| {
        for (id, name) in &state.styles {
            if menu::check(ui, name, "", state.style.as_ref() == Some(id)).clicked() {
                chosen = Some(Command::Style(*id));
            }
        }
        if state.styles.is_empty() {
            ui.add_enabled(false, egui::Button::new("No styles in this document"));
        }
    });
    menu::sep(ui);
    if let Some(command) = row(ui, "&New Comment", Command::AddComment) {
        chosen = Some(command);
    }
    menu::sep(ui);
    if let Some(command) = row(ui, "Select &All", Command::SelectAll) {
        chosen = Some(command);
    }
    chosen
}

/// One row: the command's name as marked, its key from the table.
fn row(ui: &mut egui::Ui, label: &str, command: Command) -> Option<Command> {
    menu::item(ui, label, shortcut(&command))
        .clicked()
        .then_some(command)
}
