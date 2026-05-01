

# RFCSystem Context

Frame's grammar is anchored by one token: **`@@`**, the **system context token**. Every Frame-language construct begins with `@@` — directives, system declarations, instantiation, return values, parameter access, self-calls. Native code never does. The token is how the framepiler tells Frame islands from the native ocean: a token starting with `@@` belongs to Frame; everything else passes through to the target language unchanged.

### What `@@` Reaches Depends on Where It Appears

`@@` is **scope-sensitive**. The same token reaches different things at module scope vs. inside a handler, action, or operation.

| Scope | What `@@` reaches | Examples |
|-------|-------------------|----------|
| **Module scope** | Frame directives, system declarations, system instantiation | `@@target python_3`, `@@codegen { ... }`, `@@[persist]`, `@@system Name { ... }`, `@@SystemName(args)` |
| **Inside a handler, action, or operation** | The **dispatch context** of the current interface call | `@@:return`, `@@:params.x`, `@@:event`, `@@:data.k`, `@@:self.method()`, `@@:system.state` |

At module scope, `@@` is followed by a Frame keyword (`target`, `codegen`, `persist`, `system`) or a system name being instantiated. Inside a handler, the most-used target is the dispatch context — so common that Frame gives it dedicated syntax: a colon after `@@`.

### The Colon: Descending Into the Dispatch Context

`@@:` reads as "into the dispatch context." The colon is the descent operator; what it descends into is the per-call context the runtime pushed onto `_context_stack` when this interface call started.

The accessor grammar is uniform:

- **`:`** descends through Frame's namespace hierarchy
- **`.`** accesses a field on the resolved object

So `@@:params.x` reads as "into the context, navigate to `params`, then access field `x`." `@@:system.state` reads as "into the context, navigate to `system`, then access field `state`." And `@@:return` reads as "into the context, the return slot" — because `return` resolves to a value, not a container, no `.` follows.

### `@@:` Defaults to `@@:return`

Bare `@@:` — the colon descent with no member name — is an alias for `@@:return`. This is the most-used context member, and the default exists so that the most common operation (setting the return value) can be written most concisely.

That single rule explains all three return-value forms:

| Form | Reads as |
|------|----------|
| `@@:return = expr` | explicit member, explicit assignment |
| `@@:(expr)` | bare `@@:` (defaults to `return`) called with `expr` to assign |
| `@@:return(expr)` | explicit member called with `expr` to assign **and** exit the handler |

`@@:(expr)` and `@@:return = expr` produce identical generated code. `@@:return(expr)` adds an immediate native `return` after the assignment, replacing the two-statement pattern of "set value, then exit."

### Bare `@@` Is Never Written

`@@` always has something following it. At module scope it's followed by a Frame keyword or a system name. Inside a handler it's followed by `:` (descending into the dispatch context) and then a context member or `(expr)`. Bare `@@` by itself is a parse error — there's nothing for it to mean.

### `@@:self` and `@@:system` Are Paths, Not Values

Both must be followed by a member access:

- `@@:self.method(args)` — call your own interface (validated against `interface:`; E601 if the method doesn't exist, E602 on arity mismatch)
- `@@:system.state` — read the current state name

Bare `@@:self` is **E603**; bare `@@:system` is **E604**. This is deliberate. Letting these escape into native code as free-floating values would force Frame to commit to a target-specific type for "the system's self-reference," which would break source-portability across the 17 backends — a single Rust target alone has at least four plausible self-types (`&Self`, `&mut Self`, `Rc<RefCell<Self>>`, `Arc<Mutex<Self>>`).

For patterns that need a system to hold a reference to another system, pass the reference in at construction time from native call-site code (e.g., `child = ChildSystem(self)` written natively, *not* through `@@`). For patterns that need to hand a self-reference to native code from inside a handler, write a small action that returns `self` in your target language — this localizes the target-specific code to one place rather than scattering it through handler bodies.
