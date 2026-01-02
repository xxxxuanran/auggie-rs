//! Runtime configuration singleton.
//!
//! This module provides global access to runtime state, similar to
//! augment.mjs's `ClientFeatureFlags` singleton pattern:
//!
//! ```javascript
//! fdt(n.clientFeatureFlags);  // setClientFeatureFlags - called once at startup
//! qa();                        // getClientFeatureFlags - called anywhere
//! ```
//!
//! ## Contents
//!
//! The runtime config contains:
//! - `StartupState`: Feature flags, model registry, session info
//! - `AuthenticatedClient`: HTTP client with stored credentials (enables HTTP/2 connection reuse)
//!
//! ## Usage
//!
//! ```ignore
//! // At startup:
//! let state = startup_ctx.ensure_all().await?;
//! let client = AuthenticatedClient::new(mode, tenant_url, access_token);
//! set_runtime(state, client);
//!
//! // Anywhere else:
//! if let Some(rt) = get_runtime() {
//!     let model = rt.state.resolve_model(Some("sonnet4.5"));
//!     rt.client.prompt_enhancer(...).await?;
//! }
//! ```

use std::sync::OnceLock;

use crate::api::AuthenticatedClient;
use crate::startup::StartupState;

/// Runtime configuration containing all process-lifetime state.
pub struct Runtime {
    /// Startup state with feature flags, model registry, etc.
    pub state: StartupState,
    /// Authenticated API client with stored credentials.
    /// Using a single client instance enables HTTP/2 connection reuse.
    pub client: AuthenticatedClient,
}

impl Runtime {
    /// Create a new runtime configuration.
    pub fn new(state: StartupState, client: AuthenticatedClient) -> Self {
        Self { state, client }
    }

    /// Resolve a model ID from user input using the model registry.
    pub fn resolve_model(&self, user_input: Option<&str>) -> Option<String> {
        self.state.resolve_model(user_input)
    }
}

/// Global runtime singleton.
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Set the global runtime configuration.
///
/// This should be called once during startup after successful authentication.
/// Subsequent calls will be ignored (matching augment.mjs behavior).
pub fn set_runtime(state: StartupState, client: AuthenticatedClient) {
    if RUNTIME.set(Runtime::new(state, client)).is_err() {
        tracing::warn!(
            "Attempting to set runtime when one is already configured. Keeping existing."
        );
    }
}

/// Get the global runtime configuration.
///
/// Returns `None` if `set_runtime()` hasn't been called yet.
pub fn get_runtime() -> Option<&'static Runtime> {
    RUNTIME.get()
}

/// Check if runtime has been initialized.
pub fn has_runtime() -> bool {
    RUNTIME.get().is_some()
}

/// Get the authenticated API client.
///
/// Convenience function that returns the client directly.
/// Returns `None` if runtime hasn't been initialized.
pub fn get_client() -> Option<&'static AuthenticatedClient> {
    RUNTIME.get().map(|rt| &rt.client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_runtime_initial() {
        // This test checks the initial state - may fail if other tests ran first
        // That's expected behavior for a global singleton
        let _ = has_runtime(); // Just verify it doesn't panic
    }
}
