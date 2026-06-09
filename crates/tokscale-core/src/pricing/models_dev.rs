use super::cache;
use super::litellm::ModelPricing;
use serde::Deserialize;
use std::collections::HashMap;

const CACHE_FILENAME: &str = "pricing-models-dev.json";
const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 200;
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

pub async fn fetch() -> Result<PricingDataset, reqwest::Error> {
    fetch_inner(MODELS_DEV_URL, true).await
}

async fn fetch_inner(url: &str, use_cache: bool) -> Result<PricingDataset, reqwest::Error> {
    if use_cache {
        if let Some(cached) = load_cached() {
            return Ok(cached);
        }
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut last_error: Option<reqwest::Error> = None;

    for attempt in 0..MAX_RETRIES {
        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status();

                if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    eprintln!(
                        "[tokscale] models.dev HTTP {} (attempt {}/{})",
                        status,
                        attempt + 1,
                        MAX_RETRIES
                    );
                    if attempt == MAX_RETRIES - 1 {
                        return Err(response.error_for_status().unwrap_err());
                    }
                    let _ = response.bytes().await;
                    tokio::time::sleep(std::time::Duration::from_millis(
                        INITIAL_BACKOFF_MS * (1 << attempt),
                    ))
                    .await;
                    continue;
                }

                if !status.is_success() {
                    eprintln!("[tokscale] models.dev HTTP {}", status);
                    return Err(response.error_for_status().unwrap_err());
                }

                let content = response.text().await?;
                match parse_dataset(&content) {
                    Ok(data) => {
                        if let Err(e) = cache::save_cache(CACHE_FILENAME, &data) {
                            eprintln!(
                                "[tokscale] Warning: Failed to cache models.dev pricing at {}: {}",
                                cache::get_cache_path(CACHE_FILENAME).display(),
                                e
                            );
                        }
                        return Ok(data);
                    }
                    Err(e) => {
                        eprintln!("[tokscale] models.dev JSON parse failed: {}", e);
                        return Ok(HashMap::new());
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[tokscale] models.dev network error (attempt {}/{}): {}",
                    attempt + 1,
                    MAX_RETRIES,
                    e
                );
                last_error = Some(e);
                if attempt < MAX_RETRIES - 1 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        INITIAL_BACKOFF_MS * (1 << attempt),
                    ))
                    .await;
                }
            }
        }
    }

    match last_error {
        Some(e) => Err(e),
        None => Ok(HashMap::new()),
    }
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn retryable_status_server(status_line: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        thread::spawn(move || {
            for _ in 0..MAX_RETRIES {
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

    #[tokio::test]
    async fn fetch_returns_error_after_retryable_http_statuses() {
        let url = retryable_status_server("HTTP/1.1 503 Service Unavailable");

        let result = fetch_inner(&url, false).await;

        assert!(result.is_err());
    }
}
