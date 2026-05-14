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

    fn emit_route_to_state(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        let compartment_class = format!("{}Compartment", system.name);
        let states: Vec<&str> = system
            .machine
            .as_ref()
            .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default();
        let mut route_code = String::new();
        for (i, state) in states.iter().enumerate() {
            let prefix = if i == 0 { "if" } else { "} else if" };
            route_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
            route_code.push_str(&format!("    _state_{}(__e, compartment)\n", state));
        }
        if !states.is_empty() {
            route_code.push_str("}");
        }
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name").with_type("String"),
                Param::new("__e").with_type(&event_class),
                Param::new("compartment").with_type(&compartment_class),
            ],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: route_code,
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
        // RFC-0019: no enter/exit cascade. `$>` / `<$` are ordinary events,
        // dispatched to the current (leaf) state via __route_to_state.
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while (__next_compartment != null) {{
    val next_compartment = __next_compartment!!
    __next_compartment = null
    val exit_event = {evt}("<$", __compartment.exit_args)
    __route_to_state(__compartment.state, exit_event, __compartment)
    __compartment = next_compartment
    if (next_compartment.forward_event == null) {{
        val enter_event = {evt}("$>", __compartment.enter_args)
        __route_to_state(__compartment.state, enter_event, __compartment)
    }} else {{
        val forward_event = next_compartment.forward_event!!
        next_compartment.forward_event = null
        val enter_event = {evt}("$>", __compartment.enter_args)
        __route_to_state(__compartment.state, enter_event, __compartment)
        if (forward_event._message != "$>") {{
            __router(forward_event)
        }}
    }}
    for (ctx in _context_stack) {{
        ctx._transitioned = true
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

    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__router(__e)\n__process_transition_loop()".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        let event_class = format!("{}FrameEvent", system.name);
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&event_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "__route_to_state(__compartment.state, __e, __compartment)".to_string(),
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
