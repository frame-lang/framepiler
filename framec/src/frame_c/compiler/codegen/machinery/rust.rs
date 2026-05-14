//! Rust machinery generator.
//!
//! Moved verbatim from `rust_system::generate_rust_machinery` (4.2 plan
//! §7.1.P2 — typed-ownership canary). Body strings preserved exactly so
//! the matrix gate proves byte-for-byte equivalence with the pre-
//! refactor codegen.
//!
//! Rust differs structurally from the dynamic-typed backends:
//!
//! - **`__hsm_chain` is a method** (with a `match leaf { ... }` body
//!   returning `&'static [&'static str]`), not a static class attribute.
//!   Rust doesn't have a class-level static-dict ergonomic — a method
//!   with a match is the natural form.
//! - **Typed signatures.** `__prepareEnter` takes `Vec<String>` enter
//!   args, `__route_to_state` takes `&str` state name and `&Event`
//!   reference.
//! - **`__kernel` takes no params.** It pulls the current event off the
//!   context stack (`_context_stack.last().unwrap().event.clone()`)
//!   instead of receiving it as an argument — Rust's borrow checker
//!   makes the explicit-arg form awkward here.
//! - **`__router` body is a `match` directly**, not a delegate to
//!   `__route_to_state`. They use the same match table but the helper
//!   indirection isn't necessary on Rust.
//!
//! These differences are exactly what the trait abstracts: each backend
//! does what's right for the language; the trait only enforces "you
//! emit these 8 nodes in this order."

use crate::frame_c::compiler::codegen::ast::{CodegenNode, Param, Visibility};
use crate::frame_c::compiler::codegen::machinery::MachineryGenerator;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(crate) struct RustMachinery;

impl MachineryGenerator for RustMachinery {
    fn lang_name(&self) -> &'static str {
        "rust"
    }

    fn emit_hsm_chain(
        &self,
        _system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode> {
        // __hsm_chain — match on leaf, returns the chain as a static slice.
        let mut chain_body = String::from("match leaf {\n");
        for (leaf, chain) in chains {
            let entries = chain
                .iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(", ");
            chain_body.push_str(&format!("    \"{}\" => &[{}],\n", leaf, entries));
        }
        chain_body.push_str("    _ => &[],\n}");
        Some(CodegenNode::Method {
            name: "__hsm_chain".to_string(),
            params: vec![Param::new("leaf").with_type("&str")],
            return_type: Some("&'static [&'static str]".to_string()),
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
        _system: &SystemAst,
        compartment_class: &str,
    ) -> Option<CodegenNode> {
        // __prepareEnter — build the destination HSM chain leaf-down. The
        // `enter_args` Vec is cloned into every layer's `enter_args` field
        // so the cascade's per-layer synthesized `$>` events all carry
        // the same payload (per the signature-match rule in
        // docs/frame_runtime.md § "How propagation works in the runtime").
        Some(CodegenNode::Method {
            name: "__prepareEnter".to_string(),
            params: vec![
                Param::new("leaf").with_type("&str"),
                Param::new("enter_args").with_type("Vec<String>"),
            ],
            return_type: Some(compartment_class.to_string()),
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"let chain = self.__hsm_chain(leaf);
let mut comp: Option<{0}> = None;
for name in chain.iter() {{
    let mut new_comp = {0}::new(name);
    new_comp.enter_args = enter_args.clone();
    if let Some(parent) = comp.take() {{
        new_comp.parent_compartment = Some(Box::new(parent));
    }}
    comp = Some(new_comp);
}}
comp.expect("chain must contain at least the leaf state")"#,
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
        // __prepareExit — populate exit_args on every layer of current chain.
        Some(CodegenNode::Method {
            name: "__prepareExit".to_string(),
            params: vec![Param::new("exit_args").with_type("Vec<String>")],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"self.__compartment.exit_args = exit_args.clone();
let mut cursor = self.__compartment.parent_compartment.as_deref_mut();
while let Some(c) = cursor {
    c.exit_args = exit_args.clone();
    cursor = c.parent_compartment.as_deref_mut();
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

    fn emit_route_to_state(&self, system: &SystemAst) -> Option<CodegenNode> {
        // __route_to_state — cascade router. Same dispatch table as
        // __router but takes an explicit state name.
        let event_class = format!("{}FrameEvent", system.name);
        let mut route_code = String::from("match state_name {\n");
        if let Some(ref machine) = system.machine {
            for state in &machine.states {
                route_code.push_str(&format!(
                    "    \"{}\" => self._state_{}(__e),\n",
                    state.name, state.name
                ));
            }
        }
        route_code.push_str("    _ => {}\n}");
        Some(CodegenNode::Method {
            name: "__route_to_state".to_string(),
            params: vec![
                Param::new("state_name").with_type("&str"),
                Param::new("__e").with_type(&format!("&{}", event_class)),
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
        // RFC-0019: no enter/exit cascade. `$>` / `<$` are ordinary
        // events, dispatched to the current (leaf) state via
        // __route_to_state; an ancestor's `$>` / `<$` runs only if the
        // leaf forwards (=> $^).
        //
        // __process_transition_loop — drains queued transitions. On
        // each: dispatch `<$` to the current (leaf) state, switch
        // compartments, dispatch `$>` to the new (leaf) state, then
        // re-check.
        Some(CodegenNode::Method {
            name: "__process_transition_loop".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"while self.__next_compartment.is_some() {{
    let next_compartment = self.__next_compartment.take().unwrap();
    // Exit the current (leaf) state
    let exit_state = self.__compartment.state.clone();
    let exit_args = self.__compartment.exit_args.clone();
    let exit_event = {evt}::new_with_params("<$", &exit_args);
    self.__route_to_state(&exit_state, &exit_event);
    // Switch to the new compartment
    self.__compartment = next_compartment;
    // Enter the new (leaf) state — or the forwarded event
    if self.__compartment.forward_event.is_none() {{
        let enter_state = self.__compartment.state.clone();
        let enter_args = self.__compartment.enter_args.clone();
        let enter_event = {evt}::new_with_params("$>", &enter_args);
        self.__route_to_state(&enter_state, &enter_event);
    }} else {{
        let forward_event = self.__compartment.forward_event.take().unwrap();
        let enter_state = self.__compartment.state.clone();
        let enter_args = self.__compartment.enter_args.clone();
        let enter_event = {evt}::new_with_params("$>", &enter_args);
        self.__route_to_state(&enter_state, &enter_event);
        if forward_event.message != "$>" {{
            self.__router(&forward_event);
        }}
    }}
    for ctx in self._context_stack.iter_mut() {{
        ctx._transitioned = true;
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

    fn emit_kernel(&self, _system: &SystemAst) -> Option<CodegenNode> {
        // __kernel — thin: route then drain. Takes no param; pulls the
        // current event off the context stack (Rust's borrow checker
        // makes the explicit-arg form awkward here).
        Some(CodegenNode::Method {
            name: "__kernel".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: r#"let __e = self._context_stack.last().unwrap().event.clone();
self.__router(&__e);
self.__process_transition_loop();"#
                    .to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }

    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode> {
        // __router — dispatches events to current state method via
        // match. Same dispatch table as __route_to_state but reads
        // self.__compartment.state instead of taking an explicit name.
        let event_class = format!("{}FrameEvent", system.name);
        let router_code = generate_rust_router_dispatch(system);
        Some(CodegenNode::Method {
            name: "__router".to_string(),
            params: vec![Param::new("__e").with_type(&format!("&{}", event_class))],
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
        // __transition — caches next compartment (deferred).
        Some(CodegenNode::Method {
            name: "__transition".to_string(),
            params: vec![Param::new("next_compartment").with_type(compartment_class)],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "self.__next_compartment = Some(next_compartment);".to_string(),
                span: None,
            }],
            is_async: false,
            is_static: false,
            visibility: Visibility::Private,
            decorators: vec![],
        })
    }
}

/// Generate Rust router dispatch — match on compartment state name.
/// Moved here from `rust_system.rs` (was a sibling private helper).
fn generate_rust_router_dispatch(system: &SystemAst) -> String {
    let mut code = String::new();
    code.push_str("match self.__compartment.state.as_str() {\n");

    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            code.push_str(&format!(
                "    \"{}\" => self._state_{}(__e),\n",
                state.name, state.name
            ));
        }
    }

    code.push_str("    _ => {}\n");
    code.push_str("}");
    code
}
