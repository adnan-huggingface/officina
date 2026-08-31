//! `<text:list-style>` and `<text:outline-style>` — how a document numbers
//! things.
//!
//! **A list is a structure here, not a property.** WordprocessingML numbers a
//! paragraph by hanging a `<w:numPr>` on it and leaves the nesting implicit in
//! the level number; ODF wraps the paragraphs in `<text:list>` and
//! `<text:list-item>` elements and leaves the level implicit in the nesting.
//! The two say the same thing from opposite ends, and the body reader is where
//! one becomes the other: it counts how deep it is and writes the count onto
//! the paragraph. This module is only the definitions.
//!
//! The other difference worth stating is that ODF does not spell out a level's
//! whole label. Where `<w:lvlText>` is `%1.%2.` outright, ODF gives a prefix, a
//! suffix, and a count of how many levels to show — and says the numbers
//! between are joined with a full stop. The label is assembled here rather than
//! at layout time, because the model's `Level::text` is the format string and
//! everything downstream already knows how to read one.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::numbering::{AbstractNum, Level, Num, NumFormat, Suffix};
use wp_model::prop::Justify;
use wp_model::units::Twips;
use wp_model::Numbering;

use crate::props;
use crate::styles::Styles;
use crate::xml::{attr_in, attr_length, end_local_name, local_name, skip_element};

/// Where a list style's numbering ended up, by the name the body calls it.
pub type Lists = HashMap<String, u32>;

/// Reads one `<text:list-style>`, whose start tag the caller has just seen.
pub fn list_style(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    styles: &mut Styles,
    numbering: &mut Numbering,
) {
    let name = attr_in(e, b"style", b"name").unwrap_or_default();
    let levels = levels(reader, b"list-style", styles);
    if name.is_empty() {
        return;
    }
    let id = define(numbering, levels);
    styles.lists.insert(name, id);
}

/// Reads `<text:outline-style>` — the numbering of the headings.
///
/// Returns nothing where the outline numbers nothing, which is the usual case:
/// a document writes all nine levels out with `style:num-format=""` to say that
/// its headings are not numbered, and giving those paragraphs a numbering would
/// put a counter in front of every heading in the document.
pub fn outline_style(
    reader: &mut Reader<&[u8]>,
    styles: &mut Styles,
    numbering: &mut Numbering,
) -> Option<u32> {
    let levels = levels(reader, b"outline-style", styles);
    let numbered = levels
        .iter()
        .flatten()
        .any(|level| level.format != NumFormat::None);
    match numbered {
        true => Some(define(numbering, levels)),
        false => None,
    }
}

fn define(numbering: &mut Numbering, levels: Vec<Option<Level>>) -> u32 {
    let (abstract_id, num_id) = numbering.free_ids();
    let mut definition = AbstractNum::new(abstract_id);
    for level in levels.into_iter().flatten() {
        definition.set_level(level);
    }
    numbering.insert_abstract(definition);
    numbering.insert_num(Num::new(num_id, abstract_id));
    num_id
}

/// The `<text:list-level-style-*>` children of a list or outline style.
fn levels(reader: &mut Reader<&[u8]>, end: &[u8], styles: &Styles) -> Vec<Option<Level>> {
    let mut levels: Vec<Option<Level>> = vec![None; wp_model::numbering::LEVELS];
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return levels,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                let kind = match name.as_slice() {
                    b"list-level-style-number" | b"outline-level-style" => Some(Kind::Number),
                    b"list-level-style-bullet" => Some(Kind::Bullet),
                    b"list-level-style-image" => Some(Kind::Image),
                    _ => None,
                };
                match kind {
                    Some(kind) => {
                        let level = one(reader, &e, kind, empty, styles);
                        if let Some(level) = level {
                            let at = level.index as usize;
                            if at < levels.len() {
                                levels[at] = Some(level);
                            }
                        }
                    }
                    None if !empty => skip_element(reader, &name),
                    None => {}
                }
            }
            Event::End(e) if end_local_name(&e) == end => return levels,
            Event::Eof => return levels,
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Bullet,
    Image,
}

fn one(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    kind: Kind,
    empty: bool,
    styles: &Styles,
) -> Option<Level> {
    // `text:level` counts from one; the model counts from zero, as its own
    // format does.
    let index = attr_in(e, b"text", b"level")?.trim().parse::<u8>().ok()?;
    let index = index.checked_sub(1)?;
    if index as usize >= wp_model::numbering::LEVELS {
        if !empty {
            let name = local_name(e).to_vec();
            skip_element(reader, &name);
        }
        return None;
    }
    let mut level = Level::new(index);
    level.format = match kind {
        Kind::Bullet | Kind::Image => NumFormat::Bullet,
        Kind::Number => format(attr_in(e, b"style", b"num-format").as_deref()),
    };
    level.start = attr_in(e, b"text", b"start-value")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1);
    level.text = match kind {
        Kind::Bullet => attr_in(e, b"text", b"bullet-char")
            .unwrap_or_default()
            .into(),
        Kind::Image => "".into(),
        Kind::Number => label(
            index,
            attr_in(e, b"style", b"num-prefix").as_deref(),
            attr_in(e, b"style", b"num-suffix").as_deref(),
            attr_in(e, b"text", b"display-levels")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(1),
            level.format == NumFormat::None,
        )
        .into(),
    };
    if !empty {
        let name = local_name(e).to_vec();
        children(reader, &name, &mut level, styles);
    }
    Some(level)
}

/// `<style:list-level-properties>` and the level's own text properties.
fn children(reader: &mut Reader<&[u8]>, end: &[u8], level: &mut Level, styles: &Styles) {
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"list-level-properties" => {
                        list_level_properties(&e, level);
                        if !empty {
                            alignment(reader, level);
                        }
                    }
                    b"text-properties" => {
                        props::text_properties(&e, &styles.faces, &mut level.run);
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    _ if !empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == end => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

/// The older way of stating a level's indent, which is still what a good many
/// producers write: a space before the label and a width for the label itself.
fn list_level_properties(e: &BytesStart<'_>, level: &mut Level) {
    if let Some(align) = attr_in(e, b"fo", b"text-align") {
        level.justify = match align.as_str() {
            "center" => Justify::Center,
            "end" | "right" => Justify::End,
            _ => Justify::Start,
        };
    }
    let before = attr_length(e, b"space-before");
    let width = attr_length(e, b"min-label-width");
    if before.is_some() || width.is_some() {
        let before = before.unwrap_or(Twips(0));
        let width = width.unwrap_or(Twips(0));
        // The two together are the same shape the other format states as an
        // indent and a hanging one: the text starts at their sum, and the label
        // hangs back by the label's own width.
        level.para.indent.start = Some(Twips(before.0 + width.0));
        level.para.indent.hanging = Some(width);
    }
}

/// `<style:list-level-label-alignment>` — the newer way, and the one Word's
/// ODF export writes.
fn alignment(reader: &mut Reader<&[u8]>, level: &mut Level) {
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                if name == b"list-level-label-alignment" {
                    if let Some(start) = attr_in(&e, b"fo", b"margin-left")
                        .as_deref()
                        .and_then(crate::xml::length)
                    {
                        level.para.indent.start = Some(start);
                    }
                    if let Some(indent) = attr_in(&e, b"fo", b"text-indent")
                        .as_deref()
                        .and_then(crate::xml::length)
                    {
                        match indent.0 < 0 {
                            true => level.para.indent.hanging = Some(Twips(-indent.0)),
                            false => level.para.indent.first_line = Some(indent),
                        }
                    }
                    level.suffix = match attr_in(&e, b"text", b"label-followed-by").as_deref() {
                        Some("space") => Suffix::Space,
                        Some("nothing") => Suffix::Nothing,
                        _ => Suffix::Tab,
                    };
                }
                if !empty {
                    skip_element(reader, &name);
                }
            }
            Event::End(e) if end_local_name(&e) == b"list-level-properties" => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

/// The label a level shows, as the format string the model keeps.
///
/// ODF never writes the string out. It gives a prefix, a suffix and a count of
/// how many levels to show, and says the numbers between are joined with a full
/// stop — so a second level showing two of them reads `%1.%2` before its own
/// suffix goes on the end.
fn label(index: u8, prefix: Option<&str>, suffix: Option<&str>, display: u8, none: bool) -> String {
    if none {
        return prefix.unwrap_or_default().to_string() + suffix.unwrap_or_default();
    }
    let display = display.max(1).min(index + 1);
    let first = index + 1 - display;
    let numbers: Vec<String> = (first..=index).map(|at| format!("%{}", at + 1)).collect();
    format!(
        "{}{}{}",
        prefix.unwrap_or_default(),
        numbers.join("."),
        suffix.unwrap_or_default()
    )
}

/// `style:num-format` is the shape of one number, spelled as an example of it.
fn format(text: Option<&str>) -> NumFormat {
    match text {
        None | Some("") => NumFormat::None,
        Some("1") => NumFormat::Decimal,
        Some("a") => NumFormat::LowerLetter,
        Some("A") => NumFormat::UpperLetter,
        Some("i") => NumFormat::LowerRoman,
        Some("I") => NumFormat::UpperRoman,
        Some(other) => NumFormat::Other(other.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(xml: &str) -> (Styles, Numbering) {
        let mut styles = Styles::default();
        let mut numbering = Numbering::new();
        let mut table = wp_model::StyleTable::new();
        let mut reader = Reader::from_str(xml);
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) if local_name(&e) == b"styles" => crate::styles::read(
                    &mut reader,
                    b"styles",
                    &mut table,
                    &mut styles,
                    &mut numbering,
                    &mut std::collections::HashMap::new(),
                ),
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        (styles, numbering)
    }

    #[test]
    fn a_numbered_level_assembles_the_label_odf_never_writes_out() {
        let (styles, numbering) = read(concat!(
            r#"<office:styles><text:list-style style:name="L1">"#,
            r#"<text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/>"#,
            r#"<text:list-level-style-number text:level="2" style:num-format="1" style:num-suffix="." text:display-levels="2"/>"#,
            r#"<text:list-level-style-number text:level="3" style:num-format="a" style:num-prefix="(" style:num-suffix=")"/>"#,
            r#"</text:list-style></office:styles>"#
        ));
        let id = styles.lists["L1"];
        assert_eq!(&*numbering.level(id, 0).unwrap().text, "%1.");
        assert_eq!(&*numbering.level(id, 1).unwrap().text, "%1.%2.");
        assert_eq!(&*numbering.level(id, 2).unwrap().text, "(%3)");
        assert_eq!(
            numbering.level(id, 2).unwrap().format,
            NumFormat::LowerLetter
        );
    }

    #[test]
    fn a_bullet_is_the_character_it_shows() {
        let (styles, numbering) = read(concat!(
            r#"<office:styles><text:list-style style:name="L2">"#,
            r#"<text:list-level-style-bullet text:level="1" text:bullet-char="&#8226;">"#,
            r#"<style:list-level-properties text:list-level-position-and-space-mode="label-alignment">"#,
            r#"<style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.25in" fo:margin-left="0.5in"/>"#,
            r#"</style:list-level-properties>"#,
            r#"<style:text-properties style:font-name="Symbol"/>"#,
            r#"</text:list-level-style-bullet></text:list-style></office:styles>"#
        ));
        let level = numbering.level(styles.lists["L2"], 0).expect("level one");
        assert_eq!(level.format, NumFormat::Bullet);
        assert_eq!(&*level.text, "\u{2022}");
        assert_eq!(level.para.indent.start, Some(Twips(720)));
        assert_eq!(level.para.indent.hanging, Some(Twips(360)));
        assert_eq!(level.suffix, Suffix::Tab);
    }

    /// The usual case, and the one that would put a counter in front of every
    /// heading in the document if it were read as numbering.
    #[test]
    fn an_outline_that_numbers_nothing_is_no_numbering_at_all() {
        let (styles, _) = read(concat!(
            r#"<office:styles><text:outline-style style:name="Outline">"#,
            r#"<text:outline-level-style text:level="1" style:num-format=""/>"#,
            r#"<text:outline-level-style text:level="2" style:num-format=""/>"#,
            r#"</text:outline-style></office:styles>"#
        ));
        assert_eq!(styles.outline, None);
    }

    #[test]
    fn an_outline_that_does_number_them_is_one() {
        let (styles, numbering) = read(concat!(
            r#"<office:styles><text:outline-style style:name="Outline">"#,
            r#"<text:outline-level-style text:level="1" style:num-format="I" style:num-suffix="."/>"#,
            r#"<text:outline-level-style text:level="2" style:num-format="1" text:display-levels="2"/>"#,
            r#"</text:outline-style></office:styles>"#
        ));
        let id = styles.outline.expect("the headings are numbered");
        assert_eq!(
            numbering.level(id, 0).unwrap().format,
            NumFormat::UpperRoman
        );
        assert_eq!(&*numbering.level(id, 1).unwrap().text, "%1.%2");
    }

    /// The older way of stating an indent, which is not the way the newer one
    /// states it and must not be read as though it were.
    #[test]
    fn a_level_stated_the_old_way_indents_the_same_distance() {
        let (styles, numbering) = read(concat!(
            r#"<office:styles><text:list-style style:name="L3">"#,
            r#"<text:list-level-style-number text:level="1" style:num-format="1">"#,
            r#"<style:list-level-properties text:space-before="0.25in" text:min-label-width="0.25in"/>"#,
            r#"</text:list-level-style-number></text:list-style></office:styles>"#
        ));
        let level = numbering.level(styles.lists["L3"], 0).expect("level one");
        assert_eq!(level.para.indent.start, Some(Twips(720)));
        assert_eq!(level.para.indent.hanging, Some(Twips(360)));
    }
}
