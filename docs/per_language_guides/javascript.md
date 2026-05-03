# Per-Language Guide: JavaScript

JavaScript is the dynamic-typing baseline among Frame's typed-
JS-family targets. The Frame JavaScript backend emits ES2017+
(`class`, `async`/`await`, template literals) without TypeScript's
type layer. Frame source for JavaScript looks similar to
TypeScript but without the `: type` annotations on the generated
output.

This guide documents the JavaScript-specific patterns.

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. JavaScript
is fully spec-conformant on every row.

---

## Foundation: untyped class with member methods

A Frame system targeting JavaScript generates a single `.js` file
containing:

- A `class WithInterface { ... }` (untyped).
- A `constructor()` that fires the start-state's `$>` cascade.
- One `greet(name)` method per interface entry.
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers.

```javascript
class WithInterface {
    constructor() {
        this.call_count = 0;
        // start-state $> cascade fires here
    }

    greet(name) {
        // ... handler body
        return result;
    }
}
```

Frame's `self.field` lowers to `this.field`.

---

## Domain fields: untyped properties initialized in constructor

Domain fields lower to property assignments in `constructor`:

```frame
domain:
    call_count: number = 0
    name: string = "alice"
```

```javascript
constructor() {
    this.call_count = 0;
    this.name = "alice";
}
```

Frame's `: type` annotation is documentation only — JavaScript is
dynamically typed.

---

## Strings: `+` for concat, template literals for interpolation

```frame
$Ready {
    greet(name: string): string {
        this.call_count += 1
        @@:(`Hello, ${name}!`)
        return
    }
}
```

```javascript
greet(name) {
    this.call_count += 1;
    return `Hello, ${name}!`;
}
```

---

## Async: `async` / `await` with Promise

Frame's `async` interface methods on JavaScript lower to
`async`-marked methods returning Promises:

```frame
async fetch(key: string): string {
    @@:return = await self.cache.get(key)
}
```

```javascript
async fetch(key) {
    const __result = await this.cache.get(key);
    return __result;
}
```

The matrix harness drives async tests via `(async () => { ... })()`
or by exporting an async `main()` that the runner awaits.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to a property
instantiated via `new Counter()`:

```javascript
class Embedding {
    constructor() {
        this.counter = new Counter();
    }

    notify() {
        this.counter.bump(1);
    }
}
```

---

## Loop idioms — both work

Standard JavaScript loop constructs. Idiom 1 compiles to native
passthrough.

---

## Multi-system per file: works as you'd expect

Multiple `@@system` blocks compile to multiple class declarations.

---

## Comments and the Oceans Model

`//` (line) and `/* ... */` (block) and `/** ... */` (JSDoc).

---

## Idiomatic patterns and common gotchas

**`new` for class instantiation.** `new Counter()`. Frame's
`@@Counter()` lowers to `new Counter()`.

**`this.field` everywhere.** Inside methods.

**`null` vs `undefined`.** Both exist. Frame's nullable values
default to `null`.

**No type checking at runtime.** Wrong types fail at first use,
not on declaration. Frame's `: type` annotations are
documentation.

**Module systems vary.** ESM (`import`/`export`) for modern Node
and browsers; CommonJS (`require`/`module.exports`) for older
Node. The matrix harness uses CommonJS by default.

**`console.log(...)` for output.**

---

## Persist contract — `@@[save]` / `@@[load]`

A `@@[persist]` system must declare two operations under the
`operations:` section: one tagged `@@[save]` (returns the
serialized blob) and one tagged `@@[load]` (instance method
that mutates self from a blob). The op names are yours to
pick — these match the JavaScript convention.

```frame
@@[persist]
@@system Counter {
    operations:
        @@[save]
        saveState(): string {}

        @@[load]
        restoreState(data: string) {}

    interface:  bump()
    machine:    $Active { bump() { self.n = self.n + 1 } }
    domain:     n: int = 0
}
```

Load is an instance method (allocate, then populate):

```javascript
const c2 = new Counter();
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
    @@[save]    saveState(): string {}
    @@[load]    restoreState(data: string) {}

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
flight, `this._context_stack` empty). Calling it from inside a
handler throws `Error("E700: system not quiescent")`. Catchable
via `try/catch`, but recovery isn't possible — the handler's
context frame is corrupted; discard the instance and restore from
a prior snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — JavaScript shows ✅ on
  every row.
- `tests/common/positive/primary/02_interface.fjs` — canonical
  interface-method shape.
- `framec/src/frame_c/compiler/codegen/backends/javascript.rs` —
  JavaScript backend codegen.
