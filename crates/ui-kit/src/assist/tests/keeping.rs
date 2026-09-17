//! B4 and B5: the settings box, and what is said before anything leaves the
//! computer.

use ::assist::{FailureKind, Turn};

use super::*;

impl Fake {
    /// The next check waits until the sender is used or dropped.
    pub fn check_waits(self: &Arc<Self>) -> mpsc::Sender<()> {
        let (go, gate) = mpsc::channel();
        *self.check_gate.lock().unwrap() = Some(gate);
        go
    }
}

impl Desk {
    /// Clicks the last painted text that is `words`: the one on the card at
    /// the transcript's end, when the composer has a button of that name too.
    pub fn click_last(&mut self, drive: &Driver, words: &str) {
        let painted = self.painted(drive);
        let text = painted
            .texts()
            .into_iter()
            .rev()
            .find(|text| text.text == words && text.shown().area() > 0.0)
            .unwrap_or_else(|| panic!("“{words}” is not on the screen: {:?}", painted.strings()));
        drive.click(self, text.shown().center());
    }

    /// Frames until the painted text `words` is on the screen.
    pub fn until_seen(&mut self, drive: &Driver, words: &str) {
        let started = Instant::now();
        loop {
            if self.everywhere(drive).iter().any(|text| text == words) {
                return;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "“{words}” never appeared: {:?}",
                self.everywhere(drive)
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
fn the_settings_box_offers_each_claude_model_with_its_cost_and_says_what_the_session_spent() {
    let scratch = Scratch::new("box-models");
    let cost = Usage {
        input: 2000,
        output: 500,
        ..Usage::default()
    };
    let scripted = Scripted::new([Turn::says("Done.").costs(cost)]);
    let reach = Fake::new().helpers(vec![Box::new(scripted)]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&claude_with_key()));
    drive.settle(&mut desk);

    desk.assist.open_settings();
    let seen = desk.everywhere(&drive);
    for words in [
        "Assist Settings",
        "Claude, over the internet",
        "Opus 5 (best) — about 2 cents a paragraph",
        "Sonnet 5 (faster, cheaper) — under a cent a paragraph",
        "Haiku 4.5 (cheapest) — under half a cent a paragraph",
        "If Opus 5 declines, let Anthropic's recommended model answer",
        "Nothing has been asked yet.",
    ] {
        assert!(seen.iter().any(|text| text == words), "{words}: {seen:?}");
    }
    assert!(
        desk.assist.holds_keyboard(drive.ctx()),
        "the box has the keyboard"
    );
    desk.click(&drive, "Cancel");
    assert!(!desk.assist.box_up());

    // A request that cost something: the box says what, in words and cents.
    desk.ask(&drive, "Tidy it.");
    desk.click_last(&drive, "Send");
    desk.finished(&drive);
    assert_eq!(desk.assist.spent(), cost);
    desk.assist.open_settings();
    let seen = desk.everywhere(&drive);
    assert!(
        seen.iter().any(|text| text
            == "The assistant read about 1,500 words and wrote about 380, for about 2 cents."),
        "{seen:?}"
    );

    // Another model is checked, then kept, with the key it had.
    desk.click(
        &drive,
        "Sonnet 5 (faster, cheaper) — under a cent a paragraph",
    );
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(kept.claude.model, "claude-sonnet-5");
    assert_eq!(kept.claude.key, "sk-ant-saved-key");
    let checked = reach.checked.lock().unwrap().clone();
    assert_eq!(checked.len(), 1, "a model not asked before is checked");
    assert_eq!(checked[0].claude.model, "claude-sonnet-5");
    let seen = desk.seen(&drive);
    assert!(
        seen.iter().any(|text| text == "Claude — Sonnet 5"),
        "{seen:?}"
    );

    // Saving what was already kept asks nothing.
    desk.assist.open_settings();
    desk.click(&drive, "Save");
    assert!(!desk.assist.box_up());
    assert_eq!(reach.checked.lock().unwrap().len(), 1);
}

#[test]
fn a_key_is_kept_only_once_the_service_has_accepted_it_and_is_never_painted() {
    const WRONG: &str = "sk-ant-wrong-key-1234";
    const RIGHT: &str = "sk-ant-right-key-5678";
    let scratch = Scratch::new("box-key");
    let refused = Failure::new(
        FailureKind::Unauthorized,
        "Claude did not accept the key. It said: “invalid x-api-key”",
    );
    let accepted = "Claude accepted the key, and Opus 5 is there to answer.";
    let reach = Fake::new()
        .finds(vec![Row::Local, Row::ClaudeWithKey, Row::Service])
        .checks(vec![Err(refused.clone()), Ok(accepted.to_owned())]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.settings());
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    let mut every_frame: Vec<String> = Vec::new();

    // Choosing Claude with a key asks for the key, and keeps nothing yet.
    desk.click(&drive, "Claude, over the internet");
    desk.click(&drive, "Use this");
    drive.settle(&mut desk);
    assert!(desk.assist.box_up(), "the key is asked for in the box");
    assert!(!scratch.settings().exists(), "nothing is kept yet");
    drive.type_text(&mut desk, WRONG);
    every_frame.extend(desk.everywhere(&drive));
    let go = reach.check_waits();
    desk.click(&drive, "Save");
    desk.until_seen(&drive, "Asking whether it answers to this…");
    every_frame.extend(desk.everywhere(&drive));
    go.send(()).unwrap();
    desk.until_seen(&drive, &refused.sentence);
    every_frame.extend(desk.everywhere(&drive));
    assert!(desk.assist.box_up(), "a key refused leaves the box up");
    assert!(!scratch.settings().exists(), "and is not kept");
    let checked = reach.checked.lock().unwrap().clone();
    assert_eq!(checked.len(), 1);
    assert_eq!(checked[0].helper, Some(Choice::Claude));
    assert_eq!(
        checked[0].claude.key, WRONG,
        "the key typed is the key checked"
    );
    assert_eq!(
        desk.painted(&drive).colour_of("did not accept the key"),
        Some(theme::INK_ERROR)
    );

    // The right key, typed over the wrong one, is kept once accepted.
    let dots = desk
        .painted(&drive)
        .texts()
        .into_iter()
        .find(|text| !text.text.is_empty() && text.text.chars().all(|c| c == '•'))
        .expect("the key's field shows dots");
    assert_eq!(dots.text.chars().count(), WRONG.len(), "one dot a letter");
    drive.click(&mut desk, dots.shown().center());
    drive.press(&mut desk, "ctrl+A");
    drive.type_text(&mut desk, RIGHT);
    every_frame.extend(desk.everywhere(&drive));
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    every_frame.extend(desk.everywhere(&drive));
    let kept = Settings::read(&scratch.settings()).expect("the settings were kept");
    assert_eq!(kept.helper, Some(Choice::Claude));
    assert_eq!(kept.claude.key, RIGHT);
    assert_eq!(reach.checked.lock().unwrap()[1].claude.key, RIGHT);
    assert!(
        desk.assist.transcript().contains(&Entry::Note {
            sentence: accepted.into(),
            action: None
        }),
        "what the service said is in the transcript: {:?}",
        desk.assist.transcript()
    );

    assert!(
        every_frame.len() > 20,
        "the frames were read: {every_frame:?}"
    );
    for text in &every_frame {
        for key in [WRONG, RIGHT] {
            assert!(!text.contains(key), "a key was painted: “{text}”");
            assert!(
                !text.contains(&key[7..]),
                "part of a key was painted: “{text}”"
            );
        }
    }
    assert!(!format!("{:?}", desk.assist.settings()).contains(RIGHT));
}

#[test]
fn the_first_request_to_a_helper_over_the_internet_says_what_will_be_sent_and_waits_for_send() {
    let scratch = Scratch::new("consent");
    let claude = Scripted::new([Turn::says("Shorter."), Turn::says("Shorter still.")]);
    let heard = claude.heard();
    let reach = Fake::new().helpers(vec![
        Box::new(claude),
        Box::new(Scripted::new([Turn::says("From the service.")])),
        Box::new(Scripted::new([Turn::says("From Ollama.")])),
    ]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&claude_with_key()));
    drive.settle(&mut desk);

    let anthropic = "The selected paragraph and the two around it will be sent to Anthropic. \
                     Assist asks once for each place your words may go.";
    desk.ask(&drive, "Shorten it.");
    let seen = desk.seen(&drive);
    assert!(
        seen.iter().any(|text| text == "Before this is sent"),
        "{seen:?}"
    );
    assert!(seen.iter().any(|text| text == anthropic), "{seen:?}");
    for _ in 0..5 {
        drive.settle(&mut desk);
    }
    assert!(!desk.assist.is_working(), "it waits");
    assert_eq!(reach.connections(), 0, "and nothing was asked");

    // Not this time: the words go back to the composer.
    desk.click(&drive, "Not this time");
    let seen = desk.seen(&drive);
    assert!(!seen.iter().any(|text| text == "Before this is sent"));
    assert_eq!(desk.assist.composer(), "Shorten it.");
    assert!(desk.assist.transcript().is_empty());
    assert_eq!(reach.connections(), 0);

    // Send: the request goes.
    drive.press(&mut desk, "Enter");
    assert!(desk.seen(&drive).iter().any(|text| text == anthropic));
    desk.click_last(&drive, "Send");
    desk.finished(&drive);
    assert_eq!(
        desk.assist.transcript(),
        [
            Entry::Asked("Shorten it.".into()),
            Entry::Said {
                words: "Shorter.".into(),
                kept: true
            }
        ]
    );

    // A later request to the same helper goes at once.
    desk.ask(&drive, "Shorter still, please.");
    assert!(!desk
        .seen(&drive)
        .iter()
        .any(|text| text == "Before this is sent"));
    desk.finished(&drive);
    assert_eq!(heard.lock().unwrap().len(), 2);

    // Another service is a new place for the words to go: it asks again, by
    // name.
    desk.assist.open_settings();
    desk.click(&drive, "Another service (advanced)");
    desk.click(&drive, "https://example.com/v1");
    drive.type_text(&mut desk, "https://api.example.com/v1");
    drive.press(&mut desk, "Tab");
    drive.type_text(&mut desk, "sk-service");
    drive.press(&mut desk, "Tab");
    drive.type_text(&mut desk, "some-model");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(kept.helper, Some(Choice::Service));
    assert_eq!(kept.service.address, "https://api.example.com/v1");
    assert_eq!(kept.service.key, "sk-service");
    assert_eq!(kept.service.model, "some-model");
    desk.ask(&drive, "Shorten it again.");
    let seen = desk.seen(&drive);
    assert!(
        seen.iter().any(|text| text
            == "The selected paragraph and the two around it will be sent to api.example.com. \
                Assist asks once for each place your words may go."),
        "{seen:?}"
    );
    desk.click_last(&drive, "Send");
    desk.finished(&drive);
    assert_eq!(
        desk.assist.transcript().last(),
        Some(&Entry::Said {
            words: "From the service.".into(),
            kept: true
        })
    );

    // Ollama on this computer sends nothing away, and never asks.
    desk.assist.open_settings();
    desk.click(&drive, "Ollama");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    desk.ask(&drive, "And again.");
    assert!(!desk
        .seen(&drive)
        .iter()
        .any(|text| text == "Before this is sent"));
    desk.finished(&drive);
    assert_eq!(
        desk.assist.transcript().last(),
        Some(&Entry::Said {
            words: "From Ollama.".into(),
            kept: true
        })
    );

    // Where a helper is decides it, whatever it is.
    let at = |choice: Choice, address: &str| {
        let mut settings = Settings {
            helper: Some(choice),
            ..Settings::default()
        };
        settings.ollama.address = address.to_owned();
        settings.service.address = address.to_owned();
        ::assist::destination(&settings)
    };
    assert_eq!(at(Choice::Service, "http://127.0.0.1:8080/v1"), None);
    assert_eq!(at(Choice::Service, "http://localhost:8080/v1"), None);
    assert_eq!(
        at(Choice::Ollama, "http://192.168.1.4:11434"),
        Some("Ollama at 192.168.1.4:11434".to_owned())
    );
    assert_eq!(at(Choice::Local, ""), None);
}

/// A key typed for a helper that was then not chosen is not kept, even when
/// the service refused it and another helper was saved instead.
#[test]
fn a_key_a_service_refused_is_not_kept_when_another_helper_is_saved() {
    const REFUSED: &str = "sk-ant-refused-key-9999";
    let scratch = Scratch::new("box-refused-key");
    let reach = Fake::new().checks(vec![Err(Failure::new(
        FailureKind::Unauthorized,
        "Claude did not accept the key.",
    ))]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&ollama_here()));
    drive.settle(&mut desk);
    desk.assist.open_settings();
    desk.click(&drive, "Claude, over the internet");
    desk.click(&drive, "sk-ant-…");
    drive.type_text(&mut desk, REFUSED);
    desk.click(&drive, "Save");
    desk.until_seen(&drive, "Claude did not accept the key.");

    desk.click(&drive, "Ollama");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    let file = std::fs::read_to_string(scratch.settings()).unwrap();
    assert!(!file.contains(REFUSED), "the refused key was kept: {file}");
    assert_eq!(
        Settings::read(&scratch.settings()).unwrap(),
        ollama_here(),
        "the file is as it was"
    );
    assert_eq!(
        reach.checked.lock().unwrap().len(),
        1,
        "Ollama as it was needs no check"
    );
}

/// Each request is priced at the helper it went to: Claude's at the model it
/// asked, and a helper that costs nothing adds words and no cents. Which
/// model the box has ticked changes nothing about what was spent.
#[test]
fn each_request_is_priced_at_the_helper_it_went_to() {
    let scratch = Scratch::new("box-priced");
    let cost = |input, output| Usage {
        input,
        output,
        ..Usage::default()
    };
    let reach = Fake::new().helpers(vec![
        Box::new(Scripted::new([
            Turn::says("On Opus.").costs(cost(2000, 500))
        ])),
        Box::new(Scripted::new([
            Turn::says("On Haiku.").costs(cost(2000, 500))
        ])),
        Box::new(Scripted::new([
            Turn::says("On Ollama.").costs(cost(1000, 100))
        ])),
    ]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&claude_with_key()));
    drive.settle(&mut desk);

    desk.ask(&drive, "One.");
    desk.click_last(&drive, "Send");
    desk.finished(&drive);
    assert!((desk.assist.cents().unwrap() - 2.25).abs() < 1e-9);

    desk.assist.open_settings();
    desk.click(
        &drive,
        "Haiku 4.5 (cheapest) — under half a cent a paragraph",
    );
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    desk.ask(&drive, "Two.");
    desk.finished(&drive);
    assert!(
        (desk.assist.cents().unwrap() - 2.7).abs() < 1e-9,
        "Opus's request at Opus's price, Haiku's at Haiku's: {:?}",
        desk.assist.cents()
    );

    desk.assist.open_settings();
    desk.click(&drive, "Ollama");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    desk.ask(&drive, "Three.");
    desk.finished(&drive);
    assert!(
        (desk.assist.cents().unwrap() - 2.7).abs() < 1e-9,
        "Ollama costs nothing"
    );
    assert_eq!(desk.assist.spent(), cost(5000, 1100));

    let line = "The assistant read about 3,800 words and wrote about 830, for about 3 cents.";
    desk.assist.open_settings();
    assert!(desk.everywhere(&drive).iter().any(|text| text == line));
    desk.click(&drive, "Claude, over the internet");
    desk.click(&drive, "Opus 5 (best) — about 2 cents a paragraph");
    let seen = desk.everywhere(&drive);
    assert!(
        seen.iter().any(|text| text == line),
        "ticking another model reprices nothing: {seen:?}"
    );
}

/// A key typed, refused, and then left for the key already on this computer
/// is not kept: the check asked about the computer's key, not the typed one.
#[test]
fn a_key_typed_then_left_for_another_sign_in_is_not_kept() {
    const TYPED: &str = "sk-ant-typed-and-left";
    let scratch = Scratch::new("left-key");
    let reach = Fake::new()
        .finds(vec![
            Row::ClaudeHere(::assist::ClaudeLogin::Environment),
            Row::Local,
            Row::ClaudeWithKey,
            Row::Service,
        ])
        .checks(vec![
            Err(Failure::new(FailureKind::Unauthorized, "Refused.")),
            Ok("Accepted.".into()),
        ]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.settings());
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    desk.click(&drive, "Claude, over the internet");
    desk.click(&drive, "Use this");
    drive.settle(&mut desk);
    drive.type_text(&mut desk, TYPED);
    desk.click(&drive, "Save");
    desk.until_seen(&drive, "Refused.");

    desk.click(&drive, "A key I paste");
    drive.settle(&mut desk);
    desk.click(&drive, "The key already on this computer");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(kept.claude.login, ::assist::ClaudeLogin::Environment);
    assert_eq!(kept.claude.key, "", "the typed key was not kept");
    assert!(!std::fs::read_to_string(scratch.settings())
        .unwrap()
        .contains(TYPED));
    assert_eq!(
        reach.checked.lock().unwrap()[1].claude.key,
        "",
        "and the check was of the computer's key"
    );
}
