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
mod scanner;
#[cfg(test)]
mod tests;

pub use cache::{BlobsCache, Checkpoint, FileBlob};
pub use scanner::ScanResult;

use anyhow::Result;
use cache::compute_path_uuid;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

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

/// Workspace manager for tracking file changes and uploads
pub struct WorkspaceManager {
    root_path: PathBuf,
    /// Default patterns to always ignore (directories like .git, node_modules, etc.)
    ignore_patterns: HashSet<String>,
    /// Gitignore matcher built from .gitignore and .augmentignore files
    gitignore: Option<Gitignore>,
    /// In-memory blobs cache (matches augment.mjs structure)
    blobs_cache: Arc<RwLock<BlobsCache>>,
    /// Path to persistent cache file (one per project)
    cache_file_path: PathBuf,
    /// Upload status
    upload_status: Arc<RwLock<UploadStatus>>,
    /// Content sequence counter
    content_seq_counter: Arc<RwLock<u64>>,
}

impl WorkspaceManager {
    /// Create a new workspace manager
    pub fn new(root_path: PathBuf) -> Self {
        let mut ignore_patterns = HashSet::new();

        // Common patterns to ignore (always applied)
        for pattern in &[
            ".git",
            ".gitignore",
            ".augmentignore",
            "node_modules",
            "target",
            ".augment",
            "dist",
            "build",
            ".next",
            ".venv",
            "venv",
            "__pycache__",
            ".DS_Store",
        ] {
            ignore_patterns.insert(pattern.to_string());
        }

        // Load .gitignore and .augmentignore files
        let gitignore = Self::load_ignore_files(&root_path);

        // Determine cache file path (~/.augment/blobs/<uuid>.json)
        let path_uuid = compute_path_uuid(&root_path);
        let cache_file_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".augment")
            .join("blobs")
            .join(format!("{}.json", path_uuid));

        Self {
            root_path,
            ignore_patterns,
            gitignore,
            blobs_cache: Arc::new(RwLock::new(BlobsCache::default())),
            cache_file_path,
            upload_status: Arc::new(RwLock::new(UploadStatus::default())),
            content_seq_counter: Arc::new(RwLock::new(1000)),
        }
    }

    /// Load .gitignore and .augmentignore files from the workspace root
    fn load_ignore_files(root_path: &Path) -> Option<Gitignore> {
        let mut builder = GitignoreBuilder::new(root_path);
        let mut has_patterns = false;

        // Load .gitignore if it exists
        let gitignore_path = root_path.join(".gitignore");
        if gitignore_path.exists() {
            if let Some(err) = builder.add(&gitignore_path) {
                warn!("Failed to parse .gitignore: {}", err);
            } else {
                info!("Loaded ignore patterns from .gitignore");
                has_patterns = true;
            }
        }

        // Load .augmentignore if it exists (same format as .gitignore)
        let augmentignore_path = root_path.join(".augmentignore");
        if augmentignore_path.exists() {
            if let Some(err) = builder.add(&augmentignore_path) {
                warn!("Failed to parse .augmentignore: {}", err);
            } else {
                info!("Loaded ignore patterns from .augmentignore");
                has_patterns = true;
            }
        }

        if has_patterns {
            builder.build().ok()
        } else {
            None
        }
    }

    /// Get the root path
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Get the root path as string (normalized with forward slashes)
    #[allow(dead_code)]
    pub fn root_path_str(&self) -> String {
        self.root_path.to_string_lossy().replace('\\', "/")
    }

    /// Load persistent state from disk
    pub async fn load_state(&self) -> Result<()> {
        let cache = BlobsCache::load(&self.cache_file_path)?;
        let mut cache_lock = self.blobs_cache.write().await;
        *cache_lock = cache;

        // Update content_seq_counter to be higher than any existing content_seq
        let max_seq = cache_lock
            .path_to_blob
            .values()
            .map(|e| e.content_seq)
            .max()
            .unwrap_or(1000);
        let mut counter = self.content_seq_counter.write().await;
        *counter = max_seq + 1;

        debug!(
            "Loaded {} blob entries from cache",
            cache_lock.path_to_blob.len()
        );
        Ok(())
    }

    /// Save persistent state to disk
    pub async fn save_state(&self) -> Result<()> {
        let cache_lock = self.blobs_cache.read().await;
        cache_lock.save(&self.cache_file_path)?;
        debug!(
            "Saved {} blob entries to cache",
            cache_lock.path_to_blob.len()
        );
        Ok(())
    }

    /// Check if a path should be ignored (public for tests)
    pub fn should_ignore_path(&self, path: &Path) -> bool {
        scanner::should_ignore(path, &self.ignore_patterns, self.gitignore.as_ref())
    }

    /// Scan workspace and collect file information (fast scan)
    pub async fn scan_and_collect(&self) -> Result<Vec<FileBlob>> {
        let blobs = scanner::scan_workspace(
            &self.root_path,
            &self.ignore_patterns,
            self.gitignore.as_ref(),
        );
        Ok(blobs)
    }

    /// Scan and return files that need to be uploaded (not in cache)
    pub async fn scan_and_get_files_to_upload(&self) -> Result<Vec<FileBlob>> {
        let all_blobs = self.scan_and_collect().await?;
        let cache = self.blobs_cache.read().await;

        let to_upload: Vec<FileBlob> = all_blobs
            .into_iter()
            .filter(|blob| !cache.has_blob(&blob.blob_name))
            .collect();

        debug!(
            "Files to upload: {} (out of {} scanned)",
            to_upload.len(),
            cache.len()
        );
        Ok(to_upload)
    }

    /// Get files that need to be uploaded (comparing scan results with cache)
    pub async fn get_files_to_upload(&self) -> Vec<FileBlob> {
        match self.scan_and_get_files_to_upload().await {
            Ok(files) => files,
            Err(e) => {
                warn!("Failed to get files to upload: {}", e);
                Vec::new()
            }
        }
    }

    /// Mark blob_names as uploaded (updates the cache with mtime and content_seq)
    pub async fn mark_as_uploaded(&self, blob_names: &[String]) {
        let mut cache = self.blobs_cache.write().await;
        let mut counter = self.content_seq_counter.write().await;

        for blob_name in blob_names {
            if !cache.has_blob(blob_name) {
                cache.blob_to_path.insert(blob_name.clone(), String::new());
            }
        }

        *counter += blob_names.len() as u64;
        debug!("Marked {} blobs as uploaded", blob_names.len());
    }

    /// Mark files as uploaded with full information (path, mtime, blob_name)
    ///
    /// Uses the mtime captured during scan to avoid race conditions where
    /// a file might change between scan and upload completion.
    pub async fn mark_files_as_uploaded(&self, files: &[FileBlob]) {
        let mut cache = self.blobs_cache.write().await;
        let mut counter = self.content_seq_counter.write().await;

        for file in files {
            let content_seq = *counter;
            *counter += 1;

            // Use scan-time mtime to avoid race condition:
            // If we re-fetch mtime here and file changed after scan,
            // we'd cache wrong mtime -> next scan thinks file unchanged -> stale content
            cache.update(
                file.path.clone(),
                file.mtime,
                file.blob_name.clone(),
                content_seq,
            );
        }

        debug!("Marked {} files as uploaded with full info", files.len());
    }

    /// Get upload status
    #[allow(dead_code)]
    pub async fn get_upload_status(&self) -> UploadStatus {
        self.upload_status.read().await.clone()
    }

    /// Set upload status
    pub async fn set_upload_status(&self, status: UploadStatus) {
        let mut lock = self.upload_status.write().await;
        *lock = status;
    }

    /// Get current checkpoint with all known blob_names
    pub async fn get_checkpoint(&self) -> Checkpoint {
        let cache = self.blobs_cache.read().await;
        Checkpoint {
            checkpoint_id: None,
            added_blobs: cache.get_uploaded_blob_names().into_iter().collect(),
            deleted_blobs: Vec::new(),
        }
    }

    /// Get all current blob_names (from cache)
    #[allow(dead_code)]
    pub async fn get_current_blob_names(&self) -> Vec<String> {
        let cache = self.blobs_cache.read().await;
        cache.get_uploaded_blob_names().into_iter().collect()
    }

    /// Get the blobs cache for direct access
    pub fn blobs_cache(&self) -> &Arc<RwLock<BlobsCache>> {
        &self.blobs_cache
    }

    /// Get files that need upload by comparing with previous cache state
    #[allow(dead_code)]
    pub async fn get_files_needing_upload(&self, blobs: &[FileBlob]) -> Vec<FileBlob> {
        let cache = self.blobs_cache.read().await;
        blobs
            .iter()
            .filter(|blob| !cache.has_blob(&blob.blob_name))
            .cloned()
            .collect()
    }

    /// Incremental scan using mtime optimization.
    ///
    /// This is much faster than full scan for large projects:
    /// - Only reads file content when mtime changed
    /// - Detects deleted files automatically
    /// - Returns unchanged blob_names from cache
    pub async fn scan_incremental(&self) -> ScanResult {
        let cache = self.blobs_cache.read().await;
        scanner::scan_workspace_incremental(
            &self.root_path,
            &cache,
            &self.ignore_patterns,
            self.gitignore.as_ref(),
        )
    }

    /// Remove deleted files from cache.
    /// Returns the blob_names that were removed.
    pub async fn remove_deleted_from_cache(&self, deleted_paths: &[String]) -> Vec<String> {
        if deleted_paths.is_empty() {
            return Vec::new();
        }

        let mut cache = self.blobs_cache.write().await;
        let mut removed_blobs = Vec::new();

        for path in deleted_paths {
            if let Some(entry) = cache.path_to_blob.remove(path) {
                cache.blob_to_path.remove(&entry.blob_name);
                removed_blobs.push(entry.blob_name);
            }
        }

        if !removed_blobs.is_empty() {
            debug!("Removed {} deleted files from cache", removed_blobs.len());
        }

        removed_blobs
    }

    /// Sync cache with filesystem state, removing entries for deleted files.
    /// Returns the list of removed blob_names.
    #[allow(dead_code)]
    pub async fn sync_cache_with_filesystem(&self) -> Vec<String> {
        let current_files = match self.scan_and_collect().await {
            Ok(files) => files,
            Err(e) => {
                warn!("Failed to scan workspace for cache sync: {}", e);
                return Vec::new();
            }
        };

        let valid_blobs: HashSet<String> =
            current_files.iter().map(|f| f.blob_name.clone()).collect();

        let mut cache = self.blobs_cache.write().await;
        let deleted = cache.retain_blobs(&valid_blobs);

        if !deleted.is_empty() {
            debug!("Removed {} stale blob entries from cache", deleted.len());
        }

        deleted
    }
}

/// Shared workspace manager type for async operations
pub type SharedWorkspaceManager = Arc<RwLock<WorkspaceManager>>;

/// Create a shared workspace manager
pub fn create_shared_workspace_manager(root_path: PathBuf) -> SharedWorkspaceManager {
    Arc::new(RwLock::new(WorkspaceManager::new(root_path)))
}
