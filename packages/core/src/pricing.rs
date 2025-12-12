//! Pricing calculation module
//!
//! Receives pricing data from TypeScript and calculates costs for messages.
//! Optimized for high-throughput lookups with pre-computed indices.

use std::collections::HashMap;

/// Internal pricing data for a single model
#[derive(Debug, Clone, Default)]
pub struct ModelPricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_read_input_token_cost: f64,
    pub cache_creation_input_token_cost: f64,
}

/// Pre-computed key entry for fast fuzzy matching
#[derive(Debug, Clone)]
struct IndexedKey {
    original: String,
    lowercase: String,
}

fn is_word_boundary_match(haystack: &str, needle: &str) -> bool {
    if let Some(pos) = haystack.find(needle) {
        let before_ok = pos == 0 || {
            let ch = haystack[..pos].chars().last().unwrap();
            !ch.is_alphanumeric()
        };
        let after_ok = pos + needle.len() == haystack.len() || {
            let ch = haystack[pos + needle.len()..].chars().next().unwrap();
            !ch.is_alphanumeric()
        };
        before_ok && after_ok
    } else {
        false
    }
}

/// Pricing dataset containing all model pricing
/// Optimized with pre-computed indices for fast lookups
#[derive(Debug, Clone, Default)]
pub struct PricingData {
    models: HashMap<String, ModelPricing>,
    /// Pre-sorted keys with lowercase versions (computed once via finalize())
    sorted_keys: Vec<IndexedKey>,
    /// Cache of model_id -> resolved key for repeated lookups
    resolution_cache: HashMap<String, Option<String>>,
}

impl PricingData {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            sorted_keys: Vec::new(),
            resolution_cache: HashMap::new(),
        }
    }

    /// Add pricing for a model
    pub fn add_model(&mut self, model_id: String, pricing: ModelPricing) {
        self.models.insert(model_id, pricing);
    }

    /// Finalize the pricing data by pre-computing sorted keys
    /// Call this after all models have been added
    pub fn finalize(&mut self) {
        let mut keys: Vec<IndexedKey> = self
            .models
            .keys()
            .map(|k| IndexedKey {
                original: k.clone(),
                lowercase: k.to_lowercase(),
            })
            .collect();
        keys.sort_by(|a, b| a.original.cmp(&b.original));
        self.sorted_keys = keys;
        self.resolution_cache.clear();
    }

    /// Get pricing for a model with fuzzy matching (uses pre-computed indices)
    pub fn get_pricing(&self, model_id: &str) -> Option<&ModelPricing> {
        // Check resolution cache first
        if let Some(cached) = self.resolution_cache.get(model_id) {
            return cached.as_ref().and_then(|k| self.models.get(k));
        }

        // Direct lookup
        if let Some(pricing) = self.models.get(model_id) {
            return Some(pricing);
        }

        // Try with provider prefixes
        let prefixes = ["anthropic/", "openai/", "google/", "bedrock/"];
        for prefix in prefixes {
            let key = format!("{}{}", prefix, model_id);
            if let Some(pricing) = self.models.get(&key) {
                return Some(pricing);
            }
        }

        // Normalize model name for Cursor-style names
        let normalized = Self::normalize_cursor_model_name(model_id);
        if let Some(ref norm) = normalized {
            if let Some(pricing) = self.models.get(norm) {
                return Some(pricing);
            }
            for prefix in prefixes {
                let key = format!("{}{}", prefix, norm);
                if let Some(pricing) = self.models.get(&key) {
                    return Some(pricing);
                }
            }
        }

        // Fuzzy matching using pre-computed sorted keys
        let lower_model = model_id.to_lowercase();
        let lower_normalized = normalized.as_ref().map(|s| s.to_lowercase());

        for indexed in &self.sorted_keys {
            if is_word_boundary_match(&indexed.lowercase, &lower_model) {
                return self.models.get(&indexed.original);
            }
            if let Some(ref ln) = lower_normalized {
                if is_word_boundary_match(&indexed.lowercase, ln) {
                    return self.models.get(&indexed.original);
                }
            }
        }

        for indexed in &self.sorted_keys {
            if is_word_boundary_match(&lower_model, &indexed.lowercase) {
                return self.models.get(&indexed.original);
            }
            if let Some(ref ln) = lower_normalized {
                if is_word_boundary_match(ln, &indexed.lowercase) {
                    return self.models.get(&indexed.original);
                }
            }
        }

        None
    }

    /// Get pricing with caching (mutable version for building cache)
    pub fn get_pricing_cached(&mut self, model_id: &str) -> Option<&ModelPricing> {
        // Check cache first
        if let Some(cached) = self.resolution_cache.get(model_id) {
            return cached.as_ref().and_then(|k| self.models.get(k));
        }

        // Resolve the key
        let resolved_key = self.resolve_model_key(model_id);

        // Cache the result
        self.resolution_cache
            .insert(model_id.to_string(), resolved_key.clone());

        resolved_key.and_then(|k| self.models.get(&k))
    }

    /// Resolve model_id to the actual key in the pricing map
    fn resolve_model_key(&self, model_id: &str) -> Option<String> {
        // Direct lookup
        if self.models.contains_key(model_id) {
            return Some(model_id.to_string());
        }

        // Try with provider prefixes
        let prefixes = ["anthropic/", "openai/", "google/", "bedrock/"];
        for prefix in prefixes {
            let key = format!("{}{}", prefix, model_id);
            if self.models.contains_key(&key) {
                return Some(key);
            }
        }

        // Normalize model name
        let normalized = Self::normalize_cursor_model_name(model_id);
        if let Some(ref norm) = normalized {
            if self.models.contains_key(norm) {
                return Some(norm.clone());
            }
            for prefix in prefixes {
                let key = format!("{}{}", prefix, norm);
                if self.models.contains_key(&key) {
                    return Some(key);
                }
            }
        }

        // Fuzzy matching using pre-computed sorted keys
        let lower_model = model_id.to_lowercase();
        let lower_normalized = normalized.as_ref().map(|s| s.to_lowercase());

        for indexed in &self.sorted_keys {
            if is_word_boundary_match(&indexed.lowercase, &lower_model) {
                return Some(indexed.original.clone());
            }
            if let Some(ref ln) = lower_normalized {
                if is_word_boundary_match(&indexed.lowercase, ln) {
                    return Some(indexed.original.clone());
                }
            }
        }

        for indexed in &self.sorted_keys {
            if is_word_boundary_match(&lower_model, &indexed.lowercase) {
                return Some(indexed.original.clone());
            }
            if let Some(ref ln) = lower_normalized {
                if is_word_boundary_match(ln, &indexed.lowercase) {
                    return Some(indexed.original.clone());
                }
            }
        }

        None
    }

    /// Normalize Cursor-style model names to standard format
    fn normalize_cursor_model_name(model_id: &str) -> Option<String> {
        let lower = model_id.to_lowercase();

        // Claude models
        if lower.contains("opus") {
            if lower.contains("4.5") || lower.contains("4-5") {
                return Some("opus-4-5".to_string());
            } else if lower.contains("4") {
                return Some("opus-4".to_string());
            }
        }
        if lower.contains("sonnet") {
            if lower.contains("4.5") || lower.contains("4-5") {
                return Some("sonnet-4-5".to_string());
            } else if lower.contains("4") {
                return Some("sonnet-4".to_string());
            } else if lower.contains("3.7") || lower.contains("3-7") {
                return Some("sonnet-3-7".to_string());
            } else if lower.contains("3.5") || lower.contains("3-5") {
                return Some("sonnet-3-5".to_string());
            }
        }
        if lower.contains("haiku") && (lower.contains("4.5") || lower.contains("4-5")) {
            return Some("haiku-4-5".to_string());
        }

        // OpenAI models
        if lower == "o3" {
            return Some("o3".to_string());
        }
        if lower.starts_with("gpt-4o") || lower == "gpt-4o" {
            return Some("gpt-4o".to_string());
        }
        if lower.starts_with("gpt-4.1") || lower.contains("gpt-4.1") {
            return Some("gpt-4.1".to_string());
        }

        // Gemini models
        if lower.contains("gemini-2.5-pro") {
            return Some("gemini-2.5-pro".to_string());
        }
        if lower.contains("gemini-2.5-flash") {
            return Some("gemini-2.5-flash".to_string());
        }

        None
    }

    /// Calculate cost for token usage
    pub fn calculate_cost(
        &self,
        model_id: &str,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
    ) -> f64 {
        let pricing = match self.get_pricing(model_id) {
            Some(p) => p,
            None => return 0.0,
        };

        let input_cost = input as f64 * pricing.input_cost_per_token;
        let output_cost = (output + reasoning) as f64 * pricing.output_cost_per_token;
        let cache_read_cost = cache_read as f64 * pricing.cache_read_input_token_cost;
        let cache_write_cost = cache_write as f64 * pricing.cache_creation_input_token_cost;

        input_cost + output_cost + cache_read_cost + cache_write_cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_cost() {
        let mut pricing = PricingData::new();
        pricing.add_model(
            "claude-3-5-sonnet-20241022".to_string(),
            ModelPricing {
                input_cost_per_token: 3.0 / 1_000_000.0,
                output_cost_per_token: 15.0 / 1_000_000.0,
                cache_read_input_token_cost: 0.3 / 1_000_000.0,
                cache_creation_input_token_cost: 3.75 / 1_000_000.0,
            },
        );
        pricing.finalize();

        let cost = pricing.calculate_cost("claude-3-5-sonnet-20241022", 1000, 500, 2000, 100, 0);

        assert!((cost - 0.011475).abs() < 0.0001);
    }

    #[test]
    fn test_fuzzy_matching() {
        let mut pricing = PricingData::new();
        pricing.add_model(
            "anthropic/claude-3-5-sonnet-20241022".to_string(),
            ModelPricing {
                input_cost_per_token: 3.0 / 1_000_000.0,
                output_cost_per_token: 15.0 / 1_000_000.0,
                cache_read_input_token_cost: 0.3 / 1_000_000.0,
                cache_creation_input_token_cost: 3.75 / 1_000_000.0,
            },
        );
        pricing.finalize();

        assert!(pricing.get_pricing("claude-3-5-sonnet-20241022").is_some());
    }

    #[test]
    fn test_cached_lookup() {
        let mut pricing = PricingData::new();
        pricing.add_model(
            "anthropic/claude-3-5-sonnet-20241022".to_string(),
            ModelPricing {
                input_cost_per_token: 3.0 / 1_000_000.0,
                output_cost_per_token: 15.0 / 1_000_000.0,
                cache_read_input_token_cost: 0.3 / 1_000_000.0,
                cache_creation_input_token_cost: 3.75 / 1_000_000.0,
            },
        );
        pricing.finalize();

        // First lookup resolves and caches
        assert!(pricing
            .get_pricing_cached("claude-3-5-sonnet-20241022")
            .is_some());

        // Second lookup uses cache
        assert!(pricing
            .get_pricing_cached("claude-3-5-sonnet-20241022")
            .is_some());
    }
}
