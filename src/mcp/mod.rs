//! MCP (Model Context Protocol) server implementation using rmcp.
//!
//! This module implements an MCP server using the official Rust MCP SDK (rmcp).
//! The server provides tools for codebase retrieval and prompt enhancement.

mod handlers;
mod server;
mod tools;
pub mod types;

// Re-export public items
pub use handlers::run_mcp_server;
pub use server::AuggieMcpServer;
