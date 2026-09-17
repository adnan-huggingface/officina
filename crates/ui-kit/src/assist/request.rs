//! A request on a thread of its own, and anything else the pane waits for
//! without stopping the window.
//!
//! **The document never leaves the window's thread.** The helper runs here,
//! and the window goes on painting. When the helper wants a tool, the call is
//! sent to the window and this thread waits for the result; the window runs
//! the tool on a frame of its own, against its own document, and sends the
//! result back. A window that lets go of the request drops its end of the
//! reply, and the tool is answered as stopped at once.

use std::sync::mpsc;

use ::assist::{
    Effort, Ending, Event, Failure, FailureKind, Host, Session, StopFlag, ToolCall, ToolResult,
    Usage,
};

/// What a request's thread tells the window.
pub(crate) enum Message {
    /// Something the helper said or did.
    Event(Event),
    /// A tool the helper wants run, and where its result goes.
    Run(ToolCall, mpsc::Sender<ToolResult>),
    /// The request is over: how it ended, what it cost, and the session, whose
    /// conversation has what the request added when it finished and nothing of
    /// it when it did not.
    Ended {
        ended: Result<Ending, Failure>,
        spent: Usage,
        session: Box<Session>,
    },
}

/// A request under way, as the window holds it.
pub(crate) struct Request {
    pub from: mpsc::Receiver<Message>,
    pub stop: StopFlag,
}

/// Asks `session` on a thread of its own. `wake` is called whenever the
/// thread has something for the window, so that a window with nothing else
/// to do paints it at once.
pub(crate) fn start(
    mut session: Session,
    words: String,
    effort: Effort,
    wake: impl Fn() + Send + 'static,
) -> Request {
    let (tell, from) = mpsc::channel();
    let stop = StopFlag::new();
    let flag = stop.clone();
    std::thread::spawn(move || {
        let before = session.usage();
        let ended = {
            let mut host = Channel {
                tell: tell.clone(),
                wake: &wake,
            };
            session.ask(&words, effort, &flag, &mut host)
        };
        let spent = less(session.usage(), before);
        // A window that has let go has dropped its end; nobody is told.
        let _ = tell.send(Message::Ended {
            ended,
            spent,
            session: Box::new(session),
        });
        wake();
    });
    Request { from, stop }
}

/// What a request cost: its session's count at the end, less the count at the
/// start.
fn less(after: Usage, before: Usage) -> Usage {
    Usage {
        input: after.input.saturating_sub(before.input),
        output: after.output.saturating_sub(before.output),
        cache_read: after.cache_read.saturating_sub(before.cache_read),
        cache_write: after.cache_write.saturating_sub(before.cache_write),
    }
}

/// The session's host on the request's thread: everything goes to the window.
struct Channel<'a> {
    tell: mpsc::Sender<Message>,
    wake: &'a (dyn Fn() + Send),
}

impl Host for Channel<'_> {
    fn event(&mut self, event: &Event) {
        let _ = self.tell.send(Message::Event(event.clone()));
        (self.wake)();
    }

    fn run(&mut self, call: &ToolCall) -> ToolResult {
        let (answer, answered) = mpsc::channel();
        if self.tell.send(Message::Run(call.clone(), answer)).is_err() {
            return stopped(call);
        }
        (self.wake)();
        // Waits for the window's next frame, or for the window to let go.
        answered.recv().unwrap_or_else(|_| stopped(call))
    }
}

fn stopped(call: &ToolCall) -> ToolResult {
    ToolResult::error(
        call,
        "The person stopped the request before this could run.",
    )
}

/// A thread's answer, while it is awaited.
pub(crate) struct Background<T> {
    answer: mpsc::Receiver<T>,
}

/// Where a [`Background`] stands.
pub(crate) enum Awaited<T> {
    Waiting,
    Done(T),
    /// The thread ended without answering, which only a panic does.
    Gone,
}

impl<T: Send + 'static> Background<T> {
    pub fn spawn(
        work: impl FnOnce() -> T + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let (tell, answer) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tell.send(work());
            wake();
        });
        Background { answer }
    }

    pub fn poll(&self) -> Awaited<T> {
        match self.answer.try_recv() {
            Ok(answer) => Awaited::Done(answer),
            Err(mpsc::TryRecvError::Empty) => Awaited::Waiting,
            Err(mpsc::TryRecvError::Disconnected) => Awaited::Gone,
        }
    }
}

/// A look or a check nobody waits for any more, whose thread still has
/// refusals to report.
pub(crate) trait Drain {
    /// How many times the thread was refused, once it has finished.
    fn drained(&mut self) -> Option<usize>;
}

impl<T: Send + 'static> Drain for Background<(T, usize)> {
    fn drained(&mut self) -> Option<usize> {
        match self.poll() {
            Awaited::Waiting => None,
            Awaited::Done((_, refused)) => Some(refused),
            Awaited::Gone => Some(0),
        }
    }
}

/// Runs `work` on the calling thread's behalf and says how many times it was
/// refused for being under a test: the count is kept on the thread that was
/// refused, and the window counts it again on its own.
pub(crate) fn counting<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let before = ::assist::offline::refused();
    let answer = work();
    (answer, ::assist::offline::refused() - before)
}

/// Counts `times` refusals on this thread.
pub(crate) fn count_again(times: usize) {
    for _ in 0..times {
        ::assist::offline::count();
    }
}

/// The failure a request's thread leaves behind when it ends without a word.
pub(crate) fn vanished() -> Failure {
    Failure::new(
        FailureKind::Dropped,
        "The request ended without an answer. Try again.",
    )
}
