use apr_core::validate_instance;
use serde_json::json;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn unproductive_schema_cycles_are_rejected_in_a_bounded_subprocess() {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "schema_cycle_probe", "--nocapture"])
        .env("APR_SCHEMA_CYCLE_PROBE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "recursive-schema validation aborted or accepted a cycle: {status}"
            );
            break;
        }
        if started.elapsed() > Duration::from_secs(3) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("recursive-schema validation exceeded three seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn schema_cycle_probe() {
    if std::env::var_os("APR_SCHEMA_CYCLE_PROBE").is_none() {
        return;
    }
    let schema = json!({"$defs": {
        "a": {"$ref": "#/$defs/b"}, "b": {"allOf": [{"$ref": "#/$defs/a"}]}
    }, "$ref": "#/$defs/a"});
    assert!(validate_instance(&schema, &json!({})).is_err());
}

#[test]
fn repeated_references_have_a_finite_expansion_budget() {
    let mut definitions = serde_json::Map::new();
    definitions.insert("x0".into(), json!({"type": "object"}));
    for index in 1..=13 {
        let reference = format!("#/$defs/x{}", index - 1);
        definitions.insert(
            format!("x{index}"),
            json!({"allOf": [{"$ref": reference}, {"$ref": reference}]}),
        );
    }
    let schema = json!({"$defs": definitions, "$ref": "#/$defs/x13"});
    assert!(validate_instance(&schema, &json!({})).is_err());
}
