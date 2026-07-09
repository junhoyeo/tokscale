//! Config-driven model-name aliasing for grouping.
//!
//! Different supply channels report the same physical model under different
//! name-strings (for example `claude-opus-4-8`, `claude-opus-4-8-cc`, and
//! `anthropic/claude-opus-4-8` are all one model), so usage stats split across
//! multiple rows. A user-configured `{alias: canonical}` map, read from
//! `settings.json` under `modelAliases`, folds those variants into one canonical
//! model.
//!
//! The fold runs as the terminal step of [`crate::normalize_model_for_grouping`],
//! so it applies uniformly to the models report, every `--group-by`, monthly and
//! hourly reports, the graph, and the TUI. It is deliberately **not** applied
//! before pricing (per-message cost is computed on the raw model id upstream), so
//! folding can only relabel and merge already-costed buckets and can never change
//! a cost total. It is orthogonal to the pricing alias table
//! ([`crate::pricing`]) and to `provider_identity` — it touches only the model
//! dimension.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// Upper bound on the number of configured aliases retained. Oversized configs
/// are truncated rather than rejected, mirroring the capacity guard in
/// [`crate::pricing`]'s custom-pricing loader.
const MAX_MODEL_ALIASES: usize = 4096;

/// On-disk shape of the flat `modelAliases` object in `settings.json`
/// (`{ "alias": "canonical" }`). `#[serde(transparent)]` keeps the serialized
/// form a bare map. Deserialization is lossy: a malformed value (a non-object,
/// or an entry whose value is not a string) is skipped instead of failing the
/// whole settings load.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ModelAliasMap {
    /// Raw `alias -> canonical` pairs exactly as written in the config.
    pub entries: BTreeMap<String, String>,
}

impl<'de> Deserialize<'de> for ModelAliasMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Read the node as a generic value first so a malformed `modelAliases`
        // (e.g. an array or scalar) degrades to an empty map instead of
        // misaligning the parent settings deserializer. Keep only string-valued
        // entries; skip anything else.
        let value = serde_json::Value::deserialize(deserializer)?;
        let entries = match value {
            serde_json::Value::Object(object) => object
                .into_iter()
                .filter_map(|(key, value)| match value {
                    serde_json::Value::String(canonical) => Some((key, canonical)),
                    _ => None,
                })
                .collect(),
            _ => BTreeMap::new(),
        };
        Ok(Self { entries })
    }
}

/// Runtime resolver built from [`ModelAliasMap`]: keys and values are normalized
/// through [`crate::normalize_syntactic`] so lookups match regardless of case,
/// dated suffix, or `.`-vs-`-` spelling, and canonical values land in the same
/// space the grouping key uses. Empty keys/values and self-maps are dropped; the
/// number of entries is capped.
#[derive(Debug, Default)]
pub(crate) struct ModelAliasResolver {
    map: HashMap<String, String>,
}

impl ModelAliasResolver {
    /// Build a resolver from configured aliases. Both sides of each pair are run
    /// through [`crate::normalize_syntactic`] exactly once: keys are placed in the
    /// same space as incoming (already-normalized) model names, and canonical
    /// values are stored pre-normalized. `apply` returns a canonical value
    /// verbatim — it is never re-resolved or re-normalized — so the value written
    /// here is exactly the label shown in reports. Empty keys/values and
    /// self-maps are dropped, and the number of entries is capped.
    pub(crate) fn from_config(config: &ModelAliasMap) -> Self {
        let mut map = HashMap::new();
        for (raw_alias, raw_canonical) in &config.entries {
            if map.len() >= MAX_MODEL_ALIASES {
                break;
            }
            let alias = crate::normalize_syntactic(raw_alias);
            let canonical = crate::normalize_syntactic(raw_canonical);
            if alias.is_empty() || canonical.is_empty() || alias == canonical {
                continue;
            }
            map.insert(alias, canonical);
        }
        Self { map }
    }

    /// Resolve one model name. `name` must already be `normalize_syntactic`'d (it
    /// is, since the only caller is [`crate::normalize_model_for_grouping`]).
    /// Resolution is single-hop — the canonical value is never re-resolved — so
    /// alias chains collapse one step and cycles are structurally impossible.
    /// Returns `name` unchanged on a miss.
    pub(crate) fn apply(&self, name: String) -> String {
        match self.map.get(&name) {
            Some(canonical) => canonical.clone(),
            None => name,
        }
    }
}

static GLOBAL: OnceLock<ModelAliasResolver> = OnceLock::new();
static EMPTY: OnceLock<ModelAliasResolver> = OnceLock::new();

/// Install the process-wide model-alias resolver. Intended to be called once at
/// startup (mirroring the pricing service's one-time init); the first call wins
/// and later calls are ignored. Until this is called, grouping is a strict
/// identity no-op, so callers and tests that never install a resolver are
/// unaffected.
pub fn set_global(config: &ModelAliasMap) {
    let _ = GLOBAL.set(ModelAliasResolver::from_config(config));
}

/// The installed resolver, or a shared empty one (identity no-op) when unset.
pub(crate) fn global() -> &'static ModelAliasResolver {
    GLOBAL
        .get()
        .unwrap_or_else(|| EMPTY.get_or_init(ModelAliasResolver::default))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver(pairs: &[(&str, &str)]) -> ModelAliasResolver {
        let entries = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        ModelAliasResolver::from_config(&ModelAliasMap { entries })
    }

    #[test]
    fn folds_three_variants_to_one_canonical() {
        let r = resolver(&[
            ("claude-opus-4-8-cc", "claude-opus-4-8"),
            ("anthropic/claude-opus-4-8", "claude-opus-4-8"),
        ]);
        // All three real-world spellings collapse to the canonical name. The
        // third needs no map entry: syntactic normalization already lowercases it.
        for input in [
            "claude-opus-4-8-cc",
            "anthropic/claude-opus-4-8",
            "Claude-Opus-4-8",
        ] {
            assert_eq!(
                r.apply(crate::normalize_syntactic(input)),
                "claude-opus-4-8",
                "input {input} should fold to claude-opus-4-8"
            );
        }
    }

    #[test]
    fn keys_match_case_and_dotted_insensitively() {
        // Config key written with upper case and a dotted version still matches
        // the normalized input, because both sides run through normalize_syntactic.
        let r = resolver(&[("Claude-Opus-4.8-CC", "claude-opus-4-8")]);
        assert_eq!(
            r.apply(crate::normalize_syntactic("claude-opus-4-8-cc")),
            "claude-opus-4-8"
        );
    }

    #[test]
    fn drops_empty_and_self_maps() {
        let r = resolver(&[
            ("", "claude-opus-4-8"),
            ("claude-opus-4-8-cc", ""),
            ("gpt-5.5", "gpt-5.5"),
        ]);
        assert!(r.map.is_empty());
    }

    #[test]
    fn resolution_is_single_hop() {
        // {a: b, b: c} resolves a -> b (not c) and never loops.
        let r = resolver(&[("model-a", "model-b"), ("model-b", "model-c")]);
        assert_eq!(r.apply("model-a".to_string()), "model-b");
        assert_eq!(r.apply("model-b".to_string()), "model-c");
    }

    #[test]
    fn miss_is_identity() {
        let r = resolver(&[("claude-opus-4-8-cc", "claude-opus-4-8")]);
        assert_eq!(r.apply("gpt-5.5".to_string()), "gpt-5.5");
    }

    #[test]
    fn empty_resolver_is_identity() {
        let r = ModelAliasResolver::default();
        assert_eq!(
            r.apply("claude-opus-4-8-cc".to_string()),
            "claude-opus-4-8-cc"
        );
    }

    #[test]
    fn respects_capacity_cap() {
        let entries: BTreeMap<String, String> = (0..MAX_MODEL_ALIASES + 100)
            .map(|i| (format!("alias-{i}"), format!("canonical-{i}")))
            .collect();
        let r = ModelAliasResolver::from_config(&ModelAliasMap { entries });
        assert_eq!(r.map.len(), MAX_MODEL_ALIASES);
    }

    #[test]
    fn deserialize_is_lossy_over_non_string_values() {
        // Non-string values are skipped; string entries survive.
        let parsed: ModelAliasMap =
            serde_json::from_str(r#"{"a": "b", "n": 5, "arr": ["x"]}"#).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries.get("a").map(String::as_str), Some("b"));
    }

    #[test]
    fn deserialize_of_non_object_is_empty() {
        // A misuse (array/scalar instead of an object) degrades to empty, not error.
        assert!(serde_json::from_str::<ModelAliasMap>("[]")
            .unwrap()
            .entries
            .is_empty());
        assert!(serde_json::from_str::<ModelAliasMap>("\"oops\"")
            .unwrap()
            .entries
            .is_empty());
    }

    #[test]
    fn serialize_round_trips_as_flat_map() {
        let map = ModelAliasMap {
            entries: [(
                "claude-opus-4-8-cc".to_string(),
                "claude-opus-4-8".to_string(),
            )]
            .into_iter()
            .collect(),
        };
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, r#"{"claude-opus-4-8-cc":"claude-opus-4-8"}"#);
        assert_eq!(serde_json::from_str::<ModelAliasMap>(&json).unwrap(), map);
    }
}
