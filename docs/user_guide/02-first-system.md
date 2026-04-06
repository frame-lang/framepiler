# Your First System

In this chapter you'll install the Frame transpiler, write a simple state machine, transpile it, and run the result.

## Install

```bash
cargo install framec
```

Verify the installation:

```bash
framec --version
```

## Write

Create a file called `hello.fpy`:

```
@@target python_3

@@system Hello {
    interface:
        greet()

    machine:
        $Start {
            greet() {
                print("Hello from Frame!")
            }
        }
}
```

Let's break this down:

- **`@@target python_3`** — tells the transpiler to generate Python
- **`@@system Hello`** — declares a state machine called `Hello`
- **`interface:`** — the public API. `greet()` is a method callers can invoke
- **`machine:`** — the states. There's one state, `$Start`
- **`$Start`** — a state. The first state listed is always the starting state
- **`greet() { ... }`** — a handler. When `greet()` is called while in `$Start`, this code runs

## Transpile

```bash
framec hello.fpy
```

This writes the generated Python to stdout. To save it to a file:

```bash
framec hello.fpy -o hello.py
```

## Examine the Output

The generated `hello.py` contains a `Hello` class. Open it and take a look — the generated code is straightforward, with no magic and no runtime dependencies. You can read it, debug it, and step through it like any other Python code.

## Run

```bash
python3 hello.py
```

Nothing happens yet — we declared the class but didn't instantiate it. Add native Python code after the system block:

```
@@target python_3

@@system Hello {
    interface:
        greet()

    machine:
        $Start {
            greet() {
                print("Hello from Frame!")
            }
        }
}

if __name__ == '__main__':
    h = @@Hello()
    h.greet()
```

Transpile and run again:

```bash
framec hello.fpy -o hello.py && python3 hello.py
```

Output:

```
Hello from Frame!
```

The `if __name__` block is native Python — the transpiler passes it through unchanged.

## The Anatomy of a System

Every Frame system has the same structure:

```
@@system Name {
    operations:
        # Public methods that bypass the state machine 

    interface:
        # Public methods — how the outside world talks to this system

    machine:
        # States and their event handlers — the behavior

    actions:
        # Private helper methods 

    domain:
        # Instance variables
}
```

The sections must appear in this order: `operations:` → `interface:` → `machine:` → `actions:` → `domain:`. All sections are optional.

## Try It

Modify `hello.fpy` to add a second method `farewell()` that prints a goodbye message. Transpile and run it to verify.

[<- Previous: Introduction](01-introduction.md) | [Next: States and Handlers ->](03-states.md)
