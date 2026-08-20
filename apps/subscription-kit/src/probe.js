// Live connection probe — Pencil's pattern: ask the provider's cheapest model a
// trivial question with everything locked down (1 turn, no tools). Success is
// the only proof that install + login + network + billing all line up.
//
// Improvements over Pencil:
//   - error classification (the amber light tells you WHY)
//   - in-flight dedup per provider (re-click never stacks requests)
//   - elapsed time in the result (UI can show a Keychain hint past ~10s)

import { runClaude } from "./providers/claude.js";
import { runCodex } from "./providers/codex.js";
import { getProvider } from "./providers/registry.js";
import { diagnose } from "./auth/diagnose.js";

const inFlight = new Map(); // provider -> { promise, abortController }

function classify(message) {
  const m = (message || "").toLowerCase();
  if (/abort|timeout|timed out/.test(m)) return "timeout";
  if (/login|logged|auth|credential|unauthorized|401|403/.test(m)) return "auth";
  if (/enotfound|econnrefused|network|fetch failed|socket/.test(m)) return "network";
  if (/rate|429|overloaded|529/.test(m)) return "rate-limit";
  return "unknown";
}

export async function probe(providerId, { apiKey } = {}) {
  const spec = getProvider(providerId);
  if (!spec) {
    const e = new Error(`unknown provider '${providerId}'`);
    e.code = 400;
    throw e;
  }

  const existing = inFlight.get(providerId);
  if (existing) {
    existing.abortController.abort();
    inFlight.delete(providerId);
  }

  const abortController = new AbortController();
  const t0 = Date.now();
  const timer = setTimeout(() => abortController.abort(), spec.probe.timeoutMs);

  const promise = (async () => {
    try {
      const run = providerId === "claude" ? runClaude : runCodex;
      const { text } = await run({
        model: spec.probe.model,
        system: "Reply briefly.",
        prompt: spec.probe.prompt,
        apiKey,
        abortController,
      });
      return {
        provider: providerId,
        status: "connected",
        elapsedMs: Date.now() - t0,
        sample: text.slice(0, 20),
        mode: apiKey ? "api-key" : "subscription",
      };
    } catch (e) {
      const reason = classify(e.message);
      // A failed probe is the moment the user needs diagnosis — attach it.
      const layers = await diagnose(providerId).catch(() => null);
      return {
        provider: providerId,
        status: "not-connected",
        elapsedMs: Date.now() - t0,
        mode: apiKey ? "api-key" : "subscription",
        error: { reason, message: e.message },
        diagnosis: layers,
        hint:
          reason === "timeout" && process.platform === "darwin"
            ? "逾時：若有 macOS 鑰匙圈授權視窗跳出，請允許後重試。"
            : layers?.credentials?.hint || null,
      };
    } finally {
      clearTimeout(timer);
      inFlight.delete(providerId);
    }
  })();

  inFlight.set(providerId, { promise, abortController });
  return promise;
}
