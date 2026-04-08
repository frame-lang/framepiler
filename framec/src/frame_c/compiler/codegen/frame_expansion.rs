//! Frame statement expansion and handler body splicing.
//!
//! This module handles the core Frame-to-native-code transformation:
//! - Splicing handler bodies: scanning for Frame statements in native code
//!   and replacing them with target-language expansions
//! - Frame statement expansion: converting -> $State, => $^, push$, pop$,
//!   return sugar, $.var, @@:return, etc. to target language code
//! - Helper functions for extracting transition targets, args, state vars

use crate::frame_c::visitors::TargetLanguage;
use crate::frame_c::compiler::frame_ast::Type;
use crate::frame_c::compiler::splice::Splicer;
use crate::frame_c::compiler::native_region_scanner::{
    NativeRegionScanner, Region, FrameSegmentKind,
    python::NativeRegionScannerPy,
    typescript::NativeRegionScannerTs,
    rust::NativeRegionScannerRust,
    csharp::NativeRegionScannerCs,
    c::NativeRegionScannerC,
    cpp::NativeRegionScannerCpp,
    java::NativeRegionScannerJava,
    go::NativeRegionScannerGo,
    javascript::NativeRegionScannerJs,
    php::NativeRegionScannerPhp,
    kotlin::NativeRegionScannerKotlin,
    swift::NativeRegionScannerSwift,
    ruby::NativeRegionScannerRuby,
    erlang::NativeRegionScannerErlang,
    lua::NativeRegionScannerLua,
    dart::NativeRegionScannerDart,
    gdscript::NativeRegionScannerGDScript,
};
use super::codegen_utils::{
    HandlerContext, expression_to_string, state_var_init_value, to_snake_case,
    cpp_map_type, cpp_wrap_any_arg, java_map_type, kotlin_map_type,
    swift_map_type, csharp_map_type, go_map_type, type_to_cpp_string,
};


/// Resolve the storage key for a positional state-arg in a transition.
///
/// State args used to be stored under the integer index as a string key
/// (`"0"`, `"1"`, ...). The dispatch reader now expects them under the
/// declared param name. When the target state is known and has declared
/// params at this index, return the param name; otherwise fall back to
/// the integer index for backwards compatibility (e.g. transitions to
/// states without declared params).
fn resolve_state_arg_key(i: usize, target_state: &str, ctx: &HandlerContext) -> String {
    ctx.state_param_names
        .get(target_state)
        .and_then(|names| names.get(i))
        .cloned()
        .unwrap_or_else(|| i.to_string())
}

/// Resolve the storage key for a positional enter-arg in a transition.
///
/// Mirror of `resolve_state_arg_key` for enter args. The enter handler
/// dispatch now reads `__e._parameters[name]`, so transition codegen
/// must write `enter_args[name]` instead of the legacy positional
/// integer key. Falls back to the integer index when the target state
/// has no declared enter handler params at this position.
fn resolve_enter_arg_key(i: usize, target_state: &str, ctx: &HandlerContext) -> String {
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
fn resolve_exit_arg_key(i: usize, ctx: &HandlerContext) -> String {
    ctx.state_exit_param_names
        .get(&ctx.state_name)
        .and_then(|names| names.get(i))
        .cloned()
        .unwrap_or_else(|| i.to_string())
}

/// Splice handler body from a span (used by Arcanum-based generation)
pub(crate) fn splice_handler_body_from_span(span: &crate::frame_c::compiler::ast::Span, source: &[u8], lang: TargetLanguage, ctx: &HandlerContext) -> String {
    // Ensure span is within bounds
    if span.start >= source.len() || span.end > source.len() || span.start >= span.end {
        return String::new();
    }

    let body_bytes = &source[span.start..span.end];

    // Find the opening brace
    let open_brace = match body_bytes.iter().position(|&b| b == b'{') {
        Some(pos) => pos,
        None => return String::from_utf8_lossy(body_bytes).trim().to_string(),
    };

    // Scan for Frame segments within the body
    let mut scanner = get_native_scanner(lang);
    let scan_result = match scanner.scan(body_bytes, open_brace) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };

    // Generate expansions for each Frame segment
    let mut expansions = Vec::new();
    for region in &scan_result.regions {
        if let Region::FrameSegment { span, kind, indent } = region {
            let expansion = generate_frame_expansion(body_bytes, span, *kind, *indent, lang, ctx);
            expansions.push(expansion);
        }
    }

    // Use splicer to combine native + generated Frame code
    let splicer = Splicer;
    let spliced = splicer.splice(body_bytes, &scan_result.regions, &expansions);

    if std::env::var("FRAME_DEBUG_SPLICER").is_ok() {
        eprintln!("[splice_handler_body_from_span] Spliced result: {:?}", spliced.text);
    }

    // The splicer produces content WITHOUT the outer braces
    // Normalize indentation: remove common leading whitespace from all lines
    let text = spliced.text.trim_start_matches('\n').trim_end();
    let text = normalize_indentation(text);
    // Strip unreachable code after terminal statements (strict languages)
    // 1. Strip ;; (empty statement after return)
    // 2. Remove lines after a bare "return;" / "return" until end of block
    if matches!(lang, TargetLanguage::Java | TargetLanguage::Kotlin | TargetLanguage::Swift | TargetLanguage::CSharp | TargetLanguage::Go) {
        // Swift/Kotlin/Go don't use semicolons — strip trailing semicolons from lines
        let text = if matches!(lang, TargetLanguage::Swift | TargetLanguage::Kotlin | TargetLanguage::Go) {
            text.lines()
                .map(|line| {
                    let trimmed = line.trim_end();
                    if trimmed.ends_with(';') {
                        // Strip trailing semicolons, but handle ";;" -> nothing
                        let stripped = trimmed.trim_end_matches(';');
                        if stripped.is_empty() && line.trim() == ";" {
                            // Lone semicolon line — remove entirely
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
    let min_indent = lines.iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    // Strip the common indentation from all lines
    lines.iter()
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
pub(crate) fn generate_frame_expansion(body_bytes: &[u8], span: &crate::frame_c::compiler::native_region_scanner::RegionSpan, kind: FrameSegmentKind, indent: usize, lang: TargetLanguage, ctx: &HandlerContext) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    // Use scanner's indent value to match native code indentation
    // This ensures Frame expansions align with surrounding native code
    let indent_str = " ".repeat(indent);

    match kind {
        FrameSegmentKind::Transition => {
            // Parse transition: (exit_args)? -> (enter_args)? $State(state_args)?
            // For Python/TypeScript: Create compartment and call __transition()
            // For Rust: Use simpler _transition() approach

            // Check for pop-transition: -> pop$
            if segment_text.contains("pop$") {
                // Pop-transition: pop state from stack and transition to it
                // Includes return to exit handler (code after -> pop$ is unreachable)
                match lang {
                    TargetLanguage::Python3 => format!(
                        "{}__saved = self._state_stack.pop()\n{}self.__transition(__saved)\n{}return",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::GDScript => format!(
                        "{}var __saved = self._state_stack.pop_back()\n{}self.__transition(__saved)\n{}return",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::TypeScript => format!(
                        "{}const __saved = this._state_stack.pop()!;\n{}this.__transition(__saved);\n{}return;",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Dart => format!(
                        "{}final __saved = this._state_stack.removeLast();\n{}this.__transition(__saved);\n{}return;",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::JavaScript => format!(
                        "{}const __saved = this._state_stack.pop();\n{}this.__transition(__saved);\n{}return;",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Rust => format!(
                        "{}let __popped = self._state_stack.pop().unwrap();\n{}self.__transition(__popped);\n{}return;",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::C => format!(
                        "{}{}_Compartment* __saved = ({}_Compartment*){}_FrameVec_pop(self->_state_stack);\n{}{}_transition(self, __saved);\n{}return;",
                        indent_str, ctx.system_name, ctx.system_name, ctx.system_name, indent_str, ctx.system_name, indent_str
                    ),
                    TargetLanguage::Cpp => format!(
                        "{}auto __saved = std::move(_state_stack.back()); _state_stack.pop_back();\n{}__transition(std::move(__saved));\n{}return;",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Java => format!(
                        "{}var __saved = _state_stack.remove(_state_stack.size() - 1);\n{}__transition(__saved);\n{}return;",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Kotlin => format!(
                        "{}val __saved = _state_stack.removeAt(_state_stack.size - 1)\n{}__transition(__saved)\n{}return",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Swift => format!(
                        "{}let __saved = _state_stack.removeLast()\n{}__transition(__saved)\n{}return",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::CSharp => format!(
                        "{}var __saved = _state_stack[_state_stack.Count - 1]; _state_stack.RemoveAt(_state_stack.Count - 1);\n{}__transition(__saved);\n{}return;",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Go => format!(
                        "{}__saved := s._state_stack[len(s._state_stack)-1]\n{}s._state_stack = s._state_stack[:len(s._state_stack)-1]\n{}s.__transition(__saved)\n{}return",
                        indent_str, indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Php => format!(
                        "{}$__saved = array_pop($this->_state_stack);\n{}$this->__transition($__saved);\n{}return;",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Ruby => format!(
                        "{}__saved = @_state_stack.pop\n{}__transition(__saved)\n{}return",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Lua => format!(
                        "{}local __saved = table.remove(self._state_stack)\n{}self:__transition(__saved)\n{}return",
                        indent_str, indent_str, indent_str
                    ),
                    TargetLanguage::Erlang => {
                        format!("{}[__PoppedState | __RestStack] = Data#data.frame_stack,\n{}{{next_state, __PoppedState, Data#data{{frame_stack = __RestStack}}, [{{reply, From, ok}}]}}",
                            indent_str, indent_str)
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            } else {
                let target = extract_transition_target(&segment_text);
                let (exit_args, enter_args) = extract_transition_args(&segment_text);
                let state_args = extract_state_args(&segment_text);

                // Expand state variable references in arguments
                let exit_str = exit_args.map(|a| expand_state_vars_in_expr(&a, lang, ctx));
                let enter_str = enter_args.map(|a| expand_state_vars_in_expr(&a, lang, ctx));
                let state_str = state_args.map(|a| expand_state_vars_in_expr(&a, lang, ctx));

                // Get compartment class name from system name
                let _compartment_class = format!("{}Compartment", ctx.system_name);

                match lang {
                    TargetLanguage::Python3 => {
                        // Create compartment, set fields, call __transition
                        // Store exit_args in CURRENT compartment before creating new one
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
                                    .map(|(i, a)| {
                                        let key = resolve_exit_arg_key(i, ctx);
                                        format!("\"{}\": {}", key, a)
                                    })
                                    .collect();
                                code.push_str(&format!("{}self.__compartment.exit_args = {{{}}}\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}__compartment = {}Compartment(\"{}\", parent_compartment=self.__compartment.copy())\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
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
                                code.push_str(&format!("{}__compartment.state_args = {{{}}}\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Set enter_args if present (named keys, mirrors state_args)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
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
                                code.push_str(&format!("{}__compartment.enter_args = {{{}}}\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!("{}self.__transition(__compartment)\n{}return", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::GDScript => {
                        // GDScript: .new() constructor, no keyword args, no dict comprehension
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            let entries: Vec<String> = args.iter().enumerate()
                                .map(|(i, a)| {
                                    let key = resolve_exit_arg_key(i, ctx);
                                    format!("\"{}\": {}", key, a)
                                })
                                .collect();
                            code.push_str(&format!("{}self.__compartment.exit_args = {{{}}}\n", indent_str, entries.join(", ")));
                        }

                        // Create new compartment: positional args, .new() constructor
                        code.push_str(&format!("{}var __compartment = {}Compartment.new(\"{}\", self.__compartment.copy())\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
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
                                code.push_str(&format!("{}__compartment.state_args = {{{}}}\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Set enter_args if present (named keys, mirrors state_args)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
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
                                code.push_str(&format!("{}__compartment.enter_args = {{{}}}\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Call __transition and return
                        code.push_str(&format!("{}self.__transition(__compartment)\n{}return", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                        // Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
                                    .map(|(i, a)| {
                                        let key = resolve_exit_arg_key(i, ctx);
                                        format!("\"{}\": {}", key, a)
                                    })
                                    .collect();
                                code.push_str(&format!("{}this.__compartment.exit_args = {{{}}};\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}const __compartment = new {}Compartment(\"{}\", this.__compartment.copy());\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
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
                                code.push_str(&format!("{}__compartment.state_args = {{{}}};\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Set enter_args if present (named keys, mirrors state_args)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
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
                                code.push_str(&format!("{}__compartment.enter_args = {{{}}};\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!("{}this.__transition(__compartment);\n{}return;", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Dart => {
                        // Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}this.__compartment.exit_args[\"{}\"] = {};\n", indent_str, key, arg));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}final __compartment = {}Compartment(\"{}\", this.__compartment.copy());\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
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
                                code.push_str(&format!("{}__compartment.state_args = {{{}}};\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__compartment.enter_args[\"{}\"] = {};\n", indent_str, key, arg));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!("{}this.__transition(__compartment);\n{}return;", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Rust => {
                        // Rust uses compartment-based transition with enter/exit args
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}self.__compartment.exit_args.insert(\"{}\".to_string(), {}.to_string());\n", indent_str, key, arg));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}let mut __compartment = {}Compartment::new(\"{}\");\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!("{}__compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));\n", indent_str));

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__compartment.enter_args.insert(\"{}\".to_string(), {}.to_string());\n", indent_str, key, arg));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!("{}self.__transition(__compartment);\n{}return;", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::C => {
                        // C: Create compartment and call transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}{}_FrameDict_set(self->__compartment->exit_args, \"{}\", (void*)(intptr_t)({}));\n", indent_str, ctx.system_name, key, arg));
                            }
                        }

                        // Create new compartment
                        code.push_str(&format!("{}{}_Compartment* __compartment = {}_Compartment_new(\"{}\");\n", indent_str, ctx.system_name, ctx.system_name, target));
                        if ctx.parent_state.is_some() {
                            code.push_str(&format!("{}__compartment->parent_compartment = {}_Compartment_copy(self->__compartment);\n", indent_str, ctx.system_name));
                        }

                        // Set state_args if present (split by comma for positional args)
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_state_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}{}_FrameDict_set(__compartment->state_args, \"{}\", (void*)(intptr_t)({}));\n", indent_str, ctx.system_name, key, arg));
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}{}_FrameDict_set(__compartment->enter_args, \"{}\", (void*)(intptr_t)({}));\n", indent_str, ctx.system_name, key, arg));
                            }
                        }

                        // Call transition and return to exit the handler
                        code.push_str(&format!("{}{}_transition(self, __compartment);\n{}return;", indent_str, ctx.system_name, indent_str));
                        code
                    }
                    TargetLanguage::Cpp => {
                        // C++: Create unique_ptr compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let wrapped = cpp_wrap_any_arg(arg);
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}__compartment->exit_args[\"{}\"] = std::any({});\n", indent_str, key, wrapped));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}auto __new_compartment = std::make_unique<{}Compartment>(\"{}\");\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!("{}__new_compartment->parent_compartment = __compartment->clone();\n", indent_str));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!("{}__new_compartment->state_args[\"{}\"] = std::any({});\n", indent_str, name, value));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!("{}__new_compartment->state_args[\"{}\"] = std::any({});\n", indent_str, key, a));
                                    }
                                }
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let wrapped = cpp_wrap_any_arg(arg);
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__new_compartment->enter_args[\"{}\"] = std::any({});\n", indent_str, key, wrapped));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!("{}__transition(std::move(__new_compartment));\n{}return;", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Java => {
                        // Java: Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment if present (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}__compartment.exit_args.put(\"{}\", {});\n", indent_str, key, arg));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}var __compartment = new {}Compartment(\"{}\");\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!("{}__compartment.parent_compartment = this.__compartment.copy();\n", indent_str));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!("{}__compartment.state_args.put(\"{}\", {});\n", indent_str, name, value));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!("{}__compartment.state_args.put(\"{}\", {});\n", indent_str, key, a));
                                    }
                                }
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__compartment.enter_args.put(\"{}\", {});\n", indent_str, key, arg));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!("{}__transition(__compartment);\n{}return;", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Kotlin => {
                        // Kotlin: no `new`, no semicolons, [] indexer
                        let mut code = String::new();

                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}__compartment.exit_args[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }

                        code.push_str(&format!("{}val __compartment = {}Compartment(\"{}\")\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!("{}__compartment.parent_compartment = this.__compartment.copy()\n", indent_str));

                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!("{}__compartment.state_args[\"{}\"] = {}\n", indent_str, name, value));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!("{}__compartment.state_args[\"{}\"] = {}\n", indent_str, key, a));
                                    }
                                }
                            }
                        }

                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__compartment.enter_args[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }

                        code.push_str(&format!("{}__transition(__compartment)\n{}return", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Swift => {
                        // Swift: no `new`, no semicolons, `let`, `nil`
                        let mut code = String::new();

                        // Store exit_args on CURRENT compartment (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}self.__compartment.exit_args[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }

                        code.push_str(&format!("{}let __compartment = {}Compartment(state: \"{}\")\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!("{}__compartment.parent_compartment = self.__compartment.copy()\n", indent_str));

                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!("{}__compartment.state_args[\"{}\"] = {}\n", indent_str, name, value));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!("{}__compartment.state_args[\"{}\"] = {}\n", indent_str, key, a));
                                    }
                                }
                            }
                        }

                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__compartment.enter_args[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }

                        code.push_str(&format!("{}__transition(__compartment)\n{}return", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::CSharp => {
                        // C#: Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}__compartment.exit_args[\"{}\"] = {};\n", indent_str, key, arg));
                            }
                        }

                        // Create new compartment — block scope prevents redeclaration in multiple branches
                        code.push_str(&format!("{}{{ var __new_compartment = new {}Compartment(\"{}\");\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!("{}__new_compartment.parent_compartment = __compartment.Copy();\n", indent_str));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!("{}__new_compartment.state_args[\"{}\"] = {};\n", indent_str, name, value));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!("{}__new_compartment.state_args[\"{}\"] = {};\n", indent_str, key, a));
                                    }
                                }
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__new_compartment.enter_args[\"{}\"] = {};\n", indent_str, key, arg));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!("{}__transition(__new_compartment); }}\n{}return;", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Go => {
                        // Go: Create compartment, set fields, call __transition
                        let mut code = String::new();

                        // Store exit_args in current compartment (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}s.__compartment.exitArgs[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }

                        // Create new compartment with parent_compartment for HSM support
                        code.push_str(&format!("{}__compartment := new{}Compartment(\"{}\")\n", indent_str, ctx.system_name, target));
                        code.push_str(&format!("{}__compartment.parentCompartment = s.__compartment.copy()\n", indent_str));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                for (i, a) in args.iter().enumerate() {
                                    if let Some(eq_pos) = a.find('=') {
                                        let name = a[..eq_pos].trim();
                                        let value = a[eq_pos + 1..].trim();
                                        code.push_str(&format!("{}__compartment.stateArgs[\"{}\"] = {}\n", indent_str, name, value));
                                    } else {
                                        let key = resolve_state_arg_key(i, &target, ctx);
                                        code.push_str(&format!("{}__compartment.stateArgs[\"{}\"] = {}\n", indent_str, key, a));
                                    }
                                }
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__compartment.enterArgs[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }

                        // Call __transition and return to exit the handler
                        code.push_str(&format!("{}s.__transition(__compartment)\n{}return", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Php => {
                        let mut code = String::new();

                        // Store exit_args in current compartment (named keys)
                        if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}$this->__compartment->exit_args[\"{}\"] = {};\n", indent_str, key, arg));
                            }
                        }

                        code.push_str(&format!("{}$__compartment = new {}Compartment(\"{}\", $this->__compartment->copy());\n", indent_str, ctx.system_name, target));

                        // Set state_args if present
                        if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            if !args.is_empty() {
                                let entries: Vec<String> = args.iter().enumerate()
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
                                code.push_str(&format!("{}$__compartment->state_args = [{}];\n", indent_str, entries.join(", ")));
                            }
                        }

                        // Set enter_args if present (named keys)
                        if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            for (i, arg) in args.iter().enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}$__compartment->enter_args[\"{}\"] = {};\n", indent_str, key, arg));
                            }
                        }

                        code.push_str(&format!("{}$this->__transition($__compartment);\n{}return;", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Ruby => {
                        let mut code = String::new();
                        if let Some(ref exit) = exit_str {
                            for (i, arg) in exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}@__compartment.exit_args[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }
                        code.push_str(&format!("{}__compartment = {}Compartment.new(\"{}\", @__compartment.copy)\n", indent_str, ctx.system_name, target));
                        if let Some(ref state) = state_str {
                            for (i, a) in state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).enumerate() {
                                if let Some(eq_pos) = a.find('=') {
                                    code.push_str(&format!("{}__compartment.state_args[\"{}\"] = {}\n", indent_str, a[..eq_pos].trim(), a[eq_pos+1..].trim()));
                                } else {
                                    let key = resolve_state_arg_key(i, &target, ctx);
                                    code.push_str(&format!("{}__compartment.state_args[\"{}\"] = {}\n", indent_str, key, a));
                                }
                            }
                        }
                        if let Some(ref enter) = enter_str {
                            for (i, arg) in enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__compartment.enter_args[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }
                        code.push_str(&format!("{}__transition(__compartment)\n{}return", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Lua => {
                        // TODO: Lua-specific transition — follows Python pattern
                        let mut code = String::new();
                        if let Some(ref exit) = exit_str {
                            for (i, arg) in exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).enumerate() {
                                let key = resolve_exit_arg_key(i, ctx);
                                code.push_str(&format!("{}self.__compartment.exit_args[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }
                        code.push_str(&format!("{}local __compartment = {}.new(\"{}\")\n", indent_str, format!("{}Compartment", ctx.system_name), target));
                        if let Some(ref state) = state_str {
                            for (i, a) in state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).enumerate() {
                                if let Some(eq_pos) = a.find('=') {
                                    code.push_str(&format!("{}__compartment.state_args[\"{}\"] = {}\n", indent_str, a[..eq_pos].trim(), a[eq_pos+1..].trim()));
                                } else {
                                    let key = resolve_state_arg_key(i, &target, ctx);
                                    code.push_str(&format!("{}__compartment.state_args[\"{}\"] = {}\n", indent_str, key, a));
                                }
                            }
                        }
                        if let Some(ref enter) = enter_str {
                            for (i, arg) in enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).enumerate() {
                                let key = resolve_enter_arg_key(i, &target, ctx);
                                code.push_str(&format!("{}__compartment.enter_args[\"{}\"] = {}\n", indent_str, key, arg));
                            }
                        }
                        code.push_str(&format!("{}self:__transition(__compartment)\n{}return", indent_str, indent_str));
                        code
                    }
                    TargetLanguage::Erlang => {
                        // Path D: use __frame_transition for full lifecycle
                        let erlang_state = to_snake_case(&target);
                        let mut code = String::new();

                        // Build exit_args map
                        let exit_map = if let Some(ref exit) = exit_str {
                            let args: Vec<&str> = exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            let entries: Vec<String> = args.iter().enumerate()
                                .map(|(i, a)| format!("<<\"{}\">> => {}", i, a))
                                .collect();
                            format!("#{{{}}}", entries.join(", "))
                        } else {
                            "#{}".to_string()
                        };

                        // Build enter_args map
                        let enter_map = if let Some(ref enter) = enter_str {
                            let args: Vec<&str> = enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            let entries: Vec<String> = args.iter().enumerate()
                                .map(|(i, a)| format!("<<\"{}\">> => {}", i, a))
                                .collect();
                            format!("#{{{}}}", entries.join(", "))
                        } else {
                            "#{}".to_string()
                        };

                        // Build state_args map
                        let state_map = if let Some(ref state) = state_str {
                            let args: Vec<&str> = state.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                            let entries: Vec<String> = args.iter().enumerate()
                                .map(|(i, a)| {
                                    if let Some(eq_pos) = a.find('=') {
                                        format!("<<\"{}\">> => {}", a[..eq_pos].trim(), a[eq_pos+1..].trim())
                                    } else {
                                        format!("<<\"{}\">> => {}", i, a)
                                    }
                                })
                                .collect();
                            format!("#{{{}}}", entries.join(", "))
                        } else {
                            "#{}".to_string()
                        };

                        code.push_str(&format!("{}frame_transition__({}, Data, {}, {}, {}, From)",
                            indent_str, erlang_state, exit_map, enter_map, state_map));
                        code
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            }
        }
        FrameSegmentKind::TransitionForward => {
            // Transition-Forward: -> => $State
            // 1. Transition to the target state (exit current)
            // 2. Forward current event to new state (instead of sending $>)
            // 3. Return (event was handled by new state)
            let target = extract_transition_target(&segment_text);
            match lang {
                TargetLanguage::Python3 => {
                    // Create compartment with forward_event set to current event
                    let mut code = String::new();
                    code.push_str(&format!("{}__compartment = {}Compartment(\"{}\", parent_compartment=self.__compartment.copy())\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.forward_event = __e\n", indent_str));
                    code.push_str(&format!("{}self.__transition(__compartment)\n", indent_str));
                    code.push_str(&format!("{}return", indent_str));
                    code
                }
                TargetLanguage::GDScript => {
                    // GDScript: .new() constructor, positional args
                    let mut code = String::new();
                    code.push_str(&format!("{}var __compartment = {}Compartment.new(\"{}\", self.__compartment.copy())\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.forward_event = __e\n", indent_str));
                    code.push_str(&format!("{}self.__transition(__compartment)\n", indent_str));
                    code.push_str(&format!("{}return", indent_str));
                    code
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    // Create compartment with forward_event set to current event
                    let mut code = String::new();
                    code.push_str(&format!("{}const __compartment = new {}Compartment(\"{}\", this.__compartment.copy());\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.forward_event = __e;\n", indent_str));
                    code.push_str(&format!("{}this.__transition(__compartment);\n", indent_str));
                    code.push_str(&format!("{}return;", indent_str));
                    code
                }
                TargetLanguage::Dart => {
                    // Create compartment with forward_event set to current event
                    let mut code = String::new();
                    code.push_str(&format!("{}final __compartment = {}Compartment(\"{}\", this.__compartment.copy());\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.forward_event = __e;\n", indent_str));
                    code.push_str(&format!("{}this.__transition(__compartment);\n", indent_str));
                    code.push_str(&format!("{}return;", indent_str));
                    code
                }
                TargetLanguage::Rust => {
                    // Rust uses compartment-based transition with forward event
                    let mut code = String::new();
                    code.push_str(&format!("{}let mut __compartment = {}Compartment::new(\"{}\");\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.forward_event = Some(__e.clone());\n", indent_str));
                    code.push_str(&format!("{}self.__transition(__compartment);\n", indent_str));
                    code.push_str(&format!("{}return;", indent_str));
                    code
                }
                TargetLanguage::C => {
                    // C: Create compartment with forward event and call transition
                    let mut code = String::new();
                    code.push_str(&format!("{}{}_Compartment* __compartment = {}_Compartment_new(\"{}\");\n", indent_str, ctx.system_name, ctx.system_name, target));
                    if ctx.parent_state.is_some() {
                        code.push_str(&format!("{}__compartment->parent_compartment = {}_Compartment_copy(self->__compartment);\n", indent_str, ctx.system_name));
                    }
                    code.push_str(&format!("{}__compartment->forward_event = __e;\n", indent_str));
                    code.push_str(&format!("{}{}_transition(self, __compartment);\n", indent_str, ctx.system_name));
                    code.push_str(&format!("{}return;", indent_str));
                    code
                }
                TargetLanguage::Cpp => {
                    // C++: Create unique_ptr compartment with forward event
                    let mut code = String::new();
                    code.push_str(&format!("{}auto __new_compartment = std::make_unique<{}Compartment>(\"{}\");\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__new_compartment->parent_compartment = __compartment->clone();\n", indent_str));
                    code.push_str(&format!("{}__new_compartment->forward_event = std::make_unique<{}FrameEvent>(__e);\n", indent_str, ctx.system_name));
                    code.push_str(&format!("{}__transition(std::move(__new_compartment));\n", indent_str));
                    code.push_str(&format!("{}return;", indent_str));
                    code
                }
                TargetLanguage::Java => {
                    // Java: Create compartment with forward event
                    let mut code = String::new();
                    code.push_str(&format!("{}var __compartment = new {}Compartment(\"{}\");\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.parent_compartment = this.__compartment.copy();\n", indent_str));
                    code.push_str(&format!("{}__compartment.forward_event = __e;\n", indent_str));
                    code.push_str(&format!("{}__transition(__compartment);\n", indent_str));
                    code.push_str(&format!("{}return;", indent_str));
                    code
                }
                TargetLanguage::Kotlin => {
                    let mut code = String::new();
                    code.push_str(&format!("{}val __compartment = {}Compartment(\"{}\")\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.parent_compartment = this.__compartment.copy()\n", indent_str));
                    code.push_str(&format!("{}__compartment.forward_event = __e\n", indent_str));
                    code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                    code.push_str(&format!("{}return", indent_str));
                    code
                }
                TargetLanguage::Swift => {
                    let mut code = String::new();
                    code.push_str(&format!("{}let __compartment = {}Compartment(state: \"{}\")\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.parent_compartment = self.__compartment.copy()\n", indent_str));
                    code.push_str(&format!("{}__compartment.forward_event = __e\n", indent_str));
                    code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                    code.push_str(&format!("{}return", indent_str));
                    code
                }
                TargetLanguage::Php => {
                    let mut code = String::new();
                    code.push_str(&format!("{}$__compartment = new {}Compartment(\"{}\", $this->__compartment->copy());\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}$__compartment->forward_event = $__e;\n", indent_str));
                    code.push_str(&format!("{}$this->__transition($__compartment);\n", indent_str));
                    code.push_str(&format!("{}return;", indent_str));
                    code
                }
                TargetLanguage::CSharp => {
                    // C#: Create compartment with forward event — block scope
                    let mut code = String::new();
                    code.push_str(&format!("{}{{ var __new_compartment = new {}Compartment(\"{}\");\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__new_compartment.parent_compartment = __compartment.Copy();\n", indent_str));
                    code.push_str(&format!("{}__new_compartment.forward_event = __e;\n", indent_str));
                    code.push_str(&format!("{}__transition(__new_compartment); }}\n", indent_str));
                    code.push_str(&format!("{}return;", indent_str));
                    code
                }
                TargetLanguage::Go => {
                    // Go: Create compartment with forward event
                    let mut code = String::new();
                    code.push_str(&format!("{}__compartment := new{}Compartment(\"{}\")\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.parentCompartment = s.__compartment.copy()\n", indent_str));
                    code.push_str(&format!("{}__compartment.forwardEvent = __e\n", indent_str));
                    code.push_str(&format!("{}s.__transition(__compartment)\n", indent_str));
                    code.push_str(&format!("{}return", indent_str));
                    code
                }
                TargetLanguage::Ruby => {
                    let mut code = String::new();
                    code.push_str(&format!("{}__compartment = {}Compartment.new(\"{}\", @__compartment.copy)\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.forward_event = __e\n", indent_str));
                    code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                    code.push_str(&format!("{}return", indent_str));
                    code
                }
                TargetLanguage::Lua => {
                    // TODO: Lua-specific transition-forward
                    let mut code = String::new();
                    code.push_str(&format!("{}local __compartment = {}Compartment.new(\"{}\")\n", indent_str, ctx.system_name, target));
                    code.push_str(&format!("{}__compartment.forward_event = __e\n", indent_str));
                    code.push_str(&format!("{}self:__transition(__compartment)\n", indent_str));
                    code.push_str(&format!("{}return", indent_str));
                    code
                }
                TargetLanguage::Erlang => String::new(), // TODO: Erlang gen_statem codegen
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::Forward => {
            // HSM forward: call parent state's handler for the same event
            if let Some(ref parent) = ctx.parent_state {
                match lang {
                    // Python/TypeScript: call _state_Parent(__e) to dispatch via unified state method
                    TargetLanguage::Python3 | TargetLanguage::GDScript => format!("{}self._state_{}(__e)", indent_str, parent),
                    TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => format!("{}this._state_{}(__e);", indent_str, parent),
                    // Rust: call parent state router (not specific handler) to dispatch via match
                    TargetLanguage::Rust => format!("{}self._state_{}(__e);", indent_str, parent),
                    // C: call System_state_Parent(self, __e) since C has no methods
                    TargetLanguage::C => format!("{}{}_state_{}(self, __e);", indent_str, ctx.system_name, parent),
                    // C++: call _state_Parent(__e) — forward is not terminal, no return
                    TargetLanguage::Cpp => format!("{}_state_{}(__e);", indent_str, parent),
                    // Java/C#: call _state_Parent(__e) — forward is not terminal, no return
                    TargetLanguage::Java | TargetLanguage::CSharp => format!("{}_state_{}(__e);", indent_str, parent),
                    TargetLanguage::Kotlin | TargetLanguage::Swift => format!("{}_state_{}(__e)", indent_str, parent),
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
                        format!("{}{}({{call, From}}, {}, Data)", indent_str, parent_atom, event_atom)
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            } else {
                // No parent state - just return (shouldn't happen in valid HSM)
                match lang {
                    TargetLanguage::Python3 | TargetLanguage::GDScript => format!("{}return  # Forward to parent (no parent)", indent_str),
                    TargetLanguage::Ruby | TargetLanguage::Kotlin | TargetLanguage::Swift
                        | TargetLanguage::Lua => {
                        format!("{}return // Forward to parent (no parent)", indent_str)
                    }
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Rust | TargetLanguage::Dart
                        | TargetLanguage::C | TargetLanguage::Cpp | TargetLanguage::Java
                        | TargetLanguage::CSharp | TargetLanguage::Go | TargetLanguage::Php => {
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
            // Check if this is a push-then-transition: push$ -> $State
            let has_transition = segment_text.contains("->");
            let target = if has_transition {
                extract_transition_target(&segment_text)
            } else {
                String::new()
            };

            match lang {
                // Python/TypeScript/C: push copy, then separate transition
                TargetLanguage::Python3 => {
                    let push_code = format!("{}self._state_stack.append(self.__compartment.copy())", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}self._transition(\"{}\", None, None)", push_code, indent_str, target)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::GDScript => {
                    let push_code = format!("{}self._state_stack.append(self.__compartment.copy())", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}self._transition(\"{}\", null, null)", push_code, indent_str, target)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    let push_code = format!("{}this._state_stack.push(this.__compartment.copy());", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}this._transition(\"{}\", null, null);", push_code, indent_str, target)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Dart => {
                    let push_code = format!("{}this._state_stack.add(this.__compartment.copy());", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}this.__transition({}Compartment(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                // Rust: __push_transition atomically moves compartment to stack and transitions
                TargetLanguage::Rust => {
                    if !target.is_empty() {
                        format!("{}self.__push_transition({}Compartment::new(\"{}\"));\n{}return;",
                            indent_str, ctx.system_name, target, indent_str)
                    } else {
                        // Push without transition (bare push$) — clone state name first to avoid borrow conflict
                        format!("{}{{\n{0}    let __state = self.__compartment.state.clone();\n{0}    self._state_stack.push(std::mem::replace(&mut self.__compartment, {}Compartment::new(&__state)));\n{0}}}",
                            indent_str, ctx.system_name)
                    }
                }
                TargetLanguage::C => {
                    let push_code = format!("{}{}_FrameVec_push(self->_state_stack, {}_Compartment_copy(self->__compartment));",
                        indent_str, ctx.system_name, ctx.system_name);
                    if !target.is_empty() {
                        format!("{}\n{}{}_transition(self, {}_Compartment_new(\"{}\"));",
                            push_code, indent_str, ctx.system_name, ctx.system_name, target)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Cpp => {
                    let push_code = format!("{}_state_stack.push_back(__compartment->clone());", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}__transition(std::make_unique<{}Compartment>(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Java => {
                    let push_code = format!("{}_state_stack.add(__compartment.copy());", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}__transition(new {}Compartment(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Kotlin => {
                    let push_code = format!("{}_state_stack.add(__compartment.copy())", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}__transition({}Compartment(\"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Swift => {
                    let push_code = format!("{}_state_stack.append(__compartment.copy())", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}__transition({}Compartment(state: \"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Go => {
                    let push_code = format!("{}s._state_stack = append(s._state_stack, s.__compartment.copy())", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}s.__transition(new{}Compartment(\"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::CSharp => {
                    let push_code = format!("{}_state_stack.Add(__compartment.Copy());", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}__transition(new {}Compartment(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Php => {
                    let push_code = format!("{}$this->_state_stack[] = $this->__compartment->copy();", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}$this->__transition(new {}Compartment(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Ruby => {
                    let push_code = format!("{}@_state_stack.push(@__compartment.copy)", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}__transition({}Compartment.new(\"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str)
                    } else {
                        push_code
                    }
                }
                TargetLanguage::Lua => {
                    // TODO: Lua-specific push
                    let push_code = format!("{}self._state_stack[#self._state_stack + 1] = self.__compartment:copy()", indent_str);
                    if !target.is_empty() {
                        format!("{}\n{}self:__transition({}Compartment.new(\"{}\"))\n{}return",
                            push_code, indent_str, ctx.system_name, target, indent_str)
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
                        format!("{}self.frame_stack = [{} | self.frame_stack]", indent_str, state_atom)
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
                TargetLanguage::Rust => format!("{}self._state_stack.pop();", indent_str),
                TargetLanguage::C => format!("{}{}_FrameVec_pop(self->_state_stack);", indent_str, ctx.system_name),
                TargetLanguage::Cpp => format!("{}_state_stack.pop_back();", indent_str),
                TargetLanguage::Java => format!("{}_state_stack.remove(_state_stack.size() - 1);", indent_str),
                TargetLanguage::Kotlin => format!("{}_state_stack.removeAt(_state_stack.size - 1)", indent_str),
                TargetLanguage::Swift => format!("{}_state_stack.removeLast()", indent_str),
                TargetLanguage::CSharp => format!("{}_state_stack.RemoveAt(_state_stack.Count - 1);", indent_str),
                TargetLanguage::Go => format!("{}s._state_stack = s._state_stack[:len(s._state_stack)-1]", indent_str),
                TargetLanguage::Php => format!("{}array_pop($this->_state_stack);", indent_str),
                TargetLanguage::Ruby => format!("{}@_state_stack.pop", indent_str),
                TargetLanguage::Lua => format!("{}table.remove(self._state_stack)", indent_str),
                TargetLanguage::Erlang => {
                    format!("{}[_ | __RestStack] = self.frame_stack,\n{}self.frame_stack = __RestStack",
                        indent_str, indent_str)
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::StateVar => {
            // Extract variable name from "$.varName"
            let var_name = extract_state_var_name(&segment_text);
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
                    // Access state var via compartment chain navigation + state_context matching
                    // Navigation handles HSM: walks parent_compartment chain to find correct state
                    format!("{{ let mut __sv_comp = &self.__compartment; while __sv_comp.state != \"{}\" {{ __sv_comp = __sv_comp.parent_compartment.as_ref().unwrap(); }} match &__sv_comp.state_context {{ {}StateContext::{}(ctx) => ctx.{}, _ => unreachable!() }} }}",
                        ctx.state_name, ctx.system_name, ctx.state_name, var_name)
                },
                TargetLanguage::C => {
                    // For C, access via FrameDict_get with cast
                    // Note: This is for reads; writes are handled by detecting assignment context
                    format!("(int)(intptr_t){}_FrameDict_get(self->__compartment->state_vars, \"{}\")", ctx.system_name, var_name)
                },
                TargetLanguage::Cpp => {
                    let cpp_type = ctx.state_var_types.get(var_name.as_str())
                        .map(|t| cpp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    if ctx.use_sv_comp {
                        format!("std::any_cast<{}>(__sv_comp->state_vars[\"{}\"])", cpp_type, var_name)
                    } else {
                        format!("std::any_cast<{}>(__compartment->state_vars[\"{}\"])", cpp_type, var_name)
                    }
                },
                TargetLanguage::Java => {
                    let java_type = ctx.state_var_types.get(var_name.as_str())
                        .map(|t| java_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let cast = if java_type == "Object" { String::new() } else { format!("({}) ", java_type) };
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars.get(\"{}\")", cast, var_name)
                    } else {
                        format!("{}__compartment.state_vars.get(\"{}\")", cast, var_name)
                    }
                },
                TargetLanguage::Kotlin => {
                    let kt_type = ctx.state_var_types.get(var_name.as_str())
                        .map(|t| kotlin_map_type(t))
                        .unwrap_or_else(|| "Int".to_string());
                    let cast = if kt_type == "Any?" { String::new() } else { format!(" as {}", kt_type) };
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]{}",  var_name, cast)
                    } else {
                        format!("__compartment.state_vars[\"{}\"]{}",  var_name, cast)
                    }
                },
                TargetLanguage::Swift => {
                    let sw_type = ctx.state_var_types.get(var_name.as_str())
                        .map(|t| swift_map_type(t))
                        .unwrap_or_else(|| "Int".to_string());
                    let cast = if sw_type == "Any" { String::new() } else { format!(" as! {}", sw_type) };
                    if ctx.use_sv_comp {
                        format!("__sv_comp.state_vars[\"{}\"]{}",  var_name, cast)
                    } else {
                        format!("__compartment.state_vars[\"{}\"]{}",  var_name, cast)
                    }
                },
                TargetLanguage::Go => {
                    let go_type = ctx.state_var_types.get(var_name.as_str())
                        .map(|t| go_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let assertion = if go_type == "any" || go_type.is_empty() { String::new() } else { format!(".({})", go_type) };
                    if ctx.use_sv_comp {
                        format!("__sv_comp.stateVars[\"{}\"]{}", var_name, assertion)
                    } else {
                        format!("s.__compartment.stateVars[\"{}\"]{}", var_name, assertion)
                    }
                },
                TargetLanguage::CSharp => {
                    let cs_type = ctx.state_var_types.get(var_name.as_str())
                        .map(|t| csharp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let cast = if cs_type == "object" { String::new() } else { format!("({}) ", cs_type) };
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[\"{}\"]", cast, var_name)
                    } else {
                        format!("{}__compartment.state_vars[\"{}\"]", cast, var_name)
                    }
                },
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
            let var_name = if text.starts_with("$.") {
                let rest = &text[2..];
                let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
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
            let expanded_expr = expand_state_vars_in_expr(expr, lang, ctx);

            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}self.__compartment.state_vars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[\"{}\"] = {};", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}this.__compartment.state_vars[\"{}\"] = {};", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Php => {
                    if ctx.use_sv_comp {
                        format!("{}$__sv_comp->state_vars[\"{}\"] = {};", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}$this->__compartment->state_vars[\"{}\"] = {};", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Ruby => {
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}@__compartment.state_vars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Rust => {
                    // Evaluate RHS first (immutable borrow) to avoid borrow conflict with mutable write.
                    // Navigation handles HSM: walks parent_compartment chain to find correct state.
                    format!(concat!(
                        "{}{{\n",
                        "{0}    let __rhs = {};\n",
                        "{0}    let mut __sv_comp: *mut {}Compartment = &mut self.__compartment;\n",
                        "{0}    unsafe {{ while (*__sv_comp).state != \"{}\" {{ __sv_comp = (*__sv_comp).parent_compartment.as_mut().unwrap().as_mut(); }} }}\n",
                        "{0}    unsafe {{ if let {}StateContext::{}(ref mut ctx) = (*__sv_comp).state_context {{ ctx.{} = __rhs; }} }}\n",
                        "{0}}}"
                    ),
                        indent_str, expanded_expr,
                        ctx.system_name, ctx.state_name,
                        ctx.system_name, ctx.state_name, var_name)
                },
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
                        format!("{}__sv_comp->state_vars[\"{}\"] = std::any({});", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}__compartment->state_vars[\"{}\"] = std::any({});", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Java => {
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars.put(\"{}\", {});", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}__compartment.state_vars.put(\"{}\", {});", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Kotlin | TargetLanguage::Swift => {
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}__compartment.state_vars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Go => {
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.stateVars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}s.__compartment.stateVars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::CSharp => {
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[\"{}\"] = {};", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}__compartment.state_vars[\"{}\"] = {};", indent_str, var_name, expanded_expr)
                    }
                }
                TargetLanguage::Lua => {
                    if ctx.use_sv_comp {
                        format!("{}__sv_comp.state_vars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
                    } else {
                        format!("{}self.__compartment.state_vars[\"{}\"] = {}", indent_str, var_name, expanded_expr)
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
            // Check if this is assignment (@@:return = expr) or read (@@:return)
            let trimmed = segment_text.trim();
            if let Some(eq_pos) = trimmed.find('=') {
                // Check it's not ==
                if eq_pos + 1 < trimmed.len() && trimmed.as_bytes().get(eq_pos + 1) != Some(&b'=') {
                    // Assignment: @@:return = expr
                    let expr = trimmed[eq_pos + 1..].trim().trim_end_matches(';').trim();
                    let expanded_expr = expand_state_vars_in_expr(expr, lang, ctx);
                    match lang {
                        TargetLanguage::Python3 | TargetLanguage::GDScript => format!("{}self._context_stack[-1]._return = {}", indent_str, expanded_expr),
                        TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => format!("{}this._context_stack[this._context_stack.length - 1]._return = {};", indent_str, expanded_expr),
                        TargetLanguage::C => format!("{}{}_CTX(self)->_return = (void*)(intptr_t)({});", indent_str, ctx.system_name, expanded_expr),
                        TargetLanguage::Rust => {
                            // For Rust, evaluate expression first to avoid borrow conflicts
                            // (expression may read from context_stack while we need mutable access to set _return)
                            format!("{}let __return_val = Box::new({}) as Box<dyn std::any::Any>;\n{}if let Some(ctx) = self._context_stack.last_mut() {{ ctx._return = Some(__return_val); }}", indent_str, expanded_expr, indent_str)
                        }
                        TargetLanguage::Cpp => format!("{}_context_stack.back()._return = std::any({});", indent_str, expanded_expr),
                        TargetLanguage::Java => format!("{}_context_stack.get(_context_stack.size() - 1)._return = {};", indent_str, expanded_expr),
                        TargetLanguage::Kotlin => format!("{}_context_stack[_context_stack.size - 1]._return = {}", indent_str, expanded_expr),
                        TargetLanguage::Swift => format!("{}_context_stack[_context_stack.count - 1]._return = {}", indent_str, expanded_expr),
                        TargetLanguage::CSharp => format!("{}_context_stack[_context_stack.Count - 1]._return = {};", indent_str, expanded_expr),
                        TargetLanguage::Go => format!("{}s._context_stack[len(s._context_stack)-1]._return = {}", indent_str, expanded_expr),
                        TargetLanguage::Php => format!("{}$this->_context_stack[count($this->_context_stack) - 1]->_return = {};", indent_str, expanded_expr),
                        TargetLanguage::Ruby => format!("{}@_context_stack[@_context_stack.length - 1]._return = {}", indent_str, expanded_expr),
                        TargetLanguage::Lua => format!("{}self._context_stack[#self._context_stack]._return = {}", indent_str, expanded_expr),
                        TargetLanguage::Erlang => {
                            let erl_expr = expanded_expr.replace("self.", "Data#data.");
                            format!("{}__ReturnVal = {}", indent_str, erl_expr)
                        }
                        TargetLanguage::Graphviz => unreachable!(),
                    }
                } else {
                    // Read: @@:return (== check)
                    match lang {
                        TargetLanguage::Python3 | TargetLanguage::GDScript => "self._context_stack[-1]._return".to_string(),
                        TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => "this._context_stack[this._context_stack.length - 1]._return".to_string(),
                        TargetLanguage::C => format!("{}_RETURN(self)", ctx.system_name),
                        TargetLanguage::Rust => "self._context_stack.last().and_then(|ctx| ctx._return.as_ref())".to_string(),
                        TargetLanguage::Cpp => "std::any_cast<std::string>(_context_stack.back()._return)".to_string(),
                        TargetLanguage::Java => "_context_stack.get(_context_stack.size() - 1)._return".to_string(),
                        TargetLanguage::Kotlin => "_context_stack[_context_stack.size - 1]._return".to_string(),
                        TargetLanguage::Swift => "_context_stack[_context_stack.count - 1]._return".to_string(),
                        TargetLanguage::CSharp => "_context_stack[_context_stack.Count - 1]._return".to_string(),
                        TargetLanguage::Go => "s._context_stack[len(s._context_stack)-1]._return".to_string(),
                        TargetLanguage::Php => "$this->_context_stack[count($this->_context_stack) - 1]->_return".to_string(),
                        TargetLanguage::Ruby => "@_context_stack[@_context_stack.length - 1]._return".to_string(),
                        TargetLanguage::Lua => "self._context_stack[#self._context_stack]._return".to_string(),
                        TargetLanguage::Erlang => "__ReturnVal".to_string(),
                        TargetLanguage::Graphviz => unreachable!(),
                    }
                }
            } else {
                // Read: @@:return
                match lang {
                    TargetLanguage::Python3 | TargetLanguage::GDScript => "self._context_stack[-1]._return".to_string(),
                    TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => "this._context_stack[this._context_stack.length - 1]._return".to_string(),
                    TargetLanguage::Rust => "self._context_stack.last().and_then(|ctx| ctx._return.as_ref())".to_string(),
                    TargetLanguage::Cpp => "std::any_cast<std::string>(_context_stack.back()._return)".to_string(),
                    TargetLanguage::Java => "_context_stack.get(_context_stack.size() - 1)._return".to_string(),
                    TargetLanguage::Kotlin => "_context_stack[_context_stack.size - 1]._return".to_string(),
                    TargetLanguage::Swift => "_context_stack[_context_stack.count - 1]._return".to_string(),
                    TargetLanguage::CSharp => "_context_stack[_context_stack.Count - 1]._return".to_string(),
                    TargetLanguage::Php => "$this->_context_stack[count($this->_context_stack) - 1]->_return".to_string(),
                    TargetLanguage::Go => "s._context_stack[len(s._context_stack)-1]._return".to_string(),
                    TargetLanguage::Ruby => "@_context_stack[@_context_stack.length - 1]._return".to_string(),
                    TargetLanguage::C => format!("{}_RETURN(self)", ctx.system_name),
                    TargetLanguage::Lua => "self._context_stack[#self._context_stack]._return".to_string(),
                    TargetLanguage::Erlang => "__ReturnVal".to_string(),
                    TargetLanguage::Graphviz => unreachable!(),
                }
            }
        }
        FrameSegmentKind::ContextReturnExpr => {
            // @@:(expr) - set context return value (concise form)
            // Extract expression from between @@:( and )
            let trimmed = segment_text.trim();
            // Find @@:( and extract everything between ( and final )
            let expr = if let Some(start) = trimmed.find("@@:(") {
                let inner_start = start + 4; // after "@@:("
                let inner = &trimmed[inner_start..];
                // Remove trailing ) — the parser already balanced parens
                if inner.ends_with(')') {
                    &inner[..inner.len() - 1]
                } else {
                    inner
                }
            } else {
                trimmed
            };
            let expanded_expr = expand_state_vars_in_expr(expr.trim(), lang, ctx);
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => format!("{}self._context_stack[-1]._return = {}", indent_str, expanded_expr),
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => format!("{}this._context_stack[this._context_stack.length - 1]._return = {};", indent_str, expanded_expr),
                TargetLanguage::C => format!("{}{}_CTX(self)->_return = (void*)(intptr_t)({});", indent_str, ctx.system_name, expanded_expr),
                TargetLanguage::Rust => {
                    format!("{}let __return_val = Box::new({}) as Box<dyn std::any::Any>;\n{}if let Some(ctx) = self._context_stack.last_mut() {{ ctx._return = Some(__return_val); }}", indent_str, expanded_expr, indent_str)
                }
                TargetLanguage::Cpp => format!("{}_context_stack.back()._return = std::any({});", indent_str, expanded_expr),
                TargetLanguage::Java => format!("{}_context_stack.get(_context_stack.size() - 1)._return = {};", indent_str, expanded_expr),
                TargetLanguage::Kotlin => format!("{}_context_stack[_context_stack.size - 1]._return = {}", indent_str, expanded_expr),
                TargetLanguage::Swift => format!("{}_context_stack[_context_stack.count - 1]._return = {}", indent_str, expanded_expr),
                TargetLanguage::CSharp => format!("{}_context_stack[_context_stack.Count - 1]._return = {};", indent_str, expanded_expr),
                TargetLanguage::Go => format!("{}s._context_stack[len(s._context_stack)-1]._return = {}", indent_str, expanded_expr),
                TargetLanguage::Php => format!("{}$this->_context_stack[count($this->_context_stack) - 1]->_return = {};", indent_str, expanded_expr),
                TargetLanguage::Ruby => format!("{}@_context_stack[@_context_stack.length - 1]._return = {}", indent_str, expanded_expr),
                TargetLanguage::Lua => format!("{}self._context_stack[#self._context_stack]._return = {}", indent_str, expanded_expr),
                TargetLanguage::Erlang => {
                    let erl_expr = expanded_expr.replace("self.", "Data#data.");
                    format!("{}__ReturnVal = {}", indent_str, erl_expr)
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextEvent => {
            // @@:event - interface event name
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => "self._context_stack[-1].event._message".to_string(),
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => "this._context_stack[this._context_stack.length - 1].event._message".to_string(),
                TargetLanguage::C => format!("{}_CTX(self)->event->_message", ctx.system_name),
                // Rust: handlers receive __e as parameter, use it directly to avoid borrow conflicts
                TargetLanguage::Rust => "__e.message.clone()".to_string(),
                TargetLanguage::Cpp => "_context_stack.back()._event._message".to_string(),
                TargetLanguage::Java => "_context_stack.get(_context_stack.size() - 1)._event._message".to_string(),
                TargetLanguage::Kotlin => "_context_stack[_context_stack.size - 1]._event._message".to_string(),
                TargetLanguage::Swift => "_context_stack[_context_stack.count - 1]._event._message".to_string(),
                TargetLanguage::CSharp => "_context_stack[_context_stack.Count - 1]._event._message".to_string(),
                TargetLanguage::Go => "s._context_stack[len(s._context_stack)-1]._event._message".to_string(),
                TargetLanguage::Php => "$this->_context_stack[count($this->_context_stack) - 1]->_event->_message".to_string(),
                TargetLanguage::Ruby => "@_context_stack[@_context_stack.length - 1]._event._message".to_string(),
                TargetLanguage::Lua => "self._context_stack[#self._context_stack]._event._message".to_string(),
                TargetLanguage::Erlang => {
                    let event_atom = to_snake_case(&ctx.event_name);
                    format!("{}", event_atom)
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextData => {
            // @@:data[key] - call-scoped data (read)
            // Extract key from "@@:data[key]" — key includes user quotes (e.g. 'key' or "key")
            let key = extract_bracket_key(&segment_text, "@@:data");
            let bare_key = key.trim_matches('"').trim_matches('\'');
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => format!("self._context_stack[-1]._data[{}]", key),
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => format!("this._context_stack[this._context_stack.length - 1]._data[{}]", key),
                TargetLanguage::C => format!("{}_DATA(self, \"{}\")", ctx.system_name, bare_key),
                TargetLanguage::Rust => {
                    format!("self._context_stack.last().and_then(|ctx| ctx._data.get(\"{}\")).and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default()", bare_key)
                }
                TargetLanguage::Cpp => format!("_context_stack.back()._data[\"{}\"]", bare_key),
                TargetLanguage::Java => format!("_context_stack.get(_context_stack.size() - 1)._data.get(\"{}\")", bare_key),
                TargetLanguage::Kotlin => format!("_context_stack[_context_stack.size - 1]._data[\"{}\"]", bare_key),
                TargetLanguage::Swift => format!("_context_stack[_context_stack.count - 1]._data[\"{}\"]", bare_key),
                TargetLanguage::CSharp => format!("_context_stack[_context_stack.Count - 1]._data[\"{}\"]", bare_key),
                TargetLanguage::Go => format!("s._context_stack[len(s._context_stack)-1]._data[\"{}\"]", bare_key),
                TargetLanguage::Php => format!("$this->_context_stack[count($this->_context_stack) - 1]->_data[\"{}\"]", bare_key),
                TargetLanguage::Ruby => format!("@_context_stack[@_context_stack.length - 1]._data[{}]", key),
                TargetLanguage::Lua => format!("self._context_stack[#self._context_stack]._data[{}]", key),
                TargetLanguage::Erlang => "undefined".to_string(), // gen_statem has no context data
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextDataAssign => {
            // @@:data[key] = expr - call-scoped data (assignment)
            // Extract key and value from "@@:data[key] = expr;"
            let key = extract_bracket_key(&segment_text, "@@:data");
            let bare_key = key.trim_matches('"').trim_matches('\'');
            // Find the = and extract the expression
            let trimmed = segment_text.trim();
            let eq_pos = trimmed.find('=').unwrap_or(trimmed.len());
            let expr = trimmed[eq_pos + 1..].trim().trim_end_matches(';').trim();
            let expanded_expr = expand_state_vars_in_expr(expr, lang, ctx);
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => format!("{}self._context_stack[-1]._data[{}] = {}", indent_str, key, expanded_expr),
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => format!("{}this._context_stack[this._context_stack.length - 1]._data[{}] = {};", indent_str, key, expanded_expr),
                TargetLanguage::C => format!("{}{}_DATA_SET(self, \"{}\", {});", indent_str, ctx.system_name, bare_key, expanded_expr),
                TargetLanguage::Rust => {
                    format!("{}if let Some(ctx) = self._context_stack.last_mut() {{ ctx._data.insert(\"{}\".to_string(), Box::new({}) as Box<dyn std::any::Any>); }}", indent_str, bare_key, expanded_expr)
                }
                TargetLanguage::Cpp => format!("{}_context_stack.back()._data[\"{}\"] = {};", indent_str, bare_key, expanded_expr),
                TargetLanguage::Java => format!("{}_context_stack.get(_context_stack.size() - 1)._data.put(\"{}\", {});", indent_str, bare_key, expanded_expr),
                TargetLanguage::Kotlin => format!("{}_context_stack[_context_stack.size - 1]._data[\"{}\"] = {}", indent_str, bare_key, expanded_expr),
                TargetLanguage::Swift => format!("{}_context_stack[_context_stack.count - 1]._data[\"{}\"] = {}", indent_str, bare_key, expanded_expr),
                TargetLanguage::CSharp => format!("{}_context_stack[_context_stack.Count - 1]._data[\"{}\"] = {};", indent_str, bare_key, expanded_expr),
                TargetLanguage::Go => format!("{}s._context_stack[len(s._context_stack)-1]._data[\"{}\"] = {}", indent_str, bare_key, expanded_expr),
                TargetLanguage::Php => format!("{}$this->_context_stack[count($this->_context_stack) - 1]->_data[\"{}\"] = {};", indent_str, bare_key, expanded_expr),
                TargetLanguage::Ruby => format!("{}@_context_stack[@_context_stack.length - 1]._data[\"{}\"] = {}", indent_str, bare_key, expanded_expr),
                TargetLanguage::Lua => format!("{}self._context_stack[#self._context_stack]._data[\"{}\"] = {}", indent_str, bare_key, expanded_expr),
                TargetLanguage::Erlang => format!("{}ok", indent_str), // gen_statem has no context data
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextParams => {
            // @@:params[key] - explicit parameter access
            // Extract key from "@@:params[key]" — key includes user quotes
            let key = extract_bracket_key(&segment_text, "@@:params");
            let bare_key = key.trim_matches('"').trim_matches('\'');
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => format!("self._context_stack[-1].event._parameters[{}]", key),
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!("this._context_stack[this._context_stack.length - 1].event._parameters[{}]", key),
                TargetLanguage::Dart => format!("this._context_stack[this._context_stack.length - 1].event._parameters![{}]", key),
                TargetLanguage::C => bare_key.to_string(),
                TargetLanguage::Rust => bare_key.to_string(),
                TargetLanguage::Cpp => bare_key.to_string(),
                TargetLanguage::Java => format!("_context_stack.get(_context_stack.size() - 1)._event._parameters.get(\"{}\")", bare_key),
                TargetLanguage::Kotlin => format!("_context_stack[_context_stack.size - 1]._event._parameters[\"{}\"]", bare_key),
                TargetLanguage::Swift => format!("_context_stack[_context_stack.count - 1]._event._parameters[\"{}\"]", bare_key),
                TargetLanguage::CSharp => format!("_context_stack[_context_stack.Count - 1]._event._parameters[\"{}\"]", bare_key),
                TargetLanguage::Go => format!("s._context_stack[len(s._context_stack)-1]._event._parameters[\"{}\"]", bare_key),
                TargetLanguage::Php => format!("$this->_context_stack[count($this->_context_stack) - 1]->_event->_parameters[\"{}\"]", bare_key),
                TargetLanguage::Ruby => format!("@_context_stack[@_context_stack.length - 1]._event._parameters[\"{}\"]", bare_key),
                TargetLanguage::Lua => format!("self._context_stack[#self._context_stack]._event._parameters[\"{}\"]", bare_key),
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
                        format!("compile_error!(\"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\");",
                            tagged_system_name, tagged_system_name)
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
        FrameSegmentKind::ReturnStatement => {
            // Native return keyword detected in handler body.
            // Extract expression after "return" (if any).
            let after_return = segment_text.trim()
                .strip_prefix("return").unwrap_or("")
                .trim().trim_end_matches(';').trim();

            if after_return.is_empty() {
                // Bare `return` — valid, exits the handler. Pass through as native.
                format!("{}return", indent_str)
            } else if after_return.starts_with("@@:") || after_return.starts_with("@@(") {
                // E408: `return @@:<anything>` — combining native return with Frame context
                eprintln!("E408: Cannot combine `return` with Frame context syntax `{}`. \
                    Use `@@:(expr)` to set the return value, then `return` on a separate line.",
                    after_return);
                String::new()
            } else {
                // W415: `return <expr>` in event handler — value is silently lost
                eprintln!("W415: `return {}` in event handler '{}' — the return value is lost. \
                    Use `@@:({})` to set the return value, or bare `return` to exit.",
                    after_return, ctx.event_name, after_return);
                // Pass through as native — it compiles but doesn't do what the user expects
                format!("{}{}", indent_str, segment_text.trim())
            }
        }
    }
}

/// Extract bracketed key from syntax like "@@:data[key]" or "@@:params[key]"
/// Returns the raw content between [ and ] — including any user-supplied quotes.
/// For languages that need a bare key (C, Rust), call .trim_matches on the result.
pub(crate) fn extract_bracket_key(text: &str, prefix: &str) -> String {
    if let Some(rest) = text.strip_prefix(prefix) {
        if let Some(start) = rest.find('[') {
            if let Some(end) = rest.find(']') {
                return rest[start + 1..end].trim().to_string();
            }
        }
    }
    "".to_string()
}

/// Extract transition target from transition text
pub(crate) fn extract_transition_target(text: &str) -> String {
    // Find $StateName after -> in the transition text
    // This handles both "-> $State" and "$$[+] -> $State"
    if let Some(arrow_pos) = text.find("->") {
        let after_arrow = &text[arrow_pos + 2..];
        if let Some(dollar_pos) = after_arrow.find('$') {
            let after_dollar = &after_arrow[dollar_pos + 1..];
            let end = after_dollar.find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(after_dollar.len());
            return after_dollar[..end].to_string();
        }
    }
    // Fallback: find last $ (for simple "-> $State" without prefix)
    if let Some(dollar_pos) = text.rfind('$') {
        let after_dollar = &text[dollar_pos + 1..];
        let end = after_dollar.find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after_dollar.len());
        after_dollar[..end].to_string()
    } else {
        "Unknown".to_string()
    }
}

/// Extract transition arguments (exit_args, enter_args) from transition text
/// Syntax: (exit_args)? -> (enter_args)? $State(state_args)?
pub(crate) fn extract_transition_args(text: &str) -> (Option<String>, Option<String>) {
    let text = text.trim();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    // Skip leading whitespace
    while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }

    // Check for (exit_args) before ->
    let mut exit_args: Option<String> = None;
    if i < n && bytes[i] == b'(' {
        if let Some(close_idx) = find_balanced_paren(bytes, i, n) {
            exit_args = Some(String::from_utf8_lossy(&bytes[i+1..close_idx-1]).to_string());
            i = close_idx;
            while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
        }
    }

    // Skip ->
    if i + 1 < n && bytes[i] == b'-' && bytes[i + 1] == b'>' {
        i += 2;
        while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
    }

    // Check for (enter_args) after ->
    let mut enter_args: Option<String> = None;
    if i < n && bytes[i] == b'(' {
        if let Some(close_idx) = find_balanced_paren(bytes, i, n) {
            enter_args = Some(String::from_utf8_lossy(&bytes[i+1..close_idx-1]).to_string());
        }
    }

    (exit_args, enter_args)
}

/// Extract state_args from transition text: -> $State(state_args)?
/// Returns the comma-separated args string inside the parens after $State
pub(crate) fn extract_state_args(text: &str) -> Option<String> {
    // Find $StateName( pattern
    let bytes = text.as_bytes();
    let n = bytes.len();

    // Find $ after ->
    let arrow_pos = text.find("->")?;
    let after_arrow = &text[arrow_pos + 2..];

    // Find $ for state name
    let dollar_pos = after_arrow.find('$')?;
    let state_start = dollar_pos + 1;

    // Skip the state name (alphanumeric + underscore)
    let mut i = state_start;
    while i < after_arrow.len() {
        let c = after_arrow.as_bytes()[i];
        if c.is_ascii_alphanumeric() || c == b'_' {
            i += 1;
        } else {
            break;
        }
    }

    // Check if there's a ( immediately after state name
    if i < after_arrow.len() && after_arrow.as_bytes()[i] == b'(' {
        let paren_start = arrow_pos + 2 + i;
        if let Some(close_idx) = find_balanced_paren(bytes, paren_start, n) {
            // Return content between ( and )
            return Some(String::from_utf8_lossy(&bytes[paren_start+1..close_idx-1]).to_string());
        }
    }

    None
}

/// Find the closing paren for a balanced paren block, returns index after ')'
pub(crate) fn find_balanced_paren(bytes: &[u8], mut i: usize, end: usize) -> Option<usize> {
    if i >= end || bytes[i] != b'(' { return None; }
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    while i < end {
        let b = bytes[i];
        if let Some(q) = in_str {
            if b == b'\\' { i += 2; continue; }
            if b == q { in_str = None; }
            i += 1; continue;
        }
        match b {
            b'\'' | b'"' => { in_str = Some(b); i += 1; }
            b'(' => { depth += 1; i += 1; }
            b')' => { depth -= 1; i += 1; if depth == 0 { return Some(i); } }
            _ => { i += 1; }
        }
    }
    None
}

/// Extract state variable name from "$.varName"
pub(crate) fn extract_state_var_name(text: &str) -> String {
    // Skip "$." prefix and get identifier
    if text.starts_with("$.") {
        let after_prefix = &text[2..];
        let end = after_prefix.find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after_prefix.len());
        after_prefix[..end].to_string()
    } else {
        "unknown".to_string()
    }
}

/// Expand state variable references ($.varName) and context syntax (@@) in an expression string
/// Uses compartment.state_vars for Python/TypeScript
/// For HSM: uses __sv_comp when ctx.use_sv_comp is true (navigates to correct parent compartment)
pub(crate) fn expand_state_vars_in_expr(expr: &str, lang: TargetLanguage, ctx: &HandlerContext) -> String {
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
                TargetLanguage::Rust => result.push_str(&format!(
                    "{{ let mut __sv_comp = &self.__compartment; while __sv_comp.state != \"{}\" {{ __sv_comp = __sv_comp.parent_compartment.as_ref().unwrap(); }} match &__sv_comp.state_context {{ {}StateContext::{}(ctx) => ctx.{}, _ => unreachable!() }} }}",
                    ctx.state_name, ctx.system_name, ctx.state_name, var_name)),
                TargetLanguage::C => {
                    if ctx.use_sv_comp {
                        result.push_str(&format!("(int)(intptr_t){}_FrameDict_get(__sv_comp->state_vars, \"{}\")", ctx.system_name, var_name))
                    } else {
                        result.push_str(&format!("(int)(intptr_t){}_FrameDict_get(self->__compartment->state_vars, \"{}\")", ctx.system_name, var_name))
                    }
                }
                TargetLanguage::Cpp => {
                    let cpp_type = ctx.state_var_types.get(&var_name)
                        .map(|t| cpp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    if ctx.use_sv_comp {
                        result.push_str(&format!("std::any_cast<{}>(__sv_comp->state_vars[\"{}\"])", cpp_type, var_name))
                    } else {
                        result.push_str(&format!("std::any_cast<{}>(__compartment->state_vars[\"{}\"])", cpp_type, var_name))
                    }
                }
                TargetLanguage::Java => {
                    let java_type = ctx.state_var_types.get(&var_name)
                        .map(|t| java_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let cast = if java_type == "Object" { String::new() } else { format!("({}) ", java_type) };
                    if ctx.use_sv_comp {
                        result.push_str(&format!("{}__sv_comp.state_vars.get(\"{}\")", cast, var_name))
                    } else {
                        result.push_str(&format!("{}__compartment.state_vars.get(\"{}\")", cast, var_name))
                    }
                }
                TargetLanguage::Kotlin => {
                    let kt_type = ctx.state_var_types.get(&var_name)
                        .map(|t| kotlin_map_type(t))
                        .unwrap_or_else(|| "Int".to_string());
                    let cast = if kt_type == "Any?" { String::new() } else { format!(" as {}", kt_type) };
                    if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]{}",  var_name, cast))
                    } else {
                        result.push_str(&format!("__compartment.state_vars[\"{}\"]{}",  var_name, cast))
                    }
                }
                TargetLanguage::Swift => {
                    let sw_type = ctx.state_var_types.get(&var_name)
                        .map(|t| swift_map_type(t))
                        .unwrap_or_else(|| "Int".to_string());
                    let cast = if sw_type == "Any" { String::new() } else { format!(" as! {}", sw_type) };
                    if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.state_vars[\"{}\"]{}",  var_name, cast))
                    } else {
                        result.push_str(&format!("__compartment.state_vars[\"{}\"]{}",  var_name, cast))
                    }
                }
                TargetLanguage::CSharp => {
                    let cs_type = ctx.state_var_types.get(&var_name)
                        .map(|t| csharp_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    let cast = if cs_type == "object" { String::new() } else { format!("({}) ", cs_type) };
                    if ctx.use_sv_comp {
                        result.push_str(&format!("{}__sv_comp.state_vars[\"{}\"]", cast, var_name))
                    } else {
                        result.push_str(&format!("{}__compartment.state_vars[\"{}\"]", cast, var_name))
                    }
                }
                TargetLanguage::Go => {
                    let go_type = ctx.state_var_types.get(&var_name)
                        .map(|t| go_map_type(t))
                        .unwrap_or_else(|| "int".to_string());
                    if ctx.use_sv_comp {
                        result.push_str(&format!("__sv_comp.stateVars[\"{}\"].({})", var_name, go_type))
                    } else {
                        result.push_str(&format!("s.compartment.stateVars[\"{}\"].({})", var_name, go_type))
                    }
                }
                TargetLanguage::Php => {
                    if ctx.use_sv_comp {
                        result.push_str(&format!("$__sv_comp->state_vars[\"{}\"]", var_name))
                    } else {
                        result.push_str(&format!("$this->__compartment->state_vars[\"{}\"]", var_name))
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
        } else if i + 1 < bytes.len() && bytes[i] == b'@' && bytes[i + 1] == b'@' {
            // Found @@ - context syntax
            i += 2; // Skip "@@"
            if i < bytes.len() && bytes[i] == b':' {
                i += 1; // Skip ":"
                // Check which context field
                if i + 5 < bytes.len() && &bytes[i..i + 6] == b"return" {
                    // @@:return
                    i += 6;
                    match lang {
                        TargetLanguage::Python3 | TargetLanguage::GDScript => result.push_str("self._context_stack[-1]._return"),
                        TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => result.push_str("this._context_stack[this._context_stack.length - 1]._return"),
                        TargetLanguage::C => result.push_str(&format!("{}_RETURN(self)", ctx.system_name)),
                        TargetLanguage::Rust => result.push_str("self._context_stack.last().and_then(|ctx| ctx._return.as_ref())"),
                        TargetLanguage::Cpp => result.push_str("std::any_cast<std::string>(this->_context_stack.back()._return)"),
                        TargetLanguage::Java => result.push_str("_context_stack.get(_context_stack.size() - 1)._return"),
                        TargetLanguage::Kotlin => result.push_str("_context_stack[_context_stack.size - 1]._return"),
                        TargetLanguage::Swift => result.push_str("_context_stack[_context_stack.count - 1]._return"),
                        TargetLanguage::CSharp => result.push_str("_context_stack[_context_stack.Count - 1]._return"),
                        TargetLanguage::Go => result.push_str("s.contextStack[len(s.contextStack)-1].returnVal"),
                        TargetLanguage::Php => result.push_str("$this->_context_stack[count($this->_context_stack) - 1]->_return"),
                        TargetLanguage::Ruby => result.push_str("@_context_stack[@_context_stack.length - 1]._return"),
                        TargetLanguage::Lua => result.push_str("self._context_stack[#self._context_stack]._return"),
                        TargetLanguage::Erlang => {}, // TODO: Erlang gen_statem codegen
                        TargetLanguage::Graphviz => unreachable!(),
                    }
                } else if i + 4 < bytes.len() && &bytes[i..i + 5] == b"event" {
                    // @@:event
                    i += 5;
                    match lang {
                        TargetLanguage::Python3 | TargetLanguage::GDScript => result.push_str("self._context_stack[-1].event._message"),
                        TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => result.push_str("this._context_stack[this._context_stack.length - 1].event._message"),
                        TargetLanguage::C => result.push_str(&format!("{}_CTX(self)->event->_message", ctx.system_name)),
                        // Rust: handlers receive __e as parameter, use it directly to avoid borrow conflicts
                        TargetLanguage::Rust => result.push_str("__e.message.clone()"),
                        TargetLanguage::Cpp => result.push_str("this->_context_stack.back()._event._message"),
                        TargetLanguage::Java => result.push_str("_context_stack.get(_context_stack.size() - 1)._event._message"),
                        TargetLanguage::Kotlin => result.push_str("_context_stack[_context_stack.size - 1]._event._message"),
                        TargetLanguage::Swift => result.push_str("_context_stack[_context_stack.count - 1]._event._message"),
                        TargetLanguage::CSharp => result.push_str("_context_stack[_context_stack.Count - 1]._event._message"),
                        TargetLanguage::Go => result.push_str("s.contextStack[len(s.contextStack)-1].event.message"),
                        TargetLanguage::Php => result.push_str("$this->_context_stack[count($this->_context_stack) - 1]->_event->_message"),
                        TargetLanguage::Ruby => result.push_str("@_context_stack[@_context_stack.length - 1]._event._message"),
                        TargetLanguage::Lua => result.push_str("self._context_stack[#self._context_stack]._event._message"),
                        TargetLanguage::Erlang => {}, // TODO: Erlang gen_statem codegen
                        TargetLanguage::Graphviz => unreachable!(),
                    }
                } else if i + 3 < bytes.len() && &bytes[i..i + 4] == b"data" {
                    // @@:data[key]
                    i += 4;
                    if i < bytes.len() && bytes[i] == b'[' {
                        i += 1; // Skip '['
                        let start = i;
                        while i < bytes.len() && bytes[i] != b']' {
                            i += 1;
                        }
                        let key = String::from_utf8_lossy(&bytes[start..i]).trim().trim_matches('"').trim_matches('\'').to_string();
                        if i < bytes.len() {
                            i += 1; // Skip ']'
                        }
                        match lang {
                            TargetLanguage::Python3 | TargetLanguage::GDScript => result.push_str(&format!("self._context_stack[-1]._data[\"{}\"]", key)),
                            TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => result.push_str(&format!("this._context_stack[this._context_stack.length - 1]._data[\"{}\"]", key)),
                            TargetLanguage::C => result.push_str(&format!("{}_DATA(self, \"{}\")", ctx.system_name, key)),
                            TargetLanguage::Rust => result.push_str(&format!("self._context_stack.last().and_then(|ctx| ctx._data.get(\"{}\")).and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default()", key)),
                            TargetLanguage::Cpp => result.push_str(&format!("_context_stack.back()._data[\"{}\"]", key)),
                            TargetLanguage::Java => result.push_str(&format!("_context_stack.get(_context_stack.size() - 1)._data.get(\"{}\")", key)),
                            TargetLanguage::Kotlin => result.push_str(&format!("_context_stack[_context_stack.size - 1]._data[\"{}\"]", key)),
                            TargetLanguage::Swift => result.push_str(&format!("_context_stack[_context_stack.count - 1]._data[\"{}\"]", key)),
                            TargetLanguage::CSharp => result.push_str(&format!("_context_stack[_context_stack.Count - 1]._data[\"{}\"]", key)),
                            TargetLanguage::Go => result.push_str(&format!("s.contextStack[len(s.contextStack)-1].data[\"{}\"]", key)),
                            TargetLanguage::Php => result.push_str(&format!("$this->_context_stack[count($this->_context_stack) - 1]->_data[\"{}\"]", key)),
                            TargetLanguage::Ruby => result.push_str(&format!("@_context_stack[@_context_stack.length - 1]._data[\"{}\"]", key)),
                            TargetLanguage::Lua => result.push_str(&format!("self._context_stack[#self._context_stack]._data[\"{}\"]", key)),
                            TargetLanguage::Erlang => {}, // TODO: Erlang gen_statem codegen
                            TargetLanguage::Graphviz => unreachable!(),
                        }
                    }
                } else if i + 5 < bytes.len() && &bytes[i..i + 6] == b"params" {
                    // @@:params[key]
                    i += 6;
                    if i < bytes.len() && bytes[i] == b'[' {
                        i += 1; // Skip '['
                        let start = i;
                        while i < bytes.len() && bytes[i] != b']' {
                            i += 1;
                        }
                        let key = String::from_utf8_lossy(&bytes[start..i]).trim().trim_matches('"').trim_matches('\'').to_string();
                        if i < bytes.len() {
                            i += 1; // Skip ']'
                        }
                        match lang {
                            TargetLanguage::Python3 | TargetLanguage::GDScript => result.push_str(&format!("self._context_stack[-1].event._parameters[\"{}\"]", key)),
                            TargetLanguage::TypeScript | TargetLanguage::JavaScript => result.push_str(&format!("this._context_stack[this._context_stack.length - 1].event._parameters[\"{}\"]", key)),
                            TargetLanguage::Dart => result.push_str(&format!("this._context_stack[this._context_stack.length - 1].event._parameters![\"{}\"]", key)),
                            TargetLanguage::C => result.push_str(&format!("(intptr_t){}_PARAM(self, \"{}\")", ctx.system_name, key)),
                            // Rust: for params access, just use the handler's direct parameter
                            TargetLanguage::Rust => result.push_str(&key),
                            TargetLanguage::Cpp => result.push_str(&key),
                            TargetLanguage::Java => result.push_str(&format!("_context_stack.get(_context_stack.size() - 1)._event._parameters.get(\"{}\")", key)),
                            TargetLanguage::Kotlin => result.push_str(&format!("_context_stack[_context_stack.size - 1]._event._parameters[\"{}\"]", key)),
                            TargetLanguage::Swift => result.push_str(&format!("_context_stack[_context_stack.count - 1]._event._parameters[\"{}\"]", key)),
                            TargetLanguage::CSharp => result.push_str(&format!("_context_stack[_context_stack.Count - 1]._event._parameters[\"{}\"]", key)),
                            TargetLanguage::Go => result.push_str(&format!("s.contextStack[len(s.contextStack)-1].event.parameters[\"{}\"]", key)),
                            TargetLanguage::Php => result.push_str(&format!("$this->_context_stack[count($this->_context_stack) - 1]->_event->_parameters[\"{}\"]", key)),
                            TargetLanguage::Ruby => result.push_str(&format!("@_context_stack[@_context_stack.length - 1]._event._parameters[\"{}\"]", key)),
                            TargetLanguage::Lua => result.push_str(&format!("self._context_stack[#self._context_stack]._event._parameters[\"{}\"]", key)),
                            TargetLanguage::Erlang => {}, // TODO: Erlang gen_statem codegen
                            TargetLanguage::Graphviz => unreachable!(),
                        }
                    }
                } else {
                    // Unknown, pass through
                    result.push_str("@@:");
                }
            } else {
                // Just @@ without . or :, pass through
                result.push_str("@@");
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Get the native region scanner for the target language
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
        TargetLanguage::Graphviz => {
            panic!("No native region scanner for {:?}", lang)
        }
    }
}
