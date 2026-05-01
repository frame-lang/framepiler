# Per-Language Guide: Kotlin

Kotlin's design choices — concise class syntax, type inference,
nullable types, and `suspend fun` for async — make it the most
ergonomic of the JVM-family Frame targets. Where Java requires
verbose `public class WithInterface { ... }` boilerplate, Kotlin's
generated output is half the lines for the same semantic surface.
The two distinguishing features for Frame source are: `suspend fun`
for async (compiler-driven, no explicit `await` at call sites) and
companion objects for state-name placement (a Kotlin-specific quirk
in the section-comment preservation).

This guide documents the Kotlin-specific patterns. It assumes you
are already familiar with Frame's core syntax and Kotlin basics
(`class`, `val`/`var`, `fun`, `suspend fun`, companion objects).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Kotlin is
fully spec-conformant on every row.

---

## Foundation: concise class with member methods

A Frame system targeting Kotlin generates a single `.kt` file
containing:

- A `class WithInterface { ... }` with primary-constructor
  parameters mapped from `@@system Foo(args)` system-args.
- Domain fields as `var` properties with type inference where
  possible.
- Public interface methods (`fun greet(name: String): String`).
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers.
- A `companion object` for state-name constants and any
  state-scope NativeBlocks (the Kotlin-specific quirk for
  comment preservation, see below).

```kotlin
class WithInterface {
    var call_count: Int = 0
    // ... runtime fields

    init {
        // start-state $> cascade fires here
    }

    fun greet(name: String): String {
        // ... handler body
        return result
    }

    companion object {
        // state-name constants, etc.
    }
}
```

Frame's `self.field` lowers to `this.field` (Kotlin's instance
reference is `this`). Method calls are `s.greet("World")` — no
explicit `this.` needed at call sites.

---

## Domain fields: `var` with type inference

Domain fields lower to `var` properties on the class:

```frame
domain:
    call_count = 0           // type inferred as Int
    name: String = "alice"   // explicit type
```

```kotlin
var call_count: Int = 0
var name: String = "alice"
```

Kotlin infers types from initializers, so the Frame `: type`
annotation is optional when the initializer is unambiguous.
For nullable fields, declare with `?`:

```frame
domain:
    parent: Counter? = null
```

```kotlin
var parent: Counter? = null
```

Frame doesn't auto-add `?` for nullable types — you must declare
the nullability explicitly in the type string.

**Frame's type names map cleanly to Kotlin types:**

| Frame              | Kotlin             | Notes |
|--------------------|--------------------|-------|
| `int` / `Int`      | `Int`              | preferred: `Int` for type-fidelity |
| `String`           | `String`           | |
| `Boolean`          | `Boolean`          | |
| `Double`           | `Double`           | |
| `List<T>`          | `kotlin.collections.List<T>` | imported by default |
| `MutableList<T>`   | `kotlin.collections.MutableList<T>` | for mutable collections |

---

## Strings: `+` for concat, template strings for interpolation

Kotlin's `+` operator concatenates strings (similar to Java).
Template strings (`"$name"` or `"${expr}"`) handle interpolation
without explicit conversion:

```frame
$Ready {
    greet(name: String): String {
        this.call_count += 1
        @@:("Hello, " + name + "!")
        return
    }
}
```

```kotlin
fun greet(name: String): String {
    this.call_count += 1
    return "Hello, " + name + "!"
}
```

For interpolation:

```frame
$Active {
    log() {
        this.message = "count=${this.count}, name=${this.name}"
    }
}
```

Pass through verbatim — Kotlin's template string syntax doesn't
interfere with Frame's `@@:` markers.

---

## Async: `suspend fun` — no explicit `await` at call sites

Frame's `async` interface methods on Kotlin lower to `suspend
fun`:

```frame
async fetch(key: String): String {
    @@:return = await self.cache.get(key)
}
```

```kotlin
suspend fun fetch(key: String): String {
    val __result = cache.get(key)
    return __result
}
```

**Key difference from other async targets**: Kotlin's `suspend
fun` is compiler-driven — there's no explicit `await EXPR` or
`co_await EXPR` at call sites. A `suspend fun` can call another
`suspend fun` directly; the compiler handles continuation
passing. Frame's `await EXPR` markers are stripped on Kotlin
output because the suspend marker is on the function definition,
not the call site.

To call a `suspend fun` from non-suspend context, you need a
coroutine builder (`runBlocking { ... }` or `launch { ... }` in
a `CoroutineScope`):

```kotlin
fun main() = runBlocking {
    val s = WithInterface()
    val result = s.fetch("key")   // works inside suspend context
    println(result)
}
```

For the matrix tests, `runBlocking { ... }` wraps the test
driver. See `memory/phase6_async_2026_04_27.md` for the Kotlin
async wiring context.

The dependency: `org.jetbrains.kotlinx:kotlinx-coroutines-core`
must be on the classpath. The matrix harness configures this
via `kotlinx-coroutines-core-*.jar` in
`docker/runners/kotlin_runner.sh`.

---

## Companion objects: where state-name constants and trailing comments live

Kotlin's class body doesn't naturally accept top-level (class-
scope) constants — those belong in a `companion object`. Frame's
generated Kotlin output uses a companion object for state-name
constants and any class-scope NativeBlocks (e.g., trailing
comments after the last action).

This is the Kotlin-specific quirk in section-comment preservation:
the codegen emits class-scope comment blocks into the companion
object rather than at class scope directly. See
`memory/section_comments_complete_2026_04_27.md` (the C++
trailing-comment newline fix has a Kotlin sibling).

You don't need to do anything special in your Frame source —
the codegen handles the placement automatically. The companion
object is private to the class and doesn't affect the public
API.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to a typed property
instantiated via direct call (no `new` keyword in Kotlin):

```kotlin
class Embedding {
    val counter: Counter = Counter()

    fun notify() {
        counter.bump(1)
    }
}
```

Calls to `self.counter.bump(n)` lower to `counter.bump(n)`. The
Phase 7 multi-system fuzz fix landed the Kotlin codegen for
this case (no-`new` in `expand_tagged_in_domain`); see
`memory/phase7_multisys_2026_04_27.md`.

---

## Loop idioms — both work

Kotlin has `while`, `do-while`, and range-based `for`. Frame's
idiom 1 (`while cond { ... }`) compiles to a native Kotlin
`while` block via passthrough.

```frame
$Counting {
    tick() {
        var i = 0
        while (i < 10) {
            i++
        }
    }
}
```

Kotlin-native braces and `i++` work inside the Frame `while`
block — no escaping needed.

---

## Multi-system per file: works as you'd expect

A `.fkt` source containing multiple `@@system` blocks compiles
to a single `.kt` file with multiple class definitions:

```frame
@@system Producer { ... }
@@system Consumer { ... }
```

Both `Producer` and `Consumer` end up as separate classes in
the same `.kt` output. Kotlin has no per-file structural
constraint analogous to Java's one-public-class rule.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Kotlin the same way it
applies to every other backend. The comment leaders are `//`
(line) and `/* ... */` (block) and `/** ... */` (KDoc).

```frame
@@[target("kotlin")]

import kotlin.system.exitProcess

// Module-prolog block — passes through as Kotlin source.

@@system Counter {
    machine:
        // Section-level comments are preserved as native // blocks.
        $Counting {
            tick() { ... }
        }
}
```

Section-level leading comments are preserved. Class-scope
trailing comments end up inside the companion object as noted
above.

---

## Idiomatic patterns and common gotchas

**No `new` for class instantiation.** `WithInterface()` (no
`new`) is the constructor call. Frame's `@@WithInterface()`
lowers to bare `WithInterface()`.

**`this.field`, not `self.field`.** Inside handler bodies,
Frame's `self.x` lowers to `this.x` (or just `x` since
Kotlin's resolution is implicit on member access). Native
passthrough should use `this.` if disambiguation is needed.

**`val` vs `var`.** `val` is read-only (Kotlin's `final`
equivalent — the reference cannot be reassigned, though the
referenced object may still be mutable). `var` is mutable.
Frame's domain fields use `var` since they're mutable
state. Handler-local declarations should use `val` where
possible for immutability hints.

**Type inference is your friend.** `var count = 0` inferred
to `Int`. `var name = "alice"` inferred to `String`. Don't
write the type unless you need a non-default (nullable,
`Long` instead of `Int`, etc).

**`null` is the absent-value marker, but only for nullable
types.** Kotlin's null safety means non-nullable types
(`String`) cannot hold null; nullable types (`String?`)
can. Use `?` deliberately.

**`?.` for safe navigation, `!!` for non-null assertions.**
A nullable field accessed via `s.parent?.bump(1)` is a no-op
if `parent` is null. `s.parent!!.bump(1)` throws
`NullPointerException` if null. Frame doesn't auto-emit
either form — write the safety operator you need in handler
bodies.

**`suspend fun` cannot be called from non-suspend context.**
If you target async, your test driver or caller must be
inside a coroutine scope (`runBlocking`, `launch`, etc).
Trying to call a `suspend fun` from a normal `fun` is a
compile error.

**Kotlin's stdlib is small.** `kotlin.collections` is
auto-imported (List, Map, Set, etc). Other utilities require
explicit `import` in the prolog. Common ones:
`kotlin.system.exitProcess`, `kotlin.io.println` (auto),
`kotlinx.coroutines.runBlocking`.

---

## Persist quiescent contract — E700

`save_state()` requires the system to be quiescent (no event in
flight, `_context_stack` empty). Calling it from inside a handler
throws `RuntimeException("E700: system not quiescent")`. Catchable
via `try/catch`, but recovery isn't possible — the handler's
context frame is corrupted; discard the instance and restore from
a prior snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend
  capability table; Kotlin shows ✅ on every row.
- `tests/common/positive/primary/02_interface.fkt` —
  canonical interface-method shape.
- `framec/src/frame_c/compiler/codegen/backends/kotlin.rs` —
  Kotlin backend codegen.
- `framepiler_test_env/docker/runners/kotlin_runner.sh` —
  matrix runner; classpath includes `kotlinx-coroutines-core`.
- `memory/phase6_async_2026_04_27.md` — context on Kotlin
  async wiring (Stage 5 of Phase 6).
- `memory/phase7_multisys_2026_04_27.md` — context on the
  Kotlin no-`new` cross-system field fix.
- `memory/section_comments_complete_2026_04_27.md` — context
  on the companion-object placement of class-scope
  NativeBlocks.
