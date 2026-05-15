//! C++ machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_cpp_machinery` (4.2
//! plan §7.1.P3). The final backend, completing the 17-language sweep
//! (Erlang scoped out — gen_statem owns dispatch).
//!
//! C++ quirks worth flagging:
//!
//! - Compartments are `std::shared_ptr<SysCompartment>` throughout
//!   (RAII for cleanup; no `destroy` postlude needed).
//! - `hsm_chain()` returns `std::unordered_map<std::string,
//!   std::vector<std::string>>` by value; `__prepareEnter` saves it to
//!   a local `chain_table` before iterating to avoid the temporary's
//!   `operator[]` dangling.
//! - `__route_to_state` uses cascading `if (state_name == "...")` —
//!   same shape as the Java/Kotlin/Swift/C# family.
//! - Transition loop uses `std::move` on `__next_compartment` and
//!   `forward_event` to avoid ref-count churn through the queue.
//!
//! `__create` (the RFC-0017 factory) is emitted by the Constructor arm
//! in `backends/cpp.rs`, not here.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct CppMachinery;

impl MachineryGenerator for CppMachinery {
    fn lang_name(&self) -> &'static str {
        "cpp"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        let mut chain_method = String::from(
            "std::unordered_map<std::string, std::vector<std::string>> hsm_chain() {\n    return {\n",
        );
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("        {{\"{}\", {{{}}} }},\n", leaf, chain_str));
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
        let comp_ptr = format!("std::shared_ptr<{}>", compartment_class);
        Some(CodegenNode::Method {
            name: "__prepareEnter".to_string(),
            params: vec![
                Param::new("leaf").with_type("const std::string&"),
                Param::new("state_args").with_type("std::vector<std::any>"),
                Param::new("enter_args").with_type("std::vector<std::any>"),
            ],
            return_type: Some(comp_ptr.clone()),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    // Assign hsm_chain() to a local before iterating — the
                    // method returns by value, so a range-for over the
                    // temporary's [] result would dangle.
                    r#"{0} comp = nullptr;
auto chain_table = hsm_chain();
for (const auto& name : chain_table[leaf]) {{
    auto new_comp = std::make_shared<{1}>(name);
    new_comp->state_args = state_args;
    new_comp->enter_args = enter_args;
    new_comp->parent_compartment = comp;
    comp = new_comp;
}}
return comp;"#,
                    comp_ptr, compartment_class
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
        let comp_ptr = format!("std::shared_ptr<{}>", compartment_class);
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type("std::vector<std::any>")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{} comp = __compartment;
while (comp) {{
    comp->exit_args = exit_args;
    comp = comp->parent_compartment;
}}"#,
                    comp_ptr
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
        // RFC-0020: __router holds the dispatch table directly; no
        // separate __route_to_state helper.
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
        // C++ specifics:
        // - `__next_compartment` is `shared_ptr<Compartment>`; `std::move`
        //   leaves the source null so it acts as the loop guard.
        // - `forward_event` is `unique_ptr<FrameEvent>`; we `std::move`
        //   it out into a local before dispatching so the destination
        //   compartment's slot is clear.
        // - Synthesized `<$`/`$>` events are stack-local FrameEvents
        //   passed by reference into `__router`.
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"// Route event to current state.
__router(__e);
// Drain any transitions queued by the handler.
while (__next_compartment) {{
    auto next_compartment = std::move(__next_compartment);
    // Exit the current (leaf) state.
    {evt} __exit_event("<$", __compartment->exit_args);
    __router(__exit_event);
    // Switch to the new compartment.
    __compartment = std::move(next_compartment);
    // Three-branch forward-event handling.
    if (!__compartment->forward_event) {{
        // No forwarded event — synthesize a fresh $>.
        {evt} __enter_event("$>", __compartment->enter_args);
        __router(__enter_event);
    }} else if (__compartment->forward_event->_message == "$>") {{
        // Forwarded event IS $> — dispatch directly so the
        // destination's $> handler receives the caller's payload.
        auto forward_event = std::move(__compartment->forward_event);
        __router(*forward_event);
    }} else {{
        // Forwarded event is not $> — initialize the destination
        // with a fresh $>, then dispatch the forward.
        auto forward_event = std::move(__compartment->forward_event);
        {evt} __enter_event("$>", __compartment->enter_args);
        __router(__enter_event);
        __router(*forward_event);
    }}
    for (auto& ctx : _context_stack) {{
        ctx._transitioned = true;
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

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: __router is the single dispatch primitive. It
        // reads __compartment->state at call time and routes to the
        // matching state dispatcher inline (no __route_to_state
        // indirection).
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
                "{} (__compartment->state == \"{}\") {{\n",
                prefix, state
            ));
            router_code.push_str(&format!("    _state_{}(__e, __compartment);\n", state));
        }
        if !states.is_empty() {
            router_code.push_str("}");
        }
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
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
        let comp_ptr = format!("std::shared_ptr<{}>", compartment_class);
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next").with_type(&comp_ptr)],
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
