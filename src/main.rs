use anyhow::Result;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::{fmt, EnvFilter};

mod api;
mod cli;
mod command;
mod domain;
mod mcp;
mod metadata;
mod oauth;
mod runtime;
mod session;
mod startup;
mod telemetry;
mod workspace;

use api::{ApiCliMode, AuthenticatedClient};
use cli::{resolve_workspace_root, Cli, Commands};
use runtime::set_runtime;
use startup::StartupContext;
use workspace::create_shared_workspace_manager;

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
        // Run startup ensure flow first (auth, api, feature flags, metadata)
        // This matches augment.mjs: ensure() runs in main BEFORE Dgn()
        let mut startup_ctx = match StartupContext::new(ApiCliMode::Mcp, None) {
            Ok(ctx) => ctx,
            Err(e) => {
                warn!("Failed to create startup context: {}", e);
                // Degraded startup: run MCP server without runtime or workspace
                return mcp::run_mcp_server(None, None).await;
            }
        };

        let state = match startup_ctx.ensure_all().await {
            Ok(state) => state,
            Err(e) => {
                warn!("Startup validation failed: {}", e);
                info!("⚠️ Continuing without full validation - some tools may not work");

                if cli.model.is_some() {
                    warn!(
                        "Cannot validate --model={} without successful startup",
                        cli.model.as_deref().unwrap_or("")
                    );
                }

                // Degraded startup: no workspace initialization if ensure fails
                return mcp::run_mcp_server(None, None).await;
            }
        };

        // Resolve model using the loaded model_info_registry
        let resolved_model = state.resolve_model(cli.model.as_deref());
        if let Some(ref m) = resolved_model {
            info!("🎯 Using model: {}", m);
        }

        // Create authenticated client with stored credentials
        let client = AuthenticatedClient::new(
            ApiCliMode::Mcp,
            state.tenant_url().to_string(),
            state.access_token().to_string(),
        );

        // Store runtime in global singleton (like augment.mjs's fdt())
        set_runtime(state, client);

        // Initialize workspace (after ensure/runtime)
        let workspace_root = resolve_workspace_root(cli.workspace_root)?;
        info!("🔍 Initializing workspace at: {}", workspace_root.display());
        let workspace_manager = create_shared_workspace_manager(workspace_root);

        // Start background workspace init (load_state + sync_full)
        info!("🔄 Starting workspace initialization in background...");
        let wm = workspace_manager.clone();
        tokio::spawn(async move {
            let wm_guard = wm.read().await;
            wm_guard.initialize().await;
        });

        // Now call MCP server - it only handles server startup
        return mcp::run_mcp_server(Some(workspace_manager), resolved_model).await;
    }

    // Otherwise, handle subcommands
    match cli.command {
        Some(Commands::Login {
            login_url,
            augment_cache_dir,
        }) => {
            command::run_login(login_url, augment_cache_dir).await?;
        }
        Some(Commands::Logout) => {
            command::run_logout().await?;
        }
        Some(Commands::Status) => {
            command::run_status().await?;
        }
        Some(Commands::Preview {
            workspace_root,
            verbose,
        }) => {
            command::run_preview(workspace_root, verbose).await?;
        }
        None => {
            // No command specified, show help
            eprintln!("No command specified. Use --help for usage information.");
            eprintln!("Use 'auggie login' to authenticate or 'auggie --mcp' to start MCP server.");
        }
    }

    Ok(())
}
