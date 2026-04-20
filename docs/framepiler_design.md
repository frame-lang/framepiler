# Framepiler Design

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

Architecture and internals of the Frame transpiler. For the Frame language itself, see [Frame Language Reference](frame_language.md). For the generated runtime, see [Frame Runtime Architecture](frame_runtime.md).

## Table of Contents

- [Design Principles](#design-principles)
- [Pipeline Overview](#pipeline-overview)
- [Stage 0: Segmenter](#stage-0-segmenter)
- [Stage 1: Lexer](#stage-1-lexer)
- [Stage 2: Parser](#stage-2-parser)
- [Stage 3: Arcanum](#stage-3-arcanum)
- [Stage 4: Validator](#stage-4-validator)
- [Stage 5: Codegen](#stage-5-codegen)
- [Stage 6: Backend Emitter](#stage-6-backend-emitter)
- [Stage 7: Assembler](#stage-7-assembler)
- [Dispatch Architecture](#dispatch-architecture)
- [GraphViz Pipeline](#graphviz-pipeline)
- [Scanner Infrastructure](#scanner-infrastructure)
- [File Structure](#file-structure)

---

## Design Principles

**Classical compiler architecture.** Each pipeline stage has a single responsibility, receives immutable input, and produces new output. No stage mutates a previous stage's output.

**Only the Segmenter touches raw bytes for boundary detection.** Native/Frame boundaries are identified once, then downstream stages work with structured data.

**The Oceans Model is a Segmenter concern.** The Lexer, Parser, Arcanum, Validator, and Codegen know nothing about native prolog/epilog. They only see Frame constructs.

**Fail early, fail hard.** Every stage either succeeds completely or produces clear diagnostics. No silent recovery or fallbacks.

---

## Pipeline Overview

```
Source File (raw bytes)
    │
    ▼
┌──────────────────────────────────────────────┐
│  Stage 0: Segmenter                          │
│  Separate native ocean from Frame islands    │
│  Output: SourceMap                           │
└──────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────┐
│  Stage 1: Lexer                              │
│  Tokenize @@system block                     │
│  Output: TokenStream                         │
└──────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────┐
│  Stage 2: Parser                             │
│  Build AST from tokens                       │
│  Output: SystemAst                           │
└──────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────┐
│  Stage 3: Arcanum (Symbol Table)             │
│  Catalog systems, states, events, variables  │
│  Output: Arcanum                             │
└──────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────┐
│  Stage 4: Validator                          │
│  Semantic checks                             │
│  Output: Vec<Diagnostic>                     │
└──────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────┐
│  Stage 5: Codegen                            │
│  AST → CodegenNode IR                        │
│  Output: CodegenNode tree                    │
└──────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────┐
│  Stage 6: Backend Emitter                    │
│  IR → target language source                 │
│  Output: String                              │
└──────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────┐
│  Stage 7: Assembler                          │
│  Stitch native + generated + native          │
│  Output: Final source file                   │
└──────────────────────────────────────────────┘
```

---

## Stage 0: Segmenter

Scans raw source bytes and produces a `SourceMap` — an ordered list of segments partitioning the entire file into typed regions.

```rust
pub fn segment(source: &[u8], lang: TargetLanguage) -> Result<SourceMap, SegmentError>;
```

### SourceMap

```rust
pub struct SourceMap {
    pub segments: Vec<Segment>,
    pub source: Vec<u8>,
    pub target: TargetLanguage,
}

pub enum Segment {
    Native { span: Span },
    Pragma { kind: PragmaKind, span: Span, value: Option<String> },
    System { outer_span: Span, body_span: Span, name: String },
}
```

Uses language-specific `SyntaxSkipper` to avoid false `@@` detection inside strings and comments. Uses `BodyCloser` to find matching `}` for `@@system` blocks.

---

## Stage 1: Lexer

Tokenizes `@@system` block bytes. Operates in two modes:

- **Structural mode**: Frame keywords, identifiers, operators, delimiters
- **Native-aware mode**: Activated inside handler/action/operation bodies. Recognizes Frame constructs (`-> $`, `=> $^`, `push$`, `pop$`, `$.`, `@@`) and passes everything else through as `NativeCode` tokens

Uses `SyntaxSkipper` to skip native strings/comments during Frame construct detection.

---

## Stage 2: Parser

Recursive descent parser consuming the token stream. Builds a `SystemAst` containing:

- System name and parameters
- Interface method declarations
- Machine with states, state variables, handlers
- Handler bodies as interleaved `Statement::NativeCode` and Frame statement nodes
- Actions, operations, domain variables

---

## Stage 3: Arcanum

Symbol table built from `SystemAst`:

- Catalogs all systems, states, events, variables, interface methods
- Resolves HSM parent-child relationships
- Computes effective codegen configuration (auto-enables `frame_event` when needed)
- Builds per-state handler registry

---

## Stage 4: Validator

Semantic checks against AST + Arcanum:

| Code | Check |
|------|-------|
| E116 | Duplicate state name |
| E400 | Code after terminal statement |
| E401 | Frame syntax in action/operation |
| E402 | Unknown transition target |
| E403 | `=> $^` without parent |
| E405 | Parameter arity mismatch |
| E410 | Duplicate state variable |
| E413 | HSM cycle |
| E601 | `@@:self.method()` targets unknown interface method |
| E602 | `@@:self.method()` argument count mismatch |

---

## Stage 5: Codegen

Transforms `SystemAst` + `Arcanum` into a `CodegenNode` tree — a language-agnostic intermediate representation.

### CodegenNode IR

```rust
pub enum CodegenNode {
    // Structural
    Module { imports, items },
    Class { name, fields, methods, base_classes, derives },
    Constructor { params, body, super_call },
    Method { name, params, return_type, body, is_async, is_static, visibility },

    // Statements
    VarDecl { name, type_annotation, init, is_const },
    Assignment { target, value },
    Return { value },
    If { condition, then_block, else_block },
    Match { scrutinee, arms },
    While { condition, body },
    For { var, iterable, body },

    // Frame-specific
    Transition { target_state, exit_args, enter_args, state_args, indent },
    ChangeState { target_state, state_args, indent },  // internal IR node, not exposed to Frame users
    Forward { to_parent, indent },
    StackPush { indent },
    StackPop { indent },
    SelfInterfaceCall { method_name, args, has_return },

    // Expressions
    Literal, Ident, BinaryOp, UnaryOp, Call, MethodCall,
    FieldAccess, IndexAccess, SelfRef, Array, Dict, Ternary,
    Lambda, Cast, New,

    // Native code
    NativeBlock { code, span },
}
```

> **Note:** `ChangeState` is an internal IR node, not exposed to Frame users. The `->>` (change-state) operator was removed in Frame V4.

### NativeRegionScanner

Scans handler body bytes for Frame constructs within native code. Each target language has its own scanner FSM that respects the language's string/comment syntax.

Recognition patterns:

| Token sequence | Region type |
|---------------|-------------|
| `-> $<ident>` | Transition |
| `-> =>` | Forwarding transition |
| `=> $^` | Forward to parent |
| `push$` | Stack push |
| `pop$` | Stack pop |
| `$.` `<ident>` | State variable |
| `@@:params.` `<ident>` | Context parameter |
| `@@:return` | Context return |
| `@@:event` | Context event |
| `@@:data.` `<ident>` | Context data |
| `@@:self.` `<ident>` `(` | Self interface call |
| `@@:system.state` | Current state accessor |

#### Self Interface Call Recognition

The scanner detects `@@:self.<ident>(` as a self-interface-call. It then uses `balanced_paren_end()` to find the closing `)`, capturing the full argument list. The complete token `@@:self.<ident>(<args>)` is emitted as a `SelfInterfaceCall` region.

The scanner distinguishes between system accessors and self-calls by the prefix and parentheses:
- `@@:system.state` — no parens → accessor (expand to state name access)
- `@@:self.method()` — parens → self-call (expand to interface call)
- `@@:self.method(a, b)` — parens with args → self-call with arguments

**Whitespace handling:** The scanner trims trailing whitespace from the
preceding native text ONLY when the Frame segment is standalone on its
line (everything between the previous newline and the segment is
whitespace). For inline usage — e.g. Swift's `let x = @@:self.method()`
— the preceding native text is preserved verbatim, so the expansion
becomes `let x = self.method()` with the space intact. Whitespace-
sensitive languages (Swift, Dart) rely on this; collapsing `= @@:self`
to `=self` would produce invalid target code. The check is a
look-back from the segment start to the previous `\n`, requiring all
intervening bytes to be space or tab to qualify as standalone.

### Splicer

Takes the scanner's region list and builds `CodegenNode` output by emitting `NativeBlock` nodes for native regions and expanding Frame regions into their `CodegenNode` equivalents.

For `SelfInterfaceCall` regions, the splicer emits a `SelfInterfaceCall` node containing the method name and arguments. The backend emitter translates this to the target language's self-call syntax.

---

## Stage 6: Backend Emitter

Each target language implements the `LanguageBackend` trait:

```rust
pub trait LanguageBackend {
    fn emit(&self, node: &CodegenNode, ctx: &mut EmitContext) -> String;
    fn target_language(&self) -> TargetLanguage;
    fn runtime_imports(&self) -> Vec<String>;
    fn class_syntax(&self) -> ClassSyntax;
}
```

17 backends: Python, TypeScript, JavaScript, Rust, C, C++, Java, C#, Go, PHP, Kotlin, Swift, Ruby, Erlang, Lua, Dart, GDScript. See the [Language Reference](frame_language.md#target) for the complete list of supported target languages.

**Erlang** bypasses the class pipeline entirely — it generates OTP `gen_statem` modules with `-behaviour(gen_statem)`, `-record(data, {...})`, and state callback functions.

### Per-Language Dispatch

| Backend | Router | State Dispatch | State Var Access |
|---------|--------|---------------|-----------------|
| Python | `if/elif` | `if/elif` | `self.__compartment.state_vars["name"]` |
| TypeScript | `switch` | `switch` | `this.#compartment.stateVars["name"]` |
| Rust | `match` | `match` | `self.__compartment.state_vars.get("name")` |
| C | `if/else if` + `strcmp` | `if/else if` | `FrameDict_get(compartment->state_vars, "name")` |

### Self Interface Call Emission

| Backend | `SelfInterfaceCall { method: "m", args: "a, b" }` |
|---------|-----|
| Python | `self.m(a, b)` |
| TypeScript | `this.m(a, b)` |
| Rust | `self.m(a, b)` |
| C | `SystemName_m(self, a, b)` |
| C++ | `this->m(a, b)` |
| Go | `s.M(a, b)` |
| Java | `this.m(a, b)` |

---

## Stage 7: Assembler

Reassembles the final output from `SourceMap` + generated code:

1. Walk `SourceMap.segments` in order
2. `Segment::Native` → extract text from source bytes, append
3. `Segment::Pragma` → skip (consumed by earlier stages)
4. `Segment::System` → look up generated code, append
5. Post-process: expand `@@SystemName()` tagged instantiations in native regions

---

## Dispatch Architecture

Three-layer dispatch in generated code:

```
Interface method call
  → Create FrameEvent + FrameContext
  → Push context onto _context_stack
  → Kernel processes event
    → Router selects state dispatch by compartment.state
      → State dispatch selects handler by event name
        → Handler method (user code + Frame expansions)
  → Pop context, return _return value
```

### Kernel

```frame
kernel(event):
    router(event)
    while next_compartment is not None:
        nc = next_compartment
        next_compartment = None
        router(FrameEvent("<$", compartment.exit_args))    // exit
        compartment = nc                                    // switch
        if nc.forward_event is None:
            router(FrameEvent("$>", compartment.enter_args)) // enter
        else:
            // forward: send $> first, then forwarded event
```

---

## Async Dispatch Per Language

Async (`async interface_method(): T`) triggers a post-pass
`make_system_async` that flips `is_async = true` on every non-static,
non-constructor method in the dispatch chain, then rewalks the method
bodies to inject the target's `await` keyword on each dispatch call.
Per-language specifics:

| Target | Signature | await injection | Entry point |
|---|---|---|---|
| Python | `async def foo(): ...` | `await self.__kernel(e)` | `asyncio.run(main())` |
| TypeScript/JavaScript | `async foo(): Promise<T>` | `await this.__kernel(e)` | `await worker.init()` |
| Rust | `async fn` + `Box::pin(async move { ... }).await` | postfix `.await` | runtime-specific |
| Dart | `Future<T> foo() async` | `await __kernel(e)` | `await worker.init()` |
| GDScript | plain `func` (no keyword) | bare `await __kernel(e)` | `await worker.init()` |
| Kotlin | `suspend fun` | **bare** (suspend→suspend calls need no keyword) | `runBlocking { worker.init() }` |
| Swift | `func foo() async -> T` | `await __kernel(e)` | `Task { await w.initAsync() }` |
| C# | `async Task<T>` | `await __kernel(e)` | `await Main(args)` |
| Java | public interface only: `CompletableFuture<T>` | (sync dispatch, no await) | `worker.init().get()` |
| C++23 | `FrameTask<T>` | `co_await __kernel(e)` | `worker.init().get()` |

### Java — interface-only async

Java has no native `async`/`await` keyword. The dispatch chain stays
synchronous; only the *public* interface methods are marked async, and
they wrap their result in `CompletableFuture.completedFuture(...)` at
return. This keeps the internal call graph tight (no `.thenCompose(...)`
chains through `__kernel` → `__router` → `_state_X`) while exposing
a future-shaped API. Implemented in
`system_codegen.rs::make_java_interface_async`, which runs instead of
the generic `make_system_async` for Java.

Callers: `String s = worker.get_status().get();`

### C++ — FrameTask<T> coroutines

C++23 async uses a self-contained coroutine promise type emitted at
file scope (header-guarded `FRAME_TASK_PRELUDE` in `backends/cpp.rs`)
before any async class:

```cpp
template <typename T>
struct FrameTask {
    struct promise_type {
        T value_{};
        std::exception_ptr err_;
        FrameTask get_return_object() noexcept { ... }
        std::suspend_never initial_suspend() noexcept { return {}; }
        std::suspend_always final_suspend() noexcept { return {}; }
        template <typename U> void return_value(U&& v) { ... }
        void unhandled_exception() noexcept { ... }
    };
    // move-only handle management + get() accessor
    T get() { ... }          // caller extracts here
    bool await_ready() const noexcept { ... }
    T await_resume() { ... }
};
template <> struct FrameTask<void> { /* return_void specialization */ };
```

Design notes:

- **`suspend_never` initial** — the coroutine body starts running as
  soon as it's constructed. There's no scheduler involved; Frame's
  state machine has no true async I/O, so `co_await` just threads
  return values through nested coroutines.
- **`suspend_always` final** — the handle lives until `.get()` / the
  destructor, so the caller can extract the return value after
  `co_return`.
- **Post-pass `rewrite_return_to_co_return`** — the state-dispatch
  and frame-expansion emitters sprinkle plain `return;` / `return expr;`
  at transition/forward sites (~20 emit points). Plain `return` inside
  a coroutine is ill-formed, so the Cpp backend rewrites each one to
  `co_return` before emitting the method body.
- **Multi-@@system files** — the `#ifndef FRAME_TASK_H` guard prevents
  template redefinition when more than one async class lives in the
  same translation unit (e.g. `33_ai_agent.fcpp`).
- **Target flag** — C++ target must compile with `-std=c++23` (or
  C++20+). Framec accepts `cpp`, `cpp_17`, `cpp_20`, `cpp_23` as
  aliases for the Cpp backend.

### C — double-return marshalling

The C `FrameContext._return` slot is a `void*`. Integer and pointer
return values round-trip cleanly through `(intptr_t)` casts; doubles
don't — `(intptr_t)(3.14)` truncates the fractional part. For handlers
with `float`/`double` return types the runtime emitter (`runtime.rs`)
emits per-system helpers:

```c
static inline void* Sys_pack_double(double v) {
    void* p = 0;
    memcpy(&p, &v, sizeof(double));
    return p;
}
static inline double Sys_unpack_double(void* p) {
    double d;
    memcpy(&d, &p, sizeof(double));
    return d;
}
```

Emit sites (`state_dispatch.rs`, `frame_expansion.rs`,
`interface_gen.rs`) branch on the handler's declared return type via
`HandlerContext.current_return_type`, falling back to `(void*)(intptr_t)`
for non-double types. Safe on every 64-bit target (both `void*` and
`double` are 8 bytes).

The same C backend also now carries pointer-typed parameters through
state args (`fmt_bind_param`) and event args (`fmt_unpack`) — any type
ending in `*` is emitted as-is instead of being cast to `int`
through `intptr_t`.

### Erlang — @@:self via frame_dispatch__

Erlang's `@@:self.method(args)` routes through the already-generated
`frame_dispatch__` helper, which invokes the current state function
directly (bypassing `gen_statem:call`, which would deadlock on
`self()`) and extracts the reply action:

```erlang
Baseline = element(2, frame_dispatch__(get_base, [], Data)),
```

**Known limitation:** `frame_dispatch__` returns `{NewData, RetVal}`.
The current expansion takes only `element(2, ...)`, dropping
`NewData` — so a state transition inside a @@:self-called handler is
lost. Pure-query @@:self (the case exercised by `39_self_call.ferl`)
works end-to-end. The transition-guard scenarios in the Python/Go
versions of the `@@:self` tests remain `@@skip`'d on Erlang; fixing
them requires a compile-time rewrite pass that renames `Data` →
`Data1` → `Data2` after each dispatch call in the handler body.

---

## GraphViz Pipeline

GraphViz bypasses the `CodegenNode` IR (designed for imperative code, wrong abstraction for graphs). Instead:

```
SystemAst + Arcanum → SystemGraph (graph IR) → DOT emitter → DOT text
```

The `SystemGraph` IR captures states, transitions, HSM hierarchy, and handler metadata. The DOT emitter produces valid DOT that the VS Code extension renders via `@viz-js/viz` (GraphViz compiled to WASM).

---

## Scanner Infrastructure

Frame uses 44 Frame-generated state machines (`.frs` → `.gen.rs`) for scanning:

| Category | Count | Purpose |
|----------|-------|---------|
| SyntaxSkipper FSMs | 15 | Per-language comment/string skipping |
| BodyCloser FSMs | 15 | Per-language brace matching |
| Scope Scanner FSMs | 1 | Erlang `fun...end` closure detection |
| Sub-machine FSMs | 3 | Expression scanning, context parsing, state var parsing |

Each language's `SyntaxSkipper` implements:

```rust
pub trait SyntaxSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser>;
    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize>;
    fn skip_string(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize>;
    fn find_line_end(&self, bytes: &[u8], start: usize, end: usize) -> usize;
    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize>;
    fn skip_nested_scope(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize>;
}
```

---

## File Structure

```
framec/src/
├── main.rs                              # CLI entry point
├── lib.rs                               # WASM entry point
└── frame_c/
    ├── cli.rs                           # Argument parsing
    ├── driver.rs                        # File I/O, dispatches to pipeline
    ├── compiler/
    │   ├── pipeline/
    │   │   ├── compiler.rs              # Orchestrates all stages
    │   │   └── config.rs                # Pipeline configuration
    │   ├── segmenter/mod.rs             # Stage 0: Source segmentation
    │   ├── lexer/mod.rs                 # Stage 1: Tokenization
    │   ├── pipeline_parser/mod.rs       # Stage 2: Token → AST
    │   ├── arcanum.rs                   # Stage 3: Symbol table
    │   ├── frame_validator.rs           # Stage 4: Semantic validation
    │   ├── codegen/
    │   │   ├── ast.rs                   # CodegenNode IR
    │   │   ├── system_codegen.rs        # Stage 5: AST → IR
    │   │   ├── frame_expansion.rs       # Frame statement → target code
    │   │   ├── state_dispatch.rs        # State methods, event dispatch
    │   │   ├── interface_gen.rs         # Interface wrappers, persistence
    │   │   ├── runtime.rs              # FrameEvent, Compartment generation
    │   │   ├── backend.rs              # LanguageBackend trait
    │   │   ├── backends/               # 17 language emitters
    │   │   ├── erlang_system.rs        # Erlang gen_statem bypass
    │   │   └── block_transform.rs      # Output block transformation (Lua)
    │   ├── assembler/mod.rs            # Stage 7: Output assembly
    │   ├── graphviz/                   # GraphViz IR + DOT emitter
    │   ├── native_region_scanner/      # Frame statement detection in native code
    │   │   ├── unified.rs              # Shared scanner + SyntaxSkipper trait
    │   │   ├── <lang>.rs               # Per-language skipper glue
    │   │   ├── <lang>_skipper.frs      # Frame FSM spec
    │   │   └── <lang>_skipper.gen.rs   # Generated FSM
    │   ├── body_closer/                # Per-language brace matching FSMs
    │   └── splice.rs                   # Splicer: interleave native + Frame
    └── visitors/mod.rs                 # TargetLanguage enum
```