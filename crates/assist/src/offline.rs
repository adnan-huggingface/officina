//! A process that must reach nothing: one under a test.
//!
//! **A test never reaches the network.** A test that asked Claude would spend
//! the developer's money and send whatever the test document says to a service,
//! and one that asked the developer's own Ollama, or ran their `ant` to read a
//! login, would pass or fail by what that machine happens to have. So once a
//! process has [`enter`]ed, every helper but the scripted one refuses before it
//! opens a connection, the computer is not searched for a login or a server,
//! and each refusal is counted, so that a test can say it reached for none.
//!
//! `ui_kit::headless::enter` enters it, alongside the refusal of file
//! choosers. This crate's own tests never do: their helpers talk to listeners
//! the tests open themselves, and the switch is for the whole process.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::event::{Failure, FailureKind};

static OFFLINE: AtomicBool = AtomicBool::new(false);

thread_local! {
    // Per thread, as the chooser count is: tests run side by side, and a count
    // that one test could see another's refusals in says nothing.
    static REFUSED: Cell<usize> = const { Cell::new(0) };
}

/// From here on, nothing this process asks of a helper leaves it.
pub fn enter() {
    OFFLINE.store(true, Ordering::SeqCst);
}

pub fn active() -> bool {
    OFFLINE.load(Ordering::SeqCst)
}

/// How many times this thread has reached for a helper, or looked for one on
/// the computer, and been refused.
///
/// The count is kept where the refusal happened. A request the window runs on
/// a thread of its own is refused on that thread, so the window counts the
/// refusal again, with [`count`], when the failure reaches it; a test that
/// reads this on its own thread then sees it.
pub fn refused() -> usize {
    REFUSED.with(Cell::get)
}

/// Whether `helper` may be asked. Under a test it may not, and the refusal is
/// counted and handed back as the failure the request ends in.
pub(crate) fn check(helper: &str) -> Result<(), Failure> {
    if !active() {
        return Ok(());
    }
    count();
    Err(Failure::new(
        FailureKind::Offline,
        format!("{helper} is not asked from a test."),
    ))
}

/// Counts a refusal on this thread: a look at the computer that was not
/// taken, or a refused request whose failure has reached this thread from
/// another.
pub fn count() {
    REFUSED.with(|count| count.set(count.get() + 1));
}
