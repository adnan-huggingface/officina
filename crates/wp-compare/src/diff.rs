//! Matching two readings of the same page against each other.
//!
//! Both sides arrive as a flat list of words, each with a page, a left edge and
//! the top of the line it sits on. Nothing here knows where either list came
//! from — which is what makes the matching testable on a machine with no Word,
//! no fonts and no document.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

/// How far apart two words' tops may be and still be called one line.
///
/// Smaller than any line this project has seen and larger than the hundredth
/// of a point the two sides disagree by within one line.
const SAME_LINE: f64 = 3.0;

/// Above this many words on one page the table below is not worth building.
const PAIRWISE_LIMIT: usize = 4_000;

/// Which of a page's flows a word belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Body,
    Header,
    Footer,
    Note,
}

impl fmt::Display for Band {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Band::Body => "body",
            Band::Header => "header",
            Band::Footer => "footer",
            Band::Note => "note",
        })
    }
}

/// One word, and where a page put it.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub page: u32,
    /// Which flow of the page laid it, where that is known. Word's side of the
    /// comparison comes off a rendered page, which no longer remembers.
    pub band: Option<Band>,
    /// Left edge, in points from the left of the page.
    pub x: f64,
    /// The baseline the word was set on, in points from the top of the page.
    ///
    /// The baseline rather than the top of anything: a glyph's box starts at
    /// its own ink, which sits a different distance below the line in every
    /// face and at every size, and the two renderers do not have to share an
    /// idea of a line to agree about a baseline.
    pub baseline: f64,
    pub text: String,
}

/// What the two sides disagreed about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// Both sides laid the word; this is how far ours is from Word's.
    Moved { dx: f64, dy: f64 },
    /// Word laid it and we did not.
    Missing,
    /// We laid it and Word did not.
    Extra,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    pub page: u32,
    /// The flow that laid it, where that is known — see [`Word::band`].
    pub band: Option<Band>,
    pub text: String,
    pub kind: Kind,
}

impl Difference {
    /// How far out this is, for ranking.
    ///
    /// An unmatched word sorts above every shift: a word in the wrong place is
    /// a smaller fault than a word that is on the wrong page or absent, and a
    /// list that buries the second under the first hides the worse of the two.
    pub fn magnitude(&self) -> f64 {
        match self.kind {
            Kind::Moved { dx, dy } => dx.abs().max(dy.abs()),
            Kind::Missing | Kind::Extra => f64::INFINITY,
        }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub pages_ours: usize,
    pub pages_word: usize,
    pub matched: usize,
    /// Matched words further out than the threshold.
    pub over: usize,
    pub unmatched: usize,
    /// The largest shift among the words that matched.
    pub worst: f64,
    /// The middle shift, which is how a convention difference between the two
    /// sides shows itself as one number rather than as a finding per word.
    pub middle: (f64, f64),
    pub differences: Vec<Difference>,
}

impl Report {
    /// The one number for the document: everything that is out of place.
    ///
    /// A count over a threshold rather than a mean, because a mean is
    /// dominated by whatever is worst and cannot be driven to zero — and zero
    /// is the only total worth aiming at.
    pub fn scalar(&self) -> usize {
        self.over + self.unmatched
    }
}

/// Compares two readings, page by page.
///
/// Page by page rather than document-wide because pagination is the one thing
/// that has to agree first: a single alignment over the whole document smears
/// one early page break across every page after it, and reports a thousand
/// differences where there is one.
pub fn compare(ours: &[Word], theirs: &[Word], threshold: f64) -> Report {
    let ours = by_page(ours);
    let theirs = by_page(theirs);
    let mut report = Report {
        pages_ours: ours.len(),
        pages_word: theirs.len(),
        ..Report::default()
    };

    let mut shifts: Vec<(f64, f64)> = Vec::new();
    let empty: Vec<&Word> = Vec::new();

    for (page, their_words) in &theirs {
        let our_words = ours.get(page).unwrap_or(&empty);
        let our_order = reading_order(our_words);
        let their_order = reading_order(their_words);
        let pairs = pair_up(&our_order, &their_order);

        let mut ours_seen = vec![false; our_order.len()];
        let mut theirs_seen = vec![false; their_order.len()];
        for (i, j) in pairs {
            ours_seen[i] = true;
            theirs_seen[j] = true;
            let (mine, theirs) = (our_order[i], their_order[j]);
            let (dx, dy) = (mine.x - theirs.x, mine.baseline - theirs.baseline);
            shifts.push((dx, dy));
            report.matched += 1;
            let out = dx.abs().max(dy.abs());
            if out > report.worst {
                report.worst = out;
            }
            if out > threshold {
                report.over += 1;
                report.differences.push(Difference {
                    page: *page,
                    band: mine.band,
                    text: mine.text.clone(),
                    kind: Kind::Moved { dx, dy },
                });
            }
        }
        report
            .differences
            .extend(unmatched(&their_order, &theirs_seen, *page, Kind::Missing));
        report
            .differences
            .extend(unmatched(&our_order, &ours_seen, *page, Kind::Extra));
    }

    // A page we laid that Word never reached is every word on it, extra.
    for (page, our_words) in &ours {
        if !theirs.contains_key(page) {
            let order = reading_order(our_words);
            let none = vec![false; order.len()];
            report
                .differences
                .extend(unmatched(&order, &none, *page, Kind::Extra));
        }
    }

    report.unmatched = report
        .differences
        .iter()
        .filter(|d| !matches!(d.kind, Kind::Moved { .. }))
        .count();
    report.middle = (
        median(shifts.iter().map(|(dx, _)| *dx)),
        median(shifts.iter().map(|(_, dy)| *dy)),
    );
    report.differences.sort_by(|a, b| {
        b.magnitude()
            .partial_cmp(&a.magnitude())
            .unwrap_or(Ordering::Equal)
            .then(a.page.cmp(&b.page))
            .then(a.text.cmp(&b.text))
    });
    report
}

fn unmatched<'a>(
    words: &'a [&'a Word],
    seen: &'a [bool],
    page: u32,
    kind: Kind,
) -> impl Iterator<Item = Difference> + 'a {
    words
        .iter()
        .enumerate()
        .filter(move |(index, _)| !seen.get(*index).copied().unwrap_or(false))
        .map(move |(_, word)| Difference {
            page,
            band: word.band,
            text: word.text.clone(),
            kind,
        })
}

fn by_page(words: &[Word]) -> BTreeMap<u32, Vec<&Word>> {
    let mut pages: BTreeMap<u32, Vec<&Word>> = BTreeMap::new();
    for word in words {
        pages.entry(word.page).or_default().push(word);
    }
    pages
}

/// Words in the order a reader meets them: down the page, then across it.
///
/// Lines are gathered before anything is sorted across one, because sorting on
/// the top alone puts two words of the same line in a different order the
/// moment the two sides report that line's top a hundredth of a point apart —
/// and a pair swapped that way is reported as two faults rather than none.
fn reading_order<'a>(words: &[&'a Word]) -> Vec<&'a Word> {
    let mut sorted: Vec<&Word> = words.to_vec();
    sorted.sort_by(|a, b| {
        a.baseline
            .partial_cmp(&b.baseline)
            .unwrap_or(Ordering::Equal)
    });

    let mut out: Vec<&Word> = Vec::with_capacity(sorted.len());
    let mut line: Vec<&Word> = Vec::new();
    for word in sorted {
        if line
            .first()
            .is_some_and(|first| word.baseline - first.baseline > SAME_LINE)
        {
            across(&mut line, &mut out);
        }
        line.push(word);
    }
    across(&mut line, &mut out);
    out
}

fn across<'a>(line: &mut Vec<&'a Word>, out: &mut Vec<&'a Word>) {
    line.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(Ordering::Equal));
    out.append(line);
}

/// The longest run of words both sides have, in the same order.
///
/// An ordinary longest-common-subsequence: it is what keeps one inserted word
/// from throwing every word after it out of step, which is the whole reason
/// the two lists cannot simply be walked in parallel.
fn pair_up(ours: &[&Word], theirs: &[&Word]) -> Vec<(usize, usize)> {
    let (n, m) = (ours.len(), theirs.len());
    if n > PAIRWISE_LIMIT || m > PAIRWISE_LIMIT {
        // A page this dense is not a page; pair them off in order rather than
        // allocating a table of sixteen million cells for it.
        return (0..n.min(m)).map(|index| (index, index)).collect();
    }

    let width = m + 1;
    let at = |i: usize, j: usize| i * width + j;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = match ours[i].text == theirs[j].text {
                true => table[at(i + 1, j + 1)] + 1,
                false => table[at(i + 1, j)].max(table[at(i, j + 1)]),
            };
        }
    }

    let (mut i, mut j) = (0, 0);
    let mut pairs = Vec::new();
    while i < n && j < m {
        if ours[i].text == theirs[j].text {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<f64> = values.collect();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    values[values.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(page: u32, x: f64, baseline: f64, text: &str) -> Word {
        Word {
            page,
            band: Some(Band::Body),
            x,
            baseline,
            text: text.into(),
        }
    }

    fn line(page: u32, baseline: f64, texts: &[(f64, &str)]) -> Vec<Word> {
        texts
            .iter()
            .map(|(x, text)| word(page, *x, baseline, text))
            .collect()
    }

    #[test]
    fn a_page_that_agrees_reports_nothing() {
        let theirs = line(1, 100.0, &[(72.0, "one"), (100.0, "two")]);
        let report = compare(&theirs, &theirs, 1.0);
        assert_eq!(report.scalar(), 0);
        assert_eq!(report.matched, 2);
        assert!(report.differences.is_empty());
    }

    #[test]
    fn a_word_moved_further_than_the_threshold_is_the_finding() {
        let theirs = line(1, 100.0, &[(72.0, "one"), (100.0, "two")]);
        let ours = line(1, 100.0, &[(72.0, "one"), (105.4, "two")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.scalar(), 1);
        assert_eq!(report.differences.len(), 1);
        let found = &report.differences[0];
        assert_eq!(found.text, "two");
        let Kind::Moved { dx, dy } = found.kind else {
            panic!("a word both sides laid has moved rather than gone missing");
        };
        assert!((dx - 5.4).abs() < 0.001, "dx was {dx}");
        assert!(dy.abs() < 0.001, "dy was {dy}");
    }

    #[test]
    fn a_shift_under_the_threshold_is_measured_but_not_reported() {
        let theirs = line(1, 100.0, &[(72.0, "one")]);
        let ours = line(1, 100.0, &[(72.14, "one")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.scalar(), 0);
        assert!((report.worst - 0.14).abs() < 0.001, "{}", report.worst);
    }

    /// The reason for the subsequence: one word we failed to lay must cost one
    /// finding, not one for every word after it on the page.
    #[test]
    fn a_missing_word_does_not_throw_the_rest_of_the_line_out_of_step() {
        let theirs = line(
            1,
            100.0,
            &[
                (72.0, "the"),
                (90.0, "quick"),
                (120.0, "brown"),
                (150.0, "fox"),
            ],
        );
        // `fox` keeps Word's own left edge: the point here is the one word
        // that is absent, and a second fault would prove nothing about it.
        let ours = line(1, 100.0, &[(72.0, "the"), (90.0, "quick"), (150.0, "fox")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.unmatched, 1);
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.differences[0].text, "brown");
        assert_eq!(report.differences[0].kind, Kind::Missing);
    }

    #[test]
    fn a_word_we_laid_and_word_did_not_is_extra() {
        let theirs = line(1, 100.0, &[(72.0, "media"), (100.0, "options,")]);
        let ours = line(
            1,
            100.0,
            &[(72.0, "media"), (100.0, "options,"), (140.0, "message")],
        );
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.differences[0].kind, Kind::Extra);
        assert_eq!(report.differences[0].text, "message");
    }

    /// An unmatched word outranks any shift, however large the shift.
    #[test]
    fn what_is_absent_ranks_above_what_is_merely_out_of_place() {
        let theirs = line(1, 100.0, &[(72.0, "one"), (100.0, "two"), (140.0, "three")]);
        let ours = line(1, 100.0, &[(99.0, "one"), (100.0, "two")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.differences[0].text, "three");
        assert_eq!(report.differences[0].kind, Kind::Missing);
    }

    /// Two words of one line whose tops the two sides report a hundredth of a
    /// point apart are still one line, and still in the order they are read.
    #[test]
    fn a_line_is_gathered_before_anything_is_sorted_across_it() {
        let theirs = vec![
            word(1, 72.0, 100.00, "one"),
            word(1, 100.0, 100.00, "two"),
            word(1, 72.0, 112.00, "three"),
        ];
        let ours = vec![
            word(1, 100.0, 100.02, "two"),
            word(1, 72.0, 99.99, "one"),
            word(1, 72.0, 112.01, "three"),
        ];
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.matched, 3);
        assert_eq!(report.scalar(), 0);
    }

    /// A page we lay and Word never reaches is not silently nothing.
    #[test]
    fn a_page_only_one_side_has_is_counted_word_by_word() {
        let theirs = line(1, 100.0, &[(72.0, "one")]);
        let ours = [
            line(1, 100.0, &[(72.0, "one")]),
            line(2, 100.0, &[(72.0, "spilled"), (110.0, "over")]),
        ]
        .concat();
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.pages_ours, 2);
        assert_eq!(report.pages_word, 1);
        assert_eq!(report.unmatched, 2);
    }

    /// A convention difference between the two sides is one number, not a
    /// finding per word.
    #[test]
    fn the_middle_shift_is_reported_so_a_uniform_offset_reads_as_one_thing() {
        let theirs = line(1, 100.0, &[(72.0, "one"), (100.0, "two"), (140.0, "three")]);
        let ours = line(1, 104.0, &[(72.0, "one"), (100.0, "two"), (140.0, "three")]);
        let report = compare(&ours, &theirs, 1.0);
        assert!((report.middle.1 - 4.0).abs() < 0.001, "{:?}", report.middle);
        assert!(report.middle.0.abs() < 0.001, "{:?}", report.middle);
    }
}
