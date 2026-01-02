//! Authenticated API client with stored credentials.
//!
//! This module provides `AuthenticatedClient`, a wrapper around `ApiClient` that
//! stores tenant URL and access token for the lifetime of the process.
//!
//! Benefits:
//! - No need to pass credentials to every API call
//! - HTTP/2 connection reuse (same Client instance = connection pooling)
//! - Cleaner API signatures

use std::sync::Arc;

use anyhow::Result;

use super::client::{ApiClient, CliMode};
use super::types::GetModelsResponse;
use crate::domain::Checkpoint;

/// Authenticated API client with stored credentials.
///
/// Created once at startup after successful authentication, then used
/// for all API calls throughout the process lifetime.
///
/// Internally uses the same `reqwest::Client`, enabling HTTP/2 multiplexing
/// and connection reuse for requests to the same tenant URL.
#[derive(Clone)]
pub struct AuthenticatedClient {
    inner: Arc<ApiClient>,
    tenant_url: String,
    access_token: String,
}

impl AuthenticatedClient {
    /// Create a new authenticated client.
    ///
    /// # Arguments
    /// * `mode` - CLI mode (MCP, ACP, etc.) for User-Agent
    /// * `tenant_url` - Tenant URL from authentication
    /// * `access_token` - Access token from authentication
    pub fn new(mode: CliMode, tenant_url: String, access_token: String) -> Self {
        Self {
            inner: Arc::new(ApiClient::with_mode(mode)),
            tenant_url,
            access_token,
        }
    }

    /// Create from an existing ApiClient (for testing or custom configuration).
    pub fn from_client(client: ApiClient, tenant_url: String, access_token: String) -> Self {
        Self {
            inner: Arc::new(client),
            tenant_url,
            access_token,
        }
    }

    /// Get the tenant URL.
    pub fn tenant_url(&self) -> &str {
        &self.tenant_url
    }

    /// Get the access token.
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Get the underlying ApiClient (for advanced use cases).
    pub fn inner(&self) -> &ApiClient {
        &self.inner
    }

    // ========== API Methods ==========

    /// Fetch model configuration from get-models endpoint.
    pub async fn get_models(&self) -> Result<GetModelsResponse> {
        self.inner
            .get_models(&self.tenant_url, &self.access_token)
            .await
    }

    /// Perform batch upload of blobs.
    pub async fn batch_upload(
        &self,
        blobs: Vec<super::types::BatchUploadBlob>,
    ) -> Result<super::types::BatchUploadResponse> {
        self.inner
            .batch_upload(&self.tenant_url, &self.access_token, blobs)
            .await
    }

    /// Perform codebase retrieval search.
    pub async fn codebase_retrieval(
        &self,
        query: &str,
        checkpoint: Checkpoint,
    ) -> Result<super::types::CodebaseRetrievalResponse> {
        self.inner
            .agents()
            .codebase_retrieval(
                &self.tenant_url,
                &self.access_token,
                query.to_string(),
                checkpoint,
            )
            .await
    }

    /// Enhance a prompt using the prompt enhancer endpoint.
    pub async fn prompt_enhancer(
        &self,
        prompt: String,
        chat_history: Option<Vec<super::types::ChatHistoryExchange>>,
        conversation_id: Option<String>,
        model: Option<String>,
        checkpoint: Option<Checkpoint>,
    ) -> Result<super::types::PromptEnhancerResult> {
        self.inner
            .prompt_enhancer(
                &self.tenant_url,
                &self.access_token,
                prompt,
                chat_history,
                conversation_id,
                model,
                checkpoint,
            )
            .await
    }

    /// Record tool use events for telemetry.
    pub async fn record_request_events(
        &self,
        events: Vec<super::types::ToolUseEvent>,
    ) -> Result<()> {
        self.inner
            .record_request_events(&self.tenant_url, &self.access_token, events)
            .await
    }
}

impl std::fmt::Debug for AuthenticatedClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthenticatedClient")
            .field("tenant_url", &self.tenant_url)
            .field("access_token", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_authenticated_client_creation() {
        let client = AuthenticatedClient::new(
            CliMode::Mcp,
            "https://test.augmentcode.com".to_string(),
            "test-token".to_string(),
        );

        assert_eq!(client.tenant_url(), "https://test.augmentcode.com");
        assert_eq!(client.access_token(), "test-token");
    }

    #[test]
    fn test_debug_redacts_token() {
        let client = AuthenticatedClient::new(
            CliMode::Mcp,
            "https://test.augmentcode.com".to_string(),
            "secret-token-123".to_string(),
        );

        let debug_str = format!("{:?}", client);
        assert!(!debug_str.contains("secret-token-123"));
        assert!(debug_str.contains("[REDACTED]"));
    }
}
