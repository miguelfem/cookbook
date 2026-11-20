use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_JSON_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("attachment is invalid or inactive")]
    Unauthorized,
    #[error("operation is not permitted in this attachment")]
    Forbidden,
    #[error("{0}")]
    Service(String),
    #[error("{0}")]
    Protocol(String),
    #[error("{message}")]
    Remote {
        code: String,
        message: String,
        diagnostic: Option<apr_core::AdmissionDiagnostic>,
    },
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    NotFound(String),
    #[error("request key already belongs to different inputs or process version")]
    RequestConflict,
    #[error("submission key already belongs to a different semantic result")]
    SubmissionConflict,
    #[error("handoff token, generation, or state is no longer valid")]
    StaleHandoff,
    #[error("{0}")]
    LimitExceeded(String),
    #[error("{0}")]
    Integrity(String),
    #[error("{0}")]
    State(String),
    #[error(transparent)]
    Core(#[from] apr_core::CoreError),
    #[error(transparent)]
    Artifact(#[from] crate::artifacts::ArtifactError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl RuntimeError {
    pub fn code(&self) -> &str {
        match self {
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::Service(_) => "SERVICE_UNAVAILABLE",
            Self::Protocol(_) => "UNSUPPORTED_VERSION",
            Self::Remote { code, .. } => code,
            Self::InvalidInput(_) | Self::Json(_) => "INVALID_INPUT",
            Self::NotFound(_) => "NOT_FOUND",
            Self::RequestConflict => "REQUEST_CONFLICT",
            Self::SubmissionConflict => "SUBMISSION_CONFLICT",
            Self::StaleHandoff => "STALE_HANDOFF",
            Self::LimitExceeded(_)
            | Self::Artifact(
                crate::artifacts::ArtifactError::QuotaExceeded { .. }
                | crate::artifacts::ArtifactError::TooLarge { .. },
            ) => "LIMIT_EXCEEDED",
            Self::Integrity(_) | Self::Artifact(_) => "ARTIFACT_INTEGRITY",
            Self::Core(_) => "INVALID_MANIFEST",
            Self::State(_) => "INVALID_STATE",
            Self::Database(_) | Self::Io(_) => "INTERNAL_ERROR",
        }
    }
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunEnvelope {
    pub protocol_version: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_step_id: Option<String>,
    pub process_version: String,
    #[serde(default)]
    pub config_digest: String,
    #[serde(default)]
    pub fixture: bool,
    pub state: String,
    pub revision: u64,
    pub summary: String,
    pub completed_steps: usize,
    pub steps: Vec<StepSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<AgentRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepSummary {
    pub id: String,
    pub state: String,
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_run_id: Option<String>,
    #[serde(default)]
    pub definition_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_key: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRequest {
    pub request_id: String,
    pub step_instance_id: String,
    pub generation: u32,
    pub capability: String,
    pub instructions: String,
    pub input: Value,
    pub result_schema: Value,
    pub continuation_token: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub protocol_version: String,
    pub action: String,
    pub run_id: String,
    pub request_id: String,
    pub step_instance_id: String,
    pub generation: u32,
    pub submission_key: String,
    pub continuation_token: String,
    pub output: Value,
    pub provenance: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubmissionReceipt {
    pub protocol_version: String,
    pub run_id: String,
    pub request_id: String,
    pub submission_key: String,
    pub output_digest: String,
    pub status: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub sequence: u64,
    pub kind: String,
    pub detail: Value,
}
