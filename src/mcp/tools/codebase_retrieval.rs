//! Codebase retrieval tool implementation.

use rmcp::{model::*, ErrorData as McpError};
use tracing::{debug, info, warn};

use crate::api::{ApiCliMode, ApiClient, BatchUploadBlob};
use crate::mcp::types::CodebaseRetrievalArgs;
use crate::session::AuthSessionStore;
use crate::workspace::SharedWorkspaceManager;

/// Maximum blobs per batch upload request
pub const BATCH_UPLOAD_SIZE: usize = 50;

/// Execute codebase retrieval
pub async fn codebase_retrieval(
    workspace_manager: &Option<SharedWorkspaceManager>,
    args: CodebaseRetrievalArgs,
) -> Result<CallToolResult, McpError> {
    // Get workspace manager
    let workspace_manager = match workspace_manager {
        Some(wm) => wm.clone(),
        None => {
            return Ok(CallToolResult::error(vec![Content::text(
                "Error: Workspace not initialized. Please ensure you're running from a valid workspace directory.",
            )]));
        }
    };

    // Get session
    let session_store = match AuthSessionStore::new(None) {
        Ok(store) => store,
        Err(e) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Error accessing session: {}",
                e
            ))]));
        }
    };

    if !session_store.is_logged_in() {
        return Ok(CallToolResult::error(vec![Content::text(
            "Error: Not logged in. Please run 'auggie login' first.",
        )]));
    }

    let session = match session_store.get_session() {
        Ok(Some(s)) => s,
        _ => {
            return Ok(CallToolResult::error(vec![Content::text(
                "Error: Could not read session information.",
            )]));
        }
    };

    // Perform incremental scan using mtime optimization
    info!("🔄 Performing incremental scan...");
    let api_client = ApiClient::with_mode(ApiCliMode::Mcp);

    // Incremental scan: only reads files with changed mtime
    let scan_result = {
        let wm = workspace_manager.read().await;
        wm.scan_incremental().await
    };

    info!(
        "📊 Scan result: {} to upload, {} unchanged, {} deleted",
        scan_result.to_upload.len(),
        scan_result.unchanged_blobs.len(),
        scan_result.deleted_paths.len()
    );

    // Remove deleted files from cache
    if !scan_result.deleted_paths.is_empty() {
        let wm = workspace_manager.read().await;
        let removed = wm
            .remove_deleted_from_cache(&scan_result.deleted_paths)
            .await;
        if !removed.is_empty() {
            info!("🗑️ Removed {} deleted files from cache", removed.len());
        }
    }

    // Upload new/modified files
    let mut uploaded_blobs = Vec::new();
    if !scan_result.to_upload.is_empty() {
        info!(
            "📤 Uploading {} new/modified files...",
            scan_result.to_upload.len()
        );

        // Upload in batches
        for chunk in scan_result.to_upload.chunks(BATCH_UPLOAD_SIZE) {
            let blobs: Vec<BatchUploadBlob> = chunk
                .iter()
                .map(|fb| BatchUploadBlob {
                    path: fb.path.clone(),
                    content: fb.content.clone(),
                })
                .collect();

            match api_client
                .batch_upload(&session.tenant_url, &session.access_token, blobs)
                .await
            {
                Ok(response) => {
                    // Mark as uploaded with full info (including mtime)
                    let wm = workspace_manager.read().await;
                    let uploaded_files: Vec<_> = chunk
                        .iter()
                        .zip(response.blob_names.iter())
                        .map(|(fb, _)| fb.clone())
                        .collect();
                    wm.mark_files_as_uploaded(&uploaded_files).await;
                    uploaded_blobs.extend(response.blob_names);
                    debug!("Uploaded batch of {} files", chunk.len());
                }
                Err(e) => {
                    warn!("Batch upload failed: {}", e);
                    // Continue with what we have uploaded so far
                }
            }
        }

        // Save state after upload
        {
            let wm = workspace_manager.read().await;
            if let Err(e) = wm.save_state().await {
                warn!("Failed to save workspace state: {}", e);
            }
        }
    }

    // Build checkpoint: unchanged blobs + newly uploaded blobs
    let mut all_blobs = scan_result.unchanged_blobs;
    all_blobs.extend(uploaded_blobs);

    let checkpoint = crate::workspace::Checkpoint {
        checkpoint_id: None,
        added_blobs: all_blobs,
        deleted_blobs: Vec::new(),
    };

    info!(
        "🔍 Searching codebase with {} indexed files...",
        checkpoint.added_blobs.len()
    );

    // Call API
    let result = api_client
        .agents()
        .codebase_retrieval(
            &session.tenant_url,
            &session.access_token,
            args.information_request,
            checkpoint,
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
