//! A conversation with a helper: the user's words, the helper's answers, and
//! the results of the tools it asked for.
//!
//! **Only ever appended to, or cut back to where a request began.** Claude signs
//! its thinking, and a conversation whose earlier turns were edited carries
//! thinking the service will not accept; so a turn is never rewritten. A request
//! that does not finish is taken back whole, as if it had not been asked.

use serde_json::Value;

/// A tool the helper wants run: which, with what, and the id its result must
/// carry back.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// What came of running a tool, in words for the helper to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub id: String,
    pub content: String,
    /// The tool could not do what was asked; `content` says why, so that the
    /// helper can try something else.
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(call: &ToolCall, content: impl Into<String>) -> ToolResult {
        ToolResult {
            id: call.id.clone(),
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(call: &ToolCall, content: impl Into<String>) -> ToolResult {
        ToolResult {
            id: call.id.clone(),
            content: content.into(),
            is_error: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    /// A block of one service's own that means nothing to Assist and must go
    /// back to that service exactly as it came — Claude's thinking, whose
    /// signature the service checks. Any other service is never sent it.
    Opaque {
        service: &'static str,
        block: Value,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user(words: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            content: vec![Block::Text(words.into())],
        }
    }

    pub fn assistant(content: Vec<Block>) -> Message {
        Message {
            role: Role::Assistant,
            content,
        }
    }

    /// The results of one message's tool calls, all in one turn: a helper that
    /// sees them split across turns learns to ask for one tool at a time.
    pub fn results(results: Vec<ToolResult>) -> Message {
        Message {
            role: Role::User,
            content: results.into_iter().map(Block::ToolResult).collect(),
        }
    }

    /// The tools this message asks for, in the order it asked.
    pub fn calls(&self) -> impl Iterator<Item = &ToolCall> {
        self.content.iter().filter_map(|block| match block {
            Block::ToolCall(call) => Some(call),
            _ => None,
        })
    }

    /// The message's words, its text blocks run together.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                Block::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conversation {
    messages: Vec<Message>,
}

impl Conversation {
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Forgets everything: a new conversation, as Clear in the pane gives.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Adds a turn. Public so that a hand-run measurement outside the crate
    /// — `cargo xtask assist-spike` — can ask a helper something without a
    /// session around it.
    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Cuts the conversation back to its first `len` messages — only ever to
    /// where a request began.
    pub(crate) fn truncate(&mut self, len: usize) {
        self.messages.truncate(len);
    }
}
