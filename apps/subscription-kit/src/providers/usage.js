// Token-usage normalisation — the one place that knows what each provider's
// `usage` object actually counts.
//
// Why this file exists: Anthropic's `usage.input_tokens` is NOT "how many input
// tokens this request read". With prompt caching active — and the Claude Agent
// SDK caches on every call — the prefix is billed under
// `cache_creation_input_tokens` / `cache_read_input_tokens`, and `input_tokens`
// holds only the uncached remainder after the last cache breakpoint. Reading it
// alone reports a 16,183-token request as 10 tokens.
//
// The buckets are NOT interchangeable: they carry different prices, so they stay
// separate all the way out to the wire and the caller applies the weights.
// Verified 2026-09-01 against the official pages —
//   https://platform.claude.com/docs/en/about-claude/pricing#prompt-caching
//   https://platform.claude.com/docs/en/build-with-claude/prompt-caching
//     cache read (hit or refresh) ... 0.1x  the base input price
//     5-minute cache write ........... 1.25x the base input price
//     1-hour cache write ............. 2x    the base input price
// Summing the three into one number and pricing it at the base rate converts a
// large under-report into a large over-report. Nothing here adds them up.
//
// Normalised shape (both providers are mapped onto it):
//   input_tokens ..................... billed at the FULL base input rate
//   cache_read_input_tokens .......... served from cache (0.1x)
//   cache_creation_5m_input_tokens ... written with a 5-minute TTL (1.25x)
//   cache_creation_1h_input_tokens ... written with a 1-hour TTL (2x)
//   cache_creation_input_tokens ...... the two write buckets summed
//   output_tokens

function count(v) {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? Math.trunc(n) : 0;
}

export function emptyUsage() {
  return {
    input_tokens: 0,
    output_tokens: 0,
    cache_read_input_tokens: 0,
    cache_creation_input_tokens: 0,
    cache_creation_5m_input_tokens: 0,
    cache_creation_1h_input_tokens: 0,
  };
}

/// Anthropic (`@anthropic-ai/claude-agent-sdk` result message `usage`).
///
/// `cache_creation` carries the per-TTL split; `cache_creation_input_tokens` is
/// their total. When the split is missing or short of the total, the unexplained
/// remainder is attributed to the 1-hour bucket: that is both the more expensive
/// write (so the estimate stays a ceiling rather than under-quoting the user)
/// and what the Agent SDK is observed to use.
export function normalizeClaudeUsage(u) {
  if (!u || typeof u !== "object") return emptyUsage();
  const created = count(u.cache_creation_input_tokens);
  // When the split is exact (split5m + split1h === created) this reproduces it
  // unchanged; when it is missing or short, the remainder lands in the 1-hour
  // bucket rather than being invented as cheap 5-minute writes.
  const c5m = Math.min(count(u.cache_creation?.ephemeral_5m_input_tokens), created);
  return {
    input_tokens: count(u.input_tokens),
    output_tokens: count(u.output_tokens),
    cache_read_input_tokens: count(u.cache_read_input_tokens),
    cache_creation_input_tokens: created,
    cache_creation_5m_input_tokens: c5m,
    cache_creation_1h_input_tokens: created - c5m,
  };
}

/// OpenAI Codex (`@openai/codex-sdk` turn.completed `usage`).
///
/// Opposite convention to Anthropic's: Codex's `input_tokens` is the FULL input
/// count and `cached_input_tokens` is a subset of it, so the full-rate remainder
/// has to be derived by subtraction. Codex has no cache-write charge.
export function normalizeCodexUsage(u) {
  if (!u || typeof u !== "object") return emptyUsage();
  const total = count(u.input_tokens);
  const cached = Math.min(count(u.cached_input_tokens), total);
  return {
    input_tokens: total - cached,
    output_tokens: count(u.output_tokens),
    cache_read_input_tokens: cached,
    cache_creation_input_tokens: 0,
    cache_creation_5m_input_tokens: 0,
    cache_creation_1h_input_tokens: 0,
  };
}

/// Every input token the request actually read, at any price.
export function totalInputTokens(usage) {
  return (
    count(usage?.input_tokens) +
    count(usage?.cache_read_input_tokens) +
    count(usage?.cache_creation_input_tokens)
  );
}

/// OpenAI-shaped `usage` for the HTTP contract.
///
/// `prompt_tokens` is the honest total, matching OpenAI's own semantics (their
/// `prompt_tokens` includes cached tokens and `prompt_tokens_details.cached_tokens`
/// names the discounted subset). The cache-write fields are extensions — OpenAI
/// has no equivalent concept, Anthropic charges for one, and the TTL decides
/// whether the multiplier is 1.25x or 2x. A client that understands only the
/// OpenAI contract still gets a correct token count; a client that wants the
/// cost right has everything it needs to weight the buckets.
export function openAiUsage(usage) {
  const promptTokens = totalInputTokens(usage);
  const completionTokens = count(usage?.output_tokens);
  return {
    prompt_tokens: promptTokens,
    completion_tokens: completionTokens,
    total_tokens: promptTokens + completionTokens,
    prompt_tokens_details: {
      cached_tokens: count(usage?.cache_read_input_tokens),
      cache_creation_tokens: count(usage?.cache_creation_input_tokens),
      cache_creation_5m_tokens: count(usage?.cache_creation_5m_input_tokens),
      cache_creation_1h_tokens: count(usage?.cache_creation_1h_input_tokens),
    },
  };
}
