//! Interactive-session localisation. Five languages, mirroring the canonical
//! terminology researched for the GUI surface (margin notes = 眉批/批注/
//! AI注釈/AI 주석; the default translation depth is named Standard/一般/標準/
//! 표준 — never "fast").
//!
//! Scope: the interactive screens (menus, forms, notices, confirm gate).
//! `--help`, `--json` events and log-style progress lines stay English — they
//! are contracts for scripts and agents, not reading surfaces.
//!
//! Detection: `TRANSLATUS_LANG` overrides, then `LC_ALL` / `LC_MESSAGES` /
//! `LANG`. Unknown locales fall back to English. Tests pin English via
//! [`force`], so label-anchored tests never depend on the host locale.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhTw,
    ZhCn,
    Ja,
    Ko,
}

static FORCED: OnceLock<Lang> = OnceLock::new();
static DETECTED: OnceLock<Lang> = OnceLock::new();

/// Pin the language (first caller wins — used by tests and `TRANSLATUS_LANG`).
#[cfg_attr(not(test), allow(dead_code))] // test-only: pins the language under parallel tests
pub fn force(lang: Lang) {
    let _ = FORCED.set(lang);
}

fn parse_locale(raw: &str) -> Option<Lang> {
    let v = raw.trim().to_ascii_lowercase();
    if v.is_empty() || v == "c" || v == "posix" {
        return None;
    }
    let v = v.replace('-', "_");
    Some(if v.starts_with("zh") {
        if v.contains("tw") || v.contains("hk") || v.contains("mo") || v.contains("hant") {
            Lang::ZhTw
        } else {
            Lang::ZhCn
        }
    } else if v.starts_with("ja") {
        Lang::Ja
    } else if v.starts_with("ko") {
        Lang::Ko
    } else {
        Lang::En
    })
}

pub fn lang() -> Lang {
    if let Some(l) = FORCED.get() {
        return *l;
    }
    *DETECTED.get_or_init(|| {
        for var in ["TRANSLATUS_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(v) = std::env::var(var) {
                if let Some(l) = parse_locale(&v) {
                    return l;
                }
            }
        }
        Lang::En
    })
}

/// The translation for `key` in the active language.
pub fn tr(key: &str) -> &'static str {
    let row = t(key);
    match lang() {
        Lang::En => row[0],
        Lang::ZhTw => row[1],
        Lang::ZhCn => row[2],
        Lang::Ja => row[3],
        Lang::Ko => row[4],
    }
}

/// `tr` with a single `{}` placeholder substituted.
pub fn tr1(key: &str, value: &str) -> String {
    tr(key).replacen("{}", value, 1)
}

/// Localised "<n> <unit>" counts. CJK languages have no plural forms.
pub fn trn(unit: &str, n: usize) -> String {
    let row = t(unit);
    let pat = match lang() {
        Lang::En => {
            if n == 1 {
                row[0]
            } else {
                // English plural pattern is stored after a `|`.
                row[0].split_once('|').map(|(_, p)| p).unwrap_or(row[0])
            }
        }
        Lang::ZhTw => row[1],
        Lang::ZhCn => row[2],
        Lang::Ja => row[3],
        Lang::Ko => row[4],
    };
    let pat = if lang() == Lang::En {
        pat.split_once('|').map(|(s, _)| s).unwrap_or(pat)
    } else {
        pat
    };
    pat.replacen("{}", &n.to_string(), 1)
}

/// [en, zh-Hant, zh-Hans, ja, ko]. Unknown keys fall back to the key itself
/// in debug builds loudly, release quietly.
fn t(key: &str) -> [&'static str; 5] {
    match key {
        // ── counts ──────────────────────────────────────────────────────
        "n.chapter" => ["{} chapter|{} chapters", "{} 章", "{} 章", "{} 章", "{}개 장"],
        "n.book" => ["{} book|{} books", "{} 本書", "{} 本书", "{} 冊", "책 {}권"],
        "n.segment" => ["{} segment|{} segments", "{} 段", "{} 段", "{} 段落", "{}개 문단"],
        // ── main menu ───────────────────────────────────────────────────
        "menu.title" => ["What would you like to do?", "想做什麼？", "想做什么？", "何をしますか？", "무엇을 할까요?"],
        "menu.translate" => ["Translate", "翻譯", "翻译", "翻訳", "번역"],
        "menu.translate.sub" => ["Turn a book into your language", "把一本書翻成你的語言", "把一本书翻成你的语言", "本をあなたの言語に翻訳します", "책을 내 언어로 번역합니다"],
        "menu.annotate" => ["Annotate", "眉批", "批注", "AI注釈", "AI 주석"],
        "menu.annotate.sub" => ["Add margin notes, keeping the original text", "保留原文，只加眉批", "保留原文，只加批注", "原文はそのまま、AI注釈だけを追加", "원문은 그대로 두고 AI 주석만 추가"],
        "menu.estimate" => ["Estimate", "估價", "估价", "見積もり", "견적"],
        "menu.estimate.sub" => ["Price a run before you commit to it", "開跑前先算要花多少", "开跑前先算要花多少", "実行前に費用を見積もります", "실행 전에 비용을 계산합니다"],
        "menu.settings" => ["Settings", "設定", "设置", "設定", "설정"],
        "menu.settings.sub" => ["Model source, API key, and defaults", "模型來源、API 金鑰與預設值", "模型来源、API 密钥与默认值", "モデルのソース、APIキー、既定値", "모델 소스, API 키, 기본값"],
        // ── book list ───────────────────────────────────────────────────
        "list.title" => ["Choose a book", "選一本書", "选一本书", "本を選ぶ", "책 선택"],
        "list.empty" => ["nothing here", "這裡沒有東西", "这里没有东西", "何もありません", "아무것도 없습니다"],
        "list.elsewhere" => ["Somewhere else…", "其他位置…", "其他位置…", "別の場所…", "다른 위치…"],
        "list.elsewhere.sub" => ["type a path to a book or folder", "輸入書檔或資料夾路徑", "输入书档或文件夹路径", "本またはフォルダのパスを入力", "책 파일 또는 폴더 경로 입력"],
        // ── empty discovery / path form ─────────────────────────────────
        "nobooks.title" => ["No books found yet", "還沒找到書", "还没找到书", "本がまだ見つかりません", "아직 책을 찾지 못했습니다"],
        "nobooks.l1" => ["  The book list scans the folder you launch translatus from —", "  書單掃描的是你啟動 translatus 的資料夾：", "  书单扫描的是你启动 translatus 的文件夹：", "  ブック一覧は translatus を起動したフォルダを走査します。", "  책 목록은 translatus를 실행한 폴더를 검색합니다."],
        "nobooks.l2" => ["  right now that is {} (and one level below).", "  目前是 {}（含往下一層）。", "  目前是 {}（含往下一层）。", "  現在は {}（1 階層下まで）です。", "  현재 위치: {} (한 단계 아래 포함)."],
        "nobooks.l3" => ["  Next: type a path to a book or a folder of books,", "  下一步：輸入書檔或書資料夾的路徑，", "  下一步：输入书档或书文件夹的路径，", "  次に、本またはフォルダのパスを入力するか、", "  다음: 책 파일 또는 폴더 경로를 입력하거나,"],
        "nobooks.l4" => ["  or set a permanent Books folder under Settings.", "  或在「設定」裡設定固定的書籍資料夾。", "  或在「设置」里设置固定的书籍文件夹。", "  設定で常用のブックフォルダを指定してください。", "  설정에서 기본 책 폴더를 지정하세요."],
        "path.title" => ["Open a book", "開啟書籍", "打开书籍", "本を開く", "책 열기"],
        "path.tip" => ["Tip: set a Books folder in Settings so it is always scanned.", "提示：在「設定」設好書籍資料夾，之後會自動掃描。", "提示：在「设置」设好书籍文件夹，之后会自动扫描。", "ヒント：設定でブックフォルダを指定すると常に走査されます。", "팁: 설정에서 책 폴더를 지정하면 항상 검색됩니다."],
        "path.field" => ["Path", "路徑", "路径", "パス", "경로"],
        "path.placeholder" => ["a book file, or a folder of books — e.g. ~/Books", "書檔或書資料夾，例如 ~/Books", "书档或书文件夹，例如 ~/Books", "本のファイル、またはフォルダ（例：~/Books）", "책 파일 또는 폴더 (예: ~/Books)"],
        "path.help" => ["Paste or type a path to an .epub / .txt, or to a folder to browse.", "貼上或輸入 .epub／.txt 檔案路徑，或輸入資料夾路徑來瀏覽。", "粘贴或输入 .epub／.txt 文件路径，或输入文件夹路径来浏览。", ".epub / .txt のパス、または一覧表示するフォルダのパスを入力。", ".epub / .txt 파일 경로 또는 탐색할 폴더 경로를 입력하세요."],
        "path.open" => ["Open this path", "開啟這個路徑", "打开这个路径", "このパスを開く", "이 경로 열기"],
        "path.bad.title" => ["Not a book or a folder", "不是書檔也不是資料夾", "不是书档也不是文件夹", "本でもフォルダでもありません", "책도 폴더도 아닙니다"],
        "path.bad.onlyepub" => ["  Only .epub and .txt files can be opened.", "  只能開啟 .epub 與 .txt 檔。", "  只能打开 .epub 与 .txt 文件。", "  開けるのは .epub と .txt のみです。", "  .epub과 .txt 파일만 열 수 있습니다."],
        "path.bad.missing" => ["  Nothing exists at that path — check for typos.", "  這個路徑不存在，檢查一下有沒有打錯。", "  这个路径不存在，检查一下有没有打错。", "  そのパスは存在しません。入力ミスをご確認ください。", "  해당 경로가 없습니다. 오타를 확인하세요."],
        "folder.empty.title" => ["No books in that folder", "這個資料夾裡沒有書", "这个文件夹里没有书", "そのフォルダに本がありません", "폴더에 책이 없습니다"],
        "folder.empty.l1" => ["  {} has no .epub or .txt (one level deep).", "  {} 裡沒有 .epub 或 .txt（含往下一層）。", "  {} 里没有 .epub 或 .txt（含往下一层）。", "  {} に .epub / .txt がありません（1 階層下まで）。", "  {} 에 .epub / .txt가 없습니다 (한 단계 아래 포함)."],
        // ── config form ─────────────────────────────────────────────────
        "cfg.title" => ["How should it read?", "這本書要怎麼讀？", "这本书要怎么读？", "どう読みますか？", "이 책을 어떻게 읽을까요?"],
        "cfg.h.translation" => ["Translation", "翻譯", "翻译", "翻訳", "번역"],
        "cfg.h.notes" => ["Margin notes", "眉批", "批注", "AI注釈", "AI 주석"],
        "cfg.h.model" => ["Model", "模型", "模型", "モデル", "모델"],
        "cfg.h.output" => ["Output", "輸出", "输出", "出力", "출력"],
        "cfg.translate" => ["Translate", "翻譯", "翻译", "翻訳する", "번역"],
        "cfg.translate.help" => ["Off, with notes on, keeps the original text and only adds margin notes.", "關閉翻譯、開著眉批＝保留原文、只加眉批。", "关闭翻译、开着批注＝保留原文、只加批注。", "オフにして注釈のみオンにすると、原文のまま注釈だけ追加します。", "번역을 끄고 주석만 켜면 원문은 그대로 두고 주석만 추가합니다."],
        "cfg.into" => ["Into", "翻成", "翻成", "翻訳先", "번역 언어"],
        "cfg.into.help" => ["The language the finished book will be written in.", "成品要用什麼語言。", "成品要用什么语言。", "完成した本の言語です。", "완성본의 언어입니다."],
        "cfg.depth" => ["Depth", "深度", "深度", "翻訳の深さ", "번역 깊이"],
        "cfg.depth.standard" => ["standard", "一般", "一般", "標準", "표준"],
        "cfg.depth.expert" => ["expert", "專家", "专家", "エキスパート", "전문가"],
        "cfg.depth.help" => ["Expert reads the whole book first to lock terminology, then reviews its own draft. Slower and dearer.", "專家模式先讀完全書鎖定術語，再校訂自己的草稿。較慢也較貴。", "专家模式先读完全书锁定术语，再校订自己的草稿。较慢也较贵。", "エキスパートは全体を先に読んで用語を固定し、草稿を校訂します。遅く、費用も高めです。", "전문가 모드는 책 전체를 먼저 읽어 용어를 고정한 뒤 초안을 교정합니다. 더 느리고 비쌉니다."],
        "cfg.layout" => ["Layout", "版面", "版面", "レイアウト", "레이아웃"],
        "cfg.layout.only" => ["translation only", "純譯文", "纯译文", "訳文のみ", "번역문만"],
        "cfg.layout.side" => ["side by side", "雙語對照", "双语对照", "対訳", "대역"],
        "cfg.layout.help" => ["Side by side keeps each source paragraph next to its translation.", "雙語對照把每段原文與譯文放在一起。", "双语对照把每段原文与译文放在一起。", "対訳は原文と訳文を段落ごとに並べます。", "대역은 원문과 번역문을 문단별로 나란히 둡니다."],
        "cfg.notes" => ["Add notes", "加眉批", "加批注", "AI注釈を付ける", "AI 주석 추가"],
        "cfg.notes.help" => ["Background notes written for you specifically, placed where you are likely to want them.", "專為你寫的背景眉批，放在你最可能需要的地方。", "专为你写的背景批注，放在你最可能需要的地方。", "あなたのために書かれた背景注釈を、必要になりそうな箇所に置きます。", "나를 위해 쓰인 배경 주석을 필요할 만한 곳에 배치합니다."],
        "cfg.services" => ["Help me with", "幫我補", "帮我补", "補ってほしいもの", "도움 받을 항목"],
        "cfg.services.placeholder" => ["pick one or more — easier than writing it", "勾一個以上就行，比用寫的簡單", "勾一个以上就行，比用写的简单", "1 つ以上選ぶだけ。書くより簡単です", "하나 이상 선택 — 쓰는 것보다 쉽습니다"],
        "cfg.services.help" => ["What the notes should do for you. Picking is enough; the box below is optional precision.", "眉批要為你做什麼。用勾的就夠；下面的欄位是選填的補充。", "批注要为你做什么。用勾的就够；下面的栏位是选填的补充。", "注釈に何をしてほしいか。選ぶだけで十分。下の欄は任意の補足です。", "주석이 무엇을 해 줄지 선택하세요. 선택만으로 충분하며 아래 칸은 선택 사항입니다."],
        "cfg.why" => ["Why this book", "為什麼讀這本", "为什么读这本", "この本を読む理由", "이 책을 읽는 이유"],
        "cfg.why.placeholder" => ["optional — who you are, what you hope to get", "選填：你是誰、想從書裡拿到什麼", "选填：你是谁、想从书里拿到什么", "任意：あなたは誰で、何を得たいか", "선택 — 나는 누구이고 무엇을 얻고 싶은지"],
        "cfg.why.help" => ["Optional. Anything the picks above cannot say, in your own words: who you are, why this book.", "選填。上面勾不出來的，用你自己的話說：你是誰、為什麼讀這本。", "选填。上面勾不出来的，用你自己的话说：你是谁、为什么读这本。", "任意。上の選択で言えないことを自分の言葉で：あなたは誰か、なぜこの本か。", "선택. 위 선택으로 담기 어려운 것을 내 말로: 나는 누구이고 왜 이 책인지."],
        "cfg.anchors" => ["You already know", "你已經懂的", "你已经懂的", "すでに詳しいこと", "이미 잘 아는 것"],
        "cfg.anchors.placeholder" => ["optional — comma-separated: software, running a team", "選填：逗號分隔：軟體、帶過團隊", "选填：逗号分隔：软件、带过团队", "任意：カンマ区切り（例：ソフトウェア、チーム運営）", "선택 — 쉼표로 구분: 소프트웨어, 팀 운영"],
        "cfg.anchors.help" => ["Things you know well, comma-separated. Notes explain new ideas by bridging from these.", "你熟悉的領域，逗號分隔。眉批會從這些地方搭橋解釋新概念。", "你熟悉的领域，逗号分隔。批注会从这些地方搭桥解释新概念。", "詳しい分野をカンマ区切りで。注釈はそこから橋を架けて新しい概念を説明します。", "잘 아는 분야를 쉼표로 구분해 적으세요. 주석이 여기서 다리를 놓아 새 개념을 설명합니다."],
        "cfg.level" => ["Explain for", "講給誰聽", "讲给谁听", "説明レベル", "설명 수준"],
        "cfg.level.beginner" => ["beginner (plain)", "入門白話", "入门白话", "入門（平易）", "입문 (쉽게)"],
        "cfg.level.general" => ["general", "一般", "一般", "標準", "표준"],
        "cfg.level.insider" => ["insider (expert)", "內行", "内行", "上級者", "전문가"],
        "cfg.level.help" => ["beginner = everyday words and examples · general = normal · insider = only what you could not look up.", "入門＝日常語言＋例子講到懂 · 一般＝正常 · 內行＝只補查不到的。", "入门＝日常语言＋例子讲到懂 · 一般＝正常 · 内行＝只补查不到的。", "入門＝日常語と例で丁寧に · 標準＝ふつう · 上級者＝調べても出ないことだけ。", "입문 = 일상어와 예시로 쉽게 · 표준 = 보통 · 전문가 = 찾기 어려운 것만."],
        "cfg.voice" => ["Voice", "聲線", "声线", "語り口", "말투"],
        "cfg.voice.study" => ["study (quiet)", "書齋（克制）", "书斋（克制）", "書斎（控えめ）", "서재 (절제)"],
        "cfg.voice.companion" => ["companion (friendly)", "陪讀（親近）", "陪读（亲近）", "伴読（親しみ）", "동반 (친근)"],
        "cfg.voice.help" => ["study = quiet scholar in the margin · companion = friendly co-reader, more reactions.", "書齋＝頁邊安靜的學者 · 陪讀＝一起讀的朋友，反應更多。", "书斋＝页边安静的学者 · 陪读＝一起读的朋友，反应更多。", "書斎＝余白の静かな学者 · 伴読＝一緒に読む友人、リアクション多め。", "서재 = 여백의 조용한 학자 · 동반 = 함께 읽는 친구, 반응이 더 많음."],
        "cfg.density" => ["Density", "密度", "密度", "注釈の密度", "주석 밀도"],
        "cfg.density.sparse" => ["sparse", "精", "精", "控えめ", "간결"],
        "cfg.density.medium" => ["balanced", "適中", "适中", "ふつう", "보통"],
        "cfg.density.rich" => ["rich", "豐", "丰", "豊富", "풍부"],
        "cfg.density.help" => ["How often notes appear.", "眉批出現的頻率。", "批注出现的频率。", "注釈の頻度です。", "주석이 나타나는 빈도입니다."],
        "cfg.notelang" => ["Notes language", "眉批語言", "批注语言", "注釈の言語", "주석 언어"],
        "cfg.notelang.auto" => ["auto", "自動", "自动", "自動", "자동"],
        "cfg.notelang.help" => ["auto: follows the translation target; without translation, matches the language you write in.", "自動：跟隨翻譯目標語言；不翻譯時，跟隨你書寫的語言。", "自动：跟随翻译目标语言；不翻译时，跟随你书写的语言。", "自動：翻訳先の言語に従います。翻訳しない場合はあなたが書いた言語に合わせます。", "자동: 번역 대상 언어를 따르고, 번역하지 않으면 내가 쓴 언어를 따릅니다."],
        "cfg.source" => ["Source", "來源", "来源", "ソース", "소스"],
        "cfg.model" => ["Model", "模型", "模型", "モデル", "모델"],
        "cfg.saveto" => ["Save to", "存到", "存到", "保存先", "저장 위치"],
        "cfg.saveto.help" => ["Leave blank to save next to the book. Type a path to put it elsewhere.", "留空＝存在書旁邊；輸入路徑可存到別處。", "留空＝存在书旁边；输入路径可存到别处。", "空欄なら本と同じ場所に保存。パスを入力すると別の場所へ。", "비워 두면 책 옆에 저장. 경로를 입력하면 다른 곳에 저장합니다."],
        "cfg.continue" => ["Continue", "繼續", "继续", "続ける", "계속"],
        // sources
        "src.subscription" => ["subscription", "訂閱", "订阅", "サブスク", "구독"],
        "src.subscription.sub" => ["your Codex / Claude plan", "你的 Codex / Claude 方案", "你的 Codex / Claude 方案", "あなたの Codex / Claude プラン", "내 Codex / Claude 요금제"],
        "src.subscription.cost" => ["no API key; needs the local sidecar running", "免 API 金鑰；需先啟動本機 sidecar", "免 API 密钥；需先启动本机 sidecar", "APIキー不要。ローカル sidecar の起動が必要", "API 키 불필요, 로컬 sidecar 실행 필요"],
        "src.apikey" => ["api key", "API 金鑰", "API 密钥", "APIキー", "API 키"],
        "src.apikey.sub" => ["billed per use", "按用量計費", "按用量计费", "従量課金", "사용량 과금"],
        "src.apikey.cost" => ["an OpenAI-compatible key; a book is usually well under a few dollars", "任何 OpenAI 相容金鑰；一本書通常遠低於幾美元", "任何 OpenAI 兼容密钥；一本书通常远低于几美元", "OpenAI 互換キー。1 冊は通常数ドル未満", "OpenAI 호환 키, 책 한 권은 보통 몇 달러 미만"],
        "src.ollama" => ["ollama", "Ollama", "Ollama", "Ollama", "Ollama"],
        "src.ollama.sub" => ["local model", "本機模型", "本机模型", "ローカルモデル", "로컬 모델"],
        "src.ollama.cost" => ["free and offline; speed depends on your machine", "免費、離線；速度看你的機器", "免费、离线；速度看你的机器", "無料・オフライン。速度はマシン次第", "무료·오프라인, 속도는 기기에 따라 다름"],
        // services (nine)
        "svc.terms" => ["unfamiliar terms and jargon", "術語與專有名詞", "术语与专有名词", "用語・専門語", "용어와 전문어"],
        "svc.history" => ["historical context", "時代背景", "时代背景", "時代背景", "시대 배경"],
        "svc.author" => ["the author's circumstances", "作者的處境", "作者的处境", "著者の事情", "저자의 상황"],
        "svc.culture" => ["cultural references", "文化典故", "文化典故", "文化的な言及", "문화적 인용"],
        "svc.characters" => ["who's who", "人物與關係", "人物与关系", "人物相関", "인물 관계"],
        "svc.concepts" => ["the ideas behind the text", "概念白話拆解", "概念白话拆解", "概念のかみくだき", "개념 풀이"],
        "svc.world" => ["real-world examples and echoes", "連到真實世界", "连到真实世界", "現実世界とのつながり", "현실 세계와의 연결"],
        "svc.methods" => ["the methods, with their limits", "拆方法與邊界", "拆方法与边界", "手法とその限界", "방법론과 한계"],
        "svc.research" => ["citable facts and structure", "可引用的事實與結構", "可引用的事实与结构", "引用できる事実と構成", "인용 가능한 사실과 구조"],
        // notices in config
        "cfg.nothing.title" => ["Nothing to do", "沒有事情可做", "没有事情可做", "実行する処理がありません", "실행할 작업이 없습니다"],
        "cfg.nothing.l1" => ["  Both services are switched off, so there is no run to make.", "  翻譯與眉批都關著，沒有可以執行的工作。", "  翻译与批注都关着，没有可以执行的工作。", "  翻訳もAI注釈もオフのため、実行するものがありません。", "  번역과 AI 주석이 모두 꺼져 있어 실행할 작업이 없습니다."],
        "cfg.nothing.l2" => ["  Switch on Translate, Add notes, or both.", "  打開「翻譯」或「加眉批」，或兩個都開。", "  打开「翻译」或「加批注」，或两个都开。", "  「翻訳する」か「AI注釈を付ける」をオンにしてください。", "  '번역' 또는 'AI 주석 추가'를 켜 주세요."],
        "cfg.who.title" => ["Notes need to know who they are for", "眉批需要知道是寫給誰的", "批注需要知道是写给谁的", "AI注釈には読み手の手がかりが必要です", "AI 주석에는 독자에 대한 단서가 필요합니다"],
        "cfg.who.l1" => ["  Margin notes are written for a specific reader — that is the", "  眉批是為特定讀者而寫的，這正是它的", "  批注是为特定读者而写的，这正是它的", "  AI注釈は特定の読み手のために書かれます。", "  AI 주석은 특정 독자를 위해 쓰입니다."],
        "cfg.who.l2" => ["  whole point of them. Without a reason to pause, they collapse", "  意義所在。沒有停留的理由，它就退化成", "  意义所在。没有停留的理由，它就退化成", "  立ち止まる理由がなければ、ただの汎用", "  멈출 이유가 없으면 그저 범용"],
        "cfg.who.l3" => ["  into a generic study guide.", "  一份通用的導讀講義。", "  一份通用的导读讲义。", "  ガイドになってしまいます。", "  해설 자료가 되어 버립니다."],
        "cfg.who.l4" => ["  Pick something under \"Help me with\" (easiest), write a line in", "  在「幫我補」勾一項（最簡單），或在「為什麼讀這本」", "  在「帮我补」勾一项（最简单），或在「为什么读这本」", "  「補ってほしいもの」から選ぶ（最も簡単）か、", "  '도움 받을 항목'에서 선택하거나(가장 쉬움),"],
        "cfg.who.l5" => ["  \"Why this book\", or switch notes off.", "  寫一句話，或把眉批關掉。", "  写一句话，或把批注关掉。", "  「この本を読む理由」に一行書くか、注釈をオフに。", "  '이 책을 읽는 이유'에 한 줄 쓰거나 주석을 끄세요."],
        "cfg.codex.title" => ["Before using Codex", "使用 Codex 之前", "使用 Codex 之前", "Codex を使う前に", "Codex 사용 전"],
        "cfg.codex.l1" => ["  Make sure you trust this book's source and that its content", "  請先確認你信任這本書的來源，內容不含", "  请先确认你信任这本书的来源，内容不含", "  この本の入手元を信頼でき、内容に悪意ある指示が", "  이 책의 출처를 신뢰할 수 있고 내용에 악성 지시가"],
        "cfg.codex.l2" => ["  contains no malicious instructions. Codex controls its own", "  惡意指令。Codex 的本機資料存取由它自己控制。", "  恶意指令。Codex 的本机数据访问由它自己控制。", "  含まれないことを確認してください。ローカルデータへの", "  없는지 확인하세요. 로컬 데이터 접근은 Codex 자체가"],
        "cfg.codex.l3" => ["  local-data access. If you are unsure, use an API key or Ollama.", "  不確定的話，改用 API 金鑰或 Ollama。", "  不确定的话，改用 API 密钥或 Ollama。", "  アクセスは Codex 自身が制御します。不安なら APIキーか Ollama を。", "  제어합니다. 확실하지 않으면 API 키나 Ollama를 사용하세요."],
        // ── settings ────────────────────────────────────────────────────
        "set.title" => ["Settings", "設定", "设置", "設定", "설정"],
        "set.subtitle" => ["Saved locally — new books start from these defaults", "存在本機；新書會從這些預設值開始", "存在本机；新书会从这些默认值开始", "ローカルに保存。新しい本はこの既定値から始まります", "로컬에 저장 — 새 책은 이 기본값에서 시작합니다"],
        "set.h.source" => ["Where the model comes from", "模型從哪裡來", "模型从哪里来", "モデルの入手元", "모델 소스"],
        "set.h.newbooks" => ["What new books start with", "新書的起始設定", "新书的起始设置", "新しい本の初期設定", "새 책의 시작 설정"],
        "set.baseurl" => ["Base URL", "Base URL", "Base URL", "Base URL", "Base URL"],
        "set.baseurl.placeholder" => ["provider default", "供應商預設", "供应商默认", "プロバイダ既定", "제공자 기본값"],
        "set.baseurl.help" => ["Any OpenAI-compatible endpoint. Remote ones must use https.", "任何 OpenAI 相容端點；遠端必須用 https。", "任何 OpenAI 兼容端点；远端必须用 https。", "OpenAI 互換のエンドポイント。リモートは https 必須。", "OpenAI 호환 엔드포인트. 원격은 https 필수."],
        "set.key" => ["API key", "API 金鑰", "API 密钥", "APIキー", "API 키"],
        "set.key.help" => ["Kept in your OS keychain, never in a config file.", "存在系統鑰匙圈，不會寫進設定檔。", "存在系统钥匙串，不会写进配置文件。", "OS のキーチェーンに保存され、設定ファイルには書かれません。", "OS 키체인에 저장되며 설정 파일에는 기록되지 않습니다."],
        "set.key.saved" => ["saved", "已儲存", "已保存", "保存済み", "저장됨"],
        "set.key.notset" => ["not set", "未設定", "未设置", "未設定", "설정 안 됨"],
        "set.key.savedhint" => ["saved ({})", "已儲存（{}）", "已保存（{}）", "保存済み（{}）", "저장됨 ({})"],
        "set.test" => ["Test connection", "測試連線", "测试连接", "接続テスト", "연결 테스트"],
        "set.test.help" => ["Sends one small request and reports what came back.", "送出一個小請求，回報結果。", "送出一个小请求，回报结果。", "小さなリクエストを送って結果を表示します。", "작은 요청을 보내 결과를 보여 줍니다."],
        "set.forget" => ["Forget saved key", "刪除已存金鑰", "删除已存密钥", "保存済みキーを削除", "저장된 키 삭제"],
        "set.forget.help" => ["Deletes it from the keychain.", "從鑰匙圈刪除。", "从钥匙串删除。", "キーチェーンから削除します。", "키체인에서 삭제합니다."],
        "set.token" => ["Access token", "存取權杖", "访问令牌", "アクセストークン", "액세스 토큰"],
        "set.token.help" => ["The token the sidecar prints at startup. Kept in your OS keychain.", "sidecar 啟動時印出的權杖，存在系統鑰匙圈。", "sidecar 启动时打印的令牌，存在系统钥匙串。", "sidecar 起動時に表示されるトークン。OS のキーチェーンに保存されます。", "sidecar 시작 시 출력되는 토큰. OS 키체인에 저장됩니다."],
        "set.token.dialog.prompt" => ["Paste the access token the sidecar printed when it started.", "貼上 sidecar 啟動時印出的存取權杖。", "贴上 sidecar 启动时打印的访问令牌。", "sidecar 起動時に表示されたアクセストークンを貼り付けてください。", "sidecar 시작 시 출력된 액세스 토큰을 붙여 넣으세요."],
        "set.key.dialog.prompt" => ["Paste your key. It goes straight to the OS keychain.", "貼上你的金鑰，會直接存進系統鑰匙圈。", "贴上你的密钥，会直接存进系统钥匙串。", "キーを貼り付けてください。OS のキーチェーンに直接保存されます。", "키를 붙여 넣으세요. OS 키체인에 바로 저장됩니다."],
        "set.key.dialog.note" => ["Nothing is echoed, and it never reaches a config file.", "輸入不會顯示在畫面上，也不會寫進設定檔。", "输入不会显示在屏幕上，也不会写进配置文件。", "入力は画面に表示されず、設定ファイルに書かれることもありません。", "입력은 화면에 표시되지 않으며 설정 파일에 기록되지 않습니다."],
        "set.key.err.title" => ["Could not save the key", "無法儲存金鑰", "无法保存密钥", "キーを保存できませんでした", "키를 저장하지 못했습니다"],
        "set.save.err.title" => ["Could not save settings", "無法儲存設定", "无法保存设置", "設定を保存できませんでした", "설정을 저장하지 못했습니다"],
        "set.save.err.l1" => ["Your choices apply to this session only.", "這些選擇只在這次啟動內有效。", "这些选择只在这次启动内有效。", "この選択は今回の起動中のみ有効です。", "이 선택은 이번 실행에서만 적용됩니다."],
        "set.test.title" => ["Connection test", "連線測試", "连接测试", "接続テスト", "연결 테스트"],
        "set.forget.done.title" => ["Key removed", "金鑰已刪除", "密钥已删除", "キーを削除しました", "키를 삭제했습니다"],
        "set.forget.done.l1" => ["The saved key has been deleted from your keychain.", "已從系統鑰匙圈刪除儲存的金鑰。", "已从系统钥匙串删除保存的密钥。", "保存済みのキーをキーチェーンから削除しました。", "저장된 키를 키체인에서 삭제했습니다."],
        "test.endpoint" => ["Endpoint", "端點", "端点", "エンドポイント", "엔드포인트"],
        "test.model" => ["Model", "模型", "模型", "モデル", "모델"],
        "test.keyrow" => ["Key", "金鑰", "密钥", "キー", "키"],
        "test.default" => ["provider default", "供應商預設", "供应商默认", "プロバイダ既定", "제공자 기본값"],
        "test.present" => ["present", "已設定", "已设置", "設定あり", "설정됨"],
        "test.absent" => ["none", "未設定", "未设置", "なし", "없음"],
        "test.ok" => ["answered (tokens in/out: {})", "有回應（tokens 進/出：{}）", "有回应（tokens 进/出：{}）", "応答あり（トークン入/出: {}）", "응답 수신 (토큰 입/출: {})"],
        "test.reply" => ["reply: {}", "回覆：{}", "回复：{}", "返答: {}", "응답: {}"],
        "test.sidecar.hint" => ["If this is subscription mode, check the sidecar is running:", "若你用的是訂閱模式，請先確認 sidecar 已啟動：", "若你用的是订阅模式，请先确认 sidecar 已启动：", "サブスクモードの場合は sidecar が起動しているか確認してください:", "구독 모드라면 sidecar가 실행 중인지 확인하세요:"],
        // ── the live run board ──────────────────────────────────────────
        "run.left" => ["~{} left", "約剩 {}", "约剩 {}", "残り約 {}", "약 {} 남음"],
        "run.chapters" => ["chapters", "章", "章", "章", "챕터"],
        "run.chapter.title" => ["Chapter {}", "第 {} 章", "第 {} 章", "第{}章", "제{}장"],
        "run.stop" => ["  ^C  stop — progress is saved, re-running resumes for free", "  ^C 停止：進度已存檔，重跑免費續跑", "  ^C 停止：进度已存档，重跑免费续跑", "  ^C 停止: 進行状況は保存済み。再実行で無料再開", "  ^C 중지: 진행 상황은 저장됨, 재실행 시 무료로 이어짐"],
        "run.done.title" => ["Translation complete", "翻譯完成", "翻译完成", "翻訳完了", "번역 완료"],
        "run.gaps.title" => ["Translation finished with gaps", "翻譯完成（有缺段）", "翻译完成（有缺段）", "翻訳完了（未訳あり）", "번역 완료 (누락 있음)"],
        "run.facts" => ["{}, {} characters, {}", "{}、{} 字、{}", "{}、{} 字、{}", "{}・{} 文字・{}", "{} · {}자 · {}"],
        "run.saved" => ["Saved to {}", "已存到 {}", "已存到 {}", "保存先: {}", "저장 위치: {}"],
        "run.failed" => ["{} could not be translated — re-run to retry just those", "{} 沒翻出來：重跑只會重試這些", "{} 没翻出来：重跑只会重试这些", "{} は翻訳できませんでした。再実行でその分だけ再試行します", "{} 은(는) 번역되지 않았습니다. 재실행 시 해당 부분만 재시도합니다"],
        "run.resume" => ["  re-run the same command to resume — cached segments are never re-billed", "  重跑同一個指令即可續跑：已快取的段落不會重複計費", "  重跑同一个指令即可续跑：已缓存的段落不会重复计费", "  同じコマンドを再実行すると再開します。キャッシュ済みの部分に再課金はありません", "  같은 명령을 다시 실행하면 이어집니다. 캐시된 부분은 다시 과금되지 않습니다"],
        "set.into" => ["Translate into", "預設翻成", "默认翻成", "既定の翻訳先", "기본 번역 언어"],
        "set.booksdir" => ["Books folder", "書籍資料夾", "书籍文件夹", "ブックフォルダ", "책 폴더"],
        "set.booksdir.placeholder" => ["not set — the list scans where you launch translatus", "未設定：書單掃描你啟動 translatus 的位置", "未设置：书单扫描你启动 translatus 的位置", "未設定：一覧は起動した場所を走査します", "설정 안 됨 — 목록은 실행 위치를 검색합니다"],
        "set.booksdir.help" => ["A folder the book list always scans (one level deep), wherever you launch from.", "書單永遠會掃描的資料夾（含往下一層），不管你從哪裡啟動。", "书单永远会扫描的文件夹（含往下一层），不管你从哪里启动。", "起動場所に関係なく常に走査されるフォルダ（1 階層下まで）。", "실행 위치와 관계없이 항상 검색되는 폴더 (한 단계 아래 포함)."],
        "set.whyread" => ["Why I read", "我為什麼讀書", "我为什么读书", "読む理由", "읽는 이유"],
        "set.whyread.help" => ["Prefills new books. Each book can still say something different.", "新書的預填值；每本書仍可各自不同。", "新书的预填值；每本书仍可各自不同。", "新しい本の初期値。冊ごとに変えられます。", "새 책의 기본값. 책마다 다르게 쓸 수 있습니다."],
        "set.services.placeholder" => ["whatever the book calls for", "看書需要什麼", "看书需要什么", "本に合わせて", "책에 맞게"],
        "set.services.help" => ["Default note angles for new books.", "新書的預設眉批角度。", "新书的默认批注角度。", "新しい本の注釈の既定の観点。", "새 책의 기본 주석 관점."],
        "set.notedensity" => ["Note density", "眉批密度", "批注密度", "AI注釈の密度", "AI 주석 밀도"],
        "set.save" => ["Save", "儲存", "保存", "保存", "저장"],
        // ── legends & widget chrome ─────────────────────────────────────
        "leg.move" => ["move", "移動", "移动", "移動", "이동"],
        "leg.search" => ["search", "搜尋", "搜索", "検索", "검색"],
        "leg.open" => ["open", "開啟", "打开", "開く", "열기"],
        "leg.back" => ["back", "返回", "返回", "戻る", "뒤로"],
        "leg.select" => ["select", "選取", "选取", "選択", "선택"],
        "leg.field" => ["field", "欄位", "栏位", "項目", "항목"],
        "leg.toggle" => ["toggle", "切換", "切换", "切替", "전환"],
        "leg.edit" => ["edit", "編輯", "编辑", "編集", "편집"],
        "leg.change" => ["change", "調整", "调整", "変更", "변경"],
        "leg.choose" => ["choose", "選擇", "选择", "選ぶ", "선택"],
        "leg.run" => ["run", "執行", "执行", "実行", "실행"],
        "leg.onarrow" => ["on ➤ to continue", "在 ➤ 上繼續", "在 ➤ 上继续", "➤ で続行", "➤ 에서 계속"],
        "leg.continue" => ["continue", "繼續", "继续", "続行", "계속"],
        "leg.save" => ["save", "儲存", "保存", "保存", "저장"],
        "leg.cancel" => ["cancel", "取消", "取消", "キャンセル", "취소"],
        "leg.clear" => ["clear", "清空", "清空", "クリア", "지우기"],
        "leg.done" => ["done", "完成", "完成", "完了", "완료"],
        "multi.selected" => ["{} selected", "已選 {}", "已选 {}", "{} 件選択", "{}개 선택"],
        // ── confirm gate ────────────────────────────────────────────────
        "go.cost.title" => ["What this would cost", "這樣會花多少", "这样会花多少", "かかる費用", "예상 비용"],
        "go.est" => ["  Estimated cost  ~${}", "  預估費用  約 ${}", "  预估费用  约 ${}", "  推定費用  約 ${}", "  예상 비용  약 ${}"],
        "go.est.note" => ["An estimate, not a quote — the real figure depends on how the book splits.", "是估算不是報價；實際數字取決於書怎麼切分。", "是估算不是报价；实际数字取决于书怎么切分。", "見積もりであり確定額ではありません。実際は本の分割に依存します。", "견적일 뿐 확정 금액이 아닙니다. 실제 비용은 책 분할에 따라 달라집니다."],
        "go.noprice" => ["No published price for {} — cost will be reported after the run.", "{} 沒有公開價目；跑完後回報實際用量。", "{} 没有公开价目；跑完后回报实际用量。", "{} は公開価格がありません。実行後に実測を報告します。", "{} 는 공개 가격이 없습니다. 실행 후 실제 사용량을 보고합니다."],
        "go.stopsafe" => ["Stopping is safe. Progress is checkpointed and resuming is never re-billed.", "隨時可以停。進度有存檔，續跑不會重複計費。", "随时可以停。进度有存档，续跑不会重复计费。", "いつでも停止できます。進捗は保存され、再開で再課金されません。", "언제든 중단해도 됩니다. 진행은 저장되며 재개 시 재과금되지 않습니다."],
        "go.samecmd" => ["Same run, as a command:  {}", "同一個工作的指令版：  {}", "同一个工作的指令版：  {}", "同じ実行のコマンド版：  {}", "같은 실행의 명령어:  {}"],
        "go.free" => ["free", "免費", "免费", "無料", "무료"],
        "go.unpriced" => ["unpriced model", "無價目模型", "无价目模型", "価格未登録モデル", "가격 미등록 모델"],
        "gate.both" => ["Translate {} into {to} and add margin notes", "把 {} 翻成 {to} 並加上眉批", "把 {} 翻成 {to} 并加上批注", "{} を {to} に翻訳し、AI注釈を追加", "{} 를 {to} 로 번역하고 AI 주석 추가"],
        "gate.translate" => ["Translate {} into {to}", "把 {} 翻成 {to}", "把 {} 翻成 {to}", "{} を {to} に翻訳", "{} 를 {to} 로 번역"],
        "gate.annotate" => ["Add margin notes to {}, keeping the original text", "為 {} 加眉批，保留原文", "为 {} 加批注，保留原文", "{} にAI注釈を追加（原文はそのまま）", "{} 에 AI 주석 추가 (원문 유지)"],
        "gate.nothing" => ["Nothing selected", "沒有選任何服務", "没有选任何服务", "何も選択されていません", "선택된 작업 없음"],
        "chars.arrow" => ["  {}  ·  {} characters  →  {}", "  {}  ·  {} 字  →  {}", "  {}  ·  {} 字  →  {}", "  {}  ·  {} 文字  →  {}", "  {}  ·  {}자  →  {}"],
        "chars.noarrow" => ["  {}  ·  {} characters", "  {}  ·  {} 字", "  {}  ·  {} 字", "  {}  ·  {} 文字", "  {}  ·  {}자"],
        "leg.confirm" => ["confirm", "確認", "确认", "確定", "확인"],
        "go.comma" => [", ", "，", "，", "、", ", "],
        _ => {
            debug_assert!(false, "missing i18n key: {key}");
            ["", "", "", "", ""]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_parsing_covers_the_five_languages() {
        assert_eq!(parse_locale("zh_TW.UTF-8"), Some(Lang::ZhTw));
        assert_eq!(parse_locale("zh-Hant"), Some(Lang::ZhTw));
        assert_eq!(parse_locale("zh_CN.UTF-8"), Some(Lang::ZhCn));
        assert_eq!(parse_locale("ja_JP.UTF-8"), Some(Lang::Ja));
        assert_eq!(parse_locale("ko_KR.UTF-8"), Some(Lang::Ko));
        assert_eq!(parse_locale("en_US.UTF-8"), Some(Lang::En));
        assert_eq!(parse_locale("fr_FR.UTF-8"), Some(Lang::En));
        assert_eq!(parse_locale("C"), None);
        assert_eq!(parse_locale(""), None);
    }

    #[test]
    fn every_key_has_all_five_languages() {
        // A sample across sections; the debug_assert in t() catches typos at
        // call sites, this catches half-filled rows.
        for key in [
            "menu.title",
            "list.title",
            "path.open",
            "cfg.title",
            "cfg.services",
            "cfg.level.beginner",
            "cfg.voice.companion",
            "svc.world",
            "set.subtitle",
            "set.booksdir",
            "leg.move",
            "gate.both",
            "go.stopsafe",
            "n.chapter",
        ] {
            let row = t(key);
            for (i, s) in row.iter().enumerate() {
                assert!(!s.is_empty(), "{key}[{i}] is empty");
            }
        }
    }

    #[test]
    fn english_plural_pattern_splits() {
        force(Lang::En);
        // force() may lose the race to another test; only assert when English
        // actually won the pin.
        if lang() == Lang::En {
            assert_eq!(trn("n.chapter", 1), "1 chapter");
            assert_eq!(trn("n.chapter", 3), "3 chapters");
        }
    }
}
