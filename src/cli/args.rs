use clap::{Parser, Subcommand};

/// Auggie CLI - MCP server with OAuth authentication
#[derive(Parser)]
#[command(name = "auggie")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Run as MCP server over stdio
    #[arg(long)]
    pub mcp: bool,

    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,

    /// Workspace root (auto-detects git root if absent)
    #[arg(short = 'w', long)]
    pub workspace_root: Option<String>,

    /// Select model to use
    #[arg(short = 'm', long)]
    pub model: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Authenticate with Augment using OAuth
    Login {
        /// Custom OAuth login URL (for debugging/dev deployments only)
        #[arg(long, hide = true)]
        login_url: Option<String>,

        /// Directory to store Augment cache files (session data, etc.). Defaults to ~/.augment
        #[arg(long)]
        augment_cache_dir: Option<String>,
    },
    /// Logout from Augment
    Logout,
    /// Show current session status
    Status,
    /// Preview files that will be uploaded (dry-run)
    Preview {
        /// Workspace root (defaults to current directory or git root)
        #[arg(short = 'w', long)]
        workspace_root: Option<String>,

        /// Show all files (not just summary)
        #[arg(short, long)]
        verbose: bool,
    },
}
