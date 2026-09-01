# 讀者側寫契約（Reader Profile）

註解的個人化來自「讀者側寫」：一份 JSON 文件，說明你為何讀這本書、你已經懂什麼、想要什麼樣的幫助。你可以自己填，也可以把本頁最下方的標準 prompt 丟給你自己的 AI（ChatGPT、Claude、Gemini⋯），讓它根據對你的了解代填。

側寫只決定「在哪裡停留、選什麼角度、怎麼搭橋」。**註解內文永遠不會描述、稱呼或評判你**：這是引擎鎖定的硬規則，且由程式端逐則檢查，不是模型自律。

## Schema

```json
{
  "presets": ["terms", "world", "methods"],
  "purpose": "為什麼讀這本書（自由文字，等同 --profile；有勾任一服務時可省略）",
  "anchors": ["軟體工程師", "讀過《國富論》", "熟悉日本戰國史"],
  "voice": "study",
  "level": "general",
  "lang": "繁體中文",
  "density": "medium",
  "style": "（可選）整段自訂的註解風格說明"
}
```

所有欄位皆可省略；出現未知欄位會直接報錯（避免打錯字被靜默忽略）。各欄位：

| 欄位 | 意義 | 值 |
|---|---|---|
| `presets` | 服務選單（用選的，比用寫的準）：你要註解為你做什麼。至少勾一項時 `purpose` 可省略 | `terms` 術語與專名 / `history` 時代背景 / `author` 作者處境 / `culture` 典故 / `characters` 人物關係 / `concepts` 概念白話拆解 / `world` 世界連結（把書接到真實世界的後世發展與同類事件） / `methods` 拆方法與原則（含適用條件與代價） / `research` 研究輔助（可引用事實、出處、結構、爭議點） |
| `purpose` | 用你自己的話補充（選填；服務選單講不清楚時才需要） | 自由文字 |
| `level` | 講給誰聽：註解假設你有多少基礎 | `beginner` 入門白話（日常語言＋例子講到懂） / `general` 一般（預設） / `insider` 內行（只補查不到的） |
| `anchors` | 認知錨：你已熟悉的職業、領域、讀過的書、生活經驗。註解解釋新概念時會優先從這些熟悉領域搭橋（類比、對照），讓你用既有認知理解新內容。最多 16 條、每條 80 字內 | 字串陣列 |
| `voice` | 聲線：`study`＝克制書齋（預設）；`companion`＝口語（口吻更接近日常說話、短反應批比例更高） | `study` \| `companion` |
| `lang` | 註解語言 | 語言名 |
| `density` | 密度 | `sparse` \| `medium` \| `rich` |
| `style` | 整段自訂風格（覆蓋 voice 的預設段落；硬規則不可覆蓋） | 自由文字 |

## 使用方式

```bash
# CLI：檔案路徑或 inline JSON 皆可
translatus annotate book.epub --note-profile profile.json --provider ollama --model qwen2.5
translatus translate book.epub --to 繁體中文 --annotate --note-profile '{"purpose":"想拆管理方法論","anchors":["新創營運"]}'

# 指令參數永遠優先於側寫文件
translatus annotate book.epub --note-profile profile.json --density rich
```

經 MCP（`translatus mcp`）呼叫 `annotate_book` / `translate_book` 時，`note_profile` 只接受 inline JSON（`{` 開頭）：檔案路徑形式等同任意讀檔，會被拒絕。

側寫視為不可信輸入：anchors 有數量與長度上限、style 有長度上限、未知 preset id 忽略並警告。

## 給你的 AI 的標準 prompt

把下面整段貼給任何了解你的 AI 助手，把它的輸出存成 `profile.json`（或直接貼進 `--note-profile`）：

```text
我要用一個「個人化書籍註解」工具讀一本書：《（書名）》。
請根據你對我的了解，輸出一份 JSON（只輸出 JSON，不要其他文字），schema 如下：

{
  "presets": ["從 terms(術語)/history(時代背景)/author(作者處境)/culture(典故)/characters(人物關係)/concepts(概念白話拆解)/world(連到真實世界)/methods(拆方法與原則)/research(研究輔助) 挑 1~3 個我最需要的服務"],
  "purpose": "一段話：我讀這本書最可能想搞懂什麼（具體，不要泛泛；服務選單講不清楚的才寫）",
  "anchors": ["3~8 條我已熟悉的領域/職業/讀過的書/生活經驗，短標籤，之後註解會用它們當類比的起點"],
  "level": "beginner（入門白話）/ general（一般）/ insider（內行）：依我對這本書領域的基礎選",
  "voice": "study（克制書齋）或 companion（口語）：依我平常偏好的閱讀口吻選"
}

規則：
- anchors 只列你有把握我真的熟的東西：錯的錨會產生錯的類比，寧缺勿濫。
- purpose 針對這本書寫，不是我的通用簡介。
- 不確定的欄位就省略。
```

## 隱私

側寫只在本機使用、只進入註解生成的 prompt；引擎無遙測（見 SECURITY.md 的自驗步驟）。要讓你的 AI 代填時，是你把資訊交給你自己的 AI，而不是交給我們。
