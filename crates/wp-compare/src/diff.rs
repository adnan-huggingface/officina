//! Matching two readings of the same page against each other.
//!
//! Both sides arrive as a flat list of words, each with a page, a left edge and
//! the top of the line it sits on. Nothing here knows where either list came
//! from — which is what makes the matching testable on a machine with no Word,
//! no fonts and no document.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// How far apart two words' tops may be and still be called one line.
///
/// Smaller than any line this project has seen and larger than the hundredth
/// of a point the two sides disagree by within one line.
const SAME_LINE: f64 = 3.0;

/// How far from a page's own idea of its offset a line pairing may sit.
///
/// Three lines of ordinary type. Wider than any real disagreement about where
/// one line goes, narrower than the several lines a repetition can slide by.
const FAR: f64 = 36.0;

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
        let mine = lines_of(our_words);
        let theirs = lines_of(their_words);

        let (paired, _, _) = pair_lines(&mine, &theirs);
        let (paired, my_spare, their_spare) = plausible(paired, &mine, &theirs);
        for (i, j) in paired {
            in_line(
                &mine[i],
                &theirs[j],
                *page,
                threshold,
                &mut shifts,
                &mut report,
            );
        }
        for index in their_spare {
            report
                .differences
                .extend(all_of(&theirs[index], *page, Kind::Missing));
        }
        for index in my_spare {
            report
                .differences
                .extend(all_of(&mine[index], *page, Kind::Extra));
        }
    }

    // A page we laid that Word never reached is every word on it, extra.
    for (page, our_words) in &ours {
        if !theirs.contains_key(page) {
            for line in lines_of(our_words) {
                report.differences.extend(all_of(&line, *page, Kind::Extra));
            }
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

/// One line of Word's against the line of ours it was matched to.
fn in_line(
    mine: &[&Word],
    theirs: &[&Word],
    page: u32,
    threshold: f64,
    shifts: &mut Vec<(f64, f64)>,
    report: &mut Report,
) {
    let my_text: Vec<&str> = mine.iter().map(|w| w.text.as_str()).collect();
    let their_text: Vec<&str> = theirs.iter().map(|w| w.text.as_str()).collect();
    let mut mine_seen = vec![false; mine.len()];
    let mut theirs_seen = vec![false; theirs.len()];

    for (i, j) in pair_up(&my_text, &their_text) {
        mine_seen[i] = true;
        theirs_seen[j] = true;
        let (mine, theirs) = (mine[i], theirs[j]);
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
                page,
                band: mine.band,
                text: mine.text.clone(),
                kind: Kind::Moved { dx, dy },
            });
        }
    }
    report
        .differences
        .extend(unmatched(theirs, &theirs_seen, page, Kind::Missing));
    report
        .differences
        .extend(unmatched(mine, &mine_seen, page, Kind::Extra));
}

fn all_of(line: &[&Word], page: u32, kind: Kind) -> Vec<Difference> {
    let none = vec![false; line.len()];
    unmatched(line, &none, page, kind).collect()
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

/// A page's words gathered into lines, in the order a reader meets them.
///
/// Lines are gathered before anything is sorted across one, because sorting on
/// the baseline alone puts two words of the same line in a different order the
/// moment the two sides report that baseline a hundredth of a point apart —
/// and a pair swapped that way is reported as two faults rather than none.
fn lines_of<'a>(words: &[&'a Word]) -> Vec<Vec<&'a Word>> {
    let mut sorted: Vec<&Word> = words.to_vec();
    sorted.sort_by(|a, b| {
        a.baseline
            .partial_cmp(&b.baseline)
            .unwrap_or(Ordering::Equal)
    });

    let mut lines: Vec<Vec<&Word>> = Vec::new();
    for word in sorted {
        let same = lines
            .last()
            .and_then(|line: &Vec<&Word>| line.first())
            .is_some_and(|first| word.baseline - first.baseline <= SAME_LINE);
        match same {
            true => lines
                .last_mut()
                .expect("same implies one exists")
                .push(word),
            false => lines.push(vec![word]),
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(Ordering::Equal));
    }
    lines
}

/// Which of our lines is which of Word's, and what neither side could place.
///
/// **Neither the words nor the lines can be matched by a plain subsequence,
/// and finding that out twice is what this shape is for.** A page repeats
/// itself: the same heading word, the same "the", the same sentence down a
/// table column — and a subsequence pairs one occurrence with a far away other
/// at no cost to its own score, the moment one side holds something the other
/// does not. Word's rendering carries a watermark's words and this gathers
/// none of them, so `watermark.docx` slid by a repetition and reported three
/// hundred differences on a page that has none. Matching lines rather than
/// words fixed the words and left the lines sliding for the same reason: that
/// document's every line reads `Body text under a watermark.`, so nothing in
/// the text says which of them the extra one is.
///
/// So the lines that *cannot* slide go first. A line whose text appears once
/// on each side has only one place it can go, and the longest rising run of
/// those is a set of fixed points down the page. Everything else is matched
/// only *between* two of them, where there is no room left to slide. This is
/// patience diff, and the property it is being used for is exactly the one it
/// was invented for.
///
/// Within a region, two passes: exact line text, and then what is left walked
/// in step and paired wherever two lines still share half their words — a line
/// that differs by one word is the ordinary case and must not cost the rest of
/// that line its measurements.
fn pair_lines(
    mine: &[Vec<&Word>],
    theirs: &[Vec<&Word>],
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let my_text: Vec<String> = mine.iter().map(joined).collect();
    let their_text: Vec<String> = theirs.iter().map(joined).collect();
    let my_keys: Vec<&str> = my_text.iter().map(String::as_str).collect();
    let their_keys: Vec<&str> = their_text.iter().map(String::as_str).collect();

    let mut paired: Vec<(usize, usize)> = Vec::new();
    let mut from = (0usize, 0usize);
    let fixed = fixed_points(&my_keys, &their_keys);
    for (a, b) in fixed.iter().copied().chain([(mine.len(), theirs.len())]) {
        within(
            mine,
            theirs,
            &my_keys,
            &their_keys,
            from,
            (a, b),
            &mut paired,
        );
        if a < mine.len() && b < theirs.len() {
            paired.push((a, b));
            from = (a + 1, b + 1);
        }
    }

    let mut mine_taken = vec![false; mine.len()];
    let mut theirs_taken = vec![false; theirs.len()];
    for (i, j) in &paired {
        mine_taken[*i] = true;
        theirs_taken[*j] = true;
    }
    paired.sort_unstable();
    let spare = |taken: &[bool]| -> Vec<usize> {
        taken
            .iter()
            .enumerate()
            .filter(|(_, done)| !**done)
            .map(|(index, _)| index)
            .collect()
    };
    let (my_spare, their_spare) = (spare(&mine_taken), spare(&theirs_taken));
    (paired, my_spare, their_spare)
}

/// Pairs the lines lying between two fixed points, where nothing can slide far.
fn within(
    mine: &[Vec<&Word>],
    theirs: &[Vec<&Word>],
    my_keys: &[&str],
    their_keys: &[&str],
    from: (usize, usize),
    to: (usize, usize),
    paired: &mut Vec<(usize, usize)>,
) {
    if from.0 >= to.0 || from.1 >= to.1 {
        return;
    }
    let mut mine_taken = vec![false; to.0 - from.0];
    let mut theirs_taken = vec![false; to.1 - from.1];
    for (i, j) in pair_up(&my_keys[from.0..to.0], &their_keys[from.1..to.1]) {
        mine_taken[i] = true;
        theirs_taken[j] = true;
        paired.push((from.0 + i, from.1 + j));
    }

    let (mut i, mut j) = (0, 0);
    while i < mine_taken.len() && j < theirs_taken.len() {
        if mine_taken[i] {
            i += 1;
            continue;
        }
        if theirs_taken[j] {
            j += 1;
            continue;
        }
        if alike(&mine[from.0 + i], &theirs[from.1 + j]) {
            mine_taken[i] = true;
            theirs_taken[j] = true;
            paired.push((from.0 + i, from.1 + j));
        }
        i += 1;
        j += 1;
    }
}

/// The lines that have only one place they could possibly go.
///
/// A line whose text occurs exactly once on each side cannot be confused with
/// another; the longest rising run of those is the most fixed points that can
/// be believed at once, and everything else is matched between them.
fn fixed_points(mine: &[&str], theirs: &[&str]) -> Vec<(usize, usize)> {
    let once = |keys: &[&str]| -> HashMap<String, usize> {
        let mut seen: HashMap<String, usize> = HashMap::new();
        for key in keys {
            *seen.entry((*key).to_string()).or_insert(0) += 1;
        }
        seen
    };
    let (my_count, their_count) = (once(mine), once(theirs));
    let their_at: HashMap<&str, usize> = theirs
        .iter()
        .enumerate()
        .map(|(index, key)| (*key, index))
        .collect();

    let candidates: Vec<(usize, usize)> = mine
        .iter()
        .enumerate()
        .filter(|(_, key)| {
            my_count.get(**key).copied() == Some(1) && their_count.get(**key).copied() == Some(1)
        })
        .filter_map(|(index, key)| their_at.get(*key).map(|at| (index, *at)))
        .collect();

    rising(&candidates)
}

/// The longest run of pairs that rises on both sides.
///
/// Quadratic on purpose: this runs over the handful of lines on one page that
/// are unique on both sides, and the clear version is worth more here than the
/// clever one.
fn rising(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if pairs.is_empty() {
        return Vec::new();
    }
    let mut best = vec![1usize; pairs.len()];
    let mut prev = vec![usize::MAX; pairs.len()];
    let (mut longest, mut end) = (1usize, 0usize);
    for index in 1..pairs.len() {
        for before in 0..index {
            if pairs[before].1 < pairs[index].1 && best[before] + 1 > best[index] {
                best[index] = best[before] + 1;
                prev[index] = before;
            }
        }
        if best[index] > longest {
            longest = best[index];
            end = index;
        }
    }
    let mut run = Vec::with_capacity(longest);
    let mut at = end;
    while at != usize::MAX {
        run.push(pairs[at]);
        at = prev[at];
    }
    run.reverse();
    run
}

/// Whether two lines are recognisably the same line.
///
/// Half their words in common. Enough that one word wrapped away, or one never
/// laid, still leaves a line worth measuring word by word; not so little that
/// two unrelated short lines are called the same.
fn alike(mine: &[&Word], theirs: &[&Word]) -> bool {
    let mut left: Vec<&str> = mine.iter().map(|w| w.text.as_str()).collect();
    let common = theirs
        .iter()
        .filter(|word| match left.iter().position(|t| *t == word.text) {
            Some(at) => {
                left.remove(at);
                true
            }
            None => false,
        })
        .count();
    let most = mine.len().max(theirs.len());
    most > 0 && common * 2 >= most
}

/// Drops line pairings that cannot be the same line, whatever their text says.
///
/// The last defence, and the one that does not depend on a page saying
/// anything distinctive. `watermark.docx` is one phrase repeated three times a
/// line and forty lines down the page: there is not one line on it that occurs
/// once, so there are no fixed points to hang anything on, and a subsequence
/// over identical text is free to pair our ninth line with Word's eighteenth.
/// It reported those as words a hundred and ten points out of place, which is
/// nine lines — a number about the page's shape rather than about the
/// document.
///
/// A line that is really the same line sits near where Word put it. *Near*
/// cannot mean "at the same height", because a page may be genuinely and
/// wholly shifted and that is a finding rather than a reason to give up; so it
/// means near the middle of what every other pairing on this page says the
/// offset is. The median carries a page that has really moved, and refuses the
/// pairing that only the repetition made possible.
fn plausible(
    paired: Vec<(usize, usize)>,
    mine: &[Vec<&Word>],
    theirs: &[Vec<&Word>],
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let top = |line: &Vec<&Word>| line.first().map(|word| word.baseline).unwrap_or(0.0);
    let drift = |(i, j): &(usize, usize)| top(&mine[*i]) - top(&theirs[*j]);
    let middle = median(paired.iter().map(drift));

    let mut mine_taken = vec![false; mine.len()];
    let mut theirs_taken = vec![false; theirs.len()];
    let kept: Vec<(usize, usize)> = paired
        .into_iter()
        .filter(|pair| (drift(pair) - middle).abs() <= FAR)
        .inspect(|(i, j)| {
            mine_taken[*i] = true;
            theirs_taken[*j] = true;
        })
        .collect();

    let spare = |taken: &[bool]| -> Vec<usize> {
        taken
            .iter()
            .enumerate()
            .filter(|(_, done)| !**done)
            .map(|(index, _)| index)
            .collect()
    };
    let (my_spare, their_spare) = (spare(&mine_taken), spare(&theirs_taken));
    (kept, my_spare, their_spare)
}

fn joined(line: &Vec<&Word>) -> String {
    line.iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The longest run of keys both sides have, in the same order.
///
/// An ordinary longest-common-subsequence: it is what keeps one inserted line
/// from throwing every line after it out of step, which is the whole reason
/// the two readings cannot simply be walked in parallel.
fn pair_up(ours: &[&str], theirs: &[&str]) -> Vec<(usize, usize)> {
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
            table[at(i, j)] = match ours[i] == theirs[j] {
                true => table[at(i + 1, j + 1)] + 1,
                false => table[at(i + 1, j)].max(table[at(i, j + 1)]),
            };
        }
    }

    let (mut i, mut j) = (0, 0);
    let mut pairs = Vec::new();
    while i < n && j < m {
        if ours[i] == theirs[j] {
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

    /// The defect that made the matching go line-first, from `watermark.docx`.
    ///
    /// Word's rendering carries the watermark's own words and ours gathers
    /// none of them, and the body repeats itself. Matching word against word,
    /// the whole page slid by one repetition and reported every word of it as
    /// hundreds of points out of place — three hundred differences on a page
    /// that has none.
    #[test]
    fn a_word_only_word_laid_does_not_slide_a_page_of_repeated_text() {
        let body = |baseline: f64| {
            line(
                1,
                baseline,
                &[
                    (72.0, "Body"),
                    (100.0, "text"),
                    (130.0, "under"),
                    (160.0, "a"),
                    (170.0, "watermark."),
                ],
            )
        };
        let ours = [body(100.0), body(120.0), body(140.0)].concat();
        // Word draws the same three lines, and a watermark across them that we
        // never gather — and its words are words the body uses too.
        let theirs = [
            body(100.0),
            line(1, 110.0, &[(200.0, "Body"), (300.0, "watermark.")]),
            body(120.0),
            body(140.0),
        ]
        .concat();

        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(
            report.over, 0,
            "no body word has moved: {:?}",
            report.differences
        );
        assert_eq!(report.matched, 15, "every body word is matched");
        assert_eq!(
            report.unmatched, 2,
            "the watermark's two words, and nothing else"
        );
        assert!(report.worst < 0.001, "worst was {}", report.worst);
        assert!(
            report.middle.0.abs() < 0.001,
            "a slid page shows up as a middle shift: {:?}",
            report.middle
        );
    }

    /// A line whose words are wholly different is not forced onto some other
    /// line merely because both are left over.
    #[test]
    fn two_unrelated_leftover_lines_are_not_called_the_same_line() {
        let ours = line(1, 100.0, &[(72.0, "Nothing"), (120.0, "alike")]);
        let theirs = line(1, 100.0, &[(72.0, "Entirely"), (120.0, "other")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.matched, 0);
        assert_eq!(report.unmatched, 4);
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
