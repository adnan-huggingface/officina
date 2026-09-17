//! What this computer already has that could answer, and the first-run card's
//! rows in the order they are offered.
//!
//! **Notice, offer, never assume.** A person who has `ANTHROPIC_API_KEY` set,
//! or has logged in with Anthropic's `ant` command, or runs Ollama, has already
//! chosen once; the card offers what it found first, in words, and preselects
//! it. When nothing is found the helper on this computer is first: it needs no
//! account, no card and no network after its download. Claude with a key and
//! another service are always offered.
//!
//! Claude Code's own login and a Claude.ai session are not looked at: they
//! belong to Anthropic's products, and a Claude.ai subscription is not an API
//! account.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::http::Http;
use crate::settings::{Choice, ClaudeLogin, Settings};

/// Where Ollama listens unless told otherwise.
pub const OLLAMA: &str = "http://127.0.0.1:11434";

/// How long `ant` may take to print a token. It renews an expired one first,
/// which is a request of its own.
const ANT_WAIT: Duration = Duration::from_secs(10);

/// What the computer is asked.
pub trait Machine {
    /// An environment variable, when it is set to something.
    fn var(&self, name: &str) -> Option<String>;
    /// A token from the `ant` command's login, when it is installed and
    /// logged in.
    fn ant_token(&self) -> Option<String>;
    /// The models an Ollama server on this computer has, when one answers.
    fn ollama_models(&self) -> Option<Vec<String>>;
}

/// The computer the application runs on. Under a test it has nothing: no
/// variable is read, no command is run and no server is asked, and each look
/// not taken is counted.
pub struct ThisComputer;

impl Machine for ThisComputer {
    fn var(&self, name: &str) -> Option<String> {
        if crate::offline::active() {
            crate::offline::count();
            return None;
        }
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    fn ant_token(&self) -> Option<String> {
        if crate::offline::active() {
            crate::offline::count();
            return None;
        }
        // Another program called `ant` — Apache's build tool is one — fails
        // these arguments and says nothing that passes for a token.
        let mut command = Command::new("ant");
        command.args(["auth", "print-credentials", "--access-token"]);
        let token = first_line_within(command, ANT_WAIT)?;
        let plausible = !token.is_empty() && !token.contains(char::is_whitespace);
        plausible.then_some(token)
    }

    fn ollama_models(&self) -> Option<Vec<String>> {
        if crate::offline::active() {
            crate::offline::count();
            return None;
        }
        let response = Http::quick(OLLAMA)
            .get(&format!("{OLLAMA}/api/tags"), "Ollama")
            .ok()?;
        if response.status != 200 {
            return None;
        }
        let tags: Value = serde_json::from_reader(response.body.take(1024 * 1024)).ok()?;
        Some(
            tags["models"]
                .as_array()?
                .iter()
                .filter_map(|model| model["name"].as_str().map(str::to_owned))
                .collect(),
        )
    }
}

/// The first line a command prints, trimmed, if it prints one and succeeds
/// within `wait`. The command is killed when the time is up, and a process it
/// left behind holding its output open does not hold this up: the line is
/// waited for on a thread of its own, for no longer than the command is.
fn first_line_within(mut command: Command, wait: Duration) -> Option<String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        // A window application that runs a console program otherwise
        // flashes a console window.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let deadline = Instant::now() + wait;
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let (tell, told) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = BufReader::new(stdout).read_line(&mut line);
        let _ = tell.send(read.ok().map(|_| line));
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let left = deadline.saturating_duration_since(Instant::now());
    let line = told.recv_timeout(left).ok().flatten()?;
    status.success().then(|| line.trim().to_owned())
}

/// A row of the first-run card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Claude, with the credential this computer already has.
    ClaudeHere(ClaudeLogin),
    /// The Ollama server on this computer, with the first model it has.
    OllamaHere { model: String },
    /// The helper on this computer, downloaded once.
    Local,
    /// Claude, with a key the person pastes.
    ClaudeWithKey,
    /// Another service, by address and key.
    Service,
}

impl Row {
    /// The row's title on the card. The words "model", "LLM", "token",
    /// "endpoint" and "GPU" are not among them.
    pub fn title(&self) -> String {
        match self {
            Row::ClaudeHere(ClaudeLogin::Ant) => "Claude, with the login on this computer".into(),
            Row::ClaudeHere(_) => "Claude, with the key already on this computer".into(),
            Row::OllamaHere { model } => format!("Ollama on this computer ({model})"),
            Row::Local => "A helper on this computer".into(),
            Row::ClaudeWithKey => "Claude, over the internet".into(),
            Row::Service => "Another service (advanced)".into(),
        }
    }

    /// The settings once this row is chosen.
    pub fn choose(&self, settings: &mut Settings) {
        match self {
            Row::ClaudeHere(login) => {
                settings.helper = Some(Choice::Claude);
                settings.claude.login = *login;
            }
            Row::OllamaHere { model } => {
                settings.helper = Some(Choice::Ollama);
                settings.ollama.model = model.clone();
            }
            Row::Local => settings.helper = Some(Choice::Local),
            Row::ClaudeWithKey => {
                settings.helper = Some(Choice::Claude);
                settings.claude.login = ClaudeLogin::Key;
            }
            Row::Service => settings.helper = Some(Choice::Service),
        }
    }
}

/// The card's rows, first the preselected one.
pub fn ladder(machine: &dyn Machine) -> Vec<Row> {
    let mut rows = Vec::new();
    // A key in the environment is what the Anthropic SDKs would use before a
    // login, and asking for it costs nothing; `ant` is only run without one.
    let in_environment = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]
        .iter()
        .any(|name| machine.var(name).is_some());
    if in_environment {
        rows.push(Row::ClaudeHere(ClaudeLogin::Environment));
    } else if machine.ant_token().is_some() {
        rows.push(Row::ClaudeHere(ClaudeLogin::Ant));
    }
    // A server with nothing pulled into it has nothing to answer with.
    if let Some(model) = machine
        .ollama_models()
        .and_then(|models| models.into_iter().next())
    {
        rows.push(Row::OllamaHere { model });
    }
    rows.extend([Row::Local, Row::ClaudeWithKey, Row::Service]);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct Fake {
        vars: Vec<(&'static str, &'static str)>,
        ant: Option<&'static str>,
        ollama: Option<Vec<&'static str>>,
        ant_asked: Cell<bool>,
    }

    impl Machine for Fake {
        fn var(&self, name: &str) -> Option<String> {
            self.vars
                .iter()
                .find(|(key, value)| *key == name && !value.is_empty())
                .map(|(_, value)| value.to_string())
        }

        fn ant_token(&self) -> Option<String> {
            self.ant_asked.set(true);
            self.ant.map(str::to_owned)
        }

        fn ollama_models(&self) -> Option<Vec<String>> {
            self.ollama
                .as_ref()
                .map(|models| models.iter().map(|model| model.to_string()).collect())
        }
    }

    /// `ant` is run with a deadline that holds even when something it started
    /// keeps its output open, and a command that says nothing, or fails, gives
    /// no token.
    #[cfg(unix)]
    #[test]
    fn a_command_asked_for_a_token_is_waited_on_no_longer_than_its_deadline() {
        let shell = |script: &str| {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            command
        };
        let wait = Duration::from_millis(700);
        let started = Instant::now();
        assert_eq!(
            first_line_within(shell("(sleep 5) & echo token-123"), wait).as_deref(),
            Some("token-123"),
            "a line printed before a lingering child is enough"
        );
        assert_eq!(first_line_within(shell("(sleep 5) &"), wait), None);
        assert_eq!(first_line_within(shell("sleep 5"), wait), None);
        assert_eq!(first_line_within(shell("echo token; exit 1"), wait), None);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "each gave up by its deadline, not the children's ({:?})",
            started.elapsed()
        );
        assert_eq!(
            first_line_within(Command::new("no-such-command-anywhere"), wait),
            None
        );
    }

    #[test]
    fn the_ladder_offers_what_the_machine_has_in_the_order_the_card_shows() {
        let always = [Row::Local, Row::ClaudeWithKey, Row::Service];
        assert_eq!(
            ladder(&Fake::default()),
            always,
            "nothing found: the helper here first"
        );

        let empty_key = Fake {
            vars: vec![("ANTHROPIC_API_KEY", "")],
            ..Fake::default()
        };
        assert_eq!(
            ladder(&empty_key),
            always,
            "a variable set to nothing is not a key"
        );

        let key = Fake {
            vars: vec![("ANTHROPIC_API_KEY", "sk-ant")],
            ant: Some("token"),
            ..Fake::default()
        };
        let rows = ladder(&key);
        assert_eq!(rows[0], Row::ClaudeHere(ClaudeLogin::Environment));
        assert_eq!(rows[1..], always);
        assert!(
            !key.ant_asked.get(),
            "ant is not run when the environment has a key"
        );

        let token = Fake {
            vars: vec![("ANTHROPIC_AUTH_TOKEN", "tok")],
            ..Fake::default()
        };
        assert_eq!(ladder(&token)[0], Row::ClaudeHere(ClaudeLogin::Environment));

        let ant = Fake {
            ant: Some("token"),
            ..Fake::default()
        };
        assert_eq!(ladder(&ant)[0], Row::ClaudeHere(ClaudeLogin::Ant));

        let ollama = Fake {
            ollama: Some(vec!["qwen3:1.7b", "llama3.2:latest"]),
            ..Fake::default()
        };
        let rows = ladder(&ollama);
        assert_eq!(
            rows[0],
            Row::OllamaHere {
                model: "qwen3:1.7b".into()
            }
        );
        assert_eq!(rows[1..], always);

        let empty_ollama = Fake {
            ollama: Some(vec![]),
            ..Fake::default()
        };
        assert_eq!(
            ladder(&empty_ollama),
            always,
            "an Ollama with no models has nothing to offer"
        );

        let both = Fake {
            ant: Some("token"),
            ollama: Some(vec!["qwen3:1.7b"]),
            ..Fake::default()
        };
        let rows = ladder(&both);
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows[0],
            Row::ClaudeHere(ClaudeLogin::Ant),
            "a Claude login before Ollama"
        );
        assert!(matches!(rows[1], Row::OllamaHere { .. }));
        assert_eq!(rows[2..], always);

        // Choosing a row is choosing its helper, the way it was found.
        let mut settings = Settings::default();
        rows[0].choose(&mut settings);
        assert_eq!(settings.helper, Some(Choice::Claude));
        assert_eq!(settings.claude.login, ClaudeLogin::Ant);
        rows[1].choose(&mut settings);
        assert_eq!(settings.helper, Some(Choice::Ollama));
        assert_eq!(settings.ollama.model, "qwen3:1.7b");

        for row in rows {
            let title = row.title().to_lowercase();
            for word in ["model", "llm", "token", "endpoint", "gpu"] {
                assert!(!title.contains(word), "“{title}” says {word}");
            }
        }
    }
}
