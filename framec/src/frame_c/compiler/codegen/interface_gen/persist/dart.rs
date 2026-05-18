//! Dart persist codegen.
//!
//! `jsonEncode` / `jsonDecode` via `dart:convert`, with per-state
//! typed restore via comprehensions (`<T>[for ... ]` /
//! `<K,V>{for ... }`) so the rehydrated `state_args` / `enter_args`
//! lists carry reified element types — required for handler bodies
//! that index without defensive casts. See `dart_types` module for
//! the rationale.
//!
//! Legacy contract uses a `Sys._restore()` named constructor that
//! bypasses the regular ctor's `$>` enter dispatch. New contract
//! mutates `this` in place — the existing instance's ctor already
//! ran, so the start-state enter has fired once (acceptable per the
//! RFC amendment's "$S0 enter on restore" trade-off).

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::frame_ast::SystemAst;

use super::super::{
    dart_conv_expr, extract_tagged_system_name, nested_uses_new_contract, parse_dart_type,
};

pub(in crate::frame_c::compiler::codegen::interface_gen) fn generate(
    system: &SystemAst,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let compartment_type = format!("{}Compartment", system.name);

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

    // save_state
    let mut save_body = String::new();
    save_body.push_str(
        "if (_context_stack.isNotEmpty) throw Exception(\"E700: system not quiescent\");\n",
    );
    save_body.push_str(&format!(
        "Map<String, dynamic>? serializeComp({}? comp) {{\n",
        compartment_type
    ));
    save_body.push_str("    if (comp == null) return null;\n");
    save_body.push_str("    return {\n");
    save_body.push_str("        'state': comp.state,\n");
    save_body.push_str("        'state_args': List<dynamic>.from(comp.state_args),\n");
    save_body.push_str("        'state_vars': Map<String, dynamic>.from(comp.state_vars),\n");
    save_body.push_str("        'enter_args': List<dynamic>.from(comp.enter_args),\n");
    save_body.push_str("        'exit_args': List<dynamic>.from(comp.exit_args),\n");
    save_body.push_str("        'forward_event': comp.forward_event,\n");
    save_body.push_str("        'parent_compartment': serializeComp(comp.parent_compartment),\n");
    save_body.push_str("    };\n");
    save_body.push_str("}\n");
    save_body.push_str("return jsonEncode({\n");
    save_body.push_str("    '_compartment': serializeComp(this.__compartment),\n");
    save_body
        .push_str("    '_state_stack': this._state_stack.map((c) => serializeComp(c)).toList(),\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "    '{0}': this.{0} != null ? jsonDecode(this.{0}.saveState()) : null,\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("    '{}': this.{},\n", var.name, var.name));
        }
    }
    save_body.push_str("});");

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

    // _restore named constructor (legacy only)
    if !uses_new_contract {
        methods.push(CodegenNode::NativeBlock {
            code: format!(
                "{system}._restore() : __compartment = {comp}(\"\"), __next_compartment = null {{\n\
                 \x20   _state_stack = [];\n\
                 \x20   _context_stack = [];\n\
                 }}",
                system = system.name,
                comp = compartment_type,
            ),
            span: None,
        });
    }

    // Per-state typed restore data.
    let dart_state_param_types: Vec<(String, Vec<String>)> = system
        .machine
        .as_ref()
        .map(|m| {
            m.states
                .iter()
                .filter(|s| !s.params.is_empty())
                .map(|s| {
                    let types: Vec<String> = s
                        .params
                        .iter()
                        .map(|p| match &p.param_type {
                            crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                                t.trim().to_string()
                            }
                            _ => "dynamic".to_string(),
                        })
                        .collect();
                    (s.name.clone(), types)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut restore_body = String::new();
    restore_body.push_str(&format!(
        "{}? deserializeComp(dynamic data) {{\n",
        compartment_type
    ));
    restore_body.push_str("    if (data == null || data is! Map) return null;\n");
    restore_body.push_str(&format!(
        "    final comp = {}(data['state'] as String);\n",
        compartment_type
    ));
    restore_body
        .push_str("    comp.state_vars = Map<String, dynamic>.from(data['state_vars'] ?? {});\n");
    restore_body.push_str("    final __saRaw = (data['state_args'] as List?) ?? <dynamic>[];\n");
    restore_body.push_str("    final __eaRaw = (data['enter_args'] as List?) ?? <dynamic>[];\n");
    restore_body
        .push_str("    comp.exit_args = List<dynamic>.from(data['exit_args'] ?? <dynamic>[]);\n");
    if !dart_state_param_types.is_empty() {
        restore_body.push_str("    switch (comp.state) {\n");
        for (state_name, param_types) in &dart_state_param_types {
            restore_body.push_str(&format!("        case '{}':\n", state_name));
            for (i, ty_str) in param_types.iter().enumerate() {
                let parsed = parse_dart_type(ty_str);
                let conv_sa = dart_conv_expr(&parsed, &format!("__saRaw[{i}]"));
                let conv_ea = dart_conv_expr(&parsed, &format!("__eaRaw[{i}]"));
                restore_body.push_str(&format!(
                    "            if (__saRaw.length > {i}) comp.state_args.add({conv_sa});\n"
                ));
                restore_body.push_str(&format!(
                    "            if (__eaRaw.length > {i}) comp.enter_args.add({conv_ea});\n"
                ));
            }
            restore_body.push_str("            break;\n");
        }
        restore_body.push_str("        default:\n");
        restore_body.push_str("            comp.state_args.addAll(__saRaw);\n");
        restore_body.push_str("            comp.enter_args.addAll(__eaRaw);\n");
        restore_body.push_str("            break;\n");
        restore_body.push_str("    }\n");
    } else {
        restore_body.push_str("    comp.state_args.addAll(__saRaw);\n");
        restore_body.push_str("    comp.enter_args.addAll(__eaRaw);\n");
    }
    restore_body.push_str("    comp.forward_event = data['forward_event'];\n");
    restore_body
        .push_str("    comp.parent_compartment = deserializeComp(data['parent_compartment']);\n");
    restore_body.push_str("    return comp;\n");
    restore_body.push_str("}\n");
    restore_body.push_str(&format!(
        "final _parsed = jsonDecode({}) as Map<String, dynamic>;\n",
        load_param_name
    ));
    if !uses_new_contract {
        restore_body.push_str(&format!("final instance = {}._restore();\n", system.name));
    }
    restore_body.push_str(&format!(
        "{}.__compartment = deserializeComp(_parsed['_compartment'])!;\n",
        target
    ));
    restore_body.push_str(&format!("{}.__next_compartment = null;\n", target));
    restore_body.push_str(&format!(
        "{}._state_stack = (_parsed['_state_stack'] as List?)?.map((c) => deserializeComp(c)!).toList() ?? <{}>[];\n",
        target, compartment_type
    ));
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            if nested_uses_new_contract(child_sys) {
                restore_body.push_str(&format!(
                    "{0}.{1} = {2}(); if (_parsed['{1}'] != null) {0}.{1}.restoreState(jsonEncode(_parsed['{1}']));\n",
                    target, var.name, child_sys
                ));
                continue;
            }
            restore_body.push_str(&format!(
                "{0}.{1} = _parsed['{1}'] != null ? {2}.restoreState(jsonEncode(_parsed['{1}'])) : {2}();\n",
                target, var.name, child_sys
            ));
        } else {
            let ty = match &var.var_type {
                crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.trim(),
                _ => "dynamic",
            };
            let parsed = parse_dart_type(ty);
            let conv = dart_conv_expr(&parsed, &format!("_parsed['{}']", var.name));
            restore_body.push_str(&format!("{}.{} = {};\n", target, var.name, conv));
        }
    }
    if !uses_new_contract {
        restore_body.push_str("return instance;");
    }

    let (load_return, load_static) = if uses_new_contract {
        (None, false)
    } else {
        (Some(system.name.clone()), true)
    };
    methods.push(CodegenNode::Method {
        name: load_method_name.clone(),
        params: vec![Param::new(&load_param_name).with_type("String")],
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
