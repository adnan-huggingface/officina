//! Matching two readings of the same page against each other.
//!
//! Both sides arrive as a flat list of words, each with a page, a left edge and
//! the top of the line it sits on. Nothing here knows where either list came
//! from — which is what makes the matching testable on a machine with no Word,
//! no fonts and no document.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// How far apart two baselines may be and still belong to one line.
///
/// **Measured, because there is no gap in the distribution to put it in — and
/// the two sides do not have to agree about which side of it a gap falls on.**
/// Within one line the two partings observed are hundredths of a point, up to
/// 0.60pt in `nested-tables.docx` where a line carries two sizes. Between
/// genuinely separate lines: 3.00pt at the tightest in the corpus, and 2.1pt
/// between two rows of labels in a diagram on page 8 of the demonstration
/// document — where *our* rendering of the same two rows puts them 1.9pt apart.
///
/// That pair is the whole story. At 3.0 and at 2.0 the two sides fell on
/// opposite sides of the threshold and cut the page into lines two different
/// ways, and forty-seven words that both sides had laid within half a point of
/// each other were reported as words only one side had. One point sits clear of
/// both: three times the widest parting within a line, and half the narrowest
/// gap between two. But no number is *safe*, which is why [`cuts`] no longer
/// lets either side answer the question alone.
const SAME_LINE: f64 = 1.0;

/// How far from a page's own idea of its offset a line pairing may sit.
///
/// Three lines of ordinary type. Wider than any real disagreement about where
/// one line goes, narrower than the several lines a repetition can slide by.
///
/// It has a cost, and the cost is honest: on a page whose leading is wrong the
/// drift grows line by line, and past this the lines stop being paired and
/// their words are reported as unplaceable rather than as a shift of forty
/// points. `watermark.docx` is that page — our leading there is about 15.98 pt
/// against Word's 16.94 — and it reads as a large number either way, which is
/// what it is.
const FAR: f64 = 36.0;

/// Past this a word is not merely out of place, it is somewhere else.
///
/// Five points is about a word's own width at the sizes documents are set in,
/// and about half a line. Nothing chose it but the wish for a second number
/// coarse enough that ordinary drift never reaches it.
pub const BADLY: f64 = 5.0;

/// The most cells the table below will be asked to hold.
///
/// Sixteen million, which is sixty-four megabytes and about a second. No line
/// and no page in this project's corpus comes within three orders of it; it is
/// here so that a document built to be pathological cannot turn a measurement
/// into an out-of-memory, and what happens when it is reached is a refusal
/// rather than a guess. See [`Report::refused`].
const PAIRWISE_LIMIT: usize = 16_000_000;

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

/// The same mark, spelled the same way on both sides.
///
/// **A symbol font has no Unicode, only glyph numbers.** A bulleted list stores
/// its bullet as Symbol 0xB7, which reaches a document as U+F0B7 in the private
/// use area — a codepoint that means nothing except "the 0xB7th glyph of
/// whatever face this run names". Word's own PDF export writes down the
/// character it *drew* instead, U+2022, and so the two sides name the same ink
/// two ways. Both put it in the same place to a tenth of a point, and comparing
/// the names rather than the marks reported fifty-four of them, on one document,
/// as words only one side had laid.
///
/// One entry, because one is what occurs: U+F0B7 is the only private-use
/// character anywhere in the corpus or in the documents this has been run
/// against. Anything else that turns up belongs here too, with the same kind of
/// evidence — never a guess at what a glyph number might mean.
pub fn spelled(text: &str) -> String {
    const SAME: [(char, char); 1] = [('\u{f0b7}', '\u{2022}')];
    match text
        .chars()
        .any(|c| SAME.iter().any(|(from, _)| c == *from))
    {
        false => text.to_owned(),
        true => text
            .chars()
            .map(|c| match SAME.iter().find(|(from, _)| c == *from) {
                Some((_, to)) => *to,
                None => c,
            })
            .collect(),
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
    /// Where the word is, on whichever side laid it. Carried because "we did
    /// not lay this" is not a finding anybody can act on: the question is
    /// always *where on the page*, and answering it meant reaching for
    /// `--words` and reading two sorted lists by eye.
    pub at: (f64, f64),
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
    pub pages_theirs: usize,
    pub matched: usize,
    /// Matched words further out than the threshold.
    pub over: usize,
    /// Matched words further out than [`BADLY`].
    ///
    /// A second, coarser count, because three numbers that only ever say *how
    /// many* are blind to work moving about: one word going from three points
    /// out to half a point while another goes the other way leaves every one of
    /// them where it was. A word crossing five points is a different kind of
    /// event from a word crossing one, and counting both makes that trade
    /// visible.
    pub badly: usize,
    pub unmatched: usize,
    /// The largest shift among the words that matched.
    pub worst: f64,
    /// How many times the matching refused a stretch as too large to pair.
    ///
    /// Nought for every document anyone has measured with this. It is reported
    /// because the alternative to reporting it is a page pairing its words off
    /// in the order they happen to be in, which reads exactly like a page that
    /// agrees — a silent cap is the one failure this tool must not have.
    pub refused: usize,
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
        pages_theirs: theirs.len(),
        ..Report::default()
    };

    let mut shifts: Vec<(f64, f64)> = Vec::new();
    let mut refused = 0usize;
    let empty: Vec<&Word> = Vec::new();

    for (page, their_words) in &theirs {
        let our_words = ours.get(page).unwrap_or(&empty);
        // Twice over, and both times from the two readings together: the
        // coarse cuts say which words are compared with which, and the fine
        // ones say what order they are read in. Neither may be decided by one
        // side alone — see [`cuts`] and [`order`].
        let coarse = cuts(our_words, their_words, SAME_LINE);
        let fine = cuts(our_words, their_words, OWN_ROW);
        let mine = lines_of(our_words, &coarse, &fine);
        let theirs = lines_of(their_words, &coarse, &fine);

        let (paired, _, _) = pair_lines(&mine, &theirs, &mut refused);
        let (paired, my_spare, their_spare) = plausible(paired, &mine, &theirs);
        for (i, j) in paired {
            in_line(
                &mine[i],
                &theirs[j],
                *page,
                threshold,
                &mut shifts,
                &mut refused,
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
            let alone = (
                cuts(our_words, &[], SAME_LINE),
                cuts(our_words, &[], OWN_ROW),
            );
            for line in lines_of(our_words, &alone.0, &alone.1) {
                report.differences.extend(all_of(&line, *page, Kind::Extra));
            }
        }
    }

    report.refused = refused;
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

/// How the two readings of a page were cut into lines, and what each line says.
///
/// The first question to ask when a report claims nothing matched, and the
/// reason it is part of the tool: everything downstream compares *sequences*,
/// so two readings that were cut up differently cannot agree about anything,
/// and the way that shows up is a page of words each side supposedly laid
/// alone. Twice in one afternoon the answer was here and nowhere else.
///
/// Returns the lines of each side, in reading order, as the text the matching
/// actually sees.
pub fn grouping(ours: &[Word], theirs: &[Word], page: u32) -> (Vec<String>, Vec<String>) {
    let by_page = |words: &[Word]| -> Vec<Word> {
        words.iter().filter(|w| w.page == page).cloned().collect()
    };
    let (mine, theirs) = (by_page(ours), by_page(theirs));
    let (mine, theirs): (Vec<&Word>, Vec<&Word>) = (mine.iter().collect(), theirs.iter().collect());
    let coarse = cuts(&mine, &theirs, SAME_LINE);
    let fine = cuts(&mine, &theirs, OWN_ROW);
    let said = |words: &[&Word]| -> Vec<String> {
        lines_of(words, &coarse, &fine)
            .iter()
            .map(|line| format!("{:8.2}  {}", line[0].baseline, joined(line)))
            .collect()
    };
    (said(&mine), said(&theirs))
}

/// One line of Word's against the line of ours it was matched to.
fn in_line(
    mine: &[&Word],
    theirs: &[&Word],
    page: u32,
    threshold: f64,
    shifts: &mut Vec<(f64, f64)>,
    refused: &mut usize,
    report: &mut Report,
) {
    let my_text: Vec<&str> = mine.iter().map(|w| w.text.as_str()).collect();
    let their_text: Vec<&str> = theirs.iter().map(|w| w.text.as_str()).collect();
    let mut mine_seen = vec![false; mine.len()];
    let mut theirs_seen = vec![false; theirs.len()];

    // What one side cut into several words and the other did not, found first
    // and matched between. See [`welded`].
    let welds = welded(&my_text, &their_text);
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut from = (0usize, 0usize);
    let asked = *refused;
    for weld in welds
        .iter()
        .copied()
        .chain([((mine.len(), theirs.len()), (0, 0))])
    {
        let (start, end) = weld;
        for (i, j) in pair_up(
            &my_text[from.0..start.0],
            &their_text[from.1..start.1],
            refused,
        ) {
            pairs.push((from.0 + i, from.1 + j));
        }
        if start.0 < mine.len() {
            pairs.push(start);
            for seen in mine_seen.iter_mut().take(end.0).skip(start.0) {
                *seen = true;
            }
            for seen in theirs_seen.iter_mut().take(end.1).skip(start.1) {
                *seen = true;
            }
            from = end;
        }
    }
    // Only where there was a subsequence to repair. Gluing a refusal would
    // pair a page's words off in the order they arrived in, which is the guess
    // the refusal exists to avoid.
    if *refused == asked {
        glued(
            &my_text,
            &their_text,
            &mut pairs,
            &mut mine_seen,
            &mut theirs_seen,
        );
    }
    pairs.sort_unstable();
    for (i, j) in pairs {
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
        if out > BADLY {
            report.badly += 1;
        }
        if out > threshold {
            report.over += 1;
            report.differences.push(Difference {
                page,
                band: mine.band,
                at: (mine.x, mine.baseline),
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

/// The stretches one side wrote as several words and the other as one.
///
/// **Found before anything else, because they are the strongest evidence a
/// line holds.** Several words running together into exactly the other side's
/// one word is not a coincidence a page produces by accident; two identical
/// short words in different places are produced by every page there is. Left to
/// itself the subsequence pairs the latter and destroys the former: on page 9
/// of the demonstration document Word draws `RX_` and we draw `RX` and `_`, and
/// the subsequence paired our `RX` with a *different* `RX` thirty-two points
/// away — which was free, as far as it could see, and left nothing that could
/// be welded. Nineteen words on a page where nothing had moved.
///
/// Greedy, left to right, and only ever recording a match that consumes more
/// than one word on one of the two sides. A single word equal to a single word
/// is exactly what the subsequence is good at and is left to it.
type Weld = ((usize, usize), (usize, usize));

fn welded(mine: &[&str], theirs: &[&str]) -> Vec<Weld> {
    let mut found: Vec<Weld> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < mine.len() && j < theirs.len() {
        let (mut left, mut right) = (mine[i].to_string(), theirs[j].to_string());
        let (mut end_i, mut end_j) = (i + 1, j + 1);
        while left != right {
            if left.len() < right.len() && end_i < mine.len() {
                left.push_str(mine[end_i]);
                end_i += 1;
            } else if right.len() < left.len() && end_j < theirs.len() {
                right.push_str(theirs[end_j]);
                end_j += 1;
            } else {
                break;
            }
        }
        let welded = left == right && (end_i > i + 1 || end_j > j + 1);
        match welded {
            true => {
                found.push(((i, j), (end_i, end_j)));
                i = end_i;
                j = end_j;
            }
            false => {
                i += 1;
                j += 1;
            }
        }
    }
    found
}

/// Pairs what the two sides cut into words differently.
///
/// **A word is not a thing a page has; it is a thing a reader finds on one.**
/// Neither renderer writes words — both put marks down — so "I/O" is one word
/// here and three there whenever the calls that drew it happened to fall
/// differently, and a matcher that only ever pairs one word with one word calls
/// four words unplaceable on a diagram where nothing has moved. Both sides do
/// cut at whitespace and at a gap wide enough to read as one, which agrees
/// nearly always; this is what is left.
///
/// Run through what the subsequence could not pair, accumulating from both
/// sides until the two accumulations read the same. When they do, the pair
/// recorded is the *first* word of each group, because where a word begins is
/// where its pen went down and that is the thing being measured. The rest of
/// each group is then accounted for and is not reported as absent.
///
/// The added pairs keep the run in order, so a caller may still walk them as
/// one increasing sequence. The words absorbed into a pair are marked seen
/// here, because they are neither absent nor a measurement of their own: they
/// are the rest of a word that has already been accounted for.
fn glued(
    mine: &[&str],
    theirs: &[&str],
    pairs: &mut Vec<(usize, usize)>,
    mine_seen: &mut [bool],
    theirs_seen: &mut [bool],
) {
    let mut found: Vec<(usize, usize)> = Vec::new();
    let mut from = (0usize, 0usize);
    for (stop_i, stop_j) in pairs.iter().copied().chain([(mine.len(), theirs.len())]) {
        let (mut i, mut j) = from;
        while i < stop_i && j < stop_j {
            let (mut left, mut right) = (mine[i].to_string(), theirs[j].to_string());
            let (mut end_i, mut end_j) = (i + 1, j + 1);
            while left != right {
                // Extend whichever side has read less of the page so far. If
                // neither can be extended the two simply differ, which is the
                // finding the caller is here for.
                if left.len() < right.len() && end_i < stop_i {
                    left.push_str(mine[end_i]);
                    end_i += 1;
                } else if right.len() < left.len() && end_j < stop_j {
                    right.push_str(theirs[end_j]);
                    end_j += 1;
                } else {
                    break;
                }
            }
            match left == right {
                true => {
                    found.push((i, j));
                    for absorbed in mine_seen.iter_mut().take(end_i).skip(i) {
                        *absorbed = true;
                    }
                    for absorbed in theirs_seen.iter_mut().take(end_j).skip(j) {
                        *absorbed = true;
                    }
                    i = end_i;
                    j = end_j;
                }
                false => {
                    i += 1;
                    j += 1;
                }
            }
        }
        from = (stop_i + 1, stop_j + 1);
    }
    pairs.extend(found);
    pairs.sort_unstable();
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
            at: (word.x, word.baseline),
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

/// Where a page's lines end, decided from *both* readings of it at once.
///
/// **Neither side may answer this alone.** Each renderer reports its own
/// baselines, and the two differ by a few tenths of a point; any fixed gap that
/// decides where one line ends and the next begins will eventually have a real
/// gap sitting across it, and then the two sides cut the same page into
/// different lines. Word's diagram on page 8 of the demonstration document puts
/// two rows of labels 2.1pt apart and ours puts them 1.9pt apart: at a
/// threshold of 2.0 Word saw two lines and we saw one, no line could be paired
/// with any other, and forty-seven words that both had laid within half a point
/// of each other were each reported as a word only one side had.
///
/// So the cuts are made once, over every baseline on the page from both
/// readings together, and both are then cut in the same places. The partition
/// stops being a property of one rendering and becomes a property of the page,
/// which is the only thing the two have in common. A gap that is genuine on
/// both sides still cuts; a gap either side reports differently now cuts both
/// or neither.
fn cuts(ours: &[&Word], theirs: &[&Word], gap: f64) -> Vec<f64> {
    let mut all: Vec<f64> = ours
        .iter()
        .chain(theirs.iter())
        .map(|word| word.baseline)
        .collect();
    all.sort_by(|a, b| a.total_cmp(b));
    let mut cuts = Vec::new();
    for pair in all.windows(2) {
        if pair[1] - pair[0] > gap {
            cuts.push((pair[0] + pair[1]) / 2.0);
        }
    }
    cuts
}

/// A page's words gathered into lines, in the order a reader meets them.
///
/// Lines are gathered before anything is sorted across one, because sorting on
/// the baseline alone puts two words of the same line in a different order the
/// moment the two sides report that baseline a hundredth of a point apart —
/// and a pair swapped that way is reported as two faults rather than none.
fn lines_of<'a>(words: &[&'a Word], cuts: &[f64], fine: &[f64]) -> Vec<Vec<&'a Word>> {
    let mut lines: Vec<Vec<&Word>> = Vec::new();
    let mut at = usize::MAX;
    let mut sorted: Vec<&Word> = words.to_vec();
    sorted.sort_by(|a, b| a.baseline.total_cmp(&b.baseline));
    for word in sorted {
        let which = cuts.partition_point(|cut| *cut <= word.baseline);
        if which != at {
            lines.push(Vec::new());
            at = which;
        }
        lines.last_mut().expect("just pushed one").push(word);
    }
    for line in &mut lines {
        order(line, fine);
    }
    lines
}

/// Two baselines this far apart are two rows, for the purpose of reading order.
///
/// Wider than the 0.60pt a single line parts by when it carries two sizes, and
/// narrower than the gap between two rows of labels in the tightest diagram
/// seen. It decides only the order words are read in, never which of them are
/// compared with which — but it is still cut from *both* readings at once,
/// because it too settles what sequence each side presents, and a sequence one
/// side arrives at alone is a sequence the other need not agree with. Splitting
/// each side by its own rows, which is the version this had first, put four of
/// our words on a row of their own and reordered the line around them.
const OWN_ROW: f64 = 0.75;

/// A line's words, in the order this side would read them.
///
/// **By each side's own rows first, and only then across.** A group is cut from
/// the baselines of both readings together, so it sometimes holds two rows of a
/// diagram whose labels stand a point apart — and sorting such a group by `x`
/// alone shuffles the two rows into each other. Word draws `network_ook_sm.c`
/// as one word beginning at 370.3 and we draw it in seven pieces from 370.7,
/// with a `Manager` from the row above at 377.5: sorted across, that `Manager`
/// lands in the middle of our filename, the two readings of the group cease to
/// be the same sequence of words at all, and eight words nobody had moved were
/// reported as words only one side laid.
///
/// Sorting by the raw baseline is what this must not do either — that is what
/// swaps two words of one line the moment the two sides report their baseline a
/// hundredth of a point apart. So the rows are found first, by the gaps in this
/// side's own baselines, and the sort is by row and then by `x`.
fn order(line: &mut [&Word], fine: &[f64]) {
    line.sort_by(|a, b| {
        let row = |word: &Word| fine.partition_point(|cut| *cut <= word.baseline);
        row(a).cmp(&row(b)).then(a.x.total_cmp(&b.x))
    });
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
    refused: &mut usize,
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
            Side {
                lines: mine,
                keys: &my_keys,
            },
            Side {
                lines: theirs,
                keys: &their_keys,
            },
            from,
            (a, b),
            &mut paired,
            refused,
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

/// One side's lines, and the text of each of them run together.
///
/// The two travel everywhere as a pair — a line is compared by its words and
/// found by its text — so they are one thing here rather than two arguments
/// that must be kept in step by whoever calls.
#[derive(Clone, Copy)]
struct Side<'a> {
    lines: &'a [Vec<&'a Word>],
    keys: &'a [&'a str],
}

/// Pairs the lines lying between two fixed points, where nothing can slide far.
fn within(
    mine: Side<'_>,
    theirs: Side<'_>,
    from: (usize, usize),
    to: (usize, usize),
    paired: &mut Vec<(usize, usize)>,
    refused: &mut usize,
) {
    let (my_keys, their_keys) = (mine.keys, theirs.keys);
    let (mine, theirs) = (mine.lines, theirs.lines);
    if from.0 >= to.0 || from.1 >= to.1 {
        return;
    }
    let mut mine_taken = vec![false; to.0 - from.0];
    let mut theirs_taken = vec![false; to.1 - from.1];
    for (i, j) in pair_up(&my_keys[from.0..to.0], &their_keys[from.1..to.1], refused) {
        mine_taken[i] = true;
        theirs_taken[j] = true;
        paired.push((from.0 + i, from.1 + j));
    }

    // What the subsequence could not pair, matched by resemblance — and each of
    // ours looks a little way *ahead* rather than only at the one line of
    // Word's standing opposite it. Walking the two in lockstep looks right and
    // is not: Word sets a footnote's reference on its own raised baseline and
    // this project sets it on the body's, so Word's page has a line ours has
    // not, and every line after it stood one place out of step. Each pairing
    // failed, both sides advanced, and `footnotes-endnotes.docx` came back with
    // nothing matched at all — forty words reported unplaceable on a page whose
    // every word both sides had laid within a fifth of a point.
    let (mut i, mut j) = (0, 0);
    while i < mine_taken.len() {
        if mine_taken[i] {
            i += 1;
            continue;
        }
        let mut found = None;
        let mut looked = 0;
        for k in j..theirs_taken.len() {
            if theirs_taken[k] {
                continue;
            }
            if alike(&mine[from.0 + i], &theirs[from.1 + k]) {
                found = Some(k);
                break;
            }
            looked += 1;
            if looked >= LOOK_AHEAD {
                break;
            }
        }
        if let Some(k) = found {
            mine_taken[i] = true;
            theirs_taken[k] = true;
            paired.push((from.0 + i, from.1 + k));
            // Never backwards: what is paired stays in the order it is read in.
            j = k + 1;
        }
        i += 1;
    }
}

/// How many of Word's unpaired lines one of ours may be held against.
///
/// Small on purpose. It is here for the line or two one side has and the other
/// has not — a raised footnote reference, a heading that wrapped — and not for
/// searching a page: the further this looks the more it can pair two lines that
/// merely resemble each other, which is the failure `plausible` exists to catch
/// and this should not be manufacturing.
const LOOK_AHEAD: usize = 4;

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

/// A line as one string, for asking whether two lines are the same line.
///
/// Run together with nothing between the words, because a word boundary is not
/// something either renderer wrote down — see [`glued`]. Our "SPI Radio" and
/// Word's "SPIRadio" are the same line of the same diagram, and a key that
/// keeps the space says they are two different lines and leaves both of them
/// unplaceable. Two lines that differ only in where their spaces fall will
/// collide here, and are then matched word by word like any other pair.
fn joined(line: &Vec<&Word>) -> String {
    line.iter().map(|word| word.text.as_str()).collect()
}

/// The longest run of keys both sides have, in the same order.
///
/// An ordinary longest-common-subsequence: it is what keeps one inserted line
/// from throwing every line after it out of step, which is the whole reason
/// the two readings cannot simply be walked in parallel.
fn pair_up(ours: &[&str], theirs: &[&str], refused: &mut usize) -> Vec<(usize, usize)> {
    let (n, m) = (ours.len(), theirs.len());
    if (n + 1).saturating_mul(m + 1) > PAIRWISE_LIMIT {
        // Nothing is paired, so every word of it is reported as one side's
        // alone — which is what it is, as far as anyone here can honestly say.
        *refused += 1;
        return Vec::new();
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
        assert_eq!(report.pages_theirs, 1);
        assert_eq!(report.unmatched, 2);
    }

    /// Neither renderer writes words down; both put marks on a page. Word's
    /// export of a diagram sets "I", "/" and "O" with three positioning calls
    /// against the one our playback draws, and four words were called
    /// unplaceable on a diagram where nothing had moved.
    #[test]
    fn a_word_one_side_cut_in_three_is_still_the_word_the_other_laid() {
        let theirs = line(
            1,
            100.0,
            &[(72.0, "I"), (76.0, "/"), (79.0, "O"), (100.0, "pins")],
        );
        let ours = line(1, 100.0, &[(72.0, "I/O"), (100.0, "pins")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.unmatched, 0, "{:?}", report.differences);
        assert_eq!(report.matched, 2);
        assert_eq!(report.over, 0);
    }

    /// The measurement is of where the pen went down, so a word gathered from
    /// several marks is measured from the first of them and a real shift in it
    /// is still reported.
    #[test]
    fn a_word_pieced_together_is_measured_from_where_its_pen_went_down() {
        let theirs = line(1, 100.0, &[(72.0, "I"), (76.0, "/"), (79.0, "O")]);
        let ours = line(1, 100.0, &[(77.4, "I/O")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.differences.len(), 1);
        let Kind::Moved { dx, .. } = report.differences[0].kind else {
            panic!("both sides laid it");
        };
        assert!((dx - 5.4).abs() < 0.001, "dx was {dx}");
    }

    /// Where a space falls is not something either renderer wrote down, so it
    /// cannot be what decides whether two lines are the same line.
    #[test]
    fn two_lines_that_differ_only_in_a_space_are_the_same_line() {
        let theirs = line(1, 100.0, &[(72.0, "SPIRadio"), (120.0, "in")]);
        let ours = line(1, 100.0, &[(72.0, "SPI"), (95.0, "Radio"), (120.0, "in")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.unmatched, 0, "{:?}", report.differences);
        assert_eq!(report.matched, 2);
    }

    /// Gluing must not pair two words that merely start alike: what is joined
    /// has to read the same to the last letter.
    #[test]
    fn what_the_two_sides_really_disagree_about_is_not_glued_over() {
        let theirs = line(1, 100.0, &[(72.0, "Trans"), (100.0, "mitter")]);
        let ours = line(1, 100.0, &[(72.0, "Transceiver")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.matched, 0);
        assert_eq!(report.unmatched, 3);
    }

    /// A stretch too large to pair is said out loud. The alternative — pairing
    /// the words off in whatever order they arrived in — reads exactly like a
    /// page that agrees, and a measuring tool that quietly stops measuring is
    /// worse than one that stops.
    #[test]
    fn a_stretch_too_large_to_pair_is_refused_rather_than_guessed_at() {
        let many: Vec<(f64, &str)> = (0..4_001).map(|n| (n as f64, "word")).collect();
        let side = line(1, 100.0, &many);
        let report = compare(&side, &side, 1.0);
        assert!(report.refused > 0);
        assert_eq!(report.matched, 0);
        assert_eq!(report.unmatched, 4_001 * 2);
    }

    /// Two counts rather than one, so that work moving about is visible. A
    /// single count of what is out of place says the same thing whether a word
    /// sits a point out or half a line out.
    #[test]
    fn a_word_far_out_is_counted_twice_and_a_word_barely_out_once() {
        let theirs = line(1, 100.0, &[(72.0, "near"), (120.0, "far"), (200.0, "same")]);
        let ours = line(1, 100.0, &[(74.0, "near"), (128.0, "far"), (200.0, "same")]);
        let report = compare(&ours, &theirs, 1.0);
        assert_eq!(report.over, 2, "both are past a point");
        assert_eq!(report.badly, 1, "only one is past five");
        assert!((report.worst - 8.0).abs() < 0.001);
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
