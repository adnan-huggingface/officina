//! Editing the text of a paragraph.
//!
//! Everything here works on the model and knows nothing about a window, which is
//! what lets it be tested by reading the answer rather than by looking at it.
//!
//! **A paragraph's text is not a string.** It is a tree of inlines, each holding
//! runs, each holding pieces — and an offset into "the paragraph's text" has to
//! be resolved through all three to find the byte to change. Doing that once,
//! here, is what stops every editing command from having its own idea of where
//! character 40 is.
//!
//! Deleted text is *not* part of the text. A tracked deletion is drawn and is
//! not in the document, so an offset never lands inside one and typing never
//! splits one — which is the difference between an editor that respects tracked
//! changes and one that quietly rewrites them.

use wp_model::doc::{Inline, Paragraph, Piece, Run};
use wp_model::prop::RunProps;

/// A byte offset inside one piece of one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    /// Index into the paragraph's runs, in document order.
    pub run: usize,
    /// Index into that run's pieces.
    pub piece: usize,
    /// Byte offset within the piece's text.
    pub offset: usize,
}

/// The length of a paragraph's text, in bytes.
pub fn len(paragraph: &Paragraph) -> usize {
    paragraph.text().len()
}

/// Finds where a text offset lands.
///
/// `None` when the paragraph has no text at all, which is the empty-paragraph
/// case and is handled by the caller rather than invented here.
pub fn spot_at(paragraph: &Paragraph, offset: usize) -> Option<Spot> {
    let mut seen = 0usize;
    for (run_index, run) in paragraph.runs().iter().enumerate() {
        for (piece_index, piece) in run.content.iter().enumerate() {
            let width = piece_len(piece);
            if width == 0 {
                continue;
            }
            if offset <= seen + width {
                return Some(Spot {
                    run: run_index,
                    piece: piece_index,
                    offset: offset - seen,
                });
            }
            seen += width;
        }
    }
    None
}

/// How many bytes of the paragraph's *text* a piece contributes.
///
/// The model's own count, not a second one: `wp-layout` measures a fragment's
/// place in the paragraph with the same function, and a caret placed by one
/// definition and drawn by another lands somewhere neither of them meant.
fn piece_len(piece: &Piece) -> usize {
    piece.text_len()
}

/// The formatting a caret at `offset` would type in.
///
/// Word's rule: the run *before* the caret, because that is what the user was
/// last typing in. At the very start of a paragraph there is nothing before, so
/// the run after it answers, and an empty paragraph answers with its own mark.
pub fn props_at(paragraph: &Paragraph, offset: usize) -> RunProps {
    let runs = paragraph.runs();
    if runs.is_empty() {
        return paragraph.props.mark.as_deref().cloned().unwrap_or_default();
    }
    let index = match spot_at(paragraph, offset) {
        Some(spot) if spot.offset == 0 && spot.run > 0 => spot.run - 1,
        Some(spot) => spot.run,
        None => runs.len() - 1,
    };
    runs[index].props.clone()
}

/// Inserts `text` at `offset`, in the formatting a caret there would have.
///
/// Returns the offset after the inserted text.
pub fn insert(paragraph: &mut Paragraph, offset: usize, text: &str) -> usize {
    if text.is_empty() {
        return offset;
    }
    match spot_at(paragraph, offset) {
        Some(spot) => insert_at(paragraph, spot, text),
        // Nothing to land in: the paragraph is empty, or holds only things that
        // are not text. The mark's formatting is what a new run takes.
        None => {
            let props = props_at(paragraph, 0);
            paragraph.content.push(Inline::Run(Run {
                props,
                content: vec![Piece::Text(text.into())],
                prop_change: None,
            }));
        }
    }
    offset + text.len()
}

/// Inserts a non-text piece — a page break, a tab — at a text offset.
///
/// A text piece the offset lands inside is split around the newcomer; a
/// boundary needs no splitting. Returns how many bytes of *text* the piece
/// added, which for a page break is none: the caret's world is bytes, and a
/// break is not a byte.
pub fn insert_piece(paragraph: &mut Paragraph, offset: usize, piece: Piece) -> usize {
    insert_pieces(paragraph, offset, vec![piece])
}

/// The same, for several pieces that go in together and in order.
///
/// **A field is not one piece and cannot be inserted one piece at a time.**
/// Its start, its instruction, its separator, its cached result and its end
/// carry no text between them, so every one of them lands at the same offset —
/// and a second call at that offset goes *before* what the first one put
/// there. Inserting `{ PAGE }` a piece at a time wrote it backwards, end
/// first. So the whole field arrives as one splice.
pub fn insert_pieces(paragraph: &mut Paragraph, offset: usize, pieces: Vec<Piece>) -> usize {
    let added: usize = pieces.iter().map(piece_len).sum();
    match spot_at(paragraph, offset) {
        Some(spot) => {
            let Some(run) = nth_run_mut(paragraph, spot.run) else {
                return 0;
            };
            match run.content.get_mut(spot.piece) {
                Some(Piece::Text(existing)) if spot.offset > 0 && spot.offset < existing.len() => {
                    let owned = existing.to_string();
                    let (left, right) = owned.split_at(spot.offset);
                    *existing = left.into();
                    let tail = Piece::Text(right.into());
                    let at = spot.piece + 1;
                    run.content
                        .splice(at..at, pieces.into_iter().chain(std::iter::once(tail)));
                }
                Some(_) => {
                    let at = if spot.offset == 0 {
                        spot.piece
                    } else {
                        spot.piece + 1
                    };
                    run.content.splice(at..at, pieces);
                }
                None => run.content.extend(pieces),
            }
        }
        // The paragraph has no text to land in: the pieces start a run of the
        // mark's own formatting, same as typing into an empty paragraph.
        None => {
            let props = props_at(paragraph, 0);
            paragraph.content.push(Inline::Run(Run {
                props,
                content: pieces,
                prop_change: None,
            }));
        }
    }
    added
}

fn insert_at(paragraph: &mut Paragraph, spot: Spot, text: &str) {
    let Some(run) = nth_run_mut(paragraph, spot.run) else {
        return;
    };
    match run.content.get_mut(spot.piece) {
        Some(Piece::Text(existing)) => {
            let mut owned = existing.to_string();
            let at = spot.offset.min(owned.len());
            owned.insert_str(at, text);
            *existing = owned.into();
        }
        // The offset landed on a tab, a break or a symbol, which cannot be typed
        // into. A new text piece goes beside it, on the side the offset names.
        Some(_) => {
            let at = if spot.offset == 0 {
                spot.piece
            } else {
                spot.piece + 1
            };
            run.content.insert(at, Piece::Text(text.into()));
        }
        None => run.content.push(Piece::Text(text.into())),
    }
}

/// Removes the bytes in `range` from the paragraph's text.
pub fn remove(paragraph: &mut Paragraph, range: std::ops::Range<usize>) {
    if range.is_empty() {
        return;
    }
    // Back to front, so an earlier removal does not move a later one.
    let total = len(paragraph);
    let mut cuts: Vec<(usize, usize, std::ops::Range<usize>)> = Vec::new();
    let mut seen = 0usize;
    for (run_index, run) in paragraph.runs().iter().enumerate() {
        for (piece_index, piece) in run.content.iter().enumerate() {
            let width = piece_len(piece);
            if width == 0 {
                // Something that draws but holds no text — a floating picture's
                // anchor, an embedded object, a page break — is *somewhere*
                // even though no offset names it, and a cut that spans that
                // place takes it. Skipping it instead is what leaves a picture
                // in both halves of a split paragraph, a split being a cut of
                // the front and a cut of the back: neither range overlaps
                // something of no width, so neither range removes it.
                //
                // A cut that runs to the very end of the text takes what sits
                // at the very end too, or the piece at `total` would survive
                // both halves the same way.
                let covered = seen >= range.start && (seen < range.end || range.end == total);
                if covered && drawn(piece) {
                    cuts.push((run_index, piece_index, 0..0));
                }
                continue;
            }
            let piece_range = seen..seen + width;
            let overlap = range.start.max(piece_range.start)..range.end.min(piece_range.end);
            if overlap.start < overlap.end {
                cuts.push((
                    run_index,
                    piece_index,
                    (overlap.start - seen)..(overlap.end - seen),
                ));
            }
            seen += width;
        }
    }
    for (run_index, piece_index, within) in cuts.into_iter().rev() {
        let Some(run) = nth_run_mut(paragraph, run_index) else {
            continue;
        };
        match run.content.get_mut(piece_index) {
            Some(Piece::Text(text)) => {
                let mut owned = text.to_string();
                let start = within.start.min(owned.len());
                let end = within.end.min(owned.len());
                owned.replace_range(start..end, "");
                *text = owned.into();
            }
            // A tab, a break or a symbol is one character and goes whole.
            Some(_) => {
                run.content.remove(piece_index);
            }
            None => {}
        }
    }
    prune(paragraph);
}

/// Whether a piece puts something on the page while holding none of the
/// paragraph's text.
///
/// An *inline* picture is not one of these: it is one character wide — see
/// [`wp_model::doc::OBJECT`] — and a range reaches it the ordinary way.
fn drawn(piece: &Piece) -> bool {
    matches!(
        piece,
        Piece::Drawing(_) | Piece::Embedded { .. } | Piece::Break(_)
    )
}

/// Drops runs and pieces that hold nothing.
///
/// Not cosmetic: an empty `<w:t>` is legal but an empty run is what a
/// backspaced-away word leaves behind, and a document that accumulates one per
/// edit grows without bound.
pub fn prune(paragraph: &mut Paragraph) {
    fn walk(content: &mut Vec<Inline>) {
        for inline in content.iter_mut() {
            match inline {
                Inline::Run(run) => {
                    run.content
                        .retain(|piece| !matches!(piece, Piece::Text(text) if text.is_empty()));
                }
                Inline::Hyperlink(link) => walk(&mut link.content),
                Inline::Revised { content, .. } => walk(content),
                Inline::Structured(sdt) => walk(&mut sdt.content),
                Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                    walk(content)
                }
                Inline::Anchor(_) | Inline::Math(_) => {}
            }
        }
        content.retain(|inline| match inline {
            // A run with properties and no content is what a caret sitting in
            // freshly cleared text is, so one is kept while it is the only
            // thing there — the caller prunes after the edit, not during it.
            Inline::Run(run) => !run.content.is_empty(),
            _ => true,
        });
    }
    walk(&mut paragraph.content);
}

/// Splits a paragraph in two at `offset`.
///
/// The first keeps the original's properties and identity; the second takes the
/// properties and a *fresh* identity, because `w14:paraId` names one paragraph
/// and two paragraphs sharing one would make the writer pair them both to the
/// same bytes.
pub fn split(paragraph: &Paragraph, offset: usize) -> (Paragraph, Paragraph) {
    let mut head = paragraph.clone();
    let mut tail = paragraph.clone();
    let total = len(paragraph);
    remove(&mut head, offset..total);
    remove(&mut tail, 0..offset);

    head.section = None;
    tail.id = None;
    tail.text_id = None;
    // A section break belongs to the paragraph that *ends* the section, which
    // after a split is the second one.
    tail.section = paragraph.section.clone();
    (head, tail)
}

/// Joins `tail` onto the end of `head`.
pub fn merge(head: &Paragraph, tail: &Paragraph) -> Paragraph {
    let mut joined = head.clone();
    joined.content.extend(tail.content.iter().cloned());
    // The surviving paragraph is the first one, and it inherits the section
    // break of the second — otherwise deleting a paragraph mark would delete
    // the section break that was on it.
    joined.section = tail.section.clone().or(head.section.clone());
    prune(&mut joined);
    joined
}

/// The word boundaries either side of `offset`.
///
/// Word's definition: a run of letters and digits, or a run of anything else.
/// Trailing spaces belong to the word before them, which is why Ctrl+Backspace
/// after "hello " deletes the space with the word.
pub fn word_at(text: &str, offset: usize) -> std::ops::Range<usize> {
    if text.is_empty() {
        return 0..0;
    }
    // The offset arrives in bytes and is snapped to a character boundary
    // before anything slices with it. `len - 1` is not a boundary when the
    // last character is multi-byte — the probe that ran every frame took
    // exactly that index, and typing an en dash brought the application down.
    let mut at = offset.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    let class = |i: usize| -> u8 {
        let c = text[i..].chars().next().unwrap_or(' ');
        if c.is_alphanumeric() {
            1
        } else if c.is_whitespace() {
            2
        } else {
            3
        }
    };
    // The class the word takes: the character under the caret, or the last
    // character when the caret sits at the very end.
    let probe = if at == text.len() {
        text.char_indices().next_back().map(|(i, _)| i).unwrap_or(0)
    } else {
        at
    };
    let mut start = at;
    while start > 0 {
        let previous = text[..start]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        if class(previous) != class(probe) {
            break;
        }
        start = previous;
    }
    let mut end = at;
    while end < text.len() && class(end) == class(probe) {
        end += text[end..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    // A word takes the spaces after it.
    while end < text.len() && class(end) == 2 {
        end += text[end..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    start..end
}

/// The start of the word before `offset`, for Ctrl+Left.
pub fn word_start_before(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut at = offset;
    // Step back over the spaces first, then over the word.
    while at > 0 {
        let previous = previous_char(text, at);
        if !text[previous..at].chars().all(char::is_whitespace) {
            break;
        }
        at = previous;
    }
    while at > 0 {
        let previous = previous_char(text, at);
        if text[previous..at].chars().all(char::is_whitespace) {
            break;
        }
        at = previous;
    }
    at
}

/// The start of the word after `offset`, for Ctrl+Right.
pub fn word_start_after(text: &str, offset: usize) -> usize {
    let mut at = offset.min(text.len());
    while at < text.len() {
        let next = next_char(text, at);
        if text[at..next].chars().all(char::is_whitespace) {
            break;
        }
        at = next;
    }
    while at < text.len() {
        let next = next_char(text, at);
        if !text[at..next].chars().all(char::is_whitespace) {
            break;
        }
        at = next;
    }
    at
}

pub fn previous_char(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())]
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

pub fn next_char(text: &str, offset: usize) -> usize {
    let at = offset.min(text.len());
    at + text[at..].chars().next().map(char::len_utf8).unwrap_or(0)
}

/// The nth run of a paragraph, mutably, however deeply it is wrapped.
pub fn nth_run_mut(paragraph: &mut Paragraph, index: usize) -> Option<&mut Run> {
    fn walk<'a>(content: &'a mut [Inline], want: usize, seen: &mut usize) -> Option<&'a mut Run> {
        for inline in content {
            match inline {
                Inline::Run(run) => {
                    if *seen == want {
                        return Some(run);
                    }
                    *seen += 1;
                }
                Inline::Hyperlink(link) => {
                    if let Some(found) = walk(&mut link.content, want, seen) {
                        return Some(found);
                    }
                }
                Inline::Revised { content, .. } => {
                    if let Some(found) = walk(content, want, seen) {
                        return Some(found);
                    }
                }
                Inline::Structured(sdt) => {
                    if let Some(found) = walk(&mut sdt.content, want, seen) {
                        return Some(found);
                    }
                }
                Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                    if let Some(found) = walk(content, want, seen) {
                        return Some(found);
                    }
                }
                Inline::Anchor(_) | Inline::Math(_) => {}
            }
        }
        None
    }
    let mut seen = 0;
    walk(&mut paragraph.content, index, &mut seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::Toggle;

    #[test]
    fn a_word_probe_beside_a_multibyte_character_stays_on_boundaries() {
        // "Dec 2016 –" freshly typed put the caret after the dash, and the
        // probe of the last character took `len - 1` — the dash's middle byte.
        let range = word_at("Dec 2016 –", "Dec 2016 –".len());
        assert!(range.end <= "Dec 2016 –".len());
        // An offset landing inside a character snaps to its start.
        let mid = word_at("a–b", 2);
        assert!(mid.start <= 1);
        let _ = word_at("–", 1);
        let _ = word_at("–", 3);
        let _ = word_at("née – aussi", 5);
    }

    fn bold(text: &str) -> Run {
        let mut run = Run::of(text);
        run.props.toggles.set(Toggle::Bold, true);
        run
    }

    fn two_runs() -> Paragraph {
        Paragraph {
            content: vec![Inline::Run(Run::of("plain ")), Inline::Run(bold("bold"))],
            ..Paragraph::new()
        }
    }

    #[test]
    fn typing_lands_in_the_run_the_offset_names() {
        let mut paragraph = two_runs();
        let after = insert(&mut paragraph, 3, "XYZ");
        assert_eq!(paragraph.text(), "plaXYZin bold");
        assert_eq!(after, 6);
    }

    #[test]
    fn typing_at_a_boundary_joins_the_run_before_it() {
        // Word's rule: what you type takes the formatting of what is behind the
        // caret, because that is what you were last typing in.
        let mut paragraph = two_runs();
        assert_eq!(paragraph.text(), "plain bold");
        insert(&mut paragraph, 6, "X");
        assert_eq!(paragraph.text(), "plain Xbold");
        let runs = paragraph.runs();
        assert_eq!(runs[0].text(), "plain X", "the plain run took it");
        assert!(!runs[0].props.bold());
    }

    #[test]
    fn the_formatting_a_caret_would_type_in_is_the_run_behind_it() {
        let paragraph = two_runs();
        assert!(!props_at(&paragraph, 3).bold(), "inside the plain run");
        assert!(!props_at(&paragraph, 6).bold(), "at the boundary");
        assert!(props_at(&paragraph, 8).bold(), "inside the bold run");
        assert!(props_at(&paragraph, 10).bold(), "at the end");
    }

    #[test]
    fn an_empty_paragraph_types_in_its_own_marks_formatting() {
        let mut paragraph = Paragraph::new();
        let mut mark = RunProps::default();
        mark.toggles.set(Toggle::Bold, true);
        paragraph.props.mark = Some(Box::new(mark));
        insert(&mut paragraph, 0, "new");
        assert_eq!(paragraph.text(), "new");
        assert!(paragraph.runs()[0].props.bold());
    }

    #[test]
    fn deleting_a_range_takes_it_from_every_run_it_crosses() {
        let mut paragraph = two_runs();
        // "plain bold" without bytes 3..8 is "pla" + "ld".
        remove(&mut paragraph, 3..8);
        assert_eq!(paragraph.text(), "plald");
        assert_eq!(paragraph.runs().len(), 2, "both runs survive");
    }

    #[test]
    fn deleting_a_whole_run_leaves_no_empty_run_behind() {
        let mut paragraph = two_runs();
        remove(&mut paragraph, 6..10);
        assert_eq!(paragraph.text(), "plain ");
        assert!(!paragraph.runs()[0].props.bold());
        assert_eq!(paragraph.runs().len(), 1);
    }

    #[test]
    fn a_tracked_deletion_is_not_in_the_text_and_is_never_typed_into() {
        let paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("kept")),
                Inline::Revised {
                    revision: wp_model::Revision::Deleted(wp_model::Mark::new(1, "A")),
                    content: vec![Inline::Run(Run {
                        content: vec![Piece::Deleted("gone".into())],
                        ..Run::new()
                    })],
                },
                Inline::Run(Run::of("after")),
            ],
            ..Paragraph::new()
        };
        assert_eq!(paragraph.text(), "keptafter");
        // Offset 4 is the boundary between "kept" and "after"; it must not land
        // inside the deletion.
        let spot = spot_at(&paragraph, 4).expect("a spot");
        assert_eq!(spot.run, 0);

        let mut edited = paragraph.clone();
        insert(&mut edited, 4, "X");
        assert_eq!(edited.text(), "keptXafter");
        assert_eq!(
            edited.shown_text(),
            "keptXgoneafter",
            "the deletion is still there"
        );
    }

    #[test]
    fn a_tab_is_one_character_and_goes_whole() {
        let mut paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![Piece::Text("a".into()), Piece::Tab, Piece::Text("b".into())],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        assert_eq!(paragraph.text(), "a\tb");
        remove(&mut paragraph, 1..2);
        assert_eq!(paragraph.text(), "ab");
    }

    #[test]
    fn splitting_gives_the_second_paragraph_a_fresh_identity() {
        // Two paragraphs sharing a `w14:paraId` would both pair with the same
        // bytes when the document is written.
        let mut paragraph = two_runs();
        paragraph.id = Some(0x1234_5678);
        let (head, tail) = split(&paragraph, 6);
        assert_eq!(head.text(), "plain ");
        assert_eq!(tail.text(), "bold");
        assert_eq!(head.id, Some(0x1234_5678));
        assert_eq!(tail.id, None);
        assert!(tail.runs()[0].props.bold(), "the formatting travels");
    }

    #[test]
    fn a_section_break_stays_with_the_paragraph_that_ends_the_section() {
        let mut paragraph = two_runs();
        paragraph.section = Some(Box::new(wp_model::SectionProps::new()));
        let (head, tail) = split(&paragraph, 3);
        assert!(head.section.is_none());
        assert!(tail.section.is_some(), "the break moved to the new end");
    }

    #[test]
    fn merging_joins_the_runs_and_keeps_the_section_break() {
        let mut second = Paragraph::of("second");
        second.section = Some(Box::new(wp_model::SectionProps::new()));
        let joined = merge(&Paragraph::of("first "), &second);
        assert_eq!(joined.text(), "first second");
        assert!(joined.section.is_some());
    }

    fn picture(anchored: bool) -> Piece {
        Piece::Drawing(Box::new(wp_model::doc::Drawing {
            source: Vec::new().into(),
            anchored,
            extent: (wp_model::Emu(914_400), wp_model::Emu(457_200)),
            rel: Some("rId9".into()),
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::doc::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: false,
            tone: None,
            text: None,
            outline: None,
        }))
    }

    /// The same picture at the front, in the middle and at the end.
    fn each_placing(anchored: bool) -> Vec<Paragraph> {
        [
            vec![picture(anchored), Piece::Text("abcd".into())],
            vec![
                Piece::Text("ab".into()),
                picture(anchored),
                Piece::Text("cd".into()),
            ],
            vec![Piece::Text("abcd".into()), picture(anchored)],
        ]
        .into_iter()
        .map(|content| Paragraph {
            content: vec![Inline::Run(Run {
                content,
                ..Run::new()
            })],
            ..Paragraph::new()
        })
        .collect()
    }

    #[test]
    fn an_inline_picture_is_one_character_of_the_paragraph() {
        // Word's own model, and what lets the caret step over a picture at all:
        // an offset that names it is an offset the arrows can reach.
        let paragraph = each_placing(false).swap_remove(1);
        assert_eq!(paragraph.text(), "ab\u{1}cd");
        assert_eq!(len(&paragraph), 5);
        // Two offsets, one either side of it, and typing at each goes to the
        // side it names.
        let mut before = paragraph.clone();
        insert(&mut before, 2, "X");
        assert_eq!(before.text(), "abX\u{1}cd");
        let mut after = paragraph.clone();
        insert(&mut after, 3, "X");
        assert_eq!(after.text(), "ab\u{1}Xcd");
    }

    #[test]
    fn deleting_the_character_a_picture_is_takes_the_whole_picture() {
        let mut paragraph = each_placing(false).swap_remove(1);
        remove(&mut paragraph, 2..3);
        assert_eq!(paragraph.text(), "abcd");
        assert!(paragraph.drawings().is_empty(), "and it is gone");
    }

    #[test]
    fn splitting_a_paragraph_never_leaves_a_picture_in_both_halves() {
        // Enter used to copy the picture: a drawing held none of the
        // paragraph's text, so neither half's removal ever reached it. Every
        // placing and every split point, because the ends are where a rule
        // about spans goes wrong.
        for anchored in [false, true] {
            for paragraph in each_placing(anchored) {
                for at in 0..=len(&paragraph) {
                    let (head, tail) = split(&paragraph, at);
                    assert_eq!(
                        head.drawings().len() + tail.drawings().len(),
                        1,
                        "one picture between them, anchored={anchored}, split at {at}, \
                         head {:?} tail {:?}",
                        head.text(),
                        tail.text()
                    );
                }
            }
        }
    }

    #[test]
    fn a_word_takes_the_spaces_after_it() {
        // Ctrl+Backspace after "hello " deletes the space with the word.
        assert_eq!(word_at("hello world", 2), 0..6);
        assert_eq!(word_at("hello world", 8), 6..11);
        assert_eq!(word_at("", 0), 0..0);
    }

    #[test]
    fn ctrl_arrow_moves_by_words_the_way_word_does() {
        let text = "one two  three";
        assert_eq!(word_start_after(text, 0), 4);
        assert_eq!(word_start_after(text, 4), 9);
        assert_eq!(word_start_after(text, 9), text.len());
        assert_eq!(word_start_before(text, text.len()), 9);
        assert_eq!(word_start_before(text, 9), 4);
        assert_eq!(word_start_before(text, 4), 0);
        assert_eq!(word_start_before(text, 0), 0);
    }

    #[test]
    fn moving_by_characters_never_lands_inside_one() {
        let text = "aébc";
        assert_eq!(next_char(text, 0), 1);
        assert_eq!(next_char(text, 1), 3, "é is two bytes");
        assert_eq!(previous_char(text, 3), 1);
        assert_eq!(next_char(text, text.len()), text.len());
        assert_eq!(previous_char(text, 0), 0);
    }
}
