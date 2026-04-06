# Advanced Topics

This chapter covers features you'll reach for as your Frame systems grow: system context, operations, persistence, multi-system files, and visualization.

## System Context

When an interface method is called, Frame creates a *context* that handlers can access with the `@@` prefix:

| Syntax | Meaning |
|--------|---------|
| `@@.param` | Access interface parameter `param` |
| `@@:return` | Get or set the return value |
| `@@:event` | The name of the interface method that was called |
| `@@:data[key]` | Call-scoped data that persists across transitions |

### Accessing Parameters

```
interface:
    process(input, mode)

machine:
    $Ready {
        process(input, mode) {
            # These are equivalent:
            result = transform(input)   # direct parameter
            result = transform(@@.input) # system context

            @@:return = result
        }
    }
```

`@@.param` is a shorthand for accessing interface parameters. It's most useful in actions, which don't receive event parameters directly.

### Return Value

`@@:return` is the slot where the interface method's return value lives:

```
calculate(a, b): int = 0 {
    @@:return = @@.a + @@.b
}
```

The `return expr` syntax in handlers is sugar for `@@:return = expr` followed by an implicit return.

### Call-Scoped Data

`@@:data[key]` stores data that survives transitions within a single interface call:

```
$Validating {
    submit(order) {
        @@:data["order"] = order
        -> $Processing
    }
}

$Processing {
    $>() {
        order = @@:data["order"]  # Still available after transition
        process(order)
    }
}
```

The data is scoped to the interface call — once `submit()` returns to the caller, the data is gone.

## Operations

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

## Persistence

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

The transpiler generates:

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

## Codegen Options

The `@@codegen` directive controls code generation:

```
@@codegen {
    frame_event: on
}
```

Currently the only option is `frame_event`:

- **`off`** (default) — lean generated code, events are internal
- **`on`** — generates `FrameEvent` and `FrameContext` classes, needed for enter/exit parameters, event forwarding, and `@@:return`

The transpiler auto-enables `frame_event` when features that require it are used, with a warning if you explicitly set it to `off`.

## Multi-System Files

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

## GraphViz Visualization

Generate a state chart diagram from any Frame file:

```bash
framec -l graphviz myfile.fpy | dot -Tpng -o chart.png
```

This produces a DOT graph showing states as nodes and transitions as edges. Labels on transitions show the events that trigger them. Labeled transitions (`-> "label" $State`) use the label text on the edge.

For multi-system files, each system generates its own diagram.

## What's Next

You now know the full Frame language. Here are some directions to explore:

- Browse the [supported languages](../../README.md#supported-languages) and try a different target
- Read the [CONTRIBUTING guide](../../CONTRIBUTING.md) if you want to help improve the transpiler
- Check the [GitHub issues](https://github.com/frame-lang/framepiler/issues) for feature requests and discussions

[<- Previous: Async](09-async.md)
