//! `$.varName` state-variable read and `$.varName = expr` write
//! expansion across the 17 backends.
//!
//! Two sub-emitters, both `pub(super)`:
//!
//! - `expand_state_var` — `$.varName` access. Reads the value
//!   out of the compartment's `state_vars` map/dict and casts
//!   it to the declared type for typed targets. Has a string-
//!   interpolation fast path: if the access appears inside a
//!   string-interpolation context (Python f-string, JS backtick,
//!   etc.) we emit just the read expression without the cast
//!   ceremony — the interpolation host coerces.
//! - `expand_state_var_assign` — `$.varName = expr` write.
//!   Resolves the RHS through `expand_expression` (so nested
//!   `@@:` constructs are lowered before the assignment is
//!   emitted) and writes back into the compartment store with
//!   the per-target type-correct put / cast.
//!
//! Both arms share the same per-target `state_var_types` lookup
//! that drives type-ignorant codegen for static-typed backends
//! — no special-casing per Frame type, the user's declared type
//! string flows straight through to the target's reflective
//! container.

use super::super::codegen_utils::{
    cpp_map_type, csharp_map_type, go_map_type, java_map_type, kotlin_map_type, swift_map_type,
    to_snake_case, HandlerContext,
};
use super::expand_expression;
use super::utility::{extract_state_var_name, php_prefix_params};
use crate::frame_c::compiler::native_region_scanner::{RegionSpan, SegmentMetadata};
use crate::frame_c::visitors::TargetLanguage;

pub(super) fn expand_state_var(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

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
            super::super::rust_system::rust_expand_state_var_read(ctx, &var_name)
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
                format!(
                    "(__sv_comp.state_vars[{}{}{}] as! {})",
                    q, var_name, q, sw_type
                )
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
                format!(
                    "s.__compartment.stateVars[{}{}{}]{}",
                    q, var_name, q, assertion
                )
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
            // Dart's compartment.state_vars is `Map<String, dynamic>`.
            // Reading returns `dynamic`, which propagates `num` to
            // arithmetic and breaks `int` typed assignments (e.g.,
            // `int + dynamic = num`, can't assign to `int`). Cast
            // to the declared type so downstream typing holds.
            let dart_type = ctx
                .state_var_types
                .get(var_name.as_str())
                .map(|t| match t.as_str() {
                    "int" => "int",
                    "float" | "number" | "double" => "double",
                    "str" | "string" | "String" => "String",
                    "bool" | "boolean" => "bool",
                    _ => "",
                })
                .unwrap_or("");
            let access = if ctx.per_handler {
                format!("compartment.state_vars[{}{}{}]", q, var_name, q)
            } else if ctx.use_sv_comp {
                format!("__sv_comp.state_vars[{}{}{}]", q, var_name, q)
            } else {
                format!("this.__compartment.state_vars[{}{}{}]", q, var_name, q)
            };
            if dart_type.is_empty() {
                access
            } else {
                format!("({} as {})", access, dart_type)
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

pub(super) fn expand_state_var_assign(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

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
        TargetLanguage::Rust => super::super::rust_system::rust_expand_state_var_write(
            &indent_str,
            ctx,
            &var_name,
            &expanded_expr,
        ),
        TargetLanguage::C => {
            if ctx.per_handler {
                format!(
                    "{}{}_FrameDict_set(compartment->state_vars, \"{}\", (void*)(intptr_t)({}));",
                    indent_str, ctx.system_name, var_name, expanded_expr
                )
            } else if ctx.use_sv_comp {
                format!(
                    "{}{}_FrameDict_set(__sv_comp->state_vars, \"{}\", (void*)(intptr_t)({}));",
                    indent_str, ctx.system_name, var_name, expanded_expr
                )
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
