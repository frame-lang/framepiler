//! Validation result types.
//!
//! Historically this module hosted the entire legacy validator (the
//! `Validator` struct + ~1200 lines of byte-scanning checks). With
//! `validate_module_with_mode` now delegating to the V4 pipeline
//! (`pipeline::compile_ast_based` → `frame_validator::FrameValidator`),
//! the legacy validator is unreachable from production. What remains
//! here are the public result types still used by the
//! `validate_module` API: `ValidationIssue` and `ValidationResult`.

#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub ok: bool,
    pub issues: Vec<ValidationIssue>,
}
