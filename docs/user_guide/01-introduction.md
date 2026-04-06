# Introduction

## What is Frame?

Frame is a language for specifying state machines. You write a compact specification of states, transitions, and event handlers, and the Frame transpiler generates a full implementation in your target language — Python, TypeScript, Rust, C, and many others.

Frame is not a standalone language. It's designed to live *inside* your native source files, side by side with regular code. The transpiler expands your Frame specifications and passes everything else through unchanged.

## Native Code Passes Through

Frame is designed to coexist with your existing code. You write `@@system` blocks alongside regular native code — imports, helper functions, test harnesses — and the transpiler only touches the Frame blocks. Everything else passes through unchanged.

```
# Regular Python — passes through unchanged
import logging

logger = logging.getLogger(__name__)

# Frame specification — gets expanded into a full class
@@target python_3

@@system TrafficLight {
    interface:
        next()
    machine:
        $Green {
            next() {
                -> $Yellow
            }
        }
        $Yellow {
            next() {
                -> $Red
            }
        }
        $Red {
            next() {
                -> $Green
            }
        }
}

# Regular Python again — passes through unchanged
if __name__ == '__main__':
    light = @@TrafficLight()
    light.next()  # Green -> Yellow
    light.next()  # Yellow -> Red
    light.next()  # Red -> Green
```

The transpiler expands the `@@system TrafficLight { ... }` block into a full Python class. The `import`, the `logger`, the `if __name__` block — all of that passes through exactly as you wrote it. This is Frame's core principle: **native code passes through unchanged**.

## Why State Machines?

State machines make certain kinds of programs dramatically simpler:

- **UI workflows** — login flows, wizards, form validation
- **Protocol handlers** — TCP connections, WebSocket sessions
- **Game logic** — character states, turn management
- **Device controllers** — hardware modes, sensor management
- **Business processes** — order fulfillment, approval chains

The pattern is the same: your system is in one of several *states*, it responds to *events* differently depending on which state it's in, and events can cause *transitions* between states.

You can implement this with if/else chains or switch statements, but they become tangled as complexity grows. Frame gives you a clean, declarative way to express the same logic.

## What the Transpiler Does

The transpiler (`framec`) reads your source file and:

1. Finds `@@system` blocks
2. Parses the Frame specification inside each block
3. Generates a full class with state dispatch, transitions, and lifecycle management
4. Passes all native code through unchanged
5. Writes the combined output

The generated code is readable, debuggable, and uses no runtime library. It's just a class in your target language.

## Target Language

The `@@target` directive inside the file is the authoritative declaration of which native language the file targets. The transpiler uses it to determine how to parse native code regions (string/comment syntax), which code generator to use, and what output to produce.

The `@@target` can be overridden by a CLI flag (`-l <language>`) or other configuration, but if neither is provided, the in-file `@@target` is what controls compilation.

## File Extensions

Frame source files conventionally use a target-specific extension:

| Target | Extension | Example |
|--------|-----------|---------|
| Python | `.fpy` | `traffic_light.fpy` |
| TypeScript | `.fts` | `traffic_light.fts` |
| Rust | `.frs` | `traffic_light.frs` |
| C | `.fc` | `traffic_light.fc` |
| Go | `.fgo` | `traffic_light.fgo` |
| Java | `.fjava` | `traffic_light.fjava` |

The file extension is a hint — nothing more. It helps editors and build tools recognize Frame files, but the transpiler does not use it to determine the target language. The `@@target` directive (or a CLI override) is what matters.

## Next

In the next chapter, you'll install the transpiler and write your first Frame system.

[Next: Your First System ->](02-first-system.md)
