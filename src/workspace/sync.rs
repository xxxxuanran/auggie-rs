//! Workspace synchronization logic.
//!
//! Provides unified sync operations that combine:
//! - Incremental scanning (mtime-based change detection)
//! - Batch upload with fallback to sequential
//! - Cache management

use tracing::{debug, info, warn};

use crate::api::AuthenticatedClient;

use super::cache::Checkpoint;
use super::manager::WorkspaceManager;
use super::upload::{create_upload_batches, upload_batch_with_fallback};
use super::UploadStatus;

/// Result of a workspace sync operation
pub struct SyncResult {
    /// Checkpoint containing all blob names (unchanged + newly uploaded)
    pub checkpoint: Checkpoint,
    /// Number of files uploaded in this sync
    pub uploaded_count: usize,
    /// Number of files that were already up-to-date
    pub unchanged_count: usize,
    /// Number of deleted files removed from cache
    pub deleted_count: usize,
}

/// Callback for reporting sync progress
pub trait SyncProgressCallback: Send + Sync {
    fn on_progress(&self, uploaded: usize, total: usize);
}

/// No-op progress callback
pub struct NoOpProgress;
impl SyncProgressCallback for NoOpProgress {
    fn on_progress(&self, _uploaded: usize, _total: usize) {}
}

/// Progress callback that updates UploadStatus
pub struct UploadStatusProgress<'a> {
    pub manager: &'a WorkspaceManager,
    pub total_files: usize,
}

impl SyncProgressCallback for UploadStatusProgress<'_> {
    fn on_progress(&self, uploaded: usize, _total: usize) {
        // Note: This is a sync callback, but set_upload_status is async
        // We'll handle this differently in the sync function
        let _ = (uploaded, self.total_files);
    }
}

/// Perform incremental sync of workspace.
///
/// This is the main sync function used by codebase_retrieval:
/// 1. Scans for changed files (using mtime optimization)
/// 2. Uploads new/modified files in batches
/// 3. Updates cache with uploaded files
/// 4. Returns checkpoint with all known blob names
pub async fn sync_incremental(
    manager: &WorkspaceManager,
    client: &AuthenticatedClient,
) -> SyncResult {
    // Perform incremental scan
    info!("🔄 Performing incremental scan...");
    let scan_result = manager.scan_incremental().await;

    info!(
        "📊 Scan result: {} to upload, {} unchanged, {} deleted",
        scan_result.to_upload.len(),
        scan_result.unchanged_blobs.len(),
        scan_result.deleted_paths.len()
    );

    let deleted_count = scan_result.deleted_paths.len();
    let unchanged_count = scan_result.unchanged_blobs.len();

    // Remove deleted files from cache
    if !scan_result.deleted_paths.is_empty() {
        let removed = manager
            .remove_deleted_from_cache(&scan_result.deleted_paths)
            .await;
        if !removed.is_empty() {
            info!("🗑️ Removed {} deleted files from cache", removed.len());
        }
    }

    // Upload new/modified files
    let mut uploaded_blobs = Vec::new();
    let mut uploaded_count = 0;

    if !scan_result.to_upload.is_empty() {
        info!(
            "📤 Uploading {} new/modified files...",
            scan_result.to_upload.len()
        );

        let batches = create_upload_batches(&scan_result.to_upload);
        debug!("Split into {} batches", batches.len());

        for batch in batches {
            let result = upload_batch_with_fallback(client, &batch).await;

            // Mark uploaded files in cache
            if !result.uploaded_files.is_empty() {
                manager.mark_files_as_uploaded(&result.uploaded_files).await;
                uploaded_blobs.extend(result.blob_names);
                uploaded_count += result.batch_uploaded + result.sequential_uploaded;
            }
        }

        // Save state after upload
        if let Err(e) = manager.save_state().await {
            warn!("Failed to save workspace state: {}", e);
        }
    }

    // Build checkpoint: unchanged blobs + newly uploaded blobs
    let mut all_blobs = scan_result.unchanged_blobs;
    all_blobs.extend(uploaded_blobs);

    let checkpoint = Checkpoint {
        checkpoint_id: None,
        added_blobs: all_blobs,
        deleted_blobs: Vec::new(),
    };

    SyncResult {
        checkpoint,
        uploaded_count,
        unchanged_count,
        deleted_count,
    }
}

/// Perform full sync of workspace (for background upload).
///
/// Unlike incremental sync, this:
/// 1. Scans all files (not just changed ones)
/// 2. Updates UploadStatus during progress
/// 3. Returns total counts
pub async fn sync_full(manager: &WorkspaceManager, client: &AuthenticatedClient) -> SyncResult {
    info!("🔄 Starting full workspace sync...");

    // Scan workspace
    if let Err(e) = manager.scan_and_collect().await {
        warn!("Failed to scan workspace: {}", e);
        return SyncResult {
            checkpoint: Checkpoint {
                checkpoint_id: None,
                added_blobs: Vec::new(),
                deleted_blobs: Vec::new(),
            },
            uploaded_count: 0,
            unchanged_count: 0,
            deleted_count: 0,
        };
    }

    // Get files to upload
    let files_to_upload = manager.get_files_to_upload().await;

    if files_to_upload.is_empty() {
        info!("✅ No files to upload (all files already indexed)");
        let checkpoint = manager.get_checkpoint().await;
        return SyncResult {
            checkpoint,
            uploaded_count: 0,
            unchanged_count: 0,
            deleted_count: 0,
        };
    }

    let total_files = files_to_upload.len();
    info!("📤 Uploading {} files...", total_files);

    // Update initial status
    manager
        .set_upload_status(UploadStatus {
            total_files,
            uploaded_files: 0,
            is_uploading: true,
            upload_complete: false,
            last_error: None,
        })
        .await;

    let mut uploaded_count = 0;
    let batches = create_upload_batches(&files_to_upload);
    debug!("Split into {} batches", batches.len());

    for batch in batches {
        let result = upload_batch_with_fallback(client, &batch).await;

        // Mark uploaded files in cache
        if !result.uploaded_files.is_empty() {
            manager.mark_files_as_uploaded(&result.uploaded_files).await;
            uploaded_count += result.batch_uploaded + result.sequential_uploaded;

            // Update progress
            manager
                .set_upload_status(UploadStatus {
                    total_files,
                    uploaded_files: uploaded_count,
                    is_uploading: true,
                    upload_complete: false,
                    last_error: None,
                })
                .await;

            debug!("Upload progress: {}/{} files", uploaded_count, total_files);
        }
    }

    // Save state after upload
    if let Err(e) = manager.save_state().await {
        warn!("Failed to save workspace state: {}", e);
    }

    // Mark upload complete
    manager
        .set_upload_status(UploadStatus {
            total_files,
            uploaded_files: uploaded_count,
            is_uploading: false,
            upload_complete: true,
            last_error: None,
        })
        .await;

    info!(
        "✅ Full sync complete: {}/{} files uploaded",
        uploaded_count, total_files
    );

    let checkpoint = manager.get_checkpoint().await;

    SyncResult {
        checkpoint,
        uploaded_count,
        unchanged_count: 0,
        deleted_count: 0,
    }
}
