//! JSON emitter for the semantic model.

use super::builder::{ModelOutput, SystemModel};

/// Emit the model as pretty-printed JSON.
pub fn emit_model_json(systems: Vec<SystemModel>) -> String {
    let output = ModelOutput {
        format: "frame-model".to_string(),
        version: "1.0".to_string(),
        systems,
    };
    serde_json::to_string_pretty(&output).unwrap_or_else(|e| {
        format!("{{\"error\": \"JSON serialization failed: {}\"}}", e)
    })
}
