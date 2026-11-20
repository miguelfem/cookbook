use apr_connectors::mcp::{self, McpBinding, pin_executable};
use apr_connectors::{Auth, Connection, CredentialStore, DispatchError, HttpBroker, Secret};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct EmptyVault;
impl CredentialStore for EmptyVault {
    fn get(&self, _: &str) -> Result<Secret, DispatchError> {
        Err(DispatchError::CredentialUnavailable)
    }
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_apr"))
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
fn python() -> PathBuf {
    let output = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    fs::canonicalize(String::from_utf8(output.stdout).unwrap().trim()).unwrap()
}
fn write_target(path: &Path, python: &Path, body: &str) {
    fs::write(path, format!("#!{}\n{body}", python.display())).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).unwrap();
}
fn prepared_target(
    root: &Path,
    label: &str,
    python: &Path,
    body: &str,
    target_timeout_ms: u64,
    max_payload_bytes: u64,
) -> Connection {
    let target = root.join(format!("{label}-cli"));
    write_target(&target, python, body);
    let spec_path = root.join(format!("{label}-spec.json"));
    let bundle = root.join(format!("{label}-bundle"));
    let schema = json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false});
    fs::write(
        &spec_path,
        serde_json::to_vec(&json!({
            "schema_version":1,"name":"normalize_text","cli_executable":target,
            "cli_args":[],"dependency_files":[],"credential_env":null,
            "input_schema":schema,"output_schema":schema,
            "timeout_ms":target_timeout_ms,"max_input_bytes":max_payload_bytes,
            "max_output_bytes":max_payload_bytes
        }))
        .unwrap(),
    )
    .unwrap();
    let result = success(cli(&[
        "adapter",
        "prepare",
        "--spec",
        spec_path.to_str().unwrap(),
        "--output",
        bundle.to_str().unwrap(),
        "--python",
        python.to_str().unwrap(),
    ]));
    assert_eq!(result["ready"], true);
    let binding = serde_json::from_value(result["binding_proposal"].clone()).unwrap();
    Connection {
        mcp: Some(binding),
        metering: None,
        label: label.into(),
        credential_label: None,
        endpoint: "stdio:local".into(),
        method: "POST".into(),
        auth: Auth::None,
        input_schema: schema,
        output_schema: json!({"type":"object"}),
        timeout_ms: 5000,
        max_response_bytes: 65536,
        loopback_test: false,
    }
}

#[test]
fn adapter_prepare_check_and_reviewed_stdio_dispatch_use_pinned_files() {
    let temporary = tempfile::tempdir().unwrap();
    let python = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    let python = fs::canonicalize(String::from_utf8(python.stdout).unwrap().trim()).unwrap();
    let source_spec = temporary.path().join("source-spec.json");
    let bundle = temporary.path().join("bundle");
    let dependency = temporary.path().join("fixture-data.txt");
    let target_cli = temporary.path().join("synthetic-cli");
    let calls = temporary.path().join("calls.txt");
    fs::write(&dependency, "fixed dependency").unwrap();
    let code = format!(
        "#!{}\n{}",
        python.display(),
        include_str!("../../../integrations/adapters/synthetic_cli.py")
    );
    fs::write(&target_cli, code).unwrap();
    let mut permissions = fs::metadata(&target_cli).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&target_cli, permissions).unwrap();
    let schema = json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false});
    fs::write(
        &source_spec,
        serde_json::to_vec(&json!({
            "schema_version":1,"name":"normalize_text","cli_executable":target_cli,
        "cli_args":[calls],"dependency_files":[dependency],
            "credential_env":null,"input_schema":schema,"output_schema":schema,
            "timeout_ms":2000,"max_input_bytes":4096,"max_output_bytes":4096
        }))
        .unwrap(),
    )
    .unwrap();
    let result = success(cli(&[
        "adapter",
        "prepare",
        "--spec",
        source_spec.to_str().unwrap(),
        "--output",
        bundle.to_str().unwrap(),
        "--python",
        python.to_str().unwrap(),
    ]));
    assert_eq!(result["ready"], true);
    assert_eq!(result["grant_created"], false);
    assert!(!calls.exists(), "prepare must not invoke the target CLI");
    assert_eq!(
        success(cli(&[
            "adapter",
            "check",
            "--directory",
            bundle.to_str().unwrap()
        ]))["ready"],
        true
    );
    assert!(!calls.exists(), "check must not invoke the target CLI");

    let manifest_path = bundle.join("manifest.json");
    let original_manifest = fs::read(&manifest_path).unwrap();
    let mut changed_manifest: Value = serde_json::from_slice(&original_manifest).unwrap();
    changed_manifest["args"][0] = json!("-E");
    fs::write(
        &manifest_path,
        serde_json::to_vec(&changed_manifest).unwrap(),
    )
    .unwrap();
    let changed = success(cli(&[
        "adapter",
        "check",
        "--directory",
        bundle.to_str().unwrap(),
    ]));
    assert_eq!(changed["ready"], false);
    assert!(changed["binding_proposal"].is_null());
    fs::write(&manifest_path, original_manifest).unwrap();

    let binding: McpBinding = serde_json::from_value(result["binding_proposal"].clone()).unwrap();
    let connection = Connection {
        mcp: Some(binding),
        metering: None,
        label: "synthetic adapter".into(),
        credential_label: None,
        endpoint: "stdio:local".into(),
        method: "POST".into(),
        auth: Auth::None,
        input_schema: schema,
        output_schema: json!({"type":"object"}),
        timeout_ms: 5000,
        max_response_bytes: 16384,
        loopback_test: false,
    };
    let discovery = mcp::discover(&connection, &EmptyVault).unwrap();
    assert_eq!(discovery["tools"][0]["name"], "normalize_text");
    assert!(
        !calls.exists(),
        "metadata discovery must not invoke the CLI"
    );
    let result = HttpBroker::new(EmptyVault)
        .dispatch(&connection, &json!({"text":"  MIXED  "}))
        .unwrap();
    assert_eq!(result.output["structuredContent"]["text"], "mixed");
    assert!(calls.exists());

    fs::write(&dependency, "changed dependency").unwrap();
    let check = success(cli(&[
        "adapter",
        "check",
        "--directory",
        bundle.to_str().unwrap(),
    ]));
    assert_eq!(check["ready"], false);
    assert!(
        check["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("dependency"))
    );
    assert!(
        HttpBroker::new(EmptyVault)
            .dispatch(&connection, &json!({"text":"again"}))
            .is_err()
    );
    assert_eq!(fs::read_to_string(&calls).unwrap(), "called");

    fs::write(&dependency, "fixed dependency").unwrap();
    let source = bundle.join("adapter.py");
    let original_source = fs::read(&source).unwrap();
    fs::write(&source, b"raise SystemExit('tampered')\n").unwrap();
    assert_eq!(
        success(cli(&[
            "adapter",
            "check",
            "--directory",
            bundle.to_str().unwrap()
        ]))["ready"],
        false
    );
    assert!(
        mcp::discover(&connection, &EmptyVault).is_err(),
        "pinned bootstrap must reject changed source"
    );
    fs::write(&source, original_source).unwrap();

    let frozen_spec = bundle.join("spec.json");
    let original_spec = fs::read(&frozen_spec).unwrap();
    fs::write(&frozen_spec, b"{\"name\":\"tampered\"}").unwrap();
    assert!(
        mcp::discover(&connection, &EmptyVault).is_err(),
        "pinned bootstrap must reject changed spec"
    );
    fs::write(&frozen_spec, original_spec).unwrap();

    fs::remove_file(&target_cli).unwrap();
    let check = success(cli(&[
        "adapter",
        "check",
        "--directory",
        bundle.to_str().unwrap(),
    ]));
    assert_eq!(check["ready"], false);
    assert!(
        check["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("cli"))
    );
}

#[test]
fn bootstrap_uses_verified_spec_bytes_after_the_path_changes() {
    let temp = tempfile::tempdir().unwrap();
    let python = python();
    let spec = temp.path().join("spec.json");
    let source = temp.path().join("mutate.py");
    fs::write(&spec, r#"{"name":"pinned"}"#).unwrap();
    fs::write(&source, "import json,pathlib,sys\npathlib.Path(sys.argv[1]).write_text('{\"name\":\"changed\"}')\nassert json.loads(PINNED_SPEC_BYTES)['name'] == 'pinned'\nprint('verified bytes used')\n").unwrap();
    let output = Command::new(python)
        .args([
            "-I",
            "-S",
            "-u",
            "-c",
            include_str!("../../../integrations/adapters/bootstrap.py"),
        ])
        .arg(&spec)
        .arg(&source)
        .arg(pin_executable(&spec).unwrap())
        .arg(pin_executable(&source).unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "verified bytes used"
    );
    assert_eq!(fs::read_to_string(spec).unwrap(), r#"{"name":"changed"}"#);
}

#[test]
fn outer_deadline_kills_the_target_cli_in_the_same_process_group() {
    let temp = tempfile::tempdir().unwrap();
    let started = temp.path().join("started");
    let late = temp.path().join("late");
    let body = format!(
        "import json,pathlib,sys,time,os\njson.load(sys.stdin)\npathlib.Path({}).write_text(str(os.getpid()))\ntime.sleep(1.5)\npathlib.Path({}).write_text('survived')\nprint(json.dumps({{'text':'late'}}))\n",
        serde_json::to_string(started.to_str().unwrap()).unwrap(),
        serde_json::to_string(late.to_str().unwrap()).unwrap()
    );
    let mut connection = prepared_target(temp.path(), "slow", &python(), &body, 3000, 4096);
    connection.timeout_ms = 700;
    assert!(
        HttpBroker::new(EmptyVault)
            .dispatch(&connection, &json!({"text":"wait"}))
            .is_err()
    );
    let pid: i32 = fs::read_to_string(&started)
        .expect("target must start before deadline")
        .parse()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_ne!(
        unsafe { libc::kill(pid, 0) },
        0,
        "target CLI survived outer deadline"
    );
    std::thread::sleep(Duration::from_millis(900));
    assert!(!late.exists(), "target performed work after outer deadline");
}

#[test]
fn near_transport_limit_payload_still_fits_the_mcp_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let body = "import json,sys\nvalue=json.load(sys.stdin)\nprint(json.dumps(value,separators=(',',':')))\n";
    let connection = prepared_target(temp.path(), "large", &python(), body, 3000, 60 * 1024);
    let text = "A".repeat(60_000);
    let result = HttpBroker::new(EmptyVault)
        .dispatch(&connection, &json!({"text":text}))
        .unwrap();
    assert_eq!(
        result.output["structuredContent"]["text"]
            .as_str()
            .unwrap()
            .len(),
        60_000
    );
    assert!(result.response_bytes <= 65536);
    let source = temp.path().join("large-spec.json");
    let mut oversized: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    oversized["max_output_bytes"] = json!(65536);
    fs::write(&source, serde_json::to_vec(&oversized).unwrap()).unwrap();
    let oversized_bundle = temp.path().join("oversized-bundle");
    let python_path = python();
    let rejected = cli(&[
        "adapter",
        "prepare",
        "--spec",
        source.to_str().unwrap(),
        "--output",
        oversized_bundle.to_str().unwrap(),
        "--python",
        python_path.to_str().unwrap(),
    ]);
    assert!(
        !rejected.status.success(),
        "unreserved output must be rejected"
    );
}
