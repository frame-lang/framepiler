# Per-Language Guide: Java

Java's class-file model and concurrency primitives are the two things
that most distinguish it from the broader C-family default. Java's
"one public class per file" rule means a `.fjava` source can declare
at most one `@@system` whose generated public class matches the file
name. Java's `CompletableFuture<T>` provides the async boundary,
which framec uses for `async` interface methods — but the internal
dispatch stays synchronous, with `.completedFuture(...)` wrapping the
result.

This guide documents the Java-specific patterns. It assumes you are
already familiar with Frame's core syntax and Java basics
(`public class`, `static`, `String`, `CompletableFuture`).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Java is
fully spec-conformant on the runtime; multi-system per file is the
only language-shape skip (`[j]`).

---

## Foundation: public class with member methods

A Frame system targeting Java generates a single `.java` file
containing:

- A `public class WithInterface { ... }` whose name must match the
  filename.
- Domain fields as private members.
- Public interface methods (`public String greet(String name)`).
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers as private methods.

```java
public class WithInterface {
    public WithInterface() {
        // start-state $> cascade fires here
    }

    public String greet(String name) {
        // ... handler body
    }

    private int call_count = 0;
    // ... runtime fields
}
```

Frame's `self.field` lowers to `this.field` in the generated Java.
Domain fields are private by default; interface methods are
public.

---

## E430: one `@@system` per file

Java requires the public class name to match the source filename.
A `.fjava` source can therefore declare exactly one `@@system`
whose name becomes the public class name. Two or more `@@system`
blocks in a single `.fjava` file are rejected at the framec stage
with **E430** ("Java requires one system per file").

```frame
@@[target("java")]

@@system Producer { ... }
@@[main]
@@system Consumer { ... }   // ← E430 fires here
```

To use multiple systems on Java:

1. Split each `@@system` into its own `.fjava` file.
2. Name each file to match the system name (Frame and Java both
   require this for the public class).
3. Compile them together as a multi-class application.

The framepiler matrix harness's "multi-system per file" tests
skip on Java with `@@skip -- java requires one system per file`;
this is a language-shape skip, not a framec gap. Cross-system
embedding via `var inner: Other = @@Other()` works as expected
when the systems live in separate files.

---

## Domain fields: typed members with default initializers

Domain fields live as private member variables on the class:

```frame
domain:
    call_count: int = 0
    name: String = "alice"
    items: List<Integer> = new ArrayList<>()
```

```java
private int call_count = 0;
private String name = "alice";
private List<Integer> items = new ArrayList<>();
```

The Frame `: type` annotation IS the Java type — write `: String`,
`: List<Integer>`, `: Map<String, Object>`, etc. Frame doesn't
auto-prefix `java.util.` — declare imports in the prolog.

**Frame type names map cleanly to Java types:**

| Frame             | Java                | Notes |
|-------------------|---------------------|-------|
| `int`             | `int`               | primitive 32-bit |
| `String`          | `String`            | preferred over `: str` for type-fidelity |
| `boolean`         | `boolean`           | primitive |
| `double`          | `double`            | primitive 64-bit |
| `List<T>`         | `java.util.List<T>` | needs `import java.util.List` |
| `Map<K,V>`        | `java.util.Map<K,V>`| needs import |

---

## Strings: `String + String` works directly

Java's `+` is overloaded for String concatenation, so Frame's
`+`-chain works the same way it does on Python/JS:

```frame
$Ready {
    greet(name: String): String {
        this.call_count += 1
        @@:("Hello, " + name + "!")
        return
    }
}
```

```java
public String greet(String name) {
    this.call_count += 1;
    return "Hello, " + name + "!";
}
```

For more complex formatting, use `String.format(...)` or
`String.join(...)`:

```frame
$Active {
    log() {
        this.message = String.format("count=%d, name=%s", this.count, this.name)
    }
}
```

`StringBuilder` is the right choice for tight loops that build up
strings — pass through verbatim if you need it.

---

## Async: `CompletableFuture<T>` — typed boundary, sync interior

Frame's `async` interface methods on Java return
`CompletableFuture<T>`. The wrapper sets up the context, calls
the synchronous dispatch chain, and wraps the return value in
`CompletableFuture.completedFuture(...)`:

```frame
async fetch(key: String): String {
    @@:return = await this.cache.get(key)
}
```

```java
public CompletableFuture<String> fetch(String key) {
    // ... synchronous handler body
    String __result = ...;
    return CompletableFuture.completedFuture(__result);
}
```

**Key design decisions** (per `memory/java_async_2026_04_26.md`):

- The internal dispatch chain stays synchronous. The
  `__kernel(__e)` call is plain Java method invocation, not
  `runAsync`. The async typing is a contract boundary, not an
  execution-model change.
- The constructor fires the start-state's `$>` cascade
  synchronously — two-phase init (separate `init()` returning
  `CompletableFuture<Void>`) buys nothing on a synchronous
  runtime.
- An `init()` method is also emitted, returning
  `CompletableFuture.completedFuture(null)`, for cross-language
  API parity. Callers writing `system.init().get()` portably
  work, but the body is a no-op since the constructor already
  drove initialization.
- No executor parameter, no Reactor, no RxJava. Pure JDK
  `java.util.concurrent.CompletableFuture` — users who need a
  custom executor wrap the system in a layer that provides one.

Implementation: `make_java_interface_async` in
`framec_native_codegen/system_codegen.rs`.

**For genuinely-async work** (I/O, long computation), the
synchronous-interior model means the future is "already
completed" on return. If your async handler needs to actually
yield, restructure to dispatch the work to an executor inside
the handler body and chain via `.thenApply(...)` —
framec doesn't auto-emit this pattern.

---

## Loop idioms — both work

Java has `while`, `for`, and (Java 5+) enhanced-for / range-for.
Frame's idiom 1 (`while cond { ... }`) compiles to a native
Java `while` block via passthrough.

```frame
$Counting {
    tick() {
        int i = 0;
        while (i < 10) {
            i++;
        }
    }
}
```

Java-native braces and `i++` work inside the Frame `while`
block — no escaping needed.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to a typed `Counter`
field instantiated via `new Counter()` in the embedder's
constructor:

```java
public class Embedding {
    private Counter counter;

    public Embedding() {
        this.counter = new Counter();
        // ... start state $> fires
    }

    public void notify() {
        this.counter.bump(1);
    }
}
```

Calls to `self.counter.bump(n)` lower to `this.counter.bump(n)`.
Lifecycle is the JVM garbage collector's responsibility.

Cross-system embedding requires the `Counter.java` file to live
alongside `Embedding.java`, since each `@@system` is one Java
class per file. The harness compiles both into the same
classpath.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Java the same way it applies to
every other backend. The comment leaders are `//` (line) and
`/* ... */` (block) and `/** ... */` (Javadoc).

```frame
@@[target("java")]

// Module-prolog block — passes through as Java source.
import java.util.List;
import java.util.concurrent.CompletableFuture;

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

**`new` is required for class instantiation.** Frame's
`@@WithInterface()` lowers to `new WithInterface()` in the test
driver. Don't write `WithInterface s = WithInterface()` — that's
not legal Java.

**`this.field`, not `self.field`.** Inside handler bodies, Frame's
`self.x` lowers to `this.x` since Java's instance reference is
`this`. If you write native Java inside a handler, use `this.`.

**Imports go in the prolog, before `@@system`.** Java's `import`
declarations are file-scope and must precede any class definition.
Frame's prolog (above `@@system`) is the natural home for
imports.

**Public-class name must match filename.** Save your `.fjava`
source with the same name as the `@@system` (e.g., `WithInterface.fjava`
for `@@system WithInterface`). Framec emits `WithInterface.java`
which Java's compiler will reject if the filename doesn't match
the public class name.

**`@@:return = expr` for typed returns.** Java's typed return
contract means a method declared `(): String` must return a
`String`. Frame's wrapper extracts the value from the
FrameContext's return slot — set it with `@@:return = expr` or
the shorthand `@@:(expr)` in the handler body.

**Boxing/unboxing happens automatically for primitives in
generic contexts.** A `List<Integer>` field with `int` values
gets auto-boxed via `Integer.valueOf(...)`. Frame doesn't
generate explicit boxing — Java's compiler handles it.

**`null` is the universal absent-value marker.** Java's `null`
serves the same role as Python's `None` and JavaScript's
`undefined`. Frame source can use `null` consistently for
default values.

**Casting may be needed for downstream calls.** Frame's
`@@:return = self.list.get(0)` from a `List<Object>` would need
explicit casting on the receiving side: `String s = (String)
system.foo()`. Frame doesn't auto-cast.

---

## Persist contract — `@@[save]` / `@@[load]`

A `@@[persist]` system must declare two operations under the
`operations:` section: one tagged `@@[save]` (returns the
serialized blob) and one tagged `@@[load]` (instance method
that mutates self from a blob). The op names are yours to
pick — these match the Java convention.

```frame
@@[persist]
@@system Counter {
    operations:
        @@[save]
        save_state(): String {}

        @@[load]
        restore_state(data: String) {}

    interface:  bump()
    machine:    $Active { bump() { self.n = self.n + 1 } }
    domain:     n: int = 0
}
```

Load is an instance method (allocate, then populate):

```java
Counter c2 = new Counter();
c2.restore_state(data);
```

The bare `@@[persist]` form (no `@@[save]` / `@@[load]` ops) is
rejected with **E814** since framepiler `b3aebc5` (2026-05-03).

### Post-load hook: `@@[on_load]`

A third optional attribute fires user code after
`restore_state` finishes populating self — useful for re-establishing
derived state, firing watchers, validating invariants:

```frame
operations:
    @@[save]    save_state(): String {}
    @@[load]    restore_state(data: String) {}

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

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Java shows ✅ on every row except multi-system per file
  (🚫[j]) and async ([f] — `CompletableFuture<T>`-typed boundary
  with sync interior, documented as the implementation choice).
- `tests/common/positive/primary/02_interface.fjava` — canonical
  interface-method shape with `String + String` concat.
- `tests/java/positive/async_basic.fjava` — async-method
  reference test exercising `CompletableFuture<T>`.
- `tests/common/positive/demos/19_async_http_client.fjava` —
  larger async demo.
- `framec/src/frame_c/compiler/codegen/system_codegen.rs` —
  `make_java_interface_async` is the entry point for the async
  wrapper emit.
- `memory/java_async_2026_04_26.md` — context on the async
  design decisions (sync interior, no executor parameter,
  `init()` for parity).
