//! Span-and-text extraction utilities used by the action/operation
//! emitters and the persist-restore codegen.
//!
//! Two functions, both `pub(crate)` so the rest of the codegen layer
//! can reach them via `interface_gen::extract_body_content` /
//! `interface_gen::extract_tagged_system_name`. External callers
//! today: `rust_system` (three sites — domain-field nested-system
//! detection in the restore path).

use crate::frame_c::compiler::frame_ast::Span;

/// Extract body content from source using span.
///
/// Strips the outer braces and extracts the inner content while
/// preserving consistent line-by-line indentation for proper
/// re-indentation by backends.
pub(crate) fn extract_body_content(source: &[u8], span: &Span) -> String {
    let bytes = &source[span.start..span.end];
    let content = String::from_utf8_lossy(bytes).to_string();

    // Strip outer braces if present
    let trimmed = content.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let inner = &trimmed[1..trimmed.len() - 1];

        let lines: Vec<&str> = inner.lines().collect();

        // Skip leading and trailing empty lines, but preserve internal
        // structure.
        let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(0);
        let end = lines
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map(|i| i + 1)
            .unwrap_or(lines.len());

        if start >= end {
            return String::new();
        }

        // Return lines with preserved indentation — the NativeBlock
        // emitter normalises whitespace downstream.
        lines[start..end].join("\n")
    } else {
        trimmed.to_string()
    }
}

/// Extract the child `@@System()` name from a domain field's
/// initializer text. Returns `Some("Counter")` for `@@Counter()`,
/// `@@Counter(args)`, etc. Returns `None` for any non-tagged-system
/// initializer (primitives, native constructors like `new Counter()`
/// after `expand_system_instantiation_in_domain` has already run,
/// etc.).
///
/// Used by persist codegen to detect domain fields holding nested
/// system instances. For those, `save_state` recurses into the child's
/// `saveState` and `restore_state` rebuilds via the child's
/// `restoreState` — preserving class identity through a JSON
/// round-trip that would otherwise produce a plain object dict.
pub(crate) fn extract_tagged_system_name(init: &str) -> Option<&str> {
    let s = init.trim();
    let rest = s.strip_prefix("@@")?;
    let end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}
