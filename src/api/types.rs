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
    pub blobs: crate::workspace::Checkpoint,
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
