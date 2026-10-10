use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{TimeZone, Utc};
use serde_json::Value;

use super::{UsageMetric, UsageOutput};

const SUBSCRIPTIONS_URL: &str = "https://grok.com/rest/subscriptions";
const TASK_USAGE_URL: &str = "https://grok.com/rest/tasks/usage";
const BILLING_GRPC_URL: &str = "https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig";
/// SuperGrok / Grok Build weekly credit pool (`~/.grok` OIDC). Distinct from
/// Cursor-metered Grok Bot (`GetSandUsageStatus`).
const GROK_CREDITS_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
/// Monthly included-allowance meter for the same SuperGrok subscription.
const GROK_MONTHLY_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing";
const GROK_USER_AGENT: &str = "Grok Build";
const WEEKLY_PERIOD_TYPE: &str = "USAGE_PERIOD_TYPE_WEEKLY";

#[derive(Debug, Clone)]
struct Credentials {
    token: String,
    email: Option<String>,
}

#[derive(Debug)]
enum ProtoValue<'a> {
    Varint(u64),
    Fixed32(u32),
    Fixed64,
    Bytes(&'a [u8]),
}

fn grok_home() -> std::path::PathBuf {
    std::env::var_os("GROK_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| crate::paths::home_dir().map(|home| home.join(".grok")))
        .unwrap_or_else(|| std::path::PathBuf::from(".grok"))
}

fn auth_path() -> std::path::PathBuf {
    grok_home().join("auth.json")
}

pub fn has_credentials() -> bool {
    auth_path().exists()
}

fn read_credentials() -> Result<Vec<Credentials>> {
    let content = std::fs::read_to_string(auth_path())?;
    let doc: Value = serde_json::from_str(&content)?;
    credential_candidates_from_value(&doc)
}

fn credential_candidates_from_value(doc: &Value) -> Result<Vec<Credentials>> {
    let entries = doc
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Grok auth.json must contain an object."))?;

    let mut candidates: Vec<_> = entries
        .iter()
        .filter_map(|(scope, value)| {
            let entry = value.as_object()?;
            let token = entry
                .get("key")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())?
                .to_string();
            let email = entry
                .get("email")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            let priority = if scope.contains("auth.x.ai") { 0 } else { 1 };
            Some((priority, Credentials { token, email }))
        })
        .collect();

    candidates.sort_by_key(|(priority, _)| *priority);
    let credentials: Vec<_> = candidates
        .into_iter()
        .map(|(_, credentials)| credentials)
        .collect();

    if credentials.is_empty() {
        anyhow::bail!("No Grok token found. Run 'grok login'.");
    }
    Ok(credentials)
}

fn bearer_request(client: &reqwest::Client, token: &str, url: &str) -> reqwest::RequestBuilder {
    client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .header("Accept", "application/json")
        .header("User-Agent", GROK_USER_AGENT)
}

async fn fetch_subscriptions(client: &reqwest::Client, token: &str) -> Result<Value> {
    let resp = bearer_request(client, token, SUBSCRIPTIONS_URL)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("Grok subscriptions request failed (HTTP {status})");
    }
    let text = resp.text().await?;
    if text.trim_start().starts_with('<') {
        anyhow::bail!("Grok subscriptions returned HTML");
    }
    Ok(serde_json::from_str(&text)?)
}

async fn fetch_task_usage(client: &reqwest::Client, token: &str) -> Result<Value> {
    let resp = bearer_request(client, token, TASK_USAGE_URL).send().await?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("NEEDS_AUTH");
    }
    if !status.is_success() {
        anyhow::bail!("Grok task usage request failed (HTTP {status})");
    }
    Ok(resp.json().await?)
}

async fn fetch_billing_grpc(client: &reqwest::Client, token: &str) -> Result<Vec<u8>> {
    let resp = client
        .post(BILLING_GRPC_URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .header("Accept", "application/grpc-web+proto")
        .header("Content-Type", "application/grpc-web+proto")
        .header("User-Agent", GROK_USER_AGENT)
        .body(vec![0, 0, 0, 0, 0])
        .send()
        .await?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("NEEDS_AUTH");
    }
    if !status.is_success() {
        anyhow::bail!("Grok billing request failed (HTTP {status})");
    }
    Ok(resp.bytes().await?.to_vec())
}

fn fetch_agent_billing(timeout: Duration) -> Option<Value> {
    let mut child = Command::new("grok")
        .args(["agent", "--no-leader", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines().map_while(std::result::Result::ok) {
            let _ = tx.send(line);
        }
    });

    let result = (|| {
        let stdin = child.stdin.as_mut()?;
        let initialize = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "1",
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false
                }
            }
        });
        let billing = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "x.ai/billing",
            "params": {}
        });
        writeln!(stdin, "{}", serde_json::to_string(&initialize).ok()?).ok()?;
        writeln!(stdin, "{}", serde_json::to_string(&billing).ok()?).ok()?;
        stdin.flush().ok()?;

        let response = wait_for_rpc_response(&rx, 2, timeout)?;
        if response.get("error").is_some() {
            return None;
        }
        response.get("result").cloned()
    })();

    let _ = child.kill();
    let _ = child.wait();

    result
}

fn wait_for_rpc_response(
    rx: &mpsc::Receiver<String>,
    expected_id: i64,
    timeout: Duration,
) -> Option<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let line = rx.recv_timeout(remaining).ok()?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) == Some(expected_id) {
            return Some(value);
        }
    }
}

fn title_words(raw: &str) -> String {
    raw.replace(['_', '-'], " ")
        .split_whitespace()
        .map(|word| {
            let lower = word.to_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_subscription_tier(raw: &str) -> String {
    let trimmed = raw
        .trim_start_matches("SUBSCRIPTION_TIER_")
        .trim_start_matches("TIER_");
    title_words(trimmed)
}

fn parse_subscription_plan(value: &Value) -> Option<String> {
    let subscriptions = value.get("subscriptions")?.as_array()?;
    let chosen = subscriptions.iter().find(|sub| {
        sub.get("status")
            .and_then(Value::as_str)
            .map(|status| status.eq_ignore_ascii_case("active"))
            .unwrap_or(false)
    })?;

    let tier = chosen.get("tier").and_then(Value::as_str)?;
    Some(normalize_subscription_tier(tier))
}

fn numeric_value(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        return number.is_finite().then_some(number);
    }
    if let Some(text) = value.as_str() {
        return text.parse::<f64>().ok().filter(|number| number.is_finite());
    }
    value
        .as_object()
        .and_then(|object| object.get("val").or_else(|| object.get("value")))
        .and_then(numeric_value)
}

fn number_at(value: &Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    numeric_value(current)
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str().map(ToString::to_string)
}

fn epoch_at(value: &Value, path: &[&str]) -> Option<String> {
    number_at(value, path).and_then(|ts| epoch_to_rfc3339(ts as i64))
}

fn format_cents(cents: f64) -> String {
    format!("${:.2}", cents / 100.0)
}

fn cycle_label(start: Option<&str>, end: Option<&str>) -> String {
    let Some(start) = start else {
        return "Credits".into();
    };
    let Some(end) = end else {
        return "Credits".into();
    };
    let Ok(start) = chrono::DateTime::parse_from_rfc3339(start) else {
        return "Credits".into();
    };
    let Ok(end) = chrono::DateTime::parse_from_rfc3339(end) else {
        return "Credits".into();
    };
    let days = (end - start).num_days();
    if (6..=8).contains(&days) {
        "Weekly".into()
    } else if (27..=33).contains(&days) {
        "Monthly".into()
    } else {
        "Credits".into()
    }
}

fn parse_billing_json_metric(value: &Value) -> Option<UsageMetric> {
    if let Some(metric) = parse_billing_json_object(value) {
        return Some(metric);
    }
    match value {
        Value::Array(items) => items.iter().find_map(parse_billing_json_metric),
        Value::Object(object) => object.values().find_map(parse_billing_json_metric),
        _ => None,
    }
}

fn parse_billing_json_object(value: &Value) -> Option<UsageMetric> {
    let monthly_limit = number_at(value, &["monthlyLimit"])
        .or_else(|| number_at(value, &["config", "monthlyLimit"]));
    let total_used = number_at(value, &["usage", "totalUsed"])
        .or_else(|| number_at(value, &["totalUsed"]))
        .or_else(|| number_at(value, &["config", "usage", "totalUsed"]));
    let percent = if let (Some(limit), Some(used)) = (monthly_limit, total_used) {
        if limit > 0.0 {
            Some((used / limit * 100.0).clamp(0.0, 100.0))
        } else {
            None
        }
    } else {
        number_at(value, &["usedPercent"])
            .or_else(|| number_at(value, &["usagePercent"]))
            .or_else(|| number_at(value, &["creditUsagePercent"]))
    }?;
    let percent = percent.is_finite().then(|| percent.clamp(0.0, 100.0))?;

    let start = string_at(value, &["billingCycle", "billingPeriodStart"])
        .or_else(|| string_at(value, &["billingPeriodStart"]))
        .or_else(|| epoch_at(value, &["billingPeriodStart"]));
    let end = string_at(value, &["billingCycle", "billingPeriodEnd"])
        .or_else(|| string_at(value, &["billingPeriodEnd"]))
        .or_else(|| epoch_at(value, &["billingPeriodEnd"]));

    let remaining_label = if let (Some(limit), Some(used)) = (monthly_limit, total_used) {
        let remaining = (limit - used).max(0.0);
        Some(format!(
            "{}/{} left",
            format_cents(remaining),
            format_cents(limit)
        ))
    } else {
        None
    };

    Some(UsageMetric {
        label: cycle_label(start.as_deref(), end.as_deref()),
        used_percent: percent,
        remaining_percent: 100.0 - percent,
        remaining_label,
        resets_at: end,
    })
}

fn push_limit_metric(
    metrics: &mut Vec<UsageMetric>,
    label: &str,
    used: Option<f64>,
    limit: Option<f64>,
    reset: Option<String>,
) {
    let Some(limit) = limit.filter(|limit| *limit > 0.0) else {
        return;
    };
    let used = used.unwrap_or(0.0).clamp(0.0, limit);
    let used_percent = (used / limit * 100.0).clamp(0.0, 100.0);
    let remaining_label = format!("{:.0}/{:.0} left", limit - used, limit);
    if metrics.iter().any(|metric| {
        metric.label == label
            && (metric.used_percent - used_percent).abs() < 0.0001
            && metric.remaining_label.as_deref() == Some(remaining_label.as_str())
    }) {
        return;
    }
    metrics.push(UsageMetric {
        label: label.into(),
        used_percent,
        remaining_percent: 100.0 - used_percent,
        remaining_label: Some(remaining_label),
        resets_at: reset,
    });
}

fn collect_task_usage_metrics(value: &Value, metrics: &mut Vec<UsageMetric>) {
    if let Value::Object(object) = value {
        let reset = object
            .get("resetTime")
            .or_else(|| object.get("resetsAt"))
            .or_else(|| object.get("resetAt"))
            .and_then(Value::as_str)
            .map(ToString::to_string);
        push_limit_metric(
            metrics,
            "Tasks",
            object.get("usage").and_then(numeric_value),
            object.get("limit").and_then(numeric_value),
            reset.clone(),
        );
        push_limit_metric(
            metrics,
            "Frequent",
            object.get("frequentUsage").and_then(numeric_value),
            object.get("frequentLimit").and_then(numeric_value),
            reset.clone(),
        );
        push_limit_metric(
            metrics,
            "Occasional",
            object.get("occasionalUsage").and_then(numeric_value),
            object.get("occasionalLimit").and_then(numeric_value),
            reset,
        );
        for child in object.values() {
            collect_task_usage_metrics(child, metrics);
        }
    } else if let Value::Array(items) = value {
        for child in items {
            collect_task_usage_metrics(child, metrics);
        }
    }
}

fn read_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0_u64;
    let mut shift = 0_u32;
    while *pos < data.len() && shift <= 63 {
        let byte = data[*pos];
        *pos += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
    None
}

fn next_proto_field<'a>(data: &'a [u8], pos: &mut usize) -> Option<(u32, ProtoValue<'a>)> {
    let key = read_varint(data, pos)?;
    let field = u32::try_from(key >> 3).ok()?;
    let wire = key & 0x07;
    match wire {
        0 => read_varint(data, pos).map(|value| (field, ProtoValue::Varint(value))),
        1 => {
            if *pos + 8 > data.len() {
                return None;
            }
            *pos += 8;
            Some((field, ProtoValue::Fixed64))
        }
        2 => {
            let len = usize::try_from(read_varint(data, pos)?).ok()?;
            if *pos + len > data.len() {
                return None;
            }
            let bytes = &data[*pos..*pos + len];
            *pos += len;
            Some((field, ProtoValue::Bytes(bytes)))
        }
        5 => {
            if *pos + 4 > data.len() {
                return None;
            }
            let value =
                u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            Some((field, ProtoValue::Fixed32(value)))
        }
        _ => None,
    }
}

fn timestamp_message_to_rfc3339(data: &[u8]) -> Option<String> {
    let mut pos = 0;
    while pos < data.len() {
        let (field, value) = next_proto_field(data, &mut pos)?;
        if field == 1 {
            if let ProtoValue::Varint(seconds) = value {
                return epoch_to_rfc3339(i64::try_from(seconds).ok()?);
            }
        }
    }
    None
}

fn epoch_to_rfc3339(seconds: i64) -> Option<String> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .map(|dt| dt.to_rfc3339())
}

fn parse_billing_config_proto(data: &[u8]) -> Option<UsageMetric> {
    let mut pos = 0;
    let mut used_percent: Option<f64> = None;
    let mut start: Option<String> = None;
    let mut end: Option<String> = None;

    while pos < data.len() {
        let (field, value) = next_proto_field(data, &mut pos)?;
        match (field, value) {
            (1, ProtoValue::Fixed32(bits)) => {
                let percent = f32::from_bits(bits) as f64;
                if percent.is_finite() {
                    used_percent = Some(percent.clamp(0.0, 100.0));
                }
            }
            (4, ProtoValue::Bytes(bytes)) => {
                start = timestamp_message_to_rfc3339(bytes);
            }
            (5, ProtoValue::Bytes(bytes)) => {
                end = timestamp_message_to_rfc3339(bytes);
            }
            _ => {}
        }
    }

    let percent = used_percent?;
    Some(UsageMetric {
        label: cycle_label(start.as_deref(), end.as_deref()),
        used_percent: percent,
        remaining_percent: 100.0 - percent,
        remaining_label: None,
        resets_at: end,
    })
}

fn parse_grpc_billing_metric(body: &[u8]) -> Option<UsageMetric> {
    let mut pos = 0;
    while pos + 5 <= body.len() {
        let flag = body[pos];
        let len = u32::from_be_bytes([body[pos + 1], body[pos + 2], body[pos + 3], body[pos + 4]])
            as usize;
        pos += 5;
        if pos + len > body.len() {
            return None;
        }
        let payload = &body[pos..pos + len];
        pos += len;
        if flag & 0x80 != 0 {
            continue;
        }

        let mut payload_pos = 0;
        while payload_pos < payload.len() {
            let (field, value) = next_proto_field(payload, &mut payload_pos)?;
            if field == 1 {
                if let ProtoValue::Bytes(config) = value {
                    if let Some(metric) = parse_billing_config_proto(config) {
                        return Some(metric);
                    }
                }
            }
        }
    }
    None
}

fn fetch_network_usage(credentials: &Credentials) -> Result<UsageOutput> {
    let mut plan: Option<String> = None;
    let mut metrics = Vec::new();
    let mut errors = Vec::new();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = tokscale_core::http::client_builder()
            .timeout(Duration::from_secs(12))
            .build()?;

        let proxy = fetch_proxy_usage(
            &client,
            &credentials.token,
            GROK_CREDITS_URL,
            GROK_MONTHLY_URL,
        )
        .await;
        plan = proxy.plan;
        metrics.extend(proxy.metrics);
        errors.extend(proxy.errors);
        if metrics.is_empty() {
            match fetch_billing_grpc(&client, &credentials.token).await {
                Ok(body) => {
                    if let Some(metric) = parse_grpc_billing_metric(&body) {
                        metrics.push(metric);
                    } else {
                        errors.push("Grok billing response was not recognized".to_string());
                    }
                }
                Err(error) => errors.push(format!("Grok billing request failed: {error}")),
            }
        }

        if metrics.is_empty() {
            match fetch_task_usage(&client, &credentials.token).await {
                Ok(task_usage) => collect_task_usage_metrics(&task_usage, &mut metrics),
                Err(error) => errors.push(format!("Grok task usage request failed: {error}")),
            }
        }

        if plan.is_none() {
            match fetch_subscriptions(&client, &credentials.token).await {
                Ok(subscriptions) => {
                    plan = parse_subscription_plan(&subscriptions);
                }
                Err(error) => errors.push(format!("Grok subscriptions request failed: {error}")),
            }
        }

        Ok::<_, anyhow::Error>(())
    })?;

    if metrics.is_empty() && plan.is_none() {
        let detail = if errors.is_empty() {
            "no usage or active subscription data returned".to_string()
        } else {
            errors.join("; ")
        };
        anyhow::bail!("Grok usage unavailable: {detail}");
    }

    Ok(usage_output(plan, credentials.email.clone(), metrics))
}

#[derive(Default)]
struct ProxyUsage {
    plan: Option<String>,
    metrics: Vec<UsageMetric>,
    errors: Vec<String>,
}

async fn fetch_proxy_usage(
    client: &reqwest::Client,
    token: &str,
    weekly_url: &str,
    monthly_url: &str,
) -> ProxyUsage {
    let mut usage = ProxyUsage::default();
    // Prefer the cli-chat-proxy SuperGrok meters (weekly credits + monthly
    // allowance). Grok Bot is a different pool on the Cursor account.
    match fetch_proxy_billing(client, token, weekly_url).await {
        Ok(credits) => {
            if let Some(metric) = parse_proxy_weekly_metric(&credits) {
                usage.metrics.push(metric);
            } else {
                usage
                    .errors
                    .push("Grok weekly credits response was not recognized".to_string());
            }
            if let Some(tier) = credits
                .get("subscriptionTiers")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|tier| !tier.is_empty())
            {
                usage.plan = Some(normalize_subscription_tier(tier));
            }
        }
        Err(error) => usage
            .errors
            .push(format!("Grok weekly credits request failed: {error}")),
    }

    // The monthly allowance remains useful when the weekly endpoint
    // fails or changes its response shape.
    match fetch_proxy_billing(client, token, monthly_url).await {
        Ok(monthly) => {
            if let Some(metric) = parse_proxy_monthly_metric(&monthly) {
                usage.metrics.push(metric);
            }
            if usage.plan.is_none() {
                if let Some(tier) = monthly
                    .get("subscriptionTiers")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|tier| !tier.is_empty())
                {
                    usage.plan = Some(normalize_subscription_tier(tier));
                }
            }
        }
        Err(error) => usage
            .errors
            .push(format!("Grok monthly billing request failed: {error}")),
    }
    usage
}

async fn fetch_proxy_billing(client: &reqwest::Client, token: &str, url: &str) -> Result<Value> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("User-Agent", GROK_USER_AGENT)
        .send()
        .await?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("NEEDS_AUTH");
    }
    if !status.is_success() {
        anyhow::bail!("Grok proxy billing request failed (HTTP {status})");
    }
    let text = resp.text().await?;
    if text.trim_start().starts_with('<') {
        anyhow::bail!("Grok proxy billing returned HTML");
    }
    Ok(serde_json::from_str(&text)?)
}

fn parse_proxy_weekly_metric(value: &Value) -> Option<UsageMetric> {
    let config = value.get("config")?;
    let used_percent = match proxy_weekly_used_percent(config) {
        Some(percent) => percent,
        None => {
            // Empty week after reset: period present, percent fields omitted.
            if proxy_self_contained_weekly_period(config).is_some() {
                0.0
            } else {
                return None;
            }
        }
    };
    let end = proxy_period_end(config);
    Some(UsageMetric {
        label: "Weekly".into(),
        used_percent,
        remaining_percent: 100.0 - used_percent,
        remaining_label: None,
        resets_at: end,
    })
}

fn proxy_weekly_used_percent(config: &Value) -> Option<f64> {
    let percent = match config.get("creditUsagePercent")? {
        Value::Number(number) => number.as_f64()?,
        Value::String(raw) => raw.parse::<f64>().ok()?,
        _ => return None,
    };
    percent.is_finite().then(|| percent.clamp(0.0, 100.0))
}

fn proxy_self_contained_weekly_period(config: &Value) -> Option<()> {
    let period = config.get("currentPeriod")?;
    if period.get("type").and_then(Value::as_str) != Some(WEEKLY_PERIOD_TYPE) {
        return None;
    }
    let start = period.get("start").and_then(Value::as_str)?;
    let end = period.get("end").and_then(Value::as_str)?;
    let start = chrono::DateTime::parse_from_rfc3339(start).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(end).ok()?;
    (end > start).then_some(())
}

fn proxy_period_end(config: &Value) -> Option<String> {
    config
        .pointer("/currentPeriod/end")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            string_at(config, &["billingPeriodEnd"])
                .or_else(|| epoch_at(config, &["billingPeriodEnd"]))
        })
}

fn parse_proxy_monthly_metric(value: &Value) -> Option<UsageMetric> {
    let config = value.get("config").unwrap_or(value);
    let limit = number_at(config, &["monthlyLimit", "val"])
        .or_else(|| number_at(config, &["monthlyLimit"]))
        .filter(|limit| *limit > 0.0)?;
    let used = number_at(config, &["used", "val"])
        .or_else(|| number_at(config, &["used"]))
        .or_else(|| number_at(config, &["usage", "totalUsed", "val"]))
        .or_else(|| number_at(config, &["usage", "totalUsed"]))
        .filter(|used| *used >= 0.0)?;
    let used_percent = (used / limit * 100.0).clamp(0.0, 100.0);
    let end = proxy_period_end(config).or_else(|| {
        string_at(config, &["billingCycle", "billingPeriodEnd"])
            .or_else(|| string_at(config, &["billingPeriodEnd"]))
    });
    Some(UsageMetric {
        label: "Monthly".into(),
        used_percent,
        remaining_percent: 100.0 - used_percent,
        remaining_label: Some(format!(
            "{}/{} left",
            format_cents((limit - used).max(0.0)),
            format_cents(limit)
        )),
        resets_at: end,
    })
}

fn usage_output(
    plan: Option<String>,
    email: Option<String>,
    metrics: Vec<UsageMetric>,
) -> UsageOutput {
    UsageOutput {
        provider: "Grok".into(),
        account: None,
        credential_source: None,
        plan,
        email,
        metrics,
        reset_credits: None,
        credit_status: None,
        spend_control: None,
    }
}

pub fn fetch() -> Result<UsageOutput> {
    let credentials = read_credentials()?;
    let mut errors = Vec::new();
    let mut plan_only: Option<UsageOutput> = None;

    for (index, credential) in credentials.iter().enumerate() {
        match fetch_network_usage(credential) {
            Ok(output) if !output.metrics.is_empty() => return Ok(output),
            Ok(output) => {
                if plan_only.is_none() {
                    plan_only = Some(output);
                }
            }
            Err(error) => errors.push(format!("Grok credential #{} failed: {error}", index + 1)),
        }
    }

    // The `grok agent --no-leader stdio` billing fallback runs without
    // credential context, so its metrics cannot be attributed to a specific
    // account. Skip it when auth.json holds multiple credentials to avoid
    // merging metrics with a plan/email that belongs to a different account.
    if credentials.len() > 1 {
        errors
            .push("Grok agent billing fallback skipped: multiple credentials present".to_string());
    } else if let Some(billing) = fetch_agent_billing(Duration::from_secs(4)) {
        let mut metrics = Vec::new();
        if let Some(metric) = parse_billing_json_metric(&billing) {
            metrics.push(metric);
        }
        collect_task_usage_metrics(&billing, &mut metrics);
        if !metrics.is_empty() {
            return Ok(usage_output(
                plan_only.as_ref().and_then(|output| output.plan.clone()),
                plan_only
                    .as_ref()
                    .and_then(|output| output.email.clone())
                    .or_else(|| {
                        credentials
                            .first()
                            .and_then(|credential| credential.email.clone())
                    }),
                metrics,
            ));
        }
    } else {
        errors.push("Grok agent billing RPC unavailable".to_string());
    }

    if let Some(output) = plan_only {
        return Ok(output);
    }

    let detail = if errors.is_empty() {
        "no usage or active subscription data returned".to_string()
    } else {
        errors.join("; ")
    };
    anyhow::bail!("Grok usage unavailable: {detail}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::usage::test_server::spawn_server;

    #[test]
    fn fallback_uses_registered_grok_provider() {
        let output = usage_output(None, None, Vec::new());
        assert_eq!(output.provider, "Grok");
    }

    #[test]
    fn monthly_meter_survives_weekly_failure_or_unrecognized_response() {
        for weekly_status in [503, 200] {
            let (base, log) = spawn_server(move |path, _| match path {
                "/weekly" => (weekly_status, "{}".to_string()),
                "/monthly" => (
                    200,
                    serde_json::json!({
                        "subscriptionTiers": "super_grok",
                        "config": {
                            "monthlyLimit": { "val": 20000 },
                            "used": { "val": 5000 }
                        }
                    })
                    .to_string(),
                ),
                _ => (404, "{}".to_string()),
            });
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let usage = runtime.block_on(async {
                let client = reqwest::Client::new();
                fetch_proxy_usage(
                    &client,
                    "test-token",
                    &format!("{base}/weekly"),
                    &format!("{base}/monthly"),
                )
                .await
            });

            assert_eq!(usage.metrics.len(), 1, "weekly status {weekly_status}");
            assert_eq!(usage.metrics[0].label, "Monthly");
            assert_eq!(usage.metrics[0].remaining_percent, 75.0);
            assert_eq!(usage.plan.as_deref(), Some("Super Grok"));
            assert_eq!(log.lock().unwrap().len(), 2);
        }
    }

    #[test]
    fn proxy_weekly_subscription_tier_is_normalized() {
        let (base, _) = spawn_server(move |path, _| match path {
            "/weekly" => (
                200,
                serde_json::json!({
                    "subscriptionTiers": "super_grok",
                    "config": { "creditUsagePercent": 25 }
                })
                .to_string(),
            ),
            "/monthly" => (503, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let usage = runtime.block_on(async {
            let client = reqwest::Client::new();
            fetch_proxy_usage(
                &client,
                "test-token",
                &format!("{base}/weekly"),
                &format!("{base}/monthly"),
            )
            .await
        });
        assert_eq!(usage.plan.as_deref(), Some("Super Grok"));
        assert_eq!(usage.metrics.len(), 1);
        assert_eq!(usage.metrics[0].label, "Weekly");
        assert_eq!(usage.metrics[0].remaining_percent, 75.0);
    }

    fn push_varint(mut value: u64, out: &mut Vec<u8>) {
        while value >= 0x80 {
            out.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
    }

    fn push_len_field(field: u64, payload: &[u8], out: &mut Vec<u8>) {
        push_varint((field << 3) | 2, out);
        push_varint(payload.len() as u64, out);
        out.extend_from_slice(payload);
    }

    fn push_fixed32_field(field: u64, value: u32, out: &mut Vec<u8>) {
        push_varint((field << 3) | 5, out);
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn timestamp_message(seconds: u64) -> Vec<u8> {
        let mut out = Vec::new();
        push_varint(1 << 3, &mut out);
        push_varint(seconds, &mut out);
        out
    }

    #[test]
    fn parses_billing_json_metric() {
        let value = serde_json::json!({
            "billingCycle": {
                "billingPeriodStart": "2026-06-01T00:00:00Z",
                "billingPeriodEnd": "2026-07-01T00:00:00Z"
            },
            "monthlyLimit": { "val": 10000 },
            "usage": {
                "includedUsed": { "val": 1250 },
                "onDemandUsed": { "val": 0 },
                "totalUsed": { "val": 1250 }
            }
        });

        let metric = parse_billing_json_metric(&value).expect("billing metric");
        assert_eq!(metric.label, "Monthly");
        assert_eq!(metric.used_percent, 12.5);
        assert_eq!(
            metric.remaining_label.as_deref(),
            Some("$87.50/$100.00 left")
        );
        assert_eq!(metric.resets_at.as_deref(), Some("2026-07-01T00:00:00Z"));
    }

    #[test]
    fn parses_proxy_weekly_and_monthly_metrics() {
        let weekly = serde_json::json!({
            "subscriptionTiers": "super_grok",
            "config": {
                "creditUsagePercent": 42.5,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-06-01T00:00:00Z",
                    "end": "2026-06-08T00:00:00Z"
                }
            }
        });
        let weekly_metric = parse_proxy_weekly_metric(&weekly).expect("weekly");
        assert_eq!(weekly_metric.label, "Weekly");
        assert!((weekly_metric.used_percent - 42.5).abs() < f64::EPSILON);
        assert_eq!(
            weekly_metric.resets_at.as_deref(),
            Some("2026-06-08T00:00:00Z")
        );

        let weekly_string = serde_json::json!({
            "config": {
                "creditUsagePercent": "18",
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-06-01T00:00:00Z",
                    "end": "2026-06-08T00:00:00Z"
                }
            }
        });
        let string_metric = parse_proxy_weekly_metric(&weekly_string).expect("weekly string");
        assert!((string_metric.used_percent - 18.0).abs() < f64::EPSILON);

        let monthly = serde_json::json!({
            "config": {
                "monthlyLimit": { "val": 20000 },
                "used": { "val": 5000 },
                "billingPeriodEnd": "2026-07-01T00:00:00Z"
            }
        });
        let monthly_metric = parse_proxy_monthly_metric(&monthly).expect("monthly");
        assert_eq!(monthly_metric.label, "Monthly");
        assert!((monthly_metric.used_percent - 25.0).abs() < f64::EPSILON);
        assert_eq!(
            monthly_metric.remaining_label.as_deref(),
            Some("$150.00/$200.00 left")
        );

        let zero_monthly = serde_json::json!({
            "config": {
                "monthlyLimit": { "val": 0 },
                "used": { "val": 0 }
            }
        });
        assert!(parse_proxy_monthly_metric(&zero_monthly).is_none());
    }

    #[test]
    fn rejects_non_finite_billing_json_percentages() {
        for value in [
            serde_json::json!({ "usedPercent": "NaN" }),
            serde_json::json!({ "usedPercent": "inf" }),
            serde_json::json!({
                "monthlyLimit": "NaN",
                "usage": { "totalUsed": 10 }
            }),
        ] {
            assert!(parse_billing_json_metric(&value).is_none());
        }
    }

    #[test]
    fn reads_multiple_credential_candidates_with_auth_scope_first() {
        let value = serde_json::json!({
            "https://example.com": {
                "key": "secondary-token",
                "email": "secondary@example.com"
            },
            "https://auth.x.ai": {
                "key": "primary-token",
                "email": "primary@example.com"
            }
        });

        let credentials = credential_candidates_from_value(&value).expect("credential candidates");
        assert_eq!(credentials.len(), 2);
        assert_eq!(credentials[0].token, "primary-token");
        assert_eq!(credentials[0].email.as_deref(), Some("primary@example.com"));
        assert_eq!(credentials[1].token, "secondary-token");
        assert_eq!(
            credentials[1].email.as_deref(),
            Some("secondary@example.com")
        );
    }

    #[test]
    fn parses_grpc_billing_percent_frame() {
        let mut config = Vec::new();
        push_fixed32_field(1, 25.0_f32.to_bits(), &mut config);
        push_len_field(4, &timestamp_message(1_780_272_000), &mut config);
        push_len_field(5, &timestamp_message(1_782_864_000), &mut config);

        let mut message = Vec::new();
        push_len_field(1, &config, &mut message);

        let mut frame = Vec::new();
        frame.push(0);
        frame.extend_from_slice(&(message.len() as u32).to_be_bytes());
        frame.extend_from_slice(&message);

        let metric = parse_grpc_billing_metric(&frame).expect("billing metric");
        assert_eq!(metric.label, "Monthly");
        assert_eq!(metric.used_percent, 25.0);
        assert_eq!(
            metric.resets_at.as_deref(),
            Some("2026-07-01T00:00:00+00:00")
        );
    }

    #[test]
    fn parses_task_usage_metrics() {
        let value = serde_json::json!({
            "frequentUsage": 3,
            "frequentLimit": 10,
            "occasionalUsage": 1,
            "occasionalLimit": 5
        });
        let mut metrics = Vec::new();
        collect_task_usage_metrics(&value, &mut metrics);

        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].label, "Frequent");
        assert_eq!(metrics[0].used_percent, 30.0);
        assert_eq!(metrics[1].label, "Occasional");
        assert_eq!(metrics[1].used_percent, 20.0);
    }

    #[test]
    fn normalizes_grok_subscription_plan() {
        let value = serde_json::json!({
            "subscriptions": [
                {
                    "tier": "SUBSCRIPTION_TIER_SUPER_GROK_PRO",
                    "status": "active"
                }
            ]
        });
        assert_eq!(
            parse_subscription_plan(&value).as_deref(),
            Some("Super Grok Pro")
        );
    }

    #[test]
    fn ignores_inactive_subscription_plan() {
        let value = serde_json::json!({
            "subscriptions": [
                {
                    "tier": "SUBSCRIPTION_TIER_GROK_PRO",
                    "status": "inactive"
                }
            ]
        });

        assert_eq!(parse_subscription_plan(&value), None);
    }

    #[test]
    fn prefers_active_subscription_plan() {
        let value = serde_json::json!({
            "subscriptions": [
                {
                    "tier": "SUBSCRIPTION_TIER_GROK_PRO",
                    "status": "inactive"
                },
                {
                    "tier": "SUBSCRIPTION_TIER_SUPER_GROK_PRO",
                    "status": "active"
                }
            ]
        });

        assert_eq!(
            parse_subscription_plan(&value).as_deref(),
            Some("Super Grok Pro")
        );
    }
}
