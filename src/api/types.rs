//! API request and response types for Augment services.
//!
//! This module contains data structures for communicating with
//! the Augment backend API.

use serde::{Deserialize, Serialize};

/// Token request body
#[derive(Debug, Serialize)]
pub(super) struct TokenRequest {
    pub grant_type: String,
    pub client_id: String,
    pub code_verifier: String,
    pub redirect_uri: String,
    pub code: String,
}

/// Token response from the API
#[derive(Debug, Deserialize)]
pub(super) struct TokenResponse {
    pub access_token: String,
}

/// Batch upload blob item
#[derive(Debug, Clone, Serialize)]
pub struct BatchUploadBlob {
    pub path: String,
    pub content: String,
}

/// Batch upload request body
#[derive(Debug, Serialize)]
pub(super) struct BatchUploadRequest {
    pub blobs: Vec<BatchUploadBlob>,
}

/// Batch upload response
#[derive(Debug, Deserialize)]
pub struct BatchUploadResponse {
    pub blob_names: Vec<String>,
}

/// Codebase retrieval request body
#[derive(Debug, Serialize)]
pub(super) struct CodebaseRetrievalRequest {
    pub information_request: String,
    pub blobs: crate::domain::Checkpoint,
    pub dialog: Vec<serde_json::Value>,
    pub max_output_length: i32,
    pub disable_codebase_retrieval: bool,
    pub enable_commit_retrieval: bool,
}

/// Codebase retrieval response
#[derive(Debug, Deserialize)]
pub struct CodebaseRetrievalResponse {
    pub formatted_retrieval: String,
}

// ============================================================================
// Chat Stream Types (for legacy prompt enhancer path)
// ============================================================================

/// Blobs structure for chat-stream requests
#[derive(Debug, Clone, Serialize)]
pub(super) struct ChatStreamBlobs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    pub added_blobs: Vec<String>,
    pub deleted_blobs: Vec<String>,
}

impl From<crate::domain::Checkpoint> for ChatStreamBlobs {
    fn from(cp: crate::domain::Checkpoint) -> Self {
        Self {
            checkpoint_id: cp.checkpoint_id,
            added_blobs: cp.added_blobs,
            deleted_blobs: cp.deleted_blobs,
        }
    }
}

/// Chat stream request body (for legacy prompt enhancer via chat-stream endpoint)
///
/// This matches the request structure used by augment.mjs for silent chat requests.
/// See ENHANCER_AUGMENT.md section 3.4 for full field documentation.
#[derive(Debug, Serialize)]
pub(super) struct ChatStreamRequest {
    /// The message to send (LUr-wrapped prompt for enhancement)
    pub message: String,
    /// Chat history for context
    pub chat_history: Vec<ChatHistoryExchange>,
    /// Blob/checkpoint data for codebase context
    pub blobs: ChatStreamBlobs,
    /// Silent mode - don't show in UI
    pub silent: bool,
    /// Chat mode
    pub mode: String,
    /// Tool definitions (empty for prompt enhancement)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_definitions: Vec<serde_json::Value>,
    /// Nodes (empty for legacy path)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<serde_json::Value>,
    /// Model ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Conversation ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

// ============================================================================
// Prompt Enhancer Types (new endpoint)
// ============================================================================

/// Prompt enhancer text node
#[derive(Debug, Serialize)]
pub(super) struct PromptEnhancerTextNode {
    pub content: String,
}

/// Prompt enhancer node
#[derive(Debug, Serialize)]
pub(super) struct PromptEnhancerNode {
    pub id: i32,
    #[serde(rename = "type")]
    pub node_type: i32,
    pub text_node: PromptEnhancerTextNode,
}

/// Chat history exchange for prompt enhancer (simplified)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistoryExchange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Prompt enhancer request body
#[derive(Debug, Serialize)]
pub(super) struct PromptEnhancerRequest {
    pub nodes: Vec<PromptEnhancerNode>,
    pub chat_history: Vec<ChatHistoryExchange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub mode: String,
}

/// Prompt enhancer streaming response chunk
#[derive(Debug, Deserialize)]
pub struct PromptEnhancerChunk {
    #[serde(default)]
    pub text: Option<String>,
}

/// Prompt enhancer result
#[derive(Debug)]
pub struct PromptEnhancerResult {
    pub enhanced_prompt: String,
}

// ============================================================================
// Tool Use Event Telemetry Types
// ============================================================================

/// Tool use event data for telemetry
#[derive(Debug, Clone, Serialize)]
pub struct ToolUseData {
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_output_is_error: bool,
    pub tool_run_duration_ms: u64,
    pub tool_input: String,
    pub tool_input_len: usize,
    pub is_mcp_tool: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_history_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output_len: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_lines_added: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_lines_deleted: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_diff: Option<String>,
}

/// Event wrapper for tool use data
#[derive(Debug, Clone, Serialize)]
pub struct ToolUseEventWrapper {
    pub tool_use_data: ToolUseData,
}

/// Single event in the request
#[derive(Debug, Clone, Serialize)]
pub struct RequestEvent {
    pub time: String,
    pub event: ToolUseEventWrapper,
}

/// Request body for record-request-events API
#[derive(Debug, Serialize)]
pub(super) struct RecordRequestEventsRequest {
    pub events: Vec<RequestEvent>,
}

/// Tool use event for collection (internal representation)
#[derive(Debug, Clone)]
pub struct ToolUseEvent {
    pub request_id: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: String,
    pub tool_output_is_error: bool,
    pub tool_run_duration_ms: u64,
    pub is_mcp_tool: bool,
    pub conversation_id: Option<String>,
    pub chat_history_length: Option<usize>,
    pub tool_output_len: Option<usize>,
    pub tool_lines_added: Option<u32>,
    pub tool_lines_deleted: Option<u32>,
    pub tool_use_diff: Option<String>,
    pub event_time: chrono::DateTime<chrono::Utc>,
}

// ============================================================================
// API Status Codes (matches augment.mjs Am enum)
// ============================================================================

/// API status codes matching augment.mjs internal status codes.
///
/// These are translated from HTTP status codes and used for error handling.
/// See augment.mjs line 231845-231860 for the original definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ApiStatus {
    /// Success
    Ok = 0,
    /// Request was cancelled (client closed connection) - retryable
    Cancelled = 1,
    /// Unknown error
    Unknown = 2,
    /// Service unavailable - retryable
    Unavailable = 3,
    /// Endpoint not found / not implemented
    Unimplemented = 4,
    /// Invalid request arguments
    InvalidArgument = 5,
    /// Rate limit exceeded
    ResourceExhausted = 6,
    /// Authentication failed - FATAL, requires login
    Unauthenticated = 7,
    /// Permission denied (closed beta, account disabled) - FATAL
    PermissionDenied = 8,
    /// Request timeout
    DeadlineExceeded = 9,
    /// Request body too large
    AugmentTooLarge = 10,
    /// Client-side timeout
    AugmentClientTimeout = 11,
    /// Client version too old - FATAL, requires upgrade
    AugmentUpgradeRequired = 12,
}

impl ApiStatus {
    /// Convert from i32 status code
    pub fn from_i32(code: i32) -> Self {
        match code {
            0 => ApiStatus::Ok,
            1 => ApiStatus::Cancelled,
            2 => ApiStatus::Unknown,
            3 => ApiStatus::Unavailable,
            4 => ApiStatus::Unimplemented,
            5 => ApiStatus::InvalidArgument,
            6 => ApiStatus::ResourceExhausted,
            7 => ApiStatus::Unauthenticated,
            8 => ApiStatus::PermissionDenied,
            9 => ApiStatus::DeadlineExceeded,
            10 => ApiStatus::AugmentTooLarge,
            11 => ApiStatus::AugmentClientTimeout,
            12 => ApiStatus::AugmentUpgradeRequired,
            _ => ApiStatus::Unknown,
        }
    }

    /// Convert from HTTP status code to internal API status
    /// See augment.mjs fbn function (line 231938-231964)
    pub fn from_http_status(http_status: u16) -> Self {
        match http_status {
            200..=299 => ApiStatus::Ok,
            400 => ApiStatus::InvalidArgument,
            401 => ApiStatus::Unauthenticated,
            403 => ApiStatus::PermissionDenied,
            404 => ApiStatus::Unimplemented,
            408 => ApiStatus::AugmentClientTimeout,
            413 => ApiStatus::AugmentTooLarge,
            426 => ApiStatus::AugmentUpgradeRequired,
            429 => ApiStatus::ResourceExhausted,
            499 => ApiStatus::Cancelled,
            504 => ApiStatus::DeadlineExceeded,
            500..=599 => ApiStatus::Unavailable,
            _ => ApiStatus::Unknown,
        }
    }

    /// Check if this error is fatal (requires user action, cannot continue)
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            ApiStatus::Unauthenticated
                | ApiStatus::PermissionDenied
                | ApiStatus::AugmentUpgradeRequired
        )
    }

    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self, ApiStatus::Cancelled | ApiStatus::Unavailable)
    }

    /// Get the error message for this status
    pub fn error_message(&self) -> &'static str {
        match self {
            ApiStatus::Ok => "Success",
            ApiStatus::Cancelled => "Request was cancelled",
            ApiStatus::Unknown => "Unknown error occurred",
            ApiStatus::Unavailable => "Service temporarily unavailable",
            ApiStatus::Unimplemented => "Endpoint not found",
            ApiStatus::InvalidArgument => "Invalid request",
            ApiStatus::ResourceExhausted => "Rate limit exceeded. Please wait and try again",
            ApiStatus::Unauthenticated => {
                "Authentication failed. Please run 'auggie login' to re-authenticate"
            }
            ApiStatus::PermissionDenied => {
                "Auggie CLI is in closed beta. If you're part of an Enterprise organization \
                 and would like to get access, contact: contact@augmentcode.com. \
                 For non-enterprise users, sign up for the waitlist at augment.new"
            }
            ApiStatus::DeadlineExceeded => "Request timed out",
            ApiStatus::AugmentTooLarge => "Request body too large",
            ApiStatus::AugmentClientTimeout => "Client timeout",
            ApiStatus::AugmentUpgradeRequired => {
                "Client upgrade required. Please update to the latest version"
            }
        }
    }
}

impl std::fmt::Display for ApiStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error_message())
    }
}

// ============================================================================
// API Error Type (for detailed error handling)
// ============================================================================

/// API error with status code and details.
///
/// This is similar to the `la` (APIError) class in augment.mjs.
/// It captures the HTTP status, internal API status, and error details.
#[derive(Debug, Clone)]
pub struct ApiError {
    /// Internal API status code (0-12)
    pub status: ApiStatus,
    /// HTTP status code
    pub http_status: u16,
    /// Error message
    pub message: String,
    /// Request ID (for debugging)
    pub request_id: Option<String>,
    /// Whether this error should trigger a re-login prompt
    pub requires_relogin: bool,
}

impl ApiError {
    /// Create from HTTP status code and response body
    pub fn from_http_response(http_status: u16, body: String, request_id: Option<String>) -> Self {
        let status = ApiStatus::from_http_status(http_status);
        let requires_relogin = matches!(
            status,
            ApiStatus::Unauthenticated | ApiStatus::PermissionDenied
        );

        let message = match status {
            ApiStatus::Unauthenticated => {
                format!(
                    "Authentication failed (HTTP {}). Your token may have expired. \
                     Please run 'auggie login' to re-authenticate.",
                    http_status
                )
            }
            ApiStatus::PermissionDenied => {
                format!(
                    "Permission denied (HTTP {}). {}",
                    http_status,
                    status.error_message()
                )
            }
            ApiStatus::ResourceExhausted => {
                format!(
                    "Rate limit exceeded (HTTP {}). Please wait and try again.",
                    http_status
                )
            }
            ApiStatus::AugmentUpgradeRequired => {
                format!(
                    "Client upgrade required (HTTP {}). Please update to the latest version.",
                    http_status
                )
            }
            _ => {
                if body.is_empty() {
                    format!(
                        "API error (HTTP {}): {}",
                        http_status,
                        status.error_message()
                    )
                } else {
                    format!("API error (HTTP {}): {}", http_status, body)
                }
            }
        };

        Self {
            status,
            http_status,
            message,
            request_id,
            requires_relogin,
        }
    }

    /// Check if this error is fatal (requires user action)
    pub fn is_fatal(&self) -> bool {
        self.status.is_fatal()
    }

    /// Get a hint message for the user
    pub fn user_hint(&self) -> &'static str {
        match self.status {
            ApiStatus::Unauthenticated => {
                "Your session has expired. Please run 'auggie login' to re-authenticate."
            }
            ApiStatus::PermissionDenied => {
                "Your account does not have access to this feature. \
                 Contact your administrator or sign up at augment.new"
            }
            ApiStatus::AugmentUpgradeRequired => "Please update auggie to the latest version.",
            ApiStatus::ResourceExhausted => {
                "You have exceeded the rate limit. Please wait a moment and try again."
            }
            ApiStatus::Unavailable => {
                "The Augment service is temporarily unavailable. Please try again later."
            }
            _ => "An unexpected error occurred. Please try again or contact support.",
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ApiError {}

// ============================================================================
// Get Models API Types (for connection validation and feature flags)
// ============================================================================

/// User info from get-models response
#[derive(Debug, Clone, Deserialize)]
pub struct GetModelsUser {
    pub id: String,
    pub email: String,
    pub tenant_id: String,
    pub tenant_name: String,
}

/// Single model info from get-models response
#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub model: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
}

/// Feature flags from get-models response (v1 format)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FeatureFlagsV1 {
    #[serde(default)]
    pub enable_codebase_retrieval: Option<bool>,
    #[serde(default)]
    pub enable_commit_retrieval: Option<bool>,
    #[serde(default)]
    pub enable_prompt_enhancer: Option<bool>,
    #[serde(default)]
    pub enable_telemetry: Option<bool>,
    #[serde(default)]
    pub enable_mcp_mode: Option<bool>,
    #[serde(default)]
    pub enable_cli_mode: Option<bool>,
    /// Catch-all for unknown flags
    #[serde(flatten)]
    pub other: std::collections::HashMap<String, serde_json::Value>,
}

/// Feature flags from get-models response (v2 format with explicit enabled/disabled)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FeatureFlagsV2 {
    #[serde(default)]
    pub enabled: Vec<String>,
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// Get models response (full fields for feature flags and validation)
#[derive(Debug, Clone, Deserialize)]
pub struct GetModelsResponse {
    /// Default model to use
    #[serde(default)]
    pub default_model: Option<String>,

    /// Available models list
    #[serde(default)]
    pub models: Vec<ModelInfo>,

    /// Supported programming languages
    #[serde(default)]
    pub languages: Vec<String>,

    /// Feature flags (v1 format - key-value pairs)
    #[serde(default)]
    pub feature_flags: FeatureFlagsV1,

    /// Feature flags (v2 format - explicit enabled/disabled lists)
    #[serde(default, rename = "feature_flags_v2")]
    pub feature_flags_v2: Option<FeatureFlagsV2>,

    /// User tier (e.g., "free", "pro", "enterprise")
    #[serde(default)]
    pub user_tier: Option<String>,

    /// User information
    #[serde(default)]
    pub user: Option<GetModelsUser>,

    /// Error status code from API (e.g., 8 = account disabled for CLI/MCP mode)
    #[serde(default)]
    pub status: Option<i32>,
}

impl GetModelsResponse {
    /// Check if a feature flag is enabled (checks both v1 and v2 formats)
    pub fn is_feature_enabled(&self, flag_name: &str) -> bool {
        // First check v2 format (more explicit)
        if let Some(ref v2) = self.feature_flags_v2 {
            if v2.enabled.contains(&flag_name.to_string()) {
                return true;
            }
            if v2.disabled.contains(&flag_name.to_string()) {
                return false;
            }
        }

        // Then check v1 format
        match flag_name {
            "enable_codebase_retrieval" => {
                self.feature_flags.enable_codebase_retrieval.unwrap_or(true)
            }
            "enable_commit_retrieval" => {
                self.feature_flags.enable_commit_retrieval.unwrap_or(false)
            }
            "enable_prompt_enhancer" => self.feature_flags.enable_prompt_enhancer.unwrap_or(true),
            "enable_telemetry" => self.feature_flags.enable_telemetry.unwrap_or(false),
            "enable_mcp_mode" => self.feature_flags.enable_mcp_mode.unwrap_or(true),
            "enable_cli_mode" => self.feature_flags.enable_cli_mode.unwrap_or(true),
            other => self
                .feature_flags
                .other
                .get(other)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    }

    /// Check if MCP mode is enabled for this account
    pub fn is_mcp_enabled(&self) -> bool {
        if self.status == Some(8) {
            return false;
        }
        self.is_feature_enabled("enable_mcp_mode")
    }

    /// Check if CLI mode is enabled for this account
    pub fn is_cli_enabled(&self) -> bool {
        if self.status == Some(8) {
            return false;
        }
        self.is_feature_enabled("enable_cli_mode")
    }

    /// Get the default model name
    pub fn get_default_model(&self) -> Option<&str> {
        self.default_model.as_deref()
    }
}

// ============================================================================
// Validation Result (for connection validation)
// ============================================================================

/// Result of a connection validation check
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// Connection is valid
    Ok,
    /// Invalid credentials (401/403)
    InvalidCredentials(String),
    /// Connection error (network issues)
    ConnectionError(String),
    /// Server error (5xx)
    ServerError(String),
    /// Invalid URL configuration
    InvalidUrl(String),
}
