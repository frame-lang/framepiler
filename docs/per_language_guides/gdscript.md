# Per-Language Guide: GDScript

GDScript is the only Frame target whose runtime is an *engine*, not a
language interpreter. Every generated `.gd` script runs inside Godot —
compiled lazily by Godot's GDScriptParser, executed by the engine's
script VM, and dispatched alongside scenes, signals, and the
SceneTree. This shapes the Frame source you write in non-obvious
ways: reserved method names will silently override `Object` built-ins,
`extends` is required at the script's top, and the headless test
harness wires up via `func _init():` rather than a `main()`.

This guide documents the GDScript-specific idioms, constraints, and
patterns. It assumes you are already familiar with Frame's core
syntax and basic Godot scripting concepts (`extends`, `func _init()`,
`SceneTree`).

For the canonical capability table, see
`framec-test-env/docs/runtime-capability-matrix.md`. GDScript is
fully spec-conformant on the runtime — the only language-specific
hazards are E501 (reserved methods) and Godot's dynamic dispatch
model.

---

## Foundation: every script runs inside Godot

A Frame system targeting GDScript generates a single `.gd` script
containing:

- A class extending the script's `extends` directive (you write
  this in the prolog — usually `extends SceneTree` for headless
  tests, or `extends Node`/`extends Node2D` for in-game state
  machines).
- One inner method per state (`func _state_<S>(__e, compartment):`)
  + per-handler methods (`func _s_<S>_hdl_<kind>_<event>(...):`).
- `func _init():` as the constructor — fires the start-state's `$>`
  cascade automatically (the same as `__init__` on Python or `new`
  on JS).
- The dispatch `__kernel(...)` and `__router(...)` are pure GDScript
  code; nothing dispatches via Godot signals or `_process(...)`
  callbacks unless you wire that up yourself in the prolog.

```gdscript
extends SceneTree

class FrameCounter:
    var _state = "counting"
    var __compartment = ...

    func _init():
        # start-state $> cascade fires here
        ...

    func tick():
        var __e = FrameEvent.new("tick", null)
        __kernel(__e)
        ...
```

The Frame state machine is just a regular GDScript class. You
instantiate it, you call methods on it, and Godot's script VM
handles the rest. There is no engine-level magic.

---

## E501: GDScript reserved methods are a real footgun

Every GDScript class inherits from `Object` (or `RefCounted` in
Godot 4), which carries a battery of built-in methods used by the
engine's reflection: `get`, `set`, `call`, `connect`, `free`,
`to_string`, etc. If a Frame interface method has one of those
names, it silently overrides the `Object` method and every call
site that does `obj.get("foo")` ends up routed through the user's
Frame interface method instead of the engine's reflection method.
This is a classic Godot footgun.

Framec catches this with **E501** at the Frame stage:

```frame
@@[target("gdscript")]

@@system Bad {
    interface:
        get(): str        # ← E501: collides with Object.get
}
```

```
E501: Interface method 'get' in system 'Bad' collides with GDScript's
built-in `Object.get` method. Calls like `obj.get(...)` would silently
invoke the Frame interface method instead of the engine method,
breaking core GDScript reflection. Rename it (suggested: 'get_value').
```

The validator carries a curated list of reserved names with
suggested renames; see
`framec/.../frame_validator.rs::gdscript_reserved_method_rename`
for the full table. Common ones:

| Frame name        | Reserved method group        | Suggested rename     |
|-------------------|------------------------------|----------------------|
| `get` / `set`     | property reflection          | `get_value` / `set_value` |
| `call` / `callv`  | method dispatch              | `invoke` / `invoke_with_args` |
| `connect` / `emit_signal` | signals             | `connect_handler` / `emit` |
| `free` / `queue_free` | object lifecycle         | `dispose` / `schedule_free` |
| `get_class` / `is_class` | class reflection      | `class_name` / `is_a` |
| `to_string`       | stringification              | `describe`           |

If your domain genuinely needs to expose `get`/`set` semantics
(for property reflection on the state machine itself), declare
the interface method with the rename and call the engine's `get`
explicitly via `Object.get(self, ...)` if you ever need it.

---

## Constructor: `func _init():` fires the start-state cascade

Frame's `_init()` constructor fires the start-state's `$>` cascade
during construction, the same way other backends do (`__init__` on
Python, `new()` on Rust). On GDScript this means:

- Instantiating with `var s = FrameMySystem.new()` runs the start
  state's enter handler immediately.
- For a `SceneTree`-extending headless harness, you typically call
  the system from inside the harness's `_init()`:

```gdscript
extends SceneTree

func _init():
    var s = FrameMySystem.new()    # fires $> on start state
    s.do_thing()
    quit()                          # exit the headless engine
```

The `quit()` call is essential — without it, a `SceneTree`
script will hold the engine open indefinitely.

For in-game state machines (`extends Node` etc), use `_ready()` or
your scene-script's lifecycle to instantiate the state machine
when the scene loads.

---

## Domain fields: dynamic typing, but `: type` is documentation

GDScript has optional static typing (Godot 4.x). Frame's `: type`
annotations on domain fields lower to GDScript type annotations:

```frame
domain:
    var n: int = 0
    var name: str = "alice"
    var items: list = []
```

```gdscript
var n: int = 0
var name: String = "alice"
var items: Array = []
```

Reads use `self.field`, writes use `self.field = ...` — same as
every C-family target. The `: type` annotation is enforced by
Godot's runtime if you opt into typed mode, but most user-written
GDScript leans on the dynamic side.

**Frame's type names map to GDScript types:**

| Frame  | GDScript     | Notes |
|--------|--------------|-------|
| `int`  | `int`        | 64-bit integer |
| `str`  | `String`     | always Godot's `String`, not `StringName` |
| `bool` | `bool`       | |
| `float`| `float`      | 64-bit float |
| `list` | `Array`      | dynamic, untyped |
| `map`  | `Dictionary` | |

For Godot-specific types (`Vector2`, `Vector3`, `Color`, `Node`,
etc), declare the type with the engine name as the Frame `: type`
string — Frame passes it through as an opaque type:

```frame
domain:
    var pos: Vector2 = Vector2(0, 0)
    var owner: Node = null
```

Framec emits these verbatim. Godot resolves them at script-load
time.

---

## State variables: `$.var`

State-scoped variables behave the same as on every other backend —
they live on the state's compartment and are accessed via `$.var`
in handler bodies, lowering to compartment field access in the
generated script:

```gdscript
func _s_counting_hdl_event_tick(self, __e, compartment):
    compartment.count += 1
```

Multi-state state-vars work as expected. Nothing GDScript-specific.

---

## Loop idioms — both work

GDScript has both `while` (idiom 1) and supports the state-flow
loop (idiom 2). Either is fine, with the usual cross-target
caveat that idiom 2 is more portable.

```frame
$Counting {
    tick() {
        var n: int = 0
        while n < 10 {
            n = n + 1
        }
    }
}
```

GDScript's `while` works exactly as you'd expect. Pass-through is
verbatim.

---

## Async — supported via Godot's `await`

GDScript 4.x has an `await` keyword for coroutines, used primarily
for waiting on signals. Frame's `async` interface methods lower to
`await` on GDScript:

```frame
async fetch(key: str) {
    @@:return = await self.cache.get(key)
}
```

```gdscript
func fetch(key: String):
    var __result = await self.cache.get(key)
    ...
```

The implementation uses GDScript's signal-coroutine model — your
async target needs to return a value that GDScript can `await` on
(typically a signal, a `Tween`, or another `await`-able call). For
purely synchronous "test" async, the harness wraps the return in
a `signal` emission or `Engine.get_main_loop().process_frame`
yield.

The capability matrix shows GDScript async as ✅ (Stage 5 of Phase 6
async wiring). See `tests/common/positive/cross_backend/`
async fixtures for canonical examples.

---

## Multi-system per file: pick a primary with `@@[main]`

GDScript scripts privilege one class per file: the script-level
`extends` directive, `_init()`, and module-scope `var`/`func`
declarations all belong to a single class identity. A `.fgd` source
with multiple `@@system` blocks therefore needs one — and only one —
system marked as the file's primary. Per RFC-0014, that's
`@@[main]`:

```frame
@@system Sub {
    interface: ping(): int
    machine:
        $Idle { ping(): int { @@:(42) } }
}

@@[main]
@@system Top {
    interface: run(): int
    machine:
        $Active { run(): int { @@:(self.sub.ping()) } }
    domain:
        sub: Sub = @@Sub()
}
```

The `@@[main]` system emits at script-module scope. Every other
system wraps as an inner class (`class Sub extends RefCounted:`)
beneath it. Cross-references inside `@@[main]` resolve to inner
classes (`var sub = Sub.new()` finds `Sub` as a sibling); cross-
references between non-main systems resolve as sibling inner classes.

**Without `@@[main]`** in a multi-system file, framec rejects the
build with **E805**:

```
E805: Module declares 2 systems (Sub, Top) but no `@@[main]` attribute.
Add `@@[main]` above the system that callers should instantiate via
the module's primary entry point. For GDScript this is the system
returned by `preload("<file>.gd").new()`.
```

Single-system files don't need the attribute — the lone system is
implicitly primary. Adding `@@[main]` to a single-system file is
allowed (redundant but harmless) and simplifies the rule.

**Two `@@[main]`** in the same file is **E806** — only one system
per file may occupy the script-level slot.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to
`self.counter = FrameCounter.new()` — direct construction, no
indirection. Calls to `self.counter.bump()` lower to
`self.counter.bump()` in the generated script. GDScript's dynamic
dispatch makes this trivial.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to GDScript the same way it applies
to every other backend. The comment leader for native code is
`#`.

```frame
@@[target("gdscript")]
extends SceneTree

# Module-prolog block — passes through as GDScript source.
# Use `#` for line comments.

@@system Counter {
    machine:
        # Comments inside @@system blocks are also `#` comments.
        $Counting {
            tick() { ... }
        }
}
```

Section-level leading comments are preserved as native `#`
comment blocks attached to the corresponding generated declaration.

---

## Tooling: headless Godot for matrix tests

The Frame test matrix runs GDScript fixtures through a headless
Godot binary in batch mode. The harness convention is:

1. The fixture script must `extends SceneTree`.
2. `func _init()` is the entry point.
3. After running the test, call `quit()` to terminate the engine.
4. Print PASS / FAIL via `print(...)`; the runner classifies
   based on stdout and exit code.

```gdscript
extends SceneTree

func _init():
    var s = FrameMySystem.new()
    var result = s.do_thing()
    if result == expected:
        print("PASS")
    else:
        print("FAIL: got " + str(result))
        quit(1)
        return
    quit()
```

For non-headless development (running inside the Godot editor),
the same script attached to a scene's root node would work via
`_ready()` instead of `_init()`. Frame doesn't impose either
convention — both compile from the same `.fgd` source if the
prolog declares the appropriate `extends`.

See `framec-test-env/docker/runners/gdscript_runner.sh` for
the matrix-side mechanism.

### Cross-file fixtures — pre-import is required

Frame's RFC-0022 `@@import "other.fgd"` lowers to GDScript's
`const X = preload("res://other.gd")` form. The generated
factory bodies (`static func _create(): var c = SystemName.new()`)
reference each system by its `class_name`, which Godot only
registers as a global identifier *after* it has imported the
project. For a CLI-only workflow without an editor pass, run a
pre-import step before any `--script` invocation:

```bash
godot --headless --path . --import         # populate .godot/ cache
godot --headless --path . --script smoke.gd
```

Without the import pass, `class_name App` is unknown inside the
script's own `static func _create()` body and the run fails with
`Identifier not found: App`. The matrix test harness already does
the import step; this note is for ad-hoc CLI smoke harnesses
exercising cross-file composition.

---

## Idiomatic patterns and common gotchas

**Always declare an `extends ...` in the prolog.** A GDScript
file without `extends` defaults to `extends RefCounted`, which is
fine for utility classes but won't run as a script via
`godot --script` without a `SceneTree` parent. For matrix tests,
use `extends SceneTree`. For in-game state machines, use the
appropriate Node subclass.

**`func _init()` is your constructor.** GDScript classes don't
have an explicit `constructor` keyword — `_init()` is the
convention. Frame generates this automatically; if you write
your own `_init()` in the prolog, append your code or restructure
to call `_init()` of the inner Frame class manually.

**`print(...)` works without import.** GDScript's built-in
`print(...)` is a global function — no `import` or `extends` is
needed. This is identical to Python; cross-target test harnesses
that use print-based assertions work cleanly on GDScript.

**`assert(...)` is a built-in.** GDScript's `assert(cond, msg)`
fires only in debug builds; in release builds it's a no-op. For
matrix tests (which run in debug mode by default), this is fine.
For shipping game code, use explicit `if not cond: panic(...)`
or signal-based error handling.

**Float vs int matters more than on Python.** GDScript's `int /
int` returns `int` (truncation), not `float` like Python 3. If
you need float division, use `float(a) / b` or `a / float(b)`.
Frame doesn't auto-coerce — `: int` stays `int`, `: float` stays
`float`.

**`null` is the `null` literal, not `None`.** GDScript uses `null`
for the absent/uninitialized value, not Python's `None`. Frame
source for cross-target portability should use `null` and let
the dynamic targets accept it (Python accepts `null` only via
specific conventions; the matrix test runner normalizes this).

**Headless mode requires `quit()`.** A `SceneTree`-extending
script that doesn't call `quit()` will hold the engine open until
the watchdog kills it. Always call `quit()` (or `quit(exit_code)`
for explicit exit codes) at the end of your test driver.

---

## Persist contract — `@@[save]` / `@@[load]`

A persisted system declares three system-level attributes:
`@@[persist(<blob_type>)]`, `@@[save(<save_method_name>)]`, and
`@@[load(<load_method_name>)]`. Framec generates the save/load pair
on the system class — save returns the blob, load is an instance
method that mutates self.

```frame
@@[persist(String)]
@@[save(save_state)]
@@[load(restore_state)]
@@system Counter {
    interface:  bump()
    machine:    $Active { bump() { self.n = self.n + 1 } }
    domain:     n: int = 0
}
```

Load is an instance method (allocate, then populate):

```gdscript
var c2 = Counter.new()
c2.restore_state(data)
```

**Wire format — Godot binary Variant (`var_to_bytes` / `bytes_to_var`).**
Where most backends emit JSON inside their persist blob, **GDScript
emits Godot's native binary Variant format** via `var_to_bytes` /
`bytes_to_var`. The motivation is fidelity: Godot's
`JSON.parse_string` returns every JSON number as `float`, erasing
the `int` vs `float` distinction Variant carries. A persisted
`int`-typed domain field or list element comes back as `float`, and
`Array.has(typed_int)` after restore returns `false` even when the
value is present (the list holds floats, `has` does type-equal
match). `var_to_bytes` round-trips Variants exactly — int stays
int, float stays float, nested arrays / dictionaries / strings /
booleans / null all preserve type. The wire-format **shape** still
matches every other backend (a `PackedByteArray`); the **encoding**
inside is Godot binary, not JSON. Mirrors Erlang's ETF and Lua's
serpent fidelity-exception rationale.

```gdscript
# Codegen sketch:
func save_op() -> PackedByteArray:
    if not self._context_stack.is_empty():
        push_error("E700: system not quiescent")
        return PackedByteArray()
    var state_data = {}                         # @@[no_persist] fields omitted
    state_data["_compartment"] = _ser_chain.call(self.__compartment)
    state_data["_state_stack"] = stack_arr
    state_data["n"] = self.n
    return var_to_bytes(state_data)

func restore_state(data: PackedByteArray) -> void:
    var state_data = bytes_to_var(data)
    self.n = state_data.get("n", null)
    # … rest of restore …
```

**Cross-language consumers**: Godot binary Variant format is not
directly readable by other languages. A consumer that wants to
inspect a GDScript persist blob from outside Godot either has to
run the Godot CLI to deserialize, or wait for the post-4.2 opt-in
cross-language wire format (MessagePack / tagged JSON via
`@@[persist_format(...)]`). Not as universal as JSON, but the
trade-off is the same as Erlang's ETF and Lua's serpent: native
fidelity wins over cross-language readability when the language
has types JSON can't represent.

**@@[no_persist] interaction.** Fields marked `@@[no_persist]` are
omitted from the `state_data` dictionary on save. On load, the
freshly-constructed Counter's domain initializers (set in `_init`)
populate them. Matches RFC-0016.1's "leave at `domain:` default"
contract.

**Why the JSON migration was reverted (2026-05-13).** An earlier
attempt to migrate GDScript persist to JSON (matching the other
JSON-shaped backends) was reverted after a real fidelity bug: a
user reported that `Array.has(persisted_int)` returned false after
restore. Investigation confirmed `JSON.parse_string`'s float-only
return policy and prompted the fidelity-exception decision
documented here. See `_scratch/bug_json_int_float*` for the
original repro.

Bare `@@[persist]` (no save/load names) is rejected with **E814**.
The legacy operation-attribute form (`operations: @@[save] foo()`)
is rejected with **E819** at framec 4.1.0+; the codemod at
`scripts/migrate_rfc0015.py` rewrites old fixtures mechanically.

## Persist quiescent contract — E700

`save_state()` requires the system to be quiescent (no event in
flight, `self._context_stack` empty). GDScript has no exceptions,
so calling it from inside a handler calls `push_error("E700:
system not quiescent")` (visible in the Godot output panel) and
returns an empty `PackedByteArray`. Callers must check
`if snap.size() == 0:` to detect the violation. Recovery isn't
possible — the handler's context frame is corrupted; discard the
instance and restore from a prior snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; GDScript shows ✅ on every row.
- `tests/common/positive/primary/26_state_params.fgd` — canonical
  state-parameter test, headless harness shape.
- `tests/common/positive/primary/13_system_return.fgd` —
  return-value contract test.
- `framec/src/frame_c/compiler/frame_validator.rs` —
  `gdscript_reserved_method_rename` carries the curated list of
  E501-flagged names with suggested renames.
- `framec-test-env/docker/runners/gdscript_runner.sh` — the
  headless Godot batch runner used by the test matrix.
- `framec-test-env/fuzz/cases_negative/e501_gdscript_reserved_method.fpy`
  — negative fixture exercising the E501 path.
