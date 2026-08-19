//! Capability-parity enforcement (see CAPABILITIES.toml header + the desktop
//! app's capability-parity design). These tests keep the manifest honest in
//! both directions, deterministically and offline:
//!
//! 1. Declared CLI commands/flags must exist in the real clap surface
//!    (a manifest can't promise vaporware).
//! 2. Every `TranslateConfig` field must be claimed by some capability, and
//!    every claimed field must exist (adding a config field without declaring
//!    a capability — the classic "forgot the CLI" failure — is a red test).
//! 3. Every declared test anchor must exist as a test function somewhere in
//!    the workspace sources (a capability ships with tests or not at all).

use super::Cli;
use clap::CommandFactory;
use et_core::config::TranslateConfig;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("../../../CAPABILITIES.toml");
const KNOWN_SURFACES: [&str; 2] = ["cli", "gui"];

fn manifest() -> toml::Value {
    toml::from_str(MANIFEST).expect("CAPABILITIES.toml must parse as TOML")
}

fn capabilities(m: &toml::Value) -> Vec<toml::value::Table> {
    m.get("capability")
        .and_then(|v| v.as_array())
        .expect("CAPABILITIES.toml must contain [[capability]] entries")
        .iter()
        .map(|v| v.as_table().expect("capability must be a table").clone())
        .collect()
}

fn str_list(cap: &toml::value::Table, key: &str) -> Vec<String> {
    cap.get(key)
        .map(|v| {
            v.as_array()
                .unwrap_or_else(|| panic!("`{key}` must be an array of strings"))
                .iter()
                .map(|s| s.as_str().expect("array items must be strings").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn cap_id(cap: &toml::value::Table) -> &str {
    cap.get("id")
        .and_then(|v| v.as_str())
        .expect("capability needs an id")
}

/// The schema itself: unique ids, valid surfaces, reasons for deliberate
/// asymmetry, and a `cli` table wherever the cli surface is claimed.
#[test]
fn manifest_schema_is_valid() {
    let m = manifest();
    let caps = capabilities(&m);
    assert!(
        !caps.is_empty(),
        "manifest declares at least one capability"
    );

    let mut seen = BTreeSet::new();
    for cap in &caps {
        let id = cap_id(cap);
        assert!(seen.insert(id.to_string()), "duplicate capability id: {id}");
        assert!(
            cap.get("since").and_then(|v| v.as_str()).is_some(),
            "{id}: `since` version is required"
        );

        let surfaces = str_list(cap, "surfaces");
        assert!(!surfaces.is_empty(), "{id}: surfaces must not be empty");
        for s in &surfaces {
            assert!(
                KNOWN_SURFACES.contains(&s.as_str()),
                "{id}: unknown surface `{s}`"
            );
        }
        // Deliberate asymmetry must be explicit — a single-surface capability
        // needs a written reason, or it reads as accidental drift.
        if surfaces.len() < KNOWN_SURFACES.len() {
            let reason = cap.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                !reason.trim().is_empty(),
                "{id}: surfaces {surfaces:?} lacks a `reason` — intentional asymmetry must be declared"
            );
        }
        if surfaces.iter().any(|s| s == "cli") {
            assert!(
                cap.get("cli").and_then(|v| v.as_table()).is_some(),
                "{id}: claims the cli surface but has no `cli` table"
            );
        }
        assert!(
            !str_list(cap, "tests").is_empty(),
            "{id}: every capability must anchor at least one test"
        );
    }
}

/// (a) Every declared command/flag exists in the real clap Command tree.
/// A flag must exist on at least one of its capability's listed commands
/// (or on the root when the command list is empty / the flag is global).
#[test]
fn declared_cli_commands_and_flags_exist() {
    let root = Cli::command();
    let m = manifest();

    let has_flag =
        |cmd: &clap::Command, flag: &str| cmd.get_arguments().any(|a| a.get_long() == Some(flag));

    for cap in capabilities(&m) {
        let id = cap_id(&cap).to_string();
        if !str_list(&cap, "surfaces").iter().any(|s| s == "cli") {
            continue;
        }
        let cli = cap.get("cli").and_then(|v| v.as_table()).unwrap();
        let commands: Vec<String> = cli
            .get("commands")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect())
            .unwrap_or_default();
        let flags: Vec<String> = cli
            .get("flags")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect())
            .unwrap_or_default();

        let mut cmds: Vec<&clap::Command> = Vec::new();
        for name in &commands {
            // Space-separated walks a nested path ("mcp install"), so a
            // capability can name a sub-subcommand instead of only the top
            // level — otherwise nested surfaces silently escape this gate.
            let mut node = &root;
            let mut resolved: Option<&clap::Command> = None;
            for part in name.split_whitespace() {
                let sub = node
                    .get_subcommands()
                    .find(|c| c.get_name() == part)
                    .unwrap_or_else(|| {
                        panic!("{id}: declared command `{name}` does not exist in the CLI")
                    });
                resolved = Some(sub);
                node = sub;
            }
            cmds.push(
                resolved.unwrap_or_else(|| panic!("{id}: declared command `{name}` is empty")),
            );
        }

        for flag in &flags {
            let long = flag
                .strip_prefix("--")
                .unwrap_or_else(|| panic!("{id}: flag `{flag}` must be written with a leading --"));
            let found = cmds.iter().any(|c| has_flag(c, long)) || has_flag(&root, long);
            assert!(
                found,
                "{id}: declared flag `{flag}` exists on none of {commands:?} (nor globally) — \
                 the manifest promises a CLI surface the binary doesn't have"
            );
        }
    }
}

/// (b) TranslateConfig fields ↔ manifest `config` union, both directions.
/// This is the trap for the "forgot to expose it" failure mode: a new feature
/// almost always lands a new config field, and an undeclared field turns CI red.
#[test]
fn config_fields_fully_declared() {
    let json =
        serde_json::to_value(TranslateConfig::new("繁體中文")).expect("TranslateConfig serializes");
    let actual: BTreeSet<String> = json
        .as_object()
        .expect("TranslateConfig serializes to an object")
        .keys()
        .cloned()
        .collect();

    let m = manifest();
    let mut declared = BTreeSet::new();
    let mut owners: Vec<(String, String)> = Vec::new(); // (field, capability)
    for cap in capabilities(&m) {
        let id = cap_id(&cap).to_string();
        for field in str_list(&cap, "config") {
            declared.insert(field.clone());
            owners.push((field, id.clone()));
        }
    }

    for (field, id) in &owners {
        assert!(
            actual.contains(field),
            "capability `{id}` declares config field `{field}` which does not exist \
             on TranslateConfig — stale or misspelled manifest entry"
        );
    }
    for field in &actual {
        assert!(
            declared.contains(field),
            "TranslateConfig field `{field}` is not claimed by any capability in \
             CAPABILITIES.toml — new config means a new/updated capability declaration \
             (and a decision about its cli/gui surfaces)"
        );
    }
}

/// (c) Every declared test anchor exists as a test function in the sources.
#[test]
fn declared_test_anchors_exist() {
    let root = workspace_root();
    let mut sources = Vec::new();
    collect_rs_sources(&root.join("crates"), &mut sources);
    collect_rs_sources(&root.join("apps"), &mut sources);
    assert!(
        sources.len() > 5,
        "source scan looks wrong (found {} .rs files under {})",
        sources.len(),
        root.display()
    );
    let all: String = sources.iter().map(|(_, s)| s.as_str()).collect();

    let m = manifest();
    for cap in capabilities(&m) {
        let id = cap_id(&cap).to_string();
        for anchor in str_list(&cap, "tests") {
            let needle = format!("fn {anchor}(");
            assert!(
                all.contains(&needle),
                "{id}: test anchor `{anchor}` not found in any .rs source — \
                 capabilities must keep their tests alive (or update the manifest)"
            );
        }
    }
}

/// Anchor for the `json-output` capability: --json is a global flag, available
/// on every subcommand.
#[test]
fn json_flag_is_global() {
    let root = Cli::command();
    let json = root
        .get_arguments()
        .find(|a| a.get_long() == Some("json"))
        .expect("--json exists on the root command");
    assert!(json.is_global_set(), "--json must be a global flag");
}

fn workspace_root() -> PathBuf {
    // apps/cli/ → workspace root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves")
}

fn collect_rs_sources(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name.starts_with('.') {
                continue;
            }
            collect_rs_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                out.push((path, content));
            }
        }
    }
}
