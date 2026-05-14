//! GDScript persist codegen.
//!
//! GDScript fidelity-exception: wire format is Godot binary Variant
//! (`var_to_bytes` / `bytes_to_var`), NOT JSON. The brief
//! JSON-for-all migration was reverted after a real fidelity bug:
//! Godot's `JSON.parse_string` returns every JSON number as
//! `float`, erasing the `int` vs `float` distinction Variant
//! draws. A persisted `int`-typed domain field or list element
//! came back as `float`, and `Array.has(typed_int)` after restore
//! returned false even when the value was present (the list held
//! floats). `var_to_bytes` round-trips Variants exactly. Mirrors
//! Erlang's ETF and Lua's serpent fidelity-exception rationale.
//! See `docs/per_language_guides/gdscript.md`.
//!
//! Compartment chain serialization is iterative because GDScript
//! lambdas can't recurse: collect the chain into an array, then
//! build dicts bottom-up so each level can reference its parent's
//! already-constructed Dict.

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
        .unwrap_or_else(|| "data".to_string());

    // save_state — iterative compartment-chain serialization.
    let mut save_body = String::new();
    save_body.push_str("if not self._context_stack.is_empty():\n");
    save_body.push_str("    push_error(\"E700: system not quiescent\")\n");
    save_body.push_str("    return PackedByteArray()\n");
    save_body.push_str("# Serialize compartment chain iteratively\n");
    save_body.push_str("var _ser_chain = func(comp):\n");
    save_body.push_str("    var chain = []\n");
    save_body.push_str("    var cur = comp\n");
    save_body.push_str("    while cur != null:\n");
    save_body.push_str("        chain.append(cur)\n");
    save_body.push_str("        cur = cur.parent_compartment\n");
    save_body.push_str("    chain.reverse()\n");
    save_body.push_str("    var result = null\n");
    save_body.push_str("    for c in chain:\n");
    save_body.push_str("        var d = {}\n");
    save_body.push_str("        d[\"state\"] = c.state\n");
    save_body.push_str("        d[\"state_args\"] = c.state_args.duplicate()\n");
    save_body.push_str("        d[\"state_vars\"] = c.state_vars.duplicate()\n");
    save_body.push_str("        d[\"enter_args\"] = c.enter_args.duplicate()\n");
    save_body.push_str("        d[\"exit_args\"] = c.exit_args.duplicate()\n");
    save_body.push_str("        d[\"parent_compartment\"] = result\n");
    save_body.push_str("        result = d\n");
    save_body.push_str("    return result\n");
    save_body.push_str("var state_data = {}\n");
    save_body.push_str("state_data[\"_compartment\"] = _ser_chain.call(self.__compartment)\n");
    save_body.push_str("var stack_arr = []\n");
    save_body.push_str("for c in self._state_stack:\n");
    save_body.push_str("    stack_arr.append(_ser_chain.call(c))\n");
    save_body.push_str("state_data[\"_state_stack\"] = stack_arr\n");

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            // Nested child returns Godot-binary PackedByteArray
            // (var_to_bytes shape). Decode to Variant before embedding.
            save_body.push_str(&format!(
                "state_data[\"{0}\"] = bytes_to_var(self.{0}.save_state()) if self.{0} != null else null\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!(
                "state_data[\"{}\"] = self.{}\n",
                var.name, var.name
            ));
        }
    }
    save_body.push_str("return var_to_bytes(state_data)");

    methods.push(CodegenNode::Method {
        name: save_method_name.clone(),
        params: vec![],
        return_type: Some("PackedByteArray".to_string()),
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
    let target = if uses_new_contract {
        "self"
    } else {
        "instance"
    };
    let mut restore_body = String::new();
    restore_body.push_str(&format!(
        "var state_data = bytes_to_var({})\n",
        load_param_name
    ));
    restore_body.push_str("var _deser_chain = func(d):\n");
    restore_body.push_str("    if d == null:\n");
    restore_body.push_str("        return null\n");
    restore_body.push_str("    # Collect chain into array (child first)\n");
    restore_body.push_str("    var chain = []\n");
    restore_body.push_str("    var cur = d\n");
    restore_body.push_str("    while cur != null:\n");
    restore_body.push_str("        chain.append(cur)\n");
    restore_body.push_str("        cur = cur.get(\"parent_compartment\", null)\n");
    restore_body.push_str("    chain.reverse()\n");
    restore_body.push_str("    var result = null\n");
    restore_body.push_str("    for cd in chain:\n");
    restore_body.push_str(&format!(
        "        var comp = {}.new(cd[\"state\"])\n",
        compartment_type
    ));
    restore_body.push_str("        comp.state_args = cd.get(\"state_args\", {})\n");
    restore_body.push_str("        comp.state_vars = cd.get(\"state_vars\", {})\n");
    restore_body.push_str("        comp.enter_args = cd.get(\"enter_args\", {})\n");
    restore_body.push_str("        comp.exit_args = cd.get(\"exit_args\", {})\n");
    restore_body.push_str("        comp.parent_compartment = result\n");
    restore_body.push_str("        result = comp\n");
    restore_body.push_str("    return result\n");

    let _ = uses_new_contract;
    restore_body.push_str(&format!(
        "{}.__compartment = _deser_chain.call(state_data[\"_compartment\"])\n",
        target
    ));
    restore_body.push_str(&format!("{}.__next_compartment = null\n", target));
    restore_body.push_str(&format!("{}._state_stack = []\n", target));
    restore_body.push_str("for c in state_data.get(\"_state_stack\", []):\n");
    restore_body.push_str(&format!(
        "    {}._state_stack.append(_deser_chain.call(c))\n",
        target
    ));
    restore_body.push_str(&format!("{}._context_stack = []\n", target));

    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            restore_body.push_str(&format!(
                "var __raw_{0} = state_data.get(\"{0}\", null)\n",
                var.name
            ));
            if nested_uses_new_contract(child_sys) {
                restore_body.push_str(&format!(
                    "if __raw_{1} != null:\n    {0}.{1} = {2}.new()\n    {0}.{1}.restore_state(var_to_bytes(__raw_{1}))\nelse:\n    {0}.{1} = null\n",
                    target, var.name, child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "{0}.{1} = {2}.restore_state(var_to_bytes(__raw_{1})) if __raw_{1} != null else null\n",
                    target, var.name, child_sys
                ));
            }
        } else {
            restore_body.push_str(&format!(
                "{}.{} = state_data.get(\"{}\", null)\n",
                target, var.name, var.name
            ));
        }
    }

    if !uses_new_contract {
        restore_body.push_str("return instance");
    }

    let (load_params, load_return, load_static) = if uses_new_contract {
        (
            vec![Param::new(&load_param_name).with_type("PackedByteArray")],
            None,
            false,
        )
    } else {
        (
            vec![Param::new(&load_param_name).with_type("PackedByteArray")],
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
