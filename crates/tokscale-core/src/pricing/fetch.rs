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
            Err(error) => {
                last_error = Some(format!(
                    "{source} network error: {}",
                    super::describe_error(&error)
                ))
            }
        }

        if attempt < MAX_RETRIES - 1 {
            tokio::time::sleep(Duration::from_millis(INITIAL_BACKOFF_MS * (1 << attempt))).await;
        }
    }

    Err(last_error.unwrap_or_else(|| format!("{source} fetch ended without a response")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reserve a port, then release it, so a connect to it is refused rather
    /// than timing out. Binding first is what makes the port reliably free:
    /// hardcoding one would race any other listener on the machine.
    fn refused_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("read the bound port").port();
        drop(listener);
        format!("http://127.0.0.1:{port}/pricing.json")
    }

    // @keep: this is the #1238 regression, and the assertion is deliberately
    // not on reqwest's or hyper's wording.
    //
    // `reqwest::Error`'s Display for a send failure is "error sending request
    // for url (...)" no matter what went wrong underneath, so a rejected
    // certificate, a refused connection and a DNS failure all print the same
    // line. #1238 arrived with exactly that line and could not be triaged from
    // it. What this pins is the property that made triage possible: the
    // message `get_with_retry` returns must carry more than Display does.
    #[tokio::test]
    async fn network_error_carries_the_cause_display_alone_would_drop() {
        let url = refused_url();
        let client = pricing_client().expect("build the pricing client");

        let message = get_with_retry(&client, &url, "TestSource")
            .await
            .expect_err("a connect to a closed port must fail");

        assert!(
            message.starts_with("TestSource network error: "),
            "the source label must still lead the message, got: {message}"
        );

        // The bare Display, which is what this code used to print. Anchoring on
        // "longer than Display" rather than on the literal cause text keeps a
        // dependency bump from reddening this test for rewording its prose.
        let displayed = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .expect_err("a connect to a closed port must fail")
            .to_string();
        assert!(
            message.len() > format!("TestSource network error: {displayed}").len(),
            "describe_error must add the source chain beyond Display, got: {message}"
        );
    }
}
