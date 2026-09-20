//! The helper on this computer, proved without a gigabyte or a network: a
//! model of a few dozen kilobytes that the test writes itself, and a server
//! on loopback for the download.

use std::io::Write;
use std::net::TcpListener;

use candle_core::quantized::{GgmlDType, QTensor};
use candle_core::DType;
use serde_json::json;

use super::*;
use crate::conversation::ToolResult;
use crate::conversation::{Conversation, Message};
use crate::Effort;

/// A Qwen3-shaped model of `layers` layers, `hidden` wide, over a vocabulary
/// of `vocab` — written as a real GGUF, so that what the tests load is loaded
/// by the code the person's own weights go through.
///
/// **A quantized block is 32 numbers wide**, so every dimension that is
/// quantized is a multiple of 32; the weights themselves are arbitrary, which
/// is all a test of the machinery needs.
fn tiny_model(path: &Path, layers: usize, hidden: usize, vocab: usize) {
    tiny_model_holding(path, layers, hidden, vocab, 256)
}

/// The same, with the room the model says it was trained to hold.
fn tiny_model_holding(path: &Path, layers: usize, hidden: usize, vocab: usize, context: u32) {
    let device = Device::Cpu;
    let heads = 2usize;
    let head_dim = hidden / heads;
    let ffn = hidden * 2;
    let quantized = |rows: usize, cols: usize| {
        let data: Vec<f32> = (0..rows * cols)
            .map(|i| ((i % 17) as f32 - 8.0) / 32.0)
            .collect();
        let tensor = Tensor::from_vec(data, (rows, cols), &device).expect("a tensor");
        QTensor::quantize(&tensor, GgmlDType::Q4_0).expect("quantized")
    };
    let ones = |n: usize| {
        let tensor = Tensor::ones(n, DType::F32, &device).expect("a tensor");
        QTensor::quantize(&tensor, GgmlDType::F32).expect("a norm")
    };
    let mut tensors: Vec<(String, QTensor)> = vec![
        ("token_embd.weight".into(), quantized(vocab, hidden)),
        ("output_norm.weight".into(), ones(hidden)),
        ("output.weight".into(), quantized(vocab, hidden)),
    ];
    for layer in 0..layers {
        let at = format!("blk.{layer}");
        for (name, tensor) in [
            ("attn_q.weight", quantized(heads * head_dim, hidden)),
            ("attn_k.weight", quantized(heads * head_dim, hidden)),
            ("attn_v.weight", quantized(heads * head_dim, hidden)),
            ("attn_output.weight", quantized(hidden, heads * head_dim)),
            ("attn_q_norm.weight", ones(head_dim)),
            ("attn_k_norm.weight", ones(head_dim)),
            ("attn_norm.weight", ones(hidden)),
            ("ffn_norm.weight", ones(hidden)),
            ("ffn_gate.weight", quantized(ffn, hidden)),
            ("ffn_up.weight", quantized(ffn, hidden)),
            ("ffn_down.weight", quantized(hidden, ffn)),
        ] {
            tensors.push((format!("{at}.{name}"), tensor));
        }
    }
    let metadata: Vec<(&str, gguf_file::Value)> = vec![
        (
            "general.architecture",
            gguf_file::Value::String("qwen3".into()),
        ),
        (
            "qwen3.attention.head_count",
            gguf_file::Value::U32(heads as u32),
        ),
        (
            "qwen3.attention.head_count_kv",
            gguf_file::Value::U32(heads as u32),
        ),
        (
            "qwen3.attention.key_length",
            gguf_file::Value::U32(head_dim as u32),
        ),
        ("qwen3.block_count", gguf_file::Value::U32(layers as u32)),
        (
            "qwen3.embedding_length",
            gguf_file::Value::U32(hidden as u32),
        ),
        ("qwen3.context_length", gguf_file::Value::U32(context)),
        (
            "qwen3.attention.layer_norm_rms_epsilon",
            gguf_file::Value::F32(1e-6),
        ),
        ("qwen3.rope.freq_base", gguf_file::Value::F32(10_000.0)),
    ];
    let metadata: Vec<(&str, &gguf_file::Value)> = metadata
        .iter()
        .map(|(name, value)| (*name, value))
        .collect();
    let tensors: Vec<(&str, &QTensor)> = tensors
        .iter()
        .map(|(name, tensor)| (name.as_str(), tensor))
        .collect();
    let mut file = std::fs::File::create(path).expect("a file");
    gguf_file::write(&mut file, &metadata, &tensors).expect("written");
}

/// A tokenizer of whole words, with the template's own tokens in it: enough
/// to turn a prompt into numbers and numbers back into words.
fn tiny_tokenizer(words: &[&str]) -> Tokenizer {
    use tokenizers::models::wordlevel::WordLevel;
    use tokenizers::pre_tokenizers::whitespace::Whitespace;
    let mut vocab = ahash::AHashMap::new();
    for (id, word) in ["<unk>", "<|im_start|>", "<|im_end|>", "<|endoftext|>"]
        .iter()
        .chain(words.iter())
        .enumerate()
    {
        vocab.insert((*word).to_owned(), id as u32);
    }
    let model = WordLevel::builder()
        .vocab(vocab)
        .unk_token("<unk>".into())
        .build()
        .expect("a vocabulary");
    let mut tokenizer = Tokenizer::new(model);
    tokenizer.with_pre_tokenizer(Some(Whitespace {}));
    tokenizer
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("assist-local-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn asked(words: &str) -> Conversation {
    let mut conversation = Conversation::default();
    conversation.push(Message::user(words));
    conversation
}

/// The runtime generates, stops at the end token, and lets go when Stop is
/// pressed — on a model the test wrote itself, with nothing downloaded.
#[test]
fn a_tiny_random_model_generates_tokens_stops_at_the_end_token_and_honours_stop() {
    let dir = scratch("tiny");
    let path = dir.join("tiny.gguf");
    tiny_model(&path, 2, 64, 96);
    assert!(
        std::fs::metadata(&path).expect("written").len() < 200_000,
        "a test's model is small enough to make every time"
    );

    // Words for the vocabulary, and an end token the model can reach.
    let words: Vec<String> = (0..80).map(|n| format!("w{n}")).collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let tokenizer = tiny_tokenizer(&words);
    let mut file = std::fs::File::open(&path).expect("it opens");
    let mut local = Local::with(&mut file, tokenizer).expect("it loads");
    assert_eq!(local.name(), NAME);

    let mut said = String::new();
    let answer = local.answer(
        &Request {
            system: "You are a helper.",
            tools: &[],
            conversation: &asked("w3 w4"),
            effort: Effort::Low,
        },
        &StopFlag::default(),
        &mut |words| said.push_str(words),
    );
    assert!(answer.ending.is_ok(), "{:?}", answer.ending);
    assert!(answer.usage.input > 0, "the prompt was counted");
    assert!(
        answer.usage.output > 0 && answer.usage.output <= MOST_TOKENS as u64,
        "{:?} tokens",
        answer.usage.output
    );
    assert_eq!(
        said.is_empty(),
        answer.message.content.is_empty(),
        "what was streamed is what came back"
    );

    // The end token ends it. Whatever this model's arbitrary weights make of
    // the prompt, a run whose end tokens are the one it would have made stops
    // at once and says it finished.
    let request = Request {
        system: "You are a helper.",
        tools: &[],
        conversation: &asked("w3 w4"),
        effort: Effort::Low,
    };
    local.decides_greedily();
    local.ends_at(local.said_first(&prompt_for(&request)));
    let answer = local.answer(&request, &StopFlag::default(), &mut |_| {});
    assert!(
        matches!(answer.ending, Ok(Ending::Finished)),
        "{:?}",
        answer.ending
    );
    assert_eq!(answer.usage.output, 0, "it stopped at the very first token");
    assert!(
        answer.message.content.is_empty(),
        "and said nothing after it"
    );

    // Stop pressed while it is writing: it lets go between tokens, and what
    // it had made is what it says. (With no end token to reach, since the
    // check above set one.)
    local.ends_at(Vec::new());
    let stop = StopFlag::default();
    let watching = stop.clone();
    let mut said_words = 0;
    let answer = local.answer(
        &Request {
            system: "You are a helper.",
            tools: &[],
            conversation: &asked("w3 w4"),
            effort: Effort::Low,
        },
        &stop,
        &mut |_| {
            said_words += 1;
            watching.stop();
        },
    );
    assert!(
        matches!(answer.ending, Ok(Ending::Stopped)),
        "{:?}",
        answer.ending
    );
    assert!(
        said_words <= 2,
        "it stopped at the word it was told to: {said_words}"
    );

    // A request longer than the model has room for is said, not sent: a model
    // asked for one token more than its tables hold reaches past their end.
    let small = dir.join("small.gguf");
    tiny_model_holding(&small, 2, 64, 96, 32);
    let mut file = std::fs::File::open(&small).expect("it opens");
    let mut cramped = Local::with(&mut file, tiny_tokenizer(&words)).expect("it loads");
    let mut long = Conversation::default();
    long.push(Message::user(
        (0..60)
            .map(|n| format!("w{}", n % 80))
            .collect::<Vec<_>>()
            .join(" "),
    ));
    let answer = cramped.answer(
        &Request {
            system: "You are a helper.",
            tools: &[],
            conversation: &long,
            effort: Effort::Low,
        },
        &StopFlag::default(),
        &mut |_| {},
    );
    let failed = answer.ending.expect_err("it is refused");
    assert!(
        failed
            .sentence
            .contains("longer than the helper on this computer can hold"),
        "{}",
        failed.sentence
    );

    // Stop before a token is made: nothing is generated, and the answer says
    // so rather than failing.
    let stop = StopFlag::default();
    stop.stop();
    let answer = local.answer(
        &Request {
            system: "You are a helper.",
            tools: &[],
            conversation: &asked("w3"),
            effort: Effort::Low,
        },
        &stop,
        &mut |_| {},
    );
    assert!(matches!(answer.ending, Ok(Ending::Stopped)));
    assert_eq!(answer.usage.output, 0);

    // A model whose weights are not a model at all is said to be, not
    // panicked over.
    let rubbish = dir.join("rubbish.gguf");
    std::fs::write(&rubbish, b"not a model").expect("written");
    let mut file = std::fs::File::open(&rubbish).expect("it opens");
    let failed = Local::with(&mut file, tiny_tokenizer(&words));
    assert!(failed.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The next token is sampled the way the model's card says, not taken
/// greedily: asked the same thing twice, a model of arbitrary weights says
/// two different things. (Greedy decoding says the same thing every time —
/// and, in a small model, the dullest thing it can.)
#[test]
fn the_helper_samples_its_words_rather_than_taking_the_likeliest_every_time() {
    assert_eq!(
        SAMPLING,
        Sampling::TopKThenTopP {
            k: 20,
            p: 0.8,
            temperature: 0.7
        },
        "Qwen's recommendation for thinking off"
    );
    let dir = scratch("sampled");
    let path = dir.join("tiny.gguf");
    tiny_model(&path, 2, 64, 96);
    let words: Vec<String> = (0..80).map(|n| format!("w{n}")).collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let mut file = std::fs::File::open(&path).expect("it opens");
    let mut local = Local::with(&mut file, tiny_tokenizer(&words)).expect("it loads");
    // No end token to reach, so each answer runs to the room the model has.
    local.ends_at(Vec::new());
    let request = Request {
        system: "You are a helper.",
        tools: &[],
        conversation: &asked("w3 w4"),
        effort: Effort::Low,
    };
    let mut answers = Vec::new();
    for _ in 0..2 {
        let mut said = String::new();
        let answer = local.answer(&request, &StopFlag::default(), &mut |words| {
            said.push_str(words)
        });
        assert!(answer.ending.is_ok(), "{:?}", answer.ending);
        assert!(answer.usage.output > 20, "{} tokens", answer.usage.output);
        answers.push(said);
    }
    assert_ne!(
        answers[0], answers[1],
        "two answers to one request are two answers"
    );
}

/// What the model writes becomes the same events every other helper's answer
/// does: words are words, `<tool_call>` is a call, and something that looks
/// like a call but is not stays as the text it is.
#[test]
fn the_models_tool_call_syntax_becomes_the_same_tool_events() {
    let (blocks, wants) = read_answer(
        "I will look at it.\n<tool_call>\n{\"name\": \"read_paragraphs\", \"arguments\": \
         {\"first\": 2, \"last\": 4}}\n</tool_call>",
    );
    assert!(wants, "it wants the tool run");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0], Block::Text("I will look at it.".to_owned()));
    match &blocks[1] {
        Block::ToolCall(call) => {
            assert_eq!(call.name, "read_paragraphs");
            assert_eq!(call.input, json!({"first": 2, "last": 4}));
            assert!(!call.id.is_empty(), "every call is named");
        }
        other => panic!("{other:?}"),
    }

    // Two calls in one answer, and the ids differ.
    let (blocks, wants) = read_answer(
        "<tool_call>{\"name\": \"a\", \"arguments\": {}}</tool_call>\
         <tool_call>{\"name\": \"b\", \"arguments\": {\"x\": 1}}</tool_call>",
    );
    assert!(wants);
    let ids: Vec<String> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::ToolCall(call) => Some(call.id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);

    // Words alone are words alone.
    let (blocks, wants) = read_answer("The third paragraph says the same thing twice.");
    assert!(!wants);
    assert_eq!(blocks.len(), 1);

    // A call whose JSON will not parse is left as what it is, rather than
    // guessed at: the person sees what the model wrote, and nothing runs.
    let (blocks, wants) = read_answer("before <tool_call>{\"name\": oops}</tool_call> after");
    assert!(!wants, "nothing is run on a guess");
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        Block::Text(words) => {
            assert!(words.contains("oops"), "{words}");
            assert!(words.starts_with("before"), "{words}");
        }
        other => panic!("{other:?}"),
    }

    // An opening tag with no closing one is text too.
    let (blocks, wants) = read_answer("mid-thought <tool_call>{\"name\": \"a\"}");
    assert!(!wants);
    assert_eq!(blocks.len(), 1);
}

/// The prompt is the model's own template: the instructions and the tools in
/// the system turn, each turn between its markers, the tools' results as tool
/// responses, and thinking off.
#[test]
fn the_prompt_is_the_models_own_template_with_thinking_off() {
    let tools = vec![Tool::new(
        "read_paragraphs",
        "Read paragraphs first to last.",
        json!({"type": "object", "additionalProperties": false, "required": ["first"],
               "properties": {"first": {"type": "integer"}}}),
    )];
    let mut conversation = Conversation::default();
    conversation.push(Message::user("Improve paragraph 2."));
    conversation.push(Message::assistant(vec![Block::ToolCall(ToolCall {
        id: "call_1".into(),
        name: "read_paragraphs".into(),
        input: json!({"first": 2}),
    })]));
    conversation.push(Message::results(vec![ToolResult {
        id: "call_1".into(),
        content: "[2] The thing about it".into(),
        is_error: false,
    }]));
    let prompt = prompt_for(&Request {
        system: "You are Assist.",
        tools: &tools,
        conversation: &conversation,
        effort: Effort::Low,
    });

    assert!(
        prompt.starts_with("<|im_start|>system\nYou are Assist."),
        "{prompt}"
    );
    assert!(prompt.contains("# Tools"), "{prompt}");
    assert!(prompt.contains("\"name\":\"read_paragraphs\""), "{prompt}");
    assert!(
        prompt.contains("<|im_start|>user\nImprove paragraph 2.<|im_end|>"),
        "{prompt}"
    );
    assert!(prompt.contains("<tool_call>"), "the call it made: {prompt}");
    assert!(
        prompt.contains("<tool_response>\n[2] The thing about it\n</tool_response>"),
        "{prompt}"
    );
    // Thinking off: the answer is opened with an empty think block, which is
    // what the template itself writes when it is asked for.
    assert!(
        prompt.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"),
        "{prompt}"
    );
    // And a helper with no tools is not told about tools.
    let bare = prompt_for(&Request {
        system: "You are Assist.",
        tools: &[],
        conversation: &conversation,
        effort: Effort::Low,
    });
    assert!(!bare.contains("# Tools"), "{bare}");
}

// -------------------------------------------------------------- the download

/// A server on loopback that serves `body`, honours range requests, and can
/// be told to cut the connection after `cut` bytes.
fn serving(body: Vec<u8>, cut: Option<usize>) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            use std::io::Read as _;
            while head.len() < 4 || &head[head.len() - 4..] != b"\r\n\r\n" {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            let head = String::from_utf8_lossy(&head).to_string();
            if head.is_empty() {
                break;
            }
            let from = head
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="))
                .or_else(|| {
                    head.lines()
                        .find_map(|line| line.strip_prefix("Range: bytes="))
                })
                .and_then(|value| value.trim_end_matches('-').parse::<usize>().ok())
                .unwrap_or(0);
            let part = &body[from.min(body.len())..];
            // A server that honours a range says which bytes it is sending,
            // as every real one does: the download checks it before it adds
            // what arrives to what it kept.
            let head = match from {
                0 => format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n", part.len()),
                _ => format!(
                    "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: \
                     bytes {from}-{}/{}\r\n",
                    part.len(),
                    body.len().saturating_sub(1),
                    body.len()
                ),
            };
            let _ = write!(stream, "{head}connection: close\r\n\r\n");
            let sent = match cut {
                Some(cut) if from == 0 => &part[..cut.min(part.len())],
                _ => part,
            };
            let _ = stream.write_all(sent);
            let _ = stream.flush();
            if cut.is_some() && from == 0 {
                // The connection goes while the answer is arriving, as a
                // connection on a train does.
                drop(stream);
                continue;
            }
        }
    });
    (address, handle)
}

/// The download checks what arrived: a file whose hash is not the one
/// Officina expects is refused, removed, and said to be, rather than run.
#[test]
fn a_download_whose_hash_does_not_match_is_refused_and_removed() {
    let dir = scratch("hash");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    let body = b"these are not the weights you are looking for".to_vec();
    let (address, _server) = serving(body.clone(), None);
    let piece = Piece {
        file: "weights.gguf",
        // A leaked address: the server is on loopback and nothing else is.
        url: Box::leak(format!("{address}/weights.gguf").into_boxed_str()),
        bytes: body.len() as u64,
        sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    };
    let http = crate::http::Http::new(&address);
    let failed = fetch(&http, &piece, &folder, &StopFlag::default(), &mut |_| {})
        .expect_err("it is refused");
    assert!(
        failed.sentence.contains("checksum does not match"),
        "{}",
        failed.sentence
    );
    assert!(
        !folder.join("weights.gguf").exists(),
        "and nothing is left to be run"
    );
    assert!(
        !folder.join("weights.gguf.part").exists(),
        "not even half of it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A download cut off part way carries on from where it stopped rather than
/// starting again, and the file it leaves is whole.
#[test]
fn an_interrupted_download_resumes_where_it_stopped() {
    let dir = scratch("resume");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    let body: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
    let sha = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let (address, _server) = serving(body.clone(), Some(20_000));
    let piece = Piece {
        file: "weights.gguf",
        url: Box::leak(format!("{address}/weights.gguf").into_boxed_str()),
        bytes: body.len() as u64,
        sha256: Box::leak(sha.into_boxed_str()),
    };
    let http = crate::http::Http::new(&address);

    // The first attempt is cut off: what arrived is kept.
    let mut first = 0u64;
    let failed = fetch(&http, &piece, &folder, &StopFlag::default(), &mut |got| {
        first = got
    });
    assert!(failed.is_err(), "the connection went");
    let part = folder.join("weights.gguf.part");
    let kept = std::fs::metadata(&part).expect("kept").len();
    assert!(kept > 0 && kept < body.len() as u64, "{kept} bytes of it");

    // The second asks for the rest, and only the rest.
    let mut seen = Vec::new();
    fetch(&http, &piece, &folder, &StopFlag::default(), &mut |got| {
        seen.push(got)
    })
    .expect("it finishes");
    assert!(
        seen.first().is_some_and(|first| *first > kept),
        "it carried on from {kept}: {:?}",
        seen.first()
    );
    let whole = std::fs::read(folder.join("weights.gguf")).expect("it is there");
    assert_eq!(whole, body, "and it is the file that was asked for");
    assert!(!part.exists(), "with nothing half-done left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The weights live in the cache directory, are not fetched twice, and can be
/// removed with a word about how much came back.
#[test]
fn the_weights_are_in_the_cache_directory_and_removing_them_says_what_came_back() {
    let dir = scratch("cache");
    assert!(
        folder(&dir).starts_with(&dir) && folder(&dir).ends_with("qwen3-1.7b-q4-k-m"),
        "{:?}",
        folder(&dir)
    );
    assert!(!have(&dir), "nothing is downloaded yet");

    // Files of the right names and sizes: what `have` looks for.
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    for piece in MODEL.pieces() {
        let file = std::fs::File::create(folder.join(piece.file)).expect("a file");
        file.set_len(piece.bytes).expect("sized");
    }
    assert!(have(&dir), "and now it is there");

    let freed = remove(&dir).expect("removed");
    assert_eq!(freed, MODEL.bytes());
    assert!(!have(&dir));
    assert!(!folder.exists(), "and the folder goes with them");
    assert_eq!(remove(&dir).expect("nothing to remove"), 0);

    // The words the card uses for a size.
    assert_eq!(size_of(MODEL.weights.bytes), "1.3 GB");
    assert_eq!(size_of(11_422_654), "11 MB");
    assert_eq!(size_of(999), "1.0 kB");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pinned model is named, licensed and sized, and its files are asked for
/// by hash — the facts the card shows before anything is downloaded.
#[test]
fn the_pinned_model_says_what_it_is_before_anything_is_downloaded() {
    assert_eq!(MODEL.licence, "Apache-2.0");
    assert!(MODEL.name.contains("Qwen3"));
    assert!(MODEL.about.starts_with("https://"));
    for piece in MODEL.pieces() {
        assert!(piece.url.starts_with("https://"), "{}", piece.url);
        assert_eq!(piece.sha256.len(), 64, "{}", piece.file);
        assert!(
            piece.sha256.chars().all(|c| c.is_ascii_hexdigit()),
            "{}",
            piece.sha256
        );
        assert!(piece.bytes > 0);
    }
    assert!(
        MODEL.bytes() > MODEL.weights.bytes,
        "both files are counted"
    );
    assert!(GOOD_AT.contains("rewording"), "{GOOD_AT}");
}

/// No test downloads the real weights — in this crate either, whose tests
/// deliberately do not enter offline mode because they serve themselves on
/// loopback. The one function that would reach out for a gigabyte refuses
/// while a test is what is running.
#[test]
fn a_test_never_downloads_the_helper() {
    let dir = scratch("offline");
    let failed = download_model(&dir, &StopFlag::default(), &mut |_| {})
        .expect_err("no test downloads anything");
    assert!(matches!(failed.kind, FailureKind::Offline), "{failed:?}");
    assert!(!have(&dir), "and nothing landed");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The download's client is the one that may follow a redirect, because what
/// it fetches is a public file asked for with no key; every other client
/// refuses one, so that a key cannot travel to wherever a redirect points.
#[test]
fn only_the_download_follows_a_redirect() {
    let moved = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = format!("http://{}", moved.local_addr().expect("an address"));
    let (target, _server) = serving(b"the file itself".to_vec(), None);
    let to = format!("{target}/file");
    std::thread::spawn(move || {
        for stream in moved.incoming().take(2) {
            let Ok(mut stream) = stream else { break };
            use std::io::Read as _;
            let mut head = [0u8; 1024];
            let _ = stream.read(&mut head);
            let _ = write!(
                stream,
                "HTTP/1.1 302 Found\r\nlocation: {to}\r\ncontent-length: 0\r\nconnection: \
                 close\r\n\r\n"
            );
        }
    });

    let answered = crate::http::Http::for_download(&address)
        .get(&format!("{address}/weights"), &[], NAME)
        .expect("it answers");
    assert_eq!(answered.status, 200, "the download followed it");

    let refused = crate::http::Http::new(&address)
        .get(&format!("{address}/weights"), &[], NAME)
        .expect("it answers");
    assert_eq!(refused.status, 302, "and a helper's client did not");
}

/// Chosen in the settings, the helper on this computer is what a request goes
/// to, and it answers as any other provider does — on a model of the test's
/// own, so that nothing is downloaded and nothing is reached.
#[test]
fn the_local_helper_answers_a_request_as_any_other_provider_does() {
    let dir = scratch("provider");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    tiny_model(&folder.join(MODEL.weights.file), 2, 64, 96);
    let words: Vec<String> = (0..80).map(|n| format!("w{n}")).collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    tiny_tokenizer(&words)
        .save(folder.join(MODEL.tokenizer.file), false)
        .expect("a tokenizer on disk");

    let settings = crate::Settings {
        helper: Some(crate::Choice::Local),
        ..crate::Settings::default()
    };
    // Where the weights are is given, never guessed: told nothing, the helper
    // says it has not been downloaded.
    let nowhere = crate::connect(&settings);
    assert_eq!(
        nowhere.name(),
        "The helper on this computer",
        "told nothing, it is the row that has not been downloaded"
    );
    let mut helper = crate::connect_in(&settings, Some(&dir));
    assert_eq!(helper.name(), NAME, "the model was found and read");

    let answer = helper.answer(
        &Request {
            system: "You are a helper.",
            tools: &[],
            conversation: &asked("w3 w4"),
            effort: Effort::Low,
        },
        &StopFlag::default(),
        &mut |_| {},
    );
    assert!(answer.ending.is_ok(), "{:?}", answer.ending);
    assert!(answer.usage.output > 0, "it said something");
    // What it costs is nothing: the tokens are counted, because a person may
    // want to know how much it wrote, but nothing was bought.
    assert_eq!(answer.usage.cache_read, 0);
    assert_eq!(answer.usage.cache_write, 0);

    // And the check says it is ready, in words that name the model.
    let said = crate::check_in(&settings, Some(&dir)).expect("ready");
    assert!(said.contains(MODEL.name), "{said}");
    assert!(said.contains("rewording"), "{said}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The download fetches what is missing, in order, reporting how far along
/// the whole thing is — and does not fetch again what is already there and
/// right.
#[test]
fn the_download_fetches_what_is_missing_and_no_more() {
    let dir = scratch("both");
    let weights = b"weights, such as they are".to_vec();
    let tokenizer = b"{\"tokenizer\": true}".to_vec();
    let (address, _server) = serving_files(
        [
            ("/weights.gguf", weights.clone()),
            ("/tokenizer.json", tokenizer.clone()),
        ]
        .into(),
    );
    let model = Model {
        name: "A tiny helper",
        licence: "Apache-2.0",
        about: "https://example.invalid/about",
        weights: Piece {
            file: "weights.gguf",
            url: Box::leak(format!("{address}/weights.gguf").into_boxed_str()),
            bytes: weights.len() as u64,
            sha256: Box::leak(hash_bytes(&weights).into_boxed_str()),
        },
        tokenizer: Piece {
            file: "tokenizer.json",
            url: Box::leak(format!("{address}/tokenizer.json").into_boxed_str()),
            bytes: tokenizer.len() as u64,
            sha256: Box::leak(hash_bytes(&tokenizer).into_boxed_str()),
        },
        memory: 1,
    };
    let http = crate::http::Http::new(&address);

    let mut seen: Vec<Progress> = Vec::new();
    download(&http, &model, &dir, &StopFlag::default(), &mut |progress| {
        seen.push(progress)
    })
    .expect("both files");
    let folder = folder(&dir);
    assert_eq!(
        std::fs::read(folder.join("weights.gguf")).expect("there"),
        weights
    );
    assert_eq!(
        std::fs::read(folder.join("tokenizer.json")).expect("there"),
        tokenizer
    );
    assert!(
        seen.iter().all(|p| p.total == model.bytes()),
        "the whole is what the bar shows: {seen:?}"
    );
    assert!(
        seen.last().is_some_and(|p| p.done == model.bytes()),
        "and it ends full: {seen:?}"
    );

    // Again: nothing is fetched twice, and the bar goes straight to full.
    let when = std::fs::metadata(folder.join("weights.gguf"))
        .and_then(|found| found.modified())
        .expect("a time");
    let mut again: Vec<Progress> = Vec::new();
    download(&http, &model, &dir, &StopFlag::default(), &mut |progress| {
        again.push(progress)
    })
    .expect("nothing to do");
    assert_eq!(again.len(), 2, "one report per file: {again:?}");
    assert_eq!(
        std::fs::metadata(folder.join("weights.gguf"))
            .and_then(|found| found.modified())
            .expect("a time"),
        when,
        "the file was not written again"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A loopback server that serves several files by path.
fn serving_files(
    files: std::collections::HashMap<&'static str, Vec<u8>>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            use std::io::Read as _;
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while head.len() < 4 || &head[head.len() - 4..] != b"\r\n\r\n" {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            let head = String::from_utf8_lossy(&head).to_string();
            let path = head
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_owned();
            match files.get(path.as_str()) {
                Some(body) => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                }
            }
            let _ = stream.flush();
        }
    });
    (address, handle)
}

/// A server that serves more than the file is supposed to be is stopped at
/// the first chunk that goes over, and what it wrote is removed: a mirror
/// with the wrong file, or a proxy with a login page, does not fill the disk
/// while the hash waits at the end.
#[test]
fn a_file_longer_than_it_should_be_is_stopped_rather_than_written() {
    let dir = scratch("too-long");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    let body: Vec<u8> = vec![7u8; 4 * 1024 * 1024];
    let (address, _server) = serving(body, None);
    let piece = Piece {
        file: "weights.gguf",
        url: Box::leak(format!("{address}/weights.gguf").into_boxed_str()),
        bytes: 1_000,
        sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    };
    let http = crate::http::Http::new(&address);
    let failed = fetch(&http, &piece, &folder, &StopFlag::default(), &mut |_| {})
        .expect_err("it is stopped");
    assert!(
        failed.sentence.contains("longer than it should be"),
        "{}",
        failed.sentence
    );
    let left: u64 = std::fs::read_dir(&folder)
        .expect("the folder")
        .filter_map(Result::ok)
        .map(|entry| entry.metadata().map(|found| found.len()).unwrap_or(0))
        .sum();
    assert!(left < 1_000_000, "{left} bytes left on disk");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A server that answers a range request with the whole file starts the file
/// again rather than adding it to what was kept — and one that says which
/// bytes it is sending is believed.
#[test]
fn a_part_answer_that_starts_again_is_not_added_to_what_was_kept() {
    let dir = scratch("bad-range");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    let body: Vec<u8> = (0..100_000u32).map(|n| (n % 251) as u8).collect();
    let sha = hash_bytes(&body);
    // Answers 206 but sends the whole body from the start, and says so.
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    let whole = body.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            use std::io::Read as _;
            let mut head = [0u8; 2048];
            let _ = stream.read(&mut head);
            let _ = write!(
                stream,
                "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes \
                 0-{}/{}\r\nconnection: close\r\n\r\n",
                whole.len(),
                whole.len() - 1,
                whole.len()
            );
            let _ = stream.write_all(&whole);
            let _ = stream.flush();
        }
    });
    // Forty thousand bytes of it are already here.
    let part = folder.join("weights.gguf.part");
    std::fs::write(&part, &body[..40_000]).expect("a part");
    let piece = Piece {
        file: "weights.gguf",
        url: Box::leak(format!("{address}/weights.gguf").into_boxed_str()),
        bytes: body.len() as u64,
        sha256: Box::leak(sha.into_boxed_str()),
    };
    let http = crate::http::Http::new(&address);
    fetch(&http, &piece, &folder, &StopFlag::default(), &mut |_| {}).expect("it finishes");
    assert_eq!(
        std::fs::read(folder.join("weights.gguf")).expect("there"),
        body,
        "the file is the file, not the file with a head on it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Removing takes the half-downloads with it, says how much really came
/// back, and leaves no folder behind.
#[test]
fn removing_takes_a_half_download_too() {
    let dir = scratch("half");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    std::fs::write(folder.join(MODEL.tokenizer.file), b"a tokenizer").expect("written");
    std::fs::write(
        folder.join(format!("{}.part", MODEL.weights.file)),
        vec![0u8; 5_000],
    )
    .expect("written");
    assert!(!have(&dir), "half a download is not a helper");
    let freed = remove(&dir).expect("removed");
    assert_eq!(
        freed,
        5_000 + "a tokenizer".len() as u64,
        "what was really there"
    );
    assert!(!folder.exists(), "and the folder goes");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two windows share the cache, and only one of them downloads: the other is
/// told so rather than writing into the same half-file.
#[test]
fn two_downloads_at_once_are_one_download() {
    let dir = scratch("two");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    let body = b"the file".to_vec();
    let (address, _server) = serving_files([("/tokenizer.json", body.clone())].into());
    let model = Model {
        name: "A tiny helper",
        licence: "Apache-2.0",
        about: "https://example.invalid/about",
        weights: Piece {
            file: "tokenizer.json",
            url: Box::leak(format!("{address}/tokenizer.json").into_boxed_str()),
            bytes: body.len() as u64,
            sha256: Box::leak(hash_bytes(&body).into_boxed_str()),
        },
        tokenizer: Piece {
            file: "tokenizer.json",
            url: Box::leak(format!("{address}/tokenizer.json").into_boxed_str()),
            bytes: body.len() as u64,
            sha256: Box::leak(hash_bytes(&body).into_boxed_str()),
        },
        memory: 1,
    };
    let http = crate::http::Http::new(&address);
    // The first has the folder; the second is told to wait rather than
    // writing into the same part file.
    let _first = Lock::taken(&folder).expect("the first takes it");
    let failed = download(&http, &model, &dir, &StopFlag::default(), &mut |_| {})
        .expect_err("the second is refused");
    assert!(
        failed.sentence.contains("already being downloaded"),
        "{}",
        failed.sentence
    );
    drop(_first);
    // Let go, the next one may have it.
    download(&http, &model, &dir, &StopFlag::default(), &mut |_| {}).expect("it downloads");
    assert!(
        !folder.join("downloading").exists(),
        "and gives the folder back"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the person reads is the answer, never the model's own tool-call
/// syntax: the tags and the JSON between them are not words, and nothing of
/// an unfinished call is shown at all.
#[test]
fn the_words_streamed_are_the_answer_without_its_tool_calls() {
    let said = "I will read it.\n<tool_call>\n{\"name\": \"read_paragraphs\", \"arguments\": \
                {\"first\": 2}}\n</tool_call>\nThat is what it says.";
    assert_eq!(
        words_of(said),
        "I will read it.\n\nThat is what it says.",
        "the call is not words"
    );
    // A call still being written shows nothing of itself.
    assert_eq!(
        words_of("Reading it now.\n<tool_call>\n{\"name\": \"read"),
        "Reading it now.\n"
    );
    // And an answer with no call in it is itself.
    assert_eq!(words_of("Nothing to do here."), "Nothing to do here.");

    // Word by word, as the model makes them: the call is never handed over,
    // and a letter that arrives in two halves is mended rather than dropped.
    assert_eq!(next_words("I will", ""), Some("I will".to_owned()));
    assert_eq!(
        next_words("I will read", "I will"),
        Some(" read".to_owned())
    );
    assert_eq!(next_words("I will read", "I will read"), None);
    assert_eq!(
        next_words("Reading.\n<tool_call>\n{\"name\"", "Reading.\n"),
        None,
        "nothing of a call being written"
    );
    assert_eq!(
        next_words(
            "Reading.\n<tool_call>\n{\"name\": \"a\"}\n</tool_call>\nDone.",
            "Reading.\n"
        ),
        Some("\nDone.".to_owned()),
        "and the words after it, without it"
    );
    // A character split across two tokens: what was shown is caught up with.
    assert_eq!(
        next_words("caf\u{FFFD}", ""),
        Some("caf\u{FFFD}".to_owned())
    );
    assert_eq!(
        next_words("café", "caf\u{FFFD}"),
        Some("é".to_owned()),
        "the letter is finished rather than lost"
    );

    // Driven through the runtime: whatever the tiny model says, what is
    // streamed is what `read_answer` would keep as text.
    let dir = scratch("streamed");
    let path = dir.join("tiny.gguf");
    tiny_model(&path, 2, 64, 96);
    let words: Vec<String> = (0..80).map(|n| format!("w{n}")).collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let mut file = std::fs::File::open(&path).expect("it opens");
    let mut local = Local::with(&mut file, tiny_tokenizer(&words)).expect("it loads");
    let mut streamed = String::new();
    let answer = local.answer(
        &Request {
            system: "You are a helper.",
            tools: &[],
            conversation: &asked("w3 w4"),
            effort: Effort::Low,
        },
        &StopFlag::default(),
        &mut |words| streamed.push_str(words),
    );
    let kept: String = answer
        .message
        .content
        .iter()
        .filter_map(|block| match block {
            Block::Text(words) => Some(words.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        streamed.trim(),
        kept.trim(),
        "what was read is what was said"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Half a model is not a helper, and the sentence says what to do about it.
#[test]
fn half_a_download_says_to_download_it_again() {
    let dir = scratch("half-load");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    tiny_model(&folder.join(MODEL.weights.file), 2, 64, 96);
    let failed = match Local::load(&folder) {
        Err(failure) => failure,
        Ok(_) => panic!("the tokenizer is missing"),
    };
    assert!(matches!(failed.kind, FailureKind::NotReady), "{failed:?}");
    assert!(
        failed.sentence.contains("not all there"),
        "{}",
        failed.sentence
    );
    assert!(
        failed.sentence.contains("downloads it again"),
        "{}",
        failed.sentence
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The model is read once and kept: a helper made again — after Stop, after
/// the settings are saved, after the conversation is cleared — is the model
/// already in memory rather than a gigabyte read afresh, and letting go of
/// it is what the weights being removed does.
#[test]
fn the_model_is_read_once_and_kept() {
    let dir = scratch("kept");
    let folder = folder(&dir);
    std::fs::create_dir_all(&folder).expect("a folder");
    tiny_model(&folder.join(MODEL.weights.file), 2, 64, 96);
    let words: Vec<String> = (0..80).map(|n| format!("w{n}")).collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    tiny_tokenizer(&words)
        .save(folder.join(MODEL.tokenizer.file), false)
        .expect("a tokenizer on disk");

    let first = Local::load(&folder).expect("it loads");
    let second = Local::load(&folder).expect("it loads");
    assert!(
        first.is_the_same_model_as(&second),
        "the second request is the model the first read"
    );
    // Dropping one helper does not take the model with it: the next request
    // after a Stop makes a helper afresh and finds it still there.
    drop(first);
    let third = Local::load(&folder).expect("it loads");
    assert!(third.is_the_same_model_as(&second));

    // Let go of — the weights removed, or another helper chosen — and the
    // next one is read from disk.
    forget();
    let after = Local::load(&folder).expect("it loads");
    assert!(
        !after.is_the_same_model_as(&second),
        "what was let go of is not handed out again"
    );
    forget();
    let _ = std::fs::remove_dir_all(&dir);
}
