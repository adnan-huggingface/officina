//! A1: a conversation, a scripted helper, and the loop between them.

use serde_json::json;

use super::*;
use crate::anthropic::{Anthropic, Login};
use crate::compatible::Compatible;
use crate::conversation::{Block, Role};
use crate::event::{Ending, FailureKind};
use crate::scripted::{Scripted, Turn};
use crate::session::{Session, MOST_STEPS};

#[test]
fn a_scripted_provider_plays_its_turns_as_the_events_a_real_one_streams() {
    let scripted = Scripted::new([Turn::says("Read")
        .then_says("ing.")
        .and_calls("read_paragraphs", json!({"first": 1, "last": 2}))]);
    let heard = scripted.heard();
    let (from_script, answer) = ask(scripted, reading(), "Improve the wording.");
    assert_eq!(from_script, reading_events());
    assert_eq!(answer.ending, Ok(Ending::WantsTools));
    assert_eq!(answer.message.text(), "Reading.");

    let claude = serve(vec![Reply::events(&claude_reading())]);
    let (from_claude, _) = ask(
        Anthropic::new(
            &claude.address,
            Box::new(|| Ok(Login::Key("sk-ant-test".into()))),
            "claude-opus-5",
        ),
        reading(),
        "Improve the wording.",
    );
    assert_eq!(from_script, from_claude, "the script speaks as Claude does");

    let ollama = serve(vec![Reply::events(&compatible_reading())]);
    let (from_ollama, _) = ask(
        Compatible::new("Ollama", &format!("{}/v1", ollama.address), None, "qwen3"),
        reading(),
        "Improve the wording.",
    );
    assert_eq!(from_script, from_ollama, "and as Ollama does");

    // And it kept what it was asked.
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].system, SYSTEM);
    assert_eq!(heard[0].tools, ["read_paragraphs"]);
    assert_eq!(heard[0].conversation, asking("Improve the wording."));
}

#[test]
fn a_session_hands_each_tool_call_to_the_host_and_sends_every_result_back_in_one_turn() {
    let scripted = Scripted::new([
        Turn::calls("read_paragraphs", json!({"first": 1, "last": 1}))
            .and_calls("read_paragraphs", json!({"first": 3, "last": 3})),
        Turn::says("Paragraphs 1 and 3 say the same thing."),
    ]);
    let heard = scripted.heard();
    let mut session = Session::new(Box::new(scripted), SYSTEM, reading());
    let mut host = Recorder {
        result: "the text".into(),
        ..Recorder::default()
    };
    let ended = session.ask(
        "Do any paragraphs repeat?",
        Effort::Low,
        &StopFlag::new(),
        &mut host,
    );
    assert_eq!(ended, Ok(Ending::Finished));

    let ids: Vec<&str> = host.ran.iter().map(|call| call.id.as_str()).collect();
    assert_eq!(
        ids,
        ["call_1", "call_2"],
        "each call run, in the order asked"
    );
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Done(_)))
            .count(),
        2,
        "one ending per exchange"
    );
    assert!(matches!(host.events[0], Event::ToolCall(_)));
    assert_eq!(host.events[2], Event::Done(Ending::WantsTools));
    assert_eq!(host.events.last(), Some(&Event::Done(Ending::Finished)));

    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 2);
    assert!(
        heard.iter().all(|asked| asked.effort == Effort::Low),
        "the effort holds for the request"
    );
    let second = heard[1].conversation.messages();
    assert_eq!(second.len(), 3);
    assert_eq!(second[2].role, Role::User);
    let results: Vec<(&str, &str)> = second[2]
        .content
        .iter()
        .map(|block| match block {
            Block::ToolResult(result) => (result.id.as_str(), result.content.as_str()),
            other => panic!("only results in the turn, found {other:?}"),
        })
        .collect();
    assert_eq!(
        results,
        [
            ("call_1", "the text for call_1"),
            ("call_2", "the text for call_2")
        ],
        "both results in one turn, each with its call's id"
    );

    let kept = session.conversation().messages();
    assert_eq!(
        kept.len(),
        4,
        "the request, the calls, the results and the answer"
    );
    assert_eq!(kept[3].text(), "Paragraphs 1 and 3 say the same thing.");
}

#[test]
fn an_answer_with_nothing_to_say_is_not_kept_and_the_next_words_join_the_results() {
    let scripted = Scripted::new([
        Turn::calls("read_paragraphs", json!({"first": 1, "last": 1})),
        Turn::ends(Ending::Finished),
    ]);
    let mut session = Session::new(Box::new(scripted), SYSTEM, reading());
    let ended = session.ask(
        "Check it.",
        Effort::Usual,
        &StopFlag::new(),
        &mut Recorder::default(),
    );
    assert_eq!(ended, Ok(Ending::Finished));
    let kept = session.conversation().messages();
    assert_eq!(
        kept.len(),
        3,
        "the request, the call and the results; no empty answer"
    );
    assert_eq!(kept[2].role, Role::User);

    // Claude ends its turn with nothing after the tools; the next request
    // sends the results and the new words as one turn.
    let silent = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {
                "id": "msg_e", "type": "message", "role": "assistant", "model": "claude-opus-5",
                "content": [], "stop_reason": null, "usage": {"input_tokens": 5, "output_tokens": 1}}}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                   "usage": {"output_tokens": 1}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    let server = serve(vec![
        Reply::events(&claude_reading()),
        Reply::events(&silent),
        Reply::events(&claude_says("Yes.")),
    ]);
    let claude = Anthropic::new(
        &server.address,
        Box::new(|| Ok(Login::Key("sk-ant-test".into()))),
        "claude-opus-5",
    );
    let mut session = Session::new(Box::new(claude), SYSTEM, reading());
    let mut host = Recorder {
        result: "[1] First.".into(),
        ..Recorder::default()
    };
    let stop = StopFlag::new();
    assert_eq!(
        session.ask("Improve it.", Effort::Usual, &stop, &mut host),
        Ok(Ending::Finished)
    );
    assert_eq!(
        session.ask("And now?", Effort::Usual, &stop, &mut host),
        Ok(Ending::Finished)
    );
    let (_, _, third) = (server.heard(), server.heard(), server.heard());
    let messages = third.body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3, "{messages:#?}");
    assert_eq!(
        messages[2],
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "[1] First. for call_1"},
            {"type": "text", "text": "And now?"},
        ]})
    );
    assert!(
        messages
            .iter()
            .all(|message| !message["content"].as_array().unwrap().is_empty()),
        "no message is sent empty"
    );
}

#[test]
fn a_request_that_fails_leaves_the_conversation_as_it_was_before_it() {
    let mut session = Session::new(
        Box::new(Scripted::new([
            Turn::says("Done."),
            // A tool runs, then the helper fails.
            Turn::calls("read_paragraphs", json!({"first": 1, "last": 1})),
            Turn::fails(FailureKind::Busy, "Claude is busy."),
            Turn::ends(Ending::Declined { category: None }),
            Turn::says("Half an answer").then_says(", cut off"),
            // Stop pressed while the tools of the second call run.
            Turn::calls("read_paragraphs", json!({"first": 1, "last": 1}))
                .and_calls("read_paragraphs", json!({"first": 2, "last": 2})),
        ])),
        SYSTEM,
        reading(),
    );
    let mut host = Recorder::default();
    let stop = StopFlag::new();
    assert_eq!(
        session.ask("First.", Effort::Usual, &stop, &mut host),
        Ok(Ending::Finished)
    );
    let settled = session.conversation().clone();
    assert_eq!(settled.len(), 2);

    let failed = session.ask("Second.", Effort::Usual, &stop, &mut host);
    assert_eq!(
        failed.map_err(|failure| failure.kind),
        Err(FailureKind::Busy)
    );
    assert_eq!(host.ran.len(), 1, "the tool before the failure did run");
    assert_eq!(session.conversation(), &settled);

    let declined = session.ask("Third.", Effort::Usual, &stop, &mut host);
    assert_eq!(declined, Ok(Ending::Declined { category: None }));
    assert_eq!(session.conversation(), &settled);

    // Stopped between two pieces of an answer.
    let halted = StopFlag::new();
    let mut stopper = Recorder::default();
    let mut words = 0;
    let ended = {
        struct StopOnWords<'a>(&'a mut Recorder, &'a StopFlag, &'a mut usize);
        impl Host for StopOnWords<'_> {
            fn event(&mut self, event: &Event) {
                if matches!(event, Event::Text(_)) {
                    *self.2 += 1;
                    self.1.stop();
                }
                self.0.event(event);
            }
            fn run(&mut self, call: &ToolCall) -> ToolResult {
                self.0.run(call)
            }
        }
        session.ask(
            "Fourth.",
            Effort::Usual,
            &halted,
            &mut StopOnWords(&mut stopper, &halted, &mut words),
        )
    };
    assert_eq!(ended, Ok(Ending::Stopped));
    assert_eq!(words, 1, "nothing arrived after Stop");
    assert_eq!(session.conversation(), &settled);

    let stopping = StopFlag::new();
    let mut host = Recorder {
        stop_after_first: Some(stopping.clone()),
        ..Recorder::default()
    };
    let ended = session.ask("Fifth.", Effort::Usual, &stopping, &mut host);
    assert_eq!(ended, Ok(Ending::Stopped));
    assert_eq!(host.ran.len(), 1, "no tool runs once Stop is pressed");
    assert_eq!(session.conversation(), &settled);

    // Stop pressed while a request's only tool runs: its result is not sent.
    let scripted = Scripted::new([
        Turn::calls("read_paragraphs", json!({"first": 1, "last": 1})),
        Turn::says("Never said."),
    ]);
    let heard = scripted.heard();
    let mut single = Session::new(Box::new(scripted), SYSTEM, reading());
    let stopping = StopFlag::new();
    let mut host = Recorder {
        stop_after_first: Some(stopping.clone()),
        ..Recorder::default()
    };
    let ended = single.ask("Sixth.", Effort::Usual, &stopping, &mut host);
    assert_eq!(ended, Ok(Ending::Stopped));
    assert_eq!(heard.lock().unwrap().len(), 1, "nothing more was asked");
    assert!(single.conversation().is_empty());

    // A helper that never stops asking is stopped, and forgotten.
    let circling: Vec<Turn> = (0..MOST_STEPS + 5)
        .map(|_| Turn::calls("read_paragraphs", json!({"first": 1, "last": 1})))
        .collect();
    let mut session = Session::new(Box::new(Scripted::new(circling)), SYSTEM, reading());
    let mut host = Recorder::default();
    let ended = session.ask("Go round.", Effort::Usual, &StopFlag::new(), &mut host);
    let failure = ended.unwrap_err();
    assert_eq!(failure.kind, FailureKind::TooManySteps);
    assert!(
        failure.sentence.contains("scripted helper"),
        "{}",
        failure.sentence
    );
    assert_eq!(host.ran.len(), MOST_STEPS);
    assert!(session.conversation().is_empty());
}
