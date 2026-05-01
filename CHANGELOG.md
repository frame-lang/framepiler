# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased] - 2026-05-01

### Added — persist contract (wave 8 closure)

- **E700 quiescent contract for `save_state`.** Mid-event saves now error with `E700: system not quiescent` instead of producing partial / undefined snapshots. Per-backend mechanism: throw on JVM/.NET/dynamic langs/C++, panic on Rust/Go, abort on C/Swift, push_error on GDScript, implicit (gen_statem deadlock) on Erlang. Hard cut, no soft warning. Documented in `frame_runtime.md`, `rfcs/rfc-0012.md`, and all 17 per-language guides.
- **Nested `@@SystemName` persist parity across all 17 backends.** Wave 8 closure: nested-system save/restore now works on every backend including the previously-blocked C and Erlang. C uses cJSON recursive embedding; Erlang uses gen_statem process trees with `child:save_state` recursing through Pids. JVM (Java + Kotlin) gained Option A nested-system support to match the existing 12-backend rollout.
- **Erlang multi-statement handler + cross-system call** — pre-existing limitation closed: handlers that combine self-mutation with cross-system calls (e.g., `self.n = self.n + 1; self.child.bump()`) now compile correctly. Cross-system rewrite extended to match `Data1#data.field`, `Data2#data.field`, etc. (the per-statement record-update suffix that emerges in chained handlers).
- **Lua `int` domain field type coercion on persist restore** — cjson decodes JSON numbers as Lua floats by default; declared `int` fields now coerce via `math.floor()` on restore so they round-trip with the integer subtype intact (Lua 5.3+).
- **70+ new persist tests** (tests 84–88 across 14 wired backends + multi-system Erlang variants in `tests/erlang/multi/`) covering: nested HSM × persist, 3-level nested HSM × persist, numeric typing in nested persist, multi-instance independence, E700 quiescent error path, plus the existing 5-deep nested chain extended to C and Erlang.
- **RFC-0012 expanded** with three new sections marked deferred pending customer feedback: cycles in the persist graph (Option A E702 recommended), Python pickle → JSON migration, adversarial input threat model + E701 corrupted-snapshot proposal.
- **Python pickle security warning** — `frame_runtime.md` and the Python per-language guide now warn that `pickle.loads` on attacker-controlled input is RCE. JSON migration tracked in RFC-0012, deferred pending customer feedback.

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
