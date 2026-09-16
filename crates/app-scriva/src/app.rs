//! The application: the window, the commands, and the keys.
//!
//! **Every command goes through one enum and one dispatcher.** The menu, the
//! toolbar and the keyboard cannot answer the same command differently if there
//! is only one answer — Calx's finding 34, and the reason its menu bar and
//! toolbar stayed in step.

use std::path::{Path, PathBuf};

use ui_kit::{dialog, egui, menu, AppId, DocumentApp, Recent, SCRIVA};
use wp_model::doc::{Block, Document, Paragraph};
use wp_model::prop::{Justify, LineSpacing, Toggle};
use wp_model::units::{HalfPoint, Line240, Twips};

use crate::clip;
use crate::commands::tooltip;
use crate::edit::{self, Caret, History, Selection};
use crate::find::{self, Finder};
use crate::shaper::Egui;
use crate::text;
use crate::view::{self, View};

mod bands;
mod context;
mod dialogs;
mod find_bar;
mod font_dialog;
mod goto;
mod help;
mod notices;
mod page_setup;
mod strips;
mod surface;
mod tables;
mod watermark;
mod word_count;

pub(crate) use font_dialog::FontDraft;
pub(crate) use notices::{thousands, Notice};
pub(crate) use page_setup::PageSetupDraft;
pub(crate) use strips::shading_rows;
pub(crate) use tables::TableAt;

/// A comment being written in the Review pane, before it is posted: the
/// words it is about, the comment it answers if it is a reply, and the text
/// so far.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Draft {
    pub scope: wp_model::Scope,
    pub range: Selection,
    pub reply_to: Option<u32>,
    pub text: String,
    /// Take the keyboard on the next frame — set when the draft opens.
    pub focus: bool,
}

/// Which part of the window has the keyboard.
///
/// F6 walks it round — document, Navigate pane, Review pane, find bar,
/// toolbar — skipping what is not open, and Escape from anywhere but the
/// document brings it back to the document without closing anything. Held
/// here rather than read from egui's focus because two of the stops, the
/// panes, keep no egui widget focused: their rows are walked by the arrows
/// the pane reads for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Keyboard {
    #[default]
    Document,
    Navigate,
    Review,
    Find,
    Toolbar,
}

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
    /// The board's text, taking the formatting of where it lands — what
    /// Ctrl+Shift+V does, and the right-click menu's Paste Unformatted.
    PasteUnformatted,
    /// The right-click menu of the page, opened from the keyboard at the
    /// caret: Shift+F10.
    ContextMenu,
    /// The right-click menu's two rows for the link it was opened on.
    OpenLink,
    CopyLinkAddress,
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
    /// The toolbar's grid picker: a table of this many rows and columns,
    /// without a dialog.
    InsertTableOf(usize, usize),
    /// The Size box for the selected picture or chart.
    PictureSize,
    /// The picked picture put back to the size its own pixels ask for.
    PictureOriginalSize,
    /// The picked picture anchored at the left, the centre or the right of
    /// the column — `Alignment::Left`, `Center` or `Right`.
    AlignPicture(wp_model::doc::Alignment),
    /// Takes the selected picture or chart out of the document.
    DeletePicture,
    /// One of the margin presets, whole. Header, footer and gutter distances
    /// ride along unchanged.
    Margins(wp_model::PageMargins),
    /// The dialog for margins none of the presets offer.
    CustomMargins,
    /// Layout ▸ Page Setup…: paper, orientation, margins and the bands'
    /// distance from the edge, in one box.
    PageSetup,
    /// Format ▸ Font… (Ctrl+D).
    FontDialog,
    /// Edit ▸ Go To… (Ctrl+G): the popover on the status bar's page count.
    GoToPage,
    /// The Word Count box, from the status bar's count.
    WordCount,
    /// Help ▸ Keyboard Shortcuts…, User Guide, About Scriva.
    KeyboardShortcuts,
    UserGuide,
    About,
    Orient(wp_model::Orientation),
    /// A paper size, stated portrait; the section's orientation re-applies.
    Paper(wp_model::units::Twips, wp_model::units::Twips),
    Grow,
    Shrink,
    Size(HalfPoint),
    /// Format ▸ Font — a face by name. Word's font box speaks for the Latin
    /// slots only; East Asian and complex runs keep their own faces.
    Font(String),
    /// Format ▸ Text Colour — a palette entry, or `Auto` for Automatic.
    Color(wp_model::Color),
    /// The dialog for a colour the palette does not offer.
    CustomColor,
    Highlight(wp_model::Highlight),
    /// Paragraph ▸ Paragraph…: spacing and indents, by number.
    ParagraphDialog,
    /// Table ▸ Insert ▸ Row Above / Row Below: a blank row shaped like the
    /// caret's, on the side named.
    InsertRow {
        below: bool,
    },
    /// Table ▸ Insert ▸ Column Left / Column Right: a blank column beside the
    /// caret's, as wide as the one to its right.
    InsertColumn {
        after: bool,
    },
    /// Table ▸ Delete ▸ Row, Column, Table: the caret's. Deleting the last
    /// row or the last column deletes the table, which is what Word does.
    DeleteRow,
    DeleteColumn,
    DeleteTable,
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
    /// View ▸ Comments: whether a comment's text is washed and its marker
    /// drawn.
    ShowComments,
    /// Put the caret at the start of a paragraph, and show it.
    ///
    /// The flow travels with the number: every flow counts its paragraphs from
    /// zero, so "paragraph 4" alone names four different places in a document
    /// with three headers.
    GoTo(wp_model::Scope, usize),
    /// Select whole paragraphs of the text, `from` through to before `to` —
    /// a heading and its content, from the Navigate pane.
    SelectParagraphs(usize, usize),
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
    PreviousChange,
    /// Settle one change, named by its mark — a card's own button.
    AcceptChange(wp_model::Mark),
    RejectChange(wp_model::Mark),
    /// Put the caret at a change or a comment, from its card.
    GoToChange(wp_model::Mark),
    GoToComment(u32),
    /// A comment on the selection, or on the word at the caret: a draft in
    /// the Review pane.
    AddComment,
    /// A reply to a comment, drafted under its card.
    ReplyComment(u32),
    /// Reply to the comment the caret stands in.
    ReplyHere,
    ResolveComment(u32, bool),
    /// Resolve, or reopen, the comment the caret stands in.
    ResolveHere,
    /// The draft in the pane, posted or thrown away.
    PostComment,
    DiscardComment,
    /// Delete the comment the caret stands in.
    DeleteComment,
    DeleteCommentOf(u32),
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
    /// OpenDocument text.
    Odt,
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
            Some("odt") | Some("ott") => Format::Odt,
            _ => Format::Docx,
        }
    }

    /// Whether saving in this format throws formatting away.
    ///
    /// Only the two text formats do. A `.odt` is a whole document format with a
    /// splicing writer behind it, so saving one keeps everything a `.docx` save
    /// keeps — and the warning this drives names Markdown and plain text by
    /// what they cannot hold, which is not a thing that could be said about
    /// OpenDocument without saying something false.
    pub fn is_lossy(self) -> bool {
        matches!(self, Format::Markdown | Format::Text)
    }

    /// Whether this format can be written at all.
    ///
    /// A `.doc` is a memory image with a fast-save log on the end: writing one
    /// back means rebuilding every byte offset in it, and one wrong offset makes
    /// a file Word opens as something else. So it is read and saved as `.docx`.
    ///
    /// An `.odt` is written, through a splicing writer of its own: the package
    /// it came out of is kept and `content.xml` is edited a paragraph at a
    /// time, so a save keeps every part and every element the reader does not
    /// model. See `wp_odf::write`.
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

/// What a file chooser was opened for, done once it answers.
///
/// A chooser's answer arrives in a later frame than the one that asked — on
/// Linux it is another program's window (see `ui_kit::chooser`) — so whatever
/// was to follow it has to be carried along rather than run in the next line.
enum Chosen {
    /// Save As, and then whatever the save was standing in the way of: the
    /// Close or Exit of a document that had never been saved.
    SaveAs(Option<Box<Command>>),
    Open,
    ExportPdf,
    Picture,
}

/// What a crossing into the other package format renamed: each drawing or
/// link by its place in document order, and the name it had before. Kept so
/// that a save that fails can put the names back.
type Renamed = Vec<(usize, Option<std::sync::Arc<str>>)>;

pub struct Scriva {
    pub(crate) document: Document,
    /// The package the document came out of. The writer edits it rather than
    /// building a new one, which is the whole of the preservation guarantee.
    package: Option<ooxml::Package>,
    /// The same thing for an OpenDocument file, whose package is a zip with
    /// rules of its own and cannot be an [`ooxml::Package`]. Exactly one of the
    /// two is ever set, and which one it is decides which writer a save goes
    /// through.
    container: Option<wp_odf::Container>,
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
    pub(crate) history: History,
    pub(crate) selection: Selection,
    /// Which of the document's flows the caret is in: the text, or one header
    /// or footer being edited in place.
    ///
    /// **A header is edited where it is drawn, not in a box.** A real one
    /// holds a table of revisions, a logo and three fields; a dialog that took
    /// its text and gave text back would hand that document a paragraph where
    /// its table used to be. So the editor moves into the band instead — the
    /// same caret, the same keys, the same undo — and this says where it is.
    pub(crate) scope: wp_model::Scope,
    /// Where the caret stood in the text before a band was opened, so closing
    /// one puts it back rather than at the top of the page. Word does the
    /// same, and a caret that jumps on the way out loses the user's place.
    left_behind: Option<Selection>,
    shaper: Option<Egui>,
    recent: Recent,
    pub(crate) message: Option<(String, String)>,
    pending: Option<Pending>,
    /// A file chooser that is open, and what it was opened for.
    asking: Option<ui_kit::chooser::Asking<Chosen>>,
    /// Where the pages are scrolled to, in screen points.
    scroll: f32,
    /// How far the desk is to move this frame, in points on the glass,
    /// set by Page Up and Page Down: the view moves by a page or a screen
    /// and the caret keeps its place on it, which a reveal — as little as
    /// shows the caret — would not do.
    scroll_by: Option<f32>,
    /// Set when a document is opened or a new one begun, and cleared when
    /// a zoom is chosen: while it holds, the desk shows a whole page at a
    /// time whatever its size, as Word first shows one. Not consumed on
    /// the first frame that knows a size, because the window's first frames
    /// are at its modest opening size and the maximized one comes a few
    /// frames later; a fit taken once, then, was a page a third of the
    /// screen.
    zoom_follows_desk: bool,
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
    /// The comment being written in the Review pane, before it is posted.
    pub(crate) draft: Option<Draft>,
    /// Where the keyboard is: see [`Keyboard`].
    pub(crate) keyboard: Keyboard,
    /// The Navigate pane's state for the session: the row the keyboard is
    /// on, the headings folded shut, the filter typed, whether the bookmarks
    /// are open, and the row last scrolled to.
    pub(crate) nav_row: usize,
    pub(crate) nav_collapsed: std::collections::BTreeSet<usize>,
    pub(crate) nav_filter: String,
    pub(crate) nav_bookmarks_open: bool,
    pub(crate) nav_scrolled: Option<usize>,
    /// The Review pane's row the keyboard is on.
    pub(crate) review_row: usize,
    /// Whether a field in a pane held the keyboard this frame.
    pub(crate) pane_held: bool,
    /// Which cards the Review pane shows.
    pub(crate) review_filter: crate::panes::review::Filter,
    /// The card the pane last scrolled into view for the caret, so it is
    /// scrolled to once per arrival and not on every frame.
    pub(crate) review_scrolled: Option<crate::panes::review::CardKey>,
    /// The custom-margins dialog, in inches: top, bottom, left, right, and
    /// then how far the header sits from the top of the paper and the footer
    /// from the bottom. Word's own Page Setup keeps those last two on the same
    /// sheet, under "From edge", and they belong with the margins because
    /// they are measured against the same four edges.
    pub(crate) page_setup: Option<PageSetupDraft>,
    /// Format ▸ Font…: as opened, and as it stands.
    pub(crate) font_draft: Option<(FontDraft, FontDraft)>,
    /// The Go To popover's field while it is open.
    pub(crate) goto: Option<String>,
    word_count_up: bool,
    shortcuts_up: bool,
    about_up: bool,
    /// The last six colours chosen in the colour box, newest first.
    recent_colours: Vec<[u8; 3]>,
    /// The status notice: a sentence and the clock when it was first shown.
    pub(crate) notice: Option<(String, Option<f64>)>,
    /// The notice bar's facts about this document.
    pub(crate) notices: Vec<Notice>,
    /// When the page badge on the desk stops showing: set by a scroll of the
    /// desk, and not by the caret moving.
    pub(crate) badge_until: f64,
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
    /// The faces this document names that the machine draws in something
    /// else, for the status bar to say so and the listing to say what.
    substitutions: Vec<ui_kit::fonts::Substitution>,
    /// Whether the listing of substituted faces is open.
    fonts_listing: bool,
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
    /// Shift+F10 was pressed: the surface opens its menu at the caret on
    /// its next draw, where the response the menu hangs from exists.
    context_requested: bool,
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
    matches_for: (u64, String, find::Options),
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
    /// The frame clock's reading at the last key or click, which is when the
    /// caret's blink last started over.
    blink_from: f64,
    /// The colour the toolbar's colour button applies, and the highlight its
    /// marker applies: the last one chosen, as Word's split buttons keep.
    pub(crate) last_colour: wp_model::Color,
    pub(crate) last_highlight: wp_model::Highlight,
    /// The size box's text while it is being typed into.
    pub(crate) size_text: Option<String>,
    /// Whether a field on the toolbar held the keyboard this frame, so that
    /// what is typed there is not also typed into the document.
    pub(crate) field_held: bool,
    /// The same for the find bar, which the toolbar draws last.
    find_held: bool,
    /// Formatting chosen with nothing selected and the caret at the edge of
    /// a word — or in a paragraph with no word — for the typing that
    /// follows, as Word keeps it: bold pressed after the last letter of a
    /// word, a colour picked there. Held with the caret it was chosen at,
    /// and only good while the caret is still there; the first text typed
    /// takes it and carries it on. A caret *between* the letters of a word
    /// formats the word instead, which is also Word's rule.
    next_props: Option<(Caret, wp_model::RunProps)>,
    /// Every comment's range, for the washes, and the document revision it
    /// was worked out for.
    comment_ranges: Vec<crate::revise::CommentRange>,
    washes_for: u64,
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
        // A test never reaches the desktop: no chooser on the developer's
        // screen, no document of the test's in their recent list.
        #[cfg(test)]
        ui_kit::headless::enter();
        Scriva {
            document: blank(),
            package: None,
            container: None,
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
            asking: None,
            scroll: 0.0,
            scroll_by: None,
            zoom_follows_desk: true,
            focused: true,
            sweeping: false,
            fields: wp_layout::FieldValues::new(),
            navigator: false,
            reviewer: false,
            author: crate::revise::Author::new("Scriva user"),
            draft: None,
            keyboard: Keyboard::Document,
            nav_row: 0,
            nav_collapsed: Default::default(),
            nav_filter: String::new(),
            nav_bookmarks_open: false,
            nav_scrolled: None,
            review_row: 0,
            pane_held: false,
            review_filter: Default::default(),
            review_scrolled: None,
            page_setup: None,
            font_draft: None,
            goto: None,
            word_count_up: false,
            shortcuts_up: false,
            about_up: false,
            recent_colours: Vec::new(),
            notice: None,
            notices: Vec::new(),
            badge_until: 0.0,
            table_draft: None,
            color_draft: None,
            column_draft: None,
            cell_margin_draft: None,
            pending_fonts: None,
            fonts_settling: false,
            substitutions: Vec::new(),
            fonts_listing: false,
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
            context_requested: false,
            finder: None,
            finder_focused: false,
            find_matches: Vec::new(),
            matches_for: (u64::MAX, String::new(), find::Options::default()),
            surface_id: None,
            viewport: egui::Vec2::ZERO,
            zoom_draft: None,
            zoom_fresh: false,
            blink_from: 0.0,
            last_colour: wp_model::Color::Rgb([0xC0, 0x00, 0x00]),
            last_highlight: wp_model::Highlight::Yellow,
            size_text: None,
            field_held: false,
            find_held: false,
            next_props: None,
            comment_ranges: Vec::new(),
            washes_for: u64::MAX,
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

    pub(crate) fn changed(&mut self) {
        self.dirty = true;
        self.stamp = self.stamp.wrapping_add(1);
        self.view.invalidate();
    }

    pub(crate) fn caret(&self) -> Caret {
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

    pub(crate) fn showing_comments(&self) -> bool {
        self.view.show_comments
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
        // Formatting chosen for the typing to come shows on the toolbar as
        // Word shows it: the button lit before a letter is typed.
        if let Some((at, props)) = &self.next_props {
            if *at == self.caret() && self.selection.is_empty() {
                return (
                    props.toggles.is_on(Toggle::Bold),
                    props.toggles.is_on(Toggle::Italic),
                    props.underline.is_some_and(|u| u.kind.draws()),
                );
            }
        }
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

    /// Whether the selection is struck through, for the toolbar.
    pub(crate) fn struck(&self) -> bool {
        let range = self.formatting_range();
        edit::all_runs(&self.document, self.scope, range, |props| {
            props.toggles.is_on(Toggle::Strike)
        })
    }

    /// Whether every covered paragraph is in a bulleted list, and whether
    /// every one is in a numbered one.
    pub(crate) fn list_state(&self) -> (bool, bool) {
        (self.in_list(true), self.in_list(false))
    }

    /// The line spacing of the caret's paragraph, where it is a multiple of
    /// the line — the only kind the menu offers.
    pub(crate) fn line_spacing_at(&self) -> Option<Line240> {
        let paragraph = self
            .document
            .paragraph_in(self.scope, self.caret().paragraph)?;
        match self
            .document
            .styles
            .resolve_paragraph(&paragraph.props, None)
            .para
            .spacing
            .line
        {
            Some(LineSpacing::Multiple(line)) => Some(line),
            Some(_) => None,
            None => Some(Line240::SINGLE),
        }
    }

    /// The paragraph style the selection is in — the caret's paragraph's,
    /// or the one every covered paragraph shares; nothing when they differ.
    pub(crate) fn style_at(&self) -> Option<wp_model::StyleId> {
        let (start, end) = self.selection.ordered();
        let paragraphs = self.document.paragraphs_in(self.scope);
        let default = self
            .document
            .styles
            .default_style(wp_model::StyleKind::Paragraph);
        let mut found: Option<Option<wp_model::StyleId>> = None;
        for index in start.paragraph..=end.paragraph.min(paragraphs.len().saturating_sub(1)) {
            let style = paragraphs
                .get(index)
                .and_then(|p| p.props.style)
                .or(default);
            match found {
                Some(other) if other != style => return None,
                _ => found = Some(style),
            }
        }
        found.flatten()
    }

    /// The face the selection is set in, resolved through the styles and
    /// the theme the way the page resolves it — or nothing when the
    /// selection mixes faces, which is what an empty box says.
    pub(crate) fn face_at(&self) -> Option<String> {
        let mut face: Option<String> = None;
        for props in self.resolved_runs() {
            let family = wp_layout::resolve::family(
                &props,
                &self.document.theme,
                wp_model::prop::Script::Ascii,
                "Calibri",
            )
            .to_string();
            match &face {
                Some(other) if *other != family => return None,
                _ => face = Some(family),
            }
        }
        face
    }

    /// The size the selection is set in, in half-points, or nothing when
    /// it mixes sizes.
    pub(crate) fn size_at(&self) -> Option<HalfPoint> {
        let mut size: Option<HalfPoint> = None;
        for props in self.resolved_runs() {
            let this = props.font_size();
            match size {
                Some(other) if other != this => return None,
                _ => size = Some(this),
            }
        }
        size
    }

    /// Whether a picture or chart is picked, for the menus.
    pub(crate) fn has_picked(&self) -> bool {
        self.picked.is_some()
    }

    /// The face each quick style's paragraphs are set in, where this machine
    /// has it — for a menu that shows a style in its own face.
    pub(crate) fn style_faces(
        &self,
        ctx: &egui::Context,
    ) -> Vec<(wp_model::StyleId, String, Option<egui::FontFamily>)> {
        self.quick_styles()
            .into_iter()
            .map(|(id, name)| {
                let props = wp_model::ParaProps {
                    style: Some(id),
                    ..wp_model::ParaProps::default()
                };
                let run = self.document.styles.resolve_paragraph(&props, None).run;
                let family = wp_layout::resolve::family(
                    &run,
                    &self.document.theme,
                    wp_model::prop::Script::Ascii,
                    "Calibri",
                );
                let bold = run.toggles.is_on(wp_model::prop::Toggle::Bold);
                let italic = run.toggles.is_on(wp_model::prop::Toggle::Italic);
                let face = ui_kit::fonts::named_face(&family, bold, italic)
                    .filter(|face| ui_kit::fonts::bound(ctx, face));
                (id, name, face)
            })
            .collect()
    }

    /// The colour the selection is set in where every run agrees.
    pub(crate) fn colour_at(&self) -> Option<wp_model::Color> {
        let mut colour: Option<wp_model::Color> = None;
        for props in self.resolved_runs() {
            let this = props.color.unwrap_or(wp_model::Color::Auto);
            match colour {
                Some(other) if other != this => return None,
                _ => colour = Some(this),
            }
        }
        colour
    }

    /// The highlight the selection wears where every run agrees.
    pub(crate) fn highlight_at(&self) -> Option<wp_model::Highlight> {
        let mut highlight: Option<wp_model::Highlight> = None;
        for props in self.resolved_runs() {
            let this = props.highlight.unwrap_or(wp_model::Highlight::None);
            match highlight {
                Some(other) if other != this => return None,
                _ => highlight = Some(this),
            }
        }
        highlight
    }

    /// The run properties the selection covers, each resolved through its
    /// paragraph's style chain — one run, the caret's, when nothing is
    /// selected.
    fn resolved_runs(&self) -> Vec<wp_model::RunProps> {
        let styles = &self.document.styles;
        let (start, end) = self.selection.ordered();
        let paragraphs = self.document.paragraphs_in(self.scope);
        let mut out = Vec::new();
        if self.selection.is_empty() {
            if let Some(paragraph) = paragraphs.get(start.paragraph) {
                let layers = styles.resolve_paragraph(&paragraph.props, None);
                let direct = text::props_at(paragraph, start.offset);
                out.push(styles.resolve_run(&layers, &direct));
            }
            return out;
        }
        for index in start.paragraph..=end.paragraph.min(paragraphs.len().saturating_sub(1)) {
            let Some(paragraph) = paragraphs.get(index) else {
                continue;
            };
            let layers = styles.resolve_paragraph(&paragraph.props, None);
            for run in paragraph.runs() {
                out.push(styles.resolve_run(&layers, &run.props));
            }
        }
        out
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

    pub(crate) fn open_path(&mut self, path: &Path) {
        match Format::of(path) {
            Format::Docx => self.open_docx(path),
            Format::Doc => self.open_doc(path),
            Format::Odt => self.open_odt(path),
            other => self.open_text(path, other),
        }
        self.zoom_follows_desk = true;
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
        self.container = None;
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
        self.notices.clear();
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
    /// Opens an `.odt`.
    ///
    /// The same shape as a `.docx` and not the same as a `.doc`: the package
    /// the document came out of is kept, and a save edits it. What that buys is
    /// stated in `wp_odf::write` — the parts this crate does not model come
    /// back byte for byte, and so does every element inside the ones it does.
    fn open_odt(&mut self, path: &Path) {
        match wp_odf::open(path) {
            Ok((document, media, container)) => {
                self.document = document;
                self.package = None;
                self.container = Some(container);
                self.parts = None;
                self.adopt_document_fonts();
                self.pictures.clear();
                // ODF names a picture by the path it sits at in the package
                // rather than by a relationship, so the reader mints the names
                // and the bytes arrive beside the document.
                self.pictures
                    .adopt(media.into_iter().map(|picture| (picture.rel, picture.data)));
                self.path = Some(path.to_path_buf());
                self.dirty = false;
                self.history.clear();
                self.selection = Selection::default();
                self.scope = wp_model::Scope::Body;
                self.left_behind = None;
                self.band_page = None;
                self.picked = None;
                self.scroll = 0.0;
                self.notices.clear();
                self.stamp = self.stamp.wrapping_add(1);
                self.view.invalidate();
                self.recent.remember(SCRIVA, path);
                self.refresh_fields();
            }
            Err(error) => {
                self.message = Some((
                    "Cannot open".to_owned(),
                    format!(
                        "{}

{error}",
                        path.display()
                    ),
                ));
            }
        }
    }

    fn open_doc(&mut self, path: &Path) {
        match wp_doc::open(path) {
            Ok((document, media)) => {
                self.document = document;
                self.package = None;
                self.container = None;
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
                self.notices.clear();
                self.stamp = self.stamp.wrapping_add(1);
                self.view.invalidate();
                self.recent.remember(SCRIVA, path);
                self.refresh_fields();
                // A fact about this document, for the band under the toolbar:
                // a box would have to be dismissed before the copy could be
                // read.
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.post_notice(
                    format!(
                        "Opened as a copy of {name}. Saving writes {}.docx; the page frame's \
                         shapes are shown but not written.",
                        self.published_name()
                    ),
                    None,
                );
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
                self.container = None;
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
                self.notices.clear();
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

    /// Writes the document where it belongs, and says whether it did. A
    /// document that belongs nowhere yet asks where, and is not saved by the
    /// time this returns.
    fn save(&mut self) -> bool {
        let Some(path) = self.path.clone() else {
            self.save_as(None);
            return false;
        };
        match Format::of(&path) {
            Format::Docx => {}
            Format::Odt => return self.save_odt(&path),
            other => return self.save_text(&path, other),
        }
        // A new document, or one read out of a `.doc` or a `.odt`. Author the
        // package once; from here on it is edited by the same splice writer
        // that edits a document Word wrote. Whatever package the document
        // arrived in is not this one, and is let go with the format it
        // belonged to — **but only once this one is on disk**. Let go of it
        // first and a Save As into `.docx` that fails sends the document back
        // to the `.odt` it came from with nothing to write it through, and the
        // next Ctrl+S authors that file again from nothing.
        let mut authored = None;
        let (mut pictures, mut links) = (Vec::new(), Vec::new());
        if self.package.is_none() {
            match wp_docx::write::blank::package_for(&self.document) {
                Ok(mut package) => {
                    pictures = self.carry_loose_pictures(&mut package);
                    links = self.relate_addresses(&mut package);
                    authored = Some(package);
                }
                Err(error) => {
                    self.message = Some(("Cannot save".to_owned(), error.to_string()));
                    return false;
                }
            }
        }
        let package = match authored.as_mut() {
            Some(package) => package,
            None => match self.package.as_mut() {
                Some(package) => package,
                None => {
                    self.save_as(None);
                    return false;
                }
            },
        };
        let written = wp_docx::save(&mut self.document, package, &path);
        match written {
            Ok(()) => {
                if let Some(package) = authored {
                    // The drawings name the package's relationships now, not
                    // the loose pictures, and the painter resolves those
                    // through this index. Without it every picture is a box
                    // from the next layout on, as at the other places a part
                    // is added.
                    self.parts = wp_docx::DocumentParts::locate_in(&package).ok();
                    self.package = Some(package);
                    self.container = None;
                }
                self.dirty = false;
                self.recent.remember(SCRIVA, &path);
                true
            }
            Err(error) => {
                self.put_back(pictures, links);
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

    /// Says no, in the one place where an OpenDocument document can do less
    /// than a Word one.
    ///
    /// A picture is three things in a package — bytes, a name for them and a
    /// drawing that uses the name — and this authors none of the three for an
    /// ODF package yet. Refusing where the user asks is the honest answer;
    /// authoring a `.docx` package underneath a document that will be saved as
    /// `.odt` is how a picture would arrive on screen and be gone from the file.
    fn refuse_in_open_document(&mut self, why: &str) -> bool {
        if self.container.is_none() {
            return false;
        }
        self.message = Some((
            "Not yet".to_owned(),
            format!("{why}. Save it as a .docx first."),
        ));
        true
    }

    /// Writes an OpenDocument text document.
    ///
    /// Through the package it was opened from wherever there is one, which is
    /// what makes a save keep the parts and the elements this project does not
    /// model. A document that came from somewhere else — a new one, or a
    /// `.docx` being saved as `.odt` — has a package authored for it once, and
    /// from then on it is edited by the same splice writer.
    fn save_odt(&mut self, path: &Path) -> bool {
        // As in `save`: the package the document arrived in is let go only once
        // the container replacing it is on disk, so that a Save As into `.odt`
        // that fails leaves a `.docx` with the package it came in.
        let mut authored = None;
        let (mut pictures, mut links, mut carried) = (Vec::new(), Vec::new(), Vec::new());
        if self.container.is_none() {
            match wp_odf::write::blank::container_for(&self.document) {
                Ok(mut container) => {
                    (pictures, carried) = self.carry_pictures_into(&mut container);
                    links = self.state_addresses();
                    authored = Some(container);
                }
                Err(error) => {
                    self.message = Some(("Cannot save".to_owned(), error.to_string()));
                    return false;
                }
            }
        }
        let container = match authored.as_mut() {
            Some(container) => container,
            None => match self.container.as_mut() {
                Some(container) => container,
                None => return false,
            },
        };
        let written = wp_odf::save(&mut self.document, container, path);
        match written {
            Ok(()) => {
                if let Some(container) = authored {
                    self.container = Some(container);
                    self.package = None;
                    self.parts = None;
                    // The package the pictures were painted from is gone; the
                    // bytes carried into the container are what paints them now.
                    self.pictures.adopt(carried);
                }
                self.dirty = false;
                self.recent.remember(SCRIVA, path);
                true
            }
            Err(error) => {
                self.put_back(pictures, links);
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

    /// Puts the pictures a `.doc` or an `.odt` brought with it into the
    /// package being authored, and re-points the document's drawings at them.
    ///
    /// **A relationship that names no part is not a missing picture to Word,
    /// it is a damaged file.** So this happens before the document part is
    /// written, and a picture that cannot be embedded takes its drawing's
    /// relationship with it rather than leaving one dangling. Answers what it
    /// re-pointed, for [`Scriva::put_back`] to undo if the write then fails.
    fn carry_loose_pictures(&mut self, package: &mut ooxml::Package) -> Renamed {
        let mut embedded: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (rel, bytes) in self.pictures.loose() {
            let Some(content_type) = image_content_type(bytes) else {
                continue;
            };
            if let Ok(id) = wp_docx::media::embed(package, bytes, content_type) {
                embedded.insert(rel.clone(), id);
            }
        }
        let mut renamed = Vec::new();
        if self.pictures.loose().is_empty() {
            return renamed;
        }
        for (at, drawing) in self.document.drawings_mut().into_iter().enumerate() {
            let Some(rel) = drawing.rel.as_deref() else {
                continue;
            };
            // Only the names this document minted for itself are re-pointed;
            // anything else is a relationship the package already knows. What
            // was minted is what the loose store holds: each reader that hands
            // pictures out loose names them its own way — `doc-picture-N` for a
            // `.doc`, `odf-picture-N` for an `.odt` — and a prefix knew only one.
            if !self.pictures.loose().contains_key(rel) {
                continue;
            }
            let id = embedded.get(rel).map(|id| id.as_str().into());
            renamed.push((at, std::mem::replace(&mut drawing.rel, id)));
        }
        renamed
    }

    /// Puts the pictures of a document leaving its `.docx` into the
    /// OpenDocument package being authored for it, and re-points its drawings
    /// at them.
    ///
    /// ODF has no relationships: a frame names its picture by its path in the
    /// package, so each picture goes in under `Pictures/` and its drawing names
    /// that path, which is what the writer resolves it by. A drawing already in
    /// ODF's own spelling is left alone. Answers what it re-pointed, and the
    /// bytes it carried — taken up as loose pictures only once the save
    /// succeeds, because until then the package they came from still paints
    /// them.
    fn carry_pictures_into(
        &mut self,
        container: &mut wp_odf::Container,
    ) -> (Renamed, Vec<(String, Vec<u8>)>) {
        let wanted: Vec<(usize, String)> = self
            .document
            .drawings_mut()
            .iter()
            .enumerate()
            .filter(|(_, drawing)| drawing.source_in(wp_model::SourceFormat::Odf).is_none())
            .filter_map(|(at, drawing)| Some((at, drawing.rel.as_deref()?.to_owned())))
            .collect();
        let mut placed: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut carried = Vec::new();
        let mut targets = Vec::new();
        for (at, rel) in wanted {
            if let Some(href) = placed.get(&rel) {
                targets.push((at, href.clone()));
                continue;
            }
            let Some(bytes) = self
                .pictures
                .bytes(self.package.as_ref(), self.parts.as_ref(), &rel)
            else {
                continue;
            };
            let Some(media_type) = image_content_type(bytes) else {
                continue;
            };
            let Ok(href) = wp_odf::write::media::embed(container, bytes, media_type) else {
                continue;
            };
            carried.push((href.clone(), bytes.to_vec()));
            placed.insert(rel, href.clone());
            targets.push((at, href));
        }
        let mut renamed = Vec::new();
        let mut drawings = self.document.drawings_mut();
        for (at, href) in targets {
            if let Some(drawing) = drawings.get_mut(at) {
                renamed.push((at, drawing.rel.replace(href.as_str().into())));
            }
        }
        (renamed, carried)
    }

    /// Relates every link to an address outside the document, for a document
    /// going into a `.docx` it did not come out of.
    ///
    /// Such a document holds addresses where WordprocessingML wants the names of
    /// relationships: ODF states a link's address on the link. Written as they
    /// stand they are an `r:id` naming no relationship, and Word refuses the
    /// file rather than the link. Answers what it re-pointed.
    fn relate_addresses(&mut self, package: &mut ooxml::Package) -> Renamed {
        let mut renamed = Vec::new();
        for (at, link) in self.document.hyperlinks_mut().into_iter().enumerate() {
            if link.anchor.is_some() {
                continue;
            }
            let Some(address) = link.rel.clone() else {
                continue;
            };
            // A link that cannot be related loses its target rather than
            // keeping one Word would take for a damaged file.
            let id = wp_docx::link::relate(package, &address)
                .ok()
                .map(|id| id.as_str().into());
            renamed.push((at, std::mem::replace(&mut link.rel, id)));
        }
        renamed
    }

    /// The other way: a document leaving its `.docx` for an `.odt` has links
    /// that name relationships, and ODF has none, so each takes the address
    /// its relationship held. Kept as a name it would be a link to nowhere,
    /// and nothing would say so. Answers what it re-pointed.
    fn state_addresses(&mut self) -> Renamed {
        let mut renamed = Vec::new();
        let Some(parts) = &self.parts else {
            return renamed;
        };
        for (at, link) in self.document.hyperlinks_mut().into_iter().enumerate() {
            if link.anchor.is_some() {
                continue;
            }
            let Some(address) = link
                .rel
                .as_deref()
                .and_then(|rel| parts.external_target(rel))
            else {
                continue;
            };
            let address: std::sync::Arc<str> = address.into();
            renamed.push((at, link.rel.replace(address)));
        }
        renamed
    }

    /// Puts back what a crossing into the other format renamed, for a save that
    /// did not happen.
    ///
    /// The document goes back to the package it came in, and its pictures and
    /// links have to go back to the names that package knows them by. Left
    /// renamed, they name relationships of a package that was never written:
    /// every paragraph holding one reads back as changed on the next save and
    /// is rewritten, and what the reader did not model in it goes with it.
    fn put_back(&mut self, pictures: Renamed, links: Renamed) {
        let mut drawings = self.document.drawings_mut();
        for (at, rel) in pictures {
            if let Some(drawing) = drawings.get_mut(at) {
                drawing.rel = rel;
            }
        }
        let mut hyperlinks = self.document.hyperlinks_mut();
        for (at, rel) in links {
            if let Some(link) = hyperlinks.get_mut(at) {
                link.rel = rel;
            }
        }
    }

    /// Asks where to save, and saves there once the chooser answers — then
    /// runs `after`, if the save was standing in its way.
    fn save_as(&mut self, after: Option<Box<Command>>) {
        // Proposed under its own name and beside itself, as Word proposes it.
        // A `.doc` opened as a copy already has the `.docx` name it will be
        // saved under, and Save As is the key pressed to choose where that
        // copy goes, not to type its name again.
        let name = self
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Document1.docx".to_owned());
        let mut chooser = rfd::FileDialog::new()
            .set_title("Save As")
            .add_filter("Word document", &["docx"])
            .add_filter("OpenDocument text", &["odt"])
            .add_filter("Markdown", &["md"])
            .add_filter("Plain text", &["txt"])
            .set_file_name(name);
        if let Some(directory) = self.path.as_ref().and_then(|path| path.parent()) {
            chooser = chooser.set_directory(directory);
        } else if let Some(directory) = self.recent.directory() {
            chooser = chooser.set_directory(directory);
        }
        self.asking = Some(ui_kit::chooser::Asking::system(
            move || chooser.save_file(),
            Chosen::SaveAs(after),
        ));
    }

    /// What a chooser's answer was for, done now that it has one.
    fn chosen(&mut self, path: Option<PathBuf>, then: Chosen, ctx: &egui::Context) {
        let Some(path) = path else {
            return;
        };
        match then {
            Chosen::SaveAs(after) => {
                if self.save_to(path) {
                    if let Some(command) = after {
                        self.finish(*command, ctx);
                    }
                }
            }
            Chosen::Open => self.open_path(&path),
            Chosen::ExportPdf => self.export_pdf_to(path),
            Chosen::Picture => self.insert_picture_from(&path),
        }
    }

    /// The whole of Save As once the chooser has closed: the name decides the
    /// format, the format decides whether it can be written at all, and the
    /// document takes the new name whether or not it keeps the old one's
    /// format.
    ///
    /// Separate from [`Scriva::save_as`] because a file dialog is the one part
    /// of this path no test can open, and everything that has ever gone wrong
    /// in it is on this side of the dialog.
    pub(crate) fn save_to(&mut self, path: PathBuf) -> bool {
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
        // The new name is taken up only if it can actually be written. A Save
        // As that fails after renaming the document leaves it pointing at a
        // path that does not work, and the next Ctrl+S goes there rather than
        // to the file the user still has.
        let previous = self.path.replace(path);
        if self.save() {
            return true;
        }
        self.path = previous;
        false
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
    /// `Saved report.docx`, in the status bar.
    fn say_saved(&mut self) {
        let name = self
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".to_owned());
        self.say(format!("Saved {name}"));
    }

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
            .set_title("Export as PDF")
            .add_filter("PDF", &["pdf"])
            .set_file_name(format!("{stem}.pdf"));
        // Next to the document itself, which is where a resume's PDF belongs.
        if let Some(directory) = self.path.as_ref().and_then(|path| path.parent()) {
            chooser = chooser.set_directory(directory);
        } else if let Some(directory) = self.recent.directory() {
            chooser = chooser.set_directory(directory);
        }
        self.asking = Some(ui_kit::chooser::Asking::system(
            move || chooser.save_file(),
            Chosen::ExportPdf,
        ));
    }

    /// The PDF itself, once the chooser has said where it goes.
    fn export_pdf_to(&mut self, path: PathBuf) {
        let stem = self.published_name();
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
        self.post_notice(
            "Printing is not wired up on this platform. Export a PDF instead.",
            Some(("Export\u{2026}", Command::ExportPdf)),
        );
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
        self.zoom_follows_desk = true;
        self.document = blank();
        self.package = None;
        self.container = None;
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
        self.notices.clear();
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
                    .set_title("Open")
                    .add_filter(
                        "All documents",
                        &[
                            "docx", "docm", "dotx", "doc", "dot", "odt", "ott", "md", "txt",
                        ],
                    )
                    .add_filter("Word documents", &["docx", "docm", "dotx"])
                    .add_filter("Word 97-2003", &["doc", "dot"])
                    .add_filter("OpenDocument text", &["odt", "ott"])
                    .add_filter("Markdown", &["md", "markdown"])
                    .add_filter("Plain text", &["txt"]);
                if let Some(directory) = self.recent.directory() {
                    chooser = chooser.set_directory(directory);
                }
                self.asking = Some(ui_kit::chooser::Asking::system(
                    move || chooser.pick_file(),
                    Chosen::Open,
                ));
            }
            Command::Reopen(path) => self.open_path(&path),
            Command::ForgetRecent => self.recent.clear(SCRIVA),
            Command::Save => {
                if self.save() {
                    self.say_saved();
                }
            }
            Command::SaveAs => self.save_as(None),
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
                    self.say("Copied");
                } else if self.copy_selection() {
                    self.say("Copied");
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
            Command::PasteUnformatted => {
                self.picked = None;
                if let Some(text) = clipboard_get().filter(|text| !text.is_empty()) {
                    self.paste_text(&text);
                }
            }
            Command::ContextMenu => {
                self.menu_link = self.link_at(self.caret());
                self.context_requested = true;
            }
            Command::OpenLink => {
                if let Some(destination) = self.menu_link.clone() {
                    self.follow_link(destination);
                }
            }
            Command::CopyLinkAddress => {
                if let Some(destination) = &self.menu_link {
                    let address = match destination {
                        crate::links::Destination::Away(url) => url.clone(),
                        crate::links::Destination::Here(name) => format!("#{name}"),
                    };
                    clipboard_set(&address, &address, "");
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
            Command::Font(name) => {
                let name: &str = &name;
                self.format_runs(move |props| {
                    // A theme reference outranks the cached name beside it, so
                    // leaving one behind would silently undo this choice.
                    props.fonts.ascii = Some(name.into());
                    props.fonts.high_ansi = Some(name.into());
                    props.fonts.ascii_theme = None;
                    props.fonts.high_ansi_theme = None;
                })
            }
            Command::Color(color) => self.format_runs(move |props| props.color = Some(color)),
            Command::CustomColor => {
                // Opened on the colour the selection has, so the sliders and
                // the well start from it rather than from black.
                let hex = match self.colour_at() {
                    Some(wp_model::Color::Rgb([r, g, b])) => format!("{r:02X}{g:02X}{b:02X}"),
                    _ => String::new(),
                };
                self.color_draft = Some((ColorTarget::Text, hex));
            }
            Command::ParagraphDialog => self.open_paragraph_dialog(),
            Command::InsertRow { below } => self.insert_row(below),
            Command::InsertColumn { after } => self.insert_column(after),
            Command::DeleteRow => self.delete_row(),
            Command::DeleteColumn => self.delete_column(),
            Command::DeleteTable => self.delete_table(),
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
            Command::CustomMargins | Command::PageSetup => self.open_page_setup(),
            Command::FontDialog => self.open_font_dialog(),
            Command::GoToPage => {
                self.goto = Some((self.caret_page() + 1).to_string());
            }
            Command::WordCount => self.word_count_up = true,
            Command::KeyboardShortcuts => self.shortcuts_up = true,
            Command::UserGuide => self.open_user_guide(),
            Command::About => self.about_up = true,
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
            Command::Zoom(zoom) => {
                self.view.zoom = zoom;
                self.zoom_follows_desk = false;
            }
            Command::ShowMarks => {
                self.view.show_marks = !self.view.show_marks;
                self.view.invalidate();
            }
            Command::ShowRevisions => {
                self.view.show_revisions = !self.view.show_revisions;
                self.view.invalidate();
            }
            Command::ShowComments => self.view.show_comments = !self.view.show_comments,
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
            Command::AcceptChange(mark) => self.settle(&mark, crate::revise::Resolve::Accept),
            Command::RejectChange(mark) => self.settle(&mark, crate::revise::Resolve::Reject),
            Command::NextChange => self.step_change(true),
            Command::PreviousChange => self.step_change(false),
            Command::GoToChange(mark) => {
                if let Some(change) = crate::revise::tracked(&self.document)
                    .into_iter()
                    .find(|change| change.mark == mark)
                {
                    self.run(Command::GoTo(change.scope, change.paragraph));
                }
            }
            Command::GoToComment(id) => {
                if let Some(range) = self
                    .comment_ranges_now()
                    .iter()
                    .find(|range| range.id == id)
                    .cloned()
                {
                    self.go_to(range.scope, range.range.ordered().0);
                    self.selection = range.range;
                    self.reveal = Some(self.caret());
                }
            }
            Command::ReplyComment(id) => {
                if let Some(range) = self
                    .comment_ranges_now()
                    .iter()
                    .find(|range| range.id == id)
                    .cloned()
                {
                    self.reviewer = true;
                    self.draft = Some(Draft {
                        scope: range.scope,
                        range: range.range,
                        reply_to: Some(id),
                        text: String::new(),
                        focus: true,
                    });
                }
            }
            Command::ReplyHere => match self.comment_at_caret() {
                Some(id) => self.run(Command::ReplyComment(id)),
                None => self.no_comment_here(),
            },
            Command::ResolveComment(id, done) => {
                if crate::revise::resolve_comment(&mut self.document, &mut self.history, id, done) {
                    self.changed();
                }
            }
            Command::ResolveHere => match self.comment_at_caret() {
                Some(id) => {
                    let done = self
                        .document
                        .comment(id)
                        .is_some_and(|comment| comment.done);
                    self.run(Command::ResolveComment(id, !done));
                }
                None => self.no_comment_here(),
            },
            Command::PostComment => {
                let before = self.document.comments.len();
                self.post_draft();
                if self.document.comments.len() > before {
                    self.say("Comment added");
                }
            }
            Command::DiscardComment => self.draft = None,
            Command::DeleteCommentOf(id) => {
                if crate::revise::delete_comment(&mut self.document, &mut self.history, id) {
                    self.selection = Selection::at(clamp(&self.document, self.scope, self.caret()));
                    self.changed();
                }
            }
            Command::AddComment => {
                // **Word refuses this outside the main story**, in those
                // words: "Comments, endnotes and footnotes can only be added
                // to the main story." Asked over COM against a header's range
                // it declines rather than commenting somewhere else, and so
                // does this — a refusal the user can read beats a comment that
                // silently lands on whatever paragraph of the text wore the
                // caret's number.
                if self.editing_band() {
                    self.say("A header or footer cannot carry a comment");
                } else {
                    // Nothing selected: the word at the caret, as Word does.
                    // A refusal here asked the user to select something
                    // before saying what they had to say.
                    let range = if self.selection.is_empty() {
                        let caret = self.caret();
                        let content = self.paragraph_text(caret.paragraph);
                        let word = text::word_at(&content, caret.offset);
                        // The word without the space after it: the space
                        // belongs to the word for deleting, not for talking
                        // about.
                        let end = word.start + content[word.clone()].trim_end().len();
                        Selection {
                            anchor: Caret {
                                paragraph: caret.paragraph,
                                offset: word.start,
                            },
                            head: Caret {
                                paragraph: caret.paragraph,
                                offset: end,
                            },
                        }
                    } else {
                        self.selection
                    };
                    self.reviewer = true;
                    self.draft = Some(Draft {
                        scope: self.scope,
                        range,
                        reply_to: None,
                        text: String::new(),
                        focus: true,
                    });
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
                // Going somewhere puts the keyboard where the caret is.
                self.keyboard = Keyboard::Document;
            }
            Command::SelectParagraphs(from, to) => {
                self.close_band();
                let last = self.paragraph_count().saturating_sub(1);
                let end = to.saturating_sub(1).min(last);
                let length = self.paragraph_text(end).len();
                self.selection = Selection {
                    anchor: Caret {
                        paragraph: from.min(last),
                        offset: 0,
                    },
                    head: Caret {
                        paragraph: end,
                        offset: length,
                    },
                };
                self.keyboard = Keyboard::Document;
                self.reveal = Some(self.caret());
            }
            Command::UpdateToc => self.update_toc(),
            Command::InsertPicture => self.insert_picture_from_file(),
            Command::InsertTable => {
                self.table_draft = Some(["2".to_owned(), "2".to_owned()]);
            }
            Command::InsertTableOf(rows, columns) => self.insert_table(rows, columns),
            Command::PictureSize => self.open_size_dialog(),
            Command::PictureOriginalSize => self.picture_original_size(),
            Command::AlignPicture(alignment) => self.align_picture(alignment),
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
            self.say("No table of contents to update: Insert \u{203a} Update Table of Contents builds one from the headings");
            return;
        };
        let entries = wp_model::outline::table_of_contents(&self.document, span.levels.clone());
        if entries.is_empty() {
            self.say(
                "No headings: a table of contents is built from paragraphs in a heading style",
            );
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
            self.say("No tracked changes");
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
            self.say("No tracked changes");
            return;
        };
        let mark = found.mark.clone();
        if crate::revise::resolve_one(&mut self.document, &mut self.history, &mark, how) {
            self.selection = Selection::at(clamp(&self.document, self.scope, self.caret()));
            self.changed();
        }
    }

    /// The next change after the caret, or the one before it, wrapping round
    /// the document either way.
    fn step_change(&mut self, forward: bool) {
        let changes = crate::revise::tracked(&self.document);
        let here = (self.scope, self.caret().paragraph);
        let found = if forward {
            changes
                .iter()
                .find(|change| (change.scope, change.paragraph) > here)
                .or_else(|| changes.first())
        } else {
            changes
                .iter()
                .rev()
                .find(|change| (change.scope, change.paragraph) < here)
                .or_else(|| changes.last())
        };
        if let Some(change) = found {
            self.run(Command::GoTo(change.scope, change.paragraph));
        }
    }

    /// Settles one named change — a card's own Accept or Reject.
    fn settle(&mut self, mark: &wp_model::Mark, how: crate::revise::Resolve) {
        if crate::revise::resolve_one(&mut self.document, &mut self.history, mark, how) {
            self.selection = Selection::at(clamp(&self.document, self.scope, self.caret()));
            self.changed();
        }
    }

    /// Posts the comment being written in the pane, if there is one and it
    /// says anything.
    fn post_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let text = draft.text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let author = self.author.clone();
        match draft.reply_to {
            None => {
                crate::revise::add_comment(
                    &mut self.document,
                    &mut self.history,
                    draft.scope,
                    draft.range,
                    &author.name,
                    &author.initials,
                    &text,
                );
            }
            Some(parent) => {
                crate::revise::reply_to_comment(
                    &mut self.document,
                    &mut self.history,
                    parent,
                    &author.name,
                    &author.initials,
                    &text,
                );
            }
        }
        self.reviewer = true;
        self.changed();
    }

    /// Whether a box holds the window: a question, a chooser, a message, or
    /// one of the dialogs' drafts.
    fn box_up(&self) -> bool {
        self.pending.is_some()
            || self.asking.is_some()
            || self.message.is_some()
            || self.page_setup.is_some()
            || self.font_draft.is_some()
            || self.goto.is_some()
            || self.word_count_up
            || self.shortcuts_up
            || self.about_up
            || self.table_draft.is_some()
            || self.color_draft.is_some()
            || self.column_draft.is_some()
            || self.cell_margin_draft.is_some()
            || self.watermark_draft.is_some()
            || self.paragraph_draft.is_some()
            || self.size_draft.is_some()
            || self.zoom_draft.is_some()
    }

    /// The stops F6 walks, in order, with only the open ones in.
    fn keyboard_order(&self) -> Vec<Keyboard> {
        let mut order = vec![Keyboard::Document];
        if self.navigator {
            order.push(Keyboard::Navigate);
        }
        if self.reviewer {
            order.push(Keyboard::Review);
        }
        if self.finder.is_some() {
            order.push(Keyboard::Find);
        }
        order.push(Keyboard::Toolbar);
        order
    }

    /// F6 and Shift+F6 move the keyboard round the window; Escape in a pane
    /// or on the toolbar brings it back to the document and closes nothing.
    /// Read before anything is drawn, so this frame shows the result.
    fn cycle_keyboard(&mut self, ui: &egui::Ui) {
        if self.box_up() || egui::Popup::is_any_open(ui.ctx()) {
            return;
        }
        let (forward, back) = ui.input_mut(|i| {
            (
                ui_kit::keys::take(i, egui::Modifiers::NONE, egui::Key::F6),
                ui_kit::keys::take(i, egui::Modifiers::SHIFT, egui::Key::F6),
            )
        });
        if forward || back {
            let order = self.keyboard_order();
            let at = order
                .iter()
                .position(|stop| *stop == self.keyboard)
                .unwrap_or(0);
            let next = match forward {
                true => (at + 1) % order.len(),
                false => (at + order.len() - 1) % order.len(),
            };
            self.give_keyboard(order[next], ui.ctx());
            return;
        }
        if matches!(
            self.keyboard,
            Keyboard::Navigate | Keyboard::Review | Keyboard::Toolbar
        ) && ui.input_mut(|i| ui_kit::keys::take(i, egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.give_keyboard(Keyboard::Document, ui.ctx());
        }
    }

    /// Hands the keyboard to one stop: the document's surface, a pane's own
    /// rows, the find bar's field, or the toolbar's first control.
    pub(crate) fn give_keyboard(&mut self, to: Keyboard, ctx: &egui::Context) {
        self.keyboard = to;
        match to {
            Keyboard::Document => {
                if let Some(id) = self.surface_id {
                    ctx.memory_mut(|m| m.request_focus(id));
                }
            }
            // A pane keeps no widget focused: its rows are its own.
            Keyboard::Navigate | Keyboard::Review => ctx.memory_mut(|m| {
                if let Some(focused) = m.focused() {
                    m.surrender_focus(focused);
                }
            }),
            Keyboard::Find => {
                if let Some(finder) = &mut self.finder {
                    finder.focus = true;
                }
            }
            Keyboard::Toolbar => {
                // The first control that can be pressed: a disabled one —
                // Undo on a fresh document — cannot hold the focus.
                if let Some(first) = crate::toolbar::drawn(ctx)
                    .into_iter()
                    .find(|control| control.enabled)
                {
                    ctx.memory_mut(|m| m.request_focus(first.id));
                }
            }
        }
    }

    fn no_comment_here(&mut self) {
        self.say("No comment at the caret");
    }

    /// Every comment's range, worked out once per document revision.
    pub(crate) fn comment_ranges_now(&mut self) -> &[crate::revise::CommentRange] {
        if self.washes_for != self.stamp {
            self.washes_for = self.stamp;
            self.comment_ranges = crate::revise::comment_ranges(&self.document);
        }
        &self.comment_ranges
    }

    fn delete_comment_here(&mut self) {
        match self.comment_at_caret() {
            Some(id) => self.run(Command::DeleteCommentOf(id)),
            None => self.no_comment_here(),
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
            // The typing's formatting: what was chosen at the caret, if
            // anything was, so that a second press takes it off again.
            f(&self.typing_props())
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
            // After a word's last letter the change is for what is typed
            // next, not for the word — Word's rule, and the one a hand
            // meets most: a word typed in red and Automatic chosen at its
            // end stays red, and the next letters are black. It recoloured
            // the word, taking a caret at its end to be in it. Before a
            // word's first letter the word takes it, as Word does too.
            if !content.is_empty() && (word.is_empty() || caret.offset >= word.end) {
                let mut props = self.typing_props();
                change(&mut props);
                self.next_props = Some((caret, props));
                return;
            }
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

    /// The formatting the next typed text takes: what was chosen at the
    /// caret, if the caret is still where it was chosen, else the run's.
    pub(crate) fn typing_props(&self) -> wp_model::RunProps {
        let caret = self.caret();
        if let Some((at, props)) = &self.next_props {
            if *at == caret && self.selection.is_empty() {
                return props.clone();
            }
        }
        edit::paragraph_at(&self.document, self.scope, caret.paragraph)
            .map(|paragraph| text::props_at(&paragraph, caret.offset))
            .unwrap_or_default()
    }

    /// The chosen formatting, taken for the typing at hand: only while the
    /// caret is where it was chosen, and taken once — the text typed
    /// carries it on.
    fn take_next_props(&mut self) -> Option<wp_model::RunProps> {
        let caret = self.caret();
        match self.next_props.take() {
            Some((at, props)) if at == caret && self.selection.is_empty() => Some(props),
            _ => None,
        }
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

    /// Whether every covered paragraph is already in a list of this kind.
    fn in_list(&self, bullets: bool) -> bool {
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
        (start.paragraph..=last).all(|index| {
            paragraphs
                .get(index)
                .is_some_and(|p| p.props.numbering.is_some_and(kind))
        })
    }

    /// The bullet and numbering buttons: on when every covered paragraph is
    /// already in a list of that kind, and the press then takes them out.
    fn toggle_list(&mut self, bullets: bool) {
        let on = self.in_list(bullets);
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
            .set_title("Insert Picture")
            .add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "bmp"])
            .add_filter("All files", &["*"]);
        if let Some(directory) = self.recent.directory() {
            chooser = chooser.set_directory(directory);
        }
        self.asking = Some(ui_kit::chooser::Asking::system(
            move || chooser.pick_file(),
            Chosen::Picture,
        ));
    }

    /// The picture itself, once the chooser has said which.
    fn insert_picture_from(&mut self, path: &Path) {
        let read = std::fs::read(path)
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
        // An OpenDocument picture is one thing rather than three: the bytes go
        // into the package under `Pictures/` and the drawing names that path.
        // Taken up as a loose picture as well, which is what paints it.
        if let Some(container) = &mut self.container {
            let Ok(href) = wp_odf::write::media::embed(container, data, content_type) else {
                return false;
            };
            self.pictures.adopt([(href.clone(), data.to_vec())]);
            return self.place_picture(&href, width, height);
        }
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

        self.place_picture(&rel, width, height)
    }

    /// Puts the drawing for a picture already in the package at the caret, as a
    /// paragraph of its own.
    fn place_picture(&mut self, rel: &str, width: u32, height: u32) -> bool {
        let clip = vec![picture_paragraph(
            rel,
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
        // Word leaves a picture it has just put in picked, with its handles
        // showing, so that the next thing done is done to the picture — its
        // size, its place — rather than typed beside it.
        self.picked = self
            .document
            .paragraphs_in(self.scope)
            .get(caret.paragraph)
            .and_then(|paragraph| drawing_ending_at(paragraph, caret.offset))
            .map(|nth| crate::drawings::Picked {
                paragraph: caret.paragraph,
                nth,
            });
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
        if self.refuse_in_open_document("Charts cannot be added to an OpenDocument file yet") {
            return false;
        }
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
            source_format: wp_model::SourceFormat::Authored,
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
        let (query, options) = self
            .finder
            .as_ref()
            .map(|finder| (finder.query.clone(), finder.options))
            .unwrap_or_default();
        if self.matches_for == (self.stamp, query.clone(), options) {
            return;
        }
        self.find_matches = find::matches(&self.document, &query, options);
        self.matches_for = (self.stamp, query, options);
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
        let options = finder.options;
        let selection_matches = self
            .selected_text()
            .is_some_and(|text| find::equals(&text, &query, options));
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
    /// The command a keystroke asks for, from the one table the menus and
    /// the tooltips read.
    fn keys(&mut self, ui: &egui::Ui) -> Option<Command> {
        crate::commands::keys(ui)
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
                // With Shift held it is Ctrl+Shift+V, which pastes the text
                // alone: the platform makes the same event of both.
                egui::Event::Paste(text) => {
                    self.picked = None;
                    match ui.input(|i| i.modifiers.shift) {
                        true => self.paste_text(&text),
                        false => self.paste_matching(&text),
                    }
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

    pub(crate) fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
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
                // The view moves — by a whole page where one fits the desk,
                // else by a screen — and the caret keeps its place on it,
                // as Word's does. Where the leap runs off the document the
                // caret goes to its first line or its last, rather than
                // nowhere.
                let down = key == Key::PageDown;
                let leap = self.page_leap() * if down { 1.0 } else { -1.0 };
                let step = leap as f64 / (self.view.zoom * view::SCALE).max(0.01);
                let next =
                    view::step_from(&self.view, self.scope, caret, step).unwrap_or_else(|| {
                        match down {
                            false => Caret {
                                paragraph: 0,
                                offset: 0,
                            },
                            true => Caret {
                                paragraph: last,
                                offset: self.paragraph_text(last).len(),
                            },
                        }
                    });
                self.set_caret(next, extend);
                self.scroll_by = Some(leap);
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
                // Shift+Tab comes back up. In a table it goes from cell to
                // cell, and Ctrl+Tab is how a tab gets into one. Anywhere else
                // it is a tab.
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
                } else if modifiers.command || !self.tab_to_cell(modifiers.shift) {
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
                // Escape leaves things one at a time, the mode first: an open
                // header or footer, then the find bar, then the selection.
                if self.scope != wp_model::Scope::Body {
                    self.close_band();
                } else if self.finder.is_some() {
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
        // Not after a leap: the view has moved, the caret with it.
        if (self.selection, self.stamp) != was && self.scroll_by.is_none() {
            self.reveal = Some(self.caret());
        }
    }

    /// Types text, recording it as a tracked insertion when tracking is on.
    pub(crate) fn type_text(&mut self, input: &str) {
        let with = self.take_next_props();
        if !self.document.settings.track_changes {
            let caret = edit::type_text_with(
                &mut self.document,
                self.scope,
                &mut self.history,
                self.selection,
                input,
                with,
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
        match crate::revise::record_insertion_with(target, start.offset, input, &author, id, with) {
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
                self.say("Track Changes cannot record an edit inside a hyperlink, a content control or a field");
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
        // No line to step to means this is the first line of the flow, or
        // the last: Up goes to its start and Down to its end, as Word's
        // do — so that Shift+Up at the end of the first line selects it.
        // The step was refused and the caret stayed, and nothing was
        // selected.
        view::step_from(&self.view, self.scope, caret, step).unwrap_or_else(|| {
            let offset = match (view::line_span(&self.view, self.scope, caret), down) {
                (Some((start, _)), false) => start,
                (Some((_, end)), true) => end,
                (None, false) => 0,
                (None, true) => self.paragraph_text(caret.paragraph).len(),
            };
            Caret {
                paragraph: caret.paragraph,
                offset,
            }
        })
    }

    /// Lays the document out if it has changed since it last was — the
    /// view's own check, which also watches the field values. Nothing is
    /// laid out in the frame the fonts changed in: measuring now would
    /// measure the type the document replaced, and the page would be thrown
    /// away and done again a frame later regardless.
    fn lay_out(&mut self) {
        if self.fonts_settling {
            return;
        }
        let stamp = self.stamp;
        let fields = self.fields.clone();
        if let Some(shaper) = &mut self.shaper {
            self.view.refresh(&self.document, &fields, stamp, shaper);
        }
    }

    /// How far Page Up and Page Down move the desk, in points on the glass:
    /// a page and its gap where those fit the desk — so that at the
    /// whole-page zoom each press shows the next page whole, as Word's
    /// does — and a screen otherwise.
    fn page_leap(&self) -> f32 {
        let screen = self.viewport.y.max(60.0);
        let zoom = (self.view.zoom * view::SCALE) as f32;
        let pitch = self
            .view
            .pages()
            .first()
            .map(|page| (page.geometry.height + view::GAP as f64) as f32 * zoom);
        match pitch {
            Some(pitch) if pitch <= screen + 1.0 => pitch,
            _ => screen,
        }
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
    /// The alignment, or none stated.
    justify: Option<Justify>,
    /// Which of [`LINE_KINDS`] the line spacing is: Single, 1.5, Double,
    /// Exactly, At least, Multiple — or the style's, at the end.
    line: usize,
    /// The number beside Exactly and At least (points) and Multiple (lines).
    line_value: String,
}

/// The line-spacing kinds the paragraph box offers, in its order.
const LINE_KINDS: [&str; 7] = [
    "Single",
    "1.5 lines",
    "Double",
    "Exactly",
    "At least",
    "Multiple",
    "(style's)",
];

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
        source_format: wp_model::SourceFormat::Authored,
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
    // The layout here measures type the way Word 2013 and later do — with
    // the face's own advances — and a document that says nothing about it is
    // a Word 2007 document to Word, laid out with other metrics: the same
    // paragraph in Calibri and its twin Carlito broke its lines two points
    // apart in that mode and identically in this one. Saying so is what
    // makes the file come back from Word as it was drawn here.
    document.settings.compatibility_mode = 15;
    // What a new document is in Word today, measured on the reference machine
    // (Word 16.0.20326, 2026-09-13): twelve points, eight points after every
    // paragraph, and a line of 278 to 240 — the document defaults Word writes
    // into a new file's `styles.xml`. Stated here and written out, because a
    // file that states nothing is *not* single-spaced to Word: it lays such a
    // file with these same defaults, and a new document that drew single
    // spaced here came back from Word a third taller. Word's kerning and
    // ligature defaults are left out: the layout here measures type without
    // them, and a file that asks for them would be laid out differently there.
    document
        .styles
        .set_doc_defaults(wp_model::style::DocDefaults {
            para: wp_model::prop::ParaProps {
                spacing: wp_model::prop::Spacing {
                    after: Some(Twips(160)),
                    line: Some(LineSpacing::Multiple(Line240(278))),
                    ..Default::default()
                },
                ..Default::default()
            },
            run: wp_model::prop::RunProps {
                size: Some(HalfPoint(24)),
                ..Default::default()
            },
        });
    let mut normal = wp_model::Style::new("Normal", wp_model::StyleKind::Paragraph);
    normal.default = true;
    normal.name = Some("Normal".into());
    // Both Latin faces. Word draws a letter past U+007F in the `hAnsi` face,
    // and a style that names only `ascii` leaves that face to the document
    // defaults — which a new document does not state, and which Word then
    // takes to be Times New Roman: "café" would be two faces in Word and one
    // here.
    normal.run.fonts.ascii = Some("Calibri".into());
    normal.run.fonts.high_ansi = Some("Calibri".into());
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
        let bar = self.toolbar_row(ui);
        // The row under the toolbar — find bar, notice, or the strip for
        // where the caret is — which is always there, so that nothing it
        // says moves the page.
        let row = self.context_row(ui);
        if let Some(command) = command.or(bar).or(row) {
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
        use ui_kit::theme;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let pages = self.view.pages().len().max(1);
            let page = view::caret_rect(&self.view, self.scope, self.caret())
                .map(|(index, _)| index + 1)
                .unwrap_or(1);
            // The page count is the Go To popover's anchor, and the word
            // count opens the box with the rest of the figures.
            let page_label = ui
                .add(egui::Button::new(format!("Page {page} of {pages}")).frame(false))
                .on_hover_text("Go to a page  Ctrl+G");
            if page_label.clicked() && self.goto.is_none() {
                self.goto = Some(page.to_string());
            }
            self.goto_popover(ui, &page_label);
            ui.separator();
            let words = match self.selected_plain_text() {
                Some(text) => format!(
                    "{} of {} words",
                    thousands(crate::app::word_count::Count::of_text(&text).words),
                    thousands(self.word_count())
                ),
                None => format!("{} words", thousands(self.word_count())),
            };
            if ui
                .add(egui::Button::new(words).frame(false))
                .on_hover_text("Word count")
                .clicked()
            {
                self.word_count_up = true;
            }
            ui.separator();
            // The chips: what mode the document is in, each a small pill
            // with words on it, lit when its state is on.
            let tracking = self.document.settings.track_changes;
            if chip(
                ui,
                "Track changes",
                tracking,
                &tooltip(&Command::TrackChanges),
            )
            .clicked()
            {
                self.run(Command::TrackChanges);
            }
            if self.editing_band() {
                let label = match self.in_footer() {
                    true => "Editing footer \u{00b7} Esc",
                    false => "Editing header \u{00b7} Esc",
                };
                if chip(ui, label, true, "Back to the text  Esc").clicked() {
                    self.run(Command::CloseChrome);
                }
            }
            if let Some(table) = self.table_at_caret() {
                chip(
                    ui,
                    &format!("Table {} \u{00d7} {}", table.rows, table.columns),
                    false,
                    "The caret is in a table; its commands are on the strip and the Table menu",
                );
            }
            self.status_notice(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Word's corner, right to left: the percentage with its menu
                // of presets and fits, zoom in, the slider with its 100%
                // detent, zoom out. The buttons step to the next round ten.
                let shown = (self.view.zoom * 100.0).round();
                let mut percent = shown;
                ui.spacing_mut().item_spacing.x = 4.0;
                let label = ui
                    .add(
                        egui::Button::new(format!("{}% \u{2304}", percent as i32))
                            .frame(false)
                            .min_size(egui::vec2(48.0, 0.0)),
                    )
                    .on_hover_text("Zoom: presets, the two fits, or an exact number");
                let fits = (self.fit_percent(true), self.fit_percent(false));
                let mut zoom_to: Option<f64> = None;
                let mut custom = false;
                menu::under(&label, |ui| {
                    for preset in [50, 75, 100, 125, 150, 200] {
                        if menu::check(ui, &format!("{preset}%"), "", shown as i32 == preset)
                            .clicked()
                        {
                            zoom_to = Some(preset as f64);
                        }
                    }
                    menu::sep(ui);
                    if let Some(fit) = fits.0 {
                        if menu::item(ui, "Page &Width", "").clicked() {
                            zoom_to = Some(fit as f64);
                        }
                    }
                    if let Some(fit) = fits.1 {
                        if menu::item(ui, "W&hole Page", "").clicked() {
                            zoom_to = Some(fit as f64);
                        }
                    }
                    menu::sep(ui);
                    if menu::item(ui, "&Custom\u{2026}", "").clicked() {
                        custom = true;
                    }
                });
                if let Some(zoom) = zoom_to {
                    percent = zoom;
                }
                if custom {
                    self.zoom_draft = Some((percent as i32).to_string());
                    self.zoom_fresh = true;
                }
                if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                    percent = (percent / 10.0).floor() * 10.0 + 10.0;
                }
                zoom_slider(ui, &mut percent);
                if ui
                    .small_button("\u{2212}")
                    .on_hover_text("Zoom out")
                    .clicked()
                {
                    percent = (percent / 10.0).ceil() * 10.0 - 10.0;
                }
                let percent = percent.clamp(10.0, 500.0);
                if percent != shown {
                    self.view.zoom = percent / 100.0;
                    self.zoom_follows_desk = false;
                }
                // Word keeps quiet about a face it had to stand in for, and a
                // user whose every line breaks somewhere else is left to
                // wonder why. One phrase here, and the whole story one click
                // away.
                if !self.substitutions.is_empty() {
                    let label = match self.substitutions.len() {
                        1 => "1 font substituted".to_owned(),
                        n => format!("{n} fonts substituted"),
                    };
                    let hover: Vec<String> = self
                        .substitutions
                        .iter()
                        .map(|s| format!("{} \u{2192} {}", s.asked, s.shown))
                        .collect();
                    ui.separator();
                    if chip(ui, &label, false, &hover.join("\n")).clicked() {
                        self.fonts_listing = true;
                    }
                }
                let _ = theme::STATUS;
            });
        });
    }

    fn overlay(&mut self, ctx: &egui::Context) {
        if let Some(asking) = self.asking.take() {
            match asking.answered() {
                Ok((path, then)) => self.chosen(path, then, ctx),
                Err(asking) => {
                    if !ui_kit::chooser::waiting(ctx) {
                        self.asking = Some(asking);
                    }
                    return;
                }
            }
        }
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
        if self.page_setup.is_some() {
            self.page_setup_dialog(ctx);
            return;
        }
        if self.font_draft.is_some() {
            self.font_dialog(ctx);
            return;
        }
        if self.word_count_up {
            self.word_count_dialog(ctx);
            return;
        }
        if self.shortcuts_up {
            self.shortcuts_dialog(ctx);
            return;
        }
        if self.about_up {
            self.about_dialog(ctx);
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
        if self.fonts_listing {
            self.fonts_dialog(ctx);
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
                // A document that has never been saved asks where first, and
                // what it was closing for waits for that answer too.
                if self.path.is_none() {
                    self.save_as(Some(command));
                } else if self.save() {
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
        // A file dropped on the window opens it, through the same guard Open
        // goes through, which is how a document arrives when it is already
        // in front of you in a file manager; a picture dropped goes in at
        // the caret.
        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            let picture = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "bmp"
                    )
                });
            if picture {
                self.insert_picture_from(&path);
            } else if self.pending.is_none() && self.asking.is_none() {
                self.guarded(Command::Reopen(path));
            }
        }
        // A change of fonts lands between frames, so the atlas this frame draws
        // with is still the old one and every width the shaper has cached was
        // measured against it — and a family registered but not yet live is
        // not a family epaint substitutes for, it is one it panics on. The new
        // type is handed over here and *believed* one frame later, when the
        // shaper is thrown away and the page is laid out again in the face the
        // document actually asked for.
        if let Some((faces, named)) = self.pending_fonts.take() {
            self.substitutions = ui_kit::fonts::embed_document(ui.ctx(), &faces, &named);
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
        self.lay_out();
        self.pane_held = false;
        self.cycle_keyboard(ui);
        if self.navigator {
            if let Some(command) = self.navigate_pane(ui) {
                self.run(command);
            }
        } else if self.keyboard == Keyboard::Navigate {
            self.keyboard = Keyboard::Document;
        }
        if self.reviewer {
            if let Some(command) = self.review_pane(ui) {
                self.run(command);
            }
        } else {
            self.draft = None;
            if self.keyboard == Keyboard::Review {
                self.keyboard = Keyboard::Document;
            }
        }
        let bar_held = self.find_held || self.field_held;

        // Any key, letter or press starts the caret's blink over, solid.
        let touched = ui.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Text(_)
                        | egui::Event::PointerButton { pressed: true, .. }
                )
            })
        });
        if touched {
            self.blink_from = ui.input(|i| i.time);
        }

        // Ctrl+scroll and a trackpad pinch zoom the page, like Word.
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if zoom_delta != 1.0 {
            self.view.zoom = (self.view.zoom * zoom_delta as f64).clamp(0.10, 5.0);
            self.zoom_follows_desk = false;
        }

        // While a dialog or the find bar holds the keyboard, keys belong to it:
        // without this, searching for "bug" also types "bug" into the document.
        let blocked = self.pending.is_some()
            || self.asking.is_some()
            || self.message.is_some()
            || self.pane_held
            || self.keyboard != Keyboard::Document
            || self.page_setup.is_some()
            || self.font_draft.is_some()
            || self.goto.is_some()
            || self.word_count_up
            || self.shortcuts_up
            || self.about_up
            || self.table_draft.is_some()
            || self.color_draft.is_some()
            || self.column_draft.is_some()
            || self.cell_margin_draft.is_some()
            || self.watermark_draft.is_some()
            || self.paragraph_draft.is_some()
            || self.size_draft.is_some()
            || self.zoom_draft.is_some()
            || bar_held
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
        // What the keys and the typing changed is laid out before the desk
        // is painted, so that the frame a letter is typed in shows the
        // letter and the caret after it. The page was laid out once, above,
        // before the keys were read: the desk then painted the page as it
        // was, with a caret whose offset no line of that page reached, and
        // the caret fell back to the line's left edge for the frame — a
        // flash at the start of the line on every keystroke.
        self.lay_out();

        self.surface(ui);
        // Nothing holding the keyboard is the document holding it. Left to
        // nobody — after a dialog closes, or before the first click — the
        // keyboard is egui's to hand round: each Tab walked it one title along
        // the menu bar while the page took the same Tab as its own, a title
        // holding it takes Enter as a click, and the next paragraph mark typed
        // opened a menu, whose letters then ate the words that followed —
        // struck through, bolded, realigned. The surface holds on to Tab once
        // it has the keyboard, so having it is the whole of the fix.
        if !blocked && ui.memory(|m| m.focused().is_none()) {
            if let Some(id) = self.surface_id {
                ui.memory_mut(|m| m.request_focus(id));
            }
        }
        // A chooser asked this frame answers in a later one, and nothing but a
        // frame will pick the answer up.
        if self.asking.is_some() {
            ui.ctx().request_repaint();
        }
    }
}

impl Scriva {
    /// Replaces the page setup, undoably, and relays the document out.
    fn set_section(&mut self, section: wp_model::SectionProps) {
        let caret = self.caret();
        edit::set_section(&mut self.document, &mut self.history, caret, section);
        self.changed();
    }

    fn finish(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            other => self.run(other),
        }
    }
}

/// Which of a paragraph's drawings ends exactly at `offset` — the one a
/// paste just put in, with the caret after it — counted the way
/// [`Paragraph::drawings`] counts them, anchored ones included.
fn drawing_ending_at(paragraph: &Paragraph, offset: usize) -> Option<usize> {
    let mut at = 0;
    let mut nth = 0;
    for run in paragraph.runs() {
        for piece in &run.content {
            let len = piece.text_len();
            if let wp_model::doc::Piece::Drawing(drawing) = piece {
                if !drawing.anchored && at + len == offset {
                    return Some(nth);
                }
                nth += 1;
            }
            at += len;
        }
    }
    None
}

/// A status-bar chip: a small pill with words on it, in the soft ink, lit
/// with the on-tint when its state is on. Words, so that no state is
/// carried by colour alone.
fn chip(ui: &mut egui::Ui, label: &str, on: bool, tip: &str) -> egui::Response {
    use ui_kit::theme;
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(theme::TEXT_SMALL),
        theme::INK_SOFT,
    );
    let size = egui::vec2(galley.size().x + 16.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = match (on, response.hovered()) {
        (true, _) => theme::TINT_ON,
        (false, true) => theme::TINT_HOVER,
        _ => egui::Color32::TRANSPARENT,
    };
    ui.painter().rect(
        rect,
        11.0,
        fill,
        egui::Stroke::new(1.0, theme::CHROME_RULE),
        egui::StrokeKind::Inside,
    );
    let at = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(
        at,
        galley,
        match on {
            true => theme::INK,
            false => theme::INK_SOFT,
        },
    );
    response.on_hover_text(tip)
}

/// A hairline the full width of the bar.
fn rule(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 5.0), egui::Sense::hover());
    let y = rect.center().y.round() + 0.5;
    ui.painter().line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(1.0, ui_kit::theme::CHROME_RULE),
    );
}

#[cfg(test)]
mod tests;
