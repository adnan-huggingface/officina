//! Assist in Scriva's window: the pane on the right, in the place Review
//! takes, a tab of it; the keys and rows that open it; the helper's tool
//! calls, run against the document a frame at a time; and what the window
//! says about proposals — a notice when they arrive with the pane put away, a
//! chip while the helper works, and a line when a document is saved with
//! some still open.
//!
//! The pane is made the first time it is wanted, so a window nobody asks
//! for help in never reads Assist's settings, and a test never reaches a
//! helper it did not give the pane itself.

use ui_kit::assist::{Asked, Assist, Card, Chosen, Offer, Prepared, Ran, Scope as Chip, Verb};

use super::*;
use crate::assistant::{self, About, Kind};

/// The quick verbs: the chips above the composer, and — all but the one that
/// needs words of the person's own — the right-click menu's rows.
pub(crate) const VERBS: [(&str, &str); 5] = [
    ("Improve the wording", "Improve the wording."),
    ("Fix spelling and grammar", "Fix the spelling and grammar."),
    ("Make it shorter", "Make it shorter."),
    ("Summarize", "Summarize it, in your reply."),
    ("Translate\u{2026}", "Translate it into "),
];

/// The pane's own rows in its menu, after its Settings and Clear.
const MENU: [&str; 2] = [
    "&Accept All the Assistant's Changes",
    "&Reject All the Assistant's Changes",
];

/// The panes that share the right-hand side, as both headers name them.
pub(crate) const TABS: [&str; 2] = ["Review", "Assist"];

/// What a notice about unseen proposals begins with, to find it again.
const UNSEEN: &str = "The assistant ";

/// A card in the pane, and what it settles.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Carded {
    pub card: u64,
    pub kind: Kind,
    /// Its buttons are gone.
    pub settled: bool,
}

impl Scriva {
    /// The pane, made the first time it is wanted.
    pub(crate) fn assist_mut(&mut self) -> &mut Assist {
        self.assist
            .get_or_insert_with(|| Box::new(Assist::new(assistant::setup())))
    }

    /// Whether the helper is answering a request.
    pub(crate) fn assistant_working(&self) -> bool {
        self.assist
            .as_ref()
            .is_some_and(|assist| assist.is_working())
    }

    /// Opens the pane in the right-hand side, with the keyboard in its
    /// composer, and takes down the notice that sent the person there.
    pub(crate) fn show_assist(&mut self) {
        self.assisting = true;
        self.reviewer = false;
        self.keyboard = Keyboard::Assist;
        self.assist_mut().focus();
        self.unseen = (0, 0);
        self.notices
            .retain(|notice| !notice.text.starts_with(UNSEEN));
    }

    /// Opens the Review pane in the right-hand side.
    pub(crate) fn show_reviewer(&mut self) {
        self.reviewer = true;
        self.assisting = false;
    }

    /// Puts the pane away, from the menu row that says it is open.
    pub(crate) fn hide_assist(&mut self) {
        self.assisting = false;
    }

    /// Ctrl+Alt+A: opens the pane, or gives it the keyboard; from the pane
    /// itself, puts it away.
    pub(crate) fn toggle_assist(&mut self) {
        match self.assisting && self.keyboard == Keyboard::Assist {
            true => self.assisting = false,
            false => self.show_assist(),
        }
    }

    /// The text's selection: the one the person is making, or, while a
    /// header is open, the one they left in the text.
    fn text_selection(&self) -> Selection {
        let selection = match self.scope {
            wp_model::Scope::Body => self.selection,
            _ => self.left_behind.unwrap_or_default(),
        };
        let clamp_in = |caret| clamp(&self.document, wp_model::Scope::Body, caret);
        Selection {
            anchor: clamp_in(selection.anchor),
            head: clamp_in(selection.head),
        }
    }

    /// A request, put into words and handed to the pane.
    pub(crate) fn ask_assistant(&mut self, asked: Asked) {
        let about = About::ALL
            .get(asked.scope)
            .copied()
            .unwrap_or(About::Paragraph);
        let words = self.word_count();
        let sent = assistant::request(
            &self.document,
            self.text_selection(),
            about,
            &asked.words,
            words,
        );
        let leaves = assistant::leaves(about, words);
        // What the helper is about to be shown is the document as it stands;
        // an edit of the person's after this moves its numbers.
        if !self.assist_mut().is_busy() {
            self.assist_seen = self.history.generation();
        }
        self.assist_mut().send(Prepared::new(&asked, sent, leaves));
    }

    /// A quick verb from the right-click menu: sent at once, about the
    /// selection or the caret's paragraph, with the pane open to show it.
    pub(crate) fn assist_verb(&mut self, index: usize) {
        let Some((_, asks)) = VERBS.get(index) else {
            return;
        };
        self.show_assist();
        let scope = match self.text_selection().is_empty() {
            true => 1,
            false => 0,
        };
        self.ask_assistant(Asked {
            words: (*asks).to_owned(),
            scope,
            effort: ::assist::Effort::Low,
        });
    }

    /// Runs what the helper asked for, and settles the cards whose changes
    /// have left the text: called on every frame, whether the pane shows or
    /// not, before the page is laid out.
    pub(super) fn tend_assist(&mut self, ctx: &egui::Context) {
        if self.assist.is_none() {
            return;
        }
        // Cards whose changes have gone are settled before anything else, so
        // that a comment made now cannot be taken for one deleted since.
        if self.cards_for != self.stamp {
            self.cards_for = self.stamp;
            self.retire_cards();
        }
        let Some(assist) = self.assist.as_mut() else {
            return;
        };
        if let Some(call) = assist.poll(ctx) {
            if assist.wanted(&call) {
                self.run_assist_call(call);
            }
        }
    }

    fn run_assist_call(&mut self, call: ui_kit::assist::Call) {
        // The paragraph numbers the helper has are the ones it was shown. A
        // person who has edited the document since has moved them, so nothing
        // it asks for is done until it has been shown the document again.
        let stale = assistant::edits(&call.tool) && self.history.generation() != self.assist_seen;
        let done = match stale {
            true => assistant::changed_meanwhile(&call.tool),
            false => {
                let author = assistant::author_at(&self.proposal_time());
                assistant::run(&mut self.document, &mut self.history, &call.tool, &author)
            }
        };
        let mut ran = Ran::new(done.result);
        if let Some(line) = done.line {
            ran = ran.said(line);
        }
        if done.proposal.is_some() {
            // Where the caret stood, the text may have moved under it.
            let moved = done.moved;
            self.selection = Selection {
                anchor: moved.caret(self.selection.anchor),
                head: moved.caret(self.selection.head),
            };
            if let Some(left) = self.left_behind {
                self.left_behind = Some(Selection {
                    anchor: moved.caret(left.anchor),
                    head: moved.caret(left.head),
                });
            }
        }
        if let Some(proposal) = done.proposal {
            self.cards += 1;
            let card = self.cards;
            let actions: &[&str] = match proposal.kind {
                Kind::Changes(_) => &["Accept", "Reject", "Show"],
                Kind::Comment { .. } => &["Show", "Delete"],
            };
            ran = ran.card(Card {
                id: card,
                title: proposal.title,
                body: proposal.body,
                actions: actions.iter().map(|action| (*action).to_owned()).collect(),
                verdict: None,
            });
            if !self.assisting {
                self.note_unseen(&proposal.kind);
            }
            self.carded.push(Carded {
                card,
                kind: proposal.kind,
                settled: false,
            });
            self.selection = Selection {
                anchor: clamp(&self.document, self.scope, self.selection.anchor),
                head: clamp(&self.document, self.scope, self.selection.head),
            };
            self.changed();
            self.cards_for = self.stamp;
        }
        // What the helper did is the document it goes on from.
        self.assist_seen = self.history.generation();
        if let Some(assist) = self.assist.as_mut() {
            assist.answer(&call, ran);
        }
    }

    /// The time the next proposal carries: now, or a second after the last
    /// one, and none a change in the text already carries.
    fn proposal_time(&mut self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0);
        let mut at = now.max(self.proposal_time + 1);
        while assistant::standing(&self.document, &assistant::time_of(at)) {
            at += 1;
        }
        self.proposal_time = at;
        assistant::time_of(at)
    }

    /// A proposal arrived with the pane put away: the notice under the
    /// toolbar says how many, with Show.
    fn note_unseen(&mut self, kind: &Kind) {
        match kind {
            Kind::Changes(_) => self.unseen.0 += 1,
            Kind::Comment { .. } => self.unseen.1 += 1,
        }
        let plural = |count: usize, what: &str| match count {
            1 => format!("1 {what}"),
            n => format!("{n} {what}s"),
        };
        let text = match self.unseen {
            (changes, 0) => format!("{UNSEEN}proposed {}", plural(changes, "change")),
            (0, comments) => format!("{UNSEEN}left {}", plural(comments, "comment")),
            (changes, comments) => format!(
                "{UNSEEN}proposed {} and left {}",
                plural(changes, "change"),
                plural(comments, "comment")
            ),
        };
        self.notices
            .retain(|notice| !notice.text.starts_with(UNSEEN));
        self.post_notice(text, Some(("Show", Command::ShowAssist)));
    }

    /// Cards whose changes are no longer open — settled in Review, undone —
    /// lose their buttons.
    fn retire_cards(&mut self) {
        let Some(assist) = self.assist.as_mut() else {
            return;
        };
        for carded in self.carded.iter_mut().filter(|carded| !carded.settled) {
            let gone =
                match &carded.kind {
                    Kind::Changes(date) => !assistant::standing(&self.document, date),
                    Kind::Comment { id, date } => !self.document.comments.iter().any(|comment| {
                        comment.id == *id && comment.date.as_deref() == Some(&**date)
                    }),
                };
            if gone {
                carded.settled = true;
                assist.settle(carded.card, "No longer open");
            }
        }
    }

    /// A button on a card.
    fn assist_card(&mut self, card: u64, action: usize) {
        let Some(carded) = self.carded.iter().find(|c| c.card == card).cloned() else {
            return;
        };
        let verdict = match (&carded.kind, action) {
            (Kind::Changes(date), 0 | 1) => {
                let how = match action {
                    0 => crate::revise::Resolve::Accept,
                    _ => crate::revise::Resolve::Reject,
                };
                match assistant::settle(&mut self.document, &mut self.history, date, how) {
                    0 => "No longer open",
                    _ if action == 0 => "Accepted",
                    _ => "Rejected",
                }
            }
            (Kind::Changes(date), _) => {
                let first = crate::revise::tracked(&self.document)
                    .into_iter()
                    .find(|change| change.mark.date.as_deref() == Some(date))
                    .map(|change| change.mark);
                if let Some(mark) = first {
                    self.run(Command::GoToChange(mark));
                }
                return;
            }
            (Kind::Comment { id, .. }, 0) => {
                self.run(Command::GoToComment(*id));
                return;
            }
            (Kind::Comment { id, date }, _) => {
                // Its own comment, by the time it was made: an id a deleted
                // comment had goes to the next one, and a card deletes only
                // the comment it is about.
                let its_own =
                    self.document.comments.iter().any(|comment| {
                        comment.id == *id && comment.date.as_deref() == Some(&**date)
                    });
                match its_own
                    && crate::revise::delete_comment(&mut self.document, &mut self.history, *id)
                {
                    true => "Deleted",
                    false => "No longer open",
                }
            }
        };
        self.settle_card(card, verdict);
        self.selection = Selection::at(clamp(&self.document, self.scope, self.caret()));
        self.changed();
        self.cards_for = self.stamp;
    }

    fn settle_card(&mut self, card: u64, verdict: &str) {
        if let Some(carded) = self.carded.iter_mut().find(|c| c.card == card) {
            carded.settled = true;
        }
        if let Some(assist) = self.assist.as_mut() {
            assist.settle(card, verdict);
        }
    }

    /// Accept all, or Reject all, of the assistant's changes, and nobody
    /// else's.
    pub(crate) fn settle_assistant(&mut self, how: crate::revise::Resolve) {
        let open: Vec<u64> = self
            .carded
            .iter()
            .filter(|carded| {
                !carded.settled
                    && matches!(&carded.kind, Kind::Changes(date)
                        if assistant::standing(&self.document, date))
            })
            .map(|carded| carded.card)
            .collect();
        if assistant::settle_all(&mut self.document, &mut self.history, how) == 0 {
            self.say("The assistant has no changes open");
            return;
        }
        let verdict = match how {
            crate::revise::Resolve::Accept => "Accepted",
            crate::revise::Resolve::Reject => "Rejected",
        };
        for card in open {
            self.settle_card(card, verdict);
        }
        self.selection = Selection::at(clamp(&self.document, self.scope, self.caret()));
        self.changed();
        self.cards_for = self.stamp;
    }

    /// Stops the helper and clears the conversation: the document it was
    /// about is gone.
    pub(super) fn end_conversation(&mut self) {
        if let Some(assist) = self.assist.as_mut() {
            assist.clear();
        }
        self.carded.clear();
        self.unseen = (0, 0);
    }

    /// The right-hand side: Review or Assist, whichever is open, at one
    /// width, so that changing tabs does not move the page.
    pub(super) fn right_side(&mut self, ui: &mut egui::Ui) {
        if !self.assisting && self.keyboard == Keyboard::Assist {
            self.give_keyboard(Keyboard::Document, ui.ctx());
        }
        if !self.reviewer {
            if self.keyboard == Keyboard::Review {
                self.keyboard = Keyboard::Document;
            }
            // A comment being written waits while Assist has the place; only
            // closing Review throws it away.
            if !self.assisting {
                self.draft = None;
            }
        }
        if !self.reviewer && !self.assisting {
            return;
        }
        let command = egui::Panel::right("scriva-right")
            .default_size(ui_kit::assist::WIDTH)
            .resizable(true)
            .frame(egui::Frame::new().fill(ui_kit::theme::CHROME))
            .show(ui, |ui| match self.assisting {
                true => self.assist_pane(ui),
                false => self.review_pane(ui),
            })
            .inner;
        if let Some(command) = command {
            self.run(command);
        }
    }

    /// The pane, and what was chosen on it.
    fn assist_pane(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let ctx = ui.ctx().clone();
        let selection = self.text_selection();
        let selected_words = self
            .selected_plain_text()
            .filter(|_| !selection.is_empty())
            .map(|text| crate::app::word_count::Count::of_text(&text).words);
        let words = self.word_count();
        let scopes = About::ALL.map(|about| Chip {
            name: about.name(),
            words: match about {
                About::Selection => selected_words,
                About::Paragraph => None,
                About::Document => Some(words),
            },
        });
        let verbs: Vec<Verb> = VERBS
            .iter()
            .map(|(label, asks)| Verb { label, asks })
            .collect();
        let offer = Offer {
            scopes: &scopes,
            following: match selection.is_empty() {
                true => 1,
                false => 0,
            },
            verbs: &verbs,
            menu: &MENU,
            tabs: &TABS,
            tab: 1,
        };
        let chosen = self.assist_mut().show(ui, &offer);
        // The keyboard is the pane's once anything in it has the focus — the
        // composer, a card's button, the first-run card's — and stays the
        // pane's until another stop takes it: a click on the page, the
        // toolbar, the find bar, F6, Escape. A menu or a box over the window
        // may hold the focus meanwhile; once nothing holds it, the composer
        // takes it back, or what was typed next would go nowhere.
        let pane = ui.max_rect();
        let focused = ctx
            .memory(|m| m.focused())
            .and_then(|id| ctx.read_response(id))
            .map(|widget| pane.contains_rect(widget.rect));
        match focused {
            Some(true) => self.keyboard = Keyboard::Assist,
            None if self.keyboard == Keyboard::Assist => {
                if let Some(assist) = &self.assist {
                    assist.keep_keyboard(&ctx);
                }
            }
            _ => {}
        }
        match chosen? {
            Chosen::Ask(asked) => {
                self.ask_assistant(asked);
                None
            }
            Chosen::Card { card, action } => {
                self.assist_card(card, action);
                None
            }
            Chosen::Menu(0) => Some(Command::AcceptAssistant),
            Chosen::Menu(_) => Some(Command::RejectAssistant),
            Chosen::Tab(_) => Some(Command::Reviewer),
            Chosen::Leave => {
                self.give_keyboard(Keyboard::Document, &ctx);
                None
            }
            Chosen::Close => {
                self.assisting = false;
                None
            }
            Chosen::StopDownload => None,
        }
    }
}
