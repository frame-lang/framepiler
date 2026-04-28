# The Frame Runtime, by example

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

This document explains Frame's runtime by building up a system one
feature at a time. We start with the simplest possible Frame source
and add capability incrementally. Each step shows what changes — in
the source and in the generated code — and explains the runtime
mechanism that change activates.

The example throughout is a lamp. It starts as a system that can
only be turned on, and grows from there.

---

## Step 1 — A system that accepts a call

The simplest Frame system has no states and one interface method:

```frame
@@system Lamp {
    interface:
        turn_on()
}
```

A system called `Lamp` that exposes one method. No machine block,
no states, no handlers.

Here's what's generated:

```python
class LampFrameEvent:
    def __init__(self, message, parameters):
        self._message = message
        self._parameters = parameters

class LampFrameContext:
    def __init__(self, event):
        self.event = event

class Lamp:
    def __init__(self):
        self._context_stack = []

    def turn_on(self):
        __e = LampFrameEvent("turn_on", [])
        __ctx = LampFrameContext(__e)
        self._context_stack.append(__ctx)
        self.__kernel(__e)
        self._context_stack.pop()

    def __kernel(self, __e):
        pass
```

When someone calls `lamp.turn_on()`, the wrapper packages the call
as a **FrameEvent** — a message name and a list of parameters. It
then builds a **FrameContext** that holds the event, pushes it onto
the context stack, hands the event to the kernel, and pops the
context when the kernel returns.

The kernel is empty. There's nothing to dispatch to. The runtime is
set up to route events to handlers, but no handlers exist yet.

> **In statically-typed targets** — `_parameters` becomes a typed
> list whose element type is the target's "any" — `List<Object>`
> in Java, `Vec<Box<dyn Any>>` in Rust, `[Any]` in Swift,
> `List<object>` in C#. The wrapper packs heterogeneous parameter
> values into the list and the handler unpacks them with casts at
> the binding site (covered in Step 9). FrameEvent and FrameContext
> are emitted as proper classes with typed fields rather than
> Python's free-form attribute assignment.

The next step gives the system a state, and the kernel gets work
to do.

---

## Step 2 — Adding a state

Now the lamp has a state:

```frame
@@system Lamp {
    interface:
        turn_on()

    machine:
        **$Operating {}**
}
```

One state called `$Operating`, no handlers in it. The interface
method `turn_on()` still has nowhere to go, but the runtime now
has a state to track.

The runtime needs a record of the active state, so a Compartment
class shows up:

```python
class LampCompartment:
    def __init__(self, state):
        self.state = state
```

A compartment is just a record of which state the system is in.
Right now it holds nothing but the state's name; later steps will
give it more fields as features that need them appear. The system
points at the current compartment through a new field:

```python
def __init__(self):
    self._context_stack = []
    self.__compartment = LampCompartment("Operating")
```

`__compartment` is `$Operating`, the only state available.

The kernel can't stay empty if there's a state to dispatch to.
Events arrive via the kernel, so the kernel hands them to a router
that knows how to find the right state:

```python
def __kernel(self, __e):
    self.__router(__e)

def __router(self, __e):
    state_name = self.__compartment.state
    if state_name == "Operating":
        self._state_Operating(__e, self.__compartment)

def _state_Operating(self, __e, compartment):
    pass
```

The router reads the current state's name and calls that state's
dispatcher — one branch per state in a static if/elif chain. Each
state has its own dispatcher function that would match an event's
message to a handler, but `$Operating` has no handlers, so its
dispatcher is empty. When `turn_on()` arrives, the dispatcher gets
called and returns immediately.

The full path now: caller invokes `turn_on()` → wrapper builds the
event and context → kernel calls the router → router calls the
state dispatcher → dispatcher does nothing → everything unwinds.

---

## Step 3 — Adding a handler

```frame
@@system Lamp {
    interface:
        turn_on()

    machine:
        $Operating {
            **turn_on() {
                print("lamp is on")
            }**
        }
}
```

The state declares a handler for `turn_on()`. The dispatcher,
which was empty in Step 2, gains a branch:

```python
def _state_Operating(self, __e, compartment):
    # dispatcher
    if __e._message == "turn_on":
        self._s_Operating_hdl_user_turn_on(__e, compartment); return
```

The dispatcher checks the event's message. If it matches
`"turn_on"`, it calls the handler and returns. And the handler
itself appears:

```python
def _s_Operating_hdl_user_turn_on(self, __e, compartment):
    print("lamp is on")
```

A handler is the unique piece of code that runs for one specific
event in one specific state. `$Operating` has its own handler for
`turn_on`. If another state also declared a `turn_on` handler,
that would be a separate method — different state, different
handler. The state-event pair identifies exactly one handler.

The handler name has four parts:

- `_s` — marks it as runtime-generated state code.
- `Operating` — the state this handler belongs to.
- `hdl_user` — the kind of handler. `user` means it's for an
  interface method.
- `turn_on` — the event being handled.

Pattern: `_s_<State>_hdl_<kind>_<event>`.

The single underscore prefix avoids Python's name mangling —
`__name` gets rewritten to `_ClassName__name` inside classes,
which would break the predictable naming. Other targets use the
same scheme so generated code looks the same everywhere.

Each handler is a separate method rather than inlined into the
dispatcher. Stack traces land on the specific handler that ran.

---

## Step 4 — Adding a second state

```frame
@@system Lamp {
    interface:
        turn_on()

    machine:
        $Off {
            turn_on() {
            }
        }

        **$On {}**
}
```

A second state, `$On`. Empty for now — no handlers, no way to get
there. The `turn_on` handler in `$Off` does nothing.

The first state declared in the machine block is the **start
state**. The system enters it when constructed and stays there
until a transition moves it somewhere else. `$Off` is the start
state because it's listed first.

`$On` exists in the source and gets generated, but nothing ever
reaches it. There's no transition to `$On` anywhere, so the system
enters `$Off` and stays there forever. This is legal but useless.

The router needs to know about both states, so it gains a branch
for `$On`:

```python
def __router(self, __e):
    state_name = self.__compartment.state
    if state_name == "Off":
        self._state_Off(__e, self.__compartment)
    **elif state_name == "On":
        self._state_On(__e, self.__compartment)**
```

One branch per state — the router checks the current state's name
and calls its dispatcher. `$On` gets its own dispatcher, empty for
now since the state has no handlers:

```python
**def _state_On(self, __e, compartment):
    pass**
```

The runtime is set up to dispatch to `$On` if the system ever gets
there. It just never does.

---

## Step 5 — Adding a transition

```frame
@@system Lamp {
    interface:
        turn_on()

    machine:
        $Off {
            turn_on() {
                **-> $On**
            }
        }

        $On {}
}
```

`turn_on` now transitions to `$On`. The lamp can finally reach its
other state.

The system needs to track that a transition is pending, so the
constructor gains a field:

```python
def __init__(self):
    self._context_stack = []
    self.__compartment = LampCompartment("Off")
    **self.__next_compartment = None**
```

`__next_compartment` holds the destination of a transition that's
been requested but not yet processed. It's `None` when no
transition is pending.

The handler doesn't switch states directly — it queues the switch:

```python
def _s_Off_hdl_user_turn_on(self, __e, compartment):
    **next_comp = LampCompartment("On")
    self.__transition(next_comp)
    return**
```

It builds a compartment for the destination, hands it to
`__transition`, and returns. The transition doesn't happen here.
`__transition` is small — it just caches the destination:

```python
**def __transition(self, next_compartment):
    self.__next_compartment = next_compartment**
```

The actual state switch happens in the kernel, which now checks
for a pending transition after the router returns:

```python
def __kernel(self, __e):
    self.__router(__e)

    **# __next_compartment is set if a transition occurred
    # during the router call.
    if self.__next_compartment is not None:
        next_compartment = self.__next_compartment
        self.__next_compartment = None
        self.__compartment = next_compartment**
```

If a transition was queued during the router call, the kernel
pulls the destination out of `__next_compartment`, clears the
field, and switches the system's current compartment.

This is Frame's **deferred transition** model. Handlers don't
switch states; they queue a switch. The kernel does the actual
switching after the handler finishes. This avoids problems where a
handler ends up running in the wrong state because it switched
mid-way through, and gives the kernel a single place to manage the
lifecycle of state changes.

The full path when `turn_on()` is called from `$Off`: the wrapper
builds the event and pushes a context; the kernel calls the
router; the router calls `_state_Off`; `_state_Off` calls
`_s_Off_hdl_user_turn_on`; the handler builds an `$On` compartment
and calls `__transition` then returns; the router returns; the
kernel sees `__next_compartment` is set and switches
`__compartment` to the new one; the wrapper pops the context.

The system is now in `$On`. Calling `turn_on()` again does
nothing — `$On` has no handler for it.

---

## Step 6 — Lifecycle handlers

```frame
@@system Lamp {
    interface:
        turn_on()

    machine:
        $Off {
            **<$() {
                print("lamp going on")
            }**
            turn_on() {
                **-> "switch flipped" $On**
            }
        }

        $On {
            **$>() {
                print("lamp is on")
            }**
        }
}
```

Three additions:

- `$Off` declares an exit handler `<$()`.
- `$On` declares an enter handler `$>()`.
- The transition is now decorated with a label, `"switch flipped"`.

Decorated transition labels are diagnostic only — they show up in
generated diagrams and traces but don't affect runtime behavior.
The label is the only source change that doesn't activate any
runtime code.

Lifecycle handlers do activate runtime code. When the lamp
transitions from `$Off` to `$On`, the runtime fires `$Off`'s `<$`
handler, then switches the compartment, then fires `$On`'s `$>`
handler. The kernel grows to handle this — instead of just
swapping compartments, it now synthesizes lifecycle events around
the switch:

```python
def __kernel(self, __e):
    self.__router(__e)

    # __next_compartment is set if a transition occurred
    # during the router call.
    if self.__next_compartment is not None:
        next_compartment = self.__next_compartment
        self.__next_compartment = None

        **# Fire <$ on the current state
        exit_event = LampFrameEvent("<$", [])
        self.__router(exit_event)**

        self.__compartment = next_compartment

        **# Fire $> on the new state
        enter_event = LampFrameEvent("$>", [])
        self.__router(enter_event)**
```

The kernel synthesizes two FrameEvents during a transition. First
it builds a `<$` event and routes it — this reaches the *current*
compartment, which is still the source state, so the source
state's exit handler runs. Then the kernel switches `__compartment`
to the new one. Then it builds a `$>` event and routes it — this
reaches the new compartment, so the destination state's enter
handler runs.

Lifecycle events go through the same router as user events.
There's no special path for them. The dispatcher matches their
messages (`"$>"`, `"<$"`) the same way it matches `"turn_on"`, so
each state's dispatcher gains branches for the lifecycle messages
its state declares:

```python
def _state_Off(self, __e, compartment):
    **if __e._message == "<$":
        self._s_Off_hdl_frame_exit(__e, compartment); return**
    if __e._message == "turn_on":
        self._s_Off_hdl_user_turn_on(__e, compartment); return

def _state_On(self, __e, compartment):
    **if __e._message == "$>":
        self._s_On_hdl_frame_enter(__e, compartment); return**
```

Each state's dispatcher matches the lifecycle messages and routes
to the right handler. The handler methods themselves are
straightforward:

```python
**def _s_Off_hdl_frame_exit(self, __e, compartment):
    print("lamp going on")

def _s_On_hdl_frame_enter(self, __e, compartment):
    print("lamp is on")**
```

The `frame` part of the name is what we deferred explaining in
Step 3. Earlier we saw the four-part naming pattern
`_s_<State>_hdl_<kind>_<event>`. The `kind` is `user` for
interface methods and `frame` for lifecycle handlers (`$>` becomes
`frame_enter`, `<$` becomes `frame_exit`).

The split keeps the namespaces disjoint. A user could declare an
interface method named `enter` and it would generate
`_s_<State>_hdl_user_enter` — a different method from
`_s_<State>_hdl_frame_enter`. No collision possible.

The start state's `$>` handler also needs to run when the system
is first constructed — entering the start state is itself a state
entry — so the constructor fires it. Like every other entry into
the kernel, the firing goes through a wrapper that pushes a
FrameContext, runs the router, and pops:

```python
def __init__(self):
    self._context_stack = []
    self.__compartment = LampCompartment("Off")
    self.__next_compartment = None

    **# Fire $> on the start state
    enter_event = LampFrameEvent("$>", [])
    enter_ctx = LampFrameContext(enter_event)
    self._context_stack.append(enter_ctx)
    self.__router(enter_event)
    self._context_stack.pop()**
```

The push/pop is the same pattern interface method wrappers use.
Any handler that runs during the cascade — including the start
state's `$>` — needs a context on the stack so that `@@:return`,
`@@:data`, and other context-stack reads have something to
resolve against. The context pushed here is discarded after the
constructor returns; there's no caller to give the return value
to. But the handler runs without crashing on an empty stack.

The lamp doesn't currently declare `$Off.$>`, so this routes
through the dispatcher and finds no match, doing nothing. But the
mechanism is there for any state that has an enter handler.

Calling `turn_on()` now exercises the full lifecycle pipeline.
The wrapper builds the event and pushes a context, then the
kernel calls the router which calls `_state_Off`'s dispatcher,
which calls the `turn_on` handler. The handler queues a
transition to `$On` and returns. The router returns, and the
kernel sees `__next_compartment` is set. It synthesizes a `<$`
event and calls the router again — this reaches `_state_Off`
(still the current state), whose dispatcher matches `<$` and
calls `_s_Off_hdl_frame_exit`, which prints "lamp going on". The
kernel then switches `__compartment` to `$On` and synthesizes a
`$>` event. The router calls `_state_On` (the new current state),
whose dispatcher matches `$>` and calls
`_s_On_hdl_frame_enter`, which prints "lamp is on". Finally the
wrapper pops the context.

Two prints, in order: "lamp going on", "lamp is on". The lamp has
gone from `$Off` to `$On` and run code on both sides of the
transition.

---

## Step 7 — Domain field

```frame
@@system Lamp {
    interface:
        turn_on()

    machine:
        $Off {
            <$() {
                print("lamp going on")
            }
            turn_on() {
                -> "switch flipped" $On
            }
        }

        $On {
            $>() {
                **self.cycles = self.cycles + 1
                print(f"lamp is on (cycle {self.cycles})")**
            }
        }

    **domain:
        cycles: int = 0**
}
```

A `domain` block declares system-wide instance fields. `cycles`
counts how many times the lamp has been turned on, surviving every
state transition.

Domain fields are accessed with `self.x` — just normal target
language attribute access. Frame doesn't mediate domain reads or
writes the way it does state variables or context data. The
constructor gains a line to initialize the field:

```python
def __init__(self):
    **self.cycles = 0**
    self._context_stack = []
    self.__compartment = LampCompartment("Off")
    self.__next_compartment = None

    enter_event = LampFrameEvent("$>", [])
    enter_ctx = LampFrameContext(enter_event)
    self._context_stack.append(enter_ctx)
    self.__router(enter_event)
    self._context_stack.pop()
```

Domain initializers go before everything else in the constructor
so their values are available if any startup code references them.

The handler body in `$On.$>` reads and writes `self.cycles` as
ordinary Python. There's no special accessor pattern. Domain
fields are the simplest of Frame's three data stores — they live
on the system instance directly.

The other two stores (state variables and context data) require
runtime support to manage their lifetimes. State variables live in
a compartment's `state_vars` dict and reset each time the state is
entered. Context data lives in a FrameContext's `_data` dict and
exists only for the duration of one interface call. We'll see both
of those in later steps.

For now, the lamp tracks its on-count across all calls to
`turn_on()`. Calling `turn_on()` four times prints
`(cycle 1)` ... `(cycle 4)`, then ignores further calls because
`$On` has no handler for `turn_on`.

---

## Step 8 — Return value

```frame
@@system Lamp {
    interface:
        turn_on()
        **is_on(): bool**

    machine:
        $Off {
            <$() {
                print("lamp going on")
            }
            turn_on() {
                -> "switch flipped" $On
            }
            **is_on(): bool {
                @@:(false)
            }**
        }

        $On {
            $>() {
                self.cycles = self.cycles + 1
                print(f"lamp is on (cycle {self.cycles})")
            }
            **is_on(): bool {
                @@:(true)
            }**
        }

    domain:
        cycles: int = 0
}
```

`is_on(): bool` returns whether the lamp is currently on. Each
state has its own handler — `$Off.is_on` returns `false`, `$On.is_on`
returns `true`. The state-event pair determines which handler runs,
which determines the answer.

`@@:(expr)` sets the return value. It's shorthand for
`@@:return = expr`. Both compile to the same generated code.

The return value needs somewhere to live during the call, so
FrameContext gains a slot for it:

```python
class LampFrameContext:
    def __init__(self, event):
        self.event = event
        **self._return = None**
```

`_return` is where the return value lives during an interface
call. The wrapper reads it after the kernel returns. The wrapper
for `is_on` declares a return type and reads the slot at the end:

```python
**def is_on(self) -> bool:
    __e = LampFrameEvent("is_on", [])
    __ctx = LampFrameContext(__e)
    self._context_stack.append(__ctx)
    self.__kernel(__e)
    return self._context_stack.pop()._return**
```

It's the same shape as `turn_on`'s wrapper, plus a final line that
returns `_return` from the popped context. The Python return type
annotation comes from the Frame source.

The dispatchers gain branches for `is_on`:

```python
def _state_Off(self, __e, compartment):
    if __e._message == "<$":
        self._s_Off_hdl_frame_exit(__e, compartment); return
    if __e._message == "turn_on":
        self._s_Off_hdl_user_turn_on(__e, compartment); return
    **if __e._message == "is_on":
        self._s_Off_hdl_user_is_on(__e, compartment); return**

def _state_On(self, __e, compartment):
    if __e._message == "$>":
        self._s_On_hdl_frame_enter(__e, compartment); return
    **if __e._message == "is_on":
        self._s_On_hdl_user_is_on(__e, compartment); return**
```

And each state has its own `is_on` handler:

```python
**def _s_Off_hdl_user_is_on(self, __e, compartment):
    self._context_stack[-1]._return = False

def _s_On_hdl_user_is_on(self, __e, compartment):
    self._context_stack[-1]._return = True**
```

`@@:(false)` compiles to `self._context_stack[-1]._return = False`.
The handler writes to the top-of-stack context's `_return` slot.
The wrapper reads the same slot when the kernel returns.

When `is_on()` is called from `$On`, the wrapper builds the event
and pushes a context. The kernel calls the router, which calls
`_state_On`. The dispatcher matches `is_on` and calls
`_s_On_hdl_user_is_on`, which sets `_return` to `True`. The
router returns; the kernel returns (no transition queued); the
wrapper pops the context and returns `_return`.

The caller gets `True`. The same flow with the lamp in `$Off`
returns `False`. Same interface call, different result based on
state — the basic value of state machines.

> **In statically-typed targets** — The wrapper's behavior depends
> on whether the source declares a return type. With a return
> type (`is_on(): bool`), the wrapper emits `return …_return` —
> same as Python. Without one (`log()`), the wrapper has no
> `return` statement and the method is genuinely `void`.
>
> This means `@@:return` or `@@:(...)` inside a void-declared
> method is meaningless — the wrapper has nowhere to read the
> slot. The framepiler rejects this at compile time rather than
> emitting code the target compiler would reject. (Returning a
> value from a void method is a type error in Java, Rust, Go,
> Swift, C#, Kotlin, Dart, TypeScript, C, and C++.)
>
> Dynamic targets — Python, JavaScript, Ruby, Lua, PHP, GDScript,
> Erlang — don't make the void-vs-typed distinction at runtime.
> Their wrappers always read and return the slot regardless of
> whether the source declared a return type. A method declared
> without `: type` in Python that uses `@@:(42)` will return 42 to
> the caller; the same source compiled for Java would be rejected
> by the framepiler.
>
> The asymmetry is deliberate. Frame matches each target's native
> conventions rather than imposing a single uniform rule across
> all 17 backends. Strongly-typed targets enforce void-vs-typed at
> compile time; Frame respects that. Dynamic targets always return
> *something* (`undefined`, `None`, `nil`); the wrapper carrying
> the slot is the natural idiom.

### Default return values

What if a state doesn't declare a handler for an interface method
that has a return type? Right now `is_on()` is handled in both
states, but if `$Off` had no `is_on` handler, the dispatcher would
fall through and `_return` would still be `None` when the wrapper
read it. The caller would get `None` for a method declared to
return `bool`.

Frame solves this with **default return values** in the interface
declaration:

```frame
interface:
    turn_on()
    **is_on(): bool = false**
```

The `= false` after the return type is the default. If no handler
sets `@@:return`, the wrapper returns this value. The FrameContext
constructor takes the default and initializes `_return` to it
rather than `None`:

```python
class LampFrameContext:
    def __init__(self, event**, return_default**):
        self.event = event
        self._return = **return_default**
```

The wrapper for each method passes its declared default in:

```python
def is_on(self) -> bool:
    __e = LampFrameEvent("is_on", [])
    __ctx = LampFrameContext(__e**, False**)
    self._context_stack.append(__ctx)
    self.__kernel(__e)
    return self._context_stack.pop()._return
```

The wrapper for each interface method passes the appropriate
default. For `turn_on()` (no return type), there's no default —
the FrameContext just gets `None` and the wrapper doesn't read
`_return` at all.

If a handler sets `@@:return = expr`, that overwrites the default.
If no handler sets it, the wrapper returns whatever the default
was. From the caller's point of view, every interface method with
a return type produces a value — never `None` or undefined.

Defaults are useful when some handlers compute a return value and
others want to fall through with a sensible "nothing to report"
value. A method declared `call(): str = "error"` lets handlers
that don't explicitly set `@@:return` produce `"error"` automatically —
useful for "operation rejected" or "not applicable in this state"
semantics.

A method declared with a return type but no default gets the
target language's null/zero/empty value as the implicit default
(`None` in Python, `null` in JavaScript, the zero value in Go,
etc.). Explicit defaults are recommended in static targets where
the implicit default may not be what you want.

### Native `return` vs `@@:`

Native `return` in a handler exits the dispatch method but does
not set `_return`. The framepiler emits warning W415 when a
handler uses `return expr` because the value is almost certainly
meant to be the interface return value. To set the return value,
use `@@:return = expr` or `@@:(expr)`. Bare `return` (with no
expression) exits the handler early without affecting the return
value.

### Async

Interface methods can be declared `async`. Whether and how that
propagates through the wrapper depends on the target language —
Python uses `async def` and `await`, JavaScript returns a Promise,
and other targets handle it differently. The runtime structure
described here doesn't change; the wrapper, kernel, and dispatch
layers work the same. For target-specific async behavior, see the
[Frame Language Reference](frame_language.md).

---

## Step 9 — Interface method parameters

```frame
@@system Lamp {
    interface:
        **turn_on(brightness: int)**
        is_on(): bool

    machine:
        $Off {
            <$() {
                print("lamp going on")
            }
            **turn_on(brightness: int) {
                print(f"requested brightness: {brightness}")
                -> "switch flipped" $On
            }**
            is_on(): bool {
                @@:(false)
            }
        }

        $On {
            $>() {
                self.cycles = self.cycles + 1
                print(f"lamp is on (cycle {self.cycles})")
            }
            is_on(): bool {
                @@:(true)
            }
        }

    domain:
        cycles: int = 0
}
```

`turn_on` now takes a `brightness` parameter. The handler reads
it and prints it before transitioning.

The wrapper accepts the parameter and packs it into the
FrameEvent:

```python
def turn_on(self, **brightness: int**):
    __e = LampFrameEvent("turn_on", **[brightness]**)
    __ctx = LampFrameContext(__e)
    self._context_stack.append(__ctx)
    self.__kernel(__e)
    self._context_stack.pop()
```

`_parameters` was always a list. With no parameters declared, the
list was empty. Now it holds the value the caller passed in. The
handler binds the parameter to a local at the top of its body:

```python
def _s_Off_hdl_user_turn_on(self, __e, compartment):
    **brightness = __e._parameters[0]**
    print(f"requested brightness: {brightness}")
    next_comp = LampCompartment("On")
    self.__transition(next_comp)
    return
```

Each parameter is read from `__e._parameters` by
position — `_parameters[0]` for the first declared parameter,
`_parameters[1]` for the second, and so on. After binding, the
handler body uses the parameters as ordinary locals.

> **In statically-typed targets** — `_parameters[0]` is typed
> "any" (`Object` in Java, `Box<dyn Any>` in Rust, `Any` in
> Swift), so binding requires a cast to the declared parameter
> type. The framepiler emits the cast based on the source's type
> annotation:
>
> ```java
> int brightness = (Integer) __e._parameters.get(0);
> ```
>
> ```rust
> let brightness: i32 = *__e._parameters[0].downcast_ref::<i32>().unwrap();
> ```
>
> Same positional access pattern, just with the cast appropriate
> to the target. Authors don't write the casts; the framepiler
> generates them from the parameter's declared type. Because
> Frame source declares all parameter types, every cast is known
> at code-generation time — there's no dynamic type checking at
> runtime.

### Positional, with named access in source

`_parameters` is a positional list at the wire level. Frame source
lets you access parameters by name (`brightness` in the handler
body, or `@@:params.brightness` if you prefer the explicit form),
but that named access is a compile-time rewrite. The framepiler
binds each parameter to a typed local at the top of the handler
and rewrites named references to use that local.

This is the same mechanism every statically-typed language uses
for function parameters: positional at the calling convention,
named at the source. The wire format is positional; the source is
named; the framepiler bridges them.

### `@@:params.x`

`@@:params.x` is the explicit form of named parameter access. In a
handler body it's interchangeable with the bare parameter name —
both compile to the same local. It's most useful in actions, which
don't have parameters of their own but can read the calling
handler's parameters via the context stack. We'll see actions in
Step 13.

---

## Step 10 — State arguments

```frame
@@system Lamp {
    interface:
        turn_on(brightness: int)
        is_on(): bool
        **get_brightness(): int**

    machine:
        $Off {
            <$() {
                print("lamp going on")
            }
            turn_on(brightness: int) {
                **-> "switch flipped" $On(brightness)**
            }
            is_on(): bool {
                @@:(false)
            }
            **get_brightness(): int {
                @@:(0)
            }**
        }

        **$On(brightness: int) {**
            $>() {
                self.cycles = self.cycles + 1
                **print(f"lamp is on at brightness {brightness} (cycle {self.cycles})")**
            }
            is_on(): bool {
                @@:(true)
            }
            **get_brightness(): int {
                @@:(brightness)
            }**
        }

    domain:
        cycles: int = 0
}
```

`$On` now declares a state parameter `brightness: int`. The
transition `-> $On(brightness)` passes the value at transition
time. Handlers in `$On` can reference `brightness` as a local.

A new interface method, `get_brightness()`, returns the current
brightness — `0` from `$Off` (no state arg), the actual value from
`$On`.

Compartments need to carry state args, so the Compartment class
gains a field:

```python
class LampCompartment:
    def __init__(self, state):
        self.state = state
        **self.state_args = []**
```

`state_args` is a positional list, same shape as
`FrameEvent._parameters`. It holds the values passed by the
transition. The transition itself populates `state_args` on the
new compartment before queueing it:

```python
def _s_Off_hdl_user_turn_on(self, __e, compartment):
    brightness = __e._parameters[0]
    next_comp = LampCompartment("On")
    **next_comp.state_args = [brightness]**
    self.__transition(next_comp)
    return
```

The handler builds the new compartment, then populates its
`state_args` before calling `__transition`. The kernel doesn't
need to know about state args specifically — it just switches
compartments. The args travel with the compartment.

Handlers in `$On` bind state args at the top, the same way they
bind event parameters:

```python
def _s_On_hdl_frame_enter(self, __e, compartment):
    **brightness = compartment.state_args[0]**
    self.cycles = self.cycles + 1
    print(f"lamp is on at brightness {brightness} (cycle {self.cycles})")

def _s_On_hdl_user_get_brightness(self, __e, compartment):
    **brightness = compartment.state_args[0]**
    self._context_stack[-1]._return = brightness
```

Every handler in `$On` reads `brightness` from
`compartment.state_args[0]` — same prologue pattern as event
parameters, just reading from a different list.

State args persist for the lifetime of the compartment. They're
set once when the compartment is built and read by every handler
that runs in that state. This is different from event parameters,
which arrive with each individual call.

When `turn_on(75)` is called from `$Off`, the wrapper builds the
event with `_parameters = [75]`. The handler binds `brightness =
75`, builds an `$On` compartment with `state_args = [75]`, calls
`__transition` and returns. The kernel fires `<$` on `$Off`,
switches compartment, fires `$>` on `$On`. `$On.$>` binds
`brightness = 75` from the compartment and prints "lamp is on at
brightness 75". Calling `get_brightness()` afterward reads the
same `state_args` slot and returns `75`. The brightness is part
of the state's identity until the lamp transitions away.

---

## Step 11 — Enter args

```frame
@@system Lamp {
    interface:
        turn_on(brightness: int)
        is_on(): bool
        get_brightness(): int

    machine:
        $Off {
            <$() {
                print("lamp going on")
            }
            turn_on(brightness: int) {
                **-> "switch flipped" ("hello") $On(brightness)**
            }
            is_on(): bool {
                @@:(false)
            }
            get_brightness(): int {
                @@:(0)
            }
        }

        $On(brightness: int) {
            **$>(greeting: str) {**
                self.cycles = self.cycles + 1
                **print(f"{greeting} — lamp is on at brightness {brightness} (cycle {self.cycles})")**
            }
            is_on(): bool {
                @@:(true)
            }
            get_brightness(): int {
                @@:(brightness)
            }
        }

    domain:
        cycles: int = 0
}
```

The transition `-> ("hello") $On(brightness)` now also supplies an
**enter arg**. `$On.$>` declares a parameter `greeting: str` to
receive it.

Enter args travel to the destination state's `$>` handler. They're
distinct from state args, which any handler in the state can read.
Enter args are a one-time payload for the entry itself.

The transition syntax now distinguishes three argument groups:

| Position | Goes to | Channel |
|---|---|---|
| `(args) -> ...` | Source state's `<$` handler | exit args |
| `... -> (args) $State` | Destination state's `$>` handler | enter args |
| `... $State(args)` | Destination compartment's state args | state args |

Step 11 introduces enter args; Step 12 will introduce exit args.

The Compartment class gains another field for enter args:

```python
class LampCompartment:
    def __init__(self, state):
        self.state = state
        self.state_args = []
        **self.enter_args = []**
```

The handler populates `enter_args` alongside `state_args` before
queueing the transition:

```python
def _s_Off_hdl_user_turn_on(self, __e, compartment):
    brightness = __e._parameters[0]
    next_comp = LampCompartment("On")
    next_comp.state_args = [brightness]
    **next_comp.enter_args = ["hello"]**
    self.__transition(next_comp)
    return
```

Same pattern as state args, just a different field. The kernel
needs to pass these along to the `$>` handler, so it now reads
`enter_args` off the new compartment and packs them into the
synthesized `$>` event:

```python
def __kernel(self, __e):
    self.__router(__e)

    if self.__next_compartment is not None:
        next_compartment = self.__next_compartment
        self.__next_compartment = None

        exit_event = LampFrameEvent("<$", [])
        self.__router(exit_event)

        self.__compartment = next_compartment

        **enter_event = LampFrameEvent("$>", next_compartment.enter_args)**
        self.__router(enter_event)
```

From the `$>` handler's point of view, the enter args look like
ordinary event parameters. The handler binds them at the top:

```python
def _s_On_hdl_frame_enter(self, __e, compartment):
    **greeting = __e._parameters[0]**
    brightness = compartment.state_args[0]
    self.cycles = self.cycles + 1
    print(f"{greeting} — lamp is on at brightness {brightness} (cycle {self.cycles})")
```

`greeting` is bound from `__e._parameters[0]` — the FrameEvent the
kernel built. `brightness` is bound from `compartment.state_args[0]`
— still on the compartment.

Enter args and state args travel through different paths. Enter
args ride on the synthesized `$>` event. State args stay on the
compartment. But once they're bound to locals, the handler body
doesn't care which is which.

---

## Step 12 — Exit args

```frame
@@system Lamp {
    interface:
        turn_on(brightness: int)
        **turn_off(reason: str)**
        is_on(): bool
        get_brightness(): int

    machine:
        $Off {
            **$>(last_reason: str) {
                print(f"lamp went dark: {last_reason}")
            }**
            <$() {
                print("lamp going on")
            }
            turn_on(brightness: int) {
                -> "switch flipped" ("hello") $On(brightness)
            }
            is_on(): bool {
                @@:(false)
            }
            get_brightness(): int {
                @@:(0)
            }
        }

        $On(brightness: int) {
            $>(greeting: str) {
                self.cycles = self.cycles + 1
                print(f"{greeting} — lamp is on at brightness {brightness} (cycle {self.cycles})")
            }
            **<$(reason: str) {
                print(f"turning off: {reason}")
            }**
            **turn_off(reason: str) {
                (reason) -> "switch flipped" (reason) $Off
            }**
            is_on(): bool {
                @@:(true)
            }
            get_brightness(): int {
                @@:(brightness)
            }
        }

    domain:
        cycles: int = 0
}
```

A new interface method, `turn_off(reason: str)`, lets the lamp
turn off with a reason. The transition
`(reason) -> (reason) $Off` carries `reason` to both
`$On.<$` (as exit arg) and `$Off.$>` (as enter arg).

`$Off.$>` is also new — until now `$Off` had no enter handler.
With exit args flowing into `$Off`, it makes sense to have an
enter handler that uses them.

Compartments need a third arg field for exit args:

```python
class LampCompartment:
    def __init__(self, state):
        self.state = state
        self.state_args = []
        self.enter_args = []
        **self.exit_args = []**
```

The transition populates `exit_args` on the *current*
compartment, not the next one — the exit handler that runs is the
source state's, so its compartment is where the args belong:

```python
def _s_On_hdl_user_turn_off(self, __e, compartment):
    reason = __e._parameters[0]
    **compartment.exit_args = [reason]**
    next_comp = LampCompartment("Off")
    next_comp.enter_args = [reason]
    self.__transition(next_comp)
    return
```

The handler sets `compartment.exit_args` and the destination
compartment's `enter_args` separately, even though both hold the
same value here. They're independent channels — a transition could
pass different values to each.

The kernel reads `exit_args` off the current compartment when
synthesizing `<$`, and `enter_args` off the destination when
synthesizing `$>`:

```python
def __kernel(self, __e):
    self.__router(__e)

    if self.__next_compartment is not None:
        next_compartment = self.__next_compartment
        self.__next_compartment = None

        **exit_event = LampFrameEvent("<$", self.__compartment.exit_args)**
        self.__router(exit_event)

        self.__compartment = next_compartment

        enter_event = LampFrameEvent("$>", next_compartment.enter_args)
        self.__router(enter_event)
```

The `<$` handler binds its exit args from `__e._parameters`, the
same prologue pattern user handlers and `$>` handlers both use:

```python
def _s_On_hdl_frame_exit(self, __e, compartment):
    **reason = __e._parameters[0]**
    print(f"turning off: {reason}")
```

When `turn_off("bedtime")` is called from `$On`, the wrapper
builds the event with `_parameters = ["bedtime"]`. The handler
binds `reason = "bedtime"`, sets `compartment.exit_args =
["bedtime"]` on the current `$On` compartment, builds the `$Off`
compartment with `enter_args = ["bedtime"]`, calls `__transition`,
and returns. The kernel synthesizes a `<$` event with
`_parameters = ["bedtime"]` and routes it; `$On.<$` runs and
prints "turning off: bedtime". The kernel then switches the
compartment to `$Off` and synthesizes `$>` with `_parameters =
["bedtime"]`; `$Off.$>` runs and prints "lamp went dark:
bedtime".

Three argument channels are now active: state args, enter args,
exit args. They're independent — a transition can use any
combination.

#### Argument-receiver contract

Each of the three arg channels has a strict-match contract enforced
by framec at compile time:

| Site             | Receiver                       | Code  |
|------------------|--------------------------------|-------|
| `(args) -> $T`   | source state's `<$(...)`       | E419  |
| `-> (args) $T`   | target state's `$>(...)`       | E417  |
| `-> $T(args)`    | target state's state params    | E405  |

The receiver must exist if the transition supplies args, and the
caller's count must fit the receiver's declared signature. For
EventParam-backed receivers (E417, E419), trailing `default_value`
declarations relax the lower bound — `<$(a, b = "x")` accepts 1
or 2 supplied args, with the default filling in for `b` when
omitted. StateParam doesn't carry defaults today, so `-> $T(args)`
requires an exact count match.

The check fires only when the transition supplies args. A
transition `-> $T` against a state with `<$(a, b)` is allowed —
the handler then runs with its params unbound, which is a
runtime concern, not a structural error. The strict-match
direction is "if the caller provides, the receiver must accept",
not "if the receiver exists, the caller must provide".

---

## Step 13 — Actions

```frame
@@system Lamp {
    interface:
        turn_on(brightness: int)
        turn_off(reason: str)
        is_on(): bool
        get_brightness(): int

    machine:
        $Off {
            $>(last_reason: str) {
                **self.log_event(f"off: {last_reason}")**
            }
            <$() {
                **self.log_event("turning on")**
            }
            turn_on(brightness: int) {
                -> "switch flipped" ("hello") $On(brightness)
            }
            is_on(): bool {
                @@:(false)
            }
            get_brightness(): int {
                @@:(0)
            }
        }

        $On(brightness: int) {
            $>(greeting: str) {
                self.cycles = self.cycles + 1
                **self.log_event(f"on at {brightness}: {greeting}")**
            }
            <$(reason: str) {
                **self.log_event(f"turning off: {reason}")**
            }
            turn_off(reason: str) {
                (reason) -> "switch flipped" (reason) $Off
            }
            is_on(): bool {
                @@:(true)
            }
            get_brightness(): int {
                @@:(brightness)
            }
        }

    **actions:
        log_event(msg: str) {
            print(f"[event] {msg}")
            self.event_count = self.event_count + 1
        }**

    domain:
        cycles: int = 0
        **event_count: int = 0**
}
```

The lamp now has an **action**: `log_event(msg)`. Actions are
private helper methods. Handlers call them when the same code
needs to run from multiple places — every lifecycle handler in the
lamp now logs through `log_event` rather than printing inline.

The action becomes a regular method on the system class:

```python
**def log_event(self, msg: str):
    print(f"[event] {msg}")
    self.event_count = self.event_count + 1**
```

No special wrapping, no context stack, no kernel involvement —
just code that handlers can call. The handlers call it directly:

```python
def _s_On_hdl_frame_enter(self, __e, compartment):
    greeting = __e._parameters[0]
    brightness = compartment.state_args[0]
    self.cycles = self.cycles + 1
    **self.log_event(f"on at {brightness}: {greeting}")**
```

`log_event(...)` in Frame source compiles to `self.log_event(...)`
in Python. Same as any method call.

### What actions can and can't do

Actions can:

- Read and write domain fields (`self.event_count`)
- Call `@@:return`, `@@:params`, `@@:event`, `@@:data` (we'll see
  these in Step 15)
- Call other actions and operations
- Call `@@:self.method()` to invoke interface methods

Actions cannot use any of Frame's state-machine syntax:

- No `-> $State` — actions can't transition.
- No `push$` or `pop$` — actions can't manipulate the state stack.
- No `$.varName` — actions can't access state variables.

The last restriction is structural. State variables live on a
specific compartment. Handlers receive their compartment as a
parameter (we'll see why in later steps). Actions don't — they're
called from handlers in any state, so there's no single
compartment that's theirs to receive. `$.varName` has nothing to
resolve against in an action, so the framepiler rejects it (E401).

If an action needs a value that only a handler has access to, the
handler passes it as an argument. For state variables (which we
haven't seen yet — they're introduced in Step 18), this is the
standard pattern:

```
handler reads its state var → passes value to action → action uses it
```

Actions read domain fields directly (`self.x`) and context data
through `@@:data`. State variables are the one store actions can't
reach without help.

### `@@:event`

Inside an action, `@@:event` evaluates to the name of the
interface method that's currently being processed. It's read
through the context stack:

```python
@@:event   →   self._context_stack[-1].event._message
```

Useful when an action needs to know which interface method called
it — for diagnostic logging, conditional behavior, etc.

---

## Step 14 — Operations

```frame
@@system Lamp {
    **operations:
        static version(): str {
            return "1.0.0"
        }

        get_event_count(): int {
            return self.event_count
        }**

    interface:
        turn_on(brightness: int)
        turn_off(reason: str)
        is_on(): bool
        get_brightness(): int

    machine:
        // ... (unchanged)

    actions:
        log_event(msg: str) {
            print(f"[event] {msg}")
            self.event_count = self.event_count + 1
        }

    domain:
        cycles: int = 0
        event_count: int = 0
}
```

**Operations** are public methods that bypass the state machine
entirely. They don't dispatch through the kernel, don't push a
context, and don't go through any state's dispatcher. They're just
plain methods on the system class.

The lamp adds two operations:

- `version()` — a static method that returns the lamp's version
  string. Useful for diagnostic queries.
- `get_event_count()` — a non-static method that returns the
  current event count from the domain.

The operations compile straight to methods on the system class:

```python
**@staticmethod
def version() -> str:
    return "1.0.0"

def get_event_count(self) -> int:
    return self.event_count**
```

Static operations get `@staticmethod` (or the target's equivalent)
and don't take `self`. Non-static operations take `self` and can
access domain fields directly. The body uses native `return` —
operations bypass the state machine, so `@@:return` doesn't apply.

### Operations vs interface methods

Both are public, but they differ in what they can do:

| | Interface method | Operation |
|---|---|---|
| Dispatches through kernel | Yes | No |
| Behavior depends on state | Yes | No |
| Can transition | Yes (in handler) | No |
| Can access state vars | Yes (in handler) | No |
| Has FrameContext | Yes | No |
| Uses `@@:return` | Yes | No (native return) |

Operations are the right choice for diagnostics, configuration
queries, and utilities that don't depend on which state the system
is in. Interface methods are the right choice for events the
state machine should react to.

---

## Step 15 — Context data

```frame
@@system Lamp {
    operations:
        static version(): str {
            return "1.0.0"
        }
        get_event_count(): int {
            return self.event_count
        }

    interface:
        turn_on(brightness: int)
        turn_off(reason: str)
        is_on(): bool
        get_brightness(): int

    machine:
        $Off {
            $>(last_reason: str) {
                **@@:data.timestamp = self.timestamp_now()
                self.log_event(f"off: {last_reason}")**
            }
            <$() {
                **@@:data.timestamp = self.timestamp_now()**
                self.log_event("turning on")
            }
            turn_on(brightness: int) {
                -> "switch flipped" ("hello") $On(brightness)
            }
            is_on(): bool {
                @@:(false)
            }
            get_brightness(): int {
                @@:(0)
            }
        }

        $On(brightness: int) {
            $>(greeting: str) {
                self.cycles = self.cycles + 1
                **@@:data.timestamp = self.timestamp_now()**
                self.log_event(f"on at {brightness}: {greeting}")
            }
            <$(reason: str) {
                **@@:data.timestamp = self.timestamp_now()**
                self.log_event(f"turning off: {reason}")
            }
            turn_off(reason: str) {
                (reason) -> "switch flipped" (reason) $Off
            }
            is_on(): bool {
                @@:(true)
            }
            get_brightness(): int {
                @@:(brightness)
            }
        }

    actions:
        log_event(msg: str) {
            **print(f"[{@@:data.timestamp}] [event] {msg}")**
            self.event_count = self.event_count + 1
        }
        **timestamp_now(): str {
            import datetime
            return datetime.datetime.now().isoformat()
        }**

    domain:
        cycles: int = 0
        event_count: int = 0
}
```

The lamp now records timestamps for every event. Each lifecycle
handler stashes the current time into `@@:data.timestamp` before
calling `log_event`. The action reads `@@:data.timestamp` to
include the time in its log message.

This shows what `@@:data` is for: **call-scoped scratch space
shared between a handler and the actions or lifecycle handlers it
triggers.**

The timestamp could have been a domain field, but it shouldn't
be — it's specific to one interface call. The next call gets its
own timestamp. Domain would mean every action sees whatever
timestamp the most recent handler happened to set, which is
fragile.

The timestamp could have been passed as an argument to
`log_event`. That works for one action, but as more actions need
the timestamp, every call site has to pass it. `@@:data` lets the
handler set it once and all the actions in the dispatch chain see
it.

FrameContext gains a dict for call-scoped data:

```python
class LampFrameContext:
    def __init__(self, event):
        self.event = event
        self._return = None
        **self._data = {}**
```

`_data` is a string-keyed dict, empty when the context is
constructed and populated by handlers and actions during the
call. Handlers and actions read and write it through the context
stack:

```python
def _s_On_hdl_frame_enter(self, __e, compartment):
    greeting = __e._parameters[0]
    brightness = compartment.state_args[0]
    self.cycles = self.cycles + 1
    **self._context_stack[-1]._data["timestamp"] = self.timestamp_now()**
    self.log_event(f"on at {brightness}: {greeting}")

def log_event(self, msg: str):
    **print(f"[{self._context_stack[-1]._data['timestamp']}] [event] {msg}")**
    self.event_count = self.event_count + 1
```

`@@:data.timestamp = expr` compiles to
`self._context_stack[-1]._data["timestamp"] = expr`.
`@@:data.timestamp` (read) compiles to the corresponding lookup.

The handler and the action both reach the same `_data` dict
because both run during the same interface call, with the same
context on top of the stack.

### Context data spans the dispatch chain

`@@:data` lives for one full dispatch — the original handler, any
exit/enter cascade triggered by transitions, and any actions
called by any of those. Once the interface call returns, the
context is popped and `_data` is discarded.

This means a handler can stash data, transition, and the
destination state's `$>` handler will see the same `_data`. The
context stays on top of the stack for the entire call.

A new interface call creates a fresh FrameContext with empty
`_data`. The store is per-call, not per-system.

### Why `_data` is dynamic

Domain fields and state variables have declared types. `_data`
doesn't. Keys are created on write; values are stored as the
target language's "any" type — `dict` in Python,
`HashMap<String, Box<dyn Any>>` in Rust, `Map<String, Object>` in
Java.

This is intentional. `_data` is reachable from any handler,
action, or lifecycle handler that runs during a dispatch. The set
of keys depends on which callables get reached and what they
decide to write. There's no static schema that captures this
without devolving to a dynamic map anyway. Frame represents the
store as what it actually is.

> **In statically-typed targets** — Reads from `_data` return the
> target's "any" type. When the value is assigned to a typed
> local or compared against a typed expression, the framepiler
> emits the cast automatically based on the use site's expected
> type. Direct uses (`if @@:data.flag`) work without a cast in
> targets that auto-coerce; otherwise the framepiler inserts the
> cast. Authors write the same `@@:data.x` syntax regardless of
> target.

### The three data stores

The lamp now uses all three of Frame's data stores:

| Store | Lifetime | Scope | Access |
|---|---|---|---|
| Domain | System lifetime | All states, all handlers | `self.field` |
| State variables | While the state is active | One state's handlers | `$.x` (Step 18) |
| Context data | One interface call | All handlers in the dispatch | `@@:data.k` |

State variables are the one we haven't seen yet. The lamp doesn't
need them — none of its data is "specific to a session of being
on." When we move to the Circuit Breaker in Step 18, state
variables will be the central feature.

---

## Step 16 — System parameters

```frame
**@@system Lamp(name: str = "Lamp") {**
    operations:
        static version(): str {
            return "1.0.0"
        }
        get_event_count(): int {
            return self.event_count
        }

    interface:
        turn_on(brightness: int)
        turn_off(reason: str)
        is_on(): bool
        get_brightness(): int
        **get_name(): str**

    machine:
        $Off {
            $>(last_reason: str) {
                @@:data.timestamp = self.timestamp_now()
                self.log_event(f"off: {last_reason}")
            }
            <$() {
                @@:data.timestamp = self.timestamp_now()
                self.log_event("turning on")
            }
            turn_on(brightness: int) {
                -> "switch flipped" ("hello") $On(brightness)
            }
            is_on(): bool {
                @@:(false)
            }
            get_brightness(): int {
                @@:(0)
            }
            **get_name(): str {
                @@:(self.name)
            }**
        }

        $On(brightness: int) {
            $>(greeting: str) {
                self.cycles = self.cycles + 1
                @@:data.timestamp = self.timestamp_now()
                **self.log_event(f"{self.name} on at {brightness}: {greeting}")**
            }
            <$(reason: str) {
                @@:data.timestamp = self.timestamp_now()
                self.log_event(f"turning off: {reason}")
            }
            turn_off(reason: str) {
                (reason) -> "switch flipped" (reason) $Off
            }
            is_on(): bool {
                @@:(true)
            }
            get_brightness(): int {
                @@:(brightness)
            }
            **get_name(): str {
                @@:(self.name)
            }**
        }

    actions:
        // ... (unchanged)

    domain:
        cycles: int = 0
        event_count: int = 0
        **name: str = name**
}
```

The lamp can now be named at construction time:

```python
kitchen = @@Lamp("kitchen")
desk = @@Lamp()             # default: "Lamp"
```

`name` is a **domain parameter** — a constructor argument that's
in scope when domain field initializers run. The domain block
declares `name: str = name`, which compiles to `self.name = name`
in the constructor. The same identifier means parameter on the
right, field on the left. The constructor signature picks up the
parameter:

```python
def __init__(self**, name: str = "Lamp"**):
    self.cycles = 0
    self.event_count = 0
    **self.name = name**
    self._context_stack = []
    self.__compartment = LampCompartment("Off")
    self.__next_compartment = None

    enter_event = LampFrameEvent("$>", [])
    enter_ctx = LampFrameContext(enter_event)
    self._context_stack.append(enter_ctx)
    self.__router(enter_event)
    self._context_stack.pop()
```

The default value (`"Lamp"`) is filled in by the framepiler at the
call site, so the constructor signature shows it as a default.
This works even in target languages that don't support parameter
defaults — the assembler substitutes the default into the call.

### Three groups of system parameters

The lamp uses only domain parameters. Frame supports three groups
total:

```frame
@@system Foo($(slot: int), $>(timeout: int), name: str) { ... }
```

| Group | Sigil | Lands in |
|---|---|---|
| State arg | `$(name: type)` | Start state's `compartment.state_args` |
| Enter arg | `$>(name: type)` | Start state's `compartment.enter_args` |
| Domain arg | `name: type` | Constructor parameter, used in domain initializers |

The sigils tell the framepiler where to route the value. State
args go into the start state's compartment so handlers can read
them. Enter args go to the start state's `$>` handler.

Call site:

```python
foo = @@Foo($(0), $>(1000), "primary")
```

The lamp doesn't use state or enter args at the system level — its
start state (`$Off`) doesn't take parameters, so there's nothing
to wire up.

---

## Step 17 — `const` domain fields and `@@:system.state`

```frame
@@system Lamp(name: str = "Lamp"**, max_brightness: int = 100**) {
    operations:
        static version(): str {
            return "1.0.0"
        }
        get_event_count(): int {
            return self.event_count
        }
        **get_state(): str {
            return @@:system.state
        }**

    interface:
        turn_on(brightness: int)
        turn_off(reason: str)
        is_on(): bool
        get_brightness(): int
        get_name(): str

    machine:
        $Off {
            $>(last_reason: str) {
                @@:data.timestamp = self.timestamp_now()
                self.log_event(f"off: {last_reason}")
            }
            <$() {
                @@:data.timestamp = self.timestamp_now()
                self.log_event("turning on")
            }
            turn_on(brightness: int) {
                **actual = brightness
                if actual > self.max_brightness:
                    actual = self.max_brightness
                -> "switch flipped" ("hello") $On(actual)**
            }
            // ... (rest unchanged)
        }

        // ... ($On unchanged)

    actions:
        // ... (unchanged)

    domain:
        cycles: int = 0
        event_count: int = 0
        name: str = name
        **const max_brightness: int = max_brightness**
}
```

Two additions:

- `max_brightness` is a `const` domain field. It's set from a
  system parameter at construction and can never be reassigned.
  Handlers that try will be rejected at compile time (E615).
- A new operation, `get_state()`, returns the current state name
  via `@@:system.state`.

The constructor emits the const field with a marker for its
immutability:

```python
def __init__(self, name: str = "Lamp"**, max_brightness: int = 100**):
    self.cycles = 0
    self.event_count = 0
    self.name = name
    **# const: max_brightness
    self.max_brightness = max_brightness**
    # ... rest of constructor
```

In Python, `const` is a comment-only marker — Python doesn't have
true field immutability. In other targets, the framepiler emits
the language's idiomatic keyword:

| Target | Emitted as |
|---|---|
| Java | `final int max_brightness = ...` |
| C# | `readonly int max_brightness = ...` |
| Swift | `let max_brightness: Int = ...` |
| Kotlin | `val max_brightness: Int = ...` |
| TypeScript | `readonly max_brightness: number = ...` |
| C++ | `const int max_brightness;` |
| Rust | (fields are immutable by default) |
| Python, JS, PHP, Ruby, Lua, Erlang, GDScript, C, Go | comment-only |

The framepiler enforces single-assignment at compile time
regardless of target — even in Python, assigning to a `const`
field in a handler body is E615.

`@@:system.state` compiles to a direct read off the current
compartment:

```python
def get_state(self) -> str:
    **return self.__compartment.state**
```

The state name string is just `self.__compartment.state`.
Read-only; you can't write to it.

Useful in operations (which don't go through the kernel) and in
diagnostic code that wants to know what state the system is in
without dispatching an event to find out.

### Other `@@:system` and `@@:self` access

Two prefixes worth knowing about:

- `@@:system.state` — the only `@@:system` reference currently
  defined. Reads the current state name.
- `@@:self.method(args)` — calls the system's own interface
  method. Goes through the full dispatch pipeline. We'll see this
  in Step 19 (Self-Calibrating Sensor).

Both are syntactic prefixes, not values. Bare `@@:self` or
`@@:system` is an error (E603/E604). Always chain a member.

---

## End of the lamp

The lamp has shown all the light it can on how the runtime
implements Frame's core features: states, transitions, lifecycle
handlers, return values, parameters in three categories, actions,
operations, context data, system parameters, and `const` fields.
We'll now progress to more complex examples to explore the more
sophisticated capabilities of Frame: state variables, self-calls,
push/pop, HSM, and persistence.

---

## Step 18 — Circuit Breaker (state variables)

A **circuit breaker** is a system that tolerates a few failures
but trips open after too many in a row, counts down a cooldown
period, then probes whether the downstream service has recovered.

```frame
@@target python_3

@@system CircuitBreaker {
    interface:
        call(): str = "error"
        success()
        failure()
        tick()
        status(): str = ""

    machine:
        $Closed {
            **$.failures: int = 0**

            call(): str { @@:("allowed") }
            success() { **$.failures = 0** }
            failure() {
                **$.failures = $.failures + 1
                if $.failures >= self.threshold:**
                    -> "tripped" $Open
            }
            status(): str { @@:(f"closed (**{$.failures}** failures)") }
        }
        $Open {
            **$.cooldown_remaining: int = 0**

            $>() {
                **$.cooldown_remaining = self.cooldown**
                print(f"Circuit OPEN — cooling down for {self.cooldown} ticks")
            }
            call(): str { @@:("blocked") }
            tick() {
                **$.cooldown_remaining = $.cooldown_remaining - 1
                if $.cooldown_remaining <= 0:**
                    -> "cooled down" $HalfOpen
            }
            status(): str { @@:(f"open (**{$.cooldown_remaining}** ticks left)") }
        }
        $HalfOpen {
            call(): str { @@:("testing") }
            success() {
                print("Circuit recovered")
                -> "recovered" $Closed
            }
            failure() {
                print("Still failing")
                -> "relapse" $Open
            }
            status(): str { @@:("half-open") }
        }

    domain:
        threshold: int = 3
        cooldown: int = 5
}
```

`$Closed` declares a **state variable** with `$.failures: int = 0`.
`$Open` declares its own state variable `$.cooldown_remaining`.
Both variables live on their state's compartment — they exist
while that state is current and go away when the state exits.

The headline behavior: when the breaker trips and later recovers,
`$.failures` resets to 0 automatically. The breaker doesn't
remember failures from the previous session of being closed.
That's because state variables are per-compartment, and entering
`$Closed` builds a new compartment.

Compartments need to carry state variables, so the Compartment
class gains a fourth field:

```python
class CircuitBreakerCompartment:
    def __init__(self, state):
        self.state = state
        self.state_args = []
        self.enter_args = []
        self.exit_args = []
        **self.state_vars = {}**
```

`state_vars` is a string-keyed dict, empty when the compartment is
constructed and populated by the state's `$>` handler.

> **In statically-typed targets** — `state_vars` is typed as a map
> of "any": `Map<String, Object>` in Java, `HashMap<String, Box<dyn Any>>`
> in Rust. Same access pattern, with casts at the read site
> emitted by the framepiler from the declared variable type:
>
> ```java
> int failures = (Integer) compartment.state_vars.get("failures");
> compartment.state_vars.put("failures", failures + 1);
> ```
>
> Static targets *without* type erasure (Rust, C, C++) can't use
> a string-keyed map cleanly because there's no "any" type that
> compiles efficiently. These targets generate a typed-struct-
> per-state with a tagged union across states:
>
> ```rust
> enum CompartmentVars {
>     Closed { failures: i32 },
>     Open { cooldown_remaining: i32 },
>     HalfOpen,
> }
> ```
>
> State variable access is direct field access through generated
> accessor methods rather than dictionary lookup. Same Frame
> source, different generated representation, same semantics.

`$Open`'s `$>` handler starts with an initialization block:

```python
def _s_Open_hdl_frame_enter(self, __e, compartment):
    **if "cooldown_remaining" not in compartment.state_vars:
        compartment.state_vars["cooldown_remaining"] = 0**
    compartment.state_vars["cooldown_remaining"] = self.cooldown
    print(f"Circuit OPEN — cooling down for {self.cooldown} ticks")
```

The `if "x" not in compartment.state_vars` guard is the
initialization pattern. Every state variable gets one — the
framepiler emits the guard at the top of every state's `$>`
handler.

The guard looks redundant: the compartment was just built with an
empty `state_vars`, so the check always succeeds. The guard
matters for `pop$`, which we'll see in Step 20 — popped
compartments come back with their `state_vars` already populated,
and the guard is what prevents re-initialization. The same
pattern also applies in HSM cascades (Step 21) where every layer
in a chain runs its own `$>` initialization independently.

`$Closed` doesn't declare a `$>` handler in the source, but it has
a state variable to initialize. The framepiler emits one anyway:

```python
def _s_Closed_hdl_frame_enter(self, __e, compartment):
    **if "failures" not in compartment.state_vars:
        compartment.state_vars["failures"] = 0**
```

When a state declares state variables, it gets a `$>` handler even
if the source didn't write one. The handler does whatever
initialization the variables need.

Handlers read and write state variables through the compartment
parameter:

```python
def _s_Closed_hdl_user_failure(self, __e, compartment):
    **compartment.state_vars["failures"] = compartment.state_vars["failures"] + 1
    if compartment.state_vars["failures"] >= self.threshold:**
        next_comp = CircuitBreakerCompartment("Open")
        self.__transition(next_comp)
        return

def _s_Open_hdl_user_tick(self, __e, compartment):
    **compartment.state_vars["cooldown_remaining"] = compartment.state_vars["cooldown_remaining"] - 1
    if compartment.state_vars["cooldown_remaining"] <= 0:**
        next_comp = CircuitBreakerCompartment("HalfOpen")
        self.__transition(next_comp)
        return
```

`$.failures` in Frame source compiles to
`compartment.state_vars["failures"]` — direct lookup in the dict
on the compartment parameter. `$.cooldown_remaining` works the
same way against `$Open`'s compartment.

This is why handlers receive a `compartment` parameter. State
variables live on a specific compartment; handlers need a
reference to that compartment to read or write them. The
framepiler ensures every handler gets passed the right one — the
router calls each state's dispatcher with the system's current
compartment, which is the dispatcher's own state's compartment.

### State variables reset on re-entry

Trace the breaker through a full cycle. Three `failure()` calls
climb `$.failures` from 0 to 3, hit threshold, and transition to
`$Open`. `$Open.$>` runs on a fresh compartment with empty
`state_vars`; the guard initializes `$.cooldown_remaining = 0`
and the handler body sets it to `self.cooldown` (5). Five
`tick()` calls count `$.cooldown_remaining` down from 5 to 0,
transitioning to `$HalfOpen`. A `success()` call in `$HalfOpen`
transitions to `$Closed`. `$Closed.$>` runs on a fresh
compartment with empty `state_vars`; the guard initializes
`$.failures = 0`.

The breaker is back to a clean slate. The previous session's
failure count is gone — it lived on the previous `$Closed`
compartment, which was discarded when the breaker transitioned to
`$Open`. Same for `$.cooldown_remaining`: the `$Open` compartment
is discarded when the breaker transitions to `$HalfOpen`, taking
the variable with it.

This is the difference between domain (lifetime: system) and
state variables (lifetime: this entry into the state). If
`failures` were a domain field, the count would survive `$Open`
and recovery would have to clear it explicitly. As a state
variable, it's automatic — the compartment is rebuilt on every
entry.

### Why state variables are per-compartment

State variables live on the compartment for two reasons.

**Lifetime matches the state's activity.** While the state is
current, its compartment exists and its variables are reachable.
When the state exits, the compartment is discarded (or pushed to
the state stack) and the variables go with it. Re-entering the
state builds a fresh compartment with fresh variables. There's no
cleanup the runtime has to do — the compartment lifecycle handles
it.

**Scope is hard to escape.** Frame source has `$.varName` syntax
for state variables but no syntax for "the variable in some other
state." Each handler can only reach its own state's variables
through its own compartment parameter. The framepiler enforces
this at compile time. There's no way to accidentally read or write
another state's variables.

Domain fields and context data exist for cases where data needs
to cross state boundaries. State variables are deliberately the
narrowest of the three stores.

---

## Step 19 — Self-Calibrating Sensor (self-calls)

A sensor whose calibration logic needs to read the current sensor
value through its own interface method.

```frame
@@target python_3

@@system Sensor {
    interface:
        calibrate(): bool
        reading(): int
        attempt_post_shutdown()
        trigger_shutdown()
        get_trace(): str

    machine:
        $Active {
            calibrate(): bool {
                **baseline = @@:self.reading()**
                self.offset = baseline * -1
                @@:(true)
            }
            reading(): int {
                @@:(self.sensor_value + self.offset)
            }
            attempt_post_shutdown() {
                **@@:self.trigger_shutdown()
                self.trace = self.trace + "after-call;"**
            }
            trigger_shutdown() {
                self.trace = self.trace + "shutdown-handler;"
                -> $Shutdown
            }
            get_trace(): str {
                @@:(self.trace)
            }
        }

        $Shutdown {
            calibrate(): bool { @@:(false) }
            reading(): int { @@:(0) }
            attempt_post_shutdown() { }
            trigger_shutdown() { }
            get_trace(): str { @@:(self.trace) }
        }

    domain:
        sensor_value: int = 100
        offset: int = 0
        trace: str = ""
}
```

The example shows two patterns. `calibrate()` calls
`@@:self.reading()` to get the baseline value, then computes the
offset — the basic self-call where a handler invokes one of its
own system's interface methods. `attempt_post_shutdown()` calls
`@@:self.trigger_shutdown()` and then tries to update `self.trace`
afterward; the shutdown handler transitions to `$Shutdown`, which
has consequences for the line that runs after the self-call.
That's the situation framec's automatic transition guard handles
for you — we'll work through the mechanics below.

### Self-call mechanics

`@@:self.reading()` compiles to `self.reading()` — a normal Python
method call:

```python
def _s_Active_hdl_user_calibrate(self, __e, compartment):
    **baseline = self.reading()**
    self.offset = baseline * -1
    self._context_stack[-1]._return = True
```

The framepiler validates at compile time that the method exists
in the system's interface (E601 otherwise) and that the argument
count matches (E602 otherwise). After validation, the emission is
straightforward — it really is just a method call.

What `self.reading()` reaches is the same wrapper an external
caller would invoke:

```python
def reading(self) -> int:
    __e = SensorFrameEvent("reading", [])
    __ctx = SensorFrameContext(__e, 0)
    **self._context_stack.append(__ctx)**
    self.__kernel(__e)
    return self._context_stack.pop()._return
```

Builds a FrameEvent, builds a FrameContext, pushes the context
onto the stack, runs the kernel, pops the context, returns the
value. This is where the context stack finally grows past depth
1: the wrapper for `calibrate()` pushed a context when the
external call started, and now `reading()`'s wrapper pushes
another. The stack has two entries:

```
_context_stack: [
    FrameContext(event=calibrate, _return=None, _data={}),
    FrameContext(event=reading,   _return=0,    _data={}),
]
```

The wrapper's `append` and `pop` operations have always been
there; with self-calls they finally do what they were designed
for.

Context isolation falls out of the stack structure. Inside
`reading()`, code that reads `@@:return`, `@@:event`, or
`@@:data` resolves against the top of the stack — the inner
FrameContext. Code that sets `@@:return` writes to the inner
slot:

```python
def _s_Active_hdl_user_reading(self, __e, compartment):
    self._context_stack[-1]._return = self.sensor_value + self.offset
```

`@@:(...)` compiles to `self._context_stack[-1]._return = ...`.
The `[-1]` access takes the top of the stack, which is whatever
context was pushed most recently — `reading()`'s context.

When `reading()`'s wrapper pops that context, the stack returns
to just `calibrate()`'s context. Any subsequent code in
`calibrate()`'s handler sees its own `_return`, `_data`, and
`_message` again — the inner call's writes can't bleed out.

The full execution trace when external code calls
`sensor.calibrate()` (indentation tracks call depth):

```
1.  calibrate() wrapper called
2.    push ctx_calibrate
3.    kernel → router → _state_Active → _s_Active_hdl_user_calibrate
4.      handler runs:
5.        baseline = self.reading()
6.          push ctx_reading
7.          kernel → router → _state_Active → _s_Active_hdl_user_reading
8.            sets ctx_reading._return = 100 + 0 = 100
9.          pop ctx_reading, return 100
10.       baseline = 100
11.       self.offset = -100
12.       sets ctx_calibrate._return = True
13.   pop ctx_calibrate, return True
```

Two context pushes, two context pops, two kernel runs — one for
each interface call. The outer call's state is preserved while
the inner call runs.

### Transitions during self-calls

`attempt_post_shutdown()` is more interesting because the
self-call it makes triggers a transition:

```python
def _s_Active_hdl_user_attempt_post_shutdown(self, __e, compartment):
    **self.trigger_shutdown()**
    self.trace = self.trace + "after-call;"
```

When the self-call returns, what state is the system in?

The kernel's deferred-transition model means transitions don't
happen during handler execution — they're queued and processed
after the handler returns. Self-calls are no exception: when
`trigger_shutdown()` queues `-> $Shutdown`, the kernel still
runs. `__transition` set `__next_compartment`, the handler
returned, and the kernel processed the transition before
returning to the outer caller. The full trace makes this
sequence visible:

```
1.  attempt_post_shutdown() wrapper called
2.    push ctx_attempt
3.    kernel → router → _state_Active → _s_Active_hdl_user_attempt_post_shutdown
4.      handler runs:
5.        self.trigger_shutdown()
6.          push ctx_trigger
7.          kernel → router → _state_Active → _s_Active_hdl_user_trigger_shutdown
8.            self.trace += "shutdown-handler;"
9.            __next_compartment = Shutdown compartment
10.         router returns
11.         __next_compartment is set → kernel processes the transition
12.         (no <$ or $> declared, but the compartment switch happens)
13.         __compartment is now $Shutdown's compartment
14.         **kernel marks every stacked context as transitioned**
15.       pop ctx_trigger
16.     trigger_shutdown returns
17.     **transition guard fires → return** (`self.trace += "after-call;"` is skipped)
18.   pop ctx_attempt
```

After `trigger_shutdown()` returns at line 16, the system is
already in `$Shutdown`. The line `self.trace = self.trace +
"after-call;"` does **not** run — framec inserts an automatic
transition guard immediately after every `@@:self.method(...)`
call. The trace ends up `"shutdown-handler;"` only.

This matters because code after a transitioning self-call would
otherwise be a footgun:

- It would read state variables that no longer exist (their
  compartment was just discarded).
- It would transition again, layering on top of the self-call's
  transition.
- It would run in a state where it doesn't make sense.

### The automatic transition guard

The runtime guards every self-call site for you. The
**FrameContext** gains another slot, `_transitioned`, that the
kernel sets to `True` on every stacked context after processing a
transition. After every `@@:self.method(...)` call, framec emits a
guard that returns early if the flag is set:

```python
class SensorFrameContext:
    def __init__(self, event, return_default):
        self.event = event
        self._return = return_default
        self._data = {}
        **self._transitioned = False**

def __kernel(self, __e):
    self.__router(__e)
    if self.__next_compartment is not None:
        # ...transition processing (exit cascade, switch, enter cascade)...
        **for ctx in self._context_stack:
            ctx._transitioned = True**

def _s_Active_hdl_user_attempt_post_shutdown(self, __e, compartment):
    self.trigger_shutdown()
    **if self._context_stack[-1]._transitioned: return**
    self.trace = self.trace + "after-call;"
```

The flag is per-context, set on the whole stack — so a deep self-
call chain triggering a transition guards every level. Each
handler returns through its guard, the wrappers pop their contexts
normally, and control unwinds cleanly to the original caller while
the system sits in the new state.

Strongly-typed targets emit the same shape with target-syntax for
the early return — `if (this._context_stack[length-1]._transitioned) return;`
in TypeScript, `if (_context_stack.back()._transitioned) return;`
in C++, and so on. Erlang implements the same functional contract
via a `gen_statem` case-expression on the returned data record's
`frame_current_state` rather than a flag — different mechanism,
identical observable behavior.

Self-calls go through the full dispatch pipeline so they behave
consistently with external calls — transitions, lifecycle
handlers, everything. The runtime guards against the obvious
foot-gun (continuing to run handler code after the system has
transitioned out from under it) by emitting an automatic check
after every self-call site. Authors get consistent dispatch
behavior without having to defend against in-flight transitions
themselves.

For self-calls that don't transition (like `@@:self.reading()` in
this sensor), the guard sees `_transitioned == False` and the
post-call code runs normally. The guard only short-circuits when
the called method actually queued a transition.

### Embedded self-calls and statement boundaries

Frame allows `@@:self.method()` calls inside expressions:

```frame
self.n = @@:self.compute() + 5
self.n = @@:self.foo() + @@:self.bar()
@@:return = @@:self.value() * 2
```

The transition check fires at **statement boundaries**, not within
statements. A statement containing one or more embedded self-calls
runs to completion in its own execution context; after the
statement, the handler returns if any embedded call transitioned
the system. The guard fires once at end-of-statement, regardless of
how many self-calls the expression contained.

```python
def _s_Active_hdl_user_combine(self, __e, compartment):
    self.n = self.foo() + self.bar()
    if self._context_stack[-1]._transitioned: return    # statement boundary
    self.trace += "after-combine;"
```

Both `foo()` and `bar()` always run, even if `foo()` queued a
transition before `bar()` was called. Once any embedded call sets
the `_transitioned` flag, the single statement-end check catches
it and the handler returns before the next statement.

Why statement-boundary rather than per-call abort? The runtime spec
already operates at statement boundaries — `_transitioned` is the
hook between statements. Inserting a guard mid-expression would
require per-target operator-precedence awareness across 17
backends, fragile codegen for an unusual idiom. Aligning with the
language-natural statement boundary keeps the codegen simple and
matches Frame's "Oceans Model" delegation of expression evaluation
to the target language.

If you need finer granularity — abort *between* two embedded calls
— split the expression into separate statements:

```frame
$.tmp = @@:self.foo()        // separate statement; if foo() queued
self.n = $.tmp + @@:self.bar() // a transition, bar() never runs
```

The natural one-call-per-statement idiom gives you the maximum
guard granularity Frame provides; cramming multiple self-calls into
one expression trades fineness of control for compactness.

### Validation

Self-calls have compile-time checks:

| Code | Check |
|---|---|
| E601 | Method doesn't exist in `interface:` |
| E602 | Argument count doesn't match |
| E603 | Bare `@@:self` (must be `@@:self.method(args)`) |
| E604 | Bare `@@:system` (must be `@@:system.state`) |

The validation runs at the same stage as other interface
references. By the time the framepiler emits code, all self-calls
have been resolved against real interface declarations.

---

## Step 20 — Modal Dialog Stack (push and pop)

A modal dialog stack: when a dialog opens on top of an existing
context, the system needs to remember the previous state and
return to it when the dialog closes.

```frame
@@target python_3

@@system Workflow {
    interface:
        start()
        interrupt(reason: str)
        resume()
        complete()
        tick()
        status(): str = ""

    machine:
        $Idle {
            start() {
                -> $Working
            }
            status(): str { @@:("idle") }
        }

        $Working {
            $.progress: int = 0

            $>() {
                print("started working")
            }

            interrupt(reason: str) {
                **push$
                -> $Interrupted(reason)**
            }

            complete() {
                $.progress = 100
                print(f"complete: {$.progress}%")
                -> $Idle
            }

            tick() {
                $.progress = $.progress + 10
            }

            status(): str { @@:(f"working ({$.progress}%)") }
        }

        $Interrupted(reason: str) {
            $>() {
                print(f"interrupted: {reason}")
            }

            resume() {
                **-> pop$**
            }

            status(): str { @@:(f"interrupted: {reason}") }
        }
}
```

`push$` saves the current compartment onto a stack. `-> pop$`
transitions back to the saved compartment.

The use case: the system is in `$Working` with some progress
accumulated (say `$.progress = 30`). An interrupt arrives, so
`push$` saves the `$Working` compartment, then `-> $Interrupted(reason)`
moves to the interrupted state. When `resume()` is called,
`-> pop$` restores the saved `$Working` compartment — including
`$.progress = 30` — and the workflow continues from where it left
off. Without `push$`/`pop$`, returning to `$Working` would build a
fresh compartment with `$.progress = 0` and the work in progress
would be lost.

The system needs a place to save compartments, so the constructor
gains a state stack:

```python
def __init__(self):
    self._context_stack = []
    **self._state_stack = []**
    self.__compartment = WorkflowCompartment("Idle")
    self.__next_compartment = None
    enter_event = WorkflowFrameEvent("$>", [])
    enter_ctx = WorkflowFrameContext(enter_event)
    self._context_stack.append(enter_ctx)
    self.__router(enter_event)
    self._context_stack.pop()
```

`_state_stack` is a Python list. It holds saved compartments —
references, not copies. Empty when the system starts.

`push$` appends the current compartment to that list:

```python
def _s_Working_hdl_user_interrupt(self, __e, compartment):
    reason = __e._parameters[0]
    **self._state_stack.append(self.__compartment)**
    next_comp = WorkflowCompartment("Interrupted")
    next_comp.state_args = [reason]
    self.__transition(next_comp)
    return
```

The saved compartment object — including its `state_vars` dict
with all the work-in-progress values — gets a new reference on
the stack. The compartment itself isn't copied; the stack and the
system both point at the same object.

After `push$`, the handler builds an `$Interrupted` compartment
and calls `__transition`. The kernel runs the exit cascade for
`$Working`, then enters `$Interrupted` — but the `$Working`
compartment isn't garbage collected because the state stack still
has a reference to it.

`-> pop$` reverses the operation:

```python
def _s_Interrupted_hdl_user_resume(self, __e, compartment):
    **next_comp = self._state_stack.pop()**
    self.__transition(next_comp)
    return
```

The list's `pop()` removes and returns the last item — the saved
`$Working` compartment. The handler then calls `__transition`
with it as the destination, and the kernel processes the
transition normally: fire `<$` on `$Interrupted`, switch
`__compartment`, fire `$>` on `$Working`.

This is where the initialization guard from Step 18 matters.
`$Working.$>` runs as if entering normally:

```python
def _s_Working_hdl_frame_enter(self, __e, compartment):
    **if "progress" not in compartment.state_vars:
        compartment.state_vars["progress"] = 0**
    print("started working")
```

On a fresh entry via `start()`, the compartment is new and
`state_vars` is empty — the guard sees `"progress"` is missing
and initializes it to 0. On a `pop$` re-entry, the compartment is
the saved one and `state_vars` already has `{"progress": 30}` —
the guard sees `"progress"` *is* there and skips initialization,
so `$.progress` keeps its value.

| Operation | `state_vars` at `$>` entry | Initialization runs? |
|---|---|---|
| `-> $Working` (fresh) | `{}` | yes |
| `-> pop$` (restored) | `{"progress": 30, ...}` | no |

Without the guard, `pop$` would reset state variables and the
work-in-progress would be lost. With it, restoration preserves
them. This is the whole reason the guard exists.

### Push/pop is a stack

Multiple `push$` calls layer compartments and `pop$` retrieves
them in last-in-first-out order:

```
state: $A           stack: []
push$               stack: [$A]
-> $B               state: $B
push$               stack: [$A, $B]
-> $C               state: $C
-> pop$             state: $B (popped from stack)  stack: [$A]
-> pop$             state: $A (popped from stack)  stack: []
```

The state stack is a separate concept from the context stack
(`_context_stack`) we saw in Step 19. They have different
purposes: the state stack holds compartments and is manipulated
explicitly through `push$` and `pop$` — it's used for save/restore
patterns. The context stack holds FrameContexts and is managed
automatically by interface method wrappers — it's used for nested
call isolation.

A handler can push state with `push$`, fire a self-call (which
pushes and pops a context), and the state stack is unaffected.
The two stacks operate independently.

### When the stack is empty

`-> pop$` on an empty `_state_stack` is undefined. Python raises
`IndexError`; other targets fail with their language's equivalent.
The framepiler doesn't check at compile time — keeping push/pop
balanced is the author's responsibility, like keeping any other
stack discipline.

---

## Step 21 — Thermostat (hierarchical state machines)

A thermostat that has multiple operating modes — heating, cooling,
and fan-only — but shares logic for power on/off across all of
them.

```frame
@@target python_3

@@system Thermostat {
    interface:
        power_off()
        adjust(setpoint: int)
        get_mode(): str

    machine:
        $Active {
            $.setpoint: int = 70

            $>() {
                print("thermostat active")
            }

            <$() {
                print("powering down")
            }

            adjust(setpoint: int) {
                $.setpoint = setpoint
            }

            power_off() {
                -> $Off
            }
        }

        **$Heating => $Active {**
            $>() {
                print("heating mode")
            }

            get_mode(): str {
                @@:("heating")
            }
        }

        **$Cooling => $Active {**
            $>() {
                print("cooling mode")
            }

            get_mode(): str {
                @@:("cooling")
            }
        }

        $Off {
            get_mode(): str {
                @@:("off")
            }
        }
}
```

`$Heating => $Active` declares `$Heating` as a child of `$Active`.
Same with `$Cooling`. The arrow points from child to parent. When
the system is in `$Heating`, it's *also* in `$Active` — the parent
state's compartment is part of the active chain, and parent
handlers participate in the lifecycle.

Three things become visible with HSM:

1. When the system enters `$Heating`, both `$Active.$>` and
   `$Heating.$>` fire — parent first, child second.
2. When the system leaves `$Heating`, both `$Heating.<$` and
   `$Active.<$` fire — child first, parent second.
3. The parent's compartment is reachable from the child's
   compartment.

The first two are the *cascade*. The third is what makes parameter
propagation and event forwarding work in later steps.

Compartments need to know their parent in the chain, so the
Compartment class gains a field:

```python
class ThermostatCompartment:
    def __init__(self, state):
        self.state = state
        self.state_args = []
        self.enter_args = []
        self.exit_args = []
        self.state_vars = {}
        **self.parent_compartment = None**
```

`parent_compartment` is `None` for states without an HSM parent
(`$Active`, `$Off`) and points at the parent's compartment for
HSM children (`$Heating`, `$Cooling`).

The framepiler emits a static topology table that knows which
states have parents:

```python
**_HSM_CHAIN = {
    "Active":  ["Active"],
    "Heating": ["Active", "Heating"],
    "Cooling": ["Active", "Cooling"],
    "Off":     ["Off"],
}**
```

Each entry maps a leaf state name to the chain from root to leaf.
`$Heating`'s chain is `["Active", "Heating"]` — the parent first,
the leaf last. `$Active` has just itself. The table is generated
once at compile time from the source's `=> $Parent` declarations.

The transition into an HSM state needs to build the whole chain,
not just the leaf compartment. A new helper does this:

```python
**def __prepareEnter(self, leaf, state_args, enter_args):
    previous = None
    for name in _HSM_CHAIN[leaf]:
        comp = ThermostatCompartment(name)
        comp.state_args = list(state_args)
        comp.enter_args = list(enter_args)
        comp.parent_compartment = previous
        previous = comp
    return comp**
```

For a transition to `$Heating`, the loop runs twice: builds an
`$Active` compartment, then builds a `$Heating` compartment with
`parent_compartment` pointing at the `$Active` one. The leaf
compartment is what gets returned.

Handlers that transition into an HSM state use `__prepareEnter`
instead of building the compartment directly:

```python
def _s_Off_hdl_user_adjust(self, __e, compartment):
    setpoint = __e._parameters[0]
    **next_comp = self.__prepareEnter("Heating", [], [])**
    self.__transition(next_comp)
    return
```

Same pattern as before — build the next compartment, hand it to
`__transition`, return — but the construction goes through the
helper.

The kernel needs to fire the cascade in the right order. On entry,
top-down (parent's `$>` first, then child's). On exit, bottom-up
(child's `<$` first, then parent's). The kernel's transition
processing picks up two helpers:

```python
def __kernel(self, __e):
    self.__router(__e)

    if self.__next_compartment is not None:
        next_compartment = self.__next_compartment
        self.__next_compartment = None

        **self.__fire_exit_cascade()**

        self.__compartment = next_compartment

        **self.__fire_enter_cascade()**
```

The exit cascade walks up from the current leaf compartment,
firing `<$` on each:

```python
**def __fire_exit_cascade(self):
    comp = self.__compartment
    while comp is not None:
        exit_event = ThermostatFrameEvent("<$", comp.exit_args)
        self.__route_to_state(comp.state, exit_event, comp)
        comp = comp.parent_compartment**
```

It starts at `self.__compartment` (the leaf), fires `<$` against
that compartment, walks up via `parent_compartment`, fires `<$`
against the next one, and so on until it hits `None`. For
`$Heating`, this fires `$Heating.<$` then `$Active.<$`.

The enter cascade walks down from the new chain's root, firing
`$>` on each:

```python
**def __fire_enter_cascade(self):
    chain = []
    comp = self.__compartment
    while comp is not None:
        chain.append(comp)
        comp = comp.parent_compartment

    for comp in reversed(chain):
        enter_event = ThermostatFrameEvent("$>", comp.enter_args)
        self.__route_to_state(comp.state, enter_event, comp)**
```

It collects the chain by walking up, then iterates in reverse
(root first, leaf last). For an entry into `$Heating`, this fires
`$Active.$>` then `$Heating.$>`.

Both cascades route through a small helper that calls a specific
state's dispatcher with a specific compartment, rather than the
system's current compartment:

```python
**def __route_to_state(self, state_name, __e, compartment):
    if state_name == "Active":
        self._state_Active(__e, compartment)
    elif state_name == "Heating":
        self._state_Heating(__e, compartment)
    elif state_name == "Cooling":
        self._state_Cooling(__e, compartment)
    elif state_name == "Off":
        self._state_Off(__e, compartment)**
```

This is a variant of the router from earlier steps. The original
router always uses `self.__compartment`; this one takes the
compartment as a parameter so cascades can route to ancestors.

When a handler runs during a cascade, it gets *its own state's
compartment* — not the leaf's. `$Active.$>` runs against the
`$Active` compartment; `$Heating.$>` runs against the `$Heating`
compartment. State variables stay per-state because each
compartment in the chain is its own object.

Trace what happens when external code calls `adjust(72)` from
`$Off`. The wrapper queues the event; the router calls
`_state_Off`'s dispatcher, which calls `_s_Off_hdl_user_adjust`.
The handler calls `__prepareEnter("Heating", [], [])`, which
builds an `$Active` compartment, then a `$Heating` compartment
with `parent_compartment` pointing at the `$Active` one, and
returns the `$Heating` compartment. The handler calls
`__transition(next_comp)` and returns. The kernel sees
`__next_compartment` is set; it calls `__fire_exit_cascade()`,
but the current compartment is `$Off` (no parent), so this fires
`$Off.<$` and stops — and since `$Off` has no exit handler,
nothing actually runs. The kernel sets `__compartment` to the new
leaf (`$Heating`), then calls `__fire_enter_cascade()`. This
walks up the chain from `$Heating` to find `$Active` at the root,
then iterates in reverse: fires `$Active.$>` (prints "thermostat
active") against the `$Active` compartment, then fires
`$Heating.$>` (prints "heating mode") against the `$Heating`
compartment.

The system is now in `$Heating`, with an `$Active` compartment
underneath. `__compartment` points at the leaf.
`__compartment.parent_compartment` points at `$Active`.

Calling `power_off()` from `$Heating` works because the dispatcher
pattern needs an extension. We'll cover that — and parameter
propagation between layers — in Step 22.

### Every transition rebuilds the destination chain

Worth being explicit about this: `__prepareEnter` builds *every*
compartment in the destination chain from scratch, regardless of
which compartments existed before. There's no compartment reuse,
no LCA (lowest common ancestor) optimization, no "stay where you
already are."

This means a transition between siblings under a shared parent —
say `$Heating` to `$Cooling`, both children of `$Active` — does
the following:

1. Exit cascade fires bottom-up on the source chain: `$Heating.<$`,
   then `$Active.<$`. Both run.
2. The kernel switches `__compartment` to the new chain's leaf.
3. Enter cascade fires top-down on the destination chain:
   `$Active.$>` (on a *new* `$Active` compartment), then
   `$Cooling.$>`. Both run.

The previous `$Active` compartment is discarded. Any state
variables it held are gone. Its `<$` ran on exit; the new
`$Active` compartment's `$>` runs on entry.

This differs from UML statecharts, which suppress the parent's
lifecycle on intra-subtree moves to preserve composite-state
identity. Frame's runtime treats every transition uniformly —
build the destination chain, fire the cascades, no exceptions.
The model is simpler (one rule, applied uniformly) at the cost
of composite-state persistence within a subtree.

If you need state to persist across sibling transitions, put it
in domain (it survives all transitions) rather than on the parent
state. Domain is the right tool for "this value belongs to the
system, not to any particular state's lifecycle."

The example above puts `$.setpoint` on `$Active` — which means
switching modes loses the setpoint between transitions. Step 22
revises this: `setpoint` becomes a state arg passed at every
transition, with `self.last_setpoint` (a domain field) recording
the most recent value. The state variable on the parent state
turned out to be the wrong tool for the job — exactly the kind
of mistake the rebuild-on-every-transition rule makes visible.

### Self-calls and HSM

The transition guard pattern from Step 19 (checking
`@@:system.state` after a self-call that might transition) still
works with HSM. `@@:system.state` reads the leaf state's name —
which is what you want. After a self-call that transitions from
`$Heating` to `$Cooling`, `@@:system.state` returns `"Cooling"`.
The guard pattern is unaffected by HSM depth.

---

## Step 22 — Thermostat (parameter propagation)

The thermostat's `$Active` parent state needs more than just
lifecycle handlers — it needs to receive and act on parameters.
A real thermostat would want the setpoint to be passed in at
every transition into a mode, not just via `adjust()` after the
fact.

```frame
@@target python_3

@@system Thermostat {
    interface:
        switch_to_heating(setpoint: int, reason: str)
        switch_to_cooling(setpoint: int, reason: str)
        power_off(reason: str)
        get_setpoint(): int

    machine:
        $Off {
            switch_to_heating(setpoint: int, reason: str) {
                **(reason) -> ("starting up") $Heating(setpoint)**
            }
            switch_to_cooling(setpoint: int, reason: str) {
                **(reason) -> ("starting up") $Cooling(setpoint)**
            }
        }

        **$Active(setpoint: int) {**
            **$>(message: str) {**
                print(f"thermostat active: {message}")
                self.last_setpoint = setpoint
            }

            **<$(reason: str) {**
                print(f"powering down: {reason}")
            }

            power_off(reason: str) {
                **(reason) -> $Off**
            }

            get_setpoint(): int {
                **@@:(setpoint)**
            }
        }

        **$Heating(setpoint: int) => $Active {**
            **$>(message: str) {**
                print(f"heating mode: {message}, target {setpoint}")
            }

            **<$(reason: str) {**
                print(f"heating off: {reason}")
            }
        }

        **$Cooling(setpoint: int) => $Active {**
            **$>(message: str) {**
                print(f"cooling mode: {message}, target {setpoint}")
            }

            **<$(reason: str) {**
                print(f"cooling off: {reason}")
            }
        }

    domain:
        last_setpoint: int = 0
}
```

Three things changed structurally:

1. `$Active` now declares `(setpoint: int)` as its state
   parameter, and a matching enter handler `$>(message: str)` and
   exit handler `<$(reason: str)`.
2. `$Heating` and `$Cooling` declare the *same* signatures:
   `(setpoint: int)` for the state, `$>(message: str)`,
   `<$(reason: str)`. This is required.
3. The transition syntax now passes args through all three
   channels: state arg `(setpoint)`, enter arg `("starting up")`,
   exit arg `(reason)`.

The signature-match requirement is the headline new constraint.
Let's see why it matters.

### The signature-match rule

When a transition targets `$Heating(72)`, both `$Active.$>` and
`$Heating.$>` need to run. If `$Active` declares
`$>(message: str)`, then the kernel needs to deliver a `message`
string to it. If `$Heating` declares `$>(message: str)`, the
kernel needs to deliver one to it too. The transition supplies
*one* enter arg list, and that same list flows to every layer of
the chain.

For this to be type-safe, every state in the chain has to declare
the same signature. Mismatched signatures would mean a parent
expects different arguments than what the transition provides,
or a child expects different arguments than what propagated to
it. The framepiler rejects this at compile time.

The rule applies to all three channels:

| Channel | Constraint |
|---|---|
| `state_args` | `$Child(args)` must match `$Parent(args)` exactly |
| `enter_args` | Child's `$>(args)` must match parent's `$>(args)` exactly, or both states declare none |
| `exit_args` | Child's `<$(args)` must match parent's `<$(args)` exactly, or both states declare none |

By "match" we mean same parameter names and same types. Order
matters. "Or both declare none" handles the case where neither
state declares the lifecycle handler — propagation is vacuous if
neither side reads anything.

State args follow a different runtime mechanism than enter and
exit args, but the same propagation rule applies. Enter and exit
args ride on synthesized `$>` and `<$` events — the kernel builds
one event per layer, each carrying the same args list. State
args don't ride on events; they sit on the compartment as a
persistent field that any handler in the state can read.
`__prepareEnter` writes the same values into each layer's
`state_args` because the signatures match, so a parent's user
handler reading `compartment.state_args[0]` sees the same value
the transition supplied. The signature-match rule applies to
state args for the same structural reason as enter args: a
handler reading `compartment.state_args[0]` must see a value of
the type its declaration claims.

Future work may relax this to a prefix-match rule (parent's
signature is a prefix of child's — child may extend with
additional parameters), but v4 enforces exact match.

### How propagation works in the runtime

`__prepareEnter` already walked the chain and set `state_args`
and `enter_args` on every compartment. With matching signatures,
every layer receives the same values in the same positions:

```python
def __prepareEnter(self, leaf, state_args, enter_args):
    previous = None
    for name in _HSM_CHAIN[leaf]:
        comp = ThermostatCompartment(name)
        **comp.state_args = list(state_args)**
        **comp.enter_args = list(enter_args)**
        comp.parent_compartment = previous
        previous = comp
    return comp
```

For `__prepareEnter("Heating", [72], ["starting up"])`, both the
`$Active` compartment and the `$Heating` compartment get
`state_args = [72]` and `enter_args = ["starting up"]`.

Each layer holds its own list (`list(state_args)` makes a copy).
They're independent objects with the same contents — modifications
to one wouldn't affect the others. In practice handlers don't
modify the args lists; treating them as immutable per-layer
copies is the cleanest mental model.

Exit args propagate the same way, but they're populated on the
*current* chain at transition time, not on the new chain. A new
helper `__prepareExit` walks up from the current leaf and
populates `exit_args` on every compartment in the chain:

```python
**def __prepareExit(self, exit_args):
    comp = self.__compartment
    while comp is not None:
        comp.exit_args = list(exit_args)
        comp = comp.parent_compartment**
```

Handlers that pass exit args call this helper before
transitioning:

```python
def _s_Active_hdl_user_power_off(self, __e, compartment):
    reason = __e._parameters[0]
    **self.__prepareExit([reason])**
    next_comp = self.__prepareEnter("Off", [], [])
    self.__transition(next_comp)
    return
```

By the time the kernel fires the exit cascade, every compartment
in the source chain has `exit_args` populated. The cascade reads
`comp.exit_args` from each compartment (which it was already
doing — that line was generic from Step 21), so each layer's `<$`
handler receives the values.

### Each layer binds its own copy

Lifecycle handlers use the standard parameter binding pattern —
they read from `__e._parameters`, which the kernel populates from
the compartment's args. The handler at each layer binds `message`
(or `reason`, or whatever) at the top:

```python
def _s_Active_hdl_frame_enter(self, __e, compartment):
    **message = __e._parameters[0]
    setpoint = compartment.state_args[0]**
    print(f"thermostat active: {message}")
    self.last_setpoint = setpoint

def _s_Heating_hdl_frame_enter(self, __e, compartment):
    **message = __e._parameters[0]
    setpoint = compartment.state_args[0]**
    print(f"heating mode: {message}, target {setpoint}")
```

Both handlers bind `message` from `__e._parameters[0]` — the
kernel synthesized one `$>` event per layer, each carrying the
same enter args list as `_parameters`. Both bind `setpoint` from
`compartment.state_args[0]` — but each handler's `compartment` is
its own layer's compartment (`$Active`'s compartment for
`$Active.$>`, `$Heating`'s compartment for `$Heating.$>`). The
kernel's cascade routing made sure of this in Step 21.

The user handler `get_setpoint()` reads from its own state's
compartment too:

```python
def _s_Active_hdl_user_get_setpoint(self, __e, compartment):
    **setpoint = compartment.state_args[0]**
    self._context_stack[-1]._return = setpoint
```

Because `get_setpoint` is declared on `$Active`, the dispatcher
that picks this handler is `_state_Active`'s, which receives the
`$Active` compartment when called. We'll see in Step 23 how a
call to `get_setpoint` from `$Heating` actually reaches
`$Active`'s dispatcher — that's the event forwarding mechanism.

### Cascade and context data

The cascade fires multiple lifecycle handlers — but it's still
one interface call, so they all share the same FrameContext on
top of the `_context_stack`. That has consequences for
`@@:data`. If `$Active.$>` writes `@@:data.timestamp = "T1"` and
then `$Heating.$>` writes `@@:data.timestamp = "T2"`, the second
write wins. The dict is shared; last writer takes the slot.

Same for `@@:return` set during a cascade — though setting return
values from lifecycle handlers is unusual. The point is just that
cascade layers aren't isolated from each other through the
context stack the way self-calls are. They're all part of one
dispatch.

In practice this rarely surprises anyone. Lifecycle handlers
across cascade layers usually write to disjoint keys when they
write to `@@:data` at all, and the inheritance is what authors
want — a parent's `$>` setting up `@@:data.session_id` so the
child's `$>` can use it is a reasonable pattern. But if you find
two layers both writing the same key, the later layer (closer to
the leaf on entry, closer to the leaf on exit) wins.

### Trace through a transition

Calling `switch_to_heating(72, "morning")` from `$Off`:

The wrapper builds the event with `_parameters = [72, "morning"]`.
The router calls `_state_Off`'s dispatcher, which calls the
handler. The handler binds `setpoint = 72` and `reason =
"morning"`. It calls `__prepareEnter("Heating", [72], ["starting
up"])`, which builds the `$Active` compartment with `state_args =
[72]` and `enter_args = ["starting up"]`, then builds the
`$Heating` compartment with the same args, and returns the
`$Heating` compartment.

The handler calls `__transition` and returns. The kernel sees
`__next_compartment` is set. The exit cascade fires `<$` on the
source chain — just `$Off`, which has no exit handler. Then the
kernel switches `__compartment` to the new `$Heating`
compartment.

The enter cascade walks up to find `$Active` at the root, then
iterates in reverse. First it fires `$Active.$>` against the
`$Active` compartment with `_parameters = ["starting up"]`. The
handler binds `message = "starting up"` from the event and
`setpoint = 72` from `compartment.state_args[0]`. It prints
"thermostat active: starting up" and sets `self.last_setpoint =
72`.

Then it fires `$Heating.$>` against the `$Heating` compartment
with `_parameters = ["starting up"]`. The handler binds `message
= "starting up"` from the event and `setpoint = 72` from
`compartment.state_args[0]`. It prints "heating mode: starting
up, target 72".

Two prints; two lifecycle handlers; same args at every layer.

### Switching modes (sibling transition)

Step 21 introduced the rule that every transition rebuilds the
destination chain. With parameter propagation now in play, that
rule has visible consequences for sibling transitions. Trace
through `switch_to_cooling(68, "evening")` while the system is
in `$Heating`:

The wrapper builds the event with `_parameters = [68,
"evening"]`. How the router reaches a handler in `$Heating`
(which doesn't declare `switch_to_cooling` directly) is covered
in Step 23 via `=> $^`. For this trace, the focus is on what
the kernel does once `__transition` has been called — the
dispatcher routing that got us there is settled.

The handler binds `setpoint = 68`, `reason = "evening"`. It
calls `__prepareExit(["evening"])`, which walks the current chain
and writes `["evening"]` to both `$Heating`'s and `$Active`'s
`exit_args`. It calls `__prepareEnter("Cooling", [68], ["starting
up"])`, building a fresh `$Active` compartment with `state_args =
[68]` and a `$Cooling` compartment with the same. The handler
calls `__transition` and returns.

The kernel processes the transition:

1. Exit cascade, bottom-up on the source chain. Fire
   `$Heating.<$` with `_parameters = ["evening"]`: prints
   "heating off: evening". Walk up to `$Active`. Fire
   `$Active.<$` with `_parameters = ["evening"]`: prints
   "powering down: evening".
2. Switch `__compartment` to the new `$Cooling` leaf.
3. Enter cascade, top-down on the destination chain. Fire
   `$Active.$>` with `_parameters = ["starting up"]`: prints
   "thermostat active: starting up", sets `self.last_setpoint =
   68`. Fire `$Cooling.$>` with `_parameters = ["starting up"]`:
   prints "cooling mode: starting up, target 68".

Four prints, in order:

```
heating off: evening
powering down: evening
thermostat active: starting up
cooling mode: starting up, target 68
```

Two of those — `$Active.<$` and `$Active.$>` — are the parent
state's lifecycle running on the way out and back in. The
previous `$Active` compartment is gone. Any state variables it
held are gone with it. The new `$Active` compartment is freshly
constructed.

If `$Active` had a state variable like `$.uptime`, switching from
`$Heating` to `$Cooling` would reset it. Authors who expect the
parent to "stay active" while switching modes are working from a
mental model Frame doesn't share. The runtime treats the parent
as part of the chain, and chains are rebuilt on every transition.

Use domain for cross-mode persistence (`self.uptime` survives
every transition). Use state vars for state-local concerns. The
distinction is sharper here than in flat state machines because
HSM creates the temptation to put "shared" data on the parent;
Frame's runtime model says that temptation should be resisted.

---

## Step 23 — Thermostat (event forwarding with `=> $^`)

Step 22 left a question hanging: the thermostat's `$Active` parent
declares `power_off()` and `get_setpoint()` handlers, but the
system can be in `$Heating` or `$Cooling` when a caller invokes
those methods. How does the event reach the parent's handler?

Add a single line to each child to find out:

```frame
$Heating(setpoint: int) => $Active {
    **=> $^**

    $>(message: str) {
        print(f"heating mode: {message}, target {setpoint}")
    }

    <$(reason: str) {
        print(f"heating off: {reason}")
    }
}

$Cooling(setpoint: int) => $Active {
    **=> $^**

    $>(message: str) {
        print(f"cooling mode: {message}, target {setpoint}")
    }

    <$(reason: str) {
        print(f"cooling off: {reason}")
    }
}
```

`=> $^` declares that unhandled events in this state forward to
the parent state's dispatcher. Without it, events that don't match
any of `$Heating`'s declared handlers fall off the end and are
ignored.

Forwarding is opt-in. A child without `=> $^` is genuinely sealed
— even if its parent declares a handler for an event, calling that
event while in the child does nothing. This is deliberate. State
machines often have child states that should *not* respond to
parent events (think of an `$Editing` mode where `save()` is
suppressed because there's nothing to save yet). Frame doesn't
assume forwarding; it requires the author to declare it.

### How the dispatcher changes

The state dispatcher in Step 3 ended after the last handler match.
With `=> $^`, the dispatcher gains a fall-through that routes to
the parent:

```python
def _state_Heating(self, __e, compartment):
    if __e._message == "$>":
        self._s_Heating_hdl_frame_enter(__e, compartment); return
    if __e._message == "<$":
        self._s_Heating_hdl_frame_exit(__e, compartment); return
    **# => $^ — fall through to parent
    self._state_Active(__e, compartment.parent_compartment)**
```

The fall-through line at the bottom calls the parent's dispatcher
(`_state_Active`) with the parent's compartment
(`compartment.parent_compartment`). No condition — anything that
didn't match a declared handler reaches this line.

The compartment swap is critical. The parent's dispatcher and its
handlers expect to see the parent's compartment, not the child's.
`$Heating`'s `compartment` parameter holds the `$Heating`
compartment; its `parent_compartment` field points at the
`$Active` compartment. Passing that up means `$Active`'s handlers
can read `compartment.state_args[0]` and get `$Active`'s setpoint
(which is the same value, by the propagation rule from Step 22 —
but the principle is that each layer reads from its own
compartment).

`$Cooling`'s dispatcher gets the same fall-through line:

```python
def _state_Cooling(self, __e, compartment):
    if __e._message == "$>":
        self._s_Cooling_hdl_frame_enter(__e, compartment); return
    if __e._message == "<$":
        self._s_Cooling_hdl_frame_exit(__e, compartment); return
    **self._state_Active(__e, compartment.parent_compartment)**
```

`$Active`'s dispatcher doesn't change. It already has handlers for
`power_off`, `get_setpoint`, `$>`, and `<$`. It doesn't have a
fall-through because `$Active` itself has no parent — `$Active`'s
unhandled events are genuinely ignored.

`=> $^` is only legal in states that declare an HSM parent. A
state without a parent has nothing to forward to, so the
declaration is meaningless. The framepiler rejects it at compile
time.

### Trace a forwarded event

Calling `get_setpoint()` while the system is in `$Heating`:

The wrapper builds the event with `_message = "get_setpoint"`. The
router reads `self.__compartment.state` — that's `"Heating"` — so
it calls `_state_Heating(event, self.__compartment)`. The
dispatcher checks `$>` (no match) and `<$` (no match) and falls
through to `self._state_Active(event, compartment.parent_compartment)`.

This reaches `$Active`'s dispatcher with the `$Active`
compartment. The dispatcher matches `get_setpoint` and calls
`_s_Active_hdl_user_get_setpoint(event, compartment)` — where
`compartment` is now the `$Active` compartment, not the original
`$Heating` one. The handler binds `setpoint =
compartment.state_args[0]` from the `$Active` compartment, sets
`@@:return = setpoint`, returns.

The wrapper pops the context and returns the value. The caller
gets the setpoint.

The same call from `$Cooling` would route through `$Cooling`'s
dispatcher's fall-through, reach `$Active` the same way, and
produce the same result. The parent's handler is the one source
of truth, regardless of which child the system is in.

### Forwarding doesn't change the system's state

Worth being explicit about this: `=> $^` forwards an event up the
chain at *dispatch* time. It doesn't transition the system. The
system is still in `$Heating` after `get_setpoint()` returns. No
exit cascade fires; no enter cascade fires; `__compartment`
doesn't change.

This is different from a transition that targets the parent. If
the source said `-> $Active`, that *would* transition — leaving
`$Heating` (firing `$Heating.<$`) and entering `$Active` (firing
`$Active.$>`). The cascade would also trigger because of the HSM
relationship.

`=> $^` is purely a dispatch-routing construct. The state stays
where it is; the event just gets dispatched to the parent's
handlers.

### Multi-level chains

If the thermostat had a third level — say `$EcoHeating => $Heating
=> $Active` — `=> $^` declared on `$EcoHeating` would forward to
`$Heating`. If `$Heating` also declared `=> $^`, those events
would forward again to `$Active`. The walk continues until either
a handler matches or the chain runs out.

In the dispatcher, this looks like:

```python
def _state_EcoHeating(self, __e, compartment):
    # ... handlers ...
    self._state_Heating(__e, compartment.parent_compartment)
```

`$Heating`'s dispatcher (also with `=> $^` declared) ends with:

```python
def _state_Heating(self, __e, compartment):
    # ... handlers ...
    self._state_Active(__e, compartment.parent_compartment)
```

The unhandled event walks up one level per dispatcher call, with
`compartment.parent_compartment` swapping in the right compartment
at each level. By the time `$Active`'s dispatcher runs, the
compartment is the `$Active` compartment. If `$Active` also fails
to handle the event and has no parent, the event is ignored.

Each `=> $^` declaration costs one line in one dispatcher. The
mechanism is shallow on purpose: it does exactly what it says,
nothing more.

---

## Step 24 — Approval Chain (event forwarding via transition)

A document approval workflow that routes through reviewers based
on the document type. The key trick: when the request arrives,
the workflow needs to figure out which reviewer to use, transition
to that state, and then dispatch the request to the new state's
handler.

```frame
@@target python_3

@@system Approval {
    interface:
        submit(doc_type: str, content: str)
        approve()
        reject(reason: str)
        get_status(): str = "unknown"

    machine:
        $Triage {
            submit(doc_type: str, content: str) {
                if doc_type == "expense":
                    **-> => $ExpenseReview**
                elif doc_type == "policy":
                    **-> => $PolicyReview**
                else:
                    -> $Rejected("unknown type")
            }

            get_status(): str { @@:("triage") }
        }

        $ExpenseReview {
            $>() {
                print("expense reviewer assigned")
            }
            submit(doc_type: str, content: str) {
                self.content = content
                print(f"expense review starting: {len(content)} chars")
            }
            approve() { -> $Approved }
            reject(reason: str) { -> $Rejected(reason) }
            get_status(): str { @@:("expense review") }
        }

        $PolicyReview {
            $>() {
                print("policy reviewer assigned")
            }
            submit(doc_type: str, content: str) {
                self.content = content
                print(f"policy review starting: {len(content)} chars")
            }
            approve() { -> $Approved }
            reject(reason: str) { -> $Rejected(reason) }
            get_status(): str { @@:("policy review") }
        }

        $Approved {
            get_status(): str { @@:("approved") }
        }

        $Rejected(reason: str) {
            $>() {
                print(f"rejected: {reason}")
            }
            get_status(): str { @@:("rejected") }
        }

    domain:
        content: str = ""
}
```

`$Triage`'s `submit` handler does something we haven't seen
before: `-> => $ExpenseReview`. The arrow combination means
"transition to `$ExpenseReview` *and* re-dispatch the current
event to the new state's handler."

Without this, `$Triage` would have to either handle `submit`
itself (storing the document and triggering the transition some
other way) or transition to `$ExpenseReview` and lose the
`submit` event entirely. Neither matches the natural flow:
"figure out who reviews this, hand the document to them."

Compare to Step 23's `=> $^`. That construct dispatched an event
to a parent state without changing the current state.
`-> =>` does the opposite: changes the current state *and*
re-dispatches the event to it. The two solve different problems
and use different mechanisms.

### How the runtime forwards the event

The compartment needs to carry the event that should fire after
entry. A new field appears on the compartment:

```python
class ApprovalCompartment:
    def __init__(self, state):
        self.state = state
        self.state_args = []
        self.enter_args = []
        self.exit_args = []
        self.state_vars = {}
        self.parent_compartment = None
        **self.forward_event = None**
```

`forward_event` holds a FrameEvent that the kernel should
re-dispatch after entering the new state. `None` for normal
transitions; populated for `-> =>` transitions.

The handler that uses `-> =>` populates the field on the new
compartment before transitioning:

```python
def _s_Triage_hdl_user_submit(self, __e, compartment):
    doc_type = __e._parameters[0]
    content = __e._parameters[1]
    if doc_type == "expense":
        next_comp = self.__prepareEnter("ExpenseReview", [], [])
        **next_comp.forward_event = __e**
        self.__transition(next_comp)
        return
    elif doc_type == "policy":
        next_comp = self.__prepareEnter("PolicyReview", [], [])
        **next_comp.forward_event = __e**
        self.__transition(next_comp)
        return
    else:
        next_comp = self.__prepareEnter("Rejected", ["unknown type"], [])
        self.__transition(next_comp)
        return
```

The handler builds the destination compartment, sets
`forward_event` to the current event (`__e`), then calls
`__transition` and returns. The kernel will see the
`forward_event` and act on it.

The kernel's transition processing checks for the forward and
re-dispatches it after the entry cascade:

```python
def __kernel(self, __e):
    self.__router(__e)

    if self.__next_compartment is not None:
        next_compartment = self.__next_compartment
        self.__next_compartment = None

        self.__fire_exit_cascade()

        self.__compartment = next_compartment

        **if next_compartment.forward_event is None:**
            self.__fire_enter_cascade()
        **else:
            forward_event = next_compartment.forward_event
            next_compartment.forward_event = None
            self.__fire_enter_cascade()
            self.__router(forward_event)**
```

If `forward_event` is `None`, the kernel fires the enter cascade
normally — same as before. If `forward_event` is set, the kernel
extracts it (clearing the field so it doesn't trigger again),
fires the enter cascade, and then calls the router with the
saved event. The router routes through the new state's
dispatcher, which now has the chance to handle the event.

The order matters: enter cascade first, then forward. By the
time the new state sees the event, its `$>` handler has already
run. State variables are initialized; the state is fully set up.

### Trace a forwarded transition

Calling `submit("expense", "Receipt for travel...")` from
`$Triage`:

The wrapper builds the event with `_parameters = ["expense",
"Receipt for travel..."]`. The router calls `_state_Triage`'s
dispatcher, which calls `_s_Triage_hdl_user_submit`. The handler
binds `doc_type = "expense"` and `content = "Receipt for
travel..."`, takes the first branch, builds an `$ExpenseReview`
compartment, sets `next_comp.forward_event = __e`, calls
`__transition`, and returns.

The kernel sees `__next_compartment` is set. Exit cascade runs
on `$Triage` — no handler, nothing prints. The kernel switches
`__compartment` to the new `$ExpenseReview` compartment. It sees
`forward_event` is set, so it pulls it out, clears the field,
fires the enter cascade ($ExpenseReview.$> prints "expense
reviewer assigned"), then calls the router with the saved event.

The router now routes the original `submit` event with
`__compartment` being the `$ExpenseReview` compartment. It calls
`_state_ExpenseReview`'s dispatcher, which matches `submit` and
calls `_s_ExpenseReview_hdl_user_submit`. The handler binds
`doc_type` and `content` again, stores `self.content = content`,
prints "expense review starting: ... chars".

The wrapper pops the context. The caller's `submit()` call is
done. The system is in `$ExpenseReview`, the document is stored,
and both states had a chance to print.

### `-> =>` versus `=> $^`

These two arrow forms look similar but solve different problems:

| Construct | Changes state? | Re-dispatches event? | Where used? |
|---|---|---|---|
| `=> $^` | No | Yes (to parent) | In a state's dispatcher fall-through |
| `-> =>` | Yes | Yes (to new state) | In a transition |

`=> $^` is for "this state doesn't handle this; the parent does."
The system stays where it is. Used for HSM event delegation.

`-> =>` is for "transition to a new state, and let it handle this
event." The system moves; the event moves with it. Used for
dispatch-and-handoff patterns like the approval chain.

Both rely on the compartment carrying just enough information for
the kernel to do the right thing. `=> $^` doesn't need any
runtime support beyond `parent_compartment` (which already exists
for the cascade). `-> =>` needs the `forward_event` field on the
compartment, populated at the transition site, consumed by the
kernel after entry.

---

## Step 25 — Session Persistence (`@@persist` and restore)

A counter that survives process restarts. Save its state to a
blob; restart the process; restore from the blob; the counter
keeps counting from where it left off.

```frame
@@target python_3

**@@persist**
@@system Counter {
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

`@@persist` at the top is the only source change. Everything else
is the system as it would normally be written. This single
declaration triggers two new methods on the generated class.

`@@persist` doesn't add any new runtime mechanism in the kernel
or the dispatch pipeline. The system runs identically with or
without it. What `@@persist` adds is two methods that walk the
existing data structures and serialize them: `save_state()`
returns a blob, and `restore_state(blob)` rebuilds the system
from one.

### What gets saved

Persistence serializes the data that defines the system's
position. It doesn't serialize transient call state.

| What | Saved? | Why |
|---|---|---|
| Domain (`self.total`) | Yes | System-lifetime state |
| State variables (`compartment.state_vars`) | Yes — per layer | State-lifetime state |
| Current state name (and HSM chain) | Yes | Where the system is |
| State stack (saved compartments) | Yes | Pending pop targets |
| State args, enter args, exit args (per compartment) | Yes | Each compartment's parameters |
| `parent_compartment` pointers | No | Rebuilt from `_HSM_CHAIN` |
| `_context_stack` | No | Empty between calls |
| `__next_compartment` | No | Null between calls |
| `forward_event` on compartments | No | In-flight; null between calls |

The "between calls" rule shapes the cutoff. A system at rest —
one not currently dispatching an event — has empty
`_context_stack`, null `__next_compartment`, and null
`forward_event` everywhere. These exist only during a call.
Persistence assumes save happens at rest, and restore brings the
system back to a rest state.

### The canonical format

Every backend produces the same structure when serializing,
regardless of target language:

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

A Python backend produces this as JSON or MessagePack; a Java
backend produces it via Jackson; a Rust backend uses serde. The
output is interchangeable across backends — a Python system's
saved blob can be restored by a Java system compiled from the
same Frame source.

This is the cross-host migration property the runtime promises:
the format is Frame's contract, not any backend's.

### Save

`save_state()` walks the system and produces the blob:

```python
**def save_state(self) -> str:
    chain = []
    comp = self.__compartment
    while comp is not None:
        chain.append(comp.state)
        comp = comp.parent_compartment

    compartments = []
    comp = self.__compartment
    while comp is not None:
        compartments.append({
            "state": comp.state,
            "state_args": list(comp.state_args),
            "state_vars": dict(comp.state_vars),
            "enter_args": list(comp.enter_args),
            "exit_args": list(comp.exit_args),
        })
        comp = comp.parent_compartment

    state_stack = []
    for saved in self._state_stack:
        layer = []
        comp = saved
        while comp is not None:
            layer.append({
                "state": comp.state,
                "state_args": list(comp.state_args),
                "state_vars": dict(comp.state_vars),
                "enter_args": list(comp.enter_args),
                "exit_args": list(comp.exit_args),
            })
            comp = comp.parent_compartment
        state_stack.append(list(reversed(layer)))

    blob = {
        "frame_version": "4.0",
        "schema_version": "1",
        "system_name": "Counter",
        "current_state": self.__compartment.state,
        "hsm_chain": list(reversed(chain)),
        "compartments": list(reversed(compartments)),
        "state_stack": state_stack,
        "domain": {"total": self.total},
    }
    return json.dumps(blob)**
```

The method walks `__compartment` and its `parent_compartment`
chain to capture the active HSM. It walks `_state_stack` for any
pushed compartments. It collects domain fields by name. The
blob is the entire saveable state in canonical form.

Both the active chain and each state stack layer are stored
root-first in the canonical format. The walks naturally produce
leaf-first lists (each starts at a leaf and follows
`parent_compartment` upward), so each list is reversed before
being added to the blob. Root-first ordering matches the natural
reading order and matches what `_HSM_CHAIN` uses.

### Restore

`restore_state(blob)` rebuilds the system from a blob:

```python
**def restore_state(self, blob_str: str):
    blob = json.loads(blob_str)

    # Rebuild domain
    self.total = blob["domain"]["total"]

    # Rebuild HSM chain using _HSM_CHAIN as source of truth
    leaf_state = blob["current_state"]
    expected_chain = _HSM_CHAIN[leaf_state]
    saved_chain = [c["state"] for c in blob["compartments"]]
    if saved_chain != expected_chain:
        raise RestoreError(
            f"saved chain {saved_chain} doesn't match "
            f"_HSM_CHAIN[{leaf_state}] = {expected_chain}"
        )

    by_state = {c["state"]: c for c in blob["compartments"]}
    previous = None
    for state_name in expected_chain:
        comp_data = by_state[state_name]
        comp = CounterCompartment(state_name)
        comp.state_args = list(comp_data["state_args"])
        comp.state_vars = dict(comp_data["state_vars"])
        comp.enter_args = list(comp_data["enter_args"])
        comp.exit_args = list(comp_data["exit_args"])
        comp.parent_compartment = previous
        previous = comp
    self.__compartment = previous

    # Rebuild state stack the same way
    self._state_stack = []
    for layer_data in blob["state_stack"]:
        layer_leaf = layer_data[-1]["state"]
        layer_chain = _HSM_CHAIN[layer_leaf]
        layer_by_state = {c["state"]: c for c in layer_data}
        previous = None
        for state_name in layer_chain:
            comp_data = layer_by_state[state_name]
            comp = CounterCompartment(state_name)
            comp.state_args = list(comp_data["state_args"])
            comp.state_vars = dict(comp_data["state_vars"])
            comp.enter_args = list(comp_data["enter_args"])
            comp.exit_args = list(comp_data["exit_args"])
            comp.parent_compartment = previous
            previous = comp
        self._state_stack.append(previous)

    self.__next_compartment = None**
```

The method parses the blob, validates that each saved chain
matches the topology in `_HSM_CHAIN`, rebuilds the compartments
in root-to-leaf order from the table, links `parent_compartment`
pointers (which weren't serialized but are determined by the
chain), and restores the domain.

`@@persist` emits a `RestoreError` exception class (or the
target's idiomatic equivalent — `RestoreException` in Java, a
`Result::Err` variant in Rust, etc.) when restore detects a
structural mismatch between the saved blob and the current
system's topology. It's the only error class `@@persist` adds;
the `save_state` and `restore_state` methods plus this one
exception are the entire `@@persist` surface.

`_HSM_CHAIN` is the source of truth on restore, not the saved
chain. This is deliberate. If the destination's Frame source has
a different HSM topology than the source's — say `$Cooling` was
moved under a different parent in a later release — the saved
blob's chain won't match `_HSM_CHAIN[leaf]` and restore raises
rather than silently producing a system with wrong topology. The
saved `compartments` list provides the per-compartment data
(state vars, args), but the chain structure comes from the
running code.

`hsm_chain` in the blob (the standalone string list) is
informational. Diagnostic tools can read it without instantiating
a system; restore doesn't need it because it has `_HSM_CHAIN`.

### What restore deliberately doesn't do

Restore does *not* fire `$>`. Restoring isn't entering — the
system was already in this state when saved; bringing it back
shouldn't trigger lifecycle handlers as if the state were being
entered fresh.

Compare:

| Operation | `$>` runs? | State variables |
|---|---|---|
| `-> $State` | Yes | Reset, then `$>` initializes |
| `-> pop$` | Yes | Preserved (guard skips re-init) |
| `restore_state(blob)` | No | Restored from blob |

A system constructed normally calls `$>` on the start state from
the constructor. A system restored from a blob doesn't — its
constructor still runs (which builds the basic class skeleton),
but `restore_state()` overwrites `__compartment` with the
serialized one, and no lifecycle handler fires.

The user's host code is responsible for choosing between fresh
construction and restore at startup. Typical pattern:

```python
try:
    blob = open("counter.state").read()
    counter = Counter()
    counter.restore_state(blob)
except FileNotFoundError:
    counter = Counter()  # fresh start
```

### Cross-host migration

The canonical format is the same regardless of target backend.
This means a system saved on one host can be loaded on another:

- A Java service serializes its state and sends the blob to a
  worker process compiled from the same Frame source but running
  in Rust. The Rust process restores from the blob and continues.
- A Python development environment saves a system's state mid-
  test. A Go production service (same Frame source, different
  backend) loads the blob to reproduce the failure conditions.
- A Kubernetes pod running a Frame-generated state machine is
  drained. Its state is saved, the pod terminates, a new pod
  spins up (possibly compiled for a different OS/arch), and
  resumes from the saved state.

This works because the format is Frame's contract, not any
backend's. The state machine moves between hosts, preserving its
logical state.

The property depends on source agreement. Both endpoints must be
compiled from the same Frame source — same states, same HSM
topology, same domain fields, same state variables. The
`_HSM_CHAIN` validation on restore catches topology drift; the
`schema_version` field in the blob is reserved for future
versioning, but v4 has no protocol-version negotiation. If the
destination's Frame source has diverged from the source's,
restore may succeed structurally but produce wrong behavior, or
fail with a topology mismatch. Cross-host migration in v4
assumes coordinated deployment: ship the same Frame source to
every host that might handle a blob.

### What `@@persist` doesn't change

The kernel doesn't change. The router doesn't change. Dispatchers
don't change. Compartment fields don't change. State variable
storage doesn't change. Everything we built up over the previous
24 steps still works exactly the same way.

`@@persist` adds two methods. That's the entire mechanism. The
runtime that supports persistence is the same runtime that
supports any other Frame system — persistence is a property of
the data, not the dispatch.

This is the property the doc has been building toward: a state
machine's behavior is fully captured by its source. The runtime
makes that data observable, restorable, and portable, but doesn't
change the meaning of the source. Two compiled versions of the
same system on different hosts are the same machine.