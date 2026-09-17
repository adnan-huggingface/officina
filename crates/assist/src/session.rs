//! A conversation with one helper, and the loop that runs its tools.
//!
//! A request is a turn of the user's, then as many exchanges as the helper
//! needs: each time it asks for tools, the host runs them — against the
//! document, on the document's own thread — and every result goes back in one
//! turn. The request is over when the helper finishes, or when anything else
//! happens, and in that case the conversation is cut back to where the request
//! began: a half-answered request is not something to build the next one on.
//! What the tools already did to the document stays there, where the person can
//! see it and take it back.

use crate::conversation::{Block, Conversation, Message, ToolCall, ToolResult};
use crate::event::{Ending, Event, Failure, FailureKind, Usage};
use crate::provider::{stream, Effort, Provider, Request, StopFlag};
use crate::tool::Tool;

/// The most exchanges one request may take. An editor's request is a handful —
/// read, write, check — and a helper that is still asking for tools after this
/// many is going round in a circle at the user's expense.
pub const MOST_STEPS: usize = 24;

fn says_nothing(message: &Message) -> bool {
    message.content.iter().all(|block| {
        matches!(block, Block::Text(words) if words.trim().is_empty())
            || matches!(block, Block::Opaque { .. })
    })
}

/// The application's side of a request.
pub trait Host {
    /// What the helper said or did, as it happens.
    fn event(&mut self, event: &Event);

    /// Runs one tool against the document and says what came of it.
    fn run(&mut self, call: &ToolCall) -> ToolResult;
}

pub struct Session {
    provider: Box<dyn Provider>,
    system: String,
    tools: Vec<Tool>,
    conversation: Conversation,
    usage: Usage,
}

impl Session {
    pub fn new(
        provider: Box<dyn Provider>,
        system: impl Into<String>,
        tools: Vec<Tool>,
    ) -> Session {
        Session {
            provider,
            system: system.into(),
            tools,
            conversation: Conversation::default(),
            usage: Usage::default(),
        }
    }

    /// The helper's name, as the transcript shows it.
    pub fn helper(&self) -> &str {
        self.provider.name()
    }

    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    /// What every exchange of the session has cost so far.
    pub fn usage(&self) -> Usage {
        self.usage
    }

    /// Starts again with the same helper.
    pub fn clear(&mut self) {
        self.conversation.clear();
    }

    /// Asks the helper, runs the tools it asks for through `host`, and says how
    /// the request ended: [`Ending::Finished`], or the ending or failure that
    /// cut it short.
    pub fn ask(
        &mut self,
        words: &str,
        effort: Effort,
        stop: &StopFlag,
        host: &mut dyn Host,
    ) -> Result<Ending, Failure> {
        let before = self.conversation.len();
        self.conversation.push(Message::user(words));
        let ended = self.exchanges(effort, stop, host);
        if ended != Ok(Ending::Finished) {
            self.conversation.truncate(before);
        }
        ended
    }

    fn exchanges(
        &mut self,
        effort: Effort,
        stop: &StopFlag,
        host: &mut dyn Host,
    ) -> Result<Ending, Failure> {
        for _ in 0..MOST_STEPS {
            // Stop pressed while the last tool ran: the results are not sent.
            if stop.is_set() {
                return Ok(Ending::Stopped);
            }
            let request = Request {
                system: &self.system,
                tools: &self.tools,
                conversation: &self.conversation,
                effort,
            };
            let answer = stream(self.provider.as_mut(), &request, stop, &mut |event| {
                host.event(&event)
            });
            self.usage += answer.usage;
            match answer.ending? {
                Ending::WantsTools => {}
                // A helper may end its turn after the tools with nothing to
                // say, and a message with nothing in it is one the service
                // refuses on every later request. Left out, the next request's
                // words join the results' turn.
                Ending::Finished if says_nothing(&answer.message) => return Ok(Ending::Finished),
                other => {
                    self.conversation.push(answer.message);
                    return Ok(other);
                }
            }
            let calls: Vec<ToolCall> = answer.message.calls().cloned().collect();
            if calls.is_empty() {
                return Err(Failure::garbled(
                    self.provider.name(),
                    "it asked for a tool and named none",
                ));
            }
            self.conversation.push(answer.message);
            let mut results = Vec::with_capacity(calls.len());
            for call in &calls {
                if stop.is_set() {
                    return Ok(Ending::Stopped);
                }
                results.push(host.run(call));
            }
            self.conversation.push(Message::results(results));
        }
        Err(Failure::new(
            FailureKind::TooManySteps,
            format!(
                "{} kept asking for more steps than one request may take, and was stopped.",
                self.provider.name()
            ),
        ))
    }
}
