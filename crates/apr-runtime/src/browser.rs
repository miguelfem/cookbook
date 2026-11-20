//! Isolated background browser variant of the durable computer-use graph.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::computer::ComputerOptions;
use crate::{Result, RuntimeError};

pub const WORKER_SOURCE: &str =
    include_str!("../../../integrations/computer-use/browser_worker.py");
const REQUIREMENTS: &str =
    include_str!("../../../integrations/computer-use/requirements-browser.txt");

pub struct BrowserOptions {
    pub computer: ComputerOptions,
    pub start_url: String,
    /// Exact origins; never a wildcard or a generated destination.
    pub allowed_origins: Vec<String>,
    pub browser_executable: PathBuf,
    /// Owner-reviewed actions extracted from an already successful workflow.
    pub recorded_actions: Option<Vec<Value>>,
}

fn origin(value: &str) -> Result<String> {
    let parsed = url::Url::parse(value)
        .map_err(|_| RuntimeError::InvalidInput("browser URL is invalid".into()))?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(RuntimeError::InvalidInput(
            "browser destinations require HTTP(S) without embedded credentials".into(),
        ));
    }
    Ok(format!(
        "{}://{}:{}",
        parsed.scheme(),
        parsed.host_str().unwrap(),
        parsed.port_or_known_default().unwrap()
    ))
}

pub fn manifest(options: &BrowserOptions) -> Result<Value> {
    if options.allowed_origins.is_empty() || options.allowed_origins.len() > 16 {
        return Err(RuntimeError::InvalidInput(
            "browser requires 1..16 explicit origins".into(),
        ));
    }
    let origins: Vec<_> = options
        .allowed_origins
        .iter()
        .map(|value| origin(value))
        .collect::<Result<_>>()?;
    for value in &options.allowed_origins {
        let parsed = url::Url::parse(value)
            .map_err(|_| RuntimeError::InvalidInput("invalid browser origin".into()))?;
        if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(RuntimeError::InvalidInput(
                "--origin requires an origin without path, query or fragment".into(),
            ));
        }
    }
    if !origins.contains(&origin(&options.start_url)?) {
        return Err(RuntimeError::InvalidInput(
            "browser start URL must belong to an approved origin".into(),
        ));
    }
    let pin =
        crate::worker::InterpreterPin::inspect(&options.browser_executable).map_err(|_| {
            RuntimeError::InvalidInput("browser requires an absolute installed executable".into())
        })?;
    let mut computer = options.computer.clone();
    computer.allowed_apps = vec!["ai.cookbook.isolated-browser".into()];
    let mut process = crate::computer::manifest(&computer)?;
    if let Some(actions) = &options.recorded_actions {
        let valid_text = |value: &Value| {
            value.as_str().is_some_and(|text| {
                !text.trim().is_empty() && text.len() <= 256 && !text.chars().any(char::is_control)
            })
        };
        if actions.len() != computer.max_steps as usize
            || computer.success_text.is_none()
            || actions.iter().any(|action| {
                action.as_object().is_none_or(|object| {
                    object.len() != 3
                        || action["action"] != "click"
                        || !valid_text(&action["label"])
                        || !valid_text(&action["expected_text"])
                })
            })
        {
            return Err(RuntimeError::InvalidInput("recorded browser plans require one click/label/expected_text entry per step and an explicit final success text".into()));
        }
    }
    let config = json!({"start_url":options.start_url,"allowed_origins":origins,
        "browser_executable":pin.path,"browser_sha256":pin.sha256,
        "max_steps":computer.max_steps,"max_elapsed_ms":computer.max_elapsed_ms,
        "min_confidence":computer.min_confidence,"model":computer.model,
        "allowed_text":computer.allowed_text,"decision_source":computer.decision_source,
        "success_text":computer.success_text,"recorded_actions":options.recorded_actions});
    process["description"] = json!(
        "Isolated headless browser with exact destinations, fresh observations, durable per-action effects and explicit completion evidence."
    );
    process["config"] =
        json!({"schema":{"type":"object","const":config},"defaults":config,"run_override_keys":[]});
    process["files"] = json!([{"path":"browser_worker.py","sha256":format!("{:x}",Sha256::digest(WORKER_SOURCE.as_bytes()))}]);
    let steps = process["steps"].as_array_mut().unwrap();
    for step in steps.iter_mut() {
        if step["kind"] == "script" {
            step["entrypoint"] = json!("browser_worker.py");
        }
        if options.recorded_actions.is_some()
            && step["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("decide_"))
        {
            let id = step["id"].as_str().unwrap().to_owned();
            let observe = id.replacen("decide_", "observe_", 1);
            let needs = step["needs"].clone();
            let when = step["when"].clone();
            *step = json!({"id":id,"kind":"builtin","effect":"pure","operation":"json.select",
                "needs":needs,"when":when,"retry":{"max_attempts":1},
                "inputs":{"value":{"step":observe,"pointer":"/recorded_decision"},"pointer":{"literal":""}}});
        }
        if options.recorded_actions.is_some()
            && step["id"].as_str().is_some_and(|id| id.starts_with("act_"))
        {
            step["inputs"]["decision"]["pointer"] = json!("/value");
        }
        if step["id"] == "verify" {
            step["inputs"]
                .as_object_mut()
                .unwrap()
                .remove("allowed_apps");
            step["inputs"]["allowed_origins"] = json!({"config":"/allowed_origins"});
            step["instructions"] = json!(
                "Review the isolated browser session evidence. A chosen done action or click alone does not prove completion. Return completed only with observed evidence, otherwise stopped. Never replay an uncertain action or open the user's browser to impersonate this isolated session. Credentials and arbitrary code are not supported by this helper."
            );
        }
    }
    if options.recorded_actions.is_some() {
        process["capabilities"] = json!([{"kind":"script","language":"python"},{"kind":"agent","capability":"computer_review"}]);
        process["limits"]["max_agent_requests"] = json!(1);
        process["limits"]["max_provider_calls"] = json!(0);
        process["config"]["defaults"]["decision_source"] = json!("caller");
        process["config"]["schema"]["const"]["decision_source"] = json!("caller");
    }
    let final_session = format!("session_{}", computer.max_steps - 1);
    process["steps"].as_array_mut().unwrap().push(json!({"id":"close_browser","kind":"script","entrypoint":"browser_worker.py",
        "effect":"write","needs":[final_session],"retry":{"max_attempts":1},
        "inputs":{"operation":{"literal":"close"},"session":{"step":final_session,"pointer":"/output"}},
        "result_schema":{"type":"object"}}));
    process["output"]["browser_cleanup"] = json!({"step":"close_browser","pointer":""});
    apr_core::admit_value(&process)?;
    Ok(process)
}

pub fn scaffold(directory: &Path, options: &BrowserOptions) -> Result<Value> {
    let process = manifest(options)?;
    fs::DirBuilder::new().mode(0o700).create(directory)?;
    let bytes = serde_json::to_vec_pretty(&process)?;
    for (name, contents) in [
        ("process.json", bytes.as_slice()),
        ("browser_worker.py", WORKER_SOURCE.as_bytes()),
        ("requirements-browser.txt", REQUIREMENTS.as_bytes()),
    ] {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join(name))?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    Ok(
        json!({"directory":directory,"manifest":directory.join("process.json"),
        "backend":"isolated_browser","headless":true,"granted":false,"executed":false,
        "browser_sha256":process["config"]["defaults"]["browser_sha256"],
        "allowed_origins":process["config"]["defaults"]["allowed_origins"]}),
    )
}
