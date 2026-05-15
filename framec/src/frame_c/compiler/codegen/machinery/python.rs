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
        // RFC-0020 (Python spec): the dispatch table that
        // __route_to_state used to hold is now inlined directly into
        // __router. No separate __route_to_state method is emitted.
        None
    }

    fn emit_process_transition_loop(
        &self,
        _system: &SystemAst,
        _event_class: &str,
    ) -> Option<CodegenNode> {
        // RFC-0020 (Python spec): the drain loop is inlined inside
        // __kernel. No separate __process_transition_loop method is
        // emitted.
        None
    }

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        // __kernel — dispatches an event to the current state, then
        // drains any transitions queued by the handler. Per RFC-0020
        // the drain loop is inlined here (no __process_transition_loop
        // helper), and forward-event dispatch follows the three-branch
        // protocol:
        //   - forward_event is None: synthesize a fresh $>
        //   - forward_event._message == "$>": dispatch the forward
        //     directly so the destination's $> receives the caller's
        //     original payload
        //   - otherwise: synthesize $> first, then dispatch the forward
        //     to the now-initialized state
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"# Route event to current state
self.__router(__e)
# Drain any transitions queued by the handler
while self.__next_compartment is not None:
    next_compartment = self.__next_compartment
    self.__next_compartment = None
    # Exit the current (leaf) state
    self.__router({ec}("<$", self.__compartment.exit_args))
    # Switch to the new compartment
    self.__compartment = next_compartment
    if next_compartment.forward_event is None:
        # No forwarded event — synthesize a fresh $>
        self.__router({ec}("$>", self.__compartment.enter_args))
    else:
        if next_compartment.forward_event._message == "$>":
            # Forwarded event IS $> — dispatch directly so the
            # destination's $> receives the caller's payload
            self.__router(next_compartment.forward_event)
        else:
            # Forwarded event is not $> — initialize the destination
            # with a fresh $>, then dispatch the forward to it
            self.__router({ec}("$>", self.__compartment.enter_args))
            self.__router(next_compartment.forward_event)
    next_compartment.forward_event = None
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

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        // __router — the single dispatch primitive. Reads the current
        // state from self.__compartment and routes to that state's
        // dispatcher method, passing the compartment along. Per
        // RFC-0020 the state-name match is inlined here (no separate
        // __route_to_state helper).
        let mut body = String::new();
        let states = system
            .machine
            .as_ref()
            .map(|m| m.states.as_slice())
            .unwrap_or(&[]);
        let mut first = true;
        for state in states {
            let keyword = if first { "if" } else { "elif" };
            body.push_str(&format!(
                "{} self.__compartment.state == \"{}\":\n    self._state_{}(__e, self.__compartment)\n",
                keyword, state.name, state.name
            ));
            first = false;
        }
        if first {
            // No states declared — emit a `pass` so the body is valid Python.
            body.push_str("pass");
        } else {
            // Trim trailing newline.
            body.pop();
        }
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: body,
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
