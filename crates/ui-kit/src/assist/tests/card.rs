//! B3: the first-run card.

use ::assist::{ClaudeLogin, Installed};

use super::*;

fn installed(name: &str, bytes: u64) -> Installed {
    Installed {
        name: name.to_owned(),
        bytes: Some(bytes),
        elsewhere: None,
    }
}

/// Ollama with a small model, a middling one, and a large one pulled last.
fn ollama_row() -> Row {
    Row::OllamaHere {
        models: vec![
            installed("qwen3:1.7b", 1_359_293_444),
            installed("llama3.2:latest", 2_019_393_189),
            installed("qwen3.6:27b-q5_k_m", 19_231_100_101),
        ],
    }
}

fn always() -> Vec<Row> {
    vec![
        Row::Local(::assist::local::MODELS[0]),
        Row::ClaudeWithKey,
        Row::Service,
    ]
}

fn with_first(first: Vec<Row>) -> Vec<Row> {
    first.into_iter().chain(always()).collect()
}

/// The pane with no helper chosen, finding `rows`, its card on the screen.
fn card(rows: Vec<Row>, scratch: &Scratch) -> (Driver, Desk, Arc<Fake>) {
    let reach = Fake::new().finds(rows);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.settings());
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    drive.settle(&mut desk);
    (drive, desk, reach)
}

/// The same, with a reach of the test's own — for what the rows do rather
/// than what they say.
fn card_with(reach: Arc<Fake>, scratch: &Scratch) -> (Driver, Desk, Arc<Fake>) {
    *reach.rows.lock().unwrap() = always();
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.settings());
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    drive.settle(&mut desk);
    (drive, desk, reach)
}

/// The title of the row painted on the lit fill: the one preselected.
fn lit_row(desk: &mut Desk, drive: &Driver, rows: &[Row]) -> String {
    let painted = desk.painted(drive);
    let lit = painted.filled(theme::TINT_ON);
    let titles: Vec<String> = rows.iter().map(Row::title).collect();
    let on: Vec<String> = painted
        .texts()
        .into_iter()
        .filter(|text| titles.contains(&text.text))
        .filter(|text| lit.iter().any(|fill| fill.contains_rect(text.rect)))
        .map(|text| text.text)
        .collect();
    assert_eq!(on.len(), 1, "one row is lit: {on:?}");
    on[0].clone()
}

#[test]
fn the_first_run_card_preselects_the_first_thing_found_and_the_local_helper_when_nothing_is() {
    // While the computer is looked at, the card says so.
    let scratch = Scratch::new("card-nothing");
    let reach = Fake::new().finds(always());
    let go = reach.look_waits();
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.settings());
    for _ in 0..3 {
        drive.settle(&mut desk);
    }
    let seen = desk.seen(&drive);
    assert!(seen.iter().any(|text| text == QUESTION), "{seen:?}");
    assert!(
        seen.iter()
            .any(|text| text == "Looking at what this computer already has…"),
        "{seen:?}"
    );
    go.send(()).unwrap();
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });

    // Nothing found: the helper on this computer, first and lit, named for
    // the model the computer was judged able to run.
    assert_eq!(
        lit_row(&mut desk, &drive, &always()),
        "A helper on this computer (Qwen3 4B)"
    );
    let seen = desk.seen(&drive);
    for words in [
        "Use this",
        "Not now",
        "Claude, over the internet",
        "Another service (advanced)",
    ] {
        assert!(seen.iter().any(|text| text == words), "{words}: {seen:?}");
    }
    assert!(
        seen.iter().any(|text| text == "Choose a helper first")
            || desk.assist.composer().is_empty(),
        "the composer waits for a helper"
    );
    desk.click(&drive, "Not now");
    assert_eq!(desk.chosen, [Chosen::Close], "Not now closes the pane");
    assert!(
        !scratch.settings().exists(),
        "and nothing was chosen, so the card is back next time"
    );

    // Opened again, the pane looks again: Ollama has been started meanwhile.
    desk.open = false;
    drive.settle(&mut desk);
    drive.settle(&mut desk);
    reach.finds(with_first(vec![ollama_row()]));
    desk.open = true;
    desk.until(&drive, "the new look", |desk| {
        matches!(&desk.assist.choosing, Some(Choosing::Rows(rows))
            if matches!(rows.rows[0], Row::OllamaHere { .. }))
    });

    // By the keyboard alone: the pane given the keys while it looks, and
    // Enter once the rows are there, chooses what is preselected.
    let scratch = Scratch::new("card-keys");
    let reach = Fake::new().finds(with_first(vec![ollama_row()]));
    let go = reach.look_waits();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.settings());
    desk.assist.focus();
    for _ in 0..3 {
        drive.settle(&mut desk);
    }
    go.send(()).unwrap();
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    drive.settle(&mut desk);
    drive.settle(&mut desk);
    drive.press(&mut desk, "Enter");
    drive.settle(&mut desk);
    assert_eq!(
        Settings::read(&scratch.settings()).unwrap().helper,
        Some(Choice::Ollama),
        "Enter chose the preselected row"
    );
    drive.settle(&mut desk);
    assert!(
        desk.assist.holds_keyboard(drive.ctx()),
        "and the composer has the keyboard for the first request"
    );

    // Ollama found: its row first and lit, and choosing it keeps it.
    let scratch = Scratch::new("card-ollama");
    let rows = with_first(vec![ollama_row()]);
    let (drive, mut desk, _) = card(rows.clone(), &scratch);
    assert_eq!(
        lit_row(&mut desk, &drive, &rows),
        "Ollama on this computer (qwen3:1.7b)"
    );
    desk.click(&drive, "Use this");
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(kept.helper, Some(Choice::Ollama));
    assert_eq!(kept.ollama.model, "qwen3:1.7b");
    assert!(desk.assist.choosing.is_none(), "the card is gone");
    let seen = desk.seen(&drive);
    assert!(
        seen.iter()
            .any(|text| text == "Ollama — qwen3:1.7b · on this computer"),
        "the header names the helper in words: {seen:?}"
    );
    assert_eq!(
        desk.assist.transcript(),
        [Entry::Note {
            sentence: "Assist will use Ollama — qwen3:1.7b, on this computer.".into(),
            action: None
        }]
    );
    assert!(
        seen.iter()
            .any(|text| text == "Say what you want done, in your own words"),
        "the composer is ready: {seen:?}"
    );
    assert!(desk.assist.holds_keyboard(drive.ctx()));

    // A login found: Claude first, over Ollama.
    let scratch = Scratch::new("card-ant");
    let rows = with_first(vec![Row::ClaudeHere(ClaudeLogin::Ant), ollama_row()]);
    let (drive, mut desk, _) = card(rows.clone(), &scratch);
    assert_eq!(
        lit_row(&mut desk, &drive, &rows),
        "Claude, with the login on this computer"
    );
    desk.click(&drive, "Use this");
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(kept.helper, Some(Choice::Claude));
    assert_eq!(kept.claude.login, ClaudeLogin::Ant);
    let seen = desk.seen(&drive);
    assert!(
        seen.iter().any(|text| text == "Claude — Opus 5"),
        "Opus 5 by default, and not on this computer: {seen:?}"
    );

    // Another row picked by the pointer is the one chosen.
    let scratch = Scratch::new("card-pick");
    let rows = with_first(vec![
        Row::ClaudeHere(ClaudeLogin::Environment),
        ollama_row(),
    ]);
    let (drive, mut desk, _) = card(rows.clone(), &scratch);
    desk.click(&drive, "Ollama on this computer (qwen3:1.7b)");
    assert_eq!(
        lit_row(&mut desk, &drive, &rows),
        "Ollama on this computer (qwen3:1.7b)"
    );
    desk.click(&drive, "Use this");
    assert_eq!(
        Settings::read(&scratch.settings()).unwrap().helper,
        Some(Choice::Ollama)
    );
}

#[test]
fn the_card_says_model_llm_token_endpoint_and_gpu_nowhere() {
    let banned = ["model", "llm", "token", "inference", "endpoint", "gpu"];
    let check = |seen: &[String], what: &str| {
        for text in seen {
            let lower = text.to_lowercase();
            for word in banned {
                assert!(!lower.contains(word), "{what}: “{text}” says {word}");
            }
        }
    };

    // Every row the card can have, with each sign-in Claude can be found by.
    for first in [
        vec![Row::ClaudeHere(ClaudeLogin::Environment), ollama_row()],
        vec![Row::ClaudeHere(ClaudeLogin::Ant), ollama_row()],
        vec![],
    ] {
        let scratch = Scratch::new("banned-words");
        let (drive, mut desk, _) = card(with_first(first), &scratch);
        desk.verbs = vec![("Improve the wording", "Improve the wording.")];
        let seen = desk.seen(&drive);
        assert!(seen.len() > 8, "the card is on the screen: {seen:?}");
        check(&seen, "the card");

        // And what choosing the helper that is not ready says.
        desk.click(&drive, "Use this");
        let seen = desk.seen(&drive);
        check(&seen, "the card, after choosing");
    }

    // While the computer is looked at.
    let scratch = Scratch::new("banned-looking");
    let reach = Fake::new();
    let _hold = reach.look_waits();
    let drive = Driver::new();
    let mut desk = Desk::with(reach, scratch.settings());
    let seen = desk.seen(&drive);
    assert!(seen.iter().any(|text| text == QUESTION));
    check(&seen, "the card, looking");

    // And the card for settings that could not be read.
    let scratch = Scratch::new("banned-unreadable");
    std::fs::write(scratch.settings(), "helper = [").unwrap();
    let mut desk = Desk::with(Fake::new(), scratch.settings());
    let seen = desk.seen(&drive);
    assert!(seen
        .iter()
        .any(|text| text == "Assist's settings could not be read"));
    check(
        &seen
            .into_iter()
            // The parser's own words are the file's, not the card's.
            .filter(|text| !text.starts_with("TOML"))
            .collect::<Vec<_>>(),
        "the card, unreadable",
    );
}

#[test]
fn the_ollama_row_lists_every_model_and_says_which_are_large() {
    let scratch = Scratch::new("ollama-models");
    let rows = with_first(vec![ollama_row()]);
    let (drive, mut desk, _) = card(rows, &scratch);
    let seen = desk.seen(&drive);
    for words in [
        "Ollama on this computer (qwen3:1.7b)",
        "qwen3:1.7b · 1.4 GB",
        "llama3.2:latest · 2.0 GB",
        "qwen3.6:27b-q5_k_m · 19 GB — large: needs a powerful computer",
    ] {
        assert!(seen.iter().any(|text| text == words), "{words}: {seen:?}");
    }
    assert_eq!(
        seen.iter().filter(|text| text.contains("large")).count(),
        1,
        "only the large one says so"
    );

    // Picking a model of Ollama's is choosing Ollama with it.
    desk.click(
        &drive,
        "qwen3.6:27b-q5_k_m · 19 GB — large: needs a powerful computer",
    );
    desk.click(&drive, "Use this");
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(kept.helper, Some(Choice::Ollama));
    assert_eq!(kept.ollama.model, "qwen3.6:27b-q5_k_m");

    // The settings box lists them too, looking for itself in a window that
    // has not looked yet.
    let scratch = Scratch::new("ollama-models-box");
    let reach = Fake::new().finds(with_first(vec![ollama_row()]));
    let go = reach.look_waits();
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&ollama_here()));
    drive.settle(&mut desk);
    desk.assist.open_settings();
    desk.until_seen(&drive, "Looking for Ollama's models…");
    go.send(()).unwrap();
    desk.until_seen(
        &drive,
        "qwen3.6:27b-q5_k_m · 19 GB — large: needs a powerful computer",
    );
    let seen = desk.everywhere(&drive);
    assert!(
        seen.iter().any(|text| text == "qwen3:1.7b · 1.4 GB"),
        "{seen:?}"
    );
    desk.click(&drive, "llama3.2:latest · 2.0 GB");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    assert_eq!(
        Settings::read(&scratch.settings()).unwrap().ollama.model,
        "llama3.2:latest"
    );
}

#[test]
fn the_local_row_says_what_it_will_download_before_it_downloads_anything() {
    let scratch = Scratch::new("local-row");
    let reach = Fake::new();
    let (drive, mut desk, _) = card_with(Arc::clone(&reach), &scratch);
    let seen = desk.seen(&drive);
    let said = seen
        .iter()
        .find(|text| text.starts_with("Free and private"))
        .unwrap_or_else(|| panic!("the local row's words: {seen:?}"));
    // The model, its licence, its size and the memory it will take, before
    // anything is downloaded.
    assert!(said.contains("Qwen3 4B"), "{said}");
    assert!(said.contains("Apache-2.0"), "{said}");
    assert!(said.contains("2.5 GB"), "{said}");
    assert!(said.contains("of memory"), "{said}");
    assert_eq!(
        reach.downloading.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "and nothing has been downloaded to say it"
    );

    // Chosen, the choice is kept — and the download begins, since the weights
    // are not there.
    desk.click(&drive, "Use this");
    desk.until(&drive, "the download to begin", |desk| {
        desk.assist.is_downloading()
    });
    assert_eq!(
        Settings::read(&scratch.settings()).unwrap().helper,
        Some(Choice::Local),
        "the choice was kept"
    );
    desk.assist.stop_download();
    desk.until(&drive, "the download to let go", |desk| {
        !desk.assist.is_downloading()
    });
}

/// A download that never answers does not hold the window: the frames go on,
/// the bar shows what has arrived, and Stop lets go of it.
#[test]
fn a_download_that_never_answers_does_not_hold_the_window() {
    let scratch = Scratch::new("download-waits");
    let reach = Fake::new();
    *reach.download_steps.lock().unwrap() = vec![
        ::assist::local::Progress {
            done: 1_000_000,
            total: 1_282_439_264,
        },
        ::assist::local::Progress {
            done: 500_000_000,
            total: 1_282_439_264,
        },
    ];
    let (drive, mut desk, _) = card_with(Arc::clone(&reach), &scratch);
    desk.click(&drive, "Use this");

    // The window keeps drawing while the download hangs: a frame takes a
    // moment, not a minute.
    desk.until(&drive, "the bar to show what arrived", |desk| {
        desk.assist
            .download_so_far()
            .is_some_and(|progress| progress.done >= 500_000_000)
    });
    let seen = desk.everywhere(&drive);
    assert!(
        seen.iter()
            .any(|text| text.contains("500 MB") && text.contains("1.3 GB")),
        "the bar says how much of how much: {seen:?}"
    );
    let started = std::time::Instant::now();
    for _ in 0..5 {
        drive.settle(&mut desk);
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "five frames took {:?}",
        started.elapsed()
    );
    assert!(desk.assist.is_downloading(), "and it is still going");

    // Stop lets go of it, and the bar goes with it.
    desk.click(&drive, "Stop");
    desk.until(&drive, "the download to let go", |desk| {
        !desk.assist.is_downloading()
    });
    let seen = desk.everywhere(&drive);
    assert!(
        !seen
            .iter()
            .any(|text| text.contains("Downloading the helper")),
        "{seen:?}"
    );
}

#[test]
fn settings_that_cannot_be_read_are_said_in_place_of_the_card_and_set_aside_when_a_helper_is_chosen(
) {
    let scratch = Scratch::new("unreadable");
    let garbage = "helper = [unterminated\n[claude]\nkey = \"sk-precious\"\n";
    std::fs::write(scratch.settings(), garbage).unwrap();
    let why = Settings::read(&scratch.settings()).unwrap_err();
    let reach = Fake::new().finds(with_first(vec![ollama_row()]));
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.settings());
    let seen = desk.seen(&drive);
    assert!(
        seen.iter()
            .any(|text| text == "Assist's settings could not be read"),
        "{seen:?}"
    );
    assert!(seen.contains(&why), "with the reason: {why} in {seen:?}");
    assert!(
        !seen.iter().any(|text| text == QUESTION),
        "in place of the card"
    );
    for _ in 0..5 {
        drive.settle(&mut desk);
    }
    assert!(
        reach.connected.lock().unwrap().is_empty(),
        "and nothing is asked"
    );

    // Mended by hand while the pane was closed, the file is read again when
    // it opens, and the card comes back.
    desk.open = false;
    drive.settle(&mut desk);
    drive.settle(&mut desk);
    std::fs::write(scratch.settings(), "theme = \"mine\"\n").unwrap();
    desk.open = true;
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    std::fs::write(scratch.settings(), garbage).unwrap();
    desk.open = false;
    drive.settle(&mut desk);
    drive.settle(&mut desk);
    desk.open = true;
    let seen = desk.seen(&drive);
    assert!(
        seen.iter()
            .any(|text| text == "Assist's settings could not be read"),
        "and when it is spoiled again, says so: {seen:?}"
    );

    desk.click(&drive, "Choose Again");
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    desk.click(&drive, "Use this");
    let kept = Settings::read(&scratch.settings()).expect("a file that can be read");
    assert_eq!(kept.helper, Some(Choice::Ollama));
    let aside = std::fs::read_to_string(scratch.0.join("assist.toml.unreadable")).unwrap();
    assert_eq!(aside, garbage, "the old file is kept, as it was");
    assert!(
        desk.assist.transcript().contains(&Entry::Note {
            sentence: "The settings that could not be read were kept as assist.toml.unreadable."
                .into(),
            action: None
        }),
        "{:?}",
        desk.assist.transcript()
    );
}

/// A computer below the floor: the row that says why is first and cannot be
/// chosen, so the card lights the first row that can, and pressing Use this
/// on the other says the sentence again rather than pointing at a download
/// Settings will not offer.
#[test]
fn a_computer_that_cannot_run_a_helper_has_the_first_choosable_row_lit() {
    let scratch = Scratch::new("card-no-local");
    let because = "This computer has 8.0 GB of memory; a helper worth having needs 8.5 GB to run beside your documents.";
    let rows = vec![
        Row::NoLocal {
            because: because.to_owned(),
        },
        Row::ClaudeWithKey,
        Row::Service,
    ];
    let reach = Fake::new().finds(rows.clone());
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.settings());
    drive.settle(&mut desk);
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    assert_eq!(
        lit_row(&mut desk, &drive, &rows),
        "Claude, over the internet"
    );
    let seen = desk.seen(&drive);
    assert!(seen.iter().any(|text| text == because), "{seen:?}");

    // Picked by hand and pressed: the sentence, and no settings written.
    desk.click(&drive, "No helper on this computer");
    desk.click(&drive, "Use this");
    drive.settle(&mut desk);
    let Some(Choosing::Rows(shown)) = &desk.assist.choosing else {
        panic!("the card stays up");
    };
    assert_eq!(shown.said.as_deref(), Some(because));
    assert!(!scratch.settings().exists(), "nothing was kept");
}
