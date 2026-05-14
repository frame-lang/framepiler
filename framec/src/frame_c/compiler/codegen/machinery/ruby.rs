//! Ruby machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_ruby_machinery` (4.2
//! plan §7.1.P3). Ruby uses `@`-sigil instance variables, blocks for
//! iteration (`each do |name|`), `nil` for null, and method-name
//! dispatch via `respond_to?` + `send`.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct RubyMachinery;

impl MachineryGenerator for RubyMachinery {
    fn lang_name(&self) -> &'static str {
        "ruby"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // hsm_chain — class method returning the topology table. Emitted
        // as a method (not a constant) because Ruby's TestRunner loads
        // multiple test files into the same process. A constant
        // collision warning gets captured as test output and
        // misclassified as unrecognized. Memoization via class instance
        // variable also collides across test files (same class name,
        // different topology), so we recreate the hash on each call —
        // it's small, allocation cost is negligible vs. dispatch.
        let mut chain_method = String::from("def self.hsm_chain\n    {\n");
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("        \"{}\" => [{}],\n", leaf, chain_str));
        }
        chain_method.push_str("    }\nend");
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
                Param::new("leaf"),
                Param::new("state_args"),
                Param::new("enter_args"),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"comp = nil
self.class.hsm_chain[leaf].each do |name|
    new_comp = {}.new(name)
    new_comp.state_args = state_args.dup
    new_comp.enter_args = enter_args.dup
    new_comp.parent_compartment = comp
    comp = new_comp
end
comp"#,
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
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"comp = @__compartment
while comp != nil
    comp.exit_args = exit_args.dup
    comp = comp.parent_compartment
end"#
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
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name"),
                Param::new("__e"),
                Param::new("compartment"),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"handler_name = "_state_#{state_name}"
if respond_to?(handler_name, true)
    send(handler_name, __e, compartment)
end"#
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
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while @__next_compartment != nil
    next_compartment = @__next_compartment
    @__next_compartment = nil
    exit_event = {ec}.new("<$", @__compartment.exit_args)
    __route_to_state(@__compartment.state, exit_event, @__compartment)
    @__compartment = next_compartment
    if next_compartment.forward_event == nil
        enter_event = {ec}.new("$>", @__compartment.enter_args)
        __route_to_state(@__compartment.state, enter_event, @__compartment)
    else
        forward_event = next_compartment.forward_event
        next_compartment.forward_event = nil
        enter_event = {ec}.new("$>", @__compartment.enter_args)
        __route_to_state(@__compartment.state, enter_event, @__compartment)
        if forward_event._message != "$>"
            __router(forward_event)
        end
    end
    @_context_stack.each {{ |ctx| ctx._transitioned = true }}
end"#,
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
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "# Route event to current state\n__router(__e)\n# Process any pending transition\n__process_transition_loop".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_router(&self, _system: &SystemAst) -> Option<CodegenNode> {
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__route_to_state(@__compartment.state, __e, @__compartment)".to_string(),
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
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next_compartment")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "@__next_compartment = next_compartment".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
