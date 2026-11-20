//! Pure TypeSafe Jev wire adapter. No credentials, network, retry or routing.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

#[derive(Debug, thiserror::Error)]
#[error("Jev payload does not match the bounded adapter contract")]
pub struct ContractError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    model: String,
    state: Value,
    questions: BTreeMap<String, Question>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Question {
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Value>,
    },
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
    Noul {
        instructions: Value,
        #[serde(default)]
        criteria: Option<BooleanCriteria>,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BooleanCriteria {
    #[serde(rename = "true")]
    yes: String,
    #[serde(rename = "false")]
    no: String,
}

pub struct PreparedRequest {
    body: Value,
    request: Request,
}
impl PreparedRequest {
    pub fn body(&self) -> &Value {
        &self.body
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Score {
        score: f64,
        legend: BTreeMap<String, Value>,
        probabilities: BTreeMap<String, f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Noul {
        noul: f64,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    model: String,
    answers: BTreeMap<String, Answer>,
    usage: Usage,
}

#[derive(Debug, Serialize)]
pub struct DecisionObservation {
    pub provider: &'static str,
    pub requested_model: String,
    pub reported_model: String,
    pub state_digest: String,
    pub request_digest: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

pub fn prepare(body: Value) -> Result<PreparedRequest, ContractError> {
    bounded(&body)?;
    let request: Request = serde_json::from_value(body.clone()).map_err(|_| ContractError)?;
    if !label(&request.model)
        || !structured(&request.state)
        || request.questions.is_empty()
        || request.questions.len() > 16
    {
        return Err(ContractError);
    }
    for (name, question) in &request.questions {
        if !label(name) {
            return Err(ContractError);
        }
        match question {
            Question::Choice {
                instructions,
                criteria,
            } => {
                if !structured(instructions)
                    || !(2..=32).contains(&criteria.len())
                    || criteria.iter().any(|(key, value)| {
                        !label(key) || !(value.is_null() || value.is_string() || value.is_object())
                    })
                {
                    return Err(ContractError);
                }
            }
            Question::Score {
                instructions,
                criteria,
            } => {
                if !structured(instructions)
                    || !(2..=32).contains(&criteria.len())
                    || criteria.iter().any(|value| !structured(value))
                {
                    return Err(ContractError);
                }
            }
            Question::Noul {
                instructions,
                criteria,
            } => {
                if !structured(instructions)
                    || criteria
                        .as_ref()
                        .is_some_and(|c| c.yes.is_empty() || c.no.is_empty())
                {
                    return Err(ContractError);
                }
            }
        }
    }
    Ok(PreparedRequest { body, request })
}

pub fn observe(
    request: &PreparedRequest,
    response: Value,
) -> Result<DecisionObservation, ContractError> {
    bounded(&response)?;
    let response: Response = serde_json::from_value(response).map_err(|_| ContractError)?;
    if !label(&response.model) || response.answers.keys().ne(request.request.questions.keys()) {
        return Err(ContractError);
    }
    for (name, question) in &request.request.questions {
        match (question, response.answers.get(name).ok_or(ContractError)?) {
            (Question::Noul { .. }, Answer::Noul { noul }) if probability(*noul) => {}
            (
                Question::Choice { criteria, .. },
                Answer::Choice {
                    choice,
                    probabilities,
                    confidence,
                },
            ) => {
                distribution(probabilities)?;
                check_confidence(*confidence)?;
                let selected = *probabilities.get(choice).ok_or(ContractError)?;
                if probabilities.keys().ne(criteria.keys())
                    || probabilities.values().any(|value| *value > selected + 1e-9)
                {
                    return Err(ContractError);
                }
            }
            (
                Question::Score { criteria, .. },
                Answer::Score {
                    score,
                    legend,
                    probabilities,
                    confidence,
                },
            ) => {
                distribution(probabilities)?;
                check_confidence(*confidence)?;
                if !score.is_finite()
                    || *score < 0.0
                    || *score > (criteria.len() - 1) as f64
                    || probabilities.len() != criteria.len()
                    || legend.len() != criteria.len()
                {
                    return Err(ContractError);
                }
                let mut weighted = 0.0;
                for (index, level) in criteria.iter().enumerate() {
                    let key = index.to_string();
                    if legend.get(&key) != Some(level) {
                        return Err(ContractError);
                    }
                    weighted += index as f64 * probabilities.get(&key).ok_or(ContractError)?;
                }
                // Provider examples round probabilities and scores to two decimal places.
                if (weighted - score).abs() > 0.02 {
                    return Err(ContractError);
                }
            }
            _ => return Err(ContractError),
        }
    }
    Ok(DecisionObservation {
        provider: "typesafe",
        requested_model: request.request.model.clone(),
        reported_model: response.model,
        state_digest: apr_core::content_digest(&request.request.state)
            .map_err(|_| ContractError)?,
        request_digest: apr_core::content_digest(&request.body).map_err(|_| ContractError)?,
        answers: response.answers,
        usage: response.usage,
    })
}
fn bounded(value: &Value) -> Result<(), ContractError> {
    if apr_core::canonical_bytes(value)
        .map_err(|_| ContractError)?
        .len()
        > 64 * 1024
    {
        return Err(ContractError);
    }
    Ok(())
}
fn label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
fn structured(value: &Value) -> bool {
    value.is_object() || value.is_array() || value.as_str().is_some_and(|s| !s.is_empty())
}
fn probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
fn check_confidence(value: Option<f64>) -> Result<(), ContractError> {
    if value.is_some_and(|value| !probability(value)) {
        return Err(ContractError);
    }
    Ok(())
}
fn distribution(probabilities: &BTreeMap<String, f64>) -> Result<(), ContractError> {
    if probabilities.values().any(|p| !probability(*p))
        || (probabilities.values().sum::<f64>() - 1.0).abs() > 0.005
    {
        return Err(ContractError);
    }
    Ok(())
}
