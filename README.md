# Framepiler

![CI](https://github.com/frame-lang/framepiler/actions/workflows/ci.yml/badge.svg)
![License](https://img.shields.io/badge/license-Apache--2.0-blue)
![Version](https://img.shields.io/badge/version-4.0.0-green)

The **framepiler** — Frame's transpiler. Frame is a domain-specific language for specifying state machines that transpiles to production code in multiple target languages. You write `@@system` blocks inside your native source files, and the framepiler expands them into full state machine implementations. All native code passes through unchanged — your native compiler handles everything outside the `@@system` blocks.

## Quick Start

```bash
cargo install framec
```

Create a file `hello.fpy`:

```
@@target python_3

@@system Hello {
    interface:
        greet()

    machine:
        $Start {
            greet() {
                print(f"Hello, {self.name}!")
            }
        }

    domain:
        name = "World"
}

if __name__ == "__main__":
    h = @@Hello()
    h.greet()
```

Transpile and run:

```bash
framec hello.fpy         # emits hello.py
python3 hello.py         # prints: Hello, World!
```

## Supported Languages

### Core

| Language | Target Name | Extension |
|---|---|---|
| Python | `python_3` | `.fpy` |
| TypeScript | `typescript` | `.fts` |
| JavaScript | `javascript` | `.fjs` |
| C | `c` | `.fc` |
| C++ | `cpp` | `.fcpp` |
| C# | `csharp` | `.fcs` |
| Java | `java` | `.fjava` |
| Rust | `rust` | `.frs` |
| Go | `go` | `.fgo` |

### Experimental

Kotlin, Swift, PHP, Ruby, Lua, Erlang, Dart, GDScript

### Visualization

| Output | Target Name |
|---|---|
| GraphViz DOT | `graphviz` |

## Usage

```bash
# Transpile to Python (auto-detected from @@target in file)
framec myfile.fpy

# Override target language
framec -l typescript myfile.frm

# Transpile all files in a directory
framec compile-project -l python_3 -o ./output ./src

# Generate state chart
framec -l graphviz myfile.frm | dot -Tpng -o chart.png

# See all options
framec --help
```

## Documentation

- [Getting Started](docs/frame_getting_started.md) — learn Frame from scratch
- [Language Reference](docs/frame_language.md) — complete Frame language reference
- [Cookbook](docs/frame_cookbook.md) — 20 recipes for real-world patterns
- [Runtime Architecture](docs/frame_runtime.md) — how generated code works
- [Framepiler Design](docs/framepiler_design.md) — transpiler internals
- [Contributing](CONTRIBUTING.md) — build from source, run tests, submit PRs
- [Changelog](CHANGELOG.md) — release history

## License

[Apache License 2.0](LICENSE)
