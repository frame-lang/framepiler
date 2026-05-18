//! Go persist codegen.
//!
//! `encoding/json` with type-ignorant typed restore via Marshal→
//! Unmarshal round-trip. framec emits the declared type verbatim;
//! Go's reflection-based unmarshal handles primitive / slice / map /
//! nested / user struct without per-shape branching.
//!
//! Number normalization: JSON numbers unmarshal as `float64` by
//! default; we coerce whole-number `float64` → `int` so handler
//! type assertions (`slot := stateArgs[0].(int)`) work post-restore.
//!
//! Legacy contract is a package-level free factory function
//! returning `*Sys`; new contract is a receiver method on `s *Sys`.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::codegen_utils::go_map_type;
use crate::frame_c::compiler::frame_ast::SystemAst;

use super::super::{extract_tagged_system_name, nested_uses_new_contract};

pub(in crate::frame_c::compiler::codegen::interface_gen) fn generate(
    system: &SystemAst,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let compartment_type = format!("{}Compartment", system.name);

    let uses_new_contract = system.uses_new_persist_contract();
    let save_method_name = system
        .save_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "SaveState".to_string());
    let load_method_name = system
        .load_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("Restore{}", system.name));
    let load_param_name = system
        .load_op_param_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "jsonStr".to_string());
    let target = if uses_new_contract { "s" } else { "instance" };

    let mut save_body = String::new();
    save_body.push_str("if len(s._context_stack) > 0 { panic(\"E700: system not quiescent\") }\n");
    save_body.push_str(&format!(
        "var serializeComp func(c *{}) interface{{}}\n",
        compartment_type
    ));
    save_body.push_str(&format!(
        "serializeComp = func(c *{}) interface{{}} {{\n",
        compartment_type
    ));
    save_body.push_str("    if c == nil { return nil }\n");
    save_body.push_str("    return map[string]interface{}{\n");
    save_body.push_str("        \"state\": c.state,\n");
    save_body.push_str("        \"state_args\": c.stateArgs,\n");
    save_body.push_str("        \"state_vars\": c.stateVars,\n");
    save_body.push_str("        \"enter_args\": c.enterArgs,\n");
    save_body.push_str("        \"exit_args\": c.exitArgs,\n");
    save_body.push_str("        \"forward_event\": c.forwardEvent,\n");
    save_body.push_str("        \"parent_compartment\": serializeComp(c.parentCompartment),\n");
    save_body.push_str("    }\n");
    save_body.push_str("}\n");
    save_body.push_str("data := map[string]interface{}{\n");
    save_body.push_str("    \"_compartment\": serializeComp(s.__compartment),\n");
    save_body.push_str("    \"_state_stack\": func() []interface{} {\n");
    save_body.push_str("        var arr []interface{}\n");
    save_body.push_str(
        "        for _, c := range s._state_stack { arr = append(arr, serializeComp(c)) }\n",
    );
    save_body.push_str("        return arr\n");
    save_body.push_str("    }(),\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "    \"{0}\": func() interface{{}} {{ var __raw interface{{}}; _ = json.Unmarshal([]byte(s.{0}.SaveState()), &__raw); return __raw }}(),\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("    \"{}\": s.{},\n", var.name, var.name));
        }
    }
    save_body.push_str("}\n");
    save_body.push_str("jsonBytes, _ := json.Marshal(data)\n");
    save_body.push_str("return string(jsonBytes)");

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

    let mut restore_body = String::new();
    restore_body.push_str("var _parsed map[string]interface{}\n");
    restore_body.push_str(&format!(
        "json.Unmarshal([]byte({}), &_parsed)\n",
        load_param_name
    ));
    restore_body.push_str(&format!(
        "var deserializeComp func(d interface{{}}) *{}\n",
        compartment_type
    ));
    restore_body.push_str(&format!(
        "deserializeComp = func(d interface{{}}) *{} {{\n",
        compartment_type
    ));
    restore_body.push_str("    if d == nil { return nil }\n");
    restore_body.push_str("    m := d.(map[string]interface{})\n");
    restore_body.push_str(&format!(
        "    comp := new{}Compartment(m[\"state\"].(string))\n",
        system.name
    ));
    restore_body.push_str("    normalizeNum := func(v interface{}) interface{} {\n");
    restore_body.push_str(
        "        if f, ok := v.(float64); ok && f == float64(int(f)) { return int(f) }\n",
    );
    restore_body.push_str("        return v\n");
    restore_body.push_str("    }\n");
    restore_body.push_str("    if sv, ok := m[\"state_vars\"].(map[string]interface{}); ok { for k, v := range sv { comp.stateVars[k] = normalizeNum(v) } }\n");
    restore_body.push_str("    if sa, ok := m[\"state_args\"].([]interface{}); ok { for _, v := range sa { comp.stateArgs = append(comp.stateArgs, normalizeNum(v)) } }\n");
    restore_body.push_str("    if ea, ok := m[\"enter_args\"].([]interface{}); ok { for _, v := range ea { comp.enterArgs = append(comp.enterArgs, normalizeNum(v)) } }\n");
    restore_body.push_str("    if xa, ok := m[\"exit_args\"].([]interface{}); ok { for _, v := range xa { comp.exitArgs = append(comp.exitArgs, normalizeNum(v)) } }\n");

    let go_typed_conv = |declared_type: &str, idx: usize, slot: &str| -> String {
        let t = declared_type.trim();
        if t.is_empty() {
            return String::new();
        }
        let src = format!("comp.{slot}[{idx}]");
        format!(
            "    if len(comp.{slot}) > {idx} {{\n\
             \x20       if __raw, __err := json.Marshal({src}); __err == nil {{\n\
             \x20           var __typed {t}\n\
             \x20           if json.Unmarshal(__raw, &__typed) == nil {{\n\
             \x20               comp.{slot}[{idx}] = __typed\n\
             \x20           }}\n\
             \x20       }}\n\
             \x20   }}\n"
        )
    };
    let go_state_arg_decls: Vec<(String, Vec<String>)> = system
        .machine
        .as_ref()
        .map(|m| {
            m.states
                .iter()
                .map(|s| {
                    let types: Vec<String> = s
                        .params
                        .iter()
                        .map(|p| match &p.param_type {
                            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
                            crate::frame_c::compiler::frame_ast::Type::Unknown => String::new(),
                        })
                        .collect();
                    (s.name.clone(), types)
                })
                .collect()
        })
        .unwrap_or_default();
    let go_enter_arg_decls: Vec<(String, Vec<String>)> = system
        .machine
        .as_ref()
        .map(|m| {
            m.states
                .iter()
                .map(|s| {
                    let types: Vec<String> = s
                        .enter
                        .as_ref()
                        .map(|e| {
                            e.params
                                .iter()
                                .map(|p| match &p.param_type {
                                    crate::frame_c::compiler::frame_ast::Type::Custom(s) => {
                                        s.clone()
                                    }
                                    crate::frame_c::compiler::frame_ast::Type::Unknown => {
                                        String::new()
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    (s.name.clone(), types)
                })
                .collect()
        })
        .unwrap_or_default();
    for (state_name, types) in &go_state_arg_decls {
        let mut branch = String::new();
        for (i, t) in types.iter().enumerate() {
            let conv = go_typed_conv(t, i, "stateArgs");
            if !conv.is_empty() {
                branch.push_str(&conv);
            }
        }
        if !branch.is_empty() {
            restore_body.push_str(&format!(
                "    if comp.state == \"{}\" {{\n{}    }}\n",
                state_name, branch
            ));
        }
    }
    for (state_name, types) in &go_enter_arg_decls {
        let mut branch = String::new();
        for (i, t) in types.iter().enumerate() {
            let conv = go_typed_conv(t, i, "enterArgs");
            if !conv.is_empty() {
                branch.push_str(&conv);
            }
        }
        if !branch.is_empty() {
            restore_body.push_str(&format!(
                "    if comp.state == \"{}\" {{\n{}    }}\n",
                state_name, branch
            ));
        }
    }
    restore_body.push_str("    // forward_event is typically nil in persisted state\n");
    restore_body
        .push_str("    comp.parentCompartment = deserializeComp(m[\"parent_compartment\"])\n");
    restore_body.push_str("    return comp\n");
    restore_body.push_str("}\n");
    if !uses_new_contract {
        restore_body.push_str(&format!("instance := &{}{{}}\n", system.name));
    }
    restore_body.push_str(&format!(
        "{}.__compartment = deserializeComp(_parsed[\"_compartment\"])\n",
        target
    ));
    restore_body.push_str(&format!("{}.__next_compartment = nil\n", target));
    restore_body.push_str("if stack, ok := _parsed[\"_state_stack\"].([]interface{}); ok {\n");
    restore_body.push_str(&format!(
        "    {}._state_stack = make([]*{}, 0, len(stack))\n",
        target, compartment_type
    ));
    restore_body.push_str(&format!(
        "    for _, c := range stack {{ {0}._state_stack = append({0}._state_stack, deserializeComp(c)) }}\n",
        target
    ));
    restore_body.push_str("}\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            if nested_uses_new_contract(child_sys) {
                restore_body.push_str(&format!(
                    "if __raw_{1}, err_{1} := json.Marshal(_parsed[\"{1}\"]); err_{1} == nil {{ {0}.{1} = New{2}(); {0}.{1}.LoadState(string(__raw_{1})) }}\n",
                    target, var.name, child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "if __raw_{1}, err_{1} := json.Marshal(_parsed[\"{1}\"]); err_{1} == nil {{ {0}.{1} = Restore{2}(string(__raw_{1})) }}\n",
                    target, var.name, child_sys
                ));
            }
        } else {
            let declared = match &var.var_type {
                crate::frame_c::compiler::frame_ast::Type::Custom(name) => go_map_type(name),
                _ => "interface{}".to_string(),
            };
            let go_extract = format!(
                "func() {t} {{ var __typed {t}; if __raw, err := json.Marshal(_parsed[\"{name}\"]); err == nil {{ json.Unmarshal(__raw, &__typed) }}; return __typed }}()",
                t = declared,
                name = var.name,
            );
            restore_body.push_str(&format!("{}.{} = {}\n", target, var.name, go_extract));
        }
    }
    if !uses_new_contract {
        restore_body.push_str("return instance");
    }

    let (load_return, load_static) = if uses_new_contract {
        (None, false)
    } else {
        (Some(format!("*{}", system.name)), true)
    };
    methods.push(CodegenNode::Method {
        name: load_method_name.clone(),
        params: vec![Param::new(&load_param_name).with_type("string")],
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
