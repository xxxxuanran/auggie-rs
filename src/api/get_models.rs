//! Get models API endpoint for connection validation.

use anyhow::Result;
use tracing::debug;

use super::client::ApiClient;
use super::types::{GetModelsResponse, ValidationResult};

/// Timeout for get-models requests (short, for quick validation)
const GET_MODELS_TIMEOUT_SECS: u64 = 10;

/// Timeout for validation requests (very short)
const VALIDATION_TIMEOUT_SECS: u64 = 5;

impl ApiClient {
    /// Call the get-models endpoint to validate connection and credentials.
    /// This is a lightweight endpoint that returns user info and available models.
    pub async fn get_models(
        &self,
        tenant_url: &str,
        access_token: &str,
    ) -> Result<GetModelsResponse> {
        // get-models uses an empty object as request body
        let request_body = serde_json::json!({});

        self.call_api_with_timeout(
            "get-models",
            tenant_url,
            Some(access_token),
            &request_body,
            GET_MODELS_TIMEOUT_SECS,
        )
        .await
    }

    /// Quick validation of connection and credentials.
    ///
    /// This is a lightweight check that validates:
    /// - URL is valid and reachable
    /// - Access token is valid
    /// - Server returns successful response
    ///
    /// Returns a `ValidationResult` indicating the status.
    pub async fn validate_connection(
        &self,
        tenant_url: &str,
        access_token: &str,
    ) -> ValidationResult {
        debug!("Validating connection to {}", tenant_url);

        // Attempt a quick get-models call
        let request_body = serde_json::json!({});

        match self
            .post_api_with_timeout(
                "get-models",
                tenant_url,
                Some(access_token),
                &request_body,
                VALIDATION_TIMEOUT_SECS,
                None,
            )
            .await
        {
            Ok(response) => {
                let status = response.status();

                if status.is_success() {
                    ValidationResult::Ok
                } else if status.as_u16() == 401 || status.as_u16() == 403 {
                    let msg = format!(
                        "Authentication failed (HTTP {}). Token may have expired.",
                        status.as_u16()
                    );
                    ValidationResult::InvalidCredentials(msg)
                } else if status.is_server_error() {
                    let msg = format!("Server error (HTTP {})", status.as_u16());
                    ValidationResult::ServerError(msg)
                } else {
                    let msg = format!("Unexpected response (HTTP {})", status.as_u16());
                    ValidationResult::ConnectionError(msg)
                }
            }
            Err(e) => {
                let err_str = e.to_string().to_lowercase();

                if err_str.contains("invalid url")
                    || err_str.contains("url parse")
                    || err_str.contains("relative url")
                {
                    ValidationResult::InvalidUrl(format!("Invalid URL: {}", tenant_url))
                } else if err_str.contains("dns")
                    || err_str.contains("resolve")
                    || err_str.contains("connect")
                    || err_str.contains("network")
                {
                    ValidationResult::ConnectionError(format!(
                        "Cannot connect to {}: {}",
                        tenant_url, e
                    ))
                } else {
                    ValidationResult::ConnectionError(e.to_string())
                }
            }
        }
    }
}
