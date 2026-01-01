use anyhow::Result;

use super::client::ApiClient;
use super::types::{BatchUploadBlob, BatchUploadRequest, BatchUploadResponse};

/// Timeout for batch upload requests (120 seconds)
const BATCH_UPLOAD_TIMEOUT_SECS: u64 = 120;

impl ApiClient {
    /// Call the batch-upload endpoint to upload file blobs
    pub async fn batch_upload(
        &self,
        tenant_url: &str,
        access_token: &str,
        blobs: Vec<BatchUploadBlob>,
    ) -> Result<BatchUploadResponse> {
        if blobs.is_empty() {
            return Ok(BatchUploadResponse {
                blob_names: Vec::new(),
            });
        }

        let request_body = BatchUploadRequest { blobs };
        self.call_api_with_timeout(
            "batch-upload",
            tenant_url,
            Some(access_token),
            &request_body,
            BATCH_UPLOAD_TIMEOUT_SECS,
        )
        .await
    }
}
