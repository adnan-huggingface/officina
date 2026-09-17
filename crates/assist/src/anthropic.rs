//! Claude, through Anthropic's Messages API.
//!
//! Rust has no SDK from Anthropic, so the API is spoken directly, in the shape
//! its reference gives for raw HTTP: `POST /v1/messages` with
//! `anthropic-version: 2023-06-01`, streamed as server-sent events.
//!
//! What every request carries, and why:
//!
//! - **The system prompt and the tools first, unchanging, with the prompt marked
//!   for the cache**, and the request as a whole marked too, so that the
//!   conversation so far is read back from the cache on each step of a tool
//!   loop rather than paid for again.
//! - **Every tool `strict`**, so that a call's input always fits its schema and
//!   the application never has to guess at a malformed one.
//! - **The server's own fallback** (`fallbacks: "default"`, beta
//!   `server-side-fallback-2026-07-01`) on a model that has one: a request a
//!   frontier model's safety classifier declines — a paragraph about a
//!   pharmacology exam can trip it — is answered by the model Anthropic
//!   recommends for that kind of request, in the same call. The user can turn
//!   it off.
//! - **Thinking left at the model's default**, which on Claude Opus 5 is
//!   adaptive, and `effort: low` for a quick verb on a model that takes it.
//!   Thinking comes back as signed blocks that are sent back exactly as they
//!   came.
//!
//! A refusal is an HTTP 200 whose `stop_reason` is `refusal`, so the ending is
//! read before anything else is trusted. When a fallback took over in the
//! middle of an answer, what the declined model began — its thinking, its tool
//! calls — is dropped at the boundary, and only its words go back as they were
//! shown.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::conversation::{Block, Role, ToolCall};
use crate::event::{Ending, Failure, FailureKind, Usage};
use crate::http::Http;
use crate::models;
use crate::provider::{Answer, Effort, Provider, Request, StopFlag};
use crate::sse::Sse;

/// Where Anthropic's API is, unless something says otherwise.
pub const ADDRESS: &str = "https://api.anthropic.com";

/// The tag on the blocks only Anthropic may be sent back.
pub(crate) const SERVICE: &str = "anthropic";

const VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Room for thinking and a reply together. An editor's reply is short, but
/// thinking counts against the same limit, and a stream has no timeout to fear
/// from a generous one.
const MAX_TOKENS: u64 = 64_000;

/// A credential for one request.
#[derive(Clone, PartialEq, Eq)]
pub enum Login {
    /// An API key from the Anthropic Console, sent as `x-api-key`.
    Key(String),
    /// A short-lived token, from `ant` or the environment, sent as a bearer
    /// token with the header OAuth tokens need.
    Token(String),
}

impl std::fmt::Debug for Login {
    // A key is never written anywhere, a log included.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Login::Key(_) => f.write_str("Login::Key(…)"),
            Login::Token(_) => f.write_str("Login::Token(…)"),
        }
    }
}

/// Where a request's credential comes from, asked once per request: a token
/// from `ant` lasts minutes, and a session lasts as long as the window.
pub type LoginSource = Box<dyn FnMut() -> Result<Login, Failure> + Send>;

pub struct Anthropic {
    address: String,
    login: LoginSource,
    model: String,
    fallback: bool,
    http: Http,
}

impl Anthropic {
    pub fn new(address: &str, login: LoginSource, model: &str) -> Anthropic {
        let address = address.trim_end_matches('/').to_owned();
        Anthropic {
            http: Http::new(&address),
            address,
            login,
            model: model.to_owned(),
            fallback: true,
        }
    }

    /// Claude with an API key, at Anthropic's own address.
    pub fn with_key(key: &str, model: &str) -> Anthropic {
        let key = key.to_owned();
        Anthropic::new(
            ADDRESS,
            Box::new(move || Ok(Login::Key(key.clone()))),
            model,
        )
    }

    /// Whether a declined request is handed to the server's fallback.
    pub fn fallback(mut self, on: bool) -> Anthropic {
        self.fallback = on;
        self
    }

    fn uses_fallback(&self) -> bool {
        self.fallback && models::claude_model(&self.model).is_some_and(|model| model.fallback)
    }

    /// The request's body, exactly as it is sent.
    pub fn body(&self, request: &Request) -> Value {
        let mut body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "cache_control": {"type": "ephemeral"},
            "system": [{
                "type": "text",
                "text": request.system,
                "cache_control": {"type": "ephemeral"},
            }],
            "messages": messages(request),
        });
        if !request.tools.is_empty() {
            body["tools"] = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.schema,
                        "strict": true,
                    })
                })
                .collect();
        }
        let takes_effort = models::claude_model(&self.model).is_some_and(|model| model.effort);
        if request.effort == Effort::Low && takes_effort {
            body["output_config"] = json!({"effort": "low"});
        }
        if self.uses_fallback() {
            body["fallbacks"] = json!("default");
        }
        body
    }

    /// Whether Claude takes the login, and offers the model to it — asked of
    /// the Models API (`GET /v1/models/{model}`), which reads no text, writes
    /// none, and so costs nothing. The sentence says what came back.
    pub fn check(&mut self) -> Result<String, Failure> {
        crate::offline::check(self.name())?;
        let login = (self.login)()?;
        let mut headers = vec![("anthropic-version", VERSION.to_owned())];
        match login {
            Login::Key(key) => headers.push(("x-api-key", key)),
            Login::Token(token) => {
                headers.push(("authorization", format!("Bearer {token}")));
                headers.push(("anthropic-beta", OAUTH_BETA.to_owned()));
            }
        }
        let url = format!("{}/v1/models/{}", self.address, self.model);
        let response = self.http.get(&url, &headers, "Claude")?;
        let model = match models::claude_model(&self.model) {
            Some(known) => known.name().to_owned(),
            None => format!("“{}”", self.model),
        };
        match response.status {
            200 => Ok(format!(
                "Claude accepted the key, and {model} is there to answer."
            )),
            // The API says the same for a model that does not exist and one
            // the account may not use, and says it about the model; a 404 about
            // anything else is an address that is not Anthropic's API.
            404 => {
                let said = response.error_message();
                Err(match said.starts_with("model:") {
                    true => Failure::new(
                        FailureKind::Rejected { status: 404 },
                        format!(
                            "Claude accepted the key, but does not offer {model} to it. \
                             Choose another in Assist's settings."
                        ),
                    ),
                    false => Failure::new(
                        FailureKind::Rejected { status: 404 },
                        format!(
                            "{} did not answer as Anthropic's API does (HTTP 404). \
                             Check the address in Assist's settings.",
                            crate::http::host_of(&self.address)
                        ),
                    ),
                })
            }
            _ => Err(response.failure("Claude")),
        }
    }

    fn headers(&self, login: Login) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("anthropic-version", VERSION.to_owned()),
            ("accept", "text/event-stream".to_owned()),
        ];
        let mut betas = Vec::new();
        match login {
            Login::Key(key) => headers.push(("x-api-key", key)),
            Login::Token(token) => {
                headers.push(("authorization", format!("Bearer {token}")));
                betas.push(OAUTH_BETA);
            }
        }
        if self.uses_fallback() {
            betas.push(FALLBACK_BETA);
        }
        if !betas.is_empty() {
            headers.push(("anthropic-beta", betas.join(",")));
        }
        headers
    }
}

impl Provider for Anthropic {
    fn name(&self) -> &str {
        "Claude"
    }

    fn answer(&mut self, request: &Request, stop: &StopFlag, text: &mut dyn FnMut(&str)) -> Answer {
        // Before anything else, the login included: under a test even `ant`
        // is not run.
        if let Err(refused) = crate::offline::check(self.name()) {
            return Answer::failed(refused);
        }
        // Stop pressed while the last tool ran: nothing more is sent, or paid for.
        if stop.is_set() {
            return Answer::ended(Vec::new(), Ending::Stopped, Usage::default());
        }
        let login = match (self.login)() {
            Ok(login) => login,
            Err(failure) => return Answer::failed(failure),
        };
        let url = format!("{}/v1/messages", self.address);
        let response =
            match self
                .http
                .post(&url, &self.headers(login), &self.body(request), "Claude")
            {
                Ok(response) => response,
                Err(failure) => return Answer::failed(failure),
            };
        if response.status != 200 {
            return Answer::failed(response.failure("Claude"));
        }
        read_stream(Sse::new(response.body), stop, text)
    }
}

/// The conversation in the Messages API's shape.
///
/// A message left with nothing to send — a turn of another service's blocks
/// only — is left out, since the API refuses an empty one, and turns of the
/// same role that are then side by side are sent as one: the results of a
/// helper's last tools and the person's next words, when the helper ended its
/// turn with nothing to say.
fn messages(request: &Request) -> Vec<Value> {
    let mut turns: Vec<Value> = Vec::new();
    let sendable = request
        .conversation
        .messages()
        .iter()
        .filter_map(|message| {
            let content: Vec<Value> = message
                .content
                .iter()
                .filter_map(|block| match block {
                    // The API refuses an empty text block.
                    Block::Text(text) if text.is_empty() => None,
                    Block::Text(text) => Some(json!({"type": "text", "text": text})),
                    Block::ToolCall(call) => Some(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.input,
                    })),
                    Block::ToolResult(result) => {
                        let mut block = json!({
                            "type": "tool_result",
                            "tool_use_id": result.id,
                            "content": result.content,
                        });
                        if result.is_error {
                            block["is_error"] = json!(true);
                        }
                        Some(block)
                    }
                    Block::Opaque { service, block } if *service == SERVICE => Some(block.clone()),
                    Block::Opaque { .. } => None,
                })
                .collect();
            let role = match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            (!content.is_empty()).then_some((role, content))
        });
    for (role, content) in sendable {
        match turns.last_mut() {
            Some(last) if last["role"] == role => {
                if let Some(blocks) = last["content"].as_array_mut() {
                    blocks.extend(content);
                }
            }
            _ => turns.push(json!({"role": role, "content": content})),
        }
    }
    turns
}

/// A content block as it is being streamed.
struct Part {
    block: Value,
    /// A tool call's input arrives as fragments of JSON text.
    input: String,
}

/// Reads one streamed message to its end.
pub(crate) fn read_stream<R: std::io::BufRead>(
    mut sse: Sse<R>,
    stop: &StopFlag,
    text: &mut dyn FnMut(&str),
) -> Answer {
    let mut parts: BTreeMap<u64, Part> = BTreeMap::new();
    let mut stop_reason: Option<String> = None;
    let mut category: Option<String> = None;
    let mut usage = Usage::default();
    loop {
        if stop.is_set() {
            return Answer::ended(Vec::new(), Ending::Stopped, usage);
        }
        // What was counted before a failure was billed all the same.
        let failed = |failure: Failure| Answer::failed_with(failure, usage);
        let event = match sse.next() {
            Ok(Some(event)) => event,
            Ok(None) | Err(_) => return failed(Failure::dropped("Claude")),
        };
        let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
            return failed(Failure::garbled("Claude", "an event that is not JSON"));
        };
        let kind = data["type"].as_str().unwrap_or(&event.name);
        match kind {
            "message_start" => read_usage(&mut usage, &data["message"]["usage"]),
            "content_block_start" => {
                let Some(index) = data["index"].as_u64() else {
                    return failed(Failure::garbled("Claude", "a block with no index"));
                };
                let block = data["content_block"].clone();
                if let Some(words) = block["text"].as_str().filter(|words| !words.is_empty()) {
                    text(words);
                }
                parts.insert(
                    index,
                    Part {
                        block,
                        input: String::new(),
                    },
                );
            }
            "content_block_delta" => {
                let part = data["index"]
                    .as_u64()
                    .and_then(|index| parts.get_mut(&index));
                let Some(part) = part else {
                    return failed(Failure::garbled("Claude", "a piece of no block"));
                };
                let delta = &data["delta"];
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let words = delta["text"].as_str().unwrap_or("");
                        append(&mut part.block, "text", words);
                        text(words);
                    }
                    "input_json_delta" => {
                        part.input
                            .push_str(delta["partial_json"].as_str().unwrap_or(""));
                    }
                    "thinking_delta" => {
                        append(
                            &mut part.block,
                            "thinking",
                            delta["thinking"].as_str().unwrap_or(""),
                        );
                    }
                    "signature_delta" => {
                        append(
                            &mut part.block,
                            "signature",
                            delta["signature"].as_str().unwrap_or(""),
                        );
                    }
                    "citations_delta" => {
                        if let Some(object) = part.block.as_object_mut() {
                            let citations = object.entry("citations").or_insert_with(|| json!([]));
                            if let Some(list) = citations.as_array_mut() {
                                list.push(delta["citation"].clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            // A tool call's input is read once the message has ended, when it
            // is known whether the call stands: a model that declined or ran
            // out of room leaves a call's input half written.
            "content_block_stop" => {}
            "message_delta" => {
                if let Some(reason) = data["delta"]["stop_reason"].as_str() {
                    stop_reason = Some(reason.to_owned());
                }
                if let Some(said) = data["delta"]["stop_details"]["category"].as_str() {
                    category = Some(said.to_owned());
                }
                read_usage(&mut usage, &data["usage"]);
            }
            "message_stop" => break,
            "error" => return failed(stream_error(&data["error"])),
            // `ping`, and whatever the API adds later.
            _ => {}
        }
    }
    let ending = match stop_reason.as_deref() {
        Some("tool_use") => Ending::WantsTools,
        Some("max_tokens" | "model_context_window_exceeded") => Ending::TooLong,
        Some("refusal") => Ending::Declined { category },
        _ => Ending::Finished,
    };
    let stands = matches!(ending, Ending::Finished | Ending::WantsTools);
    match kept(parts.into_values().collect(), stands) {
        Ok(content) => Answer::ended(content, ending, usage),
        Err(failure) => Answer::failed_with(failure, usage),
    }
}

/// The blocks of a finished message as the conversation keeps them.
///
/// A `fallback` block marks where a declined model stopped and another took
/// over. Before the last one, only words stand — they were shown, and the model
/// that continued was given them — and everything else the declined model began
/// is dropped: its thinking belongs to it, and its tool calls were never meant
/// to be run.
///
/// A tool call that cannot be read is a fault in an answer that `stands`, and
/// in one that was declined or cut off is only the part it was cut off in.
fn kept(parts: Vec<Part>, stands: bool) -> Result<Vec<Block>, Failure> {
    let boundary = parts
        .iter()
        .rposition(|part| part.block["type"] == "fallback");
    let mut content = Vec::new();
    for (index, Part { block, input }) in parts.into_iter().enumerate() {
        let kind = block["type"].as_str().unwrap_or("").to_owned();
        let declined = boundary.is_some_and(|boundary| index < boundary);
        match kind.as_str() {
            "fallback" => {}
            "text" => {
                let words = block["text"].as_str().unwrap_or("");
                if !words.is_empty() {
                    content.push(Block::Text(words.to_owned()));
                }
            }
            _ if declined => {}
            "tool_use" => {
                let input = match input.trim() {
                    // No fragments: the input came whole, when it came at all.
                    "" => Ok(match &block["input"] {
                        Value::Object(whole) => Value::Object(whole.clone()),
                        _ => Value::Object(Map::new()),
                    }),
                    fragments => serde_json::from_str::<Value>(fragments),
                };
                match (block["id"].as_str(), block["name"].as_str(), input) {
                    (Some(id), Some(name), Ok(input)) => content.push(Block::ToolCall(ToolCall {
                        id: id.to_owned(),
                        name: name.to_owned(),
                        input,
                    })),
                    _ if stands => {
                        return Err(Failure::garbled(
                            "Claude",
                            "a tool call that could not be read",
                        ))
                    }
                    _ => {}
                }
            }
            _ => content.push(Block::Opaque {
                service: SERVICE,
                block,
            }),
        }
    }
    Ok(content)
}

fn append(block: &mut Value, field: &str, more: &str) {
    if let Some(object) = block.as_object_mut() {
        let value = object.entry(field).or_insert_with(|| json!(""));
        let mut joined = value.as_str().unwrap_or("").to_owned();
        joined.push_str(more);
        *value = Value::String(joined);
    }
}

/// The counts a usage object gives; each is the latest, and the output count
/// grows as the answer does.
fn read_usage(usage: &mut Usage, said: &Value) {
    let count = |field: &str| said[field].as_u64();
    if let Some(n) = count("input_tokens") {
        usage.input = n;
    }
    if let Some(n) = count("output_tokens") {
        usage.output = n;
    }
    if let Some(n) = count("cache_read_input_tokens") {
        usage.cache_read = n;
    }
    if let Some(n) = count("cache_creation_input_tokens") {
        usage.cache_write = n;
    }
}

/// An error the service sent inside a stream that had already begun.
fn stream_error(error: &Value) -> Failure {
    use crate::event::FailureKind;
    let message = error["message"].as_str().unwrap_or("");
    match error["type"].as_str().unwrap_or("") {
        "overloaded_error" | "api_error" => Failure::new(
            FailureKind::Busy,
            "Claude is busy or having trouble right now; try again in a moment.",
        ),
        "rate_limit_error" => Failure::new(
            FailureKind::RateLimited { retry_after: None },
            "Claude has had too many requests; wait a little before the next.",
        ),
        other => Failure::new(
            FailureKind::Rejected { status: 200 },
            format!("Claude stopped with an error ({other}): {message}"),
        ),
    }
}
