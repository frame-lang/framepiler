//! C persist codegen.
//!
//! cJSON (`-lcjson`) for the wire format. The most divergent
//! backend — no implicit `self`, every "method" is
//! `Sys_method(Sys* self, args)`. Two flavours: legacy is a
//! factory `Sys_restore_state(json)` returning a fresh `Sys*`;
//! new contract takes `Sys* self` first and populates in place.
//!
//! Type dispatch is symbol-mangled: framec emits
//! `<sys>_persist_pack_<mangled>(value)` and
//! `<sys>_persist_unpack_<mangled>(json)` calls. The runtime
//! (`runtime.rs`) defines the symbols for blessed types (int,
//! str, bool, double, list, dict). framec stays type-ignorant
//! beyond a tiny alias-normalization table — no parsing of
//! generics, no library-API recognition, no per-element-type
//! branching here.
//!
//! Per-state typed restore via `if (strcmp(comp->state, "X")
//! == 0)` branches: framec emits, at codegen time, per-state
//! pack/unpack calls so state_args / enter_args round-trip
//! through the correct typed dispatcher.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::frame_ast::SystemAst;

use super::super::{extract_tagged_system_name, nested_uses_new_contract};

pub(in crate::frame_c::compiler::codegen::interface_gen) fn generate(
    system: &SystemAst,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

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
        "self"
    } else {
        "instance"
    };

    let type_to_string = |t: &crate::frame_c::compiler::frame_ast::Type| -> String {
        match t {
            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
            crate::frame_c::compiler::frame_ast::Type::Unknown => "int".to_string(),
        }
    };
    let state_arg_types: Vec<(String, Vec<String>)> = system
        .machine
        .as_ref()
        .map(|m| {
            m.states
                .iter()
                .map(|s| {
                    let types: Vec<String> = s
                        .params
                        .iter()
                        .map(|p| type_to_string(&p.param_type))
                        .collect();
                    (s.name.clone(), types)
                })
                .collect()
        })
        .unwrap_or_default();
    let state_enter_arg_types: Vec<(String, Vec<String>)> = system
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
                                .map(|p| type_to_string(&p.param_type))
                                .collect()
                        })
                        .unwrap_or_default();
                    (s.name.clone(), types)
                })
                .collect()
        })
        .unwrap_or_default();
    let c_mangle_type = |t: &str| -> String {
        let t = t.trim();
        let canonical = match t {
            "i32" | "i64" | "isize" | "uint" | "uintptr_t" | "intptr_t" | "long" | "short" => "int",
            "f32" | "f64" | "float" => "double",
            "boolean" => "bool",
            "string" | "String" | "str" | "char*" | "const char*" => "str",
            "List" | "Array" | "Array<any>" => "list",
            "Dict" | "Record<string, any>" => "dict",
            other => other,
        };
        canonical
            .chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => c,
                '*' => 'P',
                _ => '_',
            })
            .collect()
    };
    let c_pack_for = |frame_type: &str, val_expr: &str, sys_name: &str| -> String {
        format!(
            "{sys_name}_persist_pack_{m}({val_expr})",
            m = c_mangle_type(frame_type)
        )
    };
    let c_unpack_for = |frame_type: &str, json_expr: &str, sys_name: &str| -> String {
        format!(
            "{sys_name}_persist_unpack_{m}({json_expr})",
            m = c_mangle_type(frame_type)
        )
    };

    // serialize_compartment helper
    let mut serialize_helper = String::new();
    serialize_helper.push_str(&format!(
        "static cJSON* {}_serialize_compartment({}_Compartment* comp) {{\n",
        system.name, system.name
    ));
    serialize_helper.push_str("    if (!comp) return cJSON_CreateNull();\n");
    serialize_helper.push_str("    cJSON* obj = cJSON_CreateObject();\n");
    serialize_helper.push_str("    cJSON_AddStringToObject(obj, \"state\", comp->state);\n");
    serialize_helper.push_str("    cJSON* vars = cJSON_CreateObject();\n");
    serialize_helper.push_str(&format!(
        "    {}_FrameDict* sv = comp->state_vars;\n",
        system.name
    ));
    serialize_helper.push_str("    if (sv) {\n");
    serialize_helper.push_str("        for (int i = 0; i < sv->bucket_count; i++) {\n");
    serialize_helper.push_str(&format!(
        "            {}_FrameDictEntry* entry = sv->buckets[i];\n",
        system.name
    ));
    serialize_helper.push_str("            while (entry) {\n");
    serialize_helper.push_str("                cJSON_AddNumberToObject(vars, entry->key, (double)(intptr_t)entry->value);\n");
    serialize_helper.push_str("                entry = entry->next;\n");
    serialize_helper.push_str("            }\n");
    serialize_helper.push_str("        }\n");
    serialize_helper.push_str("    }\n");
    serialize_helper.push_str("    cJSON_AddItemToObject(obj, \"state_vars\", vars);\n");
    serialize_helper.push_str("    cJSON* sa = cJSON_CreateArray();\n");
    serialize_helper.push_str("    if (comp->state_args) {\n");
    for (state_name, types) in &state_arg_types {
        if types.is_empty() {
            continue;
        }
        serialize_helper.push_str(&format!(
            "        if (strcmp(comp->state, \"{}\") == 0) {{\n",
            state_name
        ));
        for (i, t) in types.iter().enumerate() {
            let val_expr = format!("comp->state_args->items[{}]", i);
            serialize_helper.push_str(&format!(
                "            if ({i} < comp->state_args->size) cJSON_AddItemToArray(sa, {pack});\n",
                i = i,
                pack = c_pack_for(t, &val_expr, &system.name)
            ));
        }
        serialize_helper.push_str("        }\n");
    }
    serialize_helper.push_str("    }\n");
    serialize_helper.push_str("    cJSON_AddItemToObject(obj, \"state_args\", sa);\n");
    serialize_helper.push_str("    cJSON* ea = cJSON_CreateArray();\n");
    serialize_helper.push_str("    if (comp->enter_args) {\n");
    for (state_name, types) in &state_enter_arg_types {
        if types.is_empty() {
            continue;
        }
        serialize_helper.push_str(&format!(
            "        if (strcmp(comp->state, \"{}\") == 0) {{\n",
            state_name
        ));
        for (i, t) in types.iter().enumerate() {
            let val_expr = format!("comp->enter_args->items[{}]", i);
            serialize_helper.push_str(&format!(
                "            if ({i} < comp->enter_args->size) cJSON_AddItemToArray(ea, {pack});\n",
                i = i,
                pack = c_pack_for(t, &val_expr, &system.name)
            ));
        }
        serialize_helper.push_str("        }\n");
    }
    serialize_helper.push_str("    }\n");
    serialize_helper.push_str("    cJSON_AddItemToObject(obj, \"enter_args\", ea);\n");
    serialize_helper.push_str(&format!("    cJSON_AddItemToObject(obj, \"parent_compartment\", {}_serialize_compartment(comp->parent_compartment));\n", system.name));
    serialize_helper.push_str("    return obj;\n");
    serialize_helper.push_str("}\n\n");

    // deserialize_compartment helper
    let mut deserialize_helper = String::new();
    deserialize_helper.push_str(&format!(
        "static {}_Compartment* {}_deserialize_compartment(cJSON* data) {{\n",
        system.name, system.name
    ));
    deserialize_helper.push_str("    if (!data || cJSON_IsNull(data)) return NULL;\n");
    deserialize_helper.push_str("    cJSON* state_item = cJSON_GetObjectItem(data, \"state\");\n");
    deserialize_helper.push_str(&format!(
        "    {}_Compartment* comp = {}_Compartment_new(strdup(state_item->valuestring));\n",
        system.name, system.name
    ));
    deserialize_helper.push_str("    cJSON* vars = cJSON_GetObjectItem(data, \"state_vars\");\n");
    deserialize_helper.push_str("    if (vars) {\n");
    deserialize_helper.push_str("        cJSON* var_item;\n");
    deserialize_helper.push_str("        cJSON_ArrayForEach(var_item, vars) {\n");
    deserialize_helper.push_str(&format!("            {}_FrameDict_set(comp->state_vars, var_item->string, (void*)(intptr_t)(int)var_item->valuedouble);\n", system.name));
    deserialize_helper.push_str("        }\n");
    deserialize_helper.push_str("    }\n");
    deserialize_helper.push_str("    cJSON* sa = cJSON_GetObjectItem(data, \"state_args\");\n");
    deserialize_helper.push_str("    if (sa) {\n");
    for (state_name, types) in &state_arg_types {
        if types.is_empty() {
            continue;
        }
        deserialize_helper.push_str(&format!(
            "        if (strcmp(comp->state, \"{}\") == 0) {{\n",
            state_name
        ));
        for (i, t) in types.iter().enumerate() {
            deserialize_helper.push_str(&format!(
                "            cJSON* sa_item_{i} = cJSON_GetArrayItem(sa, {i});\n"
            ));
            deserialize_helper.push_str(&format!(
                "            if (sa_item_{i}) {sys}_FrameVec_push(comp->state_args, {unpack});\n",
                i = i,
                sys = system.name,
                unpack = c_unpack_for(t, &format!("sa_item_{i}"), &system.name)
            ));
        }
        deserialize_helper.push_str("        }\n");
    }
    deserialize_helper.push_str("    }\n");
    deserialize_helper.push_str("    cJSON* ea = cJSON_GetObjectItem(data, \"enter_args\");\n");
    deserialize_helper.push_str("    if (ea) {\n");
    for (state_name, types) in &state_enter_arg_types {
        if types.is_empty() {
            continue;
        }
        deserialize_helper.push_str(&format!(
            "        if (strcmp(comp->state, \"{}\") == 0) {{\n",
            state_name
        ));
        for (i, t) in types.iter().enumerate() {
            deserialize_helper.push_str(&format!(
                "            cJSON* ea_item_{i} = cJSON_GetArrayItem(ea, {i});\n"
            ));
            deserialize_helper.push_str(&format!(
                "            if (ea_item_{i}) {sys}_FrameVec_push(comp->enter_args, {unpack});\n",
                i = i,
                sys = system.name,
                unpack = c_unpack_for(t, &format!("ea_item_{i}"), &system.name)
            ));
        }
        deserialize_helper.push_str("        }\n");
    }
    deserialize_helper.push_str("    }\n");
    deserialize_helper
        .push_str("    cJSON* parent = cJSON_GetObjectItem(data, \"parent_compartment\");\n");
    deserialize_helper.push_str(&format!(
        "    comp->parent_compartment = {}_deserialize_compartment(parent);\n",
        system.name
    ));
    deserialize_helper.push_str("    return comp;\n");
    deserialize_helper.push_str("}\n\n");

    methods.push(CodegenNode::NativeBlock {
        code: serialize_helper + &deserialize_helper,
        span: None,
    });

    // save_state
    let mut save_body = String::new();
    save_body.push_str(&format!(
        "if ({0}_FrameVec_size(self->_context_stack) > 0) {{ fprintf(stderr, \"E700: system not quiescent\\n\"); abort(); }}\n",
        system.name
    ));
    save_body.push_str("cJSON* root = cJSON_CreateObject();\n");
    save_body.push_str(&format!("cJSON_AddItemToObject(root, \"_compartment\", {}_serialize_compartment(self->__compartment));\n", system.name));

    save_body.push_str("cJSON* stack_arr = cJSON_CreateArray();\n");
    save_body.push_str(&format!(
        "for (int i = 0; i < {}_FrameVec_size(self->_state_stack); i++) {{\n",
        system.name
    ));
    save_body.push_str(&format!(
        "    {}_Compartment* comp = ({}_Compartment*){}_FrameVec_get(self->_state_stack, i);\n",
        system.name, system.name, system.name
    ));
    save_body.push_str(&format!(
        "    cJSON_AddItemToArray(stack_arr, {}_serialize_compartment(comp));\n",
        system.name
    ));
    save_body.push_str("}\n");
    save_body.push_str("cJSON_AddItemToObject(root, \"_state_stack\", stack_arr);\n");

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            save_body.push_str(&format!(
                "if (self->{name}) {{\n\
                 \x20   char* __child_json_{name} = {child}_save_state(self->{name});\n\
                 \x20   cJSON* __child_obj_{name} = cJSON_Parse(__child_json_{name});\n\
                 \x20   cJSON_AddItemToObject(root, \"{name}\", __child_obj_{name});\n\
                 \x20   free(__child_json_{name});\n\
                 }} else {{\n\
                 \x20   cJSON_AddNullToObject(root, \"{name}\");\n\
                 }}\n",
                name = var.name,
                child = child_sys
            ));
            continue;
        }
        let type_str = type_to_string(&var.var_type);
        save_body.push_str(&format!(
            "cJSON_AddItemToObject(root, \"{name}\", {sys}_persist_pack_field_{m}((void*)&self->{name}));\n",
            name = var.name,
            sys = system.name,
            m = c_mangle_type(&type_str)
        ));
    }

    save_body.push_str("char* json = cJSON_PrintUnformatted(root);\n");
    save_body.push_str("cJSON_Delete(root);\n");
    save_body.push_str("return json;");

    methods.push(CodegenNode::Method {
        name: save_method_name.clone(),
        params: vec![],
        return_type: Some("char*".to_string()),
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
        "cJSON* root = cJSON_Parse({});\n",
        load_param_name
    ));
    if uses_new_contract {
        restore_body.push_str("if (!root) return;\n\n");
    } else {
        restore_body.push_str("if (!root) return NULL;\n\n");
    }

    if !uses_new_contract {
        restore_body.push_str(&format!(
            "{}* instance = malloc(sizeof({}));\n",
            system.name, system.name
        ));
        restore_body.push_str(&format!(
            "instance->_state_stack = {}_FrameVec_new();\n",
            system.name
        ));
        restore_body.push_str(&format!(
            "instance->_context_stack = {}_FrameVec_new();\n",
            system.name
        ));
    }
    restore_body.push_str(&format!("{}->__next_compartment = NULL;\n\n", target));

    restore_body.push_str("cJSON* comp_data = cJSON_GetObjectItem(root, \"_compartment\");\n");
    restore_body.push_str(&format!(
        "{}->__compartment = {}_deserialize_compartment(comp_data);\n\n",
        target, system.name
    ));

    restore_body.push_str("cJSON* stack_arr = cJSON_GetObjectItem(root, \"_state_stack\");\n");
    restore_body.push_str("if (stack_arr) {\n");
    restore_body.push_str("    cJSON* stack_item;\n");
    restore_body.push_str("    cJSON_ArrayForEach(stack_item, stack_arr) {\n");
    restore_body.push_str(&format!(
        "        {}_Compartment* comp = {}_deserialize_compartment(stack_item);\n",
        system.name, system.name
    ));
    restore_body.push_str("        if (comp) {\n");
    restore_body.push_str(&format!(
        "            {}_FrameVec_push({}->_state_stack, comp);\n",
        system.name, target
    ));
    restore_body.push_str("        }\n");
    restore_body.push_str("    }\n");
    restore_body.push_str("}\n\n");

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            let body = if nested_uses_new_contract(child_sys) {
                format!(
                    "{{\n\
                     \x20   cJSON* __child_obj_{name} = cJSON_GetObjectItem(root, \"{name}\");\n\
                     \x20   if (__child_obj_{name} && !cJSON_IsNull(__child_obj_{name})) {{\n\
                     \x20       char* __child_json_{name} = cJSON_PrintUnformatted(__child_obj_{name});\n\
                     \x20       {tgt}->{name} = {child}_new();\n\
                     \x20       {child}_restore_state({tgt}->{name}, __child_json_{name});\n\
                     \x20       free(__child_json_{name});\n\
                     \x20   }} else {{\n\
                     \x20       {tgt}->{name} = NULL;\n\
                     \x20   }}\n\
                     }}\n",
                    tgt = target,
                    name = var.name,
                    child = child_sys
                )
            } else {
                format!(
                    "{{\n\
                     \x20   cJSON* __child_obj_{name} = cJSON_GetObjectItem(root, \"{name}\");\n\
                     \x20   if (__child_obj_{name} && !cJSON_IsNull(__child_obj_{name})) {{\n\
                     \x20       char* __child_json_{name} = cJSON_PrintUnformatted(__child_obj_{name});\n\
                     \x20       {tgt}->{name} = {child}_restore_state(__child_json_{name});\n\
                     \x20       free(__child_json_{name});\n\
                     \x20   }} else {{\n\
                     \x20       {tgt}->{name} = NULL;\n\
                     \x20   }}\n\
                     }}\n",
                    tgt = target,
                    name = var.name,
                    child = child_sys
                )
            };
            restore_body.push_str(&body);
            continue;
        }
        let type_str = type_to_string(&var.var_type);
        restore_body.push_str(&format!(
            "{sys}_persist_unpack_field_{m}(cJSON_GetObjectItem(root, \"{name}\"), (void*)&{tgt}->{name});\n",
            sys = system.name,
            m = c_mangle_type(&type_str),
            name = var.name,
            tgt = target
        ));
    }

    restore_body.push_str("\ncJSON_Delete(root);\n");
    if !uses_new_contract {
        restore_body.push_str("return instance;");
    }

    let (load_return, load_static) = if uses_new_contract {
        (None, false)
    } else {
        (Some(format!("{}*", system.name)), true)
    };
    methods.push(CodegenNode::Method {
        name: load_method_name.clone(),
        params: vec![Param::new(&load_param_name).with_type("const char*")],
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
