//! Assist's pane: where a person asks for a change in their own words, sees
//! the answer arrive, sees what the helper read and changed, and stops it —
//! and where the helper is chosen, once, in words.
//!
//! **One pane for both applications, knowing neither document.** Each
//! application gives it a [`Setup`] — what the helper is told, and the tools
//! it may ask for — and, every frame it is drawn, an [`Offer`]: what a request
//! can be about, the quick verbs, and rows of its own for the pane's menu. The
//! pane hands back what the person chose ([`Chosen`]), and a request is sent
//! only once the application has put the document's part of it into words
//! ([`Prepared`]).
//!
//! **The document never leaves the window's thread.** A request runs on a
//! thread of its own. When the helper wants a tool, [`Assist::poll`] hands the
//! call to the application — never more than one a frame — and the
//! application runs it against its own document, through its own editing
//! functions, and gives back what came of it with [`Assist::answer`]. `poll`
//! is called on every frame, whether the pane is drawn or not: a request goes
//! on while the pane is closed.
//!
//! **The window never waits on a helper.** Stop is immediate on the screen. A
//! helper waiting for its first word cannot hear Stop until the word comes,
//! which with Ollama loading a model can be minutes, so the pane lets go of
//! it, counts what it cost once it ends, and gives the next request a helper
//! of its own, from the conversation as it stood before.
//!
//! **Nothing leaves the computer unannounced.** The first request to a helper
//! elsewhere says, in the pane, what will be sent and where, and waits for
//! Send. The agreement holds for as long as the words would go to the same
//! place.
//!
//! **Under a test, nothing is reached.** The helpers refuse on the thread that
//! asked them (`assist::offline`), and each refusal is counted again when it
//! reaches the window's thread, where [`crate::headless::helpers_refused`]
//! reads it.

mod choosing;
mod request;
mod settings_box;
mod transcript;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use ::assist::models::{claude_model, ClaudeModel};
use ::assist::{
    Choice, Conversation, Effort, Ending, Event, Failure, FailureKind, Provider, Row, Session,
    Settings, Tool, ToolCall, ToolResult, Usage,
};
use eframe::egui;

use choosing::{Choosing, Chose};
use request::{count_again, counting, Background, Drain, Message};
use settings_box::{Closed, SettingsBox};
use transcript::Pressed;
pub use transcript::{Action, Card, Entry, NOT_KEPT};

use crate::{dialog, menu, theme};

/// How wide the pane opens, in points: room for a card's two buttons and a
/// sentence of a dozen words to a line.
pub const WIDTH: f32 = 340.0;

/// How often a window with a request under way looks for what arrived, when
/// nothing else wakes it.
const LOOK_AGAIN: Duration = Duration::from_millis(100);

/// What the application tells a session's helper, the same for every request.
#[derive(Debug, Clone)]
pub struct Setup {
    /// What the helper is, and how it works: the editor's instructions.
    /// Unchanging, so that a service reads it back from its cache.
    pub system: String,
    /// The tools the helper may ask the application to run.
    pub tools: Vec<Tool>,
}

/// How the pane reaches beyond the window: the computer, looked at for a
/// helper; the helper the settings name; and the check a key is put to. The
/// applications use [`Real`]; a test gives its own.
pub trait Reach: Send + Sync {
    /// The first-run card's rows. Run on a thread of its own.
    fn look(&self) -> Vec<Row> {
        ::assist::ladder(&::assist::ThisComputer)
    }

    /// The helper `settings` name, ready to ask.
    fn connect(&self, settings: &Settings) -> Box<dyn Provider> {
        ::assist::connect_in(settings, downloads().as_deref())
    }

    /// Whether that helper is there and takes the key. Run on a thread of
    /// its own.
    fn check(&self, settings: &Settings) -> Result<String, Failure> {
        ::assist::check_in(settings, downloads().as_deref())
    }

    /// Downloads the helper on this computer, reporting how far along it is
    /// and reading `stop` as it goes. Run on a thread of its own, which is
    /// why the window never waits for it.
    fn download(
        &self,
        stop: &::assist::StopFlag,
        progress: &mut dyn FnMut(::assist::local::Progress),
    ) -> Result<(), Failure> {
        let Some(cache) = downloads() else {
            return Err(Failure::new(
                ::assist::FailureKind::NotReady,
                "There is nowhere to keep the helper: Officina's cache directory could not \
                 be made.",
            ));
        };
        ::assist::local::download_model(&cache, stop, progress)
    }

    /// Whether the helper on this computer is downloaded already, and how
    /// much space its files take.
    fn downloaded(&self) -> Option<u64> {
        let cache = downloads()?;
        ::assist::local::have(&cache).then(|| ::assist::local::MODEL.bytes())
    }

    /// Removes the downloaded helper, and says how much space came back.
    fn remove_download(&self) -> Result<u64, String> {
        let Some(cache) = downloads() else {
            return Ok(0);
        };
        // What is in memory goes with what is on disk: a model kept for the
        // next request is a gigabyte held for a helper that is no longer
        // there.
        ::assist::local::forget();
        ::assist::local::remove(&cache).map_err(|why| why.to_string())
    }
}

/// Where a downloaded helper's weights live: the suite's cache directory,
/// which under a test is a directory of the test's own and never the
/// person's. `None` when it cannot be made, which reads as "not downloaded".
pub fn downloads() -> Option<std::path::PathBuf> {
    crate::paths::cache_dir(crate::OFFICINA).ok()
}

/// The computer, the helpers and the checks as they are.
pub struct Real;

impl Reach for Real {}

/// A thing a request can be about.
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    /// "Selection", "Paragraph", "Whole document".
    pub name: &'a str,
    /// How many words it is, when that is worth saying before they are sent.
    pub words: Option<usize>,
}

/// A quick verb: a request with a name on a chip.
#[derive(Debug, Clone, Copy)]
pub struct Verb<'a> {
    pub label: &'a str,
    /// What the request says. A verb whose label ends in "…" needs words of
    /// the person's own, and puts these in the composer to be finished.
    pub asks: &'a str,
}

/// What the application offers the pane on a frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Offer<'a> {
    pub scopes: &'a [Scope<'a>],
    /// Which scope the selection makes it now.
    pub following: usize,
    pub verbs: &'a [Verb<'a>],
    /// The application's own rows in the pane's menu, `&` marking the letter.
    pub menu: &'a [&'a str],
    /// The panes that share the pane's place, named in its header, and which
    /// of them this is. None: the header says "Assist".
    pub tabs: &'a [&'a str],
    pub tab: usize,
}

/// A request the person made, before the application has put it into words.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub words: String,
    /// Which of the offer's scopes it is about.
    pub scope: usize,
    pub effort: Effort,
}

/// What was chosen on the pane, for the application to act on.
#[derive(Debug, Clone, PartialEq)]
pub enum Chosen {
    /// A request, to be put into words and handed back with [`Assist::send`]
    /// — typed, a quick verb, or a failed request tried again, whose
    /// document part is put into words afresh.
    Ask(Asked),
    /// An action on a card: which card, and which of its actions.
    Card { card: u64, action: usize },
    /// One of the application's own rows in the pane's menu.
    Menu(usize),
    /// Another tab of the header.
    Tab(usize),
    /// Escape: the keyboard goes back to the document.
    Leave,
    /// The pane's close button, or "Not now" on the first-run card.
    Close,
    /// Stop on the download's bar.
    StopDownload,
}

/// A request in words: what the transcript shows, what the helper is sent,
/// and what leaves the computer if the helper is elsewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub shown: String,
    /// The document's part and the request, as the helper reads them.
    pub sent: String,
    pub effort: Effort,
    /// Which scope it is about, for trying it again.
    pub scope: usize,
    /// What is sent, in a phrase: "The selected paragraph and the two around
    /// it".
    pub leaves: String,
}

impl Prepared {
    pub fn new(asked: &Asked, sent: impl Into<String>, leaves: impl Into<String>) -> Prepared {
        Prepared {
            shown: asked.words.clone(),
            sent: sent.into(),
            effort: asked.effort,
            scope: asked.scope,
            leaves: leaves.into(),
        }
    }
}

/// A tool the helper wants run, as the pane hands it over.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub tool: ToolCall,
    /// Which handing-over this is, of all the pane has made. Only the call
    /// out now is answered: one from a request stopped since, or one already
    /// answered, goes to nobody, whatever the calls are called.
    number: u64,
}

/// What came of a tool the application ran.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    /// What goes back to the helper.
    pub result: ToolResult,
    /// What the transcript says it did, in a line: "read paragraphs 12–14".
    pub line: Option<String>,
    /// The change it made, for the person to settle.
    pub card: Option<Card>,
}

impl Ran {
    pub fn new(result: ToolResult) -> Ran {
        Ran {
            result,
            line: None,
            card: None,
        }
    }

    pub fn said(mut self, line: impl Into<String>) -> Ran {
        self.line = Some(line.into());
        self
    }

    pub fn card(mut self, card: Card) -> Ran {
        self.card = Some(card);
        self
    }
}

/// A download's progress, for its bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    /// `None` until the server has said.
    pub total: Option<u64>,
}

/// A download under way: the thread doing it, what it has reported so far,
/// and the flag that stops it.
struct Downloading {
    work: request::Background<Result<(), Failure>>,
    said: mpsc::Receiver<::assist::local::Progress>,
    stop: ::assist::StopFlag,
}

/// A request under way.
struct Running {
    request: request::Request,
    /// The call the application has, and where its result goes.
    out: Option<(u64, mpsc::Sender<ToolResult>)>,
    /// The transcript's length when the request began.
    first: usize,
    /// The conversation as the request found it.
    before: Conversation,
    /// The entry the helper's words are going into.
    said: Option<usize>,
    /// The helper, as a failure's sentence names it.
    helper: String,
    /// The Claude model it was sent to, which is what its cost is priced at.
    priced: Option<ClaudeModel>,
    prepared: Prepared,
}

/// The pane.
pub struct Assist {
    setup: Setup,
    reach: Arc<dyn Reach>,
    /// Where the settings are kept, or why there is nowhere.
    path: Result<PathBuf, String>,
    /// What the file holds; `Err` with the reason when it cannot be read.
    settings: Result<Settings, String>,
    /// Where a request's words go under those settings, when not here.
    place: Option<String>,
    choosing: Option<Choosing>,
    /// The last look's rows, for the settings box.
    found: Vec<Row>,
    session: Option<Session>,
    /// Where the next session begins, when a stopped request took the last
    /// one with it.
    resume: Option<Conversation>,
    /// The settings changed in a way the helper must hear, though it is the
    /// same helper: the next request asks a new one, carrying on.
    renew: bool,
    running: Option<Running>,
    /// How many calls the pane has handed over.
    calls: u64,
    /// Requests stopped and let go, heard only for what they cost.
    let_go: Vec<(mpsc::Receiver<Message>, Option<ClaudeModel>)>,
    /// Looks and checks nobody waits for any more, heard only for the
    /// refusals their threads counted.
    draining: Vec<Box<dyn Drain>>,
    transcript: Vec<Entry>,
    /// The request each Try Again in the transcript tries again, by entry.
    retries: Vec<(usize, Prepared)>,
    composer: String,
    /// Whether the composer had the keyboard at the end of the last frame.
    composer_had_keyboard: bool,
    /// The keyboard goes to the pane on the next frame.
    focus: bool,
    /// The scope the person chose, over the one the selection makes it.
    scope: Option<usize>,
    following: usize,
    /// Whether the person has agreed to their words going to `place`.
    consented: bool,
    /// A request waiting for that agreement, and where it would go.
    consent: Option<(Prepared, String)>,
    settings_box: Option<SettingsBox>,
    spent: Usage,
    /// What Claude's part of that cost, in cents, once Claude was asked.
    cents: Option<f64>,
    download: Option<Progress>,
    /// The download under way: the thread, and the stop that ends it.
    downloading: Option<Downloading>,
    /// How many requests in a row the helper on this computer has failed,
    /// and whether the sentence about a bigger one has been said already.
    failed_in_a_row: usize,
    said_what_it_is_not_good_at: bool,
    /// The frame the pane was last drawn on, to tell a pane just opened.
    drawn: Option<u64>,
    /// Whether a menu was open when the pane was last drawn. egui closes a
    /// menu on Escape and leaves the key for whoever reads it next: without
    /// this, the Escape that closed a menu also gave the keyboard away, and
    /// the Enter that chose a row sent the composer's words.
    popup_before: bool,
    ctx: Option<egui::Context>,
}

fn composer_id() -> egui::Id {
    egui::Id::new("ui-kit-assist-composer")
}

fn more_id() -> egui::Id {
    egui::Id::new("ui-kit-assist-more")
}

/// Where the last frame drew the pane's `⋯`, which has no words to be found
/// by: what a test — the pane's own, or an application's — presses instead
/// of a person.
pub fn more_button(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp(more_id()))
}

/// The helper in the header's words: its name, and where it is when that is
/// here.
fn named(settings: &Settings, place: &Option<String>) -> String {
    let mut name = settings.helper_name();
    if place.is_none() && settings.helper != Some(Choice::Local) {
        name.push_str(" · on this computer");
    }
    name
}

impl Assist {
    /// The pane both applications open, reading the suite's settings.
    pub fn new(setup: Setup) -> Assist {
        let path = crate::paths::assist_settings().map_err(|error| error.to_string());
        Assist::at(setup, Arc::new(Real), path)
    }

    /// The pane, reaching out through `reach`, with its settings in `path`.
    pub fn with(setup: Setup, reach: Arc<dyn Reach>, path: PathBuf) -> Assist {
        Assist::at(setup, reach, Ok(path))
    }

    fn at(setup: Setup, reach: Arc<dyn Reach>, path: Result<PathBuf, String>) -> Assist {
        let settings = match &path {
            Ok(path) => Settings::read(path),
            Err(why) => Err(format!("There is nowhere to keep them: {why}")),
        };
        let place = settings.as_ref().ok().and_then(::assist::destination);
        Assist {
            setup,
            reach,
            path,
            settings,
            place,
            choosing: None,
            found: Vec::new(),
            session: None,
            resume: None,
            renew: false,
            running: None,
            calls: 0,
            let_go: Vec::new(),
            draining: Vec::new(),
            transcript: Vec::new(),
            retries: Vec::new(),
            composer: String::new(),
            composer_had_keyboard: false,
            focus: false,
            scope: None,
            following: 0,
            consented: false,
            consent: None,
            settings_box: None,
            spent: Usage::default(),
            cents: None,
            download: None,
            downloading: None,
            failed_in_a_row: 0,
            said_what_it_is_not_good_at: false,
            drawn: None,
            popup_before: false,
            ctx: None,
        }
    }

    // ---- what the application asks -------------------------------------

    pub fn transcript(&self) -> &[Entry] {
        &self.transcript
    }

    pub fn composer(&self) -> &str {
        &self.composer
    }

    /// Whether a request is under way.
    pub fn is_working(&self) -> bool {
        self.running.is_some()
    }

    /// Whether a request is under way or waiting to be agreed to: what says
    /// whether the words the application prepares now are the ones that go.
    pub fn is_busy(&self) -> bool {
        self.running.is_some() || self.consent.is_some()
    }

    /// Whether the settings box is up, holding the keyboard.
    pub fn box_up(&self) -> bool {
        self.settings_box.is_some()
    }

    /// Whether the pane has the keyboard: its composer, or its box.
    pub fn holds_keyboard(&self, ctx: &egui::Context) -> bool {
        self.box_up() || ctx.memory(|m| m.has_focus(composer_id()))
    }

    /// The settings in force, when they could be read.
    pub fn settings(&self) -> Option<&Settings> {
        self.settings.as_ref().ok()
    }

    /// What this window's requests have cost, in the services' counts.
    pub fn spent(&self) -> Usage {
        self.spent
    }

    /// What Claude's part of that cost, in US cents, once Claude was asked:
    /// each request priced at the model it went to.
    pub fn cents(&self) -> Option<f64> {
        self.cents
    }

    /// The scope a request made now would be about.
    pub fn scope(&self) -> usize {
        self.scope.unwrap_or(self.following)
    }

    /// Whether the request that asked for `call` still waits for it. An
    /// application that has not yet run a call may skip one nobody wants.
    pub fn wanted(&self, call: &Call) -> bool {
        self.running
            .as_ref()
            .and_then(|running| running.out.as_ref())
            .is_some_and(|(number, _)| *number == call.number)
    }

    // ---- what the application tells it ---------------------------------

    /// Gives the pane the keyboard on its next frame: the composer, or the
    /// first-run card's "Use this" while the card is up.
    pub fn focus(&mut self) {
        self.focus = true;
    }

    /// Gives the composer the keyboard at once, unless a card or the
    /// settings box is up: for an application whose keyboard is still the
    /// pane's when nothing holds the focus — a menu opened from the composer
    /// has closed, or a box over the window has gone.
    pub fn keep_keyboard(&self, ctx: &egui::Context) {
        if self.choosing.is_none() && !self.box_up() {
            ctx.memory_mut(|m| m.request_focus(composer_id()));
        }
    }

    /// Opens the settings box.
    pub fn open_settings(&mut self) {
        self.open_box(None, false);
    }

    /// Sets what became of a card's change, and takes its buttons away.
    pub fn settle(&mut self, card: u64, verdict: &str) {
        for entry in &mut self.transcript {
            if let Entry::Card(shown) = entry {
                if shown.id == card {
                    shown.verdict = Some(verdict.to_owned());
                }
            }
        }
    }

    /// The download under way, for its bar; `None` takes the bar away.
    pub fn set_download(&mut self, progress: Option<Progress>) {
        self.download = progress;
    }

    /// Starts downloading the helper on this computer, if it is not being
    /// downloaded already.
    ///
    /// **On a thread, always.** A gigabyte over a slow connection is an hour,
    /// and a window that waited for it would be a window nobody could close.
    /// What the thread reports arrives on a channel the pane reads as it
    /// draws; the flag it holds is what Stop sets.
    pub fn start_download(&mut self, ctx: &egui::Context) {
        if self.downloading.is_some() {
            return;
        }
        let reach = Arc::clone(&self.reach);
        let wake = ctx.clone();
        let stop = ::assist::StopFlag::default();
        let theirs = stop.clone();
        let (tell, said) = mpsc::channel();
        self.download = Some(Progress {
            done: 0,
            total: Some(::assist::local::MODEL.bytes()),
        });
        self.downloading = Some(Downloading {
            work: request::Background::spawn(
                move || {
                    reach.download(&theirs, &mut |progress| {
                        let _ = tell.send(progress);
                    })
                },
                move || wake.request_repaint(),
            ),
            said,
            stop,
        });
    }

    /// Whether a download is under way now.
    pub fn is_downloading(&self) -> bool {
        self.downloading.is_some()
    }

    /// How far the download has got, for a test to wait on and for a window
    /// to say elsewhere.
    pub fn download_so_far(&self) -> Option<Progress> {
        self.download
    }

    /// Stops the download, keeping what has arrived: starting it again
    /// carries on from there.
    pub fn stop_download(&mut self) {
        if let Some(downloading) = &self.downloading {
            downloading.stop.stop();
        }
    }

    /// A frame's worth of the download: what it has reported, and what became
    /// of it once it ends.
    fn tend_download(&mut self, ctx: &egui::Context) {
        let Some(downloading) = &mut self.downloading else {
            return;
        };
        while let Ok(progress) = downloading.said.try_recv() {
            self.download = Some(Progress {
                done: progress.done,
                total: Some(progress.total),
            });
        }
        match downloading.work.poll() {
            request::Awaited::Waiting => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            request::Awaited::Done(what) => {
                self.downloading = None;
                self.download = None;
                match what {
                    Ok(()) => {
                        let ready =
                            format!("{} is ready on this computer.", ::assist::local::MODEL.name);
                        self.note(&ready, None);
                        // The helper the settings name is made afresh, so
                        // that the one refusing for want of weights is let go.
                        self.renew = true;
                    }
                    Err(failure) => self.note(&failure.sentence, None),
                }
            }
            request::Awaited::Gone => {
                self.downloading = None;
                self.download = None;
                self.note("The download ended without saying why.", None);
            }
        }
    }

    /// Sends a request. The first to a helper elsewhere waits for the person
    /// to agree to what is sent. While another request is under way, or
    /// before a helper is chosen, nothing is sent, and the words wait in the
    /// composer — unless the person has begun other words there, which are
    /// theirs.
    pub fn send(&mut self, prepared: Prepared) {
        let chosen = self.settings.as_ref().is_ok_and(|s| s.helper.is_some());
        if self.running.is_some() || self.consent.is_some() || !chosen {
            if self.composer.trim().is_empty() {
                self.composer = prepared.shown;
            }
            return;
        }
        match self.place.clone() {
            Some(to) if !self.consented => self.consent = Some((prepared, to)),
            _ => self.start(prepared),
        }
    }

    /// Stops the request under way. The screen is ready at once, whatever the
    /// helper is doing: a tool it is waiting on is answered as stopped, and a
    /// helper that has not begun to answer is let go rather than waited for.
    pub fn stop(&mut self) {
        let Some(running) = self.running.take() else {
            return;
        };
        running.request.stop.stop();
        drop(running.out);
        self.let_go.push((running.request.from, running.priced));
        // The session went with the request. The next one begins where this
        // one did, with a helper of its own.
        self.session = None;
        self.resume = Some(running.before);
        self.forget_from(running.first);
        self.note("Stopped.", None);
        self.focus = true;
    }

    /// Forgets the conversation, and empties the transcript.
    pub fn clear(&mut self) {
        self.stop();
        self.transcript.clear();
        self.retries.clear();
        self.resume = None;
        // A new conversation is a new chance: what the helper failed at
        // before this document is not held against it, and the sentence
        // about a bigger helper can be said again if it earns it.
        self.failed_in_a_row = 0;
        self.said_what_it_is_not_good_at = false;
        if let Some(session) = &mut self.session {
            session.clear();
        }
        if let Some((prepared, _)) = self.consent.take() {
            self.keep_words(prepared.shown);
        }
    }

    /// Hands over the next tool the helper wants run, if there is one, and
    /// takes in everything else a request has said. Called on every frame,
    /// whether the pane is drawn or not.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Call> {
        self.ctx = Some(ctx.clone());
        self.hear_the_let_go();
        self.draining.retain_mut(|thread| match thread.drained() {
            Some(refused) => {
                count_again(refused);
                false
            }
            None => true,
        });
        let call = self.hear();
        if self.running.is_some() || !self.let_go.is_empty() || !self.draining.is_empty() {
            ctx.request_repaint_after(LOOK_AGAIN);
        }
        call
    }

    /// Puts a card in the transcript on its own, rather than after a tool.
    ///
    /// **A request can be one thing to settle rather than several.** Scriva's
    /// proposals are a card each, because each is accepted or rejected on its
    /// own; Calx's edits land at once and are one entry in the undo history,
    /// so the card that offers Undo belongs to the request, not to the tool
    /// call that happened to be last.
    pub fn add_card(&mut self, card: Card) {
        self.transcript.push(Entry::Card(card));
    }

    /// Gives the request that asked for `call` what came of it, and puts
    /// what the tool did in the transcript. What a tool did to the document
    /// is said even when its request was stopped meanwhile; the result goes
    /// to nobody then.
    pub fn answer(&mut self, call: &Call, ran: Ran) {
        if let Some(line) = ran.line {
            self.transcript.push(Entry::Did(line));
        }
        if let Some(card) = ran.card {
            self.transcript.push(Entry::Card(card));
        }
        let Some(running) = &mut self.running else {
            return;
        };
        if running.out.as_ref().map(|(number, _)| *number) != Some(call.number) {
            return;
        }
        running.said = None;
        if let Some((_, reply)) = running.out.take() {
            let _ = reply.send(ran.result);
        }
    }

    // ---- the request's life --------------------------------------------

    fn start(&mut self, prepared: Prepared) {
        let Ok(settings) = &self.settings else {
            // No helper to ask after all: the words wait in the composer.
            self.keep_words(prepared.shown);
            return;
        };
        if std::mem::take(&mut self.renew) {
            if let Some(session) = self.session.take() {
                self.resume = Some(session.conversation().clone());
            }
        }
        let session = match self.session.take() {
            Some(session) => session,
            None => {
                let session = Session::new(
                    self.reach.connect(settings),
                    self.setup.system.clone(),
                    self.setup.tools.clone(),
                );
                match self.resume.take() {
                    Some(conversation) => session.continuing(conversation),
                    None => session,
                }
            }
        };
        let priced = (settings.helper == Some(Choice::Claude))
            .then(|| claude_model(&settings.claude.model).copied())
            .flatten();
        let before = session.conversation().clone();
        let helper = session.helper().to_owned();
        let first = self.transcript.len();
        self.transcript.push(Entry::Asked(prepared.shown.clone()));
        let wake = self.ctx.clone();
        let request = request::start(session, prepared.sent.clone(), prepared.effort, move || {
            if let Some(ctx) = &wake {
                ctx.request_repaint();
            }
        });
        self.running = Some(Running {
            request,
            out: None,
            first,
            before,
            said: None,
            helper,
            priced,
            prepared,
        });
    }

    /// Takes in what the request under way has said, up to the next tool.
    fn hear(&mut self) -> Option<Call> {
        loop {
            let running = self.running.as_mut()?;
            // The application has a call and has not answered it yet.
            if running.out.is_some() {
                return None;
            }
            match running.request.from.try_recv() {
                Ok(Message::Event(Event::Text(words))) => match running.said {
                    Some(at) => {
                        if let Some(Entry::Said { words: said, .. }) = self.transcript.get_mut(at) {
                            said.push_str(&words);
                        }
                    }
                    None => {
                        running.said = Some(self.transcript.len());
                        self.transcript.push(Entry::Said { words, kept: true });
                    }
                },
                // Words after a tool, or after an exchange, are a new entry.
                Ok(Message::Event(_)) => running.said = None,
                Ok(Message::Run(tool, reply)) => {
                    self.calls += 1;
                    running.out = Some((self.calls, reply));
                    return Some(Call {
                        tool,
                        number: self.calls,
                    });
                }
                Ok(Message::Ended {
                    ended,
                    spent,
                    session,
                }) => {
                    self.ended(ended, spent, *session);
                    return None;
                }
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let running = self.running.take()?;
                    self.resume = Some(running.before.clone());
                    self.forget_from(running.first);
                    self.failed(&request::vanished(), &running.prepared);
                    return None;
                }
            }
        }
    }

    fn ended(&mut self, ended: Result<Ending, Failure>, spent: Usage, session: Session) {
        let Some(running) = self.running.take() else {
            return;
        };
        self.count(spent, running.priced);
        self.session = Some(session);
        let counted = self.count_failure(&ended);
        match ended {
            Ok(Ending::Finished) => {}
            Ok(ending) => {
                self.forget_from(running.first);
                if let Some(sentence) = ending.sentence(&running.helper) {
                    if matches!(ending, Ending::TooLong) {
                        self.retry_note(&sentence, running.prepared);
                    } else {
                        self.note(&sentence, None);
                    }
                }
            }
            Err(failure) => {
                self.forget_from(running.first);
                self.failed(&failure, &running.prepared);
            }
        }
        // After the failure, never before it: a verdict above the thing it is
        // about reads as a verdict on the request the person just made.
        if counted {
            self.say_what_it_is_not_good_at();
        }
    }

    /// **What the small helper is not good at is said, once.** The helper on
    /// this computer is a fifth the size of the ones over the internet, and a
    /// person watching it fail twice deserves to be told where a better one
    /// is rather than left to conclude the feature is broken. It comes from
    /// the application, never from the model, and it is said once a
    /// conversation: a sentence repeated after every failure is nagging.
    fn count_failure(&mut self, ended: &Result<Ending, Failure>) -> bool {
        let local = self
            .settings
            .as_ref()
            .is_ok_and(|settings| settings.helper == Some(::assist::Choice::Local));
        // A request that was stopped, or refused because no test may reach a
        // helper, or refused for want of a key or a download, says nothing
        // about how good the helper is at the work.
        let failed = local
            && match ended {
                Err(failure) => !matches!(
                    failure.kind,
                    FailureKind::Offline | FailureKind::NotReady | FailureKind::Unauthorized
                ),
                Ok(_) => false,
            };
        match failed {
            true => self.failed_in_a_row += 1,
            false => self.failed_in_a_row = 0,
        }
        failed
    }

    fn say_what_it_is_not_good_at(&mut self) {
        if self.failed_in_a_row == 2 && !self.said_what_it_is_not_good_at {
            self.said_what_it_is_not_good_at = true;
            self.note(
                "A helper over the internet would do better at this.",
                Some(Action::Settings),
            );
        }
    }

    /// Adds what a request cost, pricing it at the Claude model it went to.
    fn count(&mut self, spent: Usage, priced: Option<ClaudeModel>) {
        self.spent += spent;
        if let Some(model) = priced {
            *self.cents.get_or_insert(0.0) += model.cents(spent);
        }
    }

    fn failed(&mut self, failure: &Failure, prepared: &Prepared) {
        // Refused on the request's thread, where the count was kept; counted
        // again here, where a test reads it.
        if failure.kind == FailureKind::Offline {
            count_again(1);
        }
        match failure.kind {
            FailureKind::Unauthorized | FailureKind::NotReady | FailureKind::Rejected { .. } => {
                self.note(&failure.sentence, Some(Action::Settings))
            }
            FailureKind::Unreachable
            | FailureKind::RateLimited { .. }
            | FailureKind::Busy
            | FailureKind::Dropped
            | FailureKind::Garbled => self.retry_note(&failure.sentence, prepared.clone()),
            FailureKind::Offline | FailureKind::TooManySteps => self.note(&failure.sentence, None),
        }
    }

    /// A sentence whose Try Again asks `prepared` again.
    fn retry_note(&mut self, sentence: &str, prepared: Prepared) {
        self.retries.push((self.transcript.len(), prepared));
        self.note(sentence, Some(Action::Retry));
    }

    /// The requests let go: what each cost is counted when it ends, a tool
    /// one of them wants is answered as stopped, and nothing else is heard.
    fn hear_the_let_go(&mut self) {
        let mut ended = Vec::new();
        let mut refused = 0;
        self.let_go.retain(|(from, priced)| loop {
            match from.try_recv() {
                Ok(Message::Ended {
                    ended: how, spent, ..
                }) => {
                    ended.push((spent, *priced));
                    if matches!(&how, Err(failure) if failure.kind == FailureKind::Offline) {
                        refused += 1;
                    }
                    return false;
                }
                // Dropping the reply answers the tool as stopped.
                Ok(Message::Run(_, reply)) => drop(reply),
                Ok(Message::Event(_)) => {}
                Err(mpsc::TryRecvError::Empty) => return true,
                Err(mpsc::TryRecvError::Disconnected) => return false,
            }
        });
        for (spent, priced) in ended {
            self.count(spent, priced);
        }
        count_again(refused);
    }

    /// Marks the helper's words from `first` on as not kept.
    fn forget_from(&mut self, first: usize) {
        for entry in self.transcript.iter_mut().skip(first) {
            if let Entry::Said { kept, .. } = entry {
                *kept = false;
            }
        }
    }

    fn note(&mut self, sentence: &str, action: Option<Action>) {
        self.transcript.push(Entry::Note {
            sentence: sentence.to_owned(),
            action,
        });
    }

    /// Puts words back in the composer, unless the person has begun others.
    fn keep_words(&mut self, words: String) {
        if self.composer.trim().is_empty() {
            self.composer = words;
        }
    }

    // ---- the helper's choice -------------------------------------------

    /// The settings as the file holds them now, or the defaults.
    fn on_file(&self) -> Settings {
        match &self.path {
            Ok(path) => Settings::read(path).unwrap_or_default(),
            Err(_) => Settings::default(),
        }
    }

    /// Opens the settings box on the file as it is now, with `draft` in it
    /// when a card row has already made a choice.
    fn open_box(&mut self, draft: Option<Settings>, focus_key: bool) {
        let opened = self.on_file();
        let draft = draft.unwrap_or_else(|| opened.clone());
        let found = match self.found.is_empty() {
            true => None,
            false => Some(self.found.clone()),
        };
        self.settings_box = Some(SettingsBox::open(opened, draft, found, focus_key));
    }

    fn look(&mut self, ctx: &egui::Context) {
        let reach = Arc::clone(&self.reach);
        let wake = ctx.clone();
        let looking = Choosing::Looking(Background::spawn(
            move || counting(|| reach.look()),
            move || wake.request_repaint(),
        ));
        self.set_choosing(Some(looking));
    }

    /// Puts `next` where the card was. A look still under way is heard to
    /// its end, for the refusals its thread counted.
    fn set_choosing(&mut self, next: Option<Choosing>) {
        if let Some(Choosing::Looking(before)) = std::mem::replace(&mut self.choosing, next) {
            self.draining.push(Box::new(before));
        }
    }

    /// Takes `settings` as the ones in force, and says what changed. Another
    /// helper is another conversation; another place for the words is
    /// another agreement; the same helper told something new — a key, the
    /// fallback — is asked afresh next time, carrying on.
    fn adopt(&mut self, settings: Settings) {
        let place = ::assist::destination(&settings);
        let was = self.settings.as_ref().ok().cloned();
        let same_helper = was.as_ref().is_some_and(|was| was.same_helper(&settings));
        let same_place = was.is_some() && self.place == place;
        let changed = was.as_ref() != Some(&settings);
        self.settings = Ok(settings.clone());
        self.place = place;
        if settings.helper.is_some() {
            self.set_choosing(None);
        }
        if !same_place {
            self.consented = false;
            if let Some((prepared, _)) = self.consent.take() {
                self.keep_words(prepared.shown);
            }
        }
        if !same_helper {
            self.stop();
            let talked = self
                .session
                .take()
                .is_some_and(|s| !s.conversation().is_empty())
                || self.resume.take().is_some_and(|c| !c.is_empty());
            self.forget_from(0);
            if settings.helper.is_some() {
                let mut sentence = format!("Assist will use {}", settings.helper_name());
                if self.place.is_none() && settings.helper != Some(Choice::Local) {
                    sentence.push_str(", on this computer");
                }
                if talked {
                    sentence.push_str(", starting a new conversation");
                }
                sentence.push('.');
                self.note(&sentence, None);
            }
        } else if changed {
            self.renew = true;
        }
    }

    /// Saves `settings`, and takes them as the ones in force.
    /// Keeps `settings`, and lets go of what the last choice held.
    fn keep(&mut self, settings: Settings) -> Result<(), String> {
        // Another helper chosen: the model read for this one is a gigabyte
        // of memory nothing is going to ask anything of.
        let was_local = self
            .settings
            .as_ref()
            .is_ok_and(|now| now.helper == Some(::assist::Choice::Local));
        if was_local && settings.helper != Some(::assist::Choice::Local) {
            ::assist::local::forget();
        }
        let path = self
            .path
            .as_ref()
            .map_err(|why| format!("Assist's settings could not be saved: {why}."))?;
        let aside = settings
            .save(path)
            .map_err(|error| format!("Assist's settings could not be saved: {error}."))?;
        self.adopt(settings);
        if let Some(aside) = aside {
            let name = aside
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.note(
                &format!("The settings that could not be read were kept as {name}."),
                None,
            );
        }
        self.focus = true;
        Ok(())
    }

    fn chose(&mut self, chose: Chose, ctx: &egui::Context) -> Option<Chosen> {
        match chose {
            Chose::NotNow => Some(Chosen::Close),
            Chose::Again => {
                self.look(ctx);
                None
            }
            Chose::Use => {
                let Some(Choosing::Rows(rows)) = &mut self.choosing else {
                    return None;
                };
                let row = rows.rows.get(rows.picked).cloned()?;
                if !row.is_ready() {
                    rows.said = Some(::assist::LOCAL_NOT_READY.to_owned());
                    return None;
                }
                let model = rows.model;
                let mut settings = self.on_file();
                row.choose(&mut settings);
                match &row {
                    // A key to paste, or an address to give: the box asks, and
                    // nothing is kept until the service has answered.
                    Row::ClaudeWithKey | Row::Service => {
                        let focus_key = matches!(row, Row::ClaudeWithKey);
                        self.open_box(Some(settings), focus_key);
                    }
                    _ => {
                        if let Row::OllamaHere { models } = &row {
                            if let Some(model) = models.get(model) {
                                settings.ollama.model = model.name.clone();
                            }
                        }
                        let local = matches!(row, Row::Local);
                        if let Err(why) = self.keep(settings) {
                            if let Some(Choosing::Rows(rows)) = &mut self.choosing {
                                rows.said = Some(why);
                            }
                            return None;
                        }
                        // The helper on this computer is chosen and not yet
                        // downloaded: choosing it is asking for it, and the
                        // bar starts there and then rather than waiting for
                        // the person to find Settings.
                        if local && self.reach.downloaded().is_none() {
                            self.start_download(ctx);
                        }
                    }
                }
                None
            }
        }
    }

    // ---- drawing ---------------------------------------------------------

    /// The settings box, over the whole window. Drawn from the application's
    /// overlay, before anything else reads the keyboard.
    pub fn overlay(&mut self, ctx: &egui::Context) {
        let Some(open) = &mut self.settings_box else {
            return;
        };
        let path = self.path.as_ref().map(PathBuf::as_path);
        let shown = open.show(ctx, path, self.spent, self.cents, &self.reach);
        if let Some(found) = open.found_now() {
            self.found = found;
        }
        match shown {
            None => {}
            // The download is the pane's to run, not the box's: the box stays
            // open, and the bar under the transcript shows how it goes.
            Some(Closed::Download) => self.start_download(ctx),
            Some(Closed::Remove) => match self.reach.remove_download() {
                Ok(0) => self.note("There was nothing downloaded to remove.", None),
                Ok(freed) => {
                    let said = format!(
                        "The helper on this computer was removed: {} came back.",
                        ::assist::local::size_of(freed)
                    );
                    self.note(&said, None);
                    self.renew = true;
                }
                Err(why) => {
                    let said = format!("The helper could not be removed: {why}.");
                    self.note(&said, None);
                }
            },
            Some(Closed::Cancelled) => {
                if let Some(mut open) = self.settings_box.take() {
                    self.draining.extend(open.leftovers());
                }
            }
            Some(Closed::Saved { settings, checked }) => match self.keep(*settings) {
                Ok(()) => {
                    if let Some(mut open) = self.settings_box.take() {
                        self.draining.extend(open.leftovers());
                    }
                    if let Some(sentence) = checked {
                        self.note(&sentence, None);
                    }
                }
                Err(why) => {
                    if let Some(open) = &mut self.settings_box {
                        open.refuse(why);
                    }
                }
            },
        }
    }

    /// Draws the pane into `ui` — the application gives it the room, a panel
    /// on the right — and says what was chosen on it.
    pub fn show(&mut self, ui: &mut egui::Ui, offer: &Offer) -> Option<Chosen> {
        let ctx = ui.ctx().clone();
        self.ctx = Some(ctx.clone());
        let frame = ctx.cumulative_frame_nr();
        let just_opened = self.drawn.is_none_or(|last| last + 1 < frame);
        self.drawn = Some(frame);
        self.tend_download(&ctx);
        if just_opened {
            // Whatever had the keyboard when the pane was put away, the
            // document has had it since.
            self.composer_had_keyboard = false;
        }
        // Read again as the pane opens, unless there is a conversation to keep
        // going: the other application may have chosen meanwhile, or the file
        // been mended, or spoiled, by hand.
        let talking = self.running.is_some()
            || self
                .session
                .as_ref()
                .is_some_and(|session| !session.conversation().is_empty())
            || self
                .resume
                .as_ref()
                .is_some_and(|conversation| !conversation.is_empty());
        let reread = just_opened && !talking && !self.box_up();
        if reread {
            if let Ok(path) = &self.path {
                match Settings::read(path) {
                    Ok(settings) => self.adopt(settings),
                    Err(why) => {
                        self.settings = Err(why);
                        self.place = None;
                        if let Some((prepared, _)) = self.consent.take() {
                            self.keep_words(prepared.shown);
                        }
                    }
                }
            }
        }
        if offer.following != self.following {
            self.following = offer.following;
            self.scope = None;
        }
        if self.scope.is_some_and(|scope| scope >= offer.scopes.len()) {
            self.scope = None;
        }
        match (&self.settings, &self.choosing) {
            (Err(why), None) => self.choosing = Some(Choosing::Unreadable(why.clone())),
            // Read again as the pane opens, a file that cannot be read is said
            // again, whatever card was up when it closed.
            (Err(why), Some(_)) if reread => {
                let why = why.clone();
                self.set_choosing(Some(Choosing::Unreadable(why)));
            }
            (Err(_), _) => {}
            (Ok(settings), _) if settings.helper.is_some() => self.set_choosing(None),
            // Nothing chosen: look at the computer, and look again each time
            // the pane is opened, since what it has may have changed.
            (Ok(_), None | Some(Choosing::Unreadable(_))) => self.look(&ctx),
            (Ok(_), Some(Choosing::Rows(_))) if just_opened => self.look(&ctx),
            (Ok(_), _) => {}
        }

        // The keys the composer answers to, read before anything is drawn.
        let composer_has_keyboard = ctx.memory(|m| m.has_focus(composer_id()));
        let keys_here = composer_has_keyboard || self.composer_had_keyboard;
        let popup = egui::Popup::is_any_open(&ctx) || (self.popup_before && !just_opened);
        let (enter, escape, arrows) = match keys_here && !popup && !self.box_up() {
            true => ui.input_mut(|i| {
                let arrows = i.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::ArrowUp
                                | egui::Key::ArrowDown
                                | egui::Key::ArrowLeft
                                | egui::Key::ArrowRight,
                            pressed: true,
                            ..
                        }
                    )
                });
                (
                    crate::keys::take(i, egui::Modifiers::NONE, egui::Key::Enter),
                    crate::keys::take(i, egui::Modifiers::NONE, egui::Key::Escape),
                    arrows,
                )
            }),
            false => (false, false, false),
        };
        // The arrows move the composer's caret. egui holds them for a field
        // only from its second frame with the keyboard, and on the first
        // would move the keyboard itself somewhere else in the window.
        if arrows {
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }

        let mut chosen = None;
        self.header(ui, offer, &mut chosen);
        let foot = egui::Panel::bottom("ui-kit-assist-foot")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| self.foot(ui, offer, enter))
            .inner;
        chosen = chosen.or(foot);
        let body = crate::scroll::show(
            ui,
            egui::ScrollArea::vertical()
                .id_salt("ui-kit-assist-transcript")
                .auto_shrink([false, false])
                .stick_to_bottom(true),
            |ui| self.body(ui),
        )
        .inner;
        chosen = chosen.or(body);

        if escape {
            ctx.memory_mut(|m| m.surrender_focus(composer_id()));
            chosen = Some(Chosen::Leave);
        }
        if self.focus {
            let target = match &self.choosing {
                Some(Choosing::Rows(_)) => choosing::use_this_widget(&ctx),
                Some(_) => None,
                None => Some(composer_id()),
            };
            // A card still looking has no button yet: the keyboard waits for it.
            if let Some(target) = target {
                self.focus = false;
                ctx.memory_mut(|m| m.request_focus(target));
            }
        }
        // Kept for the next frame: egui keeps Escape and the arrows for a
        // field only from its second frame with the keyboard.
        self.composer_had_keyboard = !escape && ctx.memory(|m| m.has_focus(composer_id()));
        self.popup_before = egui::Popup::is_any_open(&ctx);
        chosen
    }

    fn header(&mut self, ui: &mut egui::Ui, offer: &Offer, chosen: &mut Option<Chosen>) {
        let header = egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.set_min_height(theme::PANE_HEADER - 10.0);
                ui.horizontal(|ui| {
                    if offer.tabs.is_empty() {
                        ui.label(egui::RichText::new("Assist").strong().size(theme::TEXT));
                    } else {
                        for (index, name) in offer.tabs.iter().enumerate() {
                            let label = egui::RichText::new(*name).strong().size(theme::TEXT);
                            if ui.selectable_label(index == offer.tab, label).clicked()
                                && index != offer.tab
                            {
                                *chosen = Some(Chosen::Tab(index));
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("×").frame(false))
                            .on_hover_text("Close the pane")
                            .clicked()
                        {
                            *chosen = Some(Chosen::Close);
                        }
                        let more = transcript::dots(ui).on_hover_text("More");
                        ui.ctx().data_mut(|d| d.insert_temp(more_id(), more.rect));
                        menu::under(&more, |ui| {
                            if menu::item(ui, "&Settings…", "").clicked() {
                                self.open_box(None, false);
                            }
                            if menu::item(ui, "&Clear the Conversation", "").clicked() {
                                self.clear();
                            }
                            if !offer.menu.is_empty() {
                                menu::sep(ui);
                            }
                            for (index, row) in offer.menu.iter().enumerate() {
                                if menu::item(ui, row, "").clicked() {
                                    *chosen = Some(Chosen::Menu(index));
                                }
                            }
                        });
                        if self.running.is_some() {
                            if transcript::plain(ui, "Stop").clicked() {
                                self.stop();
                            }
                            ui.add(egui::Spinner::new().size(12.0).color(theme::INK_SOFT));
                        }
                    });
                });
                if let Ok(settings) = &self.settings {
                    if settings.helper.is_some() {
                        ui.label(
                            egui::RichText::new(named(settings, &self.place))
                                .size(theme::TEXT_SMALL)
                                .color(theme::INK_SOFT),
                        );
                    }
                }
            });
        let rule = header.response.rect.bottom() + 0.5;
        ui.painter().hline(
            header.response.rect.x_range(),
            rule,
            egui::Stroke::new(1.0, theme::CHROME_RULE),
        );
    }

    /// The transcript, and the cards that stand at its end while they wait:
    /// the first-run card, and the agreement to what is sent.
    fn body(&mut self, ui: &mut egui::Ui) -> Option<Chosen> {
        let mut chosen = None;
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        let busy = self.running.is_some() || self.consent.is_some();
        let mut pressed = None;
        for (index, entry) in self.transcript.iter().enumerate() {
            if let Some(press) = transcript::entry(ui, entry, busy) {
                pressed = Some((index, press));
            }
        }
        match pressed {
            Some((_, Pressed::Card { card, action })) => {
                chosen = Some(Chosen::Card { card, action })
            }
            Some((_, Pressed::Note(Action::Settings))) => self.open_box(None, false),
            // The failed request, asked again: the application puts its
            // document part into words afresh, since its tools may have
            // changed the document before it failed.
            Some((index, Pressed::Note(Action::Retry))) if !busy => {
                if let Some((_, prepared)) = self.retries.iter().find(|(at, _)| *at == index) {
                    chosen = Some(Chosen::Ask(Asked {
                        words: prepared.shown.clone(),
                        scope: prepared.scope,
                        effort: prepared.effort,
                    }));
                }
            }
            Some(_) | None => {}
        }
        if let Some(choosing) = &mut self.choosing {
            let mut refused = 0;
            let chose = choosing::card(ui, choosing, &mut refused);
            count_again(refused);
            if let Choosing::Rows(rows) = choosing {
                self.found = rows.rows.clone();
            }
            if let Some(chose) = chose {
                chosen = self.chose(chose, &ui.ctx().clone()).or(chosen);
            }
        }
        if let Some((prepared, to)) = &self.consent {
            let mut answer = None;
            transcript::card_frame(ui, theme::ACCENT, |ui| {
                ui.label(egui::RichText::new("Before this is sent").strong());
                ui.add(
                    egui::Label::new(format!(
                        "{} will be sent to {to}. Assist asks once for each place \
                         your words may go.",
                        prepared.leaves
                    ))
                    .wrap(),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if transcript::primary(ui, "Send").clicked() {
                        answer = Some(true);
                    }
                    if transcript::plain(ui, "Not this time").clicked() {
                        answer = Some(false);
                    }
                });
            });
            match answer {
                Some(true) => {
                    if let Some((prepared, _)) = self.consent.take() {
                        self.consented = true;
                        self.start(prepared);
                    }
                }
                Some(false) => {
                    if let Some((prepared, _)) = self.consent.take() {
                        self.keep_words(prepared.shown);
                        self.focus = true;
                    }
                }
                None => {}
            }
        }
        ui.add_space(8.0);
        chosen
    }

    /// The bottom of the pane: the download's bar, the quick verbs, the
    /// composer, the scope and Send.
    fn foot(&mut self, ui: &mut egui::Ui, offer: &Offer, enter: bool) -> Option<Chosen> {
        let mut chosen = None;
        let ready = self.running.is_none()
            && self.consent.is_none()
            && self.choosing.is_none()
            && self.settings.as_ref().is_ok_and(|s| s.helper.is_some());
        if let Some(progress) = self.download {
            if download_bar(ui, progress) {
                self.stop_download();
                chosen = Some(Chosen::StopDownload);
            }
            ui.add_space(6.0);
        }
        let scope = self.scope();
        if !offer.verbs.is_empty() {
            ui.add_enabled_ui(ready, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
                    for verb in offer.verbs {
                        if transcript::chip(ui, verb.label, false).clicked() {
                            match verb.label.ends_with('…') {
                                // The verb needs the person's words: it starts
                                // the request and waits for the rest.
                                true => {
                                    self.composer = verb.asks.to_owned();
                                    self.focus = true;
                                    let end = self.composer.chars().count();
                                    let mut state = egui::text_edit::TextEditState::load(
                                        ui.ctx(),
                                        composer_id(),
                                    )
                                    .unwrap_or_default();
                                    state.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::one(egui::text::CCursor::new(
                                            end,
                                        )),
                                    ));
                                    state.store(ui.ctx(), composer_id());
                                }
                                false => {
                                    chosen = Some(Chosen::Ask(Asked {
                                        words: verb.asks.to_owned(),
                                        scope,
                                        effort: Effort::Low,
                                    }));
                                }
                            }
                        }
                    }
                });
            });
            ui.add_space(6.0);
        }
        let hint = match ready || self.running.is_some() {
            true => "Say what you want done, in your own words",
            false => "Choose a helper first",
        };
        ui.scope(|ui| {
            dialog::form_style(ui.style_mut());
            egui::ScrollArea::vertical()
                .id_salt("ui-kit-assist-composer-scroll")
                .max_height(120.0)
                .show(ui, |ui| {
                    ui.add_enabled_ui(self.choosing.is_none(), |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.composer)
                                .id(composer_id())
                                .hint_text(hint)
                                .desired_rows(2)
                                .desired_width(f32::INFINITY)
                                .return_key(egui::KeyboardShortcut::new(
                                    egui::Modifiers::SHIFT,
                                    egui::Key::Enter,
                                )),
                        )
                    });
                });
        });
        // Escape and the arrows are the composer's while it has the keyboard:
        // egui would otherwise give the focus up on Escape before anything
        // here could read what the key meant.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                composer_id(),
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if !offer.scopes.is_empty() {
                ui.label(
                    egui::RichText::new("About:")
                        .size(theme::TEXT_SMALL)
                        .color(theme::INK_SOFT),
                );
                if let Some(chip) = offer.scopes.get(scope) {
                    let response = scope_chip(ui, chip.name);
                    menu::under(&response, |ui| {
                        for (index, other) in offer.scopes.iter().enumerate() {
                            if menu::check(ui, other.name, "", index == scope).clicked() {
                                self.scope = Some(index);
                            }
                        }
                    });
                    if let Some(words) = chip.words {
                        ui.label(
                            egui::RichText::new(format!("{} words", thousands(words as u64)))
                                .size(theme::TEXT_SMALL)
                                .color(theme::INK_SOFT),
                        );
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let words = self.composer.trim();
                let can = ready && !words.is_empty();
                let send = ui
                    .add_enabled_ui(can, |ui| transcript::primary(ui, "Send"))
                    .inner;
                if (send.clicked() || (enter && can)) && chosen.is_none() {
                    chosen = Some(Chosen::Ask(Asked {
                        words: words.to_owned(),
                        scope,
                        effort: Effort::Usual,
                    }));
                    self.composer.clear();
                }
            });
        });
        chosen
    }
}

impl Drop for Assist {
    /// A window that goes stops what it asked: nothing more is sent, or paid
    /// for, on behalf of a pane nobody can see.
    fn drop(&mut self) {
        if let Some(running) = &self.running {
            running.request.stop.stop();
        }
    }
}

/// The scope chip: a pill with its name, and a chevron drawn after it.
fn scope_chip(ui: &mut egui::Ui, name: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        name.to_owned(),
        egui::FontId::proportional(theme::TEXT_SMALL),
        theme::INK,
    );
    let size = egui::vec2(galley.size().x + 32.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visuals = ui.style().interact(&response);
    let edge = match response.hovered() || response.has_focus() {
        true => theme::FIELD_EDGE_HOT,
        false => theme::FIELD_EDGE,
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        11.0,
        theme::FIELD,
        egui::Stroke::new(1.0, edge),
        egui::StrokeKind::Inside,
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(1.0),
            12.0,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Outside,
        );
    }
    let text_at = egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_at, galley, visuals.text_color());
    let at = egui::pos2(rect.right() - 12.0, rect.center().y);
    transcript::chevron(painter, at, theme::INK_SOFT);
    response
}

/// The download's bar: how much of how much, the bar, and Stop. Says whether
/// Stop was pressed.
fn download_bar(ui: &mut egui::Ui, progress: Progress) -> bool {
    let mut stop = false;
    ui.horizontal(|ui| {
        let words = match progress.total {
            Some(total) => format!(
                "Downloading the helper: {} of {}",
                ::assist::size_words(progress.done),
                ::assist::size_words(total)
            ),
            None => format!(
                "Downloading the helper: {}",
                ::assist::size_words(progress.done)
            ),
        };
        ui.label(
            egui::RichText::new(words)
                .size(theme::TEXT_SMALL)
                .color(theme::INK),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            stop = transcript::plain(ui, "Stop").clicked();
        });
    });
    let (track, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(track, 3.0, theme::CHROME_RULE);
    if let Some(total) = progress.total.filter(|total| *total > 0) {
        let share = (progress.done as f64 / total as f64).clamp(0.0, 1.0) as f32;
        let filled =
            egui::Rect::from_min_size(track.min, egui::vec2(track.width() * share, track.height()));
        painter.rect_filled(filled, 3.0, theme::ACCENT);
    }
    stop
}

/// A count with its thousands marked: "4,210".
pub(crate) fn thousands(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
