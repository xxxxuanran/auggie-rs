//! Codebase retrieval tool implementation.

use rmcp::{model::*, ErrorData as McpError};
use tracing::info;

use crate::api::{ApiCliMode, ApiClient};
use crate::mcp::types::CodebaseRetrievalArgs;
use crate::workspace::{sync_incremental, SharedWorkspaceManager};

use super::common::{require_session, tool_error};

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

    // Get session
    let session = match require_session() {
        Ok(s) => s,
        Err(e) => return Ok(e),
    };

    // Sync workspace (scan + upload)
    let api_client = ApiClient::with_mode(ApiCliMode::Mcp);
    let sync_result = {
        let wm = workspace_manager.read().await;
        sync_incremental(&wm, &api_client, &session.tenant_url, &session.access_token).await
    };

    info!(
        "🔍 Searching codebase with {} indexed files...",
        sync_result.checkpoint.added_blobs.len()
    );

    // Call API
    let result = api_client
        .agents()
        .codebase_retrieval(
            &session.tenant_url,
            &session.access_token,
            args.information_request,
            sync_result.checkpoint,
        )
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
