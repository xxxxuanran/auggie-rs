//! Startup initialization and ensure mechanism.
//!
//! This module implements a startup flow similar to augment.mjs:
//! - auth.ensure() - Validate authentication is available
//! - api.ensure() - Validate API connection
//! - featureFlags.ensure() - Fetch feature flags and model config
//!
//! The ensure mechanism provides fail-fast behavior: if any critical
//! validation fails, the server exits immediately with a clear error message.
//!
//! ## Runtime Configuration
//!
//! After startup, the validated state should be stored in the `runtime` module's
//! global singleton. See `crate::runtime` for the `ClientFeatureFlags`-like pattern.

mod ensure;
mod model_resolver;

pub use ensure::{EnsureError, EnsureResult, StartupContext, StartupState};
pub use model_resolver::{ModelInfoEntry, ModelInfoRegistry};
