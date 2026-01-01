//! Blob cache management for workspace indexing.
//!
//! This module provides caching functionality for tracking uploaded file blobs,
//! matching the structure used by augment.mjs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use uuid::Uuid;

/// Namespace UUID for generating project-specific UUIDs (custom namespace for Auggie)
const AUGGIE_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

/// Compute a UUID v5 from the workspace root path for unique state file naming
/// UUID v5 is deterministic - same path always produces the same UUID
pub fn compute_path_uuid(path: &std::path::Path) -> String {
    // Normalize path to forward slashes for consistent hashing across platforms
    let normalized = path.to_string_lossy().replace('\\', "/");
    Uuid::new_v5(&AUGGIE_NAMESPACE, normalized.as_bytes()).to_string()
}

/// Represents a file with its content ready for upload
#[derive(Debug, Clone)]
pub struct FileBlob {
    pub path: String,
    pub content: String,
    pub blob_name: String,
    /// File modification time when scanned (milliseconds since epoch)
    pub mtime: u64,
}

/// Workspace checkpoint containing blob information for API requests
/// Note: added_blobs is a list of blob_names (SHA256 hashes of path+content)
/// to match the Augment API format used by acetool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    pub added_blobs: Vec<String>,
    pub deleted_blobs: Vec<String>,
}

/// Single file entry matching augment.mjs FileInfo structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// File modification time (milliseconds since epoch)
    pub mtime: u64,
    /// SHA256 hash of path + content
    pub blob_name: String,
    /// Content sequence number for tracking changes
    pub content_seq: u64,
}

/// Blobs cache for a single project - matches augment.mjs structure
/// This is stored as one file per project: ~/.augment/blobs/<uuid>.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlobsCache {
    /// Map of relative path to file entry (matches _allPathNames in augment.mjs)
    pub path_to_blob: HashMap<String, FileEntry>,
    /// Reverse index: blob_name to relative path (matches _blobNameToPathName in augment.mjs)
    #[serde(default)]
    pub blob_to_path: HashMap<String, String>,
}

impl BlobsCache {
    /// Load cache from file
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read blobs cache from {}", path.display()))?;
        let mut cache: BlobsCache = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse blobs cache from {}", path.display()))?;

        // Rebuild reverse index if empty (for backwards compatibility)
        if cache.blob_to_path.is_empty() && !cache.path_to_blob.is_empty() {
            cache.rebuild_reverse_index();
        }

        Ok(cache)
    }

    /// Save cache to file
    pub fn save(&self, path: &Path) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize blobs cache")?;
        fs::write(path, content)
            .with_context(|| format!("Failed to write blobs cache to {}", path.display()))
    }

    /// Rebuild the reverse index from path_to_blob
    fn rebuild_reverse_index(&mut self) {
        self.blob_to_path.clear();
        for (path, entry) in &self.path_to_blob {
            self.blob_to_path
                .insert(entry.blob_name.clone(), path.clone());
        }
    }

    /// Get all uploaded blob_names
    pub fn get_uploaded_blob_names(&self) -> HashSet<String> {
        self.path_to_blob
            .values()
            .map(|e| e.blob_name.clone())
            .collect()
    }

    /// Get blob_name for a path
    #[allow(dead_code)]
    pub fn get_blob_name(&self, path: &str) -> Option<&String> {
        self.path_to_blob.get(path).map(|e| &e.blob_name)
    }

    /// Get path for a blob_name (reverse lookup)
    #[allow(dead_code)]
    pub fn get_path(&self, blob_name: &str) -> Option<&String> {
        self.blob_to_path.get(blob_name)
    }

    /// Update or insert a file entry
    pub fn update(&mut self, path: String, mtime: u64, blob_name: String, content_seq: u64) {
        // Remove old blob_name from reverse index if path exists
        if let Some(old_entry) = self.path_to_blob.get(&path) {
            if old_entry.blob_name != blob_name {
                self.blob_to_path.remove(&old_entry.blob_name);
            }
        }

        // Insert new entry
        let entry = FileEntry {
            mtime,
            blob_name: blob_name.clone(),
            content_seq,
        };
        self.path_to_blob.insert(path.clone(), entry);
        self.blob_to_path.insert(blob_name, path);
    }

    /// Remove a file entry
    #[allow(dead_code)]
    pub fn remove(&mut self, path: &str) {
        if let Some(entry) = self.path_to_blob.remove(path) {
            self.blob_to_path.remove(&entry.blob_name);
        }
    }

    /// Check if a blob_name exists
    pub fn has_blob(&self, blob_name: &str) -> bool {
        self.blob_to_path.contains_key(blob_name)
    }

    /// Get the number of tracked files
    pub fn len(&self) -> usize {
        self.path_to_blob.len()
    }

    /// Check if cache is empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.path_to_blob.is_empty()
    }

    /// Retain only entries whose blob_name is in the valid set, remove others.
    /// Returns the list of removed blob_names.
    #[allow(dead_code)]
    pub fn retain_blobs(&mut self, valid_blobs: &HashSet<String>) -> Vec<String> {
        let mut deleted = Vec::new();

        self.path_to_blob.retain(|_path, entry| {
            if valid_blobs.contains(&entry.blob_name) {
                true
            } else {
                deleted.push(entry.blob_name.clone());
                false
            }
        });

        for blob_name in &deleted {
            self.blob_to_path.remove(blob_name);
        }

        deleted
    }
}

/// Compute blob_name using SHA256 hash of path + content
/// This matches the acetool format: sha256(path.encode('utf-8') + content.encode('utf-8'))
pub fn compute_blob_name(relative_path: &str, content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(relative_path.as_bytes());
    hasher.update(content);
    let result = hasher.finalize();
    format!("{:x}", result)
}
