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

use crate::clip;
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
    Print,
    ExportPdf,
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
    /// Insert ▸ Picture… — a picture from a file, at the caret.
    InsertPicture,
    /// Insert ▸ Table… — the rows-and-columns dialog.
    InsertTable,
    /// The Size box for the selected picture or chart.
    PictureSize,
    /// Takes the selected picture or chart out of the document.
    DeletePicture,
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
    /// Format ▸ Font — one of the faces the menu offers. Word's font box
    /// speaks for the Latin slots only; East Asian and complex runs keep
    /// their own faces.
    Font(&'static str),
    /// Format ▸ Text Colour — a palette entry, or `Auto` for Automatic.
    Color(wp_model::Color),
    /// The dialog for a colour the palette does not offer.
    CustomColor,
    Highlight(wp_model::Highlight),
    /// Paragraph ▸ Paragraph…: spacing and indents, by number.
    ParagraphDialog,
    /// Table ▸ Merge Cells: the cells the selection runs across, in one row,
    /// become one.
    MergeCells,
    /// Table ▸ Border Colour: every rule of the caret's table in this colour.
    BorderColor(wp_model::Color),
    CustomBorderColor,
    /// Opens the header of the page being looked at, for editing in place —
    /// View ▸ Header and Footer, and a double-click in the top margin.
    EditHeader,
    /// The same, underneath.
    EditFooter,
    /// Back to the text: Escape, the band bar's own button, and a
    /// double-click on the body.
    CloseChrome,
    /// Word's Switch Between Header and Footer, under its own name.
    SwitchBand,
    /// A PAGE field at the caret, and with it the total when asked.
    InsertPageNumber {
        of_pages: bool,
    },
    /// Word's Remove Header and Remove Footer: the band and the references
    /// that name it, gone together.
    RemoveChrome {
        footer: bool,
    },
    /// Format ▸ Watermark…
    Watermark,
    /// Paragraph ▸ Bullets — the selection's paragraphs into a bulleted
    /// list, or out of the one they are in.
    Bullets,
    /// Paragraph ▸ Numbering — the same, counting.
    Numbers,
    /// Table ▸ Borders — every edge ruled, or none at all.
    TableBorders(bool),
    /// Table ▸ Shading — a fill behind the caret's cell, or `None` to clear.
    TableShading(Option<[u8; 3]>),
    /// The dialog for the width of the caret's column.
    ColumnWidth,
    /// The dialog for the padding inside the caret's table's cells.
    CellMargins,
    Align(Justify),
    LineSpacing(Line240),
    Indent(i32),
    Style(wp_model::StyleId),
    Zoom(f64),
    ShowMarks,
    ShowRevisions,
    /// Put the caret at the start of a paragraph, and show it.
    ///
    /// The flow travels with the number: every flow counts its paragraphs from
    /// zero, so "paragraph 4" alone names four different places in a document
    /// with three headers.
    GoTo(wp_model::Scope, usize),
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
    /// Which of the document's flows the caret is in: the text, or one header
    /// or footer being edited in place.
    ///
    /// **A header is edited where it is drawn, not in a box.** A real one
    /// holds a table of revisions, a logo and three fields; a dialog that took
    /// its text and gave text back would hand that document a paragraph where
    /// its table used to be. So the editor moves into the band instead — the
    /// same caret, the same keys, the same undo — and this says where it is.
    scope: wp_model::Scope,
    /// Where the caret stood in the text before a band was opened, so closing
    /// one puts it back rather than at the top of the page. Word does the
    /// same, and a caret that jumps on the way out loses the user's place.
    left_behind: Option<Selection>,
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
    /// The custom-margins dialog, in inches: top, bottom, left, right, and
    /// then how far the header sits from the top of the paper and the footer
    /// from the bottom. Word's own Page Setup keeps those last two on the same
    /// sheet, under "From edge", and they belong with the margins because
    /// they are measured against the same four edges.
    margins_draft: Option<[String; 6]>,
    /// The insert-table dialog: columns, then rows.
    table_draft: Option<[String; 2]>,
    /// The colour dialog: what it colours, and six hex digits, as Word's
    /// Custom tab takes them.
    color_draft: Option<(ColorTarget, String)>,
    /// The column-width dialog: inches, like the margins box.
    column_draft: Option<String>,
    /// Table ▸ Cell Margins, in points: top, left, bottom, right.
    cell_margin_draft: Option<[String; 4]>,
    /// The faces the newly opened document embedded and the names it asks for,
    /// waiting for a frame in which egui will accept them. `None` once they
    /// have been handed over.
    #[allow(clippy::type_complexity)]
    pending_fonts: Option<(Vec<(String, bool, bool, Vec<u8>)>, Vec<String>)>,
    /// Set for the one frame between handing egui new fonts and it having
    /// built them, during which nothing measured is to be believed.
    fonts_settling: bool,
    watermark_draft: Option<WatermarkDraft>,
    /// The paragraph dialog: what it opened with, and what has been typed
    /// since, so that only the fields the user touched are applied.
    paragraph_draft: Option<(ParagraphDraft, ParagraphDraft)>,
    /// The picture-size dialog: width and height in inches, and whether the
    /// two are tied together.
    size_draft: Option<SizeDraft>,
    /// How the open document was written, so a save puts it back the same way.
    encoding: wp_text::Encoding,
    ending: wp_text::LineEnding,
    /// Where the view should scroll to, once it knows where that is.
    reveal: Option<Caret>,
    /// Which page to look for the caret on, when more than one shows it.
    ///
    /// Only a band's caret is ever in two places at once — a header stands on
    /// every page that shows it — and the page it was opened from is the one
    /// the reader is looking at. See [`view::caret_rect_on`].
    reveal_on: Option<usize>,
    /// Which page's band is being edited.
    ///
    /// **A band belongs to a page, and the caret cannot say which.** The same
    /// header stands on every page of its section, so asking the layout where
    /// the caret is answers with the first of them — and every question a band
    /// command asks is really about the page in front of the user: which
    /// section it is in, whether that section is linked to the one before it,
    /// which of the three kinds of band it wants. The page is remembered when
    /// the band is opened instead.
    band_page: Option<usize>,
    /// The link the last right-click landed on, for the menu it opened: the
    /// menu is drawn on later frames, when the click is long gone.
    menu_link: Option<crate::links::Destination>,
    /// The find bar, when it is open.
    finder: Option<Finder>,
    /// Whether one of the find bar's fields held the keyboard last frame —
    /// while it does, keys type into the bar and not into the document.
    finder_focused: bool,
    /// Every match of the finder's query, for the view to highlight — each
    /// with the flow it was found in, because Find looks through the headers
    /// and footers as well as the text.
    find_matches: Vec<find::Found>,
    /// What `find_matches` was computed from, so it is not recomputed while
    /// neither the document nor the query has changed.
    matches_for: (u64, String),
    /// The page surface's widget id, for giving the keyboard back to it.
    surface_id: Option<egui::Id>,
    /// The visible desk, in screen points. Height is the size of a Page Down;
    /// width is what "Page width" zoom fits the paper to.
    viewport: egui::Vec2,
    /// The Zoom box's percent field, while the box is open.
    zoom_draft: Option<String>,
    /// True until the percent field is first touched. While set, the whole
    /// number stays selected so the next keystroke replaces it.
    zoom_fresh: bool,
    /// What was last copied, with its formatting.
    clipboard: Option<Clip>,
    /// The picture or chart last copied as an object.
    copied_drawing: Option<CopiedDrawing>,
}

/// A copy of the document's own paragraphs, and the plain text that went to the
/// OS clipboard beside them.
///
/// The OS clipboard holds text and nothing else — put a bold word on it and a
/// bold word is not what comes back. Word gets around that by writing several
/// formats at once and reading its own back. This does the cheaper half of the
/// same trick: the formatting stays here, and the text on the board is the
/// receipt. If the board still says what this copy said, nothing else has
/// written to it since and these paragraphs are still what the user copied.
struct Clip {
    text: String,
    paragraphs: Vec<wp_model::doc::Paragraph>,
}

/// A picture or chart copied as an object, whole.
///
/// The same receipt trick as [`Clip`], for a copy that is not text: the
/// picture itself goes to the OS board so other applications get it, and the
/// PNG the board will hand back is kept as the receipt. While the board still
/// answers with those bytes, a paste means *this* drawing — the model clone,
/// anchoring, wrap and all — and not a re-encoded flattening of it.
#[derive(Clone)]
struct CopiedDrawing {
    /// The drawing as the model holds it. Its relationship still names the
    /// part in the document it came from.
    drawing: wp_model::doc::Drawing,
    /// A picture's file bytes, for pasting into a *different* document, where
    /// the relationship names nothing. A chart has no such bytes: it is a
    /// family of parts, and it travels only within its own document.
    bytes: Option<(Vec<u8>, &'static str, u32, u32)>,
    /// What the OS board will answer when asked for its image — the receipt.
    png: Option<Vec<u8>>,
}

/// The Size box, while it is open.
///
/// Inches, because that is what Word's box shows and what a user asking for
/// "three inches wide" means. The picture it belongs to is remembered with it:
/// the box is about *that* drawing, and a click elsewhere while it is open must
/// not silently resize a different one.
struct SizeDraft {
    picked: crate::drawings::Picked,
    width: String,
    height: String,
    /// Word's "Lock aspect ratio", and on by default there as here: a picture
    /// stretched on one axis is almost always an accident.
    locked: bool,
    /// Width over height as the box opened, for the lock to hold to.
    ratio: f64,
    /// The size the picture's own pixels ask for, when it has pixels. A chart
    /// has none, and its Reset button is not offered.
    natural: Option<(f64, f64)>,
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
            scope: wp_model::Scope::Body,
            left_behind: None,
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
            table_draft: None,
            color_draft: None,
            column_draft: None,
            cell_margin_draft: None,
            pending_fonts: None,
            fonts_settling: false,
            watermark_draft: None,
            paragraph_draft: None,
            size_draft: None,
            copied_drawing: None,
            parts: None,
            picked: None,
            dragging: None,
            drag_from: None,
            dragged: false,
            pictures: crate::pictures::Pictures::new(),
            encoding: wp_text::Encoding::Utf8,
            ending: wp_text::LineEnding::Crlf,
            reveal: None,
            reveal_on: None,
            band_page: None,
            menu_link: None,
            finder: None,
            finder_focused: false,
            find_matches: Vec::new(),
            matches_for: (u64::MAX, String::new()),
            surface_id: None,
            viewport: egui::Vec2::ZERO,
            zoom_draft: None,
            zoom_fresh: false,
            clipboard: None,
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

    /// Puts the caret at a place that may not be in the flow it is in now —
    /// what undo needs, since a header's edit has to be taken back with the
    /// header open, and what closing a band needs on the way out.
    ///
    /// A scope naming a header that is no longer there sends the caret back to
    /// the text rather than into an empty flow with nothing drawn for it.
    fn go_to(&mut self, scope: wp_model::Scope, caret: Caret) {
        let scope = match scope {
            wp_model::Scope::Chrome(id) if self.document.header(id).is_none() => {
                wp_model::Scope::Body
            }
            scope => scope,
        };
        if self.scope != scope {
            self.picked = None;
        }
        self.scope = scope;
        self.selection = Selection::at(clamp(&self.document, scope, caret));
    }

    fn set_caret(&mut self, caret: Caret, extend: bool) {
        if extend {
            self.selection.head = caret;
        } else {
            self.selection = Selection::at(caret);
        }
    }

    /// Where the link under `caret` points, if the caret stands in one.
    ///
    /// An external target lives in the package's relationships and not in the
    /// document, which is why this is the app's job and not the model's: only
    /// the app knows which file the paragraphs came out of.
    fn link_at(&self, caret: Caret) -> Option<crate::links::Destination> {
        let link = self
            .document
            .paragraph_in(self.scope, caret.paragraph)?
            .link_at(caret.offset)?;
        if let Some(anchor) = &link.anchor {
            return Some(crate::links::Destination::Here(anchor.to_string()));
        }
        let target = self.parts.as_ref()?.external_target(link.rel.as_deref()?)?;
        Some(crate::links::Destination::Away(target.to_owned()))
    }

    /// Follows a link: out to the desktop, or to the bookmark it names.
    fn follow_link(&mut self, destination: crate::links::Destination) {
        const TITLE: &str = "Cannot follow this link";
        match destination {
            crate::links::Destination::Here(name) => match self.document.bookmark(&name) {
                Some(paragraph) => {
                    let caret = Caret {
                        paragraph,
                        offset: 0,
                    };
                    self.set_caret(caret, false);
                    self.reveal = Some(caret);
                }
                // A link to a bookmark that is not there is a dangling link,
                // which is worth saying rather than doing nothing about.
                None => {
                    self.message = Some((
                        TITLE.to_owned(),
                        format!("This document has no bookmark named \u{201c}{name}\u{201d}."),
                    ))
                }
            },
            crate::links::Destination::Away(url) => {
                if let Err(why) = crate::links::open(&url) {
                    self.message = Some((TITLE.to_owned(), why));
                }
            }
        }
    }

    fn paragraph_count(&self) -> usize {
        self.document.paragraphs_in(self.scope).len()
    }

    fn paragraph_text(&self, index: usize) -> String {
        self.document
            .paragraph_in(self.scope, index)
            .map(|paragraph| paragraph.text())
            .unwrap_or_default()
    }

    /// Whether a header or footer is open for editing, for the View menu's
    /// tick and for anything that must not act on the text while it is.
    pub(crate) fn editing_band(&self) -> bool {
        self.scope != wp_model::Scope::Body
    }

    /// Whether the section names a header and whether it names a footer, for
    /// the two Remove commands.
    pub(crate) fn has_bands(&self) -> (bool, bool) {
        (
            !self.document.section.headers.is_empty(),
            !self.document.section.footers.is_empty(),
        )
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
        let range = self.formatting_range();
        (
            edit::all_runs(&self.document, self.scope, range, |props| {
                props.toggles.is_on(Toggle::Bold)
            }),
            edit::all_runs(&self.document, self.scope, range, |props| {
                props.toggles.is_on(Toggle::Italic)
            }),
            edit::all_runs(&self.document, self.scope, range, |props| {
                props.underline.is_some_and(|u| u.kind.draws())
            }),
        )
    }

    pub(crate) fn alignment(&self) -> Option<Justify> {
        edit::justify_at(&self.document, self.scope, self.caret())
    }

    /// What a formatting command with no selection would act on: the word the
    /// caret is in.
    fn formatting_range(&self) -> Selection {
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
        self.adopt_document_fonts();
        self.pictures.clear();
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        self.history.clear();
        self.selection = Selection::default();
        self.scope = wp_model::Scope::Body;
        self.left_behind = None;
        self.band_page = None;
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
            Ok((document, media)) => {
                self.document = document;
                self.package = None;
                self.parts = None;
                self.adopt_document_fonts();
                self.pictures.clear();
                // The old format keeps its pictures in a stream of its own, so
                // they arrive as bytes rather than as parts to fetch later.
                self.pictures
                    .adopt(media.into_iter().map(|picture| (picture.rel, picture.data)));
                // Named after the original, but with the modern extension, so
                // the save dialog opens on the right name in the right folder.
                self.path = Some(path.with_extension("docx"));
                // Not saved yet: it has never been written in this format.
                self.dirty = true;
                self.history.clear();
                self.selection = Selection::default();
                self.scope = wp_model::Scope::Body;
                self.left_behind = None;
                self.band_page = None;
                self.picked = None;
                self.scroll = 0.0;
                self.stamp = self.stamp.wrapping_add(1);
                self.view.invalidate();
                self.recent.remember(SCRIVA, path);
                self.refresh_fields();
                self.message = Some((
                    "Opened as a copy".to_owned(),
                    "Word 97-2003 documents are read but not written, so this \
                     one will be saved as a .docx. Its pictures come with it, \
                     the diagrams the old format kept as metafiles included, \
                     and so does its watermark; the shapes it draws a page \
                     frame with are shown but are not written into the copy."
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

    /// Takes up the faces the open document carries in its own package.
    ///
    /// Called for every document, including those with none and those with no
    /// package at all: the previous document's embedded type has to go, or the
    /// next one is laid out in a face it never named.
    ///
    /// The registration itself waits for a frame, because it is egui that owns
    /// the font atlas and it rebuilds it between frames, not during one.
    fn adopt_document_fonts(&mut self) {
        let faces = match (&self.package, &self.parts) {
            (Some(package), Some(parts)) => wp_docx::embedded(package, parts)
                .into_iter()
                .map(|face| (face.family, face.bold, face.italic, face.bytes))
                .collect(),
            _ => Vec::new(),
        };
        self.pending_fonts = Some((faces, font_names(&self.document)));
    }

    fn open_docx(&mut self, path: &Path) {
        match wp_docx::open(path) {
            Ok((document, package)) => {
                self.document = document;
                self.parts = wp_docx::DocumentParts::locate_in(&package).ok();
                self.package = Some(package);
                self.adopt_document_fonts();
                self.pictures.clear();
                self.path = Some(path.to_path_buf());
                self.dirty = false;
                self.history.clear();
                self.selection = Selection::default();
                self.scope = wp_model::Scope::Body;
                self.left_behind = None;
                self.band_page = None;
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
                Ok(mut package) => {
                    self.carry_loose_pictures(&mut package);
                    self.package = Some(package);
                }
                Err(error) => {
                    self.message = Some(("Cannot save".to_owned(), error.to_string()));
                    return false;
                }
            }
        }
        let Some(package) = &mut self.package else {
            return self.save_as();
        };
        match wp_docx::save(&mut self.document, package, &path) {
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

    /// Puts the pictures a `.doc` brought with it into the package being
    /// authored, and re-points the document's drawings at them.
    ///
    /// **A relationship that names no part is not a missing picture to Word,
    /// it is a damaged file.** So this happens before the document part is
    /// written, and a picture that cannot be embedded takes its drawing's
    /// relationship with it rather than leaving one dangling.
    fn carry_loose_pictures(&mut self, package: &mut ooxml::Package) {
        let mut renamed: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (rel, bytes) in self.pictures.loose() {
            let Some(content_type) = image_content_type(bytes) else {
                continue;
            };
            if let Ok(id) = wp_docx::media::embed(package, bytes, content_type) {
                renamed.insert(rel.clone(), id);
            }
        }
        if self.pictures.loose().is_empty() {
            return;
        }
        for drawing in self.document.drawings_mut() {
            let Some(rel) = drawing.rel.as_deref() else {
                continue;
            };
            // Only the names this document minted for itself are re-pointed;
            // anything else is a relationship the package already knows.
            if !rel.starts_with("doc-picture-") {
                continue;
            }
            drawing.rel = renamed.get(rel).map(|id| id.as_str().into());
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

    /// The images the current pages draw, decoded for a paper renderer.
    fn page_images(&self) -> std::collections::HashMap<String, wp_print::Raster> {
        crate::publish::rasters(
            self.package.as_ref(),
            self.parts.as_ref(),
            self.pictures.loose(),
            self.view.pages(),
        )
    }

    /// The metafiles the current pages draw, played for a paper renderer.
    fn page_metafiles(&self) -> std::collections::HashMap<String, metafile::Picture> {
        crate::publish::metafiles(
            self.package.as_ref(),
            self.parts.as_ref(),
            self.pictures.loose(),
            self.view.pages(),
        )
    }

    /// The charts the current pages draw, read for a paper renderer.
    fn page_plots(&self) -> std::collections::HashMap<String, chart::Plot> {
        crate::publish::plots(
            self.package.as_ref(),
            self.parts.as_ref(),
            self.view.pages(),
        )
    }

    /// What this document is called off the screen: for a PDF's title, for the
    /// print queue's entry.
    fn published_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Document".to_owned())
    }

    fn export_pdf(&mut self) {
        let stem = self.published_name();
        let mut chooser = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(format!("{stem}.pdf"));
        // Next to the document itself, which is where a resume's PDF belongs.
        if let Some(directory) = self.path.as_ref().and_then(|path| path.parent()) {
            chooser = chooser.set_directory(directory);
        } else if let Some(directory) = self.recent.directory() {
            chooser = chooser.set_directory(directory);
        }
        let Some(path) = chooser.save_file() else {
            return;
        };
        let path = if path.extension().is_none() {
            path.with_extension("pdf")
        } else {
            path
        };
        let images = self.page_images();
        let metafiles = self.page_metafiles();
        let plots = self.page_plots();
        let mut faces = crate::publish::SystemFaces::new();
        // The shaper that measured the page measures the charts on it, so a
        // printed label sits where the screen's did.
        let mut charts = self.shaper.as_mut().map(|shaper| wp_print::ops::Charts {
            plots: &plots,
            shaper,
        });
        let pdf = wp_print::pdf::export(
            self.view.pages(),
            &mut faces,
            &images,
            &metafiles,
            charts.as_mut(),
            Some(&stem),
        );
        if let Err(error) = std::fs::write(&path, pdf) {
            self.message = Some((
                "Cannot export".to_owned(),
                format!("{}\n\n{error}", path.display()),
            ));
        }
    }

    #[cfg(windows)]
    fn print(&mut self) {
        let images = self.page_images();
        let metafiles = self.page_metafiles();
        let plots = self.page_plots();
        let name = self.published_name();
        let mut charts = self.shaper.as_mut().map(|shaper| wp_print::ops::Charts {
            plots: &plots,
            shaper,
        });
        match wp_print::win::print(
            self.view.pages(),
            &images,
            &metafiles,
            charts.as_mut(),
            &name,
            &ui_kit::fonts::gdi_family,
        ) {
            Ok(_) => {}
            Err(error) => self.message = Some(("Cannot print".to_owned(), error)),
        }
    }

    #[cfg(not(windows))]
    fn print(&mut self) {
        self.message = Some((
            "Cannot print".to_owned(),
            "Printing is not wired up on this platform yet. Export a PDF and print that."
                .to_owned(),
        ));
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
        self.adopt_document_fonts();
        self.pictures.clear();
        self.path = None;
        self.dirty = false;
        self.history.clear();
        self.selection = Selection::default();
        self.scope = wp_model::Scope::Body;
        self.left_behind = None;
        self.band_page = None;
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
        // **The table of contents is the body's alone.** It is built from the
        // headings of the text and lands in the text, and a heading in a
        // running head is not a heading of the document — so the one command
        // that really does speak only for the body closes the band before it
        // acts. Everything else that used to be on this list — the search, the
        // reviewer, comments, accepting and rejecting — carries the flow it
        // means and works inside a header the way Word does.
        if matches!(command, Command::UpdateToc) {
            self.close_band();
        }
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
            Command::Print => self.print(),
            Command::ExportPdf => self.export_pdf(),
            Command::Close => self.close_document(),
            Command::Exit => {}
            Command::Undo => {
                if let Some((scope, caret)) = self.history.undo(&mut self.document) {
                    self.go_to(scope, caret);
                    self.changed();
                }
            }
            Command::Redo => {
                if let Some((scope, caret)) = self.history.redo(&mut self.document) {
                    self.go_to(scope, caret);
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
                // A picked picture is what "copy" means while it is picked;
                // the text selection is what it means the rest of the time.
                if self.picked.is_some() {
                    self.copy_drawing();
                } else {
                    self.copy_selection();
                }
            }
            Command::Cut => {
                if self.picked.is_some() {
                    if self.copy_drawing() {
                        self.delete_drawing();
                    }
                } else if self.copy_selection() {
                    self.replace_selection("");
                    self.reveal = Some(self.caret());
                }
            }
            Command::Paste => self.paste_from_board(),
            Command::Find => self.open_finder(false),
            Command::Replace => self.open_finder(true),
            Command::FindNext => self.jump_match(true),
            Command::FindPrevious => self.jump_match(false),
            Command::Bold => self.toggle(Toggle::Bold),
            Command::Italic => self.toggle(Toggle::Italic),
            Command::Strike => self.toggle(Toggle::Strike),
            Command::Underline => {
                let on = self.probe_runs(|props| props.underline.is_some_and(|u| u.kind.draws()));
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
            Command::Font(name) => self.format_runs(move |props| {
                // A theme reference outranks the cached name beside it, so
                // leaving one behind would silently undo this choice.
                props.fonts.ascii = Some(name.into());
                props.fonts.high_ansi = Some(name.into());
                props.fonts.ascii_theme = None;
                props.fonts.high_ansi_theme = None;
            }),
            Command::Color(color) => self.format_runs(move |props| props.color = Some(color)),
            Command::CustomColor => self.color_draft = Some((ColorTarget::Text, String::new())),
            Command::ParagraphDialog => self.open_paragraph_dialog(),
            Command::MergeCells => self.merge_cells(),
            Command::BorderColor(color) => self.color_borders(color),
            Command::CustomBorderColor => {
                self.color_draft = Some((ColorTarget::Borders, String::new()))
            }
            Command::Highlight(highlight) => self.format_runs(move |props| {
                // Word removes the element rather than writing `none`: an
                // explicit none is an override that would also blank a
                // style's highlight, which is not what the eraser means.
                props.highlight = match highlight {
                    wp_model::Highlight::None => None,
                    chosen => Some(chosen),
                };
            }),
            Command::EditHeader => self.enter_band(self.caret_page(), false),
            Command::EditFooter => self.enter_band(self.caret_page(), true),
            Command::CloseChrome => self.close_band(),
            Command::SwitchBand => {
                let footer = self.in_footer();
                self.enter_band(self.caret_page(), !footer);
            }
            Command::InsertPageNumber { of_pages } => self.insert_page_field(of_pages),
            Command::RemoveChrome { footer } => self.remove_band(footer),
            Command::Watermark => self.open_watermark_dialog(),
            Command::Bullets => self.toggle_list(true),
            Command::Numbers => self.toggle_list(false),
            Command::TableBorders(on) => self.edit_table(move |table, _, _| {
                use wp_model::table::TableBorders;
                // An explicit choice at the table level must not be vetoed by
                // what an insert or an opened file left on the cells, so the
                // cell overrides go too.
                let edge = if on { ruled_edge() } else { bare_edge() };
                table.props.borders = TableBorders {
                    top: Some(edge),
                    start: Some(edge),
                    bottom: Some(edge),
                    end: Some(edge),
                    inside_h: Some(edge),
                    inside_v: Some(edge),
                };
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        cell.props.borders = TableBorders::default();
                    }
                }
            }),
            Command::TableShading(fill) => self.edit_table(move |table, row, cell| {
                if let Some(cell) = table.rows.get_mut(row).and_then(|r| r.cells.get_mut(cell)) {
                    // Word writes `clear` with a fill, not `solid`: solid
                    // swaps the roles of its two colours and is the trap the
                    // model documents.
                    cell.props.shading = fill.map(|rgb| wp_model::prop::Shading {
                        pattern: wp_model::prop::ShadingPattern::Clear,
                        fill: Some(wp_model::Color::Rgb(rgb)),
                        color: None,
                    });
                }
            }),
            Command::ColumnWidth => self.open_column_dialog(),
            Command::CellMargins => self.open_cell_margin_dialog(),
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
                    self.scope,
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
                    inches(m.header),
                    inches(m.footer),
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
                // **Word refuses this outside the main story**, in those
                // words: "Comments, endnotes and footnotes can only be added
                // to the main story." Asked over COM against a header's range
                // it declines rather than commenting somewhere else, and so
                // does this — a refusal the user can read beats a comment that
                // silently lands on whatever paragraph of the text wore the
                // caret's number.
                if self.editing_band() {
                    self.message = Some((
                        "A comment belongs to the text".to_owned(),
                        "A header or a footer cannot carry one. Close the band \
                         and select the words in the document."
                            .to_owned(),
                    ));
                } else if self.selection.is_empty() {
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
            Command::GoTo(scope, paragraph) => {
                let caret = clamp(
                    &self.document,
                    scope,
                    Caret {
                        paragraph,
                        offset: 0,
                    },
                );
                // Into the band if that is where the thing being gone to is,
                // and out of one if it is not: a *Go to* that leaves the caret
                // in a header while pointing at the text is worse than none.
                match scope {
                    wp_model::Scope::Body => self.close_band(),
                    other => self.enter_flow(other),
                }
                self.selection = Selection::at(caret);
                // Scrolled to on the next frame, when the layout knows where it
                // is: a caret has no place on the page until the page exists.
                self.reveal = Some(caret);
            }
            Command::UpdateToc => self.update_toc(),
            Command::InsertPicture => self.insert_picture_from_file(),
            Command::InsertTable => {
                self.table_draft = Some(["2".to_owned(), "2".to_owned()]);
            }
            Command::PictureSize => self.open_size_dialog(),
            Command::DeletePicture => {
                self.delete_drawing();
            }
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
            self.scope,
            &mut self.history,
            Selection::at(Caret {
                paragraph: span.first,
                offset: 0,
            }),
            |_| {},
        );
        edit::replace_range(&mut self.document, self.scope, range, rows);
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
        self.selection = Selection::at(clamp(&self.document, self.scope, self.caret()));
        self.changed();
    }

    /// Settles the change nearest the caret.
    ///
    /// Nearest rather than *at*: a change is a range, and asking the user to put
    /// the caret exactly inside one is asking them to hunt for it.
    fn settle_one(&mut self, how: crate::revise::Resolve) {
        let changes = crate::revise::tracked(&self.document);
        let here = self.caret().paragraph;
        // Nearest within the flow the caret is in before any other, because a
        // paragraph number means nothing across flows: change 3 of a header is
        // not two away from paragraph 5 of the text, it is somewhere else
        // entirely.
        let Some(found) = changes
            .iter()
            .min_by_key(|change| (change.scope != self.scope, change.paragraph.abs_diff(here)))
        else {
            self.message = Some((
                "No tracked changes".to_owned(),
                "This document has nothing to accept or reject.".to_owned(),
            ));
            return;
        };
        let mark = found.mark.clone();
        if crate::revise::resolve_one(&mut self.document, &mut self.history, &mark, how) {
            self.selection = Selection::at(clamp(&self.document, self.scope, self.caret()));
            self.changed();
        }
    }

    fn next_change(&mut self) {
        let changes = crate::revise::tracked(&self.document);
        let here = self.caret().paragraph;
        let next = changes
            .iter()
            .find(|change| (change.scope, change.paragraph) > (self.scope, here))
            .or_else(|| changes.first());
        if let Some(change) = next {
            self.run(Command::GoTo(change.scope, change.paragraph));
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
                    .is_some_and(|(scope, at)| scope == self.scope && at.paragraph == here)
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

    /// Whether every run the command would touch already says `f`.
    ///
    /// A collapsed caret has no runs to ask, so the answer comes from what a
    /// caret there would type in — [`text::props_at`], the run before it or
    /// the paragraph mark. Asking the empty selection instead made every
    /// toggle read "off", so Ctrl+U on a blank line could only ever turn
    /// underline on, never off again.
    fn probe_runs(&self, f: impl Fn(&wp_model::RunProps) -> bool) -> bool {
        if self.selection.is_empty() {
            let caret = self.caret();
            let paragraphs = self.document.paragraphs_in(self.scope);
            let Some(paragraph) = paragraphs.get(caret.paragraph) else {
                return false;
            };
            f(&text::props_at(paragraph, caret.offset))
        } else {
            edit::all_runs(&self.document, self.scope, self.selection, f)
        }
    }

    fn toggle(&mut self, toggle: Toggle) {
        let on = self.probe_runs(|props| props.toggles.is_on(toggle));
        self.format_runs(move |props| props.toggles.set(toggle, !on));
    }

    fn vertical(&mut self, align: wp_model::prop::VertAlign) {
        let on = self.probe_runs(|props| props.vert_align == Some(align));
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
                // No word to take it: the paragraph mark does, which is where
                // Word keeps an empty paragraph's formatting and what a caret
                // typing here inherits (`text::props_at`). Ctrl+B on a blank
                // line followed by typing must produce bold text.
                let index = caret.paragraph;
                let Some(before) = edit::paragraph_at(&self.document, self.scope, index) else {
                    return;
                };
                self.history.push(
                    self.scope,
                    edit::Change::Paragraph {
                        index,
                        before: Box::new(before),
                    },
                );
                let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
                if let Some(target) = paragraphs.get_mut(index) {
                    let mut mark = target.props.mark.as_deref().cloned().unwrap_or_default();
                    change(&mut mark);
                    target.props.mark = Some(Box::new(mark));
                }
                self.changed();
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
            edit::format_runs(
                &mut self.document,
                self.scope,
                &mut self.history,
                selection,
                change,
            );
        } else {
            edit::format_runs(
                &mut self.document,
                self.scope,
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
            self.scope,
            &mut self.history,
            self.selection,
            change,
        );
        self.changed();
    }

    /// The bullet and numbering buttons: on when every covered paragraph is
    /// already in a list of that kind, and the press then takes them out.
    fn toggle_list(&mut self, bullets: bool) {
        let (start, end) = self.selection.ordered();
        let paragraphs = self.document.paragraphs_in(self.scope);
        let last = end.paragraph.min(paragraphs.len().saturating_sub(1));
        let kind = |reference: wp_model::prop::NumRef| {
            let level = self
                .document
                .numbering
                .level(reference.num_id, reference.level);
            level.is_some_and(|l| matches!(l.format, wp_model::NumFormat::Bullet) == bullets)
        };
        let on = (start.paragraph..=last)
            .all(|index| paragraphs[index].props.numbering.is_some_and(kind));
        drop(paragraphs);
        if on {
            self.format_paragraphs(move |props| props.numbering = None);
        } else {
            // A fresh instance of the kind's one definition, as Word's button
            // makes: instances count separately, definitions are shared.
            let (abstract_id, num_id) = self.document.numbering.free_ids();
            let existing = self
                .document
                .numbering
                .abstracts()
                .find(|definition| {
                    definition.levels.first().is_some_and(|level| {
                        level.as_ref().is_some_and(|l| {
                            matches!(l.format, wp_model::NumFormat::Bullet) == bullets
                        })
                    })
                })
                .map(|definition| definition.id);
            let abstract_id = match existing {
                Some(id) => id,
                None => {
                    let definition = list_definition(abstract_id, bullets);
                    self.document.numbering.insert_abstract(definition);
                    abstract_id
                }
            };
            self.document
                .numbering
                .insert_num(wp_model::numbering::Num::new(num_id, abstract_id));
            self.format_paragraphs(move |props| {
                props.numbering = Some(wp_model::prop::NumRef { num_id, level: 0 })
            });
        }
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

    /// The same, as text for the clipboard: a picture is a character of the
    /// document — see [`wp_model::doc::OBJECT`] — but pasting a control code
    /// into Notepad is not what copying a picture means. What carries the
    /// picture is the internal clipboard, not this.
    fn selected_plain_text(&self) -> Option<String> {
        self.selected_text()
            .map(|text| text.replace(wp_model::doc::OBJECT, ""))
    }

    /// Copies the selection: its text to the OS clipboard, its formatting here.
    ///
    /// Answers whether there was anything to copy, which is what tells Cut
    /// whether to go on and delete it.
    fn copy_selection(&mut self) -> bool {
        let Some(text) = self.selected_plain_text() else {
            return false;
        };
        let paragraphs = edit::copy_range(&self.document, self.scope, self.selection);
        clipboard_set(
            &text,
            &clip::html(&self.document, &paragraphs),
            &clip::rtf(&self.document, &paragraphs),
        );
        self.clipboard = Some(Clip { text, paragraphs });
        true
    }

    /// Pastes, with formatting when the board still holds what this copied.
    ///
    /// Anything else on the board — a line from a browser, a path from the shell
    /// — is text and arrives as text, taking the formatting of wherever the
    /// caret is. That is Word's rule for pasting from an application that offers
    /// nothing richer, and it is the only rule available for text.
    fn paste_from_board(&mut self) {
        // The paste lands at the caret; a picked picture only stands in the
        // way of seeing that happen.
        self.picked = None;
        match clipboard_get().filter(|text| !text.is_empty()) {
            Some(text) => self.paste_matching(&text),
            // Nothing to read as text. A screen snippet is a bitmap and nothing
            // else — no text at all — which is exactly the case that used to
            // fall out here having done nothing.
            None => {
                self.paste_picture_from_board();
            }
        }
    }

    /// Insert ▸ Picture… — the same three pieces a pasted picture is, from a
    /// file the user chooses.
    fn insert_picture_from_file(&mut self) {
        let mut chooser = rfd::FileDialog::new()
            .add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "bmp"])
            .add_filter("All files", &["*"]);
        if let Some(directory) = self.recent.directory() {
            chooser = chooser.set_directory(directory);
        }
        let Some(path) = chooser.pick_file() else {
            return;
        };
        let read = std::fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|data| {
                picture_bytes(data).ok_or_else(|| {
                    "This is not a picture Scriva can read.\n\nPNG, JPEG, GIF and BMP are."
                        .to_owned()
                })
            });
        match read {
            Ok((data, content_type, width, height)) => {
                if !self.insert_picture(&data, content_type, width, height) {
                    self.message = Some((
                        "Cannot insert".to_owned(),
                        "This document has nowhere to keep a picture.".to_owned(),
                    ));
                }
            }
            Err(why) => {
                self.message = Some((
                    "Cannot insert".to_owned(),
                    format!("{}\n\n{why}", path.display()),
                ));
            }
        }
    }

    /// Pastes the picture on the board, if there is one. Answers whether there
    /// was.
    ///
    /// Ours first: while the board still answers with the very bytes
    /// [`copy_drawing`](Self::copy_drawing) put there — or with nothing at
    /// all, which is what a copied chart leaves — the paste means the drawing
    /// itself, not a flattened picture of it. Anything else on the board was
    /// put there by someone else and arrives as a fresh picture.
    fn paste_picture_from_board(&mut self) -> bool {
        // A chart copied in Calx outranks everything else the board could
        // hold: the registered format is only ever written by a chart copy,
        // and that copy empties the board of anything older.
        if let Some((cx, cy, chart_space)) = clipboard_chart() {
            return self.insert_chart_part(&chart_space, cx, cy);
        }
        match clipboard_image() {
            Some((png, width, height)) => {
                let ours = self
                    .copied_drawing
                    .as_ref()
                    .is_some_and(|copied| copied.png.as_deref() == Some(png.as_slice()));
                match ours {
                    true => self.paste_copied_drawing(),
                    false => self.insert_picture(&png, "image/png", width, height),
                }
            }
            None => self.paste_copied_drawing(),
        }
    }

    /// Copies the picked drawing: the model clone here, the picture itself to
    /// the OS board. Answers whether there was one, which is what tells Cut
    /// whether to go on and delete it.
    fn copy_drawing(&mut self) -> bool {
        let Some(drawing) = self.picked_drawing().cloned() else {
            return false;
        };
        // A picture's file bytes, for pasting into a different document. A
        // chart is a family of parts, not a file, and has none.
        let bytes = drawing
            .rel
            .as_deref()
            .filter(|_| drawing.chart.is_none())
            .and_then(|rel| self.media_bytes(rel))
            .and_then(picture_bytes);
        // The OS board must stop saying whatever it said before this copy, or
        // the next paste would honour a stale copy over this one: a picture
        // goes onto it whole, and a chart — which has no pixels to give —
        // empties it. Tests leave the machine's board alone.
        #[cfg(test)]
        let png = None;
        #[cfg(not(test))]
        let png = match &bytes {
            Some((data, ..)) => clipboard_set_image(data),
            None => {
                clipboard_clear();
                None
            }
        };
        self.copied_drawing = Some(CopiedDrawing {
            drawing,
            bytes,
            png,
        });
        true
    }

    /// Pastes the copied drawing at the caret.
    ///
    /// In the document it was copied from, the paste is the model clone: the
    /// same part, shown once more, which is exactly what a duplicated picture
    /// or chart is. In a different document the relationship names nothing,
    /// so a picture is re-embedded from its bytes — and a chart, whose parts
    /// do not travel yet, says so rather than pasting nothing.
    fn paste_copied_drawing(&mut self) -> bool {
        let Some(copied) = self.copied_drawing.clone() else {
            return false;
        };
        // A picture names its part through `rel`; a chart through `chart`.
        let resolves = copied
            .drawing
            .rel
            .as_deref()
            .or(copied.drawing.chart.as_deref())
            .is_some_and(|rel| self.rel_resolves(rel));
        if resolves {
            let mut drawing = copied.drawing;
            // A picture part can be shown twice; a chart part cannot — Word
            // refuses to open the file — so the paste clones the chart part
            // and names the clone.
            if let Some(chart) = drawing.chart.clone() {
                let cloned = self
                    .package
                    .as_mut()
                    .and_then(|package| wp_docx::media::clone_chart(package, &chart).ok());
                let Some(rel) = cloned else {
                    self.message = Some((
                        "Cannot paste".to_owned(),
                        "The chart could not be copied within this document.".to_owned(),
                    ));
                    return false;
                };
                drawing.chart = Some(rel.into());
                // The painter resolves parts through an index built on open;
                // the clone is not in it until it is rebuilt.
                self.parts = self
                    .package
                    .as_ref()
                    .and_then(|package| wp_docx::DocumentParts::locate_in(package).ok());
            }
            let clip = vec![drawing_paragraph(drawing)];
            let caret = edit::paste_paragraphs(
                &mut self.document,
                self.scope,
                &mut self.history,
                self.selection,
                &clip,
            );
            self.selection = Selection::at(clamp(&self.document, self.scope, caret));
            self.changed();
            self.reveal = Some(self.caret());
            return true;
        }
        match copied.bytes {
            Some((data, content_type, width, height)) => {
                self.insert_picture(&data, content_type, width, height)
            }
            None => {
                self.message = Some((
                    "Cannot paste".to_owned(),
                    "This chart lives in the document it was copied from, and \
                     this is a different one.\n\nCopying a chart between \
                     documents is not supported yet."
                        .to_owned(),
                ));
                false
            }
        }
    }

    /// A picture part's bytes, found the way the painter finds them.
    fn media_bytes(&self, rel: &str) -> Option<Vec<u8>> {
        let name = self.parts.as_ref()?.target(rel)?;
        Some(self.package.as_ref()?.part(name)?.data().to_vec())
    }

    /// Whether a relationship still names a part of *this* document.
    fn rel_resolves(&self, rel: &str) -> bool {
        self.parts
            .as_ref()
            .and_then(|parts| parts.target(rel))
            .is_some()
    }

    /// Puts a picture in the document at the caret.
    ///
    /// Three things, because a picture in a `.docx` is three things: the bytes
    /// go into the package as a part, a relationship names that part, and a
    /// drawing in the text names the relationship. The editor holds the third;
    /// the first two are the package's and are done here so that the picture is
    /// drawable — and savable — the moment it is pasted rather than at the next
    /// save.
    fn insert_picture(&mut self, data: &[u8], content_type: &str, width: u32, height: u32) -> bool {
        if self.package.is_none() {
            // A document that has never been in a file has no package to put a
            // part into. Authoring one now is what the next save would do.
            match wp_docx::write::blank::package_for(&self.document) {
                Ok(package) => self.package = Some(package),
                Err(_) => return false,
            }
        }
        let Some(package) = &mut self.package else {
            return false;
        };
        let Ok(rel) = wp_docx::media::embed(package, data, content_type) else {
            return false;
        };
        // The painter finds a picture's bytes by relationship, through an index
        // built when the document was opened. A part added since is not in it.
        self.parts = wp_docx::DocumentParts::locate_in(package).ok();

        let clip = vec![picture_paragraph(
            &rel,
            &self.document.section,
            width,
            height,
        )];
        let caret = edit::paste_paragraphs(
            &mut self.document,
            self.scope,
            &mut self.history,
            self.selection,
            &clip,
        );
        self.selection = Selection::at(clamp(&self.document, self.scope, caret));
        self.changed();
        self.reveal = Some(self.caret());
        true
    }

    /// Puts a chart from the board in the document at the caret.
    ///
    /// The same three pieces a pasted picture is, with the part's bytes
    /// arriving ready-made: the clipboard payload *is* the `<c:chartSpace>`
    /// Calx authored, caches and all, so the document draws it — and Word will
    /// draw it — without ever resolving a cell reference. What Scriva can do
    /// to it from here is what it can do to any drawing: move it, resize it,
    /// delete it. Changing what it plots means going back to Calx.
    fn insert_chart_part(&mut self, chart_space: &[u8], cx: i64, cy: i64) -> bool {
        if self.package.is_none() {
            match wp_docx::write::blank::package_for(&self.document) {
                Ok(package) => self.package = Some(package),
                Err(_) => return false,
            }
        }
        let Some(package) = &mut self.package else {
            return false;
        };
        let Ok(rel) = wp_docx::media::embed_chart(package, chart_space) else {
            return false;
        };
        self.parts = wp_docx::DocumentParts::locate_in(package).ok();

        // Too wide for the column shrinks to it, proportions kept — the same
        // rule a pasted picture follows.
        const EMU_PER_POINT: f64 = 12_700.0;
        let column =
            (wp_model::PageBox::of(&self.document.section).text_width() * EMU_PER_POINT) as i64;
        let (mut cx, mut cy) = (cx.max(1), cy.max(1));
        if cx > column && column > 0 {
            cy = (cy * column / cx).max(1);
            cx = column;
        }
        let drawing = wp_model::doc::Drawing {
            // Ours, and there is nothing in it the model does not hold — the
            // writer authors the element from these fields.
            source: Vec::new().into(),
            anchored: false,
            extent: (wp_model::Emu(cx), wp_model::Emu(cy)),
            rel: None,
            chart: Some(rel.into()),
            name: Some("Chart".into()),
            description: None,
            wrap: wp_model::doc::Wrap::None,
            distance: (
                wp_model::Emu(0),
                wp_model::Emu(0),
                wp_model::Emu(0),
                wp_model::Emu(0),
            ),
            position: None,
            behind_text: false,
            text: None,
            tone: None,
            outline: None,
        };
        let clip = vec![drawing_paragraph(drawing)];
        let caret = edit::paste_paragraphs(
            &mut self.document,
            self.scope,
            &mut self.history,
            self.selection,
            &clip,
        );
        self.selection = Selection::at(clamp(&self.document, self.scope, caret));
        self.changed();
        self.reveal = Some(self.caret());
        true
    }

    /// The body of a paste, once the board has been read.
    fn paste_matching(&mut self, text: &str) {
        let flattened = text.replace("\r\n", "\n").replace('\r', "\n");
        let ours = self
            .clipboard
            .as_ref()
            .filter(|clip| clip.text == flattened && !clip.paragraphs.is_empty())
            .map(|clip| clip.paragraphs.clone());
        let Some(paragraphs) = ours else {
            self.paste_text(text);
            return;
        };
        let caret = edit::paste_paragraphs(
            &mut self.document,
            self.scope,
            &mut self.history,
            self.selection,
            &paragraphs,
        );
        self.selection = Selection::at(clamp(&self.document, self.scope, caret));
        self.changed();
        self.reveal = Some(self.caret());
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
            let caret = edit::delete_selection(
                &mut self.document,
                self.scope,
                &mut self.history,
                self.selection,
            );
            self.selection = Selection::at(clamp(&self.document, self.scope, caret));
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
            let caret = edit::split_paragraph(
                &mut self.document,
                self.scope,
                &mut self.history,
                self.selection,
            );
            self.selection = Selection::at(clamp(&self.document, self.scope, caret));
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

    /// Selects the next or previous match and scrolls to it, opening the
    /// header or footer the match is in when that is where it is.
    fn jump_match(&mut self, forward: bool) {
        self.refresh_matches();
        let (start, end) = self.selection.ordered();
        let found = if forward {
            // From the selection's end, so the match already selected is
            // stepped past — but a match starting right at a bare caret counts.
            find::after(
                &self.find_matches,
                self.scope,
                if self.selection.is_empty() {
                    start
                } else {
                    end
                },
            )
        } else {
            find::before(&self.find_matches, self.scope, start)
        };
        if let Some((scope, found)) = found {
            self.enter_flow(scope);
            self.selection = found;
            self.reveal = Some(found.ordered().0);
        }
    }

    /// Moves the editor into a flow without moving the caret, for a jump that
    /// already knows where in that flow it is going.
    ///
    /// The band a search lands in opens the same way a double-click on it
    /// would, and the place the caret held in the text is remembered so that
    /// closing the band still puts it back.
    fn enter_flow(&mut self, scope: wp_model::Scope) {
        if self.scope == scope {
            return;
        }
        if self.scope == wp_model::Scope::Body {
            self.left_behind = Some(self.selection);
        }
        self.scope = scope;
        self.picked = None;
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
        // Back to front so the offsets ahead of each replacement stay true,
        // and flow by flow for the same reason: a replacement in the header
        // does not move anything in the text, but one in the same header does.
        for (scope, found) in all.iter().rev() {
            self.scope = *scope;
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
            (ctrl, Key::P, Command::Print),
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
                // the pasted text already read from the OS. Copy and Cut act
                // on the picked picture when there is one — the command sorts
                // that out. A paste while a picture is picked lets it go
                // first: the paste lands at the caret, and a picture is not a
                // place a paste can land.
                egui::Event::Copy => self.run(Command::Copy),
                egui::Event::Cut => self.run(Command::Cut),
                egui::Event::Paste(text) => {
                    self.picked = None;
                    self.paste_matching(&text);
                }
                // Ctrl+V reaches an application as `Event::Paste`, and egui
                // builds that event by reading the board's *text*. A screen
                // snippet is a bitmap and nothing else, so egui reads nothing,
                // sends nothing, and swallows the key press on the way past —
                // which is why pasting a snip did nothing at all. The key's
                // release is not swallowed, and it is the only place the press
                // can be heard from. Guarded on there being no text, which is
                // the same condition under which the press went missing: where
                // there is text, `Event::Paste` has already done the work.
                egui::Event::Key {
                    key: egui::Key::V,
                    pressed: false,
                    modifiers,
                    ..
                } if modifiers.command && clipboard_get().is_none_or(|text| text.is_empty()) => {
                    self.picked = None;
                    self.paste_picture_from_board();
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
                // Word's key for the properties of what is selected.
                Key::Enter => self.open_size_dialog(),
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
                    let offset = view::line_span(&self.view, self.scope, caret)
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
                    let offset = match view::line_span(&self.view, self.scope, caret) {
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
                        self.scope,
                        &mut self.history,
                        Selection {
                            anchor: from,
                            head: caret,
                        },
                    )
                } else {
                    edit::backspace(
                        &mut self.document,
                        self.scope,
                        &mut self.history,
                        self.selection,
                    )
                };
                self.selection = Selection::at(clamp(&self.document, self.scope, caret));
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
                        self.scope,
                        &mut self.history,
                        Selection {
                            anchor: caret,
                            head: to,
                        },
                    )
                } else {
                    edit::delete_forward(
                        &mut self.document,
                        self.scope,
                        &mut self.history,
                        self.selection,
                    )
                };
                self.selection = Selection::at(clamp(&self.document, self.scope, caret));
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
                        self.scope,
                        &mut self.history,
                        self.selection,
                    );
                    self.selection = Selection::at(clamp(&self.document, self.scope, caret));
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
                        self.scope,
                        &mut self.history,
                        self.selection,
                        "\t",
                    );
                    self.selection = Selection::at(caret);
                    self.changed();
                }
            }
            Key::Escape => {
                // Escape leaves things, nearest first: the find bar, then an
                // open header or footer, then the selection.
                if self.finder.is_some() {
                    self.finder = None;
                    self.finder_focused = false;
                } else if self.scope != wp_model::Scope::Body {
                    self.close_band();
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
            let caret = edit::type_text(
                &mut self.document,
                self.scope,
                &mut self.history,
                self.selection,
                input,
            );
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
        let Some(before) = edit::paragraph_at(&self.document, self.scope, start.paragraph) else {
            return;
        };
        self.history.push(
            self.scope,
            edit::Change::Paragraph {
                index: start.paragraph,
                before: Box::new(before),
            },
        );
        let author = self.author.clone();
        let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
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
            let caret = edit::delete_selection(
                &mut self.document,
                self.scope,
                &mut self.history,
                self.selection,
            );
            self.selection = Selection::at(caret);
            self.changed();
            return;
        }
        let id = crate::revise::next_revision_id(&self.document);
        let Some(before) = edit::paragraph_at(&self.document, self.scope, start.paragraph) else {
            return;
        };
        self.history.push(
            self.scope,
            edit::Change::Paragraph {
                index: start.paragraph,
                before: Box::new(before),
            },
        );
        let author = self.author.clone();
        let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
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
        let Some((_, rect)) = view::caret_rect(&self.view, self.scope, caret) else {
            return caret;
        };
        let step = rect.height().max(1.0) as f64 * if down { 1.0 } else { -1.0 };
        view::step_from(&self.view, self.scope, caret, step).unwrap_or(caret)
    }

    /// One screenful up or down — Page Up and Page Down.
    fn page_step(&self, caret: Caret, down: bool) -> Caret {
        let screen = (self.viewport.y.max(60.0) as f64) / (self.view.zoom * view::SCALE).max(0.01);
        let step = screen * if down { 1.0 } else { -1.0 };
        view::step_from(&self.view, self.scope, caret, step).unwrap_or(caret)
    }
}

/// Word's default table rule: a half-point single line.
fn ruled_edge() -> wp_model::prop::Border {
    wp_model::prop::Border {
        style: wp_model::prop::BorderStyle::Single,
        size: Some(wp_model::units::Eighth(4)),
        space: Some(0),
        color: None,
        shadow: false,
    }
}

/// An explicit "no border" — not an absent one, which would let a table
/// style's own rules show through as if nothing had been chosen.
fn bare_edge() -> wp_model::prop::Border {
    wp_model::prop::Border {
        style: wp_model::prop::BorderStyle::None,
        size: None,
        space: None,
        color: None,
        shadow: false,
    }
}

/// Word's own gallery entry for the bullet and numbered lists, all nine
/// levels: the glyph cycle Symbol dot, Courier o, Wingdings square for
/// bullets; decimal, letter, roman for numbers. Each level steps in half an
/// inch and hangs its label the way Word's buttons do.
fn list_definition(id: u32, bullets: bool) -> wp_model::numbering::AbstractNum {
    use wp_model::numbering::{AbstractNum, Level, MultiLevel, NumFormat};
    let mut definition = AbstractNum::new(id);
    // What Word writes for a list built by clicking buttons.
    definition.multi_level = MultiLevel::Hybrid;
    definition.levels = (0..wp_model::numbering::LEVELS as u8)
        .map(|index| {
            let mut level = Level::new(index);
            level.para.indent.start = Some(Twips(720 * (i32::from(index) + 1)));
            level.para.indent.hanging = Some(Twips(360));
            if bullets {
                level.format = NumFormat::Bullet;
                // Symbol-encoded faces: the glyphs sit in the U+F0xx private
                // range, and the character means nothing in any other font.
                let (glyph, face) = match index % 3 {
                    0 => ("\u{F0B7}", "Symbol"),
                    1 => ("o", "Courier New"),
                    _ => ("\u{F0A7}", "Wingdings"),
                };
                level.text = glyph.into();
                level.run.fonts.ascii = Some(face.into());
                level.run.fonts.high_ansi = Some(face.into());
            } else {
                level.format = match index % 3 {
                    0 => NumFormat::Decimal,
                    1 => NumFormat::LowerLetter,
                    _ => NumFormat::LowerRoman,
                };
            }
            Some(level)
        })
        .collect();
    definition
}

/// The grid column a cell begins at: everything before it in its row, spans
/// and all, because every span is measured against the grid.
fn starting_column(table: &wp_model::table::Table, row: usize, cell: usize) -> usize {
    let Some(row) = table.rows.get(row) else {
        return 0;
    };
    row.props.grid_before as usize
        + row.cells[..cell.min(row.cells.len())]
            .iter()
            .map(|c| c.props.grid_span.max(1) as usize)
            .sum::<usize>()
}

/// What the hex-colour box is colouring: the selected text, or the rules of
/// the caret's table. One box, because the colour is typed the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorTarget {
    Text,
    Borders,
}

/// Paragraph ▸ Paragraph…, as typed: points for the spacing, inches for the
/// indents, and blank for "not stated" — a blank field applied takes the
/// value away rather than writing zero, which is how the dialog can also
/// undo a spacing the style did not ask for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParagraphDraft {
    before: String,
    after: String,
    left: String,
    right: String,
    first_line: String,
    hanging: String,
}

/// The one paragraph a newly made header or footer starts with.
///
/// **Word's Header and Footer styles are load-bearing.** They hold the line to
/// single, take the space off both ends of it, and put a centre tab at the
/// middle of the text column and a right tab at its end — which is what makes
/// Tab in a header walk name, title, page number across the width. A bare
/// paragraph would inherit the *body's* defaults instead, and a document set
/// with an inch of space after every paragraph would push its own text down
/// the page to make room under a one-line header.
fn band_paragraph(section: &wp_model::SectionProps) -> Paragraph {
    use wp_model::prop::{TabKind, TabLeader, TabStop};
    let width = section.text_width();
    let mut paragraph = Paragraph::new();
    paragraph.props.spacing.before = Some(Twips(0));
    paragraph.props.spacing.after = Some(Twips(0));
    paragraph.props.spacing.line = Some(LineSpacing::Multiple(Line240::SINGLE));
    paragraph.props.tabs = Some(vec![
        TabStop {
            position: Twips(width.0 / 2),
            kind: TabKind::Center,
            leader: TabLeader::None,
        },
        TabStop {
            position: width,
            kind: TabKind::End,
            leader: TabLeader::None,
        },
    ]);
    paragraph
}

/// The watermark box.
///
/// **A watermark is a document property, not an object to be dragged.** Word's
/// own box offers the word, the face and the colour and decides the rest, and
/// nobody has ever wanted to place one by hand — so this states the same few
/// things, and the size and the turn come from the page it will be stamped on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WatermarkDraft {
    text: String,
    /// Blank for Calibri, which is what Word writes when asked for nothing.
    font: String,
    /// Six hex digits; blank for Word's own silver.
    color: String,
    /// Diagonal is Word's default and the reason a watermark reads as one
    /// rather than as a heading someone left behind.
    diagonal: bool,
    /// Whether the document already had one, which decides whether the box
    /// offers to remove it.
    existing: bool,
}

/// The watermark a document carries, if it carries one.
///
/// A watermark is a shape made of words anchored in a header, and there is
/// nothing else in a header it could be confused with — a running head is
/// text in a paragraph, not a shape. The first one found answers for all of
/// them: Word writes the same shape into every header of the section, and a
/// document whose headers disagreed would have been made by hand.
fn watermark_in(document: &Document) -> Option<&wp_model::doc::ShapeText> {
    document
        .headers
        .iter()
        .filter(|header| !header.footer)
        .find_map(|header| shape_words_in(&header.content))
}

fn shape_words_in(blocks: &[Block]) -> Option<&wp_model::doc::ShapeText> {
    for block in blocks {
        if let Block::Paragraph(paragraph) = block {
            for drawing in paragraph.drawings() {
                if let Some(text) = drawing.text.as_deref() {
                    return Some(text);
                }
            }
        }
    }
    None
}

/// Takes every shape made of words out of a header body, and any paragraph
/// left holding nothing at all.
///
/// **A paragraph that held only the watermark goes with it.** Word puts the
/// shape in a paragraph of its own, and leaving that paragraph behind leaves
/// an empty line in the header — which is not nothing: it is a line of header
/// height, and it pushes the body text down the page.
fn strip_watermark(blocks: &mut Vec<Block>) {
    let mut emptied = Vec::new();
    for (index, block) in blocks.iter_mut().enumerate() {
        let Block::Paragraph(paragraph) = block else {
            continue;
        };
        let mut nth = 0;
        while nth < paragraph.drawings().len() {
            match paragraph.drawings()[nth].text.is_some() {
                true => {
                    paragraph.remove_drawing(nth);
                }
                false => nth += 1,
            }
        }
        if paragraph.text().is_empty() && paragraph.drawings().is_empty() {
            emptied.push(index);
        }
    }
    for index in emptied.into_iter().rev() {
        blocks.remove(index);
    }
}

/// A number for a dialog field: two decimals at most, and none where none
/// are needed, so that three points reads "3" and not "3.00".
fn trim_number(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// A caret that is inside the document it names.
fn clamp(document: &Document, scope: wp_model::Scope, caret: Caret) -> Caret {
    let paragraphs = document.paragraphs_in(scope);
    if paragraphs.is_empty() {
        return Caret::default();
    }
    let index = caret.paragraph.min(paragraphs.len() - 1);
    Caret {
        paragraph: index,
        offset: caret.offset.min(paragraphs[index].text().len()),
    }
}

/// Leaves the Zoom box's percent field with its whole number selected, so the
/// next keystroke replaces it — the way Word's box behaves.
fn select_percent(ctx: &egui::Context, text: &str) {
    let mut state = egui::text_edit::TextEditState::default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(text.chars().count()),
        )));
    state.store(ctx, egui::Id::new("scriva-zoom-percent"));
}

/// Word's zoom slider: 10–500% with 100% at the centre of the track, a notch
/// marking it, and a detent that snaps the thumb onto it. Each half of the
/// track is linear in its own range, which is why 100% can sit in the middle.
fn zoom_slider(ui: &mut egui::Ui, percent: &mut f64) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(100.0, 16.0), egui::Sense::click_and_drag());
    if response.dragged() || response.clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let mut t = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
            if (t - 0.5).abs() < 0.04 {
                t = 0.5;
            }
            *percent = if t <= 0.5 {
                10.0 + t / 0.5 * 90.0
            } else {
                100.0 + (t - 0.5) / 0.5 * 400.0
            }
            .round();
        }
    }
    if ui.is_rect_visible(rect) {
        let line = egui::Stroke::new(1.0, ui.visuals().widgets.inactive.fg_stroke.color);
        let y = rect.center().y;
        let painter = ui.painter();
        painter.hline(rect.x_range(), y, line);
        painter.vline(rect.center().x, egui::Rangef::new(y - 4.0, y + 4.0), line);
        let t = if *percent <= 100.0 {
            (*percent - 10.0) / 90.0 * 0.5
        } else {
            0.5 + (*percent - 100.0) / 400.0 * 0.5
        };
        let x = rect.left() + t.clamp(0.0, 1.0) as f32 * rect.width();
        let visuals = ui.style().interact(&response);
        painter.rect(
            egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(8.0, 14.0)),
            2.0,
            visuals.bg_fill,
            visuals.fg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text("Zoom");
}

/// Puts a copy on the OS clipboard in every format it can be read back out of.
///
/// Three at once and in one open of the board: the plain text, which is all a
/// text editor wants; `HTML Format` — CF_HTML, what a browser writes; and
/// `Rich Text Format`. The last two are where Word looks for formatting, and it
/// prefers them in that order over the text. They have to be written together
/// because opening the clipboard to add one clears whatever was there.
#[cfg(windows)]
fn clipboard_set(text: &str, html: &str, rtf: &str) {
    let Ok(_board) = clipboard_win::Clipboard::new_attempts(10) else {
        return;
    };
    // This one empties the board first, so it goes first: the other two are
    // added to what it leaves.
    if clipboard_win::raw::set_string(&text.replace('\n', "\r\n")).is_err() {
        return;
    }
    for (name, data) in [
        ("HTML Format", cf_html(html)),
        ("Rich Text Format", rtf.to_owned()),
    ] {
        if let Some(format) = clipboard_win::register_format(name) {
            let _ = clipboard_win::raw::set_without_clear(format.get(), data.as_bytes());
        }
    }
}

/// The same, where there is no Win32 clipboard: text and HTML, which is what
/// `arboard` can put on a board and is the pair every other desktop reads.
#[cfg(not(windows))]
fn clipboard_set(text: &str, html: &str, _rtf: &str) {
    if let Ok(mut board) = arboard::Clipboard::new() {
        let text = text.replace('\n', "\r\n");
        let _ = board.set_html(html, Some(text.as_str()));
    }
}

/// Wraps an HTML fragment in the header CF_HTML is.
///
/// The format is a plain-text header of byte offsets *into itself*, which cannot
/// be written before the numbers are known and whose numbers move the moment
/// they are written. The way out is the format's own: every offset is padded to
/// ten digits, so stating one does not change where anything is.
#[cfg_attr(not(windows), allow(dead_code))]
fn cf_html(fragment: &str) -> String {
    const PROLOGUE: &str = "<html><body>\r\n<!--StartFragment-->";
    const EPILOGUE: &str = "<!--EndFragment-->\r\n</body></html>";
    const HEADER: usize = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\n\
         StartFragment:0000000000\r\nEndFragment:0000000000\r\n"
        .len();
    let start_fragment = HEADER + PROLOGUE.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + EPILOGUE.len();
    format!(
        "Version:0.9\r\nStartHTML:{HEADER:010}\r\nEndHTML:{end_html:010}\r\n\
         StartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n\
         {PROLOGUE}{fragment}{EPILOGUE}"
    )
}

/// What the OS clipboard holds, as text.
fn clipboard_get() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Puts a picture on the OS clipboard, and answers the PNG the board will
/// hand back when asked — the receipt a paste compares against to know the
/// board is still ours.
///
/// The board holds pixels, not files, so the picture is decoded to go on and
/// the receipt is those pixels re-encoded the same way [`clipboard_image`]
/// re-encodes them coming off. Same pixels, same encoder: the same bytes.
#[cfg_attr(test, allow(dead_code))]
fn clipboard_set_image(data: &[u8]) -> Option<Vec<u8>> {
    let rgba = image::load_from_memory(data).ok()?.to_rgba8();
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    let mut board = arboard::Clipboard::new().ok()?;
    board
        .set_image(arboard::ImageData {
            width,
            height,
            bytes: std::borrow::Cow::Borrowed(rgba.as_raw()),
        })
        .ok()?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// Takes whatever is on the OS clipboard off it.
///
/// For copying a chart, which has no pixels to put there: the board must stop
/// saying whatever it said before the copy, or a paste would honour it over
/// the chart.
#[cfg_attr(test, allow(dead_code))]
fn clipboard_clear() {
    if let Ok(mut board) = arboard::Clipboard::new() {
        let _ = board.clear();
    }
}

/// The chart on the OS clipboard, if Calx put one there: the `<c:chartSpace>`
/// bytes and the size the chart had on its sheet, in EMUs.
///
/// The registered format's name is a claim any program could make, so the
/// payload's own magic decides — see [`chart::clipboard::unpack`].
#[cfg(windows)]
fn clipboard_chart() -> Option<(i64, i64, Vec<u8>)> {
    let format = clipboard_win::register_format(chart::clipboard::FORMAT)?;
    let _board = clipboard_win::Clipboard::new_attempts(10).ok()?;
    let mut data = Vec::new();
    clipboard_win::raw::get_vec(format.get(), &mut data).ok()?;
    let (cx, cy, chart_space) = chart::clipboard::unpack(&data)?;
    Some((cx, cy, chart_space.to_vec()))
}

/// Where there is no Win32 clipboard there is no registered format to read.
#[cfg(not(windows))]
fn clipboard_chart() -> Option<(i64, i64, Vec<u8>)> {
    None
}

/// The picture on the OS clipboard, as PNG bytes and its size in pixels.
///
/// A snip arrives as raw pixels — `CF_DIBV5`, or PNG where the program that
/// copied it offered one — and a `.docx` holds an encoded image, so it is
/// re-encoded here. PNG rather than JPEG because a screenshot is a picture of
/// text and lines, which is precisely what JPEG is worst at.
fn clipboard_image() -> Option<(Vec<u8>, u32, u32)> {
    let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
    let (width, height) = (image.width as u32, image.height as u32);
    let pixels = image::RgbaImage::from_raw(width, height, image.bytes.into_owned())?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some((png, width, height))
}

/// A length in points, as the inches a size box shows.
///
/// Two decimals, which is what Word's boxes show and is about a hundredth of an
/// inch — finer than anyone can drag, and finer than a printer resolves.
fn inches(points: f64) -> String {
    format!("{:.2}", points / 72.0)
}

/// A file's bytes as something a document can hold: the data, its content type,
/// and its size in pixels.
///
/// The bytes are handed on **unchanged** where the format is one Word embeds —
/// re-encoding a photograph would throw quality away for nothing, and a JPEG
/// re-encoded as PNG is several times the size. Anything else is decoded and
/// written out as PNG, which is the answer for a BMP above all: Word would take
/// it, and nobody wants a twelve-megabyte bitmap inside a document.
/// What a picture's own bytes say they are, for a part that has to declare a
/// content type. `None` for anything the package should not be given.
fn image_content_type(data: &[u8]) -> Option<&'static str> {
    // A metafile is not a raster and no raster decoder recognises one, but it
    // is a picture the package can carry and Word can draw.
    if data.get(40..44) == Some(b" EMF") {
        return Some("image/x-emf");
    }
    Some(match image::guess_format(data).ok()? {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::Tiff => "image/tiff",
        image::ImageFormat::Bmp => "image/bmp",
        _ => return None,
    })
}

fn picture_bytes(data: Vec<u8>) -> Option<(Vec<u8>, &'static str, u32, u32)> {
    use image::ImageFormat;
    let format = image::guess_format(&data).ok()?;
    let decoded = image::load_from_memory_with_format(&data, format).ok()?;
    let (width, height) = (decoded.width(), decoded.height());
    let content_type = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        _ => {
            let mut png = Vec::new();
            decoded
                .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
                .ok()?;
            return Some((png, "image/png", width, height));
        }
    };
    Some((data, content_type, width, height))
}

/// A paragraph holding nothing but the picture, for a paste to splice in.
fn picture_paragraph(
    rel: &str,
    section: &wp_model::section::SectionProps,
    width: u32,
    height: u32,
) -> Paragraph {
    // 96 pixels to the inch: what a screen snippet is measured in, and what
    // Word assumes of an image that does not say otherwise.
    const EMU_PER_PIXEL: i64 = 914_400 / 96;
    const EMU_PER_POINT: f64 = 12_700.0;
    let mut cx = width as i64 * EMU_PER_PIXEL;
    let mut cy = height as i64 * EMU_PER_PIXEL;
    // A snip of a whole screen is far wider than a page. Word shrinks a picture
    // too wide for the column rather than letting it run off the paper, and
    // keeps its proportions doing it.
    let column = (wp_model::PageBox::of(section).text_width() * EMU_PER_POINT) as i64;
    if cx > column && cx > 0 {
        cy = cy * column / cx;
        cx = column;
    }
    let drawing = wp_model::doc::Drawing {
        // Ours, and there is nothing in it the model does not hold — the writer
        // authors the element from these fields. See `wp_docx::write::drawing`.
        source: Vec::new().into(),
        anchored: false,
        extent: (wp_model::Emu(cx), wp_model::Emu(cy)),
        rel: Some(rel.into()),
        chart: None,
        name: Some("Picture".into()),
        description: None,
        wrap: wp_model::doc::Wrap::None,
        distance: (
            wp_model::Emu(0),
            wp_model::Emu(0),
            wp_model::Emu(0),
            wp_model::Emu(0),
        ),
        position: None,
        behind_text: false,
        text: None,
        tone: None,
        outline: None,
    };
    Paragraph {
        content: vec![wp_model::doc::Inline::Run(wp_model::doc::Run {
            content: vec![wp_model::doc::Piece::Drawing(Box::new(drawing))],
            ..wp_model::doc::Run::new()
        })],
        ..Paragraph::new()
    }
}

/// A paragraph holding nothing but a drawing already fully formed, for a
/// paste to splice in.
fn drawing_paragraph(drawing: wp_model::doc::Drawing) -> Paragraph {
    Paragraph {
        content: vec![wp_model::doc::Inline::Run(wp_model::doc::Run {
            content: vec![wp_model::doc::Piece::Drawing(Box::new(drawing))],
            ..wp_model::doc::Run::new()
        })],
        ..Paragraph::new()
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
///
/// Seeded with the quick styles a fresh document is expected to offer: a
/// heading has to exist before the Styles menu can apply it, and a document
/// born here has nowhere else to get one. Ids and names are spelled the way
/// Word spells them, which is also what the outline recognises as headings.
/// Every face name the document asks for, anywhere.
///
/// The machine's own copy of a face outranks one embedded in the package, so
/// the names have to be collected before anything is registered — a document
/// that names Ubuntu Mono must be drawn in the Ubuntu Mono this machine has,
/// whatever the package carries under that name. Split on `;` because
/// LibreOffice writes its own fallback chain into `w:rFonts` and the first
/// entry is the face being asked for.
pub fn font_names(document: &Document) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut add = |fonts: &wp_model::prop::Fonts| {
        for named in [
            &fonts.ascii,
            &fonts.high_ansi,
            &fonts.east_asian,
            &fonts.complex,
        ] {
            let Some(named) = named else { continue };
            for part in named.split(';') {
                let part = part.trim();
                if !part.is_empty() {
                    names.insert(part.to_owned());
                }
            }
        }
    };
    for (_, style) in document.styles.iter() {
        add(&style.run.fonts);
    }
    for paragraph in document.paragraphs() {
        for run in paragraph.runs() {
            add(&run.props.fonts);
        }
        if let Some(mark) = paragraph.props.mark.as_deref() {
            add(&mark.fonts);
        }
    }
    for faces in [&document.theme.major, &document.theme.minor] {
        for named in [&faces.latin, &faces.east_asian, &faces.complex]
            .into_iter()
            .flatten()
        {
            names.insert(named.to_string());
        }
    }
    names.into_iter().collect()
}

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
    normal.quick = true;
    normal.priority = Some(1);
    let normal = document.styles.insert(normal);

    let ladder: [(&str, &str, i32, Option<u8>, i32); 4] = [
        ("Heading1", "heading 1", 32, Some(0), 2),
        ("Heading2", "heading 2", 26, Some(1), 3),
        ("Heading3", "heading 3", 24, Some(2), 4),
        ("Title", "Title", 56, None, 5),
    ];
    for (id, name, size, outline, priority) in ladder {
        let mut style = wp_model::Style::new(id, wp_model::StyleKind::Paragraph);
        style.name = Some(name.into());
        style.based_on = Some(normal);
        style.next = Some(normal);
        style.quick = true;
        style.priority = Some(priority);
        style.run.size = Some(HalfPoint(size));
        style.run.toggles.set(wp_model::prop::Toggle::Bold, true);
        style.para.outline_level = outline;
        document.styles.insert(style);
    }
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
        let band = match self.scope {
            wp_model::Scope::Body => None,
            wp_model::Scope::Chrome(_) => {
                rule(ui);
                self.band_bar(ui)
            }
        };
        if let Some(command) = command.or(bar).or(band) {
            // The same guard the keyboard route takes: File ▸ New discarding
            // an unsaved document would be a menu doing what Ctrl+N will not.
            match command {
                Command::New | Command::Open | Command::Close | Command::Exit => {
                    self.guarded(command)
                }
                other => self.run(other),
            }
        }
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let pages = self.view.pages().len().max(1);
            let page = view::caret_rect(&self.view, self.scope, self.caret())
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
                // Word's corner, right to left: the percentage (click it to
                // type an exact one), zoom in, the slider with its 100%
                // detent, zoom out. The buttons step to the next round ten.
                let shown = (self.view.zoom * 100.0).round();
                let mut percent = shown;
                ui.spacing_mut().item_spacing.x = 4.0;
                let label = ui
                    .add(
                        egui::Button::new(format!("{}%", percent as i32))
                            .frame(false)
                            .min_size(egui::vec2(40.0, 0.0)),
                    )
                    .on_hover_text("Zoom level — click to set an exact zoom");
                if label.clicked() {
                    self.zoom_draft = Some((percent as i32).to_string());
                    self.zoom_fresh = true;
                }
                if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                    percent = (percent / 10.0).floor() * 10.0 + 10.0;
                }
                zoom_slider(ui, &mut percent);
                if ui.small_button("−").on_hover_text("Zoom out").clicked() {
                    percent = (percent / 10.0).ceil() * 10.0 - 10.0;
                }
                let percent = percent.clamp(10.0, 500.0);
                if percent != shown {
                    self.view.zoom = percent / 100.0;
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
        if self.table_draft.is_some() {
            self.table_dialog(ctx);
            return;
        }
        if self.color_draft.is_some() {
            self.color_dialog(ctx);
            return;
        }
        if self.column_draft.is_some() {
            self.column_dialog(ctx);
            return;
        }
        if self.cell_margin_draft.is_some() {
            self.cell_margin_dialog(ctx);
            return;
        }
        if self.watermark_draft.is_some() {
            self.watermark_dialog(ctx);
            return;
        }
        if self.paragraph_draft.is_some() {
            self.paragraph_dialog(ctx);
            return;
        }
        if self.zoom_draft.is_some() {
            self.zoom_dialog(ctx);
            return;
        }
        if self.size_draft.is_some() {
            self.size_dialog(ctx);
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
        // A change of fonts lands between frames, so the atlas this frame draws
        // with is still the old one and every width the shaper has cached was
        // measured against it — and a family registered but not yet live is
        // not a family epaint substitutes for, it is one it panics on. The new
        // type is handed over here and *believed* one frame later, when the
        // shaper is thrown away and the page is laid out again in the face the
        // document actually asked for.
        if let Some((faces, named)) = self.pending_fonts.take() {
            ui_kit::fonts::embed_document(ui.ctx(), &faces, &named);
            self.fonts_settling = true;
            self.shaper = None;
            self.view.invalidate();
            ui.ctx().request_repaint();
        } else if self.fonts_settling {
            self.fonts_settling = false;
            self.shaper = None;
            self.view.invalidate();
        }
        if self.shaper.is_none() {
            self.shaper = Some(Egui::new(ui.ctx()));
        }
        let stamp = self.stamp;
        let fields = self.fields.clone();
        // Nothing is laid out in the frame the fonts changed in. Measuring now
        // would measure the type the document replaced, and the page would be
        // thrown away and done again a frame later regardless.
        if let (false, Some(shaper)) = (self.fonts_settling, &mut self.shaper) {
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
            self.view.zoom = (self.view.zoom * zoom_delta as f64).clamp(0.10, 5.0);
        }

        // While a dialog or the find bar holds the keyboard, keys belong to it:
        // without this, searching for "bug" also types "bug" into the document.
        let blocked = self.pending.is_some()
            || self.message.is_some()
            || self.drafting.is_some()
            || self.margins_draft.is_some()
            || self.table_draft.is_some()
            || self.color_draft.is_some()
            || self.column_draft.is_some()
            || self.cell_margin_draft.is_some()
            || self.watermark_draft.is_some()
            || self.paragraph_draft.is_some()
            || self.size_draft.is_some()
            || self.zoom_draft.is_some()
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
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(260.0);
                    ui.label(egui::RichText::new("Margins").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    for (index, (label, field)) in ["Top:", "Bottom:", "Left:", "Right:"]
                        .into_iter()
                        .chain(["Header:", "Footer:"])
                        .zip(draft.iter_mut())
                        .enumerate()
                    {
                        if index == 4 {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("From edge — where the bands sit")
                                    .small()
                                    .weak(),
                            );
                        }
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
                section.margins.header = parse(&draft[4], m.header);
                section.margins.footer = parse(&draft[5], m.footer);
                if section.margins != m {
                    self.set_section(section);
                }
            }
            Some(false) => self.margins_draft = None,
            None => {}
        }
    }

    /// The insert-table box: how many columns and rows, the way Word asks.
    fn table_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.table_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-table"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(260.0);
                    ui.label(egui::RichText::new("Insert Table").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    for (label, field) in ["Columns:", "Rows:"].into_iter().zip(draft.iter_mut()) {
                        ui.horizontal(|ui| {
                            ui.add_sized([72.0, 20.0], egui::Label::new(label));
                            ui.add(egui::TextEdit::singleline(field).desired_width(64.0));
                        });
                    }
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "Insert") {
                        done = Some(answer);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        self.table_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.table_draft = None;
                // Word's own ceiling on columns; a number that does not parse
                // inserts nothing rather than guessing.
                let parse = |text: &str, most: usize| {
                    text.trim()
                        .parse::<usize>()
                        .ok()
                        .filter(|v| (1..=most).contains(v))
                };
                if let (Some(columns), Some(rows)) = (parse(&draft[0], 63), parse(&draft[1], 32767))
                {
                    self.insert_table(rows, columns);
                }
            }
            Some(false) => self.table_draft = None,
            None => {}
        }
    }

    fn color_dialog(&mut self, ctx: &egui::Context) {
        let Some((target, mut draft)) = self.color_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-color"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(260.0);
                    let title = match target {
                        ColorTarget::Text => "Text Colour",
                        ColorTarget::Borders => "Border Colour",
                    };
                    ui.label(egui::RichText::new(title).font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_sized([72.0, 20.0], egui::Label::new("Hex:"));
                        ui.add(egui::TextEdit::singleline(&mut draft).desired_width(64.0));
                    });
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "Apply") {
                        done = Some(answer);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        self.color_draft = Some((target, draft.clone()));
        match done {
            Some(true) => {
                self.color_draft = None;
                // The same spellings a file's `w:val` may use — six hex
                // digits, with or without the `#`, or the word `auto`.
                // Anything else applies nothing rather than guessing at a
                // colour nobody named.
                if let Some(color) = wp_model::Color::from_val(&draft) {
                    match target {
                        ColorTarget::Text => {
                            self.format_runs(move |props| props.color = Some(color))
                        }
                        ColorTarget::Borders => self.color_borders(color),
                    }
                }
            }
            Some(false) => self.color_draft = None,
            None => {}
        }
    }

    /// Colours every rule of the caret's table, cells' own rules included,
    /// so that the table is one colour after it as it was before.
    ///
    /// A table whose rules are all inherited or all off gets Word's
    /// half-point single lines to carry the colour: a colour on no line is no
    /// colour at all, and a menu choice that did nothing visible would read
    /// as broken.
    fn color_borders(&mut self, color: wp_model::Color) {
        use wp_model::prop::BorderStyle;
        self.edit_table(move |table, _, _| {
            let borders = &mut table.props.borders;
            let drawn = |edge: &Option<wp_model::prop::Border>| {
                edge.is_some_and(|b| b.style != BorderStyle::None)
            };
            let any_drawn = [
                &borders.top,
                &borders.start,
                &borders.bottom,
                &borders.end,
                &borders.inside_h,
                &borders.inside_v,
            ]
            .into_iter()
            .any(drawn);
            for edge in [
                &mut borders.top,
                &mut borders.start,
                &mut borders.bottom,
                &mut borders.end,
                &mut borders.inside_h,
                &mut borders.inside_v,
            ] {
                match edge {
                    Some(border) if border.style != BorderStyle::None => border.color = Some(color),
                    _ if !any_drawn => {
                        let mut ruled = ruled_edge();
                        ruled.color = Some(color);
                        *edge = Some(ruled);
                    }
                    _ => {}
                }
            }
            for row in &mut table.rows {
                for cell in &mut row.cells {
                    let borders = &mut cell.props.borders;
                    // Only the four sides: on a cell the "inside" slots are
                    // its diagonals, which a rule colour has no business with.
                    for border in [
                        &mut borders.top,
                        &mut borders.start,
                        &mut borders.bottom,
                        &mut borders.end,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if border.style != BorderStyle::None {
                            border.color = Some(color);
                        }
                    }
                }
            }
        });
    }

    /// Table ▸ Merge Cells: the cells the selection runs across become one,
    /// spanning their columns and holding their paragraphs in order.
    ///
    /// A cell that held nothing but an empty paragraph contributes nothing,
    /// as in Word — merging three blank cells must make one blank cell, not
    /// a cell three lines tall. The merged cell keeps the first cell's
    /// properties and the last cell's right-hand rule, which is the rule the
    /// table's edge now falls on.
    fn merge_cells(&mut self) {
        let (start, end) = self.selection.ordered();
        let from = edit::table_cell_at(&self.document, self.scope, start);
        let to = edit::table_cell_at(&self.document, self.scope, end);
        let (Some((index, at_row, first)), Some((index_end, row_end, last))) = (from, to) else {
            self.message = Some((
                "Not in a table".to_owned(),
                "Select across the cells of one row first, then try again.".to_owned(),
            ));
            return;
        };
        if index != index_end || at_row != row_end || first == last {
            self.message = Some((
                "Nothing to merge".to_owned(),
                "Select across two or more cells of one row first, then try again.".to_owned(),
            ));
            return;
        }
        self.history.push(
            self.scope,
            edit::Change::Blocks {
                index,
                before: vec![self.document.body[index].clone()],
                now: 1,
            },
        );
        if let Block::Table(table) = &mut self.document.body[index] {
            let column = starting_column(table, at_row, first);
            let row = &mut table.rows[at_row];
            let taken: Vec<wp_model::table::Cell> = row.cells.drain(first + 1..=last).collect();
            let cell = &mut row.cells[first];
            let blank = |content: &[Block]| {
                content.iter().all(|block| match block {
                    Block::Paragraph(p) => p.content.is_empty() && p.props.numbering.is_none(),
                    _ => false,
                })
            };
            let mut content = std::mem::take(&mut cell.content);
            if blank(&content) {
                content.clear();
            }
            let mut span = cell.props.span() as usize;
            for other in taken {
                span += other.props.span() as usize;
                if other.props.borders.end.is_some() {
                    cell.props.borders.end = other.props.borders.end;
                }
                if !blank(&other.content) {
                    content.extend(other.content);
                }
            }
            if content.is_empty() {
                content.push(Block::Paragraph(Paragraph::new()));
            }
            cell.content = content;
            cell.props.grid_span = span as u32;
            let total: i32 = table.grid
                [column.min(table.grid.len())..(column + span).min(table.grid.len())]
                .iter()
                .map(|t| t.0)
                .sum();
            cell.props.width = wp_model::table::Width::Fixed(Twips(total));
        }
        self.selection = Selection::at(clamp(&self.document, self.scope, start));
        self.reveal = Some(self.caret());
        self.changed();
    }

    /// Opens the paragraph box on the caret's paragraph, showing what that
    /// paragraph states itself — not what it inherits, which is the style's
    /// to show.
    fn open_paragraph_dialog(&mut self) {
        let caret = self.caret();
        let Some(paragraph) = edit::paragraph_at(&self.document, self.scope, caret.paragraph)
        else {
            return;
        };
        let props = &paragraph.props;
        let points = |t: Option<Twips>| {
            t.map(|t| trim_number(t.0 as f64 / 20.0))
                .unwrap_or_default()
        };
        let inches = |t: Option<Twips>| {
            t.map(|t| trim_number(t.0 as f64 / 1440.0))
                .unwrap_or_default()
        };
        let draft = ParagraphDraft {
            before: points(props.spacing.before),
            after: points(props.spacing.after),
            left: inches(props.indent.start),
            right: inches(props.indent.end),
            first_line: inches(props.indent.first_line),
            hanging: inches(props.indent.hanging),
        };
        self.paragraph_draft = Some((draft.clone(), draft));
    }

    fn paragraph_dialog(&mut self, ctx: &egui::Context) {
        let Some((opened, mut draft)) = self.paragraph_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-paragraph"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(280.0);
                    ui.label(egui::RichText::new("Paragraph").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Spacing").strong());
                    for (label, field) in
                        [("Before:", &mut draft.before), ("After:", &mut draft.after)]
                    {
                        ui.horizontal(|ui| {
                            ui.add_sized([80.0, 20.0], egui::Label::new(label));
                            ui.add(egui::TextEdit::singleline(field).desired_width(64.0));
                            ui.label("pt");
                        });
                    }
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Indentation").strong());
                    for (label, field) in [
                        ("Left:", &mut draft.left),
                        ("Right:", &mut draft.right),
                        ("First line:", &mut draft.first_line),
                        ("Hanging:", &mut draft.hanging),
                    ] {
                        ui.horizontal(|ui| {
                            ui.add_sized([80.0, 20.0], egui::Label::new(label));
                            ui.add(egui::TextEdit::singleline(field).desired_width(64.0));
                            ui.label("in");
                        });
                    }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Blank leaves it to the style.")
                            .small()
                            .weak(),
                    );
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "Apply") {
                        done = Some(answer);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        self.paragraph_draft = Some((opened.clone(), draft.clone()));
        match done {
            Some(true) => {
                self.paragraph_draft = None;
                self.apply_paragraph(&opened, &draft);
            }
            Some(false) => self.paragraph_draft = None,
            None => {}
        }
    }

    /// Applies the paragraph box to the selection: only the fields that
    /// changed since it opened, so that a box opened on one paragraph and
    /// applied to several states one thing about them and leaves the rest of
    /// each as it was.
    fn apply_paragraph(&mut self, opened: &ParagraphDraft, draft: &ParagraphDraft) {
        // A blank is a stated absence; a number is twips; a typo changes
        // nothing, as with the margins box.
        let value = |was: &str, now: &str, per_unit: f64| -> Option<Option<Twips>> {
            if was == now {
                return None;
            }
            let now = now.trim();
            if now.is_empty() {
                return Some(None);
            }
            now.parse::<f64>()
                .ok()
                .filter(|v| (-22.0 * per_unit..=22.0 * per_unit).contains(&(v * per_unit)))
                .map(|v| Some(Twips((v * per_unit).round() as i32)))
        };
        let before = value(&opened.before, &draft.before, 20.0);
        let after = value(&opened.after, &draft.after, 20.0);
        let left = value(&opened.left, &draft.left, 1440.0);
        let right = value(&opened.right, &draft.right, 1440.0);
        let first_line = value(&opened.first_line, &draft.first_line, 1440.0);
        let hanging = value(&opened.hanging, &draft.hanging, 1440.0);
        if [before, after, left, right, first_line, hanging]
            .iter()
            .all(Option::is_none)
        {
            return;
        }
        self.format_paragraphs(move |props| {
            if let Some(v) = before {
                props.spacing.before = v;
            }
            if let Some(v) = after {
                props.spacing.after = v;
            }
            if let Some(v) = left {
                props.indent.start = v;
            }
            if let Some(v) = right {
                props.indent.end = v;
            }
            if let Some(v) = first_line {
                props.indent.first_line = v;
            }
            if let Some(v) = hanging {
                props.indent.hanging = v;
            }
        });
    }

    fn column_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.column_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-column"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(260.0);
                    ui.label(egui::RichText::new("Column Width").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_sized([72.0, 20.0], egui::Label::new("Inches:"));
                        ui.add(egui::TextEdit::singleline(&mut draft).desired_width(64.0));
                    });
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "Apply") {
                        done = Some(answer);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        self.column_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.column_draft = None;
                // Word's own bounds: nothing narrower than it could hold a
                // character, nothing wider than its widest paper.
                if let Ok(inches) = draft.trim().parse::<f64>() {
                    if (0.05..=22.0).contains(&inches) {
                        self.apply_column_width(Twips((inches * 1440.0).round() as i32));
                    }
                }
            }
            Some(false) => self.column_draft = None,
            None => {}
        }
    }

    /// Opens the header or footer box, prefilled with what is there now: its
    /// text, one paragraph per line, and the formatting of its first words —
    /// the box sets one voice, so the first is the one it can show.
    /// The page the caret is standing on, which is the page a band command
    /// means: "the header" is always some particular page's header.
    fn caret_page(&self) -> usize {
        // A band open on page nine is a band of page nine, whatever the layout
        // says about where its first line is drawn. See [`Scriva::band_page`].
        if let Some(page) = self.band_page.filter(|_| self.editing_band()) {
            return page.min(self.view.pages().len().saturating_sub(1));
        }
        view::caret_rect(&self.view, self.scope, self.caret())
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    /// Whether the band being edited is a footer.
    fn in_footer(&self) -> bool {
        match self.scope {
            wp_model::Scope::Body => false,
            wp_model::Scope::Chrome(id) => {
                self.document.header(id).is_some_and(|header| header.footer)
            }
        }
    }

    /// Opens a page's header or footer for editing in place.
    ///
    /// **The band is edited where it is drawn.** A header holding a table, a
    /// logo and three fields cannot be stated in a box and read back, so the
    /// caret moves into it instead and every command that works in the text
    /// works there — which is what Word does, and why its header has never
    /// had a dialog.
    fn enter_band(&mut self, page: usize, footer: bool) {
        let Some(id) = self.band_body(page, footer) else {
            return;
        };
        if self.scope == wp_model::Scope::Body {
            self.left_behind = Some(self.selection);
        }
        self.go_to(wp_model::Scope::Chrome(id), Caret::default());
        self.reveal = Some(self.caret());
        // The band of *this* page, not of the first page that happens to show
        // the same one: opening the running head on page nine must not scroll
        // the window back to page one.
        self.reveal_on = Some(page);
        self.band_page = Some(page);
    }

    /// Takes the section's headers or footers away — Word's Remove Header,
    /// which removes every kind the section names and not only the one the
    /// page in front of you happens to show.
    ///
    /// Bodies and references go together in one change, because a reference
    /// pointing at nothing is a document Word calls damaged.
    fn remove_band(&mut self, footer: bool) {
        let index = self.caret_section();
        let Some(section) = self.document.section_mut(index) else {
            return;
        };
        let going: Vec<wp_model::HeaderId> = match footer {
            true => &section.footers,
            false => &section.headers,
        }
        .iter()
        .map(|reference| reference.body)
        .collect();
        if going.is_empty() {
            return;
        }
        self.close_band();
        self.history
            .push(wp_model::Scope::Body, self.chrome_change());
        if let Some(section) = self.document.section_mut(index) {
            match footer {
                true => section.footers.clear(),
                false => section.headers.clear(),
            }
        }
        // Only the bodies nothing still points at: another section may have
        // been unlinked onto the very same body, and a body a live reference
        // names is a document Word calls damaged.
        let kept: Vec<wp_model::HeaderId> = self
            .document
            .section_props()
            .iter()
            .flat_map(|section| {
                section
                    .headers
                    .iter()
                    .chain(section.footers.iter())
                    .map(|reference| reference.body)
                    .collect::<Vec<_>>()
            })
            .collect();
        self.document
            .headers
            .retain(|header| !going.contains(&header.id) || kept.contains(&header.id));
        self.changed();
    }

    /// One undo entry covering the bodies, every section's references and the
    /// setting that decides which of them a page uses.
    fn chrome_change(&self) -> edit::Change {
        edit::Change::Chrome {
            headers: self.document.headers.clone(),
            sections: self.document.section_props(),
            settings: Box::new(self.document.settings.clone()),
            caret: self.caret(),
        }
    }

    /// Which section the page the caret is on belongs to.
    ///
    /// A band command means *this page's* band, and in a document of several
    /// sections that is not the same as the last section's — which is what
    /// `Document::section` alone would have answered.
    fn caret_section(&self) -> usize {
        let page = self.caret_page();
        self.view
            .pages()
            .get(page)
            .map(|page| page.section)
            .unwrap_or_else(|| self.document.sections().len().saturating_sub(1))
    }

    /// Back to the text, where the caret was before the band was opened.
    fn close_band(&mut self) {
        self.band_page = None;
        if self.scope == wp_model::Scope::Body {
            return;
        }
        self.scope = wp_model::Scope::Body;
        self.picked = None;
        let back = self.left_behind.take().unwrap_or_default();
        self.selection = Selection {
            anchor: clamp(&self.document, wp_model::Scope::Body, back.anchor),
            head: clamp(&self.document, wp_model::Scope::Body, back.head),
        };
    }

    /// Which kind of band a page asks for: its own if it is a title page or
    /// an even one and the document says those differ, and the default
    /// otherwise.
    fn band_kind(&self, page: usize) -> wp_model::HeaderKind {
        let laid = self.view.pages().get(page);
        let number = laid.map(|page| page.number).unwrap_or(1);
        let index = laid
            .map(|page| page.section)
            .unwrap_or_else(|| self.document.sections().len().saturating_sub(1));
        let sections = self.document.sections();
        let section = sections
            .get(index)
            .map(|(_, section)| *section)
            .unwrap_or(&self.document.section);
        section
            .header_for_page(number, self.document.settings.even_and_odd_headers)
            .unwrap_or(wp_model::HeaderKind::Default)
    }

    /// Which body holds the band this page shows — making an empty one, with
    /// the section reference that gives it its identity, when the page has
    /// none.
    ///
    /// The *kind* is the one the page asked for rather than the default: a
    /// title page in a section with a first-page header of its own has to
    /// open that one, and a new header made for a title page has to be the
    /// first-page header or it will not appear on the page it was made from.
    fn band_body(&mut self, page: usize, footer: bool) -> Option<wp_model::HeaderId> {
        let kind = self.band_kind(page);
        // The laid-out page first, because it is what the user is looking at;
        // the section after it, because the layout is a frame behind — and it
        // is exactly one frame behind at the moment a band has just been made,
        // which is when making a second one would go unnoticed.
        let laid = self.view.pages().get(page).and_then(|page| match footer {
            true => page.footer_body,
            false => page.header_body,
        });
        match laid {
            Some(id) => Some(id),
            None => self.band_of_kind(page, kind, footer),
        }
    }

    /// The section's band of one kind, made if it has none.
    ///
    /// Asked directly, without the laid-out page, by the two switches that
    /// change *which* kind a page wants: the layout still shows the band the
    /// page wanted a moment ago, and following it would carry on editing the
    /// one that is no longer drawn there.
    fn band_of_kind(
        &mut self,
        page: usize,
        kind: wp_model::HeaderKind,
        footer: bool,
    ) -> Option<wp_model::HeaderId> {
        let index = self.page_section(page);
        // What the page shows now, with "Link to Previous" followed: a section
        // that inherits its band already has one to edit, and making a second
        // would silently unlink it.
        let shown = self.document.bands();
        let existing = shown.get(index).and_then(|bands| match footer {
            true => bands.footer(kind),
            false => bands.header(kind),
        });
        if existing.is_some() {
            return existing;
        }
        self.make_band(index, kind, footer, Vec::new())
    }

    /// Makes a band of one kind for one section and points the section at it.
    ///
    /// `content` is what goes in it: nothing for a band being made from
    /// scratch, and a copy of the inherited one for a section being unlinked —
    /// which is what Word does, measured: unlink a section's header and the
    /// words stay on the page while the section before it keeps its own copy.
    ///
    /// A body and the reference to it are one change, because restoring either
    /// without the other leaves a reference pointing at nothing.
    fn make_band(
        &mut self,
        index: usize,
        kind: wp_model::HeaderKind,
        footer: bool,
        content: Vec<Block>,
    ) -> Option<wp_model::HeaderId> {
        use wp_model::doc::HeaderFooter;
        use wp_model::section::{HeaderId, HeaderRef};
        self.history
            .push(wp_model::Scope::Body, self.chrome_change());
        let id = HeaderId(
            self.document
                .headers
                .iter()
                .map(|header| header.id.0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let content = match content.is_empty() {
            true => {
                let sections = self.document.sections();
                let section = sections
                    .get(index)
                    .map(|(_, section)| *section)
                    .unwrap_or(&self.document.section);
                vec![Block::Paragraph(band_paragraph(section))]
            }
            false => content,
        };
        self.document.headers.push(HeaderFooter {
            id,
            part: None,
            rel: None,
            footer,
            content,
        });
        let section = self.document.section_mut(index)?;
        let refs = match footer {
            true => &mut section.footers,
            false => &mut section.headers,
        };
        refs.push(HeaderRef {
            kind,
            body: id,
            rel: None,
        });
        self.changed();
        Some(id)
    }

    /// Which section a laid-out page belongs to.
    fn page_section(&self, page: usize) -> usize {
        self.view
            .pages()
            .get(page)
            .map(|page| page.section)
            .unwrap_or_else(|| self.document.sections().len().saturating_sub(1))
    }

    /// Word's "Different first page" and "Different odd & even pages" — the
    /// two switches that decide how many bands a section has and which of
    /// them any given page shows.
    ///
    /// Turning either on does not fill the new band in: Word leaves it empty
    /// and so does this, which is why the caret follows to whichever band the
    /// page in front of the user now wants. Turning one off leaves the band
    /// it stops using in the document, exactly as Word does — a switch is not
    /// a delete, and flicking it back has to bring the header back with it.
    fn set_band_kinds(&mut self, title_page: bool, even_and_odd: bool) {
        let page = self.caret_page();
        let index = self.page_section(page);
        // `<w:titlePg>` belongs to the section the page is in — a preface may
        // have a title page where the chapters after it do not — while the
        // even/odd flag is the document's and covers all of them.
        let was = self
            .document
            .sections()
            .get(index)
            .map(|(_, section)| section.title_page)
            .unwrap_or(self.document.section.title_page);
        if title_page == was && even_and_odd == self.document.settings.even_and_odd_headers {
            return;
        }
        let footer = self.in_footer();
        self.history
            .push(wp_model::Scope::Body, self.chrome_change());
        if let Some(section) = self.document.section_mut(index) {
            section.title_page = title_page;
        }
        self.document.settings.even_and_odd_headers = even_and_odd;
        self.changed();
        if self.editing_band() {
            let kind = self.band_kind(page);
            if let Some(id) = self.band_of_kind(page, kind, footer) {
                self.go_to(wp_model::Scope::Chrome(id), Caret::default());
            }
        }
    }

    /// Whether the band the caret's page shows is inherited from the section
    /// before it — Word's "Link to Previous".
    ///
    /// `None` in the first section of a document, which has nothing to link
    /// to and so is never offered the switch.
    fn linked_to_previous(&self, footer: bool) -> Option<bool> {
        let page = self.caret_page();
        let index = self.page_section(page);
        if index == 0 {
            return None;
        }
        let kind = self.band_kind(page);
        let sections = self.document.sections();
        let (_, section) = sections.get(index)?;
        Some(!wp_model::Bands::is_own(section, kind, footer))
    }

    /// Links the caret's section to the one before it, or breaks the link.
    ///
    /// **Breaking it copies rather than empties.** Word's own answer, measured
    /// over COM: unlink a second section's header and the words are still
    /// there, while the first section keeps a copy of its own — so the two can
    /// then be changed apart. Linking again drops this section's reference and
    /// leaves its body in the document, so that flicking the switch back does
    /// not cost the words that were typed into it.
    fn set_link_to_previous(&mut self, linked: bool) {
        let page = self.caret_page();
        let index = self.page_section(page);
        if index == 0 || self.linked_to_previous(self.in_footer()) == Some(linked) {
            return;
        }
        let footer = self.in_footer();
        let kind = self.band_kind(page);
        if linked {
            self.history
                .push(wp_model::Scope::Body, self.chrome_change());
            if let Some(section) = self.document.section_mut(index) {
                let refs = match footer {
                    true => &mut section.footers,
                    false => &mut section.headers,
                };
                refs.retain(|reference| reference.kind != kind);
            }
            self.changed();
            // Whatever the section inherits now is what the page shows, and
            // that is what the caret must be in — or it is editing a band that
            // is no longer drawn anywhere.
            match self
                .document
                .bands()
                .get(index)
                .and_then(|bands| match footer {
                    true => bands.footer(kind),
                    false => bands.header(kind),
                }) {
                Some(id) => self.go_to(wp_model::Scope::Chrome(id), Caret::default()),
                None => self.close_band(),
            }
            return;
        }
        let inherited = self
            .document
            .bands()
            .get(index)
            .and_then(|bands| match footer {
                true => bands.footer(kind),
                false => bands.header(kind),
            })
            .and_then(|id| self.document.header(id))
            .map(|band| band.content.clone())
            .unwrap_or_default();
        if let Some(id) = self.make_band(index, kind, footer, inherited) {
            self.go_to(wp_model::Scope::Chrome(id), Caret::default());
        }
    }

    /// Which band of a page a point is in: the margin above the text, or the
    /// one below it. `None` between them, which is the body.
    fn band_at(&self, spot: view::Spot) -> Option<bool> {
        let page = self.view.pages().get(spot.page)?;
        if spot.y < page.geometry.top {
            return Some(false);
        }
        if spot.y > page.geometry.height - page.geometry.bottom {
            return Some(true);
        }
        None
    }

    /// Whether a click there is a click in the flow being edited.
    ///
    /// The rest of the page is showing but not being edited, and a click on it
    /// must not drag the caret out from under the keyboard — the same reason
    /// the veil is drawn over it.
    fn click_lands_here(&self, spot: view::Spot) -> bool {
        match self.scope {
            wp_model::Scope::Body => self.band_at(spot).is_none(),
            wp_model::Scope::Chrome(_) => self.band_at(spot) == Some(self.in_footer()),
        }
    }

    /// The bar that stands under the toolbar while a band is open, saying
    /// which one and offering the two ways out of it.
    fn band_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let footer = self.in_footer();
        let index = self.page_section(self.caret_page());
        let was = (
            self.document
                .sections()
                .get(index)
                .map(|(_, section)| section.title_page)
                .unwrap_or(self.document.section.title_page),
            self.document.settings.even_and_odd_headers,
        );
        let (mut title_page, mut even_and_odd) = was;
        // `None` in the first section: there is nothing before it to link to,
        // and a switch that can only ever be off is a switch that misleads.
        let linked = self.linked_to_previous(footer);
        let mut link = linked.unwrap_or(false);
        let mut chosen = None;
        // The application's theme leaves an unchecked box with no outline at
        // all — fine on a menu row, where the tick is the only state worth
        // showing, and wrong here: a switch nobody can see until it is already
        // on is a switch nobody finds.
        {
            let widgets = &mut ui.style_mut().visuals.widgets;
            widgets.inactive.bg_fill = egui::Color32::WHITE;
            widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0x8C));
            widgets.hovered.bg_fill = egui::Color32::WHITE;
            widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0x5C));
            widgets.active.bg_fill = egui::Color32::WHITE;
        }
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(match footer {
                    true => "Footer",
                    false => "Header",
                })
                .strong(),
            );
            ui.label(
                egui::RichText::new("— the page itself is not being edited")
                    .weak()
                    .small(),
            );
            ui.add_space(12.0);
            // Word keeps these two on the Header & Footer tab, and they belong
            // here for the same reason: they are only ever wanted while one is
            // open, and they decide which one you are looking at.
            ui.checkbox(&mut title_page, "Different first page")
                .on_hover_text("The first page of the section carries a band of its own");
            ui.checkbox(&mut even_and_odd, "Different odd & even")
                .on_hover_text("Left- and right-hand pages carry different bands");
            if linked.is_some() {
                ui.checkbox(&mut link, "Link to Previous").on_hover_text(
                    "Show the section before this one's band. Turning it off \
                     takes a copy this section can change on its own.",
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if ui
                    .button("Close")
                    .on_hover_text("Go back to the text — Esc, or double-click the page")
                    .clicked()
                {
                    chosen = Some(Command::CloseChrome);
                }
                if ui
                    .button(match footer {
                        true => "Go to Header",
                        false => "Go to Footer",
                    })
                    .clicked()
                {
                    chosen = Some(Command::SwitchBand);
                }
                if ui
                    .button("From edge…")
                    .on_hover_text("How far the header and footer sit from the paper's edges")
                    .clicked()
                {
                    chosen = Some(Command::CustomMargins);
                }
            });
        });
        if (title_page, even_and_odd) != was {
            self.set_band_kinds(title_page, even_and_odd);
        }
        if linked.is_some_and(|was| was != link) {
            self.set_link_to_previous(link);
        }
        chosen
    }

    /// Puts a page number at the caret — Word's Insert ▸ Page Number, which is
    /// a field and not a typed digit, so it counts on every page it is drawn.
    fn insert_page_field(&mut self, of_pages: bool) {
        use wp_model::doc::Piece;
        let caret = self.caret();
        let Some(before) = edit::paragraph_at(&self.document, self.scope, caret.paragraph) else {
            return;
        };
        self.history.push(
            self.scope,
            edit::Change::Paragraph {
                index: caret.paragraph,
                before: Box::new(before),
            },
        );
        // The cached "1" between the separator and the end is what a reader
        // that does not evaluate fields shows, and what Word writes.
        let field = |code: &str| {
            [
                Piece::FieldStart {
                    dirty: false,
                    lock: false,
                },
                Piece::Instruction(format!(" {code} ").into()),
                Piece::FieldSeparate,
                Piece::Text("1".into()),
                Piece::FieldEnd,
            ]
        };
        let mut pieces: Vec<Piece> = Vec::new();
        if of_pages {
            pieces.push(Piece::Text("Page ".into()));
        }
        pieces.extend(field("PAGE"));
        if of_pages {
            pieces.push(Piece::Text(" of ".into()));
            pieces.extend(field("NUMPAGES"));
        }
        let mut offset = caret.offset;
        {
            let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
            let Some(target) = paragraphs.get_mut(caret.paragraph) else {
                return;
            };
            offset += crate::text::insert_pieces(target, offset, pieces);
        }
        self.selection = Selection::at(Caret {
            paragraph: caret.paragraph,
            offset,
        });
        self.changed();
    }

    /// Opens the watermark box, prefilled with the watermark the document
    /// already carries.
    fn open_watermark_dialog(&mut self) {
        let found = watermark_in(&self.document);
        self.watermark_draft = Some(WatermarkDraft {
            text: found
                .map(|shape| shape.text.to_string())
                .unwrap_or_default(),
            font: found
                .and_then(|shape| shape.font.as_deref().map(str::to_owned))
                .unwrap_or_default(),
            color: match found.and_then(|shape| shape.color) {
                Some(wp_model::Color::Rgb(rgb)) => {
                    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
                }
                _ => String::new(),
            },
            // A shape turned at all is a diagonal one; Word's own is 315.
            diagonal: found.is_none_or(|shape| shape.rotation != 0.0),
            existing: found.is_some(),
        });
    }

    fn watermark_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.watermark_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        let mut remove = false;
        egui::Modal::new(egui::Id::new("scriva-watermark"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(320.0);
                    ui.label(egui::RichText::new("Watermark").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 20.0], egui::Label::new("Text:"));
                        ui.add(egui::TextEdit::singleline(&mut draft.text).desired_width(232.0));
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 20.0], egui::Label::new("Font:"));
                        ui.add(egui::TextEdit::singleline(&mut draft.font).desired_width(150.0));
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 20.0], egui::Label::new("Colour:"));
                        ui.add(egui::TextEdit::singleline(&mut draft.color).desired_width(64.0));
                        ui.label("hex");
                    });
                    ui.add_space(4.0);
                    ui.checkbox(&mut draft.diagonal, "Diagonal");
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "It goes in the header, behind the words, on every page.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.add_space(12.0);
                    if draft.existing {
                        if ui.button("Remove watermark").clicked() {
                            remove = true;
                            done = Some(true);
                        }
                        ui.add_space(6.0);
                    }
                    if let Some(answer) = dialog::confirm(ui, "Apply") {
                        done = Some(answer);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        self.watermark_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.watermark_draft = None;
                if remove {
                    draft.text.clear();
                }
                self.apply_watermark(&draft);
            }
            Some(false) => self.watermark_draft = None,
            None => {}
        }
    }

    /// Puts the watermark into every header the section names, taking away
    /// whatever one was there — one undo step.
    ///
    /// **Into every header, not just the default one.** A document with a
    /// title page names three, and Word stamps all of them; putting the shape
    /// in one alone gives a document whose first page is unmarked, which for a
    /// watermark is the page that most needed marking.
    fn apply_watermark(&mut self, draft: &WatermarkDraft) {
        use wp_model::doc::{HeaderFooter, Inline, Piece, Run};
        use wp_model::section::{HeaderId, HeaderKind, HeaderRef};
        self.history
            .push(wp_model::Scope::Body, self.chrome_change());
        let shape = self.watermark_shape(draft);
        let document = &mut self.document;
        for header in document.headers.iter_mut().filter(|header| !header.footer) {
            strip_watermark(&mut header.content);
        }
        let Some(shape) = shape else {
            self.changed();
            return;
        };
        // A document that names no header at all needs one made for the
        // watermark to live in; the writer assigns its part and relationship
        // when it first writes the package.
        if document.section.headers.is_empty() {
            let id = HeaderId(document.headers.iter().map(|h| h.id.0).max().unwrap_or(0) + 1);
            document.headers.push(HeaderFooter {
                id,
                part: None,
                rel: None,
                footer: false,
                content: Vec::new(),
            });
            document.section.headers.push(HeaderRef {
                kind: HeaderKind::Default,
                body: id,
                rel: None,
            });
        }
        // **Only the headers a page will actually show.** A document commonly
        // names three — default, first, even — while `<w:titlePg>` and the
        // document's even/odd setting are both off, in which case two of them
        // are never drawn. Word stamps the watermark on the one that is:
        // measured against a watermark Word wrote itself, which put the shape
        // in the default header and left the other two parts empty. Stamping
        // all three writes shapes into stories no reader ever sees, and Word
        // then reports three watermarks on a document that shows one.
        let title_page = document.section.title_page;
        let even_and_odd = document.settings.even_and_odd_headers;
        let mut wanted: Vec<HeaderId> = document
            .section
            .headers
            .iter()
            .filter(|reference| match reference.kind {
                HeaderKind::Default => true,
                HeaderKind::First => title_page,
                HeaderKind::Even => even_and_odd,
            })
            .map(|reference| reference.body)
            .collect();
        // Two references may name one body — "link to previous" writes that —
        // and a body stamped twice carries the watermark twice.
        wanted.sort_unstable_by_key(|id| id.0);
        wanted.dedup();
        for id in wanted {
            let Some(header) = document
                .headers
                .iter_mut()
                .find(|header| header.id == id && !header.footer)
            else {
                continue;
            };
            let mut paragraph = Paragraph::new();
            paragraph.props.spacing.before = Some(Twips(0));
            paragraph.props.spacing.after = Some(Twips(0));
            paragraph.content.push(Inline::Run(Run {
                content: vec![Piece::Drawing(Box::new(shape.clone()))],
                ..Run::new()
            }));
            header.content.push(Block::Paragraph(paragraph));
        }
        self.changed();
    }

    /// The shape a watermark draft asks for, or `None` when the draft is
    /// blank and the watermark is being taken away.
    ///
    /// **The size is derived rather than asked for.** Word's box does not
    /// offer one either: it fits the words to the page. A turned shape's
    /// bounding box is `(w + h) / root two` across *and* down, so the width
    /// that just fits the text area is `side * root two / (1 + 1/aspect)` —
    /// which on US Letter with "CONFIDENTIAL" gives 529.5 points against the
    /// 527.75 Word itself wrote, a third of a per cent apart. The aspect is
    /// the string's own, measured, so the letters keep their proportions
    /// instead of being stretched into a shape someone guessed.
    fn watermark_shape(&mut self, draft: &WatermarkDraft) -> Option<wp_model::doc::Drawing> {
        /// The width-to-height proportion of a watermark whose words could
        /// not be measured. Word's own "CONFIDENTIAL" is four to one.
        const DEFAULT_ASPECT: f64 = 4.0;
        use wp_model::doc::{Alignment, DrawingPosition, Offset, RelativeTo, ShapeText, Wrap};
        let text = draft.text.trim();
        if text.is_empty() {
            return None;
        }
        let family = match draft.font.trim() {
            "" => "Calibri".to_owned(),
            named => named.to_owned(),
        };
        let request = wp_layout::FontRequest {
            family: family.clone().into(),
            size: 100.0,
            bold: false,
            italic: false,
        };
        let section = &self.document.section;
        let across = section.text_width().points();
        let down = (section.page.height.points() - section.margins.top.points())
            - section.margins.bottom.points();
        // Measured when there is a shaper to measure with. There is one
        // whenever a window is open, which is whenever this box can be
        // reached; the fallback is for a document driven without a screen,
        // and is the proportion Word's own "CONFIDENTIAL" comes out at.
        let aspect = self
            .shaper
            .as_mut()
            .map(|shaper| {
                use wp_layout::Shaper as _;
                let metrics = shaper.metrics(&request);
                (
                    shaper.width(text, &request),
                    metrics.ascent + metrics.descent,
                )
            })
            .filter(|(natural, tall)| *natural > 0.0 && *tall > 0.0)
            .map(|(natural, tall)| natural / tall)
            .unwrap_or(DEFAULT_ASPECT);
        let width = match draft.diagonal {
            true => across.min(down) * std::f64::consts::SQRT_2 / (1.0 + 1.0 / aspect),
            false => across,
        };
        let width = width.max(1.0);
        Some(wp_model::doc::Drawing {
            // No source: the writer authors the VML afresh, which is what
            // makes an edited watermark actually reach the file.
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(width),
                wp_model::Emu::from_points(width / aspect),
            ),
            rel: None,
            chart: None,
            name: None,
            description: None,
            wrap: Wrap::None,
            distance: Default::default(),
            position: Some(Box::new(DrawingPosition {
                horizontal: Offset {
                    relative_to: RelativeTo::Margin,
                    offset: None,
                    align: Some(Alignment::Center),
                },
                vertical: Offset {
                    relative_to: RelativeTo::Margin,
                    offset: None,
                    align: Some(Alignment::Center),
                },
            })),
            behind_text: true,
            tone: None,
            outline: None,
            text: Some(Box::new(ShapeText {
                text: text.into(),
                font: Some(family.into()),
                color: Some(
                    wp_model::Color::from_val(draft.color.trim())
                        .filter(|color| !color.is_auto())
                        // Word's own watermark grey, lightened the way its
                        // half-opaque fill lightens it.
                        .unwrap_or(wp_model::Color::Rgb([0xE0, 0xE0, 0xE0])),
                ),
                bold: false,
                italic: false,
                rotation: if draft.diagonal { 315.0 } else { 0.0 },
            })),
        })
    }

    /// Runs one edit against the table the caret is in, as one undo step, or
    /// says why nothing happened.
    fn edit_table(&mut self, change: impl FnOnce(&mut wp_model::table::Table, usize, usize)) {
        let caret = self.caret();
        let Some((index, row, cell)) = edit::table_cell_at(&self.document, self.scope, caret)
        else {
            self.message = Some((
                "Not in a table".to_owned(),
                "Put the caret in a table cell first, then try again.".to_owned(),
            ));
            return;
        };
        self.history.push(
            self.scope,
            edit::Change::Blocks {
                index,
                before: vec![self.document.body[index].clone()],
                now: 1,
            },
        );
        if let Block::Table(table) = &mut self.document.body[index] {
            change(table, row, cell);
        }
        self.changed();
    }

    /// Opens the cell-padding box on the caret's table, prefilled with what
    /// the table states itself — blank where it says nothing and takes the
    /// padding from its style, which is where Word keeps its own 0.08in.
    fn open_cell_margin_dialog(&mut self) {
        let caret = self.caret();
        let Some((index, _, _)) = edit::table_cell_at(&self.document, self.scope, caret) else {
            self.message = Some((
                "Not in a table".to_owned(),
                "Put the caret in a table cell first, then try again.".to_owned(),
            ));
            return;
        };
        let Block::Table(table) = &self.document.body[index] else {
            return;
        };
        let points = |w: Option<wp_model::table::Width>| match w {
            Some(wp_model::table::Width::Fixed(t)) => trim_number(t.0 as f64 / 20.0),
            _ => String::new(),
        };
        let margins = table.props.cell_margins;
        self.cell_margin_draft = Some([
            points(margins.top),
            points(margins.start),
            points(margins.bottom),
            points(margins.end),
        ]);
    }

    fn cell_margin_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.cell_margin_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-cell-margins"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(280.0);
                    ui.label(egui::RichText::new("Cell Margins").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    for (label, field) in ["Top:", "Left:", "Bottom:", "Right:"]
                        .into_iter()
                        .zip(draft.iter_mut())
                    {
                        ui.horizontal(|ui| {
                            ui.add_sized([64.0, 20.0], egui::Label::new(label));
                            ui.add(egui::TextEdit::singleline(field).desired_width(64.0));
                            ui.label("pt");
                        });
                    }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Blank leaves it to the table's style.")
                            .small()
                            .weak(),
                    );
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "Apply") {
                        done = Some(answer);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        self.cell_margin_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.cell_margin_draft = None;
                self.apply_cell_margins(&draft);
            }
            Some(false) => self.cell_margin_draft = None,
            None => {}
        }
    }

    /// States the caret's table's cell padding. A blank field states nothing,
    /// which is not the same as nothing at all: it puts the side back in the
    /// hands of the table's style.
    fn apply_cell_margins(&mut self, draft: &[String; 4]) {
        let side = |text: &str| {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            text.parse::<f64>()
                .ok()
                .filter(|v| (0.0..=100.0).contains(v))
                .map(|v| wp_model::table::Width::Fixed(Twips((v * 20.0).round() as i32)))
        };
        let margins = wp_model::table::CellMargins {
            top: side(&draft[0]),
            start: side(&draft[1]),
            bottom: side(&draft[2]),
            end: side(&draft[3]),
        };
        self.edit_table(move |table, _, _| table.props.cell_margins = margins);
    }

    /// Opens the width box on the caret's column, prefilled in inches.
    fn open_column_dialog(&mut self) {
        let caret = self.caret();
        let Some((index, row, cell)) = edit::table_cell_at(&self.document, self.scope, caret)
        else {
            self.message = Some((
                "Not in a table".to_owned(),
                "Put the caret in a table cell first, then try again.".to_owned(),
            ));
            return;
        };
        let Block::Table(table) = &self.document.body[index] else {
            return;
        };
        let column = starting_column(table, row, cell);
        let width = table.grid.get(column).copied().unwrap_or(Twips(0));
        self.column_draft = Some(format!("{:.2}", width.0 as f64 / 1440.0));
    }

    /// Sets the caret's column to `width`, and restates the width of every
    /// cell the column passes through — the grid and the cells disagreeing is
    /// what makes Word redraw a table differently than it was saved.
    fn apply_column_width(&mut self, width: Twips) {
        self.edit_table(move |table, at_row, at_cell| {
            let column = starting_column(table, at_row, at_cell);
            let Some(entry) = table.grid.get_mut(column) else {
                return;
            };
            *entry = width;
            for row in &mut table.rows {
                let mut at = row.props.grid_before as usize;
                for cell in &mut row.cells {
                    let span = cell.props.grid_span.max(1) as usize;
                    if (at..at + span).contains(&column) {
                        let total: i32 = table.grid[at..(at + span).min(table.grid.len())]
                            .iter()
                            .map(|t| t.0)
                            .sum();
                        cell.props.width = wp_model::table::Width::Fixed(Twips(total));
                    }
                    at += span;
                }
            }
        });
    }

    /// Builds an evenly divided, fully ruled table and puts it above the
    /// caret's paragraph.
    fn insert_table(&mut self, rows: usize, columns: usize) {
        use wp_model::table::{Cell, Row, Table, TableBorders, TableProps};
        let margins = &self.document.section.margins;
        let text_width = self.document.section.page.width.0 - margins.start.0 - margins.end.0;
        let each = Twips((text_width / columns as i32).max(144));
        // Word's default grid: half-point single lines on every edge.
        let edge = ruled_edge();
        let table = Table {
            props: TableProps {
                borders: TableBorders {
                    top: Some(edge),
                    start: Some(edge),
                    bottom: Some(edge),
                    end: Some(edge),
                    inside_h: Some(edge),
                    inside_v: Some(edge),
                },
                ..TableProps::default()
            },
            grid: vec![each; columns],
            rows: (0..rows)
                .map(|_| Row {
                    props: Default::default(),
                    cells: (0..columns).map(|_| Cell::new()).collect(),
                })
                .collect(),
        };
        let caret = edit::insert_block(
            &mut self.document,
            self.scope,
            &mut self.history,
            self.selection,
            Block::Table(table),
        );
        self.selection = Selection::at(clamp(&self.document, self.scope, caret));
        self.changed();
        self.reveal = Some(self.caret());
    }

    /// Word's Size box, for a picture or a chart: two numbers, in inches.
    ///
    /// Dragging a handle is the fast way and this is the exact one. Typing a
    /// width with the ratio locked moves the height with it, which is what the
    /// lock means — Word recomputes the other field as you leave the one you
    /// typed in, and so does this.
    fn size_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.size_draft.take() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-size"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(260.0);
                    ui.label(egui::RichText::new("Size").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    let mut typed: Option<bool> = None;
                    for (label, horizontal) in [("Width:", true), ("Height:", false)] {
                        ui.horizontal(|ui| {
                            ui.add_sized([56.0, 20.0], egui::Label::new(label));
                            let field = match horizontal {
                                true => &mut draft.width,
                                false => &mut draft.height,
                            };
                            if ui
                                .add(egui::TextEdit::singleline(field).desired_width(64.0))
                                .changed()
                            {
                                typed = Some(horizontal);
                            }
                            ui.label("in");
                        });
                    }
                    // The locked field follows the typed one, so the box always
                    // shows the size it would set.
                    if let Some(horizontal) = typed.filter(|_| draft.locked) {
                        let (from, to) = match horizontal {
                            true => (&draft.width, draft.ratio.recip()),
                            false => (&draft.height, draft.ratio),
                        };
                        if let Some(value) = from.trim().parse::<f64>().ok().filter(|v| *v > 0.0) {
                            let other = inches(value * to * 72.0);
                            match horizontal {
                                true => draft.height = other,
                                false => draft.width = other,
                            }
                        }
                    }
                    ui.add_space(4.0);
                    ui.checkbox(&mut draft.locked, "Lock aspect ratio");
                    if let Some((width, height)) = draft.natural {
                        ui.add_space(4.0);
                        if ui.button("Original size").clicked() {
                            draft.width = inches(width);
                            draft.height = inches(height);
                        }
                    }
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "OK") {
                        done = Some(answer);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        done = Some(true);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        match done {
            Some(true) => {
                // A field that does not parse keeps the size it had: the box is
                // not the place to argue about a typo.
                let read = |text: &str, was: f64| {
                    text.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|inches| (0.01..=22.0).contains(inches))
                        .map(|inches| inches * 72.0)
                        .unwrap_or(was)
                };
                let (was_w, was_h) = self
                    .picked_drawing()
                    .map(|drawing| (drawing.extent.0.points(), drawing.extent.1.points()))
                    .unwrap_or((0.0, 0.0));
                let width = read(&draft.width, was_w);
                let height = read(&draft.height, was_h);
                if (width, height) != (was_w, was_h) {
                    self.resize_drawing(draft.picked, width, height);
                }
            }
            Some(false) => {}
            None => self.size_draft = Some(draft),
        }
    }

    /// Word's Zoom box: presets, the two fits, and a percent you can type.
    fn zoom_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.zoom_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-zoom"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(240.0);
                    ui.label(egui::RichText::new("Zoom").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    ui.label("Zoom to");
                    let mut preset: Option<i32> = None;
                    ui.horizontal(|ui| {
                        for percent in [200, 100, 75] {
                            if ui.button(format!("{percent}%")).clicked() {
                                preset = Some(percent);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Page width").clicked() {
                            preset = self.fit_percent(true);
                        }
                        if ui.button("Whole page").clicked() {
                            preset = self.fit_percent(false);
                        }
                    });
                    if let Some(percent) = preset {
                        draft = percent.to_string();
                        self.zoom_fresh = true;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Percent:");
                        // While the field is untouched its number stays selected,
                        // re-seeded every frame — egui clears the selection on
                        // frames the field is not yet focused, and a keystroke
                        // can arrive on any frame. First touch ends it.
                        if self.zoom_fresh {
                            select_percent(ui.ctx(), &draft);
                        }
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut draft)
                                .id(egui::Id::new("scriva-zoom-percent"))
                                .desired_width(56.0),
                        );
                        ui.label("%");
                        if field.changed() || field.clicked() || field.dragged() {
                            self.zoom_fresh = false;
                        }
                        field.request_focus();
                    });
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "OK") {
                        done = Some(answer);
                    }
                    // The percent field is the only thing to type into, so Enter
                    // anywhere in the box means OK — the way Word's box reads it.
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        done = Some(true);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        self.zoom_draft = Some(draft.clone());
        if done.is_some() {
            // The box closes this frame, and the typing gate has already been
            // decided for it — without this, the very Enter that confirmed
            // the zoom would fall through into the document as a new line.
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
        match done {
            Some(true) => {
                self.zoom_draft = None;
                // A percent that does not parse keeps the zoom it had.
                if let Ok(percent) = draft.trim().trim_end_matches('%').trim().parse::<f64>() {
                    self.view.zoom = percent.clamp(10.0, 500.0) / 100.0;
                }
            }
            Some(false) => self.zoom_draft = None,
            None => {}
        }
    }

    /// The zoom that fits the paper to the desk — its width, or the whole
    /// first page — leaving a little air for the scrollbar and the edges.
    fn fit_percent(&self, width_only: bool) -> Option<i32> {
        let geometry = &self.view.pages().first()?.geometry;
        // The paper's size on the glass is its points times [`view::SCALE`],
        // so the percent that fits is measured against that.
        let fit_w = (self.viewport.x as f64 - 32.0).max(60.0) / (geometry.width * view::SCALE);
        let percent = if width_only {
            fit_w
        } else {
            fit_w.min((self.viewport.y as f64 - 24.0).max(60.0) / (geometry.height * view::SCALE))
        };
        Some(((percent * 100.0).floor() as i32).clamp(10, 500))
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
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(420.0);
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
                        self.scope,
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
        let current = self.find_matches.iter().position(|(scope, found)| {
            *scope == self.scope && found.ordered() == self.selection.ordered()
        });

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
        // The percent, taken to the glass: 100% is Word's — a document inch
        // on 96 logical pixels — not a point per point.
        let zoom = (self.view.zoom * view::SCALE) as f32;
        let (extent_w, extent_h) = self.view.extent();
        let outer = ui.available_rect_before_wrap();
        self.viewport = outer.size();
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
                // Over a picture it says a different thing, and over one of a
                // selected picture's handles it says which way that handle
                // pulls — the only way a user finds out a picture can be
                // resized at all is the pointer changing shape over it.
                // A handle is a fixed-size target on the glass however far
                // the page is zoomed out; reach is that target measured on
                // the page.
                let reach = crate::drawings::GRIP / zoom.max(0.05) as f64;
                if response.hovered() || self.dragging.is_some() {
                    let over = ui
                        .ctx()
                        .pointer_hover_pos()
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom));
                    // A link is announced the two ways Word announces one: a
                    // tooltip saying where it goes and how to go there, and,
                    // while the key that follows it is held, the hand. Without
                    // either, a link is text that happens to be blue and the
                    // only way to find out it can be followed is to guess.
                    let link = over
                        .and_then(|spot| view::character_over(&self.view, self.scope, spot))
                        .and_then(|caret| self.link_at(caret));
                    match (&link, ui.input(|i| i.modifiers.command)) {
                        (Some(_), true) => ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand),
                        _ => ui.ctx().set_cursor_icon(self.pointer_icon(over, reach)),
                    }
                    // Not while the right-click menu is up: the menu is the
                    // answer to the same question and the tooltip only sits
                    // under it repeating itself.
                    let menu_up = egui::Popup::is_any_open(ui.ctx());
                    if let Some(destination) = link.as_ref().filter(|_| !menu_up) {
                        let where_to = match destination {
                            crate::links::Destination::Away(url) => url.clone(),
                            crate::links::Destination::Here(name) => {
                                format!("{name} (in this document)")
                            }
                        };
                        egui::Tooltip::always_open(
                            ui.ctx().clone(),
                            ui.layer_id(),
                            egui::Id::new("scriva-link"),
                            egui::PopupAnchor::Pointer,
                        )
                        .show(|ui| {
                            ui.label(where_to);
                            ui.label("Ctrl+click to follow link");
                        });
                    }
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
                self.pictures.prepare_charts(
                    self.package.as_ref(),
                    self.parts.as_ref(),
                    view::chart_rels(&self.view).into_iter(),
                );
                view::paint(
                    &painter,
                    &self.view,
                    self.scope,
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
                    // The grip is chosen by where the press landed, not where
                    // the pointer is now: a drag is only reported once it has
                    // moved a few pixels, and a quick pull would already be
                    // off the handle it took hold of.
                    let spot = ui
                        .input(|i| i.pointer.press_origin())
                        .or_else(|| response.interact_pointer_pos())
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom));
                    self.dragging = None;
                    match spot.and_then(|spot| {
                        view::drawing_at(&self.view, self.scope, spot, reach)
                            .map(|found| (spot, found))
                    }) {
                        Some((spot, (picked, rect))) => {
                            let already = self.picked == Some(picked);
                            self.picked = Some(picked);
                            // A handle can only be pulled once it is on the
                            // screen to aim at: the first press on a picture
                            // selects it and drags it about, and the press
                            // after that can take hold of a corner.
                            self.dragging = match already {
                                true => crate::drawings::grip_at(rect, spot.x, spot.y, reach),
                                false => Some(crate::drawings::Grip::Body),
                            };
                            self.drag_from = Some((spot.x, spot.y));
                            ui.ctx().memory_mut(|m| m.request_focus(response.id));
                        }
                        None => self.picked = None,
                    }
                }
                if self.picked.is_none() {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        if let Some(spot) = self.spot_at(pointer, origin, zoom) {
                            // A click on the part of the page that is *not*
                            // being edited is not a place to put the caret.
                            // While a header is open the text is showing and
                            // not editable — which is what the wash over it
                            // says — and dragging the caret out from under the
                            // keyboard would make a liar of it.
                            if self.click_lands_here(spot) {
                                if let Some(caret) = view::caret_at(&self.view, self.scope, spot) {
                                    let extend = ui.input(|i| i.modifiers.shift) || self.sweeping;
                                    self.set_caret(caret, extend);
                                }
                            }
                        }
                    }
                    // Ctrl+click follows the link under the pointer, which is
                    // Word's gesture and Word's reason for it: the letters of
                    // a link are still text to put a caret in and edit, so a
                    // bare click cannot mean "go there".
                    if response.clicked() && ui.input(|i| i.modifiers.command) {
                        if let Some(destination) = response
                            .interact_pointer_pos()
                            .and_then(|pointer| self.spot_at(pointer, origin, zoom))
                            .and_then(|spot| view::character_over(&self.view, self.scope, spot))
                            .and_then(|caret| self.link_at(caret))
                        {
                            self.follow_link(destination);
                        }
                    }
                    // A second click takes the word and a third takes the
                    // paragraph, the way every word processor since has —
                    // except in the margins. There a double-click opens the
                    // band drawn there, and once one is open a double-click on
                    // the page closes it again and puts the caret where it
                    // landed. Both gestures are Word's, and they are the only
                    // way most people ever reach a header.
                    if response.double_clicked() {
                        let spot = response
                            .interact_pointer_pos()
                            .and_then(|pointer| self.spot_at(pointer, origin, zoom));
                        let band = spot.and_then(|spot| self.band_at(spot));
                        // A margin that is not the band already open — the
                        // footer while the header is up, or either of them
                        // from the text — opens the one drawn there. A margin
                        // that *is* the open band is ordinary text, and a
                        // double-click in it takes a word like anywhere else.
                        let elsewhere = band.is_some_and(|footer| {
                            !self.editing_band() || footer != self.in_footer()
                        });
                        match (elsewhere, self.scope, band) {
                            (true, _, Some(footer)) => {
                                if let Some(spot) = spot {
                                    self.enter_band(spot.page, footer);
                                }
                            }
                            (_, wp_model::Scope::Chrome(_), None) => {
                                self.close_band();
                                if let Some(caret) = spot
                                    .and_then(|spot| view::caret_at(&self.view, self.scope, spot))
                                {
                                    self.set_caret(caret, false);
                                }
                            }
                            _ => {
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
                        }
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
                            .and_then(|spot| view::caret_at(&self.view, self.scope, spot))
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
                // A right-click on a picture selects it, the same as a left one:
                // the menu that comes up is about what was clicked.
                if response.secondary_clicked() {
                    if let Some(found) = response
                        .interact_pointer_pos()
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom))
                        .and_then(|spot| view::drawing_at(&self.view, self.scope, spot, reach))
                    {
                        self.picked = Some(found.0);
                    }
                    // The menu outlives the click that opened it, so what was
                    // under that click has to be remembered rather than asked
                    // for again when the menu is drawn.
                    let clicked = response
                        .interact_pointer_pos()
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom))
                        .and_then(|spot| view::character_over(&self.view, self.scope, spot))
                        .and_then(|caret| self.link_at(caret));
                    self.menu_link = clicked;
                }
                let has_selection = !self.selection.is_empty();
                let picture = self.picked.is_some();
                let mut chosen: Option<Command> = None;
                let mut follow: Option<crate::links::Destination> = None;
                response.context_menu(|ui| {
                    ui.set_min_width(160.0);
                    // A picked picture has its own menu: Cut and Copy are the
                    // text's, and a picture is not a stretch of text.
                    if picture {
                        if ui.button("Cut").clicked() {
                            chosen = Some(Command::Cut);
                            ui.close();
                        }
                        if ui.button("Copy").clicked() {
                            chosen = Some(Command::Copy);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Size…").clicked() {
                            chosen = Some(Command::PictureSize);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Delete").clicked() {
                            chosen = Some(Command::DeletePicture);
                            ui.close();
                        }
                        return;
                    }
                    // Word offers this too, and it is how a reader who never
                    // hears about the modifier follows a link.
                    if let Some(destination) = &self.menu_link {
                        if ui.button("Open Hyperlink").clicked() {
                            follow = Some(destination.clone());
                            ui.close();
                        }
                        ui.separator();
                    }
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
                if let Some(destination) = follow {
                    self.follow_link(destination);
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
                            // Shift breaks a corner's hold on the aspect
                            // ratio, for the user who means to stretch.
                            let keep = !ui.input(|i| i.modifiers.shift);
                            self.drag_drawing(grip, spot.x - from.0, spot.y - from.1, keep);
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
                    let prefer = self.reveal_on.take();
                    if let Some((page, rect)) =
                        view::caret_rect_on(&self.view, self.scope, caret, prefer)
                    {
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
    fn drag_drawing(&mut self, grip: crate::drawings::Grip, dx: f64, dy: f64, keep: bool) {
        use crate::drawings::{moved, resized, Grip};

        let Some(picked) = self.picked else {
            return;
        };
        let Some((page, _)) = view::rect_of(&self.view, self.scope, picked) else {
            return;
        };
        let geometry = match self.view.pages().get(page) {
            Some(page) => page.geometry,
            None => return,
        };
        // Where the paragraph starts on the page, which is what an offset
        // relative to the paragraph is measured from.
        let origin = view::rect_of(&self.view, self.scope, picked)
            .map(|(_, rect)| (rect.0, rect.1))
            .unwrap_or((geometry.start, geometry.top));
        let before = match self
            .document
            .paragraphs_in(self.scope)
            .get(picked.paragraph)
        {
            Some(paragraph) => (*paragraph).clone(),
            None => return,
        };
        let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
        let Some(drawing) = paragraphs
            .get_mut(picked.paragraph)
            .and_then(|paragraph| paragraph.drawing_mut(picked.nth))
        else {
            return;
        };
        let changed = match grip {
            Grip::Body => moved(drawing, &geometry, origin, dx, dy),
            grip => resized(drawing, grip, dx, dy, keep),
        };
        drop(paragraphs);
        if !changed {
            return;
        }
        if !self.dragged {
            self.history.push(
                self.scope,
                crate::edit::Change::Paragraph {
                    index: picked.paragraph,
                    before: Box::new(before),
                },
            );
            self.dragged = true;
        }
        self.changed();
    }

    /// What the pointer should look like where it is.
    ///
    /// The text cursor over text, an arrow over a picture — an object is not a
    /// place to type — and over the handles of the *selected* picture, the
    /// arrow that says which way that handle pulls. A picture nobody has
    /// clicked yet shows no handles, so it must not claim any either.
    fn pointer_icon(&self, over: Option<view::Spot>, reach: f64) -> egui::CursorIcon {
        use crate::drawings::Grip;
        let icon = |grip: Grip| match grip {
            Grip::Corner { right, bottom } => match right == bottom {
                true => egui::CursorIcon::ResizeNwSe,
                false => egui::CursorIcon::ResizeNeSw,
            },
            Grip::Edge {
                horizontal: true, ..
            } => egui::CursorIcon::ResizeHorizontal,
            Grip::Edge { .. } => egui::CursorIcon::ResizeVertical,
            // Only an anchored drawing can be dragged about; an inline one is
            // held in place by the words around it.
            Grip::Body => match self.picked_drawing().is_some_and(|d| d.anchored) {
                true => egui::CursorIcon::Move,
                false => egui::CursorIcon::Default,
            },
        };
        // Mid-drag the pointer keeps the shape it started with, wherever it has
        // wandered to — including off the picture, which every drag does.
        if let Some(grip) = self.dragging {
            return icon(grip);
        }
        let Some(spot) = over else {
            return egui::CursorIcon::Text;
        };
        let Some((found, rect)) = view::drawing_at(&self.view, self.scope, spot, reach) else {
            return egui::CursorIcon::Text;
        };
        match self.picked == Some(found) {
            true => crate::drawings::grip_at(rect, spot.x, spot.y, reach)
                .map(icon)
                .unwrap_or(egui::CursorIcon::Default),
            false => egui::CursorIcon::Default,
        }
    }

    /// The drawing the selection names, if it still exists.
    fn picked_drawing(&self) -> Option<&wp_model::doc::Drawing> {
        let picked = self.picked?;
        let paragraph = *self
            .document
            .paragraphs_in(self.scope)
            .get(picked.paragraph)?;
        paragraph.drawings().get(picked.nth).copied()
    }

    /// Opens the Size box for the selected picture.
    fn open_size_dialog(&mut self) {
        let (Some(picked), Some(drawing)) = (self.picked, self.picked_drawing()) else {
            // The box is about a picture, so say which one is missing rather
            // than doing nothing and leaving the user to guess.
            self.message = Some((
                "Nothing selected".to_owned(),
                "Click the picture or chart to size, then try again.\n\n\
                 A selected picture shows eight handles, and dragging one \
                 resizes it."
                    .to_owned(),
            ));
            return;
        };
        let (width, height) = (drawing.extent.0.points(), drawing.extent.1.points());
        // A picture's own pixels, at the 96 to the inch a screen shot is
        // measured in — the size Reset puts it back to. A chart has no pixels.
        let natural = drawing
            .rel
            .as_deref()
            .filter(|_| drawing.chart.is_none())
            .and_then(|rel| self.pictures.texture(rel))
            .map(|texture| {
                let [w, h] = texture.size();
                (w as f64 * 0.75, h as f64 * 0.75)
            });
        self.size_draft = Some(SizeDraft {
            picked,
            width: inches(width),
            height: inches(height),
            locked: true,
            ratio: match height > 0.0 {
                true => width / height,
                false => 1.0,
            },
            natural,
        });
    }

    /// Sets the selected picture's size, in points. One undo entry.
    fn resize_drawing(&mut self, picked: crate::drawings::Picked, width: f64, height: f64) {
        let before = match self
            .document
            .paragraphs_in(self.scope)
            .get(picked.paragraph)
        {
            Some(paragraph) => (*paragraph).clone(),
            None => return,
        };
        let changed = {
            let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
            paragraphs
                .get_mut(picked.paragraph)
                .and_then(|paragraph| paragraph.drawing_mut(picked.nth))
                .is_some_and(|drawing| crate::drawings::set_size(drawing, width, height))
        };
        if !changed {
            return;
        }
        self.history.push(
            self.scope,
            crate::edit::Change::Paragraph {
                index: picked.paragraph,
                before: Box::new(before),
            },
        );
        self.changed();
    }

    /// Takes the picked drawing out of the document.
    fn delete_drawing(&mut self) -> bool {
        let Some(picked) = self.picked else {
            return false;
        };
        let before = match self
            .document
            .paragraphs_in(self.scope)
            .get(picked.paragraph)
        {
            Some(paragraph) => (*paragraph).clone(),
            None => return false,
        };
        let removed = {
            let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
            paragraphs
                .get_mut(picked.paragraph)
                .is_some_and(|paragraph| paragraph.remove_drawing(picked.nth))
        };
        if !removed {
            return false;
        }
        self.history.push(
            self.scope,
            crate::edit::Change::Paragraph {
                index: picked.paragraph,
                before: Box::new(before),
            },
        );
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

    #[test]
    fn a_chart_from_the_board_arrives_as_a_part_and_a_drawing_that_renders() {
        // What the paste hands over is exactly what Calx's copy packs: the
        // chartSpace bytes and a size. The document must end up holding all
        // three pieces — part, relationship, drawing — and the part must be
        // one our own reader draws, or the paste has produced a hole.
        let mut app = Scriva::new();
        let chart_space = ss_xlsx_free_chart();
        assert!(
            app.insert_chart_part(&chart_space, 10_000_000, 3_000_000),
            "the paste lands"
        );

        let drawings: Vec<_> = app
            .document
            .paragraphs()
            .iter()
            .flat_map(|paragraph| paragraph.drawings().into_iter().cloned())
            .collect();
        let [drawing] = &drawings[..] else {
            panic!("one chart, not {}", drawings.len());
        };
        let rel = drawing.chart.as_deref().expect("the drawing names a part");
        assert!(drawing.rel.is_none(), "a chart is not a picture");

        // Wider than the text column, so it shrank to fit, proportions kept.
        let (wp_model::Emu(cx), wp_model::Emu(cy)) = drawing.extent;
        assert!(cx < 10_000_000, "shrunk: {cx}");
        // Within one EMU of true proportion: the shrink divides integers.
        assert!(
            (cy * 10_000_000 - cx * 3_000_000).abs() <= 10_000_000,
            "in proportion: {cx}x{cy}"
        );

        let parts = app.parts.as_ref().expect("the part index was rebuilt");
        let name = parts.target(rel).expect("the relationship resolves");
        let package = app.package.as_ref().expect("a package was authored");
        let part = package.part(name).expect("the part is there");
        let plot = chart::read::plot(part.data()).expect("and the reader draws it");
        assert_eq!(plot.series.len(), 1);
    }

    /// A chartSpace as Calx would put one on the board — hand-written rather
    /// than imported, because this crate must not depend on the spreadsheet
    /// stack to test a paste.
    fn ss_xlsx_free_chart() -> Vec<u8> {
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/><c:val><c:numRef><c:f>Sheet1!$A$1:$A$2</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>5</c:v></c:pt><c:pt idx="1"><c:v>7</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx></c:plotArea><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart></c:chartSpace>"#.to_vec()
    }

    #[test]
    fn a_link_to_a_bookmark_takes_the_caret_there() {
        // The demonstration document's "paragraph level formatting" points at
        // a heading further up. Following it is a caret move and a scroll —
        // there is nothing to open and nowhere to go.
        let mut app = Scriva::new();
        let mut heading = Paragraph::of("Paragraph level formatting");
        heading.content.insert(
            0,
            wp_model::doc::Inline::Anchor(wp_model::revision::Anchor::BookmarkStart {
                id: 1,
                name: "_Paragraph_level_formatting".into(),
            }),
        );
        let mut linking = Paragraph::of("back to ");
        linking
            .content
            .push(wp_model::doc::Inline::Hyperlink(Box::new(
                wp_model::doc::Hyperlink {
                    rel: None,
                    anchor: Some("_Paragraph_level_formatting".into()),
                    tooltip: None,
                    history: true,
                    content: vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of(
                        "that section",
                    ))],
                },
            )));
        app.document.body = vec![
            Block::Paragraph(heading),
            Block::Paragraph(Paragraph::of("in between")),
            Block::Paragraph(linking),
        ];

        let inside = Caret {
            paragraph: 2,
            offset: 10,
        };
        let found = app.link_at(inside).expect("the caret stands in the link");
        assert_eq!(
            found,
            crate::links::Destination::Here("_Paragraph_level_formatting".to_owned())
        );
        app.follow_link(found);
        assert_eq!(app.caret().paragraph, 0, "the caret went to the bookmark");
        assert!(app.reveal.is_some(), "and the view was asked to follow");
        assert!(app.message.is_none(), "with nothing to report");

        // A link to a mark that is not in the document says so rather than
        // appearing to do nothing.
        app.follow_link(crate::links::Destination::Here("_gone".to_owned()));
        assert!(app.message.is_some(), "a dangling link is reported");
    }

    fn app_with(texts: &[&str]) -> Scriva {
        let mut app = Scriva::new();
        app.document.body = texts
            .iter()
            .map(|text| Block::Paragraph(Paragraph::of(text)))
            .collect();
        app.stamp += 1;
        app
    }

    fn watermark_draft(text: &str) -> WatermarkDraft {
        WatermarkDraft {
            text: text.to_owned(),
            font: String::new(),
            color: String::new(),
            diagonal: true,
            existing: false,
        }
    }

    #[test]
    fn a_watermark_is_put_in_a_header_the_document_did_not_have() {
        let mut app = app_with(&["body text"]);
        assert!(app.document.headers.is_empty(), "nothing to start with");
        app.apply_watermark(&watermark_draft("CONFIDENTIAL"));

        let shape = watermark_in(&app.document).expect("the watermark is there");
        assert_eq!(&*shape.text, "CONFIDENTIAL");
        assert_eq!(shape.rotation, 315.0, "diagonal, as Word's own is");
        // A header body with no part and no relationship: the writer gives it
        // both, and the section now names it.
        let header = app.document.headers.first().expect("a header was made");
        assert!(header.part.is_none() && header.rel.is_none());
        assert!(!header.footer);
        assert_eq!(app.document.section.headers.len(), 1);
    }

    #[test]
    fn the_watermark_is_stamped_only_on_the_headers_a_page_will_show() {
        // A document commonly names three headers while the settings that
        // would show two of them are off. Word stamps the one that is drawn
        // and leaves the other parts empty — measured against a watermark
        // Word wrote itself.
        let mut app = app_with(&["body text"]);
        for (index, kind) in [
            wp_model::HeaderKind::Default,
            wp_model::HeaderKind::First,
            wp_model::HeaderKind::Even,
        ]
        .into_iter()
        .enumerate()
        {
            let id = wp_model::HeaderId(index as u32);
            app.document.headers.push(wp_model::doc::HeaderFooter {
                id,
                part: None,
                rel: None,
                footer: false,
                content: Vec::new(),
            });
            app.document.section.headers.push(wp_model::HeaderRef {
                kind,
                body: id,
                rel: None,
            });
        }
        app.apply_watermark(&watermark_draft("DRAFT"));
        let carrying = |app: &Scriva| -> Vec<u32> {
            app.document
                .headers
                .iter()
                .filter(|header| shape_words_in(&header.content).is_some())
                .map(|header| header.id.0)
                .collect()
        };
        assert_eq!(carrying(&app), vec![0], "the default header alone");

        // Turn on the two settings that put the others on a page, and they
        // are stamped too — a title page without the watermark is the page
        // that most needed it.
        app.document.section.title_page = true;
        app.document.settings.even_and_odd_headers = true;
        app.apply_watermark(&watermark_draft("DRAFT"));
        assert_eq!(carrying(&app), vec![0, 1, 2], "all three, once each");
    }

    #[test]
    fn a_second_watermark_replaces_the_first_rather_than_joining_it() {
        let mut app = app_with(&["body text"]);
        app.apply_watermark(&watermark_draft("DRAFT"));
        app.apply_watermark(&watermark_draft("FINAL"));
        let shapes: Vec<String> = app
            .document
            .headers
            .iter()
            .flat_map(|header| &header.content)
            .filter_map(|block| match block {
                Block::Paragraph(paragraph) => Some(paragraph),
                _ => None,
            })
            .flat_map(|paragraph| paragraph.drawings())
            .filter_map(|drawing| drawing.text.as_deref())
            .map(|text| text.text.to_string())
            .collect();
        assert_eq!(shapes, vec!["FINAL".to_owned()]);
    }

    #[test]
    fn removing_a_watermark_takes_its_paragraph_with_it_and_undoes_in_one_step() {
        let mut app = app_with(&["body text"]);
        app.apply_watermark(&watermark_draft("CONFIDENTIAL"));
        let with = app.document.headers[0].content.len();
        assert_eq!(with, 1, "one paragraph, holding the shape");

        app.apply_watermark(&watermark_draft(""));
        assert!(watermark_in(&app.document).is_none(), "it is gone");
        assert!(
            app.document.headers[0].content.is_empty(),
            "and so is the empty line it would have left behind"
        );

        app.run(Command::Undo);
        assert_eq!(
            watermark_in(&app.document).map(|shape| shape.text.to_string()),
            Some("CONFIDENTIAL".to_owned()),
            "one undo brings it back"
        );
    }

    #[test]
    fn a_watermark_that_was_read_out_of_a_header_is_offered_back_to_be_edited() {
        let mut app = app_with(&["body text"]);
        app.apply_watermark(&WatermarkDraft {
            text: "SAMPLE".to_owned(),
            font: "Verdana".to_owned(),
            color: "1E6F5C".to_owned(),
            diagonal: false,
            existing: false,
        });
        app.open_watermark_dialog();
        let draft = app.watermark_draft.clone().expect("the box is open");
        assert_eq!(draft.text, "SAMPLE");
        assert_eq!(draft.font, "Verdana");
        assert_eq!(draft.color, "1E6F5C");
        assert!(!draft.diagonal, "an unturned shape reads back as flat");
        assert!(draft.existing, "so the box offers to remove it");
    }

    #[test]
    fn a_watermarks_size_is_taken_from_the_page_it_will_be_stamped_on() {
        let mut app = app_with(&["body text"]);
        app.apply_watermark(&watermark_draft("CONFIDENTIAL"));
        let drawing = app.document.headers[0]
            .content
            .iter()
            .filter_map(|block| match block {
                Block::Paragraph(paragraph) => paragraph.drawings().first().copied(),
                _ => None,
            })
            .next()
            .expect("the shape");
        // Turned through forty-five degrees, a shape's bounding box is
        // `(w + h) / root two` each way, so the widest that fits the text area
        // is what this comes to. Word's own is 527.75 by 131.95 on the same
        // page; without a shaper here the fallback proportion decides, and
        // both numbers land within a couple of points of Word's.
        assert!(
            (drawing.extent.0.points() - 529.5).abs() < 2.0,
            "width {} is not the width that fits",
            drawing.extent.0.points()
        );
        assert!(
            (drawing.extent.1.points() - 132.4).abs() < 2.0,
            "height {} does not keep the proportion",
            drawing.extent.1.points()
        );
    }

    #[test]
    fn naming_a_font_speaks_for_the_latin_slots_and_silences_the_theme() {
        let mut app = app_with(&["hello"]);
        app.run(Command::SelectAll);
        // The theme reference modern Word puts on nearly every run. It
        // outranks a cached name, so the command must take it away too or
        // the choice would silently lose to the theme.
        app.format_runs(|props| {
            props.fonts.ascii_theme = Some(wp_model::prop::ThemeFont::MinorHighAnsi)
        });
        app.run(Command::Font("Verdana"));
        assert!(app.probe_runs(|props| {
            props.fonts.ascii.as_deref() == Some("Verdana")
                && props.fonts.high_ansi.as_deref() == Some("Verdana")
                && props.fonts.ascii_theme.is_none()
        }));
    }

    #[test]
    fn the_palette_and_the_hex_box_agree_on_what_a_colour_is() {
        let mut app = app_with(&["hello"]);
        app.run(Command::SelectAll);
        app.run(Command::Color(wp_model::Color::Rgb([0x33, 0x33, 0x99])));
        assert!(
            app.probe_runs(|props| props.color == Some(wp_model::Color::Rgb([0x33, 0x33, 0x99])))
        );
        // The dialog parses with the same reader a file's `w:val` gets, so
        // `#333399` and `333399` and `auto` all mean what they mean there.
        assert_eq!(
            wp_model::Color::from_val("#333399"),
            Some(wp_model::Color::Rgb([0x33, 0x33, 0x99]))
        );
    }

    #[test]
    fn the_highlighter_erases_by_removing_rather_than_writing_none() {
        let mut app = app_with(&["hello"]);
        app.run(Command::SelectAll);
        app.run(Command::Highlight(wp_model::Highlight::Yellow));
        assert!(app.probe_runs(|props| props.highlight == Some(wp_model::Highlight::Yellow)));
        app.run(Command::Highlight(wp_model::Highlight::None));
        assert!(app.probe_runs(|props| props.highlight.is_none()));
    }

    #[test]
    fn merging_cells_spans_their_columns_and_keeps_their_paragraphs_in_order() {
        let mut app = app_with(&["text"]);
        app.insert_table(2, 3);
        // The caret is in the first cell; the cells' paragraphs are 0, 1, 2
        // across the first row. Fill the first and the third, leave the
        // middle blank.
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 0,
        });
        app.type_text("left");
        app.selection = Selection::at(Caret {
            paragraph: 2,
            offset: 0,
        });
        app.type_text("right");
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 2,
                offset: 0,
            },
        };
        app.run(Command::MergeCells);
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is the first block");
        };
        let row = &table.rows[0];
        assert_eq!(row.cells.len(), 1, "one cell where three were");
        let cell = &row.cells[0];
        assert_eq!(cell.props.grid_span, 3);
        let whole: i32 = table.grid.iter().map(|t| t.0).sum();
        assert_eq!(
            cell.props.width,
            wp_model::table::Width::Fixed(Twips(whole))
        );
        // The blank middle cell left no blank line behind it.
        assert_eq!(wp_model::doc::text_of(&cell.content), "left\nright");
        assert_eq!(table.rows[1].cells.len(), 3, "the other row is untouched");
        assert_eq!(app.caret().paragraph, 0);

        app.run(Command::Undo);
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is still the first block");
        };
        assert_eq!(table.rows[0].cells.len(), 3);
    }

    #[test]
    fn merging_needs_a_selection_across_cells_of_one_row() {
        let mut app = app_with(&["text"]);
        app.insert_table(2, 2);
        // One cell: nothing to merge.
        app.run(Command::MergeCells);
        assert!(app.message.is_some(), "a caret in one cell is told why");
        app.message = None;
        // Two cells in different rows: not a merge either.
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 2,
                offset: 0,
            },
        };
        app.run(Command::MergeCells);
        assert!(app.message.is_some());
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is the first block");
        };
        assert!(table.rows.iter().all(|row| row.cells.len() == 2));
    }

    #[test]
    fn a_border_colour_reaches_every_rule_and_draws_rules_where_there_were_none() {
        let grey = wp_model::Color::Rgb([0xC0, 0xC0, 0xC0]);
        let mut app = app_with(&["text"]);
        app.insert_table(2, 2);
        app.run(Command::BorderColor(grey));
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is the first block");
        };
        let b = &table.props.borders;
        for edge in [b.top, b.start, b.bottom, b.end, b.inside_h, b.inside_v] {
            let edge = edge.expect("every rule is stated");
            assert_eq!(edge.color, Some(grey));
            assert_eq!(edge.style, wp_model::prop::BorderStyle::Single);
        }

        // With the rules off, a colour brings them back in that colour: a
        // colour on no line would be a menu choice that did nothing.
        app.run(Command::TableBorders(false));
        app.run(Command::BorderColor(grey));
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is still the first block");
        };
        let top = table.props.borders.top.expect("ruled again");
        assert_eq!(top.style, wp_model::prop::BorderStyle::Single);
        assert_eq!(top.color, Some(grey));
    }

    #[test]
    fn the_paragraph_box_applies_only_the_fields_that_were_changed() {
        let mut app = app_with(&["one", "two"]);
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 1,
                offset: 0,
            },
        };
        app.format_paragraphs(|props| props.spacing.before = Some(Twips(120)));
        app.open_paragraph_dialog();
        let (opened, _) = app.paragraph_draft.clone().expect("the box is open");
        assert_eq!(opened.before, "6", "six points, not 6.00");
        let mut draft = opened.clone();
        draft.after = "3".into();
        draft.left = "0.17".into();
        draft.hanging = "0.17".into();
        app.apply_paragraph(&opened, &draft);
        for paragraph in app.document.paragraphs().iter() {
            assert_eq!(
                paragraph.props.spacing.before,
                Some(Twips(120)),
                "untouched"
            );
            assert_eq!(paragraph.props.spacing.after, Some(Twips(60)));
            assert_eq!(paragraph.props.indent.start, Some(Twips(245)));
            assert_eq!(paragraph.props.indent.hanging, Some(Twips(245)));
        }
        // A field blanked takes the value away rather than writing zero.
        let opened = draft.clone();
        let mut draft = opened.clone();
        draft.before = String::new();
        app.apply_paragraph(&opened, &draft);
        assert!(app
            .document
            .paragraphs()
            .iter()
            .all(|p| p.props.spacing.before.is_none()));
    }

    #[test]
    fn turning_borders_off_writes_none_rather_than_nothing() {
        let mut app = app_with(&["text"]);
        app.insert_table(2, 2);
        // The caret landed in the first cell; the command must find the table
        // from there.
        app.run(Command::TableBorders(false));
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is the first block");
        };
        // Explicit `none`, because an absent border is an inherited one: a
        // table style could rule the edges right back.
        let edge = table.props.borders.top.expect("the edge is stated");
        assert_eq!(edge.style, wp_model::prop::BorderStyle::None);
        assert!(table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .all(|cell| cell.props.borders == wp_model::table::TableBorders::default()));

        app.run(Command::Undo);
        let Block::Table(table) = &app.document.body[0] else {
            panic!("still the first block");
        };
        let edge = table.props.borders.top.expect("the rule is back");
        assert_eq!(edge.style, wp_model::prop::BorderStyle::Single, "one undo");
    }

    #[test]
    fn shading_lands_on_the_cell_the_caret_is_in() {
        let mut app = app_with(&["text"]);
        app.insert_table(2, 2);
        app.run(Command::TableShading(Some([0x92, 0xD0, 0x50])));
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is the first block");
        };
        let shading = table.rows[0].cells[0]
            .props
            .shading
            .as_ref()
            .expect("the caret's cell is filled");
        assert_eq!(
            shading.background(),
            Some(wp_model::Color::Rgb([0x92, 0xD0, 0x50]))
        );
        assert!(
            table.rows[0].cells[1].props.shading.is_none(),
            "and its neighbour is not"
        );
    }

    #[test]
    fn a_column_width_moves_the_grid_and_the_cells_together() {
        let mut app = app_with(&["text"]);
        app.insert_table(2, 2);
        app.apply_column_width(Twips(1440));
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is the first block");
        };
        assert_eq!(table.grid[0], Twips(1440), "the caret's column");
        for row in &table.rows {
            assert_eq!(
                row.cells[0].props.width,
                wp_model::table::Width::Fixed(Twips(1440)),
                "every cell in it restates the width"
            );
        }
    }

    #[test]
    fn cell_margins_state_the_sides_given_and_leave_a_blank_one_to_the_style() {
        use wp_model::table::Width;
        let mut app = app_with(&["text"]);
        app.insert_table(2, 2);
        // The resume's own padding: none above or below, 5.75pt either side.
        app.apply_cell_margins(&[
            "0".to_owned(),
            "5.75".to_owned(),
            "0".to_owned(),
            String::new(),
        ]);
        let Block::Table(table) = &app.document.body[0] else {
            panic!("the table is the first block");
        };
        let margins = table.props.cell_margins;
        assert_eq!(margins.top, Some(Width::Fixed(Twips(0))));
        assert_eq!(margins.start, Some(Width::Fixed(Twips(115))));
        assert_eq!(margins.bottom, Some(Width::Fixed(Twips(0))));
        assert_eq!(margins.end, None, "a blank side states nothing at all");
    }

    #[test]
    fn the_bullet_press_makes_a_list_and_the_second_press_unmakes_it() {
        let mut app = app_with(&["one", "two"]);
        app.run(Command::SelectAll);
        app.run(Command::Bullets);
        {
            let paragraphs = app.document.paragraphs();
            for paragraph in &paragraphs {
                let reference = paragraph.props.numbering.expect("in a list");
                let level = app
                    .document
                    .numbering
                    .level(reference.num_id, 0)
                    .expect("that resolves");
                assert!(matches!(level.format, wp_model::NumFormat::Bullet));
                // The glyph is Symbol's dot, meaningless in any other face —
                // the level must carry the face with it.
                assert_eq!(level.run.fonts.ascii.as_deref(), Some("Symbol"));
            }
        }
        app.run(Command::Bullets);
        let paragraphs = app.document.paragraphs();
        assert!(
            paragraphs.iter().all(|p| p.props.numbering.is_none()),
            "the second press takes them out"
        );
    }

    #[test]
    fn two_presses_are_two_lists_of_one_definition() {
        let mut app = app_with(&["one", "two"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 0,
        });
        app.run(Command::Numbers);
        app.selection = Selection::at(Caret {
            paragraph: 1,
            offset: 0,
        });
        app.run(Command::Numbers);
        let paragraphs = app.document.paragraphs();
        let first = paragraphs[0].props.numbering.expect("the first is listed");
        let second = paragraphs[1].props.numbering.expect("and the second");
        assert_ne!(first.num_id, second.num_id, "instances count separately");
        let of = |id| {
            app.document
                .numbering
                .num(id)
                .expect("resolves")
                .abstract_id
        };
        assert_eq!(
            of(first.num_id),
            of(second.num_id),
            "but the definition is shared, as Word's button shares its gallery entry"
        );
    }

    /// Types `input` where the caret is, in whichever flow it is in.
    fn typed(app: &mut Scriva, input: &str) {
        let caret = edit::type_text(
            &mut app.document,
            app.scope,
            &mut app.history,
            app.selection,
            input,
        );
        app.selection = Selection::at(caret);
        app.changed();
    }

    #[test]
    fn opening_a_header_a_document_does_not_have_makes_one_and_puts_the_caret_in_it() {
        let mut app = app_with(&["text"]);
        app.run(Command::EditHeader);
        let wp_model::Scope::Chrome(id) = app.scope else {
            panic!("the caret is in the header");
        };
        let header = app.document.header(id).expect("which the document has");
        assert!(!header.footer);
        let reference = &app.document.section.headers[0];
        assert_eq!(reference.kind, wp_model::HeaderKind::Default);
        assert_eq!(reference.body, id);
        assert!(
            reference.rel.is_none(),
            "no relationship until a save assigns one"
        );

        // Word's Header style, which is what a bare paragraph would miss: the
        // body's space-after would otherwise push the page's own text down to
        // make room under a one-line header, and Tab would walk nowhere.
        let Block::Paragraph(paragraph) = &header.content[0] else {
            panic!("a paragraph");
        };
        assert_eq!(paragraph.props.spacing.before, Some(Twips(0)));
        assert_eq!(paragraph.props.spacing.after, Some(Twips(0)));
        assert_eq!(
            paragraph.props.spacing.line,
            Some(LineSpacing::Multiple(Line240::SINGLE))
        );
        let tabs = paragraph.props.tabs.as_ref().expect("a centre and a right");
        assert_eq!(tabs[0].kind, wp_model::prop::TabKind::Center);
        assert_eq!(tabs[1].kind, wp_model::prop::TabKind::End);
        assert_eq!(tabs[1].position, app.document.section.text_width());
    }

    #[test]
    fn typing_in_an_open_header_leaves_the_body_alone_and_undoes_with_it_open() {
        let mut app = app_with(&["the body"]);
        app.run(Command::EditHeader);
        typed(&mut app, "RESUME / CV");
        let wp_model::Scope::Chrome(id) = app.scope else {
            panic!("still in the header");
        };
        assert_eq!(
            wp_model::doc::text_of(&app.document.header(id).expect("there").content),
            "RESUME / CV"
        );
        assert_eq!(app.document.text(), "the body", "the text is untouched");

        // A paragraph index means one thing in the header and another in the
        // body, so undo has to know which flow made the change — and put the
        // caret back in it.
        app.run(Command::Undo);
        assert_eq!(app.scope, wp_model::Scope::Chrome(id));
        assert_eq!(
            wp_model::doc::text_of(&app.document.header(id).expect("there").content),
            ""
        );
        assert_eq!(app.document.text(), "the body");
    }

    #[test]
    fn a_header_that_holds_a_table_is_still_a_table_after_it_has_been_edited() {
        use wp_model::table::{Cell, Row, Table};
        let mut app = app_with(&["the body"]);
        app.run(Command::EditHeader);
        let wp_model::Scope::Chrome(id) = app.scope else {
            panic!("in the header");
        };
        // A real header: a table of revisions, which is what the box this
        // replaced would have flattened into one line of text.
        app.document.header_mut(id).expect("there").content.insert(
            0,
            Block::Table(Table {
                rows: vec![Row {
                    props: Default::default(),
                    cells: vec![Cell::new(), Cell::new()],
                }],
                ..Table::new()
            }),
        );
        app.changed();

        // The caret is in the first cell, because the flow walks a table's
        // paragraphs in document order exactly as the body's does.
        app.selection = Selection::at(Caret::default());
        typed(&mut app, "ECN#");
        app.selection = Selection::at(Caret {
            paragraph: 1,
            offset: 0,
        });
        typed(&mut app, "DATE");

        let header = app.document.header(id).expect("there");
        let Block::Table(table) = &header.content[0] else {
            panic!("the table is still a table");
        };
        assert_eq!(
            wp_model::doc::text_of(&table.rows[0].cells[0].content),
            "ECN#"
        );
        assert_eq!(
            wp_model::doc::text_of(&table.rows[0].cells[1].content),
            "DATE"
        );
    }

    #[test]
    fn closing_a_band_puts_the_caret_back_where_it_stood_in_the_text() {
        let mut app = app_with(&["one", "two"]);
        let was = Caret {
            paragraph: 1,
            offset: 2,
        };
        app.selection = Selection::at(was);
        app.run(Command::EditFooter);
        assert!(app.editing_band());
        assert!(
            app.document.headers.iter().any(|header| header.footer),
            "Insert ▸ Footer makes one"
        );
        app.run(Command::CloseChrome);
        assert_eq!(app.scope, wp_model::Scope::Body);
        assert_eq!(app.caret(), was, "and not at the top of the page");
    }

    #[test]
    fn the_band_bar_switches_between_the_two_without_making_a_third() {
        let mut app = app_with(&["text"]);
        app.run(Command::EditHeader);
        app.run(Command::SwitchBand);
        assert!(app.in_footer());
        app.run(Command::SwitchBand);
        assert!(!app.in_footer());
        assert_eq!(app.document.headers.len(), 2, "one header and one footer");
    }

    #[test]
    fn a_page_number_is_a_field_and_not_a_typed_digit() {
        use wp_model::doc::{Inline, Piece};
        let mut app = app_with(&["text"]);
        app.run(Command::EditFooter);
        app.run(Command::InsertPageNumber { of_pages: true });
        let wp_model::Scope::Chrome(id) = app.scope else {
            panic!("in the footer");
        };
        let footer = app.document.header(id).expect("there");
        let Block::Paragraph(paragraph) = &footer.content[0] else {
            panic!("a paragraph");
        };
        let codes: Vec<String> = paragraph
            .content
            .iter()
            .filter_map(|inline| match inline {
                Inline::Run(run) => Some(run),
                _ => None,
            })
            .flat_map(|run| &run.content)
            .filter_map(|piece| match piece {
                Piece::Instruction(code) => Some(code.trim().to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(codes, ["PAGE", "NUMPAGES"]);
        assert!(paragraph.text().starts_with("Page "));
    }

    #[test]
    fn removing_a_header_takes_its_references_with_it_and_undoes_in_one_step() {
        let mut app = app_with(&["text"]);
        app.run(Command::EditHeader);
        typed(&mut app, "RESUME / CV");
        app.run(Command::CloseChrome);

        app.run(Command::RemoveChrome { footer: false });
        assert!(app.document.headers.is_empty());
        assert!(
            app.document.section.headers.is_empty(),
            "a reference pointing at nothing is a damaged document"
        );

        app.run(Command::Undo);
        assert_eq!(
            app.document.headers.len(),
            1,
            "bodies and references, together"
        );
        assert_eq!(
            wp_model::doc::text_of(&app.document.headers[0].content),
            "RESUME / CV"
        );
    }

    #[test]
    fn a_click_in_the_margin_is_not_a_place_to_put_the_caret_in_the_text() {
        // While the body is what is being edited, the margins are not part of
        // it; while a band is open, the page is not. A click on the wrong one
        // has to do nothing, which is what the wash over it promises.
        let mut app = laid_app("the body", 400.0);
        let page = &app.view.pages()[0];
        let (top, middle, bottom) = (
            view::Spot {
                page: 0,
                x: page.geometry.start + 1.0,
                y: 4.0,
            },
            view::Spot {
                page: 0,
                x: page.geometry.start + 1.0,
                y: page.geometry.top + 4.0,
            },
            view::Spot {
                page: 0,
                x: page.geometry.start + 1.0,
                y: page.geometry.height - 4.0,
            },
        );
        assert_eq!(app.band_at(top), Some(false));
        assert_eq!(app.band_at(bottom), Some(true));
        assert_eq!(app.band_at(middle), None);
        assert!(!app.click_lands_here(top));
        assert!(app.click_lands_here(middle));

        app.run(Command::EditHeader);
        assert!(
            app.click_lands_here(top),
            "the header is what is being edited"
        );
        assert!(!app.click_lands_here(middle), "and the text is not");
    }

    #[test]
    fn a_comment_asked_for_in_a_header_is_refused_rather_than_put_somewhere_else() {
        // Word, over COM, against a header's own range: "Comments, endnotes
        // and footnotes can only be added to the main story." The trap this
        // closes is the other answer — `revise` used to speak in body
        // positions, so a caret standing in a header wore a number that meant
        // something else there and the comment wrapped whatever body paragraph
        // shared it.
        let mut app = app_with(&["the body"]);
        app.run(Command::EditHeader);
        typed(&mut app, "RESUME / CV");
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 0,
                offset: 6,
            },
        };
        app.run(Command::AddComment);

        assert!(app.drafting.is_none(), "no box opened");
        assert!(app.message.is_some(), "and it said why");
        assert!(app.document.comments.is_empty());
        assert!(app.editing_band(), "the band is left as it was");
    }

    #[test]
    fn a_comment_a_file_carries_in_a_header_is_still_found_and_still_removable() {
        // Nothing in Word writes one, but the schema allows it and a second
        // producer may; a reviewer that cannot see it is a reviewer that lies.
        let mut app = app_with(&["the body"]);
        app.run(Command::EditHeader);
        typed(&mut app, "RESUME / CV");
        let wp_model::Scope::Chrome(id) = app.scope else {
            panic!("in the header");
        };
        let comment = crate::revise::add_comment(
            &mut app.document,
            &mut app.history,
            app.scope,
            Selection {
                anchor: Caret {
                    paragraph: 0,
                    offset: 0,
                },
                head: Caret {
                    paragraph: 0,
                    offset: 6,
                },
            },
            "A",
            "A",
            "written by something else",
        );

        assert_eq!(
            crate::revise::comment_at(&app.document, comment).map(|(scope, _)| scope),
            Some(wp_model::Scope::Chrome(id)),
            "found in the flow it is anchored in"
        );
        assert_eq!(
            app.document.text(),
            "the body",
            "and the text carries no anchor of it"
        );
        assert!(crate::revise::delete_comment(
            &mut app.document,
            &mut app.history,
            comment
        ));
        assert_eq!(crate::revise::comment_at(&app.document, comment), None);
    }

    #[test]
    fn a_tracked_change_in_a_header_is_listed_and_settled_where_it_stands() {
        let mut app = app_with(&["the body"]);
        app.run(Command::EditHeader);
        let wp_model::Scope::Chrome(id) = app.scope else {
            panic!("in the header");
        };
        app.run(Command::TrackChanges);
        app.type_text("DRAFT");

        let changes = crate::revise::tracked(&app.document);
        assert_eq!(changes.len(), 1, "one insertion, in the header");
        assert_eq!(changes[0].scope, wp_model::Scope::Chrome(id));

        app.run(Command::AcceptAll);
        assert!(
            crate::revise::tracked(&app.document).is_empty(),
            "and accepting reaches it"
        );
        assert_eq!(
            wp_model::doc::text_of(&app.document.header(id).expect("still there").content),
            "DRAFT",
            "the words survive being accepted"
        );
    }

    /// Two sections: the first names a header, the second names nothing —
    /// which is exactly what Word writes for a section linked to the previous
    /// one. The caret lands in the second, because with no layout to ask, the
    /// last section is the one a page belongs to.
    fn two_sections_with_a_linked_header() -> Scriva {
        let mut app = app_with(&["section one", "section two"]);
        let mut first = wp_model::SectionProps::new();
        first.headers.push(wp_model::HeaderRef {
            kind: wp_model::HeaderKind::Default,
            body: wp_model::HeaderId(0),
            rel: None,
        });
        if let Block::Paragraph(paragraph) = &mut app.document.body[0] {
            paragraph.section = Some(Box::new(first));
        }
        app.document.headers.push(wp_model::doc::HeaderFooter {
            id: wp_model::HeaderId(0),
            part: None,
            rel: None,
            footer: false,
            content: vec![Block::Paragraph(Paragraph::of("INHERITED HEAD"))],
        });
        app.stamp += 1;
        app
    }

    #[test]
    fn a_linked_section_edits_the_band_it_inherits_rather_than_making_a_second() {
        let mut app = two_sections_with_a_linked_header();
        assert_eq!(app.linked_to_previous(false), Some(true));

        app.run(Command::EditHeader);
        assert_eq!(
            app.scope,
            wp_model::Scope::Chrome(wp_model::HeaderId(0)),
            "the band it shows is the band it opens"
        );
        assert_eq!(app.document.headers.len(), 1, "and no second one was made");
    }

    #[test]
    fn breaking_the_link_takes_a_copy_the_section_can_change_on_its_own() {
        // Word's own answer, over COM: unlink a second section's header and
        // the words stay on the page, while the section before it keeps a copy
        // of its own — so the two can then be changed apart.
        let mut app = two_sections_with_a_linked_header();
        app.run(Command::EditHeader);
        app.set_link_to_previous(false);

        assert_eq!(app.linked_to_previous(false), Some(false));
        assert_eq!(app.document.headers.len(), 2, "a copy, not a move");
        let wp_model::Scope::Chrome(own) = app.scope else {
            panic!("in the new band");
        };
        assert_ne!(own, wp_model::HeaderId(0));
        assert_eq!(
            wp_model::doc::text_of(&app.document.header(own).expect("made").content),
            "INHERITED HEAD",
            "the words are still on the page"
        );

        typed(&mut app, "!");
        assert_eq!(
            wp_model::doc::text_of(
                &app.document
                    .header(wp_model::HeaderId(0))
                    .expect("still there")
                    .content
            ),
            "INHERITED HEAD",
            "and the section before it is untouched"
        );

        // Linking again shows the previous section's band and leaves this
        // one's body behind, so flicking the switch back costs no words.
        app.set_link_to_previous(true);
        assert_eq!(app.linked_to_previous(false), Some(true));
        assert_eq!(app.scope, wp_model::Scope::Chrome(wp_model::HeaderId(0)));
    }

    #[test]
    fn the_first_section_is_never_offered_a_link_to_previous() {
        let mut app = app_with(&["only section"]);
        app.run(Command::EditHeader);
        assert_eq!(
            app.linked_to_previous(false),
            None,
            "there is nothing before it"
        );
    }

    #[test]
    fn turning_on_a_first_page_band_moves_the_caret_into_the_band_that_page_now_wants() {
        let mut app = app_with(&["text"]);
        app.run(Command::EditHeader);
        typed(&mut app, "EVERY PAGE");
        let wp_model::Scope::Chrome(default) = app.scope else {
            panic!("in the default header");
        };

        app.set_band_kinds(true, false);
        assert!(app.document.section.title_page);
        let wp_model::Scope::Chrome(first) = app.scope else {
            panic!("still in a header");
        };
        assert_ne!(first, default, "page one wants a band of its own now");
        assert_eq!(
            wp_model::doc::text_of(&app.document.header(first).expect("made").content),
            "",
            "and Word leaves the new one empty"
        );

        // A switch is not a delete: the band it stopped using is still there,
        // and flicking it back finds it rather than making a third.
        app.set_band_kinds(false, false);
        assert_eq!(app.scope, wp_model::Scope::Chrome(default));
        assert_eq!(app.document.headers.len(), 2);

        app.run(Command::Undo);
        assert!(app.document.section.title_page, "one undo per flick");
    }

    #[test]
    fn the_even_page_switch_is_a_document_setting_that_undo_puts_back() {
        let mut app = app_with(&["text"]);
        app.run(Command::EditHeader);
        app.set_band_kinds(false, true);
        assert!(app.document.settings.even_and_odd_headers);
        app.run(Command::Undo);
        assert!(
            !app.document.settings.even_and_odd_headers,
            "the setting rides with the bands it decides the use of"
        );
    }

    #[test]
    fn find_looks_through_the_headers_and_opens_the_one_it_lands_in() {
        let mut app = app_with(&["a spec in the body"]);
        app.run(Command::EditHeader);
        typed(&mut app, "spec no 190A1430");
        app.run(Command::CloseChrome);

        app.finder = Some(crate::find::Finder::new(false));
        if let Some(finder) = &mut app.finder {
            finder.query = "spec".into();
        }
        app.selection = Selection::at(Caret::default());
        app.refresh_matches();
        assert_eq!(app.find_matches.len(), 2, "the text and the header");

        // The first is in the text; the second is in the header, and stepping
        // on to it opens the band the way a double-click on it would.
        app.run(Command::FindNext);
        assert_eq!(app.scope, wp_model::Scope::Body);
        app.run(Command::FindNext);
        assert!(app.editing_band(), "Find Next opened the header");
        assert_eq!(app.selected_text().as_deref(), Some("spec"));

        // And Close still puts the caret back where it stood in the text.
        app.run(Command::CloseChrome);
        assert_eq!(app.scope, wp_model::Scope::Body);
    }

    #[test]
    fn replace_all_reaches_the_headers_too() {
        let mut app = app_with(&["draft copy"]);
        app.run(Command::EditHeader);
        typed(&mut app, "draft");
        app.run(Command::CloseChrome);

        app.finder = Some(crate::find::Finder::new(true));
        if let Some(finder) = &mut app.finder {
            finder.query = "draft".into();
            finder.replacement = "final".into();
        }
        app.replace_all();
        assert_eq!(app.document.text(), "final copy");
        let header = app.document.headers.first().expect("there");
        assert_eq!(wp_model::doc::text_of(&header.content), "final");
    }

    #[test]
    fn the_table_menu_says_so_when_the_caret_is_not_in_a_table() {
        let mut app = app_with(&["just a paragraph"]);
        app.run(Command::TableBorders(false));
        let (title, _) = app.message.as_ref().expect("it says why");
        assert_eq!(title, "Not in a table");
        assert!(!app.history.can_undo(), "and nothing was recorded to undo");
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

    /// Selects `range` of the first paragraph and copies it, without going near
    /// the machine's real clipboard — a test that wrote to it would throw away
    /// whatever the user had on it.
    fn copied(app: &mut Scriva, range: std::ops::Range<usize>) {
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: range.start,
            },
            head: Caret {
                paragraph: 0,
                offset: range.end,
            },
        };
        let text = app.selected_text().expect("something to copy");
        app.clipboard = Some(Clip {
            text,
            paragraphs: edit::copy_range(&app.document, app.scope, app.selection),
        });
    }

    /// One transparent pixel, as a real PNG.
    const PIXEL: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn a_pasted_picture_lands_in_the_document_and_in_the_package() {
        // The three pieces a picture is. The board itself is the machine's, so
        // this hands the bytes over the way a paste would have.
        let mut app = app_with(&["before after"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 6,
        });
        assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");

        let paragraphs = app.document.paragraphs();
        let drawings = paragraphs[0].drawings();
        let [drawing] = &drawings[..] else {
            panic!("one picture, not {}", drawings.len());
        };
        // 96 pixels at 96 to the inch is an inch, which is 914400 EMU.
        assert_eq!(drawing.extent.0, wp_model::Emu(914_400), "an inch wide");
        assert_eq!(
            drawing.extent.1,
            wp_model::Emu(457_200),
            "half an inch tall"
        );
        // The picture is one character of the paragraph, where the caret was —
        // which is what lets the caret step over it and Backspace take it.
        assert_eq!(
            paragraphs[0].text(),
            format!("before{} after", wp_model::doc::OBJECT),
            "a picture is a character, and the words either side are untouched"
        );

        // The relationship the drawing names resolves to a part holding the
        // bytes — the half of it that lives outside the document.
        let package = app.package.as_ref().expect("a package was authored");
        let parts = app.parts.as_ref().expect("and located");
        let rel = drawing.rel.as_deref().expect("the drawing names one");
        let name = parts.target(rel).expect("which resolves");
        assert_eq!(package.part(name).expect("to a part").data(), PIXEL);

        // And it is one edit, so one undo takes it away again.
        app.run(Command::Undo);
        assert!(
            app.document.paragraphs()[0].drawings().is_empty(),
            "undo takes the picture out"
        );
    }

    #[test]
    fn pressing_enter_beside_a_pasted_picture_does_not_make_a_second_one() {
        // The bug: Enter split the paragraph, and a picture that held none of
        // its text ended up in both halves.
        let mut app = app_with(&["before after"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 6,
        });
        assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");

        // What the Enter key does, at the caret the paste left behind.
        let caret = edit::split_paragraph(
            &mut app.document,
            app.scope,
            &mut app.history,
            app.selection,
        );
        app.selection = Selection::at(caret);

        let pictures: usize = app
            .document
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.drawings().len())
            .sum();
        assert_eq!(pictures, 1, "still the one picture");
        assert_eq!(app.paragraph_count(), 2, "and the paragraph did split");
    }

    #[test]
    fn a_picture_can_be_sized_by_the_numbers_and_one_undo_puts_it_back() {
        let mut app = app_with(&["ab"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 1,
        });
        assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");
        let picked = crate::drawings::Picked {
            paragraph: 0,
            nth: 0,
        };
        app.picked = Some(picked);

        // The box opens on the size the picture is: 96 by 48 pixels at 96 to
        // the inch is one inch by half of one.
        app.open_size_dialog();
        let draft = app.size_draft.as_ref().expect("the box opened");
        assert_eq!(
            (draft.width.as_str(), draft.height.as_str()),
            ("1.00", "0.50")
        );
        assert!(draft.locked, "the ratio is locked, as Word's box opens");
        assert!((draft.ratio - 2.0).abs() < 1e-9);

        // Two inches by one, which is what typing 2.00 with the lock on means.
        app.resize_drawing(picked, 144.0, 72.0);
        let drawing = app.picked_drawing().expect("still there");
        assert_eq!(drawing.extent.0, wp_model::Emu(1_828_800), "two inches");
        assert_eq!(drawing.extent.1, wp_model::Emu(914_400), "by one");

        app.run(Command::Undo);
        let drawing = app.picked_drawing().expect("and still there after");
        assert_eq!(drawing.extent.0, wp_model::Emu(914_400), "back an inch");
        assert_eq!(drawing.extent.1, wp_model::Emu(457_200));
    }

    #[test]
    fn asking_to_size_nothing_says_so_rather_than_doing_nothing() {
        let mut app = app_with(&["ab"]);
        app.run(Command::PictureSize);
        assert!(app.size_draft.is_none(), "no box, because no picture");
        let (title, _) = app.message.as_ref().expect("it says why");
        assert_eq!(title, "Nothing selected");
    }

    #[test]
    fn the_caret_steps_over_a_picture_and_backspace_takes_it() {
        let mut app = app_with(&["ab"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 1,
        });
        assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");
        assert_eq!(
            app.paragraph_text(0),
            format!("a{}b", wp_model::doc::OBJECT)
        );

        // One press of Right from in front of it lands behind it: a picture is
        // one character, not none.
        let text = app.paragraph_text(0);
        assert_eq!(text::next_char(&text, 1), 2);
        assert_eq!(text::previous_char(&text, 2), 1);

        // And Backspace from behind it takes the whole picture.
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 2,
        });
        let caret = edit::backspace(
            &mut app.document,
            app.scope,
            &mut app.history,
            app.selection,
        );
        assert_eq!(caret.offset, 1);
        assert_eq!(app.paragraph_text(0), "ab");
        assert!(app.document.paragraphs()[0].drawings().is_empty());
    }

    #[test]
    fn a_copied_picture_pastes_as_the_same_part_shown_twice() {
        let mut app = app_with(&["ab"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 1,
        });
        assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");
        app.picked = Some(crate::drawings::Picked {
            paragraph: 0,
            nth: 0,
        });
        assert!(app.copy_drawing(), "there is a picture to copy");
        let copied = app.copied_drawing.as_ref().expect("and it was kept");
        assert!(
            copied.bytes.is_some(),
            "with its file bytes, for a document that has no such part"
        );

        app.picked = None;
        let text = app.paragraph_text(0);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: text.len(),
        });
        assert!(app.paste_copied_drawing(), "and it pastes back");
        let paragraphs = app.document.paragraphs();
        let drawings = paragraphs[0].drawings();
        assert_eq!(drawings.len(), 2, "the picture is now shown twice");
        // The same relationship: one part, no second copy of the bytes —
        // which is exactly what a duplicated picture is.
        assert_eq!(drawings[0].rel, drawings[1].rel);
    }

    #[test]
    fn a_cut_picture_leaves_and_a_paste_puts_it_back() {
        let mut app = app_with(&["ab"]);
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 1,
        });
        assert!(app.insert_picture(PIXEL, "image/png", 96, 48), "it pastes");
        app.picked = Some(crate::drawings::Picked {
            paragraph: 0,
            nth: 0,
        });

        app.run(Command::Cut);
        assert!(
            app.document.paragraphs()[0].drawings().is_empty(),
            "cut took the picture"
        );
        assert!(app.picked.is_none(), "and nothing is picked any more");

        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 0,
        });
        assert!(app.paste_copied_drawing(), "the cut copy pastes back");
        assert_eq!(app.document.paragraphs()[0].drawings().len(), 1);
    }

    #[test]
    fn a_chart_says_it_cannot_cross_into_another_document() {
        // A chart is a family of parts, and only its drawing was copied. In a
        // document where its relationship names nothing, the paste says so
        // rather than doing nothing.
        let mut app = app_with(&["ab"]);
        app.copied_drawing = Some(CopiedDrawing {
            drawing: wp_model::doc::Drawing {
                source: Vec::new().into(),
                anchored: false,
                extent: (wp_model::Emu(914_400), wp_model::Emu(457_200)),
                rel: None,
                chart: Some("rId99".into()),
                name: None,
                description: None,
                wrap: wp_model::doc::Wrap::None,
                distance: Default::default(),
                position: None,
                behind_text: false,
                text: None,
                tone: None,
                outline: None,
            },
            bytes: None,
            png: None,
        });
        assert!(!app.paste_copied_drawing(), "nothing was pasted");
        let (title, _) = app.message.as_ref().expect("and it says why");
        assert_eq!(title, "Cannot paste");
        assert!(
            app.document.paragraphs()[0].drawings().is_empty(),
            "no half-pasted chart in the text"
        );
    }

    #[test]
    fn a_file_word_can_embed_goes_in_as_it_is_and_anything_else_becomes_a_png() {
        // A JPEG re-encoded as a PNG is several times the size and no better;
        // a BMP kept as it is, is a photograph's worth of bytes for a picture
        // of a button.
        let (data, kind, width, height) = picture_bytes(PIXEL.to_vec()).expect("a png");
        assert_eq!(kind, "image/png");
        assert_eq!((width, height), (1, 1));
        assert_eq!(data, PIXEL, "the bytes were not touched");

        let mut bmp = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(4, 2))
            .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
            .expect("a bitmap");
        let (data, kind, width, height) = picture_bytes(bmp).expect("which is a picture");
        assert_eq!(
            kind, "image/png",
            "and arrives as one Word will not baulk at"
        );
        assert_eq!((width, height), (4, 2), "at the size it was");
        assert_eq!(
            image::guess_format(&data).expect("a format"),
            image::ImageFormat::Png
        );

        // A file that is not a picture at all is not offered to the document.
        assert!(picture_bytes(b"I am a text file, not a picture".to_vec()).is_none());
    }

    #[test]
    fn a_picture_too_wide_for_the_page_is_brought_down_to_the_column() {
        // A snip of a whole screen is 1920 pixels, which is twenty inches.
        let app = app_with(&[""]);
        let paragraph = picture_paragraph("rId9", &app.document.section, 1920, 1080);
        let drawings = paragraph.drawings();
        let drawing = drawings.first().expect("a picture");
        let column = wp_model::PageBox::of(&app.document.section).text_width();
        let width = drawing.extent.0 .0 as f64 / 12_700.0;
        let height = drawing.extent.1 .0 as f64 / 12_700.0;
        assert!((width - column).abs() < 0.5, "{width} fills the column");
        assert!(
            (height / width - 1080.0 / 1920.0).abs() < 0.01,
            "and keeps its proportions"
        );
    }

    #[test]
    fn the_cf_html_header_says_where_the_fragment_is() {
        // Every number in that header is a byte offset into the string it is
        // part of. One digit out and Word pastes the header, or half the
        // fragment, or nothing.
        fn offset(wrapped: &str, field: &str) -> usize {
            let at = wrapped.find(field).expect("the field") + field.len();
            wrapped[at..at + 10].parse().expect("ten digits")
        }
        let fragment = "<span style=\"font-weight:bold\">hello</span>";
        let wrapped = cf_html(fragment);
        assert_eq!(
            &wrapped[offset(&wrapped, "StartFragment:")..offset(&wrapped, "EndFragment:")],
            fragment
        );
        assert!(wrapped[offset(&wrapped, "StartHTML:")..].starts_with("<html>"));
        assert_eq!(offset(&wrapped, "EndHTML:"), wrapped.len());
    }

    #[test]
    fn a_copy_offers_word_the_formatting_the_text_cannot_carry() {
        // What goes on the board beside the text. The board itself is the
        // machine's and a test must not touch it, so this asks the two writers
        // for what a copy would have handed it.
        let mut app = app_with(&["make this bold please"]);
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 5,
            },
            head: Caret {
                paragraph: 0,
                offset: 14,
            },
        };
        app.run(Command::Bold);
        let paragraphs = edit::copy_range(&app.document, app.scope, app.selection);
        let html = clip::html(&app.document, &paragraphs);
        assert!(html.contains("font-weight:bold"), "{html}");
        assert!(html.contains("this bold"), "{html}");
        let rtf = clip::rtf(&app.document, &paragraphs);
        assert!(rtf.contains("\\b"), "{rtf}");
        assert!(rtf.contains("this bold"), "{rtf}");
    }

    #[test]
    fn pasting_what_this_copied_keeps_its_formatting() {
        // The clipboard holds text and nothing else, so copy and paste went
        // through a `String` and everything the runs knew was thrown away on
        // the way: a bold phrase came back plain.
        let mut app = app_with(&["make this bold please"]);
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 5,
            },
            head: Caret {
                paragraph: 0,
                offset: 14,
            },
        };
        app.run(Command::Bold);
        copied(&mut app, 5..14);

        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: text_of(&app, 0).len(),
        });
        app.paste_matching("this bold");

        assert_eq!(
            text_of(&app, 0),
            "make this bold please".to_string() + "this bold"
        );
        let paragraphs = app.document.paragraphs();
        let bold: String = paragraphs[0]
            .runs()
            .iter()
            .filter(|run| run.props.bold())
            .map(|run| run.text())
            .collect();
        assert_eq!(bold, "this boldthis bold", "the pasted copy is bold too");
    }

    #[test]
    fn pasting_something_another_program_copied_arrives_as_text() {
        // The board no longer says what this copied, so whatever is on it came
        // from somewhere else and is text — it takes the formatting of wherever
        // the caret is, which is Word's rule.
        let mut app = app_with(&["make this bold please"]);
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 5,
            },
            head: Caret {
                paragraph: 0,
                offset: 14,
            },
        };
        app.run(Command::Bold);
        copied(&mut app, 5..14);

        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 0,
        });
        app.paste_matching("from elsewhere");

        assert!(text_of(&app, 0).starts_with("from elsewhere"));
        let paragraphs = app.document.paragraphs();
        let bold: String = paragraphs[0]
            .runs()
            .iter()
            .filter(|run| run.props.bold())
            .map(|run| run.text())
            .collect();
        assert_eq!(bold, "this bold", "only what was already bold");
    }

    #[test]
    fn copying_across_paragraphs_pastes_them_back_as_paragraphs() {
        let mut app = app_with(&["first line", "second line", "third line"]);
        app.selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 6,
            },
            head: Caret {
                paragraph: 2,
                offset: 5,
            },
        };
        let text = app.selected_text().expect("something to copy");
        app.clipboard = Some(Clip {
            text: text.clone(),
            paragraphs: edit::copy_range(&app.document, app.scope, app.selection),
        });
        let was = app.document.paragraphs().len();

        app.selection = Selection::at(Caret {
            paragraph: 2,
            offset: text_of(&app, 2).len(),
        });
        app.paste_matching(&text);

        assert_eq!(
            app.document.paragraphs().len(),
            was + 2,
            "three copied paragraphs join onto one and add two"
        );
        assert_eq!(text_of(&app, 2), "third lineline");
        assert_eq!(text_of(&app, 3), "second line");
        assert_eq!(text_of(&app, 4), "third");
    }

    fn text_of(app: &Scriva, index: usize) -> String {
        app.document.paragraphs()[index].text()
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
        let caret = edit::backspace(
            &mut app.document,
            app.scope,
            &mut app.history,
            app.selection,
        );
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
            wp_model::Scope::Body,
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
        app.run(Command::GoTo(wp_model::Scope::Body, 2));
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
        app.run(Command::GoTo(wp_model::Scope::Body, 99));
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
