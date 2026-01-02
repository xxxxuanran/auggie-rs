//! MCP server handlers.
//!
//! This module contains only the MCP server startup logic.
//! Note: Authentication ensure flow and workspace initialization are handled in main.rs.

use anyhow::Result;
use tracing::{error, info};

use crate::workspace::SharedWorkspaceManager;

use super::server::AuggieMcpServer;

/// Run the MCP server over stdio.
///
/// This function is called AFTER ensure flow and workspace initialization complete in main.rs.
/// It only handles MCP server startup.
///
/// # Arguments
/// * `workspace_manager` - Pre-initialized workspace manager (None for degraded startup)
/// * `resolved_model` - Pre-resolved model ID (resolved in main.rs after ensure)
pub async fn run_mcp_server(
    workspace_manager: Option<SharedWorkspaceManager>,
    resolved_model: Option<String>,
) -> Result<()> {
    info!("🔧 Starting Auggie MCP Tool Server...");
    info!("📝 Stdio mode (using rmcp)");

    let server = AuggieMcpServer::new(workspace_manager, resolved_model);

    run_server(server).await
}

/// Run the MCP server with the given server instance.
async fn run_server(server: AuggieMcpServer) -> Result<()> {
    use rmcp::{transport::stdio, ServiceExt};

    info!("✅ MCP tool server started");
    info!("🔗 Ready for MCP client connections");

    // Start the service
    let service = server.serve(stdio()).await.map_err(|e| {
        error!("Failed to start MCP service: {:?}", e);
        anyhow::anyhow!("Failed to start MCP service: {:?}", e)
    })?;

    // Wait for service to complete
    service.waiting().await.map_err(|e| {
        error!("MCP service error: {:?}", e);
        anyhow::anyhow!("MCP service error: {:?}", e)
    })?;

    info!("MCP server shutting down");
    Ok(())
}
