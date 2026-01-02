//! Codebase retrieval tool implementation.

use rmcp::{model::*, ErrorData as McpError};
use tracing::info;

use crate::mcp::types::CodebaseRetrievalArgs;
use crate::runtime::get_client;
use crate::workspace::{sync_incremental, SharedWorkspaceManager};

use super::common::tool_error;

/// Execute codebase retrieval
pub async fn codebase_retrieval(
    workspace_manager: &Option<SharedWorkspaceManager>,
    args: CodebaseRetrievalArgs,
) -> Result<CallToolResult, McpError> {
    // Get workspace manager
    let workspace_manager = match workspace_manager {
        Some(wm) => wm.clone(),
        None => {
            return Ok(tool_error(
                "Error: Workspace not initialized. Please ensure you're running from a valid workspace directory.",
            ));
        }
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

    // Sync workspace (scan + upload)
    let sync_result = {
        let wm = workspace_manager.read().await;
        sync_incremental(&wm, client).await
    };

    info!(
        "🔍 Searching codebase with {} indexed files...",
        sync_result.checkpoint.added_blobs.len()
    );

    // Call API
    let result = client
        .codebase_retrieval(&args.information_request, sync_result.checkpoint)
        .await;

    match result {
        Ok(response) => Ok(CallToolResult::success(vec![Content::text(
            response.formatted_retrieval,
        )])),
        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
            "Error calling codebase-retrieval API: {}",
            e
        ))])),
    }
}
