//! Droid (Factory.ai) session parser
//!
//! Parses JSON files from ~/.factory/sessions/

use super::utils::{
    file_modified_timestamp_ms_opt, lossy_lines, parse_timestamp_value, read_file_or_none,
};
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use serde_json::Value;
use std::io::{BufReader, Seek};
use std::path::{Path, PathBuf};

/// Droid settings.json structure
#[derive(Debug, Deserialize)]
pub struct DroidSettingsJson {
    pub model: Option<String>,
    #[serde(rename = "providerLock")]
    pub provider_lock: Option<String>,
    #[serde(rename = "providerLockTimestamp")]
    pub provider_lock_timestamp: Option<String>,
    #[serde(rename = "tokenUsage")]
    pub token_usage: Option<DroidTokenUsage>,
}

#[derive(Debug, Deserialize)]
pub struct DroidTokenUsage {
    #[serde(rename = "inputTokens")]
    pub input_tokens: Option<i64>,
    #[serde(rename = "outputTokens")]
    pub output_tokens: Option<i64>,
    #[serde(rename = "cacheCreationTokens")]
    pub cache_creation_tokens: Option<i64>,
    #[serde(rename = "cacheReadTokens")]
    pub cache_read_tokens: Option<i64>,
    #[serde(rename = "thinkingTokens")]
    pub thinking_tokens: Option<i64>,
}

/// Normalize model name from Droid's custom format
/// e.g., "custom:Claude-Opus-4.5-Thinking-[Anthropic]-0" -> "claude-opus-4-5-thinking-0"
/// e.g., "gemini-2.5-pro" -> "gemini-2-5-pro"
/// e.g., "Claude-Sonnet-4-[Anthropic]" -> "claude-sonnet-4"
fn normalize_model_name(model: &str) -> String {
    // Remove "custom:" prefix if present
    let mut normalized = model.strip_prefix("custom:").unwrap_or(model).to_string();

    // Handle bracket notation like "Claude-Opus-4.5-Thinking-[Anthropic]-0"
    // Remove [anything] patterns (like TypeScript's .replace(/\[.*?\]/g, ""))
    let mut result = String::new();
    let mut in_bracket = false;

    for ch in normalized.chars() {
        match ch {
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            _ if !in_bracket => result.push(ch),
            _ => {}
        }
    }

    normalized = result;

    // Remove trailing hyphens only (like TypeScript's .replace(/-+$/, ""))
    // NOTE: Do NOT remove trailing digits - TypeScript keeps them
    normalized = normalized.trim_end_matches('-').to_string();

    // Convert to lowercase (like TypeScript's .toLowerCase())
    normalized = normalized.to_lowercase();

    // Replace dots with hyphens (like TypeScript's .replace(/\./g, "-"))
    normalized = normalized.replace('.', "-");

    // Collapse multiple consecutive hyphens into one (like TypeScript's .replace(/-+/g, "-"))
    let mut collapsed = String::new();
    let mut last_was_hyphen = false;
    for ch in normalized.chars() {
        if ch == '-' {
            if !last_was_hyphen {
                collapsed.push(ch);
            }
            last_was_hyphen = true;
        } else {
            collapsed.push(ch);
            last_was_hyphen = false;
        }
    }

    collapsed
}

fn get_provider_from_model(model: &str) -> &'static str {
    provider_identity::inferred_provider_from_model(model).unwrap_or("unknown")
}

/// Get default model name based on provider when model field is missing
fn get_default_model_from_provider(provider: &str) -> String {
    match provider_identity::canonical_provider(provider)
        .as_deref()
        .unwrap_or(provider)
    {
        "anthropic" => "claude-unknown".to_string(),
        "openai" => "gpt-unknown".to_string(),
        "google" => "gemini-unknown".to_string(),
        "xai" => "grok-unknown".to_string(),
        _ => format!("{}-unknown", provider),
    }
}

/// Try to extract model name from JSONL file's system-reminder
/// Looks for pattern: "Model: Claude Opus 4.5 Thinking [Anthropic]"
fn extract_model_from_jsonl(jsonl_path: &Path) -> Option<String> {
    let file = std::fs::File::open(jsonl_path).ok()?;
    let reader = BufReader::new(file);

    // Scan more lines for parity with TypeScript which reads entire file
    // Cap at 500 lines to avoid performance issues with very large files.
    //
    // `lossy_lines` rather than `lines()`: the latter ends the iteration on the
    // first line that is not valid UTF-8, and `line.ok()?` abandoned model
    // extraction for the whole file at that point.
    for line in lossy_lines(reader).take(500) {
        // Look for Model: pattern in system-reminder
        if let Some(pos) = line.find("Model:") {
            let after_model = &line[pos + 6..];
            // Extract until [ or end of string/newline
            let model_part: String = after_model
                .chars()
                .take_while(|&c| c != '[' && c != '\\' && c != '"')
                .collect();
            let model_name = model_part.trim();
            if !model_name.is_empty() {
                return Some(normalize_model_name(model_name));
            }
        }
    }

    None
}

/// Return the fallback JSONL consulted when a settings snapshot omits its
/// model. The cache watches this path even when it is currently absent so a
/// later-created transcript invalidates the stored fallback model.
pub(crate) fn droid_jsonl_path(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".settings.json")?;
    Some(path.with_file_name(format!("{stem}.jsonl")))
}

/// Parse `providerLockTimestamp` into epoch milliseconds, rejecting values that
/// cannot describe a real provider lock.
///
/// Zero is Droid's unset sentinel, and a negative value is a clock or
/// corruption artifact — neither is a usable anchor. Both collapse to `None` so
/// the resolver treats them as absent, which keeps this anchor's validity rule
/// symmetric with the mtime one: `file_modified_timestamp_ms_opt` already
/// reports `None` for a pre-epoch mtime. Without that symmetry a negative lock
/// would survive as the resolved anchor whenever mtime was unavailable, and the
/// record would land in a 1969 bucket that no date filter can reach.
fn parse_lock_timestamp(raw: Option<&str>) -> Option<i64> {
    raw.and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.timestamp_millis())
        .filter(|&ts| ts > 0)
}

/// Pick the single instant a session's cumulative `tokenUsage` is attributed
/// to, for the sessions whose transcript cannot spread it across the calls that
/// spent it.
///
/// `providerLockTimestamp` is the wrong anchor: it records when the provider
/// was *selected*, not when the tokens were spent. Droid rewrites the totals in
/// place without touching that field, so a session left running across days
/// keeps reporting its very first instant while the totals climb, and reads as
/// silent in `--today` even while it is actively burning tokens.
///
/// The file's mtime is when the totals being read were written, which is the
/// closest available marker for when they were last accrued, so it wins. The
/// lock timestamp becomes a floor rather than the answer: usage cannot predate
/// provider selection, so a stale mtime (a restore or copy that rewound it)
/// cannot drag the record earlier than the session could possibly have run.
///
/// When the filesystem reports no mtime at all, fall back to the lock
/// timestamp, then to now() — a record with real token usage is never dropped
/// just because its timestamp could not be resolved.
fn resolve_usage_timestamp(lock_timestamp: Option<i64>, modified: Option<i64>) -> i64 {
    match (modified, lock_timestamp) {
        (Some(modified), Some(lock)) => modified.max(lock),
        (Some(modified), None) => modified,
        (None, Some(lock)) => lock,
        (None, None) => chrono::Utc::now().timestamp_millis(),
    }
}

/// One assistant reply in a Droid transcript: when it was written, and how
/// expensive it plausibly was relative to the session's other replies.
///
/// Two weights, because a reply's read cost and its write cost do not move
/// together. `context_weight` is the conversation standing before the reply,
/// which is what the call had to read; `output_weight` is the reply's own
/// bytes, which is what the call produced.
///
/// `replies` is how many assistant replies the turn stands for. It is 1 until
/// `coalesce_turns` folds a run together. The session's replies are summed and
/// carried on a single emitted record, so the session reports the number of
/// calls it made without its apportioned records counting as separate sessions.
struct TranscriptTurn {
    timestamp: i64,
    context_weight: u128,
    output_weight: u128,
    replies: i32,
}

/// Ceiling on the transcript bytes read to shape one session's spend.
///
/// Apportioning is a refinement, not a correctness requirement: the fallback
/// emits one record carrying the identical total, so declining to read an
/// oversized transcript costs attribution detail and nothing else. Cost grows
/// linearly with transcript size and is paid again on every scan while the
/// session is live, because the growing transcript keeps invalidating the
/// cached parse (measured on a 104.7 MB transcript: 148 ms per parse in
/// release, 1.28 s in debug, ~1.4 ms/MB). This ceiling bounds that per-session
/// work at roughly 45 ms in release.
const MAX_TRANSCRIPT_BYTES: u64 = 32 * 1024 * 1024;

/// Ceiling on the records one session contributes.
///
/// One record per assistant reply is unbounded in the session's length, and
/// every record is retained in the message cache. Past this many replies the
/// turns are coalesced into runs rather than truncated, so the apportioned
/// total is unchanged and only the resolution of the attribution drops.
const MAX_TURNS_PER_SESSION: usize = 1024;

/// Recover the shape of a session's spend from its transcript.
///
/// Droid records no token counts anywhere in the `*.jsonl` — only the
/// cumulative total in `*.settings.json` — so the transcript cannot say what a
/// reply cost. It can say *when* each reply happened and how much context was
/// live at the time, and cost per call tracks context length closely: nearly
/// every token in these sessions is a cache read of the conversation so far.
/// Weighting each assistant reply by the bytes accumulated since the last
/// compaction therefore recovers the relative cost of one reply against
/// another, which is all the apportioning below needs.
///
/// Only assistant replies are counted, since one reply corresponds to one API
/// call; user and tool records ride along inside the call that reads them. Each
/// reply carries two weights: the context standing *before* it, which is what
/// the call read, and its own bytes, which is what the call wrote. Compaction
/// resets the running context because it discards the transcript the following
/// calls would otherwise re-read.
///
/// Returns no turns for a transcript past `max_bytes`, or for one that could
/// not be read whole, which sends the session down the single-record fallback
/// rather than paying an unbounded read or splitting on partial evidence.
fn transcript_turns_bounded(jsonl: &Path, max_bytes: u64, max_turns: usize) -> Vec<TranscriptTurn> {
    let Ok(file) = std::fs::File::open(jsonl) else {
        return Vec::new();
    };
    // Fail closed. A metadata call that does not answer says nothing about the
    // size of what follows, and treating that silence as zero would license
    // exactly the unbounded read this ceiling exists to prevent. The session
    // still reports its identical total through the single-record fallback.
    let Ok(declared_bytes) = file.metadata().map(|meta| meta.len()) else {
        return Vec::new();
    };
    if declared_bytes > max_bytes {
        return Vec::new();
    }

    let mut turns = Vec::new();
    let mut context_bytes: u128 = 0;
    let mut reader = BufReader::new(file);

    for line in lossy_lines(&mut reader) {
        let line_bytes = line.len() as u128;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            // A malformed line still occupied context in the real conversation.
            context_bytes = context_bytes.saturating_add(line_bytes);
            continue;
        };

        let record_type = record.get("type").and_then(|v| v.as_str());
        if record_type == Some("compaction_state") {
            context_bytes = line_bytes;
            continue;
        }

        // What the call read is the conversation as it stood *before* this
        // record. A reply's own bytes are its output; charging them back to the
        // same reply would let a long answer inflate its own share of the
        // input and cache-read totals. They join the context for later calls.
        let context_read = context_bytes;
        context_bytes = context_bytes.saturating_add(line_bytes);

        if record_type != Some("message") {
            continue;
        }
        if record
            .get("message")
            .and_then(|message| message.get("role"))
            .and_then(|role| role.as_str())
            != Some("assistant")
        {
            continue;
        }

        // A non-positive timestamp is the parsers' "no usable time" sentinel,
        // not an instant before 1970 — `rebucket_date` refuses to re-key one.
        // The RFC3339 path has no such floor of its own, so a pre-epoch
        // transcript timestamp would otherwise anchor a share of the session in
        // 1969, where no date filter reaches it. Skipping the turn keeps its
        // tokens in the session: apportioning runs over the turns that remain.
        let Some(timestamp) = record
            .get("timestamp")
            .and_then(parse_timestamp_value)
            .filter(|&ts| ts > 0)
        else {
            continue;
        };

        turns.push(TranscriptTurn {
            timestamp,
            // The first reply in a session reads a context of zero bytes but
            // still cost something, so every turn carries at least one unit.
            context_weight: context_read.max(1),
            output_weight: line_bytes.max(1),
            replies: 1,
        });
    }

    // `lossy_lines` ends silently on a hard I/O failure (vanished network
    // mount, EIO), so a prefix of the transcript is indistinguishable from the
    // whole of it by the turn list alone. Apportioning the session's total over
    // that prefix would move every token the unread tail earned onto the days
    // the prefix covers, and the parse is cached under a fingerprint that a
    // finished session never invalidates again, so the misattribution would
    // stick. A short read is therefore no evidence at all: fall back.
    // Reading past `declared_bytes` is normal — a live session appends while
    // this runs — and only a read that stopped early disqualifies the split.
    if reader.stream_position().unwrap_or(0) < declared_bytes {
        return Vec::new();
    }

    coalesce_turns(turns, max_turns)
}

fn transcript_turns(jsonl: &Path) -> Vec<TranscriptTurn> {
    transcript_turns_bounded(jsonl, MAX_TRANSCRIPT_BYTES, MAX_TURNS_PER_SESSION)
}

/// Fold consecutive turns into at most `max_turns` runs, summing their weights.
///
/// Coalescing rather than truncating keeps the apportioned total intact: every
/// turn's weight still competes for the session's tokens, so no usage is
/// dropped and the parts still sum to what Droid recorded. A run reports at its
/// latest reply, which is where most of its weight sits — context grows
/// monotonically within a run, so the last reply of the run is the heaviest.
/// The parser cannot bucket by day itself: the day key is re-derived later
/// under the user's pinned timezone, which is not known here.
///
/// A run also carries the number of replies it folded, so the session's message
/// count survives coalescing: the resolution of *when* the calls happened
/// drops, but not *how many* there were.
fn coalesce_turns(turns: Vec<TranscriptTurn>, max_turns: usize) -> Vec<TranscriptTurn> {
    if max_turns == 0 || turns.len() <= max_turns {
        return turns;
    }

    let run = turns.len().div_ceil(max_turns);
    turns
        .chunks(run)
        .map(|chunk| TranscriptTurn {
            timestamp: chunk
                .iter()
                .map(|turn| turn.timestamp)
                .max()
                .unwrap_or_default(),
            context_weight: chunk
                .iter()
                .fold(0u128, |sum, turn| sum.saturating_add(turn.context_weight)),
            output_weight: chunk
                .iter()
                .fold(0u128, |sum, turn| sum.saturating_add(turn.output_weight)),
            replies: chunk
                .iter()
                .fold(0i32, |sum, turn| sum.saturating_add(turn.replies)),
        })
        .collect()
}

/// Split `total` across `weights` so the parts sum back to exactly `total`.
///
/// Allocates on the running total rather than rounding each share on its own:
/// each part is the difference between two cumulative shares, so truncation
/// error is carried forward instead of accumulating, and the final cumulative
/// share is `total` by construction. That exactness is what lets a session be
/// split across days without the daily figures drifting from the cumulative
/// number Droid actually recorded.
fn apportion(total: i64, weights: &[u128], total_weight: u128) -> Vec<i64> {
    if total_weight == 0 {
        return vec![0; weights.len()];
    }

    let mut parts = Vec::with_capacity(weights.len());
    let mut consumed_weight: u128 = 0;
    let mut allocated: i64 = 0;

    for weight in weights {
        consumed_weight += weight;
        let up_to = ((total as i128 * consumed_weight as i128) / total_weight as i128) as i64;
        parts.push(up_to - allocated);
        allocated = up_to;
    }

    parts
}

/// Parse a Droid settings.json file
pub fn parse_droid_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(data) = read_file_or_none(path) else {
        return Vec::new();
    };

    let mut bytes = data;
    let settings: DroidSettingsJson = match simd_json::from_slice(&mut bytes) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    // Skip if no token usage data
    let usage = match settings.token_usage {
        Some(u) => u,
        None => return Vec::new(),
    };

    // Calculate total tokens to check if any were used
    let total_tokens = usage.input_tokens.unwrap_or(0)
        + usage.output_tokens.unwrap_or(0)
        + usage.cache_creation_tokens.unwrap_or(0)
        + usage.cache_read_tokens.unwrap_or(0)
        + usage.thinking_tokens.unwrap_or(0);

    if total_tokens == 0 {
        return Vec::new();
    }

    // Extract session ID from filename (e.g., "uuid.settings.json" -> "uuid")
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
        .replace(".settings", "");

    // Get model and provider
    let provider = settings.provider_lock.clone().unwrap_or_else(|| {
        get_provider_from_model(settings.model.as_deref().unwrap_or("")).to_string()
    });

    let model = if let Some(m) = settings.model {
        normalize_model_name(&m)
    } else {
        // Try to extract from JSONL file
        let jsonl_path = droid_jsonl_path(path);

        if let Some(ref jsonl) = jsonl_path {
            extract_model_from_jsonl(jsonl)
                .unwrap_or_else(|| get_default_model_from_provider(&provider))
        } else {
            get_default_model_from_provider(&provider)
        }
    };

    let totals = TokenBreakdown {
        input: usage.input_tokens.unwrap_or(0).max(0),
        output: usage.output_tokens.unwrap_or(0).max(0),
        cache_read: usage.cache_read_tokens.unwrap_or(0).max(0),
        cache_write: usage.cache_creation_tokens.unwrap_or(0).max(0),
        reasoning: usage.thinking_tokens.unwrap_or(0).max(0),
    };

    let turns = droid_jsonl_path(path)
        .map(|jsonl| transcript_turns(&jsonl))
        .unwrap_or_default();
    // Read cost and write cost are apportioned on different weights. Input,
    // cache reads and cache writes are all charged for the conversation the
    // call had to send, so they follow the context standing before the reply.
    // Output and reasoning are what the call produced, so they follow the
    // reply's own size — a verbose answer on a short context earns its output
    // share without also claiming the input share of the replies around it.
    let context_weights: Vec<u128> = turns.iter().map(|turn| turn.context_weight).collect();
    let output_weights: Vec<u128> = turns.iter().map(|turn| turn.output_weight).collect();
    let total_context: u128 = context_weights.iter().sum();
    let total_output: u128 = output_weights.iter().sum();

    // Every weight is `.max(1)`, so a non-empty turn list always has positive
    // weight in both columns and the apportioning below is well defined.
    if !turns.is_empty() {
        let input = apportion(totals.input, &context_weights, total_context);
        let cache_read = apportion(totals.cache_read, &context_weights, total_context);
        let cache_write = apportion(totals.cache_write, &context_weights, total_context);
        let output = apportion(totals.output, &output_weights, total_output);
        let reasoning = apportion(totals.reasoning, &output_weights, total_output);

        // One Droid session made this many API calls, however many records its
        // spend is spread over.
        let session_replies = turns
            .iter()
            .fold(0i32, |sum, turn| sum.saturating_add(turn.replies.max(1)));

        return turns
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                let mut message = UnifiedMessage::new(
                    "droid",
                    model.clone(),
                    provider.clone(),
                    session_id.clone(),
                    turn.timestamp,
                    TokenBreakdown {
                        input: input[index],
                        output: output[index],
                        cache_read: cache_read[index],
                        cache_write: cache_write[index],
                        reasoning: reasoning[index],
                    },
                    0.0,
                );
                // These records are attribution fragments of one session, not
                // separate sessions. `sessionize` opens a new interval whenever
                // two records sit more than the idle gap apart and counts every
                // interval that reports messages, so a count on each fragment
                // would turn one Droid session into one session per apportioned
                // record. Carry the session's authoritative reply total on
                // exactly one record and make the rest count-neutral, the same
                // split `copilot_desktop` applies to its shutdown fragments:
                // the total number of calls stays exact, the session count
                // stays one.
                message.message_count = if index == 0 { session_replies } else { 0 };
                message
            })
            .collect();
    }

    // No usable transcript (missing, unreadable, or no assistant replies yet).
    // Fall back to one record for the whole session, anchored at the last write
    // so an active session still lands in the present rather than at the
    // instant its provider was locked.
    let lock_timestamp = parse_lock_timestamp(settings.provider_lock_timestamp.as_deref());
    let timestamp = resolve_usage_timestamp(lock_timestamp, file_modified_timestamp_ms_opt(path));

    vec![UnifiedMessage::new(
        "droid", model, provider, session_id, timestamp, totals, 0.0,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_model_name_custom_prefix() {
        // TypeScript keeps trailing digits: "claude-opus-4-5-thinking-0"
        assert_eq!(
            normalize_model_name("custom:Claude-Opus-4.5-Thinking-[Anthropic]-0"),
            "claude-opus-4-5-thinking-0"
        );
    }

    #[test]
    fn test_normalize_model_name_simple() {
        // Dots become hyphens: "gemini-2.5-pro" -> "gemini-2-5-pro"
        assert_eq!(normalize_model_name("gemini-2.5-pro"), "gemini-2-5-pro");
    }

    #[test]
    fn test_normalize_model_name_brackets() {
        // TypeScript keeps trailing digits: "claude-sonnet-4"
        assert_eq!(
            normalize_model_name("Claude-Sonnet-4-[Anthropic]"),
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_get_provider_from_model() {
        assert_eq!(get_provider_from_model("claude-3-sonnet"), "anthropic");
        assert_eq!(get_provider_from_model("opus-4"), "anthropic");
        assert_eq!(get_provider_from_model("sonnet-4"), "anthropic");
        assert_eq!(get_provider_from_model("haiku-3"), "anthropic");
        assert_eq!(get_provider_from_model("gpt-4o"), "openai");
        assert_eq!(get_provider_from_model("o1-preview"), "openai");
        assert_eq!(get_provider_from_model("o3-mini"), "openai");
        assert_eq!(get_provider_from_model("gemini-pro"), "google");
        assert_eq!(get_provider_from_model("grok-2"), "xai");
        assert_eq!(get_provider_from_model("unknown-model"), "unknown");
    }

    #[test]
    fn test_get_default_model_from_provider() {
        assert_eq!(
            get_default_model_from_provider("anthropic"),
            "claude-unknown"
        );
        assert_eq!(get_default_model_from_provider("openai"), "gpt-unknown");
        assert_eq!(get_default_model_from_provider("google"), "gemini-unknown");
        assert_eq!(get_default_model_from_provider("xai"), "grok-unknown");
        assert_eq!(get_default_model_from_provider("custom"), "custom-unknown");
    }

    fn write_session(dir: &Path, id: &str, usage: &str, transcript: &[&str]) -> PathBuf {
        let settings = dir.join(format!("{id}.settings.json"));
        std::fs::write(
            &settings,
            format!(r#"{{"model":"custom:Kimi-K3-0","tokenUsage":{usage}}}"#),
        )
        .unwrap();
        std::fs::write(dir.join(format!("{id}.jsonl")), transcript.join("\n")).unwrap();
        settings
    }

    fn assistant(timestamp: &str, filler: usize) -> String {
        format!(
            r#"{{"type":"message","timestamp":"{timestamp}","message":{{"role":"assistant","content":"{}"}}}}"#,
            "x".repeat(filler)
        )
    }

    fn user(timestamp: &str, filler: usize) -> String {
        format!(
            r#"{{"type":"message","timestamp":"{timestamp}","message":{{"role":"user","content":"{}"}}}}"#,
            "x".repeat(filler)
        )
    }

    #[test]
    fn test_apportion_parts_sum_to_the_original_total() {
        // Truncation must never lose or invent tokens: the daily figures have to
        // add back up to the cumulative number Droid recorded.
        let weights = vec![3u128, 1, 1, 1, 1];
        let parts = apportion(1_000_003, &weights, weights.iter().sum());

        assert_eq!(parts.len(), 5);
        assert_eq!(parts.iter().sum::<i64>(), 1_000_003);
        assert!(parts.iter().all(|&p| p >= 0));
        // The heaviest turn gets the largest share.
        assert!(parts[0] > parts[1]);
    }

    #[test]
    fn test_apportion_without_weight_yields_no_tokens() {
        assert_eq!(apportion(500, &[0, 0], 0), vec![0, 0]);
    }

    #[test]
    fn test_usage_spreads_across_the_days_the_session_ran() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // Two assistant replies a day apart, the second on a larger context.
        let settings = write_session(
            temp_dir.path(),
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            r#"{"inputTokens":900,"outputTokens":100}"#,
            &[
                r#"{"type":"session_start","id":"s"}"#,
                &assistant("2026-08-07T12:00:00Z", 10),
                &assistant("2026-08-09T12:00:00Z", 400),
            ],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 2, "one record per assistant reply");
        // Nothing is lost or invented by the split.
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 900);
        assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 100);
        // Each record lands on the reply that earned it, not on one anchor.
        assert!(messages[0].timestamp < messages[1].timestamp);
        assert_ne!(messages[0].date, messages[1].date);
        // The later reply read a longer conversation, so it carries more.
        assert!(messages[1].tokens.input > messages[0].tokens.input);
    }

    #[test]
    fn test_a_long_reply_does_not_charge_itself_for_its_own_output() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // Two replies on an identical context, one of them enormously long.
        // What each call *read* is the same, so the split must be even: the
        // long answer's own bytes are output, not context it paid to read.
        let settings = write_session(
            temp_dir.path(),
            "44444444-4444-4444-4444-444444444444",
            r#"{"inputTokens":1000}"#,
            &[
                r#"{"type":"message","timestamp":"2026-08-07T11:00:00Z","message":{"role":"user","content":"seed"}}"#,
                &assistant("2026-08-07T12:00:00Z", 50_000),
                &assistant("2026-08-07T13:00:00Z", 10),
            ],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 1000);
        // The first reply read only the short seed; the second read that plus a
        // 50KB answer, so it should carry overwhelmingly more. Charging a reply
        // for its own output would instead hand the first one that same 50KB
        // and leave the two nearly equal, so the ratio is what discriminates.
        assert!(
            messages[1].tokens.input > messages[0].tokens.input * 10,
            "a long reply inflated its own share: {:?}",
            messages.iter().map(|m| m.tokens.input).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_output_follows_the_replys_own_size_not_the_context_it_read() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // Two replies reading the same conversation, one of them far longer.
        // They read the same bytes, so the input split is even; the long one
        // produced far more, so it must carry far more of the output total.
        let settings = write_session(
            temp_dir.path(),
            "55555555-5555-5555-5555-555555555555",
            r#"{"inputTokens":1000,"outputTokens":1000,"thinkingTokens":1000}"#,
            &[
                &user("2026-08-07T11:00:00Z", 4000),
                &assistant("2026-08-07T12:00:00Z", 10),
                &assistant("2026-08-07T13:00:00Z", 10),
            ],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 1000);
        // The second reply reads the first one's bytes too, but 10 bytes on top
        // of a 4KB context barely moves the input split.
        let input_ratio = messages[1].tokens.input as f64 / messages[0].tokens.input as f64;
        assert!(
            input_ratio < 1.1,
            "identical context should split input evenly, got {:?}",
            messages.iter().map(|m| m.tokens.input).collect::<Vec<_>>()
        );
        // Same-length replies split output evenly here; the discriminating case
        // is below, where one reply is much longer than the other.
        assert_eq!(messages[0].tokens.output, messages[1].tokens.output);

        let settings = write_session(
            temp_dir.path(),
            "66666666-6666-6666-6666-666666666666",
            r#"{"inputTokens":1000,"outputTokens":1000,"thinkingTokens":1000}"#,
            &[
                &user("2026-08-07T11:00:00Z", 4000),
                &assistant("2026-08-07T12:00:00Z", 50_000),
                &assistant("2026-08-07T13:00:00Z", 10),
            ],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 1000);
        assert_eq!(
            messages.iter().map(|m| m.tokens.reasoning).sum::<i64>(),
            1000
        );
        assert!(
            messages[0].tokens.output > messages[1].tokens.output * 10,
            "the long reply should carry the output it produced: {:?}",
            messages.iter().map(|m| m.tokens.output).collect::<Vec<_>>()
        );
        assert!(
            messages[0].tokens.reasoning > messages[1].tokens.reasoning * 10,
            "reasoning is produced alongside output: {:?}",
            messages
                .iter()
                .map(|m| m.tokens.reasoning)
                .collect::<Vec<_>>()
        );
        // ...while the input split still follows what each call read, so the
        // long reply does not also take the other's input share.
        assert!(
            messages[1].tokens.input > messages[0].tokens.input,
            "input must still follow the context each call read: {:?}",
            messages.iter().map(|m| m.tokens.input).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_pre_epoch_transcript_timestamp_is_not_an_attribution_anchor() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // A clock artifact in the transcript. Anchoring a share of the session
        // on it buckets those tokens in 1969, past every date filter.
        let settings = write_session(
            temp_dir.path(),
            "77777777-7777-7777-7777-777777777777",
            r#"{"inputTokens":900,"outputTokens":100}"#,
            &[
                &assistant("1969-07-20T20:17:00Z", 10),
                &assistant("2026-08-09T12:00:00Z", 10),
            ],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(
            messages.len(),
            1,
            "the pre-epoch reply is not a usable turn"
        );
        assert!(messages.iter().all(|m| m.timestamp > 0));
        // Nothing is dropped: the surviving turn carries the whole session.
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 900);
        assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 100);
    }

    #[test]
    fn test_transcript_of_only_pre_epoch_replies_falls_back_to_one_record() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_session(
            temp_dir.path(),
            "88888888-8888-8888-8888-888888888888",
            r#"{"inputTokens":900}"#,
            &[&assistant("1969-07-20T20:17:00Z", 10)],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 900);
        assert!(
            messages[0].timestamp > 0,
            "the fallback anchor must stay in a reachable bucket"
        );
    }

    #[test]
    fn test_oversized_transcript_takes_the_single_record_path() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_session(
            temp_dir.path(),
            "99999999-9999-9999-9999-999999999999",
            r#"{"inputTokens":900}"#,
            &[
                &assistant("2026-08-07T12:00:00Z", 200),
                &assistant("2026-08-09T12:00:00Z", 200),
            ],
        );
        let jsonl = droid_jsonl_path(&settings).unwrap();
        let size = std::fs::metadata(&jsonl).unwrap().len();

        // Under the ceiling the transcript still shapes the session.
        assert_eq!(
            transcript_turns_bounded(&jsonl, size, MAX_TURNS_PER_SESSION).len(),
            2
        );
        // Past it, no turns — which is what routes the session to the
        // mtime-anchored single record instead of an unbounded read.
        assert!(transcript_turns_bounded(&jsonl, size - 1, MAX_TURNS_PER_SESSION).is_empty());
    }

    #[test]
    fn test_turn_count_is_capped_without_losing_usage() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let transcript: Vec<String> = (0..50)
            .map(|i| {
                assistant(
                    &format!("2026-08-{:02}T{:02}:00:00Z", 1 + i / 24, i % 24),
                    10,
                )
            })
            .collect();
        let settings = write_session(
            temp_dir.path(),
            "aaaaaaaa-0000-0000-0000-000000000000",
            r#"{"inputTokens":1000}"#,
            &transcript.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let jsonl = droid_jsonl_path(&settings).unwrap();

        let uncapped = transcript_turns_bounded(&jsonl, MAX_TRANSCRIPT_BYTES, usize::MAX);
        assert_eq!(uncapped.len(), 50);

        let capped = transcript_turns_bounded(&jsonl, MAX_TRANSCRIPT_BYTES, 7);
        assert!(capped.len() <= 7, "cap not honoured: {}", capped.len());
        // Coalescing folds weight rather than discarding it, so the session
        // still apportions the same total across the runs that remain.
        assert_eq!(
            capped.iter().map(|turn| turn.context_weight).sum::<u128>(),
            uncapped
                .iter()
                .map(|turn| turn.context_weight)
                .sum::<u128>()
        );
        assert_eq!(
            capped.iter().map(|turn| turn.output_weight).sum::<u128>(),
            uncapped.iter().map(|turn| turn.output_weight).sum::<u128>()
        );
        // Every run reports at a real reply instant inside the session.
        assert!(capped.iter().all(|turn| turn.timestamp > 0));
        assert!(capped.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
        // A run also remembers how many calls it stands for, so the cap costs
        // attribution resolution and not the session's message count.
        assert_eq!(capped.iter().map(|turn| turn.replies).sum::<i32>(), 50);
        assert_eq!(uncapped.iter().map(|turn| turn.replies).sum::<i32>(), 50);
    }

    #[test]
    fn test_coalesced_session_still_reports_every_reply_it_made() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // More replies than the cap, so the emitted records are runs rather
        // than calls. Counting a run as one message would report this session
        // as having made 1024 calls instead of the 1200 it made.
        let replies = MAX_TURNS_PER_SESSION + 176;
        let transcript: Vec<String> = (0..replies)
            .map(|i| assistant(&format!("2026-08-07T12:00:{:02}Z", i % 60), 10))
            .collect();
        let settings = write_session(
            temp_dir.path(),
            "aaaaaaaa-1111-1111-1111-111111111111",
            r#"{"inputTokens":1000}"#,
            &transcript.iter().map(String::as_str).collect::<Vec<_>>(),
        );

        let messages = parse_droid_file(&settings);

        assert!(
            messages.len() <= MAX_TURNS_PER_SESSION,
            "record count not capped: {}",
            messages.len()
        );
        assert_eq!(
            messages
                .iter()
                .map(|m| m.message_count as usize)
                .sum::<usize>(),
            replies
        );
        // The cap changes resolution, never the total it splits.
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 1000);
    }

    #[test]
    fn test_reply_count_rides_on_exactly_one_record() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_session(
            temp_dir.path(),
            "aaaaaaaa-2222-2222-2222-222222222222",
            r#"{"inputTokens":1000}"#,
            &[
                &assistant("2026-08-07T12:00:00Z", 10),
                &assistant("2026-08-08T12:00:00Z", 10),
            ],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 2);
        // Both calls are counted, but only one record is countable: the second
        // is an attribution fragment of the same session, not a new session.
        assert_eq!(messages.iter().map(|m| m.message_count).sum::<i32>(), 2);
        assert_eq!(messages.iter().filter(|m| m.message_count > 0).count(), 1);
    }

    #[test]
    fn test_apportioned_session_still_sessionizes_as_one_session() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // 40 replies ten minutes apart: every gap exceeds the idle threshold,
        // so each apportioned record would open its own session interval. The
        // session count must stay 1 while all 40 calls are still reported.
        let replies = 40;
        let transcript: Vec<String> = (0..replies)
            .map(|i| {
                assistant(
                    &format!(
                        "2026-08-07T{:02}:{:02}:00Z",
                        6 + (i * 10) / 60,
                        (i * 10) % 60
                    ),
                    10,
                )
            })
            .collect();
        let settings = write_session(
            temp_dir.path(),
            "aaaaaaaa-3333-3333-3333-333333333333",
            r#"{"inputTokens":4000,"outputTokens":400}"#,
            &transcript.iter().map(String::as_str).collect::<Vec<_>>(),
        );

        let messages = parse_droid_file(&settings);
        assert_eq!(messages.len(), replies as usize);
        assert_eq!(
            messages.iter().map(|m| m.message_count).sum::<i32>(),
            replies
        );

        let intervals =
            crate::sessionize::sessionize(&messages, crate::sessionize::DEFAULT_IDLE_GAP_MS);
        let metrics = crate::sessionize::compute_time_metrics(
            &intervals,
            crate::sessionize::DEFAULT_IDLE_GAP_MS,
        );
        assert_eq!(metrics.session_count, 1);
        // The totals the session actually recorded are untouched by the split.
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 4000);
        assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 400);
    }

    #[test]
    fn test_compaction_resets_the_context_weighting() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // A big conversation, compacted, then one short reply afterwards. The
        // post-compaction reply must not inherit the discarded context's weight.
        let settings = write_session(
            temp_dir.path(),
            "11111111-1111-1111-1111-111111111111",
            r#"{"inputTokens":1000}"#,
            &[
                // A long conversation the first reply had to read...
                &user("2026-08-07T11:00:00Z", 4000),
                &assistant("2026-08-07T12:00:00Z", 10),
                // ...then compaction discards it, so the reply after it reads
                // only the compacted summary.
                r#"{"type":"compaction_state","timestamp":"2026-08-08T12:00:00Z"}"#,
                &assistant("2026-08-09T12:00:00Z", 10),
            ],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 1000);
        assert!(
            messages[0].tokens.input > messages[1].tokens.input,
            "the compacted-away context should not keep charging later replies: {:?}",
            messages.iter().map(|m| m.tokens.input).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_session_without_assistant_replies_falls_back_to_one_record() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        // Usage recorded but the transcript has no assistant reply to hang it
        // on — the record must still be emitted rather than silently dropped.
        let settings = write_session(
            temp_dir.path(),
            "22222222-2222-2222-2222-222222222222",
            r#"{"inputTokens":700,"outputTokens":7}"#,
            &[r#"{"type":"session_start","id":"s"}"#],
        );

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 700);
        assert_eq!(messages[0].tokens.output, 7);
    }

    #[test]
    fn test_missing_transcript_falls_back_to_one_record() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = temp_dir
            .path()
            .join("33333333-3333-3333-3333-333333333333.settings.json");
        std::fs::write(
            &settings,
            r#"{"model":"custom:Kimi-K3-0","tokenUsage":{"inputTokens":42}}"#,
        )
        .unwrap();

        let messages = parse_droid_file(&settings);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 42);
    }

    #[test]
    fn test_parse_lock_timestamp_rejects_unusable_anchors() {
        // Zero is Droid's unset sentinel.
        assert_eq!(parse_lock_timestamp(Some("1970-01-01T00:00:00Z")), None);
        // A pre-epoch lock is a clock/corruption artifact. Keeping it would
        // outlive the mtime fallback and bucket the record in 1969, where no
        // date filter can reach it.
        assert_eq!(parse_lock_timestamp(Some("1969-07-20T20:17:00Z")), None);
        assert_eq!(parse_lock_timestamp(Some("not-a-timestamp")), None);
        assert_eq!(parse_lock_timestamp(None), None);

        assert_eq!(
            parse_lock_timestamp(Some("2026-08-07T03:32:46.663Z")),
            Some(1_786_073_566_663)
        );
    }

    #[test]
    fn test_pre_epoch_lock_falls_through_to_now_without_mtime() {
        // The resolver only reaches its now() fallback if the rejected lock
        // arrives as None, so this pins the two halves together: an unusable
        // lock plus an unavailable mtime must not yield a pre-epoch anchor.
        let lock = parse_lock_timestamp(Some("1969-07-20T20:17:00Z"));

        assert!(resolve_usage_timestamp(lock, None) > 1_700_000_000_000);
    }

    #[test]
    fn test_resolve_usage_timestamp_prefers_mtime_over_stale_lock() {
        // The regression: a session locked its provider on day 1 and was still
        // spending tokens on day 4. Anchoring on the lock timestamp reported
        // all of it against day 1 and left the session invisible in --today.
        let lock = 1_700_000_000_000;
        let modified = lock + 3 * 86_400_000;

        assert_eq!(
            resolve_usage_timestamp(Some(lock), Some(modified)),
            modified
        );
    }

    #[test]
    fn test_resolve_usage_timestamp_floors_stale_mtime_at_lock() {
        // A copy or restore can rewind mtime below the instant the provider was
        // locked. Usage cannot predate provider selection, so the lock wins.
        let lock = 1_700_000_000_000;
        let modified = lock - 86_400_000;

        assert_eq!(resolve_usage_timestamp(Some(lock), Some(modified)), lock);
    }

    #[test]
    fn test_resolve_usage_timestamp_falls_back_across_missing_inputs() {
        let lock = 1_700_000_000_000;
        let modified = 1_700_000_500_000;

        // Droid omits providerLockTimestamp on plenty of sessions.
        assert_eq!(resolve_usage_timestamp(None, Some(modified)), modified);
        // Filesystem reported no mtime: the lock is still better than now().
        assert_eq!(resolve_usage_timestamp(Some(lock), None), lock);
    }

    #[test]
    fn test_resolve_usage_timestamp_without_any_anchor_is_not_pre_epoch() {
        // Neither anchor available: the record still carries real token usage,
        // so it must land in a present-day bucket rather than at the epoch.
        assert!(resolve_usage_timestamp(None, None) > 1_700_000_000_000);
    }

    #[test]
    fn test_parse_droid_file_anchors_long_running_session_at_last_write() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir
            .path()
            .join("11111111-2222-3333-4444-555555555555.settings.json");

        // providerLockTimestamp far in the past, totals written just now —
        // the shape of a session that has been looping for days.
        let lock_ms = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        std::fs::write(
            &path,
            r#"{
                "model": "custom:Kimi-K3-(free)-0",
                "providerLockTimestamp": "2024-01-01T00:00:00Z",
                "tokenUsage": { "inputTokens": 1000, "outputTokens": 200 }
            }"#,
        )
        .unwrap();

        let messages = parse_droid_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].session_id,
            "11111111-2222-3333-4444-555555555555"
        );
        assert!(
            messages[0].timestamp > lock_ms,
            "cumulative usage anchored on the stale lock timestamp ({lock_ms}), \
             got {}",
            messages[0].timestamp
        );
    }

    #[test]
    fn test_parse_droid_settings_structure() {
        let json = r#"{
            "model": "custom:Claude-Opus-4.5-Thinking-[Anthropic]-0",
            "providerLock": "anthropic",
            "providerLockTimestamp": "2024-12-26T12:00:00Z",
            "tokenUsage": {
                "inputTokens": 1234,
                "outputTokens": 567,
                "cacheCreationTokens": 89,
                "cacheReadTokens": 12,
                "thinkingTokens": 34
            }
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let settings: DroidSettingsJson = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(
            settings.model,
            Some("custom:Claude-Opus-4.5-Thinking-[Anthropic]-0".to_string())
        );
        assert_eq!(settings.provider_lock, Some("anthropic".to_string()));

        let usage = settings.token_usage.unwrap();
        assert_eq!(usage.input_tokens, Some(1234));
        assert_eq!(usage.output_tokens, Some(567));
        assert_eq!(usage.cache_creation_tokens, Some(89));
        assert_eq!(usage.cache_read_tokens, Some(12));
        assert_eq!(usage.thinking_tokens, Some(34));
    }
}
