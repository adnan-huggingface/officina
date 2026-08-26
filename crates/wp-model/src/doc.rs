//! The document tree: body, blocks, paragraphs, runs, and the pieces inside a
//! run.
//!
//! Four levels, and each exists because the file has it:
//!
//! ```text
//! Document -> Block   (paragraph | table | content control | anchor)
//!          -> Inline  (run | hyperlink | revision wrapper | content control)
//!          -> Run     (properties + pieces)
//!          -> Piece   (text | tab | break | symbol | drawing | field | note ref)
//! ```
//!
//! Collapsing any of them loses something real. A run is not a string: it holds
//! tabs, breaks, footnote references and pictures in document order, and the
//! order is what the layout engine walks. An inline is not a run: `<w:ins>` and
//! `<w:hyperlink>` wrap runs, and flattening them either loses the tracked
//! change or makes every hyperlinked word its own paragraph.
//!
//! **Nothing here is resolved formatting.** A paragraph's `props` are what its
//! own `<w:pPr>` said and nothing more — see [`crate::style`].

use std::sync::Arc;

use crate::numbering::Numbering;
use crate::prop::{ParaProps, RunProps};
use crate::revision::{Anchor, Comment, Mark, People, PropChange, Revision};
use crate::section::{HeaderId, SectionProps};
use crate::style::StyleTable;
use crate::table::Table;
use crate::units::{Emu, Twips};
use crate::Theme;

/// One child of `<w:body>`, of a table cell, or of a header.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
    /// `<w:sdt>` — a content control wrapping whole blocks. The one in a Word
    /// cover page, and the one a data-bound form field lives in.
    Structured(Box<Sdt<Vec<Block>>>),
    /// A bookmark or comment mark that sits *between* blocks rather than inside
    /// a paragraph. Legal, and where Word puts the mark for a bookmark covering
    /// several paragraphs.
    Anchor(Anchor),
    /// `<w:altChunk>` — a whole document of some other format embedded by
    /// reference, to be merged in when Word next opens the file. Never
    /// interpreted; carried so it survives.
    AltChunk {
        rel: Arc<str>,
    },
}

impl Block {
    pub fn text(&self) -> String {
        match self {
            Block::Paragraph(p) => p.text(),
            Block::Table(t) => t.text(),
            Block::Structured(sdt) => text_of(&sdt.content),
            Block::Anchor(_) | Block::AltChunk { .. } => String::new(),
        }
    }
}

/// The text of a run of blocks, with a newline after each paragraph.
pub fn text_of(blocks: &[Block]) -> String {
    let mut out = String::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 && !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&block.text());
    }
    out
}

/// One `<w:p>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paragraph {
    pub props: ParaProps,
    pub content: Vec<Inline>,
    /// `w14:paraId` — **Word's own durable identity for this paragraph**, and
    /// the closest thing the format has to a primary key. It is what makes a
    /// splice writer possible for `document.xml`: a paragraph can be paired with
    /// the bytes it came from by identity rather than by position, so an edit
    /// three paragraphs earlier does not repaginate the whole file's rewrite.
    ///
    /// Absent in documents from producers other than Word, which is why nothing
    /// may *depend* on it — position is the fallback.
    pub id: Option<u32>,
    /// `w14:textId`. Changes when the text does; kept for round-tripping only.
    pub text_id: Option<u32>,
    /// The paragraph *mark* was inserted or deleted with track changes on —
    /// `<w:pPr><w:rPr><w:ins>`. This is what a tracked paragraph split or merge
    /// is: rejecting a deleted mark joins this paragraph to the next.
    pub mark_revision: Option<Revision>,
    /// `<w:pPrChange>` — the properties before a tracked formatting change.
    pub prop_change: Option<Box<PropChange>>,
    /// `<w:sectPr>` inside `<w:pPr>` — **this paragraph ends a section**, and
    /// these are that section's properties.
    ///
    /// Deliberately here rather than in [`ParaProps`]: a section break is a
    /// property of a paragraph in the body and is meaningless in a style, and
    /// putting it in the shared struct would let a style resolution carry one.
    pub section: Option<Box<SectionProps>>,
}

impl Paragraph {
    pub fn new() -> Paragraph {
        Paragraph::default()
    }

    /// A paragraph of plain text, for tests and for authoring.
    pub fn of(text: &str) -> Paragraph {
        let mut paragraph = Paragraph::new();
        if !text.is_empty() {
            paragraph.content.push(Inline::Run(Run::of(text)));
        }
        paragraph
    }

    /// The paragraph's text, deletions excluded.
    ///
    /// A tracked deletion is drawn but is not *in* the document, so a word
    /// count, a search and the flowed length all have to skip it. Including it
    /// is the mistake that makes Find match text the user has already deleted.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for inline in &self.content {
            inline.write_text(&mut out, true);
        }
        out
    }

    /// The text as it is *drawn*, deletions included, for a revision-showing
    /// view.
    pub fn shown_text(&self) -> String {
        let mut out = String::new();
        for inline in &self.content {
            inline.write_text(&mut out, false);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// The hyperlink covering `offset`, if a caret there stands inside one.
    ///
    /// Offsets are into [`Paragraph::text`], which is what a click on the page
    /// resolves to. The end of the link is not inside it, the same way a caret
    /// after the last letter of a word is not in the word: a link's last
    /// character is the last place following it can mean anything.
    pub fn link_at(&self, offset: usize) -> Option<&Hyperlink> {
        fn walk<'a>(content: &'a [Inline], at: &mut usize, offset: usize) -> Option<&'a Hyperlink> {
            for inline in content {
                match inline {
                    Inline::Hyperlink(link) => {
                        let start = *at;
                        let mut text = String::new();
                        inline.write_text(&mut text, true);
                        *at += text.len();
                        if (start..*at).contains(&offset) {
                            return Some(link);
                        }
                    }
                    Inline::Revised { revision, content } => {
                        // A deleted stretch is not in the text the offset is
                        // measured against, so it is not stepped over either.
                        if revision.is_present() {
                            if let Some(found) = walk(content, at, offset) {
                                return Some(found);
                            }
                        }
                    }
                    Inline::Structured(sdt) => {
                        if let Some(found) = walk(&sdt.content, at, offset) {
                            return Some(found);
                        }
                    }
                    Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                        if let Some(found) = walk(content, at, offset) {
                            return Some(found);
                        }
                    }
                    Inline::Run(_) | Inline::Math(_) | Inline::Anchor(_) => {
                        let mut text = String::new();
                        inline.write_text(&mut text, true);
                        *at += text.len();
                    }
                }
            }
            None
        }
        let mut at = 0;
        walk(&self.content, &mut at, offset)
    }

    /// Whether a bookmark of this name starts in this paragraph.
    ///
    /// An internal link names a bookmark and nothing finer, so where the mark
    /// sits within the paragraph does not matter — only which paragraph.
    pub fn starts_bookmark(&self, name: &str) -> bool {
        fn walk(content: &[Inline], name: &str) -> bool {
            content.iter().any(|inline| match inline {
                Inline::Anchor(Anchor::BookmarkStart { name: found, .. }) => &**found == name,
                Inline::Hyperlink(link) => walk(&link.content, name),
                Inline::Structured(sdt) => walk(&sdt.content, name),
                Inline::Revised { content, .. }
                | Inline::Wrapper { content, .. }
                | Inline::SimpleField { content, .. } => walk(content, name),
                Inline::Run(_) | Inline::Math(_) | Inline::Anchor(_) => false,
            })
        }
        walk(&self.content, name)
    }

    /// Every run in the paragraph, however deeply wrapped, in document order.
    ///
    /// The layout engine wants runs and does not care whether one arrived inside
    /// a hyperlink, a content control or an insertion — but it does care about
    /// the order, and about not missing the ones inside three wrappers.
    pub fn runs(&self) -> Vec<&Run> {
        let mut runs = Vec::new();
        collect_runs(&self.content, &mut runs);
        runs
    }

    /// Every drawing the paragraph holds, in document order.
    ///
    /// The position in this list is a drawing's identity for the editor: it is
    /// what a click on a picture resolves to, and what a move, a resize or a
    /// delete names. Order, not a generated id, because the file has no id for a
    /// drawing that survives Word's own round trips.
    pub fn drawings(&self) -> Vec<&Drawing> {
        self.runs()
            .iter()
            .flat_map(|run| &run.content)
            .filter_map(|piece| match piece {
                Piece::Drawing(drawing) => Some(drawing.as_ref()),
                _ => None,
            })
            .collect()
    }

    /// The same list, to change.
    pub fn drawing_mut(&mut self, nth: usize) -> Option<&mut Drawing> {
        let mut seen = 0;
        drawing_mut(&mut self.content, nth, &mut seen)
    }

    /// Takes a drawing out of the paragraph. `true` if there was one.
    pub fn remove_drawing(&mut self, nth: usize) -> bool {
        let mut seen = 0;
        remove_drawing(&mut self.content, nth, &mut seen)
    }

    /// Where a page ended the last time Word laid this document out.
    ///
    /// Word writes `<w:lastRenderedPageBreak>` into the runs when it saves, and
    /// it is the only opinion about pagination that a `.docx` contains. It is
    /// not authoritative — it is stale the moment anything changes — but it is
    /// the nearest thing to an oracle for testing a layout engine against, which
    /// is why it is modelled rather than dropped.
    pub fn rendered_page_breaks(&self) -> usize {
        self.runs()
            .iter()
            .flat_map(|run| &run.content)
            .filter(|piece| matches!(piece, Piece::LastRenderedPageBreak))
            .count()
    }
}

/// Every drawing of a tree of inlines, to change.
fn collect_drawings_mut<'a>(content: &'a mut [Inline], into: &mut Vec<&'a mut Drawing>) {
    for inline in content {
        let nested: &mut Vec<Inline> = match inline {
            Inline::Run(run) => {
                for piece in &mut run.content {
                    if let Piece::Drawing(drawing) = piece {
                        into.push(drawing);
                    }
                }
                continue;
            }
            Inline::Hyperlink(link) => &mut link.content,
            Inline::Revised { content, .. } => content,
            Inline::Structured(sdt) => &mut sdt.content,
            Inline::Wrapper { content, .. } => content,
            Inline::SimpleField { content, .. } => content,
            Inline::Math(_) | Inline::Anchor(_) => continue,
        };
        collect_drawings_mut(nested, into);
    }
}

/// The `nth` drawing of a tree of inlines, to change.
///
/// Written as a walk rather than as `runs_mut`, because handing out every run
/// mutably at once is a borrow the tree cannot give.
fn drawing_mut<'a>(
    content: &'a mut [Inline],
    nth: usize,
    seen: &mut usize,
) -> Option<&'a mut Drawing> {
    for inline in content {
        let nested: &mut Vec<Inline> = match inline {
            Inline::Run(run) => {
                for piece in &mut run.content {
                    if let Piece::Drawing(drawing) = piece {
                        if *seen == nth {
                            return Some(drawing);
                        }
                        *seen += 1;
                    }
                }
                continue;
            }
            Inline::Hyperlink(link) => &mut link.content,
            Inline::Revised { content, .. } => content,
            Inline::Structured(sdt) => &mut sdt.content,
            Inline::Wrapper { content, .. } => content,
            Inline::SimpleField { content, .. } => content,
            Inline::Anchor(_) | Inline::Math(_) => continue,
        };
        if let Some(found) = drawing_mut(nested, nth, seen) {
            return Some(found);
        }
    }
    None
}

fn remove_drawing(content: &mut [Inline], nth: usize, seen: &mut usize) -> bool {
    for inline in content {
        let nested: &mut Vec<Inline> = match inline {
            Inline::Run(run) => {
                let mut at = None;
                for (index, piece) in run.content.iter().enumerate() {
                    if matches!(piece, Piece::Drawing(_)) {
                        if *seen == nth {
                            at = Some(index);
                            break;
                        }
                        *seen += 1;
                    }
                }
                if let Some(index) = at {
                    run.content.remove(index);
                    return true;
                }
                continue;
            }
            Inline::Hyperlink(link) => &mut link.content,
            Inline::Revised { content, .. } => content,
            Inline::Structured(sdt) => &mut sdt.content,
            Inline::Wrapper { content, .. } => content,
            Inline::SimpleField { content, .. } => content,
            Inline::Anchor(_) | Inline::Math(_) => continue,
        };
        if remove_drawing(nested, nth, seen) {
            return true;
        }
    }
    false
}

fn collect_runs<'a>(content: &'a [Inline], into: &mut Vec<&'a Run>) {
    for inline in content {
        match inline {
            Inline::Run(run) => into.push(run),
            Inline::Hyperlink(link) => collect_runs(&link.content, into),
            Inline::Revised { content, .. } => collect_runs(content, into),
            Inline::Structured(sdt) => collect_runs(&sdt.content, into),
            Inline::Wrapper { content, .. } => collect_runs(content, into),
            Inline::SimpleField { content, .. } => collect_runs(content, into),
            Inline::Anchor(_) | Inline::Math(_) => {}
        }
    }
}

/// Something inside a paragraph.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Run(Run),
    Hyperlink(Box<Hyperlink>),
    /// `<w:ins>`, `<w:del>`, `<w:moveFrom>`, `<w:moveTo>` — a tracked change
    /// wrapping the content it is about.
    Revised {
        revision: Revision,
        content: Vec<Inline>,
    },
    /// An inline `<w:sdt>`.
    Structured(Box<Sdt<Vec<Inline>>>),
    /// `<w:fldSimple w:instr=" PAGE ">` — the compact spelling of a field, with
    /// its instruction on the element and its cached result inside.
    ///
    /// The same thing as the [`Piece::FieldStart`] triple and not
    /// interchangeable with it: rewriting one as the other would change every
    /// byte of a paragraph nobody edited.
    SimpleField {
        instruction: Arc<str>,
        content: Vec<Inline>,
    },
    Anchor(Anchor),
    /// `<w:smartTag>` and inline `<w:customXml>` — wrappers that carry meaning
    /// for something outside Word and none for layout. Transparent, and kept so
    /// the writer restores them around exactly the runs they held.
    Wrapper {
        name: Arc<str>,
        content: Vec<Inline>,
    },
    /// `<m:oMath>` — an equation. Not modelled and not laid out; the whole
    /// element is held as bytes so that a document containing mathematics
    /// survives being edited elsewhere. The text is kept alongside so that Find
    /// and a word count are not simply blind to it.
    Math(Box<MathBlob>),
}

impl Inline {
    fn write_text(&self, out: &mut String, skip_deleted: bool) {
        match self {
            Inline::Run(run) => run.write_text(out, skip_deleted),
            Inline::Hyperlink(link) => {
                for inline in &link.content {
                    inline.write_text(out, skip_deleted);
                }
            }
            Inline::Revised { revision, content } => {
                if skip_deleted && !revision.is_present() {
                    return;
                }
                for inline in content {
                    inline.write_text(out, skip_deleted);
                }
            }
            Inline::Structured(sdt) => {
                for inline in &sdt.content {
                    inline.write_text(out, skip_deleted);
                }
            }
            Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                for inline in content {
                    inline.write_text(out, skip_deleted);
                }
            }
            Inline::Math(math) => out.push_str(&math.text),
            Inline::Anchor(_) => {}
        }
    }
}

/// `<w:hyperlink>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Hyperlink {
    /// `r:id` — an external target, through the part's relationships.
    pub rel: Option<Arc<str>>,
    /// `w:anchor` — a bookmark in this document. Exactly one of the two is
    /// present, and which one decides whether following it leaves the document.
    pub anchor: Option<Arc<str>>,
    pub tooltip: Option<Arc<str>>,
    /// `w:history` — whether following it marks the link as visited.
    pub history: bool,
    pub content: Vec<Inline>,
}

/// An equation, held whole.
#[derive(Debug, Clone, PartialEq)]
pub struct MathBlob {
    /// The `<m:oMath>` element exactly as it was read, for the writer.
    pub source: Arc<[u8]>,
    /// Its `<m:t>` runs joined, so search and word count are not blind.
    pub text: Arc<str>,
}

/// One `<w:r>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Run {
    pub props: RunProps,
    pub content: Vec<Piece>,
    /// `<w:rPrChange>` — the formatting before a tracked change to it.
    pub prop_change: Option<Box<PropChange>>,
}

impl Run {
    pub fn new() -> Run {
        Run::default()
    }

    pub fn of(text: &str) -> Run {
        Run {
            content: vec![Piece::Text(text.into())],
            ..Run::default()
        }
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        self.write_text(&mut out, true);
        out
    }

    fn write_text(&self, out: &mut String, skip_deleted: bool) {
        for piece in &self.content {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Deleted(text) => {
                    if !skip_deleted {
                        out.push_str(text);
                    }
                }
                Piece::Tab => out.push('\t'),
                Piece::Break(Break::Line) => out.push('\n'),
                Piece::Drawing(drawing) if !drawing.anchored => out.push(OBJECT),
                Piece::Symbol { ch, .. } => out.push(*ch),
                Piece::Hyphen { breaking: false } => out.push('\u{2011}'),
                Piece::Hyphen { breaking: true } => out.push('\u{00AD}'),
                _ => {}
            }
        }
    }

    /// Whether this run holds anything that occupies space on a line.
    ///
    /// A run of nothing but a bookmark's worth of field characters is real and
    /// common, and shaping it wastes the shaper's time.
    pub fn is_blank(&self) -> bool {
        self.content.iter().all(|piece| {
            matches!(
                piece,
                Piece::FieldStart { .. }
                    | Piece::FieldSeparate
                    | Piece::FieldEnd
                    | Piece::Instruction(_)
                    | Piece::LastRenderedPageBreak
            )
        })
    }
}

/// One thing inside a run.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    /// `<w:t>`. Whether the element needs `xml:space="preserve"` is decided by
    /// the writer from the text itself rather than stored, because a reader that
    /// remembered the attribute and a writer that emitted it faithfully would
    /// still be wrong the moment the text was edited.
    Text(Arc<str>),
    /// `<w:delText>` — text inside a tracked deletion. Kept apart from ordinary
    /// text because it is drawn and is not *in* the document.
    Deleted(Arc<str>),
    /// `<w:instrText>` inside a deletion. Rare, and would otherwise be read as
    /// live field code.
    DeletedInstruction(Arc<str>),
    Tab,
    Break(Break),
    /// `<w:noBreakHyphen>` (`breaking: false`) and `<w:softHyphen>`.
    Hyphen {
        breaking: bool,
    },
    /// `<w:sym>` — a character named by font and code point, usually in the
    /// private use area. The font is part of the identity: `F0B7` is a bullet in
    /// Symbol and nothing at all anywhere else.
    Symbol {
        font: Arc<str>,
        ch: char,
    },
    Drawing(Box<Drawing>),
    /// `<w:footnoteRef/>` or `<w:endnoteRef/>` — the note's *own* number,
    /// standing at the head of the note rather than in the running text. The
    /// element is empty: which note it belongs to is where it sits.
    NoteMark {
        endnote: bool,
    },
    /// `<w:footnoteReference>`. The note itself is in `footnotes.xml`.
    FootnoteRef {
        id: i32,
        custom_mark: bool,
    },
    EndnoteRef {
        id: i32,
        custom_mark: bool,
    },
    CommentRef(u32),
    /// `<w:fldChar w:fldCharType="begin">` — the start of a legacy field.
    ///
    /// A field is three pieces at the same level rather than a container:
    /// begin, separate, end, with the instruction between the first two and the
    /// cached result between the last two. They may nest, and they may span
    /// paragraphs.
    FieldStart {
        /// `w:dirty` — the result is stale and Word will recompute it on open.
        dirty: bool,
        lock: bool,
    },
    FieldSeparate,
    FieldEnd,
    /// `<w:instrText>` — the field's code, in fragments that must be joined
    /// before they mean anything. ` PAGE ` may arrive as ` PA`, `GE `.
    Instruction(Arc<str>),
    /// `<w:lastRenderedPageBreak>` — Word's own record of where a page ended.
    /// See [`Paragraph::rendered_page_breaks`].
    LastRenderedPageBreak,
    /// `<w:ptab>` — an absolute-position tab, used by headers with a centred
    /// and a right-aligned part.
    PositionTab,
    /// `<w:object>` and `<w:pict>` — an embedded OLE object or a VML picture.
    /// Preserved whole; the relationship names the part that draws it.
    Embedded {
        rel: Option<Arc<str>>,
        /// The element exactly as it was read. See [`Drawing::source`].
        source: Arc<[u8]>,
    },
}

/// The character an inline picture occupies in a paragraph's text.
///
/// Word's own: a `.doc` stores exactly this byte where a picture or an OLE
/// object sits, and `Range.Text` still hands it back. A picture *is* a
/// character — one the caret steps over, a selection can take and Backspace
/// can delete — and a model where it is nothing instead is a model where
/// splitting a paragraph leaves the picture in both halves, because no offset
/// ever names it.
///
/// It is not text, so everything that shows the text to a human — plain-text
/// export, Markdown, a heading in a table of contents, the clipboard — takes
/// it back out again.
pub const OBJECT: char = '\u{1}';

impl Piece {
    /// How many bytes of the paragraph's text this piece contributes.
    ///
    /// The counterpart of [`Run::write_text`] with deletions skipped, and it has
    /// to stay exactly that: a caret's offset is a byte offset into
    /// [`Paragraph::text`], so anything that counts the bytes differently — the
    /// editor placing a caret, the layout telling a renderer where a fragment
    /// came from — puts the caret in the wrong place, and does it further wrong
    /// the more runs the paragraph has.
    pub fn text_len(&self) -> usize {
        match self {
            Piece::Text(text) => text.len(),
            // Drawn, and not *in* the document: an offset never lands inside a
            // tracked deletion.
            Piece::Deleted(_) => 0,
            Piece::Tab => 1,
            Piece::Break(Break::Line) => 1,
            // An inline picture is one character: see [`OBJECT`]. An anchored
            // one is not in the line at all — it is a thing on the page that a
            // paragraph happens to own — and Word gives it no character either.
            Piece::Drawing(drawing) if !drawing.anchored => OBJECT.len_utf8(),
            // The character, not one byte: `<w:sym>` is very often a private-use
            // code point that takes three.
            Piece::Symbol { ch, .. } => ch.len_utf8(),
            Piece::Hyphen { breaking: false } => '\u{2011}'.len_utf8(),
            Piece::Hyphen { breaking: true } => '\u{00AD}'.len_utf8(),
            _ => 0,
        }
    }
}

/// `<w:br>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Break {
    /// A line break within the paragraph. The default, and what Shift+Enter is.
    Line,
    Page,
    Column,
    /// `w:type="textWrapping"` with a `w:clear` — break past a floating object
    /// on the given side.
    TextWrapping(Clear),
}

impl Break {
    pub fn from_val(kind: Option<&str>, clear: Option<&str>) -> Break {
        match kind {
            Some("page") => Break::Page,
            Some("column") => Break::Column,
            Some("textWrapping") => Break::TextWrapping(match clear {
                Some("left") => Clear::Left,
                Some("right") => Clear::Right,
                Some("all") => Clear::All,
                _ => Clear::None,
            }),
            _ => Break::Line,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    All,
}

/// `<w:drawing>` — a picture, chart, shape or diagram.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    /// **The element exactly as it was read**, so a writer can put it back
    /// without understanding it.
    ///
    /// A `<w:drawing>` is a whole DrawingML document: effects, crops, rotations,
    /// text boxes, SmartArt, an `<mc:AlternateContent>` with a VML fallback for
    /// Word 2003. What is parsed out of it below is what the layout needs and
    /// nothing more, so re-emitting from those fields would destroy the rest —
    /// and *editing the paragraph that holds a picture* is an ordinary thing to
    /// do. This is the Preservation Vault applied inside a modelled part.
    ///
    /// Empty for a drawing we authored, which by construction has nothing in it
    /// that is not modelled.
    pub source: Arc<[u8]>,
    /// Inline drawings sit in the text like a very large character. Anchored
    /// ones are positioned on the page and the text flows round them, which is
    /// an entirely different layout problem.
    pub anchored: bool,
    pub extent: (Emu, Emu),
    /// `r:embed` on the image's blip — the part holding the bytes.
    pub rel: Option<Arc<str>>,
    /// `r:id` on a `<c:chart>` — the chart part this draws instead of a
    /// picture.
    ///
    /// A drawing is one or the other: `<a:graphicData>` names either a picture
    /// or a chart, and a document that puts a bar chart between two paragraphs
    /// carries it exactly as it carries a photograph, one `<w:drawing>` with a
    /// relationship inside. Without this the chart is laid out at its stated
    /// size and painted as nothing, which is a hole in the page.
    pub chart: Option<Arc<str>>,
    /// `<wp:docPr>` name and description: what a screen reader says, and what
    /// the selection pane lists.
    pub name: Option<Arc<str>>,
    pub description: Option<Arc<str>>,
    /// How text behaves around an anchored drawing.
    pub wrap: Wrap,
    /// Distance from the text on each side, for the wrap.
    pub distance: (Emu, Emu, Emu, Emu),
    /// Anchored drawings only: where it sits.
    pub position: Option<Box<DrawingPosition>>,
    /// `<wp:anchor w:behindDoc="1">`.
    pub behind_text: bool,
    /// The words *inside* the shape, when the shape is words rather than a
    /// picture of them. See [`ShapeText`].
    pub text: Option<Box<ShapeText>>,
    /// The shape drawn as itself — a rectangle with a line, a fill, or both.
    /// See [`ShapeOutline`].
    pub outline: Option<ShapeOutline>,
}

/// A shape drawn as itself rather than as a picture or as words.
///
/// **Only the rectangle.** It is the one autoshape documents use as furniture
/// — the heavy frame an engineering specification draws round every page is
/// one, and it is a shape in the header, not a page border — and a shape whose
/// geometry is not understood is better left undrawn than drawn as the wrong
/// outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeOutline {
    pub fill: Option<crate::Color>,
    /// The line round it. `None` for a shape that states it has none, which is
    /// not the same as a black one.
    pub line: Option<crate::Color>,
    pub line_width: crate::Emu,
}

/// The text a shape draws itself, with no bytes anywhere.
///
/// **A watermark is this and nothing else.** Word writes one as a piece of
/// WordArt in the header — a shape carrying a string, a face, a colour and an
/// angle — and neither DrawingML nor a `.doc`'s drawing layer ever turns it
/// into an image. A renderer that only knows how to put bytes in a rectangle
/// draws a watermarked page as a blank one, and the document quietly loses the
/// word that was the point of stamping it.
///
/// The glyphs are stretched to fill the shape rather than set at a point size:
/// that is what makes WordArt WordArt, and it is why there is no size here.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeText {
    pub text: Arc<str>,
    pub font: Option<Arc<str>>,
    pub color: Option<crate::Color>,
    pub bold: bool,
    pub italic: bool,
    /// Degrees clockwise from upright. Word's own diagonal watermark is 315.
    pub rotation: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wrap {
    /// `<wp:wrapNone>` — the text runs straight through it.
    #[default]
    None,
    /// `<wp:wrapSquare>` — round the bounding box.
    Square,
    /// `<wp:wrapTight>` / `<wp:wrapThrough>` — round the shape's own outline.
    Tight,
    /// `<wp:wrapTopAndBottom>` — no text beside it at all.
    TopAndBottom,
}

/// Where an anchored drawing sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawingPosition {
    pub horizontal: Offset,
    pub vertical: Offset,
}

/// One axis of a drawing's position: relative to something, by an amount or by
/// an alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset {
    pub relative_to: RelativeTo,
    /// `<wp:posOffset>`, in EMUs.
    pub offset: Option<Emu>,
    /// `<wp:align>` — `left`, `center`, `right`, `top`, `bottom`, `inside`,
    /// `outside`. Exclusive with the offset.
    pub align: Option<Alignment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RelativeTo {
    #[default]
    Column,
    Character,
    Margin,
    Page,
    InsideMargin,
    OutsideMargin,
    Paragraph,
    Line,
    TopMargin,
    BottomMargin,
    LeftMargin,
    RightMargin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Top,
    Bottom,
    /// Toward the binding on a mirrored spread — which side that is depends on
    /// the page number.
    Inside,
    Outside,
}

/// `<w:sdt>` — a content control, wrapping either blocks or inlines.
#[derive(Debug, Clone, PartialEq)]
pub struct Sdt<T> {
    pub id: Option<i32>,
    /// `<w:alias>` — the title shown on the control's tab.
    pub alias: Option<Arc<str>>,
    /// `<w:tag>` — a machine-readable name, and what a program filling the
    /// document in looks for.
    pub tag: Option<Arc<str>>,
    pub kind: SdtKind,
    /// `<w:dataBinding>` — the XPath tying this control to an element in a
    /// custom XML part. **This is the mechanism a document-assembly system runs
    /// on**, and losing it turns a template into a static document, which is why
    /// it is modelled from the start rather than merely retained.
    pub binding: Option<Box<DataBinding>>,
    /// `<w:lock>` — whether the control or its contents may be deleted.
    pub lock: Option<Arc<str>>,
    /// `<w:showingPlcHdr>` — the control is showing its placeholder rather than
    /// a value, so its text is not the user's.
    pub placeholder: bool,
    pub content: T,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SdtKind {
    #[default]
    RichText,
    PlainText,
    Picture,
    ComboBox,
    DropDownList,
    Date,
    Checkbox,
    DocPartObject,
    Group,
    Citation,
    Bibliography,
    RepeatingSection,
    /// Anything else, by element name, so the writer restores it.
    Other(Arc<str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataBinding {
    pub xpath: Arc<str>,
    pub prefix_mappings: Option<Arc<str>>,
    pub store_item_id: Option<Arc<str>>,
}

/// A footnote or endnote.
#[derive(Debug, Clone)]
pub struct Note {
    /// **Ids 0 and -1 are not notes.** They are the separator and the
    /// continuation separator — the little rules Word draws above a footnote
    /// area — and a reader that lists them as notes shows two empty footnotes at
    /// the top of every document.
    pub id: i32,
    pub kind: NoteKind,
    pub content: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoteKind {
    #[default]
    Normal,
    Separator,
    ContinuationSeparator,
    ContinuationNotice,
}

impl NoteKind {
    pub fn from_val(text: &str) -> Option<NoteKind> {
        Some(match text {
            "normal" => NoteKind::Normal,
            "separator" => NoteKind::Separator,
            "continuationSeparator" => NoteKind::ContinuationSeparator,
            "continuationNotice" => NoteKind::ContinuationNotice,
            _ => return None,
        })
    }

    /// Whether this is a note a reader should list.
    pub const fn is_real(self) -> bool {
        matches!(self, NoteKind::Normal)
    }
}

impl Note {
    pub fn text(&self) -> String {
        text_of(&self.content)
    }
}

/// One header or footer body.
#[derive(Debug, Clone)]
pub struct HeaderFooter {
    pub id: HeaderId,
    /// Where it came from in the package, which is its durable identity for the
    /// writer — the same reasoning as a worksheet's part name. `None` means one
    /// we authored and have not written yet.
    pub part: Option<Arc<str>>,
    /// The relationship id the section references it by.
    pub rel: Option<Arc<str>>,
    pub footer: bool,
    pub content: Vec<Block>,
}

/// Which of a document's flows a position or an edit is in.
///
/// **A document is not one body.** Its own is the long one, and every header
/// and footer is another — each a list of blocks, each edited the same way,
/// and each counting its paragraphs from zero. So a paragraph index alone
/// cannot say where it is: paragraph three means one thing in the body and
/// another in the header stamped on the page beside it, and an editor that
/// only ever names the number is an editor that can only ever reach one of
/// them.
///
/// Word draws the same line. The caret is in the body or it is in a header;
/// a selection never spans the two, and closing the header puts the caret
/// back where it was in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum Scope {
    #[default]
    Body,
    /// One header or footer body, by the identity the section references it by.
    Chrome(HeaderId),
}

/// `settings.xml`, as far as anything here needs it.
///
/// The part is large and most of it is compatibility flags from Word 6. What is
/// modelled is what changes what is drawn; the rest rides through the writer.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `<w:evenAndOddHeaders>` — a *document* setting that decides whether a
    /// section's even-page header is ever used.
    pub even_and_odd_headers: bool,
    /// `<w:mirrorMargins>` — left and right margins swap on facing pages.
    pub mirror_margins: bool,
    /// `<w:defaultTabStop>` — where tabs land when a paragraph defines none.
    pub default_tab_stop: Twips,
    /// `<w:trackChanges>` — whether edits are being tracked right now.
    pub track_changes: bool,
    /// `<w:documentProtection>` was present. Not enforced and not interpreted:
    /// the hash cannot be checked, so it is not a lock we may open.
    pub protected: bool,
    /// `<w:autoHyphenation>`.
    pub hyphenate: bool,
    /// `<w:consecutiveHyphenLimit>`, 0 for no limit.
    pub hyphen_limit: u32,
    /// `<w:zoom w:percent="…">`, so a document opens at the size it was closed.
    pub zoom: Option<u32>,
    /// `<w:rsids>` — Word's revision-save identifiers, meaningless to us and
    /// meaningful to Word's own merge. Counted rather than kept, since the
    /// writer restores the element whole.
    pub has_rsids: bool,
    /// `<w:noLeading/>` — "don't add leading between lines of text", one of the
    /// compatibility options a document converted from an older word processor
    /// carries and every Word 97 document written in this mode keeps.
    ///
    /// **It changes the height of every line in the document.** A face states
    /// how far its lines should stand apart as an ascent, a descent and a gap
    /// between one line's descender and the next line's ascender; with this
    /// set, Word drops the gap. Eight-point Arial is laid on a 9.20pt line
    /// ordinarily and on an 8.94pt one here — a quarter point a line, which is
    /// half an inch down a page of fifty and eventually a page of difference.
    pub no_leading: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            even_and_odd_headers: false,
            mirror_margins: false,
            // Word's own default: half an inch.
            default_tab_stop: Twips(720),
            track_changes: false,
            protected: false,
            hyphenate: false,
            hyphen_limit: 0,
            zoom: None,
            has_rsids: false,
            no_leading: false,
        }
    }
}

/// A whole Word document.
#[derive(Debug, Clone, Default)]
pub struct Document {
    pub body: Vec<Block>,
    /// The `<w:sectPr>` at the end of `<w:body>`, governing everything after the
    /// last paragraph that ends a section. Always present — a document with one
    /// page setup has exactly this and no others.
    pub section: SectionProps,
    pub styles: StyleTable,
    pub numbering: Numbering,
    pub theme: Theme,
    pub settings: Settings,
    pub headers: Vec<HeaderFooter>,
    pub footnotes: Vec<Note>,
    pub endnotes: Vec<Note>,
    pub comments: Vec<Comment>,
    pub people: People,
}

impl Document {
    pub fn new() -> Document {
        Document::default()
    }

    /// A document with one empty paragraph, which is the smallest thing Word
    /// will open.
    pub fn blank() -> Document {
        Document {
            body: vec![Block::Paragraph(Paragraph::new())],
            ..Document::default()
        }
    }

    pub fn text(&self) -> String {
        text_of(&self.body)
    }

    /// The blocks of one of the document's flows.
    ///
    /// A scope naming a header the document does not have answers with nothing
    /// rather than with the body: a caret left behind in a header that has
    /// since been taken away must find an empty flow, not quietly start
    /// editing the first page of the text.
    pub fn blocks(&self, scope: Scope) -> &[Block] {
        match scope {
            Scope::Body => &self.body,
            Scope::Chrome(id) => self
                .header(id)
                .map(|header| header.content.as_slice())
                .unwrap_or_default(),
        }
    }

    /// The same flow, to change. `None` when the scope names nothing.
    pub fn blocks_mut(&mut self, scope: Scope) -> Option<&mut Vec<Block>> {
        match scope {
            Scope::Body => Some(&mut self.body),
            Scope::Chrome(id) => self.header_mut(id).map(|header| &mut header.content),
        }
    }

    /// Every paragraph of one flow, in document order, the ones inside tables
    /// and content controls included.
    pub fn paragraphs_in(&self, scope: Scope) -> Vec<&Paragraph> {
        let mut out = Vec::new();
        walk_paragraphs(self.blocks(scope), &mut out);
        out
    }

    /// The same walk, to change.
    pub fn paragraphs_in_mut(&mut self, scope: Scope) -> Vec<&mut Paragraph> {
        let mut out = Vec::new();
        if let Some(blocks) = self.blocks_mut(scope) {
            walk_paragraphs_mut(blocks, &mut out);
        }
        out
    }

    /// One paragraph of one flow, by its place in that flow's own walk.
    pub fn paragraph_in(&self, scope: Scope, index: usize) -> Option<&Paragraph> {
        let mut at = 0;
        walk_to(self.blocks(scope), &mut at, index)
    }

    /// Every paragraph in the body, in document order, including the ones inside
    /// tables and content controls.
    ///
    /// Layout, numbering and search all want this walk, and each getting it
    /// slightly differently is how a list in a table ends up numbered
    /// separately from the same list outside one.
    pub fn paragraphs(&self) -> Vec<&Paragraph> {
        self.paragraphs_in(Scope::Body)
    }

    /// One paragraph by its place in that walk, without gathering the rest.
    ///
    /// [`Document::paragraphs`] allocates a vector of every paragraph in the
    /// document, which is the wrong shape for anything asked once a frame —
    /// what is under the pointer, what the caret is standing in.
    pub fn paragraph(&self, index: usize) -> Option<&Paragraph> {
        self.paragraph_in(Scope::Body, index)
    }

    /// Every paragraph, mutably, in the same document order as
    /// [`Document::paragraphs`].
    ///
    /// The order is what makes this usable: an editor and a writer both need to
    /// name a paragraph, and its position in this walk is the only name that
    /// works for a document whose paragraphs have no `w14:paraId`.
    pub fn paragraphs_mut(&mut self) -> Vec<&mut Paragraph> {
        self.paragraphs_in_mut(Scope::Body)
    }

    /// Every drawing the document holds, to change — the headers and footers
    /// included, unlike [`Document::paragraphs`], which is the body's own walk.
    ///
    /// For the one job that has to touch all of them at once: re-pointing what
    /// they name when a document is put into a package it did not come from.
    pub fn drawings_mut(&mut self) -> Vec<&mut Drawing> {
        let mut paragraphs = Vec::new();
        walk_paragraphs_mut(&mut self.body, &mut paragraphs);
        for header in &mut self.headers {
            walk_paragraphs_mut(&mut header.content, &mut paragraphs);
        }
        let mut out = Vec::new();
        for paragraph in paragraphs {
            collect_drawings_mut(&mut paragraph.content, &mut out);
        }
        out
    }

    /// Where the bookmark of this name starts, as an index into
    /// [`Document::paragraphs`].
    ///
    /// An internal hyperlink and a cross-reference both name their destination
    /// this way. `<w:bookmarkStart>` may sit between paragraphs as well as
    /// inside one, and a mark between them belongs to the paragraph after it —
    /// which is where Word puts the caret when the link is followed.
    pub fn bookmark(&self, name: &str) -> Option<usize> {
        fn walk(blocks: &[Block], name: &str, at: &mut usize) -> Option<usize> {
            for block in blocks {
                match block {
                    Block::Paragraph(paragraph) => {
                        if paragraph.starts_bookmark(name) {
                            return Some(*at);
                        }
                        *at += 1;
                    }
                    Block::Table(table) => {
                        for row in &table.rows {
                            for cell in &row.cells {
                                if let Some(found) = walk(&cell.content, name, at) {
                                    return Some(found);
                                }
                            }
                        }
                    }
                    Block::Structured(sdt) => {
                        if let Some(found) = walk(&sdt.content, name, at) {
                            return Some(found);
                        }
                    }
                    Block::Anchor(Anchor::BookmarkStart { name: found, .. })
                        if &**found == name =>
                    {
                        return Some(*at);
                    }
                    Block::Anchor(_) | Block::AltChunk { .. } => {}
                }
            }
            None
        }
        let mut at = 0;
        walk(&self.body, name, &mut at)
    }

    /// The sections, in order, each with the range of body blocks it governs.
    ///
    /// A `<w:sectPr>` *terminates* the section containing its paragraph, so the
    /// ranges run from just after the previous break to and including the
    /// paragraph carrying this one. The last range ends at the body's end and
    /// takes the document's own final properties.
    pub fn sections(&self) -> Vec<(std::ops::Range<usize>, &SectionProps)> {
        let mut out = Vec::new();
        let mut start = 0;
        for (index, block) in self.body.iter().enumerate() {
            if let Block::Paragraph(paragraph) = block {
                if let Some(section) = &paragraph.section {
                    out.push((start..index + 1, section.as_ref()));
                    start = index + 1;
                }
            }
        }
        out.push((start..self.body.len(), &self.section));
        out
    }

    /// The label of every numbered paragraph, in document order.
    ///
    /// Walks the whole body once, because a number is a function of everything
    /// before it. Rebuilt rather than patched after an edit — see
    /// [`crate::numbering::Counters`].
    pub fn list_labels(&self) -> Vec<Option<String>> {
        let mut counters = crate::numbering::Counters::new();
        self.paragraphs()
            .iter()
            .map(|paragraph| {
                let reference = self.resolved_numbering(paragraph)?;
                counters.advance(&self.numbering, reference)
            })
            .collect()
    }

    /// Which list a paragraph is in, taking its style's numbering into account.
    ///
    /// A paragraph very often carries no `<w:numPr>` of its own and is in a list
    /// because its *style* is — every "List Bullet" paragraph is exactly that —
    /// so asking the paragraph alone finds no list at all.
    fn resolved_numbering(&self, paragraph: &Paragraph) -> Option<crate::prop::NumRef> {
        if let Some(reference) = paragraph.props.numbering {
            return reference.is_numbered().then_some(reference);
        }
        let style = paragraph.props.style.or_else(|| {
            self.styles
                .default_style(crate::style::StyleKind::Paragraph)
        })?;
        self.styles
            .chain(style)
            .into_iter()
            .rev()
            .find_map(|step| self.styles.get(step)?.para.numbering)
            .filter(|reference| reference.is_numbered())
    }

    /// A real note, skipping the separators that share the list with them.
    pub fn footnote(&self, id: i32) -> Option<&Note> {
        self.footnotes
            .iter()
            .find(|note| note.id == id && note.kind.is_real())
    }

    pub fn comment(&self, id: u32) -> Option<&Comment> {
        self.comments.iter().find(|comment| comment.id == id)
    }

    pub fn header(&self, id: HeaderId) -> Option<&HeaderFooter> {
        self.headers.iter().find(|header| header.id == id)
    }

    pub fn header_mut(&mut self, id: HeaderId) -> Option<&mut HeaderFooter> {
        self.headers.iter_mut().find(|header| header.id == id)
    }

    /// Every author who has made a tracked change or left a comment.
    pub fn authors(&self) -> Vec<Arc<str>> {
        let mut people = People::default();
        for comment in &self.comments {
            people.record(&comment.author);
        }
        for paragraph in self.paragraphs() {
            if let Some(revision) = &paragraph.mark_revision {
                people.record(&revision.mark().author);
            }
            collect_authors(&paragraph.content, &mut people);
        }
        people.authors
    }
}

fn collect_authors(content: &[Inline], into: &mut People) {
    for inline in content {
        match inline {
            Inline::Revised { revision, content } => {
                into.record(&revision.mark().author);
                collect_authors(content, into);
            }
            Inline::Run(run) => {
                if let Some(change) = &run.prop_change {
                    into.record(&change.mark.author);
                }
            }
            Inline::Hyperlink(link) => collect_authors(&link.content, into),
            Inline::Structured(sdt) => collect_authors(&sdt.content, into),
            Inline::Wrapper { content, .. } => collect_authors(content, into),
            Inline::SimpleField { content, .. } => collect_authors(content, into),
            Inline::Anchor(_) | Inline::Math(_) => {}
        }
    }
}

fn walk_paragraphs_mut<'a>(blocks: &'a mut [Block], into: &mut Vec<&'a mut Paragraph>) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => into.push(paragraph),
            Block::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        walk_paragraphs_mut(&mut cell.content, into);
                    }
                }
            }
            Block::Structured(sdt) => walk_paragraphs_mut(&mut sdt.content, into),
            Block::Anchor(_) | Block::AltChunk { .. } => {}
        }
    }
}

/// The paragraph at `want` in a walk that has already counted `at` of them,
/// without gathering the rest.
///
/// [`Document::paragraphs_in`] allocates a vector of every paragraph in the
/// flow, which is the wrong shape for anything asked once a frame — what is
/// under the pointer, what the caret is standing in.
fn walk_to<'a>(blocks: &'a [Block], at: &mut usize, want: usize) -> Option<&'a Paragraph> {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => {
                if *at == want {
                    return Some(paragraph);
                }
                *at += 1;
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        if let Some(found) = walk_to(&cell.content, at, want) {
                            return Some(found);
                        }
                    }
                }
            }
            Block::Structured(sdt) => {
                if let Some(found) = walk_to(&sdt.content, at, want) {
                    return Some(found);
                }
            }
            Block::Anchor(_) | Block::AltChunk { .. } => {}
        }
    }
    None
}

fn walk_paragraphs<'a>(blocks: &'a [Block], into: &mut Vec<&'a Paragraph>) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => into.push(paragraph),
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        walk_paragraphs(&cell.content, into);
                    }
                }
            }
            Block::Structured(sdt) => walk_paragraphs(&sdt.content, into),
            Block::Anchor(_) | Block::AltChunk { .. } => {}
        }
    }
}

/// Convenience for building a revision wrapper in tests and in an editor.
pub fn inserted_by(author: &str, id: u32, content: Vec<Inline>) -> Inline {
    Inline::Revised {
        revision: Revision::Inserted(Mark::new(id, author)),
        content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numbering::{AbstractNum, Level, Num};
    use crate::prop::{NumRef, ParaProps};
    use crate::revision::Mark;
    use crate::style::{Style, StyleKind};
    use crate::table::{Cell, Row, Table};

    #[test]
    fn a_deletion_is_drawn_but_does_not_count_as_text() {
        let mut paragraph = Paragraph::of("The original ");
        paragraph.content.push(Inline::Revised {
            revision: Revision::Deleted(Mark::new(1, "Adnan Khan")),
            content: vec![Inline::Run(Run {
                content: vec![Piece::Deleted("removed ".into())],
                ..Run::new()
            })],
        });
        paragraph
            .content
            .push(Inline::Run(Run::of("sentence stays.")));

        assert_eq!(paragraph.text(), "The original sentence stays.");
        assert_eq!(
            paragraph.shown_text(),
            "The original removed sentence stays.",
            "the revision view has to show what was deleted"
        );
    }

    #[test]
    fn an_insertion_is_ordinary_text_until_it_is_rejected() {
        let paragraph = Paragraph {
            content: vec![inserted_by(
                "Adnan Khan",
                0,
                vec![Inline::Run(Run::of("added"))],
            )],
            ..Paragraph::new()
        };
        assert_eq!(paragraph.text(), "added");
        assert_eq!(paragraph.shown_text(), "added");
    }

    #[test]
    fn a_click_inside_a_link_finds_it_and_one_past_its_last_letter_does_not() {
        let paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("go to the ")),
                Inline::Hyperlink(Box::new(Hyperlink {
                    rel: Some("rId9".into()),
                    anchor: None,
                    tooltip: None,
                    history: true,
                    content: vec![Inline::Run(Run::of("download page"))],
                })),
                Inline::Run(Run::of(".")),
            ],
            ..Paragraph::new()
        };
        assert_eq!(paragraph.text(), "go to the download page.");
        assert!(paragraph.link_at(0).is_none(), "before it");
        assert!(paragraph.link_at(9).is_none(), "the space in front of it");
        for offset in 10..23 {
            assert!(
                paragraph.link_at(offset).is_some(),
                "{offset} is one of the link's own letters"
            );
        }
        assert!(paragraph.link_at(23).is_none(), "the full stop after it");
        assert_eq!(
            paragraph.link_at(10).and_then(|link| link.rel.clone()),
            Some("rId9".into())
        );
    }

    #[test]
    fn one_paragraph_by_index_is_the_one_the_whole_walk_puts_there() {
        let document = Document {
            body: vec![
                Block::Paragraph(Paragraph::of("before")),
                Block::Table(crate::table::Table {
                    grid: vec![crate::units::Twips(1440)],
                    rows: vec![crate::table::Row {
                        cells: vec![crate::table::Cell {
                            props: crate::table::CellProps::new(),
                            content: vec![Block::Paragraph(Paragraph::of("inside"))],
                        }],
                        ..crate::table::Row::new()
                    }],
                    ..crate::table::Table::new()
                }),
                Block::Paragraph(Paragraph::of("after")),
            ],
            ..Document::new()
        };
        let all = document.paragraphs();
        for (index, paragraph) in all.iter().enumerate() {
            assert_eq!(
                document.paragraph(index).map(|found| found.text()),
                Some(paragraph.text()),
                "paragraph {index}"
            );
        }
        assert!(document.paragraph(all.len()).is_none());
    }

    #[test]
    fn a_bookmark_names_the_paragraph_a_link_to_it_lands_on() {
        let mut heading = Paragraph::of("Paragraph level formatting");
        heading.content.insert(
            0,
            Inline::Anchor(Anchor::BookmarkStart {
                id: 3,
                name: "_Paragraph_level_formatting".into(),
            }),
        );
        let document = Document {
            body: vec![
                Block::Paragraph(Paragraph::of("first")),
                Block::Paragraph(heading),
                Block::Paragraph(Paragraph::of("last")),
            ],
            ..Document::new()
        };
        assert_eq!(document.bookmark("_Paragraph_level_formatting"), Some(1));
        assert_eq!(document.bookmark("_no_such_mark"), None);
    }

    #[test]
    fn runs_are_found_however_deeply_they_are_wrapped() {
        let paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("plain ")),
                Inline::Hyperlink(Box::new(Hyperlink {
                    rel: Some("rId4".into()),
                    anchor: None,
                    tooltip: None,
                    history: true,
                    content: vec![inserted_by(
                        "Adnan Khan",
                        1,
                        vec![Inline::Run(Run::of("linked"))],
                    )],
                })),
            ],
            ..Paragraph::new()
        };
        assert_eq!(paragraph.runs().len(), 2);
        assert_eq!(paragraph.text(), "plain linked");
    }

    #[test]
    fn a_section_break_terminates_rather_than_beginning() {
        // Two paragraphs, the first ending a landscape section. Read as a
        // container, the landscape properties would land on the second.
        let mut first = Paragraph::of("landscape page");
        let mut landscape = SectionProps::new();
        landscape.page = landscape.page.rotated();
        first.section = Some(Box::new(landscape));

        let document = Document {
            body: vec![
                Block::Paragraph(first),
                Block::Paragraph(Paragraph::of("portrait page")),
            ],
            ..Document::new()
        };

        let sections = document.sections();
        assert_eq!(sections.len(), 2);
        assert_eq!(
            sections[0].0,
            0..1,
            "the break's own paragraph is inside it"
        );
        assert_eq!(
            sections[0].1.page.orientation,
            crate::section::Orientation::Landscape
        );
        assert_eq!(sections[1].0, 1..2);
        assert_eq!(
            sections[1].1.page.orientation,
            crate::section::Orientation::Portrait
        );
    }

    #[test]
    fn a_document_with_one_page_setup_has_exactly_one_section() {
        let document = Document::blank();
        let sections = document.sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, 0..1);
    }

    #[test]
    fn paragraphs_inside_tables_are_part_of_the_document() {
        let document = Document {
            body: vec![
                Block::Paragraph(Paragraph::of("before")),
                Block::Table(Table {
                    rows: vec![Row {
                        cells: vec![Cell {
                            content: vec![Block::Paragraph(Paragraph::of("inside"))],
                            ..Cell::new()
                        }],
                        ..Row::new()
                    }],
                    ..Table::new()
                }),
                Block::Paragraph(Paragraph::of("after")),
            ],
            ..Document::new()
        };
        let texts: Vec<String> = document.paragraphs().iter().map(|p| p.text()).collect();
        assert_eq!(texts, ["before", "inside", "after"]);
    }

    #[test]
    fn a_paragraph_is_in_a_list_because_its_style_is() {
        // "List Bullet" paragraphs carry no numPr of their own. Asking the
        // paragraph alone finds no list, and the document draws no bullets.
        let mut document = Document::new();
        let mut definition = AbstractNum::new(0);
        definition.set_level(Level::new(0));
        document.numbering.insert_abstract(definition);
        document.numbering.insert_num(Num::new(1, 0));

        let mut style = Style::new("ListNumber", StyleKind::Paragraph);
        style.para.numbering = Some(NumRef {
            num_id: 1,
            level: 0,
        });
        let style = document.styles.insert(style);

        document.body = vec![
            Block::Paragraph(Paragraph {
                props: ParaProps {
                    style: Some(style),
                    ..ParaProps::default()
                },
                ..Paragraph::of("first")
            }),
            Block::Paragraph(Paragraph {
                props: ParaProps {
                    style: Some(style),
                    ..ParaProps::default()
                },
                ..Paragraph::of("second")
            }),
            Block::Paragraph(Paragraph::of("not a list")),
        ];

        let labels = document.list_labels();
        assert_eq!(labels[0].as_deref(), Some("1."));
        assert_eq!(labels[1].as_deref(), Some("2."));
        assert_eq!(labels[2], None);
    }

    #[test]
    fn a_num_id_of_zero_on_a_paragraph_takes_it_out_of_its_styles_list() {
        let mut document = Document::new();
        let mut definition = AbstractNum::new(0);
        definition.set_level(Level::new(0));
        document.numbering.insert_abstract(definition);
        document.numbering.insert_num(Num::new(1, 0));

        let mut style = Style::new("ListNumber", StyleKind::Paragraph);
        style.para.numbering = Some(NumRef {
            num_id: 1,
            level: 0,
        });
        let style = document.styles.insert(style);

        document.body = vec![Block::Paragraph(Paragraph {
            props: ParaProps {
                style: Some(style),
                numbering: Some(NumRef {
                    num_id: 0,
                    level: 0,
                }),
                ..ParaProps::default()
            },
            ..Paragraph::of("not numbered after all")
        })];
        assert_eq!(document.list_labels(), vec![None]);
    }

    #[test]
    fn the_separators_at_the_top_of_the_footnote_list_are_not_footnotes() {
        let document = Document {
            footnotes: vec![
                Note {
                    id: -1,
                    kind: NoteKind::Separator,
                    content: vec![],
                },
                Note {
                    id: 0,
                    kind: NoteKind::ContinuationSeparator,
                    content: vec![],
                },
                Note {
                    id: 1,
                    kind: NoteKind::Normal,
                    content: vec![Block::Paragraph(Paragraph::of("A real note."))],
                },
            ],
            ..Document::new()
        };
        assert!(document.footnote(-1).is_none());
        assert!(document.footnote(0).is_none());
        assert_eq!(document.footnote(1).unwrap().text(), "A real note.");
    }

    #[test]
    fn the_authors_are_gathered_from_the_changes_and_the_comments() {
        let mut document = Document::new();
        document.comments.push(Comment::new(1, "Reviewer"));
        document.body = vec![Block::Paragraph(Paragraph {
            content: vec![inserted_by(
                "Adnan Khan",
                0,
                vec![Inline::Run(Run::of("x"))],
            )],
            ..Paragraph::new()
        })];
        let authors = document.authors();
        assert_eq!(authors.len(), 2);
        assert!(authors.iter().any(|a| a.as_ref() == "Reviewer"));
        assert!(authors.iter().any(|a| a.as_ref() == "Adnan Khan"));
    }

    #[test]
    fn a_run_of_field_characters_occupies_no_space() {
        let field = Run {
            content: vec![
                Piece::FieldStart {
                    dirty: false,
                    lock: false,
                },
                Piece::Instruction(" PAGE ".into()),
            ],
            ..Run::new()
        };
        assert!(field.is_blank());
        assert!(!Run::of("text").is_blank());
    }

    #[test]
    fn words_own_page_breaks_are_counted_rather_than_dropped() {
        // The only opinion about pagination a .docx contains, and the nearest
        // thing to an oracle a layout engine can be tested against.
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("before ".into()),
                    Piece::LastRenderedPageBreak,
                    Piece::Text("after".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        assert_eq!(paragraph.rendered_page_breaks(), 1);
        assert_eq!(paragraph.text(), "before after");
    }

    #[test]
    fn a_break_reads_its_kind_and_its_clear_together() {
        assert_eq!(Break::from_val(None, None), Break::Line);
        assert_eq!(Break::from_val(Some("page"), None), Break::Page);
        assert_eq!(
            Break::from_val(Some("textWrapping"), Some("all")),
            Break::TextWrapping(Clear::All)
        );
    }
}
