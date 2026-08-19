//! The formatted halves of a copy: CF_HTML and Rich Text Format.
//!
//! Word does not read a `.docx` off the clipboard. It reads *formats*, and the
//! two that carry formatting from a program that is not Word are `HTML Format`
//! — CF_HTML, the one a browser writes — and `Rich Text Format`. A copy that
//! puts only text on the board arrives in Word as Calibri 11 whatever it left
//! as, which is what these two are for.
//!
//! Both are written from the properties the screen resolves from — the style
//! chain, the numbering layer, the theme — so what Word receives is what Scriva
//! drew, not what the run happened to state directly. A word in a Heading 1 is
//! bold and sixteen point on the board even though its own `<w:rPr>` is empty.
//!
//! **A fragment of one paragraph is written without a paragraph around it.**
//! Half a sentence copied out of the middle of one belongs in the middle of
//! another, and a `<p>` — or a trailing `\par` — is what turns a phrase into a
//! paragraph of its own on arrival. Between paragraphs the break is written;
//! after the last one it is not, which is the rule Word's own copy follows.
//!
//! **Stated limits.** A hyperlink arrives as text in its own formatting: the
//! address is a relationship id, and resolving it needs the package the
//! paragraphs came out of rather than the paragraphs. Pictures, charts, tables
//! and footnotes are not carried — what the editor copies is paragraphs, and
//! these two write what it copies.

use std::fmt::Write as _;
use std::sync::Arc;

use wp_model::color::{Highlight, Theme};
use wp_model::doc::{Break, Document, Paragraph, Piece};
use wp_model::numbering::NumFormat;
use wp_model::prop::{Justify, LineSpacing, NumRef, RunProps, Script, Toggle, VertAlign};
use wp_model::style::Layers;
use wp_model::units::Twips;

/// One run's appearance, resolved: what both writers ask about.
#[derive(Debug, Clone, PartialEq)]
struct Look {
    family: Arc<str>,
    /// Points, because that is the unit both formats state a size in — HTML as
    /// `pt` and RTF as half-points.
    size: f64,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    caps: bool,
    small_caps: bool,
    color: Option<[u8; 3]>,
    highlight: Option<[u8; 3]>,
    shading: Option<[u8; 3]>,
    vert: VertAlign,
}

/// One paragraph's shape, resolved.
#[derive(Debug, Clone, PartialEq)]
struct Shape {
    align: Justify,
    before: Twips,
    after: Twips,
    left: Twips,
    right: Twips,
    /// Signed: a hanging indent is negative, which is how both formats say it.
    first: Twips,
    line: Option<LineSpacing>,
    shading: Option<[u8; 3]>,
    /// Which list the paragraph is in, and whether the level draws a bullet
    /// rather than a counter.
    list: Option<(NumRef, bool)>,
}

/// The whole of a copy as CF_HTML's fragment — what goes between the
/// `StartFragment` and `EndFragment` markers, which the clipboard adds.
pub fn html(document: &Document, paragraphs: &[Paragraph]) -> String {
    let mut out = String::new();
    // The lists standing open around the current paragraph, innermost last.
    let mut open: Vec<(NumRef, bool)> = Vec::new();
    let alone = paragraphs.len() == 1;

    for paragraph in paragraphs {
        let layers = resolve(document, paragraph);
        let shape = shape(document, paragraph, &layers);
        nest(&mut out, &mut open, shape.list);

        let tag = match (shape.list.is_some(), alone) {
            (true, _) => "li",
            (false, true) => "",
            (false, false) => "p",
        };
        if !tag.is_empty() {
            let _ = write!(out, "<{tag} style=\"{}\">", paragraph_css(&shape));
        }
        let mut space = true;
        for run in paragraph.runs() {
            let look = look(document, &layers, &run.props);
            let mut body = String::new();
            for piece in &run.content {
                piece_html(piece, &mut body, &mut space);
            }
            if !body.is_empty() {
                let _ = write!(out, "<span style=\"{}\">{}</span>", run_css(&look), body);
            }
        }
        if !tag.is_empty() {
            let _ = write!(out, "</{tag}>");
        }
    }

    nest(&mut out, &mut open, None);
    out
}

/// The whole of a copy as an RTF document.
pub fn rtf(document: &Document, paragraphs: &[Paragraph]) -> String {
    // The body first: it is what discovers the fonts and colours the header has
    // to declare, and a table cannot be written before it is known.
    let mut table = Table::default();
    let mut body = String::new();

    for (index, paragraph) in paragraphs.iter().enumerate() {
        if index > 0 {
            body.push_str("\\par\n");
        }
        let layers = resolve(document, paragraph);
        let shape = shape(document, paragraph, &layers);
        body.push_str("\\pard\\plain");
        paragraph_rtf(&shape, &mut table, &mut body);
        for run in paragraph.runs() {
            let look = look(document, &layers, &run.props);
            let mut text = String::new();
            for piece in &run.content {
                piece_rtf(piece, &mut table, &mut text);
            }
            if !text.is_empty() {
                body.push_str("{\\plain");
                run_rtf(&look, &mut table, &mut body);
                body.push(' ');
                body.push_str(&text);
                body.push('}');
            }
        }
        body.push('\n');
    }

    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\uc1\\deff0\n{\\fonttbl");
    for (index, family) in table.fonts.iter().enumerate() {
        let _ = write!(out, "{{\\f{index}\\fnil\\fcharset0 {family};}}");
    }
    out.push_str("}\n{\\colortbl;");
    for rgb in &table.colors {
        let _ = write!(
            out,
            "\\red{}\\green{}\\blue{};",
            rgb[0] as u32, rgb[1] as u32, rgb[2] as u32
        );
    }
    out.push_str("}\n");
    out.push_str(&body);
    out.push('}');
    out
}

// -------------------------------------------------------------- resolution

/// A paragraph's formatting with its style chain and its list level heard from.
fn resolve(document: &Document, paragraph: &Paragraph) -> Layers {
    let numbering = numbering_of(document, paragraph).and_then(|r| document.numbering.layers(r));
    document
        .styles
        .resolve_paragraph(&paragraph.props, numbering.as_ref())
}

/// Which list a paragraph is in — its own answer, or its style's.
///
/// A style may put a paragraph in a list without the paragraph saying so, which
/// is how every `ListParagraph` in a Word document works, and `w:numId="0"` on
/// the paragraph cancels it.
fn numbering_of(document: &Document, paragraph: &Paragraph) -> Option<NumRef> {
    if let Some(reference) = paragraph.props.numbering {
        return reference.is_numbered().then_some(reference);
    }
    let style = paragraph.props.style.or_else(|| {
        document
            .styles
            .default_style(wp_model::StyleKind::Paragraph)
    })?;
    document
        .styles
        .chain(style)
        .into_iter()
        .rev()
        .find_map(|step| document.styles.get(step)?.para.numbering)
        .filter(|reference| reference.is_numbered())
}

fn shape(document: &Document, paragraph: &Paragraph, layers: &Layers) -> Shape {
    let para = &layers.para;
    let list = numbering_of(document, paragraph).map(|reference| {
        let bullet = document
            .numbering
            .level(reference.num_id, reference.level)
            .is_some_and(|level| level.format == NumFormat::Bullet);
        (reference, bullet)
    });
    Shape {
        align: para.justify.unwrap_or_default(),
        before: para.spacing.before.unwrap_or(Twips(0)),
        after: para.spacing.after.unwrap_or(Twips(0)),
        left: para.indent.start.unwrap_or(Twips(0)),
        right: para.indent.end.unwrap_or(Twips(0)),
        first: para.indent.first_line_offset(),
        line: para.spacing.line,
        shading: para
            .shading
            .and_then(|s| s.background())
            .and_then(|c| c.resolve(&document.theme)),
        list,
    }
}

fn look(document: &Document, layers: &Layers, direct: &RunProps) -> Look {
    let props = document.styles.resolve_run(layers, direct);
    let theme: &Theme = &document.theme;
    Look {
        family: wp_layout::resolve::family(&props, theme, Script::Ascii, "Calibri"),
        size: props.font_size().points(),
        bold: props.bold(),
        italic: props.italic(),
        underline: props.underline.is_some_and(|u| u.kind.draws()),
        strike: props.toggles.is_on(Toggle::Strike) || props.toggles.is_on(Toggle::DoubleStrike),
        caps: props.toggles.is_on(Toggle::Caps),
        small_caps: props.toggles.is_on(Toggle::SmallCaps),
        color: props.color.and_then(|c| c.resolve(theme)),
        highlight: props.highlight.and_then(Highlight::rgb),
        shading: props
            .shading
            .and_then(|s| s.background())
            .and_then(|c| c.resolve(theme)),
        vert: props.vert_align.unwrap_or_default(),
    }
}

// -------------------------------------------------------------------- HTML

/// Opens and closes `<ul>`/`<ol>` so that the stack matches the paragraph about
/// to be written.
///
/// A list in HTML is a container and a list in Word is a property of each
/// paragraph, so the nesting has to be inferred: a deeper level opens, a
/// shallower one closes, and a different `numId` at the same depth closes and
/// opens again — two lists in a row are two lists, not one with a gap in it.
fn nest(out: &mut String, open: &mut Vec<(NumRef, bool)>, list: Option<(NumRef, bool)>) {
    let depth = list.map(|(r, _)| r.level as usize + 1).unwrap_or(0);
    while open.len() > depth
        || (open.len() == depth
            && depth > 0
            && open[depth - 1].0.num_id != list.expect("depth > 0").0.num_id)
    {
        let (_, bullet) = open.pop().expect("len > 0");
        out.push_str(match bullet {
            true => "</ul>",
            false => "</ol>",
        });
    }
    while open.len() < depth {
        let (reference, bullet) = list.expect("depth > 0");
        out.push_str(match bullet {
            true => "<ul>",
            false => "<ol>",
        });
        open.push((reference, bullet));
    }
}

fn paragraph_css(shape: &Shape) -> String {
    let mut css = format!(
        "margin-top:{};margin-bottom:{}",
        pt(shape.before),
        pt(shape.after)
    );
    // A list item's indent is the `<ul>`'s to give: stating it again here would
    // add Word's list indent to the one the browser already applied.
    if shape.list.is_none() {
        let _ = write!(
            css,
            ";margin-left:{};margin-right:{};text-indent:{}",
            pt(shape.left),
            pt(shape.right),
            pt(shape.first)
        );
    }
    let _ = write!(
        css,
        ";text-align:{}",
        match shape.align {
            Justify::Start => "left",
            Justify::Center => "center",
            Justify::End => "right",
            Justify::Both | Justify::Distribute => "justify",
        }
    );
    match shape.line {
        // `atLeast` and `exact` have no CSS between them; both come out as the
        // height they ask for, which is the one of the two meanings a browser
        // can keep.
        Some(LineSpacing::AtLeast(twips)) | Some(LineSpacing::Exact(twips)) => {
            let _ = write!(css, ";line-height:{}", pt(twips));
        }
        Some(LineSpacing::Multiple(line)) => {
            let _ = write!(css, ";line-height:{:.0}%", line.0 as f64 / 240.0 * 100.0);
        }
        None => {}
    }
    if let Some(rgb) = shape.shading {
        let _ = write!(css, ";background:{}", hex(rgb));
    }
    css
}

fn run_css(look: &Look) -> String {
    let stack: Vec<String> = faces(&look.family)
        .map(|name| format!("'{name}'"))
        .collect();
    let mut css = format!(
        "font-family:{};font-size:{:.1}pt",
        stack.join(","),
        look.size
    );
    if look.bold {
        css.push_str(";font-weight:bold");
    }
    if look.italic {
        css.push_str(";font-style:italic");
    }
    match (look.underline, look.strike) {
        (true, true) => css.push_str(";text-decoration:underline line-through"),
        (true, false) => css.push_str(";text-decoration:underline"),
        (false, true) => css.push_str(";text-decoration:line-through"),
        (false, false) => {}
    }
    if look.caps {
        css.push_str(";text-transform:uppercase");
    }
    if look.small_caps {
        css.push_str(";font-variant:small-caps");
    }
    match look.vert {
        VertAlign::Superscript => css.push_str(";vertical-align:super;font-size:smaller"),
        VertAlign::Subscript => css.push_str(";vertical-align:sub;font-size:smaller"),
        VertAlign::Baseline => {}
    }
    if let Some(rgb) = look.color {
        let _ = write!(css, ";color:{}", hex(rgb));
    }
    if let Some(rgb) = look.highlight.or(look.shading) {
        let _ = write!(css, ";background:{}", hex(rgb));
    }
    css
}

fn piece_html(piece: &Piece, out: &mut String, space: &mut bool) {
    match piece {
        Piece::Text(text) => escape(text, out, space),
        // A tab has no HTML. `mso-tab-count` is the property Word writes for one
        // and reads back as one; the em space beside it is for everything else,
        // which would otherwise show nothing at all.
        Piece::Tab => {
            out.push_str("<span style=\"mso-tab-count:1\">&emsp;</span>");
            *space = false;
        }
        Piece::Break(Break::Line) => {
            out.push_str("<br>");
            *space = true;
        }
        Piece::Hyphen { breaking: false } => {
            out.push_str("&#8209;");
            *space = false;
        }
        Piece::Hyphen { breaking: true } => {
            out.push_str("&shy;");
            *space = false;
        }
        // The font is half of a symbol's identity — `F0B7` is a bullet in Symbol
        // and nothing anywhere else — so it is named on the character itself.
        Piece::Symbol { font, ch } => {
            let _ = write!(
                out,
                "<span style=\"font-family:'{}'\">&#{};</span>",
                font, *ch as u32
            );
            *space = false;
        }
        _ => {}
    }
}

/// Writes text as HTML, with the runs of spaces kept.
///
/// HTML collapses whitespace: a browser given two spaces draws one, and a
/// sentence that lined its columns up with spaces arrives with the columns gone.
/// Word's own HTML writes `&nbsp;` for the second of every pair and so does
/// this.
fn escape(text: &str, out: &mut String, space: &mut bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            ' ' if *space => {
                out.push_str("&nbsp;");
                continue;
            }
            ' ' => {
                out.push(' ');
                *space = true;
                continue;
            }
            c => out.push(c),
        }
        *space = false;
    }
}

/// The faces a run's font names, in order of preference.
///
/// `w:ascii` is one face's name in the format's own account of itself, and it is
/// a *list* in a document LibreOffice wrote: `Liberation Sans;Arial`. The
/// separator it chose ends a declaration in CSS and ends an entry in an RTF font
/// table, so a name carrying one has to be split before either format sees it —
/// and both of them can say the whole list, which is more than the one name is.
fn faces(family: &str) -> impl Iterator<Item = &str> {
    family
        .split(';')
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

fn pt(twips: Twips) -> String {
    format!("{:.1}pt", twips.points())
}

// --------------------------------------------------------------------- RTF

/// The fonts and colours an RTF document has to declare before it uses them.
#[derive(Debug, Default)]
struct Table {
    fonts: Vec<String>,
    colors: Vec<[u8; 3]>,
}

impl Table {
    /// The entry for a font, made if it is the first time it is asked for.
    ///
    /// One face, not a list: an RTF font table entry ends at a semicolon, so a
    /// name that is really a fallback list is taken at its first choice.
    fn font(&mut self, family: &str) -> usize {
        let name = faces(family).next().unwrap_or("Calibri");
        match self.fonts.iter().position(|f| f == name) {
            Some(index) => index,
            None => {
                self.fonts.push(name.to_owned());
                self.fonts.len() - 1
            }
        }
    }

    /// The one-based index the colour table uses: entry zero is `auto`, which is
    /// the absence of a colour rather than one of them.
    fn color(&mut self, rgb: [u8; 3]) -> usize {
        match self.colors.iter().position(|c| *c == rgb) {
            Some(index) => index + 1,
            None => {
                self.colors.push(rgb);
                self.colors.len()
            }
        }
    }
}

fn paragraph_rtf(shape: &Shape, table: &mut Table, out: &mut String) {
    out.push_str(match shape.align {
        Justify::Start => "\\ql",
        Justify::Center => "\\qc",
        Justify::End => "\\qr",
        Justify::Both | Justify::Distribute => "\\qj",
    });
    let _ = write!(
        out,
        "\\sb{}\\sa{}\\li{}\\ri{}\\fi{}",
        shape.before.0, shape.after.0, shape.left.0, shape.right.0, shape.first.0
    );
    match shape.line {
        // A multiple is stated in the same 240ths RTF measures a single line in,
        // which is the one place the two formats agree exactly.
        Some(LineSpacing::Multiple(line)) => {
            let _ = write!(out, "\\sl{}\\slmult1", line.0);
        }
        Some(LineSpacing::AtLeast(twips)) => {
            let _ = write!(out, "\\sl{}\\slmult0", twips.0);
        }
        // A negative height is how RTF says `exact`: the line does not grow for
        // a tall glyph, it clips it.
        Some(LineSpacing::Exact(twips)) => {
            let _ = write!(out, "\\sl-{}\\slmult0", twips.0);
        }
        None => {}
    }
    if let Some(rgb) = shape.shading {
        let _ = write!(out, "\\cbpat{}", table.color(rgb));
    }
    if let Some((_, bullet)) = shape.list {
        // The paragraph-numbering group predates Word 97 and Word still reads
        // it, which is what makes a copied list arrive as a list rather than as
        // paragraphs that used to be one. The `\pntext` group beside it is the
        // label spelled out, for a reader that ignores the definition.
        let symbol = table.font("Symbol");
        match bullet {
            true => {
                let _ = write!(
                    out,
                    "{{\\pntext\\f{symbol}\\'b7\\tab}}{{\\*\\pn\\pnlvlblt\\pnf{symbol}\\pnindent0{{\\pntxtb\\'b7}}}}"
                );
            }
            false => {
                out.push_str(
                    "{\\pntext\\f0 1.\\tab}{\\*\\pn\\pnlvlbody\\pnf0\\pnindent0\\pnstart1\\pndec{\\pntxta.}}",
                );
            }
        }
    }
}

fn run_rtf(look: &Look, table: &mut Table, out: &mut String) {
    let _ = write!(
        out,
        "\\f{}\\fs{}",
        table.font(&look.family),
        // Half-points, and rounded rather than truncated: 11.5pt is 23 and not
        // 22.
        (look.size * 2.0).round() as i64
    );
    for (on, word) in [
        (look.bold, "\\b"),
        (look.italic, "\\i"),
        (look.underline, "\\ul"),
        (look.strike, "\\strike"),
        (look.caps, "\\caps"),
        (look.small_caps, "\\scaps"),
    ] {
        if on {
            out.push_str(word);
        }
    }
    match look.vert {
        VertAlign::Superscript => out.push_str("\\super"),
        VertAlign::Subscript => out.push_str("\\sub"),
        VertAlign::Baseline => {}
    }
    if let Some(rgb) = look.color {
        let _ = write!(out, "\\cf{}", table.color(rgb));
    }
    if let Some(rgb) = look.highlight {
        let _ = write!(out, "\\highlight{}", table.color(rgb));
    } else if let Some(rgb) = look.shading {
        let _ = write!(out, "\\chcbpat{}", table.color(rgb));
    }
}

fn piece_rtf(piece: &Piece, table: &mut Table, out: &mut String) {
    match piece {
        Piece::Text(text) => rtf_text(text, out),
        Piece::Tab => out.push_str("\\tab "),
        Piece::Break(Break::Line) => out.push_str("\\line "),
        Piece::Hyphen { breaking: false } => out.push_str("\\_"),
        Piece::Hyphen { breaking: true } => out.push_str("\\-"),
        // A symbol is a byte in its own font's encoding rather than a Unicode
        // character: the code point is in the private use area and means nothing
        // to a reader that is not told the face.
        Piece::Symbol { font, ch } => {
            let _ = write!(
                out,
                "{{\\f{}\\'{:02x}}}",
                table.font(font),
                *ch as u32 & 0xff
            );
        }
        _ => {}
    }
}

/// Writes text as RTF, which is ASCII and escapes.
fn rtf_text(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\line "),
            '\t' => out.push_str("\\tab "),
            c if (c as u32) < 0x80 => out.push(c),
            // `\u` takes a *signed* sixteen-bit word, and a character above the
            // basic plane takes two of them — the surrogate pair, which is what
            // the control word was defined over. The `?` after each is the
            // fallback `\uc1` in the header promises.
            c => {
                let mut buffer = [0u16; 2];
                for unit in c.encode_utf16(&mut buffer) {
                    let _ = write!(out, "\\u{}?", *unit as i16);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Block, Run};
    use wp_model::numbering::{AbstractNum, Level, Num};
    use wp_model::prop::{Fonts, ParaProps, Underline, UnderlineKind};
    use wp_model::style::{Style, StyleKind};
    use wp_model::units::{HalfPoint, Line240};

    fn document() -> Document {
        Document::blank()
    }

    fn run(text: &str, props: RunProps) -> Run {
        Run {
            props,
            ..Run::of(text)
        }
    }

    fn paragraph(runs: Vec<Run>) -> Paragraph {
        Paragraph {
            content: runs.into_iter().map(wp_model::doc::Inline::Run).collect(),
            ..Paragraph::new()
        }
    }

    fn bold() -> RunProps {
        let mut props = RunProps::new();
        props.toggles.set(Toggle::Bold, true);
        props
    }

    #[test]
    fn a_bold_word_is_bold_in_both_formats() {
        let document = document();
        let paragraphs = vec![paragraph(vec![
            run("plain ", RunProps::new()),
            run("bold", bold()),
        ])];
        let html = html(&document, &paragraphs);
        assert!(html.contains("font-weight:bold"), "{html}");
        assert!(html.contains(">bold</span>"), "{html}");
        let rtf = rtf(&document, &paragraphs);
        assert!(rtf.contains("\\b"), "{rtf}");
        assert!(rtf.contains("bold}"), "{rtf}");
    }

    #[test]
    fn a_font_that_is_really_a_list_of_them_does_not_break_either_format() {
        // What LibreOffice writes: one attribute holding two names. The
        // separator it picked ends a CSS declaration and ends an RTF font table
        // entry, so left alone it took the rest of both with it.
        let document = document();
        let props = RunProps {
            fonts: Fonts {
                ascii: Some("Liberation Sans;Arial".into()),
                ..Fonts::default()
            },
            ..RunProps::new()
        };
        let paragraphs = vec![paragraph(vec![run("text", props)])];
        let html = html(&document, &paragraphs);
        assert!(
            html.contains("font-family:'Liberation Sans','Arial';"),
            "{html}"
        );
        let rtf = rtf(&document, &paragraphs);
        assert!(rtf.contains("\\fcharset0 Liberation Sans;}"), "{rtf}");
        assert!(!rtf.contains("Arial"), "{rtf}");
    }

    #[test]
    fn what_a_style_says_reaches_the_board_though_the_run_says_nothing() {
        let mut document = document();
        let mut style = Style::new("Heading1", StyleKind::Paragraph);
        style.run.size = Some(HalfPoint(32));
        style.run.toggles.set(Toggle::Bold, true);
        style.run.fonts = Fonts {
            ascii: Some("Cambria".into()),
            ..Fonts::default()
        };
        let id = document.styles.insert(style);
        let paragraphs = vec![Paragraph {
            props: ParaProps {
                style: Some(id),
                ..ParaProps::new()
            },
            ..paragraph(vec![run("Results", RunProps::new())])
        }];

        // The run's own properties are empty. Anything reading only those would
        // put eleven-point Calibri on the board and lose the heading.
        let html = html(&document, &paragraphs);
        assert!(html.contains("font-size:16.0pt"), "{html}");
        assert!(html.contains("Cambria"), "{html}");
        assert!(html.contains("font-weight:bold"), "{html}");
        let rtf = rtf(&document, &paragraphs);
        assert!(rtf.contains("\\fs32"), "{rtf}");
        assert!(rtf.contains("Cambria;"), "{rtf}");
    }

    #[test]
    fn one_paragraph_is_written_without_a_paragraph_around_it() {
        let document = document();
        let one = vec![paragraph(vec![run("half a sentence", RunProps::new())])];
        let alone = html(&document, &one);
        assert!(!alone.contains("<p"), "{alone}");
        // `\pard` is the reset that starts every paragraph, so the break to look
        // for is the control word on its own.
        let alone = rtf(&document, &one);
        assert!(!alone.contains("\\par\n"), "{alone}");

        // Two of them do have a break — between, and not after the last, which
        // would paste an empty paragraph nobody copied.
        let two = vec![
            paragraph(vec![run("first", RunProps::new())]),
            paragraph(vec![run("second", RunProps::new())]),
        ];
        let pair = html(&document, &two);
        assert_eq!(pair.matches("<p ").count(), 2, "{pair}");
        let pair = rtf(&document, &two);
        assert_eq!(pair.matches("\\par\n").count(), 1, "{pair}");
    }

    #[test]
    fn a_run_of_spaces_survives_html_collapsing_them() {
        let document = document();
        let paragraphs = vec![paragraph(vec![run("a    b", RunProps::new())])];
        let html = html(&document, &paragraphs);
        assert!(html.contains("a &nbsp;&nbsp;&nbsp;b"), "{html}");
    }

    #[test]
    fn the_characters_that_mean_something_to_a_format_are_escaped() {
        let document = document();
        let paragraphs = vec![paragraph(vec![run("a<b>&c {d} \\e — ü", RunProps::new())])];
        let html = html(&document, &paragraphs);
        assert!(html.contains("a&lt;b&gt;&amp;c"), "{html}");
        let rtf = rtf(&document, &paragraphs);
        assert!(rtf.contains("\\{d\\} \\\\e"), "{rtf}");
        // An em dash and a u-umlaut are not ASCII and RTF is.
        assert!(rtf.contains("\\u8212?"), "{rtf}");
        assert!(rtf.contains("\\u252?"), "{rtf}");
    }

    #[test]
    fn a_colour_is_declared_once_and_used_by_number() {
        let document = document();
        let red = RunProps {
            color: Some(wp_model::color::Color::Rgb([0xFF, 0x00, 0x00])),
            ..RunProps::new()
        };
        let paragraphs = vec![paragraph(vec![
            run("one", red.clone()),
            run("two", red),
            run("three", RunProps::new()),
        ])];
        let rtf = rtf(&document, &paragraphs);
        assert_eq!(rtf.matches("\\red255\\green0\\blue0;").count(), 1, "{rtf}");
        assert_eq!(rtf.matches("\\cf1").count(), 2, "{rtf}");
        let html = html(&document, &paragraphs);
        assert_eq!(html.matches("color:#ff0000").count(), 2, "{html}");
    }

    #[test]
    fn an_underline_and_a_strike_are_one_declaration_in_html() {
        let document = document();
        let mut props = RunProps {
            underline: Some(Underline {
                kind: UnderlineKind::Single,
                color: None,
            }),
            ..RunProps::new()
        };
        props.toggles.set(Toggle::Strike, true);
        let paragraphs = vec![paragraph(vec![run("struck", props)])];
        let html = html(&document, &paragraphs);
        assert!(
            html.contains("text-decoration:underline line-through"),
            "{html}"
        );
        let rtf = rtf(&document, &paragraphs);
        assert!(rtf.contains("\\ul\\strike"), "{rtf}");
    }

    #[test]
    fn a_paragraphs_shape_is_stated_in_the_units_each_format_uses() {
        let document = document();
        let props = ParaProps {
            justify: Some(Justify::Center),
            spacing: wp_model::prop::Spacing {
                before: Some(Twips(120)),
                after: Some(Twips(240)),
                line: Some(LineSpacing::Multiple(Line240(360))),
                ..wp_model::prop::Spacing::default()
            },
            indent: wp_model::prop::Indent {
                start: Some(Twips(720)),
                hanging: Some(Twips(360)),
                ..wp_model::prop::Indent::default()
            },
            ..ParaProps::new()
        };
        let paragraphs = vec![
            Paragraph {
                props: props.clone(),
                ..paragraph(vec![run("centred", RunProps::new())])
            },
            Paragraph {
                props,
                ..paragraph(vec![run("also", RunProps::new())])
            },
        ];
        let html = html(&document, &paragraphs);
        assert!(html.contains("text-align:center"), "{html}");
        assert!(html.contains("margin-top:6.0pt"), "{html}");
        assert!(html.contains("margin-left:36.0pt"), "{html}");
        assert!(html.contains("text-indent:-18.0pt"), "{html}");
        assert!(html.contains("line-height:150%"), "{html}");
        let rtf = rtf(&document, &paragraphs);
        assert!(
            rtf.contains("\\qc\\sb120\\sa240\\li720\\ri0\\fi-360"),
            "{rtf}"
        );
        assert!(rtf.contains("\\sl360\\slmult1"), "{rtf}");
    }

    /// A document with one bulleted list and one numbered one.
    fn listed() -> Document {
        let mut document = Document::blank();
        for (id, format) in [(1u32, NumFormat::Bullet), (2, NumFormat::Decimal)] {
            let mut definition = AbstractNum::new(id);
            for index in 0..3u8 {
                let mut level = Level::new(index);
                level.format = format.clone();
                definition.set_level(level);
            }
            document.numbering.insert_abstract(definition);
            document.numbering.insert_num(Num::new(id, id));
        }
        document
    }

    fn item(num_id: u32, level: u8, text: &str) -> Paragraph {
        Paragraph {
            props: ParaProps {
                numbering: Some(NumRef { num_id, level }),
                ..ParaProps::new()
            },
            ..paragraph(vec![run(text, RunProps::new())])
        }
    }

    #[test]
    fn a_copied_list_arrives_as_a_list() {
        let document = listed();
        let paragraphs = vec![item(1, 0, "one"), item(1, 0, "two"), item(2, 0, "first")];
        let html = html(&document, &paragraphs);
        // The bulleted pair share one `<ul>`; the numbered one is its own list
        // and not a third item in the first.
        assert_eq!(html.matches("<ul>").count(), 1, "{html}");
        assert_eq!(html.matches("</ul>").count(), 1, "{html}");
        assert_eq!(html.matches("<ol>").count(), 1, "{html}");
        assert_eq!(html.matches("<li ").count(), 3, "{html}");
        assert!(html.ends_with("</ol>"), "{html}");

        let rtf = rtf(&document, &paragraphs);
        assert_eq!(rtf.matches("\\pnlvlblt").count(), 2, "{rtf}");
        assert_eq!(rtf.matches("\\pndec").count(), 1, "{rtf}");
    }

    #[test]
    fn a_deeper_level_nests_and_comes_back_out() {
        let document = listed();
        let paragraphs = vec![
            item(1, 0, "one"),
            item(1, 1, "under one"),
            item(1, 0, "two"),
        ];
        let html = html(&document, &paragraphs);
        assert_eq!(html.matches("<ul>").count(), 2, "{html}");
        assert_eq!(html.matches("</ul>").count(), 2, "{html}");
        // The inner list closes before the outer one's next item, or the item
        // would sit at the wrong depth.
        let inner = html.find("</ul>").expect("a close");
        let last = html.rfind("<li ").expect("an item");
        assert!(inner < last, "{html}");
    }

    #[test]
    fn a_list_closes_when_the_paragraphs_stop_being_one() {
        let document = listed();
        let paragraphs = vec![
            item(1, 0, "one"),
            paragraph(vec![run("after", RunProps::new())]),
        ];
        let html = html(&document, &paragraphs);
        assert!(html.contains("</ul><p "), "{html}");
    }

    #[test]
    fn a_tab_and_a_break_are_the_things_they_are_in_both() {
        let document = document();
        let paragraphs = vec![paragraph(vec![Run {
            content: vec![
                Piece::Text("a".into()),
                Piece::Tab,
                Piece::Break(Break::Line),
                Piece::Text("b".into()),
            ],
            ..Run::new()
        }])];
        let html = html(&document, &paragraphs);
        assert!(html.contains("mso-tab-count:1"), "{html}");
        assert!(html.contains("<br>"), "{html}");
        let rtf = rtf(&document, &paragraphs);
        assert!(rtf.contains("a\\tab \\line b"), "{rtf}");
    }

    #[test]
    fn the_body_of_the_document_is_not_what_is_written() {
        // Only the paragraphs handed over — a copy is a selection, and the
        // document is there for its styles, its numbering and its theme.
        let mut document = document();
        document.body = vec![Block::Paragraph(paragraph(vec![run(
            "not copied",
            RunProps::new(),
        )]))];
        let html = html(
            &document,
            &[paragraph(vec![run("copied", RunProps::new())])],
        );
        assert!(!html.contains("not copied"), "{html}");
        assert!(html.contains("copied"), "{html}");
    }
}
