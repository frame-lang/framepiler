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

    fn emit_route_to_state(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        let compartment_class = format!("{}Compartment", system.name);
        let dispatch_call = if self.is_ts {
            "(this as any)[handler_name]".to_string()
        } else {
            "this[handler_name]".to_string()
        };
        let params = if self.is_ts {
            vec![
                Param::new("state_name").with_type("string"),
                Param::new("__e").with_type(&event_class),
                Param::new("compartment").with_type(&compartment_class),
            ]
        } else {
            vec![
                Param::new("state_name"),
                Param::new("__e"),
                Param::new("compartment"),
            ]
        };
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params,
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"const handler_name = `_state_${{state_name}}`;
const handler = {dispatch};
if (handler) {{
    handler.call(this, __e, compartment);
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
                    r#"while (this.__next_compartment !== null) {{
    const next_compartment = this.__next_compartment;
    this.__next_compartment = null;
    // Exit the current (leaf) state
    const exit_event = new {evt}("<$", this.__compartment.exit_args);
    this.__route_to_state(this.__compartment.state, exit_event, this.__compartment);
    // Switch to the new compartment
    this.__compartment = next_compartment;
    // Enter the new (leaf) state — or the forwarded event
    if (next_compartment.forward_event === null) {{
        const enter_event = new {evt}("$>", this.__compartment.enter_args);
        this.__route_to_state(this.__compartment.state, enter_event, this.__compartment);
    }} else {{
        const forward_event = next_compartment.forward_event;
        next_compartment.forward_event = null;
        const enter_event = new {evt}("$>", this.__compartment.enter_args);
        this.__route_to_state(this.__compartment.state, enter_event, this.__compartment);
        if (forward_event._message !== "$>") {{
            this.__router(forward_event);
        }}
    }}
    // Mark all stacked contexts as transitioned
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

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"// Route event to current state
this.__router(__e);
// Process any pending transition
this.__process_transition_loop();"#
                    .to_string(),
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
                code: "this.__route_to_state(this.__compartment.state, __e, this.__compartment);"
                    .to_string(),
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
