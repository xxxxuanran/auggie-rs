use anyhow::Result;
use tracing::{debug, error};

use super::client::ApiClient;
use super::types::{
    RecordRequestEventsRequest, RequestEvent, ToolUseData, ToolUseEvent, ToolUseEventWrapper,
};

impl ApiClient {
    /// Record request events for telemetry
    ///
    /// This sends tool use events to the Augment backend for analytics.
    /// Events are grouped by request_id and sent in batches.
    pub async fn record_request_events(
        &self,
        tenant_url: &str,
        access_token: &str,
        events: Vec<ToolUseEvent>,
    ) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        // Group events by request_id
        let mut grouped: std::collections::HashMap<String, Vec<&ToolUseEvent>> =
            std::collections::HashMap::new();
        for event in &events {
            grouped
                .entry(event.request_id.clone())
                .or_default()
                .push(event);
        }

        // Send each group as a separate request
        for (request_id, group) in grouped {
            let request_events: Vec<RequestEvent> = group
                .into_iter()
                .map(|e| {
                    let tool_input_json = &e.tool_input;
                    RequestEvent {
                        time: e.event_time.to_rfc3339(),
                        event: ToolUseEventWrapper {
                            tool_use_data: ToolUseData {
                                tool_name: e.tool_name.clone(),
                                tool_use_id: e.tool_use_id.clone(),
                                tool_output_is_error: e.tool_output_is_error,
                                tool_run_duration_ms: e.tool_run_duration_ms,
                                tool_input: tool_input_json.clone(),
                                tool_input_len: tool_input_json.len(),
                                is_mcp_tool: e.is_mcp_tool,
                                conversation_id: e.conversation_id.clone(),
                                chat_history_length: e.chat_history_length,
                                tool_output_len: e.tool_output_len,
                                tool_lines_added: e.tool_lines_added,
                                tool_lines_deleted: e.tool_lines_deleted,
                                tool_use_diff: e.tool_use_diff.clone(),
                            },
                        },
                    }
                })
                .collect();

            let request_body = RecordRequestEventsRequest {
                events: request_events,
            };

            debug!("Sending {} events to record-request-events", events.len());

            let response = self
                .post_api_with_timeout(
                    "record-request-events",
                    tenant_url,
                    Some(access_token),
                    &request_body,
                    super::client::DEFAULT_TIMEOUT_SECS,
                    Some(&request_id),
                )
                .await?;

            let status = response.status();
            if !status.is_success() {
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                error!(
                    "record-request-events failed with status {}: {}",
                    status, error_text
                );
                // Don't fail the whole operation for telemetry errors
            } else {
                debug!(
                    "Successfully sent telemetry events for request {}",
                    request_id
                );
            }
        }

        Ok(())
    }
}
