//! Common utilities for MCP tools.

use rmcp::model::{CallToolResult, Content};

use crate::session::{AuthSessionStore, SessionData};

/// Error result for tool failures
pub fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.into())])
}

/// Get the current session, returning a tool error if not logged in.
///
/// This is a common pattern used by tools that require authentication.
pub fn require_session() -> Result<SessionData, CallToolResult> {
    let session_store = match AuthSessionStore::new(None) {
        Ok(store) => store,
        Err(e) => {
            return Err(tool_error(format!("Error accessing session: {}", e)));
        }
    };

    if !session_store.is_logged_in() {
        return Err(tool_error(
            "Error: Not logged in. Please run 'auggie login' first.",
        ));
    }

    match session_store.get_session() {
        Ok(Some(session)) => Ok(session),
        _ => Err(tool_error("Error: Could not read session information.")),
    }
}
