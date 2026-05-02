// Attribute scanner — wrapper module.
//
// Wires the Frame-generated state machine in `attribute_scanner.gen.rs`
// to the segmenter's pragma-identification path. Replaces the legacy
// hand-written `identify_pragma` in `segmenter/mod.rs` (which lives on
// in parallel during the A0 transition until parity is verified).
//
// Public API:
//
//   scan_attribute(bytes, start) -> AttributeSpan
//
// `start` is the byte offset of the first `@` of the `@@<name>` or
// `@@[name]` pragma. The returned span describes the surface form
// (bracket vs bare), the name span, and the optional args/value span.
// Callers translate the name bytes into `PragmaKind` via the lookup
// table they already maintain — keeping that mapping out of the
// scanner lets new attributes register in one place (the segmenter's
// classification table) without touching the FSM.
//
// Why FSM: see the header of `attribute_scanner.frs`. Same precedent
// as `body_closer/*.frs`, `native_region_scanner/*.frs`, and
// `gdscript_multisys/multisys_assembler.frs`.

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]
#![allow(clippy::get_first)]
#![allow(clippy::assign_op_pattern)]

include!("attribute_scanner.gen.rs");

// ---------------------------------------------------------------------
// Byte-class helpers used by the FSM. The Frame source calls these by
// bare name; the FSM module imports them via this `include!` shim.
// Same convention as `gdscript_multisys/mod.rs`'s `line_is_blank` etc.
// ---------------------------------------------------------------------

/// Characters that can appear in an attribute name. Names are
/// `[A-Za-z0-9_-]+` — alphanumeric plus underscore and hyphen. Hyphen
/// is included to keep `run-expect` and `skip-if` (existing bare-form
/// keywords) on the same path as the bracket form's identifier rule;
/// pre-existing kebab-case is preserved without special-casing.
pub(crate) fn is_attr_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Span describing an `@@<name>` or `@@[name(args)]` pragma at a given
/// byte position. Surface-form-agnostic — callers branch on
/// `is_bracket_form` if the distinction matters (e.g., for the
/// RFC-0013 hard-cut migration error reporting on bare `@@persist`).
#[derive(Debug, Clone)]
pub struct AttributeSpan {
    /// Surface form: `true` for `@@[name]`/`@@[name(args)]`, `false`
    /// for bare `@@name <value?>`.
    pub is_bracket_form: bool,

    /// Name span (always set). Byte offsets into the original input;
    /// `bytes[name_start..name_end]` is the attribute identifier
    /// (e.g., `b"persist"`, `b"target"`, `b"save"`).
    pub name_start: usize,
    pub name_end: usize,

    /// Args span (bracket form only). When present, covers the
    /// parentheses and their contents inclusive — e.g., for
    /// `@@[target("python_3")]` the span is `("python_3")`.
    /// `None` when the bracket form has no `(args)` block.
    pub args_span: Option<(usize, usize)>,

    /// Value span (bare form only). The trimmed rest-of-line after
    /// the keyword. `None` when the bare form has no value or the
    /// surface form is bracket.
    pub value_span: Option<(usize, usize)>,

    /// Position of the first byte AFTER the parsed pragma. For the
    /// bracket form this is just past the closing `]`; for the bare
    /// form it's at the end-of-line (the `\n` itself, if any).
    /// Lets callers resume scanning without re-walking.
    pub end_pos: usize,
}

impl AttributeSpan {
    /// Borrow the name bytes directly from the input slice. Avoids a
    /// copy at the lookup site since pragma classification is a
    /// straight `match` against byte literals.
    pub fn name<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.name_start..self.name_end]
    }

    /// Borrow the args bytes (without the surrounding parens), if
    /// present. Used for the `@@[target("python_3")]` → `"python_3"`
    /// extraction in the segmenter.
    pub fn args_inner<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        let (start, end) = self.args_span?;
        // span includes the surrounding parens; strip them
        if end > start + 1 && bytes[start] == b'(' && bytes[end - 1] == b')' {
            Some(&bytes[start + 1..end - 1])
        } else {
            Some(&bytes[start..end])
        }
    }

    /// Borrow the args bytes WITH surrounding parens. Used by callers
    /// that want to reconstruct the original `(args)` text verbatim
    /// (e.g., for `@@[persist(domain=[...])]` value-passthrough).
    pub fn args_with_parens<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        let (start, end) = self.args_span?;
        Some(&bytes[start..end])
    }

    /// Borrow the value bytes (bare form, post-trim). Used for
    /// `@@codegen { ... }`, `@@run-expect 42`, etc.
    pub fn value<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        let (start, end) = self.value_span?;
        Some(&bytes[start..end])
    }
}

/// Scan an attribute pragma at `start` (must point at the first `@`
/// of `@@`). Returns the parsed span — caller is responsible for
/// classifying the name into a `PragmaKind` and for any per-kind
/// downstream parsing of the args/value.
pub fn scan_attribute(bytes: &[u8], start: usize) -> AttributeSpan {
    let mut fsm = AttributeScannerFsm::new();
    fsm.bytes = bytes.to_vec();
    fsm.scan(start);
    AttributeSpan {
        is_bracket_form: fsm.is_bracket_form,
        name_start: fsm.name_start,
        name_end: fsm.name_end,
        args_span: if fsm.has_args {
            Some((fsm.args_start, fsm.args_end))
        } else {
            None
        },
        value_span: if fsm.has_value {
            Some((fsm.value_start, fsm.value_end))
        } else {
            None
        },
        end_pos: fsm.pos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bracket_form_no_args() {
        // `@@[persist]`
        let src = b"@@[persist]\n";
        let span = scan_attribute(src, 0);
        assert!(span.is_bracket_form);
        assert_eq!(span.name(src), b"persist");
        assert!(span.args_span.is_none());
        assert!(span.value_span.is_none());
        assert_eq!(span.end_pos, 11); // just past `]`
    }

    #[test]
    fn bracket_form_with_args() {
        // `@@[target("python_3")]`
        let src = b"@@[target(\"python_3\")]\n";
        let span = scan_attribute(src, 0);
        assert!(span.is_bracket_form);
        assert_eq!(span.name(src), b"target");
        assert_eq!(span.args_inner(src), Some(b"\"python_3\"".as_ref()));
        assert_eq!(span.args_with_parens(src), Some(b"(\"python_3\")".as_ref()));
        assert!(span.value_span.is_none());
    }

    #[test]
    fn bracket_form_nested_parens_in_args() {
        // `@@[migrate(from=foo(1), to=2)]` — depth tracking holds
        // the args block together across nested calls.
        let src = b"@@[migrate(from=foo(1), to=2)]\n";
        let span = scan_attribute(src, 0);
        assert!(span.is_bracket_form);
        assert_eq!(span.name(src), b"migrate");
        assert_eq!(span.args_inner(src), Some(b"from=foo(1), to=2".as_ref()));
    }

    #[test]
    fn bare_form_with_value() {
        // `@@target python_3`
        let src = b"@@target python_3\n";
        let span = scan_attribute(src, 0);
        assert!(!span.is_bracket_form);
        assert_eq!(span.name(src), b"target");
        assert_eq!(span.value(src), Some(b"python_3".as_ref()));
    }

    #[test]
    fn bare_form_no_value() {
        // `@@persist`
        let src = b"@@persist\n";
        let span = scan_attribute(src, 0);
        assert!(!span.is_bracket_form);
        assert_eq!(span.name(src), b"persist");
        assert!(span.value_span.is_none());
    }

    #[test]
    fn bare_form_trims_trailing_whitespace() {
        let src = b"@@target python_3   \r\n";
        let span = scan_attribute(src, 0);
        assert_eq!(span.value(src), Some(b"python_3".as_ref()));
    }

    #[test]
    fn bracket_form_with_whitespace_before_close() {
        // `@@[name(args) ]` — tolerated.
        let src = b"@@[target(\"go\") ]\n";
        let span = scan_attribute(src, 0);
        assert!(span.is_bracket_form);
        assert_eq!(span.name(src), b"target");
        assert_eq!(span.args_inner(src), Some(b"\"go\"".as_ref()));
    }

    #[test]
    fn bracket_form_main() {
        // RFC-0014 `@@[main]`
        let src = b"@@[main]\n";
        let span = scan_attribute(src, 0);
        assert!(span.is_bracket_form);
        assert_eq!(span.name(src), b"main");
        assert!(span.args_span.is_none());
    }

    #[test]
    fn rfc_0012_amendment_attributes() {
        for name in &[b"save".as_ref(), b"load", b"no_persist", b"migrate"] {
            let mut src = Vec::from(b"@@[".as_ref());
            src.extend_from_slice(name);
            src.extend_from_slice(b"]\n");
            let span = scan_attribute(&src, 0);
            assert!(span.is_bracket_form);
            assert_eq!(span.name(&src), *name);
        }
    }
}
