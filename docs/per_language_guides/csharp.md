# Per-Language Guide: C#

C# is structurally close to Java — both targets share class-based
OO, garbage-collected runtimes, typed methods, and `+`-overloaded
String concatenation. The two distinguishing C# idioms for Frame
source are: `Task<T>` (and `async`/`await`) for async, and the
relaxed file-class naming rule (multiple top-level classes per file
are allowed, unlike Java).

This guide documents the C#-specific patterns. It assumes you are
already familiar with Frame's core syntax and C# basics (`class`,
`Task<T>`, `async`/`await`, `using` directives).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. C# is
fully spec-conformant on every row.

---

## Foundation: class with member methods

A Frame system targeting C# generates a single `.cs` file
containing:

- A `public class WithInterface { ... }` with private domain
  fields and public interface methods.
- An `init()` constructor that fires the start-state's `$>`
  cascade.
- One `public string greet(string name)`-style method per
  interface entry.
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers as private methods.

```csharp
public class WithInterface {
    private int call_count = 0;
    // ... runtime fields

    public WithInterface() {
        // start-state $> cascade fires here
    }

    public string greet(string name) {
        // ... handler body
        return result;
    }
}
```

Frame's `self.field` lowers to `this.field` in the generated C#.
Multiple `@@system` blocks per file are supported (no
one-class-per-file rule).

---

## Domain fields: typed properties with default initializers

Domain fields lower to private member fields with optional default
initializers:

```frame
domain:
    call_count: int = 0
    name: string = "alice"
    items: List<int> = new List<int>()
```

```csharp
private int call_count = 0;
private string name = "alice";
private List<int> items = new List<int>();
```

The Frame `: type` annotation IS the C# type — write `: string`,
`: List<int>`, `: Dictionary<string, int>`, etc.

**Frame type names map cleanly to C# types:**

| Frame              | C#                  | Notes |
|--------------------|---------------------|-------|
| `int`              | `int`               | 32-bit signed |
| `string`           | `string`            | C# alias for `System.String` |
| `bool`             | `bool`              | |
| `double`           | `double`            | |
| `List<T>`          | `System.Collections.Generic.List<T>` | needs `using` |
| `Dictionary<K,V>`  | `Dictionary<K,V>`   | needs `using` |

---

## Strings: `+` for concat, `$"..."` for interpolation

C#'s `+` operator concatenates `string` values (overloaded).
Interpolated string literals use `$"..."` with `{expr}`
placeholders:

```frame
$Ready {
    greet(name: string): string {
        this.call_count += 1
        @@:("Hello, " + name + "!")
        return
    }
}
```

```csharp
public string greet(string name) {
    this.call_count += 1;
    return "Hello, " + name + "!";
}
```

For interpolation:

```frame
$Active {
    log() {
        this.message = $"count={this.count}, name={this.name}"
    }
}
```

`string.Format(...)` and `StringBuilder` are also available for
more complex cases.

---

## Async: `Task<T>` with `async`/`await`

Frame's `async` interface methods on C# lower to `async Task<T>`
return types with `await` at call sites:

```frame
async fetch(key: string): string {
    @@:return = await self.cache.get(key)
}
```

```csharp
public async Task<string> fetch(string key) {
    var __result = await cache.get(key);
    return __result;
}
```

C# async is mature and well-integrated:

- `Task<T>` is the universal async return type for typed async
  methods. `Task` (no `<T>`) for void async.
- `await EXPR` at call sites — same shape as Python/JS/TS.
- `Task.Run(...)` for offloading sync work to a thread pool.
- The matrix harness uses `.Result` or `await` from a `Main`
  with `static async Task Main(string[] args)` (C# 7.1+).

There is no separate "completed-future" wrapper pattern needed
on C# (unlike Java's `CompletableFuture.completedFuture(...)`)
because C#'s compiler-generated state machine handles the
synchronous-completion case efficiently.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to a typed property
instantiated via `new Counter()` in the constructor:

```csharp
public class Embedding {
    private Counter counter;

    public Embedding() {
        this.counter = new Counter();
        // start-state $> fires
    }

    public void notify() {
        this.counter.bump(1);
    }
}
```

Calls to `self.counter.bump(n)` lower to `this.counter.bump(n)`.
Lifecycle is the .NET garbage collector's responsibility.

---

## Loop idioms — both work

C# has `while`, `do-while`, `for`, and `foreach`. Frame's idiom 1
(`while cond { ... }`) compiles to a native C# `while` block via
passthrough.

```frame
$Counting {
    tick() {
        var i = 0;
        while (i < 10) {
            i++;
        }
    }
}
```

C#-native braces and `i++` work inside the Frame `while` block —
no escaping needed.

---

## Multi-system per file: works as you'd expect

A `.fcs` source containing multiple `@@system` blocks compiles
to a single `.cs` file with multiple class definitions:

```frame
@@system Producer { ... }
@@[main]
@@system Consumer { ... }
```

Both `Producer` and `Consumer` end up as separate `public class`
blocks in the same `.cs` output. C# has no per-file structural
constraint analogous to Java's one-public-class rule.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to C# the same way it applies to
every other backend. The comment leaders are `//` (line),
`/* ... */` (block), and `///` (XML doc-comment for tooling).

```frame
@@[target("csharp")]

using System;
using System.Collections.Generic;

// Module-prolog block — passes through as C# source.

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

**`new` for class instantiation.** Frame's `@@WithInterface()`
lowers to `new WithInterface()` — same as Java.

**`this.field`, not `self.field`.** Inside handler bodies,
Frame's `self.x` lowers to `this.x`. The `this.` prefix is
optional in C# but Frame's codegen emits it for clarity.

**`using` directives go in the prolog.** C#'s `using` (the
namespace-import form) is file-scope and must precede any class
definition. Frame's prolog (above `@@system`) is where they go.
The `using ... = ...;` aliasing form is also supported.

**`namespace` in the prolog if you need package isolation.** If
your generated code lives in a specific .NET namespace, declare
`namespace MyApp.StateMachines { ... }` in the prolog,
wrapping the @@system block. Frame doesn't auto-emit a namespace.

**`Task` vs `Task<T>`.** Async interface methods that return a
value declare `(): T` in Frame source; the wrapper emits
`Task<T>`. Async methods with no return type use `(): void`
in Frame; the wrapper emits `Task`.

**Cast at call sites for typed extraction.** When calling a Frame
interface method from C# code, the test driver may need to cast
the result to the declared type:

```csharp
var result = (string)s.greet("World");
var count = (int)s.get_count();
```

This is because the Frame runtime's return-slot is `object` —
the cast extracts the typed value. This is a test-driver
concern; production callers can use generics or explicit return
types.

**No exceptions in Frame's runtime.** Frame's state machines
don't throw exceptions themselves. Handler bodies that call
into exception-throwing APIs should write `try`/`catch` blocks
explicitly via Oceans Model passthrough.

**`null` is the absent-value marker.** C# uses `null` for unset
references. Nullable value types (`int?`) use the same `null`.

**`var` for type inference.** C# 3.0+ has `var` for local
variable type inference. Frame doesn't auto-emit `var` —
generated code uses explicit types. Inside handler bodies via
Oceans Model passthrough, you can use `var` freely.

---

## Persist quiescent contract — E700

`SaveState()` requires the system to be quiescent (no event in
flight, `_context_stack` empty). Calling it from inside a handler
throws `System.Exception` with message `E700: system not
quiescent`. Catchable via `try/catch`, but recovery isn't
possible — the handler's context frame is corrupted; discard the
instance and restore from a prior snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; C# shows ✅ on every row.
- `tests/common/positive/primary/02_interface.fcs` — canonical
  interface-method shape with `+`-string concat.
- `framec/src/frame_c/compiler/codegen/backends/csharp.rs` —
  C# backend codegen.
- `framepiler_test_env/docker/runners/csharp_runner.sh` —
  matrix runner; uses the .NET SDK.
