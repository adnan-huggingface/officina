//! A4: failures, and Stop.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::json;

use super::*;
use crate::anthropic::{Anthropic, Login};
use crate::compatible::Compatible;
use crate::event::{Ending, Failure, FailureKind};

fn claude(address: &str) -> Anthropic {
    Anthropic::new(
        address,
        Box::new(|| Ok(Login::Key("sk-ant-test".into()))),
        "claude-opus-5",
    )
}

fn failure_of(events: &[Event]) -> &Failure {
    match events.last() {
        Some(Event::Failed(failure)) => failure,
        other => panic!("the request should have failed, and ended {other:?}"),
    }
}

#[test]
fn a_refusal_a_rate_limit_and_a_dropped_connection_are_each_a_sentence_not_a_hang() {
    let started = Instant::now();

    let refusal = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_r", "type": "message", "role": "assistant", "model": "claude-opus-5",
            "content": [], "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "I can"}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta",
            "delta": {"stop_reason": "refusal", "stop_sequence": null,
                      "stop_details": {"type": "refusal", "category": "cyber",
                                       "explanation": "The request was declined."}},
            "usage": {"output_tokens": 3}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    let rate_limit = Reply::status(
        429,
        json!({"type": "error", "error": {"type": "rate_limit_error",
               "message": "Number of request tokens has exceeded your per-minute rate limit"}}),
    )
    .header("retry-after", "7");
    let cut_off = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_d", "type": "message", "role": "assistant", "model": "claude-opus-5",
            "content": [], "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "Half an ans"}}),
        ),
    ]);
    let overloaded = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_o", "type": "message", "role": "assistant", "model": "claude-opus-5",
            "content": [], "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        ),
        (
            "error",
            json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
        ),
    ]);
    let server = serve(vec![
        Reply::events(&refusal),
        rate_limit,
        Reply::events(&cut_off).then(Step::Drop),
        Reply::events(&overloaded).then(Step::Drop),
        Reply::status(
            401,
            json!({"type": "error", "error": {
            "type": "authentication_error", "message": "invalid x-api-key"}}),
        ),
    ]);

    let (events, answer) = ask(claude(&server.address), reading(), "Hello.");
    let declined = Ending::Declined {
        category: Some("cyber".into()),
    };
    assert_eq!(events.last(), Some(&Event::Done(declined.clone())));
    assert_eq!(answer.ending, Ok(declined.clone()));
    assert_eq!(
        declined.sentence("Claude").as_deref(),
        Some("Claude declined this request.")
    );

    let (events, _) = ask(claude(&server.address), reading(), "Hello.");
    let failure = failure_of(&events);
    assert_eq!(
        failure.kind,
        FailureKind::RateLimited {
            retry_after: Some(Duration::from_secs(7))
        }
    );
    assert_eq!(
        failure.sentence,
        "Claude has had too many requests; it asks to wait 7 seconds before the next."
    );

    let (events, answer) = ask(claude(&server.address), reading(), "Hello.");
    assert_eq!(
        events[0],
        Event::Text("Half an ans".into()),
        "what arrived was shown"
    );
    assert_eq!(
        answer.usage.input, 10,
        "what was billed before the drop is counted"
    );
    let failure = failure_of(&events);
    assert_eq!(failure.kind, FailureKind::Dropped);
    assert_eq!(
        failure.sentence,
        "The connection to Claude dropped, and the answer was cut off."
    );

    let (events, _) = ask(claude(&server.address), reading(), "Hello.");
    assert_eq!(failure_of(&events).kind, FailureKind::Busy);

    let (events, _) = ask(claude(&server.address), reading(), "Hello.");
    let failure = failure_of(&events);
    assert_eq!(failure.kind, FailureKind::Unauthorized);
    assert_eq!(
        failure.sentence,
        "Claude did not accept the key. It said: “invalid x-api-key”"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "none of them waited on anything"
    );

    // Another service's stream that stops short is the same sentence.
    let server = serve(vec![
        Reply::events(&compatible_reading()[..120]).then(Step::Drop)
    ]);
    let (events, _) = ask(
        Compatible::new("Ollama", &server.address, None, "qwen3"),
        reading(),
        "Hello.",
    );
    assert_eq!(failure_of(&events).kind, FailureKind::Dropped);
}

#[test]
fn a_redirect_is_not_followed_so_the_key_goes_nowhere_else() {
    let elsewhere = serve(vec![Reply::events(&claude_says("Done."))]);
    let moved = Reply::status(302, json!({}))
        .header("location", &format!("{}/v1/messages", elsewhere.address));
    let server = serve(vec![moved]);
    let (events, _) = ask(claude(&server.address), reading(), "Hello.");
    let failure = failure_of(&events);
    assert_eq!(failure.kind, FailureKind::Rejected { status: 302 });
    assert!(
        failure.sentence.contains("not followed"),
        "{}",
        failure.sentence
    );
    assert_eq!(server.heard().headers["x-api-key"], "sk-ant-test");
    assert!(
        elsewhere.heard_nothing(),
        "nothing, and no key, went to the other address"
    );
}

#[test]
fn a_service_that_is_not_there_is_a_sentence_naming_it() {
    let address = nobody_home();
    let host = address.trim_start_matches("http://").to_owned();
    let started = Instant::now();

    let (events, _) = ask(
        Compatible::new("Ollama", &format!("{address}/v1"), None, "qwen3"),
        reading(),
        "Hello.",
    );
    let failure = failure_of(&events);
    assert_eq!(failure.kind, FailureKind::Unreachable);
    assert!(
        failure
            .sentence
            .starts_with(&format!("No answer from Ollama at {host}")),
        "{}",
        failure.sentence
    );

    let (events, _) = ask(claude(&address), reading(), "Hello.");
    let failure = failure_of(&events);
    assert_eq!(failure.kind, FailureKind::Unreachable);
    assert!(
        failure
            .sentence
            .starts_with(&format!("No answer from Claude at {host}")),
        "{}",
        failure.sentence
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a refused connection is known at once"
    );

    // An address that is not one is said to be so.
    let (events, _) = ask(
        Compatible::new("a service", "not an address", None, "m"),
        reading(),
        "Hello.",
    );
    assert_eq!(failure_of(&events).kind, FailureKind::Unreachable);
}

#[test]
fn stop_ends_a_request_between_two_chunks() {
    let head = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_s", "type": "message", "role": "assistant", "model": "claude-opus-5",
            "content": [], "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "Hello"}}),
        ),
    ]);
    let rest = sse(&[(
        "content_block_delta",
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": " world"}}),
    )]);
    // The rest is held back until the test has pressed Stop, so a helper that
    // read on would find it.
    let (go, held) = mpsc::channel();
    let server = serve(vec![Reply::events(&head)
        .then(Step::Wait(held))
        .then(Step::Send(rest))
        .then(Step::AwaitClose)]);

    let stop = StopFlag::new();
    let pressed = stop.clone();
    let go = Arc::new(Mutex::new(Some(go)));
    let (events, answer) = exchange(
        claude(&server.address),
        reading(),
        asking("Hello."),
        Effort::Usual,
        stop,
        move |event| {
            if matches!(event, Event::Text(_)) {
                pressed.stop();
                if let Some(go) = go.lock().unwrap().take() {
                    let _ = go.send(());
                }
            }
        },
    );
    assert_eq!(
        events,
        [Event::Text("Hello".into()), Event::Done(Ending::Stopped)]
    );
    assert_eq!(answer.ending, Ok(Ending::Stopped));
    assert!(server.saw_close(), "the connection was let go");

    // Stop pressed before a request goes out: it does not go.
    let server = serve(vec![Reply::events(&claude_says("Done."))]);
    let pressed = StopFlag::new();
    pressed.stop();
    let (events, _) = exchange(
        claude(&server.address),
        reading(),
        asking("Hello."),
        Effort::Usual,
        pressed,
        |_| {},
    );
    assert_eq!(events, [Event::Done(Ending::Stopped)]);
    std::thread::sleep(Duration::from_millis(100));
    assert!(server.heard_nothing());
}
