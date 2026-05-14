//! Java persist codegen.
//!
//! Jackson `ObjectMapper` + `TypeReference` for per-state typed
//! restore. Primitives must be boxed for TypeReference (generics
//! can't take primitives — `TypeReference<int>` is illegal). The
//! `__deserComp` helper is static and takes the ObjectMapper as a
//! param so a single mapper reference flows through the whole
//! restore tree.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::codegen_utils::java_map_type;
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
    let target = if uses_new_contract {
        "this"
    } else {
        "__instance"
    };

    let java_box = |t: &str| -> String {
        match t {
            "int" => "Integer".to_string(),
            "double" => "Double".to_string(),
            "float" => "Float".to_string(),
            "boolean" => "Boolean".to_string(),
            "long" => "Long".to_string(),
            "char" => "Character".to_string(),
            "byte" => "Byte".to_string(),
            "short" => "Short".to_string(),
            other => other.to_string(),
        }
    };
    let state_param_types: Vec<(String, Vec<String>)> = system
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
                                java_box(&java_map_type(t))
                            }
                            _ => "Object".to_string(),
                        })
                        .collect();
                    (s.name.clone(), types)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut ser_body = String::new();
    ser_body.push_str("if (comp == null) return null;\n");
    ser_body.push_str("var j = new java.util.LinkedHashMap<String, Object>();\n");
    ser_body.push_str("j.put(\"state\", comp.state);\n");
    ser_body.push_str(
        "j.put(\"state_vars\", new java.util.LinkedHashMap<>(comp.state_vars));\n",
    );
    ser_body.push_str("j.put(\"state_args\", new java.util.ArrayList<>(comp.state_args));\n");
    ser_body.push_str("j.put(\"enter_args\", new java.util.ArrayList<>(comp.enter_args));\n");
    ser_body.push_str("j.put(\"parent\", __serComp(comp.parent_compartment));\n");
    ser_body.push_str("return j;");

    methods.push(CodegenNode::Method {
        name: "__serComp".to_string(),
        params: vec![Param::new("comp").with_type(&compartment_class)],
        return_type: Some("Object".to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: ser_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    let mut deser_body = String::new();
    deser_body.push_str("if (node == null || node.isNull()) return null;\n");
    deser_body.push_str(&format!(
        "var c = new {}(node.get(\"state\").asText());\n",
        compartment_class
    ));
    deser_body.push_str("if (node.has(\"state_vars\")) {\n");
    deser_body.push_str("    var fields = node.get(\"state_vars\").fields();\n");
    deser_body.push_str("    while (fields.hasNext()) {\n");
    deser_body.push_str("        var e = fields.next();\n");
    deser_body.push_str("        c.state_vars.put(e.getKey(), mapper.convertValue(e.getValue(), Object.class));\n");
    deser_body.push_str("    }\n");
    deser_body.push_str("}\n");
    deser_body.push_str(
        "var __sa = node.has(\"state_args\") ? node.get(\"state_args\") : null;\n",
    );
    deser_body.push_str(
        "var __ea = node.has(\"enter_args\") ? node.get(\"enter_args\") : null;\n",
    );
    if !state_param_types.is_empty() {
        deser_body.push_str("switch (c.state) {\n");
        for (state_name, param_types) in &state_param_types {
            deser_body.push_str(&format!("    case \"{}\":\n", state_name));
            for (i, ty) in param_types.iter().enumerate() {
                deser_body.push_str(&format!(
                    "        if (__sa != null && __sa.size() > {i}) c.state_args.add(mapper.convertValue(__sa.get({i}), new com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}}));\n"
                ));
                deser_body.push_str(&format!(
                    "        if (__ea != null && __ea.size() > {i}) c.enter_args.add(mapper.convertValue(__ea.get({i}), new com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}}));\n"
                ));
            }
            deser_body.push_str("        break;\n");
        }
        deser_body.push_str("    default:\n");
        deser_body.push_str(
            "        if (__sa != null) for (var n : __sa) c.state_args.add(mapper.convertValue(n, Object.class));\n",
        );
        deser_body.push_str(
            "        if (__ea != null) for (var n : __ea) c.enter_args.add(mapper.convertValue(n, Object.class));\n",
        );
        deser_body.push_str("        break;\n");
        deser_body.push_str("}\n");
    } else {
        deser_body.push_str(
            "if (__sa != null) for (var n : __sa) c.state_args.add(mapper.convertValue(n, Object.class));\n",
        );
        deser_body.push_str(
            "if (__ea != null) for (var n : __ea) c.enter_args.add(mapper.convertValue(n, Object.class));\n",
        );
    }
    deser_body.push_str(
        "if (node.has(\"parent\") && !node.get(\"parent\").isNull()) c.parent_compartment = __deserComp(node.get(\"parent\"), mapper);\n",
    );
    deser_body.push_str("return c;");

    methods.push(CodegenNode::Method {
        name: "__deserComp".to_string(),
        params: vec![
            Param::new("node").with_type("com.fasterxml.jackson.databind.JsonNode"),
            Param::new("mapper").with_type("com.fasterxml.jackson.databind.ObjectMapper"),
        ],
        return_type: Some(compartment_class.clone()),
        body: vec![CodegenNode::NativeBlock {
            code: deser_body,
            span: None,
        }],
        is_async: false,
        is_static: true,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    let mut save_body = String::new();
    save_body.push_str("if (!_context_stack.isEmpty()) throw new RuntimeException(\"E700: system not quiescent\");\n");
    save_body.push_str("var mapper = new com.fasterxml.jackson.databind.ObjectMapper();\n");
    save_body.push_str("var __j = new java.util.LinkedHashMap<String, Object>();\n");
    save_body.push_str("__j.put(\"_compartment\", __serComp(__compartment));\n");
    save_body.push_str("var __stack = new java.util.ArrayList<Object>();\n");
    save_body.push_str("for (var c : _state_stack) __stack.add(__serComp(c));\n");
    save_body.push_str("__j.put(\"_state_stack\", __stack);\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "try {{ __j.put(\"{0}\", {0} != null ? mapper.readTree({0}.save_state()) : null); }} catch (Exception e) {{ throw new RuntimeException(e); }}\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("__j.put(\"{}\", {});\n", var.name, var.name));
        }
    }
    save_body.push_str("try { return mapper.writeValueAsString(__j); } catch (Exception e) { throw new RuntimeException(e); }");

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

    let mut restore_body = String::new();
    restore_body.push_str("var mapper = new com.fasterxml.jackson.databind.ObjectMapper();\n");
    restore_body.push_str("com.fasterxml.jackson.databind.JsonNode __j;\n");
    restore_body.push_str(&format!(
        "try {{ __j = mapper.readTree({}); }} catch (Exception e) {{ throw new RuntimeException(e); }}\n",
        load_param_name
    ));
    let _ = (uses_new_contract, sys);
    restore_body.push_str(&format!(
        "{}.__compartment = __deserComp(__j.get(\"_compartment\"), mapper);\n",
        target
    ));
    restore_body.push_str("if (__j.has(\"_state_stack\")) {\n");
    restore_body.push_str(&format!(
        "    {}._state_stack = new java.util.ArrayList<>();\n",
        target
    ));
    restore_body.push_str(&format!(
        "    for (var __sc : __j.get(\"_state_stack\")) {}._state_stack.add(__deserComp(__sc, mapper));\n",
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
                    "if (__j.has(\"{name}\") && !__j.get(\"{name}\").isNull()) {{ {tgt}.{name} = new {child}(); {tgt}.{name}.restore_state(__j.get(\"{name}\").toString()); }}\n",
                    tgt = target,
                    name = var.name,
                    child = child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "if (__j.has(\"{name}\") && !__j.get(\"{name}\").isNull()) {tgt}.{name} = {child}.restore_state(__j.get(\"{name}\").toString());\n",
                    tgt = target,
                    name = var.name,
                    child = child_sys
                ));
            }
            continue;
        }
        let java_type: String = match &var.var_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                java_box(&java_map_type(t))
            }
            _ => "Object".to_string(),
        };
        restore_body.push_str(&format!(
            "if (__j.has(\"{name}\")) {tgt}.{name} = mapper.convertValue(__j.get(\"{name}\"), new com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}});\n",
            tgt = target,
            name = var.name,
            ty = java_type
        ));
    }
    if !uses_new_contract {
        restore_body.push_str("return __instance;");
    }

    let (load_return, load_static) = if uses_new_contract {
        (None, false)
    } else {
        (Some(sys.clone()), true)
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
