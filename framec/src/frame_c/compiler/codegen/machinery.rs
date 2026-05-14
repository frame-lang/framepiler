//! Per-system runtime-scaffold codegen (Strategy refactor).
//!
//! ## What this module is
//!
//! Every backend emits the same 8-node per-system runtime scaffold:
//!
//! | # | Node                       | Role                                              |
//! |---|----------------------------|---------------------------------------------------|
//! | 1 | `_HSM_CHAIN` table         | static topology: leaf -> root-to-leaf chain       |
//! | 2 | `__prepareEnter(...)`      | builds destination compartment chain on transition |
//! | 3 | `__prepareExit(...)`       | populates `exit_args` on the source chain         |
//! | 4 | `__route_to_state(...)`    | dispatches an event to a specific state's handler |
//! | 5 | `__process_transition_loop()` | drains pending transitions (RFC-0019 leaf dispatch) |
//! | 6 | `__kernel(...)`            | router + drain                                    |
//! | 7 | `__router(...)`            | delegates to current compartment's dispatcher     |
//! | 8 | `__transition(...)`        | caches the next compartment                       |
//!
//! Before this refactor, the same 8-node scaffold was emitted by **14
//! near-identical `generate_<lang>_machinery` functions** in
//! `system_codegen.rs` (~2,600 LOC of structural copy-paste). Every
//! kernel-level change (RFC-0017, RFC-0019, etc.) had to be hand-applied
//! 14 times — the root cause of the Arc-A "ALL 17 BACKENDS COMPLETE
//! with 12 broken" failure, per the 4.2 plan §7.1.
//!
//! ## What this module is *not* (yet)
//!
//! Stage 1 (this file) collapses the dispatch layer behind a trait but
//! keeps the per-backend method bodies as string templates inside each
//! `impl`. The byte-for-byte output of every backend is unchanged
//! relative to the previous `generate_<lang>_machinery` fn — the matrix
//! gate verifies this.
//!
//! Stage 2 (planned) extracts shared method bodies behind syntax
//! primitives (`Dialect` trait — null literal, field access, loop form,
//! etc.). That's where the real LOC reduction comes from; it lives in a
//! separate refactor on top of this one. See 4.2 plan §7.1.P2+.
//!
//! ## Adding a new backend
//!
//! 1. Create `framec/src/frame_c/compiler/codegen/machinery/<lang>.rs`.
//! 2. `impl MachineryGenerator for <Lang>Machinery` — fill in all 8
//!    methods. The compiler enforces completeness.
//! 3. Add the new variant to `dispatch_machinery()` in this file.
//! 4. Validate the matrix for the new backend.
//!
//! ## Out of scope: Erlang
//!
//! Erlang is **not** a `MachineryGenerator` consumer. The class-based
//! kernel scaffold (`__prepareEnter`, `__route_to_state`, etc.) doesn't
//! apply: `gen_statem` owns dispatch natively, and the per-system code
//! is emitted by the parallel `erlang_system.rs` codegen path. The
//! `system_codegen::generate_frame_machinery` dispatcher's
//! `TargetLanguage::Erlang` arm is intentionally a no-op.

use super::ast::CodegenNode;
use super::system_codegen::compute_hsm_chains;
use crate::frame_c::compiler::frame_ast::SystemAst;

/// Codegen for the per-system runtime scaffold. One impl per backend.
///
/// **Contract**: every method must return a single `CodegenNode`. The
/// driver `generate_machinery` calls each method exactly once, in the
/// order they appear in the trait definition, and emits the results as
/// a `Vec<CodegenNode>` representing the system's method table.
///
/// **Discipline**: the method bodies are still string-templated. That
/// is deliberate for Stage 1 — the goal here is to *organise* the
/// per-backend code so that an audit (cross-backend diff, RFC change
/// propagation, dead-code detection) is tractable. Stage 2 will share
/// method bodies via syntax primitives.
pub(crate) trait MachineryGenerator {
    /// Backend identifier — used for assertions and diagnostic output.
    fn lang_name(&self) -> &'static str;

    /// Node 1 — `_HSM_CHAIN` topology table (or language-equivalent).
    ///
    /// Most languages emit a static class field; PHP emits an instance
    /// method because PHP lacks static initializer syntax; Erlang has
    /// no machinery here (gen_statem owns the kernel).
    fn emit_hsm_chain(
        &self,
        system: &SystemAst,
        chains: &[(String, Vec<String>)],
    ) -> Option<CodegenNode>;

    /// Node 2 — `__prepareEnter(leaf, state_args, enter_args)`.
    ///
    /// Builds the destination compartment chain on a transition. Each
    /// compartment in the chain receives its own copy of state_args
    /// and enter_args; the signature-match rule (frame_validator) means
    /// every layer's signature is identical.
    fn emit_prepare_enter(
        &self,
        system: &SystemAst,
        compartment_class: &str,
    ) -> Option<CodegenNode>;

    /// Node 3 — `__prepareExit(exit_args)`.
    ///
    /// Populates `exit_args` on every compartment in the *source* chain
    /// before the kernel dispatches `<$` to the leaf. Walks
    /// `parent_compartment` from current up to the root.
    fn emit_prepare_exit(&self, system: &SystemAst) -> Option<CodegenNode>;

    /// Node 4 — `__route_to_state(state_name, __e, compartment)`.
    ///
    /// Dispatches an event to a specific state's handler with a
    /// specific compartment. Distinct from `__router`, which always
    /// uses the system's current compartment. Used by `=> $^`
    /// ancestor-forward lowering (RFC-0019): the leaf handler calls
    /// `__route_to_state(<parent>, __e, compartment.parent_compartment)`.
    fn emit_route_to_state(&self, system: &SystemAst) -> Option<CodegenNode>;

    /// Node 5 — `__process_transition_loop()`.
    ///
    /// Drains the `__next_compartment` queue. RFC-0019: only the leaf
    /// state's `<$` and `$>` fire automatically; ancestors fire only
    /// if the leaf forwards via `=> $^`.
    fn emit_process_transition_loop(
        &self,
        system: &SystemAst,
        event_class: &str,
    ) -> Option<CodegenNode>;

    /// Node 6 — `__kernel(__e)`.
    ///
    /// The main event loop. Routes the event to the current compartment's
    /// dispatcher, then drains any pending transitions.
    fn emit_kernel(&self, system: &SystemAst) -> Option<CodegenNode>;

    /// Node 7 — `__router(__e)`.
    ///
    /// Delegates to `__route_to_state(self.__compartment.state, __e,
    /// self.__compartment)`. Always uses the leaf compartment.
    fn emit_router(&self, system: &SystemAst) -> Option<CodegenNode>;

    /// Node 8 — `__transition(next_compartment)`.
    ///
    /// Caches the next compartment. The actual transition is deferred to
    /// `__process_transition_loop` so the current handler runs to
    /// completion first (the "queue, don't dispatch" rule).
    fn emit_transition(
        &self,
        system: &SystemAst,
        compartment_class: &str,
    ) -> Option<CodegenNode>;
}

/// Drive a `MachineryGenerator` to produce the per-system runtime
/// scaffold. Emission order is part of the contract (see above).
///
/// An impl may return `None` for a node if the language's runtime model
/// folds that node into the platform (e.g., Erlang's gen_statem is the
/// kernel; the `__kernel` / `__router` / `__transition` nodes don't
/// exist as separate generated methods).
pub(crate) fn generate_machinery<G: MachineryGenerator + ?Sized>(
    g: &G,
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let chains = compute_hsm_chains(system);
    [
        g.emit_hsm_chain(system, &chains),
        g.emit_prepare_enter(system, compartment_class),
        g.emit_prepare_exit(system),
        g.emit_route_to_state(system),
        g.emit_process_transition_loop(system, event_class),
        g.emit_kernel(system),
        g.emit_router(system),
        g.emit_transition(system, compartment_class),
    ]
    .into_iter()
    .flatten()
    .collect()
}

// --- per-backend impls live in submodules; each is small (~200 LOC) ---

pub(crate) mod gdscript;
pub(crate) mod java;
pub(crate) mod javascript;
pub(crate) mod python;
pub(crate) mod rust;
