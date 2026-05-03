# Per-Language Guide: Dart

Dart's design — class-based OO with optional typing, `Future<T>` for
async, garbage collection, both AOT-compiled (Flutter releases) and
JIT-compiled (development) — makes it ergonomically similar to
Kotlin/Swift but with its own conventions. The two distinguishing
Dart idioms for Frame source are: `Future<T>` for async (with
`async`/`await` syntax similar to TypeScript) and `${expr}` string
interpolation.

This guide documents the Dart-specific patterns. It assumes you are
already familiar with Frame's core syntax and Dart basics (`class`,
`var` / `final` / `const`, `Future<T>`, `async` / `await`).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Dart is
fully spec-conformant on every row.

---

## Foundation: class with member methods

A Frame system targeting Dart generates a single `.dart` file
containing:

- A `class WithInterface { ... }` with member fields and methods.
- A constructor (`WithInterface()`) that fires the start-state's
  `$>` cascade.
- One `String greet(String name)`-style method per interface
  entry.
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers.

```dart
class WithInterface {
    num call_count = 0;
    // ... runtime fields

    WithInterface() {
        // start-state $> cascade fires here
    }

    String greet(String name) {
        // ... handler body
        return result;
    }
}
```

Frame's `self.field` lowers to `this.field`. Method calls are
`s.greet("World")`.

---

## Domain fields: typed members with default initializers

Domain fields lower to member fields:

```frame
domain:
    call_count: num = 0
    name: String = "alice"
    items: List<int> = []
```

```dart
num call_count = 0;
String name = "alice";
List<int> items = [];
```

The Frame `: type` annotation IS the Dart type — write `: String`,
`: List<int>`, `: Map<String, dynamic>`, etc.

**Frame type names map cleanly to Dart types:**

| Frame              | Dart                | Notes |
|--------------------|---------------------|-------|
| `int`              | `int`               | arbitrary-precision on JIT, 64-bit on AOT |
| `num`              | `num`               | `int` or `double` |
| `String`           | `String`            | |
| `bool`             | `bool`              | |
| `double`           | `double`            | 64-bit |
| `List<T>`          | `List<T>`           | |
| `Map<K,V>`         | `Map<K,V>`          | |

Note: Dart's `num` is the supertype of `int` and `double` —
useful when the value could be either.

---

## Strings: `+` for concat, `${expr}` for interpolation

Dart's `+` operator concatenates `String` values. Interpolated
string literals use `"${expr}"` (or `"$variable"` for simple
identifiers):

```frame
$Ready {
    greet(name: String): String {
        this.call_count += 1
        @@:("Hello, ${name}!")
        return
    }
}
```

```dart
String greet(String name) {
    this.call_count += 1;
    return "Hello, ${name}!";
}
```

Dart's interpolation syntax (`${expr}`) is identical to Kotlin's
template strings, similar to JavaScript backticks but using
double-quotes. Pass through verbatim.

---

## Async: `Future<T>` with `async`/`await`

Frame's `async` interface methods on Dart lower to `async`-marked
functions returning `Future<T>`:

```frame
async fetch(key: String): String {
    @@:return = await self.cache.get(key)
}
```

```dart
Future<String> fetch(String key) async {
    var __result = await cache.get(key);
    return __result;
}
```

Dart async is mature:

- `async` keyword goes after the parameter list (Dart-specific
  syntax; other targets put it before).
- `await EXPR` at call sites — same shape as TypeScript/JS.
- Async functions return `Future<T>` automatically; non-async
  functions returning futures must construct them explicitly.
- The matrix harness drives async tests via `await` from a
  `Future<void> main()`.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to a typed field
instantiated via direct call:

```dart
class Embedding {
    Counter counter;

    Embedding() : counter = Counter() {
        // start-state $> fires
    }

    void notify() {
        counter.bump(1);
    }
}
```

Calls to `self.counter.bump(n)` lower to `counter.bump(n)`. The
constructor uses Dart's initializer-list syntax (`: counter =
Counter()`) before the body.

---

## Loop idioms — both work

Dart has `while`, `do-while`, `for`, and `for-in`. Frame's idiom
1 (`while cond { ... }`) compiles to a native Dart `while`
block via passthrough.

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

Dart-native braces and `i++` work inside the Frame `while` block
— no escaping needed.

---

## Multi-system per file: works as you'd expect

A `.fdart` source containing multiple `@@system` blocks compiles
to a single `.dart` file with multiple class definitions:

```frame
@@system Producer { ... }
@@[main]
@@system Consumer { ... }
```

Both `Producer` and `Consumer` end up as separate classes in the
same `.dart` output. Dart has no per-file structural constraint
on multi-class definitions.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Dart the same way it applies to
every other backend. The comment leaders are `//` (line),
`/* ... */` (block), and `///` (DartDoc).

```frame
@@[target("dart")]

// Module-prolog block — passes through as Dart source.

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

**Constructor call without `new`.** Dart 2.0+ deprecated `new` —
`Counter()` (no `new`) is the constructor call. Frame's
`@@Counter()` lowers to bare `Counter()`.

**`final` for run-once-set, `var` for mutable.** Dart's `final`
is equivalent to Java's final — the reference can't be reassigned,
though the referenced object may still be mutable. `var` is for
fully mutable values. Frame's domain fields are mutable
(`var`-style).

**`var` for type inference.** Dart's `var` infers types from
initializers. For Frame domain fields, explicit `: type` is
preferred for documentation.

**`null` is the absent-value marker.** Dart 2.12+ has null
safety — types like `String` cannot hold null; nullable types
(`String?`) can. Frame's nullable types should declare `?`
explicitly.

**`?.` for safe navigation, `!` for non-null assertion.** Same
operators as Kotlin/Swift. `s.parent?.bump(1)` is safe;
`s.parent!.bump(1)` throws if null.

**`Exception()` for thrown errors.** Dart's `throw Exception(msg)`
is the standard error-throwing pattern. Test drivers use this
for assertion failures.

**AOT vs JIT execution.** Dart can be compiled ahead-of-time
(for Flutter releases) or JIT-compiled (for development). Frame
output works in both modes. For matrix tests, the harness uses
the Dart VM (JIT) by default; for AOT testing, snapshot the
compiled output via `dart compile aot-snapshot`.

**`pubspec.yaml` for dependencies.** If your generated Dart
depends on packages from pub.dev, declare them in
`pubspec.yaml`. The matrix harness has a default `pubspec.yaml`
for the test runner; production projects use their own.

---

## Persist contract — `@@[save]` / `@@[load]`

A `@@[persist]` system must declare two operations under the
`operations:` section: one tagged `@@[save]` (returns the
serialized blob) and one tagged `@@[load]` (instance method
that mutates self from a blob). The op names are yours to
pick — these match the Dart convention.

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

```dart
var c2 = Counter();
c2.restoreState(data);
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
throws `Exception("E700: system not quiescent")`. Catchable via
`try/catch`, but recovery isn't possible — the handler's context
frame is corrupted; discard the instance and restore from a prior
snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Dart shows ✅ on every row.
- `tests/common/positive/primary/02_interface.fdart` —
  canonical interface-method shape with `${...}` interpolation.
- `framec/src/frame_c/compiler/codegen/backends/dart.rs` —
  Dart backend codegen.
- `framepiler_test_env/docker/runners/dart_runner.sh` — matrix
  runner; uses the Dart SDK.
