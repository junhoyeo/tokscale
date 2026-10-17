//! Roo Code task parser
//!
//! Parses task-based logs from VS Code globalStorage directories:
//! - `tasks/<taskId>/ui_messages.json`
//! - `tasks/<taskId>/api_conversation_history.json`

use super::utils::{extract_i64, parse_timestamp_str, read_file_or_none};
use super::UnifiedMessage;
use crate::provider_identity;
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Shared base parser version for the roo/kilo task-log format.
///
/// Roo Code, Kilo Code, and Cline all parse this format through
/// [`parse_roo_kilo_file`], so a change here alters what byte-identical task
/// logs parse to for every one of them at once. Bump this base when that
/// happens; `message_cache::parser_version()` derives each member's version
/// from it (base plus a per-client offset that preserves independent history)
/// so no member can be left serving stale cache entries.
///
/// v2->v3: a positive `cost` is marked provider-reported so submission keeps it
/// instead of re-pricing (and zeroing) the row, Roo/Kilo Code `tokensIn` has
/// the cache buckets it already contains subtracted out (Cline's stay
/// disjoint), and a bare `apiProtocol` no longer outranks the provider named by
/// the model id.
pub(crate) const ROO_KILO_TASK_LOG_PARSER_BASE_VERSION: u32 = 3;

#[derive(Debug, Deserialize)]
struct UiMessageEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    say: Option<String>,
    text: Option<String>,
    ts: Option<Value>,
    #[serde(rename = "modelInfo")]
    model_info: Option<UiModelInfo>,
}

/// Per-message model identity, written by current Cline on every
/// `ui_messages.json` entry as `modelInfo`.
///
/// Cline 4.x no longer writes the `<model>` tag inside
/// `<environment_details>` blocks nor the `apiProtocol` field of the
/// `api_req_started` payload, so those file-level heuristics resolve
/// `unknown/unknown` for every current task. When an entry carries
/// `modelInfo`, it is the authoritative identity for that message; older
/// Roo-style records keep the legacy heuristics (#1321).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiModelInfo {
    provider_id: Option<String>,
    model_id: Option<String>,
}

pub fn parse_roocode_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_roo_kilo_file(path, "roocode")
}

pub(crate) fn parse_roo_kilo_file(path: &Path, source: &str) -> Vec<UnifiedMessage> {
    let Some(data) = read_file_or_none(path) else {
        return Vec::new();
    };

    let mut bytes = data;
    let entries: Vec<UiMessageEntry> = match simd_json::from_slice(&mut bytes) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let session_id = extract_session_id(path);
    let (model_id, agent) = read_task_metadata(path);

    let mut messages = Vec::new();
    for entry in entries {
        if entry.entry_type.as_deref() != Some("say")
            || entry.say.as_deref() != Some("api_req_started")
        {
            continue;
        }

        let text = match entry.text {
            Some(t) => t,
            None => continue,
        };

        let timestamp = match parse_entry_timestamp(entry.ts.as_ref()) {
            Some(ts) => ts,
            None => continue,
        };

        let payload = match parse_api_req_started_payload(&text) {
            Some(p) => p,
            None => continue,
        };

        // `modelInfo` is per-message and states the model that actually
        // answered this request; the file-level `<model>`-tag heuristic
        // labels every row in a task with the last tag it saw, so the
        // per-entry identity wins whenever it is present.
        let model = entry
            .model_info
            .as_ref()
            .and_then(|info| info.model_id.clone())
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| model_id.clone());
        // A bare `apiProtocol` ("openai", "anthropic") names the wire format
        // the extension spoke, not who served the request: Kilo Code's own
        // gateway logs `openai` for `anthropic/claude-sonnet-4`. So the
        // provider resolves, in order, from:
        // 1. a nested `apiProtocol` ("bedrock/anthropic"), which carries
        //    reseller routing that `modelInfo.providerId` would flatten;
        // 2. `modelInfo.providerId`, current Cline's per-message identity
        //    (#1321);
        // 3. the model id itself: its vendor prefix (`anthropic/...`) or, for
        //    a bare id, its model family;
        // 4. the bare `apiProtocol`, as a last resort.
        let api_protocol = payload
            .api_protocol
            .as_deref()
            .map(str::trim)
            .filter(|protocol| !protocol.is_empty());
        let model_info_provider = entry
            .model_info
            .as_ref()
            .and_then(|info| info.provider_id.as_deref())
            .map(str::trim)
            .filter(|provider| !provider.is_empty());
        let provider = api_protocol
            .filter(|protocol| protocol.contains('/'))
            .or(model_info_provider)
            .map(str::to_string)
            .or_else(|| provider_from_model_id(&model))
            .or_else(|| api_protocol.map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string());

        // On Roo and Kilo Code, `tokensIn` is gross of cache, so the cache
        // buckets come out of input to stop counting them twice. The wire
        // protocol is NOT the discriminator: Roo `416fa5727e` (2025-10-31)
        // changed `Task.ts` to persist `tokensIn: costResult.totalInputTokens`,
        // which is base + cache for *both* protocols, and current Roo's
        // `consolidateTokenUsage.ts` says the same ("tokensIn now stores TOTAL
        // input tokens"). Only pre-2025-10 Anthropic-protocol rows were net of
        // cache, and there the subtraction is self-limiting: such a row records
        // base input with the cache buckets beside it, so subtracting a value
        // it never included can only reach zero, never go negative, and the
        // cache buckets are still reported in full.
        //
        // Cline forked this format and did NOT follow that change: it keeps
        // the three buckets disjoint on purpose ("Classic Cline/webview
        // metrics expect tokensIn, cacheReads, and cacheWrites to be disjoint
        // buckets" -- `apps/vscode/src/sdk/message-translator.ts`), and its own
        // totals add all three. Subtracting there would undercount real input,
        // so the client decides.
        let input = if gross_input_includes_cache(source) {
            residual_input(payload.tokens_in, payload.cache_reads, payload.cache_writes)
        } else {
            payload.tokens_in
        };

        let mut message = UnifiedMessage::new_with_agent(
            source,
            model,
            provider,
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input,
                output: payload.tokens_out,
                cache_read: payload.cache_reads,
                cache_write: payload.cache_writes,
                cache_write_1h: 0,
                reasoning: 0,
            },
            payload.cost,
            agent.clone(),
        );
        // The task log's `cost` is computed by the extension from its own
        // bundled price sheet (Roo `Task.ts` persists
        // `cost: totalCost ?? costResult.totalCost`), so a positive value is
        // the charge the client itself recorded and is kept for models
        // tokscale may not price. A zero is different: it is also what the
        // extension writes when *it* has no rate for the model, and that is
        // indistinguishable here from a genuinely free request. Marking it
        // authoritative would submit a known-priced request as a complete $0,
        // so zero is left for repricing -- the same rule
        // `CostProvenance::WhenPositive` applies to the opencode-schema
        // clients.
        if payload.cost_reported && payload.cost > 0.0 {
            message.mark_provider_reported_cost();
        }
        messages.push(message);
    }

    messages
}

fn extract_session_id(path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn read_task_metadata(ui_messages_path: &Path) -> (String, Option<String>) {
    let history_path = sibling_history_path(ui_messages_path);
    let content = match std::fs::read_to_string(&history_path) {
        Ok(c) => c,
        Err(_) => return ("unknown".to_string(), None),
    };

    extract_model_and_agent(&content)
}

fn sibling_history_path(ui_messages_path: &Path) -> PathBuf {
    ui_messages_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("api_conversation_history.json")
}

fn extract_model_and_agent(content: &str) -> (String, Option<String>) {
    const ENV_START: &str = "<environment_details>";
    const ENV_END: &str = "</environment_details>";

    let mut offset = 0usize;
    let mut last_model: Option<String> = None;
    let mut last_slug: Option<String> = None;
    let mut last_name: Option<String> = None;

    while let Some(start_rel) = content[offset..].find(ENV_START) {
        let start_idx = offset + start_rel + ENV_START.len();
        let rest = &content[start_idx..];

        let Some(end_rel) = rest.find(ENV_END) else {
            break;
        };
        let end_idx = start_idx + end_rel;
        let block = &content[start_idx..end_idx];

        if let Some(model) = extract_tag_value(block, "model") {
            last_model = Some(model);
        }
        if let Some(slug) = extract_tag_value(block, "slug") {
            last_slug = Some(slug);
        }
        if let Some(name) = extract_tag_value(block, "name") {
            last_name = Some(name);
        }

        offset = end_idx + ENV_END.len();
    }

    let model = last_model.unwrap_or_else(|| "unknown".to_string());
    let agent = last_slug.or(last_name);
    (model, agent)
}

fn extract_tag_value(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);

    let start_idx = block.find(&open)? + open.len();
    let rest = &block[start_idx..];
    let end_rel = rest.find(&close)?;
    let value = rest[..end_rel].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_entry_timestamp(ts: Option<&Value>) -> Option<i64> {
    let value = ts?;
    let ts_str = if let Some(s) = value.as_str() {
        s.to_string()
    } else if let Some(i) = value.as_i64() {
        i.to_string()
    } else {
        value.as_u64()?.to_string()
    };

    parse_timestamp_str(&ts_str)
}

struct ApiReqStartedPayload {
    cost: f64,
    cost_reported: bool,
    tokens_in: i64,
    tokens_out: i64,
    cache_reads: i64,
    cache_writes: i64,
    api_protocol: Option<String>,
}

/// Whether this client's `tokensIn` includes the cache buckets.
///
/// Roo and Kilo Code persist `costResult.totalInputTokens`, which is gross of
/// cache; Cline deliberately keeps the buckets disjoint. The standalone Cline
/// CLI lane in `cline.rs` subtracts because the SDK usage event it reads *is*
/// gross -- that is a different source, not a disagreement.
fn gross_input_includes_cache(source: &str) -> bool {
    !matches!(source, "cline")
}

/// Non-cached input tokens for a task-log row whose `tokensIn` is gross of
/// the cache buckets, clamped at zero so a pre-2025-10 row that recorded
/// `tokensIn` net of cache cannot produce negative input.
fn residual_input(tokens_in: i64, cache_reads: i64, cache_writes: i64) -> i64 {
    tokens_in
        .saturating_sub(cache_reads)
        .saturating_sub(cache_writes)
        .max(0)
}

fn parse_api_req_started_payload(text: &str) -> Option<ApiReqStartedPayload> {
    let mut bytes = text.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    let reported_cost = extract_f64(value.get("cost")).filter(|v| v.is_finite() && *v >= 0.0);
    let cost = reported_cost.unwrap_or(0.0);
    let tokens_in = extract_i64(value.get("tokensIn")).unwrap_or(0).max(0);
    let tokens_out = extract_i64(value.get("tokensOut")).unwrap_or(0).max(0);
    let cache_reads = extract_i64(value.get("cacheReads")).unwrap_or(0).max(0);
    let cache_writes = extract_i64(value.get("cacheWrites")).unwrap_or(0).max(0);
    let api_protocol = value
        .get("apiProtocol")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(ApiReqStartedPayload {
        cost,
        cost_reported: reported_cost.is_some(),
        tokens_in,
        tokens_out,
        cache_reads,
        cache_writes,
        api_protocol,
    })
}

fn extract_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|val| {
        val.as_f64()
            .or_else(|| val.as_i64().map(|v| v as f64))
            .or_else(|| val.as_u64().map(|v| v as f64))
            .or_else(|| val.as_str().and_then(|s| s.parse::<f64>().ok()))
    })
}

/// First path segments that route a request rather than name the vendor that
/// served it. `ollama/` is a local runtime, and `kilo-auto/` is Kilo's own
/// Auto router (Kilo stores those turns as provider `kilo`, model
/// `kilo-auto/<tier>` -- see `kilo-vscode/src/shared/provider-model.ts`), so
/// canonicalizing either as a vendor invents a provider.
const MODEL_ID_ROUTING_PREFIXES: &[(&str, Option<&str>)] =
    &[("ollama", None), ("kilo-auto", Some("kilo"))];

fn provider_from_model_id(model: &str) -> Option<String> {
    if let Some((vendor, rest)) = model.split_once('/') {
        let vendor = vendor.trim();
        if !rest.is_empty() {
            if let Some((_, provider)) = MODEL_ID_ROUTING_PREFIXES
                .iter()
                .find(|(prefix, _)| vendor.eq_ignore_ascii_case(prefix))
            {
                // A routing prefix names the router, not the vendor: either
                // the router is itself the billed provider (`kilo`), or the
                // routed model has to speak for itself (`ollama`), which
                // family inference below already handles.
                return provider
                    .map(str::to_string)
                    .or_else(|| infer_provider_from_family(model));
            }
            if let Some(provider) = provider_identity::canonical_provider(vendor) {
                return Some(provider);
            }
        }
    }
    infer_provider_from_family(model)
}

/// Family inference for a bare model id.
///
/// Delimited matching only, as in the Cursor and Reasonix lanes: an ordinary
/// word that merely contains a family name (`engroked`) must not be
/// attributed to that vendor. The trade is deliberate -- `contains`-style
/// matching would also resolve mid-word families (`chatgpt-4o-latest`,
/// `CodeLlama-34b-Instruct-hf`), but here a wrong vendor is worse than
/// `unknown`, because this value becomes both the report label and the
/// pricing hint, and `main` resolved none of these ids at all.
fn infer_provider_from_family(model: &str) -> Option<String> {
    provider_identity::inferred_provider_from_model_delimited(model).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_task(
        dir: &TempDir,
        task_id: &str,
        ui_messages_content: &str,
        history_content: Option<&str>,
    ) -> PathBuf {
        let task_dir = dir.path().join("tasks").join(task_id);
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("ui_messages.json"), ui_messages_content).unwrap();
        if let Some(history) = history_content {
            fs::write(task_dir.join("api_conversation_history.json"), history).unwrap();
        }
        task_dir.join("ui_messages.json")
    }

    /// `tokensIn` is gross of cache on both protocols in every log this parser
    /// still sees (Roo `416fa5727e`), so the cache buckets come out of input
    /// regardless of `apiProtocol` -- including when the row omits it, which
    /// is what current Cline writes.
    #[test]
    fn test_parse_roocode_cache_overlap_and_cost_provenance() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {"type":"say","say":"api_req_started","ts":1748038758034,
   "text":"{\"cost\":0,\"tokensIn\":1000,\"tokensOut\":10,\"cacheReads\":600,\"cacheWrites\":100,\"apiProtocol\":\"openai\"}"},
  {"type":"say","say":"api_req_started","ts":1748038758035,
   "text":"{\"cost\":0.5,\"tokensIn\":1000,\"tokensOut\":10,\"cacheReads\":600,\"cacheWrites\":100,\"apiProtocol\":\"anthropic\"}"},
  {"type":"say","say":"api_req_started","ts":1748038758036,
   "text":"{\"tokensIn\":1000,\"tokensOut\":10}"},
  {"type":"say","say":"api_req_started","ts":1748038758037,
   "text":"{\"cost\":0.25,\"tokensIn\":1000,\"tokensOut\":10,\"cacheReads\":600,\"cacheWrites\":100}",
   "modelInfo":{"providerId":"openai","modelId":"gpt-5"}}
]"#;
        let path = setup_task(&dir, "task-overlap", ui_messages, None);

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 4);

        // `tokensIn` is gross of cache, so input is the residual.
        assert_eq!(messages[0].tokens.input, 300);
        assert_eq!(messages[0].tokens.cache_read, 600);
        assert_eq!(messages[0].tokens.cache_write, 100);
        // An explicit zero is what the extension writes when *it* had no rate
        // for the model, so it stays estimable rather than authoritative.
        assert_eq!(messages[0].cost, 0.0);
        assert!(!messages[0].has_authoritative_cost());

        // Anthropic protocol: modern logs are gross of cache here too.
        assert_eq!(messages[1].tokens.input, 300);
        assert_eq!(messages[1].cost, 0.5);
        assert!(messages[1].has_authoritative_cost());

        // No `cost` key: left for the pricing service to estimate.
        assert!(!messages[2].has_authoritative_cost());

        // No `apiProtocol` at all (current Cline): still de-overlapped.
        assert_eq!(messages[3].tokens.input, 300);
        assert_eq!(messages[3].tokens.cache_read, 600);
        assert_eq!(messages[3].provider_id, "openai");
        assert!(messages[3].has_authoritative_cost());
    }

    /// A pre-2025-10 Anthropic-protocol row recorded `tokensIn` net of cache.
    /// Subtracting buckets it never included must clamp at zero rather than
    /// go negative, and must never reduce the cache buckets themselves.
    #[test]
    fn test_parse_roocode_legacy_net_input_clamps_at_zero() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {"type":"say","say":"api_req_started","ts":1748038758034,
   "text":"{\"cost\":0.1,\"tokensIn\":300,\"tokensOut\":10,\"cacheReads\":600,\"cacheWrites\":100,\"apiProtocol\":\"anthropic\"}"}
]"#;
        let path = setup_task(&dir, "task-legacy-net", ui_messages, None);

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.cache_read, 600);
        assert_eq!(messages[0].tokens.cache_write, 100);
    }

    #[test]
    fn test_provider_from_model_id() {
        assert_eq!(
            provider_from_model_id("anthropic/claude-sonnet-4").as_deref(),
            Some("anthropic")
        );
        assert_eq!(
            provider_from_model_id("x-ai/grok-code-fast-1").as_deref(),
            Some("xai")
        );
        // `ollama/` is a routing prefix: the routed model names the vendor.
        assert_eq!(
            provider_from_model_id("ollama/claude-sonnet-4").as_deref(),
            Some("anthropic")
        );
        assert_eq!(
            provider_from_model_id("ollama/qwen3-coder").as_deref(),
            Some("qwen")
        );
        // An unknown routed model resolves to nothing rather than to the
        // runtime that served it.
        assert_eq!(provider_from_model_id("ollama/my-local-finetune"), None);
        // `kilo-auto/` is Kilo's own router, and Kilo is the billed provider.
        for model in ["kilo-auto/frontier", "kilo-auto/free", "KILO-AUTO/small"] {
            assert_eq!(
                provider_from_model_id(model).as_deref(),
                Some("kilo"),
                "{model}"
            );
        }
        // A family name inside a longer word is not a family match.
        assert_eq!(provider_from_model_id("engroked"), None);
        assert_eq!(
            provider_from_model_id("claude-sonnet-3-7").as_deref(),
            Some("anthropic")
        );
    }

    #[test]
    fn test_parse_roocode_valid_api_req_started() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"cacheReads\":20,\"cacheWrites\":5,\"apiProtocol\":\"anthropic\"}"
  },
  {
    "type": "say",
    "say": "assistant_message",
    "ts": "2026-02-18T12:00:01Z",
    "text": "{}"
  }
]"#;
        let history = r#"before
<environment_details>
<model>claude-sonnet-4</model>
<slug>architect</slug>
<name>Architect</name>
</environment_details>
after"#;
        let path = setup_task(&dir, "task-abc", ui_messages, Some(history));

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "roocode");
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].session_id, "task-abc");
        // 100 gross input less the 20 + 5 cache buckets it contains.
        assert_eq!(messages[0].tokens.input, 75);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.cache_write, 5);
        assert_eq!(messages[0].cost, 0.12);
        assert_eq!(messages[0].agent.as_deref(), Some("architect"));
    }

    /// Cline 4.x writes neither the `<model>` tag nor `apiProtocol`; it states
    /// model identity per message through `modelInfo` (#1321). Without the
    /// per-message preference every such task resolved to `unknown/unknown`
    /// and priced at $0.
    #[test]
    fn test_parse_roocode_prefers_model_info_over_legacy_fields() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "ts": 1789022952376,
    "type": "say",
    "say": "task",
    "text": "666",
    "modelInfo": {"providerId": "anthropic", "modelId": "claude-sonnet-5", "mode": "act"}
  },
  {
    "ts": 1789022955199,
    "type": "say",
    "say": "api_req_started",
    "text": "{\"request\":\"<task>666</task>\",\"tokensIn\":3638,\"tokensOut\":409,\"cacheWrites\":0,\"cacheReads\":0,\"cost\":0.011366}",
    "modelInfo": {"providerId": "anthropic", "modelId": "claude-sonnet-5", "mode": "act"}
  }
]"#;
        // A history file exists but carries no <model> tag, matching current
        // Cline: the environment_details block is present, the model is not.
        let history = r#"before
<environment_details>
<slug>act</slug>
</environment_details>
after"#;
        let path = setup_task(&dir, "task-cline4", ui_messages, Some(history));

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-5");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 3638);
        assert_eq!(messages[0].tokens.output, 409);
        assert_eq!(messages[0].cost, 0.011366);
    }

    /// A task that switched models mid-task: the per-message `modelInfo`
    /// labels each row with the model that answered it, where the file-level
    /// heuristic stamped every row with the last `<model>` tag.
    #[test]
    fn test_parse_roocode_model_info_labels_each_message() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.1,\"tokensIn\":10,\"tokensOut\":1,\"apiProtocol\":\"openai\"}",
    "modelInfo": {"providerId": "openai", "modelId": "gpt-5.1"}
  },
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:05:00Z",
    "text": "{\"cost\":0.2,\"tokensIn\":20,\"tokensOut\":2}",
    "modelInfo": {"providerId": "anthropic", "modelId": "claude-sonnet-5"}
  }
]"#;
        let path = setup_task(&dir, "task-switch", ui_messages, None);

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "gpt-5.1");
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[1].model_id, "claude-sonnet-5");
        // The second entry carries no `apiProtocol`, so its `modelInfo`
        // provider fills in.
        assert_eq!(messages[1].provider_id, "anthropic");
    }

    /// A nested `apiProtocol` routes through a reseller; the bare
    /// `modelInfo.providerId` must not flatten it away.
    #[test]
    fn test_parse_roocode_nested_api_protocol_outranks_model_info_provider() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.1,\"tokensIn\":10,\"tokensOut\":1,\"apiProtocol\":\"bedrock/anthropic\"}",
    "modelInfo": {"providerId": "anthropic", "modelId": "claude-sonnet-5"}
  }
]"#;
        let path = setup_task(&dir, "task-nested", ui_messages, None);

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "bedrock/anthropic");
        assert_eq!(messages[0].model_id, "claude-sonnet-5");
    }

    /// Blank `modelInfo` fields must not blank out a working legacy
    /// resolution; empty strings fall through to the file-level heuristic.
    #[test]
    fn test_parse_roocode_blank_model_info_falls_back_to_legacy() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"apiProtocol\":\"anthropic\"}",
    "modelInfo": {"providerId": "", "modelId": "  "}
  }
]"#;
        let history = r#"
<environment_details>
<model>claude-sonnet-4</model>
</environment_details>
"#;
        let path = setup_task(&dir, "task-blank", ui_messages, Some(history));

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].provider_id, "anthropic");
    }

    #[test]
    fn test_parse_roocode_skips_malformed_payload_entry() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "not-json"
  },
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:02Z",
    "text": "{\"cost\":0.03,\"tokensIn\":10,\"tokensOut\":2,\"cacheReads\":1,\"cacheWrites\":0,\"apiProtocol\":\"openai\"}"
  }
]"#;
        let path = setup_task(&dir, "task-def", ui_messages, None);

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].model_id, "unknown");
        assert_eq!(messages[0].agent, None);
    }

    #[test]
    fn test_parse_roocode_preserves_nested_reseller_api_protocol() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"cacheReads\":20,\"cacheWrites\":5,\"apiProtocol\":\"bedrock/anthropic\"}"
  }
]"#;
        let history = r#"before
<environment_details>
<model>claude-sonnet-4</model>
</environment_details>
after"#;
        let path = setup_task(&dir, "task-nested-provider", ui_messages, Some(history));

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "bedrock/anthropic");
    }

    #[test]
    fn test_parse_roocode_skips_invalid_timestamp() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "not-a-time",
    "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"cacheReads\":20,\"cacheWrites\":5,\"apiProtocol\":\"anthropic\"}"
  }
]"#;
        let path = setup_task(&dir, "task-time", ui_messages, None);

        let messages = parse_roocode_file(&path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_roocode_invalid_file_json_is_ignored() {
        let dir = TempDir::new().unwrap();
        let path = setup_task(&dir, "task-invalid", "{not-json", None);

        let messages = parse_roocode_file(&path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_extract_model_and_agent_prefers_slug_then_name() {
        let content = r#"
<environment_details>
<model>gpt-5</model>
<name>Builder</name>
</environment_details>
<environment_details>
<model>gpt-5.1</model>
<slug>reviewer</slug>
<name>Reviewer</name>
</environment_details>
"#;

        let (model, agent) = extract_model_and_agent(content);
        assert_eq!(model, "gpt-5.1");
        assert_eq!(agent.as_deref(), Some("reviewer"));
    }
}
