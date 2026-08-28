//! LM Studio local-server usage parser.
//!
//! The OpenAI-compatible server writes pretty-printed final responses beneath
//! `~/.lmstudio/server-logs/`. This parser supports both Chat Completions and
//! Responses API usage shapes while extracting only the response identity,
//! model, local timestamp, and balanced `usage` object. Prompt and response
//! bodies are neither deserialized nor retained.

use super::utils::file_modified_timestamp_ms;
use super::UnifiedMessage;
use crate::TokenBreakdown;
use chrono::{Local, LocalResult, NaiveDateTime, TimeZone};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

const USAGE_MARKER: &[u8] = b"\"usage\"";

/// Bytes of one server log held in memory at a time.
///
/// `server-logs/` accumulates every request and response the local server
/// handled, prompt and completion bodies included, and nothing rotates it -- a
/// long-running server produces one file that grows without bound. Reading it
/// whole puts that growth into the scan's peak.
///
/// Refusing an oversized log is not an option either: these logs only grow, so
/// every mature install would eventually cross any fixed ceiling and then lose
/// *all* of its usage, past and future, with no diagnostic. The window instead
/// bounds what one file costs while still reading every record in it.
///
/// A record is a `usage` object plus the log lines immediately before it, so
/// records are local and a window this size holds many at once.
const SERVER_LOG_WINDOW_BYTES: usize = 8 * 1024 * 1024;

/// Bytes pulled from the log per read.
const SERVER_LOG_CHUNK_BYTES: usize = 64 * 1024;

/// How far past an eviction boundary the identity scan reads.
///
/// Enough to span a whole `"id"` / `"model"` field or a log header, so a field
/// straddling the cut is still recovered. These are short: a response id is
/// tens of bytes and a header under fifty.
const IDENTITY_OVERLAP_BYTES: usize = 512;

#[derive(Debug, Default, Deserialize)]
struct PromptTokenDetails {
    #[serde(default, alias = "cache_read_tokens")]
    cached_tokens: i64,
    #[serde(default, alias = "cache_write_tokens")]
    cache_creation_input_tokens: i64,
}

#[derive(Debug, Default, Deserialize)]
struct OutputTokenDetails {
    #[serde(default)]
    reasoning_tokens: i64,
}

#[derive(Debug, Default, Deserialize)]
struct UsagePayload {
    #[serde(
        default,
        alias = "promptTokens",
        alias = "input_tokens",
        alias = "inputTokens"
    )]
    prompt_tokens: i64,
    #[serde(
        default,
        alias = "completionTokens",
        alias = "output_tokens",
        alias = "outputTokens"
    )]
    completion_tokens: i64,
    #[serde(default, alias = "totalTokens")]
    total_tokens: i64,
    #[serde(default, alias = "input_tokens_details", alias = "inputTokensDetails")]
    prompt_tokens_details: PromptTokenDetails,
    #[serde(
        default,
        alias = "completion_tokens_details",
        alias = "completionTokensDetails",
        alias = "outputTokensDetails"
    )]
    output_tokens_details: OutputTokenDetails,
    #[serde(default)]
    cached_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
    #[serde(default)]
    reasoning_tokens: i64,
}

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len().saturating_sub(needle.len()) {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

fn skip_ascii_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

/// Find the next `"usage": {`, and report how far the scan is *certain* of.
///
/// The second value is the first offset that might still begin a match. A
/// streaming caller must not advance its scan floor past it: `"usage"` is only
/// accepted once a colon, any whitespace, and an opening brace follow, so a
/// refill boundary landing anywhere in that run leaves a marker that this call
/// rejects but a later one would accept. Advancing past it drops the record for
/// good -- silently, because the bytes are then behind the window.
fn scan_usage_object_start(bytes: &[u8], from: usize) -> (Option<(usize, usize)>, usize) {
    let mut cursor = from;
    while let Some(marker) = find_bytes(bytes, USAGE_MARKER, cursor) {
        cursor = marker + USAGE_MARKER.len();
        if marker > 0 && bytes[marker - 1] == b'\\' {
            continue;
        }
        let value = skip_ascii_whitespace(bytes, cursor);
        // Ran out of buffer inside the run this match needs: undecidable here.
        if value >= bytes.len() {
            return (None, marker);
        }
        if bytes[value] != b':' {
            continue;
        }
        let brace = skip_ascii_whitespace(bytes, value + 1);
        if brace >= bytes.len() {
            return (None, marker);
        }
        if bytes[brace] == b'{' {
            return (Some((marker, brace)), marker);
        }
    }
    // No marker matched. A marker split across the boundary could still start
    // in the final `USAGE_MARKER.len() - 1` bytes.
    (
        None,
        bytes
            .len()
            .saturating_sub(USAGE_MARKER.len().saturating_sub(1)),
    )
}

fn balanced_object_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn json_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate().skip(start + 1) {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Some(index + 1);
        }
    }
    None
}

fn last_json_string_field(bytes: &[u8], field: &[u8]) -> Option<String> {
    let mut marker = Vec::with_capacity(field.len() + 2);
    marker.push(b'"');
    marker.extend_from_slice(field);
    marker.push(b'"');
    let mut cursor = 0usize;
    let mut found = None;
    while let Some(index) = find_bytes(bytes, &marker, cursor) {
        cursor = index + marker.len();
        if index > 0 && bytes[index - 1] == b'\\' {
            continue;
        }
        let mut value = skip_ascii_whitespace(bytes, cursor);
        if bytes.get(value) != Some(&b':') {
            continue;
        }
        value = skip_ascii_whitespace(bytes, value + 1);
        let Some(end) = json_string_end(bytes, value) else {
            continue;
        };
        if let Ok(parsed) = serde_json::from_slice::<String>(&bytes[value..end]) {
            found = Some(parsed);
        }
    }
    found
}

/// Timestamp of the last LM Studio log header in `bytes`.
///
/// Anchored to the header's shape -- `[<ts>]` at the start of a line and
/// immediately followed by `[` -- rather than to any bracketed date. A model
/// that prints `[2020-01-01 00:00:00]` in its answer lands *after* the real
/// header in this region, so a last-match-wins scan over bare brackets would
/// date the usage from the model's own output.
fn last_log_timestamp(bytes: &[u8]) -> Option<i64> {
    let text = String::from_utf8_lossy(bytes);
    let mut parsed = None;
    for line in text.split('\n') {
        let line = line.strip_prefix('\r').unwrap_or(line);
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some(end) = rest.find(']') else {
            continue;
        };
        // The header always continues into its next bracketed field.
        if !rest[end + 1..].starts_with('[') {
            continue;
        }
        let Ok(naive) = NaiveDateTime::parse_from_str(&rest[..end], "%Y-%m-%d %H:%M:%S") else {
            continue;
        };
        parsed = match Local.from_local_datetime(&naive) {
            LocalResult::Single(value) => Some(value.timestamp_millis()),
            LocalResult::Ambiguous(first, _) => Some(first.timestamp_millis()),
            LocalResult::None => parsed,
        };
    }
    parsed
}

fn non_negative(value: i64) -> i64 {
    value.max(0)
}

fn normalized_tokens(usage: &UsagePayload) -> Option<TokenBreakdown> {
    let prompt = non_negative(usage.prompt_tokens);
    let output = non_negative(usage.completion_tokens);
    let cache_read = non_negative(
        usage
            .prompt_tokens_details
            .cached_tokens
            .max(usage.cached_tokens),
    )
    .min(prompt);
    let cache_write = non_negative(
        usage
            .prompt_tokens_details
            .cache_creation_input_tokens
            .max(usage.cache_creation_input_tokens),
    )
    .min(prompt.saturating_sub(cache_read));
    let reasoning = non_negative(
        usage
            .output_tokens_details
            .reasoning_tokens
            .max(usage.reasoning_tokens),
    )
    .min(output);
    let total = non_negative(usage.total_tokens).max(prompt.saturating_add(output));
    if total == 0 {
        return None;
    }
    let input = total
        .saturating_sub(output)
        .saturating_sub(cache_read)
        .saturating_sub(cache_write);
    Some(TokenBreakdown {
        input,
        output: output.saturating_sub(reasoning),
        cache_read,
        cache_write,
        reasoning,
    })
}

fn source_hash(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())[..12].to_string()
}

fn fallback_dedup_key(path: &Path, marker: usize, model: &str, tokens: &TokenBreakdown) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(marker.to_le_bytes());
    hasher.update(model.as_bytes());
    for value in [
        tokens.input,
        tokens.output,
        tokens.cache_read,
        tokens.cache_write,
    ] {
        hasher.update(value.to_le_bytes());
    }
    format!("lmstudio:{:x}", hasher.finalize())
}

pub fn parse_lmstudio_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_lmstudio_file_windowed(path, SERVER_LOG_WINDOW_BYTES, SERVER_LOG_CHUNK_BYTES)
}

/// [`parse_lmstudio_file`] with the buffer sizes injected.
///
/// The window and chunk are parameters purely so tests can drive the boundary
/// arithmetic with a few hundred bytes instead of writing an 8 MiB fixture --
/// the paths that matter here are "a record straddles a refill" and "a record
/// exceeds the window", and both are about the boundary, not its size.
fn parse_lmstudio_file_windowed(
    path: &Path,
    window_bytes: usize,
    chunk_bytes: usize,
) -> Vec<UnifiedMessage> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut reader = std::io::BufReader::new(file);
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let session_id = format!("lmstudio:{}", source_hash(path));
    let mut messages = Vec::new();

    // `window` is the part of the log not yet turned into messages.
    // `window_start` is its absolute offset in the file, which is what keeps
    // `fallback_dedup_key` addressing the same byte it addressed when this
    // parser read the whole file at once -- a shifted offset would mint new
    // dedup keys for records already submitted.
    let mut window: Vec<u8> = Vec::new();
    let mut window_start = 0usize;
    // Absolute offset where the metadata for the next record begins.
    let mut metadata_start = 0usize;
    // Absolute offset the marker search has already ruled out, so refilling the
    // window does not rescan what it has seen.
    let mut scanned_to = 0usize;
    let mut chunk = vec![0u8; chunk_bytes.max(1)];
    // Identity recovered from bytes the window had to evict before the record
    // they belong to was complete.
    let mut carried = CarriedIdentity::default();

    loop {
        let read = reader.read(&mut chunk).unwrap_or(0);
        if read > 0 {
            window.extend_from_slice(&chunk[..read]);
        }
        let at_eof = read == 0;

        // Drain every record the window now holds in full.
        loop {
            let scan_from = scanned_to.saturating_sub(window_start).min(window.len());
            let (found, certain_to) = scan_usage_object_start(&window, scan_from);
            let Some((marker, object_start)) = found else {
                // Only what the scan is certain about may be retired; the rest
                // has to be rescanned once more bytes arrive.
                scanned_to = window_start + certain_to;
                break;
            };
            let Some(object_end) = balanced_object_end(&window, object_start) else {
                // The object is cut off by the window edge. Leave the scan
                // floor *behind* the marker so the next refill re-finds it;
                // at EOF it will never complete, so retire it instead.
                scanned_to = if at_eof {
                    window_start + window.len()
                } else {
                    window_start + marker
                };
                break;
            };
            scanned_to = window_start + object_end;

            let absolute_marker = window_start + marker;
            let absolute_end = window_start + object_end;
            let metadata_from = metadata_start.saturating_sub(window_start).min(marker);
            if let Some(message) = message_from_record(
                &window[object_start..object_end],
                &window[metadata_from..marker],
                &carried,
                path,
                absolute_marker,
                &session_id,
                fallback_timestamp,
            ) {
                messages.push(message);
            }
            metadata_start = absolute_end;
            carried.clear();
        }

        // Release what is behind the next record's metadata. Everything before
        // it has already produced whatever messages it was going to.
        let keep_from = metadata_start
            .max(window_start)
            .saturating_sub(window_start)
            .min(window.len());
        if keep_from > 0 {
            window.drain(..keep_from);
            window_start += keep_from;
        }

        // A response body longer than the window pushes its own header out of
        // it. That header is where the response id, the model, and the log
        // timestamp live, so evicting it blind would leave the record with an
        // unknown model, the file's mtime for a date, and a path-derived dedup
        // key -- which stops matching the same response in a mirrored log and
        // drifts the date every time the file is appended to. The identity is
        // therefore lifted out of the bytes on their way past.
        if window.len() > window_bytes {
            let overflow = window.len() - window_bytes;
            // Read past the cut before dropping: the eviction boundary falls
            // wherever the refill arithmetic puts it, which is happily in the
            // middle of `"id":"chatcmpl-..."`. A field split across the cut
            // would be in neither the evicted slice nor the retained window.
            // The overlap only reaches forward into bytes that are still ahead
            // of the current record's marker, so it cannot pick up a later
            // record's identity.
            let absorb_to = overflow
                .saturating_add(IDENTITY_OVERLAP_BYTES)
                .min(window.len());
            carried.absorb(&window[..absorb_to]);
            window.drain(..overflow);
            window_start += overflow;
            metadata_start = metadata_start.max(window_start);
            scanned_to = scanned_to.max(window_start);
        }

        if at_eof {
            break;
        }
    }

    messages
}

/// Build one message from a `usage` object and the log text before it.
///
/// Split out of the scan loop so the loop deals only in window arithmetic:
/// this half sees two slices and never an offset it has to translate.
/// Response identity rescued from window bytes before they were dropped.
///
/// A record's `id`, `model` and log timestamp sit ahead of its `usage` object,
/// so a long response body can push them out of the window before the record
/// completes. Keeping them here makes the result independent of where the
/// buffer boundaries happened to fall.
#[derive(Default)]
struct CarriedIdentity {
    response_id: Option<String>,
    model: Option<String>,
    timestamp: Option<i64>,
}

impl CarriedIdentity {
    /// Take the identity out of bytes about to be discarded. Later bytes win,
    /// matching the "last one before the usage object" rule applied to a window
    /// that still holds them.
    fn absorb(&mut self, bytes: &[u8]) {
        if let Some(id) = response_id_in(bytes) {
            self.response_id = Some(id);
        }
        if let Some(model) = model_in(bytes) {
            self.model = Some(model);
        }
        if let Some(timestamp) = last_log_timestamp(bytes) {
            self.timestamp = Some(timestamp);
        }
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

fn response_id_in(bytes: &[u8]) -> Option<String> {
    last_json_string_field(bytes, b"id").filter(|value| {
        ["chatcmpl-", "cmpl-", "resp_"]
            .iter()
            .any(|prefix| value.starts_with(prefix))
    })
}

fn model_in(bytes: &[u8]) -> Option<String> {
    last_json_string_field(bytes, b"model").filter(|value| !value.trim().is_empty())
}

fn message_from_record(
    usage_object: &[u8],
    metadata: &[u8],
    carried: &CarriedIdentity,
    path: &Path,
    marker: usize,
    session_id: &str,
    fallback_timestamp: i64,
) -> Option<UnifiedMessage> {
    let usage = serde_json::from_slice::<UsagePayload>(usage_object).ok()?;
    let tokens = normalized_tokens(&usage)?;

    // What the window still holds is nearer the usage object than anything
    // evicted, so it wins; the carried value is the fallback, not the default.
    let response_id = response_id_in(metadata).or_else(|| carried.response_id.clone());
    let model = model_in(metadata)
        .or_else(|| carried.model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let timestamp = last_log_timestamp(metadata)
        .or(carried.timestamp)
        .unwrap_or(fallback_timestamp);
    if timestamp <= 0 {
        return None;
    }

    let dedup_key = response_id
        .map(|id| format!("lmstudio:{id}"))
        .unwrap_or_else(|| fallback_dedup_key(path, marker, &model, &tokens));
    let mut message = UnifiedMessage::new_with_dedup(
        "lmstudio",
        model,
        "lmstudio",
        session_id.to_string(),
        timestamp,
        tokens,
        0.0,
        Some(dedup_key),
    );
    message.mark_provider_reported_cost();
    message.is_turn_start = true;
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// One response preceded by `pad_bytes` of log filler, so a record is only
    /// reachable by a parse that carries state across refills.
    fn padded_log(index: usize, pad_bytes: usize) -> String {
        format!(
            "[filler] {}\n[2026-07-09 10:00:{:02}][INFO][fixture-model]\n\
             Final response: {{\"id\":\"chatcmpl-{index}\",\"model\":\"fixture-model\",\
             \"usage\":{{\"prompt_tokens\":{},\"completion_tokens\":5,\"total_tokens\":{}}}}}\n",
            "p".repeat(pad_bytes),
            index % 60,
            10 + index,
            15 + index,
        )
    }

    /// Every record survives a log many times larger than the buffer holding
    /// it, with records straddling refill boundaries.
    ///
    /// This is the regression that matters most in this file. Bounding the read
    /// by *refusing* an oversized log turned "uses memory" into "loses every
    /// record in that file, past and future, with no diagnostic" -- and these
    /// logs only ever grow, so every mature install reaches it eventually.
    #[test]
    fn reads_every_record_in_a_log_larger_than_the_window() {
        let mut file = NamedTempFile::new().unwrap();
        let records = 40;
        for i in 0..records {
            file.write_all(padded_log(i, 200).as_bytes()).unwrap();
        }
        file.flush().unwrap();

        let size = file.as_file().metadata().unwrap().len() as usize;
        // Deliberately far below the file: the window must not have to hold it.
        let window = 512;
        let chunk = 64;
        assert!(size > window * 10, "fixture must dwarf the window");

        let messages = parse_lmstudio_file_windowed(file.path(), window, chunk);
        assert_eq!(
            messages.len(),
            records,
            "every record must survive a log the parse cannot hold at once"
        );
        // Per-record values, so this also proves each usage object kept the
        // metadata that preceded it rather than a neighbour's.
        for (i, message) in messages.iter().enumerate() {
            assert_eq!(message.tokens.input, 10 + i as i64);
            assert_eq!(
                message.dedup_key.as_deref(),
                Some(&*format!("lmstudio:chatcmpl-{i}"))
            );
        }
    }

    /// Buffer sizes must not change what is parsed, only how much is resident.
    #[test]
    fn window_and_chunk_sizes_do_not_change_the_result() {
        let mut file = NamedTempFile::new().unwrap();
        for i in 0..12 {
            file.write_all(padded_log(i, 300).as_bytes()).unwrap();
        }
        file.flush().unwrap();

        let reference = parse_lmstudio_file(file.path());
        assert_eq!(reference.len(), 12);
        // Windows below one record length legitimately drop records, so
        // the agreement claim only covers windows that can hold one.
        for (window, chunk) in [(512, 1), (1024, 7), (4096, 64), (1 << 20, 1 << 16)] {
            let windowed = parse_lmstudio_file_windowed(file.path(), window, chunk);
            assert_eq!(
                windowed, reference,
                "window={window} chunk={chunk} changed the parse"
            );
        }
    }

    /// A response body long enough to evict its own header must still be
    /// attributed to the right model, date and response id.
    ///
    /// Without carrying the identity past eviction the record falls back to an
    /// unknown model, the file's mtime, and a path-derived dedup key -- which
    /// stops matching the same response in a mirrored log and moves the date
    /// every time the file is appended to.
    #[test]
    fn a_response_that_outgrows_the_window_keeps_its_identity() {
        let mut file = NamedTempFile::new().unwrap();
        // Header and identity first, then a body far longer than the window,
        // then the usage object -- the real shape of a long completion.
        let body = "b".repeat(4096);
        write!(
            file,
            "[2026-07-09 10:00:00][INFO][fixture-model]\n\
             Final response: {{\"id\":\"chatcmpl-far\",\"model\":\"big-model\",\
             \"choices\":[{{\"text\":\"{body}\"}}],\
             \"usage\":{{\"prompt_tokens\":31,\"completion_tokens\":7,\"total_tokens\":38}}}}\n"
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_lmstudio_file_windowed(file.path(), 512, 64);
        assert_eq!(messages.len(), 1, "the record must survive eviction");
        let message = &messages[0];
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("lmstudio:chatcmpl-far"),
            "the response id must survive its own body"
        );
        assert_eq!(message.model_id, "big-model");
        assert_eq!(message.tokens.input, 31);
        // 2026-07-09 local, not the file's mtime.
        assert_eq!(message.date, "2026-07-09");
        // And the windowed parse agrees with the unwindowed one.
        assert_eq!(messages, parse_lmstudio_file(file.path()));
    }

    /// A bracketed date inside the model's answer is not a log header.
    #[test]
    fn a_timestamp_printed_by_the_model_is_not_taken_as_the_log_time() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            "[2026-07-09 10:00:00][INFO][fixture-model]\n\
             Final response: {{\"id\":\"chatcmpl-quoted\",\"model\":\"fixture-model\",\
             \"choices\":[{{\"text\":\"the log said [2019-03-04 05:06:07] earlier\"}}],\
             \"usage\":{{\"prompt_tokens\":12,\"completion_tokens\":3,\"total_tokens\":15}}}}\n"
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_lmstudio_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].date, "2026-07-09",
            "the header dates the usage, not a date the model printed"
        );
    }

    /// A record whose own bytes exceed the window is dropped, and only it.
    #[test]
    fn a_record_larger_than_the_window_does_not_take_the_file_with_it() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(padded_log(0, 32).as_bytes()).unwrap();
        file.write_all(padded_log(1, 4096).as_bytes()).unwrap();
        file.write_all(padded_log(2, 32).as_bytes()).unwrap();
        file.flush().unwrap();

        let keys: Vec<String> = parse_lmstudio_file_windowed(file.path(), 512, 64)
            .into_iter()
            .filter_map(|m| m.dedup_key)
            .collect();
        assert!(
            keys.contains(&"lmstudio:chatcmpl-0".to_string()),
            "records before an oversized one survive: {keys:?}"
        );
        assert!(
            keys.contains(&"lmstudio:chatcmpl-2".to_string()),
            "records after an oversized one survive: {keys:?}"
        );
    }

    #[test]
    fn parses_exact_components_and_marks_local_cost_authoritative() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"[2026-07-09 10:00:00][INFO][fixture-model]
Final response: {{
  "id": "chatcmpl-fixture",
  "model": "fixture-model",
  "choices": [{{"message": {{"content": "synthetic {{ braces }}"}}}}],
  "usage": {{
    "prompt_tokens": 100,
    "completion_tokens": 12,
    "total_tokens": 112,
    "prompt_tokens_details": {{"cached_tokens": 40, "cache_creation_input_tokens": 10}}
  }}
}}
"#
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_lmstudio_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "lmstudio");
        assert_eq!(messages[0].model_id, "fixture-model");
        assert_eq!(messages[0].tokens.input, 50);
        assert_eq!(messages[0].tokens.output, 12);
        assert_eq!(messages[0].tokens.cache_read, 40);
        assert_eq!(messages[0].tokens.cache_write, 10);
        assert_eq!(messages[0].tokens.total(), 112);
        assert_eq!(messages[0].cost, 0.0);
        assert!(messages[0].has_authoritative_cost());
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("lmstudio:chatcmpl-fixture")
        );
    }

    #[test]
    fn keeps_distinct_identical_usage_and_skips_partial_or_zero_records() {
        let mut file = NamedTempFile::new().unwrap();
        for id in ["chatcmpl-a", "chatcmpl-b"] {
            writeln!(
                file,
                "[2026-07-09 11:00:00][INFO][m]\n{}",
                serde_json::json!({
                    "id": id,
                    "model": "m",
                    "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}
                })
            )
            .unwrap();
        }
        writeln!(file, "{{\"usage\":{{\"prompt_tokens\":0}}}}").unwrap();
        write!(file, "{{\"usage\":{{\"prompt_tokens\":9").unwrap();
        file.flush().unwrap();

        let messages = parse_lmstudio_file(file.path());
        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.total())
                .sum::<i64>(),
            20
        );
    }

    #[test]
    fn parses_responses_api_usage_without_double_counting_reasoning() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "[2026-07-09 11:30:00][INFO][responses-model]\n{}",
            serde_json::json!({
                "id": "resp_fixture",
                "model": "responses-model",
                "output": [{"type": "reasoning"}, {"type": "message"}],
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 40,
                    "total_tokens": 140,
                    "input_tokens_details": {"cached_tokens": 30},
                    "output_tokens_details": {"reasoning_tokens": 10}
                }
            })
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_lmstudio_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "responses-model");
        assert_eq!(messages[0].tokens.input, 70);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 30);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].tokens.reasoning, 10);
        assert_eq!(messages[0].tokens.total(), 140);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("lmstudio:resp_fixture")
        );
    }

    #[test]
    fn ignores_response_content_that_looks_like_metadata() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "[2026-07-09 11:00:00][INFO][real-model]\n{}",
            serde_json::json!({
                "id": "chatcmpl-real",
                "model": "real-model",
                "choices": [{
                    "message": {
                        "content": r#"{"id":"chatcmpl-fake","model":"fake-model"}"#
                    }
                }],
                "usage": {
                    "prompt_tokens": 7,
                    "completion_tokens": 3,
                    "total_tokens": 10
                }
            })
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_lmstudio_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "real-model");
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("lmstudio:chatcmpl-real")
        );
    }

    #[test]
    fn uses_reported_total_without_losing_component_closure() {
        let usage = UsagePayload {
            prompt_tokens: 20,
            completion_tokens: 5,
            total_tokens: 30,
            prompt_tokens_details: PromptTokenDetails {
                cached_tokens: 8,
                cache_creation_input_tokens: 2,
            },
            ..UsagePayload::default()
        };
        let tokens = normalized_tokens(&usage).unwrap();
        assert_eq!(tokens.input, 15);
        assert_eq!(tokens.total(), 30);
    }
}
