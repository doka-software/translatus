// Security regression: every dynamic value interpolated into the panel's
// innerHTML must pass through esc()/escUrl(). Probe results carry provider/SDK
// error strings (untrusted); an unescaped `<img onerror=…>` in an error body
// would execute in the host page (XSS in a privileged webview when embedded).
//
// Run: node test/panel-escape.test.js
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

// ---- 1. Behavioural: esc()/escUrl() neutralize hostile values ----
// Minimal DOM stubs so the Web Component module can load under Node.
globalThis.HTMLElement = class {};
globalThis.customElements = { define() {}, get() {} };
globalThis.document = {
  createElement: () => ({ innerHTML: "", content: { cloneNode: () => ({}) } }),
};

const { esc, escUrl } = await import("../ui/llm-connect-panel.js");

const hostile = `<img src=x onerror=alert(1)> & "quotes" 'single'`;
const escaped = esc(hostile);
assert.ok(!escaped.includes("<img"), "tags must be neutralized");
assert.ok(!escaped.includes('"quotes"'), "double quotes must be escaped");
assert.ok(escaped.includes("&lt;img"), "lt escaped");
assert.ok(escaped.includes("&quot;quotes&quot;"), "quot escaped");
assert.ok(escaped.includes("&#39;single&#39;"), "single quote escaped");
assert.equal(esc(null), "", "null-safe");
assert.equal(esc(undefined), "", "undefined-safe");

assert.equal(escUrl("javascript:alert(1)"), "#", "javascript: URLs dropped");
assert.equal(escUrl("file:///etc/passwd"), "#", "file: URLs dropped");
assert.equal(escUrl("https://example.com/a?b=1"), "https://example.com/a?b=1");
assert.equal(
  escUrl(`https://e.com/"><script>`),
  "https://e.com/&quot;&gt;&lt;script&gt;",
  "http(s) URLs still escaped for the attribute context",
);

// ---- 2. Structural: the known untrusted sinks stay wrapped in esc() ----
const src = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "../ui/llm-connect-panel.js"),
  "utf8",
);
for (const mustContain of [
  'esc(res.hint || res.error?.message || "")', // probe error strings
  "esc(p.policyNote)", // registry policy note
  "escUrl(p.install.url)", // install link href
  "escUrl(p.apiKey.consoleUrl)", // console link href
  'esc(res.error?.reason || "?")', // diagnosis reason
  "esc(this.runnerUrl)", // runner URL echo
]) {
  assert.ok(
    src.includes(mustContain),
    `expected escaped sink in source: ${mustContain}`,
  );
}

console.log("panel-escape: all assertions passed");
