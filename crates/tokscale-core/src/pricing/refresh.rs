use super::{cache, litellm, models_dev, openrouter, ModelPricing};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct PricingRefreshSource {
    pub source: &'static str,
    pub models: usize,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PricingRefreshReport {
    pub sources: Vec<PricingRefreshSource>,
}

impl PricingRefreshReport {
    pub fn succeeded(&self) -> bool {
        self.sources.iter().all(|source| source.error.is_none())
    }
}

fn persist(
    source: &'static str,
    filename: &str,
    fetched: Result<HashMap<String, ModelPricing>, String>,
) -> PricingRefreshSource {
    let mut models = 0;
    let error = fetched
        .and_then(|data| {
            if data.is_empty() {
                return Err("source returned no usable pricing rows".to_string());
            }
            cache::save_cache(filename, &data)
                .map_err(|e| format!("could not save pricing cache: {e}"))?;
            models = data.len();
            Ok(())
        })
        .err();
    PricingRefreshSource {
        source,
        models,
        error,
    }
}

pub(super) async fn refresh() -> Result<PricingRefreshReport, String> {
    crate::network::require_online("Pricing refresh")?;
    // Fetch with all disk access disabled, then publish successful sources here.
    // A network failure or invalid payload must not erase a portable stale cache.
    let (litellm, openrouter, models_dev) = tokio::join!(
        litellm::fetch_uncached(),
        openrouter::fetch_uncached(),
        models_dev::fetch_uncached()
    );
    Ok(PricingRefreshReport {
        sources: vec![
            persist("litellm", "pricing-litellm.json", litellm),
            persist("openrouter", "pricing-openrouter.json", openrouter),
            persist("models.dev", "pricing-models-dev.json", models_dev),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;

    #[test]
    #[serial_test::serial]
    fn refresh_preserves_failed_sources_and_reports_failed_cache_writes() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut env = EnvGuard::capture(&["TOKSCALE_CONFIG_DIR"]);
        env.set("TOKSCALE_CONFIG_DIR", temp.path());
        let data = HashMap::from([(
            "example".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000002),
                ..Default::default()
            },
        )]);
        let result = persist("litellm", "pricing-litellm.json", Ok(data.clone()));
        assert_eq!(result.models, 1);
        assert!(result.error.is_none());
        let path = cache::get_cache_path("pricing-litellm.json");
        let before = std::fs::read(&path).unwrap();
        for failed in [Err("network unavailable".to_string()), Ok(HashMap::new())] {
            assert!(persist("litellm", "pricing-litellm.json", failed)
                .error
                .is_some());
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        // A directory at the destination makes the atomic replacement fail on
        // every platform, including privileged CI users that bypass chmod.
        std::fs::create_dir(cache::get_cache_path("blocked.json")).unwrap();
        let failed = persist("models.dev", "blocked.json", Ok(data));
        assert_eq!(failed.models, 0);
        assert!(failed.error.unwrap().contains("could not save"));
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}
