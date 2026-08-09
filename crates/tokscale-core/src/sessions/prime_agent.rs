//! Prime Agent session parser.
//!
//! Prime Agent stores root sessions in `~/.prime/agent/sessions/*.jsonl` and
//! RLM child sessions below the sibling `session-artifacts` tree. Both use the
//! Pi append-only JSONL record format, so token extraction is shared with the
//! Pi parser. `child_usage_attributed` records are never emitted as messages:
//! tokscale scans each child's own transcript directly. Their usage metadata is
//! used only to reverse aggregate parent usage that Prime may persist while
//! serializing a fork, before the copied parent is deduplicated across files.

use super::pi::{parse_pi_format_rlm_file, PiSessionEntry, PiSessionHeader, PiUsage};
use super::utils::parse_timestamp_str;
use super::UnifiedMessage;
use crate::TokenBreakdown;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn parse_prime_agent_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_pi_format_rlm_file(path, "prime-agent", "prime-agent")
}

#[derive(Debug, Clone)]
struct PrimeAttribution {
    id: String,
    timestamp: Option<i64>,
    child_usage: TokenBreakdown,
    aggregate_usage: TokenBreakdown,
}

#[derive(Debug, Clone)]
struct ChildMessageUsage {
    timestamp: Option<i64>,
    usage: TokenBreakdown,
}

#[derive(Debug, Clone)]
struct PrimeUsageAdjustment {
    dedup_key: String,
    persisted_usage: TokenBreakdown,
    attributions: Vec<PrimeAttribution>,
}

#[derive(Debug, Default)]
pub(crate) struct PrimeFileAccounting {
    source_path: PathBuf,
    attributions: Vec<PrimeAttribution>,
    adjustments: Vec<PrimeUsageAdjustment>,
    child_message_usages: Vec<ChildMessageUsage>,
    child_parent_path: Option<PathBuf>,
    fork_parent_path: Option<PathBuf>,
}

fn usage_breakdown(usage: &PiUsage) -> TokenBreakdown {
    TokenBreakdown {
        input: usage.input.unwrap_or(0).max(0),
        output: usage.output.unwrap_or(0).max(0),
        cache_read: usage.cache_read.unwrap_or(0).max(0),
        cache_write: usage.cache_write.unwrap_or(0).max(0),
        reasoning: 0,
    }
}

fn add_usage(total: &mut TokenBreakdown, usage: &TokenBreakdown) {
    total.input = total.input.saturating_add(usage.input);
    total.output = total.output.saturating_add(usage.output);
    total.cache_read = total.cache_read.saturating_add(usage.cache_read);
    total.cache_write = total.cache_write.saturating_add(usage.cache_write);
}

fn maximize_usage(total: &mut TokenBreakdown, usage: &TokenBreakdown) {
    total.input = total.input.max(usage.input);
    total.output = total.output.max(usage.output);
    total.cache_read = total.cache_read.max(usage.cache_read);
    total.cache_write = total.cache_write.max(usage.cache_write);
}

fn subtract_usage(total: &mut TokenBreakdown, usage: &TokenBreakdown) {
    total.input = total.input.saturating_sub(usage.input).max(0);
    total.output = total.output.saturating_sub(usage.output).max(0);
    total.cache_read = total.cache_read.saturating_sub(usage.cache_read).max(0);
    total.cache_write = total.cache_write.saturating_sub(usage.cache_write).max(0);
}

type UsageKey = (i64, i64, i64, i64);
type LineageUsageKey = (PathBuf, UsageKey);

fn usage_key(usage: &TokenBreakdown) -> UsageKey {
    (
        usage.input,
        usage.output,
        usage.cache_read,
        usage.cache_write,
    )
}

fn lineage_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn referenced_lineage_path(source_file: &Path, referenced: &Path) -> PathBuf {
    if referenced.is_absolute() {
        lineage_path(referenced)
    } else {
        lineage_path(
            &source_file
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(referenced),
        )
    }
}

/// Read Prime-only accounting records that are intentionally absent from the
/// shared Pi message representation. `messages` may come from the source cache;
/// their stable order is used to associate target entry ids with emitted rows.
pub(crate) fn analyze_prime_agent_accounting(
    path: &Path,
    messages: &[UnifiedMessage],
) -> PrimeFileAccounting {
    let Ok(file) = std::fs::File::open(path) else {
        return PrimeFileAccounting::default();
    };

    let source_path = lineage_path(path);
    let mut found_header = false;
    let mut is_rlm_child = false;
    let mut child_parent_path = None;
    let mut fork_parent_path = None;
    let mut message_index = 0usize;
    let mut targets: HashMap<String, (String, TokenBreakdown)> = HashMap::new();
    let mut attributions: HashMap<String, Vec<PrimeAttribution>> = HashMap::new();
    let mut child_message_usages = Vec::new();

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !found_header {
            if let Ok(header) = serde_json::from_str::<PiSessionHeader>(trimmed) {
                if header.entry_type == "session" {
                    found_header = true;
                    is_rlm_child = header.rlm_depth.unwrap_or(0) > 0;
                    let parent_path = header
                        .parent_session
                        .as_deref()
                        .map(Path::new)
                        .map(|parent| referenced_lineage_path(path, parent));
                    if is_rlm_child {
                        child_parent_path = parent_path;
                    } else {
                        fork_parent_path = parent_path;
                    }
                    continue;
                }
            }
            let is_pre_session_title = serde_json::from_str::<serde_json::Value>(trimmed)
                .ok()
                .and_then(|value| {
                    value
                        .get("type")
                        .and_then(|kind| kind.as_str())
                        .map(str::to_owned)
                })
                .is_some_and(|kind| kind == "title");
            if is_pre_session_title {
                continue;
            }
            return PrimeFileAccounting::default();
        }

        let Ok(entry) = serde_json::from_str::<PiSessionEntry>(trimmed) else {
            continue;
        };
        let entry_timestamp = entry.timestamp.as_deref().and_then(parse_timestamp_str);
        if entry.entry_type == "child_usage_attributed" {
            if let (Some(id), Some(target_id), Some(child_usage), Some(aggregate_usage)) = (
                entry.id,
                entry.target_id,
                entry.child_usage,
                entry.aggregate_usage,
            ) {
                attributions
                    .entry(target_id)
                    .or_default()
                    .push(PrimeAttribution {
                        id,
                        timestamp: entry_timestamp,
                        child_usage: usage_breakdown(&child_usage),
                        aggregate_usage: usage_breakdown(&aggregate_usage),
                    });
            }
            continue;
        }
        if entry.entry_type != "message" {
            continue;
        }
        let Some(message) = entry.message else {
            continue;
        };
        if message.role.as_deref() != Some("assistant")
            || message.usage.is_none()
            || message.model.is_none()
        {
            continue;
        }

        let parsed = messages.get(message_index);
        message_index += 1;
        let Some(parsed) = parsed else {
            continue;
        };
        if is_rlm_child {
            child_message_usages.push(ChildMessageUsage {
                timestamp: entry_timestamp,
                usage: parsed.tokens.clone(),
            });
        }
        if let (Some(id), Some(dedup_key)) = (entry.id, parsed.dedup_key.clone()) {
            targets.insert(id, (dedup_key, parsed.tokens.clone()));
        }
    }

    let all_attributions = attributions
        .values()
        .flat_map(|entries| entries.iter().cloned())
        .collect();
    let mut adjustments = Vec::new();
    for (target_id, entries) in attributions {
        let Some((dedup_key, persisted_usage)) = targets.get(&target_id) else {
            continue;
        };
        let mut matching_prefix = None;
        for (index, entry) in entries.iter().enumerate() {
            if entry.aggregate_usage == *persisted_usage {
                matching_prefix = Some(entries[..=index].to_vec());
            }
        }
        if let Some(prefix) = matching_prefix {
            adjustments.push(PrimeUsageAdjustment {
                dedup_key: dedup_key.clone(),
                persisted_usage: persisted_usage.clone(),
                attributions: prefix,
            });
        }
    }

    PrimeFileAccounting {
        source_path,
        attributions: all_attributions,
        adjustments,
        child_message_usages,
        child_parent_path,
        fork_parent_path,
    }
}

fn fallback_key_base(key: &str) -> Option<&str> {
    if !key.starts_with("prime-agent:message:") {
        return None;
    }
    let mut parts = key.rsplitn(5, ':');
    parts.next()?;
    parts.next()?;
    parts.next()?;
    parts.next()?;
    parts.next()
}

fn rewrite_fallback_usage(key: &str, usage: &TokenBreakdown) -> String {
    fallback_key_base(key).map_or_else(
        || key.to_string(),
        |base| {
            format!(
                "{base}:{}:{}:{}:{}",
                usage.input, usage.output, usage.cache_read, usage.cache_write
            )
        },
    )
}

/// Subtract child usage only when a matching RLM transcript was actually
/// parsed, then collapse fork copies. Missing/pruned children remain represented
/// by Prime's aggregate parent usage instead of disappearing from the total.
pub(crate) fn reconcile_prime_agent_messages(
    messages: Vec<UnifiedMessage>,
    accounting: &[PrimeFileAccounting],
) -> Vec<UnifiedMessage> {
    const ATTRIBUTION_TIMESTAMP_TOLERANCE_MS: i64 = 1_000;

    let mut available_children: HashMap<LineageUsageKey, Vec<Option<i64>>> = HashMap::new();
    for file in accounting {
        if let Some(parent_path) = &file.child_parent_path {
            for child in &file.child_message_usages {
                available_children
                    .entry((parent_path.clone(), usage_key(&child.usage)))
                    .or_default()
                    .push(child.timestamp);
            }
        }
    }

    // Attribution ids survive fork serialization. Record every file that owns
    // a copy, but match only a child response whose header points back to that
    // parent session and whose completion timestamp is the same event (Prime
    // writes the two records within milliseconds). This disambiguates equal
    // token buckets produced by separate children in one parent.
    let mut unique_attributions: BTreeMap<
        String,
        (TokenBreakdown, Option<i64>, BTreeSet<PathBuf>),
    > = BTreeMap::new();
    for file in accounting {
        for attribution in &file.attributions {
            let (_, _, owners) = unique_attributions
                .entry(attribution.id.clone())
                .or_insert_with(|| {
                    (
                        attribution.child_usage.clone(),
                        attribution.timestamp,
                        BTreeSet::new(),
                    )
                });
            owners.insert(file.source_path.clone());
            if let Some(parent) = &file.fork_parent_path {
                owners.insert(parent.clone());
            }
        }
    }

    let mut represented_attributions = HashSet::new();
    for (id, (usage, attribution_timestamp, owners)) in unique_attributions {
        let mut timed_candidates = Vec::new();
        let mut untimed_candidates = Vec::new();
        for owner in owners {
            let key = (owner, usage_key(&usage));
            let Some(children) = available_children.get(&key) else {
                continue;
            };
            for (index, child_timestamp) in children.iter().enumerate() {
                match (attribution_timestamp, *child_timestamp) {
                    (Some(attribution), Some(child)) => {
                        let distance = attribution.abs_diff(child) as i64;
                        if distance <= ATTRIBUTION_TIMESTAMP_TOLERANCE_MS {
                            timed_candidates.push((distance, key.clone(), index));
                        }
                    }
                    _ => untimed_candidates.push((key.clone(), index)),
                }
            }
        }

        timed_candidates.sort_by_key(|candidate| candidate.0);
        let selected = timed_candidates.first().and_then(|best| {
            let tied = timed_candidates.get(1).is_some_and(|next| next.0 == best.0);
            (!tied).then(|| (best.1.clone(), best.2))
        });
        let selected = selected.or_else(|| {
            (timed_candidates.is_empty() && untimed_candidates.len() == 1)
                .then(|| untimed_candidates.remove(0))
        });
        if let Some((key, index)) = selected {
            if let Some(children) = available_children.get_mut(&key) {
                children.swap_remove(index);
                represented_attributions.insert(id);
            }
        }
    }

    let mut adjustment_groups: HashMap<String, Vec<&PrimeUsageAdjustment>> = HashMap::new();
    let mut attribution_fallback_bases = HashSet::new();
    for adjustment in accounting.iter().flat_map(|file| &file.adjustments) {
        let identity = fallback_key_base(&adjustment.dedup_key)
            .inspect(|base| {
                attribution_fallback_bases.insert((*base).to_string());
            })
            .unwrap_or(&adjustment.dedup_key)
            .to_string();
        adjustment_groups
            .entry(identity)
            .or_default()
            .push(adjustment);
    }

    let mut grouped: HashMap<String, Vec<UnifiedMessage>> = HashMap::new();
    let mut group_order = Vec::new();
    for (ordinal, message) in messages.into_iter().enumerate() {
        let identity = message.dedup_key.as_deref().map_or_else(
            || format!("prime-agent:unkeyed:{ordinal}"),
            |key| {
                fallback_key_base(key)
                    .filter(|base| attribution_fallback_bases.contains(*base))
                    .unwrap_or(key)
                    .to_string()
            },
        );
        if !grouped.contains_key(&identity) {
            group_order.push(identity.clone());
        }
        grouped.entry(identity).or_default().push(message);
    }

    let mut deduped = Vec::with_capacity(group_order.len());
    for identity in group_order {
        let mut group = grouped.remove(&identity).unwrap_or_default();
        let Some(mut representative) = group.first().cloned() else {
            continue;
        };
        let Some(adjustments) = adjustment_groups.get(&identity) else {
            for duplicate in group.iter().skip(1) {
                maximize_usage(&mut representative.tokens, &duplicate.tokens);
            }
            deduped.push(representative);
            continue;
        };

        let mut base_usage = TokenBreakdown::default();
        let mut found_base = false;
        let mut all_attributions: BTreeMap<String, TokenBreakdown> = BTreeMap::new();
        for adjustment in adjustments {
            let mut own_usage = adjustment.persisted_usage.clone();
            for attribution in &adjustment.attributions {
                subtract_usage(&mut own_usage, &attribution.child_usage);
                all_attributions
                    .entry(attribution.id.clone())
                    .or_insert_with(|| attribution.child_usage.clone());
            }
            maximize_usage(&mut base_usage, &own_usage);
            found_base = true;
        }
        for message in &group {
            let is_aggregate_copy = adjustments.iter().any(|adjustment| {
                message.dedup_key.as_deref() == Some(&adjustment.dedup_key)
                    && message.tokens == adjustment.persisted_usage
            });
            if !is_aggregate_copy {
                maximize_usage(&mut base_usage, &message.tokens);
                found_base = true;
            }
        }
        if !found_base {
            for message in &group {
                maximize_usage(&mut base_usage, &message.tokens);
            }
        }
        for (id, usage) in all_attributions {
            if !represented_attributions.contains(&id) {
                add_usage(&mut base_usage, &usage);
            }
        }

        representative.tokens = base_usage;
        if let Some(key) = representative.dedup_key.as_deref() {
            representative.dedup_key = Some(rewrite_fallback_usage(key, &representative.tokens));
        }
        group.clear();
        deduped.push(representative);
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn session_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn parses_root_session_without_counting_child_attribution_records() {
        let file = session_file(
            r#"{"type":"session","version":3,"id":"root-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"session_info","id":"info","parentId":null,"timestamp":"2026-08-08T00:00:00.500Z","name":"My renamed thread"}
{"type":"message","id":"assistant-1","parentId":"info","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"msg_provider_001","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}
{"type":"child_usage_attributed","id":"usage-1","parentId":"assistant-1","timestamp":"2026-08-08T00:00:02.000Z","targetId":"assistant-1","childUsage":{"input":500,"output":200,"cacheRead":0,"cacheWrite":0,"totalTokens":700},"aggregateUsage":{"input":600,"output":250,"cacheRead":20,"cacheWrite":10,"totalTokens":880},"origin":"spawn_task"}"#,
        );

        let messages = parse_prime_agent_file(file.path());

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "prime-agent");
        assert_eq!(message.session_id, "root-1");
        assert_eq!(message.workspace_key.as_deref(), Some("/tmp/project"));
        assert_eq!(message.tokens.input, 100);
        assert_eq!(message.tokens.output, 50);
        assert_eq!(message.tokens.cache_read, 20);
        assert_eq!(message.tokens.cache_write, 10);
        assert_eq!(message.agent, None, "a root thread name is not an agent");
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("prime-agent:response:msg_provider_001")
        );
    }

    #[test]
    fn attributes_rlm_child_messages_to_the_session_name() {
        let file = session_file(
            r#"{"type":"session","version":3,"id":"child-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","parentSession":"/tmp/root.jsonl","rlmDepth":1}
{"type":"session_info","id":"info","parentId":null,"timestamp":"2026-08-08T00:00:00.500Z","name":"api-reviewer"}
{"type":"message","id":"assistant-1","parentId":"info","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"openai","model":"gpt-5.4","usage":{"input":40,"output":12,"cacheRead":8,"cacheWrite":0,"totalTokens":60}}}"#,
        );

        let messages = parse_prime_agent_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent.as_deref(), Some("api-reviewer"));
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].model_id, "gpt-5.4");
    }

    #[test]
    fn keeps_aggregate_parent_when_the_attributed_child_is_unavailable() {
        let file = session_file(
            r#"{"type":"session","version":3,"id":"fork-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250}}}
{"type":"child_usage_attributed","id":"usage-1","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":50,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":70},"aggregateUsage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250},"origin":"spawn_task"}"#,
        );

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);
        let messages = reconcile_prime_agent_messages(messages, &[accounting]);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 150);
        assert_eq!(messages[0].tokens.output, 70);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.cache_write, 10);
    }

    #[test]
    fn blank_model_message_does_not_shift_accounting_alignment() {
        let file = session_file(
            r#"{"type":"session","version":3,"id":"root","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"blank","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"","responseId":"blank-response","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}
{"type":"message","id":"parent","parentId":"blank","timestamp":"2026-08-08T00:00:02.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250}}}
{"type":"child_usage_attributed","id":"usage-1","parentId":"parent","timestamp":"2026-08-08T00:00:03.000Z","targetId":"parent","childUsage":{"input":50,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":70},"aggregateUsage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250},"origin":"spawn_task"}"#,
        );

        let messages = parse_prime_agent_file(file.path());
        let accounting = analyze_prime_agent_accounting(file.path(), &messages);

        assert_eq!(messages.len(), 2);
        assert_eq!(accounting.adjustments.len(), 1);
        assert_eq!(
            accounting.adjustments[0].dedup_key,
            "prime-agent:response:parent-response"
        );
    }

    #[test]
    fn sibling_forks_preserve_each_distinct_unavailable_child_delta() {
        fn tokens(input: i64) -> TokenBreakdown {
            TokenBreakdown {
                input,
                ..TokenBreakdown::default()
            }
        }
        fn parent_message(input: i64, session: &str) -> UnifiedMessage {
            let mut message = UnifiedMessage::new(
                "prime-agent",
                "claude-opus-5",
                "anthropic",
                session,
                1,
                tokens(input),
                0.0,
            );
            message.dedup_key = Some("prime-agent:response:shared-parent".to_string());
            message
        }
        fn fork_accounting(
            source: &str,
            attribution_id: &str,
            child_input: i64,
        ) -> PrimeFileAccounting {
            let attribution = PrimeAttribution {
                id: attribution_id.to_string(),
                timestamp: Some(1),
                child_usage: tokens(child_input),
                aggregate_usage: tokens(100 + child_input),
            };
            PrimeFileAccounting {
                source_path: PathBuf::from(source),
                attributions: vec![attribution.clone()],
                adjustments: vec![PrimeUsageAdjustment {
                    dedup_key: "prime-agent:response:shared-parent".to_string(),
                    persisted_usage: tokens(100 + child_input),
                    attributions: vec![attribution],
                }],
                ..PrimeFileAccounting::default()
            }
        }

        let messages = vec![parent_message(150, "fork-a"), parent_message(130, "fork-b")];
        let accounting = [
            fork_accounting("fork-a.jsonl", "child-a", 50),
            fork_accounting("fork-b.jsonl", "child-b", 30),
        ];
        let messages = reconcile_prime_agent_messages(messages, &accounting);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 180);
    }

    #[test]
    fn equal_child_usage_is_matched_by_parent_lineage_and_completion_time() {
        let dir = tempfile::TempDir::new().unwrap();
        let parent_path = dir.path().join("parent.jsonl");
        let child_path = dir.path().join("child.jsonl");
        std::fs::write(
            &parent_path,
            r#"{"type":"session","version":3,"id":"parent","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent-a","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"model-a","responseId":"parent-response-a","usage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}
{"type":"child_usage_attributed","id":"usage-a","parentId":"parent-a","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent-a","childUsage":{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50},"aggregateUsage":{"input":150,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":150},"origin":"spawn_task"}
{"type":"message","id":"parent-b","parentId":"usage-a","timestamp":"2026-08-08T00:00:10.000Z","message":{"role":"assistant","provider":"anthropic","model":"model-b","responseId":"parent-response-b","usage":{"input":250,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":250}}}
{"type":"child_usage_attributed","id":"usage-b","parentId":"parent-b","timestamp":"2026-08-08T00:00:11.000Z","targetId":"parent-b","childUsage":{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50},"aggregateUsage":{"input":250,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":250},"origin":"spawn_task"}
"#,
        )
        .unwrap();
        std::fs::write(
            &child_path,
            format!(
                r#"{{"type":"session","version":3,"id":"child","timestamp":"2026-08-08T00:00:10.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child-message","parentId":null,"timestamp":"2026-08-08T00:00:11.001Z","message":{{"role":"assistant","provider":"anthropic","model":"child-model","responseId":"child-response","usage":{{"input":50,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":50}}}}}}
"#,
                serde_json::to_string(&parent_path.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();

        let parent_messages = parse_prime_agent_file(&parent_path);
        let child_messages = parse_prime_agent_file(&child_path);
        let accounting = [
            analyze_prime_agent_accounting(&parent_path, &parent_messages),
            analyze_prime_agent_accounting(&child_path, &child_messages),
        ];
        let messages = reconcile_prime_agent_messages(
            parent_messages.into_iter().chain(child_messages).collect(),
            &accounting,
        );

        let parent_a = messages
            .iter()
            .find(|message| {
                message.dedup_key.as_deref() == Some("prime-agent:response:parent-response-a")
            })
            .unwrap();
        let parent_b = messages
            .iter()
            .find(|message| {
                message.dedup_key.as_deref() == Some("prime-agent:response:parent-response-b")
            })
            .unwrap();
        assert_eq!(parent_a.tokens.input, 150);
        assert_eq!(parent_b.tokens.input, 200);
    }

    #[test]
    fn same_sized_child_from_another_parent_does_not_authorize_subtraction() {
        let dir = tempfile::TempDir::new().unwrap();
        let parent_path = dir.path().join("parent-a.jsonl");
        let child_path = dir.path().join("child-b.jsonl");
        let unrelated_parent = dir.path().join("parent-b.jsonl");
        std::fs::write(
            &parent_path,
            r#"{"type":"session","version":3,"id":"parent-a","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"parent","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"parent-response","usage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250}}}
{"type":"child_usage_attributed","id":"usage-a","parentId":"parent","timestamp":"2026-08-08T00:00:02.000Z","targetId":"parent","childUsage":{"input":50,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":70},"aggregateUsage":{"input":150,"output":70,"cacheRead":20,"cacheWrite":10,"totalTokens":250},"origin":"spawn_task"}
"#,
        )
        .unwrap();
        std::fs::write(
            &child_path,
            format!(
                r#"{{"type":"session","version":3,"id":"child-b","timestamp":"2026-08-08T00:00:01.000Z","cwd":"/tmp/project","parentSession":{},"rlmDepth":1}}
{{"type":"message","id":"child","parentId":null,"timestamp":"2026-08-08T00:00:02.000Z","message":{{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"child-response","usage":{{"input":50,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":70}}}}}}
"#,
                serde_json::to_string(&unrelated_parent.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();

        let parent_messages = parse_prime_agent_file(&parent_path);
        let child_messages = parse_prime_agent_file(&child_path);
        let accounting = [
            analyze_prime_agent_accounting(&parent_path, &parent_messages),
            analyze_prime_agent_accounting(&child_path, &child_messages),
        ];
        let messages = reconcile_prime_agent_messages(
            parent_messages.into_iter().chain(child_messages).collect(),
            &accounting,
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.input)
                .sum::<i64>(),
            200
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.output)
                .sum::<i64>(),
            90
        );
    }

    #[test]
    fn copied_fork_history_keeps_a_cross_session_dedup_key() {
        let original = session_file(
            r#"{"type":"session","version":3,"id":"root-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"assistant-1","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"msg_provider_001","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}"#,
        );
        let fork = session_file(
            r#"{"type":"session","version":3,"id":"fork-2","timestamp":"2026-08-08T01:00:00.000Z","cwd":"/tmp/project","parentSession":"/tmp/root.jsonl","rlmDepth":0}
{"type":"message","id":"assistant-1","parentId":null,"timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","responseId":"msg_provider_001","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}"#,
        );

        let original = parse_prime_agent_file(original.path());
        let fork = parse_prime_agent_file(fork.path());

        assert_eq!(original.len(), 1);
        assert_eq!(fork.len(), 1);
        assert_eq!(original[0].dedup_key, fork[0].dedup_key);
    }

    #[test]
    fn copied_fork_history_without_response_or_event_timestamp_still_deduplicates() {
        let original = session_file(
            r#"{"type":"session","version":3,"id":"root-1","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project","rlmDepth":0}
{"type":"message","id":"assistant-1","parentId":null,"message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        let fork = session_file(
            r#"{"type":"session","version":3,"id":"fork-2","timestamp":"2026-08-08T01:00:00.000Z","cwd":"/tmp/project","parentSession":"/tmp/root.jsonl","rlmDepth":0}
{"type":"message","id":"assistant-1","parentId":null,"message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":10,"totalTokens":180}}}"#,
        );

        let original = parse_prime_agent_file(original.path());
        let fork = parse_prime_agent_file(fork.path());

        assert_ne!(original[0].timestamp, fork[0].timestamp);
        assert_eq!(original[0].dedup_key, fork[0].dedup_key);
    }

    #[test]
    fn rejects_the_rlm_subagent_catalog_as_a_session() {
        let file = session_file(
            r#"{"type":"rlm_subagent","childId":"sub-deadbeef","sessionName":"worker","sessionFile":"/tmp/child.jsonl"}"#,
        );

        assert!(parse_prime_agent_file(file.path()).is_empty());
    }
}
