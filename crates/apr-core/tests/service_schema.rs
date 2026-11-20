use apr_core::{
    validate_instance, validate_schema, validate_service_instance, validate_service_schema,
};
use serde_json::{Value, json};

const DRAFT7: &str = "http://json-schema.org/draft-07/schema#";
const DRAFT202012: &str = "https://json-schema.org/draft/2020-12/schema";

#[test]
fn service_draft7_tuple_and_dependencies_use_draft7_semantics() {
    let schema = json!({"$schema": DRAFT7, "type": "object",
        "properties": {"coordinates": {"type": "array", "items": [
            {"type": "integer"}, {"type": "string"}
        ], "additionalItems": false}},
        "dependencies": {
            "coordinates": ["unit"],
            "unit": {"properties": {"unit": {"enum": ["meters"]}}}
        }
    });
    validate_service_schema(&schema).unwrap();
    validate_service_instance(
        &schema,
        &json!({"coordinates": [1, "north"], "unit": "meters"}),
    )
    .unwrap();
    for invalid in [
        json!({"coordinates": [1, "north", true], "unit": "meters"}),
        json!({"coordinates": ["1", "north"], "unit": "meters"}),
        json!({"coordinates": [1, "north"]}),
        json!({"coordinates": [1, "north"], "unit": "feet"}),
    ] {
        assert!(
            validate_service_instance(&schema, &invalid).is_err(),
            "accepted {invalid}"
        );
    }
    assert!(validate_schema(&schema).is_err());
    assert!(validate_instance(&schema, &json!({})).is_err());
}

#[test]
fn draft7_ref_ignores_sibling_validation_but_policy_inspects_it() {
    let schema = json!({"$schema": DRAFT7,
        "definitions": {"value": {"type": "integer"}},
        "$ref": "#/definitions/value", "type": "string"});
    validate_service_instance(&schema, &json!(4)).unwrap();
    assert!(validate_service_instance(&schema, &json!("four")).is_err());

    let hidden_external = json!({"$schema": DRAFT7,
        "definitions": {"value": {"type": "integer"}},
        "$ref": "#/definitions/value", "allOf": [{"$ref": "https://example.invalid/remote"}]});
    assert!(validate_service_schema(&hidden_external).is_err());
    let modern = json!({"$schema": DRAFT202012, "$defs": {"value": {"type": "integer"}},
        "$ref": "#/$defs/value", "minimum": 5});
    assert!(validate_service_instance(&modern, &json!(4)).is_err());
    validate_service_instance(&modern, &json!(5)).unwrap();
}

#[test]
fn service_schema_defaults_to_202012_and_rejects_unknown_or_nested_dialects() {
    for schema in [
        json!({"type": "integer"}),
        json!({"$schema": DRAFT202012, "type": "integer"}),
    ] {
        validate_service_schema(&schema).unwrap();
        validate_service_instance(&schema, &json!(1)).unwrap();
        assert!(validate_service_instance(&schema, &json!("1")).is_err());
    }
    for schema in [
        json!({"$schema": "http://json-schema.org/draft-06/schema#"}),
        json!({"$schema": DRAFT7, "properties": {"x": {"$schema": DRAFT202012}}}),
        json!({"$schema": DRAFT202012, "$defs": {"x": {"$schema": DRAFT7}}}),
    ] {
        assert!(
            validate_service_schema(&schema).is_err(),
            "accepted {schema}"
        );
    }
}

#[test]
fn both_dialects_reject_external_cyclic_and_excessively_deep_references() {
    for dialect in [DRAFT7, DRAFT202012] {
        let tuple_or_item = if dialect == DRAFT7 {
            json!({"type": "array", "items": [{"$ref": "file:///tmp/private.json"}]})
        } else {
            json!({"type": "array", "prefixItems": [{"$ref": "file:///tmp/private.json"}]})
        };
        let mut external = tuple_or_item;
        external["$schema"] = json!(dialect);
        assert!(
            validate_service_schema(&external).is_err(),
            "accepted {external}"
        );

        let definitions = if dialect == DRAFT7 {
            "definitions"
        } else {
            "$defs"
        };
        let reference = format!("#/{definitions}/a");
        let mut cyclic = json!({"$schema": dialect, "$ref": reference});
        cyclic[definitions] = json!({"a": {"$ref": reference}});
        assert!(
            validate_service_schema(&cyclic).is_err(),
            "accepted {cyclic}"
        );

        let mut chain = Value::Bool(true);
        for _ in 0..70 {
            chain = json!({"allOf": [chain]});
        }
        // JSON's canonical depth bound also applies to service schemas.
        assert!(validate_service_schema(&chain).is_err());

        let mut deep_definitions = serde_json::Map::new();
        for i in 0..=65 {
            deep_definitions.insert(
                format!("x{i}"),
                if i == 65 {
                    json!({"type": "integer"})
                } else {
                    json!({"$ref": format!("#/{definitions}/x{}", i + 1)})
                },
            );
        }
        let mut deep = json!({"$schema": dialect, "$ref": format!("#/{definitions}/x0")});
        deep[definitions] = json!(deep_definitions);
        assert!(
            validate_service_schema(&deep).is_err(),
            "accepted deep refs"
        );

        let mut expanded = serde_json::Map::new();
        expanded.insert("x0".into(), json!({"type": "object"}));
        for i in 1..=13 {
            let prior = format!("#/{definitions}/x{}", i - 1);
            expanded.insert(
                format!("x{i}"),
                json!({"allOf": [{"$ref": prior}, {"$ref": prior}]}),
            );
        }
        let mut costly = json!({"$schema": dialect, "$ref": format!("#/{definitions}/x13")});
        costly[definitions] = json!(expanded);
        assert!(
            validate_service_schema(&costly).is_err(),
            "accepted expansion"
        );
    }
}

#[test]
fn draft7_dependencies_schema_positions_are_checked_but_property_lists_are_data() {
    let safe =
        json!({"$schema": DRAFT7, "dependencies": {"$ref": ["https://example.invalid/not-a-ref"]}});
    validate_service_schema(&safe).unwrap();
    let unsafe_schema = json!({"$schema": DRAFT7, "dependencies": {
        "x": {"$ref": "https://example.invalid/remote"}}});
    assert!(validate_service_schema(&unsafe_schema).is_err());
    let unsafe_extra = json!({"$schema": DRAFT7, "type": "array", "items": [{"type": "integer"}],
        "additionalItems": {"$ref": "https://example.invalid/remote"}});
    assert!(validate_service_schema(&unsafe_extra).is_err());
    validate_service_schema(&json!({"$schema": DRAFT7,
        "const": {"$ref": "https://example.invalid/literal-data"}}))
    .unwrap();
}
