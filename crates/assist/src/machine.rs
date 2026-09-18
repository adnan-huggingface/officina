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

use crate::event::Failure;
use crate::http::Http;
use crate::settings::{Choice, ClaudeLogin, Settings};

/// Where Ollama listens unless told otherwise.
pub const OLLAMA: &str = "http://127.0.0.1:11434";

/// How long `ant` may take to print a token. It renews an expired one first,
/// which is a request of its own.
const ANT_WAIT: Duration = Duration::from_secs(10);

/// Past this size a model is large. A model needs about as much free memory
/// as it takes on the disk, and a laptop with sixteen gigabytes has about half
/// of them free beside the programs already running; past that a model runs
/// slowly, or not at all, unless the computer is a powerful one.
pub const LARGE: u64 = 8_000_000_000;

/// A model a server on this computer has pulled, and its size on the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub name: String,
    /// `None` when the server does not say.
    pub bytes: Option<u64>,
    /// Where the server passes what the model is asked on to, when that is
    /// not this computer: "ollama.com" for one of Ollama's cloud models.
    pub elsewhere: Option<String>,
}

impl Installed {
    /// Whether what the model is asked stays on this computer. Ollama lists
    /// its cloud models, and any model made with a remote host, beside the
    /// ones it runs itself, and passes their requests on.
    pub fn runs_here(&self) -> bool {
        self.elsewhere.is_none()
    }

    pub fn is_large(&self) -> bool {
        self.bytes.is_some_and(|bytes| bytes > LARGE)
    }

    /// Its size as the card says it, when the server said.
    pub fn size(&self) -> Option<String> {
        self.bytes.map(size_words)
    }
}

/// A size in bytes as a person reads one, in the decimal units a disk and
/// Ollama's own list use: "850 MB", "1.4 GB", "19 GB". A size that would round
/// up to the next unit is said in it: "1.0 GB", never "1000 MB".
pub fn size_words(bytes: u64) -> String {
    let b = bytes as f64;
    let (kb, mb, gb) = (1e3, 1e6, 1e9);
    if b >= 9.95 * gb {
        format!("{:.0} GB", b / gb)
    } else if b >= 0.9995 * gb {
        format!("{:.1} GB", b / gb)
    } else if b >= 0.9995 * mb {
        format!("{:.0} MB", b / mb)
    } else if b >= 0.9995 * kb {
        format!("{:.0} KB", b / kb)
    } else {
        format!("{bytes} bytes")
    }
}

/// What the computer is asked.
pub trait Machine {
    /// An environment variable, when it is set to something.
    fn var(&self, name: &str) -> Option<String>;
    /// A token from the `ant` command's login, when it is installed and
    /// logged in.
    fn ant_token(&self) -> Option<String>;
    /// The models an Ollama server on this computer has, when one answers,
    /// most recently pulled first.
    fn ollama_models(&self) -> Option<Vec<Installed>>;
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

    fn ollama_models(&self) -> Option<Vec<Installed>> {
        if crate::offline::active() {
            crate::offline::count();
            return None;
        }
        ollama_models_at(OLLAMA)
    }
}

/// What the Ollama server at `address` has pulled, as its own list gives it:
/// `GET /api/tags`, whose models carry their size in bytes.
pub(crate) fn ollama_models_at(address: &str) -> Option<Vec<Installed>> {
    ollama_list(address).ok()
}

/// The same list, or why it could not be had.
fn ollama_list(address: &str) -> Result<Vec<Installed>, Failure> {
    let response = Http::quick(address).get(&format!("{address}/api/tags"), &[], "Ollama")?;
    if response.status != 200 {
        return Err(response.failure("Ollama"));
    }
    let tags: Value = serde_json::from_reader(response.body.take(1024 * 1024))
        .map_err(|_| Failure::garbled("Ollama", "its list of models"))?;
    let models = tags["models"]
        .as_array()
        .ok_or_else(|| Failure::garbled("Ollama", "its list of models"))?;
    Ok(models
        .iter()
        .filter_map(|model| {
            let name = model["name"].as_str()?.to_owned();
            // `remote_host` is how Ollama marks a model it passes on; a
            // cloud model's name says so as well, and is believed either way.
            let elsewhere = model["remote_host"]
                .as_str()
                .filter(|host| !host.trim().is_empty())
                .map(place_of)
                .or_else(|| is_cloud_name(&name).then(|| CLOUD.to_owned()));
            Some(Installed {
                name,
                bytes: model["size"].as_u64(),
                elsewhere,
            })
        })
        .collect())
}

/// Where Ollama's cloud models are answered.
const CLOUD: &str = "ollama.com";

/// Whether a model's name is one of Ollama's cloud models': its tag is
/// `cloud`, or ends `-cloud` ("gpt-oss:120b-cloud", "glm-4.6:cloud").
pub(crate) fn is_cloud_name(name: &str) -> bool {
    name.rsplit_once(':')
        .is_some_and(|(_, tag)| tag == "cloud" || tag.ends_with("-cloud"))
}

/// A server's address as a person reads it: its host, and its port unless
/// that is the one its scheme implies.
fn place_of(address: &str) -> String {
    let host = crate::http::host_of(address);
    let implied = match address.split_once("://") {
        Some(("https", _)) => ":443",
        Some(("http", _)) => ":80",
        _ => return host,
    };
    host.strip_suffix(implied)
        .map_or(host.clone(), str::to_owned)
}

/// Where the Ollama server at `address` sends what `model` is asked: `None`
/// for this computer, or the place it passes the request on to. A model the
/// list does not name is judged by its name; a list that cannot be had is a
/// failure, since nothing may be sent until the answer is known.
pub(crate) fn where_ollama_runs(address: &str, model: &str) -> Result<Option<String>, Failure> {
    crate::offline::check("Ollama")?;
    let models = ollama_list(address.trim_end_matches('/'))?;
    let tagged = format!("{model}:latest");
    Ok(
        match models
            .into_iter()
            .find(|installed| installed.name == model || installed.name == tagged)
        {
            Some(installed) => installed.elsewhere,
            None => is_cloud_name(model).then(|| CLOUD.to_owned()),
        },
    )
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
    /// The Ollama server on this computer, with every model it has: the first
    /// is the one the row offers.
    OllamaHere { models: Vec<Installed> },
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
            Row::OllamaHere { models } => match models.first() {
                Some(model) => format!("Ollama on this computer ({})", model.name),
                None => "Ollama on this computer".into(),
            },
            Row::Local => "A helper on this computer".into(),
            Row::ClaudeWithKey => "Claude, over the internet".into(),
            Row::Service => "Another service (advanced)".into(),
        }
    }

    /// What the row says under its title: what the helper is, what it costs,
    /// and what leaves the computer. No more technical than the title.
    pub fn about(&self) -> String {
        let claude = || {
            let cents = crate::models::claude_model(crate::models::DEFAULT_CLAUDE_MODEL)
                .map(|model| crate::models::cost_words(model.paragraph_cents()))
                .unwrap_or_default();
            format!(
                "The best results, for {cents} a paragraph. What you select, and a \
                 little of the document around it, is sent to Anthropic."
            )
        };
        match self {
            Row::ClaudeHere(ClaudeLogin::Ant) => format!(
                "Uses the login of the ant command on this computer. {}",
                claude()
            ),
            Row::ClaudeHere(_) => format!(
                "Uses the Anthropic key already set on this computer. {}",
                claude()
            ),
            Row::OllamaHere { .. } => "Ollama runs on this computer, so nothing you write \
                                       leaves it. How quickly it answers depends on this computer."
                .into(),
            Row::Local => {
                // What will be downloaded, before anything is: the model, its
                // licence and its size, from the one constant that also says
                // what the download must hash to.
                let model = crate::local::MODEL;
                format!(
                    "Free and private: nothing you write leaves this computer. Downloads {} \
                     once ({}, {}). Slower and simpler than Claude — {}.",
                    crate::local::size_of(model.bytes()),
                    model.name,
                    model.licence,
                    crate::local::GOOD_AT
                )
            }
            Row::ClaudeWithKey => format!(
                "Needs an Anthropic API key (a Claude.ai subscription is not one). {}",
                claude()
            ),
            Row::Service => "An address and a key, for a service you already use.".into(),
        }
    }

    /// Whether choosing the row gives a helper that can answer now.
    ///
    /// The helper on this computer is ready in the sense that matters here:
    /// choosing it does something — it downloads what it needs and then
    /// answers — rather than being a row that says "later".
    pub fn is_ready(&self) -> bool {
        true
    }

    /// The settings once this row is chosen.
    pub fn choose(&self, settings: &mut Settings) {
        match self {
            // At the address the row names, whatever an earlier choice left:
            // the card says Claude's rows go to Anthropic, and Ollama's to
            // the server on this computer that was looked at.
            Row::ClaudeHere(login) => {
                settings.helper = Some(Choice::Claude);
                settings.claude.login = *login;
                settings.claude.address = crate::anthropic::ADDRESS.to_owned();
            }
            Row::OllamaHere { models } => {
                settings.helper = Some(Choice::Ollama);
                settings.ollama.address = OLLAMA.to_owned();
                if let Some(model) = models.first() {
                    settings.ollama.model = model.name.clone();
                }
            }
            Row::Local => settings.helper = Some(Choice::Local),
            Row::ClaudeWithKey => {
                settings.helper = Some(Choice::Claude);
                settings.claude.login = ClaudeLogin::Key;
                settings.claude.address = crate::anthropic::ADDRESS.to_owned();
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
    // A server with nothing pulled into it has nothing to answer with, and a
    // model it passes on elsewhere is not one the row, which says nothing
    // leaves this computer, may offer. The row offers the most recent model
    // that is not large — a person who pulled a small one did so to use it —
    // and lists the rest after it, in the server's order.
    let here = machine.ollama_models().map(|models| {
        models
            .into_iter()
            .filter(Installed::runs_here)
            .collect::<Vec<_>>()
    });
    if let Some(models) = here.filter(|models| !models.is_empty()) {
        let (small, large): (Vec<Installed>, Vec<Installed>) =
            models.into_iter().partition(|model| !model.is_large());
        rows.push(Row::OllamaHere {
            models: small.into_iter().chain(large).collect(),
        });
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
        /// Each model's name and size in gigabytes.
        ollama: Option<Vec<(&'static str, f64)>>,
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

        fn ollama_models(&self) -> Option<Vec<Installed>> {
            self.ollama.as_ref().map(|models| {
                models
                    .iter()
                    .map(|(name, gb)| Installed {
                        name: name.to_string(),
                        bytes: Some((gb * 1e9) as u64),
                        elsewhere: None,
                    })
                    .collect()
            })
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
            ollama: Some(vec![("qwen3:1.7b", 1.4), ("llama3.2:latest", 2.0)]),
            ..Fake::default()
        };
        let rows = ladder(&ollama);
        assert_eq!(rows[0].title(), "Ollama on this computer (qwen3:1.7b)");
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
            ollama: Some(vec![("qwen3:1.7b", 1.4)]),
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
            for said in [row.title(), row.about()] {
                let lower = said.to_lowercase();
                for word in ["model", "llm", "token", "inference", "endpoint", "gpu"] {
                    assert!(!lower.contains(word), "“{said}” says {word}");
                }
            }
        }
        assert_eq!(
            Row::Local.is_ready(),
            crate::provider::LOCAL_READY,
            "the helper on this computer is ready when its runtime is"
        );
        assert!(Row::ClaudeWithKey.is_ready() && Row::Service.is_ready());
    }

    /// A computer with an Ollama that has `0`, and nothing else.
    struct Here(Vec<Installed>);

    impl Machine for Here {
        fn var(&self, _: &str) -> Option<String> {
            None
        }
        fn ant_token(&self) -> Option<String> {
            None
        }
        fn ollama_models(&self) -> Option<Vec<Installed>> {
            Some(self.0.clone())
        }
    }

    /// A model Ollama passes on to another server — one of its cloud models,
    /// or one made with a remote host — sends what it is asked there. The row
    /// says nothing leaves the computer, so such a model is not offered on it.
    #[test]
    fn a_model_ollama_passes_on_elsewhere_is_not_offered() {
        use crate::tests::{serve, Reply};
        use serde_json::json;

        let tags = json!({"models": [
            {"name": "gpt-oss:120b-cloud", "model": "gpt-oss:120b-cloud",
             "remote_model": "gpt-oss:120b", "remote_host": "https://ollama.com:443",
             "modified_at": "2026-09-03T10:00:00Z", "size": 384, "digest": "c"},
            {"name": "helper:latest", "model": "helper:latest", "remote_model": "big",
             "remote_host": "http://gpu.example.com:11434", "size": 300, "digest": "d"},
            {"name": "glm-4.6:cloud", "model": "glm-4.6:cloud", "size": 400, "digest": "e"},
            {"name": "qwen3:1.7b", "model": "qwen3:1.7b",
             "modified_at": "2026-08-01T10:00:00Z", "size": 1_359_293_444_u64, "digest": "b"}
        ]});
        let server = serve(vec![Reply::status(200, tags)]);
        let models = ollama_models_at(&server.address).expect("the server answered");
        let places: Vec<(&str, Option<&str>)> = models
            .iter()
            .map(|model| (model.name.as_str(), model.elsewhere.as_deref()))
            .collect();
        assert_eq!(
            places,
            [
                ("gpt-oss:120b-cloud", Some("ollama.com")),
                ("helper:latest", Some("gpu.example.com:11434")),
                ("glm-4.6:cloud", Some("ollama.com")),
                ("qwen3:1.7b", None),
            ],
            "where each is answered: the server's word, or the name's"
        );
        assert!(!models[0].runs_here() && models[3].runs_here());
        for (name, cloud) in [
            ("gpt-oss:120b-cloud", true),
            ("glm-4.6:cloud", true),
            ("qwen3:1.7b", false),
            ("cloud", false),
            ("my-cloud", false),
            ("cloudy:7b", false),
        ] {
            assert_eq!(is_cloud_name(name), cloud, "{name}");
        }

        let rows = ladder(&Here(models.clone()));
        let Row::OllamaHere { models: offered } = &rows[0] else {
            panic!("Ollama first: {rows:?}");
        };
        let names: Vec<&str> = offered.iter().map(|model| model.name.as_str()).collect();
        assert_eq!(names, ["qwen3:1.7b"], "only the model on this computer");
        assert_eq!(
            ladder(&Here(models[..3].to_vec())),
            [Row::Local, Row::ClaudeWithKey, Row::Service],
            "an Ollama with nothing of its own has nothing to offer"
        );
    }

    /// A row is chosen as the card describes it, whatever an earlier choice
    /// left in the file: Ollama on this computer at this computer's address,
    /// Claude at Anthropic's.
    #[test]
    fn a_row_is_chosen_at_the_address_the_card_describes() {
        let mut settings = Settings::default();
        settings.ollama.address = "http://gpu.example.com:11434".into();
        settings.claude.address = "https://relay.example.com".into();
        settings.service.address = "https://api.example.com/v1".into();

        let ollama = Row::OllamaHere {
            models: vec![Installed {
                name: "qwen3:1.7b".into(),
                bytes: None,
                elsewhere: None,
            }],
        };
        let mut chosen = settings.clone();
        ollama.choose(&mut chosen);
        assert_eq!(chosen.ollama.address, OLLAMA);
        assert_eq!(chosen.ollama.model, "qwen3:1.7b");
        for row in [
            Row::ClaudeHere(ClaudeLogin::Environment),
            Row::ClaudeHere(ClaudeLogin::Ant),
            Row::ClaudeWithKey,
        ] {
            let mut chosen = settings.clone();
            row.choose(&mut chosen);
            assert_eq!(chosen.claude.address, crate::anthropic::ADDRESS, "{row:?}");
        }
        // A service's address is given in the settings box, which shows it.
        let mut chosen = settings.clone();
        Row::Service.choose(&mut chosen);
        assert_eq!(chosen.service, settings.service);
    }

    /// Ollama's own list, read from a server as Ollama serves it: each model
    /// with its size, the row offering the most recent that is not large and
    /// listing the rest after it.
    #[test]
    fn ollamas_models_come_with_their_sizes_and_the_small_ones_first() {
        use crate::tests::{nobody_home, serve, Reply};
        use serde_json::json;

        let tags = json!({"models": [
            {"name": "qwen3.6:27b-q5_k_m", "model": "qwen3.6:27b-q5_k_m",
             "modified_at": "2026-09-01T10:00:00Z", "size": 19_231_100_101_u64,
             "digest": "a", "details": {"parameter_size": "26.9B", "quantization_level": "Q5_K_M"}},
            {"name": "qwen3:1.7b", "model": "qwen3:1.7b",
             "modified_at": "2026-08-01T10:00:00Z", "size": 1_359_293_444_u64,
             "digest": "b", "details": {"parameter_size": "2.0B", "quantization_level": "Q4_K_M"}},
            {"name": "unsized:latest", "model": "unsized:latest"},
            {"model": "nameless"}
        ]});
        let server = serve(vec![Reply::status(200, tags)]);
        let models = ollama_models_at(&server.address).expect("the server answered");
        assert_eq!(server.heard().path, "/api/tags");
        assert_eq!(
            models,
            [
                Installed {
                    name: "qwen3.6:27b-q5_k_m".into(),
                    bytes: Some(19_231_100_101),
                    elsewhere: None
                },
                Installed {
                    name: "qwen3:1.7b".into(),
                    bytes: Some(1_359_293_444),
                    elsewhere: None
                },
                Installed {
                    name: "unsized:latest".into(),
                    bytes: None,
                    elsewhere: None
                },
            ],
            "every model that has a name, in the server's order"
        );
        assert_eq!(models[0].size().as_deref(), Some("19 GB"));
        assert!(models[0].is_large());
        assert_eq!(models[1].size().as_deref(), Some("1.4 GB"));
        assert!(!models[1].is_large());
        assert_eq!(models[2].size(), None);
        assert!(!models[2].is_large(), "a size nobody said is not large");
        assert_eq!(ollama_models_at(&nobody_home()), None);

        // The row offers the small one, though the large one is more recent.
        let rows = ladder(&Here(models.clone()));
        let Row::OllamaHere { models: offered } = &rows[0] else {
            panic!("Ollama first: {rows:?}");
        };
        let names: Vec<&str> = offered.iter().map(|model| model.name.as_str()).collect();
        assert_eq!(
            names,
            ["qwen3:1.7b", "unsized:latest", "qwen3.6:27b-q5_k_m"],
            "the small ones first, in the server's order, then the large"
        );
        let mut settings = Settings::default();
        rows[0].choose(&mut settings);
        assert_eq!(settings.ollama.model, "qwen3:1.7b");

        // Only large models: the most recent is still offered.
        let large_only: Vec<Installed> = models.into_iter().take(1).collect();
        assert_eq!(
            ladder(&Here(large_only))[0].title(),
            "Ollama on this computer (qwen3.6:27b-q5_k_m)"
        );

        for (bytes, words) in [
            (512, "512 bytes"),
            (999_400, "999 KB"),
            (999_600, "1 MB"),
            (999_700_000, "1.0 GB"),
            (850_000_000, "850 MB"),
            (1_100_000_000, "1.1 GB"),
            (9_940_000_000, "9.9 GB"),
            (9_960_000_000, "10 GB"),
            (17_420_432_756, "17 GB"),
        ] {
            assert_eq!(size_words(bytes), words, "{bytes} bytes");
        }
    }
}
