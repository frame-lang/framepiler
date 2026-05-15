//! Frame Code Generation Pipeline
//! ==============================
//!
//! framec's codegen translates Frame AST → language-specific source for
//! 17 backends (Python, C, Rust, etc.). The pipeline is AST-based: a
//! language-agnostic [`CodegenNode`] tree is built per system, then
//! emitted by a per-language [`LanguageBackend`].
//!
//! ## High-level flow
//!
//! ```text
//!   Frame source
//!        │
//!        ▼
//!   parser/lexer  ─►  SystemAst + MachineAst + Arcanum (semantic enrichment)
//!        │
//!        ▼
//!   generate_system(system, arcanum, ...) in system_codegen.rs
//!        │
//!        ├─►  generate_*_compartment_types()  in runtime.rs
//!        │       (per-system FrameEvent / FrameContext / Compartment classes)
//!        │
//!        ├─►  generate_*_machinery()         via MachineryGenerator trait
//!        │       (kernel / router / transition queue / HSM helpers)
//!        │       Per-backend impls live in machinery/<lang>.rs
//!        │
//!        ├─►  generate_per_handler_methods() in state_dispatch.rs
//!        │       (per-state dispatchers + named handler methods)
//!        │       Per-backend impls in state_dispatch/handler_methods/<lang>.rs
//!        │       Handler bodies are walked by frame_expansion (see below)
//!        │
//!        ├─►  generate_interface_methods()   in interface_gen.rs
//!        │       (public interface wrappers + @@[persist] save/load)
//!        │       Persist impls per backend in interface_gen/persist/<lang>.rs
//!        │
//!        ├─►  Constructor IR node            in system_codegen/constructor.rs
//!        │       (bare ctor + factory for @@!Foo() / @@Foo(args) — RFC-0017)
//!        │
//!        ▼
//!   CodegenNode tree
//!        │
//!        ▼
//!   backends/<lang>.rs emit()  →  source text
//!        │
//!        ▼
//!   Language source file
//! ```
//!
//! Erlang follows a parallel path through `erlang_system.rs` because
//! the gen_statem callback model doesn't fit the class-based primitive
//! set. The Rust kernel has Rust-specific helpers in `rust_system.rs`.
//!
//! ## Module map
//!
//! ### Top-level files (orchestration + shared infrastructure)
//!
//! - **`mod.rs`** — this file; module declarations + re-exports.
//! - **`ast.rs`** — [`CodegenNode`] enum, the language-agnostic IR.
//!   Every codegen path builds a tree of these nodes; per-backend
//!   `LanguageBackend::emit` walks the tree to produce source text.
//! - **`backend.rs`** — the [`LanguageBackend`] trait plus
//!   [`EmitContext`] (indent state, system name, etc.) and
//!   [`get_backend`] (dispatch by `TargetLanguage`).
//! - **`system_codegen.rs`** — top-level entry point
//!   `generate_system`. Orchestrates the per-system pipeline:
//!   per-system support types, machinery, state dispatchers,
//!   interface wrappers, constructor IR.
//! - **`machinery.rs`** — [`MachineryGenerator`] trait. Defines the
//!   runtime primitive contract every backend implements
//!   (`emit_kernel`, `emit_router`, `emit_transition`,
//!   `emit_hsm_chain`, `emit_prepare_enter`, `emit_prepare_exit`).
//!   See [RFC-0020](docs/rfcs/rfc-0020.md) for the canonical kernel
//!   contract.
//! - **`state_dispatch.rs`** — per-state dispatcher generation +
//!   shared helpers for the per-handler architecture. Drives
//!   per-language emitters in `state_dispatch/handler_methods/`.
//! - **`frame_expansion.rs`** — the `@@`-syntax expander. When a
//!   handler body contains Frame constructs (`@@:return`,
//!   `@@:self.X()`, `-> $State`, `=> $^`, etc.), this module walks
//!   the body, identifies regions, and dispatches to per-construct
//!   expanders in `frame_expansion/`.
//! - **`interface_gen.rs`** — interface method wrapper generation
//!   plus `@@[persist]` save/load codegen. Per-backend persist impls
//!   in `interface_gen/persist/`.
//! - **`runtime.rs`** — per-system runtime support types (FrameEvent,
//!   FrameContext, Compartment classes), one set per backend.
//! - **`erlang_system.rs`** — full gen_statem-based Erlang generator;
//!   bypasses the class-based pipeline.
//! - **`rust_system.rs`** — Rust-specific helpers (kernel signature,
//!   borrow-checker workarounds).
//! - **`codegen_utils.rs`** — shared utilities ([`HandlerContext`],
//!   type-string mappers, expression-to-string helpers).
//! - **`block_transform.rs`** — post-pass block-level rewriters
//!   (async-await injection, etc.).
//! - **`output_block_*.gen.rs`** — Frame-defined output-block parsers
//!   (generated; do not edit by hand).
//!
//! ### Subdirectories
//!
//! - **`backends/<lang>.rs`** — one file per backend. Implements
//!   `LanguageBackend::emit` to turn [`CodegenNode`] trees into
//!   language source. Owns the Constructor IR arm that lays out
//!   `_init` / `_create` / factory shapes.
//! - **`machinery/<lang>.rs`** — one per backend. Implements
//!   [`MachineryGenerator`] for the runtime primitives.
//! - **`state_dispatch/handler_methods/<lang>.rs`** — per-backend
//!   handler-method emitter. Drives the per-handler body
//!   (param binding, state-var init, return-init, user body
//!   expansion via [`frame_expansion::emit_handler_body_via_statements`]).
//! - **`frame_expansion/*.rs`** — one file per `@@` / `=>` / `->`
//!   construct (`return.rs`, `self_call.rs`, `forward.rs`,
//!   `transition.rs`, `stack.rs`, etc.). Each defines the per-language
//!   lowering for that construct.
//! - **`interface_gen/persist/<lang>.rs`** — per-backend `@@[persist]`
//!   save/load codegen. The persist contract is documented in
//!   [RFC-0012](docs/rfcs/rfc-0012.md).
//! - **`system_codegen/constructor.rs`** — the per-backend
//!   match-arm that emits the init-event-block (the start `$>`
//!   dispatch inside the factory body). See [RFC-0020](docs/rfcs/rfc-0020.md).
//! - **`erlang_system/*.rs`** — gen_statem-specific helpers (state
//!   functions, persist, runtime helpers).
//!
//! ## Per-backend extension points
//!
//! Adding a new backend means implementing four things in lockstep:
//!
//! 1. **`backends/<lang>.rs`** — `LanguageBackend::emit` for every
//!    [`CodegenNode`] variant. Handles syntactic shape: brace vs.
//!    indent, keyword names, type spellings, etc.
//! 2. **`machinery/<lang>.rs`** — `MachineryGenerator` for the runtime
//!    primitives. Per-RFC-0020 the contract is `__kernel(event)` +
//!    `__router(event)` only; `__route_to_state` and
//!    `__process_transition_loop` are not emitted.
//! 3. **`state_dispatch/handler_methods/<lang>.rs`** — handler-method
//!    body emitter. Param binding, state-var init guards,
//!    return-init, and delegating to `emit_handler_body_via_statements`
//!    for user code.
//! 4. **`frame_expansion/<construct>.rs`** match arms — extend each
//!    construct's match to emit the new language's lowering. Touch
//!    every file under `frame_expansion/`.
//!
//! Plus per-language per-construct arms in `dispatch_syntax.rs`
//! (`fmt_if`, `fmt_forward`, `fmt_init_sv`, etc.) and shared
//! type-mapping helpers in `codegen_utils.rs`.
//!
//! ## Reference RFCs (load-bearing for the pipeline)
//!
//! - **[RFC-0012](docs/rfcs/rfc-0012.md)** — `@@[persist]` save/load.
//! - **[RFC-0013](docs/rfcs/rfc-0013.md)** — `@@[target(...)]` and
//!   the attribute-position grammar.
//! - **[RFC-0015](docs/rfcs/rfc-0015.md)** — `@@[create(name)]`
//!   factory rename + factory-only construction.
//! - **[RFC-0017](docs/rfcs/rfc-0017.md)** — bare-ctor + factory
//!   split for `@@!Foo()` (no-initialization construction).
//! - **[RFC-0019](docs/rfcs/rfc-0019.md)** — leaf-dispatch model:
//!   `$>` / `<$` are ordinary events; HSM child states that declare
//!   state-vars must emit an explicit `$>() { => $^ }` to cascade
//!   the enter event up to the parent's state-var initializer.
//!   **This is a fixture-author requirement, not auto-synthesized
//!   by framec.** See the per-state synthesis at
//!   `state_dispatch.rs::generate_per_handler_methods` lines 792-833.
//! - **[RFC-0020](docs/rfcs/rfc-0020.md)** — runtime reference
//!   architecture. Canonical kernel contract for all 17 backends.

pub mod ast;
pub mod backend;
pub mod backends;
pub mod block_transform;
pub mod codegen_utils;
pub mod erlang_system;
pub mod frame_expansion;
pub mod interface_gen;
pub mod machinery;
pub mod runtime;
pub mod rust_system;
pub mod state_dispatch;
pub mod system_codegen;

pub use ast::CodegenNode;
pub use backend::{get_backend, ClassSyntax, EmitContext, LanguageBackend};
pub use runtime::{
    generate_c_compartment_types, generate_compartment_class, generate_cpp_compartment_types,
    generate_csharp_compartment_types, generate_frame_context_class, generate_frame_event_class,
    generate_go_compartment_types, generate_java_compartment_types,
    generate_kotlin_compartment_types, generate_rust_compartment_types,
    generate_swift_compartment_types,
};
pub use system_codegen::generate_system;
