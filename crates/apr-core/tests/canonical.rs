use apr_core::{canonical_bytes, content_digest};
use serde_json::{Value, json};

#[test]
fn canonical_encoding_matches_an_independent_python_hashlib_fixture() {
    // Python 3 json.dumps(sort_keys=True, ensure_ascii=False, separators=(',', ':'))
    // and hashlib.sha256(b'apr-json-v1\0' + encoded). No APR code produced this fixture.
    let value = json!({"z": "ñ\n", "a": [true, null, 1, 1.0], "nested": {"b": 2, "a": "é"}});
    assert_eq!(
        String::from_utf8(canonical_bytes(&value).unwrap()).unwrap(),
        "{\"a\":[true,null,1,1.0],\"nested\":{\"a\":\"é\",\"b\":2},\"z\":\"ñ\\n\"}"
    );
    assert_eq!(
        content_digest(&value).unwrap(),
        "sha256:cfced7123841323c32026b686ffbd7770e21c36996232d8a215a7c5fdb4f7ba7"
    );
    let reordered: Value =
        serde_json::from_str(r#"{"nested":{"a":"é","b":2},"a":[true,null,1,1.0],"z":"ñ\n"}"#)
            .unwrap();
    assert_eq!(
        content_digest(&value).unwrap(),
        content_digest(&reordered).unwrap()
    );
    assert_ne!(
        content_digest(&json!(1)).unwrap(),
        content_digest(&json!(1.0)).unwrap()
    );
}

#[test]
fn excessively_nested_values_are_rejected_before_hashing() {
    let mut value = json!(true);
    for _ in 0..66 {
        value = json!([value]);
    }
    assert!(canonical_bytes(&value).is_err());
}

#[test]
fn numeric_content_identity_survives_persistence_and_reload() {
    // Includes IEEE-754 edge cases from serde_json's float-roundtrip regression
    // suite: canonical bytes must identify the same value after SQLite reload.
    let value = json!([
        51.248178375505404_f64,
        -93.31137037688033_f64,
        2.0030397744267762e-253_f64,
        7.101215824554616e260_f64
    ]);
    let saved = canonical_bytes(&value).unwrap();
    let reloaded: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(value, reloaded);
    assert_eq!(
        content_digest(&value).unwrap(),
        content_digest(&reloaded).unwrap()
    );
}
