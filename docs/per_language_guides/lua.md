# Per-Language Guide: Lua

Lua is the Frame target with the smallest standard library. There is
no `class` keyword (objects are tables with metatables), no `import`
statement (everything is global until you `require`), no built-in
async, no native list type beyond integer-indexed tables (1-indexed),
no string type beyond an opaque immutable byte sequence with a
metatable. The Frame source you write looks similar to other dynamic
targets, but the generated Lua leans heavily on Lua's
metatable-based OO convention — and a few language idioms (`:` vs
`.` method calls, `..` for concatenation, `1`-indexing) need to be
written into the source explicitly.

This guide documents the Lua-specific patterns. It assumes you are
already familiar with Frame's core syntax and basic Lua (tables,
metatables, `function`, `local`).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Lua is fully
spec-conformant on the runtime; the only language-natural skip is
`async` (no language-level async/await).

---

## Foundation: metatable-based class shape

A Frame system targeting Lua generates a single `.lua` module
containing:

- A "class" table (`Counter`) with a metatable whose `__index`
  points back to the class, enabling inheritance-style method
  dispatch.
- A `Counter.new(...)` constructor function returning an instance
  table whose metatable points at the class.
- One `Counter:<event>(...)` method per interface method (using
  the colon syntax that auto-passes `self`).
- Internal `Counter:_state_<S>(...)` and
  `Counter:_s_<S>_hdl_<kind>_<event>(...)` helpers for dispatch.

```lua
local Counter = {}
Counter.__index = Counter

function Counter.new(seed)
    local self = setmetatable({}, Counter)
    self.n = seed or 0
    -- ... start state $> cascade fires here
    return self
end

function Counter:bump(by)
    self.n = self.n + by
end

function Counter:get()
    return self.n
end

return Counter
```

Frame's `self.field` lowers to `self.field` (table-key access), and
method definitions use the colon syntax (`function Counter:method()`)
so `self` is auto-passed by callers using `instance:method()`.

---

## `:` vs `.` — colon for method calls, dot for field access

Lua has two function-call syntaxes that look similar but behave
differently:

- `instance.method(args)` — calls `method` with explicitly-passed
  arguments. `self` is *not* passed automatically.
- `instance:method(args)` — calls `method` with `self =
  instance` automatically prepended.

Frame's source uses dot syntax (`self.bump(n)`) but framec
generates **colon syntax** in the output for method calls on
`self` and on cross-system fields:

```lua
self:bump(n)             -- generated
self.counter:bump(n)     -- generated for cross-system embedding
```

This is the framec codegen handling — you do *not* write `:` in
Frame source. The native passthrough rule still requires colon
syntax for any handwritten Lua method calls inside handler
bodies, though:

```frame
$Active {
    log() {
        // Native Lua passthrough — must use : for self method calls.
        self:print_log()
    }
}
```

If you forget the colon, Lua will silently bind whatever was
positionally first as `self`, breaking the call.

---

## Domain fields: keys on the instance table

Domain fields live as keys on the instance table itself:

```frame
domain:
    var n: int = 0
    var name: str = "alice"
```

```lua
function Counter.new()
    local self = setmetatable({}, Counter)
    self.n = 0
    self.name = "alice"
    -- ...
    return self
end
```

Reads use `self.n`, writes use `self.n = ...` — same as every
dynamic target. Frame's `: type` annotation is documentation only;
Lua has no compile-time type system. The wrapper always returns a
value (per the dynamic-target return contract — see
`docs/frame_runtime.md`).

---

## Strings: `..` for concat, `tostring(...)` for coercion

Lua's string concatenation operator is `..` (two dots), not `+`.
Type coercion to string requires `tostring(...)` — concatenating a
number with a string raises an error otherwise:

```frame
$Ready {
    greet(name: str): str {
        self.call_count = self.call_count + 1
        @@:("Hello, " .. tostring(name) .. "!")
        return
    }
}
```

```lua
function Counter:greet(name)
    self.call_count = self.call_count + 1
    return "Hello, " .. tostring(name) .. "!"
end
```

For more complex formatting, use `string.format(fmt, ...)` (Lua's
`printf`-style helper):

```frame
$Active {
    log() {
        self.message = string.format("count=%d, name=%s", self.n, self.name)
    }
}
```

This also lowers verbatim — the prolog needs no `require` for
`string.format` (it's built in).

---

## State variables: `$.var`

State-scoped variables behave the same as on every other backend.
They live on the state's compartment, accessed as `$.var` in
handler bodies. Generated Lua stores compartment data on a sub-
table:

```lua
function Counter:_s_counting_hdl_event_tick(__e, compartment)
    compartment.count = compartment.count + 1
end
```

Multi-state state-vars work as expected. Nothing Lua-specific
beyond table-key access semantics.

---

## Loop idioms — both work

Lua has both `while cond do ... end` and `for i = 1, n do ... end`.
Frame's idiom 1 (`while cond { ... }`) compiles to a native Lua
`while` block via passthrough. Idiom 2 (state-flow loop) also
works the same way.

Note the Lua-native shape: `while cond do ... end` (no braces,
explicit `end` keyword). Frame's brace syntax (`while cond { ... }`)
in `.flua` source is the Frame-syntax wrapper — you write Frame's
`{ ... }` and framec emits Lua's `do ... end`:

```frame
$Counting {
    tick() {
        local i = 0
        while i < 10 {
            i = i + 1
        }
    }
}
```

```lua
function Counter:tick()
    local i = 0
    while i < 10 do
        i = i + 1
    end
end
```

If you prefer to write the Lua-native shape directly via Oceans
Model passthrough, escape outside Frame's brace structure — but
this is rarely worth it.

**Lua arrays are 1-indexed.** `t[1]` is the first element, `t[0]`
is unset by default. Frame doesn't auto-translate array indexes
across targets — your handler bodies need to use 1-indexing for
Lua even if you wrote 0-indexing for Python/JS in the same
project. This is the most common cross-target portability
gotcha.

---

## No async — language-natural skip

Lua has no language-level async/await. The matrix capability
table shows Lua's async row as 🚫 (language-natural skip).
Asynchronous work in Lua typically uses:

- Coroutines (`coroutine.create`/`coroutine.resume`/
  `coroutine.yield`) for cooperative multitasking inside a single
  OS thread.
- Embedding in a host that provides an event loop (`copas`,
  `lua-ev`, `luvit`).
- LuaSocket for I/O without an explicit async layer.

These are user concerns, not Frame's. Write the `require` calls in
the prolog and call into the libraries from handler bodies.

---

## Multi-system per file: works via separate class tables

A `.flua` source containing multiple `@@system` blocks compiles to
a single `.lua` file with multiple class tables, each `return`ing
its own metatable-based class:

```frame
@@system Producer { ... }
@@system Consumer { ... }
```

Both `Producer` and `Consumer` end up as separate class-tables in
the same `.lua` file. Lua has no per-file structural constraint
on multi-class definitions.

---

## Cross-system fields: nested instance tables

`var counter: Counter = @@Counter()` lowers to an instance-table
field on the embedding system's instance:

```lua
function Embedding.new()
    local self = setmetatable({}, Embedding)
    self.counter = Counter.new()
    -- ...
    return self
end
```

Calls to `self.counter.bump(n)` lower to `self.counter:bump(n)` —
the colon-call shape for method dispatch on the embedded
instance. This was a framec codegen fix — see
`memory/phase7_multisys_2026_04_27.md` for the round in which Lua's
chained-call colon emit landed.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Lua the same way it applies to
every other backend. The comment leaders are `--` (line) and
`--[[ ... ]]` (block).

```frame
@@target lua

-- Module-prolog block — passes through as Lua source.
local socket = require("socket")

@@system Counter {
    machine:
        -- Section-level comments are preserved into the generated
        -- .lua file as native -- comment blocks.
        $Counting {
            tick() { ... }
        }
}
```

Section-level leading comments are preserved into the generated
output as native `--` comment blocks attached to the
corresponding generated declaration.

---

## Idiomatic patterns and common gotchas

**Use `:` for method calls, `.` for field access.** Lua
distinguishes between the two — `instance.method(args)` does *not*
pass `self`. Frame source uses dot syntax which framec lowers to
colon, but native passthrough inside handler bodies must use
colon explicitly.

**`local` declarations do not survive into Frame domain.** Frame's
domain block declares fields on the instance (`self.field`).
Native `local x = 0` declarations in the prolog are file-scoped
locals, not instance fields. Use `var x: int = 0` in the domain
block to declare an instance field.

**Tables are 1-indexed by convention but not by enforcement.**
Lua tables can use any key (including 0 or negative integers).
Built-in functions like `ipairs`, `table.insert`, `table.remove`,
and `#t` (length operator) only work on 1-indexed sequences with
no gaps. Stick to 1-indexing unless you have a specific reason
not to.

**`nil` is the absence-of-value marker.** Lua's `nil` is the
type for unset / absent values, including unset table keys
(`t.missing == nil` returns `true`). Use `nil` consistently in
Frame source for default values; Frame doesn't auto-translate
`null` from cross-target sources.

**Method calls on `nil` raise immediately.** Lua doesn't have
optional chaining (`?.`). If `self.counter` could be `nil`, you
must check before the call:

```frame
$Active {
    notify() {
        if self.counter ~= nil {
            self.counter:bump(1)
        }
    }
}
```

The `~=` is Lua's not-equal operator (not `!=`). Native
passthrough — write Lua-native operators in your handler bodies.

**`print()` is the universal output.** Lua's built-in `print(...)`
takes any number of arguments and tab-separates them. For
cross-target test harnesses, `print("PASS")` works on Lua
identically to Python.

**`require()` returns a module's `return` value.** When
embedding cross-system fields, the generated `.lua` file `return`s
a class table at the bottom; users `require("counter")` to get
back the class table and call `Counter.new(...)` on it. Frame's
multi-system-per-file output uses `return { Producer = Producer,
Consumer = Consumer }` to expose multiple classes from one
module file.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Lua shows ✅ on every row except `async` (🚫 — language-
  natural skip).
- `tests/common/positive/primary/02_interface.flua` — canonical
  interface-method shape with `..` string concat.
- `tests/common/positive/primary/10_state_var_basic.flua` —
  state-var read/write pattern.
- `framepiler_test_env/docker/runners/TestRunner.lua` — single-
  process Lua test dispatcher used by the matrix harness;
  captures `print()` output for assertion checking.
- `framec/src/frame_c/compiler/codegen/backends/lua.rs` — Lua
  backend codegen, including the colon-call rewrite for
  cross-system method dispatch.
