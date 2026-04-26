# Frame Runtime Walkthrough

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

A progressive buildup of Frame's runtime, introducing one feature at
a time. Each step adds a single Frame capability and shows which
part of the always-emitted runtime skeleton becomes live as a result.

For the normative specification, see
[Frame Runtime Reference](frame_runtime.md).

**Note on examples:** code snippets in this tutorial are
*illustrative*. They convey structure and intent but may elide
boilerplate. For verified runnable examples, see the conformance
suite.

---

## The central claim

**The runtime skeleton is always fully emitted. Your Frame source
doesn't grow the runtime — it grows the handlers that plug into
machinery already present.**

Even a one-state, one-event system generates the complete FrameEvent
/ FrameContext / Compartment classes, both stacks, the full kernel
transition loop, and all lifecycle-event routing. Per-system
scaffolding is ~95 lines in Python regardless of features used.

What changes as you add features is which parts of the skeleton get
invoked. This tutorial walks through that progression so you can see
each mechanism light up in context.

---

## The three data stores

Before starting, internalize Frame's three storage locations:

| Store | Lifetime | Scope | Accessor |
|---|---|---|---|
| Domain | System lifetime | All handlers, all states | `self.field` |
| State vars | State-instance lifetime | One state's handlers | `$.x` |
| Context data | One dispatch chain | One interface call's handlers | `@@:data.k` |

**Decision rule:** pick the store with the shortest lifetime that
covers the data's scope. Context data for per-call, state vars for
per-state-visit, domain for the whole system.

Steps 1–4 introduce all three, in order of increasing lifetime. By
Step 4 you have a complete data-storage toolkit. Steps 5–9 add
structural mechanisms — transitions, HSM, self-calls, persistence —
but no new stores.

---

## Step 1 — Single state with context data

Start with the minimum viable Frame system: one state, one interface
method, using context data to carry information between the handler
and its actions.

### Frame source

```frame
@@target python_3

@@system Logger {
    interface:
        log(msg: str)

    machine:
        $Active {
            log(msg: str) {
                @@:data.ts = @@:self.now()
                @@:data.formatted = f"[{@@:data.ts}] {msg}"
                print(@@:data.formatted)
            }
        }

    actions:
        now(): int { return @@:self.get_time() }
}
```

### What's live in this step

**Everything is present. Only a subset is invoked:**

| Skeleton piece | Status in Step 1 |
|---|---|
| `FrameEvent` class | Emitted; used for the `log` event |
| `FrameContext` class | Emitted; pushed on `log()` call; `_data` used |
| `Compartment` class | Emitted; one compartment for `$Active` |
| `_state_stack` | Initialized to `[]`; never appended |
| `_context_stack` | Used for this call's context |
| `__next_compartment` | Initialized to `None`; never set (no transitions) |
| `__kernel` `while` loop | Body never executes (no pending transitions) |
| `__router`, state dispatcher, handler method | All used |

### Why `@@:data` from the start

`@@:data` is a dynamic, string-keyed map in every target. It's the
scratch store for one interface call — born when the call starts,
discarded when the call returns.

**Why use it instead of local variables?** Two reasons that will
become clearer in later steps:

1. `@@:data` is reachable by actions called from the handler. The
   `now()` action reads `@@:data` without needing the handler to pass
   it as a parameter.
2. `@@:data` persists across lifecycle events within one dispatch
   chain. A handler that transitions can set `@@:data.k`, and the
   target state's `$>` will see the same value (Step 5 will show
   this).

### Dynamic in every target

`@@:data` has no declared schema. In dynamically-typed targets
(Python, JavaScript, Ruby) this is invisible. In statically-typed
targets (Java, Rust, Go, etc.) the store is a string-keyed map of
"any" type (`Map<String, Object>`, `HashMap<String, Box<dyn Any>>`,
etc.) — reads may require a cast at the use site.

This is intentional. `@@:data`'s scope spans an arbitrary subset of
a system's handlers and actions — whichever get reached during one
dispatch. A declared schema would need to be the union of every
key any callable might set, which devolves to "dynamic map with
types tracked externally." Frame represents this honestly.

### Generated code excerpt

```python
class Logger:
    def __init__(self):
        self.__compartment = LoggerCompartment("Active")
        self.__next_compartment = None
        self._state_stack = []
        self._context_stack = []
        # Fire the start state's $>
        __frame_event = LoggerFrameEvent("$>", self.__compartment.enter_args)
        self.__kernel(__frame_event)

    def log(self, msg: str):
        __e = LoggerFrameEvent("log", [msg])
        __ctx = LoggerFrameContext(__e, None)
        self._context_stack.append(__ctx)
        self.__kernel(__e)
        self._context_stack.pop()

    def _state_Active(self, __e, compartment):
        if __e._message == "$>":
            self._s_Active_hdl_frame_enter(__e, compartment); return
        if __e._message == "log":
            self._s_Active_hdl_user_log(__e, compartment); return

    def _s_Active_hdl_frame_enter(self, __e, compartment):
        pass

    def _s_Active_hdl_user_log(self, __e, compartment):
        msg = __e._parameters[0]
        self._context_stack[-1]._data["ts"] = self.now()
        self._context_stack[-1]._data["formatted"] = \
            f"[{self._context_stack[-1]._data['ts']}] {msg}"
        print(self._context_stack[-1]._data["formatted"])

    def now(self):
        return self.get_time()  # native code
```

Notice: `@@:data.ts` compiled to `self._context_stack[-1]._data["ts"]`.
The action `now()` reads the same dict via the same path.

---

## Step 2 — Adding a return value

Add a method that returns data. This activates the `_return` slot on
FrameContext.

### Frame source

```frame
@@system Logger {
    interface:
        log(msg: str): str
        get_ts(): int

    machine:
        $Active {
            log(msg: str): str {
                @@:data.ts = @@:self.now()
                @@:data.formatted = f"[{@@:data.ts}] {msg}"
                print(@@:data.formatted)
                @@:(@@:data.formatted)    // return the formatted string
            }

            get_ts(): int {
                @@:(@@:self.now())
            }
        }

    actions:
        now(): int { return @@:self.get_time() }
}
```

### What's newly live

| Skeleton piece | Newly used |
|---|---|
| `FrameContext._return` field | Written by `@@:(...)`; read by interface wrapper |

Everything else from Step 1 stays the same. The runtime didn't grow;
handlers now write to a slot that was already there.

### The context grew one field, not a new class

FrameContext from Step 1 was already this:

```python
class FrameContext:
    def __init__(self, event, return_val):
        self.event = event
        self._return = return_val    # was None; now used
        self._data = {}
```

In Step 1, `_return` was set to None and never referenced. In Step 2,
handlers write to it via `@@:return` or `@@:(...)`, and the
interface wrapper reads it before popping the context.

### Generated code excerpt

```python
def log(self, msg: str) -> str:
    __e = LoggerFrameEvent("log", [msg])
    __ctx = LoggerFrameContext(__e, None)
    self._context_stack.append(__ctx)
    self.__kernel(__e)
    return self._context_stack.pop()._return   # <-- new

def _s_Active_hdl_user_log(self, __e, compartment):
    msg = __e._parameters[0]
    self._context_stack[-1]._data["ts"] = self.now()
    self._context_stack[-1]._data["formatted"] = \
        f"[{self._context_stack[-1]._data['ts']}] {msg}"
    print(self._context_stack[-1]._data["formatted"])
    self._context_stack[-1]._return = \
        self._context_stack[-1]._data["formatted"]   # <-- new: @@:(...)
```

The `_return` slot existed in Step 1's generated code; Step 2 just
writes to it.

### Return values across target languages

`@@:return = expr` and `@@:(expr)` both set the FrameContext's return
slot. Whether the **wrapper** exposes that slot to the caller depends
on the target language's typing conventions.

**Strongly-typed targets** (TypeScript, Java, Kotlin, Swift, C#, Dart,
C, C++, Go, Rust): the wrapper emits `return …` only when the source
declares a return type. Without `: type` the method is genuinely
`void` and the target compiler would reject a value-returning
statement.

**Dynamic targets** (Python, JavaScript, Ruby, Lua, PHP, GDScript,
Erlang): the wrapper *always* exposes the return slot. Even
`get_ts()` (no `: type`) compiles to a wrapper that returns whatever
`@@:(…)` left in the slot, or the default if no handler set it.

The Logger above runs the same way in any target: callers get a
return value where one was set. The Python source happens to declare
`(): int` and `(): str` for documentation, but those annotations are
not what makes the wrapper return — in Python they wouldn't be
needed. Only strongly-typed targets *require* them.

See [Frame Runtime Reference — Return values across target
languages](frame_runtime.md#return-values-across-target-languages)
for the cross-target table and the reasoning behind the split.

---

## Step 3 — Adding domain fields

Add system-wide state that persists across all calls. This introduces
the second data store.

### Frame source

```frame
@@system Logger {
    interface:
        log(msg: str): str
        total(): int

    machine:
        $Active {
            log(msg: str): str {
                @@:data.ts = @@:self.now()
                self.count = self.count + 1
                @@:data.formatted = f"[{@@:data.ts}] #{self.count}: {msg}"
                print(@@:data.formatted)
                @@:(@@:data.formatted)
            }

            total(): int {
                @@:(self.count)
            }
        }

    actions:
        now(): int { return @@:self.get_time() }

    domain:
        count: int = 0
}
```

### What's newly live

**Nothing in the runtime skeleton.** Domain is plain `self.x` — a
native instance field. It doesn't involve the compartment, the
context stack, or any Frame-specific machinery.

| Store | Where it lives |
|---|---|
| Context data (`@@:data`) | `self._context_stack[-1]._data` |
| State vars (`$.x`, coming in Step 4) | `compartment.state_vars` |
| Domain (`self.x`) | `self` directly |

### Why this matters pedagogically

Domain is the one data store Frame doesn't mediate. It's a plain
instance field in every target — Python attribute, Rust struct field,
Java instance variable. You reach it with the target's normal syntax.

This clarifies Frame's philosophy: Frame provides stores for *state-
scoped* and *call-scoped* data (where lifetime management requires
framework awareness) but gets out of the way for *system-scoped* data
(where the target language already has a perfectly good mechanism).

### Generated code excerpt

```python
class Logger:
    def __init__(self):
        self.count = 0                    # <-- domain init
        self.__compartment = LoggerCompartment("Active")
        # ... rest of scaffolding
```

That's the entirety of domain's runtime contribution: one line in
the constructor.

---

## Step 4 — Second state and state variables

Add a second state and transitions between them. State vars (the
third data store) become available.

### Frame source

```frame
@@system Logger {
    interface:
        log(msg: str): str
        pause()
        resume()
        total(): int

    machine:
        $Active {
            $.accepted: int = 0

            log(msg: str): str {
                @@:data.ts = @@:self.now()
                $.accepted = $.accepted + 1
                self.count = self.count + 1
                @@:data.formatted = f"[{@@:data.ts}] #{self.count} (session: {$.accepted}): {msg}"
                print(@@:data.formatted)
                @@:(@@:data.formatted)
            }

            pause() { -> $Paused }
            total(): int { @@:(self.count) }
        }

        $Paused {
            $.dropped: int = 0

            log(msg: str): str {
                $.dropped = $.dropped + 1
                @@:("dropped")
            }

            resume() { -> $Active }
            total(): int { @@:(self.count) }
        }

    actions:
        now(): int { return @@:self.get_time() }

    domain:
        count: int = 0
}
```

### What's newly live

This step activates the biggest chunk of previously-dormant runtime:

| Skeleton piece | Newly used |
|---|---|
| `__next_compartment` field | Assigned by `__transition()` |
| `__kernel` `while` loop | Body executes when transition pending |
| `__prepareEnter` helper | Constructs target compartment |
| Kernel exit cascade | Fires `<$` on old state |
| Kernel enter cascade | Fires `$>` on new state |
| `Compartment.state_vars` | Written by `$.x` assignments |
| Lifecycle event routing | `$>` / `<$` dispatched to handlers |
| Second state dispatcher | `_state_Paused` generated and routed to |
| Router's second branch | `if state == "Paused"` added |

### The payoff — all three stores visible

A single handler in `$Active` now touches all three stores:

```frame
log(msg: str): str {
    @@:data.ts = @@:self.now()      // context data — this call only
    $.accepted = $.accepted + 1      // state var — this session in Active
    self.count = self.count + 1      // domain — lifetime of system
    ...
}
```

The contrast is the point:

- `@@:data.ts` — gone the instant `log()` returns.
- `$.accepted` — resets when `$Active` exits (via `pause()`) and
  re-enters (via `resume()`). Invisible in `$Paused`.
- `self.count` — survives transitions, accessible from either state.

### State var initialization is lazy

`$.accepted: int = 0` generates initialization in the `$>` handler:

```python
def _s_Active_hdl_frame_enter(self, __e, compartment):
    if "accepted" not in compartment.state_vars:
        compartment.state_vars["accepted"] = 0
```

The `if not in` guard looks redundant now but matters for Step 7
(`push$` / `pop$`), where restored state_vars must survive
re-initialization.

### Generated code excerpt

```python
def _state_Active(self, __e, compartment):
    if __e._message == "$>":
        self._s_Active_hdl_frame_enter(__e, compartment); return
    if __e._message == "log":
        self._s_Active_hdl_user_log(__e, compartment); return
    if __e._message == "pause":
        self._s_Active_hdl_user_pause(__e, compartment); return
    # ...

def _s_Active_hdl_user_pause(self, __e, compartment):
    next_comp = self.__prepareEnter("Paused", [], [])
    self.__transition(next_comp)
    return

def __prepareEnter(self, leaf, state_args, enter_args):
    comp = None
    for name in _HSM_CHAIN[leaf]:
        comp = LoggerCompartment(name, comp)
        comp.state_args = list(state_args)
        comp.enter_args = list(enter_args)
    return comp
```

### The recap

You've now seen all three data stores. The rest of this tutorial
adds structural mechanisms — enter/exit args, HSM, push/pop, self-
calls, persistence — but no new stores. Everything else builds on
these three.

---

## Step 5 — Enter and exit args

Pass data into lifecycle handlers at transition time.

### Frame source

```frame
@@system Logger {
    interface:
        log(msg: str): str
        pause(reason: str)
        resume()

    machine:
        $Active {
            $.accepted: int = 0

            log(msg: str): str { /* ... */ }

            pause(reason: str) {
                (reason) -> $Paused
            }
        }

        $Paused {
            $.reason: str = "unknown"

            $>(reason: str) {
                $.reason = reason
                print(f"paused: {reason}")
            }

            <$() { print("resuming") }

            resume() { -> $Active }
        }

    domain:
        count: int = 0
}
```

### What's newly live

| Skeleton piece | Newly used |
|---|---|
| `Compartment.enter_args` | Populated by `__prepareEnter`'s third parameter |
| `Compartment.exit_args` | Populated by `__prepareExit` (if exit args used) |
| `$>` handler parameter binding | Prologue reads from `compartment.enter_args` |
| `<$` handler parameter binding | Prologue reads from `compartment.exit_args` |

### Three parameter channels

Frame gives transitions three parameter channels:

| Channel | Syntax | Goes to |
|---|---|---|
| `state_args` | `-> $State(args)` | Target state's declared parameters |
| `enter_args` | `-> (args) $State` | Target state's `$>` handler |
| `exit_args` | `(args) -> $State` | Source state's `<$` handler |

They're orthogonal — a single transition can use any combination:

```frame
("cleanup") -> ("hello") $Target("initial_value")
// exit_args: ["cleanup"]
// enter_args: ["hello"]
// state_args: ["initial_value"]
```

### Generated code excerpt

```python
def _s_Active_hdl_user_pause(self, __e, compartment):
    reason = __e._parameters[0]
    next_comp = self.__prepareEnter("Paused", [], [reason])  # enter_args
    self.__transition(next_comp)
    return

def _s_Paused_hdl_frame_enter(self, __e, compartment):
    reason = compartment.enter_args[0]    # <-- bound from enter_args
    if "reason" not in compartment.state_vars:
        compartment.state_vars["reason"] = "unknown"
    compartment.state_vars["reason"] = reason
    print(f"paused: {reason}")
```

### `@@:data` persists across the dispatch chain

A subtle property becomes visible here: **context data set before a
transition is still available in the target state's `$>`.**

```frame
log(msg: str) {
    @@:data.audit_id = "abc123"
    if needs_pause {
        ("audit") -> $Paused
    }
}

$Paused.<$(reason: str) {
    // @@:data.audit_id is still "abc123" here
    audit_log(@@:data.audit_id, reason)
}
```

The context stack holds the outermost interface call's context for
the entire dispatch chain (handler → exit cascade → transition →
enter cascade). All lifecycle handlers triggered by one interface
call share the same `@@:data`. This is what makes `@@:data` useful —
it's the only store that spans transitions within a dispatch.

---

## Step 6 — Hierarchical state machines

Add an HSM relationship between two states.

### Frame source

```frame
@@system SessionManager {
    interface:
        activate(user_id: str)
        ping()
        disconnect(reason: str)

    machine:
        $Idle {
            activate(user_id: str) {
                -> $LoggedIn(user_id)
            }
        }

        $LoggedIn(user_id: str) {
            $.login_time: int = 0

            $>() {
                $.login_time = @@:self.now()
                print(f"session started for {user_id}")
            }

            <$() { print("session ended") }

            disconnect(reason: str) {
                print(f"disconnect: {reason}")
                -> $Idle
            }
        }

        $Authenticated(user_id: str) => $LoggedIn {
            $.permissions: list = []

            $>() {
                $.permissions = @@:self.load_perms(user_id)
                print(f"authenticated")
            }

            <$() { print("auth cleared") }

            ping() {
                print("authenticated ping")
            }
        }

    actions:
        now(): int { return @@:self.get_time() }
        load_perms(uid): list { return [] }
}
```

Note that `$Authenticated => $LoggedIn` requires matching signatures:
- state_args: both declare `(user_id: str)`
- enter_args: both declare empty `$>()`
- exit_args: both declare empty `<$()`

### What's newly live

| Skeleton piece | Newly used |
|---|---|
| `Compartment.parent_compartment` | Populated by `__prepareEnter` for HSM layers |
| `_HSM_CHAIN` topology table | Consulted to build multi-layer chains |
| Multi-layer kernel cascade | `$>` fires top-down, `<$` bottom-up |
| Uniform parameter propagation | Same args populated at every layer |

### The full `=> $Parent` contract

Declaring `$Authenticated => $LoggedIn` commits to six things:

1. **Compartment chain.** Entering `$Authenticated` constructs a
   LoggedIn compartment as the parent of Authenticated's compartment.
2. **Matching state_args signature.** Both declare `(user_id: str)`.
3. **Matching enter signature.** Both declare `$>()`.
4. **Matching exit signature.** Both declare `<$()`.
5. **Lifecycle cascade.** LoggedIn's `$>` fires before
   Authenticated's; LoggedIn's `<$` fires after Authenticated's.
6. **Event forwarding.** Unhandled events in Authenticated forward
   to LoggedIn only via explicit `=> $^`.

This is a propagation relationship. Both handlers run; neither
replaces the other. On entry to `$Authenticated("alice")`:

```
LoggedIn.$>  runs with user_id = "alice"
Authenticated.$>  runs with user_id = "alice"
```

Two handlers, each doing its own initialization, with the same args.

### Uniform parameter propagation

When transitioning `-> $Authenticated("alice")`, the transition's
`state_args` populate *every layer* of the chain:

```python
next_comp = self.__prepareEnter("Authenticated", ["alice"], [])

# Result:
#   loggedin_comp.state_args = ["alice"]
#   authenticated_comp.state_args = ["alice"]
#   authenticated_comp.parent_compartment → loggedin_comp
```

Each layer owns its own list (no shared references). Both `$>`
handlers see `user_id = "alice"`.

### Explicit forwarding via `=> $^`

Note that `ping()` in `$Authenticated` doesn't automatically forward
unhandled events to `$LoggedIn`. If `$LoggedIn` had a `ping()`
handler and `$Authenticated` wanted to delegate to it, Authenticated
would need to declare it:

```frame
$Authenticated => $LoggedIn {
    => $^   // <-- explicit: unhandled events forward to parent
}
```

Without `=> $^`, unhandled events are silently dropped. The HSM
relationship doesn't imply automatic event delegation; it only
establishes the structural chain.

### Generated code excerpt

```python
# Emitted once per system:
_HSM_CHAIN = {
    "Idle": ["Idle"],
    "LoggedIn": ["LoggedIn"],
    "Authenticated": ["LoggedIn", "Authenticated"],
}

def _s_Idle_hdl_user_activate(self, __e, compartment):
    user_id = __e._parameters[0]
    next_comp = self.__prepareEnter("LoggedIn", [user_id], [])
    self.__transition(next_comp)
    return

# Kernel's enter cascade fires top-down:
#   _state_LoggedIn._s_LoggedIn_hdl_frame_enter    <-- first
#   (Authenticated is not in this chain, skipped)

# For a transition to Authenticated:
def activate_auth(self, user_id):
    next_comp = self.__prepareEnter("Authenticated", [user_id], [])
    # next_comp is the Authenticated compartment
    # next_comp.parent_compartment is the LoggedIn compartment
    # Both have state_args = [user_id]
    self.__transition(next_comp)

# Enter cascade fires:
#   _s_LoggedIn_hdl_frame_enter      <-- runs first (top-down)
#   _s_Authenticated_hdl_frame_enter <-- runs second
```

---

## Step 7 — State stack (push$ / pop$)

Save and restore states, preserving state_vars.

### Frame source

```frame
@@system Workflow {
    interface:
        start()
        interrupt(reason: str)
        resume_from_interrupt()

    machine:
        $Idle {
            start() { -> $Working }
        }

        $Working {
            $.progress: int = 0

            interrupt(reason: str) {
                push$
                -> $Interrupted(reason)
            }

            complete() { -> $Idle }
        }

        $Interrupted(reason: str) {
            resume_from_interrupt() {
                -> pop$    // restores $Working with $.progress preserved
            }
        }
}
```

### What's newly live

| Skeleton piece | Newly used |
|---|---|
| `_state_stack` | Appended by `push$`, popped by `-> pop$` |
| `if not in` guard in `$>` handlers | Now meaningful — skips re-init on pop$ restore |

### The saved compartment retains state_vars

When `-> pop$` restores a compartment:

```python
def _s_Interrupted_hdl_user_resume(self, __e, compartment):
    next_comp = self._state_stack.pop()
    # next_comp is the saved Working compartment
    # Its state_vars still contain $.progress from before
    self.__transition(next_comp)
```

The kernel fires `$Interrupted.<$`, switches to the popped
compartment, fires `$Working.$>`. Working's `$>` runs:

```python
def _s_Working_hdl_frame_enter(self, __e, compartment):
    if "progress" not in compartment.state_vars:   # <-- key line
        compartment.state_vars["progress"] = 0
```

Because the restored compartment's `state_vars["progress"]` is
already populated (from before the push), the guard skips
re-initialization. `$.progress` retains its old value.

This is why the `if not in` guard was added in Step 4. It looked
redundant there because there was no mechanism to restore existing
state_vars. Step 7 activates the guard's purpose.

### Reference vs copy semantics

```frame
push$
-> $Interrupted(reason)    // immediate transition
```

`push$` alone saves a *reference* to the current compartment. If the
handler keeps running after `push$` (and modifies state_vars), the
stack entry sees those modifications — it's the same object.

`push$ -> $State` creates a new compartment via the transition; the
pushed reference points at the old compartment, which is now off the
active chain. This is the common case and gives snapshot semantics.

---

## Step 8 — Self-calls and reentrancy

Call the system's own interface methods from within a handler.

### Frame source

```frame
@@system Sensor {
    interface:
        calibrate(): bool
        reading(): int

    machine:
        $Active {
            $.offset: int = 0
            $.sensor_value: int = 0

            calibrate(): bool {
                baseline = @@:self.reading()
                $.offset = baseline * -1
                @@:(true)
            }

            reading(): int {
                @@:($.sensor_value + $.offset)
            }
        }
}
```

### What's newly live

| Skeleton piece | Newly used |
|---|---|
| `_context_stack` depth > 1 | First time the stack has multiple entries |
| Nested context isolation | Inner `@@:return` doesn't touch outer |

Until this step, `_context_stack` always had exactly one entry —
pushed by the interface wrapper, popped when it returned. Self-calls
are why the stack is a stack.

### Context isolation during self-call

```python
def _s_Active_hdl_user_calibrate(self, __e, compartment):
    # _context_stack at this point: [ctx_calibrate]
    baseline = self.reading()
    # Inside reading():
    #   _context_stack: [ctx_calibrate, ctx_reading]
    #   @@:event in reading is "reading" (from ctx_reading)
    #   @@:return in reading refers to ctx_reading._return
    #   After reading() returns, ctx_reading is popped.
    # _context_stack: [ctx_calibrate]
    compartment.state_vars["offset"] = baseline * -1
    self._context_stack[-1]._return = True
```

Each self-call pushes its own context. The calling handler's context
is preserved. When the self-call returns, its context is popped and
the original handler's context is top-of-stack again.

`@@:return`, `@@:data`, `@@:event` all resolve against the top of
the context stack. During a self-call, these point at the inner
call's context; after return, they point back at the outer.

### Self-calls dispatch through the normal pipeline

`@@:self.reading()` is expanded to a normal interface method call:

```python
baseline = self.reading()   # not a direct handler call
```

The generated `reading()` method constructs a FrameEvent, pushes a
FrameContext, runs the kernel, dispatches to
`_s_Active_hdl_user_reading`, pops the context, returns `_return`.

This means self-calls can:
- Trigger transitions (if the called handler transitions, the
  current state changes)
- Fire lifecycle cascades (if the self-call causes a transition,
  `<$` and `$>` fire)
- Recurse (self-calls can call themselves, subject to normal stack
  depth limits)

### Validation at compile time

The framepiler validates self-calls:

| Code | Check |
|---|---|
| E601 | Method doesn't exist in `interface:` block |
| E602 | Argument count mismatch |

Self-calls are checked against the declared interface, not against
handler implementations. This catches typos and refactoring mistakes
before runtime.

---

## Step 9 — Persistence

Save and restore a system's complete state.

### Frame source

```frame
@@system Counter {
    @@persist

    interface:
        tick()
        get_count(): int

    machine:
        $Active {
            $.session_ticks: int = 0

            tick() {
                $.session_ticks = $.session_ticks + 1
                self.total = self.total + 1
            }

            get_count(): int { @@:(self.total) }
        }

    domain:
        total: int = 0
}
```

### What's newly live

Nothing new in the runtime machinery. Persistence serializes data
structures that already exist.

| Data Store | Serialized? |
|---|---|
| Domain (`self.total`) | Yes |
| State vars (`compartment.state_vars`) | Yes — per layer |
| Context data (`@@:data`) | **No** (per-call; not persistent) |
| State stack | Yes — each saved compartment |
| Current HSM chain | Yes — each layer's compartment |
| `parent_compartment` links | **No** (reconstructed from `_HSM_CHAIN`) |
| `_context_stack` | **No** (empty at steady state) |
| `__next_compartment` | **No** (null at steady state) |

### The payoff — runtime is complete by Step 8

By Step 8, every piece of Frame's runtime machinery is live. Step 9
adds nothing new; it just serializes the structures that steps 1–8
established.

That's the structural claim this walkthrough set out to make: the
runtime is complete as a fixed skeleton; your source invokes parts
of it. Persistence closes the loop — the runtime's state is entirely
capturable, because every piece of machinery is either persistent
(serialized) or transient (discarded by design).

### Canonical serialization schema

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
}
```

Every backend produces this same structure (in MessagePack or JSON)
when `@@persist cross_host` is enabled. The schema is Frame's
invariant, not any backend's.

### Restore does not fire `$>`

Critical semantic: restoring state is not entering it.

| Operation | Enter Handler | State Vars |
|---|---|---|
| `-> $State` (normal) | Invoked | Reset (then init by handler) |
| `-> pop$` | Invoked | Preserved (guard skips init) |
| `restore_state()` | **NOT invoked** | Restored from save blob |

On restore, the system reconstructs compartments from the save blob,
populates their fields, and is immediately ready to receive the next
interface call. No lifecycle events fire. The state machine picks up
exactly where the save happened.

### Cross-host migration

If the save blob is produced by a system compiled in one target
language and the restore is performed by a system compiled in another
target language, the protocol works identically — provided both
sides were compiled from the same Frame source and use the canonical
MessagePack format.

This is the migration scenario: a state machine that moves between
hosts, preserving its logical state. It's the most severe test of
Frame's functional-equivalence commitment. If the demo works, the
runtime is correct. See RFC 15b (migration protocol) and RFC 16
(conformance testing) for the formal specification and verification
regime.

---

## Recap

| Step | Added feature | Runtime piece activated |
|---|---|---|
| 1 | `@@:data` | `FrameContext._data` used |
| 2 | Return values | `FrameContext._return` used |
| 3 | Domain fields | (plain `self.x`) |
| 4 | States + transitions | Kernel transition loop, cascades |
| 5 | Enter/exit args | `enter_args` / `exit_args` populated |
| 6 | HSM | `parent_compartment`, multi-layer cascade |
| 7 | push$ / pop$ | `_state_stack` used |
| 8 | Self-calls | `_context_stack` depth > 1 |
| 9 | Persistence | Serialization of existing structures |

Each step adds Frame source, not runtime machinery. The runtime
skeleton is a fixed commitment the framepiler makes on your behalf;
the features you use determine which parts get invoked.

## Where to go next

- [Frame Runtime Reference](frame_runtime.md) — the normative
  specification of everything introduced here, plus the parts this
  tutorial didn't cover.
- [Frame Language Reference](frame_language.md) — the Frame source
  language itself.
- [Frame Cookbook](frame_cookbook.md) — worked examples showing
  Frame patterns in practice.
- RFC 11, RFC 15a, RFC 15b, RFC 16 — design documents for the
  architectural commitments this tutorial introduces.