# Frame Runtime Architecture

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

How Frame's language features are implemented in generated code. For
the language itself, see [Frame Language Reference](frame_language.md).
For framepiler internals, see [Framepiler Design](framepiler_design.md).
For a progressive walkthrough that builds up the runtime piece by
piece, see [Frame Runtime Walkthrough](frame_runtime_walkthrough.md).

## Table of Contents

- [Overview](#overview)
- [Core Data Structures](#core-data-structures)
- [The Three Data Stores](#the-three-data-stores)
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
- [Runtime Edge Cases](#runtime-edge-cases)

---

## Overview

Frame uses a **deferred transition** model. State changes are cached
during event handling and processed by a central kernel after handler
completion. This prevents stack overflow in long-running services and
enables event forwarding.

All target languages implement the identical kernel/router/dispatcher/
handler architecture described here.

### The runtime skeleton is always fully emitted

Even a one-state, one-event system produces the complete
FrameEvent / FrameContext / Compartment classes, both stacks
(`_state_stack`, `_context_stack`), the full kernel transition loop,
and all lifecycle-event routing. Features don't add runtime
machinery — they add handler bodies that invoke machinery already
present.

This is deliberate: it gives every Frame system the same runtime
shape regardless of which features it uses, making the generated code
predictable in footprint and uniform to debug. Per-system scaffolding
cost is ~95 lines in Python, with comparable ratios in every target.

### Key architectural commitments

Frame's runtime rests on four commitments that shape everything else:

1. **Functional equivalence across backends.** A Frame system's
   behavior is defined by its source, not by its implementation in
   any target language. The migration scenario (compiling in one
   language, serializing, resuming in another) is the stress test of
   this commitment.
2. **Per-handler dispatch.** Every event handler is a separate named
   method, not an inlined arm of a monolithic if/elif chain. Stack
   traces, breakpoints, and profilers point at the specific handler
   that ran.
3. **Uniform compartment construction.** Every compartment (leaf or
   HSM ancestor) is constructed and populated identically. No special
   cases for ancestors; no runtime walking to find the right
   compartment.
4. **Uniform parameter propagation.** The three parameter channels
   (state_args, enter_args, exit_args) propagate from transition
   sites to every layer of the HSM chain identically.

---

## Core Data Structures

### Compartment

The compartment is Frame's central runtime data structure — a closure
for states that preserves the state itself, the data from the various
scopes, and runtime data needed for Frame machine semantics.

```
Compartment {
    state: string                    # Current state identifier
    state_args: list                 # State parameters — positional
    state_vars: dict                 # State-local variables ($.varName)
    enter_args: list                 # $> handler parameters — positional
    exit_args: list                  # <$ handler parameters — positional
    forward_event: Event?            # Stashed event for -> => forwarding
    parent_compartment: Compartment? # HSM parent reference
}
```

**Key invariants:**
- Every system maintains `__compartment` (current state's compartment)
  and `__next_compartment` (pending transition target, null when
  idle).
- `parent_compartment` is populated at transition time from the
  system's static HSM topology. Handlers never walk this chain to
  find state-local data — they receive their own compartment as a
  parameter.
- Each compartment owns its `state_args`, `enter_args`, `exit_args`
  lists. Even when values propagate across HSM layers (see
  [Uniform Parameter Propagation](#uniform-parameter-propagation)),
  each layer holds its own list copy — no aliasing across layers.

### FrameEvent

Lean routing object:

```
FrameEvent {
    _message: string     # Event type ("$>", "<$", "methodName")
    _parameters: list    # Positional parameter vector
}
```

Special messages: `"$>"` (enter), `"<$"` (exit).

**Parameters are positional.** `_parameters` is a positional list at
the wire level. Named access in Frame source (`@@:params.x` or the
declared parameter name) is a **compile-time rewrite** that binds each
parameter to a typed local at the top of the handler body. This is
the same mechanism any statically-typed language uses for function
parameters: positional at the calling convention, named in source,
resolved at compile time.

### FrameContext

Call context for interface method invocations:

```
FrameContext {
    event: FrameEvent    # Reference to interface event
    _return: any         # Return value slot
    _data: dict          # Call-scoped data (@@:data)
}
```

- **Pushed** when an interface method is called.
- **Popped** when the interface method returns.
- Lifecycle events (`$>`, `<$`) use the existing context (no
  push/pop).
- Nested interface calls each have their own context (reentrancy).

### System Fields

Every generated system class contains:

```
__compartment: Compartment           # Current state's compartment
__next_compartment: Compartment?     # Pending transition target
_state_stack: list[Compartment]      # State history stack
_context_stack: list[FrameContext]   # Interface call context stack
```

---

## The Three Data Stores

Frame gives handlers exactly three places to store data. They differ
by **lifetime** and **scope**:

| Store | Lifetime | Scope | Accessor | Storage |
|---|---|---|---|---|
| Domain | System lifetime | All handlers, all states | `self.field` | System instance |
| State vars | State-instance lifetime | One state's handlers | `$.x` | `compartment.state_vars` |
| Context data | One dispatch chain | One interface call's handlers | `@@:data.k` | `_context_stack[-1]._data` |

These three are orthogonal. They answer three different questions:
"what does the system remember," "what does this state remember,"
"what does this call carry."

### Decision rule

**Pick the store with the shortest lifetime that covers the data's
scope.**

- Context data for data needed only within one interface call.
- State vars for data local to a state's time of being current.
- Domain for data that persists across all states and all calls.

### Context data is always a dynamic map

Unlike domain fields and state variables, context data has no declared
schema. Keys are created on write, values are stored as the target's
"any" type (`dict` in Python, `Map<String, Object>` in Java,
`HashMap<String, Box<dyn Any>>` in Rust, and so on).

This is intentional. `@@:data`'s scope spans an arbitrary subset of a
system's handlers, actions, and lifecycle handlers — whatever gets
reached during one dispatch chain. A declared schema would need to be
the union of every key any callable might set, which devolves to
"dynamic map with types tracked externally." Frame represents this
honestly: the store is dynamic in every target.

**In dynamically-typed targets** (Python, JavaScript, Ruby, Lua, PHP,
GDScript, Erlang) this is invisible — reads and writes look like
ordinary variable access.

**In statically-typed targets** (Java, Kotlin, C#, Swift, Go, Rust,
C, C++, TypeScript, Dart) reads evaluate to the target's "any" type
and may require a cast at the use site. When a value is assigned to a
typed local, the framepiler emits the cast automatically; in more
general expressions, the cast is user-written.

### What each store is good for

**Domain.** System-wide state. Configuration that doesn't change.
Counters that span many interface calls. Connections, caches, and
anything else with system lifetime.

**State vars.** State-specific state (literally). A timer's remaining
duration, a buffer's contents, a workflow phase's accumulator — data
that exists only while a particular state is active and should reset
when the state exits and re-enters.

**Context data.** Call-scoped scratch. Timestamps for *this* call.
Intermediate decisions the handler makes and needs to pass to an
action it will call. Data that spans the lifecycle events triggered
by a transition in this call (`<$` of the source state, `$>` of the
target state — both see the same `@@:data`).

Context data is the only store reachable by actions (actions don't
have a compartment, so `$.x` is unavailable there — see
[Actions and Operations](#actions-and-operations)).

---

## Dispatch Model

Frame's event dispatch is structured as a **four-layer pipeline**,
with each layer holding a single responsibility:

```
┌───────────────────────────────────────────────────────────────────┐
│ Layer 0 — Kernel                                                  │
│   __kernel(e)                                                     │
│   • Entry point for every event — interface calls, lifecycle      │
│     events, forwarded events all flow through here.               │
│   • Calls __router(e) to dispatch the event.                      │
│   • After the router returns, processes any pending transition    │
│     by synthesizing <$ / $> events and re-entering __router.      │
│   • Loops until __next_compartment is null.                       │
│                                                                   │
│   One kernel per system. Drives the pipeline.                     │
└──────────────────────────────┬────────────────────────────────────┘
                               │
                               ▼
┌───────────────────────────────────────────────────────────────────┐
│ Layer 1 — Router                                                  │
│   __router(e)                                                     │
│   • Reads the active state name off self.__compartment.           │
│   • Dispatches to the matching state dispatcher via static        │
│     if/elif chain.                                                │
│   • Passes self.__compartment explicitly as a third argument.     │
│                                                                   │
│   One router per system. Routes to N state dispatchers.           │
└──────────────────────────────┬────────────────────────────────────┘
                               │
                               ▼
┌───────────────────────────────────────────────────────────────────┐
│ Layer 2 — State dispatcher                                        │
│   _state_<State>(e, compartment)                                  │
│   • Flat series of guarded calls, one per declared event.         │
│   • Each match terminates with a handler call and return.         │
│   • Optional trailing call to parent dispatcher when state        │
│     declares `=> $^`.                                             │
│                                                                   │
│   One dispatcher per state. Routes to K handler methods.          │
└──────────────────────────────┬────────────────────────────────────┘
                               │
                               ▼
┌───────────────────────────────────────────────────────────────────┐
│ Layer 3 — Handler method                                          │
│   _s_<State>_hdl_<kind>_<event>(e, compartment)                   │
│   • Binds handler params from e._parameters / enter_args /        │
│     exit_args at the top of the method.                           │
│   • compartment always belongs to this handler's own state —      │
│     HSM forwards pre-shift it at each `=> $^`, so no walk         │
│     inside the handler.                                           │
│   • Executes the user-written handler body with $.x, @@:return,   │
│     @@:self.foo(), etc. expanded by the codegen.                  │
│                                                                   │
│   One method per (state, event). Contains only this handler's    │
│   logic.                                                          │
└───────────────────────────────────────────────────────────────────┘
```

**Why four layers?** Each layer does one thing. The kernel knows
nothing about event kinds or state names. The router knows nothing
about event messages. The dispatcher knows nothing about handler
bodies. The handler knows nothing about routing or transitions.

This matters for evolution. Adding a new event form changes only the
mangler and the handler emitter, not the layers above. Adding a new
transition form changes only the kernel. Adding a new state doesn't
change the kernel or router structure, just appends one branch to the
router and one dispatcher method. Single-axis-of-change per layer.

### Lifecycle events travel through the same pipeline

The kernel synthesizes `$>` and `<$` FrameEvents during transitions
and hands them to the router exactly like user events. They are
routed to `_s_<State>_hdl_frame_enter` / `_s_<State>_hdl_frame_exit`
through the normal dispatcher switch. There is no separate lifecycle
fast-path — lifecycle events are ordinary events with reserved
messages.

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

        # Exit current state (cascade bottom-up for HSM)
        self.__fire_exit_cascade()

        # Switch compartment
        self.__compartment = next_compartment

        # Enter new state (cascade top-down for HSM)
        if next_compartment.forward_event is None:
            self.__fire_enter_cascade()
        else:
            # Send $> first, then forward the stashed event
            forward_event = next_compartment.forward_event
            next_compartment.forward_event = None
            self.__fire_enter_cascade()
            self.__router(forward_event)
```

The kernel drives the pipeline. When a handler calls `__transition`,
it only *caches* the next compartment — the kernel processes it after
the handler returns. This deferred model prevents stack overflow in
long-running services and cleanly separates handler logic from
lifecycle plumbing.

The cascade helpers (`__fire_exit_cascade`, `__fire_enter_cascade`)
walk the HSM chain appropriately — see
[Hierarchical State Machines](#hierarchical-state-machines).

### `__router`

Static if/elif dispatch to state dispatcher methods:

```python
def __router(self, __e):
    state_name = self.__compartment.state
    if state_name == "Idle":
        self._state_Idle(__e, self.__compartment)
    elif state_name == "Working":
        self._state_Working(__e, self.__compartment)
    elif state_name == "Done":
        self._state_Done(__e, self.__compartment)
```

One branch per state. The router passes `self.__compartment`
explicitly as a third argument so dispatchers and their handler
methods see the active compartment as a named local.

### `__transition`

Caches the next compartment (deferred):

```python
def __transition(self, next_compartment):
    self.__next_compartment = next_compartment
```

This does NOT execute immediately. The kernel processes it after the
handler returns.

### `__prepareEnter`

Constructs the destination HSM chain for a transition. Data-driven
from a static HSM topology table emitted once per system:

```python
# Emitted once per system, derived from static HSM declarations:
_HSM_CHAIN = {
    'Sibling':    ['Sibling'],
    'Parent':     ['Parent'],
    'Child':      ['Parent', 'Child'],
    'Grandchild': ['Parent', 'Child', 'Grandchild'],
}

def __prepareEnter(self, leaf, state_args, enter_args):
    comp = None
    for name in _HSM_CHAIN[leaf]:
        comp = Compartment(name, comp)
        comp.state_args = list(state_args)
        comp.enter_args = list(enter_args)
        # state_vars initialize lazily in the state's $> handler
        # exit_args remain [] until set by a future __prepareExit
    return comp
```

Every compartment in the chain receives its own list copy of
`state_args` and `enter_args`. All layers hold the same values — no
shared references — so mutation of one layer's list doesn't affect
other layers.

For non-HSM transitions, the chain has length 1 and the pattern still
works.

### `__prepareExit`

Populates exit_args across the source HSM chain before the kernel's
exit cascade fires:

```python
def __prepareExit(self, exit_args):
    comp = self.__compartment
    while comp is not None:
        comp.exit_args = list(exit_args)
        comp = comp.parent_compartment
```

No topology table needed — the source chain already exists via the
current compartment's `parent_compartment` links. For transitions
without exit_args, this call is omitted entirely.

### State Dispatchers

**Each state is implemented as two tiers:** a *thin dispatcher*
(`_state_<Name>`) that routes wire messages to dedicated *handler
methods* (`_s_<Name>_hdl_<kind>_<event>`). The handler body for each
event lives in its own named method — not inlined into a monolithic
if/elif.

#### Dispatcher shape

The dispatcher is a flat series of guarded calls — each branch
terminates on match with `return`:

```python
def _state_MyState(self, __e, compartment):
    if __e._message == "$>":      self._s_MyState_hdl_frame_enter(__e, compartment); return
    if __e._message == "<$":      self._s_MyState_hdl_frame_exit(__e, compartment); return
    if __e._message == "process": self._s_MyState_hdl_user_process(__e, compartment); return
    if __e._message == "reset":   self._s_MyState_hdl_user_reset(__e, compartment); return
    # The parent forward below is emitted ONLY when the state declares `=> $^`.
    # When absent, unhandled events fall off the end silently.
    self._state_ParentName(__e, compartment.parent_compartment)
```

#### Handler method shape

Each handler is a standalone method. Parameters carry the event and
the compartment that was active when the router ran:

```python
def _s_MyState_hdl_frame_enter(self, __e, compartment):
    # State-var initialization (only lifecycle enter does this)
    if "count" not in compartment.state_vars:
        compartment.state_vars["count"] = 0

def _s_MyState_hdl_user_process(self, __e, compartment):
    # Parameter binding from positional _parameters
    data = __e._parameters[0]
    # Handler body
    compartment.state_vars["count"] = compartment.state_vars["count"] + 1
```

#### Name mangling

| Handler kind | Method name |
|---|---|
| Lifecycle `$>` | `_s_<State>_hdl_frame_enter` |
| Lifecycle `<$` | `_s_<State>_hdl_frame_exit` |
| User interface method `probe` | `_s_<State>_hdl_user_probe` |

The `hdl_frame_*` vs `hdl_user_*` prefix split is a namespace
guarantee: a user interface method named `enter` or `exit` cannot
collide with a lifecycle handler because they live in disjoint
namespaces. This is resolved at codegen time, not at runtime.

Single-underscore prefix (`_s_…`) avoids Python's name mangling
(`__name` → `_ClassName__name`). Target languages without this
concern still use the same scheme for uniformity across backends.

---

## Transitions

### Simple Transition (`-> $State`)

```python
next_comp = self.__prepareEnter("Target", [], [])
self.__transition(next_comp)
return  # Handler exits; kernel processes transition
```

### Transition with State Args

```python
next_comp = self.__prepareEnter("Target", [value1, value2], [])
self.__transition(next_comp)
return
```

### Transition with Enter Args

```python
next_comp = self.__prepareEnter("Target", [], [data, priority])
self.__transition(next_comp)
return
```

### Transition with Exit Args

```python
self.__prepareExit([cleanup_data])
next_comp = self.__prepareEnter("Target", [], [])
self.__transition(next_comp)
return
```

### Full transition with all three channels

```python
self.__prepareExit(["done"])
next_comp = self.__prepareEnter("Target", ["ABC123"], ["hello"])
self.__transition(next_comp)
return
```

Three calls, three responsibilities:

| Step | Responsibility | Acts on |
|---|---|---|
| `__prepareExit` | Populate source chain for exit cascade | Current chain |
| `__prepareEnter` | Construct destination chain | New chain |
| `__transition` | Cache destination; kernel processes rest | Kernel signal |

### Event Forwarding (`-> => $State`)

```python
next_comp = self.__prepareEnter("Target", [], [])
next_comp.forward_event = __e  # Stash current event
self.__transition(next_comp)
return
```

The kernel fires `$>` on the target first, then re-routes the stashed
event through the target's dispatcher.

### Transition to Popped State (`-> pop$`)

```python
next_comp = self._state_stack.pop()
self.__transition(next_comp)
return
```

The popped compartment retains all its state variables — no
reinitialization (see [State Stack Operations](#state-stack-operations)).

---

## State Stack Operations

### Push

```python
self._state_stack.append(self.__compartment)
```

The stack entry and `__compartment` point to the **same object**.
`push$` saves a reference, not a copy. Modifications to the
compartment after `push$` are visible through both. For snapshot
semantics, use `push$ -> $State` which creates a new compartment via
the transition.

### Reentry vs History

| Transition Type | State Variable Behavior |
|---|---|
| `-> $State` (normal) | Variables reset to initial values |
| `-> pop$` (history) | Variables preserved from saved compartment |

The `if not in compartment.state_vars` guard in `$>` handlers is what
makes this work. On `pop$` restoration, state_vars are already
populated (from the saved compartment), so the guard skips
re-initialization. On normal entry, state_vars is empty, so the
guard lets initialization run.

---

## Hierarchical State Machines

Frame's HSM model is built on three interlocking commitments:

1. **Uniform compartment construction.** Every compartment (leaf or
   ancestor) is constructed and populated identically by
   `__prepareEnter`. No special cases for ancestors.
2. **Uniform parameter propagation.** State_args, enter_args, and
   exit_args all propagate from the transition site to every layer
   of the chain.
3. **Exact-match signature rule.** `$Child => $Parent` requires
   matching signatures across all three parameter channels.

Together these give `=> $Parent` a complete specialization contract.

### Uniform Parameter Propagation

**Every parameter channel that flows into an HSM state propagates
uniformly across the chain.**

The three parameter channels are:
- `state_args` — declared state parameters, supplied at transition
  via `-> $State(args)`
- `enter_args` — `$>` handler parameters, supplied at transition via
  `-> (args) $State`
- `exit_args` — `<$` handler parameters, supplied at transition via
  `(args) -> $State`

All three:
- Are supplied at the transition site
- Target the leaf state syntactically
- Propagate to every layer in the HSM chain
- Must have signatures that match across `$Child => $Parent` edges

After `__prepareEnter` for a transition to `$Leaf` with chain
`[Root, Middle, Leaf]` and values `state_args=[a, b]`, `enter_args=[x]`:

| Field | Root | Middle | Leaf |
|---|---|---|---|
| `state` | `"Root"` | `"Middle"` | `"Leaf"` |
| `state_args` | `[a, b]` | `[a, b]` | `[a, b]` |
| `enter_args` | `[x]` | `[x]` | `[x]` |
| `exit_args` | `[]` | `[]` | `[]` |
| `state_vars` | `{}` (lazy init) | `{}` (lazy init) | `{}` (lazy init) |
| `parent_compartment` | `None` | `→ Root` | `→ Middle` |

**Each compartment owns its list.** Even though values are the same
across layers, each compartment holds an independent list (Option A
— no shared references). This prevents aliasing bugs if any layer's
handler mutates its received args.

**State_vars are NOT propagated.** State_vars are state-local
declarations, not parameters. Each state's state_vars are private to
that state and initialize independently in that state's `$>` handler.

### The `=> $Parent` Contract

`$Child => $Parent` commits Child and Parent to:

1. **Compartment chain.** Entering `$Child` from outside Parent's
   hierarchy constructs a Parent compartment as the parent of Child's
   compartment (recursively if Parent has its own HSM parent).
2. **Matching state_args signature.** Child and Parent declare the
   same state parameters (names and types).
3. **Matching enter signature.** Child's `$>` signature equals
   Parent's `$>` signature.
4. **Matching exit signature.** Child's `<$` signature equals
   Parent's `<$` signature, or neither state declares `<$`.
5. **Lifecycle cascade.** On entry from outside, Parent's `$>` fires
   before Child's, with the same args. On exit to outside, Parent's
   `<$` fires after Child's, with the same args.
6. **Event forwarding.** Unhandled events in Child forward to Parent
   only via explicit `=> $^` at Child's dispatcher. No implicit
   forwarding.

This is a propagation relationship, not an override relationship.
Both Child's and Parent's handlers run; neither replaces the other.

### Signature-Match Rule (v4)

If `$Child => $Parent`, then `$Child`'s signatures must **equal**
`$Parent`'s signatures:

- **state_args signature:** declared parameters match (names and
  types).
- **enter_args signature:** `$>` parameters match.
- **exit_args signature:** `<$` parameters match, OR neither declares
  `<$`.

Transitively: if `$Grandchild => $Child => $Parent`, all three share
the same signatures across all three channels.

**Rationale.** Uniform propagation populates every layer's args with
the transition's values. For each layer's handler to use them
correctly, each layer's signature must accept those values.
Mismatched signatures would let a handler see args of the wrong shape
— type error at runtime, silent misbehavior in dynamically-typed
backends. Rejecting mismatches at compile time makes propagation
type-safe.

**Future direction.** Prefix-match (Parent's signature is a prefix of
Child's; Child may extend) is deferred to RFC 12 based on v4
experience. Exact-match is the v4 default because it's simpler and
relaxing exact→prefix is non-breaking.

### Parent Forward (`=> $^`)

At the forward site, the child shifts `compartment` one level up to
the parent's own compartment:

```python
def _s_Child_hdl_user_event_b(self, __e, compartment):
    # handler-local work…
    self._state_Parent(__e, compartment.parent_compartment)
```

This is the only place in the pipeline where compartment traversal
happens. Handler methods never walk — they always receive their own
state's compartment ready-to-use.

The chain composes naturally. Grandchild → Child → Parent is
`compartment.parent_compartment` applied once at each forward site.

### Default Forward

A bare `=> $^` declaration at the state level adds a trailing
unguarded forward at the dispatcher tail:

```python
def _state_Child(self, __e, compartment):
    if __e._message == "specific_event":
        self._s_Child_hdl_user_specific_event(__e, compartment); return
    # Trailing forward — emitted only because the state declares `=> $^`
    self._state_Parent(__e, compartment.parent_compartment)
```

**Unhandled events do NOT automatically forward.** A state must
explicitly declare `=> $^` (either at the state level for default
forwarding, or at specific handler sites for selective forwarding)
for unhandled events to reach its parent. Without `=> $^`, unhandled
events simply fall off the dispatcher and are ignored.

This is a deliberate design choice. HSM forwarding is an explicit
capability, not an implicit fallback. A state that is structurally a
Child (declared via `=> $Parent`) does not automatically delegate
unhandled events — it must opt in via `=> $^`.

### Kernel Cascade Semantics

When the kernel processes a transition into a chain
`[Root, Middle, Leaf]`:

**Exit cascade** — fires first, on the source chain, bottom-up:

1. `<$` fires on the source leaf (with its populated `exit_args`).
2. `<$` fires on each ancestor walking up the source chain (each with
   its populated `exit_args`).

**Enter cascade** — fires after compartment switch, on destination
chain, top-down:

1. `$>` fires on Root (with its populated `enter_args`).
2. `$>` fires on Middle (with its populated `enter_args`).
3. `$>` fires on Leaf (with its populated `enter_args`).

All layers in each cascade see their own populated args. Under exact-
match, those args are the same values at every layer; Parent and
Child handlers bind the same typed locals from the same values.

### Transition during cascade

Two rules govern transitions that occur inside a lifecycle handler:

**Rule 1: A transition aborts the remaining cascade.** Once a handler
calls `__transition` during a cascade, no further handlers in the
current chain fire. The kernel finishes the current handler, then
processes the new transition's exit cascade on whatever state is
currently active, followed by the new destination's enter cascade.

**Example:** if `$Root`'s `$>` transitions during an enter cascade
on `[Root, Middle, Leaf]`, Middle's and Leaf's `$>` never fire. The
system entered Root, transitioned, and is now following the new
transition's cascade.

**Rule 2: Event forwarding to a parent requires explicit `=> $^`.**
Even when an HSM relationship exists, unhandled events do not
automatically propagate to ancestors. A state declares `=> $^` either
at the state level (for unconditional forwarding on unhandled events)
or at specific handler sites (for conditional forwarding). Without
`=> $^`, an event that isn't handled by a state's dispatcher is
silently dropped, regardless of whether ancestors could handle it.

These two rules keep HSM control flow explicit and predictable. No
event reaches a handler the source didn't declare intent to reach.

---

## System Context and Return Values

### Accessor Grammar

All `@@` accessors follow a uniform grammar:

- **`:`** (colon) — navigates Frame's namespace hierarchy
- **`.`** (dot) — accesses a field on the resolved object

| Frame Syntax | Runtime Access (Python) |
|---|---|
| `@@:params.x` | `x` — bound at handler prologue from `__e._parameters[N]` |
| `@@:return` | `self._context_stack[-1]._return` |
| `@@:event` | `self._context_stack[-1].event._message` |
| `@@:data.key` | `self._context_stack[-1]._data["key"]` |
| `@@:self.method(args)` | `self.method(args)` (generated interface call) |
| `@@:system.state` | `self.__compartment.state` |

### Handler Parameter Binding

When a handler is invoked, the emitter generates a prologue that
binds each declared parameter to a typed local from the event's
positional parameter vector:

```python
def _s_Active_hdl_user_tick(self, __e, compartment):
    # Prologue: bind declared params from __e._parameters
    step = __e._parameters[0]
    dir  = __e._parameters[1]
    # Handler body references `step` and `dir` as typed locals.
    # @@:params.step in Frame source compiles to bare `step`.
```

Lifecycle handler prologues bind from the compartment's
`enter_args` / `exit_args` instead:

```python
def _s_Active_hdl_frame_enter(self, __e, compartment):
    # Prologue: bind declared enter params from compartment.enter_args
    greeting = compartment.enter_args[0]
    # Handler body
```

### Interface Method Pattern

```python
def get_status(self) -> str:
    __e = FrameEvent("get_status", [])
    __ctx = FrameContext(__e, None)
    self._context_stack.append(__ctx)
    self.__kernel(__e)
    return self._context_stack.pop()._return
```

### Setting Return Value

`@@:return = value` or `@@:(value)` both write the **FrameContext's
return slot** — the field on the FrameContext that the interface
method's wrapper reads after `__kernel` returns. They are the only way
to set it; nothing else writes that slot from Frame source.

```python
# Both compile to:
self._context_stack[-1]._return = value
```

**Note:** `return expr` in handlers is a native return that exits the
dispatch method — the value is NOT set on the FrameContext. The
framepiler emits W415 warning for this pattern.

### Return values across target languages

Whether the wrapper exposes the FrameContext's return slot to the
caller depends on the target language's typing conventions. Frame
matches each target's native idiom rather than imposing one.

**Strongly-typed targets** (TypeScript, Java, Kotlin, Swift, C#,
Dart, C, C++, Go, Rust). The wrapper emits `return …` only when the
Frame source declares a return type:

```frame
get_count(): int { @@:(self.n) }     // wrapper emits `return …;`
log()                                // wrapper has no `return`
```

The declared type also flows into the generated wrapper signature, so
a method declared without `: type` is genuinely `void`. The target
compiler would reject a `return value` statement inside it. This is
Frame respecting the target's compile-time contract.

**Dynamic targets** (Python, JavaScript, Ruby, Lua, PHP, GDScript,
Erlang). The wrapper *always* emits a return that exposes the
FrameContext's return slot, regardless of whether the source declared
a return type. If no handler called `@@:(...)`, the slot still holds
the default (`null`/`None`/`nil`) and the caller receives that.
Source-level `: type` annotations in dynamic targets are documentation
only; the wrapper behavior is the same with or without them.

```frame
// In Python — both forms produce a wrapper that returns the slot:
get_count(): int { @@:(self.n) }     // wrapper returns `_return`
get_count() { @@:(self.n) }          // wrapper still returns `_return`
peek()                               // wrapper returns the default null
```

Why two rules? Strongly-typed targets enforce void-vs-typed at compile
time and would reject a return from a void-declared method. Dynamic
targets always return *something* — there is no void to honor — so the
wrapper always carries the slot to the caller.

### Last Writer Wins

If multiple handlers in a transition chain set `@@:return`, the last
value wins. The context stack's top-of-stack holds the outermost
interface call's return slot; cascade handlers all write to the same
slot.

### Reentrancy

Each interface call pushes its own context:

```
_context_stack: [
    FrameContext(event="outer", _return="outer_value", _data={"k":"v"}),
    FrameContext(event="inner", _return="inner_value", _data={})
]
```

Inner `@@:return` / `@@:data` do not affect outer contexts. Each
layer's `@@` accessors resolve against its own top-of-stack entry.

---

## Self Interface Calls

`@@:self.method(args)` allows a system to call its own interface
methods from within handlers and actions. The transpiler expands this
into a native self-call on the generated interface method.

### Codegen Expansion

| Target | `@@:self.getStatus()` expands to |
|---|---|
| Python | `self.getStatus()` |
| TypeScript | `this.getStatus()` |
| Rust | `self.getStatus()` |
| C | `SystemName_getStatus(self)` |
| C++ | `this->getStatus()` |
| Go | `s.GetStatus()` |
| Java | `this.getStatus()` |

### Dispatch Pipeline

The expansion calls the generated interface method, which follows the
standard pipeline. The self-call pushes a new FrameContext onto the
stack; the outer context is preserved:

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

| Code | Check |
|---|---|
| E601 | Method does not exist in `interface:` block |
| E602 | Argument count mismatch |

These validations are not possible with native self-calls, which the
transpiler treats as opaque native code.

---

## State Variables

### Storage

State variables are stored in `compartment.state_vars`, keyed by
declared name. Because HSM forwards pre-shift the compartment
parameter one level at each `=> $^`, a handler method's `compartment`
parameter is **always** its own state's compartment:

```python
# Inside a handler method for state A (flat or HSM, doesn't matter).
value = compartment.state_vars["counter"]
compartment.state_vars["counter"] = value + 1
```

There is no HSM walk inside handler methods. The walk happens at each
forward site via `compartment.parent_compartment`, so by the time the
parent's handler runs, `compartment` is already pointing at the
parent's own compartment.

### Initialization

State variables initialize inside the lifecycle `$>` handler method,
guarded by `if not in`:

```python
def _s_MyState_hdl_frame_enter(self, __e, compartment):
    if "counter" not in compartment.state_vars:
        compartment.state_vars["counter"] = 0
    if "data" not in compartment.state_vars:
        compartment.state_vars["data"] = {}
```

The guard preserves state-var content on `pop$` restoration — the
initializer runs only when the var doesn't already exist.

### Lifecycle

| Event | Behavior |
|---|---|
| `-> $State` (normal entry) | Variables initialized to declared values |
| `-> pop$` (history entry) | Variables restored from saved compartment |
| Within state | Variables persist until state exits |
| Self-transition (`-> $CurrentState`) | Variables reset (full lifecycle: `<$`, state switch, `$>`) |

### Per-target representation

State_vars storage varies meaningfully between target category:

**Dynamic targets** (Python, JavaScript, Ruby, Lua, PHP, GDScript)
represent state_vars as string-keyed maps. This matches native
idiom — dict, object, hash, table — and integrates naturally with the
target's type system.

**Static targets with type erasure** (Java, Kotlin, C#, Swift, Dart,
TypeScript) use string-keyed maps of the target's "any" type
(`Map<String, Object>`, `[String: Any]`, etc.). State_var access
requires casts; the framepiler inserts them where types are known at
the Frame source level.

**Static targets without type erasure** (Rust, C, C++) use
typed-struct-per-state representations with a tagged union across
states. State_var access is direct field access through generated
accessor methods. This preserves compile-time type safety and matches
the native idiom for these languages.

The three representations are semantically equivalent — same set of
state_vars, same scoping rules, same initialization semantics,
identical canonical serialization under `@@persist`. They differ only
in how they're encoded in the target's type system.

---

## Actions and Operations

### Actions

Private helpers with access to domain fields and context data:

```python
def __my_action(self, param):
    self.domain_var = param        # Can access domain vars
    @@:data.timestamp = now()      # Can access context data
    # CANNOT use: -> $State, push$, pop$, $.varName
```

**Actions have no compartment parameter.** Handler methods receive
`compartment` because each one is statically bound to exactly one
state — the framepiler knows at codegen time which state's
compartment to pass. Actions are called from handlers in any state,
so no single compartment is theirs to receive.

This is why `$.varName` is E401 inside actions: the name has no
static referent the symbol table can resolve. If an action needs a
state variable's value, the caller passes it as a plain argument.

Actions can still reach context data (`@@:data`) because the context
is per-call, not per-state. The call-scoped stack is `self`-attached;
actions inherit it from the calling handler's context.

### Operations

Public methods bypassing the state machine:

```python
def my_operation(self, param) -> int:
    return self.domain_var + param  # Direct domain access, no kernel
```

Operations don't dispatch through the state machine. They can declare
return types using the target language's native syntax. Since the
body is native code, `return value` works as expected.

---

## Persistence

### Canonical state format

A persisted Frame system serializes the following structure:

```
StateBlob {
    frame_version: string
    schema_version: string
    system_name: string
    current_state: string         # leaf state name
    hsm_chain: [string]           # root-to-leaf state names
    compartments: [CompartmentBlob]
    state_stack: [[CompartmentBlob]]
    domain: map<string, Value>
}

CompartmentBlob {
    state: string
    state_args: [Value]
    state_vars: map<string, Value>
    enter_args: [Value]
    exit_args: [Value]
    # forward_event omitted: should be null at save time
    # parent_compartment omitted: reconstructed from static HSM chain
}
```

**Parent compartment pointers are NOT serialized.** They reconstruct
from the static `_HSM_CHAIN` topology table on restore, using the
saved `hsm_chain` list.

**`_context_stack` is NOT serialized.** Context is per-call; a system
in steady state (between interface calls) has an empty context stack.

**`__next_compartment` is NOT serialized.** Should be null at save
time; saving mid-dispatch is not supported.

### What gets serialized

| Component | Description |
|---|---|
| Current HSM chain | Leaf compartment plus each ancestor's compartment |
| State stack | All pushed compartments, each with their own chain |
| Domain variables | All current values |
| Metadata | Frame version, schema version, system name |

Each compartment serializes its full field set: `state`, `state_args`,
`state_vars`, `enter_args`, `exit_args`. All four args/vars channels
must be preserved because they affect what handlers see on restore.

### Restore semantics

Restore does NOT invoke `$>`. The state is being *restored*, not
*entered*.

| Operation | Enter Handler | State Vars |
|---|---|---|
| `-> $State` | Invoked | Reset (then initialized by handler) |
| `-> pop$` | Invoked | Preserved (guard in handler skips init) |
| `restore_state()` | NOT invoked | Restored from save blob |

Restore algorithm:

1. Parse the save blob.
2. Verify `frame_version` and `schema_version` compatibility.
3. Reconstruct each compartment in the HSM chain using
   `_HSM_CHAIN[leaf_state]`, populating fields from the saved
   CompartmentBlobs.
4. Link `parent_compartment` pointers using the reconstructed chain.
5. Reconstruct the state stack similarly.
6. Populate domain fields.
7. System is now in the restored state; the next interface call
   dispatches normally.

### Per-language serialization

Every target's `@@persist` emitter must produce the canonical format
above when cross-host migration is enabled. Native-format fallback
(pickle for Python, native binary for other targets) may be
available for single-host persistence where interop isn't required.

| Language | Canonical codec | Native fallback |
|---|---|---|
| Python | MessagePack or JSON | pickle |
| TypeScript / JavaScript | JSON | JSON |
| Rust | serde_json / rmp-serde | serde_json |
| C | cJSON | cJSON |
| C++ | nlohmann/json | nlohmann/json |
| Java | Jackson | Jackson |
| Kotlin | kotlinx.serialization | kotlinx.serialization |
| Go | encoding/json | encoding/json |
| Swift | JSONEncoder / Codable | JSONEncoder |
| C# | System.Text.Json | System.Text.Json |
| Dart | dart:convert | dart:convert |
| PHP | json_encode | json_encode |
| Ruby | JSON | JSON |
| Lua | manual JSON | manual JSON |
| GDScript | JSON built-in | JSON |
| Erlang | jsx or manual | jsx |

---

## Per-Language Patterns

This section documents how Frame's runtime concepts map to each
target. Use it when writing Frame specs for a specific target — the
native code inside handlers, actions, and epilog must follow the
target's patterns.

### Instantiation

`@@SystemName()` expands to the target language's construction
pattern:

| Language | Declaration + instantiation | Cleanup |
|---|---|---|
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

C is the only backend requiring explicit cleanup via
`System_destroy(s)`. Rust uses ownership semantics — no explicit
destroy needed.

### Interface Method Calls

| Language | Call syntax |
|---|---|
| Python | `s.method(args)` |
| TypeScript/JS | `s.method(args)` |
| Rust | `s.method(args)` |
| C | `System_method(s, args)` |
| C++ | `s->method(args)` or `s.method(args)` |
| Java/Kotlin/C#/Dart | `s.method(args)` |
| Swift | `s.method(args)` |
| Go | `s.Method(args)` (exported) |
| PHP | `$s->method(args)` |
| Ruby | `s.method(args)` |
| Lua | `s:method(args)` |
| GDScript | `s.method(args)` |

### Action and Operation Calls (inside handlers)

| Language | Call syntax |
|---|---|
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

C actions are generated as free functions prefixed with the system
name. All other languages generate them as methods.

### Domain Field Declarations

Domain fields are declared as `name: type` or `name: type = init`.
The type and initializer pass through verbatim to the target:

| Language | Example |
|---|---|
| Python | `count: int = 0` |
| TypeScript | `count: number = 0` |
| Rust | `count: i64 = 0` |
| C | `count: int = 0` (generates `int count;`) |
| C++ | `count: int = 0` |
| Java | `count: int = 0` (generates `int count = 0;`) |
| Kotlin | `count: Int = 0` |
| Go | `count: int = 0` |

**C arrays:** Declare as `name: type` where type includes the array
size. Example: `buffer: char[64]` generates `char buffer[64];`.

**Rust domain fields** become struct fields. Use Rust types directly:
`items: Vec<String> = Vec::new()`.

**Init is optional** for static targets that zero-initialize (C, C++,
Go). Required for dynamic targets where uninitialized fields would
be undefined.

### String Comparison in Native Code

When comparing strings in handler native code, use the target's
string comparison:

| Language | Equality check |
|---|---|
| Python | `s == "value"` |
| TypeScript/JS | `s === "value"` |
| Rust | `s == "value"` |
| C | `strcmp(s, "value") == 0` |
| C++ | `s == "value"` |
| Java | `s.equals("value")` |
| Kotlin | `s == "value"` |
| Go | `s == "value"` |
| Others | `s == "value"` |

### String Interpolation Support

Frame constructs (`$.varName`, `@@:`) work inside string interpolation
expressions for 8 languages:

| Language | Interpolation syntax | Supported |
|---|---|---|
| Python | `f"...{expr}..."` | yes |
| TypeScript/JS | `` `...${expr}...` `` | yes |
| Kotlin/Dart | `"...${expr}..."` | yes |
| C# | `$"...{expr}..."` | yes |
| Ruby | `"...#{expr}..."` | yes |
| Swift | `"...\(expr)..."` | yes |
| C, C++, Java, Go, Lua, Erlang, PHP | n/a | no interpolation syntax |

---

## Runtime Edge Cases

### Stack Underflow

`-> pop$` on an empty state stack is undefined behavior. The
generated code does not check — the pop will fail with a
language-specific error (IndexError in Python, panic in Rust, etc.).
Frame does not guarantee graceful handling.

### Self-Transitions

A transition to the current state (`-> $CurrentState`) fires the full
lifecycle: `<$` (exit), state switch, `$>` (enter). State variables
are reset to their initial values. This is intentional —
self-transitions are a reinitialization mechanism.

### Exceptions in Handlers

If native code throws an exception inside a handler, the pending
transition (if any) does NOT execute. The `__next_compartment` field
is set but the kernel loop never processes it. The context stack
entry is NOT popped — the interface wrapper's `finally` or equivalent
handles cleanup. Behavior varies by target language.

### Context Data Scope

`@@:data` is scoped to the current interface call's context. It
exists for the duration of the dispatch chain (handler → exit cascade
→ transition → enter cascade) and is discarded when the interface
method returns. All handlers within the same dispatch chain share
the same context data. A new interface call creates a fresh context
with empty data.

### HSM Depth

Frame does not cap HSM chain depth. Pathological chains (e.g.,
`$A => $B => $C => ... => $Z`) allocate N compartments per transition
and walk N levels for each cascade. This is the user's responsibility
— Frame emits what the source declares.