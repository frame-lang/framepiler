# Variables

Frame provides three kinds of variables, each with different scope and lifetime. This chapter covers domain variables, state variables, and state parameters.

## Domain Variables

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
                return self.count
            }
        }

    domain:
        count: int = 0
}
```

Domain variables are **native code** — the transpiler passes them through to the generated class as instance fields. The syntax matches your target language:

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

## State Variables

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
        return $.attempts
    }
}
```

Key behaviors:

- **Scoped to one state** — `$.attempts` only exists in `$Retrying`. Other states can't see it.
- **Reset on normal transition** — when you enter `$Retrying` via `-> $Retrying`, state variables reset to their declared initial values (`0` and `None` here).
- **Preserved on history transition** — when you enter via `-> pop$`, state variables keep their values from when the state was pushed. More on this in [Chapter 7](07-transitions.md).

The `$.` prefix is how you read and write state variables.

## When to Use Which

| Variable | Scope | Lifetime | Use for |
|----------|-------|----------|---------|
| Domain (`self.x`) | All states | System lifetime | Shared data, configuration, accumulated results |
| State (`$.x`) | One state | Until next transition into that state | Retry counts, per-state buffers, temporary state |

A good rule: if multiple states need it, use domain. If only one state needs it and it should reset each time you enter that state, use a state variable.

## State Parameters

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
                return "Home"
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
                return $.title
            }
        }
}
```

When you write `-> $Page("Settings", "/settings")`, the arguments initialize the target state's variables — the first argument maps to the first declared `$.` variable, the second to the second, and so on.

## System Parameters

You can also pass arguments when constructing a system. System parameters can initialize domain variables:

```
@@target python_3

@@system Server (port, host) {
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
        port: int = 8080
        host: str = "localhost"
}

if __name__ == '__main__':
    s = @@Server(3000, "0.0.0.0")
    s.start()  # "Starting on 0.0.0.0:3000"
```

System parameters override the default values of matching domain variables.

## Try It

Build a `Stopwatch` with states `$Stopped` and `$Running`. Use a domain variable for elapsed time (persists across stops/starts) and a state variable in `$Running` for the start timestamp.

[<- Previous: Actions](05-actions.md) | [Next: Transitions in Depth ->](07-transitions.md)
