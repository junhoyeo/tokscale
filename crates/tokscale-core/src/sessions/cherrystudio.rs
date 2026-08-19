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

use super::utils::{file_modified_timestamp_ms, for_each_json_line, parse_timestamp_str};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const CLIENT_ID: &str = "cherrystudio";

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
        merged.tokens.reasoning = merged.tokens.reasoning.max(tokens.reasoning);
    }

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
        let Some(usage) = message.get("usage").and_then(Value::as_object) else {
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
        models.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
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
}
