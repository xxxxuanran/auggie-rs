//! Startup initialization and ensure mechanism.
//!
//! This module implements a startup flow similar to augment.mjs:
//! - auth.ensure() - Validate authentication is available
//! - api.ensure() - Validate API connection
//! - featureFlags.ensure() - Fetch feature flags and model config
//!
//! The ensure mechanism provides fail-fast behavior: if any critical
//! validation fails, the server exits immediately with a clear error message.

mod ensure;

pub use ensure::{EnsureError, EnsureResult, StartupContext, StartupState};
