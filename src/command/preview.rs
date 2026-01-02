use anyhow::Result;
use std::path::PathBuf;

use crate::cli;
use crate::workspace::WorkspaceManager;

pub async fn run_preview(workspace_root: Option<String>, verbose: bool) -> Result<()> {
    // Resolve workspace root
    let root_path = match workspace_root {
        Some(path) => PathBuf::from(path),
        None => {
            // Try to find git root, fall back to current directory
            cli::find_git_root().unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        }
    };

    if !root_path.exists() {
        anyhow::bail!("Workspace path does not exist: {}", root_path.display());
    }

    println!("Scanning workspace: {}\n", root_path.display());

    // Create workspace manager and scan
    let manager = WorkspaceManager::new(root_path);
    let blobs = manager.scan_and_collect().await?;

    // Calculate stats
    let total_files = blobs.len();
    let total_bytes: usize = blobs.iter().map(|b| b.content.len()).sum();

    // Format size
    let size_str = if total_bytes >= 1024 * 1024 {
        format!("{:.2} MB", total_bytes as f64 / (1024.0 * 1024.0))
    } else if total_bytes >= 1024 {
        format!("{:.2} KB", total_bytes as f64 / 1024.0)
    } else {
        format!("{} bytes", total_bytes)
    };

    println!("Summary:");
    println!("  Files to upload: {}", total_files);
    println!("  Total size: {}", size_str);

    // Check for potentially sensitive patterns that slipped through
    let sensitive_patterns = ["password", "secret", "credential", "api_key", "apikey"];
    let mut sensitive_files: Vec<&str> = Vec::new();
    for blob in &blobs {
        let lower_path = blob.path.to_lowercase();
        for pattern in &sensitive_patterns {
            if lower_path.contains(pattern) {
                sensitive_files.push(&blob.path);
                break;
            }
        }
    }

    if !sensitive_files.is_empty() {
        println!(
            "\n⚠️  Warning: {} file(s) may contain sensitive data:",
            sensitive_files.len()
        );
        for path in &sensitive_files {
            println!("    - {}", path);
        }
        println!("\n  Consider adding these to .gitignore or .augmentignore");
    }

    // Verbose mode: list all files
    if verbose {
        println!("\nFiles:");
        for blob in &blobs {
            let size = blob.content.len();
            let size_str = if size >= 1024 {
                format!("{:.1}K", size as f64 / 1024.0)
            } else {
                format!("{}B", size)
            };
            println!("  {:>8}  {}", size_str, blob.path);
        }
    } else if total_files > 0 {
        println!("\n  Use --verbose to see all files");
    }

    Ok(())
}
