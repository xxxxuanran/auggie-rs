//! Prompt enhancer tool implementation.

use rmcp::{model::*, ErrorData as McpError};

use crate::api::{ApiCliMode, ApiClient};
use crate::mcp::types::PromptEnhancerArgs;

use super::common::{require_session, tool_error};

/// Enhance and improve a user prompt
pub async fn prompt_enhancer(args: PromptEnhancerArgs) -> Result<CallToolResult, McpError> {
    let prompt = args.prompt;

    // Check for empty prompt
    if prompt.trim().is_empty() {
        return Ok(tool_error("Error: Cannot enhance empty prompt"));
    }

    // Combine prompt with context if provided
    let full_prompt = if let Some(ctx) = args.context {
        format!("{}\n\nContext: {}", prompt, ctx)
    } else {
        prompt
    };

    // Get session
    let session = match require_session() {
        Ok(s) => s,
        Err(e) => return Ok(e),
    };

    // Call API
    let api_client = ApiClient::with_mode(ApiCliMode::Mcp);
    match api_client
        .prompt_enhancer(
            &session.tenant_url,
            &session.access_token,
            full_prompt,
            None, // chat_history
            None, // conversation_id
            None, // model
        )
        .await
    {
        Ok(result) => Ok(CallToolResult::success(vec![Content::text(
            result.enhanced_prompt,
        )])),
        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
            "Error calling prompt-enhancer API: {}",
            e
        ))])),
    }
}
