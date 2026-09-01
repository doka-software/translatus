//! The exit code has to agree with the summary.
//!
//! A run that reaches the end having failed every segment still writes an output
//! file and a resumable cache, so it is not an error — but it is not a success
//! either. It used to exit 0 while printing `done:` and leaving a file full of
//! untranslated source text, which meant a script or CI job that checks only the
//! exit code read a total failure as a clean run.
//!
//! Everything here is offline: the "provider" is a loopback socket that answers
//! every request with 500.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;

/// A loopback HTTP server that fails every request. Returns its base URL and
/// keeps serving on a background thread until the test ends.
fn spawn_failing_provider() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            // Read just enough to let the client finish writing.
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ = s.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            );
            let _ = s.flush();
            drop::<TcpStream>(s);
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("et_exit_{name}"))
}

#[test]
fn total_translation_failure_exits_nonzero() {
    let input = tmp("in.txt");
    let output = tmp("out.txt");
    let job = tmp("out.etjob");
    for p in [&input, &output, &job] {
        let _ = std::fs::remove_file(p);
    }
    std::fs::write(&input, "Hello world.\n\nSecond paragraph.\n").unwrap();

    let base = spawn_failing_provider();
    let out = Command::new(env!("CARGO_BIN_EXE_translatus"))
        .args(["translate"])
        .arg(&input)
        .args([
            "--to",
            "English",
            "--provider",
            "openai",
            "--model",
            "gpt-5.4-mini",
        ])
        .env("OPENAI_API_KEY", "not-a-real-key")
        .args(["--base-url", &base])
        .arg("--output")
        .arg(&output)
        .arg("--job")
        .arg(&job)
        .output()
        .expect("run translatus");

    let stdout = String::from_utf8_lossy(&out.stdout);
    for p in [&input, &output, &job] {
        let _ = std::fs::remove_file(p);
    }

    assert!(
        stdout.contains("failed"),
        "the summary should report failures: {stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "every segment failed, so the exit code must not say success.\nstdout:\n{stdout}"
    );
}

/// The resume screens read the job's chapter statuses as "already paid for".
/// A chapter whose calls all failed is NOT in the cache and WILL be re-billed,
/// so it must not be recorded as done — otherwise a book that failed against an
/// unreachable endpoint would come back priced as almost free.
#[test]
fn a_failed_chapter_is_not_recorded_as_finished() {
    let input = tmp("resume_in.txt");
    let output = tmp("resume_out.txt");
    let job = tmp("resume_out.etjob");
    for p in [&input, &output, &job] {
        let _ = std::fs::remove_file(p);
    }
    std::fs::write(&input, "Hello world.\n\nSecond paragraph.\n").unwrap();

    let base = spawn_failing_provider();
    let _ = Command::new(env!("CARGO_BIN_EXE_translatus"))
        .args(["translate"])
        .arg(&input)
        .args([
            "--to",
            "English",
            "--provider",
            "openai",
            "--model",
            "gpt-5.4-mini",
        ])
        .env("OPENAI_API_KEY", "not-a-real-key")
        .args(["--base-url", &base])
        .arg("--output")
        .arg(&output)
        .arg("--job")
        .arg(&job)
        .output()
        .expect("run translatus");

    let store = et_core::job::JobStore::open_readonly(&job)
        .expect("the job file is readable")
        .expect("the run created a job file");
    let done = store.done_chapters().expect("chapter statuses");
    assert!(
        done.is_empty(),
        "nothing was cached, so nothing may count as done: {done:?}"
    );
    assert_eq!(
        store.total_chapters().expect("meta"),
        Some(1),
        "the chapter count is recorded so a resume can say 'n of m'"
    );
    drop(store);
    for p in [&input, &output, &job] {
        let _ = std::fs::remove_file(p);
    }
}

/// …and the other direction: a run that really finished is recorded, or the
/// resume screens would never show anything.
#[test]
fn a_successful_chapter_is_recorded_as_finished() {
    let input = tmp("resume_ok_in.txt");
    let output = tmp("resume_ok_out.txt");
    let job = tmp("resume_ok_out.etjob");
    for p in [&input, &output, &job] {
        let _ = std::fs::remove_file(p);
    }
    std::fs::write(&input, "Hello world.\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_translatus"))
        .args(["translate"])
        .arg(&input)
        .args(["--to", "English"]) // mock provider by default
        .arg("--output")
        .arg(&output)
        .arg("--job")
        .arg(&job)
        .output()
        .expect("run translatus");
    assert_eq!(out.status.code(), Some(0));

    let store = et_core::job::JobStore::open_readonly(&job)
        .expect("readable")
        .expect("the run created a job file");
    assert_eq!(store.done_chapters().expect("statuses"), vec![0]);
    assert_eq!(store.total_chapters().expect("meta"), Some(1));
    drop(store);
    for p in [&input, &output, &job] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn a_clean_run_still_exits_zero() {
    let input = tmp("ok_in.txt");
    let output = tmp("ok_out.txt");
    let job = tmp("ok_out.etjob");
    for p in [&input, &output, &job] {
        let _ = std::fs::remove_file(p);
    }
    std::fs::write(&input, "Hello world.\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_translatus"))
        .args(["translate"])
        .arg(&input)
        .args(["--to", "English"]) // mock provider by default
        .arg("--output")
        .arg(&output)
        .arg("--job")
        .arg(&job)
        .output()
        .expect("run translatus");

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    for p in [&input, &output, &job] {
        let _ = std::fs::remove_file(p);
    }
    assert_eq!(
        out.status.code(),
        Some(0),
        "a fully successful run must stay exit 0.\nstdout:\n{stdout}"
    );
}

/// The refusal has to happen before any work, and it has to be an error rather
/// than a silent fallback to the wrong wire format.
#[test]
fn anthropic_provider_is_refused_up_front() {
    let input = tmp("anth_in.txt");
    let _ = std::fs::remove_file(&input);
    std::fs::write(&input, "Hello world.\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_translatus"))
        .args(["translate"])
        .arg(&input)
        .args([
            "--to",
            "English",
            "--provider",
            "anthropic",
            "--model",
            "claude-sonnet-5",
        ])
        .env("ANTHROPIC_API_KEY", "not-a-real-key")
        .output()
        .expect("run translatus");
    let _ = std::fs::remove_file(&input);

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.code() != Some(0),
        "an unimplemented provider must not exit 0: {combined}"
    );
    assert!(
        combined.contains("not implemented"),
        "the user needs to be told why: {combined}"
    );
}
