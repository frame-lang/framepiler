//! Rust system code generation.
//!
//! Owns the Rust-specific codegen pipeline: machinery methods, state
//! dispatch, push/pop transitions, and (in future phases) fields,
//! constructor, interface dispatch, and Frame-statement expansion
//! delegates.
//!
//! `backends/rust.rs` handles the lower-level `CodegenNode → String`
//! rendering and is not modified by this module.

use super::ast::{CodegenNode, Param, Visibility};
use super::codegen_utils::type_to_string;
use crate::frame_c::compiler::arcanum::{Arcanum, HandlerEntry};
use crate::frame_c::compiler::frame_ast::{InterfaceMethod, StateVarAst, SystemAst, Type};

/// Generate the complete Rust system from a Frame AST.
///
/// Called from `system_codegen::generate_system` when target is Rust.
/// Returns a CodegenNode containing the full Rust implementation.
pub fn generate_rust_system(system: &SystemAst, arcanum: &Arcanum, source: &[u8]) -> CodegenNode {
    let lang = crate::frame_c::visitors::TargetLanguage::Rust;

    // Phase 1: delegates to shared path for fields, constructor, interface,
    // actions, operations. Machinery + dispatch already owned here.
    super::system_codegen::generate_system_shared(system, arcanum, lang, source)
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
