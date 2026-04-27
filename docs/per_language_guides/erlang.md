# Per-Language Guide: Erlang

Frame's Erlang backend is the most idiomatically distinct of the 17
targets. Where the other 16 backends share a broadly C-family default
(mutable state on a struct/object, `while` loops, dot-calls), Erlang
runs on `gen_statem` (OTP), with immutable records, tail-position
recursion, and PID-based message passing. The framec output reflects
this — and so do the patterns you write in `.ferl` Frame source.

This guide documents the Erlang-specific idioms, constraints, and
divergences from the cross-target default. It assumes you are already
familiar with Frame's core syntax (`@@system`, `interface:`,
`machine:`, `domain:`, transitions, handlers).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. For the
self-call guard divergence specifically, see
`docs/erlang_alignment_requirements.md`.

---

## Foundation: `gen_statem`

A Frame system targeting Erlang generates a single `.erl` module that
implements OTP's `gen_statem` behaviour. The state-machine semantics
that on other backends are emitted as a hand-written kernel +
compartment + dispatch loop are, in Erlang, expressed via
`gen_statem`'s own state-name + state-data shape.

```erlang
-module(counter).
-behaviour(gen_statem).
-export([start_link/0, ...]).

start_link() ->
    gen_statem:start_link(?MODULE, [], []).

init([]) ->
    {ok, s0, #data{frame_current_state = s0, n = 0}}.

callback_mode() -> [state_functions, state_enter].
```

The Frame state machine's "current state" maps to `gen_statem`'s state
name; Frame's compartment / domain / context fields map to fields on
the `-record(data, {...})`. Transitions emit `{next_state, NewState,
NewData}` tuples.

**What this means in practice:**

- You get OTP supervision, hot-code reload, and `sys` introspection
  for free — `gen_statem:start_link/3` returns `{ok, Pid}` and the
  process plugs into supervisor trees the OTP way.
- The Frame runtime's `__kernel`, `_state_<S>`, and dispatch helpers
  do not exist as separate functions in the Erlang output — the
  callbacks are the dispatch.
- Some Frame mechanisms (the self-call transition guard, the
  exit cascade) are implemented using `gen_statem`-native primitives
  rather than the runtime patterns used on other backends.

---

## Loop idioms — Frame has two; only one works on Erlang

Frame source can express iteration in two distinct ways:

**Idiom 1 — imperative `while`** (target passthrough). Works on every
backend with a `while` keyword: Python, JS, TS, Java, Kotlin, Swift,
C#, Dart, Go, C, C++, Rust, PHP, Ruby, Lua, GDScript.

```frame
work() {
    var n: int = 0
    while n < 10 {
        n = n + 1
    }
}
```

The `while` block lowers to whatever `while` means in the target — for
Python it is `while n < 10:`, for C it is `while (n < 10) {`, etc. It
is pure native passthrough; framec does not parse the loop body.

**Idiom 2 — state-flow loop** (state-machine self-transition). Works
on every backend, including Erlang. Iteration becomes a
state-machine self-transition that re-fires the same state until a
terminal condition is met.

```frame
$Counting {
    $> {
        if $.n >= 10 {
            -> $Done
        }
    }

    tick() {
        $.n = $.n + 1
        -> $Counting     // re-fire enter; loops via cascade
    }
}
```

Erlang has no `while` keyword — the language is functional and
iteration is expressed via tail-position recursion. There is no
sensible target-passthrough mapping for a Frame `while` block in
Erlang, so **idiom 1 simply does not compile under Erlang**.

The four control-flow fixtures named `while_*.ferl` in the test
matrix are state-flow ports of the corresponding Python/JS/etc.
fixtures: same observable contract, expressed via Frame's
state-machine primitives instead of a native `while`.

**Rule of thumb:** if you are writing Frame source intended to compile
across all 17 backends, prefer idiom 2 by default. If you write
idiom 1 for clarity in C-family targets, you are responsible for
authoring an Erlang variant that uses idiom 2.

---

## One `@@system` per file

`gen_statem` modules are *modules* — there is one `-module(...)`
declaration per `.erl` file, and the module name must match the
filename. Erlang's compilation model gives no way to put two modules
in one file.

This means `.ferl` source must contain exactly one `@@system`. Two or
more `@@system` blocks in a single `.ferl` file are rejected at the
framec stage with **E431** ("Erlang requires one system per file").

Cross-system composition is still supported — you split the systems
across separate `.ferl` files, generate them separately, and
`erlc`-compile the result as a multi-module application. The
framepiler matrix harness today expects one `.f<ext>` per case, so
multi-system Erlang fixtures are not exercised end-to-end in matrix
runs (a harness convention limitation, not a framec gap). Manually
compiling and running a two-module pair (`counter.erl + driver.erl`,
generated separately by framec from separate `.ferl` sources) works
out of the box.

---

## Cross-system instantiation: `inner = @@Counter()` lowers to PID wiring

On dynamic / brace-family targets, a domain field initialized with a
tagged `@@SubSystem()` constructor lowers to direct instantiation:

```frame
domain:
    var counter: Counter = @@Counter()
```

```python
self.counter = Counter()      # Python
```

Erlang's actor model has no shared-memory object instances — the only
way to "embed" another `gen_statem` is to spawn it and hold its
PID. Framec lowers `@@Counter()` to:

```erlang
counter:start_link(args)      % returns {ok, Pid}
```

But `start_link/N` returns the OTP `{ok, Pid}` tuple, and the field
on `#data{}` needs the bare `Pid` for `gen_statem:call/2` to work.
Framec inserts an `element(2, …)` unwrap on the field value:

```erlang
#data{counter = element(2, counter:start_link([]))}
```

The user-facing `start_link/N` API still returns `{ok, Pid}` so
drivers and supervisors that pattern-match on the OTP convention
keep working — only the embedded reference is unwrapped to a bare
PID.

**Cross-system call rewrite.** Frame's idiomatic `self.inner.bump()`
(read: "send the `bump` event to the embedded `inner` system") on
other targets becomes a dot-call. On Erlang, the existing
`self.X` → `Data#data.X` substitution is already in place, but the
result `Data#data.inner.bump()` is not valid Erlang (parameterized
modules were removed years ago). Framec's Erlang codegen runs a
post-pass that walks `system.domain` for cross-system fields
(initializers starting with `@@<Name>(`) and rewrites every
`Data#data.<field>.<method>(args)` call to:

```erlang
<sys_module>:<method>(Data#data.<field>, args)
```

This produces module-qualified calls into the embedded system's API
exports. Manual paren-depth tracking on the args ensures nested
calls and commas don't break the rewrite.

**You do not need to do anything special** in your Frame source —
`self.inner.bump()` works on Erlang exactly as it does on every
other target. The lowering is invisible.

---

## Self-call transition guard — case-expression, not `_transitioned` flag

The Frame runtime spec requires that after `@@:self.method(...)`, if
the call caused a transition, the rest of the calling handler must
not run. On 16 of 17 backends, framec emits an explicit guard:

```python
# 16 backends emit this shape (Python shown):
self.inner_method()
if self._context_stack[-1]._transitioned:
    return
do_the_post_call_work()
```

The runtime sets `_transitioned = True` whenever a transition fires,
and the guard short-circuits the calling handler.

**Erlang implements the same functional contract via a different
mechanism** — `gen_statem`'s own state-data flow. After
`frame_dispatch__` returns, a `case`-expression on
`Data#data.frame_current_state` short-circuits via
`{next_state, NewState, NewData, ...}` if the state changed:

```erlang
case Data2#data.frame_current_state of
    s0 -> [post-call code];
    _  -> {next_state, Data2#data.frame_current_state, Data2}
end
```

Implemented in `erlang_wrap_self_call_guards`
(`framec/.../erlang_system.rs:1411-1589`); codegen's generic
`generate_self_call_guard` returns empty for Erlang
(`framec/.../frame_expansion.rs:4752`).

**Why the divergence?** Erlang is functional. A mutable
`_transitioned` flag on a record requires a re-bind through every
call layer — the case-expression on the post-call state name is
both more idiomatic and structurally simpler in OTP.

The contract is functional, not structural. The transition-guard
test (test 53) passes on all 17 backends, including Erlang — only
the mechanism differs. If you are writing Frame source, you do not
notice the divergence. See `docs/erlang_alignment_requirements.md`
for the catalogue of trade-offs and the deferred path to
structural alignment (parked because the current divergence is
recommended on idiomaticity grounds).

---

## HSM cascade — fully spec-conformant

`gen_statem`'s built-in `state_enter` mode fires `enter` on the
*leaf* state only, with no symmetric `state_exit` callback. Frame's
runtime spec requires HSM transitions to fire the full
exit-leaf-to-LCA cascade followed by the full enter-LCA-to-leaf
cascade.

For most of v4, Erlang's HSM rows in the capability matrix carried
footnote `[d]` documenting "leaf only" enter cascade. **As of
2026-04-26, this divergence is closed.** Framec now emits the full
cascade for Erlang (commit referenced in
`memory/erlang_hsm_cascade_2026_04_26.md`); the implementation
walks `state_hsm_parents` in `erlang_system.rs` and emits the
appropriate enter / exit calls.

You can rely on the spec contract — HSM transitions on Erlang fire
exit and enter callbacks in the same order as on every other
target.

The forward-transition re-dispatch case (`-> => $State`) was the
final remaining divergence and was implemented via `gen_statem`'s
`{next_event, ...}` action (commit referenced in
`memory/erlang_complete_2026_04_26.md`). The Erlang capability
footnote `[d]` was removed.

**The only remaining intentional divergence is the self-call guard
mechanism (footnote `[i]`)** — see the previous section.

---

## Async: actor model replaces async/await

Frame's `async` annotation lowers to language-native async
primitives on most backends — `async def` / `await` (Python, JS),
`CompletableFuture<T>` (Java), `Future<T>` (Dart), etc.

Erlang has no `async` / `await` keyword because the *entire process
model* is asynchronous. A `gen_statem:call/2` is itself a
synchronous request-response over message passing; for async work
you spawn another process and have it send a message back. There
is no point in framec emitting an "async" annotation for Erlang —
every `gen_statem:call` is already an inter-process message.

Capability matrix row "async" for Erlang shows `🚫[h]` with the
footnote: "actor model + selective receive replaces async/await".
This is a language-shape skip, not a framec gap. If you have an
async pattern that you want to express portably, write it as an
embedded sub-system that the parent system delegates to via
`@@:self.subsystem.work()` — the message-passing semantics give
you the asynchronous behavior natively.

---

## Domain fields on the `#data{}` record

Domain fields live on a `-record(data, {...})` definition, threaded
through every `gen_statem` callback as the third element of the
`{next_state, State, Data}` tuple.

```frame
domain:
    var n: int = 0
    var name: str = "alice"
```

```erlang
-record(data, {
    frame_current_state,
    frame_event_args,
    frame_exit_args,
    n = 0,
    name = <<"alice">>,
    _transitioned = false
}).
```

Reads use `Data#data.field`; writes use `Data2 = Data#data{field =
NewValue}`. The runtime's record-update syntax is what you'd write
in idiomatic Erlang — there's no hidden mutability.

`@@:return = expr` writes to `Data#data.frame_event_args` (the
return slot), which the wrapper extracts after the dispatch
completes.

**Frame's `: type` annotations are documentation on Erlang.**
Erlang has no compile-time type checker (Dialyzer is a separate
pass that runs on the generated `.erl`). The wrapper always emits
a return regardless of whether the source declared `: type` —
the dynamic-target return-value contract applies. See
`docs/frame_runtime.md` "Return values across target languages."

---

## Comments and the Oceans Model

Frame's "Oceans Model" — native code passes through verbatim —
applies to Erlang the same way it applies to every other backend.
The comment leader for native code is `%` (Erlang's line-comment),
not `#` or `//`.

```frame
@@target erlang
%%
%% This whole module-prolog block is native Erlang and passes
%% through to the .erl output unchanged. Use Erlang comment
%% syntax for prolog comments.
%%

@@system Counter {
    machine:
        % Comments inside @@system blocks before / after declarations
        % are also written in target-native syntax — Erlang's `%`.
        $Counting {
            tick() { ... }
        }
}
```

Section-level leading comments (above interface methods, domain
vars, states, actions, operations, handlers) are preserved into
the generated `.erl` output as native comment blocks attached to
the corresponding generated declaration.

---

## Tooling: erlc, escript, and the test harness

The Erlang test matrix uses an escript-based sidecar driver
convention. A `.ferl` test source compiles via framec to a single
`.erl` module; the harness pairs it with a separate
`run_test.escript` that contains the test driver (`init` →
`gen_statem:call` sequence → assertions).

This pattern exists because:

1. Erlang has no in-language `assert` / test runner equivalent to
   Python's `assert` or JUnit's `@Test` — you express assertions
   as patterns on `gen_statem:call/2` results inside the escript
   driver.
2. Test cases that exercise multiple systems would require
   multi-file fixtures; the escript driver is the natural place
   to coordinate that across modules in matrix runs.

If you are writing your own Erlang Frame project outside the
matrix, you do not need the escript harness — you can use any
Erlang test runner (`eunit`, `common_test`, `proper`) on the
generated `.erl` modules directly.

See `framepiler_test_env/docker/runners/erlang_batch.sh` for the
matrix-side mechanism.

---

## Idiomatic patterns and common gotchas

**Use lowercase atoms for state names in Frame source.** Frame's
state names (`$StateName`) lower to Erlang atoms via
`snake_case` conversion: `$Counting` → `counting`. If you use
mixed case in Frame, the generated atom will be lowercase
canonical. For predictability, write Frame state names so the
canonical lowering matches the atom you'd hand-write.

**`@@:return = expr` is required for non-`init` events.** On
brace-family targets it's possible to write `var x: int = ...`
inside a handler and use it without ever returning; the wrapper
will return `null`/`undefined` from the dynamic target's slot.
On Erlang the equivalent always works, but if you intend the
caller to receive a value, explicitly set `@@:return = expr`.
The slot defaults to `undefined` on Erlang otherwise — usually
not what you want.

**No mutable closures in handler bodies.** Erlang has no closure
mutability — `fun() -> X = X + 1, X end.` does not work. Frame's
E407 validator catches Frame statements inside any nested-scope
syntax (including Erlang `fun()` constructions) and rejects with
"Frame statements inside nested scopes are not supported".
Lifting state to domain or to a sub-state is the workaround;
this matches v4's portability rules across the other 16 backends.

**Records are not maps.** Erlang records desugar to tagged tuples
at compile time — `Data#data.field` is a tuple-element access,
not a map lookup. If you write Frame source that expects
JSON-shaped data, you'll want maps (`#{}`) on the Erlang side,
which means writing the field type as `: map` and accessing via
`maps:get/2` in native passthrough. Frame doesn't auto-convert.

**`io:format/2` on the Erlang side, not `print(...)`.** Frame's
`@@:print(...)` lowers to the target's print primitive on most
backends; Erlang's print is `io:format/2` with a format string,
not a single positional argument. Use native Erlang
passthrough for print:

```frame
$Counting {
    tick() {
        % Native Erlang passthrough — io:format works as expected.
        io:format("tick: ~p~n", [self.n])
    }
}
```

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Erlang-specific footnotes `[a]`–`[m]`.
- `docs/erlang_alignment_requirements.md` — catalogue of the
  self-call guard divergence and the deferred structural
  alignment path. Recommended reading if you are debugging
  unexpected post-self-call behavior.
- `tests/common/positive/control_flow/while_*.ferl` —
  state-flow loop idiom canonical examples.
- `tests/common/positive/cross_backend/53_transition_guard.ferl`
  — Erlang implementation of the self-call transition guard
  test.
- `framec/src/frame_c/compiler/codegen/erlang_system.rs` — the
  Erlang backend codegen. The cross-system instantiation
  rewrite, `expand_tagged_in_domain_erlang`, and
  `erlang_wrap_self_call_guards` live here.
