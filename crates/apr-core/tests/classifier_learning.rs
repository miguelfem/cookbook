use apr_core::{admit_value, classifier::execute};
use serde_json::{Value, json};

fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "../../../examples/adaptive-classifier/classify/process.json"
    ))
    .unwrap()
}
fn discovery() -> Value {
    json!([{"id":"refund-a","text":"I was charged twice; please refund one payment."},
        {"id":"refund-b","text":"I cancelled but still received an invoice."}])
}
fn additions() -> Value {
    json!([{"id":"billing","description":"Charges, invoices and refunds.","example_ids":["refund-a","refund-b"],"rationale":"Two distinct requests concern payments."}])
}
fn draft() -> Value {
    let base = manifest();
    json!({"base_manifest":base,"base_process_ref":admit_value(&base).unwrap().version,"cases":discovery(),"additions":additions()})
}
fn heldout() -> Value {
    json!([
        {"id":"a1","text":"Reset my password.","expected":"account"},
        {"id":"a2","text":"Update my profile photo.","expected":"account"},
        {"id":"b1","text":"Refund an accidental purchase.","expected":"billing"},
        {"id":"b2","text":"Explain this invoice.","expected":"billing"},
        {"id":"o1","text":"Will it rain tomorrow?","expected":"other"},
        {"id":"o2","text":"Recommend a book.","expected":"other"}])
}
fn observations(records: &Value, candidate: bool) -> Value {
    json!({"items":records.as_array().unwrap().iter().map(|r| {
        let label = r["expected"].as_str().unwrap_or("other");
        let label = if !candidate && label == "billing" {"other"} else {label};
        json!({"key":r["id"],"output":{"answers":{"category":{"choice":label,"confidence":0.95}}}})
    }).collect::<Vec<_>>()})
}
fn evaluation() -> Value {
    let candidate = execute("classifier.candidate", &draft()).unwrap();
    let heldout = heldout();
    let plan = execute(
        "classifier.evaluation_plan",
        &json!({"candidate":candidate,"heldout":heldout}),
    )
    .unwrap();
    json!({"plan":plan,"baseline":observations(&heldout,false),"candidate":observations(&heldout,true)})
}

#[test]
fn collection_is_opt_in_bounded_and_requires_complete_committed_predictions() {
    let records = discovery();
    let mut input = json!({"records":records,"observations":observations(&records,false),
        "questions":manifest()["steps"][0]["inputs"]["questions"]["literal"],"enabled":false});
    assert_eq!(
        execute("classifier.collect", &input).unwrap()["cases"],
        json!([])
    );
    input["enabled"] = json!(true);
    assert_eq!(
        execute("classifier.collect", &input).unwrap()["cases"],
        records
    );
    input["observations"]["items"][0]["output"]["answers"]["category"] =
        json!({"choice":"account","confidence":0.1});
    assert_eq!(
        execute("classifier.collect", &input).unwrap()["other_count"],
        1
    );
    input["observations"]["items"].as_array_mut().unwrap().pop();
    assert!(execute("classifier.collect", &input).is_err());
}

#[test]
fn candidate_is_add_only_and_no_change_is_a_successful_outcome() {
    let original = draft();
    let candidate = execute("classifier.candidate", &original).unwrap();
    assert_ne!(
        candidate["candidate_process_ref"],
        original["base_process_ref"]
    );
    assert_eq!(
        candidate["candidate_manifest"]["steps"][0]["inputs"]["questions"],
        candidate["candidate_manifest"]["steps"][1]["inputs"]["questions"]
    );
    for mutate in [
        "existing",
        "duplicate_support",
        "missing_support",
        "stale_base",
        "changed_graph",
    ] {
        let mut wrong = original.clone();
        match mutate {
            "existing" => wrong["additions"][0]["id"] = json!("account"),
            "duplicate_support" => wrong["additions"][0]["example_ids"][1] = json!("refund-a"),
            "missing_support" => wrong["additions"][0]["example_ids"][1] = json!("missing"),
            "stale_base" => wrong["base_process_ref"] = json!("sha256:wrong"),
            _ => {
                wrong["base_manifest"]["steps"][0]["connection"] = json!("other-account");
            }
        }
        assert!(execute("classifier.candidate", &wrong).is_err(), "{mutate}");
    }
    let mut no_change = original;
    no_change["additions"] = json!([]);
    let no_change = execute("classifier.candidate", &no_change).unwrap();
    assert_eq!(no_change["status"], "no_change");
    assert_eq!(
        no_change["candidate_process_ref"],
        no_change["base_process_ref"]
    );
    assert!(
        execute(
            "classifier.evaluation_plan",
            &json!({"candidate":no_change,"heldout":heldout()})
        )
        .is_err()
    );
}

#[test]
fn heldout_is_independent_covers_every_category_and_cannot_change_candidate() {
    let candidate = execute("classifier.candidate", &draft()).unwrap();
    let original = json!({"candidate":candidate,"heldout":heldout()});
    for mutate in [
        "text_leak",
        "id_leak",
        "unknown_label",
        "too_few",
        "candidate_changed",
    ] {
        let mut wrong = original.clone();
        match mutate {
            "text_leak" => {
                wrong["heldout"][0]["text"] =
                    json!("  I WAS CHARGED TWICE; PLEASE REFUND ONE PAYMENT. \n")
            }
            "id_leak" => wrong["heldout"][0]["id"] = json!("refund-a"),
            "unknown_label" => wrong["heldout"][0]["expected"] = json!("unapproved"),
            "too_few" => {
                wrong["heldout"].as_array_mut().unwrap().pop();
            }
            _ => {
                wrong["candidate"]["candidate_manifest"]["description"] =
                    json!("Changed after review")
            }
        }
        assert!(
            execute("classifier.evaluation_plan", &wrong).is_err(),
            "{mutate}"
        );
    }
}

#[test]
fn evaluation_gate_reports_real_improvement_and_rejects_regression_or_missing_confidence() {
    let input = evaluation();
    let report = execute("classifier.evaluate", &input).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["baseline_correct"], 4);
    assert_eq!(report["candidate_correct"], 6);
    for mutate in [
        "regression",
        "missing_confidence",
        "low_confidence",
        "missing_prediction",
        "duplicate_prediction",
    ] {
        let mut wrong = input.clone();
        let answer = &mut wrong["candidate"]["items"][0]["output"]["answers"]["category"];
        match mutate {
            "regression" => answer["choice"] = json!("other"),
            "missing_confidence" => {
                answer.as_object_mut().unwrap().remove("confidence");
            }
            "low_confidence" => answer["confidence"] = json!(0.74),
            "missing_prediction" => {
                wrong["candidate"]["items"].as_array_mut().unwrap().pop();
            }
            _ => wrong["candidate"]["items"][1]["key"] = json!("a1"),
        }
        let result = execute("classifier.evaluate", &wrong);
        assert!(
            result.is_err() || result.unwrap()["passed"] == false,
            "{mutate}"
        );
    }
}

#[test]
fn installed_recipes_admit_without_provider_features_or_accounts() {
    for source in [
        include_str!("../../../examples/adaptive-classifier/classify/process.json"),
        include_str!("../../../examples/adaptive-classifier/propose/process.json"),
        include_str!("../../../examples/adaptive-classifier/evaluate/process.json"),
    ] {
        apr_core::admit_json(source.as_bytes()).unwrap();
    }
}
