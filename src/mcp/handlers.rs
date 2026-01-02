//! MCP server handlers and startup logic.
//!
//! This module contains the server startup, background upload, and utility functions.

use anyhow::{Context, Result};
use tracing::{error, info, warn};

use crate::api::{ApiCliMode, AuthenticatedClient};
use crate::runtime::{get_client, set_runtime};
use crate::startup::StartupContext;
use crate::workspace::{create_shared_workspace_manager, sync_full, SharedWorkspaceManager};

use super::server::AuggieMcpServer;

/// Perform background upload of all files using the global authenticated client.
pub(super) async fn background_upload(workspace_manager: SharedWorkspaceManager) {
    let client = match get_client() {
        Some(c) => c,
        None => {
            warn!("Cannot perform background upload: no authenticated client");
            return;
        }
    };

    let sync_result = {
        let wm = workspace_manager.read().await;
        sync_full(&wm, client).await
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
///
/// # Arguments
/// * `workspace_root` - Optional workspace root path (auto-detects git root if absent)
/// * `model` - Optional model ID to use for prompt enhancement (from CLI -m/--model)
pub async fn run_mcp_server(workspace_root: Option<String>, model: Option<String>) -> Result<()> {
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

    // Run startup ensure flow (auth, api, feature flags)
    let mut startup_ctx = match StartupContext::new(ApiCliMode::Mcp, None) {
        Ok(ctx) => ctx,
        Err(e) => {
            warn!("Failed to create startup context: {}", e);
            // Continue without startup validation - tools will fail if not logged in
            let server = AuggieMcpServer::new(Some(workspace_manager), None);
            return run_server(server).await;
        }
    };

    let resolved_model: Option<String>;

    match startup_ctx.ensure_all().await {
        Ok(state) => {
            // Resolve user-provided model using the loaded model_info_registry
            resolved_model = state.resolve_model(model.as_deref());

            if let Some(ref m) = resolved_model {
                info!("🎯 Using model: {}", m);
            }

            // Create authenticated client with stored credentials
            // This enables HTTP/2 connection reuse for all API calls
            let client = AuthenticatedClient::new(
                ApiCliMode::Mcp,
                state.tenant_url().to_string(),
                state.access_token().to_string(),
            );

            // Store runtime in global singleton (like augment.mjs's fdt())
            set_runtime(state, client);

            // Start background upload using the global client
            info!("🔄 Starting workspace indexing in background...");
            let wm = workspace_manager.clone();

            tokio::spawn(async move {
                background_upload(wm).await;
            });
        }
        Err(e) => {
            warn!("Startup validation failed: {}", e);
            info!("⚠️ Continuing without full validation - some tools may not work");

            if model.is_some() {
                warn!(
                    "Cannot validate --model={} without successful startup",
                    model.as_deref().unwrap_or("")
                );
            }
            resolved_model = None;
        }
    }

    // Create server with workspace manager and resolved model
    let server = AuggieMcpServer::new(Some(workspace_manager), resolved_model);

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
