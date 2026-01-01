use anyhow::Result;

use super::AgentsApi;
use crate::api::types::{CodebaseRetrievalRequest, CodebaseRetrievalResponse};
use crate::workspace::Checkpoint;

/// Timeout for codebase retrieval requests (120 seconds)
const CODEBASE_RETRIEVAL_TIMEOUT_SECS: u64 = 120;

impl<'a> AgentsApi<'a> {
    /// Call the agents/codebase-retrieval endpoint
    pub async fn codebase_retrieval(
        &self,
        tenant_url: &str,
        access_token: &str,
        information_request: String,
        checkpoint: Checkpoint,
    ) -> Result<CodebaseRetrievalResponse> {
        let request_body = CodebaseRetrievalRequest {
            information_request,
            blobs: checkpoint,
            dialog: Vec::new(),
            max_output_length: 0,
            disable_codebase_retrieval: false,
            enable_commit_retrieval: false,
        };

        self.call_api_with_timeout(
            "codebase-retrieval",
            tenant_url,
            Some(access_token),
            &request_body,
            CODEBASE_RETRIEVAL_TIMEOUT_SECS,
        )
        .await
    }
}
