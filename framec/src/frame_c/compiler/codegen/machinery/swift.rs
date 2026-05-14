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

    fn emit_route_to_state(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        let compartment_class = format!("{}Compartment", system.name);
        let states: Vec<&str> = system
            .machine
            .as_ref()
            .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default();
        let mut route_code = String::new();
        for (i, state) in states.iter().enumerate() {
            let prefix = if i == 0 { "if" } else { "} else if" };
            route_code.push_str(&format!("{} state_name == \"{}\" {{\n", prefix, state));
            route_code.push_str(&format!("    _state_{}(__e, compartment)\n", state));
        }
        if !states.is_empty() {
            route_code.push_str("}");
        }
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name").with_type("String"),
                Param::new("__e").with_type(&event_class),
                Param::new("compartment").with_type(&compartment_class),
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
        _system: &SystemAst,
        event_class: &str,
    ) -> Option<CodegenNode> {
        // RFC-0019: no enter/exit cascade. `$>` / `<$` are ordinary events,
        // dispatched to the current (leaf) state via __route_to_state.
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while __next_compartment != nil {{
    let next_compartment = __next_compartment!
    __next_compartment = nil
    let exit_event = {evt}(message: "<$", parameters: __compartment.exit_args)
    __route_to_state(__compartment.state, exit_event, __compartment)
    __compartment = next_compartment
    if next_compartment.forward_event == nil {{
        let enter_event = {evt}(message: "$>", parameters: __compartment.enter_args)
        __route_to_state(__compartment.state, enter_event, __compartment)
    }} else {{
        let forward_event = next_compartment.forward_event!
        next_compartment.forward_event = nil
        let enter_event = {evt}(message: "$>", parameters: __compartment.enter_args)
        __route_to_state(__compartment.state, enter_event, __compartment)
        if forward_event._message != "$>" {{
            __router(forward_event)
        }}
    }}
    for i in 0..<_context_stack.count {{
        _context_stack[i]._transitioned = true
    }}
}}"#,
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

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__router(__e)\n__process_transition_loop()".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__route_to_state(__compartment.state, __e, __compartment)".to_string(),
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
