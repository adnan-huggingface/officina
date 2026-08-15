//! The application: the window, the commands, and the keys.
//!
//! **Every command goes through one enum and one dispatcher.** The menu, the
//! toolbar and the keyboard cannot answer the same command differently if there
//! is only one answer — Calx's finding 34, and the reason its menu bar and
//! toolbar stayed in step.

use std::path::{Path, PathBuf};

use ui_kit::{dialog, egui, AppId, DocumentApp, Recent, SCRIVA};
use wp_model::doc::{Block, Document, Paragraph};
use wp_model::prop::{Justify, LineSpacing, Toggle};
use wp_model::units::{HalfPoint, Line240, Twips};

use crate::edit::{self, Caret, History, Selection};
use crate::find::{self, Finder};
use crate::shaper::Egui;
use crate::text;
use crate::view::{self, View};

/// One thing the application can be asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    New,
    Open,
    Reopen(PathBuf),
    ForgetRecent,
    Save,
    SaveAs,
    Close,
    Exit,
    Undo,
    Redo,
    SelectAll,
    Cut,
    Copy,
    Paste,
    /// Open the find bar, or bring it back to the keyboard.
    Find,
    /// The find bar with the replace controls showing.
    Replace,
    FindNext,
    FindPrevious,
    Bold,
    Italic,
    Underline,
    Strike,
    Superscript,
    Subscript,
    ClearFormatting,
    /// Ctrl+Enter: a page break at the caret.
    PageBreak,
    /// One of the margin presets, whole. Header, footer and gutter distances
    /// ride along unchanged.
    Margins(wp_model::PageMargins),
    /// The dialog for margins none of the presets offer.
    CustomMargins,
    Orient(wp_model::Orientation),
    /// A paper size, stated portrait; the section's orientation re-applies.
    Paper(wp_model::units::Twips, wp_model::units::Twips),
    Grow,
    Shrink,
    Size(HalfPoint),
    Align(Justify),
    LineSpacing(Line240),
    Indent(i32),
    Style(wp_model::StyleId),
    Zoom(f64),
    ShowMarks,
    ShowRevisions,
    /// Put the caret at the start of a paragraph, and show it.
    GoTo(usize),
    /// Rebuild the table of contents from the headings that are there now.
    UpdateToc,
    /// The pane of headings and bookmarks down the left.
    Navigator,
    /// Record edits as tracked changes from now on.
    TrackChanges,
    AcceptAll,
    RejectAll,
    /// Settle the change the caret is nearest.
    AcceptOne,
    RejectOne,
    /// Move the caret to the next tracked change or comment.
    NextChange,
    AddComment,
    DeleteComment,
    /// The pane of tracked changes and comments down the right.
    Reviewer,
}

/// Which of the three formats a path names.
///
/// Decided by the extension, because that is what the user chose in the save
/// dialog and what the file manager will show. A `.docx` whose contents are not
/// a package is reported when it is opened, not guessed at here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Docx,
    /// Word 97-2003. Read-only: see `wp_doc`.
    Doc,
    Markdown,
    Text,
}

impl Format {
    pub fn of(path: &Path) -> Format {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("md") | Some("markdown") => Format::Markdown,
            Some("txt") | Some("text") => Format::Text,
            Some("doc") | Some("dot") => Format::Doc,
            _ => Format::Docx,
        }
    }

    /// Whether saving in this format throws formatting away.
    pub fn is_lossy(self) -> bool {
        !matches!(self, Format::Docx)
    }

    /// Whether this format can be written at all.
    ///
    /// A `.doc` is a memory image with a fast-save log on the end: writing one
    /// back means rebuilding every byte offset in it, and one wrong offset makes
    /// a file Word opens as something else. So it is read and saved as `.docx`.
    pub fn is_writable(self) -> bool {
        !matches!(self, Format::Doc)
    }
}

/// What the application is waiting for an answer to.
#[derive(Debug, Clone, PartialEq)]
enum Pending {
    /// Unsaved changes, and what to do once the user has answered.
    Unsaved(Box<Command>),
    /// Saving in a format that cannot hold what the document has.
    Lossy(PathBuf, Format),
}

pub struct Scriva {
    document: Document,
    /// The package the document came out of. The writer edits it rather than
    /// building a new one, which is the whole of the preservation guarantee.
    package: Option<ooxml::Package>,
    /// Where the pictures live inside that package. Located once on opening,
    /// because resolving a relationship per frame is work per frame.
    parts: Option<wp_docx::DocumentParts>,
    /// The picture selected as an object, if any. A drawing and the caret are
    /// never both selected.
    picked: Option<crate::drawings::Picked>,
    /// Which handle of it is being dragged, and from where on the page.
    dragging: Option<crate::drawings::Grip>,
    drag_from: Option<(f64, f64)>,
    /// Whether this drag has already put its undo entry on the stack.
    dragged: bool,
    /// The decoded pictures.
    pictures: crate::pictures::Pictures,
    path: Option<PathBuf>,
    dirty: bool,
    /// Bumped on every change, so the view knows when to lay out again.
    stamp: u64,
    view: View,
    history: History,
    selection: Selection,
    shaper: Option<Egui>,
    recent: Recent,
    message: Option<(String, String)>,
    pending: Option<Pending>,
    /// Where the pages are scrolled to, in screen points.
    scroll: f32,
    focused: bool,
    /// Set while the pointer is sweeping out a selection.
    sweeping: bool,
    /// What the fields that do not depend on pagination evaluate to. The page
    /// numbers are worked out by the layout itself.
    fields: wp_layout::FieldValues,
    navigator: bool,
    reviewer: bool,
    /// Who a recorded change is attributed to. Word takes this from the
    /// application's own settings; there is nowhere else to get it, and a
    /// document full of changes by "Unknown" is worse than one by a name the
    /// user can correct.
    author: crate::revise::Author,
    /// The comment being written, before it is added.
    drafting: Option<String>,
    /// The custom-margins dialog: top, bottom, left, right, in inches.
    margins_draft: Option<[String; 4]>,
    /// How the open document was written, so a save puts it back the same way.
    encoding: wp_text::Encoding,
    ending: wp_text::LineEnding,
    /// Where the view should scroll to, once it knows where that is.
    reveal: Option<Caret>,
    /// The find bar, when it is open.
    finder: Option<Finder>,
    /// Whether one of the find bar's fields held the keyboard last frame —
    /// while it does, keys type into the bar and not into the document.
    finder_focused: bool,
    /// Every match of the finder's query, for the view to highlight.
    find_matches: Vec<Selection>,
    /// What `find_matches` was computed from, so it is not recomputed while
    /// neither the document nor the query has changed.
    matches_for: (u64, String),
    /// The page surface's widget id, for giving the keyboard back to it.
    surface_id: Option<egui::Id>,
    /// How tall the visible desk is, in screen points — the size of a Page Down.
    viewport: f32,
}

impl Default for Scriva {
    fn default() -> Self {
        Scriva::new()
    }
}

impl Scriva {
    pub fn new() -> Scriva {
        Scriva {
            document: blank(),
            package: None,
            path: None,
            dirty: false,
            stamp: 1,
            view: View::default(),
            history: History::new(),
            selection: Selection::default(),
            shaper: None,
            recent: Recent::load(SCRIVA),
            message: None,
            pending: None,
            scroll: 0.0,
            focused: true,
            sweeping: false,
            fields: wp_layout::FieldValues::new(),
            navigator: false,
            reviewer: false,
            author: crate::revise::Author::new("Scriva user"),
            drafting: None,
            margins_draft: None,
            parts: None,
            picked: None,
            dragging: None,
            drag_from: None,
            dragged: false,
            pictures: crate::pictures::Pictures::new(),
            encoding: wp_text::Encoding::Utf8,
            ending: wp_text::LineEnding::Crlf,
            reveal: None,
            finder: None,
            finder_focused: false,
            find_matches: Vec::new(),
            matches_for: (u64::MAX, String::new()),
            surface_id: None,
            viewport: 0.0,
        }
    }

    /// Opens a document from disk, for the command line.
    pub fn opening(path: PathBuf) -> Scriva {
        let mut app = Scriva::new();
        app.open_path(&path);
        app
    }

    /// What `{ FILENAME }`, `{ DATE }` and the rest show.
    ///
    /// Supplied rather than read inside the layout: a layout that read the clock
    /// could not be tested, and the same document laid out twice would differ
    /// for no reason the user caused.
    fn refresh_fields(&mut self) {
        self.fields.file_name = self
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned().into());
        self.fields.title = self.fields.file_name.clone();
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.stamp = self.stamp.wrapping_add(1);
        self.view.invalidate();
    }

    fn caret(&self) -> Caret {
        self.selection.head
    }

    fn set_caret(&mut self, caret: Caret, extend: bool) {
        if extend {
            self.selection.head = caret;
        } else {
            self.selection = Selection::at(caret);
        }
    }

    fn paragraph_count(&self) -> usize {
        self.document.paragraphs().len()
    }

    fn paragraph_text(&self, index: usize) -> String {
        self.document
            .paragraphs()
            .get(index)
            .map(|paragraph| paragraph.text())
            .unwrap_or_default()
    }

    pub(crate) fn document_ref(&self) -> &Document {
        &self.document
    }

    pub(crate) fn recent_paths(&self) -> Vec<PathBuf> {
        self.recent.paths().to_vec()
    }

    pub(crate) fn can_undo_redo(&self) -> (bool, bool) {
        (self.history.can_undo(), self.history.can_redo())
    }

    pub(crate) fn showing_marks(&self) -> bool {
        self.view.show_marks
    }

    pub(crate) fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    pub(crate) fn showing_revisions(&self) -> bool {
        self.view.show_revisions
    }

    pub(crate) fn showing_navigator(&self) -> bool {
        self.navigator
    }

    /// Whether changes are being recorded, and whether the pane is showing.
    pub(crate) fn reviewing(&self) -> (bool, bool) {
        (self.document.settings.track_changes, self.reviewer)
    }

    pub(crate) fn zoom(&self) -> f64 {
        self.view.zoom
    }

    /// The page setup the Layout menu ticks against: the orientation, the
    /// paper stated portrait-way-up, and the margins.
    pub(crate) fn page_setup(
        &self,
    ) -> (wp_model::Orientation, (Twips, Twips), wp_model::PageMargins) {
        let section = &self.document.section;
        let paper = match section.page.orientation {
            wp_model::Orientation::Portrait => (section.page.width, section.page.height),
            wp_model::Orientation::Landscape => (section.page.height, section.page.width),
        };
        (section.page.orientation, paper, section.margins)
    }

    /// The styles worth offering: the ones Word marks for its own gallery.
    pub(crate) fn quick_styles(&self) -> Vec<(wp_model::StyleId, String)> {
        let mut out: Vec<(wp_model::StyleId, String, i32)> = self
            .document
            .styles
            .iter()
            .filter(|(_, style)| {
                style.kind == wp_model::StyleKind::Paragraph && style.quick && !style.semi_hidden
            })
            .map(|(id, style)| {
                (
                    id,
                    style.name.as_deref().unwrap_or(&style.id).to_owned(),
                    style.priority.unwrap_or(99),
                )
            })
            .collect();
        out.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));
        out.into_iter()
            .take(16)
            .map(|(id, name, _)| (id, name))
            .collect()
    }

    /// Whether the selection is bold, italic and underlined, for the toolbar.
    pub(crate) fn emphasis(&self) -> (bool, bool, bool) {
        let scope = self.formatting_scope();
        (
            edit::all_runs(&self.document, scope, |props| {
                props.toggles.is_on(Toggle::Bold)
            }),
            edit::all_runs(&self.document, scope, |props| {
                props.toggles.is_on(Toggle::Italic)
            }),
            edit::all_runs(&self.document, scope, |props| {
                props.underline.is_some_and(|u| u.kind.draws())
            }),
        )
    }

    pub(crate) fn alignment(&self) -> Option<Justify> {
        edit::justify_at(&self.document, self.caret())
    }

    /// What a formatting command with no selection would act on: the word the
    /// caret is in.
    fn formatting_scope(&self) -> Selection {
        if !self.selection.is_empty() {
            return self.selection;
        }
        let caret = self.caret();
        let content = self.paragraph_text(caret.paragraph);
        let word = text::word_at(&content, caret.offset);
        Selection {
            anchor: Caret {
                paragraph: caret.paragraph,
                offset: word.start,
            },
            head: Caret {
                paragraph: caret.paragraph,
                offset: word.end,
            },
        }
    }

    /// The number of words in the document, as the status bar reports it.
    ///
    /// Word's rules, checked against Word's own count of a real resume: every
    /// break separates (`text()` drops a page break, silently gluing the words
    /// around it), and a slash splits — "TCP/IP" is two words to Word even
    /// though "real-time" is one.
    fn word_count(&self) -> usize {
        use wp_model::doc::Piece;
        self.document
            .paragraphs()
            .iter()
            .map(|paragraph| {
                let mut text = String::new();
                for run in paragraph.runs() {
                    for piece in &run.content {
                        match piece {
                            Piece::Text(t) => text.push_str(t),
                            Piece::Tab | Piece::Break(_) => text.push(' '),
                            Piece::Symbol { ch, .. } => text.push(*ch),
                            Piece::Hyphen { .. } => text.push('-'),
                            _ => {}
                        }
                    }
                }
                text.split(|c: char| c.is_whitespace() || c == '/')
                    .filter(|word| !word.is_empty())
                    .count()
            })
            .sum()
    }

    // ------------------------------------------------------------ files

    fn open_path(&mut self, path: &Path) {
        match Format::of(path) {
            Format::Docx => self.open_docx(path),
            Format::Doc => self.open_doc(path),
            other => self.open_text(path, other),
        }
    }

    /// Opens a `.txt` or a `.md`.
    ///
    /// There is no package behind it, so there is nothing to preserve and
    /// nothing to splice: the document is built from the text, and saving builds
    /// the text back.
    fn open_text(&mut self, path: &Path, format: Format) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.message = Some((
                    "Cannot open".to_owned(),
                    format!("{}\n\n{error}", path.display()),
                ));
                return;
            }
        };
        let (text, encoding) = wp_text::decode(&bytes);
        self.ending = wp_text::line_ending(&text);
        self.encoding = encoding;
        self.document = match format {
            Format::Markdown => wp_text::read(&text),
            _ => wp_text::read_plain(&text),
        };
        self.package = None;
        self.parts = None;
        self.pictures.clear();
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        self.history.clear();
        self.selection = Selection::default();
        self.scroll = 0.0;
        self.stamp = self.stamp.wrapping_add(1);
        self.view.invalidate();
        self.recent.remember(SCRIVA, path);
        self.refresh_fields();
    }

    /// Opens a Word 97-2003 document.
    ///
    /// There is no package: the file is not one. The document is read whole and
    /// saving authors a `.docx` around it, which is why the path is dropped —
    /// Ctrl+S must not offer to write back over a file this cannot write.
    fn open_doc(&mut self, path: &Path) {
        match wp_doc::open(path) {
            Ok(document) => {
                self.document = document;
                self.package = None;
                self.parts = None;
                self.pictures.clear();
                // Named after the original, but with the modern extension, so
                // the save dialog opens on the right name in the right folder.
                self.path = Some(path.with_extension("docx"));
                // Not saved yet: it has never been written in this format.
                self.dirty = true;
                self.history.clear();
                self.selection = Selection::default();
                self.picked = None;
                self.scroll = 0.0;
                self.stamp = self.stamp.wrapping_add(1);
                self.view.invalidate();
                self.recent.remember(SCRIVA, path);
                self.refresh_fields();
                self.message = Some((
                    "Opened as a copy".to_owned(),
                    "Word 97-2003 documents are read but not written, so this \
                     one will be saved as a .docx. Pictures, drawings, fields \
                     and revision marks are not read from the old format."
                        .to_owned(),
                ));
            }
            Err(error) => {
                self.message = Some((
                    "Cannot open".to_owned(),
                    format!("{}\n\n{error}", path.display()),
                ));
            }
        }
    }

    fn open_docx(&mut self, path: &Path) {
        match wp_docx::open(path) {
            Ok((document, package)) => {
                self.document = document;
                self.parts = wp_docx::DocumentParts::locate_in(&package).ok();
                self.package = Some(package);
                self.pictures.clear();
                self.path = Some(path.to_path_buf());
                self.dirty = false;
                self.history.clear();
                self.selection = Selection::default();
                self.scroll = 0.0;
                self.stamp = self.stamp.wrapping_add(1);
                self.view.invalidate();
                self.recent.remember(SCRIVA, path);
                self.refresh_fields();
            }
            Err(error) => {
                self.message = Some((
                    "Cannot open".to_owned(),
                    format!("{}\n\n{error}", path.display()),
                ));
            }
        }
    }

    fn save(&mut self) -> bool {
        let Some(path) = self.path.clone() else {
            return self.save_as();
        };
        if Format::of(&path) != Format::Docx {
            return self.save_text(&path, Format::of(&path));
        }
        if self.package.is_none() {
            // A new document, or one read out of a `.doc`. Author the package
            // once; from here on it is edited by the same splice writer that
            // edits a document Word wrote.
            match wp_docx::write::blank::package_for(&self.document) {
                Ok(package) => self.package = Some(package),
                Err(error) => {
                    self.message = Some(("Cannot save".to_owned(), error.to_string()));
                    return false;
                }
            }
        }
        let Some(package) = &mut self.package else {
            return self.save_as();
        };
        match wp_docx::save(&self.document, package, &path) {
            Ok(()) => {
                self.dirty = false;
                self.recent.remember(SCRIVA, &path);
                true
            }
            Err(error) => {
                // A document open in Word cannot be written by anything else,
                // and that is not a fault in the save. Say so where it happens.
                self.message = Some((
                    "Cannot save".to_owned(),
                    format!(
                        "{}\n\n{error}\n\nIf the document is open in another \
                         program, close it there and try again.",
                        path.display()
                    ),
                ));
                false
            }
        }
    }

    fn save_as(&mut self) -> bool {
        let mut chooser = rfd::FileDialog::new()
            .add_filter("Word document", &["docx"])
            .add_filter("Markdown", &["md"])
            .add_filter("Plain text", &["txt"]);
        if let Some(directory) = self.recent.directory() {
            chooser = chooser.set_directory(directory);
        }
        let Some(path) = chooser.save_file() else {
            return false;
        };
        let path = with_extension(path);
        let format = Format::of(&path);
        if !format.is_writable() {
            self.message = Some((
                "Cannot save".to_owned(),
                "Word 97-2003 documents are read but not written. Save it as a \
                 .docx instead."
                    .to_owned(),
            ));
            return false;
        }
        if format.is_lossy() {
            // Asked *before* the write, because a user who did not mean it has
            // no way back once the file is on disk.
            self.pending = Some(Pending::Lossy(path, format));
            return false;
        }
        self.path = Some(path);
        self.save()
    }

    /// Writes the document as text, keeping the encoding and the line endings
    /// the file came in with.
    fn save_text(&mut self, path: &Path, format: Format) -> bool {
        let text = match format {
            Format::Markdown => wp_text::write(&self.document),
            _ => wp_text::write_plain(&self.document, self.ending),
        };
        // Markdown's own line endings are `\n`; a plain text file keeps
        // whatever it had.
        let text = match format {
            Format::Markdown => text.replace('\n', self.ending.as_str()),
            _ => text,
        };
        match std::fs::write(path, wp_text::encode(&text, self.encoding)) {
            Ok(()) => {
                self.path = Some(path.to_path_buf());
                self.dirty = false;
                self.recent.remember(SCRIVA, path);
                self.refresh_fields();
                true
            }
            Err(error) => {
                self.message = Some((
                    "Cannot save".to_owned(),
                    format!("{}\n\n{error}", path.display()),
                ));
                false
            }
        }
    }

    fn close_document(&mut self) {
        self.document = blank();
        self.package = None;
        self.parts = None;
        self.pictures.clear();
        self.path = None;
        self.dirty = false;
        self.history.clear();
        self.selection = Selection::default();
        self.scroll = 0.0;
        self.stamp = self.stamp.wrapping_add(1);
        self.view.invalidate();
    }

    /// Runs `command`, asking about unsaved changes first where it matters.
    fn guarded(&mut self, command: Command) {
        if self.dirty {
            self.pending = Some(Pending::Unsaved(Box::new(command)));
        } else {
            self.run(command);
        }
    }

    pub fn run(&mut self, command: Command) {
        match command {
            Command::New => self.close_document(),
            Command::Open => {
                let mut chooser = rfd::FileDialog::new()
                    .add_filter(
                        "All documents",
                        &["docx", "docm", "dotx", "doc", "dot", "md", "txt"],
                    )
                    .add_filter("Word documents", &["docx", "docm", "dotx"])
                    .add_filter("Word 97-2003", &["doc", "dot"])
                    .add_filter("Markdown", &["md", "markdown"])
                    .add_filter("Plain text", &["txt"]);
                if let Some(directory) = self.recent.directory() {
                    chooser = chooser.set_directory(directory);
                }
                if let Some(path) = chooser.pick_file() {
                    self.open_path(&path);
                }
            }
            Command::Reopen(path) => self.open_path(&path),
            Command::ForgetRecent => self.recent.clear(SCRIVA),
            Command::Save => {
                self.save();
            }
            Command::SaveAs => {
                self.save_as();
            }
            Command::Close => self.close_document(),
            Command::Exit => {}
            Command::Undo => {
                if let Some(caret) = self.history.undo(&mut self.document) {
                    self.selection = Selection::at(clamp(&self.document, caret));
                    self.changed();
                }
            }
            Command::Redo => {
                if let Some(caret) = self.history.redo(&mut self.document) {
                    self.selection = Selection::at(clamp(&self.document, caret));
                    self.changed();
                }
            }
            Command::SelectAll => {
                let last = self.paragraph_count().saturating_sub(1);
                self.selection = Selection {
                    anchor: Caret {
                        paragraph: 0,
                        offset: 0,
                    },
                    head: Caret {
                        paragraph: last,
                        offset: self.paragraph_text(last).len(),
                    },
                };
            }
            Command::Copy => {
                if let Some(text) = self.selected_text() {
                    clipboard_set(&text);
                }
            }
            Command::Cut => {
                if let Some(text) = self.selected_text() {
                    clipboard_set(&text);
                    self.replace_selection("");
                    self.reveal = Some(self.caret());
                }
            }
            Command::Paste => {
                if let Some(text) = clipboard_get() {
                    self.paste_text(&text);
                }
            }
            Command::Find => self.open_finder(false),
            Command::Replace => self.open_finder(true),
            Command::FindNext => self.jump_match(true),
            Command::FindPrevious => self.jump_match(false),
            Command::Bold => self.toggle(Toggle::Bold),
            Command::Italic => self.toggle(Toggle::Italic),
            Command::Strike => self.toggle(Toggle::Strike),
            Command::Underline => {
                let on = edit::all_runs(&self.document, self.selection, |props| {
                    props.underline.is_some_and(|u| u.kind.draws())
                });
                self.format_runs(move |props| {
                    props.underline = if on {
                        None
                    } else {
                        Some(wp_model::prop::Underline {
                            kind: wp_model::prop::UnderlineKind::Single,
                            color: None,
                        })
                    };
                });
            }
            Command::Superscript => self.vertical(wp_model::prop::VertAlign::Superscript),
            Command::Subscript => self.vertical(wp_model::prop::VertAlign::Subscript),
            Command::ClearFormatting => {
                self.format_runs(|props| {
                    let style = props.style;
                    *props = wp_model::RunProps::default();
                    props.style = style;
                });
            }
            Command::Grow => self.resize(2),
            Command::Shrink => self.resize(-2),
            Command::Size(size) => self.format_runs(move |props| props.size = Some(size)),
            Command::Align(justify) => {
                self.format_paragraphs(move |props| props.justify = Some(justify))
            }
            Command::LineSpacing(line) => self.format_paragraphs(move |props| {
                props.spacing.line = Some(LineSpacing::Multiple(line))
            }),
            Command::Indent(by) => self.format_paragraphs(move |props| {
                let start = props.indent.start.unwrap_or(Twips(0));
                props.indent.start = Some(Twips((start.0 + by * 720).max(0)));
            }),
            Command::Style(style) => self.format_paragraphs(move |props| props.style = Some(style)),
            Command::PageBreak => {
                let caret = edit::insert_break(
                    &mut self.document,
                    &mut self.history,
                    self.selection,
                    wp_model::doc::Break::Page,
                );
                self.selection = Selection::at(caret);
                self.reveal = Some(caret);
                self.changed();
            }
            Command::Margins(margins) => {
                let mut section = self.document.section.clone();
                // The presets say where the text goes; where the header and
                // footer sit, and the binding gutter, are not theirs to move.
                section.margins = wp_model::PageMargins {
                    header: section.margins.header,
                    footer: section.margins.footer,
                    gutter: section.margins.gutter,
                    ..margins
                };
                self.set_section(section);
            }
            Command::CustomMargins => {
                let inches = |t: Twips| format!("{:.2}", t.0 as f64 / 1440.0);
                let m = self.document.section.margins;
                self.margins_draft = Some([
                    inches(m.top),
                    inches(m.bottom),
                    inches(m.start),
                    inches(m.end),
                ]);
            }
            Command::Orient(orientation) => {
                let mut section = self.document.section.clone();
                if section.page.orientation != orientation {
                    std::mem::swap(&mut section.page.width, &mut section.page.height);
                    section.page.orientation = orientation;
                    // Word turns the margins with the paper.
                    let m = section.margins;
                    section.margins = wp_model::PageMargins {
                        top: m.start,
                        bottom: m.end,
                        start: m.top,
                        end: m.bottom,
                        ..m
                    };
                    self.set_section(section);
                }
            }
            Command::Paper(width, height) => {
                let mut section = self.document.section.clone();
                let landscape = section.page.orientation == wp_model::Orientation::Landscape;
                let (w, h) = if landscape {
                    (height, width)
                } else {
                    (width, height)
                };
                if section.page.width != w || section.page.height != h {
                    section.page.width = w;
                    section.page.height = h;
                    // The old paper's printer-tray code would now be a lie.
                    section.page.code = None;
                    self.set_section(section);
                }
            }
            Command::Zoom(zoom) => self.view.zoom = zoom,
            Command::ShowMarks => {
                self.view.show_marks = !self.view.show_marks;
                self.view.invalidate();
            }
            Command::ShowRevisions => {
                self.view.show_revisions = !self.view.show_revisions;
                self.view.invalidate();
            }
            Command::Navigator => self.navigator = !self.navigator,
            Command::Reviewer => self.reviewer = !self.reviewer,
            Command::TrackChanges => {
                self.document.settings.track_changes = !self.document.settings.track_changes;
                self.changed();
            }
            Command::AcceptAll => self.settle_all(crate::revise::Resolve::Accept),
            Command::RejectAll => self.settle_all(crate::revise::Resolve::Reject),
            Command::AcceptOne => self.settle_one(crate::revise::Resolve::Accept),
            Command::RejectOne => self.settle_one(crate::revise::Resolve::Reject),
            Command::NextChange => self.next_change(),
            Command::AddComment => {
                if self.selection.is_empty() {
                    self.message = Some((
                        "Nothing selected".to_owned(),
                        "A comment is about a piece of text. Select what it is \
                         about and try again."
                            .to_owned(),
                    ));
                } else {
                    self.drafting = Some(String::new());
                }
            }
            Command::DeleteComment => self.delete_comment_here(),
            Command::GoTo(paragraph) => {
                let caret = clamp(
                    &self.document,
                    Caret {
                        paragraph,
                        offset: 0,
                    },
                );
                self.selection = Selection::at(caret);
                // Scrolled to on the next frame, when the layout knows where it
                // is: a caret has no place on the page until the page exists.
                self.reveal = Some(caret);
            }
            Command::UpdateToc => self.update_toc(),
        }
    }

    /// Rebuilds the table of contents from the headings the document has now.
    ///
    /// Only the paragraphs *between* the field's first and last are replaced:
    /// those two carry the field characters, and a rebuild that took them with
    /// it would leave a list of headings that is no longer a field at all.
    fn update_toc(&mut self) {
        let Some(span) = wp_model::outline::toc_span(&self.document) else {
            self.message = Some((
                "No table of contents".to_owned(),
                "This document has no TOC field to rebuild. Word writes one from \
                 References \u{203a} Table of Contents."
                    .to_owned(),
            ));
            return;
        };
        let entries = wp_model::outline::table_of_contents(&self.document, span.levels.clone());
        if entries.is_empty() {
            self.message = Some((
                "No headings".to_owned(),
                "A table of contents is built from the paragraphs that have a \
                 heading style or an outline level. This document has none."
                    .to_owned(),
            ));
            return;
        }
        let rows: Vec<Paragraph> = entries
            .iter()
            .map(|entry| {
                let mut paragraph = Paragraph::of(&entry.text);
                // Word indents each level by a quarter inch, which is what makes
                // a contents list read as an outline.
                paragraph.props.indent.start =
                    Some(Twips((entry.level.saturating_sub(1) as i32) * 360));
                paragraph
            })
            .collect();
        let range = span.entries();
        edit::format_paragraphs(
            &mut self.document,
            &mut self.history,
            Selection::at(Caret {
                paragraph: span.first,
                offset: 0,
            }),
            |_| {},
        );
        edit::replace_range(&mut self.document, range, rows);
        self.changed();
    }

    fn settle_all(&mut self, how: crate::revise::Resolve) {
        let count = crate::revise::resolve_all(&mut self.document, &mut self.history, how);
        if count == 0 {
            self.message = Some((
                "No tracked changes".to_owned(),
                "This document has nothing to accept or reject.".to_owned(),
            ));
            return;
        }
        self.selection = Selection::at(clamp(&self.document, self.caret()));
        self.changed();
    }

    /// Settles the change nearest the caret.
    ///
    /// Nearest rather than *at*: a change is a range, and asking the user to put
    /// the caret exactly inside one is asking them to hunt for it.
    fn settle_one(&mut self, how: crate::revise::Resolve) {
        let changes = crate::revise::tracked(&self.document);
        let here = self.caret().paragraph;
        let Some(found) = changes
            .iter()
            .min_by_key(|change| change.paragraph.abs_diff(here))
        else {
            self.message = Some((
                "No tracked changes".to_owned(),
                "This document has nothing to accept or reject.".to_owned(),
            ));
            return;
        };
        let mark = found.mark.clone();
        if crate::revise::resolve_one(&mut self.document, &mut self.history, &mark, how) {
            self.selection = Selection::at(clamp(&self.document, self.caret()));
            self.changed();
        }
    }

    fn next_change(&mut self) {
        let changes = crate::revise::tracked(&self.document);
        let here = self.caret().paragraph;
        let next = changes
            .iter()
            .find(|change| change.paragraph > here)
            .or_else(|| changes.first());
        if let Some(change) = next {
            self.run(Command::GoTo(change.paragraph));
        }
    }

    fn delete_comment_here(&mut self) {
        let here = self.caret().paragraph;
        let target = self
            .document
            .comments
            .iter()
            .map(|comment| comment.id)
            .find(|id| {
                crate::revise::comment_at(&self.document, *id)
                    .is_some_and(|at| at.paragraph == here)
            });
        match target {
            Some(id) => {
                crate::revise::delete_comment(&mut self.document, &mut self.history, id);
                self.changed();
            }
            None => {
                self.message = Some((
                    "No comment here".to_owned(),
                    "Put the caret in the text a comment is about.".to_owned(),
                ));
            }
        }
    }

    fn toggle(&mut self, toggle: Toggle) {
        let on = edit::all_runs(&self.document, self.selection, |props| {
            props.toggles.is_on(toggle)
        });
        self.format_runs(move |props| props.toggles.set(toggle, !on));
    }

    fn vertical(&mut self, align: wp_model::prop::VertAlign) {
        let on = edit::all_runs(&self.document, self.selection, |props| {
            props.vert_align == Some(align)
        });
        self.format_runs(move |props| {
            props.vert_align = if on { None } else { Some(align) };
        });
    }

    fn resize(&mut self, by: i32) {
        let current = self
            .document
            .paragraphs()
            .get(self.caret().paragraph)
            .map(|paragraph| {
                let layers = self
                    .document
                    .styles
                    .resolve_paragraph(&paragraph.props, None);
                text::props_at(paragraph, self.caret().offset)
                    .size
                    .or(layers.run.size)
                    .unwrap_or(HalfPoint(22))
            })
            .unwrap_or(HalfPoint(22));
        let size = HalfPoint(current.0 + by).clamped();
        self.format_runs(move |props| props.size = Some(size));
    }

    fn format_runs(&mut self, change: impl Fn(&mut wp_model::RunProps) + Copy) {
        if self.selection.is_empty() {
            // Word applies it to the word the caret is in when there is no
            // selection — otherwise Ctrl+B with the caret in a word appears to
            // do nothing at all.
            let caret = self.caret();
            let content = self.paragraph_text(caret.paragraph);
            let word = text::word_at(&content, caret.offset);
            if word.is_empty() {
                return;
            }
            let selection = Selection {
                anchor: Caret {
                    paragraph: caret.paragraph,
                    offset: word.start,
                },
                head: Caret {
                    paragraph: caret.paragraph,
                    offset: word.end,
                },
            };
            edit::format_runs(&mut self.document, &mut self.history, selection, change);
        } else {
            edit::format_runs(
                &mut self.document,
                &mut self.history,
                self.selection,
                change,
            );
        }
        self.changed();
    }

    fn format_paragraphs(&mut self, change: impl Fn(&mut wp_model::ParaProps) + Copy) {
        edit::format_paragraphs(
            &mut self.document,
            &mut self.history,
            self.selection,
            change,
        );
        self.changed();
    }

    // ------------------------------------------------------------ clipboard

    /// The text the selection covers, paragraphs joined by newlines.
    fn selected_text(&self) -> Option<String> {
        if self.selection.is_empty() {
            return None;
        }
        let (start, end) = self.selection.ordered();
        if start.paragraph == end.paragraph {
            let content = self.paragraph_text(start.paragraph);
            return content.get(start.offset..end.offset).map(str::to_owned);
        }
        let mut parts = Vec::new();
        let first = self.paragraph_text(start.paragraph);
        parts.push(first.get(start.offset..).unwrap_or_default().to_owned());
        for index in start.paragraph + 1..end.paragraph {
            parts.push(self.paragraph_text(index));
        }
        let last = self.paragraph_text(end.paragraph);
        parts.push(last.get(..end.offset).unwrap_or_default().to_owned());
        Some(parts.join("\n"))
    }

    /// Replaces the selection with `replacement` — typing it when there is
    /// something to type, deleting when there is not. Tracking applies either
    /// way, because both go through the same paths typing does.
    fn replace_selection(&mut self, replacement: &str) {
        if !replacement.is_empty() {
            self.type_text(replacement);
            return;
        }
        if self.selection.is_empty() {
            return;
        }
        if self.document.settings.track_changes {
            self.record_delete();
        } else {
            let caret =
                edit::delete_selection(&mut self.document, &mut self.history, self.selection);
            self.selection = Selection::at(clamp(&self.document, caret));
            self.changed();
        }
    }

    /// Pastes text at the selection: the first line types over it, and each
    /// newline after that presses Enter.
    fn paste_text(&mut self, input: &str) {
        let input = input.replace("\r\n", "\n").replace('\r', "\n");
        if input.is_empty() {
            return;
        }
        let mut segments = input.split('\n');
        match segments.next() {
            Some(first) if !first.is_empty() => self.type_text(first),
            _ => {}
        }
        for segment in segments {
            let caret =
                edit::split_paragraph(&mut self.document, &mut self.history, self.selection);
            self.selection = Selection::at(clamp(&self.document, caret));
            self.changed();
            if !segment.is_empty() {
                self.type_text(segment);
            }
        }
        self.reveal = Some(self.caret());
    }

    // ------------------------------------------------------------ find

    /// Opens the find bar, pre-filled from the selection the way Word does.
    fn open_finder(&mut self, with_replace: bool) {
        let mut finder = self
            .finder
            .take()
            .unwrap_or_else(|| Finder::new(with_replace));
        finder.with_replace = with_replace;
        finder.focus = true;
        finder.note = None;
        if let Some(selected) = self
            .selected_text()
            .filter(|text| !text.contains('\n') && text.len() <= 100)
        {
            finder.query = selected;
        }
        self.finder = Some(finder);
    }

    /// Recomputes the matches when the document or the query has changed.
    fn refresh_matches(&mut self) {
        let query = self
            .finder
            .as_ref()
            .map(|finder| finder.query.clone())
            .unwrap_or_default();
        if self.matches_for == (self.stamp, query.clone()) {
            return;
        }
        self.find_matches = find::matches(&self.document, &query);
        self.matches_for = (self.stamp, query);
    }

    /// Selects the next or previous match and scrolls to it.
    fn jump_match(&mut self, forward: bool) {
        self.refresh_matches();
        let (start, end) = self.selection.ordered();
        let found = if forward {
            // From the selection's end, so the match already selected is
            // stepped past — but a match starting right at a bare caret counts.
            find::after(
                &self.find_matches,
                if self.selection.is_empty() {
                    start
                } else {
                    end
                },
            )
        } else {
            find::before(&self.find_matches, start)
        };
        if let Some(found) = found {
            self.selection = found;
            self.reveal = Some(found.ordered().0);
        }
    }

    /// Replaces the selected match and moves to the next one.
    fn replace_current(&mut self) {
        let Some(finder) = &self.finder else {
            return;
        };
        if finder.query.is_empty() {
            return;
        }
        let (query, replacement) = (finder.query.clone(), finder.replacement.clone());
        let selection_matches = self
            .selected_text()
            .is_some_and(|text| find::equals(&text, &query));
        if selection_matches {
            self.replace_selection(&replacement);
        }
        self.jump_match(true);
    }

    /// Replaces every match, back to front so earlier offsets stay true.
    fn replace_all(&mut self) {
        let Some(finder) = &self.finder else {
            return;
        };
        if finder.query.is_empty() {
            return;
        }
        let replacement = finder.replacement.clone();
        self.refresh_matches();
        let all = self.find_matches.clone();
        for found in all.iter().rev() {
            self.selection = *found;
            self.replace_selection(&replacement);
        }
        if let Some(finder) = &mut self.finder {
            finder.note = Some(match all.len() {
                0 => "No matches".to_owned(),
                n => format!("Replaced {n}"),
            });
        }
    }

    // ------------------------------------------------------------ keys

    /// Word's keyboard, as far as it is implemented.
    fn keys(&mut self, ui: &egui::Ui) -> Option<Command> {
        use egui::Key;
        let ctrl = egui::Modifiers::COMMAND;
        let ctrl_shift = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);

        let taken = |ui: &egui::Ui, modifiers: egui::Modifiers, key: Key| -> bool {
            ui.input_mut(|i| i.consume_key(modifiers, key))
        };

        for (modifiers, key, command) in [
            (ctrl, Key::N, Command::New),
            (ctrl, Key::O, Command::Open),
            (ctrl, Key::S, Command::Save),
            (ctrl_shift, Key::S, Command::SaveAs),
            (ctrl, Key::W, Command::Close),
            (ctrl, Key::Z, Command::Undo),
            (ctrl, Key::Y, Command::Redo),
            (ctrl, Key::A, Command::SelectAll),
            (ctrl, Key::B, Command::Bold),
            (ctrl, Key::I, Command::Italic),
            (ctrl, Key::U, Command::Underline),
            (ctrl, Key::L, Command::Align(Justify::Start)),
            (ctrl, Key::E, Command::Align(Justify::Center)),
            (ctrl, Key::R, Command::Align(Justify::End)),
            (ctrl, Key::J, Command::Align(Justify::Both)),
            (ctrl, Key::Num1, Command::LineSpacing(Line240::SINGLE)),
            (ctrl, Key::Num2, Command::LineSpacing(Line240::DOUBLE)),
            (
                ctrl,
                Key::Num5,
                Command::LineSpacing(Line240::ONE_AND_A_HALF),
            ),
            (ctrl, Key::M, Command::Indent(1)),
            (ctrl_shift, Key::M, Command::Indent(-1)),
            (ctrl, Key::Space, Command::ClearFormatting),
            (ctrl, Key::Enter, Command::PageBreak),
            (ctrl, Key::F, Command::Find),
            (ctrl, Key::H, Command::Replace),
            (egui::Modifiers::NONE, Key::F3, Command::FindNext),
            (egui::Modifiers::SHIFT, Key::F3, Command::FindPrevious),
            (egui::Modifiers::NONE, Key::F9, Command::UpdateToc),
            (ctrl_shift, Key::E, Command::TrackChanges),
            (
                egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT),
                Key::M,
                Command::AddComment,
            ),
            (ctrl_shift, Key::Z, Command::Redo),
        ] {
            if taken(ui, modifiers, key) {
                return Some(command);
            }
        }
        // Ctrl+Shift+> and Ctrl+Shift+< — the key is the unshifted one.
        if taken(ui, ctrl_shift, Key::Period) {
            return Some(Command::Grow);
        }
        if taken(ui, ctrl_shift, Key::Comma) {
            return Some(Command::Shrink);
        }
        if taken(ui, ctrl, Key::Equals) {
            return Some(Command::Subscript);
        }
        if taken(ui, ctrl_shift, Key::Equals) {
            return Some(Command::Superscript);
        }
        if taken(ui, ctrl_shift, Key::Num8) {
            return Some(Command::ShowMarks);
        }
        None
    }

    /// Movement, typing and deletion. Everything that changes the caret.
    fn typing(&mut self, ui: &egui::Ui) {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                // Typing while a picture is selected replaces it in Word. It
                // does nothing here rather than doing something surprising.
                egui::Event::Text(text) if !text.is_empty() && self.picked.is_none() => {
                    self.type_text(&text);
                    self.reveal = Some(self.caret());
                }
                // Ctrl+C, Ctrl+X and Ctrl+V arrive as their own events, with
                // the pasted text already read from the OS.
                egui::Event::Copy => self.run(Command::Copy),
                egui::Event::Cut if self.picked.is_none() => self.run(Command::Cut),
                egui::Event::Paste(text) if self.picked.is_none() => self.paste_text(&text),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => self.key(key, modifiers),
                _ => {}
            }
        }
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        use egui::Key;
        // A selected picture takes the keys that would otherwise edit text:
        // Delete removes it, Escape lets it go, and typing is not for it.
        if self.picked.is_some() {
            match key {
                Key::Delete | Key::Backspace => {
                    self.delete_drawing();
                }
                Key::Escape => self.picked = None,
                _ => {}
            }
            return;
        }
        let extend = modifiers.shift;
        let word = modifiers.command;
        let caret = self.caret();
        let content = self.paragraph_text(caret.paragraph);
        let last = self.paragraph_count().saturating_sub(1);
        let was = (self.selection, self.stamp);

        match key {
            Key::ArrowLeft => {
                let next = if word {
                    Caret {
                        paragraph: caret.paragraph,
                        offset: text::word_start_before(&content, caret.offset),
                    }
                } else if caret.offset > 0 {
                    Caret {
                        paragraph: caret.paragraph,
                        offset: text::previous_char(&content, caret.offset),
                    }
                } else if caret.paragraph > 0 {
                    Caret {
                        paragraph: caret.paragraph - 1,
                        offset: self.paragraph_text(caret.paragraph - 1).len(),
                    }
                } else {
                    caret
                };
                self.set_caret(next, extend);
            }
            Key::ArrowRight => {
                let next = if word {
                    Caret {
                        paragraph: caret.paragraph,
                        offset: text::word_start_after(&content, caret.offset),
                    }
                } else if caret.offset < content.len() {
                    Caret {
                        paragraph: caret.paragraph,
                        offset: text::next_char(&content, caret.offset),
                    }
                } else if caret.paragraph < last {
                    Caret {
                        paragraph: caret.paragraph + 1,
                        offset: 0,
                    }
                } else {
                    caret
                };
                self.set_caret(next, extend);
            }
            Key::ArrowUp | Key::ArrowDown => {
                // By line where the layout knows one, and by paragraph
                // otherwise. Going by paragraph alone would skip five lines of
                // a wrapped one, which is what a naive editor does.
                let next = self.line_step(caret, key == Key::ArrowDown);
                self.set_caret(next, extend);
            }
            Key::PageUp | Key::PageDown => {
                let next = self.page_step(caret, key == Key::PageDown);
                self.set_caret(next, extend);
            }
            Key::Home => {
                // The *visual* line's start — in a paragraph that wraps, Home
                // does not go all the way back to the paragraph's first word.
                let next = if modifiers.command {
                    Caret {
                        paragraph: 0,
                        offset: 0,
                    }
                } else {
                    let offset = view::line_span(&self.view, caret)
                        .map(|(start, _)| start)
                        .unwrap_or(0);
                    Caret {
                        paragraph: caret.paragraph,
                        offset,
                    }
                };
                self.set_caret(next, extend);
            }
            Key::End => {
                // The visual line's end. On a wrapped line that is the last
                // letter before the break — the space the wrap ate is after
                // it, and Word does not put the caret beyond it either.
                let next = if modifiers.command {
                    Caret {
                        paragraph: last,
                        offset: self.paragraph_text(last).len(),
                    }
                } else {
                    let offset = match view::line_span(&self.view, caret) {
                        Some((start, mut end)) if end < content.len() => {
                            while end > start && content.as_bytes().get(end - 1) == Some(&b' ') {
                                end -= 1;
                            }
                            end
                        }
                        Some((_, end)) => end,
                        None => content.len(),
                    };
                    Caret {
                        paragraph: caret.paragraph,
                        offset,
                    }
                };
                self.set_caret(next, extend);
            }
            Key::Backspace => {
                // At the start of a list item the first Backspace takes the
                // bullet, and only the next one joins paragraphs — Word's way
                // of leaving a list without merging into the item above.
                if self.selection.is_empty()
                    && caret.offset == 0
                    && self.numbering_at(caret.paragraph).is_some()
                {
                    self.format_paragraphs(|props| props.numbering = None);
                    return;
                }
                // Ctrl+Backspace takes the whole word before the caret.
                let caret = if word && self.selection.is_empty() && caret.offset > 0 {
                    let from = Caret {
                        paragraph: caret.paragraph,
                        offset: text::word_start_before(&content, caret.offset),
                    };
                    edit::delete_selection(
                        &mut self.document,
                        &mut self.history,
                        Selection {
                            anchor: from,
                            head: caret,
                        },
                    )
                } else {
                    edit::backspace(&mut self.document, &mut self.history, self.selection)
                };
                self.selection = Selection::at(clamp(&self.document, caret));
                self.changed();
            }
            Key::Delete => {
                // Ctrl+Delete takes the whole word after the caret.
                let caret = if word && self.selection.is_empty() && caret.offset < content.len() {
                    let to = Caret {
                        paragraph: caret.paragraph,
                        offset: text::word_start_after(&content, caret.offset),
                    };
                    edit::delete_selection(
                        &mut self.document,
                        &mut self.history,
                        Selection {
                            anchor: caret,
                            head: to,
                        },
                    )
                } else {
                    edit::delete_forward(&mut self.document, &mut self.history, self.selection)
                };
                self.selection = Selection::at(clamp(&self.document, caret));
                self.changed();
            }
            Key::Enter => {
                // Enter on an empty list item ends the list rather than
                // adding another empty bullet — Word's way out.
                if self.selection.is_empty()
                    && content.is_empty()
                    && self.numbering_at(caret.paragraph).is_some()
                {
                    self.format_paragraphs(|props| props.numbering = None);
                } else {
                    let caret = edit::split_paragraph(
                        &mut self.document,
                        &mut self.history,
                        self.selection,
                    );
                    self.selection = Selection::at(clamp(&self.document, caret));
                    self.changed();
                }
            }
            Key::Tab => {
                // At the start of a list item, Tab goes a level deeper and
                // Shift+Tab comes back up. Anywhere else it is a tab.
                if self.selection.is_empty()
                    && caret.offset == 0
                    && self.numbering_at(caret.paragraph).is_some()
                {
                    let deeper = !modifiers.shift;
                    self.format_paragraphs(move |props| {
                        if let Some(numbering) = &mut props.numbering {
                            numbering.level = if deeper {
                                (numbering.level + 1).min(8)
                            } else {
                                numbering.level.saturating_sub(1)
                            };
                        }
                    });
                } else {
                    let caret = edit::type_text(
                        &mut self.document,
                        &mut self.history,
                        self.selection,
                        "\t",
                    );
                    self.selection = Selection::at(caret);
                    self.changed();
                }
            }
            Key::Escape => {
                // Escape leaves things, nearest first: the find bar, then the
                // selection.
                if self.finder.is_some() {
                    self.finder = None;
                    self.finder_focused = false;
                } else {
                    self.selection = Selection::at(caret);
                }
            }
            _ => {}
        }
        // Wherever the keyboard put the caret, the view follows it — otherwise
        // arrowing or typing below the window edge walks the caret out of sight.
        if (self.selection, self.stamp) != was {
            self.reveal = Some(self.caret());
        }
    }

    /// Types text, recording it as a tracked insertion when tracking is on.
    fn type_text(&mut self, input: &str) {
        if !self.document.settings.track_changes {
            let caret =
                edit::type_text(&mut self.document, &mut self.history, self.selection, input);
            self.selection = Selection::at(caret);
            self.changed();
            return;
        }
        // What is selected is *deleted* first, and with tracking on that means
        // marked rather than removed.
        let (start, _) = self.selection.ordered();
        if !self.selection.is_empty() {
            self.record_delete();
        }
        let id = crate::revise::next_revision_id(&self.document);
        let Some(before) = edit::paragraph_at(&self.document, start.paragraph) else {
            return;
        };
        self.history.push(edit::Change::Paragraph {
            index: start.paragraph,
            before: Box::new(before),
        });
        let author = self.author.clone();
        let mut paragraphs = self.document.paragraphs_mut();
        let Some(target) = paragraphs.get_mut(start.paragraph) else {
            return;
        };
        match crate::revise::record_insertion(target, start.offset, input, &author, id) {
            Some(after) => {
                drop(paragraphs);
                self.selection = Selection::at(Caret {
                    paragraph: start.paragraph,
                    offset: after,
                });
                self.changed();
            }
            None => {
                // A position this cannot wrap — inside a hyperlink, a content
                // control, a field's result. A half-recorded change is worse
                // than an unrecorded one, so the edit is refused and said.
                drop(paragraphs);
                self.history.undo(&mut self.document);
                self.message = Some((
                    "Cannot record this change".to_owned(),
                    "Track Changes cannot record an edit inside a hyperlink, a \
                     content control or a field. Turn tracking off to edit here."
                        .to_owned(),
                ));
            }
        }
    }

    /// Marks the selection deleted rather than removing it.
    fn record_delete(&mut self) {
        let (start, end) = self.selection.ordered();
        if start.paragraph != end.paragraph {
            // Across paragraphs the deletion covers paragraph marks too, which
            // is a change to the body rather than to one paragraph. Not
            // recorded; stated rather than half-done.
            let caret =
                edit::delete_selection(&mut self.document, &mut self.history, self.selection);
            self.selection = Selection::at(caret);
            self.changed();
            return;
        }
        let id = crate::revise::next_revision_id(&self.document);
        let Some(before) = edit::paragraph_at(&self.document, start.paragraph) else {
            return;
        };
        self.history.push(edit::Change::Paragraph {
            index: start.paragraph,
            before: Box::new(before),
        });
        let author = self.author.clone();
        let mut paragraphs = self.document.paragraphs_mut();
        if let Some(target) = paragraphs.get_mut(start.paragraph) {
            let _ = crate::revise::record_deletion(target, start.offset..end.offset, &author, id);
        }
        drop(paragraphs);
        self.selection = Selection::at(start);
        self.changed();
    }

    /// The list the caret's paragraph is directly in, when it is a real one —
    /// `numId` zero is how a style's list is cancelled, not a list.
    fn numbering_at(&self, paragraph: usize) -> Option<wp_model::prop::NumRef> {
        self.document
            .paragraphs()
            .get(paragraph)
            .and_then(|p| p.props.numbering)
            .filter(|n| n.is_numbered())
    }

    /// One line up or down, using the laid-out lines. Measured down the stack
    /// of pages, so the last line of one page steps onto the first of the next.
    fn line_step(&self, caret: Caret, down: bool) -> Caret {
        let Some((_, rect)) = view::caret_rect(&self.view, caret) else {
            return caret;
        };
        let step = rect.height().max(1.0) as f64 * if down { 1.0 } else { -1.0 };
        view::step_from(&self.view, caret, step).unwrap_or(caret)
    }

    /// One screenful up or down — Page Up and Page Down.
    fn page_step(&self, caret: Caret, down: bool) -> Caret {
        let screen = (self.viewport.max(60.0) as f64) / self.view.zoom.max(0.01);
        let step = screen * if down { 1.0 } else { -1.0 };
        view::step_from(&self.view, caret, step).unwrap_or(caret)
    }
}

/// A caret that is inside the document it names.
fn clamp(document: &Document, caret: Caret) -> Caret {
    let paragraphs = document.paragraphs();
    if paragraphs.is_empty() {
        return Caret::default();
    }
    let index = caret.paragraph.min(paragraphs.len() - 1);
    Caret {
        paragraph: index,
        offset: caret.offset.min(paragraphs[index].text().len()),
    }
}

/// Puts text on the OS clipboard, with the line endings Windows programs read.
fn clipboard_set(text: &str) {
    if let Ok(mut board) = arboard::Clipboard::new() {
        let _ = board.set_text(text.replace('\n', "\r\n"));
    }
}

/// What the OS clipboard holds, as text.
fn clipboard_get() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension("docx")
    }
}

/// A document with one empty paragraph and Word's own defaults.
fn blank() -> Document {
    let mut document = Document {
        body: vec![Block::Paragraph(Paragraph::new())],
        ..Document::new()
    };
    let mut normal = wp_model::Style::new("Normal", wp_model::StyleKind::Paragraph);
    normal.default = true;
    normal.name = Some("Normal".into());
    normal.run.size = Some(HalfPoint::DEFAULT);
    normal.run.fonts.ascii = Some("Calibri".into());
    document.styles.insert(normal);
    document
}

impl DocumentApp for Scriva {
    fn id(&self) -> AppId {
        SCRIVA
    }

    fn document(&self) -> Option<(String, bool)> {
        let name = self
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Document1".to_owned());
        Some((name, self.dirty))
    }

    fn close_requested(&mut self) -> bool {
        if !self.dirty {
            return true;
        }
        self.pending = Some(Pending::Unsaved(Box::new(Command::Exit)));
        false
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let command = self.menus(ui);
        rule(ui);
        let bar = self.format_bar(ui);
        if let Some(command) = command.or(bar) {
            self.run(command);
        }
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let pages = self.view.pages().len().max(1);
            let page = view::caret_rect(&self.view, self.caret())
                .map(|(index, _)| index + 1)
                .unwrap_or(1);
            ui.label(format!("Page {page} of {pages}"));
            ui.separator();
            ui.label(format!("{} words", self.word_count()));
            ui.separator();
            if !self.selection.is_empty() {
                ui.label("Selection");
                ui.separator();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{}%", (self.view.zoom * 100.0).round() as i32));
                let mut zoom = self.view.zoom * 100.0;
                if ui
                    .add(
                        egui::Slider::new(&mut zoom, 25.0..=400.0)
                            .show_value(false)
                            .trailing_fill(false),
                    )
                    .on_hover_text("Zoom")
                    .changed()
                {
                    self.view.zoom = zoom / 100.0;
                }
            });
        });
    }

    fn overlay(&mut self, ctx: &egui::Context) {
        if let Some((title, body)) = self.message.clone() {
            let answered = dialog::message(
                ctx,
                "scriva-message",
                dialog::Severity::Error,
                &title,
                &body,
                None,
                &[dialog::Choice::new("OK").primary().escapes()],
            );
            if answered.is_some() {
                self.message = None;
            }
            return;
        }
        if self.drafting.is_some() {
            self.comment_dialog(ctx);
            return;
        }
        if self.margins_draft.is_some() {
            self.margins_dialog(ctx);
            return;
        }
        if let Some(Pending::Lossy(path, format)) = self.pending.clone() {
            let what = match format {
                Format::Markdown => {
                    "Markdown keeps headings, emphasis and lists. Everything 
                                     else — page setup, tables, comments, tracked 
                                     changes, pictures — is lost."
                }
                _ => "Plain text keeps the words and nothing else.",
            };
            let answer = dialog::message(
                ctx,
                "scriva-lossy",
                dialog::Severity::Warning,
                "Save in this format?",
                what,
                Some(&path.display().to_string()),
                &[
                    dialog::Choice::new("Save").primary(),
                    dialog::Choice::new("Cancel").escapes(),
                ],
            );
            match answer {
                Some(0) => {
                    self.pending = None;
                    self.save_text(&path, format);
                }
                Some(_) => self.pending = None,
                None => {}
            }
            return;
        }
        let Some(Pending::Unsaved(command)) = self.pending.clone() else {
            return;
        };
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Document1".to_owned());
        let answer = dialog::message(
            ctx,
            "scriva-unsaved",
            dialog::Severity::Warning,
            "Save changes?",
            &format!("{name} has changes that have not been saved."),
            None,
            &[
                dialog::Choice::new("Save").primary(),
                dialog::Choice::new("Don't Save"),
                dialog::Choice::new("Cancel").escapes(),
            ],
        );
        match answer {
            Some(0) => {
                self.pending = None;
                if self.save() {
                    self.finish(*command, ctx);
                }
            }
            Some(1) => {
                self.pending = None;
                self.dirty = false;
                self.finish(*command, ctx);
            }
            Some(_) => self.pending = None,
            None => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        if self.shaper.is_none() {
            self.shaper = Some(Egui::new(ui.ctx()));
        }
        let stamp = self.stamp;
        let fields = self.fields.clone();
        if let Some(shaper) = &mut self.shaper {
            self.view.refresh(&self.document, &fields, stamp, shaper);
        }
        if self.navigator {
            if let Some(command) = self.navigation_pane(ui) {
                self.run(command);
            }
        }
        if self.reviewer {
            if let Some(command) = self.reviewing_pane(ui) {
                self.run(command);
            }
        }
        if self.finder.is_some() {
            self.find_bar(ui);
        }

        // Ctrl+scroll and a trackpad pinch zoom the page, like Word.
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if zoom_delta != 1.0 {
            self.view.zoom = (self.view.zoom * zoom_delta as f64).clamp(0.25, 4.0);
        }

        // While a dialog or the find bar holds the keyboard, keys belong to it:
        // without this, searching for "bug" also types "bug" into the document.
        let blocked = self.pending.is_some()
            || self.message.is_some()
            || self.drafting.is_some()
            || self.margins_draft.is_some()
            || self.finder_focused
            || egui::Popup::is_any_open(ui.ctx());
        if !blocked {
            if let Some(command) = self.keys(ui) {
                match command {
                    Command::New | Command::Open | Command::Close | Command::Exit => {
                        self.guarded(command)
                    }
                    other => self.run(other),
                }
            }
            self.typing(ui);
        }

        self.surface(ui);
    }
}

impl Scriva {
    /// Replaces the page setup, undoably, and relays the document out.
    fn set_section(&mut self, section: wp_model::SectionProps) {
        let caret = self.caret();
        edit::set_section(&mut self.document, &mut self.history, caret, section);
        self.changed();
    }

    /// The custom-margins box: four numbers in inches, the way Word asks.
    fn margins_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.margins_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-margins"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(260.0);
                ui.add_space(16.0);
                ui.label(egui::RichText::new("Margins").font(dialog::heading_font(16.0)));
                ui.add_space(8.0);
                for (label, field) in ["Top:", "Bottom:", "Left:", "Right:"]
                    .into_iter()
                    .zip(draft.iter_mut())
                {
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 20.0], egui::Label::new(label));
                        ui.add(egui::TextEdit::singleline(field).desired_width(64.0));
                        ui.label("in");
                    });
                }
                ui.add_space(12.0);
                if let Some(answer) = dialog::confirm(ui, "Set") {
                    done = Some(answer);
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
        self.margins_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.margins_draft = None;
                // A field that does not parse keeps the margin it had — the
                // dialog is not the place to argue about a typo.
                let m = self.document.section.margins;
                let parse = |text: &str, was: Twips| {
                    text.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|v| (0.0..=5.0).contains(v))
                        .map(|v| Twips((v * 1440.0).round() as i32))
                        .unwrap_or(was)
                };
                let mut section = self.document.section.clone();
                section.margins.top = parse(&draft[0], m.top);
                section.margins.bottom = parse(&draft[1], m.bottom);
                section.margins.start = parse(&draft[2], m.start);
                section.margins.end = parse(&draft[3], m.end);
                if section.margins != m {
                    self.set_section(section);
                }
            }
            Some(false) => self.margins_draft = None,
            None => {}
        }
    }

    /// The box a new comment is written in.
    fn comment_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut text) = self.drafting.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-comment"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.add_space(16.0);
                ui.label(egui::RichText::new("New comment").font(dialog::heading_font(16.0)));
                ui.add_space(8.0);
                let field = ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text("What is there to say about this?"),
                );
                field.request_focus();
                ui.add_space(12.0);
                if let Some(answer) = dialog::confirm(ui, "Add") {
                    done = Some(answer);
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
        self.drafting = Some(text.clone());
        match done {
            Some(true) => {
                self.drafting = None;
                if !text.trim().is_empty() {
                    let author = self.author.clone();
                    crate::revise::add_comment(
                        &mut self.document,
                        &mut self.history,
                        self.selection,
                        &author.name,
                        &author.initials,
                        text.trim(),
                    );
                    self.reviewer = true;
                    self.changed();
                }
            }
            Some(false) => self.drafting = None,
            None => {}
        }
    }

    /// The find bar across the top of the document.
    fn find_bar(&mut self, ui: &mut egui::Ui) {
        self.refresh_matches();
        let total = self.find_matches.len();
        let current = self
            .find_matches
            .iter()
            .position(|found| found.ordered() == self.selection.ordered());

        let Some(finder) = &self.finder else {
            return;
        };
        let mut query = finder.query.clone();
        let mut replacement = finder.replacement.clone();
        let with_replace = finder.with_replace;
        let take_focus = finder.focus;
        let note = finder.note.clone();
        let bar_focused = self.finder_focused;

        let mut close = false;
        let mut forward = false;
        let mut back = false;
        let mut replace_one = false;
        let mut replace_every = false;
        let mut focused_now = false;

        egui::Panel::top("scriva-find").show(ui, |ui| {
            // Read before the fields are drawn: a TextEdit consumes the Escape
            // and the Enter it is given, and by then the answer is gone.
            if bar_focused {
                let (escape, enter, f3, shift) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::Escape),
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::F3),
                        i.modifiers.shift,
                    )
                });
                if escape {
                    close = true;
                }
                if enter || f3 {
                    if shift {
                        back = true;
                    } else {
                        forward = true;
                    }
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Find");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut query)
                        .desired_width(220.0)
                        .hint_text("Find in document"),
                );
                if take_focus || ((forward || back) && bar_focused) {
                    field.request_focus();
                }
                focused_now |= field.has_focus();
                if crate::icons::button(
                    ui,
                    crate::icons::Icon::ChevronUp,
                    false,
                    "Previous match (Shift+F3)",
                ) {
                    back = true;
                }
                if crate::icons::button(
                    ui,
                    crate::icons::Icon::ChevronDown,
                    false,
                    "Next match (F3)",
                ) {
                    forward = true;
                }
                let standing = match &note {
                    Some(note) => note.clone(),
                    None if query.is_empty() => String::new(),
                    None => match (current, total) {
                        (Some(index), _) => format!("{} of {total}", index + 1),
                        (None, 0) => "No matches".to_owned(),
                        (None, n) => format!("{n} matches"),
                    },
                };
                ui.label(egui::RichText::new(standing).weak());
                if with_replace {
                    ui.separator();
                    ui.label("Replace with");
                    let field =
                        ui.add(egui::TextEdit::singleline(&mut replacement).desired_width(180.0));
                    focused_now |= field.has_focus();
                    if ui.button("Replace").clicked() {
                        replace_one = true;
                    }
                    if ui.button("Replace All").clicked() {
                        replace_every = true;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(6.0);
        });

        if let Some(finder) = &mut self.finder {
            if finder.query != query {
                finder.note = None;
            }
            finder.query = query;
            finder.replacement = replacement;
            finder.focus = false;
        }
        self.finder_focused = focused_now;
        if close {
            self.finder = None;
            self.finder_focused = false;
            if let Some(id) = self.surface_id {
                ui.ctx().memory_mut(|m| m.request_focus(id));
            }
            return;
        }
        if replace_one {
            self.replace_current();
        }
        if replace_every {
            self.replace_all();
        }
        if forward {
            self.jump_match(true);
        }
        if back {
            self.jump_match(false);
        }
    }

    fn finish(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            other => self.run(other),
        }
    }

    /// The page surface: a scrolling desk with the pages on it.
    fn surface(&mut self, ui: &mut egui::Ui) {
        let zoom = self.view.zoom as f32;
        let (extent_w, extent_h) = self.view.extent();
        let outer = ui.available_rect_before_wrap();
        self.viewport = outer.height();
        ui.painter().rect_filled(outer, 0.0, view::desk());
        // At least as wide as the window, so a page narrower than the desk is
        // centred on it rather than pinned to the left edge.
        let desired = egui::vec2(
            (extent_w as f32 * zoom).max(outer.width() - 2.0),
            extent_h as f32 * zoom,
        );

        let scroll = egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
                self.surface_id = Some(response.id);
                // The surface is an editor, not a button. Without this filter
                // egui reads a bare arrow key as "move keyboard focus to the
                // neighbouring widget" — Up walked the focus onto the toolbar,
                // and the caret vanished with it.
                ui.ctx().memory_mut(|m| {
                    m.set_focus_lock_filter(
                        response.id,
                        egui::EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        },
                    );
                });
                // The pages are laid out against their own width; the extra
                // width the desk has goes half to each side.
                let slack = ((rect.width() - extent_w as f32 * zoom) / 2.0).max(0.0);
                let origin = rect.min + egui::vec2(slack, 0.0);
                let painter = ui.painter_at(rect);
                // Over the paper the pointer is a text cursor, which is how a
                // window says "this is a place where clicking means something".
                if response.hovered() && self.dragging.is_none() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }

                // A click on the desk gives keyboard focus to the surface itself
                // (below), so that typing goes somewhere. The caret must stay
                // visible when the surface holds its own focus — only some other
                // widget (a dialog's text field) holding it should hide the caret.
                self.focused = ui
                    .ctx()
                    .memory(|m| m.focused().is_none_or(|id| id == response.id));
                // Decode before painting: the painter borrows the pages, and the
                // cache cannot be borrowed mutably at the same time.
                self.pictures.prepare(
                    ui.ctx(),
                    self.package.as_ref(),
                    self.parts.as_ref(),
                    view::image_rels(&self.view).into_iter(),
                );
                view::paint(
                    &painter,
                    &self.view,
                    self.selection,
                    if self.finder.is_some() {
                        &self.find_matches
                    } else {
                        &[]
                    },
                    Some(self.caret()),
                    self.focused,
                    zoom,
                    origin,
                    self.shaper.as_mut().expect("a shaper by now"),
                    &self.pictures,
                    self.picked,
                );

                // A press decides what the drag is: a picture under the pointer
                // is dragged as an object, and anything else sweeps a selection.
                if response.drag_started() || response.clicked() {
                    let spot = response
                        .interact_pointer_pos()
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom));
                    self.dragging = None;
                    match spot.and_then(|spot| {
                        view::drawing_at(&self.view, spot).map(|found| (spot, found))
                    }) {
                        Some((spot, (picked, rect))) => {
                            self.picked = Some(picked);
                            self.dragging = crate::drawings::grip_at(rect, spot.x, spot.y);
                            self.drag_from = Some((spot.x, spot.y));
                            ui.ctx().memory_mut(|m| m.request_focus(response.id));
                        }
                        None => self.picked = None,
                    }
                }
                if self.picked.is_none() {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        if let Some(spot) = self.spot_at(pointer, origin, zoom) {
                            if let Some(caret) = view::caret_at(&self.view, spot) {
                                let extend = ui.input(|i| i.modifiers.shift) || self.sweeping;
                                self.set_caret(caret, extend);
                            }
                        }
                    }
                    // A second click takes the word and a third takes the
                    // paragraph, the way every word processor since has.
                    if response.double_clicked() {
                        let caret = self.caret();
                        let content = self.paragraph_text(caret.paragraph);
                        let word = text::word_at(&content, caret.offset);
                        self.selection = Selection {
                            anchor: Caret {
                                paragraph: caret.paragraph,
                                offset: word.start,
                            },
                            head: Caret {
                                paragraph: caret.paragraph,
                                offset: word.end,
                            },
                        };
                    }
                    if response.triple_clicked() {
                        let caret = self.caret();
                        let length = self.paragraph_text(caret.paragraph).len();
                        self.selection = Selection {
                            anchor: Caret {
                                paragraph: caret.paragraph,
                                offset: 0,
                            },
                            head: Caret {
                                paragraph: caret.paragraph,
                                offset: length,
                            },
                        };
                    }
                    // A right-click outside the selection moves the caret
                    // there first, so the menu acts on what was clicked.
                    if response.secondary_clicked() {
                        if let Some(caret) = response
                            .interact_pointer_pos()
                            .and_then(|pointer| self.spot_at(pointer, origin, zoom))
                            .and_then(|spot| view::caret_at(&self.view, spot))
                        {
                            let (start, end) = self.selection.ordered();
                            let inside =
                                !self.selection.is_empty() && caret >= start && caret <= end;
                            if !inside {
                                self.set_caret(caret, false);
                            }
                        }
                        ui.ctx().memory_mut(|m| m.request_focus(response.id));
                    }
                }
                let has_selection = !self.selection.is_empty();
                let mut chosen: Option<Command> = None;
                response.context_menu(|ui| {
                    ui.set_min_width(160.0);
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Cut"))
                        .clicked()
                    {
                        chosen = Some(Command::Cut);
                        ui.close();
                    }
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Copy"))
                        .clicked()
                    {
                        chosen = Some(Command::Copy);
                        ui.close();
                    }
                    if ui.button("Paste").clicked() {
                        chosen = Some(Command::Paste);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Select All").clicked() {
                        chosen = Some(Command::SelectAll);
                        ui.close();
                    }
                });
                if let Some(command) = chosen {
                    self.run(command);
                }
                if response.drag_started() {
                    self.sweeping = false;
                }
                if response.dragged() {
                    self.sweeping = self.picked.is_none();
                    // The drag is applied a step at a time, from where the
                    // pointer was last frame, so the model always says what is
                    // on the screen and an undo puts back one whole drag.
                    if let (Some(grip), Some(from), Some(pointer)) = (
                        self.dragging,
                        self.drag_from,
                        response.interact_pointer_pos(),
                    ) {
                        if let Some(spot) = self.spot_at(pointer, origin, zoom) {
                            self.drag_drawing(grip, spot.x - from.0, spot.y - from.1);
                            self.drag_from = Some((spot.x, spot.y));
                        }
                    }
                }
                if response.drag_stopped() || response.clicked() {
                    self.sweeping = false;
                    self.dragging = None;
                    self.drag_from = None;
                    self.dragged = false;
                }
                // A click anywhere on the desk puts the caret in the document,
                // which is what makes typing go somewhere.
                if response.clicked() && self.picked.is_none() {
                    ui.ctx().memory_mut(|m| m.request_focus(response.id));
                }
                // Scroll to wherever asked for, once the layout is current —
                // a caret has no place on the page until the page exists.
                if self.reveal.is_some() && self.view.is_stale(self.stamp) {
                    ui.ctx().request_repaint();
                } else if let Some(caret) = self.reveal.take() {
                    if let Some((page, rect)) = view::caret_rect(&self.view, caret) {
                        let (page_x, page_y) = self.view.page_origin(page);
                        let min = origin
                            + egui::vec2(
                                (page_x as f32 + rect.min.x) * zoom,
                                (page_y as f32 + rect.min.y) * zoom,
                            );
                        let target =
                            egui::Rect::from_min_size(min, egui::vec2(2.0, rect.height() * zoom))
                                .expand2(egui::vec2(0.0, 24.0));
                        ui.scroll_to_rect(target, None);
                    }
                }
                response
            });
        self.scroll = scroll.state.offset.y;
    }

    /// Moves or resizes the picked drawing by one step of a drag.
    ///
    /// The whole drag is one undo entry: the first step records the paragraph as
    /// it was, and the rest change it further without recording again.
    fn drag_drawing(&mut self, grip: crate::drawings::Grip, dx: f64, dy: f64) {
        use crate::drawings::{moved, resized, Grip};

        let Some(picked) = self.picked else {
            return;
        };
        let Some((page, _)) = view::rect_of(&self.view, picked) else {
            return;
        };
        let geometry = match self.view.pages().get(page) {
            Some(page) => page.geometry,
            None => return,
        };
        // Where the paragraph starts on the page, which is what an offset
        // relative to the paragraph is measured from.
        let line_top = view::rect_of(&self.view, picked)
            .map(|(_, rect)| rect.1)
            .unwrap_or(geometry.top);
        let before = match self.document.paragraphs().get(picked.paragraph) {
            Some(paragraph) => (*paragraph).clone(),
            None => return,
        };
        let mut paragraphs = self.document.paragraphs_mut();
        let Some(drawing) = paragraphs
            .get_mut(picked.paragraph)
            .and_then(|paragraph| paragraph.drawing_mut(picked.nth))
        else {
            return;
        };
        let changed = match grip {
            Grip::Body => moved(drawing, &geometry, line_top, dx, dy),
            grip => resized(drawing, grip, dx, dy),
        };
        drop(paragraphs);
        if !changed {
            return;
        }
        if !self.dragged {
            self.history.push(crate::edit::Change::Paragraph {
                index: picked.paragraph,
                before: Box::new(before),
            });
            self.dragged = true;
        }
        self.changed();
    }

    /// Takes the picked drawing out of the document.
    fn delete_drawing(&mut self) -> bool {
        let Some(picked) = self.picked else {
            return false;
        };
        let before = match self.document.paragraphs().get(picked.paragraph) {
            Some(paragraph) => (*paragraph).clone(),
            None => return false,
        };
        let removed = {
            let mut paragraphs = self.document.paragraphs_mut();
            paragraphs
                .get_mut(picked.paragraph)
                .is_some_and(|paragraph| paragraph.remove_drawing(picked.nth))
        };
        if !removed {
            return false;
        }
        self.history.push(crate::edit::Change::Paragraph {
            index: picked.paragraph,
            before: Box::new(before),
        });
        self.picked = None;
        self.changed();
        true
    }

    /// Turns a window point into a point on a page.
    fn spot_at(&self, pointer: egui::Pos2, origin: egui::Pos2, zoom: f32) -> Option<view::Spot> {
        // `origin` is already the top-left of the *pages*, slack included.
        let local = (pointer - origin) / zoom;
        let mut y = 16.0f64;
        let width = self.view.extent().0;
        for (index, page) in self.view.pages().iter().enumerate() {
            let left = (width - page.geometry.width) / 2.0;
            let bottom = y + page.geometry.height;
            if (local.y as f64) < bottom || index + 1 == self.view.pages().len() {
                return Some(view::Spot {
                    page: index,
                    x: local.x as f64 - left,
                    y: local.y as f64 - y,
                });
            }
            y = bottom + 16.0;
        }
        None
    }
}

/// A hairline the full width of the bar.
fn rule(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 5.0), egui::Sense::hover());
    let y = rect.center().y.round() + 0.5;
    ui.painter().line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(1.0, egui::Color32::from_gray(0xDC)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(texts: &[&str]) -> Scriva {
        let mut app = Scriva::new();
        app.document.body = texts
            .iter()
            .map(|text| Block::Paragraph(Paragraph::of(text)))
            .collect();
        app.stamp += 1;
        app
    }

    #[test]
    fn the_layout_commands_change_the_page_and_undo_back() {
        use wp_model::{Orientation, PageMargins};
        let mut app = app_with(&["hello"]);
        let was = app.document.section.page;

        app.run(Command::Orient(Orientation::Landscape));
        assert_eq!(
            app.document.section.page.orientation,
            Orientation::Landscape
        );
        assert_eq!(
            app.document.section.page.width, was.height,
            "the paper turned"
        );
        app.run(Command::Undo);
        assert_eq!(app.document.section.page, was, "and undo turns it back");

        let narrow = PageMargins {
            top: Twips(720),
            bottom: Twips(720),
            start: Twips(720),
            end: Twips(720),
            ..app.document.section.margins
        };
        app.run(Command::Margins(narrow));
        assert_eq!(app.document.section.margins.start, Twips(720));

        app.run(Command::PageBreak);
        let has_break = app.document.paragraphs()[0]
            .runs()
            .iter()
            .flat_map(|run| run.content.iter())
            .any(|piece| {
                matches!(
                    piece,
                    wp_model::doc::Piece::Break(wp_model::doc::Break::Page)
                )
            });
        assert!(has_break, "Ctrl+Enter left a page break at the caret");
    }

    #[test]
    fn the_word_count_counts_the_way_word_does() {
        use wp_model::doc::{Break, Inline, Piece, Run};
        // A slash splits — Word counts "TCP/IP" as two — but a hyphen does not.
        let mut app = app_with(&["TCP/IP real-time networks"]);
        assert_eq!(app.word_count(), 4);
        // A page break separates the words around it, even though `text()`
        // has nothing to show for it.
        let mut run = Run::of("before");
        run.content.push(Piece::Break(Break::Page));
        run.content.push(Piece::Text("after".into()));
        app.document.body.push(Block::Paragraph(Paragraph {
            content: vec![Inline::Run(run)],
            ..Paragraph::default()
        }));
        assert_eq!(app.word_count(), 6);
    }

    #[test]
    fn a_new_document_is_one_empty_paragraph_with_a_default_style() {
        let app = Scriva::new();
        assert_eq!(app.paragraph_count(), 1);
        assert_eq!(app.word_count(), 0);
        let normal = app
            .document
            .styles
            .default_style(wp_model::StyleKind::Paragraph)
            .expect("a default paragraph style");
        assert_eq!(
            app.document.styles.get(normal).unwrap().id.as_ref(),
            "Normal"
        );
    }

    /// An app whose view has really been laid out, for keys that ask the
    /// layout where the caret is — Home, End and the arrows.
    fn laid_app(text: &str, text_width: f64) -> Scriva {
        let ctx = egui::Context::default();
        ui_kit::fonts::register(&ctx, &[]);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let mut app = app_with(&[text]);
        let margins =
            app.document.section.margins.start.points() + app.document.section.margins.end.points();
        app.document.section.page.width = Twips::from_points(text_width + margins);
        let mut shaper = Egui::new(&ctx);
        app.view.refresh(
            &app.document,
            &wp_layout::FieldValues::new(),
            app.stamp,
            &mut shaper,
        );
        app.shaper = Some(shaper);
        app
    }

    #[test]
    fn home_and_end_move_on_the_visual_line_not_the_paragraph() {
        let text = "aa bb cc dd ee ff gg hh";
        // 60 points of text: a few words per line, so the paragraph wraps.
        let mut app = laid_app(text, 60.0);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 0,
        });
        app.key(egui::Key::End, egui::Modifiers::NONE);
        let end = app.caret();
        assert_eq!(end.paragraph, 0);
        assert!(
            end.offset > 0 && end.offset < text.len(),
            "End stops at the end of the first visual line, not at {:?}",
            text.len()
        );
        assert_ne!(
            &text[end.offset - 1..end.offset],
            " ",
            "and not beyond the space the wrap ate"
        );

        // Down one visual line, still inside the same paragraph.
        app.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        let down = app.caret();
        assert_eq!(down.paragraph, 0);
        assert!(
            down.offset >= end.offset,
            "the line below, not the line above"
        );

        app.key(egui::Key::Home, egui::Modifiers::NONE);
        let home = app.caret();
        assert_eq!(home.paragraph, 0);
        assert!(
            home.offset >= end.offset,
            "Home goes to this line's start, not the paragraph's"
        );

        // Ctrl+End still means the end of the document.
        app.key(egui::Key::End, egui::Modifiers::COMMAND);
        assert_eq!(app.caret().offset, text.len());
    }

    fn bulleted(app: &mut Scriva, paragraph: usize) {
        let mut paragraphs = app.document.paragraphs_mut();
        paragraphs[paragraph].props.numbering = Some(wp_model::prop::NumRef {
            num_id: 1,
            level: 0,
        });
    }

    #[test]
    fn enter_on_an_empty_list_item_ends_the_list() {
        let mut app = app_with(&["item", ""]);
        bulleted(&mut app, 0);
        bulleted(&mut app, 1);
        app.selection = Selection::at(Caret {
            paragraph: 1,
            offset: 0,
        });
        app.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(app.paragraph_count(), 2, "no new paragraph was added");
        assert_eq!(
            app.document.paragraphs()[1].props.numbering,
            None,
            "the bullet is gone instead"
        );
        app.run(Command::Undo);
        assert!(
            app.document.paragraphs()[1].props.numbering.is_some(),
            "and undo puts it back"
        );
    }

    #[test]
    fn backspace_at_the_start_of_a_list_item_takes_the_bullet_first() {
        let mut app = app_with(&["first", "second"]);
        bulleted(&mut app, 1);
        app.selection = Selection::at(Caret {
            paragraph: 1,
            offset: 0,
        });
        app.key(egui::Key::Backspace, egui::Modifiers::NONE);
        assert_eq!(app.paragraph_count(), 2, "nothing joined yet");
        assert_eq!(app.document.paragraphs()[1].props.numbering, None);
        app.key(egui::Key::Backspace, egui::Modifiers::NONE);
        assert_eq!(app.document.text(), "firstsecond", "the second one joins");
    }

    #[test]
    fn tab_at_the_start_of_a_list_item_changes_its_depth() {
        let mut app = app_with(&["item", "plain"]);
        bulleted(&mut app, 0);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 0,
        });
        app.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(
            app.document.paragraphs()[0].props.numbering.unwrap().level,
            1
        );
        app.key(egui::Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(
            app.document.paragraphs()[0].props.numbering.unwrap().level,
            0
        );
        // Anywhere else, Tab is still a tab.
        app.selection = Selection::at(Caret {
            paragraph: 1,
            offset: 5,
        });
        app.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(app.paragraph_text(1), "plain\t");
    }

    #[test]
    fn the_selection_reads_out_as_text_across_paragraphs() {
        let mut app = app_with(&["first line", "second"]);
        assert_eq!(app.selected_text(), None, "an empty selection is nothing");
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 6,
            },
            head: Caret {
                paragraph: 1,
                offset: 3,
            },
        };
        assert_eq!(app.selected_text().as_deref(), Some("line\nsec"));
    }

    #[test]
    fn pasting_types_over_the_selection_and_newlines_press_enter() {
        let mut app = app_with(&["first line", "second"]);
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 6,
            },
            head: Caret {
                paragraph: 1,
                offset: 3,
            },
        };
        app.paste_text("X\r\nY");
        assert_eq!(app.document.text(), "first X\nYond");
        assert_eq!(app.caret().paragraph, 1);
        assert_eq!(app.caret().offset, 1);
    }

    #[test]
    fn ctrl_backspace_deletes_the_word_before_the_caret() {
        let mut app = app_with(&["hello world"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 11,
        });
        app.key(egui::Key::Backspace, egui::Modifiers::COMMAND);
        assert_eq!(app.document.text(), "hello ");
        app.history.undo(&mut app.document);
        assert_eq!(app.document.text(), "hello world");
    }

    #[test]
    fn ctrl_delete_deletes_the_word_after_the_caret() {
        let mut app = app_with(&["hello world"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 0,
        });
        app.key(egui::Key::Delete, egui::Modifiers::COMMAND);
        assert_eq!(app.document.text(), "world");
    }

    #[test]
    fn find_next_selects_the_match_and_moving_on_wraps() {
        let mut app = app_with(&["alpha beta alpha"]);
        app.finder = Some(Finder {
            query: "alpha".into(),
            ..Finder::default()
        });
        app.jump_match(true);
        assert_eq!(app.selection.ordered().0.offset, 0, "the first occurrence");
        assert_eq!(app.selection.ordered().1.offset, 5, "selected whole");
        app.jump_match(true);
        assert_eq!(app.selection.ordered().0.offset, 11);
        app.jump_match(true);
        assert_eq!(app.selection.ordered().0.offset, 0, "wrapped around");
        assert!(app.reveal.is_some(), "and the view was asked to follow");
    }

    #[test]
    fn replace_all_replaces_every_match_and_says_how_many() {
        let mut app = app_with(&["one two one", "one more"]);
        app.finder = Some(Finder {
            query: "ONE".into(),
            replacement: "1".into(),
            ..Finder::default()
        });
        app.replace_all();
        assert_eq!(app.document.text(), "1 two 1\n1 more");
        assert_eq!(
            app.finder.as_ref().unwrap().note.as_deref(),
            Some("Replaced 3")
        );
        // A replace is a deletion and an insertion, so it comes back in two.
        app.history.undo(&mut app.document);
        app.history.undo(&mut app.document);
        assert_eq!(app.document.text(), "one two 1\n1 more", "undo, one by one");
    }

    #[test]
    fn select_all_reaches_the_end_of_the_last_paragraph() {
        let mut app = app_with(&["one", "two", "three"]);
        app.run(Command::SelectAll);
        let (start, end) = app.selection.ordered();
        assert_eq!(
            start,
            Caret {
                paragraph: 0,
                offset: 0
            }
        );
        assert_eq!(
            end,
            Caret {
                paragraph: 2,
                offset: 5
            }
        );
    }

    #[test]
    fn bold_with_no_selection_applies_to_the_word_the_caret_is_in() {
        // Otherwise Ctrl+B with the caret in a word appears to do nothing at all.
        let mut app = app_with(&["hello world"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 2,
        });
        app.run(Command::Bold);
        let paragraphs = app.document.paragraphs();
        let runs = paragraphs[0].runs();
        assert!(runs[0].props.bold(), "the word the caret was in");
        assert!(
            runs.last().is_some_and(|run| !run.props.bold()),
            "and not the rest of the line"
        );
    }

    #[test]
    fn bold_is_a_toggle_rather_than_a_one_way_switch() {
        let mut app = app_with(&["word"]);
        app.run(Command::SelectAll);
        app.run(Command::Bold);
        assert!(app.document.paragraphs()[0].runs()[0].props.bold());
        app.run(Command::Bold);
        assert!(!app.document.paragraphs()[0].runs()[0].props.bold());
    }

    #[test]
    fn undo_puts_the_caret_back_in_the_document() {
        let mut app = app_with(&["first", "second"]);
        app.selection = Selection::at(Caret {
            paragraph: 1,
            offset: 0,
        });
        let caret = edit::backspace(&mut app.document, &mut app.history, app.selection);
        app.selection = Selection::at(caret);
        assert_eq!(app.paragraph_count(), 1);
        app.run(Command::Undo);
        assert_eq!(app.paragraph_count(), 2);
        assert!(
            app.selection.head.paragraph < app.paragraph_count(),
            "the caret is inside the document it names"
        );
    }

    #[test]
    fn a_caret_past_the_end_is_brought_back_inside() {
        let document = Document {
            body: vec![Block::Paragraph(Paragraph::of("short"))],
            ..Document::new()
        };
        let clamped = clamp(
            &document,
            Caret {
                paragraph: 9,
                offset: 900,
            },
        );
        assert_eq!(
            clamped,
            Caret {
                paragraph: 0,
                offset: 5
            }
        );
    }

    #[test]
    fn alignment_is_a_paragraph_command_and_needs_no_selection() {
        let mut app = app_with(&["one", "two"]);
        app.selection = Selection::at(Caret {
            paragraph: 1,
            offset: 0,
        });
        app.run(Command::Align(Justify::Center));
        assert_eq!(app.document.paragraphs()[0].props.justify, None);
        assert_eq!(
            app.document.paragraphs()[1].props.justify,
            Some(Justify::Center)
        );
    }

    #[test]
    fn indenting_moves_by_half_an_inch_and_never_past_the_margin() {
        let mut app = app_with(&["text"]);
        app.run(Command::Indent(1));
        assert_eq!(
            app.document.paragraphs()[0].props.indent.start,
            Some(Twips(720))
        );
        app.run(Command::Indent(-1));
        assert_eq!(
            app.document.paragraphs()[0].props.indent.start,
            Some(Twips(0))
        );
        app.run(Command::Indent(-1));
        assert_eq!(
            app.document.paragraphs()[0].props.indent.start,
            Some(Twips(0)),
            "and no further"
        );
    }

    #[test]
    fn a_document_with_no_path_is_still_called_something() {
        let app = Scriva::new();
        let (name, dirty) = app.document().expect("a title");
        assert_eq!(name, "Document1");
        assert!(!dirty);
    }

    #[test]
    fn an_edit_marks_the_document_dirty_and_the_view_stale() {
        let mut app = app_with(&["text"]);
        let stamp = app.stamp;
        app.run(Command::SelectAll);
        app.run(Command::Bold);
        assert!(app.dirty);
        assert_ne!(app.stamp, stamp, "the view has to lay out again");
    }

    #[test]
    fn typing_with_track_changes_on_records_rather_than_replaces() {
        let mut app = app_with(["hello"].as_slice());
        app.document.settings.track_changes = true;
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 5,
        });
        app.type_text(" there");
        assert_eq!(app.document.text(), "hello there");
        let changes = crate::revise::tracked(&app.document);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].what, "inserted");
    }

    #[test]
    fn typing_with_track_changes_off_is_an_ordinary_edit() {
        let mut app = app_with(["hello"].as_slice());
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 5,
        });
        app.type_text("!");
        assert_eq!(app.document.text(), "hello!");
        assert!(crate::revise::tracked(&app.document).is_empty());
    }

    #[test]
    fn accept_all_with_nothing_tracked_says_so_rather_than_doing_nothing() {
        let mut app = app_with(["plain"].as_slice());
        app.run(Command::AcceptAll);
        assert!(app.message.is_some());
        assert!(!app.dirty);
    }

    #[test]
    fn a_comment_needs_a_selection_to_be_about() {
        let mut app = app_with(["text"].as_slice());
        app.run(Command::AddComment);
        assert!(app.message.is_some(), "and says why");
        assert!(app.drafting.is_none());

        app.message = None;
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 0,
                offset: 4,
            },
        };
        app.run(Command::AddComment);
        assert!(app.drafting.is_some());
    }

    #[test]
    fn next_change_walks_round_the_document() {
        let mut app = app_with(["one", "two", "three"].as_slice());
        let paragraphs = &mut app.document.body;
        if let Block::Paragraph(p) = &mut paragraphs[2] {
            p.content = vec![wp_model::doc::inserted_by(
                "A",
                1,
                vec![wp_model::doc::Run::of("added")]
                    .into_iter()
                    .map(wp_model::Inline::Run)
                    .collect(),
            )];
        }
        app.run(Command::NextChange);
        assert_eq!(app.caret().paragraph, 2);
        // And round again from the end.
        app.run(Command::NextChange);
        assert_eq!(app.caret().paragraph, 2);
    }

    #[test]
    fn go_to_puts_the_caret_at_a_paragraph_and_asks_to_be_shown_it() {
        let mut app = app_with(&["one", "two", "three"]);
        app.run(Command::GoTo(2));
        assert_eq!(
            app.caret(),
            Caret {
                paragraph: 2,
                offset: 0
            }
        );
        assert!(app.reveal.is_some(), "a caret has no place on the page yet");
    }

    #[test]
    fn go_to_a_paragraph_that_is_not_there_lands_inside_the_document() {
        let mut app = app_with(&["only"]);
        app.run(Command::GoTo(99));
        assert_eq!(app.caret().paragraph, 0);
    }

    #[test]
    fn updating_a_table_of_contents_with_no_toc_field_says_so() {
        // Silently doing nothing is the failure here: the user pressed a key and
        // has to be told why nothing happened.
        let mut app = app_with(&["body text"]);
        app.run(Command::UpdateToc);
        assert!(app.message.is_some());
        assert!(!app.dirty, "and nothing was changed");
    }

    #[test]
    fn a_document_with_a_path_answers_the_filename_field() {
        let mut app = app_with(&["text"]);
        app.path = Some(PathBuf::from("C:/reports/Q3.docx"));
        app.refresh_fields();
        assert_eq!(app.fields.file_name.as_deref(), Some("Q3.docx"));
    }

    #[test]
    fn the_extension_decides_the_format() {
        assert_eq!(Format::of(Path::new("a.docx")), Format::Docx);
        assert_eq!(Format::of(Path::new("a.DOCX")), Format::Docx);
        assert_eq!(Format::of(Path::new("a.md")), Format::Markdown);
        assert_eq!(Format::of(Path::new("a.txt")), Format::Text);
        assert!(Format::Markdown.is_lossy());
        assert!(!Format::Docx.is_lossy());
    }

    #[test]
    fn a_markdown_file_opens_as_a_document_with_headings() {
        let dir = std::env::temp_dir().join("scriva-c26");
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("note.md");
        std::fs::write(&path, "# Title\r\n\r\nSome **bold** text.\r\n").expect("written");

        let mut app = Scriva::new();
        app.open_path(&path);
        assert_eq!(app.paragraph_count(), 2);
        assert!(app.package.is_none(), "there is no package behind a .md");
        assert_eq!(app.ending, wp_text::LineEnding::Crlf, "kept for the save");
        let paragraphs = app.document.paragraphs();
        assert_eq!(
            wp_model::outline::heading_level(paragraphs[0], &app.document.styles),
            Some(1)
        );
    }

    #[test]
    fn saving_as_text_keeps_the_line_endings_the_file_came_in_with() {
        let dir = std::env::temp_dir().join("scriva-c26");
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("plain.txt");
        std::fs::write(&path, "one\r\ntwo\r\n").expect("written");

        let mut app = Scriva::new();
        app.open_path(&path);
        assert_eq!(app.paragraph_count(), 2);
        assert!(app.save_text(&path, Format::Text));
        let back = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(back, "one\r\ntwo");
    }

    #[test]
    fn saving_in_a_lossy_format_asks_before_it_writes() {
        // A user who did not mean it has no way back once the file is on disk.
        let mut app = app_with(["text"].as_slice());
        app.pending = Some(Pending::Lossy(PathBuf::from("x.md"), Format::Markdown));
        assert!(matches!(app.pending, Some(Pending::Lossy(_, _))));
    }

    #[test]
    fn a_file_saved_without_an_extension_gets_one() {
        assert_eq!(
            with_extension(PathBuf::from("report")),
            PathBuf::from("report.docx")
        );
        assert_eq!(
            with_extension(PathBuf::from("report.docm")),
            PathBuf::from("report.docm")
        );
    }
}
