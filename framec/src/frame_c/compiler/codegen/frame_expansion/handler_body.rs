//! Handler body splicing + per-target `@@:return` typed read.
//!
//! Three closely-coupled helpers, all called from the per-state
//! handler emission path:
//!
//! - `resolve_state_arg_key(i, target_state, ctx)` — resolves
//!   the storage key for a positional state-arg in a transition.
//!   Returns the declared param name from the target state's
//!   signature when available, otherwise the bare index. The
//!   Rust backend uses the declared name for typed-StateContext
//!   field assignment.
//! - `context_return_read_typed(lang, frame_type, system_name)` —
//!   emits the per-target downcast for `@@:return` reads. Every
//!   typed target stores the context-stack `_return` slot in
//!   an untyped slot (`Object`/`Any`/`void*`/`std::any`/…); a
//!   bare access loses the static type at the first use. This
//!   helper restores the type the user declared.
//! - `emit_handler_body_via_statements(span, source, lang, ctx)` —
//!   the AST-statement-driven splicer that scans the handler
//!   body's source bytes, classifies regions, walks the
//!   resulting Statement stream, and emits expanded native
//!   code interleaved with `generate_frame_expansion` output for
//!   every Frame segment. Handles the transition-vs-return-expr
//!   ordering, the standalone-self-call indent prefix, and the
//!   deferred self-call guard (D1 fix: fire at statement
//!   boundaries, not mid-expression).

use super::super::codegen_utils::{
    cpp_map_type, csharp_map_type, go_map_type, java_map_type, kotlin_map_type, swift_map_type,
    HandlerContext,
};
use super::utility::{normalize_indentation, split_transition_return, strip_java_unreachable};
use crate::frame_c::compiler::native_region_scanner::{FrameSegmentKind, Region};
use crate::frame_c::visitors::TargetLanguage;

/// Resolve the storage key for a positional state-arg in a transition.
/// Returns the declared param name — used by Rust backend for typed
/// StateContext struct field assignment.
pub(crate) fn resolve_state_arg_key(i: usize, target_state: &str, ctx: &HandlerContext) -> String {
    ctx.state_param_names
        .get(target_state)
        .and_then(|names| names.get(i))
        .cloned()
        .unwrap_or_else(|| i.to_string())
}

/// `@@:return` typed-read expansion across all 17 targets.
///
/// The per-call context stack's `_return` slot is untyped in every
/// typed target (`Object`/`Any`/`void*`/`std::any`/…). A bare read
/// fails the first time the result hits an arithmetic op, a typed
/// self-call arg, or a return-type-checked `return`. This helper
/// emits the target's native downcast to the handler's *declared*
/// return type, spelled the target way — type-ignorant, so it works
/// for any type the user wrote (see
/// docs/contributing/type-ignorant-codegen.md). Two targets keep a
/// per-category branch because the target language forces it: C
/// (`void*` ABI — `double` rides via the bit-pun, primitives via
/// `(intptr_t)`, pointers as-is) and Java (no `(int) Object` —
/// primitives must box-then-unbox).
///
/// Dynamic-typed targets (Python, JavaScript, Ruby, Lua, PHP, Dart,
/// GDScript) don't need a cast — they get the bare access.
pub(super) fn context_return_read_typed(
    lang: TargetLanguage,
    frame_type: &str,
    system_name: &str,
    event_name: &str,
) -> String {
    let _ = event_name; // unused on non-Rust targets
    match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => {
            "self._context_stack[-1]._return".to_string()
        }
        TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
            "this._context_stack[this._context_stack.length - 1]._return".to_string()
        }
        TargetLanguage::C => {
            // C's `_return` slot is a `void*` — a per-category cast back
            // is forced by the ABI: `double` rides via the memcpy
            // bit-pun, a string is already a pointer, everything else
            // fits in the integer width. (See type-ignorant-codegen.md
            // category 3.)
            let raw = format!("{}_RETURN(self)", system_name);
            match frame_type {
                "float" | "double" | "f32" | "f64" => {
                    format!("{}_unpack_double({})", system_name, raw)
                }
                "str" | "string" | "String" | "char*" | "const char*" => {
                    format!("((const char*){})", raw)
                }
                "int" | "bool" => format!("((int)(intptr_t){})", raw),
                _ => raw,
            }
        }
        TargetLanguage::Rust => {
            super::super::rust_system::rust_context_return_read_typed(
                frame_type,
                system_name,
                event_name,
            )
        }
        TargetLanguage::Cpp => {
            format!(
                "std::any_cast<{}>(_context_stack.back()._return)",
                cpp_map_type(frame_type)
            )
        }
        TargetLanguage::Java => {
            let raw = "_context_stack.get(_context_stack.size() - 1)._return";
            // The JVM forbids `(int) Object`; a primitive receiver has to
            // box-then-unbox. Reference types take a plain cast.
            let mapped = java_map_type(frame_type);
            let (boxed, prim): (&str, Option<&str>) = match mapped.as_str() {
                "int" => ("Integer", Some("intValue")),
                "long" => ("Long", Some("longValue")),
                "double" => ("Double", Some("doubleValue")),
                "float" => ("Float", Some("floatValue")),
                "boolean" => ("Boolean", Some("booleanValue")),
                "char" => ("Character", Some("charValue")),
                "byte" => ("Byte", Some("byteValue")),
                "short" => ("Short", Some("shortValue")),
                other => (other, None),
            };
            match prim {
                Some(m) => format!("(({}) {}).{}()", boxed, raw, m),
                None => format!("(({}) {})", boxed, raw),
            }
        }
        TargetLanguage::Kotlin => {
            let raw = "_context_stack[_context_stack.size - 1]._return";
            format!("({} as {})", raw, kotlin_map_type(frame_type))
        }
        TargetLanguage::Swift => {
            let raw = "_context_stack[_context_stack.count - 1]._return";
            format!("({} as! {})", raw, swift_map_type(frame_type))
        }
        TargetLanguage::CSharp => {
            let raw = "_context_stack[_context_stack.Count - 1]._return";
            format!("(({}) {})", csharp_map_type(frame_type), raw)
        }
        TargetLanguage::Go => {
            let raw = "s._context_stack[len(s._context_stack)-1]._return";
            format!("{}.({})", raw, go_map_type(frame_type))
        }
        TargetLanguage::Php => {
            "$this->_context_stack[count($this->_context_stack) - 1]->_return".to_string()
        }
        TargetLanguage::Ruby => "@_context_stack[@_context_stack.length - 1]._return".to_string(),
        TargetLanguage::Lua => "self._context_stack[#self._context_stack]._return".to_string(),
        TargetLanguage::Erlang => "__ReturnVal".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    }
}

/// Emit handler body by scanning for Frame segments and walking them as AST statements.
///
/// Pipeline: source bytes → scanner → regions → statements → expansion walk → output string.
/// NativeCode passes through verbatim; Frame constructs are expanded per-language.
pub(crate) fn emit_handler_body_via_statements(
    span: &crate::frame_c::compiler::ast::Span,
    source: &[u8],
    lang: TargetLanguage,
    ctx: &HandlerContext,
) -> String {
    use crate::frame_c::compiler::frame_ast::Statement;
    use crate::frame_c::compiler::native_region_scanner::regions_to_statements;

    if span.start >= source.len() || span.end > source.len() || span.start >= span.end {
        return String::new();
    }

    let body_bytes = &source[span.start..span.end];
    let open_brace = match body_bytes.iter().position(|&b| b == b'{') {
        Some(pos) => pos,
        None => return String::from_utf8_lossy(body_bytes).trim().to_string(),
    };

    // Scanner does the hard work
    let mut scanner = super::scanner_dispatch::get_native_scanner(lang);
    let scan_result = match scanner.scan(body_bytes, open_brace) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };

    // Convert regions to typed AST statements
    let statements = regions_to_statements(body_bytes, &scan_result.regions);

    // Walk statements — NativeCode passes through, Frame constructs get expanded.
    // We still call generate_frame_expansion() for Frame constructs by looking up
    // the original Region to get the span/kind/metadata/indent it needs.
    let mut out = String::new();
    let mut frame_idx = 0usize; // Index into FrameSegment regions
    let frame_regions: Vec<_> = scan_result
        .regions
        .iter()
        .filter(|r| matches!(r, Region::FrameSegment { .. }))
        .collect();

    // Track which statement indices to skip (consumed by lookahead)
    let mut skip_set = std::collections::HashSet::new();
    // Deferred self-call transition guard — emitted after the native
    // line containing the self-call completes (so `;` lands before guard).
    let mut pending_guard: Option<String> = None;

    for (stmt_idx, stmt) in statements.iter().enumerate() {
        if skip_set.contains(&stmt_idx) {
            // This statement was consumed by a prior lookahead — skip it
            // but still advance frame_idx if it's a Frame statement
            if !matches!(stmt, Statement::NativeCode(_)) {
                frame_idx += 1;
            }
            continue;
        }
        match stmt {
            Statement::NativeCode(text) => {
                if let Some(guard) = pending_guard.take() {
                    // Tight option 2 (D1 fix): the transition guard must
                    // fire AT A STATEMENT BOUNDARY, not mid-expression.
                    // The boundary is signaled by a newline in the next
                    // NativeCode segment — when present, we've reached
                    // end-of-line and the assignment is complete.
                    //
                    // If the next NativeCode lacks a newline, it's a
                    // continuation of the same statement (e.g. ` + 5`,
                    // `, lit`, ` && other`). Emitting the guard here
                    // would split the expression. Re-stash the guard
                    // and let it propagate to the NEXT NativeCode that
                    // DOES end the line. A subsequent self-call segment
                    // may overwrite the guard with its own — that's
                    // correct: `_transitioned` is monotonic, so a single
                    // statement-end check catches "any embedded call
                    // transitioned the system".
                    if let Some(nl_pos) = text.find('\n') {
                        out.push_str(&text[..=nl_pos]);
                        out.push_str(&guard);
                        out.push('\n');
                        out.push_str(&text[nl_pos + 1..]);
                    } else {
                        // No newline — keep guard pending; emit text only.
                        out.push_str(text);
                        pending_guard = Some(guard);
                    }
                } else {
                    out.push_str(text);
                }
            }
            _ => {
                // Look up the corresponding original Region for expansion parameters
                if frame_idx < frame_regions.len() {
                    if let Region::FrameSegment {
                        span: seg_span,
                        kind,
                        indent,
                        metadata,
                    } = frame_regions[frame_idx]
                    {
                        let expansion = super::generate_frame_expansion(
                            body_bytes, seg_span, *kind, *indent, lang, ctx, metadata,
                        );

                        // ── Transition control flow ──────────────────────
                        // Transition expansions end with `return` to exit the
                        // handler after the state change. But if a return-expr
                        // (`@@:(expr)`) follows the transition in the same
                        // scope, the return makes it unreachable.
                        //
                        // Fix: separate the expansion body from the trailing
                        // `return`. Emit body, consume any same-scope
                        // return-expr, then emit `return`. This is a clean
                        // separation of transition semantics (the expansion)
                        // from handler control flow (the orchestrator).
                        let is_transition = matches!(
                            kind,
                            FrameSegmentKind::Transition | FrameSegmentKind::StackPop
                        );
                        if is_transition {
                            let (body, return_kw) = split_transition_return(&expansion);
                            // Multi-line expansion on same line as native code
                            // needs a line break first
                            if !out.is_empty() && !out.ends_with('\n') && body.contains('\n') {
                                out.push('\n');
                            }
                            out.push_str(body);

                            if !return_kw.is_empty() {
                                // Scan forward for a return-expr that directly
                                // follows this transition in the same block.
                                //
                                // Two conditions must BOTH hold:
                                // 1. Only whitespace NativeCode between them
                                //    (content like `else:` or `}` = different block)
                                // 2. The return-expr has the same scanner-computed
                                //    indent as the transition (catches Python's
                                //    indent-based scoping where dedent is whitespace)
                                //
                                // Together these handle both brace languages
                                // (content stops the scan) and indent languages
                                // (indent mismatch stops the consume).
                                //
                                // For nested-control-flow transitions where
                                // the user expected an outer-scope
                                // `@@:(value)` to apply on the transition path,
                                // see W705 in `frame_validator.rs` — the spec
                                // is "code after a transition is unreachable",
                                // and the validator warns when the user's
                                // pattern would silently leak a default value
                                // (Issue #4 in FRAMEC_BUGS.md).
                                let next_frame_idx = frame_idx + 1;
                                for j in (stmt_idx + 1)..statements.len() {
                                    match &statements[j] {
                                        Statement::NativeCode(text) if text.trim().is_empty() => {
                                            continue
                                        }
                                        Statement::ContextReturnExpr { .. }
                                        | Statement::ReturnCall { .. } => {
                                            if next_frame_idx < frame_regions.len() {
                                                if let Region::FrameSegment {
                                                    span: ret_span,
                                                    kind: ret_kind,
                                                    indent: ret_indent,
                                                    metadata: ret_meta,
                                                } = frame_regions[next_frame_idx]
                                                {
                                                    if *ret_indent == *indent {
                                                        let ret_exp = super::generate_frame_expansion(
                                                            body_bytes,
                                                            ret_span,
                                                            *ret_kind,
                                                            *ret_indent,
                                                            lang,
                                                            ctx,
                                                            ret_meta,
                                                        );
                                                        out.push('\n');
                                                        out.push_str(&ret_exp);
                                                        skip_set.insert(j);
                                                    }
                                                }
                                            }
                                            break;
                                        }
                                        _ => break,
                                    }
                                }
                                // Emit return on its own line at the transition's indent
                                out.push('\n');
                                out.push_str(&" ".repeat(*indent));
                                out.push_str(return_kw);
                            }
                        } else {
                            // ── Non-transition expansion ─────────────────
                            if !out.is_empty() && !out.ends_with('\n') && expansion.contains('\n') {
                                out.push('\n');
                            }
                            // Self-call: bare call expression needs indent
                            // prefix (standalone) or statement terminator.
                            let is_standalone_self_call = *kind
                                == FrameSegmentKind::ContextSelfCall
                                && (out.is_empty() || out.ends_with('\n'));
                            if is_standalone_self_call {
                                out.push_str(&" ".repeat(*indent));
                            }
                            out.push_str(&expansion);
                            if is_standalone_self_call {
                                match lang {
                                    TargetLanguage::Python3
                                    | TargetLanguage::GDScript
                                    | TargetLanguage::Ruby
                                    | TargetLanguage::Lua => {}
                                    _ => out.push(';'),
                                }
                            }

                            // Self-call guard — deferred until native line ends
                            if *kind == FrameSegmentKind::ContextSelfCall {
                                let guard = super::generate_self_call_guard(
                                    *indent,
                                    lang,
                                    &ctx.system_name,
                                );
                                if !guard.is_empty() {
                                    pending_guard = Some(guard);
                                }
                            }
                        }
                    }
                    frame_idx += 1;
                }
            }
        }
    }

    // Flush any remaining deferred self-call guard
    if let Some(guard) = pending_guard.take() {
        out.push('\n');
        out.push_str(&guard);
    }

    // Same post-processing as splice path
    let text = out.trim_start_matches('\n').trim_end();
    let text = normalize_indentation(text);
    if matches!(
        lang,
        TargetLanguage::Java
            | TargetLanguage::Kotlin
            | TargetLanguage::Swift
            | TargetLanguage::CSharp
            | TargetLanguage::Go
    ) {
        let text = if matches!(
            lang,
            TargetLanguage::Swift | TargetLanguage::Kotlin | TargetLanguage::Go
        ) {
            text.lines()
                .map(|line| {
                    let trimmed = line.trim_end();
                    if trimmed.ends_with(';') {
                        let stripped = trimmed.trim_end_matches(';');
                        if stripped.is_empty() && line.trim() == ";" {
                            String::new()
                        } else {
                            stripped.to_string()
                        }
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text.replace(";;", ";")
        };
        strip_java_unreachable(&text)
    } else {
        text
    }
}
