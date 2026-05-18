//! C# persist codegen.
//!
//! `System.Text.Json` — type-ignorant typed restore via JSON
//! round-trip (`JsonSerializer.Serialize` then `Deserialize<T>`).
//! Per-state typed conversion (D10) reads the declared param type
//! verbatim, so framec doesn't have to parse generics or detect
//! container kinds — System.Text.Json reflection handles
//! primitives, `List<T>`, `Dictionary<K,V>`, nested structures, and
//! user types with `[JsonPropertyName]`.
//!
//! Legacy contract uses `RuntimeHelpers.GetUninitializedObject` to
//! bypass the constructor.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::codegen_utils::csharp_map_type;
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
        .unwrap_or_else(|| "SaveState".to_string());
    let load_method_name = system
        .load_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "RestoreState".to_string());
    let load_param_name = system
        .load_op_param_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "json".to_string());
    let target = if uses_new_contract {
        "this"
    } else {
        "__instance"
    };

    let mut ser_body = String::new();
    ser_body.push_str("if (comp == null) return null;\n");
    ser_body.push_str("var j = new Dictionary<string, object>();\n");
    ser_body.push_str("j[\"state\"] = comp.state;\n");
    ser_body.push_str("var sv = new Dictionary<string, object>(comp.state_vars);\n");
    ser_body.push_str("j[\"state_vars\"] = sv;\n");
    ser_body.push_str("j[\"state_args\"] = new List<object>(comp.state_args);\n");
    ser_body.push_str("j[\"enter_args\"] = new List<object>(comp.enter_args);\n");
    ser_body.push_str("j[\"parent\"] = __SerComp(comp.parent_compartment);\n");
    ser_body.push_str("return j;");

    methods.push(CodegenNode::Method {
        name: "__SerComp".to_string(),
        params: vec![Param::new("comp").with_type(&compartment_class)],
        return_type: Some("object".to_string()),
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
    deser_body.push_str("if (el.ValueKind == System.Text.Json.JsonValueKind.Null) return null;\n");
    deser_body.push_str(&format!(
        "var c = new {}(el.GetProperty(\"state\").GetString());\n",
        compartment_class
    ));
    deser_body.push_str("if (el.TryGetProperty(\"state_vars\", out var sv) && sv.ValueKind == System.Text.Json.JsonValueKind.Object) {\n");
    deser_body.push_str("    foreach (var kv in sv.EnumerateObject()) {\n");
    deser_body.push_str("        if (kv.Value.ValueKind == System.Text.Json.JsonValueKind.Number) { if (kv.Value.TryGetInt32(out int __ii)) c.state_vars[kv.Name] = __ii; else if (kv.Value.TryGetInt64(out long __il)) c.state_vars[kv.Name] = __il; else c.state_vars[kv.Name] = kv.Value.GetDouble(); }\n");
    deser_body.push_str("        else if (kv.Value.ValueKind == System.Text.Json.JsonValueKind.String) c.state_vars[kv.Name] = kv.Value.GetString();\n");
    deser_body.push_str("        else c.state_vars[kv.Name] = kv.Value.ToString();\n");
    deser_body.push_str("    }\n");
    deser_body.push_str("}\n");
    deser_body.push_str("if (el.TryGetProperty(\"state_args\", out var sa) && sa.ValueKind == System.Text.Json.JsonValueKind.Array) {\n");
    deser_body.push_str(
        "    foreach (var v in sa.EnumerateArray()) c.state_args.Add(__convertJsonValue(v));\n",
    );
    deser_body.push_str("}\n");
    deser_body.push_str("if (el.TryGetProperty(\"enter_args\", out var ea) && ea.ValueKind == System.Text.Json.JsonValueKind.Array) {\n");
    deser_body.push_str(
        "    foreach (var v in ea.EnumerateArray()) c.enter_args.Add(__convertJsonValue(v));\n",
    );
    deser_body.push_str("}\n");

    let cs_typed_conv = |declared_type: &str, idx: usize, slot: &str| -> String {
        let t = declared_type.trim();
        if t.is_empty() {
            return String::new();
        }
        format!(
            "    if (c.{slot}.Count > {idx} && c.{slot}[{idx}] != null) {{\n\
             \x20       try {{\n\
             \x20           var __raw = System.Text.Json.JsonSerializer.Serialize(c.{slot}[{idx}]);\n\
             \x20           c.{slot}[{idx}] = System.Text.Json.JsonSerializer.Deserialize<{t}>(__raw);\n\
             \x20       }} catch {{ /* leave generic value in place */ }}\n\
             \x20   }}\n"
        )
    };
    let state_arg_decls: Vec<(String, Vec<String>)> = system
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
    let enter_arg_decls: Vec<(String, Vec<String>)> = system
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
    let mut any_per_state = false;
    for (state_name, types) in &state_arg_decls {
        let mut branch = String::new();
        for (i, t) in types.iter().enumerate() {
            let conv = cs_typed_conv(t, i, "state_args");
            if !conv.is_empty() {
                branch.push_str(&conv);
            }
        }
        if !branch.is_empty() {
            if !any_per_state {
                deser_body.push_str("// D10 per-state typed list conversion\n");
                any_per_state = true;
            }
            deser_body.push_str(&format!(
                "if (c.state == \"{}\") {{\n{}}}\n",
                state_name, branch
            ));
        }
    }
    for (state_name, types) in &enter_arg_decls {
        let mut branch = String::new();
        for (i, t) in types.iter().enumerate() {
            let conv = cs_typed_conv(t, i, "enter_args");
            if !conv.is_empty() {
                branch.push_str(&conv);
            }
        }
        if !branch.is_empty() {
            if !any_per_state {
                deser_body.push_str("// D10 per-state typed list conversion\n");
                any_per_state = true;
            }
            deser_body.push_str(&format!(
                "if (c.state == \"{}\") {{\n{}}}\n",
                state_name, branch
            ));
        }
    }
    deser_body.push_str("if (el.TryGetProperty(\"parent\", out var p) && p.ValueKind != System.Text.Json.JsonValueKind.Null) {\n");
    deser_body.push_str("    c.parent_compartment = __DeserComp(p);\n");
    deser_body.push_str("}\n");
    deser_body.push_str("return c;");

    methods.push(CodegenNode::Method {
        name: "__DeserComp".to_string(),
        params: vec![Param::new("el").with_type("System.Text.Json.JsonElement")],
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

    let mut conv_body = String::new();
    conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.Number) {\n");
    conv_body.push_str("    if (v.TryGetInt32(out int __i)) return __i;\n");
    conv_body.push_str("    if (v.TryGetInt64(out long __l)) return __l;\n");
    conv_body.push_str("    return v.GetDouble();\n");
    conv_body.push_str("}\n");
    conv_body.push_str(
        "if (v.ValueKind == System.Text.Json.JsonValueKind.String) return v.GetString();\n",
    );
    conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.True) return true;\n");
    conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.False) return false;\n");
    conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.Array) {\n");
    conv_body.push_str("    var __list = new System.Collections.Generic.List<object>();\n");
    conv_body.push_str(
        "    foreach (var __ne in v.EnumerateArray()) __list.Add(__convertJsonValue(__ne));\n",
    );
    conv_body.push_str("    return __list;\n");
    conv_body.push_str("}\n");
    conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.Object) {\n");
    conv_body.push_str(
        "    var __dict = new System.Collections.Generic.Dictionary<string, object>();\n",
    );
    conv_body.push_str("    foreach (var __prop in v.EnumerateObject()) __dict[__prop.Name] = __convertJsonValue(__prop.Value);\n");
    conv_body.push_str("    return __dict;\n");
    conv_body.push_str("}\n");
    conv_body.push_str("return v.ToString();");
    methods.push(CodegenNode::Method {
        name: "__convertJsonValue".to_string(),
        params: vec![Param::new("v").with_type("System.Text.Json.JsonElement")],
        return_type: Some("object".to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: conv_body,
            span: None,
        }],
        is_async: false,
        is_static: true,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    let mut save_body = String::new();
    save_body.push_str("if (_context_stack.Count > 0) throw new System.Exception(\"E700: system not quiescent\");\n");
    save_body.push_str("var __j = new Dictionary<string, object>();\n");
    save_body.push_str("__j[\"_compartment\"] = __SerComp(__compartment);\n");
    save_body.push_str("var __stack = new List<object>();\n");
    save_body.push_str("foreach (var c in _state_stack) { __stack.Add(__SerComp(c)); }\n");
    save_body.push_str("__j[\"_state_stack\"] = __stack;\n");

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "__j[\"{0}\"] = {0} != null ? System.Text.Json.JsonDocument.Parse({0}.SaveState()).RootElement.Clone() : (object)null;\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("__j[\"{}\"] = {};\n", var.name, var.name));
        }
    }

    save_body.push_str("var __opts = new System.Text.Json.JsonSerializerOptions { TypeInfoResolver = new System.Text.Json.Serialization.Metadata.DefaultJsonTypeInfoResolver() };\n");
    save_body.push_str("return System.Text.Json.JsonSerializer.Serialize(__j, __opts);");

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
    restore_body.push_str(&format!(
        "var __doc = System.Text.Json.JsonDocument.Parse({});\n",
        load_param_name
    ));
    restore_body.push_str("var __root = __doc.RootElement;\n");
    if !uses_new_contract {
        restore_body.push_str(&format!(
            "var __instance = ({0})System.Runtime.CompilerServices.RuntimeHelpers.GetUninitializedObject(typeof({0}));\n",
            sys,
        ));
        restore_body.push_str(&format!(
            "__instance._state_stack = new List<{}>();\n",
            compartment_class,
        ));
        restore_body.push_str(&format!(
            "__instance._context_stack = new List<{}FrameContext>();\n",
            sys,
        ));
    }
    restore_body.push_str(&format!(
        "{}.__compartment = __DeserComp(__root.GetProperty(\"_compartment\"));\n",
        target
    ));
    restore_body.push_str("if (__root.TryGetProperty(\"_state_stack\", out var __stack)) {\n");
    restore_body.push_str(&format!(
        "    {}._state_stack = new List<{}>();\n",
        target, compartment_class
    ));
    restore_body.push_str(&format!(
        "    foreach (var item in __stack.EnumerateArray()) {{ {}._state_stack.Add(__DeserComp(item)); }}\n",
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
                    "if (__root.TryGetProperty(\"{name}\", out var __{name})) {{ if (__{name}.ValueKind != System.Text.Json.JsonValueKind.Null) {{ {tgt}.{name} = new {child}(); {tgt}.{name}.RestoreState(__{name}.GetRawText()); }} }}\n",
                    tgt = target,
                    name = var.name,
                    child = child_sys
                )
            } else {
                format!(
                    "if (__root.TryGetProperty(\"{name}\", out var __{name})) {{ if (__{name}.ValueKind != System.Text.Json.JsonValueKind.Null) {{ {tgt}.{name} = {child}.RestoreState(__{name}.GetRawText()); }} }}\n",
                    tgt = target,
                    name = var.name,
                    child = child_sys
                )
            };
            restore_body.push_str(&body);
        } else {
            let declared = match &var.var_type {
                crate::frame_c::compiler::frame_ast::Type::Custom(t) => csharp_map_type(t),
                _ => "object".to_string(),
            };
            restore_body.push_str(&format!(
                "if (__root.TryGetProperty(\"{name}\", out var __{name})) {{ try {{ {tgt}.{name} = System.Text.Json.JsonSerializer.Deserialize<{t}>(__{name}.GetRawText()); }} catch {{ }} }}\n",
                tgt = target,
                name = var.name,
                t = declared
            ));
        }
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
