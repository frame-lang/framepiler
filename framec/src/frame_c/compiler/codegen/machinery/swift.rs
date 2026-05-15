//! Swift machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_swift_machinery` (4.2
//! plan §7.1.P3). Swift quirks worth flagging:
//!
//! - `hsm_chain` and `__prepareEnter` are emitted as `private static
//!   func`s (Swift forbids instance-method calls on `self` until all
//!   stored properties are initialized, so the init path needs static
//!   versions). The kernel/router/transition methods remain instance
//!   methods.
//! - `__create` (the RFC-0017 factory) is emitted by the Constructor
//!   arm in `backends/swift.rs` alongside `init()` and `__frame_init`,
//!   not here.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct SwiftMachinery;

impl MachineryGenerator for SwiftMachinery {
    fn lang_name(&self) -> &'static str {
        "swift"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // hsm_chain — class method (so it's callable from init before all
        // stored properties are initialized; Swift forbids instance-method
        // calls on `self` until that point).
        let mut chain_method = String::from(
            "private static func hsm_chain() -> [String: [String]] {\n    return [\n",
        );
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("        \"{}\": [{}],\n", leaf, chain_str));
        }
        chain_method.push_str("    ]\n}");
        Some(CodegenNode::NativeBlock {
            code: chain_method,
            span: None,
        })
    }

    fn emit_prepare_enter(
        &self,
        system: &SystemAst,
        compartment_class: &str,
    ) -> Option<CodegenNode> {
        // __prepareEnter — class method (must be callable from init before
        // stored properties are initialized; doesn't touch instance state
        // anyway). Constructs the destination HSM chain.
        Some(CodegenNode::NativeBlock {
            code: format!(
                r#"private static func __prepareEnter(_ leaf: String, _ state_args: [Any], _ enter_args: [Any]) -> {0} {{
    var comp: {0}? = nil
    for name in {1}.hsm_chain()[leaf]! {{
        let new_comp = {0}(state: name)
        new_comp.state_args = state_args
        new_comp.enter_args = enter_args
        new_comp.parent_compartment = comp
        comp = new_comp
    }}
    return comp!
}}"#,
                compartment_class, system.name
            ),
            span: None,
        })
    }

    fn emit_prepare_exit(&self, system: &SystemAst) -> Option<CodegenNode> {
        let compartment_class = format!("{}Compartment", system.name);
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type("[Any]")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"var comp: {}? = __compartment
while comp != nil {{
    comp!.exit_args = exit_args
    comp = comp!.parent_compartment
}}"#,
                    compartment_class
                ),
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
        // transitions queued by the handler. Three-branch forward-
        // event protocol matches the Python reference.
        //
        // Swift specifics:
        // - `nil` for absent optional; `!= nil`/`== nil` checks.
        // - `FrameContext` is a class (reference) so `for ctx in
        //   _context_stack { ctx._transitioned = true }` mutates the
        //   shared instance — no indexed assignment needed.
        // - `String ==` is value equality.
        let event_class = format!("{}FrameEvent", system.name);
        let compartment_class = format!("{}Compartment", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"// Route event to current state.
__router(__e)
// Drain any transitions queued by the handler.
while __next_compartment != nil {{
    let next_compartment: {comp} = __next_compartment!
    __next_compartment = nil
    // Exit the current (leaf) state.
    let exit_event = {evt}(message: "<$", parameters: __compartment.exit_args)
    __router(exit_event)
    // Switch to the new compartment.
    __compartment = next_compartment
    // Three-branch forward-event handling.
    let forward_event: {evt}? = next_compartment.forward_event
    next_compartment.forward_event = nil
    if forward_event == nil {{
        // No forwarded event — synthesize a fresh $>.
        let enter_event = {evt}(message: "$>", parameters: __compartment.enter_args)
        __router(enter_event)
    }} else if forward_event!._message == "$>" {{
        // Forwarded event IS $> — dispatch directly so the
        // destination's $> handler receives the caller's payload.
        __router(forward_event!)
    }} else {{
        // Forwarded event is not $> — initialize the destination
        // with a fresh $>, then dispatch the forward.
        let enter_event = {evt}(message: "$>", parameters: __compartment.enter_args)
        __router(enter_event)
        __router(forward_event!)
    }}
    for ctx in _context_stack {{
        ctx._transitioned = true
    }}
}}"#,
                    comp = compartment_class,
                    evt = event_class
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
        // __compartment.state at call time and routes to the matching
        // state dispatcher inline (no __route_to_state indirection).
        let event_class = format!("{}FrameEvent", system.name);
        let states: Vec<&str> = system
            .machine
            .as_ref()
            .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default();
        let mut router_code = String::new();
        for (i, state) in states.iter().enumerate() {
            let prefix = if i == 0 { "if" } else { "} else if" };
            router_code.push_str(&format!(
                "{} __compartment.state == \"{}\" {{\n",
                prefix, state
            ));
            router_code.push_str(&format!("    _state_{}(__e, __compartment)\n", state));
        }
        if !states.is_empty() {
            router_code.push_str("}");
        }
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
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
        _system: &SystemAst,
        compartment_class: &str,
    ) -> Option<CodegenNode> {
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next").with_type(compartment_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__next_compartment = next".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
