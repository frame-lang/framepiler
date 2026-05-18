//! Swift persist codegen.
//!
//! Foundation `JSONSerialization` — dict-based serialization with
//! two private `__serComp` / `__deserComp` helpers. Typed domain
//! field restore uses a single-element `[T]` array decoded via
//! `JSONDecoder` to thread Swift's type system across the
//! `Any`-shaped JSON boundary.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::codegen_utils::swift_map_type;
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
        .unwrap_or_else(|| "saveState".to_string());
    let load_method_name = system
        .load_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "restoreState".to_string());
    let load_param_name = system
        .load_op_param_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "json".to_string());
    let target = if uses_new_contract {
        "self"
    } else {
        "instance"
    };

    let mut ser_body = String::new();
    ser_body.push_str("if comp == nil { return nil }\n");
    ser_body.push_str("var j: [String: Any] = [:]\n");
    ser_body.push_str("j[\"state\"] = comp!.state\n");
    ser_body.push_str("var sv: [String: Any] = [:]\n");
    ser_body.push_str("for (k, v) in comp!.state_vars { sv[k] = v }\n");
    ser_body.push_str("j[\"state_vars\"] = sv\n");
    ser_body.push_str("j[\"state_args\"] = comp!.state_args\n");
    ser_body.push_str("j[\"enter_args\"] = comp!.enter_args\n");
    ser_body.push_str("j[\"parent\"] = __serComp(comp!.parent_compartment) as Any\n");
    ser_body.push_str("return j");

    methods.push(CodegenNode::Method {
        name: "__serComp".to_string(),
        params: vec![Param::new("comp").with_type(&format!("{}?", compartment_class))],
        return_type: Some("[String: Any]?".to_string()),
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
    deser_body.push_str("guard let d = dict else { return nil }\n");
    deser_body.push_str("guard let state = d[\"state\"] as? String else { return nil }\n");
    deser_body.push_str(&format!("let c = {}(state: state)\n", compartment_class));
    deser_body.push_str("if let sv = d[\"state_vars\"] as? [String: Any] {\n");
    deser_body.push_str("    for (k, v) in sv { c.state_vars[k] = v }\n");
    deser_body.push_str("}\n");
    deser_body.push_str("if let sa = d[\"state_args\"] as? [Any] {\n");
    deser_body.push_str("    c.state_args = sa\n");
    deser_body.push_str("}\n");
    deser_body.push_str("if let ea = d[\"enter_args\"] as? [Any] {\n");
    deser_body.push_str("    c.enter_args = ea\n");
    deser_body.push_str("}\n");
    deser_body.push_str("if let parent = d[\"parent\"] as? [String: Any] {\n");
    deser_body.push_str("    c.parent_compartment = __deserComp(parent)\n");
    deser_body.push_str("}\n");
    deser_body.push_str("return c");

    methods.push(CodegenNode::Method {
        name: "__deserComp".to_string(),
        params: vec![Param::new("dict").with_type("[String: Any]?")],
        return_type: Some(format!("{}?", compartment_class)),
        body: vec![CodegenNode::NativeBlock {
            code: deser_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    let mut save_body = String::new();
    save_body
        .push_str("if !_context_stack.isEmpty { fatalError(\"E700: system not quiescent\") }\n");
    save_body.push_str("var j: [String: Any] = [:]\n");
    save_body.push_str("j[\"_compartment\"] = __serComp(__compartment) as Any\n");
    save_body.push_str("var stack: [[String: Any]] = []\n");
    save_body.push_str("for c in _state_stack { if let s = __serComp(c) { stack.append(s) } }\n");
    save_body.push_str("j[\"_state_stack\"] = stack\n");
    for var in &system.domain {
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let init = var.initializer_text.as_deref().unwrap_or("");
        if extract_tagged_system_name(init).is_some() {
            save_body.push_str(&format!(
                "if let __raw_{0} = {0}.saveState().data(using: .utf8), let __nested_{0} = try? JSONSerialization.jsonObject(with: __raw_{0}) {{ j[\"{0}\"] = __nested_{0} }}\n",
                var.name
            ));
        } else {
            save_body.push_str(&format!("j[\"{}\"] = {}\n", var.name, var.name));
        }
    }
    save_body.push_str("let data = try! JSONSerialization.data(withJSONObject: j)\n");
    save_body.push_str("return String(data: data, encoding: .utf8)!");

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
    restore_body.push_str(&format!(
        "let data = {}.data(using: .utf8)!\n",
        load_param_name
    ));
    restore_body.push_str(
        "let _parsed = try! JSONSerialization.jsonObject(with: data) as! [String: Any]\n",
    );
    let _ = (uses_new_contract, sys);
    restore_body.push_str(&format!(
        "{}.__compartment = {}.__deserComp(_parsed[\"_compartment\"] as? [String: Any])!\n",
        target, target
    ));
    restore_body.push_str("if let stack = _parsed[\"_state_stack\"] as? [[String: Any]] {\n");
    restore_body.push_str(&format!("    {}._state_stack = []\n", target));
    restore_body.push_str(&format!(
        "    for sc in stack {{ if let c = {}.__deserComp(sc) {{ {}._state_stack.append(c) }} }}\n",
        target, target
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
                    "if let __raw_{1} = _parsed[\"{1}\"], let __data_{1} = try? JSONSerialization.data(withJSONObject: __raw_{1}), let __json_{1} = String(data: __data_{1}, encoding: .utf8) {{ {0}.{1} = {2}(); {0}.{1}.restoreState(__json_{1}) }}\n",
                    target, var.name, child_sys
                ));
            } else {
                restore_body.push_str(&format!(
                    "if let __raw_{1} = _parsed[\"{1}\"], let __data_{1} = try? JSONSerialization.data(withJSONObject: __raw_{1}), let __json_{1} = String(data: __data_{1}, encoding: .utf8) {{ {0}.{1} = {2}.restoreState(__json_{1}) }}\n",
                    target, var.name, child_sys
                ));
            }
        } else {
            let swift_type = match &var.var_type {
                crate::frame_c::compiler::frame_ast::Type::Custom(t) => swift_map_type(t),
                _ => "Any".to_string(),
            };
            if swift_type == "Any" {
                restore_body.push_str(&format!(
                    "if let v = _parsed[\"{1}\"] {{ {0}.{1} = v }}\n",
                    target, var.name
                ));
            } else {
                restore_body.push_str(&format!(
                    "if let __raw = _parsed[\"{name}\"], let __data = try? JSONSerialization.data(withJSONObject: [__raw]), let __arr = try? JSONDecoder().decode([{t}].self, from: __data), let __v = __arr.first {{ {tgt}.{name} = __v }}\n",
                    tgt = target,
                    name = var.name,
                    t = swift_type
                ));
            }
        }
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
