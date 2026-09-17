//! Requests to a helper over HTTP, and what their failures say.
//!
//! **Blocking, on the helper's own thread.** A request is one call that returns
//! when the answer's headers have come, and the answer is read as it arrives;
//! the window goes on painting on its own thread meanwhile, so there is no
//! async runtime to carry.
//!
//! **The certificates are the operating system's.** HTTPS is rustls, and the
//! roots it trusts are whatever the platform trusts, through
//! `rustls-platform-verifier` — the certificate store a company installs its own
//! root into, the same store the browser uses. ureq's default would compile
//! Mozilla's store into the binary, which is bundling, and which would refuse
//! exactly the corporate proxy the browser accepts.
//!
//! A helper on this computer is reached directly, whatever proxy the
//! environment names: a proxy cannot reach a server that listens only on the
//! loopback address.

use std::io::{BufReader, Read};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::event::{Failure, FailureKind};

/// How long a helper may take to accept a connection. A server on this
/// computer that is not running refuses at once; one that is listening
/// answers well inside this.
const CONNECT: Duration = Duration::from_secs(10);

/// How long a helper may take before its answer begins. Ollama loads a model
/// before it says anything, which on a slow disk is most of a minute.
const FIRST_WORD: Duration = Duration::from_secs(300);

/// How long an answer may take in all. Longer than any editor's request should
/// run; past it the answer is cut off with a sentence rather than waited on.
const WHOLE_ANSWER: Duration = Duration::from_secs(600);

/// The most of an error's body that is read: enough for any service's
/// message, and not a way to fill memory.
const ERROR_BODY: u64 = 64 * 1024;

pub(crate) struct Http {
    agent: ureq::Agent,
}

/// A response whose headers have arrived.
pub(crate) struct Response {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: BufReader<ureq::BodyReader<'static>>,
}

impl Http {
    /// A client for the helper at `address`.
    pub fn new(address: &str) -> Http {
        Http::build(address, CONNECT, FIRST_WORD, WHOLE_ANSWER)
    }

    /// A client for a quick question — whether a server is there at all —
    /// that gives up in a moment rather than hold up the card that asked.
    pub fn quick(address: &str) -> Http {
        let moment = Duration::from_secs(2);
        Http::build(address, Duration::from_millis(500), moment, moment)
    }

    fn build(address: &str, connect: Duration, first_word: Duration, whole: Duration) -> Http {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .root_certs(RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let mut config = ureq::Agent::config_builder()
            .tls_config(tls)
            // A 429 or a 529 is an answer to read, not an error to raise: its
            // body says why and its headers say how long to wait.
            .http_status_as_error(false)
            // A redirect is not followed. ureq drops only `Authorization` when
            // it follows one, so a key in `x-api-key` would go to wherever the
            // redirect pointed; a helper's address is where its key may go.
            .max_redirects(0)
            .user_agent(concat!("officina-assist/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(connect))
            .timeout_recv_response(Some(first_word))
            .timeout_recv_body(Some(whole));
        if is_loopback(address) {
            config = config.proxy(None);
        }
        Http {
            agent: config.build().new_agent(),
        }
    }

    /// Sends `body` to `url` and waits for the answer's headers.
    pub fn post(
        &self,
        url: &str,
        headers: &[(&str, String)],
        body: &Value,
        helper: &str,
    ) -> Result<Response, Failure> {
        let mut request = self.agent.post(url);
        for (name, value) in headers {
            request = request.header(*name, value.as_str());
        }
        let sent = request
            .header("content-type", "application/json")
            .send(body.to_string());
        respond(sent, url, helper)
    }

    /// Asks `url` a question that sends nothing and costs nothing, and waits
    /// for the answer's headers.
    pub fn get(
        &self,
        url: &str,
        headers: &[(&str, String)],
        helper: &str,
    ) -> Result<Response, Failure> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, value.as_str());
        }
        respond(request.call(), url, helper)
    }
}

fn respond(
    sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    url: &str,
    helper: &str,
) -> Result<Response, Failure> {
    let response = sent.map_err(|error| unreachable(error, url, helper))?;
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    Ok(Response {
        status: response.status().as_u16(),
        retry_after,
        body: BufReader::new(response.into_body().into_reader()),
    })
}

impl Response {
    /// What an error response says, in the service's own words when it gives
    /// them: `{"error": {"message": …}}`, `{"error": "…"}`, or the text itself.
    pub fn error_message(self) -> String {
        let mut text = String::new();
        let _ = self.body.take(ERROR_BODY).read_to_string(&mut text);
        let said = serde_json::from_str::<Value>(&text).ok().and_then(|json| {
            let error = json.get("error")?;
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
                .map(str::to_owned)
        });
        said.unwrap_or_else(|| text.trim().chars().take(300).collect())
    }

    /// The failure an HTTP error status stands for, in words for the
    /// transcript.
    pub fn failure(self, helper: &str) -> Failure {
        let status = self.status;
        let retry_after = self.retry_after;
        let said = self.error_message();
        let because = match said.is_empty() {
            true => String::new(),
            false => format!(" It said: “{}”", said.trim_end_matches('.')),
        };
        match status {
            401 => Failure::new(
                FailureKind::Unauthorized,
                format!("{helper} did not accept the key.{because}"),
            ),
            402 => Failure::new(
                FailureKind::Unauthorized,
                format!(
                    "{helper} says the account needs attention before it will answer.{because}"
                ),
            ),
            403 => Failure::new(
                FailureKind::Unauthorized,
                format!("{helper} does not allow this key to make this request.{because}"),
            ),
            429 => Failure::new(
                FailureKind::RateLimited { retry_after },
                match retry_after {
                    Some(wait) => format!(
                        "{helper} has had too many requests; it asks to wait {} before the next.",
                        seconds(wait)
                    ),
                    None => format!(
                        "{helper} has had too many requests; wait a little before the next."
                    ),
                },
            ),
            300..=399 => Failure::new(
                FailureKind::Rejected { status },
                format!(
                    "{helper} answered from another address (HTTP {status}), which is not \
                     followed, so that nothing is sent where it was not meant to go. \
                     Check the address in Assist's settings."
                ),
            ),
            500 | 502 | 503 | 504 | 529 => Failure::new(
                FailureKind::Busy,
                format!("{helper} is busy or having trouble right now; try again in a moment."),
            ),
            _ => Failure::new(
                FailureKind::Rejected { status },
                format!("{helper} turned the request down (HTTP {status}).{because}"),
            ),
        }
    }
}

fn seconds(wait: Duration) -> String {
    match wait.as_secs() {
        1 => "1 second".to_owned(),
        n => format!("{n} seconds"),
    }
}

/// A request that got no answer at all, and why, naming where it was sent.
fn unreachable(error: ureq::Error, url: &str, helper: &str) -> Failure {
    let host = host_of(url);
    match error {
        ureq::Error::Timeout(ureq::Timeout::RecvResponse) => Failure::new(
            FailureKind::Busy,
            format!("{helper} at {host} did not begin to answer in time."),
        ),
        ureq::Error::Timeout(ureq::Timeout::RecvBody) => Failure::new(
            FailureKind::Dropped,
            format!("{helper} took longer than ten minutes, and the answer was cut off."),
        ),
        ureq::Error::BadUri(_) => Failure::new(
            FailureKind::Unreachable,
            format!("“{url}” is not an address {helper} can be reached at."),
        ),
        other if untrusted(&other).is_some() => Failure::new(
            FailureKind::Unreachable,
            format!(
                "The connection to {helper} at {host} could not be trusted, so nothing was sent ({}).",
                untrusted(&other).unwrap_or_default()
            ),
        ),
        other => Failure::new(
            FailureKind::Unreachable,
            format!("No answer from {helper} at {host} — is it running, and is this computer online? ({other})"),
        ),
    }
}

/// Why TLS refused the connection, when it did: as ureq's own error, or inside
/// the I/O error the encrypted stream returned, which is where a certificate
/// the platform does not trust is reported.
fn untrusted(error: &ureq::Error) -> Option<String> {
    match error {
        ureq::Error::Rustls(refusal) => Some(refusal.to_string()),
        ureq::Error::Io(io) => io
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            .map(ToString::to_string),
        _ => None,
    }
}

/// The host and port of an address, as a person would recognize it.
pub(crate) fn host_of(address: &str) -> String {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // Credentials in an address are nobody's business in a transcript.
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    match host.is_empty() {
        true => address.to_owned(),
        false => host.to_owned(),
    }
}

/// Whether the address is this computer's own, which a request to it never
/// leaves.
pub(crate) fn is_loopback(address: &str) -> bool {
    let host = host_of(address);
    let name = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => host
            .rsplit_once(':')
            .map_or(host.as_str(), |(name, _)| name),
    };
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_named_by_its_host_and_a_helper_here_is_reached_directly() {
        assert_eq!(host_of("https://api.anthropic.com"), "api.anthropic.com");
        assert_eq!(host_of("http://localhost:11434/v1"), "localhost:11434");
        assert_eq!(host_of("https://user:pw@example.com/v1"), "example.com");
        assert!(is_loopback("http://localhost:11434/v1"));
        assert!(is_loopback("http://127.0.0.1:8080"));
        assert!(is_loopback("http://[::1]:8080/v1"));
        assert!(!is_loopback("https://api.anthropic.com"));
        assert!(!is_loopback("http://192.168.1.4:11434"));
    }

    #[test]
    fn a_certificate_that_is_not_trusted_is_said_to_be_so() {
        let url = "https://example.com/v1/messages";
        let expired = rustls::Error::InvalidCertificate(rustls::CertificateError::Expired);
        let wrapped = std::io::Error::new(std::io::ErrorKind::InvalidData, expired.clone());
        for error in [ureq::Error::Rustls(expired), ureq::Error::Io(wrapped)] {
            let failure = unreachable(error, url, "Claude");
            assert_eq!(failure.kind, FailureKind::Unreachable);
            assert!(
                failure.sentence.starts_with(
                    "The connection to Claude at example.com could not be trusted, so nothing was sent ("
                ),
                "{}",
                failure.sentence
            );
            assert!(
                failure.sentence.to_lowercase().contains("expired"),
                "{}",
                failure.sentence
            );
        }
        let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let failure = unreachable(ureq::Error::Io(refused), url, "Claude");
        assert!(
            failure
                .sentence
                .starts_with("No answer from Claude at example.com"),
            "{}",
            failure.sentence
        );
    }
}
