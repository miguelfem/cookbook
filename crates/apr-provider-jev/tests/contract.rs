use apr_provider_jev::{observe, prepare};
use serde_json::json;

fn request() -> serde_json::Value {
    json!({"model":"jev-latest","state":{"ticket":"export failed"},"questions":{
     "team":{"type":"choice","instructions":"Choose owner","criteria":{"ops":"Operations","eng":"Engineering"}},
     "severity":{"type":"score","instructions":"Rate impact","criteria":["low","medium","high"]},
     "urgent":{"type":"noul","instructions":"Is urgent?"}
    }})
}
fn response() -> serde_json::Value {
    json!({"model":"jev-reported-model","answers":{
 "team":{"type":"choice","choice":"eng","probabilities":{"ops":0.1,"eng":0.9},"confidence":0.7},
 "severity":{"type":"score","score":1.6,"legend":{"0":"low","1":"medium","2":"high"},"probabilities":{"0":0.05,"1":0.3,"2":0.65},"confidence":0.6},
 "urgent":{"type":"noul","noul":0.95}
},"usage":{"input_tokens":312,"output_tokens":48}})
}

#[test]
fn typed_observations_preserve_models_usage_and_missing_confidence() {
    let request = prepare(request()).unwrap();
    let observed = observe(&request, response()).unwrap();
    let value = serde_json::to_value(observed).unwrap();
    assert_eq!(value["requested_model"], "jev-latest");
    assert_eq!(value["reported_model"], "jev-reported-model");
    assert_eq!(value["usage"]["input_tokens"], 312);
    assert!(value["answers"]["urgent"].get("confidence").is_none());
    assert_eq!(request.body()["questions"]["team"]["type"], "choice");
    let mut missing = response();
    missing["answers"]["team"]
        .as_object_mut()
        .unwrap()
        .remove("confidence");
    assert!(
        serde_json::to_value(observe(&request, missing).unwrap()).unwrap()["answers"]["team"]
            .get("confidence")
            .is_none()
    );
}

#[test]
fn drift_missing_answers_bad_probabilities_and_invented_choices_are_rejected() {
    let request = prepare(request()).unwrap();
    for (pointer, value) in [
        ("/answers/team/choice", json!("unknown")),
        ("/answers/team/probabilities/ops", json!(1.1)),
        ("/answers/urgent/noul", json!(-0.2)),
        ("/answers/severity/score", json!(0.1)),
        ("/answers/severity/legend/0", json!("unexpected")),
        ("/answers/team/confidence", json!("high")),
        ("/usage/input_tokens", json!(-3)),
    ] {
        let mut changed = response();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(observe(&request, changed).is_err(), "{pointer}");
    }
    let mut missing = response();
    missing["answers"].as_object_mut().unwrap().remove("urgent");
    assert!(observe(&request, missing).is_err());
    let mut extra = response();
    extra["answers"]["invented"] = json!({"type":"noul","noul":1});
    assert!(observe(&request, extra).is_err());
}
