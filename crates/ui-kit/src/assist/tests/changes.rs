//! What the pane does when its settings change under it: in the other
//! application, in its own box, or nowhere to keep them at all.

use ::assist::Turn;

use super::*;

/// Claude with its key, `key`.
fn claude_keyed(key: &str) -> Settings {
    let mut settings = claude_with_key();
    settings.claude.key = key.to_owned();
    settings
}

/// Another service, over the internet.
fn a_service() -> Settings {
    let mut settings = Settings {
        helper: Some(Choice::Service),
        ..Settings::default()
    };
    settings.service.address = "https://api.example.com/v1".into();
    settings.service.model = "some-model".into();
    settings
}

/// Types `key` over the key in the box's key field, the one field of dots.
fn retype_key(desk: &mut Desk, drive: &Driver, key: &str) {
    let dots = desk
        .painted(drive)
        .texts()
        .into_iter()
        .find(|text| !text.text.is_empty() && text.text.chars().all(|c| c == '•'))
        .expect("the key field");
    drive.click(desk, dots.shown().center());
    drive.press(desk, "ctrl+A");
    drive.type_text(desk, key);
}

/// Closes the pane for a couple of frames, runs `meanwhile`, and opens it.
fn reopen(desk: &mut Desk, drive: &Driver, meanwhile: impl FnOnce()) {
    desk.open = false;
    drive.settle(desk);
    drive.settle(desk);
    meanwhile();
    desk.open = true;
    drive.settle(desk);
    drive.settle(desk);
}

/// A choice the other application saved is taken up when the pane opens with
/// no conversation, agreement and all: the words waiting for Anthropic are not
/// sent to the service the other application chose.
#[test]
fn settings_changed_elsewhere_are_taken_up_when_the_pane_opens_with_no_conversation() {
    let scratch = Scratch::new("elsewhere");
    let reach = Fake::new().helpers(vec![Box::new(Scripted::new([
        Turn::says("From the service."),
        Turn::says("Still the service."),
    ]))]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&claude_with_key()));
    drive.settle(&mut desk);
    desk.ask(&drive, "Shorten it.");
    assert!(desk
        .seen(&drive)
        .iter()
        .any(|text| text.contains("will be sent to Anthropic")));

    reopen(&mut desk, &drive, || {
        a_service().save(&scratch.settings()).unwrap();
    });
    let seen = desk.seen(&drive);
    assert!(
        !seen
            .iter()
            .any(|text| text.contains("will be sent to Anthropic")),
        "the agreement asked for Anthropic is gone: {seen:?}"
    );
    assert!(
        seen.iter()
            .any(|text| text == "api.example.com — some-model"),
        "{seen:?}"
    );
    assert_eq!(desk.assist.composer(), "Shorten it.", "the words wait");
    assert_eq!(reach.connections(), 0, "and nothing was sent");
    assert!(desk.assist.transcript().contains(&Entry::Note {
        sentence: "Assist will use api.example.com — some-model.".into(),
        action: None,
    }));

    desk.click(&drive, "Send");
    let seen = desk.seen(&drive);
    assert!(
        seen.iter()
            .any(|text| text.contains("will be sent to api.example.com")),
        "{seen:?}"
    );
    desk.click_last(&drive, "Send");
    desk.finished(&drive);
    assert_eq!(reach.connected.lock().unwrap()[0], a_service());

    // With a conversation under way, the pane keeps its helper however the
    // file changes, and the conversation goes on with it.
    reopen(&mut desk, &drive, || {
        claude_with_key().save(&scratch.settings()).unwrap();
    });
    let seen = desk.seen(&drive);
    assert!(
        seen.iter()
            .any(|text| text == "api.example.com — some-model"),
        "{seen:?}"
    );
    desk.ask(&drive, "And again.");
    assert!(!desk
        .seen(&drive)
        .iter()
        .any(|text| text == "Before this is sent"));
    desk.finished(&drive);
    assert_eq!(
        desk.assist.transcript().last(),
        Some(&Entry::Said {
            words: "Still the service.".into(),
            kept: true
        })
    );
    assert_eq!(reach.connections(), 1, "the same helper as before");
}

/// The same helper told something new — the fallback, a new key — is asked
/// afresh next time and carries the conversation on; another model is
/// another conversation, and the transcript says so.
#[test]
fn the_same_helper_told_something_new_carries_the_conversation_on() {
    let scratch = Scratch::new("same-helper");
    let first = Scripted::new([Turn::says("One done.")]);
    let second = Scripted::new([Turn::says("Two done.")]);
    let second_heard = second.heard();
    let third = Scripted::new([Turn::says("Three done.")]);
    let third_heard = third.heard();
    let reach = Fake::new().helpers(vec![Box::new(first), Box::new(second), Box::new(third)]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&claude_with_key()));
    drive.settle(&mut desk);
    desk.ask(&drive, "One.");
    desk.click_last(&drive, "Send");
    desk.finished(&drive);

    desk.assist.open_settings();
    desk.click(
        &drive,
        "If Opus 5 declines, let Anthropic's recommended model answer",
    );
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    assert!(!Settings::read(&scratch.settings()).unwrap().claude.fallback);
    assert_eq!(
        reach.checked.lock().unwrap().len(),
        0,
        "the fallback needs no check"
    );
    assert!(
        !desk.assist.transcript().iter().any(|entry| matches!(
            entry,
            Entry::Note { sentence, .. } if sentence.starts_with("Assist will use")
        )),
        "it is the same helper: {:?}",
        desk.assist.transcript()
    );

    desk.ask(&drive, "Two.");
    assert!(!desk
        .seen(&drive)
        .iter()
        .any(|text| text == "Before this is sent"));
    desk.finished(&drive);
    assert_eq!(reach.connections(), 2, "asked afresh");
    assert!(!reach.connected.lock().unwrap()[1].claude.fallback);
    let said: Vec<String> = second_heard.lock().unwrap()[0]
        .conversation
        .messages()
        .iter()
        .map(Message::text)
        .collect();
    assert_eq!(
        said,
        [
            "Context: paragraph 1.\n\nRequest: One.",
            "One done.",
            "Context: paragraph 1.\n\nRequest: Two.",
        ],
        "carrying the conversation on"
    );

    // Another model is another conversation.
    desk.assist.open_settings();
    desk.click(
        &drive,
        "Haiku 4.5 (cheapest) — under half a cent a paragraph",
    );
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    assert!(desk.assist.transcript().contains(&Entry::Note {
        sentence: "Assist will use Claude — Haiku 4.5, starting a new conversation.".into(),
        action: None,
    }));
    assert!(
        desk.assist
            .transcript()
            .iter()
            .all(|entry| !matches!(entry, Entry::Said { kept: true, .. })),
        "the old answers say they are not kept"
    );
    desk.ask(&drive, "Three.");
    desk.finished(&drive);
    let said: Vec<String> = third_heard.lock().unwrap()[0]
        .conversation
        .messages()
        .iter()
        .map(Message::text)
        .collect();
    assert_eq!(said, ["Context: paragraph 1.\n\nRequest: Three."]);
}

/// A box saves its own edit onto the file as the other application left it:
/// a key replaced there is not put back.
#[test]
fn a_box_saves_its_edit_onto_what_the_other_window_saved_since() {
    let scratch = Scratch::new("onto");
    let reach = Fake::new().helpers(vec![Box::new(Scripted::new([Turn::says("Done.")]))]);
    let drive = Driver::new();
    let mut desk = Desk::with(
        Arc::clone(&reach),
        scratch.holding(&claude_keyed("sk-ant-old")),
    );
    drive.settle(&mut desk);
    desk.ask(&drive, "One.");
    desk.click_last(&drive, "Send");
    desk.finished(&drive);

    desk.assist.open_settings();
    drive.settle(&mut desk);
    // The other application replaces the key while the box is open.
    claude_keyed("sk-ant-new")
        .save(&scratch.settings())
        .unwrap();
    desk.click(
        &drive,
        "If Opus 5 declines, let Anthropic's recommended model answer",
    );
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(
        kept.claude.key, "sk-ant-new",
        "the other window's key stands"
    );
    assert!(!kept.claude.fallback, "and this window's edit is in");
    assert_eq!(desk.assist.settings(), Some(&kept));

    // A box opened later starts from the file as it is: its key field holds
    // the newer key, one dot a letter.
    claude_keyed("sk-ant-newer")
        .save(&scratch.settings())
        .unwrap();
    desk.assist.open_settings();
    let dots: Vec<usize> = desk
        .everywhere(&drive)
        .iter()
        .filter(|text| !text.is_empty() && text.chars().all(|c| c == '•'))
        .map(|text| text.chars().count())
        .collect();
    assert_eq!(dots, ["sk-ant-newer".len()], "the box shows the newer key");
    desk.click(&drive, "Save");
    assert_eq!(
        Settings::read(&scratch.settings()).unwrap().claude.key,
        "sk-ant-newer"
    );
    assert_eq!(desk.assist.settings().unwrap().claude.key, "sk-ant-newer");
}

/// With nowhere to keep settings — no home directory — the pane says so, and
/// a choice fails with the reason rather than being written somewhere shared.
#[test]
fn a_pane_with_nowhere_to_keep_settings_says_why() {
    let reach = Fake::new().finds(vec![
        Row::Local(::assist::local::MODELS[0]),
        Row::ClaudeWithKey,
        Row::Service,
    ]);
    let drive = Driver::new();
    let mut desk = Desk::new(Assist::at(
        setup(),
        Arc::clone(&reach) as Arc<dyn Reach>,
        Err("there is no home directory".into()),
    ));
    let seen = desk.seen(&drive);
    assert!(
        seen.iter()
            .any(|text| text == "There is nowhere to keep them: there is no home directory"),
        "{seen:?}"
    );
    desk.click(&drive, "Choose Again");
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    desk.click(&drive, "Claude, over the internet");
    desk.click(&drive, "Use this");
    drive.type_text(&mut desk, "sk-ant-anything");
    desk.click(&drive, "Save");
    desk.until_seen(
        &drive,
        "Assist's settings could not be saved: there is no home directory.",
    );
    assert!(
        reach.checked.lock().unwrap().is_empty(),
        "a key that cannot be kept is not sent to be checked"
    );
    assert!(desk.assist.box_up(), "and the box stays, to be cancelled");
}

/// A check nobody waits for any more still has its refusals counted, when its
/// thread at last finishes.
#[test]
fn a_check_cancelled_under_way_is_still_counted() {
    struct Refusing {
        gate: Mutex<Option<mpsc::Receiver<()>>>,
    }
    impl Reach for Refusing {
        // Nothing found, and nothing refused: only the check counts here.
        fn look(&self) -> Vec<Row> {
            Vec::new()
        }

        fn check(&self, _: &Settings) -> Result<String, Failure> {
            if let Some(gate) = self.gate.lock().unwrap().take() {
                let _ = gate.recv_timeout(PATIENCE);
            }
            ::assist::offline::count();
            Err(Failure::new(
                ::assist::FailureKind::Offline,
                "Nothing is checked from a test.",
            ))
        }
    }
    let scratch = Scratch::new("cancelled-check");
    let (go, gate) = mpsc::channel();
    let reach = Arc::new(Refusing {
        gate: Mutex::new(Some(gate)),
    });
    let drive = Driver::new();
    let mut desk = Desk::new(Assist::with(
        setup(),
        reach,
        scratch.holding(&ollama_here()),
    ));
    drive.settle(&mut desk);
    desk.assist.open_settings();
    desk.click(&drive, "Claude, over the internet");
    desk.click(&drive, "Save");
    desk.until_seen(&drive, "Asking whether it answers to this…");
    drive.press(&mut desk, "Escape");
    drive.settle(&mut desk);
    assert!(
        !desk.assist.box_up(),
        "cancelled while the check was under way"
    );
    let before = crate::headless::helpers_refused();
    go.send(()).unwrap();
    desk.until(&drive, "the refusal to be counted", |_| {
        crate::headless::helpers_refused() > before
    });
    assert_eq!(crate::headless::helpers_refused() - before, 1);
}

/// Save keeps the helper the box shows, and checks it, whatever the other
/// application chose while the box was open: a key typed for Claude is not
/// saved, unchecked, beside the other application's Ollama.
#[test]
fn the_box_saves_the_helper_it_shows_and_checks_it() {
    let scratch = Scratch::new("shown-helper");
    let reach = Fake::new();
    let drive = Driver::new();
    let mut desk = Desk::with(
        Arc::clone(&reach),
        scratch.holding(&claude_keyed("sk-ant-0000")),
    );
    drive.settle(&mut desk);
    desk.assist.open_settings();
    drive.settle(&mut desk);
    ollama_here().save(&scratch.settings()).unwrap();

    retype_key(&mut desk, &drive, "sk-ant-1111");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());

    let checked = reach.checked.lock().unwrap().clone();
    assert_eq!(checked.len(), 1, "the typed key was checked");
    assert_eq!(checked[0].helper, Some(Choice::Claude));
    assert_eq!(checked[0].claude.key, "sk-ant-1111");
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(
        kept.helper,
        Some(Choice::Claude),
        "the helper the box showed"
    );
    assert_eq!(kept.claude.key, "sk-ant-1111");
    assert_eq!(
        kept.ollama,
        ollama_here().ollama,
        "and the other window's Ollama part"
    );
}

/// What the other application saves while a check is under way stands: the
/// box writes its edit onto the file as it is when the answer comes — once
/// that, too, has been checked, since it is not what was.
#[test]
fn a_key_saved_elsewhere_during_a_check_stands_once_it_too_is_checked() {
    let scratch = Scratch::new("during-check");
    let reach = Fake::new();
    let drive = Driver::new();
    let mut desk = Desk::with(
        Arc::clone(&reach),
        scratch.holding(&claude_keyed("sk-ant-old")),
    );
    drive.settle(&mut desk);
    desk.assist.open_settings();
    desk.click(
        &drive,
        "Haiku 4.5 (cheapest) — under half a cent a paragraph",
    );
    let go = reach.check_waits();
    desk.click(&drive, "Save");
    desk.until(&drive, "the first check to be under way", |_| {
        reach.check_gate.lock().unwrap().is_none()
    });
    let go_again = reach.check_waits();
    claude_keyed("sk-ant-new")
        .save(&scratch.settings())
        .unwrap();
    go.send(()).unwrap();
    desk.until(&drive, "the second check to be under way", |_| {
        reach.check_gate.lock().unwrap().is_none()
    });
    // What the second check does not ask about is taken as the file has it
    // when that check answers.
    let mut unasked = claude_keyed("sk-ant-new");
    unasked.claude.fallback = false;
    unasked.save(&scratch.settings()).unwrap();
    go_again.send(()).unwrap();
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(kept.claude.model, "claude-haiku-4-5", "this box's edit");
    assert_eq!(kept.claude.key, "sk-ant-new", "and the key saved meanwhile");
    assert!(
        !kept.claude.fallback,
        "and what was saved during the last check"
    );
    let checked: Vec<(String, String)> = reach
        .checked
        .lock()
        .unwrap()
        .iter()
        .map(|settings| (settings.claude.model.clone(), settings.claude.key.clone()))
        .collect();
    assert_eq!(
        checked,
        [
            ("claude-haiku-4-5".to_owned(), "sk-ant-old".to_owned()),
            ("claude-haiku-4-5".to_owned(), "sk-ant-new".to_owned()),
        ],
        "what was written was checked as it was written"
    );
}

/// A check that passed says nothing of settings the other window saved while
/// it was under way: those are checked before anything is written, and when
/// the service refuses them nothing is, and the box says why.
#[test]
fn settings_saved_elsewhere_during_a_check_are_checked_before_they_are_kept() {
    let scratch = Scratch::new("during-check-refused");
    let reach = Fake::new().checks(vec![
        Ok("Claude accepted the key.".to_owned()),
        Err(Failure::new(
            ::assist::FailureKind::Unauthorized,
            "Claude did not accept the key.",
        )),
    ]);
    let drive = Driver::new();
    let mut desk = Desk::with(
        Arc::clone(&reach),
        scratch.holding(&claude_keyed("sk-ant-old")),
    );
    drive.settle(&mut desk);
    desk.assist.open_settings();
    desk.click(
        &drive,
        "Haiku 4.5 (cheapest) — under half a cent a paragraph",
    );
    let go = reach.check_waits();
    desk.click(&drive, "Save");
    desk.until_seen(&drive, "Asking whether it answers to this…");
    claude_keyed("sk-ant-refused")
        .save(&scratch.settings())
        .unwrap();
    go.send(()).unwrap();
    desk.until_seen(&drive, "Claude did not accept the key.");
    assert!(desk.assist.box_up(), "the box stays");
    assert_eq!(
        Settings::read(&scratch.settings()).unwrap(),
        claude_keyed("sk-ant-refused"),
        "and nothing of this box was written"
    );
    assert_eq!(reach.checked.lock().unwrap().len(), 2);
    assert!(
        !desk.assist.transcript().iter().any(
            |entry| matches!(entry, Entry::Note { sentence, .. } if sentence.contains("accepted"))
        ),
        "the first check's answer is not said as if it held"
    );
}

/// A key typed while the box shows one service's address goes to that
/// address, and is saved with it, whatever address the other window saved
/// meanwhile.
#[test]
fn a_key_typed_for_the_address_shown_goes_there_and_is_saved_with_it() {
    let scratch = Scratch::new("key-address");
    let reach = Fake::new();
    let drive = Driver::new();
    let mut shown = a_service();
    shown.service.key = "sk-shown".into();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&shown));
    drive.settle(&mut desk);
    desk.assist.open_settings();
    drive.settle(&mut desk);
    let mut moved = shown.clone();
    moved.service.address = "https://other.example.com/v1".into();
    moved.save(&scratch.settings()).unwrap();

    retype_key(&mut desk, &drive, "sk-typed");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    let checked = reach.checked.lock().unwrap().clone();
    assert_eq!(checked.len(), 1);
    assert_eq!(
        (
            checked[0].service.address.as_str(),
            checked[0].service.key.as_str()
        ),
        ("https://api.example.com/v1", "sk-typed"),
        "the key went to the address the box showed"
    );
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(kept.service, checked[0].service, "and was saved with it");
}

/// A key pasted while the box shows "Key:" is checked, and saved, as a key —
/// not beside the other window's choice of the ant command's login, which
/// the check would have asked about instead.
#[test]
fn a_key_pasted_is_checked_and_saved_with_the_sign_in_it_was_pasted_for() {
    let scratch = Scratch::new("key-login");
    let reach = Fake::new();
    let drive = Driver::new();
    let mut desk = Desk::with(
        Arc::clone(&reach),
        scratch.holding(&claude_keyed("sk-ant-old")),
    );
    drive.settle(&mut desk);
    desk.assist.open_settings();
    drive.settle(&mut desk);
    let mut ant = claude_keyed("sk-ant-old");
    ant.claude.login = ::assist::ClaudeLogin::Ant;
    ant.save(&scratch.settings()).unwrap();

    retype_key(&mut desk, &drive, "sk-ant-pasted");
    desk.click(&drive, "Save");
    desk.until(&drive, "the box to close", |desk| !desk.assist.box_up());
    let checked: Vec<_> = reach
        .checked
        .lock()
        .unwrap()
        .iter()
        .map(|settings| (settings.claude.login, settings.claude.key.clone()))
        .collect();
    assert_eq!(
        checked,
        [(::assist::ClaudeLogin::Key, "sk-ant-pasted".to_owned())]
    );
    let kept = Settings::read(&scratch.settings()).unwrap();
    assert_eq!(
        (kept.claude.login, kept.claude.key.as_str()),
        (::assist::ClaudeLogin::Key, "sk-ant-pasted")
    );
}

/// The box says where Claude is asked, as the card does: Anthropic, or the
/// address the settings file names instead, which the box has no field for.
#[test]
fn the_box_says_where_claude_is_asked() {
    let scratch = Scratch::new("claude-where");
    let drive = Driver::new();
    let mut desk = Desk::with(Fake::new(), scratch.holding(&claude_with_key()));
    drive.settle(&mut desk);
    desk.assist.open_settings();
    let anthropic =
        "What you select, and a little of the document around it, is sent to Anthropic.";
    let seen = desk.everywhere(&drive);
    assert!(seen.iter().any(|text| text == anthropic), "{seen:?}");
    desk.click(&drive, "Cancel");
    drive.settle(&mut desk);

    let mut relay = claude_with_key();
    relay.claude.address = "https://relay.example.com/anthropic".into();
    relay.save(&scratch.settings()).unwrap();
    desk.assist.open_settings();
    let seen = desk.everywhere(&drive);
    assert!(
        seen.iter().any(|text| text
            == "What you select, and a little of the document around it, is sent to \
                Claude at relay.example.com."),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|text| text == anthropic));
}

/// A request waiting for the person's agreement is not lost when the file
/// turns out, as the pane opens again, not to be readable: the words go back
/// to the composer.
#[test]
fn a_request_waiting_for_agreement_is_kept_when_the_settings_cannot_be_read() {
    let scratch = Scratch::new("waiting-unreadable");
    let drive = Driver::new();
    let mut desk = Desk::with(Fake::new(), scratch.holding(&claude_with_key()));
    drive.settle(&mut desk);
    desk.ask(&drive, "Shorten it.");
    assert!(desk
        .seen(&drive)
        .iter()
        .any(|text| text == "Before this is sent"));
    reopen(&mut desk, &drive, || {
        std::fs::write(scratch.settings(), "helper = [").unwrap();
    });
    let seen = desk.seen(&drive);
    assert!(
        !seen.iter().any(|text| text == "Before this is sent"),
        "{seen:?}"
    );
    assert!(seen
        .iter()
        .any(|text| text == "Assist's settings could not be read"));
    assert_eq!(desk.assist.composer(), "Shorten it.");
}

/// A look still under way when a helper is chosen elsewhere is heard to its
/// end, for the refusals its thread counted.
#[test]
fn a_look_put_away_by_a_choice_made_elsewhere_is_still_counted() {
    struct Slow {
        gate: Mutex<Option<mpsc::Receiver<()>>>,
    }
    impl Reach for Slow {
        fn look(&self) -> Vec<Row> {
            if let Some(gate) = self.gate.lock().unwrap().take() {
                let _ = gate.recv_timeout(PATIENCE);
            }
            ::assist::offline::count();
            vec![Row::Local(::assist::local::MODELS[0])]
        }
    }
    let scratch = Scratch::new("look-put-away");
    let (go, gate) = mpsc::channel();
    let reach = Arc::new(Slow {
        gate: Mutex::new(Some(gate)),
    });
    let drive = Driver::new();
    let mut desk = Desk::new(Assist::with(setup(), reach, scratch.settings()));
    drive.settle(&mut desk);
    assert!(matches!(desk.assist.choosing, Some(Choosing::Looking(_))));
    reopen(&mut desk, &drive, || {
        ollama_here().save(&scratch.settings()).unwrap();
    });
    assert!(
        desk.assist.choosing.is_none(),
        "the helper chosen elsewhere is taken"
    );
    let before = crate::headless::helpers_refused();
    go.send(()).unwrap();
    desk.until(&drive, "the look's refusal to be counted", |_| {
        crate::headless::helpers_refused() > before
    });
    assert_eq!(crate::headless::helpers_refused() - before, 1);
}
