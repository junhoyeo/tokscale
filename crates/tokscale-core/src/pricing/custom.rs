use super::litellm::ModelPricing;
use crate::sessions::synthetic::normalize_synthetic_model;
use serde::de::{MapAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const CUSTOM_PRICING_FILENAME: &str = "custom-pricing.json";
const TOKENS_PER_MILLION: f64 = 1_000_000.0;
const MAX_CUSTOM_PRICING_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CUSTOM_PRICING_MODEL_CAPACITY: usize = 10_000;

#[derive(Clone, Default)]
pub struct CustomPricing {
    models: HashMap<String, ModelPricing>,
}

pub struct CustomLookupResult<'a> {
    pub matched_key: &'a str,
    pub pricing: &'a ModelPricing,
}

#[derive(Deserialize)]
struct RawCustomPricingFile {
    #[serde(default, deserialize_with = "deserialize_models")]
    models: Vec<(String, Value)>,
}

#[derive(Deserialize)]
struct CustomModelPricing {
    input_cost_per_million_tokens: Option<f64>,
    input_cost_per_million_tokens_above_128k_tokens: Option<f64>,
    input_cost_per_million_tokens_above_200k_tokens: Option<f64>,
    input_cost_per_million_tokens_above_256k_tokens: Option<f64>,
    input_cost_per_million_tokens_above_272k_tokens: Option<f64>,
    input_cost_per_token: Option<f64>,
    input_cost_per_token_above_128k_tokens: Option<f64>,
    input_cost_per_token_above_200k_tokens: Option<f64>,
    input_cost_per_token_above_256k_tokens: Option<f64>,
    input_cost_per_token_above_272k_tokens: Option<f64>,
    output_cost_per_million_tokens: Option<f64>,
    output_cost_per_million_tokens_above_128k_tokens: Option<f64>,
    output_cost_per_million_tokens_above_200k_tokens: Option<f64>,
    output_cost_per_million_tokens_above_256k_tokens: Option<f64>,
    output_cost_per_million_tokens_above_272k_tokens: Option<f64>,
    output_cost_per_token: Option<f64>,
    output_cost_per_token_above_128k_tokens: Option<f64>,
    output_cost_per_token_above_200k_tokens: Option<f64>,
    output_cost_per_token_above_256k_tokens: Option<f64>,
    output_cost_per_token_above_272k_tokens: Option<f64>,
    cache_creation_input_token_cost_per_million_tokens: Option<f64>,
    cache_creation_input_token_cost_per_million_tokens_above_200k_tokens: Option<f64>,
    cache_creation_input_token_cost: Option<f64>,
    cache_creation_input_token_cost_above_200k_tokens: Option<f64>,
    cache_read_input_token_cost_per_million_tokens: Option<f64>,
    cache_read_input_token_cost_per_million_tokens_above_200k_tokens: Option<f64>,
    cache_read_input_token_cost_per_million_tokens_above_272k_tokens: Option<f64>,
    cache_read_input_token_cost: Option<f64>,
    cache_read_input_token_cost_above_200k_tokens: Option<f64>,
    cache_read_input_token_cost_above_272k_tokens: Option<f64>,
}

impl CustomModelPricing {
    fn into_model_pricing(self) -> Result<ModelPricing, String> {
        let input_cost_per_token = base_price(
            self.input_cost_per_million_tokens,
            self.input_cost_per_token,
            "input_cost_per_million_tokens",
            "input_cost_per_token",
        )?;
        let output_cost_per_token = base_price(
            self.output_cost_per_million_tokens,
            self.output_cost_per_token,
            "output_cost_per_million_tokens",
            "output_cost_per_token",
        )?;

        if !input_cost_per_token.is_some_and(|value| value > 0.0)
            && !output_cost_per_token.is_some_and(|value| value > 0.0)
        {
            return Err(
                "at least one of input or output pricing must be present and positive".into(),
            );
        }

        Ok(ModelPricing {
            input_cost_per_token,
            input_cost_per_token_above_128k_tokens: price_field(
                self.input_cost_per_million_tokens_above_128k_tokens,
                self.input_cost_per_token_above_128k_tokens,
                "input_cost_per_million_tokens_above_128k_tokens",
                "input_cost_per_token_above_128k_tokens",
            )?,
            input_cost_per_token_above_200k_tokens: price_field(
                self.input_cost_per_million_tokens_above_200k_tokens,
                self.input_cost_per_token_above_200k_tokens,
                "input_cost_per_million_tokens_above_200k_tokens",
                "input_cost_per_token_above_200k_tokens",
            )?,
            input_cost_per_token_above_256k_tokens: price_field(
                self.input_cost_per_million_tokens_above_256k_tokens,
                self.input_cost_per_token_above_256k_tokens,
                "input_cost_per_million_tokens_above_256k_tokens",
                "input_cost_per_token_above_256k_tokens",
            )?,
            input_cost_per_token_above_272k_tokens: price_field(
                self.input_cost_per_million_tokens_above_272k_tokens,
                self.input_cost_per_token_above_272k_tokens,
                "input_cost_per_million_tokens_above_272k_tokens",
                "input_cost_per_token_above_272k_tokens",
            )?,
            output_cost_per_token,
            output_cost_per_token_above_128k_tokens: price_field(
                self.output_cost_per_million_tokens_above_128k_tokens,
                self.output_cost_per_token_above_128k_tokens,
                "output_cost_per_million_tokens_above_128k_tokens",
                "output_cost_per_token_above_128k_tokens",
            )?,
            output_cost_per_token_above_200k_tokens: price_field(
                self.output_cost_per_million_tokens_above_200k_tokens,
                self.output_cost_per_token_above_200k_tokens,
                "output_cost_per_million_tokens_above_200k_tokens",
                "output_cost_per_token_above_200k_tokens",
            )?,
            output_cost_per_token_above_256k_tokens: price_field(
                self.output_cost_per_million_tokens_above_256k_tokens,
                self.output_cost_per_token_above_256k_tokens,
                "output_cost_per_million_tokens_above_256k_tokens",
                "output_cost_per_token_above_256k_tokens",
            )?,
            output_cost_per_token_above_272k_tokens: price_field(
                self.output_cost_per_million_tokens_above_272k_tokens,
                self.output_cost_per_token_above_272k_tokens,
                "output_cost_per_million_tokens_above_272k_tokens",
                "output_cost_per_token_above_272k_tokens",
            )?,
            cache_creation_input_token_cost: price_field(
                self.cache_creation_input_token_cost_per_million_tokens,
                self.cache_creation_input_token_cost,
                "cache_creation_input_token_cost_per_million_tokens",
                "cache_creation_input_token_cost",
            )?,
            cache_creation_input_token_cost_above_200k_tokens: price_field(
                self.cache_creation_input_token_cost_per_million_tokens_above_200k_tokens,
                self.cache_creation_input_token_cost_above_200k_tokens,
                "cache_creation_input_token_cost_per_million_tokens_above_200k_tokens",
                "cache_creation_input_token_cost_above_200k_tokens",
            )?,
            cache_read_input_token_cost: price_field(
                self.cache_read_input_token_cost_per_million_tokens,
                self.cache_read_input_token_cost,
                "cache_read_input_token_cost_per_million_tokens",
                "cache_read_input_token_cost",
            )?,
            cache_read_input_token_cost_above_200k_tokens: price_field(
                self.cache_read_input_token_cost_per_million_tokens_above_200k_tokens,
                self.cache_read_input_token_cost_above_200k_tokens,
                "cache_read_input_token_cost_per_million_tokens_above_200k_tokens",
                "cache_read_input_token_cost_above_200k_tokens",
            )?,
            cache_read_input_token_cost_above_272k_tokens: price_field(
                self.cache_read_input_token_cost_per_million_tokens_above_272k_tokens,
                self.cache_read_input_token_cost_above_272k_tokens,
                "cache_read_input_token_cost_per_million_tokens_above_272k_tokens",
                "cache_read_input_token_cost_above_272k_tokens",
            )?,
        })
    }
}

impl CustomPricing {
    pub fn default_path() -> PathBuf {
        crate::paths::get_config_dir().join(CUSTOM_PRICING_FILENAME)
    }

    pub fn load_from_default_path() -> Self {
        Self::load_from_path(&Self::default_path())
    }

    pub fn load_from_path(path: &Path) -> Self {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                warn_custom_pricing(path, format_args!("failed to stat file: {err}"));
                return Self::default();
            }
        };

        if metadata.len() > MAX_CUSTOM_PRICING_FILE_BYTES {
            warn_custom_pricing(
                path,
                format_args!(
                    "file is too large ({} bytes; max {} bytes)",
                    metadata.len(),
                    MAX_CUSTOM_PRICING_FILE_BYTES
                ),
            );
            return Self::default();
        }

        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                warn_custom_pricing(path, format_args!("failed to read file: {err}"));
                return Self::default();
            }
        };

        Self::load_from_str(&content, path)
    }

    pub fn from_models(models: HashMap<String, ModelPricing>) -> Self {
        let mut normalized =
            HashMap::with_capacity(models.len().min(MAX_CUSTOM_PRICING_MODEL_CAPACITY));
        for (key, pricing) in models {
            normalized.insert(key.to_lowercase(), pricing);
        }
        Self { models: normalized }
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &ModelPricing)> {
        self.models
            .iter()
            .map(|(model_id, pricing)| (model_id.as_str(), pricing))
    }

    pub fn lookup(&self, model_id: &str) -> Option<&ModelPricing> {
        self.lookup_with_key(model_id).map(|result| result.pricing)
    }

    pub fn lookup_with_key(&self, model_id: &str) -> Option<CustomLookupResult<'_>> {
        let raw_key = model_id.to_lowercase();
        if let Some(pricing) = self.models.get_key_value(&raw_key) {
            return Some(CustomLookupResult {
                matched_key: pricing.0,
                pricing: pricing.1,
            });
        }

        let normalized_key = normalize_synthetic_model(model_id).to_lowercase();
        if normalized_key != raw_key {
            if let Some(pricing) = self.models.get_key_value(&normalized_key) {
                return Some(CustomLookupResult {
                    matched_key: pricing.0,
                    pricing: pricing.1,
                });
            }
        }

        None
    }

    fn load_from_str(content: &str, path: &Path) -> Self {
        let raw: RawCustomPricingFile = match serde_json::from_str(content) {
            Ok(raw) => raw,
            Err(err) => {
                warn_custom_pricing(path, format_args!("failed to parse JSON: {err}"));
                return Self::default();
            }
        };

        let mut models =
            HashMap::with_capacity(raw.models.len().min(MAX_CUSTOM_PRICING_MODEL_CAPACITY));
        for (model_id, value) in raw.models {
            let lower_key = model_id.to_lowercase();
            let entry: CustomModelPricing = match serde_json::from_value(value) {
                Ok(entry) => entry,
                Err(err) => {
                    warn_custom_pricing(
                        path,
                        format_args!("skipping {model_id}: malformed pricing entry: {err}"),
                    );
                    continue;
                }
            };
            let pricing = match entry.into_model_pricing() {
                Ok(pricing) => pricing,
                Err(err) => {
                    warn_custom_pricing(path, format_args!("skipping {model_id}: {err}"));
                    continue;
                }
            };

            if models.insert(lower_key.clone(), pricing).is_some() {
                warn_custom_pricing(
                    path,
                    format_args!(
                        "duplicate model key after lowercasing, last entry wins: {lower_key}"
                    ),
                );
            }
        }

        Self { models }
    }
}

fn base_price(
    per_million: Option<f64>,
    per_token: Option<f64>,
    per_million_field: &str,
    per_token_field: &str,
) -> Result<Option<f64>, String> {
    price_field(per_million, per_token, per_million_field, per_token_field)
}

fn price_field(
    per_million: Option<f64>,
    per_token: Option<f64>,
    per_million_field: &str,
    per_token_field: &str,
) -> Result<Option<f64>, String> {
    match (per_million, per_token) {
        (Some(_), Some(_)) => Err(format!(
            "{per_million_field} and {per_token_field} cannot both be set"
        )),
        (Some(value), None) => validate_non_negative(value, per_million_field).map(to_per_token),
        (None, Some(value)) => validate_non_negative(value, per_token_field),
        (None, None) => Ok(None),
    }
}

fn validate_non_negative(value: f64, field: &str) -> Result<Option<f64>, String> {
    if value.is_finite() && value >= 0.0 {
        Ok(Some(value))
    } else {
        Err(format!("{field} must be non-negative and finite"))
    }
}

fn to_per_token(per_million: Option<f64>) -> Option<f64> {
    let per_million = per_million?;
    Some(per_million / TOKENS_PER_MILLION)
}

fn warn_custom_pricing(path: &Path, message: fmt::Arguments<'_>) {
    eprintln!(
        "[tokscale] Warning: custom pricing {}: {message}",
        path.display()
    );
}

fn deserialize_models<'de, D>(deserializer: D) -> Result<Vec<(String, Value)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ModelsVisitor;

    impl<'de> Visitor<'de> for ModelsVisitor {
        type Value = Vec<(String, Value)>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of model ids to pricing entries")
        }

        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut entries = Vec::with_capacity(
                access
                    .size_hint()
                    .unwrap_or(0)
                    .min(MAX_CUSTOM_PRICING_MODEL_CAPACITY),
            );
            while let Some((key, value)) = access.next_entry::<String, Value>()? {
                entries.push((key, value));
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(ModelsVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn pricing(input: f64, output: f64) -> ModelPricing {
        ModelPricing {
            input_cost_per_token: Some(input),
            output_cost_per_token: Some(output),
            ..Default::default()
        }
    }

    #[test]
    fn loads_empty_when_file_missing() {
        let temp = TempDir::new().unwrap();
        let loaded = CustomPricing::load_from_path(&temp.path().join("missing.json"));

        assert!(loaded.lookup("anything").is_none());
    }

    #[test]
    fn loads_valid_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "$schema": "https://tokscale.ai/custom-pricing.schema.json",
                "models": {
                        "accounts/fireworks/routers/kimi-k2p6-turbo": {
                        "input_cost_per_million_tokens": 2.00,
                        "output_cost_per_million_tokens": 8.00,
                        "cache_read_input_token_cost_per_million_tokens": 0.30,
                        "source": "https://docs.fireworks.ai/serverless/pricing",
                        "notes": "Fireworks Kimi K2.6 Turbo"
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);
        let pricing = loaded
            .lookup("accounts/fireworks/routers/kimi-k2p6-turbo")
            .unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(pricing.input_cost_per_token, Some(0.000002));
        assert_eq!(pricing.output_cost_per_token, Some(0.000008));
        assert_eq!(pricing.cache_read_input_token_cost, Some(0.0000003));
    }

    #[test]
    fn loads_litellm_per_token_fields() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "copy-pasted": {
                        "input_cost_per_token": 0.000002,
                        "output_cost_per_token": 0.000008,
                        "cache_read_input_token_cost": 0.0000003,
                        "source": "copied from LiteLLM-shaped JSON"
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);
        let pricing = loaded.lookup("copy-pasted").unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(0.000002));
        assert_eq!(pricing.output_cost_per_token, Some(0.000008));
        assert_eq!(pricing.cache_read_input_token_cost, Some(0.0000003));
    }

    #[test]
    fn loads_mixed_per_million_and_litellm_per_token_entries() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "per-million": {
                        "input_cost_per_million_tokens": 2.00,
                        "output_cost_per_million_tokens": 8.00
                    },
                    "per-token": {
                        "input_cost_per_token": 0.00000095,
                        "output_cost_per_token": 0.000004
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert_eq!(
            loaded.lookup("per-million").unwrap().input_cost_per_token,
            Some(0.000002)
        );
        assert_eq!(
            loaded.lookup("per-token").unwrap().input_cost_per_token,
            Some(0.00000095)
        );
    }

    #[test]
    fn drops_entry_when_per_million_and_per_token_alias_both_set() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "ambiguous": {
                        "input_cost_per_million_tokens": 2.00,
                        "input_cost_per_token": 0.000002,
                        "output_cost_per_million_tokens": 8.00
                    },
                    "good": {
                        "input_cost_per_million_tokens": 1.00,
                        "output_cost_per_million_tokens": 4.00
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert!(loaded.lookup("ambiguous").is_none());
        assert_eq!(
            loaded.lookup("good").unwrap().input_cost_per_token,
            Some(0.000001)
        );
    }

    #[test]
    fn rejects_out_of_range_json_number_before_loading_entries() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "too-large": {
                        "input_cost_per_million_tokens": 1e500,
                        "output_cost_per_million_tokens": 8.00
                    },
                    "good": {
                        "input_cost_per_million_tokens": 2.00,
                        "output_cost_per_million_tokens": 8.00
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert!(loaded.is_empty());
    }

    #[test]
    fn tolerates_malformed_json() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(&path, r#"{"models": {"#).unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert!(loaded.is_empty());
        assert!(loaded.lookup("model").is_none());
    }

    #[test]
    fn tolerates_malformed_entry_keeps_others() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "bad": {
                        "input_cost_per_million_tokens": "not-a-number",
                        "output_cost_per_million_tokens": 8.00
                    },
                    "good": {
                        "input_cost_per_million_tokens": 2.00,
                        "output_cost_per_million_tokens": 8.00
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert!(loaded.lookup("bad").is_none());
        assert_eq!(
            loaded.lookup("good").unwrap().input_cost_per_token,
            Some(0.000002)
        );
    }

    #[test]
    fn keeps_entries_with_input_or_output_price() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "missing-output": {
                        "input_cost_per_million_tokens": 2.00
                    },
                    "missing-input": {
                        "output_cost_per_million_tokens": 8.00
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert_eq!(
            loaded
                .lookup("missing-output")
                .unwrap()
                .input_cost_per_token,
            Some(0.000002)
        );
        assert_eq!(
            loaded
                .lookup("missing-input")
                .unwrap()
                .output_cost_per_token,
            Some(0.000008)
        );
    }

    #[test]
    fn drops_entries_with_no_input_or_output_prices() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "cache-only": {
                        "cache_read_input_token_cost_per_million_tokens": 0.30
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert!(loaded.lookup("cache-only").is_none());
    }

    #[test]
    fn drops_entries_with_zero_prices() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "all-zero": {
                        "input_cost_per_million_tokens": 0.0,
                        "output_cost_per_million_tokens": 0.0
                    },
                    "free-input": {
                        "input_cost_per_million_tokens": 0.0,
                        "output_cost_per_million_tokens": 8.00
                    },
                    "negative-output": {
                        "input_cost_per_million_tokens": 2.00,
                        "output_cost_per_million_tokens": -8.00
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert!(loaded.lookup("all-zero").is_none());
        assert_eq!(
            loaded.lookup("free-input").unwrap().output_cost_per_token,
            Some(0.000008)
        );
        assert!(loaded.lookup("negative-output").is_none());
    }

    #[test]
    fn rejects_non_finite_prices() {
        assert!(validate_non_negative(f64::NAN, "input").is_err());
        assert!(validate_non_negative(f64::INFINITY, "input").is_err());
        assert!(validate_non_negative(f64::NEG_INFINITY, "input").is_err());
    }

    #[test]
    fn ignores_unknown_bookkeeping_fields() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "annotated": {
                        "input_cost_per_million_tokens": 2.00,
                        "output_cost_per_million_tokens": 8.00,
                        "source": "https://example.com/pricing",
                        "notes": "kept for the user, ignored by tokscale"
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert_eq!(
            loaded.lookup("annotated").unwrap().input_cost_per_token,
            Some(0.000002)
        );
    }

    #[test]
    fn drops_oversized_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_CUSTOM_PRICING_FILE_BYTES + 1).unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert!(loaded.is_empty());
    }

    #[test]
    fn case_insensitive_lookup() {
        let mut models = HashMap::new();
        models.insert("MiXeD-Model".to_string(), pricing(0.000002, 0.000008));
        let loaded = CustomPricing::from_models(models);

        assert_eq!(
            loaded.lookup("mixed-model").unwrap().input_cost_per_token,
            Some(0.000002)
        );
        assert_eq!(
            loaded.lookup("MIXED-MODEL").unwrap().output_cost_per_token,
            Some(0.000008)
        );
    }

    #[test]
    fn duplicate_keys_last_wins() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "Model-A": {
                        "input_cost_per_million_tokens": 1.00,
                        "output_cost_per_million_tokens": 4.00
                    },
                    "model-a": {
                        "input_cost_per_million_tokens": 2.00,
                        "output_cost_per_million_tokens": 8.00
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert_eq!(
            loaded.lookup("MODEL-A").unwrap().input_cost_per_token,
            Some(0.000002)
        );
    }

    #[test]
    fn literal_duplicate_keys_last_wins() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "model-a": {
                        "input_cost_per_million_tokens": 1.00,
                        "output_cost_per_million_tokens": 4.00
                    },
                    "model-a": {
                        "input_cost_per_million_tokens": 2.00,
                        "output_cost_per_million_tokens": 8.00
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        assert_eq!(
            loaded.lookup("model-a").unwrap().input_cost_per_token,
            Some(0.000002)
        );
    }
}
