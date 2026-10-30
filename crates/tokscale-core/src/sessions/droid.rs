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
use std::collections::HashMap;
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
    /// Last model Factory Router actually served when `model` is `auto`.
    #[serde(rename = "effectiveFactoryRouterModel")]
    pub effective_factory_router_model: Option<DroidRouterModel>,
}

#[derive(Debug, Deserialize)]
pub struct DroidRouterModel {
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
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

fn is_routing_model(model: &str) -> bool {
    crate::pricing::lookup::is_routing_label(model)
}

/// Normalized model id, or `None` when the value is empty or a router label.
/// The second string is the original text, kept for pricing lookup.
fn concrete_model(raw: &str) -> Option<(String, String)> {
    let normalized = normalize_model_name(raw);
    if normalized.is_empty() || is_routing_model(&normalized) {
        None
    } else {
        Some((normalized, raw.to_string()))
    }
}

/// Provider for a resolved underlying model. Infer from the model id first so
/// Factory Router's `providerLock: "factory"` does not hide GLM/Kimi/Claude
/// as a generic Factory row; the lock is only the unknown-family fallback.
fn provider_for_resolved_model(model: &str, lock: Option<&str>) -> String {
    let inferred = get_provider_from_model(model);
    if inferred != "unknown" {
        inferred.to_string()
    } else {
        lock.filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| inferred.to_string())
    }
}

/// Session-level model, whether `settings.model` is a router label, and the
/// raw string pricing should see.
///
/// Auto sessions keep their configured label only as a last resort: the last
/// routed model (`effectiveFactoryRouterModel`) and the JSONL `Model:` reminder
/// are both more specific. Per-turn `message.modelId` is applied later and
/// wins over this fallback for every session, including an explicit
/// `settings.model`. Droid rewrites that field to whatever is selected *now*,
/// so it is only the label for calls the transcript never named.
fn session_model_and_router(settings: &DroidSettingsJson, path: &Path) -> (String, bool, String) {
    match settings.model.as_deref() {
        Some(raw) => {
            let normalized = normalize_model_name(raw);
            if !is_routing_model(&normalized) {
                return (normalized, false, raw.to_string());
            }
            if let Some((routed, pricing_raw)) = settings
                .effective_factory_router_model
                .as_ref()
                .and_then(|router| router.model_id.as_deref())
                .and_then(concrete_model)
            {
                return (routed, true, pricing_raw);
            }
            if let Some((from_jsonl, pricing_raw)) = droid_jsonl_path(path)
                .as_ref()
                .and_then(|jsonl| extract_model_from_jsonl(jsonl))
                .as_deref()
                .and_then(concrete_model)
            {
                return (from_jsonl, true, pricing_raw);
            }
            (normalized, true, raw.to_string())
        }
        None => {
            let provider = settings.provider_lock.as_deref().unwrap_or("");
            let pricing_raw = droid_jsonl_path(path)
                .as_ref()
                .and_then(|jsonl| extract_model_from_jsonl(jsonl))
                .unwrap_or_else(|| {
                    get_default_model_from_provider(if provider.is_empty() {
                        "unknown"
                    } else {
                        provider
                    })
                });
            (normalize_model_name(&pricing_raw), false, pricing_raw)
        }
    }
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
                return Some(model_name.to_string());
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
    /// Concrete `message.modelId` from this assistant reply, when present.
    model_raw: Option<String>,
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
/// work at roughly 45 ms in release for the original 32 MB bound.
///
/// The bound is 128 MB. A switched session past 32 MB used to skip this
/// split and stamp the whole cumulative total with `settings.model`, which
/// is the last model the user selected. 128 MB is about 180 ms at the
/// measured 1.4 ms/MB, and it still refuses a pathological transcript.
const MAX_TRANSCRIPT_BYTES: u64 = 128 * 1024 * 1024;

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

        let model_raw = record
            .get("message")
            .and_then(|message| message.get("modelId"))
            .and_then(|value| value.as_str())
            .filter(|raw| concrete_model(raw).is_some())
            .map(str::to_string);

        turns.push(TranscriptTurn {
            timestamp,
            // The first reply in a session reads a context of zero bytes but
            // still cost something, so every turn carries at least one unit.
            context_weight: context_read.max(1),
            output_weight: line_bytes.max(1),
            replies: 1,
            model_raw,
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

    // A real model call is the assistant record that carries `message.modelId`.
    // Droid also writes `repl-tool-call:` rows (and other tool echoes) as
    // assistant messages with no model. Counting those as calls hands them a
    // share of the cumulative total, and the share falls through to
    // `settings.model` — the model selected at the end of the session — so a
    // switch rewrites the earlier spend. When the transcript names any call,
    // keep only those calls. The dropped rows already advanced `context_bytes`,
    // so the next real call still reads them. A transcript with no modelId at
    // all keeps every assistant reply, which is the only shape older sessions
    // have.
    if turns.iter().any(|turn| turn.model_raw.is_some()) {
        turns.retain(|turn| turn.model_raw.is_some());
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
///
/// Callers never pass turns of more than one model: the last reply would
/// otherwise relabel the whole run.
fn fold_turn_run<'a>(chunk: impl IntoIterator<Item = &'a TranscriptTurn>) -> TranscriptTurn {
    let mut folded = TranscriptTurn {
        timestamp: 0,
        context_weight: 0,
        output_weight: 0,
        replies: 0,
        model_raw: None,
    };
    let mut timestamp: Option<i64> = None;
    let mut last: Option<&TranscriptTurn> = None;
    for turn in chunk {
        timestamp = Some(timestamp.map_or(turn.timestamp, |ts| ts.max(turn.timestamp)));
        folded.context_weight = folded.context_weight.saturating_add(turn.context_weight);
        folded.output_weight = folded.output_weight.saturating_add(turn.output_weight);
        folded.replies = folded.replies.saturating_add(turn.replies);
        last = Some(turn);
    }
    folded.timestamp = timestamp.unwrap_or_default();
    folded.model_raw = last.and_then(|turn| turn.model_raw.clone());
    folded
}

/// Cap a session's turns at `max_turns` records without mixing models.
///
/// A record never folds turns of two models, and it only folds turns of
/// different runs of one model when nothing else fits. A run is a stretch of
/// consecutive turns of one model.
///
/// When the session has at most `max_turns` runs, each run is cut into chunks
/// of `chunk` consecutive turns, and no record crosses a run boundary.
/// `chunk` is the smallest size that fits, starting at
/// `turns.len() / max_turns` (rounded up). A session that already fit at that
/// starting size keeps exactly the records it had before. The record count
/// only goes down as `chunk` grows, and one chunk per run always fits, so a
/// binary search finds the smallest `chunk`.
///
/// A session with more runs than `max_turns` cannot keep runs apart, because
/// every run needs a record of its own. A session that alternates models on
/// every reply is this case. Its turns are cut into windows of consecutive
/// turns, and each window emits one record per model it contains. That record
/// count does not always go down as the window grows: a wider window can
/// straddle more model switches. `fitting_window_width` picks a width that
/// fits with a bounded number of probes; see there.
///
/// A session that used more distinct models than `max_turns` cannot fit
/// without mixing models, so it gets one record per model.
///
/// Every path only regroups turns. Each model's summed weights and reply count
/// are the same before and after, so each model keeps its exact share of the
/// session's tokens and calls.
fn coalesce_turns(turns: Vec<TranscriptTurn>, max_turns: usize) -> Vec<TranscriptTurn> {
    if max_turns == 0 || turns.len() <= max_turns {
        return turns;
    }

    let smallest = turns.len().div_ceil(max_turns).max(1);
    let runs = model_runs(&turns);
    if runs.len() <= max_turns {
        let records = |chunk: usize| -> usize {
            runs.iter()
                .map(|run| (run.end - run.start).div_ceil(chunk))
                .sum()
        };
        // One chunk per run is `runs.len()` records, which fits, and the
        // longest run is at least `smallest` turns long.
        let mut lo = smallest;
        let mut hi = runs
            .iter()
            .map(|run| run.end - run.start)
            .max()
            .unwrap_or(1)
            .max(smallest);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if records(mid) <= max_turns {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        return runs
            .iter()
            .flat_map(|run| turns[run.clone()].chunks(hi).map(fold_turn_run))
            .collect();
    }

    let models = turns
        .iter()
        .map(|turn| turn.model_raw.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .len();
    if models > max_turns {
        return fold_turn_windows(&turns, turns.len());
    }
    let (width, _probes) = fitting_window_width(&turns, max_turns, models);
    fold_turn_windows(&turns, width)
}

/// Extra widths probed below the first ladder step that fits, to find a
/// narrower width that also fits.
const WINDOW_REFINE_PROBES: usize = 8;

/// A window width that fits `max_turns` records, and how many widths were
/// counted to find it.
///
/// The window record count is not monotonic in the width, so the narrowest
/// fitting width cannot be found by bisection, and trying every width costs
/// O(turns²/`max_turns`). Instead the widths form a doubling ladder from
/// `smallest` up to `fits_anyway`. A window of `2w` turns is exactly two
/// windows of `w`, so its count is never higher, and the first ladder step
/// that fits is the narrowest ladder step that fits. `fits_anyway` is a width
/// whose windows are few enough to fit even if every window holds every
/// model, so the ladder always ends in a fit. Then up to
/// `WINDOW_REFINE_PROBES` widths spread evenly between `smallest` and that
/// step are tried, and the narrowest of them that fits wins. When the gap is
/// at most `WINDOW_REFINE_PROBES` widths, every width in it is tried, so a
/// short session still gets its narrowest fitting width.
///
/// This is not always the narrowest width that fits, but it always fits, and
/// the work is bounded: at most `log2(fits_anyway / smallest) + 2` ladder
/// probes plus `WINDOW_REFINE_PROBES`, each one pass over the turns that stops
/// once its count passes the cap.
fn fitting_window_width(
    turns: &[TranscriptTurn],
    max_turns: usize,
    models: usize,
) -> (usize, usize) {
    // Fewer than `smallest` turns per window means more than `max_turns`
    // windows, and every window emits at least one record.
    let smallest = turns.len().div_ceil(max_turns).max(1);
    let fits_anyway = turns.len().div_ceil(max_turns / models).max(smallest);
    let mut probes = 0;
    let mut fits = |width: usize| {
        probes += 1;
        windowed_record_count(turns, width, max_turns) <= max_turns
    };

    let mut width = smallest;
    while width < fits_anyway && !fits(width) {
        width = width.saturating_mul(2).min(fits_anyway);
    }
    if width == smallest {
        return (width, probes);
    }
    // Every width strictly between `smallest` and the fitting step is a
    // candidate. Counts are not monotonic, so a width below a step that did
    // not fit may still fit; spread the probes over the whole gap.
    let gap = width - smallest - 1;
    let step = gap.div_ceil(WINDOW_REFINE_PROBES).max(1);
    let narrower = (1..=gap.min(WINDOW_REFINE_PROBES))
        .map(|k| smallest + k * step)
        .take_while(|&candidate| candidate < width)
        .find(|&candidate| fits(candidate));
    (narrower.unwrap_or(width), probes)
}

/// The index ranges of each stretch of consecutive same-model turns.
fn model_runs(turns: &[TranscriptTurn]) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    while start < turns.len() {
        let model = turns[start].model_raw.as_deref();
        let mut end = start + 1;
        while end < turns.len() && turns[end].model_raw.as_deref() == model {
            end += 1;
        }
        runs.push(start..end);
        start = end;
    }
    runs
}

/// Group one window's turns by model, ordered by each model's last reply in
/// the window so the folded records stay in timeline order.
fn window_model_groups(window: &[TranscriptTurn]) -> Vec<Vec<&TranscriptTurn>> {
    let mut groups: Vec<(usize, Vec<&TranscriptTurn>)> = Vec::new();
    let mut index_of: HashMap<Option<&str>, usize> = HashMap::new();
    for (position, turn) in window.iter().enumerate() {
        let group = *index_of
            .entry(turn.model_raw.as_deref())
            .or_insert_with(|| {
                groups.push((position, Vec::new()));
                groups.len() - 1
            });
        groups[group].0 = position;
        groups[group].1.push(turn);
    }
    groups.sort_by_key(|(last, _)| *last);
    groups.into_iter().map(|(_, turns)| turns).collect()
}

/// Records that windows of `width` turns would emit. Counting stops once it
/// passes `limit`, since the caller only asks whether the width fits.
fn windowed_record_count(turns: &[TranscriptTurn], width: usize, limit: usize) -> usize {
    let mut count = 0;
    let mut models: Vec<Option<&str>> = Vec::new();
    for window in turns.chunks(width) {
        models.clear();
        let mut previous = None;
        for turn in window {
            let model = turn.model_raw.as_deref();
            // Only a run's first turn can add a model.
            if previous != Some(model) {
                models.push(model);
                previous = Some(model);
            }
        }
        models.sort_unstable();
        models.dedup();
        count += models.len();
        if count > limit {
            break;
        }
    }
    count
}

fn fold_turn_windows(turns: &[TranscriptTurn], width: usize) -> Vec<TranscriptTurn> {
    turns
        .chunks(width)
        .flat_map(|window| window_model_groups(window).into_iter().map(fold_turn_run))
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
    let usage = match settings.token_usage.as_ref() {
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

    let (session_model, session_is_router, session_pricing_raw) =
        session_model_and_router(&settings, path);
    let billing_provider = settings.provider_lock.clone();
    let provider_lock = settings.provider_lock.as_deref();
    let session_provider = if session_is_router {
        provider_for_resolved_model(&session_model, provider_lock)
    } else {
        billing_provider
            .clone()
            .unwrap_or_else(|| get_provider_from_model(&session_model).to_string())
    };

    let totals = TokenBreakdown {
        input: usage.input_tokens.unwrap_or(0).max(0),
        output: usage.output_tokens.unwrap_or(0).max(0),
        cache_read: usage.cache_read_tokens.unwrap_or(0).max(0),
        cache_write: usage.cache_creation_tokens.unwrap_or(0).max(0),
        cache_write_1h: 0,
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

        let identities: Vec<(String, String, String)> = turns
            .iter()
            .map(|turn| {
                resolve_turn_identity(
                    turn,
                    &session_model,
                    provider_lock,
                    &session_provider,
                    &session_pricing_raw,
                )
            })
            .collect();
        // One countable record per model. A single total on the first record
        // would report every request as whichever model ran first, which is
        // the same bug as stamping the token total with the last model.
        // Later records of that model stay at zero so a long single-model
        // session is still one session, not one session per call.
        let mut reply_on_first = vec![0i32; turns.len()];
        let mut first_index: HashMap<&str, usize> = HashMap::new();
        for (index, turn) in turns.iter().enumerate() {
            let replies = turn.replies.max(1);
            let model = identities[index].0.as_str();
            if let Some(&first) = first_index.get(model) {
                reply_on_first[first] = reply_on_first[first].saturating_add(replies);
            } else {
                first_index.insert(model, index);
                reply_on_first[index] = replies;
            }
        }

        return turns
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                let (model, provider, pricing_raw) = identities[index].clone();
                let mut message = UnifiedMessage::new(
                    "droid",
                    model,
                    provider,
                    session_id.clone(),
                    turn.timestamp,
                    TokenBreakdown {
                        input: input[index],
                        output: output[index],
                        cache_read: cache_read[index],
                        cache_write: cache_write[index],
                        cache_write_1h: 0,
                        reasoning: reasoning[index],
                    },
                    0.0,
                )
                .with_billing(
                    billing_provider.as_deref(),
                    super::BillingIdentityOrigin::ExplicitSession,
                )
                .with_pricing_model(&pricing_raw);
                // Count-neutral fragments keep a single-model session as one
                // session. The first record of each model carries that model's
                // calls, so a switch does not file them under the first model.
                message.message_count = reply_on_first[index];
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
        "droid",
        session_model,
        session_provider,
        session_id,
        timestamp,
        totals,
        0.0,
    )
    .with_billing(
        billing_provider.as_deref(),
        super::BillingIdentityOrigin::ExplicitSession,
    )
    .with_pricing_model(&session_pricing_raw)]
}

fn resolve_turn_identity(
    turn: &TranscriptTurn,
    session_model: &str,
    provider_lock: Option<&str>,
    session_provider: &str,
    session_pricing_raw: &str,
) -> (String, String, String) {
    // The call's own model wins even when `settings.model` is explicit.
    // That field is the selection at the end of the session, not the model
    // that served the earlier calls.
    if let Some((model, pricing_raw)) = turn.model_raw.as_deref().and_then(concrete_model) {
        let provider = provider_for_resolved_model(&model, provider_lock);
        return (model, provider, pricing_raw);
    }
    (
        session_model.to_string(),
        session_provider.to_string(),
        session_pricing_raw.to_string(),
    )
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
        assert_eq!(get_provider_from_model("glm-5.3"), "zai");
        assert_eq!(get_provider_from_model("kimi-k3"), "moonshotai");
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

    fn assistant_with_model(timestamp: &str, filler: usize, model_id: &str) -> String {
        format!(
            r#"{{"type":"message","timestamp":"{timestamp}","message":{{"role":"assistant","content":"{}","modelId":"{model_id}"}}}}"#,
            "x".repeat(filler)
        )
    }

    fn write_named_session(
        dir: &Path,
        id: &str,
        settings_json: &str,
        transcript: &[&str],
    ) -> PathBuf {
        let settings = dir.join(format!("{id}.settings.json"));
        std::fs::write(&settings, settings_json).unwrap();
        std::fs::write(dir.join(format!("{id}.jsonl")), transcript.join("\n")).unwrap();
        settings
    }

    #[test]
    fn auto_session_uses_per_turn_model_id() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_named_session(
            temp_dir.path(),
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            r#"{"model":"auto","providerLock":"factory","tokenUsage":{"inputTokens":900,"outputTokens":100},"effectiveFactoryRouterModel":{"modelId":"glm-5.3"}}"#,
            &[
                r#"{"type":"session_start","id":"s"}"#,
                &assistant_with_model("2026-10-08T09:00:00Z", 10, "glm-5.3"),
                &assistant_with_model("2026-10-08T09:01:00Z", 20, "glm-5.3"),
            ],
        );

        let messages = parse_droid_file(&settings);
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|m| m.model_id == "glm-5-3"));
        assert!(messages.iter().all(|m| m.provider_id == "zai"));
        assert!(messages
            .iter()
            .all(|m| m.pricing_model_id.as_deref() == Some("glm-5.3")));
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 900);
        assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 100);
    }

    #[test]
    fn auto_session_splits_when_router_changes_model() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_named_session(
            temp_dir.path(),
            "bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee",
            r#"{"model":"auto","providerLock":"factory","tokenUsage":{"inputTokens":1000,"outputTokens":100}}"#,
            &[
                &assistant_with_model("2026-10-08T09:00:00Z", 10, "glm-5.3"),
                &assistant_with_model("2026-10-08T09:02:00Z", 10, "claude-opus-5-5"),
            ],
        );

        let messages = parse_droid_file(&settings);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "glm-5-3");
        assert_eq!(messages[0].provider_id, "zai");
        assert_eq!(messages[1].model_id, "claude-opus-5-5");
        assert_eq!(messages[1].provider_id, "anthropic");
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 1000);
    }

    #[test]
    fn auto_session_falls_back_to_effective_router_model() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_named_session(
            temp_dir.path(),
            "cccccccc-bbbb-cccc-dddd-eeeeeeeeeeee",
            r#"{"model":"auto","providerLock":"factory","tokenUsage":{"inputTokens":400,"outputTokens":50},"effectiveFactoryRouterModel":{"modelId":"glm-5.3"}}"#,
            &[&assistant("2026-10-08T09:00:00Z", 10)],
        );

        let messages = parse_droid_file(&settings);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "glm-5-3");
        assert_eq!(messages[0].provider_id, "zai");
        assert_eq!(messages[0].pricing_model_id.as_deref(), Some("glm-5.3"));
    }

    #[test]
    fn auto_session_without_served_model_keeps_the_routing_label() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_named_session(
            temp_dir.path(),
            "dddddddd-bbbb-cccc-dddd-eeeeeeeeeeee",
            r#"{"model":"auto","providerLock":"factory","tokenUsage":{"inputTokens":400}}"#,
            &[&assistant("2026-10-08T09:00:00Z", 10)],
        );

        let messages = parse_droid_file(&settings);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "auto");
        assert_eq!(messages[0].provider_id, "factory");
        assert_eq!(messages[0].pricing_model_id, None);
    }

    #[test]
    fn call_model_id_overrides_the_final_settings_model() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_named_session(
            temp_dir.path(),
            "eeeeeeee-bbbb-cccc-dddd-eeeeeeeeeeee",
            r#"{"model":"claude-opus-5-5","providerLock":"anthropic","tokenUsage":{"inputTokens":400}}"#,
            &[&assistant_with_model("2026-10-08T09:00:00Z", 10, "glm-5.3")],
        );

        let messages = parse_droid_file(&settings);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "glm-5-3");
        assert_eq!(messages[0].provider_id, "zai");
        assert_eq!(messages[0].pricing_model_id.as_deref(), Some("glm-5.3"));
        assert_eq!(messages[0].tokens.input, 400);
    }

    fn repl_tool_call(timestamp: &str) -> String {
        format!(
            r#"{{"type":"message","id":"repl-tool-call:abc","timestamp":"{timestamp}","message":{{"role":"assistant","content":"yyyy","visibility":"user_only"}}}}"#
        )
    }

    #[test]
    fn switched_explicit_models_keep_each_call() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let settings = write_named_session(
            temp_dir.path(),
            "ffffffff-bbbb-cccc-dddd-eeeeeeeeeeee",
            r#"{"model":"deepseek-v4.1-flash-1017","providerLock":"factory","tokenUsage":{"inputTokens":1000,"outputTokens":100,"cacheReadTokens":5000,"cacheCreationTokens":200}}"#,
            &[
                &assistant_with_model("2026-10-29T13:04:24Z", 30, "claude-opus-5-5"),
                &repl_tool_call("2026-10-29T13:04:24Z"),
                &assistant_with_model("2026-10-29T15:07:31Z", 10, "kimi-k3"),
                &assistant_with_model("2026-10-29T15:08:44Z", 10, "deepseek-v4.1-flash-1017"),
            ],
        );

        let messages = parse_droid_file(&settings);
        let models: Vec<&str> = messages.iter().map(|m| m.model_id.as_str()).collect();
        assert_eq!(
            models,
            vec!["claude-opus-5-5", "kimi-k3", "deepseek-v4-1-flash-1017",]
        );
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[1].provider_id, "moonshotai");
        assert_eq!(messages[2].provider_id, "deepseek");
        assert!(messages
            .iter()
            .all(|m| m.model_id != "deepseek-v4-1-flash-1017" || m.tokens.input < 1000));
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 1000);
        assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 100);
        assert_eq!(
            messages.iter().map(|m| m.tokens.cache_read).sum::<i64>(),
            5000
        );
        assert_eq!(
            messages.iter().map(|m| m.tokens.cache_write).sum::<i64>(),
            200
        );
        assert_eq!(
            messages.iter().map(|m| m.message_count).collect::<Vec<_>>(),
            vec![1, 1, 1]
        );
    }

    #[test]
    fn coalesce_does_not_merge_across_a_model_switch() {
        let turns = vec![
            TranscriptTurn {
                timestamp: 1,
                context_weight: 10,
                output_weight: 1,
                replies: 1,
                model_raw: Some("claude-opus-5-5".to_string()),
            },
            TranscriptTurn {
                timestamp: 2,
                context_weight: 10,
                output_weight: 1,
                replies: 1,
                model_raw: Some("claude-opus-5-5".to_string()),
            },
            TranscriptTurn {
                timestamp: 3,
                context_weight: 10,
                output_weight: 1,
                replies: 1,
                model_raw: Some("kimi-k3".to_string()),
            },
        ];
        let capped = coalesce_turns(turns, 1);
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[0].model_raw.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(capped[0].replies, 2);
        assert_eq!(capped[0].context_weight, 20);
        assert_eq!(capped[1].model_raw.as_deref(), Some("kimi-k3"));
        assert_eq!(capped[1].replies, 1);
    }

    fn alternating_turns(count: usize, models: &[&str]) -> Vec<TranscriptTurn> {
        (0..count)
            .map(|i| TranscriptTurn {
                timestamp: 1_000 + i as i64,
                context_weight: 100 + (i as u128 * 7) % 13,
                output_weight: 1 + (i as u128 * 3) % 5,
                replies: 1,
                model_raw: Some(models[i % models.len()].to_string()),
            })
            .collect()
    }

    /// (context weight, output weight, replies) summed per model.
    fn per_model_totals(turns: &[TranscriptTurn]) -> HashMap<Option<String>, (u128, u128, i32)> {
        let mut totals = HashMap::new();
        for turn in turns {
            let entry = totals.entry(turn.model_raw.clone()).or_insert((0, 0, 0));
            entry.0 += turn.context_weight;
            entry.1 += turn.output_weight;
            entry.2 += turn.replies;
        }
        totals
    }

    fn snapshot(turns: &[TranscriptTurn]) -> Vec<(i64, u128, u128, i32, Option<String>)> {
        turns
            .iter()
            .map(|turn| {
                (
                    turn.timestamp,
                    turn.context_weight,
                    turn.output_weight,
                    turn.replies,
                    turn.model_raw.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn coalesce_caps_a_session_that_alternates_models_every_reply() {
        let turns = alternating_turns(2000, &["claude-opus-5-5", "kimi-k3"]);
        let expected = per_model_totals(&turns);

        let capped = coalesce_turns(turns, MAX_TURNS_PER_SESSION);

        assert!(
            capped.len() <= MAX_TURNS_PER_SESSION,
            "record count not capped: {}",
            capped.len()
        );
        assert_eq!(per_model_totals(&capped), expected);
        assert!(capped.iter().all(|turn| turn.model_raw.is_some()));
        assert!(capped.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
    }

    #[test]
    fn coalesce_keeps_a_single_model_session_on_same_model_runs() {
        let turns = alternating_turns(2000, &["claude-opus-5-5"]);
        let expected = per_model_totals(&turns);
        // Today's shape: chunks of ceil(2000 / 1024) = 2 consecutive turns.
        let runs: Vec<_> = snapshot(&turns)
            .chunks(2)
            .map(|chunk| {
                (
                    chunk.iter().map(|t| t.0).max().unwrap(),
                    chunk.iter().map(|t| t.1).sum::<u128>(),
                    chunk.iter().map(|t| t.2).sum::<u128>(),
                    chunk.iter().map(|t| t.3).sum::<i32>(),
                    chunk[0].4.clone(),
                )
            })
            .collect();

        let capped = coalesce_turns(turns, MAX_TURNS_PER_SESSION);

        assert_eq!(snapshot(&capped), runs);
        assert_eq!(per_model_totals(&capped), expected);
    }

    #[test]
    fn coalesce_caps_a_mix_of_long_runs_and_alternation() {
        // A long Opus stretch, then a stretch that alternates three models on
        // every reply, then a long Kimi stretch.
        let mut turns = alternating_turns(600, &["claude-opus-5-5"]);
        turns.extend(alternating_turns(
            1500,
            &["claude-opus-5-5", "kimi-k3", "deepseek-v4-1-flash-1017"],
        ));
        turns.extend(alternating_turns(400, &["kimi-k3"]));
        for (i, turn) in turns.iter_mut().enumerate() {
            turn.timestamp = 1_000 + i as i64;
        }
        let expected = per_model_totals(&turns);

        let capped = coalesce_turns(turns, MAX_TURNS_PER_SESSION);

        assert!(
            capped.len() <= MAX_TURNS_PER_SESSION,
            "record count not capped: {}",
            capped.len()
        );
        assert_eq!(per_model_totals(&capped), expected);
        assert!(capped.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
    }

    #[test]
    fn coalesce_leaves_a_short_alternating_session_alone() {
        let turns = alternating_turns(300, &["claude-opus-5-5", "kimi-k3"]);
        let before = snapshot(&turns);
        assert_eq!(
            snapshot(&coalesce_turns(turns, MAX_TURNS_PER_SESSION)),
            before
        );
    }

    #[test]
    fn coalesce_keeps_one_record_per_model_when_models_outnumber_the_cap() {
        let turns = alternating_turns(30, &["a-1", "b-1", "c-1"]);
        let expected = per_model_totals(&turns);

        let capped = coalesce_turns(turns, 2);

        // Three models cannot share two records without mixing, so the floor
        // is one record per model.
        assert_eq!(capped.len(), 3);
        assert_eq!(per_model_totals(&capped), expected);
    }

    #[test]
    fn test_alternating_session_records_are_capped_with_exact_totals() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let replies = 2000;
        let models = ["claude-opus-5-5", "glm-5.3"];
        let transcript: Vec<String> = (0..replies)
            .map(|i| {
                assistant_with_model(
                    &format!("2026-08-07T12:{:02}:{:02}Z", (i / 60) % 60, i % 60),
                    10,
                    models[i % 2],
                )
            })
            .collect();
        let settings = write_session(
            temp_dir.path(),
            "aaaaaaaa-9999-9999-9999-999999999999",
            r#"{"inputTokens":100000,"outputTokens":7000}"#,
            &transcript.iter().map(String::as_str).collect::<Vec<_>>(),
        );

        let messages = parse_droid_file(&settings);

        assert!(
            messages.len() <= MAX_TURNS_PER_SESSION,
            "record count not capped: {}",
            messages.len()
        );
        assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 100000);
        assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 7000);
        for model in ["claude-opus-5-5", "glm-5-3"] {
            assert_eq!(
                messages
                    .iter()
                    .filter(|m| m.model_id == model)
                    .map(|m| m.message_count)
                    .sum::<i32>(),
                1000
            );
        }
    }

    #[test]
    fn coalesce_keeps_runs_apart_when_the_runs_alone_fit_the_cap() {
        // 8192 turns that switch model every 9 replies: 911 runs, under the
        // cap. One record per run fits, so no record may fold turns from two
        // runs. Windows of 15 turns would straddle switches and emit 1093
        // records, so a search that assumed the window count only falls as the
        // width grows skipped width 9 and folded across runs at width 16.
        let turns: Vec<TranscriptTurn> = (0..8192usize)
            .map(|i| TranscriptTurn {
                timestamp: 1_000 + i as i64,
                context_weight: 10 + (i as u128 % 7),
                output_weight: 1 + (i as u128 % 3),
                replies: 1,
                model_raw: Some(
                    if (i / 9) % 2 == 0 {
                        "claude-opus-5-5"
                    } else {
                        "kimi-k3"
                    }
                    .to_string(),
                ),
            })
            .collect();
        let expected = per_model_totals(&turns);
        // Each run reports at its last reply.
        let run_ends: Vec<i64> = (0..8192i64)
            .filter(|i| i % 9 == 8 || *i == 8191)
            .map(|i| 1_000 + i)
            .collect();

        let capped = coalesce_turns(turns, MAX_TURNS_PER_SESSION);

        assert_eq!(capped.len(), 911);
        assert_eq!(
            capped.iter().map(|turn| turn.timestamp).collect::<Vec<_>>(),
            run_ends
        );
        assert_eq!(per_model_totals(&capped), expected);
    }

    #[test]
    fn coalesce_uses_the_narrowest_window_that_fits() {
        // Four runs over a cap of three, so runs must share records. Windows of
        // 4 fit (aaaa | bbab -> 3 records), but windows of 5 to 7 do not, so a
        // search that assumed the count only falls as the width grows skipped
        // width 4 and folded the first four replies into a record at turn 7.
        let models = ["a-1", "a-1", "a-1", "a-1", "b-1", "b-1", "a-1", "b-1"];
        let turns: Vec<TranscriptTurn> = models
            .iter()
            .enumerate()
            .map(|(i, model)| TranscriptTurn {
                timestamp: 1 + i as i64,
                context_weight: 10,
                output_weight: 1,
                replies: 1,
                model_raw: Some(model.to_string()),
            })
            .collect();
        let expected = per_model_totals(&turns);

        let capped = coalesce_turns(turns, 3);

        assert_eq!(
            capped
                .iter()
                .map(|turn| (turn.timestamp, turn.model_raw.as_deref(), turn.replies))
                .collect::<Vec<_>>(),
            vec![
                (4, Some("a-1"), 4),
                (7, Some("a-1"), 1),
                (8, Some("b-1"), 3)
            ]
        );
        assert_eq!(per_model_totals(&capped), expected);
    }

    #[test]
    fn window_search_probes_a_bounded_number_of_widths() {
        // 400,000 replies alternating two models. Trying every width from the
        // smallest upward counted about 400,000 / 1024 widths, each a pass over
        // the turns, which took seconds on a transcript well under the byte
        // ceiling.
        let turns = alternating_turns(400_000, &["claude-opus-5-5", "kimi-k3"]);
        let smallest = turns.len().div_ceil(MAX_TURNS_PER_SESSION);
        let fits_anyway = turns.len().div_ceil(MAX_TURNS_PER_SESSION / 2);
        let ladder = (fits_anyway as f64 / smallest as f64).log2().ceil() as usize + 1;

        let (width, probes) = fitting_window_width(&turns, MAX_TURNS_PER_SESSION, 2);

        assert!(
            probes <= ladder + WINDOW_REFINE_PROBES,
            "{probes} probes for {} turns",
            turns.len()
        );
        assert!(windowed_record_count(&turns, width, usize::MAX) <= MAX_TURNS_PER_SESSION);
        assert!(width <= fits_anyway);
        let expected = per_model_totals(&turns);
        let capped = coalesce_turns(turns, MAX_TURNS_PER_SESSION);
        assert!(capped.len() <= MAX_TURNS_PER_SESSION);
        assert_eq!(per_model_totals(&capped), expected);
    }
}
