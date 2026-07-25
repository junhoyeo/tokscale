//! Hermes Agent session parser
//!
//! Parses aggregated session rows from Hermes Agent's SQLite state database:
//! - `~/.hermes/state.db`
//! - `$HERMES_HOME/state.db`

use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use rusqlite::Connection;
use std::path::Path;
use tracing::warn;

const HERMES_AGENT_NAME: &str = "Hermes Agent";

fn timestamp_secs_to_ms(timestamp: f64) -> i64 {
    if timestamp > 1e12 {
        timestamp as i64
    } else {
        (timestamp * 1000.0) as i64
    }
}

fn resolved_provider(billing_provider: Option<String>, model_id: &str) -> String {
    billing_provider
        .filter(|provider| !provider.trim().is_empty())
        .and_then(|provider| provider_identity::canonical_provider(provider.trim()))
        .or_else(|| provider_identity::inferred_provider_from_model(model_id).map(str::to_string))
        .unwrap_or_else(|| "hermes".to_string())
}

pub fn parse_hermes_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Hermes state database"
            );
            return Vec::new();
        }
    };

    let query = r#"
        SELECT
            smu.session_id,
            smu.model,
            MAX(smu.billing_provider) AS billing_provider,
            s.started_at,
            CASE WHEN smu.model = s.model THEN COALESCE(s.message_count, 0) ELSE 0 END AS message_count,
            SUM(smu.input_tokens)        AS input_tokens,
            SUM(smu.output_tokens)       AS output_tokens,
            SUM(smu.cache_read_tokens)   AS cache_read_tokens,
            SUM(smu.cache_write_tokens)  AS cache_write_tokens,
            SUM(smu.reasoning_tokens)    AS reasoning_tokens,
            SUM(smu.estimated_cost_usd)  AS estimated_cost_usd,
            SUM(smu.actual_cost_usd)     AS actual_cost_usd
        FROM session_model_usage smu
        JOIN sessions s ON s.id = smu.session_id
        WHERE smu.model IS NOT NULL
          AND TRIM(smu.model) != ''
        GROUP BY smu.session_id, smu.model, s.started_at, s.message_count
        HAVING SUM(smu.input_tokens) > 0
            OR SUM(smu.output_tokens) > 0
            OR SUM(smu.cache_read_tokens) > 0
            OR SUM(smu.cache_write_tokens) > 0
            OR SUM(smu.reasoning_tokens) > 0
            OR MAX(smu.actual_cost_usd) > 0
            OR MAX(smu.estimated_cost_usd) > 0
    "#;

    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Hermes session query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, Option<i32>>(4)?.unwrap_or(0),
            row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            row.get::<_, Option<i64>>(6)?.unwrap_or(0),
            row.get::<_, Option<i64>>(7)?.unwrap_or(0),
            row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            row.get::<_, Option<i64>>(9)?.unwrap_or(0),
            row.get::<_, Option<f64>>(10)?,
            row.get::<_, Option<f64>>(11)?,
        ))
    }) {
        Ok(r) => r,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to execute Hermes session query"
            );
            return Vec::new();
        }
    };

    rows.filter_map(|row| match row {
        Ok(row) => Some(row),
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to decode Hermes session row"
            );
            None
        }
    })
    .map(
        |(
            session_id,
            model_id,
            billing_provider,
            started_at,
            message_count,
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
            estimated_cost,
            actual_cost,
        )| {
            let provider = resolved_provider(billing_provider, &model_id);
            let dedup_key = format!("hermes:{session_id}:{model_id}");
            let mut msg = UnifiedMessage::new_with_agent(
                "hermes",
                model_id,
                provider,
                session_id.clone(),
                timestamp_secs_to_ms(started_at),
                TokenBreakdown {
                    input: input.max(0),
                    output: output.max(0),
                    cache_read: cache_read.max(0),
                    cache_write: cache_write.max(0),
                    reasoning: reasoning.max(0),
                },
                actual_cost
                    .filter(|c| *c > 0.0)
                    .or(estimated_cost.filter(|c| *c > 0.0))
                    .unwrap_or(0.0)
                    .max(0.0),
                Some(HERMES_AGENT_NAME.to_string()),
            );
            msg.message_count = message_count.max(0);
            msg.dedup_key = Some(dedup_key);
            msg
        },
    )
    .collect()
}
