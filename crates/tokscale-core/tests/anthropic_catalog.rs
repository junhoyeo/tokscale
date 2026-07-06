//! Catalog invariant tests against the real cached pricing datasets.
//!
//! `#[ignore]`d so CI (which has no `~/.config/tokscale/cache/`) skips them.
//! Run manually when datasets are cached:
//!
//! ```sh
//! cargo test -p tokscale-core --test anthropic_catalog -- --ignored
//! ```
//!
//! These assert INVARIANTS, not exact keys:
//!
//! * [`anthropic_catalog_resolves_to_correct_family_version_and_price`] drives
//!   the Anthropic-family catalog from the model ids ACTUALLY seen in local
//!   session data (committed as `tests/fixtures/local_model_ids.txt`). Every
//!   `claude-*` id must resolve to a key carrying the same family token and the
//!   same major-minor version token (accepting both `4-8` and `4.8` spellings),
//!   at a price within ±25% of the official rate (band tolerates regional
//!   variants). A handful of additional hardcoded provider/regional forms guard
//!   shapes that may not currently appear in local data.
//! * [`real_local_models_all_resolve_sanely`] iterates the FULL harvested id
//!   set (all vendors) and asserts each id either resolves to a key whose
//!   matched family token matches the id's known vendor token, or is a
//!   documented acceptable-None — and that NO id resolves to a cross-family key.
//!
//! Retired `claude-2.x` ids and bare brand tokens must resolve to None.

use std::collections::HashMap;
use tokscale_core::pricing::lookup::PricingLookup;
use tokscale_core::pricing::{litellm, models_dev, openrouter, ModelPricing};

const FIXTURE: &str = include_str!("fixtures/local_model_ids.txt");

/// One harvested id row from the committed fixture.
struct FixtureEntry {
    id: String,
    provider: Option<String>,
    /// Vendor/family token the matched key must contain. `*` => generic.
    family: String,
    /// Dashed major-minor version token (e.g. "4-8"); dotted accepted too.
    /// Empty => no version assertion.
    version: String,
    /// Official $/MTok, or `None` for a documented acceptable-None id.
    input_per_mtok: Option<f64>,
    output_per_mtok: Option<f64>,
}

fn parse_fixture() -> Vec<FixtureEntry> {
    let mut entries = Vec::new();
    for line in FIXTURE.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        assert_eq!(
            cols.len(),
            6,
            "malformed fixture row (expected 6 pipe-separated columns): {line:?}"
        );
        let provider = if cols[1] == "-" {
            None
        } else {
            Some(cols[1].to_string())
        };
        let version = if cols[3] == "-" {
            String::new()
        } else {
            cols[3].to_string()
        };
        let parse_price = |s: &str| -> Option<f64> {
            if s == "NONE" {
                None
            } else {
                Some(
                    s.parse()
                        .unwrap_or_else(|_| panic!("bad price {s:?} in {line:?}")),
                )
            }
        };
        entries.push(FixtureEntry {
            id: cols[0].to_string(),
            provider,
            family: cols[2].to_string(),
            version,
            input_per_mtok: parse_price(cols[4]),
            output_per_mtok: parse_price(cols[5]),
        });
    }
    entries
}

/// Hardcoded provider/regional/suffix forms that round out the harvested set.
/// These guard shapes that may not currently appear in local data but must
/// keep resolving correctly.
struct CatalogEntry {
    id: &'static str,
    family: &'static str,
    version: &'static str,
    input_per_mtok: f64,
    output_per_mtok: f64,
}

const EXTRA_CATALOG: &[CatalogEntry] = &[
    e("claude-fable-5[1m]", "fable", "5", 10.0, 50.0),
    e("claude-opus-4-1", "opus", "4-1", 15.0, 75.0),
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

/// Build the lookup exactly as `PricingService` does for cached datasets
/// (filter github_copilot, no cursor overrides needed for these ids, models.dev
/// as the long-tail source), or `None` when the cache is absent.
fn load_lookup() -> Option<PricingLookup> {
    let (Some(litellm_data), Some(openrouter_data), Some(models_dev_data)) = (
        litellm::load_cached_any_age(),
        openrouter::load_cached_any_age(),
        models_dev::load_cached_any_age(),
    ) else {
        eprintln!("pricing dataset cache absent; skipping catalog invariant test");
        return None;
    };
    Some(PricingLookup::new_with_models_dev(
        filter_litellm(litellm_data),
        openrouter_data,
        HashMap::new(),
        HashMap::new(),
        models_dev_data,
    ))
}

fn lookup_id(
    lookup: &PricingLookup,
    id: &str,
    provider: Option<&str>,
) -> Option<(String, ModelPricing)> {
    lookup
        .lookup_with_provider(id, provider)
        .map(|r| (r.matched_key, r.pricing))
}

#[test]
#[ignore]
fn anthropic_catalog_resolves_to_correct_family_version_and_price() {
    let Some(lookup) = load_lookup() else {
        return;
    };

    let mut failures = Vec::new();

    // Drive the Anthropic family from the harvested real ids.
    for entry in parse_fixture() {
        let is_anthropic = entry.family == "opus"
            || entry.family == "sonnet"
            || entry.family == "haiku"
            || entry.family == "fable"
            || entry.id.contains("claude");
        if !is_anthropic {
            continue;
        }

        let resolved = lookup_id(&lookup, &entry.id, entry.provider.as_deref());

        // Documented acceptable-None ids must stay unpriced.
        if entry.input_per_mtok.is_none() {
            if let Some((key, pricing)) = resolved {
                failures.push(format!(
                    "{}: documented acceptable-None resolved to {} at {:?}",
                    entry.id, key, pricing.input_cost_per_token
                ));
            }
            continue;
        }

        let Some((matched_key, pricing)) = resolved else {
            failures.push(format!("{}: did not resolve", entry.id));
            continue;
        };
        let matched_lower = matched_key.to_lowercase();

        if !matched_lower.contains(&entry.family) {
            failures.push(format!(
                "{}: resolved to {} (missing family token {:?})",
                entry.id, matched_key, entry.family
            ));
        }
        if !entry.version.is_empty() && !key_has_version(&matched_lower, &entry.version) {
            failures.push(format!(
                "{}: resolved to {} (missing version token {:?})",
                entry.id, matched_key, entry.version
            ));
        }
        match (pricing.input_cost_per_token, entry.input_per_mtok) {
            (Some(input), Some(expected)) if within_band(input, expected) => {}
            (other, Some(expected)) => failures.push(format!(
                "{}: input price {:?} outside ±25% of ${}/MTok (key {})",
                entry.id, other, expected, matched_key
            )),
            _ => {}
        }
        match (pricing.output_cost_per_token, entry.output_per_mtok) {
            (Some(output), Some(expected)) if within_band(output, expected) => {}
            (other, Some(expected)) => failures.push(format!(
                "{}: output price {:?} outside ±25% of ${}/MTok (key {})",
                entry.id, other, expected, matched_key
            )),
            _ => {}
        }
    }

    // Hardcoded provider/regional/suffix forms.
    for entry in EXTRA_CATALOG {
        let Some((matched_key, pricing)) = lookup_id(&lookup, entry.id, None) else {
            failures.push(format!("{}: did not resolve", entry.id));
            continue;
        };
        let matched_lower = matched_key.to_lowercase();
        if !matched_lower.contains(entry.family) {
            failures.push(format!(
                "{}: resolved to {} (missing family token {:?})",
                entry.id, matched_key, entry.family
            ));
        }
        if !key_has_version(&matched_lower, entry.version) {
            failures.push(format!(
                "{}: resolved to {} (missing version token {:?})",
                entry.id, matched_key, entry.version
            ));
        }
        match pricing.input_cost_per_token {
            Some(input) if within_band(input, entry.input_per_mtok) => {}
            other => failures.push(format!(
                "{}: input price {:?} outside ±25% of ${}/MTok (key {})",
                entry.id, other, entry.input_per_mtok, matched_key
            )),
        }
        match pricing.output_cost_per_token {
            Some(output) if within_band(output, entry.output_per_mtok) => {}
            other => failures.push(format!(
                "{}: output price {:?} outside ±25% of ${}/MTok (key {})",
                entry.id, other, entry.output_per_mtok, matched_key
            )),
        }
    }

    // Retired models absent from all datasets, and bare brand tokens, must
    // resolve unpriced — never to another model's price.
    for id in ["claude-2.1", "claude-2.0", "claude", "anthropic"] {
        if let Some((key, pricing)) = lookup_id(&lookup, id, None) {
            failures.push(format!(
                "{}: must be None, resolved to {} at {:?}",
                id, key, pricing.input_cost_per_token
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "catalog invariant violations:\n{}",
        failures.join("\n")
    );
}

/// Map a fixture vendor/family token to the set of substrings that, if any
/// appears in a matched key, would indicate a CROSS-FAMILY misresolution.
/// Returns the list of foreign vendor tokens that must NOT appear.
fn cross_family_tokens(family: &str) -> Vec<&'static str> {
    // Mutually-exclusive vendor token groups. A key matched for one group must
    // not carry a token from any other group.
    const GROUPS: &[(&[&str], &[&str])] = &[
        // (family tokens that map to this group, foreign tokens to reject)
        (
            &["opus", "sonnet", "haiku", "fable", "claude"],
            &[
                "gpt", "gemini", "grok", "glm", "kimi", "qwen", "deepseek", "mistral", "llama",
            ],
        ),
        (
            &["gpt"],
            &[
                "claude", "opus", "sonnet", "haiku", "gemini", "grok", "glm", "kimi", "qwen",
                "deepseek", "mistral", "llama",
            ],
        ),
        (
            &["gemini"],
            &[
                "claude", "opus", "sonnet", "haiku", "gpt", "grok", "glm", "kimi", "qwen",
                "deepseek", "mistral", "llama",
            ],
        ),
        (
            &["grok"],
            &[
                "claude", "opus", "sonnet", "haiku", "gpt", "gemini", "glm", "kimi", "qwen",
                "deepseek", "mistral", "llama",
            ],
        ),
        (
            &["glm"],
            &[
                "claude", "opus", "sonnet", "haiku", "gpt", "gemini", "grok", "kimi", "qwen",
                "deepseek", "mistral", "llama",
            ],
        ),
        (
            &["kimi"],
            &[
                "claude", "opus", "sonnet", "haiku", "gpt", "gemini", "grok", "glm", "qwen",
                "deepseek", "mistral", "llama",
            ],
        ),
    ];
    for (members, foreign) in GROUPS {
        if members.contains(&family) {
            return foreign.to_vec();
        }
    }
    Vec::new()
}

#[test]
#[ignore]
fn real_local_models_all_resolve_sanely() {
    let Some(lookup) = load_lookup() else {
        return;
    };

    let mut failures = Vec::new();

    for entry in parse_fixture() {
        let resolved = lookup_id(&lookup, &entry.id, entry.provider.as_deref());

        // Documented acceptable-None: must stay unpriced. Flag ANY resolution
        // (matching the sibling catalog test), not just an input-priced one —
        // a key with `input: None` but `output: Some(..)` is still a wrong
        // resolution and must not slip through.
        if entry.input_per_mtok.is_none() {
            if let Some((key, pricing)) = resolved {
                failures.push(format!(
                    "{} [{}]: documented acceptable-None resolved to {} at in={:?} out={:?}",
                    entry.id,
                    entry.provider.as_deref().unwrap_or("-"),
                    key,
                    pricing.input_cost_per_token,
                    pricing.output_cost_per_token
                ));
            }
            continue;
        }

        let Some((matched_key, _)) = resolved else {
            failures.push(format!(
                "{} [{}]: expected to resolve but returned None",
                entry.id,
                entry.provider.as_deref().unwrap_or("-")
            ));
            continue;
        };
        let lower = matched_key.to_lowercase();

        // Generic family ("*") only checks that SOMETHING resolved (above).
        if entry.family == "*" {
            continue;
        }

        // Known vendor token must appear in the matched key.
        if !lower.contains(&entry.family) {
            failures.push(format!(
                "{} [{}]: matched key {} is missing expected family token {:?}",
                entry.id,
                entry.provider.as_deref().unwrap_or("-"),
                matched_key,
                entry.family
            ));
        }

        // And no FOREIGN vendor token may appear (cross-family misresolution).
        for foreign in cross_family_tokens(&entry.family) {
            if lower.contains(foreign) {
                failures.push(format!(
                    "{} [{}]: CROSS-FAMILY misresolution — key {} carries foreign token {:?} (expected {:?})",
                    entry.id,
                    entry.provider.as_deref().unwrap_or("-"),
                    matched_key,
                    foreign,
                    entry.family
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "real-local-model sanity violations:\n{}",
        failures.join("\n")
    );
}
