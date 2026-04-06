# Transitions in Depth

You've already used simple transitions (`-> $State`). Frame supports a rich set of transition forms with enter/exit events, parameters, forwarding, and history. This chapter covers all of them.

## Enter and Exit Events

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
    c.connect()            # prints "Cleaning up..." wait, no:
                           # $Disconnected has no <$, so just
                           # prints "Connection established"
    c.disconnect()         # prints "Cleaning up connection"
                           # then "Ready to connect"
```

The enter handler (`$>`) is the natural place for initialization. The exit handler (`<$`) is for cleanup. Both are optional.

## Enter and Exit Parameters

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

## The Full Transition Form

A transition can carry exit args, enter args, state args, and a label:

```
(exit_args) -> (enter_args) "label" $State(state_args)
```

- **Exit args**: passed to current state's `<$` handler
- **Enter args**: passed to target state's `$>` handler
- **Label**: a string for diagram generation (no runtime effect)
- **State args**: initialize the target state's variables

You rarely need all of these at once, but they compose freely.

## Event Forwarding

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

## State Stack and History

Frame has a built-in state stack for saving and restoring states. This enables patterns like modal dialogs, subroutine states, and undo.

### Push

`push$` saves the current state (including all state variables) onto the stack:

```
$Normal {
    help() {
        push$
        -> $HelpMode
    }
}
```

### Pop

`-> pop$` transitions to whatever state was last pushed:

```
$HelpMode {
    done() {
        -> pop$   # Returns to $Normal (or wherever we came from)
    }
}
```

The critical difference from a normal transition: **state variables are restored**. If `$Normal` had `$.count = 5` when it was pushed, `$.count` will be `5` when popped back — not reset to its initial value.

### Example: Subroutine State

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
                return "editing"
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
                return "searching"
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

## Transition Summary

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

## Try It

Build a `Wizard` with states `$Step1`, `$Step2`, `$Step3`, and `$Review`. Use `push$` before each forward step and `-> pop$` for the "back" button, so users can go back and their form data (state variables) is preserved.

[<- Previous: Variables](06-variables.md) | [Next: Hierarchical State Machines ->](08-hsm.md)
