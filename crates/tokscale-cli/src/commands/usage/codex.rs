use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::helpers::capitalize;
use super::{UsageAccount, UsageMetric, UsageOutput};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Auth {
    tokens: Option<Tokens>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tokens {
    #[serde(skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Usage {
    email: Option<String>,
    plan_type: Option<String>,
    rate_limit: Option<RateLimit>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct RateLimit {
    primary_window: Option<Window>,
    secondary_window: Option<Window>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Window {
    used_percent: Option<i64>,
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Refresh {
    access_token: Option<String>,
    refresh_token: Option<String>,
    #[allow(dead_code)]
    expires_in: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccount {
    tokens: Tokens,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodexCredentialsStore {
    version: i32,
    #[serde(rename = "activeAccountId")]
    active_account_id: String,
    accounts: HashMap<String, CodexAccount>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexAccountInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(rename = "accountId", skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "isActive")]
    pub is_active: bool,
}

#[derive(Debug, Clone)]
enum CredentialSource {
    File(PathBuf),
    Keychain,
    Store(String),
}

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

fn codex_store_path() -> PathBuf {
    crate::paths::get_config_dir().join("codex-credentials.json")
}

#[cfg(test)]
fn codex_store_path_in_home(home_dir: &Path) -> PathBuf {
    home_dir
        .join(".config")
        .join("tokscale")
        .join("codex-credentials.json")
}

fn current_auth_paths() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let mut paths = Vec::new();

    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        if !codex_home.trim().is_empty() {
            paths.push(PathBuf::from(codex_home).join("auth.json"));
        }
    }

    paths.push(home.join(".config").join("codex").join("auth.json"));
    paths.push(home.join(".codex").join("auth.json"));
    paths
}

fn auth_write_path() -> Result<PathBuf> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        if !codex_home.trim().is_empty() {
            return Ok(PathBuf::from(codex_home).join("auth.json"));
        }
    }

    let home = home_dir()?;
    let config_path = home.join(".config").join("codex").join("auth.json");
    if config_path.exists() {
        return Ok(config_path);
    }

    let legacy_path = home.join(".codex").join("auth.json");
    if legacy_path.exists() {
        return Ok(legacy_path);
    }

    Ok(config_path)
}

fn read_current_credentials() -> Result<(Auth, CredentialSource)> {
    for p in current_auth_paths() {
        if p.exists() {
            let content = std::fs::read_to_string(&p)?;
            if let Ok(auth) = serde_json::from_str::<Auth>(&content) {
                if auth
                    .tokens
                    .as_ref()
                    .and_then(|t| t.access_token.as_ref())
                    .is_some()
                {
                    return Ok((auth, CredentialSource::File(p)));
                }
            }
        }
    }

    if let Ok(raw) = super::helpers::read_keychain("Codex Auth") {
        if let Ok(auth) = serde_json::from_str::<Auth>(&raw) {
            if auth
                .tokens
                .as_ref()
                .and_then(|t| t.access_token.as_ref())
                .is_some()
            {
                return Ok((auth, CredentialSource::Keychain));
            }
        }
    }

    anyhow::bail!("No Codex credentials found. Run 'codex' to log in.")
}

fn auth_document(tokens: &Tokens) -> serde_json::Value {
    serde_json::json!({
        "tokens": tokens,
        "last_refresh": chrono::Utc::now().to_rfc3339(),
    })
}

fn save_auth_tokens(path: &Path, tokens: &Tokens) -> Result<()> {
    let content = serde_json::to_string_pretty(&auth_document(tokens))?;
    super::helpers::atomic_write_secret(path, content.as_bytes())
        .with_context(|| format!("Failed to write Codex auth to {}", path.display()))
}

fn remove_auth_file_if_account_matches(
    path: &Path,
    account_id: &str,
    account_tokens: &Tokens,
) -> Result<()> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(auth) = serde_json::from_str::<Auth>(&content) else {
        return Ok(());
    };
    let Some(tokens) = auth.tokens else {
        return Ok(());
    };

    if derive_account_id(&tokens) == account_id || same_token_identity(&tokens, account_tokens) {
        std::fs::remove_file(path)
            .with_context(|| format!("Failed to remove Codex auth at {}", path.display()))?;
    }

    Ok(())
}

fn persist_tokens(source: &CredentialSource, tokens: &Tokens) {
    match source {
        CredentialSource::File(path) => {
            if let Err(e) = save_auth_tokens(path, tokens) {
                eprintln!("warning: failed to save Codex credentials: {e}");
            }
        }
        CredentialSource::Store(account_id) => {
            if let Err(e) = update_account_tokens(account_id, tokens.clone()) {
                eprintln!("warning: failed to save Codex account credentials: {e}");
            }
        }
        CredentialSource::Keychain => {}
    }
}

fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

fn derive_account_id(tokens: &Tokens) -> String {
    if let Some(account_id) = tokens.account_id.as_deref() {
        let trimmed = account_id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Some(id_token) = tokens.id_token.as_deref() {
        let trimmed = id_token.trim();
        if !trimmed.is_empty() {
            return format!("id-{}", hash_token(trimmed));
        }
    }

    tokens
        .access_token
        .as_deref()
        .map(|token| format!("token-{}", hash_token(token)))
        .unwrap_or_else(|| "account".to_string())
}

fn normalized_token_field(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn same_token_identity(a: &Tokens, b: &Tokens) -> bool {
    match (
        normalized_token_field(a.account_id.as_deref()),
        normalized_token_field(b.account_id.as_deref()),
    ) {
        (Some(a_id), Some(b_id)) => return a_id == b_id,
        (Some(_), None) | (None, Some(_)) => {}
        (None, None) => {}
    }

    match (
        normalized_token_field(a.id_token.as_deref()),
        normalized_token_field(b.id_token.as_deref()),
    ) {
        (Some(a_id), Some(b_id)) => return a_id == b_id,
        (Some(_), None) | (None, Some(_)) => {}
        (None, None) => {}
    }

    match (
        normalized_token_field(a.access_token.as_deref()),
        normalized_token_field(b.access_token.as_deref()),
    ) {
        (Some(a_token), Some(b_token)) => a_token == b_token,
        _ => false,
    }
}

fn next_available_account_id(store: &CodexCredentialsStore, base_id: &str) -> String {
    if !store.accounts.contains_key(base_id) {
        return base_id.to_string();
    }

    for suffix in 2usize.. {
        let candidate = format!("{base_id}-{suffix}");
        if !store.accounts.contains_key(&candidate) {
            return candidate;
        }
    }

    unreachable!("unbounded suffix search must eventually find an unused Codex account id")
}

fn validate_label_available(
    store: &CodexCredentialsStore,
    account_id: &str,
    label: Option<&str>,
) -> Result<()> {
    let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) else {
        return Ok(());
    };
    let needle = label.to_lowercase();

    for (id, account) in &store.accounts {
        if id == account_id {
            continue;
        }
        if account
            .label
            .as_deref()
            .map(str::trim)
            .map(str::to_lowercase)
            .as_deref()
            == Some(needle.as_str())
        {
            anyhow::bail!("Codex account label already exists: {label}");
        }
    }

    Ok(())
}

pub fn load_credentials_store() -> Option<CodexCredentialsStore> {
    load_credentials_store_from_path(&codex_store_path())
}

#[cfg(test)]
fn load_credentials_store_from_home(home_dir: &Path) -> Option<CodexCredentialsStore> {
    load_credentials_store_from_path(&codex_store_path_in_home(home_dir))
}

fn load_credentials_store_from_path(path: &Path) -> Option<CodexCredentialsStore> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut store = serde_json::from_str::<CodexCredentialsStore>(&content).ok()?;

    if store.version != 1 || store.accounts.is_empty() {
        return None;
    }

    if !store.accounts.contains_key(&store.active_account_id) {
        if let Some(first_id) = first_account_id(&store) {
            store.active_account_id = first_id;
            let _ = save_credentials_store_at_path(path, &store);
        }
    }

    Some(store)
}

fn save_credentials_store(store: &CodexCredentialsStore) -> Result<()> {
    save_credentials_store_at_path(&codex_store_path(), store)
}

#[cfg(test)]
fn save_credentials_store_in_home(home_dir: &Path, store: &CodexCredentialsStore) -> Result<()> {
    let path = codex_store_path_in_home(home_dir);
    save_credentials_store_at_path(&path, store)
}

fn save_credentials_store_at_path(path: &Path, store: &CodexCredentialsStore) -> Result<()> {
    let json = serde_json::to_string_pretty(store)?;
    super::helpers::atomic_write_secret(path, json.as_bytes())
        .with_context(|| format!("Failed to write Codex account store to {}", path.display()))
}

fn resolve_account_id(store: &CodexCredentialsStore, name_or_id: &str) -> Option<String> {
    let needle = name_or_id.trim();
    if needle.is_empty() {
        return None;
    }

    if store.accounts.contains_key(needle) {
        return Some(needle.to_string());
    }

    let needle_lower = needle.to_lowercase();
    for (id, account) in &store.accounts {
        if account
            .label
            .as_deref()
            .map(str::trim)
            .map(str::to_lowercase)
            .as_deref()
            == Some(needle_lower.as_str())
        {
            return Some(id.clone());
        }
    }

    None
}

fn account_info(
    store: &CodexCredentialsStore,
    account_id: &str,
    account: &CodexAccount,
) -> CodexAccountInfo {
    CodexAccountInfo {
        id: account_id.to_string(),
        label: account.label.clone(),
        account_id: account.tokens.account_id.clone(),
        created_at: account.created_at.clone(),
        is_active: account_id == store.active_account_id,
    }
}

fn first_account_id(store: &CodexCredentialsStore) -> Option<String> {
    store
        .accounts
        .iter()
        .min_by(|(a_id, a), (b_id, b)| {
            let a_name = a.label.as_deref().unwrap_or(a_id).to_lowercase();
            let b_name = b.label.as_deref().unwrap_or(b_id).to_lowercase();
            a_name.cmp(&b_name).then_with(|| a_id.cmp(b_id))
        })
        .map(|(id, _)| id.clone())
}

struct RemovedCodexAccount {
    info: CodexAccountInfo,
    tokens: Tokens,
    next_active_tokens: Option<Tokens>,
}

fn remove_account_from_store(
    store: &mut CodexCredentialsStore,
    name_or_id: &str,
) -> Result<RemovedCodexAccount> {
    let resolved = resolve_account_id(store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {name_or_id}"))?;
    let removed_was_active = store.active_account_id == resolved;
    let account = store
        .accounts
        .remove(&resolved)
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {resolved}"))?;
    let removed_tokens = account.tokens.clone();
    let removed = CodexAccountInfo {
        id: resolved,
        label: account.label,
        account_id: account.tokens.account_id.clone(),
        created_at: account.created_at,
        is_active: removed_was_active,
    };

    let next_active_tokens = if removed_was_active {
        if let Some(next_id) = first_account_id(store) {
            store.active_account_id = next_id.clone();
            store.accounts.get(&next_id).map(|a| a.tokens.clone())
        } else {
            store.active_account_id.clear();
            None
        }
    } else {
        None
    };

    Ok(RemovedCodexAccount {
        info: removed,
        tokens: removed_tokens,
        next_active_tokens,
    })
}

pub fn list_accounts() -> Vec<CodexAccountInfo> {
    let store = match load_credentials_store() {
        Some(store) => store,
        None => return Vec::new(),
    };

    let mut accounts: Vec<_> = store
        .accounts
        .iter()
        .map(|(id, account)| account_info(&store, id, account))
        .collect();

    accounts.sort_by(|a, b| {
        if a.is_active != b.is_active {
            return if a.is_active {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        let la = a.label.as_deref().unwrap_or(&a.id).to_lowercase();
        let lb = b.label.as_deref().unwrap_or(&b.id).to_lowercase();
        la.cmp(&lb)
    });

    accounts
}

fn save_account_from_auth(auth: Auth, label: Option<&str>) -> Result<CodexAccountInfo> {
    save_account_from_auth_at_path(&codex_store_path(), auth, label, true)
}

#[cfg(test)]
fn save_account_from_auth_in_home(
    home_dir: &Path,
    auth: Auth,
    label: Option<&str>,
) -> Result<CodexAccountInfo> {
    save_account_from_auth_at_path(&codex_store_path_in_home(home_dir), auth, label, true)
}

#[cfg(test)]
fn save_account_from_auth_in_home_with_active(
    home_dir: &Path,
    auth: Auth,
    label: Option<&str>,
    make_active: bool,
) -> Result<CodexAccountInfo> {
    save_account_from_auth_at_path(
        &codex_store_path_in_home(home_dir),
        auth,
        label,
        make_active,
    )
}

fn save_account_from_auth_at_path(
    store_path: &Path,
    auth: Auth,
    label: Option<&str>,
    make_active: bool,
) -> Result<CodexAccountInfo> {
    let tokens = auth
        .tokens
        .ok_or_else(|| anyhow::anyhow!("No Codex tokens."))?;
    if tokens
        .access_token
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        anyhow::bail!("No Codex access token.");
    }

    let base_account_id = derive_account_id(&tokens);
    let mut store =
        load_credentials_store_from_path(store_path).unwrap_or_else(|| CodexCredentialsStore {
            version: 1,
            active_account_id: base_account_id.clone(),
            accounts: HashMap::new(),
        });

    let existing_same_identity = store
        .accounts
        .get(&base_account_id)
        .map(|existing| same_token_identity(&existing.tokens, &tokens))
        .unwrap_or(false);

    if existing_same_identity {
        validate_label_available(&store, &base_account_id, label)?;
        if let Some(existing) = store.accounts.get_mut(&base_account_id) {
            existing.tokens = tokens;
            if let Some(label) = label.map(str::trim).filter(|s| !s.is_empty()) {
                existing.label = Some(label.to_string());
            }
        }
        if make_active {
            store.active_account_id = base_account_id.clone();
        }
        save_credentials_store_at_path(store_path, &store)?;

        let account = store
            .accounts
            .get(&base_account_id)
            .ok_or_else(|| anyhow::anyhow!("Failed to save Codex account"))?;
        return Ok(account_info(&store, &base_account_id, account));
    }

    let account_id = if store.accounts.contains_key(&base_account_id) {
        next_available_account_id(&store, &base_account_id)
    } else {
        base_account_id
    };

    validate_label_available(&store, &account_id, label)?;

    let account = CodexAccount {
        tokens,
        created_at: chrono::Utc::now().to_rfc3339(),
        label: label
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };

    store.accounts.insert(account_id.clone(), account);
    if make_active || store.active_account_id.trim().is_empty() {
        store.active_account_id = account_id.clone();
    }
    save_credentials_store_at_path(store_path, &store)?;

    let account = store
        .accounts
        .get(&account_id)
        .ok_or_else(|| anyhow::anyhow!("Failed to save Codex account"))?;
    Ok(account_info(&store, &account_id, account))
}

pub fn import_auth_file_without_activating(
    path: &Path,
    label: Option<&str>,
) -> Result<CodexAccountInfo> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read Codex auth from {}", path.display()))?;
    let auth = serde_json::from_str::<Auth>(&content)
        .with_context(|| format!("Failed to parse Codex auth from {}", path.display()))?;
    save_account_from_auth_at_path(&codex_store_path(), auth, label, false)
}

fn update_account_tokens(account_id: &str, tokens: Tokens) -> Result<()> {
    let mut store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Codex accounts"))?;
    let account = store
        .accounts
        .get_mut(account_id)
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {account_id}"))?;
    account.tokens = tokens;
    save_credentials_store(&store)
}

fn load_account(name_or_id: Option<&str>) -> Result<(String, CodexAccount, CodexAccountInfo)> {
    let store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Codex accounts"))?;
    let resolved = match name_or_id {
        Some(name) => resolve_account_id(&store, name)
            .ok_or_else(|| anyhow::anyhow!("Codex account not found: {name}"))?,
        None => store.active_account_id.clone(),
    };
    let account = store
        .accounts
        .get(&resolved)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {resolved}"))?;
    let info = account_info(&store, &resolved, &account);
    Ok((resolved, account, info))
}

fn auth_from_account(account: &CodexAccount) -> Auth {
    Auth {
        tokens: Some(account.tokens.clone()),
    }
}

pub fn has_credentials() -> bool {
    if load_credentials_store()
        .map(|store| !store.accounts.is_empty())
        .unwrap_or(false)
    {
        return true;
    }

    read_current_credentials().is_ok()
}

async fn refresh_token(client: &reqwest::Client, rt: &str) -> Result<Refresh> {
    let resp = client
        .post("https://auth.openai.com/oauth/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", rt),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Codex token refresh failed (HTTP {})", resp.status());
    }
    Ok(resp.json().await?)
}

async fn fetch_usage(
    client: &reqwest::Client,
    token: &str,
    account_id: Option<&str>,
) -> Result<Usage> {
    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        );
    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let resp = req.send().await?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("NEEDS_AUTH");
    }
    if !status.is_success() {
        anyhow::bail!("Codex usage request failed (HTTP {status})");
    }
    let body = resp.text().await?;
    if body.trim().starts_with('<') {
        anyhow::bail!("NEEDS_AUTH");
    }
    Ok(serde_json::from_str(&body)?)
}

fn metric_from_window(label: &str, window: &Window) -> UsageMetric {
    let pct = window.used_percent.unwrap_or(0).clamp(0, 100) as f64;
    UsageMetric {
        label: label.into(),
        used_percent: pct,
        remaining_percent: 100.0 - pct,
        remaining_label: None,
        resets_at: window
            .reset_at
            .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
            .map(|dt| dt.to_rfc3339()),
    }
}

async fn fetch_with_auth_async(
    auth: Auth,
    source: CredentialSource,
    provider_name: String,
    account: Option<UsageAccount>,
) -> Result<UsageOutput> {
    let tokens = auth
        .tokens
        .ok_or_else(|| anyhow::anyhow!("No Codex tokens."))?;
    let access_token = tokens
        .access_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No Codex access token."))?;

    let client = reqwest::Client::new();
    let resp = match fetch_usage(&client, &access_token, tokens.account_id.as_deref()).await {
        Ok(r) => r,
        Err(e) if e.to_string().contains("NEEDS_AUTH") => {
            let rt_str = tokens
                .refresh_token
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No refresh token."))?;
            let refreshed = refresh_token(&client, rt_str).await?;
            let new = refreshed
                .access_token
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Refresh returned no token."))?;

            let mut updated_tokens = tokens.clone();
            updated_tokens.access_token = Some(new.clone());
            if let Some(new_rt) = refreshed.refresh_token {
                updated_tokens.refresh_token = Some(new_rt);
            }
            persist_tokens(&source, &updated_tokens);

            fetch_usage(&client, &new, updated_tokens.account_id.as_deref()).await?
        }
        Err(e) => return Err(e),
    };

    let plan = resp.plan_type.as_deref().map(capitalize);
    let mut metrics = Vec::new();
    if let Some(ref rl) = resp.rate_limit {
        if let Some(ref w) = rl.primary_window {
            metrics.push(metric_from_window("Session", w));
        }
        if let Some(ref w) = rl.secondary_window {
            metrics.push(metric_from_window("Weekly", w));
        }
    }

    Ok(UsageOutput {
        provider: provider_name,
        account,
        plan,
        email: resp.email,
        metrics,
    })
}

fn fetch_with_auth(
    auth: Auth,
    source: CredentialSource,
    provider_name: String,
    account: Option<UsageAccount>,
) -> Result<UsageOutput> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(fetch_with_auth_async(auth, source, provider_name, account))
}

pub fn fetch() -> Result<UsageOutput> {
    let (auth, source) = read_current_credentials()?;
    fetch_with_auth(auth, source, "Codex".into(), None)
}

fn usage_account_from_saved(
    store: &CodexCredentialsStore,
    account_id: &str,
    account: &CodexAccount,
) -> UsageAccount {
    UsageAccount {
        id: account_id.to_string(),
        label: account.label.clone(),
        is_active: account_id == store.active_account_id,
    }
}

pub fn fetch_all() -> Result<Vec<UsageOutput>> {
    let Some(store) = load_credentials_store() else {
        return fetch().map(|output| vec![output]);
    };

    if store.accounts.is_empty() {
        return fetch().map(|output| vec![output]);
    }

    let mut account_ids: Vec<_> = store.accounts.keys().cloned().collect();
    account_ids.sort_by(|a, b| {
        if a == &store.active_account_id {
            std::cmp::Ordering::Less
        } else if b == &store.active_account_id {
            std::cmp::Ordering::Greater
        } else {
            let la = store
                .accounts
                .get(a)
                .and_then(|account| account.label.as_deref())
                .unwrap_or(a)
                .to_lowercase();
            let lb = store
                .accounts
                .get(b)
                .and_then(|account| account.label.as_deref())
                .unwrap_or(b)
                .to_lowercase();
            la.cmp(&lb)
        }
    });

    let mut outputs = Vec::new();
    let mut first_error = None;
    for account_id in account_ids {
        let Some(account) = store.accounts.get(&account_id) else {
            continue;
        };
        let usage_account = usage_account_from_saved(&store, &account_id, account);
        match fetch_with_auth(
            auth_from_account(account),
            CredentialSource::Store(account_id.clone()),
            "Codex".into(),
            Some(usage_account),
        ) {
            Ok(output) => outputs.push(output),
            Err(e) if first_error.is_none() => first_error = Some(e),
            Err(_) => {}
        }
    }

    if outputs.is_empty() {
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(outputs)
        }
    } else {
        Ok(outputs)
    }
}

fn fetch_saved_account(name_or_id: Option<&str>) -> Result<(CodexAccountInfo, UsageOutput)> {
    let (account_id, account, info) = load_account(name_or_id)?;
    let usage_account = UsageAccount {
        id: info.id.clone(),
        label: info.label.clone(),
        is_active: info.is_active,
    };
    let usage = fetch_with_auth(
        auth_from_account(&account),
        CredentialSource::Store(account_id),
        "Codex".into(),
        Some(usage_account),
    )?;
    Ok((info, usage))
}

pub fn import_current_account(label: Option<&str>) -> Result<CodexAccountInfo> {
    let (auth, _) = read_current_credentials()?;
    save_account_from_auth(auth, label)
}

pub fn save_current_account_as_active(label: Option<&str>) -> Result<CodexAccountInfo> {
    let (auth, _) = read_current_credentials()?;
    save_account_from_auth(auth, label)
}

pub fn switch_active_account(name_or_id: &str) -> Result<CodexAccountInfo> {
    let mut store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Codex accounts"))?;
    let resolved = resolve_account_id(&store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {name_or_id}"))?;
    let account = store
        .accounts
        .get(&resolved)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {resolved}"))?;

    let path = auth_write_path()?;
    save_auth_tokens(&path, &account.tokens)?;

    store.active_account_id = resolved.clone();
    save_credentials_store(&store)?;

    Ok(account_info(&store, &resolved, &account))
}

pub fn remove_account(name_or_id: &str) -> Result<CodexAccountInfo> {
    let mut store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Codex accounts"))?;
    let removal = remove_account_from_store(&mut store, name_or_id)?;

    if let Some(tokens) = removal.next_active_tokens {
        let path = auth_write_path()?;
        save_auth_tokens(&path, &tokens)?;
    } else if removal.info.is_active {
        let path = auth_write_path()?;
        remove_auth_file_if_account_matches(&path, &removal.info.id, &removal.tokens)?;
    }

    save_credentials_store(&store)?;
    Ok(removal.info)
}

pub fn run_codex_import(name: Option<String>) -> Result<()> {
    use colored::Colorize;

    let info = import_current_account(name.as_deref())?;
    let display = info.label.as_deref().unwrap_or(&info.id);

    println!("\n  {}\n", "Codex - Import".cyan());
    println!(
        "  {}",
        format!("Imported Codex account {}", display.bold()).green()
    );
    println!("{}", format!("  Account ID: {}", info.id).bright_black());
    println!();

    Ok(())
}

pub fn run_codex_accounts(json: bool) -> Result<()> {
    use colored::Colorize;

    let accounts = list_accounts();
    if json {
        #[derive(Serialize)]
        struct Output {
            accounts: Vec<CodexAccountInfo>,
        }
        println!("{}", serde_json::to_string_pretty(&Output { accounts })?);
        return Ok(());
    }

    if accounts.is_empty() {
        println!("\n  {}\n", "No saved Codex accounts.".yellow());
        return Ok(());
    }

    println!("{}", "\n  Codex - Accounts\n".cyan());
    for account in &accounts {
        let name = if let Some(label) = &account.label {
            format!("{} ({})", label, account.id)
        } else {
            account.id.clone()
        };
        let marker = if account.is_active { "*" } else { "-" };
        let marker_colored = if account.is_active {
            marker.green().to_string()
        } else {
            marker.bright_black().to_string()
        };
        println!("  {} {}", marker_colored, name);
        if let Some(account_id) = &account.account_id {
            println!(
                "{}",
                format!("    Account ID: {}", account_id).bright_black()
            );
        }
    }
    println!();

    Ok(())
}

pub fn run_codex_switch(name: &str) -> Result<()> {
    use colored::Colorize;

    let info = switch_active_account(name)?;
    let display = info.label.as_deref().unwrap_or(&info.id);

    println!(
        "\n  {}\n",
        format!("Active Codex account set to {}", display.bold()).green()
    );

    Ok(())
}

pub fn run_codex_remove(name: &str) -> Result<()> {
    use colored::Colorize;

    let info = remove_account(name)?;
    let display = info.label.as_deref().unwrap_or(&info.id);

    println!(
        "\n  {}\n",
        format!("Removed Codex account {}", display.bold()).green()
    );

    Ok(())
}

pub fn run_codex_status(name: Option<String>, json: bool) -> Result<()> {
    use colored::Colorize;

    let result = if name.is_some() || load_credentials_store().is_some() {
        fetch_saved_account(name.as_deref()).map(|(account, usage)| (Some(account), usage))
    } else {
        fetch().map(|usage| (None, usage))
    };

    if json {
        #[derive(Serialize)]
        struct Output {
            #[serde(skip_serializing_if = "Option::is_none")]
            account: Option<CodexAccountInfo>,
            #[serde(skip_serializing_if = "Option::is_none")]
            usage: Option<UsageOutput>,
            #[serde(skip_serializing_if = "Option::is_none")]
            error: Option<String>,
        }
        let output = match result {
            Ok((account, usage)) => Output {
                account,
                usage: Some(usage),
                error: None,
            },
            Err(e) => Output {
                account: None,
                usage: None,
                error: Some(e.to_string()),
            },
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("\n  {}\n", "Codex - Status".cyan());
    match result {
        Ok((account, usage)) => {
            if let Some(account) = account {
                let display = account.label.as_deref().unwrap_or(&account.id);
                println!("{}", format!("  Account: {}", display).white());
                if let Some(account_id) = account.account_id {
                    println!("{}", format!("  Account ID: {}", account_id).bright_black());
                }
            }
            if let Some(email) = usage.email {
                println!("{}", format!("  Email: {}", email).white());
            }
            if let Some(plan) = usage.plan {
                println!("{}", format!("  Plan: {}", plan).white());
            }
            if usage.metrics.is_empty() {
                println!("{}", "  No quota metrics returned.".yellow());
            } else {
                for metric in usage.metrics {
                    let remaining = metric
                        .remaining_label
                        .unwrap_or_else(|| format!("{:.0}% left", metric.remaining_percent));
                    println!(
                        "  {} {}",
                        format!("{:<10}", metric.label).bright_black(),
                        remaining
                    );
                }
            }
        }
        Err(e) => {
            println!("  {}", format!("Status failed: {e}").red());
        }
    }
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tokens(access: &str, account_id: Option<&str>) -> Tokens {
        Tokens {
            access_token: Some(access.to_string()),
            refresh_token: Some("refresh".to_string()),
            account_id: account_id.map(str::to_string),
            id_token: None,
        }
    }

    fn tokens_with_id_token(access: &str, account_id: Option<&str>, id_token: &str) -> Tokens {
        Tokens {
            access_token: Some(access.to_string()),
            refresh_token: Some("refresh".to_string()),
            account_id: account_id.map(str::to_string),
            id_token: Some(id_token.to_string()),
        }
    }

    #[test]
    fn derive_account_id_prefers_account_id() {
        let tokens = tokens("access-token", Some("acct_work"));
        assert_eq!(derive_account_id(&tokens), "acct_work");
    }

    #[test]
    fn derive_account_id_falls_back_to_stable_token_hash() {
        let id = derive_account_id(&tokens("access-token", None));
        assert!(id.starts_with("token-"));
        assert_eq!(id, derive_account_id(&tokens("access-token", None)));
    }

    #[test]
    fn same_token_identity_prefers_account_id_over_rotating_id_token() {
        let a = tokens_with_id_token("access-a", Some("acct_shared"), "id-token-a");
        let b = tokens_with_id_token("access-b", Some("acct_shared"), "id-token-b");

        assert!(same_token_identity(&a, &b));
    }

    #[test]
    fn load_credentials_store_repairs_missing_active_account() -> Result<()> {
        let tmp = TempDir::new()?;
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_a".to_string(),
            CodexAccount {
                tokens: tokens("access-a", Some("acct_a")),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("zulu".to_string()),
            },
        );
        accounts.insert(
            "acct_b".to_string(),
            CodexAccount {
                tokens: tokens("access-b", Some("acct_b")),
                created_at: "2026-01-02T00:00:00Z".to_string(),
                label: Some("alpha".to_string()),
            },
        );
        let store = CodexCredentialsStore {
            version: 1,
            active_account_id: "missing".to_string(),
            accounts,
        };
        save_credentials_store_in_home(tmp.path(), &store)?;

        let loaded = load_credentials_store_from_home(tmp.path()).unwrap();
        assert_eq!(loaded.active_account_id, "acct_b");
        Ok(())
    }

    #[test]
    fn resolve_account_id_matches_label_case_insensitively() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_a".to_string(),
            CodexAccount {
                tokens: tokens("access-a", Some("acct_a")),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("Work".to_string()),
            },
        );
        let store = CodexCredentialsStore {
            version: 1,
            active_account_id: "acct_a".to_string(),
            accounts,
        };

        assert_eq!(
            resolve_account_id(&store, "work").as_deref(),
            Some("acct_a")
        );
    }

    #[test]
    fn save_account_from_auth_in_home_imports_tokens_without_touching_real_home() -> Result<()> {
        let tmp = TempDir::new()?;
        let info = save_account_from_auth_in_home(
            tmp.path(),
            Auth {
                tokens: Some(tokens("access-a", Some("acct_a"))),
            },
            Some("work"),
        )?;

        assert_eq!(info.id, "acct_a");
        assert_eq!(info.label.as_deref(), Some("work"));
        assert!(info.is_active);

        let loaded = load_credentials_store_from_home(tmp.path()).unwrap();
        assert_eq!(loaded.active_account_id, "acct_a");
        assert!(loaded.accounts.contains_key("acct_a"));
        Ok(())
    }

    #[test]
    fn save_account_from_auth_in_home_preserves_label_when_updating_same_account() -> Result<()> {
        let tmp = TempDir::new()?;
        save_account_from_auth_in_home(
            tmp.path(),
            Auth {
                tokens: Some(tokens("access-a", Some("acct_a"))),
            },
            Some("work"),
        )?;

        let info = save_account_from_auth_in_home(
            tmp.path(),
            Auth {
                tokens: Some(tokens("access-b", Some("acct_a"))),
            },
            None,
        )?;

        assert_eq!(info.id, "acct_a");
        assert_eq!(info.label.as_deref(), Some("work"));

        let loaded = load_credentials_store_from_home(tmp.path()).unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        let account = loaded.accounts.get("acct_a").unwrap();
        assert_eq!(account.label.as_deref(), Some("work"));
        assert_eq!(account.tokens.access_token.as_deref(), Some("access-b"));
        Ok(())
    }

    #[test]
    fn save_account_from_auth_in_home_keeps_existing_account_on_identity_collision() -> Result<()> {
        let tmp = TempDir::new()?;
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_shared".to_string(),
            CodexAccount {
                tokens: tokens_with_id_token("access-a", Some("acct_other"), "id-token-a"),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("work".to_string()),
            },
        );
        save_credentials_store_in_home(
            tmp.path(),
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "acct_shared".to_string(),
                accounts,
            },
        )?;

        let info = save_account_from_auth_in_home(
            tmp.path(),
            Auth {
                tokens: Some(tokens_with_id_token(
                    "access-b",
                    Some("acct_shared"),
                    "id-token-b",
                )),
            },
            None,
        )?;

        assert_eq!(info.id, "acct_shared-2");

        let loaded = load_credentials_store_from_home(tmp.path()).unwrap();
        assert_eq!(loaded.accounts.len(), 2);
        assert_eq!(loaded.active_account_id, "acct_shared-2");
        assert_eq!(
            loaded
                .accounts
                .get("acct_shared")
                .and_then(|account| account.label.as_deref()),
            Some("work")
        );
        assert!(loaded.accounts.contains_key("acct_shared-2"));
        Ok(())
    }

    #[test]
    fn save_account_from_auth_in_home_can_add_without_changing_active_account() -> Result<()> {
        let tmp = TempDir::new()?;
        save_account_from_auth_in_home(
            tmp.path(),
            Auth {
                tokens: Some(tokens("access-a", Some("acct_a"))),
            },
            Some("work"),
        )?;

        let info = save_account_from_auth_in_home_with_active(
            tmp.path(),
            Auth {
                tokens: Some(tokens("access-b", Some("acct_b"))),
            },
            Some("personal"),
            false,
        )?;

        assert_eq!(info.id, "acct_b");
        assert!(!info.is_active);

        let loaded = load_credentials_store_from_home(tmp.path()).unwrap();
        assert_eq!(loaded.active_account_id, "acct_a");
        assert!(loaded.accounts.contains_key("acct_a"));
        assert!(loaded.accounts.contains_key("acct_b"));
        Ok(())
    }

    #[test]
    fn remove_auth_file_if_account_matches_deletes_matching_auth() -> Result<()> {
        let tmp = TempDir::new()?;
        let path = tmp.path().join("auth.json");
        save_auth_tokens(&path, &tokens("access-a", Some("acct_a")))?;

        remove_auth_file_if_account_matches(&path, "acct_a", &tokens("access-a", Some("acct_a")))?;

        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn remove_auth_file_if_account_matches_keeps_other_auth() -> Result<()> {
        let tmp = TempDir::new()?;
        let path = tmp.path().join("auth.json");
        save_auth_tokens(&path, &tokens("access-b", Some("acct_b")))?;

        remove_auth_file_if_account_matches(&path, "acct_a", &tokens("access-a", Some("acct_a")))?;

        assert!(path.exists());
        Ok(())
    }

    #[test]
    fn remove_auth_file_if_account_matches_deletes_collision_suffixed_account() -> Result<()> {
        let tmp = TempDir::new()?;
        let path = tmp.path().join("auth.json");
        let account_tokens = tokens("access-a", Some("acct_a"));
        save_auth_tokens(&path, &account_tokens)?;

        remove_auth_file_if_account_matches(&path, "acct_a-2", &account_tokens)?;

        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn remove_account_from_store_keeps_active_when_removing_inactive() -> Result<()> {
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_a".to_string(),
            CodexAccount {
                tokens: tokens("access-a", Some("acct_a")),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("Work".to_string()),
            },
        );
        accounts.insert(
            "acct_b".to_string(),
            CodexAccount {
                tokens: tokens("access-b", Some("acct_b")),
                created_at: "2026-01-02T00:00:00Z".to_string(),
                label: Some("Personal".to_string()),
            },
        );
        let mut store = CodexCredentialsStore {
            version: 1,
            active_account_id: "acct_a".to_string(),
            accounts,
        };

        let removal = remove_account_from_store(&mut store, "personal")?;

        assert_eq!(removal.info.id, "acct_b");
        assert_eq!(removal.tokens.access_token.as_deref(), Some("access-b"));
        assert!(!removal.info.is_active);
        assert!(removal.next_active_tokens.is_none());
        assert_eq!(store.active_account_id, "acct_a");
        assert!(!store.accounts.contains_key("acct_b"));
        Ok(())
    }

    #[test]
    fn remove_account_from_store_selects_next_active_when_removing_active() -> Result<()> {
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_a".to_string(),
            CodexAccount {
                tokens: tokens("access-a", Some("acct_a")),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("Work".to_string()),
            },
        );
        accounts.insert(
            "acct_b".to_string(),
            CodexAccount {
                tokens: tokens("access-b", Some("acct_b")),
                created_at: "2026-01-02T00:00:00Z".to_string(),
                label: Some("Personal".to_string()),
            },
        );
        let mut store = CodexCredentialsStore {
            version: 1,
            active_account_id: "acct_a".to_string(),
            accounts,
        };

        let removal = remove_account_from_store(&mut store, "work")?;

        assert_eq!(removal.info.id, "acct_a");
        assert_eq!(removal.tokens.access_token.as_deref(), Some("access-a"));
        assert!(removal.info.is_active);
        assert_eq!(store.active_account_id, "acct_b");
        assert_eq!(
            removal
                .next_active_tokens
                .and_then(|tokens| tokens.access_token)
                .as_deref(),
            Some("access-b")
        );
        Ok(())
    }
}
