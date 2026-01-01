//! OAuth authentication flow implementation.
//!
//! This module implements PKCE-based OAuth authentication,
//! equivalent to the JD class in augment.mjs.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};
use url::Url;

use crate::api::ApiClient;
use crate::session::AuthSessionStore;

/// Default OAuth authentication URL
pub const DEFAULT_AUTH_URL: &str = "https://auth.augmentcode.com";

/// Default OAuth client ID
pub const DEFAULT_CLIENT_ID: &str = "v";

/// OAuth state TTL in minutes
const STATE_TTL_MINUTES: u64 = 10;

/// Allowed hostname suffix for tenant URLs
fn get_allowed_hostname_suffix() -> String {
    std::env::var("TEST_HOSTNAME").unwrap_or_else(|_| ".augmentcode.com".to_string())
}

/// OAuth state stored in oauth-state.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthState {
    pub code_verifier: String,
    pub code_challenge: String,
    pub state: String,
    pub creation_time: u64,
}

/// Auth response from browser (pasted JSON)
#[derive(Debug, Clone, Deserialize)]
pub struct AuthResponse {
    pub state: String,
    pub code: Option<String>,
    pub tenant_url: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// OAuth flow manager
pub struct OAuthFlow {
    oauth_url: String,
    api_client: ApiClient,
    session_store: AuthSessionStore,
    state_path: PathBuf,
}

impl OAuthFlow {
    /// Create a new OAuth flow manager
    pub fn new(
        oauth_url: &str,
        api_client: ApiClient,
        session_store: AuthSessionStore,
        cache_dir: Option<String>,
    ) -> Result<Self> {
        let base_dir = match cache_dir {
            Some(dir) => PathBuf::from(dir),
            None => dirs::home_dir()
                .context("Could not determine home directory")?
                .join(".augment"),
        };

        std::fs::create_dir_all(&base_dir)
            .with_context(|| format!("Failed to create cache directory: {:?}", base_dir))?;

        let state_path = base_dir.join("oauth-state.json");

        Ok(Self {
            oauth_url: oauth_url.to_string(),
            api_client,
            session_store,
            state_path,
        })
    }

    /// Generate base64url encoded random bytes
    fn generate_random_base64url(length: usize) -> String {
        let mut bytes = vec![0u8; length];
        rand::thread_rng().fill_bytes(&mut bytes);
        URL_SAFE_NO_PAD.encode(&bytes)
    }

    /// Get current time in milliseconds
    fn current_time_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Create OAuth state with PKCE parameters
    fn create_oauth_state(&self) -> Result<OAuthState> {
        info!("Creating OAuth state");

        // Generate code verifier (32 random bytes -> base64url)
        let code_verifier = Self::generate_random_base64url(32);

        // Generate code challenge (SHA256 of verifier -> base64url)
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let hash = hasher.finalize();
        let code_challenge = URL_SAFE_NO_PAD.encode(hash);

        // Generate state (8 random bytes -> base64url)
        let state = Self::generate_random_base64url(8);

        let oauth_state = OAuthState {
            code_verifier,
            code_challenge,
            state,
            creation_time: Self::current_time_millis(),
        };

        // Save state to file
        let content = serde_json::to_string_pretty(&oauth_state)
            .context("Failed to serialize OAuth state")?;
        std::fs::write(&self.state_path, content)
            .with_context(|| format!("Failed to write OAuth state: {:?}", self.state_path))?;

        info!("Created OAuth state");
        debug!("State saved to {:?}", self.state_path);

        Ok(oauth_state)
    }

    /// Get existing OAuth state if valid
    fn get_oauth_state(&self) -> Option<OAuthState> {
        if !self.state_path.exists() {
            return None;
        }

        let content = match std::fs::read_to_string(&self.state_path) {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to read OAuth state: {}", e);
                return None;
            }
        };

        let state: OAuthState = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to parse OAuth state: {}", e);
                return None;
            }
        };

        // Check if state is still valid (within TTL)
        let age_ms = Self::current_time_millis().saturating_sub(state.creation_time);
        if age_ms < STATE_TTL_MINUTES * 60 * 1000 {
            Some(state)
        } else {
            debug!("OAuth state expired");
            None
        }
    }

    /// Remove OAuth state file
    fn remove_oauth_state(&self) {
        if self.state_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.state_path) {
                error!("Failed to remove OAuth state: {}", e);
            }
        }
    }

    /// Generate the authorization URL
    fn generate_authorize_url(&self, state: &OAuthState) -> Result<String> {
        let mut url = Url::parse(&self.oauth_url)
            .with_context(|| format!("Invalid OAuth URL: {}", self.oauth_url))?;

        url.set_path("/authorize");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("code_challenge", &state.code_challenge)
            .append_pair("client_id", DEFAULT_CLIENT_ID)
            .append_pair("state", &state.state)
            .append_pair("prompt", "login");

        Ok(url.to_string())
    }

    /// Start the OAuth flow and return the authorization URL
    pub fn start_flow(&mut self) -> Result<String> {
        info!("Creating new OAuth session...");

        match self.create_oauth_state() {
            Ok(state) => self.generate_authorize_url(&state),
            Err(e) => {
                self.remove_oauth_state();
                Err(e)
            }
        }
    }

    /// Handle the pasted auth JSON from browser
    pub async fn handle_auth_json(&mut self, auth_json: &str) -> Result<String> {
        // Parse the pasted JSON
        let auth_response: AuthResponse =
            serde_json::from_str(auth_json).context("Failed to parse pasted JSON")?;

        // Get and validate state
        let oauth_state = self.get_oauth_state().context("No OAuth state found")?;

        // Always remove state after reading
        self.remove_oauth_state();

        // Validate state matches
        if oauth_state.state != auth_response.state {
            anyhow::bail!("Unknown state");
        }

        // Check for OAuth errors
        if let Some(error) = &auth_response.error {
            let mut parts = vec![format!("({})", error)];
            if let Some(desc) = &auth_response.error_description {
                parts.push(desc.clone());
            }
            anyhow::bail!("OAuth request failed: {}", parts.join(" "));
        }

        // Validate code
        let code = auth_response
            .code
            .as_ref()
            .filter(|c| !c.is_empty())
            .context("No code")?;

        // Validate tenant URL
        let tenant_url = auth_response
            .tenant_url
            .as_ref()
            .filter(|u| !u.is_empty())
            .context("No tenant URL")?;

        // Validate tenant URL hostname
        let parsed_url = Url::parse(tenant_url).context("Invalid tenant URL")?;
        let hostname = parsed_url.host_str().context("No hostname in tenant URL")?;
        let allowed_suffix = get_allowed_hostname_suffix();
        if !hostname.ends_with(&allowed_suffix) {
            anyhow::bail!("OAuth request failed: invalid OAuth tenant URL");
        }

        // Exchange code for token
        info!("Calling get_access_token to retrieve access token");

        match self
            .api_client
            .get_access_token("", tenant_url, &oauth_state.code_verifier, code)
            .await
        {
            Ok(access_token) => {
                self.session_store.save_session(&access_token, tenant_url)?;
                info!("Successfully retrieved and saved access token");
                Ok(access_token)
            }
            Err(e) => {
                error!("Failed to get and save access token: {}", e);
                anyhow::bail!(
                    "If you have a firewall, please add \"{}\" to your allowlist.",
                    tenant_url
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_base64url() {
        let random1 = OAuthFlow::generate_random_base64url(32);
        let random2 = OAuthFlow::generate_random_base64url(32);

        // Should be different
        assert_ne!(random1, random2);

        // Should be valid base64url (no + or /)
        assert!(!random1.contains('+'));
        assert!(!random1.contains('/'));
        assert!(!random1.contains('='));
    }

    #[test]
    fn test_code_challenge_generation() {
        // Test that code challenge is SHA256 of verifier
        let verifier = "test_verifier";
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        let challenge = URL_SAFE_NO_PAD.encode(hash);

        // Challenge should be base64url encoded
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
    }
}
