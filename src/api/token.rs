use anyhow::Result;
use tracing::debug;

use super::client::ApiClient;
use super::types::{TokenRequest, TokenResponse};
use crate::oauth::DEFAULT_CLIENT_ID;

impl ApiClient {
    /// Exchange authorization code for access token
    pub async fn get_access_token(
        &self,
        redirect_uri: &str,
        tenant_url: &str,
        code_verifier: &str,
        code: &str,
    ) -> Result<String> {
        let body = TokenRequest {
            grant_type: "authorization_code".to_string(),
            client_id: DEFAULT_CLIENT_ID.to_string(),
            code_verifier: code_verifier.to_string(),
            redirect_uri: redirect_uri.to_string(),
            code: code.to_string(),
        };

        debug!("=== Token Request ===");
        let token_response: TokenResponse = self.call_api("token", tenant_url, None, &body).await?;

        if token_response.access_token.is_empty() {
            anyhow::bail!("Token response does not contain a valid 'access_token' field");
        }

        debug!("Successfully obtained access token");
        Ok(token_response.access_token)
    }
}
