use anyhow::{Context, Result};
use rand::Rng;
use reqwest::StatusCode;
use std::time::Duration;
use tokio::time::sleep;
use tracing::debug;

/// Global retry schedule: 3 retries with exponential backoff from 1s, plus jitter.
const RETRY_BASE_DELAY_SECS: u64 = 1;
const MAX_RETRIES: usize = 3;
const RETRY_JITTER_DIVISOR: u128 = 4; // + up to 25% jitter

fn is_retriable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn is_retriable_send_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_body()
}

fn retry_base_delay(attempt: usize) -> Duration {
    let multiplier = 1u64.checked_shl(attempt as u32).unwrap_or(u64::MAX);
    Duration::from_secs(RETRY_BASE_DELAY_SECS.saturating_mul(multiplier))
}

fn add_jitter(delay: Duration) -> Duration {
    let max_jitter_ms = delay.as_millis() / RETRY_JITTER_DIVISOR;
    if max_jitter_ms == 0 {
        return delay;
    }

    let max_jitter_ms = std::cmp::min(max_jitter_ms, u128::from(u64::MAX)) as u64;
    let jitter_ms = rand::thread_rng().gen_range(0..=max_jitter_ms);
    delay + Duration::from_millis(jitter_ms)
}

pub(super) async fn send_with_retry(
    mut make_request: impl FnMut() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    let max_attempts = MAX_RETRIES + 1;

    for attempt in 0..max_attempts {
        match make_request().send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }

                let should_retry = is_retriable_status(status) && attempt < MAX_RETRIES;
                if should_retry {
                    let base_delay = retry_base_delay(attempt);
                    let delay = add_jitter(base_delay);
                    debug!(
                        "HTTP request failed with status {}; retrying in {:?} (base {:?}, attempt {}/{})",
                        status,
                        delay,
                        base_delay,
                        attempt + 1,
                        max_attempts
                    );
                    let _ = response.bytes().await;
                    sleep(delay).await;
                    continue;
                }

                return Ok(response);
            }
            Err(err) => {
                let should_retry = is_retriable_send_error(&err) && attempt < MAX_RETRIES;
                if should_retry {
                    let base_delay = retry_base_delay(attempt);
                    let delay = add_jitter(base_delay);
                    debug!(
                        "HTTP request error: {}; retrying in {:?} (base {:?}, attempt {}/{})",
                        err,
                        delay,
                        base_delay,
                        attempt + 1,
                        max_attempts
                    );
                    sleep(delay).await;
                    continue;
                }

                return Err(anyhow::Error::new(err)).with_context(|| {
                    format!("HTTP request failed after {} attempt(s)", attempt + 1)
                });
            }
        }
    }

    unreachable!("send_with_retry should have returned within max_attempts")
}
