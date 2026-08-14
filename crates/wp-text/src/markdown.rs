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
                out.push_str(&text);
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

/// A paragraph's text with `**` and `*` back around its emphasised runs.
fn markers(paragraph: &Paragraph) -> String {
    let mut out = String::new();
    for run in paragraph.runs() {
        let text = run.text();
        if text.is_empty() {
            continue;
        }
        // The markers go *inside* the spaces: `**bold** word`, never `**bold **
        // word`, which Markdown does not read as emphasis at all.
        let lead: String = text.chars().take_while(|c| c.is_whitespace()).collect();
        let tail: String = text
            .chars()
            .rev()
            .take_while(|c| c.is_whitespace())
            .collect();
        let core = &text[lead.len()..text.len() - tail.len()];
        let mark = match (run.props.bold(), run.props.italic()) {
            (true, true) => "***",
            (true, false) => "**",
            (false, true) => "*",
            (false, false) => "",
        };
        out.push_str(&lead);
        if core.is_empty() {
            out.push_str(&tail);
            continue;
        }
        out.push_str(mark);
        out.push_str(core);
        out.push_str(mark);
        out.push_str(&tail);
    }
    out
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
    let text = rest.strip_prefix(' ')?;
    Some((hashes as u8, text.trim_end_matches('#').trim()))
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

/// Splits a line into runs at its emphasis markers.
fn spans(line: &str) -> Paragraph {
    let mut paragraph = Paragraph::new();
    let mut plain = String::new();
    let mut chars = line.char_indices().peekable();

    let push = |paragraph: &mut Paragraph, text: &str, bold: bool, italic: bool| {
        if text.is_empty() {
            return;
        }
        let mut run = Run::of(text);
        run.props.toggles.set(Toggle::Bold, bold);
        run.props.toggles.set(Toggle::Italic, italic);
        if !bold && !italic {
            run.props.toggles = Default::default();
        }
        paragraph.content.push(Inline::Run(run));
    };

    while let Some((at, c)) = chars.next() {
        if c != '*' && c != '_' {
            plain.push(c);
            continue;
        }
        let double = chars.peek().map(|(_, next)| *next) == Some(c);
        let marker = if double {
            chars.next();
            format!("{c}{c}")
        } else {
            c.to_string()
        };
        let rest = &line[at + marker.len()..];
        let Some(close) = rest.find(&marker) else {
            // A lone `*` is a literal asterisk, which is what Markdown does and
            // what a document full of `*` in the middle of sentences needs.
            plain.push_str(&marker);
            continue;
        };
        push(&mut paragraph, &plain, false, false);
        plain.clear();
        push(&mut paragraph, &rest[..close], double, !double);
        // Skip what was consumed.
        let consumed = at + marker.len() + close + marker.len();
        while let Some(&(index, _)) = chars.peek() {
            if index >= consumed {
                break;
            }
            chars.next();
        }
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
        .map(|paragraph| paragraph.text())
        .collect::<Vec<_>>()
        .join(ending.as_str())
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
