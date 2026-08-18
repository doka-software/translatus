//! et-core — the single translation engine shared by the CLI (for agents) and the
//! GUI hosts embedding it. No UI/CLI/IPC concepts leak in here: it parses
//! a book, translates it, and writes it back, reporting progress via callbacks.
//!
//! Pipeline: `format::extract` → `translate::translate_book` → `format::write`,
//! with `job::JobStore` providing the content-addressed cache + checkpoint that
//! makes runs resumable and cheap.

pub mod annotate;
pub mod chunk;
pub mod config;
pub mod document;
pub mod error;
pub mod format;
pub mod job;
pub mod llm;
pub mod memory;
pub mod secrets;
pub mod settings;
pub mod translate;
pub mod validate;

pub use config::{AnnotationConfig, Density, Level, OutputMode, ProviderKind, TranslateConfig};
pub use document::{Book, Chapter, Format, Segment};
pub use error::{CoreError, Result};
pub use settings::Settings;

/// The parts of a level's system prompt, for the Settings "view prompt" panel:
/// locked head (read-only), the editable style (current + default), locked tail.
#[derive(serde::Serialize)]
pub struct PromptParts {
    pub locked_head: String,
    pub default_style: String,
    pub current_style: String,
    pub locked_tail: String,
}

/// Build the prompt parts for the UI. `custom` is the user's saved style for this
/// level (None → default). `lang_display` is shown in the locked head preview.
pub fn prompt_parts(level: Level, lang_display: &str, custom: Option<&str>) -> PromptParts {
    let default_style = translate::prompt::default_style(level).to_string();
    let current_style = custom
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_style.clone());
    PromptParts {
        locked_head: translate::prompt::locked_head(lang_display),
        default_style,
        current_style,
        locked_tail: translate::prompt::locked_tail().to_string(),
    }
}

/// The parts of the ANNOTATION prompt the Settings "眉批風格" card shows:
/// locked hard rules (read-only) + the editable style (current + default).
#[derive(serde::Serialize)]
pub struct NotePromptParts {
    pub hard_rules: String,
    pub default_style: String,
    pub current_style: String,
}

/// Build the annotation prompt parts for the UI. `custom` is the user's saved
/// note style (None → engine default).
pub fn note_prompt_parts(custom: Option<&str>) -> NotePromptParts {
    let default_style = annotate::prompt::DEFAULT_NOTE_STYLE.to_string();
    let current_style = custom
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_style.clone());
    NotePromptParts {
        hard_rules: annotate::prompt::hard_rules().to_string(),
        default_style,
        current_style,
    }
}

/// Very rough USD cost estimate from token counts. Unknown models report 0.0
/// (the UI/CLI shows token counts regardless). Prices are (input, output) per 1M
/// tokens at the providers' STANDARD tier.
///
/// ⚠️ Point-in-time prices — providers change them. Re-verify against the
/// official pages before trusting a figure; update here when they drift:
///   OpenAI:    https://developers.openai.com/api/docs/pricing
///   Anthropic: https://www.anthropic.com/pricing  (Claude API)
/// All arms below verified 2026-07-21 against those pages; gpt-5.4-mini
/// re-verified 2026-08-02（$0.75/$4.50，同時把 estimate 預設 model 從
/// gpt-4o-mini 換成 gpt-5.4-mini：gpt-4o 系不在實際提供的模型清單內，
/// 其價格 arm 保留只作歷史輸入的容錯）。
///
/// Introductory and promotional rates are deliberately NOT encoded. Claude
/// Sonnet 5 is $2/$10 through 2026-08-31 and $3/$15 from 2026-09-01; we quote
/// the standard $3/$15. A date-dependent price is a time bomb in a pure
/// function, and erring high means the estimate is a ceiling rather than a
/// figure the user can be under-quoted against.
///
/// Match order is specific→general: `gpt-5.4-mini`/`-nano` must precede the bare
/// `gpt-5.4`, and `gpt-4o-mini` precede `gpt-4o`, or the broader arm wins first.
/// The Claude arms are mutually exclusive by family name, so their order is
/// free — but keep them grouped so a new tier is obvious to add.
pub fn estimate_cost_usd(model: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    let (pin, pout): (f64, f64) = match model {
        m if m.contains("gpt-5.4-nano") => (0.20, 1.25),
        m if m.contains("gpt-5.4-mini") => (0.75, 4.50),
        m if m.contains("gpt-5.5") => (5.0, 30.0),
        m if m.contains("gpt-5.4") => (2.50, 15.0),
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.50, 10.0),
        m if m.contains("claude-haiku") => (1.0, 5.0),
        m if m.contains("claude-sonnet") => (3.0, 15.0),
        m if m.contains("claude-opus") => (5.0, 25.0),
        // Fable and Mythos are the same tier and price; Mythos is the
        // invitation-only variant.
        m if m.contains("claude-fable") || m.contains("claude-mythos") => (10.0, 50.0),
        _ => (0.0, 0.0),
    };
    (tokens_in as f64 * pin + tokens_out as f64 * pout) / 1_000_000.0
}

#[cfg(test)]
mod cost_tests {
    use super::estimate_cost_usd;

    #[test]
    fn gpt5_mini_priced_and_not_shadowed_by_bare_5_4() {
        // 1M in + 1M out at gpt-5.4-mini standard (0.75 + 4.50).
        let mini = estimate_cost_usd("gpt-5.4-mini", 1_000_000, 1_000_000);
        assert!(
            (mini - 5.25).abs() < 1e-9,
            "gpt-5.4-mini must use 0.75/4.50, got {mini}"
        );
        // The bare gpt-5.4 arm is pricier (2.50/15) — proves order didn't shadow.
        let full = estimate_cost_usd("gpt-5.4", 1_000_000, 1_000_000);
        assert!(
            (full - 17.5).abs() < 1e-9,
            "gpt-5.4 must use 2.50/15, got {full}"
        );
        assert!(mini < full, "mini must be cheaper than the bare model");
        // gpt-4o-mini must not be shadowed by gpt-4o either.
        let m4 = estimate_cost_usd("gpt-4o-mini", 1_000_000, 1_000_000);
        assert!(
            (m4 - 0.75).abs() < 1e-9,
            "gpt-4o-mini must use 0.15/0.60, got {m4}"
        );
        // Unknown model → 0.0 (token counts still shown by callers).
        assert_eq!(estimate_cost_usd("some-unknown", 9_999, 9_999), 0.0);
    }

    /// Every model the CLI and GUI hosts offer must be priced, and the
    /// tiers must stay correctly ordered.
    ///
    /// The zero assertions are the point of this test: an unpriced model does
    /// not fail loudly, it silently estimates a whole book at $0.00, which is
    /// the one wrong number a cost preview must never show.
    /// Every model any UI offers, not just the Claude ones. `gpt-5.4-nano` was
    /// unprotected: deleting its arm left the suite green while the estimate
    /// silently fell through to the bare `gpt-5.4` arm at 12x the real price.
    /// The order of these arms is load-bearing, so it needs a test that says so.
    #[test]
    fn every_offered_openai_model_is_priced_and_not_shadowed() {
        const M: u64 = 1_000_000;
        let price = |m: &str| estimate_cost_usd(m, M, M);
        for (model, expected) in [
            ("gpt-5.4-nano", 0.20 + 1.25),
            ("gpt-5.4-mini", 0.75 + 4.50),
            ("gpt-5.4", 2.50 + 15.0),
        ] {
            let got = price(model);
            assert!(
                (got - expected).abs() < 1e-9,
                "{model} must cost {expected}, got {got}"
            );
            assert!(
                got > 0.0,
                "{model} is offered but unpriced — would read $0.00"
            );
        }
        // Specific-before-general: each narrower id must stay cheaper than the
        // bare family id it is a prefix of, which is exactly what breaks if the
        // match arms are ever reordered.
        assert!(price("gpt-5.4-nano") < price("gpt-5.4-mini"));
        assert!(price("gpt-5.4-mini") < price("gpt-5.4"));
    }

    #[test]
    fn every_offered_claude_model_is_priced_and_tiers_are_ordered() {
        const M: u64 = 1_000_000;
        let price = |m: &str| estimate_cost_usd(m, M, M);

        // Exact rates, per 1M in + 1M out.
        for (model, expected) in [
            ("claude-haiku-4-5", 6.0),   // 1 + 5
            ("claude-sonnet-5", 18.0),   // 3 + 15 (standard, not the intro rate)
            ("claude-sonnet-4-6", 18.0), // 3 + 15
            ("claude-opus-4-8", 30.0),   // 5 + 25
            ("claude-opus-4-7", 30.0),   // 5 + 25
            ("claude-fable-5", 60.0),    // 10 + 50
            ("claude-mythos-5", 60.0),   // 10 + 50
        ] {
            let got = price(model);
            assert!(
                (got - expected).abs() < 1e-9,
                "{model} must cost {expected}, got {got}"
            );
            assert!(
                got > 0.0,
                "{model} is offered but unpriced — would read $0.00"
            );
        }

        // The ladder must stay monotonic; a swapped arm is otherwise invisible.
        assert!(price("claude-haiku-4-5") < price("claude-sonnet-5"));
        assert!(price("claude-sonnet-5") < price("claude-opus-4-8"));
        assert!(price("claude-opus-4-8") < price("claude-fable-5"));

        // Family arms must not shadow one another.
        assert_ne!(price("claude-opus-4-8"), price("claude-sonnet-5"));
        assert_ne!(price("claude-fable-5"), price("claude-opus-4-8"));
    }
}

/// Where a caller-supplied `base_url` is allowed to point.
///
/// One rule, two strictnesses, because the two callers differ in who chooses
/// the value — not in how much we trust the network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndpointTrust {
    /// The value came from something steerable — an MCP tool call, a script,
    /// anything an agent can be talked into. Loopback only: the legitimate
    /// overrides (a local Ollama, the subscription sidecar) all live there, and
    /// a remote endpoint would receive the operator's key as a bearer token.
    CallerSupplied,
    /// A human typed it into a settings field. Custom OpenAI-compatible
    /// endpoints are a documented feature, so remote is allowed — but only
    /// over TLS, because the whole point of the field is that a credential
    /// follows.
    OperatorConfigured,
}

/// Validate a `base_url` before any credential is sent to it.
///
/// Rejects, in both modes: non-http(s) schemes, and any URL carrying userinfo
/// — `http://127.0.0.1@evil.com/` reads as loopback to a human and resolves to
/// `evil.com` to a client, which is exactly the shape that turns a host check
/// into decoration.
pub fn validate_base_url(raw: &str, trust: EndpointTrust) -> std::result::Result<(), String> {
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| format!("base_url must start with http:// or https://; got `{raw}`"))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("base_url must be http or https; got `{scheme}`"));
    }

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(
            "base_url must not contain credentials before the host (`user@host`) — \
             the part before `@` is ignored by the client and hides the real destination"
                .into(),
        );
    }
    let host = if authority.starts_with('[') {
        authority
            .split_once(']')
            .map(|(h, _)| h.trim_start_matches('['))
            .unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err(format!("base_url has no host: `{raw}`"));
    }
    let loopback = host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1";

    match trust {
        EndpointTrust::CallerSupplied if !loopback => Err(format!(
            "base_url must point at loopback (localhost, 127.0.0.1 or ::1); got `{host}`. \
             A remote endpoint here would receive this machine's API key as a bearer token."
        )),
        EndpointTrust::OperatorConfigured if !loopback && scheme != "https" => Err(format!(
            "a remote base_url must use https; `{raw}` would send your API key in clear text."
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod base_url_tests {
    use super::{validate_base_url, EndpointTrust::*};

    #[test]
    fn caller_supplied_is_loopback_only_and_resists_the_usual_bypasses() {
        for ok in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:8765/v1",
            "http://[::1]:8080/v1",
            "https://LOCALHOST/v1",
        ] {
            assert!(validate_base_url(ok, CallerSupplied).is_ok(), "accept {ok}");
        }
        for bad in [
            "http://evil.com/v1",
            // A non-http scheme with a loopback authority: the only shape that
            // proves the scheme allowlist itself is doing work. Without this,
            // deleting that check leaves core's own suite green.
            "gopher://127.0.0.1/",
            "ftp://localhost/v1",
            "javascript://localhost/",
            // userinfo — the host is evil.com, not 127.0.0.1
            "http://127.0.0.1@evil.com/v1",
            // loopback only in the path or query
            "http://evil.com/127.0.0.1",
            "http://evil.com/?h=localhost",
            // merely prefixed with the literal
            "http://127.0.0.1.evil.com/v1",
            "http://localhost.evil.com/v1",
            "file:///etc/passwd",
            "//127.0.0.1/v1",
            "",
        ] {
            assert!(
                validate_base_url(bad, CallerSupplied).is_err(),
                "reject {bad}"
            );
        }
    }

    #[test]
    fn operator_configured_allows_remote_but_only_over_tls() {
        assert!(validate_base_url("https://api.example.com/v1", OperatorConfigured).is_ok());
        // Plain http to a remote host would put the key on the wire in clear.
        assert!(validate_base_url("http://api.example.com/v1", OperatorConfigured).is_err());
        // Local development over http stays possible.
        assert!(validate_base_url("http://127.0.0.1:11434/v1", OperatorConfigured).is_ok());
        // The userinfo trick is refused in both modes.
        assert!(validate_base_url("https://x@evil.com/v1", OperatorConfigured).is_err());
    }
}
