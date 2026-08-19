//! GitHub Copilot Desktop SQLite parser.
//!
//! The macOS desktop app stores aggregate token totals in `~/.copilot/data.db`
//! and per-session event metadata in `~/.copilot/session-state/{session_id}`.

use super::utils::{lossy_lines, sqlite_for_each_row};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use chrono::{DateTime, NaiveDateTime};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::path::Path;
use tracing::warn;

#[derive(Debug)]
struct CopilotDesktopSessionRow {
    id: String,
    model: Option<String>,
    total_input_tokens: i64,
    total_output_tokens: i64,
    total_cached_tokens: i64,
    total_reasoning_tokens: i64,
    created_at: Option<String>,
}

#[derive(Debug, Default)]
struct SessionStateMetadata {
    model: Option<String>,
    cwd: Option<String>,
    shutdowns: Vec<ShutdownUsage>,
    /// Usage the shutdown snapshots account for. This is normally the sum of
    /// `shutdowns`, but not when a snapshot was swallowed as an unknown
    /// baseline: that usage is accounted for without being emitted, and the
    /// row residual has to know it so it is not re-emitted on `created_at`.
    consumed: UsageBuckets,
}

/// One model's usage from a single `session.shutdown` record.
///
/// These carry their own timestamp, which is the only per-run timing the
/// desktop app exposes: the `sessions` row has a lifetime total and an
/// immutable `created_at`.
///
/// As read off disk the numbers are **cumulative**, not per-run: the Copilot
/// SDK's `UsageMetricsTracker` only ever adds to its per-model counters and
/// exposes no reset, and `Session.shutdown()` emits whatever the tracker holds
/// at that moment with no one-shot guard. So a session that shuts down twice
/// writes two snapshots of the same running total. [`shutdown_deltas`] turns
/// them into the per-run increments the rest of this module assumes.
#[derive(Debug, Clone)]
struct ShutdownUsage {
    /// Identity of the originating event, used to build a dedup key that
    /// survives rotation or compaction of `events.jsonl`. Position in the file
    /// would not: dropping one earlier line renumbers every record after it,
    /// so already-submitted rows would come back under new keys and be counted
    /// twice. The event's own `id` is a UUID; its `timestamp` is the fallback.
    event_id: String,
    timestamp_ms: i64,
    /// The `modelMetrics` key, trimmed. That key is the identity of the
    /// tracker counter these numbers came from, so it is what the running peak,
    /// the verbatim-record dedup, and the submitted dedup key are all grouped
    /// by. Trimming it here keeps that grouping identical to the model the
    /// emitted message is attributed to, which is trimmed too: two spellings
    /// that differ only by padding are one model everywhere or nowhere.
    model: String,
    /// Concrete model active for this shutdown when the tracker key is `auto`.
    attributed_model: Option<String>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
}

/// The five token buckets a shutdown record reports, in a fixed order so
/// cumulative snapshots can be differenced bucket-by-bucket.
type UsageBuckets = [i64; 5];

impl ShutdownUsage {
    fn buckets(&self) -> UsageBuckets {
        [
            self.input,
            self.output,
            self.cache_read,
            self.cache_write,
            self.reasoning,
        ]
    }

    fn with_buckets(self, buckets: UsageBuckets) -> Self {
        Self {
            input: buckets[0],
            output: buckets[1],
            cache_read: buckets[2],
            cache_write: buckets[3],
            reasoning: buckets[4],
            ..self
        }
    }
}

/// What one session's shutdown snapshots resolve to: the increments to emit,
/// and the usage they account for.
struct ShutdownAttribution {
    /// One entry per snapshot that added usage, already differenced into the
    /// increment it contributed.
    deltas: Vec<ShutdownUsage>,
    /// The total the snapshots account for, which the caller subtracts from the
    /// row's lifetime total. Not necessarily the sum of `deltas`.
    consumed: UsageBuckets,
}

/// Convert cumulative shutdown snapshots into the usage each one actually
/// added, so summing them reconciles against the row's lifetime total instead
/// of multiplying it.
///
/// Without this, a session that emitted an error shutdown at 100 tokens and a
/// routine one at 200 contributes 300 — the earlier snapshot counted twice,
/// and spread across two different days.
///
/// Snapshots are grouped by their `modelMetrics` key, which is the identity of
/// the tracker counter they were read from and the same identity the emitted
/// message is keyed and attributed by.
///
/// A snapshot that reports *less* than one before it (the tracker restarted
/// with the session, or the records arrived out of order) contributes nothing
/// rather than a negative bucket: each model's running peak is the baseline.
/// Cache-read growth is additionally capped by inclusive-input growth in the
/// same snapshot, because input includes cache reads and the two cannot safely
/// advance independently. Anything the snapshots leave unexplained is still
/// reconciled by the caller's residual against the `sessions` row.
///
/// `complete_from_start` says whether the log still begins where the session
/// did. When it does, the first snapshot seen for a model really is that
/// model's first, and zero is the right baseline to difference it from. When
/// it does not, an earlier snapshot may have been rotated away: that snapshot's
/// increment was already submitted under a dedup key this parse can no longer
/// reproduce, so differencing the survivor from zero would re-emit it under a
/// key whose day is only ever ratcheted upwards. The survivor is treated as an
/// unknown baseline instead — it sets the peak and contributes nothing.
///
/// That is deliberately the conservative direction. On a machine that had
/// already submitted the rotated-away snapshot it is exact; on one scanning a
/// truncated log for the first time it under-reports the baseline rather than
/// re-dating it, because nothing on disk distinguishes the two cases.
fn shutdown_deltas(
    mut snapshots: Vec<ShutdownUsage>,
    complete_from_start: bool,
) -> ShutdownAttribution {
    // A record repeated verbatim — the same event written twice by a
    // re-flushed or replayed log — describes one shutdown and must count once.
    let mut seen = HashSet::new();
    snapshots.retain(|snapshot| seen.insert((snapshot.event_id.clone(), snapshot.model.clone())));

    // Order by the envelope timestamp so "previous snapshot" means what it
    // says; the sort is stable, so records sharing a timestamp keep file order.
    snapshots.sort_by_key(|snapshot| snapshot.timestamp_ms);

    let mut peaks: HashMap<String, UsageBuckets> = HashMap::new();
    let mut deltas = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let current = snapshot.buckets();
        let baseline = match peaks.get(&snapshot.model) {
            Some(peak) => Some(*peak),
            None if complete_from_start => Some(UsageBuckets::default()),
            None => None,
        };

        let peak = peaks.entry(snapshot.model.clone()).or_insert(current);
        for (index, value) in current.iter().enumerate() {
            peak[index] = peak[index].max(*value);
        }

        let Some(baseline) = baseline else {
            continue;
        };
        let mut delta = UsageBuckets::default();
        for (index, value) in current.iter().enumerate() {
            delta[index] = value.saturating_sub(baseline[index]).max(0);
        }
        // `inputTokens` includes cache reads. If a reset/out-of-order snapshot
        // lowers inclusive input while raising cache reads, emitting that cache
        // growth independently would mint tokens: normalization would subtract
        // it from a zero input delta and then retain it as a cache bucket. Any
        // cache-read increment is therefore bounded by the inclusive-input
        // increment observed in the same snapshot.
        delta[2] = delta[2].min(delta[0]);
        if delta.iter().all(|bucket| *bucket == 0) {
            continue;
        }
        deltas.push(snapshot.with_buckets(delta));
    }

    // A later reset can reveal a higher cache-read composition without any
    // inclusive-input growth. The cap above prevents minting tokens, then this
    // pass uses spare cache capacity in already-authorized input increments so
    // the emitted bucket totals still match the final cache high-water. Newest
    // authorized increments receive the reclassification first; no increment
    // can hold more cache reads than inclusive input.
    for (model, peak) in &peaks {
        let target_cache = peak[2].min(peak[0]);
        let assigned_cache: i64 = deltas
            .iter()
            .filter(|delta| delta.model == *model)
            .map(|delta| delta.cache_read)
            .sum();
        let mut remaining_cache = target_cache.saturating_sub(assigned_cache);
        for delta in deltas
            .iter_mut()
            .rev()
            .filter(|delta| delta.model == *model)
        {
            let capacity = delta.input.saturating_sub(delta.cache_read);
            let reassigned = capacity.min(remaining_cache);
            delta.cache_read = delta.cache_read.saturating_add(reassigned);
            remaining_cache = remaining_cache.saturating_sub(reassigned);
            if remaining_cache == 0 {
                break;
            }
        }
    }

    // Every model's peak is the highest total it was ever observed holding, so
    // summing the peaks is what the snapshots account for whether or not each
    // one was emitted. Using the emitted deltas instead would hand a swallowed
    // baseline back to the residual and re-date it to `created_at`.
    let consumed = peaks
        .values()
        .fold(UsageBuckets::default(), |mut total, peak| {
            for (index, value) in peak.iter().enumerate() {
                total[index] = total[index].saturating_add(*value);
            }
            total
        });

    ShutdownAttribution { deltas, consumed }
}

pub fn parse_copilot_desktop_db(db_path: &Path) -> Vec<UnifiedMessage> {
    let query = r#"
        SELECT
            id,
            title,
            model,
            total_input_tokens,
            total_output_tokens,
            total_cached_tokens,
            total_reasoning_tokens,
            total_nano_aiu,
            created_at
        FROM sessions
        WHERE total_input_tokens > 0
           OR total_output_tokens > 0
           OR total_cached_tokens > 0
           OR total_reasoning_tokens > 0
        "#;

    let mut messages = Vec::new();
    sqlite_for_each_row(
        db_path,
        query,
        Some("Copilot Desktop session"),
        &mut |row| {
            let session = CopilotDesktopSessionRow {
                id: row.get(0)?,
                model: row.get(2)?,
                total_input_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                total_output_tokens: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                total_cached_tokens: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                total_reasoning_tokens: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                created_at: row.get(8)?,
            };
            messages.extend(session_row_to_messages(db_path, session));
            Ok(())
        },
    );

    messages
}

/// Turn one `sessions` row into the messages its usage actually belongs to.
///
/// The row holds a lifetime total against an immutable `created_at`, so
/// emitting it as-is re-dated every later turn to the day the session was
/// opened: that day grew on every rescan and the days the tokens were really
/// spent on received none of them (#962).
///
/// `session.shutdown` records carry their own timestamp and a per-model
/// breakdown, so each one is emitted at its own time and under its own model.
/// Their token counts are cumulative, so [`shutdown_deltas`] has already
/// reduced them to per-run increments by the time they arrive here.
/// Whatever they do not account for — a run that died before writing its
/// shutdown, or a session recorded by the CLI rather than the desktop app —
/// stays on `created_at` under the row's original dedup key, so the row
/// remains the authority on the all-time total and nothing is dropped.
fn session_row_to_messages(db_path: &Path, row: CopilotDesktopSessionRow) -> Vec<UnifiedMessage> {
    let metadata = read_session_state_metadata(db_path, &row.id);
    let fallback_model = metadata
        .model
        .as_deref()
        .or(row.model.as_deref())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or("auto")
        .to_string();

    let created_at_ms = row
        .created_at
        .as_deref()
        .and_then(parse_iso8601_timestamp_ms)
        .unwrap_or_else(|| {
            warn!(
                session_id = %row.id,
                created_at = ?row.created_at,
                "Copilot Desktop session has unparseable created_at; defaulting to 0"
            );
            0
        });

    let workspace_key = metadata.cwd.as_deref().and_then(normalize_workspace_key);
    let build = |model_id: String, timestamp_ms: i64, tokens, dedup_key: String| {
        let provider_id = inferred_provider_from_model(&model_id)
            .unwrap_or("github-copilot")
            .to_string();
        let mut message = UnifiedMessage::new_with_dedup(
            "copilot",
            model_id,
            provider_id,
            row.id.clone(),
            timestamp_ms,
            tokens,
            0.0,
            Some(dedup_key),
        );
        if let Some(workspace_key) = workspace_key.clone() {
            let workspace_label = workspace_label_from_key(&workspace_key);
            message.set_workspace(Some(workspace_key), workspace_label);
        }
        message
    };

    let mut messages = Vec::with_capacity(metadata.shutdowns.len() + 1);
    // The SQLite row is authoritative for every bucket it stores. The sidecar
    // and DB are separate files, so a shutdown can be observed before the row
    // catches up; consume a per-row budget to prevent that race from emitting
    // more lifetime usage than the row. Cache-write has no SQLite column and
    // remains sidecar-authoritative.
    let mut remaining_input = row.total_input_tokens.max(0);
    let mut remaining_output = row.total_output_tokens.max(0);
    let mut remaining_cache_read = row.total_cached_tokens.max(0);
    let mut remaining_reasoning = row.total_reasoning_tokens.max(0);
    for shutdown in &metadata.shutdowns {
        // `auto` is resolved for display and pricing only. It is a tracker
        // counter of its own — `modelMetrics` is keyed by the model each
        // `assistant.usage` event reported — so it keeps its own peak and its
        // own dedup key even when it is attributed to the resolved model.
        // Folding it into `fallback_model` before differencing would subtract
        // one counter's peak from another counter's total.
        let model_id = shutdown
            .attributed_model
            .clone()
            .unwrap_or_else(|| fallback_model.clone());
        let input = shutdown.input.min(remaining_input);
        let output = shutdown.output.min(remaining_output);
        let cache_read = shutdown.cache_read.min(remaining_cache_read).min(input);
        let reasoning = shutdown.reasoning.min(remaining_reasoning);
        remaining_input = remaining_input.saturating_sub(input);
        remaining_output = remaining_output.saturating_sub(output);
        remaining_cache_read = remaining_cache_read.saturating_sub(cache_read);
        remaining_reasoning = remaining_reasoning.saturating_sub(reasoning);
        let tokens = super::copilot::normalize_input_tokens(
            input,
            output,
            cache_read,
            shutdown.cache_write,
            reasoning,
        );
        if tokens.total() == 0 {
            continue;
        }
        messages.push(build(
            model_id,
            shutdown.timestamp_ms,
            tokens,
            format!(
                "copilot-desktop:{}:shutdown:{}:{}",
                row.id, shutdown.event_id, shutdown.model
            ),
        ));
    }

    // What the snapshots account for, which is not always what they emitted.
    // An unknown baseline may already have been submitted before the log head
    // disappeared, so re-emitting it would inflate that machine permanently.
    // On a first-ever scan of an already-truncated log this deliberately
    // under-reports instead; no remaining record can distinguish those cases
    // (see shutdown_deltas' safety tradeoff above).
    let consumed = metadata.consumed;
    // The row's own cache-write column does not exist, so the shutdown records
    // are the only source for that bucket and there is nothing to reconcile.
    let residual_input = (row.total_input_tokens - consumed[0]).max(0);
    let residual_cache_read = (row.total_cached_tokens - consumed[2])
        .max(0)
        .min(residual_input);
    let residual = super::copilot::normalize_input_tokens(
        residual_input,
        (row.total_output_tokens - consumed[1]).max(0),
        residual_cache_read,
        0,
        (row.total_reasoning_tokens - consumed[4]).max(0),
    );

    if residual.total() > 0 {
        messages.push(build(
            fallback_model,
            created_at_ms,
            residual,
            format!("copilot-desktop:{}", row.id),
        ));
    }

    // The SQLite row has always represented one Copilot session/message. The
    // shutdown metadata only splits that row by time and model; it must not
    // turn one legacy count into one count per attributed fragment. Assign the
    // authoritative count to exactly one fragment and make every other split
    // row count-neutral.
    for (index, message) in messages.iter_mut().enumerate() {
        message.message_count = i32::from(index == 0);
    }

    messages
}

fn read_session_state_metadata(db_path: &Path, session_id: &str) -> SessionStateMetadata {
    let Some(copilot_root) = db_path.parent() else {
        return SessionStateMetadata::default();
    };
    let events_path = copilot_root
        .join("session-state")
        .join(session_id)
        .join("events.jsonl");

    read_events_metadata(&events_path)
}

fn read_events_metadata(events_path: &Path) -> SessionStateMetadata {
    let file = match std::fs::File::open(events_path) {
        Ok(file) => file,
        Err(_) => return SessionStateMetadata::default(),
    };

    let mut metadata = SessionStateMetadata::default();
    // The SDK builds a session by replaying this file and rejects one whose
    // first event is not `session.start`, and the only removal it performs
    // keeps the prefix and drops the tail. A log that does not open with
    // `session.start` has therefore lost its head to something else, and the
    // records that used to precede the survivors are unrecoverable.
    let mut first_event_type: Option<String> = None;
    for line in lossy_lines(BufReader::new(file)) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let Some(event_type) = event.get("type").and_then(Value::as_str) else {
            continue;
        };
        if first_event_type.is_none() {
            first_event_type = Some(event_type.to_string());
        }

        match event_type {
            "session.start" if metadata.cwd.is_none() => {
                metadata.cwd = event
                    .pointer("/data/context/cwd")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|cwd| !cwd.is_empty())
                    .map(str::to_string);
            }
            "session.model_change" => {
                if let Some(model) = event
                    .pointer("/data/newModel")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|model| !model.is_empty() && model != &"auto")
                {
                    metadata.model = Some(model.to_string());
                }
            }
            "session.shutdown" => collect_shutdown_usage(&event, &mut metadata.shutdowns),
            _ => {}
        }
    }

    // Everything downstream treats one entry as one run's spend, so hand back
    // increments rather than the cumulative snapshots the app writes.
    let complete_from_start = first_event_type.as_deref() == Some("session.start");
    let attribution = shutdown_deltas(std::mem::take(&mut metadata.shutdowns), complete_from_start);
    metadata.shutdowns = attribution.deltas;
    metadata.consumed = attribution.consumed;
    metadata
}

fn collect_shutdown_usage(event: &Value, out: &mut Vec<ShutdownUsage>) {
    // The desktop app nests event payloads under `data`; a flat record is
    // accepted too so a shutdown that omits the envelope still reports usage
    // rather than silently contributing nothing.
    let payload = event.get("data").unwrap_or(event);
    // The timestamp lives on the envelope next to `id`/`parentId`, not in the
    // payload, and it is an ISO-8601 string. Reading the payload first only
    // matters for a flat record that has no envelope to read from.
    let Some(timestamp_ms) = event
        .get("timestamp")
        .or_else(|| payload.get("timestamp"))
        .and_then(Value::as_str)
        .and_then(parse_iso8601_timestamp_ms)
    else {
        return;
    };
    // `events.jsonl` is append-only in practice, but nothing guarantees it
    // stays that way, so key off the event's own identity rather than its
    // position in the file.
    let event_id = event
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            // Real records carry UUIDs. For a malformed/legacy record without
            // one, hash the stable event content: a timestamp alone collides
            // when two distinct shutdowns share the same millisecond, while a
            // file ordinal would change when earlier lines rotate away.
            let digest = Sha256::digest(event.to_string().as_bytes());
            format!("anon-{digest:x}")
        });
    let Some(metrics) = payload
        .get("modelMetrics")
        .or_else(|| event.get("modelMetrics"))
        .and_then(Value::as_object)
    else {
        return;
    };

    let current_model = payload
        .get("currentModel")
        .or_else(|| event.get("currentModel"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty() && *model != "auto")
        .map(str::to_string);

    for (model, entry) in metrics {
        let Some(usage) = entry.get("usage") else {
            continue;
        };
        let read = |key: &str| usage.get(key).and_then(Value::as_i64).unwrap_or(0).max(0);
        let tracker_model = model.trim().to_string();
        let attributed_model = match tracker_model.as_str() {
            "" | "auto" => current_model.clone(),
            _ => Some(tracker_model.clone()),
        };
        let shutdown = ShutdownUsage {
            event_id: event_id.clone(),
            timestamp_ms,
            model: tracker_model,
            attributed_model,
            input: read("inputTokens"),
            output: read("outputTokens"),
            cache_read: read("cacheReadTokens"),
            cache_write: read("cacheWriteTokens"),
            reasoning: read("reasoningTokens"),
        };
        if shutdown.input == 0
            && shutdown.output == 0
            && shutdown.cache_read == 0
            && shutdown.cache_write == 0
            && shutdown.reasoning == 0
        {
            continue;
        }
        out.push(shutdown);
    }
}

fn parse_iso8601_timestamp_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|timestamp| timestamp.and_utc().timestamp_millis())
        })
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|timestamp| timestamp.and_utc().timestamp_millis())
        })
        .or_else(|| {
            // SQLite's default datetime() text form is space-separated and may
            // carry fractional seconds ("2026-07-01 12:34:56.789"); without this
            // branch it fails every parse above and the session lands in 1970.
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .map(|timestamp| timestamp.and_utc().timestamp_millis())
        })
        .or_else(|| {
            let numeric = value.parse::<i64>().ok()?;
            // Distinguish seconds vs milliseconds: values < 10 billion are
            // assumed to be Unix seconds (common in SQLite), otherwise millis.
            if numeric > 10_000_000_000 {
                Some(numeric)
            } else {
                Some(numeric.saturating_mul(1000))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use std::fs::{self, File};
    use std::io::Write;

    #[test]
    fn parse_copilot_desktop_db_returns_empty_for_missing_database() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.db");
        assert!(parse_copilot_desktop_db(&missing).is_empty());
    }

    fn create_copilot_desktop_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT,
                title TEXT,
                session_type TEXT,
                mode TEXT,
                model TEXT,
                total_input_tokens INTEGER,
                total_output_tokens INTEGER,
                total_cached_tokens INTEGER,
                total_reasoning_tokens INTEGER,
                total_nano_aiu INTEGER,
                created_at TEXT,
                agent TEXT,
                provider_id TEXT
            );
            "#,
        )
        .unwrap();
        conn
    }

    fn insert_session(
        conn: &Connection,
        id: &str,
        model: &str,
        input: i64,
        output: i64,
        cached: i64,
        reasoning: i64,
    ) {
        conn.execute(
            r#"
            INSERT INTO sessions (
                id, title, session_type, mode, model,
                total_input_tokens, total_output_tokens, total_cached_tokens,
                total_reasoning_tokens, total_nano_aiu, created_at, agent, provider_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                id,
                "Test session",
                "chat",
                "agent",
                model,
                input,
                output,
                cached,
                reasoning,
                0_i64,
                "2026-07-01T12:34:56Z",
                "github.copilot.default",
                "github-copilot"
            ],
        )
        .unwrap();
    }

    /// Every real `events.jsonl` opens with `session.start`: the SDK refuses to
    /// load a session whose first event is anything else, so a log that does
    /// not start with one has lost its head. Fixtures that exercise shutdown
    /// attribution open with it for the same reason real logs do.
    const SESSION_START: &str = r#"{"type":"session.start","data":{},"id":"3f0a1c22-6b41-4d0e-9c7a-5e2b8d4f1a00","timestamp":"2026-07-01T19:00:00.000Z"}"#;

    fn write_events(root: &Path, session_id: &str, lines: &[&str]) {
        let events_dir = root.join("session-state").join(session_id);
        fs::create_dir_all(&events_dir).unwrap();
        let mut file = File::create(events_dir.join("events.jsonl")).unwrap();
        for line in lines {
            writeln!(file, "{}", line).unwrap();
        }
    }

    #[test]
    fn parse_copilot_desktop_db_reads_token_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 100, 50, 25, 10);
        drop(conn);

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "copilot");
        assert_eq!(message.model_id, "gpt-5.1-codex");
        assert_eq!(message.provider_id, "openai");
        assert_eq!(message.session_id, "session-1");
        assert_eq!(message.timestamp, 1_782_909_296_000);
        // total_input_tokens is inclusive of cache reads, so the cached portion
        // (25) is normalized out of input: 100 - 25 = 75.
        assert_eq!(message.tokens.input, 75);
        assert_eq!(message.tokens.output, 50);
        assert_eq!(message.tokens.cache_read, 25);
        assert_eq!(message.tokens.cache_write, 0);
        assert_eq!(message.tokens.reasoning, 10);
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("copilot-desktop:session-1")
        );
    }

    #[test]
    fn parse_copilot_desktop_db_skips_zero_token_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 0, 0, 0, 0);
        drop(conn);

        assert!(parse_copilot_desktop_db(&db_path).is_empty());
    }

    #[test]
    fn parse_copilot_desktop_db_enriches_model_and_workspace_from_events() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "auto", 100, 50, 0, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                r#"{"type":"session.start","data":{"context":{"cwd":"/Users/alice/project"}}}"#,
                r#"{"type":"session.model_change","data":{"newModel":"claude-sonnet-4-5"}}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.model_id, "claude-sonnet-4-5");
        assert_eq!(message.provider_id, "anthropic");
        assert_eq!(message.workspace_label.as_deref(), Some("project"));
    }

    #[test]
    fn keeps_reading_events_after_an_undecodable_line() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "auto", 100, 50, 0, 0);
        drop(conn);

        let events_dir = dir.path().join("session-state").join("session-1");
        fs::create_dir_all(&events_dir).unwrap();
        let mut fixture = Vec::new();
        fixture.extend_from_slice(
            br#"{"type":"session.start","data":{"context":{"cwd":"/Users/alice/project"}}}"#,
        );
        fixture.push(b'\n');
        // A lone 0xff can never appear in valid UTF-8, so `BufRead::lines()`
        // reports this line as `InvalidData` and `map_while(Result::ok)` would
        // treat it as end of file, losing the model change below it.
        fixture.extend_from_slice(b"{\"type\":\"session.note\",\"data\":\"\xff\xfe\"}\n");
        fixture.extend_from_slice(
            br#"{"type":"session.model_change","data":{"newModel":"claude-sonnet-4-5"}}"#,
        );
        fixture.push(b'\n');
        fs::write(events_dir.join("events.jsonl"), &fixture).unwrap();

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4-5");
        assert_eq!(messages[0].provider_id, "anthropic");
    }

    #[test]
    fn parse_copilot_desktop_db_uses_github_copilot_provider_for_auto() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "auto", 100, 0, 0, 0);
        drop(conn);

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "github-copilot");
    }

    /// Regression (#962): the row carries a lifetime total and an immutable
    /// `created_at`, so every rescan grew the creation day and gave the days
    /// the tokens were actually spent on nothing. `session.shutdown` records
    /// carry their own timestamp, so usage lands on the day it happened.
    #[test]
    fn shutdown_events_attribute_usage_to_their_own_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 100, 50, 25, 10);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"requests":{"count":1,"cost":1},"usage":{"inputTokens":100,"outputTokens":50,"cacheReadTokens":25,"cacheWriteTokens":0,"reasoningTokens":10}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01"}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1, "the row total is fully accounted for");
        let message = &messages[0];
        assert_eq!(
            message.timestamp, 1_782_950_400_000,
            "usage belongs to the shutdown day, not the creation day"
        );
        assert_eq!(message.model_id, "gpt-5.1-codex");
        assert_eq!(message.tokens.input, 75);
        assert_eq!(message.tokens.output, 50);
        assert_eq!(message.tokens.cache_read, 25);
        assert_eq!(message.tokens.reasoning, 10);
        assert_eq!(
            message.dedup_key.as_deref(),
            Some(
                "copilot-desktop:session-1:shutdown:9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02:gpt-5.1-codex"
            ),
            "the shutdown message is keyed by the event's own id"
        );
    }

    /// Whatever the shutdown records do not account for still has to be kept,
    /// so the row stays the authority on the all-time total when a run dies
    /// before it can write its shutdown.
    #[test]
    fn usage_beyond_the_shutdown_events_stays_at_session_creation() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 200, 100, 50, 20);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"requests":{"count":1,"cost":1},"usage":{"inputTokens":100,"outputTokens":50,"cacheReadTokens":25,"cacheWriteTokens":0,"reasoningTokens":10}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01"}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 2);
        let residual = messages
            .iter()
            .find(|message| message.timestamp == 1_782_909_296_000)
            .expect("the unaccounted remainder stays on the creation day");
        assert_eq!(residual.tokens.input, 75);
        assert_eq!(residual.tokens.output, 50);
        assert_eq!(residual.tokens.cache_read, 25);
        assert_eq!(residual.tokens.reasoning, 10);
        assert_eq!(
            residual.dedup_key.as_deref(),
            Some("copilot-desktop:session-1"),
            "the remainder keeps the row's own dedup key"
        );

        let total_input: i64 = messages.iter().map(|message| message.tokens.input).sum();
        assert_eq!(total_input, 150, "the row total is preserved exactly");
        assert_eq!(
            messages
                .iter()
                .map(|message| message.message_count)
                .sum::<i32>(),
            1,
            "a shutdown plus residual still represents one SQLite session"
        );
    }

    /// The `sessions` table has no cache-write column, so that bucket was
    /// hardcoded to zero. The shutdown records do carry it.
    #[test]
    fn shutdown_events_recover_cache_write_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 100, 50, 25, 10);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"requests":{"count":1,"cost":1},"usage":{"inputTokens":100,"outputTokens":50,"cacheReadTokens":25,"cacheWriteTokens":7,"reasoningTokens":10}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01"}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        let shutdown = messages
            .iter()
            .find(|message| message.timestamp == 1_782_950_400_000)
            .expect("shutdown message");
        assert_eq!(shutdown.tokens.cache_write, 7);
    }

    /// `modelMetrics` is keyed by model, which attributes each model's usage
    /// exactly instead of letting the last `session.model_change` claim the
    /// whole session.
    #[test]
    fn shutdown_events_split_usage_per_model() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "auto", 300, 60, 0, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"outputTokens":20}},"claude-sonnet-4-5":{"usage":{"inputTokens":200,"outputTokens":40}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01"}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        let codex = messages
            .iter()
            .find(|message| message.model_id == "gpt-5.1-codex")
            .expect("codex row");
        let claude = messages
            .iter()
            .find(|message| message.model_id == "claude-sonnet-4-5")
            .expect("claude row");
        assert_eq!(codex.tokens.input, 100);
        assert_eq!(codex.provider_id, "openai");
        assert_eq!(claude.tokens.input, 200);
        assert_eq!(claude.provider_id, "anthropic");
        assert_eq!(
            messages
                .iter()
                .map(|message| message.message_count)
                .sum::<i32>(),
            1,
            "splitting one session across models must not inflate its count"
        );
    }

    /// `currentModel` belongs to each shutdown payload. Using the final session
    /// model for every `auto` tracker fragment would move an earlier run to a
    /// model selected only later.
    #[test]
    fn auto_shutdowns_keep_the_model_active_for_each_run() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "auto", 200, 0, 0, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"currentModel":"gpt-5.1-codex","modelMetrics":{"auto":{"usage":{"inputTokens":100}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#,
                r#"{"type":"session.shutdown","data":{"currentModel":"claude-sonnet-4-5","modelMetrics":{"auto":{"usage":{"inputTokens":200}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);
        let first = messages
            .iter()
            .find(|message| message.timestamp == 1_782_936_000_000)
            .expect("first shutdown");
        let second = messages
            .iter()
            .find(|message| message.timestamp == 1_782_950_400_000)
            .expect("second shutdown");

        assert_eq!(
            (first.model_id.as_str(), first.tokens.input),
            ("gpt-5.1-codex", 100)
        );
        assert_eq!(
            (second.model_id.as_str(), second.tokens.input),
            ("claude-sonnet-4-5", 100)
        );
    }

    /// A `session.shutdown` record captured verbatim from a real
    /// `~/.copilot/session-state/<id>/events.jsonl` on macOS (Copilot CLI
    /// 1.0.25), with only the two UUIDs replaced. It pins the shape the desktop
    /// app actually writes: `timestamp` is an ISO-8601 string on the envelope
    /// next to `id`/`parentId`, `modelMetrics` is nested under `data`, and the
    /// usage bucket carries `cacheWriteTokens`. Reading the timestamp from a
    /// `ts` key under `data` finds nothing and drops the record.
    #[test]
    fn real_shutdown_record_attributes_usage_to_its_own_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.4", 21_067, 29, 19_968, 22);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","totalPremiumRequests":1,"totalApiDurationMs":2970,"sessionStartTime":1776192215193,"codeChanges":{"linesAdded":0,"linesRemoved":0,"filesModified":[]},"modelMetrics":{"gpt-5.4":{"requests":{"count":1,"cost":1},"usage":{"inputTokens":21067,"outputTokens":29,"cacheReadTokens":19968,"cacheWriteTokens":0,"reasoningTokens":22}}},"currentModel":"gpt-5.4","currentTokens":22592,"systemTokens":9923,"conversationTokens":83,"toolDefinitionsTokens":12583},"id":"c1a4b7e2-90d3-4f61-8ba5-7d2e6f0c9134","timestamp":"2026-04-14T18:43:44.922Z","parentId":"5b8f3d10-2c47-4e89-a6f0-11d9c4e78a25"}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1, "the row total is fully accounted for");
        let message = &messages[0];
        assert_eq!(
            message.timestamp, 1_776_192_224_922,
            "the envelope timestamp is the run's own time, not `created_at`"
        );
        assert_eq!(message.model_id, "gpt-5.4");
        assert_eq!(message.tokens.input, 1_099);
        assert_eq!(message.tokens.output, 29);
        assert_eq!(message.tokens.cache_read, 19_968);
        assert_eq!(message.tokens.reasoning, 22);
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("copilot-desktop:session-1:shutdown:c1a4b7e2-90d3-4f61-8ba5-7d2e6f0c9134:gpt-5.4")
        );
    }

    /// The dedup key has to identify the record, not its offset. Keying on the
    /// enumeration index holds only while `events.jsonl` is strictly
    /// append-only: rotate, truncate, or compact away an earlier shutdown and
    /// every later index shifts down, so usage that was already submitted comes
    /// back under a fresh key and the server counts it twice.
    #[test]
    fn shutdown_dedup_key_survives_an_earlier_shutdown_being_rotated_away() {
        let first = r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"outputTokens":50}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#;
        let second = r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":200,"outputTokens":60}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#;

        let key_of_second = |lines: &[&str]| -> (String, i64) {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("data.db");
            let conn = create_copilot_desktop_db(&db_path);
            // The later snapshot restates the earlier one, so the row's
            // lifetime total is the final snapshot rather than their sum.
            insert_session(&conn, "session-1", "gpt-5.1-codex", 200, 60, 0, 0);
            drop(conn);
            write_events(dir.path(), "session-1", lines);

            let messages = parse_copilot_desktop_db(&db_path);
            let key = messages
                .iter()
                .find(|message| message.timestamp == 1_782_950_400_000)
                .and_then(|message| message.dedup_key.clone())
                .expect("the second shutdown is always emitted");
            let total_input = messages.iter().map(|message| message.tokens.input).sum();
            (key, total_input)
        };

        let whole_history = key_of_second(&[SESSION_START, first, second]);
        // The earlier shutdown is gone but the log still opens where the
        // session did, so the survivor is still differenced and emitted; what
        // must not change is the key it is emitted under.
        let after_rotation = key_of_second(&[SESSION_START, second]);

        assert_eq!(
            whole_history.0, after_rotation.0,
            "dropping the earlier shutdown must not re-key the later one"
        );
        assert_eq!(
            whole_history.0,
            "copilot-desktop:session-1:shutdown:9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02:gpt-5.1-codex"
        );
        assert_eq!(whole_history.1, 200);
        assert_eq!(
            after_rotation.1, 200,
            "even a synthetic selective edit cannot exceed the authoritative row total"
        );
    }

    /// `session.shutdown` reports the tracker's running total, not the spend
    /// since the last shutdown: the SDK's `UsageMetricsTracker` only ever adds
    /// to its per-model counters and never resets, and `Session.shutdown()`
    /// emits `modelMetrics` as-is with no one-shot guard, so an error shutdown
    /// followed by a routine one writes two snapshots of the same total.
    /// Summing them counted the earlier snapshot twice and dated the phantom
    /// tokens to a day they were never spent on.
    #[test]
    fn cumulative_shutdown_snapshots_are_differenced_not_summed() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 200, 100, 0, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"error","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"outputTokens":50}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":200,"outputTokens":100}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        let total_input: i64 = messages.iter().map(|message| message.tokens.input).sum();
        let total_output: i64 = messages.iter().map(|message| message.tokens.output).sum();
        assert_eq!(
            (total_input, total_output),
            (200, 100),
            "the second snapshot restates the first; summing them would report 300/150"
        );

        let first = messages
            .iter()
            .find(|message| message.timestamp == 1_782_936_000_000)
            .expect("the first shutdown keeps its own day");
        assert_eq!((first.tokens.input, first.tokens.output), (100, 50));

        let second = messages
            .iter()
            .find(|message| message.timestamp == 1_782_950_400_000)
            .expect("the second shutdown keeps its own day");
        assert_eq!(
            (second.tokens.input, second.tokens.output),
            (100, 50),
            "only the increment accrued since the previous snapshot"
        );

        assert!(
            !messages
                .iter()
                .any(|message| message.dedup_key.as_deref() == Some("copilot-desktop:session-1")),
            "the snapshots account for the whole row, so there is no remainder"
        );
    }

    /// Distinct legacy/malformed records can share a millisecond. Their
    /// fallback identity must come from stable content rather than timestamp or
    /// mutable file position.
    #[test]
    fn idless_shutdowns_at_the_same_timestamp_keep_distinct_stable_keys() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 200, 0, 0, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100}}}},"timestamp":"2026-07-02T00:00:00.000Z","parentId":"parent-a"}"#,
                r#"{"type":"session.shutdown","data":{"modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":200}}}},"timestamp":"2026-07-02T00:00:00.000Z","parentId":"parent-b"}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);
        let keys: HashSet<&str> = messages
            .iter()
            .filter_map(|message| message.dedup_key.as_deref())
            .collect();

        assert_eq!(messages.len(), 2);
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().all(|key| key.contains(":shutdown:anon-")));
    }

    /// A record written twice — a replayed or re-flushed log — describes one
    /// shutdown. Keying on the event id is only half the fix: the parser also
    /// has to collapse the repeat before it reads the numbers off it.
    #[test]
    fn a_repeated_shutdown_record_counts_once() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 100, 50, 0, 0);
        drop(conn);
        let record = r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"outputTokens":50}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#;
        write_events(dir.path(), "session-1", &[SESSION_START, record, record]);

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1, "the repeated record is one shutdown");
        assert_eq!(messages[0].timestamp, 1_782_950_400_000);
        assert_eq!(
            (messages[0].tokens.input, messages[0].tokens.output),
            (100, 50)
        );
    }

    /// A snapshot lower than the one before it means the tracker started over
    /// with a fresh session object, or the records were read out of order.
    /// Either way the difference is not negative usage, and it must not be
    /// added on top of the peak already attributed.
    #[test]
    fn a_shutdown_snapshot_that_decreases_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 200, 100, 0, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":200,"outputTokens":100}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":50,"outputTokens":20}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        assert!(
            messages.iter().all(|message| message.tokens.input >= 0
                && message.tokens.output >= 0
                && message.tokens.cache_read >= 0
                && message.tokens.cache_write >= 0
                && message.tokens.reasoning >= 0),
            "a lower snapshot must never produce a negative bucket"
        );

        let total_input: i64 = messages.iter().map(|message| message.tokens.input).sum();
        let total_output: i64 = messages.iter().map(|message| message.tokens.output).sum();
        assert_eq!(
            (total_input, total_output),
            (200, 100),
            "the row total is the authority; the lower snapshot adds nothing"
        );

        assert!(
            !messages
                .iter()
                .any(|message| message.timestamp == 1_782_950_400_000),
            "a snapshot that explains no new usage is not emitted at all"
        );
    }

    /// The sidecar can be flushed before SQLite. A temporarily newer shutdown
    /// must not exceed the row's authoritative lifetime buckets.
    #[test]
    fn shutdown_usage_is_bounded_by_the_sqlite_row() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 100, 50, 25, 10);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":200,"outputTokens":100,"cacheReadTokens":80,"cacheWriteTokens":7,"reasoningTokens":20}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 75);
        assert_eq!(messages[0].tokens.cache_read, 25);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.reasoning, 10);
        assert_eq!(messages[0].tokens.cache_write, 7);
    }

    /// Cache is a subset of inclusive input. A sidecar that accounts for all
    /// row input but temporarily omits cache metadata cannot leave a cache-only
    /// residual that pushes normalized usage above the row total.
    #[test]
    fn residual_cache_is_bounded_by_residual_inclusive_input() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 100, 0, 50, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"cacheReadTokens":0}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);
        let normalized_input: i64 = messages
            .iter()
            .map(|message| message.tokens.input + message.tokens.cache_read)
            .sum();

        assert_eq!(normalized_input, 100);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.cache_read)
                .sum::<i64>(),
            0
        );
    }

    /// Inclusive input and cache reads are not independent totals. A reset or
    /// out-of-order snapshot may lower inclusive input while raising the cache
    /// sub-bucket, but that cannot authorize more lifetime input usage.
    #[test]
    fn cache_growth_without_inclusive_input_growth_does_not_mint_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 100, 0, 90, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"cacheReadTokens":80}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#,
                r#"{"type":"session.shutdown","data":{"modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":90,"cacheReadTokens":90}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);
        let total_input: i64 = messages
            .iter()
            .map(|message| message.tokens.input + message.tokens.cache_read)
            .sum();

        assert_eq!(
            total_input, 100,
            "the row lifetime input remains authoritative"
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.cache_read)
                .sum::<i64>(),
            90,
            "the final cache high-water is preserved without increasing total input"
        );
        assert!(
            !messages
                .iter()
                .any(|message| message.timestamp == 1_782_950_400_000),
            "cache growth without inclusive input growth is not emitted"
        );
    }

    /// Re-attribution only moves usage between days; it never creates or
    /// destroys any. Summing every emitted message — the per-day increments
    /// plus the remainder — reproduces the row's lifetime total exactly, with
    /// `input + cache_read` compared against the row's input because the
    /// normalizer moves the cached portion out of `input` into its own bucket.
    ///
    /// This invariant is what makes the placement change safe to reconcile:
    /// the day a token is credited to changes, the total does not.
    #[test]
    fn re_attribution_conserves_the_row_lifetime_total() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 200, 100, 50, 20);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"error","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"outputTokens":50,"cacheReadTokens":25,"cacheWriteTokens":0,"reasoningTokens":10}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":150,"outputTokens":75,"cacheReadTokens":40,"cacheWriteTokens":0,"reasoningTokens":15}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        let sum = |pick: fn(&UnifiedMessage) -> i64| -> i64 { messages.iter().map(pick).sum() };
        assert_eq!(
            sum(|message| message.tokens.input) + sum(|message| message.tokens.cache_read),
            200,
            "input is conserved once the cached portion is added back"
        );
        assert_eq!(sum(|message| message.tokens.output), 100);
        assert_eq!(sum(|message| message.tokens.cache_read), 50);
        assert_eq!(sum(|message| message.tokens.reasoning), 20);

        let mut days: Vec<i64> = messages.iter().map(|message| message.timestamp).collect();
        days.sort_unstable();
        assert_eq!(
            days,
            vec![1_782_909_296_000, 1_782_936_000_000, 1_782_950_400_000],
            "the same total is spread over the creation day and both shutdown days"
        );
    }

    /// A model's running peak, the verbatim-record dedup, and the dedup key the
    /// message is submitted under all have to name the same model the message
    /// is attributed to. They were keyed on the raw `modelMetrics` key while
    /// the emitted `model_id` was trimmed, so `"gpt-5.1-codex"` and
    /// `" gpt-5.1-codex "` landed on the same model with two separate peaks:
    /// the later snapshot restated the earlier one's total instead of being
    /// differenced against it, and the two records were submitted under keys
    /// that differed only by invisible whitespace.
    #[test]
    fn model_spellings_that_differ_only_by_whitespace_share_one_snapshot_series() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        // The later snapshot restates the earlier one, so the row's lifetime
        // total is the final snapshot rather than their sum.
        insert_session(&conn, "session-1", "gpt-5.1-codex", 200, 100, 0, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                SESSION_START,
                r#"{"type":"session.shutdown","data":{"shutdownType":"error","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"outputTokens":50}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#,
                r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{" gpt-5.1-codex ":{"usage":{"inputTokens":200,"outputTokens":100}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        let total_input: i64 = messages.iter().map(|message| message.tokens.input).sum();
        let total_output: i64 = messages.iter().map(|message| message.tokens.output).sum();
        assert_eq!(
            (total_input, total_output),
            (200, 100),
            "one peak: the padded spelling is the same model, so the second \
             snapshot restates the first instead of adding to it"
        );

        let second = messages
            .iter()
            .find(|message| message.timestamp == 1_782_950_400_000)
            .expect("the second shutdown keeps its own day");
        assert_eq!(
            (second.tokens.input, second.tokens.output),
            (100, 50),
            "only the increment accrued since the previous snapshot"
        );

        assert!(
            messages
                .iter()
                .all(|message| message.model_id == "gpt-5.1-codex"),
            "both spellings are attributed to the same model"
        );
        let mut keys: Vec<&str> = messages
            .iter()
            .filter_map(|message| message.dedup_key.as_deref())
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "copilot-desktop:session-1:shutdown:9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01:gpt-5.1-codex",
                "copilot-desktop:session-1:shutdown:9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02:gpt-5.1-codex",
            ],
            "one dedup identity per model: the key names the model the message \
             is attributed to, not the raw spelling it happened to be written with"
        );
    }

    /// `events.jsonl` is not guaranteed append-only, and losing the head of the
    /// file takes an earlier shutdown snapshot with it. The later record was
    /// already submitted as an increment under a dedup key that does not
    /// change, so differencing it from zero again would raise its day to the
    /// full cumulative total while the earlier day keeps the usage it was
    /// already credited with — permanently adding the rotated-away snapshot on
    /// top of a total that was already complete.
    ///
    /// The invariant: a record's emitted usage never grows because a snapshot
    /// before it disappeared, and the usage it stands for is not re-emitted
    /// somewhere else either.
    #[test]
    fn a_rotated_away_predecessor_does_not_grow_the_later_shutdown() {
        let first = r#"{"type":"session.shutdown","data":{"shutdownType":"error","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":100,"outputTokens":50}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a01","timestamp":"2026-07-01T20:00:00.000Z","parentId":null}"#;
        let second = r#"{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"gpt-5.1-codex":{"usage":{"inputTokens":200,"outputTokens":60}}}},"id":"9f2c6f0e-1d5a-4a7e-9b30-2c8d4e6f1a02","timestamp":"2026-07-02T00:00:00.000Z","parentId":null}"#;

        let parse = |lines: &[&str]| -> Vec<UnifiedMessage> {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("data.db");
            let conn = create_copilot_desktop_db(&db_path);
            // The later snapshot restates the earlier one, so the row's
            // lifetime total is the final snapshot rather than their sum.
            insert_session(&conn, "session-1", "gpt-5.1-codex", 200, 60, 0, 0);
            drop(conn);
            write_events(dir.path(), "session-1", lines);
            parse_copilot_desktop_db(&db_path)
        };
        let second_day = |messages: &[UnifiedMessage]| -> i64 {
            messages
                .iter()
                .find(|message| message.timestamp == 1_782_950_400_000)
                .map_or(0, |message| message.tokens.input)
        };

        let whole_history = parse(&[SESSION_START, first, second]);
        assert_eq!(
            second_day(&whole_history),
            100,
            "with both snapshots the later record is only its own increment"
        );

        // Compaction: the head of the log is gone, so `session.start` and the
        // earlier snapshot went with it and only the later record survives.
        let after_compaction = parse(&[second]);

        assert!(
            second_day(&after_compaction) <= 100,
            "the later record must not grow into the full cumulative total when \
             its predecessor is rotated away; it reported {}",
            second_day(&after_compaction)
        );
        let emitted: i64 = after_compaction
            .iter()
            .map(|message| message.tokens.total())
            .sum();
        assert_eq!(
            emitted, 0,
            "the surviving snapshot restates usage that is already attributed, \
             so it is not re-emitted on the creation day either"
        );
    }

    #[test]
    fn parse_iso8601_handles_space_separated_fractional_seconds() {
        // SQLite datetime() text form; must not fall through to the 1970 default.
        let ms = parse_iso8601_timestamp_ms("2026-07-01 12:34:56.789")
            .expect("space + fractional seconds should parse");
        assert_eq!(ms, 1_782_909_296_789);

        // Sibling formats still parse.
        assert_eq!(
            parse_iso8601_timestamp_ms("2026-07-01T12:34:56Z"),
            Some(1_782_909_296_000)
        );
        assert_eq!(
            parse_iso8601_timestamp_ms("2026-07-01 12:34:56"),
            Some(1_782_909_296_000)
        );
        assert_eq!(parse_iso8601_timestamp_ms("not-a-timestamp"), None);
    }
}
