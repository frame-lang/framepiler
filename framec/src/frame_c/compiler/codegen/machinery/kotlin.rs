//! Kotlin machinery generator.
//!
//! Moved verbatim from `system_codegen::generate_kotlin_machinery` (4.2
//! plan §7.1.P3). Kotlin's emission is closest to Java's but with two
//! differences worth flagging:
//!
//! - Kotlin's `@JvmStatic fun __create(...)` factory is emitted **here**
//!   as a prelude NativeBlock (not from `backends/kotlin.rs`'s
//!   Constructor arm, the way Java / Swift / C# do). Kotlin requires
//!   `__create` to live inside a `companion object`, and the
//!   class-body emitter in `backends/kotlin.rs` partitions companion
//!   members by the heuristic "NativeBlock whose text opens with
//!   `@JvmStatic`". Emitting `__create` as a sibling NativeBlock keeps
//!   that partitioning intact; emitting it inline from Constructor
//!   codegen would land it outside the companion object.
//! - `__route_to_state` uses a cascading `if (state_name == "...")`
//!   chain (Kotlin's `when (state_name)` would also work but the legacy
//!   fn used if-chain — preserved verbatim for byte-canonical parity).

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::codegen_utils::{kotlin_map_type, type_to_string};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct KotlinMachinery;

impl MachineryGenerator for KotlinMachinery {
    fn lang_name(&self) -> &'static str {
        "kotlin"
    }

    fn emit_prelude(&self, system: &SystemAst) -> Vec<CodegenNode> {
        // RFC-0017 Phase A1: companion-object factory for `@@Counter(args)`.
        // Constructs a bare instance via the primary ctor (framework only)
        // then invokes `__frame_init(args)` to run the user `$>` + cascade.
        // Two-step decoupling replaces the old `__skipInitialEnter` flag +
        // `__no_init` factory + `kotlin_type_default_expr` dance from D7.
        let create_params: Vec<String> = system
            .params
            .iter()
            .map(|p| {
                let ty = type_to_string(&p.param_type);
                format!("{}: {}", p.name, kotlin_map_type(&ty))
            })
            .collect();
        let arg_pass: Vec<String> = system.params.iter().map(|p| p.name.clone()).collect();
        let create_body = format!(
            "@JvmStatic fun __create({params}): {sys} {{\n    val c = {sys}()\n    c.__frame_init({args})\n    return c\n}}",
            sys = system.name,
            params = create_params.join(", "),
            args = arg_pass.join(", "),
        );
        vec![CodegenNode::NativeBlock {
            code: create_body,
            span: None,
        }]
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // hsm_chain — instance method returning the topology table.
        let mut chain_method = String::from(
            "private fun hsm_chain(): Map<String, List<String>> {\n    return mapOf(\n",
        );
        for (leaf, chain) in chains {
            let chain_str = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_method.push_str(&format!("        \"{}\" to listOf({}),\n", leaf, chain_str));
        }
        chain_method.push_str("    )\n}");
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
                Param::new("leaf").with_type("String"),
                Param::new("state_args").with_type("MutableList<Any?>"),
                Param::new("enter_args").with_type("MutableList<Any?>"),
            ],
            return_type: Some(compartment_class.to_string()),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"var comp: {0}? = null
for (name in hsm_chain()[leaf]!!) {{
    val new_comp = {0}(name)
    new_comp.state_args.addAll(state_args)
    new_comp.enter_args.addAll(enter_args)
    new_comp.parent_compartment = comp
    comp = new_comp
}}
return comp!!"#,
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

    fn emit_prepare_exit(&self, system: &SystemAst) -> Option<CodegenNode> {
        let compartment_class = format!("{}Compartment", system.name);
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type("MutableList<Any?>")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"var comp: {}? = __compartment
while (comp != null) {{
    comp.exit_args.clear()
    comp.exit_args.addAll(exit_args)
    comp = comp.parent_compartment
}}"#,
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

    fn emit_route_to_state(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // RFC-0020: __router holds the dispatch table directly;
        // no separate __route_to_state helper.
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
        // RFC-0020: __kernel dispatches one event then drains any
        // transitions queued by the handler. Three-branch forward-
        // event protocol matches the Python reference.
        //
        // Kotlin specifics:
        // - `__next_compartment` is `var ... : Compartment? = null` —
        //   smart-cast through a `var` member is unsafe; capture into
        //   a local `val` via `!!` after the null check.
        // - `==` is value-equality (calls `equals()`); no `.equals()`
        //   needed for the `forward_event._message == "$>"` compare.
        // - Iterating `for (ctx in _context_stack)` mutates by ref
        //   since `FrameContext` is a class.
        let event_class = format!("{}FrameEvent", system.name);
        let compartment_class = format!("{}Compartment", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"// Route event to current state.
__router(__e)
// Drain any transitions queued by the handler.
while (__next_compartment != null) {{
    val next_compartment: {comp} = __next_compartment!!
    __next_compartment = null
    // Exit the current (leaf) state.
    val exit_event = {evt}("<$", __compartment.exit_args)
    __router(exit_event)
    // Switch to the new compartment.
    __compartment = next_compartment
    // Three-branch forward-event handling.
    val forward_event: {evt}? = next_compartment.forward_event
    next_compartment.forward_event = null
    if (forward_event == null) {{
        // No forwarded event — synthesize a fresh $>.
        val enter_event = {evt}("$>", __compartment.enter_args)
        __router(enter_event)
    }} else if (forward_event._message == "$>") {{
        // Forwarded event IS $> — dispatch directly so the
        // destination's $> handler receives the caller's payload.
        __router(forward_event)
    }} else {{
        // Forwarded event is not $> — initialize the destination
        // with a fresh $>, then dispatch the forward.
        val enter_event = {evt}("$>", __compartment.enter_args)
        __router(enter_event)
        __router(forward_event)
    }}
    for (ctx in _context_stack) {{
        ctx._transitioned = true
    }}
}}"#,
                    comp = compartment_class,
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
        // __compartment.state at call time and routes to the matching
        // state dispatcher inline (no __route_to_state indirection).
        let event_class = format!("{}FrameEvent", system.name);
        let states: Vec<&str> = system
            .machine
            .as_ref()
            .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default();
        let mut router_code = String::new();
        for (i, state) in states.iter().enumerate() {
            let prefix = if i == 0 { "if" } else { "} else if" };
            router_code.push_str(&format!(
                "{} (__compartment.state == \"{}\") {{\n",
                prefix, state
            ));
            router_code.push_str(&format!("    _state_{}(__e, __compartment)\n", state));
        }
        if !states.is_empty() {
            router_code.push_str("}");
        }
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: router_code,
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
            params: vec![Param::new("next").with_type(compartment_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__next_compartment = next".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}
