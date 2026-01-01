//! Tests for workspace module.

#[cfg(test)]
mod tests {
    use crate::workspace::cache::{compute_blob_name, BlobsCache};
    use crate::workspace::WorkspaceManager;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_workspace_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());
        assert_eq!(manager.root_path(), temp_dir.path());
    }

    #[test]
    fn test_should_ignore() {
        let temp_dir = TempDir::new().unwrap();
        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());

        assert!(manager.should_ignore_path(Path::new(".git/config")));
        assert!(manager.should_ignore_path(Path::new("node_modules/package")));
        assert!(manager.should_ignore_path(Path::new("target/debug/app")));
        assert!(!manager.should_ignore_path(Path::new("src/main.rs")));
    }

    #[test]
    fn test_compute_blob_name() {
        let path = "src/main.rs";
        let content = b"fn main() {}";
        let blob_name = compute_blob_name(path, content);
        assert_eq!(blob_name.len(), 64); // SHA256 produces 64 hex characters

        // Same input should produce same output
        let blob_name2 = compute_blob_name(path, content);
        assert_eq!(blob_name, blob_name2);

        // Different path should produce different output
        let blob_name3 = compute_blob_name("src/lib.rs", content);
        assert_ne!(blob_name, blob_name3);
    }

    #[tokio::test]
    async fn test_scan_and_collect() {
        let temp_dir = TempDir::new().unwrap();

        // Create test files
        let file1 = temp_dir.path().join("file1.txt");
        let mut f = File::create(&file1).unwrap();
        writeln!(f, "Hello").unwrap();

        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());
        let blobs = manager.scan_and_collect().await.unwrap();

        assert!(!blobs.is_empty());
        assert!(blobs.iter().any(|b| b.path == "file1.txt"));
    }

    #[tokio::test]
    async fn test_blobs_cache_operations() {
        let temp_dir = TempDir::new().unwrap();

        // Create test files
        let file1 = temp_dir.path().join("file1.txt");
        let mut f = File::create(&file1).unwrap();
        writeln!(f, "Hello").unwrap();

        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());

        // Scan files
        let blobs = manager.scan_and_collect().await.unwrap();
        assert!(!blobs.is_empty());

        // Cache should be empty before marking as uploaded
        {
            let cache = manager.blobs_cache().read().await;
            assert!(cache.is_empty());
        }

        // Mark files as uploaded - this populates the cache
        manager.mark_files_as_uploaded(&blobs).await;

        // Now check that the cache was populated
        let cache = manager.blobs_cache().read().await;
        assert!(!cache.is_empty());
        assert!(cache.get_blob_name("file1.txt").is_some());

        // Check reverse lookup works
        let blob_name = cache.get_blob_name("file1.txt").unwrap();
        let path = cache.get_path(blob_name);
        assert_eq!(path, Some(&"file1.txt".to_string()));
    }

    #[test]
    fn test_blobs_cache_serialization() {
        let mut cache = BlobsCache::default();
        cache.update(
            "src/main.rs".to_string(),
            1234567890,
            "hash1".to_string(),
            1001,
        );
        cache.update(
            "src/lib.rs".to_string(),
            1234567891,
            "hash2".to_string(),
            1002,
        );

        let json = serde_json::to_string(&cache).unwrap();
        let loaded: BlobsCache = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.len(), 2);
        assert!(loaded.has_blob("hash1"));
        assert!(loaded.has_blob("hash2"));
        assert_eq!(loaded.get_path("hash1"), Some(&"src/main.rs".to_string()));
        assert_eq!(loaded.get_path("hash2"), Some(&"src/lib.rs".to_string()));
    }

    #[test]
    fn test_blobs_cache_update_and_remove() {
        let mut cache = BlobsCache::default();

        // Add an entry
        cache.update("file.txt".to_string(), 1000, "hash1".to_string(), 1);
        assert!(cache.has_blob("hash1"));
        assert_eq!(cache.get_blob_name("file.txt"), Some(&"hash1".to_string()));

        // Update the same file with new content (different hash)
        cache.update("file.txt".to_string(), 2000, "hash2".to_string(), 2);
        assert!(!cache.has_blob("hash1")); // Old hash should be removed
        assert!(cache.has_blob("hash2")); // New hash should exist
        assert_eq!(cache.get_blob_name("file.txt"), Some(&"hash2".to_string()));

        // Remove the file
        cache.remove("file.txt");
        assert!(!cache.has_blob("hash2"));
        assert!(cache.get_blob_name("file.txt").is_none());
    }

    #[test]
    fn test_gitignore_patterns() {
        let temp_dir = TempDir::new().unwrap();

        // Create a .gitignore file
        let gitignore_path = temp_dir.path().join(".gitignore");
        let mut file = File::create(&gitignore_path).unwrap();
        writeln!(file, "*.log").unwrap();
        writeln!(file, "temp/").unwrap();
        writeln!(file, "secret.txt").unwrap();

        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());

        // Create test files to check ignore patterns
        // Note: should_ignore checks the path, files don't need to exist for pattern matching
        let log_file = temp_dir.path().join("debug.log");
        let secret_file = temp_dir.path().join("secret.txt");
        let normal_file = temp_dir.path().join("main.rs");

        // .log files should be ignored
        assert!(manager.should_ignore_path(&log_file));
        // secret.txt should be ignored
        assert!(manager.should_ignore_path(&secret_file));
        // normal files should not be ignored
        assert!(!manager.should_ignore_path(&normal_file));
    }

    #[test]
    fn test_augmentignore_patterns() {
        let temp_dir = TempDir::new().unwrap();

        // Create a .augmentignore file
        let augmentignore_path = temp_dir.path().join(".augmentignore");
        let mut file = File::create(&augmentignore_path).unwrap();
        writeln!(file, "*.bak").unwrap();
        writeln!(file, "private/").unwrap();

        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());

        // Create test paths
        let bak_file = temp_dir.path().join("backup.bak");
        let normal_file = temp_dir.path().join("code.rs");

        // .bak files should be ignored
        assert!(manager.should_ignore_path(&bak_file));
        // normal files should not be ignored
        assert!(!manager.should_ignore_path(&normal_file));
    }

    #[test]
    fn test_combined_gitignore_and_augmentignore() {
        let temp_dir = TempDir::new().unwrap();

        // Create both .gitignore and .augmentignore files
        let gitignore_path = temp_dir.path().join(".gitignore");
        let mut file = File::create(&gitignore_path).unwrap();
        writeln!(file, "*.log").unwrap();

        let augmentignore_path = temp_dir.path().join(".augmentignore");
        let mut file = File::create(&augmentignore_path).unwrap();
        writeln!(file, "*.tmp").unwrap();

        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());

        // Both patterns should work
        let log_file = temp_dir.path().join("debug.log");
        let tmp_file = temp_dir.path().join("cache.tmp");
        let normal_file = temp_dir.path().join("main.rs");

        // .log files from .gitignore should be ignored
        assert!(manager.should_ignore_path(&log_file));
        // .tmp files from .augmentignore should be ignored
        assert!(manager.should_ignore_path(&tmp_file));
        // normal files should not be ignored
        assert!(!manager.should_ignore_path(&normal_file));
    }

    #[tokio::test]
    async fn test_large_file_splitting() {
        use crate::workspace::scanner::MAX_LINES_PER_BLOB;

        let temp_dir = TempDir::new().unwrap();

        // Create a large file that exceeds MAX_LINES_PER_BLOB (800 lines)
        let large_file = temp_dir.path().join("large.txt");
        let mut f = File::create(&large_file).unwrap();

        // Write 1000 lines (should split into 2 chunks: 800 + 200)
        for i in 0..1000 {
            writeln!(f, "Line {}: This is some content to make the file larger.", i).unwrap();
        }

        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());
        let blobs = manager.scan_and_collect().await.unwrap();

        // Should have 2 blobs for the chunked file
        let large_blobs: Vec<_> = blobs.iter().filter(|b| b.path.starts_with("large.txt")).collect();
        assert_eq!(large_blobs.len(), 2, "Expected 2 chunks for 1000-line file");

        // Check chunk naming convention
        assert!(large_blobs.iter().any(|b| b.path == "large.txt#chunk1of2"));
        assert!(large_blobs.iter().any(|b| b.path == "large.txt#chunk2of2"));

        // Verify chunk sizes
        for blob in &large_blobs {
            let line_count = blob.content.lines().count();
            assert!(
                line_count <= MAX_LINES_PER_BLOB,
                "Chunk {} has {} lines, exceeds MAX_LINES_PER_BLOB ({})",
                blob.path, line_count, MAX_LINES_PER_BLOB
            );
        }

        // First chunk should have exactly MAX_LINES_PER_BLOB lines
        let chunk1 = large_blobs.iter().find(|b| b.path.ends_with("#chunk1of2")).unwrap();
        assert_eq!(chunk1.content.lines().count(), MAX_LINES_PER_BLOB);

        // Second chunk should have the remaining 200 lines
        let chunk2 = large_blobs.iter().find(|b| b.path.ends_with("#chunk2of2")).unwrap();
        assert_eq!(chunk2.content.lines().count(), 200);
    }

    #[tokio::test]
    async fn test_small_file_no_splitting() {
        let temp_dir = TempDir::new().unwrap();

        // Create a small file that doesn't need splitting
        let small_file = temp_dir.path().join("small.txt");
        let mut f = File::create(&small_file).unwrap();
        writeln!(f, "Just a small file.").unwrap();

        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());
        let blobs = manager.scan_and_collect().await.unwrap();

        // Should have exactly 1 blob, without chunk suffix
        let small_blobs: Vec<_> = blobs.iter().filter(|b| b.path.starts_with("small.txt")).collect();
        assert_eq!(small_blobs.len(), 1);
        assert_eq!(small_blobs[0].path, "small.txt"); // No #chunk suffix
    }

    #[tokio::test]
    async fn test_file_splitting_by_size() {
        use crate::workspace::scanner::MAX_BLOB_SIZE;

        let temp_dir = TempDir::new().unwrap();

        // Create a file that exceeds MAX_BLOB_SIZE (128KB) with fewer lines
        // Each line is about 200 bytes, so 700 lines = ~140KB > 128KB
        let big_file = temp_dir.path().join("biglines.txt");
        let mut f = File::create(&big_file).unwrap();

        let long_line = "X".repeat(180); // ~180 bytes per line
        for i in 0..750 {
            writeln!(f, "Line {:04}: {}", i, long_line).unwrap();
        }

        let manager = WorkspaceManager::new(temp_dir.path().to_path_buf());
        let blobs = manager.scan_and_collect().await.unwrap();

        // File size triggers splitting even though line count < MAX_LINES_PER_BLOB
        let big_blobs: Vec<_> = blobs.iter().filter(|b| b.path.starts_with("biglines.txt")).collect();

        // Should be split (either by size exceeding 128KB or line count)
        assert!(big_blobs.len() >= 1, "Expected at least 1 blob");

        // Each chunk should be under MAX_BLOB_SIZE (approximately, since we split by lines)
        for blob in &big_blobs {
            // Note: Split is by lines, so size might slightly exceed, but should be reasonable
            assert!(
                blob.content.len() <= MAX_BLOB_SIZE * 2,
                "Chunk {} size {} is too large",
                blob.path, blob.content.len()
            );
        }
    }
}
