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
            "private HashMap<String, ArrayList<String>> hsm_chain() {\n    HashMap<String, ArrayList<String>> m = new HashMap<>();\n",
        );
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!(
                "    m.put(\"{}\", new ArrayList<>(java.util.Arrays.asList({})));\n",
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
                Param::new("state_args").with_type("ArrayList<Object>"),
                Param::new("enter_args").with_type("ArrayList<Object>"),
            ],
            return_type: Some(compartment_class.to_string()),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{0} comp = null;
for (String name : hsm_chain().get(leaf)) {{
    {0} new_comp = new {0}(name);
    new_comp.state_args = new ArrayList<>(state_args);
    new_comp.enter_args = new ArrayList<>(enter_args);
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
            params: vec![Param::new("exit_args").with_type("ArrayList<Object>")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{} comp = __compartment;
while (comp != null) {{
    comp.exit_args = new ArrayList<>(exit_args);
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
        // __route_to_state — cascade router. Same dispatch table as
        // __router but takes an explicit state name.
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
            route_code.push_str(&format!(
                "{} (state_name.equals(\"{}\")) {{\n",
                prefix, state
            ));
            route_code.push_str(&format!("    _state_{}(__e, compartment);\n", state));
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
        system: &SystemAst,
        event_class: &str,
    ) -> Option<CodegenNode> {
        // RFC-0019: no enter/exit cascade. `$>` / `<$` are ordinary
        // events, dispatched to the current (leaf) state via
        // __route_to_state; an ancestor's `$>` / `<$` runs only if the
        // leaf forwards (=> $^).
        //
        // __process_transition_loop — drains queued transitions.
        let compartment_class = format!("{}Compartment", system.name);
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while (__next_compartment != null) {{
    {0} next_compartment = __next_compartment;
    __next_compartment = null;
    {1} __exit_event = new {1}("<$", __compartment.exit_args);
    __route_to_state(__compartment.state, __exit_event, __compartment);
    __compartment = next_compartment;
    if (next_compartment.forward_event == null) {{
        {1} __enter_event = new {1}("$>", __compartment.enter_args);
        __route_to_state(__compartment.state, __enter_event, __compartment);
    }} else {{
        {1} forward_event = next_compartment.forward_event;
        next_compartment.forward_event = null;
        {1} __enter_event = new {1}("$>", __compartment.enter_args);
        __route_to_state(__compartment.state, __enter_event, __compartment);
        if (!forward_event._message.equals("$>")) {{
            __router(forward_event);
        }}
    }}
    for ({2}FrameContext ctx : _context_stack) {{
        ctx._transitioned = true;
    }}
}}"#,
                    compartment_class, event_class, system.name
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
        // __kernel — routes event then drains.
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__router(__e);\n__process_transition_loop();".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        // __router — delegates to __route_to_state.
        let event_class = format!("{}FrameEvent", system.name);
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
