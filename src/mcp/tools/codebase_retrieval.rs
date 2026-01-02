//! Codebase retrieval tool implementation.

use rmcp::{model::*, ErrorData as McpError};
use tracing::{debug, info, warn};

use crate::api::{ApiCliMode, ApiClient, BatchUploadBlob};
use crate::mcp::types::CodebaseRetrievalArgs;
use crate::session::AuthSessionStore;
use crate::workspace::{FileBlob, SharedWorkspaceManager};

/// Maximum batch size in bytes (4MB)
const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

/// Minimum batch size in bytes (256KB) - don't go smaller than this
const MIN_BATCH_BYTES: usize = 256 * 1024;

/// Maximum blobs per batch upload request (fallback)
pub const BATCH_UPLOAD_SIZE: usize = 50;

/// Split files into batches by byte size (returns owned clones)
fn batch_by_bytes(files: &[FileBlob], max_bytes: usize) -> Vec<Vec<FileBlob>> {
    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_bytes = 0;

    for file in files {
        let file_size = file.content.len();

        // If single file exceeds max, put it in its own batch
        if file_size > max_bytes {
            if !current_batch.is_empty() {
                batches.push(current_batch);
                current_batch = Vec::new();
                current_bytes = 0;
            }
            batches.push(vec![file.clone()]);
            continue;
        }

        // Would adding this file exceed the limit?
        if current_bytes + file_size > max_bytes && !current_batch.is_empty() {
            batches.push(current_batch);
            current_batch = Vec::new();
            current_bytes = 0;
        }

        current_batch.push(file.clone());
        current_bytes += file_size;
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    batches
}

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

    // Upload new/modified files with adaptive batch sizing
    let mut uploaded_blobs = Vec::new();
    if !scan_result.to_upload.is_empty() {
        info!(
            "📤 Uploading {} new/modified files...",
            scan_result.to_upload.len()
        );

        // Start with max batch size, reduce on 413 errors
        let mut current_max_bytes = MAX_BATCH_BYTES;

        // Create batches by byte size
        let batches = batch_by_bytes(&scan_result.to_upload, current_max_bytes);
        debug!("Split into {} batches", batches.len());

        for batch in batches {
            let mut retry_batch = batch;
            let mut retries = 0;
            const MAX_RETRIES: u32 = 3;

            while !retry_batch.is_empty() && retries < MAX_RETRIES {
                let blobs: Vec<BatchUploadBlob> = retry_batch
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
                        wm.mark_files_as_uploaded(&retry_batch).await;
                        uploaded_blobs.extend(response.blob_names);
                        debug!("Uploaded batch of {} files", retry_batch.len());
                        break;
                    }
                    Err(e) => {
                        let is_too_large = e.to_string().contains("413")
                            || e.to_string().to_lowercase().contains("too large")
                            || e.to_string().to_lowercase().contains("payload");

                        if is_too_large && current_max_bytes > MIN_BATCH_BYTES {
                            // Halve the batch size and retry
                            current_max_bytes = (current_max_bytes / 2).max(MIN_BATCH_BYTES);
                            warn!(
                                "Batch too large, reducing to {}KB and retrying...",
                                current_max_bytes / 1024
                            );
                            // Re-batch the current files
                            let sub_batches = batch_by_bytes(&retry_batch, current_max_bytes);
                            // Process first sub-batch, queue rest
                            if let Some(first) = sub_batches.into_iter().next() {
                                retry_batch = first;
                            }
                            retries += 1;
                        } else {
                            warn!("Batch upload failed: {}", e);
                            break;
                        }
                    }
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
