//! A helper that plays canned turns: the only helper a test may use.
//!
//! It answers in exactly the events a real helper streams, so the pane and the
//! applications are tested against the same shapes Claude and Ollama produce,
//! and it keeps what it was asked, so a test can read what would have been
//! sent.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::conversation::{Block, Conversation, ToolCall};
use crate::event::{Ending, Failure, FailureKind, Usage};
use crate::provider::{Answer, Effort, Provider, Request, StopFlag};

/// One canned answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    text: Vec<String>,
    calls: Vec<(String, Value)>,
    ending: Option<Result<Ending, Failure>>,
    usage: Usage,
}

impl Turn {
    /// An answer that says `words` and ends.
    pub fn says(words: &str) -> Turn {
        Turn {
            text: vec![words.to_owned()],
            calls: Vec::new(),
            ending: None,
            usage: Usage::default(),
        }
    }

    /// An answer that asks for one tool and nothing else.
    pub fn calls(tool: &str, input: Value) -> Turn {
        Turn {
            text: Vec::new(),
            calls: vec![(tool.to_owned(), input)],
            ending: None,
            usage: Usage::default(),
        }
    }

    /// An answer that ends at once in `ending`.
    pub fn ends(ending: Ending) -> Turn {
        Turn {
            text: Vec::new(),
            calls: Vec::new(),
            ending: Some(Ok(ending)),
            usage: Usage::default(),
        }
    }

    /// A request that fails with `sentence`.
    pub fn fails(kind: FailureKind, sentence: &str) -> Turn {
        Turn {
            text: Vec::new(),
            calls: Vec::new(),
            ending: Some(Err(Failure::new(kind, sentence))),
            usage: Usage::default(),
        }
    }

    /// More words, arriving as a piece of their own.
    pub fn then_says(mut self, words: &str) -> Turn {
        self.text.push(words.to_owned());
        self
    }

    /// What the answer is said to have cost, as a service counts it.
    pub fn costs(mut self, usage: Usage) -> Turn {
        self.usage = usage;
        self
    }

    /// Another tool asked for in the same answer.
    pub fn and_calls(mut self, tool: &str, input: Value) -> Turn {
        self.calls.push((tool.to_owned(), input));
        self
    }
}

/// What the scripted helper was asked, one entry per exchange.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub system: String,
    pub tools: Vec<String>,
    pub conversation: Conversation,
    pub effort: Effort,
}

pub struct Scripted {
    turns: VecDeque<Turn>,
    heard: Arc<Mutex<Vec<Heard>>>,
    calls: usize,
}

impl Scripted {
    pub fn new(turns: impl IntoIterator<Item = Turn>) -> Scripted {
        Scripted {
            turns: turns.into_iter().collect(),
            heard: Arc::default(),
            calls: 0,
        }
    }

    /// What the helper will have been asked, readable after it has been moved
    /// into a session.
    pub fn heard(&self) -> Arc<Mutex<Vec<Heard>>> {
        Arc::clone(&self.heard)
    }
}

impl Provider for Scripted {
    fn name(&self) -> &str {
        "The scripted helper"
    }

    fn answer(&mut self, request: &Request, stop: &StopFlag, text: &mut dyn FnMut(&str)) -> Answer {
        // As a real helper does: once Stop is pressed, nothing more is sent.
        if stop.is_set() {
            return Answer::ended(Vec::new(), Ending::Stopped, Usage::default());
        }
        self.heard.lock().unwrap().push(Heard {
            system: request.system.to_owned(),
            tools: request.tools.iter().map(|tool| tool.name.clone()).collect(),
            conversation: request.conversation.clone(),
            effort: request.effort,
        });
        let Some(turn) = self.turns.pop_front() else {
            return Answer::failed(Failure::new(
                FailureKind::NotReady,
                "The scripted helper has no more turns: the test asked more than it wrote.",
            ));
        };
        let mut content = Vec::new();
        for piece in &turn.text {
            if stop.is_set() {
                return Answer::ended(content, Ending::Stopped, Usage::default());
            }
            text(piece);
            match content.last_mut() {
                Some(Block::Text(words)) => words.push_str(piece),
                _ => content.push(Block::Text(piece.clone())),
            }
        }
        for (name, input) in turn.calls.iter().cloned() {
            self.calls += 1;
            content.push(Block::ToolCall(ToolCall {
                id: format!("call_{}", self.calls),
                name,
                input,
            }));
        }
        if stop.is_set() {
            return Answer::ended(content, Ending::Stopped, Usage::default());
        }
        let ending = turn.ending.unwrap_or(Ok(match turn.calls.is_empty() {
            true => Ending::Finished,
            false => Ending::WantsTools,
        }));
        Answer {
            message: crate::conversation::Message::assistant(content),
            ending,
            usage: turn.usage,
        }
    }
}
