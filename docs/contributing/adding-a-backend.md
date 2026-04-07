# Adding a New Language Backend

This guide walks you through adding a new target language to the Frame transpiler. A backend is the final stage of the compilation pipeline — it takes Frame's intermediate representation and emits code in your target language.

## Overview

Adding a backend requires these deliverables:

1. **SyntaxSkipper** — recognizes your language's strings, comments, and block delimiters so the segmenter can find `@@system` blocks in native code
2. **BodyCloser** — understands how blocks end in your language (braces, indentation, `end`, etc.)
3. **ImportScanner** — detects import/require/use statements so they can be preserved correctly
4. **Backend codegen file** — the main implementation that emits target language code from Frame IR
5. **Pipeline registration** — register the new language in the driver and CLI
6. **Tests** — compilation tests and integration tests

## Before You Start

Study an existing backend that's close to your target language:

| Your language is like... | Study this backend |
|---|---|
| Class-based, braces, `this` | `typescript.rs` or `java.rs` |
| Class-based, indentation | `python.rs` |
| Struct-based, braces | `rust.rs` or `go.rs` |
| Function-based | `c.rs` |

All backends live in `framec/src/frame_c/compiler/codegen/backends/`.

## Step 1: SyntaxSkipper

Location: `framec/src/frame_c/compiler/native_region_scanner/`

The syntax skipper tells the segmenter how to skip over your language's:

- String literals (single-quoted, double-quoted, template strings, raw strings, etc.)
- Comments (line comments, block comments)
- Block delimiters (braces, indentation, keywords)

This ensures the segmenter doesn't mistake a `@@system` inside a string or comment for a real Frame block.

Create a `.frs` grammar file and its corresponding `.gen.rs` generated parser, following the pattern of existing skippers.

## Step 2: BodyCloser

Location: `framec/src/frame_c/compiler/body_closer/`

The body closer understands how your language terminates blocks. For brace-delimited languages (C, Java, TypeScript), this is straightforward — count braces. For indentation-based languages (Python, GDScript), it needs to track indentation levels.

## Step 3: ImportScanner

Location: `framec/src/frame_c/compiler/import_scanner/`

The import scanner detects import statements in the preamble so they can be correctly positioned in the output. Each language has its own import syntax (`import`, `require`, `use`, `#include`, etc.).

## Step 4: Backend Codegen

Location: `framec/src/frame_c/compiler/codegen/backends/your_language.rs`

This is the main implementation. The backend receives Frame's IR (CodegenNode tree) and emits target language code. Key responsibilities:

- **System structure** — class/struct declaration, constructor, fields
- **State methods** — one dispatch method per state
- **Kernel/router/transition** — the runtime infrastructure methods
- **Compartment** — the state data structure (inner class, struct, or equivalent)
- **Interface methods** — public API that creates events and calls the kernel
- **Actions/operations** — private/public helper methods
- **Domain variables** — instance field declarations and initialization

The backend must emit code that implements Frame's runtime semantics:

- Deferred transitions (handler sets next state, kernel processes after handler returns)
- Enter/exit lifecycle events
- Event forwarding
- State stack operations (if used)
- Context stack (if `frame_event` is enabled)

## Step 5: Pipeline Registration

Register your language in:

- `framec/src/frame_c/driver.rs` — add a target language enum variant and metadata (name, extension, core/experimental)
- `framec/src/frame_c/cli.rs` — add CLI flag support

## Step 6: Tests

At minimum:

- **Compilation tests** — Frame files that compile without error for your language
- **Integration tests** — transpile Frame files, run the generated code in your language's runtime, verify behavior

Follow the existing test patterns in the codebase.

## Tips

- Start with a simple system (one state, one handler) and get it compiling end-to-end before tackling advanced features
- Use the `--format model` flag to see the IR that your backend will receive
- Run `cargo clippy -- -D warnings` frequently — CI enforces zero warnings
- Test with the common test fixtures to ensure parity with other backends

## Questions?

Open an issue with the [backend request template](https://github.com/frame-lang/framepiler/issues/new?template=backend_request.yml) to discuss your planned backend before starting.
