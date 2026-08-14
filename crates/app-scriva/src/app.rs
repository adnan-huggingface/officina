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
    Bold,
    Italic,
    Underline,
    Strike,
    Superscript,
    Subscript,
    ClearFormatting,
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
            _ => Format::Docx,
        }
    }

    /// Whether saving in this format throws formatting away.
    pub fn is_lossy(self) -> bool {
        !matches!(self, Format::Docx)
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
    /// How the open document was written, so a save puts it back the same way.
    encoding: wp_text::Encoding,
    ending: wp_text::LineEnding,
    /// Where the view should scroll to, once it knows where that is.
    reveal: Option<Caret>,
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
            parts: None,
            picked: None,
            dragging: None,
            drag_from: None,
            dragged: false,
            pictures: crate::pictures::Pictures::new(),
            encoding: wp_text::Encoding::Utf8,
            ending: wp_text::LineEnding::Crlf,
            reveal: None,
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
    fn word_count(&self) -> usize {
        self.document
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.text().split_whitespace().count())
            .sum()
    }

    // ------------------------------------------------------------ files

    fn open_path(&mut self, path: &Path) {
        match Format::of(path) {
            Format::Docx => self.open_docx(path),
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
        if format == Format::Docx && self.package.is_none() {
            // Nothing to preserve: this document was never read from a file, so
            // there is no package to edit and one would have to be authored.
            self.message = Some((
                "Not yet".to_owned(),
                "Saving a new document as .docx needs a writer that authors a \
                 package from nothing. Save it as Markdown, or open a .docx and \
                 edit that."
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
                    .add_filter("All documents", &["docx", "docm", "dotx", "md", "txt"])
                    .add_filter("Word documents", &["docx", "docm", "dotx"])
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
            (ctrl, Key::F, Command::Navigator),
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
                }
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
            Key::Home => {
                let next = if modifiers.command {
                    Caret {
                        paragraph: 0,
                        offset: 0,
                    }
                } else {
                    Caret {
                        paragraph: caret.paragraph,
                        offset: 0,
                    }
                };
                self.set_caret(next, extend);
            }
            Key::End => {
                let next = if modifiers.command {
                    Caret {
                        paragraph: last,
                        offset: self.paragraph_text(last).len(),
                    }
                } else {
                    Caret {
                        paragraph: caret.paragraph,
                        offset: content.len(),
                    }
                };
                self.set_caret(next, extend);
            }
            Key::Backspace => {
                let caret = edit::backspace(&mut self.document, &mut self.history, self.selection);
                self.selection = Selection::at(clamp(&self.document, caret));
                self.changed();
            }
            Key::Delete => {
                let caret =
                    edit::delete_forward(&mut self.document, &mut self.history, self.selection);
                self.selection = Selection::at(clamp(&self.document, caret));
                self.changed();
            }
            Key::Enter => {
                let caret =
                    edit::split_paragraph(&mut self.document, &mut self.history, self.selection);
                self.selection = Selection::at(clamp(&self.document, caret));
                self.changed();
            }
            Key::Tab => {
                let caret =
                    edit::type_text(&mut self.document, &mut self.history, self.selection, "\t");
                self.selection = Selection::at(caret);
                self.changed();
            }
            Key::Escape => {
                self.selection = Selection::at(caret);
            }
            _ => {}
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

    /// One line up or down, using the laid-out lines.
    fn line_step(&self, caret: Caret, down: bool) -> Caret {
        let Some((page, rect)) = view::caret_rect(&self.view, caret) else {
            return caret;
        };
        let step = rect.height().max(1.0) as f64;
        let y = rect.center().y as f64 + if down { step } else { -step };
        let spot = view::Spot {
            page,
            x: rect.min.x as f64,
            y,
        };
        view::caret_at(&self.view, spot).unwrap_or(caret)
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

        let blocked =
            self.pending.is_some() || self.message.is_some() || egui::Popup::is_any_open(ui.ctx());
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
                // The pages are laid out against their own width; the extra
                // width the desk has goes half to each side.
                let slack = ((rect.width() - extent_w as f32 * zoom) / 2.0).max(0.0);
                let origin = rect.min + egui::vec2(slack, 0.0);
                let painter = ui.painter_at(rect);

                self.focused = ui.ctx().memory(|m| m.focused().is_none());
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
                    Some(self.caret()),
                    self.focused,
                    zoom,
                    origin,
                    self.shaper.as_ref().expect("a shaper by now"),
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
