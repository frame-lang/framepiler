//! JavaScript / TypeScript persist codegen.
//!
//! Same wire format as Python (field-by-field JSON). The two
//! languages share an emitter parameterised by `is_ts: bool` — the
//! only real differences are type annotations on the recursive
//! serialize/deserialize helpers (`any` / `<sys>Compartment | null`)
//! and the casts on the `_state_stack.map` callbacks.
//!
//! RFC-0012 amendment: under the new contract, `restoreState` is an
//! instance method that mutates `this`. Under the legacy contract,
//! it's a static factory that constructs via
//! `Object.create(Foo.prototype)` (bypassing the user constructor)
//! and returns the constructed instance. `_init` has already fired
//! the start-state enter once on the (legacy) ordinary constructor
//! path; the new-contract form skips that reset on restore — the
//! "$S0 enter on restore" trade-off per the RFC amendment.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::frame_ast::SystemAst;

use super::super::{extract_tagged_system_name, nested_uses_new_contract};

pub(in crate::frame_c::compiler::codegen::interface_gen) fn generate(
    system: &SystemAst,
    is_ts: bool,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // RFC-0012 amendment: branch on new contract. Save was
    // already an instance method; load was a static factory
    // using `Object.create(Foo.prototype)`. Under the new
    // contract, both become user-named instance methods that
    // mutate `this` directly — no construction bypass needed.
    let uses_new_contract = system.uses_new_persist_contract();
    let save_method_name = system
        .save_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "saveState".to_string());
    let load_method_name = system
        .load_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "restoreState".to_string());
    let load_param_name = system
        .load_op_param_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "json".to_string());
    let target = if uses_new_contract {
        "this"
    } else {
        "instance"
    };

    // Generate saveState method
    let mut save_body = String::new();
    save_body.push_str("if (this._context_stack.length > 0) { throw new Error(\"E700: system not quiescent\"); }\n");
    if is_ts {
        save_body.push_str("const serializeComp = (c: any): any => {\n");
    } else {
        save_body.push_str("const serializeComp = (c) => {\n");
    }
    save_body.push_str("    if (!c) return null;\n");
    save_body.push_str("    return {\n");
    save_body.push_str("        state: c.state,\n");
    save_body.push_str("        state_args: {...c.state_args},\n");
    save_body.push_str("        state_vars: {...c.state_vars},\n");
    save_body.push_str("        enter_args: {...c.enter_args},\n");
    save_body.push_str("        exit_args: {...c.exit_args},\n");
    save_body.push_str("        forward_event: c.forward_event,\n");
    save_body.push_str("        parent_compartment: serializeComp(c.parent_compartment),\n");
    save_body.push_str("    };\n");
    save_body.push_str("};\n");
    save_body.push_str("return JSON.stringify({\n");
    save_body.push_str("    _compartment: serializeComp(this.__compartment),\n");
    if is_ts {
        save_body
            .push_str("    _state_stack: this._state_stack.map((c: any) => serializeComp(c)),\n");
    } else {
        save_body.push_str("    _state_stack: this._state_stack.map((c) => serializeComp(c)),\n");
    }

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "    {0}: this.{0} ? JSON.parse(this.{0}.saveState()) : null,\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("    {}: this.{},\n", var.name, var.name));
        }
    }

    save_body.push_str("});\n");

    methods.push(CodegenNode::Method {
        name: save_method_name.clone(),
        params: vec![],
        return_type: Some("string".to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: save_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    // Generate restoreState method
    let mut restore_body = String::new();
    if is_ts {
        restore_body.push_str(&format!(
            "const deserializeComp = (data: any): {}Compartment | null => {{\n",
            system.name
        ));
    } else {
        restore_body.push_str("const deserializeComp = (data) => {\n");
    }
    restore_body.push_str("    if (!data) return null;\n");
    restore_body.push_str(&format!(
        "    const comp = new {}Compartment(data.state);\n",
        system.name
    ));
    restore_body.push_str("    comp.state_args = {...(data.state_args || {})};\n");
    restore_body.push_str("    comp.state_vars = {...(data.state_vars || {})};\n");
    restore_body.push_str("    comp.enter_args = {...(data.enter_args || {})};\n");
    restore_body.push_str("    comp.exit_args = {...(data.exit_args || {})};\n");
    restore_body.push_str("    comp.forward_event = data.forward_event;\n");
    restore_body
        .push_str("    comp.parent_compartment = deserializeComp(data.parent_compartment);\n");
    restore_body.push_str("    return comp;\n");
    restore_body.push_str("};\n");
    // Use `_parsed` for the parsed object — under the new
    // contract the user's load param name might collide with
    // a local called `data` (e.g., `unpickle(data: string)`).
    restore_body.push_str(&format!(
        "const _parsed = JSON.parse({});\n",
        load_param_name
    ));
    // Legacy only: construct via Object.create (skips constructor —
    // no initial-state enter side effects). The new contract
    // form mutates `this` in place.
    if !uses_new_contract {
        restore_body.push_str(&format!(
            "const instance = Object.create({}.prototype);\n",
            system.name
        ));
    }
    restore_body.push_str(&format!(
        "{}.__compartment = deserializeComp(_parsed._compartment);\n",
        target
    ));
    restore_body.push_str(&format!("{}.__next_compartment = null;\n", target));
    if is_ts {
        restore_body.push_str(&format!(
            "{}._state_stack = (_parsed._state_stack || []).map((c: any) => deserializeComp(c));\n",
            target
        ));
    } else {
        restore_body.push_str(&format!(
            "{}._state_stack = (_parsed._state_stack || []).map((c) => deserializeComp(c));\n",
            target
        ));
    }
    restore_body.push_str(&format!("{}._context_stack = [];\n", target));

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            if nested_uses_new_contract(child_sys) {
                restore_body.push_str(&format!(
                    "if (_parsed.{1} != null) {{ {0}.{1} = new {2}(); {0}.{1}.restoreState(JSON.stringify(_parsed.{1})); }} else {{ {0}.{1} = null; }}\n",
                    target, var.name, child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "{0}.{1} = _parsed.{1} != null ? {2}.restoreState(JSON.stringify(_parsed.{1})) : null;\n",
                    target, var.name, child_sys
                ));
            }
        } else {
            restore_body.push_str(&format!(
                "{}.{} = _parsed.{};\n",
                target, var.name, var.name
            ));
        }
    }

    if !uses_new_contract {
        restore_body.push_str("return instance;");
    }

    let (load_params, load_return, load_static) = if uses_new_contract {
        (
            vec![Param::new(&load_param_name).with_type("string")],
            None,
            false,
        )
    } else {
        (
            vec![Param::new(&load_param_name).with_type("string")],
            Some(system.name.clone()),
            true,
        )
    };
    methods.push(CodegenNode::Method {
        name: load_method_name.clone(),
        params: load_params,
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
