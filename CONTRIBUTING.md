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
./scripts/install-hooks.sh   # enables pre-commit doc-sample validation
```

### Running Tests

```bash
# Unit tests (in this repo)
cargo test

# Integration tests (17 languages, in separate repo)
# See docs/contributing/testing.md for full guide
cd /path/to/framepiler_test_env/docker
make test              # All languages
make test-python       # Single language
```

### Code Quality

All PRs must pass these checks (CI enforces them):

```bash
cargo clippy -- -D warnings           # No warnings allowed
cargo fmt --check                     # Consistent formatting
python3 scripts/validate_doc_samples.py   # Every runnable docs/ example compiles and runs
```

The doc validator extracts runnable Frame blocks from `docs/*.md`, compiles them with `framec`, and runs the generated Python. It's also wired into the pre-commit hook (see `.githooks/pre-commit`) so edits to docs are checked before they're committed.

## Project Structure

```
framepiler/
├── framec/                    # The framepiler
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
│   ├── frame_getting_started.md  # Tutorial
│   ├── frame_language.md         # Language reference
│   ├── frame_runtime.md          # Runtime architecture
│   ├── framepiler_design.md      # Transpiler internals
│   └── contributing/             # Contributor deep-dives
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

### Working with Generated Scanners (`.frs` → `.gen.rs`)

Several internal scanners and parsers in `framec/src/frame_c/compiler/` are themselves Frame state machines. They live as paired files:

- `<name>.frs` — Frame source (the truth)
- `<name>.gen.rs` — generated Rust (committed for build hermeticity, **never hand-edit**)

Examples include `native_region_scanner/context_parser.frs`, `codegen/output_block_parser.frs`, and the per-language `import_scanner/*_import.frs` and `body_closer/*.frs` files.

To modify scanner behavior:

1. Edit the `.frs` source.
2. Build a bootstrap framec: `cargo build --release`
3. Regenerate the matching `.gen.rs` — see [Adding a Backend](docs/contributing/adding-a-backend.md#regenerating-output_block_parsergenrs) for the canonical command pattern.
4. Run `cargo test` and the doc validator to confirm nothing regressed.
5. Commit the paired `.frs` and `.gen.rs` together.

If you find yourself reaching for a `.gen.rs` file directly, stop — the change will be silently overwritten the next regen.

## Code Style

- Follow standard Rust conventions
- `cargo fmt` is the authority on formatting
- `cargo clippy` with `-D warnings` is enforced — fix all warnings
- Prefer clarity over cleverness

## License

By contributing, you agree that your contributions will be licensed under the [Apache License 2.0](LICENSE).
