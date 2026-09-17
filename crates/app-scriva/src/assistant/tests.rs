//! The words, the tools and the proposals, against documents built here.

use super::*;
use wp_model::doc::Block;

/// A document with Scriva's own styles, holding `texts`, the first a
/// heading when `heading` says so.
fn document(texts: &[&str], heading: bool) -> Document {
    let mut document = crate::app::Scriva::new().document;
    document.body = texts
        .iter()
        .map(|text| Block::Paragraph(Paragraph::of(text)))
        .collect();
    if heading {
        let style = document.styles.lookup("Heading1");
        if let Some(Block::Paragraph(first)) = document.body.first_mut() {
            first.props.style = style;
        }
    }
    document
}

fn texts(document: &Document) -> Vec<String> {
    document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .collect()
}

fn styles(document: &Document) -> Vec<Option<StyleId>> {
    document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.props.style)
        .collect()
}

fn call(name: &str, input: Value) -> ToolCall {
    ToolCall {
        id: "call_1".into(),
        name: name.into(),
        input,
    }
}

fn at(paragraph: usize, offset: usize) -> Caret {
    Caret { paragraph, offset }
}

fn span(from: Caret, to: Caret) -> Selection {
    Selection {
        anchor: from,
        head: to,
    }
}

const FIRST: &str = "2026-09-17T10:00:00Z";
const SECOND: &str = "2026-09-17T10:00:01Z";

fn rewrite(
    document: &mut Document,
    history: &mut History,
    first: usize,
    last: usize,
    markdown: &str,
    date: &str,
) -> Done {
    run(
        document,
        history,
        &call(
            "replace_paragraphs",
            json!({"first": first, "last": last, "markdown": markdown}),
        ),
        &author_at(date),
    )
}

/// What the helper is sent: the size, every heading with its number, the
/// paragraphs the request is about with two on each side — numbered from
/// the document's first, as Markdown, as they read with their changes
/// accepted — the words selected, and the request. The whole document is
/// every paragraph; a caret with nothing selected is its paragraph.
#[test]
fn a_request_carries_the_scope_numbered_with_the_paragraphs_around_it_and_the_headings() {
    let mut word = document(
        &[
            "Title", "one", "two", "three", "four", "five", "six", "seven",
        ],
        true,
    );
    let heading2 = word.styles.lookup("Heading2");
    if let Block::Paragraph(paragraph) = &mut word.body[5] {
        paragraph.props.style = heading2;
    }
    // Paragraph 4's "three" has a word deleted: not shown.
    if let Block::Paragraph(paragraph) = &mut word.body[3] {
        paragraph.content.push(Inline::Revised {
            revision: wp_model::Revision::Deleted(Mark::new(9, "Someone")),
            content: vec![Inline::Run(wp_model::doc::Run::of(" gone"))],
        });
        paragraph.content.push(Inline::Run(wp_model::doc::Run {
            props: {
                let mut bold = RunProps::default();
                bold.toggles.set(Toggle::Bold, true);
                bold
            },
            ..wp_model::doc::Run::of(" strong")
        }));
    }
    let selection = span(at(3, 2), at(4, 2));
    let sent = request(
        &word,
        selection,
        About::Selection,
        "Improve the wording.",
        1234,
    );
    assert_eq!(
        sent,
        "The document has 8 paragraphs and 1,234 words.\n\
         Its headings:\n\
         [1] # Title\n\
         [6] ## five\n\
         \n\
         The request is about paragraphs 4 to 5, the selection. Here are paragraphs 2 to 7:\n\
         [2] one\n\
         [3] two\n\
         [4] three **strong**\n\
         [5] four\n\
         [6] ## five\n\
         [7] six\n\
         \n\
         The request: Improve the wording."
    );

    // A selection that only reaches the start of a paragraph is not about it.
    let sent = request(&word, span(at(4, 0), at(2, 0)), About::Selection, "Go.", 3);
    assert!(
        sent.contains("about paragraphs 3 to 4, the selection. Here are paragraphs 1 to 6:"),
        "{sent}"
    );

    // Within one paragraph, the words selected are said; at the start, the
    // shown paragraphs stop at the first.
    let sent = request(&word, span(at(1, 0), at(1, 2)), About::Selection, "Why?", 3);
    assert!(
        sent.contains("about paragraph 2, the selection. Here are paragraphs 1 to 4:"),
        "{sent}"
    );
    assert!(
        sent.contains("The words selected: \u{201c}on\u{201d}"),
        "{sent}"
    );

    // A caret alone is its paragraph; at the end, the shown ones stop at the
    // last.
    let sent = request(
        &word,
        Selection::at(at(7, 1)),
        About::Selection,
        "Fix it.",
        3,
    );
    assert!(
        sent.contains("about paragraph 8, where the caret is. Here are paragraphs 6 to 8:"),
        "{sent}"
    );
    assert!(!sent.contains("words selected"));

    // The whole document is every paragraph.
    let sent = request(
        &word,
        Selection::at(at(0, 0)),
        About::Document,
        "Summarize it.",
        9,
    );
    assert!(sent.contains("The request is about the whole document.\nHere are paragraphs 1 to 8:\n[1] # Title\n[2] one"), "{sent}");
    assert!(
        sent.ends_with("[8] seven\n\nThe request: Summarize it."),
        "{sent}"
    );

    // A document with no heading says so, and one paragraph is "is".
    let plain = document(&["alone"], false);
    let sent = request(&plain, Selection::at(at(0, 0)), About::Paragraph, "Go.", 1);
    assert!(
        sent.starts_with("The document has 1 paragraph and 1 word.\nIt has no headings.\n"),
        "{sent}"
    );
    assert!(sent.contains("Here is paragraph 1:\n[1] alone"), "{sent}");

    // What leaves the computer, in words.
    assert_eq!(
        leaves(About::Document, 1234),
        "The whole document, 1,234 words"
    );
    assert!(leaves(About::Selection, 0).starts_with("The selected paragraphs"));
}

/// Every tool's schema is one a helper can be held to strictly; reading
/// answers with the paragraphs numbered, says what it left out past its
/// limit, and a range the document does not have is an error that names
/// the range it does.
#[test]
fn the_four_tools_are_strict_and_reading_answers_with_the_paragraphs_named() {
    let tools = tools();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "read_paragraphs",
            "replace_paragraphs",
            "insert_paragraphs",
            "comment"
        ]
    );
    for tool in &tools {
        assert!(tool.is_strict(), "{} is strict", tool.name);
        let required = tool.schema["required"].as_array().expect("required");
        let properties = tool.schema["properties"].as_object().expect("properties");
        assert_eq!(
            required.len(),
            properties.len(),
            "{}: all required",
            tool.name
        );
    }
    assert_eq!(setup().tools, tools);
    assert_eq!(setup().system, ::assist::prompt::scriva());

    let mut word = document(&["Title", "one", "two"], true);
    let before = word.clone();
    let mut history = History::new();
    let read = |word: &mut Document, history: &mut History, first: i64, last: i64| {
        run(
            word,
            history,
            &call("read_paragraphs", json!({"first": first, "last": last})),
            &author_at(FIRST),
        )
    };
    let done = read(&mut word, &mut history, 2, 3);
    assert_eq!(done.result.content, "[2] one\n[3] two");
    assert!(!done.result.is_error);
    assert_eq!(done.line.as_deref(), Some("read paragraphs 2\u{2013}3"));
    assert_eq!(done.proposal, None);
    let done = read(&mut word, &mut history, 1, 1);
    assert_eq!(done.result.content, "[1] # Title");
    assert_eq!(done.line.as_deref(), Some("read paragraph 1"));

    for (first, last) in [(0, 1), (2, 4), (3, 2), (-1, 2)] {
        let done = read(&mut word, &mut history, first, last);
        assert!(done.result.is_error, "{first}..{last}");
        assert_eq!(done.line, None);
        if first >= 0 {
            assert!(
                done.result.content.contains("numbered 1 to 3"),
                "{}",
                done.result.content
            );
        }
    }
    let done = run(
        &mut word,
        &mut history,
        &call("delete_everything", json!({})),
        &author_at(FIRST),
    );
    assert!(done.result.is_error);
    assert!(done
        .result
        .content
        .contains("no tool called delete_everything"));
    assert_eq!(word.body, before.body, "reading changes nothing");
    assert!(!history.can_undo());

    // A long reading stops at its limit and says what it left out.
    let many: Vec<String> = (1..=MOST_READ + 5).map(|n| format!("p{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    let mut long = document(&many, false);
    let done = read(&mut long, &mut history, 1, (MOST_READ + 5) as i64);
    let shown: Vec<&str> = done.result.content.lines().collect();
    assert_eq!(shown.len(), MOST_READ + 1);
    assert_eq!(
        shown.last().copied(),
        Some(
            format!(
                "(Paragraphs {} to {} not shown: ask for them.)",
                MOST_READ + 1,
                MOST_READ + 5
            )
            .as_str()
        )
    );
}

/// A heading rewritten without a `#` is still a heading; the words the
/// helper marks `**` are bold and the rest take the paragraph's plain
/// words' formatting; a `#` line is a heading of its level.
#[test]
fn a_rewritten_heading_is_still_a_heading_and_its_bold_words_are_still_bold() {
    let mut word = document(&["Title words", "Body words"], true);
    let (heading1, heading2) = (
        word.styles.lookup("Heading1"),
        word.styles.lookup("Heading2"),
    );
    // The body paragraph is in a face of its own.
    let mut face = RunProps::default();
    face.fonts.ascii = Some("Carlito".into());
    if let Block::Paragraph(paragraph) = &mut word.body[1] {
        paragraph.content = vec![Inline::Run(wp_model::doc::Run {
            props: face.clone(),
            ..wp_model::doc::Run::of("Body words")
        })];
    }
    let mut history = History::new();
    let done = rewrite(
        &mut word,
        &mut history,
        1,
        2,
        "A better title\nBody with **strong** words",
        FIRST,
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), ["A better title", "Body with strong words"]);
    assert_eq!(styles(&word), [heading1, None]);
    let runs: Vec<(String, bool, Option<String>)> = word.paragraphs()[1]
        .runs()
        .iter()
        .map(|run| {
            (
                run.text(),
                run.props.bold(),
                run.props.fonts.ascii.as_deref().map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(
        runs,
        [
            ("Body with ".to_owned(), false, Some("Carlito".to_owned())),
            ("strong".to_owned(), true, Some("Carlito".to_owned())),
            (" words".to_owned(), false, Some("Carlito".to_owned())),
        ]
    );

    // A `#` line makes a heading of its level, and a paragraph after a
    // heading that has no old one to follow takes the style after it.
    let mut word = document(&["Title", "Body"], true);
    let done = rewrite(
        &mut word,
        &mut history,
        1,
        1,
        "Title\nA sentence\n## Section",
        FIRST,
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), ["Title", "A sentence", "Section", "Body"]);
    let normal = word.styles.lookup("Normal");
    assert_eq!(styles(&word), [heading1, normal, heading2, None]);

    // A heading line first, in place of a body paragraph, and a line after it.
    let mut word = document(&["Title", "Body"], true);
    let done = rewrite(&mut word, &mut history, 2, 2, "## Part\ntext", FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), ["Title", "Part", "text"]);
    assert_eq!(styles(&word), [heading1, heading2, None]);

    // A body paragraph made a heading, alone, keeps its old look on reject,
    // its mark's own formatting included.
    let mut word = document(&["Title", "Body"], true);
    if let Block::Paragraph(body) = &mut word.body[1] {
        let mut bold = RunProps::default();
        bold.toggles.set(Toggle::Bold, true);
        body.props.mark = Some(Box::new(bold));
    }
    let before = word.clone();
    rewrite(&mut word, &mut history, 2, 2, "# Body", FIRST);
    assert_eq!(styles(&word), [heading1, heading1]);
    settle(&mut word, &mut history, FIRST, Resolve::Reject);
    assert_eq!(word.body, before.body);
}

/// Reject gives the paragraphs back exactly as they were; one Undo takes the
/// whole proposal away, and Redo brings it back; and the numbers the helper
/// is told move are the numbers that moved.
#[test]
fn reject_puts_the_paragraph_back_and_undo_takes_the_proposal_away_entirely() {
    let original = document(&["Title words", "The thing about it is this.", "End"], true);
    for how in [Resolve::Reject, Resolve::Accept] {
        let mut word = original.clone();
        let mut history = History::new();
        let done = rewrite(
            &mut word,
            &mut history,
            2,
            2,
            "It is this.\nAnd that.",
            FIRST,
        );
        assert_eq!(
            done.result.content,
            "Proposed: the person sees it as a tracked change, and accepts or rejects it. \
             The paragraphs after 2 are now numbered 2 higher."
        );
        assert_eq!(
            done.line.as_deref(),
            Some("proposed new wording for paragraph 2")
        );
        let proposal = done.proposal.expect("a proposal");
        assert_eq!(proposal.kind, Kind::Changes(FIRST.into()));
        assert_eq!(proposal.title, "Paragraph 2");
        assert_eq!(proposal.body, "It is this.\nAnd that.");
        // The old paragraph stands where it was, struck, with the two new
        // ones after it: what was paragraph 3 is paragraph 5.
        assert_eq!(texts(&word)[4], "End");
        let proposed = word.body.clone();

        // One Undo, and it is gone; Redo, and it is back.
        history.undo(&mut word);
        assert_eq!(word.body, original.body, "undone whole");
        history.redo(&mut word);
        assert_eq!(word.body, proposed, "redone whole");

        let settled = settle(&mut word, &mut history, FIRST, how);
        assert!(settled > 0);
        assert!(revise::tracked(&word).is_empty(), "{how:?}: nothing left");
        match how {
            Resolve::Reject => assert_eq!(word.body, original.body, "rejected exactly"),
            Resolve::Accept => {
                assert_eq!(
                    texts(&word),
                    ["Title words", "It is this.", "And that.", "End"]
                )
            }
        }
        // Settling is one step too.
        history.undo(&mut word);
        assert_eq!(word.body, proposed, "{how:?}: the settling undone");
    }
}

/// Three paragraphs rewritten as one, and one as three: accepted, the new
/// paragraphs; rejected, the old, with their styles.
#[test]
fn a_rewrite_into_more_or_fewer_paragraphs_accepts_to_the_new_ones_and_rejects_to_the_old() {
    let original = document(&["Title", "one", "two", "three", "End"], true);
    let heading = original.styles.lookup("Heading1");
    for (first, last, markdown, accepted) in [
        (2, 4, "all in one", vec!["Title", "all in one", "End"]),
        (
            2,
            2,
            "a\nb\nc",
            vec!["Title", "a", "b", "c", "two", "three", "End"],
        ),
        (
            1,
            3,
            "New title\nboth",
            vec!["New title", "both", "three", "End"],
        ),
    ] {
        for how in [Resolve::Accept, Resolve::Reject] {
            let mut word = original.clone();
            let mut history = History::new();
            let done = rewrite(&mut word, &mut history, first, last, markdown, FIRST);
            assert!(!done.result.is_error, "{}", done.result.content);
            settle(&mut word, &mut history, FIRST, how);
            assert!(revise::tracked(&word).is_empty(), "{markdown:?} {how:?}");
            match how {
                Resolve::Accept => {
                    assert_eq!(texts(&word), accepted, "{markdown:?} accepted");
                    assert_eq!(styles(&word)[0], heading, "{markdown:?}: the title's style");
                    assert!(
                        styles(&word)[1..].iter().all(|style| style != &heading),
                        "{markdown:?}: {:?}",
                        styles(&word)
                    );
                }
                Resolve::Reject => {
                    assert_eq!(word.body, original.body, "{markdown:?} rejected");
                }
            }
        }
    }
}

/// New paragraphs, after one or before the first, and paragraphs taken
/// out — the last ones included, whose mark cannot go — are proposals that
/// accept to what was asked and reject to what was there.
#[test]
fn paragraphs_inserted_or_taken_out_are_proposals_too() {
    let original = document(&["Title", "one", "two"], true);
    let heading = original.styles.lookup("Heading1");
    let normal = original.styles.lookup("Normal");
    let insert = |word: &mut Document, history: &mut History, after: usize, markdown: &str| {
        run(
            word,
            history,
            &call(
                "insert_paragraphs",
                json!({"after": after, "markdown": markdown}),
            ),
            &author_at(FIRST),
        )
    };
    for (after, markdown, accepted, looks) in [
        (
            1,
            "new a\nnew b",
            vec!["Title", "new a", "new b", "one", "two"],
            vec![heading, normal, normal, None, None],
        ),
        (
            0,
            "first\n# Before",
            vec!["first", "Before", "Title", "one", "two"],
            vec![None, heading, heading, None, None],
        ),
        (
            3,
            "last",
            vec!["Title", "one", "two", "last"],
            vec![heading, None, None, None],
        ),
    ] {
        for how in [Resolve::Accept, Resolve::Reject] {
            let mut word = original.clone();
            let mut history = History::new();
            let done = insert(&mut word, &mut history, after, markdown);
            assert!(!done.result.is_error, "{}", done.result.content);
            let proposal = done.proposal.expect("a proposal");
            assert_eq!(proposal.kind, Kind::Changes(FIRST.into()));
            history.undo(&mut word);
            assert_eq!(word.body, original.body, "{markdown:?}: one step");
            history.redo(&mut word);
            settle(&mut word, &mut history, FIRST, how);
            assert!(revise::tracked(&word).is_empty());
            match how {
                Resolve::Accept => {
                    assert_eq!(texts(&word), accepted, "{markdown:?}");
                    assert_eq!(styles(&word), looks, "{markdown:?}");
                }
                Resolve::Reject => assert_eq!(word.body, original.body, "{markdown:?}"),
            }
        }
    }
    let done = insert(&mut original.clone(), &mut History::new(), 4, "x");
    assert!(done.result.is_error);
    let done = insert(&mut original.clone(), &mut History::new(), 1, "   ");
    assert!(done.result.is_error, "nothing to put in");

    // Taken out: from the middle, at the end, and the whole document.
    for (first, last, accepted) in [
        (2, 2, vec!["Title", "two"]),
        (2, 3, vec!["Title"]),
        (1, 3, vec![""]),
    ] {
        for how in [Resolve::Accept, Resolve::Reject] {
            let mut word = original.clone();
            let mut history = History::new();
            let done = rewrite(&mut word, &mut history, first, last, "", FIRST);
            assert!(!done.result.is_error, "{}", done.result.content);
            assert_eq!(
                done.proposal.as_ref().map(|p| p.body.as_str()),
                Some("Taken out.")
            );
            assert_eq!(
                texts(&word).len(),
                3,
                "nothing is gone until it is accepted"
            );
            settle(&mut word, &mut history, FIRST, how);
            assert!(revise::tracked(&word).is_empty());
            match how {
                Resolve::Accept => assert_eq!(texts(&word), accepted, "{first}..{last}"),
                Resolve::Reject => assert_eq!(word.body, original.body, "{first}..{last}"),
            }
        }
    }
    // The title keeps its style when what follows it is taken out, and the
    // paragraph holding the struck words looks like the title meanwhile, so
    // that the page does not change shape while the proposal stands.
    let mut word = original.clone();
    rewrite(&mut word, &mut History::new(), 2, 3, "", FIRST);
    assert_eq!(
        styles(&word),
        [heading, None, heading],
        "the paragraph that survives the join already looks like the title"
    );
    settle(&mut word, &mut History::new(), FIRST, Resolve::Accept);
    assert_eq!(styles(&word), [heading]);
}

/// Taking out a paragraph where there is no neighbour to join it to — before
/// a table, inside a cell, after an empty paragraph — proposes exactly that
/// paragraph, and taking out what is not there is refused.
#[test]
fn a_paragraph_taken_out_beside_a_table_or_an_empty_one_is_still_one_proposal() {
    let cell = |text: &str| {
        Block::Table(wp_model::table::Table {
            rows: vec![wp_model::table::Row {
                cells: vec![wp_model::table::Cell {
                    props: wp_model::table::CellProps::new(),
                    content: vec![Block::Paragraph(Paragraph::of(text))],
                }],
                ..wp_model::table::Row::new()
            }],
            ..wp_model::table::Table::new()
        })
    };
    let body = |blocks: Vec<Block>| {
        let mut word = document(&["x"], false);
        word.body = blocks;
        word
    };
    let table_document = || {
        body(vec![
            Block::Paragraph(Paragraph::of("before")),
            cell("in the cell"),
            Block::Paragraph(Paragraph::of("after")),
        ])
    };
    // 1: before a table. 2: the cell's only paragraph. 3: after a table.
    for (number, accepted) in [
        (1, vec!["", "in the cell", "after"]),
        (2, vec!["before", "", "after"]),
        (3, vec!["before", "in the cell", ""]),
    ] {
        for how in [Resolve::Accept, Resolve::Reject] {
            let mut word = table_document();
            let original = word.clone();
            let mut history = History::new();
            let done = rewrite(&mut word, &mut history, number, number, "", FIRST);
            assert!(!done.result.is_error, "{number}: {}", done.result.content);
            assert!(
                done.result.content.contains("the empty paragraph stays"),
                "{number}: {}",
                done.result.content
            );
            assert_eq!(texts(&word).len(), 3, "{number}: nothing gone yet");
            history.undo(&mut word);
            assert_eq!(word.body, original.body, "{number}: one step");
            history.redo(&mut word);
            settle(&mut word, &mut history, FIRST, how);
            assert!(revise::tracked(&word).is_empty(), "{number}");
            assert_eq!(
                word.body
                    .iter()
                    .filter(|b| matches!(b, Block::Table(_)))
                    .count(),
                1,
                "{number}: the table stands either way"
            );
            match how {
                Resolve::Accept => assert_eq!(texts(&word), accepted, "{number}"),
                Resolve::Reject => assert_eq!(word.body, original.body, "{number}"),
            }
        }
    }

    // With a neighbour to join, the paragraph goes whole and nothing is said.
    let mut word = document(&["one", "two", "three"], false);
    let done = rewrite(&mut word, &mut History::new(), 2, 2, "", FIRST);
    assert!(!done.result.content.contains("empty paragraph"));

    // The last paragraph, taken out after an empty one: there is no next
    // paragraph to take the text, so the mark joins backwards. Backspace
    // keeps the look of the paragraph before only after text, so what is
    // left is restyled to look like it all the same.
    let mut word = document(&["", "gone"], true);
    let heading = word.styles.lookup("Heading1");
    let mut history = History::new();
    let done = rewrite(&mut word, &mut history, 2, 2, "", FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    assert_eq!(styles(&word), [heading, heading], "while it is open");
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), [""]);
    assert_eq!(styles(&word), [heading]);

    // With a paragraph after it, the one taken out gives its text to nobody:
    // what follows is left exactly as it was.
    let mut word = document(&["", "gone", "after"], true);
    let mut history = History::new();
    let done = rewrite(&mut word, &mut history, 2, 2, "", FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), ["", "after"]);
    assert_eq!(styles(&word), [heading, None]);

    // Nothing to take out is a refusal, and leaves the document alone.
    let mut word = document(&[""], false);
    let original = word.clone();
    let mut history = History::new();
    let done = rewrite(&mut word, &mut history, 1, 1, "", FIRST);
    assert!(done.result.is_error);
    assert_eq!(word.body, original.body);
    assert!(!history.can_undo());
}

/// A paragraph holding what its line cannot show — a picture, a note, a
/// field, an equation, a link, a page break — is not rewritten: the words
/// would land and the rest would go. The request says which paragraphs those
/// are, and a line break inside a paragraph is `<br>`, so that its text
/// cannot look like another numbered line.
#[test]
fn a_paragraph_holding_what_its_line_cannot_show_is_not_rewritten() {
    use wp_model::doc::{Break, Drawing, Run};
    let holders: Vec<(&str, Inline)> = vec![
        (
            "a picture",
            Inline::Run(Run {
                content: vec![Piece::Drawing(Box::new(Drawing {
                    source: Vec::new().into(),
                    source_format: wp_model::SourceFormat::Authored,
                    anchored: false,
                    extent: (wp_model::Emu(914_400), wp_model::Emu(457_200)),
                    rel: Some("rId9".into()),
                    chart: None,
                    name: None,
                    description: None,
                    wrap: wp_model::doc::Wrap::None,
                    distance: Default::default(),
                    position: None,
                    behind_text: false,
                    text: None,
                    tone: None,
                    outline: None,
                }))],
                ..Run::new()
            }),
        ),
        (
            "a footnote or endnote",
            Inline::Run(Run {
                content: vec![Piece::FootnoteRef {
                    id: 1,
                    custom_mark: false,
                }],
                ..Run::new()
            }),
        ),
        (
            "a field",
            Inline::Run(Run {
                content: vec![
                    Piece::FieldStart {
                        dirty: false,
                        lock: false,
                    },
                    Piece::Instruction(" PAGE ".into()),
                    Piece::FieldSeparate,
                    Piece::Text("2".into()),
                    Piece::FieldEnd,
                ],
                ..Run::new()
            }),
        ),
        (
            "a page or column break",
            Inline::Run(Run {
                content: vec![Piece::Break(Break::Page)],
                ..Run::new()
            }),
        ),
        (
            "an equation",
            Inline::Math(Box::new(wp_model::doc::MathBlob {
                source: std::sync::Arc::from(&b"<m:oMath/>"[..]),
                text: "x+y".into(),
            })),
        ),
        (
            "a link",
            Inline::Hyperlink(Box::new(wp_model::Hyperlink {
                rel: None,
                anchor: Some("x".into()),
                tooltip: None,
                history: true,
                content: vec![Inline::Run(Run::of("linked"))],
            })),
        ),
    ];
    for (what, inline) in holders {
        let mut word = document(&["Title", "text and", "after"], true);
        if let Block::Paragraph(paragraph) = &mut word.body[1] {
            paragraph.content.push(inline);
        }
        let before = word.clone();
        let mut history = History::new();
        let done = rewrite(&mut word, &mut history, 2, 2, "new words", FIRST);
        assert!(done.result.is_error, "{what}");
        assert!(
            done.result.content.contains(what),
            "{what}: {}",
            done.result.content
        );
        assert_eq!(word.body, before.body, "{what}");
        assert!(!history.can_undo(), "{what}");
        // And the request says so, so that the helper comments instead.
        let sent = request(
            &word,
            Selection::at(at(1, 0)),
            About::Paragraph,
            "Fix it.",
            4,
        );
        assert!(
            sent.contains("Paragraph 2 holds a picture, a note, a field"),
            "{what}: {sent}"
        );
    }

    // A line break is `<br>`, and the words after it are not a line of their
    // own — a paragraph cannot pass itself off as another paragraph.
    let mut word = document(&["Title", "first", "after"], true);
    if let Block::Paragraph(paragraph) = &mut word.body[1] {
        if let Some(Inline::Run(run)) = paragraph.content.first_mut() {
            run.content.push(Piece::Break(Break::Line));
            run.content.push(Piece::Text("[9] second".into()));
        }
    }
    let sent = request(
        &word,
        Selection::at(at(1, 0)),
        About::Paragraph,
        "Fix it.",
        4,
    );
    assert!(sent.contains("[2] first<br>\\[9] second"), "{sent}");
    assert_eq!(
        sent.lines().filter(|line| line.starts_with('[')).count(),
        4,
        "three paragraphs and a heading, each one line: {sent}"
    );
    let mut history = History::new();
    let done = rewrite(&mut word, &mut history, 2, 2, "one<br>two", FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), ["Title", "one\ntwo", "after"]);
}

/// A formatting change someone else has open is not changed again: a
/// paragraph remembers the properties it had before one such change, and not
/// before two, so a proposal that would restyle it is refused whole.
#[test]
fn a_proposal_leaves_a_formatting_change_someone_else_has_open_alone() {
    let changed = |word: &mut Document, index: usize| {
        if let Block::Paragraph(paragraph) = &mut word.body[index] {
            paragraph.prop_change = Some(Box::new(wp_model::PropChange {
                mark: Mark::new(90, "Adnan Khan"),
                previous: wp_model::revision::PreviousProps::Paragraph(Box::default()),
            }));
        }
    };
    // The last paragraph of the document, restyled by a rewrite: refused.
    let mut word = document(&["Title", "Body"], true);
    changed(&mut word, 1);
    let before = word.clone();
    let mut history = History::new();
    let done = rewrite(&mut word, &mut history, 2, 2, "# Body", FIRST);
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("formatting is still open"),
        "{}",
        done.result.content
    );
    assert_eq!(word.body, before.body);
    assert!(!history.can_undo());

    // New paragraphs after it, which would take its mark over: refused too.
    let done = run(
        &mut word,
        &mut history,
        &call(
            "insert_paragraphs",
            json!({"after": 2, "markdown": "a sentence"}),
        ),
        &author_at(FIRST),
    );
    assert!(done.result.is_error);
    assert_eq!(word.body, before.body);

    // Anywhere but the end, the paragraph is struck whole and the new one
    // stands beside it: the change it has open is left as it is, and either
    // answer is exact.
    let mut word = document(&["Title", "Body", "End"], true);
    changed(&mut word, 1);
    let before = word.clone();
    let done = rewrite(&mut word, &mut history, 2, 2, "# Body", FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    let heading = word.styles.lookup("Heading1");
    assert!(
        revise::tracked(&word)
            .iter()
            .any(|change| &*change.mark.author == "Adnan Khan"),
        "the person's formatting change is left as it was"
    );
    let mut accepted = word.clone();
    settle(&mut accepted, &mut History::new(), FIRST, Resolve::Accept);
    assert_eq!(texts(&accepted), ["Title", "Body", "End"]);
    assert_eq!(styles(&accepted)[1], heading, "the new paragraph's style");
    assert!(
        revise::tracked(&accepted).is_empty(),
        "the struck paragraph took its own formatting change with it"
    );
    settle(&mut word, &mut history, FIRST, Resolve::Reject);
    assert_eq!(word.body, before.body, "rejected exactly");

    // New paragraphs before the next one change neither neighbour.
    let mut word = document(&["Title", "Body", "End"], true);
    changed(&mut word, 1);
    let before = word.clone();
    let done = run(
        &mut word,
        &mut history,
        &call(
            "insert_paragraphs",
            json!({"after": 2, "markdown": "a sentence"}),
        ),
        &author_at(FIRST),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    let mut accepted = word.clone();
    settle(&mut accepted, &mut History::new(), FIRST, Resolve::Accept);
    assert_eq!(texts(&accepted), ["Title", "Body", "a sentence", "End"]);
    assert_eq!(
        revise::tracked(&accepted).len(),
        1,
        "still only the person's own: {:?}",
        revise::tracked(&accepted)
    );
    settle(&mut word, &mut history, FIRST, Resolve::Reject);
    assert_eq!(word.body, before.body, "rejected exactly");
}

/// A heading of a level the document has no style for takes the nearest
/// level it has, and the helper is told; a document with no heading styles
/// gets ordinary paragraphs, and is told that. A line after a heading is
/// body text even where the heading's style names nothing to follow it.
#[test]
fn headings_go_as_deep_as_the_documents_own_styles_do() {
    let mut word = document(&["Title", "Body"], true);
    let mut history = History::new();
    let done = rewrite(&mut word, &mut history, 2, 2, "#### Deep", FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    assert!(
        done.result
            .content
            .contains("no style for a level-4 heading, so that heading is level 3"),
        "{}",
        done.result.content
    );
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(
        wp_model::outline::heading_level(word.paragraphs()[1], &word.styles),
        Some(3)
    );

    // A document whose heading style names no next style: what follows a
    // heading is still body text, not another heading.
    let mut word = wp_text::markdown::read("# Title\n\nBody\n");
    let heading = word.paragraphs()[0].props.style;
    assert!(heading.is_some());
    assert_eq!(
        heading
            .and_then(|style| word.styles.get(style))
            .and_then(|style| style.next),
        None,
        "its style names nothing to follow it"
    );
    let done = run(
        &mut word,
        &mut history,
        &call(
            "insert_paragraphs",
            json!({"after": 1, "markdown": "a sentence"}),
        ),
        &author_at(FIRST),
    );
    assert!(!done.result.is_error, "{}", done.result.content);
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), ["Title", "a sentence", "Body"]);
    assert_eq!(
        wp_model::outline::heading_level(word.paragraphs()[1], &word.styles),
        None,
        "body text after the heading"
    );

    // A document with no heading style at all says so.
    let mut plain = document(&["one", "two"], false);
    plain.styles = wp_model::style::StyleTable::default();
    let done = rewrite(&mut plain, &mut history, 1, 1, "# Title", FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    assert!(
        done.result.content.contains("no heading styles"),
        "{}",
        done.result.content
    );
}

/// What the helper echoes back word for word comes back as the same text:
/// the lines are written escaped where they would read as Markdown, and an
/// empty paragraph is `<empty>`.
#[test]
fn what_the_helper_echoes_comes_back_as_the_same_text() {
    let texts_in = [
        "[1] Smith, J. and others",
        "1. Introduction",
        "- 5 degrees",
        "> quoted",
        "# of items",
        "---",
        "",
        "Terms marked * apply; see note *",
        "call snake_case_name",
    ];
    let mut word = document(&texts_in, false);
    let sent = request(
        &word,
        Selection::at(at(0, 0)),
        About::Document,
        "Keep it.",
        9,
    );
    let lines: Vec<&str> = sent
        .lines()
        .filter(|line| line.starts_with('['))
        .map(|line| line.split_once("] ").map(|(_, rest)| rest).unwrap_or(""))
        .collect();
    assert_eq!(lines.len(), texts_in.len(), "{sent}");
    let echoed = lines.join("\n");
    let mut history = History::new();
    let done = rewrite(&mut word, &mut history, 1, texts_in.len(), &echoed, FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), texts_in);
}

/// Two proposals side by side, and a person's own tracked change: each card
/// settles its own proposal's changes and nothing else, and the count of
/// open proposals follows.
#[test]
fn each_card_settles_its_own_proposal_and_no_other() {
    let mut word = document(&["Title", "one", "two"], true);
    let mut history = History::new();
    // The person's own tracked typing, beside what the assistant will change.
    if let Block::Paragraph(paragraph) = &mut word.body[1] {
        revise::record_insertion(paragraph, 3, " mine", &Author::new("Adnan Khan"), 1)
            .expect("typed");
    }
    rewrite(&mut word, &mut history, 2, 2, "one rewritten", FIRST);
    // The first proposal left the old paragraph struck where it was, so
    // "two" is paragraph 4 now, as the first proposal's answer said.
    rewrite(&mut word, &mut history, 4, 4, "two rewritten", SECOND);
    assert_eq!(open(&word), 2);
    assert!(standing(&word, FIRST) && standing(&word, SECOND));

    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert!(!standing(&word, FIRST));
    assert!(standing(&word, SECOND), "the other proposal is still open");
    assert_eq!(open(&word), 1);
    assert_eq!(texts(&word), ["Title", "one rewritten", "two rewritten"]);
    assert!(
        !revise::tracked(&word)
            .iter()
            .any(|change| &*change.mark.author == "Adnan Khan"),
        "the person's insertion was inside what the proposal replaced, and went with it"
    );

    settle(&mut word, &mut history, SECOND, Resolve::Reject);
    assert_eq!(texts(&word), ["Title", "one rewritten", "two"]);
    assert_eq!(open(&word), 0);

    // A person's change beside a proposal outlives Accept all.
    let mut word = document(&["Title", "one", "two"], true);
    if let Block::Paragraph(paragraph) = &mut word.body[2] {
        revise::record_insertion(paragraph, 3, " mine", &Author::new("Adnan Khan"), 1)
            .expect("typed");
    }
    rewrite(&mut word, &mut history, 2, 2, "one rewritten", FIRST);
    rewrite(&mut word, &mut history, 1, 1, "Heading", SECOND);
    // Four changes to each proposal: the words struck, the mark with them,
    // the new words, and the new mark.
    assert_eq!(settle_all(&mut word, &mut history, Resolve::Accept), 8);
    let left = revise::tracked(&word);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(&*left[0].mark.author, "Adnan Khan");
    assert_eq!(texts(&word), ["Heading", "one rewritten", "two mine"]);
    assert_eq!(settle_all(&mut word, &mut history, Resolve::Accept), 0);
}

/// The document's own Track Changes setting has no say: a proposal is
/// recorded, and the setting is left as it was.
#[test]
fn a_proposal_is_tracked_whether_track_changes_is_on_or_not() {
    for on in [false, true] {
        let mut word = document(&["Title", "one"], true);
        word.settings.track_changes = on;
        let mut history = History::new();
        rewrite(&mut word, &mut history, 2, 2, "one rewritten", FIRST);
        let changes = revise::tracked(&word);
        assert!(!changes.is_empty(), "tracked with the setting {on}");
        assert!(changes.iter().all(|change| &*change.mark.author == AUTHOR));
        assert!(changes
            .iter()
            .all(|change| change.mark.date.as_deref() == Some(FIRST)));
        assert_eq!(word.settings.track_changes, on);
    }
}

/// A refusal leaves the document as it was and tells the helper why, in
/// words it can act on: a link in the way, a table between the paragraphs.
#[test]
fn a_proposal_that_cannot_be_recorded_is_refused_whole() {
    let mut word = document(&["Title", "see ", "after"], true);
    if let Block::Paragraph(paragraph) = &mut word.body[1] {
        paragraph
            .content
            .push(Inline::Hyperlink(Box::new(wp_model::Hyperlink {
                rel: None,
                anchor: Some("x".into()),
                tooltip: None,
                history: true,
                content: vec![Inline::Run(wp_model::doc::Run::of("linked"))],
            })));
    }
    let before = word.clone();
    let mut history = History::new();
    let done = rewrite(&mut word, &mut history, 2, 2, "no link", FIRST);
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("a link"),
        "{}",
        done.result.content
    );
    assert_eq!(done.proposal, None);
    assert_eq!(word.body, before.body);
    assert!(!history.can_undo());

    let mut word = document(&["Title", "before", "after"], true);
    word.body.insert(
        2,
        Block::Table(wp_model::table::Table {
            rows: vec![wp_model::table::Row {
                cells: vec![wp_model::table::Cell {
                    props: wp_model::table::CellProps::new(),
                    content: vec![Block::Paragraph(Paragraph::of("cell"))],
                }],
                ..wp_model::table::Row::new()
            }],
            ..wp_model::table::Table::new()
        }),
    );
    let before = word.clone();
    let done = rewrite(&mut word, &mut history, 2, 4, "all", FIRST);
    assert!(done.result.is_error);
    assert!(
        done.result.content.contains("a table")
            && done.result.content.contains("on one side of it"),
        "{}",
        done.result.content
    );
    assert_eq!(word.body, before.body);
    // One cell's paragraph alone is fine.
    let done = rewrite(&mut word, &mut history, 3, 3, "a cell", FIRST);
    assert!(!done.result.is_error, "{}", done.result.content);
    settle(&mut word, &mut history, FIRST, Resolve::Accept);
    assert_eq!(texts(&word), ["Title", "before", "a cell", "after"]);

    // Too many at once.
    let many: Vec<String> = (0..MOST_WRITTEN + 1).map(|n| format!("p{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    let mut long = document(&many, false);
    let done = rewrite(&mut long, &mut history, 1, MOST_WRITTEN + 1, "x", FIRST);
    assert!(done.result.is_error);
}

/// What the helper writes is read as paragraphs: numbers it echoes are not
/// text, blank lines separate, and emphasis and headings are kept.
#[test]
fn the_helpers_markdown_is_read_a_paragraph_a_line() {
    let lines = read_lines(
        "[12] # Heading\n\n[13] plain **bold** words\nsnake_case stays\n[x] not a number",
    );
    let summary: Vec<(Option<u8>, String)> = lines
        .iter()
        .map(|line| {
            let paragraph = Paragraph {
                content: line.content.clone(),
                ..Paragraph::new()
            };
            (line.heading, paragraph.text())
        })
        .collect();
    assert_eq!(
        summary,
        [
            (Some(1), "Heading".to_owned()),
            (None, "plain bold words".to_owned()),
            (None, "snake_case stays".to_owned()),
            (None, "[x] not a number".to_owned()),
        ]
    );
    assert!(read_lines("  \n\n").is_empty());
}

#[test]
fn a_proposals_time_is_written_as_word_writes_one() {
    assert_eq!(time_of(0), "1970-01-01T00:00:00Z");
    assert_eq!(time_of(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(time_of(1_789_646_400), "2026-09-17T12:00:00Z");
    assert_eq!(time_of(1_789_646_400 + 3_661), "2026-09-17T13:01:01Z");
}
