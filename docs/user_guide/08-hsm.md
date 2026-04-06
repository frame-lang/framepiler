# Hierarchical State Machines

When state machines grow, you'll find states that share common behavior. Hierarchical state machines (HSM) let child states delegate events to parent states, reducing duplication. This chapter covers parent states, forwarding, and default handlers.

## The Problem

Imagine a media player where every state needs to handle `get_status()` and `emergency_stop()`:

```
$Playing {
    pause()     { -> $Paused }
    get_status(): str { return "playing" }
    emergency_stop() { cleanup(); -> $Stopped }
}

$Paused {
    play()      { -> $Playing }
    get_status(): str { return "paused" }
    emergency_stop() { cleanup(); -> $Stopped }
}

$Buffering {
    ready()     { -> $Playing }
    get_status(): str { return "buffering" }
    emergency_stop() { cleanup(); -> $Stopped }
}
```

`emergency_stop()` is duplicated in every state. If you add a new state, you have to remember to include it.

## Parent States

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
                return "playing"
            }
        }

        $Paused => $Active {
            play() {
                -> $Playing
            }
            get_status(): str {
                return "paused"
            }
        }

        $Stopped {
            play() {
                -> $Playing
            }
            get_status(): str {
                return "stopped"
            }
        }
}
```

`$Playing` and `$Paused` are children of `$Active`. But there's a catch — **events don't automatically forward to the parent**.

## Explicit Forwarding

Frame uses **explicit forwarding**. A child state must explicitly delegate events to its parent with `=> $^`:

```
$Playing => $Active {
    pause() {
        -> $Paused
    }
    get_status(): str {
        return "playing"
    }
    => $^    # Forward everything else to $Active
}
```

The bare `=> $^` at the end of a state is a **default forward** — any event not handled by `$Playing` gets sent to `$Active`. So `stop()` will be handled by `$Active`'s handler.

Without `=> $^`, unhandled events are silently ignored, even if the parent has a handler for them. This is intentional — it gives you full control over what gets forwarded.

## Forwarding Within a Handler

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

## Default Forward vs Selective Handling

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

## A Complete Example

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
                return "appliance"
            }
        }

        $Off {
            power_on() {
                print("Powering on")
                -> $Idle
            }
            get_info(): str {
                return "off"
            }
        }

        $Idle => $Base {
            set_mode(mode) {
                if mode == "turbo":
                    -> $Turbo
            }
            get_info(): str {
                return "idle"
            }
            => $^
        }

        $Turbo => $Base {
            set_mode(mode) {
                if mode == "normal":
                    -> $Idle
            }
            get_info(): str {
                return "turbo"
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

## Rules

- A state can have at most one parent
- Parent chains can't form cycles (`$A => $B => $A` is an error)
- `=> $^` only works in states that have a parent (error E403 otherwise)
- Parent states can themselves have parents (multi-level HSM)
- The parent state doesn't need to be a state the system ever transitions *to* — it can be an abstract handler collection

## Try It

Build a `Form` system with a parent state `$Validated` that handles `validate(): bool`. Create child states `$NameEntry` and `$EmailEntry` that each handle their own input but forward validation to the parent.

[<- Previous: Transitions in Depth](07-transitions.md) | [Next: Async ->](09-async.md)
