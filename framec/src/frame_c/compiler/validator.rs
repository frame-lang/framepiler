//! Validation result types used by the `validate_module` API.

#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub ok: bool,
    pub issues: Vec<ValidationIssue>,
}
