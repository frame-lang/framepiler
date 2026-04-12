# Getting Started with Frame

## Table of Contents

- [Introduction](#introduction)
    - [What is Frame?](#what-is-frame)
    - [Frame Lives Inside Your Code](#frame-lives-inside-your-code)
    - [Why Machines?](#why-machines)
    - [What the Framepiler Does](#what-the-framepiler-does)
    - [Target Language](#target-language)
    - [File Extensions](#file-extensions)
- [Your First System](#your-first-system)
    - [Install](#install)
    - [Write](#write)
    - [Transpile](#transpile)
    - [Examine the Output](#examine-the-output)
    - [Run](#run)
    - [The Anatomy of a System](#the-anatomy-of-a-system)
- [States and Handlers](#states-and-handlers)
    - [A Light Switch](#a-light-switch)
    - [How Dispatch Works](#how-dispatch-works)
    - [Return Values](#return-values)
    - [A More Interesting Example](#a-more-interesting-example)
    - [Deferred Transitions](#deferred-transitions)
    - [Unhandled Events](#unhandled-events)
- [Events and the Interface](#events-and-the-interface)
    - [Interface Methods](#interface-methods)
    - [Parameters](#parameters)
    - [Typed Parameters](#typed-parameters)
    - [Return Values](#return-values-1)
    - [Multiple Parameters and Returns](#multiple-parameters-and-returns)
    - [Events That Change Behavior by State](#events-that-change-behavior-by-state)
- [Actions](#actions)
    - [The Problem](#the-problem)
    - [Actions to the Rescue](#actions-to-the-rescue)
    - [What Actions Can Do](#what-actions-can-do)
    - [What Actions Cannot Do](#what-actions-cannot-do)
    - [Actions vs Operations vs Native Functions](#actions-vs-operations-vs-native-functions)
- [Variables](#variables)
    - [Domain Variables](#domain-variables)
    - [State Variables](#state-variables)
    - [When to Use Which](#when-to-use-which)
    - [State Parameters](#state-parameters)
    - [System Parameters](#system-parameters)
- [Transitions in Depth](#transitions-in-depth)
    - [Enter and Exit Events](#enter-and-exit-events)
    - [Enter and Exit Parameters](#enter-and-exit-parameters)
    - [The Full Transition Form](#the-full-transition-form)
    - [Event Forwarding](#event-forwarding)
    - [State Stack and History](#state-stack-and-history)
    - [Transition Summary](#transition-summary)
- [Hierarchical State Machines](#hierarchical-state-machines)
    - [The Problem](#the-problem-1)
    - [Parent States](#parent-states)
    - [Explicit Forwarding](#explicit-forwarding)
    - [Forwarding Within a Handler](#forwarding-within-a-handler)
    - [Default Forward vs Selective Handling](#default-forward-vs-selective-handling)
    - [A Complete Example](#a-complete-example)
    - [Rules](#rules)
- [Self Interface Calls](#self-interface-calls)
    - [Calling Your Own Interface](#calling-your-own-interface)
    - [Context Isolation](#context-isolation)
    - [State Sensitivity](#state-sensitivity)
- [Async](#async)
    - [Declaring Async Methods](#declaring-async-methods)
    - [How Async Propagates](#how-async-propagates)
    - [Language Support](#language-support)
    - [Two-Phase Initialization](#two-phase-initialization)
- [Advanced Topics](#advanced-topics)
    - [System Context](#system-context)
    - [Operations](#operations)
    - [Persistence](#persistence)
    - [Codegen Options](#codegen-options)
    - [Multi-System Files](#multi-system-files)
    - [GraphViz Visualization](#graphviz-visualization)

---

## Introduction

### What is Frame?

Frame is a language for specifying *automata* — state machines, pushdown automata, and Turing machines. To simplify nomenclature, Frame documentation refers to the automata it creates — of any kind — as *machines*.

Frame's unit of definition for a single machine is a *system*. Here's one:

```
@@system TrafficLight {
    interface:
        next()
    machine:
        $Green {
            next() { -> $Yellow }
        }
        $Yellow {
            next() { -> $Red }
        }
        $Red {
            next() { -> $Green }
        }
}
```

This defines a traffic light with three states. Calling `next()` transitions between them: Green to Yellow to Red to Green. The `->` operator means "transition to." That's the core of Frame — states, events, and transitions, expressed directly.

The framepiler generates a full implementation of this system in your target language — Python, TypeScript, Rust, C, and many others. The generated code is a plain class with no runtime dependencies: readable, debuggable, and ready to use.

### Frame Lives Inside Your Code

Frame is not a standalone language. It's designed to live *inside* your native source files, side by side with regular code.

Frame makes heavy use of a special token — `@@` — to mark Frame pragmas and constructs that should be preprocessed by the *framepiler* (Frame's compilation tool). Everything without `@@` passes through unchanged.

```
@@target python_3

import logging

logger = logging.getLogger(__name__)

@@system TrafficLight {
    interface:
        next()
    machine:
        $Green {
            next() { -> $Yellow }
        }
        $Yellow {
            next() { -> $Red }
        }
        $Red {
            next() { -> $Green }
        }
}

if __name__ == '__main__':
    light = @@TrafficLight()
    light.next()  # Green -> Yellow
    light.next()  # Yellow -> Red
    light.next()  # Red -> Green
```

The `@@target python_3` pragma tells the framepiler which language to generate. The `@@system` block is expanded into a Python class. The `@@TrafficLight()` construct instantiates the system — the framepiler expands it to the appropriate native constructor.

The `import`, the `logger`, the `if __name__` block — all native Python — pass through exactly as written. This is Frame's core design: **native code is the ocean, Frame systems are islands.** The framepiler only touches the islands.

### Why Machines?

Machines make certain kinds of programs dramatically simpler:

- **UI workflows** — login flows, wizards, form validation
- **Protocol handlers** — TCP connections, WebSocket sessions
- **Game logic** — character states, turn management
- **Device controllers** — hardware modes, sensor management
- **Business processes** — order fulfillment, approval chains

The pattern is the same: your system is in one of several *states*, it responds to *events* differently depending on which state it's in, and events can cause *transitions* between states.

You can implement this with if/else chains or switch statements, but they become tangled as complexity grows. Frame gives you a clean, declarative way to express the same logic.

### What the Framepiler Does

The framepiler (`framec`) reads your source file and:

1. Finds `@@system` blocks
2. Parses the Frame specification inside each block
3. Generates a full class with state dispatch, transitions, and lifecycle management
4. Passes all native code through unchanged
5. Writes the combined output

The generated code is readable, debuggable, and uses no runtime library. It's just a class in your target language.

### Target Language

The `@@target` directive inside the file is the authoritative declaration of which native language the file targets. The framepiler uses it to determine how to parse native code regions (string/comment syntax), which code generator to use, and what output to produce.

The `@@target` can be overridden by a CLI flag (`-l <language>`) or other configuration, but if neither is provided, the in-file `@@target` is what controls compilation.

### File Extensions

Frame source files conventionally use a target-specific extension:

| Target | Extension | Example |
|--------|-----------|---------|
| Python | `.fpy` | `traffic_light.fpy` |
| TypeScript | `.fts` | `traffic_light.fts` |
| Rust | `.frs` | `traffic_light.frs` |
| C | `.fc` | `traffic_light.fc` |
| Go | `.fgo` | `traffic_light.fgo` |
| Java | `.fjava` | `traffic_light.fjava` |

The file extension is a hint — nothing more. It helps editors and build tools recognize Frame files, but the framepiler does not use it to determine the target language. The `@@target` directive (or a CLI override) is what matters.

---

## Your First System

### Install

```bash
cargo install framec
```

Verify the installation:

```bash
framec --version
```

### Write

Create a file called `hello.fpy`:

```
@@target python_3

@@system Hello {
    interface:
        greet()

    machine:
        $Start {
            greet() {
                print("Hello from Frame!")
            }
        }
}
```

Let's break this down:

- **`@@target python_3`** — tells the framepiler to generate Python
- **`@@system Hello`** — declares a state machine called `Hello`
- **`interface:`** — the public API. `greet()` is a method callers can invoke
- **`machine:`** — the states. There's one state, `$Start`
- **`$Start`** — a state. The first state listed is always the starting state
- **`greet() { ... }`** — a handler. When `greet()` is called while in `$Start`, this code runs

### Transpile

```bash
framec hello.fpy
```

This writes the generated Python to stdout. To save it to a file:

```bash
framec hello.fpy -o hello.py
```

### Examine the Output

The generated `hello.py` contains a `Hello` class. Open it and take a look — the generated code is straightforward, with no magic and no runtime dependencies. You can read it, debug it, and step through it like any other Python code.

### Run

```bash
python3 hello.py
```

Nothing happens yet — we declared the class but didn't instantiate it. Add native Python code after the system block:

```
@@target python_3

@@system Hello {
    interface:
        greet()

    machine:
        $Start {
            greet() {
                print("Hello from Frame!")
            }
        }
}

if __name__ == '__main__':
    h = @@Hello()
    h.greet()
```

Transpile and run again:

```bash
framec hello.fpy -o hello.py && python3 hello.py
```

Output:

```
Hello from Frame!
```

The `if __name__` block is native Python — the framepiler passes it through unchanged.

### The Anatomy of a System

Every Frame system has the same structure:

```
@@system Name {
    operations:
        # Public methods that bypass the state machine 

    interface:
        # Public methods — how the outside world talks to this system

    machine:
        # States and their event handlers — the behavior

    actions:
        # Private helper methods 

    domain:
        # Instance variables
}
```

The sections must appear in this order: `operations:` -> `interface:` -> `machine:` -> `actions:` -> `domain:`. All sections are optional.

### Try It

Modify `hello.fpy` to add a second method `farewell()` that prints a goodbye message. Transpile and run it to verify.

---

## States and Handlers

A system's power comes from having multiple states, each responding to the same events differently.

### A Light Switch

```
@@target python_3

@@system LightSwitch {
    interface:
        toggle()
        status(): str = "unknown"

    machine:
        $Off {
            toggle() {
                -> $On
            }
            status(): str {
                @@:return = "off"
            }
        }

        $On {
            toggle() {
                -> $Off
            }
            status(): str {
                @@:return = "on"
            }
        }
}

if __name__ == '__main__':
    sw = @@LightSwitch()
    print(sw.status())  # "off"
    sw.toggle()
    print(sw.status())  # "on"
    sw.toggle()
    print(sw.status())  # "off"
```

Key points:

- **`$Off` and `$On`** are states. The `$` prefix identifies state names
- **The first state listed (`$Off`) is the start state**
- **`-> $On`** is a transition — it moves the system from the current state to `$On`
- **`toggle()` does different things depending on the current state** — that's the whole point

### How Dispatch Works

When you call `sw.toggle()`:

1. The system routes `toggle` to the current state's handler
2. If the current state is `$Off`, the `$Off` version of `toggle()` runs
3. If the current state is `$On`, the `$On` version runs
4. If a handler triggers a transition (`-> $State`), the system changes state after the handler finishes

Events that a state doesn't handle are silently ignored. If `$Off` doesn't have a `toggle()` handler, calling `toggle()` while in `$Off` simply does nothing.

### Return Values

The `status()` method shows how to return values from handlers:

```
status(): str = "unknown"
```

In the `interface:` block, `: str` declares the return type and `= "unknown"` sets the default return value. If the current state doesn't handle `status()`, the caller gets `"unknown"`.

Inside a handler, `@@:return` sets the return value:

```
status(): str {
    @@:return = "off"
}
```

### A More Interesting Example

Here's a turnstile — locked until you insert a coin, then it lets one person through and locks again:

```
@@target python_3

@@system Turnstile {
    interface:
        coin()
        push(): str = "blocked"

    machine:
        $Locked {
            coin() {
                -> $Unlocked
            }
            push(): str {
                @@:return = "locked - insert coin"
            }
        }

        $Unlocked {
            coin() {
                # Already unlocked, coin is wasted
            }
            push(): str {
                -> $Locked
                @@:return = "welcome"
            }
        }
}

if __name__ == '__main__':
    t = @@Turnstile()
    print(t.push())    # "locked - insert coin"
    t.coin()
    print(t.push())    # "welcome" (and locks again)
    print(t.push())    # "locked - insert coin"
```

Notice that a handler can both transition and set a return value. The transition is *deferred* — it happens after the handler finishes, so `@@:return = "welcome"` sets the value before the system moves to `$Locked`.

### Deferred Transitions

This is an important concept: **transitions don't happen immediately**. When a handler executes `-> $State`, the system records the target but doesn't switch yet. The transition is processed after the handler returns.

This means:

- Code after `->` in the same handler still executes in the current state
- You can set return values after a transition
- You can do cleanup work after triggering a transition

### Unhandled Events

If an event arrives and the current state has no handler for it, **the event is silently ignored**. This is by design — it means you only need to declare handlers for events you care about in each state.

```
$Locked {
    coin() {
        -> $Unlocked
    }
    # push() is not handled here — it returns the default "blocked"
}
```

Wait — that's not quite right. In the turnstile example above, `$Locked` *does* handle `push()`. If it didn't, `push()` would return the default value (`"blocked"`) declared in the interface.

### Try It

Build a `Door` system with three states: `$Closed`, `$Open`, and `$Locked`. It should support `open()`, `close()`, `lock()`, and `unlock()` — but you can only lock a closed door, and you can only open an unlocked door.

---

## Events and the Interface

The `interface:` block is how the outside world communicates with your state machine. Each method declared there becomes an event that gets routed to the current state.

### Interface Methods

```
interface:
    start()
    stop()
    set_speed(speed)
    get_status(): str = "unknown"
    calculate(a, b): int = 0
```

Each declaration specifies:

- **Method name** — becomes the event name
- **Parameters** — passed to handlers
- **Return type** (optional) — `: type` after the parameter list
- **Default return value** (optional) — `= value` after the return type

### Parameters

Interface methods can take parameters, and handlers receive them:

```
@@target python_3

@@system Greeter {
    interface:
        greet(name)

    machine:
        $Ready {
            greet(name) {
                print(f"Hello, {name}!")
            }
        }
}

if __name__ == '__main__':
    g = @@Greeter()
    g.greet("Alice")   # "Hello, Alice!"
    g.greet("Bob")     # "Hello, Bob!"
```

The parameter names in the handler must match the interface declaration. The values are native code — strings, numbers, objects, whatever your target language supports.

### Typed Parameters

You can add type annotations to parameters:

```
interface:
    set_position(x: float, y: float)
    send_message(to: str, body: str)
```

Type annotations are passed through to the generated code. Their exact semantics depend on your target language (enforced in TypeScript, advisory in Python, required in Rust/C).

### Return Values

To return values from a state machine, declare the return type and default in the interface:

```
interface:
    get_count(): int = 0
```

Then in handlers, use `@@:return`:

```
$Counting {
    get_count(): int {
        @@:return = self.count
    }
}
```

There are two forms for setting the return value on the context stack:

- **`@@:(expr)`** — the concise form (preferred). Sets the return value and can be followed by `return` on a separate line to exit the handler.
- **`@@:return = expr`** — the long form. Sets the return value in a single statement.

If the current state doesn't handle the event, the caller gets the default value (`0` in this case). Note: bare `return` exits the dispatch method but does NOT set the return value — always use `@@:(expr)` or `@@:return = expr` to set return values.

**String returns across languages**: When returning string literals with `@@:("value")`, the framepiler automatically wraps the literal for typed backends (Rust: `String::from("value")`, C++: `std::string("value")`). For computed strings, use the target language's string construction (e.g., `@@:(format!("hello {}", name))` for Rust, `@@:(std::string("hello ") + name)` for C++). All other languages accept bare string literals without wrapping.

### Multiple Parameters and Returns

```
@@target python_3

@@system Calculator {
    interface:
        add(a, b): int = 0
        multiply(a, b): int = 0
        get_last(): int = 0

    machine:
        $Ready {
            add(a, b): int {
                self.last_result = a + b
                @@:return = self.last_result
            }
            multiply(a, b): int {
                self.last_result = a * b
                @@:return = self.last_result
            }
            get_last(): int {
                @@:return = self.last_result
            }
        }

    domain:
        last_result: int = 0
}

if __name__ == '__main__':
    calc = @@Calculator()
    print(calc.add(3, 4))       # 7
    print(calc.multiply(5, 6))  # 30
    print(calc.get_last())      # 30
```

### Events That Change Behavior by State

The real power shows when different states handle the same event differently:

```
@@target python_3

@@system Player {
    interface:
        play()
        pause()
        get_state(): str = "unknown"

    machine:
        $Stopped {
            play() {
                print("Starting playback")
                -> $Playing
            }
            get_state(): str {
                @@:return = "stopped"
            }
        }

        $Playing {
            pause() {
                print("Pausing")
                -> $Paused
            }
            get_state(): str {
                @@:return = "playing"
            }
        }

        $Paused {
            play() {
                print("Resuming")
                -> $Playing
            }
            pause() {
                print("Stopping")
                -> $Stopped
            }
            get_state(): str {
                @@:return = "paused"
            }
        }
}
```

Notice:
- `$Stopped` ignores `pause()` — you can't pause something that isn't playing
- `$Playing` ignores `play()` — you can't play something already playing
- `$Paused` handles both — play resumes, pause stops entirely

This is the pattern: each state declares *only the events it cares about*. Everything else is silently ignored.

### Try It

Build a `Counter` system with `increment()`, `decrement()`, `reset()`, and `get_value(): int = 0`. Add a `$Frozen` state where increment and decrement are ignored but reset still works.

---

## Actions

Actions are private helper methods that keep your event handlers clean. They can access domain variables and interface context, but they cannot perform transitions or stack operations.

### The Problem

As handlers grow, they accumulate native code — logging, validation, API calls. This clutters the state machine logic:

```
$Processing {
    submit(order) {
        # 20 lines of validation...
        # 10 lines of logging...
        # 5 lines of notification...
        -> $Complete
    }
}
```

The transition (`-> $Complete`) is buried. The state machine's structure is hard to see.

### Actions to the Rescue

Move the native code into actions:

```
@@target python_3

@@system OrderProcessor {
    interface:
        submit(order)

    machine:
        $Idle {
            submit(order) {
                validate_order(order)
                log_submission(order)
                -> $Processing
            }
        }

        $Processing {
            # ...
        }

    actions:
        validate_order(order) {
            if not order.get("item"):
                raise ValueError("Order must have an item")
        }

        log_submission(order) {
            print(f"Order submitted: {order['item']}")
        }
}
```

The handler is now readable — you can see the three steps and the transition at a glance.

### What Actions Can Do

Actions are native code methods. They can:

- Accept parameters
- Return values
- Access domain variables (via `self` in Python, `this` in TypeScript, etc.)
- Call other actions
- Call any native code
- Access interface context via `@@:params.x`, `@@:return`, `@@:event`, `@@:data.key`
- Call interface methods on the system via `@@:self.method()`

```
actions:
    calculate_tax(amount): float {
        return amount * self.tax_rate
    }

    format_receipt(item, amount) {
        tax = calculate_tax(amount)
        total = amount + tax
        return f"{item}: ${total:.2f}"
    }
```

### What Actions Cannot Do

Actions are deliberately restricted from Frame constructs that affect state:

- `-> $State` — transitions
- `-> => $State` — transition with forwarding
- `=> $^` — parent forwarding
- `push$` / `pop$` — stack operations
- `$.varName` — state variables

These restrictions exist because actions don't have state context — they're called from handlers but don't know which state is active. All state-related decisions belong in handlers.

If you need a state variable's value in an action, pass it as a parameter:

```
$Counting {
    report() {
        print_count($.count)
    }
}

actions:
    print_count(n) {
        print(f"Current count: {n}")
    }
```

### Actions vs Operations vs Native Functions

Frame has three kinds of methods besides event handlers. Here's when to use each:

- **Action**: private helper that needs access to domain variables. Cannot trigger transitions or access state variables. Called from handlers.
- **Operation**: public method that bypasses the state machine entirely. Good for utility methods, version info, or debug introspection. Declared in the `operations:` section (see [Advanced Topics](#advanced-topics)).
- **Native function**: a regular function outside the system. No access to domain variables. Use for pure computation or code shared across systems.

The key distinction: actions are *private* helpers for handlers, operations are *public* methods that skip the state machine, and native functions live outside the system entirely.

### Try It

Take the `Turnstile` from the States and Handlers section and add actions for `log_entry()` and `log_rejection()`. Call them from the appropriate handlers.

---

## Variables

Frame provides three kinds of variables, each with different scope and lifetime.

### Domain Variables

Domain variables are declared in the `domain:` block. They're instance fields that persist for the lifetime of the system, across all state transitions.

```
@@target python_3

@@system Counter {
    interface:
        increment()
        get_count(): int = 0

    machine:
        $Counting {
            increment() {
                self.count = self.count + 1
            }
            get_count(): int {
                @@:return = self.count
            }
        }

    domain:
        count: int = 0
}
```

Domain variables are **native code** — the framepiler passes them through to the generated class as instance fields. The syntax matches your target language:

```
# Python
domain:
    count: int = 0
    name: str = "default"

# TypeScript
domain:
    count: number = 0
    name: string = "default"

# Rust
domain:
    count: i32 = 0
    name: String = String::from("default")
```

Access them using your language's normal syntax (`self.count` in Python, `this.count` in TypeScript, etc.).

### State Variables

State variables are scoped to a single state. They're declared at the top of a state block with the `$.` prefix:

```
$Retrying {
    $.attempts: int = 0
    $.last_error = None

    submit(data) {
        $.attempts = $.attempts + 1
        result = try_submit(data)
        if result.ok:
            -> $Done
        else:
            $.last_error = result.error
            if $.attempts >= 3:
                -> $Failed
    }

    get_attempts(): int {
        @@:return = $.attempts
    }
}
```

Key behaviors:

- **Scoped to one state** — `$.attempts` only exists in `$Retrying`. Other states can't see it.
- **Reset on normal transition** — when you enter `$Retrying` via `-> $Retrying`, state variables reset to their declared initial values (`0` and `None` here).
- **Preserved on history transition** — when you enter via `-> pop$`, state variables keep their values from when the state was pushed. More on this in [Transitions in Depth](#transitions-in-depth).

The `$.` prefix is how you read and write state variables.

### When to Use Which

| Variable | Scope | Lifetime | Use for |
|----------|-------|----------|---------|
| Domain (`self.x`) | All states | System lifetime | Shared data, configuration, accumulated results |
| State (`$.x`) | One state | Until next transition into that state | Retry counts, per-state buffers, temporary state |

A good rule: if multiple states need it, use domain. If only one state needs it and it should reset each time you enter that state, use a state variable.

### State Parameters

You can pass arguments to a state during transition:

```
@@target python_3

@@system Router {
    interface:
        navigate(path)
        get_title(): str = ""

    machine:
        $Home {
            navigate(path) {
                if path == "/settings":
                    -> $Page("Settings", "/settings")
                elif path == "/profile":
                    -> $Page("Profile", "/profile")
            }
            get_title(): str {
                @@:return = "Home"
            }
        }

        $Page {
            $.title = ""
            $.path = ""

            navigate(path) {
                if path == "/":
                    -> $Home
                else:
                    -> $Page(path.title(), path)
            }
            get_title(): str {
                @@:return = $.title
            }
        }
}
```

When you write `-> $Page("Settings", "/settings")`, the arguments initialize the target state's variables — the first argument maps to the first declared `$.` variable, the second to the second, and so on.

### System Parameters

You can also pass arguments when constructing a system. System parameters can initialize domain variables:

```
@@target python_3

@@system Server(port: int, host: str) {
    interface:
        start()

    machine:
        $Idle {
            start() {
                print(f"Starting on {self.host}:{self.port}")
                -> $Running
            }
        }
        $Running {
        }

    domain:
        port: int = port
        host: str = host
}

if __name__ == '__main__':
    s = @@Server(3000, "0.0.0.0")
    s.start()  # "Starting on 0.0.0.0:3000"
```

System parameters in the header (`port: int, host: str`) become constructor arguments. When a domain variable's initializer references a system parameter by name (`port: int = port`), the constructor assigns the parameter value at construction time. If no system parameter of that name exists, the literal default is used instead.

Frame V4 also supports **state parameters** and **enter parameters** in the system header, prefixed with sigils:

| Sigil | Kind | Stored in | Read by |
|-------|------|-----------|---------|
| `$(name: type)` | State | `compartment.state_args` | Start state handlers via bare `name` |
| `$>(name: type)` | Enter | `compartment.enter_args` | Start state `$>` handler via bare `name` |
| (bare) | Domain | `self.field` | Any handler via `self.field` |

```
@@system Robot($(x: int), $>(msg: str), name: str) {
    ...
}
r = @@Robot($(42), $>("hello"), "R2")
```

See the [Frame Language Reference](frame_language.md#passing-system-parameters) for the complete parameter syntax.

### Try It

Build a `Stopwatch` with states `$Stopped` and `$Running`. Use a domain variable for elapsed time (persists across stops/starts) and a state variable in `$Running` for the start timestamp.

---

## Transitions in Depth

You've already used simple transitions (`-> $State`). Frame supports a rich set of transition forms with enter/exit events, parameters, forwarding, and history.

### Enter and Exit Events

When a transition happens, Frame fires lifecycle events:

1. The current state's **exit handler** (`<$`) runs
2. The system switches to the new state
3. The new state's **enter handler** (`$>`) runs

```
@@target python_3

@@system Connection {
    interface:
        connect()
        disconnect()

    machine:
        $Disconnected {
            $>() {
                print("Ready to connect")
            }
            connect() {
                -> $Connected
            }
        }

        $Connected {
            $>() {
                print("Connection established")
            }
            <$() {
                print("Cleaning up connection")
            }
            disconnect() {
                -> $Disconnected
            }
        }
}

if __name__ == '__main__':
    c = @@Connection()       # prints "Ready to connect"
    c.connect()            # prints "Connection established"
    c.disconnect()         # prints "Cleaning up connection"
                           # then "Ready to connect"
```

The enter handler (`$>`) is the natural place for initialization. The exit handler (`<$`) is for cleanup. Both are optional.

### Enter and Exit Parameters

You can pass arguments to enter and exit handlers through transitions:

```
$Idle {
    start(config) {
        -> (config) $Running
    }
}

$Running {
    $>(config) {
        print(f"Starting with config: {config}")
    }

    stop(reason) {
        (reason) -> $Idle
    }

    <$(reason) {
        print(f"Stopping because: {reason}")
    }
}
```

The syntax:

| Form | Meaning |
|------|---------|
| `-> (args) $State` | Pass `args` to the target's `$>` handler |
| `(args) -> $State` | Pass `args` to the current state's `<$` handler |
| `(exit_args) -> (enter_args) $State` | Both |

Parameters are positional — the first argument maps to the first parameter of the handler.

### The Full Transition Form

A transition can carry exit args, enter args, state args, and a label:

```
(exit_args) -> (enter_args) "label" $State(state_args)
```

- **Exit args**: passed to current state's `<$` handler
- **Enter args**: passed to target state's `$>` handler
- **Label**: a string for diagram generation (no runtime effect)
- **State args**: initialize the target state's variables

You rarely need all of these at once, but they compose freely.

### Event Forwarding

Sometimes you want the target state to handle the *same event* that triggered the transition. This is event forwarding:

```
$Connecting {
    receive(data) {
        # We got data while still connecting — transition
        # to Ready and let it handle this data
        -> => $Ready
    }
}

$Ready {
    receive(data) {
        process(data)
    }
}
```

The `-> =>` syntax means: transition to `$Ready`, and after its `$>` handler runs, forward the `receive(data)` event to it. The target state sees the event as if it were called directly.

### State Stack and History

Frame has a built-in state stack for saving and restoring states. This enables patterns like modal dialogs, subroutine states, and undo.

#### Push

`push$` saves the current state (including all state variables) onto the stack:

```
$Normal {
    help() {
        push$
        -> $HelpMode
    }
}
```

#### Pop

`-> pop$` transitions to whatever state was last pushed:

```
$HelpMode {
    done() {
        -> pop$   # Returns to $Normal (or wherever we came from)
    }
}
```

The critical difference from a normal transition: **state variables are restored**. If `$Normal` had `$.count = 5` when it was pushed, `$.count` will be `5` when popped back — not reset to its initial value.

#### Example: Subroutine State

```
@@target python_3

@@system Editor {
    interface:
        type_char(ch)
        enter_search()
        exit_search()
        get_mode(): str = ""

    machine:
        $Editing {
            $.buffer = ""

            type_char(ch) {
                $.buffer = $.buffer + ch
            }
            enter_search() {
                push$
                -> $Searching
            }
            get_mode(): str {
                @@:return = "editing"
            }
        }

        $Searching {
            $.query = ""

            type_char(ch) {
                $.query = $.query + ch
            }
            exit_search() {
                -> pop$   # Back to $Editing with buffer intact
            }
            get_mode(): str {
                @@:return = "searching"
            }
        }
}

if __name__ == '__main__':
    e = @@Editor()
    e.type_char("H")
    e.type_char("i")
    e.enter_search()       # push $Editing, go to $Searching
    e.type_char("f")       # types into search query, not buffer
    e.exit_search()        # pop back to $Editing — buffer still has "Hi"
    e.type_char("!")       # buffer is now "Hi!"
```

### Transition Summary

| Syntax | Effect |
|--------|--------|
| `-> $State` | Simple transition |
| `-> $State(args)` | Transition with state arguments |
| `-> (args) $State` | Transition with enter arguments |
| `(args) -> $State` | Transition with exit arguments |
| `-> => $State` | Transition with event forwarding |
| `push$` | Save current state to stack |
| `-> pop$` | Restore last saved state |
| `-> "label" $State` | Labeled transition (for diagrams) |

### Try It

Build a `Wizard` with states `$Step1`, `$Step2`, `$Step3`, and `$Review`. Use `push$` before each forward step and `-> pop$` for the "back" button, so users can go back and their form data (state variables) is preserved.

---

## Hierarchical State Machines

When state machines grow, you'll find states that share common behavior. Hierarchical state machines (HSM) let child states delegate events to parent states, reducing duplication.

### The Problem

Imagine a media player where every state needs to handle `get_status()` and `emergency_stop()`:

```
$Playing {
    pause()     { -> $Paused }
    get_status(): str { @@:return = "playing" }
    emergency_stop() { cleanup(); -> $Stopped }
}

$Paused {
    play()      { -> $Playing }
    get_status(): str { @@:return = "paused" }
    emergency_stop() { cleanup(); -> $Stopped }
}

$Buffering {
    ready()     { -> $Playing }
    get_status(): str { @@:return = "buffering" }
    emergency_stop() { cleanup(); -> $Stopped }
}
```

`emergency_stop()` is duplicated in every state. If you add a new state, you have to remember to include it.

### Parent States

Declare a parent with `=> $ParentState` after the state name:

```
@@target python_3

@@system MediaPlayer {
    interface:
        play()
        pause()
        stop()
        get_status(): str = "unknown"

    machine:
        $Active {
            stop() {
                print("Stopping")
                -> $Stopped
            }
        }

        $Playing => $Active {
            pause() {
                -> $Paused
            }
            get_status(): str {
                @@:return = "playing"
            }
        }

        $Paused => $Active {
            play() {
                -> $Playing
            }
            get_status(): str {
                @@:return = "paused"
            }
        }

        $Stopped {
            play() {
                -> $Playing
            }
            get_status(): str {
                @@:return = "stopped"
            }
        }
}
```

`$Playing` and `$Paused` are children of `$Active`. But there's a catch — **events don't automatically forward to the parent**.

### Explicit Forwarding

Frame uses **explicit forwarding**. A child state must explicitly delegate events to its parent with `=> $^`:

```
$Playing => $Active {
    pause() {
        -> $Paused
    }
    get_status(): str {
        @@:return = "playing"
    }
    => $^    # Forward everything else to $Active
}
```

The bare `=> $^` at the end of a state is a **default forward** — any event not handled by `$Playing` gets sent to `$Active`. So `stop()` will be handled by `$Active`'s handler.

Without `=> $^`, unhandled events are silently ignored, even if the parent has a handler for them. This is intentional — it gives you full control over what gets forwarded.

### Forwarding Within a Handler

You can also forward from inside a specific handler:

```
$Playing => $Active {
    pause() {
        log_pause()
        => $^    # Let parent handle this too
    }
}
```

Here, `$Playing` does some work on `pause()` and then forwards it to the parent. The parent's `pause()` handler (if any) will run next.

`=> $^` can appear anywhere in a handler, not just at the end.

### Default Forward vs Selective Handling

There are two common patterns:

**Default forward** — handle some events, forward the rest:

```
$Child => $Parent {
    specific_event() {
        # Handle locally
    }
    => $^    # Forward everything else
}
```

**Selective forward** — handle some events, forward specific others:

```
$Child => $Parent {
    event_a() {
        # Handle locally only
    }
    event_b() {
        # Handle locally, then forward
        => $^
    }
    # event_c is neither handled nor forwarded — ignored
}
```

### A Complete Example

```
@@target python_3

@@system Appliance {
    interface:
        power_on()
        power_off()
        set_mode(mode)
        get_info(): str = ""

    machine:
        $Base {
            power_off() {
                print("Powering off")
                -> $Off
            }
            get_info(): str {
                @@:return = "appliance"
            }
        }

        $Off {
            power_on() {
                print("Powering on")
                -> $Idle
            }
            get_info(): str {
                @@:return = "off"
            }
        }

        $Idle => $Base {
            set_mode(mode) {
                if mode == "turbo":
                    -> $Turbo
            }
            get_info(): str {
                @@:return = "idle"
            }
            => $^
        }

        $Turbo => $Base {
            set_mode(mode) {
                if mode == "normal":
                    -> $Idle
            }
            get_info(): str {
                @@:return = "turbo"
            }
            => $^
        }
}

if __name__ == '__main__':
    a = @@Appliance()           # starts in $Off (first state)
    a.power_on()              # -> $Idle
    print(a.get_info())       # "idle" (handled by $Idle)
    a.set_mode("turbo")       # -> $Turbo
    a.power_off()             # Forwarded to $Base -> $Off
    print(a.get_info())       # "off"
```

Both `$Idle` and `$Turbo` inherit `power_off()` from `$Base` through `=> $^`. Without the default forward, `power_off()` would be ignored in those states.

### Rules

- A state can have at most one parent
- Parent chains can't form cycles (`$A => $B => $A` is an error)
- `=> $^` only works in states that have a parent (error E403 otherwise)
- Parent states can themselves have parents (multi-level HSM)
- The parent state doesn't need to be a state the system ever transitions *to* — it can be an abstract handler collection

### Try It

Build a `Form` system with a parent state `$Validated` that handles `validate(): bool`. Create child states `$NameEntry` and `$EmailEntry` that each handle their own input but forward validation to the parent.

---

## Self Interface Calls

Sometimes a handler or action needs to call one of the system's own interface methods. Frame provides `@@:self.method()` for this purpose.

### Calling Your Own Interface

```
@@target python_3

@@system Sensor {
    interface:
        calibrate()
        reading(): float = 0.0

    machine:
        $Active {
            calibrate() {
                baseline = @@:self.reading()
                self.offset = baseline * -1
            }
            reading(): float {
                @@:(self.raw_value + self.offset)
            }
        }

    domain:
        raw_value: float = 0.0
        offset: float = 0.0
}
```

`@@:self.reading()` dispatches through the full kernel pipeline — it constructs a FrameEvent, pushes a context, routes through the current state's handler, and returns the result. It's identical to an external caller invoking `sensor.reading()`.

Why not just use `self.reading()` (native Python)? You can — it works the same way mechanically. But `@@:self.reading()` gives the transpiler visibility: it validates the method exists, checks argument counts, and enables tracing and debugging integration. It also makes the self-call portable across all 17 target languages without worrying about native self-reference syntax.

### Context Isolation

A self-call is a reentrant dispatch. Each call gets its own context on the context stack:

```
$Processing {
    analyze() {
        # @@:event == "analyze"
        status = @@:self.get_status()
        # Inside get_status handler: @@:event == "get_status"
        # Back here: @@:event == "analyze" (restored)
        print(f"Status during analysis: {status}")
    }

    get_status(): str {
        @@:("processing")
    }
}
```

The calling handler's `@@:event`, `@@:params`, `@@:return`, and `@@:data` are all preserved across the self-call. The called handler sees its own isolated context.

### State Sensitivity

Because `@@:self.method()` goes through the kernel, the handler that executes depends on the **current state at the time of the call**. If a transition has been deferred, the self-call dispatches to the new state's handler:

```
$Calibrating {
    run_calibration() {
        // Self-call before transition — dispatches to $Calibrating's handler
        baseline = @@:self.reading()    // returns "raw"

        // Self-call in an action also dispatches to current state
        self.do_calibration()

        -> $Ready
    }
    reading(): str {
        @@:("raw")
    }
}

$Ready {
    reading(): str {
        @@:("calibrated")
    }
}
```

After `run_calibration()` returns, the system transitions to `$Ready`. A subsequent external call to `reading()` would dispatch to `$Ready`'s handler and return `"calibrated"`.

### Try It

Build a `HealthMonitor` system with `check()` and `is_healthy(): bool`. Have the `check()` handler call `@@:self.is_healthy()` and transition to `$Degraded` if the result is false.

---

## Async

Some state machines need to do asynchronous work — network calls, file I/O, timers. Frame supports `async` declarations that generate async/await code in languages that support it.

### Declaring Async Methods

Add `async` before interface methods, actions, or operations:

```
interface:
    async connect(url: str)
    async receive(): Message
    get_state(): str          # This one stays sync
```

### How Async Propagates

If *any* interface method is declared `async`, the **entire system** becomes async. All generated methods — including ones you declared as sync — will be async in the output.

This means callers must `await` every method on an async system, even `get_state()` above. This is a consequence of how state machines dispatch events internally: the system can't know at compile time which handler will run, so it must assume any call might be async.

Sync methods on an async system still work correctly — awaiting a synchronous function is a no-op in most languages.

### Example

```
@@target python_3

@@system HttpClient {
    interface:
        async fetch(url: str): str = ""
        get_last_url(): str = ""

    machine:
        $Idle {
            async fetch(url: str): str {
                self.last_url = url
                response = await http_get(url)
                -> $Done
                @@:return = response
            }
            get_last_url(): str {
                @@:return = self.last_url
            }
        }

        $Done {
            async fetch(url: str): str {
                self.last_url = url
                response = await http_get(url)
                @@:return = response
            }
            get_last_url(): str {
                @@:return = self.last_url
            }
        }

    actions:
        async http_get(url: str): str {
            import aiohttp
            async with aiohttp.ClientSession() as session:
                async with session.get(url) as resp:
                    return await resp.text()
        }

    domain:
        last_url: str = ""
}
```

The generated Python code uses `async def` for all dispatch methods and `await` for internal calls.

### Language Support

| Language | Async Support | Mechanism |
|----------|--------------|-----------|
| Python | Yes | `async def` / `await` |
| TypeScript | Yes | `async` / `await`, `Promise<T>` |
| Rust | Yes | `async fn` / `.await` |
| C | No | Warning emitted, `async` ignored |
| Go | Not needed | Goroutines handle concurrency without coloring |
| Java 21+ | Not needed | Virtual threads handle concurrency without coloring |

Languages like Go and Java don't need async/await — their concurrency models are "one-color," meaning any function can do concurrent work without special syntax. The `async` keyword is simply ignored for these targets.

### Two-Phase Initialization

Constructors can't be async in most languages. If your start state's enter handler (`$>`) needs to do async work, Frame generates a two-phase init:

1. The constructor creates the system and sets the initial state (sync)
2. A generated `init()` method fires the enter event (async)

```python
# Usage:
client = @@HttpClient()     # sync — just creates the object
await client.init()         # async — fires $Idle's $>() handler
await client.fetch("...")   # async — normal usage
```

### Try It

Add async `save()` and `load()` methods to the `Editor` example from the Transitions in Depth section. The `$Editing` state should handle both, writing/reading the buffer to/from a file.

---

## Advanced Topics

This section covers features you'll reach for as your Frame systems grow: system context, operations, persistence, multi-system files, and visualization.

### System Context

When an interface method is called, Frame creates a *context* that handlers can access with the `@@` prefix:

| Syntax | Meaning |
|--------|---------|
| `@@:params.x` | Access interface parameter `x` |
| `@@:return` | Get or set the return value |
| `@@:event` | The name of the interface method that was called |
| `@@:data.key` | Call-scoped data that persists across transitions |
| `@@:self` | Reference to this system instance |
| `@@:self.state` | Current state name (string) |

#### Accessing Parameters

```
interface:
    process(input, mode)

machine:
    $Ready {
        process(input, mode) {
            # These are equivalent:
            result = transform(input)        # direct parameter
            result = transform(@@:params.input) # system context

            @@:return = result
        }
    }
```

`@@:params.x` accesses interface parameters by name. It's most useful in actions, which don't receive event parameters directly.

#### Return Value

`@@:return` is the slot where the interface method's return value lives:

```
calculate(a, b): int = 0 {
    @@:return = a + b
}
```

`@@:return = expr` sets the return value. The concise form `@@:(expr)` does the same thing. In handlers, `return` is always native — it exits the dispatch method but does NOT set the return value.

#### Call-Scoped Data

`@@:data.key` stores data that survives transitions within a single interface call:

```
$Validating {
    submit(order) {
        @@:data.order = order
        -> $Processing
    }
}

$Processing {
    $>() {
        order = @@:data.order  # Still available after transition
        process(order)
    }
}
```

The data is scoped to the interface call — once `submit()` returns to the caller, the data is gone.

### Operations

Operations are public methods that bypass the state machine entirely:

```
@@system Config {
    operations:
        static version(): str {
            return "4.0.0"
        }

        get_debug_info(): str {
            return f"items={len(self.items)}"
        }

    interface:
        add(item)

    machine:
        $Active {
            add(item) {
                self.items.append(item)
            }
        }

    domain:
        items = []
}
```

- **Static operations** don't have access to `self` — they're class methods
- **Non-static operations** can access domain variables but bypass the state machine
- Operations cannot use Frame constructs (transitions, state variables, etc.)

Use operations for utility methods, version info, debug introspection — anything that shouldn't be part of the state machine.

### Persistence

Add `@@persist` before a system to generate save/restore methods:

```
@@target python_3
@@persist

@@system Session {
    interface:
        login(user)
        logout()

    machine:
        $LoggedOut {
            login(user) {
                self.current_user = user
                -> $LoggedIn
            }
        }

        $LoggedIn {
            logout() {
                self.current_user = None
                -> $LoggedOut
            }
        }

    domain:
        current_user = None
}
```

The framepiler generates:

- `save_state()` — serializes the current state, state variables, state stack, and domain variables
- `restore_state(data)` — static method that reconstructs a system from saved data

```python
# Save
data = session.save_state()
store_to_database(data)

# Restore later
data = load_from_database()
session = Session.restore_state(data)
# session is now in whatever state it was in when saved
```

What gets persisted: current state, state variables, state stack, state arguments, and domain variables.

### Codegen Options

The `@@codegen` directive controls code generation:

```
@@codegen {
    frame_event: on
}
```

Currently the only option is `frame_event`:

- **`off`** (default) — lean generated code, events are internal
- **`on`** — generates `FrameEvent` and `FrameContext` classes, needed for enter/exit parameters, event forwarding, and `@@:return`

The framepiler auto-enables `frame_event` when features that require it are used, with a warning if you explicitly set it to `off`.

### Multi-System Files

A single file can contain multiple `@@system` blocks:

```
@@target python_3

@@system Logger {
    interface:
        log(msg)
    machine:
        $Active {
            log(msg) {
                print(f"LOG: {msg}")
            }
        }
}

@@system App {
    interface:
        start()
    machine:
        $Init {
            start() {
                self.logger.log("App started")
                -> $Running
            }
        }
        $Running {
        }
    domain:
        logger = @@Logger()
}
```

Each system is independent — they don't share state. They interact through their public interfaces, just like any other objects.

### GraphViz Visualization

Generate a state chart diagram from any Frame file:

```bash
framec -l graphviz myfile.fpy | dot -Tpng -o chart.png
```

This produces a DOT graph showing states as nodes and transitions as edges. Labels on transitions show the events that trigger them. Labeled transitions (`-> "label" $State`) use the label text on the edge.

For multi-system files, each system generates its own diagram.

### What's Next

You now know the full Frame language. Here are some directions to explore:

- Browse the [Cookbook](frame_cookbook.md) for 21 complete, runnable examples
- Browse the [supported languages](../README.md#supported-languages) and try a different target
- Read the [CONTRIBUTING guide](../CONTRIBUTING.md) if you want to help improve the framepiler
- Check the [GitHub issues](https://github.com/frame-lang/framepiler/issues) for feature requests and discussions