//! Lua machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_lua_machinery` (4.2
//! plan §7.1.P3). Lua's quirks vs. the dynamic-dispatch family:
//!
//! - `hsm_chain` is built via sequential `t[...] = {...}` assignments
//!   rather than a multi-line literal, because the Lua block transformer
//!   rewrites multi-line `{ ... }` as a Frame block.
//! - `__prepareEnter` accepts `nil` for state/enter args and substitutes
//!   `{}` internally — emitting `{}` at the transition site would also
//!   trip the block transformer.
//! - Method dispatch is `self:method()` (colon syntax) and field access
//!   is `self.field` (dot syntax); handlers are looked up via
//!   `self["_state_" .. state_name]` and invoked with `handler(self, ...)`.

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct LuaMachinery;

impl MachineryGenerator for LuaMachinery {
    fn lang_name(&self) -> &'static str {
        "lua"
    }

    fn emit_hsm_chain(
        &self,
        system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // hsm_chain — class method returning the topology table.
        // Built via sequential assignments rather than a literal table
        // expression: the Lua block transformer treats `{ … }` on
        // multiple lines as a Frame block (matching `if`/`while`
        // bodies) and rewrites it incorrectly. Sequential assignments
        // avoid the multi-line literal entirely.
        let mut chain_method = String::from("function ");
        chain_method.push_str(&system.name);
        chain_method.push_str(":hsm_chain()\n    local t = {}\n");
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("    t[\"{}\"] = {{{}}}\n", leaf, chain_str));
        }
        chain_method.push_str("    return t\nend");
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
        // __prepareEnter — constructs the destination HSM chain. Accepts
        // nil for empty args lists so transition sites can call
        // `self:__prepareEnter("X", nil, nil)` without emitting `{}`
        // literals (the Lua block transformer mishandles `{}` inside
        // if/else bodies — see transition emission notes).
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
                    r#"state_args = state_args or {{}}
enter_args = enter_args or {{}}
local comp = nil
local chain = self:hsm_chain()[leaf]
for i = 1, #chain do
    local new_comp = {}.new(chain[i])
    new_comp.state_args = {{}}
    for j = 1, #state_args do new_comp.state_args[j] = state_args[j] end
    new_comp.enter_args = {{}}
    for j = 1, #enter_args do new_comp.enter_args[j] = enter_args[j] end
    new_comp.parent_compartment = comp
    comp = new_comp
end
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
                code: r#"local comp = self.__compartment
while comp ~= nil do
    comp.exit_args = {}
    for j = 1, #exit_args do comp.exit_args[j] = exit_args[j] end
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
                    r#"-- Route event to current state.
self:__router(__e)
-- Drain any transitions queued by the handler.
while self.__next_compartment ~= nil do
    local next_compartment = self.__next_compartment
    self.__next_compartment = nil
    local exit_event = {ec}.new("<$", self.__compartment.exit_args)
    self:__router(exit_event)
    self.__compartment = next_compartment
    local forward_event = next_compartment.forward_event
    next_compartment.forward_event = nil
    if forward_event == nil then
        local enter_event = {ec}.new("$>", self.__compartment.enter_args)
        self:__router(enter_event)
    elseif forward_event._message == "$>" then
        self:__router(forward_event)
    else
        local enter_event = {ec}.new("$>", self.__compartment.enter_args)
        self:__router(enter_event)
        self:__router(forward_event)
    end
    for _, ctx in ipairs(self._context_stack) do
        ctx._transitioned = true
    end
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

    fn emit_router(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: __router is the single dispatch primitive.
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"local handler = self["_state_" .. self.__compartment.state]
if handler then
    handler(self, __e, self.__compartment)
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

    fn emit_transition(
        &self,
        _system: &SystemAst,
        _compartment_class: &str,
    ) -> Option<CodegenNode> {
        // __transition method — caches next compartment (deferred)
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
