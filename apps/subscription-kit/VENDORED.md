# Vendored from llm-subscription-kit

Upstream: a private repository held in the container workspace. Its address is
deliberately not recorded here — naming a private repo in a tree that gets
published tells the world an unpublished asset exists and where to knock.

Provenance: base `cef4db8` (2026-06-15), plus a selective sync of
`package.json`, `src/providers/claude.js`, `src/providers/registry.js`,
`ui/llm-connect-panel.js`, and the provenance comment in `src/server.js`
from upstream `4389851` + `da88264` (2026-07-06 — MIT relicense, neutral
Claude policy wording, lockfile license sync). Upstream `43cbe75` (SSE
support on `/v1/chat/completions`) is intentionally **not** vendored yet.

2026-07-07 security sync from upstream `e8605a7`: `ui/llm-connect-panel.js`
(HTML-escape every dynamic innerHTML value via `esc()`/`escUrl()` — XSS
hardening), `test/panel-escape.test.js`, and the `npm test` script wiring.

2026-08-02 npm supply-chain fix: this copy ran `npm audit fix` locally first
(fast-uri 3.1.5, hono 4.12.33, @hono/node-server 2.0.12, body-parser 2.3.0,
@modelcontextprotocol/sdk 1.30.0; audit=0, npm test 24 green). Upstream was
then given the same fix as `00f1f1b` — lockfile versions verified identical,
so this copy is aligned with upstream on this batch.

2026-08-15 supply-chain + parity sync: second npm advisory batch fixed on both
sides (`npm audit fix`; notably ip-address SSRF misclassification, high) —
upstream `34251d6`, this copy's lockfile verified version-identical after the
same fix; audit=0, npm test 24 green on both. The loopback guarantees that had
shipped downstream-first in this copy (enforced loopback-only bind + socket-peer
`isLoopbackPeer` in `originAllowed` + `test/loopback.test.js`) are now canonical
upstream as `a30d2ec`, restoring 先改上游 parity. Remaining intentional deltas:
upstream SSE (`43cbe75`, still not vendored) and provenance naming comments.

改 kit 請改上游 repo 再重新 vendor（rsync src/ ui/ package*.json README.md），不要只改這份副本。

2026-08-16 發布安全調整：Codex 路徑改用最小環境、空白暫存工作目錄、停用 MCP、
web search 與 shell 環境繼承，並只在使用者選擇 Codex 訂閱時提醒先判斷書籍來源與
內容是否可信。書籍來源判斷與 Codex 本身的資料存取邊界不由本套件代辦。

2026-08-19 selective sync (E2E findings, upstream first then vendored):
`src/server.js` request-timeout default 150s → 600s, and
`src/providers/claude.js` + `src/providers/registry.js` current-generation
model ids (`claude-opus-5`, `claude-sonnet-5`; default `claude-sonnet-5`).
Copied by hand rather than wholesale — this copy carries `claudeChildEnv` /
`claudeQueryOptions` exports that upstream does not, and a full file copy
silently drops them (it did, once, during this sync; `npm test` caught it).

2026-08-24 sync from upstream `4df43a8`, applied as surgical patches (this
copy's hardened structure — token gate, claudeQueryOptions, workDir — is
kept): `EADDRINUSE` startup error now names `LLM_SUB_KIT_PORT`; the Claude
SDK's native CLI binary is resolved once at startup, pinned via
`pathToClaudeCodeExecutable`, and logged (a per-request "Native CLI binary
not found" after an install swap now carries an honest restart hint);
README documents the port override. npm test 34 green.

2026-09-01 成本回報修正（先改上游再 vendor）：訂閱路徑的 `tokens_in` 嚴重低估。
開了 prompt caching 之後，Anthropic 的 `usage.input_tokens` 只剩最後一個 cache
breakpoint 之後的未快取尾巴，前綴全落在 `cache_creation_input_tokens` /
`cache_read_input_tokens`；kit 只讀前者，實測把 10,183 token 的請求回報成 10。
上游新增 `src/providers/usage.js`（三種 input token 分欄正規化），claude.js /
codex.js 改走它，server.js 的 OpenAI usage 改成 `prompt_tokens` = 真實總量 +
`prompt_tokens_details`（`cached_tokens` 是 OpenAI 既有欄位，`cache_creation_*`
是本 kit 擴充），並補 14 個離線 regression check。三種 token 單價不同
（cache read 0.1x／5m write 1.25x／1h write 2x 基礎 input 價，2026-09-01 查證
<https://platform.claude.com/docs/en/about-claude/pricing#prompt-caching>），
所以刻意不加總成一個數字，加權留給呼叫端（本 repo 是 `estimate_cost_usd`）。
Codex 路徑同時修正：Codex 的 `input_tokens` 本來就含 `cached_input_tokens`，
不拆開會把該打一折的 token 用全價計費。

上游對應提交：`9533fa7`。這份副本與該提交逐檔相同，唯一差異仍是先前記錄的
provenance 註解、安全強化結構與未 vendor 的 SSE（`43cbe75`）。
