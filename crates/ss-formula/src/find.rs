//! Finding text in a workbook, and replacing it.
//!
//! Two things about a cell can be searched and they are not the same thing.
//! `=B1*C1` *shows* `4200`; its source says neither. Excel calls the choice
//! "Look in", and it matters most when it decides what a replacement lands on:
//! typing `4200` over a displayed value would replace the formula that computed
//! it with a literal, which is not what anybody means by Replace.
//!
//! So finding may look at either, and replacing only ever rewrites the cell's
//! *source* — the formula text where there is one, and the entry itself where
//! there is not. A cell found by its displayed value whose source does not
//! contain the text is reported rather than rewritten, because the alternative
//! is to quietly turn a formula into a number.

use ss_model::{Cell, CellRef, CellValue, Workbook};

use crate::edit::{typed_cell, Change, Patch};

/// What to look for, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub needle: String,
    pub match_case: bool,
    /// The whole cell has to be the text, not merely contain it.
    pub whole_cell: bool,
    /// Look at what the cell is made of rather than at what it shows.
    pub in_formulas: bool,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            needle: String::new(),
            match_case: false,
            whole_cell: false,
            // Excel's own default for both Find and Replace, and the only one
            // under which Replace can do anything to a formula.
            in_formulas: true,
        }
    }
}

/// One cell that matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit {
    pub sheet: usize,
    pub at: CellRef,
}

/// What a cell offers the search: its source, or what it shows.
fn haystack(book: &Workbook, sheet: usize, at: CellRef, cell: &Cell, query: &Query) -> String {
    if query.in_formulas {
        if let Some(formula) = book.sheet(sheet).and_then(|s| s.formula_at(at)) {
            return format!("={}", formula.text);
        }
        // Not the formatted text: a date entered as 2026-08-10 and displayed
        // as 10-Aug is `2026-08-10` underneath, and that is what Replace would
        // be rewriting.
        return match cell.value {
            CellValue::Text(id) => book.strings.resolve(id).to_string(),
            CellValue::Number(n) => ss_model::format_general(n),
            CellValue::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_string(),
            CellValue::Error(e) => e.as_str().to_string(),
            CellValue::Blank => String::new(),
        };
    }
    crate::filter::shown(book, sheet, at, Some(cell))
}

/// Whether `text` matches, with `*` and `?` honoured as Excel honours them.
pub fn matches(text: &str, query: &Query) -> bool {
    if query.needle.is_empty() {
        return false;
    }
    let (text, needle) = if query.match_case {
        (text.to_string(), query.needle.clone())
    } else {
        (text.to_lowercase(), query.needle.to_lowercase())
    };
    if needle.contains(['*', '?']) {
        // A wildcard is always anchored at both ends, so "contains" is spelled
        // by putting stars round it — which is what Excel does too, and it is
        // why `*` in a non-whole-cell search is not the no-op it looks like.
        let pattern = if query.whole_cell {
            needle
        } else {
            format!("*{needle}*")
        };
        return crate::filter::wildcard(&text, &pattern);
    }
    if query.whole_cell {
        text == needle
    } else {
        text.contains(&needle)
    }
}

/// Every cell that matches, in reading order, sheet by sheet.
///
/// A full scan of the cells that exist, which is what a Find is: there is no
/// index, and building one for a dialog nobody has opened yet would cost more
/// than every search anybody runs.
pub fn all(book: &Workbook, sheets: &[usize], query: &Query) -> Vec<Hit> {
    let mut hits = Vec::new();
    if query.needle.is_empty() {
        return hits;
    }
    for &sheet in sheets {
        let Some(model) = book.sheet(sheet) else {
            continue;
        };
        for (at, cell) in model.cells.iter() {
            if matches(&haystack(book, sheet, at, cell, query), query) {
                hits.push(Hit { sheet, at });
            }
        }
    }
    hits
}

/// The next match after `from`, wrapping round to the start.
///
/// `from` is a position rather than a hit, so that starting the search at the
/// cursor works whether or not the cursor is itself on a match.
pub fn next(
    book: &Workbook,
    sheets: &[usize],
    from: Hit,
    query: &Query,
    backwards: bool,
) -> Option<Hit> {
    let hits = all(book, sheets, query);
    if hits.is_empty() {
        return None;
    }
    // Reading order across sheets is the order `sheets` was given in, which is
    // not the same as ordering by index once the search starts on sheet 3.
    let rank = |hit: &Hit| {
        let sheet = sheets.iter().position(|s| *s == hit.sheet).unwrap_or(0);
        (sheet, hit.at.row, hit.at.col)
    };
    let here = rank(&from);
    if backwards {
        hits.iter()
            .rev()
            .find(|hit| rank(hit) < here)
            .or_else(|| hits.last())
            .copied()
    } else {
        hits.iter()
            .find(|hit| rank(hit) > here)
            .or_else(|| hits.first())
            .copied()
    }
}

/// What replacing did, so the caller can say so rather than guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Replaced {
    pub cells: usize,
    /// Matches found by what they showed, whose source says something else.
    /// Left alone: rewriting them would replace a formula with its own result.
    pub skipped: usize,
}

/// Rewrites `hits`, returning the change and what it managed to do.
pub fn replace(book: &mut Workbook, hits: &[Hit], query: &Query, with: &str) -> (Change, Replaced) {
    let mut report = Replaced::default();
    let mut patches: Vec<Patch> = Vec::new();

    for hit in hits {
        let Some(cell) = book.sheet(hit.sheet).and_then(|s| s.get(hit.at)).copied() else {
            continue;
        };
        // Always the source, whatever the search looked at.
        let source = haystack(
            book,
            hit.sheet,
            hit.at,
            &cell,
            &Query {
                in_formulas: true,
                ..query.clone()
            },
        );
        let Some(rewritten) = substitute(&source, query, with) else {
            report.skipped += 1;
            continue;
        };
        let new_cell = typed_cell(book, hit.sheet, cell.style, &rewritten);
        patches.push(Patch::Cells {
            sheet: hit.sheet,
            cells: vec![(hit.at, Some(new_cell))],
        });
        report.cells += 1;
    }

    (Change::new("Replace", patches), report)
}

/// `text` with every occurrence of the needle replaced, or `None` if it holds
/// none — which is not the same as holding one that replaces to itself.
fn substitute(text: &str, query: &Query, with: &str) -> Option<String> {
    if query.needle.is_empty() {
        return None;
    }
    if query.whole_cell {
        return matches(text, query).then(|| with.to_string());
    }
    // A wildcard needle has no single span to swap out — `a*b` names a shape,
    // not a string — so the whole cell is what gets replaced, which is what
    // Excel does with one too.
    if query.needle.contains(['*', '?']) {
        return matches(text, query).then(|| with.to_string());
    }
    if query.match_case {
        return text
            .contains(&query.needle)
            .then(|| text.replace(&query.needle, with));
    }
    // Case-insensitively, which `str::replace` cannot do: the needle is found
    // in a lowercased copy and cut out of the original by the same offsets, so
    // the text around it keeps the case the user typed.
    let (hay, needle) = (text.to_lowercase(), query.needle.to_lowercase());
    if !hay.contains(&needle) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut cut = 0usize;
    let mut search = 0usize;
    while let Some(offset) = hay[search..].find(&needle) {
        let at = search + offset;
        out.push_str(&text[cut..at]);
        out.push_str(with);
        cut = at + needle.len();
        search = cut;
    }
    out.push_str(&text[cut..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{apply, input};

    fn at(a1: &str) -> CellRef {
        CellRef::from_a1(a1).expect("valid")
    }

    fn book_with(entries: &[(&str, &str)]) -> Workbook {
        let mut book = Workbook::blank();
        for (a1, typed) in entries {
            let change = input(&mut book, 0, at(a1), typed);
            apply(&mut book, change);
        }
        // A formula's cached value is what a value search looks at, and a
        // workbook nobody has recalculated has none.
        crate::workbook::recalculate(&mut book);
        book
    }

    fn shown(book: &Workbook, a1: &str) -> String {
        crate::filter::shown(book, 0, at(a1), book.sheets[0].get(at(a1)))
    }

    #[test]
    fn a_search_finds_the_cells_it_should_and_not_the_ones_it_should_not() {
        let book = book_with(&[("A1", "Apple"), ("A2", "pineapple"), ("A3", "APPLE")]);
        let hits = |q: Query| {
            all(&book, &[0], &q)
                .into_iter()
                .map(|h| h.at.to_a1())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            hits(Query {
                needle: "apple".into(),
                ..Default::default()
            }),
            ["A1", "A2", "A3"],
            "case-insensitive and anywhere in the cell"
        );
        assert_eq!(
            hits(Query {
                needle: "apple".into(),
                match_case: true,
                ..Default::default()
            }),
            ["A2"]
        );
        assert_eq!(
            hits(Query {
                needle: "apple".into(),
                whole_cell: true,
                ..Default::default()
            }),
            ["A1", "A3"]
        );
        assert_eq!(
            hits(Query {
                needle: "p?ne*".into(),
                ..Default::default()
            }),
            ["A2"],
            "wildcards, Excel's two"
        );
    }

    #[test]
    fn what_a_formula_shows_and_what_it_says_are_different_searches() {
        let book = book_with(&[("A1", "3"), ("A2", "4"), ("A3", "=A1*A2")]);
        let hit = |q: Query| all(&book, &[0], &q).first().map(|h| h.at.to_a1());

        assert_eq!(
            hit(Query {
                needle: "A1".into(),
                ..Default::default()
            }),
            Some("A3".to_string()),
            "the source mentions A1"
        );
        assert_eq!(
            hit(Query {
                needle: "A1".into(),
                in_formulas: false,
                ..Default::default()
            }),
            None,
            "nothing on the sheet shows the text A1"
        );
        assert_eq!(
            hit(Query {
                needle: "12".into(),
                in_formulas: false,
                ..Default::default()
            }),
            Some("A3".to_string()),
            "but it shows 12"
        );
    }

    #[test]
    fn replacing_rewrites_the_source_and_leaves_a_number_a_number() {
        let mut book = book_with(&[("A1", "the cat sat"), ("A2", "1234"), ("A3", "=A1")]);
        let query = Query {
            needle: "cat".into(),
            ..Default::default()
        };
        let hits = all(&book, &[0], &query);
        let (change, report) = replace(&mut book, &hits, &query, "dog");
        apply(&mut book, change);
        assert_eq!(report.cells, 1);
        assert_eq!(shown(&book, "A1"), "the dog sat");

        // A number stays a number rather than becoming the text of one.
        let query = Query {
            needle: "23".into(),
            ..Default::default()
        };
        let hits = all(&book, &[0], &query);
        let (change, _) = replace(&mut book, &hits, &query, "99");
        apply(&mut book, change);
        assert!(matches!(
            book.sheets[0].get(at("A2")).map(|c| c.value),
            Some(CellValue::Number(n)) if n == 1994.0
        ));
    }

    #[test]
    fn a_match_found_by_its_value_is_not_replaced_over_its_formula() {
        // The trap: A3 shows `12`, so a value search finds it. Rewriting it
        // would put the literal 99 where `=A1*A2` was and lose the formula.
        let mut book = book_with(&[("A1", "3"), ("A2", "4"), ("A3", "=A1*A2")]);
        let query = Query {
            needle: "12".into(),
            in_formulas: false,
            whole_cell: true,
            ..Default::default()
        };
        let hits = all(&book, &[0], &query);
        assert_eq!(hits.len(), 1);
        let (change, report) = replace(&mut book, &hits, &query, "99");
        apply(&mut book, change);
        assert_eq!(
            report,
            Replaced {
                cells: 0,
                skipped: 1
            }
        );
        assert_eq!(
            book.sheets[0].formula_at(at("A3")).map(|f| f.text.clone()),
            Some("A1*A2".to_string()),
            "left alone"
        );
    }

    #[test]
    fn the_case_around_a_replacement_is_the_users_own() {
        let mut book = book_with(&[("A1", "Cat CAT cat")]);
        let query = Query {
            needle: "cat".into(),
            ..Default::default()
        };
        let hits = all(&book, &[0], &query);
        let (change, _) = replace(&mut book, &hits, &query, "dog");
        apply(&mut book, change);
        assert_eq!(shown(&book, "A1"), "dog dog dog");
    }

    #[test]
    fn find_next_walks_in_reading_order_and_comes_back_round() {
        let book = book_with(&[("A1", "x"), ("C1", "x"), ("B3", "x")]);
        let query = Query {
            needle: "x".into(),
            ..Default::default()
        };
        let step = |from: &str| {
            next(
                &book,
                &[0],
                Hit {
                    sheet: 0,
                    at: at(from),
                },
                &query,
                false,
            )
            .map(|h| h.at.to_a1())
        };
        assert_eq!(step("A1"), Some("C1".into()));
        assert_eq!(step("C1"), Some("B3".into()));
        assert_eq!(step("B3"), Some("A1".into()), "wraps");

        let back = next(
            &book,
            &[0],
            Hit {
                sheet: 0,
                at: at("A1"),
            },
            &query,
            true,
        );
        assert_eq!(back.map(|h| h.at.to_a1()), Some("B3".into()), "wraps back");
    }
}
