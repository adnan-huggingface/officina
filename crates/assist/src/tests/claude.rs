//! A2: Claude, over a socket.

use serde_json::json;

use super::*;
use crate::anthropic::{Anthropic, Login};
use crate::conversation::{Block, ToolCall};
use crate::event::{Ending, FailureKind, Usage};
use crate::session::Session;

fn with_key(address: &str, model: &str) -> Anthropic {
    Anthropic::new(
        address,
        Box::new(|| Ok(Login::Key("sk-ant-test".into()))),
        model,
    )
}

#[test]
fn an_anthropic_stream_over_a_socket_becomes_text_tool_calls_and_a_stop() {
    let server = serve(vec![Reply::events(&claude_reading())]);
    let (events, answer) = ask(
        with_key(&server.address, "claude-opus-5"),
        reading(),
        "Improve it.",
    );
    assert_eq!(events, reading_events());
    assert_eq!(answer.ending, Ok(Ending::WantsTools));
    assert_eq!(
        answer.message.content,
        vec![
            Block::Opaque {
                service: "anthropic",
                block: json!({"type": "thinking", "thinking": "", "signature": "c2lnbmVk"}),
            },
            Block::Text("Reading.".into()),
            Block::ToolCall(ToolCall {
                id: "call_1".into(),
                name: "read_paragraphs".into(),
                input: json!({"first": 1, "last": 2}),
            }),
        ]
    );
    assert_eq!(
        answer.usage,
        Usage {
            input: 120,
            output: 42,
            cache_read: 900,
            cache_write: 0,
        }
    );
    assert_eq!(server.heard().path, "/v1/messages");

    let server = serve(vec![Reply::events(&claude_says("Done."))]);
    let (events, answer) = ask(
        with_key(&server.address, "claude-opus-5"),
        reading(),
        "Thanks.",
    );
    assert_eq!(
        events,
        [Event::Text("Done.".into()), Event::Done(Ending::Finished)]
    );
    assert_eq!(answer.message.content, [Block::Text("Done.".into())]);
}

#[test]
fn the_request_to_anthropic_names_the_model_the_strict_tools_the_cache_and_the_fallback() {
    let server = serve(vec![
        Reply::events(&claude_says("Done.")),
        Reply::events(&claude_says("Done.")),
        Reply::events(&claude_says("Done.")),
    ]);
    let quick = |provider: Anthropic| {
        exchange(
            provider,
            reading(),
            asking("Fix the spelling."),
            Effort::Low,
            StopFlag::new(),
            |_| {},
        )
    };

    quick(with_key(&server.address, "claude-opus-5"));
    let heard = server.heard();
    assert_eq!(heard.path, "/v1/messages");
    assert_eq!(heard.headers["x-api-key"], "sk-ant-test");
    assert_eq!(heard.headers["anthropic-version"], "2023-06-01");
    assert_eq!(
        heard.headers["anthropic-beta"],
        "server-side-fallback-2026-07-01"
    );
    assert!(!heard.headers.contains_key("authorization"));
    let expected = json!({
        "model": "claude-opus-5",
        "max_tokens": 64000,
        "stream": true,
        "cache_control": {"type": "ephemeral"},
        "system": [{"type": "text", "text": SYSTEM, "cache_control": {"type": "ephemeral"}}],
        "tools": [{
            "name": "read_paragraphs",
            "description": reading()[0].description,
            "input_schema": reading()[0].schema,
            "strict": true,
        }],
        "messages": [{"role": "user", "content": [{"type": "text", "text": "Fix the spelling."}]}],
        "output_config": {"effort": "low"},
        "fallbacks": "default",
    });
    assert_eq!(
        heard.body, expected,
        "thinking is left at the model's default"
    );
    assert!(
        reading().iter().all(Tool::is_strict),
        "a strict tool's schema is closed"
    );

    // Haiku takes no effort and has no fallback, and is sent neither.
    quick(with_key(&server.address, "claude-haiku-4-5"));
    let heard = server.heard();
    assert_eq!(heard.body["model"], "claude-haiku-4-5");
    assert!(heard.body.get("output_config").is_none());
    assert!(heard.body.get("fallbacks").is_none());
    assert!(!heard.headers.contains_key("anthropic-beta"));

    // And the fallback can be turned off.
    quick(with_key(&server.address, "claude-opus-5").fallback(false));
    let heard = server.heard();
    assert!(heard.body.get("fallbacks").is_none());
    assert!(!heard.headers.contains_key("anthropic-beta"));
    assert_eq!(heard.body["output_config"], json!({"effort": "low"}));
}

/// Claude's answer after a fallback took over halfway: the declined model
/// thought, said a few words and began a tool call; the model that continued
/// thought, finished the sentence and called a tool of its own.
fn fallback_midway() -> String {
    let block = |index: u64, block: Value| {
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": index, "content_block": block}),
        )
    };
    let delta = |index: u64, delta: Value| {
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": index, "delta": delta}),
        )
    };
    let stop = |index: u64| {
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": index}),
        )
    };
    sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_3", "type": "message", "role": "assistant", "model": "claude-opus-5",
            "content": [], "stop_reason": null, "usage": {"input_tokens": 50, "output_tokens": 1}}}),
        ),
        block(
            0,
            json!({"type": "thinking", "thinking": "", "signature": ""}),
        ),
        delta(
            0,
            json!({"type": "signature_delta", "signature": "declined-signature"}),
        ),
        stop(0),
        block(1, json!({"type": "text", "text": ""})),
        delta(1, json!({"type": "text_delta", "text": "I will "})),
        stop(1),
        block(
            2,
            json!({"type": "tool_use", "id": "toolu_declined", "name": "read_paragraphs", "input": {}}),
        ),
        delta(
            2,
            json!({"type": "input_json_delta", "partial_json": "{\"first\": 9, \"last\": 9}"}),
        ),
        stop(2),
        block(
            3,
            json!({"type": "fallback", "from": {"model": "claude-opus-5"}, "to": {"model": "claude-opus-4-8"}}),
        ),
        stop(3),
        block(
            4,
            json!({"type": "thinking", "thinking": "", "signature": ""}),
        ),
        delta(
            4,
            json!({"type": "signature_delta", "signature": "kept-signature"}),
        ),
        stop(4),
        block(5, json!({"type": "text", "text": ""})),
        delta(5, json!({"type": "text_delta", "text": "read them."})),
        stop(5),
        block(
            6,
            json!({"type": "tool_use", "id": "call_7", "name": "read_paragraphs", "input": {}}),
        ),
        delta(
            6,
            json!({"type": "input_json_delta", "partial_json": "{\"first\": 1, \"last\": 2}"}),
        ),
        stop(6),
        (
            "message_delta",
            json!({"type": "message_delta",
            "delta": {"stop_reason": "tool_use", "stop_sequence": null}, "usage": {"output_tokens": 30}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ])
}

#[test]
fn thinking_goes_back_as_it_came_and_what_a_declined_model_began_does_not() {
    // The thinking before a tool call is sent back with the call, untouched.
    let server = serve(vec![
        Reply::events(&claude_reading()),
        Reply::events(&claude_says("Done.")),
    ]);
    let mut session = Session::new(
        Box::new(with_key(&server.address, "claude-opus-5")),
        SYSTEM,
        reading(),
    );
    let mut host = Recorder {
        result: "[1] First.".into(),
        ..Recorder::default()
    };
    let ended = session.ask("Improve it.", Effort::Usual, &StopFlag::new(), &mut host);
    assert_eq!(ended, Ok(Ending::Finished));
    let _first = server.heard();
    let second = server.heard();
    assert_eq!(
        second.body["messages"],
        json!([
            {"role": "user", "content": [{"type": "text", "text": "Improve it."}]},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "", "signature": "c2lnbmVk"},
                {"type": "text", "text": "Reading."},
                {"type": "tool_use", "id": "call_1", "name": "read_paragraphs",
                 "input": {"first": 1, "last": 2}},
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_1", "content": "[1] First. for call_1"},
            ]},
        ])
    );
    assert_eq!(
        session.usage().input,
        120 + 200,
        "each exchange's cost is counted"
    );

    // After a fallback, the declined model's thinking and its call are gone;
    // its words stand, since they were shown and continued.
    let server = serve(vec![
        Reply::events(&fallback_midway()),
        Reply::events(&claude_says("Done.")),
    ]);
    let mut session = Session::new(
        Box::new(with_key(&server.address, "claude-opus-5")),
        SYSTEM,
        reading(),
    );
    let mut host = Recorder::default();
    let ended = session.ask("Improve it.", Effort::Usual, &StopFlag::new(), &mut host);
    assert_eq!(ended, Ok(Ending::Finished));
    let ran: Vec<&str> = host.ran.iter().map(|call| call.id.as_str()).collect();
    assert_eq!(ran, ["call_7"], "the declined model's call is never run");
    let calls_shown = host
        .events
        .iter()
        .filter(|event| matches!(event, Event::ToolCall(_)))
        .count();
    assert_eq!(calls_shown, 1);
    let _first = server.heard();
    let second = server.heard();
    assert_eq!(
        second.body["messages"][1],
        json!({"role": "assistant", "content": [
            {"type": "text", "text": "I will "},
            {"type": "thinking", "thinking": "", "signature": "kept-signature"},
            {"type": "text", "text": "read them."},
            {"type": "tool_use", "id": "call_7", "name": "read_paragraphs",
             "input": {"first": 1, "last": 2}},
        ]})
    );
}

/// A stream in which a tool call is begun and never finished, then `rest`.
fn half_a_call(rest: &[(&str, Value)]) -> String {
    let mut events = vec![
        (
            "message_start",
            json!({"type": "message_start", "message": {
                "id": "msg_h", "type": "message", "role": "assistant", "model": "claude-opus-5",
                "content": [], "stop_reason": null, "usage": {"input_tokens": 50, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block":
                {"type": "tool_use", "id": "toolu_half", "name": "read_paragraphs", "input": {}}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "input_json_delta", "partial_json": "{\"first\": 9,"}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
    ];
    events.extend(rest.iter().cloned());
    events.push(("message_stop", json!({"type": "message_stop"})));
    sse(&events)
}

fn ended(reason: &str) -> (&'static str, Value) {
    (
        "message_delta",
        json!({"type": "message_delta", "delta": {"stop_reason": reason, "stop_sequence": null},
               "usage": {"output_tokens": 5}}),
    )
}

#[test]
fn a_tool_call_cut_short_is_dropped_but_one_in_an_answer_that_stands_must_be_whole() {
    let fallback_then_words = half_a_call(&[
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 1, "content_block":
                {"type": "fallback", "from": {"model": "claude-opus-5"}, "to": {"model": "claude-opus-4-8"}}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 1}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 2,
                   "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 2,
                   "delta": {"type": "text_delta", "text": "Done."}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 2}),
        ),
        ended("end_turn"),
    ]);
    let server = serve(vec![
        Reply::events(&fallback_then_words),
        Reply::events(&half_a_call(&[ended("refusal")])),
        Reply::events(&half_a_call(&[ended("max_tokens")])),
        Reply::events(&half_a_call(&[ended("tool_use")])),
    ]);
    let asked = || {
        ask(
            with_key(&server.address, "claude-opus-5"),
            reading(),
            "Hello.",
        )
    };

    let (events, answer) = asked();
    assert_eq!(
        events,
        [Event::Text("Done.".into()), Event::Done(Ending::Finished)],
        "the declined model's half-written call is no fault"
    );
    assert_eq!(answer.message.content, [Block::Text("Done.".into())]);

    let (events, _) = asked();
    assert_eq!(events, [Event::Done(Ending::Declined { category: None })]);
    let (events, _) = asked();
    assert_eq!(events, [Event::Done(Ending::TooLong)]);

    let (events, answer) = asked();
    assert!(
        matches!(&events[..], [Event::Failed(failure)] if failure.kind == FailureKind::Garbled),
        "{events:?}"
    );
    assert_eq!(answer.usage.input, 50);
}

#[test]
fn a_login_from_ant_is_sent_as_a_bearer_token_and_a_key_as_a_key() {
    let server = serve(vec![
        Reply::events(&claude_says("Done.")),
        Reply::events(&claude_says("Done.")),
    ]);
    let token = Anthropic::new(
        &server.address,
        Box::new(|| Ok(Login::Token("token-from-ant".into()))),
        "claude-opus-5",
    );
    let (events, _) = ask(token, reading(), "Hello.");
    assert_eq!(events.last(), Some(&Event::Done(Ending::Finished)));
    let heard = server.heard();
    assert_eq!(heard.headers["authorization"], "Bearer token-from-ant");
    assert_eq!(
        heard.headers["anthropic-beta"],
        "oauth-2025-04-20,server-side-fallback-2026-07-01"
    );
    assert!(!heard.headers.contains_key("x-api-key"));

    ask(
        with_key(&server.address, "claude-sonnet-5"),
        reading(),
        "Hello.",
    );
    let heard = server.heard();
    assert_eq!(heard.headers["x-api-key"], "sk-ant-test");
    assert!(!heard.headers.contains_key("authorization"));
    assert!(!heard.headers.contains_key("anthropic-beta"));

    // A login that is gone is a sentence, and nothing is sent.
    let server = serve(vec![Reply::events(&claude_says("Done."))]);
    let gone = Anthropic::new(
        &server.address,
        Box::new(|| {
            Err(crate::event::Failure::new(
                FailureKind::Unauthorized,
                "The login from the ant command is not there any more.",
            ))
        }),
        "claude-opus-5",
    );
    let (events, _) = ask(gone, reading(), "Hello.");
    assert!(matches!(
        &events[..],
        [Event::Failed(failure)] if failure.kind == FailureKind::Unauthorized
    ));
    std::thread::sleep(Duration::from_millis(100));
    assert!(server.heard_nothing());
}
