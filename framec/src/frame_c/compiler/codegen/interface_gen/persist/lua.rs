//! Lua persist codegen.
//!
//! Lua fidelity-exception: wire format is Lua's native textual
//! table-literal serialization via the `serpent` library
//! (https://github.com/pkulchenko/serpent), NOT JSON. Rationale:
//! lua-cjson decodes every JSON number as a Lua float
//! (`lua_Number`), erasing the Lua 5.3+ integer subtype
//! distinction. Most user code is unaffected (Lua's `==` is
//! numeric-equal across int/float) but code that uses
//! `math.type()` subtype queries or bitwise ops on persisted
//! integers silently breaks. Serpent dumps each value with the
//! syntax Lua's parser will read back as the same type — integers
//! stay integers, floats stay floats. Mirrors Erlang's ETF and
//! GDScript's `var_to_bytes` fidelity-exception rationale.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
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
        .unwrap_or_else(|| "save_state".to_string());
    let load_method_name = system
        .load_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "restore_state".to_string());
    let load_param_name = system
        .load_op_param_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "json_str".to_string());
    let target = if uses_new_contract {
        "self"
    } else {
        "instance"
    };

    let mut save_body = String::new();
    save_body.push_str(
        "if #self._context_stack > 0 then error(\"E700: system not quiescent\") end\n",
    );
    save_body.push_str("local serpent = require(\"serpent\")\n");
    save_body.push_str("local function serialize_comp(comp)\n");
    save_body.push_str("    if not comp then return nil end\n");
    save_body.push_str("    local t = {}\n");
    save_body.push_str("    t.state = comp.state\n");
    save_body.push_str("    t.state_args = comp.state_args\n");
    save_body.push_str("    t.state_vars = comp.state_vars\n");
    save_body.push_str("    t.enter_args = comp.enter_args\n");
    save_body.push_str("    t.exit_args = comp.exit_args\n");
    save_body.push_str("    t.forward_event = comp.forward_event\n");
    save_body
        .push_str("    t.parent_compartment = serialize_comp(comp.parent_compartment)\n");
    save_body.push_str("    return t\n");
    save_body.push_str("end\n");
    save_body.push_str("local stack = {}\n");
    save_body.push_str("for _, c in ipairs(self._state_stack) do\n");
    save_body.push_str("    stack[#stack + 1] = serialize_comp(c)\n");
    save_body.push_str("end\n");
    save_body.push_str("local result = {}\n");
    save_body.push_str("result._compartment = serialize_comp(self.__compartment)\n");
    save_body.push_str("result._state_stack = stack\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "result.{0} = (self.{0} ~= nil) and (select(2, serpent.load(self.{0}:save_state()))) or nil\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("result.{} = self.{}\n", var.name, var.name));
        }
    }
    save_body.push_str("return serpent.dump(result)");

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
    restore_body.push_str("local serpent = require(\"serpent\")\n");
    restore_body.push_str(&format!(
        "local ok, _parsed = serpent.load({})\n",
        load_param_name
    ));
    restore_body
        .push_str("if not ok then error(\"persist load failed: \" .. tostring(_parsed)) end\n");
    restore_body.push_str("local function deserialize_comp(d)\n");
    restore_body.push_str("    if not d then return nil end\n");
    restore_body.push_str(&format!(
        "    local comp = {}.new(d.state)\n",
        compartment_type
    ));
    restore_body.push_str("    comp.state_args = d.state_args or {}\n");
    restore_body.push_str("    comp.state_vars = d.state_vars or {}\n");
    restore_body.push_str("    comp.enter_args = d.enter_args or {}\n");
    restore_body.push_str("    comp.exit_args = d.exit_args or {}\n");
    restore_body.push_str("    comp.forward_event = d.forward_event\n");
    restore_body
        .push_str("    comp.parent_compartment = deserialize_comp(d.parent_compartment)\n");
    restore_body.push_str("    return comp\n");
    restore_body.push_str("end\n");
    if !uses_new_contract {
        restore_body.push_str("local instance = {}\n");
        restore_body.push_str(&format!(
            "setmetatable(instance, {{__index = {}}})\n",
            system.name
        ));
    }
    restore_body.push_str(&format!(
        "{}.__compartment = deserialize_comp(_parsed._compartment)\n",
        target
    ));
    restore_body.push_str(&format!("{}.__next_compartment = nil\n", target));
    restore_body.push_str(&format!("{}._state_stack = {{}}\n", target));
    restore_body.push_str(&format!("{}._context_stack = {{}}\n", target));
    restore_body.push_str("if _parsed._state_stack then\n");
    restore_body.push_str("    for _, c in ipairs(_parsed._state_stack) do\n");
    restore_body.push_str(&format!(
        "        {0}._state_stack[#{0}._state_stack + 1] = deserialize_comp(c)\n",
        target
    ));
    restore_body.push_str("    end\n");
    restore_body.push_str("end\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            if nested_uses_new_contract(child_sys) {
                restore_body.push_str(&format!(
                    "if _parsed.{1} ~= nil then {0}.{1} = {2}:new(); {0}.{1}:restore_state(serpent.dump(_parsed.{1})) else {0}.{1} = nil end\n",
                    target, var.name, child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "if _parsed.{1} ~= nil then {0}.{1} = {2}.restore_state(serpent.dump(_parsed.{1})) else {0}.{1} = nil end\n",
                    target, var.name, child_sys
                ));
            }
            continue;
        }
        restore_body
            .push_str(&format!("{}.{} = _parsed.{}\n", target, var.name, var.name));
    }
    if !uses_new_contract {
        restore_body.push_str("return instance");
    }

    let (load_return, load_static) = if uses_new_contract {
        (None, false)
    } else {
        (Some(system.name.clone()), true)
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
