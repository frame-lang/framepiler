# Frame Runtime Architecture

How Frame's language features are implemented in generated code. For the language itself, see [Frame Language Reference](frame_language.md). For framepiler internals, see [Framepiler Design](framepiler_design.md).

## Table of Contents

- [Overview](#overview)
- [Core Data Structures](#core-data-structures)
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
    state_args: dict           # Arguments passed via $State(args)
    state_vars: dict           # State variables declared with $.varName
    enter_args: dict           # Arguments passed via -> (args) $State
    exit_args: dict            # Arguments passed via (args) -> $State
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
    _parameters: dict | null   # Event arguments
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

Dispatches events to state-specific handler methods:

```python
def __router(self, __e):
    state_name = self.__compartment.state
    if state_name == "Idle":
        self._state_Idle(__e)
    elif state_name == "Working":
        self._state_Working(__e)
```

### `__transition`

Caches the next compartment (deferred):

```python
def __transition(self, next_compartment):
    self.__next_compartment = next_compartment
```

This does NOT execute immediately. The kernel processes it after the handler returns.

### State Methods

Each state generates a dispatch method:

```python
def _state_MyState(self, __e):
    if __e._message == "$>":
        # Initialize state variables
        self.__compartment.state_vars["count"] = 0
        # Execute enter handler body
    elif __e._message == "<$":
        # Execute exit handler body
    elif __e._message == "process":
        # Execute event handler body
    # Unhandled events: do nothing (no auto-forward)
```

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
__compartment.state_args["param1"] = value1
self.__transition(__compartment)
return
```

### Transition with Enter Args

```python
__compartment = Compartment("Target")
__compartment.enter_args["0"] = data
__compartment.enter_args["1"] = priority
self.__transition(__compartment)
return
```

### Transition with Exit Args

```python
self.__compartment.exit_args["0"] = cleanup_data  # Set on CURRENT compartment
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

Generated as a direct call to the parent's state method:

```python
def _state_Child(self, __e):
    if __e._message == "event_b":
        self._state_Parent(__e)  # Direct call
```

### Default Forward

A bare `=> $^` at state level adds an else clause:

```python
def _state_Child(self, __e):
    if __e._message == "specific_event":
        ...
    else:
        self._state_Parent(__e)  # Default forward
```

**V4 semantics:** Unhandled events are NOT automatically forwarded. `=> $^` must be used explicitly.

---

## System Context and Return Values

### Accessor Grammar

All `@@` accessors follow a uniform grammar:

- **`:`** (colon) — navigates Frame's namespace hierarchy
- **`.`** (dot) — accesses a field on the resolved object

| Frame Syntax | Runtime Access (Python) |
|-------------|------------------------|
| `@@:params.x` | `self._context_stack[-1].event._parameters["x"]` |
| `@@:return` | `self._context_stack[-1]._return` |
| `@@:event` | `self._context_stack[-1].event._message` |
| `@@:data.key` | `self._context_stack[-1]._data["key"]` |
| `@@:self` | `self` |
| `@@:self.state` | `self.__compartment.state` |

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

```
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

```
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

State variables are stored in `compartment.state_vars`:

```python
# Access
value = self.__compartment.state_vars["counter"]

# Assignment
self.__compartment.state_vars["counter"] = value + 1
```

### Initialization

State variables initialize when the `$>` handler runs:

```python
def _state_MyState(self, __e):
    if __e._message == "$>":
        self.__compartment.state_vars["counter"] = 0
        self.__compartment.state_vars["data"] = {}
```

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

| Language | Strategy | Format |
|----------|----------|--------|
| Python | `pickle` | Binary |
| TypeScript | `JSON.stringify`/`JSON.parse` | JSON |
| Rust | `serde_json` | JSON |
| C | cJSON library | JSON |

Languages not shown follow the TypeScript/JSON pattern. See the [Language Reference](frame_language.md#persistence) for full coverage.

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

### Compartment Implementation

| Language | State Vars | State Stack | Self |
|----------|-----------|-------------|------|
| Python | `dict` | `list` | `self` |
| TypeScript | `Record<string, any>` | `Array` | `this` |
| Rust | `HashMap` or typed `StateContext` enum | `Vec` | `self` |
| C | `FrameDict*` | `FrameVec*` | `self->` |
| C++ | `std::unordered_map<string, any>` | `std::vector` | `this->` |
| Java | `HashMap<String, Object>` | `ArrayList` | `this` |
| Go | `map[string]interface{}` | `[]Compartment` | `s.` |

### Router Dispatch

| Language | Pattern |
|----------|---------|
| Python | `if/elif` chain |
| TypeScript | `switch` statement |
| Rust | `match` expression |
| C | `if/else if` with `strcmp` |
| C++ | `if/else if` with `==` |
| Java | `switch` statement |
| Go | `switch` statement |

### Self Interface Call Expansion

| Language | `@@:self.method(args)` |
|----------|------------------------|
| Python | `self.method(args)` |
| TypeScript | `this.method(args)` |
| Rust | `self.method(args)` |
| C | `SystemName_method(self, args)` |
| C++ | `this->method(args)` |
| Java | `this.method(args)` |
| Go | `s.Method(args)` |