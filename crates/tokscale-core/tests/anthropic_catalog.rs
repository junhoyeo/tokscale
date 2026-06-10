//! Catalog invariant test against the real cached pricing datasets.
//!
//! `#[ignore]`d so CI (which has no `~/.config/tokscale/cache/`) skips it.
//! Run manually when datasets are cached:
//!
//! ```sh
//! cargo test -p tokscale-core --test anthropic_catalog -- --ignored
//! ```
//!
//! Asserts INVARIANTS, not exact keys: every id in the official Anthropic
//! catalog must resolve to a key carrying the same family token and the same
//! major-minor version token (accepting both `4-8` and `4.8` spellings), at a
//! price within ±25% of the official rate (band tolerates regional variants).
//! Retired `claude-2.x` ids and bare brand tokens must resolve to None.

use std::collections::HashMap;
use tokscale_core::pricing::lookup::PricingLookup;
use tokscale_core::pricing::{litellm, models_dev, openrouter, ModelPricing};

struct CatalogEntry {
    id: &'static str,
    family: &'static str,
    /// Major-minor version token in dashed spelling (e.g. "4-8"); the dotted
    /// spelling is accepted too. Bare-major models (fable-5) use "5".
    version: &'static str,
    /// Official $/MTok.
    input_per_mtok: f64,
    output_per_mtok: f64,
}

const CATALOG: &[CatalogEntry] = &[
    // Current models
    e("claude-fable-5", "fable", "5", 10.0, 50.0),
    e("claude-fable-5[1m]", "fable", "5", 10.0, 50.0),
    e("claude-opus-4-8", "opus", "4-8", 5.0, 25.0),
    e("claude-opus-4-7", "opus", "4-7", 5.0, 25.0),
    e("claude-opus-4-6", "opus", "4-6", 5.0, 25.0),
    e("claude-sonnet-4-6", "sonnet", "4-6", 3.0, 15.0),
    e("claude-haiku-4-5", "haiku", "4-5", 1.0, 5.0),
    // Legacy models
    e("claude-opus-4-5", "opus", "4-5", 5.0, 25.0),
    e("claude-opus-4-1", "opus", "4-1", 15.0, 75.0),
    e("claude-sonnet-4-5", "sonnet", "4-5", 3.0, 15.0),
    // Dated forms
    e("claude-haiku-4-5-20251001", "haiku", "4-5", 1.0, 5.0),
    e("claude-opus-4-5-20251101", "opus", "4-5", 5.0, 25.0),
    e("claude-opus-4-1-20250805", "opus", "4-1", 15.0, 75.0),
    e("claude-sonnet-4-5-20250929", "sonnet", "4-5", 3.0, 15.0),
    // Provider forms
    e("anthropic.claude-opus-4-8", "opus", "4-8", 5.0, 25.0),
    e(
        "us.anthropic.claude-sonnet-4-6-v1:0",
        "sonnet",
        "4-6",
        3.0,
        15.0,
    ),
];

const fn e(
    id: &'static str,
    family: &'static str,
    version: &'static str,
    input_per_mtok: f64,
    output_per_mtok: f64,
) -> CatalogEntry {
    CatalogEntry {
        id,
        family,
        version,
        input_per_mtok,
        output_per_mtok,
    }
}

/// Mirror of `PricingService::filter_litellm_data`: github_copilot/ entries
/// use subscription pricing ($0.00) and are excluded from lookups.
fn filter_litellm(mut data: HashMap<String, ModelPricing>) -> HashMap<String, ModelPricing> {
    data.retain(|key, _| !key.to_lowercase().starts_with("github_copilot/"));
    data
}

fn within_band(actual_per_token: f64, official_per_mtok: f64) -> bool {
    let official_per_token = official_per_mtok / 1e6;
    (actual_per_token - official_per_token).abs() <= official_per_token * 0.25
}

fn key_has_version(matched_lower: &str, version: &str) -> bool {
    let dashed = version.to_string();
    let dotted = version.replace('-', ".");
    matched_lower.contains(&dashed) || matched_lower.contains(&dotted)
}

#[test]
#[ignore]
fn anthropic_catalog_resolves_to_correct_family_version_and_price() {
    let (Some(litellm_data), Some(openrouter_data), Some(models_dev_data)) = (
        litellm::load_cached_any_age(),
        openrouter::load_cached_any_age(),
        models_dev::load_cached_any_age(),
    ) else {
        eprintln!("pricing dataset cache absent; skipping catalog invariant test");
        return;
    };

    let lookup = PricingLookup::new_with_models_dev(
        filter_litellm(litellm_data),
        openrouter_data,
        HashMap::new(),
        models_dev_data,
    );

    let mut failures = Vec::new();

    for entry in CATALOG {
        let Some(result) = lookup.lookup(entry.id) else {
            failures.push(format!("{}: did not resolve", entry.id));
            continue;
        };
        let matched_lower = result.matched_key.to_lowercase();

        if !matched_lower.contains(entry.family) {
            failures.push(format!(
                "{}: resolved to {} (missing family token {:?})",
                entry.id, result.matched_key, entry.family
            ));
        }
        if !key_has_version(&matched_lower, entry.version) {
            failures.push(format!(
                "{}: resolved to {} (missing version token {:?})",
                entry.id, result.matched_key, entry.version
            ));
        }

        match result.pricing.input_cost_per_token {
            Some(input) if within_band(input, entry.input_per_mtok) => {}
            other => failures.push(format!(
                "{}: input price {:?} outside ±25% of ${}/MTok (key {})",
                entry.id, other, entry.input_per_mtok, result.matched_key
            )),
        }
        match result.pricing.output_cost_per_token {
            Some(output) if within_band(output, entry.output_per_mtok) => {}
            other => failures.push(format!(
                "{}: output price {:?} outside ±25% of ${}/MTok (key {})",
                entry.id, other, entry.output_per_mtok, result.matched_key
            )),
        }
    }

    // Retired models absent from all datasets, and bare brand tokens, must
    // resolve unpriced — never to another model's price.
    for id in ["claude-2.1", "claude-2.0", "claude"] {
        if let Some(result) = lookup.lookup(id) {
            failures.push(format!(
                "{}: must be None, resolved to {} at {:?}",
                id, result.matched_key, result.pricing.input_cost_per_token
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "catalog invariant violations:\n{}",
        failures.join("\n")
    );
}
