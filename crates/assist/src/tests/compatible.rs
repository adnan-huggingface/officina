//! A3: another service, over a socket.

use serde_json::json;

use super::*;
use crate::compatible::Compatible;
use crate::conversation::{Block, ToolCall};
use crate::event::Ending;
use crate::session::Session;

#[test]
fn a_compatible_stream_over_a_socket_becomes_the_same_events() {
    let server = serve(vec![Reply::events(&compatible_reading())]);
    let address = format!("{}/v1/", server.address);
    let (events, answer) = ask(
        Compatible::new("Ollama", &address, None, "qwen3"),
        reading(),
        "Improve it.",
    );
    assert_eq!(events, reading_events());
    assert_eq!(
        answer.message.content,
        vec![
            Block::Text("Reading.".into()),
            Block::ToolCall(ToolCall {
                id: "call_1".into(),
                name: "read_paragraphs".into(),
                input: json!({"first": 1, "last": 2}),
            }),
        ]
    );
    let heard = server.heard();
    assert_eq!(heard.path, "/v1/chat/completions");
    assert!(
        !heard.headers.contains_key("authorization"),
        "Ollama is sent no key"
    );

    // A server that sends a call whole, with no id, and says `stop` for it.
    let whole = sse(&[(
        "",
        json!({"choices": [{"index": 0, "finish_reason": "stop", "delta": {
            "role": "assistant", "content": "",
            "tool_calls": [{"function": {"name": "read_paragraphs",
                                         "arguments": {"first": 4, "last": 4}}}]}}]}),
    )]);
    let server = serve(vec![Reply::events(&whole)]);
    let (events, _) = ask(
        Compatible::new("Ollama", &server.address, None, "qwen3"),
        reading(),
        "Improve it.",
    );
    assert_eq!(
        events,
        [
            Event::ToolCall(ToolCall {
                id: "call_0".into(),
                name: "read_paragraphs".into(),
                input: json!({"first": 4, "last": 4}),
            }),
            Event::Done(Ending::WantsTools),
        ]
    );
}

#[test]
fn a_tool_result_goes_to_a_compatible_service_as_a_tool_message() {
    let server = serve(vec![
        Reply::events(&compatible_reading()),
        Reply::events(&compatible_says("Done.")),
    ]);
    let service = Compatible::new(
        "example.com",
        &format!("{}/v1", server.address),
        Some("key-1".into()),
        "some-model",
    );
    let mut session = Session::new(Box::new(service), SYSTEM, reading());
    let mut host = Recorder {
        result: "[1] First.".into(),
        ..Recorder::default()
    };
    let ended = session.ask("Improve it.", Effort::Low, &StopFlag::new(), &mut host);
    assert_eq!(ended, Ok(Ending::Finished));

    let first = server.heard();
    assert_eq!(first.headers["authorization"], "Bearer key-1");
    assert_eq!(
        first.body,
        json!({
            "model": "some-model",
            "stream": true,
            "messages": [
                {"role": "system", "content": SYSTEM},
                {"role": "user", "content": "Improve it."},
            ],
            "tools": [{"type": "function", "function": {
                "name": "read_paragraphs",
                "description": reading()[0].description,
                "parameters": reading()[0].schema,
            }}],
        }),
        "no effort is sent to a service that has no such thing"
    );
    let second = server.heard();
    assert_eq!(
        second.body["messages"],
        json!([
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": "Improve it."},
            {"role": "assistant", "content": "Reading.", "tool_calls": [{
                "id": "call_1", "type": "function",
                "function": {"name": "read_paragraphs", "arguments": "{\"first\":1,\"last\":2}"},
            }]},
            {"role": "tool", "tool_call_id": "call_1", "content": "[1] First. for call_1"},
        ])
    );
    assert_eq!(session.conversation().messages()[3].text(), "Done.");
}
