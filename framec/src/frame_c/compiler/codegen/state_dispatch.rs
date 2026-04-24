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
                format!("local {name} = self.__compartment.state_args[{}]\n", index + 1)
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
                format!("{cs_type} {name} = ({cs_type}) __compartment.state_args[{index}];\n")
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
                format!("{indent}var {name} = ({cs_type}) {list}[{index}];\n")
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
                format!("{java_type} {name} = ({java_type}) __compartment.state_args.get({index});\n")
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
                format!("{indent}var {name} = ({java_type}) {list}.get({index});\n")
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
                format!("val {name} = __compartment.state_args[{index}] as {kt_type}\n")
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
                format!("{indent}val {name} = {list}[{index}] as {kt_type}\n")
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
                format!("let {name} = __compartment.state_args[{index}] as! {sw_type}\n")
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
                format!("{indent}let {name} = {list}[{index}] as! {sw_type}\n")
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
                    format!("{indent}{cpp_type} {name} = std::any_cast<{cpp_type}>({list}[{index}]);\n")
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
            fn c_param_type_and_cast(type_str: &str) -> (String, String) {
                let t = type_str.trim();
                match t {
                    "str" | "string" | "String" | "char*" | "const char*" => {
                        ("const char*".to_string(), "(const char*)".to_string())
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
            fmt_elif: |msg| format!("}} else if (strcmp(__e->_message, \"{}\") == 0) {{\n", msg),
            fmt_hsm_nav: |state, sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
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
            fmt_bind_param: |name, type_str, _sys, index| {
                let (c_type, cast) = c_param_type_and_cast(type_str);
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
            fmt_unpack: |name, type_str, indent, _sys, source, _default, index| {
                let list = match source {
                    "enter" => "self->__compartment->enter_args",
                    "exit" => "self->__compartment->exit_args",
                    _ => "__e->_parameters",
                };
                let (c_type, cast) = c_param_type_and_cast(type_str);
                // _parameters / enter_args / exit_args are FrameVec*; dereference ->items[N].
                format!(
                    "{indent}{c_type} {name} = {cast}{list}->items[{index}];\n"
                )
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
        code.push_str(&format!(
            "{indent}{self_prefix}{method}(__e, compartment){semi}\n"
        ));
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
        code.push_str(&format!(
            "{indent}{self_prefix}{method_name}(__e, compartment){semi}\n"
        ));
        code.push_str(&format!("{indent}return{semi}\n"));
        code.push_str(close);
    }

    // Default-forward trailing call — emitted only when the state
    // declares `=> $^`. The forward shifts `compartment` up one level
    // (see docs/frame_runtime.md § "Parent Forward").
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            code.push_str(&format!(
                "{self_prefix}_state_{}(__e, compartment.parent_compartment){semi}\n",
                parent
            ));
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
    let mut event_param_names: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
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

    // Identify the start state (first state in the machine) so the
    // Rust dispatch can switch on whether this state's lifecycle params
    // are bound from system header (start) or from transitions (non-start).
    let start_state_name_for_dispatch = machine
        .states
        .first()
        .map(|s| s.name.clone())
        .unwrap_or_default();

    // Generate one _state_{StateName} dispatch method per state for ALL languages
    for state_entry in arcanum.get_enhanced_states(system_name) {
        // Find state variables and default_forward for this state from the machine AST
        let state_ast = machine.states.iter().find(|s| s.name == state_entry.name);
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
        TargetLanguage::Python3 | TargetLanguage::TypeScript | TargetLanguage::JavaScript
    ) {
        methods.extend(generate_per_handler_methods(
            lang,
            system_name,
            machine,
            arcanum,
            source,
            has_state_vars,
            &defined_systems,
            &state_param_names,
            &state_enter_param_names,
            &state_exit_param_names,
            &event_param_names,
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
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    let start_state_name = machine
        .states
        .first()
        .map(|s| s.name.clone())
        .unwrap_or_default();

    for state_entry in arcanum.get_enhanced_states(system_name) {
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
                            crate::frame_c::compiler::frame_ast::Type::Unknown => {
                                "int".to_string()
                            }
                        };
                        (sv.name.clone(), type_str)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let state_ast = machine
            .states
            .iter()
            .find(|s| s.name == state_entry.name);
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
            );
            methods.push(method);
        }

        for (_event, handler_entry) in &state_entry.handlers {
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
            );
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
            )
        }
        _ => unreachable!(
            "generate_per_handler_method_for_lang called with non-per-handler target {:?}",
            lang
        ),
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
        current_return_type: handler.return_type.clone(),
    };

    let mut body = String::new();

    // State-param binding. State params (declared via `$State(a, b)`) flow
    // through `compartment.state_args` — bind them to named locals at the
    // top of every handler so handler bodies can reference them by name.
    if let Some(sp_names) = state_param_names.get(state_name) {
        for (i, name) in sp_names.iter().enumerate() {
            body.push_str(&format!("const {} = compartment.state_args[{}];\n", name, i));
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
        current_return_type: handler.return_type.clone(),
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
    let body_src = emit_handler_body_via_statements(
        &handler.body_span,
        source,
        TargetLanguage::Python3,
        &ctx,
    );
    body.push_str(&body_src);

    // Empty body placeholder — Python requires a statement.
    if body.trim().is_empty() {
        body.push_str("pass\n");
    }

    CodegenNode::Method {
        name: method_name,
        params: vec![
            Param::new("__e"),
            Param::new("compartment"),
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
        current_return_type: None,
    };

    // Generate the dispatch body based on __e._message / __e.message
    // Use unified dispatch for languages that have DispatchSyntax defined.
    let body_code = if matches!(
        lang,
        TargetLanguage::Python3 | TargetLanguage::TypeScript | TargetLanguage::JavaScript
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
        // TypeScript/JavaScript have migrated to per-handler dispatch —
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
            vec![Param::new("__e").with_type(&event_type)]
        }
        TargetLanguage::Rust => {
            let event_type = format!("&{}FrameEvent", _system_name);
            vec![Param::new("__e").with_type(&event_type)]
        }
        TargetLanguage::C => {
            let event_type = format!("{}_FrameEvent*", _system_name);
            vec![Param::new("__e").with_type(&event_type)]
        }
        TargetLanguage::Cpp => {
            let event_type = format!("{}FrameEvent&", _system_name);
            vec![Param::new("__e").with_type(&event_type)]
        }
        TargetLanguage::Java
        | TargetLanguage::Kotlin
        | TargetLanguage::CSharp
        | TargetLanguage::Swift => {
            let event_type = format!("{}FrameEvent", _system_name);
            vec![Param::new("__e").with_type(&event_type)]
        }
        TargetLanguage::Go => {
            let event_type = format!("*{}FrameEvent", _system_name);
            vec![Param::new("__e").with_type(&event_type)]
        }
        // Python per-handler architecture: dispatcher takes the active
        // state's compartment as a second param (see docs/frame_runtime.md
        // § "Dispatch Model"). Other dynamic languages still use monolithic
        // dispatch for now.
        TargetLanguage::Python3 => {
            vec![Param::new("__e"), Param::new("compartment")]
        }
        // Dynamic languages: untyped event parameter
        TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::GDScript
        | TargetLanguage::Lua => {
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
        current_return_type: handler.return_type.clone(),
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
        } else if !non_start_state_param_names.is_empty() {
            // Non-start state with declared state params: pattern-match
            // the typed state context and bind each declared param to a
            // local at the top of the handler.
            for name in non_start_state_param_names {
                sys_param_preamble.push_str(&format!(
                    "let {0} = if let {1}StateContext::{2}(ref ctx) = self.__compartment.state_context {{ ctx.{0}.clone() }} else {{ Default::default() }};\n",
                    name, system_name, state_name
                ));
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
