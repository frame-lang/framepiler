//! JavaScript / TypeScript machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_javascript_machinery`
//! (4.2 plan §7.1.P3). TypeScript and JavaScript share the same
//! machinery emission with the only difference being type annotations
//! and the dispatch-cast for handler lookup. Modeled here as one impl
//! parameterised by `is_ts: bool` — matches the pre-refactor shape
//! exactly (which the matrix gate verifies byte-canonical).

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct JavaScriptMachinery {
    pub(crate) is_ts: bool,
}

impl MachineryGenerator for JavaScriptMachinery {
    fn lang_name(&self) -> &'static str {
        if self.is_ts {
            "typescript"
        } else {
            "javascript"
        }
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        let mut chain_lines = String::new();
        if self.is_ts {
            chain_lines.push_str("static readonly _HSM_CHAIN: Record<string, string[]> = {\n");
        } else {
            chain_lines.push_str("static _HSM_CHAIN = {\n");
        }
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_lines.push_str(&format!("    \"{}\": [{}],\n", leaf, chain_str));
        }
        chain_lines.push_str("};");
        Some(CodegenNode::NativeBlock {
            code: chain_lines,
            span: None,
        })
    }

    fn emit_prepare_enter(
        &self,
        system: &SystemAst,
        compartment_class: &str,
    ) -> Option<CodegenNode> {
        let params = if self.is_ts {
            vec![
                Param::new("leaf").with_type("string"),
                Param::new("state_args").with_type("any[]"),
                Param::new("enter_args").with_type("any[]"),
            ]
        } else {
            vec![
                Param::new("leaf"),
                Param::new("state_args"),
                Param::new("enter_args"),
            ]
        };
        Some(CodegenNode::Method {
            name: "__prepareEnter".to_string(),
            params,
            return_type: if self.is_ts {
                Some(compartment_class.to_string())
            } else {
                None
            },
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"let comp{cast} = null;
for (const name of {sys}._HSM_CHAIN[leaf]) {{
    const new_comp = new {comp}(name);
    new_comp.state_args = [...state_args];
    new_comp.enter_args = [...enter_args];
    new_comp.parent_compartment = comp;
    comp = new_comp;
}}
return comp{nonnull};"#,
                    cast = if self.is_ts {
                        format!(": {} | null", compartment_class)
                    } else {
                        String::new()
                    },
                    sys = system.name,
                    comp = compartment_class,
                    nonnull = if self.is_ts { "!" } else { "" }
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
        let params = if self.is_ts {
            vec![Param::new("exit_args").with_type("any[]")]
        } else {
            vec![Param::new("exit_args")]
        };
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params,
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"let comp{cast} = this.__compartment;
while (comp !== null) {{
    comp.exit_args = [...exit_args];
    comp = comp.parent_compartment;
}}"#,
                    cast = if self.is_ts {
                        format!(": {} | null", compartment_class)
                    } else {
                        String::new()
                    },
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
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"// Route event to current state.
this.__router(__e);
// Drain any transitions queued by the handler.
while (this.__next_compartment !== null) {{
    const next_compartment = this.__next_compartment;
    this.__next_compartment = null;
    // Exit the current (leaf) state.
    const exit_event = new {evt}("<$", this.__compartment.exit_args);
    this.__router(exit_event);
    // Switch to the new compartment.
    this.__compartment = next_compartment;
    // Three-branch forward-event handling.
    const forward_event = next_compartment.forward_event;
    next_compartment.forward_event = null;
    if (forward_event === null) {{
        // No forwarded event — synthesize a fresh $>.
        const enter_event = new {evt}("$>", this.__compartment.enter_args);
        this.__router(enter_event);
    }} else if (forward_event._message === "$>") {{
        // Forwarded event IS $> — dispatch directly so the
        // destination's $> handler receives the caller's payload.
        this.__router(forward_event);
    }} else {{
        // Forwarded event is not $> — initialize the destination
        // with a fresh $>, then dispatch the forward.
        const enter_event = new {evt}("$>", this.__compartment.enter_args);
        this.__router(enter_event);
        this.__router(forward_event);
    }}
    for (const ctx of this._context_stack) {{
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
        // RFC-0020: __router is the single dispatch primitive. Inline
        // state-name match; no __route_to_state indirection.
        let event_class = format!("{}FrameEvent", system.name);
        let states: Vec<&str> = system
            .machine
            .as_ref()
            .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default();
        let dispatch_call = if self.is_ts {
            "(this as any)[handler_name]".to_string()
        } else {
            "this[handler_name]".to_string()
        };
        // Use reflective dispatch keyed on state string so optional
        // state methods (private/missing) still fall through cleanly,
        // matching the prior __route_to_state behavior.
        let _ = states;
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"const handler_name = `_state_${{this.__compartment.state}}`;
const handler = {dispatch};
if (handler) {{
    handler.call(this, __e, this.__compartment);
}}"#,
                    dispatch = dispatch_call
                ),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_transition(&self, _system: &SystemAst, compartment_class: &str) -> Option<CodegenNode> {
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next_compartment").with_type(compartment_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "this.__next_compartment = next_compartment;".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
