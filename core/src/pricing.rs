//! Pricing calculation module
//!
//! Receives pricing data from TypeScript and calculates costs for messages.

use std::collections::HashMap;

/// Internal pricing data for a single model
#[derive(Debug, Clone, Default)]
pub struct ModelPricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_read_input_token_cost: f64,
    pub cache_creation_input_token_cost: f64,
}

/// Pricing dataset containing all model pricing
#[derive(Debug, Clone, Default)]
pub struct PricingData {
    models: HashMap<String, ModelPricing>,
}

impl PricingData {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    /// Add pricing for a model
    pub fn add_model(&mut self, model_id: String, pricing: ModelPricing) {
        self.models.insert(model_id, pricing);
    }

    /// Get pricing for a model with fuzzy matching
    pub fn get_pricing(&self, model_id: &str) -> Option<&ModelPricing> {
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
        // e.g., "claude-4-sonnet" → "sonnet-4", "4-opus" → "opus-4"
        let normalized = Self::normalize_cursor_model_name(model_id);
        if let Some(ref norm) = normalized {
            if let Some(pricing) = self.models.get(norm) {
                return Some(pricing);
            }
            // Try with prefixes on normalized name
            for prefix in prefixes {
                let key = format!("{}{}", prefix, norm);
                if let Some(pricing) = self.models.get(&key) {
                    return Some(pricing);
                }
            }
        }

        // Fuzzy matching - check if model_id is contained in any key or vice versa
        let lower_model = model_id.to_lowercase();
        let lower_normalized = normalized.as_ref().map(|s| s.to_lowercase());
        
        for (key, pricing) in &self.models {
            let lower_key = key.to_lowercase();
            
            // Check original model name
            if lower_key.contains(&lower_model) || lower_model.contains(&lower_key) {
                return Some(pricing);
            }
            
            // Check normalized name
            if let Some(ref ln) = lower_normalized {
                if lower_key.contains(ln) || ln.contains(&lower_key) {
                    return Some(pricing);
                }
            }
        }

        None
    }

    /// Normalize Cursor-style model names to standard format
    /// e.g., "claude-4-sonnet" → "sonnet-4", "claude-4-opus-thinking" → "opus-4"
    fn normalize_cursor_model_name(model_id: &str) -> Option<String> {
        let lower = model_id.to_lowercase();
        
        // Map Cursor model patterns to standard names
        // Claude models: claude-X-{model} or X-{model} → {model}-X
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
        if lower.contains("haiku") {
            if lower.contains("4.5") || lower.contains("4-5") {
                return Some("haiku-4-5".to_string());
            }
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
            None => return 0.0, // No pricing found
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

        let cost = pricing.calculate_cost(
            "claude-3-5-sonnet-20241022",
            1000, // input
            500,  // output
            2000, // cache_read
            100,  // cache_write
            0,    // reasoning
        );

        // Expected: (1000 * 3/1M) + (500 * 15/1M) + (2000 * 0.3/1M) + (100 * 3.75/1M)
        // = 0.003 + 0.0075 + 0.0006 + 0.000375 = 0.011475
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

        // Should find via prefix matching
        assert!(pricing.get_pricing("claude-3-5-sonnet-20241022").is_some());
    }
}
