# Per-Language Guide: PHP

PHP's design quirks — `$variable` prefix on every variable, `$this`
for instance reference, `.` for string concatenation (not `+`), no
type system before PHP 7 — make it stylistically distinct from the
C-family default. Frame source for PHP looks similar at the
state-machine level but uses PHP-native syntax inside handler
bodies.

This guide documents the PHP-specific patterns. It assumes you are
already familiar with Frame's core syntax and PHP basics
(`<?php`, `$variables`, `$this`, classes, namespaces).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. PHP is
fully spec-conformant; `async` is the only language-natural skip
(no native async/await).

---

## Foundation: class with `$this`-prefixed members

A Frame system targeting PHP generates a single `.php` file
containing:

- A `<?php` header (declared in the prolog).
- A `class WithInterface { ... }` with member fields and methods.
- A `function __construct() { ... }` constructor that fires the
  start-state's `$>` cascade.
- One `function greet(string $name): string` method per
  interface entry.

```php
<?php

class WithInterface {
    public $call_count = 0;
    // ... runtime fields

    function __construct() {
        // start-state $> cascade fires here
    }

    function greet(string $name): string {
        // ... handler body
        return $result;
    }
}
```

Frame's `self.field` lowers to `$this->field` (PHP's instance
reference is `$this` and member access uses `->`).

---

## Domain fields: typed members with default initializers

Domain fields lower to public members:

```frame
domain:
    call_count: int = 0
    name: str = "alice"
```

```php
public $call_count = 0;
public $name = "alice";
```

PHP 7.4+ supports typed properties. Frame's `: int` lowers to
`int $call_count = 0;` for typed-property targets. For PHP 7.0–7.3,
type annotations on properties don't compile — declare without
the type or upgrade the target.

---

## Strings: `.` for concat, `"$var"` interpolation

PHP uses `.` (period) for string concatenation, not `+`.
Double-quoted strings interpolate variables directly (`"$name"`)
or with braces for complex expressions (`"{$obj->field}"`):

```frame
$Ready {
    greet(name: str): str {
        $this->call_count += 1
        @@:("Hello, " . $name . "!")
        return
    }
}
```

```php
function greet(string $name): string {
    $this->call_count += 1;
    return "Hello, " . $name . "!";
}
```

Note the `.` instead of `+` — this is PHP-specific. Frame doesn't
auto-translate `+` to `.`; you must write `.` in PHP source.

For interpolation:

```frame
$Active {
    log() {
        $this->message = "count={$this->count}, name={$this->name}"
    }
}
```

Curly braces are required when interpolating object members
(`{$obj->field}`) — the bare `"$obj->field"` form is
context-sensitive.

---

## No async — language-natural skip

PHP has no native async/await. The matrix capability table shows
PHP's async row as 🚫 (language-natural skip). Asynchronous work
in PHP typically uses:

- ReactPHP (event loop + promises) for async I/O.
- Swoole (extension-based async runtime) for concurrent
  request handling.
- Plain `pcntl_fork` for OS-level concurrency.

These are user concerns. The state machine itself is single-
process synchronous.

---

## Cross-system fields: direct instantiation

`var counter: Counter = @@Counter()` lowers to a member field
instantiated via `new Counter()` in the constructor. The
phase7_multisys round landed the PHP tagged-init constructor
body fix:

```php
class Embedding {
    public $counter;

    function __construct() {
        $this->counter = new Counter();
        // start-state $> fires
    }

    function notify() {
        $this->counter->bump(1);
    }
}
```

Calls to `self.counter.bump(n)` lower to `$this->counter->bump(n)`
— note the `->` chain.

---

## Loop idioms — both work

PHP has `while`, `do-while`, `for`, and `foreach`. Frame's idiom
1 (`while cond { ... }`) compiles to a native PHP `while` block
via passthrough.

---

## Multi-system per file: works as you'd expect

A `.fphp` source containing multiple `@@system` blocks compiles
to a single `.php` file with multiple class definitions. PHP has
no per-file class limit.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to PHP the same way it applies
to every other backend. The comment leaders are `//` (line),
`#` (also line, PHP-specific), and `/* ... */` (block).

```frame
@@[target("php")]
<?php

// Module-prolog block — passes through as PHP source.

@@system Counter {
    machine:
        // Section-level comments are preserved as native // blocks.
        $Counting {
            tick() { ... }
        }
}
```

---

## Idiomatic patterns and common gotchas

**`$this->field`, not `self.field` or `$this.field`.** PHP uses
`->` for object member access (not `.`). Frame's `self.x` lowers
to `$this->x`. Frame state-vars `$.x` lower to compartment
access — internally framec handles this.

**`new` for class instantiation.** Frame's `@@WithInterface()`
lowers to `new WithInterface()`.

**Variable prefix `$`.** Every PHP variable starts with `$`.
Frame doesn't auto-prefix — handler-local declarations should
write `$x = 0` explicitly.

**`null` is the absent-value marker.** Same as Python's `None`,
JS's `undefined` (but distinct).

**`strict_types=1` for type enforcement.** Adding
`declare(strict_types=1);` to the prolog enforces the
type-hints; without it, PHP coerces silently. Frame doesn't
auto-emit this declaration.

**`echo` and `print` for output, not `console.log`.** PHP's
`echo "Hello\n";` is the standard print idiom. The Frame
matrix harness uses `echo` for assertion output.

---

## Persist contract — `@@[save]` / `@@[load]`

A `@@[persist]` system must declare two operations under the
`operations:` section: one tagged `@@[save]` (returns the
serialized blob) and one tagged `@@[load]` (instance method
that mutates self from a blob). The op names are yours to
pick — these match the PHP convention.

```frame
@@[persist]
@@system Counter {
    operations:
        @@[save]
        save_state(): string {}

        @@[load]
        restore_state(data: string) {}

    interface:  bump()
    machine:    $Active { bump() { self.n = self.n + 1 } }
    domain:     n: int = 0
}
```

Load is an instance method (allocate, then populate):

```php
$c2 = new Counter();
$c2->restore_state($data);
```

The bare `@@[persist]` form (no `@@[save]` / `@@[load]` ops) is
rejected with **E814** since framepiler `b3aebc5` (2026-05-03).

### Post-load hook: `@@[on_load]`

A third optional attribute fires user code after
`restore_state` finishes populating self — useful for re-establishing
derived state, firing watchers, validating invariants:

```frame
operations:
    @@[save]    save_state(): string {}
    @@[load]    restore_state(data: string) {}

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
throws `\Exception("E700: system not quiescent")`. Catchable via
`try/catch`, but recovery isn't possible — the handler's context
frame is corrupted; discard the instance and restore from a prior
snapshot. See
[`docs/frame_runtime.md`](../frame_runtime.md) and
[`rfc-0012`](../rfcs/rfc-0012.md) for the full contract.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; PHP shows ✅ on every row except `async` (🚫 —
  language-natural skip).
- `tests/common/positive/primary/02_interface.fphp` — canonical
  interface-method shape with `.`-string concat.
- `framec/src/frame_c/compiler/codegen/backends/php.rs` — PHP
  backend codegen.
- `memory/phase7_multisys_2026_04_27.md` — context on the PHP
  tagged-init constructor body fix.
