//! Wrapper-call validation for generated Frame output.
//!
//! Validates that generated wrapper calls (__frame_transition, __frame_forward,
//! __frame_stack_*) have correct syntax: balanced parentheses, quoted state
//! names, proper semicolon usage per language.

use crate::frame_c::visitors::TargetLanguage;

#[derive(Debug, Clone)]
pub struct NativeDiagnostic {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

/// Validate wrapper calls in spliced output text.
/// Returns diagnostics for malformed wrappers.
pub fn validate_wrappers(spliced_text: &str, lang: TargetLanguage) -> Vec<NativeDiagnostic> {
    let require_semicolon = matches!(
        lang,
        TargetLanguage::TypeScript
            | TargetLanguage::JavaScript
            | TargetLanguage::C
            | TargetLanguage::Cpp
            | TargetLanguage::Java
            | TargetLanguage::CSharp
            | TargetLanguage::Rust
            | TargetLanguage::Php
    );
    let forbid_semicolon = matches!(lang, TargetLanguage::Python3 | TargetLanguage::Ruby);
    // Languages where wrapper validation applies
    let has_wrappers = matches!(
        lang,
        TargetLanguage::Python3
            | TargetLanguage::TypeScript
            | TargetLanguage::JavaScript
            | TargetLanguage::Rust
            | TargetLanguage::C
            | TargetLanguage::Cpp
            | TargetLanguage::Java
            | TargetLanguage::CSharp
            | TargetLanguage::Go
            | TargetLanguage::Php
            | TargetLanguage::Kotlin
            | TargetLanguage::Swift
            | TargetLanguage::Ruby
            | TargetLanguage::Lua
    );
    if !has_wrappers {
        return Vec::new();
    }

    let mut diags = Vec::new();
    let bytes = spliced_text.as_bytes();
    let mut i = 0usize;
    let n = bytes.len();

    while i < n {
        let line_start = i;
        while i < n && bytes[i] != b'\n' {
            i += 1;
        }
        let line_end = i;
        if i < n {
            i += 1;
        }

        // Trim leading whitespace
        let mut s = line_start;
        while s < line_end && (bytes[s] == b' ' || bytes[s] == b'\t') {
            s += 1;
        }
        if s >= line_end {
            continue;
        }

        let is_transition = starts_with(bytes, s, b"__frame_transition");
        let is_forward = starts_with(bytes, s, b"__frame_forward");
        let is_stack = starts_with(bytes, s, b"__frame_stack_");

        if !is_transition && !is_forward && !is_stack {
            continue;
        }

        // Check balanced parentheses
        if !has_balanced_parens(bytes, s, line_end) {
            diags.push(NativeDiagnostic {
                start: s,
                end: line_end,
                message: "unbalanced parentheses in wrapper".into(),
            });
            continue;
        }

        // Transition: check first arg is quoted state name
        if is_transition {
            if let Some((arg_start, arg_end)) = paren_payload(bytes, s, line_end) {
                if let Some((false, msg)) = check_transition_first_arg(bytes, arg_start, arg_end) {
                    diags.push(NativeDiagnostic {
                        start: s,
                        end: line_end,
                        message: msg,
                    });
                }
            }
        }

        // Forward/stack: check no arguments
        if is_forward || is_stack {
            if let Some((arg_start, arg_end)) = paren_payload(bytes, s, line_end) {
                if has_non_ws(bytes, arg_start, arg_end) {
                    diags.push(NativeDiagnostic {
                        start: s,
                        end: line_end,
                        message: "wrapper takes no arguments".into(),
                    });
                }
            }
        }

        // Semicolon checks
        let has_semi = ends_with_semicolon(bytes, s, line_end);
        if require_semicolon && !has_semi {
            diags.push(NativeDiagnostic {
                start: s,
                end: line_end,
                message: "missing semicolon terminator".into(),
            });
        }
        if forbid_semicolon && has_semi {
            diags.push(NativeDiagnostic {
                start: s,
                end: line_end,
                message: format!(
                    "semicolon not allowed in {} wrapper",
                    match lang {
                        TargetLanguage::Python3 => "Python",
                        TargetLanguage::Ruby => "Ruby",
                        _ => "this language's",
                    }
                ),
            });
        }
    }

    diags
}

// --- Helper functions ---

fn starts_with(hay: &[u8], start: usize, needle: &[u8]) -> bool {
    let m = start + needle.len();
    m <= hay.len() && &hay[start..m] == needle
}

fn has_balanced_parens(hay: &[u8], start: usize, end: usize) -> bool {
    let mut seen_open = false;
    let mut depth = 0i32;
    let mut i = start;
    while i < end {
        if hay[i] == b'(' {
            seen_open = true;
            depth += 1;
        }
        if hay[i] == b')' {
            depth -= 1;
        }
        i += 1;
    }
    !seen_open || depth == 0
}

fn ends_with_semicolon(hay: &[u8], start: usize, end: usize) -> bool {
    let mut i = end;
    while i > start {
        i -= 1;
        let b = hay[i];
        if b == b'\n' || b == b'\r' || b == b' ' || b == b'\t' {
            continue;
        }
        return b == b';';
    }
    false
}

fn has_non_ws(hay: &[u8], mut start: usize, mut end: usize) -> bool {
    while start < end && (hay[start] == b' ' || hay[start] == b'\t') {
        start += 1;
    }
    while end > start && (hay[end - 1] == b' ' || hay[end - 1] == b'\t') {
        end -= 1;
    }
    start < end
}

fn paren_payload(hay: &[u8], start: usize, end: usize) -> Option<(usize, usize)> {
    let mut i = start;
    while i < end && hay[i] != b'(' {
        i += 1;
    }
    if i >= end {
        return None;
    }
    let mut depth = 1i32;
    i += 1;
    while i < end {
        if hay[i] == b'(' {
            depth += 1;
        } else if hay[i] == b')' {
            depth -= 1;
            if depth == 0 {
                return Some((i - (i - start - 1) + start, i));
            }
        }
        i += 1;
    }
    None
}

fn check_transition_first_arg(
    hay: &[u8],
    arg_start: usize,
    arg_end: usize,
) -> Option<(bool, String)> {
    let mut i = arg_start;
    while i < arg_end && (hay[i] == b' ' || hay[i] == b'\t') {
        i += 1;
    }
    if i >= arg_end || (hay[i] != b'\'' && hay[i] != b'"') {
        return Some((
            false,
            "transition wrapper: first argument must be quoted state".into(),
        ));
    }
    let q = hay[i];
    i += 1;
    let name_start = i;
    while i < arg_end && (hay[i].is_ascii_alphanumeric() || hay[i] == b'_') {
        i += 1;
    }
    if i == name_start {
        return Some((false, "transition wrapper: empty state name".into()));
    }
    if !(hay[name_start].is_ascii_alphabetic() || hay[name_start] == b'_') {
        return Some((false, "transition wrapper: invalid state identifier".into()));
    }
    if i >= arg_end || hay[i] != q {
        return Some((
            false,
            "transition wrapper: first argument must be quoted state".into(),
        ));
    }
    Some((true, String::new()))
}
