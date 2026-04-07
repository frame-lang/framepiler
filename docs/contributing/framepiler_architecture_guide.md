# Framepiler Architecture Guide

This document describes the internal architecture of the Frame transpiler for contributors who want to understand, modify, or extend the codebase.

## The Oceans Model

Frame's compilation model treats native code as pass-through. A Frame source file has three regions:

1. **Prolog** — native code before the first `@@system` block
2. **System blocks** — one or more `@@system { ... }` specifications
3. **Epilog** — native code after the last system block

The framepiler parses and expands only the system blocks. The prolog and epilog are passed through verbatim. The final output is the reassembly of: prolog + expanded systems + epilog.

This is the "Oceans Model" — native code is the ocean, system blocks are islands. The framepiler only touches the islands.

## Compilation Pipeline

Frame source flows through 8 stages:

```
Source File
    │
    ▼
┌──────────────┐
│ 0. Segmenter │ Split into prolog / system blocks / epilog
└──────┬───────┘
       │ (system blocks only)
       ▼
┌──────────────┐
│ 1. Lexer     │ Tokenize Frame syntax
└──────┬───────┘
       ▼
┌──────────────┐
│ 2. Parser    │ Build abstract syntax tree (AST)
└──────┬───────┘
       ▼
┌──────────────┐
│ 3. Arcanum   │ Build symbol table
└──────┬───────┘
       ▼
┌──────────────┐
│ 4. Validator │ Semantic checks (40+ error codes)
└──────┬───────┘
       ▼
┌──────────────┐
│ 5. Codegen   │ AST → CodegenNode IR tree
└──────┬───────┘
       ▼
┌──────────────┐
│ 6. Backend   │ CodegenNode IR → target language text
└──────┬───────┘
       ▼
┌──────────────┐
│ 7. Assembler │ Reassemble prolog + generated code + epilog
└──────┴───────┘
       │
       ▼
   Output File
```

### Stage 0: Segmenter

The segmenter scans the source file for `@@system` boundaries. It must understand the target language's string and comment syntax to avoid false matches (e.g., `@@system` inside a string literal). This is handled by language-specific **SyntaxSkippers**.

Location: `framec/src/frame_c/compiler/segmenter/`

### Stage 1: Lexer

Tokenizes Frame syntax within system blocks. Produces a token stream for the parser.

Location: `framec/src/frame_c/compiler/lexer/`

### Stage 2: Parser

Builds the AST from the token stream. The AST models: system declarations, interface methods, states, handlers, transitions, variable declarations, and all Frame constructs.

Location: `framec/src/frame_c/compiler/parser.rs`, `frame_parser.rs`

### Stage 3: Arcanum

The symbol table builder. Resolves state references, validates parent chains, catalogs variables, and collects metadata needed by later stages.

Location: `framec/src/frame_c/compiler/arcanum.rs`

### Stage 4: Validator

Runs semantic validation passes over the AST. Enforces rules like:

- No transitions in actions (E401)
- No unknown state targets (E402)
- No `=> $^` without a parent (E403)
- No HSM cycles (E413)
- Arity matching for parameters (E405)
- Unreachable code detection (E400)

Location: `framec/src/frame_c/compiler/validation/`

### Stage 5: Codegen

Transforms the AST into a `CodegenNode` intermediate representation (IR). The IR is a tree of language-agnostic nodes that represent the generated code structure: class declarations, method definitions, dispatch logic, transition infrastructure, etc.

Location: `framec/src/frame_c/compiler/codegen/`

### Stage 6: Backend

Each target language has a backend that walks the `CodegenNode` IR and emits text in the target language. The backend handles language-specific concerns: syntax, type mapping, class vs. struct, indentation vs. braces, async patterns, etc.

Location: `framec/src/frame_c/compiler/codegen/backends/` (one file per language)

### Stage 7: Assembler

Reassembles the final output: prolog (verbatim) + generated system code + epilog (verbatim).

Location: `framec/src/frame_c/compiler/` (assembler modules)

## Runtime Architecture

The framepiler generates a self-contained class (or struct) for each `@@system` block. The generated code implements these runtime concepts:

### Compartment

The **compartment** is Frame's central data structure — a closure that captures the complete context of a state:

| Field | Purpose |
|-------|---------|
| `state` | Current state identifier (string) |
| `state_args` | Arguments passed via `-> $State(args)` |
| `state_vars` | State variables declared with `$.varName` |
| `enter_args` | Arguments for the `$>` enter handler |
| `exit_args` | Arguments for the `<$` exit handler |
| `forward_event` | Stashed event for `-> =>` forwarding |

The state stack is a stack of compartments. When `push$` executes, the entire compartment is copied onto the stack. When `-> pop$` executes, the saved compartment is restored, preserving all state variables.

### Kernel

The **kernel** (`__kernel`) is the central event processing loop. It:

1. Routes the event to the current state via the **router** (`__router`)
2. Checks for a pending transition (`__next_compartment`)
3. If a transition is pending:
   - Sends the exit event (`<$`) to the current state
   - Switches to the new compartment
   - Sends the enter event (`$>`) to the new state — OR handles event forwarding
4. Repeats until no more transitions are pending

This implements the **deferred transition model**: handlers call `__transition()` to record a target, but the actual state switch happens in the kernel after the handler returns.

### Router

The **router** (`__router`) dispatches events to the correct state method by name. In most backends this is a dynamic dispatch: look up `_state_{state_name}` and call it.

### State Methods

Each state generates a dispatch method (`_state_StateName`) that switches on the event message and calls the appropriate handler code. This is where the user's handler code lives in the generated output.

### FrameEvent and FrameContext

When `frame_event` mode is enabled (auto-enabled for advanced features):

- **FrameEvent** — a lean routing object: `{ _message: string, _parameters: dict }`
- **FrameContext** — call context for interface methods: `{ event, _return, _data }`. Pushed onto a `_context_stack` for reentrancy support.

The context provides `@@.param`, `@@:return`, `@@:event`, and `@@:data[key]` access in Frame source.

## Generated System Structure

```
class System:
    # Inner types
    SystemCompartment        # State data structure
    FrameEvent              # Event routing (if frame_event=on)
    FrameContext            # Call context (if frame_event=on)

    # Instance fields
    __compartment           # Current state's compartment
    __next_compartment      # Pending transition target (deferred model)
    _state_stack            # Compartment stack (if push$/pop$ used)
    _context_stack          # Interface call context stack
    <domain variables>      # From domain: section

    # Runtime infrastructure
    __kernel()              # Event processing loop
    __router()              # Dispatch to state method
    __transition()          # Record pending transition
    _state_X()              # Per-state dispatch method

    # User-defined
    interface_methods()     # Public API → creates event → calls kernel
    __action_methods()      # Private helpers (actions:)
    operation_methods()     # Public, bypass kernel (operations:)
```

## Async Architecture

When any interface method is declared `async`:

1. The kernel, router, and all state dispatch methods become async
2. ALL interface methods become async (shared dispatch path)
3. Internal calls use `await`
4. Constructors remain sync — a generated `init()` method handles async enter events (two-phase init)

The async transformation is mechanical: `def` → `async def`, `self.method()` → `await self.method()` for all dispatch calls. The specific syntax varies by backend.

Languages with "one-color" concurrency (Go, Java 21+) ignore `async` entirely — the synchronous generated code works correctly because concurrency is handled by the runtime.

## Key Source Files

| File | Purpose |
|------|---------|
| `framec/src/main.rs` | CLI entry point |
| `framec/src/lib.rs` | Library + WASM entry |
| `framec/src/frame_c/cli.rs` | Argument parsing, flag definitions |
| `framec/src/frame_c/driver.rs` | Target language enum and metadata |
| `framec/src/frame_c/config.rs` | Configuration handling |
| `framec/src/frame_c/compiler/parser.rs` | Main parser |
| `framec/src/frame_c/compiler/ast.rs` | AST definitions |
| `framec/src/frame_c/compiler/arcanum.rs` | Symbol table |
| `framec/src/frame_c/compiler/frame_validator.rs` | Validation entry point |
| `framec/src/frame_c/compiler/codegen/` | IR generation and backends |
| `framec/src/frame_c/compiler/codegen/backends/` | One file per target language |
| `framec/src/frame_c/compiler/pipeline/` | Pipeline orchestration |

## Language Infrastructure

Each target language requires three support modules in addition to its backend:

### SyntaxSkipper
Location: `compiler/native_region_scanner/`

Recognizes strings, comments, and block delimiters in the target language. Used by the segmenter to correctly identify `@@system` boundaries in native code.

### BodyCloser
Location: `compiler/body_closer/`

Understands how blocks terminate in the target language. Brace-delimited languages count braces. Indentation-based languages track indent levels.

### ImportScanner
Location: `compiler/import_scanner/`

Detects import/require/use statements so they can be correctly positioned in the output.

## Adding a Backend

See the step-by-step guide: [Adding a Backend](adding-a-backend.md)

## Error Codes

Error codes are organized by range:

| Range | Category |
|-------|----------|
| E0xx | Parse errors |
| E1xx | Structural errors (missing target, duplicate states, etc.) |
| E4xx | Semantic errors (forbidden syntax, unknown states, etc.) |
| W4xx | Warnings |

Location: `framec/src/frame_c/compiler/validation/`

## Alternative Output: `--format model`

The `--format model` flag produces a JSON representation of the parsed AST instead of generated code. This skips stages 3-7 (no validation, no codegen). Used by tooling (VS Code extension, visualization).
