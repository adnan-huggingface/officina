//! An application running without a person in front of it: under a test.
//!
//! **A test must never reach the desktop.** The in-process driver can run any
//! command an application has, and two of them go outside the process: the
//! file chooser, which on Linux is the desktop portal's window on whatever
//! display the session has, and the configuration directory, where every save
//! and every open is written into the recent list. The day the driver was
//! written a test that meant Insert ▸ Page Number reached Insert ▸ Picture…
//! instead and put three file choosers on the developer's own screen, and a
//! test that opened the corpus replaced their recent list with it.
//!
//! [`enter`] makes the process headless for good: an operating-system chooser
//! is not asked but counted, and answers "cancelled" — the answer the code
//! after it already handles — and the configuration directory is one of the
//! process's own under the temporary directory. The driver enters it, and so
//! does each application's constructor when compiled for its own tests, so a
//! test written without the driver is held to the same rule.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static HEADLESS: AtomicBool = AtomicBool::new(false);

thread_local! {
    // Per thread, because tests run side by side on threads of their own and
    // a count one test could see another's refusals in says nothing.
    static CHOOSERS: Cell<usize> = const { Cell::new(0) };
}

/// From here on, nothing this process does reaches the desktop.
pub fn enter() {
    HEADLESS.store(true, Ordering::SeqCst);
}

pub fn active() -> bool {
    HEADLESS.load(Ordering::SeqCst)
}

/// How many operating-system choosers this thread has asked for and been
/// refused, so a test can say whether a command reached one.
pub fn choosers_refused() -> usize {
    CHOOSERS.with(Cell::get)
}

pub(crate) fn refuse_chooser() {
    CHOOSERS.with(|count| count.set(count.get() + 1));
}

/// The configuration directory a headless process uses in place of the
/// user's: its own, so two test binaries running at once keep apart.
pub(crate) fn config_base() -> PathBuf {
    std::env::temp_dir().join(format!("officina-headless-{}", std::process::id()))
}
