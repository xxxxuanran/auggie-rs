//! Workspace management for codebase indexing.
//!
//! This module handles file scanning, hash computation, checkpoint management,
//! and blob upload tracking for the codebase-retrieval tool.
//!
//! Implements the "Hybrid Strategy" (方案 A):
//! - Fast scan on startup without blocking
//! - Background async upload of all files
//! - Incremental upload on search (only new/modified files)
//! - Optional checkpoint support for optimization

mod cache;
mod manager;
mod scanner;
mod sync;
#[cfg(test)]
mod tests;
mod types;
mod upload;

// Re-exports
pub use cache::{Checkpoint, FileBlob};
pub use manager::WorkspaceManager;
pub use sync::{sync_full, sync_incremental, SyncResult};
pub use types::{create_shared_workspace_manager, SharedWorkspaceManager, UploadStatus};
