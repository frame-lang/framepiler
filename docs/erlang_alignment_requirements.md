# Erlang structural alignment — self-call transition guard

> **Status: deferred.** Erlang's current self-call transition-guard
> mechanism diverges structurally from the other 16 backends but
> meets the functional contract. This document captures the gap and
> the path to structural alignment so the work is well-scoped if it
> becomes a requirement. It does not implement the change.
>
> **Recommendation: keep the divergence.** Erlang's mechanism is
> idiomatic to OTP and `gen_statem`; structural alignment would
> sacrifice native state-machine semantics for a cosmetic match
> with the other backends. The functional contract — post-call
> code does not execute after a transitioning self-call — holds in
> both forms.

## Background

Across 15 of the 17 framepiler backends, the self-call transition
guard is a two-piece mechanism:

1. **`FrameContext._transitioned: bool`** — a mutable flag, set
   by the kernel on every stacked context after processing a
   transition.
2. **Codegen guard** — `framec/.../frame_expansion.rs:4692-4754`
   `generate_self_call_guard` emits a per-target snippet inserted
   immediately after every `@@:self.method(...)` call site:
   ```python
   self.method(...)
   if self._context_stack[-1]._transitioned: return
   ```

The pattern: call dispatched, kernel ran any queued transition,
flag set on every stacked context, post-call guard returns early
if a transition happened. Same shape in Python, TypeScript,
JavaScript, Rust, C, C++, Java, Kotlin, Swift, C#, Go, PHP, Ruby,
Lua, GDScript.

Erlang takes a different path.

## Current Erlang mechanism — `gen_statem` case-expression

Erlang doesn't carry a `_transitioned` flag. Instead, the
generated handler captures the pre-call state, dispatches via
`frame_dispatch__`, and short-circuits with a case-expression on
the returned `Data` record's `frame_current_state` field.

Implemented in `erlang_wrap_self_call_guards`
(`framec/src/frame_c/compiler/codegen/erlang_system.rs:1411-1589`).
Codegen's `generate_self_call_guard` returns an empty string for
Erlang (`framec/src/frame_c/compiler/codegen/frame_expansion.rs:4752`)
— the wrapping happens in the runtime support layer instead of as
an emitted statement.

### Generated shape

```erlang
{Data2, _} = frame_dispatch__(trigger, [], Data1),
case Data2#data.frame_current_state of
    s0 ->
        % post-call code only runs if state is still s0
        Data3 = Data2#data{...},
        {keep_state, Data3, [{reply, From, ok}]};
    _ ->
        % state changed → short-circuit to gen_statem state transition
        {next_state, Data2#data.frame_current_state, Data2,
         [{reply, From, undefined}]}
end
```

### Why Erlang chose this

- **`gen_statem` ergonomics** — every handler must return a tuple
  shaped like `{keep_state, ...}` or `{next_state, ...}`. The
  case-expression naturally selects between the two.
- **Functional paradigm** — Erlang prefers immutable data records
  and pattern-matching over mutable flags. A `_transitioned` field
  would still be functionally correct but unidiomatic.
- **Single dispatch boundary** — `frame_dispatch__` returns the
  whole `Data` record, so comparing `frame_current_state` before
  and after costs one record-field read and one match. No extra
  state to thread.

## The functional contract holds

The capability matrix (`runtime-capability-matrix.md`, row
"Self-call transition guard") marks Erlang ✅ with footnote `[i]`.
The contract is "post-call code does not run after a transitioning
self-call." Both mechanisms satisfy it.

The promoted test 53 (`tests/common/positive/primary/53_transition_guard.f<ext>`)
asserts the trace lacks `"run:after-self-call"` — a black-box
behavioral assertion that doesn't depend on which mechanism is
used.

## Alignment path (deferred)

Bringing Erlang into structural alignment with the flag-based
backends would require coordinated changes in two places:

### Runtime layer (`erlang_system.rs`)

1. **Add a flag to the `data` record.** The Erlang FrameContext
   surrogate is the `-record(data, {...})` defined around
   `erlang_system.rs` (currently around line 2444). Add
   `_transitioned = false` as a record field, alongside
   `frame_current_state`.

2. **Set the flag in `frame_dispatch__`.** After `frame_dispatch__`
   processes a transition (i.e. `frame_current_state` changes),
   set `_transitioned = true` on every stacked context. The
   data-record threading model means context stack lives within
   `Data`; the kernel-equivalent step that walks the context stack
   needs to set the flag on each entry.

3. **Replace the case-expression wrapping.** `erlang_wrap_self_call_guards`
   (`erlang_system.rs:1411-1589`) currently emits the `case
   Data#data.frame_current_state of <state> -> ...; _ -> {next_state, ...}
   end` shape. Replace with a direct early-return pattern that
   reads the flag:
   ```erlang
   {Data2, _} = frame_dispatch__(...),
   case lists:last(Data2#data._context_stack)#frame_context._transitioned of
       true -> {next_state, Data2#data.frame_current_state, Data2, [...]};
       false -> [post-call code]
   end
   ```
   Or, more idiomatically, an explicit early-return helper.

### Codegen layer (`frame_expansion.rs`)

4. **Replace the empty-string Erlang branch.** Line ~4752 currently
   returns `String::new()` for Erlang. Replace with a per-target
   guard emission that matches the other 16:
   ```rust
   TargetLanguage::Erlang => format!(
       "case lists:last(Data#data._context_stack)#frame_context._transitioned of \
        true -> {{next_state, Data#data.frame_current_state, Data, []}}; \
        false -> ok end,\n"
   ),
   ```
   The exact form depends on whether the Erlang runtime exposes
   `_context_stack` symmetrically and how `Data` flows through the
   handler — both are non-trivial in the `gen_statem` model.

### Test alignment

5. **Update test 53's Erlang variant** (if not already shipping).
   The test is mechanism-agnostic — it asserts the trace shape —
   so the `.ferl` file shouldn't need changes.

## Trade-off table

| Aspect | Current (case-expression) | With `_transitioned` flag |
|---|---|---|
| Idiomaticity in OTP | Native — matches `gen_statem` patterns | Foreign — mutable flag in functional core |
| Performance | Single record-field read + match | Two ops: flag write per kernel transition + flag read per call site |
| Code size | Compact: case-expr replaces guard | Separate flag-set step + per-call-site guard |
| Cross-target consistency | One backend out of step | All 17 structurally aligned |
| Maintainability | Erlang-specific; needs OTP knowledge | Uniform with other backends |
| Risk surface | Tested, shipping, smoke-tested in matrix | New code path; needs full re-verification across HSM, persist, async paths |

Current Erlang wins on idiomaticity, performance, code size, and
risk surface. Alignment wins on cross-target consistency and
maintainability for non-Erlang readers.

## Recommendation

**Keep the current divergence.** The functional contract is met,
the matrix documents it, and Erlang's mechanism is appropriate to
its runtime model. The cost of alignment is high (touching the
data-record threading and the `gen_statem` interaction layer)
relative to the gain (cosmetic structural sameness).

If alignment becomes a requirement (e.g., for a tooling step that
inspects generated code structurally and demands shape parity),
the changes above are well-scoped — runtime layer is the
substantial work; codegen is a small frame_expansion edit.

## References

- `framec/src/frame_c/compiler/codegen/erlang_system.rs:1411-1589`
  — `erlang_wrap_self_call_guards`. The wrap entry point.
- `framec/src/frame_c/compiler/codegen/frame_expansion.rs:4692-4754`
  — `generate_self_call_guard`. Per-target emission table; line
  4752 is the Erlang empty-string branch.
- `framepiler_test_env/docs/runtime-capability-matrix.md` — row
  "Self-call transition guard"; footnote `[i]` is the canonical
  divergence note.
- `framepiler_test_env/tests/common/positive/primary/53_transition_guard.ferl`
  — the Erlang variant of the transition-guard regression test
  (after Phase 5.4 lands).
