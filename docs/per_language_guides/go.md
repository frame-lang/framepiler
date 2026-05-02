# Per-Language Guide: Go

Go's design choices — pointer receivers, capitalized exports,
goroutines + channels for concurrency, structural interfaces — make
it distinct from the Java/C# typed-class default. Frame source for
Go reads similarly to other typed targets, but a few language idioms
need attention: interface method names must be capitalized to be
exported, the system handle is a `*WithInterface` pointer with
`s *WithInterface` method receivers, and `async` is a language-natural
skip (Go is "one-color" — goroutines + channels do not map onto the
kernel-callback structure framec uses).

This guide documents the Go-specific patterns. It assumes you are
already familiar with Frame's core syntax and Go basics (struct
methods, pointer receivers, capitalization-as-export, packages).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Go is fully
spec-conformant on the runtime; `async` is the only language-natural
skip (`[g]`).

---

## Foundation: pointer-receiver methods on a struct

A Frame system targeting Go generates a single `.go` file
containing:

- A `type WithInterface struct { ... }` with domain fields and
  Frame runtime fields.
- A `NewWithInterface(...) *WithInterface` constructor returning
  a pointer to a heap-allocated instance.
- One `func (s *WithInterface) Greet(name string) string`-style
  method per interface entry — pointer-receiver, capitalized to
  be exported.
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers as lowercase (unexported) methods.

```go
type WithInterface struct {
    call_count int
    // ... runtime fields
}

func NewWithInterface() *WithInterface {
    s := &WithInterface{}
    // start-state $> cascade fires here
    return s
}

func (s *WithInterface) Greet(name string) string {
    // ... handler body
    return result
}
```

Frame's `self.field` lowers to `s.field` (the receiver name `s`,
not `self` or `this`). Method calls use `sm.Greet("World")` — Go's
pointer-method-receiver auto-derefs on call, so you don't write
`(*sm).Greet(...)`.

---

## Capitalize interface method names — exported is the contract

Go uses identifier capitalization for export visibility:

- `Greet` (capital G) — exported, callable from other packages.
- `greet` (lowercase g) — package-private.

Frame's interface methods are *the* public API of the generated
state machine. Frame source for Go must capitalize interface
method names to make them callable:

```frame
@@[target("go")]

@@system WithInterface {
    interface:
        Greet(name: string): string    // ← capital G — exported
        GetCount(): int                // ← capital G — exported

    machine:
        $Ready {
            Greet(name: string): string { ... }
        }
}
```

The lowercase form (`greet`) compiles but generates an unexported
method, which means cross-package callers can't invoke it. Inside
a single `package main` driver, lowercase is fine; for library-style
state machines, capitalize.

This is one of the most consistent gotchas across Go targets —
Frame source written for cross-target portability often uses
lowercase method names (matching Python/JS conventions), and
that breaks Go's export contract. The matrix-side fixtures
capitalize all interface methods for Go specifically.

---

## Domain fields: typed struct fields with default initializers

Domain fields live as fields on the `struct WithInterface`:

```frame
domain:
    call_count: int = 0
    name: string = "alice"
    items: []int = nil
```

```go
type WithInterface struct {
    call_count int
    name       string
    items      []int
    // ... runtime fields
}
```

Reads are `s.call_count`, writes are `s.call_count = ...`. The
Frame `: type` annotation IS the Go type — write `: string`,
`: []int`, `: map[string]int`, `: *Counter` (for cross-system
embedding), etc. Frame doesn't auto-prefix package paths —
declare imports in the prolog.

**Default initialization quirk.** Go has no in-struct default
initializer syntax — `int call_count = 0` doesn't exist. Frame's
`: int = 0` lowers to a struct field declaration plus an explicit
assignment in `NewWithInterface`:

```go
type WithInterface struct {
    call_count int        // declaration only
}

func NewWithInterface() *WithInterface {
    s := &WithInterface{
        call_count: 0,      // initializer in constructor
    }
    return s
}
```

This is fine for primitive types where the zero-value would have
been used anyway. For non-zero defaults (`name string = "alice"`),
the constructor's struct literal carries the value.

---

## Strings: `+` for concat, `fmt.Sprintf` for printf-style

Go's `+` operator concatenates strings, similar to Java/C#:

```frame
$Ready {
    Greet(name: string): string {
        s.call_count += 1
        @@:("Hello, " + name + "!")
        return
    }
}
```

```go
func (s *WithInterface) Greet(name string) string {
    s.call_count += 1
    return "Hello, " + name + "!"
}
```

For more complex formatting, use `fmt.Sprintf` (the printf-style
helper):

```frame
$Active {
    Log() {
        s.message = fmt.Sprintf("count=%d, name=%s", s.count, s.name)
    }
}
```

Note: `fmt.Sprintf` requires `import "fmt"` in the prolog. The
matrix tests put `import "fmt"` and `import "os"` at the top of
every `.fgo` source.

---

## Cross-system fields: pointer to embedded struct

`var counter: *Counter = @@Counter()` lowers to a
`*Counter` field on the embedding struct, allocated via
`NewCounter()` in the constructor:

```go
type Embedding struct {
    counter *Counter   // pointer field
}

func NewEmbedding() *Embedding {
    s := &Embedding{
        counter: NewCounter(),
    }
    return s
}

func (s *Embedding) Notify() {
    s.counter.Bump(1)
}
```

Calls to `self.counter.bump(n)` lower to `s.counter.Bump(n)` —
note the capitalization on the called method (it's an exported
interface method on the embedded system, so it's capitalized).

This was a framec codegen fix in the Phase 7 multi-system round
(see `memory/phase7_multisys_2026_04_27.md`); the Go cross-system
field shape uses pointer fields consistently.

---

## No async — language-natural skip

Go's concurrency model is goroutines + channels, not async/await.
The matrix capability table shows Go's async row as 🚫[g] —
"Go's concurrency model is goroutines + channels, which doesn't
map cleanly onto the kernel-callback structure framec uses. Tests
skip with `@@skip -- go is one-color`."

For asynchronous behavior, write `go someFunc()` to spawn a
goroutine, use channels for communication, and `select` for
multiplexing. These are user concerns, not Frame's. The state
machine itself is single-goroutine.

If you need to drive a Frame state machine from multiple
goroutines, wrap the system in a coordination layer that
serializes calls through a channel. Frame's runtime does not
synchronize across goroutines.

---

## Loop idioms — both work

Go has `for` (the only loop construct — used for both while-
style and counted iteration) and channel-based `for-range`.
Frame's idiom 1 (`while cond { ... }`) compiles to a Go-style
`for cond { ... }` block:

```frame
$Counting {
    Tick() {
        i := 0
        for i < 10 {
            i++
        }
    }
}
```

Note: Go's loop keyword is `for`, not `while`. Frame source
written with the `while` keyword for Go targets is a Frame-syntax
wrapper that lowers to `for` — the Frame `while cond { ... }`
shape *is* what you write, but framec generates the Go-native
`for cond { ... }` shape.

For idiomatic counted iteration, use `for i := 0; i < n; i++ { ... }`
inside the handler body via Oceans Model passthrough.

---

## Multi-system per file: works as you'd expect

A `.fgo` source containing multiple `@@system` blocks compiles to
a single `.go` file with multiple struct/method-family definitions:

```frame
@@system Producer { ... }
@@[main]
@@system Consumer { ... }
```

Both `Producer` and `Consumer` end up in the same file under the
declared package. Go has no per-file structural constraint on
multi-struct definitions.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Go the same way it applies to
every other backend. The comment leaders are `//` (line) and
`/* ... */` (block).

```frame
@@[target("go")]

package main

import "fmt"

// Module-prolog block — passes through as Go source.

@@system Counter {
    machine:
        // Section-level comments are preserved as native // blocks.
        $Counting {
            Tick() { ... }
        }
}
```

Section-level leading comments are preserved as native `//`
blocks attached to the corresponding generated declaration.
Standard Go-style godoc comments (preceding exported identifiers)
work via the same mechanism.

---

## Idiomatic patterns and common gotchas

**`package main` + `func main()` for runnable programs.** Any
test driver or executable Frame source must declare
`package main` in the prolog and provide `func main() { ... }`
either inline or in another file in the same directory. Library-
style packages use `package <name>` instead.

**Pointer receivers throughout.** Frame's emitted method shape
uses `s *WithInterface` pointer receivers. Mixing pointer and
value receivers on the same type is technically allowed in Go
but discouraged; Frame's codegen sticks to pointer receivers
consistently for state-mutating semantics.

**`nil` for absent values.** Go uses `nil` for unset pointers,
slices, maps, channels, interfaces, and function values. Frame
source can use `nil` consistently. Note: `nil` is *not* a valid
zero-value for primitives (you'd use `0` for int, `""` for
string).

**`s.field`, not `self.field`.** Inside handler bodies, Frame's
`self.x` lowers to `s.x` (the receiver name). If you write
native Go inside a handler, use `s.` to access the struct's
fields.

**Slice and map zero values.** A field declared `: []int` with
no initializer defaults to `nil` (an empty slice that's not yet
allocated). Append to it via `s.field = append(s.field, ...)`
which auto-allocates on first use.

**`for-range` for iteration over slices/maps.** Pass through
verbatim — Frame doesn't have a special syntax for iteration
over collections beyond the cross-target idioms 1 and 2.

**Imports go in the prolog.** Go's `import "fmt"` declarations
are file-scope and must precede any struct/method definitions.
Frame's prolog (above `@@system`) is where they go.

**Capitalized = exported, lowercase = unexported.** This applies
to *every* identifier — types, methods, fields, functions. If
you want a Frame state machine usable from a separate package,
capitalize the system name and all interface method names. A
state machine consumed only inside `package main` doesn't need
capitalization.

**No exception handling — return `error` instead.** Frame's
state machines don't throw or return error values themselves.
For handler bodies that call into error-returning Go APIs,
write `if err != nil { ... }` blocks via Oceans Model
passthrough. Frame doesn't provide error-handling abstractions.

---

## Persist quiescent contract — E700

`SaveState()` requires the system to be quiescent (no event in
flight, `_context_stack` empty). Calling it from inside a handler
panics with `"E700: system not quiescent"`. Go panics on contract
violation rather than returning `(string, error)` to keep the
public signature `string` and avoid forcing every caller to
handle a (theoretically-impossible-when-used-correctly) error.
Catchable via `defer func() { recover() }()`, but recovery isn't
possible — the handler's context frame is corrupted; discard the
instance and restore from a prior snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Go shows ✅ on every row except `async` (🚫[g] —
  language-natural skip).
- `tests/common/positive/primary/02_interface.fgo` — canonical
  interface-method shape with `+`-string concat and capitalized
  method names.
- `tests/common/positive/primary/09_stack.fgo` — push/pop state
  stack reference.
- `framec/src/frame_c/compiler/codegen/backends/go.rs` — Go
  backend codegen.
- `memory/phase7_multisys_2026_04_27.md` — context on the Go
  pointer-field cross-system fix.
