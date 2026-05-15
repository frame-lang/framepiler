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

    fn emit_route_to_state(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: __router holds the dispatch table directly;
        // no separate __route_to_state helper.
        None
    }

    fn emit_process_transition_loop(
        &self,
        _system: &SystemAst,
        _event_class: &str,
    ) -> Option<CodegenNode> {
        // RFC-0020: drain loop is inlined into __kernel.
        None
    }

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: __kernel dispatches one event then drains any
        // transitions queued by the handler. Three-branch
        // forward-event protocol matches the Python reference.
        //
        // Go specifics:
        // - Receiver `s`, pointer fields throughout.
        // - `__next_compartment` is `*FooCompartment`; nil = no transition queued.
        // - `forwardEvent` is `*FooFrameEvent`; nil-check decides branch.
        // - Synthesized events are stack-allocated struct literals;
        //   GC handles cleanup of both synth and forward events.
        let event_type = format!("*{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_type)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"// Route event to current state.
s.__router(__e)
// Drain any transitions queued by the handler.
for s.__next_compartment != nil {{
    next_compartment := s.__next_compartment
    s.__next_compartment = nil
    // Exit the current (leaf) state.
    exit_event := &{sys}FrameEvent{{_message: "<$", _parameters: s.__compartment.exitArgs}}
    s.__router(exit_event)
    // Switch to the new compartment.
    s.__compartment = next_compartment
    // Three-branch forward-event handling.
    forward_event := next_compartment.forwardEvent
    next_compartment.forwardEvent = nil
    if forward_event == nil {{
        // No forwarded event — synthesize a fresh $>.
        enter_event := &{sys}FrameEvent{{_message: "$>", _parameters: s.__compartment.enterArgs}}
        s.__router(enter_event)
    }} else if forward_event._message == "$>" {{
        // Forwarded event IS $> — dispatch directly so the
        // destination's $> handler receives the caller's payload.
        s.__router(forward_event)
    }} else {{
        // Forwarded event is not $> — initialize the destination
        // with a fresh $>, then dispatch the forward.
        enter_event := &{sys}FrameEvent{{_message: "$>", _parameters: s.__compartment.enterArgs}}
        s.__router(enter_event)
        s.__router(forward_event)
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

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: __router is the single dispatch primitive. Reads
        // s.__compartment.state at call time and routes to the
        // matching state dispatcher inline (no __route_to_state
        // indirection).
        let event_type = format!("*{}FrameEvent", system.name);
        let states: Vec<&str> = system
            .machine
            .as_ref()
            .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default();
        let mut router_code = String::from("switch s.__compartment.state {\n");
        for state in &states {
            router_code.push_str(&format!("case \"{}\":\n", state));
            router_code.push_str(&format!("    s._state_{}(__e, s.__compartment)\n", state));
        }
        router_code.push_str("}");
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&event_type)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: router_code,
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
