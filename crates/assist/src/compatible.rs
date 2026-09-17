//! Ollama, and any other service that speaks the chat-completions shape.
//!
//! **One shape, many servers.** Ollama serves it on the computer itself, and so
//! do most other ways of running a model locally; many hosted services serve it
//! too. The address given is the one the service documents as its base — for
//! Ollama `http://localhost:11434/v1` — and requests go to
//! `<address>/chat/completions`, streamed.
//!
//! The servers that speak it do not all speak it alike, so what is read is
//! forgiving: a tool call's arguments may come as fragments of text or as one
//! object, its id may be missing, and a server may say `stop` where it meant
//! `tool_calls`. What is sent is the plainest form every one of them accepts.

use std::collections::BTreeMap;
use std::io::Read;

use serde_json::{json, Map, Value};

use crate::conversation::{Block, Role, ToolCall};
use crate::event::{Ending, Failure, FailureKind, Usage};
use crate::http::Http;
use crate::provider::{Answer, Provider, Request, StopFlag};
use crate::sse::Sse;

pub struct Compatible {
    name: String,
    address: String,
    key: Option<String>,
    model: String,
    http: Http,
}

impl Compatible {
    /// The service called `name` in the transcript, at `address`, answering
    /// as `model`. `key` is sent as a bearer token when there is one.
    pub fn new(name: &str, address: &str, key: Option<String>, model: &str) -> Compatible {
        let address = address.trim_end_matches('/').to_owned();
        Compatible {
            name: name.to_owned(),
            http: Http::new(&address),
            address,
            key,
            model: model.to_owned(),
        }
    }

    /// What the service says it has, asked of its list (`GET <address>/models`),
    /// which sends nothing and costs nothing: the ids, in its order. A service
    /// that does not take the key says so here.
    pub fn models(&self) -> Result<Vec<String>, Failure> {
        crate::offline::check(&self.name)?;
        let headers: Vec<(&str, String)> = self
            .key
            .iter()
            .map(|key| ("authorization", format!("Bearer {key}")))
            .collect();
        let url = format!("{}/models", self.address);
        let response = self.http.get(&url, &headers, &self.name)?;
        if response.status != 200 {
            return Err(response.failure(&self.name));
        }
        let list: Value = serde_json::from_reader(response.body.take(4 * 1024 * 1024))
            .map_err(|_| Failure::garbled(&self.name, "its list of what it can answer with"))?;
        Ok(list["data"]
            .as_array()
            .map(|data| {
                data.iter()
                    .filter_map(|model| model["id"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The request's body, exactly as it is sent.
    pub fn body(&self, request: &Request) -> Value {
        let mut body = json!({
            "model": self.model,
            "stream": true,
            "messages": messages(request),
        });
        if !request.tools.is_empty() {
            body["tools"] = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.schema,
                        },
                    })
                })
                .collect();
        }
        body
    }
}

impl Provider for Compatible {
    fn name(&self) -> &str {
        &self.name
    }

    fn answer(&mut self, request: &Request, stop: &StopFlag, text: &mut dyn FnMut(&str)) -> Answer {
        if let Err(refused) = crate::offline::check(&self.name) {
            return Answer::failed(refused);
        }
        // Stop pressed while the last tool ran: nothing more is sent.
        if stop.is_set() {
            return Answer::ended(Vec::new(), Ending::Stopped, Usage::default());
        }
        let mut headers = vec![("accept", "text/event-stream".to_owned())];
        if let Some(key) = &self.key {
            headers.push(("authorization", format!("Bearer {key}")));
        }
        let url = format!("{}/chat/completions", self.address);
        let response = match self
            .http
            .post(&url, &headers, &self.body(request), &self.name)
        {
            Ok(response) => response,
            Err(failure) => return Answer::failed(failure),
        };
        if response.status != 200 {
            let status = response.status;
            let mut failure = response.failure(&self.name);
            // Ollama answers a model it does not have with a 404.
            if status == 404 {
                failure = Failure::new(
                    FailureKind::Rejected { status },
                    format!(
                        "{} has no model called “{}”, or no chat at this address. {}",
                        self.name, self.model, failure.sentence
                    ),
                );
            }
            return Answer::failed(failure);
        }
        read_stream(Sse::new(response.body), &self.name, stop, text)
    }
}

/// The conversation in the chat-completions shape, the system prompt first.
fn messages(request: &Request) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": request.system})];
    for message in request.conversation.messages() {
        let words: Vec<&str> = message
            .content
            .iter()
            .filter_map(|block| match block {
                Block::Text(text) if !text.is_empty() => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let words = words.join("\n\n");
        match message.role {
            Role::User => {
                // A tool's result is a message of its own, before any words.
                for block in &message.content {
                    if let Block::ToolResult(result) = block {
                        let content = match result.is_error {
                            true => format!("Error: {}", result.content),
                            false => result.content.clone(),
                        };
                        out.push(json!({
                            "role": "tool",
                            "tool_call_id": result.id,
                            "content": content,
                        }));
                    }
                }
                if !words.is_empty() {
                    out.push(json!({"role": "user", "content": words}));
                }
            }
            Role::Assistant => {
                let calls: Vec<Value> = message
                    .calls()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": {"name": call.name, "arguments": call.input.to_string()},
                        })
                    })
                    .collect();
                // A turn of another service's blocks only has nothing to say
                // here.
                if words.is_empty() && calls.is_empty() {
                    continue;
                }
                let mut said = json!({"role": "assistant", "content": words});
                if !calls.is_empty() {
                    said["tool_calls"] = Value::Array(calls);
                }
                out.push(said);
            }
        }
    }
    out
}

/// A tool call as it is being streamed.
#[derive(Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
    whole: Option<Value>,
}

fn read_stream<R: std::io::BufRead>(
    mut sse: Sse<R>,
    helper: &str,
    stop: &StopFlag,
    text: &mut dyn FnMut(&str),
) -> Answer {
    let mut words = String::new();
    let mut calls: BTreeMap<u64, Call> = BTreeMap::new();
    let mut finish: Option<String> = None;
    let mut usage = Usage::default();
    loop {
        if stop.is_set() {
            return Answer::ended(Vec::new(), Ending::Stopped, usage);
        }
        let event = match sse.next() {
            Ok(Some(event)) => event,
            // Some servers end the stream without `[DONE]` once they have said
            // why they finished.
            Ok(None) if finish.is_some() => break,
            Ok(None) | Err(_) => return Answer::failed(Failure::dropped(helper)),
        };
        if event.data.trim() == "[DONE]" {
            break;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(&event.data) else {
            return Answer::failed(Failure::garbled(helper, "a piece that is not JSON"));
        };
        if let Some(error) = chunk.get("error") {
            let message = error["message"].as_str().or(error.as_str()).unwrap_or("");
            return Answer::failed(Failure::new(
                FailureKind::Rejected { status: 200 },
                format!("{helper} stopped with an error: {message}"),
            ));
        }
        if let Some(n) = chunk["usage"]["prompt_tokens"].as_u64() {
            usage.input = n;
        }
        if let Some(n) = chunk["usage"]["completion_tokens"].as_u64() {
            usage.output = n;
        }
        let choice = &chunk["choices"][0];
        if let Some(piece) = choice["delta"]["content"]
            .as_str()
            .filter(|piece| !piece.is_empty())
        {
            words.push_str(piece);
            text(piece);
        }
        if let Some(pieces) = choice["delta"]["tool_calls"].as_array() {
            for piece in pieces {
                let index = piece["index"].as_u64().unwrap_or(calls.len() as u64);
                let call = calls.entry(index).or_default();
                if let Some(id) = piece["id"].as_str().filter(|id| !id.is_empty()) {
                    call.id = id.to_owned();
                }
                let function = &piece["function"];
                if let Some(name) = function["name"].as_str().filter(|name| !name.is_empty()) {
                    call.name = name.to_owned();
                }
                match &function["arguments"] {
                    Value::String(fragment) => call.arguments.push_str(fragment),
                    Value::Object(_) => call.whole = Some(function["arguments"].clone()),
                    _ => {}
                }
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            finish = Some(reason.to_owned());
        }
    }
    let mut content = Vec::new();
    if !words.is_empty() {
        content.push(Block::Text(words));
    }
    for (index, call) in calls {
        if call.name.is_empty() {
            return Answer::failed(Failure::garbled(helper, "a tool call with no name"));
        }
        let input = match (call.whole, call.arguments.trim()) {
            (Some(whole), _) => whole,
            (None, "") => Value::Object(Map::new()),
            (None, text) => match serde_json::from_str(text) {
                Ok(input) => input,
                Err(_) => {
                    return Answer::failed(Failure::garbled(
                        helper,
                        "a tool call whose arguments are not JSON",
                    ))
                }
            },
        };
        let id = match call.id.is_empty() {
            true => format!("call_{index}"),
            false => call.id,
        };
        content.push(Block::ToolCall(ToolCall {
            id,
            name: call.name,
            input,
        }));
    }
    let asked = content
        .iter()
        .any(|block| matches!(block, Block::ToolCall(_)));
    let ending = match finish.as_deref() {
        Some("length") => Ending::TooLong,
        Some("content_filter") => Ending::Declined { category: None },
        _ if asked => Ending::WantsTools,
        _ => Ending::Finished,
    };
    Answer::ended(content, ending, usage)
}
