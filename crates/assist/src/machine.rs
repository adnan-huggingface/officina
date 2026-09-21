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
    /// What the computer is made of, as far as a helper's room and speed
    /// go.
    fn hardware(&self) -> Hardware;
}

/// What a computer has that decides whether a helper can run on it, and
/// which.
///
/// **Nothing is run to find out.** Memory is read from the system, the
/// processor's vector instructions asked of the processor, and a graphics
/// processor looked for by the tool its driver ships; a model is never
/// loaded to see whether it fits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hardware {
    /// Memory in bytes, the whole of it.
    pub memory: u64,
    /// Whether the processor has the wide vector instructions a model is
    /// read with at any speed: AVX2 on x86-64 (roughly 2015 on), always on
    /// arm64.
    pub fast_vectors: bool,
    /// A graphics processor with its own memory, when one is there, and
    /// whether this build can use it.
    pub graphics: Option<Graphics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graphics {
    pub name: String,
    /// Its memory in bytes, when the driver says.
    pub memory: Option<u64>,
    /// What use Officina can make of it.
    pub runs: Runs,
}

/// What use Officina can make of a graphics card — and so, when it can make
/// none, which sentence the person is owed.
///
/// **"Not here" and "not anywhere" are different things**, and saying the
/// first when the second is true sends a person to fetch a build they are
/// already running. candle compiles no kernels at all below
/// [`crate::local::KERNEL_FLOOR`], so a card under it is not a matter of
/// which archive was downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runs {
    /// The helper runs on this card, in this build.
    Here,
    /// Not in this build — its kernels are not in it — but the graphics
    /// build's would.
    InTheGraphicsBuild,
    /// In no build of Officina: the card is older than any kernels candle
    /// compiles.
    Nowhere,
    /// This build has kernels the card could run, and the driver would not
    /// open it — held by another process, or not there to be opened.
    NotOpened,
}

/// What a card keeps for itself beside a model: the context, the display.
pub const CARD_ROOM: u64 = 1_000_000_000;

/// What the rest of the computer needs while a model runs: the applications,
/// the document, the system. A model is offered only where it and this fit
/// in memory together.
pub const ROOM: u64 = 4_000_000_000;

/// The bar's sixth item, in seconds: about how long a person waits before
/// the first word of an answer. A model whose measured wait on a processor
/// alone is longer is not offered on one.
pub const FIRST_WORD_BAR: u32 = 5;

/// The largest model of the catalogue this computer can hold and run within
/// the bar, or none.
///
/// **Where the answer is none, no local helper is offered.** A person is told
/// what the computer lacks rather than given a helper below the bar, which
/// the first release did and the user judged a toy. Three things decide it:
/// the processor's vector instructions, without which no size reads a
/// request in any useful time; memory — a model's weights read in plus what
/// it works in, with [`ROOM`] over for everything else; and the measured
/// wait before the first word on a processor alone, against
/// [`FIRST_WORD_BAR`]. As measured (`bugs/assist-bar.md`), no model of the
/// catalogue meets that wait on a processor alone, so on a processor the
/// answer is none; on a graphics processor this build can use, the card's
/// memory and the card's measured wait decide.
pub fn tier(hardware: &Hardware) -> Option<&'static crate::local::Model> {
    tier_of(hardware, &crate::local::MODELS)
}

/// The same, over any catalogue — the measured one, or a test's.
pub fn tier_of<'a>(
    hardware: &Hardware,
    models: &'a [crate::local::Model],
) -> Option<&'a crate::local::Model> {
    // A graphics processor this build can use: the largest model its memory
    // holds with room over, at the card's measured wait. The processor's
    // memory and instructions do not come into it.
    if let Some(card) = hardware
        .graphics
        .as_ref()
        .filter(|card| card.runs == Runs::Here)
    {
        let memory = card.memory.unwrap_or(0);
        return models.iter().rev().find(|model| {
            model.memory.saturating_add(CARD_ROOM) <= memory
                && model.waits.graphics <= FIRST_WORD_BAR
        });
    }
    if !hardware.fast_vectors {
        return None;
    }
    models.iter().rev().find(|model| {
        model.memory.saturating_add(ROOM) <= hardware.memory
            && model.waits.processor <= FIRST_WORD_BAR
    })
}

/// Why `machine` is offered no helper of its own, when it is not — for a
/// helper chosen under an earlier version, or on another computer, whose
/// settings name one that is not on this computer's disk and that this
/// computer cannot run within the bar. One that is on the disk answers
/// regardless: the person downloaded it, and it is theirs. Under a test the
/// computer is not examined, and the answer is that it can.
pub fn cannot_run_local(machine: &dyn Machine) -> Option<String> {
    if crate::offline::active() {
        return None;
    }
    let hardware = machine.hardware();
    match tier(&hardware) {
        Some(_) => None,
        None => Some(why_not(&hardware)),
    }
}

/// Why this computer is offered no helper of its own, in words a person can
/// act on.
pub fn why_not(hardware: &Hardware) -> String {
    why_not_of(hardware, &crate::local::MODELS)
}

/// The same, over any catalogue.
pub fn why_not_of(hardware: &Hardware, models: &[crate::local::Model]) -> String {
    let smallest = &models[0];
    // A card the helper runs on that is not offered a model: too little
    // memory, or a
    // wait not yet measured — said as the card's, since the processor was
    // not asked.
    if let Some(card) = hardware
        .graphics
        .as_ref()
        .filter(|card| card.runs == Runs::Here)
    {
        let needs = crate::local::size_of(smallest.memory + CARD_ROOM);
        let mut why = match card.memory {
            Some(memory) if memory < smallest.memory.saturating_add(CARD_ROOM) => format!(
                "This computer's graphics processor has {} of its own memory; a helper worth \
                 having needs {needs} of it.",
                crate::local::size_of(memory)
            ),
            Some(_) => "Officina has not yet measured its helper on a graphics processor like \
                        this one, so it does not offer one here yet."
                .to_owned(),
            None => format!(
                "Officina could not tell how much memory of its own this computer's graphics \
                 processor has; a helper worth having needs {needs} of it."
            ),
        };
        why.push_str(" Ollama on this computer, or Claude, can answer.");
        return why;
    }
    let needs = crate::local::size_of(smallest.memory + ROOM);
    let has = crate::local::size_of(hardware.memory);
    let fits = hardware.memory >= smallest.memory.saturating_add(ROOM);
    let mut why = match (hardware.fast_vectors, hardware.memory, fits) {
        (false, _, _) => {
            "This computer's processor is too old to run a helper worth having.".to_owned()
        }
        (true, 0, _) => format!(
            "Officina could not tell how much memory this computer has; a helper worth having \
             needs {needs} to run beside your documents."
        ),
        (true, _, false) => format!(
            "This computer has {has} of memory; a helper worth having needs {needs} to run beside \
             your documents."
        ),
        // Room enough, and still no: the wait. Said with the measured number,
        // so that the sentence changes when the number does.
        (true, _, true) => format!(
            "On this computer's processor alone, Officina's own helper would take {} before \
             its first word; a helper worth having takes a few seconds.",
            seconds(smallest.waits.processor)
        ),
    };
    // The card, named, and **which** "no" it is: sending a person to fetch
    // the graphics build they are already running is worse than saying
    // nothing, and so is telling them a card from last year is too old.
    match &hardware.graphics {
        Some(card) => why.push_str(&match card.runs {
            Runs::InTheGraphicsBuild => format!(
                " It has a graphics processor ({}) that this build of Officina cannot use: the \
                 graphics build would, and so would Ollama on this computer.",
                card.name
            ),
            Runs::Nowhere => format!(
                " It has a graphics processor ({}) that is older than Officina's helper can \
                 use: that takes a card from 2020 on, the GeForce RTX 30 series or later. \
                 Ollama on this computer, or Claude, can answer.",
                card.name
            ),
            Runs::NotOpened => format!(
                " It has a graphics processor ({}) that the driver would not open for \
                 Officina — another program may have it to itself. Ollama on this computer, \
                 or Claude, can answer.",
                card.name
            ),
            // Unreachable: a card the helper runs on was answered for above.
            Runs::Here => String::new(),
        }),
        None => {
            why.push_str(" Ollama on a computer with a graphics processor, or Claude, can answer.")
        }
    }
    why
}

/// The computer the application runs on. Under a test it has nothing: no
/// variable is read, no command is run and no server is asked, and each look
/// not taken is counted.
pub struct ThisComputer;

impl Machine for ThisComputer {
    fn hardware(&self) -> Hardware {
        if crate::offline::active() {
            crate::offline::count();
            // A test's computer has nothing: no memory is read and no
            // command run, and the rows that follow are the ones a computer
            // that can run nothing gets.
            return Hardware {
                memory: 0,
                fast_vectors: false,
                graphics: None,
            };
        }
        Hardware {
            memory: probe::memory().unwrap_or(0),
            fast_vectors: probe::fast_vectors(),
            graphics: probe::graphics(),
        }
    }

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

/// How much of a command's output is read and thrown away after the line that
/// was wanted. Far above what any probe of a machine prints — `nvidia-smi`
/// gives about sixty bytes a graphics card — and there so that a command
/// which will not stop talking cannot be read for ever.
const DRAIN_MOST: u64 = 1 << 20;

/// The first line a command prints, trimmed, if it prints a line with
/// something on it and succeeds within `wait`. The command is killed when the
/// time is up, and a process it left behind holding its output open does not
/// hold this up: the line is waited for on a thread of its own, for no longer
/// than the command is.
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
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let read = reader.read_line(&mut line);
        let _ = tell.send(read.ok().map(|_| line));
        // Whatever else it has to say is read and thrown away. A command
        // that prints a line for every graphics card would otherwise be
        // killed writing the second, into a pipe that closed after the
        // first, and then judged a failure for it — and the card's memory,
        // which the first line already gave, would be thrown away with it.
        //
        // **This thread lives as long as anything holds the pipe open, not
        // as long as the deadline.** Killing the command closes its own end,
        // but a process it left behind keeps its inherited end open, and
        // this waits on that. The bound below is on what is read, not on how
        // long: it is there so that a command which will not stop talking
        // cannot be read for ever, and it is far above anything a probe of a
        // machine prints.
        let _ = std::io::copy(&mut (&mut reader).take(DRAIN_MOST), &mut std::io::sink());
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
    let line = line.trim().to_owned();
    (status.success() && !line.is_empty()).then_some(line)
}

/// A row of the first-run card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Claude, with the credential this computer already has.
    ClaudeHere(ClaudeLogin),
    /// The Ollama server on this computer, with every model it has: the first
    /// is the one the row offers.
    OllamaHere { models: Vec<Installed> },
    /// The helper on this computer, downloaded once: the largest model of
    /// the catalogue the computer can run, and where — the graphics
    /// processor the tier judged, or the processor.
    Local {
        model: crate::local::Model,
        on: crate::local::Where,
    },
    /// No helper on this computer: what it lacks, and what would answer
    /// instead. Shown, not choosable.
    NoLocal { because: String },
    /// Claude, with a key the person pastes.
    ClaudeWithKey,
    /// Another service, by address and key.
    Service,
}

/// A wait in words: "a few seconds", "about half a minute", "about a minute".
fn seconds(n: u32) -> String {
    match n {
        0..=9 => "a few seconds".to_owned(),
        10..=19 => "about a quarter of a minute".to_owned(),
        20..=44 => "about half a minute".to_owned(),
        45..=89 => "about a minute".to_owned(),
        _ => format!("about {} minutes", n.div_ceil(60)),
    }
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
            Row::Local { model, .. } => {
                format!("A helper on this computer ({})", model.short_name())
            }
            Row::NoLocal { .. } => "No helper on this computer".into(),
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
            Row::Local { model, on } => {
                // What will be downloaded, before anything is: the model, its
                // licence and its size, from the constant that also says what
                // the download must hash to — and where it will run, at what
                // wait, from the measured numbers for that kind of device.
                let wait = match on {
                    crate::local::Where::Graphics => model.waits.graphics,
                    crate::local::Where::Processor => model.waits.processor,
                };
                format!(
                    "Free and private: nothing you write leaves this computer. Downloads {} \
                     once ({}, {}), and uses about {} of memory while it works {}. The first \
                     word of an answer takes {}.",
                    crate::local::size_of(model.bytes()),
                    model.short_name(),
                    model.licence,
                    crate::local::size_of(model.memory),
                    on.words(),
                    seconds(wait),
                )
            }
            Row::NoLocal { because } => because.clone(),
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
        !matches!(self, Row::NoLocal { .. })
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
            Row::Local { model, .. } => {
                settings.helper = Some(Choice::Local);
                settings.local.model = model.folder.to_owned();
            }
            Row::NoLocal { .. } => {}
            Row::ClaudeWithKey => {
                settings.helper = Some(Choice::Claude);
                settings.claude.login = ClaudeLogin::Key;
                settings.claude.address = crate::anthropic::ADDRESS.to_owned();
            }
            Row::Service => settings.helper = Some(Choice::Service),
        }
    }
}

/// How the computer is examined, one platform at a time.
mod probe {
    use super::Graphics;
    use super::Runs;

    /// The computer's memory in bytes.
    pub fn memory() -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            let info = std::fs::read_to_string("/proc/meminfo").ok()?;
            let line = info.lines().find(|line| line.starts_with("MemTotal:"))?;
            let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
            Some(kilobytes * 1024)
        }
        #[cfg(target_os = "macos")]
        {
            let mut command = std::process::Command::new("sysctl");
            command.args(["-n", "hw.memsize"]);
            super::first_line_within(command, std::time::Duration::from_secs(3))?
                .trim()
                .parse()
                .ok()
        }
        #[cfg(target_os = "windows")]
        {
            let mut status = windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX {
                dwLength: std::mem::size_of::<
                    windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX,
                >() as u32,
                dwMemoryLoad: 0,
                ullTotalPhys: 0,
                ullAvailPhys: 0,
                ullTotalPageFile: 0,
                ullAvailPageFile: 0,
                ullTotalVirtual: 0,
                ullAvailVirtual: 0,
                ullAvailExtendedVirtual: 0,
            };
            // SAFETY: a valid, correctly sized structure is passed, as the
            // call requires.
            let ok = unsafe {
                windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut status)
            };
            (ok != 0).then_some(status.ullTotalPhys)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            None
        }
    }

    /// Whether the processor has the vector instructions a model is read
    /// with at any speed.
    pub fn fast_vectors() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            std::arch::is_x86_feature_detected!("avx2")
        }
        #[cfg(target_arch = "aarch64")]
        {
            true
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            false
        }
    }

    /// A graphics processor, as its driver's own tool reports one. Only
    /// NVIDIA's is asked for now: it is the one candle could use, and the one
    /// most likely to be running an Ollama.
    pub fn graphics() -> Option<Graphics> {
        // With the driver linked in, the driver's own list is the answer.
        // Asking a command-line tool to contradict it would add nothing and
        // could only mislead: the tool cannot say why a card was not opened.
        #[cfg(feature = "cuda")]
        {
            graphics_of(crate::local::card_in_use().as_ref(), crate::local::cards())
        }
        // A build with no driver linked into it has only the tool.
        #[cfg(not(feature = "cuda"))]
        {
            named_graphics()
        }
    }

    /// The card's row from what the driver said: the one open, which this
    /// build can use by the fact of having chosen it; else the largest the
    /// driver lists, which it cannot — the kernels are not in this binary,
    /// or not for that card — so that the sentence names it and points at
    /// the other archive. None where the driver lists nothing.
    ///
    /// Kept apart from asking the driver so that the rule can be tested
    /// without a graphics processor.
    #[cfg(any(feature = "cuda", test))]
    pub fn graphics_of(
        in_use: Option<&crate::local::Card>,
        listed: &[crate::local::Card],
    ) -> Option<Graphics> {
        if let Some(card) = in_use {
            return Some(from_driver(card, Runs::Here));
        }
        // Not running on one. Which "no" it is decides what the person is
        // told: a card under the floor is under every build's floor, and
        // telling them to fetch the graphics build would send them for what
        // they already have.
        let card = crate::local::largest_card(listed)?;
        let runs = match card.runs_the_kernels() {
            true => Runs::NotOpened,
            false => Runs::Nowhere,
        };
        Some(from_driver(card, runs))
    }

    /// A card as the driver described it. **Its memory is always known**:
    /// the driver said it, which is the whole reason for asking the driver
    /// rather than a program that may not be installed.
    #[cfg(any(feature = "cuda", test))]
    pub fn from_driver(card: &crate::local::Card, runs: Runs) -> Graphics {
        Graphics {
            name: card.name.clone(),
            memory: Some(card.memory),
            runs,
        }
    }

    /// The graphics processor as its driver's own tool names it. Only a
    /// build without the CUDA feature comes here: one with it has the driver
    /// itself to ask, which is better in every way, and needs no program to
    /// be installed alongside.
    #[cfg(not(feature = "cuda"))]
    fn named_graphics() -> Option<Graphics> {
        // Asked the way `ant` is: no console window on Windows, and a
        // deadline, since a driver asleep can take seconds to answer.
        let mut command = std::process::Command::new("nvidia-smi");
        command.args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ]);
        let line = super::first_line_within(command, std::time::Duration::from_secs(5))?;
        Some(card_in_line(&line))
    }

    /// The card in one line of `nvidia-smi --query-gpu=name,memory.total
    /// --format=csv,noheader,nounits`, which is `NVIDIA GeForce RTX 3090,
    /// 24576`. Kept apart from the running of the command so that what it
    /// makes of a line can be tested without a graphics card.
    ///
    /// **The number is MiB**, as the tool's own header says; read as millions
    /// it understated every card by a twentieth. No model is offered on the
    /// strength of it — a card named this way is never one the helper runs
    /// on — but the figure is printed to the person in the sentence about
    /// the card, and a wrong one there is still wrong. A card whose memory
    /// cannot be read is still a card: the sentence for that says the memory
    /// is unknown rather than judging the processor instead.
    ///
    /// A card named this way is never one the helper runs on: only a build
    /// with no CUDA in it asks the tool, and such a build has no kernels for
    /// any card. The tool is not asked for the card's capability, so the
    /// most that can be said of it is that *this* build cannot use it.
    #[cfg(any(not(feature = "cuda"), test))]
    pub fn card_in_line(line: &str) -> Graphics {
        // The memory is the last field, so that a card with a comma in its
        // name loses none of it; `--format=csv` does not quote, and the two
        // fields asked for are all there are.
        let (name, mebibytes) = line.rsplit_once(',').unwrap_or((line, ""));
        let memory = mebibytes
            .trim()
            .parse::<u64>()
            .ok()
            .map(|mib| mib * (1 << 20));
        Graphics {
            name: name.trim().to_owned(),
            memory,
            // A build with no CUDA in it is the only one that asks the tool,
            // and the tool does not say the card's capability — so what can
            // be said is that this build cannot, not that none could.
            runs: Runs::InTheGraphicsBuild,
        }
    }
}

/// The card's rows, first the preselected one.
pub fn ladder(machine: &dyn Machine) -> Vec<Row> {
    ladder_of(machine, &crate::local::MODELS)
}

/// The same, over any catalogue — the measured one, or a test's.
pub fn ladder_of(machine: &dyn Machine, models: &[crate::local::Model]) -> Vec<Row> {
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
    // The helper on this computer: the largest model the computer can hold,
    // or a row that says why there is none — never a helper below the bar.
    let hardware = machine.hardware();
    let on = match hardware
        .graphics
        .as_ref()
        .is_some_and(|card| card.runs == Runs::Here)
    {
        true => crate::local::Where::Graphics,
        false => crate::local::Where::Processor,
    };
    match tier_of(&hardware, models) {
        Some(model) => rows.push(Row::Local { model: *model, on }),
        None => rows.push(Row::NoLocal {
            because: why_not_of(&hardware, models),
        }),
    }
    rows.extend([Row::ClaudeWithKey, Row::Service]);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// The catalogue as it would be on a computer that reads a request fast
    /// enough — what the ladder's own tests run over, since the measured one
    /// is offered on no processor alone.
    const FAST: [crate::local::Model; 2] = [
        crate::local::Model {
            waits: crate::local::Waits {
                processor: 3,
                graphics: 1,
            },
            ..crate::local::QWEN3_4B
        },
        crate::local::Model {
            waits: crate::local::Waits {
                processor: 4,
                graphics: 1,
            },
            ..crate::local::QWEN3_8B
        },
    ];

    #[derive(Default)]
    struct Fake {
        vars: Vec<(&'static str, &'static str)>,
        ant: Option<&'static str>,
        /// Each model's name and size in gigabytes.
        ollama: Option<Vec<(&'static str, f64)>>,
        ant_asked: Cell<bool>,
        hardware: Option<Hardware>,
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

        fn hardware(&self) -> Hardware {
            self.hardware.clone().unwrap_or_else(ample)
        }
    }

    /// A computer that can hold the smallest model of the catalogue and no
    /// more: what most of these tests assume about the computer, so that the
    /// row they expect is the smallest.
    fn ample() -> Hardware {
        Hardware {
            memory: FAST[0].memory + ROOM,
            fast_vectors: true,
            graphics: None,
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

    /// A command with more to say than is read must not be judged a failure
    /// for the pipe that closed under it. `nvidia-smi` prints a line for
    /// every graphics card, and on a computer with two it was killed writing
    /// the second — so the card's memory came back unknown, and the helper on
    /// this computer was withheld from a machine that could run it. Found on
    /// a workstation with an RTX 3090 and a Tesla P40, where the probe failed
    /// about three times in four.
    #[cfg(unix)]
    #[test]
    fn a_command_with_more_to_say_than_is_read_is_not_failed_for_the_closing_pipe() {
        let shell = |script: &str| {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            command
        };
        let wait = Duration::from_secs(5);
        assert_eq!(
            first_line_within(
                shell("echo first; sleep 0.2; echo second; echo third"),
                wait
            )
            .as_deref(),
            Some("first"),
            "the first line is the answer, and the lines after it are not an error"
        );
        // The rule it must not swallow: a command that fails still gives
        // nothing, however many lines it printed first.
        assert_eq!(
            first_line_within(shell("echo first; sleep 0.2; echo second; exit 1"), wait),
            None,
            "a command that fails on its own account is still a failure"
        );
        // More than a pipe holds. Three short lines fit in the pipe's own
        // buffer, so they prove only that the pipe stayed open; a command
        // with more to write than that blocks unless the rest is truly read,
        // and then never exits, and then dies at the deadline.
        assert_eq!(
            first_line_within(
                shell("echo first; head -c 200000 /dev/zero | tr '\\0' x; echo"),
                wait
            )
            .as_deref(),
            Some("first"),
            "the rest is read, not merely left open"
        );
        assert_eq!(
            first_line_within(shell("echo; echo second"), wait),
            None,
            "a line with nothing on it is not an answer"
        );
        // And the limit of it, said plainly: what is read after the line is
        // bounded, so a command that will not stop talking is cut off after
        // `DRAIN_MOST` and fails with the pipe, as it did before. No probe
        // of a machine comes near it — this writes a megabyte more than the
        // bound to reach it at all.
        let over = format!(
            "echo first; head -c {} /dev/zero | tr '\\0' x; echo",
            DRAIN_MOST + (1 << 20)
        );
        assert_eq!(
            first_line_within(shell(&over), wait),
            None,
            "past the bound it is cut off, and a cut-off command gives nothing"
        );
    }

    /// **Which "no" it is decides what the person is told.** A card under
    /// the floor is under every build's floor, so telling its owner to fetch
    /// the graphics build sends them for what they are already running; a
    /// card this build simply has no kernels for is the opposite. Neither is
    /// ever offered a model: that is what the first case of [`Runs`] means,
    /// and a card offered one it cannot run dies at the first request with
    /// `CUDA_ERROR_INVALID_PTX`.
    #[test]
    fn which_no_a_card_gets_decides_which_build_the_person_is_sent_for() {
        let card = |ordinal, name: &str, memory, capability| crate::local::Card {
            ordinal,
            name: name.to_owned(),
            memory,
            capability,
        };
        let pascal = card(0, "Tesla P40", 24_000_000_000, (6, 1));
        let ampere = card(1, "NVIDIA GeForce RTX 3090", 25_000_000_000, (8, 6));
        let sentence = |row: Graphics| {
            why_not_of(
                &Hardware {
                    memory: 64_000_000_000,
                    fast_vectors: true,
                    graphics: Some(row),
                },
                &FAST,
            )
        };

        // Running on one: that card, and the helper is offered on it.
        let row =
            probe::graphics_of(Some(&ampere), &[pascal.clone(), ampere.clone()]).expect("a card");
        assert_eq!(row.name, "NVIDIA GeForce RTX 3090");
        assert_eq!(row.runs, Runs::Here);

        // Listed, and older than any kernels candle builds: not offered, and
        // **not** told to fetch the build they are running.
        let row = probe::graphics_of(None, std::slice::from_ref(&pascal)).expect("a card");
        assert_eq!(row.runs, Runs::Nowhere);
        assert_eq!(row.memory, Some(24_000_000_000));
        let why = sentence(row);
        assert!(why.contains("Tesla P40"), "{why}");
        assert!(
            why.contains("older than Officina's helper can use"),
            "{why}"
        );
        assert!(
            !why.contains("graphics build"),
            "a card no build can use must not send them for another build: {why}"
        );

        // Listed, new enough, and the driver would not open it.
        let row = probe::graphics_of(None, std::slice::from_ref(&ampere)).expect("a card");
        assert_eq!(row.runs, Runs::NotOpened);
        let why = sentence(row);
        assert!(why.contains("would not open"), "{why}");
        assert!(!why.contains("older than"), "{why}");

        // The tool's answer, which only a build with no kernels asks for:
        // this build cannot, and the other one could.
        let row = probe::card_in_line("NVIDIA GeForce RTX 3090, 24576");
        assert_eq!(row.runs, Runs::InTheGraphicsBuild);
        let why = sentence(row);
        assert!(why.contains("the graphics build would"), "{why}");

        // None of the three is ever offered a model.
        for runs in [Runs::Nowhere, Runs::NotOpened, Runs::InTheGraphicsBuild] {
            let hardware = Hardware {
                memory: 64_000_000_000,
                fast_vectors: false,
                graphics: Some(Graphics {
                    name: "a card".to_owned(),
                    memory: Some(80_000_000_000),
                    runs,
                }),
            };
            assert_eq!(
                tier_of(&hardware, &FAST),
                None,
                "{runs:?} must not be offered a model"
            );
        }

        // The card named is the largest, not the first the driver lists.
        let smaller = card(0, "NVIDIA GeForce GTX 1080", 8_000_000_000, (6, 1));
        let bigger = card(1, "Tesla P40", 24_000_000_000, (6, 1));
        let row = probe::graphics_of(None, &[smaller, bigger]).expect("a card");
        assert_eq!(
            row.name, "Tesla P40",
            "the sentence names the card worth having, not whichever came first"
        );

        // Nothing listed: nothing said here.
        assert_eq!(probe::graphics_of(None, &[]), None);
    }

    /// A card the driver described is never of unknown memory, whichever
    /// build asked and whether or not its kernels run on it. The sentence
    /// about memory Officina could not read is for the other source — the
    /// command-line tool, which can answer `[N/A]`.
    #[test]
    fn a_card_the_driver_named_is_not_of_unknown_memory() {
        let card = crate::local::Card {
            ordinal: 1,
            name: "NVIDIA GeForce RTX 3090".to_owned(),
            memory: 25_769_803_776,
            capability: (8, 6),
        };
        for runs in [Runs::Here, Runs::NotOpened] {
            let row = probe::from_driver(&card, runs);
            assert_eq!(row.name, "NVIDIA GeForce RTX 3090");
            assert_eq!(
                row.memory,
                Some(25_769_803_776),
                "the driver said the memory, so the row has it"
            );
            assert_eq!(row.runs, runs);
        }
        // And a card of that size, said by the driver, holds the 8B.
        let offered = |memory| {
            let hardware = Hardware {
                memory: 8_000_000_000,
                fast_vectors: false,
                graphics: Some(probe::from_driver(
                    &crate::local::Card {
                        ordinal: 0,
                        name: "a card".to_owned(),
                        memory,
                        capability: (8, 6),
                    },
                    Runs::Here,
                )),
            };
            tier_of(&hardware, &crate::local::MODELS).map(|model| model.folder)
        };
        assert_eq!(
            offered(25_769_803_776),
            Some(crate::local::QWEN3_8B.folder),
            "a processor too small for anything does not come into it"
        );
        // **What the driver reports is under what the box says**: about two
        // per cent, because some of the card's memory is not the program's
        // to have. So the guide's figures are a size larger than the
        // arithmetic alone suggests — a card sold as 8 GB gets the smaller
        // helper, and one sold as 10 GB the larger. When these move, the
        // guide is wrong and this fails.
        assert_eq!(
            offered(8_361_279_488),
            Some(crate::local::QWEN3_4B.folder),
            "a card sold as 8 GB reports too little for the 8B"
        );
        assert_eq!(
            offered(10_401_873_920),
            Some(crate::local::QWEN3_8B.folder),
            "a card sold as 10 GB reports enough"
        );
        assert_eq!(offered(4_000_000_000), None, "and a small card, nothing");
    }

    /// What one line of `nvidia-smi` says about a card. The number is
    /// **MiB** — read as millions it understates every card by a twentieth,
    /// which on a card of 8 GB is the difference between the 8B being
    /// offered and not.
    #[test]
    fn a_cards_line_gives_its_name_and_its_memory_in_mebibytes() {
        let card = probe::card_in_line("NVIDIA GeForce RTX 3090, 24576");
        assert_eq!(card.name, "NVIDIA GeForce RTX 3090");
        assert_eq!(card.memory, Some(24_576 * 1024 * 1024));
        // The tool is asked only by a build with no kernels for any card.
        assert_eq!(card.runs, Runs::InTheGraphicsBuild);
        // 8 GB is 8192 MiB, which is 8.59 GB and not 8.19: read as millions
        // the tool's number understates every card by a twentieth.
        let eight = probe::card_in_line("NVIDIA GeForce RTX 3070, 8192");
        assert_eq!(eight.memory, Some(8_589_934_592));
        assert!(
            eight.memory.unwrap_or(0) > 8_192 * 1_000_000,
            "not millions: {:?}",
            eight.memory
        );
        // A comma in the name loses none of the memory.
        let comma = probe::card_in_line("Some Card, Special Edition, 16384");
        assert_eq!(comma.name, "Some Card, Special Edition");
        assert_eq!(comma.memory, Some(16_384 * 1024 * 1024));
        assert_eq!(comma.runs, Runs::InTheGraphicsBuild);
        // A card whose memory will not read is still a card, with the memory
        // unknown — the sentence for that says so.
        let unreadable = probe::card_in_line("Tesla P40, [N/A]");
        assert_eq!(unreadable.name, "Tesla P40");
        assert_eq!(unreadable.memory, None);
    }

    #[test]
    fn the_ladder_offers_what_the_machine_has_in_the_order_the_card_shows() {
        let always = [
            Row::Local {
                model: FAST[0],
                on: crate::local::Where::Processor,
            },
            Row::ClaudeWithKey,
            Row::Service,
        ];
        assert_eq!(
            ladder_of(&Fake::default(), &FAST),
            always,
            "nothing found: the helper here first"
        );

        let empty_key = Fake {
            vars: vec![("ANTHROPIC_API_KEY", "")],
            ..Fake::default()
        };
        assert_eq!(
            ladder_of(&empty_key, &FAST),
            always,
            "a variable set to nothing is not a key"
        );

        let key = Fake {
            vars: vec![("ANTHROPIC_API_KEY", "sk-ant")],
            ant: Some("token"),
            ..Fake::default()
        };
        let rows = ladder_of(&key, &FAST);
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
        assert_eq!(
            ladder_of(&token, &FAST)[0],
            Row::ClaudeHere(ClaudeLogin::Environment)
        );

        let ant = Fake {
            ant: Some("token"),
            ..Fake::default()
        };
        assert_eq!(ladder_of(&ant, &FAST)[0], Row::ClaudeHere(ClaudeLogin::Ant));

        let ollama = Fake {
            ollama: Some(vec![("qwen3:1.7b", 1.4), ("llama3.2:latest", 2.0)]),
            ..Fake::default()
        };
        let rows = ladder_of(&ollama, &FAST);
        assert_eq!(rows[0].title(), "Ollama on this computer (qwen3:1.7b)");
        assert_eq!(rows[1..], always);

        let empty_ollama = Fake {
            ollama: Some(vec![]),
            ..Fake::default()
        };
        assert_eq!(
            ladder_of(&empty_ollama, &FAST),
            always,
            "an Ollama with no models has nothing to offer"
        );

        let both = Fake {
            ant: Some("token"),
            ollama: Some(vec![("qwen3:1.7b", 1.4)]),
            ..Fake::default()
        };
        let rows = ladder_of(&both, &FAST);
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
            Row::Local {
                model: FAST[0],
                on: crate::local::Where::Processor
            }
            .is_ready(),
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
        fn hardware(&self) -> Hardware {
            ample()
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

        let rows = ladder_of(&Here(models.clone()), &FAST);
        let Row::OllamaHere { models: offered } = &rows[0] else {
            panic!("Ollama first: {rows:?}");
        };
        let names: Vec<&str> = offered.iter().map(|model| model.name.as_str()).collect();
        assert_eq!(names, ["qwen3:1.7b"], "only the model on this computer");
        assert_eq!(
            ladder_of(&Here(models[..3].to_vec()), &FAST),
            [
                Row::Local {
                    model: FAST[0],
                    on: crate::local::Where::Processor
                },
                Row::ClaudeWithKey,
                Row::Service
            ],
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
        let rows = ladder_of(&Here(models.clone()), &FAST);
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
            ladder_of(&Here(large_only), &FAST)[0].title(),
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

    /// A computer that cannot hold the smallest model, or whose processor
    /// lacks the vector instructions, is offered no helper of its own: the
    /// card says what it lacks and what would answer instead, and the row
    /// cannot be chosen.
    #[test]
    fn a_computer_below_the_floor_is_not_offered_a_local_helper_and_is_told_why() {
        let small = Hardware {
            memory: 8_000_000_000,
            fast_vectors: true,
            graphics: None,
        };
        assert_eq!(
            tier(&small),
            None,
            "8 GB holds no model beside the documents"
        );
        let rows = ladder_of(
            &Fake {
                hardware: Some(small.clone()),
                ..Fake::default()
            },
            &FAST,
        );
        let Some(Row::NoLocal { because }) = rows.first() else {
            panic!("{rows:?}");
        };
        assert!(because.contains("8.0 GB of memory"), "{because}");
        assert!(because.contains("needs 8.5 GB"), "{because}");
        assert!(!rows[0].is_ready(), "the row cannot be chosen");
        let mut settings = Settings::default();
        rows[0].choose(&mut settings);
        assert_eq!(settings.helper, None, "and choosing it changes nothing");
        assert_eq!(rows[0].title(), "No helper on this computer");
        assert_eq!(rows[0].about(), *because);
        assert_eq!(&rows[1..], [Row::ClaudeWithKey, Row::Service]);

        // An old processor: memory is not the question.
        let old = Hardware {
            memory: 64_000_000_000,
            fast_vectors: false,
            graphics: None,
        };
        assert_eq!(tier_of(&old, &FAST), None);
        assert!(
            why_not(&old).contains("processor is too old"),
            "{}",
            why_not(&old)
        );
        let unknown = Hardware {
            memory: 0,
            fast_vectors: true,
            graphics: None,
        };
        assert!(
            why_not(&unknown).contains("could not tell how much memory"),
            "{}",
            why_not(&unknown)
        );

        // A saved choice of the helper on this computer, on a computer that
        // cannot run one, is refused with the same sentence.
        let cannot = cannot_run_local(&Fake {
            hardware: Some(small.clone()),
            ..Fake::default()
        });
        assert_eq!(cannot.as_deref(), Some(why_not(&small).as_str()));

        // A graphics processor Officina's helper cannot use yet is named, so
        // that the person knows Ollama on this computer would.
        let card = Hardware {
            memory: 8_000_000_000,
            fast_vectors: true,
            graphics: Some(Graphics {
                name: "NVIDIA GeForce RTX 3090".into(),
                memory: Some(24_000_000_000),
                runs: Runs::InTheGraphicsBuild,
            }),
        };
        assert!(
            why_not(&card).contains("the graphics build would, and so would Ollama"),
            "{}",
            why_not(&card)
        );
        assert!(
            why_not(&small).ends_with(
                "Ollama on a computer with a graphics processor, or Claude, can answer."
            ),
            "{}",
            why_not(&small)
        );
    }

    /// The helper offered is the largest of the catalogue the computer can
    /// hold with room over for everything else, and its row says what the
    /// wait before the first word will feel like.
    #[test]
    fn the_tier_is_the_largest_model_the_computer_can_hold_and_says_how_it_will_feel() {
        let with = |memory: u64| Hardware {
            memory,
            fast_vectors: true,
            graphics: None,
        };
        let (small, large) = (&FAST[0], &FAST[1]);
        assert_eq!(tier_of(&with(small.memory + ROOM - 1), &FAST), None);
        assert_eq!(tier_of(&with(small.memory + ROOM), &FAST), Some(small));
        assert_eq!(tier_of(&with(large.memory + ROOM - 1), &FAST), Some(small));
        assert_eq!(tier_of(&with(large.memory + ROOM), &FAST), Some(large));
        assert_eq!(
            tier_of(&with(64_000_000_000), &FAST),
            Some(large),
            "nothing above the catalogue"
        );

        // The catalogue as measured: on a processor alone, no size meets the
        // bar's wait, however much memory there is — and the row says so with
        // the number, and where the answer lies.
        for model in &crate::local::MODELS {
            assert!(
                model.waits.processor > FIRST_WORD_BAR,
                "{}: {}s",
                model.name,
                model.waits.processor
            );
        }
        let plenty = with(64_000_000_000);
        assert_eq!(tier(&plenty), None);
        let why = why_not(&plenty);
        assert!(why.contains("before its first word"), "{why}");
        assert!(
            why.contains(&seconds(crate::local::MODELS[0].waits.processor)),
            "{why}"
        );
        assert!(
            why.contains("Ollama on a computer with a graphics processor, or Claude"),
            "{why}"
        );
        assert!(matches!(ladder(&Fake::default())[0], Row::NoLocal { .. }));
        assert_eq!(
            cannot_run_local(&Fake::default()).as_deref(),
            Some(why.as_str())
        );
        let mut card = plenty.clone();
        card.graphics = Some(Graphics {
            name: "NVIDIA GeForce RTX 3090".into(),
            memory: Some(24_000_000_000),
            runs: Runs::InTheGraphicsBuild,
        });
        assert!(
            why_not(&card).contains("the graphics build would, and so would Ollama"),
            "{}",
            why_not(&card)
        );

        let rows = ladder_of(
            &Fake {
                hardware: Some(with(16_000_000_000)),
                ..Fake::default()
            },
            &FAST,
        );
        assert_eq!(
            rows[0],
            Row::Local {
                model: *large,
                on: crate::local::Where::Processor
            },
            "16 GB holds the 8B: {rows:?}"
        );
        assert_eq!(rows[0].title(), "A helper on this computer (Qwen3 8B)");
        let about = rows[0].about();
        assert!(about.contains("5.0 GB"), "{about}");
        assert!(about.contains("7.5 GB of memory"), "{about}");
        assert!(about.contains("first word of an answer takes"), "{about}");
        assert!(about.contains(&seconds(large.waits.processor)), "{about}");
        assert!(rows[0].is_ready());
        let mut settings = Settings::default();
        rows[0].choose(&mut settings);
        assert_eq!(settings.helper, Some(Choice::Local));
        assert_eq!(settings.local.model, large.folder);
        assert_eq!(settings.local.model().folder, large.folder);

        // The words for a wait.
        assert_eq!(seconds(4), "a few seconds");
        assert_eq!(seconds(15), "about a quarter of a minute");
        assert_eq!(seconds(30), "about half a minute");
        assert_eq!(seconds(60), "about a minute");
        assert_eq!(seconds(150), "about 3 minutes");
    }

    /// A graphics processor this build can use is judged by its own memory
    /// and the model's measured wait on a card: the largest that fits with
    /// room over, whatever the processor and its memory are.
    #[test]
    fn a_usable_graphics_processor_offers_the_largest_model_it_holds_at_the_cards_wait() {
        let card = |memory: u64| Hardware {
            // A small, old processor: not what decides.
            memory: 4_000_000_000,
            fast_vectors: false,
            graphics: Some(Graphics {
                name: "NVIDIA GeForce RTX 3060".into(),
                memory: Some(memory),
                runs: Runs::Here,
            }),
        };
        let (small, large) = (&FAST[0], &FAST[1]);
        assert_eq!(tier_of(&card(small.memory + CARD_ROOM - 1), &FAST), None);
        assert_eq!(tier_of(&card(small.memory + CARD_ROOM), &FAST), Some(small));
        assert_eq!(
            tier_of(&card(large.memory + CARD_ROOM - 1), &FAST),
            Some(small)
        );
        assert_eq!(tier_of(&card(large.memory + CARD_ROOM), &FAST), Some(large));
        assert_eq!(tier_of(&card(24_000_000_000), &FAST), Some(large));

        // Too little card memory, said as the card's; a card whose memory the
        // driver did not say; and a card whose wait is not yet measured.
        let why = why_not_of(&card(4_000_000_000), &FAST);
        assert!(
            why.starts_with("This computer's graphics processor has 4.0 GB of its own memory"),
            "{why}"
        );
        assert!(
            why.ends_with("Ollama on this computer, or Claude, can answer."),
            "{why}"
        );

        // A card of unknown memory can still reach the tier — the tool can
        // answer `[N/A]` — and it is judged as no memory rather than as a
        // processor, so the sentence for it is said.
        let unknown_memory = Graphics {
            name: "Tesla P40".to_owned(),
            memory: None,
            runs: Runs::Here,
        };
        assert_eq!(
            tier_of(
                &Hardware {
                    memory: 64_000_000_000,
                    fast_vectors: true,
                    graphics: Some(unknown_memory)
                },
                &FAST
            ),
            None
        );
        assert!(why.contains("needs 5.5 GB of it"), "{why}");
        let mut unknown = card(0);
        unknown.graphics.as_mut().unwrap().memory = None;
        assert!(why_not_of(&unknown, &FAST).contains(
            "could not tell how much memory of its own this computer's graphics processor has"
        ));
        let unmeasured = [crate::local::Model {
            waits: crate::local::Waits {
                processor: 1,
                graphics: u32::MAX,
            },
            ..crate::local::QWEN3_8B
        }];
        assert_eq!(tier_of(&card(24_000_000_000), &unmeasured), None);
        assert!(why_not_of(&card(24_000_000_000), &unmeasured).contains("not yet measured"));

        // The ladder puts the card's model first, and the row says where.
        let rows = ladder_of(
            &Fake {
                hardware: Some(card(24_000_000_000)),
                ..Fake::default()
            },
            &FAST,
        );
        assert_eq!(
            rows[0],
            Row::Local {
                model: *large,
                on: crate::local::Where::Graphics
            },
            "{rows:?}"
        );
        let about = rows[0].about();
        assert!(
            about.contains("while it works on this computer's graphics processor"),
            "{about}"
        );
        assert!(about.contains("takes a few seconds"), "{about}");
    }

    /// A graphics processor this build cannot use — the portable build, or a
    /// driver that did not answer — is named where the computer is judged by
    /// its processor, and the graphics build is pointed at, since it would.
    #[test]
    fn a_graphics_processor_this_build_cannot_use_is_named_and_the_other_build_pointed_at() {
        let hardware = Hardware {
            memory: 32_000_000_000,
            fast_vectors: true,
            graphics: Some(Graphics {
                name: "NVIDIA GeForce RTX 3090".into(),
                memory: Some(24_000_000_000),
                runs: Runs::InTheGraphicsBuild,
            }),
        };
        // Judged as a processor: the measured catalogue offers nothing there.
        assert_eq!(tier(&hardware), None);
        let why = why_not(&hardware);
        assert!(
            why.starts_with("On this computer's processor alone"),
            "{why}"
        );
        assert!(why.contains("graphics processor (NVIDIA GeForce RTX 3090) that this build of Officina cannot use"), "{why}");
        assert!(why.contains("the graphics build would"), "{why}");
        // And a fast catalogue on that processor is offered as before: the
        // card that cannot be used does not stand in the way.
        assert_eq!(tier_of(&hardware, &FAST), Some(&FAST[1]));
    }

    /// Every model of the catalogue has its wait on a graphics processor
    /// measured, and within the bar — so a card that holds one is offered
    /// it: the larger from 8.5 GB of card memory, the smaller from 5.5.
    #[test]
    fn the_graphics_waits_are_measured_and_within_the_bar() {
        for model in &crate::local::MODELS {
            assert!(
                model.waits.graphics != u32::MAX && model.waits.graphics <= FIRST_WORD_BAR,
                "{}: {}s",
                model.name,
                model.waits.graphics
            );
            assert!(
                model.waits.processor > FIRST_WORD_BAR,
                "{}: the processor is another matter",
                model.name
            );
        }
        let card = |memory: u64| Hardware {
            memory: 8_000_000_000,
            fast_vectors: true,
            graphics: Some(Graphics {
                name: "NVIDIA GeForce RTX 3060".into(),
                memory: Some(memory),
                runs: Runs::Here,
            }),
        };
        use crate::local::{QWEN3_4B, QWEN3_8B};
        assert_eq!(tier(&card(24_000_000_000)), Some(&QWEN3_8B));
        assert_eq!(tier(&card(12_000_000_000)), Some(&QWEN3_8B));
        assert_eq!(tier(&card(8_000_000_000)), Some(&QWEN3_4B));
        assert_eq!(tier(&card(4_000_000_000)), None);
        let rows = ladder(&Fake {
            hardware: Some(card(24_000_000_000)),
            ..Fake::default()
        });
        assert_eq!(
            rows[0],
            Row::Local {
                model: QWEN3_8B,
                on: crate::local::Where::Graphics
            }
        );
        assert!(
            rows[0]
                .about()
                .contains("first word of an answer takes a few seconds"),
            "{}",
            rows[0].about()
        );
    }
}
