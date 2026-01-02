//! MCP server implementation.
//!
//! This module contains the AuggieMcpServer struct and its tool routing.

use rmcp::{
    handler::server::router::tool::ToolRouter, handler::server::wrapper::Parameters, model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use std::time::Instant;

use crate::runtime::get_client;
use crate::telemetry::TelemetryReporter;
use crate::workspace::SharedWorkspaceManager;

use super::tools;
use super::types::*;

/// Auggie MCP Server
#[derive(Clone)]
pub struct AuggieMcpServer {
    workspace_manager: Option<SharedWorkspaceManager>,
    tool_router: ToolRouter<Self>,
    telemetry: TelemetryReporter,
    /// Model ID to use for prompt enhancement (from CLI -m/--model flag)
    model: Option<String>,
}

#[tool_router]
impl AuggieMcpServer {
    /// Create a new Auggie MCP server
    ///
    /// # Arguments
    /// * `workspace_manager` - Optional shared workspace manager for codebase indexing
    /// * `model` - Optional model ID to use for prompt enhancement (from CLI -m/--model)
    pub fn new(workspace_manager: Option<SharedWorkspaceManager>, model: Option<String>) -> Self {
        Self {
            workspace_manager,
            tool_router: Self::tool_router(),
            telemetry: TelemetryReporter::new(),
            model,
        }
    }

    /// Get the configured model ID
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Echo back the input message
    #[tool(description = "Echo back the input message")]
    fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> Result<CallToolResult, McpError> {
        tools::echo(args)
    }

    /// Get current Augment session information
    #[tool(
        name = "get_session_info",
        description = "Get current Augment session information"
    )]
    fn get_session_info(
        &self,
        Parameters(args): Parameters<GetSessionInfoArgs>,
    ) -> Result<CallToolResult, McpError> {
        tools::get_session_info(args)
    }

    /// IMPORTANT: This is the primary tool for searching the codebase.
    #[tool(
        name = "codebase-retrieval",
        description = "IMPORTANT: This is the primary tool for searching the codebase. Please consider as the FIRST CHOICE for any codebase searches.\n\nThis MCP tool is Augment's context engine, the world's best codebase context engine. It:\n1. Takes in a natural language description of the code you are looking for;\n2. Uses a proprietary retrieval/embedding model suite that produces the highest-quality recall of relevant code snippets from across the codebase;\n3. Maintains a real-time index of the codebase, so the results are always up-to-date and reflects the current state of the codebase;\n4. Can retrieve across different programming languages;\n5. Only reflects the current state of the codebase on the disk, and has no information on version control or code history.\n\nThe `codebase-retrieval` MCP tool should be used in the following cases:\n* When you don't know which files contain the information you need\n* When you want to gather high level information about the task you are trying to accomplish\n* When you want to gather information about the codebase in general\n\nExamples of good queries:\n* \"Where is the function that handles user authentication?\"\n* \"What tests are there for the login functionality?\"\n* \"How is the database connected to the application?\"\n\nExamples of bad queries:\n* \"Find definition of constructor of class Foo\" (use grep tool instead)\n* \"Find all references to function bar\" (use grep tool instead)\n* \"Show me how Checkout class is used in services/payment.py\" (use file view tool instead)\n* \"Show context of the file foo.py\" (use file view tool instead)\n\nALWAYS use codebase-retrieval when you're unsure of exact file locations."
    )]
    async fn codebase_retrieval(
        &self,
        Parameters(args): Parameters<CodebaseRetrievalArgs>,
    ) -> Result<CallToolResult, McpError> {
        let start_time = Instant::now();
        let request_id = format!("mcp-request-{}", chrono::Utc::now().timestamp_millis());
        let tool_use_id = format!("mcp-tool-{}", chrono::Utc::now().timestamp_millis());
        let conversation_id = format!("mcp-conversation-{}", chrono::Utc::now().timestamp_millis());
        let tool_input = serde_json::json!({
            "information_request": &args.information_request
        });

        // Execute the tool
        let result = tools::codebase_retrieval(&self.workspace_manager, args).await;
        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Record telemetry based on result
        let (is_error, output_len) = match &result {
            Ok(r) => {
                let is_err = r.is_error.unwrap_or(false);
                // Estimate output length from first content item
                let len = if is_err {
                    None
                } else {
                    r.content.first().map(|c| format!("{:?}", c).len())
                };
                (is_err, len)
            }
            Err(_) => (true, None),
        };

        self.telemetry
            .record_tool_use(
                request_id,
                "codebase-retrieval".to_string(),
                tool_use_id,
                tool_input,
                is_error,
                duration_ms,
                true,
                Some(conversation_id),
                output_len,
            )
            .await;

        // Flush telemetry if we have an authenticated client
        if let Some(client) = get_client() {
            self.telemetry.flush(client).await;
        }

        result
    }

    /// Enhance and improve a user prompt
    #[tool(
        name = "prompt-enhancer",
        description = "Enhance and improve a user prompt to be clearer, more specific, and more actionable.\n\nThis tool takes a natural language prompt and rewrites it to be:\n1. More specific and detailed\n2. Clearer in intent and expected outcome\n3. Better structured for AI understanding\n4. More actionable with concrete steps\n\nUse this tool when:\n* You have a vague or unclear prompt that needs improvement\n* You want to refine a prompt for better results\n* You need to make a prompt more specific or detailed\n* You want to transform a simple request into a comprehensive instruction\n\nThe enhanced prompt will preserve the original intent while making it more effective for AI processing."
    )]
    async fn prompt_enhancer(
        &self,
        Parameters(args): Parameters<PromptEnhancerArgs>,
    ) -> Result<CallToolResult, McpError> {
        tools::prompt_enhancer(&self.workspace_manager, args, self.model.clone()).await
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
        let server = AuggieMcpServer::new(None, None);
        assert!(server.workspace_manager.is_none());
        assert!(server.model.is_none());
    }

    #[test]
    fn test_mcp_server_with_model() {
        let server = AuggieMcpServer::new(None, Some("claude-sonnet-4-5".to_string()));
        assert!(server.workspace_manager.is_none());
        assert_eq!(server.model(), Some("claude-sonnet-4-5"));
    }
}
