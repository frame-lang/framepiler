//! C machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_c_machinery` (4.2 plan
//! §7.1.P3). C diverges from every other backend along several axes:
//!
//! - **No class.** Every method is a free function `Sys_method(self,
//!   ...)`. `self` is an explicit parameter.
//! - **No GC.** Cleanup is manual: an extra `destroy(self)` postlude
//!   function frees the compartment chain, state stack, context stack,
//!   and the system struct itself. The trait's `emit_postlude` hook
//!   carries it.
//! - **`hsm_chain` is a switch on `leaf`** that materializes a
//!   `static const char* []` and returns its length via an out-pointer,
//!   not a map literal. Same role as Node 1 in other backends, just
//!   shaped to C.
//! - **`__route_to_state`** uses an if/else-if chain over
//!   `strcmp(state_name, "...") == 0` — same shape as the
//!   Java/Kotlin/Swift trio.
//!
//! The two dispatch helpers (`router_dispatch`, `route_to_state_dispatch`)
//! were previously private to `system_codegen.rs`; they now live with the
//! C machinery since nothing else uses them.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct CMachinery;

/// Thin `__router` body — delegates to `__route_to_state` with the
/// active compartment.
fn c_router_dispatch(system: &SystemAst) -> String {
    let sys = &system.name;
    format!("{sys}_route_to_state(self, self->__compartment->state, __e, self->__compartment);")
}

/// Per-handler `__route_to_state` body. Routes by state name to the
/// state's dispatcher with a specific compartment — used by cascade
/// helpers in addition to the `__router` thin wrapper. See
/// `docs/frame_runtime.md § "Dispatch Model"`.
fn c_route_to_state_dispatch(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();
    if let Some(ref machine) = system.machine {
        for (i, state) in machine.states.iter().enumerate() {
            let cond = if i == 0 { "if" } else { "} else if" };
            code.push_str(&format!(
                "{} (strcmp(state_name, \"{}\") == 0) {{\n    {}_state_{}(self, __e, compartment);\n",
                cond, state.name, sys, state.name
            ));
        }
        if !machine.states.is_empty() {
            code.push_str("}");
        }
    }
    code
}

impl MachineryGenerator for CMachinery {
    fn lang_name(&self) -> &'static str {
        "c"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // hsm_chain — function that fills out *out_chain with const char*
        // pointers and returns the chain length. C has no map literal,
        // so we use a switch on the leaf name.
        let mut chain_body = String::from("if (false) { (void)0; }\n");
        for (leaf, chain) in chains {
            chain_body.push_str(&format!(
                "    else if (strcmp(leaf, \"{}\") == 0) {{\n        static const char* __chain[] = {{ ",
                leaf
            ));
            for (i, name) in chain.iter().enumerate() {
                if i > 0 {
                    chain_body.push_str(", ");
                }
                chain_body.push_str(&format!("\"{}\"", name));
            }
            chain_body.push_str(&format!(
                " }};\n        *out_chain = __chain;\n        return {};\n    }}\n",
                chain.len()
            ));
        }
        chain_body.push_str("    *out_chain = NULL;\n    return 0;");
        Some(CodegenNode::Method {
            name: "__hsm_chain".to_string(),
            params: vec![
                Param::new("leaf").with_type("const char*"),
                Param::new("out_chain").with_type("const char***"),
            ],
            return_type: Some("int".to_string()),
            body: vec![CodegenNode::NativeBlock {
                code: chain_body,
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_prepare_enter(
        &self,
        system: &SystemAst,
        _compartment_class: &str,
    ) -> Option<CodegenNode> {
        let sys = &system.name;
        Some(CodegenNode::Method {
            name: "__prepareEnter".to_string(),
            params: vec![
                Param::new("leaf").with_type("const char*"),
                Param::new("state_args").with_type(&format!("{}_FrameVec*", sys)),
                Param::new("enter_args").with_type(&format!("{}_FrameVec*", sys)),
            ],
            return_type: Some(format!("{}_Compartment*", sys)),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"const char** chain = NULL;
int n = {sys}_hsm_chain(self, leaf, &chain);
{sys}_Compartment* comp = NULL;
for (int i = 0; i < n; i++) {{
    {sys}_Compartment* nc = {sys}_Compartment_new(chain[i]);
    if (state_args) {{
        for (int j = 0; j < state_args->size; j++) {sys}_FrameVec_push(nc->state_args, state_args->items[j]);
    }}
    if (enter_args) {{
        for (int j = 0; j < enter_args->size; j++) {sys}_FrameVec_push(nc->enter_args, enter_args->items[j]);
    }}
    nc->parent_compartment = comp;  // adopts ref
    comp = nc;
}}
return comp;"#,
                    sys = sys
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
        let sys = &system.name;
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type(&format!("{}_FrameVec*", sys))],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{sys}_Compartment* comp = self->__compartment;
while (comp != NULL) {{
    // Clear any prior exit_args before copying the new ones in.
    while (comp->exit_args->size > 0) comp->exit_args->size--;
    if (exit_args) {{
        for (int j = 0; j < exit_args->size; j++) {sys}_FrameVec_push(comp->exit_args, exit_args->items[j]);
    }}
    comp = comp->parent_compartment;
}}"#,
                    sys = sys
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
        let sys = &system.name;
        // __route_to_state — cascade router. Same dispatch logic as
        // __router but takes an explicit state name and compartment.
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name").with_type("const char*"),
                Param::new("__e").with_type(&format!("{}_FrameEvent*", sys)),
                Param::new("compartment").with_type(&format!("{}_Compartment*", sys)),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: c_route_to_state_dispatch(system),
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
        _event_class: &str,
    ) -> Option<CodegenNode> {
        let sys = &system.name;
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while (self->__next_compartment != NULL) {{
    {sys}_Compartment* next_compartment = self->__next_compartment;
    self->__next_compartment = NULL;
    // Exit the current (leaf) state
    {sys}_FrameEvent* __exit_event = {sys}_FrameEvent_new("<$", self->__compartment->exit_args, 0);
    {sys}_route_to_state(self, self->__compartment->state, __exit_event, self->__compartment);
    {sys}_FrameEvent_destroy(__exit_event);
    {sys}_Compartment_unref(self->__compartment);
    self->__compartment = next_compartment;
    // Enter the new (leaf) state — or the forwarded event
    if (next_compartment->forward_event == NULL) {{
        {sys}_FrameEvent* __enter_event = {sys}_FrameEvent_new("$>", self->__compartment->enter_args, 0);
        {sys}_route_to_state(self, self->__compartment->state, __enter_event, self->__compartment);
        {sys}_FrameEvent_destroy(__enter_event);
    }} else {{
        {sys}_FrameEvent* forward_event = next_compartment->forward_event;
        next_compartment->forward_event = NULL;
        {sys}_FrameEvent* __enter_event = {sys}_FrameEvent_new("$>", self->__compartment->enter_args, 0);
        {sys}_route_to_state(self, self->__compartment->state, __enter_event, self->__compartment);
        {sys}_FrameEvent_destroy(__enter_event);
        if (strcmp(forward_event->_message, "$>") != 0) {{
            {sys}_router(self, forward_event);
        }}
    }}
    for (int __i = 0; __i < self->_context_stack->size; __i++) {{
        (({sys}_FrameContext*)self->_context_stack->items[__i])->_transitioned = 1;
    }}
}}"#,
                    sys = sys
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
        let sys = &system.name;
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    "{sys}_router(self, __e);\n{sys}_process_transition_loop(self);",
                    sys = sys
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
        let sys = &system.name;
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: c_router_dispatch(system),
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
        system: &SystemAst,
        _compartment_class: &str,
    ) -> Option<CodegenNode> {
        let sys = &system.name;
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next_compartment").with_type(&format!("{}_Compartment*", sys))],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "self->__next_compartment = next_compartment;".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_postlude(&self, system: &SystemAst) -> Vec<CodegenNode> {
        let sys = &system.name;
        // destroy method - cleanup system resources
        vec![CodegenNode::Method {
            name: "destroy".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"// Unref current compartment (may free if not on stack)
if (self->__compartment) {sys}_Compartment_unref(self->__compartment);
if (self->__next_compartment) {sys}_Compartment_unref(self->__next_compartment);
// Unref all state stack entries
if (self->_state_stack) {{
    for (int __i = 0; __i < self->_state_stack->size; __i++) {{
        {sys}_Compartment_unref(({sys}_Compartment*)self->_state_stack->items[__i]);
    }}
    {sys}_FrameVec_destroy(self->_state_stack);
}}
if (self->_context_stack) {sys}_FrameVec_destroy(self->_context_stack);
free(self);"#,
                    sys = sys
                ),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        }]
    }
}
