# Security Policy

## Reporting a vulnerability

Please **do not** open a public issue for security problems.

Report vulnerabilities privately via
[GitHub private vulnerability reporting](https://github.com/doka-software/translatus/security/advisories/new)
(Security → "Report a vulnerability" on the repository page). You should
receive an initial response within a few days.

## Threat model

Translatus runs entirely on the user's machine — there is no server component
holding user data. Last full audit: 2026-08 (pre–open-source security review).

### What we defend against

**Your data staying yours — nothing is sent to us.**

- The engine and CLI make network requests to exactly one destination: the
  model endpoint **you** configure. There is no telemetry, no analytics, no
  crash reporting, no update pings, no phone-home of any kind.
- In subscription mode, the local sidecar (`apps/subscription-kit`) reaches the
  official Anthropic or OpenAI backend through its official SDK (using your own
  login). The sidecar itself binds to `127.0.0.1` only. Before using Codex
  subscription mode, users must decide whether they trust the book's source
  and content; Codex itself controls its local-data access.

**Credential safety.**

- The CLI never accepts an API key as a command-line argument, where it could
  appear in shell history or process listings. It resolves credentials from
  the OS keychain or the provider environment variable for the run only. When
  a key *is* saved (the engine's
  `secrets` store, used by desktop hosts), it goes into the OS keychain (macOS
  Keychain / Windows Credential Manager / Linux Secret Service) — never into
  config files, logs, or stdout.
- HTTP redirects are disabled on every request that carries a key, so a `30x`
  can never replay your bearer token to a different host.
- The MCP server refuses a caller-supplied `base_url` that is not loopback.
  Redirect hardening only covers the hop *after* the request leaves; the
  endpoint itself is chosen by whoever calls the tool, and an MCP caller is an
  agent whose inputs can be steered by anything it has read. A remote endpoint
  there would receive your key as a bearer token on the first hop, with the
  redirect guard never involved. When a caller does supply a `base_url`, the
  worker additionally runs without `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` in
  its environment and disables OS-keychain lookup, so there is no ambient
  credential to fall back on. Operators may explicitly bind a sidecar's local
  access token to one exact URL with `TRANSLATUS_MCP_ENDPOINT_URL` plus
  `TRANSLATUS_MCP_ENDPOINT_TOKEN`; that scoped token is never sent to any other
  caller-selected listener.
- The MCP server refuses `@file` in `profile` and `note_style`. The CLI expands
  those to read a local file; over MCP that is an arbitrary read on the
  machine running the server. Callers pass the text itself.
- The sidecar never persists credentials and never returns them in any
  response or log line; subscription mode additionally strips `*_API_KEY`
  environment variables so usage stays subscription-billed.

**Malicious book files — every EPUB/TXT is treated as untrusted input.**

- No filesystem extraction: EPUB entries are held in memory only, so zip-slip
  (`../` names) and symlink entries can never touch your disk.
- Nor can they be *laundered*: an entry whose name is absolute or contains a
  `..` component is dropped when the book is read, so it cannot ride into the
  translated output and attack whatever unpacks that next. OCF requires
  relative, `..`-free paths, so no legitimate entry is affected.
- Zip-bomb guards: per-entry (256 MiB) and whole-archive (1 GiB)
  decompressed-size caps, plus an entry-count cap (65,536).
- XML entity safety: custom and external DTD entities are never expanded — no
  XXE, no billion-laughs memory blowup.
- Element nesting is capped (256 levels) so a hostile document cannot overflow
  the stack via deep recursion.
- Each guard has a regression test with a malicious fixture
  (`crates/core/src/format/epub.rs`, `crates/core/src/format/dom.rs`).

**The job cache holds your book in the clear.**

- Resuming works by keeping translated passages and any margin notes in a
  SQLite file next to the output (`<output>.etjob`). That content is stored as
  plaintext, not hashed or encrypted — it has to be, because re-rendering reads
  it back.
- New cache files are created `0600` (owner-only) on Unix rather than inheriting
  the ambient umask. Delete the `.etjob` file when you no longer need to resume;
  nothing else depends on it.
- Final EPUB/TXT output is fully written and synced through an owner-only
  same-directory temporary file, then atomically renamed into place. A path
  swapped to a symlink while a long model job runs is replaced, not followed,
  and a crash cannot leave a half-written output at the destination.

**Subscription-sidecar quota boundary.**

- The sidecar accepts one paid completion at a time by default and returns 429
  for another; set `LLM_SUB_KIT_MAX_IN_FLIGHT` only if you deliberately want a
  higher limit. MCP likewise permits one translation/annotation job while
  estimates, pings, and cancellation remain available.
- Every non-health sidecar request requires an exact local access token. Host
  apps set `LLM_SUB_KIT_TOKEN`; standalone startup generates and prints a fresh
  one-time token. A wrong bearer is rejected rather than reinterpreted as a
  provider key, and the local token is never forwarded to a model.
- If the HTTP/MCP client disconnects or cancels, the abort signal reaches the
  provider SDK and its child instead of continuing until the normal timeout.
- The Claude SDK path exposes no built-in tools or skills, loads no settings,
  persists no session transcript, and receives a minimal child environment.

**Untrusted model output.**

- Translated text and annotations returned by the model are XML-escaped before
  they are written into the output EPUB — model output can never become live
  markup (`<script>`, event handlers) in your reader. Only the original book's
  own inline tags re-enter, via byte-faithful placeholder restore.
- The connect-panel Web Component HTML-escapes every runtime value (provider /
  SDK error strings are untrusted) and restricts link hrefs to `http(s)`.

### What we do NOT defend against

Honest boundaries — these are out of scope by design:

- **A malicious or dishonest model endpoint.** You choose the endpoint; we
  validate the *structure* of its output (placeholder integrity, escaping),
  not its meaning. An endpoint that returns wrong, biased, or garbage
  translations is not something software can catch.
- **The original book's own markup.** Output EPUBs preserve the source book
  byte-faithfully; a book whose markup exploits some *other* reader app stays
  exploitative. We add no new live markup, but we do not sanitize the original.
- **An attacker already running code as your user.** They can read the same
  files and keychain entries the engine can; that is the OS trust boundary.
- **A disclosed sidecar access token.** Treat the generated local token like a
  session secret. A process that obtains it can spend through the sidecar until
  that sidecar restarts; standalone tokens rotate on every start.

### Verify the "no telemetry" claim yourself

You do not have to take our word for it:

1. **Read the call sites.** The engine's only HTTP client lives in
   `crates/core/src/llm/openai.rs` (posts to `{your base_url}/chat/completions`).
   `grep -rn "reqwest\|fetch(" crates apps` — every hit is that client, the
   loopback sidecar, or the official provider SDKs.
2. **Watch the wire.** Run a translation under a network monitor (Little
   Snitch, `nettop`, Wireshark). The only connections you will see go to the
   model endpoint you configured (or the official provider backend in
   subscription mode).
3. **Check the dependencies.** No analytics/telemetry/crash-reporting SDK
   appears in `Cargo.toml`, `Cargo.lock`, or `apps/subscription-kit/package.json`.
