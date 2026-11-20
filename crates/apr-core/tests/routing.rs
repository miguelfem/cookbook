use std::collections::BTreeMap;

use apr_core::{
    Binding, admit_value, content_digest,
    routing::{
        BranchWhen, CallDefinition, CallGraphLimits, CallTarget, DecisionPolicy, JoinDefinition,
        RouteNode, analyze_call_graph, evaluate_decision, resolve_join, validate_call_graph,
        validate_policy, validate_routing_graph,
    },
};
use serde_json::{Value, json};

fn policy(value: Value) -> DecisionPolicy {
    serde_json::from_value(value).unwrap()
}

#[test]
fn exact_rules_choose_the_first_typed_match_and_record_the_frozen_evidence() {
    let definition = policy(json!({"source":"rules","routes":["number","text","other"],
        "cases":[{"value":{"input":"/value"},"equals":7,"route":"number"},
            {"value":{"input":"/value"},"equals":"7","route":"text"}],"default":"other"}));
    validate_policy(&definition).unwrap();
    let result = evaluate_decision(
        &definition,
        &json!({"value":"7","unrelated":"private"}),
        &BTreeMap::new(),
        &json!({}),
        None,
    )
    .unwrap();
    assert_eq!(result.route, "text");
    assert_eq!(result.source, "rules");
    assert_eq!(result.candidates, vec!["number", "text", "other"]);
    assert_eq!(
        result.policy_digest,
        content_digest(&serde_json::to_value(&definition).unwrap()).unwrap()
    );
    assert_eq!(
        result.input_digest,
        content_digest(&result.observation).unwrap()
    );
    assert!(!serde_json::to_string(&result).unwrap().contains("private"));
    let fallback = evaluate_decision(
        &definition,
        &json!({"value":false}),
        &BTreeMap::new(),
        &json!({}),
        None,
    )
    .unwrap();
    assert_eq!(fallback.route, "other");
    assert_eq!(fallback.reason, "default");
}

#[test]
fn typed_choice_uses_declared_fallbacks_without_inventing_confidence() {
    let definition = policy(
        json!({"source":"choice","routes":["accept","review","none"],"value":{"input":"/route"},
        "confidence":{"input":"/confidence"},"min_confidence":0.8,"no_match":"none","needs_clarification":"review"}),
    );
    for (input, route, reason) in [
        (
            json!({"route":"accept","confidence":0.9}),
            "accept",
            "selected",
        ),
        (
            json!({"route":"accept","confidence":0.7}),
            "review",
            "needs_clarification",
        ),
        (json!({"route":"accept"}), "review", "needs_clarification"),
        (
            json!({"route":"unregistered","confidence":0.99}),
            "none",
            "no_match",
        ),
    ] {
        let result =
            evaluate_decision(&definition, &input, &BTreeMap::new(), &json!({}), None).unwrap();
        assert_eq!(result.route, route);
        assert_eq!(result.reason, reason);
        assert_eq!(
            result.observation["confidence"],
            input.get("confidence").cloned().unwrap_or(Value::Null)
        );
    }
    for invalid in [
        json!({"route":7}),
        json!({"route":"accept","confidence":"0.9"}),
        json!({"route":"accept","confidence":1.01}),
    ] {
        assert!(
            evaluate_decision(&definition, &invalid, &BTreeMap::new(), &json!({}), None).is_err()
        );
    }
    let no_threshold = policy(
        json!({"source":"choice","routes":["accept","review","none"],"value":{"input":"/route"},"no_match":"none","needs_clarification":"review"}),
    );
    assert_eq!(
        evaluate_decision(
            &no_threshold,
            &json!({"route":"accept"}),
            &BTreeMap::new(),
            &json!({}),
            None
        )
        .unwrap()
        .route,
        "accept"
    );
}

#[test]
fn policies_reject_undeclared_routes_invalid_thresholds_and_unbounded_evidence() {
    for invalid in [
        json!({"source":"rules","routes":["yes","yes"],"cases":[],"default":"yes"}),
        json!({"source":"rules","routes":["yes"],"cases":[],"default":"no"}),
        json!({"source":"rules","routes":["yes"],"cases":[{"value":{"literal":true},"equals":true,"route":"no"}],"default":"yes"}),
        json!({"source":"choice","routes":["yes","no"],"value":{"input":"/route"},"no_match":"no","needs_clarification":"missing"}),
        json!({"source":"choice","routes":["yes","no"],"value":{"input":"/route"},"min_confidence":0.8,"no_match":"no","needs_clarification":"no"}),
        json!({"source":"choice","routes":["yes","no"],"value":{"input":"/route"},"confidence":{"input":"/confidence"},"min_confidence":-0.01,"no_match":"no","needs_clarification":"no"}),
    ] {
        assert!(validate_policy(&policy(invalid)).is_err());
    }
    let definition = policy(
        json!({"source":"rules","routes":["yes"],"cases":[{"value":{"input":""},"equals":true,"route":"yes"}],"default":"yes"}),
    );
    assert!(
        evaluate_decision(
            &definition,
            &json!("x".repeat(50 * 1024)),
            &BTreeMap::new(),
            &json!({}),
            None
        )
        .is_err()
    );
}

#[test]
fn conditional_outputs_require_the_matching_branch_or_an_explicit_join() {
    let gate = policy(
        json!({"source":"rules","routes":["accept","reject"],"cases":[],"default":"reject"}),
    );
    let accept = BranchWhen {
        decision: "gate".into(),
        route: "accept".into(),
    };
    let reject = BranchWhen {
        decision: "gate".into(),
        route: "reject".into(),
    };
    let empty = BTreeMap::new();
    let gate_dependency = vec!["gate".to_owned()];
    let join_dependencies = vec![
        "gate".to_owned(),
        "accepted".to_owned(),
        "rejected".to_owned(),
    ];
    let join = JoinDefinition {
        decision: "gate".into(),
        branches: BTreeMap::from([
            (
                "accept".into(),
                Binding::Step {
                    step: "accepted".into(),
                    pointer: "".into(),
                },
            ),
            (
                "reject".into(),
                Binding::Step {
                    step: "rejected".into(),
                    pointer: "".into(),
                },
            ),
        ]),
    };
    let mut nodes = vec![
        RouteNode {
            id: "gate",
            kind: "decision",
            needs: &[],
            inputs: &empty,
            foreach_items: None,
            when: None,
            decision: Some(&gate),
            join: None,
            call: None,
        },
        RouteNode {
            id: "accepted",
            kind: "builtin",
            needs: &gate_dependency,
            inputs: &empty,
            foreach_items: None,
            when: Some(&accept),
            decision: None,
            join: None,
            call: None,
        },
        RouteNode {
            id: "rejected",
            kind: "builtin",
            needs: &gate_dependency,
            inputs: &empty,
            foreach_items: None,
            when: Some(&reject),
            decision: None,
            join: None,
            call: None,
        },
        RouteNode {
            id: "merge",
            kind: "join",
            needs: &join_dependencies,
            inputs: &empty,
            foreach_items: None,
            when: None,
            decision: None,
            join: Some(&join),
            call: None,
        },
    ];
    validate_routing_graph(
        &nodes,
        &BTreeMap::from([(
            "result".into(),
            Binding::Step {
                step: "merge".into(),
                pointer: "".into(),
            },
        )]),
    )
    .unwrap();
    assert!(
        validate_routing_graph(
            &nodes,
            &BTreeMap::from([(
                "result".into(),
                Binding::Step {
                    step: "accepted".into(),
                    pointer: "".into()
                }
            )])
        )
        .is_err()
    );
    let mut bad_join = join.clone();
    bad_join.branches.insert(
        "accept".into(),
        Binding::Step {
            step: "rejected".into(),
            pointer: "".into(),
        },
    );
    nodes[3].join = Some(&bad_join);
    assert!(validate_routing_graph(&nodes, &empty).is_err());
    let outputs = BTreeMap::from([("accepted".into(), json!({"approved":true}))]);
    assert_eq!(
        resolve_join(&join, "accept", &json!({}), &outputs, &json!({})).unwrap(),
        json!({"route":"accept","output":{"approved":true}})
    );
    assert!(resolve_join(&join, "reject", &json!({}), &outputs, &json!({})).is_err());
}

#[test]
fn pinned_calls_reject_missing_versions_cycles_excess_depth_and_fanout() {
    let root = format!("sha256:{}", "a".repeat(64));
    let child = format!("sha256:{}", "b".repeat(64));
    let leaf = format!("sha256:{}", "c".repeat(64));
    let limits = CallGraphLimits {
        max_depth: 8,
        max_children: 256,
    };
    let mut graph = BTreeMap::from([
        (
            root.clone(),
            vec![CallTarget {
                process_ref: child.clone(),
                instances: 2,
            }],
        ),
        (
            child.clone(),
            vec![CallTarget {
                process_ref: leaf.clone(),
                instances: 3,
            }],
        ),
        (leaf.clone(), vec![]),
    ]);
    let pinned = validate_call_graph(&root, &graph, limits).unwrap();
    assert_eq!(pinned.worst_case_children, 8);
    assert_eq!(pinned.max_depth, 2);
    assert_eq!(pinned.versions.len(), 3);
    assert!(
        validate_call_graph(
            &root,
            &graph,
            CallGraphLimits {
                max_depth: 1,
                max_children: 256
            }
        )
        .is_err()
    );
    graph.get_mut(&leaf).unwrap().push(CallTarget {
        process_ref: root.clone(),
        instances: 1,
    });
    assert!(validate_call_graph(&root, &graph, limits).is_err());
    graph.remove(&leaf);
    assert!(validate_call_graph(&root, &graph, limits).is_err());
    graph.insert(leaf, vec![]);
    graph.get_mut(&root).unwrap()[0].instances = 100;
    assert!(validate_call_graph(&root, &graph, limits).is_err());
    let empty = BTreeMap::new();
    let unpinned = CallDefinition {
        process_ref: "latest".into(),
        output_pointer: "".into(),
    };
    let nodes = [RouteNode {
        id: "invoke",
        kind: "call",
        needs: &[],
        inputs: &empty,
        foreach_items: None,
        when: None,
        decision: None,
        join: None,
        call: Some(&unpinned),
    }];
    assert!(validate_routing_graph(&nodes, &empty).is_err());
}

fn version(index: usize) -> String {
    format!("sha256:{index:064x}")
}

fn call_chain(depth: usize) -> BTreeMap<String, Vec<CallTarget>> {
    (0..=depth)
        .map(|index| {
            let children = if index == depth {
                vec![]
            } else {
                vec![CallTarget {
                    process_ref: version(index + 1),
                    instances: 1,
                }]
            };
            (version(index), children)
        })
        .collect()
}

#[test]
fn explicit_call_limits_allow_128_edges_and_reject_a_129th() {
    let limits = CallGraphLimits {
        max_depth: 128,
        max_children: 4096,
    };
    let admitted = validate_call_graph(&version(0), &call_chain(128), limits).unwrap();
    assert_eq!(admitted.max_depth, 128);
    assert_eq!(admitted.worst_case_children, 128);
    assert_eq!(admitted.versions.len(), 129);
    assert!(validate_call_graph(&version(0), &call_chain(129), limits).is_err());
    assert!(
        validate_call_graph(
            &version(0),
            &call_chain(9),
            CallGraphLimits {
                max_depth: 8,
                max_children: 256,
            },
        )
        .is_err()
    );
}

#[test]
fn structural_analysis_counts_shared_subgraphs_once_and_calls_with_multiplicity() {
    let graph = BTreeMap::from([
        (
            version(0),
            vec![
                CallTarget {
                    process_ref: version(1),
                    instances: 2,
                },
                CallTarget {
                    process_ref: version(2),
                    instances: 3,
                },
                CallTarget {
                    process_ref: version(1),
                    instances: 5,
                },
            ],
        ),
        (
            version(1),
            vec![CallTarget {
                process_ref: version(3),
                instances: 7,
            }],
        ),
        (
            version(2),
            vec![CallTarget {
                process_ref: version(1),
                instances: 11,
            }],
        ),
        (version(3), vec![]),
    ]);
    let analyzed = analyze_call_graph(&version(0), &graph, 128).unwrap();
    // Seven calls to the shared first subtree each create eight runs. Three
    // calls to the second subtree each create 1 + 11 * 8 = 89 runs.
    assert_eq!(analyzed.worst_case_children, 323);
    assert_eq!(analyzed.max_depth, 3);
    assert_eq!(analyzed.versions, (0..4).map(version).collect::<Vec<_>>());
    assert_eq!(
        analyzed.postorder,
        vec![version(3), version(1), version(2), version(0)]
    );
    assert!(analyze_call_graph(&version(0), &graph, 2).is_err());
    assert!(
        validate_call_graph(
            &version(0),
            &graph,
            CallGraphLimits {
                max_depth: 128,
                max_children: 322
            }
        )
        .is_err()
    );
    assert_eq!(
        validate_call_graph(
            &version(0),
            &graph,
            CallGraphLimits {
                max_depth: 128,
                max_children: 323
            }
        )
        .unwrap(),
        analyzed
    );
    assert!(
        serde_json::to_value(&analyzed)
            .unwrap()
            .get("postorder")
            .is_none()
    );
}

#[test]
fn structural_analysis_saturates_cardinality_without_expanding_execution_paths() {
    let mut graph = call_chain(40);
    for edges in graph.values_mut() {
        if let Some(edge) = edges.first_mut() {
            edge.instances = 1000;
            let duplicate = edge.clone();
            edges.push(duplicate);
        }
    }
    let analyzed = analyze_call_graph(&version(0), &graph, 128).unwrap();
    assert_eq!(analyzed.worst_case_children, u32::MAX);
    assert_eq!(analyzed.max_depth, 40);
    assert_eq!(analyzed.versions.len(), 41);
    assert_eq!(
        analyzed.postorder,
        (0..=40).rev().map(version).collect::<Vec<_>>()
    );
    assert!(
        validate_call_graph(
            &version(0),
            &graph,
            CallGraphLimits {
                max_depth: 128,
                max_children: 4096
            }
        )
        .is_err()
    );
}

fn wide_graph(versions: usize) -> BTreeMap<String, Vec<CallTarget>> {
    let mut graph: BTreeMap<_, Vec<_>> = (0..versions)
        .map(|index| (version(index), vec![]))
        .collect();
    for index in 1..versions {
        graph
            .get_mut(&version((index - 1) / 100))
            .unwrap()
            .push(CallTarget {
                process_ref: version(index),
                instances: 1,
            });
    }
    graph
}

#[test]
fn reachable_version_limit_is_independent_of_depth_and_child_multiplicity() {
    let graph = wide_graph(4096);
    let analyzed = analyze_call_graph(&version(0), &graph, 128).unwrap();
    assert_eq!(analyzed.versions.len(), 4096);
    assert_eq!(analyzed.postorder.len(), 4096);
    assert_eq!(analyzed.max_depth, 2);
    assert_eq!(analyzed.worst_case_children, 4095);
    let error = analyze_call_graph(&version(0), &wide_graph(4097), 128).unwrap_err();
    assert!(error.to_string().contains("4096 distinct versions"));

    let mut catalog = graph;
    catalog.insert(
        version(4096),
        vec![CallTarget {
            process_ref: "unreachable-invalid-reference".into(),
            instances: 0,
        }],
    );
    assert_eq!(
        analyze_call_graph(&version(0), &catalog, 128).unwrap(),
        analyzed
    );
}

#[test]
fn iterative_analysis_rejects_cycles_missing_versions_and_invalid_expansion_bounds() {
    let mut graph = call_chain(127);
    graph.get_mut(&version(127)).unwrap().push(CallTarget {
        process_ref: version(0),
        instances: 1,
    });
    assert!(
        analyze_call_graph(&version(0), &graph, 128)
            .unwrap_err()
            .to_string()
            .contains("recursive")
    );
    graph.get_mut(&version(127)).unwrap()[0].process_ref = version(127);
    assert!(
        analyze_call_graph(&version(0), &graph, 128)
            .unwrap_err()
            .to_string()
            .contains("recursive")
    );
    graph.get_mut(&version(127)).unwrap()[0].process_ref = version(128);
    assert!(
        analyze_call_graph(&version(0), &graph, 128)
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
    assert!(analyze_call_graph(&version(999), &graph, 128).is_err());
    assert!(analyze_call_graph("latest", &graph, 128).is_err());
    for invalid in [0, 1001] {
        let mut graph = call_chain(1);
        graph.get_mut(&version(0)).unwrap()[0].instances = invalid;
        assert!(analyze_call_graph(&version(0), &graph, 128).is_err());
    }
    let mut graph = call_chain(1);
    graph.insert(
        version(0),
        vec![
            CallTarget {
                process_ref: version(1),
                instances: 1
            };
            101
        ],
    );
    assert!(analyze_call_graph(&version(0), &graph, 128).is_err());
    for depth in [0, 129] {
        assert!(analyze_call_graph(&version(0), &call_chain(0), depth).is_err());
    }
    assert!(
        validate_call_graph(
            &version(0),
            &call_chain(0),
            CallGraphLimits {
                max_depth: 128,
                max_children: 4097
            }
        )
        .is_err()
    );
}

fn capacity_process() -> Value {
    json!({"schema_version":"0.2","name":"capacity.fixture","description":"Synthetic capacity fixture",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"builtin"},{"kind":"artifacts"}],
        "limits":{"max_step_ms":5000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[{"id":"snapshot","kind":"builtin","operation":"json.write","needs":[],"effect":"pure","retry":{"max_attempts":1},
            "inputs":{"value":{"input":""}}}],
        "output":{"artifact":{"step":"snapshot","pointer":"/artifact"}}})
}

#[test]
fn expanded_manifest_limits_keep_omitted_limits_at_legacy_defaults() {
    let original = capacity_process();
    let admitted = admit_value(&original).unwrap();
    assert_eq!(admitted.manifest.limits.max_call_depth, 8);
    assert_eq!(admitted.manifest.limits.max_child_runs, 256);
    assert_eq!(admitted.manifest.limits.max_step_instances, 10_000);
    assert_eq!(
        serde_json::to_value(&admitted.manifest.limits).unwrap(),
        original["limits"]
    );

    let mut expanded = original;
    expanded["limits"]["max_call_depth"] = json!(128);
    expanded["limits"]["max_child_runs"] = json!(4096);
    expanded["limits"]["max_step_instances"] = json!(100_000);
    let admitted = admit_value(&expanded).unwrap();
    assert_eq!(admitted.manifest.limits.max_call_depth, 128);
    assert_eq!(admitted.manifest.limits.max_child_runs, 4096);
    assert_eq!(admitted.manifest.limits.max_step_instances, 100_000);
    for (field, invalid) in [
        ("max_call_depth", 129),
        ("max_child_runs", 4097),
        ("max_step_instances", 100_001),
    ] {
        let mut excessive = expanded.clone();
        excessive["limits"][field] = json!(invalid);
        assert!(
            admit_value(&excessive).is_err(),
            "{field} accepted an excessive limit"
        );
    }
}

#[test]
fn exclusive_foreach_alternatives_fit_but_parallel_or_same_route_expansions_add() {
    let mut process = capacity_process();
    process["limits"]["max_step_instances"] = json!(1003);
    process["steps"] = json!([
        {"id":"route","kind":"decision","effect":"pure","needs":[],"retry":{"max_attempts":1},"inputs":{},
         "decision":{"source":"rules","routes":["left","right"],"cases":[],"default":"left"}},
        {"id":"left","kind":"builtin","operation":"json.write","needs":["route"],"effect":"pure","retry":{"max_attempts":1},
         "when":{"decision":"route","route":"left"},"inputs":{"value":{"item":""}},
         "foreach":{"items":{"literal":[]},"key":"/id","max_items":1000,"concurrency":1}},
        {"id":"right","kind":"builtin","operation":"json.write","needs":["route"],"effect":"pure","retry":{"max_attempts":1},
         "when":{"decision":"route","route":"right"},"inputs":{"value":{"item":""}},
         "foreach":{"items":{"literal":[]},"key":"/id","max_items":1000,"concurrency":1}}
    ]);
    process["output"] = json!({"value":{"literal":true}});
    admit_value(&process).unwrap();

    let mut same_route = process.clone();
    same_route["steps"][2]["when"]["route"] = json!("left");
    assert!(admit_value(&same_route).is_err());
    let mut parallel = process.clone();
    parallel["steps"][2].as_object_mut().unwrap().remove("when");
    assert!(admit_value(&parallel).is_err());
    process["limits"]["max_step_instances"] = json!(1002);
    assert!(
        admit_value(&process).is_err(),
        "skipped definitions still need their base step rows"
    );
}

fn routed_manifest() -> Value {
    json!({"schema_version":"0.2","name":"route.fixture","description":"Choose a declared branch",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{},"run_override_keys":[]},"capabilities":[{"kind":"builtin"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1000000,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[
            {"id":"gate","kind":"decision","effect":"pure","needs":[],"inputs":{},"retry":{"max_attempts":1},
                "decision":{"source":"rules","routes":["yes","no"],"cases":[{"value":{"input":"/accept"},"equals":true,"route":"yes"}],"default":"no"}},
            {"id":"yes","kind":"builtin","operation":"json.select","effect":"pure","needs":["gate"],"retry":{"max_attempts":1},"when":{"decision":"gate","route":"yes"},"inputs":{"value":{"literal":{"approved":true}},"pointer":{"literal":""}}},
            {"id":"no","kind":"builtin","operation":"json.select","effect":"pure","needs":["gate"],"retry":{"max_attempts":1},"when":{"decision":"gate","route":"no"},"inputs":{"value":{"literal":{"approved":false}},"pointer":{"literal":""}}},
            {"id":"merge","kind":"join","effect":"pure","needs":["gate","yes","no"],"retry":{"max_attempts":1},"inputs":{},"join":{"decision":"gate","branches":{"yes":{"step":"yes","pointer":"/value"},"no":{"step":"no","pointer":"/value"}}}}
        ],"output":{"result":{"step":"merge","pointer":""}}})
}

#[test]
fn version_two_admits_routing_only_after_shape_and_branch_validation() {
    let manifest = routed_manifest();
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/process-v0.2.schema.json")).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&manifest)
        .unwrap();
    let admitted = apr_core::admit_value(&manifest).unwrap();
    assert_eq!(admitted.manifest.schema_version, "0.2");
    let mut old = manifest.clone();
    old["schema_version"] = json!("0.1");
    assert!(apr_core::admit_value(&old).is_err());
    let mut broken = manifest.clone();
    broken["steps"][1]["when"]["route"] = json!("absent");
    assert!(apr_core::admit_value(&broken).is_err());
    broken = manifest;
    broken["steps"][0]["decision"]["cases"][0]["value"] = json!({"step":"yes","pointer":""});
    assert!(apr_core::admit_value(&broken).is_err());
}
