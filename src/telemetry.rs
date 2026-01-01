//! Telemetry module for collecting and reporting usage events.
//!
//! This module provides functionality for collecting tool use events
//! and periodically uploading them to the Augment backend.

use crate::api::{ApiClient, ToolUseEvent};
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Environment variable to disable non-essential traffic (telemetry)
pub const DISABLE_TELEMETRY_ENV: &str = "AUGMENT_DISABLE_NONESSENTIAL_TRAFFIC";

/// Check if telemetry is enabled
pub fn is_telemetry_enabled() -> bool {
    match std::env::var(DISABLE_TELEMETRY_ENV) {
        Ok(val) => {
            let val_lower = val.to_lowercase();
            // Disabled if set to "1", "true", "yes", "on"
            !matches!(val_lower.as_str(), "1" | "true" | "yes" | "on")
        }
        // Enabled by default if env var is not set
        Err(_) => true,
    }
}

/// Telemetry reporter for collecting and sending tool use events
#[derive(Clone)]
pub struct TelemetryReporter {
    events: Arc<RwLock<Vec<ToolUseEvent>>>,
    enabled: bool,
}

impl TelemetryReporter {
    /// Create a new telemetry reporter
    pub fn new() -> Self {
        let enabled = is_telemetry_enabled();
        if !enabled {
            debug!("Telemetry disabled via {}", DISABLE_TELEMETRY_ENV);
        }
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
            enabled,
        }
    }

    /// Check if telemetry is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Record a tool use event
    pub async fn record_tool_use(
        &self,
        request_id: String,
        tool_name: String,
        tool_use_id: String,
        tool_input: serde_json::Value,
        tool_output_is_error: bool,
        tool_run_duration_ms: u64,
        is_mcp_tool: bool,
        conversation_id: Option<String>,
        tool_output_len: Option<usize>,
    ) {
        if !self.enabled {
            return;
        }

        let tool_input_str = serde_json::to_string(&tool_input).unwrap_or_default();

        let event = ToolUseEvent {
            request_id,
            tool_name,
            tool_use_id,
            tool_input: tool_input_str,
            tool_output_is_error,
            tool_run_duration_ms,
            is_mcp_tool,
            conversation_id,
            chat_history_length: Some(0),
            tool_output_len,
            tool_lines_added: None,
            tool_lines_deleted: None,
            tool_use_diff: None,
            event_time: Utc::now(),
        };

        let mut events = self.events.write().await;
        events.push(event);
        debug!("Recorded telemetry event, total pending: {}", events.len());
    }

    /// Flush all pending events to the server
    pub async fn flush(&self, api_client: &ApiClient, tenant_url: &str, access_token: &str) {
        if !self.enabled {
            return;
        }

        let events = {
            let mut events = self.events.write().await;
            std::mem::take(&mut *events)
        };

        if events.is_empty() {
            return;
        }

        debug!("Flushing {} telemetry events", events.len());

        if let Err(e) = api_client
            .record_request_events(tenant_url, access_token, events)
            .await
        {
            warn!("Failed to send telemetry events: {}", e);
            // Don't re-queue events on failure to avoid unbounded growth
        }
    }

    /// Get the number of pending events
    pub async fn pending_count(&self) -> usize {
        self.events.read().await.len()
    }
}

impl Default for TelemetryReporter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarRestore {
        prev: Option<String>,
    }

    impl EnvVarRestore {
        fn new() -> Self {
            Self {
                prev: std::env::var(DISABLE_TELEMETRY_ENV).ok(),
            }
        }
    }

    impl Drop for EnvVarRestore {
        fn drop(&mut self) {
            match &self.prev {
                Some(value) => std::env::set_var(DISABLE_TELEMETRY_ENV, value),
                None => std::env::remove_var(DISABLE_TELEMETRY_ENV),
            }
        }
    }

    #[test]
    fn test_is_telemetry_enabled_default() {
        let _env_lock_guard = env_lock().lock().unwrap();
        let _env_restore = EnvVarRestore::new();
        // When env var is not set, telemetry should be enabled
        std::env::remove_var(DISABLE_TELEMETRY_ENV);
        assert!(is_telemetry_enabled());
    }

    #[test]
    fn test_is_telemetry_disabled() {
        let _env_lock_guard = env_lock().lock().unwrap();
        let _env_restore = EnvVarRestore::new();
        std::env::set_var(DISABLE_TELEMETRY_ENV, "1");
        assert!(!is_telemetry_enabled());

        std::env::set_var(DISABLE_TELEMETRY_ENV, "true");
        assert!(!is_telemetry_enabled());

        std::env::set_var(DISABLE_TELEMETRY_ENV, "TRUE");
        assert!(!is_telemetry_enabled());

        std::env::set_var(DISABLE_TELEMETRY_ENV, "yes");
        assert!(!is_telemetry_enabled());

        std::env::set_var(DISABLE_TELEMETRY_ENV, "on");
        assert!(!is_telemetry_enabled());

        // Clean up
        std::env::remove_var(DISABLE_TELEMETRY_ENV);
    }

    #[test]
    fn test_is_telemetry_enabled_with_other_values() {
        let _env_lock_guard = env_lock().lock().unwrap();
        let _env_restore = EnvVarRestore::new();
        std::env::set_var(DISABLE_TELEMETRY_ENV, "0");
        assert!(is_telemetry_enabled());

        std::env::set_var(DISABLE_TELEMETRY_ENV, "false");
        assert!(is_telemetry_enabled());

        std::env::set_var(DISABLE_TELEMETRY_ENV, "no");
        assert!(is_telemetry_enabled());

        // Clean up
        std::env::remove_var(DISABLE_TELEMETRY_ENV);
    }

    #[tokio::test]
    async fn test_telemetry_reporter_disabled() {
        let _env_lock_guard = env_lock().lock().unwrap();
        let _env_restore = EnvVarRestore::new();
        // Clean up first to ensure clean state (tests may run in parallel)
        std::env::remove_var(DISABLE_TELEMETRY_ENV);

        // Now set to disable
        std::env::set_var(DISABLE_TELEMETRY_ENV, "1");
        let reporter = TelemetryReporter::new();
        assert!(!reporter.is_enabled());

        // Recording should be no-op when disabled
        reporter
            .record_tool_use(
                "req-1".to_string(),
                "test-tool".to_string(),
                "use-1".to_string(),
                serde_json::json!({"test": "input"}),
                false,
                100,
                true,
                None,
                Some(50),
            )
            .await;

        assert_eq!(reporter.pending_count().await, 0);

        // Clean up
        std::env::remove_var(DISABLE_TELEMETRY_ENV);
    }

    #[tokio::test]
    async fn test_telemetry_reporter_enabled() {
        let _env_lock_guard = env_lock().lock().unwrap();
        let _env_restore = EnvVarRestore::new();
        std::env::remove_var(DISABLE_TELEMETRY_ENV);
        let reporter = TelemetryReporter::new();
        assert!(reporter.is_enabled());

        reporter
            .record_tool_use(
                "req-1".to_string(),
                "test-tool".to_string(),
                "use-1".to_string(),
                serde_json::json!({"test": "input"}),
                false,
                100,
                true,
                None,
                Some(50),
            )
            .await;

        assert_eq!(reporter.pending_count().await, 1);
    }
}
