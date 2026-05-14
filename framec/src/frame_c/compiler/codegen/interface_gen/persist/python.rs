//! Python persist codegen.
//!
//! Python uses field-by-field JSON (the same wire shape as
//! TS / JS / Ruby / Lua / PHP / Dart) — not whole-object pickle.
//! The blob is `bytes` (UTF-8 JSON). Wire format stays uniform
//! across backends, and `@@[no_persist]` is a per-field skip.
//!
//! RFC-0012 amendment: when `@@[save]` / `@@[load]` operations are
//! declared on the system, emit both as instance methods under the
//! user's chosen names. Otherwise emit the legacy `save_state` /
//! `restore_state` pair (factory-style `restore_state`).
//!
//! Nested `@@SystemName()` domain fields round-trip via the child's
//! own `save_state` / `restore_state` — preserves class identity
//! across the JSON boundary that would otherwise produce a plain
//! dict.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::frame_ast::SystemAst;

use super::super::{extract_tagged_system_name, nested_uses_new_contract};

pub(in crate::frame_c::compiler::codegen::interface_gen) fn generate(
    system: &SystemAst,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // RFC-0012 amendment: branch on new contract. Same pattern
    // as GDScript — when @@[save] / @@[load] declared, emit
    // both as instance methods under the user's chosen names.
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

    let comp_cls = format!("{}Compartment", system.name);

    // ---- save body ----
    let mut save_body = String::new();
    save_body.push_str(
        "if self._context_stack:\n    raise RuntimeError(\"E700: system not quiescent\")\n",
    );
    save_body.push_str("import json\n");
    save_body.push_str("def _ser_comp(c):\n");
    save_body.push_str("    if c is None:\n        return None\n");
    save_body.push_str("    return {\"state\": c.state, \"state_args\": list(c.state_args), \"state_vars\": dict(c.state_vars), \"enter_args\": list(c.enter_args), \"exit_args\": list(c.exit_args), \"parent_compartment\": _ser_comp(c.parent_compartment)}\n");
    save_body.push_str("state_data = {\"_compartment\": _ser_comp(self.__compartment), \"_state_stack\": [_ser_comp(c) for c in self._state_stack]}\n");
    for var in &system.domain {
        // RFC-0016.1: `@@[no_persist]` fields are transient — skip.
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            // Nested @@SystemName(): round-trip via the child's
            // save_state (which itself returns UTF-8 JSON bytes).
            save_body.push_str(&format!(
                "state_data[\"{0}\"] = json.loads(self.{0}.save_state()) if self.{0} is not None else None\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("state_data[\"{0}\"] = self.{0}\n", var.name));
        }
    }
    save_body.push_str("return json.dumps(state_data).encode(\"utf-8\")");
    methods.push(CodegenNode::Method {
        name: save_method_name.clone(),
        params: vec![],
        return_type: Some("bytes".to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: save_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    // ---- load body ----
    // `target` is `self` under the new contract (instance method
    // mutating self) or `instance` under the legacy one (static
    // factory returning a fresh, construction-bypassed object).
    let target = if uses_new_contract {
        "self"
    } else {
        "instance"
    };
    let mut restore_body = String::new();
    // Capture the blob before `import json` — the user's load
    // param could be named `json`, which the import would shadow.
    restore_body.push_str(&format!("_blob = {}\n", load_param_name));
    restore_body.push_str("import json\n");
    restore_body.push_str("_raw = _blob.decode(\"utf-8\") if isinstance(_blob, (bytes, bytearray)) else _blob\n");
    restore_body.push_str("_parsed = json.loads(_raw)\n");
    restore_body.push_str("def _deser_comp(d):\n");
    restore_body.push_str("    if d is None:\n        return None\n");
    restore_body.push_str(&format!("    comp = {}(d[\"state\"])\n", comp_cls));
    restore_body.push_str("    comp.state_args = list(d.get(\"state_args\", []))\n");
    restore_body.push_str("    comp.state_vars = dict(d.get(\"state_vars\", {}))\n");
    restore_body.push_str("    comp.enter_args = list(d.get(\"enter_args\", []))\n");
    restore_body.push_str("    comp.exit_args = list(d.get(\"exit_args\", []))\n");
    restore_body.push_str(
        "    comp.parent_compartment = _deser_comp(d.get(\"parent_compartment\"))\n",
    );
    restore_body.push_str("    return comp\n");
    if !uses_new_contract {
        restore_body.push_str(&format!(
            "instance = {}.__new__({})\n",
            system.name, system.name
        ));
    }
    restore_body.push_str(&format!(
        "{0}.__compartment = _deser_comp(_parsed[\"_compartment\"])\n{0}.__next_compartment = None\n{0}._state_stack = [_deser_comp(c) for c in _parsed.get(\"_state_stack\", [])]\n{0}._context_stack = []\n",
        target
    ));
    for var in &system.domain {
        // RFC-0016.1: `@@[no_persist]` fields aren't in the blob —
        // leave them at their `domain:` default (set on construction).
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if let Some(child_sys) = extract_tagged_system_name(init) {
            if nested_uses_new_contract(child_sys) {
                restore_body.push_str(&format!(
                    "if _parsed.get(\"{1}\") is not None:\n    {0}.{1} = {2}()\n    {0}.{1}.restore_state(json.dumps(_parsed[\"{1}\"]).encode(\"utf-8\"))\nelse:\n    {0}.{1} = None\n",
                    target, var.name, child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "{0}.{1} = {2}.restore_state(json.dumps(_parsed[\"{1}\"]).encode(\"utf-8\")) if _parsed.get(\"{1}\") is not None else None\n",
                    target, var.name, child_sys
                ));
            }
        } else {
            restore_body.push_str(&format!(
                "{0}.{1} = _parsed.get(\"{1}\")\n",
                target, var.name
            ));
        }
    }
    if !uses_new_contract {
        restore_body.push_str("return instance");
    }
    if uses_new_contract {
        methods.push(CodegenNode::Method {
            name: load_method_name.clone(),
            params: vec![Param::new(&load_param_name).with_type("bytes")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: restore_body,
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        });
    } else {
        methods.push(CodegenNode::Method {
            name: "restore_state".to_string(),
            params: vec![Param::new("data").with_type("bytes")],
            return_type: Some(format!("'{}'", system.name)),
            body: vec![CodegenNode::NativeBlock {
                code: restore_body,
                span: None,
            }],
            is_async: false,
            is_static: true,
            visibility: Visibility::Public,
            decorators: vec![],
        });
    }

    methods
}
