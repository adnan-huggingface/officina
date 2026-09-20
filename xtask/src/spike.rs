//! `cargo xtask assist-spike` — how fast the helper on this computer is.
//!
//! **Hand-run, never in the gate.** It reads the weights a person's own
//! Officina would have downloaded and generates from them, printing how long
//! the model took to load, how many tokens a second it makes, and how much
//! memory the process grew by. Nothing in the test suite may do this — the
//! weights are a gigabyte and the tests must run on a machine that has never
//! seen them — so the measurement lives here, and what it measures is what
//! the first-run card is allowed to claim.

use std::path::PathBuf;
use std::time::Instant;

use assist::local::{self, Local, Sampling};
use assist::{Block, Conversation, Effort, Message, Provider, Request, StopFlag};

/// Runs the model and says how it went. An optional argument is a `.gguf`
/// file to read with `tokenizer.json` beside it, or the cache directory to
/// read the catalogue's smallest model from; the default is Officina's own. An
/// argument `--ask=<words>` is the request to make instead of the standard
/// one, for hearing how the model does at something a person asked;
/// `--scriva` sends it the way Scriva does, with Scriva's tools and an empty
/// document of one paragraph, so that what is heard is what a person would
/// see; and `--greedy` takes the likeliest token every time, as the first
/// release did, to hear the difference sampling makes.
pub fn run(args: &[String]) -> Result<(), String> {
    let (flags, folders): (Vec<&String>, Vec<&String>) =
        args.iter().partition(|arg| arg.starts_with("--"));
    let asks: Vec<&String> = flags
        .iter()
        .copied()
        .filter(|flag| flag.starts_with("--ask="))
        .collect();
    let as_scriva = flags.iter().any(|flag| *flag == "--scriva");
    let greedy = flags.iter().any(|flag| *flag == "--greedy");
    // A second request in the same process, after the first: what a session's
    // second request costs once the brief and the tools are already read.
    let again: Option<&str> = flags
        .iter()
        .find(|flag| flag.starts_with("--again="))
        .map(|flag| flag.trim_start_matches("--again="));
    // A flag mistyped is a condition not measured, and a difference that is
    // no difference would go into the record as fact.
    if let Some(unknown) = flags.iter().find(|flag| {
        !["--scriva", "--greedy"].contains(&flag.as_str())
            && !flag.starts_with("--ask=")
            && !flag.starts_with("--again=")
    }) {
        return Err(format!(
            "assist-spike does not know {unknown}: it takes a folder, --ask=<words>, \
             --again=<words>, --scriva and --greedy"
        ));
    }
    if asks.len() > 1 {
        return Err("assist-spike takes one --ask=".to_owned());
    }
    let folder = match folders.first() {
        Some(given) => PathBuf::from(given),
        None => ui_kit::paths::cache_dir(ui_kit::OFFICINA)
            .map_err(|why| format!("the cache directory: {why}"))?,
    };
    println!("Reading {}", folder.display());
    let started = Instant::now();
    // A `.gguf` given by name is read as it is, with `tokenizer.json` beside
    // it: how a candidate model is heard before it is in the catalogue.
    let mut local = match folder.extension().is_some_and(|ext| ext == "gguf") {
        true => {
            let mut file = std::fs::File::open(&folder).map_err(|why| why.to_string())?;
            let tokenizer = folder.with_file_name("tokenizer.json");
            Local::read(&mut file, &tokenizer).map_err(|failure| failure.sentence)?
        }
        // A folder is the cache directory, and the model the catalogue's
        // smallest — what a person's own Officina would run there.
        false => Local::load(&folder, &local::MODELS[0]).map_err(|failure| failure.sentence)?,
    };
    let read = started.elapsed();
    println!("  read in {:.1}s", read.as_secs_f64());

    let standard = "Rewrite this sentence so that it is plainer, and say nothing else: \
                    The thing about the situation is that it is one which we have to deal \
                    with in a manner that is timely.";
    let asked = match asks.first() {
        Some(ask) => ask.trim_start_matches("--ask="),
        None => standard,
    };
    if greedy {
        local.choosing(Sampling::ArgMax);
        println!("  choosing greedily");
    }
    let document = wp_model::doc::Document::blank();
    let asks: Vec<&str> = std::iter::once(asked).chain(again).collect();
    for (round, asked) in asks.iter().enumerate() {
        let asked = *asked;
        if round > 0 {
            println!("Again, in the same process: {asked:?}");
        }
        let sent = match as_scriva {
            true => scriva::assistant::request(
                &document,
                scriva::edit::Selection::default(),
                scriva::assistant::About::Paragraph,
                asked,
                0,
            ),
            false => asked.to_owned(),
        };
        let tools = match as_scriva {
            true => scriva::assistant::tools(),
            false => Vec::new(),
        };
        let mut conversation = Conversation::default();
        conversation.push(Message::user(&sent));
        let system = assist::prompt::scriva();
        let request = Request {
            system: &system,
            tools: &tools,
            conversation: &conversation,
            effort: Effort::Low,
        };
        let started = Instant::now();
        let mut said = String::new();
        let mut first: Option<std::time::Duration> = None;
        let answer = local.answer(&request, &StopFlag::default(), &mut |words| {
            first.get_or_insert_with(|| started.elapsed());
            said.push_str(words)
        });
        let took = started.elapsed();
        // Reading the request and writing the answer are different speeds, and
        // only the second is what a person watches: the first word tells them
        // apart.
        if let Some(first) = first {
            println!(
                "  first word after {:.1}s ({} tokens of request)",
                first.as_secs_f64(),
                answer.usage.input
            );
            let after = (took - first).as_secs_f64().max(0.001);
            println!(
                "  then {:.1} tokens a second",
                (answer.usage.output.saturating_sub(1)) as f64 / after
            );
        }
        match &answer.ending {
            Ok(ending) => println!("  ended: {ending:?}"),
            Err(failure) => return Err(failure.sentence.clone()),
        }
        let rate = answer.usage.output as f64 / took.as_secs_f64().max(0.001);
        println!(
            "  {} tokens in, {} out, {:.1}s — {rate:.1} tokens a second",
            answer.usage.input,
            answer.usage.output,
            took.as_secs_f64()
        );
        println!("  it said: {}", said.trim());
        for block in &answer.message.content {
            if let Block::ToolCall(call) = block {
                println!("  it called {}:", call.name);
                println!("{:#}", call.input);
            }
        }
    }
    if let Some(peak) = peak_memory() {
        println!("  memory: {} at its highest", local::size_of(peak));
    }
    Ok(())
}

/// How much memory this process has used at its highest, where the system
/// says: Linux keeps it in `/proc/self/status`.
fn peak_memory() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}
