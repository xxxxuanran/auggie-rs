use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, EnvFilter};

mod api;
mod mcp;
mod oauth;
mod session;
mod telemetry;
mod workspace;

use session::AuthSessionStore;

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
