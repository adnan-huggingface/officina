//! Every command with a name and a key, in one table.
//!
//! The menus print a shortcut beside each row, the toolbar prints one in each
//! tooltip, the keyboard handler matches one, and the user guide lists them —
//! four copies of the same fact, and the audit found them disagreeing: the
//! Review menu promised Alt+F7 for Next Change and no key handler had ever
//! heard of it. So the fact lives here once. A menu row asks [`shortcut`] for
//! what to print, a tooltip asks [`tooltip`], the frame asks [`keys`] which
//! command a keystroke was, and the guide's tables are checked against the
//! same rows. A key that is claimed and not wired can no longer exist,
//! because claiming it is wiring it.

use ui_kit::egui::{Key, Modifiers};
use wp_model::prop::Justify;
use wp_model::units::Line240;

use crate::app::Command;

/// One command as the keyboard, the menus and the tooltips know it.
pub struct Entry {
    /// The name a tooltip and the guide use — without the menu's `&` mark
    /// and without a trailing ellipsis.
    pub name: &'static str,
    /// The key as printed: `Ctrl+Shift+>` for a key egui calls Period.
    pub shown: &'static str,
    /// The key as pressed, with exactly these modifiers. `None` for a key
    /// the window does not read itself — Alt+F4 is the window manager's, and
    /// Ctrl+C reaches the document as a copy event rather than a key.
    pub key: Option<(Modifiers, Key)>,
    pub command: Command,
}

const CTRL: Modifiers = Modifiers::COMMAND;
const CTRL_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const CTRL_ALT: Modifiers = Modifiers::COMMAND.plus(Modifiers::ALT);
const NONE: Modifiers = Modifiers::NONE;

const fn entry(
    name: &'static str,
    shown: &'static str,
    key: Option<(Modifiers, Key)>,
    command: Command,
) -> Entry {
    Entry {
        name,
        shown,
        key,
        command,
    }
}

/// The table. A command with two keys has two rows; the first is the one
/// the menus print.
pub const TABLE: &[Entry] = &[
    entry("New", "Ctrl+N", Some((CTRL, Key::N)), Command::New),
    entry("Open", "Ctrl+O", Some((CTRL, Key::O)), Command::Open),
    entry("Save", "Ctrl+S", Some((CTRL, Key::S)), Command::Save),
    entry(
        "Save As",
        "Ctrl+Shift+S",
        Some((CTRL_SHIFT, Key::S)),
        Command::SaveAs,
    ),
    entry("Print", "Ctrl+P", Some((CTRL, Key::P)), Command::Print),
    entry("Export as PDF", "", None, Command::ExportPdf),
    entry("Close", "Ctrl+W", Some((CTRL, Key::W)), Command::Close),
    entry("Exit", "Alt+F4", None, Command::Exit),
    entry("Undo", "Ctrl+Z", Some((CTRL, Key::Z)), Command::Undo),
    entry("Redo", "Ctrl+Y", Some((CTRL, Key::Y)), Command::Redo),
    entry(
        "Redo",
        "Ctrl+Shift+Z",
        Some((CTRL_SHIFT, Key::Z)),
        Command::Redo,
    ),
    entry("Cut", "Ctrl+X", None, Command::Cut),
    entry("Copy", "Ctrl+C", None, Command::Copy),
    entry("Paste", "Ctrl+V", None, Command::Paste),
    entry(
        "Paste Unformatted",
        "Ctrl+Shift+V",
        None,
        Command::PasteUnformatted,
    ),
    entry("Find", "Ctrl+F", Some((CTRL, Key::F)), Command::Find),
    entry("Replace", "Ctrl+H", Some((CTRL, Key::H)), Command::Replace),
    entry("Find Next", "F3", Some((NONE, Key::F3)), Command::FindNext),
    entry(
        "Find Previous",
        "Shift+F3",
        Some((Modifiers::SHIFT, Key::F3)),
        Command::FindPrevious,
    ),
    entry(
        "Select All",
        "Ctrl+A",
        Some((CTRL, Key::A)),
        Command::SelectAll,
    ),
    entry("Bold", "Ctrl+B", Some((CTRL, Key::B)), Command::Bold),
    entry("Italic", "Ctrl+I", Some((CTRL, Key::I)), Command::Italic),
    entry(
        "Underline",
        "Ctrl+U",
        Some((CTRL, Key::U)),
        Command::Underline,
    ),
    entry("Strikethrough", "", None, Command::Strike),
    entry(
        "Superscript",
        "Ctrl+Shift+=",
        Some((CTRL_SHIFT, Key::Equals)),
        Command::Superscript,
    ),
    entry(
        "Subscript",
        "Ctrl+=",
        Some((CTRL, Key::Equals)),
        Command::Subscript,
    ),
    entry(
        "Grow",
        "Ctrl+Shift+>",
        Some((CTRL_SHIFT, Key::Period)),
        Command::Grow,
    ),
    entry(
        "Shrink",
        "Ctrl+Shift+<",
        Some((CTRL_SHIFT, Key::Comma)),
        Command::Shrink,
    ),
    entry(
        "Clear Formatting",
        "Ctrl+Space",
        Some((CTRL, Key::Space)),
        Command::ClearFormatting,
    ),
    entry("Watermark", "", None, Command::Watermark),
    entry("Picture Size", "", None, Command::PictureSize),
    entry("Original Size", "", None, Command::PictureOriginalSize),
    entry(
        "Picture Left",
        "",
        None,
        Command::AlignPicture(wp_model::doc::Alignment::Left),
    ),
    entry(
        "Picture Centre",
        "",
        None,
        Command::AlignPicture(wp_model::doc::Alignment::Center),
    ),
    entry(
        "Picture Right",
        "",
        None,
        Command::AlignPicture(wp_model::doc::Alignment::Right),
    ),
    entry("Delete Picture", "", None, Command::DeletePicture),
    entry("Bullets", "", None, Command::Bullets),
    entry("Numbering", "", None, Command::Numbers),
    entry(
        "Align Left",
        "Ctrl+L",
        Some((CTRL, Key::L)),
        Command::Align(Justify::Start),
    ),
    entry(
        "Centre",
        "Ctrl+E",
        Some((CTRL, Key::E)),
        Command::Align(Justify::Center),
    ),
    entry(
        "Align Right",
        "Ctrl+R",
        Some((CTRL, Key::R)),
        Command::Align(Justify::End),
    ),
    entry(
        "Justify",
        "Ctrl+J",
        Some((CTRL, Key::J)),
        Command::Align(Justify::Both),
    ),
    entry(
        "Single",
        "Ctrl+1",
        Some((CTRL, Key::Num1)),
        Command::LineSpacing(Line240::SINGLE),
    ),
    entry(
        "1.5 Lines",
        "Ctrl+5",
        Some((CTRL, Key::Num5)),
        Command::LineSpacing(Line240::ONE_AND_A_HALF),
    ),
    entry(
        "Double",
        "Ctrl+2",
        Some((CTRL, Key::Num2)),
        Command::LineSpacing(Line240::DOUBLE),
    ),
    entry(
        "Increase Indent",
        "Ctrl+M",
        Some((CTRL, Key::M)),
        Command::Indent(1),
    ),
    entry(
        "Decrease Indent",
        "Ctrl+Shift+M",
        Some((CTRL_SHIFT, Key::M)),
        Command::Indent(-1),
    ),
    entry("Paragraph", "", None, Command::ParagraphDialog),
    entry("Custom Margins", "", None, Command::CustomMargins),
    entry(
        "Page Break",
        "Ctrl+Enter",
        Some((CTRL, Key::Enter)),
        Command::PageBreak,
    ),
    entry("Picture", "", None, Command::InsertPicture),
    entry("Insert Table", "", None, Command::InsertTable),
    entry(
        "Update Table of Contents",
        "F9",
        Some((NONE, Key::F9)),
        Command::UpdateToc,
    ),
    entry("Edit Header", "", None, Command::EditHeader),
    entry("Edit Footer", "", None, Command::EditFooter),
    entry("Row Above", "", None, Command::InsertRow { below: false }),
    entry("Row Below", "", None, Command::InsertRow { below: true }),
    entry(
        "Column Left",
        "",
        None,
        Command::InsertColumn { after: false },
    ),
    entry(
        "Column Right",
        "",
        None,
        Command::InsertColumn { after: true },
    ),
    entry("Delete Row", "", None, Command::DeleteRow),
    entry("Delete Column", "", None, Command::DeleteColumn),
    entry("Delete Table", "", None, Command::DeleteTable),
    entry("Column Width", "", None, Command::ColumnWidth),
    entry("Cell Margins", "", None, Command::CellMargins),
    entry("Merge Cells", "", None, Command::MergeCells),
    entry(
        "Track Changes",
        "Ctrl+Shift+E",
        Some((CTRL_SHIFT, Key::E)),
        Command::TrackChanges,
    ),
    entry(
        "Next Change",
        "Alt+F7",
        Some((Modifiers::ALT, Key::F7)),
        Command::NextChange,
    ),
    entry(
        "Previous Change",
        "Alt+Shift+F7",
        Some((Modifiers::ALT.plus(Modifiers::SHIFT), Key::F7)),
        Command::PreviousChange,
    ),
    entry("Accept", "", None, Command::AcceptOne),
    entry("Reject", "", None, Command::RejectOne),
    entry("Accept All", "", None, Command::AcceptAll),
    entry("Reject All", "", None, Command::RejectAll),
    entry(
        "New Comment",
        "Ctrl+Alt+M",
        Some((CTRL_ALT, Key::M)),
        Command::AddComment,
    ),
    entry("Reply to Comment", "", None, Command::ReplyHere),
    entry("Resolve Comment", "", None, Command::ResolveHere),
    entry("Delete Comment", "", None, Command::DeleteComment),
    entry(
        "Reviewing Pane",
        "Alt+Shift+C",
        Some((Modifiers::ALT.plus(Modifiers::SHIFT), Key::C)),
        Command::Reviewer,
    ),
    entry("Navigation Pane", "", None, Command::Navigator),
    entry(
        "Context Menu",
        "Shift+F10",
        Some((Modifiers::SHIFT, Key::F10)),
        Command::ContextMenu,
    ),
    entry("Open Hyperlink", "", None, Command::OpenLink),
    entry("Copy Link Address", "", None, Command::CopyLinkAddress),
    entry(
        "Formatting Marks",
        "Ctrl+Shift+8",
        Some((CTRL_SHIFT, Key::Num8)),
        Command::ShowMarks,
    ),
    entry("Tracked Changes", "", None, Command::ShowRevisions),
    entry("Comments", "", None, Command::ShowComments),
    entry("Header and Footer", "", None, Command::EditHeader),
];

/// The table's row for a command: the first, where it has two.
pub fn entry_for(command: &Command) -> Option<&'static Entry> {
    TABLE.iter().find(|entry| entry.command == *command)
}

/// The key the menus print beside `command`, or nothing.
pub fn shortcut(command: &Command) -> &'static str {
    entry_for(command).map(|entry| entry.shown).unwrap_or("")
}

/// The name the table gives `command`, or nothing for one it does not list.
pub fn name(command: &Command) -> &'static str {
    entry_for(command).map(|entry| entry.name).unwrap_or("")
}

/// What a control's tooltip says: the name, and the key after two spaces
/// where there is one — `Bold  Ctrl+B`.
pub fn tooltip(command: &Command) -> String {
    tooltip_named(name(command), command)
}

/// The same, for a control whose name is not the command's — a toggle that
/// reads `Track changes` where the menu row reads `Track Changes`.
pub fn tooltip_named(name: &str, command: &Command) -> String {
    match shortcut(command) {
        "" => name.to_owned(),
        key => format!("{name}  {key}"),
    }
}

/// The command a keystroke in this frame asks for, taken out of the input so
/// that nothing after reads it again. Exactly these modifiers: Ctrl+Z does
/// not answer for Ctrl+Shift+Z, in any order of the table.
pub fn keys(ui: &ui_kit::egui::Ui) -> Option<Command> {
    for entry in TABLE {
        let Some((modifiers, key)) = entry.key else {
            continue;
        };
        if ui.input_mut(|input| ui_kit::keys::take(input, modifiers, key)) {
            return Some(entry.command.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_claimed_by_one_command_only() {
        // Two commands on one key would be decided by the order of the
        // table, which is no way to decide anything.
        let mut seen: Vec<(Modifiers, Key)> = Vec::new();
        for entry in TABLE {
            if let Some(key) = entry.key {
                assert!(!seen.contains(&key), "{:?} is claimed twice", entry.shown);
                seen.push(key);
            }
        }
    }

    #[test]
    fn a_shown_key_is_a_pressed_key_or_says_why_not() {
        // Every key the menus print is one the frame reads, except the
        // few the operating system or egui delivers another way — the
        // board's three keys arrive as events, Ctrl+Shift+V as the paste
        // event with Shift held.
        for entry in TABLE {
            if entry.shown.is_empty() || entry.key.is_some() {
                continue;
            }
            assert!(
                matches!(
                    entry.shown,
                    "Alt+F4" | "Ctrl+X" | "Ctrl+C" | "Ctrl+V" | "Ctrl+Shift+V"
                ),
                "{} prints {} and nothing reads it",
                entry.name,
                entry.shown
            );
        }
    }
}
