//! Session info tool implementation.

use rmcp::{model::*, ErrorData as McpError};

use crate::mcp::types::GetSessionInfoArgs;
use crate::session::AuthSessionStore;

/// Get current Augment session information
pub fn get_session_info(_args: GetSessionInfoArgs) -> Result<CallToolResult, McpError> {
    let session_store = match AuthSessionStore::new(None) {
        Ok(store) => store,
        Err(e) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {}",
                e
            ))]));
        }
    };

    let info = if session_store.is_logged_in() {
        match session_store.get_session() {
            Ok(Some(session)) => {
                format!(
                    "Logged in\nTenant URL: {}\nScopes: {:?}",
                    session.tenant_url, session.scopes
                )
            }
            _ => "Session exists but could not be read".to_string(),
        }
    } else {
        "Not logged in".to_string()
    };

    Ok(CallToolResult::success(vec![Content::text(info)]))
}
