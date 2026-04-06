//! `--format model` JSON output for Frame systems.
//!
//! Emits a semantic JSON model of the state machine structure.
//! Consumed by the VS Code extension Drawing Manager for UML rendering.

mod builder;
mod emitter;

pub use builder::build_system_model;
pub use emitter::emit_model_json;
