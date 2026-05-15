//! PHP machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_php_machinery` (4.2
//! plan §7.1.P3). PHP uses `$` sigils on every variable, `->` for
//! method/field access, `=>` for map literals, `foreach...as`. Like
//! Python/GDScript, `hsm_chain` is an instance method.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct PhpMachinery;

impl MachineryGenerator for PhpMachinery {
    fn lang_name(&self) -> &'static str {
        "php"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        let mut chain_method = String::from("public function hsm_chain() {\n    return [\n");
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("        \"{}\" => [{}],\n", leaf, chain_str));
        }
        chain_method.push_str("    ];\n}");
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
                    r#"$comp = null;
foreach ($this->hsm_chain()[$leaf] as $name) {{
    $new_comp = new {}($name);
    $new_comp->state_args = $state_args;
    $new_comp->enter_args = $enter_args;
    $new_comp->parent_compartment = $comp;
    $comp = $new_comp;
}}
return $comp;"#,
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
                code: r#"$comp = $this->__compartment;
while ($comp !== null) {
    $comp->exit_args = $exit_args;
    $comp = $comp->parent_compartment;
}"#
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
        // RFC-0020: __router holds the dispatch table directly.
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
        // RFC-0020: __kernel dispatches one event then drains; 3-branch
        // forward-event protocol matches the Python reference.
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"// Route event to current state.
$this->__router($__e);
// Drain any transitions queued by the handler.
while ($this->__next_compartment !== null) {{
    $next_compartment = $this->__next_compartment;
    $this->__next_compartment = null;
    $exit_event = new {evt}("<$", $this->__compartment->exit_args);
    $this->__router($exit_event);
    $this->__compartment = $next_compartment;
    $forward_event = $next_compartment->forward_event;
    $next_compartment->forward_event = null;
    if ($forward_event === null) {{
        $enter_event = new {evt}("$>", $this->__compartment->enter_args);
        $this->__router($enter_event);
    }} else if ($forward_event->_message === "$>") {{
        $this->__router($forward_event);
    }} else {{
        $enter_event = new {evt}("$>", $this->__compartment->enter_args);
        $this->__router($enter_event);
        $this->__router($forward_event);
    }}
    foreach ($this->_context_stack as $ctx) {{
        $ctx->_transitioned = true;
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

    fn emit_router(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: __router is the single dispatch primitive.
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"$handler_name = "_state_" . $this->__compartment->state;
if (method_exists($this, $handler_name)) {
    $this->$handler_name($__e, $this->__compartment);
}"#
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
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next_compartment")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "$this->__next_compartment = $next_compartment;".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
