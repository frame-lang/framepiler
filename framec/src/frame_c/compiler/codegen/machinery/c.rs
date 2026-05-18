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

/// RFC-0020 `__router` body — the single dispatch primitive.
/// Reads `self->__compartment->state` at call time and routes to the
/// matching state dispatcher with the active compartment. The
/// intermediate `__route_to_state` indirection that earlier codegen
/// emitted is gone.
fn c_router_inline_dispatch(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();
    if let Some(ref machine) = system.machine {
        for (i, state) in machine.states.iter().enumerate() {
            let cond = if i == 0 { "if" } else { "} else if" };
            code.push_str(&format!(
                "{} (strcmp(self->__compartment->state, \"{}\") == 0) {{\n    {}_state_{}(self, __e, self->__compartment);\n",
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

    fn emit_route_to_state(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: the dispatch table that __route_to_state used to
        // hold is now inlined directly into __router. No separate
        // __route_to_state function is emitted.
        None
    }

    fn emit_process_transition_loop(
        &self,
        _system: &SystemAst,
        _event_class: &str,
    ) -> Option<CodegenNode> {
        // RFC-0020: the drain loop is inlined inside __kernel. No
        // separate __process_transition_loop function is emitted.
        None
    }

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        let sys = &system.name;
        // __kernel — dispatches an event then drains any transitions
        // queued by the handler. Per RFC-0020 the drain loop is
        // inlined here, and forward-event dispatch follows the
        // three-branch protocol:
        //   - forward_event NULL: synthesize a fresh $>
        //   - forward_event is $>: dispatch the forward directly so
        //     the destination receives the caller's original payload
        //   - otherwise: synthesize $> first, then dispatch the forward
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"{sys}_router(self, __e);
while (self->__next_compartment != NULL) {{
    {sys}_Compartment* next_compartment = self->__next_compartment;
    self->__next_compartment = NULL;
    // Exit the current (leaf) state
    {sys}_FrameEvent* __exit_event = {sys}_FrameEvent_new("<$", self->__compartment->exit_args, 0);
    {sys}_router(self, __exit_event);
    {sys}_FrameEvent_destroy(__exit_event);
    {sys}_Compartment_unref(self->__compartment);
    self->__compartment = next_compartment;
    if (next_compartment->forward_event == NULL) {{
        // No forwarded event — synthesize a fresh $>
        {sys}_FrameEvent* __enter_event = {sys}_FrameEvent_new("$>", self->__compartment->enter_args, 0);
        {sys}_router(self, __enter_event);
        {sys}_FrameEvent_destroy(__enter_event);
    }} else if (strcmp(next_compartment->forward_event->_message, "$>") == 0) {{
        // Forwarded event IS $> — dispatch directly so the
        // destination's $> handler receives the caller's payload.
        // The forward_event is borrowed (owned by the wrapper that
        // queued the transition) — do NOT destroy it here.
        {sys}_FrameEvent* forward_event = next_compartment->forward_event;
        next_compartment->forward_event = NULL;
        {sys}_router(self, forward_event);
    }} else {{
        // Forwarded event is not $> — initialize the destination
        // with a fresh $>, then dispatch the forward to it. The
        // forward_event is borrowed; only the synthesized $> belongs
        // to the kernel and is freed here.
        {sys}_FrameEvent* forward_event = next_compartment->forward_event;
        next_compartment->forward_event = NULL;
        {sys}_FrameEvent* __enter_event = {sys}_FrameEvent_new("$>", self->__compartment->enter_args, 0);
        {sys}_router(self, __enter_event);
        {sys}_FrameEvent_destroy(__enter_event);
        {sys}_router(self, forward_event);
    }}
    // Mark every stacked context as having transitioned. Read by
    // @@:self.X() guard so outer self-calls short-circuit.
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

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        let sys = &system.name;
        // __router — single dispatch primitive. Reads
        // self->__compartment->state at call time and routes to the
        // current state's dispatcher, passing self->__compartment so
        // the dispatcher can reach state-args / state-vars / enter-args
        // / exit-args. Same primitive used for wrapper events and for
        // the synthesized <$ / $> events emitted inside __kernel's
        // drain loop.
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: c_router_inline_dispatch(system),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_transition(&self, system: &SystemAst, _compartment_class: &str) -> Option<CodegenNode> {
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
