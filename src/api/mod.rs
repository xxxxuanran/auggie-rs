//! API client for Augment services.
//!
//! This module provides HTTP client functionality for communicating with
//! Augment backend services, equivalent to the Eye class in augment.mjs.

mod prompt_enhancer;
mod types;

pub use types::{
    BatchUploadBlob, BatchUploadResponse, ChatHistoryExchange, CodebaseRetrievalResponse,
    PromptEnhancerResult, ToolUseEvent,
};

// Re-export CliMode for use in other modules
pub use self::CliMode as ApiCliMode;

use anyhow::{Context, Result};
use rand::Rng;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error};
use types::{BatchUploadRequest, CodebaseRetrievalRequest, TokenRequest, TokenResponse};
use types::{RecordRequestEventsRequest, RequestEvent, ToolUseData, ToolUseEventWrapper};
use url::Url;
use uuid::Uuid;

use crate::oauth::DEFAULT_CLIENT_ID;
use crate::workspace::Checkpoint;

/// Default request timeout in seconds
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Timeout for codebase retrieval requests (120 seconds)
const CODEBASE_RETRIEVAL_TIMEOUT_SECS: u64 = 120;

/// Global retry schedule: 3 retries with exponential backoff from 1s, plus jitter.
const RETRY_BASE_DELAY_SECS: u64 = 1;
const MAX_RETRIES: usize = 3;
const RETRY_JITTER_DIVISOR: u128 = 4; // + up to 25% jitter

/// Default CLI version (from Cargo.toml)
const DEFAULT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// CLI running mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliMode {
    Mcp,
    Acp,
    Interactive,
    NonInteractive,
}

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

async fn send_with_retry(
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

impl CliMode {
    fn as_str(&self) -> &'static str {
        match self {
            CliMode::Mcp => "mcp",
            CliMode::Acp => "acp",
            CliMode::Interactive => "interactive",
            CliMode::NonInteractive => "noninteractive",
        }
    }
}

/// Build the User-Agent string for a specific mode
fn build_user_agent_with_mode(mode: CliMode) -> String {
    let version = std::env::var("AUGGIE_VERSION").unwrap_or_else(|_| DEFAULT_VERSION.to_string());
    std::env::var("AUGGIE_USER_AGENT")
        .unwrap_or_else(|_| format!("augment.cli/{}/{}", version, mode.as_str()))
}

/// Build the default User-Agent string (for backwards compatibility)
fn build_user_agent() -> String {
    let version = std::env::var("AUGGIE_VERSION").unwrap_or_else(|_| DEFAULT_VERSION.to_string());
    let mode = std::env::var("AUGGIE_MODE").unwrap_or_else(|_| "noninteractive".to_string());
    std::env::var("AUGGIE_USER_AGENT")
        .unwrap_or_else(|_| format!("augment.cli/{}/{}", version, mode))
}

/// API client for Augment services
pub struct ApiClient {
    client: Client,
    user_agent: String,
    session_id: String,
}

impl ApiClient {
    /// Create a new API client
    pub fn new(user_agent: Option<String>) -> Self {
        let user_agent = user_agent.unwrap_or_else(build_user_agent);
        let session_id = Uuid::new_v4().to_string();

        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            user_agent,
            session_id,
        }
    }

    /// Create a new API client with a specific CLI mode
    pub fn with_mode(mode: CliMode) -> Self {
        Self::new(Some(build_user_agent_with_mode(mode)))
    }

    /// Build the token endpoint URL
    fn build_token_url(tenant_url: &str) -> Result<Url> {
        let base = Url::parse(tenant_url)
            .with_context(|| format!("Invalid tenant URL: {}", tenant_url))?;
        base.join("token")
            .with_context(|| format!("Failed to build token URL from: {}", tenant_url))
    }

    /// Exchange authorization code for access token
    pub async fn get_access_token(
        &self,
        redirect_uri: &str,
        tenant_url: &str,
        code_verifier: &str,
        code: &str,
    ) -> Result<String> {
        let url = Self::build_token_url(tenant_url)?;
        let request_id = Uuid::new_v4().to_string();

        let body = TokenRequest {
            grant_type: "authorization_code".to_string(),
            client_id: DEFAULT_CLIENT_ID.to_string(),
            code_verifier: code_verifier.to_string(),
            redirect_uri: redirect_uri.to_string(),
            code: code.to_string(),
        };

        debug!("=== Token Request ===");
        debug!("URL: {}", url);

        let response = send_with_retry(|| {
            self.client
                .post(url.clone())
                .header("Content-Type", "application/json")
                .header("User-Agent", &self.user_agent)
                .header("x-request-id", &request_id)
                .header("x-request-session-id", &self.session_id)
                .json(&body)
        })
        .await
        .with_context(|| format!("Failed to send request to {}", url))?;

        let status = response.status();
        debug!("=== Token Response ===");
        debug!("Status: {}", status);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!(
                "Token request failed with status {}: {}",
                status, error_text
            );
            anyhow::bail!(
                "Token request failed with status {}: {}",
                status,
                error_text
            );
        }

        let response_text = response
            .text()
            .await
            .context("Failed to read response body")?;
        let token_response: TokenResponse =
            serde_json::from_str(&response_text).context("Failed to parse token response")?;

        if token_response.access_token.is_empty() {
            anyhow::bail!("Token response does not contain a valid 'access_token' field");
        }

        debug!("Successfully obtained access token");
        Ok(token_response.access_token)
    }

    /// Make an authenticated API request
    #[allow(dead_code)]
    pub async fn call_api<T, R>(
        &self,
        endpoint: &str,
        base_url: &str,
        access_token: Option<&str>,
        body: &T,
    ) -> Result<R>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        self.call_api_with_timeout(endpoint, base_url, access_token, body, DEFAULT_TIMEOUT_SECS)
            .await
    }

    /// Make an authenticated API request with custom timeout
    pub async fn call_api_with_timeout<T, R>(
        &self,
        endpoint: &str,
        base_url: &str,
        access_token: Option<&str>,
        body: &T,
        timeout_secs: u64,
    ) -> Result<R>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        let base =
            Url::parse(base_url).with_context(|| format!("Invalid base URL: {}", base_url))?;
        let url = base
            .join(endpoint)
            .with_context(|| format!("Failed to build URL for endpoint: {}", endpoint))?;

        let request_id = Uuid::new_v4().to_string();
        debug!("=== API Request ===");
        debug!("URL: {}", url);
        debug!("Timeout: {}s", timeout_secs);

        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .context("Failed to build HTTP client")?;

        let response = send_with_retry(|| {
            let mut request = client
                .post(url.clone())
                .header("Content-Type", "application/json")
                .header("User-Agent", &self.user_agent)
                .header("x-request-id", &request_id)
                .header("x-request-session-id", &self.session_id);

            if let Some(token) = access_token {
                request = request.header("Authorization", format!("Bearer {}", token));
            }

            request.json(body)
        })
        .await
        .with_context(|| format!("Failed to send request to {}", url))?;

        let status = response.status();
        debug!("=== API Response ===");
        debug!("Status: {}", status);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("API request failed with status {}: {}", status, error_text);
            anyhow::bail!("API request failed with status {}: {}", status, error_text);
        }

        let response_text = response
            .text()
            .await
            .context("Failed to read response body")?;
        serde_json::from_str(&response_text).context("Failed to parse API response")
    }

    /// Call the batch-upload endpoint to upload file blobs
    pub async fn batch_upload(
        &self,
        tenant_url: &str,
        access_token: &str,
        blobs: Vec<BatchUploadBlob>,
    ) -> Result<BatchUploadResponse> {
        if blobs.is_empty() {
            return Ok(BatchUploadResponse {
                blob_names: Vec::new(),
            });
        }

        let request_body = BatchUploadRequest { blobs };
        self.call_api_with_timeout(
            "batch-upload",
            tenant_url,
            Some(access_token),
            &request_body,
            CODEBASE_RETRIEVAL_TIMEOUT_SECS,
        )
        .await
    }

    /// Call the agents/codebase-retrieval endpoint
    pub async fn agent_codebase_retrieval(
        &self,
        tenant_url: &str,
        access_token: &str,
        information_request: String,
        checkpoint: Checkpoint,
    ) -> Result<CodebaseRetrievalResponse> {
        let request_body = CodebaseRetrievalRequest {
            information_request,
            blobs: checkpoint,
            dialog: Vec::new(),
            max_output_length: 0,
            disable_codebase_retrieval: false,
            enable_commit_retrieval: false,
        };

        self.call_api_with_timeout(
            "agents/codebase-retrieval",
            tenant_url,
            Some(access_token),
            &request_body,
            CODEBASE_RETRIEVAL_TIMEOUT_SECS,
        )
        .await
    }

    /// Call the prompt-enhancer endpoint
    pub async fn prompt_enhancer(
        &self,
        tenant_url: &str,
        access_token: &str,
        prompt: String,
        chat_history: Option<Vec<ChatHistoryExchange>>,
        conversation_id: Option<String>,
        model: Option<String>,
    ) -> Result<PromptEnhancerResult> {
        prompt_enhancer::call_prompt_enhancer(
            &self.user_agent,
            &self.session_id,
            tenant_url,
            access_token,
            prompt,
            chat_history,
            conversation_id,
            model,
        )
        .await
    }

    /// Record request events for telemetry
    ///
    /// This sends tool use events to the Augment backend for analytics.
    /// Events are grouped by request_id and sent in batches.
    pub async fn record_request_events(
        &self,
        tenant_url: &str,
        access_token: &str,
        events: Vec<ToolUseEvent>,
    ) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        // Group events by request_id
        let mut grouped: std::collections::HashMap<String, Vec<&ToolUseEvent>> =
            std::collections::HashMap::new();
        for event in &events {
            grouped
                .entry(event.request_id.clone())
                .or_default()
                .push(event);
        }

        // Send each group as a separate request
        for (request_id, group) in grouped {
            let request_events: Vec<RequestEvent> = group
                .into_iter()
                .map(|e| {
                    let tool_input_json = &e.tool_input;
                    RequestEvent {
                        time: e.event_time.to_rfc3339(),
                        event: ToolUseEventWrapper {
                            tool_use_data: ToolUseData {
                                tool_name: e.tool_name.clone(),
                                tool_use_id: e.tool_use_id.clone(),
                                tool_output_is_error: e.tool_output_is_error,
                                tool_run_duration_ms: e.tool_run_duration_ms,
                                tool_input: tool_input_json.clone(),
                                tool_input_len: tool_input_json.len(),
                                is_mcp_tool: e.is_mcp_tool,
                                conversation_id: e.conversation_id.clone(),
                                chat_history_length: e.chat_history_length,
                                tool_output_len: e.tool_output_len,
                                tool_lines_added: e.tool_lines_added,
                                tool_lines_deleted: e.tool_lines_deleted,
                                tool_use_diff: e.tool_use_diff.clone(),
                            },
                        },
                    }
                })
                .collect();

            let request_body = RecordRequestEventsRequest {
                events: request_events,
            };

            // Use the request_id as the x-request-id header
            let base = Url::parse(tenant_url)
                .with_context(|| format!("Invalid tenant URL: {}", tenant_url))?;
            let url = base
                .join("record-request-events")
                .context("Failed to build URL for record-request-events")?;

            debug!("Sending {} events to record-request-events", events.len());

            let response = send_with_retry(|| {
                self.client
                    .post(url.clone())
                    .header("Content-Type", "application/json")
                    .header("User-Agent", &self.user_agent)
                    .header("x-request-id", &request_id)
                    .header("x-request-session-id", &self.session_id)
                    .header("Authorization", format!("Bearer {}", access_token))
                    .json(&request_body)
            })
            .await
            .with_context(|| format!("Failed to send request to {}", url))?;

            let status = response.status();
            if !status.is_success() {
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                error!(
                    "record-request-events failed with status {}: {}",
                    status, error_text
                );
                // Don't fail the whole operation for telemetry errors
            } else {
                debug!("Successfully sent telemetry events for request {}", request_id);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_token_url() {
        let url = ApiClient::build_token_url("https://example.augmentcode.com/").unwrap();
        assert_eq!(url.as_str(), "https://example.augmentcode.com/token");

        let url = ApiClient::build_token_url("https://example.augmentcode.com").unwrap();
        assert_eq!(url.as_str(), "https://example.augmentcode.com/token");
    }

    #[test]
    fn test_build_user_agent() {
        let ua = build_user_agent();
        assert!(ua.starts_with("augment.cli/"));
    }
}
