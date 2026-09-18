//! Assist in Calx's window: the pane on the right, where the chart inspector
//! sits; the keys and rows that open it; the helper's tool calls, run against
//! the workbook a frame at a time; and what a whole request leaves behind — a
//! wash over the cells it wrote and one entry in the undo history.
//!
//! **One request is one thing to undo.** Calx has no tracked changes and
//! grows none for this: each tool call lands at once, as Excel's own tools
//! do, and the window keeps the changes that take them back. When the request
//! ends they go on the undo stack together, labelled with what the person
//! asked, so that one Ctrl+Z — or Undo on the card — gives back the workbook
//! as it was before the assistant touched it.
//!
//! **The inspector wins the slot.** Both panes stand on the right, and a
//! chart being selected is the person pointing at the chart: the inspector
//! shows, and Assist waits until the chart is let go.

use calx::assistant::{self, About, Done};
use ss_formula::edit::Change;
use ss_model::{CellRange, CellRef};
use ui_kit::assist::{Asked, Assist, Card, Chosen, Offer, Prepared, Ran, Scope, Verb};
use ui_kit::egui;

use crate::Calx;

/// The quick verbs, above the composer: what people ask for without knowing
/// how to ask. Each is a request with the scope already set.
pub(crate) const VERBS: [(&str, &str); 4] = [
    (
        "Explain the selection",
        "Explain what these cells hold and what the formulas do.",
    ),
    (
        "Add a total",
        "Add a total for these figures, in the first empty cell after them, and label it.",
    ),
    (
        "Clean up this column",
        "Make the values in this column consistent with one another, and say what you changed.",
    ),
    (
        "Fill in the pattern",
        "Carry on the pattern in these cells for the rows that are still empty.",
    ),
];

/// The rows Calx adds to the pane's `⋯` menu.
pub(crate) const MENU: [&str; 1] = ["&Undo the assistant's changes"];

/// How long the cells a request wrote stay washed, in seconds.
pub(crate) const WASH: f64 = 4.0;

/// A card in the pane, and where the change it takes back sits.
pub(crate) struct Carded {
    pub card: u64,
    /// How deep the undo stack was once its entry was on it. A card whose
    /// entry is no longer the top one cannot be undone on its own — undoing
    /// it would take back whatever was done since — so it says so instead.
    pub depth: usize,
    pub settled: bool,
}

impl Calx {
    pub(crate) fn assist_mut(&mut self) -> &mut Assist {
        // Made the first time it is wanted: a window nobody asks for help in
        // never reads Assist's settings, and a test that does not want one
        // never gets a helper.
        self.assist
            .get_or_insert_with(|| Box::new(Assist::new(assistant::setup())))
    }

    pub(crate) fn assistant_working(&self) -> bool {
        self.assist.as_ref().is_some_and(|assist| assist.is_busy())
    }

    pub(crate) fn show_assist(&mut self) {
        self.assisting = true;
        self.assist_keyboard = true;
        self.assist_mut().focus();
    }

    pub(crate) fn hide_assist(&mut self) {
        self.assisting = false;
        self.assist_keyboard = false;
    }

    pub(crate) fn toggle_assist(&mut self) {
        match self.assisting {
            true => self.hide_assist(),
            false => self.show_assist(),
        }
    }

    /// The selection, as a range and as the scope chip that follows it.
    pub(crate) fn assist_scope(&self) -> (CellRange, About) {
        let range = self.grid.selection.active_range();
        let about = match range.start == range.end {
            true => About::Cell,
            false => About::Selection,
        };
        (range, about)
    }

    /// The right-hand side: the chart inspector while a chart is selected,
    /// and Assist otherwise. One panel at a time, and the inspector first,
    /// because a chart being selected is the person pointing at the chart.
    pub(crate) fn right_side(&mut self, ui: &mut egui::Ui) {
        if self.chart_panel(ui) {
            return;
        }
        if !self.assisting {
            return;
        }
        let scopes: Vec<Scope> = About::ALL
            .iter()
            .map(|about| Scope {
                name: about.name(),
                words: None,
            })
            .collect();
        let (_, following) = self.assist_scope();
        let verbs: Vec<Verb> = VERBS
            .iter()
            .map(|(label, asks)| Verb { label, asks })
            .collect();
        let offer = Offer {
            scopes: &scopes,
            following: About::ALL.iter().position(|a| *a == following).unwrap_or(0),
            verbs: &verbs,
            menu: &MENU,
            tabs: &[],
            tab: 0,
        };
        let mut chosen = None;
        egui::Panel::right("calx-assist")
            .default_size(ui_kit::assist::WIDTH)
            .resizable(true)
            .frame(egui::Frame::new().fill(ui.visuals().window_fill))
            .show(ui, |ui| {
                chosen = self.assist_mut().show(ui, &offer);
            });
        match chosen {
            Some(Chosen::Ask(asked)) => self.ask_assistant(asked),
            Some(Chosen::Card { card, action }) => self.assist_card(card, action),
            Some(Chosen::Menu(0)) => self.undo_the_assistants_changes(),
            Some(Chosen::Leave) => self.assist_keyboard = false,
            Some(Chosen::Close) => self.hide_assist(),
            _ => {}
        }
    }

    /// Puts the person's request into words and sends it.
    fn ask_assistant(&mut self, asked: Asked) {
        let (range, following) = self.assist_scope();
        let about = About::ALL.get(asked.scope).copied().unwrap_or(following);
        let sheet = self.grid.sheet_index;
        let request = assistant::request(&self.doc.workbook, sheet, range, about, &asked.words);
        let leaves = match about {
            About::Cell => format!(
                "Cell {} and the first rows of the sheet",
                range.start.to_a1()
            ),
            About::Selection => format!(
                "The cells {} and the first rows of the sheet",
                assistant::a1(range)
            ),
            About::Sheet => "The first rows of the sheet, and what is selected".to_owned(),
            About::Workbook => "The sheets' names and the first rows of this one".to_owned(),
        };
        // The workbook it is about, as it stands: a call that arrives after
        // the person has edited is refused rather than run against cells that
        // have moved.
        self.assist_seen = self.edits;
        self.asked_words = asked.words.clone();
        if let Some(assist) = self.assist.as_mut() {
            assist.send(Prepared::new(&asked, request, leaves));
        }
    }

    /// A frame's worth of the helper: one tool call, run and answered, and
    /// the card when the request has ended.
    pub(crate) fn tend_assist(&mut self, ctx: &egui::Context) {
        if self.assist.is_none() {
            return;
        }
        self.retire_cards();
        let now = ctx.input(|i| i.time);
        let call = self
            .assist
            .as_mut()
            .and_then(|assist| assist.poll(ctx))
            .filter(|call| {
                self.assist
                    .as_ref()
                    .is_some_and(|assist| assist.wanted(call))
            });
        if let Some(call) = call {
            self.run_assist_call(call, now);
        }
        // The request has ended: what it changed becomes one entry, with a
        // card that takes it back.
        let working = self.assistant_working();
        if self.assist_working && !working {
            let asked = std::mem::take(&mut self.asked_words);
            self.assist_request_ended(&asked);
        }
        self.assist_working = working;
    }

    fn run_assist_call(&mut self, call: ui_kit::assist::Call, now: f64) {
        let stale = assistant::edits(&call.tool) && self.edits != self.assist_seen;
        let sheet = self.grid.sheet_index;
        let done = match stale {
            true => assistant::changed_meanwhile(&call.tool),
            false => assistant::run(
                &mut self.doc.workbook,
                sheet,
                &call.tool,
                &crate::protection_refusal,
            ),
        };
        self.land_assist(sheet, done, &call, now);
    }

    /// What a tool call left behind: the entry it adds to the request's undo,
    /// the cells it wrote, and the line the transcript shows.
    fn land_assist(&mut self, sheet: usize, done: Done, call: &ui_kit::assist::Call, now: f64) {
        if let Some(undo) = done.undo {
            match self.assist_entry.as_mut() {
                Some(entry) => assistant::gather(entry, undo),
                None => self.assist_entry = Some(undo),
            }
            self.edited = true;
            self.grid.invalidate();
        }
        if let Some(line) = &done.line {
            self.assist_did.push(line.clone());
        }
        if !done.wrote.is_empty() {
            self.assist_wrote.extend(done.wrote.iter().copied());
            // Every cell the request has written so far, not just this call's:
            // a total column written and then filled is one thing that
            // happened, and the person should see all of it.
            let wrote = self.assist_wrote.clone();
            self.wash_cells(sheet, &wrote, now);
        }
        self.assist_over.extend(done.overwrote.iter().copied());
        if let Some(added) = done.added {
            // The grid is moved to it, and no more: `show_sheet` commits an
            // open cell editor, which would put an edit of the person's in
            // the middle of the request's one entry.
            self.grid.open_sheet(&self.doc.workbook, added);
            self.status = self.doc.workbook.sheets[added].name.clone();
        }
        // What the helper did is the workbook it goes on from.
        self.assist_seen = self.edits;
        let mut ran = Ran::new(done.result);
        if let Some(line) = done.line {
            ran = ran.said(line);
        }
        if let Some(assist) = self.assist.as_mut() {
            assist.answer(call, ran);
        }
    }

    /// The cells a request wrote, washed in the accent for a few seconds.
    fn wash_cells(&mut self, sheet: usize, cells: &[CellRef], now: f64) {
        let mut ranges: Vec<CellRange> = Vec::new();
        for at in cells {
            match ranges
                .iter_mut()
                .find(|range| range.start.col == at.col && range.end.row + 1 == at.row)
            {
                Some(range) => range.end.row = at.row,
                None => ranges.push(CellRange::new(*at, *at)),
            }
        }
        self.grid.wash = Some(calx::grid::Wash {
            sheet,
            ranges,
            until: now + WASH,
        });
    }

    /// The request has ended: its changes become one entry, and its card says
    /// what it did.
    pub(crate) fn assist_request_ended(&mut self, asked: &str) {
        let empty = self
            .assist_entry
            .as_ref()
            .is_none_or(|entry| entry.is_empty());
        let Some(entry) = self.assist_entry.take().filter(|_| !empty) else {
            self.assist_wrote.clear();
            self.assist_over.clear();
            self.assist_did.clear();
            return;
        };
        let wrote = std::mem::take(&mut self.assist_wrote);
        let over = std::mem::take(&mut self.assist_over);
        let did = std::mem::take(&mut self.assist_did);
        let label = format!("Assistant: {}", first_words(asked));
        let entry = Change::new(label.clone(), entry.patches);
        self.undo.push(entry);
        self.redo.clear();
        self.cards += 1;
        let card = self.cards;
        self.carded.push(Carded {
            card,
            depth: self.undo.len(),
            settled: false,
        });
        if let Some(assist) = self.assist.as_mut() {
            assist.add_card(Card {
                id: card,
                title: label,
                body: what_it_did(&wrote, &over, &did),
                actions: vec!["Undo".to_owned()],
                verdict: None,
            });
        }
    }

    /// A card whose change has been undone another way, or overtaken by an
    /// edit since, loses its button and says which.
    fn retire_cards(&mut self) {
        let depth = self.undo.len();
        let mut settling: Vec<(u64, &'static str)> = Vec::new();
        for carded in self.carded.iter_mut().filter(|carded| !carded.settled) {
            let verdict = match depth.cmp(&carded.depth) {
                std::cmp::Ordering::Less => Some("Undone"),
                std::cmp::Ordering::Greater => Some("No longer the last change"),
                std::cmp::Ordering::Equal => None,
            };
            if let Some(verdict) = verdict {
                carded.settled = true;
                settling.push((carded.card, verdict));
            }
        }
        if let Some(assist) = self.assist.as_mut() {
            for (card, verdict) in settling {
                assist.settle(card, verdict);
            }
        }
    }

    /// Undo on a card: the request's own entry, while it is the last one.
    fn assist_card(&mut self, card: u64, _action: usize) {
        let Some(carded) = self.carded.iter().find(|carded| carded.card == card) else {
            return;
        };
        let verdict = match carded.depth == self.undo.len() {
            true => {
                self.undo();
                "Undone"
            }
            false => "No longer the last change",
        };
        if let Some(carded) = self.carded.iter_mut().find(|carded| carded.card == card) {
            carded.settled = true;
        }
        if let Some(assist) = self.assist.as_mut() {
            assist.settle(card, verdict);
        }
    }

    /// The pane's menu row: the assistant's last change, taken back.
    pub(crate) fn undo_the_assistants_changes(&mut self) {
        let last = self
            .carded
            .iter()
            .rev()
            .find(|carded| !carded.settled)
            .map(|carded| carded.card);
        if let Some(card) = last {
            self.assist_card(card, 0);
            return;
        }
        // A card settled by an undo says so for good, but a redo puts those
        // same changes back: what is on top of the stack is what decides.
        let assistants = self
            .undo
            .last()
            .is_some_and(|entry| entry.label.starts_with("Assistant: "));
        match assistants {
            true => self.undo(),
            false => self.status = "The assistant has nothing left to undo".to_owned(),
        }
    }

    /// The workbook the conversation was about is gone: the request stops and
    /// the transcript goes with it.
    pub(crate) fn end_assist_conversation(&mut self) {
        if let Some(assist) = self.assist.as_mut() {
            assist.clear();
        }
        self.carded.clear();
        self.assist_entry = None;
        self.assist_wrote.clear();
        self.assist_over.clear();
        self.assist_did.clear();
        self.asked_words.clear();
        self.grid.wash = None;
    }
}

/// The first words of a request, for the undo entry's label.
fn first_words(asked: &str) -> String {
    let words: Vec<&str> = asked.split_whitespace().take(6).collect();
    let short = words.join(" ");
    match short.len() < asked.trim().len() {
        true => format!("{short}…"),
        false => short,
    }
}

/// What a request did, in the person's terms: how many cells, where, which of
/// them held something before — and, where it wrote no cells at all, what its
/// tools said they did instead, since a card that reads "nothing was written"
/// over three deleted rows tells the person the opposite of the truth.
fn what_it_did(wrote: &[CellRef], over: &[CellRef], did: &[String]) -> String {
    if wrote.is_empty() {
        let said: Vec<String> = did
            .iter()
            .filter(|line| !line.starts_with("read "))
            .map(|line| {
                let mut words = line.chars();
                match words.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + words.as_str(),
                    None => String::new(),
                }
            })
            .collect();
        return match said.is_empty() {
            true => "Nothing was written.".to_owned(),
            false => format!("{}.", said.join("; ")),
        };
    }
    let columns: Vec<String> = {
        let mut seen: Vec<u32> = Vec::new();
        for at in wrote {
            if !seen.contains(&at.col) {
                seen.push(at.col);
            }
        }
        seen.iter()
            .map(|col| ss_model::cell::column_name(*col))
            .collect()
    };
    let mut said = format!(
        "Wrote {} in {}",
        match wrote.len() {
            1 => "1 cell".to_owned(),
            n => format!("{n} cells"),
        },
        columns.join(", ")
    );
    if !over.is_empty() {
        let named: Vec<String> = over.iter().take(12).map(|at| at.to_a1()).collect();
        said.push_str(&format!(
            ". {} held something before: {}",
            match over.len() {
                1 => "One cell".to_owned(),
                n => format!("{n} cells"),
            },
            named.join(", ")
        ));
    }
    said
}

#[cfg(test)]
mod tests;
