//! Trae (ByteDance AI IDE) integration: credential decryption, token refresh,
//! usage sync, and CLI commands.
//!
//! Two international variants:
//!
//! | Variant | Product line | App Support directory |
//! |---------|--------------|-----------------------|
//! | Solo    | Solo         | TRAE SOLO             |
//! | Ide     | IDE          | Trae                  |
//!
//! Each variant has its own ``ClientId``; the underlying
//! storage/decrypt/token/sync logic is shared.
//!
//! The China variants (trae.com.cn) are intentionally not integrated:
//! the CN backend does not expose a session-level usage query API.
//! They will be added if/when upstream releases an official endpoint.

pub mod auth {
    //! Trae credential management with automatic token refresh.
    //!
    //! Two international Trae variants:
    //!
    //! | Variant | Product line | App Support directory | tokscale client id |
    //! |---------|--------------|-----------------------|--------------------|
    //! | Solo    | Solo         | TRAE SOLO             | trae-solo          |
    //! | Ide     | IDE          | Trae                  | trae               |
    //!
    //! Shared backend (api-sg-central.trae.ai); shared account but usage data
    //! is tracked independently per variant.
    //!
    //! The China variants (trae.com.cn) are intentionally not integrated:
    //! the CN backend does not expose a session-level usage query API.
    //! They will be added if/when upstream releases an official endpoint.
    //!
    //! Credential lifecycle (highest to lowest priority):
    //! 1. Cached `access_token` still valid → use it directly.
    //! 2. Cached `refresh_token` still valid → call `ExchangeToken` to mint a new
    //!    pair and write it back to disk.
    //! 3. `refresh_token` also expired → decrypt the Trae desktop client's
    //!    `storage.json` (`iCubeAuthInfo` entry).
    //! 4. Decryption fails / no `storage.json` → fall back to
    //!    `trae login --manual` (paste a JWT).
    //!
    //! Cache path: `~/.config/tokscale/trae-cache/credentials-{solo,ide}.json`

    use crate::trae::safestorage;
    use anyhow::{Context, Result};
    use base64::Engine;
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Serialize};
    use std::io::Write;
    use std::path::PathBuf;

    // ── API endpoints (constants, not secrets) ─────────────────────────────

    /// Solo / IDE international API host. Read from the `host` field inside
    /// `iCubeAuthInfo` when available; this constant is the hardcoded fallback.
    pub const INTL_HOST: &str = "https://api-sg-central.trae.ai";

    /// Solo / IDE international ClientID. Extracted from `main.js`'s `QE()`
    /// function and verified end-to-end against `ExchangeToken`.
    pub const INTL_CLIENT_ID: &str = "en1oxy7wnw8j9n";

    const EXCHANGE_TOKEN_PATH: &str = "/cloudide/api/v3/trae/oauth/ExchangeToken";

    // ── Storage ────────────────────────────────────────────────────────────

    /// Which Trae variant (international, 2 of them).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum TraeVariant {
        /// TRAE SOLO — international Solo product
        Solo,
        /// Trae — international IDE product
        Ide,
    }

    impl TraeVariant {
        /// tokscale client id string for this variant.
        pub fn client_str(&self) -> &'static str {
            match self {
                Self::Solo => "trae-solo",
                Self::Ide => "trae",
            }
        }

        /// macOS Application Support directory name.
        pub fn app_dir_name(&self) -> &'static str {
            match self {
                Self::Solo => "TRAE SOLO",
                Self::Ide => "Trae",
            }
        }

        pub fn cli_arg(&self) -> &'static str {
            match self {
                Self::Solo => "solo",
                Self::Ide => "ide",
            }
        }

        fn default_host(&self) -> &'static str {
            INTL_HOST
        }

        fn default_client_id(&self) -> &'static str {
            INTL_CLIENT_ID
        }

        fn credentials_filename(&self) -> &'static str {
            match self {
                Self::Solo => "credentials-solo.json",
                Self::Ide => "credentials-ide.json",
            }
        }
    }

    // ── Cache paths ────────────────────────────────────────────────────────

    /// Root directory for Trae sync cache (credentials + sessions + manifest).
    pub fn get_trae_cache_dir() -> PathBuf {
        crate::paths::get_config_dir().join("trae-cache")
    }

    /// How the token was obtained.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum TokenSource {
        /// Auto-decrypted from the Trae desktop client.
        Auto,
        /// Pasted manually by the user.
        Manual,
    }

    /// Cached credentials (persisted to `trae-cache/credentials-*.json`).
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CachedCredentials {
        pub variant: TraeVariant,
        pub token: String,
        pub refresh_token: String,
        /// `access_token` expiration (ISO 8601, UTC).
        pub expired_at: String,
        /// `refresh_token` expiration (ISO 8601, UTC).
        pub refresh_expired_at: String,
        pub host: String,
        pub client_id: String,
        pub source: TokenSource,
        pub user_id: Option<String>,
    }

    impl CachedCredentials {
        /// Whether `access_token` is expired (with a 5-minute safety margin).
        fn is_token_expired(&self) -> bool {
            parse_iso_to_timestamp(&self.expired_at)
                .map(|ts| Utc::now().timestamp_millis() > ts - 300_000)
                .unwrap_or(true)
        }

        /// Whether `refresh_token` is expired (with a 1-day safety margin).
        fn is_refresh_expired(&self) -> bool {
            parse_iso_to_timestamp(&self.refresh_expired_at)
                .map(|ts| Utc::now().timestamp_millis() > ts - 86_400_000)
                .unwrap_or(true)
        }
    }

    fn parse_iso_to_timestamp(s: &str) -> Option<i64> {
        DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp_millis())
    }

    fn creds_path(variant: TraeVariant) -> PathBuf {
        get_trae_cache_dir().join(variant.credentials_filename())
    }

    fn ensure_cache_dir() -> Result<()> {
        let dir = get_trae_cache_dir();
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(())
    }

    fn save_credentials(creds: &CachedCredentials) -> Result<()> {
        ensure_cache_dir()?;
        let path = creds_path(creds.variant);
        let json = serde_json::to_string_pretty(creds)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?
                .write_all(json.as_bytes())?;
        }
        #[cfg(not(unix))]
        std::fs::write(&path, json)?;
        Ok(())
    }

    fn load_credentials(variant: TraeVariant) -> Option<CachedCredentials> {
        let path = creds_path(variant);
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn clear_credentials(variant: TraeVariant) -> Result<()> {
        let path = creds_path(variant);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    // ── Credential resolution (full priority chain) ────────────────────────

    /// Resolve the active `access_token` for the given variant. Handles
    /// expiration → refresh → fall back to decrypting `storage.json`.
    pub async fn resolve_token(variant: TraeVariant) -> Result<String> {
        // 1. Cache hit & still valid.
        if let Some(mut creds) = load_credentials(variant) {
            if !creds.is_token_expired() {
                return Ok(creds.token);
            }
            // 2. `access_token` expired but `refresh_token` still valid.
            if !creds.is_refresh_expired() {
                match exchange_token(
                    creds.host.clone(),
                    creds.client_id.clone(),
                    &creds.refresh_token,
                    &creds.token,
                )
                .await
                {
                    Ok(new) => {
                        creds.token = new.token;
                        creds.refresh_token = new.refresh_token;
                        creds.expired_at = new.expired_at;
                        creds.refresh_expired_at = new.refresh_expired_at;
                        save_credentials(&creds)?;
                        return Ok(creds.token);
                    }
                    Err(e) => {
                        eprintln!(
                        "  Trae {} refresh token failed: {e}; falling back to storage.json decryption",
                        variant.client_str()
                    );
                    }
                }
            }
        }

        // 3. Decrypt from the Trae desktop client's `storage.json`.
        //
        // The on-disk credentials can be stale even on the very first read:
        // the desktop client may have shipped them with an expired
        // `access_token` if it hasn't been opened recently. Run the same
        // expiry → refresh dance as the cache-hit branch so the caller
        // doesn't immediately hit a 401.
        match decrypt_from_storage(variant) {
            Ok(mut creds) => {
                if creds.is_token_expired() {
                    if creds.is_refresh_expired() {
                        eprintln!(
                            "  Trae {} decrypted credentials are fully expired; falling through to manual login",
                            variant.client_str()
                        );
                    } else {
                        match exchange_token(
                            creds.host.clone(),
                            creds.client_id.clone(),
                            &creds.refresh_token,
                            &creds.token,
                        )
                        .await
                        {
                            Ok(new) => {
                                creds.token = new.token;
                                creds.refresh_token = new.refresh_token;
                                creds.expired_at = new.expired_at;
                                creds.refresh_expired_at = new.refresh_expired_at;
                                save_credentials(&creds)?;
                                return Ok(creds.token);
                            }
                            Err(e) => {
                                eprintln!(
                                    "  Trae {} decrypted token is stale and refresh failed: {e}",
                                    variant.client_str()
                                );
                            }
                        }
                    }
                } else {
                    save_credentials(&creds)?;
                    return Ok(creds.token);
                }
            }
            Err(e) => {
                eprintln!("  Trae {} auto-decrypt failed: {e}", variant.client_str());
            }
        }

        // 4. Everything failed.
        Err(anyhow::anyhow!(
        "Could not obtain a Trae {} access token. Run `tokscale trae login --manual --variant {}` to paste a JWT manually.",
        variant.client_str(),
        variant.cli_arg()
    ))
    }

    // ── storage.json decryption ────────────────────────────────────────────

    fn decrypt_from_storage(variant: TraeVariant) -> Result<CachedCredentials> {
        let home = dirs::home_dir().context("could not determine home directory")?;
        let app_dir = home
            .join("Library/Application Support")
            .join(variant.app_dir_name());
        let storage = app_dir.join("User/globalStorage/storage.json");

        if !storage.exists() {
            return Err(anyhow::anyhow!(
                "storage.json for {} not found (expected at: {})",
                variant.app_dir_name(),
                storage.display()
            ));
        }

        let obj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&storage)?)?;
        let b64 = obj
            .get("iCubeAuthInfo://icube.cloudide")
            .and_then(|v| v.as_str())
            .or_else(|| {
                // Walk every key starting with iCubeAuthInfo and take the longest
                // value (in practice that's the cloudide entry).
                obj.as_object().and_then(|o| {
                    o.iter()
                        .filter(|(k, _)| k.starts_with("iCubeAuthInfo"))
                        .map(|(_, v)| v)
                        .filter_map(|v| v.as_str())
                        .max_by_key(|s| s.len())
                })
            })
            .context("no iCubeAuthInfo entry found in storage.json")?;

        let json = safestorage::decrypt_base64_blob(b64)?;
        let raw: serde_json::Value =
            serde_json::from_str(&json).context("decrypted iCubeAuthInfo is not valid JSON")?;

        let token = raw["token"]
            .as_str()
            .context("missing `token` field")?
            .to_string();
        let refresh_token = raw["refreshToken"]
            .as_str()
            .context("missing `refreshToken` field")?
            .to_string();
        let expired_at = raw["expiredAt"]
            .as_str()
            .map(String::from)
            .unwrap_or_default();
        let refresh_expired_at = raw["refreshExpiredAt"]
            .as_str()
            .map(String::from)
            .unwrap_or_default();
        let host = raw["host"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| variant.default_host().to_string());
        let user_id = raw["userId"].as_str().map(String::from);

        Ok(CachedCredentials {
            variant,
            token,
            refresh_token,
            expired_at,
            refresh_expired_at,
            host,
            client_id: variant.default_client_id().to_string(),
            source: TokenSource::Auto,
            user_id,
        })
    }

    // ── HTTP operations ────────────────────────────────────────────────────

    /// ExchangeToken response (nested under `Result`).
    #[derive(Debug, Deserialize)]
    struct ExchangeResult {
        #[serde(rename = "Token")]
        token: String,
        #[serde(rename = "RefreshToken")]
        refresh_token: String,
        #[serde(rename = "TokenExpireAt")]
        token_expire_at: i64, // epoch milliseconds
        #[serde(rename = "RefreshExpireAt")]
        refresh_expire_at: i64,
    }

    #[derive(Debug, Deserialize)]
    struct ExchangeResponse {
        #[serde(rename = "Result")]
        result: ExchangeResult,
    }

    struct TokenPair {
        token: String,
        refresh_token: String,
        expired_at: String,
        refresh_expired_at: String,
    }

    /// Call the `ExchangeToken` endpoint to mint a new `access_token` from a
    /// `refresh_token`.
    async fn exchange_token(
        host: String,
        client_id: String,
        refresh_token: &str,
        current_token: &str,
    ) -> Result<TokenPair> {
        let client = reqwest::Client::new();
        let url = format!("{}{}", host, EXCHANGE_TOKEN_PATH);
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-cloudide-token", current_token)
            .json(&serde_json::json!({
                "ClientID": client_id,
                "RefreshToken": refresh_token,
                "ClientSecret": "-",
                "UserID": ""
            }))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "ExchangeToken returned {}: {}",
                status,
                body
            ));
        }
        let data: ExchangeResponse = resp.json().await?;
        let r = data.result;
        Ok(TokenPair {
            token: r.token,
            refresh_token: r.refresh_token,
            expired_at: epoch_ms_to_iso(r.token_expire_at),
            refresh_expired_at: epoch_ms_to_iso(r.refresh_expire_at),
        })
    }

    fn epoch_ms_to_iso(ms: i64) -> String {
        match chrono::DateTime::from_timestamp_millis(ms) {
            Some(dt) => dt.to_rfc3339(),
            None => String::new(),
        }
    }

    // ── Public utility ─────────────────────────────────────────────────────

    /// Obtain the access token + host for a variant. Host is read from the
    /// credentials cache, with a hardcoded fallback.
    pub async fn get_token_and_host(variant: TraeVariant) -> Result<(String, String)> {
        let token = resolve_token(variant).await?;
        // `resolve_token` writes the cache file on success, but a concurrent
        // `logout`, partial write, or corrupted JSON could still leave us
        // unable to load it back. Fall back to the variant's default host
        // instead of panicking on `unwrap()`.
        let host = load_credentials(variant)
            .map(|c| c.host)
            .unwrap_or_else(|| variant.default_host().to_string());
        Ok((token, host))
    }

    /// Decode the JWT payload (second `.`-separated segment) as JSON.
    ///
    /// JWTs use unpadded base64url per RFC 7519. The previous implementation
    /// appended `===` and then stripped all `=` with `trim_end_matches`,
    /// which silently produced bad input for any payload whose length wasn't
    /// already a multiple of 4 — every such token then looked instantly
    /// expired because `exp` / `iat` couldn't be read.
    fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() < 2 {
            return None;
        }
        // Some encoders still emit trailing `=`; URL_SAFE_NO_PAD rejects
        // padding so strip it here.
        let raw = parts[1].trim_end_matches('=');
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(raw)
            .ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Persist a user-pasted JWT (no live expiration check — the expirations
    /// are decoded straight from the JWT payload).
    pub fn save_manual_token(
        variant: TraeVariant,
        token: String,
        host: Option<String>,
    ) -> Result<()> {
        ensure_cache_dir()?;
        // Decode `exp` and `iat` from the JWT payload.
        let (expired_at, refresh_expired_at) = if let Some(payload) = decode_jwt_payload(&token) {
            let exp = payload["exp"].as_i64().map_or_else(
                || (Utc::now() + chrono::Duration::days(14)).to_rfc3339(),
                epoch_secs_to_iso,
            );
            // Estimate `refresh_expired_at` as `iat + 180 days`.
            let iat = payload["iat"].as_i64().map_or_else(
                || (Utc::now() + chrono::Duration::days(180)).to_rfc3339(),
                |ts| {
                    chrono::DateTime::from_timestamp(ts + 180 * 86400, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default()
                },
            );
            (exp, iat)
        } else {
            (String::new(), String::new())
        };

        let host = host.unwrap_or_else(|| variant.default_host().to_string());
        let creds = CachedCredentials {
            variant,
            token,
            refresh_token: String::new(),
            expired_at,
            refresh_expired_at,
            host,
            client_id: variant.default_client_id().to_string(),
            source: TokenSource::Manual,
            user_id: None,
        };
        save_credentials(&creds)
    }

    fn epoch_secs_to_iso(secs: i64) -> String {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    }

    /// Clear the cached credentials for a variant.
    pub fn logout(variant: TraeVariant) -> Result<()> {
        clear_credentials(variant)
    }

    /// Whether the variant has usable credentials (no network calls).
    pub fn has_credentials(variant: TraeVariant) -> bool {
        load_credentials(variant).is_some()
    }

    /// Iterator over all 2 supported variants.
    pub fn all_variants() -> [TraeVariant; 2] {
        [TraeVariant::Solo, TraeVariant::Ide]
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_variant_client_str() {
            assert_eq!(TraeVariant::Solo.client_str(), "trae-solo");
            assert_eq!(TraeVariant::Ide.client_str(), "trae");
        }

        #[test]
        fn test_variant_default_host() {
            assert_eq!(TraeVariant::Solo.default_host(), INTL_HOST);
            assert_eq!(TraeVariant::Ide.default_host(), INTL_HOST);
        }

        #[test]
        fn test_variant_serialize() {
            let solo = serde_json::to_string(&TraeVariant::Solo).unwrap();
            assert_eq!(solo, r#""Solo""#);
            let ide = serde_json::to_string(&TraeVariant::Ide).unwrap();
            assert_eq!(ide, r#""Ide""#);
        }

        #[test]
        fn test_epoch_ms_to_iso() {
            let dt = chrono::DateTime::parse_from_rfc3339("2026-05-21T11:29:04.295Z").unwrap();
            let ms = dt.timestamp_millis();
            let iso = epoch_ms_to_iso(ms);
            assert!(iso.contains("2026-05-21"));
        }

        #[test]
        fn test_all_variants_count() {
            assert_eq!(all_variants().len(), 2);
        }

        fn encode_jwt_payload(payload: &serde_json::Value) -> String {
            let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(b"{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
            let body = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(payload).unwrap());
            let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"sig");
            format!("{header}.{body}.{sig}")
        }

        #[test]
        fn test_decode_jwt_payload_extracts_exp_and_iat() {
            // Payload length 27 → not a multiple of 4. The previous
            // implementation stripped padding and choked on this.
            let payload = serde_json::json!({ "exp": 1900000000_i64, "iat": 1780000000_i64 });
            let token = encode_jwt_payload(&payload);
            let decoded = decode_jwt_payload(&token).expect("decode succeeds");
            assert_eq!(decoded["exp"].as_i64(), Some(1900000000));
            assert_eq!(decoded["iat"].as_i64(), Some(1780000000));
        }

        #[test]
        fn test_decode_jwt_payload_handles_url_safe_chars() {
            // Force a payload that exercises base64url `-` / `_` substitution.
            // `>` and `?` map to `+` / `/` in standard base64; in URL-safe
            // they map to `-` / `_`. Using a string with a known byte that
            // produces `_` in the encoding ensures we don't regress to
            // STANDARD-only decoding.
            let payload = serde_json::json!({ "data": "??>>??" });
            let token = encode_jwt_payload(&payload);
            assert!(token.contains('_') || token.contains('-'));
            let decoded = decode_jwt_payload(&token).expect("decode succeeds");
            assert_eq!(decoded["data"].as_str(), Some("??>>??"));
        }

        #[test]
        fn test_decode_jwt_payload_rejects_malformed_token() {
            assert!(decode_jwt_payload("not-a-jwt").is_none());
            assert!(decode_jwt_payload("").is_none());
            assert!(decode_jwt_payload("badbase64!@#.badbase64!@#").is_none());
        }
    }
}

pub mod safestorage {
    //! Decrypt the `iCubeAuthInfo://*` blobs stored in the Trae desktop
    //! client's `globalStorage/storage.json`.
    //!
    //! The algorithm is a faithful Rust port of the Trae client's
    //! `byteCrypto.js` module (the `V8e()` decrypt path). Field meanings,
    //! constants, and the overall flow are kept identical to the source —
    //! any changes must be re-validated against the byte offsets.
    //!
    //! ## Blob layout
    //!
    //! ```text
    //! [magic 6 bytes: "tc\x05\x10\x00\x00"]
    //! [salt 32 bytes, random]
    //! [ciphertext N×16 bytes, AES-128-CBC + PKCS7 padding]
    //! ```
    //!
    //! ## Key derivation (`BG` function)
    //!
    //! ```text
    //! hardcoded_pw = JG XOR KG    // 64 bytes, hardcoded (obfuscated) in client source
    //! kdf_buf      = SHA-512(salt) || hardcoded_pw   // 128 bytes
    //! kdf_out      = SHA-512(kdf_buf)                // 64 bytes
    //! aes_key      = kdf_out[0..16]
    //! iv           = kdf_out[16..32]
    //! ```
    //!
    //! ## Plaintext integrity check (tail of `V8e`)
    //!
    //! ```text
    //! plaintext = [hash 64 bytes] [data N bytes]
    //! Requires SHA-512(data) == hash; otherwise treated as corrupt.
    //! ```

    use aes::Aes128;
    use anyhow::{anyhow, Context, Result};
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    use sha2::{Digest, Sha512};

    type Aes128CbcDec = cbc::Decryptor<Aes128>;

    const MAGIC: [u8; 6] = [b't', b'c', 0x05, 0x10, 0x00, 0x00];
    const SALT_LEN: usize = 32;
    const HASH_LEN: usize = 64;
    const AES_KEY_LEN: usize = 16;
    const IV_LEN: usize = 16;

    /// `JG` array — extracted from `byteCrypto.js`; the first 64 bytes of the
    /// AES inverse S-box (obfuscation material).
    const JG: [u8; 64] = [
        82, 9, 106, 213, 48, 54, 165, 56, 191, 64, 163, 158, 129, 243, 215, 251, 124, 227, 57, 130,
        155, 47, 255, 135, 52, 142, 67, 68, 196, 222, 233, 203, 84, 123, 148, 50, 166, 194, 35, 61,
        238, 76, 149, 11, 66, 250, 195, 78, 8, 46, 161, 102, 40, 217, 36, 178, 118, 91, 162, 73,
        109, 139, 209, 37,
    ];
    /// `KG` array — extracted from `byteCrypto.js`; the other half of the
    /// hardcoded "password".
    const KG: [u8; 64] = [
        31, 221, 168, 51, 136, 7, 199, 49, 177, 18, 16, 89, 39, 128, 236, 95, 96, 81, 127, 169, 25,
        181, 74, 13, 45, 229, 122, 159, 147, 201, 156, 239, 160, 224, 59, 77, 174, 42, 245, 176,
        200, 235, 187, 60, 131, 83, 153, 97, 23, 43, 4, 126, 186, 119, 214, 38, 225, 105, 20, 99,
        85, 33, 12, 125,
    ];

    fn hardcoded_password() -> [u8; 64] {
        let mut pw = [0u8; 64];
        for i in 0..64 {
            pw[i] = JG[i] ^ KG[i];
        }
        pw
    }

    fn derive_key_iv(salt: &[u8]) -> Result<([u8; AES_KEY_LEN], [u8; IV_LEN])> {
        if salt.len() != SALT_LEN {
            return Err(anyhow!(
                "wrong salt length: expected {}, got {}",
                SALT_LEN,
                salt.len()
            ));
        }
        let sha_salt = Sha512::digest(salt);
        let mut kdf_buf = [0u8; 128];
        kdf_buf[..HASH_LEN].copy_from_slice(&sha_salt);
        kdf_buf[HASH_LEN..].copy_from_slice(&hardcoded_password());
        let kdf_out = Sha512::digest(kdf_buf);
        let mut key = [0u8; AES_KEY_LEN];
        let mut iv = [0u8; IV_LEN];
        key.copy_from_slice(&kdf_out[..AES_KEY_LEN]);
        iv.copy_from_slice(&kdf_out[AES_KEY_LEN..AES_KEY_LEN + IV_LEN]);
        Ok((key, iv))
    }

    /// Decrypt a base64-encoded blob and return the plaintext UTF-8 string
    /// (typically JSON).
    pub fn decrypt_base64_blob(b64: &str) -> Result<String> {
        let raw = B64
            .decode(b64.trim())
            .context("failed to base64-decode iCubeAuthInfo")?;
        let pt = decrypt_blob(&raw)?;
        String::from_utf8(pt).context("decrypted plaintext is not valid UTF-8")
    }

    /// Decrypt a raw-bytes blob and return the plaintext (with the SHA-512
    /// integrity prefix stripped).
    pub fn decrypt_blob(blob: &[u8]) -> Result<Vec<u8>> {
        let min_len = MAGIC.len() + SALT_LEN + 16;
        if blob.len() < min_len {
            return Err(anyhow!(
                "blob too short: {} bytes (need at least {} bytes)",
                blob.len(),
                min_len
            ));
        }
        if blob[..MAGIC.len()] != MAGIC {
            return Err(anyhow!(
                "magic header mismatch: got {:02x?}, expected {:02x?}",
                &blob[..MAGIC.len()],
                MAGIC
            ));
        }
        let salt = &blob[MAGIC.len()..MAGIC.len() + SALT_LEN];
        let ciphertext = &blob[MAGIC.len() + SALT_LEN..];
        if !ciphertext.len().is_multiple_of(16) {
            return Err(anyhow!(
                "ciphertext length {} is not a multiple of the AES block size (16)",
                ciphertext.len()
            ));
        }
        let (key, iv) = derive_key_iv(salt)?;
        let mut buf = ciphertext.to_vec();
        let plaintext_len = Aes128CbcDec::new(&key.into(), &iv.into())
            .decrypt_padded_mut::<Pkcs7>(&mut buf)
            .map_err(|e| anyhow!("AES-CBC decryption failed: {e}"))?
            .len();
        buf.truncate(plaintext_len);
        if buf.len() < HASH_LEN {
            return Err(anyhow!(
                "plaintext too short: {} bytes (need at least {} bytes for the hash prefix)",
                buf.len(),
                HASH_LEN
            ));
        }
        let (expected_hash, data) = buf.split_at(HASH_LEN);
        let actual_hash = Sha512::digest(data);
        if expected_hash != actual_hash.as_slice() {
            return Err(anyhow!("SHA-512 integrity check failed"));
        }
        Ok(data.to_vec())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use aes::Aes128;
        use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};

        type Aes128CbcEnc = cbc::Encryptor<Aes128>;

        /// Encrypt with the same algorithm — used only by tests. Production
        /// blobs are encrypted by the Trae client itself.
        fn encrypt_for_test(plain_text: &[u8]) -> Vec<u8> {
            // Fixed salt so the test is deterministic.
            let salt = [0xAAu8; SALT_LEN];
            let (key, iv) = derive_key_iv(&salt).unwrap();
            // hash || data
            let hash = Sha512::digest(plain_text);
            let mut buf = Vec::with_capacity(HASH_LEN + plain_text.len() + 16);
            buf.extend_from_slice(&hash);
            buf.extend_from_slice(plain_text);
            // PKCS7 padding happens inside `encrypt_padded_mut`; reserve space.
            let unpadded_len = buf.len();
            buf.resize(unpadded_len + 16, 0);
            let ct_len = Aes128CbcEnc::new(&key.into(), &iv.into())
                .encrypt_padded_mut::<Pkcs7>(&mut buf, unpadded_len)
                .unwrap()
                .len();
            buf.truncate(ct_len);
            let mut blob = Vec::with_capacity(MAGIC.len() + SALT_LEN + ct_len);
            blob.extend_from_slice(&MAGIC);
            blob.extend_from_slice(&salt);
            blob.extend_from_slice(&buf);
            blob
        }

        #[test]
        fn test_hardcoded_password_constant() {
            // This value must not change — changing it breaks decryption of
            // every existing Trae client blob.
            let pw = hardcoded_password();
            assert_eq!(pw.len(), 64);
            // First byte = JG[0] ^ KG[0] = 82 ^ 31 = 77 = 0x4d
            assert_eq!(pw[0], 0x4d);
            // Last byte = 37 ^ 125 = 88 = 0x58
            assert_eq!(pw[63], 0x58);
        }

        #[test]
        fn test_round_trip_simple_json() {
            let plain = br#"{"token":"abc","refreshToken":"xyz"}"#;
            let blob = encrypt_for_test(plain);
            let decrypted = decrypt_blob(&blob).expect("decrypt succeeds");
            assert_eq!(&decrypted, plain);
        }

        #[test]
        fn test_round_trip_unicode() {
            let plain = "Hello, world 🌍 — Unicode test".as_bytes();
            let blob = encrypt_for_test(plain);
            let decrypted = decrypt_blob(&blob).expect("decrypt succeeds");
            assert_eq!(&decrypted, plain);
        }

        #[test]
        fn test_decrypt_base64_blob_round_trip() {
            let plain = br#"{"hello":"world"}"#;
            let blob = encrypt_for_test(plain);
            let b64 = B64.encode(&blob);
            let s = decrypt_base64_blob(&b64).expect("decrypt + utf8 ok");
            assert_eq!(s, r#"{"hello":"world"}"#);
        }

        #[test]
        fn test_wrong_magic_rejected() {
            let mut blob = encrypt_for_test(b"test");
            blob[0] = b'x';
            let err = decrypt_blob(&blob).unwrap_err();
            assert!(err.to_string().contains("magic"));
        }

        #[test]
        fn test_too_short_rejected() {
            let blob = vec![0u8; 16];
            let err = decrypt_blob(&blob).unwrap_err();
            assert!(err.to_string().contains("blob too short"));
        }

        #[test]
        fn test_tampered_ciphertext_caught_by_hash_check() {
            let plain = br#"{"token":"abc"}"#;
            let mut blob = encrypt_for_test(plain);
            // Flip a byte in the ciphertext — PKCS7 unpadding may still
            // succeed (depending on the block), but SHA-512 will fail.
            let last = blob.len() - 1;
            blob[last - 16] ^= 0x01;
            let err = decrypt_blob(&blob);
            assert!(err.is_err(), "expected decrypt to fail on tampered blob");
        }

        #[test]
        fn test_ciphertext_not_block_aligned_rejected() {
            // Deliberately construct a blob whose ciphertext is misaligned.
            let mut blob = Vec::new();
            blob.extend_from_slice(&MAGIC);
            blob.extend_from_slice(&[0u8; SALT_LEN]);
            blob.extend_from_slice(&[0u8; 17]); // 17 bytes — not a multiple of 16
            let err = decrypt_blob(&blob).unwrap_err();
            assert!(err.to_string().contains("AES block size"));
        }
    }
}

pub mod sync {
    //! Trae usage sync: paginated pulls from the official API, persisted to a
    //! local cache with a manifest.
    //!
    //! Mirrors the Antigravity (manifest + lock) and Cursor (HTTP API) patterns.
    //!
    //! Trae IDE and Trae Solo share account usage data. The variant only
    //! controls where credentials are discovered; synced usage is stored once
    //! under the single `trae` client cache.

    use super::auth::{self, get_trae_cache_dir, TraeVariant};
    use anyhow::{Context, Result};
    use chrono::Utc;
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;
    use std::time::Duration;

    const PAGE_SIZE: i32 = 20;
    const API_PAGE_DELAY_MS: u64 = 300;
    const OVERLAP_MARGIN_SECS: i64 = 7200; // 2-hour overlap buffer
    const MANIFEST_VERSION: i32 = 1;

    // ── Manifest ───────────────────────────────────────────────────────────

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TraeManifest {
        pub version: i32,
        pub last_synced_at: i64, // epoch seconds
        pub sessions: Vec<TraeSessionEntry>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TraeSessionEntry {
        pub session_id: String,
        pub usage_time: i64, // epoch seconds
        pub artifact_path: String,
    }

    fn manifest_path() -> PathBuf {
        get_trae_cache_dir().join("manifest.json")
    }

    fn sessions_dir() -> PathBuf {
        get_trae_cache_dir().join("sessions")
    }

    fn load_manifest() -> Result<TraeManifest> {
        let path = manifest_path();
        if !path.exists() {
            return Ok(TraeManifest {
                version: MANIFEST_VERSION,
                last_synced_at: 0,
                sessions: Vec::new(),
            });
        }
        let content = std::fs::read_to_string(&path)?;
        serde_json::from_str(&content).context("failed to parse manifest JSON")
    }

    fn save_manifest(manifest: &TraeManifest) -> Result<()> {
        let dir = get_trae_cache_dir();
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }
        let json = serde_json::to_string_pretty(manifest)?;
        std::fs::write(manifest_path(), json)?;
        Ok(())
    }

    fn ensure_sessions_dir() -> Result<PathBuf> {
        let dir = sessions_dir();
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }
        Ok(dir)
    }

    // ── HTTP client ────────────────────────────────────────────────────────

    #[derive(Debug, Deserialize)]
    struct UsageResponse {
        #[serde(rename = "user_usage_group_by_sessions")]
        sessions: Option<Vec<serde_json::Value>>,
        total: Option<i32>,
    }

    /// Paginated call to the usage API; returns the raw JSON session list.
    async fn fetch_usage_pages(
        host: &str,
        token: &str,
        start_time: i64,
        end_time: i64,
        usage_types: &[i32],
    ) -> Result<Vec<serde_json::Value>> {
        let client = reqwest::Client::new();
        let url = format!("{}/trae/api/v1/pay/query_user_usage_group_by_session", host);
        let mut all = Vec::new();
        let mut page = 1;

        loop {
            let payload = serde_json::json!({
                "start_time": start_time,
                "end_time": end_time,
                "page_size": PAGE_SIZE,
                "page_num": page,
                "usage_type": usage_types,
            });

            let resp = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("authorization", format!("Cloud-IDE-JWT {}", token))
                .timeout(Duration::from_secs(30))
                .json(&payload)
                .send()
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("usage API returned {}: {}", status, body));
            }

            let data: UsageResponse = resp.json().await?;
            let sessions = data.sessions.unwrap_or_default();
            let batch = sessions.len();
            all.extend(sessions);

            // When the API omits `total`, keep paginating until an empty
            // page. `unwrap_or(0)` would produce `total=0` which makes
            // `all.len() >= total` immediately true, silently truncating
            // data after the first non-empty page.
            if batch == 0 || data.total.is_some_and(|t| all.len() >= t as usize) {
                break;
            }
            page += 1;
            tokio::time::sleep(Duration::from_millis(API_PAGE_DELAY_MS)).await;
        }

        Ok(all)
    }

    // ── Sync lock ──────────────────────────────────────────────────────────

    const SYNC_LOCK_ACQUIRE_ATTEMPTS: usize = 3;

    #[derive(Debug)]
    struct SyncLockGuard {
        path: PathBuf,
    }

    impl SyncLockGuard {
        fn acquire(cache_dir: &std::path::Path) -> Result<Self> {
            let lock_path = cache_dir.join("sync.lock");
            if !cache_dir.exists() {
                std::fs::create_dir_all(cache_dir)?;
            }
            let mut stale_recoveries = 0usize;
            loop {
                match std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&lock_path)
                {
                    Ok(mut file) => {
                        use std::io::Write;
                        let _ = writeln!(file, "{} {}", std::process::id(), Utc::now().timestamp());
                        return Ok(Self { path: lock_path });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        // Only evict the lock when its owner is provably
                        // dead. Live syncs MUST keep exclusive access for as
                        // long as they hold the PID — otherwise two
                        // processes overlap on the manifest and delete each
                        // other's session artifacts.
                        if let Some((existing_pid, _)) = read_sync_lock(&lock_path) {
                            if pid_is_alive(existing_pid) {
                                return Err(anyhow::anyhow!(
                                    "another trae sync is in progress (pid {existing_pid}); aborting"
                                ));
                            }
                        }
                        if stale_recoveries >= SYNC_LOCK_ACQUIRE_ATTEMPTS {
                            return Err(anyhow::anyhow!(
                                "could not acquire trae sync lock after {SYNC_LOCK_ACQUIRE_ATTEMPTS} stale-lock recoveries; another process keeps recreating the lock file"
                            ));
                        }
                        stale_recoveries += 1;
                        let _ = std::fs::remove_file(&lock_path);
                        continue;
                    }
                    Err(e) => {
                        return Err(anyhow::Error::new(e).context("failed to acquire sync lock"));
                    }
                }
            }
        }
    }

    impl Drop for SyncLockGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn read_sync_lock(path: &std::path::Path) -> Option<(u32, u64)> {
        let contents = std::fs::read_to_string(path).ok()?;
        let mut parts = contents.split_whitespace();
        let pid = parts.next()?.parse::<u32>().ok()?;
        let timestamp = parts.next()?.parse::<u64>().ok()?;
        Some((pid, timestamp))
    }

    fn pid_is_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        #[cfg(unix)]
        {
            // `kill(pid, 0)` is a signal-free liveness probe. EPERM (errno
            // 1) still means the process exists, just that we lack
            // permission to signal it.
            let result = unsafe { libc_kill(pid as i32, 0) };
            result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(1)
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            // No portable PID probe available. Treat the lock as stale so a
            // crashed previous run doesn't permanently block subsequent
            // syncs. This matches the policy used by Antigravity sync and
            // accepts a small concurrent-corruption risk on Windows; that
            // risk is acceptable because tokscale is a single-user CLI and
            // overlapping syncs are rare in practice.
            false
        }
    }

    #[cfg(unix)]
    extern "C" {
        #[link_name = "kill"]
        fn libc_kill(pid: i32, sig: i32) -> i32;
    }

    // ── Main sync logic ────────────────────────────────────────────────────

    /// Run a single incremental sync for one variant.
    pub async fn sync_variant(
        variant: TraeVariant,
        since_days: i64,
        usage_types: &[i32],
    ) -> Result<usize> {
        let _lock = SyncLockGuard::acquire(&get_trae_cache_dir())?;

        let (token, host) = auth::get_token_and_host(variant)
            .await
            .context("failed to obtain Trae access token")?;

        let now = Utc::now().timestamp();
        let manifest = load_manifest()?;
        // Incremental start point: take the earlier of (user-requested, manifest
        // record) so we never lose data across runs.
        let since_user = now - since_days * 86400;
        let since_manifest = if manifest.last_synced_at > 0 {
            manifest.last_synced_at - OVERLAP_MARGIN_SECS
        } else {
            0
        };
        let start_time = if since_manifest > 0 {
            since_user.min(since_manifest)
        } else {
            since_user
        };
        let end_time = now;

        let sessions = fetch_usage_pages(&host, &token, start_time, end_time, usage_types).await?;

        if sessions.is_empty() {
            return Ok(0);
        }

        let dir = ensure_sessions_dir()?;
        let mut next_manifest = TraeManifest {
            version: MANIFEST_VERSION,
            last_synced_at: now,
            sessions: manifest.sessions.clone(),
        };

        // Write the whole batch to a single JSON file.
        let batch_ts = Utc::now().format("%Y%m%dT%H%M%S").to_string();
        let artifact_filename = format!("usage-{}.json", batch_ts);
        let artifact_path = dir.join(&artifact_filename);
        let json = serde_json::to_string_pretty(&sessions)?;
        std::fs::write(&artifact_path, json)?;

        // Merge new sessions into the manifest.
        let mut existing_ids: std::collections::HashSet<String> = next_manifest
            .sessions
            .iter()
            .map(|e| e.session_id.clone())
            .collect();

        for s in &sessions {
            let sid = s["session_id"].as_str().unwrap_or("?").to_string();
            let ut = s["usage_time"].as_i64().unwrap_or(0);
            if existing_ids.insert(sid.clone()) {
                next_manifest.sessions.push(TraeSessionEntry {
                    session_id: sid,
                    usage_time: ut,
                    artifact_path: format!("sessions/{}", artifact_filename),
                });
            }
        }

        // Garbage-collect old artifacts not referenced by the manifest.
        next_manifest.sessions.sort_by_key(|e| e.usage_time);
        next_manifest
            .sessions
            .dedup_by_key(|e| e.session_id.clone());

        let valid_paths: std::collections::HashSet<String> = next_manifest
            .sessions
            .iter()
            .map(|e| e.artifact_path.clone())
            .collect();

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    let rel = format!("sessions/{}", name);
                    if !valid_paths.contains(&rel) && name.ends_with(".json") {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }

        save_manifest(&next_manifest)?;

        Ok(sessions.len())
    }

    /// CLI entry point: sync Trae usage once using a selected credential source.
    ///
    /// `variants` comes from CLI flags. Because Trae IDE and Trae Solo return
    /// the same account-level usage data, only one variant needs to succeed.
    /// We iterate through every variant with credentials and stop at the first
    /// success; if a variant fails (expired refresh token, unreadable cache,
    /// transient HTTP error), we fall over to the next one before giving up.
    pub async fn run_trae_sync(
        variants: &[TraeVariant],
        since_days: i64,
        include_aux: bool,
    ) -> Result<()> {
        let usage_types: Vec<i32> = if include_aux {
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        } else {
            vec![5, 6]
        };

        let credentialed: Vec<TraeVariant> = variants
            .iter()
            .copied()
            .filter(|v| auth::has_credentials(*v))
            .collect();

        if credentialed.is_empty() {
            if variants.is_empty() {
                println!("  No Trae credentials found. Run `tokscale trae login` first.");
            } else {
                for variant in variants {
                    println!(
                        "  Trae {}: no credentials — run `tokscale trae login --variant {}` first",
                        variant.client_str(),
                        variant.cli_arg()
                    );
                }
            }
            return Ok(());
        }

        let mut last_err: Option<anyhow::Error> = None;
        for variant in &credentialed {
            match sync_variant(*variant, since_days, &usage_types).await {
                Ok(n) => {
                    println!(
                        "  Trae: synced {n} sessions (using {} credentials)",
                        variant.client_str()
                    );
                    return Ok(());
                }
                Err(e) => {
                    eprintln!(
                        "  Trae sync failed using {} credentials: {e}",
                        variant.client_str()
                    );
                    last_err = Some(e);
                }
            }
        }

        // Every credentialed variant has been tried and failed. Surface the
        // last error so the user sees a non-zero exit instead of a silent
        // success after the per-variant `eprintln!` lines scroll off.
        match last_err {
            Some(e) => Err(e.context("all Trae credential sources failed")),
            None => Ok(()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_read_sync_lock_parses_pid_and_timestamp() {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("sync.lock");
            std::fs::write(&path, "12345 1776000000\n").unwrap();
            let (pid, ts) = read_sync_lock(&path).expect("readable");
            assert_eq!(pid, 12345);
            assert_eq!(ts, 1776000000);
        }

        #[test]
        fn test_read_sync_lock_returns_none_on_malformed() {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("sync.lock");
            std::fs::write(&path, "not a pid\n").unwrap();
            assert!(read_sync_lock(&path).is_none());
        }

        #[test]
        fn test_pid_is_alive_zero_is_dead() {
            assert!(!pid_is_alive(0));
        }

        #[test]
        #[cfg(unix)]
        fn test_pid_is_alive_current_process_is_alive() {
            let me = std::process::id();
            assert!(pid_is_alive(me));
        }

        #[test]
        fn test_acquire_recovers_from_stale_lock_with_dead_pid() {
            // PID 0 is reserved by the kernel and never represents a live
            // user-space process — perfect stand-in for a crashed sync.
            let tmp = tempfile::tempdir().unwrap();
            let cache_dir = tmp.path();
            std::fs::write(cache_dir.join("sync.lock"), "0 1\n").unwrap();
            // Should evict the stale lock and acquire a fresh one.
            let guard = SyncLockGuard::acquire(cache_dir).expect("stale lock is recovered");
            assert!(cache_dir.join("sync.lock").exists());
            drop(guard);
            // Drop releases the lock.
            assert!(!cache_dir.join("sync.lock").exists());
        }

        #[test]
        #[cfg(unix)]
        fn test_acquire_rejects_when_owner_is_alive() {
            // Use our own PID — guaranteed alive. acquire() must refuse.
            let tmp = tempfile::tempdir().unwrap();
            let cache_dir = tmp.path();
            let alive_pid = std::process::id();
            std::fs::write(cache_dir.join("sync.lock"), format!("{alive_pid} 1\n")).unwrap();
            let err = SyncLockGuard::acquire(cache_dir).unwrap_err();
            assert!(err.to_string().contains("another trae sync is in progress"));
            // Lock file must remain untouched so the live owner can release it.
            assert!(cache_dir.join("sync.lock").exists());
        }

        #[test]
        fn test_acquire_writes_pid_and_timestamp() {
            let tmp = tempfile::tempdir().unwrap();
            let cache_dir = tmp.path();
            let guard = SyncLockGuard::acquire(cache_dir).expect("first acquire");
            let (pid, _) = read_sync_lock(&cache_dir.join("sync.lock")).expect("readable");
            assert_eq!(pid, std::process::id());
            drop(guard);
        }
    }
}
