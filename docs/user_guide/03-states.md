# States and Handlers

A state machine's power comes from having multiple states, each responding to the same events differently. In this chapter you'll build a system with multiple states and transitions between them.

## A Light Switch

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
                return "off"
            }
        }

        $On {
            toggle() {
                -> $Off
            }
            status(): str {
                return "on"
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

## How Dispatch Works

When you call `sw.toggle()`:

1. The system routes `toggle` to the current state's handler
2. If the current state is `$Off`, the `$Off` version of `toggle()` runs
3. If the current state is `$On`, the `$On` version runs
4. If a handler triggers a transition (`-> $State`), the system changes state after the handler finishes

Events that a state doesn't handle are silently ignored. If `$Off` doesn't have a `toggle()` handler, calling `toggle()` while in `$Off` simply does nothing.

## Return Values

The `status()` method shows how to return values from handlers:

```
status(): str = "unknown"
```

In the `interface:` block, `: str` declares the return type and `= "unknown"` sets the default return value. If the current state doesn't handle `status()`, the caller gets `"unknown"`.

Inside a handler, `return` sets the return value:

```
status(): str {
    return "off"
}
```

## A More Interesting Example

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
                return "locked - insert coin"
            }
        }

        $Unlocked {
            coin() {
                # Already unlocked, coin is wasted
            }
            push(): str {
                -> $Locked
                return "welcome"
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

Notice that a handler can both transition and return a value. The transition is *deferred* — it happens after the handler finishes, so `return "welcome"` executes before the system moves to `$Locked`.

## Deferred Transitions

This is an important concept: **transitions don't happen immediately**. When a handler executes `-> $State`, the system records the target but doesn't switch yet. The transition is processed after the handler returns.

This means:

- Code after `->` in the same handler still executes in the current state
- You can set return values after a transition
- You can do cleanup work after triggering a transition

## Unhandled Events

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

## Try It

Build a `Door` system with three states: `$Closed`, `$Open`, and `$Locked`. It should support `open()`, `close()`, `lock()`, and `unlock()` — but you can only lock a closed door, and you can only open an unlocked door.

[<- Previous: Your First System](02-first-system.md) | [Next: Events and the Interface ->](04-events.md)
