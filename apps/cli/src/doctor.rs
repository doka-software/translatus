//! `translatus doctor` — one command that answers the four questions every
//! broken-install report starts with: is the binary I'm running the one on
//! PATH (and does its install still exist on disk)? can the subscription
//! sidecar be found and run? is the default sidecar port free? which agent
//! clients actually have this MCP server registered?
//!
//! Each of those took several minutes of `lsof`/`find`/`mcp list` archaeology
//! the first time an agent hit them in the wild; a health check exists so
//! nobody does that archaeology twice.

use serde_json::json;

pub struct Check {
    pub name: &'static str,
    /// "ok" | "warn" | "fail" | "info"
    pub status: &'static str,
    pub detail: String,
}

fn path_lookup(bin: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p).find_map(|dir| {
            let c = dir.join(bin);
            c.is_file().then_some(c)
        })
    })
}

fn check_binary(out: &mut Vec<Check>) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            out.push(Check {
                name: "binary",
                status: "fail",
                detail: format!("current_exe unavailable: {e}"),
            });
            return;
        }
    };
    // A canonicalize failure here is the "upgraded/removed while running"
    // state: the process keeps running from a deleted inode, but every new
    // spawn from this path (sidecar, MCP self-exec) fails.
    let canon = match std::fs::canonicalize(&exe) {
        Ok(c) => c,
        Err(_) => {
            out.push(Check {
                name: "binary",
                status: "fail",
                detail: format!(
                    "{} no longer exists on disk — the install was likely upgraded or \
                     removed while this process was running. Reinstall or re-run from \
                     the current install.",
                    exe.display()
                ),
            });
            return;
        }
    };
    match path_lookup("translatus").map(|p| std::fs::canonicalize(&p).unwrap_or(p)) {
        Some(on_path) if on_path == canon => out.push(Check {
            name: "binary",
            status: "ok",
            detail: format!(
                "{} (v{}, on PATH)",
                canon.display(),
                env!("CARGO_PKG_VERSION")
            ),
        }),
        Some(on_path) => out.push(Check {
            name: "binary",
            status: "warn",
            detail: format!(
                "running {} but PATH resolves `translatus` to {} — two installs; \
                 MCP registrations launch the PATH one",
                canon.display(),
                on_path.display()
            ),
        }),
        None => out.push(Check {
            name: "binary",
            status: "warn",
            detail: format!(
                "{} is not reachable via PATH — MCP registrations made from here \
                 pin an absolute path and will break if this install moves",
                canon.display()
            ),
        }),
    }
}

fn check_sidecar(out: &mut Vec<Check>) {
    let override_dir = std::env::var_os("TRANSLATUS_SIDECAR_DIR").map(std::path::PathBuf::from);
    match crate::sidecar::locate_kit(override_dir) {
        Ok(dir) => {
            let deps = dir.join("node_modules").is_dir();
            out.push(Check {
                name: "sidecar kit",
                status: "ok",
                detail: format!(
                    "{}{}",
                    dir.display(),
                    if deps {
                        " (dependencies installed)"
                    } else {
                        " (dependencies not installed yet — first subscription run does it)"
                    }
                ),
            });
        }
        Err(e) => out.push(Check {
            name: "sidecar kit",
            status: "warn",
            detail: format!("{e:#}"),
        }),
    }
    match std::process::Command::new("node").arg("--version").output() {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let major: u32 = v
                .trim_start_matches('v')
                .split('.')
                .next()
                .and_then(|m| m.parse().ok())
                .unwrap_or(0);
            if major >= 20 {
                out.push(Check {
                    name: "node",
                    status: "ok",
                    detail: v,
                });
            } else {
                out.push(Check {
                    name: "node",
                    status: "warn",
                    detail: format!("{v} — the sidecar needs Node ≥ 20"),
                });
            }
        }
        _ => out.push(Check {
            name: "node",
            status: "warn",
            detail: "node not found on PATH — subscription provider unavailable \
                     (API key and Ollama sources are unaffected)"
                .into(),
        }),
    }
}

fn check_port(out: &mut Vec<Check>) {
    match std::net::TcpListener::bind("127.0.0.1:8765") {
        Ok(_) => out.push(Check {
            name: "port 8765",
            status: "ok",
            detail: "free (the documented hand-run sidecar port)".into(),
        }),
        Err(_) => out.push(Check {
            name: "port 8765",
            status: "warn",
            detail: "occupied by another process. `--provider subscription` is \
                     unaffected (it picks its own free port); a hand-run sidecar \
                     can use `LLM_SUB_KIT_PORT=<port> npm start`"
                .into(),
        }),
    }
}

fn check_mcp_registrations(out: &mut Vec<Check>) {
    let ours = crate::mcp::registered_by_us();
    for (bin, name) in crate::mcp::known_clients() {
        if path_lookup(bin).is_none() {
            out.push(Check {
                name: "mcp",
                status: "info",
                detail: format!("{name}: client CLI not installed"),
            });
            continue;
        }
        let listed = std::process::Command::new(bin)
            .args(["mcp", "list"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("translatus"));
        match listed {
            Some(true) => out.push(Check {
                name: "mcp",
                status: "ok",
                detail: format!(
                    "{name}: translatus registered{}",
                    if ours.iter().any(|b| b == bin) {
                        " (added by `translatus mcp install`)"
                    } else {
                        ""
                    }
                ),
            }),
            Some(false) => out.push(Check {
                name: "mcp",
                status: "info",
                detail: format!("{name}: not registered (run `translatus mcp install`)"),
            }),
            None => out.push(Check {
                name: "mcp",
                status: "warn",
                detail: format!("{name}: `{bin} mcp list` failed — cannot tell"),
            }),
        }
    }
}

fn check_settings(out: &mut Vec<Check>) {
    let Some(path) = crate::tui::store::settings_path() else {
        out.push(Check {
            name: "settings",
            status: "warn",
            detail: "no config directory available".into(),
        });
        return;
    };
    if !path.exists() {
        out.push(Check {
            name: "settings",
            status: "info",
            detail: format!("{} (not created yet — defaults apply)", path.display()),
        });
        return;
    }
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).map_err(|e| e.to_string()))
    {
        Ok(_) => out.push(Check {
            name: "settings",
            status: "ok",
            detail: path.display().to_string(),
        }),
        Err(e) => out.push(Check {
            name: "settings",
            status: "fail",
            detail: format!("{} does not parse: {e}", path.display()),
        }),
    }
}

/// Run every check; returns the process exit code (1 only on a hard fail).
pub fn run(json_out: bool) -> i32 {
    let mut checks = Vec::new();
    check_binary(&mut checks);
    check_sidecar(&mut checks);
    check_port(&mut checks);
    check_mcp_registrations(&mut checks);
    check_settings(&mut checks);

    if json_out {
        let arr: Vec<_> = checks
            .iter()
            .map(|c| json!({ "name": c.name, "status": c.status, "detail": c.detail }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "event": "doctor",
                "version": env!("CARGO_PKG_VERSION"),
                "checks": arr,
            }))
            .unwrap_or_default()
        );
    } else {
        for c in &checks {
            let mark = match c.status {
                "ok" => "✓",
                "warn" => "!",
                "fail" => "✗",
                _ => "·",
            };
            println!("  {mark} {:<12} {}", c.name, c.detail);
        }
    }
    if checks.iter().any(|c| c.status == "fail") {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The doctor must degrade to reports, never to a crash: run the full set
    // in whatever state the test machine is in and require the four core
    // check names to be present exactly once each (mcp appears per client).
    #[test]
    fn doctor_always_reports_every_check() {
        let mut checks = Vec::new();
        check_binary(&mut checks);
        check_sidecar(&mut checks);
        check_port(&mut checks);
        check_settings(&mut checks);
        for name in ["binary", "sidecar kit", "node", "port 8765", "settings"] {
            assert_eq!(
                checks.iter().filter(|c| c.name == name).count(),
                1,
                "missing or duplicated check: {name}"
            );
        }
        assert!(checks
            .iter()
            .all(|c| matches!(c.status, "ok" | "warn" | "fail" | "info")));
    }
}
