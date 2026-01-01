//! Echo tool implementation.

use rmcp::{model::*, ErrorData as McpError};

use crate::mcp::types::EchoArgs;

/// Echo back the input message
pub fn echo(args: EchoArgs) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::text(&args.message)]))
}
