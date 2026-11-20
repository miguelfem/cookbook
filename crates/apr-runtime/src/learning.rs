//! Bounded view over retained, successful opt-in classifier runs. This is not a
//! separate queue or a subscription: ordinary run retention still applies.
use apr_core::{classifier::template_questions, content_digest};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::{Result, Runtime, RuntimeError, engine::bounded_detail};

impl Runtime {
    pub(crate) fn learning_cases(
        &self,
        assignment: &str,
        process_ref: &str,
        limit: Option<usize>,
        after: Option<&str>,
    ) -> Result<Value> {
        let process = self.catalog_process(assignment, process_ref)?;
        let manifest: Value = serde_json::from_slice(&process.canonical_bytes)?;
        template_questions(&manifest, &json!(process_ref))?;
        let limit = limit.unwrap_or(1);
        if !(1..=5).contains(&limit)
            || after.is_some_and(|s| s.len() > 160 || s.chars().any(char::is_control))
        {
            return Err(RuntimeError::InvalidInput(
                "invalid learning history page".into(),
            ));
        }
        let created: i64 = if let Some(id) = after {
            self.db
                .query_row(
                    "SELECT created_at FROM runs WHERE id=?1 AND assignment_id=?2 AND version=?3",
                    params![id, assignment, process_ref],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or(RuntimeError::Forbidden)?
        } else {
            -1
        };
        let mut query = self.db.prepare(
            "SELECT id,config,json_extract(output,'$.learning') FROM runs
            WHERE assignment_id=?1 AND version=?2 AND state='succeeded' AND fixture=0
              AND json_extract(config,'$.learning_enabled')=1
              AND json_extract(output,'$.learning.enabled')=1
              AND json_extract(output,'$.learning.other_count')>0
              AND (created_at>?3 OR (created_at=?3 AND id>?4))
            ORDER BY created_at,id LIMIT ?5",
        )?;
        let rows = query
            .query_map(
                params![
                    assignment,
                    process_ref,
                    created,
                    after.unwrap_or(""),
                    limit + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut truncated = rows.len() > limit;
        let mut batches = Vec::new();
        let mut next = None;
        let mut bytes = 0;
        for (id, config, collection) in rows.into_iter().take(limit) {
            let collection: Value = serde_json::from_str(&collection)?;
            let config: Value = serde_json::from_str(&config)?;
            let batch = json!({"run_id":id,"process_ref":process_ref,"config_digest":content_digest(&config)?,
                "model":config["model"],"collection":collection});
            let size = serde_json::to_vec(&batch)?.len();
            if bytes + size > 40 * 1024 {
                if batches.is_empty() {
                    return Err(RuntimeError::InvalidInput(
                        "learning batch exceeds inspection bound".into(),
                    ));
                }
                truncated = true;
                break;
            }
            bytes += size;
            next = Some(id);
            batches.push(batch);
        }
        let response = json!({"process_ref":process_ref,"batches":batches,"next_cursor":if truncated {next} else {None},
            "truncated":truncated,"scope":"Retained successful non-fixture runs of this exact version and assignment; no live subscription or independent learning queue."});
        bounded_detail(&response)?;
        Ok(response)
    }
}
