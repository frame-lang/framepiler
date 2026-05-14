//! C# machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_csharp_machinery` (4.2
//! plan §7.1.P3). Structurally near-identical to Java's emission;
//! syntactic differences only: `Dictionary` + `List<T>`,
//! `null`-coalescing assertion `!`, `foreach` over the dict's index,
//! `string`-as-keyword keyword.
//!
//! `__create` (the RFC-0017 factory) is emitted by the Constructor arm
//! in `backends/csharp.rs`, not here.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct CSharpMachinery;

impl MachineryGenerator for CSharpMachinery {
    fn lang_name(&self) -> &'static str {
        "csharp"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        let mut chain_method = String::from(
            "private Dictionary<string, List<string>> hsm_chain() {\n    return new Dictionary<string, List<string>> {\n",
        );
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!(
                "        {{ \"{}\", new List<string> {{ {} }} }},\n",
                leaf, chain_str
            ));
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
        Some(CodegenNode::Method {
            name: "__prepareEnter".to_string(),
            params: vec![
                Param::new("leaf").with_type("string"),
                Param::new("state_args").with_type("List<object>"),
                Param::new("enter_args").with_type("List<object>"),
            ],
            return_type: Some(compartment_class.to_string()),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{0}? comp = null;
foreach (string name in hsm_chain()[leaf]) {{
    {0} new_comp = new {0}(name);
    new_comp.state_args = new List<object>(state_args);
    new_comp.enter_args = new List<object>(enter_args);
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
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type("List<object>")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{}? comp = __compartment;
while (comp != null) {{
    comp.exit_args = new List<object>(exit_args);
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
                Param::new("state_name").with_type("string"),
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
        if (forward_event._message != "$>") {{
            __router(forward_event);
        }}
    }}
    foreach (var ctx in _context_stack) {{
        ctx._transitioned = true;
    }}
}}"#,
                    compartment_class, event_class
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
