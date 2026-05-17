//! Rust persistence emission — `save_state` / `restore_state`
//! methods using serde_json.
//!
//! Serializes the entire compartment chain (HSM parent links),
//! the StateContext enum, the state stack, and the domain
//! variables — type-ignorantly. framec emits the user's
//! declared field types verbatim and serde does the
//! per-type marshalling at compile time. RFC-0012 amendment
//! supports both the new persist contract
//! (`@@[save(name)]` / `@@[load(name)]` renames) and the
//! legacy framework-method shape.

use super::super::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::frame_ast::{SystemAst, Type};

// ─── Persistence ─────────────────────────────────────────────────────

/// Generate Rust `save_state` and `restore_state` methods using
/// serde_json. Serializes the entire compartment chain (HSM parent
/// links), StateContext enums, state stack, and domain variables.
pub(crate) fn generate_rust_persistence_methods(system: &SystemAst) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // RFC-0012 amendment: branch on new contract.
    let uses_new_contract = system.uses_new_persist_contract();
    let save_method_name = system
        .save_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "save_state".to_string());
    let load_method_name = system
        .load_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "restore_state".to_string());
    let load_param_name = system
        .load_op_param_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "json".to_string());

    // ── save_state ───────────────────────────────────────────────
    let mut save_body = String::new();

    // Quiescent contract (E700): saving while a handler is on the
    // call stack would persist partial state. See RFC-0012. Rust
    // panics on contract violation rather than changing the public
    // signature to Result, which would force every caller to add
    // .unwrap() / ?.
    save_body.push_str(
        "if !self._context_stack.is_empty() { panic!(\"E700: system not quiescent\"); }\n",
    );

    save_body.push_str(&format!(
        "fn serialize_state_context(ctx: &{}StateContext) -> serde_json::Value {{\n",
        system.name
    ));
    save_body.push_str("    match ctx {\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            // Tuple-variant states have either state-vars (`$.x = 0`)
            // OR state-args (`$S(x: int)`) — both become fields on
            // the Context struct. The pre-fix code iterated only
            // state_vars, so a state declared `$S1(x: int)` (with
            // no state_vars) was matched as a unit variant and
            // emitted empty `json!({})`, dropping the state-arg
            // value. Iterate BOTH so all Context fields land in
            // the saved JSON. Same root pattern as the JSON-backend
            // persist sweep (framepiler 8fd22d2).
            let has_ctx_fields = !state.state_vars.is_empty() || !state.params.is_empty();
            if !has_ctx_fields {
                save_body.push_str(&format!(
                    "        {}StateContext::{} => serde_json::json!({{}}),\n",
                    system.name, state.name
                ));
            } else {
                save_body.push_str(&format!(
                    "        {}StateContext::{}(ctx) => serde_json::json!({{\n",
                    system.name, state.name
                ));
                for param in &state.params {
                    save_body.push_str(&format!(
                        "            \"{}\": ctx.{},\n",
                        param.name, param.name
                    ));
                }
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

    // Nested @@SystemName() instances round-trip via child save_state.
    for var in &system.domain {
        // RFC-0016.1: `@@[no_persist]` fields are transient — skip.
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if super::super::interface_gen::extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "    \"{0}\": serde_json::from_str::<serde_json::Value>(&self.{0}.save_state()).unwrap_or(serde_json::Value::Null),\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("    \"{}\": self.{},\n", var.name, var.name));
        }
    }

    save_body.push_str("}).to_string()");

    methods.push(CodegenNode::Method {
        name: save_method_name.clone(),
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
    // Coerce-via-borrow: works for `&str` (legacy) or `String`
    // (new contract user-declared) without per-type branching.
    let mut restore_body = String::new();
    restore_body.push_str(&format!("let __json_str: &str = &{};\n", load_param_name));
    restore_body
        .push_str("let data: serde_json::Value = serde_json::from_str(__json_str).expect(\"E950: persist load — input is not valid JSON\");\n");

    restore_body.push_str(&format!(
        "fn deserialize_state_context(state: &str, data: &serde_json::Value) -> {}StateContext {{\n",
        system.name
    ));
    restore_body.push_str("    match state {\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            // Mirror the save side: iterate state.params + state_vars
            // so the Context struct's tuple-variant payload is
            // reconstructed correctly. State-args declared via
            // `$S(x: int)` end up in state.params with the type
            // info needed for json extraction.
            let has_ctx_fields = !state.state_vars.is_empty() || !state.params.is_empty();
            if !has_ctx_fields {
                restore_body.push_str(&format!(
                    "        \"{}\" => {}StateContext::{},\n",
                    state.name, system.name, state.name
                ));
            } else {
                restore_body.push_str(&format!(
                    "        \"{}\" => {}StateContext::{}({}Context {{\n",
                    state.name, system.name, state.name, state.name
                ));
                for param in &state.params {
                    let json_extract = rust_json_extract(&param.name, &param.param_type);
                    restore_body
                        .push_str(&format!("            {}: {},\n", param.name, json_extract));
                }
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
    restore_body.push_str("    let state = data[\"state\"].as_str().expect(\"E951: persist load — compartment missing string \\\"state\\\" field\");\n");
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

    if uses_new_contract {
        // Instance method: mutate &mut self in place. Write each
        // field directly. No struct literal needed because `self`
        // is the existing instance.
        restore_body.push_str("self._state_stack = stack;\n");
        restore_body.push_str("self._context_stack = vec![];\n");
        restore_body.push_str("self.__compartment = compartment;\n");
        restore_body.push_str("self.__next_compartment = None;\n");
        for var in &system.domain {
            // RFC-0016.1: `@@[no_persist]` fields aren't in the blob.
            // The field already holds its `domain:` default because
            // restore_state runs on an instance constructed via `new()`,
            // which seeded every field from its declared initializer.
            if var.attributes.iter().any(|a| a.name == "no_persist") {
                continue;
            }
            let init = var.initializer_text.as_deref().unwrap_or("");
            if let Some(child_sys) = super::super::interface_gen::extract_tagged_system_name(init) {
                if super::super::interface_gen::nested_uses_new_contract(child_sys) {
                    // Nested system on new contract: instance-method
                    // load (no static factory available).
                    restore_body.push_str(&format!(
                        "self.{0} = {1}::new();\n\
                         self.{0}.restore_state(data[\"{0}\"].to_string());\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!(
                        "self.{0} = {1}::restore_state(&data[\"{0}\"].to_string());\n",
                        var.name, child_sys
                    ));
                }
            } else {
                let json_extract = rust_json_extract_unwrap(&var.name, &var.var_type);
                restore_body.push_str(&format!("self.{} = {};\n", var.name, json_extract));
            }
        }
    } else {
        // Legacy: build the struct literal and return it. Frame's
        // ctor-bypass via direct struct literal — equivalent of
        // GetUninitializedObject + manual field population in one
        // expression.
        restore_body.push_str(&format!("let instance = {} {{\n", system.name));
        restore_body.push_str("    _state_stack: stack,\n");
        restore_body.push_str("    _context_stack: vec![],\n");
        restore_body.push_str("    __compartment: compartment,\n");
        restore_body.push_str("    __next_compartment: None,\n");
        for var in &system.domain {
            let init = var.initializer_text.as_deref().unwrap_or("");
            // RFC-0016.1: `@@[no_persist]` fields aren't in the blob.
            // The legacy contract builds the instance via a struct
            // literal, so we still have to emit a value for the field —
            // use the declared `domain:` default. (Legacy is E814-rejected
            // in practice; this branch is dead but kept consistent.)
            if var.attributes.iter().any(|a| a.name == "no_persist") {
                restore_body.push_str(&format!(
                    "    {}: {},\n",
                    var.name,
                    if init.is_empty() {
                        "Default::default()"
                    } else {
                        init
                    }
                ));
                continue;
            }
            if let Some(child_sys) = super::super::interface_gen::extract_tagged_system_name(init) {
                if super::super::interface_gen::nested_uses_new_contract(child_sys) {
                    // Nested on new contract — can't use static factory
                    // in struct-literal init. Allocate, populate, then
                    // close struct with this field as a temporary.
                    restore_body.push_str(&format!(
                        "    {0}: {{ let mut __c = {1}::new(); __c.restore_state(data[\"{0}\"].to_string()); __c }},\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!(
                        "    {0}: {1}::restore_state(&data[\"{0}\"].to_string()),\n",
                        var.name, child_sys
                    ));
                }
            } else {
                let json_extract = rust_json_extract_unwrap(&var.name, &var.var_type);
                restore_body.push_str(&format!("    {}: {},\n", var.name, json_extract));
            }
        }
        restore_body.push_str("};\n");
        restore_body.push_str("instance");
    }

    let user_load_type = system.load_op_param_type();
    let (load_return, load_static, load_param_type) = if uses_new_contract {
        // Instance method: takes &mut self implicitly via Method node
        // (is_static=false), no return. Honor the user-declared param
        // type (e.g. `String`); default to `String` when the user hasn't
        // declared one (RFC-0015 system-level `@@[save(name)]/@@[load(name)]`
        // has no operation declaration to read a type from). `String`
        // matches the symmetric save_state's return type, so user code
        // can pass the save() result back into load() without
        // adding `.as_str()` at every call site.
        let t = user_load_type
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| "String".to_string());
        (None, false, t)
    } else {
        // Static factory: takes only the json param, returns Self.
        (Some(system.name.clone()), true, "&str".to_string())
    };
    // Borrow-or-own dance: the body unconditionally calls
    // `serde_json::from_str(&data)`, which requires a `&str`. If the
    // user declared `String` (owned), `&data` already coerces to
    // `&str` via deref. If the declared type is something exotic,
    // codegen still emits `&data` and the user's type must implement
    // Deref<Target=str>.
    methods.push(CodegenNode::Method {
        name: load_method_name.clone(),
        params: vec![Param::new(&load_param_name).with_type(&load_param_type)],
        return_type: load_return,
        body: vec![CodegenNode::NativeBlock {
            code: restore_body,
            span: None,
        }],
        is_async: false,
        is_static: load_static,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    methods
}

// Type-ignorant: serde infers the target type from the surrounding
// struct field. Works for any T: serde::Deserialize — primitives,
// String, Vec<T>, HashMap<K, V>, user-defined #[derive(Deserialize)]
// types. framec just emits the field-name lookup; serde does the
// type-aware work via the user's declared field type.
fn rust_json_extract(var_name: &str, _var_type: &Type) -> String {
    format!(
        "serde_json::from_value(data[\"{}\"].clone()).unwrap_or_default()",
        var_name
    )
}

fn rust_json_extract_unwrap(var_name: &str, _var_type: &Type) -> String {
    format!(
        "serde_json::from_value(data[\"{}\"].clone()).unwrap_or_else(|e| panic!(\"E952: persist load — field {} failed to deserialize: {{}}\", e))",
        var_name, var_name
    )
}
