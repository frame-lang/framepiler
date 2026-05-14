//! `@@:self` / `@@:self.method(...)` self-reference / re-entrant
//! dispatch expansion.
//!
//! Two arms:
//!
//! - `expand_context_self` — bare `@@:self`. Emits the per-target
//!   self / instance pointer (`self` in most languages, `this` in
//!   the C-family / TS, `&mut self` for Rust, etc.).
//! - `expand_context_self_call` — `@@:self.method(args)`.
//!   Re-entrant call back into the running system's dispatch.
//!   The transition guard `if _transitioned then return` is
//!   emitted separately by `emit_handler_body_via_statements` so
//!   it lands at a statement boundary.

use super::expand_expression;
use super::super::codegen_utils::HandlerContext;
use super::utility::strip_outer_parens;
use crate::frame_c::compiler::native_region_scanner::{RegionSpan, SegmentMetadata};
use crate::frame_c::visitors::TargetLanguage;

pub(super) fn expand_context_self(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:self — bare system instance reference
            match lang {
                TargetLanguage::Python3
                | TargetLanguage::GDScript
                | TargetLanguage::Ruby
                | TargetLanguage::Lua
                | TargetLanguage::Swift => "self".to_string(),
                TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Java
                | TargetLanguage::Kotlin
                | TargetLanguage::CSharp
                | TargetLanguage::Dart => "this".to_string(),
                TargetLanguage::Cpp => "this".to_string(),
                TargetLanguage::C => "self".to_string(),
                TargetLanguage::Go => "s".to_string(),
                TargetLanguage::Php => "$this".to_string(),
                TargetLanguage::Rust => super::super::rust_system::rust_self_ref().to_string(),
                TargetLanguage::Erlang => "self".to_string(),
                TargetLanguage::Graphviz => unreachable!(),
            }
}

pub(super) fn expand_context_self_call(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:self.method(args) — reentrant interface call with transition guard
            // Extract method name and args from segment text: @@:self.method(args)
            let trimmed = segment_text.trim();
            let (method_name, raw_args_with_parens) =
                if let SegmentMetadata::SelfCall { method, args } = metadata {
                    (method.as_str(), args.as_str())
                } else {
                    let after_self = trimmed.strip_prefix("@@:self.").unwrap_or(trimmed);
                    let paren_pos = after_self.find('(').unwrap_or(after_self.len());
                    (&after_self[..paren_pos], &after_self[paren_pos..])
                };
            // Recursively expand Frame syntax nested inside the args —
            // e.g. `@@:self.foo(@@:return)`, `@@:self.foo(@@:params.x)`,
            // `@@:self.foo(self.op())`, etc. Without this the inner
            // segment would leak verbatim into target source and fail
            // to parse (e.g. literal `@@:return` in Python output).
            let expanded_args = if raw_args_with_parens.len() >= 2
                && raw_args_with_parens.starts_with('(')
                && raw_args_with_parens.ends_with(')')
            {
                let inner = strip_outer_parens(raw_args_with_parens);
                if inner.is_empty() {
                    raw_args_with_parens.to_string()
                } else {
                    format!("({})", expand_expression(inner, lang, ctx))
                }
            } else {
                raw_args_with_parens.to_string()
            };
            let args_with_parens = expanded_args.as_str();

            // Generate the native self-call
            let call_expr = match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    format!("self.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
                    format!("this.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Rust => {
                    // Rust's borrow checker rejects `self.foo(self.bar(x))`
                    // because both calls take `&mut self` at the same time.
                    // When the already-expanded args contain another
                    // `self.<method>(` pattern, hoist the inner call into
                    // a let-binding inside a block expression:
                    //   { let __rs_tmpN = self.bar(x); self.foo(__rs_tmpN) }
                    // Sequential `let` bindings in a block are two
                    // separate borrows — not simultaneous — so the
                    // checker accepts.
                    if args_with_parens.contains("self.") {
                        let inner = strip_outer_parens(args_with_parens);
                        format!(
                            "{{ let __rs_tmp_arg = {}; self.{}(__rs_tmp_arg) }}",
                            inner, method_name
                        )
                    } else {
                        format!("self.{}{}", method_name, args_with_parens)
                    }
                }
                TargetLanguage::Swift => {
                    format!("self.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Cpp => format!("this->{}{}", method_name, args_with_parens),
                TargetLanguage::C => {
                    if args_with_parens == "()" {
                        format!("{}_{}(self)", ctx.system_name, method_name)
                    } else {
                        let inner_args = strip_outer_parens(args_with_parens);
                        format!("{}_{}(self, {})", ctx.system_name, method_name, inner_args)
                    }
                }
                TargetLanguage::Java | TargetLanguage::Kotlin | TargetLanguage::CSharp => {
                    format!("this.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Go => {
                    let go_method =
                        format!("{}{}", method_name[..1].to_uppercase(), &method_name[1..]);
                    format!("s.{}{}", go_method, args_with_parens)
                }
                TargetLanguage::Php => format!("$this->{}{}", method_name, args_with_parens),
                TargetLanguage::Ruby => format!("self.{}{}", method_name, args_with_parens),
                TargetLanguage::Lua => format!("self:{}{}", method_name, args_with_parens),
                TargetLanguage::Erlang => {
                    // Emit bare `self.method(args)` and let the Erlang
                    // handler post-pass (erlang_system.rs::
                    // erlang_rewrite_native_classified_full) recognize the
                    // pattern as an `InterfaceCall` and rewrite it to
                    // `{DataN, Result} = frame_dispatch__(method, [args],
                    // DataPrev)`. That pass threads NewData forward
                    // through the rest of the handler body via
                    // `data_gen`/`data_var` — so `self.field` reads and
                    // `-> $State` transitions after a @@:self call
                    // correctly see the state changes the called
                    // handler made.
                    format!("self.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Graphviz => unreachable!(),
            };

            // @@:self.method() — check if standalone (only whitespace before @@:
            // in the source) or inline (preceded by native code like `x = `).
            // The scanner trims trailing whitespace from native text for
            // standalone constructs, so we must provide indent_str. For
            // inline, native text provides the indent.
            //
            // We detect this from the segment_text position: if the segment
            // starts at a position where the preceding byte is whitespace or
            // newline, it's standalone. The scanner always sets indent > 0
            // for self-calls (line's leading whitespace for the guard), so
            // we can't use indent == 0 as the inline signal.
            //
            // Instead, check if the raw output ends with whitespace (inline
            // context: native text like "baseline = " precedes us) or with
            // a newline (standalone: previous line ended, we start fresh).
            //
            // Actually, the simplest correct approach: the expansion is
            // always just the call expression. The orchestrator adds the
            // guard. For standalone, the scanner trimmed the whitespace so
            // indent_str fills the gap. For inline, the scanner kept the
            // native text. In BOTH cases, indent_str is correct:
            //   standalone: trimmed ws (16 sp) + indent_str (16 sp) call = 16 sp call ✓
            //   inline: native "baseline = " + indent_str (16 sp) call = broken!
            //
            // So we DO need to distinguish. Use the preceding native text:
            // if it was trimmed (standalone), the segment immediately follows
            // a newline in the output. If not trimmed (inline), it follows
            // non-newline content. But we don't have access to `out` here.
            //
            // Cleanest: just return call_expr. The standalone case needs
            // indent_str, which the orchestrator can add based on indent > 0
            // and whether the expansion doesn't already start with whitespace.
            call_expr
}
