//! B6 and B7: the pane in a window, and the download's bar.

use ::assist::{FailureKind, Turn};

use super::*;

/// A pane with a helper chosen, in a window of the driver's.
fn ready(name: &str, helpers: Vec<Box<dyn Provider>>) -> (Scratch, Driver, Desk) {
    let scratch = Scratch::new(name);
    let reach = Fake::new().helpers(helpers);
    let drive = Driver::new();
    let mut desk = Desk::with(reach, scratch.holding(&ollama_here()));
    drive.settle(&mut desk);
    (scratch, drive, desk)
}

#[test]
fn escape_in_the_pane_hands_the_keyboard_back() {
    // At every pace a hand could type at.
    for pace in 0..=2 {
        let (_scratch, drive, mut desk) = ready(&format!("escape-{pace}"), Vec::new());
        let idle = |desk: &mut Desk| {
            for _ in 0..pace {
                drive.settle(desk);
            }
        };
        desk.assist.focus();
        drive.settle(&mut desk);
        idle(&mut desk);
        drive.type_text(&mut desk, "hello");
        idle(&mut desk);
        assert_eq!(desk.assist.composer(), "hello", "pace {pace}");
        assert_eq!(desk.typed, "", "pace {pace}: the pane had the keys");
        drive.press(&mut desk, "Escape");
        idle(&mut desk);
        assert_eq!(desk.chosen, [Chosen::Leave], "pace {pace}");
        assert!(!desk.assist.holds_keyboard(drive.ctx()), "pace {pace}");
        drive.type_text(&mut desk, "abc");
        assert_eq!(
            desk.typed, "abc",
            "pace {pace}: what is typed next is the document's"
        );
        assert_eq!(
            desk.assist.composer(),
            "hello",
            "pace {pace}: and the words wait in the composer"
        );
    }

    // Escape on the very frame after the pane was given the keyboard, before
    // egui would keep the key for the field.
    let (_scratch, drive, mut desk) = ready("escape-at-once", Vec::new());
    desk.assist.focus();
    drive.settle(&mut desk);
    drive.press(&mut desk, "Escape");
    assert_eq!(desk.chosen, [Chosen::Leave]);
    drive.type_text(&mut desk, "x");
    assert_eq!(desk.typed, "x");

    // The arrows move the composer's caret, on the very frame after the pane
    // was given the keyboard too, where egui would move the keyboard itself.
    let (_scratch, drive, mut desk) = ready("arrows-at-once", Vec::new());
    desk.assist.focus();
    drive.settle(&mut desk);
    drive.press(&mut desk, "ArrowDown");
    drive.type_text(&mut desk, "ac");
    drive.press(&mut desk, "ArrowLeft");
    drive.type_text(&mut desk, "b");
    assert_eq!(desk.assist.composer(), "abc");
    assert!(desk.assist.holds_keyboard(drive.ctx()));

    // Put away with the keyboard and opened again, the pane does not take
    // the document's keys on the frame it comes back.
    let (_scratch, drive, mut desk) = ready("reopened", Vec::new());
    desk.assist.focus();
    drive.settle(&mut desk);
    drive.type_text(&mut desk, "a draft");
    desk.open = false;
    drive.settle(&mut desk);
    drive
        .ctx()
        .memory_mut(|m| m.request_focus(egui::Id::new("desk-document")));
    drive.settle(&mut desk);
    desk.open = true;
    drive.press(&mut desk, "Enter");
    drive.settle(&mut desk);
    assert!(
        desk.asked.is_empty(),
        "the draft was not sent: {:?}",
        desk.asked
    );
    assert_eq!(desk.assist.composer(), "a draft");

    // Escape in the settings box closes the box, and the pane keeps the
    // keyboard it had.
    let (_scratch, drive, mut desk) = ready("escape-box", Vec::new());
    desk.assist.open_settings();
    drive.settle(&mut desk);
    drive.press(&mut desk, "Escape");
    drive.settle(&mut desk);
    assert!(!desk.assist.box_up());
    assert!(
        desk.chosen.is_empty(),
        "the box took the key: {:?}",
        desk.chosen
    );
}

#[test]
fn a_quick_verb_sends_at_once_with_little_effort_and_one_that_needs_words_waits_for_them() {
    let scripted = Scripted::new([
        Turn::calls("read_paragraphs", reading(1)),
        Turn::says("Better."),
        Turn::says("En français."),
    ]);
    let heard = scripted.heard();
    let (_scratch, drive, mut desk) = ready("verbs", vec![Box::new(scripted)]);
    desk.verbs = vec![
        ("Improve the wording", "Improve the wording."),
        ("Translate…", "Translate into "),
    ];
    desk.answers = Box::new(|_| None);
    drive.settle(&mut desk);

    desk.click(&drive, "Improve the wording");
    assert_eq!(
        desk.asked,
        [Asked {
            words: "Improve the wording.".into(),
            scope: 1,
            effort: Effort::Low,
        }],
        "sent at once, asking for little effort"
    );
    assert_eq!(desk.assist.composer(), "", "nothing was typed for it");
    desk.until(&drive, "the call", |desk| desk.held.is_some());

    // While it works, the chips wait.
    desk.click(&drive, "Improve the wording");
    desk.click(&drive, "Translate…");
    assert_eq!(desk.asked.len(), 1);
    assert_eq!(desk.assist.composer(), "");
    let held = desk.held.clone().unwrap();
    desk.release(Ran::new(ToolResult::ok(&held.tool, "paragraph one")));
    desk.finished(&drive);
    assert!(
        heard
            .lock()
            .unwrap()
            .iter()
            .all(|asked| asked.effort == Effort::Low),
        "the effort holds for the whole request"
    );

    // A verb that needs words starts the request and waits for them.
    desk.click(&drive, "Translate…");
    assert_eq!(desk.asked.len(), 1, "nothing was sent");
    assert_eq!(desk.assist.composer(), "Translate into ");
    assert!(desk.assist.holds_keyboard(drive.ctx()));
    drive.type_text(&mut desk, "French");
    assert_eq!(desk.assist.composer(), "Translate into French");
    drive.press(&mut desk, "Enter");
    assert_eq!(
        desk.asked[1],
        Asked {
            words: "Translate into French".into(),
            scope: 1,
            effort: Effort::Usual,
        }
    );
    desk.finished(&drive);
    assert_eq!(heard.lock().unwrap().last().unwrap().effort, Effort::Usual);
}

#[test]
fn the_scope_chip_follows_the_host_until_the_person_chooses_another() {
    let (_scratch, drive, mut desk) = ready(
        "scope",
        vec![Box::new(Scripted::new([Turn::says("Summed up.")]))],
    );
    desk.scopes = vec![
        ("Selection", Some(42)),
        ("Paragraph", None),
        ("Whole document", Some(4210)),
    ];
    desk.following = 1;
    let seen = desk.seen(&drive);
    assert_eq!(desk.assist.scope(), 1);
    assert!(seen.iter().any(|text| text == "Paragraph"), "{seen:?}");
    let counted =
        |text: &String| text.ends_with(" words") && text.starts_with(|c: char| c.is_ascii_digit());
    assert!(
        !seen.iter().any(counted),
        "no count for a paragraph: {seen:?}"
    );

    // The selection changes, and the chip with it.
    desk.following = 0;
    let seen = desk.seen(&drive);
    assert_eq!(desk.assist.scope(), 0);
    assert!(seen.iter().any(|text| text == "Selection"), "{seen:?}");
    assert!(seen.iter().any(|text| text == "42 words"), "{seen:?}");

    // The person chooses the whole document: it stays chosen while the
    // selection says the same, and states how much that is.
    desk.click(&drive, "Selection");
    drive.settle(&mut desk);
    desk.click(&drive, "Whole document");
    for _ in 0..3 {
        drive.settle(&mut desk);
    }
    assert_eq!(desk.assist.scope(), 2);
    let seen = desk.seen(&drive);
    assert!(seen.iter().any(|text| text == "Whole document"), "{seen:?}");
    assert!(seen.iter().any(|text| text == "4,210 words"), "{seen:?}");
    desk.ask(&drive, "Sum it up.");
    assert_eq!(
        desk.asked[0].scope, 2,
        "the request is about what was chosen"
    );
    desk.finished(&drive);

    // A new selection is followed again.
    desk.following = 1;
    drive.settle(&mut desk);
    assert_eq!(desk.assist.scope(), 1);
    let seen = desk.seen(&drive);
    assert!(seen.iter().any(|text| text == "Paragraph"), "{seen:?}");
}

#[test]
fn the_panes_menu_opens_settings_clears_the_conversation_and_carries_the_hosts_rows() {
    let scripted = Scripted::new([Turn::says("First."), Turn::says("Second.")]);
    let heard = scripted.heard();
    let (_scratch, drive, mut desk) = ready("menu", vec![Box::new(scripted)]);
    desk.menu = vec!["&Accept All", "&Reject All"];
    desk.ask(&drive, "One.");
    desk.finished(&drive);
    assert_eq!(desk.assist.transcript().len(), 2);

    let open = |desk: &mut Desk| {
        let more = more_button(drive.ctx()).expect("the pane drew its menu button");
        drive.click(desk, more.center());
        drive.settle(desk);
        drive.settle(desk);
    };
    open(&mut desk);
    let (rows, menus) = crate::menu::innermost_rows(drive.ctx());
    assert_eq!(menus, 1);
    let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Settings…",
            "Clear the Conversation",
            "Accept All",
            "Reject All"
        ]
    );
    assert!(crate::menu::clashes(drive.ctx()).is_empty());

    drive.press(&mut desk, "A");
    drive.settle(&mut desk);
    assert_eq!(desk.chosen, [Chosen::Menu(0)], "the application's own row");
    open(&mut desk);
    drive.press(&mut desk, "R");
    drive.settle(&mut desk);
    assert_eq!(desk.chosen, [Chosen::Menu(0), Chosen::Menu(1)]);

    open(&mut desk);
    drive.press(&mut desk, "S");
    drive.settle(&mut desk);
    assert!(desk.assist.box_up(), "Settings… opens the box");
    drive.press(&mut desk, "Escape");
    drive.settle(&mut desk);
    assert!(!desk.assist.box_up());

    open(&mut desk);
    drive.press(&mut desk, "C");
    drive.settle(&mut desk);
    assert!(
        desk.assist.transcript().is_empty(),
        "the transcript is empty"
    );
    desk.ask(&drive, "Two.");
    desk.finished(&drive);
    let heard = heard.lock().unwrap();
    assert_eq!(
        heard[1]
            .conversation
            .messages()
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>(),
        ["Context: paragraph 1.\n\nRequest: Two."],
        "and the conversation starts again"
    );
}

#[test]
fn every_part_of_the_pane_is_painted_where_it_can_be_seen() {
    let scripted = Scripted::new([
        Turn::says("It reads well.").and_calls("read_paragraphs", reading(1)),
        Turn::says("I rewrote the first paragraph."),
        Turn::fails(
            FailureKind::Busy,
            "The scripted helper is busy; try again in a moment.",
        ),
    ]);
    let (_scratch, drive, mut desk) = ready("painted", vec![Box::new(scripted)]);
    desk.verbs = vec![("Improve the wording", "Improve the wording.")];
    desk.answers = Box::new(|call: &ToolCall| {
        Some(
            Ran::new(ToolResult::ok(call, "proposed"))
                .said("read paragraph 1")
                .card(Card {
                    id: 7,
                    title: "Replaced paragraph 1".into(),
                    body: "A better sentence.".into(),
                    actions: vec!["Accept".into(), "Reject".into()],
                    verdict: None,
                }),
        )
    });
    desk.ask(&drive, "Improve paragraph 1.");
    desk.finished(&drive);
    desk.ask(&drive, "And the next.");
    desk.finished(&drive);
    desk.assist.set_download(Some(Progress {
        done: 300_000_000,
        total: Some(1_100_000_000),
    }));

    let expected = [
        "Assist",
        "Ollama — qwen3:1.7b · on this computer",
        "Improve paragraph 1.",
        "It reads well.",
        "read paragraph 1",
        "Replaced paragraph 1",
        "A better sentence.",
        "Accept",
        "Reject",
        "I rewrote the first paragraph.",
        "And the next.",
        "The scripted helper is busy; try again in a moment.",
        "Try Again",
        "Downloading the helper: 300 MB of 1.1 GB",
        "Stop",
        "Improve the wording",
        "About:",
        "Paragraph",
        "Say what you want done, in your own words",
        "Send",
    ];
    let pane = desk.pane;
    let painted = desk.painted(&drive);
    assert!(
        pane.width() >= WIDTH - 1.0,
        "the pane has its width: {pane:?}"
    );
    for words in expected {
        let text = painted
            .text(words)
            .unwrap_or_else(|| panic!("“{words}” was not painted: {:?}", painted.strings()));
        let shown = text.shown();
        assert!(
            shown.width() > 1.0 && shown.height() > 1.0,
            "“{words}” is painted into a clip with no room: {:?} in {:?}",
            text.rect,
            text.clip
        );
        assert!(
            pane.contains_rect(shown),
            "“{words}” is outside the pane: {shown:?} in {pane:?}"
        );
    }

    // The card's buttons say which was pressed, and a settled card says how.
    desk.click(&drive, "Accept");
    assert_eq!(desk.chosen, [Chosen::Card { card: 7, action: 0 }]);
    desk.assist.settle(7, "Accepted");
    let seen = desk.seen(&drive);
    assert!(seen.iter().any(|text| text == "Accepted"), "{seen:?}");
    assert!(!seen.iter().any(|text| text == "Reject"), "{seen:?}");

    // The first-run card, too, and in a small window.
    let scratch = Scratch::new("painted-card");
    let reach = Fake::new().finds(vec![Row::Local, Row::ClaudeWithKey, Row::Service]);
    let drive = Driver::sized(egui::vec2(900.0, 600.0));
    let mut desk = Desk::with(reach, scratch.settings());
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    let pane = desk.pane;
    let painted = desk.painted(&drive);
    for words in [
        QUESTION,
        "A helper on this computer",
        "Claude, over the internet",
        "Use this",
        "Not now",
        "Choose a helper first",
    ] {
        let text = painted
            .text(words)
            .unwrap_or_else(|| panic!("“{words}” was not painted: {:?}", painted.strings()));
        let shown = text.shown();
        assert!(
            shown.width() > 1.0 && shown.height() > 1.0 && pane.contains_rect(shown),
            "“{words}” is not where it can be seen: {shown:?} in {pane:?}"
        );
    }
}

#[test]
fn the_download_bar_fills_as_the_bytes_arrive_and_offers_stop() {
    let (_scratch, drive, mut desk) = ready("download", Vec::new());
    let bar = |desk: &mut Desk| {
        let painted = desk.painted(&drive);
        let height = |rect: &egui::Rect| (rect.height() - 6.0).abs() < 0.01;
        let track = painted
            .filled(theme::CHROME_RULE)
            .into_iter()
            .find(|rect| height(rect));
        let fill = painted
            .filled(theme::ACCENT)
            .into_iter()
            .find(|rect| height(rect));
        (painted, track, fill)
    };

    desk.assist.set_download(Some(Progress {
        done: 250_000_000,
        total: Some(1_000_000_000),
    }));
    let (painted, track, fill) = bar(&mut desk);
    assert!(painted
        .text("Downloading the helper: 250 MB of 1.0 GB")
        .is_some());
    let (track, fill) = (track.expect("a track"), fill.expect("a fill"));
    assert!(track.width() > 200.0, "{track:?}");
    assert_eq!(fill.left(), track.left());
    assert!(
        (fill.width() / track.width() - 0.25).abs() < 0.01,
        "a quarter: {fill:?} of {track:?}"
    );

    desk.assist.set_download(Some(Progress {
        done: 750_000_000,
        total: Some(1_000_000_000),
    }));
    let (painted, track, fill) = bar(&mut desk);
    assert!(painted
        .text("Downloading the helper: 750 MB of 1.0 GB")
        .is_some());
    let (track, fill) = (track.unwrap(), fill.unwrap());
    assert!((fill.width() / track.width() - 0.75).abs() < 0.01);

    // A size not yet known: the bytes, and a track with nothing in it.
    desk.assist.set_download(Some(Progress {
        done: 750_000_000,
        total: None,
    }));
    let (painted, track, fill) = bar(&mut desk);
    assert!(painted.text("Downloading the helper: 750 MB").is_some());
    assert!(track.is_some());
    assert!(fill.is_none());

    desk.click(&drive, "Stop");
    assert_eq!(desk.chosen, [Chosen::StopDownload]);

    desk.assist.set_download(None);
    let seen = desk.seen(&drive);
    assert!(
        !seen.iter().any(|text| text.starts_with("Downloading")),
        "{seen:?}"
    );
}
