use serde::{Deserialize, Serialize};

/// Translation level. Sentence is the internal id for the standard
/// (single-pass) path — the public name is "standard mode" (一般/標準/표준);
/// Expert is multi-pass, book-level consistency. The `sentence` value and the
/// historical aliases stay accepted forever (cache/config compatibility).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Level {
    #[default]
    Sentence,
    Expert,
}

impl std::str::FromStr for Level {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "sentence" | "fast" | "standard" | "句" | "逐句" | "快速" | "標準" | "一般" => {
                Ok(Level::Sentence)
            }
            "expert" | "專家" | "专家" => Ok(Level::Expert),
            other => Err(format!("unknown level: {other}")),
        }
    }
}

/// How translated text is written back into the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum OutputMode {
    /// Replace the source text with the translation.
    #[default]
    Replace,
    /// Keep source and append the translation as a sibling block (bilingual).
    Bilingual,
}

impl std::str::FromStr for OutputMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "replace" => Ok(OutputMode::Replace),
            "bilingual" => Ok(OutputMode::Bilingual),
            other => Err(format!("unknown output mode: {other}")),
        }
    }
}

/// Which LLM backend to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ProviderKind {
    /// Offline echo provider — proves the pipeline & format fidelity without an API key.
    #[default]
    Mock,
    /// OpenAI `/v1/chat/completions`-compatible (OpenAI, many local servers, OpenRouter…).
    OpenAi,
    /// Anthropic `/v1/messages`.
    Anthropic,
    /// Local Ollama.
    Ollama,
}

impl std::str::FromStr for ProviderKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mock" => Ok(ProviderKind::Mock),
            "openai" => Ok(ProviderKind::OpenAi),
            "anthropic" => Ok(ProviderKind::Anthropic),
            "ollama" => Ok(ProviderKind::Ollama),
            other => Err(format!("unknown provider: {other}")),
        }
    }
}

/// How often annotations should stop at a paragraph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Density {
    /// Only the most essential notes.
    Sparse,
    #[default]
    Medium,
    /// Frequent, generous notes (still substance-gated).
    Rich,
}

impl std::str::FromStr for Density {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "sparse" | "精" | "疏" => Ok(Density::Sparse),
            "medium" | "適中" | "适中" => Ok(Density::Medium),
            "rich" | "豐" | "丰" => Ok(Density::Rich),
            other => Err(format!("unknown density: {other}")),
        }
    }
}

/// The margin-note voice: which engine-default style paragraph applies when
/// the user has not written a custom `style`. Two fixed registers — the hard
/// rules (neutrality, no reader-addressing, no spoilers) are identical in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteVoice {
    /// 書齋：the restrained scholarly register (the historical default).
    #[default]
    Study,
    /// 陪讀：a companion register — more conversational, a higher share of
    /// short reaction notes. Same hard rules, same length ceiling.
    Companion,
}

impl std::str::FromStr for NoteVoice {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "study" | "書齋" | "书斋" => Ok(NoteVoice::Study),
            "companion" | "陪讀" | "陪读" => Ok(NoteVoice::Companion),
            other => Err(format!("unknown note voice: {other} (study | companion)")),
        }
    }
}

/// The explanation level (講解水位): how much scaffolding the notes assume.
/// `General` is the default; `Beginner` explains everything in everyday
/// language with examples; `Insider` skips anything an insider would know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExplainLevel {
    /// 入門白話：assume no background; explain to be understood.
    Beginner,
    #[default]
    General,
    /// 內行：only what an insider could not easily look up.
    Insider,
}

impl std::str::FromStr for ExplainLevel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "beginner" | "入門" | "入门" => Ok(ExplainLevel::Beginner),
            "general" | "一般" => Ok(ExplainLevel::General),
            "insider" | "內行" | "内行" => Ok(ExplainLevel::Insider),
            other => Err(format!(
                "unknown explain level: {other} (beginner | general | insider)"
            )),
        }
    }
}

/// Reader-personalised margin notes (眉批): the engine reads the whole book and
/// leaves neutral background notes where THIS reader is likely to want them.
/// Orthogonal to translation — can run with it or alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationConfig {
    /// Free-text reader background + motivation. Used only to decide WHERE to
    /// stop, WHAT angle to take and HOW deep to go — never quoted in the notes.
    /// OPTIONAL by design since v7: most readers can pick what they want far
    /// more accurately than they can articulate it, so an empty profile with
    /// at least one service preset ticked is a fully supported configuration
    /// (the prompts substitute an honest fallback line). Callers enforce
    /// "profile or at least one service".
    pub reader_profile: String,
    /// The explanation level (講解水位; see `ExplainLevel`). Enters
    /// `annotation_signature` — changing it re-annotates, never re-bills.
    #[serde(default)]
    pub level: ExplainLevel,
    /// Cognitive anchors (認知錨): short phrases naming what the reader already
    /// knows — profession, fields, books read, lived contexts. The prompts use
    /// them to bridge new material FROM familiar ground (analogies, contrasts);
    /// like the profile they steer selection and angle only, and are never
    /// quoted or referenced in the note text. Normalised via
    /// `annotate::prompt::canonical_anchors` (trimmed, deduped, capped) — only
    /// that canonical list enters prompts and the annotation signature.
    #[serde(default)]
    pub anchors: Vec<String>,
    /// Which engine-default style register applies when `style` is None.
    /// A custom `style` always wins over the voice's default paragraph.
    #[serde(default)]
    pub voice: NoteVoice,
    /// Language the notes are written in. `None` = follow the reader profile's
    /// own language (annotate-only); callers translating a book resolve this to
    /// the target language before building the config (AN-007).
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub density: Density,
    /// User-editable note STYLE paragraph (tone, depth preference, length
    /// target…), mirroring `TranslateConfig::custom_prompt`: it is injected
    /// into the writing (N1) and review (N2) prompts inside the locked hard
    /// rules, so it can tune quality but never break the content contract.
    /// `None` = the engine default (`annotate::prompt::DEFAULT_NOTE_STYLE`).
    #[serde(default)]
    pub style: Option<String>,
    /// Preset help angles the reader ticked without typing (App chips /
    /// CLI `--note-presets`). Fixed lowercase ids — see
    /// `annotate::prompt::PRESETS` (terms, history, author, culture,
    /// characters, concepts). Unknown ids are ignored (callers warn); order
    /// and duplicates are normalised away, so only the SET of valid ids is
    /// meaningful — that canonical set enters `annotation_signature` (change
    /// the ticks → re-annotate; the translation cache is never touched).
    #[serde(default)]
    pub presets: Vec<String>,
}

/// The reader-profile contract (讀者側寫契約): the one document a reader — or
/// the reader's own AI agent, handed our standard prompt — fills to steer the
/// margin notes. CLI accepts it as a JSON file (`--note-profile p.json`) or
/// inline JSON; MCP accepts inline JSON only (no @file over the wire). Every
/// field is optional; explicit flags override profile fields. UNTRUSTED INPUT:
/// consumers must run the values through the same sanitisation as any other
/// caller-supplied text (canonical anchors/presets, style length cap) — the
/// contract never grants a bypass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderProfile {
    /// Why this reader is reading THIS book (maps to `reader_profile`).
    #[serde(default)]
    pub purpose: Option<String>,
    /// "beginner" | "general" | "insider" (maps to `level`).
    #[serde(default)]
    pub level: Option<String>,
    /// Cognitive anchors — what the reader already knows (maps to `anchors`).
    #[serde(default)]
    pub anchors: Vec<String>,
    /// Preset help-angle ids (maps to `presets`; unknown ids are ignored).
    #[serde(default)]
    pub presets: Vec<String>,
    /// "study" | "companion" (maps to `voice`).
    #[serde(default)]
    pub voice: Option<String>,
    /// Note language (maps to `lang`).
    #[serde(default)]
    pub lang: Option<String>,
    /// "sparse" | "medium" | "rich" (maps to `density`).
    #[serde(default)]
    pub density: Option<String>,
    /// Custom style paragraph (maps to `style`; length-capped downstream).
    #[serde(default)]
    pub style: Option<String>,
}

impl ReaderProfile {
    /// Parse a profile document. Unknown keys are an error on purpose: the
    /// document is usually machine-written from our published schema, and a
    /// silently-dropped typo ("ancors") would quietly weaken every note.
    pub fn from_json(raw: &str) -> Result<Self, String> {
        serde_json::from_str(raw).map_err(|e| format!("reader profile JSON invalid: {e}"))
    }
}

/// Everything needed to translate one book. Shared verbatim by CLI and Desktop —
/// the desktop "advanced settings" panel is just a visualisation of these fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateConfig {
    /// BCP-47-ish target language label, e.g. "繁體中文", "English", "日本語".
    pub target_lang: String,
    pub level: Level,
    /// Injected into the system prompt's user section; cannot override the hard rules.
    #[serde(default)]
    pub custom_prompt: Option<String>,
    pub output_mode: OutputMode,
    pub provider: ProviderKind,
    pub model: String,
    /// Max source units per LLM batch.
    #[serde(default = "default_max_batch_sentences")]
    pub max_batch_sentences: usize,
    /// Soft token ceiling per chunk.
    #[serde(default = "default_max_chunk_tokens")]
    pub max_chunk_tokens: usize,
    /// Max batches translated concurrently. 1 = sequential (safe default, and
    /// what subscription mode should use — concurrent provider subprocesses
    /// are heavy and rate-limit-prone). Direct API providers handle several at
    /// once and finish far faster. NOT part of `cache_signature` (it changes
    /// throughput, never the produced translation).
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// Reader-personalised annotations. `None` = feature off. NOT part of
    /// `cache_signature()` — annotations have their own signature so changing
    /// the reader profile never re-bills the translation cache.
    #[serde(default)]
    pub annotations: Option<AnnotationConfig>,
}

fn default_max_batch_sentences() -> usize {
    10
}
fn default_max_chunk_tokens() -> usize {
    1500
}
fn default_concurrency() -> usize {
    1
}

impl TranslateConfig {
    pub fn new(target_lang: impl Into<String>) -> Self {
        Self {
            target_lang: target_lang.into(),
            level: Level::default(),
            custom_prompt: None,
            output_mode: OutputMode::default(),
            provider: ProviderKind::default(),
            model: "mock".to_string(),
            max_batch_sentences: default_max_batch_sentences(),
            max_chunk_tokens: default_max_chunk_tokens(),
            concurrency: default_concurrency(),
            annotations: None,
        }
    }

    /// Stable hash of the settings that, when changed, must invalidate cached translations.
    pub fn cache_signature(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(self.target_lang.as_bytes());
        h.update(format!("{:?}", self.level).as_bytes());
        h.update(self.custom_prompt.as_deref().unwrap_or("").as_bytes());
        h.update(self.model.as_bytes());
        h.update(format!("{:?}", self.provider).as_bytes());
        format!("{:x}", h.finalize())
    }

    /// Stable hash of the settings that, when changed, must invalidate cached
    /// ANNOTATIONS (and only those). Independent from `cache_signature()` on
    /// purpose: editing the reader profile re-annotates but never re-bills the
    /// translation, and vice versa. `None` when annotations are off.
    pub fn annotation_signature(&self) -> Option<String> {
        use sha2::{Digest, Sha256};
        let a = self.annotations.as_ref()?;
        let mut h = Sha256::new();
        // Version salt: bump when the annotation prompt/protocol changes in a
        // way that makes old cached notes stale.
        //   v1 → v2 (2026-07-06): chapter-level selection pass (AN-013) + note
        //   placement (AN-014). Old plain-string note caches naturally miss and
        //   re-annotate under the new architecture. The TRANSLATION signature
        //   (`cache_signature`) is untouched — never re-bills translations.
        //   v2 → v3 (2026-07-07): editable note-style layer — the length target
        //   moved from the hard rules into the (overridable) default style, so
        //   every v2 prompt text changed; `style` joins the hash so editing the
        //   style re-annotates (never re-bills the translation).
        //   v3 → v4 (2026-07-19): preset help angles (`presets`) join the hash
        //   (tick a chip → re-annotate), and the prompt texts changed with the
        //   product rename 眉批 → 眉批. The TRANSLATION signature is untouched.
        //   v4 → v5 (2026-07-19): the margin-note writing framework landed —
        //   `DEFAULT_NOTE_STYLE`, `select_system()` and `review_system()` prompt
        //   texts all changed materially. None of those three enter the hash on
        //   their own (default style hashes as "" here; the two system prompts
        //   are prompt-only), so the salt bump is what makes old caches miss.
        //   v5 → v6 (2026-07-19): framework second pass — hard_rules gained the
        //   no-forward-spoiler + factual-external-connection rules, select_system
        //   gained "climaxes aren't note-spots / must map to a reader need /
        //   prefer backward references", and DEFAULT_NOTE_STYLE gained native
        //   register + the world-connection layer. Same reason as v5: the prompt
        //   text changed but does not enter the hash on its own.
        //   v6 → v7 (2026-08-15): the reader-profile contract landed — cognitive
        //   anchors (`anchors`) and the voice register (`voice`) join the hash,
        //   the plan pass gained the book-wide thread map (threads in the plan
        //   JSON + backward-reference planning in selection), and N1/N2 gained
        //   the reader-boundary output check. Prompt texts changed materially;
        //   the salt bump is what makes old v6 caches miss.
        // NOTE: the salt keeps the historical "inkferry-" prefix on purpose —
        // the project was renamed InkFerry → Translatus (2026-07-19), but the
        // salt is an opaque compatibility contract: renaming it would silently
        // invalidate every user's note cache for a pure branding change.
        h.update(b"inkferry-anno-v7");
        h.update(b"\x00");
        h.update(a.reader_profile.as_bytes());
        h.update(b"\x00");
        // Explanation level (講解水位) — part of the v7 recipe (same
        // unreleased batch as anchors/voice).
        h.update(format!("{:?}", a.level).as_bytes());
        h.update(b"\x00");
        // Canonical anchors only — like the presets, the signature reacts to
        // the meaningful list, not to whitespace/duplicate/overflow noise.
        h.update(
            crate::annotate::prompt::canonical_anchors(&a.anchors)
                .join("\x1f")
                .as_bytes(),
        );
        h.update(b"\x00");
        h.update(format!("{:?}", a.voice).as_bytes());
        h.update(b"\x00");
        h.update(a.lang.as_deref().unwrap_or("").as_bytes());
        h.update(b"\x00");
        h.update(format!("{:?}", a.density).as_bytes());
        h.update(b"\x00");
        h.update(a.style.as_deref().unwrap_or("").as_bytes());
        h.update(b"\x00");
        // Canonical form only: unknown ids fall out, order/dup don't matter —
        // the signature reacts to the meaningful set, nothing else.
        h.update(
            crate::annotate::prompt::canonical_presets(&a.presets)
                .join(",")
                .as_bytes(),
        );
        h.update(b"\x00");
        h.update(self.model.as_bytes());
        h.update(b"\x00");
        h.update(format!("{:?}", self.provider).as_bytes());
        Some(format!("{:x}", h.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_signature_is_stable_and_sensitive() {
        let base = TranslateConfig::new("繁體中文");
        // GOLDEN: this value is part of the on-disk cache contract. If it ever
        // changes, every existing `.etjob` silently invalidates and paying users
        // get re-billed. Only update this constant deliberately, with a migration
        // plan — never just to make a green build.
        assert_eq!(
            base.cache_signature(),
            "3cc51ff09424a66187c8cb6f7705eceae3582eb7ebd0136f2ec888c3498be70d",
            "cache_signature drifted — this re-bills every existing cache; intentional?"
        );
        // Deterministic.
        assert_eq!(
            base.cache_signature(),
            TranslateConfig::new("繁體中文").cache_signature()
        );
        // Every cache-relevant field must change the signature (else stale cache reuse).
        let mut c = TranslateConfig::new("繁體中文");
        c.target_lang = "简体中文".into();
        assert_ne!(
            base.cache_signature(),
            c.cache_signature(),
            "target_lang must affect sig"
        );
        let mut c = TranslateConfig::new("繁體中文");
        c.level = Level::Expert;
        assert_ne!(
            base.cache_signature(),
            c.cache_signature(),
            "level must affect sig"
        );
        let mut c = TranslateConfig::new("繁體中文");
        c.model = "gpt-x".into();
        assert_ne!(
            base.cache_signature(),
            c.cache_signature(),
            "model must affect sig"
        );
        let mut c = TranslateConfig::new("繁體中文");
        c.custom_prompt = Some("style".into());
        assert_ne!(
            base.cache_signature(),
            c.cache_signature(),
            "custom_prompt must affect sig"
        );
    }

    fn anno_cfg() -> TranslateConfig {
        let mut c = TranslateConfig::new("繁體中文");
        c.annotations = Some(AnnotationConfig {
            reader_profile: "工程師，想讀懂經濟學經典".into(),
            level: ExplainLevel::General,
            anchors: Vec::new(),
            voice: NoteVoice::Study,
            lang: Some("繁體中文".into()),
            density: Density::Medium,
            style: None,
            presets: Vec::new(),
        });
        c
    }

    // The re-billing contract in the other direction: turning annotations on,
    // or editing the reader profile, must NOT touch the translation signature.
    #[test]
    fn annotations_never_affect_cache_signature() {
        let base = TranslateConfig::new("繁體中文");
        assert_eq!(base.cache_signature(), anno_cfg().cache_signature());
        // …including the editable note style: tuning 眉批 quality must never
        // re-bill the translation cache.
        let mut styled = anno_cfg();
        styled.annotations.as_mut().unwrap().style = Some("偏短、口語".into());
        assert_eq!(base.cache_signature(), styled.cache_signature());
        // …and the preset help angles (chips): ticking one re-annotates only.
        let mut chipped = anno_cfg();
        chipped.annotations.as_mut().unwrap().presets = vec!["terms".into(), "history".into()];
        assert_eq!(base.cache_signature(), chipped.cache_signature());
    }

    #[test]
    fn annotation_signature_is_sensitive_and_independent() {
        let base = anno_cfg();
        let sig = base.annotation_signature().expect("annotations on");
        // Deterministic.
        assert_eq!(sig, anno_cfg().annotation_signature().unwrap());
        // Off → None.
        assert_eq!(
            TranslateConfig::new("繁體中文").annotation_signature(),
            None
        );
        // Never colliding with the translation signature namespace.
        assert_ne!(sig, base.cache_signature());

        // Every annotation-relevant field must change the signature.
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().reader_profile = "歷史系學生".into();
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "profile must affect sig"
        );
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().lang = Some("English".into());
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "lang must affect sig"
        );
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().density = Density::Rich;
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "density must affect sig"
        );
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().style = Some("偏短、口語".into());
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "note style must affect sig (change style → re-annotate)"
        );
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().presets = vec!["terms".into()];
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "presets must affect sig (tick a chip → re-annotate)"
        );
        let mut c = anno_cfg();
        c.model = "gpt-x".into();
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "model must affect sig"
        );
        let mut c = anno_cfg();
        c.provider = ProviderKind::OpenAi;
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "provider must affect sig"
        );

        // …while the TRANSLATION-only fields must not (independence).
        let mut c = anno_cfg();
        c.target_lang = "English".into();
        c.custom_prompt = Some("style".into());
        c.level = Level::Expert;
        assert_eq!(sig, c.annotation_signature().unwrap());
    }

    // Salt-bump contract (v6 → v7, 2026-08-15): the reader-profile contract
    // landed — anchors + voice join the hash, the plan pass gained the thread
    // map, N1/N2 gained the reader-boundary check. The prompt-text changes do
    // not enter the hash on their own, so the v7 signature must never equal
    // what the v6 recipe produced, or stale v6 caches would keep the old notes
    // (translations untouched).
    #[test]
    fn annotation_signature_salt_bumped_to_v7() {
        use sha2::{Digest, Sha256};
        let cfg = anno_cfg();
        let a = cfg.annotations.as_ref().unwrap();
        let mut h = Sha256::new();
        h.update(b"inkferry-anno-v6");
        h.update(b"\x00");
        h.update(a.reader_profile.as_bytes());
        h.update(b"\x00");
        h.update(a.lang.as_deref().unwrap_or("").as_bytes());
        h.update(b"\x00");
        h.update(format!("{:?}", a.density).as_bytes());
        h.update(b"\x00");
        h.update(a.style.as_deref().unwrap_or("").as_bytes());
        h.update(b"\x00");
        h.update(
            crate::annotate::prompt::canonical_presets(&a.presets)
                .join(",")
                .as_bytes(),
        );
        h.update(b"\x00");
        h.update(cfg.model.as_bytes());
        h.update(b"\x00");
        h.update(format!("{:?}", cfg.provider).as_bytes());
        let v6 = format!("{:x}", h.finalize());
        assert_ne!(
            cfg.annotation_signature().unwrap(),
            v6,
            "the v7 salt must invalidate every v6 note cache"
        );
    }

    // The v7 fields drive re-annotation: adding an anchor or switching the
    // voice register must produce different notes, so both must miss the old
    // cache — while (like every annotation field) never re-billing the
    // translation cache.
    #[test]
    fn anchors_and_voice_enter_annotation_signature_only() {
        let base = anno_cfg();
        let sig = base.annotation_signature().unwrap();
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().anchors = vec!["軟體工程師".into()];
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "anchors must affect sig"
        );
        assert_eq!(base.cache_signature(), c.cache_signature());
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().voice = NoteVoice::Companion;
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "voice must affect sig"
        );
        assert_eq!(base.cache_signature(), c.cache_signature());
        // Canonicalisation: whitespace / duplicates / overflow don't re-bill.
        let sig_of = |anchors: &[&str]| {
            let mut c = anno_cfg();
            c.annotations.as_mut().unwrap().anchors =
                anchors.iter().map(|s| s.to_string()).collect();
            c.annotation_signature().unwrap()
        };
        assert_eq!(
            sig_of(&["軟體工程師", " 軟體工程師 ", ""]),
            sig_of(&["軟體工程師"])
        );
    }

    // The explanation level drives re-annotation (a beginner edition and an
    // insider edition are different products) and never re-bills translation.
    #[test]
    fn explain_level_enters_annotation_signature_only() {
        let base = anno_cfg();
        let sig = base.annotation_signature().unwrap();
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().level = ExplainLevel::Beginner;
        assert_ne!(
            sig,
            c.annotation_signature().unwrap(),
            "level must affect sig"
        );
        assert_eq!(base.cache_signature(), c.cache_signature());
        let mut c = anno_cfg();
        c.annotations.as_mut().unwrap().level = ExplainLevel::Insider;
        assert_ne!(sig, c.annotation_signature().unwrap());
    }

    // The reader-profile contract file round-trips, and unknown keys are a
    // hard error (a machine-written typo must not silently weaken the notes).
    #[test]
    fn reader_profile_contract_parses_and_rejects_unknown_keys() {
        let p = ReaderProfile::from_json(
            r#"{"purpose":"想拆解方法論","anchors":["創業者"],"presets":["terms","world"],"voice":"companion","density":"rich","level":"beginner"}"#,
        )
        .unwrap();
        assert_eq!(p.purpose.as_deref(), Some("想拆解方法論"));
        assert_eq!(p.anchors, vec!["創業者"]);
        assert_eq!(p.voice.as_deref(), Some("companion"));
        assert!(ReaderProfile::from_json(r#"{"ancors":["typo"]}"#).is_err());
        assert!(ReaderProfile::from_json("not json").is_err());
        // Empty object is a valid (all-default) profile.
        assert!(ReaderProfile::from_json("{}").is_ok());
    }

    // The presets hash only their canonical set: unknown ids fall out (they do
    // nothing, so they must not re-annotate), and order/duplicates don't matter.
    #[test]
    fn annotation_presets_enter_signature_only_canonically() {
        let base = anno_cfg();
        let sig = |presets: &[&str]| {
            let mut c = base.clone();
            c.annotations.as_mut().unwrap().presets =
                presets.iter().map(|s| s.to_string()).collect();
            c.annotation_signature().unwrap()
        };
        // Unknown-only == none at all (ignored ids can't re-bill notes).
        assert_eq!(sig(&[]), sig(&["nonsense"]));
        // Order and duplicates normalise away.
        assert_eq!(
            sig(&["terms", "history"]),
            sig(&["history", "terms", "terms"])
        );
        // A valid tick does change the signature.
        assert_ne!(sig(&[]), sig(&["terms"]));
        assert_ne!(sig(&["terms"]), sig(&["terms", "history"]));
    }

    // Old configs (serialized before annotations existed) must still load.
    #[test]
    fn config_json_backcompat_without_annotations() {
        let j = r#"{"target_lang":"繁體中文","level":"sentence","output_mode":"replace","provider":"mock","model":"mock"}"#;
        let c: TranslateConfig = serde_json::from_str(j).unwrap();
        assert!(c.annotations.is_none());
    }
}
