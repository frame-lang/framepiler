# Framepiler — AI Context

## What This Is

The framepiler — Frame's transpiler. Frame is a DSL for state machines that transpiles to 17 target languages. The framepiler is written in Rust and compiles to both a native CLI binary and WASM.

## Build & Test

```bash
cargo build                     # Debug build
cargo build --release           # Release build
cargo test                      # 244 unit tests
cargo clippy -- -D warnings     # Lint
cargo fmt --check               # Format check
```

## Integration Tests

The full 17-language test suite is in a **separate repo** (`framepiler_test_env`), expected as a sibling directory.

Run integration tests via Docker:
```bash
cd ../framepiler_test_env/docker
make test                       # All 16 ARM64 languages
make test-python                # Single language
make test-all                   # All 17 including GDScript
```

The Makefile cross-compiles framec from this repo's source automatically.

**After any codegen change, run the affected language's integration tests.**

## Architecture

```
Source file → Segmenter → Lexer → Parser → Arcanum → Validator → Codegen → Backend → Assembler → Output
```

- Native code (outside `@@system` blocks) passes through unchanged ("Oceans Model")
- `@@system` blocks are expanded into full state machine implementations
- Each language backend is in `framec/src/frame_c/compiler/codegen/backends/<lang>.rs`

## Key Directories

```
framec/src/
├── main.rs                          # CLI entry point
├── lib.rs                           # WASM entry point
└── frame_c/
    ├── cli.rs                       # Argument parsing, subcommands
    ├── driver.rs                    # File I/O, dispatches to pipeline
    ├── compiler/
    │   ├── codegen/
    │   │   ├── backends/            # One file per target language
    │   │   ├── frame_expansion.rs   # Frame statement → target code
    │   │   ├── state_dispatch.rs    # State methods, event dispatch
    │   │   ├── interface_gen.rs     # Interface wrappers
    │   │   └── runtime.rs           # FrameEvent, Compartment structs
    │   ├── lexer/                   # Tokenization
    │   ├── pipeline_parser/         # Token → AST
    │   ├── segmenter/               # Split prolog/system/epilog
    │   ├── assembler/               # Reassemble output
    │   ├── arcanum.rs               # Symbol table
    │   ├── frame_validator.rs       # Semantic validation
    │   └── pipeline/compiler.rs     # Orchestrates all stages
    └── visitors/mod.rs              # TargetLanguage enum
```

## Common Tasks

### Fixing a backend bug
1. Identify the failing test in the test env
2. Look at the generated code: `framec compile -l <target> -o /tmp <test>.f<ext>`
3. Find the codegen in `compiler/codegen/backends/<lang>.rs` or `frame_expansion.rs`
4. Fix, rebuild, run: `make test-<lang>` in the test env

### Adding a feature to all backends
1. Update `frame_expansion.rs` or `state_dispatch.rs` for the shared logic
2. Add per-language cases in the `match lang { ... }` blocks
3. Run `make test` to validate all languages

### Frame syntax reference
- `-> $State` — transition
- `=> $^` — forward to parent
- `push$` / `pop$` — state stack operations
- `@@:return` — set return value
- `@@:params["key"]` — access interface parameter
- `@@:event` — current event name
- `@@:data["key"]` — call-scoped data
- `$.varName` — state variable

## Rules

- Never edit generated files — always edit the source that generates them
- Frame has no type system — types are opaque strings passed through as `Type::Custom(String)`
- All 17 backends must be kept in sync for shared features
- Test files are in the test env repo, not here
