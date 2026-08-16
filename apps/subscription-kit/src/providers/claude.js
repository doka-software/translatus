// Claude provider — drives the official @anthropic-ai/claude-agent-sdk.
//
// Auth model (Pencil-style): in subscription mode we pass an env WITHOUT any
// ANTHROPIC_API_KEY, so the SDK-bundled claude binary falls back to the user's
// local OAuth subscription credential (macOS Keychain "Claude Code-credentials"
// or ~/.claude/.credentials.json). In api-key mode the key is injected into the
// child env for this request only — never into our own process env.
//
// Policy note (last verified 2026-06): Anthropic policy restricts third-party
// products from routing Pro/Max subscription credentials, and since 2026-06-15
// Agent SDK use is metered against a small per-plan credit at API rates. This
// mode may stop working at any time; the stable paths are an API key or
// Ollama. Surfaced to users via `policyNote` in registry.js.

import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

export const CLAUDE_MODELS = [
  "claude-fable-5",
  "claude-opus-4-8",
  "claude-opus-4-7",
  "claude-opus-4-6",
  "claude-opus-4-5",
  "claude-sonnet-4-6",
  "claude-haiku-4-5",
];

// Same shape Pencil uses for its live probe (AGENT_PROBE_SETTINGS).
export const CLAUDE_PROBE = {
  model: "claude-haiku-4-5",
  prompt: "What is 2+2? Reply with just the number.",
  timeoutMs: 45_000,
};

export function claudeChildEnv(apiKey, source = process.env) {
  const env = {};
  // USER/LOGNAME are load-bearing: without USER the macOS keychain lookup for
  // the "Claude Code-credentials" item fails and every subscription-mode call
  // dies with "Not logged in · Please run /login" even though the user is.
  for (const key of ["HOME", "PATH", "USER", "LOGNAME", "TMPDIR", "TEMP", "TMP", "LANG", "LC_ALL", "NO_COLOR", "CLAUDE_CONFIG_DIR"]) {
    if (typeof source[key] === "string") env[key] = source[key];
  }
  if (apiKey) env.ANTHROPIC_API_KEY = apiKey;
  return env;
}

export function claudeQueryOptions({ model, system, apiKey, abortController, cwd }) {
  return {
    model,
    maxTurns: 1,
    cwd,
    persistSession: false,
    // `allowedTools` controls auto-approval, not availability. `tools: []` is
    // the SDK's actual no-built-ins boundary; the other fields are defence in
    // depth against future default changes.
    tools: [],
    skills: [],
    allowedTools: [],
    disallowedTools: ["Read", "Bash", "Edit", "Write", "Glob", "Grep", "WebFetch", "WebSearch", "Agent", "Skill"],
    mcpServers: {},
    settingSources: [],
    systemPrompt: system || "You are a precise assistant.",
    permissionMode: "dontAsk",
    env: claudeChildEnv(apiKey),
    ...(abortController ? { abortController } : {}),
  };
}

export async function runClaude({ model, system, prompt, apiKey, abortController }) {
  const { query } = await import("@anthropic-ai/claude-agent-sdk");
  const workDir = await fs.mkdtemp(path.join(os.tmpdir(), "translatus-claude-"));
  let text = "";
  let resultText = "";
  let usage = { input_tokens: 0, output_tokens: 0 };
  let subtype, isError;
  let reportedModel = null;
  try {
    const q = query({
      prompt,
      options: claudeQueryOptions({ model, system, apiKey, abortController, cwd: workDir }),
    });
    for await (const m of q) {
      if (m.type === "assistant" && m.message?.content) {
        for (const c of m.message.content) if (c.type === "text") text += c.text;
      }
      if (m.type === "result") {
        subtype = m.subtype;
        isError = m.is_error;
        if (typeof m.result === "string") resultText = m.result;
        if (m.usage) usage = { input_tokens: m.usage.input_tokens ?? 0, output_tokens: m.usage.output_tokens ?? 0 };
        // The CLI reports usage per REAL model id — this is the routing proof,
        // not an echo of what we asked for.
        if (m.modelUsage && typeof m.modelUsage === "object") {
          reportedModel = Object.keys(m.modelUsage).join(",");
        }
      }
    }
  } finally {
    await fs.rm(workDir, { recursive: true, force: true }).catch(() => {});
  }
  if (isError || (subtype && subtype !== "success")) {
    throw new Error(`claude result subtype=${subtype} is_error=${isError}`);
  }
  const out = text.trim() || resultText.trim();
  if (!out) throw new Error("claude returned empty text");
  return { text: out, usage, reportedModel };
}
