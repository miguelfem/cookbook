use apr_core::{admit_value, resolve_bindings_with_context};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn process() -> Value {
    json!({"schema_version":"0.1","name":"foreach.fixture","description":"Bounded item snapshots",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"builtin"}],"limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[{"id":"each","kind":"builtin","operation":"json.select","effect":"pure","needs":[],"retry":{"max_attempts":2},
            "foreach":{"items":{"input":"/items"},"key":"/id","max_items":3,"concurrency":2},
            "inputs":{"value":{"item":""},"pointer":{"literal":"/value"}}}],
        "output":{"items":{"step":"each","pointer":"/items"}}})
}

#[test]
fn bounded_foreach_admits_item_bindings_only_inside_its_own_context() {
    let admitted = admit_value(&process()).unwrap();
    let values = resolve_bindings_with_context(
        &admitted.manifest.steps[0].inputs,
        &json!({}),
        &BTreeMap::new(),
        &json!({}),
        Some(&json!({"id":"b","value":7})),
    )
    .unwrap();
    assert_eq!(
        values,
        json!({"value":{"id":"b","value":7},"pointer":"/value"})
    );
    assert!(
        resolve_bindings_with_context(
            &admitted.manifest.steps[0].inputs,
            &json!({}),
            &BTreeMap::new(),
            &json!({}),
            None
        )
        .is_err()
    );
    let mut outside = process();
    outside["steps"][0]
        .as_object_mut()
        .unwrap()
        .remove("foreach");
    assert!(admit_value(&outside).is_err());
    let mut invalid = process();
    invalid["steps"][0]["foreach"]["items"] = json!({"item":"/items"});
    assert!(admit_value(&invalid).is_err());
    invalid = process();
    invalid["steps"][0]["foreach"]["concurrency"] = json!(4);
    assert!(admit_value(&invalid).is_err());
    invalid = process();
    invalid["steps"][0]["foreach"]["items"] = json!({"step":"absent","pointer":"/items"});
    assert!(admit_value(&invalid).is_err());
}

#[test]
fn v02_foreach_can_key_scalar_items_by_their_full_value() {
    let mut scalar = process();
    scalar["steps"][0]["foreach"]["key"] = json!("");
    assert!(admit_value(&scalar).is_err());

    scalar["schema_version"] = json!("0.2");
    let admitted = admit_value(&scalar).unwrap();
    let each = admitted.manifest.steps[0].foreach.as_ref().unwrap();
    let address = json!("person01@example.test");
    assert_eq!(address.pointer(&each.key), Some(&address));
}
