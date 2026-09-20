//! Assist's ⋯ menu ▸ Settings: the helper, and what it needs, in one box.
//!
//! The same choices as the first-run card, in its words, and under the one
//! chosen what that helper needs: for Claude how it signs in, the key, which
//! of the three answers — in words, with what a paragraph costs on each —
//! whether a request it declines goes to Anthropic's fallback, and what this
//! session has spent; for Ollama and another service the address, the key and
//! the model's name. A key typed here is checked, with one request that costs
//! nothing, before it is kept, and the box says what the service answered. A
//! key is never painted: its field shows dots.

use std::sync::Arc;

use std::path::Path;

use ::assist::models::{claude_model, cost_words, CLAUDE_MODELS};
use ::assist::{Choice, ClaudeLogin, Failure, Row, Settings, Usage};
use eframe::egui;

use super::choosing::model_words;
use super::request::{count_again, counting, Awaited, Background, Drain};
use super::Reach;
use crate::{dialog, theme};

const ID: &str = "ui-kit-assist-settings";

/// The id of the key field, which takes the keyboard when the box is opened
/// to paste one.
pub(crate) fn key_id() -> egui::Id {
    egui::Id::new("ui-kit-assist-key")
}

pub(crate) struct SettingsBox {
    /// The file as it was when the box opened: what the edit is an edit of.
    opened: Settings,
    pub draft: Settings,
    /// What the computer was found to have: the sign-ins offered beside a
    /// pasted key, and Ollama's models.
    found: Vec<Row>,
    /// Whether `found` is from a look, rather than nothing yet.
    looked: bool,
    /// A look of the box's own, when it opened with nothing found: Ollama's
    /// models are worth listing, and a look takes a moment.
    looking: Option<Background<(Vec<Row>, usize)>>,
    /// A look that has just arrived, for the pane to keep.
    arrived: Option<Vec<Row>>,
    /// The check under way, the edit it is a check of, and the settings it
    /// asked about: the edit onto the file as the file was then.
    checking: Option<(Checking, Edit, Settings)>,
    /// Why the settings were not kept: the service's answer, a choice that
    /// cannot answer yet, a file that could not be written.
    refused: Option<String>,
    focus_key: bool,
}

/// A check's thread: what the service answered, and the refusals the thread
/// counted.
type Checking = Background<(Result<String, Failure>, usize)>;

/// How the box closed.
pub(crate) enum Closed {
    Cancelled,
    /// Download the helper on this computer, or take it off again — asked
    /// for from the box, done by the pane, which owns the thread.
    Download,
    Remove,
    /// To be kept: the settings to write, which are what was checked when
    /// anything was, and what the service answered.
    Saved {
        settings: Box<Settings>,
        checked: Option<String>,
    },
}

/// What the box changed: the file as the box opened on it, and what the
/// person made of it.
pub(crate) struct Edit {
    opened: Settings,
    edited: Settings,
}

impl Edit {
    /// The edit onto the file as it is now: each value the box changed, every
    /// other value as the file has it, and the helper the box showed, which is
    /// the one the person saved whoever chose another meanwhile.
    pub fn onto(&self, current: &Settings) -> Settings {
        let mut merged = self.edited.onto(&self.opened, current);
        merged.helper = self.edited.helper;
        merged
    }
}

impl SettingsBox {
    /// The box, on the file as it is (`opened`), showing `draft`. `found` is
    /// what a look found, or `None` when no look has been made, in which case
    /// the box makes one if it needs it.
    pub fn open(
        opened: Settings,
        draft: Settings,
        found: Option<Vec<Row>>,
        focus_key: bool,
    ) -> SettingsBox {
        SettingsBox {
            opened,
            draft,
            looked: found.is_some(),
            found: found.unwrap_or_default(),
            looking: None,
            arrived: None,
            checking: None,
            refused: None,
            focus_key,
        }
    }

    /// The threads a closed box leaves behind, still to report their
    /// refusals.
    pub fn leftovers(&mut self) -> Vec<Box<dyn Drain>> {
        let mut left: Vec<Box<dyn Drain>> = Vec::new();
        if let Some((checking, _, _)) = self.checking.take() {
            left.push(Box::new(checking));
        }
        if let Some(looking) = self.looking.take() {
            left.push(Box::new(looking));
        }
        left
    }

    /// What the box's own look found, once, for the pane to keep.
    pub fn found_now(&mut self) -> Option<Vec<Row>> {
        self.arrived.take()
    }

    /// Looks at the computer, once: for Ollama's models, and for whether a
    /// helper of its own can run on it.
    fn look(&mut self, ctx: &egui::Context, reach: &Arc<dyn Reach>) {
        if let Some(looking) = &self.looking {
            match looking.poll() {
                Awaited::Waiting => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Awaited::Done((rows, refused)) => {
                    count_again(refused);
                    self.looking = None;
                    self.found = rows.clone();
                    self.arrived = Some(rows);
                }
                Awaited::Gone => self.looking = None,
            }
            return;
        }
        // Whatever helper is chosen: the look also says whether this computer
        // can run a helper of its own, which decides whether that row is
        // offered at all.
        if self.looked {
            return;
        }
        self.looked = true;
        let reach = Arc::clone(reach);
        let wake = ctx.clone();
        self.looking = Some(Background::spawn(
            move || counting(|| reach.look()),
            move || wake.request_repaint(),
        ));
    }

    /// Says why the settings could not be kept, and stays up.
    pub fn refuse(&mut self, why: String) {
        self.refused = Some(why);
    }

    /// Draws the box, and says how it closed, if it did. The settings are
    /// kept in `path`, which is read again when Save is pressed and again
    /// when a check answers: what the box saves is its edit, onto the file as
    /// it is by then, and only once exactly that has been checked.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        path: Result<&Path, &String>,
        spent: Usage,
        cents: Option<f64>,
        reach: &Arc<dyn Reach>,
    ) -> Option<Closed> {
        if let Some((checking, _, _)) = &self.checking {
            match checking.poll() {
                Awaited::Waiting => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Awaited::Done((answer, refused)) => {
                    count_again(refused);
                    let (_, edit, asked) = self.checking.take().expect("a check under way");
                    match answer {
                        Ok(sentence) => {
                            let now = edit.onto(&on_file(path));
                            // The other window saved something the check did
                            // not ask about: that is asked in its turn, and
                            // this answer says nothing of it.
                            if needs_check(&now, &asked) {
                                self.check(ctx, reach, edit, now);
                            } else {
                                return Some(Closed::Saved {
                                    settings: Box::new(now),
                                    checked: Some(sentence),
                                });
                            }
                        }
                        Err(failure) => self.refused = Some(failure.sentence),
                    }
                }
                Awaited::Gone => {
                    self.checking = None;
                    self.refused = Some("The check ended without an answer. Try again.".into());
                }
            }
        }
        self.look(ctx, reach);
        let checking = self.checking.is_some();
        let mut answer = None;
        // Asked for on the local helper's row, and done by the pane, which
        // owns the thread: the box only says what the person pressed.
        let mut asked: Option<Closed> = None;
        egui::Modal::new(egui::Id::new(ID))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(dialog::WIDTH);
                    ui.label(
                        egui::RichText::new("Assist Settings").font(dialog::heading_font(16.0)),
                    );
                    ui.add_space(6.0);
                    ui.add_enabled_ui(!checking, |ui| {
                        self.helpers(ui);
                        match self.draft.helper {
                            Some(Choice::Local) => {
                                asked = self.on_this_computer(ui, reach);
                            }
                            Some(Choice::Claude) => self.claude(ui, spent, cents),
                            Some(Choice::Ollama) => self.ollama(ui, spent, cents),
                            Some(Choice::Service) => self.service(ui, spent, cents),
                            _ => {}
                        }
                    });
                    ui.add_space(6.0);
                    if checking {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0).color(theme::INK_SOFT));
                            ui.label(
                                egui::RichText::new("Asking whether it answers to this…")
                                    .color(theme::INK_SOFT),
                            );
                        });
                    } else if let Some(why) = &self.refused {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(why.as_str()).color(theme::INK_ERROR),
                            )
                            .wrap(),
                        );
                    }
                    let pressed = dialog::row(ui, |ui| {
                        let cancel = dialog::button(ui, "Cancel", false).clicked();
                        let save = ui
                            .add_enabled_ui(!checking, |ui| dialog::button(ui, "Save", true))
                            .inner
                            .clicked();
                        match (save, cancel) {
                            (true, _) => Some(true),
                            (_, true) => Some(false),
                            _ => None,
                        }
                    });
                    answer = pressed.or_else(|| dialog::answered(ui));
                });
            });
        if asked.is_some() {
            return asked;
        }
        match answer {
            // Cancel is always a way out, a check under way included: its
            // answer is simply not waited for.
            Some(false) => Some(Closed::Cancelled),
            Some(true) if !checking => self.save(ctx, path, reach),
            _ => None,
        }
    }

    /// The edit: the draft, with only the part for the helper chosen taken
    /// from it. What was typed for a helper not chosen — a key a service
    /// refused, then another helper picked — is not the person's to keep.
    fn edited(&self) -> Settings {
        let mut edited = self.draft.clone();
        if edited.helper != Some(Choice::Claude) {
            edited.claude = self.opened.claude.clone();
        } else if edited.claude.login != ClaudeLogin::Key {
            // Nor is a key typed and then left for another way of signing in:
            // the check asks about that way, not the key.
            edited.claude.key = self.opened.claude.key.clone();
        }
        if edited.helper != Some(Choice::Ollama) {
            edited.ollama = self.opened.ollama.clone();
        }
        if edited.helper != Some(Choice::Service) {
            edited.service = self.opened.service.clone();
        }
        edited
    }

    fn save(
        &mut self,
        ctx: &egui::Context,
        path: Result<&Path, &String>,
        reach: &Arc<dyn Reach>,
    ) -> Option<Closed> {
        // With nowhere to keep them, nothing is asked: a key checked and then
        // not kept is a key sent for nothing.
        if let Err(why) = path {
            self.refused = Some(format!("Assist's settings could not be saved: {why}."));
            return None;
        }
        let current = on_file(path);
        let edit = Edit {
            opened: self.opened.clone(),
            edited: self.edited(),
        };
        let merged = edit.onto(&current);
        match merged.helper {
            None => {
                self.refused = Some("Choose a helper first.".into());
                None
            }

            _ if !needs_check(&merged, &current) => Some(Closed::Saved {
                settings: Box::new(merged),
                checked: None,
            }),
            _ => {
                self.refused = None;
                self.check(ctx, reach, edit, merged);
                None
            }
        }
    }

    /// Asks the service about `asked`, which is `edit` onto the file.
    fn check(&mut self, ctx: &egui::Context, reach: &Arc<dyn Reach>, edit: Edit, asked: Settings) {
        let reach = Arc::clone(reach);
        let wake = ctx.clone();
        let question = asked.clone();
        self.checking = Some((
            Background::spawn(
                move || counting(|| reach.check(&question)),
                move || wake.request_repaint(),
            ),
            edit,
            asked,
        ));
    }

    fn helpers(&mut self, ui: &mut egui::Ui) {
        dialog::section(ui, "Helper");
        // A computer that cannot run a helper worth having is not offered
        // one here either: the row says why, where the choice would be.
        let cannot = self.found.iter().find_map(|row| match row {
            Row::NoLocal { because } => Some(because.clone()),
            _ => None,
        });
        for (choice, words) in [
            (Choice::Local, "A helper on this computer"),
            (Choice::Claude, "Claude, over the internet"),
            (Choice::Ollama, "Ollama"),
            (Choice::Service, "Another service (advanced)"),
        ] {
            if choice == Choice::Local {
                if let Some(because) = &cannot {
                    // A saved choice this computer cannot honour: taken back
                    // where the person can see why, not written over.
                    if self.draft.helper == Some(Choice::Local) {
                        self.draft.helper = None;
                        self.refused = Some(because.clone());
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(because.as_str())
                                .size(theme::TEXT_SMALL)
                                .color(theme::INK_SOFT),
                        )
                        .wrap(),
                    );
                    continue;
                }
            }
            if ui
                .radio_value(&mut self.draft.helper, Some(choice), words)
                .changed()
            {
                self.refused = None;
            }
        }
    }

    /// The helper on this computer: what it is, what it costs in disk, and
    /// the one button — download it, or take it off again.
    ///
    /// **What it will download is said before it downloads.** The model's
    /// name, its licence and its size are on the row, from the same constant
    /// the download checks what arrives against.
    fn on_this_computer(&mut self, ui: &mut egui::Ui, reach: &Arc<dyn Reach>) -> Option<Closed> {
        dialog::section(ui, "The helper on this computer");
        // The model: the one the settings name, or the one the look found
        // this computer can run.
        if self.draft.local.model.trim().is_empty() {
            if let Some(Row::Local(offered)) =
                self.found.iter().find(|row| matches!(row, Row::Local(_)))
            {
                self.draft.local.model = offered.folder.to_owned();
            }
        }
        let model = *self.draft.local.model();
        let have = reach.have(&model);
        let taken = reach.downloaded();
        let words = match have {
            true => format!(
                "{} ({}) is downloaded; the downloaded helpers take {} together.",
                model.name,
                model.licence,
                ::assist::local::size_of(taken),
            ),
            false => format!(
                "{} ({}) — {} to download, about {} of memory while it runs.",
                model.name,
                model.licence,
                ::assist::local::size_of(model.bytes()),
                ::assist::local::size_of(model.memory),
            ),
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(words)
                    .size(theme::TEXT_SMALL)
                    .color(theme::INK_SOFT),
            )
            .wrap(),
        );
        let mut closed = None;
        // Remove takes every downloaded helper — a withdrawn one a person no
        // longer sees offered included — so it is there whenever anything is.
        ui.horizontal(|ui| {
            if !have
                && ui
                    .button(format!(
                        "Download it ({})",
                        ::assist::local::size_of(model.bytes())
                    ))
                    .clicked()
            {
                closed = Some(Closed::Download);
            }
            if taken > 0
                && ui
                    .button(format!(
                        "Remove the downloaded helpers (frees {})",
                        ::assist::local::size_of(taken)
                    ))
                    .clicked()
            {
                closed = Some(Closed::Remove);
            }
        });
        closed
    }

    fn claude(&mut self, ui: &mut egui::Ui, spent: Usage, cents: Option<f64>) {
        dialog::section(ui, "Claude");
        let claude = &mut self.draft.claude;
        // A sign-in the computer does not have is not offered, unless it is
        // the one already chosen.
        let logins: Vec<(ClaudeLogin, &str)> = [
            (ClaudeLogin::Key, "A key I paste"),
            (ClaudeLogin::Environment, "The key already on this computer"),
            (ClaudeLogin::Ant, "The login of the ant command"),
        ]
        .into_iter()
        .filter(|(login, _)| {
            *login == ClaudeLogin::Key
                || *login == claude.login
                || self.found.contains(&Row::ClaudeHere(*login))
        })
        .collect();
        if logins.len() > 1 {
            dialog::labelled(ui, "Sign in with:", |ui| {
                let current = logins
                    .iter()
                    .find(|(login, _)| *login == claude.login)
                    .map_or("", |(_, words)| words);
                egui::ComboBox::from_id_salt("ui-kit-assist-login")
                    .selected_text(current)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for (login, words) in &logins {
                            ui.selectable_value(&mut claude.login, *login, *words);
                        }
                    });
            });
        }
        if claude.login == ClaudeLogin::Key {
            dialog::labelled(ui, "Key:", |ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut claude.key)
                        .id(key_id())
                        .password(true)
                        .hint_text("sk-ant-…")
                        .desired_width(260.0),
                );
                if self.focus_key {
                    field.request_focus();
                    self.focus_key = false;
                }
            });
            note(
                ui,
                "From the Anthropic Console, and paid for as it is used. A Claude.ai \
                 subscription is not a key.",
            );
        }
        dialog::labelled(ui, "Model:", |ui| {
            ui.vertical(|ui| {
                for model in CLAUDE_MODELS {
                    let words = format!(
                        "{} — {} a paragraph",
                        model.words,
                        cost_words(model.paragraph_cents())
                    );
                    ui.radio_value(&mut claude.model, model.id.to_owned(), words);
                }
                // A model named in the file that the box does not offer is
                // still the one chosen, and says so.
                if claude_model(&claude.model).is_none() {
                    let _ = ui.radio(true, claude.model.as_str());
                }
            });
        });
        dialog::labelled(ui, "", |ui| {
            ui.checkbox(
                &mut claude.fallback,
                "If Opus 5 declines, let Anthropic's recommended model answer",
            );
        });
        // The box has no field for Claude's address, so it says where that
        // is, as the card does.
        let place = ::assist::destination(&self.draft).unwrap_or_else(|| "Anthropic".into());
        note(
            ui,
            &format!(
                "What you select, and a little of the document around it, is sent to {place}."
            ),
        );
        spent_line(ui, spent, cents);
    }

    fn ollama(&mut self, ui: &mut egui::Ui, spent: Usage, cents: Option<f64>) {
        dialog::section(ui, "Ollama");
        let ollama = &mut self.draft.ollama;
        dialog::labelled(ui, "Address:", |ui| {
            dialog::field(ui, &mut ollama.address, 260.0);
        });
        dialog::labelled(ui, "Model:", |ui| {
            dialog::field(ui, &mut ollama.model, 260.0);
        });
        if self.looking.is_some() {
            note(ui, "Looking for Ollama's models…");
        }
        let found = self.found.iter().find_map(|row| match row {
            Row::OllamaHere { models } => Some(models),
            _ => None,
        });
        for model in found.into_iter().flatten() {
            dialog::labelled(ui, "", |ui| {
                let on = ollama.model == model.name;
                if ui
                    .selectable_label(
                        on,
                        egui::RichText::new(model_words(model)).size(theme::TEXT_SMALL),
                    )
                    .clicked()
                {
                    ollama.model = model.name.clone();
                }
            });
        }
        spent_line(ui, spent, cents);
    }

    fn service(&mut self, ui: &mut egui::Ui, spent: Usage, cents: Option<f64>) {
        dialog::section(ui, "Another service");
        let service = &mut self.draft.service;
        dialog::labelled(ui, "Address:", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut service.address)
                    .hint_text("https://example.com/v1")
                    .desired_width(260.0),
            );
        });
        dialog::labelled(ui, "Key:", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut service.key)
                    .password(true)
                    .desired_width(260.0),
            );
        });
        dialog::labelled(ui, "Model:", |ui| {
            dialog::field(ui, &mut service.model, 260.0);
        });
        note(
            ui,
            "Any service that answers the way OpenAI's chat completions do. What you \
             select, and a little of the document around it, is sent to it.",
        );
        spent_line(ui, spent, cents);
    }
}

/// The settings in `path` as they are now, or the defaults.
fn on_file(path: Result<&Path, &String>) -> Settings {
    match path {
        Ok(path) => Settings::read(path).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

/// Whether the settings ask a service something the file's did not: another
/// helper, another key, another address or another model.
fn needs_check(draft: &Settings, saved: &Settings) -> bool {
    if draft.helper != saved.helper {
        return true;
    }
    let (claude, was) = (&draft.claude, &saved.claude);
    match draft.helper {
        Some(Choice::Claude) => {
            claude.login != was.login
                || claude.key != was.key
                || claude.model != was.model
                || claude.address != was.address
        }
        Some(Choice::Ollama) => draft.ollama != saved.ollama,
        Some(Choice::Service) => draft.service != saved.service,
        _ => false,
    }
}

/// A line of small print under a row, lined up with the fields.
fn note(ui: &mut egui::Ui, words: &str) {
    dialog::labelled(ui, "", |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(words)
                    .size(theme::TEXT_SMALL)
                    .color(theme::INK_SOFT),
            )
            .wrap(),
        );
    });
}

fn spent_line(ui: &mut egui::Ui, spent: Usage, cents: Option<f64>) {
    dialog::labelled(ui, "This session:", |ui| {
        ui.add(
            egui::Label::new(egui::RichText::new(spent_words(spent, cents)).color(theme::INK_SOFT))
                .wrap(),
        );
    });
}

/// What the session has spent, in words, and in cents once Claude was
/// asked: the counts are the services', read as three quarters of a word
/// each, and the cents are what each request cost at the model it went to.
pub(crate) fn spent_words(spent: Usage, cents: Option<f64>) -> String {
    let read = spent.input + spent.cache_read + spent.cache_write;
    let wrote = spent.output;
    if read + wrote == 0 {
        return "Nothing has been asked yet.".to_owned();
    }
    let mut words = format!(
        "The assistant read about {} words and wrote about {}",
        about(read * 3 / 4),
        about(wrote * 3 / 4)
    );
    if let Some(cents) = cents {
        words.push_str(&format!(", for {}", cost_words(cents)));
    }
    words.push('.');
    words
}

/// A count as "about" deserves it: two figures, with thousands marked.
fn about(count: u64) -> String {
    let mut rounded = count;
    let mut scale = 1;
    while rounded >= 100 {
        rounded = (rounded + 5) / 10;
        scale *= 10;
    }
    super::thousands(rounded * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_said_to_two_figures_with_its_thousands_marked() {
        assert_eq!(about(0), "0");
        assert_eq!(about(42), "42");
        assert_eq!(about(375), "380");
        assert_eq!(about(1537), "1,500");
        assert_eq!(about(1550), "1,600");
        assert_eq!(about(996), "1,000");
        assert_eq!(about(1_234_567), "1,200,000");
    }
}
