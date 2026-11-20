use apr_core::{Binding, resolve_bindings, resolve_bindings_with_context};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn resolve_inputs_literals_and_committed_outputs_without_interpreting_strings() {
    let bindings = BTreeMap::from([
        (
            "literal".into(),
            Binding::Literal {
                literal: json!("${HOME}"),
            },
        ),
        (
            "escaped".into(),
            Binding::Input {
                input: "/a~1b/~0/0".into(),
            },
        ),
        (
            "answer".into(),
            Binding::Step {
                step: "review".into(),
                pointer: "/answer".into(),
            },
        ),
        (
            "nothing".into(),
            Binding::Input {
                input: "/empty".into(),
            },
        ),
    ]);
    let input = json!({"a/b": {"~": [42]}, "empty": null});
    let outputs = BTreeMap::from([("review".into(), json!({"answer": true}))]);
    assert_eq!(
        resolve_bindings(&bindings, &input, &outputs).unwrap(),
        json!({"literal": "${HOME}", "escaped": 42, "answer": true, "nothing": null})
    );
    assert!(resolve_bindings(&bindings, &input, &BTreeMap::new()).is_err());
    assert!(resolve_bindings(&bindings, &json!({}), &outputs).is_err());
    let invalid = BTreeMap::from([(
        "invalid".into(),
        Binding::Input {
            input: "/~x".into(),
        },
    )]);
    assert!(resolve_bindings(&invalid, &json!({"~x": 1}), &outputs).is_err());
}

#[test]
fn configuration_is_resolved_only_from_the_explicit_frozen_context() {
    let bindings = BTreeMap::from([(
        "settings".into(),
        Binding::Config {
            config: "/layout".into(),
        },
    )]);
    let frozen = json!({"layout":{"width":1200,"label":"${HOME}"}});
    assert_eq!(
        resolve_bindings_with_context(&bindings, &json!({}), &BTreeMap::new(), &frozen, None)
            .unwrap(),
        json!({"settings":{"width":1200,"label":"${HOME}"}})
    );
    assert!(resolve_bindings(&bindings, &json!({}), &BTreeMap::new()).is_err());
}
