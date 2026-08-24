//! Laying a paragraph out into lines.
//!
//! The pipeline, and where each part lives:
//!
//! ```text
//! paragraph -> resolve properties          (wp_model::style)
//!           -> itemize into units          (here: `units`)
//!           -> measure                     (crate::shape::Shaper)
//!           -> find break opportunities    (crate::linebreak)
//!           -> fill lines, greedily        (here: `fill`)
//!           -> stack, align, justify       (here: `finish`)
//! ```
//!
//! **Greedy, not Knuth-Plass, on purpose.** Word breaks a line as soon as the
//! next word does not fit and never looks back, which is why a Word document has
//! the occasional stretched line that a typesetter would have avoided. Matching
//! Word's *breaks* is the goal (`DESIGN.md` §5); a better algorithm would
//! produce better-looking pages that break in different places, which is the one
//! thing this must not do.
//!
//! **A line is one fragment per word, not one per run.** That is what makes
//! justification a matter of moving fragments apart rather than of asking the
//! renderer to stretch the spaces inside a string, and it is what gives the
//! caret a byte offset to land on.

use wp_model::doc::{Break, Inline, Paragraph, Piece, Run};
use wp_model::numbering::Suffix;
use wp_model::prop::{Justify, LineSpacing, RunProps, TabKind, TabLeader, TabStop};
use wp_model::style::Layers;
use wp_model::units::Twips;

use crate::field::{FieldMark, FieldValues};
use crate::linebreak;
use crate::resolve::{self, TextStyle};
use crate::shape::{FontRequest, Metrics, Shaper};

/// Where a laid-out fragment came from in the document.
///
/// Without this the caret cannot be placed: a click lands on a point of a line,
/// and what has to come back is a position in the text. `run` indexes
/// [`Paragraph::runs`], which is document order however deeply the runs were
/// wrapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub run: usize,
    pub piece: usize,
    /// Byte range within the **paragraph's** text — [`Paragraph::text`], which
    /// is what a caret's offset counts.
    ///
    /// Not within the piece. Naming the piece is the obvious thing and it is
    /// wrong the moment a paragraph holds more than one: a click in the second
    /// run comes back as an offset into the first, so the caret jumps to the
    /// start of the paragraph and a selection paints the wrong letters. A
    /// paragraph of one run — which is most of a test corpus and almost none of
    /// a real document — cannot tell the difference.
    pub start: usize,
    pub end: usize,
}

/// What a fragment draws.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text {
        text: String,
        /// The advance of each character, so the caret can be placed inside the
        /// fragment without measuring again.
        advances: Vec<f64>,
        /// Drawn at the end of this fragment because a soft hyphen broke here.
        hyphen: bool,
    },
    /// A tab's whitespace, and the leader drawn across it.
    Tab {
        leader: TabLeader,
        /// The leader drawn across the gap, once the gap's width is known —
        /// a table of contents is dots, and Word draws them as characters of
        /// the paragraph's own face at that face's own advance, from where the
        /// entry ends to where the page number begins.
        fill: String,
        advances: Vec<f64>,
    },
    /// An inline drawing, and the relationship naming the part that holds its
    /// bytes. Without the relationship the painter has a rectangle of the right
    /// size and nothing to put in it.
    Object {
        height: f64,
        rel: Option<std::sync::Arc<str>>,
        /// The chart part, when the drawing is a chart rather than a picture.
        chart: Option<std::sync::Arc<str>>,
        /// Which of the paragraph's drawings this is, and `None` for an
        /// object that is not one of them: a list's picture bullet is drawn
        /// like a drawing but cannot be clicked, moved or deleted, because
        /// there is nothing in the paragraph to delete.
        nth: Option<usize>,
    },
    /// The bullet or number of a list paragraph.
    ///
    /// It carries its own text: the label is not in the document's runs, so a
    /// renderer handed only the paragraph has nothing to draw it from, and a
    /// variant that says only "a label goes here" is one a painter can do
    /// nothing with but skip.
    Label { text: String, advances: Vec<f64> },
}

/// One drawn piece of a line.
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    /// Left edge, from the left of the *page's text column*. With a run
    /// border this is the left of the *box*; the text starts `lead` further on.
    pub x: f64,
    pub width: f64,
    /// Room reserved before the text for the left side of a run's border.
    pub lead: f64,
    pub style: TextStyle,
    pub content: Content,
    pub source: Option<Source>,
    /// Set when this is the *result* of a field, so a second pass can put the
    /// page number in it. A field's instruction is never drawn and never
    /// reaches here.
    pub field: Option<FieldMark>,
}

/// One laid-out line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub fragments: Vec<Fragment>,
    /// Top of the line, from the top of the paragraph.
    pub y: f64,
    /// Baseline, from the top of the line.
    pub baseline: f64,
    /// Total advance to the next line's top — after the line-spacing rule.
    pub height: f64,
    /// Ink extent above and below the baseline, before the spacing rule.
    pub ascent: f64,
    pub descent: f64,
    /// Left edge and used width, for a caret at the end of the line and for
    /// drawing a selection.
    pub x: f64,
    pub width: f64,
    /// What Word's half-point accumulator counts toward for this line — see
    /// [`crate::shape::Pitch`]. Equal to `height` when the line opts out
    /// (fixed spacing, a picture taller than the type, a shaper that answers
    /// naturally), which makes the drift zero and the dance a no-op.
    pub ideal: f64,
    /// The break that ended this line, if it was not the margin.
    pub ended_by: Option<Break>,
}

/// A paragraph, laid out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LaidParagraph {
    pub lines: Vec<Line>,
    /// `<w:spacing w:before>` and `w:after`, in points, after the contextual
    /// rules have been applied by the caller.
    pub space_before: f64,
    pub space_after: f64,
    /// The height of the lines alone.
    pub height: f64,
}

impl LaidParagraph {
    /// Height including the space above and below.
    pub fn total_height(&self) -> f64 {
        self.space_before + self.height + self.space_after
    }
}

/// The list label of a numbered paragraph.
#[derive(Debug, Clone, PartialEq)]
pub struct ListLabel {
    pub text: String,
    /// The level's own `<w:rPr>` — a bullet's font lives here, and it is why a
    /// Wingdings bullet does not turn the paragraph into Wingdings.
    pub props: RunProps,
    pub suffix: Suffix,
    /// A bullet keeps only its size from the paragraph; a *number* takes the
    /// paragraph mark's bold and italic too. Measured against Word.
    pub bullet: bool,
    /// `<w:numPicBullet>` — a picture drawn in place of the glyph. The text
    /// stays: it is the character Word leaves in `<w:lvlText>` for a reader
    /// that cannot fetch the image, and it is what the label reads as.
    pub picture: Option<wp_model::numbering::PictureBullet>,
}

/// What the layout needs beyond the paragraph itself.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    pub theme: &'a wp_model::color::Theme,
    /// The number of the note whose own content is being laid out, which is
    /// what `<w:footnoteRef/>` draws. `None` anywhere but inside a note.
    pub note_mark: Option<&'a str>,
    /// The mark each note is referenced by. Built once per document — see
    /// [`crate::notes::NoteMarks`].
    pub notes: &'a crate::notes::NoteMarks,
    /// What the table style says about the cell being laid out, if this is
    /// one. Carried on the context because it changes per cell and every
    /// paragraph inside that cell needs it.
    pub table_part: Option<&'a wp_model::TablePart>,
    /// The document's styles, for the character style a *run* names.
    ///
    /// The paragraph's own chain is resolved before layout begins, but
    /// `<w:rStyle>` is per run — one paragraph's runs may name Strong, Subtle
    /// Emphasis and nothing at all — so the table has to be in reach here.
    pub styles: &'a wp_model::style::StyleTable,
    /// `<w:defaultTabStop>` — where tabs land when the paragraph defines none.
    pub default_tab: Twips,
    /// The face to use when neither the run nor the theme names one.
    pub fallback_font: &'a str,
    /// Whether the machine truly has a face of this name.
    ///
    /// Asked before translating a symbol font's private-use bullet to its
    /// Unicode stand-in: a machine that has Symbol itself draws the same glyph
    /// Word does, and the translation is only for the machine that does not.
    pub has_face: fn(&str) -> bool,
    /// Whether tracked deletions are drawn. Word's default is to show them.
    pub show_revisions: bool,
    /// Whether `w:vanish` text is drawn — the formatting-marks switch.
    pub show_hidden: bool,
    /// What each field evaluates to, where it is known.
    ///
    /// Empty on the first pass — a page number cannot be known before the page
    /// exists — and filled in for the second.
    pub fields: &'a FieldValues,
    /// Which band is being laid out: `None` for the document body, `Some(page)`
    /// for the header or footer of that page. See [`FieldMark::band`].
    pub band: Option<u32>,
    /// Which paragraphs a float anchored to the page or a margin narrows, and
    /// by how much. Empty on the first pass, because where such a float sits
    /// is not known until the pages exist — see [`crate::block::Wraps`].
    pub wraps: &'a crate::block::Wraps,
}

impl Default for Context<'_> {
    fn default() -> Self {
        Context {
            theme: Box::leak(Box::new(wp_model::color::Theme::default())),
            notes: Box::leak(Box::new(crate::notes::NoteMarks::default())),
            note_mark: None,
            table_part: None,
            styles: Box::leak(Box::new(wp_model::style::StyleTable::default())),
            default_tab: Twips(720),
            fallback_font: "Calibri",
            has_face: |_| false,
            show_revisions: true,
            show_hidden: false,
            fields: Box::leak(Box::new(FieldValues::default())),
            band: None,
            wraps: Box::leak(Box::new(crate::block::Wraps::default())),
        }
    }
}

/// One indivisible piece of the paragraph, before lines exist.
#[derive(Debug, Clone)]
struct Unit {
    style: TextStyle,
    source: Option<Source>,
    field: Option<FieldMark>,
    kind: UnitKind,
    /// Width of the part that must fit on the line.
    width: f64,
    /// Width of the trailing spaces, which may hang past the margin.
    trailing: f64,
    /// Part of `width` that is a run border's own room, before the text.
    lead: f64,
    /// No line may be broken between this unit and the one before it.
    ///
    /// A paragraph is cut into units at every break opportunity *and* at every
    /// seam where the measuring has to change — a new run, a new script, a
    /// field. Only the first kind is a place a line may end, and without this
    /// flag the second kind becomes one: the demonstration document's bold
    /// quotation mark was left alone at the end of a line, with the words it
    /// opens on the next.
    joined: bool,
}

#[derive(Debug, Clone)]
enum UnitKind {
    /// `text` includes the trailing spaces; `content` is the byte length of the
    /// part before them.
    Text {
        text: String,
        advances: Vec<f64>,
        hyphen: bool,
    },
    Tab {
        leader: TabLeader,
        /// This is the tab that follows a list label, which lands somewhere a
        /// tab typed by a user does not. See `fill`.
        after_label: bool,
    },
    Break(Break),
    Object {
        height: f64,
        rel: Option<std::sync::Arc<str>>,
        chart: Option<std::sync::Arc<str>>,
        nth: Option<usize>,
    },
    Label {
        text: String,
        advances: Vec<f64>,
    },
}

/// Something beside the paragraph that its lines have to make room for.
///
/// A floating table, a drop cap, or a picture the text is set beside. The
/// depth is measured from the top of the paragraph, so a paragraph that starts
/// halfway down the float is told how much of it is left. Both sides at once,
/// because a page may hold a picture at each margin and the text between them
/// is narrowed from both.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Obstacle {
    /// How far down from the top of this paragraph the obstacle still reaches.
    pub depth: f64,
    /// How much measure it takes away, from the left.
    pub indent: f64,
    /// How much it takes away from the right.
    pub inset: f64,
}

/// Lays a paragraph out into lines of `width` points.
#[allow(clippy::too_many_arguments)]
pub fn layout(
    paragraph: &Paragraph,
    index: usize,
    layers: &Layers,
    label: Option<&ListLabel>,
    ctx: &Context<'_>,
    width: f64,
    obstacle: Option<Obstacle>,
    shaper: &mut dyn Shaper,
) -> LaidParagraph {
    let units = units(paragraph, index, layers, label, ctx, shaper);
    let indent = &layers.para.indent;
    let start = indent.start.map(|t| t.points()).unwrap_or(0.0);
    let end = indent.end.map(|t| t.points()).unwrap_or(0.0);
    let first = indent.first_line_offset().points();

    let tabs = tab_stops(layers, ctx);
    // Which lines a float narrows cannot be known before the lines exist, and
    // the lines cannot be laid without knowing how wide they are. So it is
    // settled by going round: lay them, see which ones the float actually
    // reaches, lay them again. Two passes settle every real case; the bound is
    // there so a pathological one cannot spin.
    let mut indents: Vec<(f64, f64)> = Vec::new();
    let mut lines;
    for _ in 0..4 {
        lines = fill(&units, width, start, end, first, &tabs, ctx, &indents);
        finish(
            &mut lines, layers, width, start, end, first, &indents, paragraph, ctx, shaper,
        );
        let settled = beside(&lines, obstacle);
        if settled == indents {
            return done(lines, layers, paragraph, ctx, shaper);
        }
        indents = settled;
    }
    let mut lines = fill(&units, width, start, end, first, &tabs, ctx, &indents);
    finish(
        &mut lines, layers, width, start, end, first, &indents, paragraph, ctx, shaper,
    );
    done(lines, layers, paragraph, ctx, shaper)
}

/// The paragraph, once its lines have settled.
fn done(
    lines: Vec<Line>,
    layers: &Layers,
    paragraph: &Paragraph,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
) -> LaidParagraph {
    let _ = (paragraph, ctx);
    let mut lines = lines;
    lead_in(&mut lines, shaper);
    let height = lines.iter().map(|line| line.height).sum();
    LaidParagraph {
        lines,
        space_before: layers
            .para
            .spacing
            .before
            .map(|t| t.points())
            .unwrap_or(0.0),
        space_after: layers.para.spacing.after.map(|t| t.points()).unwrap_or(0.0),
        height,
    }
}

/// Fills every leadered tab on the line with the characters Word draws there.
///
/// Measured: a dot leader is the paragraph's own full stop repeated at the
/// face's own advance, laid from where the text before the tab ends and
/// stopping short of where the text after it begins. Nothing is squeezed or
/// spread — the gap simply holds as many as it holds.
fn lead_in(lines: &mut [Line], shaper: &mut dyn Shaper) {
    for line in lines.iter_mut() {
        for fragment in line.fragments.iter_mut() {
            let style = fragment.style.clone();
            let width = fragment.width;
            let Content::Tab {
                leader,
                fill,
                advances,
            } = &mut fragment.content
            else {
                continue;
            };
            let Some(glyph) = leader.character() else {
                continue;
            };
            let one = shaper.width(&glyph.to_string(), &style.font);
            if one <= 0.0 {
                continue;
            }
            let count = (width / one).floor().max(0.0) as usize;
            *fill = std::iter::repeat_n(glyph, count).collect();
            *advances = vec![one; count];
        }
    }
}

/// A note's number, which the layout makes rather than the document holding.
///
/// Emitted as a *label* and not as text for the same reason a list's bullet is:
/// it is drawn, it takes room on the line, and it is not a character of the
/// paragraph. A caret cannot sit inside it and a search cannot find it.
#[allow(clippy::too_many_arguments)]
fn push_note_mark(
    mark: &str,
    props: &RunProps,
    run: usize,
    piece: usize,
    at: usize,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    out: &mut Vec<Unit>,
) {
    let style = style_for(mark.chars().next().unwrap_or('1'), props, ctx);
    let mut advances = Vec::new();
    let width = measure(mark, &style, shaper, &mut advances);
    out.push(Unit {
        style,
        source: Some(Source {
            run,
            piece,
            start: at,
            end: at,
        }),
        field: None,
        kind: UnitKind::Label {
            text: mark.to_owned(),
            advances,
        },
        width,
        trailing: 0.0,
        joined: false,
        lead: 0.0,
    });
}

/// How far each line has to stand clear of an obstacle beside the paragraph.
///
/// A line is pushed aside when any part of it lies within the obstacle's depth,
/// which is what makes the line that straddles the bottom of a floating table
/// the last narrow one rather than the first wide one.
fn beside(lines: &[Line], obstacle: Option<Obstacle>) -> Vec<(f64, f64)> {
    let clear = (0.0, 0.0);
    let Some(obstacle) = obstacle.filter(|o| o.depth > 0.0 && (o.indent > 0.0 || o.inset > 0.0))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut y = 0.0;
    for line in lines {
        out.push(if y < obstacle.depth - 0.01 {
            (obstacle.indent, obstacle.inset)
        } else {
            clear
        });
        y += line.height;
    }
    while out.last() == Some(&clear) {
        out.pop();
    }
    out
}

/// The tab stops in force, the paragraph's own ahead of the document default.
fn tab_stops(layers: &Layers, ctx: &Context<'_>) -> Vec<TabStop> {
    layers
        .para
        .tabs
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|stop| stop.kind != TabKind::Clear)
        .collect::<Vec<_>>()
        .tap_sorted(ctx)
}

trait Sorted {
    fn tap_sorted(self, ctx: &Context<'_>) -> Self;
}

impl Sorted for Vec<TabStop> {
    fn tap_sorted(mut self, _ctx: &Context<'_>) -> Self {
        self.sort_by_key(|stop| stop.position);
        self
    }
}

/// Where the next tab lands, from `x` points into the text column.
fn next_tab(x: f64, stops: &[TabStop], default: Twips) -> (f64, TabKind, TabLeader) {
    for stop in stops {
        let at = stop.position.points();
        if at > x + 0.01 {
            return (at, stop.kind, stop.leader);
        }
    }
    // Past the last explicit stop, the document's default interval takes over —
    // measured from the left of the text column, not from the last stop.
    let step = default.points().max(1.0);
    let next = ((x / step).floor() + 1.0) * step;
    (next, TabKind::Start, TabLeader::None)
}

/// Walks the paragraph into indivisible units.
#[allow(clippy::too_many_arguments)]
fn units(
    paragraph: &Paragraph,
    index: usize,
    layers: &Layers,
    label: Option<&ListLabel>,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
) -> Vec<Unit> {
    let mut out = Vec::new();

    if let Some(label) = label {
        // The label takes the level's own formatting laid over the paragraph
        // *mark's*, which is what keeps a bullet in Symbol while the text
        // stays in Calibri — and sizes it with the paragraph rather than with
        // the document default, which is what keeps a 10.5pt list item's line
        // from being as tall as 12pt type.
        let mut props = layers.run.clone();
        if let Some(mark) = &paragraph.props.mark {
            props.layer(mark, wp_model::prop::Layer::Direct);
        }
        if label.bullet {
            // A bullet glyph never goes bold or italic with its paragraph —
            // Word draws the same dot beside a bold item and a plain one, and
            // the same dash whether or not the mark carries `<w:b/>`. Only the
            // size follows the text. The level's own rPr goes on next, so a
            // numbering that *defines* a bold bullet still gets one.
            for toggle in [
                wp_model::Toggle::Bold,
                wp_model::Toggle::BoldCs,
                wp_model::Toggle::Italic,
                wp_model::Toggle::ItalicCs,
            ] {
                props.toggles.set(toggle, false);
            }
        }
        props.layer(&label.props, wp_model::prop::Layer::Direct);
        let script = label
            .text
            .chars()
            .next()
            .map(resolve::face_for)
            .unwrap_or(wp_model::prop::Script::Ascii);
        let mut style = resolve::text_style(&props, ctx.theme, script, ctx.fallback_font);
        // A label is never underlined or struck through. Word takes the
        // paragraph mark's bold, italic and size into the number — but not its
        // underline: a mark carrying `w:u` (LibreOffice copies the run's
        // formatting there) bolds "1." without drawing a line under it, and
        // the tab after the label stays bare too.
        style.underline = wp_model::prop::UnderlineKind::None;
        style.underline_color = None;
        style.strike = false;
        style.double_strike = false;
        let mut advances = Vec::new();
        let width = measure(&label.text, &style, shaper, &mut advances);
        // A picture bullet is drawn instead of the glyph, at the size the
        // numbering states and not at the image's own — the icons Word ships
        // are hundreds of pixels across. A shape that states no size gets the
        // type's, which is the one measurement always to hand.
        let (kind, width) = match &label.picture {
            Some(picture) => {
                let side = |stated: f64| match stated > 0.0 {
                    true => stated,
                    false => style.font.size,
                };
                (
                    UnitKind::Object {
                        height: side(picture.height),
                        rel: Some(picture.rel.clone()),
                        chart: None,
                        nth: None,
                    },
                    side(picture.width),
                )
            }
            None => (
                UnitKind::Label {
                    text: label.text.clone(),
                    advances,
                },
                width,
            ),
        };
        out.push(Unit {
            style,
            source: None,
            field: None,
            kind,
            width,
            trailing: 0.0,
            joined: false,
            lead: 0.0,
        });
        match label.suffix {
            Suffix::Tab => out.push(tab_unit(&out[0].style, TabLeader::None, true)),
            Suffix::Space => {
                let style = out[0].style.clone();
                let mut advances = Vec::new();
                let width = measure(" ", &style, shaper, &mut advances);
                out.push(Unit {
                    style,
                    source: None,
                    field: None,
                    kind: UnitKind::Text {
                        text: " ".to_string(),
                        advances,
                        hyphen: false,
                    },
                    width: 0.0,
                    trailing: width,
                    joined: false,
                    lead: 0.0,
                });
            }
            Suffix::Nothing => {}
        }
    }

    let runs = paragraph.runs();
    let deleted = deleted_runs(paragraph);
    // Field state runs *across* runs: a field's begin, its instruction, its
    // separator and its result are very often four different `<w:r>` elements.
    let mut fields = FieldWalk::new(index, ctx.band);
    // How far into the paragraph's text each run starts. Counted over every run
    // in `runs()` order whether or not it is drawn, because that is the walk a
    // caret's offset is counted along — see `Piece::text_len`.
    let mut base = 0usize;
    for (run_index, run) in runs.iter().enumerate() {
        let length: usize = run.content.iter().map(Piece::text_len).sum();
        if !(deleted.contains(&run_index) && !ctx.show_revisions) {
            push_run(
                run,
                run_index,
                base,
                layers,
                ctx,
                shaper,
                &mut fields,
                &mut out,
            );
        }
        base += length;
    }
    number_drawings(paragraph, &mut out);
    join_unbreakable(&mut out);
    out
}

/// Where the walk is among a paragraph's fields.
///
/// Fields nest — a `TOC` holds a `PAGEREF` per row — so this is a stack rather
/// than a flag, and the ordinal is handed out at the `begin` so it does not
/// depend on how deeply the field sits.
struct FieldWalk {
    paragraph: usize,
    band: Option<u32>,
    next_ordinal: usize,
    stack: Vec<OpenField>,
}

struct OpenField {
    ordinal: usize,
    instruction: String,
    /// Before the separator everything is code; after it, everything is result.
    in_result: bool,
    kind: Option<wp_model::field::Kind>,
}

impl FieldWalk {
    fn new(paragraph: usize, band: Option<u32>) -> FieldWalk {
        FieldWalk {
            paragraph,
            band,
            next_ordinal: 0,
            stack: Vec::new(),
        }
    }

    fn begin(&mut self) {
        let ordinal = self.next_ordinal;
        self.next_ordinal += 1;
        self.stack.push(OpenField {
            ordinal,
            instruction: String::new(),
            in_result: false,
            kind: None,
        });
    }

    fn instruction(&mut self, text: &str) {
        if let Some(open) = self.stack.last_mut() {
            open.instruction.push_str(text);
        }
    }

    fn separate(&mut self) {
        if let Some(open) = self.stack.last_mut() {
            open.in_result = true;
            open.kind = wp_model::Field::parse(&open.instruction).map(|field| field.kind());
        }
    }

    fn end(&mut self) {
        self.stack.pop();
    }

    /// Whether what is being read now is a field's code rather than its result.
    fn in_instruction(&self) -> bool {
        self.stack.last().is_some_and(|open| !open.in_result)
    }

    /// The innermost field whose result this is.
    fn mark(&self) -> Option<FieldMark> {
        let open = self.stack.last()?;
        open.in_result.then(|| FieldMark {
            paragraph: self.paragraph,
            ordinal: open.ordinal,
            band: self.band,
            kind: open.kind.unwrap_or(wp_model::field::Kind::Other),
        })
    }
}

fn tab_unit(style: &TextStyle, leader: TabLeader, after_label: bool) -> Unit {
    Unit {
        style: style.clone(),
        source: None,
        field: None,
        kind: UnitKind::Tab {
            leader,
            after_label,
        },
        width: 0.0,
        trailing: 0.0,
        joined: false,
        lead: 0.0,
    }
}

/// The indices, into `paragraph.runs()`, of runs inside a tracked deletion.
fn deleted_runs(paragraph: &Paragraph) -> Vec<usize> {
    let mut out = Vec::new();
    let mut index = 0usize;
    fn walk(content: &[Inline], inside: bool, index: &mut usize, out: &mut Vec<usize>) {
        for inline in content {
            match inline {
                Inline::Run(_) => {
                    if inside {
                        out.push(*index);
                    }
                    *index += 1;
                }
                Inline::Revised { revision, content } => {
                    walk(content, inside || !revision.is_present(), index, out)
                }
                Inline::Hyperlink(link) => walk(&link.content, inside, index, out),
                Inline::Structured(sdt) => walk(&sdt.content, inside, index, out),
                Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                    walk(content, inside, index, out)
                }
                Inline::Anchor(_) | Inline::Math(_) => {}
            }
        }
    }
    walk(&paragraph.content, false, &mut index, &mut out);
    out
}

/// Marks the seams that are not places a line may end.
///
/// The paragraph has been cut wherever the *measuring* has to change — a new
/// run, a change of script, a field boundary — and those cuts are not break
/// opportunities. The text either side of each seam is asked, one boundary at
/// a time, exactly as the text inside a run was asked.
fn join_unbreakable(out: &mut [Unit]) {
    for i in 1..out.len() {
        let (before, after) = (&out[i - 1], &out[i]);
        let (UnitKind::Text { text: left, .. }, UnitKind::Text { text: right, .. }) =
            (&before.kind, &after.kind)
        else {
            continue;
        };
        let (Some(last), Some(first)) = (left.chars().next_back(), right.chars().next()) else {
            continue;
        };
        out[i].joined = !linebreak::may_break_at(last, first);
    }
}

/// Tells each inline drawing which of the paragraph's drawings it is.
///
/// An anchored drawing is not laid out inline but is still counted, so that the
/// number means the same thing here as it does to `Paragraph::drawing_mut`.
fn number_drawings(paragraph: &Paragraph, out: &mut [Unit]) {
    let inline: Vec<usize> = paragraph
        .drawings()
        .iter()
        .enumerate()
        .filter(|(_, drawing)| !drawing.anchored)
        .map(|(index, _)| index)
        .collect();
    let mut next = inline.iter();
    for unit in out.iter_mut() {
        // A picture bullet is an object too, and it is not one of these:
        // numbering it would hand a click on the bullet the paragraph's
        // first real drawing.
        if let UnitKind::Object { nth: Some(nth), .. } = &mut unit.kind {
            *nth = next.next().copied().unwrap_or(0);
        }
    }
}

/// How a slice of text being laid out maps back onto the paragraph's bytes.
#[derive(Debug, Clone)]
enum Mapping {
    /// The text is the piece's own, starting at this byte of the paragraph, so
    /// an offset into one is an offset into the other.
    Verbatim(usize),
    /// The text only stands *for* the piece — a field's computed value drawn
    /// instead of the one the file cached, or a tracked deletion, which is drawn
    /// and occupies none of the paragraph's text. Every fragment of it names the
    /// whole span the piece occupies, so a caret cannot come to rest inside
    /// something that has no bytes of its own to rest on.
    WholeOf(std::ops::Range<usize>),
}

impl Mapping {
    /// The range a unit covering `start..end` of the slice belongs to.
    fn range(&self, start: usize, end: usize) -> (usize, usize) {
        match self {
            Mapping::Verbatim(base) => (base + start, base + end),
            Mapping::WholeOf(span) => (span.start, span.end),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_run(
    run: &Run,
    index: usize,
    // Where this run's first piece begins in the paragraph's text.
    base: usize,
    layers: &Layers,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    fields: &mut FieldWalk,
    out: &mut Vec<Unit>,
) {
    // The paragraph's run layer, then whatever character style this run names,
    // then the run's own direct properties on top.
    let props = ctx.styles.resolve_run(layers, &run.props);
    if props.hidden() && !ctx.show_hidden {
        return;
    }

    // A field's instruction is never drawn; only its cached result is. Between
    // `begin` and `separate` everything is code, and a renderer that draws it
    // shows the user ` PAGE \* MERGEFORMAT ` in the middle of the sentence.
    // Where this piece begins in the paragraph's text. It advances for every
    // piece, drawn or not: a run the reader skipped still holds bytes that the
    // pieces after it are counted from.
    let mut at = base;
    for (piece_index, piece) in run.content.iter().enumerate() {
        let span = at..at + piece.text_len();
        at = span.end;
        match piece {
            Piece::FieldStart { .. } => fields.begin(),
            Piece::FieldSeparate => fields.separate(),
            Piece::FieldEnd => {
                // A field can close with nothing between its separator and its
                // end — Google Docs writes `{ PAGE }` that way, cached result
                // and all, and leaves it to the reader to work the number out.
                // There is nothing to substitute into, so a placeholder of no
                // width is drawn instead: it carries the mark, which is what
                // tells the second pass which page this field landed on.
                if let Some(mark) = fields.mark() {
                    if !already_drawn(out, Some(mark)) {
                        match ctx.fields.get(mark) {
                            Some(value) => {
                                push_text(
                                    value,
                                    &props,
                                    index,
                                    piece_index,
                                    Mapping::WholeOf(span.clone()),
                                    ctx,
                                    shaper,
                                    out,
                                );
                                mark_last(out, Some(mark));
                            }
                            None => out.push(Unit {
                                style: style_for(' ', &props, ctx),
                                source: None,
                                field: Some(mark),
                                kind: UnitKind::Text {
                                    text: String::new(),
                                    advances: Vec::new(),
                                    hyphen: false,
                                },
                                width: 0.0,
                                trailing: 0.0,
                                joined: false,
                                lead: 0.0,
                            }),
                        }
                    }
                }
                fields.end();
            }
            Piece::Instruction(text) => fields.instruction(text),
            Piece::DeletedInstruction(_) => {}
            _ if fields.in_instruction() => {}
            Piece::Text(text) | Piece::Deleted(text)
                if !(matches!(piece, Piece::Deleted(_)) && !ctx.show_revisions) =>
            {
                // A field whose value is known draws that instead of what the
                // file cached — and the *whole* result is replaced by the first
                // piece of it, because a cached `12` is very often a run `1` and
                // a run `2` and substituting into both would give `77`.
                let mark = fields.mark();
                match mark.and_then(|mark| ctx.fields.get(mark)) {
                    Some(value) if !already_drawn(out, mark) => {
                        push_text(
                            value,
                            &props,
                            index,
                            piece_index,
                            Mapping::WholeOf(span.clone()),
                            ctx,
                            shaper,
                            out,
                        );
                        mark_last(out, mark);
                    }
                    Some(_) => {}
                    None => {
                        // A deletion is drawn and holds none of the paragraph's
                        // text, so its span is empty and `Verbatim` would run
                        // its offsets over whatever follows it.
                        let map = match piece {
                            Piece::Deleted(_) => Mapping::WholeOf(span.clone()),
                            _ => Mapping::Verbatim(span.start),
                        };
                        push_text(text, &props, index, piece_index, map, ctx, shaper, out);
                        mark_last(out, mark);
                    }
                }
            }
            Piece::Tab => {
                let style = style_for('\t', &props, ctx);
                let mut unit = tab_unit(&style, TabLeader::None, false);
                unit.field = fields.mark();
                // A tab is one byte of the paragraph's text and the caret has to
                // be able to sit on either side of it.
                unit.source = Some(Source {
                    run: index,
                    piece: piece_index,
                    start: span.start,
                    end: span.end,
                });
                out.push(unit);
            }
            Piece::Break(kind) => {
                let style = style_for(' ', &props, ctx);
                out.push(Unit {
                    style,
                    source: None,
                    field: None,
                    kind: UnitKind::Break(*kind),
                    width: 0.0,
                    trailing: 0.0,
                    joined: false,
                    lead: 0.0,
                });
            }
            Piece::Hyphen { breaking } => {
                // A non-breaking hyphen draws; a soft one draws only where the
                // line actually breaks, which `linebreak` decides.
                let text = if *breaking { "\u{00AD}" } else { "\u{2011}" };
                push_text(
                    text,
                    &props,
                    index,
                    piece_index,
                    Mapping::Verbatim(span.start),
                    ctx,
                    shaper,
                    out,
                );
            }
            Piece::Symbol { ch, font } => {
                let mut props = props.clone();
                props.fonts.ascii = Some(font.clone());
                props.fonts.high_ansi = Some(font.clone());
                props.fonts.ascii_theme = None;
                props.fonts.high_ansi_theme = None;
                push_text(
                    &ch.to_string(),
                    &props,
                    index,
                    piece_index,
                    Mapping::Verbatim(span.start),
                    ctx,
                    shaper,
                    out,
                );
            }
            // The note's own number, at the head of the note itself.
            Piece::NoteMark { .. } => {
                let Some(mark) = ctx.note_mark else { continue };
                push_note_mark(
                    mark,
                    &props,
                    index,
                    piece_index,
                    span.start,
                    ctx,
                    shaper,
                    out,
                );
            }
            // The little number where a note is referenced. The run carries
            // the `FootnoteReference` character style, which is what raises
            // and shrinks it — nothing here has to know it is superscript.
            Piece::FootnoteRef { id, custom_mark } | Piece::EndnoteRef { id, custom_mark } => {
                if *custom_mark {
                    // The author supplied the mark as text of its own; drawing
                    // a number beside it would show both.
                    continue;
                }
                let endnote = matches!(piece, Piece::EndnoteRef { .. });
                let Some(mark) = ctx.notes.mark(endnote, *id) else {
                    continue;
                };
                push_note_mark(
                    mark,
                    &props,
                    index,
                    piece_index,
                    span.start,
                    ctx,
                    shaper,
                    out,
                );
            }
            Piece::Drawing(drawing) if !drawing.anchored => {
                let style = style_for(' ', &props, ctx);
                out.push(Unit {
                    style,
                    source: Some(Source {
                        run: index,
                        piece: piece_index,
                        start: span.start,
                        end: span.end,
                    }),
                    field: None,
                    kind: UnitKind::Object {
                        height: drawing.extent.1.points(),
                        rel: drawing.rel.clone(),
                        chart: drawing.chart.clone(),
                        // Filled in once the whole paragraph is walked: a unit
                        // does not know how many drawings came before it.
                        nth: Some(0),
                    },
                    width: drawing.extent.0.points(),
                    trailing: 0.0,
                    joined: false,
                    lead: 0.0,
                });
            }
            _ => {}
        }
    }
}

/// Whether this field's result has already been drawn once.
fn already_drawn(out: &[Unit], mark: Option<FieldMark>) -> bool {
    let Some(mark) = mark else {
        return false;
    };
    out.iter().any(|unit| unit.field == Some(mark))
}

fn mark_last(out: &mut [Unit], mark: Option<FieldMark>) {
    if let Some(unit) = out.last_mut() {
        unit.field = mark;
    }
}

fn style_for(c: char, props: &RunProps, ctx: &Context<'_>) -> TextStyle {
    resolve::text_style(props, ctx.theme, resolve::face_for(c), ctx.fallback_font)
}

fn measure(text: &str, style: &TextStyle, shaper: &mut dyn Shaper, into: &mut Vec<f64>) -> f64 {
    let font = if style.small_caps {
        style.small_cap_font()
    } else {
        style.font.clone()
    };
    let mut raw = Vec::new();
    shaper.advances(text, &font, &mut raw);
    let mut total = 0.0;
    for advance in raw {
        let width = advance.width + style.letter_spacing;
        into.push(width);
        total += width;
    }
    total
}

/// Splits a run's text into units at its break opportunities, and at the script
/// boundaries that change which face draws it.
#[allow(clippy::too_many_arguments)]
fn push_text(
    text: &str,
    props: &RunProps,
    run: usize,
    piece: usize,
    map: Mapping,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    out: &mut Vec<Unit>,
) {
    if text.is_empty() {
        return;
    }
    let breaks = linebreak::opportunities(text);
    let mut bounds: Vec<usize> = vec![0];
    bounds.extend(breaks.iter().copied());
    // A run may mix scripts — "Mixed مرحبا inline." is one `<w:t>` — and each
    // script wants a different face, so a unit may not span the boundary.
    let mut previous_script = None;
    for (offset, c) in text.char_indices() {
        let script = resolve::face_for(c);
        if previous_script.is_some_and(|before| before != script) && offset > 0 {
            bounds.push(offset);
        }
        previous_script = Some(script);
    }
    bounds.push(text.len());
    bounds.sort_unstable();
    bounds.dedup();

    // A run's own border takes room outside the type on all four sides. The
    // horizontal share falls on the ends of the run: the first piece is pushed
    // right of where it would have sat, the last reserves the same again.
    let first_bound = bounds.first().copied().unwrap_or(0);
    let last_bound = bounds.last().copied().unwrap_or(0);
    let pad = style_for(text.chars().next().unwrap_or(' '), props, ctx).border_pad();

    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let slice = &text[start..end];
        if slice.is_empty() {
            continue;
        }
        let lead = if start == first_bound { pad } else { 0.0 };
        let tail = if end == last_bound { pad } else { 0.0 };
        let first = slice.chars().next().unwrap_or(' ');
        let style = style_for(first, props, ctx);
        let drawn = style.transform(slice);
        let drawn = drawn.as_deref().unwrap_or(slice);

        // The trailing spaces hang past the margin rather than counting toward
        // the line's width.
        let content_len = slice
            .char_indices()
            .rev()
            .take_while(|(_, c)| linebreak::is_hanging_space(*c))
            .last()
            .map(|(offset, _)| offset)
            .unwrap_or(slice.len());

        let mut advances = Vec::new();
        let total = measure(drawn, &style, shaper, &mut advances);
        // `content_len` is a byte offset into `slice`; when the text was
        // transformed for small capitals the two may differ in length, so the
        // split is done on character count rather than on bytes.
        let content_chars = slice[..content_len].chars().count();
        let content_width: f64 = advances.iter().take(content_chars).sum();

        let (from, to) = map.range(start, end);
        out.push(Unit {
            style,
            source: Some(Source {
                run,
                piece,
                start: from,
                end: to,
            }),
            field: None,
            kind: UnitKind::Text {
                text: drawn.to_string(),
                advances,
                hyphen: linebreak::breaks_with_hyphen(text, end),
            },
            width: content_width + lead + tail,
            trailing: total - content_width,
            joined: false,
            lead,
        });
    }
}

/// Greedy line filling.
#[allow(clippy::too_many_arguments)]
fn fill(
    units: &[Unit],
    width: f64,
    start: f64,
    end: f64,
    first: f64,
    tabs: &[TabStop],
    ctx: &Context<'_>,
    // How far each line stands clear of something beside the paragraph, on
    // the left and on the right. Short or empty means the rest of the lines
    // are unobstructed.
    beside: &[(f64, f64)],
) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut fragments: Vec<Fragment> = Vec::new();
    // Where the two pens stood before each fragment was placed, so that a
    // cluster that must not be broken can be taken back off the line it did
    // not fit on and carried whole to the next.
    let mut rewind: Vec<(f64, f64)> = Vec::new();
    let mut is_first = true;
    // Two pens, and the difference between them is the whole trailing-space
    // rule. `pen` is where the next unit starts; `used` is where the line's
    // *content* ends. A line's width is `used`, so the spaces at the end of it
    // hang past the margin instead of counting against it.
    let mut pen = 0.0f64;
    let mut used = 0.0f64;
    let available = |is_first: bool, line: usize| {
        let left = if is_first { start + first } else { start };
        let (aside, inset) = beside.get(line).copied().unwrap_or((0.0, 0.0));
        (width - end - left - aside - inset).max(1.0)
    };

    let mut index = 0usize;
    while index < units.len() {
        let unit = &units[index];
        let limit = available(is_first, lines.len());
        match &unit.kind {
            UnitKind::Break(kind) => {
                index += 1;
                lines.push(raw_line(std::mem::take(&mut fragments), used, Some(*kind)));
                pen = 0.0;
                used = 0.0;
                is_first = false;
                continue;
            }
            UnitKind::Tab {
                leader,
                after_label,
            } => {
                let left = if is_first { start + first } else { start };
                let here = pen + left;
                // The tab that follows a list label goes to the paragraph's own
                // left indent before it considers any tab stop. That is what
                // lines a bullet's first line up with the wrapped lines under
                // it; sending it to the next default stop instead leaves half
                // an inch of white between every bullet and its text.
                let (stop, kind, stop_leader) = if *after_label && start > here + 0.01 {
                    (start, TabKind::Start, TabLeader::None)
                } else {
                    next_tab(here, tabs, ctx.default_tab)
                };
                // A centre or right tab does not land the text on the stop: it
                // lays the text so its middle, or its right edge, sits there.
                // That needs the advance of the run the tab introduces — the
                // units up to the next tab or the line's end — which a left tab,
                // which simply advances, never has to look at. The advance counts
                // the spaces *between* those units, which are drawn; only the
                // last unit's trailing space hangs and is left out.
                let following = |from: usize| -> f64 {
                    let seg: Vec<&Unit> = units[from..]
                        .iter()
                        .take_while(|u| {
                            !matches!(u.kind, UnitKind::Tab { .. } | UnitKind::Break(_))
                        })
                        .collect();
                    seg.iter()
                        .enumerate()
                        .map(|(i, u)| u.width + if i + 1 < seg.len() { u.trailing } else { 0.0 })
                        .sum()
                };
                let seg_left = match kind {
                    TabKind::Center => stop - following(index + 1) / 2.0,
                    TabKind::End => stop - following(index + 1),
                    _ => stop,
                };
                let target = (seg_left - left).min(limit);
                let advance = (target - pen).max(0.0);
                fragments.push(Fragment {
                    x: pen,
                    width: advance,
                    lead: 0.0,
                    style: unit.style.clone(),
                    content: Content::Tab {
                        leader: if *leader == TabLeader::None {
                            stop_leader
                        } else {
                            *leader
                        },
                        // Filled in once the gap is settled — see `lead_in`.
                        fill: String::new(),
                        advances: Vec::new(),
                    },
                    source: unit.source,
                    field: unit.field,
                });
                pen += advance;
                used = pen;
                index += 1;
                continue;
            }
            _ => {}
        }

        let fits = pen + unit.width <= limit + 0.01;
        if !fits && !fragments.is_empty() {
            // The break belongs at the head of whatever cluster this unit is
            // part of, not between it and the piece it is joined to.
            let mut back = 0usize;
            while back < fragments.len() && units[index - back].joined {
                back += 1;
            }
            // A cluster wider than the whole line has nowhere better to go, so
            // it is left to break where it falls rather than looping.
            if back > 0 && back < fragments.len() {
                let keep = fragments.len() - back;
                // The pens as they stood before the first unit of the cluster
                // was placed: that unit's own `used` is the line's new end.
                used = rewind[keep].1;
                fragments.truncate(keep);
                rewind.truncate(keep);
                index -= back;
            }
            lines.push(raw_line(std::mem::take(&mut fragments), used, None));
            rewind.clear();
            pen = 0.0;
            used = 0.0;
            is_first = false;
            continue;
        }
        if !fits && fragments.is_empty() {
            // One unit wider than the whole line. Word breaks it at the margin
            // rather than letting it run off the page, and so does this: an
            // unbroken 200-character token is a URL or a hash, and a line that
            // reaches into the next column is worse than an ugly break.
            if let Some((head, tail)) = split_to_fit(unit, limit) {
                fragments.push(fragment_of(&head, pen));
                lines.push(raw_line(std::mem::take(&mut fragments), head.width, None));
                let mut rest = units[index + 1..].to_vec();
                rest.insert(0, tail);
                return continue_fill(lines, &rest, width, start, end, tabs, ctx, beside);
            }
        }

        rewind.push((pen, used));
        fragments.push(fragment_of(unit, pen));
        used = pen + unit.width;
        pen = used + unit.trailing;
        index += 1;
    }
    // The final line — except after a trailing page or column break, where
    // the paragraph mark rides the line the break ended. Word starts the new
    // page with the *next* paragraph, not with this one's empty remainder; an
    // extra line here pushed everything on the new page down by one and moved
    // a page break the author placed deliberately. A trailing *line* break is
    // different: Shift+Enter at the end of a paragraph genuinely opens an
    // empty line below itself.
    let mark_rides_the_break = fragments.is_empty()
        && matches!(
            lines.last().and_then(|line| line.ended_by),
            Some(Break::Page | Break::Column)
        );
    if !mark_rides_the_break {
        lines.push(raw_line(fragments, used, None));
    }
    lines
}

/// Restarts filling with the tail of a force-broken unit at the front.
#[allow(clippy::too_many_arguments)]
fn continue_fill(
    mut lines: Vec<Line>,
    units: &[Unit],
    width: f64,
    start: f64,
    end: f64,
    tabs: &[TabStop],
    ctx: &Context<'_>,
    beside: &[(f64, f64)],
) -> Vec<Line> {
    // Every line after the first uses the non-first-line indent, which is what
    // passing `0.0` for the first-line offset says.
    let rest = fill(units, width, start, end, 0.0, tabs, ctx, beside);
    lines.extend(rest);
    lines
}

fn fragment_of(unit: &Unit, x: f64) -> Fragment {
    let content = match &unit.kind {
        UnitKind::Text {
            text,
            advances,
            hyphen,
        } => Content::Text {
            text: text.clone(),
            advances: advances.clone(),
            hyphen: *hyphen,
        },
        UnitKind::Tab { leader, .. } => Content::Tab {
            leader: *leader,
            fill: String::new(),
            advances: Vec::new(),
        },
        UnitKind::Object {
            height,
            rel,
            chart,
            nth,
        } => Content::Object {
            height: *height,
            rel: rel.clone(),
            chart: chart.clone(),
            nth: *nth,
        },
        UnitKind::Label { text, advances } => Content::Label {
            text: text.clone(),
            advances: advances.clone(),
        },
        UnitKind::Break(_) => Content::Text {
            text: String::new(),
            advances: Vec::new(),
            hyphen: false,
        },
    };
    Fragment {
        x,
        width: unit.width + unit.trailing,
        lead: unit.lead,
        style: unit.style.clone(),
        content,
        source: unit.source,
        field: unit.field,
    }
}

/// Cuts a too-wide text unit at the last character that fits.
fn split_to_fit(unit: &Unit, limit: f64) -> Option<(Unit, Unit)> {
    let UnitKind::Text {
        text,
        advances,
        hyphen,
    } = &unit.kind
    else {
        return None;
    };
    let mut used = 0.0;
    let mut chars = 0usize;
    for advance in advances {
        if used + advance > limit && chars > 0 {
            break;
        }
        used += advance;
        chars += 1;
    }
    if chars == 0 || chars >= advances.len() {
        return None;
    }
    let split = text
        .char_indices()
        .nth(chars)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len());
    let head = Unit {
        style: unit.style.clone(),
        source: unit.source,
        field: unit.field,
        kind: UnitKind::Text {
            text: text[..split].to_string(),
            advances: advances[..chars].to_vec(),
            hyphen: false,
        },
        width: used,
        trailing: 0.0,
        joined: false,
        lead: 0.0,
    };
    let tail_width: f64 = advances[chars..].iter().sum();
    let tail = Unit {
        style: unit.style.clone(),
        // Clamped, because a unit whose text only stands *for* its piece — a
        // field's value, a tracked deletion — has fewer bytes in the paragraph
        // than on the line, and the tail of one must not start past its end.
        source: unit.source.map(|s| Source {
            start: (s.start + split).min(s.end),
            ..s
        }),
        field: unit.field,
        kind: UnitKind::Text {
            text: text[split..].to_string(),
            advances: advances[chars..].to_vec(),
            hyphen: *hyphen,
        },
        width: tail_width - unit.trailing,
        trailing: unit.trailing,
        joined: false,
        lead: 0.0,
    };
    Some((head, tail))
}

fn raw_line(fragments: Vec<Fragment>, width: f64, ended_by: Option<Break>) -> Line {
    Line {
        fragments,
        y: 0.0,
        baseline: 0.0,
        height: 0.0,
        ascent: 0.0,
        descent: 0.0,
        x: 0.0,
        width,
        ideal: 0.0,
        ended_by,
    }
}

/// Metrics, stacking, alignment and justification.
#[allow(clippy::too_many_arguments)]
fn finish(
    lines: &mut [Line],
    layers: &Layers,
    width: f64,
    start: f64,
    end: f64,
    first: f64,
    beside: &[(f64, f64)],
    paragraph: &Paragraph,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
) {
    // An empty paragraph still occupies a line, and its height comes from the
    // paragraph *mark* — a real character with a real size, which is why a blank
    // line after a heading is as tall as the heading.
    let mark = mark_metrics(paragraph, layers, ctx, shaper);
    let justify = layers.para.justify.unwrap_or_default();
    let spacing = layers.para.spacing.line.unwrap_or_default();
    let last = lines.len().saturating_sub(1);
    let mut y = 0.0;

    let mark_face = mark_font(paragraph, layers, ctx);
    for (index, line) in lines.iter_mut().enumerate() {
        let mut ascent: f64 = 0.0;
        let mut descent: f64 = 0.0;
        // The tallest face on the line decides the gap, as it decides the
        // ascent: a line of mixed sizes is spaced for the largest type on it.
        let mut gap: f64 = 0.0;
        // Room a run border demands above and below the type.
        let mut pad: f64 = 0.0;
        // Word lays a line at the font's *laid* pitch — hinted, a hair off the
        // design height — while an accumulator counts the exact value. Both
        // are collected here; the debt is settled where lines stack, because
        // it runs across paragraphs.
        let mut base: f64 = 0.0;
        let mut ideal: f64 = 0.0;
        // Something that is not type demanding height: an inline picture, a
        // superscript pushed above the ascender. Only these take a line out
        // of the dance — a face whose *measured* box is a hair taller than
        // its design height (they all are; hinting rounds up) must not.
        let mut boost: f64 = 0.0;
        // Inline pictures apart from raised type: a spacing multiple scales
        // type but never a picture, so the two must be told apart below.
        let mut object: f64 = 0.0;
        for fragment in &line.fragments {
            let metrics = fragment_metrics(fragment, shaper);
            ascent = ascent.max(metrics.ascent + fragment.style.raise);
            descent = descent.max(metrics.descent - fragment.style.raise);
            gap = gap.max(metrics.line_gap);
            // A run's border grows the line above and below by the same
            // amount it takes beside the type. Kept apart from the type's own
            // metrics because the line *pitch* is a property of the face and
            // must not change: the border adds to the height the pitch gave.
            pad = pad.max(fragment.style.border_pad());
            if let Content::Object { height, .. } = &fragment.content {
                object = object.max(*height);
                // The picture sits on the baseline, and the type of the run
                // holding it still hangs below — measured: Word lays a
                // 162.15pt picture in a 12pt run on a 164.74pt line, the
                // difference being that face's descent.
                descent = descent.max(shaper.metrics(&fragment.style.font).descent);
            } else {
                let pitch = shaper.pitch(&fragment.style.font);
                base = base.max(pitch.base);
                ideal = ideal.max(pitch.ideal);
                if fragment.style.raise != 0.0 {
                    boost = boost.max(
                        metrics.ascent + fragment.style.raise + metrics.descent
                            - fragment.style.raise.min(0.0) * 2.0,
                    );
                }
            }
        }
        if line.fragments.is_empty() {
            ascent = mark.ascent;
            descent = mark.descent;
            gap = mark.line_gap;
            let pitch = shaper.pitch(&mark_face);
            base = pitch.base;
            ideal = pitch.ideal;
        }
        // The gap counts toward the line's natural height, but not toward the
        // room above the baseline that the *type* occupies.
        let natural = ascent + descent + gap;
        if boost > ideal {
            base = boost;
            ideal = boost;
        }
        line.ascent = ascent;
        line.descent = descent;
        line.height = match spacing {
            // The multiple scales the type; a line whose height is really an
            // inline picture is laid at the picture's natural extent instead
            // (measured: a 1.2-spaced paragraph holds its 162.15pt picture on
            // a 164.74pt line, not a 194.6pt one).
            LineSpacing::Multiple(n) => (base * n.multiple()).max(natural.min(object + descent)),
            LineSpacing::AtLeast(t) => natural.max(t.points()),
            // `exact` clips a tall glyph rather than growing the line, which is
            // what makes a document with a pasted large font lose the tops of
            // its letters. Matching that is the point.
            LineSpacing::Exact(t) => t.points(),
        };
        line.ideal = match spacing {
            LineSpacing::Multiple(n) => (ideal * n.multiple()).max(natural.min(object + descent)),
            _ => line.height,
        };
        // Word seats the baseline at the face's ascent plus its line gap and
        // leaves it there: measured over Arial, Verdana and Georgia at line
        // multiples of 1.0, 1.15, 1.5 and 2.0, the first baseline of a spaced
        // paragraph never moved — every extra point of leading went *below*
        // the type, not around it.
        line.height += 2.0 * pad;
        line.ideal += 2.0 * pad;
        line.baseline = match spacing {
            // An exact line is a box the type is dropped into rather than one
            // the type decides, and Word stands the type on the bottom of it:
            // a drop cap set on an exact line three body lines tall has its
            // baseline on the third of them, which is the whole effect.
            LineSpacing::Exact(_) => (line.height - descent - pad).max(0.0),
            _ => ascent + gap + pad,
        };
        line.y = y;
        y += line.height;

        let is_first = index == 0;
        let (aside, inset) = beside.get(index).copied().unwrap_or((0.0, 0.0));
        let left = aside + if is_first { start + first } else { start };
        let limit = (width - end - left - inset).max(0.0);
        let slack = (limit - line.width).max(0.0);
        // The last line of a justified paragraph is not stretched, and neither
        // is one ended by an explicit break. `distribute` stretches both, which
        // is the whole difference between it and `both`.
        let is_last = index == last;
        let stretch = match justify {
            Justify::Both => !is_last && line.ended_by.is_none(),
            Justify::Distribute => true,
            _ => false,
        };

        line.x = left
            + match justify {
                Justify::Center => slack / 2.0,
                Justify::End => slack,
                _ => 0.0,
            };

        if stretch && line.fragments.len() > 1 && slack > 0.0 {
            let gaps = (line.fragments.len() - 1) as f64;
            let extra = slack / gaps;
            for (position, fragment) in line.fragments.iter_mut().enumerate() {
                fragment.x += extra * position as f64;
            }
            line.width = limit;
        }
    }
}

fn fragment_metrics(fragment: &Fragment, shaper: &mut dyn Shaper) -> Metrics {
    match &fragment.content {
        // An inline object's height is all above the baseline, which is what
        // makes a line holding a picture as tall as the picture.
        Content::Object { height, .. } => Metrics {
            ascent: *height,
            descent: 0.0,
            line_gap: 0.0,
        },
        _ => shaper.metrics(&fragment.style.font),
    }
}

fn mark_metrics(
    paragraph: &Paragraph,
    layers: &Layers,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
) -> Metrics {
    let mut props = layers.run.clone();
    if let Some(mark) = &paragraph.props.mark {
        props.layer(mark, wp_model::prop::Layer::Direct);
    }
    let style = style_for(' ', &props, ctx);
    shaper.metrics(&style.font)
}

/// The font a paragraph mark is drawn in, for a caret on an empty line.
pub fn mark_font(paragraph: &Paragraph, layers: &Layers, ctx: &Context<'_>) -> FontRequest {
    let mut props = layers.run.clone();
    if let Some(mark) = &paragraph.props.mark {
        props.layer(mark, wp_model::prop::Layer::Direct);
    }
    style_for(' ', &props, ctx).font
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::color::Theme;
    use wp_model::prop::{Indent, ParaProps, Spacing};
    use wp_model::units::{HalfPoint, Line240};

    fn theme() -> Theme {
        Theme::default()
    }

    fn ctx<'a>(theme: &'a Theme) -> Context<'a> {
        Context {
            theme,
            table_part: None,
            styles: Box::leak(Box::new(wp_model::style::StyleTable::default())),
            notes: Box::leak(Box::new(crate::notes::NoteMarks::default())),
            note_mark: None,
            default_tab: Twips(720),
            fallback_font: "test",
            has_face: |_| false,
            show_revisions: true,
            show_hidden: false,
            fields: Box::leak(Box::new(crate::field::FieldValues::default())),
            band: None,
            wraps: Box::leak(Box::new(crate::block::Wraps::default())),
        }
    }

    /// 10pt text through the fixed shaper: every character is exactly 5 points.
    fn layers() -> Layers {
        Layers {
            para: ParaProps::default(),
            run: RunProps {
                size: Some(HalfPoint(20)),
                ..RunProps::default()
            },
        }
    }

    fn lay(text: &str, width: f64) -> LaidParagraph {
        lay_with(Paragraph::of(text), layers(), width)
    }

    fn lay_with(paragraph: Paragraph, layers: Layers, width: f64) -> LaidParagraph {
        let theme = theme();
        let ctx = ctx(&theme);
        let mut shaper = crate::shape::Fixed;
        layout(&paragraph, 0, &layers, None, &ctx, width, None, &mut shaper)
    }

    fn texts(line: &Line) -> Vec<&str> {
        line.fragments
            .iter()
            .filter_map(|fragment| match &fragment.content {
                Content::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_paragraph_that_fits_is_one_line() {
        let laid = lay("hello world", 200.0);
        assert_eq!(laid.lines.len(), 1);
        assert_eq!(texts(&laid.lines[0]), ["hello ", "world"]);
        // Eleven characters at five points each.
        assert_eq!(laid.lines[0].width, 55.0);
    }

    #[test]
    fn a_line_breaks_at_the_last_word_that_fits() {
        // 30 points is six characters. "hello " is six and its trailing space
        // hangs, so the first line is "hello" and "world" goes below.
        let laid = lay("hello world", 30.0);
        assert_eq!(laid.lines.len(), 2);
        assert_eq!(texts(&laid.lines[0]), ["hello "]);
        assert_eq!(texts(&laid.lines[1]), ["world"]);
    }

    #[test]
    fn a_trailing_space_hangs_rather_than_pushing_the_word_down() {
        // "ab cd" is five characters — 25 points. At exactly 20 points the
        // space must hang or "cd" would be pushed to a second line while a
        // fifth of the line sat empty.
        let laid = lay("ab cd", 20.0);
        assert_eq!(laid.lines.len(), 2);
        assert_eq!(laid.lines[0].width, 10.0, "the space is not counted");
    }

    #[test]
    fn an_unbreakable_token_wider_than_the_line_is_broken_at_the_margin() {
        // Twenty characters, room for four. Left alone it would run off the
        // page and into the next column.
        let laid = lay("aaaaaaaaaaaaaaaaaaaa", 20.0);
        assert!(laid.lines.len() >= 5, "{:?}", laid.lines.len());
        for line in &laid.lines {
            assert!(line.width <= 20.01, "{} is past the margin", line.width);
        }
        let joined: String = laid
            .lines
            .iter()
            .flat_map(|line| texts(line))
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(joined, "aaaaaaaaaaaaaaaaaaaa", "nothing was lost");
    }

    #[test]
    fn an_explicit_break_ends_the_line_wherever_it_falls() {
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("one".into()),
                    Piece::Break(Break::Line),
                    Piece::Text("two".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let laid = lay_with(paragraph, layers(), 500.0);
        assert_eq!(laid.lines.len(), 2);
        assert_eq!(laid.lines[0].ended_by, Some(Break::Line));
        assert_eq!(texts(&laid.lines[1]), ["two"]);
    }

    #[test]
    fn a_first_line_indent_moves_only_the_first_line() {
        let mut layers = layers();
        layers.para.indent = Indent {
            start: Some(Twips(720)),
            first_line: Some(Twips(720)),
            ..Indent::default()
        };
        let laid = lay_with(Paragraph::of("aaa bbb ccc ddd"), layers, 100.0);
        assert!(laid.lines.len() >= 2);
        assert_eq!(laid.lines[0].x, 72.0, "half an inch plus half an inch");
        assert_eq!(laid.lines[1].x, 36.0);
    }

    #[test]
    fn a_hanging_indent_pulls_the_first_line_out() {
        let mut layers = layers();
        layers.para.indent = Indent {
            start: Some(Twips(720)),
            hanging: Some(Twips(360)),
            ..Indent::default()
        };
        let laid = lay_with(Paragraph::of("aaa bbb ccc ddd"), layers, 70.0);
        assert!(laid.lines.len() >= 2);
        assert_eq!(
            laid.lines[0].x, 18.0,
            "the first line hangs out to the left"
        );
        assert_eq!(laid.lines[1].x, 36.0);
    }

    #[test]
    fn centring_and_right_alignment_move_the_line_rather_than_the_text() {
        let mut layers = layers();
        layers.para.justify = Some(Justify::Center);
        let laid = lay_with(Paragraph::of("abcd"), layers.clone(), 100.0);
        assert_eq!(laid.lines[0].x, 40.0);

        layers.para.justify = Some(Justify::End);
        let laid = lay_with(Paragraph::of("abcd"), layers, 100.0);
        assert_eq!(laid.lines[0].x, 80.0);
    }

    #[test]
    fn justification_stretches_every_line_but_the_last() {
        let mut layers = layers();
        layers.para.justify = Some(Justify::Both);
        let laid = lay_with(Paragraph::of("aa bb cc dd ee ff gg"), layers, 40.0);
        assert!(laid.lines.len() >= 2);
        for line in &laid.lines[..laid.lines.len() - 1] {
            assert!(
                (line.width - 40.0).abs() < 0.01,
                "a justified line fills the column: {}",
                line.width
            );
        }
        let last = laid.lines.last().unwrap();
        assert!(
            last.width < 40.0,
            "the last line is left alone: {}",
            last.width
        );
    }

    #[test]
    fn line_spacing_multiplies_the_natural_height() {
        let mut layers = layers();
        let single = lay_with(Paragraph::of("x"), layers.clone(), 200.0).lines[0].height;
        layers.para.spacing = Spacing {
            line: Some(LineSpacing::Multiple(Line240::DOUBLE)),
            ..Spacing::default()
        };
        let double = lay_with(Paragraph::of("x"), layers.clone(), 200.0).lines[0].height;
        assert_eq!(double, single * 2.0);

        layers.para.spacing.line = Some(LineSpacing::Exact(Twips(120)));
        let exact = lay_with(Paragraph::of("x"), layers.clone(), 200.0).lines[0].height;
        assert_eq!(exact, 6.0, "an exact rule clips rather than growing");

        layers.para.spacing.line = Some(LineSpacing::AtLeast(Twips(600)));
        let at_least = lay_with(Paragraph::of("x"), layers, 200.0).lines[0].height;
        assert_eq!(at_least, 30.0);
    }

    #[test]
    fn an_empty_paragraph_is_a_line_as_tall_as_its_paragraph_mark() {
        // A blank line after a heading is as tall as the heading, because the
        // paragraph mark is a real character with a real size.
        let mut paragraph = Paragraph::new();
        paragraph.props.mark = Some(Box::new(RunProps {
            size: Some(HalfPoint(48)),
            ..RunProps::default()
        }));
        let laid = lay_with(paragraph, layers(), 200.0);
        assert_eq!(laid.lines.len(), 1);
        assert_eq!(laid.lines[0].height, 24.0);

        let plain = lay_with(Paragraph::new(), layers(), 200.0);
        assert_eq!(plain.lines[0].height, 10.0);
    }

    #[test]
    fn a_tab_advances_to_the_next_stop_and_then_to_the_default_interval() {
        let mut layers = layers();
        layers.para.tabs = Some(vec![TabStop {
            position: Twips(1440),
            kind: TabKind::Start,
            leader: TabLeader::Dot,
        }]);
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("a".into()),
                    Piece::Tab,
                    Piece::Text("b".into()),
                    Piece::Tab,
                    Piece::Text("c".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let laid = lay_with(paragraph, layers, 500.0);
        let line = &laid.lines[0];
        let after_first_tab = line.fragments[2].x;
        assert_eq!(after_first_tab, 72.0, "the explicit stop, at one inch");
        // Past the last explicit stop, the document default takes over: half an
        // inch intervals from the left of the column.
        let after_second_tab = line.fragments[4].x;
        assert_eq!(after_second_tab, 108.0);
        assert!(matches!(
            line.fragments[1].content,
            Content::Tab {
                leader: TabLeader::Dot,
                ..
            }
        ));
    }

    #[test]
    fn a_centre_tab_lays_the_text_that_follows_it_around_the_stop() {
        // The footer's page number: a name on the left, the number centred on a
        // stop. A left tab would begin the number at the stop; a centre tab puts
        // the stop through its middle.
        let mut layers = layers();
        layers.para.tabs = Some(vec![TabStop {
            position: Twips(1440),
            kind: TabKind::Center,
            leader: TabLeader::None,
        }]);
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("a".into()),
                    Piece::Tab,
                    Piece::Text("bbbb".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let laid = lay_with(paragraph, layers, 500.0);
        let text = &laid.lines[0].fragments[2];
        // "bbbb" is twenty points; centred on the inch its left edge is ten to
        // the left of it and its middle is on it.
        assert_eq!(text.x, 62.0);
        assert_eq!(text.x + text.width / 2.0, 72.0);
    }

    #[test]
    fn a_right_tab_lays_the_text_that_follows_it_up_to_the_stop() {
        let mut layers = layers();
        layers.para.tabs = Some(vec![TabStop {
            position: Twips(1440),
            kind: TabKind::End,
            leader: TabLeader::None,
        }]);
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("a".into()),
                    Piece::Tab,
                    Piece::Text("bbbb".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let laid = lay_with(paragraph, layers, 500.0);
        let text = &laid.lines[0].fragments[2];
        // Its right edge lands on the stop, so it begins its own width before it.
        assert_eq!(text.x + text.width, 72.0);
    }

    #[test]
    fn a_field_instruction_is_never_drawn_and_its_result_is() {
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::FieldStart {
                        dirty: false,
                        lock: false,
                    },
                    Piece::Instruction(" PAGE \\* MERGEFORMAT ".into()),
                    Piece::FieldSeparate,
                    Piece::Text("7".into()),
                    Piece::FieldEnd,
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let laid = lay_with(paragraph, layers(), 500.0);
        assert_eq!(texts(&laid.lines[0]), ["7"]);
    }

    #[test]
    fn a_tracked_deletion_is_drawn_when_revisions_show_and_not_when_they_do_not() {
        let paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("kept ")),
                Inline::Revised {
                    revision: wp_model::Revision::Deleted(wp_model::Mark::new(1, "A")),
                    content: vec![Inline::Run(Run {
                        content: vec![Piece::Deleted("gone".into())],
                        ..Run::new()
                    })],
                },
            ],
            ..Paragraph::new()
        };
        let theme = theme();
        let mut shaper = crate::shape::Fixed;

        let showing = layout(
            &paragraph,
            0,
            &layers(),
            None,
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        assert_eq!(texts(&showing.lines[0]), ["kept ", "gone"]);

        let hiding = Context {
            show_revisions: false,
            ..ctx(&theme)
        };
        let hidden = layout(
            &paragraph,
            0,
            &layers(),
            None,
            &hiding,
            500.0,
            None,
            &mut shaper,
        );
        assert_eq!(texts(&hidden.lines[0]), ["kept "]);
    }

    #[test]
    fn hidden_text_is_drawn_only_when_the_marks_are_showing() {
        let mut run = Run::of("secret");
        run.props.toggles.set(wp_model::Toggle::Vanish, true);
        let paragraph = Paragraph {
            content: vec![Inline::Run(run)],
            ..Paragraph::new()
        };
        let theme = theme();
        let mut shaper = crate::shape::Fixed;
        let hidden = layout(
            &paragraph,
            0,
            &layers(),
            None,
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        assert!(texts(&hidden.lines[0]).is_empty());

        let showing = Context {
            show_hidden: true,
            ..ctx(&theme)
        };
        let shown = layout(
            &paragraph,
            0,
            &layers(),
            None,
            &showing,
            500.0,
            None,
            &mut shaper,
        );
        assert_eq!(texts(&shown.lines[0]), ["secret"]);
    }

    #[test]
    fn a_level_that_names_a_picture_draws_it_where_the_bullet_would_go() {
        // Word leaves a Symbol dot in `<w:lvlText>` for a reader that cannot
        // fetch the image; drawing that dot is what left the demonstration
        // document saying "this bullet uses an image" beside a bullet that
        // plainly was not one. The picture goes at the size the numbering
        // states, not the image's own.
        let label = ListLabel {
            text: "\u{F0B7}".to_string(),
            props: RunProps::default(),
            suffix: Suffix::Tab,
            bullet: true,
            picture: Some(wp_model::numbering::PictureBullet {
                rel: "numbering:rId1".into(),
                width: 9.0,
                height: 9.0,
            }),
        };
        let theme = theme();
        let mut shaper = crate::shape::Fixed;
        let laid = layout(
            &Paragraph::of("item"),
            0,
            &layers(),
            Some(&label),
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        let first = &laid.lines[0].fragments[0];
        let Content::Object {
            height, rel, nth, ..
        } = &first.content
        else {
            panic!("the first fragment is the picture, not {:?}", first.content);
        };
        assert_eq!(*height, 9.0);
        assert_eq!(first.width, 9.0);
        assert_eq!(rel.as_deref(), Some("numbering:rId1"));
        assert_eq!(
            *nth, None,
            "a bullet is not one of the paragraph's drawings: nothing to pick"
        );
    }

    #[test]
    fn a_list_label_is_laid_out_before_the_text_in_its_own_font() {
        let label = ListLabel {
            text: "\u{F0B7}".to_string(),
            props: RunProps {
                fonts: wp_model::Fonts {
                    ascii: Some("Symbol".into()),
                    ..wp_model::Fonts::default()
                },
                ..RunProps::default()
            },
            suffix: Suffix::Tab,
            bullet: true,
            picture: None,
        };
        let theme = theme();
        let mut shaper = crate::shape::Fixed;
        let laid = layout(
            &Paragraph::of("item"),
            0,
            &layers(),
            Some(&label),
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        let line = &laid.lines[0];
        let Content::Label { text, .. } = &line.fragments[0].content else {
            panic!("the first fragment is the label");
        };
        // The label carries its own text. It is not in the document's runs, so
        // a renderer that is handed only the paragraph has nothing to draw.
        assert_eq!(text, "\u{F0B7}");
        assert_eq!(line.fragments[0].style.font.family.as_ref(), "Symbol");
        assert!(
            matches!(line.fragments[1].content, Content::Tab { .. }),
            "the suffix is a tab and lands on a stop"
        );
        assert_eq!(texts(line), ["item"]);
    }

    #[test]
    fn a_label_takes_the_marks_bold_but_never_its_underline() {
        // Measured against Word on probes: a paragraph mark carrying
        // `<w:b/><w:u/>` (LibreOffice copies the run's formatting onto the
        // mark) draws the *number* bold but not underlined, and the tab after
        // it stays bare too. Underline and strikethrough simply never reach a
        // list label; everything else layers as usual — for a number. A
        // *bullet* takes none of it: Word draws the same dot beside a bold
        // item and a plain one, and the same dash bullet either way, so only
        // the mark's size follows through.
        let label = ListLabel {
            text: "1.".to_string(),
            props: RunProps::default(),
            suffix: Suffix::Tab,
            bullet: false,
            picture: None,
        };
        let mut paragraph = Paragraph::of("item");
        let mut mark = RunProps::default();
        mark.toggles.set(wp_model::Toggle::Bold, true);
        mark.underline = Some(wp_model::prop::Underline {
            kind: wp_model::prop::UnderlineKind::Single,
            color: None,
        });
        paragraph.props.mark = Some(Box::new(mark));
        let theme = theme();
        let mut shaper = crate::shape::Fixed;
        let laid = layout(
            &paragraph,
            0,
            &layers(),
            Some(&label),
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        let line = &laid.lines[0];
        let fragment = &line.fragments[0];
        assert!(
            matches!(&fragment.content, Content::Label { .. }),
            "the first fragment is the label"
        );
        assert!(fragment.style.font.bold, "the mark's bold applies");
        assert_eq!(
            fragment.style.underline,
            wp_model::prop::UnderlineKind::None,
            "the mark's underline does not"
        );
        assert_eq!(
            line.fragments[1].style.underline,
            wp_model::prop::UnderlineKind::None,
            "nor does it reach the tab after the label"
        );

        // The same paragraph with a bullet: the mark's bold stays out.
        let bullet = ListLabel {
            text: "\u{2022}".to_string(),
            props: RunProps::default(),
            suffix: Suffix::Tab,
            bullet: true,
            picture: None,
        };
        let laid = layout(
            &paragraph,
            0,
            &layers(),
            Some(&bullet),
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        let fragment = &laid.lines[0].fragments[0];
        assert!(
            !fragment.style.font.bold,
            "a bullet does not go bold with its paragraph"
        );
    }

    #[test]
    fn the_tab_after_a_list_label_stops_at_the_paragraphs_own_indent() {
        // Word sends it to the hanging position rather than to the next default
        // stop, which is what lines a bullet's first line up with the wrapped
        // lines under it. Half an inch of white between every bullet and its
        // text is what the other reading looks like.
        let label = ListLabel {
            text: "\u{2022}".to_string(),
            props: RunProps::default(),
            suffix: Suffix::Tab,
            bullet: true,
            picture: None,
        };
        let mut layers = layers();
        layers.para.indent = Indent {
            start: Some(Twips(240)),
            hanging: Some(Twips(240)),
            ..Indent::default()
        };
        let theme = theme();
        let mut shaper = crate::shape::Fixed;
        let laid = layout(
            &Paragraph::of("item"),
            0,
            &layers,
            Some(&label),
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        let line = &laid.lines[0];
        let text = line
            .fragments
            .iter()
            .find(|f| matches!(&f.content, Content::Text { text, .. } if text == "item"))
            .expect("the text is on the line");
        // 240 twips is 12 points; the default tab stop is 36 and would be wrong.
        assert!(
            (text.x - 12.0).abs() < 0.01,
            "the text started at {} rather than at the indent",
            text.x
        );
    }

    #[test]
    fn a_field_that_cached_no_result_still_leaves_somewhere_to_put_one() {
        // Google Docs writes `{ PAGE }` as begin, instruction, separate, end,
        // with nothing between the separator and the end. Nothing is drawn, so
        // without a placeholder there is no fragment for the second pass to
        // find and the page number never appears.
        let run = Run {
            content: vec![
                Piece::FieldStart {
                    dirty: false,
                    lock: false,
                },
                Piece::Instruction(" PAGE ".into()),
                Piece::FieldSeparate,
                Piece::FieldEnd,
            ],
            ..Run::new()
        };
        let paragraph = Paragraph {
            content: vec![Inline::Run(run)],
            ..Paragraph::new()
        };
        let theme = theme();
        let mut shaper = crate::shape::Fixed;
        let laid = layout(
            &paragraph,
            3,
            &layers(),
            None,
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        let mark = laid.lines[0]
            .fragments
            .iter()
            .find_map(|f| f.field)
            .expect("the empty result is still marked as the field's");
        assert_eq!(mark.paragraph, 3);
        assert_eq!(mark.kind, wp_model::field::Kind::Page);
        // The placeholder is empty and has no width: it holds the mark and
        // draws nothing.
        assert_eq!(texts(&laid.lines[0]), [""]);
        assert_eq!(laid.lines[0].width, 0.0);

        // And once the value is known, it is drawn there.
        let mut values = FieldValues::new();
        values.set(mark, "7");
        let knowing = Context {
            fields: &values,
            ..ctx(&theme)
        };
        let again = layout(
            &paragraph,
            3,
            &layers(),
            None,
            &knowing,
            500.0,
            None,
            &mut shaper,
        );
        assert_eq!(texts(&again.lines[0]), ["7"]);
    }

    #[test]
    fn a_fragments_bytes_are_the_paragraphs_bytes_however_many_runs_it_has() {
        // `Source` used to name a range within the *piece* it came from. A
        // paragraph of one run cannot tell that apart from a range within the
        // paragraph, so it read as correct for as long as the tests were built
        // out of one-run paragraphs — and every run after the first handed the
        // editor an offset counted from the wrong place, which put the caret at
        // whatever byte of the first run happened to share the number.
        let paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("Hello ")),
                Inline::Run(Run::of("brave ")),
                Inline::Run(Run::of("world")),
            ],
            ..Paragraph::new()
        };
        let theme = theme();
        let mut shaper = crate::shape::Fixed;
        let laid = layout(
            &paragraph,
            0,
            &layers(),
            None,
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        let text = paragraph.text();
        for fragment in &laid.lines[0].fragments {
            let source = fragment.source.expect("text came from somewhere");
            let Content::Text { text: drawn, .. } = &fragment.content else {
                continue;
            };
            assert_eq!(
                &text[source.start..source.end],
                drawn.as_str(),
                "the fragment's range does not spell the fragment"
            );
        }
        let end = laid.lines[0]
            .fragments
            .iter()
            .filter_map(|f| f.source)
            .map(|s| s.end)
            .max();
        assert_eq!(end, Some(text.len()), "and they reach the end of the text");
    }

    #[test]
    fn a_tab_and_a_drawing_hold_the_place_they_occupy_in_the_text() {
        // A tab is a byte of the paragraph's text and carried no source at all,
        // so nothing on the line claimed it and a caret could not be put on
        // either side of it.
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("ab".into()),
                    Piece::Tab,
                    Piece::Text("cd".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let theme = theme();
        let mut shaper = crate::shape::Fixed;
        let laid = layout(
            &paragraph,
            0,
            &layers(),
            None,
            &ctx(&theme),
            500.0,
            None,
            &mut shaper,
        );
        let spans: Vec<(usize, usize)> = laid.lines[0]
            .fragments
            .iter()
            .filter_map(|f| f.source)
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(spans, [(0, 2), (2, 3), (3, 5)], "ab, the tab, cd");
    }

    #[test]
    fn a_run_mixing_scripts_is_split_so_each_gets_its_own_face() {
        // "Mixed مرحبا inline." is one `<w:t>`, and drawing it in one face is
        // how Arabic ends up in a Latin font.
        let mut run = Run::of("ab مر cd");
        run.props.fonts.ascii = Some("Latin".into());
        run.props.fonts.complex = Some("Arabic".into());
        let paragraph = Paragraph {
            content: vec![Inline::Run(run)],
            ..Paragraph::new()
        };
        let laid = lay_with(paragraph, layers(), 500.0);
        let families: Vec<&str> = laid.lines[0]
            .fragments
            .iter()
            .map(|f| f.style.font.family.as_ref())
            .collect();
        assert!(families.contains(&"Latin"), "{families:?}");
        assert!(families.contains(&"Arabic"), "{families:?}");
    }

    #[test]
    fn every_fragment_knows_where_it_came_from() {
        // Without this the caret cannot be placed. A click lands on a point of a
        // line and what has to come back is a position in the document.
        let laid = lay("hello world", 500.0);
        for fragment in &laid.lines[0].fragments {
            let source = fragment.source.expect("text came from somewhere");
            assert_eq!(source.run, 0);
            assert!(source.end > source.start);
        }
        let sources: Vec<_> = laid.lines[0]
            .fragments
            .iter()
            .filter_map(|f| f.source)
            .collect();
        assert_eq!(sources[0].start, 0);
        assert_eq!(sources[0].end, 6, "the word and its trailing space");
        assert_eq!(sources[1].start, 6);
    }

    #[test]
    fn an_inline_picture_makes_the_line_as_tall_as_itself() {
        let drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: false,
            extent: (
                wp_model::Emu::from_points(50.0),
                wp_model::Emu::from_points(40.0),
            ),
            rel: None,
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: false,
        };
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![Piece::Text("x".into()), Piece::Drawing(Box::new(drawing))],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let laid = lay_with(paragraph, layers(), 500.0);
        let line = &laid.lines[0];
        assert!(line.height >= 40.0, "{}", line.height);
        assert_eq!(line.fragments[1].width, 50.0);
    }
}
