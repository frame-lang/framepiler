//! Persistence (save_state / load_state) emission for Erlang
//! gen_statem.
//!
//! Frame's persist contract — `@@[persist]` on the system, with
//! optional `@@[save(name)]` / `@@[load(name)]` operation
//! attributes to rename the framework methods — emits two
//! module-level functions when present:
//!
//! - `<save>/1` (default name: `save_state`) — takes a Pid,
//!   returns an Erlang External Term Format binary that
//!   round-trips every Erlang term losslessly.
//! - `<load>/1` (default name: `load_state`) — takes an ETF
//!   binary, returns `{ok, Pid}` for a freshly-spawned
//!   gen_statem with the persisted state restored.
//!
//! Wire format: `term_to_binary({State, PersistedFields})` where
//! `PersistedFields` is an Erlang map. ETF is chosen over JSON
//! because it natively represents atoms, tuples, char-list
//! strings, and any-keyed maps without lossy tagging
//! conventions — and it's the same format mnesia, dets, ets,
//! and distributed Erlang use. See the Erlang per-language
//! guide for the full rationale.
//!
//! Architectural notes:
//!
//! - **Erlang persist is module-level, not instance-method.**
//!   The gen_statem actor model means save_state takes a Pid
//!   and load_state returns a fresh Pid — you can't mutate an
//!   actor in place. Both are module-level functions
//!   naturally; the `@@[save(name)]` / `@@[load(name)]`
//!   attributes only rename them.
//! - **Nested `@@SystemName` fields recurse.** A domain field
//!   initialised as `inner: T = @@T()` is detected by the
//!   `@@`-prefix probe on `initializer_text`. save_state
//!   delegates to the child module's save_state (returning an
//!   opaque binary that embeds in the parent's map);
//!   load_state hands that binary back to the child's
//!   load_state to spawn a fresh child process.
//! - **`@@[no_persist]` fields are dropped on save** and
//!   absent from `Persisted` on load. The freshly-constructed
//!   `#data{}` record's compile-time defaults (set by the
//!   user's `domain:` initializer text) populate them.

use super::super::codegen_utils::to_snake_case;
use super::ERLANG_COMPARTMENT_CONTEXT_FIELDS;
use crate::frame_c::compiler::frame_ast::SystemAst;

/// Emit `save_state/1` + `load_state/1` (or their user-renamed
/// equivalents) and patch the `-export` list. No-op when
/// `@@[persist]` is absent.
pub(super) fn emit_persistence_methods(code: &mut String, system: &SystemAst) {
    if system.persist_attr.is_none() {
        return;
    }
    // RFC-0012 amendment: respect user-named @@[save] / @@[load]
    // operations. Erlang's persist is a documented design
    // exclusion for the instance-method shape: the gen_statem
    // actor model means save_state takes a Pid, load_state
    // returns a fresh Pid (you can't mutate an actor in place
    // — Pids are immutable handles, the process holds state).
    // Both are module-level functions naturally; we just rename
    // them under the user's chosen op names. The function shapes
    // stay the same.
    let save_method_name = system
        .save_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "save_state".to_string());
    let load_method_name = system
        .load_op_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "load_state".to_string());

    // Collect domain fields with nested-system metadata. For
    // each domain var with `inner: T = @@T()` shape, store
    // (field_name, Some(child_module)) so save_state can
    // recursively serialize the child's gen_statem state via
    // its own save_state, and load_state can spawn a fresh
    // child process via the child's load_state.
    let mut domain_fields: Vec<(String, Option<String>)> = Vec::new();
    for var in &system.domain {
        // RFC-0016.1: `@@[no_persist]` fields are transient — skip
        // adding them to the save/load maps. The Erlang #data{}
        // record still declares the field (with its initializer
        // text as the compile-time default), so on load_state the
        // re-built record gets the default value for the missing
        // key — matching the RFC's "leave at the `domain:` default"
        // contract.
        if var.attributes.iter().any(|a| a.name == "no_persist") {
            continue;
        }
        let child_module = match &var.initializer_text {
            Some(t) => {
                let t = t.trim();
                t.strip_prefix("@@").and_then(|rest| {
                    rest.find('(').and_then(|paren| {
                        let sys_name = &rest[..paren];
                        if sys_name.is_empty() {
                            None
                        } else {
                            Some(to_snake_case(sys_name))
                        }
                    })
                })
            }
            None => None,
        };
        domain_fields.push((var.name.clone(), child_module));
    }

    // The full field list for non-domain serialization: per-state
    // state-vars + modal stack + canonical compartment-context
    // fields. Same as before, just split out from domain.
    let mut other_fields: Vec<String> = Vec::new();
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            let state_prefix = to_snake_case(&state.name);
            for sv in &state.state_vars {
                other_fields.push(format!("sv_{}_{}", state_prefix, sv.name));
            }
        }
    }
    other_fields.push("frame_stack".to_string());
    for field in ERLANG_COMPARTMENT_CONTEXT_FIELDS {
        other_fields.push(field.to_string());
    }

    let total_fields = domain_fields.len() + other_fields.len();
    let _ = total_fields; // retained for clarity; not needed in the ETF path

    // save_state/1 — Erlang's wire format is the Erlang External
    // Term Format (term_to_binary / binary_to_term). Rationale:
    // an Erlang programmer expects save/load to faithfully
    // round-trip every Erlang term — atoms, tuples, char-list
    // strings, maps with any key type, binaries. JSON cannot
    // represent any of those except as lossy or tagged
    // conventions. ETF is the standard library, zero-dep,
    // documented serialization format that mnesia, dets, ets and
    // distributed Erlang all use. Cross-language consumers who
    // need to inspect the payload can use an ETF parser (one
    // exists in every major language). See the Erlang per-language
    // guide for the full rationale.
    //
    // Shape: term_to_binary({State, PersistedFields}). PersistedFields
    // is a map containing only the fields we wish to persist —
    // @@[no_persist] domain fields are omitted, so on load the
    // freshly-constructed #data{} record's compile-time defaults
    // (set by the user's `domain` initializers) populate them.
    // Nested @@SystemName fields recurse: the child's save_state
    // returns a binary, which embeds as an opaque binary value;
    // load_state hands that binary to the child's load_state to
    // spawn a fresh child process.
    code.push_str(&format!("{}(Pid) ->\n", save_method_name));
    code.push_str("    {State, Data} = sys:get_state(Pid),\n");
    code.push_str("    Persisted = #{\n");
    let mut entries: Vec<String> = Vec::new();
    for (field, child_module) in &domain_fields {
        match child_module {
            Some(child) => {
                entries.push(format!(
                    "        {field} => case Data#data.{field} of\n\
                     \x20                       undefined -> undefined;\n\
                     \x20                       ChildPid_{field} ->\n\
                     \x20                           {child}:save_state(ChildPid_{field})\n\
                     \x20                   end",
                    field = field,
                    child = child
                ));
            }
            None => {
                entries.push(format!("        {field} => Data#data.{field}", field = field));
            }
        }
    }
    for field in &other_fields {
        entries.push(format!("        {field} => Data#data.{field}", field = field));
    }
    code.push_str(&entries.join(",\n"));
    code.push_str("\n    },\n");
    code.push_str("    term_to_binary({State, Persisted}).\n\n");

    // load_state/1 — takes an ETF binary, reconstructs the
    // gen_statem. Nested-system field values are themselves
    // child ETF binaries which load_state hands off to the
    // child module's load_state to spawn a fresh child Pid.
    // @@[no_persist] fields are absent from Persisted, so the
    // freshly-constructed #data{} record picks up their
    // compile-time defaults (the `domain:` initializer text).
    code.push_str(&format!("{}(Bin) ->\n", load_method_name));
    code.push_str("    {State, Persisted} = binary_to_term(Bin, [safe]),\n");
    code.push_str("    Data = #data{\n");
    let mut entries: Vec<String> = Vec::new();
    for (field, child_module) in &domain_fields {
        match child_module {
            Some(child) => {
                entries.push(format!(
                    "        {field} = case maps:get({field}, Persisted, undefined) of\n\
                     \x20                       undefined -> undefined;\n\
                     \x20                       ChildBin_{field} ->\n\
                     \x20                           {{ok, ChildPid_{field}}} = {child}:load_state(ChildBin_{field}),\n\
                     \x20                           ChildPid_{field}\n\
                     \x20                   end",
                    field = field,
                    child = child
                ));
            }
            None => {
                entries.push(format!(
                    "        {field} = maps:get({field}, Persisted, undefined)",
                    field = field
                ));
            }
        }
    }
    for field in &other_fields {
        // frame_stack / frame_state_args / frame_enter_args default
        // to [] on a fresh system; if the saved blob predates a
        // field, fall back to []. (Today no such field exists; this
        // is forward-compatibility insurance.)
        let dflt = if field == "frame_stack"
            || field == "frame_state_args"
            || field == "frame_enter_args"
        {
            "[]"
        } else {
            "undefined"
        };
        entries.push(format!(
            "        {field} = maps:get({field}, Persisted, {dflt})",
            field = field,
            dflt = dflt
        ));
    }
    code.push_str(&entries.join(",\n"));
    code.push_str("\n    },\n");
    code.push_str("    {ok, Pid} = gen_statem:start_link(?MODULE, [], []),\n");
    code.push_str("    sys:replace_state(Pid, fun(_) -> {State, Data} end),\n");
    code.push_str("    {ok, Pid}.\n\n");

    // Add save/load to exports under their user-named (or
    // legacy-default) names.
    let save_export = format!(
        "-export([{}/1, {}/1]).\n",
        save_method_name, load_method_name
    );
    // Find position after the last -export line
    if let Some(pos) = code.rfind("-export([callback_mode/0") {
        if let Some(newline) = code[pos..].find('\n') {
            let insert_pos = pos + newline + 1;
            code.insert_str(insert_pos, &save_export);
        }
    }
}
