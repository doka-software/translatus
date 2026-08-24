#!/usr/bin/env node
// Smoke test. Default: offline assertions only (no LLM call, no token spend).
// RUN_LIVE=1 adds real provider probes. They spend subscription/API quota.

import { createServer } from "../src/server.js";
import { codexChildEnv } from "../src/providers/codex.js";
import { claudeChildEnv, claudeQueryOptions } from "../src/providers/claude.js";

const LIVE = process.env.RUN_LIVE === "1";
let failures = 0;
let n = 0;

function check(name, cond, extra = "") {
  n++;
  if (cond) console.log(`ok ${n} - ${name}`);
  else {
    failures++;
    console.log(`NOT OK ${n} - ${name} ${extra}`);
  }
}

const TEST_LOCAL_TOKEN = "lsk-smoke-local-token-123456";
const server = createServer({ localToken: TEST_LOCAL_TOKEN });
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const port = server.address().port;
const base = `http://127.0.0.1:${port}`;

const j = (path, opts = {}) => {
  const headers = new Headers(opts.headers || {});
  headers.set("Authorization", `Bearer ${TEST_LOCAL_TOKEN}`);
  return fetch(`${base}${path}`, { ...opts, headers })
    .then(async (r) => ({ code: r.status, body: await r.json() }));
};

// --- health & registry ---
{
  const { code, body } = await j("/health");
  check("health ok", code === 200 && body.ok === true);
  check("health lists providers", Array.isArray(body.providers) && body.providers.includes("codex") && body.providers.includes("claude"));
  const denied = await fetch(`${base}/v1/providers`);
  check("default server rejects missing local token", denied.status === 401);
}
{
  const { code, body } = await j("/v1/providers");
  check("providers 200", code === 200);
  check("codex is listed", body.providers.some((p) => p.id === "codex"));
  check("claude carries policyNote", typeof body.providers.find((p) => p.id === "claude").policyNote === "string");
  check(
    "codex carries trusted-book reminder",
    /書籍來源可信/.test(body.providers.find((p) => p.id === "codex").policyNote),
  );
}
{
  const { code, body } = await j("/v1/models");
  check("models 200 + both families", code === 200 && body.data.some((m) => /^claude/.test(m.id)) && body.data.some((m) => /^gpt/.test(m.id)));
}

// --- layered diagnosis (no LLM call) ---
for (const p of ["claude", "codex"]) {
  const { code, body } = await j(`/v1/auth/status?provider=${p}`);
  check(`auth/status ${p} 200`, code === 200);
  check(`auth/status ${p} has runtime+credentials layers`, "runtime" in body && "credentials" in body);
}

// The Codex agentic child gets an explicit allowlist, never the sidecar's
// ambient cloud credentials or private app state.
{
  const env = codexChildEnv({
    HOME: "/safe/home",
    PATH: "/usr/bin",
    AWS_SECRET_ACCESS_KEY: "canary",
    OPENAI_API_KEY: "canary",
    PRIVATE_APP_STATE: "canary",
  });
  check("codex child keeps required environment", env.HOME === "/safe/home" && env.PATH === "/usr/bin");
  check("codex child drops ambient secrets", !JSON.stringify(env).includes("canary"));
}
{
  const env = claudeChildEnv(undefined, {
    HOME: "/safe/home",
    PATH: "/usr/bin",
    USER: "reader",
    AWS_SECRET_ACCESS_KEY: "canary",
    OPENAI_API_KEY: "canary",
    PRIVATE_APP_STATE: "canary",
  });
  const options = claudeQueryOptions({ model: "claude-haiku-4-5", system: "test", cwd: "/private/empty" });
  check("claude child keeps required environment", env.HOME === "/safe/home" && env.PATH === "/usr/bin");
  // Regression lock: dropping USER breaks macOS keychain OAuth resolution and
  // kills subscription mode with "Not logged in" while the user IS logged in.
  check("claude child keeps USER for keychain login", env.USER === "reader");
  check("claude child drops ambient secrets", !JSON.stringify(env).includes("canary"));
  check(
    "claude disables every built-in tool",
    Array.isArray(options.tools) && options.tools.length === 0 &&
      Array.isArray(options.allowedTools) && options.allowedTools.length === 0 &&
      Array.isArray(options.skills) && options.skills.length === 0 &&
      options.permissionMode === "dontAsk" && options.persistSession === false &&
      options.cwd === "/private/empty",
  );
}

// A cancelled MCP/client request must reach the provider child promptly;
// otherwise the sidecar can continue spending subscription quota after the
// caller has gone away.
{
  let providerStarted;
  const started = new Promise((resolve) => { providerStarted = resolve; });
  let providerAborted;
  const aborted = new Promise((resolve) => { providerAborted = resolve; });
  const hangingCodex = async ({ abortController }) => {
    providerStarted();
    return new Promise((resolve, reject) => {
      abortController.signal.addEventListener("abort", () => {
        providerAborted();
        reject(abortController.signal.reason || new Error("aborted"));
      }, { once: true });
    });
  };
  const cancellationServer = createServer({ runCodexImpl: hangingCodex, localToken: TEST_LOCAL_TOKEN });
  await new Promise((resolve) => cancellationServer.listen(0, "127.0.0.1", resolve));
  const cancellationPort = cancellationServer.address().port;
  const clientAbort = new AbortController();
  const request = fetch(`http://127.0.0.1:${cancellationPort}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${TEST_LOCAL_TOKEN}` },
    body: JSON.stringify({ model: "gpt-5.4-mini", messages: [{ role: "user", content: "wait" }] }),
    signal: clientAbort.signal,
  }).catch(() => null);
  await started;
  clientAbort.abort();
  const abortObserved = await Promise.race([
    aborted.then(() => true),
    new Promise((resolve) => setTimeout(() => resolve(false), 1_000)),
  ]);
  await request;
  await new Promise((resolve) => cancellationServer.close(resolve));
  check("client disconnect aborts provider promptly", abortObserved);
}

// Keep the default quota boundary small: a second paid completion is rejected
// while the first is still running.
{
  let providerStarted;
  const started = new Promise((resolve) => { providerStarted = resolve; });
  const hangingCodex = async ({ abortController }) => {
    providerStarted();
    return new Promise((resolve, reject) => {
      const stop = () => reject(new Error("stopped"));
      abortController.signal.addEventListener("abort", stop, { once: true });
    });
  };
  const cappedServer = createServer({ runCodexImpl: hangingCodex, maxInFlight: 1, localToken: TEST_LOCAL_TOKEN });
  await new Promise((resolve) => cappedServer.listen(0, "127.0.0.1", resolve));
  const cappedPort = cappedServer.address().port;
  const firstAbort = new AbortController();
  const first = fetch(`http://127.0.0.1:${cappedPort}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${TEST_LOCAL_TOKEN}` },
    body: JSON.stringify({ model: "gpt-5.4-mini", messages: [{ role: "user", content: "wait" }] }),
    signal: firstAbort.signal,
  }).catch(() => null);
  await started;
  const second = await fetch(`http://127.0.0.1:${cappedPort}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${TEST_LOCAL_TOKEN}` },
    body: JSON.stringify({ model: "gpt-5.4-mini", messages: [{ role: "user", content: "second" }] }),
  });
  check("sidecar caps paid completions", second.status === 429);
  firstAbort.abort();
  await first;
  await new Promise((resolve) => cappedServer.close(resolve));
}
{
  const { code } = await j("/v1/auth/status?provider=nope");
  check("auth/status unknown provider -> 400", code === 400);
}

// --- local model detection (graceful when Ollama absent) ---
{
  const { code, body } = await j("/v1/local/models");
  check("local/models 200 + ollama shape", code === 200 && "ollama" in body && typeof body.ollama.available === "boolean" && Array.isArray(body.ollama.models));
}

// --- request guards ---
{
  const { code } = await j("/v1/chat/completions", { method: "POST", body: "{not json", headers: { "Content-Type": "application/json" } });
  check("invalid JSON -> 400", code === 400);
}
{
  const { code } = await j("/v1/chat/completions", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model: "mystery-9000", messages: [{ role: "user", content: "hi" }] }),
  });
  check("unknown model -> 400", code === 400);
}
{
  const r = await fetch(`${base}/health`, { headers: { Origin: "https://evil.example" } });
  check("cross-origin blocked -> 403", r.status === 403);
}
{
  const r = await fetch(`${base}/v1/auth/probe`, { method: "OPTIONS", headers: { Origin: "https://evil.example" } });
  check("preflight from non-allowlisted origin -> 403", r.status === 403);
}

// --- local token gate (child process with LLM_SUB_KIT_TOKEN) ---
{
  const { spawn } = await import("node:child_process");
  const gPort = 17000 + Math.floor(Math.random() * 999);
  const child = spawn(process.execPath, ["src/server.js"], {
    env: { ...process.env, LLM_SUB_KIT_PORT: String(gPort), LLM_SUB_KIT_TOKEN: "lsk-test-secret-123" },
    stdio: ["ignore", "ignore", "pipe"],
  });
  await new Promise((resolve, reject) => {
    child.stderr.on("data", (d) => /listening/.test(String(d)) && resolve());
    setTimeout(() => reject(new Error("gated child boot timeout")), 5000);
  }).catch((e) => check("gated child boots", false, e.message));
  const gbase = `http://127.0.0.1:${gPort}`;
  const h = await fetch(`${gbase}/health`);
  check("gated /health open + flagged", h.status === 200 && (await h.json()).gated === true);
  const noTok = await fetch(`${gbase}/v1/providers`);
  check("gated endpoint without token -> 401", noTok.status === 401);
  const wrongToken = await fetch(`${gbase}/v1/providers`, { headers: { Authorization: "Bearer sk-someone-elses-key" } });
  check("wrong bearer cannot bypass local access gate", wrongToken.status === 401);
  const okTok = await fetch(`${gbase}/v1/providers`, { headers: { Authorization: "Bearer lsk-test-secret-123" } });
  check("gated endpoint with token -> 200", okTok.status === 200);
  const byoKey = await fetch(`${gbase}/v1/providers`, {
    headers: { Authorization: "Bearer lsk-test-secret-123", "X-LLM-API-Key": "sk-someone-elses-key" },
  });
  check("provider API key is separate from local access token", byoKey.status === 200);
  child.kill();
}

// --- allowlisted webview origin (child process; env is read at module load) ---
{
  const { spawn } = await import("node:child_process");
  const childPort = 18000 + Math.floor(Math.random() * 10000);
  const child = spawn(process.execPath, ["src/server.js"], {
    env: { ...process.env, LLM_SUB_KIT_PORT: String(childPort), LLM_SUB_KIT_ALLOW_ORIGINS: "tauri://localhost" },
    stdio: ["ignore", "ignore", "pipe"],
  });
  await new Promise((resolve, reject) => {
    child.stderr.on("data", (d) => /listening/.test(String(d)) && resolve());
    child.on("exit", () => reject(new Error("child died")));
    setTimeout(() => reject(new Error("child boot timeout")), 5000);
  }).catch((e) => check("allowlist child boots", false, e.message));
  const cbase = `http://127.0.0.1:${childPort}`;
  const ok = await fetch(`${cbase}/health`, { headers: { Origin: "tauri://localhost" } });
  check("allowlisted origin -> 200 + CORS header", ok.status === 200 && ok.headers.get("access-control-allow-origin") === "tauri://localhost");
  const pre = await fetch(`${cbase}/v1/auth/probe`, { method: "OPTIONS", headers: { Origin: "tauri://localhost" } });
  check("allowlisted preflight -> 204", pre.status === 204);
  const bad = await fetch(`${cbase}/health`, { headers: { Origin: "https://evil.example" } });
  check("non-allowlisted origin still 403 on allowlist server", bad.status === 403);
  child.kill();
}

// --- live (subscription-spending) section ---
if (LIVE) {
  for (const p of ["codex", "claude"]) {
    const { code, body } = await j("/v1/auth/probe", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ provider: p }),
    });
    check(`live probe ${p} responds`, code === 200 && ["connected", "not-connected"].includes(body.status), JSON.stringify(body).slice(0, 200));
    console.log(`# ${p}: ${body.status} ${body.elapsedMs}ms ${body.error ? body.error.reason : ""}`);
  }
} else {
  console.log("# live probes skipped (RUN_LIVE=1 to enable; spends real subscription tokens)");
}

server.close();
console.log(failures === 0 ? `# PASS (${n} checks)` : `# FAIL (${failures}/${n})`);
process.exit(failures === 0 ? 0 : 1);
