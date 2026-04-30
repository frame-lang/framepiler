//! State handler and dispatch code generation.
//!
//! Generates state methods (one per state) containing event handler dispatch.
//! Each language gets a per-language dispatch function that generates the
//! if/elif/switch/match chain routing events to handler bodies.

use super::ast::{CodegenNode, Param, Visibility};
use super::codegen_utils::{
    cpp_map_type, cpp_wrap_any_arg, csharp_map_type, expression_to_string, go_map_type,
    java_map_type, kotlin_map_type, state_var_init_value, swift_map_type, to_snake_case,
    type_to_cpp_string, HandlerContext,
};
use super::frame_expansion::{
    emit_handler_body_via_statements, get_native_scanner, normalize_indentation,
};
use crate::frame_c::compiler::arcanum::{Arcanum, HandlerEntry};
use crate::frame_c::compiler::frame_ast::{MachineAst, StateVarAst, SystemAst, Type};
use crate::frame_c::visitors::TargetLanguage;

// ============================================================================
// Handler Method Name Mangler
// ============================================================================

/// Canonical method name for a Frame handler in a target namespace.
///
/// The mangling splits lifecycle handlers from user interface methods via an
/// explicit `hdl_frame_*` / `hdl_user_*` prefix, so a user method named
/// `enter` (mangled `_s_A_hdl_user_enter`) cannot collide with the lifecycle
/// `$>` handler (mangled `_s_A_hdl_frame_enter`) — fixes the latent Rust-side
/// collision described in bug_enter_exit_method_collision.md.
///
/// Format: `_s_<state>_hdl_frame_enter` (lifecycle enter),
///         `_s_<state>_hdl_frame_exit`  (lifecycle exit),
///         `_s_<state>_hdl_user_<event>` (user interface method).
///
/// Event names for user methods are bare identifiers by parser invariant
/// (`[A-Za-z_][A-Za-z0-9_]*`), so no sanitization is required today. If
/// future syntax introduces non-identifier event keys, extend this helper
/// with a sanitizer rather than letting ad-hoc manglers drift.
pub(crate) fn handler_method_name(state_name: &str, handler: &HandlerEntry) -> String {
    if handler.is_enter {
        format!("_s_{}_hdl_frame_enter", state_name)
    } else if handler.is_exit {
        format!("_s_{}_hdl_frame_exit", state_name)
    } else {
        format!("_s_{}_hdl_user_{}", state_name, handler.event)
    }
}

// ============================================================================
// Unified Dispatch Syntax — shared across all if/elif-style languages
// ============================================================================

/// Language-specific syntax for state dispatch code generation.
/// One struct per language captures every varying piece, allowing a single
/// `generate_unified_state_dispatch` function to emit correct code for
/// all 16 if/elif-style languages. (Rust uses match and stays separate.)
pub(crate) struct DispatchSyntax {
    pub lang: TargetLanguage,
    /// Statement terminator (";" for C-style, "" for Python/Ruby/Lua)
    pub semi: &'static str,
    /// Placeholder for empty handler body ("pass" for Python, "" for brace langs)
    pub empty_body: &'static str,
    /// Body indent prefix (always "    ")
    pub indent: &'static str,
    /// Close brace after the FINAL handler body ("" for Python, "}\n" for brace langs)
    pub close_final: &'static str,
    /// Else clause start ("else:\n" for Python, "} else {\n" for brace langs)
    pub else_start: &'static str,
    /// Receiver prefix for calling own methods inside a generated method
    /// body ("self." for Python/Ruby/Rust, "this." for TS/JS/Java/Kotlin/
    /// C#/Dart/C++, "$this->" for PHP, "s." for Go, "self:" for Lua).
    /// Used by the per-handler thin dispatcher to emit the call site:
    ///   `<self_prefix><method>(__e, compartment)`.
    pub self_prefix: &'static str,

    // --- Callbacks for language-specific code fragments ---
    /// First `if` condition matching event message
    pub fmt_if: fn(message: &str) -> String,
    /// Subsequent `elif`/`else if` condition
    pub fmt_elif: fn(message: &str) -> String,
    /// HSM compartment navigation preamble
    pub fmt_hsm_nav: fn(state_name: &str, system_name: &str) -> String,
    /// Bind a state param to a local variable
    pub fmt_bind_param: fn(name: &str, type_str: &str, system_name: &str, index: usize) -> String,
    /// Check-and-init a state var (inside enter handler or auto-init)
    pub fmt_init_sv: fn(var_name: &str, init_val: &str, indent: &str, system_name: &str) -> String,
    /// Unpack a handler param. `source` is "event" for interface handlers,
    /// "enter" for $> handlers, "exit" for <$ handlers.
    /// `default` is `Some("expr")` for params with declared defaults
    /// (e.g., `$>(val: int = 0)`). `index` is the positional index of
    /// the param in the parameter list — used for Vec/List/Array access.
    pub fmt_unpack: fn(
        name: &str,
        type_str: &str,
        indent: &str,
        system_name: &str,
        source: &str,
        default: Option<&str>,
        index: usize,
    ) -> String,
    /// Forward call to parent state for `=> $^`
    pub fmt_forward: fn(parent_name: &str, indent: &str, system_name: &str) -> String,
}

/// Create the DispatchSyntax for a given language.
pub(crate) fn dispatch_syntax_for(lang: TargetLanguage) -> Option<DispatchSyntax> {
    match lang {
        TargetLanguage::Python3 => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "pass",
            indent: "    ",
            close_final: "",
            else_start: "else:\n",
            self_prefix: "self.",
            fmt_if: |msg| format!("if __e._message == \"{}\":\n", msg),
            fmt_elif: |msg| format!("elif __e._message == \"{}\":\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("# HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("__sv_comp = self.__compartment\n");
                s.push_str(&format!(
                    "while __sv_comp is not None and __sv_comp.state != \"{}\":\n",
                    state
                ));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys, index| {
                format!("{name} = self.__compartment.state_args[{index}]\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if \"{var_name}\" not in __sv_comp.state_vars:\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, source, default, index| {
                format!("{indent}{name} = __e._parameters[{index}]\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}self._state_{parent}(__e)\n"),
        }),
        TargetLanguage::GDScript => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "pass",
            indent: "    ",
            close_final: "",
            else_start: "else:\n",
            self_prefix: "self.",
            fmt_if: |msg| format!("if __e._message == \"{}\":\n", msg),
            fmt_elif: |msg| format!("elif __e._message == \"{}\":\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("# HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = self.__compartment\n");
                s.push_str(&format!(
                    "while __sv_comp != null and __sv_comp.state != \"{}\":\n",
                    state
                ));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys, index| {
                format!("var {name} = self.__compartment.state_args[{index}]\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if not \"{var_name}\" in __sv_comp.state_vars:\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, source, default, index| {
                format!("{indent}var {name} = __e._parameters[{index}]\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}self._state_{parent}(__e)\n"),
        }),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "this.",
            fmt_if: |msg| format!("if (__e._message === \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message === \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("let __sv_comp = this.__compartment;\n");
                s.push_str(&format!(
                    "while (__sv_comp !== null && __sv_comp.state !== \"{}\") {{\n",
                    state
                ));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment;\n");
                s.push_str("}\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys, index| {
                format!("let {name} = this.__compartment.state_args[{index}];\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!(\"{var_name}\" in __sv_comp.state_vars)) {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, source, default, index| {
                format!("{indent}let {name} = __e._parameters[{index}];\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}this._state_{parent}(__e);\n"),
        }),
        TargetLanguage::Ruby => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "end\n",
            else_start: "else\n",
            self_prefix: "self.",
            fmt_if: |msg| format!("if __e._message == \"{}\"\n", msg),
            fmt_elif: |msg| format!("elsif __e._message == \"{}\"\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("# HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("__sv_comp = @__compartment\n");
                s.push_str(&format!(
                    "while __sv_comp != nil && __sv_comp.state != \"{}\"\n",
                    state
                ));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment\n");
                s.push_str("end\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys, index| {
                format!("{name} = @__compartment.state_args[{index}]\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if !__sv_comp.state_vars.key?(\"{var_name}\")\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n\
                     {indent}end\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, _source, default, index| {
                format!("{indent}{name} = __e._parameters[{index}]\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}_state_{parent}(__e)\n"),
        }),
        TargetLanguage::Lua => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "end\n",
            else_start: "else\n",
            self_prefix: "self:",
            fmt_if: |msg| format!("if __e._message == \"{}\" then\n", msg),
            fmt_elif: |msg| format!("elseif __e._message == \"{}\" then\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("-- HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("local __sv_comp = self.__compartment\n");
                s.push_str(&format!(
                    "while __sv_comp ~= nil and __sv_comp.state ~= \"{}\" do\n",
                    state
                ));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment\n");
                s.push_str("end\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys, index| {
                format!(
                    "local {name} = self.__compartment.state_args[{}]\n",
                    index + 1
                )
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if __sv_comp.state_vars[\"{var_name}\"] == nil then\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n\
                     {indent}end\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, source, default, index| {
                let lua_index = index + 1; // Lua is 1-indexed
                format!("{indent}local {name} = __e._parameters[{lua_index}]\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}self:_state_{parent}(__e)\n"),
        }),
        TargetLanguage::Php => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "$this->",
            fmt_if: |msg| format!("if ($__e->_message == \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} elseif ($__e->_message == \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("$__sv_comp = $this->__compartment;\n");
                s.push_str(&format!(
                    "while ($__sv_comp !== null && $__sv_comp->state !== \"{}\") {{\n",
                    state
                ));
                s.push_str("    $__sv_comp = $__sv_comp->parent_compartment;\n");
                s.push_str("}\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys, index| {
                format!("${name} = $this->__compartment->state_args[{index}];\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!isset($__sv_comp->state_vars[\"{var_name}\"])) {{\n\
                     {indent}    $__sv_comp->state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, source, default, index| {
                format!("{indent}${name} = $__e->_parameters[{index}];\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}$this->_state_{parent}($__e);\n"),
        }),
        TargetLanguage::CSharp => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "this.",
            fmt_if: |msg| format!("if (__e._message == \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message == \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp_n = __compartment;\n");
                s.push_str(&format!(
                    "while (__sv_comp_n != null && __sv_comp_n.state != \"{}\") {{\n",
                    state
                ));
                s.push_str("    __sv_comp_n = __sv_comp_n.parent_compartment;\n");
                s.push_str("}\n");
                s.push_str("var __sv_comp = __sv_comp_n!;\n");
                s
            },
            fmt_bind_param: |name, type_str, _sys, index| {
                let cs_type = csharp_map_type(type_str);
                // Convert.ToXxx ladder (D8) — JSON parsers may hand back
                // a different boxed numeric type (long vs double, etc.)
                // than the declared parameter, so normalize via Convert.
                let extract = match cs_type.as_str() {
                    "double" => format!("System.Convert.ToDouble(__compartment.state_args[{index}])"),
                    "float" => format!("System.Convert.ToSingle(__compartment.state_args[{index}])"),
                    "int" => format!("System.Convert.ToInt32(__compartment.state_args[{index}])"),
                    "long" => format!("System.Convert.ToInt64(__compartment.state_args[{index}])"),
                    _ => format!("({cs_type}) __compartment.state_args[{index}]"),
                };
                format!("{cs_type} {name} = {extract};\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!__sv_comp.state_vars.ContainsKey(\"{var_name}\")) {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source, default, index| {
                let cs_type = csharp_map_type(type_str);
                let list = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                let extract = match cs_type.as_str() {
                    "double" => format!("System.Convert.ToDouble({list}[{index}])"),
                    "float" => format!("System.Convert.ToSingle({list}[{index}])"),
                    "int" => format!("System.Convert.ToInt32({list}[{index}])"),
                    "long" => format!("System.Convert.ToInt64({list}[{index}])"),
                    _ => format!("({cs_type}) {list}[{index}]"),
                };
                format!("{indent}var {name} = {extract};\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}_state_{parent}(__e);\n"),
        }),
        TargetLanguage::Java => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "this.",
            fmt_if: |msg| format!("if (__e._message.equals(\"{}\")) {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message.equals(\"{}\")) {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = __compartment;\n");
                s.push_str(&format!("while (__sv_comp != null && !__sv_comp.state.equals(\"{}\")) {{ __sv_comp = __sv_comp.parent_compartment; }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys, index| {
                let java_type = java_map_type(type_str);
                // Number-ladder unwrap so the prefetch works whether the
                // stored value is a live boxed primitive (Double from a
                // configure() call) or a deserialized BigDecimal/Long
                // that org.json may hand back when loaded from JSON via
                // @@persist. (D8 fix.)
                let extract = match java_type.as_str() {
                    "double" => format!("((Number) __compartment.state_args.get({index})).doubleValue()"),
                    "float" => format!("((Number) __compartment.state_args.get({index})).floatValue()"),
                    "long" => format!("((Number) __compartment.state_args.get({index})).longValue()"),
                    "int" => format!("((Number) __compartment.state_args.get({index})).intValue()"),
                    "short" => format!("((Number) __compartment.state_args.get({index})).shortValue()"),
                    "byte" => format!("((Number) __compartment.state_args.get({index})).byteValue()"),
                    _ => format!("({java_type}) __compartment.state_args.get({index})"),
                };
                format!("{java_type} {name} = {extract};\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!__sv_comp.state_vars.containsKey(\"{var_name}\")) {{\n\
                     {indent}    __sv_comp.state_vars.put(\"{var_name}\", {init_val});\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source, default, index| {
                let java_type = java_map_type(type_str);
                let list = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                let extract = match java_type.as_str() {
                    "double" => format!("((Number) {list}.get({index})).doubleValue()"),
                    "float" => format!("((Number) {list}.get({index})).floatValue()"),
                    "long" => format!("((Number) {list}.get({index})).longValue()"),
                    "int" => format!("((Number) {list}.get({index})).intValue()"),
                    _ => format!("({java_type}) {list}.get({index})"),
                };
                format!("{indent}var {name} = {extract};\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}_state_{parent}(__e);\n"),
        }),
        TargetLanguage::Kotlin => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "this.",
            fmt_if: |msg| format!("if (__e._message == \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message == \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = __compartment\n");
                s.push_str(&format!("while (__sv_comp != null && __sv_comp.state != \"{}\") {{ __sv_comp = __sv_comp.parent_compartment!! }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys, index| {
                let kt_type = kotlin_map_type(type_str);
                // Number-ladder unwrap so the prefetch works whether
                // the stored value is a live boxed primitive (Double
                // from a configure() call) or a deserialized
                // BigDecimal/Long that org.json may hand back when the
                // compartment was loaded from JSON via @@persist. (D8.)
                let extract = match kt_type.as_str() {
                    "Double" => format!("(__compartment.state_args[{index}] as Number).toDouble()"),
                    "Float" => format!("(__compartment.state_args[{index}] as Number).toFloat()"),
                    "Long" => format!("(__compartment.state_args[{index}] as Number).toLong()"),
                    "Int" => format!("(__compartment.state_args[{index}] as Number).toInt()"),
                    "Short" => format!("(__compartment.state_args[{index}] as Number).toShort()"),
                    "Byte" => format!("(__compartment.state_args[{index}] as Number).toByte()"),
                    _ => format!("__compartment.state_args[{index}] as {kt_type}"),
                };
                format!("val {name} = {extract}\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!__sv_comp.state_vars.containsKey(\"{var_name}\")) {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source, default, index| {
                let kt_type = kotlin_map_type(type_str);
                let list = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                let extract = match kt_type.as_str() {
                    "Double" => format!("({list}[{index}] as Number).toDouble()"),
                    "Float" => format!("({list}[{index}] as Number).toFloat()"),
                    "Long" => format!("({list}[{index}] as Number).toLong()"),
                    "Int" => format!("({list}[{index}] as Number).toInt()"),
                    _ => format!("{list}[{index}] as {kt_type}"),
                };
                format!("{indent}val {name} = {extract}\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}_state_{parent}(__e)\n"),
        }),
        TargetLanguage::Swift => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "self.",
            fmt_if: |msg| format!("if __e._message == \"{}\" {{\n", msg),
            fmt_elif: |msg| format!("}} else if __e._message == \"{}\" {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = __compartment\n");
                s.push_str(&format!("while __sv_comp.state != \"{}\" {{ __sv_comp = __sv_comp.parent_compartment! }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys, index| {
                let sw_type = swift_map_type(type_str);
                // NSNumber-ladder unwrap (D8) — JSONSerialization can hand
                // back NSNumber that doesn't satisfy `as! Double` directly
                // when the underlying numeric tag differs.
                let extract = match sw_type.as_str() {
                    "Double" => format!("(__compartment.state_args[{index}] as! NSNumber).doubleValue"),
                    "Float" => format!("(__compartment.state_args[{index}] as! NSNumber).floatValue"),
                    "Int" => format!("(__compartment.state_args[{index}] as! NSNumber).intValue"),
                    "Int64" => format!("(__compartment.state_args[{index}] as! NSNumber).int64Value"),
                    _ => format!("__compartment.state_args[{index}] as! {sw_type}"),
                };
                format!("let {name} = {extract}\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if __sv_comp.state_vars[\"{var_name}\"] == nil {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source, default, index| {
                let sw_type = swift_map_type(type_str);
                let list = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                let extract = match sw_type.as_str() {
                    "Double" => format!("({list}[{index}] as! NSNumber).doubleValue"),
                    "Float" => format!("({list}[{index}] as! NSNumber).floatValue"),
                    "Int" => format!("({list}[{index}] as! NSNumber).intValue"),
                    "Int64" => format!("({list}[{index}] as! NSNumber).int64Value"),
                    _ => format!("{list}[{index}] as! {sw_type}"),
                };
                format!("{indent}let {name} = {extract}\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}_state_{parent}(__e)\n"),
        }),
        TargetLanguage::Dart => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "this.",
            // Dart: escape $ in message strings to avoid string interpolation
            fmt_if: |msg| format!("if (__e._message == \"{}\") {{\n", msg.replace('$', "\\$")),
            fmt_elif: |msg| {
                format!(
                    "}} else if (__e._message == \"{}\") {{\n",
                    msg.replace('$', "\\$")
                )
            },
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = __compartment;\n");
                s.push_str(&format!(
                    "while (__sv_comp != null && __sv_comp.state != \"{}\") {{\n",
                    state
                ));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment!;\n");
                s.push_str("}\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys, index| {
                format!("var {name} = __compartment.state_args[{index}];\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!__sv_comp.state_vars.containsKey(\"{var_name}\")) {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source, default, index| {
                let list = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                // Dart: cast to declared type to avoid dynamic/num issues
                let dart_type = match type_str {
                    "int" | "number" | "num" => "int",
                    "double" | "float" => "double",
                    "bool" | "boolean" => "bool",
                    "String" | "str" | "string" => "String",
                    _ => "",
                };
                if dart_type.is_empty() {
                    format!("{indent}final {name} = {list}[{index}];\n")
                } else {
                    format!("{indent}final {name} = {list}[{index}] as {dart_type};\n")
                }
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}_state_{parent}(__e);\n"),
        }),
        TargetLanguage::Cpp => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "this->",
            fmt_if: |msg| format!("if (__e._message == \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message == \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("auto* __sv_comp = __compartment.get();\n");
                s.push_str(&format!("while (__sv_comp && __sv_comp->state != \"{}\") {{ __sv_comp = __sv_comp->parent_compartment.get(); }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys, index| {
                let cpp_type = cpp_map_type(type_str);
                if cpp_type == "std::any" {
                    format!("auto {name} = __compartment->state_args[{index}];\n")
                } else {
                    format!("{cpp_type} {name} = std::any_cast<{cpp_type}>(__compartment->state_args[{index}]);\n")
                }
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                // Wrap string literals in std::string() to avoid const char* / std::string mismatch in std::any
                let wrapped = if init_val.trim().starts_with('"') && init_val.trim().ends_with('"')
                {
                    format!("std::string({})", init_val)
                } else {
                    init_val.to_string()
                };
                format!(
                    "{indent}if (__sv_comp->state_vars.find(\"{var_name}\") == __sv_comp->state_vars.end()) {{\n\
                     {indent}    __sv_comp->state_vars[\"{var_name}\"] = {wrapped};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source, default, index| {
                let cpp_type = cpp_map_type(type_str);
                let list = match source {
                    "enter" => "__compartment->enter_args",
                    "exit" => "__compartment->exit_args",
                    _ => "__e._parameters",
                };
                // Don't any_cast to std::any — just copy directly
                if cpp_type == "std::any" {
                    format!("{indent}auto {name} = {list}[{index}];\n")
                } else {
                    format!(
                        "{indent}{cpp_type} {name} = std::any_cast<{cpp_type}>({list}[{index}]);\n"
                    )
                }
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}_state_{parent}(__e);\n"),
        }),
        TargetLanguage::Go => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            self_prefix: "s.",
            fmt_if: |msg| format!("if __e._message == \"{}\" {{\n", msg),
            fmt_elif: |msg| format!("}} else if __e._message == \"{}\" {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("__sv_comp := s.__compartment\n");
                s.push_str(&format!("for __sv_comp != nil && __sv_comp.state != \"{}\" {{ __sv_comp = __sv_comp.parentCompartment }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys, index| {
                let go_type = go_map_type(type_str);
                format!("{name} := s.__compartment.stateArgs[{index}].({go_type})\n_ = {name}\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if _, ok := __sv_comp.stateVars[\"{var_name}\"]; !ok {{\n\
                     {indent}    __sv_comp.stateVars[\"{var_name}\"] = {init_val}\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source, default, index| {
                let go_type = go_map_type(type_str);
                let list = match source {
                    "enter" => "s.__compartment.enterArgs",
                    "exit" => "s.__compartment.exitArgs",
                    _ => "__e._parameters",
                };
                format!("{indent}{name} := {list}[{index}].({go_type})\n{indent}_ = {name}\n")
            },
            fmt_forward: |parent, indent, _sys| format!("{indent}s._state_{parent}(__e)\n"),
        }),
        TargetLanguage::C => {
            /// Map a Frame parameter type to its C declaration + void*-cast.
            /// Strings → `const char*`, pointer-types (anything ending in `*`)
            /// stay as-is, everything else defaults to `int` via intptr_t.
            fn c_param_type_and_cast(type_str: &str, sys: &str) -> (String, String) {
                let t = type_str.trim();
                match t {
                    "str" | "string" | "String" | "char*" | "const char*" => {
                        ("const char*".to_string(), "(const char*)".to_string())
                    }
                    // Frame's `: list` maps to <sys>_FrameVec* in C
                    // (see backends/c.rs convert_type_to_c). State-args
                    // and event/enter/exit args of list type need the
                    // typed cast, not the int fallthrough.
                    "list" | "List" | "Array" | "Array<any>" => {
                        let typ = format!("{}_FrameVec*", sys);
                        let cast = format!("({})", typ);
                        (typ, cast)
                    }
                    // Same shape for `: dict` → <sys>_FrameDict*.
                    "dict" | "Dict" | "Record<string, any>" => {
                        let typ = format!("{}_FrameDict*", sys);
                        let cast = format!("({})", typ);
                        (typ, cast)
                    }
                    _ if t.ends_with('*') => (t.to_string(), format!("({})", t)),
                    _ => ("int".to_string(), "(int)(intptr_t)".to_string()),
                }
            }
            Some(DispatchSyntax {
                lang,
                semi: ";",
                empty_body: "",
                indent: "    ",
                close_final: "}\n",
                else_start: "} else {\n",
                self_prefix: "self->",
                fmt_if: |msg| format!("if (strcmp(__e->_message, \"{}\") == 0) {{\n", msg),
                fmt_elif: |msg| {
                    format!("}} else if (strcmp(__e->_message, \"{}\") == 0) {{\n", msg)
                },
                fmt_hsm_nav: |state, sys| {
                    let mut s = String::new();
                    s.push_str(
                        "// HSM: Navigate to this state's compartment for state var access\n",
                    );
                    s.push_str(&format!(
                        "{}_Compartment* __sv_comp = self->__compartment;\n",
                        sys
                    ));
                    s.push_str(&format!(
                        "while (__sv_comp != NULL && strcmp(__sv_comp->state, \"{}\") != 0) {{\n",
                        state
                    ));
                    s.push_str("    __sv_comp = __sv_comp->parent_compartment;\n");
                    s.push_str("}\n");
                    s
                },
                fmt_bind_param: |name, type_str, sys, index| {
                    let (c_type, cast) = c_param_type_and_cast(type_str, sys);
                    // state_args is now a FrameVec*, so access via ->items[N].
                    format!("{c_type} {name} = {cast}self->__compartment->state_args->items[{index}];\n")
                },
                fmt_init_sv: |var_name, init_val, indent, sys| {
                    format!(
                    "{indent}if (!{sys}_FrameDict_has(__sv_comp->state_vars, \"{var_name}\")) {{\n\
                     {indent}    {sys}_FrameDict_set(__sv_comp->state_vars, \"{var_name}\", (void*)(intptr_t)({init_val}));\n\
                     {indent}}}\n"
                )
                },
                fmt_unpack: |name, type_str, indent, sys, source, _default, index| {
                    let list = match source {
                        "enter" => "self->__compartment->enter_args",
                        "exit" => "self->__compartment->exit_args",
                        _ => "__e->_parameters",
                    };
                    let (c_type, cast) = c_param_type_and_cast(type_str, sys);
                    // _parameters / enter_args / exit_args are FrameVec*; dereference ->items[N].
                    format!("{indent}{c_type} {name} = {cast}{list}->items[{index}];\n")
                },
                fmt_forward: |parent, indent, sys| {
                    format!("{indent}{sys}_state_{parent}(self, __e);\n")
                },
            })
        }
        // Rust and Erlang stay separate (different dispatch patterns)
        _ => None,
    }
}

/// Unified state dispatch generator for all if/elif-style languages.
/// Uses DispatchSyntax to emit language-correct code without duplication.
pub(crate) fn generate_unified_state_dispatch(
    system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
    syn: &DispatchSyntax,
) -> String {
    let mut code = String::new();
    let mut first = true;
    // Only the lifecycle `$>` key signals an explicit enter handler. A user
    // interface method named `enter` is a regular event — it must not
    // suppress auto-generated state-var init, and its body must not be
    // merged into the `$>` branch.
    let has_enter_handler = handlers.contains_key("$>");

    // 1. State param binding
    for (i, sp) in state_params.iter().enumerate() {
        let type_str = match &sp.param_type {
            Type::Custom(s) => s.as_str(),
            Type::Unknown => "int",
        };
        code.push_str(&(syn.fmt_bind_param)(&sp.name, type_str, system_name, i));
    }

    // 2. HSM compartment navigation
    if !state_vars.is_empty() {
        code.push_str(&(syn.fmt_hsm_nav)(state_name, system_name));
    }

    // 3. Auto-generated enter handler for state var init (when no explicit $>)
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str(&(syn.fmt_if)("$>"));
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, syn.lang)
            } else {
                state_var_init_value(&var.var_type, syn.lang)
            };
            code.push_str(&(syn.fmt_init_sv)(
                &var.name,
                &init_val,
                syn.indent,
                system_name,
            ));
        }
        // Note: for brace langs, the closing } is handled by the next
        // fmt_elif ("} else if") or the final close_final at the end.
        first = false;
    }

    // 4. Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        // Wire message: only the lifecycle keys map to the sigil form. Handler
        // keys of literal `"enter"` / `"exit"` are user-defined interface
        // methods and dispatch under their own name (fixes user-method
        // collision with lifecycle events — bug_enter_exit_method_collision).
        let message = match event.as_str() {
            "$>" => "$>",
            "$<" => "<$",
            _ => event.as_str(),
        };

        // Emit condition
        let condition = if first {
            (syn.fmt_if)(message)
        } else {
            (syn.fmt_elif)(message)
        };
        first = false;
        code.push_str(&condition);

        // State var init in enter handler — only the lifecycle `$>` key.
        if event == "$>" && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, syn.lang)
                } else {
                    state_var_init_value(&var.var_type, syn.lang)
                };
                code.push_str(&(syn.fmt_init_sv)(
                    &var.name,
                    &init_val,
                    syn.indent,
                    system_name,
                ));
            }
        }

        // Param unpacking — lifecycle handlers read from compartment args;
        // interface handlers (including user methods named `enter` / `exit`)
        // read from event._parameters.
        let param_source = if event == "$>" {
            "enter"
        } else if event == "$<" {
            "exit"
        } else {
            "event"
        };
        for (i, param) in handler.params.iter().enumerate() {
            let type_str = match &param.symbol_type {
                Some(t) => t.as_str(),
                None => "int",
            };
            code.push_str(&(syn.fmt_unpack)(
                &param.name,
                type_str,
                syn.indent,
                system_name,
                param_source,
                param.default_value.as_deref(),
                i,
            ));
        }

        // Handler return init
        let return_init_code =
            emit_handler_return_init(handler, syn.lang, syn.indent, &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        handler_ctx.current_return_type = handler.return_type.clone();
        let body =
            emit_handler_body_via_statements(&handler.body_span, source, syn.lang, &handler_ctx);

        let mut body_has_content = !return_init_code.is_empty();
        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str(syn.indent);
                code.push_str(line);
                body_has_content = true;
            }
            code.push('\n');
        }

        // Empty body placeholder
        if !body_has_content && !syn.empty_body.is_empty() {
            code.push_str(syn.indent);
            code.push_str(syn.empty_body);
            code.push('\n');
        }
    }

    // 5. Default forward or close final block
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str(syn.else_start);
                code.push_str(&(syn.fmt_forward)(parent, syn.indent, system_name));
                code.push_str(syn.close_final);
            } else {
                code.push_str(&(syn.fmt_forward)(parent, "", system_name));
            }
        }
    } else if !first && !syn.close_final.is_empty() {
        // Close the final handler block (brace languages need `}`)
        code.push_str(syn.close_final);
    }

    code.trim_end().to_string()
}

/// Generic thin dispatcher body — emits one guarded block per handler
/// that calls the handler method and returns. Shared across all per-
/// handler-architecture targets; language syntax comes from the
/// `DispatchSyntax` struct. Handler bodies are NOT inlined — they live
/// in their own methods emitted by `generate_per_handler_methods`.
///
/// See docs/frame_runtime.md § "Dispatch Model" for the three-layer
/// pipeline rationale.
fn generate_thin_dispatcher_generic(
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    ctx: &HandlerContext,
    default_forward: bool,
    has_state_vars: bool,
    syn: &DispatchSyntax,
) -> String {
    let mut code = String::new();
    let indent = syn.indent;
    let semi = syn.semi;
    let close = syn.close_final;
    let self_prefix = syn.self_prefix;
    // PHP requires `$` on variable references (`$__e`, `$compartment`);
    // every other per-handler target emits bare identifiers.
    let var_sigil = if matches!(syn.lang, TargetLanguage::Php) {
        "$"
    } else {
        ""
    };
    // C uses free functions (`Sys_<method>(self, ...)`) instead of
    // member dispatch (`self->method(...)`). Switch to the C convention
    // for handler / forward calls.
    let is_c = matches!(syn.lang, TargetLanguage::C);

    // State params bind from compartment.state_args at the top of the
    // dispatcher. Uses fmt_bind_param for language-specific syntax.
    for (i, sp) in state_params.iter().enumerate() {
        let type_str = match &sp.param_type {
            Type::Custom(s) => s.as_str(),
            Type::Unknown => "int",
        };
        code.push_str(&(syn.fmt_bind_param)(
            &sp.name,
            type_str,
            &ctx.system_name,
            i,
        ));
    }

    // Synthesize a `$>` dispatch arm when the state has state vars but no
    // explicit `$>` handler. The synthetic `_s_<State>_hdl_frame_enter`
    // method is emitted by generate_per_handler_methods and does the
    // state-var default-init.
    let has_explicit_enter = handlers.contains_key("$>");
    if has_state_vars && !has_explicit_enter {
        let method = format!("_s_{}_hdl_frame_enter", state_name);
        code.push_str(&(syn.fmt_if)("$>"));
        if is_c {
            code.push_str(&format!(
                "{indent}{}_s_{}_hdl_frame_enter(self, __e, compartment){semi}\n",
                ctx.system_name, state_name
            ));
        } else {
            code.push_str(&format!(
                "{indent}{self_prefix}{method}({var_sigil}__e, {var_sigil}compartment){semi}\n"
            ));
        }
        code.push_str(&format!("{indent}return{semi}\n"));
        code.push_str(close);
    }

    // Sort handlers for deterministic output.
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let wire_message = match event.as_str() {
            "$>" => "$>",
            "$<" => "<$",
            other => other,
        };
        let method_name = handler_method_name(state_name, handler);
        // Each branch is its own standalone `if ... return` block so
        // the async-aware `add_await_to_dispatch_calls` pass processes
        // each call on its own line. A single-line
        // `if X: self.foo(); return` form would trigger a line-wide
        // match and prepend `await ` in front of the `if` keyword.
        code.push_str(&(syn.fmt_if)(wire_message));
        if is_c {
            code.push_str(&format!(
                "{indent}{}_{}(self, __e, compartment){semi}\n",
                ctx.system_name,
                method_name.trim_start_matches('_')
            ));
        } else {
            code.push_str(&format!(
                "{indent}{self_prefix}{method_name}({var_sigil}__e, {var_sigil}compartment){semi}\n"
            ));
        }
        code.push_str(&format!("{indent}return{semi}\n"));
        code.push_str(close);
    }

    // Default-forward trailing call — emitted only when the state
    // declares `=> $^`. The forward shifts `compartment` up one level
    // (see docs/frame_runtime.md § "Parent Forward"). Dart is null-
    // safe; assert non-null with `!` on the deref.
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            // Dart / Swift / TypeScript (`!`) and Kotlin (`!!`) assert
            // non-null at the deref. TypeScript needs it because the
            // Compartment.parent_compartment type is `Compartment | null`
            // under strict null checks.
            let bang = match syn.lang {
                TargetLanguage::Dart | TargetLanguage::Swift | TargetLanguage::TypeScript => "!",
                TargetLanguage::Kotlin => "!!",
                _ => "",
            };
            // PHP + C++ use `->` for property access; other targets use `.`.
            let deref = if matches!(syn.lang, TargetLanguage::Php | TargetLanguage::Cpp) {
                "->"
            } else {
                "."
            };
            // Go's runtime struct uses camelCase; everyone else uses snake_case.
            let parent_field = if matches!(syn.lang, TargetLanguage::Go) {
                "parentCompartment"
            } else {
                "parent_compartment"
            };
            if is_c {
                code.push_str(&format!(
                    "{}_state_{}(self, __e, compartment->parent_compartment){semi}\n",
                    ctx.system_name, parent
                ));
            } else {
                code.push_str(&format!(
                    "{self_prefix}_state_{}({var_sigil}__e, {var_sigil}compartment{deref}{parent_field}{}){semi}\n",
                    parent, bang
                ));
            }
        }
    }

    // If the dispatcher body is empty (no handlers, no default forward),
    // indent-based langs (Python) require a `pass`; brace langs accept
    // an empty body.
    if code.is_empty() && !syn.empty_body.is_empty() {
        code.push_str(syn.empty_body);
        code.push('\n');
    }

    code.trim_end().to_string()
}

/// Generate handler return_init code: sets the context return value at handler entry.
/// Returns empty string if handler has no return_init.
fn emit_handler_return_init(
    handler: &HandlerEntry,
    lang: TargetLanguage,
    indent: &str,
    system_name: &str,
) -> String {
    let Some(ref init_expr) = handler.return_init else {
        return String::new();
    };
    let assign = match lang {
        TargetLanguage::Python3 => format!("{}self._context_stack[-1]._return = {}\n", indent, init_expr),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!("{}this._context_stack[this._context_stack.length - 1]._return = {};\n", indent, init_expr),
        TargetLanguage::C => {
            // Doubles don't survive `(void*)(intptr_t)(val)` — the
            // intptr_t cast truncates. Bit-pun through memcpy via the
            // generated `Sys_pack_double` helper.
            let is_dbl = handler
                .return_type
                .as_deref()
                .map(|t| {
                    let t = t.trim();
                    t == "float" || t == "double"
                })
                .unwrap_or(false);
            if is_dbl {
                format!(
                    "{}{}_CTX(self)->_return = {}_pack_double({});\n",
                    indent, system_name, system_name, init_expr
                )
            } else {
                format!(
                    "{}{}_CTX(self)->_return = (void*)(intptr_t)({});\n",
                    indent, system_name, init_expr
                )
            }
        }
        TargetLanguage::Rust => format!("{}if let Some(ctx) = self._context_stack.last_mut() {{ ctx._return = Some(Box::new({}) as Box<dyn std::any::Any>); }}\n", indent, init_expr),
        TargetLanguage::Cpp => format!("{}_context_stack.back()._return = std::any({});\n", indent, init_expr),
        TargetLanguage::Java => format!("{}_context_stack.get(_context_stack.size() - 1)._return = {};\n", indent, init_expr),
        TargetLanguage::Kotlin => format!("{}_context_stack[_context_stack.size - 1]._return = {}\n", indent, init_expr),
        TargetLanguage::Swift => format!("{}_context_stack[_context_stack.count - 1]._return = {}\n", indent, init_expr),
        TargetLanguage::CSharp => format!("{}_context_stack[_context_stack.Count - 1]._return = {};\n", indent, init_expr),
        TargetLanguage::Go => format!("{}s._context_stack[len(s._context_stack)-1]._return = {}\n", indent, init_expr),
        TargetLanguage::Php => format!("{}$this->_context_stack[count($this->_context_stack) - 1]->_return = {};\n", indent, init_expr),
        TargetLanguage::Ruby => format!("{}@_context_stack[@_context_stack.length - 1]._return = {}\n", indent, init_expr),
        TargetLanguage::Lua => format!("{}self._context_stack[#self._context_stack]._return = {}\n", indent, init_expr),
        TargetLanguage::Dart => format!("{}_context_stack[_context_stack.length - 1]._return = {};\n", indent, init_expr),
        TargetLanguage::GDScript => format!("{}self._context_stack[self._context_stack.size() - 1]._return = {}\n", indent, init_expr),
        TargetLanguage::Erlang => format!("{}__ReturnVal = {},\n", indent, init_expr),
        TargetLanguage::Graphviz => String::new(),
    };
    assign
}

/// Generate state handler methods using the enhanced Arcanum
///
/// For all languages: Generates `_state_{StateName}(__e)` methods that dispatch internally
/// based on the event message, plus individual handler methods
pub(crate) fn generate_state_handlers_via_arcanum(
    system_name: &str,
    machine: &MachineAst,
    arcanum: &Arcanum,
    source: &[u8],
    lang: TargetLanguage,
    has_state_vars: bool,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // Collect all defined system names for @@System() validation
    let defined_systems: std::collections::HashSet<String> =
        arcanum.systems.keys().cloned().collect();

    // Build state→param-names lookup so transition codegen can convert
    // positional state args (`-> $S(42)`) into named writes
    // (`state_args["the_param_name"] = 42`). This is the canonical map —
    // both the constructor's start-state population and the transition
    // emit sites read from it (or use the same name convention) so that
    // the state dispatch reader can use a single named lookup.
    let state_param_names: std::collections::HashMap<String, Vec<String>> = machine
        .states
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.params.iter().map(|p| p.name.clone()).collect(),
            )
        })
        .collect();
    // Cascade-aware view: a state's effective param names include any
    // names declared on a child's cascade arrow `$Child => $Self(name: T)`.
    // The runtime's __prepareEnter propagates state_args to every
    // compartment in the chain, so a parent state's handlers can read
    // those values. Used only for handler-body prefetch; transition
    // writes / Rust variant decls keep using `state_param_names` (own
    // params only).
    let state_param_effective_names: std::collections::HashMap<String, Vec<String>> = {
        let mut m = state_param_names.clone();
        for s in &machine.states {
            if s.params.is_empty() {
                continue;
            }
            // Walk up the entire HSM parent chain — every ancestor's
            // handlers can see the descendant's cascade-arrow params
            // because the runtime's frame_state_args / state_args list
            // is the same value for every compartment in the chain.
            let mut current = s.parent.clone();
            while let Some(parent_name) = current {
                let entry = m.entry(parent_name.clone()).or_default();
                for p in &s.params {
                    if !entry.contains(&p.name) {
                        entry.push(p.name.clone());
                    }
                }
                current = machine
                    .states
                    .iter()
                    .find(|st| st.name == parent_name)
                    .and_then(|st| st.parent.clone());
            }
        }
        m
    };
    // Companion map: for every (state_name, param_name) in the
    // effective view, the param's declared type (as a Frame source
    // string — backends map it to native types). Used by typed-
    // language per-handler emit so cast types match the declaration
    // (otherwise C/C++/Java/etc. default to `int` and break str/bool).
    let state_param_types: std::collections::HashMap<(String, String), String> = {
        let mut m: std::collections::HashMap<(String, String), String> =
            std::collections::HashMap::new();
        for s in &machine.states {
            for p in &s.params {
                let type_str = match &p.param_type {
                    crate::frame_c::compiler::frame_ast::Type::Custom(t) => t.clone(),
                    crate::frame_c::compiler::frame_ast::Type::Unknown => "int".to_string(),
                };
                // Own params (state owns the declaration site).
                m.insert((s.name.clone(), p.name.clone()), type_str.clone());
                // Cascade-inherited: walk parent chain and register
                // the same name+type at every ancestor.
                let mut current = s.parent.clone();
                while let Some(parent_name) = current {
                    m.entry((parent_name.clone(), p.name.clone()))
                        .or_insert_with(|| type_str.clone());
                    current = machine
                        .states
                        .iter()
                        .find(|st| st.name == parent_name)
                        .and_then(|st| st.parent.clone());
                }
            }
        }
        m
    };
    // Mirror for enter handler params: maps target state name to its
    // declared `$>(name: type)` enter handler param names. Lets transition
    // codegen write enter_args by name instead of by positional index.
    let state_enter_param_names: std::collections::HashMap<String, Vec<String>> = machine
        .states
        .iter()
        .map(|s| {
            let enter_params: Vec<String> = s
                .enter
                .as_ref()
                .map(|e| e.params.iter().map(|p| p.name.clone()).collect())
                .unwrap_or_default();
            (s.name.clone(), enter_params)
        })
        .collect();
    // Mirror for exit handler params: maps source state name to its
    // declared `<$(name: type)` exit handler param names. Lets transition
    // codegen write exit_args by name. Note this is keyed by the *source*
    // state of a transition (the one we're leaving), not the target.
    let state_exit_param_names: std::collections::HashMap<String, Vec<String>> = machine
        .states
        .iter()
        .map(|s| {
            let exit_params: Vec<String> = s
                .exit
                .as_ref()
                .map(|e| e.params.iter().map(|p| p.name.clone()).collect())
                .unwrap_or_default();
            (s.name.clone(), exit_params)
        })
        .collect();

    // Build event→param-names lookup for @@:params.name → positional index resolution.
    // Built from the machine AST's interface handler params.
    let mut event_param_names: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for state in &machine.states {
        for handler in &state.handlers {
            if !handler.params.is_empty() && !event_param_names.contains_key(&handler.event) {
                event_param_names.insert(
                    handler.event.clone(),
                    handler.params.iter().map(|p| p.name.clone()).collect(),
                );
            }
        }
    }

    // Build state → declared HSM parent lookup from `$Child => $Parent`
    // declarations. Used by transition codegen to eagerly construct the new
    // compartment's parent_compartment chain — see
    // _scratch/bug_parent_compartment_hsm_walk.md.
    let state_hsm_parents: std::collections::HashMap<String, String> = machine
        .states
        .iter()
        .filter_map(|s| s.parent.as_ref().map(|p| (s.name.clone(), p.clone())))
        .collect();

    // Identify the start state (first state in the machine) so the
    // Rust dispatch can switch on whether this state's lifecycle params
    // are bound from system header (start) or from transitions (non-start).
    let start_state_name_for_dispatch = machine
        .states
        .first()
        .map(|s| s.name.clone())
        .unwrap_or_default();

    // Generate one _state_{StateName} dispatch method per state for ALL
    // languages. We iterate the AST's `machine.states` (Vec, declaration
    // order) rather than `arcanum.get_enhanced_states` (HashMap, iteration
    // order is nondeterministic between framec runs). Determinism is a
    // hard requirement for downstream caches (ccache hits to ~70% before
    // this fix, since the C backend's forward-decl section reordered
    // between runs).
    for state_ast in machine.states.iter() {
        let state_entry = match arcanum.get_enhanced_state(system_name, &state_ast.name) {
            Some(e) => e,
            None => continue,
        };
        let state_ast = Some(state_ast);
        let state_vars = state_ast.map(|s| &s.state_vars[..]).unwrap_or(&[]);
        // State params (e.g. `$Start(x: int)`) — needed so the dispatch can
        // bind compartment.state_args[name] to a local at the top of the
        // function before any handler runs.
        let state_params: &[crate::frame_c::compiler::frame_ast::StateParam] =
            state_ast.map(|s| &s.params[..]).unwrap_or(&[]);
        // V4: Enable default_forward ONLY if explicitly set with `=> $^` in state body
        // Having a parent (HSM) does NOT imply auto-forwarding
        let has_explicit_forward = state_ast.map(|s| s.default_forward).unwrap_or(false);
        let default_forward = has_explicit_forward;
        let is_start_state = state_entry.name == start_state_name_for_dispatch;

        let method = generate_state_method(
            system_name,
            &state_entry.name,
            state_entry.parent.as_deref(),
            &state_entry.handlers,
            state_vars,
            state_params,
            &state_param_names,
            &state_enter_param_names,
            &state_exit_param_names,
            &event_param_names,
            source,
            lang,
            has_state_vars,
            default_forward,
            &defined_systems,
            is_start_state,
        );
        // State-level leading comments emit as NativeBlock nodes
        // before the dispatch method itself. Same shape as
        // interface / action / operation comment plumbing.
        if let Some(state) = state_ast {
            for comment in &state.leading_comments {
                methods.push(CodegenNode::NativeBlock {
                    code: comment.clone(),
                    span: None,
                });
            }
        }
        methods.push(method);
    }

    if matches!(lang, TargetLanguage::Rust) {
        methods.extend(super::rust_system::generate_rust_handler_methods(
            system_name,
            machine,
            arcanum,
            source,
            has_state_vars,
            &defined_systems,
            &state_param_names,
            &state_enter_param_names,
            &state_exit_param_names,
        ));
    }

    // Per-handler architecture: emit one method per handler, called
    // by the thin dispatcher generated in `generate_state_method`. See
    // docs/frame_runtime.md § "Dispatch Model".
    if matches!(
        lang,
        TargetLanguage::Python3
            | TargetLanguage::TypeScript
            | TargetLanguage::JavaScript
            | TargetLanguage::Ruby
            | TargetLanguage::GDScript
            | TargetLanguage::Lua
            | TargetLanguage::Dart
            | TargetLanguage::Php
            | TargetLanguage::Go
            | TargetLanguage::Java
            | TargetLanguage::Kotlin
            | TargetLanguage::Swift
            | TargetLanguage::Cpp
            | TargetLanguage::CSharp
            | TargetLanguage::C
    ) {
        methods.extend(generate_per_handler_methods(
            lang,
            system_name,
            machine,
            arcanum,
            source,
            has_state_vars,
            &defined_systems,
            &state_param_effective_names,
            &state_enter_param_names,
            &state_exit_param_names,
            &event_param_names,
            &state_param_types,
        ));
    }

    methods
}

/// Emit per-handler methods for Python. Mirrors the structure of
/// `generate_rust_handler_methods` but with the Python-specific
/// handler-body mode flag (`per_handler: true`), so Frame expansion
/// targets `compartment.state_vars[…]` / `compartment.parent_compartment`
/// etc. rather than the legacy `__sv_comp` / `self.__compartment` forms.
pub(crate) fn generate_per_handler_methods(
    lang: TargetLanguage,
    system_name: &str,
    machine: &MachineAst,
    arcanum: &Arcanum,
    source: &[u8],
    has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // State → declared HSM parent map for use by transition codegen inside
    // handler bodies (so `-> $Child` where Child => Parent constructs the
    // full chain rather than patching parent_compartment = self.__compartment).
    let state_hsm_parents: std::collections::HashMap<String, String> = machine
        .states
        .iter()
        .filter_map(|s| s.parent.as_ref().map(|p| (s.name.clone(), p.clone())))
        .collect();

    let start_state_name = machine
        .states
        .first()
        .map(|s| s.name.clone())
        .unwrap_or_default();

    // Iterate via machine.states (Vec, deterministic) and look up the
    // enhanced state by name. See comment above the first iteration.
    for state_ast_iter in machine.states.iter() {
        let state_entry = match arcanum.get_enhanced_state(system_name, &state_ast_iter.name) {
            Some(e) => e,
            None => continue,
        };
        let is_start_state = state_entry.name == start_state_name;
        let handler_state_var_types: std::collections::HashMap<String, String> = machine
            .states
            .iter()
            .find(|s| s.name == state_entry.name)
            .map(|s| {
                s.state_vars
                    .iter()
                    .map(|sv| {
                        let type_str = match &sv.var_type {
                            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
                            crate::frame_c::compiler::frame_ast::Type::Unknown => "int".to_string(),
                        };
                        (sv.name.clone(), type_str)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let state_ast = machine.states.iter().find(|s| s.name == state_entry.name);
        let state_vars_for_init: &[StateVarAst] =
            state_ast.map(|s| &s.state_vars[..]).unwrap_or(&[]);

        // Synthesize an implicit `$>` lifecycle handler when the state has
        // state vars but the user did NOT write `$>() { … }` explicitly.
        // Without this, `$>` fires but the dispatcher has no arm for it, so
        // state-var default values are never written and subsequent reads
        // of `$.varName` hit a KeyError. Monolithic dispatch emitted this
        // synthetic arm inline; per-handler must emit it as a method.
        let has_explicit_enter = state_entry.handlers.contains_key("$>");
        if !state_vars_for_init.is_empty() && !has_explicit_enter {
            let synthetic_enter = HandlerEntry {
                event: "$>".to_string(),
                params: Vec::new(),
                return_type: None,
                return_init: None,
                body_span: crate::frame_c::compiler::ast::Span { start: 0, end: 0 },
                body_statements: Vec::new(),
                is_enter: true,
                is_exit: false,
                leading_comments: Vec::new(),
            };
            let empty: Vec<String> = Vec::new();
            let method = generate_per_handler_method_for_lang(
                lang,
                system_name,
                &state_entry.name,
                state_entry.parent.as_deref(),
                &synthetic_enter,
                state_vars_for_init,
                source,
                has_state_vars,
                defined_systems,
                &empty,
                is_start_state,
                state_param_names,
                state_enter_param_names,
                state_exit_param_names,
                event_param_names,
                &handler_state_var_types,
                &state_hsm_parents,
                state_param_types,
            );
            methods.push(method);
        }

        // Sort by event name so per-handler method emission order is
        // deterministic — matches the existing `sorted_handlers`
        // pattern in this file (lines 777, 966) and prevents the C
        // backend from emitting forward-decls in HashMap iteration
        // order (which varies between framec runs and defeated
        // ccache hits in the matrix runner).
        let mut sorted_state_handlers: Vec<_> = state_entry.handlers.iter().collect();
        sorted_state_handlers.sort_by(|a, b| a.0.cmp(b.0));
        for (_event, handler_entry) in sorted_state_handlers {
            let empty: Vec<String> = Vec::new();
            let method = generate_per_handler_method_for_lang(
                lang,
                system_name,
                &state_entry.name,
                state_entry.parent.as_deref(),
                handler_entry,
                state_vars_for_init,
                source,
                has_state_vars,
                defined_systems,
                &empty,
                is_start_state,
                state_param_names,
                state_enter_param_names,
                state_exit_param_names,
                event_param_names,
                &handler_state_var_types,
                &state_hsm_parents,
                state_param_types,
            );
            // Per-handler leading comments (from `HandlerAst.leading_comments`
            // / `EnterHandler.leading_comments` / `ExitHandler.leading_comments`,
            // threaded through arcanum's `HandlerEntry`). Emit each one as a
            // class-scope NativeBlock immediately above the per-handler
            // method definition.
            for comment in &handler_entry.leading_comments {
                methods.push(CodegenNode::NativeBlock {
                    code: comment.clone(),
                    span: None,
                });
            }
            methods.push(method);
        }
    }

    methods
}

/// Dispatch to the per-language handler-method emitter for a per-handler
/// architecture target. Each target builds the same 3-param method
/// signature `(self, __e, compartment)` but with per-language syntax for
/// param types, param binding, state-var init preamble, and statement
/// terminators. The handler body itself is emitted via
/// `emit_handler_body_via_statements` with `per_handler: true`, so
/// Frame-expansion side of the codegen routes state-var access,
/// `=> $^`, etc. to the compartment-parameter form.
fn generate_per_handler_method_for_lang(
    lang: TargetLanguage,
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    sys_param_locals: &[String],
    is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    match lang {
        TargetLanguage::Python3 => generate_python_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
        ),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            generate_typescript_handler_method(
                lang,
                system_name,
                state_name,
                parent_state,
                handler,
                state_vars_for_init,
                source,
                has_state_vars,
                defined_systems,
                sys_param_locals,
                is_start_state,
                state_param_names,
                state_enter_param_names,
                state_exit_param_names,
                event_param_names,
                handler_state_var_types,
                state_hsm_parents,
            )
        }
        TargetLanguage::Ruby => generate_ruby_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
        ),
        TargetLanguage::GDScript => generate_gdscript_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        TargetLanguage::Lua => generate_lua_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
        ),
        TargetLanguage::Dart => generate_dart_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        TargetLanguage::Php => generate_php_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
        ),
        TargetLanguage::Go => generate_go_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        TargetLanguage::Java => generate_java_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        TargetLanguage::Kotlin => generate_kotlin_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        TargetLanguage::Swift => generate_swift_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        TargetLanguage::Cpp => generate_cpp_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        TargetLanguage::CSharp => generate_csharp_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        TargetLanguage::C => generate_c_handler_method(
            system_name,
            state_name,
            parent_state,
            handler,
            state_vars_for_init,
            source,
            has_state_vars,
            defined_systems,
            sys_param_locals,
            is_start_state,
            state_param_names,
            state_enter_param_names,
            state_exit_param_names,
            event_param_names,
            handler_state_var_types,
            state_hsm_parents,
            state_param_types,
        ),
        _ => unreachable!(
            "generate_per_handler_method_for_lang called with non-per-handler target {:?}",
            lang
        ),
    }
}

/// Emit a single Ruby handler method under the per-handler contract:
///   def _s_<State>_hdl_<kind>_<event>(__e, compartment)
///     ...
///   end
fn generate_ruby_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Ruby;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding.
    if let Some(sp_names) = state_param_names.get(state_name) {
        for (i, name) in sp_names.iter().enumerate() {
            body.push_str(&format!("{} = compartment.state_args[{}]\n", name, i));
        }
    }

    // Handler-param binding.
    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        body.push_str(&format!("{} = {}[{}]\n", param.name, param_source, i));
    }

    // State-var init — lifecycle enter only, guarded via `unless key?`.
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "unless compartment.state_vars.key?(\"{}\")\n    compartment.state_vars[\"{}\"] = {}\nend\n",
                var.name, var.name, init_val
            ));
        }
    }

    // Return-init assignment.
    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    // User-written handler body via frame-expansion with per_handler=true.
    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    if body.trim().is_empty() {
        // Ruby permits empty bodies but an explicit nil keeps intent clear.
        body.push_str("nil\n");
    }

    CodegenNode::Method {
        name: method_name,
        params: vec![Param::new("__e"), Param::new("compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single Lua handler method under the per-handler contract.
/// Lua specifics: 1-indexed arrays (`[i+1]`), `local` for vars, `nil`
/// check for state-var init guard.
fn generate_lua_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Lua;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    if let Some(sp_names) = state_param_names.get(state_name) {
        for (i, name) in sp_names.iter().enumerate() {
            // Lua is 1-indexed — shift `[i]` to `[i+1]` for state_args.
            body.push_str(&format!(
                "local {} = compartment.state_args[{}]\n",
                name,
                i + 1
            ));
        }
    }

    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        body.push_str(&format!(
            "local {} = {}[{}]\n",
            param.name,
            param_source,
            i + 1
        ));
    }

    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            // Lua: nil check since tables return nil for missing keys.
            body.push_str(&format!(
                "if compartment.state_vars[\"{}\"] == nil then\n    compartment.state_vars[\"{}\"] = {}\nend\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    CodegenNode::Method {
        name: method_name,
        params: vec![Param::new("__e"), Param::new("compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single PHP handler method under the per-handler contract.
/// PHP specifics: `$param` for handler params, `$this->` receiver,
/// `$compartment->` for the passed compartment, `array_key_exists`
/// init guard.
fn generate_php_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Php;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    if let Some(sp_names) = state_param_names.get(state_name) {
        for (i, name) in sp_names.iter().enumerate() {
            body.push_str(&format!("${} = $compartment->state_args[{}];\n", name, i));
        }
    }

    let param_source = if handler.is_enter {
        "$compartment->enter_args"
    } else if handler.is_exit {
        "$compartment->exit_args"
    } else {
        "$__e->_parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        body.push_str(&format!("${} = {}[{}];\n", param.name, param_source, i));
    }

    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if (!array_key_exists(\"{}\", $compartment->state_vars)) {{\n    $compartment->state_vars[\"{}\"] = {};\n}}\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    CodegenNode::Method {
        name: method_name,
        // PHP uses `$` prefix on formal param names (the codegen emitter
        // adds it). Use bare names here — the PHP backend prefixes.
        params: vec![Param::new("__e"), Param::new("compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single Go handler method under the per-handler contract:
///   func (s *Sys) _s_<State>_hdl_<kind>_<event>(__e *SysFrameEvent, compartment *SysCompartment)
/// Go specifics: positional `compartment.stateArgs[i].(T)` type-assertions
/// for state-param / handler-param binding, `if _, ok := ...; !ok`
/// state-var init guard, `_ = name` suppresses unused-var errors.
#[allow(clippy::too_many_arguments)]
fn generate_go_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Go;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding. Go's untyped `interface{}` needs a type assertion;
    // fall back to `any` when the declared type is missing.
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — Go's
            // `:=` disallows redeclaration in the same scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            let frame_type = state_param_types
                .get(&(state_name.to_string(), name.clone()))
                .map(|s| s.as_str())
                .unwrap_or("int");
            let go_type = go_map_type(frame_type);
            body.push_str(&format!(
                "{} := compartment.stateArgs[{}].({})\n_ = {}\n",
                name, i, go_type, name
            ));
        }
    }

    // Handler-param binding.
    let param_source = if handler.is_enter {
        "compartment.enterArgs"
    } else if handler.is_exit {
        "compartment.exitArgs"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        let type_str = param.symbol_type.as_deref().unwrap_or("int");
        let go_type = go_map_type(type_str);
        body.push_str(&format!(
            "{} := {}[{}].({})\n_ = {}\n",
            param.name, param_source, i, go_type, param.name
        ));
    }

    // State-var init (lifecycle enter only).
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if _, ok := compartment.stateVars[\"{}\"]; !ok {{\n    compartment.stateVars[\"{}\"] = {}\n}}\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    // Wrap the user body in a nested block. Go method receiver is `s`,
    // and the pre-migration codegen emitted handler bodies inside an
    // `if __e._message == "…" { … }` block. Under per-handler dispatch
    // the body sits at method-scope level where `s := "…"` would
    // collide with the receiver. Adding `{ … }` restores the nested
    // scope so user locals named `s` shadow the receiver, matching
    // pre-migration semantics.
    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    if !body_src.trim().is_empty() {
        body.push_str("{\n");
        body.push_str(&body_src);
        if !body_src.ends_with('\n') {
            body.push('\n');
        }
        body.push_str("}\n");
    }

    let event_type = format!("*{}FrameEvent", system_name);
    let comp_type = format!("*{}Compartment", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single Java handler method under the per-handler contract:
///   private void _s_<State>_hdl_<kind>_<event>(SysFrameEvent __e, SysCompartment compartment)
/// Java specifics: `(Type) compartment.state_args.get(i)` typed fetches,
/// `containsKey("x")` init guard, `ArrayList`/`HashMap` accessors.
#[allow(clippy::too_many_arguments)]
fn generate_java_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Java;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding. Java needs explicit typed cast. For
    // numeric types we go through Number's ladder so the prefetch
    // works whether the stored value is a live boxed primitive
    // (Double from a configure() call) or a deserialized
    // BigDecimal/Long/Integer that org.json may hand back when the
    // compartment was loaded from JSON via @@persist. (D8 fix.)
    let java_extract = |java_type: &str, src: &str| -> String {
        match java_type {
            "double" => format!("((Number) {}).doubleValue()", src),
            "float" => format!("((Number) {}).floatValue()", src),
            "long" => format!("((Number) {}).longValue()", src),
            "int" => format!("((Number) {}).intValue()", src),
            "short" => format!("((Number) {}).shortValue()", src),
            "byte" => format!("((Number) {}).byteValue()", src),
            _ => format!("({}) {}", java_type, src),
        }
    };
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — Java
            // disallows variable redeclaration in the same scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            let frame_type = state_param_types
                .get(&(state_name.to_string(), name.clone()))
                .map(|s| s.as_str())
                .unwrap_or("int");
            let java_type = java_map_type(frame_type);
            let src = format!("compartment.state_args.get({})", i);
            body.push_str(&format!(
                "{} {} = {};\n",
                java_type,
                name,
                java_extract(&java_type, &src)
            ));
        }
    }

    // Handler-param binding.
    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        let type_str = param.symbol_type.as_deref().unwrap_or("int");
        let java_type = java_map_type(type_str);
        let src = format!("{}.get({})", param_source, i);
        body.push_str(&format!(
            "{} {} = {};\n",
            java_type,
            param.name,
            java_extract(&java_type, &src)
        ));
    }

    // State-var init (lifecycle enter only).
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if (!compartment.state_vars.containsKey(\"{}\")) {{\n    compartment.state_vars.put(\"{}\", {});\n}}\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    let event_type = format!("{}FrameEvent", system_name);
    let comp_type = format!("{}Compartment", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single Kotlin handler method under the per-handler contract:
///   private fun _s_<State>_hdl_<kind>_<event>(__e: SysFrameEvent, compartment: SysCompartment)
/// Kotlin specifics: `val x = compartment.state_args[i] as Int`, no semicolons,
/// `containsKey("x")` init guard.
#[allow(clippy::too_many_arguments)]
fn generate_kotlin_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Kotlin;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding. Kotlin uses `val name = compartment.state_args[i] as T`.
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — `val` can't
            // be redeclared in the same Kotlin scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            let frame_type = state_param_types
                .get(&(state_name.to_string(), name.clone()))
                .map(|s| s.as_str())
                .unwrap_or("int");
            let kt_type = kotlin_map_type(frame_type);
            // Number-ladder unwrap (D8) — see fmt_bind_param.
            let extract = match kt_type.as_str() {
                "Double" => format!("(compartment.state_args[{i}] as Number).toDouble()"),
                "Float" => format!("(compartment.state_args[{i}] as Number).toFloat()"),
                "Long" => format!("(compartment.state_args[{i}] as Number).toLong()"),
                "Int" => format!("(compartment.state_args[{i}] as Number).toInt()"),
                _ => format!("compartment.state_args[{i}] as {kt_type}"),
            };
            body.push_str(&format!("val {name} = {extract}\n"));
        }
    }

    // Handler-param binding.
    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        let type_str = param.symbol_type.as_deref().unwrap_or("int");
        let kt_type = kotlin_map_type(type_str);
        body.push_str(&format!(
            "val {} = {}[{}] as {}\n",
            param.name, param_source, i, kt_type
        ));
    }

    // State-var init (lifecycle enter only).
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if (!compartment.state_vars.containsKey(\"{}\")) {{\n    compartment.state_vars[\"{}\"] = {}\n}}\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    let event_type = format!("{}FrameEvent", system_name);
    let comp_type = format!("{}Compartment", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single Swift handler method under the per-handler contract:
///   private func _s_<State>_hdl_<kind>_<event>(_ __e: SysFrameEvent, _ compartment: SysCompartment)
/// Swift specifics: `let x = compartment.state_args[i] as! Int` typed
/// force-cast, `keys.contains("x")` init guard.
#[allow(clippy::too_many_arguments)]
fn generate_swift_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Swift;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding. Swift uses `let name = compartment.state_args[i] as! T`.
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — Swift's
            // `let` can't be redeclared in the same scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            let frame_type = state_param_types
                .get(&(state_name.to_string(), name.clone()))
                .map(|s| s.as_str())
                .unwrap_or("int");
            let sw_type = swift_map_type(frame_type);
            // NSNumber-ladder unwrap (D8) — see fmt_bind_param.
            let extract = match sw_type.as_str() {
                "Double" => format!("(compartment.state_args[{i}] as! NSNumber).doubleValue"),
                "Float" => format!("(compartment.state_args[{i}] as! NSNumber).floatValue"),
                "Int" => format!("(compartment.state_args[{i}] as! NSNumber).intValue"),
                "Int64" => format!("(compartment.state_args[{i}] as! NSNumber).int64Value"),
                _ => format!("compartment.state_args[{i}] as! {sw_type}"),
            };
            body.push_str(&format!("let {name} = {extract}\n"));
        }
    }

    // Handler-param binding.
    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        let type_str = param.symbol_type.as_deref().unwrap_or("int");
        let sw_type = swift_map_type(type_str);
        body.push_str(&format!(
            "let {} = {}[{}] as! {}\n",
            param.name, param_source, i, sw_type
        ));
    }

    // State-var init (lifecycle enter only).
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if compartment.state_vars[\"{}\"] == nil {{\n    compartment.state_vars[\"{}\"] = {}\n}}\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    let event_type = format!("{}FrameEvent", system_name);
    let comp_type = format!("{}Compartment", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single C++ handler method under the per-handler contract:
///   void _s_<State>_hdl_<kind>_<event>(SysFrameEvent& __e, std::shared_ptr<SysCompartment> compartment)
/// C++ specifics: `std::any_cast<T>(compartment->state_args[i])` typed cast,
/// `.count("x")` init guard, shared_ptr dereference with `->`.
#[allow(clippy::too_many_arguments)]
fn generate_cpp_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Cpp;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding. C++ uses `auto name = std::any_cast<T>(compartment->state_args[i]);`.
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — C++
            // disallows variable redeclaration in the same scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            let frame_type = state_param_types
                .get(&(state_name.to_string(), name.clone()))
                .map(|s| s.as_str())
                .unwrap_or("int");
            let cpp_type = cpp_map_type(frame_type);
            // Untyped params land as `std::any`; copy without cast.
            if cpp_type == "std::any" {
                body.push_str(&format!(
                    "auto {} = compartment->state_args[{}];\n",
                    name, i
                ));
            } else {
                body.push_str(&format!(
                    "{} {} = std::any_cast<{}>(compartment->state_args[{}]);\n",
                    cpp_type, name, cpp_type, i
                ));
            }
        }
    }

    // Handler-param binding. When the declared type maps to `std::any`
    // (i.e., untyped params), copy directly without a cast — the same
    // carve-out fmt_unpack applies for the legacy monolithic path.
    let param_source = if handler.is_enter {
        "compartment->enter_args"
    } else if handler.is_exit {
        "compartment->exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        let type_str = param.symbol_type.as_deref().unwrap_or("int");
        let cpp_type = cpp_map_type(type_str);
        if cpp_type == "std::any" {
            body.push_str(&format!("auto {} = {}[{}];\n", param.name, param_source, i));
        } else {
            body.push_str(&format!(
                "auto {} = std::any_cast<{}>({}[{}]);\n",
                param.name, cpp_type, param_source, i
            ));
        }
    }

    // State-var init (lifecycle enter only). String literals get wrapped
    // in `std::string(...)` so the any holds std::string, not const char* —
    // matches fmt_init_sv's existing carve-out in the legacy path.
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            let wrapped = if init_val.trim().starts_with('"') && init_val.trim().ends_with('"') {
                format!("std::string({})", init_val)
            } else {
                init_val
            };
            body.push_str(&format!(
                "if (compartment->state_vars.count(\"{}\") == 0) {{\n    compartment->state_vars[\"{}\"] = {};\n}}\n",
                var.name, var.name, wrapped
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    let event_type = format!("{}FrameEvent&", system_name);
    let comp_type = format!("std::shared_ptr<{}Compartment>", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single C# handler method under the per-handler contract:
///   private void _s_<State>_hdl_<kind>_<event>(SysFrameEvent __e, SysCompartment compartment)
/// C# specifics: `(T) compartment.state_args[i]` typed cast, `.ContainsKey("x")`
/// init guard.
#[allow(clippy::too_many_arguments)]
fn generate_csharp_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::CSharp;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding. C# uses `var name = (T) compartment.state_args[i];`.
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — C# disallows
            // variable redeclaration in the same scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            let frame_type = state_param_types
                .get(&(state_name.to_string(), name.clone()))
                .map(|s| s.as_str())
                .unwrap_or("int");
            let cs_type = csharp_map_type(frame_type);
            // Convert.ToXxx ladder (D8) — see fmt_bind_param.
            let cs_extract = |t: &str, src: &str| -> String {
                match t {
                    "double" => format!("System.Convert.ToDouble({src})"),
                    "float" => format!("System.Convert.ToSingle({src})"),
                    "int" => format!("System.Convert.ToInt32({src})"),
                    "long" => format!("System.Convert.ToInt64({src})"),
                    _ => format!("({t}) {src}"),
                }
            };
            let src = format!("compartment.state_args[{i}]");
            body.push_str(&format!(
                "{cs_type} {name} = {};\n",
                cs_extract(&cs_type, &src)
            ));
        }
    }

    // Handler-param binding. Convert.ToXxx ladder (D8) — handler args
    // come from typed primitive boxing, but interface_gen's @@persist
    // path can also hand back JSON-decoded numbers via the same list.
    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    let cs_extract2 = |t: &str, src: &str| -> String {
        match t {
            "double" => format!("System.Convert.ToDouble({src})"),
            "float" => format!("System.Convert.ToSingle({src})"),
            "int" => format!("System.Convert.ToInt32({src})"),
            "long" => format!("System.Convert.ToInt64({src})"),
            _ => format!("({t}) {src}"),
        }
    };
    for (i, param) in handler.params.iter().enumerate() {
        let type_str = param.symbol_type.as_deref().unwrap_or("int");
        let cs_type = csharp_map_type(type_str);
        let src = format!("{param_source}[{i}]");
        body.push_str(&format!(
            "{cs_type} {} = {};\n",
            param.name,
            cs_extract2(&cs_type, &src)
        ));
    }

    // State-var init (lifecycle enter only).
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if (!compartment.state_vars.ContainsKey(\"{}\")) {{\n    compartment.state_vars[\"{}\"] = {};\n}}\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    let event_type = format!("{}FrameEvent", system_name);
    let comp_type = format!("{}Compartment", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single C handler method under the per-handler contract:
///   static void Sys__s_<State>_hdl_<kind>_<event>(Sys* self, Sys_FrameEvent* __e, Sys_Compartment* compartment)
/// C specifics: free functions (not class methods), `(T) FrameDict_get(...)`
/// typed cast for state-var access, `->` deref, C backend prefixes
/// `Sys_` and inserts `self` automatically.
#[allow(clippy::too_many_arguments)]
fn generate_c_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::C;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: state_param_types.clone(),
    };

    let mut body = String::new();

    // State-param / handler-param binding from a void* slot. Maps Frame
    // types to native C types so `str` → `const char*`, `bool` → `int`,
    // `float`/`double` → `double` (via the runtime's `Sys_unpack_double`
    // bit-pun helper, since `(intptr_t)(0.5)` truncates), etc. Falls
    // back to `int` for unknown types — matches `DispatchSyntax::C`'s
    // `c_param_type_and_cast` helper, extended for floats.
    //
    // Returns (decl_type, full_extract_expr) where the extractor takes
    // the void* value placeholder `{val}`.
    let c_extract = |type_str: &str, val_expr: &str| -> (String, String) {
        let t = type_str.trim();
        match t {
            "str" | "string" | "String" | "char*" | "const char*" => (
                "const char*".to_string(),
                format!("(const char*){}", val_expr),
            ),
            "float" | "f32" | "f64" | "double" => (
                "double".to_string(),
                format!("{}_unpack_double({})", system_name, val_expr),
            ),
            // Frame's `: list` lands as <sys>_FrameVec* (see backends/c.rs
            // convert_type_to_c). State-args/event-args of list type
            // need the typed cast, not the int fallthrough.
            "list" | "List" | "Array" | "Array<any>" => {
                let typ = format!("{}_FrameVec*", system_name);
                let extract = format!("({}){}", typ, val_expr);
                (typ, extract)
            }
            // Same shape for `: dict` → <sys>_FrameDict*.
            "dict" | "Dict" | "Record<string, any>" => {
                let typ = format!("{}_FrameDict*", system_name);
                let extract = format!("({}){}", typ, val_expr);
                (typ, extract)
            }
            _ if t.ends_with('*') => (t.to_string(), format!("({}){}", t, val_expr)),
            _ => (
                "int".to_string(),
                format!("(int)(intptr_t){}", val_expr),
            ),
        }
    };
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Dynamic languages let the handler-param prefetch reassign
            // an outer `name`; C disallows redeclaration. When a handler
            // param shadows a state-arg, skip the state-arg prefetch and
            // let the param binding take over.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            let type_str = state_param_types
                .get(&(state_name.to_string(), name.clone()))
                .map(|s| s.as_str())
                .unwrap_or("int");
            let val_expr = format!(
                "{}_FrameVec_get(compartment->state_args, {})",
                system_name, i
            );
            let (c_type, extract) = c_extract(type_str, &val_expr);
            body.push_str(&format!("{} {} = {};\n", c_type, name, extract));
            body.push_str(&format!("(void){};\n", name));
        }
    }

    // Handler-param binding. Reuse the c_extract helper from above so
    // float/double params route through `Sys_unpack_double` (intptr_t
    // cast truncates floats — same root issue as state-args).
    let param_source_pre = if handler.is_enter {
        "compartment->enter_args"
    } else if handler.is_exit {
        "compartment->exit_args"
    } else {
        "__e->_parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        let type_str = param.symbol_type.as_deref().unwrap_or("int");
        let val_expr = format!(
            "{}_FrameVec_get({}, {})",
            system_name, param_source_pre, i
        );
        let (c_decl, extract) = c_extract(type_str, &val_expr);
        body.push_str(&format!("{} {} = {};\n", c_decl, param.name, extract));
        body.push_str(&format!("(void){};\n", param.name));
    }

    // State-var init (lifecycle enter only). Uses FrameDict_has + FrameDict_set.
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if (!{}_FrameDict_has(compartment->state_vars, \"{}\")) {{\n    {}_FrameDict_set(compartment->state_vars, \"{}\", (void*)(intptr_t)({}));\n}}\n",
                system_name, var.name, system_name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    let event_type = format!("{}_FrameEvent*", system_name);
    let comp_type = format!("{}_Compartment*", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single Dart handler method under the per-handler contract.
/// Dart: `final x = …;` param bindings with explicit type casts,
/// `containsKey("x")` init guard, null-safe compartment handling.
fn generate_dart_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    _state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::Dart;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding.
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — Dart's
            // `final` can't be redeclared in the same scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            body.push_str(&format!(
                "final {} = compartment.state_args[{}];\n",
                name, i
            ));
        }
    }

    // Handler-param binding with type cast to keep Dart's strict typing happy.
    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        let type_str = param.symbol_type.as_deref().unwrap_or("");
        let dart_type = match type_str {
            "int" | "number" | "num" => "int",
            "double" | "float" => "double",
            "bool" | "boolean" => "bool",
            "String" | "str" | "string" => "String",
            _ => "",
        };
        if dart_type.is_empty() {
            body.push_str(&format!(
                "final {} = {}[{}];\n",
                param.name, param_source, i
            ));
        } else {
            body.push_str(&format!(
                "final {} = {}[{}] as {};\n",
                param.name, param_source, i, dart_type
            ));
        }
    }

    // State-var init (lifecycle enter only).
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if (!compartment.state_vars.containsKey(\"{}\")) {{\n    compartment.state_vars[\"{}\"] = {};\n}}\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    let event_type = format!("{}FrameEvent", system_name);
    let comp_type = format!("{}Compartment", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single GDScript handler method under the per-handler contract:
///   func _s_<State>_hdl_<kind>_<event>(__e, compartment):
/// Uses GDScript syntax for declarations and the Python-like `if not in`
/// state-var init guard.
fn generate_gdscript_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
    _state_param_types: &std::collections::HashMap<(String, String), String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);
    let lang = TargetLanguage::GDScript;

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — GDScript's
            // `var` can't be redeclared in the same scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            body.push_str(&format!("var {} = compartment.state_args[{}]\n", name, i));
        }
    }

    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        body.push_str(&format!("var {} = {}[{}]\n", param.name, param_source, i));
    }

    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if not \"{}\" in compartment.state_vars:\n    compartment.state_vars[\"{}\"] = {}\n",
                var.name, var.name, init_val
            ));
        }
    }

    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    if body.trim().is_empty() {
        body.push_str("pass\n");
    }

    CodegenNode::Method {
        name: method_name,
        params: vec![Param::new("__e"), Param::new("compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single TypeScript/JavaScript handler method under the per-
/// handler contract: `_s_<State>_hdl_<kind>_<event>(__e, compartment)`.
/// Body layout mirrors `generate_python_handler_method` but with TS/JS
/// syntax for param binding, state-var init guard, and statement
/// terminators.
fn generate_typescript_handler_method(
    lang: TargetLanguage,
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding. State params (declared via `$State(a, b)`) flow
    // through `compartment.state_args` — bind them to named locals at the
    // top of every handler so handler bodies can reference them by name.
    if let Some(sp_names) = state_param_names.get(state_name) {
        let handler_param_names: std::collections::HashSet<&str> =
            handler.params.iter().map(|p| p.name.as_str()).collect();
        for (i, name) in sp_names.iter().enumerate() {
            // Skip when a handler param shadows a state-arg — JS/TS
            // `const` can't be redeclared in the same scope.
            if handler_param_names.contains(name.as_str()) {
                continue;
            }
            body.push_str(&format!(
                "const {} = compartment.state_args[{}];\n",
                name, i
            ));
        }
    }

    // Param binding. Lifecycle handlers read from compartment.enter_args /
    // exit_args; user-method handlers read from __e._parameters.
    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        body.push_str(&format!(
            "const {} = {}[{}];\n",
            param.name, param_source, i
        ));
    }

    // State-var initialization — only the lifecycle `$>` handler. The
    // `if not in` guard preserves pop$ restore semantics.
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            body.push_str(&format!(
                "if (!(\"{}\" in compartment.state_vars)) {{\n    compartment.state_vars[\"{}\"] = {};\n}}\n",
                var.name, var.name, init_val
            ));
        }
    }

    // Return-init (declared via handler `: Type = default`).
    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    // User-written handler body. Frame expansion uses ctx.per_handler=true,
    // so state-var access emits compartment.state_vars[…] and HSM forwards
    // emit this._state_Parent(__e, compartment.parent_compartment).
    let body_src = emit_handler_body_via_statements(&handler.body_span, source, lang, &ctx);
    body.push_str(&body_src);

    // Method params: __e: <Sys>FrameEvent, compartment: <Sys>Compartment.
    let event_type = format!("{}FrameEvent", system_name);
    let comp_type = format!("{}Compartment", system_name);

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Emit a single Python handler method with the per-handler contract:
///   def _s_<State>_hdl_<kind>_<event>(self, __e, compartment):
/// Body layout:
///   1. Param binding from __e._parameters (user methods) or
///      compartment.enter_args / compartment.exit_args (lifecycle).
///   2. State-var init preamble (lifecycle enter only — guarded `if not in`).
///   3. Return-init assignment (if handler declares one).
///   4. User-written handler body, expanded with per_handler: true so
///      state-var access targets `compartment.state_vars[…]`.
fn generate_python_handler_method(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    state_vars_for_init: &[StateVarAst],
    source: &[u8],
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    _sys_param_locals: &[String],
    _is_start_state: bool,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
) -> CodegenNode {
    let method_name = handler_method_name(state_name, handler);

    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false,
        per_handler: true,
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    let mut body = String::new();

    // State-param binding. State params (declared via `$State(a, b)`) flow
    // through `compartment.state_args` — bind them to named locals at the
    // top of every handler so handler bodies can reference them by name.
    if let Some(sp_names) = state_param_names.get(state_name) {
        for (i, name) in sp_names.iter().enumerate() {
            body.push_str(&format!("{} = compartment.state_args[{}]\n", name, i));
        }
    }

    // Param binding. Lifecycle handlers read from compartment.enter_args /
    // exit_args (set by the transition codegen on the target/source
    // compartment). User-method handlers read from __e._parameters.
    let param_source = if handler.is_enter {
        "compartment.enter_args"
    } else if handler.is_exit {
        "compartment.exit_args"
    } else {
        "__e._parameters"
    };
    for (i, param) in handler.params.iter().enumerate() {
        body.push_str(&format!("{} = {}[{}]\n", param.name, param_source, i));
    }

    // State-var initialization — only the lifecycle `$>` handler does this.
    // The `if not in` guard preserves pop$ restore semantics.
    if handler.is_enter {
        for var in state_vars_for_init {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Python3)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Python3)
            };
            body.push_str(&format!(
                "if \"{}\" not in compartment.state_vars:\n    compartment.state_vars[\"{}\"] = {}\n",
                var.name, var.name, init_val
            ));
        }
    }

    // Return-init (declared via handler `: Type = default`).
    let return_init_code =
        emit_handler_return_init(handler, TargetLanguage::Python3, "", &ctx.system_name);
    if !return_init_code.is_empty() {
        body.push_str(&return_init_code);
    }

    // User-written handler body. Frame expansion uses ctx.per_handler=true,
    // so state-var access emits compartment.state_vars[…] and HSM forwards
    // emit self._state_Parent(__e, compartment.parent_compartment).
    let body_src =
        emit_handler_body_via_statements(&handler.body_span, source, TargetLanguage::Python3, &ctx);
    body.push_str(&body_src);

    // Empty body placeholder — Python requires a statement.
    if body.trim().is_empty() {
        body.push_str("pass\n");
    }

    CodegenNode::Method {
        name: method_name,
        params: vec![Param::new("__e"), Param::new("compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Generate a `__state_{StateName}(__e)` method for Python/TypeScript
///
/// The method receives a FrameEvent and dispatches based on __e._message
pub(crate) fn generate_state_method(
    _system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    source: &[u8],
    lang: TargetLanguage,
    _has_state_vars: bool,
    default_forward: bool,
    defined_systems: &std::collections::HashSet<String>,
    is_start_state: bool,
) -> CodegenNode {
    // Use single underscore prefix to avoid Python name mangling
    // Python mangles __name to _ClassName__name, which breaks dynamic lookup
    let method_name = format!("_state_{}", state_name);

    // Build context for HSM forwarding
    // use_sv_comp is true when this state has state vars - we'll navigate to correct compartment
    let state_var_types: std::collections::HashMap<String, String> = state_vars
        .iter()
        .map(|sv| {
            let type_str = match &sv.var_type {
                crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
                crate::frame_c::compiler::frame_ast::Type::Unknown => "int".to_string(),
            };
            (sv.name.clone(), type_str)
        })
        .collect();

    let ctx = HandlerContext {
        system_name: _system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: String::new(), // Will be set per-handler
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: !state_vars.is_empty(),
        // Python migrates to per-handler in the separate handler-method
        // emission path (generate_python_handler_method). The dispatcher's
        // own `ctx` does not need per_handler set — its body either delegates
        // to the thin dispatcher emitter or falls through to the legacy
        // monolithic path for non-Python targets.
        per_handler: false,
        state_var_types,
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        // generate_state_method doesn't have access to the machine AST;
        // the per-handler path uses state_hsm_parents via the dedicated
        // per-handler emitter chain. This ctx is only used for the
        // dispatcher body (thin or monolithic) which doesn't emit
        // transitions. Empty map is safe here.
        state_hsm_parents: std::collections::HashMap::new(),
        current_return_type: None,
            state_param_types: std::collections::HashMap::new(),
    };

    // Generate the dispatch body based on __e._message / __e.message
    // Use unified dispatch for languages that have DispatchSyntax defined.
    let body_code = if matches!(
        lang,
        TargetLanguage::Python3
            | TargetLanguage::TypeScript
            | TargetLanguage::JavaScript
            | TargetLanguage::Ruby
            | TargetLanguage::GDScript
            | TargetLanguage::Lua
            | TargetLanguage::Dart
            | TargetLanguage::Php
            | TargetLanguage::Go
            | TargetLanguage::Java
            | TargetLanguage::Kotlin
            | TargetLanguage::Swift
            | TargetLanguage::Cpp
            | TargetLanguage::CSharp
            | TargetLanguage::C
    ) {
        // Per-handler architecture: the dispatcher body is a flat list of
        // guarded calls to per-handler methods. Handler bodies themselves
        // are emitted separately via `generate_per_handler_methods`.
        let syn = dispatch_syntax_for(lang).expect("DispatchSyntax for per-handler target");
        generate_thin_dispatcher_generic(
            state_name,
            handlers,
            state_params,
            &ctx,
            default_forward,
            !state_vars.is_empty(),
            &syn,
        )
    } else if let Some(syn) = dispatch_syntax_for(lang) {
        generate_unified_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
            &syn,
        )
    } else {
        // Only Rust and Erlang use separate dispatch paths
        match lang {
            TargetLanguage::Rust => super::rust_system::generate_rust_state_dispatch(
                _system_name,
                state_name,
                handlers,
                state_vars,
                parent_state,
                default_forward,
                is_start_state,
            ),
            TargetLanguage::Erlang => String::new(),
            _ => unreachable!("All other languages use unified dispatch"),
        }
    };

    let params = match lang {
        // TypeScript/JavaScript/Dart have migrated to per-handler dispatch —
        // dispatcher takes the active state's compartment as a second
        // param (see docs/frame_runtime.md § "Dispatch Model").
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            let event_type = format!("{}FrameEvent", _system_name);
            let comp_type = format!("{}Compartment", _system_name);
            vec![
                Param::new("__e").with_type(&event_type),
                Param::new("compartment").with_type(&comp_type),
            ]
        }
        TargetLanguage::Dart => {
            let event_type = format!("{}FrameEvent", _system_name);
            let comp_type = format!("{}Compartment", _system_name);
            vec![
                Param::new("__e").with_type(&event_type),
                Param::new("compartment").with_type(&comp_type),
            ]
        }
        TargetLanguage::Rust => {
            let event_type = format!("&{}FrameEvent", _system_name);
            vec![Param::new("__e").with_type(&event_type)]
        }
        TargetLanguage::C => {
            let event_type = format!("{}_FrameEvent*", _system_name);
            let comp_type = format!("{}_Compartment*", _system_name);
            vec![
                Param::new("__e").with_type(&event_type),
                Param::new("compartment").with_type(&comp_type),
            ]
        }
        TargetLanguage::Cpp => {
            let event_type = format!("{}FrameEvent&", _system_name);
            let comp_type = format!("std::shared_ptr<{}Compartment>", _system_name);
            vec![
                Param::new("__e").with_type(&event_type),
                Param::new("compartment").with_type(&comp_type),
            ]
        }
        TargetLanguage::Java | TargetLanguage::Kotlin | TargetLanguage::Swift => {
            let event_type = format!("{}FrameEvent", _system_name);
            let comp_type = format!("{}Compartment", _system_name);
            vec![
                Param::new("__e").with_type(&event_type),
                Param::new("compartment").with_type(&comp_type),
            ]
        }
        TargetLanguage::CSharp => {
            let event_type = format!("{}FrameEvent", _system_name);
            let comp_type = format!("{}Compartment", _system_name);
            vec![
                Param::new("__e").with_type(&event_type),
                Param::new("compartment").with_type(&comp_type),
            ]
        }
        TargetLanguage::Go => {
            let event_type = format!("*{}FrameEvent", _system_name);
            let comp_type = format!("*{}Compartment", _system_name);
            vec![
                Param::new("__e").with_type(&event_type),
                Param::new("compartment").with_type(&comp_type),
            ]
        }
        // Per-handler architecture: dispatcher takes the active state's
        // compartment as a second param (see docs/frame_runtime.md §
        // "Dispatch Model"). Other dynamic languages still use monolithic
        // dispatch for now.
        TargetLanguage::Python3
        | TargetLanguage::Ruby
        | TargetLanguage::GDScript
        | TargetLanguage::Lua
        | TargetLanguage::Php => {
            vec![Param::new("__e"), Param::new("compartment")]
        }
        // Dynamic languages: untyped event parameter
        TargetLanguage::Erlang => {
            vec![Param::new("__e")]
        }
        TargetLanguage::Graphviz => unreachable!(),
    };

    CodegenNode::Method {
        name: method_name,
        params,
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: body_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Lua state dispatch — if/elseif/then/end chain
pub(crate) fn generate_handler_from_arcanum(
    system_name: &str,
    state_name: &str,
    parent_state: Option<&str>,
    handler: &HandlerEntry,
    source: &[u8],
    lang: TargetLanguage,
    _has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    sys_param_locals: &[String],
    is_start_state: bool,
    non_start_state_param_names: &[String],
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
    event_param_names: &std::collections::HashMap<String, Vec<String>>,
    handler_state_var_types: &std::collections::HashMap<String, String>,
    state_hsm_parents: &std::collections::HashMap<String, String>,
) -> CodegenNode {
    // Build params from handler's parameter symbols
    // V4 uses native types, so we just pass them through as-is
    // For Rust: Add __e: &FrameEvent as first param
    let mut params: Vec<Param> = Vec::new();

    if matches!(lang, TargetLanguage::Rust) {
        // Rust handlers receive the FrameEvent reference
        let event_type = format!("&{}FrameEvent", system_name);
        params.push(Param::new("__e").with_type(&event_type));
    }

    // Add handler parameters — for Rust, the START STATE'S lifecycle
    // handlers ($>, $<) bind their params from `self.__sys_<name>` in
    // the body preamble (the constructor populates these from the
    // system header params), so we drop them from the signature. For
    // non-start state lifecycle handlers and all interface handlers,
    // declared params stay in the signature.
    let skip_handler_params = matches!(lang, TargetLanguage::Rust)
        && (handler.is_enter || handler.is_exit)
        && is_start_state;
    if !skip_handler_params {
        for p in &handler.params {
            let type_str = p.symbol_type.as_deref().unwrap_or("Any");
            // Clean up the type string (remove "Some(" prefix if present from debug format)
            let clean_type = if type_str.starts_with("Some(") {
                type_str.trim_start_matches("Some(").trim_end_matches(")")
            } else {
                type_str
            };
            params.push(Param::new(&p.name).with_type(clean_type));
        }
    }

    let method_name = handler_method_name(state_name, handler);

    // Build context for HSM forwarding. The state_param_names /
    // state_enter_param_names / state_exit_param_names maps are
    // populated from the caller so that the transition codegen inside
    // the handler body can resolve `state_args[i]` /
    // `enter_args[i]` / `exit_args[i]` to declared param names.
    // Without this, Rust's typed enum-of-structs StateContext would
    // emit `ctx.0 = val` (positional) instead of `ctx.initial = val`.
    let ctx = HandlerContext {
        system_name: system_name.to_string(),
        state_name: state_name.to_string(),
        event_name: handler.event.clone(),
        parent_state: parent_state.map(|s| s.to_string()),
        defined_systems: defined_systems.clone(),
        use_sv_comp: false, // Handler-specific methods don't have __sv_comp preamble
        per_handler: false, // Rust uses typed struct fields, not compartment param
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
        event_param_names: event_param_names.clone(),
        // Used by Rust transition emission to walk the destination HSM
        // chain and propagate state-args through every layer's typed
        // StateContext variant (per docs/frame_runtime.md Step 22's
        // signature-match rule). The map is also useful for any
        // future propagation step that needs ancestor lookup.
        state_hsm_parents: state_hsm_parents.clone(),
        current_return_type: handler.return_type.clone(),
        state_param_types: std::collections::HashMap::new(),
    };

    // Emit handler default return value if present
    let return_init_code = emit_handler_return_init(handler, lang, "", &ctx.system_name);

    // Rust: bind state params (declared on the start state via
    // `$Start(x: int)`) and start-state enter args (`$>(b: int)`) to
    // bare locals at the top of the handler. The constructor populates
    // `self.__sys_<name>` from the system header params for the start
    // state only. Non-start states with declared state params bind from
    // the typed `self.__compartment.state_context::<State>(ref ctx)`
    // variant — populated by transition codegen via the typed pattern
    // match in `frame_expansion.rs`.
    let mut sys_param_preamble = String::new();
    if matches!(lang, TargetLanguage::Rust) {
        if is_start_state {
            for name in sys_param_locals {
                sys_param_preamble.push_str(&format!("let {0} = self.__sys_{0}.clone();\n", name));
            }
            // Also bind any enter handler params from `self.__sys_<name>`.
            if handler.is_enter {
                for p in &handler.params {
                    sys_param_preamble
                        .push_str(&format!("let {0} = self.__sys_{0}.clone();\n", p.name));
                }
            }
        } else {
            // Non-start state with declared state params: walk the
            // HSM compartment chain to find the layer matching this
            // handler's owner state, then pattern-match its typed
            // StateContext variant. The walk is required because
            // self.__compartment is the *leaf* compartment — when a
            // cascade or `=> $^` fall-through fires this handler in
            // an ancestor state, that ancestor's compartment is up
            // the parent_compartment chain, not at the leaf. Reading
            // self.__compartment.state_context directly was the
            // bug: ancestor variants never matched and the binding
            // silently fell back to Default::default().
            //
            // Pattern syntax note: `match &__sc.state_context` makes
            // the scrutinee a reference, so the inner binding `ctx`
            // is auto-borrowed — using `ref ctx` here is rejected by
            // recent rustc as "cannot explicitly borrow within an
            // implicitly-borrowing pattern".
            let emit_walk = |sb: &mut String, name: &str, owner: &str| {
                sb.push_str(&format!(
                    concat!(
                        "let {0} = {{\n",
                        "    let mut __sc = &self.__compartment;\n",
                        "    while __sc.state != \"{2}\" {{\n",
                        "        match __sc.parent_compartment.as_deref() {{\n",
                        "            Some(p) => __sc = p,\n",
                        "            None => break,\n",
                        "        }}\n",
                        "    }}\n",
                        "    match &__sc.state_context {{\n",
                        "        {1}StateContext::{2}(ctx) => ctx.{0}.clone(),\n",
                        "        _ => Default::default(),\n",
                        "    }}\n",
                        "}};\n"
                    ),
                    name, system_name, owner
                ));
            };
            // Skip prefetch when a handler param shadows a state-arg of
            // the same name — Rust allows shadowing but the prefetch
            // would clobber the handler-supplied value with the OLD
            // compartment value, breaking writes-back-via-transition
            // and any handler that just consumes the param verbatim.
            let handler_param_names: std::collections::HashSet<&str> =
                handler.params.iter().map(|p| p.name.as_str()).collect();
            // 1. Own params — owner is self.
            for name in non_start_state_param_names {
                if handler_param_names.contains(name.as_str()) {
                    continue;
                }
                emit_walk(&mut sys_param_preamble, name, state_name);
            }
            // 2. Cascade-inherited params — declared at a descendant's
            // cascade arrow `$Descendant => $Self(name: T)`. The runtime
            // stores the value on the descendant's typed StateContext
            // variant (the leaf), so the walk targets the descendant.
            // Only emit for descendants in this state's HSM subtree.
            let already: std::collections::HashSet<&str> = non_start_state_param_names
                .iter()
                .map(|s| s.as_str())
                .collect();
            for descendant in state_hsm_parents.keys() {
                let mut cursor = state_hsm_parents.get(descendant);
                let mut on_chain = false;
                while let Some(p) = cursor {
                    if p == state_name {
                        on_chain = true;
                        break;
                    }
                    cursor = state_hsm_parents.get(p);
                }
                if !on_chain {
                    continue;
                }
                if let Some(descendant_params) = state_param_names.get(descendant) {
                    for p in descendant_params {
                        if already.contains(p.as_str()) || handler_param_names.contains(p.as_str())
                        {
                            continue;
                        }
                        emit_walk(&mut sys_param_preamble, p, descendant);
                    }
                }
            }
        }
    }

    // Splice the handler body: preserve native code, expand Frame segments
    let mut body_code = sys_param_preamble;
    body_code.push_str(&return_init_code);
    body_code.push_str(&emit_handler_body_via_statements(
        &handler.body_span,
        source,
        lang,
        &ctx,
    ));

    // Handler methods are void — returns go through the context stack.
    // Some languages strip the return type from the handler signature.
    let method_return_type = match lang {
        // These languages don't use return types on state handler methods
        TargetLanguage::TypeScript
        | TargetLanguage::Dart
        | TargetLanguage::JavaScript
        | TargetLanguage::Rust => None,
        // Dynamic languages don't need return type annotations
        TargetLanguage::Python3
        | TargetLanguage::GDScript
        | TargetLanguage::Ruby
        | TargetLanguage::Lua => None,
        // All others use the declared return type
        TargetLanguage::C
        | TargetLanguage::Cpp
        | TargetLanguage::Java
        | TargetLanguage::CSharp
        | TargetLanguage::Go
        | TargetLanguage::Php
        | TargetLanguage::Kotlin
        | TargetLanguage::Swift
        | TargetLanguage::Erlang => handler.return_type.clone(),
        TargetLanguage::Graphviz => unreachable!(),
    };

    CodegenNode::Method {
        name: method_name,
        params,
        return_type: method_return_type,
        body: vec![CodegenNode::NativeBlock {
            code: body_code,
            span: Some(crate::frame_c::compiler::frame_ast::Span {
                start: handler.body_span.start,
                end: handler.body_span.end,
            }),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}
