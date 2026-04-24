# Frame Runtime Architecture

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

How Frame's language features are implemented in generated code. For the language itself, see [Frame Language Reference](frame_language.md). For framepiler internals, see [Framepiler Design](framepiler_design.md).

## Table of Contents

- [Overview](#overview)
- [Core Data Structures](#core-data-structures)
- [Dispatch Model](#dispatch-model)
- [Runtime Methods](#runtime-methods)
- [Transitions](#transitions)
- [State Stack Operations](#state-stack-operations)
- [Hierarchical State Machines](#hierarchical-state-machines)
- [System Context and Return Values](#system-context-and-return-values)
- [Self Interface Calls](#self-interface-calls)
- [State Variables](#state-variables)
- [Actions and Operations](#actions-and-operations)
- [Persistence](#persistence)
- [Per-Language Patterns](#per-language-patterns)

---

## Overview

Frame uses a **deferred transition** model. State changes are cached during event handling and processed by a central kernel after handler completion. This prevents stack overflow in long-running services and enables event forwarding.

All target languages implement the identical kernel/router/transition architecture described here.

---

## Core Data Structures

### Compartment

The compartment is Frame's central runtime data structure — "a closure concept for states that preserve the state itself, the data from the various scopes as well as runtime data needed for Frame machine semantics."

```
Compartment {
    state: string              # Current state identifier
    state_args: list           # Arguments passed via $State(args) — positional
    state_vars: dict           # State variables declared with $.varName
    enter_args: list           # Arguments passed via -> (args) $State — positional
    exit_args: list            # Arguments passed via (args) -> $State — positional
    forward_event: Event?      # Stashed event for -> => forwarding
}
```

**Key invariants:**
- Every system maintains `__compartment` (current state) and `__next_compartment` (pending transition, null when idle)
- Compartments are **copied** when pushed to the state stack, preserving all fields

### FrameEvent

Lean routing object:

```
FrameEvent {
    _message: string           # Event type ("$>", "<$", "methodName")
    _parameters: list          # Event arguments (positional)
}
```

Special messages: `"$>"` (enter), `"<$"` (exit).

### FrameContext

Call context for interface method invocations:

```
FrameContext {
    event: FrameEvent          # Reference to interface event
    _return: any               # Return value slot
    _data: dict                # Call-scoped data
}
```

- **Pushed** when an interface method is called
- **Popped** when the interface method returns
- Lifecycle events (`$>`, `<$`) use the existing context (no push/pop)
- Nested interface calls each have their own context (reentrancy)

### System Fields

Every generated system class contains:

```
__compartment: Compartment           # Current state's compartment
__next_compartment: Compartment?     # Pending transition target
_state_stack: list[Compartment]      # State history stack
_context_stack: list[FrameContext]   # Interface call context stack
```

---

## Dispatch Model

Frame's event dispatch is structured as a **three-layer pipeline**, with each layer holding a single responsibility:

```
┌───────────────────────────────────────────────────────────────────┐
│ Layer 1 — Router                                                  │
│   __router(e)                                                     │
│   • Reads the active state name off self.__compartment.           │
│   • Looks up the matching state dispatcher.                       │
│   • Calls dispatcher(e, self.__compartment).                      │
│                                                                   │
│   One router per system. Routes to N state dispatchers.           │
└──────────────────────────────┬────────────────────────────────────┘
                               │
                               ▼
┌───────────────────────────────────────────────────────────────────┐
│ Layer 2 — State dispatcher                                        │
│   _state_<State>(e, compartment)                                  │
│   • Thin switch on e._message.                                    │
│   • Each match calls its handler method and returns.              │
│   • Optional trailing call to parent dispatcher when => $^.       │
│                                                                   │
│   One dispatcher per state. Routes to K handler methods.          │
└──────────────────────────────┬────────────────────────────────────┘
                               │
                               ▼
┌───────────────────────────────────────────────────────────────────┐
│ Layer 3 — Handler method                                          │
│   _s_<State>_hdl_<kind>_<event>(e, compartment)                   │
│   • Binds handler params from e._parameters / enter_args / exit_args. │
│   • compartment ALWAYS belongs to this handler's own state — HSM  │
│     forwards pre-shift it at each `=> $^` via                     │
│     compartment.parent_compartment, so no walk inside the handler. │
│   • Executes the user-written handler body verbatim, with `$.x`,  │
│     `@@:return`, `@@:self.foo()`, etc. expanded by the codegen.   │
│                                                                   │
│   One method per (state, event). Contains only this handler's logic. │
└───────────────────────────────────────────────────────────────────┘
```

**Why layered?** Each layer does one thing. The router knows nothing about event kinds, the dispatcher knows nothing about handler bodies, and the handler knows nothing about routing. Adding a new event form (system params, async handlers, future sigils) changes only the mangler and the handler emitter — not the dispatcher template or the router. This is deliberately unlike a monolithic "state as one function with a 50-line if/elif chain" design.

**Lifecycle events travel through the same pipeline.** The kernel synthesizes `$>` and `<$` `FrameEvent`s during transitions and hands them to the router exactly like user events. They are routed to `_s_<State>_hdl_frame_enter` / `_s_<State>_hdl_frame_exit` through the normal dispatcher switch. There is no separate lifecycle fast-path.

---

## Runtime Methods

### `__kernel`

The central event processing loop:

```python
def __kernel(self, __e):
    self.__router(__e)

    while self.__next_compartment is not None:
        next_compartment = self.__next_compartment
        self.__next_compartment = None

        # Exit current state
        exit_event = FrameEvent("<$", self.__compartment.exit_args)
        self.__router(exit_event)

        # Switch compartment
        self.__compartment = next_compartment

        # Enter new state (or forward event)
        if next_compartment.forward_event is None:
            enter_event = FrameEvent("$>", self.__compartment.enter_args)
            self.__router(enter_event)
        else:
            forward_event = next_compartment.forward_event
            next_compartment.forward_event = None
            if forward_event._message == "$>":
                self.__router(forward_event)
            else:
                # Send $> first, THEN forward
                enter_event = FrameEvent("$>", self.__compartment.enter_args)
                self.__router(enter_event)
                self.__router(forward_event)
```

The "send enter first, then forward" behavior ensures the target state is initialized before handling the forwarded event.

### `__router`

Dispatches events to state-specific **state dispatcher** methods. The router passes `self.__compartment` explicitly as a third argument so dispatchers and their handler methods see the active compartment as a named local:

```python
def __router(self, __e):
    state_name = self.__compartment.state
    if state_name == "Idle":
        self._state_Idle(__e, self.__compartment)
    elif state_name == "Working":
        self._state_Working(__e, self.__compartment)
```

### `__transition`

Caches the next compartment (deferred):

```python
def __transition(self, next_compartment):
    self.__next_compartment = next_compartment
```

This does NOT execute immediately. The kernel processes it after the handler returns.

### State Methods

**Each state is implemented as two tiers:** a *thin dispatcher* (`_state_<Name>`) that routes wire messages to dedicated *handler methods* (`_s_<Name>_hdl_<kind>_<event>`). The handler body for each event lives in its own named method — **not inlined into a monolithic if/elif**. This design moves Frame's runtime model closer to the Elements of Programming ideal of "one symbol, one concept, one function," and makes stack traces, breakpoints, and profilers point at the specific handler that ran rather than a giant switch arm with a line number.

#### Dispatcher shape

The dispatcher is a flat series of guarded calls — each branch terminates on match with `return`, so exclusivity is explicit in control flow rather than implied by `elif` binding:

```python
def _state_MyState(self, __e, compartment):
    if __e._message == "$>":          self._s_MyState_hdl_frame_enter(__e, compartment); return
    if __e._message == "<$":          self._s_MyState_hdl_frame_exit(__e, compartment); return
    if __e._message == "process":     self._s_MyState_hdl_user_process(__e, compartment); return
    if __e._message == "reset":       self._s_MyState_hdl_user_reset(__e, compartment); return
    # The parent forward below is emitted ONLY when the state declares `=> $^`.
    # When absent, unhandled events fall off the end silently — matching V4
    # semantics (no auto-forward).
    self._state_ParentName(__e, compartment)
```

#### Handler method shape

Each handler is a standalone method. Parameters carry the event and the compartment that was active when the router ran:

```python
def _s_MyState_hdl_frame_enter(self, __e, compartment):
    # state-var initialization (only the lifecycle enter does this)
    if "count" not in compartment.state_vars:
        compartment.state_vars["count"] = 0
    # user-written body
    ...

def _s_MyState_hdl_user_process(self, __e, compartment):
    # typed-param binding from event
    data = __e._parameters[0]
    # user body
    compartment.state_vars["count"] = compartment.state_vars["count"] + 1
    ...
```

#### Name mangling scheme

| Handler kind | Method name | Rationale |
|---|---|---|
| Lifecycle `$>` | `_s_<State>_hdl_frame_enter` | `frame_` prefix reserves the lifecycle namespace |
| Lifecycle `<$` | `_s_<State>_hdl_frame_exit` | same |
| User interface method `probe` | `_s_<State>_hdl_user_probe` | `user_` prefix prevents collision with lifecycle names |

A user method named `enter` or `exit` cannot collide with a lifecycle handler because the prefixes put them in disjoint namespaces (`hdl_frame_*` vs `hdl_user_*`). This is a compile-time guarantee of the mangler, not a runtime check.

Single-underscore prefix (`_s_…`) avoids Python's double-underscore name mangling (`__name` → `_ClassName__name`), which would break dynamic lookups in the router. Target languages that don't have that concern (Java, C, Rust) still use the same naming for uniformity.

#### Why two tiers?

A single "monolithic" `_state_A` function with inlined handler bodies in if/elif arms is simpler to generate but has three problems the split form solves:

1. **Debuggability.** `_s_A_hdl_user_process` on a stack trace tells you exactly which handler fired. A monolithic function tells you only the state; you have to read line numbers against the source to find the handler.
2. **No name-space collisions.** Under the monolithic design, user interface methods named `enter` or `exit` had to be manually handled against the lifecycle wire messages `$>` / `<$`. The per-handler form puts them in different method namespaces entirely.
3. **Uniform codegen contract.** Every handler is a function with the same signature `(self, __e, compartment)`. Adding a new Frame event form (system params, persistence hooks, future sigils) registers one mangled name with the mangler and flows through every backend identically, instead of being threaded through 15 hand-maintained `fmt_if` / `fmt_elif` templates.

#### Worked example — end-to-end generated shape

Frame source:

```frame
@@target python_3

@@system Counter {
    interface:
        tick(step: int): int
        reset()

    machine:
        $A {
            $.n: int = 0

            $>() {
                # lifecycle enter — initialized lazily by the `if not in` guard
            }

            tick(step: int): int {
                $.n = $.n + step
                @@:($.n)
            }

            reset() {
                $.n = 0
            }
        }
}
```

Generated Python (abbreviated — shows the dispatch pipeline, elides FrameEvent/Compartment class boilerplate):

```python
class Counter:
    def __init__(self):
        self.__compartment = CounterCompartment("A")
        self.__next_compartment = None
        self._state_stack = []
        self._context_stack = []
        # Fire the start state's $>
        __frame_event = CounterFrameEvent("$>", self.__compartment.enter_args)
        self.__kernel(__frame_event)

    # ── interface wrappers ────────────────────────────────
    def tick(self, step: int) -> int:
        __e = CounterFrameEvent("tick", [step])
        __ctx = CounterFrameContext(__e, None)
        self._context_stack.append(__ctx)
        self.__kernel(__e)
        return self._context_stack.pop()._return

    def reset(self):
        __e = CounterFrameEvent("reset", [])
        __ctx = CounterFrameContext(__e, None)
        self._context_stack.append(__ctx)
        self.__kernel(__e)
        self._context_stack.pop()

    # ── router ─────────────────────────────────────────────
    def __router(self, __e):
        state_name = self.__compartment.state
        if state_name == "A":
            self._state_A(__e, self.__compartment)

    # ── state dispatcher (Layer 2) ─────────────────────────
    def _state_A(self, __e, compartment):
        if __e._message == "$>":    self._s_A_hdl_frame_enter(__e, compartment); return
        if __e._message == "tick":  self._s_A_hdl_user_tick(__e, compartment); return
        if __e._message == "reset": self._s_A_hdl_user_reset(__e, compartment); return

    # ── handler methods (Layer 3) ──────────────────────────
    def _s_A_hdl_frame_enter(self, __e, compartment):
        # $.n = 0 initialization, guarded so pop$ restore doesn't clobber.
        if "n" not in compartment.state_vars:
            compartment.state_vars["n"] = 0

    def _s_A_hdl_user_tick(self, __e, compartment):
        step = __e._parameters[0]
        compartment.state_vars["n"] = compartment.state_vars["n"] + step
        self._context_stack[-1]._return = compartment.state_vars["n"]

    def _s_A_hdl_user_reset(self, __e, compartment):
        compartment.state_vars["n"] = 0
```

**Reading guide:**

- `tick(step)` — entry from outside the system. Constructs a `FrameEvent`, pushes a `FrameContext`, calls the kernel. The kernel routes to `_state_A`, which delegates to `_s_A_hdl_user_tick`. The handler reads the param, mutates state_vars, and sets `_return` on the top context. The interface wrapper pops the context and returns `_return` to the caller.
- `$>` (lifecycle enter) — fired by the constructor. Same pipeline, but dispatched to `_s_A_hdl_frame_enter`. State-var init lives there — not in the dispatcher.
- A user method **named `enter`** or `exit` would generate `_s_A_hdl_user_enter` / `_s_A_hdl_user_exit` — not colliding with `_s_A_hdl_frame_enter` / `_s_A_hdl_frame_exit`. The mangler namespace prefix guarantees this is decidable at codegen time, not inferred at runtime.

#### `@@:params.X` and `@@:return` under the per-handler model

These accessors are unchanged in semantics; only the **locals they resolve against** change from "inlined in the dispatcher" to "bound at the top of the handler method":

| Frame accessor | Emitted Python (inside a handler method) |
|---|---|
| `@@:params.step` | `step` (bound from `__e._parameters[N]` at the top of `_s_A_hdl_user_tick`) |
| `@@:return` (read) | `self._context_stack[-1]._return` (dynamic langs) / target-native typed downcast (static langs) |
| `@@:return = <expr>` | `self._context_stack[-1]._return = <expr>` |
| `@@:(<expr>)` | `self._context_stack[-1]._return = <expr>` |
| `@@:event` | `self._context_stack[-1].event._message` |
| `@@:data.k` | `self._context_stack[-1]._data["k"]` |

The context stack is shared across layers — it holds the outermost interface call's state. A handler method sees the same top-of-stack context as the dispatcher and the router that called it. Nested self-calls (`@@:self.other()`) push their own contexts onto the stack and pop them when the inner interface wrapper returns, so each layer's `@@:return` refers to its own slot.

---

## Transitions

### Simple Transition (`-> $State`)

```python
__compartment = Compartment("Target")
self.__transition(__compartment)
return  # Handler exits; kernel processes transition
```

### Transition with State Args

```python
__compartment = Compartment("Target")
__compartment.state_args.append(value1)
self.__transition(__compartment)
return
```

### Transition with Enter Args

```python
__compartment = Compartment("Target")
__compartment.enter_args.append(data)
__compartment.enter_args.append(priority)
self.__transition(__compartment)
return
```

### Transition with Exit Args

```python
self.__compartment.exit_args.append(cleanup_data)  # Set on CURRENT compartment
__compartment = Compartment("Target")
self.__transition(__compartment)
return
```

### Event Forwarding (`-> => $State`)

```python
__compartment = Compartment("Target")
__compartment.forward_event = __e  # Stash current event
self.__transition(__compartment)
return
```

### Transition to Popped State (`-> pop$`)

```python
__compartment = self._state_stack.pop()
self.__transition(__compartment)
return
```

The popped compartment retains all its state variables — no reinitialization.

---

## State Stack Operations

### Push

```python
self._state_stack.append(self.__compartment)
```

The stack entry and `__compartment` point to the **same object** — push$ saves a reference, not a copy. Modifications after push$ are visible through both. For snapshot semantics, use `push$` with a transition (`push$ -> $State`) which creates a new compartment.

### Reentry vs History

| Transition Type | State Variable Behavior |
|----------------|------------------------|
| `-> $State` (normal) | State vars **reset** to initial values |
| `-> pop$` (history) | State vars **preserved** from saved compartment |

---

## Hierarchical State Machines

### Parent Forward (`=> $^`)

At the forward site, the child shifts `compartment` one level up to the parent's own compartment. The parent dispatcher — and any handler methods it calls — then sees **its own state's compartment** as the parameter:

```python
def _s_Child_hdl_user_event_b(self, __e, compartment):
    # handler-local work…
    self._state_Parent(__e, compartment.parent_compartment)  # shift up one level
```

This is the only place in the pipeline where compartment traversal happens. Handler methods never walk — they always receive their own state's compartment ready-to-use.

The chain composes naturally. Grandchild → Child → Parent is `compartment.parent_compartment` applied twice, once at each forward site:

```python
def _s_Grandchild_hdl_user_event(self, __e, compartment):
    self._state_Child(__e, compartment.parent_compartment)
    # ... where Child's dispatcher in turn emits:
    # self._state_Parent(__e, compartment.parent_compartment)
```

### Default Forward

A bare `=> $^` declaration on the state adds a trailing unguarded forward at the dispatcher tail, with the same one-level shift:

```python
def _state_Child(self, __e, compartment):
    if __e._message == "specific_event": self._s_Child_hdl_user_specific_event(__e, compartment); return
    # no elif-else — emitted only because the state declares `=> $^`
    self._state_Parent(__e, compartment.parent_compartment)
```

**V4 semantics:** Unhandled events are NOT automatically forwarded. `=> $^` must be used explicitly for a state to participate in parent-chain dispatch on unhandled events.

### What Handler Methods Always See

Because the shift happens at the forward site (not inside the handler), every handler has a clean, uniform contract:

| Signature parameter | Always refers to |
|---|---|
| `compartment` | The handler's own state's compartment |
| `compartment.state_vars` | This state's state-scoped variables |
| `compartment.state_args` | This state's declared state parameters |
| `compartment.enter_args` / `.exit_args` | Lifecycle event parameters for this state |
| `self.__compartment` | The currently ACTIVE state's compartment (may differ from `compartment` in HSM parent handlers) |

Handlers never need to walk the compartment chain to find their own state's vars — the walk has been pre-applied at the forward site. This is both simpler emission (no conditional HSM-walk preamble) and faster dispatch (one pointer-chase per forward level vs. one full walk per state-var access).

Handlers that need to reason about the active state (e.g. `@@:system.state`) still use `self.__compartment.state` — that intentionally reflects "which state is active," not "which handler is running."

### Implicit state-arg binding

State parameters (declared via `$State(arg1, arg2)`) flow into `compartment.state_args` at transition time. Inside the handler method, the emitter binds them to named locals at the top:

```python
def _s_Active_hdl_user_tick(self, __e, compartment):
    # bind declared state params from compartment.state_args
    initial = compartment.state_args[0]
    # handler body
    compartment.state_vars["count"] = compartment.state_vars["count"] + initial
```

Lifecycle enter/exit param binding works the same way but reads from `compartment.enter_args` / `compartment.exit_args`.

---

## System Context and Return Values

### Accessor Grammar

All `@@` accessors follow a uniform grammar:

- **`:`** (colon) — navigates Frame's namespace hierarchy
- **`.`** (dot) — accesses a field on the resolved object

| Frame Syntax | Runtime Access (Python) |
|-------------|------------------------|
| `@@:params.x` | `self._context_stack[-1].event._parameters[N]` (positional index) |
| `@@:return` | `self._context_stack[-1]._return` |
| `@@:event` | `self._context_stack[-1].event._message` |
| `@@:data.key` | `self._context_stack[-1]._data["key"]` |
| `@@:self.method(args)` | `self.method(args)` (generated interface call) |
| `@@:system.state` | `self.__compartment.state` |

### Interface Method Pattern

```python
def get_status(self) -> str:
    __e = FrameEvent("get_status", {})
    __ctx = FrameContext(__e, None)
    self._context_stack.append(__ctx)
    self.__kernel(__e)
    return self._context_stack.pop()._return
```

### Setting Return Value

`@@:return = value` or `@@:(value)` generates:

```python
self._context_stack[-1]._return = value
```

Note: `return expr` in handlers is a native return that exits the dispatch method — the value is NOT set on the context stack. The framepiler emits W415 warning for this pattern.

### Last Writer Wins

If multiple handlers in a transition chain set `@@:return`, the last value wins.

### Reentrancy

Each interface call pushes its own context:

```
_context_stack: [
    FrameContext(event="outer", _return="outer_value"),
    FrameContext(event="inner", _return="inner_value")
]
```

Inner `@@:return` does not affect outer `@@:return`.

---

## Self Interface Calls

`@@:self.method(args)` allows a system to call its own interface methods from within handlers and actions. The transpiler expands this into a native self-call on the generated interface method.

### Codegen Expansion

| Target | `@@:self.getStatus()` expands to |
|--------|----------------------------------|
| Python | `self.getStatus()` |
| TypeScript | `this.getStatus()` |
| Rust | `self.getStatus()` |
| C | `SystemName_getStatus(self)` |
| C++ | `this->getStatus()` |
| Go | `s.GetStatus()` |
| Java | `this.getStatus()` |

### Dispatch Pipeline

The expansion calls the generated interface method, which follows the standard pipeline:

```frame
@@:self.getStatus() called inside a handler
│
├─► Expands to self.getStatus() (Python)
├─► Interface method constructs FrameEvent("getStatus", {})
├─► FrameContext created, pushed to _context_stack
├─► Kernel dispatches event
│   ├─► Router selects current state's handler
│   └─► Handler executes (may set @@:return, trigger transitions)
├─► Context popped from _context_stack
└─► Return value available to the calling handler
```

### Context Isolation

The self-call pushes its own context. The calling handler's context is preserved:

```frame
Handler processing "analyze":
    _context_stack: [ctx_analyze]

    @@:self.getStatus()
        _context_stack: [ctx_analyze, ctx_getStatus]
        // inside getStatus: @@:event == "getStatus"
        // @@:return refers to ctx_getStatus._return
        _context_stack: [ctx_analyze]   // ctx_getStatus popped

    // back in analyze handler
    // @@:event == "analyze" (ctx_analyze is top again)
```

### Validation

The transpiler validates self-calls at transpile time:

| Code | Check |
|------|-------|
| E601 | Method does not exist in `interface:` block |
| E602 | Argument count mismatch |

These validations are not possible with native self-calls, which the transpiler treats as opaque native code.

---

## State Variables

### Storage

State variables are stored in `compartment.state_vars`, keyed by declared name. Because HSM forwards pre-shift the compartment parameter one level at each `=> $^` (see [Parent Forward](#parent-forward---)), a handler method's `compartment` parameter is **always** its own state's compartment:

```python
# Inside a handler method for state A (flat or HSM, doesn't matter).
value = compartment.state_vars["counter"]

# Assignment
compartment.state_vars["counter"] = value + 1
```

There is no HSM walk inside handler methods. The walk has already been performed at each forward site via `compartment.parent_compartment`, so by the time the parent's handler runs, `compartment` is already pointing at the parent's own compartment.

### Initialization

State variables initialize inside the lifecycle `$>` handler method:

```python
def _s_MyState_hdl_frame_enter(self, __e, compartment):
    if "counter" not in compartment.state_vars:
        compartment.state_vars["counter"] = 0
    if "data" not in compartment.state_vars:
        compartment.state_vars["data"] = {}
```

The `if not in` guard preserves state-var content on `pop$` restoration (see [State Stack Operations](#state-stack-operations)) — the handler runs only when the var doesn't already exist.

### Lifecycle

| Event | Behavior |
|-------|---------|
| `-> $State` | Variables initialized to declared values |
| `-> pop$` | Variables restored from saved compartment |
| Within state | Variables persist until state exits |

---

## Actions and Operations

### Actions

Private helpers with domain and context access:

```python
def __my_action(self, param):
    self.domain_var = param  # Can access domain vars
    # Can use @@:self.method() for self-calls
    # CANNOT use: -> $State, push$, pop$, $.varName
```

Actions cannot use Frame's `$.varName` syntax (error E401). State variable values should be passed as parameters if needed by an action.

### Operations

Public methods bypassing the state machine:

```python
def my_operation(self, param) -> int:
    return self.domain_var + param  # Direct domain access, no kernel
```

---

## Persistence

### What Gets Persisted

| Component | Description |
|-----------|-------------|
| Current compartment | State name, state_args, state_vars |
| HSM parent chain | Full parent_compartment chain |
| State stack | All pushed compartments |
| Domain variables | All current values |

### Restore Semantics

Restore does NOT invoke `$>`. The state is being *restored*, not *entered*.

| Operation | Enter Handler | State Vars |
|-----------|---------------|------------|
| `-> $State` | Invoked | Reset |
| `-> pop$` | Invoked | Preserved (not reset) |
| `restore_state()` | NOT invoked | Restored |

### Per-Language Serialization

| Language | Strategy | Format | External dependency |
|----------|----------|--------|-------------------|
| Python | `pickle` | Binary | none (stdlib) |
| GDScript | `JSON` | JSON | none (built-in) |
| TypeScript | `JSON.stringify`/`JSON.parse` | JSON | none (built-in) |
| JavaScript | `JSON.stringify`/`JSON.parse` | JSON | none (built-in) |
| Rust | `serde_json` | JSON | `serde_json = "1.0"` in Cargo.toml |
| C | cJSON | JSON | `#include <cjson/cJSON.h>`, link `-lcjson` |
| C++ | `nlohmann/json` | JSON | `#include <nlohmann/json.hpp>` |
| Java | manual JSON construction | JSON | none (stdlib `StringBuilder`) |
| Kotlin | manual JSON construction | JSON | none (stdlib) |
| Swift | `JSONSerialization` | JSON | none (Foundation) |
| C# | `System.Text.Json` | JSON | none (.NET built-in) |
| Go | `encoding/json` | JSON | none (stdlib) |
| Dart | `dart:convert` | JSON | none (SDK built-in) |
| PHP | `json_encode`/`json_decode` | JSON | none (built-in) |
| Ruby | `JSON` | JSON | none (stdlib) |
| Lua | manual JSON construction | JSON | none |
| Erlang | `jsx` or manual | JSON | `jsx` (optional) |

**Dependencies that require installation:**
- **Rust:** Add `serde_json = "1.0"` to `Cargo.toml`
- **C:** Install cJSON (`apt install libcjson-dev` on Debian/Ubuntu, `brew install cjson` on macOS) and add `#include <cjson/cJSON.h>` to your prolog. Link with `-lcjson`.
- **C++:** Install nlohmann/json and add `#include <nlohmann/json.hpp>` to your prolog.

All other languages use built-in or standard library JSON support — no external dependencies.

---

## String Interpolation Support

Frame constructs (`$.varName`, `@@:`) work inside string interpolation expressions for 8 languages. The scanner detects interpolation regions within string literals and scans them for Frame constructs while skipping the surrounding string content.

| Language | Interpolation syntax | Supported |
|----------|---------------------|-----------|
| Python | `f"...{expr}..."` | yes |
| TypeScript/JS | `` `...${expr}...` `` | yes |
| Kotlin/Dart | `"...${expr}..."` | yes |
| C# | `$"...{expr}..."` | yes |
| Ruby | `"...#{expr}..."` | yes |
| Swift | `"...\(expr)..."` | yes |
| C, C++, Java, Go, Lua, Erlang, PHP | n/a | no interpolation syntax |

**Quote-aware expansion:** When `$.varName` appears inside an interpolated string, the generated dict key uses the **opposite** quote from the string delimiter. Inside `f"text {$.count}"`, the expansion uses single quotes: `state_vars['count']`. Inside `f'text {$.count}'`, it uses double quotes: `state_vars["count"]`. This prevents quote collisions that would break the string.

---

## Runtime Edge Cases

### Stack Underflow
`-> pop$` on an empty state stack is undefined behavior. The generated code does not check — the pop will fail with a language-specific error (IndexError in Python, panic in Rust, etc.). Frame does not guarantee graceful handling.

### Self-Transitions
A transition to the current state (`-> $CurrentState`) fires the full lifecycle: `<$` (exit), state switch, `$>` (enter). State variables are reset to their initial values. This is intentional — self-transitions are a reinitialization mechanism.

### Exceptions in Handlers
If native code throws an exception inside a handler, the pending transition (if any) does NOT execute. The `__next_compartment` field is set but the kernel loop never processes it. The context stack entry is NOT popped — the interface wrapper's `finally` or equivalent handles cleanup. Behavior varies by target language.

### Context Data Scope
`@@:data` is scoped to the current interface call's context. It exists for the duration of the dispatch chain (handler -> exit -> transition -> enter) and is discarded when the interface method returns. Enter and exit handlers within the same dispatch chain share the same context data. A new interface call creates a fresh context with empty data.

---

## Per-Language Patterns

This section documents how Frame's runtime concepts map to each target language. Use it when writing Frame specs for a specific target — the native code inside handlers, actions, and epilog must follow the target language's patterns.

### Instantiation

`@@SystemName()` expands to the target language's construction pattern:

| Language | Declaration + instantiation | Cleanup |
|----------|---------------------------|---------|
| Python | `s = @@System()` | garbage collected |
| TypeScript | `const s = @@System()` | garbage collected |
| JavaScript | `const s = @@System()` | garbage collected |
| Rust | `let mut s = @@System()` | ownership / drop |
| C | `System* s = @@System()` | `System_destroy(s)` |
| C++ | `auto s = @@System()` | destructor / RAII |
| Java | `var s = @@System()` | garbage collected |
| Kotlin | `val s = @@System()` | garbage collected |
| Swift | `let s = @@System()` | ARC |
| C# | `var s = @@System()` | garbage collected |
| Go | `s := @@System()` | garbage collected |
| Dart | `final s = @@System()` | garbage collected |
| PHP | `$s = @@System()` | reference counted |
| Ruby | `s = @@System()` | garbage collected |
| Lua | `local s = @@System()` | garbage collected |
| GDScript | `var s = @@System()` | reference counted |
| Erlang | `{ok, Pid} = System:start_link()` | gen_statem process |

C is the only backend that requires explicit cleanup via `System_destroy(s)`. Rust uses ownership semantics — no explicit destroy needed.

### System Class Visibility

System classes are public by default. `@@system private Foo` overrides this for languages that support class visibility:

| Language | Default (public) | `@@system private Foo` |
|----------|-----------------|----------------------|
| Java | `public class Foo` | `class Foo` |
| C# | `public class Foo` | `class Foo` |
| Swift | `public class Foo` | `class Foo` |
| TypeScript | `export class Foo` | `class Foo` |
| JavaScript | `export class Foo` | `class Foo` |
| Kotlin | `class Foo` | `private class Foo` |
| Rust | `pub struct Foo` | `struct Foo` |
| Dart | `class Foo` | `class Foo` |
| C++ | `class Foo` | `class Foo` |
| Go | `type Foo struct` | `type Foo struct` |
| PHP | `class Foo` | `class Foo` |
| Python | `class Foo:` | error (not supported) |
| Ruby | `class Foo` | error (not supported) |
| Lua | table+metatable | error (not supported) |
| GDScript | `class Foo:` | error (not supported) |
| Erlang | `-module(foo).` | error (not supported) |
| C | `struct Foo` | error (not supported) |

Helper classes (FrameEvent, Compartment, FrameContext) are always non-public regardless of the system's visibility.

### Interface Method Calls

Interface methods are called in the target language's native method syntax:

| Language | Call syntax |
|----------|------------|
| Python | `s.method(args)` |
| TypeScript/JS | `s.method(args)` |
| Rust | `s.method(args)` |
| C | `System_method(s, args)` |
| C++ | `s->method(args)` or `s.method(args)` |
| Java/Kotlin/C#/Dart | `s.method(args)` |
| Swift | `s.method(args)` |
| Go | `s.Method(args)` (exported, capitalized) |
| PHP | `$s->method(args)` |
| Ruby | `s.method(args)` |
| Lua | `s:method(args)` |
| GDScript | `s.method(args)` |
| Erlang | `gen_statem:call(Pid, {method, Args})` |

### Action Calls (inside handlers)

Actions are native methods on the system. Inside handler bodies, call them using the target's self-reference:

| Language | Call syntax inside handler |
|----------|--------------------------|
| Python | `self.action(args)` |
| TypeScript/JS | `this.action(args)` |
| Rust | `self.action(args)` |
| C | `System_action(self, args)` |
| C++ | `this->action(args)` |
| Java/Kotlin/C#/Dart | `this.action(args)` |
| Swift | `self.action(args)` |
| Go | `s.action(args)` |
| PHP | `$this->action(args)` |
| Ruby | `self.action(args)` |
| Lua | `self:action(args)` |
| GDScript | `self.action(args)` |

**Note:** C actions are generated as free functions prefixed with the system name. All other languages generate them as methods.

### Operation Calls (inside handlers)

Operations follow the same call pattern as actions. The difference is that operations bypass the state machine dispatch — they're direct method calls:

| Language | Call syntax inside handler |
|----------|--------------------------|
| Python | `self.operation(args)` |
| TypeScript/JS | `this.operation(args)` |
| Rust | `self.operation(args)` |
| C | `System_operation(self, args)` |
| C++ | `this->operation(args)` |
| Go | `s.Operation(args)` (capitalized) |
| Others | same as action call syntax |

Operations can declare return types using the target language's native syntax in the operation body. Since the body is native code, `return value;` works as expected.

### Domain Field Declarations

Domain fields are declared as `name: type` or `name: type = init`. The type and initializer are passed through verbatim to the target language:

| Language | Example |
|----------|---------|
| Python | `count: int = 0` |
| TypeScript | `count: number = 0` |
| Rust | `count: i64 = 0` |
| C | `count: int = 0` (generates `int count;`) |
| C++ | `count: int = 0` |
| Java | `count: int = 0` (generates `int count = 0;`) |
| Kotlin | `count: Int = 0` |
| Go | `count: int = 0` |

**C arrays:** Declare as `name: type` where type includes the array size. Example: `buffer: char[64]` generates `char buffer[64];`. Do **not** use `char[64]` as the type with a separate name — Frame treats the entire type string as opaque and places the identifier after it.

**Rust:** Domain fields become struct fields. Use Rust types directly: `items: Vec<String> = Vec::new()`, `map: HashMap<i64, String> = HashMap::new()`.

**Init is optional** for static targets that zero-initialize (C, C++, Go). Required for dynamic targets (Python, JS, Ruby, etc.) where uninitialized fields would be undefined.

### Compartment Implementation

| Language | State Vars | State Stack | Self |
|----------|-----------|-------------|------|
| Python | `dict` | `list` | `self` |
| TypeScript/JS | `Record<string, any>` | `Array` | `this` |
| Rust | typed `StateContext` enum | `Vec` | `self` |
| C | `FrameDict*` | linked list | `self->` |
| C++ | `std::unordered_map<string, any>` | `std::vector` | `this->` |
| Java | `HashMap<String, Object>` | `ArrayList` | `this` |
| Kotlin | `MutableMap<String, Any?>` | `MutableList` | `this` |
| Go | `map[string]interface{}` | `[]Compartment` | `s.` |
| Swift | `[String: Any]` | `[Compartment]` | `self` |
| C# | `Dictionary<string, object>` | `List` | `this` |
| Dart | `Map<String, dynamic>` | `List` | `this` |
| PHP | `array` | `array` | `$this->` |
| Ruby | `Hash` | `Array` | `@` (instance vars) |
| Lua | `table` | `table` | `self.` |
| GDScript | `Dictionary` | `Array` | `self.` |
| Erlang | gen_statem state data | list in data | n/a (functional) |

### Router Dispatch

| Language | Pattern |
|----------|---------|
| Python | `if/elif` chain |
| GDScript | `if/elif` chain |
| TypeScript/JS | `if/else if` chain |
| Rust | `match` expression |
| C | `if/else if` with `strcmp` |
| C++ | `if/else if` with `==` |
| Java | `if/else if` with `.equals()` |
| Kotlin | `if/else if` with `==` |
| Swift | `if/else if` chain |
| C# | `if/else if` chain |
| Go | `switch` statement |
| PHP | `if/elseif` chain |
| Ruby | `if/elsif` chain |
| Lua | `if/elseif` chain |
| Dart | `if/else if` chain |
| Erlang | function clause matching |

### Self Interface Call Expansion

| Language | `@@:self.method(args)` expands to |
|----------|----------------------------------|
| Python | `self.method(args)` |
| GDScript | `self.method(args)` |
| TypeScript/JS | `this.method(args)` |
| Rust | `self.method(args)` |
| C | `System_method(self, args)` |
| C++ | `this->method(args)` |
| Java/Kotlin/C#/Dart | `this.method(args)` |
| Swift | `self.method(args)` |
| Go | `s.Method(args)` |
| PHP | `$this->method(args)` |
| Ruby | `self.method(args)` |
| Lua | `self:method(args)` |

### String Comparison in Native Code

When comparing strings in handler native code, use the target language's string comparison:

| Language | Equality check |
|----------|---------------|
| Python | `s == "value"` |
| TypeScript/JS | `s === "value"` |
| Rust | `s == "value"` |
| C | `strcmp(s, "value") == 0` |
| C++ | `s == "value"` |
| Java | `s.equals("value")` |
| Kotlin | `s == "value"` |
| Go | `s == "value"` |
| Others | `s == "value"` |

---

## Migration notes — from monolithic dispatch to per-handler methods

Historically, framec emitted each state as a single monolithic function with handler bodies inlined in an if/elif chain:

```python
# Historical form — DEPRECATED
def _state_A(self, __e):
    __sv_comp = self.__compartment
    while __sv_comp and __sv_comp.state != "A":
        __sv_comp = __sv_comp.parent_compartment
    if __e._message == "$>":
        if "n" not in __sv_comp.state_vars:
            __sv_comp.state_vars["n"] = 0
    elif __e._message == "tick":
        step = __e._parameters[0]
        __sv_comp.state_vars["n"] = __sv_comp.state_vars["n"] + step
        self._context_stack[-1]._return = __sv_comp.state_vars["n"]
```

The per-handler form replaces this with a thin dispatcher plus named handler methods. Concretely:

| What changes | From | To |
|---|---|---|
| State function signature | `(self, __e)` | `(self, __e, compartment)` — router passes `self.__compartment` in |
| State function body | inlined if/elif with handler bodies | thin dispatcher calling handler methods |
| Handler body location | inlined in dispatcher | separate method `_s_<State>_hdl_<kind>_<event>` |
| State-var access | `__sv_comp.state_vars[…]` via enclosing-scope local (set by an HSM-walk preamble at dispatcher entry) | `compartment.state_vars[…]` via parameter — no walk in handlers |
| HSM forward (`=> $^`) | `self._state_Parent(__e)` | `self._state_Parent(__e, compartment.parent_compartment)` — shift up one level |
| User method `enter`/`exit` | required alias arm in dispatch template | mangled to `hdl_user_enter` — different namespace |
| Lifecycle state-var init | inside the dispatcher's `"$>"` branch | inside `_s_<State>_hdl_frame_enter` |

The router's signature change (`dispatcher(e, compartment)`) propagates into every emitter — routing, transitions, forward calls to parent dispatchers. The `_context_stack`/`_state_stack`/`__compartment` fields and their semantics are unchanged. The kernel loop is unchanged. The transition deferred-commit mechanism is unchanged.

Backends that already emit per-handler methods (Rust, Erlang via gen_statem) are structurally aligned with this model and need only mangler-level cleanup, not architectural change.

**Design call:** HSM compartment traversal happens exactly once per `=> $^` forward, via `compartment.parent_compartment` at the emission site. Handler methods receive their own state's compartment ready-to-use, with no walk preamble. This is a true O(1) shift per forward — not O(depth) per state-var access — and makes every handler's contract uniform regardless of whether its state participates in an HSM chain.