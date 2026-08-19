//! Hard, program-side validation of an LLM batch response. This is the guardrail
//! the research flags as essential: LLMs silently merge/split/drop sentences and
//! mangle placeholders. We verify count, id set, non-empty output and placeholder
//! integrity; any failure forces a retry / shrink / fallback upstream.

use crate::error::{CoreError, Result};
use crate::format::placeholder;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct Item {
    id: u64,
    translation: String,
}

/// Parse and validate a response against the units we sent.
/// `units`: id -> source (with placeholders).
pub fn parse_and_check(raw: &str, units: &HashMap<u64, String>) -> Result<HashMap<u64, String>> {
    let json = extract_json_array(raw)
        .ok_or_else(|| CoreError::Validation("no JSON array found in response".into()))?;

    let items: Vec<Item> = serde_json::from_str(&json)
        .map_err(|e| CoreError::Validation(format!("response is not [{{id,translation}}]: {e}")))?;

    if items.len() != units.len() {
        return Err(CoreError::Validation(format!(
            "count mismatch: sent {}, got {}",
            units.len(),
            items.len()
        )));
    }

    let mut out = HashMap::new();
    for it in items {
        let Some(source) = units.get(&it.id) else {
            return Err(CoreError::Validation(format!("unexpected id {}", it.id)));
        };
        if it.translation.trim().is_empty() {
            return Err(CoreError::Validation(format!(
                "empty translation for id {}",
                it.id
            )));
        }
        placeholder::validate(source, &it.translation).map_err(CoreError::Validation)?;
        out.insert(it.id, it.translation);
    }

    if out.len() != units.len() {
        return Err(CoreError::Validation("duplicate ids in response".into()));
    }
    Ok(out)
}

/// Pull the first top-level `[...]` array out of a possibly chatty response
/// (handles ```json fences and stray prose).
fn extract_json_array(s: &str) -> Option<String> {
    let start = s.find('[')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for i in start..s.len() {
        let c = bytes[i] as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units() -> HashMap<u64, String> {
        HashMap::from([
            (1, "Hello ⟦1⟧world⟦/1⟧".to_string()),
            (2, "Bye".to_string()),
        ])
    }

    #[test]
    fn accepts_valid() {
        let raw = r#"[{"id":1,"translation":"你好⟦1⟧世界⟦/1⟧"},{"id":2,"translation":"再見"}]"#;
        let out = parse_and_check(raw, &units()).unwrap();
        assert_eq!(out[&1], "你好⟦1⟧世界⟦/1⟧");
    }

    #[test]
    fn rejects_missing_placeholder() {
        let raw = r#"[{"id":1,"translation":"你好世界"},{"id":2,"translation":"再見"}]"#;
        assert!(parse_and_check(raw, &units()).is_err());
    }

    #[test]
    fn rejects_count_mismatch() {
        let raw = r#"[{"id":1,"translation":"你好⟦1⟧世界⟦/1⟧"}]"#;
        assert!(parse_and_check(raw, &units()).is_err());
    }

    #[test]
    fn tolerates_fences() {
        let raw = "```json\n[{\"id\":1,\"translation\":\"你好⟦1⟧世界⟦/1⟧\"},{\"id\":2,\"translation\":\"再見\"}]\n```";
        assert!(parse_and_check(raw, &units()).is_ok());
    }
}
