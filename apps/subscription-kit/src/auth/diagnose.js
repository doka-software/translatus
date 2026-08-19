// Layered auth diagnostics — answers "why is it not connected?" instead of
// Pencil's single amber light. Three layers, cheap to expensive:
//
//   1. runtime   — is the SDK (and its bundled CLI) resolvable?
//   2. credentials — does a local subscription credential exist?
//   3. probe     — does a real 1-turn request succeed? (see probe.js; on demand)
//
// Layer 2 only checks EXISTENCE (file stat / keychain metadata). It never reads
// secret values, so it cannot trigger a macOS Keychain password prompt.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";

function exists(p) {
  try {
    fs.accessSync(p);
    return true;
  } catch {
    return false;
  }
}

async function keychainItemExists(service) {
  if (process.platform !== "darwin") return false;
  return new Promise((resolve) => {
    // No -w: metadata lookup only, does not unlock the secret → no GUI prompt.
    execFile("security", ["find-generic-password", "-s", service], (err) => resolve(!err));
  });
}

async function sdkResolvable(pkg) {
  try {
    await import.meta.resolve(pkg);
    return true;
  } catch {
    try {
      // Fallback for runtimes without import.meta.resolve.
      const { createRequire } = await import("node:module");
      createRequire(import.meta.url).resolve(pkg);
      return true;
    } catch {
      return false;
    }
  }
}

export async function diagnoseClaude() {
  const home = os.homedir();
  const credFile = path.join(home, ".claude", ".credentials.json");
  const [runtime, keychain] = await Promise.all([
    sdkResolvable("@anthropic-ai/claude-agent-sdk"),
    keychainItemExists("Claude Code-credentials"),
  ]);
  const file = exists(credFile);
  return {
    provider: "claude",
    runtime: { ok: runtime, detail: "@anthropic-ai/claude-agent-sdk" },
    credentials: {
      ok: keychain || file,
      sources: { keychain, file: file ? credFile : null },
      hint: keychain || file ? null : "本機沒有 Claude Code 訂閱憑證：請安裝 Claude Code 並執行 claude /login",
    },
  };
}

export async function diagnoseCodex() {
  const authFile = path.join(os.homedir(), ".codex", "auth.json");
  const runtime = await sdkResolvable("@openai/codex-sdk");
  // Existence is sufficient for a cheap diagnosis. Never parse or return any
  // credential content from the sidecar's status endpoint.
  const credentialPresent = exists(authFile);
  return {
    provider: "codex",
    runtime: { ok: runtime, detail: "@openai/codex-sdk" },
    credentials: {
      ok: credentialPresent,
      sources: { file: credentialPresent ? authFile : null },
      hint: credentialPresent ? null : "本機沒有 Codex 登入：請安裝 Codex CLI 並執行 codex login（Sign in with ChatGPT）",
    },
  };
}

export async function diagnose(provider) {
  if (provider === "claude") return diagnoseClaude();
  if (provider === "codex") return diagnoseCodex();
  const e = new Error(`unknown provider '${provider}'`);
  e.code = 400;
  throw e;
}
