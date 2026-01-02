//! Prompt enhancer tool implementation.

use rmcp::{model::*, ErrorData as McpError};
use tracing::{debug, info};

use crate::mcp::types::PromptEnhancerArgs;
use crate::runtime::get_client;
use crate::workspace::SharedWorkspaceManager;

use super::common::tool_error;

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
///
/// # Arguments
/// * `workspace_manager` - Optional shared workspace manager for codebase context
/// * `args` - Tool arguments (prompt, optional context)
/// * `model` - Optional model ID to use (from CLI -m/--model flag)
pub async fn prompt_enhancer(
    workspace_manager: &Option<SharedWorkspaceManager>,
    args: PromptEnhancerArgs,
    model: Option<String>,
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

    // Get authenticated client from runtime
    let client = match get_client() {
        Some(c) => c,
        None => {
            return Ok(tool_error(
                "Error: Not authenticated. Please run 'auggie login' first.",
            ));
        }
    };

    // Log model if specified
    if let Some(ref m) = model {
        info!("Using model for prompt enhancement: {}", m);
    }

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

    // Call API with existing checkpoint and model
    match client
        .prompt_enhancer(full_prompt, None, None, model, checkpoint)
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
