//! Admission and offline worker contracts; no browser or desktop is launched.
use apr_runtime::browser::{BrowserOptions, WORKER_SOURCE, manifest, scaffold};
use apr_runtime::computer::{ComputerOptions, DecisionSource};
use serde_json::json;
use std::path::Path;

fn options() -> BrowserOptions {
    BrowserOptions {
        computer: ComputerOptions {
            max_steps: 1,
            success_text: Some("Review complete".into()),
            ..ComputerOptions::default()
        },
        start_url: "https://example.org/review".into(),
        allowed_origins: vec!["https://example.org".into()],
        browser_executable: std::env::current_exe().unwrap(),
        recorded_actions: Some(vec![
            json!({"action":"click","label":"Review","expected_text":"Review complete"}),
        ]),
    }
}

#[test]
fn recorded_graph_freezes_policy_keeps_individual_effects_and_uses_no_model() {
    let process = manifest(&options()).unwrap();
    apr_core::admit_value(&process).unwrap();
    assert_eq!(process["limits"]["max_provider_calls"], 0);
    assert_eq!(process["limits"]["max_agent_requests"], 1); // exception-only review
    assert_eq!(
        process["config"]["schema"]["const"],
        process["config"]["defaults"]
    );
    assert_eq!(
        process["config"]["defaults"]["allowed_origins"],
        json!(["https://example.org:443"])
    );
    let steps = process["steps"].as_array().unwrap();
    assert!(steps.iter().all(|step| step["retry"]["max_attempts"] == 1));
    assert_eq!(
        steps.iter().find(|step| step["id"] == "decide_0").unwrap()["kind"],
        "builtin"
    );
    assert_eq!(
        steps.iter().find(|step| step["id"] == "act_0").unwrap()["effect"],
        "write"
    );
    assert!(steps.iter().any(|step| step["id"] == "close_browser"));
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("browser");
    assert_eq!(
        scaffold(&destination, &options()).unwrap()["executed"],
        false
    );
    assert_eq!(
        std::fs::read_to_string(destination.join("browser_worker.py")).unwrap(),
        WORKER_SOURCE
    );
    assert!(scaffold(&destination, &options()).is_err());
}

#[test]
fn browser_scope_cannot_be_a_wildcard_credentials_or_an_unlisted_origin() {
    for url in [
        "file:///etc/passwd",
        "https://user:secret@example.org/",
        "https://elsewhere.test/",
    ] {
        let mut value = options();
        value.start_url = url.into();
        assert!(manifest(&value).is_err());
    }
    for origin in [
        "https://*.example.org",
        "https://example.org/private",
        "https://example.org?key=secret",
    ] {
        let mut value = options();
        value.allowed_origins = vec![origin.into()];
        assert!(manifest(&value).is_err());
    }
    let mut value = options();
    value.recorded_actions.as_mut().unwrap()[0]["script"] = json!("arbitrary code");
    assert!(manifest(&value).is_err());
    value.recorded_actions = None;
    value.computer.decision_source = DecisionSource::Jev;
    assert_eq!(manifest(&value).unwrap()["limits"]["max_provider_calls"], 1);
}

#[test]
fn browser_worker_offline_contract_uses_no_live_browser() {
    let test = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../integrations/computer-use/test_browser_worker.py");
    let output = std::process::Command::new("python3")
        .arg(test)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
