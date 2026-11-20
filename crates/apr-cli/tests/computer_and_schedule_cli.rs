use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn cli(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_apr"))
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn computer_scaffold_is_portable_credited_and_inert_without_grants() {
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("absent-state");
    let bundle = temporary.path().join("computer bundle");
    let created = success(cli(
        &state,
        &[
            "computer",
            "scaffold",
            bundle.to_str().unwrap(),
            "--app",
            "com.apple.TextEdit",
            "--max-steps",
            "2",
        ],
    ));
    assert_eq!(created["executed"], false);
    assert_eq!(created["granted"], false);
    assert_eq!(created["decision_source"], "caller");
    assert!(
        !state.exists(),
        "scaffolding must not create grants or runtime state"
    );
    let manifest = bundle.join("process.json");
    let admitted = success(cli(
        &state,
        &["process", "validate", manifest.to_str().unwrap()],
    ));
    assert_eq!(admitted["valid"], true);
    assert!(
        fs::read_to_string(bundle.join("LICENSE.upstream"))
            .unwrap()
            .contains("Copyright (c) 2026 Aaron Levin")
    );
    let original = fs::read(&manifest).unwrap();
    let duplicate = cli(
        &state,
        &[
            "computer",
            "scaffold",
            bundle.to_str().unwrap(),
            "--app",
            "com.apple.TextEdit",
        ],
    );
    assert!(!duplicate.status.success());
    assert_eq!(fs::read(&manifest).unwrap(), original);
    let options: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(options["limits"]["max_provider_calls"], 0);
    assert_eq!(options["config"]["run_override_keys"], json!([]));
}

#[test]
fn scheduled_time_requires_explicit_timezone_before_contacting_service() {
    let temporary = tempfile::tempdir().unwrap();
    let input = temporary.path().join("input.json");
    fs::write(&input, "{}").unwrap();
    let output = cli(
        &temporary.path().join("state"),
        &[
            "schedule",
            "create",
            "--attachment",
            "missing-attachment.json",
            "--name",
            "Review",
            "--process-ref",
            "test",
            "--input-file",
            input.to_str().unwrap(),
            "--request-key",
            "test",
            "--at",
            "2026-09-18T09:00:00",
        ],
    );
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "INVALID_INPUT"
    );
}

#[test]
fn service_file_generation_escapes_paths_and_never_installs_or_overwrites() {
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state & percent% dollar$");
    apr_runtime::Runtime::open(&state).unwrap();
    let plist = temporary.path().join("apr.plist");
    let unit = temporary.path().join("apr.service");
    for (platform, output) in [("launchd", &plist), ("systemd", &unit)] {
        let generated = success(cli(
            &state,
            &[
                "schedule",
                "service-file",
                "--platform",
                platform,
                "--output",
                output.to_str().unwrap(),
            ],
        ));
        assert_eq!(generated["installed"], false);
        assert!(
            !cli(
                &state,
                &[
                    "schedule",
                    "service-file",
                    "--platform",
                    platform,
                    "--output",
                    output.to_str().unwrap()
                ]
            )
            .status
            .success()
        );
    }
    let plist_text = fs::read_to_string(plist).unwrap();
    assert!(plist_text.contains("state &amp; percent% dollar$"));
    let unit_text = fs::read_to_string(unit).unwrap();
    assert!(unit_text.contains("state & percent%% dollar$$"));
    assert!(unit_text.contains("--state-dir \""));
    assert!(!unit_text.contains("ExecStart=/bin/sh"));
}
