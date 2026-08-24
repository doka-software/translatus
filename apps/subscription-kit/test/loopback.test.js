// The sidecar answers with the operator's subscription credentials and has no
// per-request auth. Two things therefore have to hold, and neither did:
//
//   1. it may only ever listen on loopback — SECURITY.md states that as a
//      guarantee, but LLM_SUB_KIT_HOST could quietly move it onto the network;
//   2. a request with no Origin may only be trusted because it came from this
//      machine, and "this machine" has to be read off the socket. The old check
//      read the `Host` header, which any client can set to `localhost`.
//
// Run: node test/loopback.test.js

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const serverPath = path.join(here, "..", "src", "server.js");

let failures = 0;
function check(name, fn) {
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (e) {
    failures++;
    console.error(`  FAIL ${name}\n       ${e.message}`);
  }
}

const { originAllowed, isLoopbackPeer } = await import(serverPath);

// A request as the server sees it: headers plus the real socket.
const req = (headers, remoteAddress) => ({ headers, socket: { remoteAddress } });

console.log("peer identity comes from the socket, not the Host header");

check("a genuine loopback peer is accepted", () => {
  assert.equal(isLoopbackPeer(req({}, "127.0.0.1")), true);
  assert.equal(isLoopbackPeer(req({}, "::1")), true);
  // Dual-stack sockets report IPv4 peers in this form.
  assert.equal(isLoopbackPeer(req({}, "::ffff:127.0.0.1")), true);
});

check("a remote peer is not loopback however it labels itself", () => {
  assert.equal(isLoopbackPeer(req({}, "203.0.113.7")), false);
  assert.equal(isLoopbackPeer(req({}, "192.168.1.20")), false);
  assert.equal(isLoopbackPeer(req({}, undefined)), false);
});

check("a spoofed Host header cannot buy trust from a remote peer", () => {
  // This is the exact bypass: no Origin, `Host: localhost`, remote socket.
  assert.equal(
    originAllowed(req({ host: "localhost:8765" }, "203.0.113.7")),
    false,
    "a remote client claiming Host: localhost must be refused"
  );
  assert.equal(
    originAllowed(req({ host: "127.0.0.1:8765" }, "203.0.113.7")),
    false,
    "a remote client claiming Host: 127.0.0.1 must be refused"
  );
});

check("a real local client still works", () => {
  assert.equal(originAllowed(req({ host: "127.0.0.1:8765" }, "127.0.0.1")), true);
  assert.equal(originAllowed(req({ host: "localhost:8765" }, "::ffff:127.0.0.1")), true);
});

console.log("the loopback bind is enforced, not just defaulted");

check("a non-loopback LLM_SUB_KIT_HOST refuses to start", () => {
  const r = spawnSync(process.execPath, [serverPath], {
    env: { ...process.env, LLM_SUB_KIT_HOST: "0.0.0.0", LLM_SUB_KIT_PORT: "0" },
    encoding: "utf8",
    timeout: 15_000,
  });
  assert.notEqual(r.status, 0, "binding to 0.0.0.0 must be fatal");
  assert.match(
    r.stderr || "",
    /refusing to bind/i,
    `expected a refusal, got: ${(r.stderr || "").slice(0, 300)}`
  );
});

if (failures) {
  console.error(`\n${failures} failed`);
  process.exit(1);
}
console.log("\nall passed");
