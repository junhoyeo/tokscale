use reqwest::{Client, Response, StatusCode};
use std::time::Duration;

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 200;

pub(crate) fn pricing_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())
}

pub(crate) async fn get_with_retry(
    client: &Client,
    url: &str,
    source: &str,
) -> Result<Response, String> {
    let mut last_error = None;

    for attempt in 0..MAX_RETRIES {
        match client.get(url).send().await {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                if !status.is_server_error() && status != StatusCode::TOO_MANY_REQUESTS {
                    return Err(format!("{source} HTTP {status}"));
                }
                last_error = Some(format!("{source} HTTP {status}"));
            }
            Err(error) => last_error = Some(format!("{source} network error: {error}")),
        }

        if attempt < MAX_RETRIES - 1 {
            tokio::time::sleep(Duration::from_millis(INITIAL_BACKOFF_MS * (1 << attempt))).await;
        }
    }

    Err(last_error.unwrap_or_else(|| format!("{source} fetch ended without a response")))
}
