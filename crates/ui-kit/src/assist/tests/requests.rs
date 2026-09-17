//! B1 and B2: a request on a thread of its own, and Stop.

use ::assist::{FailureKind, Turn};

use super::*;

#[test]
fn a_request_typed_into_the_composer_streams_its_answer_into_the_transcript() {
    let scratch = Scratch::new("streams");
    let (go, gate) = mpsc::channel();
    let paced = Paced::new(vec![vec![
        Pace::Say("The paragraph "),
        Pace::Wait(gate),
        Pace::Say("reads better now."),
    ]]);
    let heard = Arc::clone(&paced.heard);
    let reach = Fake::new().helpers(vec![Box::new(paced)]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&ollama_here()));
    drive.settle(&mut desk);

    desk.ask(&drive, "Improve the wording.");
    assert_eq!(
        desk.asked,
        [Asked {
            words: "Improve the wording.".into(),
            scope: 1,
            effort: Effort::Usual,
        }],
        "Enter asked, about the scope the selection makes it, with the usual effort"
    );
    assert_eq!(desk.assist.composer(), "", "the words left the composer");

    // The first words are on the screen while the helper is still answering,
    // and the window goes on painting meanwhile.
    desk.until(&drive, "the first words", |desk| {
        matches!(desk.assist.transcript().last(), Some(Entry::Said { .. }))
    });
    let frames = desk.frame;
    for _ in 0..5 {
        drive.settle(&mut desk);
    }
    assert!(desk.frame >= frames + 5, "the window kept painting");
    assert!(desk.assist.is_working());
    assert_eq!(
        desk.assist.transcript(),
        [
            Entry::Asked("Improve the wording.".into()),
            Entry::Said {
                words: "The paragraph ".into(),
                kept: true
            },
        ]
    );
    // A request that cannot go now is not lost: it waits in the composer.
    let waiting = Asked {
        words: "Shorten it too.".into(),
        scope: 1,
        effort: Effort::Usual,
    };
    desk.assist.send(Prepared::new(
        &waiting,
        "Request: Shorten it too.",
        "Paragraph 1",
    ));
    assert_eq!(desk.assist.composer(), "Shorten it too.");
    let seen = desk.seen(&drive);
    assert!(
        seen.iter().any(|text| text == "Improve the wording."),
        "{seen:?}"
    );
    assert!(seen.iter().any(|text| text == "The paragraph"), "{seen:?}");
    assert!(
        seen.iter().any(|text| text == "Stop"),
        "Stop is offered while the request is under way: {seen:?}"
    );

    go.send(()).unwrap();
    desk.finished(&drive);
    assert_eq!(
        desk.assist.transcript(),
        [
            Entry::Asked("Improve the wording.".into()),
            Entry::Said {
                words: "The paragraph reads better now.".into(),
                kept: true
            },
        ],
        "the rest arrived in the same entry"
    );
    let seen = desk.seen(&drive);
    assert!(
        seen.iter()
            .any(|text| text == "The paragraph reads better now."),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|text| text == "Stop"), "{seen:?}");

    // The helper was sent the application's words, not the composer's alone.
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1);
    assert_eq!(
        last_words(&heard[0]),
        "Context: paragraph 1.\n\nRequest: Improve the wording."
    );
    assert_eq!(reach.connections(), 1);
    assert!(
        desk.assist.holds_keyboard(drive.ctx()),
        "the composer keeps the keyboard for the next request"
    );
}

#[test]
fn a_tool_call_is_handed_to_the_host_one_per_frame_and_its_result_goes_back_to_the_helper() {
    let scratch = Scratch::new("tools");
    let scripted = Scripted::new([
        Turn::says("Reading.")
            .and_calls("read_paragraphs", reading(1))
            .and_calls("read_paragraphs", reading(3)),
        Turn::says("Paragraphs 1 and 3 say the same thing."),
    ]);
    let heard = scripted.heard();
    let reach = Fake::new().helpers(vec![Box::new(scripted)]);
    let drive = Driver::new();
    let mut desk = Desk::with(reach, scratch.holding(&ollama_here()));
    // The desk holds the first call, to show that nothing more is handed
    // over while one is out.
    desk.answers = Box::new(|call: &ToolCall| match first_of(call) {
        1 => None,
        n => Some(
            Ran::new(ToolResult::ok(call, format!("the text of paragraph {n}")))
                .said(format!("read paragraph {n}")),
        ),
    });
    drive.settle(&mut desk);
    desk.ask(&drive, "Do any paragraphs repeat?");

    desk.until(&drive, "the first call", |desk| desk.held.is_some());
    for _ in 0..10 {
        drive.settle(&mut desk);
    }
    assert_eq!(
        desk.ran.len(),
        1,
        "the second call waits while the first is out"
    );
    assert!(desk.assist.is_working());
    let held = desk.held.clone().unwrap();
    desk.release(
        Ran::new(ToolResult::ok(&held.tool, "the text of paragraph 1")).said("read paragraph 1"),
    );
    desk.finished(&drive);

    let frames: Vec<usize> = desk.ran.iter().map(|(frame, _, _)| *frame).collect();
    assert_eq!(frames.len(), 2);
    assert!(frames[0] < frames[1], "one call a frame: {frames:?}");
    let here = std::thread::current().id();
    assert!(
        desk.ran.iter().all(|(_, _, thread)| *thread == here),
        "every call ran on the window's thread"
    );
    assert_eq!(
        desk.assist.transcript(),
        [
            Entry::Asked("Do any paragraphs repeat?".into()),
            Entry::Said {
                words: "Reading.".into(),
                kept: true
            },
            Entry::Did("read paragraph 1".into()),
            Entry::Did("read paragraph 3".into()),
            Entry::Said {
                words: "Paragraphs 1 and 3 say the same thing.".into(),
                kept: true
            },
        ]
    );

    // Both results went back, each with its call's id, in one turn.
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 2);
    let results: Vec<(String, String)> = heard[1]
        .conversation
        .messages()
        .last()
        .unwrap()
        .content
        .iter()
        .map(|block| match block {
            Block::ToolResult(result) => (result.id.clone(), result.content.clone()),
            other => panic!("only results in the turn, found {other:?}"),
        })
        .collect();
    assert_eq!(
        results,
        [
            ("call_1".to_owned(), "the text of paragraph 1".to_owned()),
            ("call_2".to_owned(), "the text of paragraph 3".to_owned()),
        ]
    );
}

#[test]
fn words_from_a_request_that_did_not_finish_are_marked_as_not_kept() {
    let scratch = Scratch::new("not-kept");
    let scripted = Scripted::new([
        // Finished: its words stand.
        Turn::says("Done."),
        // Declined after it had begun.
        Turn::says("Here is a start.").and_calls("read_paragraphs", reading(2)),
        Turn::ends(Ending::Declined { category: None }),
        // Failed after it had begun.
        Turn::says("Looking.").and_calls("read_paragraphs", reading(4)),
        Turn::fails(
            FailureKind::Busy,
            "The scripted helper is busy; try again in a moment.",
        ),
        // The next request builds on the first only.
        Turn::says("Yes."),
    ]);
    let heard = scripted.heard();
    let reach = Fake::new().helpers(vec![Box::new(scripted)]);
    let drive = Driver::new();
    let mut desk = Desk::with(reach, scratch.holding(&ollama_here()));
    drive.settle(&mut desk);

    for words in ["Tidy it.", "Rewrite it.", "Check it."] {
        desk.ask(&drive, words);
        desk.finished(&drive);
    }
    let kept = |words: &str| {
        desk.assist
            .transcript()
            .iter()
            .find_map(|entry| match entry {
                Entry::Said { words: said, kept } if said == words => Some(*kept),
                _ => None,
            })
    };
    assert_eq!(kept("Done."), Some(true));
    assert_eq!(kept("Here is a start."), Some(false));
    assert_eq!(kept("Looking."), Some(false));
    let notes: Vec<(String, Option<Action>)> = desk
        .assist
        .transcript()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Note { sentence, action } => Some((sentence.clone(), *action)),
            _ => None,
        })
        .collect();
    assert_eq!(
        notes,
        [
            (
                "The scripted helper declined this request.".to_owned(),
                None
            ),
            (
                "The scripted helper is busy; try again in a moment.".to_owned(),
                Some(Action::Retry)
            ),
        ]
    );
    // What the tools did is still said: it is on the document.
    assert_eq!(
        desk.assist
            .transcript()
            .iter()
            .filter(|entry| matches!(entry, Entry::Did(_)))
            .count(),
        2
    );

    let seen = desk.seen(&drive);
    assert_eq!(
        seen.iter().filter(|text| *text == NOT_KEPT).count(),
        2,
        "each forgotten answer says so: {seen:?}"
    );
    assert!(seen.iter().any(|text| text == "Try Again"), "{seen:?}");

    desk.ask(&drive, "Is it done?");
    desk.finished(&drive);
    let heard = heard.lock().unwrap();
    let last = heard.last().unwrap().conversation.messages();
    let said: Vec<String> = last.iter().map(Message::text).collect();
    assert_eq!(
        said,
        [
            "Context: paragraph 1.\n\nRequest: Tidy it.",
            "Done.",
            "Context: paragraph 1.\n\nRequest: Is it done?",
        ],
        "nothing of the two unfinished requests was sent again"
    );
}

#[test]
fn a_refusal_on_the_requests_thread_is_counted_on_the_windows() {
    let scratch = Scratch::new("refusal");
    let reach = Arc::new(Fake {
        real_helpers: true,
        ..Fake::default()
    });
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&ollama_here()));
    drive.settle(&mut desk);

    let before = crate::headless::helpers_refused();
    desk.ask(&drive, "Improve the wording.");
    desk.finished(&drive);
    assert_eq!(
        crate::headless::helpers_refused() - before,
        1,
        "refused on the request's thread, counted once on this one"
    );
    assert_eq!(
        desk.assist.transcript().last(),
        Some(&Entry::Note {
            sentence: "Ollama is not asked from a test.".into(),
            action: None,
        })
    );

    // The real look is refused too, on its own thread, and counted here: two
    // variables, `ant`, and Ollama.
    let scratch = Scratch::new("refusal-look");
    let mut desk = Desk::new(Assist::with(setup(), Arc::new(Real), scratch.settings()));
    let before = crate::headless::helpers_refused();
    desk.until(&drive, "the card's rows", |desk| {
        matches!(desk.assist.choosing, Some(Choosing::Rows(_)))
    });
    assert_eq!(crate::headless::helpers_refused() - before, 4);
    let Some(Choosing::Rows(rows)) = &desk.assist.choosing else {
        unreachable!()
    };
    assert_eq!(rows.rows, [Row::Local, Row::ClaudeWithKey, Row::Service]);

    // And a real check, from the settings box.
    let scratch = Scratch::new("refusal-check");
    let mut desk = Desk::new(Assist::with(
        setup(),
        Arc::new(Real),
        scratch.holding(&claude_with_key()),
    ));
    drive.settle(&mut desk);
    desk.assist.open_settings();
    desk.click(&drive, "Another service (advanced)");
    desk.click(&drive, "https://example.com/v1");
    drive.type_text(&mut desk, "https://api.example.com/v1");
    let before = crate::headless::helpers_refused();
    desk.click(&drive, "Save");
    desk.until(&drive, "the check's refusal", |_| {
        crate::headless::helpers_refused() > before
    });
    for _ in 0..5 {
        drive.settle(&mut desk);
    }
    assert_eq!(crate::headless::helpers_refused() - before, 1);
    let seen = desk.everywhere(&drive);
    assert!(
        seen.iter()
            .any(|text| text == "api.example.com is not asked from a test."),
        "{seen:?}"
    );
    assert_eq!(
        Settings::read(&scratch.settings()).unwrap().helper,
        Some(Choice::Claude),
        "a check that did not happen keeps nothing"
    );
}

#[test]
fn stop_leaves_the_transcript_with_what_arrived_and_the_composer_ready() {
    let scratch = Scratch::new("stop");
    let (go, gate) = mpsc::channel();
    let paced = Paced::new(vec![
        vec![Pace::Say("It is tidy.")],
        vec![
            Pace::Say("Here is"),
            Pace::Wait(gate),
            Pace::Say(" the rest, which nobody sees."),
        ],
    ]);
    let first = Arc::clone(&paced.heard);
    let second = Paced::new(vec![vec![Pace::Say("Again, then.")]]);
    let heard = Arc::clone(&second.heard);
    let reach = Fake::new().helpers(vec![Box::new(paced), Box::new(second)]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&ollama_here()));
    drive.settle(&mut desk);

    // A request that finished, so that there is a conversation to carry on.
    desk.ask(&drive, "Is it tidy?");
    desk.finished(&drive);

    desk.ask(&drive, "Rewrite it.");
    desk.until(&drive, "the first words", |desk| {
        matches!(desk.assist.transcript().last(), Some(Entry::Said { words, .. }) if words == "Here is")
    });
    desk.click(&drive, "Stop");
    assert!(!desk.assist.is_working(), "stopped at once");
    assert_eq!(
        desk.assist.transcript()[2..],
        [
            Entry::Asked("Rewrite it.".into()),
            Entry::Said {
                words: "Here is".into(),
                kept: false
            },
            Entry::Note {
                sentence: "Stopped.".into(),
                action: None
            },
        ],
        "what arrived stays, marked as not kept"
    );
    drive.settle(&mut desk);
    assert!(
        desk.assist.holds_keyboard(drive.ctx()),
        "the composer has the keyboard again"
    );

    // The helper goes on after the stop, and nothing more of it is shown.
    go.send(()).unwrap();
    desk.until(&drive, "the stopped request to end", |desk| {
        desk.assist.let_go.is_empty()
    });
    assert_eq!(desk.assist.transcript().len(), 5);
    assert_eq!(first.lock().unwrap().len(), 2);

    // The composer is ready: typed into, and sent.
    drive.type_text(&mut desk, "Again.");
    assert_eq!(desk.assist.composer(), "Again.");
    drive.press(&mut desk, "Enter");
    desk.finished(&drive);
    assert_eq!(
        desk.assist.transcript().last(),
        Some(&Entry::Said {
            words: "Again, then.".into(),
            kept: true
        })
    );
    assert_eq!(reach.connections(), 2, "a helper of its own");
    let heard = heard.lock().unwrap();
    assert_eq!(
        heard[0].iter().map(Message::text).collect::<Vec<_>>(),
        [
            "Context: paragraph 1.\n\nRequest: Is it tidy?",
            "It is tidy.",
            "Context: paragraph 1.\n\nRequest: Again.",
        ],
        "from the conversation as it stood before the stopped request"
    );
    assert_eq!(
        desk.assist.transcript()[1],
        Entry::Said {
            words: "It is tidy.".into(),
            kept: true
        },
        "and the answer it carried on from is still kept"
    );

    // A tool waiting on the document is answered as stopped, and what it did
    // is still said once the document has done it.
    let scratch = Scratch::new("stop-tool");
    let scripted = Scripted::new([Turn::calls("read_paragraphs", reading(5))]);
    let tool_heard = scripted.heard();
    let reach = Fake::new().helpers(vec![Box::new(scripted)]);
    let mut desk = Desk::with(reach, scratch.holding(&ollama_here()));
    desk.answers = Box::new(|_| None);
    drive.settle(&mut desk);
    desk.ask(&drive, "Read paragraph 5.");
    desk.until(&drive, "the call", |desk| desk.held.is_some());
    desk.click(&drive, "Stop");
    desk.until(&drive, "the stopped request to end", |desk| {
        desk.assist.let_go.is_empty()
    });
    assert_eq!(
        tool_heard.lock().unwrap().len(),
        1,
        "the stopped tool's result was never sent"
    );
    let held = desk.held.clone().unwrap();
    desk.release(Ran::new(ToolResult::ok(&held.tool, "late")).said("read paragraph 5"));
    assert_eq!(
        desk.assist.transcript().last(),
        Some(&Entry::Did("read paragraph 5".into()))
    );
}

#[test]
fn stop_lets_go_of_a_helper_that_has_not_begun_to_answer() {
    let scratch = Scratch::new("let-go");
    let (go, gate) = mpsc::channel();
    let cost = Usage {
        input: 1200,
        output: 30,
        ..Usage::default()
    };
    let stuck = Paced::new(vec![
        vec![Pace::Say("Earlier.")],
        vec![
            Pace::Wait(gate),
            Pace::Say("Late words."),
            Pace::End(Ending::Finished, cost),
        ],
    ]);
    let stuck_heard = Arc::clone(&stuck.heard);
    let fresh = Paced::new(vec![vec![Pace::Say("A fresh answer.")]]);
    let fresh_heard = Arc::clone(&fresh.heard);
    let reach = Fake::new().helpers(vec![Box::new(stuck), Box::new(fresh)]);
    let drive = Driver::new();
    let mut desk = Desk::with(Arc::clone(&reach), scratch.holding(&ollama_here()));
    drive.settle(&mut desk);

    desk.ask(&drive, "Anything earlier?");
    desk.finished(&drive);
    desk.ask(&drive, "Summarize it.");
    // Asked, and waiting for its first word: a Stop pressed before the helper
    // is asked at all is read before anything is sent.
    desk.until(&drive, "the helper to be asked", |_| {
        stuck_heard.lock().unwrap().len() == 2
    });
    for _ in 0..5 {
        drive.settle(&mut desk);
    }
    assert!(desk.assist.is_working(), "the helper has said nothing yet");
    let pressed = Instant::now();
    desk.click(&drive, "Stop");
    assert!(!desk.assist.is_working());
    assert!(
        pressed.elapsed() < Duration::from_secs(2),
        "the window did not wait for the helper"
    );

    // The next request is answered while the first helper is still stuck.
    desk.ask(&drive, "Shorten it.");
    desk.finished(&drive);
    assert_eq!(
        desk.assist.transcript().last(),
        Some(&Entry::Said {
            words: "A fresh answer.".into(),
            kept: true
        })
    );
    assert_eq!(desk.assist.let_go.len(), 1, "the first is still out there");
    assert_eq!(
        fresh_heard.lock().unwrap()[0]
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>(),
        [
            "Context: paragraph 1.\n\nRequest: Anything earlier?",
            "Earlier.",
            "Context: paragraph 1.\n\nRequest: Shorten it.",
        ],
        "carrying on from before the stopped request"
    );

    // When the first helper at last answers, only what it cost is heard.
    assert_eq!(desk.assist.spent(), Usage::default());
    go.send(()).unwrap();
    desk.until(&drive, "the let-go request to end", |desk| {
        desk.assist.let_go.is_empty()
    });
    assert_eq!(desk.assist.spent(), cost, "what it cost is counted");
    assert!(
        !desk.assist.transcript().iter().any(|entry| matches!(
            entry,
            Entry::Said { words, .. } if words.contains("Late")
        )),
        "and nothing it said is shown"
    );
}

/// A call handed over for a request that was stopped before it was answered
/// is answered to nobody: not to the next request, whose calls may well have
/// the same names.
#[test]
fn a_late_answer_to_a_stopped_request_goes_to_nobody() {
    let scratch = Scratch::new("late-answer");
    let stopped = Scripted::new([Turn::calls("read_paragraphs", reading(1))]);
    let next = Scripted::new([
        Turn::calls("read_paragraphs", reading(2)),
        Turn::says("Paragraph 2 reads well."),
    ]);
    let heard = next.heard();
    let reach = Fake::new().helpers(vec![Box::new(stopped), Box::new(next)]);
    let drive = Driver::new();
    let mut desk = Desk::with(reach, scratch.holding(&ollama_here()));
    desk.answers = Box::new(|_| None);
    drive.settle(&mut desk);

    desk.ask(&drive, "Read paragraph 1.");
    desk.until(&drive, "the first call", |desk| desk.held.is_some());
    let stale = desk.held.take().unwrap();
    desk.click(&drive, "Stop");
    assert!(!desk.assist.wanted(&stale), "nobody waits for it now");

    desk.ask(&drive, "Read paragraph 2.");
    desk.until(&drive, "the second call", |desk| desk.held.is_some());
    let wanted = desk.held.clone().unwrap();
    assert_eq!(stale.tool.id, wanted.tool.id, "the two calls share a name");
    assert!(desk.assist.wanted(&wanted));

    // The document answers the stopped request's call late.
    desk.assist.answer(
        &stale,
        Ran::new(ToolResult::ok(&stale.tool, "RESULT OF THE STOPPED CALL"))
            .said("read paragraph 1"),
    );
    assert!(desk.assist.wanted(&wanted), "the next request still waits");
    desk.release(Ran::new(ToolResult::ok(
        &wanted.tool,
        "RESULT OF PARAGRAPH 2",
    )));
    desk.finished(&drive);

    let heard = heard.lock().unwrap();
    let results: Vec<String> = heard[1]
        .conversation
        .messages()
        .last()
        .unwrap()
        .content
        .iter()
        .filter_map(|block| match block {
            Block::ToolResult(result) => Some(result.content.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(results, ["RESULT OF PARAGRAPH 2"]);
    assert!(
        desk.assist
            .transcript()
            .contains(&Entry::Did("read paragraph 1".into())),
        "what the late call did is still said"
    );
}

/// Try Again asks the request that failed, put into words afresh by the
/// application, and only when nothing else is under way.
#[test]
fn try_again_asks_the_failed_request_afresh() {
    let scratch = Scratch::new("try-again");
    let scripted = Scripted::new([
        Turn::fails(FailureKind::Busy, "The scripted helper is busy."),
        Turn::says("Second done."),
        Turn::calls("read_paragraphs", reading(1)),
        Turn::says("First done."),
    ]);
    let heard = scripted.heard();
    let reach = Fake::new().helpers(vec![Box::new(scripted)]);
    let drive = Driver::new();
    let mut desk = Desk::with(reach, scratch.holding(&ollama_here()));
    desk.answers = Box::new(|_| None);
    drive.settle(&mut desk);

    desk.ask(&drive, "First request.");
    desk.finished(&drive);
    desk.following = 0;
    desk.ask(&drive, "Second request.");
    desk.finished(&drive);
    assert_eq!(desk.asked.len(), 2);

    // The first request's Try Again, pressed after the second finished.
    desk.click(&drive, "Try Again");
    assert_eq!(
        desk.asked.last(),
        Some(&Asked {
            words: "First request.".into(),
            scope: 1,
            effort: Effort::Usual,
        }),
        "the failed request, about the scope it was about"
    );
    desk.until(&drive, "the retry's call", |desk| desk.held.is_some());
    assert_eq!(
        last_words(
            heard
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .conversation
                .messages()
        ),
        "Context: paragraph 1.\n\nRequest: First request.",
        "put into words by the application again"
    );

    // While it runs, Try Again does nothing, and a draft in the composer is
    // not replaced by a request that cannot go.
    desk.click(&drive, "Try Again");
    assert_eq!(desk.asked.len(), 3);
    desk.assist.focus();
    drive.settle(&mut desk);
    drive.settle(&mut desk);
    drive.type_text(&mut desk, "my own words");
    let waiting = Asked {
        words: "Something else.".into(),
        scope: 0,
        effort: Effort::Usual,
    };
    desk.assist.send(Prepared::new(
        &waiting,
        "Request: Something else.",
        "Paragraph 1",
    ));
    assert_eq!(desk.assist.composer(), "my own words");

    desk.release(Ran::new(ToolResult::ok(
        &desk.held.clone().unwrap().tool,
        "paragraph one",
    )));
    desk.finished(&drive);
    assert_eq!(
        desk.assist.transcript().last(),
        Some(&Entry::Said {
            words: "First done.".into(),
            kept: true
        })
    );
}

/// A helper that records, once it is let go, whether it was told to stop.
struct Watched {
    gate: Option<mpsc::Receiver<()>>,
    asked: Arc<std::sync::atomic::AtomicBool>,
    stopped: Arc<Mutex<Option<bool>>>,
}

impl Provider for Watched {
    fn name(&self) -> &str {
        "The watched helper"
    }

    fn answer(&mut self, _: &Request, stop: &StopFlag, _: &mut dyn FnMut(&str)) -> Answer {
        self.asked.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(gate) = self.gate.take() {
            let _ = gate.recv_timeout(PATIENCE);
        }
        *self.stopped.lock().unwrap() = Some(stop.is_set());
        Answer {
            message: Message::assistant(Vec::new()),
            ending: Ok(Ending::Stopped),
            usage: Usage::default(),
        }
    }
}

/// A window that goes stops what its pane asked: nothing more is sent, or
/// paid for, for a pane nobody can see.
#[test]
fn a_pane_that_goes_stops_what_it_asked() {
    let scratch = Scratch::new("dropped");
    let (go, gate) = mpsc::channel();
    let asked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = Arc::new(Mutex::new(None));
    let watched = Watched {
        gate: Some(gate),
        asked: Arc::clone(&asked),
        stopped: Arc::clone(&stopped),
    };
    let reach = Fake::new().helpers(vec![Box::new(watched)]);
    let drive = Driver::new();
    let mut desk = Desk::with(reach, scratch.holding(&ollama_here()));
    drive.settle(&mut desk);
    desk.ask(&drive, "Summarize it.");
    desk.until(&drive, "the helper to be asked", |_| {
        asked.load(std::sync::atomic::Ordering::SeqCst)
    });
    drop(desk);
    go.send(()).unwrap();
    let started = Instant::now();
    while stopped.lock().unwrap().is_none() {
        assert!(started.elapsed() < PATIENCE, "the helper never finished");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(*stopped.lock().unwrap(), Some(true), "it was told to stop");
}

/// A call answered twice reaches the helper once: the second answer does not
/// fill the next call's place.
#[test]
fn a_call_answered_twice_reaches_the_helper_once() {
    let scratch = Scratch::new("answered-twice");
    let scripted = Scripted::new([
        Turn::calls("read_paragraphs", reading(1)),
        Turn::calls("read_paragraphs", reading(2)),
        Turn::says("Both read."),
    ]);
    let heard = scripted.heard();
    let reach = Fake::new().helpers(vec![Box::new(scripted)]);
    let drive = Driver::new();
    let mut desk = Desk::with(reach, scratch.holding(&ollama_here()));
    desk.answers = Box::new(|_| None);
    drive.settle(&mut desk);
    desk.ask(&drive, "Read two paragraphs.");

    desk.until(&drive, "the first call", |desk| desk.held.is_some());
    let first = desk.held.clone().unwrap();
    desk.release(Ran::new(ToolResult::ok(&first.tool, "PARAGRAPH ONE")));
    desk.until(&drive, "the second call", |desk| desk.held.is_some());
    let second = desk.held.clone().unwrap();
    assert!(!desk.assist.wanted(&first), "the first is answered");
    assert!(desk.assist.wanted(&second));
    desk.assist.answer(
        &first,
        Ran::new(ToolResult::ok(&first.tool, "PARAGRAPH ONE, AGAIN")),
    );
    assert!(desk.assist.wanted(&second), "the second still waits");
    desk.release(Ran::new(ToolResult::ok(&second.tool, "PARAGRAPH TWO")));
    desk.finished(&drive);

    let heard = heard.lock().unwrap();
    let results = |exchange: usize| -> Vec<String> {
        heard[exchange]
            .conversation
            .messages()
            .last()
            .unwrap()
            .content
            .iter()
            .filter_map(|block| match block {
                Block::ToolResult(result) => Some(result.content.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(results(1), ["PARAGRAPH ONE"]);
    assert_eq!(results(2), ["PARAGRAPH TWO"]);
}
