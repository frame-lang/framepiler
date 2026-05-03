# Per-Language Guide: Swift

Swift's design choices — strict typing, optional unwrapping, value-
type structs vs reference-type classes, native `async`/`await`/`throws`
for error propagation — make it more rigorous than the dynamic-target
default but more ergonomic than C++. Frame source for Swift uses
`class` (reference semantics) for the system handle, `var` for
mutable domain fields, and `async` interface methods that lower to
Swift's native `async func ... throws -> T` shape.

This guide documents the Swift-specific patterns. It assumes you
are already familiar with Frame's core syntax and Swift basics
(`class` / `struct`, `var` / `let`, `init()`, `async` / `await`,
optionals).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Swift is
fully spec-conformant on every row.

---

## Foundation: reference-type class with `init()`

A Frame system targeting Swift generates a single `.swift` file
containing:

- A `class WithInterface { ... }` (reference type, not a value-
  type `struct` — Frame state machines need shared mutable state
  semantics).
- An `init() { ... }` constructor that fires the start-state's
  `$>` cascade.
- Domain fields as `var` properties.
- Public interface methods (`func greet(name: String) -> String`).
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers.

```swift
class WithInterface {
    var call_count: Int = 0
    // ... runtime fields

    init() {
        // start-state $> cascade fires here
    }

    func greet(name: String) -> String {
        // ... handler body
        return result
    }
}
```

Frame's `self.field` lowers to `self.field` (Swift's instance
reference is `self`). Method calls are `s.greet("World")`. There
is no explicit type cast at call sites — Swift's strong typing
infers from context.

**Frame uses `class`, not `struct`.** Swift's `struct` is a value
type (copy-on-assign) which would break Frame's shared-mutable-
state semantics. Frame state machines always lower to `class`.

---

## Domain fields: `var` with explicit types

Domain fields lower to `var` properties on the class:

```frame
domain:
    call_count: Int = 0
    name: String = "alice"
    items: [Int] = []
```

```swift
var call_count: Int = 0
var name: String = "alice"
var items: [Int] = []
```

Reads and writes use `self.field` (with explicit `self.` for
clarity in handler bodies — Swift requires it inside closures
and prefers it for readability elsewhere).

**Frame's type names map cleanly to Swift types:**

| Frame              | Swift            | Notes |
|--------------------|------------------|-------|
| `Int`              | `Int`            | platform-width Int |
| `String`           | `String`         | |
| `Bool`             | `Bool`           | |
| `Double`           | `Double`         | |
| `[T]`              | `[T]`            | Swift array literal syntax |
| `[K: V]`           | `[K: V]`         | Swift dictionary syntax |
| `T?`               | `T?`             | optional |

Frame's `: type` annotation IS the Swift type — write `: String`,
`: [Int]`, `: [String: Int]`, `: Counter?` (for nullable cross-
system embedding).

---

## Strings: `+` for concat, interpolation with `\(...)`

Swift's `+` operator concatenates `String` values. String
interpolation uses `"\(expr)"` (backslash-paren-expression-paren):

```frame
$Ready {
    greet(name: String): String {
        self.call_count += 1
        @@:("Hello, " + name + "!")
        return
    }
}
```

```swift
func greet(name: String) -> String {
    self.call_count += 1
    return "Hello, " + name + "!"
}
```

For interpolation:

```frame
$Active {
    log() {
        self.message = "count=\(self.count), name=\(self.name)"
    }
}
```

Pass through verbatim — Swift's `\(...)` interpolation doesn't
interfere with Frame's `@@:` markers.

---

## Async: `async func ... throws -> T`

Frame's `async` interface methods on Swift lower to native
`async`/`await` syntax:

```frame
async fetch(key: String): String {
    @@:return = await self.cache.get(key)
}
```

```swift
func fetch(key: String) async throws -> String {
    let __result = try await cache.get(key)
    return __result
}
```

**Key conventions** (per `memory/phase6_async_2026_04_27.md`):

- Async interface methods are emitted as `async throws -> T` —
  the `throws` allows propagation from any await-call that may
  throw.
- Calls use the `try await` prefix — Swift requires the `try`
  before `await` if the called function can throw.
- The constructor (`init()`) stays synchronous; the
  start-state cascade fires synchronously.

Swift's `async` machinery is mature but requires Swift 5.5+ and
an executor at runtime. The matrix harness wraps test drivers
with `Task { ... }` or `await` from a `@MainActor` closure to
provide the execution environment.

For non-async interface methods, Swift's `func name() -> T` form
is used (no `throws`). Frame doesn't add `throws` to non-async
methods; if you need to throw from a sync handler, write the
declaration explicitly via Oceans Model passthrough.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to a typed property
instantiated via direct call (no `new` keyword in Swift):

```swift
class Embedding {
    var counter: Counter = Counter()

    func notify() {
        counter.bump(1)
    }
}
```

Calls to `self.counter.bump(n)` lower to `counter.bump(n)`. Swift
class instances are reference-typed, so the `var counter` property
holds a reference and mutations on the embedded counter are
visible across the same instance.

---

## Loop idioms — both work

Swift has `while`, `repeat-while`, and range-based `for-in`. Frame's
idiom 1 (`while cond { ... }`) compiles to a native Swift `while`
block via passthrough.

```frame
$Counting {
    tick() {
        var i = 0
        while i < 10 {
            i += 1
        }
    }
}
```

Swift-native braces and `+= 1` (Swift dropped `++`/`--` in 3.0)
work inside the Frame `while` block — no escaping needed.

---

## Multi-system per file: works as you'd expect

A `.fswift` source containing multiple `@@system` blocks compiles
to a single `.swift` file with multiple class definitions:

```frame
@@system Producer { ... }
@@[main]
@@system Consumer { ... }
```

Both `Producer` and `Consumer` end up as separate classes in the
same `.swift` output. Swift has no per-file structural constraint
on multi-class definitions.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Swift the same way it applies
to every other backend. The comment leaders are `//` (line) and
`/* ... */` (block) and `///` (doc-comment for tools).

```frame
@@[target("swift")]

// Module-prolog block — passes through as Swift source.
import Foundation

@@system Counter {
    machine:
        // Section-level comments are preserved as native // blocks.
        $Counting {
            tick() { ... }
        }
}
```

Section-level leading comments are preserved as native `//`
blocks attached to the corresponding generated declaration.

---

## Idiomatic patterns and common gotchas

**`init()`, not `new`.** Swift constructs class instances via
`ClassName()` — no `new` keyword. Frame's `@@WithInterface()`
lowers to `WithInterface()`.

**`self.field` everywhere.** Swift requires `self.` inside
closures (compile error otherwise) and recommends it elsewhere
for clarity. Frame's `self.x` lowers to `self.x` in the
generated Swift.

**Optionals (`T?`) require unwrapping.** A property declared
`var parent: Counter? = nil` cannot be method-called directly —
you must unwrap:

```frame
$Active {
    notify() {
        if let parent = self.parent {
            parent.bump(1)
        }
    }
}
```

Or use optional chaining: `self.parent?.bump(1)`. Force-unwrap
with `!` works but crashes on nil.

**`var` vs `let`.** `var` is mutable; `let` is immutable
(Swift's `final`). Frame's domain fields use `var`. Handler-
local declarations should use `let` where possible.

**`fatalError(...)` for unrecoverable errors.** Swift's
`fatalError` is the equivalent of Java's `throw new
RuntimeException(...)` — it terminates the process. Test
drivers commonly use `fatalError` for assertion failures:

```swift
if result != "Hello, World!" {
    fatalError("Expected 'Hello, World!', got '\(result)'")
}
```

**`try await ...` for async error propagation.** Swift's `async`
methods that can throw must be called with `try await` (in
that order). Frame's async wrapper emits the `try await` form
for await-calls inside async interface methods. Non-throwing
async still uses bare `await`.

**No `new` for arrays/dictionaries either.** `[1, 2, 3]` is
the array literal; `[:]` is the empty-dictionary literal.
Frame's domain field initializers use Swift literal syntax
verbatim.

**Type inference is your friend, but explicit types help.**
Swift can usually infer types from initializers, but for
Frame domain fields, explicit `: Int` / `: String` makes the
generated code easier to read.

---

## Persist contract — `@@[save]` / `@@[load]`

A `@@[persist]` system must declare two operations under the
`operations:` section: one tagged `@@[save]` (returns the
serialized blob) and one tagged `@@[load]` (instance method
that mutates self from a blob). The op names are yours to
pick — these match the Swift convention.

```frame
@@[persist]
@@system Counter {
    operations:
        @@[save]
        saveState(): String {}

        @@[load]
        restoreState(data: String) {}

    interface:  bump()
    machine:    $Active { bump() { self.n = self.n + 1 } }
    domain:     n: int = 0
}
```

Load is an instance method (allocate, then populate):

```swift
let c2 = Counter()
c2.restoreState(data)
```

The bare `@@[persist]` form (no `@@[save]` / `@@[load]` ops) is
rejected with **E814** since framepiler `b3aebc5` (2026-05-03).

### Post-load hook: `@@[on_load]`

A third optional attribute fires user code after
`restoreState` finishes populating self — useful for re-establishing
derived state, firing watchers, validating invariants:

```frame
operations:
    @@[save]    saveState(): String {}
    @@[load]    restoreState(data: String) {}

    @@[on_load]
    rebuild_derived() {
        self.doubled = self.n * 2
    }
```

At-most-one per system (E810). framepiler `a61390e`
(2026-05-03). See [`frame_runtime.md`](../frame_runtime.md)
"Naming the save/load methods" and [RFC-0012](../rfcs/rfc-0012.md)
for the design.

---

## Persist quiescent contract — E700

`saveState()` requires the system to be quiescent (no event in
flight, `_context_stack` empty). Calling it from inside a handler
calls `fatalError("E700: system not quiescent")`, which terminates
the process. Swift has no catchable runtime exceptions for this
class of error — quiescent violations are programming errors,
treated like array-out-of-bounds. The only correct response is to
fix the code so save_state isn't called from inside a handler. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Swift shows ✅ on every row.
- `tests/common/positive/primary/02_interface.fswift` —
  canonical interface-method shape with `+`-string concat and
  `\(...)` interpolation.
- `framec/src/frame_c/compiler/codegen/backends/swift.rs` —
  Swift backend codegen.
- `framepiler_test_env/docker/runners/swift_runner.sh` — matrix
  runner; uses Swift 5.5+ and the Swift Package Manager.
- `memory/phase6_async_2026_04_27.md` — context on Swift async
  wiring (Stage 5 of Phase 6).
