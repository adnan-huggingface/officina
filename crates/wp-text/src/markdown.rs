//! Markdown, both ways.
//!
//! Markdown and a Word document are not the same shape, and the interesting part
//! is where they disagree.
//!
//! **A heading in Markdown is a level; a heading in Word is a style.** So
//! importing `## Results` looks for a style called `Heading2` and *makes one* if
//! the document has none — a paragraph with an outline level and no style is a
//! heading nothing can restyle.
//!
//! **Emphasis in Markdown is a span; emphasis in Word is a run property.** The
//! import splits runs at the markers; the export puts the markers back at the
//! run boundaries, which is not always where they were — `**bold** text` and
//! `**bold**` + ` text` are the same document and different files.
//!
//! **An underscore inside a word is a letter.** `snake_case_name` is a name,
//! not `snake`, an italic `case` and `name`, as CommonMark reads it; an
//! asterisk inside a word still opens emphasis, as CommonMark's does.
//!
//! **Text that looks like Markdown is written escaped.** A paragraph that
//! reads `1. Introduction` or `* see note *` is written `1\. Introduction`
//! and `\* see note \*`, and a backslash before punctuation is read as that
//! punctuation, so that what is written reads back as the same text. A line
//! break inside a paragraph is written, and read, as `<br>`.
//!
//! **Stated limits.** Not implemented on import: reference links, footnotes,
//! tables, block quotes beyond one level, setext headings, and HTML. Each is
//! carried through as the literal text it is, which is what a Markdown reader
//! that does not know a construct should do — the text is not lost, it is just
//! not interpreted.

use wp_model::doc::{Block, Document, Inline, Paragraph, Piece, Run};
use wp_model::prop::{NumRef, Toggle};
use wp_model::style::{Style, StyleId, StyleKind, StyleTable};
use wp_model::units::HalfPoint;

/// Reads Markdown into a document.
pub fn read(source: &str) -> Document {
    let mut document = blank();
    let mut body = Vec::new();
    let mut list: Option<u32> = None;

    let lines = crate::encoding::lines(source);
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();

        // A fenced code block is taken whole, because everything inside it is
        // text rather than Markdown — a `#` in a code fence is a comment, not a
        // heading.
        if let Some(fence) = fence_of(trimmed) {
            index += 1;
            let mut code = Vec::new();
            while index < lines.len() && fence_of(lines[index].trim_start()) != Some(fence) {
                code.push(lines[index]);
                index += 1;
            }
            index += 1;
            for row in code {
                body.push(Block::Paragraph(code_paragraph(&mut document.styles, row)));
            }
            continue;
        }
        index += 1;

        if trimmed.is_empty() {
            list = None;
            continue;
        }
        if is_rule(trimmed) {
            body.push(Block::Paragraph(Paragraph::new()));
            continue;
        }

        if let Some((level, text)) = heading_of(trimmed) {
            list = None;
            let style = heading_style(&mut document.styles, level);
            let mut paragraph = spans(text);
            paragraph.props.style = Some(style);
            body.push(Block::Paragraph(paragraph));
            continue;
        }

        if let Some((ordered, text)) = bullet_of(trimmed) {
            let num_id = *list.get_or_insert_with(|| numbering_for(&mut document, ordered));
            let mut paragraph = spans(text);
            paragraph.props.numbering = Some(NumRef { num_id, level: 0 });
            body.push(Block::Paragraph(paragraph));
            continue;
        }
        list = None;

        if let Some(text) = trimmed
            .strip_prefix("> ")
            .or_else(|| trimmed.strip_prefix('>'))
        {
            let mut paragraph = spans(text.trim_start());
            paragraph.props.indent.start = Some(wp_model::Twips(720));
            paragraph.props.justify = None;
            body.push(Block::Paragraph(paragraph));
            continue;
        }

        body.push(Block::Paragraph(spans(trimmed)));
    }

    if body.is_empty() {
        body.push(Block::Paragraph(Paragraph::new()));
    }
    document.body = body;
    document
}

/// Writes a document as Markdown.
pub fn write(document: &Document) -> String {
    let mut out = String::new();
    let mut counter = 0u32;
    let mut previous_was_list = false;

    for paragraph in document.paragraphs() {
        let level = wp_model::outline::heading_level(paragraph, &document.styles);
        let numbered = paragraph
            .props
            .numbering
            .filter(|reference| reference.is_numbered());
        let text = markers(paragraph);

        if numbered.is_none() {
            counter = 0;
        }
        if previous_was_list && numbered.is_none() {
            out.push('\n');
        }
        previous_was_list = numbered.is_some();

        match (level, numbered) {
            (Some(level), _) => {
                out.push('\n');
                for _ in 0..level.min(6) {
                    out.push('#');
                }
                out.push(' ');
                out.push_str(&text);
                out.push_str("\n\n");
            }
            (None, Some(reference)) => {
                let ordered = document
                    .numbering
                    .level(reference.num_id, reference.level)
                    .is_some_and(|level| level.format.counts());
                if ordered {
                    counter += 1;
                    out.push_str(&format!("{counter}. {text}\n"));
                } else {
                    out.push_str(&format!("- {text}\n"));
                }
            }
            (None, None) if text.trim().is_empty() => out.push('\n'),
            (None, None) => {
                out.push_str(&escape_start(&text));
                out.push_str("\n\n");
            }
        }
    }
    // A file that ends in three blank lines is not what anybody wrote.
    while out.ends_with("\n\n\n") {
        out.pop();
    }
    out.trim_start_matches('\n').to_owned()
}

/// One paragraph as one line of Markdown, as [`write`] writes it: its
/// heading's `#`, its list's `-` or `1.`, and `**` and `*` around its bold and
/// italic runs, with a line break as `<br>` — and a paragraph with no text as
/// `<empty>`. For handing a document to something that reads it a numbered
/// line at a time; [`read_lines`] reads it back.
pub fn line(document: &Document, paragraph: &Paragraph) -> String {
    let text = markers(paragraph);
    if text.trim().is_empty() {
        return EMPTY.to_owned();
    }
    if let Some(level) = wp_model::outline::heading_level(paragraph, &document.styles) {
        return format!(
            "{} {}",
            "#".repeat(level.clamp(1, 6) as usize),
            escape_breaks(&text, false)
        );
    }
    match paragraph
        .props
        .numbering
        .filter(|reference| reference.is_numbered())
    {
        Some(reference) => {
            let ordered = document
                .numbering
                .level(reference.num_id, reference.level)
                .is_some_and(|level| level.format.counts());
            format!(
                "{} {}",
                if ordered { "1." } else { "-" },
                escape_breaks(&text, false)
            )
        }
        None => escape_breaks(&text, true),
    }
}

/// A paragraph's text with every part after a line break escaped as the start
/// of a line, and the first part too when `lead`.
///
/// **A paragraph is one line, so nothing inside it may read as another.** The
/// text after a break starts a line on the page, and a reader — a person or a
/// helper reading numbered paragraphs — takes `# x` or `[9] x` there for a
/// heading or another paragraph. After `#` or `1.` the first part is already
/// inside a line, so it is left alone. An escaped `\<br>` splits here too and
/// joins back unchanged, and a backslash the reader does not need is dropped
/// when it reads the line again.
fn escape_breaks(text: &str, lead: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for (at, part) in text.split("<br>").enumerate() {
        if at > 0 {
            out.push_str("<br>");
        }
        if at > 0 || lead {
            out.push_str(&escape_start(part));
        } else {
            out.push_str(part);
        }
    }
    out
}

/// How [`line`] writes a paragraph with no text.
pub const EMPTY: &str = "<empty>";

/// Reads what [`line`] writes: a paragraph a line, each with the level of
/// the heading it is, if it is one. Blank lines part nothing; a line of
/// [`EMPTY`] is an empty paragraph; a list's or a quote's marker is dropped,
/// and a rule is an empty paragraph.
pub fn read_lines(source: &str) -> Vec<(Option<u8>, Paragraph)> {
    let mut out = Vec::new();
    for raw in crate::encoding::lines(source) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == EMPTY || is_rule(line) {
            out.push((None, Paragraph::new()));
            continue;
        }
        if let Some((level, text)) = heading_of(line) {
            out.push((Some(level), spans(text)));
            continue;
        }
        let text = match bullet_of(line) {
            Some((_, text)) => text,
            None => line
                .strip_prefix("> ")
                .or_else(|| line.strip_prefix('>'))
                .unwrap_or(line),
        };
        out.push((None, spans(text)));
    }
    out
}

/// A paragraph's text with `**` and `*` back around its emphasised runs.
fn markers(paragraph: &Paragraph) -> String {
    let mut out = String::new();
    for run in paragraph.runs() {
        let text = readable(&run.text());
        if text.is_empty() {
            continue;
        }
        // The markers go *inside* the spaces: `**bold** word`, never `**bold **
        // word`, which Markdown does not read as emphasis at all. A run of
        // nothing but a space — the one between a bold word and an italic
        // one — is all lead, and was cut past its own end.
        let started = text.trim_start();
        let lead = &text[..text.len() - started.len()];
        let core = started.trim_end();
        let tail = &started[core.len()..];
        let mark = match (run.props.bold(), run.props.italic()) {
            (true, true) => "***",
            (true, false) => "**",
            (false, true) => "*",
            (false, false) => "",
        };
        out.push_str(&breaks(lead));
        if core.is_empty() {
            out.push_str(&breaks(tail));
            continue;
        }
        out.push_str(mark);
        out.push_str(&breaks(&escape_inline(core)));
        out.push_str(mark);
        out.push_str(&breaks(tail));
    }
    out
}

/// A line break as `<br>`, so that a paragraph stays one line; and anything
/// else that would end a line, as a space.
fn breaks(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => "<br>".to_owned(),
            '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}' => " ".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// Text with a backslash before what the reader would take for Markdown: a
/// backslash, an asterisk, an underscore not inside a word, and the `<` of
/// something that would read as a line break or an empty paragraph.
fn escape_inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (at, &c) in chars.iter().enumerate() {
        let before = at.checked_sub(1).map(|i| chars[i]);
        let after = chars.get(at + 1).copied();
        let escaped = match c {
            '\\' | '*' => true,
            '_' => {
                !(before.is_some_and(char::is_alphanumeric)
                    && after.is_some_and(|c| c.is_alphanumeric() || c == '_'))
            }
            '<' => {
                let rest: String = chars[at..].iter().take(7).collect();
                rest.starts_with("<br") || rest.starts_with(EMPTY)
            }
            _ => false,
        };
        if escaped {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A plain paragraph's text with a backslash before whatever at its start
/// would make it a heading, a list item, a quote, a rule, a fence — or, in
/// [`line`]'s numbered lines, a paragraph number.
fn escape_start(text: &str) -> String {
    let body = text.trim_start();
    let lead = &text[..text.len() - body.len()];
    let digits = body.chars().take_while(char::is_ascii_digit).count();
    let escaped = if body.starts_with('#')
        || body.starts_with('[')
        || body.starts_with('>')
        || body.starts_with("- ")
        || body.starts_with("+ ")
        || body.starts_with("```")
        || body.starts_with("~~~")
        || is_rule(body)
    {
        format!("\\{body}")
    } else if digits > 0 && (body[digits..].starts_with(". ") || body[digits..].starts_with(") ")) {
        format!("{}\\{}", &body[..digits], &body[digits..])
    } else {
        return text.to_owned();
    };
    format!("{lead}{escaped}")
}

/// `# Heading` -> (1, "Heading").
fn heading_of(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    // `#Hashtag` is not a heading: Markdown needs the space, and a document full
    // of headings called `#rust` is what happens without this.
    let text = rest.strip_prefix(' ')?.trim_end();
    // A closing run of `#` is one only after a space: `C\#` ends in a
    // character, not a closing run.
    let closed = text.trim_end_matches('#');
    let text = match closed.is_empty() || closed.ends_with(' ') {
        true => closed,
        false => text,
    };
    Some((hashes as u8, text.trim()))
}

/// `- item`, `* item`, `1. item`.
fn bullet_of(line: &str) -> Option<(bool, &str)> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some((false, rest));
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((true, rest));
        }
    }
    None
}

fn is_rule(line: &str) -> bool {
    let squashed: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    squashed.len() >= 3
        && (squashed.chars().all(|c| c == '-')
            || squashed.chars().all(|c| c == '*')
            || squashed.chars().all(|c| c == '_'))
}

fn fence_of(line: &str) -> Option<char> {
    ['`', '~']
        .into_iter()
        .find(|marker| line.starts_with(&marker.to_string().repeat(3)))
}

/// A character of a line, as emphasis sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Unit {
    /// Written as itself: an asterisk or an underscore may open or close
    /// emphasis.
    Char(char),
    /// Escaped with a backslash: only ever itself.
    Literal(char),
    /// `<br>`.
    Break,
}

impl Unit {
    fn char(self) -> Option<char> {
        match self {
            Unit::Char(c) | Unit::Literal(c) => Some(c),
            Unit::Break => None,
        }
    }
}

fn units(line: &str) -> Vec<Unit> {
    let mut out = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some(&(_, next)) if next.is_ascii_punctuation() => {
                    chars.next();
                    out.push(Unit::Literal(next));
                }
                _ => out.push(Unit::Literal('\\')),
            }
            continue;
        }
        if c == '<' {
            let rest = &line[at..];
            if let Some(tag) = ["<br>", "<br/>", "<br />"]
                .into_iter()
                .find(|tag| rest.starts_with(tag))
            {
                // The rest of the tag, all of it ASCII.
                for _ in 1..tag.len() {
                    chars.next();
                }
                out.push(Unit::Break);
                continue;
            }
        }
        out.push(Unit::Char(c));
    }
    out
}

/// Splits a line into runs at its emphasis markers.
fn spans(line: &str) -> Paragraph {
    let units = units(line);
    let mut paragraph = Paragraph::new();
    let mut plain: Vec<Unit> = Vec::new();

    let push = |paragraph: &mut Paragraph, units: &[Unit], bold: bool, italic: bool| {
        if units.is_empty() {
            return;
        }
        let mut pieces = Vec::new();
        let mut text = String::new();
        for unit in units {
            match unit.char() {
                Some(c) => text.push(c),
                None => {
                    if !text.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut text).into()));
                    }
                    pieces.push(Piece::Break(wp_model::doc::Break::Line));
                }
            }
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text.into()));
        }
        let mut run = Run {
            content: pieces,
            ..Run::new()
        };
        if bold || italic {
            run.props.toggles.set(Toggle::Bold, bold);
            run.props.toggles.set(Toggle::Italic, italic);
        }
        paragraph.content.push(Inline::Run(run));
    };
    let alphanumeric = |unit: Option<&Unit>| {
        unit.and_then(|unit| unit.char())
            .is_some_and(char::is_alphanumeric)
    };
    // How many of `c` stand in a row from `at`, written as themselves.
    let run_of = |at: usize, c: char| {
        units[at..]
            .iter()
            .take_while(|unit| **unit == Unit::Char(c))
            .count()
    };

    let mut at = 0;
    while at < units.len() {
        let c = match units[at] {
            Unit::Char(c @ ('*' | '_')) => c,
            other => {
                plain.push(other);
                at += 1;
                continue;
            }
        };
        let length = run_of(at, c);
        // Underscores after a letter are part of the word, however many.
        if c == '_' && alphanumeric(at.checked_sub(1).and_then(|i| units.get(i))) {
            plain.extend(std::iter::repeat_n(Unit::Char(c), length));
            at += length;
            continue;
        }
        let width = length.min(3);
        // The closing run: as wide, and for underscores, not followed by a
        // letter, which would make it part of a word.
        let mut close = None;
        let mut probe = at + width;
        while probe < units.len() {
            let found = run_of(probe, c);
            if found == 0 {
                probe += 1;
                continue;
            }
            if found >= width
                && probe > at + width
                && (c != '_' || !alphanumeric(units.get(probe + found)))
            {
                close = Some(probe);
                break;
            }
            probe += found;
        }
        let Some(close) = close else {
            // A lone `*` is a literal asterisk, which is what Markdown does and
            // what a document full of `*` in the middle of sentences needs.
            plain.extend(std::iter::repeat_n(Unit::Char(c), width));
            at += width;
            continue;
        };
        push(&mut paragraph, &plain, false, false);
        plain.clear();
        let (bold, italic) = match width {
            1 => (false, true),
            2 => (true, false),
            _ => (true, true),
        };
        push(&mut paragraph, &units[at + width..close], bold, italic);
        at = close + width;
    }
    push(&mut paragraph, &plain, false, false);
    paragraph
}

fn code_paragraph(styles: &mut StyleTable, text: &str) -> Paragraph {
    let style = code_style(styles);
    let mut paragraph = Paragraph::new();
    paragraph.props.style = Some(style);
    paragraph.content.push(Inline::Run(Run {
        content: vec![Piece::Text(text.into())],
        ..Run::new()
    }));
    paragraph
}

/// The `HeadingN` style, made if the document has none.
///
/// A paragraph with an outline level and no style is a heading nothing can
/// restyle, which is not what importing a heading should produce.
fn heading_style(styles: &mut StyleTable, level: u8) -> StyleId {
    let id = format!("Heading{level}");
    if let Some(found) = styles.lookup(&id) {
        return found;
    }
    let mut style = Style::new(id.as_str(), StyleKind::Paragraph);
    style.name = Some(format!("heading {level}").into());
    style.quick = true;
    style.priority = Some(level as i32);
    style.para.outline_level = Some(level - 1);
    style.para.keep_next = Some(true);
    style.run.size = Some(HalfPoint(match level {
        1 => 32,
        2 => 26,
        3 => 24,
        _ => 22,
    }));
    style.run.toggles.set(Toggle::Bold, true);
    styles.insert(style)
}

fn code_style(styles: &mut StyleTable) -> StyleId {
    if let Some(found) = styles.lookup("HTMLPreformatted") {
        return found;
    }
    let mut style = Style::new("HTMLPreformatted", StyleKind::Paragraph);
    style.name = Some("HTML Preformatted".into());
    style.run.fonts.ascii = Some("Consolas".into());
    style.run.fonts.high_ansi = Some("Consolas".into());
    style.run.size = Some(HalfPoint(20));
    styles.insert(style)
}

/// A list definition for an imported list.
fn numbering_for(document: &mut Document, ordered: bool) -> u32 {
    let abstract_id = document.numbering.nums().count() as u32;
    let num_id = abstract_id + 1;
    let mut definition = wp_model::AbstractNum::new(abstract_id);
    let mut level = wp_model::Level::new(0);
    if ordered {
        level.format = wp_model::NumFormat::Decimal;
        level.text = "%1.".into();
    } else {
        level.format = wp_model::NumFormat::Bullet;
        level.text = "\u{2022}".into();
        level.run.fonts.ascii = Some("Symbol".into());
    }
    level.para.indent.start = Some(wp_model::Twips(720));
    level.para.indent.hanging = Some(wp_model::Twips(360));
    definition.set_level(level);
    document.numbering.insert_abstract(definition);
    document
        .numbering
        .insert_num(wp_model::Num::new(num_id, abstract_id));
    num_id
}

/// A document with the defaults an imported file needs.
pub fn blank() -> Document {
    let mut document = Document::new();
    let mut normal = Style::new("Normal", StyleKind::Paragraph);
    normal.default = true;
    normal.name = Some("Normal".into());
    normal.run.size = Some(HalfPoint::DEFAULT);
    normal.run.fonts.ascii = Some("Calibri".into());
    normal.run.fonts.high_ansi = Some("Calibri".into());
    normal.para.spacing.after = Some(wp_model::Twips(160));
    document.styles.insert(normal);
    document.body = vec![Block::Paragraph(Paragraph::new())];
    document
}

/// Reads plain text: one paragraph per line, and nothing interpreted.
pub fn read_plain(source: &str) -> Document {
    let mut document = blank();
    let lines = crate::encoding::lines(source);
    document.body = if lines.is_empty() {
        vec![Block::Paragraph(Paragraph::new())]
    } else {
        lines
            .into_iter()
            .map(|line| Block::Paragraph(Paragraph::of(line)))
            .collect()
    };
    document
}

/// Writes plain text: the document's text, and nothing else.
///
/// Every piece of formatting is lost, which is what plain text *is* — and the
/// application says so before it saves, because a user who did not mean it has
/// no way back.
pub fn write_plain(document: &Document, ending: crate::encoding::LineEnding) -> String {
    document
        .paragraphs()
        .iter()
        .map(|paragraph| readable(&paragraph.text()))
        .collect::<Vec<_>>()
        .join(ending.as_str())
}

/// A paragraph's text with the pictures taken out of it.
///
/// An inline picture is a character of the document's text — see
/// [`wp_model::doc::OBJECT`] — and writing that character into a text file
/// would put a control code in the middle of a sentence. A picture in plain
/// text is nothing, which is what plain text means.
fn readable(text: &str) -> String {
    match text.contains(wp_model::doc::OBJECT) {
        true => text.replace(wp_model::doc::OBJECT, ""),
        false => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(document: &Document) -> Vec<String> {
        document
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.text())
            .collect()
    }

    #[test]
    fn a_heading_becomes_a_paragraph_with_a_heading_style() {
        // Not merely an outline level: a paragraph with a level and no style is
        // a heading nothing can restyle.
        let document = read("# Title\n\nBody text.\n");
        let paragraphs = document.paragraphs();
        let style = paragraphs[0].props.style.expect("a style");
        assert_eq!(document.styles.get(style).unwrap().id.as_ref(), "Heading1");
        assert_eq!(
            wp_model::outline::heading_level(paragraphs[0], &document.styles),
            Some(1)
        );
        assert_eq!(text_of(&document), ["Title", "Body text."]);
    }

    #[test]
    fn a_hash_with_no_space_is_not_a_heading() {
        // Otherwise a document about Rust is a document full of headings called
        // `#rust`.
        let document = read("#rust is good\n");
        assert!(document.paragraphs()[0].props.style.is_none());
        assert_eq!(text_of(&document), ["#rust is good"]);
    }

    #[test]
    fn emphasis_becomes_run_properties() {
        let document = read("plain **bold** and *italic* end\n");
        let paragraph = document.paragraphs()[0];
        let runs = paragraph.runs();
        assert_eq!(paragraph.text(), "plain bold and italic end");
        let bold = runs
            .iter()
            .find(|run| run.props.bold())
            .expect("a bold run");
        assert_eq!(bold.text(), "bold");
        let italic = runs
            .iter()
            .find(|run| run.props.italic())
            .expect("an italic run");
        assert_eq!(italic.text(), "italic");
    }

    /// A paragraph is handed over a line at a time, marked as the whole
    /// document would be; and an underscore inside a word, which a name has,
    /// reads back as a letter, where one around a word is still emphasis.
    #[test]
    fn one_paragraph_is_one_line_of_markdown_and_an_underscore_in_a_word_is_a_letter() {
        let document = read("# Title\n\nplain **bold** *it*\n\n- item\n\n1. first\n");
        let lines: Vec<String> = document
            .paragraphs()
            .iter()
            .map(|paragraph| line(&document, paragraph))
            .collect();
        assert_eq!(
            lines,
            ["# Title", "plain **bold** *it*", "- item", "1. first"]
        );
        // The space between the bold word and the italic one is a run of its
        // own, which the writer cut past its end.
        assert!(write(&document).contains("plain **bold** *it*"));

        let names =
            read("call snake_case_name and __init__ with _care_ and a_b_ or _snake_case_\n");
        let paragraph = names.paragraphs()[0];
        assert_eq!(
            paragraph.text(),
            "call snake_case_name and init with care and a_b_ or snake_case"
        );
        let emphasised: Vec<(String, bool, bool)> = paragraph
            .runs()
            .iter()
            .filter(|run| run.props.bold() || run.props.italic())
            .map(|run| (run.text(), run.props.bold(), run.props.italic()))
            .collect();
        assert_eq!(
            emphasised,
            [
                ("init".to_owned(), true, false),
                ("care".to_owned(), false, true),
                ("snake_case".to_owned(), false, true)
            ]
        );
        // Inside a word, an asterisk is still emphasis.
        let starred = read("un*frigging*believable\n");
        assert_eq!(starred.paragraphs()[0].text(), "unfriggingbelievable");
    }

    /// Paragraphs whose text looks like Markdown — a list's number, a
    /// heading's hash, a quote, a rule, stars — are written escaped and read
    /// back as the same text, and a line break inside a paragraph stays one.
    #[test]
    fn text_that_looks_like_markdown_is_written_so_that_it_reads_back_the_same() {
        let texts = [
            "[1] Smith, J. and others",
            "1. Introduction",
            "12) Twelfth",
            "- 5 degrees",
            "+ plus",
            "> quoted",
            "# of items",
            "---",
            "```",
            "Terms marked * apply; see note *",
            "__init__ and _care_ and a_b_ and back\\slash",
            "C# and <br> and <empty> as words",
            "  # indented",
        ];
        let mut document = blank();
        document.body = texts
            .iter()
            .map(|text| Block::Paragraph(Paragraph::of(text)))
            .collect();
        let mut broken = Paragraph::of("first");
        if let Some(Inline::Run(run)) = broken.content.first_mut() {
            run.content.push(Piece::Break(wp_model::doc::Break::Line));
            run.content.push(Piece::Text("second".into()));
        }
        document.body.push(Block::Paragraph(broken));
        let written = write(&document);
        let back = read(&written);
        let mut expected: Vec<String> = texts
            .iter()
            .map(|text| text.trim_start().to_owned())
            .collect();
        expected.push("first\nsecond".to_owned());
        assert_eq!(text_of(&back), expected, "{written}");
        assert!(
            back.paragraphs()
                .iter()
                .all(|paragraph| paragraph.props.style.is_none()
                    && paragraph.props.numbering.is_none()
                    && paragraph.runs().iter().all(|run| !run.props.italic())),
            "{written}"
        );
        // Each paragraph one line, and each line the same text read one at a
        // time.
        let lines: Vec<String> = document
            .paragraphs()
            .iter()
            .map(|paragraph| line(&document, paragraph))
            .collect();
        assert!(lines.iter().all(|line| !line.contains('\n')), "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some("first<br>second"));

        // What follows a break is a line on the page, so it is escaped like
        // one: a paragraph cannot pass itself off as a heading or as another
        // numbered paragraph, and it reads back the same.
        let mut broken = Document::new();
        broken.body = vec![
            Block::Paragraph(Paragraph::of("one\n# Two\n[9] Three")),
            Block::Paragraph(Paragraph::of("[1] Smith")),
        ];
        let written: Vec<String> = broken
            .paragraphs()
            .iter()
            .map(|paragraph| line(&broken, paragraph))
            .collect();
        assert_eq!(written, ["one<br>\\# Two<br>\\[9] Three", "\\[1] Smith"]);
        // A heading's own `#` needs no escape, but what follows a break in it
        // does.
        let mut headed = read("# Head\n");
        let mut head = Paragraph::of("Title\n# Two");
        head.props.style = headed.paragraphs()[0].props.style;
        headed.body = vec![Block::Paragraph(head)];
        assert_eq!(line(&headed, headed.paragraphs()[0]), "# Title<br>\\# Two");

        let back: Vec<String> = read_lines(&written.join("\n"))
            .iter()
            .map(|(_, paragraph)| paragraph.text())
            .collect();
        assert_eq!(back, ["one\n# Two\n[9] Three", "[1] Smith"]);
        let read: Vec<String> = read_lines(&lines.join("\n\n"))
            .into_iter()
            .map(|(level, paragraph)| {
                assert_eq!(level, None);
                paragraph.text()
            })
            .collect();
        assert_eq!(read, expected);
    }

    /// The numbered lines' own shapes: an empty paragraph is `<empty>`,
    /// blank lines part nothing, and a heading's level comes back.
    #[test]
    fn lines_read_a_paragraph_a_line() {
        let lines = read_lines("## Part\n\n<empty>\nplain **bold**<br/>next\n- item\n---\n");
        let summary: Vec<(Option<u8>, String, usize)> = lines
            .iter()
            .map(|(level, paragraph)| (*level, paragraph.text(), paragraph.runs().len()))
            .collect();
        assert_eq!(
            summary,
            [
                (Some(2), "Part".to_owned(), 1),
                (None, String::new(), 0),
                (None, "plain bold\nnext".to_owned(), 3),
                (None, "item".to_owned(), 1),
                (None, String::new(), 0),
            ]
        );
        let document = read_lines("C\\# heading").remove(0).1;
        assert_eq!(document.text(), "C# heading");
        assert_eq!(read("# C\\#\n").paragraphs()[0].text(), "C#");
        assert_eq!(read("# Closed ##\n").paragraphs()[0].text(), "Closed");
    }

    /// Underscores inside a word are letters however many there are, and
    /// three stars are bold and italic at once.
    #[test]
    fn underscores_inside_a_word_are_letters_in_any_number() {
        let document = read("block__element__modifier a__b__c ***both*** end\n");
        let paragraph = document.paragraphs()[0];
        assert_eq!(
            paragraph.text(),
            "block__element__modifier a__b__c both end"
        );
        let emphasised: Vec<(String, bool, bool)> = paragraph
            .runs()
            .iter()
            .filter(|run| run.props.bold() || run.props.italic())
            .map(|run| (run.text(), run.props.bold(), run.props.italic()))
            .collect();
        assert_eq!(emphasised, [("both".to_owned(), true, true)]);
    }

    #[test]
    fn a_lone_asterisk_is_an_asterisk() {
        let document = read("2 * 3 = 6\n");
        assert_eq!(text_of(&document), ["2 * 3 = 6"]);
    }

    #[test]
    fn a_bulleted_list_becomes_a_numbered_paragraph() {
        let document = read("- one\n- two\n");
        let paragraphs = document.paragraphs();
        assert_eq!(paragraphs.len(), 2);
        let reference = paragraphs[0].props.numbering.expect("in a list");
        assert!(reference.is_numbered());
        assert_eq!(
            paragraphs[1].props.numbering.map(|r| r.num_id),
            Some(reference.num_id),
            "both items are in the *same* list"
        );
        let level = document
            .numbering
            .level(reference.num_id, 0)
            .expect("a level");
        assert_eq!(level.format, wp_model::NumFormat::Bullet);
    }

    #[test]
    fn a_numbered_list_is_numbered_rather_than_bulleted() {
        let document = read("1. first\n2. second\n");
        let reference = document.paragraphs()[0].props.numbering.expect("in a list");
        let level = document.numbering.level(reference.num_id, 0).unwrap();
        assert_eq!(level.format, wp_model::NumFormat::Decimal);
    }

    #[test]
    fn a_blank_line_ends_a_list() {
        let document = read("- one\n\n- two\n");
        let paragraphs = document.paragraphs();
        let first = paragraphs[0].props.numbering.unwrap().num_id;
        let second = paragraphs
            .iter()
            .rev()
            .find_map(|p| p.props.numbering)
            .unwrap()
            .num_id;
        assert_ne!(first, second, "two lists, not one");
    }

    #[test]
    fn a_code_fence_is_taken_whole_and_nothing_inside_it_is_markdown() {
        let document = read("before\n\n```\n# not a heading\n- not a bullet\n```\n\nafter\n");
        let texts = text_of(&document);
        assert!(texts.contains(&"# not a heading".to_string()));
        assert!(texts.contains(&"- not a bullet".to_string()));
        let paragraphs = document.paragraphs();
        let code = paragraphs
            .iter()
            .find(|p| p.text() == "# not a heading")
            .unwrap();
        assert!(code.props.style.is_some(), "in a code style");
        assert!(
            wp_model::outline::heading_level(code, &document.styles).is_none(),
            "and not a heading"
        );
    }

    #[test]
    fn a_document_survives_the_round_trip_through_markdown() {
        let source = "# Title\n\nSome **bold** text.\n\n- one\n- two\n";
        let document = read(source);
        let back = write(&document);
        let again = read(&back);
        assert_eq!(text_of(&document), text_of(&again));
        assert!(back.contains("# Title"), "{back}");
        assert!(back.contains("**bold**"), "{back}");
        assert!(back.contains("- one"), "{back}");
    }

    #[test]
    fn emphasis_markers_go_inside_the_spaces() {
        // `**bold ** word` is not emphasis at all as far as Markdown is
        // concerned, so a writer that puts the markers round the run's spaces
        // produces a file that reads back as plain text.
        let mut run = Run::of("bold ");
        run.props.toggles.set(Toggle::Bold, true);
        let paragraph = Paragraph {
            content: vec![Inline::Run(run), Inline::Run(Run::of("after"))],
            ..Paragraph::new()
        };
        assert_eq!(markers(&paragraph), "**bold** after");
    }

    #[test]
    fn plain_text_is_one_paragraph_per_line() {
        let document = read_plain("one\r\ntwo\r\nthree\r\n");
        assert_eq!(text_of(&document), ["one", "two", "three"]);
        assert_eq!(
            write_plain(&document, crate::encoding::LineEnding::Crlf),
            "one\r\ntwo\r\nthree"
        );
    }

    #[test]
    fn an_empty_file_is_still_a_document() {
        assert_eq!(read("").paragraphs().len(), 1);
        assert_eq!(read_plain("").paragraphs().len(), 1);
    }

    #[test]
    fn a_horizontal_rule_becomes_a_blank_paragraph_rather_than_three_dashes() {
        let document = read("above\n\n---\n\nbelow\n");
        let texts = text_of(&document);
        assert!(!texts.iter().any(|t| t.contains("---")), "{texts:?}");
        assert!(texts.contains(&"above".to_string()));
        assert!(texts.contains(&"below".to_string()));
    }
}
