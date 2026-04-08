# Frame Language Reference

Complete reference for the Frame language. For a tutorial introduction, see [Getting Started](frame_getting_started.md).

## Table of Contents

- [Source File Structure](#source-file-structure)
- [System Declaration](#system-declaration)
- [Interface Section](#interface-section)
- [Machine Section](#machine-section)
- [Actions Section](#actions-section)
- [Operations Section](#operations-section)
- [Domain Section](#domain-section)
- [Frame Statements](#frame-statements)
- [Hierarchical State Machines](#hierarchical-state-machines)
- [System Context](#system-context)
- [Compartment](#compartment)
- [Persistence](#persistence)
- [Async](#async)
- [System Instantiation](#system-instantiation)
- [Token Summary](#token-summary)
- [Error Codes](#error-codes)
- [Complete Example](#complete-example)

---

## Source File Structure

```
<preamble>          // native code (optional)
@@target <lang>     // required, exactly once
@@codegen { ... }   // optional, at most once
<annotations>*      // zero or more (@@persist, etc.)
@@system <Name> (<params>)? {
    <sections>
}
<postamble>         // native code (optional)
```

Everything outside `@@target`, `@@codegen`, annotations, and `@@system` is native code and passes through unchanged.

### `@@target`

```
@@target <language_id>
```

Required. Must appear before `@@system`. Specifies the target language.

| ID | Language | | ID | Language |
|----|----------|-|----|----------|
| `python_3` | Python 3 | | `go` | Go |
| `typescript` | TypeScript | | `php` | PHP |
| `javascript` | JavaScript | | `kotlin` | Kotlin |
| `rust` | Rust | | `swift` | Swift |
| `c` | C (C11) | | `ruby` | Ruby |
| `cpp_17` | C++17 | | `erlang` | Erlang |
| `java` | Java | | `lua` | Lua |
| `csharp` | C# | | `dart` | Dart |
| `graphviz` | GraphViz DOT | | `gdscript` | GDScript |

The `@@target` is the authoritative declaration of the file's target language. It can be overridden by a CLI flag (`-l <language>`).

### `@@codegen`

```
@@codegen {
    <key> : <value> ,
    ...
}
```

Optional. Must appear after `@@target` and before `@@system`.

| Key | Values | Default | Meaning |
|-----|--------|---------|---------|
| `frame_event` | `on` \| `off` | `off` | Generate FrameEvent/FrameContext classes |

The framepiler auto-enables `frame_event` when features that require it are used (enter/exit parameters, event forwarding, `@@:return`, interface return values).

### `@@persist`

```
@@persist
@@persist(domain=[field1, field2])
@@persist(exclude=[field3])
```

Generates `save_state()` and `restore_state()` methods. See [Persistence](#persistence).

---

## System Declaration

```
@@system <Name> ( <system_params> )? {
    ( operations: <operations_block> )?
    ( interface: <interface_block> )?
    ( machine: <machine_block> )?
    ( actions: <actions_block> )?
    ( domain: <domain_block> )?
}
```

Sections are optional but **must appear in the order shown**: operations → interface → machine → actions → domain.

### System Parameters

Three groups, all optional, in fixed positional order:

```
@@system Name ( $(<state_params>) , $>(<enter_params>) , <domain_params> )
```

| Group | Syntax | Target |
|-------|--------|--------|
| State params | `$(<param_list>)` | Start state's `compartment.state_args` |
| Enter params | `$>(<param_list>)` | Start state's `compartment.enter_args` |
| Domain params | bare `<param_list>` | Domain variable overrides |

Groups are positional. Omitting a group shifts later groups left.

---

## Interface Section

Declares the system's public API.

```
interface:
    <method_name> ( <params>? ) (: <return_type> (= <default_value>)? )?
```

**Examples:**

```frame
interface:
    start()
    stop()
    process(data: str, priority: int)
    getStatus(): str
    getDecision(): str = "yes"
```

**Rules:**
- Method names must be unique within the interface
- Parameters: `name: type` or untyped `name`
- Default return value is a native expression, used when no handler sets `@@:return`
- A return type with no default implies `None`/`null` as default

---

## Machine Section

Contains state definitions.

### State Declaration

```
$<StateName> ( => $<ParentState> )? {
    <state_var_declarations>*
    <handlers>*
    ( => $^ )?
}
```

- State names must be unique within the system
- The **first state listed** is the start state
- `=> $ParentState` declares an HSM parent (see [HSM](#hierarchical-state-machines))

### State Variables

Must appear at the top of the state block, before any handlers.

```
$.<varName> (: <type>)? = <initializer_expr>
```

| Part | Required | Description |
|------|----------|-------------|
| `$.` | Yes | State variable prefix |
| `<varName>` | Yes | Identifier |
| `: <type>` | No | Type annotation |
| `= <initializer_expr>` | Yes | Native expression; evaluated on every state entry |

**Scope rules:**
- `$.x` always refers to the enclosing state's variable `x`
- No syntax exists to access another state's variables
- No duplicates within a state
- State variable names may shadow domain variables (no ambiguity due to `$.` prefix)

### Event Handlers

```
<event_name> ( <params>? ) (: <return_type> (= <default_value>)? )? {
    <body>
}
```

When a handler declares a return type with a default value (`= <expr>`), that expression initializes `@@:return` before the handler body executes.

The body is a mix of native code and Frame statements. Native code passes through unchanged.

### Enter Handler

```
$> ( <params>? ) {
    <body>
}
```

Called when the state is entered via a transition. Parameters come from the transition's enter args.

### Exit Handler

```
<$ ( <params>? ) {
    <body>
}
```

Called when the state is exited via a transition. Parameters come from the transition's exit args.

### Enter/Exit Parameter Mapping

Enter and exit args are passed **positionally**:

```frame
$Idle {
    start() {
        -> ("from_idle", 42) $Active
    }
}

$Active {
    $>(source: str, value: int) {
        print(f"Entered from {source} with {value}")
    }
}
```

---

## Actions Section

Private helper methods on the system class.

```frame
actions:
    validate(data): bool {
        return data is not None
    }
```

**Can access:** domain variables, `@@:return`, `@@.param`, `@@:event`, `@@:data`

**Cannot access (E401):** `-> $State`, `=> $^`, `push$`, `pop$`, `$.varName`

Actions have no state context. `return` in actions is the native language return.

---

## Operations Section

Public methods that bypass the state machine entirely.

```frame
operations:
    static version(): str {
        return "1.0.0"
    }

    get_debug_info(): str {
        return f"state={self.__compartment.state}"
    }
```

- **Static operations** have no `self`/`this` access
- **Non-static operations** can access domain variables and `@@:return`
- Same Frame statement restrictions as actions (E401)
- `return` is the native language return

---

## Domain Section

Instance variables. The domain block is **strictly native code** — use the target language's variable declaration syntax.

```frame
# Python
domain:
    count: int = 0
    label: str = "default"

# C
domain:
    int count = 0
    char* label = "default"
```

Domain variables persist across state transitions. The framepiler extracts variable **names** from each line for constructor initialization, persistence, and system parameter overrides.

---

## Frame Statements

Frame recognizes exactly **6 constructs** within handler bodies. Everything else is native code.

### Transition — `-> $State`

```
( <exit_params> )? -> ( => )? ( <enter_params> )? <label>? $<TargetState> ( <state_params> )?
```

| Form | Meaning |
|------|---------|
| `-> $State` | Simple transition |
| `-> $State(args)` | With state args |
| `-> (args) $State` | With enter args |
| `(args) -> $State` | With exit args |
| `(exit) -> (enter) $State(state)` | Full form |
| `-> "label" $State` | With label (for diagrams) |
| `-> => $State` | With event forwarding |
| `-> pop$` | Transition to popped state |

**Event forwarding** (`-> =>`): The current event is stashed on the target compartment. After the enter handler fires, the forwarded event is dispatched to the target state.

**Transition to popped state** (`-> pop$`): Pops a compartment from the state stack. Full lifecycle fires. State variables are **preserved** (not reinitialized).

Every transition is implicitly followed by a `return` — code after a transition is unreachable.

### Forward to Parent — `=> $^`

```
=> $^
```

Forwards the current event to the parent state's dispatch function. The enclosing state must have a parent declared with `=> $ParentState`.

### Stack Push — `push$`

```
push$
```

Pushes the current compartment (including all state variables) onto the state stack.

### Stack Pop — `pop$`

```
pop$
```

Pops and discards the top compartment. To transition to the popped state, use `-> pop$`.

### State Variable Access — `$.varName`

```
$.counter               // read
$.counter = <expr>      // write
```

### System Context — `@@`

```
@@.param            // interface parameter (shorthand)
@@:return = <expr>  // set return value
@@:return           // read return value
@@:event            // interface method name
@@:data[key]        // call-scoped data
@@:params[x]        // interface parameter (explicit)
```

See [System Context](#system-context) for full semantics.

**`return` is always native.** It exits the current function — it does NOT set `@@:return`. In event handlers, `return expr` silently loses the value (W415 warning). Use `@@:(expr)` to set the return value, then bare `return` to exit.

| Syntax | Effect |
|--------|--------|
| `@@:(expr)` | Set return value (concise) |
| `@@:return = expr` | Set return value (explicit) |
| `return` | Exit the handler (native — valid everywhere) |
| `return expr` | Native return — in handlers, value is lost (W415) |
| `return @@:(expr)` | Error E408 — cannot combine |

---

## Hierarchical State Machines

### Parent Declaration

```frame
$Child => $Parent {
    ...
}
```

### Explicit Forwarding

**V4 uses explicit-only forwarding.** Unhandled events are **ignored**, not forwarded.

**In-handler forward:**

```frame
$Child => $Parent {
    event_a() {
        log("Child processing")
        => $^
    }
}
```

**State-level default forward** (forwards ALL unhandled events):

```frame
$Child => $Parent {
    specific_event() { ... }
    => $^
}
```

**Key semantics:**
- `=> $^` is the **only** way to forward to parent
- `=> $^` can appear **anywhere** in a handler
- Without `=> $^`, unhandled events are **ignored**

---

## System Context

The `@@` prefix provides access to the current interface call's context.

### Architecture

Every interface call creates:

- **FrameEvent** — `{ _message: string, _parameters: dict }`
- **FrameContext** — `{ event: FrameEvent, _return: any, _data: dict }`

The context is pushed onto `_context_stack` on call and popped on return. Lifecycle events (`$>`, `<$`) use the existing context.

### Syntax

| Syntax | Meaning |
|--------|---------|
| `@@.x` | Interface parameter `x` (shorthand) |
| `@@:params[x]` | Interface parameter `x` (explicit) |
| `@@:event` | Interface method name |
| `@@:return` | Get/set return value |
| `@@:data[key]` | Get/set call-scoped data |

`@@` ALWAYS refers to the interface call context, even inside lifecycle handlers.

### Context Lifecycle

```
calc.compute(1, 2) called
│
├─► FrameEvent("compute", {a: 1, b: 2}) created
├─► FrameContext(event, _return=None, _data={}) created
├─► Context PUSHED to _context_stack
│
├─► Kernel routes event to handler
│   ├─► Handler may set @@:return, @@:data
│   ├─► Handler triggers -> $Next
│   ├─► <$ handler runs (can access @@.a, @@:return)
│   ├─► Compartment switch
│   └─► $> handler runs (can access @@.a, @@:return)
│
├─► Context POPPED
└─► Return context._return to caller
```

### Reentrancy

Each interface call pushes its own context. Nested calls are isolated — inner `@@:return` does not affect outer `@@:return`.

### Context Not Available

`@@` is not available in static operations or the initial `$>` during construction.

---

## Compartment

The **compartment** is Frame's central runtime data structure — a closure for states that preserves state identity and all scoped data.

| Field | Purpose |
|-------|---------|
| `state` | Current state identifier |
| `state_args` | Arguments via `$State(args)` |
| `state_vars` | State variables (`$.varName`) |
| `enter_args` | Arguments via `-> (args) $State` |
| `exit_args` | Arguments via `(args) -> $State` |
| `forward_event` | Stashed event for `-> =>` forwarding |

### State Stack = Compartment Stack

`push$` saves the **entire compartment** (including state variables). `-> pop$` restores it.

| Transition | State Variable Behavior |
|------------|------------------------|
| `-> $State` (normal) | **Reset** to initial values |
| `-> pop$` (history) | **Preserved** from saved compartment |

---

## Persistence

`@@persist` generates save/restore methods.

| Language | Save | Restore |
|----------|------|---------|
| Python | `save_state()` → `bytes` | `restore_state(data)` [static] |
| TypeScript | `saveState()` → `any` | `restoreState(data)` [static] |
| Rust | `save_state(&mut self)` → `String` | `restore_state(json)` [static] |
| C | `save_state(self)` → `char*` | `restore_state(json)` [static] |

**Persisted:** current state, state stack, state/enter/exit args, state vars, forward event, domain variables.

**Reinitialized on restore:** `_context_stack` (empty), `__next_compartment` (null).

**Restore does NOT invoke the enter handler** — the state is being restored, not entered.

### Field Filtering

| Form | Behavior |
|------|---------|
| `@@persist` | All domain vars |
| `@@persist(domain=[a, b])` | Only `a`, `b` |
| `@@persist(exclude=[c])` | All except `c` |

---

## Async

Interface methods, actions, and operations can be declared `async`:

```frame
interface:
    async connect(url: str)
    async receive(): Message

actions:
    async fetch_data() {
        return await http.get("/data")
    }
```

If ANY interface method is `async`, the entire dispatch chain becomes async. For async systems, a two-phase init is required: `s = @@System()` (sync), then `await s.init()` (async).

| Language | Supported | Notes |
|----------|-----------|-------|
| Python | Yes | `async def` + `await` |
| TypeScript | Yes | `async` + `await`, `Promise<T>` returns |
| Rust | Yes | `async fn` + `.await`, boxed futures for recursion |
| C | No | Warning, `async` ignored |
| Go, Java 21+ | Not needed | Concurrency is transparent |

---

## System Instantiation

Use `@@SystemName()` in native code to instantiate a Frame system:

```frame
calc = @@Calculator()
proc = @@OrderProcessor("standard", {"source": "web"}, 5)
```

The framepiler expands this to the appropriate native constructor and validates that the system name exists and arguments match.

---

## Token Summary

### Module-Level

| Token | Meaning |
|-------|---------|
| `@@target` | Declare target language |
| `@@codegen` | Configure code generation |
| `@@persist` | Enable serialization |
| `@@system` | Declare state machine |

### State Machine

| Token | Meaning |
|-------|---------|
| `$<Name>` | State reference |
| `$>` | Enter handler |
| `<$` | Exit handler |
| `$^` | Parent state reference |
| `$.` | State variable prefix |

### Statements

| Token | Meaning |
|-------|---------|
| `->` | Transition |
| `-> "label"` | Labeled transition |
| `=>` | Forward |
| `-> =>` | Transition with forwarding |
| `-> pop$` | Transition to popped state |
| `push$` | Push to state stack |
| `pop$` | Pop from state stack |
| `return` | Native return (exits handler/action/operation) |

### Context

| Token | Meaning |
|-------|---------|
| `@@.x` | Parameter shorthand |
| `@@:return` | Return value |
| `@@:event` | Event name |
| `@@:params[x]` | Parameter explicit |
| `@@:data[key]` | Call-scoped data |

---

## Error Codes

### Parse Errors (E0xx)

| Code | Name | Description |
|------|------|-------------|
| E001 | `parse-error` | Malformed Frame syntax |
| E002 | `unexpected-token` | Unexpected token in Frame construct |
| E003 | `unclosed-block` | Missing closing brace or delimiter |

### Structural Errors (E1xx)

| Code | Name | Description |
|------|------|-------------|
| E105 | `missing-target` | `@@target` directive missing or invalid |
| E111 | `duplicate-system-param` | Duplicate parameter in system declaration |
| E113 | `section-order` | System sections out of order |
| E114 | `duplicate-section` | Section declared more than once |
| E116 | `duplicate-state` | State name declared more than once |
| E117 | `duplicate-handler` | Handler declared more than once in same state |

### Semantic Errors (E4xx)

| Code | Name | Description |
|------|------|-------------|
| E400 | `unreachable-code` | Code after terminal statement |
| E401 | `frame-in-action` | Forbidden Frame statement in action or operation |
| E402 | `unknown-state` | Transition targets undefined state |
| E403 | `invalid-forward` | `=> $^` in state without parent |
| E405 | `param-arity-mismatch` | Wrong number of parameters |
| E406 | `multi-system-erlang` | Multiple systems in single file (Erlang target) |
| E407 | `frame-in-closure` | Frame statement inside nested function scope |
| E410 | `duplicate-state-var` | State variable declared more than once |
| E413 | `hsm-cycle` | Circular parent chain |

### Warnings (W4xx)

| Code | Name | Description |
|------|------|-------------|
| W414 | `unreachable-state` | State has no incoming transitions |

---

## Complete Example

```frame
import logging

@@target python_3
@@codegen {
    frame_event: on,
}

@@persist
@@system OrderProcessor ($(order_type), $>(initial_data), max_retries) {

    operations:
        static version(): str {
            return "1.0.0"
        }

    interface:
        submit(order)
        cancel(reason)
        getStatus(): str = "unknown"

    machine:
        $Idle {
            submit(order) {
                logging.info("Received order")
                $.order_data = order
                -> $Validating
            }
        }

        $Validating {
            $.order_data = None
            $.attempts: int = 0

            $>() {
                $.attempts = $.attempts + 1
                if validate($.order_data):
                    -> $Processing
                else:
                    if $.attempts >= self.max_retries:
                        -> $Failed
            }

            getStatus(): str {
                return "validating"
            }
        }

        $Processing {
            $>() {
                logging.info("Processing order")
            }

            cancel(reason) {
                (reason) -> $Cancelled
            }

            getStatus(): str {
                return "processing"
            }
        }

        $Cancelled {
            $>(reason) {
                logging.info(f"Cancelled: {reason}")
            }
        }

        $Failed {
            $>() {
                logging.error("Order failed")
            }
        }

    actions:
        validate(data) {
            return data is not None
        }

    domain:
        max_retries: int = 3
}

if __name__ == '__main__':
    proc = @@OrderProcessor("standard", {"source": "web"}, 5)
    proc.submit({"item": "widget", "qty": 3})
```
