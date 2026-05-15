//! GDScript machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_gdscript_machinery`
//! (4.2 plan §7.1.P2 — Godot-specific canary). Body strings preserved
//! exactly so the matrix gate proves byte-for-byte equivalence.
//!
//! GDScript is functionally similar to Python's machinery (dynamic
//! dispatch via `self.call(handler_name, ...)`, `Dictionary` literal
//! for the chain table) but uses Godot's GDScript syntax — `func` /
//! `var` / `null` / `:=` / colon-block scoping. The `_create` factory
//! (RFC-0017) is emitted by the Constructor arm in the GDScript
//! backend module, not here.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct GDScriptMachinery;

impl MachineryGenerator for GDScriptMachinery {
    fn lang_name(&self) -> &'static str {
        "gdscript"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // hsm_chain — class method returning the topology table.
        let mut chain_method = String::from("func hsm_chain() -> Dictionary:\n    return {\n");
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("        \"{}\": [{}],\n", leaf, chain_str));
        }
        chain_method.push_str("    }");
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
                Param::new("leaf"),
                Param::new("state_args"),
                Param::new("enter_args"),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"var comp = null
for name in self.hsm_chain()[leaf]:
    var new_comp = {}.new(name)
    new_comp.state_args = state_args.duplicate()
    new_comp.enter_args = enter_args.duplicate()
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
        // __prepareExit — populates exit_args on every layer.
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"var comp = self.__compartment
while comp != null:
    comp.exit_args = exit_args.duplicate()
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
        // RFC-0020: __route_to_state's dispatch table is inlined into
        // __router. No separate method emitted.
        None
    }

    fn emit_process_transition_loop(
        &self,
        _system: &SystemAst,
        _event_class: &str,
    ) -> Option<CodegenNode> {
        // RFC-0020: drain loop is inlined inside __kernel.
        None
    }

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: __kernel dispatches one event then drains any
        // transitions queued by the handler. Inline drain loop with
        // three-branch forward-event protocol:
        //   - forward_event == null: synthesize a fresh $>
        //   - forward_event._message == "$>": dispatch directly so
        //     the destination's $> receives the caller's payload
        //   - otherwise: synthesize $> first, then dispatch the forward
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
while self.__next_compartment != null:
    var next_compartment = self.__next_compartment
    self.__next_compartment = null
    # Exit the current (leaf) state
    self.__router({evt}.new("<$", self.__compartment.exit_args))
    # Switch to the new compartment
    self.__compartment = next_compartment
    if next_compartment.forward_event == null:
        # No forwarded event — synthesize a fresh $>
        self.__router({evt}.new("$>", self.__compartment.enter_args))
    else:
        if next_compartment.forward_event._message == "$>":
            # Forwarded event IS $> — dispatch directly so the
            # destination's $> receives the caller's payload
            self.__router(next_compartment.forward_event)
        else:
            # Forwarded event is not $> — initialize the destination
            # with a fresh $>, then dispatch the forward to it
            self.__router({evt}.new("$>", self.__compartment.enter_args))
            self.__router(next_compartment.forward_event)
    next_compartment.forward_event = null
    # Mark all stacked contexts as transitioned
    for ctx in self._context_stack:
        ctx._transitioned = true"#,
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
        // RFC-0020: __router is the single dispatch primitive. Reads
        // self.__compartment.state at call time and routes to the
        // matching state dispatcher, passing the compartment.
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
            body.push_str("pass");
        } else {
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
        // __transition method
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
