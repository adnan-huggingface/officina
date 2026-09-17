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
//! after it already handles — and the configuration and cache directories are
//! the process's own under the temporary directory. The driver enters it, and
//! so does each application's constructor when compiled for its own tests, so a
//! test written without the driver is held to the same rule.
//!
//! **Nor does a test reach an assistant.** Assist's helpers are services on
//! the network and programs on the computer: a test that asked Claude would
//! spend the developer's money and send the test's document away, and one that
//! asked their Ollama, or ran their `ant` for a login, would pass or fail by
//! what that machine happens to have. Headless, every helper but the scripted
//! one refuses before it opens a connection, nothing is looked for on the
//! computer, and each refusal is counted as a chooser is.

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
    assist::offline::enter();
}

pub fn active() -> bool {
    HEADLESS.load(Ordering::SeqCst)
}

/// How many operating-system choosers this thread has asked for and been
/// refused, so a test can say whether a command reached one.
pub fn choosers_refused() -> usize {
    CHOOSERS.with(Cell::get)
}

/// How many times this thread has reached for one of Assist's helpers — a
/// request, a login, a server on the computer — and been refused.
pub fn helpers_refused() -> usize {
    assist::offline::refused()
}

pub(crate) fn refuse_chooser() {
    CHOOSERS.with(|count| count.set(count.get() + 1));
}

/// The configuration directory a headless process uses in place of the
/// user's: its own, so two test binaries running at once keep apart.
pub(crate) fn config_base() -> PathBuf {
    std::env::temp_dir().join(format!("officina-headless-{}", std::process::id()))
}

/// The cache directory's counterpart of [`config_base`].
pub(crate) fn cache_base() -> PathBuf {
    config_base().join(".cache")
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use assist::{
        connect, ladder, Anthropic, Choice, ClaudeLogin, Compatible, Ending, Event, FailureKind,
        Host, Login, Provider, Row, Scripted, Session, Settings, StopFlag, ThisComputer, ToolCall,
        ToolResult, Turn,
    };

    struct Quiet;

    impl Host for Quiet {
        fn event(&mut self, _: &Event) {}

        fn run(&mut self, call: &ToolCall) -> ToolResult {
            ToolResult::error(call, "no tools here")
        }
    }

    fn ask(provider: Box<dyn Provider>) -> Result<Ending, assist::Failure> {
        let mut session = Session::new(provider, "You help.", Vec::new());
        session.ask(
            "Hello.",
            assist::Effort::Usual,
            &StopFlag::new(),
            &mut Quiet,
        )
    }

    #[test]
    fn a_headless_process_refuses_every_helper_but_the_scripted_one_and_counts_it() {
        super::enter();
        // Every helper is pointed here, and nothing may connect. A connection
        // is counted and dropped at once, so a helper that was not refused
        // fails quickly instead of waiting for an answer.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&connections);
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                counted.fetch_add(1, Ordering::SeqCst);
                drop(connection);
            }
        });

        let chosen = |choice: Option<Choice>, login: ClaudeLogin| {
            let mut settings = Settings {
                helper: choice,
                ..Settings::default()
            };
            settings.claude.login = login;
            settings.claude.key = "sk-ant-test".into();
            settings.claude.address = address.clone();
            settings.ollama.address = address.clone();
            settings.ollama.model = "qwen3".into();
            settings.service.address = address.clone();
            settings.service.model = "some-model".into();
            connect(&settings)
        };
        let helpers: Vec<Box<dyn Provider>> = vec![
            Box::new(Anthropic::new(
                &address,
                Box::new(|| Ok(Login::Key("sk-ant-test".into()))),
                "claude-opus-5",
            )),
            Box::new(Compatible::new(
                "Ollama",
                &format!("{address}/v1"),
                None,
                "qwen3",
            )),
            chosen(None, ClaudeLogin::Key),
            chosen(Some(Choice::Local), ClaudeLogin::Key),
            chosen(Some(Choice::Claude), ClaudeLogin::Key),
            chosen(Some(Choice::Claude), ClaudeLogin::Environment),
            // Refused before `ant` is run for its token.
            chosen(Some(Choice::Claude), ClaudeLogin::Ant),
            chosen(Some(Choice::Ollama), ClaudeLogin::Key),
            chosen(Some(Choice::Service), ClaudeLogin::Key),
        ];
        let before = super::helpers_refused();
        let count = helpers.len();
        for helper in helpers {
            let name = helper.name().to_owned();
            let failure = ask(helper).expect_err("a helper answered a test");
            assert_eq!(
                failure.kind,
                FailureKind::Offline,
                "{name}: {}",
                failure.sentence
            );
        }
        assert_eq!(super::helpers_refused() - before, count);

        // The computer is not searched: not the environment, not `ant`, and
        // not an Ollama that may well be running on it.
        assert_eq!(
            ladder(&ThisComputer),
            [Row::Local, Row::ClaudeWithKey, Row::Service]
        );
        // Two variables, `ant`, and Ollama.
        assert_eq!(super::helpers_refused() - before, count + 4);
        assert_eq!(
            connections.load(Ordering::SeqCst),
            0,
            "no helper opened a connection"
        );

        // The scripted helper plays, and is not counted.
        let scripted = Box::new(Scripted::new([Turn::says("Hello to you.")]));
        assert_eq!(ask(scripted), Ok(Ending::Finished));
        assert_eq!(super::helpers_refused() - before, count + 4);
    }
}
