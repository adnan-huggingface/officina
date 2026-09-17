//! B4: a key, checked before it is kept, with a request that costs nothing.

use serde_json::{json, Value};

use super::*;
use crate::anthropic::{Anthropic, Login};
use crate::event::FailureKind;
use crate::provider::{check, LOCAL_READY};
use crate::settings::{Choice, Settings};

fn claude_at(address: &str, key: &str, model: &str) -> Settings {
    let mut settings = Settings {
        helper: Some(Choice::Claude),
        ..Settings::default()
    };
    settings.claude.address = address.to_owned();
    settings.claude.key = key.to_owned();
    settings.claude.model = model.to_owned();
    settings
}

/// Claude is asked through the Models API and every other service through its
/// list of models: a GET that sends nothing of any document, reads no text,
/// writes none, and so is never billed. What came back is the sentence the
/// settings box shows.
#[test]
fn a_key_is_checked_with_a_request_that_costs_nothing() {
    let error = |kind: &str, message: &str| json!({"type": "error", "error": {"type": kind, "message": message}});
    let model = json!({"type": "model", "id": "claude-sonnet-5",
                       "display_name": "Claude Sonnet 5", "created_at": "2026-05-01T00:00:00Z"});
    let claude = serve(vec![
        Reply::status(200, model.clone()),
        Reply::status(401, error("authentication_error", "invalid x-api-key")),
        Reply::status(404, error("not_found_error", "model: claude-sonnet-9")),
        Reply::status(404, error("not_found_error", "Not Found")),
        Reply::status(302, json!({})).header("location", "http://elsewhere.test/"),
        Reply::status(200, model),
    ]);

    let mut settings = claude_at(&claude.address, "sk-ant-good", "claude-sonnet-5");
    assert_eq!(
        check(&settings).as_deref(),
        Ok("Claude accepted the key, and Sonnet 5 is there to answer.")
    );
    let heard = claude.heard();
    assert_eq!(
        (heard.method.as_str(), heard.path.as_str()),
        ("GET", "/v1/models/claude-sonnet-5"),
        "the Models API, never the Messages API"
    );
    assert_eq!(heard.headers["x-api-key"], "sk-ant-good");
    assert_eq!(heard.headers["anthropic-version"], "2023-06-01");
    assert!(!heard.headers.contains_key("authorization"));
    assert!(
        !heard.headers.contains_key("anthropic-beta"),
        "a key needs no beta, and the check asks for no fallback"
    );
    assert_eq!(heard.body, Value::Null, "nothing is sent");

    settings.claude.key = "sk-ant-bad".into();
    let refused = check(&settings).expect_err("a key Claude did not accept");
    assert_eq!(refused.kind, FailureKind::Unauthorized);
    assert_eq!(
        refused.sentence,
        "Claude did not accept the key. It said: “invalid x-api-key”"
    );
    claude.heard();

    settings.claude.key = "sk-ant-good".into();
    settings.claude.model = "claude-sonnet-9".into();
    let unknown = check(&settings).expect_err("a model the key is not offered");
    assert_eq!(unknown.kind, FailureKind::Rejected { status: 404 });
    assert_eq!(
        unknown.sentence,
        "Claude accepted the key, but does not offer “claude-sonnet-9” to it. \
         Choose another in Assist's settings."
    );
    assert_eq!(claude.heard().path, "/v1/models/claude-sonnet-9");

    // A 404 about anything but the model: the address is not Anthropic's API.
    let wrong = check(&settings).expect_err("an address that is not the API");
    let host = crate::http::host_of(&claude.address);
    assert_eq!(
        wrong.sentence,
        format!(
            "{host} did not answer as Anthropic's API does (HTTP 404). \
             Check the address in Assist's settings."
        )
    );
    claude.heard();

    // No redirect is followed, here as anywhere: the key would go along.
    let moved = check(&settings).expect_err("a redirect");
    assert_eq!(moved.kind, FailureKind::Rejected { status: 302 });
    claude.heard();

    // A login from `ant` goes as a bearer token with the OAuth beta.
    let mut with_token = Anthropic::new(
        &claude.address,
        Box::new(|| Ok(Login::Token("ant-token".into()))),
        "claude-opus-5",
    );
    assert_eq!(
        with_token.check().as_deref(),
        Ok("Claude accepted the key, and Opus 5 is there to answer.")
    );
    let heard = claude.heard();
    assert_eq!(heard.headers["authorization"], "Bearer ant-token");
    assert_eq!(heard.headers["anthropic-beta"], "oauth-2025-04-20");
    assert!(!heard.headers.contains_key("x-api-key"));

    // No key at all is said without a request.
    settings.claude.key.clear();
    let keyless = check(&settings).expect_err("no key");
    assert_eq!(keyless.kind, FailureKind::Unauthorized);
    assert!(claude.heard_nothing());

    // Ollama: its list, and the model in it, with or without a tag.
    let list = json!({"object": "list", "data": [
        {"id": "qwen3:1.7b", "object": "model", "owned_by": "library"},
        {"id": "llama3.2:latest", "object": "model", "owned_by": "library"}]});
    let tags = json!({"models": [
        {"name": "qwen3:1.7b", "size": 1_359_293_444_u64},
        {"name": "llama3.2:latest", "size": 2_019_393_189_u64}]});
    let ollama = serve(vec![
        Reply::status(200, list.clone()),
        Reply::status(200, tags.clone()),
        Reply::status(200, list.clone()),
        Reply::status(200, tags),
        Reply::status(200, list.clone()),
        Reply::status(200, list.clone()),
    ]);
    let mut settings = Settings {
        helper: Some(Choice::Ollama),
        ..Settings::default()
    };
    settings.ollama.address = ollama.address.clone();
    settings.ollama.model = "qwen3:1.7b".into();
    assert_eq!(
        check(&settings).as_deref(),
        Ok("Ollama answered, and has qwen3:1.7b.")
    );
    let heard = ollama.heard();
    assert_eq!(
        (heard.method.as_str(), heard.path.as_str()),
        ("GET", "/v1/models")
    );
    assert!(
        !heard.headers.contains_key("authorization"),
        "Ollama is sent no key"
    );
    assert_eq!(heard.body, Value::Null);
    assert_eq!(
        ollama.heard().path,
        "/api/tags",
        "and where the model runs, which is here"
    );
    settings.ollama.model = "llama3.2".into();
    assert!(
        check(&settings).is_ok(),
        "a model pulled without a tag is listed under latest"
    );
    ollama.heard();
    ollama.heard();
    settings.ollama.model = "mistral".into();
    let missing = check(&settings).expect_err("a model Ollama does not have");
    assert_eq!(
        missing.sentence,
        "Ollama answered, but has no “mistral”. Pull it with “ollama pull mistral”, \
         or choose another."
    );
    ollama.heard();
    settings.ollama.model.clear();
    assert_eq!(
        check(&settings).expect_err("no model chosen").sentence,
        "Ollama answered. Choose which of its models Assist should use."
    );
    ollama.heard();

    // Another service: its list, with its key as a bearer token.
    let service = serve(vec![
        Reply::status(200, json!({"data": [{"id": "some-model"}]})),
        Reply::status(200, json!({"data": [{"id": "some-model"}]})),
        Reply::status(
            401,
            json!({"error": {"message": "Incorrect API key provided"}}),
        ),
        // No list at an address that already ends in /v1.
        Reply::status(404, json!({"error": {"message": "Not Found"}})),
        Reply::status(500, json!({"error": {"message": "boom"}})),
        // No list at the address without /v1, and one with it.
        Reply::status(404, json!({"error": {"message": "Not Found"}})),
        Reply::status(200, json!({"data": [{"id": "some-model"}]})),
    ]);
    let host = crate::http::host_of(&service.address);
    let mut settings = Settings {
        helper: Some(Choice::Service),
        ..Settings::default()
    };
    settings.service.address = format!("{}/v1", service.address);
    settings.service.key = "sk-service".into();
    settings.service.model = "some-model".into();
    assert_eq!(
        check(&settings),
        Ok(format!("{host} accepted the key, and lists some-model."))
    );
    let heard = service.heard();
    assert_eq!(
        (heard.method.as_str(), heard.path.as_str()),
        ("GET", "/v1/models")
    );
    assert_eq!(heard.headers["authorization"], "Bearer sk-service");
    settings.service.model = "unlisted".into();
    assert_eq!(
        check(&settings),
        Ok(format!(
            "{host} accepted the key. It does not list “unlisted”, which some services \
             do not; if a request fails, check the name."
        ))
    );
    service.heard();
    settings.service.key = "sk-wrong".into();
    let refused = check(&settings).expect_err("a key the service did not accept");
    assert_eq!(refused.kind, FailureKind::Unauthorized);
    assert_eq!(
        refused.sentence,
        format!("{host} did not accept the key. It said: “Incorrect API key provided”")
    );
    service.heard();
    // A service with no list has still answered; one that failed has not.
    assert_eq!(
        check(&settings),
        Ok(format!(
            "{host} answered, but not with a list of what it can answer with, as most \
             services do. The key is checked when the first request goes; if that \
             fails, check the address."
        ))
    );
    assert_eq!(
        service.heard().path,
        "/v1/models",
        "an address that already ends in /v1 is not tried with another"
    );
    assert_eq!(
        check(&settings).expect_err("a service in trouble").kind,
        FailureKind::Busy
    );
    service.heard();
    // An address missing its /v1 is said to be, when the service answers there.
    let mut short = settings.clone();
    short.service.address = service.address.clone();
    let missing = check(&short).expect_err("an address without its /v1");
    assert_eq!(
        missing.sentence,
        format!(
            "{host} answers at {}/v1, not at {}. Add /v1 to the address.",
            service.address, service.address
        )
    );
    assert_eq!(service.heard().path, "/models");
    assert_eq!(service.heard().path, "/v1/models");
    settings.service.address.clear();
    assert_eq!(
        check(&settings).expect_err("no address").sentence,
        "Give the service's address."
    );

    // Nothing, and the helper that is not built yet, are said at once.
    assert_eq!(
        check(&Settings::default()).expect_err("no helper").kind,
        FailureKind::NotReady
    );
    let local = Settings {
        helper: Some(Choice::Local),
        ..Settings::default()
    };
    assert_eq!(check(&local).is_ok(), LOCAL_READY);
}

/// A model Ollama passes on to another server — one of its cloud models, or
/// one made with a remote host — is refused, by the check and by the helper
/// before its first request: Assist says Ollama keeps what is written on this
/// computer, and such a model would not.
#[test]
fn a_model_ollama_passes_on_elsewhere_is_refused_before_anything_is_sent() {
    use crate::provider::{connect, Request};
    use crate::{Effort, StopFlag};

    let list = json!({"object": "list", "data": [
        {"id": "gpt-oss:120b-cloud"}, {"id": "helper:latest"}, {"id": "qwen3:1.7b"}]});
    let tags = json!({"models": [
        {"name": "gpt-oss:120b-cloud", "remote_model": "gpt-oss:120b",
         "remote_host": "https://ollama.com:443", "size": 384},
        {"name": "helper:latest", "remote_model": "big",
         "remote_host": "http://gpu.example.com:11434", "size": 300},
        {"name": "glm-4.6:cloud", "size": 400},
        {"name": "qwen3:1.7b", "size": 1_359_293_444_u64}]});
    let ollama = serve(vec![
        Reply::status(200, list.clone()),
        Reply::status(200, tags.clone()),
        Reply::status(200, tags.clone()),
        Reply::status(200, tags.clone()),
        Reply::status(200, tags.clone()),
        Reply::status(500, json!({"error": "the list is broken"})),
        Reply::status(200, tags.clone()),
        Reply::events(&compatible_says("Done.")),
        Reply::events(&compatible_says("Again.")),
    ]);
    let mut settings = Settings {
        helper: Some(Choice::Ollama),
        ..Settings::default()
    };
    settings.ollama.address = ollama.address.clone();
    settings.ollama.model = "gpt-oss:120b-cloud".into();
    let elsewhere = |model: &str, place: &str| {
        format!(
            "Ollama passes what “{model}” is asked on to {place}, and Assist uses Ollama \
             only for what runs on this computer. Choose another model in Assist's settings."
        )
    };

    let refused = check(&settings).expect_err("a model answered elsewhere");
    assert_eq!(refused.kind, FailureKind::NotReady);
    assert_eq!(
        refused.sentence,
        elsewhere("gpt-oss:120b-cloud", "ollama.com")
    );
    assert_eq!(ollama.heard().path, "/v1/models");
    assert_eq!(ollama.heard().path, "/api/tags");

    let conversation = asking("Improve the wording.");
    let tools = reading();
    let request = Request {
        system: SYSTEM,
        tools: &tools,
        conversation: &conversation,
        effort: Effort::Usual,
    };
    let refusals = |settings: &Settings| {
        let mut helper = connect(settings);
        let answer = helper.answer(&request, &StopFlag::new(), &mut |_| {});
        answer.ending.expect_err("refused").sentence
    };
    for (model, place) in [
        ("gpt-oss:120b-cloud", "ollama.com"),
        // A model made with a remote host, named without its tag.
        ("helper", "gpu.example.com:11434"),
        // A cloud model by its name, whatever the list says of it.
        ("glm-4.6:cloud", "ollama.com"),
    ] {
        settings.ollama.model = model.into();
        assert_eq!(
            refusals(&settings),
            format!("{} Nothing was sent.", elsewhere(model, place))
        );
        assert_eq!(ollama.heard().path, "/api/tags", "{model}");
    }
    // Where a model runs, unsaid, is not taken to be here.
    settings.ollama.model = "qwen3:1.7b".into();
    assert!(refusals(&settings).starts_with("Ollama"));
    assert_eq!(ollama.heard().path, "/api/tags");

    // A model on this computer: where it runs is asked once, and then the
    // requests go.
    let mut helper = connect(&settings);
    for said in ["Done.", "Again."] {
        let answer = helper.answer(&request, &StopFlag::new(), &mut |_| {});
        assert_eq!(answer.message.text(), said);
    }
    assert_eq!(ollama.heard().path, "/api/tags");
    assert_eq!(ollama.heard().path, "/v1/chat/completions");
    assert_eq!(ollama.heard().path, "/v1/chat/completions");
    assert!(ollama.heard_nothing());
}

/// A service that has no list at the address given, and refuses the key at
/// that address with /v1 after it, has refused the key: it is not kept. One
/// with no list at either has answered, and says so.
#[test]
fn a_key_refused_where_the_service_answers_is_not_kept() {
    let service = serve(vec![
        Reply::status(404, json!({"error": {"message": "Not Found"}})),
        Reply::status(
            401,
            json!({"error": {"message": "Incorrect API key provided"}}),
        ),
        Reply::status(404, json!({"error": {"message": "Not Found"}})),
        Reply::status(404, json!({"error": {"message": "Not Found"}})),
    ]);
    let host = crate::http::host_of(&service.address);
    let mut settings = Settings {
        helper: Some(Choice::Service),
        ..Settings::default()
    };
    settings.service.address = service.address.clone();
    settings.service.key = "sk-wrong".into();
    settings.service.model = "some-model".into();
    let refused = check(&settings).expect_err("a key refused at /v1");
    assert_eq!(refused.kind, FailureKind::Unauthorized);
    assert_eq!(
        refused.sentence,
        format!(
            "{host} answers at {at}/v1, not at {at}, and did not accept the key there. \
             Add /v1 to the address, and check the key.",
            at = service.address
        )
    );
    assert_eq!(service.heard().path, "/models");
    assert_eq!(service.heard().path, "/v1/models");

    assert_eq!(
        check(&settings),
        Ok(format!(
            "{host} answered, but not with a list of what it can answer with, as most \
             services do. The key is checked when the first request goes; if that \
             fails, check the address."
        ))
    );
    assert_eq!(service.heard().path, "/models");
    assert_eq!(service.heard().path, "/v1/models");
}

/// Where the words go is said by where the helper is: Anthropic's own address,
/// another address Claude is reached through, a service's host — and nowhere,
/// for a helper at an address of this computer's own.
#[test]
fn a_request_is_said_to_go_where_the_helper_is() {
    use crate::provider::destination;
    let with = |choice: Choice, address: &str| {
        let mut settings = Settings {
            helper: Some(choice),
            ..Settings::default()
        };
        settings.ollama.address = address.to_owned();
        settings.service.address = address.to_owned();
        if !address.is_empty() {
            settings.claude.address = address.to_owned();
        }
        destination(&settings)
    };
    assert_eq!(with(Choice::Claude, ""), Some("Anthropic".to_owned()));
    assert_eq!(
        with(Choice::Claude, "https://api.anthropic.com/"),
        Some("Anthropic".to_owned())
    );
    assert_eq!(
        with(Choice::Claude, "https://relay.example.com/anthropic"),
        Some("Claude at relay.example.com".to_owned())
    );
    assert_eq!(
        with(Choice::Claude, "http://127.0.0.1:8080"),
        Some("Claude at 127.0.0.1:8080".to_owned()),
        "a relay on this computer passes the words on"
    );
    assert_eq!(with(Choice::Service, "http://127.0.0.1:8080/v1"), None);
    assert_eq!(with(Choice::Service, "http://localhost:8080/v1"), None);
    assert_eq!(
        with(Choice::Service, "https://api.example.com/v1"),
        Some("api.example.com".to_owned())
    );
    assert_eq!(with(Choice::Ollama, "http://127.0.0.1:11434"), None);
    assert_eq!(
        with(Choice::Ollama, "http://192.168.1.4:11434"),
        Some("Ollama at 192.168.1.4:11434".to_owned())
    );
    assert_eq!(with(Choice::Local, ""), None);
    assert_eq!(destination(&Settings::default()), None, "no helper");
}
