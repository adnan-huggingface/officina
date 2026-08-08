//! Retitling a chart without disturbing anything else in it.
//!
//! `chartSpace` is the most elaborate part in a workbook — axis formatting,
//! gradients, 3-D rotation, trendlines, data labels, per-point overrides — and
//! almost none of it is modeled. So the title is spliced in place, exactly as a
//! cell is: the file's own bytes go back everywhere except inside `<c:tx>`.
//!
//! Adding a title to a chart that has none *does* mean authoring an element.
//! That is allowed for the same reason `sheet_out` may author a `<c:>`: it is a
//! structure we understand completely, and everything around it is retained.
//! The schema fixes `<c:title>` as the first child of `<c:chart>`, and
//! `<c:autoTitleDeleted val="1"/>` has to be cleared at the same time or Excel
//! hides the title we just wrote.

use quick_xml::events::Event;

use crate::error::Result;
use crate::write::splice::{close, escape_text, open, prefix_of, retag, Set, Splicer};
use crate::xml::{end_local_name, local_name};

/// Replaces the chart's title text, or removes it.
///
/// A title whose text comes from a cell (`<c:tx><c:strRef>`) is left alone:
/// changing it would mean writing to the cell, which is the user's to do.
pub(crate) fn retitle(part: &str, data: &[u8], title: Option<&str>) -> Result<Vec<u8>> {
    let found = scan(part, data)?;
    if found.from_cell {
        return Ok(data.to_vec());
    }
    // Three cases, not two. A chart may carry a `<c:title>` with no text in it
    // at all — Excel writes one holding only formatting when the automatic
    // title has been switched off — and there is then no run to write into. So
    // an existing title is *replaced wholesale* unless it has runs to edit.
    let author = title.is_some() && !found.has_runs;
    let discard = title.is_none() || author;

    let mut out = Vec::with_capacity(data.len() + 128);
    let mut splicer = Splicer::new(part, data);
    let mut prefix = Vec::new();
    // A guess, used only when the chart has no DrawingML element to learn the
    // real prefix from — which happens exactly when it has no title text yet.
    let mut drawing_prefix: Vec<u8> = b"a:".to_vec();
    // Nesting inside the title, so that a `<a:t>` belonging to a data label or
    // an axis is never mistaken for the title's.
    let mut in_title = 0usize;
    let mut in_run = 0usize;
    let mut written = false;

    while let Some((event, span)) = splicer.next()? {
        match &event {
            Event::Start(e) if local_name(e) == b"chartSpace" => {
                prefix = prefix_of(e);
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::Start(e) if local_name(e) == b"chart" => {
                out.extend_from_slice(splicer.bytes(span));
                // The schema fixes the position: first child of `<c:chart>`.
                if let Some(text) = title.filter(|_| author) {
                    write_title(&mut out, &prefix, &drawing_prefix, text);
                }
            }
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"title" => {
                if matches!(event, Event::Empty(_)) {
                    if !discard {
                        out.extend_from_slice(splicer.bytes(span));
                    }
                    continue;
                }
                in_title += 1;
                if !discard {
                    out.extend_from_slice(splicer.bytes(span));
                }
            }
            Event::End(e) if end_local_name(e) == b"title" => {
                in_title = in_title.saturating_sub(1);
                if !discard {
                    out.extend_from_slice(splicer.bytes(span));
                }
            }

            _ if in_title > 0 && discard => {}

            Event::Start(e) if in_title > 0 && local_name(e) == b"r" => {
                in_run += 1;
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::End(e) if in_title > 0 && end_local_name(e) == b"r" => {
                in_run = in_run.saturating_sub(1);
                out.extend_from_slice(splicer.bytes(span));
            }
            Event::Start(e) if in_run > 0 && local_name(e) == b"t" => {
                out.extend_from_slice(splicer.bytes(span));
                // The first run gets the whole title; the rest are emptied,
                // because a title split across three runs must not come back as
                // the new text three times.
                if let (Some(text), false) = (title, written) {
                    out.extend_from_slice(escape_text(text).as_bytes());
                    written = true;
                }
            }
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) if in_run > 0 => {
                // Dropped: the run's text is whatever was just written.
            }

            // Excel writes this when the user deletes a chart's automatic
            // title. Left at 1, the title we just wrote would not be shown.
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"autoTitleDeleted" => {
                let value = if title.is_some() { "0" } else { "1" };
                out.extend_from_slice(&retag(
                    e,
                    &[Set::to(b"val", value)],
                    matches!(event, Event::Empty(_)),
                ));
            }

            _ => out.extend_from_slice(splicer.bytes(span)),
        }
        // The drawing prefix is learnt from any DrawingML element in the file,
        // wherever it is: a data label's run will do just as well as a title's.
        if drawing_prefix == b"a:" {
            if let Event::Start(e) | Event::Empty(e) = &event {
                if matches!(local_name(e), b"bodyPr" | b"p" | b"r" | b"t") {
                    let seen = prefix_of(e);
                    if !seen.is_empty() {
                        drawing_prefix = seen;
                    }
                }
            }
        }
    }

    Ok(out)
}

fn write_title(out: &mut Vec<u8>, prefix: &[u8], drawing: &[u8], text: &str) {
    open(out, prefix, b"title", &[], false);
    open(out, prefix, b"tx", &[], false);
    open(out, prefix, b"rich", &[], false);
    open(out, drawing, b"bodyPr", &[], true);
    open(out, drawing, b"p", &[], false);
    open(out, drawing, b"r", &[], false);
    open(out, drawing, b"t", &[], false);
    out.extend_from_slice(escape_text(text).as_bytes());
    close(out, drawing, b"t");
    close(out, drawing, b"r");
    close(out, drawing, b"p");
    close(out, prefix, b"rich");
    close(out, prefix, b"tx");
    open(out, prefix, b"overlay", &[Set::to(b"val", "0")], true);
    close(out, prefix, b"title");
}

struct Found {
    has_title: bool,
    /// The title has at least one text run to write into. A `<c:title>` holding
    /// only formatting has none.
    has_runs: bool,
    /// The title's text comes from a cell rather than being typed.
    from_cell: bool,
}

fn scan(part: &str, data: &[u8]) -> Result<Found> {
    let mut splicer = Splicer::new(part, data);
    let mut out = Found {
        has_title: false,
        has_runs: false,
        from_cell: false,
    };
    let mut in_title = 0usize;
    while let Some((event, _)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) => match local_name(e) {
                b"title" => {
                    out.has_title = true;
                    if !matches!(event, Event::Empty(_)) {
                        in_title += 1;
                    }
                }
                b"r" if in_title > 0 => out.has_runs = true,
                b"strRef" | b"numRef" if in_title > 0 => out.from_cell = true,
                _ => {}
            },
            Event::End(e) if end_local_name(e) == b"title" => {
                in_title = in_title.saturating_sub(1);
            }
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART: &str = concat!(
        r#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart>"#,
        r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Old </a:t></a:r>"#,
        r#"<a:r><a:t>name</a:t></a:r></a:p></c:rich></c:tx></c:title>"#,
        r#"<c:autoTitleDeleted val="1"/>"#,
        r#"<c:plotArea><c:barChart><c:ser><c:dLbls><c:tx><c:rich><a:p><a:r><a:t>label</a:t>"#,
        r#"</a:r></a:p></c:rich></c:tx></c:dLbls></c:ser></c:barChart></c:plotArea>"#,
        r#"</c:chart></c:chartSpace>"#,
    );

    fn rewritten(source: &str, title: Option<&str>) -> String {
        String::from_utf8(retitle("chart1.xml", source.as_bytes(), title).expect("writes"))
            .expect("utf-8")
    }

    #[test]
    fn a_new_title_replaces_every_run_of_the_old_one() {
        // The old title is two runs. Writing the new text into each would give
        // "NewNew"; leaving the second alone would give "Newname".
        let out = rewritten(CHART, Some("New name"));
        assert!(out.contains("<a:t>New name</a:t>"), "{out}");
        assert!(!out.contains("Old "), "{out}");
        assert!(!out.contains(">name<"), "{out}");
    }

    #[test]
    fn a_data_label_is_not_a_title() {
        // `<c:tx><c:rich><a:t>` appears inside `<c:dLbls>` too, further down
        // the same file.
        let out = rewritten(CHART, Some("New name"));
        assert!(out.contains("<a:t>label</a:t>"), "{out}");
    }

    #[test]
    fn writing_a_title_clears_the_flag_that_would_hide_it() {
        let out = rewritten(CHART, Some("New name"));
        assert!(out.contains(r#"<c:autoTitleDeleted val="0"/>"#), "{out}");
    }

    #[test]
    fn a_title_element_with_no_text_in_it_is_replaced_rather_than_edited() {
        // Excel writes one of these when the automatic title is switched off:
        // formatting and nothing to write into. Editing the runs would find
        // none and silently do nothing.
        let empty = concat!(
            r#"<c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart>"#,
            r#"<c:title><c:overlay val="0"/></c:title><c:autoTitleDeleted val="1"/>"#,
            r#"<c:plotArea><c:barChart/></c:plotArea></c:chart></c:chartSpace>"#,
        );
        let out = rewritten(empty, Some("Added"));
        assert!(out.contains("<a:t>Added</a:t>"), "{out}");
        assert_eq!(out.matches("<c:title>").count(), 1, "{out}");
        assert!(out.contains(r#"<c:autoTitleDeleted val="0"/>"#), "{out}");
    }

    #[test]
    fn a_chart_with_no_title_gets_one_in_the_position_the_schema_fixes() {
        let bare = concat!(
            r#"<c:chartSpace xmlns:c="http://c" xmlns:a="http://a"><c:chart>"#,
            r#"<c:plotArea><c:barChart/></c:plotArea></c:chart></c:chartSpace>"#,
        );
        let out = rewritten(bare, Some("Added"));
        let title = out.find("<c:title>").expect("added");
        let plot = out.find("<c:plotArea>").expect("still there");
        assert!(title < plot, "first child of <c:chart>: {out}");
        assert!(out.contains("<a:t>Added</a:t>"), "{out}");
    }

    #[test]
    fn removing_a_title_removes_the_element_and_nothing_else() {
        let out = rewritten(CHART, None);
        assert!(!out.contains("<c:title>"), "{out}");
        assert!(out.contains(r#"<c:autoTitleDeleted val="1"/>"#), "{out}");
        assert!(
            out.contains("<a:t>label</a:t>"),
            "the data label stays: {out}"
        );
    }

    #[test]
    fn a_title_that_comes_from_a_cell_is_left_alone() {
        // Rewriting it would mean writing to the cell, which is the user's to do.
        let linked = concat!(
            r#"<c:chartSpace xmlns:c="http://c"><c:chart><c:title><c:tx><c:strRef>"#,
            r#"<c:f>Sheet1!$A$1</c:f></c:strRef></c:tx></c:title></c:chart></c:chartSpace>"#,
        );
        assert_eq!(rewritten(linked, Some("Nope")), linked);
    }

    #[test]
    fn a_title_split_across_runs_comes_back_as_one_run() {
        // Which is why the caller compares before it writes: retitling to the
        // same text is not a no-op, it is a normalization. Everything outside
        // `<c:tx>` is still byte for byte.
        let out = rewritten(CHART, Some("Old name"));
        assert!(out.contains("<a:t>Old name</a:t>"), "{out}");
        assert!(
            out.contains("<c:plotArea><c:barChart>"),
            "the rest is untouched: {out}"
        );
    }
}
