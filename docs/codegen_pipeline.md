# framec Codegen Pipeline

This document maps the codegen pipeline at
`framec/src/frame_c/compiler/codegen/`. It is the architecture-level
companion to the per-file `//!` docs and to [RFC-0020](rfcs/rfc-0020.md)
(runtime reference architecture).

## Pipeline overview

```text
Frame source (.fc / .fpy / .fgd / etc.)
       │
       ▼
parser/lexer
       │
       ▼
SystemAst + MachineAst + Arcanum (semantic enrichment)
       │
       ▼
generate_system()   in codegen/system_codegen.rs
       │
       ├─►  Per-system support types  (runtime.rs)
       │       FrameEvent, FrameContext, Compartment classes
       │
       ├─►  Machinery primitives      (machinery.rs + machinery/<lang>.rs)
       │       __kernel, __router, __transition, HSM helpers
       │       Contract: RFC-0020
       │
       ├─►  Per-state dispatchers     (state_dispatch.rs)
       │       _state_<State>(event, compartment) functions
       │       + named handler methods _s_<State>_hdl_<kind>_<event>
       │       Per-backend handlers in state_dispatch/handler_methods/<lang>.rs
       │
       ├─►  Handler bodies            (frame_expansion.rs + frame_expansion/)
       │       @@-construct lowering: @@:return, @@:self.X(), -> $S,
       │       => $^, $$+, $$-, @@:data, …
       │       One file per construct.
       │
       ├─►  Interface wrappers        (interface_gen.rs)
       │       Public method wrappers + @@[persist] save/load
       │       Persist per backend in interface_gen/persist/<lang>.rs
       │
       └─►  Constructor IR            (system_codegen/constructor.rs)
               Two-artifact factory: bare ctor + _create
               Per-backend init-event-block emission
       │
       ▼
CodegenNode tree                       (ast.rs)
       │
       ▼
backends/<lang>.rs LanguageBackend::emit  →  source text
       │
       ▼
Output file
```

**Erlang** follows a parallel path through `erlang_system.rs` because
the gen_statem callback model does not match the class-based primitive
set.

**Rust** uses Rust-specific helpers in `rust_system.rs` for kernel
signature and borrow-checker workarounds (see RFC-0020 §Exceptions).

## Module map

### Top-level files

| File | Purpose |
|---|---|
| `mod.rs` | Module declarations + re-exports. Top-level `//!` doc duplicates this map. |
| `ast.rs` | `CodegenNode` enum — language-agnostic IR. |
| `backend.rs` | `LanguageBackend` trait + `EmitContext` + backend dispatch. |
| `system_codegen.rs` | `generate_system` orchestrator + helpers. |
| `machinery.rs` | `MachineryGenerator` trait (runtime primitive contract). |
| `state_dispatch.rs` | Per-state dispatcher generation + helpers. |
| `frame_expansion.rs` | `@@`-syntax handler-body expander. |
| `interface_gen.rs` | Interface wrappers + persist. |
| `runtime.rs` | Per-system support type classes. |
| `erlang_system.rs` | gen_statem-based Erlang generator. |
| `rust_system.rs` | Rust-specific helpers. |
| `codegen_utils.rs` | Shared utilities (`HandlerContext`, type maps). |
| `block_transform.rs` | Post-pass block rewriters (async injection, etc.). |
| `output_block_*.gen.rs` | Generated Frame output-block parsers — do not hand-edit. |

### Subdirectories

| Directory | Per-file model | Purpose |
|---|---|---|
| `backends/` | One per backend (17 + helpers) | `LanguageBackend::emit` impls. |
| `machinery/` | One per backend | Runtime-primitive emitters (`emit_kernel` etc.). |
| `state_dispatch/handler_methods/` | One per backend | Handler-method body emitters. |
| `state_dispatch/` (other) | Shared | Dispatcher helpers, `dispatch_syntax.rs` (per-language formatting hooks). |
| `frame_expansion/` | One per construct | `return.rs`, `self_call.rs`, `forward.rs`, `transition.rs`, `stack.rs`, etc. |
| `interface_gen/persist/` | One per backend | `@@[persist]` save/load codegen. |
| `system_codegen/constructor.rs` | Single file with per-backend arms | Init-event-block emission. |
| `erlang_system/` | Multiple | gen_statem-specific helpers (state functions, persist, runtime helpers). |
| `runtime/` | Single file | Currently a thin re-export layer. |
| `rust_system/` | Single file | Currently a thin re-export layer. |

## Per-backend extension points

Adding a new backend touches **four** locations in lockstep:

1. **`backends/<lang>.rs`** — `LanguageBackend::emit` for every
   `CodegenNode` variant. Owns:
   - Syntactic shape (brace vs. indent, semicolons, keyword names)
   - Type spellings
   - The Constructor IR arm (`_init` / `_create` / factory layout)
   - The Module / Class wrappers

2. **`machinery/<lang>.rs`** — `MachineryGenerator` impl. Per
   RFC-0020 the contract is `__kernel(event)` + `__router(event)`
   plus HSM helpers (`__prepareEnter`, `__prepareExit`,
   `__transition`, `__hsm_chain` table). **No `__route_to_state` or
   `__process_transition_loop` methods are emitted** — the dispatch
   table inlines into `__router`, the drain loop inlines into
   `__kernel`.

3. **`state_dispatch/handler_methods/<lang>.rs`** — per-handler
   method body builder. Standard layout:
   1. State-arg binding from `compartment.state_args`
   2. Param binding from `__e._parameters` (user events) /
      `compartment.enter_args` / `compartment.exit_args` (lifecycle)
   3. State-var init guards (lifecycle `$>` only)
   4. Return-init assignment
   5. User-written handler body via
      `emit_handler_body_via_statements` with `per_handler: true`

4. **`frame_expansion/*.rs`** match arms — extend each construct
   with the new language's lowering. Sites:
   - `return.rs` — `@@:return(expr)`, `@@:(expr) return`
   - `self_call.rs` — `@@:self.method()`
   - `forward.rs` — `=> $^` (HSM ancestor forward) — see also
     the cascade-forward note below.
   - `transition.rs` — `-> $State`, `-> => $State`, args
   - `stack.rs`, `pop_transition.rs` — `$$+` / `$$-` modal stack
   - `event.rs`, `interpolation.rs`, etc.

Plus per-language entries in `state_dispatch/dispatch_syntax.rs`
(`fmt_if`, `fmt_elif`, `fmt_forward`, `fmt_init_sv`, etc.) and
shared type-mapping helpers in `codegen_utils.rs`.

## The cascade-forward contract (RFC-0019)

**framec does not auto-synthesize the HSM enter-cascade.** When a
state has state-var declarations and an HSM parent, the parent's
state-vars do NOT get initialized automatically when the child is
entered.

If you want parent state-vars to initialize, the Frame source must
include an explicit `$>() { => $^ }` handler:

```frame
$Child => $Parent {
    $.child_var: int = 10

    $>() {
        // RFC-0019: forward $> to $Parent so its state-var
        // initializer runs.
        => $^
    }

    ...
}
```

This is a **fixture-authoring requirement**, not a codegen
responsibility. Pre-RFC-0019 framec auto-cascaded; post-RFC-0019 the
cascade is opt-in via explicit forwards.

The relevant codegen sites:

- `state_dispatch.rs::generate_per_handler_methods` lines 792-833
  — synthesizes an implicit `$>` method when the state has
  state-vars but no explicit `$>`, but the synthesized body is
  *empty except for state-var init guards*. No cascade-forward is
  injected.
- `frame_expansion/forward.rs::expand_forward` — lowers `=> $^` to
  `self._state_Parent(__e, compartment.parent_compartment)` (or
  per-language equivalent). Only fires when the user wrote `=> $^`.

This contract is the migration target for memory entry #341
("Migrate the matrix corpus + demos + cookbook to RFC-0019 (no
cascade)"). 12 of 17 fixture variants were migrated; 5 fixture
variants (`.fdart`, `.fgo`, `.frs`, `.fgd`, `.ferl`) were missed and
the omissions were hidden by an unrelated GDScript matrix CI
classifier bug that masked assertion failures.

## CI matrix harness quirks

Per-backend Docker runners live at
`framec-test-env/docker/runners/`. Most use a per-test invocation
model (one process per fixture). Some batch for performance:

- **`gdscript_batch.sh`** — runs multiple tests in one godot
  process via a SceneTree harness. The awk slicer assigns rc=0 to
  every test slice (only a whole-batch timeout produces rc≠0); the
  classifier compares the last PASS / `ok N -` line to the last
  `SCRIPT ERROR` / `Assertion failed:` line to decide pass/fail.
  Tests that print PASS for early subtests then hit an unrecovered
  error are correctly classified as failing.
- **`kotlin_batch.sh`** / **`java_batch.sh`** / **`csharp_batch.sh`**
  / etc. — JVM/CLR batching for cold-start cost.
- **`cpp_batch.sh`** / **`swift_batch.sh`** / **`dart_batch.sh`** /
  **`rust_batch.sh`** — parallel compilation, then per-binary exec.

Per-language failure-detection contracts:
- **TAP-shaped output**: `^ok N -` / `^not ok N -` lines.
- **Loose PASS marker**: `PASS:`, `# PASS`, or final `ok ` line.
- **Errors signalled out-of-band** (SCRIPT ERROR on stderr/stdout):
  classifier must explicitly check.

## Local validation harnesses

Co-developed alongside RFC-0020 backend work:

- `/tmp/run_py_validation.sh` — Python via `python3`
- `/tmp/run_c_validation.sh` — C via `gcc` + libcjson
  (`/opt/homebrew/{include,lib}`)
- `/tmp/run_gd_validation.sh` — GDScript via `godot --headless`
  (15s timeout per fixture)

These mirror the Docker matrix behavior for a single backend at a
time, with stricter PASS/FAIL detection (per-fixture exit code +
last-line marker comparison). Useful for debugging during codegen
changes without spinning up the full Docker matrix.

## Reference RFCs

- [RFC-0012](rfcs/rfc-0012.md) — `@@[persist]` contract
- [RFC-0013](rfcs/rfc-0013.md) — `@@[target(...)]` syntax
- [RFC-0015](rfcs/rfc-0015.md) — `@@[create(name)]` factory rename
- [RFC-0017](rfcs/rfc-0017.md) — bare-ctor + factory split for
  `@@!Foo()`
- [RFC-0018](rfcs/rfc-0018.md) — context push for start `$>`
- [RFC-0019](rfcs/rfc-0019.md) — leaf-dispatch model for `$>` / `<$`
- [RFC-0020](rfcs/rfc-0020.md) — runtime reference architecture
  (authoritative for kernel + dispatch)
- [RFC-0021](rfcs/rfc-0021.md) — runtime perf optimizations
  (deferred)
