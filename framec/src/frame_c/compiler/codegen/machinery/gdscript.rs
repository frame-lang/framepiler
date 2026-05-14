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
        // __route_to_state — cascade router.
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name"),
                Param::new("__e"),
                Param::new("compartment"),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"var handler_name = "_state_" + state_name
if self.has_method(handler_name):
    self.call(handler_name, __e, compartment)"#
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
        // __route_to_state; an ancestor's `$>` / `<$` runs only if the
        // leaf forwards (=> $^).
        //
        // __process_transition_loop — drains pending transitions.
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while self.__next_compartment != null:
    var next_compartment = self.__next_compartment
    self.__next_compartment = null
    var exit_event = {evt}.new("<$", self.__compartment.exit_args)
    self.__route_to_state(self.__compartment.state, exit_event, self.__compartment)
    self.__compartment = next_compartment
    var enter_event = {evt}.new("$>", self.__compartment.enter_args)
    if next_compartment.forward_event == null:
        self.__route_to_state(self.__compartment.state, enter_event, self.__compartment)
    else:
        var forward_event = next_compartment.forward_event
        next_compartment.forward_event = null
        self.__route_to_state(self.__compartment.state, enter_event, self.__compartment)
        if forward_event._message != "$>":
            self.__router(forward_event)
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

    fn emit_kernel(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // __kernel — routes event then drains.
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "# Route event to current state\nself.__router(__e)\n# Process any pending transition\nself.__process_transition_loop()".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_router(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // __router — delegates to __route_to_state.
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "self.__route_to_state(self.__compartment.state, __e, self.__compartment)"
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
