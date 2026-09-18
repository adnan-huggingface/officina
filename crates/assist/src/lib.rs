//! Assist's helpers: who answers a request, how it travels, and what a helper may ask the editor to do.
//!
//! **The helper proposes; the editor disposes.** Nothing here knows a document
//! or a window. A helper is given a conversation and a list of tools, and
//! answers with words and with requests to run those tools; the application
//! runs them against its own document, on its own thread, through the same
//! functions the keyboard uses, and hands back what came of each. A helper
//! never sees a file, never runs anything, and never saves.
//!
//! Four helpers answer through one [`Provider`] trait: Claude, over Anthropic's
//! Messages API ([`anthropic`]); Ollama or any other service that speaks the
//! chat-completions shape ([`compatible`]); the helper on the computer itself,
//! which is not built yet; and [`Scripted`], a list of canned turns that is the
//! only helper a test may use. A process that is under a test says so through
//! [`offline`], and from then on every other helper refuses before it opens a
//! connection.
//!
//! What the user chose, and what this computer already has that could answer,
//! are [`settings`] and [`machine`]: the first-run card's rows come from the
//! latter in the order [`ladder`] gives them.

#![forbid(unsafe_code)]

pub mod anthropic;
pub mod compatible;
mod conversation;
mod event;
mod http;
pub mod local;
pub mod machine;
pub mod models;
pub mod offline;
pub mod prompt;
mod provider;
mod scripted;
mod session;
pub mod settings;
mod sse;
mod tool;

#[cfg(test)]
mod tests;

pub use anthropic::{Anthropic, Login};
pub use compatible::Compatible;
pub use conversation::{Block, Conversation, Message, Role, ToolCall, ToolResult};
pub use event::{Ending, Event, Failure, FailureKind, Usage};
pub use machine::{ladder, size_words, Installed, Machine, Row, ThisComputer};
pub use models::{ClaudeModel, CLAUDE_MODELS, DEFAULT_CLAUDE_MODEL};
pub use provider::{
    check, check_in, connect, connect_in, destination, stream, Answer, Effort, Provider, Request,
    StopFlag, LOCAL_NOT_READY, LOCAL_READY,
};
pub use scripted::{Heard, Scripted, Turn};
pub use session::{Host, Session, MOST_STEPS};
pub use settings::{Choice, ClaudeLogin, Settings};
pub use tool::Tool;
