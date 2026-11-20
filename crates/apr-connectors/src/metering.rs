//! Explicit, content-free usage extraction and fixed-point cost estimates.
//!
//! Provider responses are NOT scanned heuristically. An owner-approved mapping
//! names numeric JSON pointers and their arithmetic. This handles tokens, images,
//! milliseconds, requests, credits, etc. without treating every API as an LLM.
//! Prices describe only the configured meters, not an invoice or a spend cap.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

const MAX_METERS: usize = 16;
const MAX_TERMS: usize = 8;
const MAX_QUANTITY: u64 = 1_000_000_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeteringConfig {
    /// Public, owner-supplied provider identifier. Never extracted from output.
    pub provider: String,
    /// Author-time evidence for the user-reviewed cost contract, never payload bodies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<CostAnalysis>,
    /// Version of the mapping and price card, frozen in the connection binding.
    pub revision: String,
    pub currency: String,
    /// Exact response identity check, useful when pricing a particular model.
    #[serde(default)]
    pub identity: Option<IdentityCheck>,
    pub meters: Vec<Meter>,
    /// Optional provider-reported monetary subtotal, NOT invoice reconciliation.
    #[serde(default)]
    pub reported_cost_pointer: Option<String>,
}

/// The agent analyzes only user-selected payload contracts at recipe creation.
/// Runtime execution uses the reviewed arithmetic, not another model invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostAnalysis {
    pub recipe: String,
    pub request_schema_digest: String,
    pub response_schema_digest: String,
    pub pricing_sources: Vec<String>,
    pub checked_at: String,
    /// Explicit unpriced fees, missing observations, or provider-specific limits.
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityCheck {
    pub pointer: String,
    pub equals: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub name: String,
    pub unit: String,
    /// All terms must be present nonnegative integers. Absent is not zero.
    pub add: Vec<String>,
    #[serde(default)]
    pub subtract: Vec<String>,
    #[serde(default)]
    pub rate: Option<Rate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rate {
    /// Decimal currency amount, e.g. "2.50" per 1,000,000 tokens.
    /// String representation avoids binary floating point and currency rounding.
    pub amount: String,
    pub per_units: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MeterObservation {
    pub name: String,
    pub unit: String,
    pub quantity: Option<u64>,
    /// Request quantities are estimates of requested work, not delivered usage.
    #[serde(default)]
    pub basis: String,
    pub issue: Option<String>,
    /// Decimal integer nanocurrency units; one currency unit = 10^9 nanos.
    pub estimated_nanos: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CostObservation {
    pub schema_version: String,
    pub provider: String,
    pub pricing_revision: String,
    pub currency: String,
    pub identity_matches: bool,
    pub meters: Vec<MeterObservation>,
    pub estimated_nanos: Option<String>,
    /// Coverage of the configured meters only, never the complete provider bill.
    pub estimate_status: String,
    pub reported_nanos: Option<String>,
    pub reported_issue: Option<String>,
}

impl MeteringConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !identifier(&self.provider)
            || !identifier(&self.revision)
            || self.currency.len() != 3
            || !self.currency.bytes().all(|b| b.is_ascii_uppercase())
            || self.meters.len() > MAX_METERS
            || (self.meters.is_empty() && self.reported_cost_pointer.is_none())
        {
            return Err("invalid metering identity, currency or meter count");
        }
        if let Some(analysis) = &self.analysis
            && (analysis.recipe.is_empty()
                || analysis.recipe.len() > 128
                || !analysis.request_schema_digest.starts_with("sha256:")
                || analysis.request_schema_digest.len() != 71
                || !analysis.request_schema_digest[7..]
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
                || !analysis.response_schema_digest.starts_with("sha256:")
                || analysis.response_schema_digest.len() != 71
                || !analysis.response_schema_digest[7..]
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
                || analysis.pricing_sources.len() > 8
                || analysis.limitations.len() > 16
                || analysis.checked_at.is_empty()
                || analysis.checked_at.len() > 64
                || analysis.pricing_sources.iter().any(|s| {
                    !s.starts_with("https://")
                        || s.len() > 2048
                        || s.contains(['@', '#'])
                        || s.chars().any(char::is_control)
                })
                || analysis
                    .limitations
                    .iter()
                    .any(|s| s.len() > 512 || s.chars().any(char::is_control)))
        {
            return Err("invalid author-time cost analysis");
        }
        if let Some(identity) = &self.identity
            && (!pointer(&identity.pointer)
                || identity.equals.is_empty()
                || identity.equals.len() > 128
                || identity.equals.chars().any(char::is_control))
        {
            return Err("invalid metering identity check");
        }
        if let Some(path) = &self.reported_cost_pointer
            && (!pointer(path) || path.starts_with("request:") || path.starts_with("constant:"))
        {
            return Err("invalid reported cost pointer");
        }
        let mut names = BTreeSet::new();
        for meter in &self.meters {
            if !identifier(&meter.name)
                || !identifier(&meter.unit)
                || !names.insert(&meter.name)
                || meter.add.is_empty()
                || meter.add.len() + meter.subtract.len() > MAX_TERMS
                || !meter.add.iter().chain(&meter.subtract).all(|s| pointer(s))
            {
                return Err("invalid or duplicate meter");
            }
            let mut paths = BTreeSet::new();
            if !meter
                .add
                .iter()
                .chain(&meter.subtract)
                .all(|p| paths.insert(p.strip_prefix("response:").unwrap_or(p)))
            {
                return Err("duplicate arithmetic term");
            }
            if let Some(rate) = &meter.rate
                && (rate.per_units == 0
                    || rate.per_units > MAX_QUANTITY
                    || decimal_nanos(&rate.amount).is_none())
            {
                return Err("invalid fixed-point rate");
            }
        }
        Ok(())
    }

    /// No I/O, credentials, arbitrary expressions, model calls or raw body copies.
    /// Only allowlisted numeric values and owner-provided labels are retained.
    pub fn observe(&self, response: &Value) -> CostObservation {
        self.observe_exchange(&Value::Null, response)
    }

    /// request:/path and response:/path name explicit numeric payload fields.
    /// Legacy /path terms still select the response. constant:N is an explicit
    /// owner-reviewed per-attempt unit, never an inferred provider charge.
    pub fn observe_exchange(&self, request: &Value, response: &Value) -> CostObservation {
        let valid = self.validate().is_ok();
        let identity_matches = valid
            && self.identity.as_ref().is_none_or(|identity| {
                select(&identity.pointer, request, response).and_then(Value::as_str)
                    == Some(identity.equals.as_str())
            });
        let mut total = 0_u128;
        let mut priced = 0_usize;
        let mut meters = Vec::new();
        for meter in self.meters.iter().take(MAX_METERS) {
            let observed = if valid {
                quantity(meter, request, response)
            } else {
                Err("invalid_configuration")
            };
            let (value, mut issue) = match observed {
                Ok(value) => (Some(value), None),
                Err(code) => (None, Some(code.to_string())),
            };
            let estimated = match (identity_matches, value, &meter.rate) {
                (true, Some(value), Some(rate)) => {
                    decimal_nanos(&rate.amount).map(|price| {
                        // Validated quantity/rate limits keep this product within u128.
                        (u128::from(value) * price).div_ceil(u128::from(rate.per_units))
                    })
                }
                _ => None,
            };
            if let Some(amount) = estimated {
                total += amount;
                priced += 1;
            } else if issue.is_none() {
                issue = Some(
                    if !identity_matches {
                        "identity_mismatch"
                    } else {
                        "missing_rate"
                    }
                    .into(),
                );
            }
            meters.push(MeterObservation {
                name: meter.name.clone(),
                unit: meter.unit.clone(),
                quantity: value,
                basis: {
                    let sources: BTreeSet<_> = meter
                        .add
                        .iter()
                        .chain(&meter.subtract)
                        .map(|p| {
                            if p.starts_with("request:") {
                                "request"
                            } else if p.starts_with("constant:") {
                                "constant"
                            } else {
                                "response"
                            }
                        })
                        .collect();
                    if sources.len() == 1 {
                        sources.iter().next().unwrap().to_string()
                    } else {
                        "mixed".into()
                    }
                },
                issue,
                estimated_nanos: estimated.map(|n| n.to_string()),
            });
        }
        let reported = if valid {
            self.reported_cost_pointer.as_ref().map(|path| {
                let value = select(path, request, response).ok_or("missing_reported_cost")?;
                let text = match value {
                    Value::String(text) => text.clone(),
                    Value::Number(number) => number.to_string(),
                    _ => return Err("invalid_reported_cost"),
                };
                decimal_nanos(&text).ok_or("invalid_reported_cost")
            })
        } else {
            None
        };
        CostObservation {
            schema_version: "1".into(),
            provider: self.provider.clone(),
            pricing_revision: self.revision.clone(),
            currency: self.currency.clone(),
            identity_matches,
            meters,
            estimated_nanos: (priced > 0).then(|| total.to_string()),
            estimate_status: if priced == 0 {
                "unknown"
            } else if priced == self.meters.len() {
                "estimated"
            } else {
                "partial"
            }
            .into(),
            reported_nanos: reported
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(ToString::to_string),
            reported_issue: reported.and_then(Result::err).map(str::to_string),
        }
    }
}

fn quantity(meter: &Meter, request: &Value, response: &Value) -> Result<u64, &'static str> {
    let sum = |paths: &[String]| -> Result<u64, &'static str> {
        let mut total = 0_u64;
        for path in paths {
            let number = if let Some(value) = path.strip_prefix("constant:") {
                value.parse::<u64>().map_err(|_| "invalid_usage")?
            } else {
                select(path, request, response)
                    .ok_or("missing_usage")?
                    .as_u64()
                    .ok_or("invalid_usage")?
            };
            if number > MAX_QUANTITY {
                return Err("usage_out_of_range");
            }
            total = total.checked_add(number).ok_or("usage_out_of_range")?;
        }
        Ok(total)
    };
    sum(&meter.add)?
        .checked_sub(sum(&meter.subtract)?)
        .ok_or("inconsistent_usage")
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}

fn select<'a>(path: &str, request: &'a Value, response: &'a Value) -> Option<&'a Value> {
    if let Some(path) = path.strip_prefix("request:") {
        request.pointer(path)
    } else {
        response.pointer(path.strip_prefix("response:").unwrap_or(path))
    }
}

fn pointer(value: &str) -> bool {
    if let Some(value) = value.strip_prefix("constant:") {
        return !value.is_empty()
            && value.len() <= 13
            && value.bytes().all(|c| c.is_ascii_digit())
            && value.parse::<u64>().is_ok_and(|n| n <= MAX_QUANTITY);
    }
    let value = value
        .strip_prefix("request:")
        .or_else(|| value.strip_prefix("response:"))
        .unwrap_or(value);
    if !value.starts_with('/') || value.len() > 256 || value.chars().any(char::is_control) {
        return false;
    }
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

/// Strict nonnegative fixed-point decimal. Never accepts NaN, exponents,
/// negative charges, more than 9 decimals, or silently rounds supplied prices.
fn decimal_nanos(value: &str) -> Option<u128> {
    if value.len() > 30 {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || whole.len() > 12
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 9
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (value.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let whole: u128 = whole.parse().ok()?;
    let fraction_value: u128 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().ok()?
    };
    Some(whole * 1_000_000_000 + fraction_value * 10_u128.pow(9 - fraction.len() as u32))
}
