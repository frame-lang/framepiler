//! Kotlin persist codegen.
//!
//! Jackson `ObjectMapper` + per-state typed restore via
//! `TypeReference`. Kotlin's reified generics make TypeReference
//! work cleanly; primitives auto-box (`Int` → `java.lang.Integer`
//! etc.) in generic contexts. `__deserComp` is a static helper
//! taking the ObjectMapper as a param so the same reference flows
//! through the whole restore tree.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::codegen_utils::kotlin_map_type;
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
        "instance"
    };

    let kt_box = |t: &str| -> String {
        match t {
            "Int" => "Int".to_string(),
            "Long" => "Long".to_string(),
            "Double" => "Double".to_string(),
            "Float" => "Float".to_string(),
            "Boolean" => "Boolean".to_string(),
            "String" => "String".to_string(),
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
                                kt_box(&kotlin_map_type(t))
                            }
                            _ => "Any".to_string(),
                        })
                        .collect();
                    (s.name.clone(), types)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut ser_body = String::new();
    ser_body.push_str("if (comp == null) return null\n");
    ser_body.push_str("val j = java.util.LinkedHashMap<String, Any?>()\n");
    ser_body.push_str("j[\"state\"] = comp.state\n");
    ser_body.push_str("j[\"state_vars\"] = java.util.LinkedHashMap(comp.state_vars)\n");
    ser_body.push_str("j[\"state_args\"] = java.util.ArrayList(comp.state_args)\n");
    ser_body.push_str("j[\"enter_args\"] = java.util.ArrayList(comp.enter_args)\n");
    ser_body.push_str("j[\"parent\"] = __serComp(comp.parent_compartment)\n");
    ser_body.push_str("return j");

    methods.push(CodegenNode::Method {
        name: "__serComp".to_string(),
        params: vec![Param::new("comp").with_type(&format!("{}?", compartment_class))],
        return_type: Some("Any?".to_string()),
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
    deser_body.push_str("if (node == null || node.isNull) return null\n");
    deser_body.push_str(&format!(
        "val c = {}(node.get(\"state\").asText())\n",
        compartment_class
    ));
    deser_body.push_str("if (node.has(\"state_vars\")) {\n");
    deser_body.push_str("    val fields = node.get(\"state_vars\").fields()\n");
    deser_body.push_str("    while (fields.hasNext()) {\n");
    deser_body.push_str("        val e = fields.next()\n");
    deser_body.push_str(
        "        c.state_vars[e.key] = mapper.convertValue(e.value, Any::class.java)\n",
    );
    deser_body.push_str("    }\n");
    deser_body.push_str("}\n");
    deser_body.push_str(
        "val __sa: com.fasterxml.jackson.databind.JsonNode? = if (node.has(\"state_args\")) node.get(\"state_args\") else null\n",
    );
    deser_body.push_str(
        "val __ea: com.fasterxml.jackson.databind.JsonNode? = if (node.has(\"enter_args\")) node.get(\"enter_args\") else null\n",
    );
    if !state_param_types.is_empty() {
        deser_body.push_str("when (c.state) {\n");
        for (state_name, param_types) in &state_param_types {
            deser_body.push_str(&format!("    \"{}\" -> {{\n", state_name));
            for (i, ty) in param_types.iter().enumerate() {
                deser_body.push_str(&format!(
                    "        if (__sa != null && __sa.size() > {i}) c.state_args.add(mapper.convertValue(__sa.get({i}), object : com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}}))\n"
                ));
                deser_body.push_str(&format!(
                    "        if (__ea != null && __ea.size() > {i}) c.enter_args.add(mapper.convertValue(__ea.get({i}), object : com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}}))\n"
                ));
            }
            deser_body.push_str("    }\n");
        }
        deser_body.push_str("    else -> {\n");
        deser_body.push_str(
            "        if (__sa != null) for (n in __sa) c.state_args.add(mapper.convertValue(n, Any::class.java))\n",
        );
        deser_body.push_str(
            "        if (__ea != null) for (n in __ea) c.enter_args.add(mapper.convertValue(n, Any::class.java))\n",
        );
        deser_body.push_str("    }\n");
        deser_body.push_str("}\n");
    } else {
        deser_body.push_str(
            "if (__sa != null) for (n in __sa) c.state_args.add(mapper.convertValue(n, Any::class.java))\n",
        );
        deser_body.push_str(
            "if (__ea != null) for (n in __ea) c.enter_args.add(mapper.convertValue(n, Any::class.java))\n",
        );
    }
    deser_body.push_str(
        "if (node.has(\"parent\") && !node.get(\"parent\").isNull) c.parent_compartment = __deserComp(node.get(\"parent\"), mapper)\n",
    );
    deser_body.push_str("return c");

    methods.push(CodegenNode::Method {
        name: "__deserComp".to_string(),
        params: vec![
            Param::new("node").with_type("com.fasterxml.jackson.databind.JsonNode?"),
            Param::new("mapper").with_type("com.fasterxml.jackson.databind.ObjectMapper"),
        ],
        return_type: Some(format!("{}?", compartment_class)),
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
    save_body.push_str("if (_context_stack.isNotEmpty()) throw RuntimeException(\"E700: system not quiescent\")\n");
    save_body.push_str("val mapper = com.fasterxml.jackson.databind.ObjectMapper()\n");
    save_body.push_str("val j = java.util.LinkedHashMap<String, Any?>()\n");
    save_body.push_str("j[\"_compartment\"] = __serComp(__compartment)\n");
    save_body.push_str("val stack = java.util.ArrayList<Any?>()\n");
    save_body.push_str("for (c in _state_stack) stack.add(__serComp(c))\n");
    save_body.push_str("j[\"_state_stack\"] = stack\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "j[\"{0}\"] = if ({0} != null) mapper.readTree({0}.save_state()) else null\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("j[\"{}\"] = {}\n", var.name, var.name));
        }
    }
    save_body.push_str("return mapper.writeValueAsString(j)");

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
    restore_body.push_str("val mapper = com.fasterxml.jackson.databind.ObjectMapper()\n");
    restore_body.push_str(&format!(
        "val _parsed = mapper.readTree({})\n",
        load_param_name
    ));
    let _ = uses_new_contract;
    restore_body.push_str(&format!(
        "{}.__compartment = __deserComp(_parsed.get(\"_compartment\"), mapper)!!\n",
        target
    ));
    restore_body.push_str("if (_parsed.has(\"_state_stack\")) {\n");
    restore_body.push_str(&format!("    {}._state_stack = mutableListOf()\n", target));
    restore_body.push_str(&format!(
        "    for (sc in _parsed.get(\"_state_stack\")) {}._state_stack.add(__deserComp(sc, mapper)!!)\n",
        target
    ));
    restore_body.push_str("}\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            let body = if nested_uses_new_contract(child_sys) {
                format!(
                    "if (_parsed.has(\"{name}\") && !_parsed.get(\"{name}\").isNull) {{ {tgt}.{name} = {child}(); {tgt}.{name}.restore_state(_parsed.get(\"{name}\").toString()) }}\n",
                    tgt = target,
                    name = var.name,
                    child = child_sys
                )
            } else {
                format!(
                    "if (_parsed.has(\"{name}\") && !_parsed.get(\"{name}\").isNull) {tgt}.{name} = {child}.restore_state(_parsed.get(\"{name}\").toString())\n",
                    tgt = target,
                    name = var.name,
                    child = child_sys
                )
            };
            restore_body.push_str(&body);
            continue;
        }
        let mapped = match &var.var_type {
            crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                kt_box(&kotlin_map_type(t))
            }
            _ => "Any".to_string(),
        };
        restore_body.push_str(&format!(
            "if (_parsed.has(\"{name}\")) {tgt}.{name} = mapper.convertValue(_parsed.get(\"{name}\"), object : com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}})\n",
            tgt = target,
            name = var.name,
            ty = mapped
        ));
    }
    if !uses_new_contract {
        restore_body.push_str("return instance");
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
