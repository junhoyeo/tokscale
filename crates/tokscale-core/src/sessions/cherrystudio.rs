//! Cherry Studio (desktop client) agent-session usage parser.
//!
//! Cherry Studio's Agent / Claude Code sessions write **standard Claude Code
//! transcripts** under its per-user app-data directory:
//! `%APPDATA%\CherryStudio\Data\Agents\.claude\projects\<workspace>\<session>.jsonl`
//! (macOS: `~/Library/Application Support/CherryStudio/Data/Agents/.claude/projects/...`,
//! Linux: `$XDG_CONFIG_HOME/CherryStudio/Data/Agents/.claude/projects/...`).
//! The V1 root omits `Data/Agents`; both roots are scanned so pre-upgrade
//! history remains available.
//!
//! Unlike a stock Claude Code transcript, Cherry Studio appends the **same API
//! call to the file 3-4 times** (different `uuid`, identical `requestId`,
//! `message.id`, and `usage`) as the streaming response progresses. `requestId`
//! is the API-call identity, so records sharing it are one call even when a
//! streaming record later gains or changes `message.id`. Naively
//! summing every assistant row triple-counts each call (verified ~3x over the
//! true figure). The canonical fix — validated against DeepSeek's platform
//! per-hour billing, <1% error — is to form alias-connected components across
//! the complete transcript before choosing one contribution; `uuid` is only a
//! fallback when neither primary ID exists.
//! Usage signatures are not identities: two distinct requests may legitimately
//! have identical token counts. Records without an identity are retained
//! conservatively. All reads are strictly read-only.
//!
//! The usage fields come from the assistant event's `message.usage`:
//! `input_tokens` (cache miss), `cache_read_input_tokens` (cache hit),
//! `cache_creation_input_tokens` (cache write) and `output_tokens`.

use super::utils::{
    file_modified_timestamp_ms, for_each_json_line, open_readonly_sqlite_opt, parse_timestamp_str,
    sqlite_for_each_row_on,
};
use super::{normalize_workspace_key, workspace_label_from_key, CostSource, UnifiedMessage};
use crate::TokenBreakdown;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const CLIENT_ID: &str = "cherrystudio";

/// The `message_kind` value Cherry Studio stamps on a built-in chat turn, as
/// opposed to `agent-session` (Agent / Claude Code mode, which the transcript
/// parser already covers).
const CHAT_MESSAGE_KIND: &str = "chat";

/// A valid usage row held until every alias in the transcript has been seen.
///
/// Cherry Studio writes partial stream snapshots before it writes the complete
/// snapshot that relates their UUID, message ID, and request ID. Deduplicating
/// while reading therefore cannot be correct: the earlier snapshots may have
/// already been emitted by the time the connecting row arrives.
struct UsageRecord {
    message: UnifiedMessage,
    /// Event timestamp before the parser falls back to the transcript mtime.
    ///
    /// Keep this separate from `message.timestamp`: a missing timestamp is not
    /// evidence that a replay happened at the file's modification time.
    event_timestamp: Option<i64>,
    request_id: Option<String>,
    aliases: Vec<String>,
}

/// Minimal disjoint-set implementation used to form alias-connected components
/// for a *single* transcript. Components are formed only after parsing, before
/// any usage is returned to the caller.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
            rank: vec![0; len],
        }
    }

    fn find(&mut self, mut node: usize) -> usize {
        let mut root = node;
        while self.parent[root] != root {
            root = self.parent[root];
        }

        while self.parent[node] != node {
            let parent = self.parent[node];
            self.parent[node] = root;
            node = parent;
        }

        root
    }

    fn union(&mut self, left: usize, right: usize) {
        let left = self.find(left);
        let right = self.find(right);
        if left == right {
            return;
        }

        match self.rank[left].cmp(&self.rank[right]) {
            std::cmp::Ordering::Less => self.parent[left] = right,
            std::cmp::Ordering::Greater => self.parent[right] = left,
            std::cmp::Ordering::Equal => {
                // Retain the existing tie-breaker: the first root wins.
                self.parent[right] = left;
                self.rank[left] += 1;
            }
        }
    }
}

/// Deduplicate only after all rows have contributed their aliases.
///
/// A component with zero or one authoritative request ID is one logical call.
/// A component with several request IDs contains a reused lower-fidelity alias
/// (message ID or UUID). It must not merge those independently authoritative
/// calls. The ambiguous partial rows are retained conservatively because there
/// is no evidence assigning them to either request.
fn dedupe_usage_records(records: Vec<UsageRecord>) -> Vec<UnifiedMessage> {
    let mut aliases = HashMap::<String, usize>::new();
    let mut components = UnionFind::new(records.len());
    for (index, record) in records.iter().enumerate() {
        for alias in &record.aliases {
            if let Some(&previous) = aliases.get(alias) {
                components.union(index, previous);
            } else {
                aliases.insert(alias.clone(), index);
            }
        }
    }

    let mut grouped = HashMap::<usize, Vec<usize>>::new();
    for index in 0..records.len() {
        grouped
            .entry(components.find(index))
            .or_default()
            .push(index);
    }

    // Keep the transcript's first-observation order, independent of HashMap
    // iteration, while each merged message carries final-snapshot metadata.
    let mut selected = Vec::new();
    for indices in grouped.into_values() {
        let request_ids: HashSet<&str> = indices
            .iter()
            .filter_map(|&index| records[index].request_id.as_deref())
            .collect();
        match request_ids.len() {
            // No stable request ID is still safely dedupable when records are
            // connected by message/UUID aliases. Rows with no aliases never
            // connect and remain separate components.
            0 | 1 => selected.push((indices[0], merge_streaming_component(&records, &indices))),
            // Do not let a malformed or replayed lower-fidelity alias collapse
            // different API calls. Preserve each request and every unproven
            // partial row rather than guessing an owner.
            _ => {
                let mut request_components = HashMap::<&str, Vec<usize>>::new();
                for &index in &indices {
                    match records[index].request_id.as_deref() {
                        Some(request_id) => request_components
                            .entry(request_id)
                            .or_default()
                            .push(index),
                        // This row could replay any request in this ambiguous
                        // component, so retain it rather than guessing.
                        None => selected.push((index, records[index].message.clone())),
                    }
                }
                selected.extend(request_components.into_values().map(|request_indices| {
                    let first_index = request_indices[0];
                    (
                        first_index,
                        merge_streaming_component(&records, &request_indices),
                    )
                }));
            }
        }
    }
    selected.sort_by_key(|(first_index, _)| *first_index);
    selected.into_iter().map(|(_, message)| message).collect()
}

/// Consolidate every replay snapshot of one logical streamed call.
///
/// Streaming usage counters are cumulative snapshots, not additive deltas. A
/// final row can therefore contain a larger value in only one usage bucket.
/// Keep the maximum independently for every bucket. Metadata comes from the
/// latest valid event timestamp (then transcript order). A transcript mtime is
/// only a file-level fallback, never evidence that one timestamp-less replay is
/// newer than an event with a valid historical timestamp.
fn merge_streaming_component(records: &[UsageRecord], indices: &[usize]) -> UnifiedMessage {
    let &final_index = indices
        .iter()
        .filter(|&&index| records[index].event_timestamp.is_some())
        .max_by_key(|&&index| (records[index].event_timestamp, index))
        // When no row supplies a parseable event timestamp, retain the last
        // source row's metadata and its file-mtime fallback timestamp.
        .unwrap_or_else(|| {
            indices
                .last()
                .expect("connected component contains at least one record")
        });
    let mut merged = records[final_index].message.clone();

    for &index in indices {
        let tokens = &records[index].message.tokens;
        merged.tokens.input = merged.tokens.input.max(tokens.input);
        merged.tokens.output = merged.tokens.output.max(tokens.output);
        merged.tokens.cache_read = merged.tokens.cache_read.max(tokens.cache_read);
        merged.tokens.cache_write = merged.tokens.cache_write.max(tokens.cache_write);
        merged.tokens.cache_write_1h = merged.tokens.cache_write_1h.max(tokens.cache_write_1h);
        merged.tokens.reasoning = merged.tokens.reasoning.max(tokens.reasoning);
    }
    merged.tokens.cache_write_1h = merged.tokens.cache_write_1h.min(merged.tokens.cache_write);

    merged
}

fn provider_for_model(model: &str) -> &'static str {
    let lower = model.to_lowercase();
    if lower.contains("deepseek") {
        "deepseek"
    } else if lower.contains("claude") {
        "anthropic"
    } else if lower.contains("gpt")
        || lower.contains("o1")
        || lower.contains("o3")
        || lower.contains("o4")
        || lower.ends_with("sol")
    {
        "openai"
    } else {
        "unknown"
    }
}

/// Derive the workspace key from a transcript path by finding the
/// `.claude/projects/<slug>` window — same logic as the Claude Code parser, and
/// Cherry Studio's layout matches it exactly.
fn workspace_from_path(path: &Path) -> (Option<String>, Option<String>) {
    let components: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();
    for window in components.windows(3) {
        if window[0] == ".claude" && window[1] == "projects" {
            let key = normalize_workspace_key(&window[2]);
            let label = key.as_deref().and_then(workspace_label_from_key);
            return (key, label);
        }
    }
    (None, None)
}

/// Parse a Cherry Studio Claude Code transcript into unified messages, collapsing
/// only repeated records with a stable per-request, message, or event identity.
pub fn parse_cherrystudio_file(path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let (workspace_key, workspace_label) = workspace_from_path(path);

    let mut records = Vec::new();
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    for_each_json_line(path, &mut |_index, line| {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if record.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(message) = record.get("message").and_then(Value::as_object) else {
            return;
        };
        let Some(usage_value) = message.get("usage") else {
            return;
        };
        let Some(usage) = usage_value.as_object() else {
            return;
        };

        let input = usage
            .get("input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0);
        let output = usage
            .get("output_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0);

        let model = message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if model.is_empty() || model == "<synthetic>" || model.eq_ignore_ascii_case("unknown") {
            return;
        }

        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0);
        let cache_creation = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0);
        let cache_write_1h = super::utils::extract_cache_write_1h(usage_value).min(cache_creation);
        let total = input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_creation);
        if total <= 0 {
            return;
        }

        // Hold every valid row until the whole transcript has been read. A
        // complete snapshot can connect UUID-only, message-only, and
        // request-only rows that appeared earlier in any order.
        let request_id = record
            .get("requestId")
            .or_else(|| message.get("requestId"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        let message_id = message
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let uuid = record
            .get("uuid")
            .or_else(|| message.get("uuid"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let mut aliases = Vec::new();
        if let Some(request_id) = &request_id {
            aliases.push(format!("request:{request_id}"));
        }
        if let Some(message_id) = message_id {
            aliases.push(format!("message:{message_id}"));
        }
        if let Some(uuid) = uuid {
            aliases.push(format!("uuid:{uuid}"));
        }

        let event_timestamp = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp_str);
        let timestamp = event_timestamp.unwrap_or(fallback_timestamp);

        let tokens = TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write: cache_creation,
            cache_write_1h,
            reasoning: 0,
        };

        let provider = provider_for_model(&model);
        let mut msg = UnifiedMessage::new(
            CLIENT_ID,
            model,
            provider,
            session_id.clone(),
            timestamp,
            tokens,
            0.0,
        );
        if let (Some(key), Some(label)) = (workspace_key.clone(), workspace_label.clone()) {
            msg.set_workspace(Some(key), Some(label));
        }
        records.push(UsageRecord {
            message: msg,
            event_timestamp,
            request_id,
            aliases,
        });
    });
    dedupe_usage_records(records)
}

/// One `ai_usage_record` row, in the column order [`build_usage_query`] emits.
struct CherryChatRow {
    id: String,
    message_id: Option<String>,
    /// Conversation title, resolved through the optional `message`/`topic` join.
    session_title: Option<String>,
    provider_id: Option<String>,
    model_id: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    /// The ledger's own uncached share of the prompt, when the build has it.
    /// This is exactly the bucket `TokenBreakdown::input` models, so it is
    /// preferred over re-deriving it by subtraction.
    no_cache_tokens: Option<i64>,
    cost: Option<f64>,
    cost_currency: Option<String>,
    created_at: Option<i64>,
    message_kind: Option<String>,
}

/// Column names present on `ai_usage_record` in this database.
///
/// Cherry Studio has shipped several spellings of this table. Naming a column
/// that a given build does not have makes the whole statement fail to prepare,
/// which would silently take the lane to zero, so the query is assembled from
/// what the file actually has instead of from one assumed schema.
fn usage_table_columns(conn: &Connection) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(mut stmt) = conn.prepare("SELECT name FROM pragma_table_info('ai_usage_record')") else {
        return names;
    };
    let Ok(mut rows) = stmt.query([]) else {
        return names;
    };
    while let Ok(Some(row)) = rows.next() {
        if let Ok(name) = row.get::<_, String>(0) {
            names.push(name);
        }
    }
    names
}

/// Whether `name` exists on `table` in this database.
fn table_has_column(conn: &Connection, table: &str, name: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
    else {
        return false;
    };
    let Ok(mut rows) = stmt.query([]) else {
        return false;
    };
    while let Ok(Some(row)) = rows.next() {
        if row.get::<_, String>(0).is_ok_and(|column| column == name) {
            return true;
        }
    }
    false
}

/// Assemble the usage query for the schema this database actually has.
///
/// Returns `None` when the table is missing the columns the lane cannot work
/// without, and the SQL otherwise. Column order here is the contract the row
/// mapping in [`parse_cherrystudio_sqlite`] depends on.
fn build_usage_query(conn: &Connection) -> Option<String> {
    let columns = usage_table_columns(conn);
    let has = |name: &str| columns.iter().any(|column| column == name);
    if !has("id") || !has("created_at") {
        return None;
    }

    // Conversation identity lives on `message` (and its title on `topic`);
    // joining them is what lets a row name the conversation it belongs to.
    let link_topic = has("message_id")
        && table_has_column(conn, "message", "id")
        && table_has_column(conn, "message", "topic_id")
        && table_has_column(conn, "topic", "id")
        && table_has_column(conn, "topic", "name");
    let message_id_column = if has("message_id") {
        "r.message_id".to_string()
    } else {
        "NULL".to_string()
    };
    // Session titles came with `topic.name`; older builds have no `topic` table.
    let title_column = if link_topic {
        "t.name".to_string()
    } else {
        "NULL".to_string()
    };
    // The cache-write bucket was renamed; either spelling may be present.
    let cache_write_column = if has("cache_write_tokens") {
        "r.cache_write_tokens".to_string()
    } else if has("cache_creation_tokens") {
        "r.cache_creation_tokens".to_string()
    } else {
        "NULL".to_string()
    };
    // The ledger's own uncached-input count. Naming a column the build lacks
    // would fail the whole statement to prepare and take the lane to zero, so
    // it is probed like every other optional column and falls back to
    // subtraction in the row mapping.
    let no_cache_column = if has("no_cache_tokens") {
        "r.no_cache_tokens"
    } else {
        "NULL"
    };
    // Provider and model are the row's identity, but a build that spells them
    // differently must still degrade to "no usage" for those columns rather
    // than to no rows at all.
    let provider_column = if has("provider_id") {
        "r.provider_id"
    } else {
        "NULL"
    };
    let model_column = if has("model_id") {
        "r.model_id"
    } else {
        "NULL"
    };

    // `message_kind` separates the two surfaces. A build without it cannot
    // tell them apart, so it must report every row: the caller's dedup key
    // still keeps each invocation distinct, and guessing would be worse.
    let (kind_column, kind_filter) = if has("message_kind") {
        (
            "r.message_kind".to_string(),
            " AND (r.message_kind IS NULL OR r.message_kind = 'chat')".to_string(),
        )
    } else {
        ("NULL".to_string(), String::new())
    };

    // `legacy-aggregate` rows are pre-summed history for the same calls, so
    // reading them beside the per-invocation rows would double count.
    let record_kind_filter = if has("record_kind") {
        "r.record_kind = 'invocation'"
    } else {
        "1 = 1"
    };

    let joins = if link_topic {
        "LEFT JOIN message m ON m.id = r.message_id\n            LEFT JOIN topic t ON t.id = m.topic_id"
    } else {
        ""
    };

    Some(format!(
        "SELECT
            r.id,
            {message_id_column},
            {title_column},
            {provider_column},
            {model_column},
            {input},
            {output},
            {cache_read},
            {cache_write},
            {reasoning},
            {cost},
            r.created_at,
            {kind_column},
            {no_cache},
            {cost_currency}
        FROM ai_usage_record r
            {joins}
        WHERE {record_kind_filter}{kind_filter}
        ORDER BY r.created_at, r.id",
        message_id_column = message_id_column,
        provider_column = provider_column,
        model_column = model_column,
        no_cache = no_cache_column,
        input = if has("input_tokens") {
            "r.input_tokens"
        } else {
            "NULL"
        },
        output = if has("output_tokens") {
            "r.output_tokens"
        } else {
            "NULL"
        },
        cache_read = if has("cache_read_tokens") {
            "r.cache_read_tokens"
        } else {
            "NULL"
        },
        cache_write = cache_write_column,
        reasoning = if has("reasoning_tokens") {
            "r.reasoning_tokens"
        } else {
            "NULL"
        },
        cost = if has("cost") { "r.cost" } else { "NULL" },
        cost_currency = if has("cost_currency") {
            "r.cost_currency"
        } else {
            "NULL"
        },
    ))
}

/// Parse Cherry Studio's own usage ledger, `CherryStudio/Data/cherrystudio.sqlite`.
///
/// Cherry Studio keeps two unrelated ledgers. Its Agent / Claude Code mode
/// writes standard Claude Code transcripts, which `parse_cherrystudio_file`
/// already reads; its built-in chat surfaces (assistant conversations, and the
/// helper features that call a provider directly) instead record one row per
/// model invocation in `ai_usage_record`. Those rows never produce a
/// transcript, which is why chat usage was untracked before this parser.
///
/// Reads are strictly read-only, and the query is assembled from the columns
/// the file actually has so a build with a different schema degrades to
/// "no usage" instead of failing to prepare. Only `record_kind = 'invocation'`
/// rows are read: `legacy-aggregate` rows are pre-summed totals that would
/// otherwise double count the same calls.
///
/// # Token buckets
///
/// The ledger's `input_tokens` and `output_tokens` are inclusive: the prompt
/// count already contains both cache buckets and the completion count already
/// contains reasoning. `TokenBreakdown` expects five non-overlapping buckets,
/// so the cache reads, cache writes and reasoning tokens are moved out of
/// input/output rather than added on top of them; otherwise `total()` counts
/// every cache write and reasoning token twice. Cost comes from the ledger's
/// own `cost` column when denominated in USD (or explicitly zero in any
/// currency), and is marked provider-reported so pricing cannot re-estimate
/// it. Nonzero amounts in other or unknown currencies are left to pricing.
///
/// # Why `agent-session` rows are excluded
///
/// `message_kind` distinguishes the two surfaces, and rows tagged
/// `agent-session` are the *same billed calls* the transcript parser already
/// reports. Measured on a real profile, that tag matches the transcripts
/// almost exactly — 2026-09-11: 11 rows / 348,476 tokens in SQLite against 11
/// deduplicated calls / 348,476 tokens in the transcripts (ratio 1.000);
/// 2026-09-13: ratio 1.004; 2026-09-28: 37 calls against 35 calls. Reading
/// them here would count Agent mode twice. `chat` rows and rows predating the
/// column carry no transcript, so they are the ones this parser adds.
pub fn parse_cherrystudio_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return Vec::new();
    };
    let Some(query) = build_usage_query(&conn) else {
        // No `ai_usage_record` table, or too few columns to be this ledger.
        return Vec::new();
    };

    // A live Cherry Studio instance holds the database open and keeps recent
    // commits in the `-wal` sidecar. Reading the main file with SQLite's
    // ordinary read-only flags still replays that log, because read-only
    // forbids writing, not recovering; rows therefore cover
    // committed-but-not-checkpointed chat turns too.
    let fallback_timestamp = file_modified_timestamp_ms(db_path);
    let mut rows: Vec<CherryChatRow> = Vec::new();
    let scan = sqlite_for_each_row_on(&conn, db_path, &query, None, &mut |row| {
        let message_kind: Option<String> = row.get(12)?;
        rows.push(CherryChatRow {
            id: row.get(0)?,
            message_id: row.get(1)?,
            session_title: row.get(2)?,
            provider_id: row.get(3)?,
            model_id: row.get(4)?,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            cache_read_tokens: row.get(7)?,
            cache_write_tokens: row.get(8)?,
            reasoning_tokens: row.get(9)?,
            cost: row.get(10)?,
            created_at: row.get(11)?,
            message_kind,
            no_cache_tokens: row.get(13)?,
            cost_currency: row.get(14)?,
        });
        Ok(())
    });
    if !scan.prepared() {
        return Vec::new();
    }

    // `message_id` -> conversation title, when this schema carries the join.
    let titles: HashMap<String, Option<String>> = rows
        .iter()
        .filter_map(|row| {
            let message_id = clean_optional(row.message_id.as_deref())?;
            Some((message_id, row.session_title.clone()))
        })
        .collect();

    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        // Belt and braces: the filter runs in SQL when the column exists, but a
        // future `message_kind` spelling must not silently slip through and
        // re-count Agent mode.
        if row
            .message_kind
            .as_deref()
            .is_some_and(|kind| kind != CHAT_MESSAGE_KIND)
        {
            continue;
        }

        let model = clean_optional(row.model_id.as_deref()).unwrap_or_default();
        let provider = clean_optional(row.provider_id.as_deref())
            .unwrap_or_else(|| provider_for_model(&model).to_string());

        // The ledger records `input_tokens` as the *whole* prompt and
        // `output_tokens` as the whole completion, with the cache and
        // reasoning shares counted again in their own columns: Cherry Studio
        // derives its uncached input as
        // `inputTokens - cacheReadTokens - cacheWriteTokens` and its text
        // output as `outputTokens - reasoningTokens`
        // (`AiUsageRecordService.ts`). `TokenBreakdown` instead models five
        // non-overlapping buckets -- the same shape mismatch documented in
        // `zcode::normalize_zcode_input_and_output` -- so feeding the raw
        // columns through double counts every cache write and every reasoning
        // token in `total()`.
        let cache_read = row.cache_read_tokens.unwrap_or(0).max(0);
        let cache_write = row.cache_write_tokens.unwrap_or(0).max(0);
        let reasoning = row.reasoning_tokens.unwrap_or(0).max(0);
        // Prefer the ledger's own uncached count; otherwise subtract both cache
        // buckets, which is the arithmetic Cherry Studio itself uses.
        let input = match row.no_cache_tokens.map(|tokens| tokens.max(0)) {
            Some(no_cache) => no_cache,
            None => row
                .input_tokens
                .unwrap_or(0)
                .max(0)
                .saturating_sub(cache_read)
                .saturating_sub(cache_write)
                .max(0),
        };
        // `output_tokens` already contains the reasoning share.
        let output = row
            .output_tokens
            .unwrap_or(0)
            .max(0)
            .saturating_sub(reasoning)
            .max(0);
        let tokens = TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write,
            cache_write_1h: 0,
            reasoning,
        };
        // The ledger's nullable cost_currency has no USD default. Missing
        // columns and NULL currencies therefore cannot identify a nonzero USD
        // charge. Explicit zero is authoritative in every currency.
        let reported_cost = row.cost.filter(|cost| {
            *cost == 0.0
                || row
                    .cost_currency
                    .as_deref()
                    .is_some_and(|currency| currency.eq_ignore_ascii_case("USD"))
        });
        if tokens.total() == 0 && !reported_cost.is_some_and(|cost| cost > 0.0) {
            continue;
        }

        // The provider is the grouping key that keeps turns together in the
        // Sessions view; most rows name neither a topic nor a conversation.
        let session_id = provider.clone();
        let timestamp = row
            .created_at
            .filter(|created_at| *created_at > 0)
            .unwrap_or(fallback_timestamp);

        let cost = reported_cost.unwrap_or(0.0);
        let mut message = UnifiedMessage::new(
            CLIENT_ID, model, provider, session_id, timestamp, tokens, cost,
        );
        // Preserve reported USD charges and explicit zero through caching.
        // Nonzero amounts in other/unknown currencies must not be copied as
        // USD; leave their USD cost for tokscale's token-based pricing.
        message.cost_source = if reported_cost.is_some() {
            CostSource::ProviderReported
        } else {
            CostSource::Estimated
        };
        // The title comes from the conversation join, not from this row.
        let row_title = row
            .message_id
            .as_deref()
            .and_then(|message_id| titles.get(message_id))
            .and_then(|title| title.clone());
        message.session_title = row_title;
        // `ai_usage_record.id` is the ledger's own primary key, so it is a
        // stable identity even when `created_at` collides.
        message.dedup_key = Some(format!("cherrystudio-sqlite:{}", row.id));
        messages.push(message);
    }

    messages
}

/// Strip null/blank spellings so an empty column degrades to `None` instead of
/// becoming an empty model or provider id.
fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_transcript(dir: &std::path::Path, name: &str, lines: &[&str]) -> std::path::PathBuf {
        let path = dir
            .join(".claude")
            .join("projects")
            .join("D--repo")
            .join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        path
    }

    #[test]
    fn union_find_handles_a_long_alias_chain_without_recursion() {
        // This order reproduces a transcript where each incoming record joins
        // the prior component. The former recursive find formed a 300,000-node
        // parent chain here and overflowed the CLI stack while grouping records.
        const CHAIN_LEN: usize = 300_000;
        let mut components = UnionFind::new(CHAIN_LEN);
        for index in 1..CHAIN_LEN {
            components.union(index, index - 1);
        }

        let root = components.find(0);
        assert_eq!(components.find(CHAIN_LEN - 1), root);
        assert_eq!(components.find(CHAIN_LEN / 2), root);
    }

    #[test]
    fn dedupes_consecutive_identical_usage_signatures() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                // The same API call appended three times while streaming.
                r#"{"type":"assistant","sessionId":"s1","uuid":"a","requestId":"request-1","timestamp":"2026-04-27T13:59:02.828Z","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"cache_read_input_tokens":200,"cache_creation_input_tokens":50,"output_tokens":30}}}"#,
                r#"{"type":"assistant","sessionId":"s1","uuid":"a","requestId":"request-1","timestamp":"2026-04-27T13:59:02.900Z","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"cache_read_input_tokens":200,"cache_creation_input_tokens":50,"output_tokens":30}}}"#,
                r#"{"type":"assistant","sessionId":"s1","uuid":"a","requestId":"request-1","timestamp":"2026-04-27T13:59:03.000Z","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"cache_read_input_tokens":200,"cache_creation_input_tokens":50,"output_tokens":30}}}"#,
                // A genuinely different call.
                r#"{"type":"assistant","sessionId":"s1","uuid":"d","requestId":"request-2","timestamp":"2026-04-27T14:00:00.000Z","message":{"id":"message-2","model":"deepseek-v4-pro","usage":{"input_tokens":40,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":10}}}"#,
            ],
        );
        let messages = parse_cherrystudio_file(&path);
        assert_eq!(
            messages.len(),
            2,
            "three rows for one request/message identity collapse to one"
        );
        assert_eq!(messages[0].tokens.total(), 380);
        assert_eq!(messages[1].tokens.total(), 50);
        assert_eq!(messages[0].workspace_key.as_deref(), Some("D--repo"));
    }

    #[test]
    fn merges_streaming_component_usage_by_field_maximums() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","uuid":"early","timestamp":"2026-04-27T13:59:02.000Z","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"cache_read_input_tokens":20,"cache_creation_input_tokens":5,"output_tokens":10}}}"#,
                r#"{"type":"assistant","uuid":"final","timestamp":"2026-04-27T13:59:03.000Z","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":80,"cache_read_input_tokens":30,"cache_creation_input_tokens":2,"output_tokens":300}}}"#,
            ],
        );

        let messages = parse_cherrystudio_file(&path);
        assert_eq!(
            messages.len(),
            1,
            "snapshots of one message ID are one call"
        );
        let message = &messages[0];
        assert_eq!(message.tokens.input, 100);
        assert_eq!(message.tokens.cache_read, 30);
        assert_eq!(message.tokens.cache_write, 5);
        assert_eq!(message.tokens.output, 300);
        assert_eq!(message.timestamp, 1_777_298_343_000);
    }

    #[test]
    fn reads_the_1h_cache_write_bucket_from_a_claude_code_transcript() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","requestId":"request-1","timestamp":"2026-04-27T13:59:02.000Z","message":{"id":"message-1","model":"claude-sonnet-4","usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100000,"cache_creation":{"ephemeral_1h_input_tokens":100000}}}}"#,
            ],
        );

        let messages = parse_cherrystudio_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.cache_write, 100000);
        assert_eq!(messages[0].tokens.cache_write_1h, 100000);
    }

    #[test]
    fn keeps_valid_event_timestamp_when_later_replay_timestamp_is_invalid() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","requestId":"request-1","timestamp":"2024-01-02T03:04:05.000Z","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                // An invalid later timestamp must not turn this historical
                // call into the transcript's current file mtime.
                r#"{"type":"assistant","requestId":"request-1","timestamp":"not-a-timestamp","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":300}}}"#,
            ],
        );

        let messages = parse_cherrystudio_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, 1_704_164_645_000);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 300);
    }

    #[test]
    fn dedupes_partial_aliases_connected_by_a_complete_row_in_any_order() {
        let rows = [
            r#"{"type":"assistant","uuid":"u","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            r#"{"type":"assistant","message":{"id":"m","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            r#"{"type":"assistant","requestId":"r","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            r#"{"type":"assistant","uuid":"u","requestId":"r","message":{"id":"m","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
        ];
        let mut order = [0, 1, 2, 3];
        loop {
            let dir = tempdir().unwrap();
            let ordered_rows: Vec<_> = order.iter().map(|&index| rows[index]).collect();
            let path = write_transcript(dir.path(), "session.jsonl", &ordered_rows);
            let messages = parse_cherrystudio_file(&path);
            assert_eq!(
                messages.len(),
                1,
                "connected aliases identify one API call in order {order:?}"
            );
            assert_eq!(messages[0].tokens.total(), 110);

            // Lexicographically enumerate all 4! stream orderings, including
            // the P1's late-complete replay.
            let Some(pivot) = (0..order.len() - 1)
                .rev()
                .find(|&index| order[index] < order[index + 1])
            else {
                break;
            };
            let swap = (pivot + 1..order.len())
                .rev()
                .find(|&index| order[pivot] < order[index])
                .unwrap();
            order.swap(pivot, swap);
            order[pivot + 1..].reverse();
        }
    }

    #[test]
    fn dedupes_replays_with_changed_uuids_and_same_primary_ids() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","uuid":"event-1","requestId":"request-1","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","uuid":"event-2","requestId":"request-1","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","uuid":"event-3","requestId":"request-1","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        let messages = parse_cherrystudio_file(&path);
        assert_eq!(
            messages.len(),
            1,
            "replays must collapse even when each record has a different UUID"
        );
        assert_eq!(messages[0].tokens.total(), 110);
    }

    #[test]
    fn keeps_distinct_primary_ids_with_identical_usage() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","uuid":"event-1","requestId":"request-1","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","uuid":"event-1","requestId":"request-2","message":{"id":"message-2","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        let messages = parse_cherrystudio_file(&path);
        assert_eq!(
            messages.len(),
            2,
            "distinct primary IDs must count even when UUID and usage match"
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.total())
                .sum::<i64>(),
            220
        );
    }

    #[test]
    fn dedupes_request_only_record_when_later_record_has_message_id() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                // Reviewer repro: a streaming row gains message.id later.
                r#"{"type":"assistant","uuid":"stream-early","requestId":"request-1","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","uuid":"stream-late","requestId":"request-1","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        assert_eq!(parse_cherrystudio_file(&path).len(), 1);
    }

    #[test]
    fn dedupes_message_only_record_when_later_record_has_request_id() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                // The inverse transition must also collapse, even though the
                // replay's event UUID differs.
                r#"{"type":"assistant","uuid":"stream-early","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","uuid":"stream-late","requestId":"request-1","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        assert_eq!(parse_cherrystudio_file(&path).len(), 1);
    }

    #[test]
    fn request_id_defines_one_call_even_when_message_id_changes() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                // Cherry Studio's requestId is its API-call ID; message.id is
                // response metadata populated as the stream evolves. A changed
                // message ID under the same request is therefore a replay, not
                // a second billed request.
                r#"{"type":"assistant","requestId":"request-1","message":{"id":"message-early","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","requestId":"request-1","message":{"id":"message-late","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        assert_eq!(parse_cherrystudio_file(&path).len(), 1);
    }

    #[test]
    fn keeps_distinct_requests_when_message_id_is_reused() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                // A malformed/replayed message ID must not override a distinct
                // API request. Each request still represents one billable call.
                r#"{"type":"assistant","requestId":"request-1","message":{"id":"message-shared","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","requestId":"request-2","message":{"id":"message-shared","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","uuid":"replay-with-new-uuid","requestId":"request-2","message":{"id":"message-shared","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        assert_eq!(
            parse_cherrystudio_file(&path).len(),
            2,
            "different request IDs stay distinct; the request-2 replay collapses"
        );
    }

    #[test]
    fn keeps_sparse_message_after_distinct_requests_reuse_its_id() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                // The two request IDs prove these are distinct calls, making
                // their shared lower-fidelity alias ambiguous.
                r#"{"type":"assistant","requestId":"request-1","message":{"id":"message-shared","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","requestId":"request-2","message":{"id":"message-shared","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                // Without a request ID, this could replay either call. Keep it
                // rather than silently discarding a potentially genuine call.
                r#"{"type":"assistant","message":{"id":"message-shared","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        assert_eq!(parse_cherrystudio_file(&path).len(), 3);
    }

    #[test]
    fn dedupes_uuid_to_complete_identity_transition() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","uuid":"stable-event","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","uuid":"stable-event","requestId":"request-1","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                // UUID changes after request/message aliases were learned.
                r#"{"type":"assistant","uuid":"replayed-event","requestId":"request-1","message":{"id":"message-1","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        assert_eq!(parse_cherrystudio_file(&path).len(), 1);
    }

    #[test]
    fn keeps_consecutive_no_id_rows_with_identical_usage() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );

        let messages = parse_cherrystudio_file(&path);
        assert_eq!(
            messages.len(),
            2,
            "rows without an identity are retained conservatively"
        );
    }

    #[test]
    #[ignore]
    fn real_transcripts_dedup_count() {
        let appdata = std::env::var("APPDATA").expect("APPDATA set");
        let base = std::path::Path::new(&appdata)
            .join("CherryStudio")
            .join(".claude")
            .join("projects");
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                let dir = entry.path();
                if dir.is_dir() {
                    if let Ok(items) = std::fs::read_dir(&dir) {
                        for item in items.flatten() {
                            if item.path().extension().and_then(|e| e.to_str()) == Some("jsonl") {
                                files.push(item.path());
                            }
                        }
                    }
                }
            }
        }
        files.sort();
        let mut total_messages = 0usize;
        let mut total_tokens = 0i64;
        let mut by_model: std::collections::HashMap<String, (usize, i64)> = Default::default();
        for path in &files {
            for msg in parse_cherrystudio_file(path) {
                total_messages += 1;
                total_tokens += msg.tokens.total();
                let e = by_model.entry(msg.model_id.clone()).or_default();
                e.0 += 1;
                e.1 += msg.tokens.total();
            }
        }
        println!("真实转录文件数: {}", files.len());
        println!("去重后总消息数: {}", total_messages);
        println!("去重后总 token: {}", total_tokens);
        let mut models: Vec<_> = by_model.into_iter().collect();
        models.sort_by_key(|a| std::cmp::Reverse(a.1 .1));
        for (m, (c, t)) in models {
            println!("  {m:<24} msgs={c:>6}  tokens={t:>14}");
        }
        assert!(total_messages > 0);
    }

    #[test]
    fn keeps_non_consecutive_same_usage_as_separate_calls() {
        let dir = tempdir().unwrap();
        let path = write_transcript(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","sessionId":"s1","uuid":"a","timestamp":"2026-04-27T13:59:02.828Z","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
                r#"{"type":"assistant","sessionId":"s1","uuid":"b","timestamp":"2026-04-27T13:59:05.000Z","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":200,"output_tokens":20}}}"#,
                r#"{"type":"assistant","sessionId":"s1","uuid":"c","timestamp":"2026-04-27T14:00:00.000Z","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            ],
        );
        let messages = parse_cherrystudio_file(&path);
        // The third row has the same signature as the first, but is not
        // consecutive, so it is a distinct call and must be kept.
        assert_eq!(messages.len(), 3);
    }

    /// An `ai_usage_record` row: id, provider, model, input, output, and the
    /// optional `message_kind` tag.
    type UsageRow<'a> = (&'a str, &'a str, &'a str, i64, i64, Option<&'a str>);

    /// Build a database shaped like a real Cherry Studio install: the ledger
    /// plus the `message`/`topic` tables its conversation join needs.
    ///
    /// Every row is written with `record_kind = 'invocation'`; the tests that
    /// care about aggregates flip one afterwards.
    fn write_usage_db(dir: &std::path::Path, rows: &[UsageRow<'_>]) -> std::path::PathBuf {
        let path = dir.join("cherrystudio.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, topic_id TEXT);
             CREATE TABLE topic (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE ai_usage_record (
                 id TEXT PRIMARY KEY,
                 record_kind TEXT,
                 message_kind TEXT,
                 message_id TEXT,
                 provider_id TEXT,
                 model_id TEXT,
                 input_tokens INTEGER,
                 output_tokens INTEGER,
                 cache_read_tokens INTEGER,
                 cache_write_tokens INTEGER,
                 reasoning_tokens INTEGER,
                 cost REAL,
                 cost_currency TEXT,
                 created_at INTEGER
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO topic (id, name) VALUES ('topic-1', 'A conversation')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, topic_id) VALUES ('message-1', 'topic-1')",
            [],
        )
        .unwrap();
        for (index, (id, provider, model, input, output, kind)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO ai_usage_record (id, record_kind, message_kind, message_id, provider_id, model_id,
                     input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens, cost, cost_currency, created_at)
                 VALUES (?1, 'invocation', ?2, 'message-1', ?3, ?4, ?5, ?6, 0, 0, 0, ?7, 'USD', ?8)",
                rusqlite::params![
                    id,
                    kind,
                    provider,
                    model,
                    input,
                    output,
                    0.01_f64,
                    1_780_000_000_000_i64 + index as i64
                ],
            )
            .unwrap();
        }
        drop(conn);
        path
    }

    #[test]
    fn sqlite_lane_reads_chat_rows_and_skips_agent_session_rows() {
        // Agent-mode calls also land in `ai_usage_record` as `agent-session`,
        // and the transcript parser already reports those. Counting them here
        // would report Agent mode twice, so the SQLite lane must skip them.
        let dir = tempdir().unwrap();
        let path = write_usage_db(
            dir.path(),
            &[
                (
                    "chat-1",
                    "deepseek",
                    "deepseek-flash",
                    1000,
                    200,
                    Some("chat"),
                ),
                (
                    "agent-1",
                    "deepseek",
                    "deepseek-flash",
                    5000,
                    500,
                    Some("agent-session"),
                ),
                ("legacy-1", "cherryai", "qwen", 10, 5, Some("chat")),
            ],
        );
        // `legacy-aggregate` rows are pre-summed history for the same calls, so
        // reading them beside the invocations would double count.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE ai_usage_record SET record_kind = 'legacy-aggregate' WHERE id = 'legacy-1'",
            [],
        )
        .unwrap();
        drop(conn);

        let messages = parse_cherrystudio_sqlite(&path);
        assert_eq!(
            messages.len(),
            1,
            "only the chat invocation is this lane's to report"
        );
        assert_eq!(messages[0].client, CLIENT_ID);
        assert_eq!(messages[0].model_id, "deepseek-flash");
        assert_eq!(messages[0].provider_id, "deepseek");
        assert_eq!(messages[0].session_id, "deepseek");
        assert_eq!(
            messages[0].session_title.as_deref(),
            Some("A conversation"),
            "the conversation join supplies the title"
        );
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[0].tokens.output, 200);
        assert_eq!(
            messages[0].cost_source,
            CostSource::ProviderReported,
            "a provider-reported cost must not be re-estimated"
        );
    }

    #[test]
    fn sqlite_lane_moves_cache_and_reasoning_shares_out_of_the_inclusive_totals() {
        // The ledger's `input_tokens` is the whole prompt (cached part
        // included) and `output_tokens` is the whole completion (reasoning
        // included). `TokenBreakdown` has five non-overlapping buckets, so
        // both shares must leave input/output instead of being added beside
        // them -- otherwise `total()` counts every cache write and reasoning
        // token twice.
        let dir = tempdir().unwrap();
        let path = dir.path().join("cherrystudio.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_usage_record (
                 id TEXT PRIMARY KEY, record_kind TEXT, message_kind TEXT, message_id TEXT,
                 provider_id TEXT, model_id TEXT, input_tokens INTEGER, output_tokens INTEGER,
                 cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER,
                 cost REAL, created_at INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ai_usage_record VALUES
                ('r1','invocation','chat',NULL,'deepseek','deepseek-flash',1000,200,800,50,25,0.01,1780000000000)",
            [],
        )
        .unwrap();
        drop(conn);

        let messages = parse_cherrystudio_sqlite(&path);
        assert_eq!(messages.len(), 1);
        let tokens = &messages[0].tokens;
        // Both the cache-read and the cache-write share leave the input bucket.
        assert_eq!(tokens.input, 150);
        // The reasoning share leaves the output bucket.
        assert_eq!(tokens.output, 175);
        assert_eq!(tokens.cache_read, 800);
        assert_eq!(tokens.cache_write, 50);
        assert_eq!(tokens.reasoning, 25);
        // The invariant that catches the double counting: the buckets must
        // still add up to the ledger's own `input_tokens + output_tokens`.
        assert_eq!(tokens.total(), 1000 + 200);
    }

    #[test]
    fn sqlite_lane_prefers_the_ledgers_own_no_cache_tokens() {
        // When the build carries `no_cache_tokens`, that column is the ledger
        // stating the uncached share outright, so it wins over re-deriving it.
        let dir = tempdir().unwrap();
        let path = dir.path().join("cherrystudio.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_usage_record (
                 id TEXT PRIMARY KEY, record_kind TEXT, message_kind TEXT, message_id TEXT,
                 provider_id TEXT, model_id TEXT, input_tokens INTEGER, output_tokens INTEGER,
                 total_tokens INTEGER, no_cache_tokens INTEGER, cache_read_tokens INTEGER,
                 cache_write_tokens INTEGER, reasoning_tokens INTEGER,
                 cost REAL, created_at INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ai_usage_record VALUES
                ('r1','invocation','chat',NULL,'deepseek','deepseek-flash',
                 10000,1000,11100,2100,7500,500,400,0.01,1780000000000)",
            [],
        )
        .unwrap();
        drop(conn);

        let messages = parse_cherrystudio_sqlite(&path);
        assert_eq!(messages.len(), 1);
        let tokens = &messages[0].tokens;
        // Deliberately differs from 10000 - 7500 - 500 = 2000, so deriving
        // the bucket instead of honoring the explicit value fails this test.
        assert_eq!(tokens.input, 2100, "no_cache_tokens is used verbatim");
        assert_eq!(tokens.output, 600, "reasoning leaves the output bucket");
        assert_eq!(tokens.total(), 11100, "buckets sum to total_tokens");
    }

    #[test]
    fn sqlite_lane_keeps_an_explicit_zero_cost() {
        // A zero in `cost` is observed data ("explicit zero-cost rows remain
        // priced"), not a missing value: re-estimating it would invent a
        // charge for a free or local model. Only a NULL cost is tokscale's to
        // price.
        let dir = tempdir().unwrap();
        let path = dir.path().join("cherrystudio.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_usage_record (
                 id TEXT PRIMARY KEY, record_kind TEXT, message_kind TEXT, message_id TEXT,
                 provider_id TEXT, model_id TEXT, input_tokens INTEGER, output_tokens INTEGER,
                 cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER,
                 cost REAL, created_at INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ai_usage_record VALUES
                ('free','invocation','chat',NULL,'ollama','llama3',100,10,0,0,0,0.0,1780000000000),
                ('unpriced','invocation','chat',NULL,'deepseek','deepseek-flash',100,10,0,0,0,NULL,1780000000000)",
            [],
        )
        .unwrap();
        drop(conn);

        let messages = parse_cherrystudio_sqlite(&path);
        assert_eq!(messages.len(), 2);
        let free = messages
            .iter()
            .find(|message| message.dedup_key.as_deref() == Some("cherrystudio-sqlite:free"))
            .expect("the zero-cost row is reported");
        let unpriced = messages
            .iter()
            .find(|message| message.dedup_key.as_deref() == Some("cherrystudio-sqlite:unpriced"))
            .expect("the NULL-cost row is reported");
        // An explicit zero cost is authoritative, not a missing value.
        assert_eq!(free.cost_source, CostSource::ProviderReported);
        assert_eq!(free.cost, 0.0);
        // A NULL cost is left for tokscale's own pricing to estimate.
        assert_eq!(unpriced.cost_source, CostSource::Estimated);
    }

    #[test]
    fn sqlite_lane_falls_back_to_the_file_mtime_when_created_at_is_unusable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cherrystudio.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_usage_record (
                 id TEXT PRIMARY KEY, record_kind TEXT, message_kind TEXT, message_id TEXT,
                 provider_id TEXT, model_id TEXT, input_tokens INTEGER, output_tokens INTEGER,
                 cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER,
                 cost REAL, created_at INTEGER);",
        )
        .unwrap();
        // A zero timestamp is the ledger's "no usable time" sentinel.
        conn.execute(
            "INSERT INTO ai_usage_record VALUES
                ('r1','invocation','chat',NULL,'deepseek','deepseek-flash',100,10,0,0,0,0,0)",
            [],
        )
        .unwrap();
        drop(conn);

        let messages = parse_cherrystudio_sqlite(&path);
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].timestamp > 0,
            "an unusable created_at must fall back to the file mtime, not epoch zero"
        );
        // With no topic the provider doubles as the session grouping key.
        assert_eq!(messages[0].session_id, "deepseek");
    }

    #[test]
    fn sqlite_lane_tolerates_a_schema_without_message_kind() {
        // Pre-`message_kind` builds recorded invocations without naming the
        // surface. The lane must still read them instead of failing to prepare.
        let dir = tempdir().unwrap();
        let path = dir.path().join("cherrystudio.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_usage_record (
                 id TEXT PRIMARY KEY, record_kind TEXT, message_id TEXT,
                 provider_id TEXT, model_id TEXT, input_tokens INTEGER, output_tokens INTEGER,
                 cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER,
                 cost REAL, created_at INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ai_usage_record VALUES
                ('r1','invocation',NULL,'deepseek','deepseek-flash',100,10,0,0,0,0,1780000000000)",
            [],
        )
        .unwrap();
        drop(conn);

        let messages = parse_cherrystudio_sqlite(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
    }

    #[test]
    fn sqlite_lane_reports_nothing_for_a_database_without_the_usage_table() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cherrystudio.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE unrelated (id TEXT);")
            .unwrap();
        drop(conn);

        assert!(parse_cherrystudio_sqlite(&path).is_empty());
    }

    #[test]
    fn sqlite_lane_only_treats_usd_or_explicit_zero_as_authoritative() {
        for currency in [Some("USD"), Some("usd"), Some("CNY"), None] {
            for cost in [Some(7.0), Some(0.0), None] {
                let dir = tempdir().unwrap();
                let path = write_usage_db(
                    dir.path(),
                    &[("r1", "deepseek", "deepseek-chat", 100, 10, Some("chat"))],
                );
                let conn = Connection::open(&path).unwrap();
                conn.execute(
                    "UPDATE ai_usage_record SET cost = ?1, cost_currency = ?2",
                    rusqlite::params![cost, currency],
                )
                .unwrap();
                drop(conn);

                let messages = parse_cherrystudio_sqlite(&path);
                assert_eq!(messages.len(), 1);
                let authoritative = cost.is_some()
                    && (cost == Some(0.0)
                        || currency.is_some_and(|value| value.eq_ignore_ascii_case("USD")));
                assert_eq!(
                    messages[0].cost_source,
                    if authoritative {
                        CostSource::ProviderReported
                    } else {
                        CostSource::Estimated
                    },
                    "cost={cost:?}, currency={currency:?}"
                );
                assert_eq!(
                    messages[0].cost,
                    if authoritative { cost.unwrap() } else { 0.0 }
                );
                assert_eq!(messages[0].tokens.total(), 110);
            }
        }
    }

    #[test]
    fn sqlite_lane_missing_currency_does_not_default_nonzero_cost_to_usd() {
        for cost in [7.0, 0.0] {
            let dir = tempdir().unwrap();
            let path = write_usage_db(
                dir.path(),
                &[("r1", "deepseek", "deepseek-chat", 100, 10, Some("chat"))],
            );
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("ALTER TABLE ai_usage_record DROP COLUMN cost_currency;")
                .unwrap();
            conn.execute("UPDATE ai_usage_record SET cost = ?1", [cost])
                .unwrap();
            drop(conn);

            let messages = parse_cherrystudio_sqlite(&path);
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].cost, 0.0);
            assert_eq!(
                messages[0].cost_source,
                if cost == 0.0 {
                    CostSource::ProviderReported
                } else {
                    CostSource::Estimated
                }
            );
        }
    }

    #[test]
    fn sqlite_lane_clamps_derived_buckets_for_partial_or_inconsistent_usage() {
        for (input, output) in [(None, None), (Some(10), Some(10)), (Some(0), Some(0))] {
            let dir = tempdir().unwrap();
            let path = write_usage_db(
                dir.path(),
                &[("r1", "deepseek", "deepseek-chat", 100, 10, Some("chat"))],
            );
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "UPDATE ai_usage_record SET input_tokens = ?1, output_tokens = ?2,
                    cache_read_tokens = 80, cache_write_tokens = 30, reasoning_tokens = 20",
                rusqlite::params![input, output],
            )
            .unwrap();
            drop(conn);

            let messages = parse_cherrystudio_sqlite(&path);
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].tokens.input, 0);
            assert_eq!(messages[0].tokens.output, 0);
            assert_eq!(messages[0].tokens.cache_read, 80);
            assert_eq!(messages[0].tokens.cache_write, 30);
            assert_eq!(messages[0].tokens.reasoning, 20);
            assert_eq!(messages[0].tokens.total(), 130);
        }
    }

    #[test]
    fn sqlite_lane_keeps_cost_only_calls_with_authoritative_charges() {
        for currency in [Some("USD"), Some("usd"), Some("CNY"), None] {
            for cost in [Some(0.25), Some(0.0), None] {
                let dir = tempdir().unwrap();
                let path = write_usage_db(
                    dir.path(),
                    &[("r1", "deepseek", "deepseek-chat", 100, 10, Some("chat"))],
                );
                let conn = Connection::open(&path).unwrap();
                conn.execute(
                    "UPDATE ai_usage_record SET input_tokens = NULL, output_tokens = NULL,
                        cost = ?1, cost_currency = ?2",
                    rusqlite::params![cost, currency],
                )
                .unwrap();
                drop(conn);

                let messages = parse_cherrystudio_sqlite(&path);
                let should_keep = cost == Some(0.25)
                    && currency.is_some_and(|value| value.eq_ignore_ascii_case("USD"));
                assert_eq!(messages.len(), usize::from(should_keep));
                if should_keep {
                    assert_eq!(messages[0].cost, 0.25);
                    assert_eq!(messages[0].cost_source, CostSource::ProviderReported);
                    assert_eq!(messages[0].tokens.total(), 0);
                    assert_eq!(messages[0].message_count, 1);
                }
            }
        }
    }

    #[test]
    fn sqlite_lane_keeps_usage_when_optional_title_join_dependencies_are_missing() {
        for schema_change in [
            "DROP TABLE topic",
            "DROP TABLE message",
            "ALTER TABLE topic DROP COLUMN name",
            "ALTER TABLE message DROP COLUMN topic_id",
            "ALTER TABLE topic RENAME COLUMN id TO other_id",
            "ALTER TABLE message RENAME COLUMN id TO other_id",
        ] {
            let dir = tempdir().unwrap();
            let path = write_usage_db(
                dir.path(),
                &[("r1", "deepseek", "deepseek-chat", 100, 10, Some("chat"))],
            );
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(schema_change).unwrap();
            drop(conn);

            let messages = parse_cherrystudio_sqlite(&path);
            assert_eq!(messages.len(), 1, "{schema_change}");
            assert_eq!(messages[0].tokens.total(), 110);
            assert_eq!(messages[0].session_title, None);
        }
    }
}
