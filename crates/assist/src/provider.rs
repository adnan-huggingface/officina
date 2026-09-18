//! The one shape every helper answers in, and the helper the settings name.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::anthropic::{Anthropic, Login};
use crate::compatible::Compatible;
use crate::conversation::{Block, Conversation, Message};
use crate::event::{Ending, Event, Failure, FailureKind, Usage};
use crate::machine::{Machine, ThisComputer};
use crate::settings::{Choice, ClaudeLogin, Settings};
use crate::tool::Tool;

/// How much thought an answer is worth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Effort {
    /// The helper's own judgment: a request the person typed.
    #[default]
    Usual,
    /// Little: a quick verb such as "Fix spelling", where speed and cost matter
    /// more than depth. Only helpers that can be told so are told.
    Low,
}

/// Everything a helper is given for one exchange.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The editor's instructions, the same for every request of a session so
    /// that a service can read them back from its cache.
    pub system: &'a str,
    pub tools: &'a [Tool],
    pub conversation: &'a Conversation,
    pub effort: Effort,
}

/// Stop, as the pane's button sets it and a helper reads it before it sends a
/// request and between two pieces of an answer.
///
/// A helper waiting for an answer to begin — Ollama loading a model, a server
/// that went quiet — reads it only when something arrives, which may be
/// minutes. The window must therefore let go of a stopped request rather than
/// wait for it to end, as the file chooser's "Stop Waiting" lets go of a
/// chooser.
#[derive(Debug, Clone, Default)]
pub struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    pub fn new() -> StopFlag {
        StopFlag::default()
    }

    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What one exchange came to: the helper's message as the conversation keeps
/// it, how it ended, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub message: Message,
    pub ending: Result<Ending, Failure>,
    pub usage: Usage,
}

impl Answer {
    pub(crate) fn failed(failure: Failure) -> Answer {
        Answer::failed_with(failure, Usage::default())
    }

    /// A failure after the service had begun to count.
    pub(crate) fn failed_with(failure: Failure, usage: Usage) -> Answer {
        Answer {
            message: Message::assistant(Vec::new()),
            ending: Err(failure),
            usage,
        }
    }

    pub(crate) fn ended(content: Vec<Block>, ending: Ending, usage: Usage) -> Answer {
        Answer {
            message: Message::assistant(content),
            ending: Ok(ending),
            usage,
        }
    }
}

/// Something that answers: a service over the network, a model on the
/// computer, or a script.
pub trait Provider: Send {
    /// The helper as the transcript names it: "Claude", "Ollama".
    fn name(&self) -> &str;

    /// One exchange: the request out, the answer back. Words are handed to
    /// `text` as they arrive, and everything else is in the [`Answer`]. The
    /// stop flag is read between pieces of the answer; once it is set the
    /// answer ends [`Ending::Stopped`] and the connection is let go.
    fn answer(&mut self, request: &Request, stop: &StopFlag, text: &mut dyn FnMut(&str)) -> Answer;
}

/// One exchange as the events a pane shows: the words as they arrive, then the
/// tools wanted, then how it ended — exactly one [`Event::Done`] or
/// [`Event::Failed`], last.
pub fn stream(
    provider: &mut dyn Provider,
    request: &Request,
    stop: &StopFlag,
    sink: &mut dyn FnMut(Event),
) -> Answer {
    let answer = provider.answer(request, stop, &mut |words| {
        sink(Event::Text(words.to_owned()));
    });
    if matches!(answer.ending, Ok(Ending::WantsTools | Ending::Finished)) {
        for call in answer.message.calls() {
            sink(Event::ToolCall(call.clone()));
        }
    }
    sink(match &answer.ending {
        Ok(ending) => Event::Done(ending.clone()),
        Err(failure) => Event::Failed(failure.clone()),
    });
    answer
}

/// Whether the helper on this computer can answer yet. It cannot until its
/// runtime and its download are built (phase 5 of the programme): until then
/// its row is offered, as the user asked, and choosing it says it is not ready
/// rather than keeping a choice that cannot answer.
pub const LOCAL_READY: bool = true;

/// The model in `cache`, read and ready, or why it is not.
fn local_helper(cache: &std::path::Path) -> Result<crate::local::Local, Failure> {
    crate::offline::check(LOCAL)?;
    crate::local::Local::load(&crate::local::folder(cache))
}

/// A helper that is there but cannot answer, and says why in its own words.
struct Refuses {
    name: &'static str,
    failure: Failure,
}

impl Provider for Refuses {
    fn name(&self) -> &str {
        self.name
    }

    fn answer(&mut self, _: &Request, _: &StopFlag, _: &mut dyn FnMut(&str)) -> Answer {
        Answer::failed(self.failure.clone())
    }
}

const NO_HELPER: &str = "No helper has been chosen yet.";
const LOCAL: &str = "The helper on this computer";
/// What choosing the helper on this computer says before its weights are
/// there: it is not a refusal but a thing to do, and the pane's download does
/// it.
pub const LOCAL_NOT_READY: &str = "The helper on this computer has not been downloaded yet. \
                                   Assist's ⋯ menu ▸ Settings downloads it.";

/// The helper the settings name, ready to ask.
pub fn connect(settings: &Settings) -> Box<dyn Provider> {
    connect_in(settings, None)
}

/// The same, told where downloaded weights live.
///
/// **`assist` does not know where the cache is.** Which directory holds a
/// person's downloads is the suite's business, not a helper's, and the
/// application that knows passes it: a crate that guessed would guess
/// differently from `ui_kit::paths` on some platform and download the model
/// twice.
pub fn connect_in(settings: &Settings, cache: Option<&std::path::Path>) -> Box<dyn Provider> {
    match settings.helper {
        None => Box::new(NotReady {
            name: "Assist",
            sentence: NO_HELPER,
        }),
        // The helper on this computer, if its weights have been downloaded.
        // The model is read here, on the request's own thread, because it is
        // a second of reading and a window must not stop for it.
        Some(Choice::Local) => match cache.map(local_helper) {
            Some(Ok(local)) => Box::new(local),
            Some(Err(failure)) => Box::new(Refuses {
                name: LOCAL,
                failure,
            }),
            None => Box::new(NotReady {
                name: LOCAL,
                sentence: LOCAL_NOT_READY,
            }),
        },
        Some(Choice::Claude) => Box::new(claude(settings)),
        Some(Choice::Ollama) => Box::new(OllamaHere {
            answers: ollama(settings),
            address: settings.ollama.address.clone(),
            model: settings.ollama.model.trim().to_owned(),
            known: false,
        }),
        Some(Choice::Service) => Box::new(service(settings)),
    }
}

/// What is said of a model Ollama passes on to `place`.
fn elsewhere(model: &str, place: &str) -> String {
    format!(
        "Ollama passes what “{model}” is asked on to {place}, and Assist uses Ollama only \
         for what runs on this computer. Choose another model in Assist's settings."
    )
}

/// Ollama, answering only with a model it runs itself. Its cloud models, and
/// any model made with a remote host, are listed beside its own and send what
/// they are asked elsewhere; nothing says so to the person, who was told
/// Ollama keeps what they write on this computer. So before its first
/// request the helper asks Ollama where the model runs, and sends nothing
/// until the answer is "here".
struct OllamaHere {
    answers: Compatible,
    address: String,
    model: String,
    /// Whether the model is known to run here.
    known: bool,
}

impl Provider for OllamaHere {
    fn name(&self) -> &str {
        self.answers.name()
    }

    fn answer(&mut self, request: &Request, stop: &StopFlag, text: &mut dyn FnMut(&str)) -> Answer {
        if !self.known {
            match crate::machine::where_ollama_runs(&self.address, &self.model) {
                Ok(None) => self.known = true,
                Ok(Some(place)) => {
                    return Answer::failed(Failure::new(
                        FailureKind::NotReady,
                        format!("{} Nothing was sent.", elsewhere(&self.model, &place)),
                    ))
                }
                Err(failure) => return Answer::failed(failure),
            }
        }
        self.answers.answer(request, stop, text)
    }
}

/// Whether the helper the settings name is there, and takes the key: one
/// question, asked of the service's list of what it can answer with, which
/// sends nothing of the document and costs nothing. `Ok` says what the service
/// answered; a key is kept only after it.
pub fn check(settings: &Settings) -> Result<String, Failure> {
    check_in(settings, None)
}

/// The same, told where downloaded weights live.
pub fn check_in(settings: &Settings, cache: Option<&std::path::Path>) -> Result<String, Failure> {
    match settings.helper {
        None => {
            crate::offline::check("Assist")?;
            Err(Failure::new(FailureKind::NotReady, NO_HELPER))
        }
        Some(Choice::Local) => {
            crate::offline::check(LOCAL)?;
            let Some(cache) = cache else {
                return Err(Failure::new(FailureKind::NotReady, LOCAL_NOT_READY));
            };
            // Whether the files are there, not whether they load: a check is
            // a question about the settings, and reading a gigabyte to answer
            // it would hold the settings box for as long as the first request
            // takes. What the files are is settled by the hash they were
            // downloaded under, and a file that will not load is said when
            // the first request asks it to.
            if !crate::local::have(cache) {
                return Err(Failure::new(FailureKind::NotReady, LOCAL_NOT_READY));
            }
            Ok(format!(
                "{} is ready on this computer: {}.",
                crate::local::MODEL.name,
                crate::local::GOOD_AT
            ))
        }
        Some(Choice::Claude) => claude(settings).check(),
        Some(Choice::Ollama) => {
            let model = settings.ollama.model.trim();
            let listed = ollama(settings).models()?;
            if model.is_empty() {
                return Err(Failure::new(
                    FailureKind::NotReady,
                    "Ollama answered. Choose which of its models Assist should use.",
                ));
            }
            // Ollama lists a model pulled without a tag under `latest`.
            let has = listed
                .iter()
                .any(|name| name == model || *name == format!("{model}:latest"));
            if has {
                if let Some(place) =
                    crate::machine::where_ollama_runs(&settings.ollama.address, model)?
                {
                    return Err(Failure::new(
                        FailureKind::NotReady,
                        elsewhere(model, &place),
                    ));
                }
            }
            match has {
                true => Ok(format!("Ollama answered, and has {model}.")),
                false => Err(Failure::new(
                    FailureKind::NotReady,
                    format!(
                        "Ollama answered, but has no “{model}”. Pull it with \
                         “ollama pull {model}”, or choose another."
                    ),
                )),
            }
        }
        Some(Choice::Service) => {
            let service_settings = &settings.service;
            let host = crate::http::host_of(&service_settings.address);
            if service_settings.address.trim().is_empty() {
                crate::offline::check("Another service")?;
                return Err(Failure::new(
                    FailureKind::NotReady,
                    "Give the service's address.",
                ));
            }
            let listed = match service(settings).models() {
                Ok(listed) => listed,
                // Not every service lists what it answers with. One that
                // says it has no such list has answered, and a key it does
                // not take is refused by the first request instead.
                Err(Failure {
                    kind:
                        FailureKind::Rejected {
                            status: 404 | 405 | 501,
                        },
                    ..
                }) => {
                    // The commonest mistake in an address is to leave off the
                    // `/v1` the service documents, and it answers exactly so.
                    let address = service_settings.address.trim_end_matches('/');
                    if !address.ends_with("/v1") {
                        let mut guessed = settings.clone();
                        guessed.service.address = format!("{address}/v1");
                        match service(&guessed).models() {
                            Ok(_) => {
                                return Err(Failure::new(
                                    FailureKind::Rejected { status: 404 },
                                    format!(
                                        "{host} answers at {address}/v1, not at {address}. \
                                         Add /v1 to the address."
                                    ),
                                ))
                            }
                            // The service is there, and has refused the key.
                            Err(Failure {
                                kind: FailureKind::Unauthorized,
                                ..
                            }) => {
                                return Err(Failure::new(
                                    FailureKind::Unauthorized,
                                    format!(
                                        "{host} answers at {address}/v1, not at {address}, \
                                         and did not accept the key there. Add /v1 to the \
                                         address, and check the key."
                                    ),
                                ))
                            }
                            Err(_) => {}
                        }
                    }
                    return Ok(format!(
                        "{host} answered, but not with a list of what it can answer \
                         with, as most services do. The key is checked when the first \
                         request goes; if that fails, check the address."
                    ));
                }
                Err(failure) => return Err(failure),
            };
            let accepted = match service_settings.key.is_empty() {
                true => format!("{host} answered"),
                false => format!("{host} accepted the key"),
            };
            let model = service_settings.model.trim();
            Ok(
                match (model.is_empty(), listed.iter().any(|name| name == model)) {
                    (true, _) => format!("{accepted}. Give the name of the model it should use."),
                    (false, true) => format!("{accepted}, and lists {model}."),
                    (false, false) => format!(
                        "{accepted}. It does not list “{model}”, which some services do not; \
                     if a request fails, check the name."
                    ),
                },
            )
        }
    }
}

fn ollama(settings: &Settings) -> Compatible {
    Compatible::new(
        "Ollama",
        &format!("{}/v1", settings.ollama.address.trim_end_matches('/')),
        None,
        &settings.ollama.model,
    )
}

fn service(settings: &Settings) -> Compatible {
    let key = Some(settings.service.key.clone()).filter(|key| !key.is_empty());
    Compatible::new(
        &crate::http::host_of(&settings.service.address),
        &settings.service.address,
        key,
        &settings.service.model,
    )
}

/// Where a request's words go when the helper the settings name is asked, if
/// that is anywhere but this computer: the name the pane gives before the
/// first request is sent — "Anthropic", "Claude at relay.example.com",
/// "api.example.com". A helper at one of this computer's own addresses sends
/// nothing away, except Claude, whose address here is a relay that passes the
/// words on.
pub fn destination(settings: &Settings) -> Option<String> {
    match settings.helper? {
        Choice::Local => None,
        Choice::Claude => {
            let host = crate::http::host_of(&claude_address(settings));
            Some(
                match host == crate::http::host_of(crate::anthropic::ADDRESS) {
                    true => "Anthropic".to_owned(),
                    false => format!("Claude at {host}"),
                },
            )
        }
        Choice::Ollama => {
            let address = &settings.ollama.address;
            (!crate::http::is_loopback(address))
                .then(|| format!("Ollama at {}", crate::http::host_of(address)))
        }
        Choice::Service => {
            let address = &settings.service.address;
            (!crate::http::is_loopback(address)).then(|| crate::http::host_of(address))
        }
    }
}

/// The address Claude is asked at: the settings', or for a login taken from
/// the environment the environment's, as the Anthropic SDKs take it — a base
/// address set beside a token is where that token is meant to go. Under a
/// test the environment is not read.
fn claude_address(settings: &Settings) -> String {
    let claude = &settings.claude;
    match claude.login {
        ClaudeLogin::Environment => (!crate::offline::active())
            .then(|| ThisComputer.var("ANTHROPIC_BASE_URL"))
            .flatten()
            .unwrap_or_else(|| claude.address.clone()),
        ClaudeLogin::Key | ClaudeLogin::Ant => claude.address.clone(),
    }
}

fn claude(settings: &Settings) -> Anthropic {
    let claude = &settings.claude;
    let address = claude_address(settings);
    let login: crate::anthropic::LoginSource = match claude.login {
        ClaudeLogin::Key => {
            let key = claude.key.clone();
            Box::new(move || match key.is_empty() {
                true => Err(Failure::new(
                    FailureKind::Unauthorized,
                    "No Anthropic key has been given. Add one in Assist's settings.",
                )),
                false => Ok(Login::Key(key.clone())),
            })
        }
        // Under a test the environment is not read at all; the request is
        // refused before the login would be.
        ClaudeLogin::Environment => Box::new(|| {
            if let Some(key) = ThisComputer.var("ANTHROPIC_API_KEY") {
                return Ok(Login::Key(key));
            }
            if let Some(token) = ThisComputer.var("ANTHROPIC_AUTH_TOKEN") {
                return Ok(Login::Token(token));
            }
            Err(Failure::new(
                FailureKind::Unauthorized,
                "The Anthropic key this computer had is no longer set. \
                 Choose another way in Assist's settings.",
            ))
        }),
        // The token is short-lived, so it is asked for on every request;
        // `ant` renews it when it must.
        ClaudeLogin::Ant => Box::new(|| {
            ThisComputer.ant_token().map(Login::Token).ok_or_else(|| {
                Failure::new(
                    FailureKind::Unauthorized,
                    "The login from the ant command is not there any more. \
                     Run `ant auth login` again, or choose another way in \
                     Assist's settings.",
                )
            })
        }),
    };
    Anthropic::new(&address, login, &claude.model).fallback(claude.fallback)
}

/// A helper with nothing behind it yet, which says so when asked.
struct NotReady {
    name: &'static str,
    sentence: &'static str,
}

impl Provider for NotReady {
    fn name(&self) -> &str {
        self.name
    }

    fn answer(&mut self, _: &Request, _: &StopFlag, _: &mut dyn FnMut(&str)) -> Answer {
        if let Err(refused) = crate::offline::check(self.name) {
            return Answer::failed(refused);
        }
        Answer::failed(Failure::new(FailureKind::NotReady, self.sentence))
    }
}
