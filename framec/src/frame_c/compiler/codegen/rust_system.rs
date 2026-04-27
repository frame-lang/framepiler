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
use super::state_dispatch::{generate_handler_from_arcanum, handler_method_name};
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

    // Actions + operations (shared — native passthrough).
    // `generate_action`/`generate_operation` return `Vec<CodegenNode>`
    // so trivia (leading-comment NativeBlocks) can prepend the
    // method node.
    for action in &system.actions {
        methods.extend(super::interface_gen::generate_action(
            action, &syntax, source,
        ));
    }
    for operation in &system.operations {
        methods.extend(super::interface_gen::generate_operation(
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
        base_classes: system.bases.clone(),
        is_abstract: false,
        derives: vec![],
        visibility: if system.visibility.as_deref() == Some("private") {
            Visibility::Private
        } else {
            Visibility::Public
        },
    };

    if needs_async {
        super::system_codegen::make_system_async(&mut class_node, &system.name, lang);
    }

    // Auto-clone non-Copy domain fields passed by value to Frame calls.
    // See `apply_rust_auto_clone` for semantics — this closes the borrow-check
    // hole where `self.do_resolve(self.name)` with a String `name` would be
    // rejected by rustc.
    let call_targets = rust_frame_call_targets(system);
    let non_copy_fields = rust_non_copy_domain_fields(system);
    apply_rust_auto_clone(&mut class_node, &call_targets, &non_copy_fields);

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

        let mut field = Field::new(&domain_var.name)
            .with_visibility(Visibility::Public)
            .with_leading_comments(domain_var.leading_comments.clone());
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
                    let expanded = expand_tagged_in_domain(init_text, TargetLanguage::Rust);
                    // Wrap string-literal defaults in `String::from(...)`
                    // when the field type maps to Rust's `String` — without
                    // this, `s: str = ""` in Frame produces `s: ""` in the
                    // constructor, which fails to compile because `""` is
                    // `&'static str` not `String`.
                    let type_str = match &domain_var.var_type {
                        Type::Custom(s) => Some(s.clone()),
                        Type::Unknown => None,
                    };
                    rust_wrap_for_boxing(&expanded, &type_str)
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

    // Compartment creation for start state — placeholder in struct
    // literal (just the leaf state, no ancestors). After the struct is
    // built we replace via __prepareEnter so the chain cascades through
    // the runtime helper exactly like every other transition.
    if let Some(ref machine) = system.machine {
        if let Some(first_state) = machine.states.first() {
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                CodegenNode::Ident(format!(
                    "{}Compartment::new(\"{}\")",
                    system.name, first_state.name
                )),
            ));
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                CodegenNode::Ident("None".to_string()),
            ));

            // Build the start chain via __prepareEnter, passing system
            // header EnterArg params as the enter_args payload. The
            // helper writes the Vec into every layer's `enter_args`
            // field so the cascade's per-layer `$>` events all see the
            // same args (signature-match rule).
            let enter_arg_pushes: Vec<String> = system
                .params
                .iter()
                .filter(|p| matches!(p.kind, ParamKind::EnterArg))
                .map(|p| format!("self.__sys_{}.to_string()", p.name))
                .collect();
            body.push(CodegenNode::NativeBlock {
                code: format!(
                    "self.__compartment = self.__prepareEnter(\"{}\", vec![{}]);",
                    first_state.name,
                    enter_arg_pushes.join(", ")
                ),
                span: None,
            });

            // Fire $> cascade. Push a frame context so handlers that
            // read @@:return / @@:data have a stack entry, then drive
            // the cascade through the same helpers a regular transition
            // uses.
            let event_class = format!("{}FrameEvent", system.name);
            let context_class = format!("{}FrameContext", system.name);
            body.push(CodegenNode::NativeBlock {
                code: format!(
                    "let __frame_event = {}::new_with_params(\"$>\", &self.__compartment.enter_args);\n\
                     let __ctx = {}::new(__frame_event, None);\n\
                     self._context_stack.push(__ctx);\n\
                     self.__fire_enter_cascade();\n\
                     self.__process_transition_loop();\n\
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
    let chains = super::system_codegen::compute_hsm_chains(system);

    // __hsm_chain — match on leaf, returns the chain as a static slice.
    let mut chain_body = String::from("match leaf {\n");
    for (leaf, chain) in &chains {
        let entries = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_body.push_str(&format!("    \"{}\" => &[{}],\n", leaf, entries));
    }
    chain_body.push_str("    _ => &[],\n}");
    methods.push(CodegenNode::Method {
        name: "__hsm_chain".to_string(),
        params: vec![Param::new("leaf").with_type("&str")],
        return_type: Some("&'static [&'static str]".to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: chain_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareEnter — build the destination HSM chain leaf-down. The
    // `enter_args` Vec is cloned into every layer's `enter_args` field
    // so the cascade's per-layer synthesized `$>` events all carry
    // the same payload (per the signature-match rule in
    // docs/frame_runtime.md § "How propagation works in the runtime").
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf").with_type("&str"),
            Param::new("enter_args").with_type("Vec<String>"),
        ],
        return_type: Some(compartment_class.to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"let chain = self.__hsm_chain(leaf);
let mut comp: Option<{0}> = None;
for name in chain.iter() {{
    let mut new_comp = {0}::new(name);
    new_comp.enter_args = enter_args.clone();
    if let Some(parent) = comp.take() {{
        new_comp.parent_compartment = Some(Box::new(parent));
    }}
    comp = Some(new_comp);
}}
comp.expect("chain must contain at least the leaf state")"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populate exit_args on every layer of current chain.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type("Vec<String>")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"self.__compartment.exit_args = exit_args.clone();
let mut cursor = self.__compartment.parent_compartment.as_deref_mut();
while let Some(c) = cursor {
    c.exit_args = exit_args.clone();
    cursor = c.parent_compartment.as_deref_mut();
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router. Same dispatch table as __router
    // but takes an explicit state name.
    let mut route_code = String::from("match state_name {\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            route_code.push_str(&format!(
                "    \"{}\" => self._state_{}(__e),\n",
                state.name, state.name
            ));
        }
    }
    route_code.push_str("    _ => {}\n}");
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("&str"),
            Param::new("__e").with_type(&format!("&{}", event_class)),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up. Walk chain from leaf to root,
    // firing <$ on each. Rust's borrow checker requires us to collect
    // (state_name, exit_args) pairs first, then route.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"let mut layers: Vec<(String, Vec<String>)> = Vec::new();
{{
    let mut cursor = Some(&self.__compartment);
    while let Some(c) = cursor {{
        layers.push((c.state.clone(), c.exit_args.clone()));
        cursor = c.parent_compartment.as_deref();
    }}
}}
for (state_name, args) in &layers {{
    let exit_event = {}::new_with_params("<$", args);
    self.__route_to_state(state_name, &exit_event);
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

    // __fire_enter_cascade — top-down. Walk chain to collect layers,
    // iterate in reverse to fire $> root-first.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"let mut layers: Vec<(String, Vec<String>)> = Vec::new();
{{
    let mut cursor = Some(&self.__compartment);
    while let Some(c) = cursor {{
        layers.push((c.state.clone(), c.enter_args.clone()));
        cursor = c.parent_compartment.as_deref();
    }}
}}
for (state_name, args) in layers.iter().rev() {{
    let enter_event = {}::new_with_params("$>", args);
    self.__route_to_state(state_name, &enter_event);
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

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"while self.__next_compartment.is_some() {{
    let next_compartment = self.__next_compartment.take().unwrap();
    self.__fire_exit_cascade();
    self.__compartment = next_compartment;
    if self.__compartment.forward_event.is_none() {{
        self.__fire_enter_cascade();
    }} else {{
        let forward_event = self.__compartment.forward_event.take().unwrap();
        self.__fire_enter_cascade();
        if forward_event.message != "$>" {{
            self.__router(&forward_event);
        }}
    }}
    let _ = {}::new("$>");  // satisfy unused-import / type checks
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

    // __kernel — thin: route then drain.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"let __e = self._context_stack.last().unwrap().event.clone();
self.__router(&__e);
self.__process_transition_loop();"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — dispatches events to current state method via match.
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

    // Only the lifecycle `$>` key signals an explicit enter handler. A user
    // interface method named `enter` is a regular event — see
    // bug_enter_exit_method_collision for why the old aliasing was wrong.
    let _has_enter_handler = handlers.contains_key("$>");
    let _needs_state_var_init = !state_vars.is_empty();

    for (event, handler) in sorted_handlers {
        // Wire message and per-handler method are keyed only off the sigil
        // lifecycle events. A user-defined `enter` / `exit` falls through to
        // the default arm and dispatches under its own name.
        let message = match event.as_str() {
            "$>" => "$>",
            "$<" => "<$",
            _ => event.as_str(),
        };

        let handler_method = handler_method_name(state_name, handler);

        let is_lifecycle = event == "$>" || event == "$<";
        if !handler.params.is_empty() && is_lifecycle {
            if is_start_state {
                code.push_str(&format!(
                    "    \"{}\" => {{ self.{}(__e); }}\n",
                    message, handler_method
                ));
                continue;
            }
            code.push_str(&format!("    \"{}\" => {{\n", message));
            for (idx, param) in handler.params.iter().enumerate() {
                // Normalize Frame type keywords so the Rust-typed match
                // arms below catch them (same concern as the handler-
                // dispatch block further down).
                let raw_type = param.symbol_type.as_deref().unwrap_or("String");
                let param_type = match raw_type {
                    "int" => "i64",
                    "float" => "f64",
                    other => other,
                };
                let extraction = match param_type {
                    "String" | "str" | "string" => format!(
                        "        let {}: String = __e.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();\n",
                        param.name, idx
                    ),
                    "i32" => format!(
                        "        let {}: i32 = __e.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name, idx
                    ),
                    "i64" => format!(
                        "        let {}: i64 = __e.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name, idx
                    ),
                    "f64" | "f32" => format!(
                        "        let {}: {} = __e.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name, param_type, idx
                    ),
                    "bool" => format!(
                        "        let {}: bool = __e.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name, idx
                    ),
                    _ => format!(
                        "        let {} = __e.parameters.get({}).and_then(|v| v.downcast_ref::<{}>()).cloned().unwrap_or_default();\n",
                        param.name, idx, param_type
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
            for (idx, param) in handler.params.iter().enumerate() {
                // Normalize Frame type keywords to the Rust target type
                // so the dispatch arms below (which are keyed on Rust
                // types) match correctly. Without this, `v: int` in a
                // Frame handler falls through to the catch-all and
                // unpacks as String, mismatching the `_s_*_handler(_, v: i64)`
                // method signature.
                let raw_type = param.symbol_type.as_deref().unwrap_or("String");
                let param_type = match raw_type {
                    "int" => "i64",
                    "float" => "f64",
                    other => other,
                };
                let extraction = match param_type {
                    "String" | "str" | "string" => format!(
                        "        let {}: String = __ctx_event.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();\n",
                        param.name, idx
                    ),
                    "i64" | "i32" | "isize" => format!(
                        "        let {}: {} = __ctx_event.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name, param_type, idx
                    ),
                    "f64" | "f32" => format!(
                        "        let {}: {} = __ctx_event.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name, param_type, idx
                    ),
                    "bool" => format!(
                        "        let {}: bool = __ctx_event.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).and_then(|s| s.parse().ok()).unwrap_or_default();\n",
                        param.name, idx
                    ),
                    _ => format!(
                        "        let {} = __ctx_event.parameters.get({}).and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();\n",
                        param.name, idx
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

    // Build the state→parent map once for all handler emissions.
    // Used by transition codegen to propagate state-args through
    // every HSM ancestor's typed StateContext variant.
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
    let start_state_param_names: Vec<String> = arcanum
        .get_enhanced_states(system_name)
        .iter()
        .find(|s| s.name == start_state_name)
        .map(|s| s.params.iter().map(|p| p.name.clone()).collect())
        .unwrap_or_default();

    // Iterate via machine.states (Vec, deterministic declaration order)
    // and look up the enhanced state by name. arcanum.get_enhanced_states
    // returns HashMap-iteration-ordered values which differ between
    // framec runs and break downstream caches (ccache).
    for state_ast_iter in machine.states.iter() {
        let state_entry = match arcanum.get_enhanced_state(system_name, &state_ast_iter.name) {
            Some(e) => e,
            None => continue,
        };
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

        // Sort by event name for deterministic emission order — see
        // state_dispatch.rs comment for context (matrix ccache hit
        // rate dropped to ~70% on C without this).
        let mut sorted_state_handlers: Vec<_> = state_entry.handlers.iter().collect();
        sorted_state_handlers.sort_by(|a, b| a.0.cmp(b.0));
        for (_event, handler_entry) in sorted_state_handlers {
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
                &std::collections::HashMap::new(), // event_param_names — Rust uses typed params directly
                &handler_state_var_types,
                &state_hsm_parents,
            );
            // Per-handler leading-comment trivia. Same shape as
            // `generate_per_handler_methods` in state_dispatch.rs:
            // emit each comment as a class-scope NativeBlock above
            // the handler method.
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

    let mut code = if method.params.is_empty() {
        format!("let mut __e = {}::new(\"{}\");\n", event_class, method.name)
    } else {
        let param_items: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("Box::new({}.to_string()) as Box<dyn std::any::Any>", p.name))
            .collect();
        let mut s = format!("let mut __e = {}::new(\"{}\");\n", event_class, method.name);
        s.push_str(&format!(
            "__e.parameters = vec![{}];\n",
            param_items.join(", ")
        ));
        s
    };

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
use super::frame_expansion::resolve_state_arg_key;

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
    // Per-handler architecture with helpers (docs/frame_runtime.md
    // Step 21+): __prepareEnter / __prepareExit / __transition.
    //
    // For HSM, the spec's signature-match rule (Step 22) means
    // exit_args, enter_args, and state_args propagate identically to
    // every layer of the source chain (exit) and destination chain
    // (enter / state). We honour that by:
    //   * routing exit_args through __prepareExit (writes to every
    //     source-chain layer),
    //   * routing enter_args through __prepareEnter (writes to every
    //     destination-chain layer),
    //   * walking the destination chain after construction and
    //     pattern-matching each layer's typed StateContext variant
    //     to populate its state-arg fields.
    //
    // The destination chain is known at codegen time via
    // `ctx.state_hsm_parents` so we emit a depth-N nested if-let
    // rather than a runtime walk — the generated code is also more
    // readable than a `match c.state.as_str()` chain.

    let mut code = String::new();

    // ---- exit_args → __prepareExit (every source-chain layer) ----
    if let Some(ref exit) = exit_str {
        let vals: Vec<String> = exit
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .map(|arg| {
                let raw = if let Some(eq_pos) = arg.find('=') {
                    arg[eq_pos + 1..].trim()
                } else {
                    arg
                };
                format!("{}.to_string()", raw)
            })
            .collect();
        if !vals.is_empty() {
            code.push_str(&format!(
                "{}self.__prepareExit(vec![{}]);\n",
                indent_str,
                vals.join(", ")
            ));
        }
    }

    // ---- enter_args → __prepareEnter (every destination-chain layer) ----
    let enter_args_vec = if let Some(ref enter) = enter_str {
        enter
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .map(|arg| {
                let raw = if let Some(eq_pos) = arg.find('=') {
                    arg[eq_pos + 1..].trim()
                } else {
                    arg
                };
                format!("{}.to_string()", raw)
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    code.push_str(&format!(
        "{}let mut __compartment = self.__prepareEnter(\"{}\", vec![{}]);\n",
        indent_str,
        target,
        enter_args_vec.join(", ")
    ));

    // ---- state_args → typed StateContext on every layer ----
    // Build the destination chain leaf→root from ctx.state_hsm_parents.
    if let Some(ref state) = state_str {
        let args: Vec<(String, String)> = state
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .enumerate()
            .map(|(i, arg)| {
                if let Some(eq_pos) = arg.find('=') {
                    (
                        arg[..eq_pos].trim().to_string(),
                        arg[eq_pos + 1..].trim().to_string(),
                    )
                } else {
                    (resolve_state_arg_key(i, target, ctx), arg.to_string())
                }
            })
            .collect();

        if !args.is_empty() {
            // Compute the destination chain (leaf → root). The
            // signature-match rule guarantees every layer in the
            // chain accepts the same state-arg names, so we write
            // identical assignments at every depth.
            let mut chain: Vec<String> = vec![target.to_string()];
            let mut cursor = target.to_string();
            while let Some(parent) = ctx.state_hsm_parents.get(&cursor) {
                chain.push(parent.clone());
                cursor = parent.clone();
            }

            // Open block scope so multiple transitions in the same
            // handler don't collide on local names.
            code.push_str(&format!("{}{{\n", indent_str));

            // Leaf — direct field on __compartment.
            let leaf = &chain[0];
            code.push_str(&format!(
                "{0}    if let {1}StateContext::{2}(ref mut ctx) = __compartment.state_context {{\n",
                indent_str, ctx.system_name, leaf
            ));
            for (k, v) in &args {
                code.push_str(&format!("{}        ctx.{} = {};\n", indent_str, k, v));
            }
            code.push_str(&format!("{}    }}\n", indent_str));

            // Ancestors — walk via parent_compartment chain. Use a
            // depth-N nested if-let so the borrow checker has a
            // single linear chain of mutable reborrows; each step
            // narrows scope so prior `ref mut` bindings are dropped.
            for depth in 1..chain.len() {
                let pad: String = "    ".repeat(depth);
                let state = &chain[depth];
                // Open `if let Some(ref mut p_<depth>)` for each ancestor.
                code.push_str(&format!(
                    "{0}    {1}if let Some(ref mut __anc_{2}) = ",
                    indent_str, pad, depth
                ));
                if depth == 1 {
                    code.push_str("__compartment.parent_compartment {\n");
                } else {
                    code.push_str(&format!("__anc_{}.parent_compartment {{\n", depth - 1));
                }
                code.push_str(&format!(
                    "{0}    {1}    if let {2}StateContext::{3}(ref mut ctx) = __anc_{4}.state_context {{\n",
                    indent_str, pad, ctx.system_name, state, depth
                ));
                for (k, v) in &args {
                    code.push_str(&format!(
                        "{}    {}        ctx.{} = {};\n",
                        indent_str, pad, k, v
                    ));
                }
                code.push_str(&format!("{}    {}    }}\n", indent_str, pad));
            }
            // Close all the `if let Some` scopes (ancestors).
            for depth in (1..chain.len()).rev() {
                let pad: String = "    ".repeat(depth);
                code.push_str(&format!("{}    {}}}\n", indent_str, pad));
            }
            // Close block scope.
            code.push_str(&format!("{}}}\n", indent_str));
        }
    }

    code.push_str(&format!(
        "{}self.__transition(__compartment);\n{}return;",
        indent_str, indent_str
    ));
    code
}

/// Rust transition-forward: build chain via __prepareEnter, set forward_event.
pub(crate) fn rust_expand_forward_transition(
    indent_str: &str,
    _ctx: &HandlerContext,
    target: &str,
) -> String {
    let mut code = String::new();
    // Forward transition supplies no explicit enter_args — the
    // forwarded event carries its own params, dispatched after the
    // enter cascade by __process_transition_loop.
    code.push_str(&format!(
        "{}let mut __compartment = self.__prepareEnter(\"{}\", Vec::new());\n",
        indent_str, target
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

/// Rust state variable write — walks the compartment chain via
/// `Option::as_deref_mut` to find the layer that owns the variable
/// (matched by state name), then writes through its typed
/// `StateContext` variant. RHS is evaluated first so it doesn't
/// race the cursor's mutable borrow of `self.__compartment`.
pub(crate) fn rust_expand_state_var_write(
    indent_str: &str,
    ctx: &HandlerContext,
    var_name: &str,
    expanded_expr: &str,
) -> String {
    format!(
        concat!(
            "{0}{{\n",
            "{0}    let __rhs = {1};\n",
            "{0}    let mut __cursor: Option<&mut {2}Compartment> = Some(&mut self.__compartment);\n",
            "{0}    while let Some(__c) = __cursor {{\n",
            "{0}        if __c.state == \"{3}\" {{\n",
            "{0}            if let {2}StateContext::{3}(ref mut ctx) = __c.state_context {{\n",
            "{0}                ctx.{4} = __rhs;\n",
            "{0}            }}\n",
            "{0}            break;\n",
            "{0}        }}\n",
            "{0}        __cursor = __c.parent_compartment.as_deref_mut();\n",
            "{0}    }}\n",
            "{0}}}"
        ),
        indent_str, expanded_expr, ctx.system_name, ctx.state_name, var_name
    )
}

/// Rust return-value boxing — wraps expression in Box<dyn Any>, handles
/// string literal conversion (&str → String) and integer/float literal
/// casts (i32 → i64, f32 → f64) to match the interface method's declared
/// Rust return type.
pub(crate) fn rust_expand_box_return(
    indent_str: &str,
    expanded_expr: &str,
    return_type: &Option<String>,
) -> String {
    let boxed_expr = rust_wrap_for_boxing(expanded_expr, return_type);
    format!(
        "{}let __return_val = Box::new({}) as Box<dyn std::any::Any>;\n\
         {}if let Some(ctx) = self._context_stack.last_mut() {{ ctx._return = Some(__return_val); }}",
        indent_str, boxed_expr, indent_str
    )
}

/// Rust return-value boxing (no leading indent on first line — used in
/// ReturnCall/ContextReturnExpr where the caller provides the indent).
pub(crate) fn rust_expand_box_return_bare(
    indent_str: &str,
    expanded_expr: &str,
    return_type: &Option<String>,
) -> String {
    let boxed_expr = rust_wrap_for_boxing(expanded_expr, return_type);
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

/// Wrap a value expression so the `Box<dyn Any>` it ends up in can be
/// downcast to exactly the type the enclosing Frame method's return
/// signature resolves to on Rust.
///
/// Three cases matter:
///   1. Frame type `str` (or a `"..."` literal with no declared type):
///      wrap as `String::from("...")`. The downcast expects `String`,
///      not `&'static str`.
///   2. Frame type `int`: add `as i64`. Integer literals in Rust default
///      to `i32`; the interface signature emits `-> i64`, so without
///      this cast the box contains `i32` and the downcast panics.
///   3. Frame type `float`: add `as f64` for the same reason.
///
/// Non-literal expressions that are already the correct type get a
/// redundant cast, which the compiler elides.
fn rust_wrap_for_boxing(expr: &str, return_type: &Option<String>) -> String {
    let trimmed = expr.trim();
    let is_string_literal = trimmed.starts_with('"') && trimmed.ends_with('"');
    match return_type.as_deref() {
        Some("int") => format!("({}) as i64", trimmed),
        Some("float") => format!("({}) as f64", trimmed),
        Some("str") | Some("string") | Some("String") | Some("Any") if is_string_literal => {
            format!("String::from({})", trimmed)
        }
        _ if is_string_literal => format!("String::from({})", trimmed),
        _ => expr.to_string(),
    }
}

/// Back-compat shim retained for callers that haven't been threaded with
/// the return-type context yet. Equivalent to passing `None`.
#[allow(dead_code)]
fn rust_wrap_string_literal(expr: &str) -> String {
    rust_wrap_for_boxing(expr, &None)
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

/// `@@:params[key]` — context parameter access.
/// In Rust, handler params are bound as typed locals in scope,
/// so @@:params.name just references the local variable directly.
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

/// `@@:return` read with a declared return type — downcasts the
/// boxed `Any` to the Frame type's native Rust representation so
/// the read is usable as an rvalue in expressions and as a typed
/// arg to a self-call. Falls back to the untyped `Option<&Box>`
/// form when the handler's return type is unknown/unsupported.
pub(crate) fn rust_context_return_read_typed(frame_type: &str) -> String {
    let base = "self._context_stack.last().and_then(|ctx| ctx._return.as_ref())";
    match frame_type {
        "int" => format!(
            "({}.and_then(|b| b.downcast_ref::<i64>()).copied().unwrap_or(0))",
            base
        ),
        "bool" => format!(
            "({}.and_then(|b| b.downcast_ref::<bool>()).copied().unwrap_or(false))",
            base
        ),
        "str" => format!(
            "({}.and_then(|b| b.downcast_ref::<String>()).cloned().unwrap_or_default())",
            base
        ),
        _ => base.to_string(),
    }
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

/// Push-with-transition: clone current compartment onto the state
/// stack (preserving its full HSM chain), build the destination
/// chain via __prepareEnter, then queue the new compartment via
/// __transition. The kernel's __process_transition_loop fires the
/// exit/enter cascades — same pipeline as a normal transition.
pub(crate) fn rust_push_transition(
    indent_str: &str,
    ctx: &super::codegen_utils::HandlerContext,
    target: &str,
) -> String {
    format!(
        "{0}self._state_stack.push(self.__compartment.clone());\n\
         {0}let __compartment = self.__prepareEnter(\"{1}\", Vec::new());\n\
         {0}self.__transition(__compartment);\n\
         {0}return;",
        indent_str, target
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

/// Pop: exit_args write (positional push)
pub(crate) fn rust_pop_exit_arg(indent: &str, value: &str) -> String {
    format!(
        "{}self.__compartment.exit_args.push({}.to_string());\n",
        indent, value
    )
}

/// Pop: stack pop
pub(crate) fn rust_pop_stack(indent: &str) -> String {
    format!(
        "{}let mut __popped = self._state_stack.pop().unwrap();\n",
        indent
    )
}

/// Pop: enter_args write (positional push)
pub(crate) fn rust_pop_enter_arg(indent: &str, value: &str) -> String {
    format!(
        "{}__popped.enter_args.push({}.to_string());\n",
        indent, value
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
            // Frame `int` maps to Rust `i64` (see RustBackend::convert_type).
            // Only the explicit `i32` hint narrows.
            "int" | "i64" => format!("data[\"{}\"].as_i64().unwrap_or(0)", var_name),
            "i32" => format!("data[\"{}\"].as_i64().unwrap_or(0) as i32", var_name),
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
            // Frame `int` maps to Rust `i64`; only explicit `i32` narrows.
            "int" | "i64" => format!("data[\"{}\"].as_i64().unwrap()", var_name),
            "i32" => format!("data[\"{}\"].as_i64().unwrap() as i32", var_name),
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

// ═══════════════════════════════════════════════════════════════════════
// Rust auto-clone for non-Copy domain fields passed to Frame calls
// ═══════════════════════════════════════════════════════════════════════
//
// Frame handlers and actions are written as native target-language code, but
// the constructs `self.<action>(...)`, `self.<operation>(...)`, and
// `self.<interface_method>(...)` are Frame-structural: they name Frame-declared
// callables and framec owns their emission semantics (the Erlang backend, for
// instance, rewrites `self.next_pid()` → `next_pid(Data)`).
//
// Rust's ownership semantics bite at the arg boundary: `self.do_resolve(self.name)`
// moves `self.name` out of `self`, which the borrow checker rejects because
// `self` is already mutably borrowed for the outer method call. The idiomatic
// user-written fix is `self.name.clone()`.
//
// Rather than ask recipe authors to write `.clone()` at every Frame-call site
// that passes a non-Copy domain field — and have to reason about when it's
// needed vs. superfluous — we apply the rewrite structurally here. It matches
// the precedent set by the Erlang classifier for `self.<action>(...)` rewrites.
//
// Scope of rewrite:
//   • Triggers only when the call target is a declared Frame action,
//     operation, or interface method of this system.
//   • Rewrites only arguments of the form `self.<field>` (exactly, after
//     trimming) where `<field>` is a domain field with a non-Copy type.
//   • Leaves alone: literals, arithmetic expressions, already-cloned args,
//     `&self.field` borrows, and any `self.field` passed to non-Frame calls.
//
// The tokenizer walks char-by-char, skipping string literals and line
// comments, and balances parens/brackets/braces when splitting the arg list.

/// Maps a Frame type string to whether the generated Rust type is `Copy`.
/// Conservative: unknown or custom types are assumed non-Copy.
fn rust_type_is_copy(type_str: &str) -> bool {
    let t = type_str.trim();
    matches!(
        t,
        "int"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "float"
            | "f32"
            | "f64"
            | "bool"
            | "boolean"
            | "char"
            | "()"
            | "number"
    )
}

/// Collect the names of all domain fields whose declared type maps to a
/// non-Copy Rust type. These are the fields that need `.clone()` when passed
/// by value to a Frame call.
fn rust_non_copy_domain_fields(system: &SystemAst) -> Vec<String> {
    system
        .domain
        .iter()
        .filter_map(|var| {
            let type_str = match &var.var_type {
                Type::Custom(s) => s.clone(),
                Type::Unknown => return None,
            };
            if rust_type_is_copy(&type_str) {
                None
            } else {
                Some(var.name.clone())
            }
        })
        .collect()
}

/// Collect the names of every Frame-callable from this system — actions,
/// operations, and interface methods. These are the call targets where an
/// auto-clone rewrite is in scope.
fn rust_frame_call_targets(system: &SystemAst) -> Vec<String> {
    let mut names = Vec::new();
    for a in &system.actions {
        names.push(a.name.clone());
    }
    for o in &system.operations {
        names.push(o.name.clone());
    }
    for m in &system.interface {
        names.push(m.name.clone());
    }
    names
}

/// Post-pass that walks a generated Rust `CodegenNode::Class` tree and applies
/// `rust_auto_clone_in_code` to every `NativeBlock` in every method body.
fn apply_rust_auto_clone(
    class: &mut CodegenNode,
    call_targets: &[String],
    non_copy_fields: &[String],
) {
    if non_copy_fields.is_empty() || call_targets.is_empty() {
        return;
    }
    if let CodegenNode::Class { methods, .. } = class {
        for method in methods.iter_mut() {
            if let CodegenNode::Method { body, .. } = method {
                for stmt in body.iter_mut() {
                    rewrite_native_blocks_in_node(stmt, call_targets, non_copy_fields);
                }
            }
        }
    }
}

fn rewrite_native_blocks_in_node(
    node: &mut CodegenNode,
    call_targets: &[String],
    non_copy_fields: &[String],
) {
    match node {
        CodegenNode::NativeBlock { code, .. } => {
            *code = rust_auto_clone_in_code(code, call_targets, non_copy_fields);
        }
        CodegenNode::If {
            then_block,
            else_block,
            ..
        } => {
            for stmt in then_block.iter_mut() {
                rewrite_native_blocks_in_node(stmt, call_targets, non_copy_fields);
            }
            if let Some(else_b) = else_block {
                for stmt in else_b.iter_mut() {
                    rewrite_native_blocks_in_node(stmt, call_targets, non_copy_fields);
                }
            }
        }
        CodegenNode::While { body, .. } | CodegenNode::For { body, .. } => {
            for stmt in body.iter_mut() {
                rewrite_native_blocks_in_node(stmt, call_targets, non_copy_fields);
            }
        }
        CodegenNode::Match { arms, .. } => {
            for arm in arms.iter_mut() {
                for stmt in arm.body.iter_mut() {
                    rewrite_native_blocks_in_node(stmt, call_targets, non_copy_fields);
                }
            }
        }
        CodegenNode::Method { body, .. } => {
            for stmt in body.iter_mut() {
                rewrite_native_blocks_in_node(stmt, call_targets, non_copy_fields);
            }
        }
        _ => {}
    }
}

/// Walk a block of native Rust code and rewrite args of Frame-call sites.
///
/// For every `self.<target>(<args>)` where `<target>` is in `call_targets`,
/// split `<args>` on top-level commas (balanced across parens/brackets/braces
/// and ignoring commas inside string literals), and for each arg whose trimmed
/// text exactly matches `self.<field>` with `<field>` in `non_copy_fields`,
/// rewrite to `self.<field>.clone()`.
fn rust_auto_clone_in_code(
    code: &str,
    call_targets: &[String],
    non_copy_fields: &[String],
) -> String {
    // Two rewrites in one pass, both gated by the Rust `SyntaxSkipper`
    // so neither can accidentally edit code inside a string literal or
    // comment:
    //   A. `self.<target>(args)` where <target> is a Frame call —
    //      non-copy-field args get `.clone()`. (The original pass.)
    //   B. `Box::new(self.<field>)` for non-Copy <field> — rewrite to
    //      `Box::new(self.<field>.clone())` so `@@:(self.s)` return-
    //      value boxing compiles. Without this, rustc rejects with
    //      E0507 (move out of borrowed `&mut self`).
    //
    // String / comment skipping is delegated to `RustSkipper` rather
    // than re-implemented inline; "never duplicate scanner logic in
    // codegen" per the project compiler-discipline rule.
    let skipper = crate::frame_c::compiler::native_region_scanner::create_skipper(
        crate::frame_c::visitors::TargetLanguage::Rust,
    );
    let bytes = code.as_bytes();
    let end = bytes.len();
    let mut out = String::with_capacity(code.len());
    let mut i = 0;
    while i < end {
        // Delegate string/comment skipping to the Rust skipper.
        if let Some(next) = skipper.skip_string(bytes, i, end) {
            out.push_str(&code[i..next]);
            i = next;
            continue;
        }
        if let Some(next) = skipper.skip_comment(bytes, i, end) {
            out.push_str(&code[i..next]);
            i = next;
            continue;
        }
        // Rewrite B: `Box::new(self.<field>)` for a non-Copy field.
        if let Some((end_idx, field)) = try_match_box_self_field(bytes, i, non_copy_fields) {
            out.push_str("Box::new(self.");
            out.push_str(field);
            out.push_str(".clone())");
            i = end_idx;
            continue;
        }
        // Rewrite A: `self.<target>(args)` where <target> is a Frame call.
        if looks_like_self_call(bytes, i) {
            let target_start = i + 5; // len("self.")
            let (target_end, paren_pos) = match find_call_target(bytes, target_start) {
                Some(pair) => pair,
                None => {
                    out.push(bytes[i] as char);
                    i += 1;
                    continue;
                }
            };
            let target = &code[target_start..target_end];
            if !call_targets.iter().any(|t| t == target) {
                out.push(bytes[i] as char);
                i += 1;
                continue;
            }
            // Find matching close paren via the Rust skipper. It
            // returns the position AFTER `)`; `close` is the position
            // OF `)` to match the slice-range semantics below.
            let after_close = match skipper.balanced_paren_end(bytes, paren_pos, end) {
                Some(p) => p,
                None => {
                    out.push(bytes[i] as char);
                    i += 1;
                    continue;
                }
            };
            let close = after_close - 1;
            // Emit `self.<target>(`
            out.push_str(&code[i..paren_pos + 1]);
            // Split args, rewrite each if it matches the non-copy pattern.
            let args_src = &code[paren_pos + 1..close];
            let rewritten_args = split_top_level_args(args_src)
                .into_iter()
                .map(|arg| rewrite_arg_if_non_copy_field(&arg, non_copy_fields))
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&rewritten_args);
            out.push(')');
            i = after_close;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// If `bytes[i..]` starts with `Box::new(self.<field>)` for any `<field>`
/// in `non_copy_fields`, return `(end_index_after_close_paren, field_name)`.
/// Otherwise `None`. Pure prefix match — caller handles the rewrite.
fn try_match_box_self_field<'a>(
    bytes: &[u8],
    i: usize,
    non_copy_fields: &'a [String],
) -> Option<(usize, &'a str)> {
    const PREFIX: &[u8] = b"Box::new(self.";
    if i + PREFIX.len() >= bytes.len() || &bytes[i..i + PREFIX.len()] != PREFIX {
        return None;
    }
    let field_start = i + PREFIX.len();
    let mut j = field_start;
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    if j == field_start || j >= bytes.len() || bytes[j] != b')' {
        return None;
    }
    let field_bytes = &bytes[field_start..j];
    for f in non_copy_fields {
        if f.as_bytes() == field_bytes {
            return Some((j + 1, f.as_str()));
        }
    }
    None
}

fn looks_like_self_call(bytes: &[u8], i: usize) -> bool {
    // Need "self." here, and not preceded by an ident char (so "myself."
    // doesn't match).
    if i + 5 > bytes.len() {
        return false;
    }
    if &bytes[i..i + 5] != b"self." {
        return false;
    }
    if i > 0 {
        let prev = bytes[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return false;
        }
    }
    true
}

/// From a position right after `self.`, find the end of the identifier and
/// the position of the `(` that starts the call. Skips whitespace between
/// ident and `(`. Returns `None` if no call (`(`) follows the identifier.
fn find_call_target(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut end = start;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if end == start {
        return None;
    }
    let mut j = end;
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    if j < bytes.len() && bytes[j] == b'(' {
        Some((end, j))
    } else {
        None
    }
}

/// Split an argument-list string on top-level commas, preserving commas inside
/// nested `()`, `[]`, `{}`, and inside string literals.
fn split_top_level_args(src: &str) -> Vec<String> {
    // Delegate string-literal skipping to the Rust skipper; only the
    // depth tracking + comma-splitting is the specialized concern here.
    let skipper = crate::frame_c::compiler::native_region_scanner::create_skipper(
        crate::frame_c::visitors::TargetLanguage::Rust,
    );
    let bytes = src.as_bytes();
    let end = bytes.len();
    let mut args = Vec::new();
    let mut i = 0;
    let mut start = 0;
    let mut depth = 0i32;
    while i < end {
        if let Some(next) = skipper.skip_string(bytes, i, end) {
            i = next;
            continue;
        }
        if let Some(next) = skipper.skip_comment(bytes, i, end) {
            i = next;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                args.push(src[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < src.len() {
        args.push(src[start..].to_string());
    } else if !src.is_empty() {
        // Empty arg after a trailing comma — preserve as empty.
        args.push(String::new());
    }
    args
}

/// If `arg` trimmed matches `self.<field>` exactly and `<field>` is in the
/// non-Copy list, rewrite to `self.<field>.clone()`. Otherwise return as-is.
fn rewrite_arg_if_non_copy_field(arg: &str, non_copy_fields: &[String]) -> String {
    let trimmed = arg.trim();
    // Preserve original whitespace around the arg so formatting stays intact.
    let prefix_end = arg.len() - arg.trim_start().len();
    let suffix_start = arg.trim_end().len() + prefix_end;
    let prefix = &arg[..prefix_end];
    let suffix = &arg[suffix_start..];
    if let Some(rest) = trimmed.strip_prefix("self.") {
        // Ensure rest is a bare ident (no further access like `.to_string()`
        // or `.x.y` — those cases the user has already handled).
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            if non_copy_fields.iter().any(|f| f == rest) {
                return format!("{}self.{}.clone(){}", prefix, rest, suffix);
            }
        }
    }
    arg.to_string()
}
