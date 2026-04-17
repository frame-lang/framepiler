//! Rust system code generation.
//!
//! Owns the Rust-specific codegen pipeline: machinery methods, state
//! dispatch, push/pop transitions, and (in future phases) fields,
//! constructor, interface dispatch, and Frame-statement expansion
//! delegates.
//!
//! `backends/rust.rs` handles the lower-level `CodegenNode → String`
//! rendering and is not modified by this module.

use super::ast::{CodegenNode, Field, Param, Visibility};
use super::codegen_utils::type_to_string;
use super::state_dispatch::generate_handler_from_arcanum;
use super::system_codegen::{expand_tagged_in_domain, init_references_param};
use crate::frame_c::compiler::arcanum::{Arcanum, HandlerEntry};
use crate::frame_c::compiler::frame_ast::{
    InterfaceMethod, MachineAst, ParamKind, StateVarAst, SystemAst, Type,
};
use crate::frame_c::visitors::TargetLanguage;

/// Generate the complete Rust system from a Frame AST.
///
/// Called from `system_codegen::generate_system` when target is Rust.
/// Returns a `CodegenNode::Class` tree that `backends/rust.rs` renders.
///
/// Owns the Rust pipeline: calls shared sub-functions where they still
/// contain Rust match arms, and Rust-specific functions (machinery,
/// dispatch, persistence) where they've been extracted.
pub fn generate_rust_system(system: &SystemAst, arcanum: &Arcanum, source: &[u8]) -> CodegenNode {
    let lang = TargetLanguage::Rust;
    let backend = super::backend::get_backend(lang);
    let syntax = backend.class_syntax();

    let needs_async = system.interface.iter().any(|m| m.is_async);
    let has_state_vars = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().any(|s| !s.state_vars.is_empty()))
        .unwrap_or(false);

    // ── Fields (Rust-specific) ──────────────────────────────────
    let fields = generate_rust_fields(system);

    // ── Methods ──────────────────────────────────────────────────
    let mut methods = Vec::new();

    // Constructor (Rust-specific)
    methods.push(generate_rust_constructor(system));

    // Frame machinery (kernel, router, transition — owned here)
    methods.extend(super::system_codegen::generate_frame_machinery(
        system, &syntax, lang,
    ));

    // Interface wrappers (delegates to generate_rust_interface_body)
    methods.extend(super::interface_gen::generate_interface_wrappers(
        system, &syntax,
    ));

    // State handlers (dispatch + individual handler methods — owned here)
    if let Some(ref machine) = system.machine {
        methods.extend(super::state_dispatch::generate_state_handlers_via_arcanum(
            &system.name,
            machine,
            arcanum,
            source,
            lang,
            has_state_vars,
        ));
    }

    // Actions + operations (shared — native passthrough)
    for action in &system.actions {
        methods.push(super::interface_gen::generate_action(
            action, &syntax, source,
        ));
    }
    for operation in &system.operations {
        methods.push(super::interface_gen::generate_operation(
            operation, &syntax, source,
        ));
    }

    // Persistence (owned here)
    if system.persist_attr.is_some() {
        methods.extend(generate_rust_persistence_methods(system));
    }

    let mut class_node = CodegenNode::Class {
        name: system.name.clone(),
        fields,
        methods,
        base_classes: vec![],
        is_abstract: false,
        derives: vec![],
    };

    if needs_async {
        super::system_codegen::make_system_async(&mut class_node, &system.name, lang);
    }

    class_node
}

// ─── Fields ──────────────────────────────────────────────────────────

/// Generate Rust struct fields: state stack, compartment, next compartment,
/// context stack, domain variables, and synthetic `__sys_*` param fields.
fn generate_rust_fields(system: &SystemAst) -> Vec<Field> {
    let mut fields = Vec::new();
    let compartment_type = format!("{}Compartment", system.name);

    // State stack
    fields.push(
        Field::new("_state_stack")
            .with_visibility(Visibility::Private)
            .with_type(&format!("Vec<{}>", compartment_type)),
    );

    // Current compartment (owned, not Option)
    fields.push(
        Field::new("__compartment")
            .with_visibility(Visibility::Private)
            .with_type(&compartment_type),
    );

    // Next compartment (deferred transition target)
    fields.push(
        Field::new("__next_compartment")
            .with_visibility(Visibility::Private)
            .with_type(&format!("Option<{}>", compartment_type)),
    );

    // Context stack for reentrant dispatch
    fields.push(
        Field::new("_context_stack")
            .with_visibility(Visibility::Private)
            .with_type(&format!("Vec<{}FrameContext>", system.name)),
    );

    // Domain variables
    let sys_param_names: Vec<String> = system.params.iter().map(|p| p.name.clone()).collect();
    for domain_var in &system.domain {
        let type_str_opt = match &domain_var.var_type {
            Type::Custom(s) => Some(s.clone()),
            Type::Unknown => None,
        };

        let mut field = Field::new(&domain_var.name).with_visibility(Visibility::Public);
        if let Some(ref t) = type_str_opt {
            field = field.with_type(t);
        }
        field.is_const = domain_var.is_const;

        let init_text_str = domain_var.initializer_text.as_deref().unwrap_or("");
        let strip_collision = init_references_param(init_text_str, &sys_param_names);
        if !strip_collision {
            if let Some(ref init_text) = &domain_var.initializer_text {
                let expanded_init = expand_tagged_in_domain(init_text, TargetLanguage::Rust);
                field = field.with_initializer(CodegenNode::Ident(expanded_init));
            }
        }

        fields.push(field);
    }

    // Synthetic __sys_* fields for state/enter header params
    for p in &system.params {
        match p.kind {
            ParamKind::StateArg | ParamKind::EnterArg => {
                let ts = type_to_string(&p.param_type);
                fields.push(
                    Field::new(&format!("__sys_{}", p.name))
                        .with_visibility(Visibility::Private)
                        .with_type(&ts),
                );
            }
            ParamKind::Domain => {}
        }
    }

    fields
}

// ─── Constructor ─────────────────────────────────────────────────────

/// Generate Rust constructor: domain var init (struct-literal folding),
/// system param stashing, compartment creation with HSM parent chain.
fn generate_rust_constructor(system: &SystemAst) -> CodegenNode {
    let mut body = Vec::new();
    let sys_param_names: Vec<String> = system.params.iter().map(|p| p.name.clone()).collect();

    // Stack init — Rust uses Vec::new() for both
    body.push(CodegenNode::assign(
        CodegenNode::field(CodegenNode::self_ref(), "_state_stack"),
        CodegenNode::Ident("Vec::new()".to_string()),
    ));
    body.push(CodegenNode::assign(
        CodegenNode::field(CodegenNode::self_ref(), "_context_stack"),
        CodegenNode::Ident("Vec::new()".to_string()),
    ));

    // Domain variable initialization
    for domain_var in &system.domain {
        let is_domain_param = system
            .params
            .iter()
            .any(|p| p.name == domain_var.name && matches!(p.kind, ParamKind::Domain));

        let init = match &domain_var.initializer_text {
            None => {
                if is_domain_param {
                    domain_var.name.clone()
                } else {
                    "Default::default()".to_string()
                }
            }
            Some(init_text) => {
                if is_domain_param {
                    domain_var.name.clone()
                } else {
                    expand_tagged_in_domain(init_text, TargetLanguage::Rust)
                }
            }
        };

        body.push(CodegenNode::assign(
            CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
            CodegenNode::Ident(init),
        ));
    }

    // __sys_* fields for state/enter header params
    for p in &system.params {
        match p.kind {
            ParamKind::StateArg | ParamKind::EnterArg => {
                body.push(CodegenNode::assign(
                    CodegenNode::field(CodegenNode::self_ref(), &format!("__sys_{}", p.name)),
                    CodegenNode::Ident(p.name.clone()),
                ));
            }
            ParamKind::Domain => {}
        }
    }

    // Compartment creation for start state
    if let Some(ref machine) = system.machine {
        if let Some(first_state) = machine.states.first() {
            let has_hsm_parent = first_state.parent.is_some();

            if has_hsm_parent {
                // Build ancestor chain from root to leaf
                let mut ancestor_chain = Vec::new();
                let mut current_parent = first_state.parent.as_ref();
                while let Some(parent_name) = current_parent {
                    if let Some(parent_state) =
                        machine.states.iter().find(|s| &s.name == parent_name)
                    {
                        ancestor_chain.push(parent_state);
                        current_parent = parent_state.parent.as_ref();
                    } else {
                        break;
                    }
                }
                ancestor_chain.reverse();

                // Block expression that creates parent chain and returns child
                let mut block_expr = String::new();
                block_expr.push_str("{\n");
                let mut prev_comp_var = "None".to_string();
                for (i, ancestor) in ancestor_chain.iter().enumerate() {
                    let comp_var = format!("__parent_comp_{}", i);
                    block_expr.push_str(&format!(
                        "let mut {} = {}Compartment::new(\"{}\");\n",
                        comp_var, system.name, ancestor.name
                    ));
                    block_expr.push_str(&format!(
                        "{}.parent_compartment = {};\n",
                        comp_var, prev_comp_var
                    ));
                    prev_comp_var = format!("Some(Box::new({}))", comp_var);
                }
                block_expr.push_str(&format!(
                    "let mut __child = {}Compartment::new(\"{}\");\n",
                    system.name, first_state.name
                ));
                block_expr.push_str(&format!(
                    "__child.parent_compartment = {};\n",
                    prev_comp_var
                ));
                block_expr.push_str("__child\n}");

                body.push(CodegenNode::assign(
                    CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                    CodegenNode::Ident(block_expr),
                ));
            } else {
                body.push(CodegenNode::assign(
                    CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                    CodegenNode::Ident(format!(
                        "{}Compartment::new(\"{}\")",
                        system.name, first_state.name
                    )),
                ));
            }

            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                CodegenNode::Ident("None".to_string()),
            ));

            // Start state enter_args from system header params
            // (state_args are NOT emitted for Rust — Rust uses the typed
            // StateContext enum, initialised by Compartment::new(); the
            // actual values live in __sys_* fields read by handlers.)
            for p in &system.params {
                if matches!(p.kind, ParamKind::EnterArg) {
                    // Reference the stored __sys_* field, not the raw param —
                    // the param was moved into the field in the struct init above.
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment.enter_args.insert(\"{}\".to_string(), self.__sys_{}.to_string());",
                            p.name, p.name
                        ),
                        span: None,
                    });
                }
            }

            // Fire $> event
            let event_class = format!("{}FrameEvent", system.name);
            let context_class = format!("{}FrameContext", system.name);
            body.push(CodegenNode::NativeBlock {
                code: format!(
                    "let __frame_event = {}::new_with_params(\"$>\", &self.__compartment.enter_args);\n\
                     let __ctx = {}::new(__frame_event, None);\n\
                     self._context_stack.push(__ctx);\n\
                     self.__kernel();\n\
                     self._context_stack.pop();",
                    event_class, context_class
                ),
                span: None,
            });
        }
    }

    // System params as constructor parameters
    let params: Vec<Param> = system
        .params
        .iter()
        .map(|p| {
            let ts = type_to_string(&p.param_type);
            Param::new(&p.name).with_type(&ts)
        })
        .collect();

    CodegenNode::Constructor {
        params,
        body,
        super_call: None,
    }
}

// ─── Machinery ───────────────────────────────────────────────────────

/// Generate Rust runtime machinery: `__kernel`, `__router`, `__transition`.
pub(crate) fn generate_rust_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // __kernel — event processing loop with deferred transitions
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"// Clone event from context stack (needed for borrow checker)
let __e = self._context_stack.last().unwrap().event.clone();
// Route event to current state
self.__router(&__e);
// Process any pending transition
while self.__next_compartment.is_some() {{
    let next_compartment = self.__next_compartment.take().unwrap();
    // Exit current state (with exit_args from current compartment)
    let exit_event = {0}::new_with_params("<$", &self.__compartment.exit_args);
    self.__router(&exit_event);
    // Switch to new compartment
    self.__compartment = next_compartment;
    // Enter new state (or forward event)
    if self.__compartment.forward_event.is_none() {{
        let enter_event = {0}::new_with_params("$>", &self.__compartment.enter_args);
        self.__router(&enter_event);
    }} else {{
        // Forward event to new state
        let forward_event = self.__compartment.forward_event.take().unwrap();
        if forward_event.message == "$>" {{
            // Forwarding enter event - just send it
            self.__router(&forward_event);
        }} else {{
            // Forwarding other event - send $> first, then forward
            let enter_event = {0}::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
            self.__router(&forward_event);
        }}
    }}
    // Mark all stacked contexts as transitioned
    for ctx in self._context_stack.iter_mut() {{
        ctx._transitioned = true;
    }}
}}"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — dispatches events to state methods via match
    let router_code = generate_rust_router_dispatch(system);
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(&format!("&{}", event_class))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition — caches next compartment (deferred)
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment").with_type(compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "self.__next_compartment = Some(next_compartment);".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

/// Generate `__push_transition` — saves current compartment on stack
/// via `std::mem::replace`, then enters the new state.
pub(crate) fn generate_rust_push_transition(system: &SystemAst) -> CodegenNode {
    let system_name = &system.name;
    let event_class = format!("{}FrameEvent", system_name);
    let compartment_class = format!("{}Compartment", system_name);

    let code = format!(
        r#"// Exit current state (old compartment still in place for routing)
let exit_event = {event_class}::new_with_params("<$", &self.__compartment.exit_args);
self.__router(&exit_event);
// Swap: old compartment moves to stack, new takes its place
let old = std::mem::replace(&mut self.__compartment, new_compartment);
self._state_stack.push(old);
// Enter new state (or forward event) — matches kernel logic
if self.__compartment.forward_event.is_none() {{
    let enter_event = {event_class}::new_with_params("$>", &self.__compartment.enter_args);
    self.__router(&enter_event);
}} else {{
    let forward_event = self.__compartment.forward_event.take().unwrap();
    if forward_event.message == "$>" {{
        self.__router(&forward_event);
    }} else {{
        let enter_event = {event_class}::new_with_params("$>", &self.__compartment.enter_args);
        self.__router(&enter_event);
        self.__router(&forward_event);
    }}
}}"#,
        event_class = event_class
    );

    CodegenNode::Method {
        name: "__push_transition".to_string(),
        params: vec![Param::new("new_compartment").with_type(&compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock { code, span: None }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

// ─── Dispatch ────────────────────────────────────────────────────────

/// Generate Rust router dispatch — match on compartment state name.
fn generate_rust_router_dispatch(system: &SystemAst) -> String {
    let mut code = String::new();
    code.push_str("match self.__compartment.state.as_str() {\n");

    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            code.push_str(&format!(
                "    \"{}\" => self._state_{}(__e),\n",
                state.name, state.name
            ));
        }
    }

    code.push_str("    _ => {}\n");
    code.push_str("}");
    code
}

/// Generate Rust state dispatch — match on event message, extract
/// typed parameters from context stack via `Box<dyn Any>` downcasting.
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

    let mut sorted_handlers: Vec<_> = handlers.iter().collect();
    sorted_handlers.sort_by_key(|(event, _)| *event);

    let _has_enter_handler = handlers.contains_key("$>") || handlers.contains_key("enter");
    let _needs_state_var_init = !state_vars.is_empty();

    for (event, handler) in sorted_handlers {
        let message = match event.as_str() {
            "$>" | "enter" => "$>",
            "$<" | "exit" => "<$",
            _ => event.as_str(),
        };

        let handler_method = match event.as_str() {
            "$>" | "enter" => format!("_s_{}_enter", state_name),
            "$<" | "exit" => format!("_s_{}_exit", state_name),
            _ => format!("_s_{}_{}", state_name, event),
        };

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
            code.push_str(&format!("    \"{}\" => {{\n", message));
            for param in &handler.params {
                let param_type = param.symbol_type.as_deref().unwrap_or("String");
                let extraction = match param_type {
                    "String" | "str" | "string" => format!(
                        "        let {0}: String = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();\n",
                        param.name
                    ),
                    "i32" => format!(
                        "        let {0}: i32 = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name
                    ),
                    "i64" => format!(
                        "        let {0}: i64 = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name
                    ),
                    "f64" | "f32" => format!(
                        "        let {0}: {1} = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name, param_type
                    ),
                    "bool" => format!(
                        "        let {0}: bool = __e.parameters.get(\"{0}\").and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
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

        if !handler.params.is_empty() {
            code.push_str(&format!("    \"{}\" => {{\n", message));
            code.push_str(
                "        let __ctx_event = &self._context_stack.last().unwrap().event;\n",
            );
            for param in &handler.params {
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

        code.push_str(&format!(
            "    \"{}\" => {{ self.{}(__e); }}\n",
            message, handler_method
        ));
    }

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

// ─── State Handlers ──────────────────────────────────────────────────

/// Generate Rust individual handler methods (`_s_State_event`) that
/// the state dispatch method calls. Other languages inline handler
/// code in the dispatch; Rust emits separate methods.
pub(crate) fn generate_rust_handler_methods(
    system_name: &str,
    machine: &MachineAst,
    arcanum: &Arcanum,
    source: &[u8],
    has_state_vars: bool,
    defined_systems: &std::collections::HashSet<String>,
    state_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_enter_param_names: &std::collections::HashMap<String, Vec<String>>,
    state_exit_param_names: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

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
        let non_start_state_param_names: Vec<String> = if !is_start_state {
            state_entry.params.iter().map(|p| p.name.clone()).collect()
        } else {
            Vec::new()
        };
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
                TargetLanguage::Rust,
                has_state_vars,
                defined_systems,
                sys_param_locals,
                is_start_state,
                &non_start_state_param_names,
                state_param_names,
                state_enter_param_names,
                state_exit_param_names,
                &handler_state_var_types,
            );
            methods.push(method);
        }
    }

    methods
}

// ─── Interface Dispatch ──────────────────────────────────────────────

/// Generate the Rust interface method body: create FrameEvent, push
/// FrameContext onto the context stack, call `__kernel`, pop, and
/// downcast the return value.
pub(crate) fn generate_rust_interface_body(
    system_name: &str,
    method: &InterfaceMethod,
    event_class: &str,
) -> CodegenNode {
    let context_class = format!("{}FrameContext", system_name);

    let params_code = if method.params.is_empty() {
        String::new()
    } else {
        method
            .params
            .iter()
            .map(|p| {
                format!(
                    "__e.parameters.insert(\"{}\".to_string(), Box::new({}.clone()) as Box<dyn std::any::Any>);",
                    p.name, p.name
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut code = format!("let mut __e = {}::new(\"{}\");\n", event_class, method.name);
    if !params_code.is_empty() {
        code.push_str(&params_code);
        code.push('\n');
    }

    code.push_str(&format!(
        "let mut __ctx = {}::new(__e, None);\n",
        context_class
    ));
    if let Some(ref init_expr) = method.return_init {
        let wrapped = if init_expr.trim().starts_with('"') && init_expr.trim().ends_with('"') {
            format!("String::from({})", init_expr.trim())
        } else {
            init_expr.clone()
        };
        code.push_str(&format!(
            "__ctx._return = Some(Box::new({}) as Box<dyn std::any::Any>);\n",
            wrapped
        ));
    }
    code.push_str("self._context_stack.push(__ctx);\n");
    code.push_str("self.__kernel();\n");

    if let Some(ref rt) = method.return_type {
        let raw_type = type_to_string(rt);
        let return_type = match raw_type.as_str() {
            "str" | "string" => "String".to_string(),
            "int" => "i64".to_string(),
            "float" => "f64".to_string(),
            "bool" => "bool".to_string(),
            "Any" => "String".to_string(),
            other => other.to_string(),
        };
        code.push_str(&format!(
            r#"let __ctx = self._context_stack.pop().unwrap();
if let Some(ret) = __ctx._return {{
    *ret.downcast::<{}>().unwrap()
}} else {{
    Default::default()
}}"#,
            return_type
        ));
    } else {
        code.push_str("self._context_stack.pop();");
    }

    CodegenNode::NativeBlock { code, span: None }
}

// ─── Frame Expansion Delegates ───────────────────────────────────────
//
// These functions are called from frame_expansion.rs Rust match arms,
// consolidating Rust-specific ownership/borrow patterns here.

use super::codegen_utils::HandlerContext;
use super::frame_expansion::{resolve_enter_arg_key, resolve_exit_arg_key, resolve_state_arg_key};

/// Rust transition expansion: compartment creation with exit/state/enter
/// args, HSM parent chain, and typed StateContext enum assignment.
pub(crate) fn rust_expand_transition(
    indent_str: &str,
    ctx: &HandlerContext,
    target: &str,
    exit_str: &Option<String>,
    state_str: &Option<String>,
    enter_str: &Option<String>,
) -> String {
    let mut code = String::new();

    if let Some(ref exit) = exit_str {
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
            code.push_str(&format!(
                "{}self.__compartment.exit_args.insert(\"{}\".to_string(), {}.to_string());\n",
                indent_str, key, value
            ));
        }
    }

    code.push_str(&format!(
        "{}let mut __compartment = {}Compartment::new(\"{}\");\n",
        indent_str, ctx.system_name, target
    ));
    code.push_str(&format!(
        "{}__compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));\n",
        indent_str
    ));

    if let Some(ref state) = state_str {
        let args: Vec<&str> = state
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .collect();
        if !args.is_empty() {
            code.push_str(&format!(
                "{}if let {}StateContext::{}(ref mut ctx) = __compartment.state_context {{\n",
                indent_str, ctx.system_name, target
            ));
            for (i, arg) in args.iter().enumerate() {
                let (key, value) = if let Some(eq_pos) = arg.find('=') {
                    (
                        arg[..eq_pos].trim().to_string(),
                        arg[eq_pos + 1..].trim().to_string(),
                    )
                } else {
                    (resolve_state_arg_key(i, target, ctx), (*arg).to_string())
                };
                code.push_str(&format!("{}    ctx.{} = {};\n", indent_str, key, value));
            }
            code.push_str(&format!("{}}}\n", indent_str));
        }
    }

    if let Some(ref enter) = enter_str {
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
                (resolve_enter_arg_key(i, target, ctx), arg.to_string())
            };
            code.push_str(&format!(
                "{}__compartment.enter_args.insert(\"{}\".to_string(), {}.to_string());\n",
                indent_str, key, value
            ));
        }
    }

    code.push_str(&format!(
        "{}self.__transition(__compartment);\n{}return;",
        indent_str, indent_str
    ));
    code
}

/// Rust transition-forward: create compartment with forwarded event.
pub(crate) fn rust_expand_forward_transition(
    indent_str: &str,
    ctx: &HandlerContext,
    target: &str,
) -> String {
    let mut code = String::new();
    code.push_str(&format!(
        "{}let mut __compartment = {}Compartment::new(\"{}\");\n",
        indent_str, ctx.system_name, target
    ));
    code.push_str(&format!(
        "{}__compartment.forward_event = Some(__e.clone());\n",
        indent_str
    ));
    code.push_str(&format!(
        "{}self.__transition(__compartment);\n",
        indent_str
    ));
    code.push_str(&format!("{}return;", indent_str));
    code
}

/// Rust state variable read — walks HSM parent_compartment chain,
/// pattern-matches StateContext enum, conditionally clones non-Copy types.
pub(crate) fn rust_expand_state_var_read(ctx: &HandlerContext, var_name: &str) -> String {
    let is_copy = ctx
        .state_var_types
        .get(var_name)
        .map(|t| {
            matches!(
                t.to_lowercase().as_str(),
                "i32"
                    | "i64"
                    | "u32"
                    | "u64"
                    | "isize"
                    | "usize"
                    | "f32"
                    | "f64"
                    | "bool"
                    | "int"
                    | "float"
                    | "number"
            )
        })
        .unwrap_or(false);
    let suffix = if is_copy { "" } else { ".clone()" };
    format!(
        "{{ let mut __sv_comp = &self.__compartment; while __sv_comp.state != \"{}\" \
         {{ __sv_comp = __sv_comp.parent_compartment.as_ref().unwrap(); }} \
         match &__sv_comp.state_context {{ {}StateContext::{}(ctx) => ctx.{}{}, _ => unreachable!() }} }}",
        ctx.state_name, ctx.system_name, ctx.state_name, var_name, suffix
    )
}

/// Rust state variable write — uses unsafe raw pointers to navigate the
/// parent_compartment chain and mutate. RHS evaluated first to avoid
/// borrow conflicts.
pub(crate) fn rust_expand_state_var_write(
    indent_str: &str,
    ctx: &HandlerContext,
    var_name: &str,
    expanded_expr: &str,
) -> String {
    format!(
        concat!(
            "{}{{\n",
            "{0}    let __rhs = {};\n",
            "{0}    let mut __sv_comp: *mut {}Compartment = &mut self.__compartment;\n",
            "{0}    unsafe {{ while (*__sv_comp).state != \"{}\" {{ __sv_comp = (*__sv_comp).parent_compartment.as_mut().unwrap().as_mut(); }} }}\n",
            "{0}    unsafe {{ if let {}StateContext::{}(ref mut ctx) = (*__sv_comp).state_context {{ ctx.{} = __rhs; }} }}\n",
            "{0}}}"
        ),
        indent_str, expanded_expr,
        ctx.system_name, ctx.state_name,
        ctx.system_name, ctx.state_name, var_name
    )
}

/// Rust return-value boxing — wraps expression in Box<dyn Any>, handles
/// string literal conversion (&str → String).
pub(crate) fn rust_expand_box_return(indent_str: &str, expanded_expr: &str) -> String {
    let boxed_expr = rust_wrap_string_literal(expanded_expr);
    format!(
        "{}let __return_val = Box::new({}) as Box<dyn std::any::Any>;\n\
         {}if let Some(ctx) = self._context_stack.last_mut() {{ ctx._return = Some(__return_val); }}",
        indent_str, boxed_expr, indent_str
    )
}

/// Rust return-value boxing (no leading indent on first line — used in
/// ReturnCall/ContextReturnExpr where the caller provides the indent).
pub(crate) fn rust_expand_box_return_bare(indent_str: &str, expanded_expr: &str) -> String {
    let boxed_expr = rust_wrap_string_literal(expanded_expr);
    format!(
        "let __return_val = Box::new({}) as Box<dyn std::any::Any>;\n\
         {}if let Some(ctx) = self._context_stack.last_mut() {{ ctx._return = Some(__return_val); }}",
        boxed_expr, indent_str
    )
}

/// Rust context data write — wraps in Box<dyn Any>, handles string literals.
pub(crate) fn rust_expand_context_data_write(
    indent_str: &str,
    key: &str,
    expanded_expr: &str,
) -> String {
    let boxed_expr = rust_wrap_string_literal(expanded_expr);
    format!(
        "{}if let Some(ctx) = self._context_stack.last_mut() {{ ctx._data.insert(\"{}\".to_string(), Box::new({}) as Box<dyn std::any::Any>); }}",
        indent_str, key, boxed_expr
    )
}

/// Convert &str literal to String::from() for Box<dyn Any> downcasting.
fn rust_wrap_string_literal(expr: &str) -> String {
    if expr.trim().starts_with('"') && expr.trim().ends_with('"') {
        format!("String::from({})", expr.trim())
    } else {
        expr.to_string()
    }
}

// ─── Inline Expression Delegates ─────────────────────────────────────
//
// Small Rust-specific expressions used by frame_expansion.rs match arms.
// Each returns a String (no indent — inline context).

/// `@@:self` bare reference
pub(crate) fn rust_self_ref() -> &'static str {
    "self"
}

/// `@@:event` — event message access
pub(crate) fn rust_event_message() -> String {
    "__e.message.clone()".to_string()
}

/// `@@:params[key]` — context parameter access
pub(crate) fn rust_context_param(key: &str) -> String {
    key.to_string()
}

/// `@@:data[key]` read — context data access with downcast
pub(crate) fn rust_context_data_get(key: &str) -> String {
    format!(
        "self._context_stack.last().and_then(|ctx| ctx._data.get(\"{}\"))\
         .and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default()",
        key
    )
}

/// `@@:return` read — context return value access
pub(crate) fn rust_context_return_read() -> String {
    "self._context_stack.last().and_then(|ctx| ctx._return.as_ref())".to_string()
}

/// `@@:system.state` — current state name
pub(crate) fn rust_system_state() -> String {
    "self.__compartment.state.clone()".to_string()
}

/// Tagged instantiation compile error (undefined system)
pub(crate) fn rust_tagged_instantiation_error(system_name: &str) -> String {
    format!(
        "compile_error!(\"Frame Error E421: Undefined system '{}' in tagged instantiation @@{}\");",
        system_name, system_name
    )
}

// ─── Statement Delegates ────────────────────────────────────────────
//
// Rust-specific statements used by frame_expansion.rs match arms.
// Each returns a String with indent_str prefix.

/// HSM parent forward: `self._state_Parent(__e);`
pub(crate) fn rust_parent_forward(indent_str: &str, parent: &str) -> String {
    format!("{}self._state_{}(__e);", indent_str, parent)
}

/// Push-with-transition: `self.__push_transition(XyzCompartment::new("State"))`
pub(crate) fn rust_push_transition(
    indent_str: &str,
    ctx: &super::codegen_utils::HandlerContext,
    target: &str,
) -> String {
    format!(
        "{}self.__push_transition({}Compartment::new(\"{}\"));\n{}return;",
        indent_str, ctx.system_name, target, indent_str
    )
}

/// Bare push: `self._state_stack.push(self.__compartment.clone())`
pub(crate) fn rust_bare_push(indent_str: &str) -> String {
    format!(
        "{}self._state_stack.push(self.__compartment.clone());",
        indent_str
    )
}

/// Bare stack pop: `self._state_stack.pop();`
pub(crate) fn rust_bare_pop(indent_str: &str) -> String {
    format!("{}self._state_stack.pop();", indent_str)
}

/// Transition guard check after self-call
pub(crate) fn rust_transition_guard(indent_str: &str) -> String {
    format!(
        "{}if self._context_stack.last().map_or(false, |ctx| ctx._transitioned) {{ return; }}",
        indent_str
    )
}

// ─── Pop Transition Delegates ───────────────────────────────────────

/// Pop: exit_args write
pub(crate) fn rust_pop_exit_arg(indent: &str, key: &str, value: &str) -> String {
    format!(
        "{}self.__compartment.exit_args.insert(\"{}\".to_string(), {}.to_string());\n",
        indent, key, value
    )
}

/// Pop: stack pop
pub(crate) fn rust_pop_stack(indent: &str) -> String {
    format!(
        "{}let mut __popped = self._state_stack.pop().unwrap();\n",
        indent
    )
}

/// Pop: enter_args write
pub(crate) fn rust_pop_enter_arg(indent: &str, key: &str, value: &str) -> String {
    format!(
        "{}__popped.enter_args.insert(\"{}\".to_string(), {}.to_string());\n",
        indent, key, value
    )
}

/// Pop: forward event
pub(crate) fn rust_pop_forward(indent: &str) -> String {
    format!("{}__popped.forward_event = Some(__e.clone());\n", indent)
}

/// Pop: variable name (Rust uses `__popped`, others use `__saved`)
pub(crate) fn rust_pop_var_name() -> &'static str {
    "__popped"
}

/// Pop: transition call
pub(crate) fn rust_pop_transition(indent: &str) -> String {
    format!("{}self.__transition(__popped);\n{}return;", indent, indent)
}

// ─── Persistence ─────────────────────────────────────────────────────

/// Generate Rust `save_state` and `restore_state` methods using
/// serde_json. Serializes the entire compartment chain (HSM parent
/// links), StateContext enums, state stack, and domain variables.
pub(crate) fn generate_rust_persistence_methods(system: &SystemAst) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // ── save_state ───────────────────────────────────────────────
    let mut save_body = String::new();

    save_body.push_str(&format!(
        "fn serialize_state_context(ctx: &{}StateContext) -> serde_json::Value {{\n",
        system.name
    ));
    save_body.push_str("    match ctx {\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            if state.state_vars.is_empty() {
                save_body.push_str(&format!(
                    "        {}StateContext::{} => serde_json::json!({{}}),\n",
                    system.name, state.name
                ));
            } else {
                save_body.push_str(&format!(
                    "        {}StateContext::{}(ctx) => serde_json::json!({{\n",
                    system.name, state.name
                ));
                for var in &state.state_vars {
                    save_body.push_str(&format!(
                        "            \"{}\": ctx.{},\n",
                        var.name, var.name
                    ));
                }
                save_body.push_str("        }),\n");
            }
        }
    }
    save_body.push_str(&format!(
        "        {}StateContext::Empty => serde_json::json!({{}}),\n",
        system.name
    ));
    save_body.push_str("    }\n");
    save_body.push_str("}\n");

    save_body.push_str(&format!(
        "fn serialize_comp(comp: &{}Compartment) -> serde_json::Value {{\n",
        system.name
    ));
    save_body.push_str("    let parent = match &comp.parent_compartment {\n");
    save_body.push_str("        Some(p) => serialize_comp(p),\n");
    save_body.push_str("        None => serde_json::Value::Null,\n");
    save_body.push_str("    };\n");
    save_body.push_str("    serde_json::json!({\n");
    save_body.push_str("        \"state\": comp.state,\n");
    save_body
        .push_str("        \"state_context\": serialize_state_context(&comp.state_context),\n");
    save_body.push_str("        \"parent_compartment\": parent,\n");
    save_body.push_str("    })\n");
    save_body.push_str("}\n");

    save_body.push_str("let compartment_data = serialize_comp(&self.__compartment);\n");
    save_body.push_str("let stack_data: Vec<serde_json::Value> = self._state_stack.iter()\n");
    save_body.push_str("    .map(|comp| serialize_comp(comp))\n");
    save_body.push_str("    .collect();\n");
    save_body.push_str("serde_json::json!({\n");
    save_body.push_str("    \"_compartment\": compartment_data,\n");
    save_body.push_str("    \"_state_stack\": stack_data,\n");

    for var in &system.domain {
        save_body.push_str(&format!("    \"{}\": self.{},\n", var.name, var.name));
    }

    save_body.push_str("}).to_string()");

    methods.push(CodegenNode::Method {
        name: "save_state".to_string(),
        params: vec![],
        return_type: Some("String".to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: save_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    // ── restore_state ────────────────────────────────────────────
    let mut restore_body = String::new();
    restore_body.push_str("let data: serde_json::Value = serde_json::from_str(json).unwrap();\n");

    restore_body.push_str(&format!(
        "fn deserialize_state_context(state: &str, data: &serde_json::Value) -> {}StateContext {{\n",
        system.name
    ));
    restore_body.push_str("    match state {\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            if state.state_vars.is_empty() {
                restore_body.push_str(&format!(
                    "        \"{}\" => {}StateContext::{},\n",
                    state.name, system.name, state.name
                ));
            } else {
                restore_body.push_str(&format!(
                    "        \"{}\" => {}StateContext::{}({}Context {{\n",
                    state.name, system.name, state.name, state.name
                ));
                for var in &state.state_vars {
                    let json_extract = rust_json_extract(&var.name, &var.var_type);
                    restore_body
                        .push_str(&format!("            {}: {},\n", var.name, json_extract));
                }
                restore_body.push_str("        }),\n");
            }
        }
    }
    restore_body.push_str(&format!(
        "        _ => {}StateContext::Empty,\n",
        system.name
    ));
    restore_body.push_str("    }\n");
    restore_body.push_str("}\n");

    restore_body.push_str(&format!(
        "fn deserialize_comp(data: &serde_json::Value) -> {}Compartment {{\n",
        system.name
    ));
    restore_body.push_str("    let state = data[\"state\"].as_str().unwrap();\n");
    restore_body.push_str(&format!(
        "    let mut comp = {}Compartment::new(state);\n",
        system.name
    ));
    restore_body.push_str("    let ctx_data = &data[\"state_context\"];\n");
    restore_body.push_str("    if !ctx_data.is_null() {\n");
    restore_body
        .push_str("        comp.state_context = deserialize_state_context(state, ctx_data);\n");
    restore_body.push_str("    }\n");
    restore_body.push_str("    if !data[\"parent_compartment\"].is_null() {\n");
    restore_body.push_str(
        "        comp.parent_compartment = Some(Box::new(deserialize_comp(&data[\"parent_compartment\"])));\n",
    );
    restore_body.push_str("    }\n");
    restore_body.push_str("    comp\n");
    restore_body.push_str("}\n");

    restore_body.push_str(&format!(
        "let stack: Vec<{}Compartment> = data[\"_state_stack\"].as_array()\n",
        system.name
    ));
    restore_body.push_str("    .map(|arr| arr.iter()\n");
    restore_body.push_str("        .map(|v| deserialize_comp(v))\n");
    restore_body.push_str("        .collect())\n");
    restore_body.push_str("    .unwrap_or_default();\n");

    restore_body.push_str("let compartment = deserialize_comp(&data[\"_compartment\"]);\n");

    restore_body.push_str(&format!("let instance = {} {{\n", system.name));
    restore_body.push_str("    _state_stack: stack,\n");
    restore_body.push_str("    _context_stack: vec![],\n");
    restore_body.push_str("    __compartment: compartment,\n");
    restore_body.push_str("    __next_compartment: None,\n");

    for var in &system.domain {
        let json_extract = rust_json_extract_unwrap(&var.name, &var.var_type);
        restore_body.push_str(&format!("    {}: {},\n", var.name, json_extract));
    }

    restore_body.push_str("};\n");
    restore_body.push_str("instance");

    methods.push(CodegenNode::Method {
        name: "restore_state".to_string(),
        params: vec![Param::new("json").with_type("&str")],
        return_type: Some(system.name.clone()),
        body: vec![CodegenNode::NativeBlock {
            code: restore_body,
            span: None,
        }],
        is_async: false,
        is_static: true,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    methods
}

fn rust_json_extract(var_name: &str, var_type: &Type) -> String {
    match var_type {
        Type::Custom(name) => match name.to_lowercase().as_str() {
            "int" | "i32" => format!("data[\"{}\"].as_i64().unwrap_or(0) as i32", var_name),
            "i64" => format!("data[\"{}\"].as_i64().unwrap_or(0)", var_name),
            "float" | "f32" | "f64" => {
                format!("data[\"{}\"].as_f64().unwrap_or(0.0)", var_name)
            }
            "bool" => format!("data[\"{}\"].as_bool().unwrap_or(false)", var_name),
            "str" | "string" => {
                format!(
                    "data[\"{}\"].as_str().unwrap_or(\"\").to_string()",
                    var_name
                )
            }
            _ => format!(
                "serde_json::from_value(data[\"{}\"].clone()).unwrap_or_default()",
                var_name
            ),
        },
        _ => format!(
            "serde_json::from_value(data[\"{}\"].clone()).unwrap_or_default()",
            var_name
        ),
    }
}

fn rust_json_extract_unwrap(var_name: &str, var_type: &Type) -> String {
    match var_type {
        Type::Custom(name) => match name.to_lowercase().as_str() {
            "int" | "i32" => format!("data[\"{}\"].as_i64().unwrap() as i32", var_name),
            "i64" => format!("data[\"{}\"].as_i64().unwrap()", var_name),
            "float" | "f32" | "f64" => format!("data[\"{}\"].as_f64().unwrap()", var_name),
            "bool" => format!("data[\"{}\"].as_bool().unwrap()", var_name),
            "str" | "string" => {
                format!("data[\"{}\"].as_str().unwrap().to_string()", var_name)
            }
            _ => format!(
                "serde_json::from_value(data[\"{}\"].clone()).unwrap()",
                var_name
            ),
        },
        _ => format!(
            "serde_json::from_value(data[\"{}\"].clone()).unwrap()",
            var_name
        ),
    }
}
