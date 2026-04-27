//! Frame statement expansion and handler body splicing.
//!
//! This module handles the core Frame-to-native-code transformation:
//! - Splicing handler bodies: scanning for Frame statements in native code
//!   and replacing them with target-language expansions
//! - Frame statement expansion: converting -> $State, => $^, push$, pop$,
//!   return sugar, $.var, @@:return, etc. to target language code
//! - Helper functions for extracting transition targets, args, state vars

use super::codegen_utils::{
    cpp_map_type, cpp_wrap_any_arg, csharp_map_type, expression_to_string, go_map_type,
    java_map_type, kotlin_map_type, replace_outside_strings_and_comments, state_var_init_value,
    swift_map_type, to_snake_case, type_to_cpp_string, HandlerContext,
};
use crate::frame_c::compiler::frame_ast::Type;
use crate::frame_c::compiler::native_region_scanner::{
    c::NativeRegionScannerC, cpp::NativeRegionScannerCpp, csharp::NativeRegionScannerCs,
    dart::NativeRegionScannerDart, erlang::NativeRegionScannerErlang,
    gdscript::NativeRegionScannerGDScript, go::NativeRegionScannerGo,
    java::NativeRegionScannerJava, javascript::NativeRegionScannerJs,
    kotlin::NativeRegionScannerKotlin, lua::NativeRegionScannerLua, php::NativeRegionScannerPhp,
    python::NativeRegionScannerPy, ruby::NativeRegionScannerRuby, rust::NativeRegionScannerRust,
    swift::NativeRegionScannerSwift, typescript::NativeRegionScannerTs, FrameSegmentKind,
    NativeRegionScanner, Region, SegmentMetadata,
};
use crate::frame_c::compiler::splice::Splicer;
use crate::frame_c::visitors::TargetLanguage;

/// C `_return` assignment with double-aware marshalling.
///
/// The `_return` slot is `void*`. Ints/bools/pointers travel via
/// `(void*)(intptr_t)(val)` cleanly. Doubles don't — `(intptr_t)(42.0)`
/// truncates the fractional part. When the handler's declared return
/// type is `float`/`double`, pack via a memcpy helper the runtime emits
/// (`Sys_pack_double`).
fn c_return_assign(system_name: &str, expanded_expr: &str, return_type: &Option<String>) -> String {
    let is_dbl = return_type
        .as_deref()
        .map(|t| {
            let t = t.trim();
            t == "float" || t == "double"
        })
        .unwrap_or(false);
    if is_dbl {
        format!(
            "{sys}_CTX(self)->_return = {sys}_pack_double({expr});",
            sys = system_name,
            expr = expanded_expr,
        )
    } else {
        format!(
            "{}_CTX(self)->_return = (void*)(intptr_t)({});",
            system_name, expanded_expr
        )
    }
}

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

/// Strip the outer parentheses from `(inner)` → `inner`.
///
/// Preconditions (checked by the caller): `s` is non-empty and wrapped
/// in a matching outer `(…)` pair. The three @@:self.method expansion
/// sites use this to unwrap `raw_args_with_parens` before splicing the
/// arg list into a target's native call form (e.g. C's free-function
/// dispatch `Sys_method(self, <inner>)`).
fn strip_outer_parens(s: &str) -> &str {
    debug_assert!(
        s.len() >= 2 && s.starts_with('(') && s.ends_with(')'),
        "strip_outer_parens called on non-paren-wrapped input: {:?}",
        s
    );
    &s[1..s.len() - 1]
}

/// Wrap a C++ expression in std::string() if it's a string literal.
/// Prevents std::bad_any_cast when storing in std::any (const char* vs std::string).
fn cpp_wrap_string_literal(expr: &str) -> String {
    let trimmed = expr.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        format!("std::string({})", trimmed)
    } else {
        expr.to_string()
    }
}

/// `@@:return` typed-read expansion across all 17 targets.
///
/// The per-call context stack's `_return` slot is untyped in every
/// typed target (`Object`/`Any`/`void*`/`std::any`/…). A bare read
/// fails the first time the result hits an arithmetic op, a typed
/// self-call arg, or a return-type-checked `return`. This helper
/// emits the target's native downcast keyed on the handler's declared
/// Frame return type (`int`/`bool`/`str`/float-ish). Unknown types
/// fall back to the raw slot access, matching each target's natural
/// default (Java returns `Object`, C returns `void*`, etc.).
///
/// Dynamic-typed targets (Python, JavaScript, Ruby, Lua, PHP, Dart,
/// GDScript) don't need a cast — they get the bare access.
pub(crate) fn context_return_read_typed(
    lang: TargetLanguage,
    frame_type: &str,
    system_name: &str,
) -> String {
    match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => {
            "self._context_stack[-1]._return".to_string()
        }
        TargetLanguage::TypeScript
        | TargetLanguage::Dart
        | TargetLanguage::JavaScript => {
            "this._context_stack[this._context_stack.length - 1]._return".to_string()
        }
        TargetLanguage::C => {
            let raw = format!("{}_RETURN(self)", system_name);
            match frame_type {
                "int" | "bool" => format!("((int)(intptr_t){})", raw),
                "str" => format!("((const char*){})", raw),
                _ => raw,
            }
        }
        TargetLanguage::Rust => super::rust_system::rust_context_return_read_typed(frame_type),
        TargetLanguage::Cpp => {
            let cpp_type = match frame_type {
                "int" => "int",
                "bool" => "bool",
                "str" => "std::string",
                _ => "std::string", // legacy default
            };
            format!(
                "std::any_cast<{}>(_context_stack.back()._return)",
                cpp_type
            )
        }
        TargetLanguage::Java => {
            let raw = "_context_stack.get(_context_stack.size() - 1)._return";
            match frame_type {
                "int" => format!("((Integer) {}).intValue()", raw),
                "bool" => format!("((Boolean) {}).booleanValue()", raw),
                "str" => format!("((String) {})", raw),
                _ => raw.to_string(),
            }
        }
        TargetLanguage::Kotlin => {
            let raw = "_context_stack[_context_stack.size - 1]._return";
            match frame_type {
                "int" => format!("({} as Int)", raw),
                "bool" => format!("({} as Boolean)", raw),
                "str" => format!("({} as String)", raw),
                _ => raw.to_string(),
            }
        }
        TargetLanguage::Swift => {
            let raw = "_context_stack[_context_stack.count - 1]._return";
            match frame_type {
                "int" => format!("({} as! Int)", raw),
                "bool" => format!("({} as! Bool)", raw),
                "str" => format!("({} as! String)", raw),
                _ => raw.to_string(),
            }
        }
        TargetLanguage::CSharp => {
            let raw = "_context_stack[_context_stack.Count - 1]._return";
            match frame_type {
                "int" => format!("((int) {})", raw),
                "bool" => format!("((bool) {})", raw),
                "str" => format!("((string) {})", raw),
                _ => raw.to_string(),
            }
        }
        TargetLanguage::Go => {
            let raw = "s._context_stack[len(s._context_stack)-1]._return";
            match frame_type {
                "int" => format!("{}.(int)", raw),
                "bool" => format!("{}.(bool)", raw),
                "str" => format!("{}.(string)", raw),
                _ => raw.to_string(),
            }
        }
        TargetLanguage::Php => {
            "$this->_context_stack[count($this->_context_stack) - 1]->_return".to_string()
        }
        TargetLanguage::Ruby => {
            "@_context_stack[@_context_stack.length - 1]._return".to_string()
        }
        TargetLanguage::Lua => {
            "self._context_stack[#self._context_stack]._return".to_string()
        }
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
    let mut scanner = get_native_scanner(lang);
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
                    // Insert the guard after the first line break in this
                    // native text (the break that ends the self-call line).
                    if let Some(nl_pos) = text.find('\n') {
                        out.push_str(&text[..=nl_pos]);
                        out.push_str(&guard);
                        out.push('\n'); // terminate the guard line
                        out.push_str(&text[nl_pos + 1..]);
                    } else {
                        // No newline — emit native text, then guard
                        out.push_str(text);
                        out.push('\n');
                        out.push_str(&guard);
                        out.push('\n');
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
                        let expansion = generate_frame_expansion(
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
                                let next_frame_idx = frame_idx + 1;
                                for j in (stmt_idx + 1)..statements.len() {
                                    match &statements[j] {
                                        Statement::NativeCode(text)
                                            if text.trim().is_empty() =>
                                        {
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
                                                        let ret_exp = generate_frame_expansion(
                                                            body_bytes, ret_span, *ret_kind,
                                                            *ret_indent, lang, ctx, ret_meta,
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
                            let is_standalone_self_call =
                                *kind == FrameSegmentKind::ContextSelfCall
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
                                let guard =
                                    generate_self_call_guard(*indent, lang, &ctx.system_name);
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

/// Strip unreachable code after terminal statements for Java.
/// Java treats code after `return;` as a compile error, unlike TypeScript/C++ which ignore it.
pub(crate) fn strip_java_unreachable(text: &str) -> String {
    let mut result = Vec::new();
    let mut skip = false;
    for line in text.lines() {
        if skip {
            // Stop skipping when we hit a closing brace or another control structure
            let trimmed = line.trim();
            if trimmed.starts_with('}') || trimmed.is_empty() {
                skip = false;
                // Don't include the empty lines that were between return and next code
                if !trimmed.is_empty() {
                    result.push(line.to_string());
                }
            }
            // Skip all other lines (unreachable code)
            continue;
        }
        result.push(line.to_string());
        // Check if this line ends with a terminal return
        let trimmed = line.trim();
        if trimmed == "return;" || trimmed == "return" {
            skip = true;
        }
    }
    result.join("\n")
}

/// Normalize indentation by removing common leading whitespace from all lines
pub(crate) fn normalize_indentation(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    // Find minimum indentation (ignoring empty lines)
    let min_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    // Strip the common indentation from all lines
    lines
        .iter()
        .map(|line| {
            if line.len() >= min_indent {
                &line[min_indent..]
            } else {
                line.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate code expansion for a Frame segment
///
/// NOTE: The scanner leaves a gap between NativeText and FrameSegment where leading
/// whitespace lives. Since the splicer doesn't copy this gap, we MUST include the
/// indentation in the expansion to preserve proper code structure.
pub(crate) fn generate_frame_expansion(
    body_bytes: &[u8],
    span: &crate::frame_c::compiler::native_region_scanner::RegionSpan,
    kind: FrameSegmentKind,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

    match kind {
        FrameSegmentKind::Transition => {
            // Parse transition: (exit_args)? -> (enter_args)? $State(state_args)?
            // For Python/TypeScript: Create compartment and call __transition()
            // For Rust: Use simpler _transition() approach

            // Check for forward-transition: -> => $State
            let is_forward = if let SegmentMetadata::Transition { is_forward, .. } = metadata {
                *is_forward
            } else {
                false
            };

            // Check for pop-transition: -> pop$
            let is_pop = if let SegmentMetadata::Transition { is_pop, .. } = metadata {
                *is_pop
            } else {
                segment_text.contains("pop$")
            };
            if is_pop {
                // Pop-transition with optional decorations (RFC-0008):
                // 1. Write exit_args to current compartment (if present)
                // 2. Pop from stack
                // 3. If enter_args present: clear + write fresh values
                // 4. If is_forward: set forward_event
                // 5. __transition + return
                let (exit_str, enter_str) = match metadata {
                    SegmentMetadata::Transition {
                        exit_args,
                        enter_args,
                        ..
                    } => (exit_args.clone(), enter_args.clone()),
                    _ => (None, None),
                };
                generate_pop_transition(&indent_str, ctx, lang, &exit_str, &enter_str, is_forward)
            } else if is_forward {
                // Forward-transition: -> => $State
                // Create compartment, set forward_event to current event,
                // call __transition, return.
                let target = match metadata {
                    SegmentMetadata::Transition { target_state, .. } => target_state.clone(),
                    _ => "Unknown".to_string(),
                };
                // Build the target state's HSM ancestry outer-in. Used
                // below by the per-handler targets (Python/TS/JS/GDScript/
                // Ruby/Lua) to construct the parent_compartment chain
                // eagerly, never duplicating the transition-source
                // compartment (see
                // _scratch/bug_parent_compartment_hsm_walk.md).
                let mut ancestors: Vec<String> = Vec::new();
                let mut cursor = target.clone();
                while let Some(parent) = ctx.state_hsm_parents.get(&cursor) {
                    ancestors.push(parent.clone());
                    cursor = parent.clone();
                }
                ancestors.reverse();

                match lang {
                    TargetLanguage::Python3 => {
                        // Forward transition: same chain construction as
                        // a regular transition (via __prepareEnter), plus
                        // forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}__compartment = self.__prepareEnter(\"{}\", [], [])\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}self.__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::GDScript => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}var __compartment = self.__prepareEnter(\"{}\", [], [])\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}self.__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}const __compartment = this.__prepareEnter(\"{}\", [], []);\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e;\n",
                            indent_str
                        ));
                        code.push_str(&format!(
                            "{}this.__transition(__compartment);\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Dart => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}final __compartment = this.__prepareEnter(\"{}\", [], []);\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e;\n",
                            indent_str
                        ));
                        code.push_str(&format!(
                            "{}this.__transition(__compartment);\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Rust => super::rust_system::rust_expand_forward_transition(
                        &indent_str,
                        ctx,
                        &target,
                    ),
                    TargetLanguage::C => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}{}_Compartment* __compartment = {}_prepareEnter(self, \"{}\", NULL, NULL);\n",
                            indent_str, ctx.system_name, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment->forward_event = __e;\n",
                            indent_str
                        ));
                        code.push_str(&format!(
                            "{}{}_transition(self, __compartment);\n",
                            indent_str, ctx.system_name
                        ));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Cpp => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}auto __compartment = __prepareEnter(\"{}\", std::vector<std::any>{{}}, std::vector<std::any>{{}});\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment->forward_event = std::make_unique<{}FrameEvent>(__e);\n",
                            indent_str, ctx.system_name
                        ));
                        code.push_str(&format!(
                            "{}__transition(std::move(__compartment));\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Java => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}{}Compartment __compartment = __prepareEnter(\"{}\", new ArrayList<>(), new ArrayList<>());\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e;\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}__transition(__compartment);\n", indent_str));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Kotlin => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}val __compartment = __prepareEnter(\"{}\", mutableListOf<Any?>(), mutableListOf<Any?>())\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::Swift => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}let __compartment = {}.__prepareEnter(\"{}\", [], [])\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::Php => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}$__compartment = $this->__prepareEnter(\"{}\", [], []);\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}$__compartment->forward_event = $__e;\n",
                            indent_str
                        ));
                        code.push_str(&format!(
                            "{}$this->__transition($__compartment);\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::CSharp => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf. Local
                        // is `__next` (not `__compartment`) — see C# regular
                        // transition for why. Wrapped in `{ ... }` block
                        // for the same reason.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}{{ {}Compartment __next = __prepareEnter(\"{}\", new List<object>(), new List<object>());\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__next.forward_event = __e;\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}__transition(__next); }}\n", indent_str));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Go => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}__compartment := s.__prepareEnter(\"{}\", []any{{}}, []any{{}})\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forwardEvent = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}s.__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::Ruby => {
                        // Forward transition: same chain via __prepareEnter,
                        // plus forward_event field set on the leaf.
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}__compartment = __prepareEnter(\"{}\", [], [])\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::Lua => {
                        // Forward transition. nil for empty args lists
                        // (block-transformer workaround — see regular-
                        // transition Lua case).
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}local __compartment = self:__prepareEnter(\"{}\", nil, nil)\n",
                            indent_str, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}self:__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::Erlang => {
                        // Forward transition: cascade exit/enter (same
                        // shape as `frame_transition__`) plus a
                        // `next_event` action that re-dispatches the
                        // originating event (`__Event`) to the new
                        // leaf after gen_statem fires its `state_enter`
                        // callback there. `__Event` is bound by the
                        // handler clause's pattern (see
                        // erlang_system.rs handler emission). Forward
                        // transitions can't carry their own
                        // exit/enter/state args at the Frame level
                        // (the syntax is just `-> => $State`), so all
                        // three arg maps are empty.
                        let erlang_state = to_snake_case(&target);
                        format!(
                            "{}frame_forward_transition__({}, __Event, Data, [], [], [], From)",
                            indent_str, erlang_state
                        )
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            } else {
                // Normal transition: -> $State with exit/enter/state args
                // Transition metadata is always populated by the scanner.
                let (target, exit_args, enter_args, state_args) = match metadata {
                    SegmentMetadata::Transition {
                        target_state,
                        exit_args,
                        enter_args,
                        state_args,
                        ..
                    } => (
                        target_state.clone(),
                        exit_args.clone(),
                        enter_args.clone(),
                        state_args.clone(),
                    ),
                    _ => unreachable!(
                        "Transition kind segment without Transition metadata: {:?}",
                        metadata
                    ),
                };

                // Expand state variable references in arguments
                let exit_str = exit_args.map(|a| expand_expression(&a, lang, ctx));
                let enter_str = enter_args.map(|a| expand_expression(&a, lang, ctx));
                let state_str = state_args.map(|a| expand_expression(&a, lang, ctx));

                // Get compartment class name from system name
                let _compartment_class = format!("{}Compartment", ctx.system_name);

                match lang {
                    TargetLanguage::Python3 => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        //   __prepareExit(exit_args) — populates
                        //     exit_args on every layer of the source chain.
                        //   __prepareEnter(leaf, state_args, enter_args) —
                        //     constructs the destination chain via the
                        //     static _HSM_CHAIN topology table; every
                        //     layer gets independent copies of the args
                        //     (uniform parameter propagation).
                        //   __transition(comp) — caches destination for
                        //     the kernel to process.
                        let mut code = String::new();

                        // Build state_args list literal.
                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };

                        // Build enter_args list literal.
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };

                        // Populate exit_args on the source chain (omitted
                        // when there are no exit_args).
                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}self.__prepareExit([{}])\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        // Construct destination chain via the helper.
                        code.push_str(&format!(
                            "{}__compartment = self.__prepareEnter(\"{}\", {}, {})\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        // Cache and return.
                        code.push_str(&format!(
                            "{}self.__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::GDScript => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}self.__prepareExit([{}])\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}var __compartment = self.__prepareEnter(\"{}\", {}, {})\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}self.__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                        // Per-handler architecture with helpers (see
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareExit / __prepareEnter / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}this.__prepareExit([{}]);\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}const __compartment = this.__prepareEnter(\"{}\", {}, {});\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}this.__transition(__compartment);\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Dart => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}this.__prepareExit([{}]);\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}final __compartment = this.__prepareEnter(\"{}\", {}, {});\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}this.__transition(__compartment);\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Rust => super::rust_system::rust_expand_transition(
                        &indent_str,
                        ctx,
                        &target,
                        &exit_str,
                        &state_str,
                        &enter_str,
                    ),
                    TargetLanguage::C => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();
                        let sys = &ctx.system_name;

                        // exit_args via __prepareExit if any provided.
                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}{{ {}_FrameVec* __ea = {}_FrameVec_new();\n",
                                    indent_str, sys, sys
                                ));
                                for v in &vals {
                                    code.push_str(&format!(
                                        "{}{}_FrameVec_push(__ea, (void*)(intptr_t)({}));\n",
                                        indent_str, sys, v
                                    ));
                                }
                                code.push_str(&format!(
                                    "{}{}_prepareExit(self, __ea);\n",
                                    indent_str, sys
                                ));
                                code.push_str(&format!(
                                    "{}{}_FrameVec_destroy(__ea); }}\n",
                                    indent_str, sys
                                ));
                            }
                        }

                        // Build state_args / enter_args FrameVecs, call __prepareEnter.
                        let state_vals: Vec<String> = if let Some(ref state) = state_str {
                            state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim().to_string()
                                    } else {
                                        arg.to_string()
                                    }
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                        let enter_vals: Vec<String> = if let Some(ref enter) = enter_str {
                            enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|s| s.to_string())
                                .collect()
                        } else {
                            Vec::new()
                        };

                        // Open block scope so locals don't collide with
                        // sibling transitions in the same handler (e.g.
                        // separate `if` branches).
                        code.push_str(&format!("{}{{\n", indent_str));
                        if state_vals.is_empty() {
                            code.push_str(&format!(
                                "{}    {}_FrameVec* __sa = NULL;\n",
                                indent_str, sys
                            ));
                        } else {
                            code.push_str(&format!(
                                "{}    {}_FrameVec* __sa = {}_FrameVec_new();\n",
                                indent_str, sys, sys
                            ));
                            for v in &state_vals {
                                code.push_str(&format!(
                                    "{}    {}_FrameVec_push(__sa, (void*)(intptr_t)({}));\n",
                                    indent_str, sys, v
                                ));
                            }
                        }
                        if enter_vals.is_empty() {
                            code.push_str(&format!(
                                "{}    {}_FrameVec* __ea = NULL;\n",
                                indent_str, sys
                            ));
                        } else {
                            code.push_str(&format!(
                                "{}    {}_FrameVec* __ea = {}_FrameVec_new();\n",
                                indent_str, sys, sys
                            ));
                            for v in &enter_vals {
                                code.push_str(&format!(
                                    "{}    {}_FrameVec_push(__ea, (void*)(intptr_t)({}));\n",
                                    indent_str, sys, v
                                ));
                            }
                        }
                        code.push_str(&format!(
                            "{}    {}_Compartment* __compartment = {}_prepareEnter(self, \"{}\", __sa, __ea);\n",
                            indent_str, sys, sys, target
                        ));
                        if !state_vals.is_empty() {
                            code.push_str(&format!(
                                "{}    {}_FrameVec_destroy(__sa);\n",
                                indent_str, sys
                            ));
                        }
                        if !enter_vals.is_empty() {
                            code.push_str(&format!(
                                "{}    {}_FrameVec_destroy(__ea);\n",
                                indent_str, sys
                            ));
                        }
                        code.push_str(&format!(
                            "{}    {}_transition(self, __compartment);\n",
                            indent_str, sys
                        ));
                        code.push_str(&format!("{}}}\n", indent_str));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Cpp => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<String> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    let raw = if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    };
                                    format!("std::any({})", cpp_wrap_any_arg(raw))
                                })
                                .collect();
                            format!("std::vector<std::any>{{{}}}", vals.join(", "))
                        } else {
                            "std::vector<std::any>{}".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<String> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|a| format!("std::any({})", cpp_wrap_any_arg(a)))
                                .collect();
                            format!("std::vector<std::any>{{{}}}", vals.join(", "))
                        } else {
                            "std::vector<std::any>{}".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<String> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|a| format!("std::any({})", cpp_wrap_any_arg(a)))
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}__prepareExit(std::vector<std::any>{{{}}});\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}auto __next = __prepareEnter(\"{}\", {}, {});\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}__transition(std::move(__next));\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Java => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            if vals.is_empty() {
                                "new ArrayList<>()".to_string()
                            } else {
                                format!(
                                    "new ArrayList<>(java.util.Arrays.asList({}))",
                                    vals.join(", ")
                                )
                            }
                        } else {
                            "new ArrayList<>()".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if vals.is_empty() {
                                "new ArrayList<>()".to_string()
                            } else {
                                format!(
                                    "new ArrayList<>(java.util.Arrays.asList({}))",
                                    vals.join(", ")
                                )
                            }
                        } else {
                            "new ArrayList<>()".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}__prepareExit(new ArrayList<>(java.util.Arrays.asList({})));\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}{}Compartment __compartment = __prepareEnter(\"{}\", {}, {});\n",
                            indent_str, ctx.system_name, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}__transition(__compartment);\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Kotlin => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            if vals.is_empty() {
                                "mutableListOf<Any?>()".to_string()
                            } else {
                                format!("mutableListOf<Any?>({})", vals.join(", "))
                            }
                        } else {
                            "mutableListOf<Any?>()".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if vals.is_empty() {
                                "mutableListOf<Any?>()".to_string()
                            } else {
                                format!("mutableListOf<Any?>({})", vals.join(", "))
                            }
                        } else {
                            "mutableListOf<Any?>()".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}__prepareExit(mutableListOf<Any?>({}))\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}val __compartment = __prepareEnter(\"{}\", {}, {})\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Swift => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}__prepareExit([{}])\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}let __compartment = {}.__prepareEnter(\"{}\", {}, {})\n",
                            indent_str, ctx.system_name, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::CSharp => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        // Note: local var is named `__next` (not
                        // `__compartment`) to avoid shadowing the field
                        // in stack-push handlers that reference the field
                        // earlier in the same block — C# rejects that
                        // even when the local is declared later.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            if vals.is_empty() {
                                "new List<object>()".to_string()
                            } else {
                                format!("new List<object> {{ {} }}", vals.join(", "))
                            }
                        } else {
                            "new List<object>()".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if vals.is_empty() {
                                "new List<object>()".to_string()
                            } else {
                                format!("new List<object> {{ {} }}", vals.join(", "))
                            }
                        } else {
                            "new List<object>()".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}__prepareExit(new List<object> {{ {} }});\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        // Wrap in `{ ... }` block scope so multiple
                        // transitions in the same handler (e.g. inside
                        // separate `if` branches) don't trigger C#
                        // CS0136 (same name used in enclosing scope).
                        code.push_str(&format!(
                            "{}{{ {}Compartment __next = __prepareEnter(\"{}\", {}, {});\n",
                            indent_str, ctx.system_name, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}__transition(__next); }}\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Go => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[]any{{{}}}", vals.join(", "))
                        } else {
                            "[]any{}".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            format!("[]any{{{}}}", vals.join(", "))
                        } else {
                            "[]any{}".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}s.__prepareExit([]any{{{}}})\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}__compartment := s.__prepareEnter(\"{}\", {}, {})\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}s.__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Php => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();
                        let current_params = ctx
                            .event_param_names
                            .get(&ctx.event_name)
                            .cloned()
                            .unwrap_or_default();
                        let php_fix =
                            |expr: &str| php_prefix_params(expr, &current_params);

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<String> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    let raw = if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    };
                                    php_fix(raw)
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<String> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    let raw = if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    };
                                    php_fix(raw)
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<String> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|a| php_fix(a))
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}$this->__prepareExit([{}]);\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}$__compartment = $this->__prepareEnter(\"{}\", {}, {});\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}$this->__transition($__compartment);\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Ruby => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+):
                        // __prepareEnter / __prepareExit / __transition.
                        let mut code = String::new();

                        let state_args_list = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };
                        let enter_args_list = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        } else {
                            "[]".to_string()
                        };

                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}__prepareExit([{}])\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}__compartment = __prepareEnter(\"{}\", {}, {})\n",
                            indent_str, target, state_args_list, enter_args_list
                        ));

                        code.push_str(&format!(
                            "{}__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Lua => {
                        // Per-handler architecture with helpers (per
                        // docs/frame_runtime_introduction.md Step 21+).
                        // Uses table.pack(...) instead of `{}` literals
                        // because the Lua block transformer mishandles
                        // `{}` table literals inside if/else bodies
                        // (sees them as nested block braces). nil is
                        // accepted by __prepareEnter / __prepareExit
                        // when there are no args.
                        let mut code = String::new();

                        // state_args
                        let state_arg = if let Some(ref state) = state_str {
                            let vals: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            if vals.is_empty() {
                                "nil".to_string()
                            } else {
                                format!("table.pack({})", vals.join(", "))
                            }
                        } else {
                            "nil".to_string()
                        };

                        // enter_args
                        let enter_arg = if let Some(ref enter) = enter_str {
                            let vals: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            if vals.is_empty() {
                                "nil".to_string()
                            } else {
                                format!("table.pack({})", vals.join(", "))
                            }
                        } else {
                            "nil".to_string()
                        };

                        // exit_args (only emitted when present)
                        if let Some(ref exit) = exit_str {
                            let vals: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !vals.is_empty() {
                                code.push_str(&format!(
                                    "{}self:__prepareExit(table.pack({}))\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}local __compartment = self:__prepareEnter(\"{}\", {}, {})\n",
                            indent_str, target, state_arg, enter_arg
                        ));

                        code.push_str(&format!(
                            "{}self:__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Erlang => {
                        // Path D: use frame_transition__ for full lifecycle.
                        //
                        // Args are emitted as positional Erlang lists (per the
                        // HashMap→Vec migration). For `name=value` entries
                        // the name is dropped and only `value` is included —
                        // the codegen relies on Frame source authors providing
                        // args in declaration order, same convention as
                        // Python/TS/Rust/etc.
                        let erlang_state = to_snake_case(&target);
                        let mut code = String::new();

                        let to_list_lit = |s: &Option<String>| -> String {
                            match s {
                                Some(joined) => {
                                    let vals: Vec<&str> = joined
                                        .split(',')
                                        .map(|x| x.trim())
                                        .filter(|x| !x.is_empty())
                                        .map(|arg| {
                                            if let Some(eq_pos) = arg.find('=') {
                                                arg[eq_pos + 1..].trim()
                                            } else {
                                                arg
                                            }
                                        })
                                        .collect();
                                    format!("[{}]", vals.join(", "))
                                }
                                None => "[]".to_string(),
                            }
                        };
                        let exit_list = to_list_lit(&exit_str);
                        let enter_list = to_list_lit(&enter_str);
                        let state_list = to_list_lit(&state_str);

                        code.push_str(&format!(
                            "{}frame_transition__({}, Data, {}, {}, {}, From)",
                            indent_str, erlang_state, exit_list, enter_list, state_list
                        ));
                        code
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            }
        }
        FrameSegmentKind::Forward => {
            // HSM forward: call parent state's handler for the same event
            if let Some(ref parent) = ctx.parent_state {
                match lang {
                    // Python/TypeScript: call _state_Parent(__e) to dispatch via unified state method
                    TargetLanguage::Python3 | TargetLanguage::GDScript => {
                        if ctx.per_handler {
                            // Per-handler architecture: shift compartment up one
                            // level at the forward site so the parent dispatcher
                            // sees its own compartment as the param.
                            format!("{}self._state_{}(__e, compartment.parent_compartment)", indent_str, parent)
                        } else {
                            format!("{}self._state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::TypeScript
                    | TargetLanguage::Dart
                    | TargetLanguage::JavaScript => {
                        if ctx.per_handler {
                            // Dart and TypeScript are null-safe; assert
                            // non-null with `!`. JavaScript ignores the
                            // postfix annotation in runtime semantics —
                            // keep it bare.
                            let bang = if matches!(
                                lang,
                                TargetLanguage::Dart | TargetLanguage::TypeScript
                            ) {
                                "!"
                            } else {
                                ""
                            };
                            format!(
                                "{}this._state_{}(__e, compartment.parent_compartment{});",
                                indent_str, parent, bang
                            )
                        } else {
                            format!("{}this._state_{}(__e);", indent_str, parent)
                        }
                    }
                    // Rust: call parent state router (not specific handler) to dispatch via match
                    TargetLanguage::Rust => {
                        super::rust_system::rust_parent_forward(&indent_str, parent)
                    }
                    // C: call System_state_Parent(self, __e, parent_compartment)
                    // — per-handler architecture shifts the compartment up
                    // one level at the forward site.
                    TargetLanguage::C => {
                        if ctx.per_handler {
                            format!(
                                "{}{}_state_{}(self, __e, compartment->parent_compartment);",
                                indent_str, ctx.system_name, parent
                            )
                        } else {
                            format!(
                                "{}{}_state_{}(self, __e);",
                                indent_str, ctx.system_name, parent
                            )
                        }
                    }
                    // C++: call _state_Parent(__e, compartment->parent_compartment)
                    // — forward is not terminal, no return.
                    TargetLanguage::Cpp => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment->parent_compartment);",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e);", indent_str, parent)
                        }
                    }
                    // Java: per-handler architecture passes compartment as
                    // second arg; shift up one level at the forward site.
                    TargetLanguage::Java => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment);",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e);", indent_str, parent)
                        }
                    }
                    TargetLanguage::CSharp => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment);",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e);", indent_str, parent)
                        }
                    }
                    TargetLanguage::Kotlin => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment!!)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::Swift => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment!)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e)", indent_str, parent)
                        }
                    }
                    // Go: call s._state_Parent(__e) — forward is not terminal, no return
                    TargetLanguage::Go => {
                        if ctx.per_handler {
                            format!(
                                "{}s._state_{}(__e, compartment.parentCompartment)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}s._state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::Php => {
                        if ctx.per_handler {
                            format!(
                                "{}$this->_state_{}($__e, $compartment->parent_compartment);",
                                indent_str, parent
                            )
                        } else {
                            format!("{}$this->_state_{}($__e);", indent_str, parent)
                        }
                    }
                    TargetLanguage::Ruby => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::Lua => {
                        if ctx.per_handler {
                            format!(
                                "{}self:_state_{}(__e, compartment.parent_compartment)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}self:_state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::Erlang => {
                        // gen_statem: delegate to parent by calling parent state function directly.
                        // The parent handler returns a gen_statem response tuple.
                        // We must ensure the reply action includes From so the caller gets a response.
                        let parent_atom = to_snake_case(parent);
                        let event_atom = to_snake_case(&ctx.event_name);
                        // Call parent directly with the same {call, From} context so
                        // the parent's reply reaches the original caller.
                        format!(
                            "{}{}({{call, From}}, {}, Data)",
                            indent_str, parent_atom, event_atom
                        )
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            } else {
                // No parent state - just return (shouldn't happen in valid HSM)
                match lang {
                    TargetLanguage::Python3 | TargetLanguage::GDScript => {
                        format!("{}return  # Forward to parent (no parent)", indent_str)
                    }
                    TargetLanguage::Ruby
                    | TargetLanguage::Kotlin
                    | TargetLanguage::Swift
                    | TargetLanguage::Lua => {
                        format!("{}return // Forward to parent (no parent)", indent_str)
                    }
                    TargetLanguage::TypeScript
                    | TargetLanguage::JavaScript
                    | TargetLanguage::Rust
                    | TargetLanguage::Dart
                    | TargetLanguage::C
                    | TargetLanguage::Cpp
                    | TargetLanguage::Java
                    | TargetLanguage::CSharp
                    | TargetLanguage::Go
                    | TargetLanguage::Php => {
                        format!("{}return; // Forward to parent (no parent)", indent_str)
                    }
                    TargetLanguage::Erlang => {
                        format!("{}{{keep_state, Data}}", indent_str)
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            }
        }
        FrameSegmentKind::StackPush => {
            let target = match metadata {
                SegmentMetadata::StackPush {
                    transition_target: Some(t),
                } => t.clone(),
                _ => String::new(),
            };

            // push$ saves a REFERENCE to the current compartment on the
            // state stack — not a copy. In GC languages this is a direct
            // assignment. In C it's a pointer save (ownership transfers to
            // stack on push-with-transition). In C++ it's a shared_ptr
            // copy (ref count increment). In Rust, clone is required for
            // bare push$ (ownership model) but push-with-transition uses
            // mem::replace (ownership transfer). pop$ restores the saved
            // reference as the current compartment.
            match lang {
                TargetLanguage::Python3 => {
                    let push_code =
                        format!("{}self._state_stack.append(self.__compartment)", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}self._transition(\"{}\", None, None)",
                            push_code, indent_str, target
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::GDScript => {
                    let push_code =
                        format!("{}self._state_stack.append(self.__compartment)", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}self._transition(\"{}\", null, null)",
                            push_code, indent_str, target
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    let push_code =
                        format!("{}this._state_stack.push(this.__compartment);", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}this._transition(\"{}\", null, null);",
                            push_code, indent_str, target
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Dart => {
                    let push_code =
                        format!("{}this._state_stack.add(this.__compartment);", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}this.__transition({}Compartment(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Rust => {
                    if !target.is_empty() {
                        super::rust_system::rust_push_transition(&indent_str, ctx, &target)
                    } else {
                        super::rust_system::rust_bare_push(&indent_str)
                    }
                }
                TargetLanguage::C => {
                    // C: save reference via ref count increment. The stack
                    // holds a ref'd pointer. The kernel's _unref on
                    // transition won't free it while the stack holds a ref.
                    let push_code = format!("{}{}_FrameVec_push(self->_state_stack, {}_Compartment_ref(self->__compartment));",
                        indent_str, ctx.system_name, ctx.system_name);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}{}_transition(self, {}_Compartment_new(\"{}\"));",
                            push_code, indent_str, ctx.system_name, ctx.system_name, target
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Cpp => {
                    // C++: shared_ptr reference save (ref count increment).
                    let push_code = format!("{}_state_stack.push_back(__compartment);", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}__transition(std::make_shared<{}Compartment>(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Java => {
                    let push_code = format!("{}_state_stack.add(__compartment);", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}__transition(new {}Compartment(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Kotlin => {
                    let push_code = format!("{}_state_stack.add(__compartment)", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}__transition({}Compartment(\"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Swift => {
                    let push_code = format!("{}_state_stack.append(__compartment)", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}__transition({}Compartment(state: \"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Go => {
                    let push_code = format!(
                        "{}s._state_stack = append(s._state_stack, s.__compartment)",
                        indent_str
                    );
                    if !target.is_empty() {
                        format!(
                            "{}\n{}s.__transition(new{}Compartment(\"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::CSharp => {
                    let push_code = format!("{}_state_stack.Add(__compartment);", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}__transition(new {}Compartment(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Php => {
                    let push_code = format!(
                        "{}$this->_state_stack[] = $this->__compartment;",
                        indent_str
                    );
                    if !target.is_empty() {
                        format!(
                            "{}\n{}$this->__transition(new {}Compartment(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Ruby => {
                    let push_code = format!("{}@_state_stack.push(@__compartment)", indent_str);
                    if !target.is_empty() {
                        format!(
                            "{}\n{}__transition({}Compartment.new(\"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Lua => {
                    let push_code = format!(
                        "{}self._state_stack[#self._state_stack + 1] = self.__compartment",
                        indent_str
                    );
                    if !target.is_empty() {
                        format!(
                            "{}\n{}self:__transition({}Compartment.new(\"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str
                        )
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Erlang => {
                    let state_atom = to_snake_case(&ctx.state_name);
                    if !target.is_empty() {
                        let target_atom = to_snake_case(&target);
                        format!("{}self.frame_stack = [{} | self.frame_stack]\n{}{{next_state, {}, Data, [{{reply, From, ok}}]}}",
                            indent_str, state_atom, indent_str, target_atom)
                    } else {
                        format!(
                            "{}self.frame_stack = [{} | self.frame_stack]",
                            indent_str, state_atom
                        )
                    }
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::StackPop => {
            // Standalone pop$ — pop the top of the stack and discard it.
            // No transition. For transitioning to the popped state, use -> pop$.
            match lang {
                TargetLanguage::Python3 => format!("{}self._state_stack.pop()", indent_str),
                TargetLanguage::GDScript => format!("{}self._state_stack.pop_back()", indent_str),
                TargetLanguage::TypeScript => format!("{}this._state_stack.pop();", indent_str),
                TargetLanguage::JavaScript => format!("{}this._state_stack.pop();", indent_str),
                TargetLanguage::Dart => format!("{}this._state_stack.removeLast();", indent_str),
                TargetLanguage::Rust => super::rust_system::rust_bare_pop(&indent_str),
                TargetLanguage::C => format!(
                    "{}{}_FrameVec_pop(self->_state_stack);",
                    indent_str, ctx.system_name
                ),
                TargetLanguage::Cpp => format!("{}_state_stack.pop_back();", indent_str),
                TargetLanguage::Java => format!(
                    "{}_state_stack.remove(_state_stack.size() - 1);",
                    indent_str
                ),
                TargetLanguage::Kotlin => {
                    format!("{}_state_stack.removeAt(_state_stack.size - 1)", indent_str)
                }
                TargetLanguage::Swift => format!("{}_state_stack.removeLast()", indent_str),
                TargetLanguage::CSharp => format!(
                    "{}_state_stack.RemoveAt(_state_stack.Count - 1);",
                    indent_str
                ),
                TargetLanguage::Go => format!(
                    "{}s._state_stack = s._state_stack[:len(s._state_stack)-1]",
                    indent_str
                ),
                TargetLanguage::Php => format!("{}array_pop($this->_state_stack);", indent_str),
                TargetLanguage::Ruby => format!("{}@_state_stack.pop", indent_str),
                TargetLanguage::Lua => format!("{}table.remove(self._state_stack)", indent_str),
                TargetLanguage::Erlang => {
                    format!(
                        "{}[_ | __RestStack] = self.frame_stack,\n{}self.frame_stack = __RestStack",
                        indent_str, indent_str
                    )
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::StateVar => {
            // Extract variable name and optional interpolation quote context
            let (var_name, interp_quote) =
                if let SegmentMetadata::StateVar { name, interp_quote } = metadata {
                    (name.clone(), *interp_quote)
                } else {
                    (extract_state_var_name(&segment_text), None)
                };
            // When inside a string interpolation, use the opposite quote
            // for dict keys to avoid collisions (e.g., f"...{d['key']}...")
            let q = match interp_quote {
                Some(b'\'') => "\"",
                Some(b'"') => "'",
                _ => "\"", // default: double quotes (standalone code or backtick)
            };
            // State variables are stored in compartment.state_vars
            // For HSM: use __sv_comp if available (navigates to correct compartment for parent states)
            match lang {
                TargetLanguage::Python3 => {
                    if ctx.per_handler {
                        // New architecture: handler method takes `compartment`
                        // as a parameter already pointing at this state's own
                        // compartment (HSM forwards pre-shift it).
                        format!("compartment.state_vars[{}{}{}]", q, var_name, q)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[{}{}{}]", q, var_name, q)
                    } else {
                        format!("self.__compartment.state_vars[{}{}{}]", q, var_name, q)
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    if ctx.per_handler {
                        // Per-handler architecture: compartment is a param.
                        format!("compartment.state_vars[{}{}{}]", q, var_name, q)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[{}{}{}]", q, var_name, q)
                    } else {
                        format!("this.__compartment.state_vars[{}{}{}]", q, var_name, q)
                    }
                }
                TargetLanguage::Php => {
                    if ctx.per_handler {
                        format!("$compartment->state_vars[{}{}{}]", q, var_name, q)
                    } else if ctx.use_sv_comp {
                        format!("$__sv_comp->state_vars[{}{}{}]", q, var_name, q)
                    } else {
                        format!("$this->__compartment->state_vars[{}{}{}]", q, var_name, q)
                    }
                }
                TargetLanguage::Ruby => {
                    if ctx.per_handler {
                        format!("compartment.state_vars[{}{}{}]", q, var_name, q)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[{}{}{}]", q, var_name, q)
                    } else {
                        format!("@__compartment.state_vars[{}{}{}]", q, var_name, q)
                    }
                }
                TargetLanguage::Rust => {
                    super::rust_system::rust_expand_state_var_read(ctx, &var_name)
                }
                TargetLanguage::C => {
                    // For C, access via FrameDict_get with type-aware cast.
                    // Per-handler takes precedence — read from the handler's
                    // named `compartment` param. When use_sv_comp is set in
                    // the legacy path, read from `__sv_comp` pointer (walked
                    // up to owning state at dispatch entry); otherwise
                    // default to `self->__compartment`.
                    let c_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|s| s.as_str())
                        .unwrap_or("int");
                    let cast = match c_type {
                        "char*" | "const char*" | "str" | "string" | "String" => "(const char*)",
                        _ => "(int)(intptr_t)",
                    };
                    let comp = if ctx.per_handler {
                        "compartment"
                    } else if ctx.use_sv_comp {
                        "__sv_comp"
                    } else {
                        "self->__compartment"
                    };
                    format!(
                        "{}{}_FrameDict_get({}->state_vars, \"{}\")",
                        cast, ctx.system_name, comp, var_name
                    )
                }
                TargetLanguage::Cpp => {
                    let cpp_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|t| cpp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    if ctx.per_handler {
                        format!(
                            "std::any_cast<{}>(compartment->state_vars[{}{}{}])",
                            cpp_type, q, var_name, q
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "std::any_cast<{}>(__sv_comp->state_vars[{}{}{}])",
                            cpp_type, q, var_name, q
                        )
                    } else {
                        format!(
                            "std::any_cast<{}>(__compartment->state_vars[{}{}{}])",
                            cpp_type, q, var_name, q
                        )
                    }
                }
                TargetLanguage::Java => {
                    let java_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|t| java_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let accessor = if ctx.per_handler {
                        format!("compartment.state_vars.get(\"{}\")", var_name)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars.get(\"{}\")", var_name)
                    } else {
                        format!("__compartment.state_vars.get(\"{}\")", var_name)
                    };
                    if java_type == "Object" {
                        accessor
                    } else {
                        format!("(({}) {})", java_type, accessor)
                    }
                }
                TargetLanguage::Kotlin => {
                    let kt_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|t| kotlin_map_type(t))
                        .unwrap_or_else(|| "Int".to_string());
                    let cast = if kt_type == "Any?" {
                        String::new()
                    } else {
                        format!(" as {}", kt_type)
                    };
                    if ctx.per_handler {
                        format!("compartment.state_vars[{}{}{}]{}", q, var_name, q, cast)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[{}{}{}]{}", q, var_name, q, cast)
                    } else {
                        format!("__compartment.state_vars[{}{}{}]{}", q, var_name, q, cast)
                    }
                }
                TargetLanguage::Swift => {
                    let sw_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|t| swift_map_type(t))
                        .unwrap_or_else(|| "Int".to_string());
                    // Wrap in parens: `(expr as! Int)` prevents ambiguity
                    // when followed by `<=`, `>=`, etc. (Swift parses
                    // `as! Int <= 0` as `as! Int<...` generic syntax).
                    if sw_type == "Any" {
                        if ctx.per_handler {
                            format!("compartment.state_vars[{}{}{}]", q, var_name, q)
                        } else if ctx.use_sv_comp {
                            format!("__sv_comp.state_vars[{}{}{}]", q, var_name, q)
                        } else {
                            format!("__compartment.state_vars[{}{}{}]", q, var_name, q)
                        }
                    } else if ctx.per_handler {
                        format!(
                            "(compartment.state_vars[{}{}{}] as! {})",
                            q, var_name, q, sw_type
                        )
                    } else if ctx.use_sv_comp {
                        format!("(__sv_comp.state_vars[{}{}{}] as! {})", q, var_name, q, sw_type)
                    } else {
                        format!(
                            "(__compartment.state_vars[{}{}{}] as! {})",
                            q, var_name, q, sw_type
                        )
                    }
                }
                TargetLanguage::Go => {
                    let go_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|t| go_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let assertion = if go_type == "any" || go_type.is_empty() {
                        String::new()
                    } else {
                        format!(".({})", go_type)
                    };
                    if ctx.per_handler {
                        format!("compartment.stateVars[{}{}{}]{}", q, var_name, q, assertion)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.stateVars[{}{}{}]{}", q, var_name, q, assertion)
                    } else {
                        format!("s.__compartment.stateVars[{}{}{}]{}", q, var_name, q, assertion)
                    }
                }
                TargetLanguage::CSharp => {
                    let cs_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|t| csharp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let cast = if cs_type == "object" {
                        String::new()
                    } else {
                        format!("({}) ", cs_type)
                    };
                    if ctx.per_handler {
                        format!("{}compartment.state_vars[{}{}{}]", cast, q, var_name, q)
                    } else if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[{}{}{}]", cast, q, var_name, q)
                    } else {
                        format!("{}__compartment.state_vars[{}{}{}]", cast, q, var_name, q)
                    }
                }
                TargetLanguage::Lua => {
                    if ctx.per_handler {
                        format!("compartment.state_vars[{}{}{}]", q, var_name, q)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[{}{}{}]", q, var_name, q)
                    } else {
                        format!("self.__compartment.state_vars[{}{}{}]", q, var_name, q)
                    }
                }
                TargetLanguage::Dart => {
                    if ctx.per_handler {
                        format!("compartment.state_vars[{}{}{}]", q, var_name, q)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[{}{}{}]", q, var_name, q)
                    } else {
                        format!("this.__compartment.state_vars[{}{}{}]", q, var_name, q)
                    }
                }
                TargetLanguage::GDScript => {
                    if ctx.per_handler {
                        format!("compartment.state_vars[{}{}{}]", q, var_name, q)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[{}{}{}]", q, var_name, q)
                    } else {
                        format!("self.__compartment.state_vars[{}{}{}]", q, var_name, q)
                    }
                }
                TargetLanguage::Erlang => {
                    // State vars stored as sv_<state>_<var> in the Data
                    // record. Emit via the `self.` placeholder so the
                    // body processor's classifier substitutes it with
                    // the live `DataN#data.` prefix — matching the live
                    // data_gen in the handler. Hardcoding `Data#data.`
                    // here would read the pre-handler snapshot and miss
                    // any updates (`$.v = ...` lines earlier in the
                    // same body).
                    let state_prefix = to_snake_case(&ctx.state_name);
                    format!("self.sv_{}_{}", state_prefix, var_name)
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::StateVarAssign => {
            // State variable assignment: $.varName = expr
            // For C, this needs to become FrameDict_set(...)
            // Parse: $.varName = expr;
            let text = segment_text.trim();
            // Extract variable name: skip "$." and collect identifier
            let var_name_owned;
            let var_name = if let SegmentMetadata::StateVar { name, .. } = metadata {
                var_name_owned = name.clone();
                var_name_owned.as_str()
            } else if text.starts_with("$.") {
                let rest = &text[2..];
                let end = rest
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                &rest[..end]
            } else {
                ""
            };
            // Extract expression: everything after '='
            let expr = if let Some(eq_pos) = text.find('=') {
                let after_eq = &text[eq_pos + 1..];
                // Trim trailing semicolon if present
                after_eq.trim().trim_end_matches(';').trim()
            } else {
                ""
            };
            // Expand state vars in the expression
            let expanded_expr = expand_expression(expr, lang, ctx);

            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    if ctx.per_handler {
                        // Per-handler architecture: compartment is a param.
                        format!(
                            "{}compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}self.__compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment.state_vars[\"{}\"] = {};",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.state_vars[\"{}\"] = {};",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}this.__compartment.state_vars[\"{}\"] = {};",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Php => {
                    // For PHP, combine interface-event params, enter-handler
                    // params, and exit-handler params since any of them may
                    // be referenced by name on the RHS of a state var write
                    // (e.g. `$.signum = sig` inside `$>(sig: str)`).
                    let mut current_params: Vec<String> = ctx
                        .event_param_names
                        .get(&ctx.event_name)
                        .cloned()
                        .unwrap_or_default();
                    if let Some(ep) = ctx.state_enter_param_names.get(&ctx.state_name) {
                        for p in ep {
                            if !current_params.contains(p) {
                                current_params.push(p.clone());
                            }
                        }
                    }
                    if let Some(ep) = ctx.state_exit_param_names.get(&ctx.state_name) {
                        for p in ep {
                            if !current_params.contains(p) {
                                current_params.push(p.clone());
                            }
                        }
                    }
                    let rhs = php_prefix_params(&expanded_expr, &current_params);
                    if ctx.per_handler {
                        format!(
                            "{}$compartment->state_vars[\"{}\"] = {};",
                            indent_str, var_name, rhs
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}$__sv_comp->state_vars[\"{}\"] = {};",
                            indent_str, var_name, rhs
                        )
                    } else {
                        format!(
                            "{}$this->__compartment->state_vars[\"{}\"] = {};",
                            indent_str, var_name, rhs
                        )
                    }
                }
                TargetLanguage::Ruby => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}@__compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Rust => super::rust_system::rust_expand_state_var_write(
                    &indent_str,
                    ctx,
                    &var_name,
                    &expanded_expr,
                ),
                TargetLanguage::C => {
                    if ctx.per_handler {
                        format!("{}{}_FrameDict_set(compartment->state_vars, \"{}\", (void*)(intptr_t)({}));",
                            indent_str, ctx.system_name, var_name, expanded_expr)
                    } else if ctx.use_sv_comp {
                        format!("{}{}_FrameDict_set(__sv_comp->state_vars, \"{}\", (void*)(intptr_t)({}));",
                            indent_str, ctx.system_name, var_name, expanded_expr)
                    } else {
                        format!("{}{}_FrameDict_set(self->__compartment->state_vars, \"{}\", (void*)(intptr_t)({}));",
                            indent_str, ctx.system_name, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Cpp => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment->state_vars[\"{}\"] = std::any({});",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp->state_vars[\"{}\"] = std::any({});",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}__compartment->state_vars[\"{}\"] = std::any({});",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Java => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment.state_vars.put(\"{}\", {});",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.state_vars.put(\"{}\", {});",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}__compartment.state_vars.put(\"{}\", {});",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Kotlin => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}__compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Swift => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}__compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Go => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment.stateVars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.stateVars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}s.__compartment.stateVars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::CSharp => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment.state_vars[\"{}\"] = {};",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.state_vars[\"{}\"] = {};",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}__compartment.state_vars[\"{}\"] = {};",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Lua => {
                    if ctx.per_handler {
                        format!(
                            "{}compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else if ctx.use_sv_comp {
                        format!(
                            "{}__sv_comp.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}self.__compartment.state_vars[\"{}\"] = {}",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Erlang => {
                    // State var assignment: $.var = expr → record update.
                    // Leave `self.` intact so the body processor's
                    // classifier gets first crack at `self.<iface>(…)`
                    // patterns in the RHS (they should become
                    // `frame_dispatch__(…)` binds, not dot-accesses on
                    // the current Data record). Any bare domain
                    // `self.<field>` reads fall through to the Plain
                    // path's substitution with the live `data_var`.
                    let state_prefix = to_snake_case(&ctx.state_name);
                    let field_name = format!("sv_{}_{}", state_prefix, var_name);
                    format!("{}self.{} = {}", indent_str, field_name, expanded_expr)
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextReturn => {
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
                let expanded_expr = expand_expression(expr, lang, ctx);
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
                    TargetLanguage::Rust => super::rust_system::rust_expand_box_return(
                        &indent_str,
                        &expanded_expr,
                        &ctx.current_return_type,
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
                context_return_read_typed(lang, rt, &ctx.system_name)
            }
        }
        FrameSegmentKind::ContextReturnExpr => {
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
            let expanded_expr = expand_expression(expr.trim(), lang, ctx);
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
                TargetLanguage::C => c_return_assign(
                    &ctx.system_name,
                    &expanded_expr,
                    &ctx.current_return_type,
                ),
                TargetLanguage::Rust => super::rust_system::rust_expand_box_return_bare(
                    &indent_str,
                    &expanded_expr,
                    &ctx.current_return_type,
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
        FrameSegmentKind::ContextEvent => {
            // @@:event - interface event name
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    "self._context_stack[-1].event._message".to_string()
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    "this._context_stack[this._context_stack.length - 1].event._message".to_string()
                }
                TargetLanguage::C => format!("{}_CTX(self)->event->_message", ctx.system_name),
                // Rust: handlers receive __e as parameter, use it directly to avoid borrow conflicts
                TargetLanguage::Rust => super::rust_system::rust_event_message(),
                TargetLanguage::Cpp => "_context_stack.back()._event._message".to_string(),
                TargetLanguage::Java => {
                    "_context_stack.get(_context_stack.size() - 1)._event._message".to_string()
                }
                TargetLanguage::Kotlin => {
                    "_context_stack[_context_stack.size - 1]._event._message".to_string()
                }
                TargetLanguage::Swift => {
                    "_context_stack[_context_stack.count - 1]._event._message".to_string()
                }
                TargetLanguage::CSharp => {
                    "_context_stack[_context_stack.Count - 1]._event._message".to_string()
                }
                TargetLanguage::Go => {
                    "s._context_stack[len(s._context_stack)-1]._event._message".to_string()
                }
                TargetLanguage::Php => {
                    "$this->_context_stack[count($this->_context_stack) - 1]->_event->_message"
                        .to_string()
                }
                TargetLanguage::Ruby => {
                    "@_context_stack[@_context_stack.length - 1]._event._message".to_string()
                }
                TargetLanguage::Lua => {
                    "self._context_stack[#self._context_stack]._event._message".to_string()
                }
                TargetLanguage::Erlang => {
                    let event_atom = to_snake_case(&ctx.event_name);
                    format!("{}", event_atom)
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextData => {
            // @@:data.key - call-scoped data (read)
            let key = if let SegmentMetadata::ContextData { key, .. } = metadata {
                key.clone()
            } else {
                extract_dot_key(&segment_text, "@@:data") // fallback
            };
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    format!("self._context_stack[-1]._data[\"{}\"]", key)
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    format!(
                        "this._context_stack[this._context_stack.length - 1]._data[\"{}\"]",
                        key
                    )
                }
                TargetLanguage::C => format!("{}_DATA(self, \"{}\")", ctx.system_name, key),
                TargetLanguage::Rust => super::rust_system::rust_context_data_get(&key),
                TargetLanguage::Cpp => format!("_context_stack.back()._data[\"{}\"]", key),
                TargetLanguage::Java => format!(
                    "_context_stack.get(_context_stack.size() - 1)._data.get(\"{}\")",
                    key
                ),
                TargetLanguage::Kotlin => {
                    format!("_context_stack[_context_stack.size - 1]._data[\"{}\"]", key)
                }
                TargetLanguage::Swift => format!(
                    "_context_stack[_context_stack.count - 1]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::CSharp => format!(
                    "_context_stack[_context_stack.Count - 1]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::Go => format!(
                    "s._context_stack[len(s._context_stack)-1]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::Php => format!(
                    "$this->_context_stack[count($this->_context_stack) - 1]->_data[\"{}\"]",
                    key
                ),
                TargetLanguage::Ruby => format!(
                    "@_context_stack[@_context_stack.length - 1]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::Lua => format!(
                    "self._context_stack[#self._context_stack]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::Erlang => "undefined".to_string(), // gen_statem has no context data
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextDataAssign => {
            // @@:data[key] = expr - call-scoped data (assignment)
            // Extract key and value from "@@:data.key = expr;"
            let key = if let SegmentMetadata::ContextData { key, .. } = metadata {
                key.clone()
            } else {
                extract_dot_key(&segment_text, "@@:data") // fallback
            };
            // Find the = and extract the expression
            let trimmed = segment_text.trim();
            let eq_pos = trimmed.find('=').unwrap_or(trimmed.len());
            let expr = trimmed[eq_pos + 1..].trim().trim_end_matches(';').trim();
            let expanded_expr = expand_expression(expr, lang, ctx);
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => format!("{}self._context_stack[-1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => format!("{}this._context_stack[this._context_stack.length - 1]._data[\"{}\"] = {};", indent_str, key, expanded_expr),
                TargetLanguage::C => format!("{}{}_DATA_SET(self, \"{}\", {});", indent_str, ctx.system_name, key, expanded_expr),
                TargetLanguage::Rust => super::rust_system::rust_expand_context_data_write(
                    &indent_str, &key, &expanded_expr,
                ),
                TargetLanguage::Cpp => format!("{}_context_stack.back()._data[\"{}\"] = {};", indent_str, key, expanded_expr),
                TargetLanguage::Java => format!("{}_context_stack.get(_context_stack.size() - 1)._data.put(\"{}\", {});", indent_str, key, expanded_expr),
                TargetLanguage::Kotlin => format!("{}_context_stack[_context_stack.size - 1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::Swift => format!("{}_context_stack[_context_stack.count - 1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::CSharp => format!("{}_context_stack[_context_stack.Count - 1]._data[\"{}\"] = {};", indent_str, key, expanded_expr),
                TargetLanguage::Go => format!("{}s._context_stack[len(s._context_stack)-1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::Php => format!("{}$this->_context_stack[count($this->_context_stack) - 1]->_data[\"{}\"] = {};", indent_str, key, expanded_expr),
                TargetLanguage::Ruby => format!("{}@_context_stack[@_context_stack.length - 1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::Lua => format!("{}self._context_stack[#self._context_stack]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::Erlang => format!("{}ok", indent_str), // gen_statem has no context data
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextParams => {
            // @@:params.<key> — handler-interface parameter access.
            //
            // Every target's state-dispatch prologue binds the declared
            // params as TYPED locals at the top of the handler body
            // (e.g., `x := __e._parameters[0].(int)` for Go,
            // `let x = __e.parameters[0] as! Int` for Swift, etc.).
            // Emitting `@@:params.x` as the raw `_parameters[idx]`
            // access (what this branch used to do) dropped the type
            // info and failed in typed targets as soon as the value
            // hit an arithmetic operator or a typed-method arg —
            // especially when nested inside another Frame construct
            // like `@@:self.typed_method(@@:params.x)`.
            //
            // The correct translation is the declared param name
            // itself: it is the already-typed local the prologue
            // bound. Dynamic targets (Python, JS, Ruby, …) see the
            // same name — just an ordinary local variable.
            //
            // Erlang's handler dispatch binds params with the
            // capitalized variant (`X` = the param `x`), matching
            // Erlang's variable-identifier rule.
            let key = if let SegmentMetadata::ContextParams { key } = metadata {
                key.clone()
            } else {
                extract_dot_key(&segment_text, "@@:params") // fallback
            };
            match lang {
                TargetLanguage::Erlang => {
                    // Erlang bindings use the capitalized form (framec's
                    // dispatch prologue rebinds `x` as `X = maps:get(...)`).
                    let mut chars = key.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                }
                // PHP identifies locals with a `$` prefix. The handler
                // prologue binds the param as `$x = $__e->_parameters[0]`.
                TargetLanguage::Php => format!("${}", key),
                TargetLanguage::Graphviz => unreachable!(),
                _ => key,
            }
        }
        FrameSegmentKind::TaggedInstantiation => {
            // `@@SystemName(args)` is emitted VERBATIM here so the
            // assembler's `expand_tagged_instantiations` pass handles
            // it uniformly alongside top-level native segments. The
            // assembler:
            //   - Validates the system name against defined systems
            //     (replacing the error-gen this arm used to do).
            //   - Calls `parse_call_args` / `resolve_call`, which
            //     expand the `$(arg)` sigil form into positional
            //     values (handler-body codegen used to strip `@@` and
            //     wrap per-language but skipped that expansion,
            //     producing `Worker($(name))` — a Python syntax error
            //     — when a handler body created a parameterized child
            //     system).
            //   - Applies the per-language constructor wrap
            //     (`new Foo(…)`, `Foo::new(…)`, `NewFoo(…)`, etc.)
            //     via `generate_constructor`.
            // Consolidating all of that in the assembler removes the
            // duplicate per-language logic this arm carried and fixes
            // the sigil-not-expanded bug.
            segment_text.to_string()
        }
        FrameSegmentKind::ReturnCall => {
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
            let expanded_expr = expand_expression(expr, lang, ctx);

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
                TargetLanguage::C => c_return_assign(
                    &ctx.system_name,
                    &expanded_expr,
                    &ctx.current_return_type,
                ),
                TargetLanguage::Rust => super::rust_system::rust_expand_box_return_bare(
                    &indent_str,
                    &expanded_expr,
                    &ctx.current_return_type,
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
        FrameSegmentKind::ContextSystemBare => {
            // Bare @@:system — should have been caught by validator (E604)
            "/* ERROR: bare @@:system */".to_string()
        }
        FrameSegmentKind::ContextSystemState => expand_system_state(lang),
        FrameSegmentKind::ContextSelf => {
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
                TargetLanguage::Rust => super::rust_system::rust_self_ref().to_string(),
                TargetLanguage::Erlang => "self".to_string(),
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextSelfCall => {
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
        FrameSegmentKind::ReturnStatement => {
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
    }
}

/// PHP helper: prefix bare identifiers matching the current handler's declared
/// params with `$`. PHP variables are sigil-required — `sig` must be `$sig`.
///
/// The Frame parser captures expression text (enter args, state-var RHS, etc.)
/// verbatim, so `-> (sig) $State` and `$.signum = sig` leak `sig` into the
/// generated PHP where it's interpreted as an undefined constant. This walks
/// the expression outside of string literals and rewrites bare-word occurrences
/// of known handler params to `$param`.
///
/// Safe under:
///   - string literals (skipped)
///   - already-prefixed `$foo` (not doubled)
///   - method/property access `->foo`, `::foo` (not prefixed — `foo` is a
///     member name, not a variable)
///   - member calls on `$this` (already has `$`)
pub(crate) fn php_prefix_params(expr: &str, params: &[String]) -> String {
    if params.is_empty() {
        return expr.to_string();
    }
    // Delegate string-literal and comment skipping to the PHP skipper
    // rather than re-implementing quote tracking inline. Only the
    // identifier-walking + param-matching logic below is PHP-specific
    // to this transformation.
    let skipper = crate::frame_c::compiler::native_region_scanner::create_skipper(
        crate::frame_c::visitors::TargetLanguage::Php,
    );
    let bytes = expr.as_bytes();
    let end = bytes.len();
    let mut out = String::with_capacity(expr.len() + 4);
    let mut i = 0;
    while i < end {
        if let Some(next) = skipper.skip_string(bytes, i, end) {
            out.push_str(&expr[i..next]);
            i = next;
            continue;
        }
        if let Some(next) = skipper.skip_comment(bytes, i, end) {
            out.push_str(&expr[i..next]);
            i = next;
            continue;
        }
        let c = bytes[i];
        // Identifier start: lowercase alpha or underscore, not already
        // preceded by `$`, `->`, `::`, or an ident char (i.e. part of a
        // larger token).
        let is_ident_start = (c.is_ascii_lowercase() || c == b'_')
            && !(i > 0 && (bytes[i - 1] == b'$' || is_ident_char(bytes[i - 1])))
            && !(i >= 2 && bytes[i - 1] == b'>' && bytes[i - 2] == b'-')
            && !(i >= 2 && bytes[i - 1] == b':' && bytes[i - 2] == b':');
        if is_ident_start {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let ident = &expr[start..i];
            // Skip PHP keywords
            let is_keyword = matches!(
                ident,
                "true" | "false" | "null" | "and" | "or" | "xor" | "new" | "return"
                | "if" | "else" | "elseif" | "while" | "for" | "foreach" | "do"
                | "switch" | "case" | "break" | "continue" | "function" | "class"
                | "public" | "private" | "protected" | "static" | "use" | "namespace"
                | "as" | "throw" | "try" | "catch" | "finally" | "instanceof"
            );
            // Next non-space char: if it's `(`, this is a function call,
            // not a variable reference — leave it alone.
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let followed_by_call = j < bytes.len() && bytes[j] == b'(';
            if !is_keyword && !followed_by_call && params.iter().any(|p| p == ident) {
                out.push('$');
            }
            out.push_str(ident);
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Extract bracketed key from syntax like "@@:data[key]" or "@@:params[key]"
/// Returns the raw content between [ and ] — including any user-supplied quotes.
/// For languages that need a bare key (C, Rust), call .trim_matches on the result.
/// Extract the key from a dot-accessor: `@@:params.key` → `key`
pub(crate) fn extract_dot_key(text: &str, prefix: &str) -> String {
    if let Some(rest) = text.strip_prefix(prefix) {
        if let Some(rest) = rest.strip_prefix('.') {
            // Extract only the identifier (alphanumeric + underscore)
            let key: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            return key;
        }
    }
    "".to_string()
}

/// Extract state variable name from "$.varName"
pub(crate) fn extract_state_var_name(text: &str) -> String {
    // Skip "$." prefix and get identifier
    if text.starts_with("$.") {
        let after_prefix = &text[2..];
        let end = after_prefix
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after_prefix.len());
        after_prefix[..end].to_string()
    } else {
        "unknown".to_string()
    }
}

/// Expand state variable references ($.varName) and context syntax (@@) in an expression string
/// Expand all Frame segments within an expression string.
///
/// This is the proper way to handle nested Frame constructs inside
/// expressions — e.g., `@@:(@@:params.a + @@:params.b)`. The expression
/// text is wrapped in a synthetic handler body `{ expr }` and run through
/// the full scanner pipeline. Each recognized Frame segment is expanded
/// via `generate_frame_expansion`, and native text passes through.
///
/// This respects the pipeline boundary: scanning happens in the scanner,
/// expansion happens in the expander. No ad-hoc string matching.
pub(crate) fn expand_expression(expr: &str, lang: TargetLanguage, ctx: &HandlerContext) -> String {
    // First expand state vars ($.x) which are handled by a dedicated function
    let with_state_vars = expand_state_vars_in_expr(expr, lang, ctx);

    // If no @@ constructs remain, return early
    if !with_state_vars.contains("@@:") && !with_state_vars.contains("@@") {
        return with_state_vars;
    }

    // Wrap expression in synthetic braces for the scanner
    let synthetic_body = format!("{{{}}}", with_state_vars);
    let body_bytes = synthetic_body.as_bytes();

    // Run the scanner on the synthetic body
    let mut scanner = get_native_scanner(lang);
    let scan_result = match scanner.scan(body_bytes, 0) {
        Ok(r) => r,
        Err(_) => return with_state_vars,
    };

    if std::env::var("FRAME_DEBUG_EXPR").is_ok() {
        eprintln!(
            "[expand_expression] input='{}' regions={}",
            with_state_vars,
            scan_result.regions.len()
        );
        for (i, r) in scan_result.regions.iter().enumerate() {
            match r {
                Region::NativeText { span } => eprintln!(
                    "  [{}] NativeText {:?} = '{}'",
                    i,
                    span,
                    String::from_utf8_lossy(&body_bytes[span.start..span.end])
                ),
                Region::FrameSegment { span, kind, .. } => eprintln!(
                    "  [{}] FrameSegment {:?} {:?} = '{}'",
                    i,
                    kind,
                    span,
                    String::from_utf8_lossy(&body_bytes[span.start..span.end])
                ),
            }
        }
    }

    // Expand each Frame segment
    let mut expansions = Vec::new();
    for region in &scan_result.regions {
        if let Region::FrameSegment {
            span,
            kind,
            metadata,
            ..
        } = region
        {
            let expansion =
                generate_frame_expansion(body_bytes, span, *kind, 0, lang, ctx, metadata);
            expansions.push(expansion);
        }
    }

    // Splice native text + expansions
    let splicer = Splicer;
    let spliced = splicer.splice(body_bytes, &scan_result.regions, &expansions);

    // Remove synthetic braces and trim
    let text = spliced.text.trim();
    text.to_string()
}

/// Expand `$.varName` state variable references within an expression string.
/// Uses compartment.state_vars for Python/TypeScript.
/// For HSM: uses __sv_comp when ctx.use_sv_comp is true (navigates to correct parent compartment)
fn expand_state_vars_in_expr(expr: &str, lang: TargetLanguage, ctx: &HandlerContext) -> String {
    // Note: deliberately NOT string-literal-aware. `$.varName` is
    // expanded even inside string literals because that's the contract
    // for Frame interpolation — e.g. `f"count is {$.count}"` in Python
    // needs `$.count` → `self.__compartment.state_vars["count"]` so the
    // f-string interpolation resolves at runtime. Equivalent pattern
    // applies to JS/TS backtick ``${ }``, Kotlin `"${ }"`, Ruby
    // `"#{}"`, Swift `"\( )"`. Any false-match would require a user
    // literally writing `$.foo` inside a non-interpolating string —
    // exceedingly unusual, and not worth losing interpolation support.
    let mut result = String::new();
    let bytes = expr.as_bytes();

    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'$' && bytes[i + 1] == b'.' {
            // Found $.varName
            i += 2; // Skip "$."
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let var_name = String::from_utf8_lossy(&bytes[start..i]).to_string();
            match lang {
                TargetLanguage::Python3 => {
                    if ctx.per_handler {
                        result.push_str(&format!("compartment.state_vars[\"{}\"]", var_name))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("self.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    if ctx.per_handler {
                        result.push_str(&format!("compartment.state_vars[\"{}\"]", var_name))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("this.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::Rust => {
                    result.push_str(&super::rust_system::rust_expand_state_var_read(
                        ctx, &var_name,
                    ));
                }
                TargetLanguage::C => {
                    let c_type = ctx
                        .state_var_types
                        .get(&var_name)
                        .map(|s| s.as_str())
                        .unwrap_or("int");
                    let cast = match c_type {
                        "char*" | "const char*" | "str" | "string" | "String" => "(const char*)",
                        _ => "(int)(intptr_t)",
                    };
                    if ctx.per_handler {
                        result.push_str(&format!(
                            "{}{}_FrameDict_get(compartment->state_vars, \"{}\")",
                            cast, ctx.system_name, var_name
                        ))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!(
                            "{}{}_FrameDict_get(__sv_comp->state_vars, \"{}\")",
                            cast, ctx.system_name, var_name
                        ))
                    } else {
                        result.push_str(&format!(
                            "{}{}_FrameDict_get(self->__compartment->state_vars, \"{}\")",
                            cast, ctx.system_name, var_name
                        ))
                    }
                }
                TargetLanguage::Cpp => {
                    let cpp_type = ctx
                        .state_var_types
                        .get(&var_name)
                        .map(|t| cpp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    if ctx.per_handler {
                        result.push_str(&format!(
                            "std::any_cast<{}>(compartment->state_vars[\"{}\"])",
                            cpp_type, var_name
                        ))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!(
                            "std::any_cast<{}>(__sv_comp->state_vars[\"{}\"])",
                            cpp_type, var_name
                        ))
                    } else {
                        result.push_str(&format!(
                            "std::any_cast<{}>(__compartment->state_vars[\"{}\"])",
                            cpp_type, var_name
                        ))
                    }
                }
                TargetLanguage::Java => {
                    let java_type = ctx
                        .state_var_types
                        .get(&var_name)
                        .map(|t| java_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let accessor = if ctx.per_handler {
                        format!("compartment.state_vars.get(\"{}\")", var_name)
                    } else if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars.get(\"{}\")", var_name)
                    } else {
                        format!("__compartment.state_vars.get(\"{}\")", var_name)
                    };
                    if java_type == "Object" {
                        result.push_str(&accessor);
                    } else {
                        // Wrap cast in parens so method calls chain correctly:
                        // ((String) map.get("k")).equals("v") not (String) map.get("k").equals("v")
                        result.push_str(&format!("(({}) {})", java_type, accessor));
                    }
                }
                TargetLanguage::Kotlin => {
                    let kt_type = ctx
                        .state_var_types
                        .get(&var_name)
                        .map(|t| kotlin_map_type(t))
                        .unwrap_or_else(|| "Int".to_string());
                    let cast = if kt_type == "Any?" {
                        String::new()
                    } else {
                        format!(" as {}", kt_type)
                    };
                    if ctx.per_handler {
                        result.push_str(&format!(
                            "compartment.state_vars[\"{}\"]{}",
                            var_name, cast
                        ))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]{}", var_name, cast))
                    } else {
                        result.push_str(&format!(
                            "__compartment.state_vars[\"{}\"]{}",
                            var_name, cast
                        ))
                    }
                }
                TargetLanguage::Swift => {
                    let sw_type = ctx
                        .state_var_types
                        .get(&var_name)
                        .map(|t| swift_map_type(t))
                        .unwrap_or_else(|| "Int".to_string());
                    if sw_type == "Any" {
                        if ctx.per_handler {
                            result.push_str(&format!(
                                "compartment.state_vars[\"{}\"]",
                                var_name
                            ))
                        } else if ctx.use_sv_comp {
                            result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                        } else {
                            result.push_str(&format!("__compartment.state_vars[\"{}\"]", var_name))
                        }
                    } else if ctx.per_handler {
                        result.push_str(&format!(
                            "(compartment.state_vars[\"{}\"] as! {})",
                            var_name, sw_type
                        ))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!(
                            "(__sv_comp.state_vars[\"{}\"] as! {})",
                            var_name, sw_type
                        ))
                    } else {
                        result.push_str(&format!(
                            "(__compartment.state_vars[\"{}\"] as! {})",
                            var_name, sw_type
                        ))
                    }
                }
                TargetLanguage::CSharp => {
                    let cs_type = ctx
                        .state_var_types
                        .get(&var_name)
                        .map(|t| csharp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let cast = if cs_type == "object" {
                        String::new()
                    } else {
                        format!("({}) ", cs_type)
                    };
                    if ctx.per_handler {
                        result.push_str(&format!(
                            "{}compartment.state_vars[\"{}\"]",
                            cast, var_name
                        ))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("{}__sv_comp.state_vars[\"{}\"]", cast, var_name))
                    } else {
                        result.push_str(&format!(
                            "{}__compartment.state_vars[\"{}\"]",
                            cast, var_name
                        ))
                    }
                }
                TargetLanguage::Go => {
                    let go_type = ctx
                        .state_var_types
                        .get(&var_name)
                        .map(|t| go_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    if ctx.per_handler {
                        result.push_str(&format!(
                            "compartment.stateVars[\"{}\"].({})",
                            var_name, go_type
                        ))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!(
                            "__sv_comp.stateVars[\"{}\"].({})",
                            var_name, go_type
                        ))
                    } else {
                        result.push_str(&format!(
                            "s.compartment.stateVars[\"{}\"].({})",
                            var_name, go_type
                        ))
                    }
                }
                TargetLanguage::Php => {
                    if ctx.per_handler {
                        result.push_str(&format!("$compartment->state_vars[\"{}\"]", var_name))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("$__sv_comp->state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!(
                            "$this->__compartment->state_vars[\"{}\"]",
                            var_name
                        ))
                    }
                }
                TargetLanguage::Ruby => {
                    if ctx.per_handler {
                        result.push_str(&format!("compartment.state_vars[\"{}\"]", var_name))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("@__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::Lua => {
                    if ctx.per_handler {
                        result.push_str(&format!("compartment.state_vars[\"{}\"]", var_name))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("self.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::Dart => {
                    if ctx.per_handler {
                        result.push_str(&format!("compartment.state_vars[\"{}\"]", var_name))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("this.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::GDScript => {
                    if ctx.per_handler {
                        result.push_str(&format!("compartment.state_vars[\"{}\"]", var_name))
                    } else if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("self.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::Erlang => {
                    // Leave `self.` placeholder so the body processor's
                    // classifier substitutes with the live `DataN#data.`
                    // — matches the live data_gen. See sibling fix in
                    // `FrameSegmentKind::StateVarRead` above.
                    let state_prefix = to_snake_case(&ctx.state_name);
                    result.push_str(&format!("self.sv_{}_{}", state_prefix, var_name))
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Generate a pop-transition with optional RFC-0008 decorations
/// (exit_args, enter_args, is_forward). Each backend emits:
///   1. exit_args writes on current compartment (if present)
///   2. Pop from stack into __saved
///   3. Clear + write enter_args on __saved (if present)
///   4. Set forward_event on __saved (if is_forward)
///   5. __transition(__saved) + return
fn generate_pop_transition(
    indent: &str,
    ctx: &HandlerContext,
    lang: TargetLanguage,
    exit_args: &Option<String>,
    enter_args: &Option<String>,
    is_forward: bool,
) -> String {
    let mut code = String::new();

    // Helper: emit exit_args writes on current compartment (positional append)
    if let Some(ref exit) = exit_args {
        for arg in exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()) {
            let value = if let Some(eq_pos) = arg.find('=') {
                arg[eq_pos + 1..].trim()
            } else {
                arg
            };
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    code.push_str(&format!(
                        "{}self.__compartment.exit_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    code.push_str(&format!(
                        "{}this.__compartment.exit_args.push({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Dart => {
                    code.push_str(&format!(
                        "{}this.__compartment.exit_args.add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Rust => {
                    code.push_str(&super::rust_system::rust_pop_exit_arg(indent, value));
                }
                TargetLanguage::C => {
                    code.push_str(&format!("{}{}_FrameVec_push(self->__compartment->exit_args, (void*)(intptr_t)({}));\n", indent, ctx.system_name, value));
                }
                TargetLanguage::Cpp => {
                    code.push_str(&format!(
                        "{}__compartment->exit_args.push_back(std::any({}));\n",
                        indent, value
                    ));
                }
                TargetLanguage::Java => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Kotlin => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.add({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Swift => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::CSharp => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.Add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Go => {
                    code.push_str(&format!(
                        "{}s.__compartment.exitArgs = append(s.__compartment.exitArgs, {})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Php => {
                    code.push_str(&format!(
                        "{}$this->__compartment->exit_args[] = {};\n",
                        indent, value
                    ));
                }
                TargetLanguage::Ruby => {
                    code.push_str(&format!(
                        "{}@__compartment.exit_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Lua => {
                    code.push_str(&format!(
                        "{}table.insert(self.__compartment.exit_args, {})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Erlang | TargetLanguage::Graphviz => {}
            }
        }
    }

    // Pop from stack
    match lang {
        TargetLanguage::Python3 => code.push_str(&format!("{}__saved = self._state_stack.pop()\n", indent)),
        TargetLanguage::GDScript => code.push_str(&format!("{}var __saved = self._state_stack.pop_back()\n", indent)),
        TargetLanguage::TypeScript => code.push_str(&format!("{}const __saved = this._state_stack.pop()!;\n", indent)),
        TargetLanguage::Dart => code.push_str(&format!("{}final __saved = this._state_stack.removeLast();\n", indent)),
        TargetLanguage::JavaScript => code.push_str(&format!("{}const __saved = this._state_stack.pop();\n", indent)),
        TargetLanguage::Rust => code.push_str(&super::rust_system::rust_pop_stack(indent)),
        TargetLanguage::C => code.push_str(&format!("{}{}_Compartment* __saved = ({}_Compartment*){}_FrameVec_pop(self->_state_stack);\n", indent, ctx.system_name, ctx.system_name, ctx.system_name)),
        TargetLanguage::Cpp => code.push_str(&format!("{}auto __saved = std::move(_state_stack.back()); _state_stack.pop_back();\n", indent)),
        TargetLanguage::Java => code.push_str(&format!("{}var __saved = _state_stack.remove(_state_stack.size() - 1);\n", indent)),
        TargetLanguage::Kotlin => code.push_str(&format!("{}val __saved = _state_stack.removeAt(_state_stack.size - 1)\n", indent)),
        TargetLanguage::Swift => code.push_str(&format!("{}let __saved = _state_stack.removeLast()\n", indent)),
        TargetLanguage::CSharp => code.push_str(&format!("{}var __saved = _state_stack[_state_stack.Count - 1]; _state_stack.RemoveAt(_state_stack.Count - 1);\n", indent)),
        TargetLanguage::Go => {
            code.push_str(&format!("{}__saved := s._state_stack[len(s._state_stack)-1]\n", indent));
            code.push_str(&format!("{}s._state_stack = s._state_stack[:len(s._state_stack)-1]\n", indent));
        }
        TargetLanguage::Php => code.push_str(&format!("{}$__saved = array_pop($this->_state_stack);\n", indent)),
        TargetLanguage::Ruby => code.push_str(&format!("{}__saved = @_state_stack.pop\n", indent)),
        TargetLanguage::Lua => code.push_str(&format!("{}local __saved = table.remove(self._state_stack)\n", indent)),
        TargetLanguage::Erlang => {
            code.push_str(&format!("{}[__PoppedState | __RestStack] = Data#data.frame_stack,\n", indent));
            // Erlang pop with decorations: handled via gen_statem pattern
            code.push_str(&format!("{}{{next_state, __PoppedState, Data#data{{frame_stack = __RestStack}}, [{{reply, From, ok}}]}}", indent));
            return code;
        }
        TargetLanguage::Graphviz => unreachable!(),
    }

    // Fresh enter_args: clear + write (RFC-0008 replace semantics, positional append).
    // The arg expression arrives straight from the Frame source — `$.items`,
    // `self.field`, `@@:params.name` and friends are Frame sigils that the
    // standard expression expander resolves to language-specific accessors
    // (e.g. `$.items` → `__sv_comp.state_vars["items"]` in Python). Without
    // this expansion pop-args like `-> ($.items) pop$` would emit the raw
    // sigil into native code and blow up at parse time.
    if let Some(ref enter) = enter_args {
        for arg in enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()) {
            let raw_value = if let Some(eq_pos) = arg.find('=') {
                arg[eq_pos + 1..].trim()
            } else {
                arg
            };
            let value_owned = expand_expression(raw_value, lang, ctx);
            let value = value_owned.as_str();
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.push({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Dart => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Rust => {
                    code.push_str(&super::rust_system::rust_pop_enter_arg(indent, value));
                }
                TargetLanguage::C => {
                    code.push_str(&format!(
                        "{}{}_FrameVec_push(__saved->enter_args, (void*)(intptr_t)({}));\n",
                        indent, ctx.system_name, value
                    ));
                }
                TargetLanguage::Cpp => {
                    code.push_str(&format!(
                        "{}__saved->enter_args.push_back(std::any({}));\n",
                        indent, value
                    ));
                }
                TargetLanguage::Java => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Kotlin => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.add({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Swift => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::CSharp => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.Add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Go => {
                    code.push_str(&format!(
                        "{}__saved.enterArgs = append(__saved.enterArgs, {})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Php => {
                    code.push_str(&format!(
                        "{}$__saved->enter_args[] = {};\n",
                        indent, value
                    ));
                }
                TargetLanguage::Ruby => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Lua => {
                    code.push_str(&format!(
                        "{}table.insert(__saved.enter_args, {})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Erlang | TargetLanguage::Graphviz => {}
            }
        }
    }

    // Forward event (RFC-0008: -> => pop$)
    if is_forward {
        match lang {
            TargetLanguage::Python3 | TargetLanguage::GDScript => {
                code.push_str(&format!("{}__saved.forward_event = __e\n", indent));
            }
            TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
                code.push_str(&format!("{}__saved.forward_event = __e;\n", indent));
            }
            TargetLanguage::Rust => {
                code.push_str(&super::rust_system::rust_pop_forward(indent));
            }
            TargetLanguage::C => {
                code.push_str(&format!("{}__saved->forward_event = __e;\n", indent));
            }
            TargetLanguage::Cpp => {
                code.push_str(&format!(
                    "{}__saved->forward_event = std::make_unique<{}FrameEvent>(__e);\n",
                    indent, ctx.system_name
                ));
            }
            TargetLanguage::Java
            | TargetLanguage::Kotlin
            | TargetLanguage::Swift
            | TargetLanguage::CSharp => {
                code.push_str(&format!("{}__saved.forward_event = __e;\n", indent));
            }
            TargetLanguage::Go => {
                code.push_str(&format!("{}__saved.forwardEvent = __e\n", indent));
            }
            TargetLanguage::Php => {
                code.push_str(&format!("{}$__saved->forward_event = $__e;\n", indent));
            }
            TargetLanguage::Ruby => {
                code.push_str(&format!("{}__saved.forward_event = __e\n", indent));
            }
            TargetLanguage::Lua => {
                code.push_str(&format!("{}__saved.forward_event = __e\n", indent));
            }
            TargetLanguage::Erlang | TargetLanguage::Graphviz => {}
        }
    }

    // Transition + return
    let var = if matches!(lang, TargetLanguage::Rust) {
        super::rust_system::rust_pop_var_name()
    } else {
        "__saved"
    };
    match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => {
            code.push_str(&format!(
                "{}self.__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
            code.push_str(&format!(
                "{}this.__transition({});\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Rust => {
            code.push_str(&super::rust_system::rust_pop_transition(indent));
        }
        TargetLanguage::C => {
            code.push_str(&format!(
                "{}{}_transition(self, {});\n{}return;",
                indent, ctx.system_name, var, indent
            ));
        }
        TargetLanguage::Cpp => {
            code.push_str(&format!(
                "{}__transition(std::move({}));\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Java => {
            code.push_str(&format!(
                "{}__transition({});\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Kotlin => {
            code.push_str(&format!(
                "{}__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::Swift => {
            code.push_str(&format!(
                "{}__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::CSharp => {
            code.push_str(&format!(
                "{}__transition({});\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Go => {
            code.push_str(&format!(
                "{}s.__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::Php => {
            code.push_str(&format!(
                "{}$this->__transition(${});\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Ruby => {
            code.push_str(&format!(
                "{}__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::Lua => {
            code.push_str(&format!(
                "{}self:__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::Erlang | TargetLanguage::Graphviz => {}
    }

    code
}

/// Get the native region scanner for the target language
/// Split a transition expansion into the body (compartment setup +
/// `__transition()` call) and the trailing `return` keyword.
///
/// Transition expansions always end with `return` or `return;` to exit
/// the handler after the state change. The orchestrator needs these
/// separated so it can insert a return-expr between the body and the
/// return when `-> $State` is followed by `@@:(expr)` in the same scope.
fn split_transition_return(expansion: &str) -> (&str, &str) {
    let trimmed = expansion.trim_end();
    if trimmed.ends_with("return;") {
        (trimmed[..trimmed.len() - 7].trim_end(), "return;")
    } else if trimmed.ends_with("return") {
        (trimmed[..trimmed.len() - 6].trim_end(), "return")
    } else {
        // Expansion doesn't end with return (e.g., Rust uses different
        // control flow, or Graphviz). Emit as-is.
        (trimmed, "")
    }
}

/// Generate the transition guard check for a self-call.
/// Emitted by the orchestrator AFTER the line containing the self-call
/// expression, on its own line at the given indentation.
pub(crate) fn generate_self_call_guard(
    indent: usize,
    lang: TargetLanguage,
    system_name: &str,
) -> String {
    let ind = " ".repeat(indent);
    match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => format!(
            "{}if self._context_stack[-1]._transitioned:\n{}    return",
            ind, ind
        ),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!(
            "{}if (this._context_stack[this._context_stack.length - 1]._transitioned) return;",
            ind
        ),
        TargetLanguage::Dart => format!(
            "{}if (this._context_stack[this._context_stack.length - 1]._transitioned) return;",
            ind
        ),
        TargetLanguage::Rust => format!(
            "{}if self._context_stack.last().map_or(false, |ctx| ctx._transitioned) {{ return; }}",
            ind
        ),
        TargetLanguage::C => format!(
            "{}if ({}_CTX(self)->_transitioned) return;",
            ind, system_name
        ),
        TargetLanguage::Cpp => format!("{}if (_context_stack.back()._transitioned) return;", ind),
        TargetLanguage::Java => format!(
            "{}if (_context_stack.get(_context_stack.size() - 1)._transitioned) return;",
            ind
        ),
        TargetLanguage::Kotlin => format!(
            "{}if (_context_stack[_context_stack.size - 1]._transitioned) return",
            ind
        ),
        TargetLanguage::Swift => format!(
            "{}if _context_stack[_context_stack.count - 1]._transitioned {{ return }}",
            ind
        ),
        TargetLanguage::CSharp => format!(
            "{}if (_context_stack[_context_stack.Count - 1]._transitioned) return;",
            ind
        ),
        TargetLanguage::Go => format!(
            "{}if s._context_stack[len(s._context_stack)-1]._transitioned {{ return }}",
            ind
        ),
        TargetLanguage::Php => format!(
            "{}if ($this->_context_stack[count($this->_context_stack) - 1]->_transitioned) return;",
            ind
        ),
        TargetLanguage::Ruby => format!(
            "{}return if @_context_stack[@_context_stack.length - 1]._transitioned",
            ind
        ),
        TargetLanguage::Lua => format!(
            "{}if self._context_stack[#self._context_stack]._transitioned then return end",
            ind
        ),
        TargetLanguage::Erlang | TargetLanguage::Graphviz => String::new(),
    }
}

pub(crate) fn get_native_scanner(lang: TargetLanguage) -> Box<dyn NativeRegionScanner> {
    match lang {
        TargetLanguage::Python3 => Box::new(NativeRegionScannerPy),
        TargetLanguage::TypeScript => Box::new(NativeRegionScannerTs),
        TargetLanguage::JavaScript => Box::new(NativeRegionScannerJs),
        TargetLanguage::Rust => Box::new(NativeRegionScannerRust),
        TargetLanguage::CSharp => Box::new(NativeRegionScannerCs),
        TargetLanguage::C => Box::new(NativeRegionScannerC),
        TargetLanguage::Cpp => Box::new(NativeRegionScannerCpp),
        TargetLanguage::Java => Box::new(NativeRegionScannerJava),
        TargetLanguage::Kotlin => Box::new(NativeRegionScannerKotlin),
        TargetLanguage::Swift => Box::new(NativeRegionScannerSwift),
        TargetLanguage::Go => Box::new(NativeRegionScannerGo),
        TargetLanguage::Php => Box::new(NativeRegionScannerPhp),
        TargetLanguage::Ruby => Box::new(NativeRegionScannerRuby),
        TargetLanguage::Erlang => Box::new(NativeRegionScannerErlang),
        TargetLanguage::Lua => Box::new(NativeRegionScannerLua),
        TargetLanguage::Dart => Box::new(NativeRegionScannerDart),
        TargetLanguage::GDScript => Box::new(NativeRegionScannerGDScript),
        // Graphviz is an output-only target (emitted from the SystemGraph IR,
        // not from native code). The validator still scans for Frame tokens
        // (e.g. @@:self.method()) during the graphviz compile path; those
        // tokens are target-language-agnostic, so any skipper works. Use the
        // Python scanner as a neutral default.
        TargetLanguage::Graphviz => Box::new(NativeRegionScannerPy),
    }
}

/// Expand `@@:system.state` to the target-language compartment state accessor.
/// Used by both handler body expansion and operation body expansion.
pub(crate) fn expand_system_state(lang: TargetLanguage) -> String {
    match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => {
            "self.__compartment.state".to_string()
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
            "this.__compartment.state".to_string()
        }
        TargetLanguage::Rust => super::rust_system::rust_system_state(),
        TargetLanguage::C => "self->__compartment->state".to_string(),
        TargetLanguage::Cpp => "__compartment->state".to_string(),
        TargetLanguage::Java | TargetLanguage::Kotlin | TargetLanguage::CSharp => {
            "__compartment.state".to_string()
        }
        TargetLanguage::Swift => "__compartment.state".to_string(),
        TargetLanguage::Go => "s.__compartment.state".to_string(),
        TargetLanguage::Php => "$this->__compartment->state".to_string(),
        TargetLanguage::Ruby => "@__compartment.state".to_string(),
        TargetLanguage::Lua => "self.__compartment.state".to_string(),
        TargetLanguage::Erlang => "\"\"".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    }
}

/// Expand `@@:system.state` occurrences in operation body code.
/// Operations are native code but `@@:system.state` is a read-only accessor
/// that's safe in non-static operations.
pub(crate) fn expand_system_state_in_code(code: &str, lang: TargetLanguage) -> String {
    let mut result = code.to_string();

    // Expand @@:system.state → compartment accessor
    if result.contains("@@:system.state") {
        result = result.replace("@@:system.state", &expand_system_state(lang));
    }

    // Expand @@:(expr) → return expr
    // In operation bodies, @@:(expr) means "return this value" (no context stack).
    // This handles patterns like @@:(@@:system.state) where the inner was already expanded.
    while let Some(start) = result.find("@@:(") {
        let after = start + 4; // position after "@@:("
        let bytes = result.as_bytes();
        let mut depth = 1i32;
        let mut j = after;
        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                j += 1;
            }
        }
        if depth == 0 {
            let expr = &result[after..j];
            let expansion = match lang {
                // Erlang: last expression IS the return value
                TargetLanguage::Erlang => expr.to_string(),
                // No-semicolon languages
                TargetLanguage::Python3
                | TargetLanguage::GDScript
                | TargetLanguage::Ruby
                | TargetLanguage::Kotlin
                | TargetLanguage::Swift
                | TargetLanguage::Lua
                | TargetLanguage::Go => format!("return {}", expr),
                // Semicolon languages
                _ => format!("return {};", expr),
            };
            result = format!("{}{}{}", &result[..start], expansion, &result[j + 1..]);
        } else {
            break; // unmatched paren — bail
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::native_region_scanner::FrameSegmentKind;
    use crate::frame_c::visitors::TargetLanguage;

    fn make_ctx(state_var_types: Vec<(&str, &str)>) -> HandlerContext {
        HandlerContext {
            system_name: "TestSys".to_string(),
            state_name: "S1".to_string(),
            event_name: "foo".to_string(),
            parent_state: None,
            defined_systems: std::collections::HashSet::new(),
            use_sv_comp: false,
            per_handler: false,
            state_var_types: state_var_types
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            state_param_names: std::collections::HashMap::new(),
            state_enter_param_names: std::collections::HashMap::new(),
            state_exit_param_names: std::collections::HashMap::new(),
            event_param_names: std::collections::HashMap::new(),
            state_hsm_parents: std::collections::HashMap::new(),
            current_return_type: None,
        }
    }

    /// Helper: call generate_frame_expansion with text as bytes + span
    fn expand(
        kind: FrameSegmentKind,
        text: &str,
        lang: TargetLanguage,
        ctx: &HandlerContext,
    ) -> String {
        let bytes = text.as_bytes();
        let span = crate::frame_c::compiler::native_region_scanner::RegionSpan {
            start: 0,
            end: bytes.len(),
        };
        generate_frame_expansion(bytes, &span, kind, 0, lang, ctx, &SegmentMetadata::None)
    }

    // =========================================================
    // Rust @@:(expr) — string literals wrapped with String::from
    // =========================================================

    #[test]
    fn test_context_return_expr_rust_string_wraps() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(\"green\")",
            TargetLanguage::Rust,
            &ctx,
        );
        assert!(
            result.contains("String::from(\"green\")"),
            "Rust @@:(\"green\") should wrap with String::from, got: {}",
            result
        );
    }

    #[test]
    fn test_context_return_expr_rust_int_no_wrap() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(42)",
            TargetLanguage::Rust,
            &ctx,
        );
        assert!(
            result.contains("Box::new(42)"),
            "Rust @@:(42) should NOT wrap with String::from, got: {}",
            result
        );
        assert!(
            !result.contains("String::from"),
            "Integer should not get String::from wrapping, got: {}",
            result
        );
    }

    #[test]
    fn test_context_return_expr_python_no_wrap() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(\"green\")",
            TargetLanguage::Python3,
            &ctx,
        );
        assert!(
            !result.contains("String::from"),
            "Python should NOT wrap string literals, got: {}",
            result
        );
        assert!(
            result.contains("\"green\""),
            "Python should pass through the literal, got: {}",
            result
        );
    }

    // =========================================================
    // Rust @@:return = expr — same wrapping
    // =========================================================

    #[test]
    fn test_context_return_assign_rust_string_wraps() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturn,
            "@@:return = \"hello\"",
            TargetLanguage::Rust,
            &ctx,
        );
        assert!(
            result.contains("String::from(\"hello\")"),
            "Rust @@:return = \"hello\" should wrap, got: {}",
            result
        );
    }

    #[test]
    fn test_context_return_assign_rust_int_no_wrap() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturn,
            "@@:return = 42",
            TargetLanguage::Rust,
            &ctx,
        );
        assert!(
            !result.contains("String::from"),
            "Rust @@:return = 42 should NOT wrap, got: {}",
            result
        );
    }

    // =========================================================
    // Rust state var READ — .clone() for non-Copy types only
    // =========================================================

    #[test]
    fn test_state_var_read_rust_string_clones() {
        let ctx = make_ctx(vec![("name", "String")]);
        let result = expand_expression("$.name", TargetLanguage::Rust, &ctx);
        assert!(
            result.contains(".clone()"),
            "String state var read should add .clone(), got: {}",
            result
        );
    }

    #[test]
    fn test_state_var_read_rust_int_no_clone() {
        let ctx = make_ctx(vec![("count", "i32")]);
        let result = expand_expression("$.count", TargetLanguage::Rust, &ctx);
        assert!(
            !result.contains(".clone()"),
            "i32 state var read should NOT add .clone(), got: {}",
            result
        );
    }

    #[test]
    fn test_state_var_read_rust_bool_no_clone() {
        let ctx = make_ctx(vec![("flag", "bool")]);
        let result = expand_expression("$.flag", TargetLanguage::Rust, &ctx);
        assert!(
            !result.contains(".clone()"),
            "bool state var read should NOT add .clone(), got: {}",
            result
        );
    }

    #[test]
    fn test_state_var_read_rust_unknown_type_clones() {
        let ctx = make_ctx(vec![]);
        let result = expand_expression("$.mystery", TargetLanguage::Rust, &ctx);
        assert!(
            result.contains(".clone()"),
            "Unknown-type state var should clone for safety, got: {}",
            result
        );
    }
}
