//! Domain types shared across modules.
//!
//! This module contains data structures that are used by multiple
//! parts of the application (API, workspace, MCP tools, etc.).
//! Moving these types here avoids circular dependencies between modules.

use serde::{Deserialize, Serialize};

/// Workspace checkpoint containing blob information for API requests.
///
/// This structure is used by both the workspace module (to track files)
/// and the API module (to send codebase retrieval requests).
///
/// Note: `added_blobs` is a list of blob_names (SHA256 hashes of path+content)
/// to match the Augment API format used by acetool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    pub added_blobs: Vec<String>,
    pub deleted_blobs: Vec<String>,
}

impl Default for Checkpoint {
    fn default() -> Self {
        Self {
            checkpoint_id: None,
            added_blobs: Vec::new(),
            deleted_blobs: Vec::new(),
        }
    }
}
