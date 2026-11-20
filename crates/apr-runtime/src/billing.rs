//! Durable billing ledger. Currency quantities are decimal integer nanounits,
//! never floats. Reservations and effect intents commit in the same transaction.
use crate::store::{StoreLock, now_ms};
use crate::{Result, Runtime, RuntimeError};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) const MIGRATION_V12: &str = r#"
CREATE TABLE billing_calls (
 intent_id TEXT PRIMARY KEY, assignment_id TEXT NOT NULL, run_id TEXT NOT NULL,
 root_run_id TEXT NOT NULL, connection_name TEXT NOT NULL, binding_revision INTEGER NOT NULL,
 occurred_at INTEGER NOT NULL, currency TEXT, reserved_nanos TEXT NOT NULL DEFAULT '0', reserved_known INTEGER NOT NULL DEFAULT 0,
 estimated_nanos TEXT, reported_nanos TEXT, reconciled_nanos TEXT, status TEXT NOT NULL,
 coverage TEXT NOT NULL DEFAULT 'unknown', provenance TEXT NOT NULL DEFAULT 'unobserved'
);
CREATE INDEX billing_scope_time ON billing_calls(assignment_id,occurred_at,intent_id);
CREATE TABLE billing_budgets (assignment_id TEXT PRIMARY KEY, policy TEXT NOT NULL, revision INTEGER NOT NULL);
CREATE TABLE billing_invoices (assignment_id TEXT NOT NULL, invoice_id TEXT NOT NULL, digest TEXT NOT NULL, payload TEXT NOT NULL, imported_at INTEGER NOT NULL, PRIMARY KEY(assignment_id,invoice_id));
CREATE TABLE billing_invoice_lines (assignment_id TEXT NOT NULL,invoice_id TEXT NOT NULL,line_id TEXT NOT NULL,intent_id TEXT NOT NULL UNIQUE,amount_nanos TEXT NOT NULL,PRIMARY KEY(assignment_id,invoice_id,line_id));
INSERT INTO billing_calls(intent_id,assignment_id,run_id,root_run_id,connection_name,binding_revision,occurred_at,status,currency,estimated_nanos,reported_nanos,coverage,provenance)
SELECT e.id,r.assignment_id,e.run_id,COALESCE(r.root_run_id,r.id),e.binding_name,e.binding_revision,e.created_at,e.status,
 json_extract(v.detail,'$.observation.currency'),json_extract(v.detail,'$.observation.estimated_nanos'),json_extract(v.detail,'$.observation.reported_nanos'),COALESCE(json_extract(v.detail,'$.observation.estimate_status'),'unknown'),COALESCE(json_extract(v.detail,'$.provenance'),'unobserved')
FROM effect_intents e JOIN runs r ON r.id=e.run_id LEFT JOIN events v ON v.sequence=(SELECT MAX(ev.sequence) FROM events ev WHERE ev.run_id=e.run_id AND ev.kind='usage.observed' AND json_extract(ev.detail,'$.intent_id')=e.id)
WHERE e.kind='connection' AND r.assignment_id IS NOT NULL;
"#;
fn invalid(message: &str) -> RuntimeError {
    RuntimeError::InvalidInput(message.into())
}
fn amount(s: &str) -> Result<u128> {
    if s.is_empty() || s.len() > 30 || !s.bytes().all(|c| c.is_ascii_digit()) {
        return Err(invalid(
            "money must be nonnegative decimal integer nanounits",
        ));
    }
    s.parse()
        .map_err(|_| invalid("monetary amount is too large"))
}
fn currency(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|c| c.is_ascii_uppercase())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetPolicy {
    pub currency: String,
    pub starts_at_ms: i64,
    pub ends_at_ms: i64,
    pub limit_nanos: String,
    /// Reviewed maximum liability per attempt. These are not provider guarantees.
    pub per_connection_max_nanos: BTreeMap<String, String>,
}
impl BudgetPolicy {
    pub fn validate(&self) -> Result<()> {
        if !currency(&self.currency)
            || self.starts_at_ms < 0
            || self.ends_at_ms <= self.starts_at_ms
            || self.per_connection_max_nanos.len() > 128
        {
            return Err(invalid("invalid budget period or currency"));
        }
        amount(&self.limit_nanos)?;
        for (name, value) in &self.per_connection_max_nanos {
            if name.is_empty() || name.len() > 128 {
                return Err(invalid("invalid budget connection"));
            }
            amount(value)?;
        }
        Ok(())
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invoice {
    pub invoice_id: String,
    pub account_label: String,
    pub connection_name: String,
    pub currency: String,
    pub starts_at_ms: i64,
    pub ends_at_ms: i64,
    pub lines: Vec<InvoiceLine>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceLine {
    pub line_id: String,
    pub intent_id: String,
    pub amount_nanos: String,
}

impl Runtime {
    /// Owner-only control; never exposed by apr_manage. A policy cannot be
    /// disabled by changing a recipe. Outside its approved period dispatch stops.
    pub fn set_billing_budget(&mut self, assignment: &str, policy: BudgetPolicy) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_assignment_active(assignment)?;
        policy.validate()?;
        self.db.execute("INSERT INTO billing_budgets(assignment_id,policy,revision) VALUES(?1,?2,1) ON CONFLICT(assignment_id) DO UPDATE SET policy=excluded.policy,revision=revision+1",params![assignment,serde_json::to_string(&policy)?])?;
        Ok(
            json!({"saved":true,"policy":policy,"enforcement":"broker_dispatch_reservations","provider_bill_guarantee":false}),
        )
    }
    pub fn billing_budget(&self, assignment: &str) -> Result<Value> {
        let saved: Option<String> = self
            .db
            .query_row(
                "SELECT policy FROM billing_budgets WHERE assignment_id=?1",
                [assignment],
                |r| r.get(0),
            )
            .optional()?;
        Ok(
            json!({"policy":saved.map(|s|serde_json::from_str::<Value>(&s)).transpose()?,"enforcement":"broker_dispatch_reservations","provider_bill_guarantee":false}),
        )
    }
    /// Whole-period totals, not page subtotals. Ledger survives run retention.
    /// One read transaction pins all rows to a coherent SQLite snapshot. Service
    /// readers already own that snapshot; direct owner calls establish their own.
    pub fn billing_period(&self, assignment: &str, start: i64, end: i64) -> Result<Value> {
        if start < 0 || end <= start {
            return Err(invalid("invalid billing window"));
        }
        let tx = if self.db.is_autocommit() {
            Some(self.db.unchecked_transaction()?)
        } else {
            None
        };
        let mut query=self.db.prepare("SELECT connection_name,currency,estimated_nanos,reported_nanos,reconciled_nanos,reserved_nanos,status,coverage FROM billing_calls WHERE assignment_id=?1 AND occurred_at>=?2 AND occurred_at<?3 ORDER BY occurred_at,intent_id")?;
        #[derive(Default)]
        struct Group {
            calls: u64,
            unknown: u64,
            estimated: u128,
            reported: u128,
            reconciled: u128,
            reserved: u128,
            ec: u64,
            pc: u64,
            ic: u64,
            not_sent: u64,
        }
        let mut groups = BTreeMap::<(String, String), Group>::new();
        let mut rows = 0u64;
        for row in query.query_map(params![assignment, start, end], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })? {
            let (name, cur, est, rep, inv, res, status, coverage) = row?;
            rows += 1;
            if rows > 1_000_000 {
                return Err(RuntimeError::LimitExceeded(
                    "billing window exceeds one million calls; narrow the period".into(),
                ));
            }
            let key = (name, cur.unwrap_or_else(|| "UNKNOWN".into()));
            if !groups.contains_key(&key) && groups.len() >= 128 {
                return Err(RuntimeError::LimitExceeded(
                    "too many billing groups; use a narrower period".into(),
                ));
            }
            let g = groups.entry(key).or_default();
            g.calls += 1;
            if matches!(status.as_str(), "not_sent" | "reconciled_no_effect") {
                g.not_sent += 1;
                continue;
            }
            if inv.is_none() && rep.is_none() && coverage != "estimated" {
                g.unknown += 1;
            }
            for (value, total, count) in [
                (est, &mut g.estimated, &mut g.ec),
                (rep, &mut g.reported, &mut g.pc),
                (inv, &mut g.reconciled, &mut g.ic),
            ] {
                if let Some(v) = value {
                    *total = total
                        .checked_add(amount(&v)?)
                        .ok_or_else(|| invalid("billing sum overflow"))?;
                    *count += 1;
                }
            }
            g.reserved = g
                .reserved
                .checked_add(amount(&res)?)
                .ok_or_else(|| invalid("billing reservation overflow"))?;
        }
        let groups:Vec<Value>=groups.into_iter().map(|((connection,currency),g)|json!({"connection":connection,"currency":currency,"calls":g.calls,"not_sent":g.not_sent,"unknown_or_partial":g.unknown,"estimated_nanos":(g.ec>0).then(||g.estimated.to_string()),"provider_reported_nanos":(g.pc>0).then(||g.reported.to_string()),"invoice_matched_nanos":(g.ic>0).then(||g.reconciled.to_string()),"reserved_nanos":g.reserved.to_string(),"estimated_calls":g.ec,"provider_reported_calls":g.pc,"invoice_matched_calls":g.ic})).collect();
        drop(query);
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(
            json!({"view":"billing","scope":"whole_period","assignment_id":assignment,"starts_at_ms":start,"ends_at_ms":end,"calls":rows,"groups":groups,"amount_columns_are_not_additive":true,"invoice_provenance":"owner_import","retained_billing_ledger":true}),
        )
    }
    /// Explicit owner import, exact connection/currency/time and intent matches.
    /// An invoice does not create an additional call or increase available budget.
    pub fn import_invoice(&mut self, assignment: &str, invoice: Invoice) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_assignment_active(assignment)?;
        if invoice.invoice_id.is_empty()
            || invoice.invoice_id.len() > 128
            || invoice.account_label.is_empty()
            || invoice.account_label.len() > 128
            || invoice.lines.is_empty()
            || invoice.lines.len() > 200
            || !currency(&invoice.currency)
            || invoice.starts_at_ms < 0
            || invoice.ends_at_ms <= invoice.starts_at_ms
        {
            return Err(invalid("invalid invoice scope"));
        }
        let payload = serde_json::to_value(&invoice)?;
        let digest = apr_core::content_digest(&payload)?;
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT digest FROM billing_invoices WHERE assignment_id=?1 AND invoice_id=?2",
                params![assignment, invoice.invoice_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(old) = existing {
            if old != digest {
                return Err(invalid("invoice identifier already has different content"));
            }
            return Ok(json!({"imported":true,"replayed":true,"invoice_id":invoice.invoice_id}));
        }
        let tx = self.db.transaction()?;
        let mut ids = std::collections::BTreeSet::new();
        let mut total = 0u128;
        for line in &invoice.lines {
            if line.line_id.is_empty() || line.line_id.len() > 128 || !ids.insert(&line.line_id) {
                return Err(invalid("duplicate or invalid invoice line"));
            }
            let n = amount(&line.amount_nanos)?;
            total = total
                .checked_add(n)
                .ok_or_else(|| invalid("invoice total overflow"))?;
            let found:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM billing_calls WHERE intent_id=?1 AND assignment_id=?2 AND connection_name=?3 AND (currency=?4 OR currency IS NULL) AND occurred_at>=?5 AND occurred_at<?6 AND status NOT IN ('not_sent','reconciled_no_effect'))",params![line.intent_id,assignment,invoice.connection_name,invoice.currency,invoice.starts_at_ms,invoice.ends_at_ms],|r|r.get(0))?;
            if !found {
                return Err(invalid(
                    "invoice line does not match connection, currency, period and intent",
                ));
            }
            tx.execute("INSERT INTO billing_invoice_lines(assignment_id,invoice_id,line_id,intent_id,amount_nanos) VALUES(?1,?2,?3,?4,?5)",params![assignment,invoice.invoice_id,line.line_id,line.intent_id,line.amount_nanos])?;
            tx.execute(
                "UPDATE billing_calls SET reconciled_nanos=?2,currency=COALESCE(currency,?3) WHERE intent_id=?1",
                params![line.intent_id, line.amount_nanos, invoice.currency],
            )?;
        }
        tx.execute("INSERT INTO billing_invoices(assignment_id,invoice_id,digest,payload,imported_at) VALUES(?1,?2,?3,?4,?5)",params![assignment,invoice.invoice_id,digest,serde_json::to_string(&payload)?,now_ms()])?;
        tx.commit()?;
        Ok(
            json!({"imported":true,"invoice_id":invoice.invoice_id,"matched_lines":invoice.lines.len(),"amount_nanos":total.to_string(),"currency":invoice.currency,"provenance":"owner_import_not_provider_authenticated"}),
        )
    }
}
/// Called before inserting an intent, inside its transaction. No paid request
/// can occur unless this succeeds. Unknown historical exposure blocks strict mode.
pub(crate) fn reserve(
    tx: &Transaction<'_>,
    run: &str,
    intent: &str,
    name: &str,
    revision: u64,
    binding: &Value,
) -> Result<()> {
    let (assignment, root): (Option<String>, String) = tx.query_row(
        "SELECT assignment_id,COALESCE(root_run_id,id) FROM runs WHERE id=?1",
        [run],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let Some(assignment) = assignment else {
        return Ok(());
    };
    let cur = binding
        .pointer("/metering/currency")
        .and_then(Value::as_str);
    let policy: Option<String> = tx
        .query_row(
            "SELECT policy FROM billing_budgets WHERE assignment_id=?1",
            [&assignment],
            |r| r.get(0),
        )
        .optional()?;
    let reserved_known = policy.is_some();
    let mut reservation = 0u128;
    if let Some(policy) = policy {
        let p: BudgetPolicy = serde_json::from_str(&policy)?;
        p.validate()?;
        if now_ms() < p.starts_at_ms || now_ms() >= p.ends_at_ms || cur != Some(p.currency.as_str())
        {
            return Err(RuntimeError::State(
                "budget period/currency requires owner review".into(),
            ));
        }
        reservation = amount(p.per_connection_max_nanos.get(name).ok_or_else(|| {
            RuntimeError::State("connection needs an approved maximum charge reservation".into())
        })?)?;
        let mut q=tx.prepare("SELECT reserved_nanos,estimated_nanos,reported_nanos,reconciled_nanos,status,currency,coverage,reserved_known FROM billing_calls WHERE assignment_id=?1 AND occurred_at>=?2 AND occurred_at<?3")?;
        let mut used = 0u128;
        for row in q.query_map(params![assignment, p.starts_at_ms, p.ends_at_ms], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, bool>(7)?,
            ))
        })? {
            let (reserved, est, rep, inv, status, currency, coverage, known_reservation) = row?;
            if matches!(status.as_str(), "not_sent" | "reconciled_no_effect") {
                continue;
            }
            if currency.as_deref() != Some(p.currency.as_str()) {
                return Err(RuntimeError::State(
                    "unpriced or different-currency exposure blocks this budget".into(),
                ));
            }
            let n = amount(&reserved)?;
            let observed = [est.as_deref(), rep.as_deref(), inv.as_deref()]
                .into_iter()
                .flatten()
                .map(amount)
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .max()
                .unwrap_or(0);
            if !known_reservation && inv.is_none() && rep.is_none() && coverage != "estimated" {
                return Err(RuntimeError::State(
                    "unknown prior charges require reconciliation before spending".into(),
                ));
            }
            used = used
                .checked_add(n.max(observed))
                .ok_or_else(|| invalid("budget overflow"))?;
        }
        if used
            .checked_add(reservation)
            .is_none_or(|v| v > amount(&p.limit_nanos).unwrap_or(0))
        {
            return Err(RuntimeError::State(
                "billing reservation limit reached".into(),
            ));
        }
    }
    tx.execute("INSERT INTO billing_calls(intent_id,assignment_id,run_id,root_run_id,connection_name,binding_revision,occurred_at,currency,reserved_nanos,reserved_known,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'dispatched')",params![intent,assignment,run,root,name,revision,now_ms(),cur,reservation.to_string(),reserved_known])?;
    Ok(())
}
pub(crate) fn settle(
    tx: &Transaction<'_>,
    intent: &str,
    status: &str,
    observation: Option<&Value>,
    provenance: &str,
) -> Result<()> {
    tx.execute(
        "UPDATE billing_calls SET status=?2 WHERE intent_id=?1",
        params![intent, status],
    )?;
    if let Some(o) = observation {
        tx.execute("UPDATE billing_calls SET currency=?2,estimated_nanos=?3,reported_nanos=?4,coverage=?5,provenance=?6 WHERE intent_id=?1",params![intent,o["currency"].as_str(),o["estimated_nanos"].as_str(),o["reported_nanos"].as_str(),o["estimate_status"].as_str().unwrap_or("unknown"),provenance])?;
    }
    Ok(())
}
