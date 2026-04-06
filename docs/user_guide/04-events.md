# Events and the Interface

The `interface:` block is how the outside world communicates with your state machine. Each method declared there becomes an event that gets routed to the current state. This chapter covers parameters, return values, and how the interface connects to handlers.

## Interface Methods

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

## Parameters

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

## Typed Parameters

You can add type annotations to parameters:

```
interface:
    set_position(x: float, y: float)
    send_message(to: str, body: str)
```

Type annotations are passed through to the generated code. Their exact semantics depend on your target language (enforced in TypeScript, advisory in Python, required in Rust/C).

## Return Values

To return values from a state machine, declare the return type and default in the interface:

```
interface:
    get_count(): int = 0
```

Then in handlers, use `return`:

```
$Counting {
    get_count(): int {
        return self.count
    }
}
```

The `return` in a handler is Frame sugar — it sets the return value and exits the handler. If the current state doesn't handle the event, the caller gets the default value (`0` in this case).

## Multiple Parameters and Returns

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
                return self.last_result
            }
            multiply(a, b): int {
                self.last_result = a * b
                return self.last_result
            }
            get_last(): int {
                return self.last_result
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

## Events That Change Behavior by State

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
                return "stopped"
            }
        }

        $Playing {
            pause() {
                print("Pausing")
                -> $Paused
            }
            get_state(): str {
                return "playing"
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
                return "paused"
            }
        }
}
```

Notice:
- `$Stopped` ignores `pause()` — you can't pause something that isn't playing
- `$Playing` ignores `play()` — you can't play something already playing
- `$Paused` handles both — play resumes, pause stops entirely

This is the pattern: each state declares *only the events it cares about*. Everything else is silently ignored.

## Try It

Build a `Counter` system with `increment()`, `decrement()`, `reset()`, and `get_value(): int = 0`. Add a `$Frozen` state where increment and decrement are ignored but reset still works.

[<- Previous: States and Handlers](03-states.md) | [Next: Actions ->](05-actions.md)
