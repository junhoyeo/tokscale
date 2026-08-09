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
    child_usage: TokenBreakdown,
    aggregate_usage: TokenBreakdown,
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
    child_message_usages: Vec<TokenBreakdown>,
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
        if message.role.as_deref() != Some("assistant") || message.usage.is_none() {
            continue;
        }
        let Some(model) = message.model.filter(|model| !model.is_empty()) else {
            continue;
        };
        drop(model);

        let Some(parsed) = messages.get(message_index) else {
            break;
        };
        message_index += 1;
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

    let child_message_usages = if is_rlm_child {
        messages
            .iter()
            .map(|message| message.tokens.clone())
            .collect()
    } else {
        Vec::new()
    };

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
    mut messages: Vec<UnifiedMessage>,
    accounting: &[PrimeFileAccounting],
) -> Vec<UnifiedMessage> {
    let mut available_children: HashMap<LineageUsageKey, usize> = HashMap::new();
    for file in accounting {
        if let Some(parent_path) = &file.child_parent_path {
            for usage in &file.child_message_usages {
                *available_children
                    .entry((parent_path.clone(), usage_key(usage)))
                    .or_default() += 1;
            }
        }
    }

    // Attribution ids survive fork serialization. Record every file that owns
    // a copy, but only match against children whose header points back to that
    // exact parent session file. A same-sized child from another lineage must
    // never authorize subtraction.
    let mut unique_attributions: BTreeMap<String, (TokenBreakdown, BTreeSet<PathBuf>)> =
        BTreeMap::new();
    for file in accounting {
        for attribution in &file.attributions {
            let (_, owners) = unique_attributions
                .entry(attribution.id.clone())
                .or_insert_with(|| (attribution.child_usage.clone(), BTreeSet::new()));
            owners.insert(file.source_path.clone());
            if let Some(parent) = &file.fork_parent_path {
                owners.insert(parent.clone());
            }
        }
    }
    let mut represented_attributions = HashSet::new();
    for (id, (usage, owners)) in unique_attributions {
        for owner in owners {
            let key = (owner, usage_key(&usage));
            let Some(count) = available_children.get_mut(&key) else {
                continue;
            };
            if *count > 0 {
                *count -= 1;
                represented_attributions.insert(id);
                break;
            }
        }
    }

    let mut attribution_fallback_bases = HashSet::new();
    for adjustment in accounting.iter().flat_map(|file| &file.adjustments) {
        if let Some(base) = fallback_key_base(&adjustment.dedup_key) {
            attribution_fallback_bases.insert(base.to_string());
        }
        let mut represented_usage = TokenBreakdown::default();
        for attribution in &adjustment.attributions {
            if represented_attributions.contains(&attribution.id) {
                add_usage(&mut represented_usage, &attribution.child_usage);
            }
        }
        if represented_usage == TokenBreakdown::default() {
            continue;
        }
        for message in &mut messages {
            if message.dedup_key.as_deref() == Some(&adjustment.dedup_key)
                && message.tokens == adjustment.persisted_usage
            {
                subtract_usage(&mut message.tokens, &represented_usage);
                if let Some(key) = message.dedup_key.as_deref() {
                    message.dedup_key = Some(rewrite_fallback_usage(key, &message.tokens));
                }
            }
        }
    }

    let mut deduped = Vec::<UnifiedMessage>::new();
    let mut seen = HashMap::<String, usize>::new();
    for message in messages {
        let Some(key) = message.dedup_key.as_deref() else {
            deduped.push(message);
            continue;
        };
        let identity = fallback_key_base(key)
            .filter(|base| attribution_fallback_bases.contains(*base))
            .unwrap_or(key)
            .to_string();
        if let Some(index) = seen.get(&identity).copied() {
            let existing = &mut deduped[index];
            existing.tokens.input = existing.tokens.input.max(message.tokens.input);
            existing.tokens.output = existing.tokens.output.max(message.tokens.output);
            existing.tokens.cache_read = existing.tokens.cache_read.max(message.tokens.cache_read);
            existing.tokens.cache_write =
                existing.tokens.cache_write.max(message.tokens.cache_write);
        } else {
            seen.insert(identity, deduped.len());
            deduped.push(message);
        }
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
