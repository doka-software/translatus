# llm-subscription-kit

讓你的 App 一鍵使用使用者既有的 **Codex/ChatGPT、Claude Code** 或 BYO API key，
不需要使用者懂 API。一個本機 sidecar + 一個可主題化的 connect UI 元件。

做法是**內嵌官方 Codex / Claude Agent SDK，訂閱模式不傳任何 API key**，
讓官方 CLI 撿本機已登入的訂閱憑證；連接狀態用「真實一輪請求」實測。

## 快速開始

```bash
npm install
npm start          # http://127.0.0.1:8765，並顯示當次的本機 access token
npm run smoke      # 離線測試，不花錢
npm run smoke:live # 加跑一次 Claude 真連線 probe（會使用額度）
```

```html
<script type="module" src="ui/llm-connect-panel.js"></script>
<llm-connect-panel runner-url="http://127.0.0.1:8765" runner-token="<local-token>"></llm-connect-panel>
```

## HTTP 契約

| Endpoint | 用途 |
|---|---|
| `GET /health` | liveness |
| `GET /v1/providers` | UI 用 registry（Claude 帶 policyNote） |
| `GET /v1/models` | OpenAI 形狀的模型清單 |
| `GET /v1/auth/status?provider=` | 分層診斷（執行環境/本機憑證），免費、不打 LLM |
| `POST /v1/auth/probe` `{provider}` | 真實 1-turn probe → connected / not-connected + 原因 + hint |
| `POST /v1/chat/completions` | OpenAI 形狀 completion（非串流） |

**認證策略 = 兩層分離**：
`Authorization: Bearer <local-token>` 是每個非 health 請求都必須通過的本機存取門，
絕不會送給模型。宿主 App 可用 `LLM_SUB_KIT_TOKEN` 指定；未指定時啟動會自動產生當次 token。
如果宿主還要走 provider API-key 模式，另用 `X-LLM-API-Key` 傳該次請求的 key。

## 安全模型

- 預設只接受無 `Origin` 的 loopback 請求（原生程式）。
- 只綁 loopback 也不夠：sidecar 預設必須驗證本機 access token，錯誤 bearer 不會被當成 provider key 放行。
- Webview 宿主（Tauri 等）需明確開白名單：`LLM_SUB_KIT_ALLOW_ORIGINS="tauri://localhost,http://tauri.localhost"`。
- 真正的 Web 部署必須把 kit 藏在自己有驗證的後端後面：這個 sidecar 花的是真錢。
- kit 永不儲存、永不轉傳使用者憑證；訂閱憑證從頭到尾在官方 CLI 自己手上。
- 預設同時只執行一個付費 completion；第二個會收到 429。確實需要並行時才設定
  `LLM_SUB_KIT_MAX_IN_FLIGHT`。HTTP client 中斷時，provider SDK 也會立即收到取消。
- Claude 路徑停用所有 built-in tools、skills、設定來源與 session transcript，並只把
  最小必要環境交給子程序。

## 政策現實（使用前必讀）

- **Codex / ChatGPT**：使用前請先確認書籍來源可信、內容沒有惡意指令；Codex 的本機資料存取範圍由 Codex 本身控制。若不確定，改用 API key 或 Ollama。
- **Claude**：2026-06-15 起第三方經訂閱認證改燒小額 Agent SDK credit（API 費率）；ToS 明文不允許第三方代路由訂閱憑證。
  UI 必須對使用者誠實顯示（registry 的 `policyNote`）。長期乾淨路徑是 API key。
- 架構上認證是可插拔層（per-request strategy）；安全邊界或政策改變時，只動這一層。

## 宿主整合

- **Rust**（ebook-translator）：et-core 的 OpenAI provider `--base-url http://127.0.0.1:8765/v1` 即用；
  UI 直接掛 `<llm-connect-panel>` 並設 allow-origins。
- **React**（canvas-app）：Web Component 直接渲染（React 19 原生支援 custom elements），或用事件橋接。
- UI 規格與必做清單見 `ui/SPEC.md`。
