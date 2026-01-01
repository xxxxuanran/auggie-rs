//! Workspace types and utilities.
//!
//! This module contains shared types for workspace management.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::manager::WorkspaceManager;

/// Upload status for tracking background upload progress
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct UploadStatus {
    pub total_files: usize,
    pub uploaded_files: usize,
    pub is_uploading: bool,
    pub upload_complete: bool,
    pub last_error: Option<String>,
}

/// Shared workspace manager type for async operations
pub type SharedWorkspaceManager = Arc<RwLock<WorkspaceManager>>;

/// Create a shared workspace manager
pub fn create_shared_workspace_manager(root_path: PathBuf) -> SharedWorkspaceManager {
    Arc::new(RwLock::new(WorkspaceManager::new(root_path)))
}
