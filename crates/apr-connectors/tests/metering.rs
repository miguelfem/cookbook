use apr_connectors::metering::MeteringConfig;
use serde_json::{Value, json};

fn config() -> MeteringConfig {
    serde_json::from_value(json!({
        "provider":"synthetic","revision":"test-2026-09-17","currency":"USD",
        "identity":{"pointer":"/model","equals":"fixture-model"},
        "meters":[
            {"name":"input","unit":"tokens","add":["/usage/input_tokens"],
             "subtract":["/usage/input_tokens_details/cached_tokens"],
             "rate":{"amount":"2.00","per_units":1000000}},
            {"name":"cached_input","unit":"tokens","add":["/usage/input_tokens_details/cached_tokens"],
             "rate":{"amount":"0.50","per_units":1000000}},
            {"name":"output","unit":"tokens","add":["/usage/output_tokens"],
             "rate":{"amount":"8","per_units":1000000}}
        ],"reported_cost_pointer":"/bill/amount"
    })).unwrap()
}
fn response() -> Value {
    json!({"model":"fixture-model","usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":200},
        "output_tokens":100,"output_tokens_details":{"reasoning_tokens":80}},
        "bill":{"amount":"0.0025"},"output":"private content must not appear in analytics"})
}
#[test]
fn splits_cached_input_without_double_counting_reasoning_or_totals() {
    let config = config();
    config.validate().unwrap();
    let record = config.observe(&response());
    assert_eq!(record.estimated_nanos.as_deref(), Some("2500000"));
    assert_eq!(record.reported_nanos.as_deref(), Some("2500000"));
    assert_eq!(record.meters[0].quantity, Some(800));
    assert_eq!(record.meters[2].quantity, Some(100));
    assert_eq!(record.estimate_status, "estimated");
    assert!(
        !serde_json::to_string(&record)
            .unwrap()
            .contains("private content")
    );
}
#[test]
fn missing_null_negative_fractional_or_inconsistent_usage_is_not_zero() {
    for usage in [
        json!({}),
        json!({"input_tokens":null}),
        json!({"input_tokens":-1}),
        json!({"input_tokens":1.5}),
        json!({"input_tokens":1,"input_tokens_details":{"cached_tokens":2}}),
    ] {
        let mut body = response();
        body["usage"] = usage;
        let record = config().observe(&body);
        assert!(record.meters[0].quantity.is_none());
        assert!(record.meters[0].estimated_nanos.is_none());
        assert_ne!(record.estimate_status, "estimated");
    }
}
#[test]
fn explicit_zero_and_unknown_are_distinct() {
    let mut body = response();
    body["usage"] =
        json!({"input_tokens":0,"input_tokens_details":{"cached_tokens":0},"output_tokens":0});
    assert_eq!(
        config().observe(&body).estimated_nanos.as_deref(),
        Some("0")
    );
    assert_eq!(config().observe(&json!({})).estimated_nanos, None);
}
#[test]
fn wrong_model_withholds_estimate_without_leaking_the_returned_identity() {
    let mut body = response();
    body["model"] = json!("private-unexpected-model");
    let record = config().observe(&body);
    assert!(!record.identity_matches);
    assert!(record.estimated_nanos.is_none());
    assert!(record.meters[0].quantity.is_some());
    assert!(
        !serde_json::to_string(&record)
            .unwrap()
            .contains("private-unexpected")
    );
}
#[test]
fn strict_rates_reject_float_ambiguity_overflow_and_duplicate_meters() {
    for amount in ["-1", "NaN", "1e-3", "1.0000000001", "1.", "1000000000000"] {
        let mut config = config();
        config.meters[0].rate.as_mut().unwrap().amount = amount.into();
        assert!(config.validate().is_err(), "{amount}");
    }
    let mut config = config();
    config.meters[0].rate.as_mut().unwrap().per_units = 0;
    assert!(config.validate().is_err());
    config.meters[0].rate.as_mut().unwrap().per_units = 1;
    config.meters.push(config.meters[0].clone());
    assert!(config.validate().is_err());
}
#[test]
fn fixed_point_rounds_up_only_at_nanocurrency_precision() {
    let mut config = config();
    config.meters.truncate(1);
    config.meters[0].subtract.clear();
    config.meters[0].rate.as_mut().unwrap().amount = "0.000000001".into();
    config.meters[0].rate.as_mut().unwrap().per_units = 3;
    let mut body = response();
    body["usage"]["input_tokens"] = json!(1);
    assert_eq!(config.observe(&body).estimated_nanos.as_deref(), Some("1"));
}
#[test]
fn generic_units_and_provider_specific_arithmetic_use_the_same_contract() {
    // Claude's noncached input and cache writes/reads are separate categories.
    let cfg: MeteringConfig=serde_json::from_value(json!({"provider":"synthetic-claude","revision":"v1","currency":"USD",
        "meters":[{"name":"input_total","unit":"tokens","add":["/usage/input_tokens","/usage/cache_read_input_tokens","/usage/cache_creation_input_tokens"]}]})).unwrap();
    assert_eq!(cfg.observe(&json!({"usage":{"input_tokens":50,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200}})).meters[0].quantity,Some(1250));
    // Gemini candidate tokens and thoughts are separate reported counts.
    let cfg: MeteringConfig=serde_json::from_value(json!({"provider":"synthetic-gemini","revision":"v1","currency":"USD",
        "meters":[{"name":"generated","unit":"tokens","add":["/usageMetadata/candidatesTokenCount","/usageMetadata/thoughtsTokenCount"]},
        {"name":"video","unit":"milliseconds","add":["/render/duration_ms"],"rate":{"amount":"0.01","per_units":1000}}]})).unwrap();
    let record=cfg.observe(&json!({"usageMetadata":{"candidatesTokenCount":100,"thoughtsTokenCount":300},"render":{"duration_ms":1500}}));
    assert_eq!(record.meters[0].quantity, Some(400));
    assert_eq!(
        record.meters[1].estimated_nanos.as_deref(),
        Some("15000000")
    );
    assert_eq!(record.estimate_status, "partial");
}
#[test]
fn rejects_bad_pointers_and_never_interprets_strings_as_usage() {
    let mut cfg = config();
    cfg.meters[0].add[0] = "/usage/~not-an-escape".into();
    assert!(cfg.validate().is_err());
    let mut body = response();
    body["usage"]["input_tokens"] = json!("1000");
    assert_eq!(
        config().observe(&body).meters[0].issue.as_deref(),
        Some("invalid_usage")
    );
}
#[test]
fn reported_charge_is_optional_strict_and_separate_from_estimate() {
    for bad in [json!(null), json!(-1), json!("-1"), json!({"secret":"x"})] {
        let mut body = response();
        body["bill"]["amount"] = bad;
        let observed = config().observe(&body);
        assert!(observed.reported_nanos.is_none());
        assert_eq!(observed.estimated_nanos.as_deref(), Some("2500000"));
    }
}

#[test]
fn recipe_payload_analysis_compiles_request_response_and_fixed_units() {
    let cfg: MeteringConfig = serde_json::from_value(json!({
        "provider":"synthetic-renderer", "revision":"recipe-v1", "currency":"USD",
        "identity":{"pointer":"request:/model","equals":"fixture"},
        "meters":[
            {"name":"requested_images","unit":"images","add":["request:/count"],"rate":{"amount":"0.02","per_units":1}},
            {"name":"rendered_ms","unit":"milliseconds","add":["response:/usage/ms"],"rate":{"amount":"0.01","per_units":1000}},
            {"name":"attempt","unit":"requests","add":["constant:1"],"rate":{"amount":"0.001","per_units":1}}
        ]
    })).unwrap();
    cfg.validate().unwrap();
    let actual = cfg.observe_exchange(
        &json!({"model":"fixture","count":3,"prompt":"private"}),
        &json!({"usage":{"ms":2000}}),
    );
    assert_eq!(actual.estimated_nanos.as_deref(), Some("81000000"));
    assert_eq!(actual.meters[0].basis, "request");
    assert_eq!(actual.meters[1].basis, "response");
    assert_eq!(actual.meters[2].basis, "constant");
    assert!(!serde_json::to_string(&actual).unwrap().contains("private"));
    assert_eq!(
        cfg.observe(&json!({"usage":{"ms":2000}})).estimated_nanos,
        None
    );
}
