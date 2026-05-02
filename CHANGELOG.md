# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased] - 2026-05-02

### Fixed — multi-line `@@:()` return expressions on indent-sensitive targets

- Multi-line expressions inside `@@:(<expr>)`, `@@:return = <expr>`, and `@@:return(<expr>)` are now re-wrapped in `(...)` when the expanded RHS contains a newline. Without the wrap, GDScript and Python parsed the assignment up to the first newline and rejected the continuation as an `Indent` parse error. Curly-brace targets receive redundant-but-harmless parens. Matrix fixture `92_return_expr_multiline.{fpy,fgd}` covers the regression. Surfaced by `frame-arcade/ch05-pacman`.

### Added — RFC-0014 `@@[main]` (wave 1)

- **`@@[main]` system attribute** to mark the file's primary system in multi-system `.fgd` files. The primary owns the script-module slot in targets that privilege one class per file (GDScript today; Java/C#/TypeScript planned in later waves). Non-main systems wrap as inner classes (sibling-resolvable from the main system's domain initializers and from each other).
- **`SystemAst.attributes: Vec<Attribute>`** — generic system-level attribute storage parallel to RFC-0013 wave 2 phase 2's per-item attributes. RFC-0014 ships `@@[main]` as the first user; `@@[persist]` stays special-cased in `persist_attr` for backwards compatibility (a follow-up will migrate it).
- **E805** — multi-system module declares zero `@@[main]`. Hard cut at parse time with a one-line migration message.
- **E806** — multi-system module declares multiple `@@[main]`. Only one system per file may occupy the primary slot.
- **GDScript multi-system fix.** Solves the long-standing "first system silently becomes the primary" bug that broke every multi-system `.fgd` whose driver instantiated the lexically-last system (the natural authoring order: primitives first, composer last). The main system's `extends Base` directive is hoisted to the top of the file so the developer-natural source order produces a valid script.
- **Reverted** the D22 `class_name` post-pass — it didn't actually solve cross-reference resolution (Godot's inner classes can't see their own script's `class_name`) and added Godot global-namespace pollution.
- **Test corpus migrated** — 204 multi-system fixtures (88 `.fgd` and 116 across other targets) updated to mark the lexically-last system `@@[main]`, matching every test driver's instantiation pattern. New fixture `tests/common/positive/primary/91_main_attr_cross_ref.fgd` exercises the cross-reference shape end-to-end in Godot.

### Added — persist contract (wave 8 closure)

- **E700 quiescent contract for `save_state`.** Mid-event saves now error with `E700: system not quiescent` instead of producing partial / undefined snapshots. Per-backend mechanism: throw on JVM/.NET/dynamic langs/C++, panic on Rust/Go, abort on C/Swift, push_error on GDScript, implicit (gen_statem deadlock) on Erlang. Hard cut, no soft warning. Documented in `frame_runtime.md`, `rfcs/rfc-0012.md`, and all 17 per-language guides.
- **Nested `@@SystemName` persist parity across all 17 backends.** Wave 8 closure: nested-system save/restore now works on every backend including the previously-blocked C and Erlang. C uses cJSON recursive embedding; Erlang uses gen_statem process trees with `child:save_state` recursing through Pids. JVM (Java + Kotlin) gained Option A nested-system support to match the existing 12-backend rollout.
- **Erlang multi-statement handler + cross-system call** — pre-existing limitation closed: handlers that combine self-mutation with cross-system calls (e.g., `self.n = self.n + 1; self.child.bump()`) now compile correctly. Cross-system rewrite extended to match `Data1#data.field`, `Data2#data.field`, etc. (the per-statement record-update suffix that emerges in chained handlers).
- **Lua `int` domain field type coercion on persist restore** — cjson decodes JSON numbers as Lua floats by default; declared `int` fields now coerce via `math.floor()` on restore so they round-trip with the integer subtype intact (Lua 5.3+).
- **70+ new persist tests** (tests 84–88 across 14 wired backends + multi-system Erlang variants in `tests/erlang/multi/`) covering: nested HSM × persist, 3-level nested HSM × persist, numeric typing in nested persist, multi-instance independence, E700 quiescent error path, plus the existing 5-deep nested chain extended to C and Erlang.
- **RFC-0012 expanded** with three new sections marked deferred pending customer feedback: cycles in the persist graph (Option A E702 recommended), Python pickle → JSON migration, adversarial input threat model + E701 corrupted-snapshot proposal.
- **Python pickle security warning** — `frame_runtime.md` and the Python per-language guide now warn that `pickle.loads` on attacker-controlled input is RCE. JSON migration tracked in RFC-0012, deferred pending customer feedback.

### Added — annotation syntax (RFC-0013)

- **`@@[name]` and `@@[name(args)]` attribute grammar.** New C#/Java/Kotlin-style annotation form across the language. Wave 1 migrated `@@persist` → `@@[persist]` (and `@@[persist(library)]` for the library form); wave 2 migrated `@@target python_3` → `@@[target("python_3")]`. Both bare forms hard-cut: bare `@@persist` errors with **E803**, bare `@@target` errors with **E804**. Test corpus and docs migrated repo-wide (~4,800 fixtures + ~30 doc samples).
- **Per-item `@@[target("lang")]`** attached to interface methods, handlers, and domain fields. Emits the item only when compiling for the named target — useful for mixed-target docs, polyglot demos, or scaffolding language-specific shim methods. Codegen filter pass (`filter_by_target_attribute`) prunes unmatched items just before emit.
- **Validator codes**: **E800** (unknown attribute name), **E801** (attribute attached at wrong attachment position — currently fires for `@@[persist]` outside system declarations), **E802** (invalid `target` argument: missing arg or unsupported language). Filter pass runs after validation so attribute-shape errors surface even on items the filter would prune.
- **Tests 89 + 90** added: per-item conditional emit on interface methods (test 89, Python + JS) and on domain fields (test 90, Python + JS). Domain-field attribute parsing supports both same-line (`@@[target("python_3")] field: int = 0`) and own-line forms.

### Added — async (carried from prior session)

- Async codegen for six new targets: Dart (`Future<T> foo() async`), GDScript (bare `await`), Kotlin (`suspend fun`), Swift (`func … async`; async entry renamed to `initAsync` since `init` is reserved), C# (`async Task<T>`), Java (`CompletableFuture<T>` on the public interface only — internal dispatch stays sync; callers `.get()`), and C++23 (`FrameTask<T>` coroutine promise emitted header-guarded at file scope). Total: 11 of 17 targets now produce real async code.
- C backend double-return marshalling — per-system `Sys_pack_double` / `Sys_unpack_double` `memcpy`-based helpers for handlers declared `float` / `double`. The `void*` return slot previously truncated fractional parts via `intptr_t`.
- C backend pointer-type parameter support — `fmt_unpack` and `fmt_bind_param` now pass types ending in `*` through as-is instead of defaulting to `int`.
- Erlang `@@:self.method(args)` full semantics — Data threading (via the existing classifier) plus transition-guard `case ...#data.frame_current_state of` wrappers around each dispatch site, so a state change inside the called handler short-circuits the rest of the caller's body.
- GDScript `@@Foo()` in domain initializers now emits `Foo.new()` (was `Foo()`, parsed as a function call on a null instance).
- Dart persist codegen updated to match the post–HashMap→Vec compartment shape (state_args / enter_args / exit_args as `List<dynamic>`, not `Map`); `_restore()` constructor initializes `late` fields; domain-field restore uses `.cast<X>()` for typed containers.
- Pop enter-args (`-> ($.items) pop$`) now routes each arg through `expand_expression` with the handler's context, so Frame sigils (`$.items`, `self.field`, `@@:params.name`) resolve to their language-specific accessors.
- C++ target pinned to C++23 (`cpp_23` alias added; `cpp` / `cpp_17` / `cpp_20` still resolve to the same backend but generated coroutine code needs `-std=c++20`+).
- Cookbook recipes #53 Byte Scanner and #54 Pushdown Parser — composed scanner + parser pipeline demonstrating Frame's `@@:self` for delimiter replay and `push$` / `pop$` as a call stack.

### Fixed

- Erlang RecordUpdate codegen strips trailing `,` / `.` from the update value — `self.count = self.count + 1,` (with Erlang statement separator attached) was emitting `Data#data{count = ... ,}` with a trailing comma inside the record-update braces, a parse error.
- Docker harness: `lua_batch.sh` now prefers `lua5.4` (Ubuntu's `lua` is 5.1 and rejects `::label::`/`goto`); `lua-cjson` installs for 5.4; `TestRunner.cs` moved `Console.SetOut` before `Task.Run` to close a race that leaked phantom TAP lines.
- Kotlin test image pulls `kotlinx-coroutines-core-jvm.jar`.

### Changed

- Integration matrix: **17 / 17 clean, 3,377 passed / 0 failed / 29 skipped** — all 29 skips are legitimate language-incompat with clear inline comments. Down from 71 skips at the start of the 2026-04-20 session (42 framec-gap tests burned down). Ten languages at zero skips.
- Unit tests: 244 → 370.

## [4.0.0] - 2026-04-05

### Added

- Frame V4 transpiler with the Oceans Model — native code passes through unchanged, `@@system` blocks expand into full state machine implementations
- 9 core language backends: Python, TypeScript, JavaScript, C, C++, C#, Java, Rust, Go
- 8 experimental backends: Kotlin, Swift, PHP, Ruby, Lua, Erlang, Dart, GDScript
- GraphViz DOT output for state chart visualization
- Hierarchical state machine (HSM) support with explicit parent forwarding
- Async/await support for Python, TypeScript, and Rust
- State persistence with `@@persist` annotation
- System context (`@@`) for interface parameter access, return values, and call-scoped data
- State variables (`$.varName`) with per-state scope
- State stack operations (`push$` / `pop$`) for history transitions
- Multi-system file support
- Project-level compilation with `compile-project` command
- WASM compilation target for browser-based transpilation
- Comprehensive validation with 40+ error codes
