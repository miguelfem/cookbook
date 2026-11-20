//! Version-matched, on-demand authoring knowledge shipped in the MCP binary.
//! No files are read from the caller's project and no provider is contacted.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Topic {
    Overview,
    WorkflowDesign,
    Authoring,
    ExecutionReview,
    Schema,
    SchemaV0_1,
    Starter,
    Routing,
    Jev,
    ClassifierLearning,
    RecipeImprovement,
    Connections,
    Adapters,
    Handoffs,
    Schedules,
}

fn embedded_json(source: &str) -> Value {
    // Embedded documents are parsed and admitted in the protocol regression suite.
    serde_json::from_str(source).expect("bundled CookBook contract is valid JSON")
}

pub(crate) fn read(topic: Topic, service: &Value, mode: &str) -> Value {
    let mut response = json!({
        "guide": topic,
        "guide_revision": "2026-09-22",
        "mcp_version": env!("CARGO_PKG_VERSION"),
        "tool_mode": mode,
        "runtime": {
            "version": service.get("runtime_version"),
            "features": {"jev":service.pointer("/features/jev"),"classifier_learning":service.pointer("/features/classifier_learning"),"connection_library":service.pointer("/features/connection_library"),"recipe_improvement":service.pointer("/features/recipe_improvement"),"parallel_execution":service.pointer("/features/parallel_execution"),"max_parallel_workers":service.pointer("/features/max_parallel_workers"),"supported_parallel_workers":service.pointer("/features/supported_parallel_workers")},
            "feature_scope":"Running service, not merely the MCP executable. Null means no project is connected yet or an older service did not report the feature. Inclusion does not establish configuration, authorization, live account health or quality."
        }
    });
    let content = match topic {
        Topic::Overview => json!({
            "purpose":"CookBook preserves solved workflows as callable recipes. The agent derives a recipe from observed work; the local runtime executes its approved steps, retains intermediate artifacts and returns a result or a precise request for judgment. One recipe can combine several tools behind a stable input/output contract. It executes the graph you author, not an automatically optimized conversation.",
            "possibilities":[
                {"use":"Replace repeated tool orchestration with one operation","design":"Combine deterministic code, approved MCP/API services and pinned CLI adapters; keep intermediate data in artifacts.","guide":"workflow_design"},
                {"use":"Use a particular external model inside a workflow","design":"Call an approved LLM or generation API with explicit provider/model and supported parameters; retain validated output and usage.","guide":"connections"},
                {"use":"Add a bounded semantic decision or score","design":"Optional Jev Choice, Score or Noul nodes can return data or feed declared routes; qualify the exact question and model.","guide":"jev"},
                {"use":"Run a batch of independent jobs","design":"Compose pinned recipes, declare dependencies, use bounded parallel workers and return keyed results together.","guide":"routing"},
                {"use":"Keep the calling agent for its native tools or open judgment","design":"Give it a focused request and evidence; execute the surrounding routine work in the recipe.","guide":"handoffs"},
                {"use":"Reuse approved tools and accounts across projects","design":"The agent connects its project and binds an owner-approved tool revision; credentials stay outside recipes and chat.","guide":"connections"},
                {"use":"Replay observed browser work or schedule a recipe","design":"Optional prepared browser/native scaffolds and durable schedules have explicit host, permission and capability limits.","guide":"schedules"},
                {"use":"Improve a recipe using recorded outcomes","design":"Propose and evaluate a candidate, then explicitly adopt it; optional learning from Other has its own guide.","guide":"recipe_improvement"}
            ],
            "guides":[
                {"guide":"workflow_design","when":"Choosing recipe capabilities or migrating a solved procedure and its agent instructions"},
                {"guide":"authoring","when":"Creating or changing a recipe"},
                {"guide":"execution_review","when":"Auditing caller boundaries, service automation, output budgets and changed context"},
                {"guide":"schema","when":"Exact manifest 0.2 fields"},
                {"guide":"schema_v0_1","when":"Maintaining a legacy 0.1 recipe"},
                {"guide":"starter","when":"First account-free recipe and input"},
                {"guide":"routing","when":"Rules, branches, nested recipes and caller review"},
                {"guide":"jev","when":"Optional Jev question/score nodes, returned observations and declared routes"},
                {"guide":"classifier_learning","when":"User-enabled collection of Other cases, category proposals and held-out evaluation"},
                {"guide":"recipe_improvement","when":"Inspecting retained runs, proposing, evaluating and explicitly adopting a recipe improvement"},
                {"guide":"connections","when":"Preparing MCP/API/CLI connections and reviewed adapters, choosing model APIs, requesting access and analyzing costs"},
                {"guide":"adapters","when":"Preparing and checking a fixed CLI/SDK adapter with the installed CLI, then proposing its exact connection"},
                {"guide":"handoffs","when":"Returning an observed agent result or handling uncertainty"},
                {"guide":"schedules","when":"User-requested schedules or computer-use workflows"}
            ],
            "next":"Reuse a known recipe reference and input contract directly. For a new or migrated workflow, read workflow_design and authoring, then only the needed contract/example. Inspect the running service and selected connection: documented capability, configuration, authorization and proven quality are separate. apr_run returns completed output when ready; inspect unfinished or truncated results using view=run and run_id. Author mode and project file access are required to register recipes.",
            "authority":"One attached assignment; no inherited client credentials. Discover tools explicitly approved for all projects with apr_inspect connection_library, then bind an exact revision with apr_manage connection.use. Publishing or expanding authority remains owner-only."
        }),
        Topic::WorkflowDesign => json!({
            "text":include_str!("../../../integrations/knowledge/workflow-design.md")
        }),
        Topic::Authoring => {
            json!({
                "text":include_str!("../../../integrations/knowledge/authoring.md"),
                "reference_first_template": {
                    "readme":include_str!("../../../examples/reference-first/README.md"),
                    "contract":embedded_json(include_str!("../../../examples/reference-first/solved-workflow-reference.json")),
                    "evidence_note":"This is a synthetic template requiring observed evidence. It is not a completed run, provider-quality result, or automatic trace compilation. Replace placeholders only with evidence the authorized agent actually observed; otherwise keep them unknown."
                }
            })
        }
        Topic::ExecutionReview => json!({
            "text":"Use workflow_design to map the whole procedure and migrate its execution instructions. Count actual caller work inside handoffs, not just agent steps; record baseline and expected model calls separately from raw HTTP calls and setup. Save a review sidecar using these examples, then call apr_manage with action process.audit, a project-relative manifest path, and the sidecar object as review. Findings inspect declared topology and metadata; they are advisory, not permission or proof of savings. These complete synthetic examples illustrate service-only work, justified judgment and compact context with full raw artifacts. Inspect the chosen connection contracts and obtain missing approvals before execution. The Python helper requires the owner's pinned interpreter grant.",
            "service":{"manifest":embedded_json(include_str!("../../../examples/execution-boundary-review/service-process.json")),"review":embedded_json(include_str!("../../../examples/execution-boundary-review/service-review.json"))},
            "mixed":{"manifest":embedded_json(include_str!("../../../examples/execution-boundary-review/mixed-process.json")),"review":embedded_json(include_str!("../../../examples/execution-boundary-review/mixed-review.json"))},
            "context":{"manifest":embedded_json(include_str!("../../../examples/execution-boundary-review/context-process.json")),"review":embedded_json(include_str!("../../../examples/execution-boundary-review/context-review.json")),"files":[{"path":"context-helper.py","text":include_str!("../../../examples/execution-boundary-review/context-helper.py")}]}
        }),
        Topic::Schema => {
            json!({"schema":embedded_json(include_str!("../../../schemas/process-v0.2.schema.json"))})
        }
        Topic::SchemaV0_1 => {
            json!({"schema":embedded_json(include_str!("../../../schemas/process.schema.json"))})
        }
        Topic::Starter => json!({
            "text":"Save this complete manifest under recipes/save-note/process.json in your attached project, validate and register it through apr_manage, then preflight and run with the sample input. It needs no provider or account. The output is an artifact reference; use apr_inspect view artifact for a bounded preview.",
            "manifest":embedded_json(include_str!("../../../examples/first-recipe/process.json")),
            "sample_input":{"note":"My first CookBook recipe"}
        }),
        Topic::Routing => json!({
            "text":include_str!("../../../integrations/knowledge/routing.md"),
            "manifest":embedded_json(include_str!("../../../examples/operations-intake/process.json")),
            "sample_input":{"severity":"routine","summary":"Synthetic account-free intake"}
        }),
        Topic::Jev => json!({
            "text":include_str!("../../../integrations/knowledge/jev.md"),
            "manifest":embedded_json(include_str!("../../../examples/jev-routing/process.json")),
            "sample_input":{"ticket":"A synthetic report of a software integration bug.","evidence_ref":"fixture:ticket:jev-routing-1"},
            "connection_example":embedded_json(include_str!("../../../examples/jev-routing/connection.json")),
            "live_execution_authorized":false
        }),
        Topic::Connections => {
            json!({"text":include_str!("../../../integrations/knowledge/connections.md")})
        }
        Topic::Adapters => json!({
            "text":include_str!("../../../integrations/knowledge/adapters.md"),
            "cli_executable":std::env::current_exe().ok(),
            "grant_created":false
        }),
        Topic::ClassifierLearning => json!({
            "text":include_str!("../../../integrations/knowledge/classifier-learning.md"),
            "classifier_manifest":embedded_json(include_str!("../../../examples/adaptive-classifier/classify/process.json")),
            "proposal_manifest":embedded_json(include_str!("../../../examples/adaptive-classifier/propose/process.json")),
            "evaluation_manifest":embedded_json(include_str!("../../../examples/adaptive-classifier/evaluate/process.json")),
            "sample_input":embedded_json(include_str!("../../../examples/adaptive-classifier/classify/input.json")),
            "connection_example":embedded_json(include_str!("../../../examples/adaptive-classifier/connection.json")),
            "learning_enabled_by_default":false,"live_execution_authorized":false
        }),
        Topic::RecipeImprovement => {
            json!({"text":include_str!("../../../integrations/knowledge/recipe-improvement.md"),
            "baseline_manifest":embedded_json(include_str!("../../../examples/recipe-improvement/baseline.json")),
            "candidate_manifest":embedded_json(include_str!("../../../examples/recipe-improvement/candidate.json")),
            "sample_input":{"answer":"Synthetic answer","evidence":"Synthetic source"},
            "mode":"controlled_end_to_end","provider_calls":0,"automatic_adoption":false})
        }
        Topic::Handoffs => {
            json!({"text":include_str!("../../../integrations/knowledge/handoffs.md")})
        }
        Topic::Schedules => {
            json!({"text":include_str!("../../../integrations/knowledge/schedules.md")})
        }
    };
    response["guide_revision"] =
        json!(apr_core::content_digest(&content).expect("bundled guide is canonical JSON"));
    response["content"] = content;
    response
}
