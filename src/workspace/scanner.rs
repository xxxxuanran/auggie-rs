//! File scanning functionality for workspace indexing.
//!
//! This module provides file system scanning utilities for collecting
//! files that need to be uploaded to the Augment backend.
//!
//! Uses `ignore::WalkBuilder` for recursive .gitignore support,
//! matching augment.mjs's ignoreTree behavior (see augment.mjs:293290).

use crate::workspace::cache::{compute_blob_name, BlobsCache, FileBlob};
use crate::workspace::manager::DEFAULT_AUGMENT_RULES;
use ignore::gitignore::Gitignore;
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;
use tracing::{debug, warn};

/// Maximum blob size in bytes.
pub const MAX_BLOB_SIZE: usize = 128 * 1024;

/// Maximum lines per blob when splitting large files.
pub const MAX_LINES_PER_BLOB: usize = 800;

/// Maximum file size to read (1MB).
/// Files larger than this are skipped to avoid memory issues.
pub const MAX_READABLE_FILE_SIZE: u64 = 1024 * 1024;

/// Legacy alias (bytes).
#[allow(dead_code)]
pub const MAX_FILE_SIZE: u64 = MAX_BLOB_SIZE as u64;

fn base_path_for_cached_path(path: &str) -> &str {
    match path.find("#chunk") {
        Some(idx) => &path[..idx],
        None => path,
    }
}

fn split_content_into_chunks(content: &str) -> Vec<String> {
    if content.is_empty() {
        return vec![String::new()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_lines: usize = 0;
    let mut current_bytes: usize = 0;

    for line in content.split_inclusive('\n') {
        let line_bytes = line.len();
        let would_exceed_lines = current_lines >= MAX_LINES_PER_BLOB;
        let would_exceed_bytes = current_bytes + line_bytes > MAX_BLOB_SIZE;

        if !current.is_empty() && (would_exceed_lines || would_exceed_bytes) {
            chunks.push(std::mem::take(&mut current));
            current_lines = 0;
            current_bytes = 0;
        }

        current.push_str(line);
        current_lines += 1;
        current_bytes += line_bytes;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    if chunks.is_empty() {
        chunks.push(String::new());
    }

    chunks
}

/// Check if a path should be ignored based on default patterns and gitignore
pub fn should_ignore(
    path: &Path,
    ignore_patterns: &HashSet<String>,
    gitignore: Option<&Gitignore>,
) -> bool {
    // First check default ignore patterns (always applied)
    let matches_default = path.components().any(|c| {
        if let Some(s) = c.as_os_str().to_str() {
            ignore_patterns.contains(s)
        } else {
            false
        }
    });

    if matches_default {
        return true;
    }

    // Then check gitignore patterns from .gitignore and .augmentignore
    if let Some(gitignore) = gitignore {
        let is_dir = path.is_dir();
        match gitignore.matched(path, is_dir) {
            ignore::Match::None => false,
            ignore::Match::Ignore(_) => true,
            ignore::Match::Whitelist(_) => false,
        }
    } else {
        false
    }
}

/// Build a WalkBuilder with all ignore rules configured.
///
/// This matches augment.mjs's three-layer ignore strategy:
/// 1. .gitignore (recursively in all directories)
/// 2. DEFAULT_AUGMENT_RULES (hardcoded sensitive file patterns)
/// 3. .augmentignore (at root, can override with !)
fn build_walker(root_path: &Path, ignore_patterns: &HashSet<String>) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root_path);

    // Enable standard gitignore processing (recursive)
    builder.standard_filters(true);
    builder.git_ignore(true);
    builder.git_global(true);
    builder.git_exclude(true);

    // Don't follow symlinks
    builder.follow_links(false);

    // Add .augmentignore support
    builder.add_custom_ignore_filename(".augmentignore");

    // Add DEFAULT_AUGMENT_RULES as global overrides
    // These patterns are ALWAYS applied (like augment.mjs LO class)
    let mut override_builder = OverrideBuilder::new(root_path);
    for pattern in DEFAULT_AUGMENT_RULES {
        // Convert to override format (! prefix means ignore)
        let ignore_pattern = format!("!{}", pattern);
        if let Err(e) = override_builder.add(&ignore_pattern) {
            warn!("Failed to add default Augment rule '{}': {}", pattern, e);
        }
    }
    if let Ok(overrides) = override_builder.build() {
        builder.overrides(overrides);
    }

    // Add directory patterns from ignore_patterns (legacy support)
    for pattern in ignore_patterns {
        // These are simple directory names like "node_modules", "target", etc.
        builder.add_ignore(root_path.join(pattern));
    }

    builder
}

/// Scan a workspace directory and collect file information.
///
/// Returns a list of FileBlobs with path, content, and blob_name.
/// This function walks the directory tree with recursive .gitignore support,
/// matching augment.mjs's ignoreTree behavior.
pub fn scan_workspace(
    root_path: &Path,
    ignore_patterns: &HashSet<String>,
    _gitignore: Option<&Gitignore>, // Legacy parameter, kept for API compatibility
) -> Vec<FileBlob> {
    let mut blobs = Vec::new();

    debug!("Scanning workspace: {}", root_path.display());

    let walker = build_walker(root_path, ignore_patterns);

    for entry in walker.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Error walking directory: {}", e);
                continue;
            }
        };

        let path = entry.path();

        // Only process files
        if !path.is_file() {
            continue;
        }

        blobs.extend(process_file(path, root_path));
    }

    debug!("Found {} files in workspace", blobs.len());

    blobs
}

/// Process a single file into a FileBlob.
///
/// Returns None if the file should be skipped (too large, binary, etc.)
fn process_file(path: &Path, root_path: &Path) -> Vec<FileBlob> {
    // Check file size and get mtime
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            warn!("Failed to get metadata for {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    // Skip files that are too large to avoid memory issues
    if metadata.len() > MAX_READABLE_FILE_SIZE {
        debug!(
            "Skipping large file ({} bytes): {}",
            metadata.len(),
            path.display()
        );
        return Vec::new();
    }

    // Get mtime from metadata
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // Read file content
    let content_bytes = match fs::read(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read file {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    // Try to convert to string, skip binary files
    let content = match String::from_utf8(content_bytes) {
        Ok(s) => s,
        Err(_) => {
            debug!("Skipping binary file: {}", path.display());
            return Vec::new();
        }
    };

    // Get relative path
    let relative_path = match path.strip_prefix(root_path) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => {
            warn!("Failed to get relative path for {}", path.display());
            return Vec::new();
        }
    };

    let chunks = split_content_into_chunks(&content);
    if chunks.len() == 1 {
        let blob_name = compute_blob_name(&relative_path, chunks[0].as_bytes());
        return vec![FileBlob {
            path: relative_path,
            content: chunks[0].clone(),
            blob_name,
            mtime,
        }];
    }

    let total_chunks = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(idx, chunk_content)| {
            let chunk_path = format!("{}#chunk{}of{}", relative_path, idx + 1, total_chunks);
            let blob_name = compute_blob_name(&chunk_path, chunk_content.as_bytes());
            FileBlob {
                path: chunk_path,
                content: chunk_content,
                blob_name,
                mtime,
            }
        })
        .collect()
}

/// Result of incremental workspace scan
pub struct ScanResult {
    /// Files that need to be uploaded (new or modified)
    pub to_upload: Vec<FileBlob>,
    /// Blob names of unchanged files (mtime didn't change)
    pub unchanged_blobs: Vec<String>,
    /// Paths of files that were deleted (in cache but not on disk)
    pub deleted_paths: Vec<String>,
}

/// Scan workspace incrementally using mtime to skip unchanged files.
///
/// This is much faster than full scan for large projects with few changes:
/// - Only reads file content when mtime changed
/// - Detects deleted files by comparing with cache
/// - Returns unchanged blob_names from cache
/// - Uses recursive .gitignore support (matching augment.mjs ignoreTree)
pub fn scan_workspace_incremental(
    root_path: &Path,
    cache: &BlobsCache,
    ignore_patterns: &HashSet<String>,
    _gitignore: Option<&Gitignore>, // Legacy parameter, kept for API compatibility
) -> ScanResult {
    let mut to_upload = Vec::new();
    let mut unchanged_blobs = Vec::new();
    let mut seen_cache_paths: HashSet<String> = HashSet::new();

    let mut cached_by_base_path: HashMap<
        String,
        Vec<(&String, &crate::workspace::cache::FileEntry)>,
    > = HashMap::new();
    for (cached_path, entry) in &cache.path_to_blob {
        let base = base_path_for_cached_path(cached_path);
        cached_by_base_path
            .entry(base.to_string())
            .or_default()
            .push((cached_path, entry));
    }

    debug!("Incremental scanning workspace: {}", root_path.display());

    let walker = build_walker(root_path, ignore_patterns);

    for entry in walker.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Error walking directory: {}", e);
                continue;
            }
        };

        let path = entry.path();

        // Only process files
        if !path.is_file() {
            continue;
        }

        // Get relative path
        let relative_path = match path.strip_prefix(root_path) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => {
                warn!("Failed to get relative path for {}", path.display());
                continue;
            }
        };

        // Get current mtime
        let current_mtime = match get_mtime(path) {
            Some(m) => m,
            None => {
                warn!("Failed to get mtime for {}", path.display());
                continue;
            }
        };

        if let Some(cached_group) = cached_by_base_path.get(&relative_path) {
            let all_match = cached_group
                .iter()
                .all(|(_p, entry)| entry.mtime == current_mtime);

            if all_match {
                for (cached_path, entry) in cached_group {
                    seen_cache_paths.insert((*cached_path).clone());
                    unchanged_blobs.push(entry.blob_name.clone());
                }
                continue;
            }

            for (cached_path, entry) in cached_group {
                debug!(
                    "File modified (mtime changed): {} ({} -> {})",
                    cached_path, entry.mtime, current_mtime
                );
            }
        }

        // Need to read content and compute hash (new file or mtime changed)
        let blobs = process_file(path, root_path);
        for blob in &blobs {
            seen_cache_paths.insert(blob.path.clone());
        }
        to_upload.extend(blobs);
    }

    // Find deleted files (in cache but not on disk)
    let deleted_paths: Vec<String> = cache
        .path_to_blob
        .keys()
        .filter(|p| !seen_cache_paths.contains(*p))
        .cloned()
        .collect();

    debug!(
        "Incremental scan: {} to upload, {} unchanged, {} deleted",
        to_upload.len(),
        unchanged_blobs.len(),
        deleted_paths.len()
    );

    ScanResult {
        to_upload,
        unchanged_blobs,
        deleted_paths,
    }
}

/// Get file modification time in milliseconds since epoch
fn get_mtime(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}
