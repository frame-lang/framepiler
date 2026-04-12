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
- [Self Reference](#self-reference)
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

Three parameter groups configure a system at construction time. Each is optional, but when present they must appear in this order: **state params** (`$()`), then **enter params** (`$>()`), then **domain params** (bare).

```
@@system Name ( $(state_params), $>(enter_params), domain_params )
```

| Group | Sigil | Target |
|-------|-------|--------|
| State arg | `$(name: type)` | Start state's `compartment.state_args` |
| Enter arg | `$>(name: type)` | Start state's `compartment.enter_args` |
| Domain arg | `name: type` (bare) | Constructor argument, used in domain field initializers |

Each param body has the same shape (`name: type` or `name: type = default`) regardless of group; only the sigil differs. The framepiler validates that state and enter args have matching declarations on the start state's `$Start(name: type)` and `$>(name: type)` handlers.

#### Param syntax

Each individual parameter follows the same shape as an interface method parameter:

```
name
name : type
name : type = default
```

- Untyped (`name`): valid in dynamically-typed targets (Python, JavaScript, Ruby, Lua, GDScript, PHP, Erlang). Static-typed targets require an explicit type.
- Typed (`name : type`): the type string is passed through verbatim to the target language's constructor signature. Use the target's native type names (`int`, `str`, `bool`, `float`, etc.).
- Defaulted (`name : type = default`): the default expression is pasted verbatim into the constructor signature. Defaults must be valid in the target language at the parameter-default position. Integer and boolean literals are portable; string and collection defaults may not be.

#### State params

`$(name: type)` declares a parameter that lands in the start state's `compartment.state_args` map under the declared name. The start state must have a matching `$Start(name: type)` declaration so the dispatch function can bind the param to a local at the top of the state body:

```frame
@@system Robot($(x: int), name: str) {
    interface:
        describe(): str

    machine:
        $Start(x: int) {
            describe(): str { @@:(self.name + "@" + str(x)) }
        }

    domain:
        name = name
}

r = @@Robot($(7), "R2D2")       // x = 7 (state arg), name = "R2D2" (domain)
```

Note the call site: state args are tagged with `$(...)` so the assembler can route them into `compartment.state_args`. See [System Instantiation](#system-instantiation) for the full call site form.

State args are also written by transitions (`-> $Start(42)`). The codegen stores transition-passed args under the same declared param name, so the dispatch reads the param identically whether the state was entered via the system constructor or a transition.

#### Enter params

`$>(name: type)` declares a parameter that lands in the start state's `compartment.enter_args` map under the declared name. The start state must have a matching `$>(name: type)` enter handler that reads the param:

```frame
@@system Worker($>(batch_size: int)) {
    interface:
        run()

    machine:
        $Start {
            $>(batch_size: int) {
                self.size = batch_size
            }
            run() {
                // process self.size items
            }
        }

    domain:
        size = 0
}

w = @@Worker($>(50))            // start state's enter handler sees batch_size = 50
```

The call site tags enter args with `$>(...)`, the same shape as the declaration. Enter args are also written by transitions that use the `-> "args" $State` form. As with state args, the codegen stores both transition-passed and constructor-passed enter args under the declared param name.

#### Domain params

Bare identifiers in the header become **constructor arguments** that are in scope when the domain field initializers run. A domain field's right-hand side can reference any header param by name:

```frame
@@system Counter(initial: int = 0) {
    interface:
        get(): int

    machine:
        $Counting {
            get(): int { @@:(self.value) }
        }

    domain:
        value = initial         // initial is a constructor arg in scope
}

c = @@Counter(10)               // value is 10
```

The codegen prepends the language-appropriate self-reference (`self.`, `this.`, `@`) to the LHS of the domain field assignment, so `value = value` (param and field with the same name) is unambiguous: it compiles to `self.value = value`.

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

**Portable init expressions:** Use Frame-portable literals for state variable initializers: `""` for strings, `0` for integers, `false` for booleans. The framepiler wraps these to match the target language's type system (e.g., `String::from("")` for Rust, `std::string("")` for C++). Target-language-specific constructors like `String::new()` are NOT portable — the Frame parser may not handle them correctly. If you need a target-specific value, write it as native code and the framepiler will pass it through unchanged.

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

**Can access:** domain variables, `@@:return`, `@@:params.x`, `@@:event`, `@@:data.key`, `@@:self`

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

Frame recognizes exactly **7 constructs** within handler bodies. Everything else is native code.

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

Saves a **reference** to the current compartment (including all state variables) onto the state stack. The compartment itself is NOT copied — the stack entry and `__compartment` point to the same object.

`push$` is almost always followed by a transition (`push$ -> $State`). The transition creates a new compartment for the target state; the old one is preserved on the stack. `-> pop$` later restores the saved reference.

**Bare `push$`** (no transition): the stack entry and current compartment are the same object. Any modifications to state variables after push$ are visible through both. `pop$` restores the same modified object. For snapshot/undo semantics, use `push$ -> $SameState(args)` to create a new compartment on transition.

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
@@:params.x         // interface parameter (by name)
@@:return = <expr>  // set return value
@@:return           // read return value
@@:event            // interface method name
@@:data.key         // call-scoped data (by key)
```

See [System Context](#system-context) for full semantics.

### Self Reference — `@@:self`

```
@@:self              // reference to this system instance
@@:self.method(args) // call own interface method (reentrant)
```

See [Self Reference](#self-reference-1) for full semantics.

**`return` is always native.** It exits the current function — it does NOT set `@@:return`. In event handlers, `return expr` silently loses the value (W415 warning). Use `@@:(expr)` or `@@:return = expr` to set return values.

| Syntax | Effect |
|--------|--------|
| `@@:(expr)` | Set return value only (concise) |
| `@@:return = expr` | Set return value only (explicit long form) |
| **`@@:return(expr)`** | **Set return value AND exit handler (one statement)** |
| `return` | Exit the handler (native — valid everywhere) |
| `return expr` | Native return — in handlers, value is lost (W415) |
| `return @@:(expr)` | Error E408 — cannot combine |

**`@@:return(expr)`** is the recommended form when you want to set the return value and immediately exit. It replaces the common two-statement pattern `@@:(expr)` + `return`. The expression inside the parens is evaluated, stored in the context return slot, and a native `return` is emitted — all in one Frame statement.

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

### Accessor Grammar

All `@@` accessors follow a uniform grammar:

- **`:`** (colon) — navigates Frame's namespace hierarchy
- **`.`** (dot) — accesses a field on the resolved object

Colon drills through Frame namespaces. Dot accesses a property on whatever you've arrived at. If the target is a value (not a container), no dot is needed.

### Context Accessors

`@@:` refers to the current execution context. It is transient — it exists for the duration of a dispatch chain and is then discarded. Multiple contexts stack on `_context_stack` during reentrant calls.

| Syntax | Meaning |
|--------|---------|
| `@@:params.x` | Interface parameter `x` |
| `@@:params` | Parameter bag (if needed as object) |
| `@@:return` | Get/set return value |
| `@@:(expr)` | Set return value (concise) |
| `@@:return(expr)` | Set return value and exit handler |
| `@@:event` | Interface method name |
| `@@:data.key` | Call-scoped data entry |

### Reentrancy

Each interface call pushes its own context. Nested calls are isolated — inner `@@:return` does not affect outer `@@:return`.

### Context Not Available

`@@` context accessors are not available in static operations or the initial `$>` during construction.

---

## Self Reference

`@@:self` is the reference to the system instance containing the current context. While contexts are transient, `@@:self` always resolves to the same persistent system instance regardless of how many contexts are stacked.

### Self Accessors

| Syntax | Meaning |
|--------|---------|
| `@@:self` | Reference to this system instance |

### Self Interface Call — `@@:self.method(args)`

A system can call its own interface methods using `@@:self.<method>(args)`. This dispatches through the full kernel pipeline — FrameEvent construction, context push, router, state dispatch, handler execution, context pop — exactly as an external call would.

```frame
$Active {
    calibrate() {
        baseline = @@:self.reading()    // reentrant self-call
        self.offset = baseline * -1
    }
    reading(): float {
        @@:(self.raw_sensor_value + self.offset)
    }
}
```

#### Semantics

- **Full dispatch.** The call goes through the kernel. The handler that executes depends on the current state at the time of the call.
- **Context isolation.** A new context is pushed onto `_context_stack`. Inside the called handler, `@@:event` is the called method's name, `@@:params` are the called method's parameters, and `@@:return` is the called method's return slot. The calling handler's context is untouched.
- **Return value.** The return value is available to the caller as a native expression, just like any function call.
- **State sensitivity.** If a transition occurred before the self-call, the call dispatches to a handler in the new state.

#### Restrictions

- Only interface methods can be called via `@@:self`. Actions and operations are called directly using native syntax.
- `@@:self` does not support calling constructors.

#### Validation

| Code | Check | Severity |
|------|-------|----------|
| E601 | Method does not exist in `interface:` block | Error |
| E602 | Argument count does not match interface declaration | Error |
| W601 | Return value not captured for method with return type | Warning |

#### Codegen Expansion

The transpiler expands `@@:self.method(args)` into the target language's native self-call on the generated interface method:

| Target | Expansion |
|--------|-----------|
| Python | `self.method(args)` |
| TypeScript | `this.method(args)` |
| Rust | `self.method(args)` |
| C | `SystemName_method(self, args)` |
| C++ | `this->method(args)` |
| Go | `s.Method(args)` |
| Java | `this.method(args)` |

The generated interface method handles FrameEvent construction, context push/pop, kernel dispatch, and return value extraction. The self-call enters the same code path as an external call.

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

Use `@@SystemName()` in native code to instantiate a Frame system. The framepiler expands this to the appropriate native constructor and validates that the system name exists and arguments match.

```frame
calc = @@Calculator()
```

### Passing system parameters

When the system header declares parameters (see [System Parameters](#system-parameters)), the call site supplies them in one of two forms. **Within a single call, all arguments must use the same form** — mixing positional and named is rejected.

#### Sigil-tagged positional form

State and enter args at the call site are tagged with the same sigils used in the declaration. Domain args remain bare. Order at the call site must match declaration order.

```frame
// Pure domain params — no sigils needed
@@system Counter(initial: int = 0) { ... }. 
c = @@Counter(10)

// Mixed: state param + domain
@@system Robot($(x: int), name: str) { ... }
r = @@Robot($(7), "R2D2")

// Pure enter param
@@system Worker($>(batch_size: int)) { ... }
w = @@Worker($>(50))

// All three groups in one header
@@system Service($(slot: int), $>(timeout: int), name: str) { ... }
s = @@Service($(0), $>(1000), "primary")
```

#### Named form

The named form omits ordering requirements and lets you supply args by declared name. Domain args use bare `name=value`; state and enter args wrap the assignment in their sigil.

```frame
@@system Robot($(x: int), name: str) { ... }
r = @@Robot($(x=7), name="R2D2")

@@system Service($(slot: int), $>(timeout: int), name: str) { ... }
s = @@Service($(slot=0), $>(timeout=1000), name="primary")
```

Named-form args may be supplied in any order. Defaults are filled in for any omitted params.

#### Defaults are substituted at the call site

Parameters with default values may be omitted from either form. The Frame assembler substitutes the declared default expression at the tagged-instantiation expansion site, so the target language never sees it as a constructor-default — it's a literal arg in the generated call.

```frame
@@system Counter(initial: int = 0) { ... }
c1 = @@Counter()         // expands to Counter(0)  — Frame substitutes the default
c2 = @@Counter(42)       // expands to Counter(42)
```

This means default values can use any expression valid in the target language at *call* scope, not just at *parameter-default* scope. It's also why the call site for `@@Counter()` works in target languages that don't natively support default arguments (Java, C, Go, etc.).

#### Validation

The framepiler validates at the assembler stage:

- The system name exists in this file.
- Sigils on the call site match the declared groups (`$(...)` for state args, `$>(...)` for enter args, bare for domain).
- All required (no-default) params are supplied.
- Named args reference declared param names (no typos).
- No duplicate named args.
- No mixing positional and named within a single call.
- State and enter args have matching declarations on the start state's `$Start(name: type)` and `$>(name: type)` handlers.

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
| `@@:params.x` | Interface parameter `x` |
| `@@:return` | Return value |
| `@@:event` | Event name |
| `@@:data.key` | Call-scoped data |

### Self

| Token | Meaning |
|-------|---------|
| `@@:self` | System instance reference |
| `@@:self.method()` | Self interface call |

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

### Self-Call Errors (E6xx)

| Code | Name | Description |
|------|------|-------------|
| E601 | `unknown-iface-method` | `@@:self.method()` targets method not in `interface:` |
| E602 | `self-call-arity` | Argument count does not match interface declaration |

### Warnings (W4xx, W6xx)

| Code | Name | Description |
|------|------|-------------|
| W414 | `unreachable-state` | State has no incoming transitions |
| W601 | `unused-self-call-return` | Return value not captured for method with return type |

---

## Complete Example

```frame
import logging

@@target python_3
@@codegen {
    frame_event: on,
}

@@persist
@@system OrderProcessor ($(order_type: str), $>(initial_data: dict), max_retries: int) {

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
                self.order_data = order
                -> $Validating
            }
        }

        $Validating {
            $.attempts: int = 0

            $>() {
                $.attempts = $.attempts + 1
                if validate(self.order_data):
                    -> $Processing
                else:
                    if $.attempts >= self.max_retries:
                        -> $Failed
            }

            getStatus(): str {
                @@:("validating")
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
                @@:("processing")
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
        order_data = None
}

if __name__ == '__main__':
    proc = @@OrderProcessor($("standard"), $>({"source": "web"}), 5)
    proc.submit({"item": "widget", "qty": 3})
```