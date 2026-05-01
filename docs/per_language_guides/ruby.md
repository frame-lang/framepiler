# Per-Language Guide: Ruby

Ruby's design — pure object-oriented (everything is an object), no
explicit `class` keyword required for top-level code, dynamic
typing, no parentheses required for method calls — makes it the
most concise of the dynamic Frame targets. Frame source for Ruby
mirrors Python's structural conventions but uses Ruby-specific
syntax inside handler bodies.

This guide documents the Ruby-specific patterns. It assumes you
are already familiar with Frame's core syntax and Ruby basics
(`class`, `def`, `self`, blocks, dynamic typing).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Ruby is
fully spec-conformant; `async` is the only language-natural skip.

---

## Foundation: class with `def` methods

A Frame system targeting Ruby generates a single `.rb` file
containing:

- A `class WithInterface` block.
- An `initialize` method (Ruby's constructor convention) that
  fires the start-state's `$>` cascade.
- One `def greet(name)` method per interface entry.
- Internal `def _state_<S>(...)` and
  `def _s_<S>_hdl_<kind>_<event>(...)` helpers.

```ruby
class WithInterface
    def initialize
        # start-state $> cascade fires here
        @call_count = 0
    end

    def greet(name)
        # ... handler body
        result
    end
end
```

Frame's `self.field` lowers to `@field` (Ruby's instance variable
convention — `@` prefix means an instance variable). Method
calls use `s.greet("World")`.

---

## Domain fields: instance variables (`@field`)

Domain fields lower to instance variables initialized in
`initialize`:

```frame
domain:
    call_count: int = 0
    name: str = "alice"
```

```ruby
def initialize
    @call_count = 0
    @name = "alice"
    # ... runtime fields
end
```

Reads use `@call_count`, writes use `@call_count = ...`. The
Frame `: type` annotation is documentation only; Ruby has no
compile-time type system.

---

## Strings: `+` or `<<` for concat, `#{expr}` interpolation

Ruby has multiple string concatenation operators:

- `"a" + "b"` — creates a new string (immutable).
- `"a" << "b"` — appends to the LHS (mutable).
- `"a#{var}b"` — interpolation inside double-quoted strings.

```frame
$Ready {
    greet(name: str): str {
        @call_count += 1
        @@:("Hello, #{name}!")
        return
    }
}
```

```ruby
def greet(name)
    @call_count += 1
    "Hello, #{name}!"
end
```

Ruby's interpolation syntax `#{expr}` is identical to PHP's
within double-quoted strings. Single-quoted strings do *not*
interpolate.

---

## No async — language-natural skip

Ruby has no native async/await. Asynchronous work uses:

- Threads (`Thread.new { ... }`) — OS-level concurrency.
- Fibers (Ruby 1.9+) — cooperative coroutines.
- EventMachine — event-loop library.
- Async (the gem) — Ruby 3.0+ async fiber library.

Capability matrix: `async` → 🚫 for Ruby (language-natural skip).

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to an instance variable
instantiated in `initialize`:

```ruby
def initialize
    @counter = Counter.new
    # start-state $> fires
end

def notify
    @counter.bump(1)
end
```

Note the `.new` constructor call — Ruby's class instantiation
idiom. Calls to `self.counter.bump(n)` lower to `@counter.bump(n)`.

---

## Loop idioms — both work

Ruby has `while`, `until`, and various block-based iteration
methods (`each`, `times`, etc). Frame's idiom 1 (`while cond
{ ... }`) compiles to a native Ruby `while` block via passthrough
— but note Ruby's `while` syntax uses `do ... end` or
`while cond ... end` (no braces):

```frame
$Counting {
    tick() {
        i = 0
        while i < 10
            i += 1
        end
    }
}
```

For idiomatic Ruby iteration, prefer `.times` or `.each` via
Oceans Model passthrough:

```ruby
10.times { |i| ... }
[1, 2, 3].each { |x| ... }
```

---

## Multi-system per file: works as you'd expect

A `.frb` source containing multiple `@@system` blocks compiles
to a single `.rb` file with multiple class definitions.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Ruby the same way it applies
to every other backend. The comment leader is `#`.

```frame
@@[target("ruby")]

# Module-prolog block — passes through as Ruby source.

@@system Counter {
    machine:
        # Section-level comments are preserved as native # blocks.
        $Counting {
            tick() { ... }
        }
}
```

---

## Idiomatic patterns and common gotchas

**`@field`, not `self.field`.** Ruby's instance variables use
the `@` prefix. Frame's `self.x` lowers to `@x`. The bare
`self.x` form in Ruby refers to a method call, not field
access.

**`.new` for class instantiation.** `Counter.new` (not
`Counter()` or `new Counter()`). Frame's `@@Counter()` lowers
to `Counter.new`.

**Implicit return from methods.** Ruby methods return the value
of the last expression evaluated. Frame's `@@:(expr)` lowers
to setting the return slot which Ruby's wrapper returns
implicitly. You don't need to write `return expr`.

**`nil` is the absent-value marker.** Ruby's `nil` is the
universal nil-value; methods called on `nil` raise
`NoMethodError`.

**`puts` for output (with newline), `print` without.** Ruby's
`puts` adds a trailing newline; `print` does not. Test drivers
use `puts` for line-oriented assertion output.

**Symbols (`:foo`) vs strings.** Ruby's symbols are interned
identifier-like values (cheaper than strings for hash keys).
Frame doesn't auto-emit symbols — use strings unless you have
a specific reason.

**`require` for loading external code.** Ruby's `require 'gem'`
loads a gem; `require_relative './file'` loads a relative file.
Use the prolog for `require` directives.

---

## Persist quiescent contract — E700

`save_state` requires the system to be quiescent (no event in
flight, `@_context_stack` empty). Calling it from inside a handler
raises `RuntimeError` with message `"E700: system not quiescent"`.
Catchable via `begin/rescue`, but recovery isn't possible — the
handler's context frame is corrupted; discard the instance and
restore from a prior snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Ruby shows ✅ except `async` (🚫 — language-natural
  skip).
- `tests/common/positive/primary/02_interface.frb` — canonical
  interface-method shape with `#{...}` interpolation.
- `framec/src/frame_c/compiler/codegen/backends/ruby.rs` —
  Ruby backend codegen.
