//! CLI-surface regressions: the binary must behave like a well-formed unix
//! tool when its output is consumed through a pipe.

/// Rust ignores SIGPIPE at startup by design, which turns `prog | head`
/// into a panic ("failed printing to stdout: Broken pipe") in the default
/// configuration. `harvest` resets the disposition in main() — this test
/// pins that: writing to a pipe with no reader must KILL by SIGPIPE, and
/// stdout errors must never surface as a panic.
#[cfg(unix)]
#[test]
fn closed_pipe_kills_by_sigpipe_not_by_panic() {
    use std::os::unix::io::FromRawFd;
    use std::os::unix::process::ExitStatusExt;

    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
    // Close the read end immediately: every later write is EPIPE- territory.
    assert_eq!(unsafe { libc::close(fds[0]) }, 0, "close(read)");

    // `audit --file …` prints a multi-line markdown report to stdout —
    // guaranteed first-write failure on an unread pipe. (Deliberately uses
    // the sample shipped in the repo: no temp files, no config plumbing.)
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_harvest"))
        .args([
            "audit",
            "--file",
            "examples/output/audit.sample.json",
            "--md",
        ])
        .stdout(unsafe { std::process::Stdio::from(std::fs::File::from_raw_fd(fds[1])) })
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("spawn harvest");
    // fds[1] was moved into Stdio; std closes the parent copy on spawn.

    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !stderr.contains("panicked"),
        "broken pipe must not panic; stderr was: {stderr}"
    );
    assert_eq!(
        out.status.signal(),
        Some(libc::SIGPIPE),
        "expected death by SIGPIPE (141-class), got status {:?}",
        out.status
    );
}

/// `harvest schema` must print one parseable draft-07 document per target,
/// with record types derived from the extraction pipeline. This is the CI
/// gate for data-contract changes, so both the happy path and the error
/// paths (unknown --target, invalid config) are pinned.
fn harvest(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_harvest"))
        .args(args)
        .output()
        .expect("spawn harvest")
}

#[test]
fn schema_command_prints_draft07_per_target() {
    let out = harvest(&["schema", "--config", "examples/config.basic.json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "one doc per target");
    let doc: serde_json::Value = serde_json::from_str(lines[0]).expect("stdout is JSON");
    assert_eq!(doc["$schema"], "http://json-schema.org/draft-07/schema#");
    assert_eq!(doc["title"], "harvest:shop");
    assert_eq!(doc["type"], "object");
    assert_eq!(doc["additionalProperties"], serde_json::json!(false));
    for f in ["title", "price"] {
        assert_eq!(
            doc["properties"][f]["type"], "string",
            "no transforms => text"
        );
        assert!(doc["required"].as_array().unwrap().iter().any(|v| v == f));
    }
}

#[test]
fn schema_command_reflects_pipeline_types() {
    let out = harvest(&[
        "schema",
        "--config",
        "examples/config.pipeline.json",
        "--target",
        "shop",
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).lines().next().unwrap()).unwrap();
    assert_eq!(doc["title"], "harvest:shop", "--target selects exactly one");
    assert_eq!(doc["properties"]["price"]["type"], "number");
    assert_eq!(doc["properties"]["desc"]["type"], "array");
    let chain = doc["properties"]["price"]["harvest:transforms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(
        chain.iter().any(|t| t.starts_with("parse_number")),
        "{chain:?}"
    );
}

#[test]
fn schema_command_errors_are_loud_not_empty() {
    let out = harvest(&[
        "schema",
        "--config",
        "examples/config.basic.json",
        "--target",
        "nope",
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no target `nope`"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
