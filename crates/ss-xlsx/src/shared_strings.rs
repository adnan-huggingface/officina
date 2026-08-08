//! The shared string table (`sharedStrings.xml`).
//!
//! Excel stores most cell text once here and references it by index, which is why
//! a 200k-row status column costs a few dozen bytes of text. We mirror that into
//! [`StringTable`], so the file's dedup and ours are the same shape.
//!
//! The rich-text structure inside an `<si>` (per-run fonts and colours) is *not*
//! modeled here. The part stays retained until the writer understands it, so the
//! formatting survives even though we currently read only the characters.

use quick_xml::events::Event;
use quick_xml::Reader;

use ss_model::{StrId, StringTable};

use crate::error::{xml_err, Result};
use crate::xml::{end_local_name, local_name, push_text};

/// Parses `sharedStrings.xml` into ids, positionally: entry *n* is `<si>` *n*.
pub(crate) fn parse(part: &str, data: &[u8], strings: &mut StringTable) -> Result<Vec<StrId>> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = true;

    let mut out: Vec<StrId> = Vec::new();
    let mut buf = Vec::new();

    // An `<si>` is either a bare `<t>` or a sequence of `<r>` runs each holding a
    // `<t>`. Both concatenate to the same string, so one accumulator serves both.
    let mut current = String::new();
    let mut in_si = false;

    // `<rPh>` holds furigana — a phonetic reading shown above the text, not part
    // of it. It contains a `<t>` exactly like a real run does, so a reader that
    // matches on `<t>` alone silently appends the pronunciation to every Japanese
    // cell it touches.
    let mut phonetic_depth = 0usize;

    // Only characters inside a `<t>` are the string. Everything else inside an
    // `<si>` — the indentation between `<r>` runs, `<rPr>` formatting elements —
    // is markup. A reader that accumulates all text inside `<si>` works perfectly
    // on Excel's own output, which has no whitespace, and then mangles every cell
    // of any file that has been pretty-printed on its way through a pipeline.
    let mut text_depth = 0usize;

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?
        {
            Event::Start(e) => match local_name(&e) {
                b"si" => {
                    in_si = true;
                    current.clear();
                }
                b"rPh" => phonetic_depth += 1,
                b"t" => text_depth += 1,
                _ => {}
            },
            Event::Empty(e) => {
                if local_name(&e) == b"si" {
                    // `<si/>` is a legitimately empty string, and it still occupies
                    // an index. Skipping it would shift every later cell's text.
                    out.push(strings.intern(""));
                }
            }
            Event::End(e) => match end_local_name(&e) {
                b"si" => {
                    if in_si {
                        out.push(strings.intern(&current));
                        in_si = false;
                    }
                }
                b"rPh" => phonetic_depth = phonetic_depth.saturating_sub(1),
                b"t" => text_depth = text_depth.saturating_sub(1),
                _ => {}
            },
            Event::Eof => break,
            ev => {
                if in_si && text_depth > 0 && phonetic_depth == 0 {
                    push_text(&mut current, &ev)?;
                }
            }
        }
        buf.clear();
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(xml: &str) -> (Vec<StrId>, StringTable) {
        let mut strings = StringTable::new();
        let ids = parse("sharedStrings.xml", xml.as_bytes(), &mut strings).expect("parses");
        (ids, strings)
    }

    fn texts(xml: &str) -> Vec<String> {
        let (ids, strings) = parse_all(xml);
        ids.into_iter()
            .map(|id| strings.resolve(id).to_owned())
            .collect()
    }

    #[test]
    fn plain_entries_read_in_order() {
        let got = texts(
            r#"<sst count="3" uniqueCount="3">
                 <si><t>Alpha</t></si>
                 <si><t>Beta</t></si>
                 <si><t>Gamma</t></si>
               </sst>"#,
        );
        assert_eq!(got, ["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn rich_text_runs_concatenate_to_one_string() {
        let got = texts(
            r#"<sst><si>
                 <r><rPr><b/></rPr><t>Bold</t></r>
                 <r><t> and plain</t></r>
               </si></sst>"#,
        );
        assert_eq!(got, ["Bold and plain"]);
    }

    #[test]
    fn phonetic_hints_are_not_part_of_the_text() {
        // A Japanese name written in kanji, with its kana reading in <rPh>.
        // Without the rPh guard this reads back as the name with the pronunciation
        // glued on the end.
        let xml = "<sst><si>\
             <t>\u{5C71}\u{7530}</t>\
             <rPh sb=\"0\" eb=\"2\"><t>\u{3084}\u{307E}\u{3060}</t></rPh>\
             <phoneticPr fontId=\"1\"/>\
             </si></sst>";
        assert_eq!(texts(xml), ["\u{5C71}\u{7530}"]);
    }

    #[test]
    fn indentation_between_runs_is_markup_not_text() {
        // Excel writes sharedStrings.xml with no whitespace, so this only bites on
        // files that have passed through a formatter — and then it bites every
        // multi-run cell in the workbook at once.
        let got = texts(
            "<sst>\n  <si>\n    <r><t>Bold</t></r>\n    <r><t> and plain</t></r>\n  </si>\n</sst>",
        );
        assert_eq!(got, ["Bold and plain"]);
    }

    #[test]
    fn an_empty_entry_still_occupies_its_index() {
        // Cells reference these by position; a dropped entry shifts all later text.
        let got = texts(r#"<sst><si><t>a</t></si><si/><si><t>c</t></si></sst>"#);
        assert_eq!(got, ["a", "", "c"]);
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn an_si_with_an_empty_t_is_the_empty_string() {
        let got = texts(r#"<sst><si><t></t></si><si><t/></si></sst>"#);
        assert_eq!(got, ["", ""]);
    }

    #[test]
    fn entities_and_preserved_whitespace_survive() {
        let got = texts(
            r#"<sst>
                 <si><t>R&amp;D</t></si>
                 <si><t xml:space="preserve">  padded  </t></si>
               </sst>"#,
        );
        assert_eq!(got, ["R&D", "  padded  "]);
    }

    #[test]
    fn namespace_prefixes_do_not_defeat_matching() {
        let got = texts(
            r#"<x:sst xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
                 <x:si><x:t>Prefixed</x:t></x:si>
               </x:sst>"#,
        );
        assert_eq!(got, ["Prefixed"]);
    }

    #[test]
    fn identical_entries_share_one_interned_string() {
        let (ids, strings) = parse_all(
            r#"<sst><si><t>Active</t></si><si><t>Active</t></si><si><t>Closed</t></si></sst>"#,
        );
        assert_eq!(ids.len(), 3, "index space keeps all three");
        assert_eq!(ids[0], ids[1], "but they intern to one id");
        assert_ne!(ids[0], ids[2]);
        assert_eq!(strings.len(), 3, "empty, Active, Closed");
    }
}
