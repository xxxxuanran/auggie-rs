//! Metadata storage for session tracking.
//!
//! This module handles persisting session metadata like last used time
//! and session count, equivalent to the MetadataManager in augment.mjs.
//!
//! See augment.mjs line 330589-330643 for the original implementation.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, warn};

/// Metadata stored in metadata.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    /// ISO 8601 timestamp of last use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,

    /// Number of sessions started
    #[serde(default)]
    pub session_count: u64,

    /// First use timestamp (for analytics)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_used: Option<String>,

    /// Client version at first use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_version: Option<String>,
}

/// Metadata manager for session tracking
///
/// Manages metadata persistence in ~/.augment/metadata.json
pub struct MetadataManager {
    metadata_path: PathBuf,
}

impl MetadataManager {
    /// Create a new metadata manager
    pub fn new(cache_dir: Option<String>) -> Result<Self> {
        let base_dir = match cache_dir {
            Some(dir) => PathBuf::from(dir),
            None => dirs::home_dir()
                .context("Could not determine home directory")?
                .join(".augment"),
        };

        // Create directory if it doesn't exist
        std::fs::create_dir_all(&base_dir)
            .with_context(|| format!("Failed to create cache directory: {:?}", base_dir))?;

        let metadata_path = base_dir.join("metadata.json");

        Ok(Self { metadata_path })
    }

    /// Read metadata from disk
    pub fn read_metadata(&self) -> Result<Metadata> {
        if !self.metadata_path.exists() {
            return Ok(Metadata::default());
        }

        let content = std::fs::read_to_string(&self.metadata_path)
            .with_context(|| format!("Failed to read metadata file: {:?}", self.metadata_path))?;

        serde_json::from_str(&content).with_context(|| "Failed to parse metadata JSON")
    }

    /// Write metadata to disk
    pub fn write_metadata(&self, metadata: &Metadata) -> Result<()> {
        let content =
            serde_json::to_string_pretty(metadata).context("Failed to serialize metadata")?;

        std::fs::write(&self.metadata_path, content)
            .with_context(|| format!("Failed to write metadata file: {:?}", self.metadata_path))?;

        debug!("Metadata saved to {:?}", self.metadata_path);
        Ok(())
    }

    /// Update session metadata (called at startup)
    ///
    /// This is equivalent to augment.mjs metadata.updateSession():
    /// - Sets lastUsed to current time
    /// - Increments sessionCount
    /// - Sets firstUsed if not already set
    pub fn update_session(&self) -> Result<()> {
        let mut metadata = self.read_metadata().unwrap_or_else(|e| {
            warn!("Failed to read metadata, starting fresh: {}", e);
            Metadata::default()
        });

        let now = chrono::Utc::now().to_rfc3339();

        // Set lastUsed to current time
        metadata.last_used = Some(now.clone());

        // Increment session count
        metadata.session_count += 1;

        // Set firstUsed if not already set
        if metadata.first_used.is_none() {
            metadata.first_used = Some(now);
            metadata.first_version = Some(env!("CARGO_PKG_VERSION").to_string());
        }

        self.write_metadata(&metadata)?;

        debug!(
            "Session updated: count={}, last_used={}",
            metadata.session_count,
            metadata.last_used.as_deref().unwrap_or("unknown")
        );

        Ok(())
    }

    /// Get the current session count
    #[allow(dead_code)]
    pub fn session_count(&self) -> u64 {
        self.read_metadata().map(|m| m.session_count).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_metadata_manager_new() {
        let tmp = tempdir().unwrap();
        let manager = MetadataManager::new(Some(tmp.path().to_string_lossy().to_string())).unwrap();
        assert!(manager.metadata_path.exists() == false); // File not created until write
    }

    #[test]
    fn test_update_session() {
        let tmp = tempdir().unwrap();
        let manager = MetadataManager::new(Some(tmp.path().to_string_lossy().to_string())).unwrap();

        // First update
        manager.update_session().unwrap();
        let metadata = manager.read_metadata().unwrap();
        assert_eq!(metadata.session_count, 1);
        assert!(metadata.last_used.is_some());
        assert!(metadata.first_used.is_some());
        assert!(metadata.first_version.is_some());

        // Second update
        manager.update_session().unwrap();
        let metadata = manager.read_metadata().unwrap();
        assert_eq!(metadata.session_count, 2);
    }

    #[test]
    fn test_read_nonexistent_metadata() {
        let tmp = tempdir().unwrap();
        let manager = MetadataManager::new(Some(tmp.path().to_string_lossy().to_string())).unwrap();

        let metadata = manager.read_metadata().unwrap();
        assert_eq!(metadata.session_count, 0);
        assert!(metadata.last_used.is_none());
    }
}
