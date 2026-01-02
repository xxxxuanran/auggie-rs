//! Batch upload logic for workspace files.
//!
//! Matches augment.mjs batch upload strategy:
//! - maxUploadBatchBlobCount = 128
//! - maxUploadBatchByteSize = 1e6
//! - On batch failure, fallback to sequential single-file uploads

use tracing::{debug, warn};

use crate::api::{AuthenticatedClient, BatchUploadBlob, BatchUploadResponse};

use super::FileBlob;

/// Maximum blobs per batch upload request (matches augment.mjs maxUploadBatchBlobCount)
pub const MAX_UPLOAD_BATCH_BLOB_COUNT: usize = 128;

/// Maximum batch size in bytes (matches augment.mjs maxUploadBatchByteSize = 1e6)
pub const MAX_UPLOAD_BATCH_BYTE_SIZE: usize = 1_000_000;

/// Split files into batches by both item count and byte size.
/// Matches augment.mjs hBe.addItem() logic: rejects if items.size >= maxItems || byteSize + n.byteSize >= maxByteSize
pub fn create_upload_batches(files: &[FileBlob]) -> Vec<Vec<FileBlob>> {
    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_bytes = 0usize;

    for file in files {
        let file_size = file.content.len();

        // Check if adding this file would exceed limits (using >= like augment.mjs)
        let would_exceed_count = current_batch.len() >= MAX_UPLOAD_BATCH_BLOB_COUNT;
        let would_exceed_bytes = current_bytes + file_size >= MAX_UPLOAD_BATCH_BYTE_SIZE;

        if (would_exceed_count || would_exceed_bytes) && !current_batch.is_empty() {
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

/// Result of uploading a single batch
pub struct BatchUploadResult {
    /// Number of files successfully uploaded in the batch request
    pub batch_uploaded: usize,
    /// Number of files successfully uploaded sequentially (fallback)
    pub sequential_uploaded: usize,
    /// Blob names returned by the server
    pub blob_names: Vec<String>,
    /// Files that were successfully uploaded (for cache marking)
    pub uploaded_files: Vec<FileBlob>,
}

/// Upload a batch of files with fallback to sequential uploads.
/// Matches augment.mjs _uploadBlobBatch + _uploadBlobsSequentially logic.
pub async fn upload_batch_with_fallback(
    client: &AuthenticatedClient,
    batch: &[FileBlob],
) -> BatchUploadResult {
    let mut result = BatchUploadResult {
        batch_uploaded: 0,
        sequential_uploaded: 0,
        blob_names: Vec::new(),
        uploaded_files: Vec::new(),
    };

    if batch.is_empty() {
        return result;
    }

    // Convert to API format
    let blobs: Vec<BatchUploadBlob> = batch
        .iter()
        .map(|fb| BatchUploadBlob {
            path: fb.path.clone(),
            content: fb.content.clone(),
        })
        .collect();

    // Try batch upload first
    let batch_result: Result<BatchUploadResponse, _> = client.batch_upload(blobs).await;

    let successfully_uploaded = match &batch_result {
        Ok(response) => {
            result
                .blob_names
                .extend(response.blob_names.iter().cloned());
            response.blob_names.len()
        }
        Err(e) => {
            warn!("Batch upload failed: {}", e);
            0
        }
    };

    // Mark batch-uploaded files
    if successfully_uploaded > 0 {
        result.batch_uploaded = successfully_uploaded;
        result
            .uploaded_files
            .extend(batch[..successfully_uploaded].iter().cloned());
        debug!("Batch uploaded {} files", successfully_uploaded);
    }

    // Fallback: upload remaining files sequentially (matches augment.mjs _uploadBlobsSequentially)
    for file in batch.iter().skip(successfully_uploaded) {
        let single_blob = vec![BatchUploadBlob {
            path: file.path.clone(),
            content: file.content.clone(),
        }];

        match client.batch_upload(single_blob).await {
            Ok(response) => {
                if !response.blob_names.is_empty() {
                    result.blob_names.extend(response.blob_names);
                    result.uploaded_files.push(file.clone());
                    result.sequential_uploaded += 1;
                    debug!("Sequential upload: {}", file.path);
                }
            }
            Err(_) => {
                // Silent fail on individual upload (matches augment.mjs: catch {})
            }
        }
    }

    result
}
