// GDScript multi-system assembler — wrapper module.
//
// Wires the Frame-generated state machine in `multisys_assembler.gen.rs`
// to the assembler's per-system emit loop:
//
//   * `wrap_inner(name)` — every non-primary system. Strips the
//     leading `extends <Base>` line, emits `class <name> extends <Base>:`
//     as the wrapper, and indents every non-blank body line one level
//     (4 spaces). GDScript inner classes resolve sibling lookups, so
//     this lets non-primary systems reference each other by bare name
//     and lets the primary (script-level) system instantiate them.
//
// "Primary" selection is the assembler's job, not this module's.
// Today the assembler picks lexically-first; RFC-0014 introduces
// `@@[main]` to let developers mark the intended primary explicitly.
// This module just wraps whatever it's given.
//
// Why the FSM expression: per
// `docs/articles/research/Parsers_as_Composed_State_Machines.md`,
// bounded line-by-line walks like this are exactly the shape Frame
// state machines fit best — each state captures one phase of the
// rewrite (skip leading blanks → read `extends` → indent body),
// transitions are explicit, and the resulting `.frs` reads top-down
// instead of as a tangle of mode flags.

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("multisys_assembler.gen.rs");

/// Wrap a system's emission as a `class <name> extends <Base>:` inner
/// class. Caller is responsible for ordering — the first system in a
/// multi-system file must NOT go through this; use `emit_first`.
pub fn wrap_inner(name: &str, code: &str) -> String {
    let mut fsm = GDScriptMultiSysAssemblerFsm::new();
    fsm.bytes = code.as_bytes().to_vec();
    fsm.wrap_inner(name.to_string());
    fsm.out
}

// ---------------------------------------------------------------------
// Helpers used by the FSM. Pure byte-walking utilities live in Rust;
// the FSM in `multisys_assembler.frs` calls these by name. Keeping
// them here (not inline in the .frs handlers) lets the Frame source
// stay focused on state transitions, matching the project's other
// FSMs (see `body_closer/python.frs` calling `skip_hash_comment` etc.
// from its wrapper module).
// ---------------------------------------------------------------------

/// True when the line at `pos` is empty or whitespace-only up to the
/// next `\n` (or end-of-buffer).
pub(crate) fn line_is_blank(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            return true;
        }
        if b != b' ' && b != b'\t' && b != b'\r' {
            return false;
        }
        i += 1;
    }
    true
}

/// Index of the first byte after the next `\n` (or `bytes.len()` if
/// the buffer has no further newline). Idempotent at EOF.
pub(crate) fn next_line_start(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// Read the line at `pos` as a `String`, excluding the trailing `\n`
/// (and a preceding `\r` if present so CRLF inputs round-trip clean).
pub(crate) fn read_line_text(bytes: &[u8], pos: usize) -> String {
    let mut end = pos;
    while end < bytes.len() && bytes[end] != b'\n' {
        end += 1;
    }
    let mut last = end;
    if last > pos && bytes[last - 1] == b'\r' {
        last -= 1;
    }
    String::from_utf8_lossy(&bytes[pos..last]).into_owned()
}

/// True when the line at `pos` starts with `needle` (after no leading
/// whitespace). Codegen emits `extends Base` flush-left, so a strict
/// prefix check is correct for the inputs this FSM sees.
pub(crate) fn line_starts_with(bytes: &[u8], pos: usize, needle: &[u8]) -> bool {
    if pos + needle.len() > bytes.len() {
        return false;
    }
    &bytes[pos..pos + needle.len()] == needle
}

/// Strip the `extends ` prefix from a line and return the remainder
/// trimmed of trailing whitespace / `:` / `\r`. The caller has
/// already verified the prefix via `line_starts_with`.
pub(crate) fn strip_extends_prefix(line: &str) -> String {
    line.strip_prefix("extends ")
        .unwrap_or(line)
        .trim_end_matches(':')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_inner_indents_body_under_class_header() {
        // Use raw lines so string-continuation indentation doesn't
        // leak into the test input.
        let code = [
            "extends RefCounted",
            "",
            "class FooFrameEvent:",
            "    var x",
            "",
            "var __compartment",
            "",
            "func _init():",
            "    pass",
            "",
        ]
        .join("\n");
        let got = wrap_inner("Foo", &code);
        // Header at top of fragment.
        assert!(
            got.contains("\n\nclass Foo extends RefCounted:\n"),
            "missing wrapper header in:\n{got}"
        );
        // First body line indented one level (was indent 0).
        assert!(
            got.contains("    class FooFrameEvent:"),
            "FooFrameEvent not indented in:\n{got}"
        );
        // Helper-class body line was indent 4, now indent 8.
        assert!(
            got.contains("        var x"),
            "FooFrameEvent body not nested in:\n{got}"
        );
        // Blank lines preserved.
        assert!(got.contains("\n\n"), "blank lines lost in:\n{got}");
    }

    #[test]
    fn wrap_inner_defaults_to_refcounted_without_extends() {
        let code = "class Bare:\n    pass\n";
        let got = wrap_inner("Bare", code);
        assert!(
            got.contains("\n\nclass Bare extends RefCounted:\n"),
            "default base not RefCounted:\n{got}"
        );
        assert!(got.contains("    class Bare:"));
    }

    #[test]
    fn line_is_blank_handles_mixed_whitespace() {
        let s = b"   \nx\n\t\n\n";
        assert!(line_is_blank(s, 0));
        assert!(!line_is_blank(s, 4)); // 'x'
        assert!(line_is_blank(s, 6));
        assert!(line_is_blank(s, 8));
    }

    #[test]
    fn read_line_text_strips_cr() {
        let s = b"hello\r\nworld\n";
        assert_eq!(read_line_text(s, 0), "hello");
        assert_eq!(read_line_text(s, 7), "world");
    }
}
