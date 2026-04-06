# Contributing to Framepiler

Thank you for your interest in contributing to the Frame language transpiler! This guide will help you get started.

## Getting Started

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)

### Building from Source

```bash
git clone https://github.com/frame-lang/framepiler.git
cd framepiler
cargo build
```

### Running Tests

```bash
cargo test
```

### Code Quality

All PRs must pass these checks (CI enforces them):

```bash
cargo clippy -- -D warnings   # No warnings allowed
cargo fmt --check              # Consistent formatting
```

## Project Structure

```
framepiler/
├── framec/                    # The transpiler
│   ├── src/
│   │   ├── main.rs           # CLI entry point
│   │   ├── lib.rs            # Library + WASM entry point
│   │   └── frame_c/          # Core compiler
│   │       ├── cli.rs        # Argument parsing
│   │       ├── driver.rs     # Target language definitions
│   │       └── compiler/     # Lexer, parser, codegen, validation
│   │           ├── lexer/
│   │           ├── parser.rs
│   │           ├── codegen/
│   │           │   └── backends/   # One file per target language
│   │           └── validation/
│   └── build.rs              # Version injection
├── docs/
│   ├── user_guide/           # The Frame User Guide
│   └── contributing/         # Contributor deep-dives
└── .github/                  # CI, issue templates
```

### Compilation Pipeline

Frame source flows through 7 stages:

1. **Segmenter** — Splits file into prolog / `@@system` blocks / epilog
2. **Lexer** — Tokenizes Frame syntax within system blocks
3. **Parser** — Builds abstract syntax tree (AST)
4. **Arcanum** — Constructs symbol table
5. **Validator** — Semantic checks (type errors, unreachable code, etc.)
6. **Codegen** — Generates intermediate representation (IR)
7. **Backend** — Emits target language code
8. **Assembler** — Reassembles prolog + generated code + epilog

Native code (outside `@@system` blocks) passes through unchanged.

## Making Changes

### Workflow

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes
4. Run `cargo build && cargo test && cargo clippy -- -D warnings && cargo fmt --check`
5. Open a pull request

### What to Work On

- Check [open issues](https://github.com/frame-lang/framepiler/issues) for bugs and feature requests
- Issues labeled `good first issue` are great starting points
- Use the issue templates when filing new bugs or feature requests

### Understanding the Architecture

For a deep dive into the compilation pipeline, runtime model, and generated code structure:
[Architecture Guide](docs/contributing/framepiler_architecture_guide.md)

### Adding a New Language Backend

This is the most common type of contribution. See the detailed guide:
[Adding a Backend](docs/contributing/adding-a-backend.md)

## Code Style

- Follow standard Rust conventions
- `cargo fmt` is the authority on formatting
- `cargo clippy` with `-D warnings` is enforced — fix all warnings
- Prefer clarity over cleverness

## License

By contributing, you agree that your contributions will be licensed under the [Apache License 2.0](LICENSE).
