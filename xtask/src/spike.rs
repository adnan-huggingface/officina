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

use assist::local::{self, Local};
use assist::{Conversation, Effort, Message, Provider, Request, StopFlag};

/// Runs the model and says how it went. An optional argument is the folder
/// the weights are in; the default is where Officina itself puts them.
pub fn run(args: &[String]) -> Result<(), String> {
    let folder = match args.first() {
        Some(given) => PathBuf::from(given),
        None => {
            let cache = ui_kit::paths::cache_dir(ui_kit::OFFICINA)
                .map_err(|why| format!("the cache directory: {why}"))?;
            local::folder(&cache)
        }
    };
    println!("Reading {}", folder.display());
    let started = Instant::now();
    let mut local = Local::load(&folder).map_err(|failure| failure.sentence)?;
    let read = started.elapsed();
    println!("  {} read in {:.1}s", local::MODEL.name, read.as_secs_f64());

    let asked = "Rewrite this sentence so that it is plainer, and say nothing else: \
                 The thing about the situation is that it is one which we have to deal \
                 with in a manner that is timely.";
    let mut conversation = Conversation::default();
    conversation.push(Message::user(asked));
    let system = assist::prompt::scriva();
    let request = Request {
        system: &system,
        tools: &[],
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
