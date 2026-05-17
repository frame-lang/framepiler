//! `@@Foo()` system instantiation, `@@:Foo(args)` return-call,
//! and native `return` statement expansion.
//!
//! Three arms:
//!
//! - `expand_system_instantiation` (`@@Foo(args)`) — emits a
//!   factory call to `Foo::create(args)` (or per-target
//!   equivalent). The bare `@@!Foo()` no-init variant lives in
//!   `no_init::generate_no_initialization`.
//! - `expand_return_call` (`@@:Foo(args)`) — same factory but
//!   in tail position; emits the construction and a target-
//!   shaped `return` so the caller sees the new instance.
//! - `expand_return_statement` — native `return expr;` /
//!   `return` keyword passthrough with W415/E408 validation.
//!   Used when the user writes a native return inside a handler
//!   to opt into native control flow rather than the Frame
//!   `@@:return` mechanism.

use super::expand_expression;
use super::super::codegen_utils::{replace_outside_strings_and_comments, HandlerContext};
use super::no_init::generate_no_initialization;
use super::utility::{c_return_assign, cpp_wrap_string_literal, paren_wrap_if_multiline};
use crate::frame_c::compiler::native_region_scanner::{RegionSpan, SegmentMetadata};
use crate::frame_c::visitors::TargetLanguage;

pub(super) fn expand_system_instantiation(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // `@@SystemName(args)` (Factory): emitted VERBATIM here so the
            // assembler's `expand_system_instantiations` post-pass rewrites
            // to the per-language constructor call. (Phase 5 of RFC-0015 D7
            // will migrate Factory rewriting into this arm and remove the
            // post-pass; for now we keep the existing behavior.)
            //
            // `@@!SystemName()` (NoInitialization, RFC-0015 D7): emitted
            // here directly as the per-language no-initialization allocation. The
            // assembler's post-pass doesn't recognize `@@!` (the trailing
            // `!` after `@@` short-circuits its uppercase check), so we
            // MUST emit the final form here.
            use crate::frame_c::compiler::frame_ast::InstantiationKind;
            if let SegmentMetadata::SystemInstantiation {
                system_name,
                kind: InstantiationKind::NoInitialization,
                ..
            } = metadata
            {
                generate_no_initialization(system_name, lang)
            } else {
                segment_text.to_string()
            }
}

pub(super) fn expand_return_call(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:return(expr) — set context return value AND exit handler.
            // This is the "set + return" one-liner. The segment text is
            // `@@:return(expr)` — extract the expression between parens.
            let trimmed = segment_text.trim();
            let expr_owned;
            let expr = if let SegmentMetadata::ReturnCall { expr } = metadata {
                expr.as_str()
            } else {
                // Fallback: parse from raw text
                expr_owned = if let Some(start) = trimmed.find('(') {
                    let inner = &trimmed[start + 1..];
                    if let Some(end) = inner.rfind(')') {
                        inner[..end].trim().to_string()
                    } else {
                        inner.trim().to_string()
                    }
                } else {
                    String::new()
                };
                &expr_owned
            };
            let expanded_expr = paren_wrap_if_multiline(&expand_expression(expr, lang, ctx));

            // Standalone @@ constructs include indent_str on all lines.
            let set_code = match lang {
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

            // Append native return on a new line with proper indent
            let ret_code = match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript | TargetLanguage::Ruby => {
                    format!("\n{}return", indent_str)
                }
                TargetLanguage::Lua => format!("\n{}return", indent_str),
                TargetLanguage::Erlang => String::new(),
                _ => format!("\n{}return;", indent_str),
            };

            format!("{}{}", set_code, ret_code)
}

pub(super) fn expand_return_statement(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // Native return keyword detected in handler body.
            // Extract expression after "return" (if any).
            let after_return = segment_text
                .trim()
                .strip_prefix("return")
                .unwrap_or("")
                .trim()
                .trim_end_matches(';')
                .trim();

            if after_return.is_empty() {
                // Bare `return` — valid, exits the handler. Pass through as native.
                format!("{}return", indent_str)
            } else if after_return.starts_with("@@:") || after_return.starts_with("@@(") {
                // E408: `return @@:<anything>` — combining native return with Frame context
                eprintln!(
                    "E408: Cannot combine `return` with Frame context syntax `{}`. \
                    Use `@@:(expr)` to set the return value, then `return` on a separate line.",
                    after_return
                );
                String::new()
            } else {
                // W415: `return <expr>` in event handler — value is silently lost
                eprintln!(
                    "W415: `return {}` in event handler '{}' — the return value is lost. \
                    Use `@@:({})` to set the return value, or bare `return` to exit.",
                    after_return, ctx.event_name, after_return
                );
                // Pass through as native — it compiles but doesn't do what the user expects
                format!("{}{}", indent_str, segment_text.trim())
            }
}
