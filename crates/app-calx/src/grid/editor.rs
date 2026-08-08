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
}

impl Editor {
    pub fn typing(at: CellRef, seed: String) -> Self {
        Editor {
            at,
            text: seed,
            mode: Mode::Enter,
            fresh: true,
        }
    }

    pub fn editing(at: CellRef, existing: String) -> Self {
        Editor {
            at,
            text: existing,
            mode: Mode::Edit,
            fresh: true,
        }
    }

    /// True when the text is a formula, which is when references are live.
    pub fn is_formula(&self) -> bool {
        self.text.starts_with('=')
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

    #[test]
    fn highlighting_keeps_every_character_of_the_text() {
        // The colouring must never eat or duplicate what the user typed.
        for text in ["=A1+SUM(B2:C3)*2", "=not a formula(", "plain", "="] {
            let job = highlight(text, egui::FontId::monospace(12.0), egui::Color32::WHITE);
            assert_eq!(job.text, text, "round trip of {text:?}");
        }
    }
}
