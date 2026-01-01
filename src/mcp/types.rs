//! MCP tool parameter types.
//!
//! These types are used with rmcp's `Parameters<T>` wrapper for automatic
//! deserialization and JSON schema generation.

use schemars::JsonSchema;
use serde::Deserialize;

/// Parameters for the echo tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EchoArgs {
    /// The message to echo
    pub message: String,
}

/// Parameters for the get_session_info tool (no arguments needed)
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSessionInfoArgs {}

/// Parameters for the codebase-retrieval tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CodebaseRetrievalArgs {
    /// A description of the information you need from the codebase
    pub information_request: String,
}

/// Parameters for the prompt-enhancer tool
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptEnhancerArgs {
    /// The original prompt text to enhance
    pub prompt: String,
    /// Optional additional context to help enhance the prompt
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}
