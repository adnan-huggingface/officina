//! The helpers, driven against listeners the tests open themselves.
//!
//! No test here reaches the network: every HTTP helper is pointed at a
//! `TcpListener` on the loopback address, which serves what the test wrote and
//! hands back what it was sent.

mod claude;
mod compatible;
mod failures;
mod session;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::conversation::{Conversation, Message, ToolCall, ToolResult};
use crate::event::Event;
use crate::provider::{stream, Answer, Effort, Provider, Request, StopFlag};
use crate::session::Host;
use crate::tool::Tool;

pub(crate) const SYSTEM: &str = "You help a person edit the document they have open.";

/// How long anything in these tests may take before it counts as a hang.
const PATIENCE: Duration = Duration::from_secs(20);

/// What the listener was sent.
#[derive(Debug)]
pub(crate) struct Heard {
    pub path: String,
    /// By lowercase name.
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

pub(crate) enum Step {
    /// Bytes of the body, as one chunk.
    Send(String),
    /// Nothing more until the test says so.
    Wait(mpsc::Receiver<()>),
    /// Waits for the client to let the connection go, and reports it.
    AwaitClose,
    /// Drops the connection where it stands, the answer unfinished.
    Drop,
}

pub(crate) struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    steps: Vec<Step>,
}

impl Reply {
    /// A 200 whose body is `stream`, sent an event at a time.
    pub fn events(stream: &str) -> Reply {
        Reply {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            steps: stream
                .split_inclusive("\n\n")
                .map(|event| Step::Send(event.to_owned()))
                .collect(),
        }
    }

    pub fn status(status: u16, body: Value) -> Reply {
        Reply {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            steps: vec![Step::Send(body.to_string())],
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn then(mut self, step: Step) -> Reply {
        self.steps.push(step);
        self
    }
}

pub(crate) struct Server {
    pub address: String,
    heard: mpsc::Receiver<Heard>,
    closed: mpsc::Receiver<()>,
}

impl Server {
    /// The next request the listener was sent.
    pub fn heard(&self) -> Heard {
        self.heard
            .recv_timeout(PATIENCE)
            .expect("the listener was sent a request")
    }

    pub fn heard_nothing(&self) -> bool {
        self.heard.try_recv().is_err()
    }

    /// Whether the client let go of a connection the listener was waiting on.
    pub fn saw_close(&self) -> bool {
        self.closed.recv_timeout(PATIENCE).is_ok()
    }
}

/// A listener that answers one connection per reply, in order.
pub(crate) fn serve(replies: Vec<Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (tell_heard, heard) = mpsc::channel();
    let (tell_closed, closed) = mpsc::channel();
    std::thread::spawn(move || {
        for reply in replies {
            let Ok((socket, _)) = listener.accept() else {
                return;
            };
            socket.set_read_timeout(Some(PATIENCE)).unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let Some(request) = read_request(&mut reader) else {
                continue;
            };
            let _ = tell_heard.send(request);
            answer(socket, reply, &tell_closed);
        }
    });
    Server {
        address,
        heard,
        closed,
    }
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Heard> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_owned();
    let mut headers = BTreeMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|length| length.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    let body = match length {
        0 => Value::Null,
        _ => serde_json::from_slice(&body).ok()?,
    };
    Some(Heard {
        path,
        headers,
        body,
    })
}

fn answer(mut socket: TcpStream, reply: Reply, closed: &mpsc::Sender<()>) {
    let mut head = format!("HTTP/1.1 {} Reply\r\n", reply.status);
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("transfer-encoding: chunked\r\nconnection: close\r\n\r\n");
    if socket.write_all(head.as_bytes()).is_err() {
        return;
    }
    let mut steps = reply.steps.into_iter();
    while let Some(step) = steps.next() {
        match step {
            Step::Send(text) => {
                let chunk = format!("{:x}\r\n{text}\r\n", text.len());
                if socket.write_all(chunk.as_bytes()).is_err() {
                    // The client has gone already, which is what a test
                    // waiting for it to go wants to hear.
                    if steps.any(|step| matches!(step, Step::AwaitClose)) {
                        let _ = closed.send(());
                    }
                    return;
                }
                let _ = socket.flush();
            }
            Step::Wait(go) => {
                let _ = go.recv_timeout(PATIENCE);
            }
            Step::AwaitClose => {
                let mut buffer = [0; 64];
                let gone = match socket.read(&mut buffer) {
                    Ok(0) => true,
                    Ok(_) => false,
                    Err(error) => matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ),
                };
                if gone {
                    let _ = closed.send(());
                }
                return;
            }
            Step::Drop => return,
        }
    }
    let _ = socket.write_all(b"0\r\n\r\n");
}

/// A port nothing listens on.
pub(crate) fn nobody_home() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    address
}

/// Server-sent events, one per `(name, data)`.
pub(crate) fn sse(events: &[(&str, Value)]) -> String {
    events
        .iter()
        .map(|(name, data)| match name.is_empty() {
            true => format!("data: {data}\n\n"),
            false => format!("event: {name}\ndata: {data}\n\n"),
        })
        .collect()
}

/// The one tool most of these tests offer.
pub(crate) fn reading() -> Vec<Tool> {
    vec![Tool::new(
        "read_paragraphs",
        "Read paragraphs first to last of the document, as Markdown. Call this \
         before proposing a change to text you have not been shown.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["first", "last"],
            "properties": {
                "first": {"type": "integer", "description": "The first paragraph's number."},
                "last": {"type": "integer", "description": "The last paragraph's number."},
            },
        }),
    )]
}

pub(crate) fn asking(words: &str) -> Conversation {
    let mut conversation = Conversation::default();
    conversation.push(Message::user(words));
    conversation
}

/// Runs one exchange on a thread of its own, and fails the test if it does
/// not end in good time: a hang is the one outcome no helper may have.
pub(crate) fn exchange(
    mut provider: impl Provider + 'static,
    tools: Vec<Tool>,
    conversation: Conversation,
    effort: Effort,
    stop: StopFlag,
    on_event: impl FnMut(&Event) + Send + 'static,
) -> (Vec<Event>, Answer) {
    let (tell, told) = mpsc::channel();
    std::thread::spawn(move || {
        let mut on_event = on_event;
        let mut events = Vec::new();
        let request = Request {
            system: SYSTEM,
            tools: &tools,
            conversation: &conversation,
            effort,
        };
        let answer = stream(&mut provider, &request, &stop, &mut |event| {
            on_event(&event);
            events.push(event);
        });
        let _ = tell.send((events, answer));
    });
    told.recv_timeout(PATIENCE)
        .expect("the helper answered, or failed, in good time")
}

/// The same exchange, with nothing unusual about it.
pub(crate) fn ask(
    provider: impl Provider + 'static,
    tools: Vec<Tool>,
    words: &str,
) -> (Vec<Event>, Answer) {
    exchange(
        provider,
        tools,
        asking(words),
        Effort::Usual,
        StopFlag::new(),
        |_| {},
    )
}

/// The exchange every helper is asked to give in the same shape: a few words
/// and a request to read paragraphs 1 and 2.
pub(crate) fn reading_events() -> Vec<Event> {
    vec![
        Event::Text("Read".into()),
        Event::Text("ing.".into()),
        Event::ToolCall(ToolCall {
            id: "call_1".into(),
            name: "read_paragraphs".into(),
            input: json!({"first": 1, "last": 2}),
        }),
        Event::Done(crate::event::Ending::WantsTools),
    ]
}

/// That exchange as Claude streams it, thinking first.
pub(crate) fn claude_reading() -> String {
    sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5",
            "content": [], "stop_reason": null,
            "usage": {"input_tokens": 120, "output_tokens": 1,
                      "cache_read_input_tokens": 900, "cache_creation_input_tokens": 0}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "signature_delta", "signature": "c2lnbmVk"}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 1,
            "content_block": {"type": "text", "text": ""}}),
        ),
        ("ping", json!({"type": "ping"})),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1,
            "delta": {"type": "text_delta", "text": "Read"}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1,
            "delta": {"type": "text_delta", "text": "ing."}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 1}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 2,
            "content_block": {"type": "tool_use", "id": "call_1", "name": "read_paragraphs", "input": {}}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 2,
            "delta": {"type": "input_json_delta", "partial_json": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 2,
            "delta": {"type": "input_json_delta", "partial_json": "{\"first\": 1,"}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 2,
            "delta": {"type": "input_json_delta", "partial_json": " \"last\": 2}"}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 2}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta",
            "delta": {"stop_reason": "tool_use", "stop_sequence": null},
            "usage": {"output_tokens": 42}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ])
}

/// A short finished answer, as Claude streams it.
pub(crate) fn claude_says(words: &str) -> String {
    sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {
            "id": "msg_2", "type": "message", "role": "assistant", "model": "claude-opus-5",
            "content": [], "stop_reason": null, "usage": {"input_tokens": 200, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": words}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta",
            "delta": {"stop_reason": "end_turn", "stop_sequence": null},
            "usage": {"output_tokens": 9}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ])
}

/// The reading exchange as a chat-completions server streams it, the tool
/// call's arguments in fragments.
pub(crate) fn compatible_reading() -> String {
    let chunk = |delta: Value, finish: Value| {
        json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "model": "qwen3",
               "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    };
    let mut stream = sse(&[
        (
            "",
            chunk(json!({"role": "assistant", "content": "Read"}), Value::Null),
        ),
        ("", chunk(json!({"content": "ing."}), Value::Null)),
        (
            "",
            chunk(
                json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
            "function": {"name": "read_paragraphs", "arguments": ""}}]}),
                Value::Null,
            ),
        ),
        (
            "",
            chunk(
                json!({"tool_calls": [{"index": 0,
            "function": {"arguments": "{\"first\": 1,"}}]}),
                Value::Null,
            ),
        ),
        (
            "",
            chunk(
                json!({"tool_calls": [{"index": 0,
            "function": {"arguments": " \"last\": 2}"}}]}),
                Value::Null,
            ),
        ),
        ("", chunk(json!({}), json!("tool_calls"))),
    ]);
    stream.push_str("data: [DONE]\n\n");
    stream
}

pub(crate) fn compatible_says(words: &str) -> String {
    let mut stream = sse(&[(
        "",
        json!({"id": "chatcmpl-2", "object": "chat.completion.chunk", "model": "qwen3",
               "choices": [{"index": 0, "delta": {"role": "assistant", "content": words},
                            "finish_reason": "stop"}]}),
    )]);
    stream.push_str("data: [DONE]\n\n");
    stream
}

/// A host that answers every call with `result` and keeps what happened.
#[derive(Default)]
pub(crate) struct Recorder {
    pub events: Vec<Event>,
    pub ran: Vec<ToolCall>,
    pub result: String,
    /// Set once the first call has been run, as a Stop pressed mid-request.
    pub stop_after_first: Option<StopFlag>,
}

impl Host for Recorder {
    fn event(&mut self, event: &Event) {
        self.events.push(event.clone());
    }

    fn run(&mut self, call: &ToolCall) -> ToolResult {
        self.ran.push(call.clone());
        if let Some(stop) = &self.stop_after_first {
            stop.stop();
        }
        ToolResult::ok(call, format!("{} for {}", self.result, call.id))
    }
}
