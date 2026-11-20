use apr_core::{admit_json, admit_value, content_digest, validate_instance};
use serde_json::{Value, json};

fn process() -> Value {
    json!({
        "schema_version": "0.1", "name": "local.review", "description": "Review local data",
        "input_schema": {"type": "object"}, "output_schema": {"type": "object"},
        "config": {"schema": {"type": "object"}, "defaults": {}, "run_override_keys": []},
        "capabilities": [{"kind": "builtin"}],
        "limits": {"max_step_ms": 1000, "max_artifact_bytes": 1000000,
                   "max_provider_calls": 0, "max_agent_requests": 0},
        "steps": [{"id": "record", "kind": "builtin", "operation": "json.write",
                   "needs": [], "effect": "pure", "retry": {"max_attempts": 2},
                   "inputs": {"value": {"input": ""}}}],
        "output": {"result": {"step": "record", "pointer": ""}}
    })
}

#[test]
fn an_admitted_process_is_an_immutable_snapshot_with_a_content_identity() {
    let mut source = process();
    let admitted = admit_value(&source).unwrap();
    source["description"] = json!("Changed source");
    assert_eq!(admitted.manifest.description, "Review local data");
    assert_eq!(admitted.version, content_digest(&process()).unwrap());
    assert_ne!(admitted.version, admit_value(&source).unwrap().version);
    assert_eq!(
        serde_json::from_slice::<Value>(&admitted.canonical_bytes).unwrap(),
        process()
    );
}

#[test]
fn invalid_manifest_shapes_and_unbounded_limits_are_rejected() {
    let mut invalid = process();
    invalid["limits"]["max_step_ms"] = json!(0);
    let error = admit_value(&invalid).unwrap_err().to_string();
    assert!(error.contains("/limits/max_step_ms"), "{error}");
    invalid = process();
    invalid["unrecognized"] = json!(true);
    assert!(admit_value(&invalid).is_err());
}

#[test]
fn dependencies_must_form_a_dag_and_bindings_must_read_declared_ancestors() {
    let mut duplicate = process();
    let repeated = duplicate["steps"][0].clone();
    duplicate["steps"].as_array_mut().unwrap().push(repeated);
    assert!(
        admit_value(&duplicate).is_err(),
        "duplicate step ID was admitted"
    );

    let mut missing = process();
    missing["steps"][0]["needs"] = json!(["absent"]);
    assert!(
        admit_value(&missing).is_err(),
        "missing dependency was admitted"
    );

    let mut cyclic = process();
    let mut second = cyclic["steps"][0].clone();
    second["id"] = json!("review");
    second["needs"] = json!(["record"]);
    cyclic["steps"].as_array_mut().unwrap().push(second);
    cyclic["steps"][0]["needs"] = json!(["review"]);
    assert!(admit_value(&cyclic).is_err(), "cycle was admitted");

    let mut undeclared = cyclic;
    undeclared["steps"][0]["needs"] = json!([]);
    undeclared["steps"][1]["needs"] = json!([]);
    undeclared["steps"][1]["inputs"]["value"] = json!({"step": "record", "pointer": ""});
    assert!(
        admit_value(&undeclared).is_err(),
        "undeclared input dependency was admitted"
    );
    undeclared["steps"][1]["needs"] = json!(["record"]);
    admit_value(&undeclared).unwrap();
    undeclared["output"]["result"]["step"] = json!("absent");
    assert!(
        admit_value(&undeclared).is_err(),
        "missing output source was admitted"
    );
}

#[test]
fn unsupported_execution_features_and_unsafe_effects_fail_admission() {
    let cases = [
        ("operation", json!("shell.exec")),
        ("effect", json!("write")),
        ("entrypoint", json!("script.py")),
    ];
    for (field, value) in cases {
        let mut invalid = process();
        invalid["steps"][0][field] = value;
        assert!(
            admit_value(&invalid).is_err(),
            "unsupported {field} was admitted"
        );
    }
    let mut invalid = process();
    invalid["steps"][0]["inputs"]["value"] = json!({"item": ""});
    assert!(admit_value(&invalid).is_err());
    invalid = process();
    invalid["limits"]["max_provider_calls"] = json!(10001);
    assert!(admit_value(&invalid).is_err());
    invalid = process();
    invalid["files"] = json!([{"path":"unused.py","sha256":"0".repeat(64)}]);
    assert!(admit_value(&invalid).is_err());

    let mut assisted = process();
    assisted["steps"][0] = json!({"id": "record", "kind": "agent", "effect": "agent", "needs": [],
        "retry": {"max_attempts": 1}, "inputs": {}, "capability": "review", "instructions": "Review",
        "result_schema": {"type": "object"}});
    assert!(
        admit_value(&assisted).is_err(),
        "agent request budget was ignored"
    );
    assisted["limits"]["max_agent_requests"] = json!(1);
    admit_value(&assisted).unwrap();
    assisted["steps"][0]["retry"]["max_attempts"] = json!(2);
    assert!(
        admit_value(&assisted).is_err(),
        "automatic agent retries were admitted"
    );
}

#[test]
fn embedded_schemas_are_checked_locally_and_defaults_must_match() {
    let mut invalid = process();
    invalid["input_schema"] = json!({"type": 42});
    assert!(
        admit_value(&invalid).is_err(),
        "invalid embedded schema was admitted"
    );
    invalid["input_schema"] = json!({"$ref": "https://example.invalid/schema.json"});
    assert!(
        admit_value(&invalid).is_err(),
        "remote reference was admitted"
    );
    invalid["input_schema"] =
        json!({"$defs": {"x": {"$ref": "file:///tmp/schema.json"}}, "$ref": "#/$defs/x"});
    assert!(
        admit_value(&invalid).is_err(),
        "nested file reference was admitted"
    );
    invalid = process();
    invalid["config"]["schema"] = json!({"type": "object", "required": ["mode"],
        "properties": {"mode": {"enum": ["review"]}}});
    assert!(
        admit_value(&invalid).is_err(),
        "invalid defaults were admitted"
    );
    invalid["config"]["defaults"] = json!({"mode": "review"});
    admit_value(&invalid).unwrap();

    let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": {"answer": {"type": "integer", "minimum": 1}},
        "type": "object", "required": ["answer"], "additionalProperties": false,
        "properties": {"answer": {"$ref": "#/$defs/answer"}}});
    validate_instance(&schema, &json!({"answer": 3})).unwrap();
    assert!(validate_instance(&schema, &json!({"answer": 0})).is_err());
    assert!(validate_instance(&schema, &json!({"answer": "3"})).is_err());
    assert!(validate_instance(&schema, &json!({"answer": 3, "extra": true})).is_err());
    // $ref inside literal data is data, not a schema resource to retrieve.
    validate_instance(
        &json!({"const": {"$ref": "https://example.invalid/"}}),
        &json!({"$ref": "https://example.invalid/"}),
    )
    .unwrap();
}

#[test]
fn json_text_rejects_duplicate_keys_before_the_manifest_is_hashed() {
    let bytes = serde_json::to_vec(&process()).unwrap();
    assert_eq!(
        admit_json(&bytes).unwrap().version,
        admit_value(&process()).unwrap().version
    );
    let source = serde_json::to_string(&process()).unwrap();
    let duplicated = source.replacen("{", "{\"name\":\"shadow.name\",", 1);
    assert!(
        admit_json(duplicated.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    let nested = source.replace(
        "\"max_step_ms\":1000",
        "\"max_step_ms\":1,\"max_step_ms\":1000",
    );
    assert!(
        admit_json(nested.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    assert!(admit_json(format!("{source} {{}}").as_bytes()).is_err());
    assert!(admit_json(&vec![b' '; 1024 * 1024 + 1]).is_err());
}

#[test]
fn configuration_override_keys_and_bindings_are_explicit() {
    let mut configured = process();
    configured["config"]["schema"] =
        json!({"type": "object", "properties": {"mode": {"type": "string"}}});
    configured["config"]["defaults"] = json!({"mode": "review"});
    configured["config"]["run_override_keys"] = json!(["mode"]);
    configured["steps"][0]["inputs"]["value"] = json!({"config":"/mode"});
    admit_value(&configured).unwrap();
    configured["config"]["run_override_keys"] = json!([""]);
    assert!(admit_value(&configured).is_err());
}

#[test]
fn native_and_connector_contracts_require_declared_permissions_and_effect_policy() {
    let native: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    admit_value(&native).unwrap();
    for field in ["effect", "retry", "files", "capabilities"] {
        let mut invalid = native.clone();
        match field {
            "effect" => invalid["steps"][0]["effect"] = json!("pure"),
            "retry" => invalid["steps"][0]["retry"]["max_attempts"] = json!(2),
            "files" => invalid["files"][0]["path"] = json!("../main.py"),
            _ => invalid["capabilities"] = json!([]),
        }
        assert!(
            admit_value(&invalid).is_err(),
            "{field} bypassed native admission"
        );
    }
    let connector: Value =
        serde_json::from_str(include_str!("../../../examples/http-json/process.json")).unwrap();
    admit_value(&connector).unwrap();
    let mut invalid = connector.clone();
    invalid["steps"][0]["connection"] = json!("different");
    assert!(admit_value(&invalid).is_err());
    let mut invalid = connector;
    invalid["steps"][0]["retry"]["max_attempts"] = json!(2);
    assert!(admit_value(&invalid).is_err());
}

#[test]
fn admission_diagnostic_is_bounded_and_does_not_echo_manifest_values() {
    let mut native: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    native["steps"][0]["effect"] = json!("pure");
    let error = admit_value(&native).unwrap_err();
    let diagnostic = error.diagnostic().expect("safe repair diagnostic");
    assert_eq!(diagnostic.field, "steps[].script");
    assert_eq!(diagnostic.reason, "invalid_python_step_contract");
    assert!(!diagnostic.repair.contains("main.py"));
    assert!(!diagnostic.repair.contains("secret"));
}
