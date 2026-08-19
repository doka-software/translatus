# Connect-flow UI 規格（框架無關）

任何 App（Tauri / Electron / Web）接 llm-subscription-kit 時，連接介面照此規格做。
參考實作：`llm-connect-panel.js`（vanilla Web Component，可直接用、可主題化）。
本規格描述公開套件實際提供的 Claude 連接流程。

## 資訊架構

1. **Picker**（卡片牆）：每個 provider 一張卡 = 名稱 + 推薦標記 + 活狀態 chip + 「設定」。
   - Codex 與 Claude 都帶各自的情境提醒，只有選擇該 provider 的訂閱模式時顯示。
   - 開啟面板的當下就對所有 provider 發 probe——卡片狀態必須是活的，不是上次的快取。
2. **Setup**（單 provider）：返回鍵 + provider 名 + 活 chip + 重新檢查鈕。
   - **步驟 1**：官方安裝/登入連結。我們永遠不自己做 OAuth、不碰、不存任何訂閱憑證。
   - **步驟 2**：「認證方式」radio。**選擇即生效即 probe，沒有儲存鈕。**
     - `subscription`：零輸入。radio 下方顯示該 provider 的 `policyNote` 原文。
     - `api-key`：密碼欄 + 儲存 + 對應 console 連結。key 由宿主 App 保管（kit 無狀態），每請求用 `X-LLM-API-Key` 傳。

宿主另外必須以 `runner-token` 屬性提供 sidecar 本機 access token；元件會將它作為
`Authorization: Bearer` 送出。這個 token 只是本機存取門，永遠不會送到模型 provider。

## 狀態 chip（四態）

| status | 文案 | 視覺 |
|---|---|---|
| `checking` | 檢查中… | 中性色點，pulse 動畫 |
| `connected` | 已連接 | 綠 |
| `not-connected` | 未連接 | 琥珀（警示不指責） |
| （無） | — | 尚未檢查不顯示 chip |

## 超越 Pencil 的三件事（必做）

1. **分層診斷**：`not-connected` 時顯示三層燈號——執行環境 ✓/✗、本機憑證 ✓/✗、連線測試 ✗(原因)，
   加 `hint` 文案（來自 `/v1/auth/probe` 回傳）。Pencil 只有一顆琥珀燈，使用者不知道差在哪一層。
2. **Keychain 等待提示**：`checking` 超過 10 秒 → 顯示「若系統跳出鑰匙圈授權視窗，請允許」。
   macOS 對新 binary 路徑首次讀 keychain 必跳系統視窗，否則 spinner 與當機無法區分。
3. **情境提醒**：只有選擇該 provider 的 subscription radio 時顯示其提醒；不要把提醒藏進 tooltip。

## 事件契約（Web Component）

- `connect-changed` `{provider, status, mode}`：每次 probe 落地。
- `provider-picked` `{provider, model}`：probe 成功（connected）時，宿主可以拿去當預設翻譯引擎。

## 主題化

全部視覺由 CSS custom properties 控制（`--lsk-*`）。宿主負責把它融進自己的設計語言
（ebook-translator 用墨流し主題覆蓋；canvas-app 用自己的）。元件不自帶品牌色。

## 每條規格的推導出處

完整證據鏈保存在本專案的非公開設計檔案中（competitive teardown §9 出處對照表）
（原始碼節錄封存於 `tech-design/raw/pencil-1.1.62/`，截圖於 `tech-design/screenshots/`）。對應關係：

| 本規格條目 | 推導自 |
|---|---|
| Picker 卡片牆 + 活狀態 | 對照表 #4、#6（Pencil `d$t` picker + 開設定即 probe） |
| 「選擇即生效即 probe，無儲存鈕」 | 對照表 #3（`mue` radio onValueChange + live log） |
| 四態 chip 與配色語意 | 對照表 #5（`r$t`/`i$t`） |
| 兩步 Setup（官方連結 + radio） | 對照表 #4、#9（`m$t`、`f$t`/`h$t`） |
| 零鍵 subscription | 對照表 #1（`getClaudeCodeEnv` 無 subscription case） |
| probe = 真實 1-turn 請求 | 對照表 #2（`probe()` + `AGENT_PROBE_SETTINGS`） |
| **Claude 警示** | `registry.js` 的公開政策說明 |
| **分層診斷**（我們的） | 對照表 #13 摩擦觀察：Pencil 琥珀燈無診斷、Keychain 卡死難辨 |
| **>10s Keychain 提示**（我們的） | 對照表 #13（live log 19:09:08 probe 被 Keychain 卡 45s abort） |
| settings/onboarding 同面板 | 對照表 #10（`QBt` wizard 模式） |
