//! Bounded deterministic route policies. This module never dispatches work.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Binding, CoreError, canonical_bytes, content_digest, resolve_bindings_with_context,
    validate_binding,
};

const MAX_POLICY_BYTES: usize = 48 * 1024;

/// Explicit limits may raise the legacy defaults up to these hard ceilings.
pub const MAX_CALL_DEPTH: u32 = 128;
pub const MAX_CHILD_RUNS: u32 = 4096;
pub const MAX_STEP_INSTANCES: u32 = 100_000;
/// Counts distinct reachable versions, including the root, independently of runs.
pub const MAX_GRAPH_VERSIONS: u32 = 4096;

const MAX_CALL_DEFINITIONS: usize = 100;
const MAX_CALL_INSTANCES: u32 = 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionPolicy {
    Rules {
        routes: Vec<String>,
        cases: Vec<RuleCase>,
        default: String,
    },
    Choice {
        routes: Vec<String>,
        value: Binding,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<Binding>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_confidence: Option<f64>,
        no_match: String,
        needs_clarification: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleCase {
    pub value: Binding,
    pub equals: Value,
    pub route: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchWhen {
    pub decision: String,
    pub route: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinDefinition {
    pub decision: String,
    pub branches: BTreeMap<String, Binding>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallDefinition {
    pub process_ref: String,
    #[serde(default)]
    pub output_pointer: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionResolution {
    pub source: String,
    pub route: String,
    pub reason: String,
    pub candidates: Vec<String>,
    pub input_digest: String,
    pub policy_digest: String,
    pub observation: Value,
}

#[derive(Clone, Debug)]
pub struct RouteNode<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub needs: &'a [String],
    pub inputs: &'a BTreeMap<String, Binding>,
    pub foreach_items: Option<&'a Binding>,
    pub when: Option<&'a BranchWhen>,
    pub decision: Option<&'a DecisionPolicy>,
    pub join: Option<&'a JoinDefinition>,
    pub call: Option<&'a CallDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallTarget {
    pub process_ref: String,
    pub instances: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct CallGraphLimits {
    pub max_depth: u32,
    pub max_children: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PinnedCallGraph {
    pub versions: Vec<String>,
    pub max_depth: u32,
    /// All-branches cardinality, saturated at u32::MAX if the exact count is larger.
    pub worst_case_children: u32,
    /// Each reachable version once, children before parents. Analysis-only data
    /// keeps the existing serialized graph descriptor unchanged.
    #[serde(skip)]
    pub postorder: Vec<String>,
}

pub fn validate_routing_graph(
    nodes: &[RouteNode<'_>],
    output: &BTreeMap<String, Binding>,
) -> Result<(), CoreError> {
    if nodes.is_empty() || nodes.len() > 100 {
        return Err(CoreError::new(
            "/steps",
            "routing requires one to 100 step definitions",
        ));
    }
    let by_id: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect();
    if by_id.len() != nodes.len() {
        return Err(CoreError::new("/steps", "step ids must be unique"));
    }
    for node in nodes {
        if node.needs.iter().collect::<BTreeSet<_>>().len() != node.needs.len()
            || node.needs.iter().any(|id| !by_id.contains_key(id.as_str()))
        {
            return Err(CoreError::new(
                "/steps/needs",
                "dependencies must be unique declared step ids",
            ));
        }
        for (kind, present) in [
            ("decision", node.decision.is_some()),
            ("join", node.join.is_some()),
            ("call", node.call.is_some()),
        ] {
            if (node.kind == kind) != present {
                return Err(CoreError::new(
                    "/steps",
                    "routing fields must match their step kind",
                ));
            }
        }
        if matches!(node.kind, "decision" | "join")
            && (!node.inputs.is_empty() || node.foreach_items.is_some())
        {
            return Err(CoreError::new(
                "/steps",
                "decisions and joins use their declared bindings and cannot expand foreach",
            ));
        }
        if let Some(policy) = node.decision {
            validate_policy(policy)?;
        }
        if let Some(call) = node.call {
            validate_process_ref(&call.process_ref)?;
            crate::validate_pointer(&call.output_pointer, "/call/output_pointer")?;
        }
    }
    let mut order = Vec::new();
    let mut ancestors = vec![BTreeSet::new(); nodes.len()];
    let mut done = BTreeSet::new();
    while done.len() < nodes.len() {
        let before = done.len();
        for (index, node) in nodes.iter().enumerate() {
            if done.contains(&index)
                || !node
                    .needs
                    .iter()
                    .all(|id| done.contains(&by_id[id.as_str()]))
            {
                continue;
            }
            for dependency in node.needs {
                let dependency = by_id[dependency.as_str()];
                let inherited = ancestors[dependency].clone();
                ancestors[index].extend(inherited);
                ancestors[index].insert(dependency);
            }
            done.insert(index);
            order.push(index);
        }
        if before == done.len() {
            return Err(CoreError::new("/steps/needs", "step dependency cycle"));
        }
    }
    let mut guards = vec![BTreeMap::<String, String>::new(); nodes.len()];
    for index in order {
        let node = &nodes[index];
        let mut required = BTreeMap::new();
        if let Some(when) = node.when {
            let gate = decision_ancestor(&when.decision, index, nodes, &by_id, &ancestors)?;
            if !nodes[gate]
                .decision
                .expect("checked decision")
                .routes()
                .contains(&when.route)
            {
                return Err(CoreError::new(
                    "/when/route",
                    "branch route is not declared by its decision",
                ));
            }
            required = guards[gate].clone();
            required.insert(when.decision.clone(), when.route.clone());
        }
        if let Some(join) = node.join {
            let gate = decision_ancestor(&join.decision, index, nodes, &by_id, &ancestors)?;
            for (decision, route) in &guards[gate] {
                if required.get(decision).is_some_and(|value| value != route) {
                    return Err(CoreError::new(
                        "/join",
                        "join has contradictory branch conditions",
                    ));
                }
                required.insert(decision.clone(), route.clone());
            }
            if join.branches.keys().collect::<BTreeSet<_>>()
                != nodes[gate]
                    .decision
                    .expect("checked decision")
                    .routes()
                    .iter()
                    .collect::<BTreeSet<_>>()
            {
                return Err(CoreError::new(
                    "/join/branches",
                    "join must explicitly cover every declared route",
                ));
            }
        }
        guards[index] = required;
        for dependency in node.needs {
            let source = &guards[by_id[dependency.as_str()]];
            let permitted = source.iter().all(|(decision, route)| {
                guards[index].get(decision) == Some(route)
                    || node.join.is_some_and(|join| join.decision == *decision)
            });
            if !permitted {
                return Err(CoreError::new(
                    "/steps/needs",
                    "conditional dependencies require a matching branch or explicit join",
                ));
            }
        }
        for binding in node
            .inputs
            .values()
            .chain(node.decision.into_iter().flat_map(DecisionPolicy::bindings))
        {
            check_source(
                binding,
                &guards[index],
                Some(&ancestors[index]),
                &by_id,
                &guards,
                node.foreach_items.is_some(),
            )?;
        }
        if let Some(binding) = node.foreach_items {
            check_source(
                binding,
                &guards[index],
                Some(&ancestors[index]),
                &by_id,
                &guards,
                false,
            )?;
        }
        if let Some(join) = node.join {
            for (route, binding) in &join.branches {
                let mut selected = guards[index].clone();
                selected.insert(join.decision.clone(), route.clone());
                check_source(
                    binding,
                    &selected,
                    Some(&ancestors[index]),
                    &by_id,
                    &guards,
                    false,
                )?;
            }
        }
    }
    for binding in output.values() {
        check_source(binding, &BTreeMap::new(), None, &by_id, &guards, false)?;
    }
    Ok(())
}

pub fn validate_call_graph(
    root: &str,
    graph: &BTreeMap<String, Vec<CallTarget>>,
    limits: CallGraphLimits,
) -> Result<PinnedCallGraph, CoreError> {
    if limits.max_children > MAX_CHILD_RUNS {
        return Err(CoreError::new(
            "/limits",
            "child run limits must stay within 4096 children",
        ));
    }
    let analyzed = analyze_call_graph(root, graph, limits.max_depth)?;
    if analyzed.worst_case_children > limits.max_children {
        return Err(CoreError::new(
            "/call",
            "child expansion exceeds the declared total child budget",
        ));
    }
    Ok(analyzed)
}

/// Validate reachable call structure without rejecting an all-branches child
/// count. Callers with routing information can use `postorder` to compute a
/// tighter execution bound, then enforce their declared run and step budgets.
///
/// Each distinct version is visited once. At most 4096 versions with 100 call
/// definitions each are inspected; multiplicities never expand graph work.
/// The root is depth zero, and `max_depth` counts nested call edges.
pub fn analyze_call_graph(
    root: &str,
    graph: &BTreeMap<String, Vec<CallTarget>>,
    max_depth: u32,
) -> Result<PinnedCallGraph, CoreError> {
    if max_depth == 0 || max_depth > MAX_CALL_DEPTH {
        return Err(CoreError::new(
            "/limits",
            "call depth limits must stay within one to 128 levels",
        ));
    }

    struct Frame<'a> {
        version: &'a str,
        edges: &'a [CallTarget],
        next_edge: usize,
    }

    let mut stack = vec![Frame {
        version: root,
        edges: call_edges(root, graph)?,
        next_edge: 0,
    }];
    let mut visiting = BTreeSet::from([root]);
    let mut bounds = BTreeMap::<&str, (u32, u32)>::new();
    let mut postorder = Vec::new();
    while let Some(frame) = stack.last_mut() {
        if let Some(edge) = frame.edges.get(frame.next_edge) {
            frame.next_edge += 1;
            if edge.instances == 0 || edge.instances > MAX_CALL_INSTANCES {
                return Err(CoreError::new(
                    "/call/instances",
                    "call expansion must have a finite bound of one to 1000",
                ));
            }
            let child = edge.process_ref.as_str();
            if visiting.contains(child) {
                return Err(CoreError::new(
                    "/call",
                    "recursive child process calls are forbidden",
                ));
            }
            if bounds.contains_key(child) {
                continue;
            }
            if visiting.len() + bounds.len() >= MAX_GRAPH_VERSIONS as usize {
                return Err(CoreError::new(
                    "/call",
                    "reachable call graph exceeds 4096 distinct versions",
                ));
            }
            if stack.len() > max_depth as usize {
                return Err(CoreError::new(
                    "/call",
                    "child call depth exceeds the declared limit",
                ));
            }
            let edges = call_edges(child, graph)?;
            visiting.insert(child);
            stack.push(Frame {
                version: child,
                edges,
                next_edge: 0,
            });
            continue;
        }

        let frame = stack.pop().expect("current call frame exists");
        let mut children = 0u32;
        let mut depth = 0u32;
        for edge in frame.edges {
            let &(descendants, child_depth) = bounds
                .get(edge.process_ref.as_str())
                .expect("child call was analyzed before its parent");
            children = children.saturating_add(
                edge.instances
                    .saturating_mul(1u32.saturating_add(descendants)),
            );
            depth = depth.max(child_depth.saturating_add(1));
        }
        // Relative subtree depth also covers a memoized child reached later
        // through a longer parent path, independently of traversal order.
        if depth > max_depth {
            return Err(CoreError::new(
                "/call",
                "child call depth exceeds the declared limit",
            ));
        }
        visiting.remove(frame.version);
        bounds.insert(frame.version, (children, depth));
        postorder.push(frame.version.to_owned());
    }
    let &(worst_case_children, max_depth) = bounds.get(root).expect("root call was analyzed");
    Ok(PinnedCallGraph {
        versions: bounds.keys().map(|version| (*version).to_owned()).collect(),
        max_depth,
        worst_case_children,
        postorder,
    })
}

fn call_edges<'a>(
    version: &str,
    graph: &'a BTreeMap<String, Vec<CallTarget>>,
) -> Result<&'a [CallTarget], CoreError> {
    validate_process_ref(version)?;
    let edges = graph.get(version).ok_or_else(|| {
        CoreError::new(
            "/call/process_ref",
            "reachable child version is missing from the pinned catalog",
        )
    })?;
    if edges.len() > MAX_CALL_DEFINITIONS {
        return Err(CoreError::new(
            "/call",
            "a process has more than 100 call definitions",
        ));
    }
    Ok(edges)
}

pub fn resolve_join(
    definition: &JoinDefinition,
    route: &str,
    input: &Value,
    outputs: &BTreeMap<String, Value>,
    config: &Value,
) -> Result<Value, CoreError> {
    let binding = definition.branches.get(route).ok_or_else(|| {
        CoreError::new(
            "/join/branches",
            "committed decision selected an undeclared join route",
        )
    })?;
    let resolved = resolve_bindings_with_context(
        &BTreeMap::from([("output".into(), binding.clone())]),
        input,
        outputs,
        config,
        None,
    )?;
    let output = json!({"route":route,"output":resolved["output"]});
    bounded(&output)?;
    Ok(output)
}

fn validate_process_ref(version: &str) -> Result<(), CoreError> {
    if version.len() != 71
        || !version.starts_with("sha256:")
        || !version.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(CoreError::new(
            "/call/process_ref",
            "child process must name an immutable sha256 version",
        ));
    }
    Ok(())
}

fn decision_ancestor(
    id: &str,
    consumer: usize,
    nodes: &[RouteNode<'_>],
    by_id: &BTreeMap<&str, usize>,
    ancestors: &[BTreeSet<usize>],
) -> Result<usize, CoreError> {
    let index = *by_id
        .get(id)
        .ok_or_else(|| CoreError::new("/decision", "referenced decision does not exist"))?;
    if !ancestors[consumer].contains(&index) || nodes[index].decision.is_none() {
        return Err(CoreError::new(
            "/decision",
            "route must refer to a declared ancestor decision",
        ));
    }
    Ok(index)
}

fn check_source(
    binding: &Binding,
    required: &BTreeMap<String, String>,
    ancestors: Option<&BTreeSet<usize>>,
    by_id: &BTreeMap<&str, usize>,
    guards: &[BTreeMap<String, String>],
    allow_item: bool,
) -> Result<(), CoreError> {
    validate_binding(binding, "/bindings")?;
    match binding {
        Binding::Item { .. } if !allow_item => {
            return Err(CoreError::new(
                "/bindings",
                "item binding requires this step's foreach context",
            ));
        }
        Binding::Step { step, .. } => {
            let source = *by_id
                .get(step.as_str())
                .ok_or_else(|| CoreError::new("/bindings", "bound step is missing"))?;
            if ancestors.is_some_and(|ancestors| !ancestors.contains(&source)) {
                return Err(CoreError::new(
                    "/bindings",
                    "binding must name a declared ancestor",
                ));
            }
            if !guards[source]
                .iter()
                .all(|(decision, route)| required.get(decision) == Some(route))
            {
                return Err(CoreError::new(
                    "/bindings",
                    "conditional output is not available on every selected path; use an explicit join",
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

impl DecisionPolicy {
    pub fn routes(&self) -> &[String] {
        match self {
            Self::Rules { routes, .. } | Self::Choice { routes, .. } => routes,
        }
    }

    pub fn bindings(&self) -> Vec<&Binding> {
        match self {
            Self::Rules { cases, .. } => cases.iter().map(|case| &case.value).collect(),
            Self::Choice {
                value, confidence, ..
            } => std::iter::once(value).chain(confidence.iter()).collect(),
        }
    }
}

pub fn validate_policy(policy: &DecisionPolicy) -> Result<(), CoreError> {
    bounded(
        &serde_json::to_value(policy)
            .map_err(|error| CoreError::new("/decision", error.to_string()))?,
    )?;
    let routes = policy.routes();
    if routes.is_empty()
        || routes.len() > 32
        || routes.iter().collect::<BTreeSet<_>>().len() != routes.len()
    {
        return Err(CoreError::new(
            "/decision/routes",
            "declare one to 32 unique routes",
        ));
    }
    for route in routes {
        validate_label(route)?;
    }
    match policy {
        DecisionPolicy::Rules { cases, default, .. } => {
            if cases.len() > 128 {
                return Err(CoreError::new(
                    "/decision/cases",
                    "at most 128 cases are supported",
                ));
            }
            if !routes.contains(default) || cases.iter().any(|case| !routes.contains(&case.route)) {
                return Err(CoreError::new(
                    "/decision",
                    "case and default routes must be declared candidates",
                ));
            }
        }
        DecisionPolicy::Choice {
            confidence,
            min_confidence,
            no_match,
            needs_clarification,
            ..
        } => {
            if !routes.contains(no_match) || !routes.contains(needs_clarification) {
                return Err(CoreError::new(
                    "/decision",
                    "fallback routes must be declared candidates",
                ));
            }
            if min_confidence
                .is_some_and(|minimum| !minimum.is_finite() || !(0.0..=1.0).contains(&minimum))
            {
                return Err(CoreError::new(
                    "/decision/min_confidence",
                    "confidence threshold must be finite and between zero and one",
                ));
            }
            if min_confidence.is_some() && confidence.is_none() {
                return Err(CoreError::new(
                    "/decision/confidence",
                    "a threshold requires an explicit confidence binding",
                ));
            }
        }
    }
    for binding in policy.bindings() {
        validate_binding(binding, "/decision")?;
    }
    Ok(())
}

pub fn evaluate_decision(
    policy: &DecisionPolicy,
    input: &Value,
    outputs: &BTreeMap<String, Value>,
    config: &Value,
    item: Option<&Value>,
) -> Result<DecisionResolution, CoreError> {
    validate_policy(policy)?;
    let (source, route, reason, observation) = match policy {
        DecisionPolicy::Rules { cases, default, .. } => {
            let mut values = Vec::new();
            let mut selected = None;
            for (index, case) in cases.iter().enumerate() {
                let resolved = resolve_bindings_with_context(
                    &BTreeMap::from([("value".into(), case.value.clone())]),
                    input,
                    outputs,
                    config,
                    item,
                )?;
                let value = resolved["value"].clone();
                bounded(&value)?;
                let matches = canonical_bytes(&value)? == canonical_bytes(&case.equals)?;
                values.push(value);
                bounded(&json!({"values":values}))?;
                if matches {
                    selected = Some((index, case.route.clone()));
                    break;
                }
            }
            let (route, reason) = selected
                .as_ref()
                .map(|(_, route)| (route.clone(), "matched"))
                .unwrap_or_else(|| (default.clone(), "default"));
            (
                "rules",
                route,
                reason,
                json!({"values":values,"matched_case":selected.map(|(index,_)|index)}),
            )
        }
        DecisionPolicy::Choice {
            routes,
            value,
            confidence,
            min_confidence,
            no_match,
            needs_clarification,
        } => {
            let resolved = resolve_bindings_with_context(
                &BTreeMap::from([("value".into(), value.clone())]),
                input,
                outputs,
                config,
                item,
            )?;
            let selected = resolved["value"].as_str().ok_or_else(|| {
                CoreError::new("/decision/value", "choice must be a route label string")
            })?;
            let observed_confidence = confidence
                .as_ref()
                .map(|binding| optional_projection(binding, input, outputs, config, item))
                .transpose()?
                .flatten();
            let probability = observed_confidence
                .as_ref()
                .map(|value| {
                    value
                        .as_f64()
                        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
                        .ok_or_else(|| {
                            CoreError::new(
                                "/decision/confidence",
                                "confidence must be a number between zero and one",
                            )
                        })
                })
                .transpose()?;
            let (route, reason) = if !routes.iter().any(|route| route == selected) {
                (no_match.clone(), "no_match")
            } else if min_confidence
                .is_some_and(|minimum| probability.is_none_or(|value| value < minimum))
            {
                (needs_clarification.clone(), "needs_clarification")
            } else {
                (selected.to_owned(), "selected")
            };
            (
                "choice",
                route,
                reason,
                json!({"value":selected,"confidence":observed_confidence}),
            )
        }
    };
    let resolution = DecisionResolution {
        source: source.into(),
        route,
        reason: reason.into(),
        candidates: policy.routes().to_vec(),
        input_digest: content_digest(&observation)?,
        policy_digest: content_digest(
            &serde_json::to_value(policy)
                .map_err(|error| CoreError::new("/decision", error.to_string()))?,
        )?,
        observation,
    };
    bounded(
        &serde_json::to_value(&resolution)
            .map_err(|error| CoreError::new("/decision", error.to_string()))?,
    )?;
    Ok(resolution)
}

fn validate_label(label: &str) -> Result<(), CoreError> {
    if label.is_empty()
        || label.len() > 64
        || !label.as_bytes()[0].is_ascii_lowercase()
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(CoreError::new(
            "/decision/routes",
            "route labels must be lowercase identifiers of at most 64 bytes",
        ));
    }
    Ok(())
}

fn bounded(value: &Value) -> Result<(), CoreError> {
    if canonical_bytes(value)?.len() > MAX_POLICY_BYTES {
        return Err(CoreError::new(
            "/decision",
            "decision policy or evidence exceeds 48 KiB",
        ));
    }
    Ok(())
}

fn optional_projection(
    binding: &Binding,
    input: &Value,
    outputs: &BTreeMap<String, Value>,
    config: &Value,
    item: Option<&Value>,
) -> Result<Option<Value>, CoreError> {
    validate_binding(binding, "/decision/confidence")?;
    let value = match binding {
        Binding::Literal { literal } => Some(literal),
        Binding::Input { input: pointer } => input.pointer(pointer),
        Binding::Config { config: pointer } => config.pointer(pointer),
        Binding::Item { item: pointer } => item
            .ok_or_else(|| CoreError::new("/decision/confidence", "item context is unavailable"))?
            .pointer(pointer),
        Binding::Step { step, pointer } => outputs
            .get(step)
            .ok_or_else(|| {
                CoreError::new(
                    "/decision/confidence",
                    "source step has no committed output",
                )
            })?
            .pointer(pointer),
    };
    value
        .map(|value| bounded(value).map(|_| value.clone()))
        .transpose()
}
