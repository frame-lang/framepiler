//! C++ persist codegen.
//!
//! `nlohmann::json` wire format with `std::any`-tagged compartment
//! args. Type-ignorant typed restore: framec emits the declared
//! type `T` verbatim into `std::any_cast<T>` and `nlohmann::json::
//! get<T>()`; nlohmann's ADL handles primitives, `std::vector`,
//! `std::map`, `std::unordered_map`, `std::string`, and user types
//! with `to_json`/`from_json` overloads — no type-string parsing
//! in framec.
//!
//! Per-state typed branches for both state_args and enter_args (D8
//! / D13 fixes) so float and vector args round-trip through the
//! correct dispatcher rather than the fallback int/double scalar
//! path.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::codegen_utils::cpp_map_type;
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
        "(*this)"
    } else {
        "__instance"
    };

    let all_state_vars: Vec<(&str, &str, &str)> = system
        .machine
        .as_ref()
        .map(|m| {
            m.states
                .iter()
                .flat_map(|s| {
                    s.state_vars.iter().map(move |sv| {
                        let type_str = match &sv.var_type {
                            crate::frame_c::compiler::frame_ast::Type::Custom(t) => t.as_str(),
                            crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
                        };
                        (s.name.as_str(), sv.name.as_str(), type_str)
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let cpp_state_arg_decls: Vec<(String, Vec<String>)> = system
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
    let cpp_enter_arg_decls: Vec<(String, Vec<String>)> = system
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

    // save_state
    let mut save_body = String::new();
    save_body.push_str(
        "if (!_context_stack.empty()) throw std::runtime_error(\"E700: system not quiescent\");\n",
    );

    save_body.push_str(&format!(
        "std::function<nlohmann::json(const {0}*)> __ser = [&](const {0}* c) -> nlohmann::json {{\n",
        compartment_class
    ));
    save_body.push_str("    if (!c) return nullptr;\n");
    save_body.push_str("    nlohmann::json __cj;\n");
    save_body.push_str("    __cj[\"state\"] = c->state;\n");
    save_body.push_str("    nlohmann::json __sv;\n");
    save_body.push_str("    for (auto& [k, v] : c->state_vars) {\n");
    for (_state, var_name, var_type) in &all_state_vars {
        let cpp_type = cpp_map_type(var_type);
        save_body.push_str(&format!(
            "        if (k == \"{}\") {{ try {{ __sv[k] = std::any_cast<{}>(v); }} catch(...) {{}} }}\n",
            var_name, cpp_type
        ));
    }
    save_body.push_str("    }\n");
    save_body.push_str("    __cj[\"state_vars\"] = __sv;\n");
    save_body.push_str("    nlohmann::json __sa = nlohmann::json::array();\n");
    save_body.push_str("    {\n");
    for (state_name, types) in &cpp_state_arg_decls {
        if types.is_empty() {
            continue;
        }
        save_body.push_str(&format!("    if (c->state == \"{}\") {{\n", state_name));
        for (i, t) in types.iter().enumerate() {
            if t.is_empty() {
                save_body.push_str(&format!(
                    "        if (c->state_args.size() > {i}) {{ try {{ __sa.push_back(std::any_cast<int>(c->state_args[{i}])); }} catch(...) {{ try {{ __sa.push_back(std::any_cast<double>(c->state_args[{i}])); }} catch(...) {{ __sa.push_back(nullptr); }} }} }}\n"
                ));
            } else {
                save_body.push_str(&format!(
                    "        if (c->state_args.size() > {i}) {{ try {{ __sa.push_back(nlohmann::json(std::any_cast<{t}>(c->state_args[{i}]))); }} catch(...) {{ __sa.push_back(nullptr); }} }}\n"
                ));
            }
        }
        save_body.push_str("    } else \n");
    }
    save_body.push_str("    {\n");
    save_body.push_str("        for (const auto& v : c->state_args) { try { __sa.push_back(std::any_cast<int>(v)); } catch(...) { try { __sa.push_back(std::any_cast<double>(v)); } catch(...) { __sa.push_back(nullptr); } } }\n");
    save_body.push_str("    }\n");
    save_body.push_str("    }\n");
    save_body.push_str("    __cj[\"state_args\"] = __sa;\n");
    save_body.push_str("    nlohmann::json __ea = nlohmann::json::array();\n");
    save_body.push_str("    {\n");
    for (state_name, types) in &cpp_enter_arg_decls {
        if types.is_empty() {
            continue;
        }
        save_body.push_str(&format!("    if (c->state == \"{}\") {{\n", state_name));
        for (i, t) in types.iter().enumerate() {
            if t.is_empty() {
                save_body.push_str(&format!(
                    "        if (c->enter_args.size() > {i}) {{ try {{ __ea.push_back(std::any_cast<int>(c->enter_args[{i}])); }} catch(...) {{ try {{ __ea.push_back(std::any_cast<double>(c->enter_args[{i}])); }} catch(...) {{ __ea.push_back(nullptr); }} }} }}\n"
                ));
            } else {
                save_body.push_str(&format!(
                    "        if (c->enter_args.size() > {i}) {{ try {{ __ea.push_back(nlohmann::json(std::any_cast<{t}>(c->enter_args[{i}]))); }} catch(...) {{ __ea.push_back(nullptr); }} }}\n"
                ));
            }
        }
        save_body.push_str("    } else\n");
    }
    save_body.push_str("    {\n");
    save_body.push_str("        for (const auto& v : c->enter_args) { try { __ea.push_back(std::any_cast<int>(v)); } catch(...) { try { __ea.push_back(std::any_cast<double>(v)); } catch(...) { __ea.push_back(nullptr); } } }\n");
    save_body.push_str("    }\n");
    save_body.push_str("    }\n");
    save_body.push_str("    __cj[\"enter_args\"] = __ea;\n");
    save_body.push_str("    __cj[\"parent\"] = __ser(c->parent_compartment.get());\n");
    save_body.push_str("    return __cj;\n");
    save_body.push_str("};\n");

    save_body.push_str("nlohmann::json __j;\n");
    save_body.push_str("__j[\"_compartment\"] = __ser(__compartment.get());\n");

    save_body.push_str("nlohmann::json __stack = nlohmann::json::array();\n");
    save_body.push_str("for (auto& c : _state_stack) { __stack.push_back(__ser(c.get())); }\n");
    save_body.push_str("__j[\"_state_stack\"] = __stack;\n");

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "__j[\"{0}\"] = {0} ? nlohmann::json::parse({0}->save_state()) : nlohmann::json(nullptr);\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("__j[\"{}\"] = {};\n", var.name, var.name));
        }
    }

    save_body.push_str("return __j.dump();");

    methods.push(CodegenNode::Method {
        name: save_method_name.clone(),
        params: vec![],
        return_type: Some("std::string".to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: save_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    // restore_state
    let mut restore_body = String::new();
    restore_body.push_str(&format!(
        "std::function<std::unique_ptr<{0}>(const nlohmann::json&)> __deser = [&](const nlohmann::json& d) -> std::unique_ptr<{0}> {{\n",
        compartment_class
    ));
    restore_body.push_str("    if (d.is_null()) return nullptr;\n");
    restore_body.push_str(&format!(
        "    auto c = std::make_unique<{}>(std::string(d[\"state\"]));\n",
        compartment_class
    ));
    restore_body.push_str("    if (d.contains(\"state_vars\")) {\n");
    restore_body.push_str("        auto& sv = d[\"state_vars\"];\n");
    for (_state, var_name, var_type) in &all_state_vars {
        let cpp_type = cpp_map_type(var_type);
        restore_body.push_str(&format!(
            "        if (sv.contains(\"{0}\")) {{ c->state_vars[\"{0}\"] = std::any(sv[\"{0}\"].get<{1}>()); }}\n",
            var_name, cpp_type
        ));
    }
    restore_body.push_str("    }\n");
    restore_body
        .push_str("    if (d.contains(\"state_args\") && d[\"state_args\"].is_array()) {\n");
    restore_body.push_str("        const auto& __sa = d[\"state_args\"];\n");
    for (state_name, types) in &cpp_state_arg_decls {
        if types.is_empty() {
            continue;
        }
        restore_body.push_str(&format!("        if (c->state == \"{}\") {{\n", state_name));
        for (i, t) in types.iter().enumerate() {
            if t.is_empty() {
                restore_body.push_str(&format!(
                    "            if (__sa.size() > {i}) {{ if (__sa[{i}].is_number_integer()) c->state_args.push_back(std::any(__sa[{i}].get<int>())); else if (__sa[{i}].is_number_float()) c->state_args.push_back(std::any(__sa[{i}].get<double>())); }}\n"
                ));
            } else {
                restore_body.push_str(&format!(
                    "            if (__sa.size() > {i}) {{ try {{ c->state_args.push_back(std::any(__sa[{i}].get<{t}>())); }} catch(...) {{ }} }}\n"
                ));
            }
        }
        restore_body.push_str("        } else \n");
    }
    restore_body.push_str("        {\n");
    restore_body.push_str("            for (const auto& v : __sa) {\n");
    restore_body.push_str("                if (v.is_number_integer()) c->state_args.push_back(std::any(v.get<int>()));\n");
    restore_body.push_str("                else if (v.is_number_float()) c->state_args.push_back(std::any(v.get<double>()));\n");
    restore_body.push_str("            }\n");
    restore_body.push_str("        }\n");
    restore_body.push_str("    }\n");
    restore_body
        .push_str("    if (d.contains(\"enter_args\") && d[\"enter_args\"].is_array()) {\n");
    restore_body.push_str("        const auto& __ea = d[\"enter_args\"];\n");
    for (state_name, types) in &cpp_enter_arg_decls {
        if types.is_empty() {
            continue;
        }
        restore_body.push_str(&format!("        if (c->state == \"{}\") {{\n", state_name));
        for (i, t) in types.iter().enumerate() {
            if t.is_empty() {
                restore_body.push_str(&format!(
                    "            if (__ea.size() > {i}) {{ if (__ea[{i}].is_number_integer()) c->enter_args.push_back(std::any(__ea[{i}].get<int>())); else if (__ea[{i}].is_number_float()) c->enter_args.push_back(std::any(__ea[{i}].get<double>())); }}\n"
                ));
            } else {
                restore_body.push_str(&format!(
                    "            if (__ea.size() > {i}) {{ try {{ c->enter_args.push_back(std::any(__ea[{i}].get<{t}>())); }} catch(...) {{ }} }}\n"
                ));
            }
        }
        restore_body.push_str("        } else \n");
    }
    restore_body.push_str("        {\n");
    restore_body.push_str("            for (const auto& v : __ea) {\n");
    restore_body.push_str("                if (v.is_number_integer()) c->enter_args.push_back(std::any(v.get<int>()));\n");
    restore_body.push_str("                else if (v.is_number_float()) c->enter_args.push_back(std::any(v.get<double>()));\n");
    restore_body.push_str("            }\n");
    restore_body.push_str("        }\n");
    restore_body.push_str("    }\n");
    restore_body.push_str("    if (d.contains(\"parent\") && !d[\"parent\"].is_null()) {\n");
    restore_body.push_str("        c->parent_compartment = __deser(d[\"parent\"]);\n");
    restore_body.push_str("    }\n");
    restore_body.push_str("    return c;\n");
    restore_body.push_str("};\n");

    restore_body.push_str(&format!(
        "auto __j = nlohmann::json::parse({});\n",
        load_param_name
    ));
    let _ = uses_new_contract;
    restore_body.push_str(&format!(
        "{}.__compartment = __deser(__j[\"_compartment\"]);\n",
        target
    ));

    restore_body.push_str("if (__j.contains(\"_state_stack\")) {\n");
    restore_body.push_str("    for (auto& __sc : __j[\"_state_stack\"]) {\n");
    restore_body.push_str(&format!(
        "        {}._state_stack.push_back(__deser(__sc));\n",
        target
    ));
    restore_body.push_str("    }\n");
    restore_body.push_str("}\n");

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            if nested_uses_new_contract(child_sys) {
                restore_body.push_str(&format!(
                    "if (__j.contains(\"{0}\") && !__j[\"{0}\"].is_null()) {{ {tgt}.{0} = std::make_shared<{1}>(); {tgt}.{0}->restore_state(__j[\"{0}\"].dump()); }}\n",
                    var.name, child_sys, tgt = target
                ));
            } else {
                restore_body.push_str(&format!(
                    "if (__j.contains(\"{0}\") && !__j[\"{0}\"].is_null()) {{ {tgt}.{0} = std::make_shared<{1}>({1}::restore_state(__j[\"{0}\"].dump())); }}\n",
                    var.name, child_sys, tgt = target
                ));
            }
        } else {
            restore_body.push_str(&format!(
                "if (__j.contains(\"{0}\")) {{ __j[\"{0}\"].get_to({tgt}.{0}); }}\n",
                var.name,
                tgt = target
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
        params: vec![Param::new(&load_param_name).with_type("const std::string&")],
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
