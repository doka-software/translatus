//! LLM abstraction. Enum dispatch (not `dyn`) keeps async-in-trait simple while
//! letting CLI/Desktop pick a backend at runtime. Every provider shares retry,
//! backoff and token/cost reporting above this layer.

pub mod mock;
pub mod openai;

use crate::config::{ProviderKind, TranslateConfig};
use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionResponse {
    pub text: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// Runtime-selected backend.
pub enum Provider {
    Mock(mock::MockProvider),
    OpenAi(openai::OpenAiProvider),
}

/// Why this provider cannot be used yet, if it cannot.
///
/// Split out of `from_config` so a caller can refuse before parsing a book, and
/// so the message has exactly one home — a user-facing string duplicated across
/// crates is one that drifts.
pub fn unsupported_reason(provider: ProviderKind) -> Option<String> {
    match provider {
        // Anthropic's API is `POST /v1/messages` with an `x-api-key` header, not
        // `/chat/completions` with a bearer token. Routing it through the OpenAI
        // client sends the operator's key to the right host in the wrong shape:
        // it fails closed with a 404, but only after the docs promised it would
        // work. Refuse instead of pretending, until a real client lands.
        ProviderKind::Anthropic => Some(
            "--provider anthropic is not implemented yet. Translatus would speak the OpenAI \
             wire format (POST /chat/completions, bearer token) to an API that expects \
             POST /v1/messages with an x-api-key header, so every request would fail.\n\
             \n\
             Instead use one of:\n\
             \x20 • --provider openai --base-url <an OpenAI-compatible gateway>\n\
             \x20 • the subscription sidecar, which reaches Claude through the official SDK"
                .into(),
        ),
        _ => None,
    }
}

impl Provider {
    /// Build from config + an optional API key (env/flag/keychain resolved by caller).
    ///
    /// This is the one place every caller (CLI, GUI host, MCP worker) funnels
    /// through, so it is where a caller-supplied `base_url` gets validated —
    /// `OperatorConfigured`, because reaching here means a human typed it into a
    /// flag or a settings field. A credential follows the URL, so a remote
    /// endpoint must be TLS.
    pub fn from_config(
        cfg: &TranslateConfig,
        api_key: Option<String>,
        base_url: Option<String>,
    ) -> Result<Provider> {
        if let Some(url) = base_url.as_deref() {
            crate::validate_base_url(url, crate::EndpointTrust::OperatorConfigured)
                .map_err(CoreError::Other)?;
        }
        match cfg.provider {
            ProviderKind::Mock => Ok(Provider::Mock(mock::MockProvider)),
            ProviderKind::OpenAi => Ok(Provider::OpenAi(openai::OpenAiProvider::new(
                cfg.model.clone(),
                api_key,
                base_url.unwrap_or_else(|| "https://api.openai.com/v1".into()),
            )?)),
            // Ollama speaks the OpenAI-compatible endpoint most local servers
            // expose, so it genuinely is the same client.
            ProviderKind::Ollama => Ok(Provider::OpenAi(openai::OpenAiProvider::new(
                cfg.model.clone(),
                None,
                base_url.unwrap_or_else(|| "http://localhost:11434/v1".into()),
            )?)),
            // See `unsupported_reason` for why this is refused rather than wired up.
            ProviderKind::Anthropic => Err(CoreError::Other(
                unsupported_reason(ProviderKind::Anthropic).unwrap_or_default(),
            )),
        }
    }

    pub async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        match self {
            Provider::Mock(p) => p.complete(req).await,
            Provider::OpenAi(p) => p.complete(req).await,
        }
    }

    /// Retry wrapper with exponential backoff for transient API failures.
    ///
    /// "Busy" is not the same kind of transient as "flaky". A subscription
    /// sidecar serves one paid completion at a time and a book-length one runs
    /// for minutes, so a 1s/2s backoff against a 429 is guaranteed to give up
    /// while the other run is still going — and giving up costs the unit, which
    /// is how starting a second book quietly puts gaps in it. Back off on the
    /// scale of the thing being waited for instead.
    pub async fn complete_retrying(
        &self,
        req: &CompletionRequest,
        max_retries: u32,
    ) -> Result<CompletionResponse> {
        const BUSY_BACKOFF_S: [u64; 6] = [15, 30, 60, 120, 120, 120];
        let mut attempt = 0u32;
        let mut busy_attempt = 0usize;
        loop {
            match self.complete(req).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    if is_busy(&e) {
                        if busy_attempt >= BUSY_BACKOFF_S.len() {
                            return Err(e);
                        }
                        let wait = BUSY_BACKOFF_S[busy_attempt];
                        busy_attempt += 1;
                        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                        continue;
                    }
                    if attempt >= max_retries {
                        return Err(e);
                    }
                    let backoff = 1u64 << attempt; // 1,2,4,8…s
                    tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                    attempt += 1;
                }
            }
        }
    }
}

/// "Wait, something else is using the endpoint" — as opposed to a flaky call.
/// Matched on the wire shape rather than a typed error because the message is
/// what every adapter actually carries.
pub(crate) fn is_busy(e: &CoreError) -> bool {
    let m = e.to_string();
    m.contains("429") || m.contains("llm_subscription_kit_busy") || m.contains("in-flight")
}

#[cfg(test)]
mod busy_tests {
    use super::*;

    /// The sidecar's "one paid completion at a time" 429 must be told apart
    /// from an ordinary transient. Getting this wrong is silent: the unit fails,
    /// the book gets a gap, and the run still reports as finished.
    #[test]
    fn a_busy_endpoint_is_not_treated_as_a_flaky_call() {
        let busy = CoreError::Other(
            "provider error: 429 Too Many Requests from http://127.0.0.1:8766/v1/chat/completions: \
             {\"error\":{\"message\":\"too many in-flight completions (limit 1)\",\
             \"type\":\"llm_subscription_kit_busy\"}}"
                .into(),
        );
        assert!(is_busy(&busy));

        for other in [
            CoreError::Other("provider error: 500 Internal Server Error".into()),
            CoreError::Other("provider error: 401 Unauthorized".into()),
            CoreError::Other("connection reset".into()),
        ] {
            assert!(!is_busy(&other), "must not read as busy: {other}");
        }
    }
}

#[cfg(test)]
mod provider_gate_tests {
    use super::*;
    use crate::config::TranslateConfig;

    fn cfg(provider: ProviderKind) -> TranslateConfig {
        TranslateConfig {
            provider,
            model: "m".into(),
            ..TranslateConfig::new("English")
        }
    }

    /// M4: the TLS rule existed and was tested, but nothing called it — the CLI
    /// handed `--base-url` straight to the client. Every caller funnels through
    /// `from_config`, so the rule has to bite here or it bites nowhere.
    #[test]
    fn remote_base_url_must_be_tls_for_every_caller() {
        let err = Provider::from_config(
            &cfg(ProviderKind::OpenAi),
            Some("k".into()),
            Some("http://api.example.com/v1".into()),
        )
        .err()
        .expect("plain http to a remote host must be refused")
        .to_string();
        assert!(err.contains("https"), "wrong error: {err}");

        // Loopback over plain http stays allowed: that is the sidecar and Ollama.
        assert!(Provider::from_config(
            &cfg(ProviderKind::OpenAi),
            Some("k".into()),
            Some("http://127.0.0.1:8765/v1".into())
        )
        .is_ok());
        // Remote over TLS is the documented custom-endpoint feature.
        assert!(Provider::from_config(
            &cfg(ProviderKind::OpenAi),
            Some("k".into()),
            Some("https://api.example.com/v1".into())
        )
        .is_ok());
        // The userinfo trick is refused here too.
        assert!(Provider::from_config(
            &cfg(ProviderKind::OpenAi),
            Some("k".into()),
            Some("https://127.0.0.1@evil.com/v1".into())
        )
        .is_err());
    }

    /// M2: `--provider anthropic` used to build an OpenAI client aimed at
    /// api.anthropic.com, so it sent `POST /chat/completions` with a bearer
    /// token to an API that speaks neither. It must refuse, not pretend.
    #[test]
    fn anthropic_refuses_rather_than_speaking_the_wrong_protocol() {
        let reason = unsupported_reason(ProviderKind::Anthropic).expect("must be unsupported");
        assert!(reason.contains("not implemented"), "unhelpful: {reason}");
        // The message has to point somewhere that works, or it is just a wall.
        assert!(
            reason.contains("--provider openai") && reason.contains("subscription"),
            "must offer a way forward: {reason}"
        );

        let err = Provider::from_config(&cfg(ProviderKind::Anthropic), Some("k".into()), None)
            .err()
            .expect("anthropic must be refused")
            .to_string();
        assert!(err.contains("not implemented"), "wrong error: {err}");

        // The providers that do work must not be caught by the same gate.
        for ok in [
            ProviderKind::Mock,
            ProviderKind::OpenAi,
            ProviderKind::Ollama,
        ] {
            assert!(unsupported_reason(ok).is_none(), "{ok:?} must stay usable");
        }
    }
}
