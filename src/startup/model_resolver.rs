//! Model resolver for matching user input to model IDs.
//!
//! This module implements the model resolution logic from augment.mjs:
//! 1. Match by shortName (e.g., "sonnet4.5" -> "claude-sonnet-4-5")
//! 2. Match by full id (e.g., "claude-sonnet-4-5")
//! 3. displayName matching returns error (no longer supported)
//! 4. Fall back to default if not found

use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, warn};

/// Model info entry from model_info_registry feature flag.
///
/// Example from get-models.json:
/// ```json
/// "claude-sonnet-4-5": {
///     "description": "Great for everyday tasks",
///     "disabled": false,
///     "displayName": "Sonnet 4.5",
///     "shortName": "sonnet4.5"
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfoEntry {
    /// Human-readable description
    #[serde(default)]
    pub description: Option<String>,

    /// Whether this model is disabled
    #[serde(default)]
    pub disabled: bool,

    /// Display name (e.g., "Sonnet 4.5")
    #[serde(default)]
    pub display_name: Option<String>,

    /// Short name for CLI use (e.g., "sonnet4.5")
    #[serde(default)]
    pub short_name: Option<String>,

    /// Whether this is the default model
    #[serde(default)]
    pub is_default: bool,

    /// Whether this is a new model
    #[serde(default)]
    pub is_new: bool,

    /// Whether this is a legacy model
    #[serde(default)]
    pub is_legacy_model: bool,

    /// Reason why the model is disabled
    #[serde(default)]
    pub disabled_reason: Option<String>,
}

/// Registry of available models, keyed by model ID.
pub type ModelInfoRegistry = HashMap<String, ModelInfoEntry>;

/// Result of model resolution.
#[derive(Debug, Clone)]
pub enum ModelResolution {
    /// Successfully resolved to a model ID
    Resolved {
        /// The resolved model ID
        id: String,
        /// The display name of the model
        display_name: Option<String>,
        /// How the model was matched
        matched_by: MatchedBy,
    },
    /// Model matched by displayName (no longer supported)
    DisplayNameNotSupported {
        /// The model ID that would have matched
        id: String,
        /// The display name
        display_name: Option<String>,
        /// The short name to use instead
        short_name: Option<String>,
    },
    /// Model not found
    NotFound,
    /// Input was "default"
    UseDefault,
}

/// How a model was matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedBy {
    ShortName,
    Id,
}

/// Parse model_info_registry from feature_flags JSON string.
pub fn parse_model_info_registry(json_str: &str) -> Option<ModelInfoRegistry> {
    match serde_json::from_str::<ModelInfoRegistry>(json_str) {
        Ok(registry) => {
            debug!(
                "Parsed model_info_registry with {} models",
                registry.len()
            );
            Some(registry)
        }
        Err(e) => {
            warn!("Failed to parse model_info_registry: {}", e);
            None
        }
    }
}

/// Resolve a user-provided model string to a model ID.
///
/// Matching priority (same as augment.mjs kB function):
/// 1. If input is "default" -> UseDefault
/// 2. Match by displayName -> DisplayNameNotSupported (error, suggest shortName)
/// 3. Match by shortName -> Resolved
/// 4. Match by full id -> Resolved
/// 5. No match -> NotFound
pub fn resolve_model(input: &str, registry: &ModelInfoRegistry) -> ModelResolution {
    let input = input.trim();

    // Handle "default" specially
    if input.eq_ignore_ascii_case("default") {
        return ModelResolution::UseDefault;
    }

    // First, check if input matches any displayName (no longer supported)
    for (id, info) in registry {
        if let Some(ref display_name) = info.display_name {
            if display_name == input {
                return ModelResolution::DisplayNameNotSupported {
                    id: id.clone(),
                    display_name: Some(display_name.clone()),
                    short_name: info.short_name.clone(),
                };
            }
        }
    }

    // Then, check if input matches any shortName
    for (id, info) in registry {
        if let Some(ref short_name) = info.short_name {
            if short_name == input {
                return ModelResolution::Resolved {
                    id: id.clone(),
                    display_name: info.display_name.clone(),
                    matched_by: MatchedBy::ShortName,
                };
            }
        }
    }

    // Finally, check if input matches a full model ID
    if let Some(info) = registry.get(input) {
        return ModelResolution::Resolved {
            id: input.to_string(),
            display_name: info.display_name.clone(),
            matched_by: MatchedBy::Id,
        };
    }

    ModelResolution::NotFound
}

/// Resolve user model input with fallback to default.
///
/// Returns the resolved model ID or None if should use API default.
pub fn resolve_model_with_fallback(
    user_input: Option<&str>,
    registry: &ModelInfoRegistry,
    default_model: Option<&str>,
) -> Option<String> {
    let input = match user_input {
        Some(s) if !s.trim().is_empty() => s,
        _ => return None, // No user input, use API default
    };

    match resolve_model(input, registry) {
        ModelResolution::Resolved { id, display_name, matched_by } => {
            // Check if model is disabled
            if let Some(info) = registry.get(&id) {
                if info.disabled {
                    let reason = info.disabled_reason.as_deref().unwrap_or("");
                    let name = display_name.as_deref().unwrap_or(&id);
                    if reason.is_empty() {
                        warn!("Model is disabled: {}. Falling back to default.", name);
                    } else {
                        warn!("Model is disabled: {} - {}. Falling back to default.", name, reason);
                    }
                    return default_model.map(|s| s.to_string());
                }
            }

            debug!(
                "Resolved model '{}' to '{}' (matched by {:?})",
                input, id, matched_by
            );
            Some(id)
        }
        ModelResolution::DisplayNameNotSupported { short_name, display_name, .. } => {
            let suggestion = short_name.as_deref().unwrap_or("the model short name or id");
            warn!(
                "Using a display name for --model is no longer supported. Use \"{}\" instead.",
                suggestion
            );
            // Fall back to default
            default_model.map(|s| s.to_string())
        }
        ModelResolution::NotFound => {
            warn!("Unknown model: \"{}\", falling back to default.", input);
            default_model.map(|s| s.to_string())
        }
        ModelResolution::UseDefault => {
            // Explicit "default" input
            default_model.map(|s| s.to_string())
        }
    }
}

/// Find the default model ID from the registry.
pub fn find_default_model(registry: &ModelInfoRegistry) -> Option<String> {
    for (id, info) in registry {
        if info.is_default && !info.disabled {
            return Some(id.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_registry() -> ModelInfoRegistry {
        let json = r#"{
            "claude-haiku-4-5": {
                "description": "Fast and efficient",
                "disabled": false,
                "displayName": "Haiku 4.5",
                "shortName": "haiku4.5"
            },
            "claude-sonnet-4-5": {
                "description": "Great for everyday tasks",
                "disabled": false,
                "displayName": "Sonnet 4.5",
                "shortName": "sonnet4.5",
                "isDefault": true
            },
            "claude-opus-4-5": {
                "description": "Best for complex tasks",
                "disabled": false,
                "displayName": "Claude Opus 4.5",
                "shortName": "opus4.5"
            },
            "disabled-model": {
                "description": "This model is disabled",
                "disabled": true,
                "displayName": "Disabled Model",
                "shortName": "disabled",
                "disabledReason": "Maintenance"
            }
        }"#;
        parse_model_info_registry(json).unwrap()
    }

    #[test]
    fn test_resolve_by_short_name() {
        let registry = sample_registry();
        match resolve_model("sonnet4.5", &registry) {
            ModelResolution::Resolved { id, matched_by, .. } => {
                assert_eq!(id, "claude-sonnet-4-5");
                assert_eq!(matched_by, MatchedBy::ShortName);
            }
            other => panic!("Expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_by_id() {
        let registry = sample_registry();
        match resolve_model("claude-opus-4-5", &registry) {
            ModelResolution::Resolved { id, matched_by, .. } => {
                assert_eq!(id, "claude-opus-4-5");
                assert_eq!(matched_by, MatchedBy::Id);
            }
            other => panic!("Expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_display_name_not_supported() {
        let registry = sample_registry();
        match resolve_model("Haiku 4.5", &registry) {
            ModelResolution::DisplayNameNotSupported { id, short_name, .. } => {
                assert_eq!(id, "claude-haiku-4-5");
                assert_eq!(short_name, Some("haiku4.5".to_string()));
            }
            other => panic!("Expected DisplayNameNotSupported, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_not_found() {
        let registry = sample_registry();
        match resolve_model("unknown-model", &registry) {
            ModelResolution::NotFound => {}
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_default() {
        let registry = sample_registry();
        match resolve_model("default", &registry) {
            ModelResolution::UseDefault => {}
            other => panic!("Expected UseDefault, got {:?}", other),
        }
    }

    #[test]
    fn test_find_default_model() {
        let registry = sample_registry();
        let default = find_default_model(&registry);
        assert_eq!(default, Some("claude-sonnet-4-5".to_string()));
    }

    #[test]
    fn test_resolve_with_fallback_disabled_model() {
        let registry = sample_registry();
        let result = resolve_model_with_fallback(
            Some("disabled"),
            &registry,
            Some("fallback-model"),
        );
        assert_eq!(result, Some("fallback-model".to_string()));
    }

    #[test]
    fn test_resolve_with_fallback_success() {
        let registry = sample_registry();
        let result = resolve_model_with_fallback(
            Some("opus4.5"),
            &registry,
            Some("fallback-model"),
        );
        assert_eq!(result, Some("claude-opus-4-5".to_string()));
    }

    #[test]
    fn test_resolve_with_fallback_no_input() {
        let registry = sample_registry();
        let result = resolve_model_with_fallback(
            None,
            &registry,
            Some("fallback-model"),
        );
        assert_eq!(result, None);
    }
}
