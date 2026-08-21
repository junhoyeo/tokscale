use super::litellm::ModelPricing;
use super::{cache, fetch};
use serde::Deserialize;
use std::collections::HashMap;

const CACHE_FILENAME: &str = "pricing-models-dev.json";
const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const PER_MILLION: f64 = 1_000_000.0;

#[derive(Deserialize)]
struct Provider {
    #[serde(default)]
    models: HashMap<String, Model>,
}

#[derive(Deserialize)]
struct Model {
    id: Option<String>,
    cost: Option<ModelCost>,
}

#[derive(Deserialize)]
struct ModelCost {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

pub type PricingDataset = HashMap<String, ModelPricing>;

pub fn load_cached() -> Option<PricingDataset> {
    cache::load_cache(CACHE_FILENAME)
}

pub fn load_cached_any_age() -> Option<PricingDataset> {
    cache::load_cache_any_age(CACHE_FILENAME)
}

pub(crate) fn parse_dataset(content: &str) -> Result<PricingDataset, serde_json::Error> {
    let providers: HashMap<String, Provider> = serde_json::from_str(content)?;
    Ok(map_providers(providers))
}

pub async fn fetch() -> Result<PricingDataset, String> {
    fetch_inner(MODELS_DEV_URL, true).await
}

/// `use_disk_cache` governs both the read below and the write at the end. See
/// the same function in `litellm.rs` for why the write is gated on the caller's
/// flag rather than on tests remembering to redirect `TOKSCALE_CONFIG_DIR`.
async fn fetch_inner(url: &str, use_disk_cache: bool) -> Result<PricingDataset, String> {
    if use_disk_cache {
        if let Some(cached) = load_cached() {
            return Ok(cached);
        }
    }

    let client = fetch::pricing_client()?;
    let response = fetch::get_with_retry(&client, url, "models.dev").await?;
    let content = response.text().await.map_err(|error| error.to_string())?;
    let data = parse_dataset(&content)
        .map_err(|error| format!("models.dev JSON parse failed: {error}"))?;
    if data.is_empty() {
        return Err("models.dev returned no usable pricing rows".to_string());
    }
    if use_disk_cache {
        if let Err(e) = cache::save_cache(CACHE_FILENAME, &data) {
            eprintln!(
                "[tokscale] Warning: Failed to cache models.dev pricing at {}: {}",
                cache::get_cache_path(CACHE_FILENAME).display(),
                e
            );
        }
    }
    Ok(data)
}

fn map_providers(providers: HashMap<String, Provider>) -> PricingDataset {
    let mut result = HashMap::new();

    for (provider_id, provider) in providers {
        for (model_key, model) in provider.models {
            let model_id = model.id.as_deref().unwrap_or(&model_key);
            let Some(pricing) = model.cost.and_then(cost_to_pricing) else {
                continue;
            };
            result.insert(format!("{provider_id}/{model_id}").to_lowercase(), pricing);
        }
    }

    result
}

fn cost_to_pricing(cost: ModelCost) -> Option<ModelPricing> {
    let input = per_token(cost.input?)?;
    let output = per_token(cost.output?)?;

    Some(ModelPricing {
        input_cost_per_token: Some(input),
        output_cost_per_token: Some(output),
        cache_read_input_token_cost: cost.cache_read.and_then(per_token),
        cache_creation_input_token_cost: cost.cache_write.and_then(per_token),
        ..Default::default()
    })
}

fn per_token(value: f64) -> Option<f64> {
    value
        .is_finite()
        .then_some(value)
        .filter(|v| *v >= 0.0)
        .map(|v| v / PER_MILLION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_env::EnvGuard;
    use serial_test::serial;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::TempDir;

    fn retryable_status_server(status_line: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        thread::spawn(move || {
            for _ in 0..3 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0; 1024];
                let _ = stream.read(&mut buffer);
                let response =
                    format!("{status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(response.as_bytes());
            }
        });

        url
    }

    fn response_server(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });

        url
    }

    #[tokio::test]
    async fn fetch_returns_error_after_retryable_http_statuses() {
        let url = retryable_status_server("HTTP/1.1 503 Service Unavailable");

        let result = fetch_inner(&url, false).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn malformed_and_empty_datasets_are_fetch_errors() {
        let malformed = fetch_inner(&response_server("not json"), false).await;
        assert!(malformed
            .unwrap_err()
            .contains("models.dev JSON parse failed"));

        let empty = fetch_inner(&response_server("{}"), false).await;
        assert!(empty.unwrap_err().contains("no usable pricing rows"));
    }

    /// models.dev carried the same defect as LiteLLM: `use_disk_cache` gated
    /// only the read, so `malformed_and_empty_datasets_are_fetch_errors` above
    /// was one successful-parse fixture away from overwriting the developer's
    /// real `pricing-models-dev.json`. See the sibling test in `litellm.rs` for
    /// why the assertion redirects `TOKSCALE_CONFIG_DIR` rather than probing
    /// the developer's home directory.
    #[tokio::test]
    #[serial]
    async fn a_fetch_with_caching_disabled_writes_no_cache_file() {
        let temp_config = TempDir::new().unwrap();
        let mut env = EnvGuard::capture(&["TOKSCALE_CONFIG_DIR"]);
        env.set("TOKSCALE_CONFIG_DIR", temp_config.path());

        let cache_path = cache::get_cache_path(CACHE_FILENAME);
        assert!(
            cache_path.starts_with(temp_config.path()),
            "the config-dir redirect must be in effect or this test proves nothing: {}",
            cache_path.display()
        );

        let url = response_server(
            r#"{"anthropic":{"models":{"claude":{"cost":{"input":3,"output":15}}}}}"#,
        );
        let data = fetch_inner(&url, false)
            .await
            .expect("the fixture serves one priced model");
        assert!(
            data.contains_key("anthropic/claude"),
            "the fetch itself must succeed"
        );
        assert_eq!(
            cache::get_cache_path(CACHE_FILENAME),
            cache_path,
            "the redirect moved while the fetch ran, so the assertion below would check a path the fetch never targeted"
        );

        assert!(
            !cache_path.exists(),
            "a fetch that opted out of the cache must not write it, but {} was created",
            cache_path.display()
        );
    }
}
