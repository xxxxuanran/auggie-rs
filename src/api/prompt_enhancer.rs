//! Prompt enhancer functionality for the Augment API.
//!
//! This module handles streaming communication with the prompt-enhancer endpoint.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use std::time::Duration;
use tracing::{debug, error};
use url::Url;
use uuid::Uuid;

use super::types::{
    ChatHistoryExchange, PromptEnhancerChunk, PromptEnhancerNode, PromptEnhancerRequest,
    PromptEnhancerResult, PromptEnhancerTextNode,
};

/// Timeout for prompt enhancer requests (300 seconds / 5 minutes)
const PROMPT_ENHANCER_TIMEOUT_SECS: u64 = 300;

/// Call the prompt-enhancer endpoint
///
/// This endpoint enhances/rewrites a user prompt to be clearer and more specific.
/// The response is streamed, with each line being a JSON object containing a `text` field.
pub async fn call_prompt_enhancer(
    user_agent: &str,
    session_id: &str,
    tenant_url: &str,
    access_token: &str,
    prompt: String,
    chat_history: Option<Vec<ChatHistoryExchange>>,
    conversation_id: Option<String>,
    model: Option<String>,
) -> Result<PromptEnhancerResult> {
    let base =
        Url::parse(tenant_url).with_context(|| format!("Invalid tenant URL: {}", tenant_url))?;
    let url = base
        .join("prompt-enhancer")
        .with_context(|| "Failed to build prompt-enhancer URL")?;

    let request_id = Uuid::new_v4().to_string();

    // Build the request body
    let request_body = PromptEnhancerRequest {
        nodes: vec![PromptEnhancerNode {
            id: 0,
            node_type: 0,
            text_node: PromptEnhancerTextNode {
                content: prompt.clone(),
            },
        }],
        chat_history: chat_history.unwrap_or_default(),
        conversation_id,
        model,
        mode: "CHAT".to_string(),
    };

    // Log request details
    debug!("=== Prompt Enhancer Request ===");
    debug!("URL: {}", url);
    debug!("Timeout: {}s", PROMPT_ENHANCER_TIMEOUT_SECS);

    // Create a client with the appropriate timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(PROMPT_ENHANCER_TIMEOUT_SECS))
        .build()?;

    let response = super::send_with_retry(|| {
        client
            .post(url.clone())
            .header("Content-Type", "application/json")
            .header("User-Agent", user_agent)
            .header("x-request-id", &request_id)
            .header("x-request-session-id", session_id)
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&request_body)
    })
    .await
    .with_context(|| format!("Failed to send request to {}", url))?;

    let status = response.status();
    debug!("=== Prompt Enhancer Response ===");
    debug!("Status: {}", status);

    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        error!(
            "Prompt enhancer API request failed with status {}: {}",
            status, error_text
        );
        anyhow::bail!(
            "Prompt enhancer API request failed with status {}: {}",
            status,
            error_text
        );
    }

    // Process streaming response
    let enhanced_text = process_streaming_response(response).await?;
    let enhanced_prompt = enhanced_text.trim().to_string();

    if enhanced_prompt.is_empty() {
        anyhow::bail!("Prompt enhancer returned empty response");
    }

    debug!("Enhanced prompt length: {}", enhanced_prompt.len());
    Ok(PromptEnhancerResult { enhanced_prompt })
}

/// Process a streaming response and extract the enhanced text
async fn process_streaming_response(response: reqwest::Response) -> Result<String> {
    let mut enhanced_text = String::new();
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("Failed to read response chunk")?;
        let chunk_str = String::from_utf8_lossy(&chunk);
        buffer.push_str(&chunk_str);

        // Process complete lines from buffer
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            // Parse JSON and extract text
            if let Ok(chunk_data) = serde_json::from_str::<PromptEnhancerChunk>(&line) {
                if let Some(text) = chunk_data.text {
                    enhanced_text.push_str(&text);
                }
            }
        }
    }

    // Process any remaining data in buffer
    if !buffer.trim().is_empty() {
        if let Ok(chunk_data) = serde_json::from_str::<PromptEnhancerChunk>(buffer.trim()) {
            if let Some(text) = chunk_data.text {
                enhanced_text.push_str(&text);
            }
        }
    }

    Ok(enhanced_text)
}
