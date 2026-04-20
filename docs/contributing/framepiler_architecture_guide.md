# Framepiler Architecture Guide

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

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
| `state_args` | Arguments passed via `-> $State(args)` OR via the system header `$(name: type)` for the start state. Keyed by the declared param name. |
| `state_vars` | State variables declared with `$.varName` |
| `enter_args` | Arguments passed via `-> (args) $State` OR via the system header `$>(name: type)` for the start state. Keyed by the declared param name. |
| `exit_args` | Arguments for the `<$` exit handler |
| `forward_event` | Stashed event for `-> =>` forwarding |

For state and enter args, **read sites and write sites both use the declared param name as the key, never a positional integer index.** This convention is enforced across all backends. The system constructor and the transition codegen both write under the name; the state dispatch and the enter handler binding both read by the name.

Rust is the exception. It has no `state_args` HashMap on the compartment (the typed enum `StateContext` doesn't support type erasure cleanly). Instead, system header state and enter args are stored as typed `__sys_<name>: <type>` fields directly on the system struct, and the start state's handler bodies bind them as locals via a preamble. See `system_codegen.rs` and `state_dispatch.rs::generate_handler_from_arcanum` for the Rust-specific path.

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

## Type Wrapping in Codegen

Two target languages — Rust and C++ — have **typed runtime storage** that requires exact type matching between the value stored and the type used to retrieve it. Frame's codegen automatically wraps portable expressions to match these type systems.

### Why wrapping exists

- **Rust**: `Box<dyn Any>` + `downcast::<T>()`. A string literal `"x"` is `&str`, but `downcast::<String>()` expects `String`. Without wrapping, the downcast fails at runtime.
- **C++**: `std::any` + `std::any_cast<T>()`. A string literal `"x"` is `const char*`, but `any_cast<std::string>()` expects `std::string`. Without wrapping, the cast throws `bad_any_cast`.

### Canonical utilities

- **`typed_init_expr(expr, var_type, lang)`** — `codegen_utils.rs`. Wraps a parsed Expression for a known target type. Used by the Rust XContext Default impl for state var inits. Handles Rust `String::from("...")`, C++ `std::string("...")`, and the parser-fallback case (integer where String expected → `String::new()`).
- **`cpp_wrap_any_arg(arg)`** — `codegen_utils.rs`. Wraps a raw string argument for C++ `std::any` storage. If the argument looks like a string literal (`starts_with('"') && ends_with('"')`), wraps in `std::string(...)`. Used by transition codegen for exit_args, enter_args, and state_args.

### String literal detection

The wrapping pattern `starts_with('"') && ends_with('"')` detects string literals. This works for:
- Empty strings: `""`
- Regular strings: `"hello"`
- Escaped quotes: `"hello \"world\""`

It does NOT detect:
- Computed strings: `format!("hello {}", name)` — already returns `String` in Rust
- Variable references: `self.name` — already the correct type from state var expansion
- Template literals or string concatenation — the user's responsibility to match types

### Copy vs non-Copy clone decision

When the Rust splicer expands `$.varName` for read access, it generates a `match &__sv_comp.state_context` pattern that borrows the state context. Non-Copy types (like `String`) need `.clone()` to extract an owned value from the shared reference. The expansion checks `ctx.state_var_types` to decide:

- **Copy types** (`i32`, `i64`, `u32`, `u64`, `f32`, `f64`, `bool`, `int`, `float`): no `.clone()` needed
- **Non-Copy types** (`String`, `str`, or any type not in the Copy list): `.clone()` added
- **Unknown type** (not in `state_var_types`): `.clone()` added for safety

### Unit test coverage

- `codegen_utils::tests` — 21 tests covering `state_var_init_value`, `typed_init_expr`, and `cpp_wrap_any_arg`
- `frame_expansion::tests` — 9 tests covering context return wrapping, state var clone, and cross-language behavior

Run: `cargo test --release codegen_utils::tests frame_expansion::tests`

## Runtime Object Lifecycle

Frame's generated runtime creates four heap-allocated object types per system. Understanding their lifecycle is critical for backends with manual memory management (C) and for debugging reference-related issues in any language.

### FrameEvent — per-call message

Created at the start of each interface method call. Carries the message name and parameter map. Destroyed at the end of the call.

- **Lifetime**: one interface call
- **Owner**: the interface wrapper method
- **C**: malloc'd, explicitly destroyed via `FrameEvent_destroy` (also frees `_parameters` dict)
- **GC languages**: GC collects after the call returns

### FrameContext — per-call context

Created alongside the FrameEvent. Holds the event reference, return slot (`_return`), and call-scoped data (`_data`). Pushed onto `_context_stack` for reentrancy tracking, popped after the call.

- **Lifetime**: one interface call
- **Owner**: the interface wrapper method
- **C**: malloc'd, explicitly destroyed via `FrameContext_destroy` (frees `_data` dict)
- **GC languages**: GC collects after pop

### Compartment — state closure

The core runtime object. Holds state name, state_vars, state_args, enter_args, exit_args, forward_event, and parent_compartment (HSM chain).

- **Lifetime**: from creation (constructor or transition) until replaced by transition or system destruction
- **Owner**: the system's `__compartment` field (current) or `_state_stack` (pushed)
- **Creation points**: `Compartment::new()` in constructor, transitions, push-with-transition
- **push$ saves a reference** — the stack and current may point to the same object (bare push$) or different objects (push-with-transition)
- **C**: reference counted (`_ref_count`). `Compartment_ref` increments, `Compartment_unref` decrements and frees at zero (cascades to parent chain)
- **C++**: `shared_ptr<Compartment>` — RAII ref counting
- **Rust**: owned values, `Clone` for bare push$, `mem::replace` for push-with-transition, `Drop` cascade
- **GC languages**: GC handles all lifecycle

### parent_compartment — HSM chain

Links child states to parent compartments in hierarchical state machines. Multiple siblings may share the same parent reference. Forms a DAG (not a cycle).

- **Lifetime**: same as the owning compartment
- **Owner**: shared reference (C: ref counted, C++: shared_ptr, GC: GC reference)
- **C**: `Compartment_ref` on assignment, `Compartment_unref` cascades through chain on destruction

### State stack — push$/pop$ history

`Vec`/`List`/`Array` of compartment references. Push adds, pop removes. All entries are cleaned up on system destruction.

- **C**: `System_destroy` walks the stack calling `Compartment_unref` on each entry, then frees the FrameVec
- **C++/Rust**: container destructor cascades to all entries
- **GC languages**: GC collects when system is unreferenced

## Alternative Output: `--format model`

The `--format model` flag produces a JSON representation of the parsed AST instead of generated code. This skips stages 3-7 (no validation, no codegen). Used by tooling (VS Code extension, visualization).
