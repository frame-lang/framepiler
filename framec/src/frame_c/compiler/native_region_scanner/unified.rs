// Unified Native Region Scanner for Frame V4
//
// This module provides a single scanning implementation for all target languages.
// The only language-specific logic is how to skip comments and strings.
//
// Sub-machines (hierarchical state manager pattern):
//   ExprScannerFsm   — PDA for scanning assignment RHS expressions
//   ContextParserFsm — FSM for parsing @@ context constructs
//   StateVarParserFsm — FSM for parsing $.varName access/assignment

#[allow(unreachable_patterns)]
#[allow(unused_mut)]
#[allow(dead_code)]
#[allow(non_snake_case)]
#[allow(unused_variables)]
mod _expr_scanner { include!("expr_scanner.gen.rs"); }

#[allow(unreachable_patterns)]
#[allow(unused_mut)]
#[allow(dead_code)]
#[allow(non_snake_case)]
#[allow(unused_variables)]
mod _context_parser { include!("context_parser.gen.rs"); }

#[allow(unreachable_patterns)]
#[allow(unused_mut)]
#[allow(dead_code)]
#[allow(non_snake_case)]
#[allow(unused_variables)]
mod _state_var_parser { include!("state_var_parser.gen.rs"); }

use _context_parser::ContextParserFsm;
use _state_var_parser::StateVarParserFsm;

use super::*;
use crate::frame_c::compiler::body_closer::BodyCloser;

/// Language-specific syntax skipper trait.
/// Each language only needs to implement how to skip its comments and strings.
pub trait SyntaxSkipper {
    /// Get the body closer for this language
    fn body_closer(&self) -> Box<dyn BodyCloser>;

    /// Try to skip a comment starting at position i.
    /// Returns Some(new_position) if a comment was skipped, None otherwise.
    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize>;

    /// Try to skip a string literal starting at position i.
    /// Returns Some(new_position) if a string was skipped, None otherwise.
    fn skip_string(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize>;

    /// Find the end of a Frame statement line, respecting language-specific string syntax.
    /// Stops at newline, semicolon, or comment start.
    fn find_line_end(&self, bytes: &[u8], start: usize, end: usize) -> usize;

    /// Try to find matching close paren, respecting language-specific string syntax.
    /// Returns Some(position after ')') if balanced, None otherwise.
    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize>;
}

/// Unified scanner that works with any language via the SyntaxSkipper trait
pub fn scan_native_regions<S: SyntaxSkipper>(
    skipper: &S,
    bytes: &[u8],
    open_brace_index: usize,
) -> Result<ScanResult, ScanError> {
    let mut closer = skipper.body_closer();
    let close = closer.close_byte(bytes, open_brace_index)
        .map_err(|e| ScanError {
            kind: ScanErrorKind::UnterminatedProtected,
            message: format!("{:?}", e)
        })?;

    let mut regions: Vec<Region> = Vec::new();
    let mut i = open_brace_index + 1;
    let end = close;
    let mut seg_start = i;

    while i < end {
        let b = bytes[i];

        // ===== FRAME STATEMENT DETECTION =====
        // Frame statements (-> $, => $, push$, pop$) contain $ which makes them
        // unambiguous — detectable at any position.
        if matches!(b, b'-' | b'=' | b'(' | b'p') {
            if let Some((_new_i, kind)) = match_frame_statement(skipper, bytes, i, end) {
                // Calculate indent (count whitespace from start of current line)
                let mut line_start = i;
                while line_start > open_brace_index + 1 && bytes[line_start - 1] != b'\n' {
                    line_start -= 1;
                }
                let indent = i - line_start;

                // Emit any preceding native text
                if seg_start < i {
                    // Trim trailing whitespace from native text (it's indentation for the Frame statement)
                    let mut native_end = i;
                    while native_end > seg_start && (bytes[native_end - 1] == b' ' || bytes[native_end - 1] == b'\t') {
                        native_end -= 1;
                    }
                    if seg_start < native_end {
                        regions.push(Region::NativeText {
                            span: RegionSpan { start: seg_start, end: native_end }
                        });
                    }
                }

                // Find end of Frame statement
                let stmt_end = skipper.find_line_end(bytes, i, end);

                regions.push(Region::FrameSegment {
                    span: RegionSpan { start: i, end: stmt_end },
                    kind,
                    indent,
                });

                i = stmt_end;
                seg_start = i;
                continue;
            }
        }

        // ===== HANDLE CURRENT CHARACTER =====

        match b {
            b'\n' => {
                i += 1;
            }

            // Try language-specific comment skip
            _ if skipper.skip_comment(bytes, i, end).is_some() => {
                // Safe: is_some() guard guarantees Some
                i = skipper.skip_comment(bytes, i, end).unwrap_or(i + 1);
            }

            // Try language-specific string skip
            _ if skipper.skip_string(bytes, i, end).is_some() => {
                // Safe: is_some() guard guarantees Some
                i = skipper.skip_string(bytes, i, end).unwrap_or(i + 1);
            }

            // ===== MID-LINE FRAME CONSTRUCTS =====

            // State variable reference: $.varName or assignment: $.varName = expr
            // Delegates to StateVarParserFsm (hierarchical state manager pattern)
            b'$' if i + 1 < end && bytes[i + 1] == b'.' => {
                if seg_start < i {
                    regions.push(Region::NativeText {
                        span: RegionSpan { start: seg_start, end: i }
                    });
                }
                let var_start = i;
                let mut parser = StateVarParserFsm::new();
                parser.bytes = bytes[..end].to_vec();
                parser.pos = i;
                parser.end = end;
                parser.do_parse();

                let kind = if parser.is_assignment {
                    FrameSegmentKind::StateVarAssign
                } else {
                    FrameSegmentKind::StateVar
                };
                regions.push(Region::FrameSegment {
                    span: RegionSpan { start: var_start, end: parser.result_end },
                    kind,
                    indent: 0,
                });
                i = parser.result_end;
                seg_start = i;
                // parser is destroyed here (state manager pattern)
            }

            // System context syntax: @@ variants
            // Delegates to ContextParserFsm (hierarchical state manager pattern)
            b'@' if i + 1 < end && bytes[i + 1] == b'@' => {
                if seg_start < i {
                    regions.push(Region::NativeText {
                        span: RegionSpan { start: seg_start, end: i }
                    });
                }
                let ctx_start = i;

                // For @@SystemName(), pre-compute balanced_paren_end via the
                // language-specific SyntaxSkipper (the FSM can't call traits).
                let after_at = i + 2;
                let mut precomputed_paren_end: usize = 0;
                if after_at < end && bytes[after_at].is_ascii_uppercase() {
                    let mut name_end = after_at;
                    while name_end < end && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_') {
                        name_end += 1;
                    }
                    if name_end < end && bytes[name_end] == b'(' {
                        if let Some(pe) = skipper.balanced_paren_end(bytes, name_end, end) {
                            precomputed_paren_end = pe;
                        }
                    }
                }

                let mut parser = ContextParserFsm::new();
                parser.bytes = bytes[..end].to_vec();
                parser.pos = after_at; // position after @@
                parser.end = end;
                parser.paren_end = precomputed_paren_end;
                parser.do_parse();

                if parser.has_result {
                    let kind = match parser.result_kind {
                        2 => FrameSegmentKind::ContextReturn,
                        3 => FrameSegmentKind::ContextEvent,
                        4 => FrameSegmentKind::ContextData,
                        5 => FrameSegmentKind::ContextDataAssign,
                        6 => FrameSegmentKind::ContextParams,
                        7 => FrameSegmentKind::TaggedInstantiation,
                        8 => FrameSegmentKind::ContextReturnExpr,
                        _ => FrameSegmentKind::ContextReturn, // shouldn't happen
                    };
                    regions.push(Region::FrameSegment {
                        span: RegionSpan { start: ctx_start, end: parser.result_end },
                        kind,
                        indent: 0,
                    });
                } else {
                    // No match — treat as native text
                    regions.push(Region::NativeText {
                        span: RegionSpan { start: ctx_start, end: parser.result_end }
                    });
                }
                i = parser.result_end;
                seg_start = i;
                // parser is destroyed here (state manager pattern)
            }

            _ => {
                i += 1;
            }
        }
    }

    // Emit any remaining native text
    if seg_start < end {
        regions.push(Region::NativeText {
            span: RegionSpan { start: seg_start, end }
        });
    }

    Ok(ScanResult { close_byte: close, regions })
}

/// Match Frame statements at start of line.
/// Returns Some((new_position, kind)) if matched, None otherwise.
///
/// Handles both:
/// - Direct Frame statements: `-> $State`, `push$`, etc.
/// Match a Frame statement at the given position.
/// Frame statements are unambiguous token sequences that can be detected
/// at any position in a handler body (not just start-of-line).
fn match_frame_statement<S: SyntaxSkipper>(
    skipper: &S,
    bytes: &[u8],
    i: usize,
    end: usize,
) -> Option<(usize, FrameSegmentKind)> {
    let pos = i;

    if pos >= end {
        return None;
    }

    let b = bytes[pos];

    // Transition variants: -> $State, -> (args) $State, -> pop$, -> => $State
    if b == b'-' && pos + 1 < end && bytes[pos + 1] == b'>' {
        let mut k = skip_ws(bytes, pos + 2, end);

        // Check for -> => $State (transition forward)
        if k + 1 < end && bytes[k] == b'=' && bytes[k + 1] == b'>' {
            k = skip_ws(bytes, k + 2, end);
            if k < end && bytes[k] == b'$' {
                return Some((k, FrameSegmentKind::TransitionForward));
            }
        }

        // Check for -> pop$ (pop transition)
        if k + 3 < end && bytes[k] == b'p' && bytes[k + 1] == b'o' && bytes[k + 2] == b'p' && bytes[k + 3] == b'$' {
            return Some((k + 4, FrameSegmentKind::StackPop));
        }

        // Check for optional enter args: -> (args) $State
        if k < end && bytes[k] == b'(' {
            if let Some(k2) = skipper.balanced_paren_end(bytes, k, end) {
                k = skip_ws(bytes, k2, end);
            }
        }

        // Check for optional label: -> "label" $State or -> (args) "label" $State
        if k < end && (bytes[k] == b'"' || bytes[k] == b'\'') {
            let quote = bytes[k];
            k += 1;
            while k < end && bytes[k] != quote {
                if bytes[k] == b'\\' && k + 1 < end { k += 2; } else { k += 1; }
            }
            if k < end { k += 1; } // Skip closing quote
            k = skip_ws(bytes, k, end);
        }

        // Regular transition: -> $State
        if k < end && bytes[k] == b'$' {
            return Some((k, FrameSegmentKind::Transition));
        }
    }

    // Forward: => $^  (with any whitespace between => and $)
    if b == b'=' && pos + 1 < end && bytes[pos + 1] == b'>' {
        let k = skip_ws(bytes, pos + 2, end);
        if k < end && bytes[k] == b'$' {
            return Some((k, FrameSegmentKind::Forward));
        }
    }

    // Transition with leading exit args: (exit_args) -> (enter_args) $State
    if b == b'(' {
        if let Some(k) = skipper.balanced_paren_end(bytes, pos, end) {
            let mut k = skip_ws(bytes, k, end);
            if k + 1 < end && bytes[k] == b'-' && bytes[k + 1] == b'>' {
                k = skip_ws(bytes, k + 2, end);
                // Optional enter args
                if k < end && bytes[k] == b'(' {
                    if let Some(k2) = skipper.balanced_paren_end(bytes, k, end) {
                        k = skip_ws(bytes, k2, end);
                    }
                }
                if k < end && bytes[k] == b'$' {
                    return Some((k, FrameSegmentKind::Transition));
                }
            }
        }
    }

    // Stack push: push$
    if b == b'p' && pos + 4 < end
        && bytes[pos + 1] == b'u'
        && bytes[pos + 2] == b's'
        && bytes[pos + 3] == b'h'
        && bytes[pos + 4] == b'$'
    {
        return Some((pos + 5, FrameSegmentKind::StackPush));
    }

    // Stack pop (standalone): pop$
    if b == b'p' && pos + 3 < end
        && bytes[pos + 1] == b'o'
        && bytes[pos + 2] == b'p'
        && bytes[pos + 3] == b'$'
    {
        return Some((pos + 4, FrameSegmentKind::StackPop));
    }

    None
}

// ============================================================================
// Whitespace helper
// ============================================================================

/// Skip horizontal whitespace (spaces and tabs). Returns new position.
#[inline]
pub fn skip_ws(bytes: &[u8], mut pos: usize, end: usize) -> usize {
    while pos < end && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    pos
}

// ============================================================================
// Common helper implementations for C-like languages
// ============================================================================

/// Skip C-style line comment: // ...
pub fn skip_line_comment(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    if i + 1 < end && bytes[i] == b'/' && bytes[i + 1] == b'/' {
        let mut j = i + 2;
        while j < end && bytes[j] != b'\n' {
            j += 1;
        }
        Some(j)
    } else {
        None
    }
}

/// Skip C-style block comment: /* ... */
pub fn skip_block_comment(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    if i + 1 < end && bytes[i] == b'/' && bytes[i + 1] == b'*' {
        let mut j = i + 2;
        while j + 1 < end {
            if bytes[j] == b'*' && bytes[j + 1] == b'/' {
                return Some(j + 2);
            }
            j += 1;
        }
        Some(end) // Unterminated, consume rest
    } else {
        None
    }
}

/// Skip Python-style comment: # ...
pub fn skip_hash_comment(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    if bytes[i] == b'#' {
        let mut j = i + 1;
        while j < end && bytes[j] != b'\n' {
            j += 1;
        }
        Some(j)
    } else {
        None
    }
}

/// Skip simple string: 'x' or "x" with backslash escapes.
///
/// Handles single-quoted strings (Python, C char literals) and double-quoted
/// strings. For Rust, use `skip_rust_string` instead — this function does not
/// distinguish Rust lifetimes from char literals.
pub fn skip_simple_string(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    let b = bytes[i];
    if b == b'\'' || b == b'"' {
        let q = b;
        let mut j = i + 1;
        while j < end {
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == q {
                return Some(j + 1);
            }
            j += 1;
        }
        Some(end) // Unterminated
    } else {
        None
    }
}

/// Skip a Rust string or char literal, distinguishing from lifetime annotations.
///
/// Rust uses `'` for both char literals (`'a'`, `'\n'`) and lifetime annotations
/// (`'a`, `'static`, `'_`). This function only consumes `'` as a char literal
/// if the pattern matches `'X'` (single char) or `'\...'` (escape sequence).
/// Lifetimes return `None` so the scanner keeps going.
///
/// Double-quoted strings (`"..."`) are handled normally.
pub fn skip_rust_string(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    let b = bytes[i];
    if b == b'"' {
        // Double-quoted string — same as skip_simple_string
        let mut j = i + 1;
        while j < end {
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == b'"' {
                return Some(j + 1);
            }
            j += 1;
        }
        Some(end) // Unterminated
    } else if b == b'\'' {
        // Single quote: distinguish char literal from lifetime.
        let j = i + 1;
        if j >= end {
            return None;
        }
        if bytes[j] == b'\\' {
            // Escape sequence: '\n', '\t', '\x41', '\u{...}', etc.
            let mut k = j + 1;
            while k < end && k < j + 12 && bytes[k] != b'\'' {
                k += 1;
            }
            if k < end && bytes[k] == b'\'' {
                return Some(k + 1);
            }
            None // No closing quote — not a char literal
        } else if j + 1 < end && bytes[j + 1] == b'\'' {
            // Simple char literal: 'a', '1', '>', etc.
            Some(j + 2)
        } else {
            // Not a char literal — lifetime annotation ('a, 'static, '_)
            None
        }
    } else {
        None
    }
}

/// Skip Python triple-quoted string: '''x''' or """x"""
pub fn skip_triple_string(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    let b = bytes[i];
    if (b == b'\'' || b == b'"') && i + 2 < end && bytes[i + 1] == b && bytes[i + 2] == b {
        let q = b;
        let mut j = i + 3;
        while j + 2 < end {
            if bytes[j] == q && bytes[j + 1] == q && bytes[j + 2] == q {
                return Some(j + 3);
            }
            j += 1;
        }
        Some(end) // Unterminated
    } else {
        None
    }
}

/// Skip TypeScript/JS template literal: `...${...}...`
pub fn skip_template_literal(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    if bytes[i] == b'`' {
        let mut j = i + 1;
        let mut brace_depth = 0i32;
        while j < end {
            if bytes[j] == b'`' && brace_depth == 0 {
                return Some(j + 1);
            }
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == b'$' && j + 1 < end && bytes[j + 1] == b'{' {
                brace_depth += 1;
                j += 2;
                continue;
            }
            if bytes[j] == b'}' && brace_depth > 0 {
                brace_depth -= 1;
            }
            j += 1;
        }
        Some(end) // Unterminated
    } else {
        None
    }
}

/// Skip Rust raw string: r#"..."#
pub fn skip_rust_raw_string(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    if bytes[i] != b'r' {
        return None;
    }
    let mut j = i + 1;
    let mut hashes = 0usize;

    // Count leading #s
    while j < end && bytes[j] == b'#' {
        hashes += 1;
        j += 1;
    }

    // Must have opening "
    if j >= end || bytes[j] != b'"' {
        return None;
    }
    j += 1;

    // Find closing "###
    while j < end {
        if bytes[j] == b'"' {
            let mut k = j + 1;
            let mut matched = 0usize;
            while matched < hashes && k < end && bytes[k] == b'#' {
                matched += 1;
                k += 1;
            }
            if matched == hashes {
                return Some(k);
            }
        }
        j += 1;
    }
    Some(end) // Unterminated
}

/// Find end of Frame statement line for C-like languages
pub fn find_line_end_c_like(bytes: &[u8], mut j: usize, end: usize) -> usize {
    let mut in_string: Option<u8> = None;

    while j < end {
        let b = bytes[j];

        if b == b'\n' {
            break;
        }

        if let Some(q) = in_string {
            if b == b'\\' {
                j += 2;
                continue;
            }
            if b == q {
                in_string = None;
            }
            j += 1;
            continue;
        }

        if b == b';' {
            break;
        }
        if b == b'/' && j + 1 < end && (bytes[j + 1] == b'/' || bytes[j + 1] == b'*') {
            break;
        }
        if b == b'\'' || b == b'"' {
            in_string = Some(b);
        }
        j += 1;
    }
    j
}

/// Find end of Frame statement line for Python
pub fn find_line_end_python(bytes: &[u8], mut j: usize, end: usize) -> usize {
    let mut in_string: Option<u8> = None;

    while j < end {
        let b = bytes[j];

        if b == b'\n' {
            break;
        }

        if let Some(q) = in_string {
            if b == b'\\' {
                j += 2;
                continue;
            }
            if b == q {
                in_string = None;
            }
            j += 1;
            continue;
        }

        if b == b'#' || b == b';' {
            break;
        }
        if b == b'\'' || b == b'"' {
            in_string = Some(b);
        }
        j += 1;
    }
    j
}

/// Find balanced paren end for C-like languages
pub fn balanced_paren_end_c_like(bytes: &[u8], mut i: usize, end: usize) -> Option<usize> {
    if i >= end || bytes[i] != b'(' {
        return None;
    }

    let mut depth = 0i32;
    let mut in_string: Option<u8> = None;

    while i < end {
        let b = bytes[i];

        if let Some(q) = in_string {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                in_string = None;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' | b'"' => {
                in_string = Some(b);
                i += 1;
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

/// Skip PHP heredoc/nowdoc: <<<EOT ... EOT; or <<<'EOT' ... EOT;
/// Position `i` must be at the first `<` of `<<<`.
/// Returns Some(position after closing identifier + optional semicolon) or None if not a heredoc.
pub fn skip_php_heredoc(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    // Must start with <<<
    if i + 2 >= end || bytes[i] != b'<' || bytes[i + 1] != b'<' || bytes[i + 2] != b'<' {
        return None;
    }
    let mut j = i + 3;

    // Skip optional whitespace after <<<
    while j < end && bytes[j] == b' ' {
        j += 1;
    }

    // Determine if nowdoc (<<<'ID') or heredoc (<<<ID or <<<"ID")
    let is_quoted = j < end && (bytes[j] == b'\'' || bytes[j] == b'"');
    let quote_char = if is_quoted { bytes[j] } else { 0 };
    if is_quoted {
        j += 1; // skip opening quote
    }

    // Extract the identifier
    let id_start = j;
    while j < end && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    if j == id_start {
        return None; // No identifier
    }
    let identifier = &bytes[id_start..j];

    // Skip closing quote if present
    if is_quoted {
        if j >= end || bytes[j] != quote_char {
            return None; // Mismatched quote
        }
        j += 1;
    }

    // Must be followed by newline (possibly with trailing whitespace/semicolon)
    while j < end && bytes[j] != b'\n' {
        j += 1;
    }
    if j < end {
        j += 1; // skip the newline
    }

    // Scan for the closing identifier at the start of a line
    while j < end {
        // Check if this line starts with (optional whitespace +) the identifier
        let line_start = j;
        // Skip leading whitespace (PHP 7.3+ flexible heredoc)
        while j < end && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }

        // Check if identifier matches
        let remaining = end - j;
        if remaining >= identifier.len() && &bytes[j..j + identifier.len()] == identifier {
            let after_id = j + identifier.len();
            // Must be followed by ; or newline or EOF
            if after_id >= end
                || bytes[after_id] == b'\n'
                || bytes[after_id] == b';'
            {
                // Found the end — skip past the identifier + optional semicolon + newline
                let mut result = after_id;
                if result < end && bytes[result] == b';' {
                    result += 1;
                }
                if result < end && bytes[result] == b'\n' {
                    result += 1;
                }
                return Some(result);
            }
        }

        // Skip to next line
        while j < end && bytes[j] != b'\n' {
            j += 1;
        }
        if j < end {
            j += 1; // skip newline
        }
    }
    Some(end) // Unterminated heredoc — consume to end
}

/// Skip Ruby percent literal: %Q{...}, %q[...], %w(...), %i<...>, %r|...|, etc.
/// Position `i` must be at the `%` character.
/// Returns Some(position after closing delimiter) or None if not a percent literal.
pub fn skip_ruby_percent_literal(bytes: &[u8], i: usize, end: usize) -> Option<usize> {
    if i >= end || bytes[i] != b'%' {
        return None;
    }

    let mut j = i + 1;
    if j >= end {
        return None;
    }

    // Optional letter qualifier: Q, q, w, W, i, I, s, x, r
    let next = bytes[j];
    if next.is_ascii_alphabetic() {
        // Must be a recognized qualifier
        if !matches!(next, b'Q' | b'q' | b'w' | b'W' | b'i' | b'I' | b's' | b'x' | b'r') {
            return None;
        }
        j += 1;
        if j >= end {
            return None;
        }
    }

    // Determine the delimiter
    let open = bytes[j];
    let (close, is_paired) = match open {
        b'{' => (b'}', true),
        b'[' => (b']', true),
        b'(' => (b')', true),
        b'<' => (b'>', true),
        // Any non-alphanumeric, non-space character can be a delimiter
        c if !c.is_ascii_alphanumeric() && c != b' ' && c != b'\n' => (c, false),
        _ => return None,
    };
    j += 1; // skip opening delimiter

    if is_paired {
        // Paired delimiters support nesting
        let mut depth = 1i32;
        while j < end && depth > 0 {
            if bytes[j] == b'\\' {
                j += 2; // skip escaped character
                continue;
            }
            if bytes[j] == open {
                depth += 1;
            } else if bytes[j] == close {
                depth -= 1;
            }
            j += 1;
        }
        if depth == 0 {
            Some(j)
        } else {
            Some(end) // Unterminated
        }
    } else {
        // Non-paired: scan to matching delimiter
        while j < end {
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == close {
                return Some(j + 1);
            }
            j += 1;
        }
        Some(end) // Unterminated
    }
}
