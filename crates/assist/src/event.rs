//! What a helper says while it answers, and how an answer ends.
//!
//! **Every request ends, and ends in words a person can act on.** A helper's
//! answer is text as it arrives, the tools it wants run, and exactly one ending:
//! finished, wanting tools, cut off, declined, stopped — or a [`Failure`], which
//! carries the sentence the transcript shows. A rate limit says how long to
//! wait; a helper that is not there is named with its address; a connection
//! that dropped says the answer was cut off. None of them is a dialog, and none
//! of them is a wait with no end.

use std::time::Duration;

use crate::conversation::ToolCall;

/// One thing a helper did while answering, in the order it did it.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Words of the answer, as they arrive.
    Text(String),
    /// A tool the helper wants run. Handed over only once the helper's message
    /// has ended: a call that was still being written when a model declined is
    /// not one to run.
    ToolCall(ToolCall),
    /// The helper's message ended this way.
    Done(Ending),
    /// The request failed, for the reason the sentence gives.
    Failed(Failure),
}

/// How a helper's message ended, when it ended at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// The helper has said all it means to.
    Finished,
    /// The helper wants the tools it named run before it goes on.
    WantsTools,
    /// The answer reached the length a helper may write at once.
    TooLong,
    /// The helper declined the request. `category` is the service's own word for
    /// why, when it gives one; it is for a log, not for the transcript.
    Declined { category: Option<String> },
    /// Stop was pressed, and the answer was let go.
    Stopped,
}

impl Ending {
    /// What the transcript says about this ending, if anything: a finished
    /// answer speaks for itself.
    pub fn sentence(&self, helper: &str) -> Option<String> {
        match self {
            Ending::Finished | Ending::WantsTools => None,
            Ending::TooLong => Some(format!(
                "The answer was longer than {helper} may write at once, and was cut off."
            )),
            Ending::Declined { .. } => Some(format!("{helper} declined this request.")),
            Ending::Stopped => Some("Stopped.".to_owned()),
        }
    }
}

/// A request that did not get an answer, and the sentence that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub sentence: String,
}

/// What went wrong, for the code that decides what to offer next; the person
/// reads [`Failure::sentence`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    /// The process is under a test, where no helper but a script may answer.
    Offline,
    /// No helper has been chosen, or the one chosen is not there yet.
    NotReady,
    /// Nothing answered at the helper's address.
    Unreachable,
    /// The helper did not accept the key or the login.
    Unauthorized,
    /// The helper asked for time before the next request.
    RateLimited { retry_after: Option<Duration> },
    /// The helper is busy or had a fault of its own; trying again may work.
    Busy,
    /// The helper refused the request as it was put. Not the user's doing, as a
    /// rule, and not something a retry changes.
    Rejected { status: u16 },
    /// The connection went while the answer was arriving.
    Dropped,
    /// The answer could not be read.
    Garbled,
    /// The helper kept asking for tools past the limit of one request.
    TooManySteps,
}

impl Failure {
    pub fn new(kind: FailureKind, sentence: impl Into<String>) -> Failure {
        Failure {
            kind,
            sentence: sentence.into(),
        }
    }

    pub(crate) fn dropped(helper: &str) -> Failure {
        Failure::new(
            FailureKind::Dropped,
            format!("The connection to {helper} dropped, and the answer was cut off."),
        )
    }

    pub(crate) fn garbled(helper: &str, what: &str) -> Failure {
        Failure::new(
            FailureKind::Garbled,
            format!("{helper}'s answer could not be read ({what})."),
        )
    }
}

/// What an answer cost, in the units the service counts. Services that do not
/// say leave it at zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    /// Input read back from the service's cache, at a tenth of the price.
    pub cache_read: u64,
    /// Input written to the service's cache, at a quarter over the price.
    pub cache_write: u64,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, other: Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}
