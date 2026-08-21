use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use fs2::FileExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use super::helpers::capitalize;
use super::{
    UsageAccount, UsageCreditStatus, UsageFetchDiagnostic, UsageFetchDiagnosticKind,
    UsageFetchDiagnosticSeverity, UsageFetchReport, UsageMetric, UsageOutput, UsageResetCredit,
    UsageResetCredits, UsageSpendControl,
};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Auth {
    tokens: Option<Tokens>,
}

#[derive(Debug, Deserialize)]
struct OpenCodeAuthDocument {
    openai: Option<OpenCodeCredential>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum OpenCodeCredential {
    #[serde(rename = "oauth")]
    Oauth {
        access: String,
        #[serde(rename = "accountId")]
        account_id: Option<String>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default, deserialize_with = "deserialize_null_default_vec")]
    additional_rate_limits: Vec<AdditionalRateLimit>,
    rate_limit_reset_credits: Option<ResetCreditsSummary>,
    credits: Option<Credits>,
    spend_control: Option<SpendControl>,
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
    used_percent: Option<f64>,
    limit_window_seconds: Option<i64>,
    #[serde(alias = "resets_at")]
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AdditionalRateLimit {
    metered_feature: Option<String>,
    limit_name: Option<String>,
    rate_limit: Option<RateLimit>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ResetCreditsSummary {
    available_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ResetCreditsResponse {
    available_count: Option<u32>,
    #[serde(default)]
    credits: Vec<ResetCredit>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ResetCredit {
    id: Option<String>,
    status: Option<String>,
    reset_type: Option<String>,
    expires_at: Option<String>,
    title: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Credits {
    balance: Option<serde_json::Value>,
    has_credits: Option<bool>,
    unlimited: Option<bool>,
    overage_limit_reached: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SpendControl {
    individual_limit: Option<serde_json::Value>,
    reached: Option<bool>,
}

fn deserialize_null_default_vec<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RateLimitResetConsumeResult {
    #[serde(default)]
    pub code: String,
    pub windows_reset: Option<u32>,
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
    OpenCodeFile(PathBuf),
    Keychain,
    Store(String),
}

impl CredentialSource {
    /// The account whose tokens tokscale may refresh and write back, if any.
    ///
    /// Only [`Self::Store`] qualifies: that is tokscale's own
    /// `<config>/codex-credentials.json`. [`Self::File`] is the codex CLI's
    /// `auth.json`, [`Self::Keychain`] is its Keychain item, and
    /// [`Self::OpenCodeFile`] belongs to OpenCode. The OpenCode adapter does not
    /// even deserialize its refresh token; writing either Codex-owned source
    /// destroys keys tokscale does not model (see [`auth_document`]), which is
    /// #1001 made against the codex CLI.
    ///
    /// This is ownership of the *file*, which is not the same as ownership of
    /// the OAuth grant, and the difference is not academic: a store entry can be
    /// a verbatim copy of the codex CLI's live tokens, and the TUI surface
    /// copies them in on its own via [`fetch_all_report_importing_current_auth`]
    /// rather than waiting for an explicit import. So refreshing a store entry
    /// can still spend a refresh token the codex CLI is holding. That hazard
    /// predates this guard and is not what this guard fixes; closing it needs
    /// grant provenance, not a storage-location check.
    fn refreshable_account_id(&self) -> Option<&str> {
        match self {
            Self::Store(account_id) => Some(account_id),
            Self::File(_) | Self::OpenCodeFile(_) | Self::Keychain => None,
        }
    }

    /// Names the credential in user-facing errors so recovery points at the
    /// application that owns the rejected login.
    fn describe(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::OpenCodeFile(path) => format!("OpenCode auth at {}", path.display()),
            Self::Keychain => "the Codex Keychain item".to_string(),
            Self::Store(account_id) => format!("tokscale account '{account_id}'"),
        }
    }

    fn reauthentication_guidance(&self) -> &'static str {
        match self {
            Self::OpenCodeFile(_) => {
                "Run OpenCode and reconnect OpenAI with '/connect', then retry."
            }
            Self::File(_) | Self::Keychain | Self::Store(_) => {
                "Run 'codex' so the Codex CLI can refresh its own login, then retry."
            }
        }
    }
}

/// The chatgpt.com / auth.openai.com endpoints the usage path talks to.
///
/// Injected rather than hardcoded so tests can point them at a local server.
/// The token endpoint is the load-bearing one: the regression this guards
/// against is a *reintroduced request*, and a hardcoded absolute URL would send
/// that request to the real auth server where no test could observe it.
#[derive(Debug, Clone)]
struct CodexEndpoints {
    usage: String,
    reset_credits: String,
    token: String,
}

impl CodexEndpoints {
    fn production() -> Self {
        Self {
            usage: "https://chatgpt.com/backend-api/wham/usage".to_string(),
            reset_credits: "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits"
                .to_string(),
            token: "https://auth.openai.com/oauth/token".to_string(),
        }
    }
}

#[derive(Debug)]
enum CodexUsageError {
    MissingCredentials,
    NeedsAuth,
    UnsupportedStoreVersion { version: i64, path: PathBuf },
}

impl fmt::Display for CodexUsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCredentials => {
                write!(f, "No Codex credentials found. Run 'codex' to log in.")
            }
            Self::NeedsAuth => write!(f, "Codex credentials need authentication"),
            Self::UnsupportedStoreVersion { version, path } => write!(
                f,
                "Unsupported Codex account store version {version} at {} (this tokscale supports version 1); refusing to modify it",
                path.display()
            ),
        }
    }
}

impl std::error::Error for CodexUsageError {}

fn has_codex_error(error: &anyhow::Error, predicate: impl Fn(&CodexUsageError) -> bool) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<CodexUsageError>()
            .is_some_and(&predicate)
    })
}

fn is_missing_credentials(error: &anyhow::Error) -> bool {
    has_codex_error(error, |error| {
        matches!(error, CodexUsageError::MissingCredentials)
    })
}

fn is_needs_auth(error: &anyhow::Error) -> bool {
    has_codex_error(error, |error| matches!(error, CodexUsageError::NeedsAuth))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexFetchIntent {
    ReadOnly,
    SaveCurrentLogin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreRepairPolicy {
    InMemoryOnly,
    Persist,
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

#[derive(Debug, Clone)]
struct CodexAccountStore {
    path: PathBuf,
}

impl CodexAccountStore {
    fn default() -> Self {
        Self {
            path: codex_store_path(),
        }
    }

    fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[cfg(test)]
    fn in_home(home_dir: &Path) -> Self {
        Self::at_path(codex_store_path_in_home(home_dir))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    fn with_lock<T>(&self, action: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let lock_path = self.lock_path();
        if let Some(dir) = lock_path.parent() {
            std::fs::create_dir_all(dir).with_context(|| {
                format!("Failed to create Codex account lock dir {}", dir.display())
            })?;
        }
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| {
                format!("Failed to open Codex account lock {}", lock_path.display())
            })?;
        lock_file.lock_exclusive().with_context(|| {
            format!("Failed to lock Codex account store {}", self.path.display())
        })?;

        let result = action(self);
        let unlock_result = lock_file.unlock().with_context(|| {
            format!(
                "Failed to unlock Codex account store {}",
                self.path.display()
            )
        });

        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn read(&self) -> Option<CodexCredentialsStore> {
        self.read_result().ok().flatten()
    }

    fn read_result(&self) -> Result<Option<CodexCredentialsStore>> {
        self.read_unlocked(StoreRepairPolicy::InMemoryOnly)
    }

    fn read_for_update(&self) -> Result<Option<CodexCredentialsStore>> {
        self.with_lock(|store| store.read_for_update_unlocked())
    }

    fn read_for_update_unlocked(&self) -> Result<Option<CodexCredentialsStore>> {
        self.read_unlocked(StoreRepairPolicy::Persist)
    }

    fn read_unlocked(
        &self,
        repair_policy: StoreRepairPolicy,
    ) -> Result<Option<CodexCredentialsStore>> {
        load_credentials_store_from_path_unlocked(self.path(), repair_policy)
    }

    #[cfg(test)]
    fn save(&self, store: &CodexCredentialsStore) -> Result<()> {
        self.with_lock(|store_file| store_file.save_unlocked(store))
    }

    fn save_unlocked(&self, store: &CodexCredentialsStore) -> Result<()> {
        let json = serde_json::to_string_pretty(store)?;
        super::helpers::atomic_write_secret(self.path(), json.as_bytes()).with_context(|| {
            format!(
                "Failed to write Codex account store to {}",
                self.path.display()
            )
        })
    }

    fn update_existing<T>(
        &self,
        missing_message: &'static str,
        action: impl FnOnce(&mut CodexCredentialsStore) -> Result<T>,
    ) -> Result<T> {
        self.with_lock(|store_file| {
            let mut store = store_file
                .read_for_update_unlocked()?
                .ok_or_else(|| anyhow::anyhow!(missing_message))?;
            let result = action(&mut store)?;
            store_file.save_unlocked(&store)?;
            Ok(result)
        })
    }
}

fn current_auth_paths() -> Vec<PathBuf> {
    let home = crate::paths::home_dir().unwrap_or_else(|| PathBuf::from("."));
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

fn opencode_auth_path() -> PathBuf {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::paths::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("share")
        });
    data_dir.join("opencode").join("auth.json")
}

fn read_opencode_credentials_at(path: &Path) -> Result<Option<(Auth, CredentialSource)>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read OpenCode auth from {}", path.display()))?;
    let document = serde_json::from_str::<OpenCodeAuthDocument>(&content)
        .with_context(|| format!("Failed to parse OpenCode auth from {}", path.display()))?;
    let Some(OpenCodeCredential::Oauth {
        access, account_id, ..
    }) = document.openai
    else {
        return Ok(None);
    };

    if access.trim().is_empty() {
        anyhow::bail!("OpenCode OAuth entry has no access token");
    }

    Ok(Some((
        Auth {
            tokens: Some(Tokens {
                access_token: Some(access),
                refresh_token: None,
                account_id,
                id_token: None,
            }),
        },
        CredentialSource::OpenCodeFile(path.to_path_buf()),
    )))
}

/// Where `switch` writes the codex CLI auth. Derived from
/// [`current_auth_paths`]: an explicit `CODEX_HOME` always wins (even if no
/// auth.json exists there yet); otherwise the first existing path, falling
/// back to the modern config location.
fn auth_write_path() -> Result<PathBuf> {
    let paths = current_auth_paths();
    let has_codex_home = std::env::var("CODEX_HOME")
        .map(|home| !home.trim().is_empty())
        .unwrap_or(false);

    if !has_codex_home {
        if let Some(existing) = paths.iter().find(|path| path.exists()) {
            return Ok(existing.clone());
        }
    }

    paths
        .into_iter()
        .next()
        .context("Could not determine Codex auth path")
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

    Err(CodexUsageError::MissingCredentials.into())
}

/// Builds a codex `auth.json` body from `tokens` alone, discarding everything
/// else the document held.
///
/// The codex CLI's own `AuthDotJson` carries seven top-level keys --
/// `auth_mode`, `OPENAI_API_KEY`, `tokens`, `last_refresh`, `agent_identity`,
/// `personal_access_token`, `bedrock_api_key` -- and this writes two of them.
/// The other five are deleted, and three of those (`OPENAI_API_KEY`,
/// `agent_identity`, `bedrock_api_key`) are standalone credentials, so the write
/// can take a user's API key with it. That is why the only caller left is
/// [`switch_active_account`], where repointing codex at another account is the
/// user's explicit request. The usage path must never reach here: it is a
/// reader, and #1001 is what happens when a reader writes.
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

/// Writes refreshed tokens back to tokscale's own account store, and nowhere
/// else. Sources tokscale does not own are silently skipped rather than
/// diverted to a different file -- they have no destination by design.
fn persist_refreshed_tokens(source: &CredentialSource, tokens: &Tokens) {
    let Some(account_id) = source.refreshable_account_id() else {
        return;
    };
    if let Err(e) = update_account_tokens(account_id, tokens.clone()) {
        eprintln!("warning: failed to save Codex account credentials: {e}");
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexAccountIdentity {
    stable_id: String,
    account_id: Option<String>,
    id_token: Option<String>,
    access_token: Option<String>,
}

impl CodexAccountIdentity {
    fn from_tokens(tokens: &Tokens) -> Self {
        let account_id = normalized_token_value(tokens.account_id.as_deref());
        let id_token = normalized_token_value(tokens.id_token.as_deref());
        let access_token = normalized_token_value(tokens.access_token.as_deref());
        let stable_id = account_id
            .clone()
            .or_else(|| {
                id_token
                    .as_deref()
                    .map(|token| format!("id-{}", hash_token(token)))
            })
            .or_else(|| {
                access_token
                    .as_deref()
                    .map(|token| format!("token-{}", hash_token(token)))
            })
            .unwrap_or_else(|| "account".to_string());

        Self {
            stable_id,
            account_id,
            id_token,
            access_token,
        }
    }

    fn stable_id(&self) -> &str {
        &self.stable_id
    }

    fn matches(&self, other: &Self) -> bool {
        if let (Some(a_id), Some(b_id)) = (self.account_id.as_deref(), other.account_id.as_deref())
        {
            return a_id == b_id;
        }

        if let (Some(a_id), Some(b_id)) = (self.id_token.as_deref(), other.id_token.as_deref()) {
            return a_id == b_id;
        }

        match (self.access_token.as_deref(), other.access_token.as_deref()) {
            (Some(a_token), Some(b_token)) => a_token == b_token,
            _ => false,
        }
    }
}

fn derive_account_id(tokens: &Tokens) -> String {
    CodexAccountIdentity::from_tokens(tokens)
        .stable_id()
        .to_string()
}

fn normalized_token_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn same_token_identity(a: &Tokens, b: &Tokens) -> bool {
    CodexAccountIdentity::from_tokens(a).matches(&CodexAccountIdentity::from_tokens(b))
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
    CodexAccountStore::default().read()
}

#[cfg(test)]
fn load_credentials_store_from_home(home_dir: &Path) -> Option<CodexCredentialsStore> {
    CodexAccountStore::in_home(home_dir).read()
}

#[cfg(test)]
fn load_credentials_store_from_path(path: &Path) -> Option<CodexCredentialsStore> {
    CodexAccountStore::at_path(path).read()
}

/// Loads the store while distinguishing "no usable store" (`Ok(None)`) from a
/// store written by a newer tokscale (`Err`). Write paths must propagate the
/// error instead of silently clobbering a future-version store; read paths can
/// treat both as "nothing usable".
fn load_credentials_store_from_path_unlocked(
    path: &Path,
    repair_policy: StoreRepairPolicy,
) -> Result<Option<CodexCredentialsStore>> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    bail_on_unknown_store_version(path, &content)?;
    let Ok(mut store) = serde_json::from_str::<CodexCredentialsStore>(&content) else {
        return Ok(None);
    };

    if store.accounts.is_empty() {
        return Ok(None);
    }

    if !store.active_account_id.trim().is_empty()
        && !store.accounts.contains_key(&store.active_account_id)
    {
        if let Some(first_id) = first_account_id(&store) {
            store.active_account_id = first_id;
            if repair_policy == StoreRepairPolicy::Persist {
                let _ = CodexAccountStore::at_path(path).save_unlocked(&store);
            }
        }
    }

    Ok(Some(store))
}

/// A future-version store may not even deserialize into the current struct, so
/// the version is checked on the raw JSON before the typed parse.
fn bail_on_unknown_store_version(path: &Path, content: &str) -> Result<()> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Ok(());
    };
    let Some(version) = value.get("version").and_then(serde_json::Value::as_i64) else {
        return Ok(());
    };
    if version != 1 {
        return Err(CodexUsageError::UnsupportedStoreVersion {
            version,
            path: path.to_path_buf(),
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
fn save_credentials_store_in_home(home_dir: &Path, store: &CodexCredentialsStore) -> Result<()> {
    CodexAccountStore::in_home(home_dir).save(store)
}

#[cfg(test)]
fn save_credentials_store_at_path(path: &Path, store: &CodexCredentialsStore) -> Result<()> {
    CodexAccountStore::at_path(path).save(store)
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

/// Case-insensitive sort key shared by every place that orders accounts:
/// the label when present, falling back to the account id.
fn account_sort_key(label: Option<&str>, id: &str) -> String {
    label.unwrap_or(id).to_lowercase()
}

fn first_account_id(store: &CodexCredentialsStore) -> Option<String> {
    store
        .accounts
        .iter()
        .min_by_key(|(id, account)| {
            (
                account_sort_key(account.label.as_deref(), id),
                (*id).clone(),
            )
        })
        .map(|(id, _)| id.clone())
}

fn remove_account_from_store(
    store: &mut CodexCredentialsStore,
    name_or_id: &str,
) -> Result<CodexAccountInfo> {
    let resolved = resolve_account_id(store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {name_or_id}"))?;
    let removed_was_active = store.active_account_id == resolved;
    let account = store
        .accounts
        .remove(&resolved)
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {resolved}"))?;
    let removed = CodexAccountInfo {
        id: resolved,
        label: account.label,
        account_id: account.tokens.account_id.clone(),
        created_at: account.created_at,
        is_active: removed_was_active,
    };

    if removed_was_active {
        store.active_account_id.clear();
    }

    Ok(removed)
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

    accounts.sort_by_key(|account| {
        (
            !account.is_active,
            account_sort_key(account.label.as_deref(), &account.id),
        )
    });

    accounts
}

fn save_account_from_auth(auth: Auth, label: Option<&str>) -> Result<CodexAccountInfo> {
    save_account_from_auth_at_path(&codex_store_path(), auth, label, true)
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
    CodexAccountStore::at_path(store_path).with_lock(|store_file| {
        let mut store =
            store_file
                .read_for_update_unlocked()?
                .unwrap_or_else(|| CodexCredentialsStore {
                    version: 1,
                    active_account_id: if make_active {
                        base_account_id.clone()
                    } else {
                        String::new()
                    },
                    accounts: HashMap::new(),
                });

        // Scan every stored account (not just the base-id key) so an account
        // stored under a collision-suffixed id (e.g. `acct_x-2`) is updated in
        // place instead of re-importing as `acct_x-3`, `acct_x-4`, ...
        let existing_identity_id = store
            .accounts
            .iter()
            .find(|(_, existing)| same_token_identity(&existing.tokens, &tokens))
            .map(|(id, _)| id.clone());

        if let Some(existing_id) = existing_identity_id {
            validate_label_available(&store, &existing_id, label)?;
            let label = label.map(str::trim).filter(|s| !s.is_empty());
            let active_changed = make_active && store.active_account_id != existing_id;
            let mut account_changed = false;
            if let Some(existing) = store.accounts.get_mut(&existing_id) {
                if existing.tokens != tokens {
                    existing.tokens = tokens;
                    account_changed = true;
                }
                if let Some(label) = label {
                    if existing.label.as_deref() != Some(label) {
                        existing.label = Some(label.to_string());
                        account_changed = true;
                    }
                }
            }
            if make_active {
                store.active_account_id = existing_id.clone();
            }
            if account_changed || active_changed {
                store_file.save_unlocked(&store)?;
            }

            let account = store
                .accounts
                .get(&existing_id)
                .ok_or_else(|| anyhow::anyhow!("Failed to save Codex account"))?;
            return Ok(account_info(&store, &existing_id, account));
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
        if make_active {
            store.active_account_id = account_id.clone();
        }
        store_file.save_unlocked(&store)?;

        let account = store
            .accounts
            .get(&account_id)
            .ok_or_else(|| anyhow::anyhow!("Failed to save Codex account"))?;
        Ok(account_info(&store, &account_id, account))
    })
}

pub struct CodexLoginImport {
    pub info: CodexAccountInfo,
    /// Non-fatal problem while snapshotting the current codex CLI login into
    /// the store; surfaced in the TUI login panel.
    pub warning: Option<String>,
}

/// Imports a freshly logged-in `auth.json` (from the TUI's temporary
/// `CODEX_HOME`) into the store without activating it.
///
/// Before importing, the codex CLI's current login is snapshotted into the
/// store as the active account so it stays tracked alongside the new one.
/// Snapshot failure is deliberately non-fatal — the new login is the primary
/// operation — but it is reported as a warning instead of being swallowed,
/// because without the snapshot the imported account may become the store's
/// active account while the codex CLI stays logged into another.
pub fn import_login_auth_file(path: &Path) -> Result<CodexLoginImport> {
    let store_path = codex_store_path();

    let warning = match read_current_credentials() {
        Ok((current_auth, _)) => {
            save_account_from_auth_at_path(&store_path, current_auth, None, true)
                .err()
                .map(|e| format!("warning: failed to save current Codex login: {e}"))
        }
        // No current codex CLI login — nothing to snapshot.
        Err(_) => None,
    };

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read Codex auth from {}", path.display()))?;
    let auth = serde_json::from_str::<Auth>(&content)
        .with_context(|| format!("Failed to parse Codex auth from {}", path.display()))?;
    let info = save_account_from_auth_at_path(&store_path, auth, None, false)?;

    Ok(CodexLoginImport { info, warning })
}

pub fn save_current_auth_account() -> Result<CodexAccountInfo> {
    let (auth, _) = read_current_credentials()?;
    save_account_from_auth(auth, None)
}

fn update_account_tokens(account_id: &str, tokens: Tokens) -> Result<()> {
    CodexAccountStore::default().update_existing("No saved Codex accounts", |store| {
        let account = store
            .accounts
            .get_mut(account_id)
            .ok_or_else(|| anyhow::anyhow!("Codex account not found: {account_id}"))?;
        account.tokens = tokens;
        Ok(())
    })
}

fn load_account(name_or_id: Option<&str>) -> Result<(String, CodexAccount, CodexAccountInfo)> {
    let store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Codex accounts"))?;
    let resolved = match name_or_id {
        Some(name) => resolve_account_id(&store, name)
            .ok_or_else(|| anyhow::anyhow!("Codex account not found: {name}"))?,
        None if store.active_account_id.trim().is_empty() => {
            anyhow::bail!("No active Codex account; pass an account name or switch to one first")
        }
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

fn has_native_credentials() -> bool {
    if load_credentials_store()
        .map(|store| !store.accounts.is_empty())
        .unwrap_or(false)
    {
        return true;
    }

    read_current_credentials().is_ok()
}

fn has_opencode_auth_candidate_at(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_file(),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn has_opencode_usage_candidate_at(path: &Path) -> bool {
    if !has_opencode_auth_candidate_at(path) {
        return false;
    }

    // Keep malformed or unreadable files active so the fetch path can report
    // the owning path instead of silently suppressing Codex usage.
    match read_opencode_credentials_at(path) {
        Ok(Some(_)) | Err(_) => true,
        Ok(None) => false,
    }
}

pub fn has_credentials() -> bool {
    has_native_credentials() || has_opencode_usage_candidate_at(&opencode_auth_path())
}

async fn refresh_token(client: &reqwest::Client, token_url: &str, rt: &str) -> Result<Refresh> {
    let resp = client
        .post(token_url)
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

fn parse_chatgpt_json_body<T>(body: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    if body.trim_start().starts_with('<') {
        return Err(CodexUsageError::NeedsAuth.into());
    }
    Ok(serde_json::from_str(body)?)
}

async fn parse_chatgpt_json_response<T>(resp: reqwest::Response, request_label: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(CodexUsageError::NeedsAuth.into());
    }
    if !status.is_success() {
        anyhow::bail!("{request_label} failed (HTTP {status})");
    }
    let body = resp.text().await?;
    parse_chatgpt_json_body(&body)
}

async fn fetch_usage(
    client: &reqwest::Client,
    usage_url: &str,
    token: &str,
    account_id: Option<&str>,
) -> Result<Usage> {
    let mut req = client
        .get(usage_url)
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
    parse_chatgpt_json_response(resp, "Codex usage request").await
}

async fn fetch_reset_credits(
    client: &reqwest::Client,
    reset_credits_url: &str,
    token: &str,
    account_id: Option<&str>,
) -> Result<ResetCreditsResponse> {
    let mut req = client
        .get(reset_credits_url)
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
    parse_chatgpt_json_response(resp, "Codex reset credits request").await
}

async fn consume_reset_credit(
    client: &reqwest::Client,
    token: &str,
    account_id: Option<&str>,
    redeem_request_id: &str,
) -> Result<RateLimitResetConsumeResult> {
    let mut req = client
        .post("https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        )
        .json(&serde_json::json!({
            "redeem_request_id": redeem_request_id,
        }));
    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let resp = req.send().await?;
    parse_chatgpt_json_response(resp, "Codex reset request").await
}

fn metric_from_window(label: &str, window: &Window) -> UsageMetric {
    let pct = window.used_percent.unwrap_or(0.0).clamp(0.0, 100.0);
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

fn rate_limit_window_label(window: &Window) -> Option<String> {
    let seconds = window.limit_window_seconds.filter(|seconds| *seconds > 0)?;
    if seconds == 7 * 24 * 60 * 60 {
        return Some("Weekly".to_string());
    }
    if seconds % (24 * 60 * 60) == 0 {
        return Some(format!("{}d", seconds / (24 * 60 * 60)));
    }
    if seconds % (60 * 60) == 0 {
        return Some(format!("{}h", seconds / (60 * 60)));
    }
    if seconds % 60 == 0 {
        return Some(format!("{}m", seconds / 60));
    }
    Some(format!("{seconds}s"))
}

fn metric_label(
    prefix: Option<&str>,
    window: &Window,
    fallback: &str,
    prefixed_fallback: &str,
) -> String {
    let dynamic_label = rate_limit_window_label(window);
    match prefix {
        Some(prefix) => format!(
            "{prefix} {}",
            dynamic_label
                .map(|label| label.to_ascii_lowercase())
                .unwrap_or_else(|| prefixed_fallback.to_string())
        ),
        None => dynamic_label.unwrap_or_else(|| fallback.to_string()),
    }
}

fn push_rate_limit_metrics(
    metrics: &mut Vec<UsageMetric>,
    prefix: Option<&str>,
    rate_limit: &RateLimit,
) {
    let label_prefix = prefix.map(str::trim).filter(|label| !label.is_empty());
    if let Some(ref w) = rate_limit.primary_window {
        let label = metric_label(label_prefix, w, "5h", "5h");
        metrics.push(metric_from_window(&label, w));
    }
    if let Some(ref w) = rate_limit.secondary_window {
        let label = metric_label(label_prefix, w, "Weekly", "week");
        metrics.push(metric_from_window(&label, w));
    }
}

fn reset_credits_from_summary(summary: Option<&ResetCreditsSummary>) -> Option<UsageResetCredits> {
    summary.and_then(|summary| {
        summary
            .available_count
            .map(|available_count| UsageResetCredits {
                available_count,
                credits: Vec::new(),
            })
    })
}

fn reset_credits_from_response(response: ResetCreditsResponse) -> Option<UsageResetCredits> {
    response
        .available_count
        .map(|available_count| UsageResetCredits {
            available_count,
            credits: response
                .credits
                .into_iter()
                .map(|credit| UsageResetCredit {
                    id: credit.id,
                    status: credit.status,
                    reset_type: credit.reset_type,
                    expires_at: credit.expires_at,
                    title: credit.title,
                    description: credit.description,
                })
                .collect(),
        })
}

/// Decide whether to issue the extra detail GET for reset credits.
///
/// We fetch the detail endpoint whenever the cheap inline summary leaves the
/// credit state unknown (absent) or already reports at least one available
/// credit to enrich. The detail call is the only source of truth for accounts
/// whose `/wham/usage` payload omits `rate_limit_reset_credits` entirely, so
/// skipping it on an absent summary would hide reset credits that production
/// can otherwise surface. We only skip when the summary is present and
/// explicitly reports zero credits: there is nothing to enrich, and firing the
/// request on every periodic TUI refresh would needlessly raise backend request
/// volume and rate-limit risk.
fn should_fetch_reset_details(summary: Option<&UsageResetCredits>) -> bool {
    summary.is_none_or(|credits| credits.available_count > 0)
}

/// Merge the cheap summary count with an optional detail response.
///
/// The detail response is only allowed to *replace* the summary when it carries
/// a concrete count (`Some`). A detail body whose `available_count` is null maps
/// to `None`; in that case we keep the known summary count rather than silently
/// dropping it (which would make the Reset button show nothing).
fn merge_reset_credits(
    summary: Option<UsageResetCredits>,
    details: Option<UsageResetCredits>,
) -> Option<UsageResetCredits> {
    details.or(summary)
}

fn json_scalar_string(value: Option<serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

async fn fetch_with_auth_async(
    endpoints: &CodexEndpoints,
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
    let mut effective_tokens = tokens.clone();
    let mut effective_access_token = access_token.clone();
    let resp = match fetch_usage(
        &client,
        &endpoints.usage,
        &access_token,
        tokens.account_id.as_deref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) if is_needs_auth(&e) => {
            // A rejected token is only tokscale's to renew when tokscale is the
            // one storing it. For the codex CLI's own auth.json or Keychain
            // item, refreshing would spend a refresh token the CLI is still
            // holding and rotate it out from under it -- the usage command is a
            // reader, so it stops here and says so.
            if source.refreshable_account_id().is_none() {
                return Err(e.context(format!(
                    "Codex usage unavailable: the access token in {} was rejected. \
                     {}",
                    source.describe(),
                    source.reauthentication_guidance()
                )));
            }
            let rt_str = tokens
                .refresh_token
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No refresh token."))?;
            let refreshed = refresh_token(&client, &endpoints.token, rt_str).await?;
            let new = refreshed
                .access_token
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Refresh returned no token."))?;

            let mut updated_tokens = tokens.clone();
            updated_tokens.access_token = Some(new.clone());
            if let Some(new_rt) = refreshed.refresh_token {
                updated_tokens.refresh_token = Some(new_rt);
            }
            persist_refreshed_tokens(&source, &updated_tokens);
            effective_access_token = new.clone();
            effective_tokens = updated_tokens.clone();

            fetch_usage(
                &client,
                &endpoints.usage,
                &new,
                updated_tokens.account_id.as_deref(),
            )
            .await?
        }
        Err(e) => return Err(e),
    };

    let plan = resp.plan_type.as_deref().map(capitalize);
    let mut metrics = Vec::new();
    if let Some(ref rl) = resp.rate_limit {
        push_rate_limit_metrics(&mut metrics, None, rl);
    }
    for limit in &resp.additional_rate_limits {
        if let Some(rate_limit) = &limit.rate_limit {
            let label = limit
                .limit_name
                .as_deref()
                .or(limit.metered_feature.as_deref())
                .map(capitalize);
            push_rate_limit_metrics(&mut metrics, label.as_deref(), rate_limit);
        }
    }

    let mut reset_credits = reset_credits_from_summary(resp.rate_limit_reset_credits.as_ref());
    if should_fetch_reset_details(reset_credits.as_ref()) {
        if let Ok(details) = fetch_reset_credits(
            &client,
            &endpoints.reset_credits,
            &effective_access_token,
            effective_tokens.account_id.as_deref(),
        )
        .await
        {
            // Only let the detail response replace the summary when it carries a
            // concrete count; a null detail count must not drop a known summary.
            reset_credits =
                merge_reset_credits(reset_credits, reset_credits_from_response(details));
        }
    }

    let credit_status = resp.credits.map(|credits| UsageCreditStatus {
        balance: json_scalar_string(credits.balance),
        has_credits: credits.has_credits,
        unlimited: credits.unlimited,
        overage_limit_reached: credits.overage_limit_reached,
    });
    let spend_control = resp.spend_control.map(|control| UsageSpendControl {
        individual_limit: json_scalar_string(control.individual_limit),
        reached: control.reached,
    });

    Ok(UsageOutput {
        provider: provider_name,
        account,
        credential_source: None,
        plan,
        email: resp.email,
        metrics,
        reset_credits,
        credit_status,
        spend_control,
    })
}

/// The whole authenticated usage path, with the endpoints as a parameter.
/// The refresh-and-write this module was fixed for lived in exactly this
/// orchestration, so it is the layer the tests must enter; [`fetch_with_auth`]
/// is one call with the production endpoints and holds no logic of its own.
fn fetch_with_auth_at(
    endpoints: &CodexEndpoints,
    auth: Auth,
    source: CredentialSource,
    provider_name: String,
    account: Option<UsageAccount>,
) -> Result<UsageOutput> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(fetch_with_auth_async(
        endpoints,
        auth,
        source,
        provider_name,
        account,
    ))
}

fn fetch_with_auth(
    auth: Auth,
    source: CredentialSource,
    provider_name: String,
    account: Option<UsageAccount>,
) -> Result<UsageOutput> {
    fetch_with_auth_at(
        &CodexEndpoints::production(),
        auth,
        source,
        provider_name,
        account,
    )
}

pub fn fetch() -> Result<UsageOutput> {
    let (auth, source) = read_current_credentials()?;
    fetch_with_auth(auth, source, "Codex".into(), None)
}

fn usage_account_from_saved(
    account_id: &str,
    account: &CodexAccount,
    active_account_id: Option<&str>,
) -> UsageAccount {
    UsageAccount {
        id: account_id.to_string(),
        label: account.label.clone(),
        is_active: active_account_id == Some(account_id),
    }
}

fn matching_account_id_for_tokens(
    store: &CodexCredentialsStore,
    tokens: &Tokens,
) -> Option<String> {
    let identity = CodexAccountIdentity::from_tokens(tokens);
    let derived = identity.stable_id().to_string();
    if store.accounts.get(&derived).is_some_and(|account| {
        identity.matches(&CodexAccountIdentity::from_tokens(&account.tokens))
    }) {
        return Some(derived);
    }

    store
        .accounts
        .iter()
        .find(|(_, account)| identity.matches(&CodexAccountIdentity::from_tokens(&account.tokens)))
        .map(|(account_id, _)| account_id.clone())
        .or_else(|| store.accounts.contains_key(&derived).then_some(derived))
}

fn current_auth_account_id_in_store(store: &CodexCredentialsStore) -> Option<String> {
    let (auth, _) = read_current_credentials().ok()?;
    let tokens = auth.tokens.as_ref()?;
    matching_account_id_for_tokens(store, tokens)
}

fn active_account_id_for_usage(store: &mut CodexCredentialsStore) -> Option<String> {
    let active_account_id = current_auth_account_id_in_store(store).or_else(|| {
        (!store.active_account_id.trim().is_empty()
            && store.accounts.contains_key(&store.active_account_id))
        .then(|| store.active_account_id.clone())
    });

    match active_account_id.as_ref() {
        Some(active_account_id) if store.active_account_id != *active_account_id => {
            store.active_account_id = active_account_id.clone();
        }
        None if !store.active_account_id.trim().is_empty() => {
            store.active_account_id.clear();
        }
        _ => {}
    }

    active_account_id
}

fn fetch_current_auth_report_at(
    diagnostics: Vec<UsageFetchDiagnostic>,
    endpoints: &CodexEndpoints,
) -> UsageFetchReport {
    let result = read_current_credentials().and_then(|(auth, source)| {
        fetch_with_auth_at(endpoints, auth, source, "Codex".into(), None)
    });
    match result {
        Ok(output) => UsageFetchReport {
            outputs: vec![output],
            diagnostics,
        },
        Err(error) => {
            let mut diagnostics = diagnostics;
            diagnostics.push(UsageFetchDiagnostic::new("Codex", None, error.to_string()));
            UsageFetchReport {
                outputs: Vec::new(),
                diagnostics,
            }
        }
    }
}

fn append_opencode_fallback_at(
    mut report: UsageFetchReport,
    auth_path: &Path,
    endpoints: &CodexEndpoints,
) -> UsageFetchReport {
    if !report.outputs.is_empty() {
        return report;
    }

    let credentials = match read_opencode_credentials_at(auth_path) {
        Ok(credentials) => credentials,
        Err(error) => {
            report
                .diagnostics
                .push(UsageFetchDiagnostic::new("Codex", None, error.to_string()));
            return report;
        }
    };
    let Some((auth, source)) = credentials else {
        return report;
    };

    match fetch_with_auth_at(endpoints, auth, source, "Codex".into(), None) {
        Ok(mut output) => {
            output.credential_source = Some("opencode".to_string());
            report.outputs.push(output);
        }
        Err(error) => {
            report
                .diagnostics
                .push(UsageFetchDiagnostic::new("Codex", None, error.to_string()))
        }
    }
    report
}

pub fn fetch_all() -> Result<Vec<UsageOutput>> {
    let report = fetch_all_report();
    if report.outputs.is_empty() {
        if let Some(diagnostic) = report.diagnostics.into_iter().next() {
            anyhow::bail!("{}", diagnostic.message);
        }
    }
    Ok(report.outputs)
}

pub fn fetch_all_report() -> UsageFetchReport {
    let endpoints = CodexEndpoints::production();
    fetch_all_report_with_opencode_fallback_at(
        CodexFetchIntent::ReadOnly,
        &endpoints,
        &opencode_auth_path(),
    )
}

pub fn fetch_all_report_importing_current_auth() -> UsageFetchReport {
    let endpoints = CodexEndpoints::production();
    fetch_all_report_with_opencode_fallback_at(
        CodexFetchIntent::SaveCurrentLogin,
        &endpoints,
        &opencode_auth_path(),
    )
}

fn fetch_all_report_with_opencode_fallback_at(
    intent: CodexFetchIntent,
    endpoints: &CodexEndpoints,
    auth_path: &Path,
) -> UsageFetchReport {
    let report = if has_native_credentials() {
        fetch_all_report_inner_at(intent, endpoints)
    } else {
        UsageFetchReport::default()
    };

    if has_opencode_auth_candidate_at(auth_path) {
        append_opencode_fallback_at(report, auth_path, endpoints)
    } else {
        report
    }
}

fn load_credentials_store_for_fetch_intent(
    intent: CodexFetchIntent,
) -> Option<CodexCredentialsStore> {
    match intent {
        CodexFetchIntent::ReadOnly => load_credentials_store(),
        CodexFetchIntent::SaveCurrentLogin => CodexAccountStore::default()
            .read_for_update()
            .ok()
            .flatten(),
    }
}

fn fetch_all_report_inner_at(
    intent: CodexFetchIntent,
    endpoints: &CodexEndpoints,
) -> UsageFetchReport {
    let mut diagnostics = Vec::new();
    let current_auth_account = if intent == CodexFetchIntent::SaveCurrentLogin {
        match save_current_auth_account() {
            Ok(info) => Some(info),
            Err(error) => {
                if !is_missing_credentials(&error) {
                    diagnostics.push(UsageFetchDiagnostic::with_kind(
                        "Codex",
                        None,
                        UsageFetchDiagnosticKind::ImportCurrentLoginFailed,
                        UsageFetchDiagnosticSeverity::Warning,
                        format!("failed to import current Codex login: {error}"),
                    ));
                }
                None
            }
        }
    } else {
        None
    };

    let Some(mut store) = load_credentials_store_for_fetch_intent(intent) else {
        return fetch_current_auth_report_at(diagnostics, endpoints);
    };

    if store.accounts.is_empty() {
        return fetch_current_auth_report_at(diagnostics, endpoints);
    }

    let active_account_id = current_auth_account
        .map(|account| account.id)
        .or_else(|| active_account_id_for_usage(&mut store));
    let mut account_ids: Vec<_> = store.accounts.keys().cloned().collect();
    account_ids.sort_by(|a, b| {
        if active_account_id.as_deref() == Some(a.as_str()) {
            std::cmp::Ordering::Less
        } else if active_account_id.as_deref() == Some(b.as_str()) {
            std::cmp::Ordering::Greater
        } else {
            let la = store
                .accounts
                .get(a)
                .and_then(|account| account.label.as_deref())
                .map(|label| account_sort_key(Some(label), a))
                .unwrap_or_else(|| account_sort_key(None, a));
            let lb = store
                .accounts
                .get(b)
                .and_then(|account| account.label.as_deref())
                .map(|label| account_sort_key(Some(label), b))
                .unwrap_or_else(|| account_sort_key(None, b));
            la.cmp(&lb).then_with(|| a.cmp(b))
        }
    });

    let mut outputs = Vec::new();
    for account_id in account_ids {
        let Some(account) = store.accounts.get(&account_id) else {
            continue;
        };
        let usage_account =
            usage_account_from_saved(&account_id, account, active_account_id.as_deref());
        match fetch_with_auth_at(
            endpoints,
            auth_from_account(account),
            CredentialSource::Store(account_id.clone()),
            "Codex".into(),
            Some(usage_account.clone()),
        ) {
            Ok(output) => outputs.push(output),
            Err(error) => diagnostics.push(UsageFetchDiagnostic::new(
                "Codex",
                Some(usage_account),
                error.to_string(),
            )),
        }
    }

    UsageFetchReport {
        outputs,
        diagnostics,
    }
}

async fn consume_reset_credit_with_auth_async(
    auth: Auth,
    source: CredentialSource,
) -> Result<RateLimitResetConsumeResult> {
    let tokens = auth
        .tokens
        .ok_or_else(|| anyhow::anyhow!("No Codex tokens."))?;
    let access_token = tokens
        .access_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No Codex access token."))?;
    let client = reqwest::Client::new();
    let redeem_request_id = uuid::Uuid::new_v4().to_string();

    match consume_reset_credit(
        &client,
        &access_token,
        tokens.account_id.as_deref(),
        &redeem_request_id,
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(e) if is_needs_auth(&e) => {
            // Every caller today passes `Store`, so this guard changes no
            // current behaviour. It is here so the ownership rule is a property
            // of both refresh sites rather than something the usage path
            // enforces and this one happens not to violate.
            if source.refreshable_account_id().is_none() {
                return Err(e.context(format!(
                    "Codex reset credit unavailable: the access token in {} was rejected. \
                     Run 'codex' so the Codex CLI can refresh its own login, then retry.",
                    source.describe()
                )));
            }
            let rt_str = tokens
                .refresh_token
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No refresh token."))?;
            let refreshed =
                refresh_token(&client, &CodexEndpoints::production().token, rt_str).await?;
            let new = refreshed
                .access_token
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Refresh returned no token."))?;

            let mut updated_tokens = tokens.clone();
            updated_tokens.access_token = Some(new.clone());
            if let Some(new_rt) = refreshed.refresh_token {
                updated_tokens.refresh_token = Some(new_rt);
            }
            persist_refreshed_tokens(&source, &updated_tokens);

            consume_reset_credit(
                &client,
                &new,
                updated_tokens.account_id.as_deref(),
                &redeem_request_id,
            )
            .await
        }
        Err(e) => Err(e),
    }
}

pub fn consume_rate_limit_reset_credit(name_or_id: &str) -> Result<RateLimitResetConsumeResult> {
    let store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Codex accounts"))?;
    let resolved = resolve_account_id(&store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {name_or_id}"))?;
    let account = store
        .accounts
        .get(&resolved)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Codex account not found: {resolved}"))?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(consume_reset_credit_with_auth_async(
        auth_from_account(&account),
        CredentialSource::Store(resolved),
    ))
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

pub fn switch_active_account(name_or_id: &str) -> Result<CodexAccountInfo> {
    CodexAccountStore::default().update_existing("No saved Codex accounts", |store| {
        let resolved = resolve_account_id(store, name_or_id)
            .ok_or_else(|| anyhow::anyhow!("Codex account not found: {name_or_id}"))?;
        let account = store
            .accounts
            .get(&resolved)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Codex account not found: {resolved}"))?;

        let path = auth_write_path()?;
        save_auth_tokens(&path, &account.tokens)?;

        store.active_account_id = resolved.clone();

        Ok(account_info(store, &resolved, &account))
    })
}

/// Removes an account from tokscale's store only. The codex CLI's own
/// `auth.json` is intentionally left untouched: rewriting it would silently
/// re-log the codex CLI into a different account (or log it out entirely).
pub fn remove_account(name_or_id: &str) -> Result<CodexAccountInfo> {
    CodexAccountStore::default().update_existing("No saved Codex accounts", |store| {
        let resolved = resolve_account_id(store, name_or_id)
            .ok_or_else(|| anyhow::anyhow!("Codex account not found: {name_or_id}"))?;
        let active_account_id = current_auth_account_id_in_store(store).or_else(|| {
            (!store.active_account_id.trim().is_empty()).then(|| store.active_account_id.clone())
        });
        if active_account_id.as_deref() == Some(resolved.as_str()) {
            anyhow::bail!(
                "Cannot remove the active Codex account. Switch to another Codex account or log out of Codex first."
            );
        }
        remove_account_from_store(store, &resolved)
    })
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
        "\n  {}",
        format!("Stopped tracking Codex account {}", display.bold()).green()
    );
    println!(
        "{}\n",
        "  The codex CLI login was not changed.".bright_black()
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
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use crate::commands::usage::test_server::{spawn_server, Seen};

    fn test_store_path(tmp: &TempDir) -> PathBuf {
        tmp.path().join("codex-credentials.json")
    }

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

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &Path) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn usage_response_treats_null_additional_rate_limits_as_empty() -> Result<()> {
        let usage: Usage = serde_json::from_value(serde_json::json!({
            "email": "plus@example.com",
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 1,
                    "reset_at": 1781929382
                },
                "secondary_window": {
                    "used_percent": 16,
                    "reset_at": 1782413780
                }
            },
            "additional_rate_limits": null
        }))?;

        assert_eq!(usage.email.as_deref(), Some("plus@example.com"));
        assert!(usage.additional_rate_limits.is_empty());
        Ok(())
    }

    #[test]
    fn rate_limit_labels_follow_window_duration() -> Result<()> {
        for (primary_seconds, secondary_seconds, expected) in [
            (18_000, 604_800, ["5h", "Weekly"]),
            (604_800, 18_000, ["Weekly", "5h"]),
        ] {
            let rate_limit: RateLimit = serde_json::from_value(serde_json::json!({
                "primary_window": {
                    "used_percent": 0,
                    "limit_window_seconds": primary_seconds
                },
                "secondary_window": {
                    "used_percent": 0,
                    "limit_window_seconds": secondary_seconds
                }
            }))?;

            let mut metrics = Vec::new();
            push_rate_limit_metrics(&mut metrics, None, &rate_limit);
            assert_eq!(metric_labels(&metrics), expected);

            let mut prefixed_metrics = Vec::new();
            push_rate_limit_metrics(&mut prefixed_metrics, Some("Spark"), &rate_limit);
            assert_eq!(
                metric_labels(&prefixed_metrics),
                expected.map(|label| format!("Spark {}", label.to_ascii_lowercase()))
            );
        }
        Ok(())
    }

    #[test]
    fn rate_limit_labels_keep_legacy_fallbacks_without_duration() -> Result<()> {
        let rate_limit: RateLimit = serde_json::from_value(serde_json::json!({
            "primary_window": { "used_percent": 10 },
            "secondary_window": { "used_percent": 20 }
        }))?;

        let mut metrics = Vec::new();
        push_rate_limit_metrics(&mut metrics, None, &rate_limit);
        assert_eq!(metric_labels(&metrics), ["5h", "Weekly"]);
        Ok(())
    }

    fn metric_labels(metrics: &[UsageMetric]) -> Vec<&str> {
        metrics.iter().map(|metric| metric.label.as_str()).collect()
    }

    #[test]
    fn chatgpt_json_body_treats_html_as_auth_expiry() {
        let error = parse_chatgpt_json_body::<ResetCreditsResponse>(
            "<html><body>please sign in</body></html>",
        )
        .unwrap_err();

        assert!(
            is_needs_auth(&error),
            "expected Codex auth expiry, got: {error:#}"
        );
    }

    #[test]
    fn chatgpt_json_body_parses_reset_credit_response() -> Result<()> {
        let response: ResetCreditsResponse = parse_chatgpt_json_body(
            r#"{"available_count":1,"credits":[{"id":"credit_1","status":"available"}]}"#,
        )?;

        assert_eq!(response.available_count, Some(1));
        assert_eq!(response.credits.len(), 1);
        assert_eq!(response.credits[0].id.as_deref(), Some("credit_1"));
        Ok(())
    }

    #[test]
    fn merge_reset_credits_preserves_summary_when_detail_count_is_null() {
        // Summary reports a known non-zero count; the detail body's
        // available_count is null (-> None). The summary count must survive so
        // the Reset button still shows it.
        let summary = Some(UsageResetCredits {
            available_count: 2,
            credits: Vec::new(),
        });
        let details = reset_credits_from_response(
            parse_chatgpt_json_body(r#"{"available_count":null}"#).unwrap(),
        );
        assert!(details.is_none());

        let merged = merge_reset_credits(summary, details);
        assert_eq!(merged.expect("summary preserved").available_count, 2);
    }

    #[test]
    fn merge_reset_credits_prefers_detail_when_present() {
        let summary = Some(UsageResetCredits {
            available_count: 2,
            credits: Vec::new(),
        });
        let details = reset_credits_from_response(
            parse_chatgpt_json_body(
                r#"{"available_count":1,"credits":[{"id":"credit_1","status":"available"}]}"#,
            )
            .unwrap(),
        );

        let merged = merge_reset_credits(summary, details).expect("detail applied");
        assert_eq!(merged.available_count, 1);
        assert_eq!(merged.credits.len(), 1);
        assert_eq!(merged.credits[0].id.as_deref(), Some("credit_1"));
    }

    #[test]
    fn merge_reset_credits_returns_detail_when_summary_absent() {
        let details = Some(UsageResetCredits {
            available_count: 3,
            credits: Vec::new(),
        });
        let merged = merge_reset_credits(None, details).expect("detail used");
        assert_eq!(merged.available_count, 3);
    }

    #[test]
    fn should_fetch_reset_details_unless_summary_is_explicitly_zero() {
        // Absent summary (unknown): fetch the detail endpoint, since it is the
        // only source of credits for accounts whose usage payload omits the
        // inline summary. Skipping here would hide reset credits in production.
        assert!(should_fetch_reset_details(None));
        // Summary present but zero credits: nothing to enrich, skip.
        assert!(!should_fetch_reset_details(Some(&UsageResetCredits {
            available_count: 0,
            credits: Vec::new(),
        })));
        // Summary present with available credits: enrich via detail call.
        assert!(should_fetch_reset_details(Some(&UsageResetCredits {
            available_count: 1,
            credits: Vec::new(),
        })));
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
    fn usage_active_account_matches_current_token_identity() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_a".to_string(),
            CodexAccount {
                tokens: tokens("access-a", Some("acct_a")),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("work".to_string()),
            },
        );
        accounts.insert(
            "acct_b".to_string(),
            CodexAccount {
                tokens: tokens("access-b", Some("acct_b")),
                created_at: "2026-01-02T00:00:00Z".to_string(),
                label: Some("personal".to_string()),
            },
        );
        let store = CodexCredentialsStore {
            version: 1,
            active_account_id: "acct_a".to_string(),
            accounts,
        };

        let current_tokens = tokens("rotated-access-b", Some("acct_b"));
        let active_id = matching_account_id_for_tokens(&store, &current_tokens);
        assert_eq!(active_id.as_deref(), Some("acct_b"));

        let account_a = store.accounts.get("acct_a").unwrap();
        let account_b = store.accounts.get("acct_b").unwrap();
        assert!(!usage_account_from_saved("acct_a", account_a, active_id.as_deref()).is_active);
        assert!(usage_account_from_saved("acct_b", account_b, active_id.as_deref()).is_active);
    }

    #[test]
    fn usage_active_account_handles_collision_suffixed_account_ids() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_shared".to_string(),
            CodexAccount {
                tokens: tokens_with_id_token("access-a", Some("acct_other"), "id-token-a"),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("old".to_string()),
            },
        );
        accounts.insert(
            "acct_shared-2".to_string(),
            CodexAccount {
                tokens: tokens_with_id_token("access-b", Some("acct_shared"), "id-token-b"),
                created_at: "2026-01-02T00:00:00Z".to_string(),
                label: Some("current".to_string()),
            },
        );
        let store = CodexCredentialsStore {
            version: 1,
            active_account_id: "acct_shared".to_string(),
            accounts,
        };

        let current_tokens =
            tokens_with_id_token("rotated-access", Some("acct_shared"), "id-token-b");
        assert_eq!(
            matching_account_id_for_tokens(&store, &current_tokens).as_deref(),
            Some("acct_shared-2")
        );
    }

    #[test]
    #[serial_test::serial]
    fn active_account_id_for_usage_updates_only_loaded_snapshot() -> Result<()> {
        let config = TempDir::new()?;
        let codex_home = TempDir::new()?;
        let auth = Auth {
            tokens: Some(tokens("rotated-access-b", Some("acct_b"))),
        };
        std::fs::write(
            codex_home.path().join("auth.json"),
            serde_json::to_string_pretty(&auth)?,
        )?;
        let store_path = test_store_path(&config);
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "acct_a".to_string(),
                accounts: HashMap::from([
                    (
                        "acct_a".to_string(),
                        CodexAccount {
                            tokens: tokens("access-a", Some("acct_a")),
                            created_at: "2026-01-01T00:00:00Z".to_string(),
                            label: Some("work".to_string()),
                        },
                    ),
                    (
                        "acct_b".to_string(),
                        CodexAccount {
                            tokens: tokens("access-b", Some("acct_b")),
                            created_at: "2026-01-02T00:00:00Z".to_string(),
                            label: Some("personal".to_string()),
                        },
                    ),
                ]),
            },
        )?;

        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config.path());
        let _codex_guard = EnvVarGuard::set_path("CODEX_HOME", codex_home.path());

        let mut loaded = load_credentials_store().expect("test store should load");
        let active_id = active_account_id_for_usage(&mut loaded);

        assert_eq!(active_id.as_deref(), Some("acct_b"));
        assert_eq!(loaded.active_account_id, "acct_b");
        let persisted: CodexCredentialsStore =
            serde_json::from_str(&std::fs::read_to_string(&store_path)?)?;
        assert_eq!(persisted.active_account_id, "acct_a");
        Ok(())
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
        let store_path = test_store_path(&tmp);
        save_credentials_store_at_path(&store_path, &store)?;

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert_eq!(loaded.active_account_id, "acct_b");
        Ok(())
    }

    #[test]
    fn load_credentials_store_read_path_does_not_persist_active_account_repair() -> Result<()> {
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
        let store_path = test_store_path(&tmp);
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "missing".to_string(),
                accounts,
            },
        )?;
        let lock_path = CodexAccountStore::at_path(&store_path).lock_path();
        if lock_path.exists() {
            std::fs::remove_file(&lock_path)?;
        }
        let before = std::fs::read_to_string(&store_path)?;

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        let after = std::fs::read_to_string(&store_path)?;

        assert_eq!(loaded.active_account_id, "acct_b");
        assert_eq!(after, before);
        assert!(!lock_path.exists());
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn usage_surface_store_loader_persists_active_account_repair() -> Result<()> {
        let config = TempDir::new()?;
        let store_path = test_store_path(&config);
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "missing".to_string(),
                accounts: HashMap::from([
                    (
                        "acct_a".to_string(),
                        CodexAccount {
                            tokens: tokens("access-a", Some("acct_a")),
                            created_at: "2026-01-01T00:00:00Z".to_string(),
                            label: Some("zulu".to_string()),
                        },
                    ),
                    (
                        "acct_b".to_string(),
                        CodexAccount {
                            tokens: tokens("access-b", Some("acct_b")),
                            created_at: "2026-01-02T00:00:00Z".to_string(),
                            label: Some("alpha".to_string()),
                        },
                    ),
                ]),
            },
        )?;

        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config.path());

        let loaded = load_credentials_store_for_fetch_intent(CodexFetchIntent::SaveCurrentLogin);

        assert_eq!(
            loaded
                .as_ref()
                .map(|store| store.active_account_id.as_str()),
            Some("acct_b")
        );
        let persisted: CodexCredentialsStore =
            serde_json::from_str(&std::fs::read_to_string(&store_path)?)?;
        assert_eq!(persisted.active_account_id, "acct_b");
        Ok(())
    }

    #[test]
    fn load_credentials_store_preserves_empty_active_account() -> Result<()> {
        let tmp = TempDir::new()?;
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_a".to_string(),
            CodexAccount {
                tokens: tokens("access-a", Some("acct_a")),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("work".to_string()),
            },
        );
        let store = CodexCredentialsStore {
            version: 1,
            active_account_id: String::new(),
            accounts,
        };
        let store_path = test_store_path(&tmp);
        save_credentials_store_at_path(&store_path, &store)?;

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert!(loaded.active_account_id.is_empty());
        let account = loaded.accounts.get("acct_a").unwrap();
        assert!(!account_info(&loaded, "acct_a", account).is_active);
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
    fn save_account_from_auth_at_path_imports_tokens_without_touching_real_home() -> Result<()> {
        let tmp = TempDir::new()?;
        let store_path = test_store_path(&tmp);
        let info = save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens("access-a", Some("acct_a"))),
            },
            Some("work"),
            true,
        )?;

        assert_eq!(info.id, "acct_a");
        assert_eq!(info.label.as_deref(), Some("work"));
        assert!(info.is_active);

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert_eq!(loaded.active_account_id, "acct_a");
        assert!(loaded.accounts.contains_key("acct_a"));
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn save_current_auth_account_imports_codex_home_auth_when_store_missing() -> Result<()> {
        let config = TempDir::new()?;
        let codex_home = TempDir::new()?;
        let auth = Auth {
            tokens: Some(tokens("access-current", Some("acct_current"))),
        };
        std::fs::write(
            codex_home.path().join("auth.json"),
            serde_json::to_string_pretty(&auth)?,
        )?;

        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config.path());
        let _codex_guard = EnvVarGuard::set_path("CODEX_HOME", codex_home.path());

        let info = save_current_auth_account()?;
        assert_eq!(info.id, "acct_current");
        assert!(info.is_active);

        let store_path = test_store_path(&config);
        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert_eq!(loaded.active_account_id, "acct_current");
        let account = loaded.accounts.get("acct_current").unwrap();
        assert_eq!(
            account.tokens.access_token.as_deref(),
            Some("access-current")
        );
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn save_current_auth_account_imports_codex_home_auth_when_store_has_other_accounts(
    ) -> Result<()> {
        let config = TempDir::new()?;
        let codex_home = TempDir::new()?;
        let auth = Auth {
            tokens: Some(tokens("access-current", Some("acct_current"))),
        };
        std::fs::write(
            codex_home.path().join("auth.json"),
            serde_json::to_string_pretty(&auth)?,
        )?;
        let store_path = test_store_path(&config);
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: String::new(),
                accounts: HashMap::from([
                    (
                        "acct_a".to_string(),
                        CodexAccount {
                            tokens: tokens("access-a", Some("acct_a")),
                            created_at: "2026-01-01T00:00:00Z".to_string(),
                            label: Some("work".to_string()),
                        },
                    ),
                    (
                        "acct_b".to_string(),
                        CodexAccount {
                            tokens: tokens("access-b", Some("acct_b")),
                            created_at: "2026-01-02T00:00:00Z".to_string(),
                            label: Some("personal".to_string()),
                        },
                    ),
                ]),
            },
        )?;

        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config.path());
        let _codex_guard = EnvVarGuard::set_path("CODEX_HOME", codex_home.path());

        let info = save_current_auth_account()?;
        assert_eq!(info.id, "acct_current");
        assert!(info.is_active);

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert_eq!(loaded.active_account_id, "acct_current");
        assert_eq!(loaded.accounts.len(), 3);
        assert!(loaded.accounts.contains_key("acct_a"));
        assert!(loaded.accounts.contains_key("acct_b"));
        assert!(loaded.accounts.contains_key("acct_current"));
        Ok(())
    }

    #[test]
    fn save_account_from_auth_at_path_preserves_label_when_updating_same_account() -> Result<()> {
        let tmp = TempDir::new()?;
        let store_path = test_store_path(&tmp);
        save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens("access-a", Some("acct_a"))),
            },
            Some("work"),
            true,
        )?;

        let info = save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens("access-b", Some("acct_a"))),
            },
            None,
            true,
        )?;

        assert_eq!(info.id, "acct_a");
        assert_eq!(info.label.as_deref(), Some("work"));

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        let account = loaded.accounts.get("acct_a").unwrap();
        assert_eq!(account.label.as_deref(), Some("work"));
        assert_eq!(account.tokens.access_token.as_deref(), Some("access-b"));
        Ok(())
    }

    #[test]
    fn save_account_from_auth_at_path_keeps_existing_account_on_identity_collision() -> Result<()> {
        let tmp = TempDir::new()?;
        let store_path = test_store_path(&tmp);
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_shared".to_string(),
            CodexAccount {
                tokens: tokens_with_id_token("access-a", Some("acct_other"), "id-token-a"),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("work".to_string()),
            },
        );
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "acct_shared".to_string(),
                accounts,
            },
        )?;

        let info = save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens_with_id_token(
                    "access-b",
                    Some("acct_shared"),
                    "id-token-b",
                )),
            },
            None,
            true,
        )?;

        assert_eq!(info.id, "acct_shared-2");

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
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
    fn save_account_from_auth_at_path_can_add_without_changing_active_account() -> Result<()> {
        let tmp = TempDir::new()?;
        let store_path = test_store_path(&tmp);
        save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens("access-a", Some("acct_a"))),
            },
            Some("work"),
            true,
        )?;

        let info = save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens("access-b", Some("acct_b"))),
            },
            Some("personal"),
            false,
        )?;

        assert_eq!(info.id, "acct_b");
        assert!(!info.is_active);

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert_eq!(loaded.active_account_id, "acct_a");
        assert!(loaded.accounts.contains_key("acct_a"));
        assert!(loaded.accounts.contains_key("acct_b"));
        Ok(())
    }

    #[test]
    fn save_account_from_auth_at_path_keeps_empty_active_when_inactive_import_is_first_account(
    ) -> Result<()> {
        let tmp = TempDir::new()?;
        let store_path = test_store_path(&tmp);
        let info = save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens("access-a", Some("acct_a"))),
            },
            Some("work"),
            false,
        )?;

        assert_eq!(info.id, "acct_a");
        assert!(!info.is_active);

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert!(loaded.active_account_id.is_empty());
        assert!(loaded.accounts.contains_key("acct_a"));
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn remove_account_refuses_current_auth_account() -> Result<()> {
        let config = TempDir::new()?;
        let codex_home = TempDir::new()?;
        let auth = Auth {
            tokens: Some(tokens("access-current", Some("acct_current"))),
        };
        std::fs::write(
            codex_home.path().join("auth.json"),
            serde_json::to_string_pretty(&auth)?,
        )?;
        let store_path = test_store_path(&config);
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "acct_current".to_string(),
                accounts: HashMap::from([
                    (
                        "acct_current".to_string(),
                        CodexAccount {
                            tokens: tokens("access-current", Some("acct_current")),
                            created_at: "2026-01-01T00:00:00Z".to_string(),
                            label: Some("work".to_string()),
                        },
                    ),
                    (
                        "acct_other".to_string(),
                        CodexAccount {
                            tokens: tokens("access-other", Some("acct_other")),
                            created_at: "2026-01-02T00:00:00Z".to_string(),
                            label: Some("personal".to_string()),
                        },
                    ),
                ]),
            },
        )?;

        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config.path());
        let _codex_guard = EnvVarGuard::set_path("CODEX_HOME", codex_home.path());

        let error =
            remove_account("acct_current").expect_err("current auth account must not be removable");
        assert!(
            error.to_string().contains("Cannot remove the active"),
            "unexpected error: {error:#}"
        );
        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert!(loaded.accounts.contains_key("acct_current"));
        assert!(loaded.accounts.contains_key("acct_other"));
        assert_eq!(loaded.active_account_id, "acct_current");
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn remove_account_refuses_store_active_account_without_current_auth() -> Result<()> {
        let config = TempDir::new()?;
        let codex_home = TempDir::new()?;
        let store_path = test_store_path(&config);
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "acct_current".to_string(),
                accounts: HashMap::from([
                    (
                        "acct_current".to_string(),
                        CodexAccount {
                            tokens: tokens("access-current", Some("acct_current")),
                            created_at: "2026-01-01T00:00:00Z".to_string(),
                            label: Some("work".to_string()),
                        },
                    ),
                    (
                        "acct_other".to_string(),
                        CodexAccount {
                            tokens: tokens("access-other", Some("acct_other")),
                            created_at: "2026-01-02T00:00:00Z".to_string(),
                            label: Some("personal".to_string()),
                        },
                    ),
                ]),
            },
        )?;

        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config.path());
        let _codex_guard = EnvVarGuard::set_path("CODEX_HOME", codex_home.path());

        let error =
            remove_account("acct_current").expect_err("store active account must not be removable");
        assert!(
            error.to_string().contains("Cannot remove the active"),
            "unexpected error: {error:#}"
        );
        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert!(loaded.accounts.contains_key("acct_current"));
        assert!(loaded.accounts.contains_key("acct_other"));
        assert_eq!(loaded.active_account_id, "acct_current");
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

        let removed = remove_account_from_store(&mut store, "personal")?;

        assert_eq!(removed.id, "acct_b");
        assert!(!removed.is_active);
        assert_eq!(store.active_account_id, "acct_a");
        assert!(!store.accounts.contains_key("acct_b"));
        Ok(())
    }

    #[test]
    fn remove_account_from_store_clears_active_when_removing_active() -> Result<()> {
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

        let removed = remove_account_from_store(&mut store, "work")?;

        assert_eq!(removed.id, "acct_a");
        assert!(removed.is_active);
        assert!(store.active_account_id.is_empty());
        let account_b = store.accounts.get("acct_b").unwrap();
        assert!(!usage_account_from_saved("acct_b", account_b, None).is_active);
        Ok(())
    }

    #[test]
    fn remove_account_from_store_clears_active_when_last_account_removed() -> Result<()> {
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_a".to_string(),
            CodexAccount {
                tokens: tokens("access-a", Some("acct_a")),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("Work".to_string()),
            },
        );
        let mut store = CodexCredentialsStore {
            version: 1,
            active_account_id: "acct_a".to_string(),
            accounts,
        };

        let removed = remove_account_from_store(&mut store, "acct_a")?;

        assert_eq!(removed.id, "acct_a");
        assert!(removed.is_active);
        assert!(store.accounts.is_empty());
        assert!(store.active_account_id.is_empty());
        Ok(())
    }

    #[test]
    fn save_account_from_auth_reuses_suffixed_account_with_same_identity() -> Result<()> {
        let tmp = TempDir::new()?;
        let store_path = tmp.path().join("codex-credentials.json");
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_shared".to_string(),
            CodexAccount {
                tokens: tokens_with_id_token("access-a", Some("acct_other"), "id-token-a"),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("work".to_string()),
            },
        );
        accounts.insert(
            "acct_shared-2".to_string(),
            CodexAccount {
                tokens: tokens_with_id_token("access-b", Some("acct_shared"), "id-token-b"),
                created_at: "2026-01-02T00:00:00Z".to_string(),
                label: Some("personal".to_string()),
            },
        );
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "acct_shared".to_string(),
                accounts,
            },
        )?;

        let info = save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens_with_id_token(
                    "access-c",
                    Some("acct_shared"),
                    "id-token-c",
                )),
            },
            None,
            true,
        )?;

        assert_eq!(info.id, "acct_shared-2");

        let loaded = load_credentials_store_from_path(&store_path).unwrap();
        assert_eq!(loaded.accounts.len(), 2);
        assert!(!loaded.accounts.contains_key("acct_shared-3"));
        assert_eq!(
            loaded
                .accounts
                .get("acct_shared-2")
                .and_then(|account| account.tokens.access_token.as_deref()),
            Some("access-c")
        );
        Ok(())
    }

    #[test]
    fn save_account_from_auth_refuses_to_overwrite_future_store_version() -> Result<()> {
        let tmp = TempDir::new()?;
        let store_path = tmp.path().join("codex-credentials.json");
        let future_store =
            r#"{"version":2,"vaults":[{"id":"acct_a","sealed":"0xdeadbeef"}],"accounts":{}}"#;
        std::fs::write(&store_path, future_store)?;

        let result = save_account_from_auth_at_path(
            &store_path,
            Auth {
                tokens: Some(tokens("access-a", Some("acct_a"))),
            },
            None,
            true,
        );

        let error = result.expect_err("future-version store must not be overwritten");
        assert!(
            error.to_string().contains("version 2"),
            "unexpected error: {error}"
        );
        assert_eq!(std::fs::read_to_string(&store_path)?, future_store);
        Ok(())
    }

    // ---------------------------------------------------------------------
    // #1001 / #1005: tokscale must not write the codex CLI's auth.json.
    // ---------------------------------------------------------------------

    /// A codex CLI `auth.json` as the CLI actually writes it. Every key outside
    /// `tokens` is one `auth_document` would delete: `OPENAI_API_KEY`,
    /// `agent_identity` and `bedrock_api_key` are standalone credentials,
    /// `auth_mode` decides which of them codex uses, and the unknown key stands
    /// in for the next field codex adds -- it owns this schema, not tokscale.
    const CODEX_AUTH_FIXTURE: &str = r#"{
  "auth_mode": "chatgpt",
  "OPENAI_API_KEY": "sk-user-owned-api-key",
  "tokens": {
    "id_token": "codex-owned-id-token",
    "access_token": "stale-access-token",
    "refresh_token": "codex-owned-refresh-token",
    "account_id": "acct_codex"
  },
  "last_refresh": "2026-07-01T00:00:00Z",
  "agent_identity": "codex-owned-agent-jwt",
  "personal_access_token": "codex-owned-pat",
  "bedrock_api_key": { "key": "codex-owned-bedrock-key" },
  "someKeyTokscaleDoesNotModel": { "keep": true }
}"#;

    const OPENCODE_AUTH_FIXTURE: &str = r#"{
  "anthropic": {
    "type": "api",
    "key": "unrelated-provider-key"
  },
  "openai": {
    "type": "oauth",
    "refresh": "opencode-owned-refresh-token",
    "access": "opencode-access-token",
    "expires": 1786089600000,
    "accountId": "acct_opencode"
  }
}"#;

    #[test]
    fn opencode_oauth_maps_only_read_only_usage_credentials() -> Result<()> {
        let tmp = TempDir::new()?;
        let auth_path = tmp.path().join("auth.json");
        std::fs::write(&auth_path, OPENCODE_AUTH_FIXTURE)?;

        let (auth, source) = read_opencode_credentials_at(&auth_path)?
            .expect("OpenCode OAuth credentials should be detected");
        let tokens = auth.tokens.expect("mapped tokens");

        assert_eq!(
            tokens.access_token.as_deref(),
            Some("opencode-access-token")
        );
        assert_eq!(tokens.account_id.as_deref(), Some("acct_opencode"));
        assert!(tokens.refresh_token.is_none());
        assert!(tokens.id_token.is_none());
        assert!(matches!(source, CredentialSource::OpenCodeFile(path) if path == auth_path));
        Ok(())
    }

    #[test]
    fn opencode_api_key_is_not_a_chatgpt_usage_credential() -> Result<()> {
        let tmp = TempDir::new()?;
        let auth_path = tmp.path().join("auth.json");
        std::fs::write(
            &auth_path,
            r#"{"openai":{"type":"api","key":"sk-openai-api-key"}}"#,
        )?;

        assert!(read_opencode_credentials_at(&auth_path)?.is_none());
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    #[cfg(not(target_os = "macos"))]
    fn opencode_api_key_does_not_activate_codex_usage() -> Result<()> {
        let home = TempDir::new()?;
        let config = TempDir::new()?;
        let codex_home = TempDir::new()?;
        let xdg_data = TempDir::new()?;
        let _home_guard = EnvVarGuard::set_path("HOME", home.path());
        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config.path());
        let _codex_guard = EnvVarGuard::set_path("CODEX_HOME", codex_home.path());
        let _xdg_data_guard = EnvVarGuard::set_path("XDG_DATA_HOME", xdg_data.path());
        let auth_path = xdg_data.path().join("opencode").join("auth.json");
        std::fs::create_dir_all(auth_path.parent().expect("auth parent"))?;
        std::fs::write(
            &auth_path,
            r#"{"openai":{"type":"api","key":"sk-openai-api-key"}}"#,
        )?;

        assert!(!has_credentials());
        Ok(())
    }

    #[test]
    fn malformed_opencode_auth_remains_a_usage_candidate() -> Result<()> {
        let tmp = TempDir::new()?;
        let auth_path = tmp.path().join("auth.json");
        std::fs::write(&auth_path, "not json")?;

        assert!(has_opencode_usage_candidate_at(&auth_path));
        Ok(())
    }

    #[test]
    fn malformed_opencode_oauth_is_reported_with_its_path() -> Result<()> {
        let tmp = TempDir::new()?;
        let auth_path = tmp.path().join("auth.json");
        std::fs::write(
            &auth_path,
            r#"{"openai":{"type":"oauth","refresh":"refresh","expires":1}}"#,
        )?;

        let error = read_opencode_credentials_at(&auth_path)
            .expect_err("OAuth without an access token must not be accepted");
        assert!(error.to_string().contains("Failed to parse OpenCode auth"));
        assert!(error.to_string().contains(&auth_path.display().to_string()));
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn opencode_auth_path_honors_xdg_data_home() -> Result<()> {
        let xdg_data = TempDir::new()?;
        let _guard = EnvVarGuard::set_path("XDG_DATA_HOME", xdg_data.path());

        assert_eq!(
            opencode_auth_path(),
            xdg_data.path().join("opencode").join("auth.json")
        );
        Ok(())
    }

    #[test]
    fn opencode_fallback_keeps_native_diagnostics_when_it_recovers_usage() -> Result<()> {
        let tmp = TempDir::new()?;
        let auth_path = tmp.path().join("auth.json");
        std::fs::write(&auth_path, OPENCODE_AUTH_FIXTURE)?;
        let (endpoints, log) = spawn_codex_server(vec![200]);
        let native_report = UsageFetchReport {
            outputs: Vec::new(),
            diagnostics: vec![UsageFetchDiagnostic::new(
                "Codex",
                None,
                "saved account token refresh failed",
            )],
        };

        let report = append_opencode_fallback_at(native_report, &auth_path, &endpoints);

        assert_eq!(report.outputs.len(), 1);
        assert_eq!(report.outputs[0].provider, "Codex");
        assert_eq!(
            report.outputs[0].credential_source.as_deref(),
            Some("opencode")
        );
        assert_eq!(
            report.outputs[0].email.as_deref(),
            Some("codex@example.com")
        );
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(
            report.diagnostics[0].message,
            "saved account token refresh failed"
        );
        assert_eq!(
            *log.lock().expect("request log"),
            vec![Seen {
                request: format!("GET {USAGE_PATH}"),
                bearer: Some("opencode-access-token".to_string()),
            }]
        );
        Ok(())
    }

    #[test]
    fn successful_native_usage_suppresses_opencode_fallback() -> Result<()> {
        let tmp = TempDir::new()?;
        let auth_path = tmp.path().join("auth.json");
        std::fs::write(&auth_path, OPENCODE_AUTH_FIXTURE)?;
        let (endpoints, log) = spawn_codex_server(vec![200]);
        let native_output = fetch_with_auth_at(
            &endpoints,
            Auth {
                tokens: Some(tokens("native-codex-access-token", Some("acct_native"))),
            },
            CredentialSource::Keychain,
            "Codex".into(),
            None,
        )?;

        let report = append_opencode_fallback_at(
            UsageFetchReport {
                outputs: vec![native_output],
                diagnostics: Vec::new(),
            },
            &auth_path,
            &endpoints,
        );

        assert_eq!(report.outputs.len(), 1);
        assert!(report.outputs[0].credential_source.is_none());
        assert_eq!(
            *log.lock().expect("request log"),
            vec![Seen {
                request: format!("GET {USAGE_PATH}"),
                bearer: Some("native-codex-access-token".to_string()),
            }]
        );
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn stale_saved_accounts_fall_back_to_opencode_without_hiding_the_failure() -> Result<()> {
        let config_dir = TempDir::new()?;
        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config_dir.path());
        let store_path = config_dir.path().join("codex-credentials.json");
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_saved".to_string(),
            CodexAccount {
                tokens: Tokens {
                    access_token: Some("stale-saved-access-token".to_string()),
                    refresh_token: Some("stale-saved-refresh-token".to_string()),
                    account_id: Some("acct_saved".to_string()),
                    id_token: None,
                },
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("old".to_string()),
            },
        );
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "acct_saved".to_string(),
                accounts,
            },
        )?;
        let store_before = std::fs::read(&store_path)?;

        let opencode_dir = TempDir::new()?;
        let auth_path = opencode_dir.path().join("auth.json");
        std::fs::write(&auth_path, OPENCODE_AUTH_FIXTURE)?;
        let auth_before = std::fs::read(&auth_path)?;

        let (base, log) = spawn_server(|path, request_index| match (path, request_index) {
            (USAGE_PATH, 0) => (401, "{}".to_string()),
            (TOKEN_PATH, 1) => (401, "{}".to_string()),
            (USAGE_PATH, 2) => (200, USAGE_BODY.to_string()),
            _ => (404, "{}".to_string()),
        });
        let endpoints = CodexEndpoints {
            usage: format!("{base}{USAGE_PATH}"),
            reset_credits: format!("{base}{RESET_CREDITS_PATH}"),
            token: format!("{base}{TOKEN_PATH}"),
        };

        let report = fetch_all_report_with_opencode_fallback_at(
            CodexFetchIntent::ReadOnly,
            &endpoints,
            &auth_path,
        );

        assert_eq!(report.outputs.len(), 1);
        assert_eq!(
            report.outputs[0].credential_source.as_deref(),
            Some("opencode")
        );
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(
            report.diagnostics[0]
                .account
                .as_ref()
                .map(|account| account.id.as_str()),
            Some("acct_saved")
        );
        assert_eq!(
            requests(&log),
            vec![
                format!("GET {USAGE_PATH}"),
                format!("POST {TOKEN_PATH}"),
                format!("GET {USAGE_PATH}"),
            ]
        );
        assert_eq!(std::fs::read(&store_path)?, store_before);
        assert_eq!(std::fs::read(&auth_path)?, auth_before);
        Ok(())
    }

    #[test]
    fn rejected_opencode_token_is_never_refreshed_or_rewritten() -> Result<()> {
        let tmp = TempDir::new()?;
        let auth_path = tmp.path().join("auth.json");
        std::fs::write(&auth_path, OPENCODE_AUTH_FIXTURE)?;
        let before = std::fs::read(&auth_path)?;
        let (endpoints, log) = spawn_codex_server(vec![401]);

        let report =
            append_opencode_fallback_at(UsageFetchReport::default(), &auth_path, &endpoints);

        assert!(report.outputs.is_empty());
        assert_eq!(std::fs::read(&auth_path)?, before);
        assert_eq!(
            requests(&log),
            vec![format!("GET {USAGE_PATH}")],
            "OpenCode-owned credentials must never reach the token endpoint"
        );
        let diagnostic = report
            .diagnostics
            .first()
            .expect("rejected OpenCode token should be diagnosed");
        assert!(diagnostic.message.contains("OpenCode"));
        assert!(diagnostic.message.contains("/connect"));
        Ok(())
    }

    /// Path components mirror production so a recorded request line is the same
    /// string the real endpoints would produce.
    const USAGE_PATH: &str = "/backend-api/wham/usage";
    const RESET_CREDITS_PATH: &str = "/backend-api/wham/rate-limit-reset-credits";
    const TOKEN_PATH: &str = "/oauth/token";

    /// `rate_limit_reset_credits` is present and zero so `should_fetch_reset_details`
    /// skips the detail GET; that keeps the request count a clean signal rather
    /// than a number the test has to explain away.
    const USAGE_BODY: &str = r#"{
      "email": "codex@example.com",
      "plan_type": "plus",
      "rate_limit": {
        "primary_window": {"used_percent": 12.5, "limit_window_seconds": 18000, "reset_at": 1781929382}
      },
      "rate_limit_reset_credits": {"available_count": 0}
    }"#;

    const REFRESH_BODY: &str = r#"{
      "access_token": "refreshed-access-token",
      "refresh_token": "refreshed-refresh-token",
      "expires_in": 3600
    }"#;

    fn spawn_codex_server(usage_statuses: Vec<u16>) -> (CodexEndpoints, Arc<Mutex<Vec<Seen>>>) {
        let mut usage_calls = 0usize;
        let (base, log) = spawn_server(move |path, _| {
            if path == USAGE_PATH {
                let status = usage_statuses
                    .get(usage_calls)
                    .copied()
                    .or_else(|| usage_statuses.last().copied())
                    .unwrap_or(200);
                usage_calls += 1;
                (status, USAGE_BODY.to_string())
            } else if path == TOKEN_PATH {
                (200, REFRESH_BODY.to_string())
            } else if path == RESET_CREDITS_PATH {
                (200, r#"{"available_count":0,"credits":[]}"#.to_string())
            } else {
                (404, "{}".to_string())
            }
        });
        (
            CodexEndpoints {
                usage: format!("{base}{USAGE_PATH}"),
                reset_credits: format!("{base}{RESET_CREDITS_PATH}"),
                token: format!("{base}{TOKEN_PATH}"),
            },
            log,
        )
    }

    fn requests(log: &Arc<Mutex<Vec<Seen>>>) -> Vec<String> {
        log.lock()
            .expect("request log")
            .iter()
            .map(|seen| seen.request.clone())
            .collect()
    }

    /// The assertion that actually pins the bug. A reintroduced
    /// refresh-on-codex's-behalf shows up here as a `POST /oauth/token` even if
    /// it never manages to write a file -- and because the token endpoint is
    /// injected, that POST lands on this server instead of vanishing into the
    /// real auth.openai.com where the test could not see it.
    fn assert_only_usage_request(log: &Arc<Mutex<Vec<Seen>>>) {
        let seen = log.lock().expect("request log");
        assert_eq!(
            *seen,
            vec![Seen {
                request: format!("GET {USAGE_PATH}"),
                // The fixture's token, so this also proves the credential came
                // from the temp home and not from the developer's real one.
                bearer: Some("stale-access-token".to_string()),
            }],
            "tokscale made a request other than the single usage GET"
        );
    }

    /// Points `CODEX_HOME` at a temp dir holding [`CODEX_AUTH_FIXTURE`], so
    /// tests enter through the production `read_current_credentials` and get a
    /// genuine `CredentialSource::File` rather than a hand-built one.
    fn codex_home_with_fixture() -> Result<(TempDir, EnvVarGuard, PathBuf)> {
        let codex_home = TempDir::new()?;
        let guard = EnvVarGuard::set_path("CODEX_HOME", codex_home.path());
        let auth_path = codex_home.path().join("auth.json");
        std::fs::write(&auth_path, CODEX_AUTH_FIXTURE)?;
        Ok((codex_home, guard, auth_path))
    }

    /// #1001, ported to codex: a rejected access token must not make tokscale
    /// refresh or rewrite the codex CLI's `auth.json`. Byte equality is the
    /// assertion that matters -- any reconstruction of the document fails it,
    /// whatever fields it happens to keep.
    #[test]
    #[serial_test::serial]
    fn rejected_token_leaves_codex_auth_json_untouched() -> Result<()> {
        let (_codex_home, _guard, auth_path) = codex_home_with_fixture()?;
        let before = std::fs::read(&auth_path)?;
        let (endpoints, log) = spawn_codex_server(vec![401]);

        let (auth, source) = read_current_credentials()?;
        assert!(
            matches!(source, CredentialSource::File(_)),
            "fixture should be read from disk, got {source:?}"
        );
        let result = fetch_with_auth_at(&endpoints, auth, source, "Codex".into(), None);

        // Integrity first: it is the claim the test exists for, and asserting
        // the error message ahead of it would let a regression report the
        // weaker failure.
        assert_eq!(
            String::from_utf8_lossy(&std::fs::read(&auth_path)?),
            String::from_utf8_lossy(&before),
            "tokscale rewrote the codex CLI's auth.json"
        );
        assert_only_usage_request(&log);

        let error = result.expect_err("401 must surface as an error, not a refresh");
        assert!(
            error.to_string().contains("Run 'codex'"),
            "error should point at the codex CLI's own login, got: {error}"
        );
        assert!(
            error.to_string().contains("auth.json"),
            "error should name the rejected credential, got: {error}"
        );
        Ok(())
    }

    /// A 403 takes the same branch as a 401 and must be just as inert.
    #[test]
    #[serial_test::serial]
    fn forbidden_response_leaves_codex_auth_json_untouched() -> Result<()> {
        let (_codex_home, _guard, auth_path) = codex_home_with_fixture()?;
        let before = std::fs::read(&auth_path)?;
        let (endpoints, log) = spawn_codex_server(vec![403]);

        let (auth, source) = read_current_credentials()?;
        let result = fetch_with_auth_at(&endpoints, auth, source, "Codex".into(), None);

        assert!(result.is_err(), "403 must surface as an error");
        assert_eq!(
            String::from_utf8_lossy(&std::fs::read(&auth_path)?),
            String::from_utf8_lossy(&before),
            "tokscale rewrote the codex CLI's auth.json"
        );
        assert_only_usage_request(&log);
        Ok(())
    }

    /// On macOS the credentials can live in the Keychain with no file on disk.
    ///
    /// The `!exists()` assertion below is a guard, not the regression: the old
    /// `persist_tokens` already no-opped on `Keychain`, so no version of this
    /// code would have created the file. What actually fails here against the
    /// unfixed code is the request count -- the old path still spent the codex
    /// CLI's refresh token, it just had nowhere to put the result.
    #[test]
    #[serial_test::serial]
    fn rejected_token_does_not_create_a_codex_auth_file() -> Result<()> {
        let codex_home = TempDir::new()?;
        let _guard = EnvVarGuard::set_path("CODEX_HOME", codex_home.path());
        let auth_path = codex_home.path().join("auth.json");
        let (endpoints, log) = spawn_codex_server(vec![401]);

        let auth: Auth = serde_json::from_str(CODEX_AUTH_FIXTURE)?;
        let result = fetch_with_auth_at(
            &endpoints,
            auth,
            CredentialSource::Keychain,
            "Codex".into(),
            None,
        );

        assert!(result.is_err(), "401 must surface as an error");
        assert!(
            !auth_path.exists(),
            "tokscale created an auth.json the codex CLI did not have"
        );
        assert_only_usage_request(&log);
        Ok(())
    }

    /// The success path never had a reason to write, but it shares the
    /// orchestration, so it is pinned too.
    #[test]
    #[serial_test::serial]
    fn successful_usage_fetch_leaves_codex_auth_json_untouched() -> Result<()> {
        let (_codex_home, _guard, auth_path) = codex_home_with_fixture()?;
        let before = std::fs::read(&auth_path)?;
        let (endpoints, log) = spawn_codex_server(vec![200]);

        let (auth, source) = read_current_credentials()?;
        let output = fetch_with_auth_at(&endpoints, auth, source, "Codex".into(), None)?;

        assert_eq!(output.email.as_deref(), Some("codex@example.com"));
        assert_eq!(output.plan.as_deref(), Some("Plus"));

        assert_eq!(
            String::from_utf8_lossy(&std::fs::read(&auth_path)?),
            String::from_utf8_lossy(&before),
            "tokscale rewrote the codex CLI's auth.json"
        );
        assert_only_usage_request(&log);
        Ok(())
    }

    /// The other half of the rule, and the reason this is not simply "never
    /// refresh": `<config>/codex-credentials.json` is tokscale's own file, and
    /// without a refresh every saved account would go stale an hour after
    /// import, leaving `usage --all` permanently broken.
    ///
    /// This pins the boundary, not the bug -- it passes against the unfixed
    /// code too, which is the point: the guard must not have over-corrected
    /// into breaking saved accounts. It does not claim the store entry's grant
    /// is unshared; see [`CredentialSource::refreshable_account_id`].
    #[test]
    #[serial_test::serial]
    fn rejected_token_still_refreshes_tokscale_own_account_store() -> Result<()> {
        let (_codex_home, _codex_guard, auth_path) = codex_home_with_fixture()?;
        let auth_before = std::fs::read(&auth_path)?;

        let config_dir = TempDir::new()?;
        let _config_guard = EnvVarGuard::set_path("TOKSCALE_CONFIG_DIR", config_dir.path());
        let store_path = config_dir.path().join("codex-credentials.json");
        let mut accounts = HashMap::new();
        accounts.insert(
            "acct_saved".to_string(),
            CodexAccount {
                tokens: Tokens {
                    access_token: Some("stale-access-token".to_string()),
                    refresh_token: Some("tokscale-stored-refresh-token".to_string()),
                    account_id: Some("acct_saved".to_string()),
                    id_token: None,
                },
                created_at: "2026-01-01T00:00:00Z".to_string(),
                label: Some("work".to_string()),
            },
        );
        save_credentials_store_at_path(
            &store_path,
            &CodexCredentialsStore {
                version: 1,
                active_account_id: "acct_saved".to_string(),
                accounts,
            },
        )?;

        let (endpoints, log) = spawn_codex_server(vec![401, 200]);
        let (_id, account, _info) = load_account(Some("acct_saved"))?;
        let output = fetch_with_auth_at(
            &endpoints,
            auth_from_account(&account),
            CredentialSource::Store("acct_saved".to_string()),
            "Codex".into(),
            None,
        )?;

        assert_eq!(output.email.as_deref(), Some("codex@example.com"));
        assert_eq!(
            requests(&log),
            vec![
                format!("GET {USAGE_PATH}"),
                format!("POST {TOKEN_PATH}"),
                format!("GET {USAGE_PATH}"),
            ],
            "the store path should refresh once and retry once"
        );

        let refreshed = load_credentials_store_from_path(&store_path)
            .expect("store must still parse")
            .accounts
            .remove("acct_saved")
            .expect("account must survive the refresh");
        assert_eq!(
            refreshed.tokens.access_token.as_deref(),
            Some("refreshed-access-token")
        );
        assert_eq!(
            refreshed.tokens.refresh_token.as_deref(),
            Some("refreshed-refresh-token")
        );
        assert_eq!(
            refreshed.label.as_deref(),
            Some("work"),
            "refreshing tokens must not drop the account's other fields"
        );

        // The refresh wrote tokscale's own store and nothing else: the codex
        // CLI's file is untouched even though a refresh did happen.
        assert_eq!(
            String::from_utf8_lossy(&std::fs::read(&auth_path)?),
            String::from_utf8_lossy(&auth_before),
            "refreshing a saved account must not touch the codex CLI's auth.json"
        );
        Ok(())
    }
}
