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

## Step 7: System Initialization Parameters

Frame supports `@@system Name(params)` headers that flow constructor arguments through to domain field initializers, the start state's `compartment.state_args`, and the start state's `compartment.enter_args`. See the [user-facing spec](../frame_language.md#system-parameters) for the three groups (domain, state, enter) and the syntax.

A new backend must wire all three groups end-to-end. The cross-cutting infrastructure already exists; the per-backend work is mechanical.

### Where the cross-cutting infrastructure lives

| Concern | File / symbol |
|---|---|
| Header span capture (segmenter) | `framec/src/frame_c/compiler/segmenter/mod.rs` — `Segment::System.header_params_span` |
| Header param parser | `framec/src/frame_c/compiler/pipeline_parser/mod.rs::parse_system_header_params` |
| AST node | `framec/src/frame_c/compiler/frame_ast.rs::SystemParam` (with `ParamKind::{Domain, StateArg, EnterArg}`) |
| Constructor builder entry | `framec/src/frame_c/compiler/codegen/system_codegen.rs` (look for `system.params` near line 1660) |
| State→param-names map | `framec/src/frame_c/compiler/codegen/codegen_utils.rs::HandlerContext.state_param_names` |
| Transition write helper | `framec/src/frame_c/compiler/codegen/frame_expansion.rs::resolve_state_arg_key` and `resolve_enter_arg_key` |

All of these are language-agnostic. Your job is to consume them in the four backend-specific places below.

### The universal rule

> **Emit domain field initialization wherever the constructor parameters are in scope, with an explicit self/this/@ prefix on the LHS. State and enter params land in the same compartment dicts that transitions populate (`state_args` / `enter_args`), keyed by the declared param name.**

Per-language, "where the constructor parameters are in scope" means:

| Language family | Where params are in scope | Field-assign syntax |
|---|---|---|
| Python, GDScript, Lua | constructor body | `self.field = field` |
| TypeScript, JavaScript, Java, Kotlin, C#, Dart | constructor body | `this.field = field;` |
| Swift | `init` body | `self.field = field` |
| C++ | constructor body or member init list | `this->field = field;` or `: field(field)` |
| Rust | struct literal in `Self::new` | `Self { field, ... }` |
| Go | factory function returning struct literal | `&S{field: field}` |
| C | `S_new(...)` factory function | `self->field = field;` |
| PHP | `__construct` body | `$this->field = $field;` |
| Ruby | `initialize` body | `@field = field` |
| Erlang | `init/N` callback, record literal | `#data{field = Field}` |

### Per-backend checklist

Six concrete steps. Each is a small change in a file that already exists.

#### 1. Class field generator (`system_codegen.rs:464`)

For each `domain_var` whose `raw_code` references a system param (use `raw_contains_word`), strip the initializer from the class declaration. Emit just the type with a neutral default. The real value is set in step 2.

#### 2. Constructor body domain loop (`system_codegen.rs:619`)

Add a `match syntax.language` branch for your backend that emits the domain field assignment in the constructor body using the language's self-prefix:

```rust
} else if matches!(syntax.language, TargetLanguage::YourLang) {
    body.push(CodegenNode::NativeBlock {
        code: format!("<self-prefix>.{}", raw_code),  // e.g. "this.{}"
        span: None,
    });
}
```

#### 3. Constructor body compartment loop

Inside the constructor's start-state-compartment-creation block, walk `system.params` once. For each entry:

- `ParamKind::StateArg` → emit `compartment.state_args[<name>] = <name>` in the language's syntax
- `ParamKind::EnterArg` → emit `compartment.enter_args[<name>] = <name>` in the language's syntax
- `ParamKind::Domain` → already handled by step 2

The write key is **always the declared name**, never the positional index. The dispatch reader (step 4 and the existing enter-handler binding) looks up by name.

#### 4. State dispatch state-args binding

In `state_dispatch.rs`, find your backend's `generate_<lang>_state_dispatch` function. At the top of the dispatch body, prepend a binding for each declared state param:

```rust
for sp in state_params {
    code.push_str(&format!(
        "<name> = <state_args lookup by name>\n"   // language-specific
    ));
}
```

This makes `state.params` (declared on the state header like `$Start(x: int)`) readable as bare locals inside handler bodies. Enter params are read by the existing enter-handler binding code; no new dispatch work needed.

#### 5. Test runner harness (`framepiler_test_env/docker/runners/runner.sh`)

Each language has its own `case` arm that constructs the system. Update yours to:

- Detect the constructor's arity from the generated code (e.g. `start_link/N` for Erlang, `Foo(a,b,c)` for OO langs)
- Parse the system header line in the source (`@@system Name(name1: type1, ...)`)  to extract the param types in declaration order
- Generate type-aware default args: `bool→false`, `str/string→""`, `int/float/number→0`, otherwise `null`/`undefined`/`nil`

Pattern your changes after the existing Erlang case in `runner.sh`.

#### 6. Specialty tests

Port the 9 `.fpy` files in `framepiler_test_env/tests/common/positive/system_params/` to your backend's extension. Each test exercises one permutation:

| Test | Permutation |
|---|---|
| `sysparam_domain_int` | single typed int domain param |
| `sysparam_domain_str` | single typed str domain param |
| `sysparam_domain_bool` | single typed bool domain param |
| `sysparam_domain_multi` | multiple mixed-type domain params |
| `sysparam_domain_default` | typed domain param with default value |
| `sysparam_state_single` | single state param via `$(name): type` |
| `sysparam_mixed` | domain param + state param in one header |
| `sysparam_enter_single` | single enter param via `$>(name): type` |
| `sysparam_enter_mixed` | all three groups in one header |

### Verification

```sh
cd /Users/marktruluck/projects/framepiler && cargo build --release
cd /Users/marktruluck/projects/framepiler_test_env/docker
rm -f framec-native .stamps/framec-native && make framec
make test-<your-lang>     # expect 9 new specialty tests passing
make test                 # full 16-language regression — expect 0 failures
```

A new backend MUST pass all 9 specialty tests with zero regressions in any other language.

## Tips

- Start with a simple system (one state, one handler) and get it compiling end-to-end before tackling advanced features
- Use the `--format model` flag to see the IR that your backend will receive
- Run `cargo clippy -- -D warnings` frequently — CI enforces zero warnings
- Test with the common test fixtures to ensure parity with other backends

## Questions?

Open an issue with the [backend request template](https://github.com/frame-lang/framepiler/issues/new?template=backend_request.yml) to discuss your planned backend before starting.

---

## Appendix: Notes from the system-init rollout

These notes capture the design decisions and lessons learned when system initialization parameters were rolled out across all 17 backends. They exist because the 6-step procedural checklist in Step 7 doesn't explain *why* — and the why matters when you hit an edge case in your new backend.

### The universal rule and why it works in 17 dialects

System params are the most cross-cutting feature in the codebase. The conceptual rule is one sentence:

> *Emit domain field initialization wherever the constructor parameters are in scope, with an explicit self/this/@ prefix on the LHS.*

We checked this rule against every language family before committing to it. It survived two extreme cases that nearly forced exceptions:

- **Erlang has no constructor.** `gen_statem` uses a `start_link/N` → `init/N` callback pair, and the system "state" is a record built from a literal expression. The rule still applies: `init/N` is "where the params are in scope" because Erlang variables bound in the function head are visible in the record literal that follows. The "self prefix" becomes the record's named-field assignment syntax: `#data{value = Value}`.

- **C has no class.** `<System>_new(int initial)` is "where the params are in scope" — a factory function that allocates a struct and returns a pointer. The "self prefix" becomes `self->field` where `self` is the pointer being constructed. No `this`, no class, but the rule still applies cleanly.

If the rule survives Erlang and C, it survives anything. New backends rarely hit a wall here; if you think you have, double-check that you're emitting initialization where the params are still in scope.

### The state_args / enter_args naming standardization

Before this rollout, transitions stored positional state and enter args under string-encoded integer keys: `state_args["0"] = first_arg`, `state_args["1"] = second_arg`. The dispatch reader pulled them out the same way. This worked because the read site and the write site agreed on the convention.

System params broke that agreement. The system constructor doesn't have positional integers — it has param names from the header. So we standardized: **all writes use the declared param name as the key, not the integer index.** Both transition codegen and the system constructor write under the name, and both the state dispatch and the enter handler binding read by the name.

The key insight is that **read and write must agree on a single naming convention**. We picked names because:

- They survive code review better than `state_args["3"]`.
- They don't break when the user reorders or renames a state's parameter list.
- They eliminate the ambiguity between system-constructor-set args and transition-set args — both look identical in the compartment.

This change required updating every transition emit site in `frame_expansion.rs` and the manual `state_args["0"]` reads in `tests/common/positive/primary/26_state_params.*` (17 versions, one per backend).

### Field/param name collision (Q2) — a non-issue

A natural worry: if the user writes

```frame
@@system Account(balance: int) {
    domain:
        balance = balance     // ambiguous?
}
```

…isn't the second `balance` ambiguous between the param and the field? **No.** The codegen prepends the language-appropriate self-reference (`self.`, `this.`, `@`, `$this->`) to the LHS of every domain field assignment. So `balance = balance` becomes `self.balance = balance` — the LHS is unambiguously the field, the RHS is unambiguously the param.

This means users never have to rename to avoid the collision. Don't reproduce hand-rolled `bal`/`who` workarounds in your test files; just write the natural form.

### Why enter params almost got deferred (and why they shouldn't have)

The first draft of the rollout deferred `$>(name)` enter params on the basis that they "visually conflict" with the existing `$>() { ... }` enter handler in state bodies. That argument was wrong. The system header `(...)` and the state body `{...}` are different parser contexts that never share scope. The deferral was based on user-readability worry, not actual ambiguity.

**Lesson: distinguish parser ambiguity from user confusion before deferring features.** The two are easily conflated in design discussions. Parser ambiguity is a hard blocker. User confusion is a documentation problem.

### Erlang's structural divergence — a case study

Erlang is the most foreign target. Here's how the universal rule maps to it concretely:

- `start_link/N` accepts the system params as positional arguments.
- The args are threaded through to `init/N` as a list: `gen_statem:start_link(?MODULE, [P1, P2, P3], [])`.
- `init([P1, P2, P3])` pattern-matches them into Erlang variables.
- Domain fields that reference params are stored in the `#data{}` record by **literal expression**: `#data{value = P1}`.
- Record field defaults can't see `init/N`'s variables (record defaults are evaluated at compile time in a different scope), so any field that references a param must use a neutral default in the record declaration (`value = undefined`) and the real value comes from the record literal in `init/N`.
- State args land in `frame_state_args = #{<<"name">> => Value}`, enter args in `frame_enter_args` the same way. Each is a binary-keyed map, populated in the record literal.
- State function clauses (the gen_statem callbacks) bind state args at the top with `Name = maps:get(<<"name">>, Data#data.frame_state_args, undefined),` — once per declared state param, regardless of which event the clause handles.

If your backend has a similar "structurally different" idiom, the same translation pattern works: identify where the constructor params are in scope, emit field assignments and compartment populations there, and make sure the dispatch reader uses the named keys.

### Open questions still unresolved (Q4 and Q6)

Two questions were deliberately left open during the rollout:

- **Q4 — default-value translation.** Defaults like `name: str = "hello"` are currently raw text pasted verbatim into the constructor signature. This works for integer literals everywhere, works for double-quoted strings in most languages, and may break for collection defaults (`[]`, `{}`) in some targets. If you hit this in your backend, surface it as a blocker — don't paper over.

- **Q6 — call-site argument disambiguation across mixed groups.** The current rule is "flat positional in declaration order" — works in every language. The spec documents it, but if a future feature wants keyword arguments at the call site (`@@Robot(name="R2D2", x=7)`), this question reopens.

Q5 (the enter-param grammar question) is closed: context disambiguates, no clash exists.

### The 9 specialty tests and what they cover

A new backend MUST pass all 9 of these. They live in `framepiler_test_env/tests/common/positive/system_params/`:

| Test | Permutation |
|---|---|
| `sysparam_domain_int` | single typed int domain param |
| `sysparam_domain_str` | single typed str domain param |
| `sysparam_domain_bool` | single typed bool domain param |
| `sysparam_domain_multi` | three params of mixed types in one header |
| `sysparam_domain_default` | typed domain param with default value |
| `sysparam_state_single` | single state param `$(x): int` |
| `sysparam_mixed` | one domain + one state param |
| `sysparam_enter_single` | single enter param `$>(b): int` |
| `sysparam_enter_mixed` | all three groups in one header |

Two tests cover the "structural" cases that tend to break unimplemented backends — `sysparam_domain_default` (defaults) and `sysparam_enter_mixed` (mixed groups). If you're debugging a new backend, start with the simpler `_int` test and work up.
