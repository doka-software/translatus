#!/usr/bin/env node
// llm-subscription-kit runner — a local sidecar that lets desktop/web apps use
// the user's Codex/ChatGPT or Claude Code subscription, or BYO API keys,
// behind one OpenAI-compatible HTTP contract.
//
// Extracted from Translatus's apps/agent-runner and generalized after the
// 2026-06-12 Pencil.dev UI/UX teardown (tech-design/PENCIL-UX-TEARDOWN.md).
//
// Endpoints
//   GET  /health                       -> { ok, providers, version }
//   GET  /v1/models                    -> OpenAI-shaped model list
//   GET  /v1/providers                 -> registry for UIs (including policy notes)
//   GET  /v1/auth/status?provider=x    -> layered diagnosis (runtime, credentials) — free, no LLM call
//   POST /v1/auth/probe {provider}     -> live 1-turn probe -> connected / not-connected + why
//   POST /v1/chat/completions          -> OpenAI-shaped completion (non-streaming)
//
// Auth strategy per request:
//   Authorization: Bearer <local access token> is always required.
//   X-LLM-API-Key optionally selects provider API-key mode for that request.

import http from "node:http";
import { randomBytes } from "node:crypto";
import { ALL_MODELS, PROVIDERS, providerOf } from "./providers/registry.js";
import { runClaude } from "./providers/claude.js";
import { runCodex } from "./providers/codex.js";
import { diagnose } from "./auth/diagnose.js";
import { probe } from "./probe.js";

// A stray env key would silently flip billing from subscription to API for
// every request. Strip at startup; api-key mode is per-request only.
for (const k of ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "OPENAI_API_KEY"]) {
  if (process.env[k]) {
    console.error(`[llm-sub-kit] WARNING: ${k} was set; unsetting so subscription mode stays subscription-billed.`);
    delete process.env[k];
  }
}

const PORT = Number(process.env.LLM_SUB_KIT_PORT || process.env.ET_RUNNER_PORT || 8765);
// Loopback only, and enforced rather than merely defaulted. This process holds
// the operator's *subscription* credentials and answers without any per-request
// auth, so a non-loopback bind would hand that to the network. SECURITY.md
// states this as a guarantee, so it has to be one: an operator who overrides it
// gets a refusal to start, not a quiet exposure.
const HOST = process.env.LLM_SUB_KIT_HOST || "127.0.0.1";
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "::1", "localhost"]);
if (!LOOPBACK_HOSTS.has(HOST)) {
  console.error(
    `[llm-sub-kit] FATAL: refusing to bind to ${HOST}. This sidecar can spend ` +
      `subscription quota, so it may only listen on ` +
      `loopback (127.0.0.1, ::1, localhost). Put a reverse proxy in front of it if ` +
      `you need remote access, and add your own authentication there.`
  );
  process.exit(1);
}
const CODEX_EFFORT = process.env.LLM_SUB_KIT_CODEX_EFFORT || "low";
// Per-request wall-clock ceiling. Without it a wedged provider subprocess (a
// A provider child that never returns after a network drop can hold the
// connection open forever and freeze the
// caller's whole job. On timeout we ABORT the provider (killing the child) and
// return an error the caller's retry logic can act on.
// Book-length work is the reason this sidecar exists, and one batch of a real
// chapter routinely generates 3k-10k tokens: measured completions here run
// 48s, 86s, 121s. A 150s ceiling clipped the largest of them, and each clip
// cost the full 150s before the caller could split and retry — one chapter
// stalled for the better part of an hour that way. Ten minutes leaves the
// timeout as a hang detector, which is what it is for.
const REQUEST_TIMEOUT_MS = Number(process.env.LLM_SUB_KIT_REQUEST_TIMEOUT_MS || 600_000);
const MAX_IN_FLIGHT = Math.max(1, Number(process.env.LLM_SUB_KIT_MAX_IN_FLIGHT || 1));
const MAX_BODY_BYTES = 8 * 1024 * 1024;
const VERSION = "0.2.0";

// Local auth is fail-closed. Host apps normally inject a per-install secret;
// standalone `npm start` gets a fresh one-time token printed at startup.
const CONFIGURED_LOCAL_TOKEN = process.env.LLM_SUB_KIT_TOKEN?.trim() || null;
const GENERATED_LOCAL_TOKEN = randomBytes(32).toString("base64url");

function bearerOf(req) {
  const h = req.headers.authorization || "";
  const m = /^Bearer\s+(.+)$/i.exec(h);
  return m ? m[1].trim() : undefined;
}

function providerApiKeyOf(req) {
  const value = req.headers["x-llm-api-key"];
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

// The local access token is never forwarded to a provider. A provider API key
// has its own header and is considered only after local authentication passes.
function authOf(req, localToken) {
  const bearer = bearerOf(req);
  if (!bearer || bearer !== localToken) return { ok: false, apiKey: undefined };
  return { ok: true, apiKey: providerApiKeyOf(req) };
}

function partText(content) {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content.map((p) => (typeof p === "string" ? p : p?.text ?? "")).join("");
  }
  return "";
}

function splitMessages(messages) {
  const system = messages.filter((m) => m.role === "system").map((m) => partText(m.content)).join("\n\n");
  const prompt = messages.filter((m) => m.role !== "system").map((m) => partText(m.content)).join("\n\n");
  return { system, prompt };
}

async function handleCompletion(body, apiKey, abortController, providers) {
  const { model, messages } = body;
  if (!model || !Array.isArray(messages)) {
    const e = new Error("missing model or messages");
    e.code = 400;
    throw e;
  }
  const route = providerOf(model);
  if (!route) {
    const e = new Error(`unknown model '${model}'; known: ${ALL_MODELS.join(", ")}`);
    e.code = 400;
    throw e;
  }
  const { system, prompt } = splitMessages(messages);
  const t0 = Date.now();
  // Abort the provider (and its child process) if it blows the wall-clock budget.
  const timer = setTimeout(
    () => abortController.abort(new Error(`request exceeded ${REQUEST_TIMEOUT_MS}ms`)),
    REQUEST_TIMEOUT_MS,
  );
  let result;
  try {
    result = route === "claude"
      ? await providers.runClaude({ model, system, prompt, apiKey, abortController })
      : await providers.runCodex({ model, system, prompt, apiKey, effort: CODEX_EFFORT, abortController });
  } catch (e) {
    if (abortController.signal.aborted) {
      const disconnected = abortController.signal.reason?.code === "CLIENT_DISCONNECTED";
      const te = new Error(
        disconnected
          ? "client disconnected; provider aborted"
          : `provider timed out after ${REQUEST_TIMEOUT_MS}ms (aborted)`,
      );
      te.code = disconnected ? 499 : 504;
      throw te;
    }
    throw e;
  } finally {
    clearTimeout(timer);
  }
  const { text, usage, reportedModel } = result;
  console.error(
    `[llm-sub-kit] requested=${model} reported=${reportedModel ?? "n/a"} ${apiKey ? "api-key" : "subscription"} ok ${Date.now() - t0}ms in=${usage.input_tokens} out=${usage.output_tokens}`,
  );
  return {
    id: `lsk-${Date.now()}`,
    object: "chat.completion",
    created: Math.floor(Date.now() / 1000),
    model,
    // Routing evidence: the model id the SDK/CLI itself reported (null when
    // the SDK doesn't surface one) — NOT an echo of the request.
    system_fingerprint: reportedModel ?? undefined,
    choices: [{ index: 0, message: { role: "assistant", content: text }, finish_reason: "stop" }],
    usage: {
      prompt_tokens: usage.input_tokens,
      completion_tokens: usage.output_tokens,
      total_tokens: usage.input_tokens + usage.output_tokens,
    },
  };
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let data = "",
      size = 0;
    req.on("data", (c) => {
      size += c.length;
      if (size > MAX_BODY_BYTES) {
        reject(Object.assign(new Error("request body too large"), { code: 413 }));
        req.destroy();
        return;
      }
      data += c;
    });
    req.on("end", () => resolve(data));
    req.on("error", reject);
  });
}

// This localhost server spends real subscription quota. Block the browser
// cross-origin / DNS-rebinding vector: native callers send no Origin header and
// Host = loopback; a web page fetch() always sends Origin. Webview hosts (e.g.
// a Tauri UI at tauri://localhost) must opt in explicitly via
// LLM_SUB_KIT_ALLOW_ORIGINS="tauri://localhost,http://tauri.localhost".
// Real web deployments must front the kit with their own authenticated backend.
const ALLOW_ORIGINS = (process.env.LLM_SUB_KIT_ALLOW_ORIGINS || "")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);

/** Is the peer on this machine? Derived from the socket, not from a header. */
export function isLoopbackPeer(req) {
  // `Host` is attacker-controlled: anyone who can reach the port can send
  // `Host: localhost`. The remote address is a property of the connection and
  // cannot be forged by the client.
  const addr = req.socket?.remoteAddress || "";
  // Node reports IPv4 peers as "::ffff:127.0.0.1" on dual-stack sockets.
  const bare = addr.replace(/^::ffff:/, "");
  return bare === "127.0.0.1" || bare === "::1" || bare.startsWith("127.");
}

export function originAllowed(req) {
  const origin = req.headers.origin;
  if (!origin) {
    // No Origin means a non-browser client. The only thing that makes that safe
    // is that it came from this machine — and that is a socket property, not a
    // header. The hostname check stays as the DNS-rebinding defense on top.
    if (!isLoopbackPeer(req)) return false;
    const hostname = (req.headers.host || "").replace(/:\d+$/, "");
    return hostname === "127.0.0.1" || hostname === "localhost";
  }
  return ALLOW_ORIGINS.includes(origin);
}

export function createServer({
  runClaudeImpl = runClaude,
  runCodexImpl = runCodex,
  maxInFlight = MAX_IN_FLIGHT,
  localToken = CONFIGURED_LOCAL_TOKEN || GENERATED_LOCAL_TOKEN,
} = {}) {
  if (typeof localToken !== "string" || localToken.length < 16) {
    throw new Error("local access token must contain at least 16 characters");
  }
  let activeCompletions = 0;
  const providers = { runClaude: runClaudeImpl, runCodex: runCodexImpl };
  return http.createServer(async (req, res) => {
    const send = (code, obj) => {
      if (res.destroyed || res.writableEnded) return;
      res.writeHead(code, { "Content-Type": "application/json" });
      res.end(JSON.stringify(obj));
    };
    const url = new URL(req.url || "/", `http://${req.headers.host || "localhost"}`);
    const path = url.pathname;
    try {
      if (!originAllowed(req)) return send(403, { error: { message: "forbidden (cross-origin / non-loopback)" } });
      if (req.headers.origin) {
        res.setHeader("Access-Control-Allow-Origin", req.headers.origin);
        res.setHeader("Vary", "Origin");
      }
      if (req.method === "OPTIONS") {
        // CORS preflight for allowlisted webview origins (JSON POSTs trigger it).
        res.writeHead(204, {
          "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
          "Access-Control-Allow-Headers": "Content-Type, Authorization, X-LLM-API-Key",
          "Access-Control-Max-Age": "600",
        });
        return res.end();
      }

      if (req.method === "GET" && path === "/health") {
        return send(200, { ok: true, providers: PROVIDERS.map((p) => p.id), version: VERSION, port: PORT, gated: true });
      }
      const auth = authOf(req, localToken);
      if (!auth.ok) {
        return send(401, { error: { message: "missing or invalid local token" } });
      }
      if (req.method === "GET" && path === "/v1/models") {
        return send(200, {
          object: "list",
          data: ALL_MODELS.map((id) => ({ id, object: "model", owned_by: "subscription" })),
        });
      }
      if (req.method === "GET" && path === "/v1/providers") {
        return send(200, { providers: PROVIDERS });
      }
      if (req.method === "GET" && path === "/v1/local/models") {
        const { detectLocal } = await import("./local.js");
        return send(200, await detectLocal());
      }
      if (req.method === "GET" && path === "/v1/auth/status") {
        const provider = url.searchParams.get("provider");
        try {
          return send(200, await diagnose(provider));
        } catch (e) {
          return send(e.code === 400 ? 400 : 500, { error: { message: e.message } });
        }
      }
      if (req.method === "POST" && path === "/v1/auth/probe") {
        let body;
        try {
          body = JSON.parse((await readBody(req)) || "{}");
        } catch {
          return send(400, { error: { message: "invalid JSON" } });
        }
        try {
          return send(200, await probe(body.provider, { apiKey: auth.apiKey }));
        } catch (e) {
          return send(e.code === 400 ? 400 : 500, { error: { message: e.message } });
        }
      }
      if (req.method === "POST" && path === "/v1/chat/completions") {
        if (activeCompletions >= maxInFlight) {
          return send(429, {
            error: {
              message: `too many in-flight completions (limit ${maxInFlight})`,
              type: "llm_subscription_kit_busy",
            },
          });
        }
        let body;
        try {
          body = JSON.parse((await readBody(req)) || "{}");
        } catch (e) {
          return send(e.code === 413 ? 413 : 400, {
            error: { message: e.code === 413 ? "request too large" : "invalid JSON" },
          });
        }
        const abortController = new AbortController();
        const disconnected = () => {
          if (res.writableEnded || abortController.signal.aborted) return;
          const reason = new Error("client disconnected");
          reason.code = "CLIENT_DISCONNECTED";
          abortController.abort(reason);
        };
        req.once("aborted", disconnected);
        res.once("close", disconnected);
        activeCompletions += 1;
        try {
          return send(200, await handleCompletion(body, auth.apiKey, abortController, providers));
        } catch (e) {
          const code = e.code === 400 ? 400 : e.code === 429 ? 429 : e.code === 504 ? 504 : 502;
          if (e.code !== 499) {
            console.error(`[llm-sub-kit] ERROR model=${body && body.model}: ${e.message}`);
          }
          return send(code, { error: { message: e.message, type: "llm_subscription_kit_error" } });
        } finally {
          activeCompletions -= 1;
          req.off("aborted", disconnected);
          res.off("close", disconnected);
        }
      }
      send(404, { error: { message: "not found" } });
    } catch (e) {
      send(500, { error: { message: e.message } });
    }
  });
}

// Started directly (not imported as a lib): boot on the configured port.
// pathToFileURL, NOT string concat: paths with spaces (e.g. macOS
// "Application Support") percent-encode in import.meta.url and a naive
// `file://${argv[1]}` comparison silently never matches.
const { pathToFileURL } = await import("node:url");
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const localToken = CONFIGURED_LOCAL_TOKEN || GENERATED_LOCAL_TOKEN;
  const server = createServer({ localToken });
  server.on("error", (e) => {
    console.error(`[llm-sub-kit] FATAL: ${e.code || ""} ${e.message}`);
    if (e.code === "EADDRINUSE") {
      console.error(
        `[llm-sub-kit] port ${PORT} is taken by another process. ` +
          `Start on a different one: LLM_SUB_KIT_PORT=<port> npm start`,
      );
    }
    process.exit(1);
  });
  server.listen(PORT, HOST, () => {
    console.error(`[llm-sub-kit] listening on http://${HOST}:${PORT} (local access token required)`);
    if (!CONFIGURED_LOCAL_TOKEN) {
      console.error(`[llm-sub-kit] one-time local access token: ${localToken}`);
      console.error("[llm-sub-kit] paste this token into Translatus (interactive session: Settings -> Access token; CLI: OPENAI_API_KEY)");
    }
    // Pin the Claude SDK's native binary now rather than on first request:
    // resolved-at-startup keeps a live server working across an install swap,
    // and a missing binary is reported here once, not as a per-request 502.
    import("./providers/claude.js")
      .then(({ preflightClaudeNative }) => {
        const r = preflightClaudeNative();
        if (r.ok) console.error(`[llm-sub-kit] claude native binary pinned: ${r.path}`);
        else console.error(`[llm-sub-kit] claude native binary not resolved (${r.detail}) — Claude subscription calls may fail; Codex/API-key paths are unaffected`);
      })
      .catch((e) => console.error(`[llm-sub-kit] claude preflight skipped: ${e?.message || e}`));
  });
}
