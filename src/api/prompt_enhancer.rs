//! Prompt enhancer functionality for the Augment API.
//!
//! This module handles communication with both:
//! - New endpoint: `/prompt-enhancer` (direct, no blobs)
//! - Legacy endpoint: `/chat-stream` with blobs (for codebase context)
//!
//! The endpoint is controlled by the `AUGGIE_USE_NEW_PROMPT_ENHANCER` environment variable:
//! - Not set or "0"/"false": Use legacy chat-stream endpoint (default, matches augment.mjs)
//! - Set to "1"/"true": Use new prompt-enhancer endpoint

use std::sync::LazyLock;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use regex::Regex;
use tracing::{debug, info, warn};

/// Cached regex for extracting enhanced prompt from XML tags.
/// Pattern: case-insensitive match of <augment-enhanced-prompt>...</augment-enhanced-prompt>
static ENHANCED_PROMPT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<augment-enhanced-prompt>\s*([\s\S]*?)\s*</augment-enhanced-prompt>")
        .expect("ENHANCED_PROMPT_RE is a valid regex")
});

use super::client::ApiClient;
use super::types::{
    ChatHistoryExchange, ChatStreamBlobs, ChatStreamRequest, PromptEnhancerChunk,
    PromptEnhancerNode, PromptEnhancerRequest, PromptEnhancerResult, PromptEnhancerTextNode,
};
use crate::domain::Checkpoint;
use uuid::Uuid;

/// Timeout for prompt enhancer requests (300 seconds / 5 minutes)
const PROMPT_ENHANCER_TIMEOUT_SECS: u64 = 300;

/// Environment variable to control endpoint selection
const ENV_USE_NEW_ENDPOINT: &str = "AUGGIE_USE_NEW_PROMPT_ENHANCER";

/// Parse a string value as a boolean flag.
/// Returns true for "1", "true", "yes", "on" (case-insensitive).
/// Returns false for all other values including empty strings.
fn parse_bool_env(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

/// Check if new prompt enhancer endpoint should be used.
/// Default: false (use legacy chat-stream endpoint to match augment.mjs default behavior)
fn should_use_new_endpoint() -> bool {
    std::env::var(ENV_USE_NEW_ENDPOINT)
        .map(|val| parse_bool_env(&val))
        .unwrap_or(false)
}

/// Build the LUr-wrapped prompt for legacy chat-stream endpoint.
///
/// This matches the prompt format used by augment.mjs for prompt enhancement
/// via the chat-stream endpoint. The wrapped prompt includes:
/// - System instructions for no tool use
/// - XML tag format requirement for output
/// - The original user prompt
///
/// Source: augment.mjs LUr function and ENHANCER_AUGGIE.md section 3.3
fn build_legacy_prompt(prompt: &str) -> String {
    format!(
        r#"You are a prompt improvement assistant. Your task is to rewrite the user's prompt to be more clear, specific, and actionable.

NO TOOLS ALLOWED - Do not use any tools or external resources. Just rewrite the prompt directly.

Please rewrite the following prompt to be more effective. Output your improved prompt inside <augment-enhanced-prompt></augment-enhanced-prompt> tags.

### ORIGINAL PROMPT ###
{}
### END ORIGINAL PROMPT ###

IMPORTANT: Output ONLY the enhanced prompt wrapped in <augment-enhanced-prompt></augment-enhanced-prompt> tags. Do not include any explanation or other text.

Example format:
<augment-enhanced-prompt>Your enhanced prompt goes here</augment-enhanced-prompt>"#,
        prompt
    )
}

/// Extract enhanced prompt from XML tags in chat-stream response.
///
/// Matches: <augment-enhanced-prompt>...</augment-enhanced-prompt>
fn extract_enhanced_prompt(response: &str) -> Option<String> {
    ENHANCED_PROMPT_RE
        .captures(response)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| !s.is_empty())
}

impl ApiClient {
    /// Call the prompt enhancer with automatic endpoint selection.
    ///
    /// This is the main entry point for prompt enhancement. It automatically
    /// selects between the new and legacy endpoints based on the
    /// `AUGGIE_USE_NEW_PROMPT_ENHANCER` environment variable.
    ///
    /// # Arguments
    /// * `tenant_url` - The tenant URL for API requests
    /// * `access_token` - The access token for authentication
    /// * `prompt` - The prompt to enhance
    /// * `chat_history` - Optional chat history for context
    /// * `conversation_id` - Optional conversation ID
    /// * `model` - Optional model to use
    /// * `checkpoint` - Optional checkpoint with blobs (used for legacy endpoint)
    pub async fn prompt_enhancer(
        &self,
        tenant_url: &str,
        access_token: &str,
        prompt: String,
        chat_history: Option<Vec<ChatHistoryExchange>>,
        conversation_id: Option<String>,
        model: Option<String>,
        checkpoint: Option<Checkpoint>,
    ) -> Result<PromptEnhancerResult> {
        if should_use_new_endpoint() {
            info!("Using new prompt-enhancer endpoint");
            self.prompt_enhancer_new(
                tenant_url,
                access_token,
                prompt,
                chat_history,
                conversation_id,
                model,
            )
            .await
        } else {
            info!("Using legacy chat-stream endpoint for prompt enhancement");
            self.prompt_enhancer_legacy(
                tenant_url,
                access_token,
                prompt,
                chat_history,
                conversation_id,
                model,
                checkpoint,
            )
            .await
        }
    }

    /// Call the new prompt-enhancer endpoint directly.
    ///
    /// This endpoint does not include blobs/checkpoint data.
    async fn prompt_enhancer_new(
        &self,
        tenant_url: &str,
        access_token: &str,
        prompt: String,
        chat_history: Option<Vec<ChatHistoryExchange>>,
        conversation_id: Option<String>,
        model: Option<String>,
    ) -> Result<PromptEnhancerResult> {
        let request_id = Uuid::new_v4().to_string();

        // Build the request body
        let request_body = PromptEnhancerRequest {
            nodes: vec![PromptEnhancerNode {
                id: 0,
                node_type: 0,
                text_node: PromptEnhancerTextNode { content: prompt },
            }],
            chat_history: chat_history.unwrap_or_default(),
            conversation_id,
            model,
            mode: "CHAT".to_string(),
        };

        debug!("=== Prompt Enhancer Request (new endpoint) ===");

        let response = self
            .post_api_with_timeout(
                "prompt-enhancer",
                tenant_url,
                Some(access_token),
                &request_body,
                PROMPT_ENHANCER_TIMEOUT_SECS,
                Some(&request_id),
            )
            .await?;

        let status = response.status();
        debug!("Status: {}", status);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!(
                "Prompt enhancer API request failed with status {}: {}",
                status,
                error_text
            );
        }

        // Process streaming response
        let enhanced_text = process_streaming_response(response).await?;
        let enhanced_prompt = enhanced_text.trim().to_string();

        if enhanced_prompt.is_empty() {
            anyhow::bail!("Prompt enhancer returned empty response");
        }

        debug!("Enhanced prompt length: {}", enhanced_prompt.len());
        Ok(PromptEnhancerResult { enhanced_prompt })
    }

    /// Call the legacy chat-stream endpoint for prompt enhancement.
    ///
    /// This endpoint includes blobs/checkpoint data for codebase context,
    /// matching the behavior of augment.mjs when `cliPromptEnhancerNewEndpointRolloutPct = 0`.
    async fn prompt_enhancer_legacy(
        &self,
        tenant_url: &str,
        access_token: &str,
        prompt: String,
        chat_history: Option<Vec<ChatHistoryExchange>>,
        conversation_id: Option<String>,
        model: Option<String>,
        checkpoint: Option<Checkpoint>,
    ) -> Result<PromptEnhancerResult> {
        let request_id = Uuid::new_v4().to_string();

        // Build LUr-wrapped message
        let wrapped_message = build_legacy_prompt(&prompt);

        // Convert checkpoint to blobs format
        let blobs = checkpoint
            .map(ChatStreamBlobs::from)
            .unwrap_or_else(|| ChatStreamBlobs {
                checkpoint_id: None,
                added_blobs: Vec::new(),
                deleted_blobs: Vec::new(),
            });

        let blob_count = blobs.added_blobs.len();
        debug!(
            "=== Prompt Enhancer Request (legacy chat-stream) with {} blobs ===",
            blob_count
        );

        // Build the request body
        let request_body = ChatStreamRequest {
            message: wrapped_message,
            chat_history: chat_history.unwrap_or_default(),
            blobs,
            silent: true,
            mode: "CHAT".to_string(),
            tool_definitions: Vec::new(),
            nodes: Vec::new(),
            model,
            conversation_id,
        };

        let response = self
            .post_api_with_timeout(
                "chat-stream",
                tenant_url,
                Some(access_token),
                &request_body,
                PROMPT_ENHANCER_TIMEOUT_SECS,
                Some(&request_id),
            )
            .await?;

        let status = response.status();
        debug!("Status: {}", status);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!(
                "Chat stream API request failed with status {}: {}",
                status,
                error_text
            );
        }

        // Process streaming response
        let full_response = process_streaming_response(response).await?;

        // Extract enhanced prompt from XML tags
        let enhanced_prompt = extract_enhanced_prompt(&full_response).ok_or_else(|| {
            warn!(
                "Failed to extract enhanced prompt from response (length: {})",
                full_response.len()
            );
            anyhow::anyhow!(
                "Failed to parse enhanced prompt from chat-stream response. \
                 The model may not have followed the expected XML format."
            )
        })?;

        if enhanced_prompt.is_empty() {
            anyhow::bail!("Prompt enhancer returned empty response");
        }

        debug!("Enhanced prompt length: {}", enhanced_prompt.len());
        Ok(PromptEnhancerResult { enhanced_prompt })
    }
}

/// Process a streaming response and extract all text content
async fn process_streaming_response(response: reqwest::Response) -> Result<String> {
    let mut text = String::new();
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("Failed to read response chunk")?;
        let chunk_str = String::from_utf8_lossy(&chunk);
        buffer.push_str(&chunk_str);

        // Process complete lines from buffer
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            // Parse JSON and extract text
            if let Ok(chunk_data) = serde_json::from_str::<PromptEnhancerChunk>(&line) {
                if let Some(t) = chunk_data.text {
                    text.push_str(&t);
                }
            }
        }
    }

    // Process any remaining data in buffer
    if !buffer.trim().is_empty() {
        if let Ok(chunk_data) = serde_json::from_str::<PromptEnhancerChunk>(buffer.trim()) {
            if let Some(t) = chunk_data.text {
                text.push_str(&t);
            }
        }
    }

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_enhanced_prompt() {
        let response = r#"Here is the enhanced prompt:

<augment-enhanced-prompt>
Write a function that calculates the factorial of a number recursively with proper error handling for negative inputs.
</augment-enhanced-prompt>

That's all!"#;

        let extracted = extract_enhanced_prompt(response);
        assert!(extracted.is_some());
        assert_eq!(
            extracted.unwrap(),
            "Write a function that calculates the factorial of a number recursively with proper error handling for negative inputs."
        );
    }

    #[test]
    fn test_extract_enhanced_prompt_multiline() {
        let response = r#"<augment-enhanced-prompt>
Line 1
Line 2
Line 3
</augment-enhanced-prompt>"#;

        let extracted = extract_enhanced_prompt(response);
        assert!(extracted.is_some());
        assert_eq!(extracted.unwrap(), "Line 1\nLine 2\nLine 3");
    }

    #[test]
    fn test_extract_enhanced_prompt_no_match() {
        let response = "This response has no XML tags";
        let extracted = extract_enhanced_prompt(response);
        assert!(extracted.is_none());
    }

    #[test]
    fn test_extract_enhanced_prompt_empty_content() {
        let response = "<augment-enhanced-prompt>   </augment-enhanced-prompt>";
        let extracted = extract_enhanced_prompt(response);
        assert!(extracted.is_none());
    }

    #[test]
    fn test_build_legacy_prompt() {
        let prompt = "Write a hello world";
        let wrapped = build_legacy_prompt(prompt);
        assert!(wrapped.contains("NO TOOLS ALLOWED"));
        assert!(wrapped.contains("<augment-enhanced-prompt>"));
        assert!(wrapped.contains("Write a hello world"));
    }

    #[test]
    fn test_parse_bool_env() {
        // Truthy values
        assert!(parse_bool_env("1"));
        assert!(parse_bool_env("true"));
        assert!(parse_bool_env("TRUE"));
        assert!(parse_bool_env("True"));
        assert!(parse_bool_env("yes"));
        assert!(parse_bool_env("YES"));
        assert!(parse_bool_env("on"));
        assert!(parse_bool_env("ON"));

        // Falsy values
        assert!(!parse_bool_env("0"));
        assert!(!parse_bool_env("false"));
        assert!(!parse_bool_env("FALSE"));
        assert!(!parse_bool_env("no"));
        assert!(!parse_bool_env("off"));
        assert!(!parse_bool_env(""));
        assert!(!parse_bool_env("random"));
    }
}
