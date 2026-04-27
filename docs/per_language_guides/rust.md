# Per-Language Guide: Rust

Rust is the only target where Frame's runtime state-machine model is
expressed entirely through Rust's type system: each state's arguments
become a typed enum variant of `StateContext`, every transition emits
a typed re-bind, and there are zero `unsafe` blocks in the generated
code.

This shape buys you compile-time HSM signature matching and panic-free
ownership of the cascade chain — but it asks more of the Frame source
in return: ownership of cross-system fields is explicit, string
concatenation is not symmetric, and the `await` syntax appears at the
call site (not just at the wrapper boundary).

This guide documents the Rust-specific idioms, constraints, and
patterns. It assumes you are already familiar with Frame's core
syntax and Rust ownership basics.

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Rust is fully
spec-conformant: every capability shows ✅ with no footnotes.

---

## Foundation: typed `StateContext` enum

A Frame system targeting Rust generates a single `.rs` module that
defines:

- A `Frame<SystemName>` struct holding the kernel state, domain
  fields, and the compartment stack.
- A `Compartment` struct with `state` (the current state's name as
  a `&'static str`), `state_context` (a typed enum capturing the
  state's arguments), and `_transitioned: bool`.
- A `StateContext` enum with one variant per state that declares
  arguments. States with no arguments use a unit variant.
- One `_state_<S>(&mut self, e: &mut FrameEvent, compartment: ...)`
  function per state, plus `_s_<S>_hdl_<kind>_<event>` per handler.

```rust
enum StateContext {
    A,                          // no args
    B { x: i32, name: String }, // typed args
}

struct Compartment {
    state: &'static str,
    state_context: StateContext,
    _transitioned: bool,
}
```

The Frame state machine's "current state" maps to the
`compartment.state` string and `compartment.state_context` enum
variant — they are kept in lockstep by codegen. Reading state args
inside a handler decodes the enum:

```rust
fn _s_b_hdl_event_tick(...) {
    if let StateContext::B { ref x, ref name } = compartment.state_context {
        // ... use x, name
    }
}
```

This shape gives you:

- **Compile-time HSM signature checks** — if you transition to a
  state with arguments, the typed enum forces you to supply the
  right shape, full stop.
- **Zero `unsafe`** — even the parent-chain walk during HSM state-arg
  propagation uses `Option<&mut Compartment>` cursor walks, not raw
  pointer casts.
- **No reflection cost** — every handler dispatches via direct
  function call, no `dyn`, no `Box<dyn Any>`, no string lookup.

---

## State arguments propagate through the typed HSM chain

When a transition supplies arguments to an HSM child whose parents
also declare matching parameters, framec emits a nested `if-let`
chain that writes the args through every ancestor's typed
`StateContext` variant:

```rust
if let StateContext::Child { ref mut x, ref mut name } = compartment.state_context {
    *x = new_x;
    *name = new_name.clone();
    if let Some(parent) = compartment.parent_compartment.as_deref_mut() {
        if let StateContext::Parent { ref mut x, ref mut name } = parent.state_context {
            *x = new_x;
            *name = new_name.clone();
        }
    }
}
```

The walk uses `Option<&mut Compartment>` (`as_deref_mut`) — no
`unsafe` casts to raw pointers. The depth of nesting is determined
by `state_hsm_parents` at codegen time, so the generated chain is
exactly as long as the inheritance chain, no shorter, no longer.

**Frame source contract** — HSM parents declare matching parameter
signatures; framec validates the match and rejects mismatched
shapes with **E406**. See test
`tests/common/positive/primary/52_hsm_state_arg_propagation.frs`
for the canonical example asserting both leaf and parent see the
propagated value.

---

## Strings: `String + &String`, not `String + String`

This is the single most common gotcha when writing Frame source for
Rust. Rust's `+` operator on strings consumes the LHS and borrows
the RHS:

```rust
// This works:
let combined = "hello".to_string() + &name;

// This does NOT compile:
let combined = "hello".to_string() + name;       // expected &str, found String
```

Frame source intended to compile cleanly across all 17 backends has
to navigate this. The cross-target portable pattern is to use
`format!`:

```frame
$Active {
    greet(name: String) {
        // Portable for Rust; lowers naturally to f-strings,
        // template literals, etc. on dynamic targets.
        @@:return = format!("hello, {}", name)
    }
}
```

For literal `+` concatenation, expect to write `& self.field` on
Rust:

```frame
$Active {
    log() {
        // Rust-specific — `&` on the borrowed side.
        self.trace = self.trace.clone() + &self.event_name;
    }
}
```

The diff harness's `_rust_trace` rewrite already adapts
generated traces for cross-backend matching; see
`fuzz/diff_harness/` for examples.

---

## `await` syntax appears at the call site

Frame's async lowers differently per target:

| Target | Lowering |
|---|---|
| Python, JS, TS | `async def f` / `async function f` + `await EXPR` |
| Java | `CompletableFuture<T>` return + `.completedFuture(...)` body |
| Dart | `async` + `await EXPR` |
| Rust | `async fn f` + **`EXPR.await`** — the `.await` is a postfix |
| C++ | `co_await EXPR.method()` |
| Kotlin | `suspend fun f` + plain call (compiler-driven) |
| Swift | `async func f` + `await EXPR` |

Rust's postfix `.await` syntax means Frame source written for
async-portability needs care. The recommended pattern is to keep
`await EXPR` in Frame source and rely on framec's per-target
lowering — which translates `await EXPR.method()` to
`EXPR.method().await` for Rust automatically.

```frame
async fetch(key: String) {
    @@:return = await self.cache.get(key)
}
```

```rust
// generated Rust
async fn fetch(...) -> ... {
    let __result = self.cache.get(key).await;
    ...
}
```

The async runtime expectation on Rust is `tokio` by default — the
test harness wraps the generated module with a `#[tokio::main]`
driver. If you bring your own runtime (smol, async-std), the
generated code is runtime-agnostic except for the test harness.

See `tests/common/positive/async_basic.frs` for the Rust async
shape. The Phase 6 fuzz `two_awaits` pattern (`await A; await B;`)
exercises sequential awaits — landed in commit referenced in
`memory/rust_full_parity_2026_04_26.md`.

---

## Cross-system fields use `Rc<RefCell<...>>`

Frame's cross-system instantiation (`var inner: Counter = @@Counter()`)
needs an interior-mutability shape on Rust because the embedding
system's `&mut self` methods will call `&mut self` methods on the
embedded system. Framec emits:

```rust
struct FrameSystem {
    counter: Rc<RefCell<FrameCounter>>,
}

impl FrameSystem {
    fn new() -> Self {
        FrameSystem {
            counter: Rc::new(RefCell::new(FrameCounter::new())),
        }
    }
}
```

Calls to `self.counter.bump()` lower to `self.counter.borrow_mut().bump()`.
This is the Rust-idiomatic shape for "object-style composition with
shared, mutable interior" — single-threaded by default. If you need
`Send + Sync`, swap `Rc<RefCell<...>>` for `Arc<Mutex<...>>` in a
hand-written wrapper layer; framec doesn't auto-emit that variant
because the threading model is the user's call.

The `Rc<RefCell<>>` shape on cross-system fields is captured in the
matrix test fixtures — see Phase 7 multi-system fuzz for the round-trip
assertion that the embedded system's state survives across calls.

---

## Domain fields: typed struct field access

Domain fields live on the `Frame<SystemName>` struct directly:

```frame
domain:
    var n: i32 = 0
    var name: String = String::from("alice")
```

```rust
struct FrameMySystem {
    n: i32,
    name: String,
    // ...
}

impl FrameMySystem {
    fn new() -> Self {
        Self { n: 0, name: String::from("alice"), ... }
    }
}
```

Reads are `self.n`, writes are `self.n = ...`. The Frame `: type`
annotation IS the Rust type — you are writing typed Rust syntax in
Frame domain declarations.

**This is more constrained than dynamic targets.** Frame source
written for Rust portability declares concrete Rust types, not the
generic `: int` / `: str` you might write for cross-target. If you
write `: int` for Rust, framec emits `i32` (the default integer
width). If you need `i64` or `u64`, declare `: i64` / `: u64`
explicitly.

The Frame compiler does not synthesize Rust-specific defaults for
domain fields beyond `String::from("...")` for string literals. For
struct types, you must provide an explicit constructor expression.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Rust the same way it applies to
every other backend. The comment leader for native code is `//`
(line) or `/* */` (block).

```frame
@@target rust

// Module-prolog block — passes through as Rust source.
use std::collections::HashMap;

@@system Counter {
    machine:
        // Comments inside @@system blocks are also written
        // in target-native syntax.
        $Counting {
            tick() { ... }
        }
}
```

Section-level leading comments (above interface methods, domain
vars, states, actions, operations, handlers) are preserved into
the generated `.rs` output as native comment blocks attached to
the corresponding generated declaration.

---

## Multi-system per file: works as you'd expect

Unlike Java (one public class per file) and Erlang (one module
per file), Rust has no per-file structural constraint on
multi-system definitions. A `.frs` source containing multiple
`@@system` blocks compiles to a single `.rs` file with multiple
`Frame<Name>` structs and their associated `impl` blocks, one
after another.

```frame
@@system Producer { ... }
@@system Consumer { ... }
```

Both `FrameProducer` and `FrameConsumer` end up in the same `.rs`
output. Visibility, Rc/RefCell wiring for cross-system embedding,
and module-export shape all work without ceremony.

---

## Idiomatic patterns and common gotchas

**Use `String::from("...")` for owned string defaults.** Frame's
domain field initialization writes the expression verbatim into
the `Self { ... }` constructor. A bare `"alice"` literal is a
`&'static str`, not `String` — if your field is `: String`, you
need `String::from("alice")` or `"alice".to_string()`.

**`format!("...", expr)` for portable string composition.**
`format!` is Rust's universal formatting macro; it handles
ownership, borrows, and trait-based formatting in one call. Use
it instead of `+` whenever the string contents include
non-string types or multiple borrowed sources.

**`self.field.clone()` to take ownership of a field's value.** If
you need to *return* a domain field's value from a handler (via
`@@:return = self.x`), and the field is a `String` or other
non-`Copy` type, you must clone it: `@@:return = self.x.clone()`.
The wrapper signature returns by value, not by reference, on
typed targets.

**`var x: i32 = 0` is a `let mut` in handler bodies.** Frame's
handler-local `var` declarations lower to `let mut` on Rust.
Reassigning is fine; if you don't need mutability, the Rust
compiler will warn you — Frame does not synthesize a non-`mut`
variant.

**Handler bodies cannot capture state into nested closures.**
Frame's E407 ("Frame statements inside nested scopes are not
supported") catches `|x| { @@:transition $State; }`-style
patterns in Rust closures. The validator's `skip_nested_scope`
detects Rust's `|args| { ... }` closure body and blocks Frame
statements inside it. Lift the work to a helper handler or
domain method.

**HSM ancestor signatures are checked at compile time, not
runtime.** A child state declaring `$Child(x: i32) => $Parent`
must have a parent that also declares `$Parent(x: i32)`. The
strict signature match is enforced both by the framec validator
(at Frame compile time) and by the Rust `if let
StateContext::Variant { x }` decode (at Rust compile time). You
cannot compile a Rust output where the chain disagrees.

**No `Send` / `Sync` by default.** The generated `Frame<Name>`
struct is not `Send + Sync` because `Rc<RefCell<...>>` isn't.
If you need to share a state machine across threads, wrap the
whole system in `Arc<Mutex<...>>` at the call site, or hand-write
a `Frame<Name>` variant using `Arc<Mutex<...>>` for cross-system
fields.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Rust shows ✅ on every row.
- `tests/common/positive/primary/52_hsm_state_arg_propagation.frs`
  — canonical HSM state-arg propagation test.
- `tests/common/positive/primary/52_deep_self_call.frs` — five-mode
  self-call cascade with transition guards at varying depths.
- `framec/src/frame_c/compiler/codegen/rust_system.rs` — the Rust
  backend codegen. `rust_expand_transition` emits the typed
  StateContext re-bind chain; `state_hsm_parents` walk drives the
  ancestor propagation.
- `memory/rust_full_parity_2026_04_26.md` — context on the final
  three Rust gaps that closed (state-arg propagation,
  `__push_transition` cascade, `unsafe` removal).
