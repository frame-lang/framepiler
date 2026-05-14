//! Python 3 machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_python3_machinery`
//! (4.2 plan §7.1.P1 — canary). Body strings preserved exactly so the
//! 17-backend matrix gate proves byte-for-byte equivalence with the
//! pre-refactor codegen.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct PythonMachinery;

impl MachineryGenerator for PythonMachinery {
    fn lang_name(&self) -> &'static str {
        "python_3"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // _HSM_CHAIN class attribute — static topology table mapping each
        // state name to its root-to-leaf ancestor chain. Used by
        // __prepareEnter to construct destination compartment chains and
        // by restore_state() to reconstruct chains from a save blob.
        let mut chain_lines = String::from("_HSM_CHAIN = {\n");
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_lines.push_str(&format!("    \"{}\": [{}],\n", leaf, chain_str));
        }
        chain_lines.push('}');
        Some(CodegenNode::NativeBlock {
            code: chain_lines,
            span: None,
        })
    }

    fn emit_prepare_enter(
        &self,
        _system: &SystemAst,
        compartment_class: &str,
    ) -> Option<CodegenNode> {
        // __prepareEnter — constructs the destination HSM chain for a
        // transition. Each compartment in the chain receives its own copy
        // of state_args / enter_args; ancestors and the leaf get the
        // same values under the signature-match rule (see
        // docs/frame_runtime.md § "Uniform Parameter Propagation").
        Some(CodegenNode::Method {
            name: "__prepareEnter".to_string(),
            params: vec![
                Param::new("leaf"),
                Param::new("state_args"),
                Param::new("enter_args"),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"comp = None
for name in self._HSM_CHAIN[leaf]:
    new_comp = {}(name)
    new_comp.state_args = list(state_args)
    new_comp.enter_args = list(enter_args)
    new_comp.parent_compartment = comp
    comp = new_comp
return comp"#,
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

    fn emit_prepare_exit(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // __prepareExit — populates exit_args on every compartment in
        // the current source chain before the kernel dispatches `<$`.
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"comp = self.__compartment
while comp is not None:
    comp.exit_args = list(exit_args)
    comp = comp.parent_compartment"#
                    .to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_route_to_state(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // __route_to_state — routes an event to a specific state's
        // dispatcher with a specific compartment, instead of using
        // self.__compartment. Used by `=> $^` ancestor-forward
        // lowering (RFC-0019).
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name"),
                Param::new("__e"),
                Param::new("compartment"),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"handler_name = f"_state_{state_name}"
handler = getattr(self, handler_name, None)
if handler:
    handler(__e, compartment)"#
                    .to_string(),
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
        // RFC-0019: no enter/exit cascade. `$>` / `<$` are ordinary
        // events, dispatched to the current (leaf) state via
        // __route_to_state. An ancestor's `$>` / `<$` runs only if the
        // leaf forwards the event (an in-handler `=> $^`, or a
        // state-level default-forward). The kernel still builds the
        // whole compartment chain (__prepareEnter); it just doesn't
        // auto-fire ancestor lifecycle handlers.
        //
        // __process_transition_loop — drains any pending transitions
        // queued by handlers (or by the start state's enter dispatch in
        // __frame_init). On each transition: dispatch `<$` to the
        // current (leaf) state, switch compartments, dispatch `$>` to
        // the new (leaf) state — then re-check for another queued
        // transition.
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while self.__next_compartment is not None:
    next_compartment = self.__next_compartment
    self.__next_compartment = None
    # Exit the current (leaf) state
    exit_event = {ec}("<$", self.__compartment.exit_args)
    self.__route_to_state(self.__compartment.state, exit_event, self.__compartment)
    # Switch to the new compartment
    self.__compartment = next_compartment
    # Enter the new (leaf) state — or the forwarded event
    if next_compartment.forward_event is None:
        enter_event = {ec}("$>", self.__compartment.enter_args)
        self.__route_to_state(self.__compartment.state, enter_event, self.__compartment)
    else:
        forward_event = next_compartment.forward_event
        next_compartment.forward_event = None
        enter_event = {ec}("$>", self.__compartment.enter_args)
        self.__route_to_state(self.__compartment.state, enter_event, self.__compartment)
        if forward_event._message != "$>":
            self.__router(forward_event)
    # Mark all stacked contexts as transitioned
    for ctx in self._context_stack:
        ctx._transitioned = True"#,
                    ec = event_class
                ),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_kernel(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // __kernel — the main event loop. Routes the event to the
        // current state's dispatcher, then drains any pending
        // transitions via __process_transition_loop.
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"# Route event to current state
self.__router(__e)
# Process any pending transition
self.__process_transition_loop()"#
                    .to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_router(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // __router — dispatches events to the current state's
        // dispatcher method. Always uses self.__compartment.
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"self.__route_to_state(self.__compartment.state, __e, self.__compartment)"#
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
        _compartment_class: &str,
    ) -> Option<CodegenNode> {
        // __transition — caches next compartment (deferred transition).
        // Does NOT execute the transition — __kernel does that after
        // the handler returns.
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next_compartment")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "self.__next_compartment = next_compartment".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
