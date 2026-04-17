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
mod _expr_scanner {
    include!("expr_scanner.gen.rs");
}

#[allow(unreachable_patterns)]
#[allow(unused_mut)]
#[allow(dead_code)]
#[allow(non_snake_case)]
#[allow(unused_variables)]
mod _context_parser {
    include!("context_parser.gen.rs");
}

#[allow(unreachable_patterns)]
#[allow(unused_mut)]
#[allow(dead_code)]
#[allow(non_snake_case)]
#[allow(unused_variables)]
mod _state_var_parser {
    include!("state_var_parser.gen.rs");
}

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

    /// Try to skip a nested function scope (closure/lambda) starting at position i.
    /// Returns Some(position after scope end) if a nested scope was found and skipped.
    /// Used to detect closures that would trap Frame statement return values.
    /// Default: None (no nested scope detection).
    fn skip_nested_scope(&self, _bytes: &[u8], _i: usize, _end: usize) -> Option<usize> {
        None
    }
}

/// Unified scanner that works with any language via the SyntaxSkipper trait
pub fn scan_native_regions<S: SyntaxSkipper>(
    skipper: &S,
    bytes: &[u8],
    open_brace_index: usize,
) -> Result<ScanResult, ScanError> {
    let mut closer = skipper.body_closer();
    let close = closer
        .close_byte(bytes, open_brace_index)
        .map_err(|e| ScanError {
            kind: ScanErrorKind::UnterminatedProtected,
            message: format!("{:?}", e),
        })?;

    let mut regions: Vec<Region> = Vec::new();
    let mut i = open_brace_index + 1;
    let end = close;
    let mut seg_start = i;

    while i < end {
        let b = bytes[i];

        // ===== NESTED SCOPE DETECTION =====
        // Skip closures/lambdas that would trap Frame statement return values.
        // Frame statements inside nested scopes are rejected with E407.
        if let Some(scope_end) = skipper.skip_nested_scope(bytes, i, end) {
            // Check if the skipped scope contains any Frame statement patterns
            let scope_bytes = &bytes[i..scope_end];
            let has_frame_stmt = scope_bytes.windows(3).any(|w| {
                // -> $ (transition)
                (w[0] == b'-' && w[1] == b'>' && w[2] == b' ')
                // => $ (forward)
                || (w[0] == b'=' && w[1] == b'>' && w[2] == b' ')
            }) || scope_bytes.windows(5).any(|w| {
                // push$
                (w[0] == b'p' && w[1] == b'u' && w[2] == b's' && w[3] == b'h' && w[4] == b'$')
                // pop$ (as standalone, not -> pop$)
                || (w[0] == b' ' && w[1] == b'p' && w[2] == b'o' && w[3] == b'p' && w[4] == b'$')
            });

            if has_frame_stmt {
                return Err(ScanError {
                    kind: ScanErrorKind::UnterminatedProtected,
                    message: "E407: Frame statement (transition, forward, push, pop) inside nested function scope. \
                              Frame control-flow statements must be directly in event handler scope, \
                              not inside closures or nested functions.".to_string(),
                });
            }

            // No Frame statements — skip the scope as native text
            i = scope_end;
            continue;
        }

        // ===== FRAME STATEMENT DETECTION =====
        // Frame statements (-> $, => $, push$, pop$, return) are detected at any position.
        // Closures are already skipped by skip_nested_scope() above.
        if matches!(b, b'-' | b'=' | b'(' | b'p' | b'r') {
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
                    while native_end > seg_start
                        && (bytes[native_end - 1] == b' ' || bytes[native_end - 1] == b'\t')
                    {
                        native_end -= 1;
                    }
                    if seg_start < native_end {
                        regions.push(Region::NativeText {
                            span: RegionSpan {
                                start: seg_start,
                                end: native_end,
                            },
                        });
                    }
                }

                // Find end of Frame statement
                let stmt_end = skipper.find_line_end(bytes, i, end);

                let stmt_bytes = &bytes[i..stmt_end];
                let stmt_text = String::from_utf8_lossy(stmt_bytes);
                let metadata = extract_segment_metadata(kind, &stmt_text);

                regions.push(Region::FrameSegment {
                    span: RegionSpan {
                        start: i,
                        end: stmt_end,
                    },
                    kind,
                    indent,
                    metadata,
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
                        span: RegionSpan {
                            start: seg_start,
                            end: i,
                        },
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
                let sv_bytes = &bytes[var_start..parser.result_end];
                let sv_text = String::from_utf8_lossy(sv_bytes);
                let metadata = extract_segment_metadata(kind, &sv_text);

                regions.push(Region::FrameSegment {
                    span: RegionSpan {
                        start: var_start,
                        end: parser.result_end,
                    },
                    kind,
                    indent: 0,
                    metadata,
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
                        span: RegionSpan {
                            start: seg_start,
                            end: i,
                        },
                    });
                }
                let ctx_start = i;

                // For @@SystemName(), pre-compute balanced_paren_end via the
                // language-specific SyntaxSkipper (the FSM can't call traits).
                let after_at = i + 2;
                let mut precomputed_paren_end: usize = 0;
                if after_at < end && bytes[after_at].is_ascii_uppercase() {
                    let mut name_end = after_at;
                    while name_end < end
                        && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
                    {
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
                        9 => FrameSegmentKind::ReturnCall,
                        10 => FrameSegmentKind::ContextSelfCall,
                        11 => FrameSegmentKind::ContextSelf,
                        12 => FrameSegmentKind::ContextSystemState,
                        13 => FrameSegmentKind::ContextSystemBare,
                        _ => FrameSegmentKind::ContextReturn, // shouldn't happen
                    };
                    let mut seg_end = parser.result_end;

                    // For `@@:(expr) return;` on the same source line,
                    // consume the trailing whitespace + `return` + optional `;`
                    // as part of the ContextReturnExpr segment. Without
                    // this the splicer leaves them as separate native
                    // text on the same line as the assignment, which
                    // most languages reject ("two statements on one
                    // line"). The expansion will emit both pieces with
                    // proper indentation.
                    if kind == FrameSegmentKind::ContextReturnExpr {
                        let mut p = seg_end;
                        // Skip trailing inline whitespace.
                        while p < end && (bytes[p] == b' ' || bytes[p] == b'\t') {
                            p += 1;
                        }
                        // Match `return` as a whole word.
                        if p + 6 <= end && &bytes[p..p + 6] == b"return" {
                            let after_return = p + 6;
                            // Make sure `return` is a whole word (not
                            // followed by an identifier character).
                            let is_word_boundary = after_return >= end
                                || !(bytes[after_return].is_ascii_alphanumeric()
                                    || bytes[after_return] == b'_');
                            if is_word_boundary {
                                let mut q = after_return;
                                // Skip whitespace after `return`.
                                while q < end && (bytes[q] == b' ' || bytes[q] == b'\t') {
                                    q += 1;
                                }
                                // Optional trailing `;`.
                                if q < end && bytes[q] == b';' {
                                    q += 1;
                                }
                                seg_end = q;
                            }
                        }
                    }

                    // Compute indent ONLY for ContextReturnExpr because
                    // its expansion uses it to indent the trailing
                    // `return` statement (the one introduced when the
                    // scanner consumed `@@:(expr) return;` as a single
                    // segment). All other @@ segments leave indent at 0
                    // because their expansions emit a single statement
                    // and the splicer's leading native text already
                    // carries the correct indentation.
                    let computed_indent = if kind == FrameSegmentKind::ContextReturnExpr
                        || kind == FrameSegmentKind::ReturnCall
                        || kind == FrameSegmentKind::ContextSelfCall
                    {
                        // Find the start of this line
                        let mut line_start = ctx_start;
                        while line_start > open_brace_index + 1 && bytes[line_start - 1] != b'\n' {
                            line_start -= 1;
                        }
                        if kind == FrameSegmentKind::ContextSelfCall {
                            // For self-calls, compute the line's leading whitespace
                            // (not the column of @@:) so the guard aligns with the statement
                            let mut ws = 0;
                            let mut p = line_start;
                            while p < ctx_start && (bytes[p] == b' ' || bytes[p] == b'\t') {
                                ws += 1;
                                p += 1;
                            }
                            ws
                        } else {
                            ctx_start - line_start
                        }
                    } else {
                        0
                    };

                    // Extract structured metadata from the segment text.
                    // This is the scanner's job — downstream stages consume
                    // metadata instead of re-parsing raw segment text.
                    let segment_bytes = &bytes[ctx_start..seg_end];
                    let segment_text = String::from_utf8_lossy(segment_bytes);
                    let metadata = extract_segment_metadata(kind, &segment_text);

                    regions.push(Region::FrameSegment {
                        span: RegionSpan {
                            start: ctx_start,
                            end: seg_end,
                        },
                        kind,
                        indent: computed_indent,
                        metadata,
                    });
                    i = seg_end;
                } else {
                    // No match — treat as native text
                    regions.push(Region::NativeText {
                        span: RegionSpan {
                            start: ctx_start,
                            end: parser.result_end,
                        },
                    });
                    i = parser.result_end;
                }
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
            span: RegionSpan {
                start: seg_start,
                end,
            },
        });
    }

    Ok(ScanResult {
        close_byte: close,
        regions,
    })
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
                return Some((k, FrameSegmentKind::Transition));
            }
        }

        // Check for -> pop$ (pop transition — this IS a transition, not standalone pop)
        if k + 3 < end
            && bytes[k] == b'p'
            && bytes[k + 1] == b'o'
            && bytes[k + 2] == b'p'
            && bytes[k + 3] == b'$'
        {
            return Some((k + 4, FrameSegmentKind::Transition));
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
                if bytes[k] == b'\\' && k + 1 < end {
                    k += 2;
                } else {
                    k += 1;
                }
            }
            if k < end {
                k += 1;
            } // Skip closing quote
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
    //
    // The trailing `$` is its own boundary (not a valid Rust/Python/Go
    // identifier char), but a JavaScript/TypeScript identifier of the
    // form `mypush$` would otherwise match `push$` as a suffix and
    // produce a spurious StackPush region. Require a leading word
    // boundary so the scanner only matches a standalone `push$`.
    if b == b'p'
        && pos + 4 < end
        && bytes[pos + 1] == b'u'
        && bytes[pos + 2] == b's'
        && bytes[pos + 3] == b'h'
        && bytes[pos + 4] == b'$'
    {
        let leading_boundary =
            pos == 0 || (!bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_');
        if leading_boundary {
            return Some((pos + 5, FrameSegmentKind::StackPush));
        }
    }

    // Stack pop (standalone): pop$
    //
    // Same rationale as `push$`: require a leading word boundary so
    // a JS-style identifier ending in `pop$` doesn't get misclassified.
    if b == b'p'
        && pos + 3 < end
        && bytes[pos + 1] == b'o'
        && bytes[pos + 2] == b'p'
        && bytes[pos + 3] == b'$'
    {
        let leading_boundary =
            pos == 0 || (!bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_');
        if leading_boundary {
            return Some((pos + 4, FrameSegmentKind::StackPop));
        }
    }

    // Return statement: return <expr>?
    //
    // Detected by the `return` keyword surrounded by word boundaries on
    // BOTH sides. The trailing-boundary check (next char not in
    // [A-Za-z0-9_]) prevents matching `returns` or `return_value`. The
    // LEADING-boundary check (previous char not in [A-Za-z0-9_]) is
    // equally important — without it, the byte-by-byte walker lands on
    // the `r` of `after_return` and matches `return` mid-identifier,
    // collapsing the Rust identifier into a Frame return statement.
    // Real-world casualty: the `output_block_parser.frs` source has
    // 15+ references to a local `after_return` flag, all of which got
    // shredded by this misclassification on regen, forcing the
    // `.gen.rs` to be hand-edited.
    //
    // Closures are already skipped by `skip_nested_scope()`, so this
    // is at handler scope. Position 0 is treated as a valid leading
    // boundary (start-of-buffer counts).
    if b == b'r'
        && pos + 5 < end
        && bytes[pos + 1] == b'e'
        && bytes[pos + 2] == b't'
        && bytes[pos + 3] == b'u'
        && bytes[pos + 4] == b'r'
        && bytes[pos + 5] == b'n'
    {
        let leading_boundary =
            pos == 0 || (!bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_');
        let after = pos + 6;
        let trailing_boundary =
            after >= end || (!bytes[after].is_ascii_alphanumeric() && bytes[after] != b'_');
        if leading_boundary && trailing_boundary {
            return Some((after, FrameSegmentKind::ReturnStatement));
        }
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

        if b == b';' || b == b'}' {
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
            if after_id >= end || bytes[after_id] == b'\n' || bytes[after_id] == b';' {
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
        if !matches!(
            next,
            b'Q' | b'q' | b'w' | b'W' | b'i' | b'I' | b's' | b'x' | b'r'
        ) {
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

/// Extract structured metadata from a Frame segment's raw text.
///
/// This is the scanner's parsing phase — it produces structured data that
/// downstream stages (codegen, validator, assembler) consume directly,
/// eliminating the need for re-parsing raw segment text.
fn extract_segment_metadata(kind: FrameSegmentKind, text: &str) -> SegmentMetadata {
    match kind {
        // --- Context accessors ---
        FrameSegmentKind::ContextParams => {
            // @@:params.key → extract key
            if let Some(rest) = text.strip_prefix("@@:params.") {
                let key: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                SegmentMetadata::ContextParams { key }
            } else {
                SegmentMetadata::None
            }
        }

        FrameSegmentKind::ContextData => {
            // @@:data.key → extract key
            if let Some(rest) = text.strip_prefix("@@:data.") {
                let key: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                SegmentMetadata::ContextData {
                    key,
                    assign_expr: None,
                }
            } else {
                SegmentMetadata::None
            }
        }

        FrameSegmentKind::ContextDataAssign => {
            // @@:data.key = expr → extract key and expr
            if let Some(rest) = text.strip_prefix("@@:data.") {
                let key: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let after_key = &rest[key.len()..];
                let expr = after_key
                    .trim()
                    .strip_prefix('=')
                    .map(|e| e.trim().trim_end_matches(';').trim().to_string());
                SegmentMetadata::ContextData {
                    key,
                    assign_expr: expr,
                }
            } else {
                SegmentMetadata::None
            }
        }

        FrameSegmentKind::ContextReturn => {
            // @@:return = expr (assignment) or @@:return (bare read)
            let trimmed = text.trim();
            if let Some(rest) = trimmed.strip_prefix("@@:return") {
                let rest = rest.trim();
                if rest.starts_with('=') && !rest.starts_with("==") {
                    let expr = rest[1..].trim().trim_end_matches(';').trim().to_string();
                    SegmentMetadata::ContextReturn {
                        assign_expr: Some(expr),
                    }
                } else {
                    SegmentMetadata::ContextReturn { assign_expr: None }
                }
            } else {
                SegmentMetadata::None
            }
        }

        FrameSegmentKind::ContextReturnExpr => {
            // @@:(expr) → extract the expression between parens
            let trimmed = text.trim();
            if let Some(start) = trimmed.find("@@:(") {
                let after_open = start + 4;
                let bytes = trimmed.as_bytes();
                let mut depth = 1i32;
                let mut p = after_open;
                while p < bytes.len() && depth > 0 {
                    match bytes[p] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    if depth > 0 {
                        p += 1;
                    }
                }
                let expr = trimmed[after_open..p].to_string();
                SegmentMetadata::ReturnExpr { expr }
            } else {
                SegmentMetadata::None
            }
        }

        FrameSegmentKind::ReturnCall => {
            // @@:return(expr) → extract expr
            let trimmed = text.trim();
            if let Some(rest) = trimmed.strip_prefix("@@:return(") {
                let expr = rest.trim_end_matches(')').to_string();
                SegmentMetadata::ReturnCall { expr }
            } else {
                SegmentMetadata::None
            }
        }

        // --- Self and system ---
        FrameSegmentKind::ContextSelfCall => {
            // @@:self.method(args) → extract method and args
            if let Some(rest) = text.strip_prefix("@@:self.") {
                if let Some(paren) = rest.find('(') {
                    let method = rest[..paren].to_string();
                    let args = rest[paren..].to_string(); // includes parens
                    SegmentMetadata::SelfCall { method, args }
                } else {
                    SegmentMetadata::None
                }
            } else {
                SegmentMetadata::None
            }
        }

        FrameSegmentKind::ContextSelf
        | FrameSegmentKind::ContextSystemState
        | FrameSegmentKind::ContextSystemBare
        | FrameSegmentKind::ContextEvent => {
            // These carry no variable content — the kind is sufficient
            SegmentMetadata::None
        }

        // --- State variables ---
        FrameSegmentKind::StateVar | FrameSegmentKind::StateVarAssign => {
            // $.varName or $.varName = expr → extract name
            if let Some(rest) = text.strip_prefix("$.") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                SegmentMetadata::StateVar { name }
            } else {
                SegmentMetadata::None
            }
        }

        // --- Transitions ---
        FrameSegmentKind::Transition => {
            // (exit)? -> (=>)? (enter)? ($State(state_args)? | pop$)
            let trimmed = text.trim();
            let has_pop = trimmed.contains("pop$");

            // Find target state: last $Uppercase identifier (empty for pop$)
            let mut target = String::new();
            let bytes = trimmed.as_bytes();
            let mut last_state_start = 0;
            for i in 0..bytes.len() {
                if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_uppercase() {
                    last_state_start = i;
                }
            }
            if last_state_start > 0 {
                let mut j = last_state_start + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                target = String::from_utf8_lossy(&bytes[last_state_start + 1..j]).to_string();
            }

            // Extract exit args: (args) before ->
            let arrow_pos = trimmed.find("->").unwrap_or(0);
            let before_arrow = &trimmed[..arrow_pos].trim();
            let exit_args = if before_arrow.starts_with('(') {
                let inner = before_arrow.trim_start_matches('(').trim_end_matches(')');
                if !inner.is_empty() {
                    Some(inner.to_string())
                } else {
                    None
                }
            } else {
                None
            };

            // Extract enter args: (args) between -> and $State
            let after_arrow = &trimmed[arrow_pos + 2..];
            let enter_args = if let Some(paren_start) = after_arrow.find('(') {
                // Check if this paren is before the $State
                let state_pos = after_arrow.find('$').unwrap_or(after_arrow.len());
                if paren_start < state_pos {
                    let paren_text = &after_arrow[paren_start..];
                    // Find matching close paren
                    let mut depth = 0;
                    let mut end = 0;
                    for (k, &b) in paren_text.as_bytes().iter().enumerate() {
                        if b == b'(' {
                            depth += 1;
                        }
                        if b == b')' {
                            depth -= 1;
                            if depth == 0 {
                                end = k + 1;
                                break;
                            }
                        }
                    }
                    let inner = &paren_text[1..end.saturating_sub(1)];
                    if !inner.is_empty() {
                        Some(inner.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Extract state args: $State(args) — parens after state name
            let state_args = if last_state_start > 0 {
                let mut j = last_state_start + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'(' {
                    let mut depth = 0;
                    let mut end = j;
                    for k in j..bytes.len() {
                        if bytes[k] == b'(' {
                            depth += 1;
                        }
                        if bytes[k] == b')' {
                            depth -= 1;
                            if depth == 0 {
                                end = k + 1;
                                break;
                            }
                        }
                    }
                    let inner =
                        String::from_utf8_lossy(&bytes[j + 1..end.saturating_sub(1)]).to_string();
                    if !inner.is_empty() {
                        Some(inner)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Check for label: -> "label" $State
            let label = after_arrow.find('"').and_then(|q_start| {
                let rest = &after_arrow[q_start + 1..];
                rest.find('"').map(|q_end| rest[..q_end].to_string())
            });

            // Detect => between -> and the target
            let is_forward = if let Some(ap) = trimmed.find("->") {
                let after = &trimmed[ap + 2..];
                let tp = after
                    .find('$')
                    .or_else(|| {
                        // For pop$, find "pop$" instead of "$"
                        after.find("pop$")
                    })
                    .unwrap_or(after.len());
                after[..tp].contains("=>")
            } else {
                false
            };

            SegmentMetadata::Transition {
                target_state: if has_pop { "pop$".to_string() } else { target },
                exit_args,
                enter_args,
                // state_args are meaningless on pop$ — the popped
                // compartment brings its own from the snapshot
                state_args: if has_pop { None } else { state_args },
                label,
                is_pop: has_pop,
                is_forward,
            }
        }

        // --- Tagged instantiation ---
        FrameSegmentKind::TaggedInstantiation => {
            // @@SystemName(args)
            if let Some(rest) = text.strip_prefix("@@") {
                if let Some(paren) = rest.find('(') {
                    let system_name = rest[..paren].to_string();
                    let args = rest[paren..].to_string();
                    SegmentMetadata::TaggedInstantiation { system_name, args }
                } else {
                    SegmentMetadata::None
                }
            } else {
                SegmentMetadata::None
            }
        }

        FrameSegmentKind::StackPush => {
            // Detect push-with-transition: `push$ -> $State`
            let transition_target = if let Some(arrow_pos) = text.find("->") {
                let after_arrow = &text[arrow_pos + 2..];
                let bytes = after_arrow.as_bytes();
                let mut target_start = None;
                for i in 0..bytes.len() {
                    if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_uppercase()
                    {
                        target_start = Some(i + 1);
                    }
                }
                target_start.map(|start| {
                    let after_dollar = &after_arrow[start..];
                    let end = after_dollar
                        .find(|c: char| !c.is_alphanumeric() && c != '_')
                        .unwrap_or(after_dollar.len());
                    after_dollar[..end].to_string()
                })
            } else {
                None
            };
            SegmentMetadata::StackPush { transition_target }
        }

        // --- Others ---
        FrameSegmentKind::Forward
        | FrameSegmentKind::StackPop
        | FrameSegmentKind::ReturnStatement => SegmentMetadata::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::native_region_scanner::rust::RustSkipper;

    /// Helper: scan a Rust handler body and return the kinds of all
    /// FrameSegment regions found, in order. The body is provided as a
    /// fully-formed string starting with `{` so the scanner can find
    /// the matching close brace via the body closer.
    fn scan_for_kinds(body: &str) -> Vec<FrameSegmentKind> {
        let bytes = body.as_bytes();
        let result = scan_native_regions(&RustSkipper, bytes, 0).expect("scan should succeed");
        result
            .regions
            .iter()
            .filter_map(|r| match r {
                Region::FrameSegment { kind, .. } => Some(*kind),
                Region::NativeText { .. } => None,
            })
            .collect()
    }

    /// Regression test for the `after_return` shred bug.
    ///
    /// Before the fix, `match_frame_statement` only checked the
    /// trailing word boundary after the `return` keyword. The
    /// byte-by-byte walker would land on the `r` of `after_return`
    /// inside `let after_return = false;` and match it as a Frame
    /// `return` statement, mangling the identifier. This was the
    /// reason `output_block_parser.gen.rs` had to be hand-edited
    /// after every regen — the .frs source's `after_return` flag
    /// (15+ references) got shredded.
    #[test]
    fn test_return_keyword_requires_leading_word_boundary() {
        // Identifier ending in `return` — must NOT be matched as a
        // Frame return statement.
        let body = "{ let after_return = false; }";
        let kinds = scan_for_kinds(body);
        assert!(
            !kinds.contains(&FrameSegmentKind::ReturnStatement),
            "`after_return = false` must not be classified as ReturnStatement, got: {:?}",
            kinds
        );
    }

    #[test]
    fn test_return_keyword_other_identifier_suffixes() {
        // Various other identifier shapes that contain the substring
        // `return` and would have been false-positively matched.
        let cases = [
            "{ let no_return = 1; }",
            "{ self.after_return = true; }",
            "{ if my_return { } }",
            "{ let _return = 0; }",
        ];
        for body in cases {
            let kinds = scan_for_kinds(body);
            assert!(
                !kinds.contains(&FrameSegmentKind::ReturnStatement),
                "Body {:?} must not produce a ReturnStatement, got: {:?}",
                body,
                kinds
            );
        }
    }

    #[test]
    fn test_return_keyword_still_matches_at_word_boundary() {
        // The fix must not break legitimate `return` matches. A bare
        // `return` after whitespace (the common case) and `return`
        // at the start of a line should still match.
        let cases = [
            "{ return; }",
            "{ return 42; }",
            "{ \n    return self.value;\n }",
        ];
        for body in cases {
            let kinds = scan_for_kinds(body);
            assert!(
                kinds.contains(&FrameSegmentKind::ReturnStatement),
                "Body {:?} should produce a ReturnStatement, got: {:?}",
                body,
                kinds
            );
        }
    }

    /// Same regression but for `push$` and `pop$`. JavaScript-style
    /// identifiers can contain `$`, so a name like `mypush$` would
    /// have collided with the `push$` keyword. Less likely to bite
    /// in practice (Rust doesn't allow `$` in identifiers) but worth
    /// guarding for backends that do.
    #[test]
    fn test_push_pop_require_leading_word_boundary() {
        let body = "{ let mypush$ = 1; let mypop$ = 2; }";
        // Note: this is invalid Rust but valid JS — and the scanner
        // shouldn't classify these as Frame stack-ops regardless of
        // target language. The scanner is language-agnostic about
        // identifier shape; the leading-boundary check makes the
        // detection conservative.
        let kinds = scan_for_kinds(body);
        assert!(
            !kinds.contains(&FrameSegmentKind::StackPush),
            "`mypush$` must not be classified as StackPush, got: {:?}",
            kinds
        );
        assert!(
            !kinds.contains(&FrameSegmentKind::StackPop),
            "`mypop$` must not be classified as StackPop, got: {:?}",
            kinds
        );
    }

    #[test]
    fn test_push_pop_still_match_at_word_boundary() {
        let body = "{ push$\n pop$ }";
        let kinds = scan_for_kinds(body);
        assert!(
            kinds.contains(&FrameSegmentKind::StackPush),
            "bare `push$` should still match, got: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&FrameSegmentKind::StackPop),
            "bare `pop$` should still match, got: {:?}",
            kinds
        );
    }

    #[test]
    fn test_self_call_recognized() {
        let kinds = scan_for_kinds("{ @@:self.reading() }");
        assert_eq!(kinds, vec![FrameSegmentKind::ContextSelfCall]);
    }

    #[test]
    fn test_self_call_with_args() {
        let kinds = scan_for_kinds("{ @@:self.process(a, b) }");
        assert_eq!(kinds, vec![FrameSegmentKind::ContextSelfCall]);
    }

    #[test]
    fn test_self_call_in_assignment() {
        let kinds = scan_for_kinds("{ let x = @@:self.getStatus(); }");
        assert_eq!(kinds, vec![FrameSegmentKind::ContextSelfCall]);
    }

    #[test]
    fn test_bare_self_recognized() {
        let kinds = scan_for_kinds("{ let s = @@:self; }");
        assert_eq!(kinds, vec![FrameSegmentKind::ContextSelf]);
    }

    #[test]
    fn test_self_call_with_nested_parens() {
        let kinds = scan_for_kinds("{ @@:self.process(foo(1, 2), bar()) }");
        assert_eq!(kinds, vec![FrameSegmentKind::ContextSelfCall]);
    }

    #[test]
    fn test_system_state_recognized() {
        let kinds = scan_for_kinds("{ let s = @@:system.state; }");
        assert_eq!(kinds, vec![FrameSegmentKind::ContextSystemState]);
    }

    #[test]
    fn test_system_state_in_return_expr() {
        let kinds = scan_for_kinds("{ @@:(@@:system.state) }");
        assert_eq!(kinds, vec![FrameSegmentKind::ContextReturnExpr]);
    }

    #[test]
    fn test_system_state_word_boundary() {
        // @@:system.stateX should NOT match — 'stateX' is not 'state'
        let kinds = scan_for_kinds("{ let s = @@:system.stateX; }");
        assert!(
            !kinds.contains(&FrameSegmentKind::ContextSystemState),
            "stateX should not match @@:system.state, got: {:?}",
            kinds
        );
    }

    #[test]
    fn test_system_unknown_property() {
        // @@:system.foo should not produce a match
        let kinds = scan_for_kinds("{ let s = @@:system.foo; }");
        assert!(
            !kinds.contains(&FrameSegmentKind::ContextSystemState),
            "@@:system.foo should not match, got: {:?}",
            kinds
        );
    }

    // ===== SegmentMetadata extraction tests =====

    /// Helper: scan a body and return metadata for all FrameSegments
    fn scan_for_metadata(body: &str) -> Vec<(FrameSegmentKind, SegmentMetadata)> {
        let bytes = body.as_bytes();
        let result = scan_native_regions(&RustSkipper, bytes, 0).expect("scan should succeed");
        result
            .regions
            .iter()
            .filter_map(|r| match r {
                Region::FrameSegment { kind, metadata, .. } => Some((*kind, metadata.clone())),
                Region::NativeText { .. } => None,
            })
            .collect()
    }

    #[test]
    fn test_metadata_context_params() {
        let metas = scan_for_metadata("{ let x = @@:params.age; }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::ContextParams { key } => assert_eq!(key, "age"),
            other => panic!("Expected ContextParams, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_context_data() {
        let metas = scan_for_metadata("{ let x = @@:data.msg; }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::ContextData { key, assign_expr } => {
                assert_eq!(key, "msg");
                assert!(assign_expr.is_none());
            }
            other => panic!("Expected ContextData, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_state_var() {
        let metas = scan_for_metadata("{ $.count }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::StateVar { name } => assert_eq!(name, "count"),
            other => panic!("Expected StateVar, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_self_call() {
        let metas = scan_for_metadata("{ @@:self.reading() }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::SelfCall { method, args } => {
                assert_eq!(method, "reading");
                assert_eq!(args, "()");
            }
            other => panic!("Expected SelfCall, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_self_call_with_args() {
        let metas = scan_for_metadata("{ @@:self.process(a, b) }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::SelfCall { method, args } => {
                assert_eq!(method, "process");
                assert_eq!(args, "(a, b)");
            }
            other => panic!("Expected SelfCall, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_transition_simple() {
        let metas = scan_for_metadata("{ -> $Active }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::Transition {
                target_state,
                is_pop,
                ..
            } => {
                assert_eq!(target_state, "Active");
                assert!(!is_pop);
            }
            other => panic!("Expected Transition, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_transition_pop() {
        let metas = scan_for_metadata("{ -> pop$ }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::Transition { is_pop, .. } => assert!(is_pop),
            other => panic!("Expected Transition with is_pop, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_transition_with_enter_args() {
        let metas = scan_for_metadata("{ -> (\"hello\") $Dialog }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::Transition {
                target_state,
                enter_args,
                ..
            } => {
                assert_eq!(target_state, "Dialog");
                assert!(enter_args.is_some());
            }
            other => panic!("Expected Transition, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_return_expr() {
        let metas = scan_for_metadata("{ @@:(42) }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::ReturnExpr { expr } => assert_eq!(expr, "42"),
            other => panic!("Expected ReturnExpr, got {:?}", other),
        }
    }

    #[test]
    fn test_metadata_tagged_instantiation() {
        let metas = scan_for_metadata("{ let x = @@Counter(10); }");
        assert_eq!(metas.len(), 1);
        match &metas[0].1 {
            SegmentMetadata::TaggedInstantiation { system_name, args } => {
                assert_eq!(system_name, "Counter");
                assert_eq!(args, "(10)");
            }
            other => panic!("Expected TaggedInstantiation, got {:?}", other),
        }
    }
}
