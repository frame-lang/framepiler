# Frame QuickStart

*A dense syntax reference. For a tutorial walkthrough see [Getting Started](frame_getting_started.md). For full semantics see the [Language Reference](frame_language.md). For runnable examples see the [Cookbook](frame_cookbook.md).*

---

## Where each doc fits

| Doc | Use when… |
|---|---|
| **frame_quickstart.md** (this) | you need to look up syntax at a glance |
| [frame_getting_started.md](frame_getting_started.md) | learning Frame for the first time |
| [frame_language.md](frame_language.md) | resolving a semantics question, reading error codes, checking per-target behavior |
| [frame_cookbook.md](frame_cookbook.md) | "is there a recipe that does X" |
| [frame_runtime.md](frame_runtime.md) | debugging generated code, understanding the dispatch model |

---

## File skeleton

```frame
<prolog native code>                  # optional
@@target python_3                     # required, exactly once
@@codegen { frame_event: on }         # optional
@@[persist]                             # optional — see Persistence
@@system Name (params)? : Base?, Base? {
    operations: ...                   # all sections optional, but in this order:
    interface:  ...                   # operations → interface → machine → actions → domain
    machine:    ...                   # (E113 if out of order)
    actions:    ...
    domain:     ...
}
<epilog native code>                  # optional
```

---

## Section order (mandatory)

```
operations  →  interface  →  machine  →  actions  →  domain
```

| Section | Public? | State-aware? | Purpose |
|---|---|---|---|
| `operations` | yes | no (bypasses state machine) | utility methods, test hooks, static functions |
| `interface` | yes | yes (routes through dispatch) | the system's events |
| `machine` | — | yes | states, state vars, handlers |
| `actions` | no (private) | no (no Frame statements allowed) | private helpers called from handlers |
| `domain` | — | — | instance fields |

---

## States

```frame
$StateName {                          # first state = start state
    $.var1: T = init                  # state var (initializer REQUIRED)
    $.var2: T = init

    $>() { ... }                      # enter handler
    $>(arg: T) { ... }                # enter with enter_args
    <$() { ... }                      # exit handler
    <$(arg: T) { ... }                # exit with exit_args

    eventName(args): ret { ... }      # event handler
}

$Child => $Parent {                   # HSM: Child inherits from Parent
    ...
    => $^                             # trailing default forward: unhandled → parent
}
```

- State variables **reset** on normal transition `-> $State`, **preserved** on `-> pop$`.
- Access state vars with `$.name`; access domain vars with `self.name` / `this.name` / per-target.

---

## Interface declarations

```frame
interface:
    event()                           # no params, no return
    event(a: T, b: U)                 # typed params (types are opaque strings)
    event(): ret                      # return type, no default
    event(): ret = "default"          # return type with default value
    async event()                     # async variant
```

---

## Frame statements (the 7 constructs)

| Form | Effect |
|---|---|
| `-> $State` | transition |
| `-> $State(args)` | transition with state args |
| `-> (enter) $State` | transition with enter args |
| `(exit) -> $State` | transition with exit args |
| `(exit) -> (enter) $State(state)` | full-form transition |
| `-> "label" $State` | labeled transition (for diagrams) |
| `-> (enter) "label" $State` | label + enter args |
| `-> => $State` | transition with event forwarding (re-dispatch current event) |
| `-> pop$` | transition to popped state |
| `-> (enter) pop$` | pop with fresh enter args (decorated pop) |
| `(exit) -> pop$` | pop with exit args |
| `-> => pop$` | pop with event forwarding |
| `=> $^` | forward current event to parent state |
| `push$` | save current compartment onto state stack |
| `pop$` | pop (rarely used bare — usually `-> pop$`) |
| `$.varName = expr` | write state variable |
| `$.varName` | read state variable |

**Every transition generates an implicit `return` — code after `->` is unreachable (E400).**

---

## `@@` context accessors

All use colon-then-namespace, dot for fields:

| Syntax | Meaning |
|---|---|
| `@@:(expr)` | set return value (concise) |
| `@@:return = expr` | set return value (long form) |
| `@@:return(expr)` | set return value AND exit handler |
| `@@:return` | read current return value |
| `@@:params.x` | interface parameter `x` |
| `@@:event` | current interface method name |
| `@@:data.key` | call-scoped data (per-dispatch) |
| `@@:self.method(args)` | reentrant self-call (method must be in `interface:` — E601) |
| `@@:system.state` | current state name (read-only string, no `$`) |

**`return expr` is native — does NOT set the return value** (W415). Use `@@:(expr)` / `@@:return = expr` / `@@:return(expr)`.

---

## HSM (Hierarchical State Machines)

```frame
$Parent {                             # can exist without ever being a direct transition target
    shared_event() { ... }
}

$Child => $Parent {                   # Child inherits Parent's handlers via => $^
    specific_event() { ... }
    specific_event_with_forward() {
        log("Child processing")
        => $^                         # forward to parent after local work
    }
    => $^                             # trailing: unhandled events go to parent
}
```

**V4 semantics: unhandled events are IGNORED unless `=> $^` explicitly forwards.** No automatic inheritance.

---

## State stack (pushdown)

```frame
push$                                 # save current compartment (by reference!)
push$ -> $State                       # usual: save + transition to new compartment
-> pop$                               # pop and transition to saved state
(exit_args) -> (enter_args) => pop$   # decorated pop: exit args + fresh enter args + forward event
```

| Transition type | State variables |
|---|---|
| `-> $State` | **reset** to initial values |
| `-> pop$` | **preserved** from the popped compartment |

---

## System parameters (3 groups, in this order in the header)

```frame
@@system Name ( $(state_args), $>(enter_args), domain_args ) { ... }
```

| Sigil | Lands in | Matched by start state |
|---|---|---|
| `$(name: T)` | `compartment.state_args` (positional) | `$Start(name: T) { ... }` |
| `$>(name: T)` | `compartment.enter_args` (positional) | `$Start { $>(name: T) { ... } }` |
| bare `name: T` | constructor arg; used in domain init | `domain: name: T = name` |

**Each param body:** `name`, `name: T`, or `name: T = default`.

---

## Instantiation (`@@SystemName`)

```frame
@@Name()                              # no params
@@Name(42)                            # one domain param
@@Name("foo", 3)                      # multiple positional domain params
@@Name(name="foo", size=3)            # named domain params
@@Name($(7))                          # state arg
@@Name($>("ready"))                   # enter arg
@@Name($(7), $>("ready"), "alice")    # all three: state, enter, domain
@@Name($(x=7), $>(msg="hi"), n="a")   # all three, named form
```

Within one call: **don't mix positional and named**. Defaults are substituted at the call site.

> **Known framec bug**: `$(...)` at call sites *inside handler bodies* is not expanded (see `_scratch/bug_state_arg_call_site_in_handler.md`). Use a bare domain param as a workaround until fixed.

---

## Persistence

```frame
@@[persist]                             # persist everything (all domain vars)
@@[persist(domain=[a, b])]              # whitelist specific domain fields
@@[persist(exclude=[c])]                # blacklist specific domain fields
```

Generated methods:

| Method | Kind | Returns |
|---|---|---|
| `save_state()` | instance | JSON string (Python: pickle bytes — see warning below) |
| `SystemName.restore_state(data)` | **static** | new restored instance |

**Restore does NOT invoke `$>`**. The state is being restored, not entered.

**Quiescent contract — E700.** Calling `save_state()` from inside
a handler is a contract violation. The runtime errors with
`E700: system not quiescent` (per-backend mechanism: throw, panic,
abort, or push_error). Only call `save_state` between events.

**Nested `@@SystemName` fields persist recursively.** All 17
backends; each child's state embeds in the parent's blob.

**Python uses pickle.** Untrusted-source blobs run arbitrary code
on `restore_state`. Don't unpickle data you didn't write yourself
without validation. JSON migration for Python is in RFC-0012,
deferred pending customer feedback.

---

## Async

- Prefix `async` on any interface method / action / operation.
- If any interface method is `async`, the **whole dispatch chain** becomes async.
- **Two-phase init**: `s = @@System()` then `await s.init()` (Swift: `initAsync()`).

| Target | Async | Notes |
|---|---|---|
| python_3, typescript, javascript, rust, dart, gdscript | yes | standard `async/await` (gdscript: bare `await`) |
| kotlin | yes | `suspend fun`, no `await` keyword on suspend→suspend calls |
| swift | yes | `initAsync()` is the async entry point (not `init`) |
| csharp | yes | `async Task<T>` |
| java | yes | `CompletableFuture<T>` on public interface only |
| cpp_23 | yes | `FrameTask<T>` coroutine; needs `-std=c++23` |
| c, go, php, ruby, lua, erlang | **no** | `async` is a framec error |

---

## Types

**Frame has no type system.** Types and initializer expressions are opaque strings passed through verbatim to the target language.

- Write native type names: `int`, `str`, `Vec<i32>`, `std::string`, etc.
- Portable init literals: `""`, `0`, `false`, `[]`, `{}`
- Non-portable init (target-specific constructors) must be written as native code, not through Frame's normalizer.
- Dynamic targets (Python, JS, Ruby, Lua, Erlang, PHP): type is optional.
- Static targets that zero-init (C, C++, Go): init is optional (E605 requires a type though).

---

## Target languages

```
python_3   typescript   javascript   rust
c          cpp_23       java         csharp
go         php          kotlin       swift
ruby       erlang       lua          dart
gdscript   graphviz
```

Set via `@@target <id>` (required) or CLI `-l <lang>`. `graphviz` emits DOT source for state-diagram rendering; pipe through `dot -Tsvg`.

---

## Visibility

| Element | Default | Override |
|---|---|---|
| `@@system Foo` | **public** (`public class`, `export class`, `pub struct`, etc.) | `@@system private Foo` |
| interface methods | public | — (always public) |
| operations | public | — (always public) |
| actions, handlers | private | — (always private) |

`@@system public Foo` is an error (redundant). `private` is an error on targets without class-level visibility (Python, Ruby, Lua, C, GDScript, Erlang).

---

## `const` domain fields

```frame
domain:
    const max_retries: int = 3        # immutable after construction
    const threshold: int = threshold  # initialized from system param
    counter: int = 0                  # mutable
```

Assignment to a `const` field in a handler body is E615. Per-target rendering: `final` (Java/Dart/Kotlin), `readonly` (C#/TS), `const` (C++), `let` (Swift); comment-only marker where the target doesn't enforce immutability.

---

## Common idioms

| Idiom | Shape |
|---|---|
| **Kernel-loop chain** (recipe 31, 110) | `$>` reads data, decides, queues next `->`; whole chain runs inside one interface call |
| **Self-replay** (recipe 53) | `-> $Start` then `@@:self.feed(ch)` — re-dispatches the byte to the new state |
| **Retry via re-entry** (recipe 24) | `-> $SameState` re-runs `$>` with reset state vars |
| **Safety overlay** (recipe 49) | HSM parent holds `e_stop`, `fault` handlers; all operational children inherit |
| **Interlock** (recipe 60) | Omit the handler in the state where the capability shouldn't exist — silent no-op |
| **Transient decision state** (recipes 5, 31, 110) | State with only a `$>` handler that branches on data captured at entry |
| **Parent-callback** (recipes 28, 48, 109) | Child system calls `self.parent.method(...)` to report results |
| **Oracle specialist** (recipe 110, [Parsers essay](articles/research/Parsers_as_Composed_State_Machines.md)) | Coordinator calls specialist's interface method; specialist runs its own FSM and returns a verdict |
| **State-as-gate** (recipes 60, 81, 83, 105) | Reach-by-construction: no handler means the capability literally doesn't exist in that state |

---

## Error codes (the ones you'll actually hit)

| Code | Meaning | Common cause |
|---|---|---|
| **E113** | section order | `interface:` declared before `operations:` |
| **E116** | duplicate state name | two `$Name { ... }` blocks |
| **E400** | unreachable code | code after `->` |
| **E401** | Frame statement in action / operation | `-> $State` inside `actions:` body |
| **E402** | transition to undefined state | typo in `-> $Nmae` |
| **E403** | `=> $^` in state without parent | forward from a non-HSM state |
| **E405** | parameter arity mismatch | transition to `$S(arg: T)` without passing arg |
| **E410** | duplicate state variable | two `$.x:` in one state |
| **E413** | HSM cycle | `$A => $B` and `$B => $A` |
| **E601** | `@@:self.X()` method not in interface | `X` is in `actions:` or `operations:` |
| **E602** | `@@:self.X()` arg count mismatch | interface has 2 params, call passes 3 |
| **E603** | bare `@@:self` | must be `@@:self.method(args)` |
| **E604** | bare `@@:system` | must be `@@:system.state` (or other member) |
| **E605** | static target, no type on domain field | add `: T` in Rust/C/C++/Go |
| **E613** | domain field shadows system param | pick different names |
| **E615** | assigning to `const` field | remove `const` or drop the assignment |
| **W414** | unreachable state | state with no incoming transitions |
| **W415** | handler return value lost | you wrote `return expr`; use `@@:(expr)` |
| **W601** | self-call return not captured | `@@:self.X()` has a return type but result is discarded |

---

## CLI quick reference

```bash
framec source.fpy                     # compile to target declared via @@target
framec source.fpy -l rust             # override target
framec source.fpy -l graphviz | dot -Tsvg -o diagram.svg
framec source.fpy -o out.py           # write output to file

cargo test                            # run framec's 370 unit tests
cargo clippy -- -D warnings           # lint
cargo fmt --check                     # format check
```

---

## 60-second starter

```frame
@@target python_3

@@system Turnstile {
    interface:
        coin()
        push(): str = "blocked"

    machine:
        $Locked {
            coin() { -> $Unlocked }
            push(): str { @@:("locked — insert coin") }
        }
        $Unlocked {
            coin() { }
            push(): str {
                @@:("welcome")
                -> $Locked
            }
        }
}

if __name__ == '__main__':
    t = @@Turnstile()
    print(t.push())     # "locked — insert coin"
    t.coin()
    print(t.push())     # "welcome"
```

`framec turnstile.fpy > turnstile.py && python3 turnstile.py`
