//! Erlang gen_statem code generation.
//!
//! This module generates complete Erlang/OTP gen_statem modules from Frame systems.
//! It bypasses the standard class-based CodegenNode pipeline entirely, producing
//! raw Erlang source text with proper gen_statem callbacks, -record(data, {}),
//! and Frame infrastructure (frame_transition__, frame_dispatch__, etc.).

mod actions_ops;
mod blocks;
mod body_processor;
mod case_arms;
mod lexical;
mod native_rewrite;
mod persist;
mod runtime_helpers;
mod self_call_guards;

use blocks::{erlang_lower_native_if, erlang_smart_join, erlang_transform_blocks};
use body_processor::{
    erlang_capitalize_params, erlang_process_body_lines, erlang_process_body_lines_full,
    erlang_process_body_lines_with_params,
};
use case_arms::{
    analyze_case_arms, erlang_inject_orphan_reply_tuples, rewrite_mixed_case_arms,
    CaseBlockClassification,
};
use lexical::{
    erlang_op_name, erlang_safe_capitalize, expand_system_instantiation_in_domain_erlang,
    raw_contains_word, replace_whole_word, replace_word, split_top_level_commas,
};
use self_call_guards::erlang_wrap_self_call_guards;

use super::ast::CodegenNode;
use super::codegen_utils::{
    convert_expression, convert_literal, expression_to_string,
    replace_outside_strings_and_comments, to_snake_case, type_to_string, HandlerContext,
};
use super::frame_expansion::emit_handler_body_via_statements;
use crate::frame_c::compiler::arcanum::Arcanum;
use crate::frame_c::compiler::frame_ast::{Expression, Literal, SystemAst, Type};
use crate::frame_c::compiler::native_region_scanner::erlang::NativeRegionScannerErlang;
use crate::frame_c::visitors::TargetLanguage;

/// Canonical list of Erlang `Data` record fields that constitute the
/// "compartment context" — per-state positional args that survive
/// across handler boundaries and MUST be saved+restored together
/// by both `push$` / `pop$` (Phase 19 wave 3) and `@@persist`
/// (Phase 24 probe). The two sites historically maintained
/// independent hardcoded lists and both missed `frame_state_args`
/// + `frame_enter_args` until separate fuzz waves surfaced the
/// gap (framepiler `3f0cd24` and `b8144d1`).
///
/// **Adding a field here is the canonical step** when a new context
/// field is added to the Data record. Call sites that emit save/
/// restore code MUST iterate this list rather than hardcoding
/// individual field names.
///
/// Excluded:
/// - `frame_current_state` — gen_statem manages this directly
///   (saved as `state` in the persist map; saved as the head atom
///   in push/pop tuples).
/// - `frame_stack` — saved by persist, but as the modal stack
///   itself (NOT a per-stack-entry context), so it doesn't appear
///   in push/pop tuples.
/// - `frame_exit_args` / `frame_context_stack` / `frame_return_val`
///   — transient (set by transition or per-handler-invocation;
///   not preserved across handler boundaries).
pub(crate) const ERLANG_COMPARTMENT_CONTEXT_FIELDS: &[&str] =
    &["frame_state_args", "frame_enter_args"];

/// Simple rewrite for contexts where Data threading isn't needed (expressions only)
fn erlang_rewrite_expr(line: &str, action_names: &[String]) -> String {
    let l = line.trim();
    for action in action_names {
        let pattern = format!("self.{}(", action);
        if l.contains(&pattern) {
            let replaced = l.replace(&pattern, &format!("{}(Data, ", action));
            return replaced.replace("(Data, )", "(Data)");
        }
    }
    replace_outside_strings_and_comments(l, TargetLanguage::Erlang, &[("self.", "Data#data.")])
}

// ============================================================================

pub(crate) fn generate_erlang_system(
    system: &SystemAst,
    _arcanum: &Arcanum,
    source: &[u8],
) -> CodegenNode {
    let sys = &system.name;
    let module_name = to_snake_case(sys);
    let mut code = String::new();

    // Collect action + operation names for native code rewriting
    // (both are module-level functions that get self.X() → X(Data) rewriting)
    let mut action_names: Vec<String> = system.actions.iter().map(|a| a.name.clone()).collect();
    action_names.extend(system.operations.iter().map(|o| o.name.clone()));

    // Collect interface method names for internal dispatch
    // (self.method() → frame_dispatch__(method, [args], Data))
    let interface_names: Vec<String> = system.interface.iter().map(|m| m.name.clone()).collect();

    // Module header
    code.push_str(&format!("-module({}).\n", module_name));
    code.push_str("-behaviour(gen_statem).\n\n");

    // Collect state names
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();

    let first_state = states
        .first()
        .map(|s| to_snake_case(s))
        .unwrap_or_else(|| "init_state".to_string());

    // State name conversion: $MyState -> my_state
    let state_atom = |name: &str| -> String { to_snake_case(name) };

    // System params (header parameters): used to thread constructor
    // arguments through start_link/N → init/1 → the #data{} record
    // literal so domain fields can reference parameters by name and
    // state params land in frame_state_args.
    let sys_params = &system.params;
    let sys_param_arity = sys_params.len();
    let sys_param_vars: Vec<String> = sys_params
        .iter()
        .map(|p| erlang_safe_capitalize(&p.name))
        .collect();

    // Exports — API functions.
    //
    // RFC-0017 Phase A6: init-decouple replaces D7's:
    //   start_link/N  (full ctor, fired user $>)
    //   '__no_init'/0 (D7 no-init helper)
    // with the uniform three-method shape:
    //   start_link/0   — bare (framework only, no user $>)
    //   frame_init/(N+1) — synchronous cast that delivers user args + fires
    //                       cascade by transitioning to self
    //   create/N        — factory: start_link/0 + frame_init/N, returns Pid
    let mut api_exports = Vec::new();
    api_exports.push("start_link/0".to_string());
    api_exports.push(format!("frame_init/{}", sys_param_arity + 1));
    api_exports.push(format!("create/{}", sys_param_arity));
    // RFC-0015 phase 1.9: `@@[create(<name>)]` adds a renamed
    // factory that delegates to `create/N` with the same arity.
    if let Some(factory_name) = system.create_op_name() {
        api_exports.push(format!(
            "{}/{}",
            to_snake_case(factory_name),
            sys_param_arity
        ));
    }
    for method in &system.interface {
        let arity = method.params.len() + 1; // +1 for Pid
        api_exports.push(format!("{}/{}", to_snake_case(&method.name), arity));
    }
    // Non-static operations are callable externally (Pid-based) AND
    // internally (Data-based). Each non-static op emits a two-clause
    // function guarded by `is_pid/1`: if the first arg is a Pid the
    // external clause dispatches through gen_statem; otherwise the
    // internal clause runs the body. Same arity in both clauses, so
    // one export covers both. See the op-function emitter below.
    for op in &system.operations {
        if op.is_static {
            continue;
        }
        // RFC-0012 amendment: framework-managed ops are emitted by
        // the persistence block (with their own export). Skip here.
        if op
            .attributes
            .iter()
            .any(|a| a.name == "save" || a.name == "load")
        {
            continue;
        }
        let arity = op.params.len() + 1; // +1 for Pid (external) / Data (internal)
        api_exports.push(format!("{}/{}", erlang_op_name(&op.name), arity));
    }
    code.push_str(&format!("-export([{}]).\n", api_exports.join(", ")));

    // Exports — gen_statem callbacks
    code.push_str("-export([callback_mode/0, init/1]).\n");

    // Exports — state functions
    let state_exports: Vec<String> = states
        .iter()
        .map(|s| format!("{}/3", state_atom(s)))
        .collect();
    if !state_exports.is_empty() {
        code.push_str(&format!("-export([{}]).\n", state_exports.join(", ")));
    }

    // Record for domain variables + state variables
    code.push_str("\n-record(data, {\n");
    let mut all_fields: Vec<String> = Vec::new();

    // Helper: does this raw domain initializer reference any system param?
    // If so, the record default must be neutral (`undefined`) and the real
    // value is bound in init/N — record defaults can't see init/N's variables.
    let raw_references_param = |raw: &str| -> bool {
        for p in sys_params {
            if raw_contains_word(raw, &p.name) {
                return true;
            }
        }
        false
    };

    // Domain vars — emit Erlang record fields from the structured
    // (name, var_type, initializer_text) slots populated by the new
    // domain_native parser. Erlang ignores the var_type entirely
    // (record fields are dynamically typed in Erlang). The initializer
    // text becomes the record field default — except when it references
    // a system param, in which case we emit `undefined` and let init/N
    // populate the real value via the record literal (Erlang record
    // defaults are evaluated at compile time and can't see init/N's
    // variables).
    for var in &system.domain {
        let init_for_record = match &var.initializer_text {
            Some(init) if raw_references_param(init) => "undefined".to_string(),
            Some(init) => expand_system_instantiation_in_domain_erlang(init),
            None => "undefined".to_string(),
        };
        all_fields.push(format!("    {} = {}", var.name, init_for_record));
    }

    // State variables — prefixed with sv_StateName_ to avoid collisions
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            let state_prefix = to_snake_case(&state.name);
            for sv in &state.state_vars {
                let field_name = format!("sv_{}_{}", state_prefix, sv.name);
                let init_val = if let Some(ref init) = sv.init {
                    expression_to_string(init, TargetLanguage::Erlang)
                } else {
                    "undefined".to_string()
                };
                all_fields.push(format!("    {} = {}", field_name, init_val));
            }
        }
    }

    // Frame infrastructure (Path D hybrid)
    all_fields.push("    frame_stack = []".to_string());
    all_fields.push(format!("    frame_current_state = {}", first_state));
    all_fields.push("    frame_enter_args = []".to_string());
    all_fields.push("    frame_exit_args = []".to_string());
    all_fields.push("    frame_state_args = []".to_string());
    all_fields.push("    frame_context_stack = []".to_string());
    all_fields.push("    frame_return_val = undefined".to_string());
    // RFC-0015 D7: gates the `(enter, _OldState, Data)` user body for
    // `@@!Foo()` no-initialization allocation. The first enter clause
    // for every state checks this flag and clears it without running
    // the user's $> body. `init([no_init])` sets it to true; any
    // subsequent transition fires the cascade normally.
    all_fields.push("    frame_skip_enter__ = false".to_string());

    code.push_str(&all_fields.join(",\n"));
    code.push('\n');
    code.push_str("}).\n\n");

    // RFC-0017 Phase A6: bare `start_link/0` returns `{ok, Pid}` from
    // a process initialized with the `frame_skip_enter__` flag set —
    // user `$>` does NOT fire. This is the `@@!Counter()` form.
    code.push_str("start_link() ->\n    gen_statem:start_link(?MODULE, [], []).\n\n");

    // frame_init/(N+1) — synchronous call that delivers the user's
    // ctor args and triggers the `$>` cascade by transitioning the
    // gen_statem to itself with the skip flag cleared. The transition
    // re-fires `state_enter`; since the flag is now false, the
    // unguarded enter clause (user `$>` body) runs.
    let frame_init_args = sys_param_vars.join(", ");
    let frame_init_list = if sys_param_vars.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", sys_param_vars.join(", "))
    };
    let frame_init_params = if sys_param_vars.is_empty() {
        "Pid".to_string()
    } else {
        format!("Pid, {}", sys_param_vars.join(", "))
    };
    code.push_str(&format!(
        "frame_init({}) ->\n    gen_statem:call(Pid, {{frame_init, {}}}).\n\n",
        frame_init_params, frame_init_list
    ));

    // create/N — public factory: bare start_link + frame_init. Returns
    // a bare Pid (matches the previous `@@Counter(args)` lowering
    // shape: `element(2, counter:start_link(args))` is now
    // `counter:create(args)` which already returns a Pid).
    let create_args = sys_param_vars.join(", ");
    code.push_str(&format!(
        "create({}) ->\n    {{ok, Pid}} = start_link(),\n    frame_init({}),\n    Pid.\n\n",
        create_args, frame_init_params
    ));

    // RFC-0015 phase 1.9: factory rename — emit `<name>(Args) ->
    // create(Args).` if `@@[create(<name>)]` is set.
    if let Some(factory_name) = system.create_op_name() {
        code.push_str(&format!(
            "{}({}) ->\n    create({}).\n\n",
            to_snake_case(factory_name),
            create_args,
            sys_param_vars.join(", ")
        ));
    }

    // Interface functions — public API
    for method in &system.interface {
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| erlang_safe_capitalize(&p.name))
            .collect();
        let all_params = {
            let mut p = vec!["Pid".to_string()];
            p.extend(params.clone());
            p
        };
        let method_snake = to_snake_case(&method.name);
        let call_args = if params.is_empty() {
            method_snake.clone()
        } else {
            format!("{{{}, {}}}", method_snake, params.join(", "))
        };

        // Erlang is dynamic per docs/frame_runtime.md: the wrapper
        // returns whatever the state machine replied with, including
        // the atom `ok` from the catch-all clause when no handler
        // matched. Don't coerce — that would contradict the dynamic-
        // lang contract and break tests that expect the raw reply.
        code.push_str(&format!(
            "{}({}) ->\n    gen_statem:call(Pid, {}).\n\n",
            method_snake,
            all_params.join(", "),
            call_args
        ));
    }

    // callback_mode/0
    code.push_str("callback_mode() -> [state_functions, state_enter].\n\n");

    // init/1 — receive system params via the list passed to gen_statem,
    // bind them as Erlang variables, then build the #data{} record literal
    // overriding fields that reference params and populating frame_state_args
    // for any $(...) state params declared in the system header.
    let init_pattern = if sys_param_vars.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", sys_param_vars.join(", "))
    };
    let mut record_overrides: Vec<String> = Vec::new();
    // Domain field overrides for fields whose initializer references a
    // header param.
    for var in &system.domain {
        if let Some(init_expr) = &var.initializer_text {
            if raw_references_param(init_expr) {
                // Substitute bare param identifiers with their
                // capitalized Erlang variable names, then emit the
                // record override.
                let mut substituted = init_expr.clone();
                for p in sys_params {
                    let cap = erlang_safe_capitalize(&p.name);
                    substituted = replace_word(&substituted, &p.name, &cap);
                }
                record_overrides.push(format!("{} = {}", var.name, substituted));
            }
        }
    }
    // State-param overrides go into frame_state_args, and enter-param
    // overrides go into frame_enter_args. After the HashMap→List
    // migration both are positional Erlang lists, so we look up the
    // ordering from the start state's declared params (state args)
    // and start state's enter handler params (enter args), then emit
    // a list literal whose Nth element is the matching system param
    // variable or `undefined` for slots without an override.
    use crate::frame_c::compiler::frame_ast::ParamKind;
    let start_state_obj = system.machine.as_ref().and_then(|m| m.states.first());

    // Build the positional state_args list from the start state's
    // declared params (e.g. `$Start(x: int, y: str)` → 2 slots).
    if let Some(start) = start_state_obj {
        if !start.params.is_empty() {
            let entries: Vec<String> = start
                .params
                .iter()
                .map(|sp| {
                    sys_params
                        .iter()
                        .find(|p| matches!(p.kind, ParamKind::StateArg) && p.name == sp.name)
                        .map(|p| erlang_safe_capitalize(&p.name))
                        .unwrap_or_else(|| "undefined".to_string())
                })
                .collect();
            record_overrides.push(format!("frame_state_args = [{}]", entries.join(", ")));
        }
        // Build the positional enter_args list from the start state's
        // `$>` handler params, if it has one.
        if let Some(ref enter) = start.enter {
            if !enter.params.is_empty() {
                let entries: Vec<String> = enter
                    .params
                    .iter()
                    .map(|ep| {
                        sys_params
                            .iter()
                            .find(|p| matches!(p.kind, ParamKind::EnterArg) && p.name == ep.name)
                            .map(|p| erlang_safe_capitalize(&p.name))
                            .unwrap_or_else(|| "undefined".to_string())
                    })
                    .collect();
                record_overrides.push(format!("frame_enter_args = [{}]", entries.join(", ")));
            }
        }
    }
    let record_literal = if record_overrides.is_empty() {
        "#data{}".to_string()
    } else {
        format!("#data{{{}}}", record_overrides.join(", "))
    };
    // RFC-0017 Phase A6: bare `init/1` always sets `frame_skip_enter__
    // = true`. The user's `$>` body NEVER fires from init — it only
    // fires when `frame_init/(N+1)` is called explicitly. The D7
    // `init([no_init])` and the parameterized `init([Seed])` clauses
    // are both gone; `init([])` is the only path.
    let _ = (init_pattern, &record_literal);
    code.push_str(&format!(
        "init([]) ->\n    {{ok, {}, #data{{frame_skip_enter__ = true}}}}.\n\n",
        first_state
    ));

    // Stash the `record_overrides` so the start state's `frame_init`
    // handler can apply them at runtime. The frame_init handler is
    // emitted alongside the state functions below.
    let frame_init_record_overrides = record_overrides.clone();

    // State functions — one per state.
    // Build helpers for HSM cascade emission:
    //   * by_name: state lookup for parent-chain walks
    //   * ancestor_chain_top_down(name): the chain from root to (name, exclusive)
    //   * needs_enter_emission(state): true if the state has either an
    //     explicit `$>` handler or state-vars to auto-init.
    //
    // The cascade contract (docs/frame_runtime.md Step 21+): when a leaf
    // is entered, every ancestor's `$>` fires top-down (root first), then
    // the leaf's. We achieve this in gen_statem (which only fires `enter`
    // on the leaf) by having the leaf's `enter` clause walk the chain
    // and call each ancestor's `frame_enter__<state>` helper before
    // running the leaf's own body.
    if let Some(ref machine) = system.machine {
        let by_name: std::collections::HashMap<&str, &_> = machine
            .states
            .iter()
            .map(|s| (s.name.as_str(), s))
            .collect();
        let ancestor_chain_top_down = |leaf: &str| -> Vec<String> {
            // Build leaf-first then reverse so the result is root → parent.
            // Excludes the leaf itself.
            let mut chain = Vec::new();
            let mut cur = by_name
                .get(leaf)
                .and_then(|s| s.parent.as_deref().map(|p| p.to_string()));
            while let Some(name) = cur {
                chain.push(name.clone());
                cur = by_name
                    .get(name.as_str())
                    .and_then(|s| s.parent.as_deref().map(|p| p.to_string()));
            }
            chain.reverse();
            chain
        };
        let needs_enter_emission = |state: &crate::frame_c::compiler::frame_ast::StateAst| {
            state.enter.is_some() || !state.state_vars.is_empty()
        };

        for (state_idx, state) in machine.states.iter().enumerate() {
            let state_name = state_atom(&state.name);
            let is_start_state = state_idx == 0;

            // RFC-0017 Phase A6: skip-flag guard for `start_link/0`.
            // Bare init sets `frame_skip_enter__ = true` so the first
            // state's enter callback bails without firing the user's
            // `$>` body. Clearing the flag in this clause keeps
            // subsequent transitions firing the cascade normally.
            // This clause must precede the unguarded enter clause so
            // Erlang's pattern matching tries it first.
            code.push_str(&format!(
                "{}(enter, _OldState, #data{{frame_skip_enter__ = true}} = Data) ->\n    {{keep_state, Data#data{{frame_skip_enter__ = false}}}};\n",
                state_name
            ));

            // RFC-0017 Phase A6: `frame_init/(N+1)` cast handler on
            // the START state. Receives the user's ctor args, applies
            // domain/state/enter overrides to the Data record, clears
            // the skip flag, and transitions to self. The transition
            // re-fires `state_enter` — now the unguarded clause
            // matches (skip flag is false), so the user `$>` body
            // runs with the proper enter_args.
            if is_start_state {
                let arg_pattern = if sys_param_vars.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", sys_param_vars.join(", "))
                };
                let mut overrides = frame_init_record_overrides.clone();
                overrides.push("frame_skip_enter__ = false".to_string());
                // `repeat_state` re-fires state_enter for the current
                // state (gen_statem's `{next_state, SameState, Data}`
                // does NOT trigger state_enter on self-transition, but
                // `repeat_state` does). With the skip flag cleared,
                // the unguarded enter clause matches and runs user $>.
                code.push_str(&format!(
                    "{}({{call, From}}, {{frame_init, {}}}, Data) ->\n    Data1 = Data#data{{{}}},\n    {{repeat_state, Data1, [{{reply, From, ok}}]}};\n",
                    state_name,
                    arg_pattern,
                    overrides.join(", ")
                ));
            }

            // Enter handler. RFC-0019: `$>` is a leaf-dispatched event —
            // gen_statem fires `enter` only on the leaf, and we run only
            // the leaf's own body here. Ancestors run their `$>` solely
            // when the leaf forwards (`=> $^` lowers to a
            // `frame_enter__<parent>(Data)` call — see frame_expansion.rs
            // and the body processor). No ancestor chain walk.
            code.push_str(&format!("{}(enter, _OldState, Data) ->\n", state_name));
            let data_gen = 0;
            // The leaf's inline body starts from `Data` (the enter
            // callback's record param). The emission code below was
            // written referring to `Data` directly.
            let leaf_data_in = "Data".to_string();
            // State-args binding: when the state was declared as
            // `$State(p1: T1, p2: T2)`, the enter handler can
            // reference each param by name. Bind from
            // `Data#data.frame_state_args` here so `$>()` body
            // and the user-event handlers (which already bind
            // these — see line 3285) see consistent values.
            // Pre-fix this was missing, leaving state-arg names
            // as free variables in `enter` clauses.
            //
            // Elide the prefetch when the enter body doesn't
            // reference the state-arg name (avoids erlc unused-
            // variable warnings on enter clauses). Frame source may
            // use either the lowercase declared name or the Erlang-
            // capitalized form — check both.
            let enter_body_src = state.enter.as_ref().and_then(|e| {
                std::str::from_utf8(&source[e.body.span.start..e.body.span.end]).ok()
            });
            for (i, sp) in state.params.iter().enumerate() {
                let cap = erlang_safe_capitalize(&sp.name);
                if let Some(body) = enter_body_src {
                    let used = raw_contains_word(body, &sp.name) || raw_contains_word(body, &cap);
                    if !used {
                        continue;
                    }
                } else if state.enter.is_none() {
                    // No enter body at all — nothing references the
                    // state-arg in the enter clause.
                    continue;
                }
                code.push_str(&format!(
                    "    {} = frame_arg_at__({}, {}#data.frame_state_args),\n",
                    cap,
                    i + 1,
                    leaf_data_in
                ));
            }
            if let Some(ref enter) = state.enter {
                // Extract enter params from frame_enter_args (positional list).
                for (i, p) in enter.params.iter().enumerate() {
                    let var_name = erlang_safe_capitalize(&p.name);
                    code.push_str(&format!(
                        "    {} = frame_arg_at__({}, {}#data.frame_enter_args),\n",
                        var_name,
                        i + 1,
                        leaf_data_in
                    ));
                }
                // Use splicer for proper $.var expansion
                let enter_ctx = HandlerContext {
                    system_name: sys.to_string(),
                    state_name: state.name.clone(),
                    event_name: "$>".to_string(),
                    parent_state: state.parent.clone(),
                    defined_systems: std::collections::HashSet::from([sys.to_string()]),
                    use_sv_comp: false,
                    per_handler: false,
                    state_var_types: std::collections::HashMap::new(),
                    state_param_names: std::collections::HashMap::new(),
                    state_enter_param_names: std::collections::HashMap::new(),
                    state_exit_param_names: std::collections::HashMap::new(),
                    event_param_names: std::collections::HashMap::new(),
                    state_hsm_parents: std::collections::HashMap::new(),
                    current_return_type: None,
                    state_param_types: std::collections::HashMap::new(),
                };
                let enter_span = crate::frame_c::compiler::ast::Span {
                    start: enter.body.span.start,
                    end: enter.body.span.end,
                };
                let raw_enter = emit_handler_body_via_statements(
                    &enter_span,
                    source,
                    TargetLanguage::Erlang,
                    &enter_ctx,
                );
                let enter_body = erlang_transform_blocks(&erlang_lower_native_if(&raw_enter));

                if !enter_body.trim().is_empty() {
                    let enter_params: Vec<(&str, String)> = enter
                        .params
                        .iter()
                        .map(|p| {
                            let cap = erlang_safe_capitalize(&p.name);
                            (p.name.as_str(), cap)
                        })
                        .collect();
                    let lines: Vec<&str> = enter_body.lines().collect();
                    let (processed, final_data, _final_rv) = erlang_process_body_lines_with_params(
                        &lines,
                        &action_names,
                        &interface_names,
                        &leaf_data_in,
                        &enter_params,
                    );
                    if !processed.is_empty() {
                        // Check if enter handler contains a transition
                        let has_enter_transition = processed
                            .iter()
                            .any(|l| l.trim().starts_with("frame_transition__("));
                        if has_enter_transition {
                            // Enter handlers can't return {next_state,...} in gen_statem state_enter mode,
                            // and {next_event,...} actions are forbidden from a state enter call.
                            // Defer the transition via a zero-delay state_timeout, which IS allowed
                            // from enter callbacks and is dispatched as a normal event afterward.
                            //
                            // The standard `frame_transition__` does:
                            //   exit-dispatch + update Data with {exit/enter/state args, target} +
                            //   {next_state, Target, ...}
                            // For state_timeout-deferred mode we need the
                            // SAME Data updates but emit `{keep_state,
                            // Data, [{state_timeout, ...}]}` instead.
                            // Pre-fix we kept only `Target` and dropped
                            // exit/enter/state args entirely — Phase 15
                            // P6 (chained $>() → -> $S2(x*2)) hit this:
                            // s2's frame_state_args stayed at s1's
                            // [LIT] instead of becoming [LIT*2].
                            let mut enter_lines = Vec::new();
                            for line in &processed {
                                let t = line.trim();
                                if t.starts_with("frame_transition__(") {
                                    let inner = t
                                        .trim_start_matches("frame_transition__(")
                                        .trim_end_matches(')');
                                    // Bracket/paren-aware split on top-
                                    // level commas. State args like
                                    // `[X * 2]` or `[X, Y]` contain
                                    // nested commas that the naive
                                    // split misses.
                                    let parts = split_top_level_commas(inner);
                                    if parts.len() >= 7 {
                                        let target = parts[0].trim();
                                        let data_in = parts[1].trim();
                                        let exit_args = parts[2].trim();
                                        let enter_args = parts[3].trim();
                                        // Capitalize state-arg name
                                        // references inside the state_
                                        // args expression so they match
                                        // the variables bound at the
                                        // top of the enter handler
                                        // (`X = frame_arg_at__(...)`).
                                        // The Frame source `[x * 2]`
                                        // becomes `[X * 2]` in Erlang.
                                        let mut state_args = parts[4].trim().to_string();
                                        for sp in &state.params {
                                            let cap = erlang_safe_capitalize(&sp.name);
                                            if cap != sp.name {
                                                state_args =
                                                    replace_whole_word(&state_args, &sp.name, &cap);
                                            }
                                        }
                                        // Update Data with exit/enter/
                                        // state args before scheduling
                                        // the deferred transition.
                                        // Mirrors frame_transition__'s
                                        // body (exit dispatch + record
                                        // update) — same effect, just
                                        // returns keep_state instead of
                                        // next_state.
                                        enter_lines.push(format!(
                                            "    __DataX1 = {}#data{{frame_exit_args = {}}},\n    __DataX2 = frame_exit_dispatch__(__DataX1),\n    __DataX3 = __DataX2#data{{frame_enter_args = {}, frame_state_args = {}, frame_current_state = {}}},\n    {{keep_state, __DataX3, [{{state_timeout, 0, {{frame_enter_transition, {}}}}}]}}",
                                            data_in, exit_args, enter_args, state_args, target, target
                                        ));
                                    }
                                } else {
                                    enter_lines.push(line.clone());
                                }
                            }
                            erlang_smart_join(&enter_lines, &mut code);
                        } else {
                            erlang_smart_join(&processed, &mut code);
                            code.push_str(",\n");
                            code.push_str(&format!("    {{keep_state, {}}}", final_data));
                        }
                    } else {
                        code.push_str(&format!("    {{keep_state, {}}}", final_data));
                    }
                    code.push_str(";\n");
                } else {
                    code.push_str(&format!("    {{keep_state, {}}};\n", leaf_data_in));
                }
            } else if !state.state_vars.is_empty() {
                // No explicit enter handler, but state has state vars — auto-init them
                let state_prefix = to_snake_case(&state.name);
                let mut data_var = leaf_data_in.clone();
                let mut gen = data_gen;
                for sv in &state.state_vars {
                    let field_name = format!("sv_{}_{}", state_prefix, sv.name);
                    let init_val = if let Some(ref init) = sv.init {
                        expression_to_string(init, TargetLanguage::Erlang)
                    } else {
                        "undefined".to_string()
                    };
                    gen += 1;
                    let new_var = format!("Data{}", gen);
                    code.push_str(&format!(
                        "    {} = {}#data{{{} = {}}},\n",
                        new_var, data_var, field_name, init_val
                    ));
                    data_var = new_var;
                }
                code.push_str(&format!("    {{keep_state, {}}};\n", data_var));
            } else {
                // No enter handler and no state vars on this state.
                // Just return whatever Data accumulated through ancestor
                // cascade calls (or `Data` if there were no ancestors).
                code.push_str(&format!("    {{keep_state, {}}};\n", leaf_data_in));
            }

            // Event handlers. The parser puts lifecycle `$>` / `<$` into
            // `state.enter` / `state.exit`, not here, so `state.handlers` only
            // contains user-defined interface methods — no lifecycle-skip
            // filter needed. A user method named `enter` or `exit` dispatches
            // as a regular event atom (fixes bug_enter_exit_method_collision).
            for handler in &state.handlers {
                let event_atom = to_snake_case(&handler.event);

                // Build parameter pattern for gen_statem call. Bind
                // the entire matched event to `__Event` so handler
                // bodies that perform a forward transition (`-> =>
                // $State`) can re-dispatch it via `frame_forward_transition__`.
                // The leading `__` follows Erlang's underscore-prefix
                // convention (suppresses unused-variable warnings for
                // handlers that don't reference it). Same convention
                // as the existing catch-all clauses below
                // (`__Event` for parent-forward dispatch).
                // Underscore-prefix params that the handler body
                // doesn't reference. Erlang treats `_Name` as
                // intentionally-unused and suppresses the warning;
                // bare `Name` triggers an unused-variable warning on
                // every clause that doesn't read it.
                let handler_body_src_for_params =
                    std::str::from_utf8(&source[handler.body.span.start..handler.body.span.end])
                        .unwrap_or("");
                // Frame source for Erlang target may reference the
                // param by either its declared (lowercase) name OR
                // its Erlang-capitalized name (e.g. `Mode` instead of
                // `mode`) — check both forms.
                let call_pattern = if handler.params.is_empty() {
                    format!("__Event = {}", event_atom)
                } else {
                    let param_names: Vec<String> = handler
                        .params
                        .iter()
                        .map(|p| {
                            let cap = erlang_safe_capitalize(&p.name);
                            let used = raw_contains_word(handler_body_src_for_params, &p.name)
                                || raw_contains_word(handler_body_src_for_params, &cap);
                            if used {
                                cap
                            } else {
                                format!("_{}", cap)
                            }
                        })
                        .collect();
                    format!("__Event = {{{}, {}}}", event_atom, param_names.join(", "))
                };

                code.push_str(&format!(
                    "{}({{call, From}}, {}, Data) ->\n",
                    state_name, call_pattern
                ));

                // State params: bind frame_state_args[i] to a local
                // Erlang variable so handler bodies can read state params
                // by their declared name. Index matches the parameter's
                // declaration order in `$State(p1, p2, ...)`. Mirrors the
                // Python dispatch preamble that prepends
                // `name = compartment.state_args[index]`.
                //
                // Elide the prefetch when the handler body doesn't
                // reference the state-arg name — otherwise erlc emits
                // an unused-variable warning on every clause whose body
                // happens not to use the cascaded state-arg. Check both
                // the lowercase declared name and the Erlang-
                // capitalized form (Frame source may use either).
                let handler_body_src =
                    std::str::from_utf8(&source[handler.body.span.start..handler.body.span.end])
                        .unwrap_or("");
                // Build the effective param list for this state: own
                // params plus any params declared at a descendant's
                // cascade arrow (`$Child => $Self(name: T)`). Erlang's
                // frame_state_args is a flat list shared by every
                // compartment in the HSM chain, so an ancestor's
                // handler can read descendant-declared names directly.
                let mut effective_params: Vec<(String, usize)> = state
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.name.clone(), i))
                    .collect();
                for descendant in machine.states.iter() {
                    let mut cursor = descendant.parent.clone();
                    let mut is_descendant = false;
                    while let Some(p) = cursor {
                        if p == state.name {
                            is_descendant = true;
                            break;
                        }
                        cursor = machine
                            .states
                            .iter()
                            .find(|s| s.name == p)
                            .and_then(|s| s.parent.clone());
                    }
                    if !is_descendant {
                        continue;
                    }
                    for (i, p) in descendant.params.iter().enumerate() {
                        if !effective_params.iter().any(|(n, _)| n == &p.name) {
                            effective_params.push((p.name.clone(), i));
                        }
                    }
                }
                for (name, idx) in &effective_params {
                    let cap = erlang_safe_capitalize(name);
                    let used = raw_contains_word(handler_body_src, name)
                        || raw_contains_word(handler_body_src, &cap);
                    if !used {
                        continue;
                    }
                    // Shadow check: the handler header pattern at
                    // `__Event = {ev, P1, P2, ...}` already binds any
                    // event param with this name. Re-binding via
                    // `Cap = frame_arg_at__(...)` would be a pattern
                    // match against the existing binding and crash if
                    // values differ (D9 follow-up — same shape as the
                    // typed-language D5 shadow check).
                    let shadowed_by_event_param = handler.params.iter().any(|p| &p.name == name);
                    if shadowed_by_event_param {
                        continue;
                    }
                    code.push_str(&format!(
                        "    {} = frame_arg_at__({}, Data#data.frame_state_args),\n",
                        cap,
                        idx + 1
                    ));
                }

                // Use emit_handler_body_via_statements for proper Frame statement expansion
                let handler_ctx = HandlerContext {
                    system_name: sys.to_string(),
                    state_name: state.name.clone(),
                    event_name: handler.event.clone(),
                    parent_state: state.parent.clone(),
                    defined_systems: std::collections::HashSet::from([sys.to_string()]),
                    use_sv_comp: false,
                    per_handler: false,
                    state_var_types: std::collections::HashMap::new(),
                    state_param_names: std::collections::HashMap::new(),
                    state_enter_param_names: std::collections::HashMap::new(),
                    state_exit_param_names: std::collections::HashMap::new(),
                    event_param_names: std::collections::HashMap::new(),
                    state_hsm_parents: std::collections::HashMap::new(),
                    current_return_type: None,
                    state_param_types: std::collections::HashMap::new(),
                };
                // Convert frame_ast::Span to ast::Span
                let body_span = crate::frame_c::compiler::ast::Span {
                    start: handler.body.span.start,
                    end: handler.body.span.end,
                };
                let raw_spliced = emit_handler_body_via_statements(
                    &body_span,
                    source,
                    TargetLanguage::Erlang,
                    &handler_ctx,
                );

                // Transform if/else { } blocks to Erlang case/of/end
                let spliced_body = erlang_transform_blocks(&erlang_lower_native_if(&raw_spliced));

                // Post-process: rewrite self.X, capitalize params, thread Data.
                // Include both handler params AND state params (declared via
                // `$Start(x: int)`) so the body can reference state-args
                // bound at the top of the clause by their declared name.
                let handler_params: Vec<(&str, String)> = handler
                    .params
                    .iter()
                    .map(|p| {
                        let capitalized = erlang_safe_capitalize(&p.name);
                        (p.name.as_str(), capitalized)
                    })
                    .chain(state.params.iter().map(|sp| {
                        let capitalized = erlang_safe_capitalize(&sp.name);
                        (sp.name.as_str(), capitalized)
                    }))
                    .collect();

                // Check if the spliced body contains a gen_statem return tuple, forward, or frame_transition
                let has_forward_call = spliced_body.contains("({call, From},");
                let has_frame_transition = spliced_body.contains("frame_transition__(")
                    || spliced_body.contains("frame_forward_transition__(");
                let has_return_tuple = spliced_body.contains("{next_state,")
                    || spliced_body.contains("{keep_state,")
                    || has_forward_call
                    || has_frame_transition;
                let has_case_block =
                    spliced_body.contains("case (") || spliced_body.contains("case(");

                if has_return_tuple {
                    // Exit handler is now handled by __frame_transition — no inlining needed

                    // Process through Data threading (handles both simple and case-block bodies)
                    let lines: Vec<&str> = spliced_body.lines().collect();
                    let (processed, _final_data, final_rv) = erlang_process_body_lines_full(
                        &lines,
                        &action_names,
                        &interface_names,
                        "Data",
                        &handler_params,
                    );
                    // Wrap any @@:self dispatch sites in transition-guard
                    // `case` expressions so a state change inside the called
                    // handler short-circuits the rest of the caller's body.
                    // No-op if the body has no `= frame_dispatch__(` lines.
                    let processed =
                        erlang_wrap_self_call_guards(&processed, &to_snake_case(&state.name));
                    // Inject gen_statem reply tuples at any orphan
                    // `__ReturnVal = "..."` leaves (innermost else branches
                    // of nested if/else that don't transition). Fixes the
                    // "bad_return_from_state_function" crash when a handler
                    // has a non-transitioning terminal case arm.
                    let processed = erlang_inject_orphan_reply_tuples(&processed, &_final_data);
                    if !processed.is_empty() {
                        // Use structured case arm analysis when a case block exists
                        if let Some((classification, arms, case_start, case_end)) =
                            analyze_case_arms(&processed)
                        {
                            match classification {
                                CaseBlockClassification::AllTerminal => {
                                    // All arms have transitions — case is terminal, use as handler return
                                    let emit_lines = &processed[..=case_end];
                                    erlang_smart_join(emit_lines, &mut code);
                                    if code.trim_end().ends_with("end,") {
                                        if let Some(pos) = code.rfind("end,") {
                                            code.replace_range(pos + 3..pos + 4, "");
                                        }
                                    }
                                }
                                CaseBlockClassification::Mixed => {
                                    // Some arms transition, some don't — per-arm rewrite.
                                    // Thread the SSA-renamed top-level return value
                                    // (`__ReturnVal_K`) so non-transition arms reply
                                    // with the value @@:return wrote BEFORE the case
                                    // block. Falls back to `"ok"` when the body has
                                    // no top-level return.
                                    let rewritten = rewrite_mixed_case_arms(
                                        &processed,
                                        &arms,
                                        case_start,
                                        case_end,
                                        &_final_data,
                                        final_rv.as_str(),
                                    );
                                    erlang_smart_join(&rewritten, &mut code);
                                }
                                CaseBlockClassification::NoTerminal => {
                                    // Case arms have no transitions of their own, but the body
                                    // leading into the case DID contain a forward (otherwise we
                                    // wouldn't be in has_return_tuple). The forward bind produces
                                    // `__FwdNextN`; the case arms only thread Data. Emit the
                                    // case block as-is, then append a conditional terminal tuple
                                    // that honors whichever transition (if any) the parent
                                    // performed — mirroring the no-case-block path below.
                                    erlang_smart_join(&processed, &mut code);
                                    let last_fwd_var: Option<String> =
                                        processed.iter().rev().find_map(|line| {
                                            line.find("__FwdNext").map(|i| {
                                                let rest = &line[i..];
                                                rest.chars()
                                                    .take_while(|c| {
                                                        c.is_ascii_alphanumeric() || *c == '_'
                                                    })
                                                    .collect::<String>()
                                            })
                                        });
                                    let reply_val = final_rv.as_str();
                                    if let Some(fwd) = last_fwd_var {
                                        code.push_str(",\n");
                                        code.push_str(&format!(
                                            "    case {} of\n        undefined -> {{keep_state, {}, [{{reply, From, {}}}]}};\n        _ -> {{next_state, {}, {}, [{{reply, From, {}}}]}}\n    end",
                                            fwd, _final_data, reply_val, fwd, _final_data, reply_val
                                        ));
                                    } else {
                                        code.push_str(",\n");
                                        code.push_str(&format!(
                                            "    {{keep_state, {}, [{{reply, From, {}}}]}}",
                                            _final_data, reply_val
                                        ));
                                    }
                                }
                            }
                        } else {
                            // No case block — use existing terminal detection for linear handlers.
                            // A raw forward tail-call (`p({call,From},...)`) IS terminal. A forward
                            // *bind* emitted by the body processor (`{Data1, __FwdNext1} = frame_unwrap_forward__(...)`)
                            // is not — post-forward code follows and must run. The leading `{` guard
                            // distinguishes them.
                            let is_terminal = |l: &str| -> bool {
                                let t = l.trim();
                                (t.contains("({call, From},") && !t.starts_with("{"))
                                    || t.starts_with("frame_transition__(")
                                    || t.starts_with("frame_forward_transition__(")
                                    || t.starts_with("{next_state,")
                                    || t.starts_with("{keep_state,")
                            };
                            let mut terminal_idx: Option<usize> = None;
                            for (idx, line) in processed.iter().enumerate() {
                                if is_terminal(line.trim()) {
                                    terminal_idx = Some(idx);
                                    break;
                                }
                            }
                            if let Some(tidx) = terminal_idx {
                                erlang_smart_join(&processed[..=tidx], &mut code);
                            } else {
                                // No inline terminal. If a forward was rewritten to a bind, emit
                                // a conditional tuple that propagates the parent's transition
                                // (if any) after post-forward statements have run.
                                erlang_smart_join(&processed, &mut code);
                                let last_fwd_var: Option<String> =
                                    processed.iter().rev().find_map(|line| {
                                        line.find("__FwdNext").map(|i| {
                                            let rest = &line[i..];
                                            rest.chars()
                                                .take_while(|c| {
                                                    c.is_ascii_alphanumeric() || *c == '_'
                                                })
                                                .collect::<String>()
                                        })
                                    });
                                let reply_val = final_rv.as_str();
                                if let Some(fwd) = last_fwd_var {
                                    code.push_str(",\n");
                                    code.push_str(&format!(
                                        "    case {} of\n        undefined -> {{keep_state, {}, [{{reply, From, {}}}]}};\n        _ -> {{next_state, {}, {}, [{{reply, From, {}}}]}}\n    end",
                                        fwd, _final_data, reply_val, fwd, _final_data, reply_val
                                    ));
                                } else {
                                    code.push_str(",\n");
                                    code.push_str(&format!(
                                        "    {{keep_state, {}, [{{reply, From, {}}}]}}",
                                        _final_data, reply_val
                                    ));
                                }
                            }
                        }
                    }
                    // Ensure clause terminator is on its own line (not hidden by % comment)
                    if !code.ends_with('\n') {
                        code.push('\n');
                    }
                    code.push_str(";\n");
                } else {
                    // No return tuple — process through Data threading and add return
                    let lines: Vec<&str> = spliced_body.lines().collect();
                    let (processed, final_data, final_rv_nr) = erlang_process_body_lines_full(
                        &lines,
                        &action_names,
                        &interface_names,
                        "Data",
                        &handler_params,
                    );
                    if processed.is_empty() {
                        code.push_str("    {keep_state, Data, [{reply, From, ok}]};\n");
                    } else {
                        // Use the body processor's authoritative
                        // final return-value name. `"ok"` when no
                        // `@@:return` writes happened in the body.
                        // Top-level writes (outside any case) bubbled up via
                        // `final_rv_nr` into a `__ReturnVal_K` SSA name. But
                        // `@@:return` inside `if`/`case` arms emits
                        // `__ReturnVal = …` without bumping `final_rv_nr`
                        // (the body processor only counts top-level writes
                        // — case-arm writes are arm-local). Detect those
                        // so the hoist-assignment branch below picks them
                        // up and uses the literal `__ReturnVal` in the
                        // reply tuple.
                        let has_top_return_val = final_rv_nr != "ok";
                        let has_case_return_val = processed
                            .iter()
                            .any(|l| l.trim().starts_with("__ReturnVal = "));
                        let has_return_val = has_top_return_val || has_case_return_val;
                        let has_transition = processed
                            .iter()
                            .any(|l| l.trim().starts_with("frame_transition__("));
                        let has_case = processed
                            .iter()
                            .any(|l| l.trim().starts_with("case ") || l.contains(" case "));
                        // When the only writes were inside case arms,
                        // `final_rv_nr` is still "ok"; the hoist branch
                        // emits `__ReturnVal = case … of … end` so the
                        // literal name is correct for the reply.
                        let reply_val: &str = if has_top_return_val {
                            final_rv_nr.as_str()
                        } else if has_case_return_val {
                            "__ReturnVal"
                        } else {
                            final_rv_nr.as_str()
                        };

                        if has_case && has_transition {
                            // Case block with transitions in some arms.
                            // Each arm must evaluate to a gen_statem return tuple:
                            //   - Arms with frame_transition__() already produce {next_state,...}
                            //   - Arms without need {keep_state, Data, [{reply, From, ReturnVal}]}
                            // The case expression IS the handler return — no trailing {keep_state,...}
                            //
                            // Default per-arm reply-value: the top-level SSA name
                            // (`reply_val`). When the user wrote @@:return BEFORE
                            // the case, the value lives in `__ReturnVal_K` and
                            // we need every non-transition arm to reply with it.
                            // An in-arm @@:return overrides this default.
                            let arm_default_rv: String = reply_val.to_string();
                            let mut rewritten = Vec::new();
                            let mut in_case = false;
                            let mut arm_has_transition = false;
                            let mut arm_return_val: Option<String> = None;

                            for line in &processed {
                                let trimmed = line.trim();

                                if trimmed.starts_with("case ") {
                                    in_case = true;
                                    arm_has_transition = false;
                                    arm_return_val = None;
                                    rewritten.push(line.clone());
                                    continue;
                                }

                                if in_case
                                    && (trimmed.starts_with("true ->")
                                        || trimmed.starts_with("; false")
                                        || trimmed.starts_with("; _"))
                                {
                                    // Entering a new arm — flush previous arm's keep_state if needed
                                    if trimmed.starts_with("; ") && !arm_has_transition {
                                        // Previous arm had no transition — inject keep_state
                                        let rv = arm_return_val
                                            .as_deref()
                                            .unwrap_or(arm_default_rv.as_str());
                                        rewritten.push(format!(
                                            "        {{keep_state, {}, [{{reply, From, {}}}]}}",
                                            final_data, rv
                                        ));
                                    }
                                    arm_has_transition = false;
                                    arm_return_val = None;
                                    rewritten.push(line.clone());
                                    continue;
                                }

                                if in_case && trimmed.starts_with("__ReturnVal = ") {
                                    let val = trimmed
                                        .trim_start_matches("__ReturnVal = ")
                                        .trim_end_matches(',');
                                    arm_return_val = Some(val.to_string());
                                    // Don't emit the assignment — embed the value in the reply tuple
                                    continue;
                                }

                                if in_case && trimmed.starts_with("frame_transition__(") {
                                    arm_has_transition = true;
                                    // Emit the transition call — it produces the arm's return tuple
                                    rewritten.push(line.clone());
                                    continue;
                                }

                                if in_case && (trimmed == "end" || trimmed == "end,") {
                                    // Last arm ending — inject keep_state if no transition
                                    if !arm_has_transition {
                                        let rv = arm_return_val
                                            .as_deref()
                                            .unwrap_or(arm_default_rv.as_str());
                                        rewritten.push(format!(
                                            "        {{keep_state, {}, [{{reply, From, {}}}]}}",
                                            final_data, rv
                                        ));
                                    }
                                    rewritten.push(format!("    end"));
                                    in_case = false;
                                    continue;
                                }

                                rewritten.push(line.clone());
                            }

                            erlang_smart_join(&rewritten, &mut code);
                            // The case expression is the handler return — just terminate the clause
                            code.push_str(";\n");
                        } else if has_return_val && has_case {
                            // Case block with __ReturnVal but no transitions — hoist
                            // the case as a value-producing expression. Per-arm rule:
                            // the arm's last expression IS the arm's value, so the
                            // `__ReturnVal = X` line must be the LAST statement in
                            // each arm. Other statements (e.g., `Data1 = Data` that
                            // the body processor injects to balance variable bindings
                            // across arms) must come BEFORE it. We buffer arm bodies,
                            // strip the assignment, and emit the RHS last.
                            let arm_boundary = |t: &str| -> bool {
                                t.ends_with("->")
                                    && (t == "true ->" || t.starts_with("; ") || t == "_ ->")
                            };
                            let case_end =
                                |t: &str| -> bool { t == "end" || t == "end," || t == "end;" };

                            let mut rewritten: Vec<String> = Vec::new();
                            let mut in_case = false;
                            let mut hoisted = false;
                            let mut arm_buf: Vec<String> = Vec::new();
                            let mut arm_value: Option<String> = None;

                            let flush_arm =
                                |buf: &mut Vec<String>,
                                 val: &mut Option<String>,
                                 out: &mut Vec<String>| {
                                    // Emit non-return-val lines first, then the value.
                                    for l in buf.drain(..) {
                                        out.push(l);
                                    }
                                    if let Some(v) = val.take() {
                                        out.push(format!("    {}", v));
                                    }
                                };

                            for line in &processed {
                                let trimmed = line.trim();
                                if trimmed.starts_with("case ") && !hoisted {
                                    rewritten.push(format!("    __ReturnVal = {}", trimmed));
                                    in_case = true;
                                    hoisted = true;
                                    continue;
                                }
                                if in_case && arm_boundary(trimmed) {
                                    flush_arm(&mut arm_buf, &mut arm_value, &mut rewritten);
                                    rewritten.push(line.clone());
                                    continue;
                                }
                                if in_case && case_end(trimmed) {
                                    flush_arm(&mut arm_buf, &mut arm_value, &mut rewritten);
                                    rewritten.push(line.clone());
                                    in_case = false;
                                    continue;
                                }
                                if in_case && trimmed.starts_with("__ReturnVal = ") {
                                    // Strip the assignment; capture RHS, drop a
                                    // possible trailing comma so it's a clean
                                    // tail expression.
                                    let val = trimmed
                                        .trim_start_matches("__ReturnVal = ")
                                        .trim_end_matches(',');
                                    arm_value = Some(val.to_string());
                                    continue;
                                }
                                if in_case {
                                    arm_buf.push(line.clone());
                                } else {
                                    rewritten.push(line.clone());
                                }
                            }
                            // Defensive flush — well-formed input always ends with
                            // `end` so this should be a no-op.
                            flush_arm(&mut arm_buf, &mut arm_value, &mut rewritten);

                            erlang_smart_join(&rewritten, &mut code);
                            code.push_str(",\n");
                            code.push_str(&format!(
                                "    {{keep_state, {}, [{{reply, From, {}}}]}};\n",
                                final_data, reply_val
                            ));
                        } else {
                            // Build the full body — processed body + keep_state terminal —
                            // then wrap any `@@:self` dispatch sites in transition-guard
                            // cases so a state change inside the called handler
                            // short-circuits the rest of the caller and propagates the
                            // new state back through gen_statem.
                            let mut full = processed.clone();
                            full.push(format!(
                                "    {{keep_state, {}, [{{reply, From, {}}}]}}",
                                final_data, reply_val
                            ));
                            let wrapped =
                                erlang_wrap_self_call_guards(&full, &to_snake_case(&state.name));
                            erlang_smart_join(&wrapped, &mut code);
                            code.push_str(";\n");
                        }
                    }
                }
            }

            // `frame_op_call` dispatch — routes external op calls into
            // the server process. Each non-static op's external wrapper
            // emits `gen_statem:call(Pid, {frame_op_call, <op>, [Args]})`;
            // here we match that message, invoke the internal op (same
            // function name, arity-1 Data clause), destructure the
            // `{UpdatedData, Result}` tuple, and reply with Result while
            // keeping the updated Data in the gen_statem state. This
            // clause is emitted in every state so ops are callable
            // regardless of the machine's current state.
            for op in &system.operations {
                if op.is_static {
                    continue;
                }
                // RFC-0012 amendment: framework-managed save/load are
                // emitted as module-level functions, not state dispatch
                // clauses. Skip here.
                if op
                    .attributes
                    .iter()
                    .any(|a| a.name == "save" || a.name == "load")
                {
                    continue;
                }
                let op_lc = erlang_op_name(&op.name);
                let n = op.params.len();
                let arg_vars: Vec<String> = (0..n).map(|i| format!("A{}", i + 1)).collect();
                let pattern_args = if arg_vars.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", arg_vars.join(", "))
                };
                let call_args = if arg_vars.is_empty() {
                    "Data".to_string()
                } else {
                    format!("Data, {}", arg_vars.join(", "))
                };
                code.push_str(&format!(
                    "{}({{call, From}}, {{frame_op_call, {}, {}}}, Data) ->\n    {{NewData, __Result}} = {}({}),\n    {{keep_state, NewData, [{{reply, From, __Result}}]}};\n",
                    state_name, op_lc, pattern_args, op_lc, call_args
                ));
            }

            // State-timeout handler for deferred enter-handler transitions.
            // When an enter handler calls -> $State, we defer via:
            //   {keep_state, Data, [{state_timeout, 0, {frame_enter_transition, Target}}]}
            // This clause processes the resulting state_timeout event.
            code.push_str(&format!(
                "{}(state_timeout, {{frame_enter_transition, Target}}, Data) ->\n    {{next_state, Target, Data}};\n",
                state_name
            ));

            // Default catch-all for unhandled events in this state.
            //
            // Frame contract: `=> $^` (default-forward) is OPTIONAL.
            // Having a parent state via `$Child => $Parent` declares
            // the HSM relationship but does NOT imply that unhandled
            // events cascade. The user must explicitly write
            // `=> $^` as a trailing clause in the state body to opt
            // into auto-cascade.
            //
            // Erlang's gen_statem requires a `[{reply, From, V}]`
            // for every call event to avoid caller deadlock. So:
            //   - If state has parent AND default_forward=true:
            //     forward unhandled call events to parent (matches
            //     other backends' explicit-cascade behavior).
            //   - Otherwise (no parent OR no default_forward): emit
            //     a no-op reply with the type default — `ok` here,
            //     matching the wrapper's null-default for typed
            //     returns. The wrapper's `frame_return_default`
            //     runs at the boundary, normalising `ok` to the
            //     declared int/str/bool default.
            if state.default_forward {
                if let Some(ref parent) = state.parent {
                    let parent_atom = state_atom(parent);
                    code.push_str(&format!("{}({{call, From}}, __Event, Data) ->\n    {}({{call, From}}, __Event, Data);\n", state_name, parent_atom));
                } else {
                    // Edge: `=> $^` declared but no parent → user
                    // bug, but framec validator catches it. Emit
                    // reply-with-ok as a defensive fallback.
                    code.push_str(&format!("{}({{call, From}}, _Event, Data) ->\n    {{keep_state, Data, [{{reply, From, ok}}]}};\n", state_name));
                }
            } else {
                // No explicit `=> $^` — unhandled events drop with
                // a deadlock-safe reply (gen_statem requires reply
                // for every call). Reply value is `ok`; the
                // per-interface-method wrapper coerces this sentinel
                // to the declared return type's default (`0` for
                // int, `<<>>` for str, `false` for bool, etc.) at
                // its own emission site, where the return type is
                // known. See `Erlang interface wrapper` below.
                code.push_str(&format!("{}({{call, From}}, _Event, Data) ->\n    {{keep_state, Data, [{{reply, From, ok}}]}};\n", state_name));
            }
            code.push_str(&format!(
                "{}(_EventType, _Event, Data) ->\n    {{keep_state, Data}}.\n\n",
                state_name
            ));
        }

        // ────────────────────────────────────────────────────────────────
        // HSM enter cascade helpers — `frame_enter__<state>(Data) -> Data`
        //
        // One helper per state that is a parent (i.e., declared as
        // `=>` parent by at least one other state) AND has either an
        // explicit `$>` handler or state-vars to auto-init. Called
        // from a descendant's `<state>(enter, _OldState, Data)`
        // clause so ancestor `$>` handlers fire top-down per spec
        // Step 21.
        //
        // States that are never a parent skip helper emission — the
        // helper would be dead code (Erlang -W produces an
        // unused-function warning).
        //
        // States whose enter handler contains a transition are NOT
        // extracted (the transition needs gen_statem's state_timeout
        // mechanism, which only applies in a state's own enter
        // clause). Those stay inline in the state function. Cascading
        // into such a state from a descendant remains a spec
        // edge-case: the descendant's leaf-clause cascade walk would
        // skip the helper call (since the helper isn't emitted). For
        // the matrix's tested HSMs this is a non-issue (no
        // transitions in intermediate-ancestor enter handlers).
        let parent_set: std::collections::HashSet<String> = machine
            .states
            .iter()
            .filter_map(|s| s.parent.clone())
            .collect();
        for state in &machine.states {
            // Detect transition-in-enter at codegen time so we can
            // skip helper emission for those states.
            let has_enter_transition = if let Some(ref enter) = state.enter {
                let enter_ctx = HandlerContext {
                    system_name: sys.to_string(),
                    state_name: state.name.clone(),
                    event_name: "$>".to_string(),
                    parent_state: state.parent.clone(),
                    defined_systems: std::collections::HashSet::from([sys.to_string()]),
                    use_sv_comp: false,
                    per_handler: false,
                    state_var_types: std::collections::HashMap::new(),
                    state_param_names: std::collections::HashMap::new(),
                    state_enter_param_names: std::collections::HashMap::new(),
                    state_exit_param_names: std::collections::HashMap::new(),
                    event_param_names: std::collections::HashMap::new(),
                    state_hsm_parents: std::collections::HashMap::new(),
                    current_return_type: None,
                    state_param_types: std::collections::HashMap::new(),
                };
                let enter_span = crate::frame_c::compiler::ast::Span {
                    start: enter.body.span.start,
                    end: enter.body.span.end,
                };
                let raw_enter = emit_handler_body_via_statements(
                    &enter_span,
                    source,
                    TargetLanguage::Erlang,
                    &enter_ctx,
                );
                let enter_body = erlang_transform_blocks(&erlang_lower_native_if(&raw_enter));
                let enter_params: Vec<(&str, String)> = enter
                    .params
                    .iter()
                    .map(|p| (p.name.as_str(), erlang_safe_capitalize(&p.name)))
                    .collect();
                let lines: Vec<&str> = enter_body.lines().collect();
                let (processed, _, _) = erlang_process_body_lines_with_params(
                    &lines,
                    &action_names,
                    &interface_names,
                    "Data",
                    &enter_params,
                );
                processed
                    .iter()
                    .any(|l| l.trim().starts_with("frame_transition__("))
            } else {
                false
            };

            if has_enter_transition {
                // Skip — leaf inline emission preserves state_timeout.
                continue;
            }

            let needs_emission = state.enter.is_some() || !state.state_vars.is_empty();
            if !needs_emission {
                continue;
            }

            // Only emit a helper for states that are actually used as
            // a parent by another state. Otherwise the helper would
            // be dead code (Erlang -W warns on unused functions).
            if !parent_set.contains(&state.name) {
                continue;
            }

            let state_atom_name = state_atom(&state.name);
            code.push_str(&format!("frame_enter__{}(Data) ->\n", state_atom_name));

            if let Some(ref enter) = state.enter {
                // Extract enter params from frame_enter_args (positional).
                for (i, p) in enter.params.iter().enumerate() {
                    let var_name = erlang_safe_capitalize(&p.name);
                    code.push_str(&format!(
                        "    {} = frame_arg_at__({}, Data#data.frame_enter_args),\n",
                        var_name,
                        i + 1
                    ));
                }
                let enter_ctx = HandlerContext {
                    system_name: sys.to_string(),
                    state_name: state.name.clone(),
                    event_name: "$>".to_string(),
                    parent_state: state.parent.clone(),
                    defined_systems: std::collections::HashSet::from([sys.to_string()]),
                    use_sv_comp: false,
                    per_handler: false,
                    state_var_types: std::collections::HashMap::new(),
                    state_param_names: std::collections::HashMap::new(),
                    state_enter_param_names: std::collections::HashMap::new(),
                    state_exit_param_names: std::collections::HashMap::new(),
                    event_param_names: std::collections::HashMap::new(),
                    state_hsm_parents: std::collections::HashMap::new(),
                    current_return_type: None,
                    state_param_types: std::collections::HashMap::new(),
                };
                let enter_span = crate::frame_c::compiler::ast::Span {
                    start: enter.body.span.start,
                    end: enter.body.span.end,
                };
                let raw_enter = emit_handler_body_via_statements(
                    &enter_span,
                    source,
                    TargetLanguage::Erlang,
                    &enter_ctx,
                );
                let enter_body = erlang_transform_blocks(&erlang_lower_native_if(&raw_enter));

                if !enter_body.trim().is_empty() {
                    let enter_params: Vec<(&str, String)> = enter
                        .params
                        .iter()
                        .map(|p| (p.name.as_str(), erlang_safe_capitalize(&p.name)))
                        .collect();
                    let lines: Vec<&str> = enter_body.lines().collect();
                    let (processed, final_data, _) = erlang_process_body_lines_with_params(
                        &lines,
                        &action_names,
                        &interface_names,
                        "Data",
                        &enter_params,
                    );
                    if !processed.is_empty() {
                        erlang_smart_join(&processed, &mut code);
                        code.push_str(",\n");
                    }
                    code.push_str(&format!("    {}.\n\n", final_data));
                } else {
                    code.push_str("    Data.\n\n");
                }
            } else {
                // No explicit enter, but state has state vars — auto-init.
                let state_prefix = to_snake_case(&state.name);
                let mut data_var = "Data".to_string();
                let mut gen = 0;
                for sv in &state.state_vars {
                    let field_name = format!("sv_{}_{}", state_prefix, sv.name);
                    let init_val = if let Some(ref init) = sv.init {
                        expression_to_string(init, TargetLanguage::Erlang)
                    } else {
                        "undefined".to_string()
                    };
                    gen += 1;
                    let new_var = format!("Data{}", gen);
                    code.push_str(&format!(
                        "    {} = {}#data{{{} = {}}},\n",
                        new_var, data_var, field_name, init_val
                    ));
                    data_var = new_var;
                }
                code.push_str(&format!("    {}.\n\n", data_var));
            }
        }
    }

    runtime_helpers::emit_runtime_helpers(&mut code, system);

    // Per-state exit handler functions
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            if let Some(ref exit) = state.exit {
                let sname = state_atom(&state.name);
                code.push_str(&format!("frame_exit__{}(Data) ->\n", sname));

                // Extract exit params (positional from frame_exit_args).
                for (i, p) in exit.params.iter().enumerate() {
                    let var_name = erlang_safe_capitalize(&p.name);
                    code.push_str(&format!(
                        "    {} = frame_arg_at__({}, Data#data.frame_exit_args),\n",
                        var_name,
                        i + 1
                    ));
                }

                // Exit handler body via splicer
                let exit_ctx = HandlerContext {
                    system_name: sys.to_string(),
                    state_name: state.name.clone(),
                    event_name: "<$".to_string(),
                    parent_state: state.parent.clone(),
                    defined_systems: std::collections::HashSet::from([sys.to_string()]),
                    use_sv_comp: false,
                    per_handler: false,
                    state_var_types: std::collections::HashMap::new(),
                    state_param_names: std::collections::HashMap::new(),
                    state_enter_param_names: std::collections::HashMap::new(),
                    state_exit_param_names: std::collections::HashMap::new(),
                    event_param_names: std::collections::HashMap::new(),
                    state_hsm_parents: std::collections::HashMap::new(),
                    current_return_type: None,
                    state_param_types: std::collections::HashMap::new(),
                };
                let exit_span = crate::frame_c::compiler::ast::Span {
                    start: exit.body.span.start,
                    end: exit.body.span.end,
                };
                let raw_exit = emit_handler_body_via_statements(
                    &exit_span,
                    source,
                    TargetLanguage::Erlang,
                    &exit_ctx,
                );

                let exit_params: Vec<(&str, String)> = exit
                    .params
                    .iter()
                    .map(|p| {
                        let cap = erlang_safe_capitalize(&p.name);
                        (p.name.as_str(), cap)
                    })
                    .collect();
                let lines: Vec<&str> = raw_exit.lines().collect();
                let (processed, final_data, _final_rv) = erlang_process_body_lines_with_params(
                    &lines,
                    &action_names,
                    &interface_names,
                    "Data",
                    &exit_params,
                );

                if !processed.is_empty() {
                    erlang_smart_join(&processed, &mut code);
                    code.push_str(",\n");
                }
                code.push_str(&format!("    {}.\n\n", final_data));
            }
        }
    }

    // Frame internal dispatch — for self.method() calls within handlers
    // Calls the state handler directly (avoids gen_statem:call deadlock)
    // Pushes/pops context stack for reentrancy
    code.push_str("frame_dispatch__(EventName, Args, Data) ->\n");
    code.push_str("    Ctx = #{return_val => undefined, data => #{}},\n");
    code.push_str(
        "    Data1 = Data#data{frame_context_stack = [Ctx | Data#data.frame_context_stack]},\n",
    );
    code.push_str("    Msg = case Args of\n");
    code.push_str("        [] -> EventName;\n");
    code.push_str("        _ -> list_to_tuple([EventName | Args])\n");
    code.push_str("    end,\n");
    code.push_str("    State = Data1#data.frame_current_state,\n");
    code.push_str("    FakeFrom = {self(), make_ref()},\n");
    code.push_str("    Result = ?MODULE:State({call, FakeFrom}, Msg, Data1),\n");
    code.push_str("    case Result of\n");
    code.push_str("        {keep_state, Data2, Actions} ->\n");
    code.push_str("            RetVal = case [V || {reply, _, V} <- Actions] of\n");
    code.push_str("                [V | _] -> V;\n");
    code.push_str("                [] -> undefined\n");
    code.push_str("            end,\n");
    code.push_str("            Data3 = Data2#data{frame_context_stack = tl(Data2#data.frame_context_stack)},\n");
    code.push_str("            {Data3, RetVal};\n");
    code.push_str("        {keep_state, Data2} ->\n");
    code.push_str("            Data3 = Data2#data{frame_context_stack = tl(Data2#data.frame_context_stack)},\n");
    code.push_str("            {Data3, undefined};\n");
    code.push_str("        {next_state, NewState, Data2, Actions} ->\n");
    code.push_str("            RetVal = case [V || {reply, _, V} <- Actions] of\n");
    code.push_str("                [V | _] -> V;\n");
    code.push_str("                [] -> undefined\n");
    code.push_str("            end,\n");
    code.push_str("            Data3 = Data2#data{frame_context_stack = tl(Data2#data.frame_context_stack), frame_current_state = NewState},\n");
    code.push_str("            {Data3, RetVal};\n");
    code.push_str("        {next_state, NewState, Data2} ->\n");
    code.push_str("            Data3 = Data2#data{frame_context_stack = tl(Data2#data.frame_context_stack), frame_current_state = NewState},\n");
    code.push_str("            {Data3, undefined}\n");
    code.push_str("    end.\n\n");

    actions_ops::emit_actions_and_operations(
        &mut code,
        system,
        source,
        &action_names,
        &interface_names,
    );

    persist::emit_persistence_methods(&mut code, system);

    // Cross-system call translation. Frame source like
    // `self.inner.bump()` (cross-target idiomatic dot-call) gets
    // rewritten to `Data#data.inner` by the body-level `self.X` →
    // `Data#data.X` substitution, leaving the call as
    // `Data#data.inner.bump(...)` — invalid Erlang (no
    // method-call-on-value syntax). For a domain field whose
    // initializer is `@@OtherSys()` the field holds a Pid, so the
    // correct Erlang shape is `othersys:bump(Data#data.inner, ...)`
    // (module-qualified call passing the Pid as the first arg).
    //
    // Walk `system.domain` for cross-system fields (those whose
    // `initializer_text` starts with `@@<Name>(`) and rewrite each
    // dot-call site at the file-text level. Same `defined_systems`
    // pattern other backends use for type/typed-field lowering, but
    // applied to call sites instead of field types.
    let mut cross_sys_fields: Vec<(String, String)> = Vec::new();
    for dv in &system.domain {
        let init = match &dv.initializer_text {
            Some(t) => t.trim(),
            None => continue,
        };
        if let Some(rest) = init.strip_prefix("@@") {
            if let Some(paren) = rest.find('(') {
                let sys_name = &rest[..paren];
                if !sys_name.is_empty() {
                    cross_sys_fields.push((dv.name.clone(), to_snake_case(sys_name)));
                }
            }
        }
    }
    for (field_name, sys_module) in &cross_sys_fields {
        // Match `Data#data.field.` and `Data<digits>#data.field.` —
        // the latter form arises when a handler chains multiple
        // statements (per-statement chaining renames the record
        // variable Data1, Data2, …). The rewrite preserves the
        // original variable name in the output receiver.
        let suffix = format!("#data.{}.", field_name);
        let mut out = String::with_capacity(code.len());
        let mut cursor = 0;
        let bytes = code.as_bytes();
        while cursor < bytes.len() {
            // Find the next `Data` token followed by optional digits
            // followed by the suffix.
            let next = code[cursor..].find("Data");
            let rel = match next {
                Some(r) => r,
                None => break,
            };
            let abs = cursor + rel;
            // Token-boundary check: previous char must not be an
            // identifier char (so we don't match inside `MyData`).
            if abs > 0 {
                let prev = bytes[abs - 1];
                if prev.is_ascii_alphanumeric() || prev == b'_' {
                    out.push_str(&code[cursor..abs + 4]);
                    cursor = abs + 4;
                    continue;
                }
            }
            // Walk past optional digits after `Data`.
            let mut var_end = abs + 4;
            while var_end < bytes.len() && bytes[var_end].is_ascii_digit() {
                var_end += 1;
            }
            // Check for the field suffix.
            if !code[var_end..].starts_with(&suffix) {
                out.push_str(&code[cursor..var_end]);
                cursor = var_end;
                continue;
            }
            let var_name = &code[abs..var_end]; // "Data" or "Data1" etc.
            out.push_str(&code[cursor..abs]);
            // Find the method name (identifier) immediately after the suffix.
            let method_start = var_end + suffix.len();
            let mut method_end = method_start;
            while method_end < bytes.len()
                && (bytes[method_end].is_ascii_alphanumeric() || bytes[method_end] == b'_')
            {
                method_end += 1;
            }
            if method_end == method_start || method_end >= bytes.len() || bytes[method_end] != b'('
            {
                // Not a method call (e.g. just a field read). Pass through.
                out.push_str(&code[abs..method_end]);
                cursor = method_end;
                continue;
            }
            let method = &code[method_start..method_end];
            // Find matching `)` for this call's args.
            let args_open = method_end;
            let mut depth: i32 = 1;
            let mut p = args_open + 1;
            while p < bytes.len() && depth > 0 {
                match bytes[p] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                p += 1;
            }
            if depth != 0 {
                // Unbalanced — leave as-is.
                out.push_str(&code[abs..p.min(bytes.len())]);
                cursor = p;
                continue;
            }
            let args_inner = &code[args_open + 1..p - 1];
            let args_inner_trim = args_inner.trim();
            let receiver = format!("{}#data.{}", var_name, field_name);
            if args_inner_trim.is_empty() {
                out.push_str(&format!("{}:{}({})", sys_module, method, receiver));
            } else {
                out.push_str(&format!(
                    "{}:{}({}, {})",
                    sys_module, method, receiver, args_inner
                ));
            }
            cursor = p;
        }
        out.push_str(&code[cursor..]);
        code = out;
    }

    // Underscore-prefixed action / operation names must be quoted in
    // Erlang because `_<name>` is reserved for ignored bindings, not
    // a bare-atom function name. The function declaration is already
    // emitted via `erlang_op_name` (which quotes), but call sites in
    // user passthrough text (`_inc(Data, ...)` written in handler or
    // operation bodies) preserve the unquoted form. Walk the emitted
    // text once and rewrite each occurrence of `<name>(` to
    // `'<name>'(` for every user action/op that starts with `_`.
    let mut underscore_names: Vec<String> = Vec::new();
    for action in &system.actions {
        if action.name.starts_with('_') {
            underscore_names.push(action.name.clone());
        }
    }
    for op in &system.operations {
        if op.name.starts_with('_') {
            underscore_names.push(op.name.clone());
        }
    }
    for name in &underscore_names {
        let needle = format!("{}(", name);
        let replacement = format!("'{}'(", name);
        // Word-boundary check: previous char must not be alphanumeric
        // or `_`, and must not be a single quote (already-quoted
        // form). Stops bogus matches like `Data#data._inc(` or
        // `'_inc(` (already quoted).
        let mut out = String::with_capacity(code.len());
        let mut cursor = 0;
        while cursor < code.len() {
            let rel = match code[cursor..].find(&needle) {
                Some(r) => r,
                None => break,
            };
            let abs = cursor + rel;
            let prev_ok = if abs == 0 {
                true
            } else {
                let prev = code.as_bytes()[abs - 1];
                !(prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'\'')
            };
            out.push_str(&code[cursor..abs]);
            if prev_ok {
                out.push_str(&replacement);
            } else {
                out.push_str(&needle);
            }
            cursor = abs + needle.len();
        }
        out.push_str(&code[cursor..]);
        code = out;
    }

    // Wrap in a NativeBlock — the assembler will stitch prolog + this + epilog
    CodegenNode::NativeBlock { code, span: None }
}
