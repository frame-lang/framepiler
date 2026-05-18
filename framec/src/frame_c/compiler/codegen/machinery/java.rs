//! Java machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_java_machinery` (4.2
//! plan §7.1.P2 — JVM/reference-semantics canary). Body strings
//! preserved exactly so the matrix gate proves byte-for-byte
//! equivalence with the pre-refactor codegen.
//!
//! Java's machinery is closer to Python's than to Rust's — `hsm_chain`
//! is an instance method (Java has no static-dict initializer
//! ergonomic), `__route_to_state` uses cascading if/else-if instead of
//! a Rust-style match table (Java's `switch` on strings is uglier than
//! the if-chain), and the typed signatures use `ArrayList<Object>` for
//! state/enter/exit args. `__create` is emitted elsewhere (the
//! Constructor arm in `backends/java.rs`), not in this machinery.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct JavaMachinery;

impl MachineryGenerator for JavaMachinery {
    fn lang_name(&self) -> &'static str {
        "java"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // hsm_chain — instance method returning the topology table.
        let mut chain_method = String::from(
            "private java.util.HashMap<String, java.util.ArrayList<String>> hsm_chain() {\n    java.util.HashMap<String, java.util.ArrayList<String>> m = new java.util.HashMap<>();\n",
        );
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!(
                "    m.put(\"{}\", new java.util.ArrayList<>(java.util.Arrays.asList({})));\n",
                leaf, chain_str
            ));
        }
        chain_method.push_str("    return m;\n}");
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
                Param::new("state_args").with_type("java.util.ArrayList<Object>"),
                Param::new("enter_args").with_type("java.util.ArrayList<Object>"),
            ],
            return_type: Some(compartment_class.to_string()),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{0} comp = null;
for (String name : hsm_chain().get(leaf)) {{
    {0} new_comp = new {0}(name);
    new_comp.state_args = new java.util.ArrayList<>(state_args);
    new_comp.enter_args = new java.util.ArrayList<>(enter_args);
    new_comp.parent_compartment = comp;
    comp = new_comp;
}}
return comp;"#,
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
        // __prepareExit — populates exit_args on every layer.
        let compartment_class = format!("{}Compartment", system.name);
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type("java.util.ArrayList<Object>")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{} comp = __compartment;
while (comp != null) {{
    comp.exit_args = new java.util.ArrayList<>(exit_args);
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
        // Java specifics:
        // - All references are bare (no `this.`); fields resolve
        //   against the enclosing class instance.
        // - GC handles event lifetimes; nil/null = nullable pointer.
        // - String compare uses `.equals(...)`; for-each
        //   `for (Type ctx : _context_stack)` mutates by reference
        //   since FrameContext is a class.
        let event_class = format!("{}FrameEvent", system.name);
        let compartment_class = format!("{}Compartment", system.name);
        let context_class = format!("{}FrameContext", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"// Route event to current state.
__router(__e);
// Drain any transitions queued by the handler.
while (__next_compartment != null) {{
    {comp} next_compartment = __next_compartment;
    __next_compartment = null;
    // Exit the current (leaf) state.
    {evt} exit_event = new {evt}("<$", __compartment.exit_args);
    __router(exit_event);
    // Switch to the new compartment.
    __compartment = next_compartment;
    // Three-branch forward-event handling.
    {evt} forward_event = next_compartment.forward_event;
    next_compartment.forward_event = null;
    if (forward_event == null) {{
        // No forwarded event — synthesize a fresh $>.
        {evt} enter_event = new {evt}("$>", __compartment.enter_args);
        __router(enter_event);
    }} else if (forward_event._message.equals("$>")) {{
        // Forwarded event IS $> — dispatch directly so the
        // destination's $> handler receives the caller's payload.
        __router(forward_event);
    }} else {{
        // Forwarded event is not $> — initialize the destination
        // with a fresh $>, then dispatch the forward.
        {evt} enter_event = new {evt}("$>", __compartment.enter_args);
        __router(enter_event);
        __router(forward_event);
    }}
    for ({ctx} ctx : _context_stack) {{
        ctx._transitioned = true;
    }}
}}"#,
                    comp = compartment_class,
                    evt = event_class,
                    ctx = context_class
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
                "{} (__compartment.state.equals(\"{}\")) {{\n",
                prefix, state
            ));
            router_code.push_str(&format!("    _state_{}(__e, __compartment);\n", state));
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

    fn emit_transition(&self, _system: &SystemAst, compartment_class: &str) -> Option<CodegenNode> {
        // __transition
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next").with_type(compartment_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__next_compartment = next;".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
