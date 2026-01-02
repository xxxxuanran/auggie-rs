//! MCP server handlers and startup logic.
//!
//! This module contains the server startup, background upload, and utility functions.

use anyhow::{Context, Result};
use tracing::{error, info, warn};

use crate::api::ApiClient;
use crate::session::AuthSessionStore;
use crate::workspace::{create_shared_workspace_manager, sync_full, SharedWorkspaceManager};

use super::server::AuggieMcpServer;

/// Perform background upload of all files
pub(super) async fn background_upload(
    workspace_manager: SharedWorkspaceManager,
    tenant_url: String,
    access_token: String,
) {
    let api_client = ApiClient::new(None);

    let sync_result = {
        let wm = workspace_manager.read().await;
        sync_full(&wm, &api_client, &tenant_url, &access_token).await
    };

    info!(
        "✅ Background upload complete: {} files uploaded",
        sync_result.uploaded_count
    );
}

/// Detect git repository root by searching upward from current directory
fn detect_git_root() -> Result<std::path::PathBuf> {
    let current = std::env::current_dir().context("Failed to get current directory")?;
    let mut path = current.as_path();

    loop {
        if path.join(".git").exists() {
            return Ok(path.to_path_buf());
        }
        match path.parent() {
            Some(parent) => path = parent,
            None => return Err(anyhow::anyhow!("No git root found")),
        }
    }
}

/// Run the MCP server over stdio
pub async fn run_mcp_server(workspace_root: Option<String>, _model: Option<String>) -> Result<()> {
    use rmcp::{transport::stdio, ServiceExt};

    info!("🔧 Starting Auggie MCP Tool Server...");
    info!("📝 Stdio mode (using rmcp)");

    // Determine workspace root: use provided path, or detect git root, or fallback to current dir
    let workspace_root = if let Some(path) = workspace_root {
        std::path::PathBuf::from(&path)
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize provided workspace root: {}", path))?
    } else {
        detect_git_root()
            .unwrap_or_else(|_| std::env::current_dir().expect("Failed to get current directory"))
    };

    info!("🔍 Initializing workspace at: {}", workspace_root.display());
    let workspace_manager = create_shared_workspace_manager(workspace_root);

    // Load persistent state
    {
        let wm = workspace_manager.read().await;
        if let Err(e) = wm.load_state().await {
            warn!("Failed to load workspace state: {}", e);
        }
    }

    info!("✅ Workspace manager initialized");

    // Try to start background upload if logged in
    let session_store = AuthSessionStore::new(None).ok();
    if let Some(store) = session_store {
        if store.is_logged_in() {
            if let Ok(Some(session)) = store.get_session() {
                info!("🔄 Starting workspace indexing in background...");
                let wm = workspace_manager.clone();
                let tenant_url = session.tenant_url.clone();
                let access_token = session.access_token.clone();

                // Spawn background upload task
                tokio::spawn(async move {
                    background_upload(wm, tenant_url, access_token).await;
                });
            }
        } else {
            info!("⚠️ Not logged in - background indexing skipped (will index on first search)");
        }
    }

    // Create server with workspace manager
    let server = AuggieMcpServer::new(Some(workspace_manager));

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
