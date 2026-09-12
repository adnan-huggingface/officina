//! The operating system's file chooser, asked without stopping the window.
//!
//! **On Linux the chooser is another program's window.** rfd asks the desktop
//! portal over D-Bus and waits for the answer with no time limit, and a call
//! made on the thread that paints the window stops the window with it: no
//! frame is drawn and no key is read until the portal answers. With a portal
//! that answers, that is a window a desktop reports as not responding for as
//! long as the chooser is open. With one that never does — none installed, one
//! that crashed, one drawing on another display — it is a window that can only
//! be killed, with whatever was unsaved in it. So on Linux the question is put
//! on a thread of its own and the window goes on painting, with a box saying
//! what it is waiting for and a way to stop waiting.
//!
//! Everywhere else the chooser is the operating system's own modal dialog,
//! whose message loop keeps the window it belongs to responding while it is
//! open, and it is asked exactly as it always was: the answer is simply picked
//! up a frame later.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui;

use crate::dialog;

/// A chooser that has been asked and has not yet answered, and what to do
/// with the answer once it has.
pub struct Asking<T> {
    answer: mpsc::Receiver<Option<PathBuf>>,
    then: T,
}

impl<T> Asking<T> {
    /// Puts the question. `chooser` is the whole of it — an `rfd::FileDialog`
    /// already configured and then asked — so that this module needs to know
    /// nothing about filters or file names.
    pub fn new(chooser: impl FnOnce() -> Option<PathBuf> + Send + 'static, then: T) -> Asking<T> {
        let (tell, answer) = mpsc::channel();
        if cfg!(target_os = "linux") {
            // A chooser that never answers leaves this thread parked for good.
            // That is the whole point: better a thread than the window.
            std::thread::spawn(move || {
                let _ = tell.send(chooser());
            });
        } else {
            let _ = tell.send(chooser());
        }
        Asking { answer, then }
    }

    /// The answer, once there is one: a path chosen, or `None` for a chooser
    /// cancelled. `Err(self)` while it is still open.
    pub fn answered(self) -> Result<(Option<PathBuf>, T), Asking<T>> {
        match self.answer.try_recv() {
            Ok(path) => Ok((path, self.then)),
            Err(mpsc::TryRecvError::Empty) => Err(self),
            // The thread went without answering, which rfd does not do; as far
            // as anyone waiting is concerned, nothing was chosen.
            Err(mpsc::TryRecvError::Disconnected) => Ok((None, self.then)),
        }
    }
}

/// The box shown while a chooser is open, and whether the user has given up
/// on it.
///
/// Stopping does not close the chooser — nothing here can — but it forgets
/// whatever it answers, and gives the window back.
pub fn waiting(ctx: &egui::Context) -> bool {
    // Nothing else will wake the window when the answer comes: it arrives on a
    // channel, not as an event.
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    dialog::message(
        ctx,
        "ui-kit-chooser",
        dialog::Severity::Info,
        "Waiting for the file chooser",
        "The file chooser is a window of its own. Choose a file there, or \
         cancel it, to carry on. If it never appeared, stop waiting: nothing \
         has been saved or opened, and nothing is lost.",
        None,
        &[dialog::Choice::new("Stop Waiting").escapes()],
    )
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_path_is_handed_over_with_what_it_was_for() {
        let mut asking = Asking::new(|| Some(PathBuf::from("/tmp/chosen.docx")), "save");
        let (path, then) = loop {
            match asking.answered() {
                Ok(answer) => break answer,
                Err(still) => asking = still,
            }
        };
        assert_eq!(path, Some(PathBuf::from("/tmp/chosen.docx")));
        assert_eq!(then, "save");
    }

    /// The fault this module exists for: a chooser that never answers. On
    /// Linux the question is somebody else's, and the caller goes on.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_chooser_that_never_answers_does_not_hold_the_caller() {
        let (_never, parked) = mpsc::channel::<()>();
        let asking = Asking::new(
            move || {
                let _ = parked.recv();
                None
            },
            (),
        );
        assert!(asking.answered().is_err(), "still waiting, and not blocked");
    }
}
