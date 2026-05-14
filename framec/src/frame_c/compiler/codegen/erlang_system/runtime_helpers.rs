//! Static framework runtime helpers emitted into every generated
//! Erlang module.
//!
//! These are the gen_statem-shaped primitives that Frame's
//! handler-body emission depends on at runtime:
//!
//! - `frame_arg_at__/2` — safe positional list accessor used by
//!   enter/exit/state-arg unpacking.
//! - `frame_transition__/7` — orchestrates exit dispatch → arg
//!   passing → gen_statem state transition with a user-supplied
//!   reply value. Threaded by `@@:return + transition` so the
//!   user's @@:return survives the state switch.
//! - `frame_forward_transition__/7` — variant that adds a
//!   `next_event` action so the in-flight event is re-dispatched
//!   to the destination state (`-> => $State` Step 24).
//! - `frame_unwrap_forward__/1` + `frame_extract_reply__/1` —
//!   flatten an HSM parent-forward's gen_statem return tuple into
//!   `{UpdatedData, NextStateOrUndefined, ParentReplyValue}` so a
//!   child's `=> $^` followed by additional statements compiles
//!   cleanly (and propagates the parent's @@:return value).
//! - `frame_exit_dispatch__/1` — RFC-0019 leaf-dispatched `<$`
//!   table that routes to `frame_exit__<state>/1` for any state
//!   that has an exit handler. Ancestors run only on `=> $^`
//!   (lowered to a direct call to the parent's exit helper).
//!
//! All emission is verbatim string-append into the shared `code`
//! buffer. The only per-system content is `frame_exit_dispatch__`
//! which iterates `system.machine.states` to emit one match arm
//! per state with an exit handler.

use super::super::codegen_utils::to_snake_case;
use crate::frame_c::compiler::frame_ast::SystemAst;

pub(super) fn emit_runtime_helpers(code: &mut String, system: &SystemAst) {
    // Positional argument accessor — safe `lists:nth/2` that returns
    // `undefined` if the list is too short or N is out of range.
    // Used by enter/exit/state arg unpacking. The 1-based index N
    // matches Erlang's list convention; framec emits N = i+1 for
    // a 0-based parameter index `i`.
    //
    // We special-case N=1 with a list-pattern match for the common
    // single-arg case (faster than calling lists:nth/2). The
    // multi-clause function form keeps each clause cheap.
    code.push_str("frame_arg_at__(_, []) -> undefined;\n");
    code.push_str("frame_arg_at__(N, _) when N < 1 -> undefined;\n");
    code.push_str("frame_arg_at__(1, [H | _]) -> H;\n");
    code.push_str("frame_arg_at__(N, L) when length(L) >= N -> lists:nth(N, L);\n");
    code.push_str("frame_arg_at__(_, _) -> undefined.\n\n");

    // Frame transition helper — orchestrates exit → arg passing → gen_statem transition.
    // The trailing `ReplyVal` argument carries the @@:return value the
    // handler set BEFORE the transition; codegen passes either the
    // SSA-renamed `__ReturnVal_K` from the body's most recent
    // `@@:return` or the atom `ok` when the handler had no return value.
    // Without this parameter, transitioning would force-replace the
    // user's return with `ok`, dropping `@@:return + transition` values.
    code.push_str(
        "frame_transition__(TargetState, Data, ExitArgs, EnterArgs, StateArgs, From, ReplyVal) ->\n",
    );
    code.push_str("    Data1 = Data#data{frame_exit_args = ExitArgs},\n");
    code.push_str("    Data2 = frame_exit_dispatch__(Data1),\n");
    code.push_str("    Data3 = Data2#data{frame_enter_args = EnterArgs, frame_state_args = StateArgs, frame_current_state = TargetState},\n");
    code.push_str("    {next_state, TargetState, Data3, [{reply, From, ReplyVal}]}.\n\n");

    // Forward transition helper — same exit/enter cascade as
    // `frame_transition__`, plus a `next_event` action that
    // re-dispatches the originating event to the new leaf after
    // gen_statem fires its `state_enter` callback there.
    //
    // Per docs/frame_runtime.md Step 24, `-> => $State` performs a
    // full transition (cascade exit, switch, cascade enter) AND
    // re-dispatches the in-flight event so the destination handles
    // it from scratch in the new state. Other backends model this
    // via a `forward_event` field on the destination compartment;
    // gen_statem's natural mechanism for "process this event next"
    // is the `next_event` enter-action, which is enqueued ahead of
    // any pending external events. We omit `{reply, From, ok}` —
    // the re-dispatched event's handler in the destination state
    // is responsible for producing the reply.
    code.push_str(
        "frame_forward_transition__(TargetState, ForwardEvent, Data, ExitArgs, EnterArgs, StateArgs, From) ->\n",
    );
    code.push_str("    Data1 = Data#data{frame_exit_args = ExitArgs},\n");
    code.push_str("    Data2 = frame_exit_dispatch__(Data1),\n");
    code.push_str("    Data3 = Data2#data{frame_enter_args = EnterArgs, frame_state_args = StateArgs, frame_current_state = TargetState},\n");
    code.push_str(
        "    {next_state, TargetState, Data3, [{next_event, {call, From}, ForwardEvent}]}.\n\n",
    );

    // HSM parent-forward unwrap. When a child's `=> $^` has post-forward code
    // (e.g., `=> $^; self.x = self.x + 1`), we can't emit the parent call as
    // a tail call — the post-forward statements would be lost. The body
    // processor instead binds:
    //   `{DataN, __FwdNextN, __FwdReplyN} = frame_unwrap_forward__(ParentCall)`.
    // This helper flattens the parent's gen_statem return tuple into a
    // 3-tuple `{UpdatedData, NextStateOrUndefined, ParentReplyValue}` so the
    // child can:
    //   - continue threading Data through its remaining statements;
    //   - honor whatever transition (if any) the parent performed; and
    //   - propagate the parent's `[{reply, From, V}]` value as the child's
    //     own reply (instead of hardcoding `ok`, which dropped the parent
    //     handler's `@@:return` write across the forward).
    // Matches the 16-backend consensus that `=> $^` returns whatever the
    // parent's handler set in `@@:return`.
    code.push_str(
        "frame_unwrap_forward__({keep_state, D, Actions}) -> {D, undefined, frame_extract_reply__(Actions)};\n",
    );
    code.push_str("frame_unwrap_forward__({keep_state, D}) -> {D, undefined, ok};\n");
    code.push_str(
        "frame_unwrap_forward__({next_state, NS, D, Actions}) -> {D, NS, frame_extract_reply__(Actions)};\n",
    );
    code.push_str("frame_unwrap_forward__({next_state, NS, D}) -> {D, NS, ok}.\n\n");
    code.push_str("frame_extract_reply__([{reply, _From, V} | _]) -> V;\n");
    code.push_str("frame_extract_reply__([_ | Rest]) -> frame_extract_reply__(Rest);\n");
    code.push_str("frame_extract_reply__([]) -> ok.\n\n");

    // Exit handler dispatch — RFC-0019: `<$` is a leaf-dispatched event.
    // On transition, only the *leaf* (the state being left) fires its
    // `<$` here. Ancestors run their `<$` solely when the leaf forwards
    // (`=> $^` inside a `<$` body lowers to `frame_exit__<parent>(Data)`).
    // No ancestor chain walk.
    code.push_str("frame_exit_dispatch__(Data) ->\n");
    code.push_str("    case Data#data.frame_current_state of\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            if state.exit.is_none() {
                // No exit handler — falls through to the `_ -> Data` arm.
                continue;
            }
            let sname = to_snake_case(&state.name);
            code.push_str(&format!(
                "        {} -> frame_exit__{}(Data);\n",
                sname, sname
            ));
        }
    }
    code.push_str("        _ -> Data\n");
    code.push_str("    end.\n\n");
}
