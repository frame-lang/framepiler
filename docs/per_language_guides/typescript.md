# Per-Language Guide: TypeScript

TypeScript is JavaScript with a static type layer. The Frame
TypeScript backend emits typed `.ts` source that compiles to
`.js` via `tsc` (or runs directly under `ts-node`). The
distinguishing feature for Frame source is that TypeScript's
type annotations are first-class — Frame's `: type` annotations
flow through to the generated TS as real type declarations and
are checked at compile time.

This guide documents the TypeScript-specific patterns. It assumes
you are already familiar with Frame's core syntax and TypeScript
basics (`class`, `interface`, generics, `async`/`await`).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. TypeScript
is fully spec-conformant on every row.

---

## Foundation: typed class with member methods

A Frame system targeting TypeScript generates a single `.ts` file
containing:

- A `class WithInterface { ... }` with typed member fields and
  methods.
- A `constructor()` that fires the start-state's `$>` cascade.
- One `greet(name: string): string` method per interface entry.
- Internal `_state_<S>(...)` and `_s_<S>_hdl_<kind>_<event>(...)`
  helpers.

```typescript
class WithInterface {
    call_count: number = 0;
    // ... runtime fields

    constructor() {
        // start-state $> cascade fires here
    }

    greet(name: string): string {
        // ... handler body
        return result;
    }
}
```

Frame's `self.field` lowers to `this.field`. Method calls use
`s.greet("World")`.

---

## Domain fields: typed properties

Domain fields lower to typed properties:

```frame
domain:
    call_count: number = 0
    name: string = "alice"
    items: number[] = []
```

```typescript
call_count: number = 0;
name: string = "alice";
items: number[] = [];
```

The Frame `: type` annotation IS the TypeScript type — write
`: string`, `: number[]`, `: Map<string, number>`, etc.

**Frame type names map cleanly to TypeScript types:**

| Frame              | TypeScript          | Notes |
|--------------------|---------------------|-------|
| `number`           | `number`            | float64 |
| `string`           | `string`            | |
| `boolean`          | `boolean`           | |
| `T[]`              | `T[]` or `Array<T>` | |
| `Record<K,V>`      | `Record<K,V>`       | |
| `T \| null`        | `T \| null`         | union for nullable |

---

## Strings: `+` for concat, template literals for interpolation

TypeScript uses JavaScript's string semantics:

- `+` overloads for concatenation.
- Backtick template literals support interpolation: `` `${expr}` ``.

```frame
$Ready {
    greet(name: string): string {
        this.call_count += 1
        @@:(`Hello, ${name}!`)
        return
    }
}
```

```typescript
greet(name: string): string {
    this.call_count += 1;
    return `Hello, ${name}!`;
}
```

---

## Async: `async` / `await` with typed Promise

Frame's `async` interface methods on TypeScript lower to
`async`-marked methods returning `Promise<T>`:

```frame
async fetch(key: string): string {
    @@:return = await self.cache.get(key)
}
```

```typescript
async fetch(key: string): Promise<string> {
    const __result = await this.cache.get(key);
    return __result;
}
```

Promises are first-class in TS; `Promise<T>` is the standard
async return shape.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to a typed property:

```typescript
class Embedding {
    counter: Counter = new Counter();

    notify() {
        this.counter.bump(1);
    }
}
```

Calls to `self.counter.bump(n)` lower to `this.counter.bump(n)`.

---

## Loop idioms — both work

Standard JavaScript/TypeScript loop constructs (`while`, `for`,
`for-of`, `for-in`). Idiom 1 compiles to native passthrough.

---

## Multi-system per file: works as you'd expect

Multiple `@@system` blocks compile to multiple class declarations
in the same `.ts` file.

---

## Comments and the Oceans Model

`//` (line) and `/* ... */` (block) and `/** ... */` (TSDoc).

```frame
@@[target("typescript")]

// Module-prolog block — passes through as TypeScript source.

@@system Counter {
    machine:
        // Section-level comments are preserved as // blocks.
        $Counting {
            tick() { ... }
        }
}
```

---

## Idiomatic patterns and common gotchas

**`new` for class instantiation.** `new Counter()`, not just
`Counter()`. Frame's `@@Counter()` lowers to `new Counter()`.

**`this.field` everywhere.** TypeScript requires `this.` for
member access inside methods (otherwise it's a free variable).

**`null` vs `undefined`.** TypeScript distinguishes both.
Frame's nullable types should declare `| null` or `| undefined`
explicitly.

**Strict mode (`tsconfig.json`).** `strict: true` enables full
type checking. Frame-generated TS works under strict mode; if
you have native passthrough that violates strict checks, relax
specific options or fix the source.

**Type-only imports** (`import type { ... }`) and
**runtime imports** (`import { ... }`) — distinct in TS.

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

- `docs/runtime-capability-matrix.md` — TypeScript shows ✅ on
  every row.
- `tests/common/positive/primary/02_interface.fts` — canonical
  interface-method shape with template literals.
- `framec/src/frame_c/compiler/codegen/backends/typescript.rs` —
  TypeScript backend codegen.
