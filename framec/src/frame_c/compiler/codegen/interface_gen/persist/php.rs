//! PHP persist codegen.
//!
//! `json_encode` / `json_decode` field-by-field wire format. Two
//! private helpers (`__serComp` instance method, `__deserComp`
//! static — static because the legacy path calls it via `self::`
//! from the static factory). Legacy contract uses
//! `ReflectionClass::newInstanceWithoutConstructor()` to bypass
//! `__construct` so the initial-state `$>()` doesn't re-fire on
//! restore; new contract mutates `$this` in place.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
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
        "$this"
    } else {
        "$instance"
    };

    let mut ser_body = String::new();
    ser_body.push_str("if ($comp === null) return null;\n");
    ser_body.push_str("$j = ['state' => $comp->state, 'state_vars' => $comp->state_vars];\n");
    ser_body.push_str("$j['state_args'] = $comp->state_args;\n");
    ser_body.push_str("$j['enter_args'] = $comp->enter_args;\n");
    ser_body.push_str("$j['parent'] = $this->__serComp($comp->parent_compartment);\n");
    ser_body.push_str("return $j;");

    methods.push(CodegenNode::Method {
        name: "__serComp".to_string(),
        params: vec![Param::new("comp")],
        return_type: None,
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
    deser_body.push_str("if ($data === null) return null;\n");
    deser_body.push_str(&format!(
        "$c = new {}($data['state']);\n",
        compartment_class
    ));
    deser_body.push_str("if (isset($data['state_vars'])) $c->state_vars = $data['state_vars'];\n");
    deser_body.push_str("if (isset($data['state_args'])) $c->state_args = $data['state_args'];\n");
    deser_body.push_str("if (isset($data['enter_args'])) $c->enter_args = $data['enter_args'];\n");
    deser_body.push_str("if (isset($data['parent'])) $c->parent_compartment = self::__deserComp($data['parent']);\n");
    deser_body.push_str("return $c;");

    methods.push(CodegenNode::Method {
        name: "__deserComp".to_string(),
        params: vec![Param::new("data")],
        return_type: None,
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
    save_body.push_str("if (!empty($this->_context_stack)) throw new \\Exception(\"E700: system not quiescent\");\n");
    save_body.push_str("$j = [];\n");
    save_body.push_str("$j['_compartment'] = $this->__serComp($this->__compartment);\n");
    save_body.push_str("$stack = [];\n");
    save_body
        .push_str("foreach ($this->_state_stack as $c) { $stack[] = $this->__serComp($c); }\n");
    save_body.push_str("$j['_state_stack'] = $stack;\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "$j['{0}'] = $this->{0} !== null ? json_decode($this->{0}->save_state(), true) : null;\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("$j['{}'] = $this->{};\n", var.name, var.name));
        }
    }
    save_body.push_str("return json_encode($j);");

    methods.push(CodegenNode::Method {
        name: save_method_name.clone(),
        params: vec![],
        return_type: None,
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
        "$_parsed = json_decode(${}, true);\n",
        load_param_name
    ));
    if !uses_new_contract {
        restore_body.push_str(&format!(
            "$instance = (new \\ReflectionClass({}::class))->newInstanceWithoutConstructor();\n",
            sys
        ));
    }
    let deser = if uses_new_contract {
        "$this->__deserComp"
    } else {
        "self::__deserComp"
    };
    restore_body.push_str(&format!("{}->_state_stack = [];\n", target));
    restore_body.push_str(&format!("{}->_context_stack = [];\n", target));
    restore_body.push_str(&format!(
        "{}->__compartment = {}($_parsed['_compartment']);\n",
        target, deser
    ));
    restore_body.push_str("if (isset($_parsed['_state_stack'])) {\n");
    restore_body.push_str(&format!(
        "    foreach ($_parsed['_state_stack'] as $sc) {{ {}->_state_stack[] = {}($sc); }}\n",
        target, deser
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
                    "if (isset($_parsed['{1}']) && $_parsed['{1}'] !== null) {{ {0}->{1} = new {2}(); {0}->{1}->restore_state(json_encode($_parsed['{1}'])); }}\n",
                    target, var.name, child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "if (isset($_parsed['{1}']) && $_parsed['{1}'] !== null) {0}->{1} = {2}::restore_state(json_encode($_parsed['{1}']));\n",
                    target, var.name, child_sys
                ));
            }
        } else {
            restore_body.push_str(&format!(
                "if (isset($_parsed['{1}'])) {0}->{1} = $_parsed['{1}'];\n",
                target, var.name
            ));
        }
    }
    if !uses_new_contract {
        restore_body.push_str("return $instance;");
    }
    methods.push(CodegenNode::Method {
        name: load_method_name.clone(),
        params: vec![Param::new(&load_param_name)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: restore_body,
            span: None,
        }],
        is_async: false,
        is_static: !uses_new_contract,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    methods
}
