use anyhow::{Context, Result};
use std::path::PathBuf;

/// Find the git root directory by searching upward from current directory.
pub fn find_git_root() -> Option<PathBuf> {
    let current = std::env::current_dir().ok()?;
    let mut path = current.as_path();

    loop {
        if path.join(".git").exists() {
            return Some(path.to_path_buf());
        }
        path = path.parent()?;
    }
}

/// Resolve the workspace root path for MCP server.
pub fn resolve_workspace_root(workspace_root: Option<String>) -> Result<PathBuf> {
    if let Some(path) = workspace_root {
        PathBuf::from(&path)
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize provided workspace root: {}", path))
    } else {
        Ok(find_git_root()
            .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory")))
    }
}
