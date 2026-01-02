//! Prompt enhancer tool implementation.

use rmcp::{model::*, ErrorData as McpError};
use tracing::debug;

use crate::api::{ApiCliMode, ApiClient};
use crate::mcp::types::PromptEnhancerArgs;
use crate::workspace::SharedWorkspaceManager;

use super::common::{require_session, tool_error};

/// Enhance and improve a user prompt.
///
/// This tool uses either:
/// - Legacy chat-stream endpoint (default): Includes codebase context via blobs for better enhancement
/// - New prompt-enhancer endpoint: Direct enhancement without codebase context
///
/// The endpoint is controlled by the `AUGGIE_USE_NEW_PROMPT_ENHANCER` environment variable.
///
/// Note: This tool does not trigger workspace synchronization. It uses whatever
/// checkpoint data is already available from previous syncs.
pub async fn prompt_enhancer(
    workspace_manager: &Option<SharedWorkspaceManager>,
    args: PromptEnhancerArgs,
) -> Result<CallToolResult, McpError> {
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

    let api_client = ApiClient::with_mode(ApiCliMode::Mcp);

    // Get existing checkpoint from workspace (no sync triggered)
    let checkpoint = match workspace_manager {
        Some(wm) => {
            let manager = wm.read().await;
            let cp = manager.get_checkpoint().await;
            debug!(
                "Using {} existing indexed files for context",
                cp.added_blobs.len()
            );
            Some(cp)
        }
        None => {
            debug!("No workspace available, enhancing without codebase context");
            None
        }
    };

    // Call API with existing checkpoint
    match api_client
        .prompt_enhancer(
            &session.tenant_url,
            &session.access_token,
            full_prompt,
            None, // chat_history
            None, // conversation_id
            None, // model
            checkpoint,
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
