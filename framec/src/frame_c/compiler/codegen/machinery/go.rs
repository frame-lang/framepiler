//! Go machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_go_machinery` (4.2
//! plan §7.1.P3). Go's emission has two structural quirks vs. the
//! Java-family typed backends:
//!
//! - There is no `self`; the receiver is named `s` and bound on every
//!   method. Pointer types are `*FooFrameEvent` / `*FooCompartment`.
//! - `__route_to_state` uses Go's `switch state_name { case ... }`
//!   rather than the cascading if-chain (Go's switch on a string is
//!   ergonomic, the if-chain is idiomatic in Java/Kotlin/Swift).
//!
//! `__create` (the RFC-0017 factory) is emitted by the Constructor arm
//! in `backends/go.rs`, not here.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct GoMachinery;

impl MachineryGenerator for GoMachinery {
    fn lang_name(&self) -> &'static str {
        "go"
    }

    fn emit_hsm_chain(
        &self,
        system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        let mut chain_method = format!(
            "func (s *{}) hsm_chain() map[string][]string {{\n    return map[string][]string{{\n",
            system.name
        );
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("        \"{}\": {{{}}},\n", leaf, chain_str));
        }
        chain_method.push_str("    }\n}");
        Some(CodegenNode::NativeBlock {
            code: chain_method,
            span: None,
        })
    }

    fn emit_prepare_enter(
        &self,
        system: &SystemAst,
        _compartment_class: &str,
    ) -> Option<CodegenNode> {
        let comp_type = format!("*{}Compartment", system.name);
        Some(CodegenNode::Method {
            name: "__prepareEnter".to_string(),
            params: vec![
                Param::new("leaf").with_type("string"),
                Param::new("state_args").with_type("[]any"),
                Param::new("enter_args").with_type("[]any"),
            ],
            return_type: Some(comp_type.clone()),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"var comp {0} = nil
for _, name := range s.hsm_chain()[leaf] {{
    new_comp := new{1}Compartment(name)
    new_comp.stateArgs = append([]any{{}}, state_args...)
    new_comp.enterArgs = append([]any{{}}, enter_args...)
    new_comp.parentCompartment = comp
    comp = new_comp
}}
return comp"#,
                    comp_type, system.name
                ),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_prepare_exit(&self, _system: &SystemAst) -> Option<CodegenNode> {
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type("[]any")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"comp := s.__compartment
for comp != nil {
    comp.exitArgs = append([]any{}, exit_args...)
    comp = comp.parentCompartment
}
_ = comp"#
                    .to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_route_to_state(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_type = format!("*{}FrameEvent", system.name);
        let comp_type = format!("*{}Compartment", system.name);
        let states: Vec<&str> = system
            .machine
            .as_ref()
            .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default();
        let mut route_code = String::from("switch state_name {\n");
        for state in &states {
            route_code.push_str(&format!("case \"{}\":\n", state));
            route_code.push_str(&format!("    s._state_{}(__e, compartment)\n", state));
        }
        route_code.push_str("}");
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name").with_type("string"),
                Param::new("__e").with_type(&event_type),
                Param::new("compartment").with_type(&comp_type),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: route_code,
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_process_transition_loop(
        &self,
        system: &SystemAst,
        _event_class: &str,
    ) -> Option<CodegenNode> {
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"for s.__next_compartment != nil {{
    next_compartment := s.__next_compartment
    s.__next_compartment = nil
    exit_event := &{sys}FrameEvent{{_message: "<$", _parameters: s.__compartment.exitArgs}}
    s.__route_to_state(s.__compartment.state, exit_event, s.__compartment)
    s.__compartment = next_compartment
    if next_compartment.forwardEvent == nil {{
        enter_event := &{sys}FrameEvent{{_message: "$>", _parameters: s.__compartment.enterArgs}}
        s.__route_to_state(s.__compartment.state, enter_event, s.__compartment)
    }} else {{
        forward_event := next_compartment.forwardEvent
        next_compartment.forwardEvent = nil
        enter_event := &{sys}FrameEvent{{_message: "$>", _parameters: s.__compartment.enterArgs}}
        s.__route_to_state(s.__compartment.state, enter_event, s.__compartment)
        if forward_event._message != "$>" {{
            s.__router(forward_event)
        }}
    }}
    for i := range s._context_stack {{
        s._context_stack[i]._transitioned = true
    }}
}}"#,
                    sys = system.name
                ),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_type = format!("*{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_type)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "s.__router(__e)\ns.__process_transition_loop()".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_type = format!("*{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&event_type)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "s.__route_to_state(s.__compartment.state, __e, s.__compartment)".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_transition(
        &self,
        system: &SystemAst,
        _compartment_class: &str,
    ) -> Option<CodegenNode> {
        let comp_type = format!("*{}Compartment", system.name);
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next").with_type(&comp_type)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "s.__next_compartment = next".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
