//! Startup ensure mechanism for validating authentication, API, and feature flags.
//!
//! This implements a flow similar to augment.mjs's ensure() pattern:
//! ```text
//! await a.auth.ensure()           // Validate credentials exist
//! await a.api.ensure()            // Validate API connection via get-models
//! await a.featureFlags.ensure()   // Load feature flags and model config
//! await a.metadata.updateSession() // Update session metadata
//! ```
//!
//! ## Fatal Status Codes
//!
//! The following API status codes are fatal and will cause immediate exit:
//! - Status 7 (Unauthenticated): Requires re-login
//! - Status 8 (PermissionDenied): Account not authorized for CLI/MCP
//! - Status 12 (UpgradeRequired): Client version too old

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{debug, error, info, warn};

use crate::api::{ApiCliMode, ApiClient, ApiStatus, GetModelsResponse, ValidationResult};

use super::model_resolver::{
    parse_model_info_registry, resolve_model_with_fallback, ModelInfoRegistry,
};
use crate::metadata::MetadataManager;
use crate::session::{AuthSessionStore, SessionData};

/// Error types for the ensure mechanism
#[derive(Debug, Clone)]
pub enum EnsureError {
    /// No authentication credentials found
    NotLoggedIn,
    /// Invalid or expired credentials (status 7 - unauthenticated)
    InvalidCredentials(String),
    /// Cannot reach the server
    ConnectionError(String),
    /// Server error (5xx)
    ServerError(String),
    /// Invalid URL configuration
    InvalidUrl(String),
    /// Account disabled for this mode (status 8 - permissionDenied)
    AccountDisabled(String),
    /// Client version too old (status 12 - augmentUpgradeRequired)
    UpgradeRequired(String),
    /// Rate limit exceeded (status 6 - resourceExhausted)
    RateLimited(String),
    /// Feature flag indicates mode is disabled
    ModeDisabled(String),
    /// Other error
    Other(String),
}

impl EnsureError {
    /// Create an EnsureError from an ApiStatus
    pub fn from_api_status(status: ApiStatus) -> Self {
        match status {
            ApiStatus::Unauthenticated => {
                EnsureError::InvalidCredentials(status.error_message().to_string())
            }
            ApiStatus::PermissionDenied => {
                EnsureError::AccountDisabled(status.error_message().to_string())
            }
            ApiStatus::AugmentUpgradeRequired => {
                EnsureError::UpgradeRequired(status.error_message().to_string())
            }
            ApiStatus::ResourceExhausted => {
                EnsureError::RateLimited(status.error_message().to_string())
            }
            ApiStatus::Unavailable => EnsureError::ServerError(status.error_message().to_string()),
            _ => EnsureError::Other(status.error_message().to_string()),
        }
    }
}

impl std::fmt::Display for EnsureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnsureError::NotLoggedIn => write!(f, "Not logged in"),
            EnsureError::InvalidCredentials(msg) => write!(f, "Invalid credentials: {}", msg),
            EnsureError::ConnectionError(msg) => write!(f, "Connection error: {}", msg),
            EnsureError::ServerError(msg) => write!(f, "Server error: {}", msg),
            EnsureError::InvalidUrl(msg) => write!(f, "Invalid URL: {}", msg),
            EnsureError::AccountDisabled(msg) => write!(f, "Account disabled: {}", msg),
            EnsureError::UpgradeRequired(msg) => write!(f, "Upgrade required: {}", msg),
            EnsureError::RateLimited(msg) => write!(f, "Rate limited: {}", msg),
            EnsureError::ModeDisabled(msg) => write!(f, "Mode disabled: {}", msg),
            EnsureError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for EnsureError {}

/// Result type for ensure operations
pub type EnsureResult<T> = std::result::Result<T, EnsureError>;

/// State of each ensure component
#[derive(Debug, Clone, Default)]
pub enum EnsureStatus {
    #[default]
    NotStarted,
    InProgress,
    Success,
    Failed(String),
}

impl EnsureStatus {
    pub fn is_success(&self) -> bool {
        matches!(self, EnsureStatus::Success)
    }
}

/// Startup state containing all validated data
#[derive(Debug, Clone)]
pub struct StartupState {
    /// Validated session data
    pub session: SessionData,
    /// Model configuration from get-models
    pub model_config: GetModelsResponse,
    /// Parsed model info registry (from feature_flags.model_info_registry)
    model_info_registry: Option<ModelInfoRegistry>,
}

impl StartupState {
    /// Create a new StartupState with parsed model_info_registry
    pub fn new(session: SessionData, model_config: GetModelsResponse) -> Self {
        // Parse model_info_registry from feature_flags
        let model_info_registry = model_config
            .feature_flags
            .other
            .get("model_info_registry")
            .and_then(|v| v.as_str())
            .and_then(parse_model_info_registry);

        if let Some(ref registry) = model_info_registry {
            debug!("Loaded {} models from model_info_registry", registry.len());
        }

        Self {
            session,
            model_config,
            model_info_registry,
        }
    }

    /// Get the tenant URL
    pub fn tenant_url(&self) -> &str {
        &self.session.tenant_url
    }

    /// Get the access token
    pub fn access_token(&self) -> &str {
        &self.session.access_token
    }

    /// Check if a feature is enabled
    pub fn is_feature_enabled(&self, flag: &str) -> bool {
        self.model_config.is_feature_enabled(flag)
    }

    /// Get the default model ID
    pub fn default_model(&self) -> Option<&str> {
        self.model_config.get_default_model()
    }

    /// Get user email if available
    pub fn user_email(&self) -> Option<&str> {
        self.model_config.user.as_ref().map(|u| u.email.as_str())
    }

    /// Get user tier if available
    pub fn user_tier(&self) -> Option<&str> {
        self.model_config.user_tier.as_deref()
    }

    /// Get the model info registry
    pub fn model_info_registry(&self) -> Option<&ModelInfoRegistry> {
        self.model_info_registry.as_ref()
    }

    /// Resolve a user-provided model string to a model ID.
    ///
    /// Matching priority (same as augment.mjs):
    /// 1. Match by shortName (e.g., "sonnet4.5" -> "claude-sonnet-4-5")
    /// 2. Match by full id (e.g., "claude-sonnet-4-5")
    /// 3. Fall back to default if not found or invalid
    ///
    /// Returns None if no user input and should use API default.
    pub fn resolve_model(&self, user_input: Option<&str>) -> Option<String> {
        match &self.model_info_registry {
            Some(registry) => {
                resolve_model_with_fallback(user_input, registry, self.default_model())
            }
            None => {
                // No registry available
                if let Some(input) = user_input {
                    if !input.trim().is_empty() {
                        warn!(
                            "model_info_registry not available, ignoring --model={}",
                            input
                        );
                    }
                }
                None
            }
        }
    }
}

/// Startup context that manages the ensure mechanism.
///
/// This context orchestrates the startup validation flow:
/// 1. auth.ensure() - Validate that credentials exist
/// 2. api.ensure() - Validate API connection via get-models
/// 3. featureFlags.ensure() - Load and validate feature flags
/// 4. metadata.updateSession() - Update session metadata
///
/// # Example
/// ```ignore
/// let ctx = StartupContext::new(CliMode::Mcp, None)?;
/// let state = ctx.ensure_all().await?;
/// // Now safe to start MCP server with validated state
/// ```
pub struct StartupContext {
    mode: ApiCliMode,
    cache_dir: Option<String>,
    session_store: AuthSessionStore,
    metadata_manager: MetadataManager,
    api_client: Arc<ApiClient>,
    auth_status: EnsureStatus,
    api_status: EnsureStatus,
    feature_flags_status: EnsureStatus,
}

impl StartupContext {
    /// Create a new startup context
    pub fn new(mode: ApiCliMode, cache_dir: Option<String>) -> Result<Self> {
        let session_store = AuthSessionStore::new(cache_dir.clone())
            .context("Failed to initialize session store")?;

        let metadata_manager = MetadataManager::new(cache_dir.clone())
            .context("Failed to initialize metadata manager")?;

        let api_client = Arc::new(ApiClient::with_mode(mode));

        Ok(Self {
            mode,
            cache_dir,
            session_store,
            metadata_manager,
            api_client,
            auth_status: EnsureStatus::NotStarted,
            api_status: EnsureStatus::NotStarted,
            feature_flags_status: EnsureStatus::NotStarted,
        })
    }

    /// Get the API client
    pub fn api_client(&self) -> Arc<ApiClient> {
        self.api_client.clone()
    }

    /// Ensure authentication is available
    async fn ensure_auth(&mut self) -> EnsureResult<SessionData> {
        info!("🔐 Checking authentication...");
        self.auth_status = EnsureStatus::InProgress;

        if !self.session_store.is_logged_in() {
            error!("❌ Not logged in");
            error!("   Please run 'auggie login' or set AUGMENT_SESSION_AUTH environment variable");
            self.auth_status = EnsureStatus::Failed("Not logged in".to_string());
            return Err(EnsureError::NotLoggedIn);
        }

        let session = self
            .session_store
            .get_session()
            .map_err(|e| {
                let msg = format!("Failed to read session: {}", e);
                self.auth_status = EnsureStatus::Failed(msg.clone());
                EnsureError::Other(msg)
            })?
            .ok_or_else(|| {
                self.auth_status = EnsureStatus::Failed("No session found".to_string());
                EnsureError::NotLoggedIn
            })?;

        info!("✅ Authentication credentials found");
        debug!("   Tenant URL: {}", session.tenant_url);
        self.auth_status = EnsureStatus::Success;

        Ok(session)
    }

    /// Ensure API connection is valid via get-models
    async fn ensure_api(&mut self, session: &SessionData) -> EnsureResult<GetModelsResponse> {
        info!("🔗 Validating API connection via get-models...");
        self.api_status = EnsureStatus::InProgress;

        // First do a quick validation
        match self
            .api_client
            .validate_connection(&session.tenant_url, &session.access_token)
            .await
        {
            ValidationResult::Ok => {
                debug!("Quick validation passed");
            }
            ValidationResult::InvalidCredentials(msg) => {
                error!("❌ {}", msg);
                error!("   Please check AUGMENT_API_TOKEN or run 'auggie login'");
                self.api_status = EnsureStatus::Failed(msg.clone());
                return Err(EnsureError::InvalidCredentials(msg));
            }
            ValidationResult::ConnectionError(msg) => {
                error!("❌ {}", msg);
                error!("   Please check AUGMENT_API_URL and network connection");
                self.api_status = EnsureStatus::Failed(msg.clone());
                return Err(EnsureError::ConnectionError(msg));
            }
            ValidationResult::ServerError(msg) => {
                error!("❌ {}", msg);
                error!("   Augment service may be temporarily unavailable");
                self.api_status = EnsureStatus::Failed(msg.clone());
                return Err(EnsureError::ServerError(msg));
            }
            ValidationResult::InvalidUrl(msg) => {
                error!("❌ {}", msg);
                error!("   Please check AUGMENT_API_URL configuration");
                self.api_status = EnsureStatus::Failed(msg.clone());
                return Err(EnsureError::InvalidUrl(msg));
            }
        }

        // Now get full model config
        let model_config = self
            .api_client
            .get_models(&session.tenant_url, &session.access_token)
            .await
            .map_err(|e| {
                let msg = format!("Failed to get model config: {}", e);
                self.api_status = EnsureStatus::Failed(msg.clone());
                EnsureError::ConnectionError(msg)
            })?;

        info!("✅ API connection validated");
        self.api_status = EnsureStatus::Success;

        Ok(model_config)
    }

    /// Ensure feature flags are loaded and mode is allowed
    ///
    /// Checks for fatal status codes from the API:
    /// - Status 7 (Unauthenticated): Requires re-login
    /// - Status 8 (PermissionDenied): Account not authorized
    /// - Status 12 (UpgradeRequired): Client version too old
    async fn ensure_feature_flags(&mut self, model_config: &GetModelsResponse) -> EnsureResult<()> {
        info!("🏁 Checking feature flags...");
        self.feature_flags_status = EnsureStatus::InProgress;

        // Check for fatal status codes from the API
        // See augment.mjs line 231845-231860 for the status code enum
        if let Some(status_code) = model_config.status {
            let api_status = ApiStatus::from_i32(status_code);

            if api_status.is_fatal() {
                let msg = api_status.error_message();
                error!("❌ {}", msg);
                self.feature_flags_status = EnsureStatus::Failed(msg.to_string());

                return match api_status {
                    ApiStatus::Unauthenticated => {
                        // Status 7: Authentication failed
                        error!("   Please run 'auggie login' to re-authenticate");
                        Err(EnsureError::InvalidCredentials(msg.to_string()))
                    }
                    ApiStatus::PermissionDenied => {
                        // Status 8: Account disabled for CLI/MCP mode
                        // This is the "closed beta" error in augment.mjs
                        Err(EnsureError::AccountDisabled(msg.to_string()))
                    }
                    ApiStatus::AugmentUpgradeRequired => {
                        // Status 12: Client version too old
                        error!("   Please update to the latest version of auggie");
                        Err(EnsureError::UpgradeRequired(msg.to_string()))
                    }
                    _ => Err(EnsureError::from_api_status(api_status)),
                };
            }

            // Non-fatal but notable status codes
            if api_status == ApiStatus::ResourceExhausted {
                warn!("⚠️  Rate limit exceeded. Some features may be limited.");
            }
        }

        // Check if mode is enabled via feature flags
        let mode_enabled = match self.mode {
            ApiCliMode::Mcp => model_config.is_mcp_enabled(),
            ApiCliMode::Acp => model_config.is_cli_enabled(), // ACP uses CLI flag
            _ => model_config.is_cli_enabled(),
        };

        if !mode_enabled {
            let mode_str = match self.mode {
                ApiCliMode::Mcp => "MCP",
                ApiCliMode::Acp => "ACP",
                _ => "CLI",
            };
            let msg = format!(
                "{} mode is disabled for your account. \
                Please contact your administrator.",
                mode_str
            );
            warn!("⚠️  {}", msg);
            // This is a warning, not an error - we may still proceed
            // depending on the strictness of enforcement
        }

        // Log feature flag summary
        if let Some(ref user) = model_config.user {
            info!("   User: {}", user.email);
        }
        if let Some(ref tier) = model_config.user_tier {
            info!("   Tier: {}", tier);
        }
        if let Some(ref default_model) = model_config.default_model {
            info!("   Default model: {}", default_model);
        }
        info!("   Available models: {}", model_config.models.len());

        // Log relevant feature flags
        debug!("   Feature flags:");
        debug!(
            "     - codebase_retrieval: {}",
            model_config.is_feature_enabled("enable_codebase_retrieval")
        );
        debug!(
            "     - prompt_enhancer: {}",
            model_config.is_feature_enabled("enable_prompt_enhancer")
        );
        debug!(
            "     - telemetry: {}",
            model_config.is_feature_enabled("enable_telemetry")
        );

        info!("✅ Feature flags loaded");
        self.feature_flags_status = EnsureStatus::Success;

        Ok(())
    }

    /// Run all ensure steps and return the validated startup state.
    ///
    /// This is the main entry point for the startup flow. It runs:
    /// 1. auth.ensure() - Validate credentials exist
    /// 2. api.ensure() - Validate API connection via get-models
    /// 3. featureFlags.ensure() - Load feature flags and validate mode
    /// 4. metadata.updateSession() - Update session metadata
    ///
    /// Returns a `StartupState` containing all validated data, or an error
    /// if any step fails.
    pub async fn ensure_all(&mut self) -> EnsureResult<StartupState> {
        // Step 1: Ensure auth
        let session = self.ensure_auth().await?;

        // Step 2: Ensure API (depends on auth)
        let model_config = self.ensure_api(&session).await?;

        // Step 3: Ensure feature flags (depends on api)
        self.ensure_feature_flags(&model_config).await?;

        // Step 4: Update session metadata (lastUsed, sessionCount)
        // This is equivalent to augment.mjs metadata.updateSession()
        if let Err(e) = self.metadata_manager.update_session() {
            warn!("Failed to update session metadata: {}", e);
            // Non-fatal, continue startup
        }

        Ok(StartupState::new(session, model_config))
    }

    /// Get current auth status
    pub fn auth_status(&self) -> &EnsureStatus {
        &self.auth_status
    }

    /// Get current API status
    pub fn api_status(&self) -> &EnsureStatus {
        &self.api_status
    }

    /// Get current feature flags status
    pub fn feature_flags_status(&self) -> &EnsureStatus {
        &self.feature_flags_status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ensure_error_display() {
        let err = EnsureError::NotLoggedIn;
        assert_eq!(err.to_string(), "Not logged in");

        let err = EnsureError::AccountDisabled("MCP mode disabled".to_string());
        assert!(err.to_string().contains("Account disabled"));
    }

    #[test]
    fn test_ensure_status() {
        let status = EnsureStatus::default();
        assert!(!status.is_success());

        let status = EnsureStatus::Success;
        assert!(status.is_success());

        let status = EnsureStatus::Failed("error".to_string());
        assert!(!status.is_success());
    }
}
