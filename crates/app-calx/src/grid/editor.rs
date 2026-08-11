//! The cell editor: what is being typed, and how it is coloured.
//!
//! Excel has two editing modes and they behave differently under the arrow
//! keys, which is the sort of thing nobody notices until it is wrong. Start
//! typing over a cell and you are in *enter* mode, where Right commits the cell
//! and moves on. Press F2 first and you are in *edit* mode, where Right moves
//! the caret through the text. Anyone who has typed `=A1+` and then reached for
//! an arrow key to point at a cell is relying on the distinction.

use ss_formula::lexer::{tokenize, Tok};
use ss_model::{CellRange, CellRef};
use ui_kit::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Opened by typing. Arrow keys commit and move to the next cell.
    Enter,
    /// Opened by F2 or a double click. Arrow keys move the caret.
    Edit,
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub at: CellRef,
    pub text: String,
    pub mode: Mode,
    /// Set for the frame the editor opens on, so focus can be claimed once.
    pub fresh: bool,
    /// The reference the mouse or the arrow keys are writing, while they are.
    pub pointing: Option<Pointing>,
}

/// Excel's point mode: mid-formula, a click or an arrow *names a cell* instead
/// of leaving the editor, and until an operator locks it in, pointing again
/// replaces the reference rather than appending another.
///
/// The lock is detected by snapshot: pointing remembers the whole text as it
/// last wrote it, and any difference means the user has typed since — at which
/// point the reference is theirs, not ours.
#[derive(Debug, Clone)]
pub struct Pointing {
    /// Byte offset in `text` where the written reference starts.
    start: usize,
    /// The text exactly as pointing last left it.
    snapshot: String,
    /// The fixed corner, where pointing began.
    pub anchor: CellRef,
    /// The moving corner.
    pub lead: CellRef,
}

impl Editor {
    pub fn typing(at: CellRef, seed: String) -> Self {
        Editor {
            at,
            text: seed,
            mode: Mode::Enter,
            fresh: true,
            pointing: None,
        }
    }

    pub fn editing(at: CellRef, existing: String) -> Self {
        Editor {
            at,
            text: existing,
            mode: Mode::Edit,
            fresh: true,
            pointing: None,
        }
    }

    /// True when the text is a formula, which is when references are live.
    pub fn is_formula(&self) -> bool {
        self.text.starts_with('=')
    }

    /// Whether a click or an arrow would point right now: either a pointed
    /// reference is still live, or the text ends where a reference could
    /// begin — after `=`, an operator, an open paren, or a comma.
    pub fn can_point(&self) -> bool {
        self.pointed().is_some() || self.armed()
    }

    /// The pointed reference's corners, while it is still live.
    pub fn pointed(&self) -> Option<(CellRef, CellRef)> {
        let p = self.pointing.as_ref()?;
        (p.snapshot == self.text).then_some((p.anchor, p.lead))
    }

    fn armed(&self) -> bool {
        if !self.is_formula() {
            return false;
        }
        matches!(
            self.text.trim_end().chars().last(),
            Some('=' | '+' | '-' | '*' | '/' | '^' | '&' | '<' | '>' | '(' | ',' | '{' | ';')
        )
    }

    /// Starts pointing at `cell`, or moves the live reference there.
    pub fn point_at(&mut self, cell: CellRef) {
        if !self.can_point() {
            return;
        }
        let start = match self.pointed() {
            Some(_) => {
                let start = self.pointing.as_ref().expect("pointed").start;
                self.text.truncate(start);
                start
            }
            None => self.text.len(),
        };
        self.write_point(start, cell, cell);
    }

    /// Stretches the live reference into a range ending at `lead`.
    pub fn point_to(&mut self, lead: CellRef) {
        let Some((anchor, _)) = self.pointed() else {
            return;
        };
        let start = self.pointing.as_ref().expect("pointed").start;
        self.text.truncate(start);
        self.write_point(start, anchor, lead);
    }

    fn write_point(&mut self, start: usize, anchor: CellRef, lead: CellRef) {
        let named = if anchor == lead {
            anchor.to_a1()
        } else {
            let a = CellRef::new(anchor.row.min(lead.row), anchor.col.min(lead.col));
            let b = CellRef::new(anchor.row.max(lead.row), anchor.col.max(lead.col));
            format!("{}:{}", a.to_a1(), b.to_a1())
        };
        self.text.push_str(&named);
        self.pointing = Some(Pointing {
            start,
            snapshot: self.text.clone(),
            anchor,
            lead,
        });
    }

    /// F4: cycles `$` on the last reference in the text —
    /// A1 → $A$1 → A$1 → $A1 → A1, both corners of a range together. The
    /// last reference rather than the caret's, because the caret lives
    /// inside the text widget; for the way F4 is actually used — pressed the
    /// moment a reference goes in — they are the same one.
    pub fn cycle_reference(&mut self) {
        if !self.is_formula() {
            return;
        }
        let refch = |c: char| c.is_ascii_alphanumeric() || c == '$' || c == ':';
        // The last run of reference-shaped characters, wherever it ends —
        // `=SUM(A1:B2)` keeps its closing paren.
        let Some(end) = self.text.rfind(|c: char| refch(c)).map(|i| i + c_len(&self.text, i))
        else {
            return;
        };
        let tail_start = self.text[..end]
            .rfind(|c: char| !refch(c))
            .map_or(0, |i| i + c_len(&self.text, i));
        if tail_start >= end {
            return;
        }
        let after = self.text[end..].to_string();
        let tail = self.text[tail_start..end].to_string();
        let mut state = None;
        let cycled: Option<Vec<String>> = tail
            .split(':')
            .map(|part| {
                let (col_abs, letters, row_abs, digits) = split_ref(part)?;
                // The first corner decides the state; both corners get it.
                let next = *state.get_or_insert(match (col_abs, row_abs) {
                    (false, false) => (true, true),
                    (true, true) => (false, true),
                    (false, true) => (true, false),
                    (true, false) => (false, false),
                });
                Some(format!(
                    "{}{}{}{}",
                    if next.0 { "$" } else { "" },
                    letters,
                    if next.1 { "$" } else { "" },
                    digits
                ))
            })
            .collect();
        if let Some(parts) = cycled {
            self.text.truncate(tail_start);
            self.text.push_str(&parts.join(":"));
            self.text.push_str(&after);
            // The reference is the user's now; pointing must not overwrite
            // the dollars it just earned.
            self.pointing = None;
        }
    }

    /// The ranges this formula names, in the order they appear.
    ///
    /// Only same-sheet references: a highlight has to be drawn *somewhere*, and
    /// there is nowhere on this sheet to draw `Data!A1`.
    pub fn references(&self) -> Vec<CellRange> {
        let Some(body) = self.text.strip_prefix('=') else {
            return Vec::new();
        };
        let Ok(tokens) = tokenize(body) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            // A qualified reference belongs to another sheet; skip the whole unit.
            if matches!(tokens[i].kind, Tok::Sheet(_)) {
                i += if matches!(tokens.get(i + 1).map(|t| &t.kind), Some(Tok::Cell(_))) {
                    2
                } else {
                    1
                };
                continue;
            }
            let (a, b) = match (
                &tokens[i].kind,
                tokens.get(i + 1).map(|t| &t.kind),
                tokens.get(i + 2).map(|t| &t.kind),
            ) {
                (Tok::Cell(a), Some(Tok::Colon), Some(Tok::Cell(b))) => {
                    i += 3;
                    (*a, *b)
                }
                (Tok::Cell(a), _, _) => {
                    i += 1;
                    (*a, *a)
                }
                _ => {
                    i += 1;
                    continue;
                }
            };
            out.push(CellRange::new(
                CellRef::new(a.row, a.col),
                CellRef::new(b.row, b.col),
            ));
        }
        out
    }
}

/// The byte length of the char starting at `i`.
fn c_len(text: &str, i: usize) -> usize {
    text[i..].chars().next().map_or(1, char::len_utf8)
}

/// `$A$13` → (true, "A", true, "13"), or None for anything that is not a
/// plain cell reference.
fn split_ref(part: &str) -> Option<(bool, String, bool, String)> {
    let mut chars = part.chars().peekable();
    let col_abs = chars.peek() == Some(&'$');
    if col_abs {
        chars.next();
    }
    let mut letters = String::new();
    while chars.peek().is_some_and(char::is_ascii_alphabetic) {
        letters.push(chars.next().expect("peeked"));
    }
    let row_abs = chars.peek() == Some(&'$');
    if row_abs {
        chars.next();
    }
    let digits: String = chars.collect();
    let plausible = !letters.is_empty()
        && letters.len() <= 3
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit());
    plausible.then_some((col_abs, letters, row_abs, digits))
}

/// The colours references cycle through, matching the boxes drawn on the grid.
///
/// Fixed rather than theme-derived: the point is that the third reference in
/// the text and the third box on the sheet are recognizably the same colour, so
/// they have to be picked from one list.
pub const REFERENCE_COLORS: [egui::Color32; 6] = [
    egui::Color32::from_rgb(0x1F, 0x77, 0xB4),
    egui::Color32::from_rgb(0xD6, 0x27, 0x28),
    egui::Color32::from_rgb(0x2C, 0xA0, 0x2C),
    egui::Color32::from_rgb(0x94, 0x67, 0xBD),
    egui::Color32::from_rgb(0xFF, 0x7F, 0x0E),
    egui::Color32::from_rgb(0x17, 0xBE, 0xCF),
];

/// Lays out formula text with each reference in its own colour.
///
/// Anything that is not a formula, and any formula that will not lex, is laid
/// out as plain text — a half-typed formula must still be readable.
pub fn highlight(text: &str, font: egui::FontId, plain: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mut push = |slice: &str, color: egui::Color32| {
        job.append(
            slice,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
    };

    let Some(body) = text.strip_prefix('=') else {
        push(text, plain);
        return job;
    };
    let Ok(tokens) = tokenize(body) else {
        push(text, plain);
        return job;
    };

    push("=", plain);
    let mut copied = 0usize;
    let mut next = 0usize;
    for token in &tokens {
        let color = match token.kind {
            Tok::Cell(_) | Tok::ColSpan { .. } | Tok::RowSpan { .. } => {
                let color = REFERENCE_COLORS[next % REFERENCE_COLORS.len()];
                // A range is three tokens but one reference, so the colon does
                // not advance the cycle and neither does the second corner.
                next += 1;
                Some(color)
            }
            _ => None,
        };
        let Some(color) = color else { continue };
        push(&body[copied..token.at], plain);
        push(&body[token.at..token.end], color);
        copied = token.end;
    }
    push(&body[copied..], plain);
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(text: &str) -> Vec<String> {
        Editor::editing(CellRef::new(0, 0), text.to_string())
            .references()
            .iter()
            .map(|r| format!("{}:{}", r.start.to_a1(), r.end.to_a1()))
            .collect()
    }

    #[test]
    fn a_formula_names_the_ranges_the_grid_should_outline() {
        assert_eq!(refs("=A1+B2"), ["A1:A1", "B2:B2"]);
        assert_eq!(refs("=SUM(A1:B3)"), ["A1:B3"]);
    }

    #[test]
    fn a_reference_to_another_sheet_has_nowhere_to_be_drawn() {
        assert_eq!(refs("=Data!A1+B2"), ["B2:B2"]);
    }

    #[test]
    fn plain_text_and_half_typed_formulas_are_not_references() {
        assert!(refs("hello").is_empty());
        assert!(
            refs("=SUM(").is_empty(),
            "does not lex, so nothing is claimed"
        );
    }

    fn a1(s: &str) -> CellRef {
        CellRef::from_a1(s).expect("test address")
    }

    #[test]
    fn pointing_replaces_until_an_operator_locks_it_in() {
        let mut e = Editor::typing(a1("A1"), "=".into());
        assert!(e.can_point());
        e.point_at(a1("B2"));
        assert_eq!(e.text, "=B2");
        e.point_at(a1("C3"));
        assert_eq!(e.text, "=C3", "pointing again replaces, not appends");
        e.text.push('+'); // the user typed an operator: C3 is theirs now
        e.point_at(a1("D4"));
        assert_eq!(e.text, "=C3+D4");
        e.point_to(a1("E6"));
        assert_eq!(e.text, "=C3+D4:E6", "a drag stretches the live reference");
        e.point_to(a1("B2"));
        assert_eq!(e.text, "=C3+B2:D4", "corners normalize whichever way it is pulled");
    }

    #[test]
    fn plain_text_never_points_and_a_finished_reference_does_not_rearm() {
        let mut e = Editor::typing(a1("A1"), "hello ".into());
        assert!(!e.can_point());
        e.point_at(a1("B2"));
        assert_eq!(e.text, "hello ", "a click will commit instead");
        // `=B2` typed by hand: no pointing is live and nothing is armed, so
        // a click commits, exactly as Excel does after a typed reference.
        let typed = Editor::typing(a1("A1"), "=B2".into());
        assert!(!typed.can_point());
    }

    #[test]
    fn f4_cycles_the_dollars_the_way_excel_does() {
        let mut e = Editor::typing(a1("A1"), "=B2".into());
        for want in ["=$B$2", "=B$2", "=$B2", "=B2"] {
            e.cycle_reference();
            assert_eq!(e.text, want);
        }
        // Both corners of a range cycle together, and the paren stays put.
        let mut r = Editor::typing(a1("A1"), "=SUM(A1:B2)".into());
        r.cycle_reference();
        assert_eq!(r.text, "=SUM($A$1:$B$2)");
        r.cycle_reference();
        assert_eq!(r.text, "=SUM(A$1:B$2)");
        // Nothing reference-shaped, nothing changed.
        let mut n = Editor::typing(a1("A1"), "=1+2".into());
        n.cycle_reference();
        assert_eq!(n.text, "=1+2", "digits alone are not a reference");
    }

    #[test]
    fn highlighting_keeps_every_character_of_the_text() {
        // The colouring must never eat or duplicate what the user typed.
        for text in ["=A1+SUM(B2:C3)*2", "=not a formula(", "plain", "="] {
            let job = highlight(text, egui::FontId::monospace(12.0), egui::Color32::WHITE);
            assert_eq!(job.text, text, "round trip of {text:?}");
        }
    }
}
