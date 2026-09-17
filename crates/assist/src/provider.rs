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

/// The helper the settings name, ready to ask.
pub fn connect(settings: &Settings) -> Box<dyn Provider> {
    match settings.helper {
        None => Box::new(NotReady {
            name: "Assist",
            sentence: "No helper has been chosen yet.",
        }),
        // Phase 5 of the programme: until the runtime and its download exist,
        // the row says what it will be and the request says it is not there.
        Some(Choice::Local) => Box::new(NotReady {
            name: "The helper on this computer",
            sentence: "The helper on this computer is not ready yet. \
                       Choose Claude or Ollama in Assist's settings for now.",
        }),
        Some(Choice::Claude) => Box::new(claude(settings)),
        Some(Choice::Ollama) => Box::new(Compatible::new(
            "Ollama",
            &format!("{}/v1", settings.ollama.address.trim_end_matches('/')),
            None,
            &settings.ollama.model,
        )),
        Some(Choice::Service) => {
            let key = Some(settings.service.key.clone()).filter(|key| !key.is_empty());
            Box::new(Compatible::new(
                &crate::http::host_of(&settings.service.address),
                &settings.service.address,
                key,
                &settings.service.model,
            ))
        }
    }
}

fn claude(settings: &Settings) -> Anthropic {
    let claude = &settings.claude;
    let (address, login): (String, crate::anthropic::LoginSource) = match claude.login {
        ClaudeLogin::Key => {
            let key = claude.key.clone();
            (
                claude.address.clone(),
                Box::new(move || match key.is_empty() {
                    true => Err(Failure::new(
                        FailureKind::Unauthorized,
                        "No Anthropic key has been given. Add one in Assist's settings.",
                    )),
                    false => Ok(Login::Key(key.clone())),
                }),
            )
        }
        // The key and the address both come from the environment, as the
        // Anthropic SDKs take them: a base address set beside a token is where
        // that token is meant to go.
        // Under a test the environment is not read at all; the request is
        // refused before the login would be.
        ClaudeLogin::Environment => (
            (!crate::offline::active())
                .then(|| ThisComputer.var("ANTHROPIC_BASE_URL"))
                .flatten()
                .unwrap_or_else(|| claude.address.clone()),
            Box::new(|| {
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
        ),
        // The token is short-lived, so it is asked for on every request;
        // `ant` renews it when it must.
        ClaudeLogin::Ant => (
            claude.address.clone(),
            Box::new(|| {
                ThisComputer.ant_token().map(Login::Token).ok_or_else(|| {
                    Failure::new(
                        FailureKind::Unauthorized,
                        "The login from the ant command is not there any more. \
                         Run `ant auth login` again, or choose another way in \
                         Assist's settings.",
                    )
                })
            }),
        ),
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
