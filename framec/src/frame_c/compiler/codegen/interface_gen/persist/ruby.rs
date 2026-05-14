//! Ruby persist codegen.
//!
//! JSON via `JSON.generate` / `JSON.parse` (stdlib). Two private
//! helper methods (`__ser_comp`, `__deser_comp`) recursively
//! serialize the compartment chain.
//!
//! RFC-0012 amendment: under the new contract `restore_state` is an
//! instance method that writes directly to `@vars` on `self`. Under
//! legacy, it's a class-method static factory that uses `.allocate`
//! + `instance_variable_set` to populate without firing `initialize`
//! (which would re-fire the initial-state `$>()` enter).

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::frame_ast::SystemAst;

use super::super::{extract_tagged_system_name, nested_uses_new_contract};

pub(in crate::frame_c::compiler::codegen::interface_gen) fn generate(
    system: &SystemAst,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let sys = &system.name;
    let compartment_class = format!("{}Compartment", sys);

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

    // Private helper: serialize compartment chain
    let mut ser_body = String::new();
    ser_body.push_str("return nil if comp.nil?\n");
    ser_body.push_str("j = {}\n");
    ser_body.push_str("j[\"state\"] = comp.state\n");
    ser_body.push_str("sv = {}\n");
    ser_body.push_str("comp.state_vars.each { |k, v| sv[k] = v }\n");
    ser_body.push_str("j[\"state_vars\"] = sv\n");
    ser_body.push_str("j[\"state_args\"] = comp.state_args\n");
    ser_body.push_str("j[\"enter_args\"] = comp.enter_args\n");
    ser_body.push_str("j[\"parent\"] = __ser_comp(comp.parent_compartment)\n");
    ser_body.push_str("j");

    methods.push(CodegenNode::Method {
        name: "__ser_comp".to_string(),
        params: vec![Param::new("comp")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: ser_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // Private helper: deserialize compartment chain
    let mut deser_body = String::new();
    deser_body.push_str("return nil if data.nil?\n");
    deser_body.push_str(&format!("c = {}.new(data[\"state\"])\n", compartment_class));
    deser_body.push_str("if data[\"state_vars\"]\n");
    deser_body.push_str("  data[\"state_vars\"].each { |k, v| c.state_vars[k] = v }\n");
    deser_body.push_str("end\n");
    deser_body.push_str("c.state_args = data[\"state_args\"] if data[\"state_args\"]\n");
    deser_body.push_str("c.enter_args = data[\"enter_args\"] if data[\"enter_args\"]\n");
    deser_body.push_str("if data[\"parent\"]\n");
    deser_body.push_str("  c.parent_compartment = __deser_comp(data[\"parent\"])\n");
    deser_body.push_str("end\n");
    deser_body.push_str("c");

    methods.push(CodegenNode::Method {
        name: "__deser_comp".to_string(),
        params: vec![Param::new("data")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: deser_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // save_state()
    let mut save_body = String::new();
    save_body.push_str("raise \"E700: system not quiescent\" unless @_context_stack.empty?\n");
    save_body.push_str("j = {}\n");
    save_body.push_str("j[\"_compartment\"] = __ser_comp(@__compartment)\n");
    save_body.push_str("stack = []\n");
    save_body.push_str("@_state_stack.each { |c| stack.push(__ser_comp(c)) }\n");
    save_body.push_str("j[\"_state_stack\"] = stack\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "j[\"{0}\"] = @{0}.nil? ? nil : JSON.parse(@{0}.save_state)\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("j[\"{}\"] = @{}\n", var.name, var.name));
        }
    }
    save_body.push_str("JSON.generate(j)");

    methods.push(CodegenNode::Method {
        name: save_method_name.clone(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: save_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    let mut restore_body = String::new();
    restore_body.push_str(&format!("_parsed = JSON.parse({})\n", load_param_name));
    if uses_new_contract {
        restore_body.push_str("@_context_stack = []\n");
        restore_body.push_str("@__next_compartment = nil\n");
        restore_body.push_str("@__compartment = __deser_comp(_parsed[\"_compartment\"])\n");
        restore_body.push_str("if _parsed[\"_state_stack\"]\n");
        restore_body.push_str(
            "  @_state_stack = _parsed[\"_state_stack\"].map { |sc| __deser_comp(sc) }\n",
        );
        restore_body.push_str("else\n");
        restore_body.push_str("  @_state_stack = []\n");
        restore_body.push_str("end\n");
        for var in &system.domain {
            if var.attributes.iter().any(|a| a.name == "no_persist") {
                continue;
            }
            let init = var.initializer_text.as_deref().unwrap_or("");
            if let Some(child_sys) = extract_tagged_system_name(init) {
                if nested_uses_new_contract(child_sys) {
                    restore_body.push_str(&format!(
                        "if _parsed.key?(\"{0}\") && !_parsed[\"{0}\"].nil? then @{0} = {1}.new; @{0}.restore_state(JSON.generate(_parsed[\"{0}\"])) end\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!(
                        "if _parsed.key?(\"{0}\") then @{0} = _parsed[\"{0}\"].nil? ? nil : {1}.restore_state(JSON.generate(_parsed[\"{0}\"])) end\n",
                        var.name, child_sys
                    ));
                }
            } else {
                restore_body.push_str(&format!(
                    "@{} = _parsed[\"{}\"] if _parsed.key?(\"{}\")\n",
                    var.name, var.name, var.name
                ));
            }
        }
    } else {
        restore_body.push_str(&format!("instance = {}.allocate\n", sys));
        restore_body.push_str("instance.instance_variable_set(:@_context_stack, [])\n");
        restore_body
            .push_str("instance.instance_variable_set(:@__next_compartment, nil)\n");
        restore_body.push_str("instance.instance_variable_set(:@__compartment, instance.send(:__deser_comp, _parsed[\"_compartment\"]))\n");
        restore_body.push_str("if _parsed[\"_state_stack\"]\n");
        restore_body.push_str("  instance.instance_variable_set(:@_state_stack, _parsed[\"_state_stack\"].map { |sc| instance.send(:__deser_comp, sc) })\n");
        restore_body.push_str("else\n");
        restore_body.push_str("  instance.instance_variable_set(:@_state_stack, [])\n");
        restore_body.push_str("end\n");
        for var in &system.domain {
            if var.attributes.iter().any(|a| a.name == "no_persist") {
                continue;
            }
            let init = var.initializer_text.as_deref().unwrap_or("");
            if let Some(child_sys) = extract_tagged_system_name(init) {
                restore_body.push_str(&format!(
                    "if _parsed.key?(\"{0}\") then instance.{0} = _parsed[\"{0}\"].nil? ? nil : {1}.restore_state(JSON.generate(_parsed[\"{0}\"])) end\n",
                    var.name, child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "instance.{} = _parsed[\"{}\"] if _parsed.key?(\"{}\")\n",
                    var.name, var.name, var.name
                ));
            }
        }
        restore_body.push_str("instance");
    }

    methods.push(CodegenNode::Method {
        name: load_method_name.clone(),
        params: vec![Param::new(&load_param_name)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: restore_body,
            span: None,
        }],
        is_async: false,
        is_static: !uses_new_contract,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    methods
}
