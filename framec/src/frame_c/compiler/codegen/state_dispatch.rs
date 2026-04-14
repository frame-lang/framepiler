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
    get_native_scanner, normalize_indentation, splice_handler_body_from_span,
};
use crate::frame_c::compiler::arcanum::{Arcanum, HandlerEntry};
use crate::frame_c::compiler::frame_ast::{MachineAst, StateVarAst, SystemAst, Type};
use crate::frame_c::visitors::TargetLanguage;

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

    // --- Callbacks for language-specific code fragments ---

    /// First `if` condition matching event message
    pub fmt_if: fn(message: &str) -> String,
    /// Subsequent `elif`/`else if` condition
    pub fmt_elif: fn(message: &str) -> String,
    /// HSM compartment navigation preamble
    pub fmt_hsm_nav: fn(state_name: &str, system_name: &str) -> String,
    /// Bind a state param to a local variable
    pub fmt_bind_param: fn(name: &str, type_str: &str, system_name: &str) -> String,
    /// Check-and-init a state var (inside enter handler or auto-init)
    pub fmt_init_sv: fn(var_name: &str, init_val: &str, indent: &str, system_name: &str) -> String,
    /// Unpack a handler param. `source` is "event" for interface handlers,
    /// "enter" for $> handlers, "exit" for <$ handlers.
    pub fmt_unpack: fn(name: &str, type_str: &str, indent: &str, system_name: &str, source: &str) -> String,
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
            fmt_if: |msg| format!("if __e._message == \"{}\":\n", msg),
            fmt_elif: |msg| format!("elif __e._message == \"{}\":\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("# HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("__sv_comp = self.__compartment\n");
                s.push_str(&format!("while __sv_comp is not None and __sv_comp.state != \"{}\":\n", state));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys| {
                format!("{name} = self.__compartment.state_args.get(\"{name}\")\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if \"{var_name}\" not in __sv_comp.state_vars:\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, _source| {
                format!("{indent}{name} = __e._parameters[\"{name}\"]\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}self._state_{parent}(__e)\n")
            },
        }),
        TargetLanguage::GDScript => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "pass",
            indent: "    ",
            close_final: "",
            else_start: "else:\n",
            fmt_if: |msg| format!("if __e._message == \"{}\":\n", msg),
            fmt_elif: |msg| format!("elif __e._message == \"{}\":\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("# HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = self.__compartment\n");
                s.push_str(&format!("while __sv_comp != null and __sv_comp.state != \"{}\":\n", state));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys| {
                format!("var {name} = self.__compartment.state_args[\"{name}\"]\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if not \"{var_name}\" in __sv_comp.state_vars:\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, _source| {
                format!("{indent}var {name} = __e._parameters[\"{name}\"]\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}self._state_{parent}(__e)\n")
            },
        }),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if (__e._message === \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message === \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("let __sv_comp = this.__compartment;\n");
                s.push_str(&format!("while (__sv_comp !== null && __sv_comp.state !== \"{}\") {{\n", state));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment;\n");
                s.push_str("}\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys| {
                format!("let {name} = this.__compartment.state_args[\"{name}\"];\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!(\"{var_name}\" in __sv_comp.state_vars)) {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, _source| {
                format!("{indent}let {name} = __e._parameters[\"{name}\"];\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}this._state_{parent}(__e);\n")
            },
        }),
        TargetLanguage::Ruby => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "end\n",
            else_start: "else\n",
            fmt_if: |msg| format!("if __e._message == \"{}\"\n", msg),
            fmt_elif: |msg| format!("elsif __e._message == \"{}\"\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("# HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("__sv_comp = @__compartment\n");
                s.push_str(&format!("while __sv_comp != nil && __sv_comp.state != \"{}\"\n", state));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment\n");
                s.push_str("end\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys| {
                format!("{name} = @__compartment.state_args[\"{name}\"]\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if !__sv_comp.state_vars.key?(\"{var_name}\")\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n\
                     {indent}end\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, _source| {
                format!("{indent}{name} = __e._parameters[\"{name}\"]\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}_state_{parent}(__e)\n")
            },
        }),
        TargetLanguage::Lua => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "end\n",
            else_start: "else\n",
            fmt_if: |msg| format!("if __e._message == \"{}\" then\n", msg),
            fmt_elif: |msg| format!("elseif __e._message == \"{}\" then\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("-- HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("local __sv_comp = self.__compartment\n");
                s.push_str(&format!("while __sv_comp ~= nil and __sv_comp.state ~= \"{}\" do\n", state));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment\n");
                s.push_str("end\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys| {
                format!("local {name} = self.__compartment.state_args[\"{name}\"]\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if __sv_comp.state_vars[\"{var_name}\"] == nil then\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n\
                     {indent}end\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, _source| {
                format!("{indent}local {name} = __e._parameters[\"{name}\"]\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}self:_state_{parent}(__e)\n")
            },
        }),
        TargetLanguage::Php => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if ($__e->_message == \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} elseif ($__e->_message == \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("$__sv_comp = $this->__compartment;\n");
                s.push_str(&format!("while ($__sv_comp !== null && $__sv_comp->state !== \"{}\") {{\n", state));
                s.push_str("    $__sv_comp = $__sv_comp->parent_compartment;\n");
                s.push_str("}\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys| {
                format!("${name} = $this->__compartment->state_args[\"{name}\"];\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!isset($__sv_comp->state_vars[\"{var_name}\"])) {{\n\
                     {indent}    $__sv_comp->state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, _source| {
                format!("{indent}${name} = $__e->_parameters[\"{name}\"];\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}$this->_state_{parent}($__e);\n")
            },
        }),
        TargetLanguage::CSharp => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if (__e._message == \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message == \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp_n = __compartment;\n");
                s.push_str(&format!("while (__sv_comp_n != null && __sv_comp_n.state != \"{}\") {{\n", state));
                s.push_str("    __sv_comp_n = __sv_comp_n.parent_compartment;\n");
                s.push_str("}\n");
                s.push_str("var __sv_comp = __sv_comp_n!;\n");
                s
            },
            fmt_bind_param: |name, type_str, _sys| {
                let cs_type = csharp_map_type(type_str);
                format!("{cs_type} {name} = ({cs_type}) __compartment.state_args[\"{name}\"];\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!__sv_comp.state_vars.ContainsKey(\"{var_name}\")) {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source| {
                let cs_type = csharp_map_type(type_str);
                let dict = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                format!("{indent}var {name} = ({cs_type}) {dict}[\"{name}\"];\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}_state_{parent}(__e);\n")
            },
        }),
        TargetLanguage::Java => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if (__e._message.equals(\"{}\")) {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message.equals(\"{}\")) {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = __compartment;\n");
                s.push_str(&format!("while (__sv_comp != null && !__sv_comp.state.equals(\"{}\")) {{ __sv_comp = __sv_comp.parent_compartment; }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys| {
                let java_type = java_map_type(type_str);
                format!("{java_type} {name} = ({java_type}) __compartment.state_args.get(\"{name}\");\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!__sv_comp.state_vars.containsKey(\"{var_name}\")) {{\n\
                     {indent}    __sv_comp.state_vars.put(\"{var_name}\", {init_val});\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source| {
                let java_type = java_map_type(type_str);
                let dict = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                format!("{indent}var {name} = ({java_type}) {dict}.get(\"{name}\");\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}_state_{parent}(__e);\n")
            },
        }),
        TargetLanguage::Kotlin => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if (__e._message == \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message == \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = __compartment\n");
                s.push_str(&format!("while (__sv_comp != null && __sv_comp.state != \"{}\") {{ __sv_comp = __sv_comp.parent_compartment!! }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys| {
                let kt_type = kotlin_map_type(type_str);
                format!("val {name} = __compartment.state_args[\"{name}\"] as {kt_type}\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!__sv_comp.state_vars.containsKey(\"{var_name}\")) {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source| {
                let kt_type = kotlin_map_type(type_str);
                let dict = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                format!("{indent}val {name} = {dict}[\"{name}\"] as {kt_type}\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}_state_{parent}(__e)\n")
            },
        }),
        TargetLanguage::Swift => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if __e._message == \"{}\" {{\n", msg),
            fmt_elif: |msg| format!("}} else if __e._message == \"{}\" {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = __compartment\n");
                s.push_str(&format!("while __sv_comp.state != \"{}\" {{ __sv_comp = __sv_comp.parent_compartment! }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys| {
                let sw_type = swift_map_type(type_str);
                format!("let {name} = __compartment.state_args[\"{name}\"] as! {sw_type}\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if __sv_comp.state_vars[\"{var_name}\"] == nil {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val}\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source| {
                let sw_type = swift_map_type(type_str);
                let dict = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters",
                };
                format!("{indent}let {name} = {dict}[\"{name}\"] as! {sw_type}\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}_state_{parent}(__e)\n")
            },
        }),
        TargetLanguage::Dart => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            // Dart: escape $ in message strings to avoid string interpolation
            fmt_if: |msg| format!("if (__e._message == \"{}\") {{\n", msg.replace('$', "\\$")),
            fmt_elif: |msg| format!("}} else if (__e._message == \"{}\") {{\n", msg.replace('$', "\\$")),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("var __sv_comp = __compartment;\n");
                s.push_str(&format!("while (__sv_comp != null && __sv_comp.state != \"{}\") {{\n", state));
                s.push_str("    __sv_comp = __sv_comp.parent_compartment!;\n");
                s.push_str("}\n");
                s
            },
            fmt_bind_param: |name, _type_str, _sys| {
                format!("var {name} = __compartment.state_args[\"{name}\"];\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (!__sv_comp.state_vars.containsKey(\"{var_name}\")) {{\n\
                     {indent}    __sv_comp.state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, _type_str, indent, _sys, source| {
                let dict = match source {
                    "enter" => "__compartment.enter_args",
                    "exit" => "__compartment.exit_args",
                    _ => "__e._parameters?",
                };
                format!("{indent}final {name} = {dict}[\"{name}\"];\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}_state_{parent}(__e);\n")
            },
        }),
        TargetLanguage::Cpp => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if (__e._message == \"{}\") {{\n", msg),
            fmt_elif: |msg| format!("}} else if (__e._message == \"{}\") {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("auto* __sv_comp = __compartment.get();\n");
                s.push_str(&format!("while (__sv_comp && __sv_comp->state != \"{}\") {{ __sv_comp = __sv_comp->parent_compartment.get(); }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys| {
                let cpp_type = cpp_map_type(type_str);
                format!("{cpp_type} {name} = std::any_cast<{cpp_type}>(__compartment->state_args[\"{name}\"]);\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if (__sv_comp->state_vars.find(\"{var_name}\") == __sv_comp->state_vars.end()) {{\n\
                     {indent}    __sv_comp->state_vars[\"{var_name}\"] = {init_val};\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source| {
                let cpp_type = cpp_map_type(type_str);
                let dict = match source {
                    "enter" => "__compartment->enter_args",
                    "exit" => "__compartment->exit_args",
                    _ => "__e._parameters",
                };
                format!("{indent}{cpp_type} {name} = std::any_cast<{cpp_type}>({dict}[\"{name}\"]);\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}_state_{parent}(__e);\n")
            },
        }),
        TargetLanguage::Go => Some(DispatchSyntax {
            lang,
            semi: "",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if __e._message == \"{}\" {{\n", msg),
            fmt_elif: |msg| format!("}} else if __e._message == \"{}\" {{\n", msg),
            fmt_hsm_nav: |state, _sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str("__sv_comp := s.__compartment\n");
                s.push_str(&format!("for __sv_comp != nil && __sv_comp.state != \"{}\" {{ __sv_comp = __sv_comp.parentCompartment }}\n", state));
                s
            },
            fmt_bind_param: |name, type_str, _sys| {
                let go_type = go_map_type(type_str);
                format!("{name} := s.__compartment.stateArgs[\"{name}\"].({go_type})\n_ = {name}\n")
            },
            fmt_init_sv: |var_name, init_val, indent, _sys| {
                format!(
                    "{indent}if _, ok := __sv_comp.stateVars[\"{var_name}\"]; !ok {{\n\
                     {indent}    __sv_comp.stateVars[\"{var_name}\"] = {init_val}\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, _sys, source| {
                let go_type = go_map_type(type_str);
                let dict = match source {
                    "enter" => "s.__compartment.enterArgs",
                    "exit" => "s.__compartment.exitArgs",
                    _ => "__e._parameters",
                };
                format!("{indent}{name} := {dict}[\"{name}\"].({go_type})\n{indent}_ = {name}\n")
            },
            fmt_forward: |parent, indent, _sys| {
                format!("{indent}s._state_{parent}(__e)\n")
            },
        }),
        TargetLanguage::C => Some(DispatchSyntax {
            lang,
            semi: ";",
            empty_body: "",
            indent: "    ",
            close_final: "}\n",
            else_start: "} else {\n",
            fmt_if: |msg| format!("if (strcmp(__e->_message, \"{}\") == 0) {{\n", msg),
            fmt_elif: |msg| format!("}} else if (strcmp(__e->_message, \"{}\") == 0) {{\n", msg),
            fmt_hsm_nav: |state, sys| {
                let mut s = String::new();
                s.push_str("// HSM: Navigate to this state's compartment for state var access\n");
                s.push_str(&format!("{}_Compartment* __sv_comp = self->__compartment;\n", sys));
                s.push_str(&format!("while (__sv_comp != NULL && strcmp(__sv_comp->state, \"{}\") != 0) {{\n", state));
                s.push_str("    __sv_comp = __sv_comp->parent_compartment;\n");
                s.push_str("}\n");
                s
            },
            fmt_bind_param: |name, type_str, sys| {
                format!("int {name} = (int)(intptr_t){sys}_FrameDict_get(self->__compartment->state_args, \"{name}\");\n")
            },
            fmt_init_sv: |var_name, init_val, indent, sys| {
                format!(
                    "{indent}if (!{sys}_FrameDict_has(__sv_comp->state_vars, \"{var_name}\")) {{\n\
                     {indent}    {sys}_FrameDict_set(__sv_comp->state_vars, \"{var_name}\", (void*)(intptr_t)({init_val}));\n\
                     {indent}}}\n"
                )
            },
            fmt_unpack: |name, type_str, indent, sys, source| {
                let dict = match source {
                    "enter" => format!("self->__compartment->enter_args"),
                    "exit" => format!("self->__compartment->exit_args"),
                    _ => format!("__e->_parameters"),
                };
                format!("{indent}int {name} = (int)(intptr_t){sys}_FrameDict_get({dict}, \"{name}\");\n")
            },
            fmt_forward: |parent, indent, sys| {
                format!("{indent}{sys}_state_{parent}(self, __e);\n")
            },
        }),
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
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // 1. State param binding
    for sp in state_params {
        let type_str = match &sp.param_type {
            Type::Custom(s) => s.as_str(),
            Type::Unknown => "int",
        };
        code.push_str(&(syn.fmt_bind_param)(&sp.name, type_str, system_name));
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
            code.push_str(&(syn.fmt_init_sv)(&var.name, &init_val, syn.indent, system_name));
        }
        // Note: for brace langs, the closing } is handled by the next
        // fmt_elif ("} else if") or the final close_final at the end.
        first = false;
    }

    // 4. Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
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

        // State var init in enter handler
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, syn.lang)
                } else {
                    state_var_init_value(&var.var_type, syn.lang)
                };
                code.push_str(&(syn.fmt_init_sv)(&var.name, &init_val, syn.indent, system_name));
            }
        }

        // Param unpacking — enter/exit handlers read from compartment args,
        // interface handlers read from event._parameters
        let param_source = if event == "$>" || event == "enter" {
            "enter"
        } else if event == "$<" || event == "exit" || event == "<$" {
            "exit"
        } else {
            "event"
        };
        for param in handler.params.iter() {
            let type_str = match &param.symbol_type {
                Some(t) => t.as_str(),
                None => "int",
            };
            code.push_str(&(syn.fmt_unpack)(&param.name, type_str, syn.indent, system_name, param_source));
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
        let body = splice_handler_body_from_span(&handler.body_span, source, syn.lang, &handler_ctx);

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
        TargetLanguage::C => format!("{}{}_CTX(self)->_return = (void*)(intptr_t)({});\n", indent, system_name, init_expr),
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
            source,
            lang,
            has_state_vars,
            default_forward,
            &defined_systems,
            is_start_state,
        );
        methods.push(method);
    }

    // For Rust: Also generate individual handler methods that the dispatch calls
    // (Python/TypeScript inline the handler code in the dispatch method)
    if matches!(lang, TargetLanguage::Rust) {
        // The system header state and enter params are bound to the
        // start state only — the constructor populates `self.__sys_<name>`
        // for each system header param. For non-start states, params
        // come from transitions (the existing pre-system-init mechanism)
        // and are read from `__e.parameters` in the dispatch.
        //
        // We pass the start state's param names as `sys_param_locals`;
        // every other state passes an empty slice and falls back to the
        // existing extraction.
        let start_state_name = machine
            .states
            .first()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let start_state_param_names: Vec<String> = arcanum
            .get_enhanced_states(system_name)
            .iter()
            .find(|s| s.name == start_state_name)
            .map(|s| s.params.iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        for state_entry in arcanum.get_enhanced_states(system_name) {
            let is_start_state = state_entry.name == start_state_name;
            // For non-start states with declared params, build the list of
            // declared param names so the handler preamble can bind from
            // the typed `compartment.state_context::<State>(ref ctx)`.
            let non_start_state_param_names: Vec<String> = if !is_start_state {
                state_entry.params.iter().map(|p| p.name.clone()).collect()
            } else {
                Vec::new()
            };
            // Build state_var_types for this state so the Rust state var
            // expansion can decide whether to add `.clone()` (non-Copy
            // types like String) or not (Copy types like i64, bool).
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
            for (_event, handler_entry) in &state_entry.handlers {
                let empty: Vec<String> = Vec::new();
                let sys_param_locals = if is_start_state {
                    &start_state_param_names
                } else {
                    &empty
                };
                let method = generate_handler_from_arcanum(
                    system_name,
                    &state_entry.name,
                    state_entry.parent.as_deref(),
                    handler_entry,
                    source,
                    lang,
                    has_state_vars,
                    &defined_systems,
                    sys_param_locals,
                    is_start_state,
                    &non_start_state_param_names,
                    &state_param_names,
                    &state_enter_param_names,
                    &state_exit_param_names,
                    &handler_state_var_types,
                );
                methods.push(method);
            }
        }
    }

    methods
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
        state_var_types,
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
    };

    // Generate the dispatch body based on __e._message / __e.message
    // Use unified dispatch for languages that have DispatchSyntax defined.
    let body_code = if let Some(syn) = dispatch_syntax_for(lang) {
        generate_unified_state_dispatch(
            _system_name, state_name, handlers, state_vars, state_params,
            source, &ctx, default_forward, &syn,
        )
    } else {
        // Only Rust and Erlang use separate dispatch paths
        match lang {
        TargetLanguage::Rust => generate_rust_state_dispatch(
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
    } };

    let params = match lang {
        TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
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
        // Dynamic languages: untyped event parameter
        TargetLanguage::Python3
        | TargetLanguage::Php
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

/// Generate Python state dispatch code (if/elif chain on __e._message)
pub(crate) fn generate_rust_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    parent_state: Option<&str>,
    default_forward: bool,
    is_start_state: bool,
) -> String {
    let mut code = String::new();
    code.push_str("match __e.message.as_str() {\n");

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    // Track if we need to initialize state vars in $>
    // (State vars now live on compartment.state_context — no enter-handler init needed)
    let _has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");
    let _needs_state_var_init = !state_vars.is_empty();

    for (event, handler) in sorted_handlers {
        // Map Frame events to their message names
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        // Determine handler method name
        let handler_method = match event.as_str() {
            "$>" | "enter" => format!("_s_{}_enter", state_name),
            "$<" | "exit" => format!("_s_{}_exit", state_name),
            _ => format!("_s_{}_{}", state_name, event),
        };

        // Handle enter/exit handlers with parameters specially. For the
        // start state's lifecycle handlers, the handler reads its params
        // from `self.__sys_<name>` (populated by the constructor from
        // system header params), so the dispatch doesn't extract or pass
        // them. For non-start states, lifecycle params come from
        // transition enter/exit args via the existing mechanism, so we
        // restore the original extraction-and-pass-as-arg path.
        let is_lifecycle =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        if !handler.params.is_empty() && is_lifecycle {
            if is_start_state {
                code.push_str(&format!(
                    "    \"{}\" => {{ self.{}(__e); }}\n",
                    message, handler_method
                ));
                continue;
            }
            // Non-start state: extract lifecycle params from event
            // (keyed by declared param name) and pass to handler.
            code.push_str(&format!("    \"{}\" => {{\n", message));
            for param in &handler.params {
                let param_type = param.symbol_type.as_deref().unwrap_or("String");
                let extraction = match param_type {
                    "String" | "str" | "string" => format!(
                        "        let {0}: String = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();\n",
                        param.name
                    ),
                    "i32" | "i64" | "isize" | "u32" | "u64" | "usize" | "int" => format!(
                        "        let {0}: {1} = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<{1}>()).copied().unwrap_or_default();\n",
                        param.name, param_type
                    ),
                    "f32" | "f64" | "float" => format!(
                        "        let {0}: {1} = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<{1}>()).copied().unwrap_or_default();\n",
                        param.name, param_type
                    ),
                    "bool" => format!(
                        "        let {0}: bool = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<bool>()).copied().unwrap_or_default();\n",
                        param.name
                    ),
                    _ => format!(
                        "        let {0} = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<{1}>()).cloned().unwrap_or_default();\n",
                        param.name, param_type
                    ),
                };
                code.push_str(&extraction);
            }
            let param_names: Vec<_> = handler.params.iter().map(|p| p.name.clone()).collect();
            code.push_str(&format!(
                "        self.{}(__e, {});\n",
                handler_method,
                param_names.join(", ")
            ));
            code.push_str("    }\n");
            continue;
        }

        // Handle non-lifecycle handlers with parameters - extract from context stack
        // (The cloned __e has empty parameters due to Box<dyn Any> not being Clone)
        if !handler.params.is_empty() {
            code.push_str(&format!("    \"{}\" => {{\n", message));
            code.push_str(
                "        let __ctx_event = &self._context_stack.last().unwrap().event;\n",
            );
            for param in &handler.params {
                // Extract parameter from context stack event, downcast to the appropriate type
                let param_type = param.symbol_type.as_deref().unwrap_or("String");
                let extraction = match param_type {
                    "String" | "str" | "string" => format!(
                        "        let {}: String = __ctx_event.parameters.get(\"{}\").and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();\n",
                        param.name, param.name
                    ),
                    "i64" | "i32" | "isize" => format!(
                        "        let {}: {} = __ctx_event.parameters.get(\"{}\").and_then(|v| v.downcast_ref::<{}>()).copied().unwrap_or_default();\n",
                        param.name, param_type, param.name, param_type
                    ),
                    "f64" | "f32" => format!(
                        "        let {}: {} = __ctx_event.parameters.get(\"{}\").and_then(|v| v.downcast_ref::<{}>()).copied().unwrap_or_default();\n",
                        param.name, param_type, param.name, param_type
                    ),
                    "bool" => format!(
                        "        let {}: bool = __ctx_event.parameters.get(\"{}\").and_then(|v| v.downcast_ref::<bool>()).copied().unwrap_or_default();\n",
                        param.name, param.name
                    ),
                    _ => format!(
                        "        let {} = __ctx_event.parameters.get(\"{}\").and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();\n",
                        param.name, param.name
                    ),
                };
                code.push_str(&extraction);
            }
            let param_names: Vec<_> = handler.params.iter().map(|p| p.name.clone()).collect();
            code.push_str(&format!(
                "        self.{}(__e, {});\n",
                handler_method,
                param_names.join(", ")
            ));
            code.push_str("    }\n");
            continue;
        }

        // State vars live on compartment.state_context — no init needed in enter handler
        // Use block syntax to ignore handler return value (dispatch doesn't return)
        code.push_str(&format!(
            "    \"{}\" => {{ self.{}(__e); }}\n",
            message, handler_method
        ));
    }

    // State vars live on compartment.state_context — no auto-generated $> init needed

    // Default case - forward to parent if default_forward, else do nothing
    if default_forward {
        if let Some(parent) = parent_state {
            code.push_str(&format!("    _ => self._state_{}(__e),\n", parent));
        } else {
            code.push_str("    _ => {}\n");
        }
    } else {
        code.push_str("    _ => {}\n");
    }

    code.push_str("}");
    code
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

    // Determine method name based on handler type
    let method_name = if handler.is_enter {
        format!("_s_{}_enter", state_name)
    } else if handler.is_exit {
        format!("_s_{}_exit", state_name)
    } else {
        format!("_s_{}_{}", state_name, handler.event)
    };

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
        state_var_types: handler_state_var_types.clone(),
        state_param_names: state_param_names.clone(),
        state_enter_param_names: state_enter_param_names.clone(),
        state_exit_param_names: state_exit_param_names.clone(),
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
    body_code.push_str(&splice_handler_body_from_span(
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
