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

    fn emit_route_to_state(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        let compartment_class = format!("{}Compartment", system.name);
        let comp_ptr = format!("std::shared_ptr<{}>", compartment_class);
        let states: Vec<&str> = system
            .machine
            .as_ref()
            .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default();
        let mut route_code = String::new();
        for (i, state) in states.iter().enumerate() {
            let prefix = if i == 0 { "if" } else { "} else if" };
            route_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
            route_code.push_str(&format!("    _state_{}(__e, compartment);\n", state));
        }
        if !states.is_empty() {
            route_code.push_str("}");
        }
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name").with_type("const std::string&"),
                Param::new("__e").with_type(&format!("{}&", event_class)),
                Param::new("compartment").with_type(&comp_ptr),
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
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while (__next_compartment) {{
    auto next_compartment = std::move(__next_compartment);
    // Exit the current (leaf) state
    {evt} __exit_event("<$", __compartment->exit_args);
    __route_to_state(__compartment->state, __exit_event, __compartment);
    __compartment = std::move(next_compartment);
    // Enter the new (leaf) state — or the forwarded event
    if (!__compartment->forward_event) {{
        {evt} __enter_event("$>", __compartment->enter_args);
        __route_to_state(__compartment->state, __enter_event, __compartment);
    }} else {{
        auto forward_event = std::move(__compartment->forward_event);
        {evt} __enter_event("$>", __compartment->enter_args);
        __route_to_state(__compartment->state, __enter_event, __compartment);
        if (forward_event->_message != "$>") {{
            __router(*forward_event);
        }}
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

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
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
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__route_to_state(__compartment->state, __e, __compartment);".to_string(),
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
