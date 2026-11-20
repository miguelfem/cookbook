#![cfg(unix)]

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use apr_runtime::artifacts::ArtifactStore;
use apr_runtime::worker::{
    InterpreterPin, WorkerError, WorkerInputArtifact, WorkerLimits, WorkerRequest, execute,
    substitute_artifacts,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn interpreter() -> InterpreterPin {
    let path = std::env::var_os("APR_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            [
                "/Library/Frameworks/Python.framework/Versions/3.11/bin/python3",
                "/usr/bin/python3",
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .expect(
                "worker tests need an existing Python 3; set APR_TEST_PYTHON to an absolute path",
            )
        });
    InterpreterPin::inspect(&path).unwrap()
}

fn run(
    code: &[u8],
    input: &Value,
    config: &Value,
    limits: WorkerLimits,
) -> Result<apr_runtime::worker::WorkerOutput, WorkerError> {
    execute(
        WorkerRequest {
            run_id: None,
            interpreter: &interpreter(),
            code,
            input,
            config,
            artifacts: &[],
            limits,
        },
        &AtomicBool::new(false),
    )
}

fn run_with_artifacts(
    code: &[u8],
    artifacts: &[WorkerInputArtifact<'_>],
    config: &Value,
    limits: WorkerLimits,
) -> Result<apr_runtime::worker::WorkerOutput, WorkerError> {
    execute(
        WorkerRequest {
            run_id: None,
            interpreter: &interpreter(),
            code,
            input: &json!({}),
            config,
            artifacts,
            limits,
        },
        &AtomicBool::new(false),
    )
}

#[test]
fn runtime_scope_does_not_change_legacy_worker_input_envelope() {
    let output = execute(
        WorkerRequest {
            run_id: Some("run_host_issued"),
            interpreter: &interpreter(),
            code: br#"import json,os,sys
r=json.load(sys.stdin)
assert set(r) == {'protocol_version','input','config','artifacts'}
assert os.environ['APR_RUN_ID'] == 'run_host_issued'
assert r['input']['run_id'] == 'run_untrusted_input'
print(json.dumps({'protocol_version':'0.1','output':{'valid':True}}))
"#,
            input: &json!({"run_id":"run_untrusted_input"}),
            config: &json!({}),
            artifacts: &[],
            limits: WorkerLimits::default(),
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output.output, json!({"valid":true}));
}

#[test]
fn binary_artifacts_use_readonly_files_and_exact_placeholders_without_large_json_payloads() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let bytes = vec![0x81; 128 * 1024];
    let reference = store
        .put(&bytes, "application/octet-stream", 1024 * 1024)
        .unwrap();
    let output = run_with_artifacts(br#"import json,os,stat,sys
r=json.load(sys.stdin)
assert len(r['artifacts']) == 1
item=next(iter(r['artifacts'].values()))
assert stat.S_IMODE(os.stat(item['path']).st_mode) == 0o400
data=open(item['path'],'rb').read()
assert len(data) == item['reference']['bytes']
open('result.bin','wb').write(data + b'\x00tail')
json.dump({'protocol_version':'0.1','output':{'nested':[{'worker_artifact':'result'}], 'ordinary':{'worker_artifact':'literal','label':'preserved'},'cwd':os.getcwd()},'artifacts':[{'name':'result','path':'result.bin','media_type':'application/octet-stream'}]},sys.stdout)
"#, &[WorkerInputArtifact { reference:&reference, bytes:&bytes }],&json!({}),WorkerLimits::default()).unwrap();
    assert!(output.stdout_bytes < 1024);
    assert_eq!(output.artifacts.len(), 1);
    assert_eq!(&output.artifacts[0].bytes[..bytes.len()], &bytes);
    assert_eq!(&output.artifacts[0].bytes[bytes.len()..], b"\x00tail");
    assert_eq!(store.read(&reference, 1024 * 1024).unwrap(), bytes);
    assert!(!Path::new(output.output["cwd"].as_str().unwrap()).exists());
    let generated = &output.artifacts[0];
    let published = store
        .put(&generated.bytes, &generated.media_type, 1024 * 1024)
        .unwrap();
    let output = substitute_artifacts(
        output.output,
        &BTreeMap::from([(generated.name.clone(), published.clone())]),
    )
    .unwrap();
    assert_eq!(
        output["nested"][0],
        serde_json::to_value(published).unwrap()
    );
    assert_eq!(
        output["ordinary"],
        json!({"worker_artifact":"literal","label":"preserved"})
    );
}

#[test]
fn artifact_declarations_reject_traversal_reserved_files_duplicates_and_unknown_placeholders() {
    let code = br#"import json,sys
r=json.load(sys.stdin)
open('result.bin','wb').write(b'ok')
json.dump({'protocol_version':'0.1','output':r['config'].get('output',{}),'artifacts':r['config']['artifacts']},sys.stdout)
"#;
    for path in [
        "../escape.bin",
        "/tmp/escape.bin",
        "nested/file.bin",
        ".",
        "script.py",
        "input-pinned",
        "back\\slash",
    ] {
        let config = json!({"artifacts":[{"name":"result","path":path,"media_type":"text/plain"}]});
        assert_eq!(
            run_with_artifacts(code, &[], &config, WorkerLimits::default()).unwrap_err(),
            WorkerError::ArtifactProtocol,
            "{path}"
        );
    }
    for declarations in [
        json!([{"name":"same","path":"result.bin","media_type":"text/plain"},{"name":"same","path":"other.bin","media_type":"text/plain"}]),
        json!([{"name":"one","path":"result.bin","media_type":"text/plain"},{"name":"two","path":"result.bin","media_type":"text/plain"}]),
        json!([{"name":"bad/name","path":"result.bin","media_type":"text/plain"}]),
        json!([{"name":"result","path":"result.bin","media_type":"invalid"}]),
    ] {
        assert_eq!(
            run_with_artifacts(
                code,
                &[],
                &json!({"artifacts":declarations}),
                WorkerLimits::default()
            )
            .unwrap_err(),
            WorkerError::ArtifactProtocol
        );
    }
    for output in [
        json!({"worker_artifact":"absent"}),
        json!({"worker_artifact":7}),
    ] {
        assert_eq!(
            run_with_artifacts(
                code,
                &[],
                &json!({"artifacts":[],"output":output}),
                WorkerLimits::default()
            )
            .unwrap_err(),
            WorkerError::ArtifactProtocol
        );
        assert_eq!(
            substitute_artifacts(output, &BTreeMap::new()).unwrap_err(),
            WorkerError::ArtifactProtocol
        );
    }
}

#[test]
fn artifact_collection_never_follows_symlinks_hardlinks_or_special_files() {
    let directory = tempfile::tempdir().unwrap();
    let external = directory.path().join("external.txt");
    std::fs::write(&external, "private outside data").unwrap();
    for operation in [
        "os.symlink(r['config']['external'],'result.bin')",
        "os.link(r['config']['external'],'result.bin')",
        "os.mkfifo('result.bin')",
        "os.mkdir('result.bin')",
    ] {
        let code = format!(
            "import json,os,sys\nr=json.load(sys.stdin)\n{operation}\njson.dump({{'protocol_version':'0.1','output':{{}},'artifacts':[{{'name':'result','path':'result.bin','media_type':'text/plain'}}]}},sys.stdout)\n"
        );
        let error = run_with_artifacts(
            code.as_bytes(),
            &[],
            &json!({"external":external}),
            WorkerLimits::default(),
        )
        .unwrap_err();
        assert_eq!(error, WorkerError::ArtifactIntegrity);
        assert!(!error.to_string().contains("private outside data"));
        assert_eq!(
            std::fs::read_to_string(&external).unwrap(),
            "private outside data"
        );
    }
}

#[test]
fn artifact_limits_apply_before_large_allocations_and_to_combined_input_output() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let bytes = b"12345678";
    let reference = store.put(bytes, "text/plain", 1024).unwrap();
    let code=br#"import json,sys
r=json.load(sys.stdin)
with open('result.bin','wb') as f: f.truncate(r['config']['size'])
json.dump({'protocol_version':'0.1','output':{},'artifacts':[{'name':'result','path':'result.bin','media_type':'application/octet-stream'}]},sys.stdout)
"#;
    let limits = WorkerLimits {
        max_artifact_bytes: 12,
        ..WorkerLimits::default()
    };
    assert_eq!(
        run_with_artifacts(
            code,
            &[WorkerInputArtifact {
                reference: &reference,
                bytes
            }],
            &json!({"size":8}),
            limits
        )
        .unwrap_err(),
        WorkerError::ArtifactLimit
    );
    assert_eq!(
        run_with_artifacts(
            code,
            &[],
            &json!({"size":64*1024*1024+1}),
            WorkerLimits::default()
        )
        .unwrap_err(),
        WorkerError::ArtifactLimit
    );
    let invalid = WorkerLimits {
        max_artifact_bytes: 64 * 1024 * 1024 + 1,
        ..WorkerLimits::default()
    };
    assert_eq!(
        run_with_artifacts(code, &[], &json!({}), invalid).unwrap_err(),
        WorkerError::InvalidLimits
    );
    let declarations:Vec<Value>=(0..33).map(|i|json!({"name":format!("f{i}"),"path":format!("f{i}.bin"),"media_type":"text/plain"})).collect();
    let many = br#"import json,sys
r=json.load(sys.stdin)
json.dump({'protocol_version':'0.1','output':{},'artifacts':r['config']['files']},sys.stdout)
"#;
    assert_eq!(
        run_with_artifacts(
            many,
            &[],
            &json!({"files":declarations}),
            WorkerLimits::default()
        )
        .unwrap_err(),
        WorkerError::ArtifactLimit
    );
    let tiny = WorkerLimits {
        max_artifact_bytes: 4,
        ..WorkerLimits::default()
    };
    assert_eq!(
        run_with_artifacts(
            b"raise RuntimeError()",
            &[WorkerInputArtifact {
                reference: &reference,
                bytes
            }],
            &json!({}),
            tiny
        )
        .unwrap_err(),
        WorkerError::ArtifactLimit
    );
}

#[test]
fn artifact_inputs_are_verified_before_starting_the_native_child() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let reference = store.put(b"expected", "text/plain", 1024).unwrap();
    assert_eq!(
        run_with_artifacts(
            b"raise RuntimeError()",
            &[WorkerInputArtifact {
                reference: &reference,
                bytes: b"modified"
            }],
            &json!({}),
            WorkerLimits::default()
        )
        .unwrap_err(),
        WorkerError::ArtifactIntegrity
    );
    let duplicate = [
        WorkerInputArtifact {
            reference: &reference,
            bytes: b"expected",
        },
        WorkerInputArtifact {
            reference: &reference,
            bytes: b"expected",
        },
    ];
    assert_eq!(
        run_with_artifacts(
            b"raise RuntimeError()",
            &duplicate,
            &json!({}),
            WorkerLimits::default()
        )
        .unwrap_err(),
        WorkerError::ArtifactProtocol
    );
}

#[test]
fn artifact_reads_stay_on_the_pinned_work_directory_after_child_renames_it() {
    let directory = tempfile::tempdir().unwrap();
    let moved = directory.path().join("moved-work");
    let code=br#"import json,os,sys
r=json.load(sys.stdin)
open('result.bin','wb').write(b'original')
original=os.getcwd()
os.rename(original,r['config']['moved'])
os.mkdir(original)
open(os.path.join(original,'result.bin'),'wb').write(b'replacement')
json.dump({'protocol_version':'0.1','output':{},'artifacts':[{'name':'result','path':'result.bin','media_type':'text/plain'}]},sys.stdout)
"#;
    let result =
        run_with_artifacts(code, &[], &json!({"moved":moved}), WorkerLimits::default()).unwrap();
    assert_eq!(result.artifacts[0].bytes, b"original");
    assert_eq!(
        std::fs::read(moved.join("result.bin")).unwrap(),
        b"original"
    );
}

#[test]
fn artifact_file_count_limit_covers_inputs_and_outputs_together() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let bytes: Vec<Vec<u8>> = (0..32).map(|i| vec![i]).collect();
    let references: Vec<_> = bytes
        .iter()
        .map(|bytes| store.put(bytes, "application/octet-stream", 64).unwrap())
        .collect();
    let inputs: Vec<_> = references
        .iter()
        .zip(&bytes)
        .map(|(reference, bytes)| WorkerInputArtifact { reference, bytes })
        .collect();
    let code=br#"import json,sys
json.load(sys.stdin)
open('result.bin','wb').write(b'')
json.dump({'protocol_version':'0.1','output':{},'artifacts':[{'name':'result','path':'result.bin','media_type':'text/plain'}]},sys.stdout)
"#;
    assert_eq!(
        run_with_artifacts(code, &inputs, &json!({}), WorkerLimits::default()).unwrap_err(),
        WorkerError::ArtifactLimit
    );
}

#[test]
fn a_real_python_worker_receives_versioned_input_and_returns_typed_output() {
    let code = br#"import json, os, stat, sys
request = json.load(sys.stdin)
assert request['protocol_version'] == '0.1'
sys.stderr.write('ok\n')
json.dump({'protocol_version':'0.1','output':{
    'total':request['input']['amount'] + request['config']['increment'],
    'literal':request['input']['literal'], 'cwd':os.getcwd(),
    'mode':stat.S_IMODE(os.stat('.').st_mode),
    'group':os.getpgrp(), 'pid':os.getpid(), 'files':os.listdir('.')
}},sys.stdout)
"#;
    let output = run(
        code,
        &json!({"amount":4,"literal":"$(touch should-not-exist); echo unsafe"}),
        &json!({"increment":3}),
        WorkerLimits::default(),
    )
    .unwrap();
    assert_eq!(output.output["total"], 7);
    assert_eq!(
        output.output["literal"],
        "$(touch should-not-exist); echo unsafe"
    );
    assert_eq!(output.output["mode"], 0o700);
    assert_eq!(output.output["group"], output.output["pid"]);
    assert_eq!(output.output["files"], json!(["script.py"]));
    assert_eq!(output.stderr_bytes, 3);
    assert!(output.stdout_bytes > 0);
    assert!(
        !Path::new(output.output["cwd"].as_str().unwrap()).exists(),
        "private work directory survived execution"
    );
}

#[test]
fn child_generated_errors_never_reveal_stderr_or_output_payloads() {
    let marker = "sensitive-worker-payload";
    let error = run(
        br#"import sys
sys.stderr.write('sensitive-worker-payload')
raise SystemExit(7)
"#,
        &json!({}),
        &json!({}),
        WorkerLimits::default(),
    )
    .unwrap_err();
    assert_eq!(error, WorkerError::Exit { code: Some(7) });
    assert!(!error.to_string().contains(marker));
    assert!(!format!("{error:?}").contains(marker));
    let error = run(
        b"print('sensitive-worker-payload')",
        &json!({}),
        &json!({}),
        WorkerLimits::default(),
    )
    .unwrap_err();
    assert_eq!(error, WorkerError::Protocol);
    assert!(!error.to_string().contains(marker));
}

#[test]
fn fresh_environment_omits_parent_secrets_and_interpreter_loader_variables() {
    let fixture = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "worker_environment_fixture", "--nocapture"])
        .env("APR_WORKER_ENV_TEST", "1")
        .env(
            "APR_WORKER_SECRET_SENTINEL",
            "synthetic-secret-that-must-not-reach-code",
        )
        .env("PYTHONHOME", fixture.path().join("missing-python-home"))
        .env("PYTHONPATH", fixture.path())
        .env("PYTHONSTARTUP", fixture.path().join("startup.py"))
        .env("LD_LIBRARY_PATH", fixture.path())
        .env("DYLD_LIBRARY_PATH", fixture.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "environment subprocess failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn worker_environment_fixture() {
    if std::env::var_os("APR_WORKER_ENV_TEST").is_none() {
        return;
    }
    let output = run(
        br#"import json, os, sys
json.load(sys.stdin)
json.dump({'protocol_version':'0.1','output':{'keys':sorted(os.environ)}},sys.stdout)
"#,
        &json!({}),
        &json!({}),
        WorkerLimits::default(),
    )
    .unwrap();
    let mut expected = vec!["HOME", "LC_ALL", "PATH", "TEMP", "TMP", "TMPDIR"];
    // Framework Python on macOS may synthesize this CoreFoundation locale key;
    // it is not copied from the parent's environment or provided by APR.
    if cfg!(target_os = "macos") {
        expected.push("__CF_USER_TEXT_ENCODING");
    }
    assert_eq!(output.output["keys"], json!(expected));
}

#[test]
fn stdout_and_stderr_floods_are_stopped_with_fixed_diagnostics() {
    for (code, expected) in [
        (
            b"import os\nwhile True: os.write(1,b'sensitive-output'*4096)".as_slice(),
            WorkerError::StdoutLimit,
        ),
        (
            b"import os\nwhile True: os.write(2,b'sensitive-output'*4096)".as_slice(),
            WorkerError::StderrLimit,
        ),
    ] {
        let limits = WorkerLimits {
            max_stdout_bytes: 1024,
            max_stderr_bytes: 1024,
            ..WorkerLimits::default()
        };
        let started = Instant::now();
        let error = run(code, &json!({}), &json!({}), limits).unwrap_err();
        assert_eq!(error, expected);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!error.to_string().contains("sensitive-output"));
    }
}

const SPAWN_CHILD: &[u8] = br#"import json, os, subprocess, sys, time
request=json.load(sys.stdin)
child=subprocess.Popen([sys.executable,'-I','-c','import time; time.sleep(60)'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
with open(request['input']['marker'],'w') as output:
    json.dump({'parent':os.getpid(),'child':child.pid,'group':os.getpgrp()},output)
if request['input'].get('finish'):
    print(json.dumps({'protocol_version':'0.1','output':{'finished':True}}))
else:
    time.sleep(60)
"#;

fn assert_terminated(pid: u64) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "stat="])
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&output.stdout);
        // A descendant zombie is already terminated and belongs to its OS
        // adopter; only the direct child can and must be reaped by this worker.
        if !output.status.success() || status.trim().is_empty() || status.trim().starts_with('Z') {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "owned fixture process {pid} remained alive"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_group_cleaned(marker: &Path) {
    let ids: Value = serde_json::from_slice(&std::fs::read(marker).unwrap()).unwrap();
    assert_eq!(ids["parent"], ids["group"]);
    for key in ["parent", "child"] {
        assert_terminated(ids[key].as_u64().unwrap());
    }
    // SAFETY: WNOHANG cannot block; this exact child PID belongs to the fixture.
    let result = unsafe {
        libc::waitpid(
            ids["parent"].as_i64().unwrap() as libc::pid_t,
            std::ptr::null_mut(),
            libc::WNOHANG,
        )
    };
    assert_eq!(result, -1, "worker left its leader unreaped");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[test]
fn timeout_kills_the_process_group_and_reaps_its_leader() {
    let fixture = tempfile::tempdir().unwrap();
    let marker = fixture.path().join("pids.json");
    let limits = WorkerLimits {
        timeout: Duration::from_millis(500),
        ..WorkerLimits::default()
    };
    let started = Instant::now();
    assert_eq!(
        run(SPAWN_CHILD, &json!({"marker":marker}), &json!({}), limits).unwrap_err(),
        WorkerError::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_group_cleaned(&marker);
}

#[test]
fn cancellation_terminates_live_descendants_and_pre_cancelled_work_never_starts() {
    let fixture = tempfile::tempdir().unwrap();
    let marker = fixture.path().join("pids.json");
    let cancelled = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| {
        let flag = Arc::clone(&cancelled);
        let marker = &marker;
        scope.spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !marker.exists() {
                assert!(Instant::now() < deadline, "worker never dispatched");
                std::thread::sleep(Duration::from_millis(10));
            }
            flag.store(true, Ordering::Release);
        });
        let error = execute(
            WorkerRequest {
                run_id: None,
                interpreter: &interpreter(),
                code: SPAWN_CHILD,
                input: &json!({"marker":marker}),
                config: &json!({}),
                artifacts: &[],
                limits: WorkerLimits::default(),
            },
            &cancelled,
        )
        .unwrap_err();
        assert_eq!(error, WorkerError::Cancelled);
    });
    assert_group_cleaned(&marker);
    std::fs::remove_file(&marker).unwrap();
    let error = execute(
        WorkerRequest {
            run_id: None,
            interpreter: &interpreter(),
            code: SPAWN_CHILD,
            input: &json!({"marker":marker}),
            config: &json!({}),
            artifacts: &[],
            limits: WorkerLimits::default(),
        },
        &cancelled,
    )
    .unwrap_err();
    assert_eq!(error, WorkerError::Cancelled);
    assert!(!marker.exists());
}

#[test]
fn successful_workers_cannot_leave_background_group_members_running() {
    let fixture = tempfile::tempdir().unwrap();
    let marker = fixture.path().join("pids.json");
    let output = run(
        SPAWN_CHILD,
        &json!({"marker":marker,"finish":true}),
        &json!({}),
        WorkerLimits::default(),
    )
    .unwrap();
    assert_eq!(output.output, json!({"finished":true}));
    assert_group_cleaned(&marker);
}

#[test]
fn nonblocking_streams_handle_stdin_and_output_backpressure_together() {
    let code = br#"import json, sys
sys.stdout.write(json.dumps({'protocol_version':'0.1','output':{'data':'y'*30000}}))
sys.stdout.flush()
sys.stderr.write('z'*7000)
sys.stderr.flush()
request=json.load(sys.stdin)
assert len(request['input']['large']) == 100000
"#;
    let output = run(
        code,
        &json!({"large":"x".repeat(100_000)}),
        &json!({}),
        WorkerLimits::default(),
    )
    .unwrap();
    assert_eq!(output.output["data"].as_str().unwrap().len(), 30_000);
    assert_eq!(output.stderr_bytes, 7000);
}

#[test]
fn malformed_or_ambiguous_worker_protocol_is_rejected() {
    for output in [
        "{}",
        "[]",
        "{\"protocol_version\":\"9.9\",\"output\":{}}",
        "{\"protocol_version\":\"0.1\",\"output\":{},\"unexpected\":true}",
        "{\"protocol_version\":\"0.1\",\"output\":{},\"output\":1}",
        "{\"protocol_version\":\"0.1\",\"output\":{}} {}",
    ] {
        let code = format!(
            "import sys\nsys.stdout.write({})",
            serde_json::to_string(output).unwrap()
        );
        assert_eq!(
            run(
                code.as_bytes(),
                &json!({}),
                &json!({}),
                WorkerLimits::default()
            )
            .unwrap_err(),
            WorkerError::Protocol
        );
    }
}

#[test]
fn changed_interpreter_identity_is_rejected_before_any_code_dispatch() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("approved-python");
    std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pin = InterpreterPin::inspect(&path).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let original_modified = file.metadata().unwrap().modified().unwrap();
    std::fs::write(&path, b"#!/bin/sh\nexit 9\n").unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(original_modified))
        .unwrap();
    let error = execute(
        WorkerRequest {
            run_id: None,
            interpreter: &pin,
            code: b"print('unreachable')",
            input: &json!({}),
            config: &json!({}),
            artifacts: &[],
            limits: WorkerLimits::default(),
        },
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error, WorkerError::InterpreterChanged);
    assert_eq!(
        InterpreterPin::inspect(Path::new("python3")).unwrap_err(),
        WorkerError::InvalidInterpreter
    );
    assert_eq!(
        InterpreterPin::inspect(&fixture.path().join("missing")).unwrap_err(),
        WorkerError::InvalidInterpreter
    );
}

#[test]
fn pinned_venv_symlink_imports_its_installed_package_and_rejects_retargeting() {
    let fixture = tempfile::tempdir().unwrap();
    let venv = fixture.path().join("env");
    let status = Command::new(&interpreter().path)
        .args(["-m", "venv", "--without-pip", "--symlinks"])
        .arg(&venv)
        .status()
        .unwrap();
    assert!(status.success());
    let python = venv.join("bin/python3");
    assert!(python.symlink_metadata().unwrap().file_type().is_symlink());
    let purelib = Command::new(&python)
        .args([
            "-I",
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .unwrap();
    assert!(purelib.status.success());
    let site_packages = PathBuf::from(String::from_utf8(purelib.stdout).unwrap().trim());
    std::fs::write(
        site_packages.join("apr_venv_probe.py"),
        "VALUE = 'venv-only'\n",
    )
    .unwrap();

    let pin = InterpreterPin::inspect(&python).unwrap();
    assert_ne!(pin.path, pin.resolved_path);
    let code = b"import json, sys, apr_venv_probe\nprint(json.dumps({'protocol_version':'0.1','output':{'value':apr_venv_probe.VALUE,'prefix':sys.prefix,'base_prefix':sys.base_prefix}}))\n";
    let input = json!({});
    let config = json!({});
    let request = || WorkerRequest {
        run_id: None,
        interpreter: &pin,
        code,
        input: &input,
        config: &config,
        artifacts: &[],
        limits: WorkerLimits::default(),
    };
    let output = execute(request(), &AtomicBool::new(false)).unwrap();
    assert_eq!(output.output["value"], "venv-only");
    assert_eq!(
        PathBuf::from(output.output["prefix"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        venv.canonicalize().unwrap()
    );
    assert_ne!(output.output["prefix"], output.output["base_prefix"]);

    std::fs::remove_file(&python).unwrap();
    symlink("/bin/sh", &python).unwrap();
    assert_eq!(
        execute(request(), &AtomicBool::new(false)).unwrap_err(),
        WorkerError::InterpreterChanged
    );
}

#[test]
fn source_input_and_resource_limits_are_checked_before_dispatch() {
    assert_eq!(
        run(
            &vec![b'x'; 1024 * 1024 + 1],
            &json!({}),
            &json!({}),
            WorkerLimits::default()
        )
        .unwrap_err(),
        WorkerError::InvalidCode
    );
    assert_eq!(
        run(&[0xff], &json!({}), &json!({}), WorkerLimits::default()).unwrap_err(),
        WorkerError::InvalidCode
    );
    let limits = WorkerLimits {
        max_input_bytes: 64,
        ..WorkerLimits::default()
    };
    assert_eq!(
        run(
            b"raise RuntimeError('must not execute')",
            &json!({"value":"x".repeat(100)}),
            &json!({}),
            limits
        )
        .unwrap_err(),
        WorkerError::InputLimit
    );
    let limits = WorkerLimits {
        timeout: Duration::ZERO,
        ..WorkerLimits::default()
    };
    assert_eq!(
        run(
            b"raise RuntimeError('must not execute')",
            &json!({}),
            &json!({}),
            limits
        )
        .unwrap_err(),
        WorkerError::InvalidLimits
    );
}
