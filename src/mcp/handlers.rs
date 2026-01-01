//! MCP server handlers and startup logic.
//!
//! This module contains the server startup, background upload, and utility functions.

use anyhow::{Context, Result};
use tracing::{debug, error, info, warn};

use crate::api::{ApiClient, BatchUploadBlob};
use crate::session::AuthSessionStore;
use crate::workspace::{create_shared_workspace_manager, SharedWorkspaceManager, UploadStatus};

use super::{AuggieMcpServer, BATCH_UPLOAD_SIZE};

/// Perform background upload of all files
pub(super) async fn background_upload(
    workspace_manager: SharedWorkspaceManager,
    tenant_url: String,
    access_token: String,
) {
    info!("🔄 Starting background upload...");

    let api_client = ApiClient::new(None);

    // Scan workspace
    {
        let wm = workspace_manager.read().await;
        if let Err(e) = wm.scan_and_collect().await {
            error!("Failed to scan workspace: {}", e);
            return;
        }
    }

    // Get files to upload
    let files_to_upload = {
        let wm = workspace_manager.read().await;
        wm.get_files_to_upload().await
    };

    if files_to_upload.is_empty() {
        info!("✅ No files to upload (all files already indexed)");
        return;
    }

    let total_files = files_to_upload.len();
    info!("📤 Uploading {} files in background...", total_files);

    // Debug print files to be uploaded
    debug!("📋 Files to be uploaded:");
    for file in &files_to_upload {
        debug!("  - {}", file.path);
    }

    // // Wait 5 seconds before starting upload
    // info!("⏳ Waiting 5 seconds before starting upload...");
    // tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    // info!("🚀 Starting batch upload...");

    // Update status
    {
        let wm = workspace_manager.read().await;
        wm.set_upload_status(UploadStatus {
            total_files,
            uploaded_files: 0,
            is_uploading: true,
            upload_complete: false,
            last_error: None,
        })
        .await;
    }

    let mut uploaded_count = 0;

    // Upload in batches
    for chunk in files_to_upload.chunks(BATCH_UPLOAD_SIZE) {
        let blobs: Vec<BatchUploadBlob> = chunk
            .iter()
            .map(|fb| BatchUploadBlob {
                path: fb.path.clone(),
                content: fb.content.clone(),
            })
            .collect();

        match api_client
            .batch_upload(&tenant_url, &access_token, blobs)
            .await
        {
            Ok(_response) => {
                // Mark files as uploaded with full path/mtime information
                let wm = workspace_manager.read().await;
                wm.mark_files_as_uploaded(chunk).await;
                uploaded_count += chunk.len();

                // Update status
                wm.set_upload_status(UploadStatus {
                    total_files,
                    uploaded_files: uploaded_count,
                    is_uploading: true,
                    upload_complete: false,
                    last_error: None,
                })
                .await;

                debug!("Uploaded batch: {}/{} files", uploaded_count, total_files);
            }
            Err(e) => {
                warn!("Batch upload failed: {}", e);
                let wm = workspace_manager.read().await;
                wm.set_upload_status(UploadStatus {
                    total_files,
                    uploaded_files: uploaded_count,
                    is_uploading: false,
                    upload_complete: false,
                    last_error: Some(e.to_string()),
                })
                .await;
                // Continue trying other batches
            }
        }
    }

    // Save state after upload
    {
        let wm = workspace_manager.read().await;
        if let Err(e) = wm.save_state().await {
            warn!("Failed to save workspace state: {}", e);
        }

        wm.set_upload_status(UploadStatus {
            total_files,
            uploaded_files: uploaded_count,
            is_uploading: false,
            upload_complete: true,
            last_error: None,
        })
        .await;
    }

    info!(
        "✅ Background upload complete: {}/{} files uploaded",
        uploaded_count, total_files
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
