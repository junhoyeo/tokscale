use super::cache;
use super::litellm::ModelPricing;
use std::collections::HashMap;
use std::sync::Arc;
use serde::Deserialize;
use tokio::sync::Semaphore;

const CACHE_FILENAME: &str = "pricing-openrouter.json";
const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 200;
const MAX_CONCURRENT_REQUESTS: usize = 10;

/// Structs for `/api/v1/models` endpoint (list all models).

#[derive(Deserialize)]
struct ModelListPricing {
    prompt: String,
    completion: String,
}

#[derive(Deserialize)]
struct ModelListItem {
    id: String,
    pricing: Option<ModelListPricing>,
}

#[derive(Deserialize)]
struct ModelsListResponse {
    data: Vec<ModelListItem>,
}

/// Structs for `/api/v1/models/{id}/endpoints` endpoint (author pricing).

#[derive(Deserialize)]
struct EndpointPricing {
    prompt: String,
    completion: String,
    #[serde(default)]
    input_cache_read: Option<String>,
    #[serde(default)]
    input_cache_write: Option<String>,
}

#[derive(Deserialize)]
struct Endpoint {
    provider_name: String,
    pricing: EndpointPricing,
}

#[derive(Deserialize)]
struct EndpointData {
    #[allow(dead_code)]
    id: String,
    endpoints: Vec<Endpoint>,
}

#[derive(Deserialize)]
struct EndpointsResponse {
    data: EndpointData,
}

/// Model ID prefix to provider name mapping.
///
/// Translates model ID prefixes like `z-ai` to their corresponding
/// provider names in the endpoints API, such as `Z.AI`.
fn get_author_provider_name(model_id: &str) -> Option<&'static str> {
    let prefix = model_id.split('/').next()?;
    
    match prefix.to_lowercase().as_str() {
        "z-ai" => Some("Z.AI"),
        "x-ai" => Some("xAI"),
        "anthropic" => Some("Anthropic"),
        "openai" => Some("OpenAI"),
        "google" => Some("Google"),
        "meta-llama" => Some("Meta"),
        "mistralai" => Some("Mistral"),
        "deepseek" => Some("DeepSeek"),
        "qwen" => Some("Alibaba"),
        "cohere" => Some("Cohere"),
        "perplexity" => Some("Perplexity"),
        "moonshotai" => Some("Moonshot AI"),
        _ => None,
    }
}

pub fn load_cached() -> Option<HashMap<String, ModelPricing>> {
    cache::load_cache(CACHE_FILENAME)
}

fn parse_price(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

async fn fetch_author_pricing(
    client: Arc<reqwest::Client>, 
    model_id: String,
    semaphore: Arc<Semaphore>,
    fallback_pricing: Option<ModelPricing>,
) -> Option<(String, ModelPricing)> {
    let _permit = semaphore.acquire().await.ok()?;
    
    let author_name = match get_author_provider_name(&model_id) {
        Some(name) => name,
        None => return fallback_pricing.map(|p| (model_id, p)),
    };
    
    let url = format!("https://openrouter.ai/api/v1/models/{}/endpoints", model_id);
    
    let response = match client.get(&url)
        .header("Content-Type", "application/json")
        .send()
        .await {
            Ok(r) => r,
            Err(_) => {
                return fallback_pricing.map(|p| (model_id, p));
            }
        };
    
    if !response.status().is_success() {
        return fallback_pricing.map(|p| (model_id, p));
    }
    
    let data: EndpointsResponse = match response.json().await {
        Ok(d) => d,
        Err(_) => {
            return fallback_pricing.map(|p| (model_id, p));
        }
    };
    
    // Find the endpoint from the author provider
    let author_endpoint = match data.data.endpoints.iter()
        .find(|e| e.provider_name == author_name) {
            Some(ep) => ep,
            None => {
                return fallback_pricing.map(|p| (model_id, p));
            }
        };
    
    let input_cost = parse_price(&author_endpoint.pricing.prompt);
    let output_cost = parse_price(&author_endpoint.pricing.completion);
    
    if input_cost.is_none() || output_cost.is_none() {
        return fallback_pricing.map(|p| (model_id, p));
    }
    
    let pricing = ModelPricing {
        input_cost_per_token: input_cost,
        output_cost_per_token: output_cost,
        cache_read_input_token_cost: author_endpoint.pricing.input_cache_read
            .as_ref()
            .and_then(|s| parse_price(s)),
        cache_creation_input_token_cost: author_endpoint.pricing.input_cache_write
            .as_ref()
            .and_then(|s| parse_price(s)),
    };
    
    Some((model_id, pricing))
}

/// Fetch all models and get author pricing for each
pub async fn fetch_all_models() -> HashMap<String, ModelPricing> {
    if let Some(cached) = load_cached() {
        return cached;
    }
    
    let client = Arc::new(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default());
    
    let mut last_error: Option<String> = None;
    
    let models_with_fallback: Vec<(String, Option<ModelPricing>)> = 'retry: {
        for attempt in 0..MAX_RETRIES {
            let response = match client.get(MODELS_URL)
                .header("Content-Type", "application/json")
                .send()
                .await {
                    Ok(r) => r,
                    Err(e) => {
                        last_error = Some(format!("network error: {}", e));
                        if attempt < MAX_RETRIES - 1 {
                            tokio::time::sleep(std::time::Duration::from_millis(
                                INITIAL_BACKOFF_MS * (1 << attempt)
                            )).await;
                        }
                        continue;
                    }
                };
            
            let status = response.status();
            if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                last_error = Some(format!("HTTP {}", status));
                let _ = response.bytes().await;
                if attempt < MAX_RETRIES - 1 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        INITIAL_BACKOFF_MS * (1 << attempt)
                    )).await;
                }
                continue;
            }
            
            if !status.is_success() {
                eprintln!("[tokscale] OpenRouter models API returned {}", status);
                break 'retry Vec::new();
            }
            
            let data: ModelsListResponse = match response.json().await {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("[tokscale] OpenRouter models JSON parse failed: {}", e);
                    break 'retry Vec::new();
                }
            };
            
            break 'retry data.data.into_iter()
                .map(|m| {
                    let fallback = m.pricing.and_then(|p| {
                        let input = parse_price(&p.prompt)?;
                        let output = parse_price(&p.completion)?;
                        Some(ModelPricing {
                            input_cost_per_token: Some(input),
                            output_cost_per_token: Some(output),
                            cache_read_input_token_cost: None,
                            cache_creation_input_token_cost: None,
                        })
                    });
                    (m.id, fallback)
                })
                .collect();
        }
        
        if let Some(err) = &last_error {
            eprintln!("[tokscale] OpenRouter fetch failed after {} retries: {}", MAX_RETRIES, err);
        }
        Vec::new()
    };
    
    if models_with_fallback.is_empty() {
        return HashMap::new();
    }
    
    let models_with_authors: Vec<(String, Option<ModelPricing>)> = models_with_fallback.into_iter()
        .filter(|(id, _)| get_author_provider_name(id).is_some())
        .collect();
    
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    
    let mut handles = Vec::with_capacity(models_with_authors.len());
    
    for (model_id, fallback) in models_with_authors {
        let client = Arc::clone(&client);
        let sem = Arc::clone(&semaphore);
        
        let handle = tokio::spawn(async move {
            fetch_author_pricing(client, model_id, sem, fallback).await
        });
        
        handles.push(handle);
    }
    
    // Collect results
    let mut result = HashMap::new();
    
    for handle in handles {
        if let Ok(Some((model_id, pricing))) = handle.await {
            result.insert(model_id, pricing);
        }
    }
    
    if !result.is_empty() {
        let _ = cache::save_cache(CACHE_FILENAME, &result);
    }
    
    result
}

pub async fn fetch_all_mapped() -> HashMap<String, ModelPricing> {
    fetch_all_models().await
}
