//! The pane, driven in an application of the tests' own.
//!
//! The application's document counts the letters it is given, and the pane
//! reaches out through a [`Fake`] that hands out canned helpers, a canned look
//! and a canned check. Nothing here reaches the network or the computer: the
//! driver makes the process headless, and the one test that uses the real
//! reach proves it.

mod card;
mod changes;
mod keeping;
mod requests;
mod window;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use ::assist::{
    Answer, Block, Choice, Ending, Failure, Message, Provider, Request, Row, Scripted, Settings,
    StopFlag, ToolCall, Usage,
};
use eframe::egui;
use serde_json::{json, Value};

use super::*;
use crate::assist::choosing::QUESTION;
use crate::drive::{Driver, Painted};
use crate::shell::DocumentApp;

/// How long a test waits for a request's thread before calling it a hang.
const PATIENCE: Duration = Duration::from_secs(20);

pub(super) const SYSTEM: &str = "You help a person edit the document they have open.";

pub(super) fn setup() -> Setup {
    Setup {
        system: SYSTEM.to_owned(),
        tools: vec![Tool::new(
            "read_paragraphs",
            "Read paragraphs first to last of the document.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["first", "last"],
                   "properties": {"first": {"type": "integer"}, "last": {"type": "integer"}}}),
        )],
    }
}

/// A directory of the test's own, removed when the test is over.
pub(super) struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "officina-assist-pane-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    /// Where the pane keeps its settings.
    pub fn settings(&self) -> PathBuf {
        self.0.join(::assist::settings::FILE)
    }

    /// The settings file, holding `settings`.
    pub fn holding(&self, settings: &Settings) -> PathBuf {
        let path = self.settings();
        settings.save(&path).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Ollama on this computer, which sends nothing away and so never asks.
pub(super) fn ollama_here() -> Settings {
    let mut settings = Settings {
        helper: Some(Choice::Ollama),
        ..Settings::default()
    };
    settings.ollama.model = "qwen3:1.7b".into();
    settings
}

/// Claude with a key, which is over the internet.
pub(super) fn claude_with_key() -> Settings {
    let mut settings = Settings {
        helper: Some(Choice::Claude),
        ..Settings::default()
    };
    settings.claude.key = "sk-ant-saved-key".into();
    settings
}

/// The reach a test gives the pane.
#[derive(Default)]
pub(super) struct Fake {
    helpers: Mutex<VecDeque<Box<dyn Provider>>>,
    /// Hand out the real helpers the settings name, which refuse under a test.
    pub real_helpers: bool,
    rows: Mutex<Vec<Row>>,
    /// The look waits for this before it answers.
    look_gate: Mutex<Option<mpsc::Receiver<()>>>,
    checks: Mutex<VecDeque<Result<String, Failure>>>,
    /// The next check waits for this before it answers.
    pub check_gate: Mutex<Option<mpsc::Receiver<()>>>,
    /// Every set of settings a helper was made for, in order.
    pub connected: Mutex<Vec<Settings>>,
    /// Every set of settings checked, in order.
    pub checked: Mutex<Vec<Settings>>,
    /// How the download goes: each step's progress, and what it ends with.
    /// The default is a download that never answers, which is what a window
    /// must not wait for.
    pub download_steps: Mutex<Vec<::assist::local::Progress>>,
    pub download_ends: Mutex<Option<Result<(), Failure>>>,
    /// Set once the download's thread has started, so a test can tell it
    /// apart from a download that never began.
    pub downloading: Arc<std::sync::atomic::AtomicUsize>,
    /// What `downloaded` says: `None` for "not downloaded".
    pub have_download: Mutex<Option<u64>>,
    /// Every removal asked for.
    pub removed: Arc<std::sync::atomic::AtomicUsize>,
}

impl Fake {
    pub fn new() -> Arc<Fake> {
        Arc::new(Fake::default())
    }

    /// Helpers to hand out, one per session, in order.
    pub fn helpers(self: &Arc<Self>, helpers: Vec<Box<dyn Provider>>) -> Arc<Self> {
        self.helpers.lock().unwrap().extend(helpers);
        Arc::clone(self)
    }

    /// What the computer is found to have.
    pub fn finds(self: &Arc<Self>, rows: Vec<Row>) -> Arc<Self> {
        *self.rows.lock().unwrap() = rows;
        Arc::clone(self)
    }

    /// The look waits until the sender is used or dropped.
    pub fn look_waits(self: &Arc<Self>) -> mpsc::Sender<()> {
        let (go, gate) = mpsc::channel();
        *self.look_gate.lock().unwrap() = Some(gate);
        go
    }

    /// What the next checks answer.
    pub fn checks(self: &Arc<Self>, answers: Vec<Result<String, Failure>>) -> Arc<Self> {
        self.checks.lock().unwrap().extend(answers);
        Arc::clone(self)
    }

    pub fn connections(&self) -> usize {
        self.connected.lock().unwrap().len()
    }
}

impl Reach for Fake {
    fn look(&self) -> Vec<Row> {
        let gate = self.look_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.recv_timeout(PATIENCE);
        }
        self.rows.lock().unwrap().clone()
    }

    fn connect(&self, settings: &Settings) -> Box<dyn Provider> {
        self.connected.lock().unwrap().push(settings.clone());
        if self.real_helpers {
            return ::assist::connect(settings);
        }
        self.helpers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Box::new(Scripted::new([])))
    }

    fn check(&self, settings: &Settings) -> Result<String, Failure> {
        self.checked.lock().unwrap().push(settings.clone());
        let gate = self.check_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.recv_timeout(PATIENCE);
        }
        self.checks
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok("It answered.".to_owned()))
    }

    fn download(
        &self,
        stop: &::assist::StopFlag,
        progress: &mut dyn FnMut(::assist::local::Progress),
    ) -> Result<(), Failure> {
        self.downloading
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let steps = self.download_steps.lock().unwrap().clone();
        for step in steps {
            if stop.is_set() {
                return Err(Failure::new(::assist::FailureKind::Dropped, "Stopped."));
            }
            progress(step);
        }
        // A download that says nothing and never ends: what the window must
        // not wait for. It reads Stop, as the real one does between chunks.
        let ends = self.download_ends.lock().unwrap().take();
        match ends {
            Some(ends) => ends,
            None => {
                while !stop.is_set() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(Failure::new(::assist::FailureKind::Dropped, "Stopped."))
            }
        }
    }

    fn downloaded(&self) -> Option<u64> {
        *self.have_download.lock().unwrap()
    }

    fn remove_download(&self) -> Result<u64, String> {
        self.removed
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let had = self.have_download.lock().unwrap().take();
        Ok(had.unwrap_or(0))
    }
}

/// One step of a [`Paced`] helper's answer.
pub(super) enum Pace {
    /// Words, unless Stop has been pressed: a helper that has heard Stop says
    /// nothing more, though what it had counted is still reported.
    Say(&'static str),
    /// Nothing, until the test lets it go: a helper waiting for its first
    /// word, which reads no Stop meanwhile.
    Wait(mpsc::Receiver<()>),
    /// The answer ends here, having cost this.
    End(Ending, Usage),
}

/// A helper whose answer arrives when the test says: the one way to see words
/// on the screen while a request is still under way.
pub(super) struct Paced {
    answers: VecDeque<Vec<Pace>>,
    /// The conversation each exchange was given.
    pub heard: Arc<Mutex<Vec<Vec<Message>>>>,
}

impl Paced {
    pub fn new(answers: Vec<Vec<Pace>>) -> Paced {
        Paced {
            answers: answers.into(),
            heard: Arc::default(),
        }
    }
}

impl Provider for Paced {
    fn name(&self) -> &str {
        "The paced helper"
    }

    fn answer(&mut self, request: &Request, stop: &StopFlag, text: &mut dyn FnMut(&str)) -> Answer {
        self.heard
            .lock()
            .unwrap()
            .push(request.conversation.messages().to_vec());
        let mut said = String::new();
        let mut ending = Ending::Finished;
        let mut usage = Usage::default();
        for pace in self.answers.pop_front().unwrap_or_default() {
            match pace {
                Pace::Say(words) => {
                    if stop.is_set() {
                        continue;
                    }
                    text(words);
                    said.push_str(words);
                }
                Pace::Wait(gate) => {
                    let _ = gate.recv_timeout(PATIENCE);
                }
                Pace::End(end, cost) => {
                    ending = end;
                    usage = cost;
                    break;
                }
            }
        }
        if stop.is_set() {
            ending = Ending::Stopped;
        }
        let content = match said.is_empty() {
            true => Vec::new(),
            false => vec![Block::Text(said)],
        };
        Answer {
            message: Message::assistant(content),
            ending: Ok(ending),
            usage,
        }
    }
}

/// How the desk answers a tool call: with what came of it, or not yet.
pub(super) type Answers = Box<dyn FnMut(&ToolCall) -> Option<Ran>>;

/// The tests' application: a pane on the right, and a document that counts
/// the letters it is given.
pub(super) struct Desk {
    pub assist: Assist,
    pub frame: usize,
    /// Each call run, the frame it ran on, and the thread it ran on.
    pub ran: Vec<(usize, ToolCall, std::thread::ThreadId)>,
    pub answers: Answers,
    /// A call the desk has not answered yet.
    pub held: Option<Call>,
    pub asked: Vec<Asked>,
    pub chosen: Vec<Chosen>,
    /// What the document was given while it had the keyboard.
    pub typed: String,
    pub scopes: Vec<(&'static str, Option<usize>)>,
    pub following: usize,
    pub verbs: Vec<(&'static str, &'static str)>,
    pub menu: Vec<&'static str>,
    pub leaves: String,
    /// Whether the pane is drawn.
    pub open: bool,
    /// Where the pane was drawn.
    pub pane: egui::Rect,
}

fn document_id() -> egui::Id {
    egui::Id::new("desk-document")
}

impl Desk {
    pub fn new(assist: Assist) -> Desk {
        Desk {
            assist,
            frame: 0,
            ran: Vec::new(),
            answers: Box::new(|call: &ToolCall| {
                Some(Ran::new(ToolResult::ok(call, "paragraph text")).said("read a paragraph"))
            }),
            held: None,
            asked: Vec::new(),
            chosen: Vec::new(),
            typed: String::new(),
            scopes: vec![("Selection", Some(42)), ("Paragraph", None)],
            following: 1,
            verbs: Vec::new(),
            menu: Vec::new(),
            leaves: "The selected paragraph and the two around it".to_owned(),
            open: true,
            pane: egui::Rect::NOTHING,
        }
    }

    /// The pane, with its settings in `path` and `reach` behind it.
    pub fn with(reach: Arc<Fake>, path: PathBuf) -> Desk {
        Desk::new(Assist::with(setup(), reach, path))
    }

    /// Answers the call the desk held, now.
    pub fn release(&mut self, ran: Ran) {
        let call = self.held.take().expect("a call held");
        self.assist.answer(&call, ran);
    }

    /// Types `words` into the composer, which is given the keyboard first,
    /// and presses Enter.
    pub fn ask(&mut self, drive: &Driver, words: &str) {
        self.assist.focus();
        drive.settle(self);
        drive.settle(self);
        drive.type_text(self, words);
        drive.press(self, "Enter");
    }

    /// Frames, a moment apart, until `done` says so.
    pub fn until(&mut self, drive: &Driver, what: &str, done: impl Fn(&Desk) -> bool) {
        let started = Instant::now();
        while !done(self) {
            assert!(
                started.elapsed() < PATIENCE,
                "waited too long for {what}; the transcript is {:#?}",
                self.assist.transcript()
            );
            drive.settle(self);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Frames until the request under way has ended.
    pub fn finished(&mut self, drive: &Driver) {
        self.until(drive, "the request to end", |desk| {
            !desk.assist.is_working()
        });
        drive.settle(self);
    }

    /// What a frame painted inside the pane.
    pub fn painted(&mut self, drive: &Driver) -> Painted {
        drive.settle(self);
        drive.paint(self, Vec::new())
    }

    /// The strings a frame painted inside the pane, where they can be seen.
    pub fn seen(&mut self, drive: &Driver) -> Vec<String> {
        let painted = self.painted(drive);
        let pane = self.pane;
        painted
            .texts()
            .into_iter()
            .filter(|text| text.shown().area() > 0.0 && pane.contains_rect(text.shown()))
            .map(|text| text.text)
            .collect()
    }

    /// Every string a frame painted where it can be seen: the pane, and a box
    /// over the window.
    pub fn everywhere(&mut self, drive: &Driver) -> Vec<String> {
        self.painted(drive)
            .texts()
            .into_iter()
            .filter(|text| text.shown().area() > 0.0)
            .map(|text| text.text)
            .collect()
    }

    /// Clicks the painted text `words` in the pane.
    pub fn click(&mut self, drive: &Driver, words: &str) {
        let at = self.find(drive, words);
        drive.click(self, at);
    }

    /// Where the painted text `words` is, in the pane or a box over it.
    pub fn find(&mut self, drive: &Driver, words: &str) -> egui::Pos2 {
        let painted = self.painted(drive);
        let text = painted.text(words).unwrap_or_else(|| {
            panic!(
                "“{words}” is not on the screen; it shows {:?}",
                painted.strings()
            )
        });
        text.shown().center()
    }
}

impl DocumentApp for Desk {
    fn id(&self) -> crate::AppId {
        crate::SCRIVA
    }

    fn overlay(&mut self, ctx: &egui::Context) {
        self.assist.overlay(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.frame += 1;
        if let Some(call) = self.assist.poll(ui.ctx()) {
            self.ran
                .push((self.frame, call.tool.clone(), std::thread::current().id()));
            match (self.answers)(&call.tool) {
                Some(ran) => self.assist.answer(&call, ran),
                None => self.held = Some(call),
            }
        }
        if self.open {
            let scopes: Vec<Scope> = self
                .scopes
                .iter()
                .map(|(name, words)| Scope {
                    name,
                    words: *words,
                })
                .collect();
            let verbs: Vec<Verb> = self
                .verbs
                .iter()
                .map(|(label, asks)| Verb { label, asks })
                .collect();
            let offer = Offer {
                scopes: &scopes,
                following: self.following,
                verbs: &verbs,
                menu: &self.menu,
                tabs: &[],
                tab: 0,
            };
            let assist = &mut self.assist;
            let shown = egui::Panel::right("desk-assist")
                .exact_size(WIDTH)
                .resizable(false)
                .frame(egui::Frame::new().fill(theme::CHROME))
                .show(ui, |ui| assist.show(ui, &offer));
            self.pane = shown.response.rect;
            match shown.inner {
                Some(Chosen::Ask(asked)) => {
                    let sent = format!("Context: paragraph 1.\n\nRequest: {}", asked.words);
                    self.assist
                        .send(Prepared::new(&asked, sent, self.leaves.clone()));
                    self.asked.push(asked);
                }
                Some(Chosen::Leave) => {
                    ui.ctx().memory_mut(|m| m.request_focus(document_id()));
                    self.chosen.push(Chosen::Leave);
                }
                Some(other) => self.chosen.push(other),
                None => {}
            }
        }
        let rect = ui.available_rect_before_wrap();
        let document = ui.interact(rect, document_id(), egui::Sense::click());
        if document.has_focus() {
            let typed: String = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            self.typed.push_str(&typed);
        }
    }
}

/// The last user message a helper was given, in words.
pub(super) fn last_words(conversation: &[Message]) -> String {
    conversation
        .iter()
        .rev()
        .find(|message| message.role == ::assist::Role::User && !message.text().is_empty())
        .map(Message::text)
        .unwrap_or_default()
}

/// The input of a call, read as a paragraph number.
pub(super) fn first_of(call: &ToolCall) -> i64 {
    call.input["first"].as_i64().unwrap_or(-1)
}

pub(super) fn reading(first: i64) -> Value {
    json!({"first": first, "last": first})
}
