//! Claude Desktop: local Code/Cowork transcripts and the Chat/Cowork/Code
//! conversation cache. Model identifiers are data, never an allowlist.
//!
//! A truncated tree is not an account history. Cumulative modelUsage counters
//! are kept as undated lower bounds, net of observed requests. Never assign an
//! old cumulative balance to the cache write time or submit it as daily usage.
mod indexeddb;
mod manifest;
#[allow(dead_code)]
mod v8;

use crate::{canonical_model_id, sessions::UnifiedMessage, TokenBreakdown};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const UNKNOWN_DATE: &str = "unknown";

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub cache_databases: usize,
    pub conversations: usize,
    pub products: BTreeMap<String, usize>,
    pub truncated_conversations: usize,
    pub decode_errors: usize,
    pub counter_resets: usize,
    pub messages_without_usage: usize,
    pub warnings: Vec<String>,
}

pub fn storage_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("Library/Application Support/Claude"),
        home.join("AppData/Roaming/Claude"),
        home.join(".config/Claude"),
    ]
}

/// Scan every retained session, with no date cutoff. Only actual transcript
/// roots are included: audit.jsonl is another copy of the same API traffic.
pub fn transcript_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for app in storage_roots(home) {
        for name in ["local-agent-mode-sessions", "claude-code-sessions"] {
            let root = app.join(name);
            let mut walk = walkdir::WalkDir::new(root).follow_links(false).into_iter();
            while let Some(entry) = walk.next() {
                let Ok(entry) = entry else { continue };
                if entry.file_type().is_dir() && entry.file_name() == ".claude" {
                    for name in ["projects", "transcripts"] {
                        let path = entry.path().join(name);
                        if path.is_dir() {
                            roots.push(path);
                        }
                    }
                    walk.skip_current_dir();
                }
            }
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

#[derive(Default)]
struct Conversation {
    messages: Vec<UnifiedMessage>,
    session_ids: HashSet<String>,
    totals: BTreeMap<String, TokenBreakdown>,
    // Last counter observation. Requests after it are additional usage.
    last_snapshot: i64,
    session: String,
}

pub struct DesktopUsage {
    pub coverage: Coverage,
    conversations: Vec<Conversation>,
}

fn identity(value: &Value) -> Option<String> {
    // Length-separated JSON tuples avoid collisions between accounts/products.
    serde_json::to_string(&[
        value.get("accountUuid")?.as_str()?,
        value.get("orgUuid")?.as_str()?,
        value.get("conversationUuid")?.as_str()?,
    ])
    .ok()
}

pub fn read(home: &Path) -> DesktopUsage {
    let mut coverage = Coverage::default();
    let mut trees: BTreeMap<String, Value> = BTreeMap::new();
    let mut deleted = HashSet::new();
    for root in storage_roots(home) {
        let path = root.join("IndexedDB/https_claude.ai_0.indexeddb.leveldb");
        if !path.is_dir() {
            continue;
        }
        coverage.cache_databases += 1;
        match indexeddb::read(&path) {
            Ok((values, errors)) => {
                coverage.decode_errors += errors.len();
                for error in errors {
                    if !coverage.warnings.contains(&error) {
                        coverage.warnings.push(error);
                    }
                }
                for record in values {
                    let Some(key) = identity(&record.value) else {
                        coverage.decode_errors += 1;
                        continue;
                    };
                    if record.store == "meta" {
                        if record.value.get("tombstone").and_then(Value::as_bool) == Some(true)
                            || record.value.get("deletedAt").is_some_and(|v| !v.is_null())
                        {
                            deleted.insert(key);
                        }
                    } else if trees.get(&key).is_none_or(|old| {
                        timestamp(&record.value["writtenAt"]) >= timestamp(&old["writtenAt"])
                    }) {
                        trees.insert(key, record.value);
                    }
                }
            }
            Err(error) => {
                coverage.decode_errors += 1;
                coverage.warnings.push(error);
            }
        }
    }
    let mut conversations = Vec::new();
    for (key, value) in trees {
        if deleted.contains(&key) {
            continue;
        }
        coverage.conversations += 1;
        *coverage
            .products
            .entry(value["product"].as_str().unwrap_or("unknown").to_string())
            .or_default() += 1;
        let tree = &value["tree"];
        if tree["hasOlder"].as_bool() == Some(true)
            || tree
                .get("earlierCursor")
                .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
        {
            coverage.truncated_conversations += 1;
        }
        conversations.push(parse_tree(&value, &mut coverage));
    }
    DesktopUsage {
        coverage,
        conversations,
    }
}

fn token_count(value: &Value) -> i64 {
    // JS stores large integral counts as doubles. Reject fractional/negative
    // values, rather than silently truncating a malformed counter.
    value
        .as_i64()
        .filter(|n| *n >= 0)
        .or_else(|| {
            value
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n < i64::MAX as f64)
                .map(|n| n as i64)
        })
        .unwrap_or(0)
}

fn tokens(value: &Value, cumulative: bool) -> TokenBreakdown {
    let fields = if cumulative {
        [
            "inputTokens",
            "outputTokens",
            "cacheReadInputTokens",
            "cacheCreationInputTokens",
        ]
    } else {
        [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
    };
    TokenBreakdown {
        input: token_count(&value[fields[0]]),
        output: token_count(&value[fields[1]]),
        cache_read: token_count(&value[fields[2]]),
        cache_write: token_count(&value[fields[3]]),
        cache_write_1h: if cumulative {
            0 // SDK cumulative counters do not report the cache TTL split.
        } else {
            token_count(&value["cache_creation"]["ephemeral_1h_input_tokens"])
                .min(token_count(&value[fields[3]]))
        },
        reasoning: 0,
    }
    // Anthropic thinking is included in outputTokens; never add it twice.
}

fn max_tokens(dst: &mut TokenBreakdown, src: &TokenBreakdown) {
    dst.input = dst.input.max(src.input);
    dst.output = dst.output.max(src.output);
    dst.cache_read = dst.cache_read.max(src.cache_read);
    dst.cache_write = dst.cache_write.max(src.cache_write);
    dst.cache_write_1h = dst
        .cache_write_1h
        .max(src.cache_write_1h)
        .min(dst.cache_write);
}

fn subtract(total: &TokenBreakdown, observed: &TokenBreakdown) -> TokenBreakdown {
    TokenBreakdown {
        input: total.input.saturating_sub(observed.input).max(0),
        output: total.output.saturating_sub(observed.output).max(0),
        cache_read: total.cache_read.saturating_sub(observed.cache_read).max(0),
        cache_write: total
            .cache_write
            .saturating_sub(observed.cache_write)
            .max(0),
        cache_write_1h: total
            .cache_write_1h
            .saturating_sub(observed.cache_write_1h)
            .max(0),
        reasoning: 0,
    }
}

fn timestamp(value: &Value) -> i64 {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or_else(|| token_count(value))
}

fn model_id(raw: &str) -> String {
    // Claude Code SDK annotates the context window on cumulative counters.
    canonical_model_id(raw.strip_suffix("[1m]").unwrap_or(raw))
}

fn real_model(raw: &str) -> bool {
    !raw.trim().is_empty() && raw != "<synthetic>" && raw != "unknown"
}

fn parse_tree(value: &Value, coverage: &mut Coverage) -> Conversation {
    let tree = &value["tree"];
    let mut conversation = Conversation {
        session: value["conversationUuid"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        ..Default::default()
    };
    let mut previous: BTreeMap<String, TokenBreakdown> = BTreeMap::new();
    let mut messages = HashMap::new();
    for collection in ["events", "messages", "chat_messages"] {
        for event in tree[collection].as_array().into_iter().flatten() {
            let payload = event.get("payload").unwrap_or(event);
            let ts = [
                payload.get("timestamp"),
                event.get("created_at"),
                event.get("serverCreatedAt"),
                payload.get("created_at"),
            ]
            .into_iter()
            .flatten()
            .map(timestamp)
            .find(|n| *n > 0)
            .unwrap_or(0);
            let kind = payload["type"]
                .as_str()
                .or_else(|| payload["role"].as_str())
                .or_else(|| payload["sender"].as_str())
                .unwrap_or_default();
            if kind == "result" {
                if let Some(usage) = payload["modelUsage"].as_object() {
                    if !usage.is_empty() {
                        conversation.last_snapshot = conversation.last_snapshot.max(ts);
                        if let Some(id) = payload["session_id"].as_str() {
                            conversation.session_ids.insert(id.to_string());
                            conversation.session = id.to_string();
                        }
                    }
                    // Sum context-window variants WITHIN one snapshot; merge
                    // successive cumulative snapshots with max, never sum.
                    let mut snapshot: BTreeMap<String, TokenBreakdown> = BTreeMap::new();
                    for (model, usage) in usage {
                        if real_model(model) {
                            snapshot
                                .entry(model_id(model))
                                .or_default()
                                .add_assign_saturating(&tokens(usage, true));
                        }
                    }
                    for (model, current) in snapshot {
                        if previous.get(&model).is_some_and(|old| {
                            current.input < old.input
                                || current.output < old.output
                                || current.cache_read < old.cache_read
                                || current.cache_write < old.cache_write
                        }) {
                            coverage.counter_resets += 1;
                        }
                        previous.insert(model.clone(), current.clone());
                        max_tokens(conversation.totals.entry(model).or_default(), &current);
                    }
                }
                continue;
            }
            if kind != "assistant" {
                continue;
            }
            let message = payload.get("message").unwrap_or(payload);
            let Some(model) = message["model"].as_str().filter(|s| real_model(s)) else {
                coverage.messages_without_usage += 1;
                continue;
            };
            let Some(usage) = message.get("usage").filter(|v| v.is_object()) else {
                coverage.messages_without_usage += 1;
                continue;
            };
            let Some(id) = message["id"]
                .as_str()
                .or_else(|| message["uuid"].as_str())
                .filter(|id| !id.is_empty())
            else {
                coverage.messages_without_usage += 1;
                continue;
            };
            let sid = payload["session_id"]
                .as_str()
                .or_else(|| payload["sessionId"].as_str())
                .unwrap_or(&conversation.session);
            let request = payload["request_id"]
                .as_str()
                .or_else(|| payload["requestId"].as_str());
            let key = request
                .map(|request| format!("{id}:{request}"))
                .unwrap_or_else(|| format!("message:{id}"));
            let mut msg = UnifiedMessage::new_with_dedup(
                "claude",
                model_id(model),
                "anthropic",
                sid,
                ts,
                tokens(usage, false),
                0.0,
                Some(key),
            );
            if ts == 0 {
                msg.date = UNKNOWN_DATE.into();
            }
            msg.agent = Some(format!(
                "Claude Desktop {}",
                value["product"].as_str().unwrap_or("chat")
            ));
            // Reuse the transcript parser's advisor identities and accounting:
            // advisor iterations add tokens, but never another turn or duration.
            let advisor_usage = serde_json::from_value::<crate::sessions::claudecode::ClaudeUsage>(
                serde_json::json!({"iterations": usage.get("iterations")}),
            )
            .expect("optional iterations alone are a valid usage block");
            let advisors =
                crate::sessions::claudecode::claude_advisor_messages(&advisor_usage, &msg);
            for mut msg in std::iter::once(msg).chain(advisors.into_iter().map(|(msg, _)| msg)) {
                if msg.timestamp == 0 {
                    msg.date = UNKNOWN_DATE.into();
                }
                let Some(key) = message_id(&msg) else {
                    continue;
                };
                if let Some(old) = messages.get_mut(&key) {
                    merge_message(old, &msg);
                } else {
                    messages.insert(key, msg);
                }
            }
        }
    }
    conversation.messages = messages.into_values().collect();
    conversation
}

fn message_id(message: &UnifiedMessage) -> Option<String> {
    let key = message.dedup_key.as_deref()?;
    if !crate::sessions::claudecode::dedup_key_is_globally_stable(key)
        || key.starts_with("claude-desktop-total:")
    {
        return None;
    }
    // Match the transcript parser's full identity, including the request ID
    // and advisor index. A message-only fallback cannot prove a request match.
    Some(key.to_string())
}

fn merge_message(existing: &mut UnifiedMessage, incoming: &UnifiedMessage) {
    // A real message id must not acquire a different model silently.
    if model_id(&existing.model_id) != model_id(&incoming.model_id) {
        existing.model_attribution_conflicted = true;
    }
    crate::sessions::claudecode::merge_message_completeness(existing, incoming);
    if existing.timestamp <= 0 && incoming.timestamp > 0 {
        existing.timestamp = incoming.timestamp;
        existing.date.clone_from(&incoming.date);
    }
}

impl DesktopUsage {
    /// Merge requests first, then add only the missing cumulative balance.
    /// `existing` contains deduplicated CLI/local Desktop transcripts, so a
    /// cloud copy never adds another copy of the same API response.
    pub fn merge_into(self, existing: &mut Vec<UnifiedMessage>) {
        let mut ids: HashMap<String, usize> = existing
            .iter()
            .enumerate()
            .filter_map(|(i, m)| message_id(m).map(|id| (id, i)))
            .collect();
        let mut summaries: BTreeMap<String, Conversation> = BTreeMap::new();
        for mut conversation in self.conversations {
            let mut overlap_sessions = HashSet::new();
            for msg in conversation.messages.drain(..) {
                let Some(id) = message_id(&msg) else {
                    continue;
                };
                if let Some(index) = ids.get(&id).copied() {
                    overlap_sessions.insert(existing[index].session_id.clone());
                    merge_message(&mut existing[index], &msg);
                } else {
                    ids.insert(id, existing.len());
                    overlap_sessions.insert(msg.session_id.clone());
                    existing.push(msg);
                }
            }
            conversation.session_ids.extend(overlap_sessions);
            // A resumed/forked cache may use a new SDK id but replay the same
            // requests. Collapse overlapping balances conservatively as well.
            loop {
                let overlap = summaries
                    .iter()
                    .find(|(session, old)| {
                        *session == &conversation.session
                            || !old.session_ids.is_disjoint(&conversation.session_ids)
                    })
                    .map(|(session, _)| session.clone());
                let Some(key) = overlap else { break };
                let old = summaries.remove(&key).expect("key was found above");
                conversation.session_ids.extend(old.session_ids);
                conversation.last_snapshot = conversation.last_snapshot.max(old.last_snapshot);
                for (model, tokens) in old.totals {
                    max_tokens(conversation.totals.entry(model).or_default(), &tokens);
                }
            }
            summaries.insert(conversation.session.clone(), conversation);
        }
        for conversation in summaries.into_values() {
            let mut observed: BTreeMap<String, TokenBreakdown> = BTreeMap::new();
            for message in existing.iter() {
                if !message
                    .dedup_key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("claude-desktop-total:"))
                    && conversation.session_ids.contains(&message.session_id)
                    && (conversation.last_snapshot == 0
                        || message.timestamp <= conversation.last_snapshot)
                {
                    observed
                        .entry(model_id(&message.model_id))
                        .or_default()
                        .add_assign_saturating(&message.tokens);
                }
            }
            for (model, total) in conversation.totals {
                let remaining = subtract(
                    &total,
                    observed.get(&model).unwrap_or(&TokenBreakdown::default()),
                );
                if remaining.total() == 0 {
                    continue;
                }
                let key = format!("claude-desktop-total:{}:{model}", conversation.session);
                let mut message = UnifiedMessage::new_with_dedup(
                    "claude",
                    model,
                    "anthropic",
                    &conversation.session,
                    0,
                    remaining,
                    0.0,
                    Some(key),
                );
                message.date = UNKNOWN_DATE.into();
                message.message_count = 0;
                message.agent = Some("Claude Desktop history (date unknown)".into());
                existing.push(message);
            }
        }
    }
}

pub fn is_undated_balance(message: &UnifiedMessage) -> bool {
    message.timestamp == 0 && message.date == UNKNOWN_DATE
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree(events: Value) -> Value {
        json!({"conversationUuid":"c","product":"cowork","tree":{"events":events}})
    }
    fn assistant(id: &str, output: i64) -> Value {
        json!({"serverCreatedAt": 1700000000000i64,"payload":{"type":"assistant","session_id":"s","request_id":"r","message":{"id":id,"model":"claude-future-99","usage":{"input_tokens":10,"output_tokens":output,"cache_read_input_tokens":100}}}})
    }
    fn result(output: i64) -> Value {
        json!({"serverCreatedAt":1700000001000i64,"payload":{"type":"result","session_id":"s","modelUsage":{"claude-future-99":{"inputTokens":30,"outputTokens":output,"cacheReadInputTokens":500,"thinkingTokens":100},"claude-tiny-100":{"inputTokens":5,"outputTokens":6}}}})
    }

    #[test]
    fn cumulative_counters_all_models_and_streams_are_not_summed_twice() {
        let value = tree(json!([
            assistant("msg", 1),
            assistant("msg", 20),
            result(80),
            result(100)
        ]));
        let conversation = parse_tree(&value, &mut Coverage::default());
        let mut existing = vec![];
        DesktopUsage {
            coverage: Coverage::default(),
            conversations: vec![conversation],
        }
        .merge_into(&mut existing);
        assert_eq!(existing.len(), 3);
        assert_eq!(existing.iter().map(|m| m.tokens.total()).sum::<i64>(), 641);
        assert_eq!(existing.iter().map(|m| m.message_count).sum::<i32>(), 1);
        assert_eq!(
            existing
                .iter()
                .filter(|m| is_undated_balance(m))
                .map(|m| m.tokens.total())
                .sum::<i64>(),
            511
        );
        assert!(existing.iter().all(|m| m.tokens.reasoning == 0));
    }

    #[test]
    fn local_transcript_overlap_and_repeated_cache_do_not_inflate() {
        let value = tree(json!([assistant("msg", 1), result(100)]));
        let mut local = parse_tree(
            &tree(json!([assistant("msg", 40)])),
            &mut Coverage::default(),
        )
        .messages;
        let conversations = (0..2)
            .map(|_| parse_tree(&value, &mut Coverage::default()))
            .collect();
        DesktopUsage {
            coverage: Coverage::default(),
            conversations,
        }
        .merge_into(&mut local);
        assert_eq!(local.len(), 3);
        assert_eq!(local.iter().map(|m| m.tokens.total()).sum::<i64>(), 641);
        assert_eq!(local[0].tokens.output, 40);
    }

    #[test]
    fn resets_are_lower_bounds_and_newer_requests_still_count() {
        let mut late = assistant("later", 7);
        late["serverCreatedAt"] = json!(1700000002000i64);
        let value = tree(json!([assistant("msg", 20), result(100), result(2), late]));
        let mut coverage = Coverage::default();
        let conversation = parse_tree(&value, &mut coverage);
        assert_eq!(coverage.counter_resets, 1);
        let mut messages = vec![];
        DesktopUsage {
            coverage,
            conversations: vec![conversation],
        }
        .merge_into(&mut messages);
        assert_eq!(messages.iter().map(|m| m.tokens.total()).sum::<i64>(), 758);
    }

    #[test]
    fn chat_without_usage_is_not_estimated_and_numeric_counts_are_checked() {
        let value = json!({"conversationUuid":"c","product":"chat","tree":{"messages":[{"role":"assistant","id":"x","model":"claude-any","text":"not a token ledger"}]}});
        let mut coverage = Coverage::default();
        assert!(parse_tree(&value, &mut coverage).messages.is_empty());
        assert_eq!(coverage.messages_without_usage, 1);
        assert_eq!(token_count(&json!(1234567890.0)), 1234567890);
        assert_eq!(token_count(&json!(-1)), 0);
        assert_eq!(token_count(&json!(1.5)), 0);
    }

    #[test]
    fn advisor_calls_keep_their_identity_across_local_and_cached_copies() {
        let mut event = assistant("parent", 20);
        event["payload"]["message"]["usage"]["iterations"] = json!([
            {"type":"message", "model":"claude-future-99", "input_tokens":10},
            {"type":"advisor_message", "model":"claude-advisor-100", "input_tokens":7, "output_tokens":3},
            {"type":"advisor_message", "model":"claude-advisor-100", "input_tokens":11, "output_tokens":2}
        ]);
        let value = tree(json!([event.clone(), event]));
        let cached = parse_tree(&value, &mut Coverage::default());
        assert_eq!(cached.messages.len(), 3);
        let mut local = cached.messages.clone();
        DesktopUsage {
            coverage: Coverage::default(),
            conversations: vec![cached],
        }
        .merge_into(&mut local);
        assert_eq!(local.len(), 3);
        assert_eq!(local.iter().map(|m| m.tokens.total()).sum::<i64>(), 153);
        assert_eq!(local.iter().map(|m| m.message_count).sum::<i32>(), 1);
        assert!(local.iter().all(|m| !m.model_attribution_conflicted));
        let advisors: Vec<_> = local
            .iter()
            .filter(|m| m.model_id == "claude-advisor-100")
            .collect();
        assert_eq!(advisors.len(), 2);
        assert!(advisors.iter().all(|m| m.duration_ms.is_none()));
    }

    #[test]
    fn different_requests_and_message_only_fallbacks_remain_distinct() {
        let mut events = Vec::new();
        for request in [Some("r1"), Some("r2"), None] {
            for output in [1, 20] {
                let mut event = assistant("shared", output);
                event["payload"]
                    .as_object_mut()
                    .unwrap()
                    .remove("request_id");
                if let Some(request) = request {
                    event["payload"]["request_id"] = json!(request);
                }
                event["payload"]["message"]["usage"]["iterations"] = json!([
                    {"type":"advisor_message", "model":"claude-advisor-100", "input_tokens":7}
                ]);
                events.push(event);
            }
        }
        let parsed = parse_tree(&tree(json!(events)), &mut Coverage::default());
        assert_eq!(parsed.messages.len(), 6);
        assert_eq!(
            parsed
                .messages
                .iter()
                .map(|m| m.tokens.total())
                .sum::<i64>(),
            411
        );
        assert_eq!(
            parsed.messages.iter().map(|m| m.message_count).sum::<i32>(),
            3
        );

        // Each local request and advisor copy must merge only with its exact
        // cache counterpart, independently of input ordering.
        let mut local = parsed.messages.clone();
        local.reverse();
        DesktopUsage {
            coverage: Coverage::default(),
            conversations: vec![parsed],
        }
        .merge_into(&mut local);
        assert_eq!(local.len(), 6);
        assert_eq!(local.iter().map(|m| m.tokens.total()).sum::<i64>(), 411);
    }

    #[test]
    fn cache_request_does_not_merge_with_a_different_local_request() {
        let mut event = assistant("shared", 20);
        let mut local =
            parse_tree(&tree(json!([event.clone()])), &mut Coverage::default()).messages;
        event["payload"]["request_id"] = json!("different");
        let cached = parse_tree(&tree(json!([event])), &mut Coverage::default());
        DesktopUsage {
            coverage: Coverage::default(),
            conversations: vec![cached],
        }
        .merge_into(&mut local);
        assert_eq!(local.len(), 2);
        assert_eq!(local.iter().map(|m| m.tokens.total()).sum::<i64>(), 260);
    }

    #[test]
    fn placeholder_models_are_excluded_but_unpriced_models_are_accepted() {
        for model in ["", "   ", "unknown", "<synthetic>", "claude-future-999"] {
            let mut event = assistant("model-test", 20);
            event["payload"]["message"]["model"] = json!(model);
            let parsed = parse_tree(&tree(json!([event])), &mut Coverage::default());
            assert_eq!(
                parsed.messages.len(),
                usize::from(model == "claude-future-999"),
                "{model}"
            );
        }
    }

    #[test]
    fn cache_split_and_hour_cache_metadata_survive_streaming_and_local_overlap() {
        let mut bare = assistant("same", 1);
        bare["payload"]["message"]["usage"] = json!({"input_tokens":1000,"output_tokens":1});
        let mut split = bare.clone();
        split["payload"]["message"]["usage"] = json!({"input_tokens":100,"output_tokens":3,"cache_read_input_tokens":900,"cache_creation_input_tokens":80,"cache_creation":{"ephemeral_1h_input_tokens":50}});
        for events in [
            json!([bare.clone(), split.clone()]),
            json!([split.clone(), bare.clone()]),
        ] {
            let parsed = parse_tree(&tree(events), &mut Coverage::default());
            assert_eq!(parsed.messages[0].tokens.input, 100);
            assert_eq!(parsed.messages[0].tokens.cache_write_1h, 50);
            assert_eq!(parsed.messages[0].tokens.total(), 1083);
        }
        let mut local = parse_tree(&tree(json!([bare])), &mut Coverage::default()).messages;
        let cached = parse_tree(&tree(json!([split])), &mut Coverage::default());
        DesktopUsage {
            coverage: Coverage::default(),
            conversations: vec![cached],
        }
        .merge_into(&mut local);
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].tokens.input, 100);
        assert_eq!(local[0].tokens.cache_write_1h, 50);
    }

    #[test]
    fn undated_advisors_do_not_acquire_a_1970_date() {
        let mut event = assistant("parent", 20);
        event.as_object_mut().unwrap().remove("serverCreatedAt");
        event["payload"]["message"]["usage"]["iterations"] = json!([
            {"type":"advisor_message", "model":"claude-advisor-100", "input_tokens":7}
        ]);
        let parsed = parse_tree(&tree(json!([event])), &mut Coverage::default());
        assert_eq!(parsed.messages.len(), 2);
        assert!(parsed.messages.iter().all(is_undated_balance));
    }

    #[test]
    fn discovers_all_retained_roots_but_never_audit_logs() {
        let home = tempfile::tempdir().unwrap();
        let root = home
            .path()
            .join("Library/Application Support/Claude/local-agent-mode-sessions/a/o");
        for s in ["old", "new"] {
            std::fs::create_dir_all(root.join(s).join(".claude/projects")).unwrap();
        }
        std::fs::write(root.join("old/audit.jsonl"), "{}").unwrap();
        let roots = transcript_roots(home.path());
        assert_eq!(roots.len(), 2);
        assert!(roots.iter().all(|p| p.ends_with(".claude/projects")));
    }
}
