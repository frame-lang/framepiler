# Per-Language Guide: Python

Python is the de facto baseline for most Frame documentation —
the cookbook examples, the runtime spec walkthroughs, and the
matrix's primary fixtures all use Python first. The Python target
maps cleanly to Frame's structural model: classes for systems,
methods for events, dynamic typing, `async def` / `await` for
async, `f"..."` for interpolation.

This guide documents the Python-specific patterns. Most are
unsurprising to anyone familiar with modern Python (3.7+).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. Python is
fully spec-conformant on every row.

---

## Foundation: class with method dispatch

A Frame system targeting Python generates a single `.py` file
containing:

- A `class WithInterface:` block.
- An `__init__(self)` constructor that fires the start-state's
  `$>` cascade.
- One `def greet(self, name)` method per interface entry.
- Internal `_state_<S>(self, ...)` and
  `_s_<S>_hdl_<kind>_<event>(self, ...)` helpers.

```python
class WithInterface:
    def __init__(self):
        # start-state $> cascade fires here
        self.call_count = 0

    def greet(self, name):
        # ... handler body
        return result
```

Frame's `self.field` lowers to `self.field` directly (Python's
instance reference is `self`). Method calls use `s.greet("World")`.

---

## Domain fields: dynamic attributes set in `__init__`

Domain fields lower to attributes set in `__init__`:

```frame
domain:
    call_count: int = 0
    name: str = "alice"
```

```python
def __init__(self):
    self.call_count = 0
    self.name = "alice"
    # ... runtime fields
```

Frame's `: type` annotation is documentation only — Python is
dynamically typed at runtime, though type hints (PEP 484) are
respected by static analyzers like mypy.

---

## Strings: `+` for concat, `f"..."` interpolation

Python's `+` operator concatenates strings. F-strings (Python 3.6+)
provide interpolation:

```frame
$Ready {
    greet(name: str): str {
        self.call_count += 1
        @@:(f"Hello, {name}!")
        return
    }
}
```

```python
def greet(self, name):
    self.call_count += 1
    return f"Hello, {name}!"
```

For older Python versions, `"Hello, {}!".format(name)` and
`"Hello, %s!" % name` also work. F-strings are preferred for
readability.

---

## Async: `async def` with `await`

Frame's `async` interface methods on Python lower to `async def`
methods returning coroutines:

```frame
async fetch(key: str): str {
    @@:return = await self.cache.get(key)
}
```

```python
async def fetch(self, key):
    __result = await self.cache.get(key)
    return __result
```

Python async is mature:

- `async def` for coroutine-returning functions.
- `await EXPR` at call sites.
- `asyncio.run(...)` to drive from sync code.
- The matrix harness uses `asyncio.run(main())` for async test
  drivers.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to an instance
attribute:

```python
def __init__(self):
    self.counter = Counter()
    # start-state $> fires

def notify(self):
    self.counter.bump(1)
```

---

## Loop idioms — both work

Python has `while`, `for-in`, and various iterator protocols.
Frame's idiom 1 (`while cond { ... }`) compiles to a Python
`while cond:` block via passthrough.

---

## Multi-system per file: works as you'd expect

A `.fpy` source containing multiple `@@system` blocks compiles
to a single `.py` file with multiple class definitions.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to Python the same way it applies
to every other backend. The comment leader is `#` (line); for
docstrings, use `"""..."""` (triple-quoted).

```frame
@@[target("python_3")]

# Module-prolog block — passes through as Python source.

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

**`self.field` everywhere.** Python's explicit `self` argument
on methods means handler bodies always reference `self.x`
explicitly. Frame's codegen handles this.

**No `new` keyword.** `Counter()` is the constructor call.
Frame's `@@Counter()` lowers to `Counter()`.

**Indentation matters.** Python is whitespace-sensitive. Frame
generates correct indentation; if you write native Python in
handler bodies, watch for tab/space consistency.

**`None` is the absent-value marker.** Python's `None` is the
universal nil-value.

**`print(...)` for output.** Built-in, no import needed.

**Common imports: `import os, sys, asyncio`.** Use the prolog
for `import` declarations.

---

## Persist quiescent contract — E700

`save_state()` requires the system to be quiescent (no event in
flight, `_context_stack` empty). Calling it from inside a handler
raises `RuntimeError("E700: system not quiescent")`. The error is
catchable via `try/except`, but recovery isn't possible — the
handler's context frame is corrupted; discard the instance and
restore from a prior snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

## ⚠️  Persist uses pickle — security warning

Frame's Python `save_state()` returns `pickle.dumps(self)` and
`restore_state()` calls `pickle.loads(blob)`. **`pickle.loads`
runs arbitrary code from the input** — a documented Python
vulnerability (see Python's pickle docs: "Never unpickle data
that could have come from an untrusted source").

If you persist Frame system state to **any** storage that an
attacker might write to — network sockets, user-uploaded files,
shared databases, browser cookies, IPC channels — you have an
RCE vector. An attacker crafts a pickle blob that runs arbitrary
Python on `restore_state()`.

**Safe uses:**
- Local files written and read by the same trusted process.
- Process-snapshot-for-crash-recovery on a single machine.
- Test fixtures.

**Unsafe uses without additional validation:**
- Session state in cookies.
- Cross-process / cross-machine state transfer.
- Anything the user can edit.

**Mitigation:** today, validate the source before calling
`restore_state`. RFC-0012 captures a proposed migration to
JSON-based persist that closes this hole; it's deferred pending
customer feedback. See `docs/rfcs/rfc-0012.md` "Python: switch
from pickle to JSON-based persist."

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; Python shows ✅ on every row.
- `tests/common/positive/primary/02_interface.fpy` — canonical
  interface-method shape with f-string interpolation.
- `framec/src/frame_c/compiler/codegen/backends/python.rs` —
  Python backend codegen.
- `memory/python_runner_fix_2026_04_26.md` — context on the
  Python test runner fix that resolved 282 silent no-op tests.
