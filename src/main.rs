use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{fmt, EnvFilter};

mod api;
mod domain;
mod mcp;
mod metadata;
mod oauth;
mod session;
mod startup;
mod telemetry;
mod workspace;

use session::AuthSessionStore;
use workspace::WorkspaceManager;

/// Auggie CLI - MCP server with OAuth authentication
#[derive(Parser)]
#[command(name = "auggie")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Run as MCP server over stdio
    #[arg(long)]
    mcp: bool,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Workspace root (auto-detects git root if absent)
    #[arg(short = 'w', long)]
    workspace_root: Option<String>,

    /// Select model to use
    #[arg(short = 'm', long)]
    model: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    // If --mcp flag is set, run as MCP server
    if cli.mcp {
        return mcp::run_mcp_server(cli.workspace_root, cli.model).await;
    }

    // Otherwise, handle subcommands
    match cli.command {
        Some(Commands::Login {
            login_url,
            augment_cache_dir,
        }) => {
            run_login(login_url, augment_cache_dir).await?;
        }
        Some(Commands::Logout) => {
            run_logout().await?;
        }
        Some(Commands::Status) => {
            run_status().await?;
        }
        Some(Commands::Preview {
            workspace_root,
            verbose,
        }) => {
            run_preview(workspace_root, verbose).await?;
        }
        None => {
            // No command specified, show help
            eprintln!("No command specified. Use --help for usage information.");
            eprintln!("Use 'auggie login' to authenticate or 'auggie --mcp' to start MCP server.");
        }
    }

    Ok(())
}

async fn run_login(login_url: Option<String>, augment_cache_dir: Option<String>) -> Result<()> {
    let login_url = login_url.unwrap_or_else(|| oauth::DEFAULT_AUTH_URL.to_string());

    let session_store = AuthSessionStore::new(augment_cache_dir.clone())?;

    // Check if already logged in
    if session_store.is_logged_in() {
        println!("⚠️  You are already logged in to Augment.");
        println!("Re-authenticating will replace your current session.\n");

        print!("Do you want to continue with re-authentication? This will invalidate your existing session. [y/N]: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        let answer = answer.trim().to_lowercase();

        if answer != "y" && answer != "yes" {
            println!("Authentication cancelled. Your existing session remains active.");
            return Ok(());
        }

        println!("Removing existing session...");
        session_store.remove_session()?;
    }

    println!("🔐 Starting Augment authentication...\n");

    let api_client = api::ApiClient::new(None);
    let mut oauth_flow =
        oauth::OAuthFlow::new(&login_url, api_client, session_store, augment_cache_dir)?;

    // Start OAuth flow
    let authorize_url = oauth_flow.start_flow()?;

    // Ask user whether to open browser
    print!("Open authentication page in browser? [Y/n]: ");
    use std::io::{self, Write};
    io::stdout().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_lowercase();

    // Default to yes if user just presses Enter
    if answer.is_empty() || answer == "y" || answer == "yes" {
        println!("🌐 Opening authentication page in your browser...");
        if open::that(&authorize_url).is_err() {
            println!("⚠️  Could not open browser automatically.");
        }
    }

    println!("Please complete authentication in your browser:");
    println!("\n{}\n", authorize_url);
    println!("After authenticating, you will receive a JSON response.");
    println!("Copy the entire JSON response and paste it below.\n");

    print!("Paste the JSON response here: ");
    io::stdout().flush()?;

    let mut pasted = String::new();
    io::stdin().read_line(&mut pasted)?;
    let pasted = pasted.trim();

    oauth_flow.handle_auth_json(pasted).await?;

    println!("\n✅ Successfully authenticated with Augment!");

    Ok(())
}

async fn run_logout() -> Result<()> {
    let session_store = AuthSessionStore::new(None)?;

    if !session_store.is_logged_in() {
        println!("You are not logged in.");
        return Ok(());
    }

    session_store.remove_session()?;
    println!("✅ Successfully logged out from Augment.");

    Ok(())
}

async fn run_status() -> Result<()> {
    let session_store = AuthSessionStore::new(None)?;

    if session_store.is_logged_in() {
        if let Some(session) = session_store.get_session()? {
            println!("✅ Logged in to Augment");
            println!("   Tenant URL: {}", session.tenant_url);
            println!("   Scopes: {:?}", session.scopes);
        } else {
            println!("⚠️  Session file exists but is invalid.");
        }
    } else {
        println!("❌ Not logged in to Augment");
        println!("   Run 'auggie login' to authenticate.");
    }

    Ok(())
}

async fn run_preview(workspace_root: Option<String>, verbose: bool) -> Result<()> {
    // Resolve workspace root
    let root_path = match workspace_root {
        Some(path) => PathBuf::from(path),
        None => {
            // Try to find git root, fall back to current directory
            find_git_root().unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
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
        println!("\n⚠️  Warning: {} file(s) may contain sensitive data:", sensitive_files.len());
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

/// Find the git root directory by searching upward from current directory
fn find_git_root() -> Option<PathBuf> {
    let current = std::env::current_dir().ok()?;
    let mut path = current.as_path();

    loop {
        if path.join(".git").exists() {
            return Some(path.to_path_buf());
        }
        path = path.parent()?;
    }
}
