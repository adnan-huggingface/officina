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
    /// Byte range within that piece's text.
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
    Tab { leader: TabLeader },
    /// An inline drawing, and the relationship naming the part that holds its
    /// bytes. Without the relationship the painter has a rectangle of the right
    /// size and nothing to put in it.
    Object {
        height: f64,
        rel: Option<std::sync::Arc<str>>,
        /// Which of the paragraph's drawings this is.
        nth: usize,
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
    /// Left edge, from the left of the *page's text column*.
    pub x: f64,
    pub width: f64,
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
}

/// What the layout needs beyond the paragraph itself.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    pub theme: &'a wp_model::color::Theme,
    /// `<w:defaultTabStop>` — where tabs land when the paragraph defines none.
    pub default_tab: Twips,
    /// The face to use when neither the run nor the theme names one.
    pub fallback_font: &'a str,
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
}

impl Default for Context<'_> {
    fn default() -> Self {
        Context {
            theme: Box::leak(Box::new(wp_model::color::Theme::default())),
            default_tab: Twips(720),
            fallback_font: "Calibri",
            show_revisions: true,
            show_hidden: false,
            fields: Box::leak(Box::new(FieldValues::default())),
            band: None,
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
        nth: usize,
    },
    Label {
        text: String,
        advances: Vec<f64>,
    },
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
    shaper: &mut dyn Shaper,
) -> LaidParagraph {
    let units = units(paragraph, index, layers, label, ctx, shaper);
    let indent = &layers.para.indent;
    let start = indent.start.map(|t| t.points()).unwrap_or(0.0);
    let end = indent.end.map(|t| t.points()).unwrap_or(0.0);
    let first = indent.first_line_offset().points();

    let tabs = tab_stops(layers, ctx);
    let mut lines = fill(&units, width, start, end, first, &tabs, ctx);
    finish(
        &mut lines, layers, width, start, end, first, paragraph, ctx, shaper,
    );

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
        // The label takes the level's own formatting laid over the paragraph's,
        // which is what keeps a bullet in Symbol while the text stays in Calibri.
        let mut props = layers.run.clone();
        props.layer(&label.props, wp_model::prop::Layer::Direct);
        let script = label
            .text
            .chars()
            .next()
            .map(resolve::face_for)
            .unwrap_or(wp_model::prop::Script::Ascii);
        let style = resolve::text_style(&props, ctx.theme, script, ctx.fallback_font);
        let mut advances = Vec::new();
        let width = measure(&label.text, &style, shaper, &mut advances);
        out.push(Unit {
            style,
            source: None,
            field: None,
            kind: UnitKind::Label {
                text: label.text.clone(),
                advances,
            },
            width,
            trailing: 0.0,
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
    for (run_index, run) in runs.iter().enumerate() {
        if deleted.contains(&run_index) && !ctx.show_revisions {
            continue;
        }
        push_run(run, run_index, layers, ctx, shaper, &mut fields, &mut out);
    }
    number_drawings(paragraph, &mut out);
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
        if let UnitKind::Object { nth, .. } = &mut unit.kind {
            *nth = next.next().copied().unwrap_or(0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_run(
    run: &Run,
    index: usize,
    layers: &Layers,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    fields: &mut FieldWalk,
    out: &mut Vec<Unit>,
) {
    // The run's own properties over the paragraph's — the last layer of the
    // resolution, done here because the character style chain was already
    // applied by the caller into `layers.run`.
    let mut props = layers.run.clone();
    props.layer(&run.props, wp_model::prop::Layer::Direct);
    if props.hidden() && !ctx.show_hidden {
        return;
    }

    // A field's instruction is never drawn; only its cached result is. Between
    // `begin` and `separate` everything is code, and a renderer that draws it
    // shows the user ` PAGE \* MERGEFORMAT ` in the middle of the sentence.
    for (piece_index, piece) in run.content.iter().enumerate() {
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
                                push_text(value, &props, index, piece_index, ctx, shaper, out);
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
                            }),
                        }
                    }
                }
                fields.end();
            }
            Piece::Instruction(text) => fields.instruction(text),
            Piece::DeletedInstruction(_) => {}
            _ if fields.in_instruction() => {}
            Piece::Text(text) | Piece::Deleted(text) => {
                if matches!(piece, Piece::Deleted(_)) && !ctx.show_revisions {
                    continue;
                }
                // A field whose value is known draws that instead of what the
                // file cached — and the *whole* result is replaced by the first
                // piece of it, because a cached `12` is very often a run `1` and
                // a run `2` and substituting into both would give `77`.
                let mark = fields.mark();
                match mark.and_then(|mark| ctx.fields.get(mark)) {
                    Some(value) if !already_drawn(out, mark) => {
                        push_text(value, &props, index, piece_index, ctx, shaper, out);
                        mark_last(out, mark);
                    }
                    Some(_) => {}
                    None => {
                        push_text(text, &props, index, piece_index, ctx, shaper, out);
                        mark_last(out, mark);
                    }
                }
            }
            Piece::Tab => {
                let style = style_for('\t', &props, ctx);
                let mut unit = tab_unit(&style, TabLeader::None, false);
                unit.field = fields.mark();
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
                });
            }
            Piece::Hyphen { breaking } => {
                // A non-breaking hyphen draws; a soft one draws only where the
                // line actually breaks, which `linebreak` decides.
                let text = if *breaking { "\u{00AD}" } else { "\u{2011}" };
                push_text(text, &props, index, piece_index, ctx, shaper, out);
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
                        start: 0,
                        end: 0,
                    }),
                    field: None,
                    kind: UnitKind::Object {
                        height: drawing.extent.1.points(),
                        rel: drawing.rel.clone(),
                        // Filled in once the whole paragraph is walked: a unit
                        // does not know how many drawings came before it.
                        nth: 0,
                    },
                    width: drawing.extent.0.points(),
                    trailing: 0.0,
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
fn push_text(
    text: &str,
    props: &RunProps,
    run: usize,
    piece: usize,
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

    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let slice = &text[start..end];
        if slice.is_empty() {
            continue;
        }
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

        out.push(Unit {
            style,
            source: Some(Source {
                run,
                piece,
                start,
                end,
            }),
            field: None,
            kind: UnitKind::Text {
                text: drawn.to_string(),
                advances,
                hyphen: linebreak::breaks_with_hyphen(text, end),
            },
            width: content_width,
            trailing: total - content_width,
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
) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut is_first = true;
    // Two pens, and the difference between them is the whole trailing-space
    // rule. `pen` is where the next unit starts; `used` is where the line's
    // *content* ends. A line's width is `used`, so the spaces at the end of it
    // hang past the margin instead of counting against it.
    let mut pen = 0.0f64;
    let mut used = 0.0f64;
    let available = |is_first: bool| {
        let left = if is_first { start + first } else { start };
        (width - end - left).max(1.0)
    };

    let mut index = 0usize;
    while index < units.len() {
        let unit = &units[index];
        let limit = available(is_first);
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
                let (stop, stop_leader) = if *after_label && start > here + 0.01 {
                    (start, TabLeader::None)
                } else {
                    let (at, _, leader) = next_tab(here, tabs, ctx.default_tab);
                    (at, leader)
                };
                let target = (stop - left).min(limit);
                let advance = (target - pen).max(0.0);
                fragments.push(Fragment {
                    x: pen,
                    width: advance,
                    style: unit.style.clone(),
                    content: Content::Tab {
                        leader: if *leader == TabLeader::None {
                            stop_leader
                        } else {
                            *leader
                        },
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
            lines.push(raw_line(std::mem::take(&mut fragments), used, None));
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
                return continue_fill(lines, &rest, width, start, end, tabs, ctx);
            }
        }

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
) -> Vec<Line> {
    // Every line after the first uses the non-first-line indent, which is what
    // passing `0.0` for the first-line offset says.
    let rest = fill(units, width, start, end, 0.0, tabs, ctx);
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
        UnitKind::Tab { leader, .. } => Content::Tab { leader: *leader },
        UnitKind::Object { height, rel, nth } => Content::Object {
            height: *height,
            rel: rel.clone(),
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
    };
    let tail_width: f64 = advances[chars..].iter().sum();
    let tail = Unit {
        style: unit.style.clone(),
        source: unit.source.map(|s| Source {
            start: s.start + split,
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
        for fragment in &line.fragments {
            let metrics = fragment_metrics(fragment, shaper);
            ascent = ascent.max(metrics.ascent + fragment.style.raise);
            descent = descent.max(metrics.descent - fragment.style.raise);
            if let Content::Object { height, .. } = &fragment.content {
                boost = boost.max(*height);
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
            let pitch = shaper.pitch(&mark_face);
            base = pitch.base;
            ideal = pitch.ideal;
        }
        let natural = ascent + descent;
        if boost > ideal {
            base = boost;
            ideal = boost;
        }
        line.ascent = ascent;
        line.descent = descent;
        line.height = match spacing {
            LineSpacing::Multiple(n) => base * n.multiple(),
            LineSpacing::AtLeast(t) => natural.max(t.points()),
            // `exact` clips a tall glyph rather than growing the line, which is
            // what makes a document with a pasted large font lose the tops of
            // its letters. Matching that is the point.
            LineSpacing::Exact(t) => t.points(),
        };
        line.ideal = match spacing {
            LineSpacing::Multiple(n) => ideal * n.multiple(),
            _ => line.height,
        };
        line.baseline = ascent + (line.height - natural).max(0.0) / 2.0;
        line.y = y;
        y += line.height;

        let is_first = index == 0;
        let left = if is_first { start + first } else { start };
        let limit = (width - end - left).max(0.0);
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
            default_tab: Twips(720),
            fallback_font: "test",
            show_revisions: true,
            show_hidden: false,
            fields: Box::leak(Box::new(crate::field::FieldValues::default())),
            band: None,
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
        layout(&paragraph, 0, &layers, None, &ctx, width, &mut shaper)
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
                leader: TabLeader::Dot
            }
        ));
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
            &mut shaper,
        );
        assert_eq!(texts(&showing.lines[0]), ["kept ", "gone"]);

        let hiding = Context {
            show_revisions: false,
            ..ctx(&theme)
        };
        let hidden = layout(&paragraph, 0, &layers(), None, &hiding, 500.0, &mut shaper);
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
            &mut shaper,
        );
        assert!(texts(&hidden.lines[0]).is_empty());

        let showing = Context {
            show_hidden: true,
            ..ctx(&theme)
        };
        let shown = layout(&paragraph, 0, &layers(), None, &showing, 500.0, &mut shaper);
        assert_eq!(texts(&shown.lines[0]), ["secret"]);
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
    fn the_tab_after_a_list_label_stops_at_the_paragraphs_own_indent() {
        // Word sends it to the hanging position rather than to the next default
        // stop, which is what lines a bullet's first line up with the wrapped
        // lines under it. Half an inch of white between every bullet and its
        // text is what the other reading looks like.
        let label = ListLabel {
            text: "\u{2022}".to_string(),
            props: RunProps::default(),
            suffix: Suffix::Tab,
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
        let again = layout(&paragraph, 3, &layers(), None, &knowing, 500.0, &mut shaper);
        assert_eq!(texts(&again.lines[0]), ["7"]);
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
