# Per-Language Guide: C++

Frame's C++ backend targets C++17 by default and C++20 for async
features (coroutines). Unlike C, C++ has a real `class` keyword,
RAII-managed objects, `std::string` for string handling, and
`shared_ptr<T>` for cross-system embedding without manual lifecycle
management. The Frame source you write looks similar to other
C-family targets, but the generated C++ is notably ergonomic
compared to the C output: stack allocation, automatic destructors,
templates, namespaces.

This guide documents the C++-specific patterns. It assumes you are
already familiar with Frame's core syntax and modern C++ (`auto`,
`std::string`, `shared_ptr`, `unique_ptr`, `co_await`).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. C++ is
fully spec-conformant on every row.

---

## Foundation: stack-allocated class with member methods

A Frame system targeting C++ generates a single `.cpp` file (or
`.h` + `.cpp` pair, configurable) containing:

- A `class WithInterface { ... }` with private domain fields,
  public interface methods, and a default constructor that fires
  the start-state's `$>` cascade.
- One `std::string greet(std::string name)`-style method per
  interface entry.
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers as private members.

```cpp
class WithInterface {
public:
    WithInterface() {
        // start-state $> cascade fires here
    }

    std::string greet(std::string name) {
        // ... handler body
    }

private:
    int call_count = 0;
    // ... runtime fields
};
```

Frame's `self.field` lowers to bare `field` (implicit `this` in C++
member methods) — no explicit `this->` is required, though the
generated code uses `this->` for clarity in some contexts.
Instances can be stack-allocated:

```cpp
WithInterface s;
auto result = s.greet("World");
```

This is more ergonomic than C — no manual `_new`/`_destroy`
lifecycle, no pointer dereferences, automatic destructor
generation.

---

## Domain fields: typed members with default initializers

Domain fields live as member variables on the class:

```frame
domain:
    call_count: int = 0
    name: std::string = "alice"
```

```cpp
private:
    int call_count = 0;
    std::string name = "alice";
```

Reads and writes use the bare field name inside member methods
(`call_count += 1`). The Frame `: type` annotation IS the C++
type — you write `: std::string`, `: std::vector<int>`, etc.

**Frame's type names map cleanly to C++ types:**

| Frame             | C++             | Notes |
|-------------------|-----------------|-------|
| `int`             | `int`           | platform `int` width |
| `std::string`     | `std::string`   | preferred; `: str` also works for cross-target Frame |
| `bool`            | `bool`          | |
| `double`          | `double`        | |
| `std::vector<T>`  | `std::vector<T>`| Frame passes through verbatim |

For STL types, declare them with the `std::` prefix in the Frame
source. Frame doesn't auto-prefix — it passes the type string
through verbatim.

---

## Strings: `std::string` with operator overloading

C++'s `std::string` overloads `+` for concatenation, so Frame's
`+` works the same way it does on Python/JS:

```frame
$Ready {
    greet(name: std::string): std::string {
        @@:(std::string("Hello, ") + name + "!")
        return
    }
}
```

```cpp
return std::string("Hello, ") + name + "!";
```

The leading `std::string("Hello, ")` is needed because the bare
`"Hello, "` literal is `const char*` — `+` would invoke pointer
arithmetic. Once any operand is a `std::string`, the rest of the
chain promotes.

For more complex formatting, use `std::format` (C++20) or
`std::stringstream`:

```frame
$Active {
    log() {
        std::stringstream ss;
        ss << "count=" << count_ << ", name=" << name_;
        message = ss.str();
    }
}
```

Pass through verbatim — `<<` is C++'s stream insertion operator,
nothing Frame-specific.

---

## Cross-system fields: `std::shared_ptr<T>`

`var counter: Counter = @@Counter()` lowers to a `std::shared_ptr<Counter>`
member field, allocated via `std::make_shared<Counter>(...)`:

```cpp
class Embedding {
private:
    std::shared_ptr<Counter> counter;

public:
    Embedding() : counter(std::make_shared<Counter>()) {}

    void notify() {
        counter->bump(1);
    }
};
```

Calls to `self.counter.bump(n)` lower to `counter->bump(n)` —
shared_ptr's overloaded `->` operator gives you direct access to
the underlying object's methods.

The `shared_ptr<T>` shape:

- Reference-counted, automatic deletion when the count hits zero.
- Safe to pass by value — copying just bumps the refcount.
- No explicit destroy/free needed — RAII handles lifecycle.

This was a framec codegen fix in the Phase 7 multi-system round
(see `memory/phase7_multisys_2026_04_27.md`); cross-system fields
on C++ now use `shared_ptr<T>` consistently.

If you need a unique-ownership shape, hand-edit the prolog or
declare the field's type explicitly as
`std::unique_ptr<Counter>` — but this requires custom move-only
semantics that framec doesn't auto-generate.

---

## Async: C++20 coroutines with `co_await`

Frame's `async` interface methods on C++ require C++20 (for
coroutine support) and lower to coroutine-typed return values:

```frame
async fetch(key: std::string): std::string {
    @@:return = co_await self.cache.get(key)
}
```

```cpp
FrameTask<std::string> fetch(std::string key) {
    auto __result = co_await cache->get(key);
    co_return __result;
}
```

The `FrameTask<T>` type is a Frame-runtime-provided coroutine
type — implementation lives in the generated module's preamble.
It wraps the C++20 promise/awaitable contract and exposes
`.get()` for synchronous extraction.

The C++20 coroutine machinery is more involved than Python's
`async def` or Rust's `async fn`:

- The compiler-driven `promise_type`, `final_suspend`, and
  `await_transform` must be defined; framec's runtime preamble
  provides these.
- The `co_await` expression is a postfix-style operator like
  Rust's `.await`, but written prefix in the C++ syntax.
- Linker setup typically requires `-fcoroutines` (gcc) or
  `-fcoroutines-ts` (clang older versions).

For the Frame test matrix, this is wired up via
`docker/runners/cpp_runner.sh`. See
`memory/audit_phase8_2026_04_26.md` for the C++ async wiring
context.

---

## Loop idioms — both work; idiom 1 is natural

C++ has `for`, `while`, `do-while`, and (C++11+) range-based
`for`. Frame's idiom 1 (`while cond { ... }`) compiles to a
native C++ `while` block via passthrough:

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

C++-native braces and `i++` work inside the Frame `while` block —
no escaping needed.

---

## Multi-system per file: works as you'd expect

A `.fcpp` source containing multiple `@@system` blocks compiles
to a single `.cpp` file with multiple class definitions:

```frame
@@system Producer { ... }
@@[main]
@@system Consumer { ... }
```

Both `Producer` and `Consumer` end up as separate `class` blocks
in the same `.cpp` output. C++ has no per-file structural
constraint on class definitions.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to C++ the same way it applies to
every other backend. The comment leaders are `//` (line) and
`/* ... */` (block).

```frame
@@[target("cpp_17")]

// Module-prolog block — passes through as C++ source.
#include <iostream>
#include <string>

@@system Counter {
    machine:
        // Section-level comments are preserved into the generated
        // .cpp file as native // comment blocks.
        $Counting {
            tick() { ... }
        }
}
```

**Trailing-comment newline fix.** A class-scope NativeBlock for a
trailing source comment after the last action used to glue the
class-close `};` onto the comment line:

```cpp
// ... structural safety property.};   ← bug
```

The C++ NativeBlock emit was fixed in commit `08ed071` to
guarantee a trailing newline; see
`memory/section_comments_complete_2026_04_27.md` for context.
The fix is idempotent — emit appends a newline if the joined
output doesn't end with one.

---

## Idiomatic patterns and common gotchas

**Stack allocation by default.** Frame systems on C++ are
stack-allocatable — `WithInterface s;` works without `new`. The
default constructor fires the start-state's `$>` cascade. If you
need heap allocation, use `auto s = std::make_unique<WithInterface>()`
explicitly.

**`std::string` literals need explicit construction sometimes.**
`"hello"` is `const char*`; `std::string("hello")` is `std::string`.
For the first operand of a `+`-chain, use the constructor; the
rest of the chain promotes via overloads.

**Implicit `this->` works inside member methods.** Frame's
`self.field` lowers to bare `field` access in the generated C++.
You do not need to write `this->field` in handler bodies (though
the codegen does for clarity in some emitted code paths).

**Domain field defaults use C++11 in-class initializers.** A
domain field declared `count: int = 0` lowers to `int count = 0;`
on the class — the C++11 in-class initializer syntax. For
non-trivial constructors (`std::vector<int>` with brace-init),
the syntax is `std::vector<int> v = {1, 2, 3};` — supported on
C++11 and later.

**`assert()` is a macro.** `<cassert>`'s `assert(cond)` aborts on
failure in debug builds, no-ops in release. For matrix tests,
debug builds are the default; for shipping code, prefer explicit
error handling.

**No exceptions in handler bodies by default.** Frame doesn't
emit `throw` or `try`/`catch` itself. If you write a handler
body that calls into an exception-throwing API, write the
`try`/`catch` explicitly via Oceans Model passthrough — Frame's
state machine semantics don't interact with C++ exception
handling.

**Coroutine compilation requires C++20.** If you target async
features, ensure your build uses `-std=c++20` (gcc/clang) or
the equivalent compiler flag. Older C++ standards don't have
`co_await` and the generated code won't compile.

---

## Persist contract — `@@[save]` / `@@[load]`

A `@@[persist]` system must declare two operations under the
`operations:` section: one tagged `@@[save]` (returns the
serialized blob) and one tagged `@@[load]` (instance method
that mutates self from a blob). The op names are yours to
pick — these match the C++ convention.

```frame
@@[persist]
@@system Counter {
    operations:
        @@[save]
        save_state(): std::string {}

        @@[load]
        restore_state(data: std::string) {}

    interface:  bump()
    machine:    $Active { bump() { self.n = self.n + 1 } }
    domain:     n: int = 0
}
```

Load is an instance method (allocate, then populate):

```cpp
Counter c2;
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
    @@[save]    save_state(): std::string {}
    @@[load]    restore_state(data: std::string) {}

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
throws `std::runtime_error("E700: system not quiescent")`.
Catchable via `try/catch`, but recovery isn't possible — the
handler's context frame is corrupted; discard the instance and
restore from a prior snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; C++ shows ✅ on every row.
- `tests/common/positive/primary/02_interface.fcpp` — canonical
  interface-method shape with `std::string` concat.
- `tests/common/positive/linux/06_oom_killer.fcpp` —
  multi-section comment preservation regression fixture (the
  trailing-comment newline fix).
- `framec/src/frame_c/compiler/codegen/backends/cpp.rs` — C++
  backend codegen.
- `framepiler_test_env/docker/runners/cpp_runner.sh` — matrix
  runner; uses `g++` or `clang++` with C++17 default and C++20
  for async tests.
- `memory/section_comments_complete_2026_04_27.md` — context on
  the trailing-comment newline fix.
- `memory/phase7_multisys_2026_04_27.md` — context on the
  shared_ptr cross-system field fix.
