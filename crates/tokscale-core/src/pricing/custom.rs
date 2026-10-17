use super::litellm::ModelPricing;
use crate::sessions::synthetic::normalize_synthetic_model;
use serde::de::{MapAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
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
    providers: HashMap<String, HashMap<String, ModelPricing>>,
    diagnostics: Vec<CustomPricingDiagnostic>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CustomPricingDiagnostic {
    pub file: String,
    pub code: String,
    pub path: String,
    pub message: String,
}

pub struct CustomLookupResult<'a> {
    pub matched_key: &'a str,
    pub pricing: &'a ModelPricing,
    pub matched_provider_id: Option<&'a str>,
    /// Whether `matched_key` was reached through synthetic-model
    /// normalization rather than the requested id. Provenance reports this,
    /// so a `hf:zai-org/GLM-4.7` usage row priced by a `glm-4.7` custom entry
    /// says which key actually matched.
    pub normalized: bool,
}

#[derive(Deserialize)]
struct RawCustomPricingFile {
    #[serde(default, deserialize_with = "deserialize_models")]
    models: Vec<(String, Value)>,
    #[serde(default)]
    provider_overrides: Vec<Value>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
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
    cache_creation_input_token_cost_per_million_tokens_above_1hr: Option<f64>,
    cache_creation_input_token_cost_per_million_tokens_above_1hr_above_200k_tokens: Option<f64>,
    cache_creation_input_token_cost_above_1hr: Option<f64>,
    cache_creation_input_token_cost_above_1hr_above_200k_tokens: Option<f64>,
    cache_read_input_token_cost_per_million_tokens: Option<f64>,
    cache_read_input_token_cost_per_million_tokens_above_200k_tokens: Option<f64>,
    cache_read_input_token_cost_per_million_tokens_above_272k_tokens: Option<f64>,
    cache_read_input_token_cost: Option<f64>,
    cache_read_input_token_cost_above_200k_tokens: Option<f64>,
    cache_read_input_token_cost_above_272k_tokens: Option<f64>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

const CUSTOM_RATE_FIELDS: &[&str] = &[
    "input_cost_per_million_tokens",
    "input_cost_per_million_tokens_above_128k_tokens",
    "input_cost_per_million_tokens_above_200k_tokens",
    "input_cost_per_million_tokens_above_256k_tokens",
    "input_cost_per_million_tokens_above_272k_tokens",
    "input_cost_per_token",
    "input_cost_per_token_above_128k_tokens",
    "input_cost_per_token_above_200k_tokens",
    "input_cost_per_token_above_256k_tokens",
    "input_cost_per_token_above_272k_tokens",
    "output_cost_per_million_tokens",
    "output_cost_per_million_tokens_above_128k_tokens",
    "output_cost_per_million_tokens_above_200k_tokens",
    "output_cost_per_million_tokens_above_256k_tokens",
    "output_cost_per_million_tokens_above_272k_tokens",
    "output_cost_per_token",
    "output_cost_per_token_above_128k_tokens",
    "output_cost_per_token_above_200k_tokens",
    "output_cost_per_token_above_256k_tokens",
    "output_cost_per_token_above_272k_tokens",
    "cache_creation_input_token_cost_per_million_tokens",
    "cache_creation_input_token_cost_per_million_tokens_above_200k_tokens",
    "cache_creation_input_token_cost",
    "cache_creation_input_token_cost_above_200k_tokens",
    "cache_creation_input_token_cost_per_million_tokens_above_1hr",
    "cache_creation_input_token_cost_per_million_tokens_above_1hr_above_200k_tokens",
    "cache_creation_input_token_cost_above_1hr",
    "cache_creation_input_token_cost_above_1hr_above_200k_tokens",
    "cache_read_input_token_cost_per_million_tokens",
    "cache_read_input_token_cost_per_million_tokens_above_200k_tokens",
    "cache_read_input_token_cost_per_million_tokens_above_272k_tokens",
    "cache_read_input_token_cost",
    "cache_read_input_token_cost_above_200k_tokens",
    "cache_read_input_token_cost_above_272k_tokens",
];

impl CustomModelPricing {
    fn suspicious_unknown_rate_field(&self) -> Option<&str> {
        self.extra
            .keys()
            .filter(|field| {
                field
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .any(|part| {
                        matches!(
                            part.to_ascii_lowercase().as_str(),
                            "cost" | "price" | "pricing" | "rate" | "rates"
                        )
                    })
            })
            .map(String::as_str)
            .min()
    }

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

        // @keep: requiring a POSITIVE rate here locked users out of the one
        // escape hatch they have for free models.
        //
        // A rate of 0.0 is a statement — "this model is free" — and it is
        // exactly what a user needs to express for a free tier that no upstream
        // dataset publishes. Demanding a positive rate made that unsayable, so
        // free usage stayed unpriced and was excluded from submission with no
        // way to correct it (#1021).
        //
        // Absence is still rejected: a row that names no rate at all says
        // nothing, and silently reading that as free would invent a $0 total
        // for genuinely unknown pricing.
        if input_cost_per_token.is_none() && output_cost_per_token.is_none() {
            return Err("at least one of input or output pricing must be present".into());
        }

        Ok(ModelPricing {
            input_cost_per_token,
            // The upstream 100K fields have provider-specific request-wide
            // semantics. Custom pricing keeps its existing generic tier schema
            // instead of silently opting users into Anthropic's policy.
            input_cost_per_token_above_100k_tokens: None,
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
            output_cost_per_token_above_100k_tokens: None,
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
            cache_creation_input_token_cost_above_100k_tokens: None,
            cache_creation_input_token_cost_above_200k_tokens: price_field(
                self.cache_creation_input_token_cost_per_million_tokens_above_200k_tokens,
                self.cache_creation_input_token_cost_above_200k_tokens,
                "cache_creation_input_token_cost_per_million_tokens_above_200k_tokens",
                "cache_creation_input_token_cost_above_200k_tokens",
            )?,
            cache_creation_input_token_cost_above_1hr: price_field(
                self.cache_creation_input_token_cost_per_million_tokens_above_1hr,
                self.cache_creation_input_token_cost_above_1hr,
                "cache_creation_input_token_cost_per_million_tokens_above_1hr",
                "cache_creation_input_token_cost_above_1hr",
            )?,
            cache_creation_input_token_cost_above_1hr_above_100k_tokens: None,
            cache_creation_input_token_cost_above_1hr_above_200k_tokens: price_field(
                self.cache_creation_input_token_cost_per_million_tokens_above_1hr_above_200k_tokens,
                self.cache_creation_input_token_cost_above_1hr_above_200k_tokens,
                "cache_creation_input_token_cost_per_million_tokens_above_1hr_above_200k_tokens",
                "cache_creation_input_token_cost_above_1hr_above_200k_tokens",
            )?,
            cache_read_input_token_cost: price_field(
                self.cache_read_input_token_cost_per_million_tokens,
                self.cache_read_input_token_cost,
                "cache_read_input_token_cost_per_million_tokens",
                "cache_read_input_token_cost",
            )?,
            cache_read_input_token_cost_above_100k_tokens: None,
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
                return Self::invalid(path, "$", "io_error", format!("failed to stat file: {err}"));
            }
        };

        if metadata.len() > MAX_CUSTOM_PRICING_FILE_BYTES {
            return Self::invalid(
                path,
                "$",
                "file_too_large",
                format!(
                    "file is {} bytes; maximum is {} bytes",
                    metadata.len(),
                    MAX_CUSTOM_PRICING_FILE_BYTES
                ),
            );
        }

        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                return Self::invalid(path, "$", "io_error", format!("failed to read file: {err}"));
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
        Self {
            models: normalized,
            ..Self::default()
        }
    }

    pub fn len(&self) -> usize {
        self.models.len() + self.providers.values().map(HashMap::len).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &ModelPricing)> {
        self.models
            .iter()
            .map(|(model_id, pricing)| (model_id.as_str(), pricing))
    }

    pub fn lookup(&self, model_id: &str) -> Option<&ModelPricing> {
        self.lookup_with_key(model_id).map(|result| result.pricing)
    }

    pub fn diagnostics(&self) -> &[CustomPricingDiagnostic] {
        &self.diagnostics
    }

    pub fn all_entries(&self) -> impl Iterator<Item = (Option<&str>, &str, &ModelPricing)> {
        self.entries()
            .map(|(model, price)| (None, model, price))
            .chain(self.providers.iter().flat_map(|(provider, models)| {
                models
                    .iter()
                    .map(move |(model, price)| (Some(provider.as_str()), model.as_str(), price))
            }))
    }

    pub fn lookup_with_key(&self, model_id: &str) -> Option<CustomLookupResult<'_>> {
        self.lookup_with_provider_key(model_id, None)
    }

    /// Model identity takes precedence over scope: scoped raw, global raw,
    /// scoped synthetic-normalized, global synthetic-normalized. No aliasing
    /// or model-derived provider identity is allowed here.
    pub fn lookup_with_provider_key(
        &self,
        model_id: &str,
        provider: Option<&str>,
    ) -> Option<CustomLookupResult<'_>> {
        let provider = provider.and_then(crate::sessions::normalize_billing_provider);
        let scoped = provider
            .as_ref()
            .and_then(|id| self.providers.get_key_value(id));
        let raw = model_id.to_lowercase();
        let normalized = normalize_synthetic_model(model_id).to_lowercase();
        for (key, was_normalized) in [(&raw, false), (&normalized, true)] {
            if was_normalized && normalized == raw {
                break;
            }
            if let Some((provider, models)) = scoped {
                if let Some((matched_key, pricing)) = models.get_key_value(key) {
                    return Some(CustomLookupResult {
                        matched_key,
                        pricing,
                        matched_provider_id: Some(provider),
                        normalized: was_normalized,
                    });
                }
            }
            if let Some((matched_key, pricing)) = self.models.get_key_value(key) {
                return Some(CustomLookupResult {
                    matched_key,
                    pricing,
                    matched_provider_id: None,
                    normalized: was_normalized,
                });
            }
        }
        None
    }

    pub fn load_from_str(content: &str, path: &Path) -> Self {
        if content.len() as u64 > MAX_CUSTOM_PRICING_FILE_BYTES {
            return Self::invalid(path, "$", "file_too_large", "file exceeds 16 MiB".into());
        }
        let raw: RawCustomPricingFile = match serde_json::from_str(content) {
            Ok(raw) => raw,
            Err(err) => {
                return Self::invalid(path, "$", "invalid_json", err.to_string());
            }
        };

        let mut result = Self::default();
        for field in raw
            .extra
            .keys()
            .filter(|field| !matches!(field.as_str(), "$schema" | "notes" | "source"))
        {
            result.error(
                path,
                &format!("$.{field}"),
                "unknown_field",
                "unknown top-level field".into(),
            );
        }
        let mut models =
            HashMap::with_capacity(raw.models.len().min(MAX_CUSTOM_PRICING_MODEL_CAPACITY));
        for (model_id, value) in raw.models {
            let lower_key = model_id.to_lowercase();
            let entry: CustomModelPricing = match serde_json::from_value(value) {
                Ok(entry) => entry,
                Err(err) => {
                    result.error(
                        path,
                        &format!("$.models.{model_id}"),
                        "invalid_pricing",
                        err.to_string(),
                    );
                    continue;
                }
            };
            if let Some(field) = entry.suspicious_unknown_rate_field() {
                result.error(
                    path,
                    &format!("$.models.{model_id}.{field}"),
                    "unknown_rate_field",
                    "unknown field looks like pricing and would otherwise be ignored".into(),
                );
                continue;
            }
            let pricing = match entry.into_model_pricing() {
                Ok(pricing) => pricing,
                Err(err) => {
                    result.error(
                        path,
                        &format!("$.models.{model_id}"),
                        "invalid_pricing",
                        err,
                    );
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

        result.models = models;
        let mut seen = HashSet::new();
        let mut duplicates = HashSet::new();
        for (index, value) in raw.provider_overrides.into_iter().enumerate() {
            let location = format!("$.provider_overrides[{index}]");
            let Some(object) = value.as_object() else {
                result.error(
                    path,
                    &location,
                    "invalid_override",
                    "expected an object".into(),
                );
                continue;
            };
            let provider = object
                .get("provider_id")
                .and_then(Value::as_str)
                .and_then(crate::sessions::normalize_billing_provider);
            let model = object
                .get("model_id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .map(str::to_lowercase);
            let (Some(provider), Some(model)) = (provider, model) else {
                result.error(
                    path,
                    &location,
                    "invalid_selector",
                    "provider_id and model_id must be nonempty strings; unknown is reserved".into(),
                );
                continue;
            };
            let key = (provider.clone(), model.clone());
            if !seen.insert(key.clone()) {
                duplicates.insert(key);
            }
            if object.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "provider_id" | "model_id" | "pricing" | "notes" | "source"
                )
            }) {
                result.error(
                    path,
                    &location,
                    "unknown_field",
                    "unknown override field".into(),
                );
                continue;
            }
            let Some(pricing) = object.get("pricing").and_then(Value::as_object) else {
                result.error(
                    path,
                    &location,
                    "invalid_pricing",
                    "pricing must be an object".into(),
                );
                continue;
            };
            if let Some((field, _)) = pricing.iter().find(|(key, value)| {
                !matches!(key.as_str(), "notes" | "source")
                    && (!CUSTOM_RATE_FIELDS.contains(&key.as_str()) || value.is_null())
            }) {
                result.error(
                    path,
                    &format!("{location}.pricing.{field}"),
                    "invalid_rate_field",
                    "unknown or null rate field".into(),
                );
                continue;
            }
            match serde_json::from_value::<CustomModelPricing>(Value::Object(pricing.clone()))
                .map_err(|err| err.to_string())
                .and_then(CustomModelPricing::into_model_pricing)
            {
                Ok(price) => {
                    result
                        .providers
                        .entry(provider)
                        .or_default()
                        .insert(model, price);
                }
                Err(err) => result.error(path, &location, "invalid_pricing", err),
            }
        }
        let mut duplicates: Vec<_> = duplicates.into_iter().collect();
        duplicates.sort();
        for (provider, model) in duplicates {
            if let Some(models) = result.providers.get_mut(&provider) {
                models.remove(&model);
            }
            result.error(path, "$.provider_overrides", "duplicate_selector", format!("duplicate ({provider}, {model}); all rules in this group disabled; global/upstream fallback may apply"));
        }
        result
    }

    fn invalid(file: &Path, path: &str, code: &str, message: String) -> Self {
        let mut result = Self::default();
        result.error(file, path, code, message);
        result
    }

    fn error(&mut self, file: &Path, path: &str, code: &str, message: String) {
        warn_custom_pricing(file, format_args!("{code} at {path}: {message}"));
        self.diagnostics.push(CustomPricingDiagnostic {
            file: file.display().to_string(),
            code: code.into(),
            path: path.into(),
            message,
        });
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
    fn loads_1hr_cache_write_rate_from_per_million_and_per_token_fields() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("custom-pricing.json");
        fs::write(
            &path,
            r#"{
                "models": {
                    "per-million": {
                        "input_cost_per_million_tokens": 10.00,
                        "output_cost_per_million_tokens": 50.00,
                        "cache_creation_input_token_cost_per_million_tokens": 12.50,
                        "cache_creation_input_token_cost_per_million_tokens_above_1hr": 20.00
                    },
                    "per-token": {
                        "input_cost_per_token": 0.00001,
                        "output_cost_per_token": 0.00005,
                        "cache_creation_input_token_cost": 0.0000125,
                        "cache_creation_input_token_cost_above_1hr": 0.00002
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = CustomPricing::load_from_path(&path);

        for key in ["per-million", "per-token"] {
            let pricing = loaded.lookup(key).unwrap();
            assert_eq!(
                pricing.cache_creation_input_token_cost_above_1hr,
                Some(0.00002),
                "{key} above_1hr rate"
            );
        }
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
    fn keeps_free_models_but_still_drops_negative_prices() {
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

        // A free model is the case this file exists to let users express when
        // no upstream dataset publishes the model (#1021). 0.0 is an assertion
        // ("free"), not an absence, so an all-zero row is kept.
        let all_zero = loaded.lookup("all-zero").expect("free model must load");
        assert_eq!(all_zero.input_cost_per_token, Some(0.0));
        assert_eq!(all_zero.output_cost_per_token, Some(0.0));

        assert_eq!(
            loaded.lookup("free-input").unwrap().output_cost_per_token,
            Some(0.000008)
        );
        // Unchanged: a negative rate is nonsense, not a statement about price.
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
