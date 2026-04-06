use super::native_region_scanner::RegionSpan;
use super::validator::ValidationIssue;

#[derive(Debug, Default, Clone)]
pub struct ImportScanResult {
    pub spans: Vec<RegionSpan>,
    pub issues: Vec<ValidationIssue>,
}

pub trait ImportScanner {
    fn scan(&self, bytes: &[u8], start: usize) -> ImportScanResult;
}

// ============================================================================
// Shared helpers — called from generated .gen.rs FSMs and .rs wrappers
// ============================================================================

/// Check if bytes at position `i` start with keyword `kw` followed by a word boundary.
pub fn starts_kw(bytes: &[u8], i: usize, kw: &[u8]) -> bool {
    let n = bytes.len();
    if i + kw.len() > n { return false; }
    if &bytes[i..i + kw.len()] != kw { return false; }
    // Require word boundary: next char must not be alphanumeric or underscore
    let j = i + kw.len();
    if j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') { return false; }
    true
}

/// Check if position `j` starts a Frame section keyword (system, machine, interface,
/// actions, operations, domain). Used by all import scanners to terminate scanning.
pub fn is_frame_section_start(bytes: &[u8], j: usize) -> bool {
    starts_kw(bytes, j, b"system")
        || starts_kw(bytes, j, b"machine")
        || starts_kw(bytes, j, b"interface")
        || starts_kw(bytes, j, b"actions")
        || starts_kw(bytes, j, b"operations")
        || starts_kw(bytes, j, b"domain")
}

pub mod python;
pub mod typescript;
pub mod csharp;
pub mod c;
pub mod cpp;
pub mod java;
pub mod rust;

// TODO: Import scanners needed for these languages:
// - Go: `import` keyword, `import ( ... )` grouped imports
// - JavaScript: `import` (same pattern as TypeScript, could share)
// - PHP: `use` namespace imports, `require`/`include`
// - Kotlin: `import` (same pattern as Java)
// - Swift: `import` keyword
// - Ruby: `require`, `require_relative`
// These languages currently work without import scanners — the pipeline
// falls back to not scanning imports, which is safe but means import
// statements aren't tracked in the SourceMap metadata.
