//! API client for Augment services.
//!
//! This module provides HTTP client functionality for communicating with
//! Augment backend services, equivalent to the Eye class in augment.mjs.

mod agents;
mod batch_upload;
mod client;
mod get_models;
mod http;
mod prompt_enhancer;
mod record_request_events;
mod token;
mod types;

#[allow(unused_imports)]
pub use agents::AgentsApi;
pub use client::{ApiClient, CliMode};

pub use self::CliMode as ApiCliMode;

#[allow(unused_imports)]
pub use types::{
    ApiError, ApiStatus, BatchUploadBlob, BatchUploadResponse, ChatHistoryExchange,
    CodebaseRetrievalResponse, FeatureFlagsV1, FeatureFlagsV2, GetModelsResponse, GetModelsUser,
    ModelInfo, PromptEnhancerResult, ToolUseEvent, ValidationResult,
};
