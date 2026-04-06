# Actions

Actions are private helper methods that keep your event handlers clean. They can access domain variables and interface context, but they cannot perform transitions or stack operations. This chapter covers why and how to use them.

## The Problem

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

## Actions to the Rescue

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

## What Actions Can Do

Actions are native code methods. They can:

- Accept parameters
- Return values
- Access domain variables (via `self` in Python, `this` in TypeScript, etc.)
- Call other actions
- Call any native code

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

## What Actions Cannot Do

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

## Actions vs Operations vs Native Functions

Frame has three kinds of methods besides event handlers. Here's when to use each:

- **Action**: private helper that needs access to domain variables. Cannot trigger transitions or access state variables. Called from handlers.
- **Operation**: public method that bypasses the state machine entirely. Good for utility methods, version info, or debug introspection. Declared in the `operations:` section (see Chapter 10).
- **Native function**: a regular function outside the system. No access to domain variables. Use for pure computation or code shared across systems.

The key distinction: actions are *private* helpers for handlers, operations are *public* methods that skip the state machine, and native functions live outside the system entirely.

## Try It

Take the `Turnstile` from Chapter 3 and add actions for `log_entry()` and `log_rejection()`. Call them from the appropriate handlers.

[<- Previous: Events and the Interface](04-events.md) | [Next: Variables ->](06-variables.md)
