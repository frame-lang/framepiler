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
    java_map_type, kotlin_map_type, state_var_init_value, swift_map_type, to_snake_case,
    type_to_cpp_string, HandlerContext,
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

/// Resolve the storage key for a positional state-arg in a transition.
/// Returns the declared param name if the target state has one at this index,
/// otherwise falls back to the integer index.
pub(crate) fn resolve_state_arg_key(i: usize, target_state: &str, ctx: &HandlerContext) -> String {
    ctx.state_param_names
        .get(target_state)
        .and_then(|names| names.get(i))
        .cloned()
        .unwrap_or_else(|| i.to_string())
}

/// Resolve the storage key for a positional enter-arg in a transition.
/// Returns the declared enter param name if available, otherwise the integer index.
pub(crate) fn resolve_enter_arg_key(i: usize, target_state: &str, ctx: &HandlerContext) -> String {
    ctx.state_enter_param_names
        .get(target_state)
        .and_then(|names| names.get(i))
        .cloned()
        .unwrap_or_else(|| i.to_string())
}

/// Resolve the storage key for a positional exit-arg in a transition.
///
/// Exit args belong to the SOURCE state of the transition (the state
/// being exited), not the target. Look up the source state's exit
/// handler param at index `i` and return its declared name.
pub(crate) fn resolve_exit_arg_key(i: usize, ctx: &HandlerContext) -> String {
    ctx.state_exit_param_names
        .get(&ctx.state_name)
        .and_then(|names| names.get(i))
        .cloned()
        .unwrap_or_else(|| i.to_string())
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
                match lang {
                    TargetLanguage::Python3 => {
                        let mut code = String::new();
                        code.push_str(&format!("{}__compartment = {}Compartment(\"{}\", parent_compartment=self.__compartment)\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}self.__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::GDScript => {
                        let mut code = String::new();
                        code.push_str(&format!("{}var __compartment = {}Compartment.new(\"{}\", self.__compartment.copy())\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}self.__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                        let mut code = String::new();
                        code.push_str(&format!("{}const __compartment = new {}Compartment(\"{}\", this.__compartment.copy());\n", indent_str, ctx.system_name, target));
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
                        let mut code = String::new();
                        code.push_str(&format!("{}final __compartment = {}Compartment(\"{}\", this.__compartment.copy());\n", indent_str, ctx.system_name, target));
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
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}{}_Compartment* __compartment = {}_Compartment_new(\"{}\");\n",
                            indent_str, ctx.system_name, ctx.system_name, target
                        ));
                        if ctx.parent_state.is_some() {
                            code.push_str(&format!("{}__compartment->parent_compartment = {}_Compartment_ref(self->__compartment);\n", indent_str, ctx.system_name));
                        }
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
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}auto __new_compartment = std::make_shared<{}Compartment>(\"{}\");\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__new_compartment->parent_compartment = __compartment;\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}__new_compartment->forward_event = std::make_unique<{}FrameEvent>(__e);\n", indent_str, ctx.system_name));
                        code.push_str(&format!(
                            "{}__transition(std::move(__new_compartment));\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Java => {
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}var __compartment = new {}Compartment(\"{}\");\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.parent_compartment = this.__compartment;\n",
                            indent_str
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
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}val __compartment = {}Compartment(\"{}\")\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.parent_compartment = this.__compartment\n",
                            indent_str
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
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}let __compartment = {}Compartment(state: \"{}\")\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.parent_compartment = self.__compartment\n",
                            indent_str
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
                        let mut code = String::new();
                        code.push_str(&format!("{}$__compartment = new {}Compartment(\"{}\", $this->__compartment->copy());\n", indent_str, ctx.system_name, target));
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
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}{{ var __new_compartment = new {}Compartment(\"{}\");\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__new_compartment.parent_compartment = __compartment;\n",
                            indent_str
                        ));
                        code.push_str(&format!(
                            "{}__new_compartment.forward_event = __e;\n",
                            indent_str
                        ));
                        code.push_str(&format!(
                            "{}__transition(__new_compartment); }}\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}return;", indent_str));
                        code
                    }
                    TargetLanguage::Go => {
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}__compartment := new{}Compartment(\"{}\")\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.parentCompartment = s.__compartment.copy()\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}__compartment.forwardEvent = __e\n", indent_str));
                        code.push_str(&format!("{}s.__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::Ruby => {
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}__compartment = {}Compartment.new(\"{}\", @__compartment.copy)\n",
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
                    TargetLanguage::Lua => {
                        let mut code = String::new();
                        code.push_str(&format!(
                            "{}local __compartment = {}Compartment.new(\"{}\")\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.forward_event = __e\n",
                            indent_str
                        ));
                        code.push_str(&format!("{}self:__transition(__compartment)\n", indent_str));
                        code.push_str(&format!("{}return", indent_str));
                        code
                    }
                    TargetLanguage::Erlang => String::new(),
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
                        // Create compartment, set fields, call __transition
                        // Store exit_args in CURRENT compartment before creating new one
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        let key = resolve_exit_arg_key(i, ctx);
                                        format!("\"{}\": {}", key, a)
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}self.__compartment.exit_args = {{{}}}\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}__compartment = {}Compartment(\"{}\", parent_compartment=self.__compartment)\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        // Check for named argument (e.g., "k=3")
                                        if let Some(eq_pos) = a.find('=') {
                                            let name = a[..eq_pos].trim();
                                            let value = a[eq_pos + 1..].trim();
                                            format!("\"{}\": {}", name, value)
                                        } else {
                                            // Positional argument - resolve to declared param name
                                            let key = resolve_state_arg_key(i, &target, ctx);
                                            format!("\"{}\": {}", key, a)
                                        }
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}__compartment.state_args = {{{}}}\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Set enter_args if present (named keys, mirrors state_args)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        if let Some(eq_pos) = a.find('=') {
                                            let name = a[..eq_pos].trim();
                                            let value = a[eq_pos + 1..].trim();
                                            format!("\"{}\": {}", name, value)
                                        } else {
                                            let key = resolve_enter_arg_key(i, &target, ctx);
                                            format!("\"{}\": {}", key, a)
                                        }
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}__compartment.enter_args = {{{}}}\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!(
                            "{}self.__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::GDScript => {
                        // GDScript: .new() constructor, no keyword args, no dict comprehension
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            let entries: Vec<String> = args
                                .iter()
                                .enumerate()
                                .map(|(i, a)| {
                                    let key = resolve_exit_arg_key(i, ctx);
                                    format!("\"{}\": {}", key, a)
                                })
                                .collect();
                            code.push_str(&format!(
                                "{}self.__compartment.exit_args = {{{}}}\n",
                                indent_str,
                                entries.join(", ")
                            ));
                        }

                        // Create new compartment: positional args, .new() constructor
                        code.push_str(&format!("{}var __compartment = {}Compartment.new(\"{}\", self.__compartment.copy())\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        if let Some(eq_pos) = a.find('=') {
                                            let name = a[..eq_pos].trim();
                                            let value = a[eq_pos + 1..].trim();
                                            format!("\"{}\": {}", name, value)
                                        } else {
                                            let key = resolve_state_arg_key(i, &target, ctx);
                                            format!("\"{}\": {}", key, a)
                                        }
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}__compartment.state_args = {{{}}}\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Set enter_args if present (named keys, mirrors state_args)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        if let Some(eq_pos) = a.find('=') {
                                            let name = a[..eq_pos].trim();
                                            let value = a[eq_pos + 1..].trim();
                                            format!("\"{}\": {}", name, value)
                                        } else {
                                            let key = resolve_enter_arg_key(i, &target, ctx);
                                            format!("\"{}\": {}", key, a)
                                        }
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}__compartment.enter_args = {{{}}}\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Call __transition and return
                        code.push_str(&format!(
                            "{}self.__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                        // Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        let key = resolve_exit_arg_key(i, ctx);
                                        format!("\"{}\": {}", key, a)
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}this.__compartment.exit_args = {{{}}};\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}const __compartment = new {}Compartment(\"{}\", this.__compartment.copy());\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        // Check for named argument (e.g., "k=3")
                                        if let Some(eq_pos) = a.find('=') {
                                            let name = a[..eq_pos].trim();
                                            let value = a[eq_pos + 1..].trim();
                                            format!("\"{}\": {}", name, value)
                                        } else {
                                            // Positional argument - resolve to declared param name
                                            let key = resolve_state_arg_key(i, &target, ctx);
                                            format!("\"{}\": {}", key, a)
                                        }
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}__compartment.state_args = {{{}}};\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Set enter_args if present (named keys, mirrors state_args)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        if let Some(eq_pos) = a.find('=') {
                                            let name = a[..eq_pos].trim();
                                            let value = a[eq_pos + 1..].trim();
                                            format!("\"{}\": {}", name, value)
                                        } else {
                                            let key = resolve_enter_arg_key(i, &target, ctx);
                                            format!("\"{}\": {}", key, a)
                                        }
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}__compartment.enter_args = {{{}}};\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!(
                            "{}this.__transition(__compartment);\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Dart => {
                        // Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}this.__compartment.exit_args[\"{}\"] = {};\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}final __compartment = {}Compartment(\"{}\", this.__compartment.copy());\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        if let Some(eq_pos) = a.find('=') {
                                            let name = a[..eq_pos].trim();
                                            let value = a[eq_pos + 1..].trim();
                                            format!("\"{}\": {}", name, value)
                                        } else {
                                            let key = resolve_state_arg_key(i, &target, ctx);
                                            format!("\"{}\": {}", key, a)
                                        }
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}__compartment.state_args = {{{}}};\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.enter_args[\"{}\"] = {};\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        // Call __transition and return to exit the handler
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
                        // C: Create compartment and call transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}{}_FrameDict_set(self->__compartment->exit_args, \"{}\", (void*)(intptr_t)({}));\n", indent_str, ctx.system_name, key, arg));
                            }
                        }

                        // Create new compartment
                        code.push_str(&format!(
                            "{}{}_Compartment* __compartment = {}_Compartment_new(\"{}\");\n",
                            indent_str, ctx.system_name, ctx.system_name, target
                        ));
                        if ctx.parent_state.is_some() {
                            code.push_str(&format!("{}__compartment->parent_compartment = {}_Compartment_ref(self->__compartment);\n", indent_str, ctx.system_name));
                        }

                        // Set state_args if present (split by comma for positional args)
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_state_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}{}_FrameDict_set(__compartment->state_args, \"{}\", (void*)(intptr_t)({}));\n", indent_str, ctx.system_name, key, arg));
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}{}_FrameDict_set(__compartment->enter_args, \"{}\", (void*)(intptr_t)({}));\n", indent_str, ctx.system_name, key, arg));
                            }
                        }

                        // Call transition and return to exit the handler
                        code.push_str(&format!(
                            "{}{}_transition(self, __compartment);\n{}return;",
                            indent_str, ctx.system_name, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Cpp => {
                        // C++: Create shared_ptr compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let wrapped = cpp_wrap_any_arg(arg);
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}__compartment->exit_args[\"{}\"] = std::any({});\n",
                                    indent_str, key, wrapped
                                ));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!(
                            "{}auto __new_compartment = std::make_shared<{}Compartment>(\"{}\");\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__new_compartment->parent_compartment = __compartment;\n",
                            indent_str
                        ));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = cpp_wrap_any_arg(a[eq_pos + 1..].trim());
                                        code.push_str(&format!("{}__new_compartment->state_args[\"{}\"] = std::any({});\n", indent_str, name, value));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        let wrapped = cpp_wrap_any_arg(a);
                                        code.push_str(&format!("{}__new_compartment->state_args[\"{}\"] = std::any({});\n", indent_str, key, wrapped));
                                    }
                                }
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let wrapped = cpp_wrap_any_arg(arg);
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__new_compartment->enter_args[\"{}\"] = std::any({});\n",
                                    indent_str, key, wrapped
                                ));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!(
                            "{}__transition(std::move(__new_compartment));\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Java => {
                        // Java: Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.exit_args.put(\"{}\", {});\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!(
                            "{}var __compartment = new {}Compartment(\"{}\");\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.parent_compartment = this.__compartment;\n",
                            indent_str
                        ));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!(
                                            "{}__compartment.state_args.put(\"{}\", {});\n",
                                            indent_str, name, value
                                        ));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!(
                                            "{}__compartment.state_args.put(\"{}\", {});\n",
                                            indent_str, key, a
                                        ));
                                    }
                                }
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.enter_args.put(\"{}\", {});\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!(
                            "{}__transition(__compartment);\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Kotlin => {
                        // Kotlin: no `new`, no semicolons, [] indexer
                        let mut code = String::new();

                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.exit_args[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}val __compartment = {}Compartment(\"{}\")\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.parent_compartment = this.__compartment\n",
                            indent_str
                        ));

                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!(
                                            "{}__compartment.state_args[\"{}\"] = {}\n",
                                            indent_str, name, value
                                        ));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!(
                                            "{}__compartment.state_args[\"{}\"] = {}\n",
                                            indent_str, key, a
                                        ));
                                    }
                                }
                            }
                        }

                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.enter_args[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Swift => {
                        // Swift: no `new`, no semicolons, `let`, `nil`
                        let mut code = String::new();

                        // Store exit_args on CURRENT compartment (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}self.__compartment.exit_args[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}let __compartment = {}Compartment(state: \"{}\")\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.parent_compartment = self.__compartment\n",
                            indent_str
                        ));

                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!(
                                            "{}__compartment.state_args[\"{}\"] = {}\n",
                                            indent_str, name, value
                                        ));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!(
                                            "{}__compartment.state_args[\"{}\"] = {}\n",
                                            indent_str, key, a
                                        ));
                                    }
                                }
                            }
                        }

                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.enter_args[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::CSharp => {
                        // C#: Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.exit_args[\"{}\"] = {};\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        // Create new compartment — block scope prevents redeclaration in multiple branches
                        code.push_str(&format!(
                            "{}{{ var __new_compartment = new {}Compartment(\"{}\");\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__new_compartment.parent_compartment = __compartment;\n",
                            indent_str
                        ));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!(
                                            "{}__new_compartment.state_args[\"{}\"] = {};\n",
                                            indent_str, name, value
                                        ));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!(
                                            "{}__new_compartment.state_args[\"{}\"] = {};\n",
                                            indent_str, key, a
                                        ));
                                    }
                                }
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__new_compartment.enter_args[\"{}\"] = {};\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!(
                            "{}__transition(__new_compartment); }}\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Go => {
                        // Go: Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}s.__compartment.exitArgs[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!(
                            "{}__compartment := new{}Compartment(\"{}\")\n",
                            indent_str, ctx.system_name, target
                        ));
                        code.push_str(&format!(
                            "{}__compartment.parentCompartment = s.__compartment.copy()\n",
                            indent_str
                        ));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!(
                                            "{}__compartment.stateArgs[\"{}\"] = {}\n",
                                            indent_str, name, value
                                        ));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!(
                                            "{}__compartment.stateArgs[\"{}\"] = {}\n",
                                            indent_str, key, a
                                        ));
                                    }
                                }
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.enterArgs[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!(
                            "{}s.__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Php => {
                        let mut code = String::new();

                        // Store exit_args in current compartment (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}$this->__compartment->exit_args[\"{}\"] = {};\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        code.push_str(&format!("{}$__compartment = new {}Compartment(\"{}\", $this->__compartment->copy());\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| {
                                        if let Some(eq_pos) = a.find('=') {
                                            let name = a[..eq_pos].trim();
                                            let value = a[eq_pos + 1..].trim();
                                            format!("\"{}\" => {}", name, value)
                                        } else {
                                            let key = resolve_state_arg_key(i, &target, ctx);
                                            format!("\"{}\" => {}", key, a)
                                        }
                                    })
                                    .collect();
                                code.push_str(&format!(
                                    "{}$__compartment->state_args = [{}];\n",
                                    indent_str,
                                    entries.join(", ")
                                ));
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}$__compartment->enter_args[\"{}\"] = {};\n",
                                    indent_str, key, arg
                                ));
                            }
                        }

                        code.push_str(&format!(
                            "{}$this->__transition($__compartment);\n{}return;",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Ruby => {
                        let mut code = String::new();
                        if let Some(ref exit) = exit_str {
                            for (i, arg) in exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .enumerate()
                            {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}@__compartment.exit_args[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }
                        code.push_str(&format!(
                            "{}__compartment = {}Compartment.new(\"{}\", @__compartment.copy)\n",
                            indent_str, ctx.system_name, target
                        ));
                        if let Some(ref state) = state_str {
                            for (i, a) in state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .enumerate()
                            {
                                if let Some(eq_pos) = a.find('=') {
                                    code.push_str(&format!(
                                        "{}__compartment.state_args[\"{}\"] = {}\n",
                                        indent_str,
                                        a[..eq_pos].trim(),
                                        a[eq_pos + 1..].trim()
                                    ));
                                } else {
                                    let key = resolve_state_arg_key(i, &target, ctx);
                                    code.push_str(&format!(
                                        "{}__compartment.state_args[\"{}\"] = {}\n",
                                        indent_str, key, a
                                    ));
                                }
                            }
                        }
                        if let Some(ref enter) = enter_str {
                            for (i, arg) in enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .enumerate()
                            {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.enter_args[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }
                        code.push_str(&format!(
                            "{}__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Lua => {
                        let mut code = String::new();
                        if let Some(ref exit) = exit_str {
                            for (i, arg) in exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .enumerate()
                            {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!(
                                    "{}self.__compartment.exit_args[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }
                        code.push_str(&format!(
                            "{}local __compartment = {}.new(\"{}\")\n",
                            indent_str,
                            format!("{}Compartment", ctx.system_name),
                            target
                        ));
                        if let Some(ref state) = state_str {
                            for (i, a) in state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .enumerate()
                            {
                                if let Some(eq_pos) = a.find('=') {
                                    code.push_str(&format!(
                                        "{}__compartment.state_args[\"{}\"] = {}\n",
                                        indent_str,
                                        a[..eq_pos].trim(),
                                        a[eq_pos + 1..].trim()
                                    ));
                                } else {
                                    let key = resolve_state_arg_key(i, &target, ctx);
                                    code.push_str(&format!(
                                        "{}__compartment.state_args[\"{}\"] = {}\n",
                                        indent_str, key, a
                                    ));
                                }
                            }
                        }
                        if let Some(ref enter) = enter_str {
                            for (i, arg) in enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .enumerate()
                            {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!(
                                    "{}__compartment.enter_args[\"{}\"] = {}\n",
                                    indent_str, key, arg
                                ));
                            }
                        }
                        code.push_str(&format!(
                            "{}self:__transition(__compartment)\n{}return",
                            indent_str, indent_str
                        ));
                        code
                    }
                    TargetLanguage::Erlang => {
                        // Path D: use __frame_transition for full lifecycle
                        let erlang_state = to_snake_case(&target);
                        let mut code = String::new();

                        // Build exit_args map
                        let exit_map = if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            let entries: Vec<String> = args
                                .iter()
                                .enumerate()
                                .map(|(i, a)| format!("<<\"{}\">> => {}", i, a))
                                .collect();
                            format!("#{{{}}}", entries.join(", "))
                        } else {
                            "#{}".to_string()
                        };

                        // Build enter_args map
                        let enter_map = if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            let entries: Vec<String> = args
                                .iter()
                                .enumerate()
                                .map(|(i, a)| format!("<<\"{}\">> => {}", i, a))
                                .collect();
                            format!("#{{{}}}", entries.join(", "))
                        } else {
                            "#{}".to_string()
                        };

                        // Build state_args map
                        let state_map = if let Some(ref state) = state_str {
                            let args: Vec<&str> = state
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .collect();
                            let entries: Vec<String> = args
                                .iter()
                                .enumerate()
                                .map(|(i, a)| {
                                    if let Some(eq_pos) = a.find('=') {
                                        format!(
                                            "<<\"{}\">> => {}",
                                            a[..eq_pos].trim(),
                                            a[eq_pos + 1..].trim()
                                        )
                                    } else {
                                        format!("<<\"{}\">> => {}", i, a)
                                    }
                                })
                                .collect();
                            format!("#{{{}}}", entries.join(", "))
                        } else {
                            "#{}".to_string()
                        };

                        code.push_str(&format!(
                            "{}frame_transition__({}, Data, {}, {}, {}, From)",
                            indent_str, erlang_state, exit_map, enter_map, state_map
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
                        format!("{}self._state_{}(__e)", indent_str, parent)
                    }
                    TargetLanguage::TypeScript
                    | TargetLanguage::Dart
                    | TargetLanguage::JavaScript => {
                        format!("{}this._state_{}(__e);", indent_str, parent)
                    }
                    // Rust: call parent state router (not specific handler) to dispatch via match
                    TargetLanguage::Rust => {
                        super::rust_system::rust_parent_forward(&indent_str, parent)
                    }
                    // C: call System_state_Parent(self, __e) since C has no methods
                    TargetLanguage::C => format!(
                        "{}{}_state_{}(self, __e);",
                        indent_str, ctx.system_name, parent
                    ),
                    // C++: call _state_Parent(__e) — forward is not terminal, no return
                    TargetLanguage::Cpp => format!("{}_state_{}(__e);", indent_str, parent),
                    // Java/C#: call _state_Parent(__e) — forward is not terminal, no return
                    TargetLanguage::Java | TargetLanguage::CSharp => {
                        format!("{}_state_{}(__e);", indent_str, parent)
                    }
                    TargetLanguage::Kotlin | TargetLanguage::Swift => {
                        format!("{}_state_{}(__e)", indent_str, parent)
                    }
                    // Go: call s._state_Parent(__e) — forward is not terminal, no return
                    TargetLanguage::Go => format!("{}s._state_{}(__e)", indent_str, parent),
                    TargetLanguage::Php => format!("{}$this->_state_{}($__e);", indent_str, parent),
                    TargetLanguage::Ruby => format!("{}_state_{}(__e)", indent_str, parent),
                    TargetLanguage::Lua => format!("{}self:_state_{}(__e)", indent_str, parent),
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
            // Extract variable name from "$.varName"
            let var_name = if let SegmentMetadata::StateVar { name } = metadata {
                name.clone()
            } else {
                extract_state_var_name(&segment_text) // fallback
            };
            // State variables are stored in compartment.state_vars
            // For HSM: use __sv_comp if available (navigates to correct compartment for parent states)
            match lang {
                TargetLanguage::Python3 => {
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]", var_name)
                    } else {
                        format!("self.__compartment.state_vars[\"{}\"]", var_name)
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]", var_name)
                    } else {
                        format!("this.__compartment.state_vars[\"{}\"]", var_name)
                    }
                }
                TargetLanguage::Php => {
                    if ctx.use_sv_comp {
                        format!("$__sv_comp->state_vars[\"{}\"]", var_name)
                    } else {
                        format!("$this->__compartment->state_vars[\"{}\"]", var_name)
                    }
                }
                TargetLanguage::Ruby => {
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]", var_name)
                    } else {
                        format!("@__compartment.state_vars[\"{}\"]", var_name)
                    }
                }
                TargetLanguage::Rust => {
                    super::rust_system::rust_expand_state_var_read(ctx, &var_name)
                }
                TargetLanguage::C => {
                    // For C, access via FrameDict_get with type-aware cast
                    let c_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|s| s.as_str())
                        .unwrap_or("int");
                    let cast = match c_type {
                        "char*" | "const char*" | "str" | "string" | "String" => "(const char*)",
                        _ => "(int)(intptr_t)",
                    };
                    format!(
                        "{}{}_FrameDict_get(self->__compartment->state_vars, \"{}\")",
                        cast, ctx.system_name, var_name
                    )
                }
                TargetLanguage::Cpp => {
                    let cpp_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|t| cpp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    if ctx.use_sv_comp {
                        format!(
                            "std::any_cast<{}>(__sv_comp->state_vars[\"{}\"])",
                            cpp_type, var_name
                        )
                    } else {
                        format!(
                            "std::any_cast<{}>(__compartment->state_vars[\"{}\"])",
                            cpp_type, var_name
                        )
                    }
                }
                TargetLanguage::Java => {
                    let java_type = ctx
                        .state_var_types
                        .get(var_name.as_str())
                        .map(|t| java_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let accessor = if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]{}", var_name, cast)
                    } else {
                        format!("__compartment.state_vars[\"{}\"]{}", var_name, cast)
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
                        if ctx.use_sv_comp {
                            format!("__sv_comp.state_vars[\"{}\"]", var_name)
                        } else {
                            format!("__compartment.state_vars[\"{}\"]", var_name)
                        }
                    } else if ctx.use_sv_comp {
                        format!("(__sv_comp.state_vars[\"{}\"] as! {})", var_name, sw_type)
                    } else {
                        format!(
                            "(__compartment.state_vars[\"{}\"] as! {})",
                            var_name, sw_type
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
                    if ctx.use_sv_comp {
                        format!("__sv_comp.stateVars[\"{}\"]{}", var_name, assertion)
                    } else {
                        format!("s.__compartment.stateVars[\"{}\"]{}", var_name, assertion)
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
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[\"{}\"]", cast, var_name)
                    } else {
                        format!("{}__compartment.state_vars[\"{}\"]", cast, var_name)
                    }
                }
                TargetLanguage::Lua => {
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]", var_name)
                    } else {
                        format!("self.__compartment.state_vars[\"{}\"]", var_name)
                    }
                }
                TargetLanguage::Dart => {
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]", var_name)
                    } else {
                        format!("this.__compartment.state_vars[\"{}\"]", var_name)
                    }
                }
                TargetLanguage::GDScript => {
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]", var_name)
                    } else {
                        format!("self.__compartment.state_vars[\"{}\"]", var_name)
                    }
                }
                TargetLanguage::Erlang => {
                    // State vars stored as sv_StateName_VarName in Data record
                    let state_prefix = to_snake_case(&ctx.state_name);
                    format!("Data#data.sv_{}_{}", state_prefix, var_name)
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
            let var_name = if let SegmentMetadata::StateVar { name } = metadata {
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
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
                        format!(
                            "{}$__sv_comp->state_vars[\"{}\"] = {};",
                            indent_str, var_name, expanded_expr
                        )
                    } else {
                        format!(
                            "{}$this->__compartment->state_vars[\"{}\"] = {};",
                            indent_str, var_name, expanded_expr
                        )
                    }
                }
                TargetLanguage::Ruby => {
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
                        format!("{}{}_FrameDict_set(__sv_comp->state_vars, \"{}\", (void*)(intptr_t)({}));",
                            indent_str, ctx.system_name, var_name, expanded_expr)
                    } else {
                        format!("{}{}_FrameDict_set(self->__compartment->state_vars, \"{}\", (void*)(intptr_t)({}));",
                            indent_str, ctx.system_name, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Cpp => {
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                TargetLanguage::Kotlin | TargetLanguage::Swift => {
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                    // State var assignment: $.var = expr → record update
                    let state_prefix = to_snake_case(&ctx.state_name);
                    let field_name = format!("sv_{}_{}", state_prefix, var_name);
                    // Rewrite self. references in the expression
                    let erl_expr = expanded_expr.replace("self.", "Data#data.");
                    format!("{}self.{} = {}", indent_str, field_name, erl_expr)
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
                        "{}{}_CTX(self)->_return = (void*)(intptr_t)({});",
                        indent_str, ctx.system_name, expanded_expr
                    ),
                    TargetLanguage::Rust => {
                        super::rust_system::rust_expand_box_return(&indent_str, &expanded_expr)
                    }
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
                    TargetLanguage::Go => format!(
                        "{}s._context_stack[len(s._context_stack)-1]._return = {}",
                        indent_str, expanded_expr
                    ),
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
                        let erl_expr = expanded_expr.replace("self.", "Data#data.");
                        format!("{}__ReturnVal = {}", indent_str, erl_expr)
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            } else {
                // Read: @@:return
                match lang {
                    TargetLanguage::Python3 | TargetLanguage::GDScript => {
                        "self._context_stack[-1]._return".to_string()
                    }
                    TargetLanguage::TypeScript
                    | TargetLanguage::Dart
                    | TargetLanguage::JavaScript => {
                        "this._context_stack[this._context_stack.length - 1]._return".to_string()
                    }
                    TargetLanguage::C => format!("{}_RETURN(self)", ctx.system_name),
                    TargetLanguage::Rust => super::rust_system::rust_context_return_read(),
                    TargetLanguage::Cpp => {
                        "std::any_cast<std::string>(_context_stack.back()._return)".to_string()
                    }
                    TargetLanguage::Java => {
                        "_context_stack.get(_context_stack.size() - 1)._return".to_string()
                    }
                    TargetLanguage::Kotlin => {
                        "_context_stack[_context_stack.size - 1]._return".to_string()
                    }
                    TargetLanguage::Swift => {
                        "_context_stack[_context_stack.count - 1]._return".to_string()
                    }
                    TargetLanguage::CSharp => {
                        "_context_stack[_context_stack.Count - 1]._return".to_string()
                    }
                    TargetLanguage::Go => {
                        "s._context_stack[len(s._context_stack)-1]._return".to_string()
                    }
                    TargetLanguage::Php => {
                        "$this->_context_stack[count($this->_context_stack) - 1]->_return"
                            .to_string()
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
                TargetLanguage::C => format!(
                    "{}_CTX(self)->_return = (void*)(intptr_t)({});",
                    ctx.system_name, expanded_expr
                ),
                TargetLanguage::Rust => {
                    super::rust_system::rust_expand_box_return_bare(&indent_str, &expanded_expr)
                }
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
                TargetLanguage::Go => format!(
                    "s._context_stack[len(s._context_stack)-1]._return = {}",
                    expanded_expr
                ),
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
                    let erl_expr = expanded_expr.replace("self.", "Data#data.");
                    format!("__ReturnVal = {}", erl_expr)
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
            // @@:params.key - dot-accessor for interface parameter
            let key = if let SegmentMetadata::ContextParams { key } = metadata {
                key.clone()
            } else {
                extract_dot_key(&segment_text, "@@:params") // fallback
            };
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => format!("self._context_stack[-1].event._parameters[\"{}\"]", key),
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!("this._context_stack[this._context_stack.length - 1].event._parameters[\"{}\"]", key),
                TargetLanguage::Dart => format!("this._context_stack[this._context_stack.length - 1].event._parameters![\"{}\"]", key),
                TargetLanguage::C => key.to_string(),
                TargetLanguage::Rust => super::rust_system::rust_context_param(&key),
                TargetLanguage::Cpp => key.to_string(),
                TargetLanguage::Java => format!("_context_stack.get(_context_stack.size() - 1)._event._parameters.get(\"{}\")", key),
                TargetLanguage::Kotlin => format!("_context_stack[_context_stack.size - 1]._event._parameters[\"{}\"]", key),
                TargetLanguage::Swift => format!("_context_stack[_context_stack.count - 1]._event._parameters[\"{}\"]", key),
                TargetLanguage::CSharp => format!("_context_stack[_context_stack.Count - 1]._event._parameters[\"{}\"]", key),
                TargetLanguage::Go => format!("s._context_stack[len(s._context_stack)-1]._event._parameters[\"{}\"]", key),
                TargetLanguage::Php => format!("$this->_context_stack[count($this->_context_stack) - 1]->_event->_parameters[\"{}\"]", key),
                TargetLanguage::Ruby => format!("@_context_stack[@_context_stack.length - 1]._event._parameters[\"{}\"]", key),
                TargetLanguage::Lua => format!("self._context_stack[#self._context_stack]._event._parameters[\"{}\"]", key),
                TargetLanguage::Erlang => "undefined".to_string(), // params accessed as variables directly
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::TaggedInstantiation => {
            // @@SystemName(args) - validated system instantiation
            // Strip @@ prefix and validate system name exists
            let native_call = segment_text.strip_prefix("@@").unwrap_or(&segment_text);

            // Extract system name (before the parenthesis)
            let tagged_system_name = if let Some(paren_pos) = native_call.find('(') {
                &native_call[..paren_pos]
            } else {
                native_call
            };

            // Validate that the system name exists in defined_systems
            if !ctx.defined_systems.contains(tagged_system_name) {
                // System not found - generate an error that will fail compilation
                match lang {
                    TargetLanguage::Python3 => {
                        format!("raise NameError(\"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}. Did you mean one of: {:?}?\")",
                            tagged_system_name, tagged_system_name, ctx.defined_systems)
                    }
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                        format!("throw new Error(\"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}. Did you mean one of: {:?}?\");",
                            tagged_system_name, tagged_system_name, ctx.defined_systems)
                    }
                    TargetLanguage::Rust => {
                        super::rust_system::rust_tagged_instantiation_error(tagged_system_name)
                    }
                    TargetLanguage::C => {
                        format!("#error \"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\"",
                            tagged_system_name, tagged_system_name)
                    }
                    TargetLanguage::Cpp => {
                        format!("static_assert(false, \"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\");",
                            tagged_system_name, tagged_system_name)
                    }
                    TargetLanguage::Java | TargetLanguage::CSharp => {
                        format!("throw new RuntimeException(\"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\");",
                            tagged_system_name, tagged_system_name)
                    }
                    TargetLanguage::Kotlin => {
                        format!("throw RuntimeException(\"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\")",
                            tagged_system_name, tagged_system_name)
                    }
                    TargetLanguage::Swift => {
                        format!("fatalError(\"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\")",
                            tagged_system_name, tagged_system_name)
                    }
                    TargetLanguage::Go => {
                        format!("panic(\"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\")",
                            tagged_system_name, tagged_system_name)
                    }
                    TargetLanguage::Ruby => {
                        format!("raise \"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\"",
                            tagged_system_name, tagged_system_name)
                    }
                    _ => {
                        format!("/* Frame Error E421: Undefined system '{}' in tagged instantiation @@{} */",
                            tagged_system_name, tagged_system_name)
                    }
                }
            } else {
                // System found - generate valid constructor call
                match lang {
                    TargetLanguage::C => {
                        // C: @@System() becomes System_new()
                        if let Some(paren_pos) = native_call.find('(') {
                            let args = &native_call[paren_pos..];
                            format!("{}_new{}", tagged_system_name, args)
                        } else {
                            native_call.to_string()
                        }
                    }
                    TargetLanguage::Cpp => {
                        // C++: @@System() becomes System() (stack allocation)
                        native_call.to_string()
                    }
                    TargetLanguage::Java | TargetLanguage::CSharp | TargetLanguage::Php => {
                        // Java/C#/PHP: @@System() becomes new System()
                        format!("new {}", native_call)
                    }
                    TargetLanguage::Kotlin | TargetLanguage::Swift => {
                        // Kotlin/Swift: @@System() becomes System() (no new keyword)
                        native_call.to_string()
                    }
                    TargetLanguage::Go => {
                        // Go: @@System() becomes NewSystem()
                        if let Some(paren_pos) = native_call.find('(') {
                            let args = &native_call[paren_pos..];
                            format!("New{}{}", tagged_system_name, args)
                        } else {
                            format!("New{}", native_call)
                        }
                    }
                    TargetLanguage::JavaScript => {
                        // JavaScript: @@System() becomes new System()
                        format!("new {}", native_call)
                    }
                    TargetLanguage::Ruby => {
                        // Ruby: @@System() becomes System.new()
                        if let Some(paren_pos) = native_call.find('(') {
                            let args = &native_call[paren_pos..];
                            format!("{}.new{}", tagged_system_name, args)
                        } else {
                            format!("{}.new", native_call)
                        }
                    }
                    _ => {
                        // Python/TypeScript/Rust: @@System() becomes System()
                        native_call.to_string()
                    }
                }
            }
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
                TargetLanguage::C => format!(
                    "{}_CTX(self)->_return = (void*)(intptr_t)({});",
                    ctx.system_name, expanded_expr
                ),
                TargetLanguage::Rust => {
                    super::rust_system::rust_expand_box_return_bare(&indent_str, &expanded_expr)
                }
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
                TargetLanguage::Go => format!(
                    "s._context_stack[len(s._context_stack)-1]._return = {}",
                    expanded_expr
                ),
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
                    let erl_expr = expanded_expr.replace("self.", "Data#data.");
                    format!("__ReturnVal = {}", erl_expr)
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
            let (method_name, args_with_parens) =
                if let SegmentMetadata::SelfCall { method, args } = metadata {
                    (method.as_str(), args.as_str())
                } else {
                    let after_self = trimmed.strip_prefix("@@:self.").unwrap_or(trimmed);
                    let paren_pos = after_self.find('(').unwrap_or(after_self.len());
                    (&after_self[..paren_pos], &after_self[paren_pos..])
                };

            // Generate the native self-call
            let call_expr = match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    format!("self.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
                    format!("this.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Rust | TargetLanguage::Swift => {
                    format!("self.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Cpp => format!("this->{}{}", method_name, args_with_parens),
                TargetLanguage::C => {
                    if args_with_parens == "()" {
                        format!("{}_{}(self)", ctx.system_name, method_name)
                    } else {
                        let inner_args = &args_with_parens[1..args_with_parens.len() - 1];
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
                TargetLanguage::Erlang => String::new(),
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
                    if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("self.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                    let accessor = if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                        if ctx.use_sv_comp {
                            result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                        } else {
                            result.push_str(&format!("__compartment.state_vars[\"{}\"]", var_name))
                        }
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
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
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
                    if ctx.use_sv_comp {
                        result.push_str(&format!("$__sv_comp->state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!(
                            "$this->__compartment->state_vars[\"{}\"]",
                            var_name
                        ))
                    }
                }
                TargetLanguage::Ruby => {
                    if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("@__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::Lua => {
                    if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("self.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::Dart => {
                    if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("this.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::GDScript => {
                    if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("self.__compartment.state_vars[\"{}\"]", var_name))
                    }
                }
                TargetLanguage::Erlang => {
                    let state_prefix = to_snake_case(&ctx.state_name);
                    result.push_str(&format!("Data#data.sv_{}_{}", state_prefix, var_name))
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

    // Helper: emit exit_args writes on current compartment
    if let Some(ref exit) = exit_args {
        for (i, arg) in exit
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .enumerate()
        {
            let (key, value) = if let Some(eq_pos) = arg.find('=') {
                (
                    arg[..eq_pos].trim().to_string(),
                    arg[eq_pos + 1..].trim().to_string(),
                )
            } else {
                (resolve_exit_arg_key(i, ctx), arg.to_string())
            };
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    code.push_str(&format!(
                        "{}self.__compartment.exit_args[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
                    code.push_str(&format!(
                        "{}this.__compartment.exit_args[\"{}\"] = {};\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Rust => {
                    code.push_str(&super::rust_system::rust_pop_exit_arg(indent, &key, &value));
                }
                TargetLanguage::C => {
                    code.push_str(&format!("{}{}_FrameDict_set(self->__compartment->exit_args, \"{}\", (void*)(intptr_t)({}));\n", indent, ctx.system_name, key, value));
                }
                TargetLanguage::Cpp => {
                    code.push_str(&format!(
                        "{}__compartment->exit_args[\"{}\"] = std::any({});\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Java => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.put(\"{}\", {});\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Kotlin => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Swift => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::CSharp => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args[\"{}\"] = {};\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Go => {
                    code.push_str(&format!(
                        "{}s.__compartment.exitArgs[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Php => {
                    code.push_str(&format!(
                        "{}$this->__compartment->exit_args[\"{}\"] = {};\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Ruby => {
                    code.push_str(&format!(
                        "{}@__compartment.exit_args[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Lua => {
                    code.push_str(&format!(
                        "{}self.__compartment.exit_args[\"{}\"] = {}\n",
                        indent, key, value
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

    // Fresh enter_args: clear + write (RFC-0008 replace semantics)
    if let Some(ref enter) = enter_args {
        for (i, arg) in enter
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .enumerate()
        {
            let (key, value) = if let Some(eq_pos) = arg.find('=') {
                (
                    arg[..eq_pos].trim().to_string(),
                    arg[eq_pos + 1..].trim().to_string(),
                )
            } else {
                (i.to_string(), arg.to_string())
            };
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    code.push_str(&format!(
                        "{}__saved.enter_args[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
                    code.push_str(&format!(
                        "{}__saved.enter_args[\"{}\"] = {};\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Rust => {
                    code.push_str(&super::rust_system::rust_pop_enter_arg(
                        indent, &key, &value,
                    ));
                }
                TargetLanguage::C => {
                    code.push_str(&format!(
                        "{}{}_FrameDict_set(__saved->enter_args, \"{}\", (void*)(intptr_t)({}));\n",
                        indent, ctx.system_name, key, value
                    ));
                }
                TargetLanguage::Cpp => {
                    code.push_str(&format!(
                        "{}__saved->enter_args[\"{}\"] = std::any({});\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Java => {
                    code.push_str(&format!(
                        "{}__saved.enter_args.put(\"{}\", {});\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Kotlin => {
                    code.push_str(&format!(
                        "{}__saved.enter_args[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Swift => {
                    code.push_str(&format!(
                        "{}__saved.enter_args[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::CSharp => {
                    code.push_str(&format!(
                        "{}__saved.enter_args[\"{}\"] = {};\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Go => {
                    code.push_str(&format!(
                        "{}__saved.enterArgs[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Php => {
                    code.push_str(&format!(
                        "{}$__saved->enter_args[\"{}\"] = {};\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Ruby => {
                    code.push_str(&format!(
                        "{}__saved.enter_args[\"{}\"] = {}\n",
                        indent, key, value
                    ));
                }
                TargetLanguage::Lua => {
                    code.push_str(&format!(
                        "{}__saved.enter_args[\"{}\"] = {}\n",
                        indent, key, value
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
            state_var_types: state_var_types
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            state_param_names: std::collections::HashMap::new(),
            state_enter_param_names: std::collections::HashMap::new(),
            state_exit_param_names: std::collections::HashMap::new(),
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
