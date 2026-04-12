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
    let body_code = match lang {
        TargetLanguage::Python3 => generate_python_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::GDScript => generate_gdscript_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            generate_typescript_state_dispatch(
                _system_name,
                state_name,
                handlers,
                state_vars,
                state_params,
                source,
                &ctx,
                default_forward,
                lang,
            )
        }
        TargetLanguage::Dart => generate_dart_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Rust => generate_rust_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            parent_state,
            default_forward,
            is_start_state,
        ),
        TargetLanguage::C => generate_c_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Cpp => generate_cpp_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Java => generate_java_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Kotlin => generate_kotlin_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Swift => generate_swift_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::CSharp => generate_csharp_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Go => generate_go_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Php => generate_php_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Ruby => generate_ruby_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Lua => generate_lua_state_dispatch(
            _system_name,
            state_name,
            handlers,
            state_vars,
            state_params,
            source,
            &ctx,
            default_forward,
        ),
        TargetLanguage::Erlang => String::new(), // TODO: Erlang gen_statem codegen
        TargetLanguage::Graphviz => unreachable!(),
    };

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
pub(crate) fn generate_python_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a local at the
    // top of the dispatch so handler bodies can read them by bare name.
    // All write sites (constructor for the start state, transition
    // codegen for both named and positional transitions) now store under
    // the declared param name, so a single named lookup is sufficient.
    for sp in state_params {
        code.push_str(&format!(
            "{name} = self.__compartment.state_args.get(\"{name}\")\n",
            name = sp.name
        ));
    }

    // HSM Compartment Navigation: When this handler accesses state vars, we need to ensure
    // we're accessing the correct compartment. If this handler was invoked via forwarding
    // from a child state, __compartment points to the child's compartment, not this state's.
    // Navigate the parent_compartment chain to find this state's compartment.
    // The while loop is a no-op if we're already in this state's compartment directly.
    if !state_vars.is_empty() {
        code.push_str(&format!(
            r#"# HSM: Navigate to this state's compartment for state var access
__sv_comp = self.__compartment
while __sv_comp is not None and __sv_comp.state != "{}":
    __sv_comp = __sv_comp.parent_compartment
"#,
            state_name
        ));
    }

    // If state has state variables but no explicit $> handler, generate one
    // Use conditional initialization to preserve values on pop-restore
    // Uses __sv_comp which was set up in preamble for HSM compartment navigation
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if __e._message == \"$>\":\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Python3)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Python3)
            };
            // Only initialize if not already set (preserves pop-restored values)
            code.push_str(&format!(
                "    if \"{}\" not in __sv_comp.state_vars:\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {}\n",
                var.name, init_val
            ));
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        // Map Frame events to their message names
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if __e._message == \"{}\":", message)
        } else {
            format!("elif __e._message == \"{}\":", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, initialize state vars first
        // Use conditional initialization to preserve values on pop-restore
        // Uses __sv_comp which was set up in preamble for HSM compartment navigation
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::Python3)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Python3)
                };
                // Only initialize if not already set (preserves pop-restored values)
                code.push_str(&format!(
                    "    if \"{}\" not in __sv_comp.state_vars:\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {}\n",
                    var.name, init_val
                ));
            }
        }

        // Generate parameter unpacking if handler has params.
        // All param dictionaries (enter_args, exit_args, interface call args)
        // are now keyed by the declared parameter name. Both lifecycle handlers
        // and interface handlers use the name-key form. The transition codegen
        // and the system constructor both write under the name; this read side
        // matches.
        let _is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for param in handler.params.iter() {
            code.push_str(&format!(
                "    {} = __e._parameters[\"{}\"]\n",
                param.name, param.name
            ));
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Python3, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Generate the handler body
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Python3,
            &handler_ctx,
        );

        // Indent the body
        let mut body_has_content = !return_init_code.is_empty();
        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
                body_has_content = true;
            }
            code.push('\n');
        }

        // If body was empty (no statements), add pass to avoid IndentationError
        if !body_has_content {
            code.push_str("    pass\n");
        }
    }

    // Add default forward clause if state has => $^ at state level
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            // Only add else clause if we have at least one if/elif above
            if !first {
                code.push_str("else:\n");
                code.push_str(&format!("    self._state_{}(__e)\n", parent));
            } else {
                // No handlers at all - just forward everything
                code.push_str(&format!("self._state_{}(__e)\n", parent));
            }
        }
    }

    // Trim trailing newlines
    code.trim_end().to_string()
}

/// Generate GDScript state dispatch code (if/elif chain on __e._message)
/// Similar to Python but with GDScript-specific syntax:
/// - `var` required for variable declarations
/// - `!= null` instead of `is not None`
/// - `.new()` constructor syntax
/// - GDScript literal values (true/false/null)
pub(crate) fn generate_gdscript_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a local at the
    // top of the dispatch. GDScript is dynamically typed so no cast.
    for sp in state_params {
        code.push_str(&format!(
            "var {0} = self.__compartment.state_args[\"{0}\"]\n",
            sp.name
        ));
    }

    // HSM Compartment Navigation
    if !state_vars.is_empty() {
        code.push_str(&format!(
            r#"# HSM: Navigate to this state's compartment for state var access
var __sv_comp = self.__compartment
while __sv_comp != null and __sv_comp.state != "{}":
    __sv_comp = __sv_comp.parent_compartment
"#,
            state_name
        ));
    }

    // If state has state variables but no explicit $> handler, generate one
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if __e._message == \"$>\":\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::GDScript)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::GDScript)
            };
            code.push_str(&format!(
                "    if not \"{}\" in __sv_comp.state_vars:\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {}\n",
                var.name, init_val
            ));
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if __e._message == \"{}\":", message)
        } else {
            format!("elif __e._message == \"{}\":", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, initialize state vars first
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::GDScript)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::GDScript)
                };
                code.push_str(&format!(
                    "    if not \"{}\" in __sv_comp.state_vars:\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {}\n",
                    var.name, init_val
                ));
            }
        }

        // Generate parameter unpacking — all params (lifecycle and interface)
        // are now keyed by declared name; the write side stores under the
        // declared name for both transition-passed and constructor-passed args.
        let _is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for param in handler.params.iter() {
            code.push_str(&format!(
                "    var {} = __e._parameters[\"{}\"]\n",
                param.name, param.name
            ));
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::GDScript, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Generate the handler body
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::GDScript,
            &handler_ctx,
        );

        // Indent the body
        let mut body_has_content = !return_init_code.is_empty();
        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
                body_has_content = true;
            }
            code.push('\n');
        }

        // If body was empty, add pass
        if !body_has_content {
            code.push_str("    pass\n");
        }
    }

    // Add default forward clause if state has => $^ at state level
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("else:\n");
                code.push_str(&format!("    self._state_{}(__e)\n", parent));
            } else {
                code.push_str(&format!("self._state_{}(__e)\n", parent));
            }
        }
    }

    // Trim trailing newlines
    code.trim_end().to_string()
}

/// Generate TypeScript/JavaScript state dispatch code (if/else chain on __e._message)
pub(crate) fn generate_typescript_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
    lang: TargetLanguage,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a local. TypeScript
    // and JavaScript are dynamically/structurally typed at runtime; the
    // dict value comes back as `any` and the user can refine it in the
    // handler body if needed.
    for sp in state_params {
        let type_ann = if matches!(lang, TargetLanguage::TypeScript) {
            ": any"
        } else {
            ""
        };
        code.push_str(&format!(
            "const {0}{1} = this.__compartment.state_args[\"{0}\"];\n",
            sp.name, type_ann
        ));
    }

    // HSM Compartment Navigation: When this handler accesses state vars, we need to ensure
    // we're accessing the correct compartment. If this handler was invoked via forwarding
    // from a child state, __compartment points to the child's compartment, not this state's.
    // Navigate the parent_compartment chain to find this state's compartment.
    if !state_vars.is_empty() {
        let type_ann = if matches!(lang, TargetLanguage::TypeScript) {
            ": any"
        } else {
            ""
        };
        code.push_str(&format!(
            "// HSM: Navigate to this state's compartment for state var access\nlet __sv_comp{} = this.__compartment;\nwhile (__sv_comp !== null && __sv_comp.state !== \"{}\") {{\n    __sv_comp = __sv_comp.parent_compartment;\n}}\n",
            type_ann, state_name));
    }

    // If state has state variables but no explicit $> handler, generate one
    // Use conditional initialization to preserve values on pop-restore
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if (__e._message === \"$>\") {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            // Only initialize if not already set (preserves pop-restored values)
            code.push_str(&format!(
                "    if (!(\"{0}\" in __sv_comp.state_vars)) {{\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {};\n",
                var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        // Map Frame events to their message names
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if (__e._message === \"{}\") {{", message)
        } else {
            format!("}} else if (__e._message === \"{}\") {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, initialize state vars first
        // Use conditional initialization to preserve values on pop-restore
        // Uses __sv_comp which was set up in preamble for HSM compartment navigation
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, lang)
                } else {
                    state_var_init_value(&var.var_type, lang)
                };
                // Only initialize if not already set (preserves pop-restored values)
                code.push_str(&format!(
                    "    if (!(\"{0}\" in __sv_comp.state_vars)) {{\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {};\n",
                    var.name, init_val
                ));
                code.push_str("    }\n");
            }
        }

        // Generate parameter unpacking if handler has params
        // For enter/exit handlers, use positional indices (transition args are positional)
        // For other handlers, use parameter names as keys (matching interface method generation)
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        // All params keyed by declared name (lifecycle and interface)
        for param in handler.params.iter() {
            code.push_str(&format!(
                "    const {} = __e._parameters?.[\"{}\"];\n",
                param.name, param.name
            ));
        }
        let _ = is_lifecycle_handler;

        // Emit handler default return value if present
        let return_init_code = emit_handler_return_init(handler, lang, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Generate the handler body
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(&handler.body_span, source, lang, &handler_ctx);

        // Indent the body
        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Add default forward clause or close the last if block
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                // Close previous block and add else clause
                code.push_str("} else {\n");
                code.push_str(&format!("    this._state_{}(__e);\n", parent));
                code.push_str("}");
            } else {
                // No handlers at all - just forward everything
                code.push_str(&format!("this._state_{}(__e);", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate Dart state dispatch code (if/else chain on __e._message)
///
/// Differences from TypeScript:
/// - `==` instead of `===`
/// - `final` instead of `const`
/// - `?["key"]` instead of `?.["key"]`
/// - `!obj.containsKey("key")` instead of `!("key" in obj)`
/// - `\$>` and `<\$` for enter/exit messages (Dart string interpolation)
pub(crate) fn generate_dart_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let lang = TargetLanguage::Dart;
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a typed local
    // via Dart `as` cast. Mirrors the Java/Kotlin preambles.
    for sp in state_params {
        let raw_type = match &sp.param_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.as_str(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
        };
        let dart_type = match raw_type {
            "int" | "i32" | "i64" => "int",
            "float" | "f32" | "f64" | "double" => "double",
            "bool" | "boolean" => "bool",
            "str" | "string" | "String" => "String",
            other => other,
        };
        code.push_str(&format!(
            "{1} {0} = this.__compartment.state_args[\"{0}\"] as {1};\n",
            sp.name, dart_type
        ));
    }

    // HSM Compartment Navigation
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "// HSM: Navigate to this state's compartment for state var access\ndynamic __sv_comp = this.__compartment;\nwhile (__sv_comp != null && __sv_comp.state != \"{}\") {{\n    __sv_comp = __sv_comp.parent_compartment;\n}}\n",
            state_name));
    }

    // Auto-generate $> handler for state var init if needed
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if (__e._message == \"\\$>\") {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, lang)
            } else {
                state_var_init_value(&var.var_type, lang)
            };
            code.push_str(&format!(
                "    if (!__sv_comp.state_vars.containsKey(\"{0}\")) {{\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {};\n",
                var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        // Map Frame events to their message names (escaped for Dart)
        let message = match event.as_str() {
            "$>" | "enter" => "\\$>",
            "$<" | "exit" => "<\\$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if (__e._message == \"{}\") {{", message)
        } else {
            format!("}} else if (__e._message == \"{}\") {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // State var init in enter handler
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, lang)
                } else {
                    state_var_init_value(&var.var_type, lang)
                };
                code.push_str(&format!(
                    "    if (!__sv_comp.state_vars.containsKey(\"{0}\")) {{\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {};\n",
                    var.name, init_val
                ));
                code.push_str("    }\n");
            }
        }

        // Generate parameter unpacking
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        // All params keyed by declared name (lifecycle and interface)
        for param in handler.params.iter() {
            code.push_str(&format!(
                "    final {} = __e._parameters?[\"{}\"];\n",
                param.name, param.name
            ));
        }
        let _ = is_lifecycle_handler;

        // Emit handler default return value if present
        let return_init_code = emit_handler_return_init(handler, lang, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Generate the handler body
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(&handler.body_span, source, lang, &handler_ctx);

        // Indent the body
        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Add default forward clause or close the last if block
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("} else {\n");
                code.push_str(&format!("    _state_{}(__e);\n", parent));
                code.push_str("}");
            } else {
                code.push_str(&format!("_state_{}(__e);", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate PHP state dispatch code (if-else chain on $__e->_message)
pub(crate) fn generate_php_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a PHP local.
    for sp in state_params {
        code.push_str(&format!(
            "${0} = $this->__compartment->state_args[\"{0}\"];\n",
            sp.name
        ));
    }

    // HSM: Navigate parent_compartment chain
    if !state_vars.is_empty() {
        code.push_str("// HSM: Navigate to this state's compartment for state var access\n$__sv_comp = $this->__compartment;\nwhile ($__sv_comp !== null && $__sv_comp->state !== \"");
        code.push_str(state_name);
        code.push_str("\") {\n    $__sv_comp = $__sv_comp->parent_compartment;\n}\n");
    }

    // Auto-generate $> handler for state var init if needed
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if ($__e->_message === \"$>\") {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Php)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Php)
            };
            code.push_str(&format!(
                "    if (!array_key_exists(\"{0}\", $__sv_comp->state_vars)) {{\n",
                var.name
            ));
            code.push_str(&format!(
                "        $__sv_comp->state_vars[\"{}\"] = {};\n",
                var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if ($__e->_message === \"{}\") {{", message)
        } else {
            format!("}} else if ($__e->_message === \"{}\") {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // State var init in enter handler
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::Php)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Php)
                };
                code.push_str(&format!(
                    "    if (!array_key_exists(\"{0}\", $__sv_comp->state_vars)) {{\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        $__sv_comp->state_vars[\"{}\"] = {};\n",
                    var.name, init_val
                ));
                code.push_str("    }\n");
            }
        }

        // Parameter unpacking
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        // All params keyed by declared name (lifecycle and interface)
        for param in handler.params.iter() {
            code.push_str(&format!(
                "    ${} = $__e->_parameters[\"{}\"] ?? null;\n",
                param.name, param.name
            ));
        }
        let _ = is_lifecycle_handler;

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Php, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body via splicer
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Php,
            &handler_ctx,
        );

        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Default forward or close
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("} else {\n");
                code.push_str(&format!("    $this->_state_{}($__e);\n", parent));
                code.push_str("}");
            } else {
                code.push_str(&format!("$this->_state_{}($__e);", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate Ruby state dispatch code (if/elsif/end chain on __e._message)
pub(crate) fn generate_ruby_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a Ruby local.
    for sp in state_params {
        code.push_str(&format!(
            "{0} = @__compartment.state_args[\"{0}\"]\n",
            sp.name
        ));
    }

    // HSM: Navigate parent_compartment chain
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "# HSM: Navigate to this state's compartment for state var access\n__sv_comp = @__compartment\nwhile __sv_comp != nil && __sv_comp.state != \"{}\"\n    __sv_comp = __sv_comp.parent_compartment\nend\n",
            state_name
        ));
    }

    // Auto-generate $> handler for state var init if needed
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if __e._message == \"$>\"\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Ruby)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Ruby)
            };
            code.push_str(&format!(
                "    if !__sv_comp.state_vars.key?(\"{}\")\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {}\n",
                var.name, init_val
            ));
            code.push_str("    end\n");
        }
        first = false;
    }

    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if __e._message == \"{}\"", message)
        } else {
            format!("elsif __e._message == \"{}\"", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // State var init in enter handler
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::Ruby)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Ruby)
                };
                code.push_str(&format!(
                    "    if !__sv_comp.state_vars.key?(\"{}\")\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {}\n",
                    var.name, init_val
                ));
                code.push_str("    end\n");
            }
        }

        // Parameter unpacking (no type casts — dynamic typing)
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for param in handler.params.iter() {
            if is_lifecycle_handler {
                let args_source = if event == "$>" || event == "enter" {
                    "@__compartment.enter_args"
                } else {
                    "@__compartment.exit_args"
                };
                code.push_str(&format!(
                    "    {} = {}[\"{}\"]\n",
                    param.name, args_source, param.name
                ));
            } else {
                code.push_str(&format!(
                    "    {} = __e._parameters[\"{}\"]\n",
                    param.name, param.name
                ));
            }
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Ruby, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body via splicer
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Ruby,
            &handler_ctx,
        );

        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Default forward or close — Ruby uses end
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("else\n");
                code.push_str(&format!("    _state_{}(__e)\n", parent));
                code.push_str("end");
            } else {
                code.push_str(&format!("_state_{}(__e)", parent));
            }
        } else if !first {
            code.push_str("end");
        }
    } else if !first {
        code.push_str("end");
    }

    code
}

/// Generate C++17 state dispatch code (if-else chain on __e._message)
pub(crate) fn generate_cpp_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment->state_args[name] to a typed local
    // at the top of the dispatch so handler bodies can read them by bare
    // name. Mirrors the Python and C preambles. The values were stored as
    // std::any in the constructor (or by the transition codegen), so we
    // pull them out via std::any_cast<Type> here.
    for sp in state_params {
        let raw_type = match &sp.param_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.as_str(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
        };
        let cpp_type = cpp_map_type(raw_type);
        code.push_str(&format!(
            "auto {0} = std::any_cast<{1}>(__compartment->state_args[\"{0}\"]);\n",
            sp.name, cpp_type
        ));
    }

    // HSM: Navigate parent_compartment chain to find this state's compartment
    // When forwarded from a child, __compartment points to the child's compartment
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "auto* __sv_comp = __compartment.get();\nwhile (__sv_comp && __sv_comp->state != \"{}\") {{ __sv_comp = __sv_comp->parent_compartment.get(); }}\n",
            state_name
        ));
    }

    // Auto-generate enter handler for state var init if no explicit $> handler
    // Use conditional init to preserve values restored by pop$
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if (__e._message == \"$>\") {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                cpp_wrap_any_arg(&expression_to_string(init, TargetLanguage::Cpp))
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Cpp)
            };
            code.push_str(&format!("    if (__compartment->state_vars.count(\"{0}\") == 0) {{ __compartment->state_vars[\"{0}\"] = std::any({1}); }}\n", var.name, init_val));
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if (__e._message == \"{}\") {{", message)
        } else {
            format!("}} else if (__e._message == \"{}\") {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, conditionally initialize
        // (preserves values restored by pop$)
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    cpp_wrap_any_arg(&expression_to_string(init, TargetLanguage::Cpp))
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Cpp)
                };
                code.push_str(&format!("    if (__compartment->state_vars.count(\"{0}\") == 0) {{ __compartment->state_vars[\"{0}\"] = std::any({1}); }}\n", var.name, init_val));
            }
        }

        // Parameter unpacking
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for (i, param) in handler.params.iter().enumerate() {
            let param_type = param
                .symbol_type
                .as_ref()
                .map(|s| cpp_map_type(s))
                .unwrap_or_else(|| "std::any".to_string());
            // Both lifecycle and interface handlers now read by declared
            // param name. The transition codegen and constructor write
            // under the name; this read side matches.
            let key = param.name.clone();
            if is_lifecycle_handler {
                // Enter/exit handlers get args from compartment
                let args_source = if event == "$>" || event == "enter" {
                    "__compartment->enter_args"
                } else {
                    "__compartment->exit_args"
                };
                if param_type == "std::any" {
                    code.push_str(&format!(
                        "    auto {} = {}[\"{}\"];\n",
                        param.name, args_source, key
                    ));
                } else {
                    code.push_str(&format!(
                        "    auto {} = std::any_cast<{}>({}[\"{}\"]);\n",
                        param.name, param_type, args_source, key
                    ));
                }
            } else {
                if param_type == "std::any" {
                    code.push_str(&format!(
                        "    auto {} = __e._parameters.at(\"{}\");\n",
                        param.name, key
                    ));
                } else {
                    code.push_str(&format!(
                        "    auto {} = std::any_cast<{}>(__e._parameters.at(\"{}\"));\n",
                        param.name, param_type, key
                    ));
                }
            }
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Cpp, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body via splicer
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Cpp,
            &handler_ctx,
        );

        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Default forward or close
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("} else {\n");
                code.push_str(&format!("    _state_{}(__e);\n", parent));
                code.push_str("}");
            } else {
                code.push_str(&format!("_state_{}(__e);", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate Java state dispatch code (if-else chain with .equals())
pub(crate) fn generate_java_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a typed local
    // via Java cast. Mirrors the Python/C/C++/Go preambles. Values are
    // boxed Object in the HashMap, so the cast is on a boxed reference;
    // Java auto-unboxes to the primitive on assignment to a primitive
    // local.
    for sp in state_params {
        let raw_type = match &sp.param_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.as_str(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
        };
        let java_type = java_map_type(raw_type);
        let cast_type = match raw_type {
            "int" | "i32" => "Integer",
            "i64" => "Long",
            "bool" | "boolean" => "Boolean",
            "float" => "Float",
            "double" | "f32" | "f64" => "Double",
            _ => java_type.as_str(),
        };
        code.push_str(&format!(
            "{0} {1} = ({2}) __compartment.state_args.get(\"{1}\");\n",
            java_type, sp.name, cast_type
        ));
    }

    // HSM Compartment Navigation
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "var __sv_comp = __compartment;\nwhile (__sv_comp != null && !__sv_comp.state.equals(\"{}\")) {{ __sv_comp = __sv_comp.parent_compartment; }}\n",
            state_name
        ));
    }

    // Auto-generate enter handler for state var init if no explicit $> handler
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if (__e._message.equals(\"$>\")) {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Java)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Java)
            };
            code.push_str(&format!(
                "    if (!__sv_comp.state_vars.containsKey(\"{0}\")) {{\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars.put(\"{}\", {});\n",
                var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if (__e._message.equals(\"{}\")) {{", message)
        } else {
            format!("}} else if (__e._message.equals(\"{}\")) {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, conditionally initialize
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::Java)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Java)
                };
                code.push_str(&format!(
                    "    if (!__sv_comp.state_vars.containsKey(\"{0}\")) {{\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars.put(\"{}\", {});\n",
                    var.name, init_val
                ));
                code.push_str("    }\n");
            }
        }

        // Parameter unpacking
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for (i, param) in handler.params.iter().enumerate() {
            let param_type = param
                .symbol_type
                .as_ref()
                .map(|s| java_map_type(s))
                .unwrap_or_else(|| "Object".to_string());
            // Both lifecycle and interface handlers now read by declared
            // param name. The transition codegen and constructor write
            // under the name; this read side matches.
            let key = param.name.clone();
            if is_lifecycle_handler {
                let args_source = if event == "$>" || event == "enter" {
                    "__compartment.enter_args"
                } else {
                    "__compartment.exit_args"
                };
                code.push_str(&format!(
                    "    var {} = ({}) {}.get(\"{}\");\n",
                    param.name, param_type, args_source, key
                ));
            } else {
                code.push_str(&format!(
                    "    var {} = ({}) __e._parameters.get(\"{}\");\n",
                    param.name, param_type, key
                ));
            }
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Java, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body via splicer
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Java,
            &handler_ctx,
        );
        // Java: strip ;; (unreachable empty statement after return)
        let body = body.replace(";;", ";");

        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Default forward or close
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("} else {\n");
                code.push_str(&format!("    _state_{}(__e);\n", parent));
                code.push_str("}");
            } else {
                code.push_str(&format!("_state_{}(__e);", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate Kotlin state dispatch code (if-else chain with == comparison, no semicolons)
pub(crate) fn generate_kotlin_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a typed local
    // via Kotlin `as` cast. Mirrors the Java/Swift preambles.
    for sp in state_params {
        let raw_type = match &sp.param_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.as_str(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "Int",
        };
        let kotlin_type = kotlin_map_type(raw_type);
        code.push_str(&format!(
            "val {0} = __compartment.state_args[\"{0}\"] as {1}\n",
            sp.name, kotlin_type
        ));
    }

    // HSM Compartment Navigation — Kotlin uses == for structural equality
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "var __sv_comp = __compartment\nwhile (__sv_comp != null && __sv_comp.state != \"{}\") {{ __sv_comp = __sv_comp.parent_compartment!! }}\n",
            state_name
        ));
    }

    // Auto-generate enter handler for state var init if no explicit $> handler
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if (__e._message == \"$>\") {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Kotlin)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Kotlin)
            };
            code.push_str(&format!(
                "    if (!__sv_comp.state_vars.containsKey(\"{0}\")) {{\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {}\n",
                var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if (__e._message == \"{}\") {{", message)
        } else {
            format!("}} else if (__e._message == \"{}\") {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, conditionally initialize
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::Kotlin)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Kotlin)
                };
                code.push_str(&format!(
                    "    if (!__sv_comp.state_vars.containsKey(\"{0}\")) {{\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {}\n",
                    var.name, init_val
                ));
                code.push_str("    }\n");
            }
        }

        // Parameter unpacking — Kotlin: `as Type` instead of `(Type)`, no semicolons
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for (i, param) in handler.params.iter().enumerate() {
            let param_type = param
                .symbol_type
                .as_ref()
                .map(|s| kotlin_map_type(s))
                .unwrap_or_else(|| "Any?".to_string());
            // Both lifecycle and interface handlers now read by declared
            // param name. The transition codegen and constructor write
            // under the name; this read side matches.
            let key = param.name.clone();
            if is_lifecycle_handler {
                let args_source = if event == "$>" || event == "enter" {
                    "__compartment.enter_args"
                } else {
                    "__compartment.exit_args"
                };
                code.push_str(&format!(
                    "    val {} = {}[\"{}\"] as {}\n",
                    param.name, args_source, key, param_type
                ));
            } else {
                code.push_str(&format!(
                    "    val {} = __e._parameters[\"{}\"] as {}\n",
                    param.name, key, param_type
                ));
            }
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Kotlin, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body via splicer
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Kotlin,
            &handler_ctx,
        );

        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Default forward or close — no semicolons
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("} else {\n");
                code.push_str(&format!("    _state_{}(__e)\n", parent));
                code.push_str("}");
            } else {
                code.push_str(&format!("_state_{}(__e)", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate Swift state dispatch code (if-else chain with == comparison, no semicolons)
pub(crate) fn generate_swift_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a typed local
    // via Swift force-cast `as!`. Mirrors the Python/C/C++/Go/Java
    // preambles. Values are stored as Any in the dictionary.
    for sp in state_params {
        let raw_type = match &sp.param_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.as_str(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "Int",
        };
        let swift_type = swift_map_type(raw_type);
        code.push_str(&format!(
            "let {0} = __compartment.state_args[\"{0}\"] as! {1}\n",
            sp.name, swift_type
        ));
    }

    // HSM Compartment Navigation — Swift uses == for comparison
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "var __sv_comp = __compartment\nwhile __sv_comp.state != \"{}\" {{ __sv_comp = __sv_comp.parent_compartment! }}\n",
            state_name
        ));
    }

    // Auto-generate enter handler for state var init if no explicit $> handler
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if __e._message == \"$>\" {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Swift)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Swift)
            };
            code.push_str(&format!(
                "    if __sv_comp.state_vars[\"{0}\"] == nil {{\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {}\n",
                var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if __e._message == \"{}\" {{", message)
        } else {
            format!("}} else if __e._message == \"{}\" {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, conditionally initialize
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::Swift)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Swift)
                };
                code.push_str(&format!(
                    "    if __sv_comp.state_vars[\"{0}\"] == nil {{\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {}\n",
                    var.name, init_val
                ));
                code.push_str("    }\n");
            }
        }

        // Parameter unpacking — Swift: `as! Type` for cast, no semicolons
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for (i, param) in handler.params.iter().enumerate() {
            let param_type = param
                .symbol_type
                .as_ref()
                .map(|s| swift_map_type(s))
                .unwrap_or_else(|| "Any".to_string());
            // Both lifecycle and interface handlers now read by declared
            // param name. The transition codegen and constructor write
            // under the name; this read side matches.
            let key = param.name.clone();
            if is_lifecycle_handler {
                let args_source = if event == "$>" || event == "enter" {
                    "__compartment.enter_args"
                } else {
                    "__compartment.exit_args"
                };
                code.push_str(&format!(
                    "    let {} = {}[\"{}\"] as! {}\n",
                    param.name, args_source, key, param_type
                ));
            } else {
                code.push_str(&format!(
                    "    let {} = __e._parameters[\"{}\"] as! {}\n",
                    param.name, key, param_type
                ));
            }
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Swift, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body via splicer
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Swift,
            &handler_ctx,
        );

        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Default forward or close — no semicolons
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("} else {\n");
                code.push_str(&format!("    _state_{}(__e)\n", parent));
                code.push_str("}");
            } else {
                code.push_str(&format!("_state_{}(__e)", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate C# state dispatch code (if-else chain with == comparison)
/// Generate Go state dispatch code (if-else chain with == comparison)
pub(crate) fn generate_go_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.stateArgs[name] to a typed local
    // via Go type assertion `val.(int)`.
    for sp in state_params {
        let raw_type = match &sp.param_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.as_str(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
        };
        let go_type = go_map_type(raw_type);
        if go_type == "any" {
            code.push_str(&format!(
                "{0} := s.__compartment.stateArgs[\"{0}\"]\n_ = {0}\n",
                sp.name
            ));
        } else {
            code.push_str(&format!(
                "{0} := s.__compartment.stateArgs[\"{0}\"].({1})\n_ = {0}\n",
                sp.name, go_type
            ));
        }
    }

    // HSM Compartment Navigation
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "__sv_comp := s.__compartment\nfor __sv_comp != nil && __sv_comp.state != \"{}\" {{ __sv_comp = __sv_comp.parentCompartment }}\n",
            state_name
        ));
    }

    // Auto-generate enter handler for state var init if no explicit $> handler
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if __e._message == \"$>\" {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Go)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Go)
            };
            code.push_str(&format!(
                "    if _, ok := __sv_comp.stateVars[\"{}\"]; !ok {{\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.stateVars[\"{}\"] = {}\n",
                var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if __e._message == \"{}\" {{", message)
        } else {
            format!("}} else if __e._message == \"{}\" {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, conditionally initialize
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::Go)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Go)
                };
                code.push_str(&format!(
                    "    if _, ok := __sv_comp.stateVars[\"{}\"]; !ok {{\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.stateVars[\"{}\"] = {}\n",
                    var.name, init_val
                ));
                code.push_str("    }\n");
            }
        }

        // Parameter unpacking
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for (i, param) in handler.params.iter().enumerate() {
            let param_type = param
                .symbol_type
                .as_ref()
                .map(|s| go_map_type(s))
                .unwrap_or_else(|| "any".to_string());
            // Both lifecycle and interface handlers now read by declared
            // param name. The transition codegen and constructor write
            // under the name; this read side matches.
            let key = param.name.clone();
            if is_lifecycle_handler {
                let args_source = if event == "$>" || event == "enter" {
                    "s.__compartment.enterArgs"
                } else {
                    "s.__compartment.exitArgs"
                };
                if param_type == "any" {
                    code.push_str(&format!(
                        "    {} := {}[\"{}\"]\n",
                        param.name, args_source, key
                    ));
                } else {
                    code.push_str(&format!(
                        "    {} := {}[\"{}\"].({})\n",
                        param.name, args_source, key, param_type
                    ));
                }
            } else {
                if param_type == "any" {
                    code.push_str(&format!(
                        "    {} := __e._parameters[\"{}\"]\n",
                        param.name, key
                    ));
                } else {
                    code.push_str(&format!(
                        "    {} := __e._parameters[\"{}\"].({})\n",
                        param.name, key, param_type
                    ));
                }
            }
            // Suppress Go "declared but not used" error
            code.push_str(&format!("    _ = {}\n", param.name));
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Go, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body via splicer
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Go,
            &handler_ctx,
        );

        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Default forward or close
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("} else {\n");
                code.push_str(&format!("    s._state_{}(__e)\n", parent));
                code.push_str("}");
            } else {
                code.push_str(&format!("s._state_{}(__e)", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

pub(crate) fn generate_csharp_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a typed local
    // via C# cast. Mirrors the Python/Java preambles.
    for sp in state_params {
        let raw_type = match &sp.param_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.as_str(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
        };
        let cs_type = csharp_map_type(raw_type);
        code.push_str(&format!(
            "{1} {0} = ({1}) __compartment.state_args[\"{0}\"];\n",
            sp.name, cs_type
        ));
    }

    // HSM Compartment Navigation
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "var __sv_comp = __compartment;\nwhile (__sv_comp != null && __sv_comp.state != \"{}\") {{ __sv_comp = __sv_comp.parent_compartment; }}\n",
            state_name
        ));
    }

    // Auto-generate enter handler for state var init if no explicit $> handler
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if (__e._message == \"$>\") {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::CSharp)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::CSharp)
            };
            code.push_str(&format!(
                "    if (!__sv_comp.state_vars.ContainsKey(\"{0}\")) {{\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {};\n",
                var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if (__e._message == \"{}\") {{", message)
        } else {
            format!("}} else if (__e._message == \"{}\") {{", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, conditionally initialize
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::CSharp)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::CSharp)
                };
                code.push_str(&format!(
                    "    if (!__sv_comp.state_vars.ContainsKey(\"{0}\")) {{\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {};\n",
                    var.name, init_val
                ));
                code.push_str("    }\n");
            }
        }

        // Parameter unpacking
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for (i, param) in handler.params.iter().enumerate() {
            let param_type = param
                .symbol_type
                .as_ref()
                .map(|s| csharp_map_type(s))
                .unwrap_or_else(|| "object".to_string());
            // Both lifecycle and interface handlers now read by declared
            // param name. The transition codegen and constructor write
            // under the name; this read side matches.
            let key = param.name.clone();
            if is_lifecycle_handler {
                let args_source = if event == "$>" || event == "enter" {
                    "__compartment.enter_args"
                } else {
                    "__compartment.exit_args"
                };
                code.push_str(&format!(
                    "    var {} = ({}) {}[\"{}\"];\n",
                    param.name, param_type, args_source, key
                ));
            } else {
                code.push_str(&format!(
                    "    var {} = ({}) __e._parameters[\"{}\"];\n",
                    param.name, param_type, key
                ));
            }
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::CSharp, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body via splicer
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::CSharp,
            &handler_ctx,
        );
        // C#: strip ;; (unreachable empty statement after return)
        let body = body.replace(";;", ";");

        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Default forward or close
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                code.push_str("} else {\n");
                code.push_str(&format!("    _state_{}(__e);\n", parent));
                code.push_str("}");
            } else {
                code.push_str(&format!("_state_{}(__e);", parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate C state dispatch code (if-else chain with strcmp)
pub(crate) fn generate_c_state_dispatch(
    system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a local at the
    // top of the dispatch so handler bodies can read them by bare name.
    // Mirrors the Python preamble: every declared state.params entry is
    // pulled out of `compartment->state_args` (named key) and exposed
    // as a typed local. Handler bodies can then write `x` instead of
    // hand-rolling a `FrameDict_get` cast on every read.
    for sp in state_params {
        let type_str = match &sp.param_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "int".to_string(),
        };
        let (c_type, cast) = match type_str.as_str() {
            "int" | "i32" | "i64" => ("int", "(intptr_t)"),
            "bool" | "boolean" => ("bool", "(intptr_t)"),
            "float" | "double" | "f32" | "f64" => ("double", "(intptr_t)"),
            "str" | "string" | "String" => ("char*", ""),
            _ => ("void*", ""),
        };
        code.push_str(&format!(
            "{} {} = ({}){}{}_FrameDict_get(self->__compartment->state_args, \"{}\");\n",
            c_type, sp.name, c_type, cast, system_name, sp.name
        ));
    }

    // HSM Compartment Navigation: When this handler accesses state vars, we need to ensure
    // we're accessing the correct compartment. If this handler was invoked via forwarding
    // from a child state, __compartment points to the child's compartment, not this state's.
    // Navigate the parent_compartment chain to find this state's compartment.
    if !state_vars.is_empty() {
        code.push_str(&format!(
            r#"// HSM: Navigate to this state's compartment for state var access
{}_Compartment* __sv_comp = self->__compartment;
while (__sv_comp != NULL && strcmp(__sv_comp->state, "{}") != 0) {{
    __sv_comp = __sv_comp->parent_compartment;
}}
"#,
            system_name, state_name
        ));
    }

    // If state has state variables but no explicit $> handler, generate one
    // Use conditional initialization to preserve values on pop-restore
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if (strcmp(__e->_message, \"$>\") == 0) {\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::C)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::C)
            };
            // Only initialize if not already set (preserves pop-restored values)
            // Use __sv_comp which was set up in preamble for HSM compartment navigation
            code.push_str(&format!(
                "    if (!{}_FrameDict_has(__sv_comp->state_vars, \"{}\")) {{\n",
                system_name, var.name
            ));
            code.push_str(&format!(
                "        {}_FrameDict_set(__sv_comp->state_vars, \"{}\", (void*)(intptr_t){});\n",
                system_name, var.name, init_val
            ));
            code.push_str("    }\n");
        }
        first = false;
    }

    // Sort handlers for deterministic output
    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        // Map Frame events to their message names
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if (strcmp(__e->_message, \"{}\") == 0) {{", message)
        } else {
            format!(
                "}} else if (strcmp(__e->_message, \"{}\") == 0) {{",
                message
            )
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // For enter handlers with state vars, initialize state vars first
        // Use conditional initialization to preserve values on pop-restore
        // Uses __sv_comp which was set up in preamble for HSM compartment navigation
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::C)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::C)
                };
                // Only initialize if not already set (preserves pop-restored values)
                code.push_str(&format!(
                    "    if (!{}_FrameDict_has(__sv_comp->state_vars, \"{}\")) {{\n",
                    system_name, var.name
                ));
                code.push_str(&format!("        {}_FrameDict_set(__sv_comp->state_vars, \"{}\", (void*)(intptr_t){});\n",
                    system_name, var.name, init_val));
                code.push_str("    }\n");
            }
        }

        // Generate parameter unpacking if handler has params
        // For enter/exit handlers, use positional indices (transition args are positional)
        // For other handlers, use parameter names as keys (matching interface method generation)
        let is_lifecycle_handler =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for param in handler.params.iter() {
            let param_type = param
                .symbol_type
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or("int");
            let c_type = match param_type {
                "int" | "i32" | "i64" => "int",
                "bool" | "boolean" => "bool",
                "float" | "double" | "f32" | "f64" => "double",
                "str" | "string" | "String" => "char*",
                _ => "void*",
            };
            let cast = if c_type == "int" || c_type == "bool" {
                "(intptr_t)"
            } else {
                ""
            };
            // All params (lifecycle and interface) keyed by declared name.
            code.push_str(&format!(
                "    {} {} = ({}){}{}_FrameDict_get(__e->_parameters, \"{}\");\n",
                c_type, param.name, c_type, cast, system_name, param.name
            ));
        }
        let _ = is_lifecycle_handler;

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::C, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Generate the handler body
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::C,
            &handler_ctx,
        );

        // Indent the body
        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
            }
            code.push('\n');
        }
    }

    // Add default forward clause or close the last if block
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if !first {
                // Close previous block and add else clause
                code.push_str("} else {\n");
                code.push_str(&format!(
                    "    {}_state_{}(self, __e);\n",
                    system_name, parent
                ));
                code.push_str("}");
            } else {
                // No handlers at all - just forward everything
                code.push_str(&format!("{}_state_{}(self, __e);", system_name, parent));
            }
        } else if !first {
            code.push_str("}");
        }
    } else if !first {
        code.push_str("}");
    }

    code
}

/// Generate Rust state dispatch code (match on __e.message)
///
/// Unlike Python/TypeScript which inline handler code, Rust dispatches to separate methods
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
                code.push_str(&format!("        let {0} = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();\n", param.name));
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
pub(crate) fn generate_lua_state_dispatch(
    _system_name: &str,
    state_name: &str,
    handlers: &std::collections::HashMap<String, HandlerEntry>,
    state_vars: &[StateVarAst],
    state_params: &[crate::frame_c::compiler::frame_ast::StateParam],
    source: &[u8],
    ctx: &HandlerContext,
    default_forward: bool,
) -> String {
    let mut code = String::new();
    let mut first = true;
    let has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");

    // State params: bind compartment.state_args[name] to a Lua local.
    for sp in state_params {
        code.push_str(&format!(
            "local {0} = self.__compartment.state_args[\"{0}\"]\n",
            sp.name
        ));
    }

    // HSM Compartment Navigation for state var access
    if !state_vars.is_empty() {
        code.push_str(&format!(
            "-- HSM: Navigate to this state's compartment for state var access\n\
             local __sv_comp = self.__compartment\n\
             while __sv_comp ~= nil and __sv_comp.state ~= \"{}\" do\n\
             __sv_comp = __sv_comp.parent_compartment\n\
             end\n",
            state_name
        ));
    }

    // Auto-generate enter handler for state var init if no explicit one
    if !state_vars.is_empty() && !has_enter_handler {
        code.push_str("if __e._message == \"$>\" then\n");
        for var in state_vars {
            let init_val = if let Some(ref init) = var.init {
                expression_to_string(init, TargetLanguage::Lua)
            } else {
                state_var_init_value(&var.var_type, TargetLanguage::Lua)
            };
            code.push_str(&format!(
                "    if __sv_comp.state_vars[\"{}\"] == nil then\n",
                var.name
            ));
            code.push_str(&format!(
                "        __sv_comp.state_vars[\"{}\"] = {}\n",
                var.name, init_val
            ));
            code.push_str("    end\n");
        }
        first = false;
    }

    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let condition = if first {
            format!("if __e._message == \"{}\" then", message)
        } else {
            format!("elseif __e._message == \"{}\" then", message)
        };
        first = false;

        code.push_str(&condition);
        code.push('\n');

        // State var init in enter handler
        if (event == "$>" || event == "enter") && !state_vars.is_empty() {
            for var in state_vars {
                let init_val = if let Some(ref init) = var.init {
                    expression_to_string(init, TargetLanguage::Lua)
                } else {
                    state_var_init_value(&var.var_type, TargetLanguage::Lua)
                };
                code.push_str(&format!(
                    "    if __sv_comp.state_vars[\"{}\"] == nil then\n",
                    var.name
                ));
                code.push_str(&format!(
                    "        __sv_comp.state_vars[\"{}\"] = {}\n",
                    var.name, init_val
                ));
                code.push_str("    end\n");
            }
        }

        // Parameter unpacking — read by named key for both lifecycle
        // (enter/exit) and interface handlers. The constructor and
        // transition codegen both write under the declared param name,
        // so the read side matches.
        let _is_lifecycle =
            event == "$>" || event == "enter" || event == "$<" || event == "exit" || event == "<$";
        for param in handler.params.iter() {
            code.push_str(&format!(
                "    local {} = __e._parameters[\"{}\"]\n",
                param.name, param.name
            ));
        }

        // Emit handler default return value if present
        let return_init_code =
            emit_handler_return_init(handler, TargetLanguage::Lua, "    ", &ctx.system_name);
        if !return_init_code.is_empty() {
            code.push_str(&return_init_code);
        }

        // Handler body
        let mut handler_ctx = ctx.clone();
        handler_ctx.event_name = event.clone();
        let raw_body = splice_handler_body_from_span(
            &handler.body_span,
            source,
            TargetLanguage::Lua,
            &handler_ctx,
        );
        // Transform if/else { } blocks to Lua if/then/elseif/else/end
        let body = super::block_transform::transform_blocks(
            &raw_body,
            super::block_transform::BlockTransformMode::Lua,
        );

        let mut body_has_content = !return_init_code.is_empty();
        for line in body.lines() {
            if !line.trim().is_empty() {
                code.push_str("    ");
                code.push_str(line);
                body_has_content = true;
            }
            code.push('\n');
        }

        if !body_has_content {
            code.push_str("    -- empty\n");
        }
    }

    // Default forward to parent state for HSM
    if default_forward {
        if let Some(ref parent) = ctx.parent_state {
            if first {
                // No handlers at all — unconditional forward
                code.push_str(&format!("self:_state_{}(__e)\n", parent));
            } else {
                // Add else branch to forward unhandled events
                code.push_str(&format!("else\n    self:_state_{}(__e)\n", parent));
            }
        }
    }

    // Close the if/elseif chain
    if !first {
        code.push_str("end\n");
    }

    code
}

/// Generate a handler method from Arcanum's HandlerEntry
///
/// Uses the handler's body_span to extract and splice native code with Frame expansions.
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
