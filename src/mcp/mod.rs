//! MCP (Model Context Protocol) server implementation using rmcp.
//!
//! This module implements an MCP server using the official Rust MCP SDK (rmcp).
//! The server provides tools for codebase retrieval and prompt enhancement.

use rmcp::{
    handler::server::router::tool::ToolRouter, handler::server::wrapper::Parameters, model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use std::time::Instant;
use tracing::{debug, info, warn};

mod handlers;
pub mod types;

// Re-export run_mcp_server from handlers
pub use handlers::run_mcp_server;

use crate::api::{ApiCliMode, ApiClient, BatchUploadBlob};
use crate::session::AuthSessionStore;
use crate::telemetry::TelemetryReporter;
use crate::workspace::SharedWorkspaceManager;
use types::*;

/// Maximum blobs per batch upload request
const BATCH_UPLOAD_SIZE: usize = 50;

/// Auggie MCP Server
#[derive(Clone)]
pub struct AuggieMcpServer {
    workspace_manager: Option<SharedWorkspaceManager>,
    tool_router: ToolRouter<Self>,
    telemetry: TelemetryReporter,
}

#[tool_router]
impl AuggieMcpServer {
    /// Create a new Auggie MCP server
    pub fn new(workspace_manager: Option<SharedWorkspaceManager>) -> Self {
        Self {
            workspace_manager,
            tool_router: Self::tool_router(),
            telemetry: TelemetryReporter::new(),
        }
    }

    /// Echo back the input message
    #[tool(description = "Echo back the input message")]
    fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(&args.message)]))
    }

    /// Get current Augment session information
    #[tool(
        name = "get_session_info",
        description = "Get current Augment session information"
    )]
    fn get_session_info(
        &self,
        Parameters(_): Parameters<GetSessionInfoArgs>,
    ) -> Result<CallToolResult, McpError> {
        let session_store = match AuthSessionStore::new(None) {
            Ok(store) => store,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {}",
                    e
                ))]));
            }
        };

        let info = if session_store.is_logged_in() {
            match session_store.get_session() {
                Ok(Some(session)) => {
                    format!(
                        "Logged in\nTenant URL: {}\nScopes: {:?}",
                        session.tenant_url, session.scopes
                    )
                }
                _ => "Session exists but could not be read".to_string(),
            }
        } else {
            "Not logged in".to_string()
        };

        Ok(CallToolResult::success(vec![Content::text(info)]))
    }

    /// IMPORTANT: This is the primary tool for searching the codebase.
    #[tool(
        name = "codebase-retrieval",
        description = r#"IMPORTANT: This is the primary tool for searching the codebase. Please consider as the FIRST CHOICE for any codebase searches.

This MCP tool is Augment's context engine, the world's best codebase context engine. It:
1. Takes in a natural language description of the code you are looking for;
2. Uses a proprietary retrieval/embedding model suite that produces the highest-quality recall of relevant code snippets from across the codebase;
3. Maintains a real-time index of the codebase, so the results are always up-to-date and reflects the current state of the codebase;
4. Can retrieve across different programming languages;
5. Only reflects the current state of the codebase on the disk, and has no information on version control or code history.

The `codebase-retrieval` MCP tool should be used in the following cases:
* When you don't know which files contain the information you need
* When you want to gather high level information about the task you are trying to accomplish
* When you want to gather information about the codebase in general

Examples of good queries:
* "Where is the function that handles user authentication?"
* "What tests are there for the login functionality?"
* "How is the database connected to the application?"

Examples of bad queries:
* "Find definition of constructor of class Foo" (use grep tool instead)
* "Find all references to function bar" (use grep tool instead)
* "Show me how Checkout class is used in services/payment.py" (use file view tool instead)
* "Show context of the file foo.py" (use file view tool instead)

ALWAYS use codebase-retrieval when you're unsure of exact file locations."#
    )]
    async fn codebase_retrieval(
        &self,
        Parameters(args): Parameters<CodebaseRetrievalArgs>,
    ) -> Result<CallToolResult, McpError> {
        let start_time = Instant::now();
        let request_id = format!("mcp-request-{}", chrono::Utc::now().timestamp_millis());
        let tool_use_id = format!("mcp-tool-{}", chrono::Utc::now().timestamp_millis());
        let conversation_id = format!("mcp-conversation-{}", chrono::Utc::now().timestamp_millis());

        let information_request = args.information_request.clone();
        let tool_input = serde_json::json!({
            "information_request": &information_request
        });

        // Get workspace manager
        let workspace_manager = match &self.workspace_manager {
            Some(wm) => wm.clone(),
            None => {
                // Record error telemetry
                self.record_tool_telemetry(
                    &request_id,
                    &tool_use_id,
                    &tool_input,
                    true,
                    start_time.elapsed().as_millis() as u64,
                    Some(&conversation_id),
                    None,
                    None,
                    None,
                )
                .await;

                return Ok(CallToolResult::error(vec![Content::text(
                    "Error: Workspace not initialized. Please ensure you're running from a valid workspace directory.",
                )]));
            }
        };

        // Get session
        let session_store = match AuthSessionStore::new(None) {
            Ok(store) => store,
            Err(e) => {
                self.record_tool_telemetry(
                    &request_id,
                    &tool_use_id,
                    &tool_input,
                    true,
                    start_time.elapsed().as_millis() as u64,
                    Some(&conversation_id),
                    None,
                    None,
                    None,
                )
                .await;

                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error accessing session: {}",
                    e
                ))]));
            }
        };

        if !session_store.is_logged_in() {
            self.record_tool_telemetry(
                &request_id,
                &tool_use_id,
                &tool_input,
                true,
                start_time.elapsed().as_millis() as u64,
                Some(&conversation_id),
                None,
                None,
                None,
            )
            .await;

            return Ok(CallToolResult::error(vec![Content::text(
                "Error: Not logged in. Please run 'auggie login' first.",
            )]));
        }

        let session = match session_store.get_session() {
            Ok(Some(s)) => s,
            _ => {
                self.record_tool_telemetry(
                    &request_id,
                    &tool_use_id,
                    &tool_input,
                    true,
                    start_time.elapsed().as_millis() as u64,
                    Some(&conversation_id),
                    None,
                    None,
                    None,
                )
                .await;

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
                information_request,
                checkpoint,
            )
            .await;

        let duration_ms = start_time.elapsed().as_millis() as u64;

        match result {
            Ok(response) => {
                let output_len = response.formatted_retrieval.len();

                // Record success telemetry
                self.record_tool_telemetry(
                    &request_id,
                    &tool_use_id,
                    &tool_input,
                    false,
                    duration_ms,
                    Some(&conversation_id),
                    Some(output_len),
                    Some(&session.tenant_url),
                    Some(&session.access_token),
                )
                .await;

                Ok(CallToolResult::success(vec![Content::text(
                    response.formatted_retrieval,
                )]))
            }
            Err(e) => {
                // Record error telemetry
                self.record_tool_telemetry(
                    &request_id,
                    &tool_use_id,
                    &tool_input,
                    true,
                    duration_ms,
                    Some(&conversation_id),
                    None,
                    Some(&session.tenant_url),
                    Some(&session.access_token),
                )
                .await;

                Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error calling codebase-retrieval API: {}",
                    e
                ))]))
            }
        }
    }

    /// Helper to record tool use telemetry
    async fn record_tool_telemetry(
        &self,
        request_id: &str,
        tool_use_id: &str,
        tool_input: &serde_json::Value,
        is_error: bool,
        duration_ms: u64,
        conversation_id: Option<&str>,
        output_len: Option<usize>,
        tenant_url: Option<&str>,
        access_token: Option<&str>,
    ) {
        // Record the event
        self.telemetry
            .record_tool_use(
                request_id.to_string(),
                "codebase-retrieval".to_string(),
                tool_use_id.to_string(),
                tool_input.clone(),
                is_error,
                duration_ms,
                true, // is_mcp_tool
                conversation_id.map(|s| s.to_string()),
                output_len,
            )
            .await;

        // Flush immediately if we have credentials
        if let (Some(url), Some(token)) = (tenant_url, access_token) {
            let api_client = ApiClient::with_mode(ApiCliMode::Mcp);
            self.telemetry.flush(&api_client, url, token).await;
        }
    }

    /// Enhance and improve a user prompt
    #[tool(
        name = "prompt-enhancer",
        description = r#"Enhance and improve a user prompt to be clearer, more specific, and more actionable.

This tool takes a natural language prompt and rewrites it to be:
1. More specific and detailed
2. Clearer in intent and expected outcome
3. Better structured for AI understanding
4. More actionable with concrete steps

Use this tool when:
* You have a vague or unclear prompt that needs improvement
* You want to refine a prompt for better results
* You need to make a prompt more specific or detailed
* You want to transform a simple request into a comprehensive instruction

The enhanced prompt will preserve the original intent while making it more effective for AI processing."#
    )]
    async fn prompt_enhancer(
        &self,
        Parameters(args): Parameters<PromptEnhancerArgs>,
    ) -> Result<CallToolResult, McpError> {
        let prompt = args.prompt;

        // Check for empty prompt
        if prompt.trim().is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(
                "Error: Cannot enhance empty prompt",
            )]));
        }

        // Combine prompt with context if provided
        let full_prompt = if let Some(ctx) = args.context {
            format!("{}\n\nContext: {}", prompt, ctx)
        } else {
            prompt
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

        // Call API
        let api_client = ApiClient::with_mode(ApiCliMode::Mcp);
        match api_client
            .prompt_enhancer(
                &session.tenant_url,
                &session.access_token,
                full_prompt,
                None, // chat_history
                None, // conversation_id
                None, // model
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
}

#[tool_handler]
impl ServerHandler for AuggieMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "auggie".to_string(),
                title: None,
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Auggie MCP Server provides codebase retrieval and prompt enhancement tools."
                    .to_string(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_creation() {
        let server = AuggieMcpServer::new(None);
        assert!(server.workspace_manager.is_none());
    }
}
