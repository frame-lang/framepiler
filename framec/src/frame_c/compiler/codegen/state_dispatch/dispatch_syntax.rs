//! Language-specific syntax variants for state dispatch code
//! generation.
//!
//! `DispatchSyntax` is a struct of function pointers — one
//! field per varying piece of per-target syntax (statement
//! terminator, `if`/`elif` keyword, HSM navigation helper,
//! parameter binding form, state-var init guard, forward call
//! shape). `dispatch_syntax_for` builds one struct per
//! language, and the unified emitter in `state_dispatch.rs`
//! consumes it.
//!
//! Sixteen if/elif-style languages share this struct (Python,
//! GDScript, TS, JS, Ruby, Lua, Php, Dart, Go, Java, Kotlin,
//! Swift, C#, C, C++, Erlang). Rust uses `match` and stays out
//! of this path entirely.

use super::super::codegen_utils::{
    cpp_map_type, csharp_map_type, go_map_type, java_map_type, kotlin_map_type, swift_map_type,
    to_snake_case,
};
use crate::frame_c::visitors::TargetLanguage;

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
                    "double" => {
                        format!("System.Convert.ToDouble(__compartment.state_args[{index}])")
                    }
                    "float" => {
                        format!("System.Convert.ToSingle(__compartment.state_args[{index}])")
                    }
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
                    "double" => {
                        format!("((Number) __compartment.state_args.get({index})).doubleValue()")
                    }
                    "float" => {
                        format!("((Number) __compartment.state_args.get({index})).floatValue()")
                    }
                    "long" => {
                        format!("((Number) __compartment.state_args.get({index})).longValue()")
                    }
                    "int" => format!("((Number) __compartment.state_args.get({index})).intValue()"),
                    "short" => {
                        format!("((Number) __compartment.state_args.get({index})).shortValue()")
                    }
                    "byte" => {
                        format!("((Number) __compartment.state_args.get({index})).byteValue()")
                    }
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
                    "Double" => {
                        format!("(__compartment.state_args[{index}] as! NSNumber).doubleValue")
                    }
                    "Float" => {
                        format!("(__compartment.state_args[{index}] as! NSNumber).floatValue")
                    }
                    "Int" => format!("(__compartment.state_args[{index}] as! NSNumber).intValue"),
                    "Int64" => {
                        format!("(__compartment.state_args[{index}] as! NSNumber).int64Value")
                    }
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
