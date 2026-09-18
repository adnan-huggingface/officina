//! The helper on this computer: a quantized model read from a GGUF file and
//! run on the CPU, with `candle`.
//!
//! **It is a helper like the others.** It implements [`Provider`], so the
//! pane, the tools and both applications cannot tell it from Claude or from
//! Ollama: the same words in, the same text and tool calls out, the same
//! Stop, the same failures. What it does differently is where it runs —
//! nothing it is asked leaves the computer, which is the whole reason it is
//! the first row of the first-run card.
//!
//! **Nothing is bundled.** The weights are a download the person asks for,
//! having seen the model's name, its licence and its size ([`MODEL`]), and
//! they live in the cache directory, never in the repository and never beside
//! the settings. [`download`] checks what arrives against the hash the
//! constant names, so a half-download or a mirror's mistake cannot be run.
//!
//! **A test writes its own model.** The runtime's tests build a GGUF of a few
//! dozen kilobytes — two layers of arbitrary weights and a vocabulary of a
//! hundred — and run it: no network, no gigabyte, and a whole test suite that
//! still finishes in a moment.

use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_qwen3::ModelWeights;
use serde_json::{json, Value};
use tokenizers::Tokenizer;

use crate::conversation::{Block, Role, ToolCall};
use crate::event::{Ending, Usage};
use crate::provider::{Answer, Provider, Request, StopFlag};
use crate::tool::Tool;
use crate::{Failure, FailureKind};

/// The helper as the transcript names it.
pub const NAME: &str = "Helper on this computer";

/// What it is good at, said on the card and in the header rather than left
/// for the person to discover.
pub const GOOD_AT: &str = "good for rewording, grammar, summaries and simple sums";

/// One file of the model, and what it must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    /// What it is called in the cache directory.
    pub file: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    /// The SHA-256 of the file, lower-case hexadecimal. What arrives is
    /// checked against it, and refused if it does not match.
    pub sha256: &'static str,
}

/// The model this version of Officina runs on this computer.
///
/// **Pinned, not discovered.** A helper whose weights could be any file the
/// day's mirror happens to serve is a helper nobody can support: the name,
/// the licence, the size and the hash are written here, and the download
/// refuses anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    pub name: &'static str,
    /// Its licence, said before anything is downloaded.
    pub licence: &'static str,
    /// Where the licence and the model card can be read.
    pub about: &'static str,
    pub weights: Piece,
    pub tokenizer: Piece,
    /// How much memory it wants while it runs, in bytes, for the card.
    pub memory: u64,
}

impl Model {
    /// Both files, in the order they are fetched: the small one first, so
    /// that a bad address is known in a second rather than in a gigabyte.
    pub fn pieces(&self) -> [Piece; 2] {
        [self.tokenizer, self.weights]
    }

    /// How much there is to download, for the card.
    pub fn bytes(&self) -> u64 {
        self.weights.bytes + self.tokenizer.bytes
    }
}

/// Qwen3 1.7B, quantized to Q4_K_M, from the llama.cpp project's own
/// conversion; the tokenizer from Qwen's own repository. Apache-2.0, which is
/// what a suite that promises its users nothing they cannot redistribute can
/// ship a pointer to.
pub const MODEL: Model = Model {
    name: "Qwen3 1.7B (Q4_K_M)",
    licence: "Apache-2.0",
    about: "https://huggingface.co/Qwen/Qwen3-1.7B",
    weights: Piece {
        file: "Qwen3-1.7B-Q4_K_M.gguf",
        url: "https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
        bytes: 1_282_439_264,
        sha256: "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5",
    },
    tokenizer: Piece {
        file: "tokenizer.json",
        url: "https://huggingface.co/Qwen/Qwen3-1.7B/resolve/main/tokenizer.json",
        bytes: 11_422_654,
        sha256: "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
    },
    memory: 2_500_000_000,
};

/// How many tokens one answer may be. A helper that never stops is worse
/// than one that stops early: the person can ask again.
pub const MOST_TOKENS: usize = 1_024;

/// Where the weights live: `<cache>/models/<model>/`.
pub fn folder(cache: &Path) -> PathBuf {
    cache.join("models").join("qwen3-1.7b-q4-k-m")
}

/// Whether both of the model's files are there.
///
/// **A file under its own name is a whole file.** The download writes into
/// `<name>.part` and renames it only once its SHA-256 matches, so what is
/// there is what was asked for; a half-download is a part file, which this
/// does not count. Whether the weights then load is a question for the first
/// request, which says so in words a person can act on.
pub fn have(cache: &Path) -> bool {
    let folder = folder(cache);
    MODEL
        .pieces()
        .iter()
        .all(|piece| folder.join(piece.file).exists())
}

/// Removes the downloaded weights, and says how many bytes came back.
pub fn remove(cache: &Path) -> std::io::Result<u64> {
    let folder = folder(cache);
    let mut freed = 0;
    for piece in MODEL.pieces() {
        // The half-downloaded file counts too: it is space the person has
        // spent, and leaving it behind means "Remove" gives back less than
        // it said and a folder that will not go.
        for path in [
            folder.join(piece.file),
            folder.join(format!("{}.part", piece.file)),
        ] {
            if let Ok(found) = std::fs::metadata(&path) {
                freed += found.len();
                std::fs::remove_file(&path)?;
            }
        }
    }
    let _ = std::fs::remove_dir(&folder);
    Ok(freed)
}

/// A size as a person reads it: "1.3 GB".
pub fn size_of(bytes: u64) -> String {
    let (unit, scale) = match bytes {
        0..=999_999 => ("kB", 1_000.0),
        1_000_000..=999_999_999 => ("MB", 1_000_000.0),
        _ => ("GB", 1_000_000_000.0),
    };
    let size = bytes as f64 / scale;
    match size < 10.0 {
        true => format!("{size:.1} {unit}"),
        false => format!("{size:.0} {unit}"),
    }
}

// -------------------------------------------------------------- the download

/// How the download says where it has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// Downloads whatever of the model is missing, on the thread that calls it.
///
/// **Not under a test.** A test that reached out for a gigabyte would be a
/// test nobody could run twice; `offline` refuses this as it refuses every
/// other helper, and the download's own tests serve themselves on loopback.
///
/// Fetches whatever of the model is missing into `cache`, reporting progress
/// and reading `stop` between chunks.
///
/// **A gigabyte on a bad connection is many attempts.** A file that arrives
/// in part is kept as `<name>.part` and the next attempt asks for the rest
/// with a range request, so an interrupted download resumes rather than
/// starting again. What is whole and right is never fetched twice.
pub fn download_model(
    cache: &Path,
    stop: &StopFlag,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), Failure> {
    // **No test downloads a gigabyte, in any crate.** `offline` refuses every
    // helper under an application's tests, but this crate's own tests do not
    // enter offline mode — they serve themselves on loopback — so the one
    // function that would reach out for the real weights refuses at compile
    // time as well. The download's own tests call `download`, below, against
    // a server of their own.
    #[cfg(test)]
    {
        let _ = (cache, stop, progress);
        Err(Failure::new(
            FailureKind::Offline,
            "A test does not download the helper.",
        ))
    }
    #[cfg(not(test))]
    {
        crate::offline::check(NAME)?;
        // A client of its own: a gigabyte is not a chat, and the timeouts that
        // suit a helper's first word would cut this off every time.
        let http = crate::http::Http::for_download(MODEL.weights.url);
        download(&http, &MODEL, cache, stop, progress)
    }
}

pub(crate) fn download(
    http: &crate::http::Http,
    model: &Model,
    cache: &Path,
    stop: &StopFlag,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), Failure> {
    let folder = folder(cache);
    std::fs::create_dir_all(&folder).map_err(|why| cannot(&folder, why))?;
    // **One download at a time, whichever window started it.** Calx and
    // Scriva share this folder; two of them writing one part file interleave
    // their bytes and then rename it out from under each other. The lock is a
    // file nobody else can have made, and it goes when the download does.
    let _lock = Lock::taken(&folder)?;
    let total = model.bytes();
    let mut done = 0u64;
    for piece in model.pieces() {
        let whole = folder.join(piece.file);
        if std::fs::metadata(&whole).is_ok_and(|found| found.len() == piece.bytes) {
            done += piece.bytes;
            progress(Progress { done, total });
            continue;
        }
        fetch(http, &piece, &folder, stop, &mut |got| {
            progress(Progress {
                done: done + got,
                total,
            })
        })?;
        done += piece.bytes;
    }
    Ok(())
}

/// One file: resumed where it stopped, checked against its hash, and put in
/// place under its own name only once it is whole and right.
///
/// **What a server sends is never trusted for its length.** The constant says
/// how big the file is, so anything past that is a mirror serving something
/// else, a proxy serving its login page, or a file that has changed: it is
/// stopped at the first chunk that goes over rather than written to the end
/// of the disk and hashed afterwards.
fn fetch(
    http: &crate::http::Http,
    piece: &Piece,
    folder: &Path,
    stop: &StopFlag,
    progress: &mut dyn FnMut(u64),
) -> Result<(), Failure> {
    let part = folder.join(format!("{}.part", piece.file));
    let have = std::fs::metadata(&part)
        .map(|found| found.len())
        .unwrap_or(0);
    // More than the file is: something else is there, and starting again is
    // the only thing that can be right.
    let have = match have >= piece.bytes {
        true => {
            let _ = std::fs::remove_file(&part);
            0
        }
        false => have,
    };
    let mut headers: Vec<(&str, String)> = Vec::new();
    if have > 0 {
        headers.push(("range", format!("bytes={have}-")));
    }
    let response = http.get(piece.url, &headers, NAME)?;
    // 206 is the range honoured — but only where the server says which bytes
    // it is sending: one that answers 206 and starts again from nothing would
    // otherwise be appended to what is already here, and a gigabyte thrown
    // away at the hash. 200 is the whole file, whatever was asked.
    let from = response.range_from();
    let resumed = response.status == 206 && from == Some(have);
    if response.status != 200 && response.status != 206 {
        let status = response.status;
        // The server says the range cannot be satisfied: what is on disk is
        // not part of the file it has, so it goes rather than being asked
        // about for ever.
        if status == 416 {
            let _ = std::fs::remove_file(&part);
        }
        return Err(Failure::new(
            FailureKind::Rejected { status },
            format!(
                "{} could not be downloaded: the server answered {status}. {}",
                piece.file,
                response.error_message()
            ),
        ));
    }
    let have = match resumed {
        true => have,
        false => 0,
    };
    let mut reader = response.body;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(have > 0)
        .write(true)
        .truncate(have == 0)
        .open(&part)
        .map_err(|why| cannot(&part, why))?;
    let mut at = have;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        if stop.is_set() {
            // Stop is the person's, not a fault: what has arrived stays in
            // the part file, and the next attempt carries on from it.
            return Err(Failure::new(
                FailureKind::Dropped,
                "The download stopped. What arrived is kept: starting it again carries on \
                 from there.",
            ));
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|why| cannot(Path::new(piece.url), why))?;
        if read == 0 {
            break;
        }
        if at + read as u64 > piece.bytes {
            // More than the file is: whatever is being served, it is not
            // what was asked for, and the rest of it is not written.
            let _ = std::fs::remove_file(&part);
            return Err(Failure::new(
                FailureKind::Garbled,
                format!(
                    "What is being served as {} is longer than it should be ({} bytes), so \
                     the download was stopped and what had arrived removed. Try again.",
                    piece.file, piece.bytes
                ),
            ));
        }
        file.write_all(&buffer[..read])
            .map_err(|why| cannot(&part, why))?;
        at += read as u64;
        progress(at);
    }
    file.flush().map_err(|why| cannot(&part, why))?;
    drop(file);
    let found = hash_of(&part).map_err(|why| cannot(&part, why))?;
    if found != piece.sha256 {
        // Not what was asked for: a mirror's mistake, a truncated file, or a
        // proxy's login page. It is removed rather than left to be run.
        let _ = std::fs::remove_file(&part);
        return Err(Failure::new(
            FailureKind::Garbled,
            format!(
                "What arrived is not {}: its checksum does not match the one Officina \
                 expects, so it was removed. Try the download again.",
                piece.file
            ),
        ));
    }
    std::fs::rename(&part, folder.join(piece.file)).map_err(|why| cannot(&part, why))?;
    Ok(())
}

/// The one download's claim on the folder, given up when it is dropped.
struct Lock(PathBuf);

impl Lock {
    fn taken(folder: &Path) -> Result<Lock, Failure> {
        let path = folder.join("downloading");
        // Stale after a day: a window that died mid-download left it, and a
        // person should not have to find a file to download again.
        if let Ok(found) = std::fs::metadata(&path) {
            let old = found
                .modified()
                .ok()
                .and_then(|when| when.elapsed().ok())
                .is_some_and(|since| since > std::time::Duration::from_secs(24 * 60 * 60));
            if old {
                let _ = std::fs::remove_file(&path);
            }
        }
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                Ok(Lock(path))
            }
            Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => Err(Failure::new(
                FailureKind::Busy,
                "The helper is already being downloaded, in this window or another one.",
            )),
            Err(why) => Err(cannot(&path, why)),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn hash_of(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn cannot(what: &Path, why: std::io::Error) -> Failure {
    Failure::new(
        FailureKind::Offline,
        format!("The download could not go on: {} ({why}).", what.display()),
    )
}

// --------------------------------------------------------------- the runtime

/// The model, read and ready to answer.
struct Ready {
    model: ModelWeights,
    tokenizer: Tokenizer,
    device: Device,
    /// The tokens that end an answer: the template's `<|im_end|>`, and the
    /// model's own end-of-text.
    ends: Vec<u32>,
    /// How many tokens the model was trained to hold at once. A model asked
    /// for one more than it has room for does not fail politely — it reaches
    /// past the end of its own tables — so the request and the answer
    /// together are kept inside it.
    context: usize,
}

/// The model read from disk, kept for as long as the helper is the chosen
/// one.
///
/// **Reading it is a gigabyte and a second or two.** A helper is made afresh
/// whenever a request begins after a Stop, after the settings are saved, or
/// after the conversation is cleared — every one of which would otherwise
/// read the whole file again and dequantize its embedding table again, which
/// is the slowest thing this feature does. It is let go of by [`forget`],
/// when the weights are removed or another helper is chosen.
static READ: Mutex<BTreeMap<PathBuf, Arc<Mutex<Ready>>>> = Mutex::new(BTreeMap::new());

/// Lets go of every model held in memory.
pub fn forget() {
    READ.lock().unwrap_or_else(|held| held.into_inner()).clear();
}

/// The helper on this computer: the model, shared with whatever else has it
/// open, and used one request at a time.
pub struct Local {
    ready: Arc<Mutex<Ready>>,
}

impl Local {
    /// Reads the model in `folder`. A minute on a cold cache, and the window
    /// is not held: the pane asks on the request's own thread.
    pub fn load(folder: &Path) -> Result<Local, Failure> {
        let weights = folder.join(MODEL.weights.file);
        let tokenizer = folder.join(MODEL.tokenizer.file);
        // Half a download is not a helper: whichever file is missing, the
        // answer is the same one, and it says what to do about it.
        for piece in MODEL.pieces() {
            if !folder.join(piece.file).exists() {
                return Err(Failure::new(
                    FailureKind::NotReady,
                    format!(
                        "The helper on this computer is not all there: {} is missing or only \
                         half downloaded. Assist ▸ Settings downloads it again.",
                        piece.file
                    ),
                ));
            }
        }
        let mut file = std::fs::File::open(&weights).map_err(|why| {
            Failure::new(
                FailureKind::NotReady,
                format!(
                    "The helper on this computer has not been downloaded yet ({why}). \
                     Assist ▸ Settings downloads it."
                ),
            )
        })?;
        // Read once: the same folder read again is the model already in
        // memory, which is what makes Stop cost nothing.
        let mut held = READ.lock().unwrap_or_else(|held| held.into_inner());
        if let Some(ready) = held.get(folder) {
            return Ok(Local {
                ready: Arc::clone(ready),
            });
        }
        let local = Local::read(&mut file, &tokenizer)?;
        held.insert(folder.to_path_buf(), Arc::clone(&local.ready));
        Ok(local)
    }

    /// The same, from anything that reads and seeks — which is what lets a
    /// test hand it a model of its own making.
    pub fn read<R: Read + Seek>(weights: &mut R, tokenizer: &Path) -> Result<Local, Failure> {
        let tokenizer = Tokenizer::from_file(tokenizer)
            .map_err(|why| garbled(format!("its tokenizer could not be read ({why})")))?;
        Local::with(weights, tokenizer)
    }

    /// The same again, with a tokenizer already in hand.
    pub fn with<R: Read + Seek>(weights: &mut R, tokenizer: Tokenizer) -> Result<Local, Failure> {
        let device = Device::Cpu;
        let content = gguf_file::Content::read(weights)
            .map_err(|why| garbled(format!("its weights could not be read ({why})")))?;
        let context = content
            .metadata
            .get("qwen3.context_length")
            .and_then(|value| value.to_u32().ok())
            .unwrap_or(4_096) as usize;
        let model = ModelWeights::from_gguf(content, weights, &device)
            .map_err(|why| garbled(format!("its weights are not a model Officina runs ({why})")))?;
        let ends = ["<|im_end|>", "<|endoftext|>"]
            .iter()
            .filter_map(|word| tokenizer.token_to_id(word))
            .collect();
        Ok(Local {
            ready: Arc::new(Mutex::new(Ready {
                model,
                tokenizer,
                device,
                ends,
                context,
            })),
        })
    }

    /// Generates, with the model locked for as long as it takes: one request
    /// at a time, which is all a person makes.
    fn generate(
        &self,
        prompt: &str,
        stop: &StopFlag,
        text: &mut dyn FnMut(&str),
    ) -> Result<(String, Ending, Usage), Failure> {
        let mut ready = self.ready.lock().unwrap_or_else(|held| held.into_inner());
        ready.generate_inner(prompt, stop, text)
    }
}

impl Ready {
    /// Generates from `prompt`, handing words to `text` as they are made, and
    /// stopping at an end token, at [`MOST_TOKENS`], or when `stop` is set.
    fn generate_inner(
        &mut self,
        prompt: &str,
        stop: &StopFlag,
        text: &mut dyn FnMut(&str),
    ) -> Result<(String, Ending, Usage), Failure> {
        let encoded = self
            .tokenizer
            .encode(prompt, false)
            .map_err(|why| garbled(format!("the request could not be tokenized ({why})")))?;
        let mut tokens: Vec<u32> = encoded.get_ids().to_vec();
        let input = tokens.len();
        // **Every request starts the model afresh.** The keys and values of
        // the last answer are still in the model's cache, and a second
        // request read against them is a model answering a question nobody
        // asked — when it does not simply fail on the shapes. The whole
        // conversation is in the prompt, so nothing is lost by clearing it.
        self.model.clear_kv_cache();
        let mut said = String::new();
        // What has been handed to `text` so far: the answer without its tool
        // calls.
        let mut shown = String::new();
        let mut made: Vec<u32> = Vec::new();
        let mut ending = Ending::TooLong;
        if input >= self.context {
            return Err(Failure::new(
                FailureKind::Rejected { status: 0 },
                format!(
                    "The request is longer than the helper on this computer can hold \
                     ({input} of {} tokens). Ask about less of the document, or choose a \
                     helper with more room.",
                    self.context
                ),
            ));
        }
        for step in 0..MOST_TOKENS {
            if stop.is_set() {
                ending = Ending::Stopped;
                break;
            }
            if tokens.len() >= self.context {
                // The model has filled the room it was trained to hold.
                ending = Ending::TooLong;
                break;
            }
            // The first pass reads the whole prompt; after that, one token at
            // a time against the keys and values already worked out.
            let (window, offset) = match step {
                0 => (&tokens[..], 0),
                _ => (&tokens[tokens.len() - 1..], tokens.len() - 1),
            };
            let input_tensor = Tensor::new(window, &self.device)
                .and_then(|t| t.unsqueeze(0))
                .map_err(|why| garbled(format!("the model would not take the request ({why})")))?;
            let logits = self
                .model
                .forward(&input_tensor, offset)
                .map_err(|why| garbled(format!("the model stopped part way ({why})")))?;
            let next = last_token(&logits)
                .map_err(|why| garbled(format!("the model's answer could not be read ({why})")))?;
            if self.ends.contains(&next) {
                ending = Ending::Finished;
                break;
            }
            tokens.push(next);
            made.push(next);
            // Decoded from the whole answer each time: a token is often half
            // a letter, and decoding one at a time turns "é" into rubbish.
            said = self.tokenizer.decode(&made, true).unwrap_or_default();
            if let Some(fresh) = next_words(&said, &shown) {
                text(&fresh);
                shown.push_str(&fresh);
            }
        }
        let usage = Usage {
            input: input as u64,
            output: made.len() as u64,
            ..Usage::default()
        };
        Ok((said, ending, usage))
    }
}

#[cfg(test)]
impl Local {
    /// The first token this model makes of `prompt` — what a test needs to
    /// prove that an end token ends the answer, whatever a model of arbitrary
    /// weights happens to say.
    fn said_first(&self, prompt: &str) -> Vec<u32> {
        let mut ready = self.ready.lock().unwrap_or_else(|held| held.into_inner());
        ready.model.clear_kv_cache();
        let encoded = ready.tokenizer.encode(prompt, false).expect("encoded");
        let tokens: Vec<u32> = encoded.get_ids().to_vec();
        let input = Tensor::new(&tokens[..], &ready.device)
            .and_then(|t| t.unsqueeze(0))
            .expect("a tensor");
        let logits = ready.model.forward(&input, 0).expect("a forward pass");
        vec![last_token(&logits).expect("a token")]
    }

    /// Whether two helpers are the same model in memory — the rule that the
    /// weights are read once, as a test can see it.
    fn is_the_same_model_as(&self, other: &Local) -> bool {
        Arc::ptr_eq(&self.ready, &other.ready)
    }

    /// The tokens that end an answer, for a test to set to what this model
    /// actually says.
    fn ends_at(&self, tokens: Vec<u32>) {
        self.ready
            .lock()
            .unwrap_or_else(|held| held.into_inner())
            .ends = tokens;
    }
}

/// What to hand to the transcript, given the whole answer so far and what
/// has been handed over already.
///
/// **What the person reads is not what the model writes.** A tool call is
/// `<tool_call>{…}</tool_call>` in the very text the model is making, and
/// every other helper keeps its calls out of its words: what goes to the
/// transcript is the answer with the calls taken out, and nothing at all
/// while one is being written.
///
/// **And a letter can arrive in two halves.** A token is often part of a
/// character, so the whole answer is decoded afresh each time; what was
/// shown as `caf<?>` becomes `café` when the second half lands, and what is
/// handed over is the difference from where the two first differ — never
/// nothing, which would leave the transcript wrong for good.
fn next_words(said: &str, shown: &str) -> Option<String> {
    let visible = words_of(said);
    if visible == shown {
        return None;
    }
    if let Some(fresh) = visible.strip_prefix(shown) {
        return (!fresh.is_empty()).then(|| fresh.to_owned());
    }
    let same = visible
        .char_indices()
        .zip(shown.chars())
        .take_while(|((_, a), b)| a == b)
        .map(|((at, c), _)| at + c.len_utf8())
        .last()
        .unwrap_or(0);
    // What was shown and is now wrong cannot be taken back from a transcript
    // that has already drawn it; what is added is the rest of the answer from
    // where it stopped being true.
    Some(visible[same..].to_owned())
}

/// The answer as a person reads it: the words, with every tool call taken
/// out — and with anything after an unfinished `<tool_call>` held back,
/// since it is being written and is not words.
fn words_of(said: &str) -> String {
    let mut out = String::new();
    let mut rest = said;
    while let Some(open) = rest.find("<tool_call>") {
        let (before, after) = rest.split_at(open);
        out.push_str(before);
        let after = &after["<tool_call>".len()..];
        match after.find("</tool_call>") {
            Some(close) => rest = &after[close + "</tool_call>".len()..],
            // Still being written: nothing after it is words yet.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The most likely token of the last position of a model's answer.
fn last_token(logits: &Tensor) -> candle_core::Result<u32> {
    let logits = logits.squeeze(0)?;
    let logits = match logits.dims().len() {
        2 => logits.get(logits.dim(0)? - 1)?,
        _ => logits,
    };
    logits.argmax(candle_core::D::Minus1)?.to_scalar::<u32>()
}

fn garbled(why: String) -> Failure {
    Failure::new(
        FailureKind::Garbled,
        format!("The helper on this computer could not answer: {why}."),
    )
}

impl Provider for Local {
    fn name(&self) -> &str {
        NAME
    }

    fn answer(&mut self, request: &Request, stop: &StopFlag, text: &mut dyn FnMut(&str)) -> Answer {
        if stop.is_set() {
            return Answer::ended(Vec::new(), Ending::Stopped, Usage::default());
        }
        let prompt = prompt_for(request);
        match self.generate(&prompt, stop, text) {
            Err(failure) => Answer::failed(failure),
            Ok((said, ending, usage)) => {
                let (blocks, wants) = read_answer(&said);
                let ending = match (ending, wants) {
                    (Ending::Finished, true) => Ending::WantsTools,
                    (ending, _) => ending,
                };
                Answer::ended(blocks, ending, usage)
            }
        }
    }
}

// -------------------------------------------------------------- the template

/// The request as the model reads it: Qwen3's chat template, with thinking
/// off.
///
/// **A model is trained on one shape of prompt.** Qwen3's is `<|im_start|>`
/// and `<|im_end|>` around each turn, the tools as JSON in the system turn,
/// and a call as `<tool_call>` JSON `</tool_call>`. Thinking is turned off by
/// opening the answer with an empty `<think>` block, which is what the
/// template itself does when it is asked for: the words are for the person,
/// and a small model's thinking costs seconds it does not have.
pub fn prompt_for(request: &Request) -> String {
    let mut out = String::new();
    out.push_str("<|im_start|>system\n");
    out.push_str(request.system.trim());
    if !request.tools.is_empty() {
        out.push_str(
            "\n\n# Tools\n\nYou may call one or more functions to assist with the \
                      user query.\n\nYou are provided with function signatures within \
                      <tools></tools> XML tags:\n<tools>\n",
        );
        for tool in request.tools {
            out.push_str(&described(tool).to_string());
            out.push('\n');
        }
        out.push_str(
            "</tools>\n\nFor each function call, return a json object with function \
                      name and arguments within <tool_call></tool_call> XML tags:\n\
                      <tool_call>\n{\"name\": <function-name>, \"arguments\": \
                      <args-json-object>}\n</tool_call>",
        );
    }
    out.push_str("<|im_end|>\n");
    for message in request.conversation.messages() {
        let who = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let mut body = String::new();
        for block in &message.content {
            match block {
                Block::Text(words) => body.push_str(words),
                Block::ToolCall(call) => body.push_str(&format!(
                    "\n<tool_call>\n{}\n</tool_call>",
                    json!({"name": call.name, "arguments": call.input})
                )),
                Block::ToolResult(result) => body.push_str(&format!(
                    "\n<tool_response>\n{}\n</tool_response>",
                    result.content
                )),
                Block::Opaque { .. } => {}
            }
        }
        out.push_str(&format!("<|im_start|>{who}\n{}<|im_end|>\n", body.trim()));
    }
    out.push_str("<|im_start|>assistant\n<think>\n\n</think>\n\n");
    out
}

/// A tool as the template describes it.
fn described(tool: &Tool) -> Value {
    json!({"type": "function", "function": {
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.schema,
    }})
}

/// What the model said, as blocks: its words, and the tools it asked for.
///
/// **A tool call is a shape in the text, not a field in a reply.** The model
/// writes `<tool_call>{…}</tool_call>`; everything outside those tags is what
/// the person reads. A call whose JSON will not parse is left as the text it
/// is rather than guessed at — the helper is asked again, and the person sees
/// what it wrote.
pub fn read_answer(said: &str) -> (Vec<Block>, bool) {
    let mut blocks = Vec::new();
    let mut words = String::new();
    let mut rest = said;
    let mut calls = 0;
    while let Some(open) = rest.find("<tool_call>") {
        let (before, after) = rest.split_at(open);
        let after = &after["<tool_call>".len()..];
        let Some(close) = after.find("</tool_call>") else {
            break;
        };
        let (inside, beyond) = after.split_at(close);
        let parsed: Option<Value> = serde_json::from_str(inside.trim()).ok();
        let call = parsed.as_ref().and_then(|value| {
            let name = value.get("name")?.as_str()?.to_owned();
            let input = value.get("arguments").cloned().unwrap_or_else(|| json!({}));
            Some((name, input))
        });
        match call {
            Some((name, input)) => {
                words.push_str(before);
                calls += 1;
                if !words.trim().is_empty() {
                    blocks.push(Block::Text(words.trim().to_owned()));
                    words.clear();
                }
                blocks.push(Block::ToolCall(ToolCall {
                    id: format!("call_{calls}"),
                    name,
                    input,
                }));
            }
            // Not a call after all: the tags and what is between them are
            // words like any others.
            None => {
                words.push_str(before);
                words.push_str("<tool_call>");
                words.push_str(inside);
                words.push_str("</tool_call>");
            }
        }
        rest = &beyond["</tool_call>".len()..];
    }
    words.push_str(rest);
    if !words.trim().is_empty() {
        blocks.push(Block::Text(words.trim().to_owned()));
    }
    (blocks, calls > 0)
}

#[cfg(test)]
mod tests;
