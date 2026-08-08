//! The shared string table, as a writer sees it.
//!
//! Reading turns `<si>` entries into interned ids and throws the ordering away,
//! because nothing downstream of the reader cares which index a string had.
//! A writer cares about very little else: a cell says `<v>7</v>`, and what makes
//! that "Total" is entry seven. So the table is read again at save time, in
//! order, and text the model has grown since is appended to the end. Existing
//! entries are never renumbered — every cell in the workbook points at them,
//! including the millions in sheets we are not otherwise touching.
//!
//! Rich text is why entries are matched by their characters. An `<si>` with
//! per-run bold survives untouched as long as some cell still holds its text;
//! typing the same text into a second cell points that cell at the same entry
//! and it inherits the formatting, which is what Excel's own dedup does.

use std::collections::HashMap;

use quick_xml::events::Event;

use ss_model::StringTable;

use crate::error::Result;
use crate::shared_strings;
use crate::write::splice::{escape_text, prefix_of, raw_attr, retag, Set, Splicer};
use crate::xml::{end_local_name, local_name, parse_u32};

/// The `<si>` entries of a workbook, and the ones a save has to add.
pub(crate) struct Sst {
    entries: Vec<String>,
    index: HashMap<String, u32>,
    /// How many entries the file had. Everything past this is ours.
    original: usize,
    /// False when the workbook has no shared-string part at all.
    present: bool,
}

impl Sst {
    /// The table of a workbook with no `sharedStrings.xml`.
    pub(crate) fn absent() -> Self {
        Sst {
            entries: Vec::new(),
            index: HashMap::new(),
            original: 0,
            present: false,
        }
    }

    pub(crate) fn read(part: &str, data: &[u8]) -> Result<Self> {
        // Parsed through the reader, into a table of its own: the ids are only
        // wanted for their order, and interning into the workbook's table would
        // be a mutation a save has no business making.
        let mut scratch = StringTable::new();
        let ids = shared_strings::parse(part, data, &mut scratch)?;
        let entries: Vec<String> = ids
            .iter()
            .map(|id| scratch.resolve(*id).to_string())
            .collect();

        let mut index = HashMap::with_capacity(entries.len());
        for (i, text) in entries.iter().enumerate() {
            // First occurrence wins, so a duplicated entry keeps pointing where
            // the file's own cells point.
            index.entry(text.clone()).or_insert(i as u32);
        }

        Ok(Sst {
            original: entries.len(),
            entries,
            index,
            present: true,
        })
    }

    /// What a text cell's `t` attribute has to say.
    pub(crate) fn cell_type(&self) -> &'static str {
        if self.present {
            "s"
        } else {
            // No table to point into, so the characters ride in the cell.
            "inlineStr"
        }
    }

    /// The entry for `text`, adding one if the table does not have it.
    ///
    /// `None` when there is no shared-string part; the caller writes the text
    /// inline instead of creating a part and the relationship it would need.
    pub(crate) fn intern(&mut self, text: &str) -> Option<u32> {
        if !self.present {
            return None;
        }
        if let Some(i) = self.index.get(text) {
            return Some(*i);
        }
        let i = self.entries.len() as u32;
        self.entries.push(text.to_string());
        self.index.insert(text.to_string(), i);
        Some(i)
    }

    /// The text at an index, for reading what a `t="s"` cell says.
    pub(crate) fn entry(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(String::as_str)
    }

    /// The entries this save added.
    pub(crate) fn added(&self) -> &[String] {
        &self.entries[self.original..]
    }
}

/// Rewrites `sharedStrings.xml` with the appended entries.
pub(crate) fn rewrite(part: &str, data: &[u8], sst: &Sst) -> Result<Vec<u8>> {
    let added = sst.added();
    let mut out = Vec::with_capacity(data.len() + added.len() * 24);
    let mut splicer = Splicer::new(part, data);
    let mut prefix = Vec::new();

    while let Some((event, span)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"sst" => {
                prefix = prefix_of(e);
                // `count` is the number of cells pointing here and `uniqueCount`
                // the number of entries. Only the second is something we know
                // exactly; the first is advisory, and is nudged by what we added
                // rather than recomputed from a scan of every sheet.
                let references = raw_attr(e, b"count")
                    .and_then(|a| parse_u32(&a.value))
                    .unwrap_or(0) as usize;
                let sets = [
                    Set::to(b"count", (references + added.len()).to_string()),
                    Set::to(b"uniqueCount", sst.entries.len().to_string()),
                ];
                let empty = matches!(event, Event::Empty(_));
                if empty && !added.is_empty() {
                    out.extend_from_slice(&retag(e, &sets, false));
                    push_entries(&mut out, added, &prefix);
                    out.extend_from_slice(b"</");
                    out.extend_from_slice(&prefix);
                    out.extend_from_slice(b"sst>");
                } else {
                    out.extend_from_slice(&retag(e, &sets, empty));
                }
            }
            Event::End(e) if end_local_name(e) == b"sst" => {
                push_entries(&mut out, added, &prefix);
                out.extend_from_slice(splicer.bytes(span));
            }
            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }

    Ok(out)
}

fn push_entries(out: &mut Vec<u8>, added: &[String], prefix: &[u8]) {
    let p = String::from_utf8_lossy(prefix).into_owned();
    for text in added {
        // Leading or trailing space is significant and would otherwise be at the
        // mercy of whatever reads the file next.
        let space = if text.trim() == text {
            ""
        } else {
            " xml:space=\"preserve\""
        };
        out.extend_from_slice(
            format!("<{p}si><{p}t{space}>{}</{p}t></{p}si>", escape_text(text)).as_bytes(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SST: &str = concat!(
        r#"<?xml version="1.0"?>"#,
        r#"<sst xmlns="http://x" count="4" uniqueCount="2"><si><t>Total</t></si>"#,
        r#"<si><r><rPr><b/></rPr><t>Bold</t></r></si></sst>"#,
    );

    fn table() -> Sst {
        Sst::read("sharedStrings.xml", SST.as_bytes()).expect("parses")
    }

    #[test]
    fn entries_keep_the_index_the_cells_already_point_at() {
        let mut sst = table();
        assert_eq!(sst.intern("Total"), Some(0));
        assert_eq!(sst.intern("Bold"), Some(1), "read through its runs");
        assert!(sst.added().is_empty());
    }

    #[test]
    fn a_new_string_is_appended_and_the_rest_is_untouched() {
        let mut sst = table();
        assert_eq!(sst.intern("Q3"), Some(2));
        let out = rewrite("sharedStrings.xml", SST.as_bytes(), &sst).expect("writes");
        let text = String::from_utf8(out).expect("utf-8");

        assert!(
            text.contains(r#"<si><r><rPr><b/></rPr><t>Bold</t></r></si>"#),
            "the rich-text entry survives verbatim: {text}"
        );
        assert!(text.ends_with("<si><t>Q3</t></si></sst>"), "{text}");
        assert!(text.contains(r#"uniqueCount="3""#), "{text}");
        assert!(text.contains(r#"count="5""#), "{text}");
    }

    #[test]
    fn adding_nothing_changes_nothing_but_the_counts_it_confirms() {
        let sst = table();
        let out = rewrite("sharedStrings.xml", SST.as_bytes(), &sst).expect("writes");
        assert_eq!(String::from_utf8(out).expect("utf-8"), SST);
    }

    #[test]
    fn significant_whitespace_is_declared_so_a_reader_keeps_it() {
        let mut sst = table();
        sst.intern("  padded ");
        let out = rewrite("sharedStrings.xml", SST.as_bytes(), &sst).expect("writes");
        let text = String::from_utf8(out).expect("utf-8");
        assert!(
            text.contains(r#"<t xml:space="preserve">  padded </t>"#),
            "{text}"
        );
    }

    #[test]
    fn a_workbook_with_no_table_puts_its_text_in_the_cell() {
        let mut sst = Sst::absent();
        assert_eq!(sst.cell_type(), "inlineStr");
        assert_eq!(sst.intern("anything"), None);
    }
}
