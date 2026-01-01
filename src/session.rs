//! Session storage for OAuth authentication.
//!
//! This module handles persisting and retrieving OAuth session data,
//! equivalent to the FE (AuthSessionStore) class in augment.mjs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

/// Default scopes for the session
pub const DEFAULT_SCOPES: &[&str] = &["read", "write"];

/// Session data structure stored in session.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionData {
    pub access_token: String,
    #[serde(alias = "tenantURL")]
    pub tenant_url: String,
    pub scopes: Vec<String>,
}

/// Authentication session store
///
/// Manages session persistence in ~/.augment/session.json (or a custom cache directory).
pub struct AuthSessionStore {
    session_path: PathBuf,
    is_logged_in: bool,
}

impl AuthSessionStore {
    /// Create a new session store
    ///
    /// # Arguments
    /// * `cache_dir` - Optional custom cache directory. Defaults to ~/.augment
    pub fn new(cache_dir: Option<String>) -> Result<Self> {
        let base_dir = match cache_dir {
            Some(dir) => PathBuf::from(dir),
            None => dirs::home_dir()
                .context("Could not determine home directory")?
                .join(".augment"),
        };

        // Create directory if it doesn't exist
        std::fs::create_dir_all(&base_dir)
            .with_context(|| format!("Failed to create cache directory: {:?}", base_dir))?;

        let session_path = base_dir.join("session.json");

        let mut store = Self {
            session_path,
            is_logged_in: false,
        };

        store.initialize_login_status();

        Ok(store)
    }

    /// Get the session file path
    #[allow(dead_code)]
    pub fn session_path(&self) -> &PathBuf {
        &self.session_path
    }

    /// Check if user is logged in
    pub fn is_logged_in(&self) -> bool {
        self.is_logged_in
    }

    /// Initialize login status by checking environment variable and session file
    fn initialize_login_status(&mut self) {
        // First check AUGMENT_SESSION_AUTH environment variable (JSON format)
        if let Ok(env_auth) = std::env::var("AUGMENT_SESSION_AUTH") {
            if self.parse_session_from_string(&env_auth).is_some() {
                self.is_logged_in = true;
                info!("Using authentication from AUGMENT_SESSION_AUTH environment variable");
                return;
            }
        }

        // Then check individual environment variables (AUGMENT_API_TOKEN + AUGMENT_API_URL)
        if let (Ok(token), Ok(url)) = (
            std::env::var("AUGMENT_API_TOKEN"),
            std::env::var("AUGMENT_API_URL"),
        ) {
            if !token.is_empty() && !url.is_empty() {
                self.is_logged_in = true;
                info!("Using authentication from AUGMENT_API_TOKEN + AUGMENT_API_URL environment variables");
                return;
            }
        }

        // Finally check session file
        if !self.session_path.exists() {
            self.is_logged_in = false;
            return;
        }

        match std::fs::read_to_string(&self.session_path) {
            Ok(content) => {
                if self.parse_session_from_string(&content).is_some() {
                    self.is_logged_in = true;
                } else {
                    self.is_logged_in = false;
                }
            }
            Err(e) => {
                error!("Failed to read session file: {}", e);
                self.is_logged_in = false;
            }
        }
    }

    /// Parse session data from JSON string
    fn parse_session_from_string(&self, raw: &str) -> Option<SessionData> {
        match serde_json::from_str::<SessionData>(raw) {
            Ok(session) => {
                // Validate required fields
                if session.access_token.is_empty()
                    || session.tenant_url.is_empty()
                    || session.scopes.is_empty()
                {
                    warn!("Session validation failed: missing or invalid required fields");
                    return None;
                }
                Some(session)
            }
            Err(e) => {
                warn!("Failed to parse session JSON: {}", e);
                None
            }
        }
    }

    /// Get the current session
    ///
    /// Priority:
    /// 1. AUGMENT_SESSION_AUTH environment variable (JSON format)
    /// 2. AUGMENT_API_TOKEN + AUGMENT_API_URL environment variables
    /// 3. session.json file
    pub fn get_session(&self) -> Result<Option<SessionData>> {
        // First check AUGMENT_SESSION_AUTH environment variable (JSON format)
        if let Ok(env_auth) = std::env::var("AUGMENT_SESSION_AUTH") {
            if let Some(session) = self.parse_session_from_string(&env_auth) {
                return Ok(Some(session));
            }
        }

        // Then check individual environment variables (AUGMENT_API_TOKEN + AUGMENT_API_URL)
        if let (Ok(token), Ok(url)) = (
            std::env::var("AUGMENT_API_TOKEN"),
            std::env::var("AUGMENT_API_URL"),
        ) {
            if !token.is_empty() && !url.is_empty() {
                return Ok(Some(SessionData {
                    access_token: token,
                    tenant_url: url,
                    scopes: DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect(),
                }));
            }
        }

        // Finally check session file
        if !self.session_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&self.session_path)
            .with_context(|| format!("Failed to read session file: {:?}", self.session_path))?;

        if let Some(session) = self.parse_session_from_string(&content) {
            return Ok(Some(session));
        }

        warn!("Invalid session data found, removing session file");
        let _ = self.remove_session();
        Ok(None)
    }

    /// Save a new session
    pub fn save_session(&self, access_token: &str, tenant_url: &str) -> Result<()> {
        let session = SessionData {
            access_token: access_token.to_string(),
            tenant_url: tenant_url.to_string(),
            scopes: DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect(),
        };

        let content =
            serde_json::to_string_pretty(&session).context("Failed to serialize session data")?;

        std::fs::write(&self.session_path, content)
            .with_context(|| format!("Failed to write session file: {:?}", self.session_path))?;

        // Update environment variables (for current process)
        std::env::set_var("AUGMENT_API_URL", tenant_url);
        std::env::set_var("AUGMENT_API_TOKEN", access_token);

        info!("Session saved successfully");
        debug!("Session saved to {:?}", self.session_path);

        Ok(())
    }

    /// Remove the current session
    pub fn remove_session(&self) -> Result<()> {
        if self.session_path.exists() {
            std::fs::remove_file(&self.session_path).with_context(|| {
                format!("Failed to remove session file: {:?}", self.session_path)
            })?;
        }

        info!("Session removed successfully");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Helper to temporarily clear auth environment variables for testing
    struct EnvGuard {
        session_auth: Option<String>,
        api_token: Option<String>,
        api_url: Option<String>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let guard = Self {
                session_auth: std::env::var("AUGMENT_SESSION_AUTH").ok(),
                api_token: std::env::var("AUGMENT_API_TOKEN").ok(),
                api_url: std::env::var("AUGMENT_API_URL").ok(),
            };
            std::env::remove_var("AUGMENT_SESSION_AUTH");
            std::env::remove_var("AUGMENT_API_TOKEN");
            std::env::remove_var("AUGMENT_API_URL");
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.session_auth {
                std::env::set_var("AUGMENT_SESSION_AUTH", v);
            }
            if let Some(v) = &self.api_token {
                std::env::set_var("AUGMENT_API_TOKEN", v);
            }
            if let Some(v) = &self.api_url {
                std::env::set_var("AUGMENT_API_URL", v);
            }
        }
    }

    #[test]
    fn test_session_store_new() {
        let _guard = EnvGuard::new();
        let tmp = tempdir().unwrap();
        let store = AuthSessionStore::new(Some(tmp.path().to_string_lossy().to_string())).unwrap();
        assert!(!store.is_logged_in());
    }

    #[test]
    fn test_session_save_and_load() {
        let tmp = tempdir().unwrap();
        let store = AuthSessionStore::new(Some(tmp.path().to_string_lossy().to_string())).unwrap();

        store
            .save_session("test_token", "https://test.augmentcode.com")
            .unwrap();

        let session = store.get_session().unwrap().unwrap();
        assert_eq!(session.access_token, "test_token");
        assert_eq!(session.tenant_url, "https://test.augmentcode.com");
        assert_eq!(session.scopes, vec!["read", "write"]);
    }

    #[test]
    fn test_session_remove() {
        let tmp = tempdir().unwrap();
        let store = AuthSessionStore::new(Some(tmp.path().to_string_lossy().to_string())).unwrap();

        store
            .save_session("test_token", "https://test.augmentcode.com")
            .unwrap();
        assert!(store.session_path().exists());

        store.remove_session().unwrap();
        assert!(!store.session_path().exists());
    }
}
