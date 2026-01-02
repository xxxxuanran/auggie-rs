use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, error};
use url::Url;
use uuid::Uuid;

use super::http::send_with_retry;

/// Default request timeout in seconds
pub(super) const DEFAULT_TIMEOUT_SECS: u64 = 30;

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
    pub(super) client: Client,
    pub(super) user_agent: String,
    pub(super) session_id: String,
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

    fn build_url(base_url: &str, endpoint: &str) -> Result<Url> {
        let base =
            Url::parse(base_url).with_context(|| format!("Invalid base URL: {}", base_url))?;
        base.join(endpoint)
            .with_context(|| format!("Failed to build URL for endpoint: {}", endpoint))
    }

    fn client_with_timeout(&self, timeout_secs: u64) -> Result<Client> {
        if timeout_secs == DEFAULT_TIMEOUT_SECS {
            return Ok(self.client.clone());
        }

        Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .context("Failed to build HTTP client")
    }

    pub(super) async fn post_api_with_timeout<T>(
        &self,
        endpoint: &str,
        base_url: &str,
        access_token: Option<&str>,
        body: &T,
        timeout_secs: u64,
        request_id: Option<&str>,
    ) -> Result<reqwest::Response>
    where
        T: Serialize,
    {
        let url = Self::build_url(base_url, endpoint)?;
        let request_id = request_id
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        debug!("=== API Request ===");
        debug!("URL: {}", url);
        debug!("Timeout: {}s", timeout_secs);

        let client = self.client_with_timeout(timeout_secs)?;

        send_with_retry(|| {
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
        .with_context(|| format!("Failed to send request to {}", url))
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
    ///
    /// If the request fails with 401/403, returns an error indicating
    /// the token may have expired and the user should run `auggie login`.
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
        let response = self
            .post_api_with_timeout(endpoint, base_url, access_token, body, timeout_secs, None)
            .await?;

        let status = response.status();
        debug!("=== API Response ===");
        debug!("Status: {}", status);

        if !status.is_success() {
            let http_status = status.as_u16();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            // Create a structured API error
            let api_error =
                super::types::ApiError::from_http_response(http_status, error_text.clone(), None);

            // Log with appropriate severity based on error type
            if api_error.requires_relogin {
                error!("❌ {}", api_error.message);
                error!("   {}", api_error.user_hint());
            } else if api_error.is_fatal() {
                error!("❌ {}", api_error.message);
            } else {
                error!("API request failed: {}", api_error.message);
            }

            anyhow::bail!(api_error);
        }

        let response_text = response
            .text()
            .await
            .context("Failed to read response body")?;
        serde_json::from_str(&response_text).context("Failed to parse API response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_user_agent() {
        let ua = build_user_agent();
        assert!(ua.starts_with("augment.cli/"));
    }

    #[test]
    fn test_build_url_token() {
        let url = ApiClient::build_url("https://example.augmentcode.com/", "token").unwrap();
        assert_eq!(url.as_str(), "https://example.augmentcode.com/token");

        let url = ApiClient::build_url("https://example.augmentcode.com", "token").unwrap();
        assert_eq!(url.as_str(), "https://example.augmentcode.com/token");
    }
}
