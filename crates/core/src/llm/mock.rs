//! Offline echo provider. It understands the sentence-level JSON protocol and
//! returns aligned output with placeholders preserved, so the full pipeline —
//! parse → batch → validate → reassemble → write — can be exercised on a real
//! EPUB with zero API key and zero cost. Translations are marked with a visible
//! prefix so the output file demonstrably differs from the source.
//!
//! It also speaks the annotation protocol (detected via the prompt markers) so
//! the FULL annotation pipeline — N-sel selection, N1 note writing and the N2
//! unification review — runs deterministically at zero cost: the selection
//! picks every 3rd unit (alternating before/after placement, decreasing
//! priority), the writer notes every selected unit, and the review drops
//! text-duplicates while keeping the rest.

use crate::annotate::prompt::{N1_MARKER, N2_MARKER, NSEL_MARKER};
use crate::error::Result;
use crate::llm::{CompletionRequest, CompletionResponse};
use serde_json::{json, Value};

/// Cap-test hook: when any unit text carries this token, the mock selection
/// pass "greedily" selects EVERY unit — letting tests prove the program-side
/// chapter cap (AN-013) is what actually limits note volume, not the model.
pub const SELECT_ALL_TOKEN: &str = "[[MOCK:SELECT-ALL]]";

/// Repair-path hook: a unit carrying this token comes back with its paired
/// placeholders stripped the first time, exactly as a real model does when the
/// source emphasises a word the target language has no separate word for
/// ("there ⟦2⟧is⟦/2⟧ one right way"). The follow-up repair request — recognised
/// by the rejection notice the engine appends — returns them intact.
pub const DROP_PAIRS_TOKEN: &str = "[[MOCK:DROP-PAIRS]]";

#[derive(Default)]
pub struct MockProvider;

impl MockProvider {
    pub async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let last = req
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");
        let tokens_in = (req.system.len() + last.len()) as u64 / 4;

        if req.system.contains(NSEL_MARKER) {
            let text = mock_select(last)?;
            let tokens_out = text.len() as u64 / 4;
            return Ok(CompletionResponse {
                text,
                tokens_in,
                tokens_out,
            });
        }
        if req.system.contains(N1_MARKER) {
            let text = mock_notes(last)?;
            let tokens_out = text.len() as u64 / 4;
            return Ok(CompletionResponse {
                text,
                tokens_in,
                tokens_out,
            });
        }
        if req.system.contains(N2_MARKER) {
            let text = mock_review(last)?;
            let tokens_out = text.len() as u64 / 4;
            return Ok(CompletionResponse {
                text,
                tokens_in,
                tokens_out,
            });
        }

        // A repair request carries the engine's rejection notice as its last
        // message; the payload it is repairing is the first.
        let is_repair = req.messages.len() > 1
            && req
                .messages
                .iter()
                .any(|m| m.content.contains("That response was rejected"));
        let last = req
            .messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or(last);

        // Try to honour the sentence protocol: {"sentences":[{"id","text"}]}
        let text = match serde_json::from_str::<Value>(last) {
            Ok(v) => {
                if let Some(arr) = v.get("sentences").and_then(|s| s.as_array()) {
                    let out: Vec<Value> = arr
                        .iter()
                        .map(|s| {
                            let id = s.get("id").cloned().unwrap_or(Value::Null);
                            let src = s.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            let t = if src.contains(DROP_PAIRS_TOKEN) && !is_repair {
                                strip_paired_placeholders(&pseudo_translate(src))
                            } else {
                                pseudo_translate(src)
                            };
                            json!({ "id": id, "translation": t })
                        })
                        .collect();
                    serde_json::to_string(&out)?
                } else {
                    pseudo_translate(last)
                }
            }
            Err(_) => pseudo_translate(last),
        };

        let tokens_out = text.len() as u64 / 4;
        Ok(CompletionResponse {
            text,
            tokens_in,
            tokens_out,
        })
    }
}

/// Identity translation with a visible marker; placeholders inside the text are
/// preserved untouched (we never look inside `⟦…⟧`).
fn pseudo_translate(src: &str) -> String {
    format!("〈譯〉{src}")
}

/// Drop every `⟦n⟧ … ⟦/n⟧` pair, keeping the words between them — the shape of
/// a real model's answer when it cannot place inline emphasis in the target.
fn strip_paired_placeholders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('⟦') {
        let Some(close) = rest[open..].find('⟧') else {
            break;
        };
        let close = open + close + '⟧'.len_utf8();
        let tag = &rest[open + '⟦'.len_utf8()..close - '⟧'.len_utf8()];
        // Keep void markers (⟦C1⟧); drop only the paired ones.
        let paired = tag
            .trim_start_matches('/')
            .chars()
            .all(|c| c.is_ascii_digit())
            && !tag.is_empty();
        out.push_str(&rest[..open]);
        if !paired {
            out.push_str(&rest[open..close]);
        }
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

/// Deterministic N-sel behaviour: pick every 3rd unit (positions 0, 3, …) with
/// alternating placement (before, after, before, …) and strictly decreasing
/// priority — so when the program-side cap trims, the earliest picks survive.
/// The `SELECT_ALL_TOKEN` in any unit text flips it to greedily selecting every
/// unit (the cap-enforcement test path).
fn mock_select(payload: &str) -> Result<String> {
    let v: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let units = v
        .get("units")
        .and_then(|u| u.as_array())
        .cloned()
        .unwrap_or_default();
    let select_all = payload.contains(SELECT_ALL_TOKEN);
    let mut selections = Vec::new();
    let mut ordinal = 0i64;
    for (i, u) in units.iter().enumerate() {
        if !select_all && i % 3 != 0 {
            continue;
        }
        let id = u.get("id").cloned().unwrap_or(Value::Null);
        let text = u.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let head: String = text.chars().take(8).collect();
        let pos = if ordinal % 2 == 0 { "before" } else { "after" };
        selections.push(json!({
            "id": id,
            "pos": pos,
            "angle": format!("背景：{head}"),
            "priority": (units.len() as i64 - i as i64).max(1),
        }));
        ordinal += 1;
    }
    Ok(json!({ "selections": selections }).to_string())
}

/// Deterministic N1 behaviour: every received unit was pre-selected, so every
/// one gets a note derived from its text — identical source paragraphs
/// therefore produce identical notes, which is what the N2 dedupe tests need.
fn mock_notes(payload: &str) -> Result<String> {
    let v: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let units = v
        .get("units")
        .and_then(|u| u.as_array())
        .cloned()
        .unwrap_or_default();
    let mut notes = Vec::new();
    let mut topics = Vec::new();
    for u in units.iter() {
        let id = u.get("id").cloned().unwrap_or(Value::Null);
        let text = u.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let head: String = text.chars().take(18).collect();
        notes.push(json!({ "id": id, "note": format!("〈註〉背景補充：{head}") }));
        topics.push(Value::String(head.chars().take(8).collect()));
    }
    Ok(json!({ "notes": notes, "topics": topics }).to_string())
}

/// Deterministic N2 behaviour: keep every judged note, except drop any whose
/// text already appeared earlier in the judged list (a second dedupe layer on
/// top of the program-side exact dedupe).
fn mock_review(payload: &str) -> Result<String> {
    let v: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let judge = v
        .get("judge")
        .and_then(|j| j.as_array())
        .cloned()
        .unwrap_or_default();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in judge {
        let id = item.get("id").cloned().unwrap_or(Value::Null);
        let note = item.get("note").and_then(|n| n.as_str()).unwrap_or("");
        let action = if seen.insert(note.trim().to_string()) {
            "keep"
        } else {
            "drop"
        };
        out.push(json!({ "id": id, "action": action }));
    }
    Ok(serde_json::to_string(&out)?)
}
