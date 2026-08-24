<p align="center">
  <img src="docs/logo.png" width="128" alt="Translatus" />
</p>

<h1 align="center">Translatus</h1>

<p align="center">用你自己的 LLM，在本機翻譯、眉批整本書。</p>

<p align="center">
  <a href="https://doka.software/translatus"><img src="https://img.shields.io/badge/website-doka.software-4a4a4a.svg" alt="Website"></a>
  <a href="https://github.com/doka-software/translatus/actions/workflows/ci.yml"><img src="https://github.com/doka-software/translatus/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT"></a>
  <a href="README.md"><img src="https://img.shields.io/badge/README-English-b23a2a.svg" alt="English"></a>
</p>

## 為什麼是 Translatus

書印出來是固定的；讀它的人不是。語言、背景知識、時代距離，都會把人擋在
一本好書外面。Translatus 用你自己的模型，把整本書翻成你的語言、在你需要
的地方寫下眉批，讓一本固定的書，變成為你準備的版本。

把章節貼進聊天機器人：格式會壞、術語會忘、翻到一半會斷。Translatus 把「一整本書」當工作單位：

- **產出是一本真正的書。** 只有文字被翻譯。inline 標籤、屬性、實體、CSS、
  圖片與 EPUB 結構逐 byte 保留，成品在任何閱讀器打開都乾淨。
- **術語撐得過四百頁。** 專家模式先掃全書鎖定詞彙表，帶著前文記憶逐章翻譯，
  再對照原文校訂草稿。鎖定的術語用哨符替換強制執行，不是「模型答應會一致」。
- **中斷不花一毛錢。** 每一段都存進本機 SQLite 快取。當機、暫停、下週再翻，
  重跑不會對已譯段落重複計費。換版面（純譯文 ↔ 雙語對照）直接從快取重出，
  零 LLM 呼叫。
- **眉批，為你而寫。** 用勾的就能開始：九種服務（術語解釋到世界連結）、
  講解深淺（入門白話到內行）、書齋或陪讀聲線；也可以寫下你是誰，或乾脆
  讓你自己的 AI 替你填好整份讀者側寫（[schema 與標準 prompt](docs/READER-PROFILE.md)）。
  引擎先畫出全書線索圖，只挑值得停留的段落落註：段落之前鋪墊，或之後
  回指你讀過的內容（絕不預告後文），最後把所有眉批統一校閱一次。眉批
  只做中性的背景補充，清楚標示，絕不混進作者的文字，且由程式逐則檢查、
  永遠不會對你說話。
- **你的模型、你的選擇。** 可用本機 Codex/ChatGPT 或 Claude Code 登入、
  任何 OpenAI 相容 API key、或本機 Ollama。書籍內容只會送到你選定的模型端點；
  金鑰存在系統鑰匙圈。
- **也為 agent 而生。** CLI 講結構化 JSON、冪等、可安全續跑；還內建 MCP
  server：`translatus mcp` 讓 Claude Code 等 agent 直接估價、翻譯、眉批
  （[見下方](#接上你的-agentmcp)）。

## 什麼情況不適合你

- **你的書是 PDF。** 目前只支援 EPUB 與 TXT；PDF 是被問最多的格式，還沒開工。
- **只想用最少設定快速出一本雙語書。**
  [bilingual_book_maker](https://github.com/yihong0618/bilingual_book_maker)
  更簡單；Translatus 的價值在整本書的尺度。
- **期待工具自帶模型。** 不會有；品質就是你接上的那顆模型，
  弱的本機模型翻出來就是弱的。

## 快速開始

```bash
brew install doka-software/tap/translatus     # Homebrew（建置約一分鐘）
```

或從 [Releases 頁面](https://github.com/doka-software/translatus/releases)下載
預編譯執行檔（macOS Apple silicon／macOS Intel／Linux x86_64）放進 `PATH`，
或用 Rust（stable）從原始碼安裝：

```bash
cargo install --locked --path apps/cli        # 裝出 `translatus`
```

Linux 從原始碼編譯還需要作業系統 keychain 與 HTTPS adapter 使用的 D-Bus、
OpenSSL 開發套件：Ubuntu／Debian 執行 `sudo apt install libdbus-1-dev
libssl-dev pkg-config`，Fedora 執行 `sudo dnf install dbus-devel openssl-devel
pkgconf-pkg-config`。

不帶參數直接執行，會開啟互動介面：

```bash
translatus
```

它會找出你附近的書，指令列做得到的事它都做得到：**翻譯**與**眉批**是兩個
可獨立開關的服務，所以關掉翻譯、留著眉批，就是「只加眉批、不動原文」。每本書
可以各自設定目標語言、一般或專家深度、版面，以及眉批的四件事：你是誰、為什麼
讀這本、想要哪幾種幫助、眉批多密、用什麼語言寫。設定畫面則管模型來源（訂閱、
你自己的 API 金鑰、或本機 Ollama）、金鑰本身（存在作業系統的 keychain，不會寫
進設定檔）、base URL，以及一鍵連線測試。

互動介面會說五種語言：English、繁體中文、简体中文、日本語、한국어，跟著你的
終端機語系走，也可以用 `TRANSLATUS_LANG=zh_TW`（等等）強制指定。設定存放於本機
的 `settings.json`；想要獨立的設定檔，把 `TRANSLATUS_CONFIG_DIR` 指到別的地方
即可。

任何花錢的動作都要先看過量化摘要並確認，Esc 一律只退一步，而且介面能做的每件事
都有對應的指令參數，過程中會把等價指令印給你看。在管線或腳本中，不帶參數的
`translatus` 會改印用法說明，互動介面只在終端機下開啟。

```bash
# 先估算成本（不會真的翻譯）
translatus estimate book.epub --to "繁體中文"

# 翻譯一本書
translatus translate book.epub --to "繁體中文" --output book.zh.epub

# 專家模式：全書一致性，較慢，真書值得
translatus translate book.epub --to "繁體中文" --level expert

# 為讀得懂的書加眉批
translatus annotate book.epub --profile "@my-background.txt"

# 不想打字？用勾的（術語 / 歷史 / 世界連結⋯共九種服務）
translatus annotate book.epub --note-presets terms,world --note-level beginner

# 陪讀聲線＋從你熟悉的領域搭橋
translatus annotate book.epub --note-presets culture,world --note-voice companion \
  --note-anchors "軟體工程師,帶過小團隊"

# 或讓你自己的 AI 填好整份讀者側寫（docs/READER-PROFILE.md）
translatus annotate book.epub --note-profile profile.json

# 翻譯＋眉批一次完成
translatus translate book.epub --to "繁體中文" --annotate --profile "..."

# 換版面重出成品：免費，零 LLM 呼叫
translatus translate book.epub --to "繁體中文" --cache-only --mode bilingual

# JSON 進、JSON 出、可續跑，給腳本與 agent
translatus --json translate book.epub --to "繁體中文"
```

任何事情被打斷了？同一條指令再跑一次。已快取的段落永不重複計費。

Provider：`--provider openai|ollama|mock` 搭配 `--model`。
`mock` 只給腳本與測試用：離線跑完整管線、完全免費，先驗證格式保真，
再花 token；互動介面只提供真實的模型來源。
人類模式開跑前會自動印一行成本估算；`translatus --help` 結尾附三種模型
來源的設定卡（含免 API key 的訂閱 sidecar）。

## 接上你的 agent（MCP）

`translatus mcp` 是 stdio [MCP](https://modelcontextprotocol.io) server，
提供 `estimate_book`、`translate_book`、`annotate_book` 三個工具，逐章回報
進度；工具結果只包含固定狀態與數值，不會把譯文或檔名帶進呼叫它的 agent
上下文。Claude Code 一行接上：

```bash
translatus mcp install     # 自動註冊到找得到的 agent（Claude Code、Codex）
```

註冊走的是各家自己的 CLI（`claude mcp add`、`codex mcp add`），不去改它們的設定檔。
第一次開啟互動介面時會主動問你一次，一個按鍵完成；安裝執行檔本身**不會**擅自寫進
別的程式的設定。要取消用 `translatus mcp uninstall`——它只會移除
`mcp install` 自己建立的註冊；要指定單一 client 用
`--client claude` / `--client codex`。`translatus doctor` 一次回報
安裝健康狀態（binary、sidecar、埠、各 client 註冊）。

或在任何 MCP client 的 JSON 設定：

```json
{
  "mcpServers": {
    "translatus": { "command": "translatus", "args": ["mcp"] }
  }
}
```

接著直接用白話請你的 agent 做事：

> 幫我估一下 ~/Books/kokoro.epub 翻成繁體中文在 gpt-4o-mini 上要多少錢，
> 可以的話直接翻，順便為「第一次讀明治時期小說的讀者」寫眉批，
> 翻完告訴我實際花費。

整本書的呼叫會跑上數分鐘；請把 client 的工具逾時調大
（細節與各工具參數見[使用指南](docs/GUIDE.md#use-with-your-agent-mcp)）。

## 文件

- **[使用指南（英文）](docs/GUIDE.md)**：安裝、三種模型來源設定、所有參數
  範例、快取與續跑行為、JSON 模式、MCP server、疑難排解。
- **[FAQ（英文）](docs/FAQ.md)**：成本、訂閱合規、引擎為何免費、資料去向、
  與先行專案的差異。
- **[眉批調校地圖（英文）](docs/ANNOTATION-TUNING.md)**：給要改眉批品質的貢獻者。
- **[CONTRIBUTING.md](CONTRIBUTING.md)** · **[SECURITY.md](SECURITY.md)** ·
  **[CHANGELOG.md](CHANGELOG.md)**

## 眉批不做的事

眉批遵守寫進引擎 prompt 的硬規則，不是靠自律：

- 只做中性背景：史實、脈絡、術語由來、文本結構。
- 不對讀者喊話，不宣稱某段「對你的意義」。
- 不寫書評（「這是全書最著名的一段⋯⋯」）。
- 稀疏是架構保證：引擎逐章選點、受硬上限約束，大多數段落安靜留白。

你的讀者背景決定它**在哪裡停留**、**用什麼角度**；想通的部分留給你。

## 保真與隱私設計

- 原文在輸出中逐 byte 保留；譯文與眉批是分離、有標示的區塊。
- 一切在本機執行：解析、快取、重組。書的內容只會分批送往你自己設定的模型端點。
- 沒有遙測、不用帳號。API key 進系統鑰匙圈，不落檔案。

## 從原始碼建置

```bash
cargo build && cargo test          # 引擎 + CLI
```

選用的訂閱 sidecar（`apps/subscription-kit`，Node ≥ 20）把你本機的
Codex/ChatGPT 或 Claude Code 登入包成 OpenAI 相容端點：

```bash
cd apps/subscription-kit && npm install && npm run smoke
npm start  # 顯示 client 連線時必填的當次本機 access token
```

如果選擇 Codex 訂閱模式，請先確認書籍來源可信、內容沒有惡意指令；Codex 的
本機資料存取範圍由 Codex 本身控制。若不確定，請改用 API key 或 Ollama。
Claude 訂閱使用者也應閱讀 sidecar 顯示的政策提醒。

## 架構

```
crates/core/   翻譯＋眉批引擎（無 UI 概念）
  format/      byte-faithful XHTML mini-DOM · EPUB · TXT · 佔位符
  translate/   一般＋專家管線 · prompts · 詞彙表強制
  annotate/    章級選點 · 眉批生成 · 全書統一校閱
  job.rs       SQLite 快取＋斷點（續跑不重計費）
apps/cli/      薄殼：JSON I/O、冪等、MCP server
apps/subscription-kit/   選用的本機訂閱 sidecar
```

## 支持這個專案

Translatus 的引擎與 CLI 免費，也會一直免費。三種幫忙的方式：

- 給顆星、回報問題、送個 PR，任何尺寸都算數。
- 贊助：[Ko-fi](https://ko-fi.com/dokasoftware) <!-- SPONSOR-LINKS -->

## 先行者與致謝

Translatus 站在幾個先行專案驗證過的想法上。完整版見
[docs/ACKNOWLEDGMENTS.md](docs/ACKNOWLEDGMENTS.md)；短版：

- [bilingual_book_maker](https://github.com/yihong0618/bilingual_book_maker)（MIT）
  開創了這個品類：一行指令、自帶 API key 產出整本雙語 EPUB；Translatus 的差異在
  byte-faithful XHTML 處理與可續跑的內容定址快取。
- [Ebook-Translator Calibre plugin](https://github.com/bookfere/Ebook-Translator-Calibre-Plugin)（GPLv3）
  展示了擺位模式與快取離線重組能走多遠；我們只做行為觀察，全部以 Rust
  clean-room 重寫，沒有複製任何程式碼，完全尊重 GPLv3。
- [translation-agent](https://github.com/andrewyng/translation-agent)（Andrew Ng）
  示範了初譯 → reflection → 改寫的迴圈，形塑了專家模式的 source-aware reflection pass。
- [DelTA](https://arxiv.org/abs/2410.08143)（ICLR 2025）論證了文件級翻譯應是
  結構化多層記憶、而不是更大的 context window，是我們全書一致性 pass 的藍圖。

## 授權

引擎與 CLI：[MIT](LICENSE) © doka.software 與 Translatus 貢獻者。
