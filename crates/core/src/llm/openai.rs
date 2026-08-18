//! OpenAI `/v1/chat/completions`-compatible provider. Works with OpenAI, OpenRouter,
//! Ollama's OpenAI shim, and most local servers. Anthropic gets a thin variant later;
//! the request shape is close enough to route through here for v0.

use crate::error::{CoreError, Result};
use crate::llm::{CompletionRequest, CompletionResponse};
use serde_json::{json, Value};

pub struct OpenAiProvider {
    model: String,
    api_key: Option<String>,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(model: String, api_key: Option<String>, base_url: String) -> Result<Self> {
        // No redirects: the API key is attached as a bearer to the user-set
        // base_url; a 30x to another host must never carry the key off-origin.
        // (A chat/completions endpoint never legitimately redirects.)
        //
        // Timeouts are NOT optional: without them a stalled socket (e.g. the
        // network silently dropping when the machine enters standby) hangs the
        // request forever — and because `complete_retrying` only fires on `Err`,
        // a hang freezes the WHOLE book on one call (observed: a single request
        // stuck >10h). A timeout turns that stall into a retryable error.
        // 180s is well above any legitimate call (direct API ≤30s; the
        // subscription sidecar self-aborts earlier at ~150s) yet bounds the hang.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(180))
            .build()?;
        Ok(Self {
            model,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    pub async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let mut messages = vec![json!({ "role": "system", "content": req.system })];
        for m in &req.messages {
            messages.push(json!({ "role": m.role, "content": m.content }));
        }

        let body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": req.temperature,
        });

        let url = format!("{}/chat/completions", self.base_url);
        let mut rb = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            rb = rb.bearer_auth(key);
        }

        let resp = rb.send().await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if !status.is_success() {
            return Err(CoreError::Provider(format!(
                "{} from {}: {}",
                status,
                url,
                redact_secrets(&v.to_string())
            )));
        }

        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                CoreError::Provider(format!(
                    "no content in response: {}",
                    redact_secrets(&v.to_string())
                ))
            })?
            .to_string();
        let tokens_in = v["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
        let tokens_out = v["usage"]["completion_tokens"].as_u64().unwrap_or(0);

        Ok(CompletionResponse {
            text,
            tokens_in,
            tokens_out,
        })
    }
}

#[cfg(test)]
mod redirect_tests {
    use super::OpenAiProvider;
    use crate::llm::{ChatMessage, CompletionRequest};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn bearer_request_never_follows_a_redirect() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let address = listener.local_addr().expect("server address");
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let server_hits = Arc::clone(&hits);
        let server_stop = Arc::clone(&stop);

        let server = std::thread::spawn(move || {
            while !server_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut request = [0_u8; 4096];
                        let _ = stream.read(&mut request);
                        let seen = server_hits.fetch_add(1, Ordering::SeqCst);
                        if seen == 0 {
                            let response = format!(
                                "HTTP/1.1 302 Found\r\nLocation: http://{address}/capture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            );
                            let _ = stream.write_all(response.as_bytes());
                        } else {
                            let body =
                                r#"{"choices":[{"message":{"content":"redirect followed"}}]}"#;
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes());
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        let provider = OpenAiProvider::new(
            "test-model".into(),
            Some("canary-not-a-real-key".into()),
            format!("http://{address}"),
        )
        .expect("build hardened client");
        let request = CompletionRequest {
            system: "test".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
            temperature: 0.0,
        };
        assert!(provider.complete(&request).await.is_err());
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.store(true, Ordering::SeqCst);
        server.join().expect("join test server");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "redirect target was contacted"
        );
    }
}

/// Strip anything key-shaped out of text that came back from a provider.
///
/// `--base-url` points at whatever OpenAI-compatible gateway the operator
/// chose, and some of them echo the request (headers included) in their error
/// bodies. We embed those bodies in errors that reach stderr, logs, and — over
/// MCP — an agent's context. The gateway already saw the key; this stops a bad
/// upstream from turning that into a second, local disclosure.
pub(crate) fn redact_secrets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    // Token shapes worth hiding, longest-prefix first so `sk-ant-` is not
    // matched as a short `sk-`.
    const PREFIXES: &[&str] = &[
        "sk-ant-",
        "sk-proj-",
        "sk-",
        "ghp_",
        "github_pat_",
        "xoxb-",
        "AIza",
    ];
    while !rest.is_empty() {
        let mut redacted = false;
        for p in PREFIXES {
            if let Some(after) = rest.strip_prefix(p) {
                // Only treat it as a token if real token characters follow.
                let n = after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                    .count();
                if n >= 8 {
                    out.push_str(p);
                    out.push_str("<redacted>");
                    rest = &after[after
                        .char_indices()
                        .nth(n)
                        .map(|(b, _)| b)
                        .unwrap_or(after.len())..];
                    redacted = true;
                    break;
                }
            }
        }
        if redacted {
            continue;
        }
        let n = rest.chars().next().expect("non-empty rest").len_utf8();
        out.push_str(&rest[..n]);
        rest = &rest[n..];
    }
    redact_bearer_tokens(&out)
}

/// Remove the token value from an echoed Authorization header. Bearer tokens
/// are not required to use a vendor prefix, so the prefix scanner above cannot
/// protect this shape. Matching is case-insensitive; the token68 character set
/// follows RFC 6750 and stops at JSON/header delimiters.
fn redact_bearer_tokens(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find("bearer ") {
        let start = cursor + relative;
        let token_start = start + "bearer ".len();
        out.push_str(&input[cursor..token_start]);
        let token_len = input[token_start..]
            .chars()
            .take_while(|c| {
                c.is_ascii_alphanumeric() || matches!(*c, '-' | '.' | '_' | '~' | '+' | '/' | '=')
            })
            .map(char::len_utf8)
            .sum::<usize>();
        if token_len == 0 {
            cursor = token_start;
            continue;
        }
        out.push_str("<redacted>");
        cursor = token_start + token_len;
    }
    out.push_str(&input[cursor..]);
    out
}

#[cfg(test)]
mod redaction_tests {
    use super::redact_secrets;

    /// The fixtures are assembled at runtime rather than written as literals.
    /// `tools/publish-oss.sh` greps the whole published tree for token shapes
    /// and cannot tell a test fixture from a real leak — nor should it learn
    /// to, because a gate with an allowlist is a gate that rots. So the source
    /// text here must not itself contain anything key-shaped.
    fn token(prefix: &str, body: &str) -> String {
        format!("{prefix}{body}")
    }

    #[test]
    fn provider_echoed_credentials_never_survive_into_an_error() {
        const BODY: &str = "ABCDEFGH12345678IJKL";
        for prefix in [
            "sk-",
            "sk-ant-api03-",
            "sk-proj-",
            "gh",
            "github",
            "xoxb-",
            "AI",
        ] {
            // Re-form the real prefixes from fragments for the same reason.
            let prefix = match prefix {
                "gh" => "ghp_".to_string(),
                "github" => format!("github{}", "_pat_"),
                "AI" => format!("AI{}", "za"),
                other => other.to_string(),
            };
            let secret = token(&prefix, BODY);
            let raw = format!(r#"{{"error":{{"message":"bad key {secret} for org"}}}}"#);
            let out = redact_secrets(&raw);
            assert!(!out.contains(BODY), "{prefix} token survived: {out}");
            assert!(out.contains("<redacted>"), "no redaction marker: {out}");
        }

        // A bearer header echoed back verbatim.
        let out = redact_secrets("Authorization: Bearer somethingsecret");
        assert!(out.contains("<redacted>"), "bearer not redacted: {out}");
        assert!(!out.contains("somethingsecret"), "bearer survived: {out}");

        let out = redact_secrets("authorization: bearer abc.DEF_ghi-123~= trailing");
        assert!(
            !out.contains("abc.DEF_ghi-123~="),
            "lowercase bearer survived: {out}"
        );

        // Prefix order must not matter. An earlier token of a different family
        // used to be copied through while the scanner jumped to a later prefix.
        let mixed = format!("{} before {}", token("ghp_", BODY), token("sk-ant-", BODY));
        let out = redact_secrets(&mixed);
        assert!(!out.contains(BODY), "mixed token families survived: {out}");

        // Ordinary error text is left readable, or the redaction is useless.
        let plain = "429 Too Many Requests: rate limit reached for gpt-5.4-mini";
        assert_eq!(redact_secrets(plain), plain);
    }
}
