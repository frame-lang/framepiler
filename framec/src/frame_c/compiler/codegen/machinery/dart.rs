//! Dart machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_dart_machinery` (4.2
//! plan §7.1.P3). Dart sits between the dynamic-dispatch family and
//! the statically-typed family:
//!
//! - Method dispatch in `__route_to_state` is a static `switch
//!   (state_name)` rather than `self["_state_..."]`, because Dart
//!   doesn't have a portable string-keyed instance-method lookup.
//! - Types are explicit (`String`, `List<dynamic>`, the
//!   compartment-class name); nullable variables use `?` and
//!   non-null assertion `!`.
//! - `\$` escapes inside raw `$>`/`<$` event names because Dart treats
//!   `$` as a string-interpolation prefix — see the `__EVT__`
//!   replacement on the transition-loop body.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct DartMachinery;

impl MachineryGenerator for DartMachinery {
    fn lang_name(&self) -> &'static str {
        "dart"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // hsm_chain — instance method returning the topology table.
        let mut chain_method =
            String::from("Map<String, List<String>> hsm_chain() {\n    return {\n");
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("        \"{}\": [{}],\n", leaf, chain_str));
        }
        chain_method.push_str("    };\n}");
        Some(CodegenNode::NativeBlock {
            code: chain_method,
            span: None,
        })
    }

    fn emit_prepare_enter(
        &self,
        _system: &SystemAst,
        compartment_class: &str,
    ) -> Option<CodegenNode> {
        // __prepareEnter — constructs the destination HSM chain.
        Some(CodegenNode::Method {
            name: "__prepareEnter".to_string(),
            params: vec![
                Param::new("leaf").with_type("String"),
                Param::new("state_args").with_type("List<dynamic>"),
                Param::new("enter_args").with_type("List<dynamic>"),
            ],
            return_type: Some(compartment_class.to_string()),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{0}? comp = null;
for (final name in hsm_chain()[leaf]!) {{
    final new_comp = {0}(name);
    new_comp.state_args = List<dynamic>.from(state_args);
    new_comp.enter_args = List<dynamic>.from(enter_args);
    new_comp.parent_compartment = comp;
    comp = new_comp;
}}
return comp!;"#,
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

    fn emit_prepare_exit(&self, system: &SystemAst) -> Option<CodegenNode> {
        let compartment_class = format!("{}Compartment", system.name);
        // __prepareExit — populates exit_args on every layer.
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type("List<dynamic>")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{}? comp = __compartment;
while (comp != null) {{
    comp.exit_args = List<dynamic>.from(exit_args);
    comp = comp.parent_compartment;
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
        // __route_to_state — cascade router. Uses the same switch-on-state
        // pattern as __router but takes an explicit state name.
        let mut route_code = String::from("switch (state_name) {\n");
        if let Some(ref machine) = system.machine {
            for state in &machine.states {
                route_code.push_str(&format!("    case \"{}\":\n", state.name));
                route_code.push_str(&format!(
                    "        _state_{}(__e, compartment);\n",
                    state.name
                ));
                route_code.push_str("        break;\n");
            }
        }
        route_code.push_str("}");
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
        // __process_transition_loop — drains queued transitions. RFC-0019:
        // <$/$> are dispatched to the leaf (current/new) compartment only —
        // ancestors run via an explicit `=> $^` forward, never a chain walk.
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"while (__next_compartment != null) {
    final next_compartment = __next_compartment!;
    __next_compartment = null;
    final exit_event = __EVT__("<\$", __compartment.exit_args);
    __route_to_state(__compartment.state, exit_event, __compartment);
    __compartment = next_compartment;
    if (next_compartment.forward_event == null) {
        final enter_event = __EVT__("\$>", __compartment.enter_args);
        __route_to_state(__compartment.state, enter_event, __compartment);
    } else {
        final forward_event = next_compartment.forward_event!;
        next_compartment.forward_event = null;
        final enter_event = __EVT__("\$>", __compartment.enter_args);
        __route_to_state(__compartment.state, enter_event, __compartment);
        if (forward_event._message != "\$>") {
            __router(forward_event);
        }
    }
    for (final ctx in _context_stack) {
        ctx._transitioned = true;
    }
}"#
                .replace("__EVT__", event_class),
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
        // __kernel — routes event then drains.
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "// Route event to current state\n__router(__e);\n// Process any pending transition\n__process_transition_loop();".to_string(),
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
        // __router — delegates to __route_to_state.
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__route_to_state(__compartment.state, __e, __compartment);".to_string(),
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
        // __transition method
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next_compartment").with_type(compartment_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__next_compartment = next_compartment;".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
