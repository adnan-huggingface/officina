//! The ink on a page that is not type, and whether the two renderings agree
//! about where it went.
//!
//! A **mark** is a rectangle of ink: a table border, an underline, a shading,
//! the box a picture was put in. Until this existed the comparison could only
//! see words, and a rule was free to move an inch without moving a number —
//! the largest thing the instrument was blind to, and the kind of blindness
//! that reads as a clean bill.
//!
//! **Both readings are reduced by the same code, and that is the whole design.**
//! Neither renderer draws a border the way the other does: Word lays a table's
//! top edge as a little square at each corner with the spans between them,
//! while Scriva lays one rule per cell, so a page of ink the two agree about to
//! a hundredth of a point arrives here as thirty rectangles against nine.
//! Nothing compares rectangles as they arrive. Both sides are first reduced to
//! their ink — duplicates dropped, touching collinear pieces run together — by
//! [`merged`], which cannot tell which side it is working on. This is the
//! lesson the line cutting in [`crate::diff`] taught twice over: any decision
//! that shapes what is compared has to be taken from both readings at once, or
//! one side is held to a convention it never agreed to.

use std::collections::BTreeMap;

use crate::diff::{Difference, Kind};

/// How far apart two pieces of one rule may be and still be one rule.
///
/// Word's spans meet exactly, or within the hundredth of a point its export
/// rounds to; Scriva's cell edges meet exactly. A twentieth of a point is
/// wider than either and far narrower than any gap a page leaves on purpose.
///
/// A rule may also be *broken* rather than merely divided, and [`bridged`] is
/// how wide a break may be.
const TOUCHING: f64 = 0.05;

/// Thinner than this in one direction and a mark is a rule rather than a box.
///
/// Only the wording of a finding turns on it. A hairline border rounds to
/// nothing and a paragraph's shading is tens of points tall; three is clear of
/// both, and the number printed beside the word is the measurement anyway.
const THIN: f64 = 3.0;

/// How far a mark may be from the one it is paired with.
///
/// The same three lines of type [`crate::diff`] allows a word, for the same
/// reason: past it the two are not one mark that moved but two marks, and
/// saying so is more use than a finding of forty points.
const FAR: f64 = 36.0;

/// How near a picture's own box a mark must be to be answering for it rather
/// than drawn inside it. See [`answered`].
const ALIKE: f64 = 1.0;

/// The thinnest a stroke is taken to be, and the twin of the same number in
/// `pdfink.py`. PDF's zero width means "one device pixel", which is not a
/// measurement in points at all.
pub const HAIRLINE: f64 = 0.05;

/// The most pairs the matching below will weigh. The same guard [`crate::diff`]
/// sets, for the same reason: a refusal is honest and a silent cap is not.
const PAIRWISE_LIMIT: usize = 4_000_000;

/// A rectangle of ink, in points from the top-left of the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    /// How far out one mark is from another: the furthest any one edge moved.
    ///
    /// Four edges rather than a corner and a size, because a rule that grew at
    /// one end and a rule that slid whole are different faults and a number
    /// that reports them alike hides one of them. It is in points, and it is
    /// the same kind of number the words are held to, so one threshold does
    /// for both.
    fn apart(&self, other: &Rect) -> f64 {
        (self.x0 - other.x0)
            .abs()
            .max((self.y0 - other.y0).abs())
            .max((self.x1 - other.x1).abs())
            .max((self.y1 - other.y1).abs())
    }

    /// The shift of the edge that moved furthest, kept signed, one per axis.
    fn shift(&self, other: &Rect) -> (f64, f64) {
        let furthest = |a: f64, b: f64| match a.abs() >= b.abs() {
            true => a,
            false => b,
        };
        (
            furthest(self.x0 - other.x0, self.x1 - other.x1),
            furthest(self.y0 - other.y0, self.y1 - other.y1),
        )
    }

    fn within(&self, outer: &Rect, slack: f64) -> bool {
        self.x0 >= outer.x0 - slack
            && self.y0 >= outer.y0 - slack
            && self.x1 <= outer.x1 + slack
            && self.y1 <= outer.y1 + slack
    }
}

/// One rectangle of ink, and the page it is on.
///
/// No band. A word carries the flow that laid it because that is worth knowing
/// and costs nothing; a mark's position comes from [`wp_print::ops::flatten`],
/// which is the paper renderer's own account of a page and does not say. It
/// could be made to, and the price would be this module restating how a page
/// is walked — the one thing [`crate::ours`] is careful never to do, because a
/// restated page is a page nobody prints.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub page: u32,
    pub rect: Rect,
    /// Whether this is the box a page put a picture or a chart in, rather than
    /// ink the page laid itself. A rendered page has no such thing — Word draws
    /// the picture's contents and nothing else — so it is only ever true on our
    /// side, and [`answered`] is what it is for.
    pub picture: bool,
}

impl Mark {
    /// How a finding names it. The measurement is the description: there is no
    /// telling a border from a shading in a rendered page, and a report that
    /// guessed would be inventing the one fact it cannot have.
    fn named(&self) -> String {
        let (width, height) = (self.rect.width(), self.rect.height());
        match (self.picture, width <= THIN, height <= THIN) {
            (true, _, _) => format!("picture {width:.1} by {height:.1}"),
            (_, false, true) => format!("rule {width:.1} wide"),
            (_, true, false) => format!("rule {height:.1} tall"),
            _ => format!("box {width:.1} by {height:.1}"),
        }
    }

    fn found(&self, kind: Kind) -> Difference {
        Difference {
            page: self.page,
            band: None,
            at: (self.rect.x0, self.rect.y0),
            text: self.named(),
            kind,
        }
    }
}

/// The same ink, said once.
///
/// Duplicates first — Word's export states a table's corner squares twice, once
/// for the row above the junction and once for the row below — and then
/// touching collinear pieces are run together, along one axis and then the
/// other, until nothing more will join. Two passes settle every page anyone has
/// measured; it is written as a loop because "enough so far" is not a reason.
pub fn merged(marks: &[Mark]) -> Vec<Mark> {
    let mut out = deduped(marks);
    loop {
        let before = out.len();
        out = run_together(out, Axis::Across);
        out = run_together(out, Axis::Down);
        if out.len() == before {
            return out;
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Across,
    Down,
}

/// A coordinate as a bucket key, to a fiftieth of a point.
///
/// Coarse enough that the last digit of an export cannot part two pieces of one
/// rule, fine enough that two rules a tenth of a point apart stay two.
fn key(value: f64) -> i64 {
    (value * 50.0).round() as i64
}

fn deduped(marks: &[Mark]) -> Vec<Mark> {
    let mut seen = std::collections::HashSet::new();
    marks
        .iter()
        .filter(|mark| {
            seen.insert((
                mark.page,
                mark.picture,
                key(mark.rect.x0),
                key(mark.rect.y0),
                key(mark.rect.x1),
                key(mark.rect.y1),
            ))
        })
        .cloned()
        .collect()
}

/// How wide a break in a rule may be and still be a break rather than a gap.
///
/// **A rule is broken where another rule crosses it, and the break is the width
/// of the rule that crosses.** Word's export makes the corner where two table
/// borders meet a little square of its own, and that square can be run into the
/// horizontal border or into the vertical one but not into both — so whichever
/// pass takes it leaves the other with a rule in pieces, each piece one
/// crossing short of its neighbour. Scriva's column rules already overlap by
/// half their thickness for the same reason the screen needs them to, and so
/// they came out whole while Word's came out in fives: nine rules against one,
/// with nothing wrong on the page at all.
///
/// Only a rule is granted this. A shading is a box, a box is not crossed, and
/// a gap between two boxes is a gap somebody meant.
fn bridged(thickness: f64) -> f64 {
    match thickness <= THIN {
        true => TOUCHING + thickness,
        false => TOUCHING,
    }
}

/// Pieces that share their extent across one axis and meet along the other,
/// joined into the one mark they draw.
fn run_together(marks: Vec<Mark>, axis: Axis) -> Vec<Mark> {
    let mut rows: BTreeMap<(u32, bool, i64, i64), Vec<Mark>> = BTreeMap::new();
    for mark in marks {
        let fixed = match axis {
            Axis::Across => (key(mark.rect.y0), key(mark.rect.y1)),
            Axis::Down => (key(mark.rect.x0), key(mark.rect.x1)),
        };
        rows.entry((mark.page, mark.picture, fixed.0, fixed.1))
            .or_default()
            .push(mark);
    }
    let mut out = Vec::new();
    for (_, mut row) in rows {
        let moving = |mark: &Mark| match axis {
            Axis::Across => (mark.rect.x0, mark.rect.x1),
            Axis::Down => (mark.rect.y0, mark.rect.y1),
        };
        row.sort_by(|a, b| moving(a).0.total_cmp(&moving(b).0));
        let mut open: Option<Mark> = None;
        for mark in row {
            let across = match axis {
                Axis::Across => mark.rect.height(),
                Axis::Down => mark.rect.width(),
            };
            let reach = bridged(across);
            let joined = open
                .as_mut()
                .filter(|so_far| moving(&mark).0 <= moving(so_far).1 + reach);
            match joined {
                Some(so_far) => match axis {
                    Axis::Across => so_far.rect.x1 = so_far.rect.x1.max(mark.rect.x1),
                    Axis::Down => so_far.rect.y1 = so_far.rect.y1.max(mark.rect.y1),
                },
                None => out.extend(open.replace(mark)),
            }
        }
        out.extend(open);
    }
    out
}

/// What the two sides made of a page's furniture.
#[derive(Debug, Default)]
pub struct Split {
    pub matched: usize,
    /// Marks both sides drew, further out than the threshold.
    pub out: usize,
    /// Marks only one side drew at all.
    pub lost: usize,
    pub worst: f64,
    /// Picture boxes Word answered by drawing the picture rather than by
    /// drawing a box. Reported rather than counted — see [`answered`].
    pub pictures: usize,
    /// Stretches too large to pair, which is a refusal and not a result.
    pub refused: usize,
    pub differences: Vec<Difference>,
}

/// Where the two readings put the page's furniture.
pub fn compare(ours: &[Mark], theirs: &[Mark], threshold: f64) -> Split {
    let (all_mine, all_theirs) = (merged(ours), merged(theirs));
    let (mine, pictures) = answered(&all_mine, &all_theirs);
    let (theirs, _) = answered(&all_theirs, &all_mine);
    let mut split = Split {
        pictures,
        ..Split::default()
    };

    let pages: std::collections::BTreeSet<u32> = mine
        .iter()
        .chain(theirs.iter())
        .map(|mark| mark.page)
        .collect();
    for page in pages {
        let on = |marks: &[Mark]| -> Vec<Mark> {
            marks.iter().filter(|m| m.page == page).cloned().collect()
        };
        let (mine, theirs) = (on(&mine), on(&theirs));
        let (paired, my_spare, their_spare) = pair_up(&mine, &theirs, &mut split.refused);
        for (i, j) in paired {
            split.matched += 1;
            let apart = mine[i].rect.apart(&theirs[j].rect);
            split.worst = split.worst.max(apart);
            if apart > threshold {
                split.out += 1;
                let (dx, dy) = mine[i].rect.shift(&theirs[j].rect);
                split
                    .differences
                    .push(mine[i].found(Kind::Moved { dx, dy }));
            }
        }
        for index in their_spare {
            split.differences.push(theirs[index].found(Kind::Missing));
        }
        for index in my_spare {
            split.differences.push(mine[index].found(Kind::Extra));
        }
    }

    split.lost = split
        .differences
        .iter()
        .filter(|found| !matches!(found.kind, Kind::Moved { .. }))
        .count();
    split.differences.sort_by(|a, b| {
        b.magnitude()
            .partial_cmp(&a.magnitude())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.page.cmp(&b.page))
            .then(a.text.cmp(&b.text))
    });
    split
}

/// What is left of one side once the insides of the pictures are set aside.
///
/// A picture is where the two renderings stop being able to answer for one
/// another. Scriva puts a box on the page and hands it to the recording or the
/// chart that fills it; Word's rendering has no box at all, only the several
/// hundred strokes the picture is made of. Comparing those against a single
/// rectangle would report every diagram in the corpus as a hundred marks Word
/// drew and we did not — a floor with no fault under it, loud enough to bury
/// anything real.
///
/// So a mark drawn *inside* a picture's box is set aside on both sides, and a
/// picture box that had anything set aside inside it is set aside with them:
/// Word drew into it, which is as much as this can honestly say. What that
/// leaves is worth having. A raster picture states its box outright in the
/// rendering and is held to a tenth of a point; and a picture box Word drew
/// nothing into at all survives to be reported, which is a picture that did not
/// render.
///
/// **What it costs.** A picture's box is set aside whole, so a rule that runs
/// under a picture is set aside with it. That is nothing on any page in the
/// corpus, where a picture sits in the flow of the text and has nothing beneath
/// it, and it would be the whole of a page set behind a full-width image. It is
/// also why a shape's own words are *not* treated this way: a WordArt watermark
/// is transparent and the body's rules run under it, so setting its box aside
/// would take them with it — those are left in the count instead, and named in
/// the record as the floor they are.
fn answered(mine: &[Mark], theirs: &[Mark]) -> (Vec<Mark>, usize) {
    let boxes: Vec<&Mark> = mine
        .iter()
        .chain(theirs.iter())
        .filter(|mark| mark.picture)
        .collect();
    if boxes.is_empty() {
        return (mine.to_vec(), 0);
    }
    let inside = |mark: &Mark| -> Option<&&Mark> {
        boxes.iter().find(|picture| {
            picture.page == mark.page
                && mark.rect.within(&picture.rect, TOUCHING)
                && mark.rect.apart(&picture.rect) > ALIKE
        })
    };
    let drawn: std::collections::HashSet<(u32, i64, i64)> = mine
        .iter()
        .chain(theirs.iter())
        .filter_map(inside)
        .map(|picture| (picture.page, key(picture.rect.x0), key(picture.rect.y0)))
        .collect();
    let kept: Vec<Mark> = mine
        .iter()
        .filter(|mark| inside(mark).is_none())
        .filter(|mark| {
            !mark.picture || !drawn.contains(&(mark.page, key(mark.rect.x0), key(mark.rect.y0)))
        })
        .cloned()
        .collect();
    (kept, drawn.len())
}

/// Each mark against the nearest one on the other side that is still free.
///
/// Nearest first over the whole page rather than in reading order, because
/// marks have no order: a page draws its furniture in whatever sequence its
/// renderer pleases and the two sequences agree about nothing. That is the
/// whole difference between this and the words, which are matched by what they
/// say and could not be matched by where they are.
fn pair_up(
    mine: &[Mark],
    theirs: &[Mark],
    refused: &mut usize,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    if mine.len().saturating_mul(theirs.len()) > PAIRWISE_LIMIT {
        *refused += 1;
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let all = (0..mine.len()).flat_map(|i| (0..theirs.len()).map(move |j| (i, j)));
    let mut candidates: Vec<(f64, usize, usize)> = all
        .map(|(i, j)| (mine[i].rect.apart(&theirs[j].rect), i, j))
        .filter(|(apart, i, j)| *apart <= FAR && mine[*i].picture == theirs[*j].picture)
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    let (mut mine_seen, mut theirs_seen) = (vec![false; mine.len()], vec![false; theirs.len()]);
    let mut paired = Vec::new();
    for (_, i, j) in candidates {
        if mine_seen[i] || theirs_seen[j] {
            continue;
        }
        mine_seen[i] = true;
        theirs_seen[j] = true;
        paired.push((i, j));
    }
    let spare = |seen: &[bool]| -> Vec<usize> {
        seen.iter()
            .enumerate()
            .filter(|(_, taken)| !**taken)
            .map(|(index, _)| index)
            .collect()
    };
    (paired, spare(&mine_seen), spare(&theirs_seen))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(page: u32, x0: f64, y0: f64, x1: f64, y1: f64) -> Mark {
        Mark {
            page,
            rect: Rect::new(x0, y0, x1, y1),
            picture: false,
        }
    }

    fn picture(page: u32, x0: f64, y0: f64, x1: f64, y1: f64) -> Mark {
        Mark {
            picture: true,
            ..rule(page, x0, y0, x1, y1)
        }
    }

    /// Word's own account of a table's top border: a square at each corner, the
    /// spans between them, and the corners stated twice.
    #[test]
    fn the_pieces_of_one_border_are_one_rule() {
        let border = [
            rule(1, 72.02, 72.00, 72.50, 72.48),
            rule(1, 72.02, 72.00, 72.50, 72.48),
            rule(1, 72.50, 72.00, 228.04, 72.48),
            rule(1, 228.05, 72.00, 228.53, 72.48),
            rule(1, 228.53, 72.00, 384.07, 72.48),
        ];
        let joined = merged(&border);
        assert_eq!(joined.len(), 1, "{joined:?}");
        assert!((joined[0].rect.x0 - 72.02).abs() < 0.001);
        assert!((joined[0].rect.x1 - 384.07).abs() < 0.001);
    }

    /// The same border as Scriva lays it: one rule per cell, meeting exactly.
    #[test]
    fn a_rule_per_cell_comes_to_the_same_rule() {
        let ours = [
            rule(1, 72.0, 72.0, 228.0, 72.5),
            rule(1, 228.0, 72.0, 384.0, 72.5),
            rule(1, 384.0, 72.0, 540.0, 72.5),
        ];
        let joined = merged(&ours);
        assert_eq!(joined.len(), 1);
        assert!((joined[0].rect.width() - 468.0).abs() < 0.001);
    }

    /// Two rules of one thickness with a real gap between them are two rules.
    #[test]
    fn a_gap_a_page_left_on_purpose_is_not_closed() {
        let apart = [
            rule(1, 72.0, 72.0, 200.0, 72.5),
            rule(1, 220.0, 72.0, 380.0, 72.5),
        ];
        assert_eq!(merged(&apart).len(), 2);
    }

    /// A column rule and a row rule cross without becoming one another.
    #[test]
    fn rules_across_the_two_axes_stay_apart() {
        let cross = [
            rule(1, 72.0, 100.0, 400.0, 100.5),
            rule(1, 200.0, 72.0, 200.5, 300.0),
        ];
        assert_eq!(merged(&cross).len(), 2);
    }

    #[test]
    fn a_rule_that_slid_is_reported_by_the_edge_that_moved_furthest() {
        let ours = [rule(1, 72.0, 100.0, 400.0, 100.5)];
        let theirs = [rule(1, 72.0, 104.0, 398.0, 104.5)];
        let split = compare(&ours, &theirs, 1.0);
        assert_eq!(split.matched, 1);
        assert_eq!(split.out, 1);
        assert_eq!(split.lost, 0);
        let Kind::Moved { dx, dy } = split.differences[0].kind else {
            panic!("a rule both sides drew has moved, not vanished");
        };
        assert!((dx - 2.0).abs() < 0.001, "{dx}");
        assert!((dy + 4.0).abs() < 0.001, "{dy}");
        assert_eq!(split.differences[0].text, "rule 328.0 wide");
    }

    /// Under the threshold nothing is reported, and the largest shift is still
    /// carried: a page that is right by a hair has to say how wide the hair is.
    #[test]
    fn a_rule_within_the_threshold_is_no_finding_but_is_still_measured() {
        let ours = [rule(1, 72.0, 100.0, 400.0, 100.5)];
        let theirs = [rule(1, 72.0, 100.4, 400.0, 100.9)];
        let split = compare(&ours, &theirs, 1.0);
        assert_eq!(split.out, 0);
        assert!(split.differences.is_empty());
        assert!((split.worst - 0.4).abs() < 0.001);
    }

    /// A picture Word drew into is not compared as a box, and the strokes
    /// inside it are not compared at all.
    #[test]
    fn what_word_drew_inside_a_picture_is_set_aside_with_the_picture() {
        let ours = [picture(1, 100.0, 100.0, 300.0, 200.0)];
        let theirs = [
            rule(1, 110.0, 110.0, 290.0, 111.0),
            rule(1, 110.0, 150.0, 290.0, 151.0),
        ];
        let split = compare(&ours, &theirs, 1.0);
        assert_eq!(split.pictures, 1);
        assert_eq!(split.lost, 0, "{:?}", split.differences);
        assert_eq!(split.matched, 0);
    }

    /// A picture Word drew nothing into at all is the finding this exists for.
    #[test]
    fn a_picture_nothing_answers_at_all_is_reported() {
        let ours = [picture(1, 100.0, 100.0, 300.0, 200.0)];
        let split = compare(&ours, &[], 1.0);
        assert_eq!(split.pictures, 0);
        assert_eq!(split.lost, 1);
        assert_eq!(split.differences[0].kind, Kind::Extra);
        assert_eq!(split.differences[0].text, "picture 200.0 by 100.0");
    }

    /// A raster picture states its box in the rendering, and is the one mark of
    /// its kind that can be held to a tenth of a point.
    #[test]
    fn a_raster_picture_is_compared_as_the_box_both_sides_state() {
        let ours = [picture(1, 72.0, 72.0, 192.0, 162.0)];
        let theirs = [picture(1, 72.0, 72.0, 192.0, 162.0)];
        let split = compare(&ours, &theirs, 1.0);
        assert_eq!(split.matched, 1);
        assert_eq!(split.out, 0);
        assert_eq!(split.pictures, 0);
    }

    /// Nothing pairs across pages: a rule on page two is not a rule on page one
    /// that moved seven hundred points.
    #[test]
    fn a_mark_on_another_page_is_not_the_same_mark_moved() {
        let ours = [rule(1, 72.0, 100.0, 400.0, 100.5)];
        let theirs = [rule(2, 72.0, 100.0, 400.0, 100.5)];
        let split = compare(&ours, &theirs, 1.0);
        assert_eq!(split.matched, 0);
        assert_eq!(split.lost, 2);
    }

    /// Past three lines of type the two are not one mark that moved.
    #[test]
    fn a_mark_further_off_than_a_mark_ever_moves_is_two_marks() {
        let ours = [rule(1, 72.0, 100.0, 400.0, 100.5)];
        let theirs = [rule(1, 72.0, 400.0, 400.0, 400.5)];
        let split = compare(&ours, &theirs, 1.0);
        assert_eq!(split.matched, 0);
        assert_eq!(split.lost, 2);
    }
}
