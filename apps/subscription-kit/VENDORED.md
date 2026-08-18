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
