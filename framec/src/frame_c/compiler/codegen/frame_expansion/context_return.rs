//! `@@:return` / `@@:return = expr` / `@@:(expr)` value-slot
//! expansion.
//!
//! Two arms — both target the per-call FrameContext's `_return`
//! slot (untyped in every static target, plain attribute in
//! dynamic targets):
//!
//! - `expand_context_return` — bare `@@:return` (typed read) or
//!   `@@:return = expr` (typed assign). Read path goes through
//!   `context_return_read_typed` so the per-target downcast to
//!   the handler's declared return type is uniform.
//! - `expand_context_return_expr` — `@@:(expr)` shorthand:
//!   evaluates expr and writes to `_return`, equivalent to
//!   `@@:return = expr`. Reads the expression's RHS through
//!   `expand_expression` so nested `@@:` constructs lower first.

use super::expand_expression;
use super::super::codegen_utils::{replace_outside_strings_and_comments, HandlerContext};
use super::utility::{c_return_assign, cpp_wrap_string_literal, paren_wrap_if_multiline};
use super::handler_body::context_return_read_typed;
use crate::frame_c::compiler::native_region_scanner::{RegionSpan, SegmentMetadata};
use crate::frame_c::visitors::TargetLanguage;

pub(super) fn expand_context_return(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:return - return value slot (assignment or read)
            // Determine if this is assignment or read from metadata (preferred) or text
            let is_assignment = if let SegmentMetadata::ContextReturn { assign_expr } = metadata {
                assign_expr.is_some()
            } else {
                let t = segment_text.trim();
                t.contains('=') && !t.contains("==")
            };
            let trimmed = segment_text.trim();
            if is_assignment {
                // Assignment: @@:return = expr
                let expr = if let SegmentMetadata::ContextReturn {
                    assign_expr: Some(e),
                } = metadata
                {
                    e.as_str()
                } else {
                    let eq_pos = trimmed.find('=').unwrap();
                    trimmed[eq_pos + 1..].trim().trim_end_matches(';').trim()
                };
                let expanded_expr = paren_wrap_if_multiline(&expand_expression(expr, lang, ctx));
                match lang {
                    TargetLanguage::Python3 | TargetLanguage::GDScript => format!(
                        "{}self._context_stack[-1]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::TypeScript
                    | TargetLanguage::Dart
                    | TargetLanguage::JavaScript => format!(
                        "{}this._context_stack[this._context_stack.length - 1]._return = {};",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::C => format!(
                        "{}{}",
                        indent_str,
                        c_return_assign(&ctx.system_name, &expanded_expr, &ctx.current_return_type),
                    ),
                    TargetLanguage::Rust => super::super::rust_system::rust_expand_box_return(
                        &indent_str,
                        &expanded_expr,
                        &ctx.current_return_type,
                        &ctx.system_name,
                        &ctx.event_name,
                    ),
                    TargetLanguage::Cpp => {
                        let wrapped = cpp_wrap_string_literal(&expanded_expr);
                        format!(
                            "{}_context_stack.back()._return = std::any({});",
                            indent_str, wrapped
                        )
                    }
                    TargetLanguage::Java => format!(
                        "{}_context_stack.get(_context_stack.size() - 1)._return = {};",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Kotlin => format!(
                        "{}_context_stack[_context_stack.size - 1]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Swift => format!(
                        "{}_context_stack[_context_stack.count - 1]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::CSharp => format!(
                        "{}_context_stack[_context_stack.Count - 1]._return = {};",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Go => {
                        // Go's generated methods use `s` as the receiver
                        // name, not `self`. Rewrite `self.` → `s.` via the
                        // string-literal-aware helper so a `self.` that
                        // happens to appear inside a string literal or
                        // comment isn't mangled.
                        let go_expr = replace_outside_strings_and_comments(
                            &expanded_expr,
                            TargetLanguage::Go,
                            &[("self.", "s.")],
                        );
                        format!(
                            "{}s._context_stack[len(s._context_stack)-1]._return = {}",
                            indent_str, go_expr
                        )
                    }
                    TargetLanguage::Php => format!(
                        "{}$this->_context_stack[count($this->_context_stack) - 1]->_return = {};",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Ruby => format!(
                        "{}@_context_stack[@_context_stack.length - 1]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Lua => format!(
                        "{}self._context_stack[#self._context_stack]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Erlang => {
                        // Leave `self.` in the expression untouched —
                        // the Erlang body processor classifies this as
                        // Plain and substitutes `self.` with the CURRENT
                        // `DataN#data.` using the live data_gen. A
                        // hardcoded `Data#data.` here would bind to the
                        // pre-handler Data and miss updates made earlier
                        // in the handler body (e.g., by `self.x = ...`
                        // or a preceding `@@:self` dispatch).
                        format!("{}__ReturnVal = {}", indent_str, expanded_expr)
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            } else {
                // Read: @@:return.
                //
                // The context-stack slot is an untyped `Any` / `Object`
                // / `void*` / `std::any` / `Option<Box<dyn Any>>` in
                // every typed target. Reading `@@:return` as that raw
                // slot fails as soon as the value hits an arithmetic
                // operator or a typed-method argument. Emit a
                // target-native downcast based on the handler's
                // declared return type (`ctx.current_return_type`)
                // so the read evaluates to a typed rvalue.
                //
                // Dynamic-typed targets (Python, JS, Ruby, Lua, PHP,
                // Dart, GDScript) need no cast.
                let rt = ctx.current_return_type.as_deref().unwrap_or("");
                context_return_read_typed(lang, rt, &ctx.system_name, &ctx.event_name)
            }
}

pub(super) fn expand_context_return_expr(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:(expr) - set context return value (concise form).
            // The scanner extends the segment span to consume any
            // trailing whitespace + `return` + `;` on the same source
            // line, so when `@@:(expr) return;` appears as a single
            // line in the source, this expansion emits BOTH the
            // assignment to the return slot AND the native return
            // statement on separate lines, properly indented.
            //
            // Detect whether the scanner consumed a trailing `return`
            // by looking for the bare `return` keyword in segment_text
            // outside of the `@@:(...)` expression.
            // Extract expression from metadata (preferred) or raw text (fallback)
            let trimmed = segment_text.trim();
            let (expr, has_native_return) = if let SegmentMetadata::ReturnExpr { expr } = metadata {
                // Check for trailing `return` in the segment text
                // (the metadata only has the expression, not the trailing keyword)
                let has_ret = if let Some(close_pos) = trimmed.rfind(')') {
                    let tail = trimmed[close_pos + 1..].trim();
                    tail.starts_with("return")
                        && (tail.len() == 6
                            || tail
                                .as_bytes()
                                .get(6)
                                .map_or(true, |b| b.is_ascii_whitespace() || *b == b';'))
                } else {
                    false
                };
                (expr.clone(), has_ret)
            } else {
                // Fallback: parse from raw text
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
                    let expr_str = trimmed[after_open..p].to_string();
                    let after_close = if p < bytes.len() { p + 1 } else { p };
                    let tail = trimmed[after_close..].trim();
                    let has_ret = tail.starts_with("return")
                        && (tail.len() == 6
                            || tail.as_bytes()[6].is_ascii_whitespace()
                            || tail.as_bytes()[6] == b';');
                    (expr_str, has_ret)
                } else {
                    (trimmed.to_string(), false)
                }
            };
            let expanded_expr = paren_wrap_if_multiline(&expand_expression(expr.trim(), lang, ctx));
            // Standalone @@ constructs include indent_str on all lines.
            // The scanner trims trailing whitespace from preceding native
            // text for standalone constructs (computed_indent > 0), so
            // indent_str reconstructs the correct indentation.
            let assignment = match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    format!(
                        "{}self._context_stack[-1]._return = {}",
                        indent_str, expanded_expr
                    )
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    format!(
                        "this._context_stack[this._context_stack.length - 1]._return = {};",
                        expanded_expr
                    )
                }
                TargetLanguage::C => {
                    c_return_assign(&ctx.system_name, &expanded_expr, &ctx.current_return_type)
                }
                TargetLanguage::Rust => super::super::rust_system::rust_expand_box_return_bare(
                    &indent_str,
                    &expanded_expr,
                    &ctx.current_return_type,
                    &ctx.system_name,
                    &ctx.event_name,
                ),
                TargetLanguage::Cpp => {
                    let wrapped = cpp_wrap_string_literal(&expanded_expr);
                    format!("_context_stack.back()._return = std::any({});", wrapped)
                }
                TargetLanguage::Java => format!(
                    "_context_stack.get(_context_stack.size() - 1)._return = {};",
                    expanded_expr
                ),
                TargetLanguage::Kotlin => format!(
                    "_context_stack[_context_stack.size - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Swift => format!(
                    "_context_stack[_context_stack.count - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::CSharp => format!(
                    "_context_stack[_context_stack.Count - 1]._return = {};",
                    expanded_expr
                ),
                TargetLanguage::Go => {
                    // String-literal-aware rewrite of `self.` → `s.` —
                    // same fix as the `@@:return = expr` Go branch above.
                    let go_expr = replace_outside_strings_and_comments(
                        &expanded_expr,
                        TargetLanguage::Go,
                        &[("self.", "s.")],
                    );
                    format!(
                        "s._context_stack[len(s._context_stack)-1]._return = {}",
                        go_expr
                    )
                }
                TargetLanguage::Php => format!(
                    "$this->_context_stack[count($this->_context_stack) - 1]->_return = {};",
                    expanded_expr
                ),
                TargetLanguage::Ruby => format!(
                    "@_context_stack[@_context_stack.length - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Lua => format!(
                    "self._context_stack[#self._context_stack]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Erlang => {
                    // Leave `self.` intact so the body processor binds
                    // it to the live data_gen — see the sibling fix in
                    // `FrameSegmentKind::ContextReturn` above.
                    format!("__ReturnVal = {}", expanded_expr)
                }
                TargetLanguage::Graphviz => unreachable!(),
            };
            if has_native_return {
                // Append a `return` statement on its own line at the
                // same indent as the assignment. The indent comes from
                // the segment's `indent` field, which the scanner sets
                // to the column position of the segment in the source.
                // The newline puts us at column 0, then indent_str
                // fills in the source's leading whitespace.
                let ret_line = match lang {
                    TargetLanguage::Python3
                    | TargetLanguage::GDScript
                    | TargetLanguage::Lua
                    | TargetLanguage::Ruby => format!("{}return", indent_str),
                    TargetLanguage::Erlang => String::new(), // Erlang has no native return statement
                    _ => format!("{}return;", indent_str),
                };
                if ret_line.is_empty() {
                    assignment
                } else {
                    format!("{}\n{}", assignment, ret_line)
                }
            } else {
                assignment
            }
}
