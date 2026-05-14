# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed — RFC-0019 uniform `$>` / `<$` dispatch (breaking)

- **The HSM enter/exit cascade is gone.** Before RFC-0019 the kernel walked the
  state's parent chain on every `$>` (top-down) and `<$` (bottom-up), firing
  every layer's lifecycle handler. After RFC-0019, `$>` and `<$` are **ordinary
  leaf-dispatched events**: only the *current* state's `$>`/`<$` runs on
  entry/exit. An ancestor's lifecycle runs **only** if the leaf explicitly
  forwards via `=> $^` (placement in the handler body controls order). A leaf
  with no `$>`/`<$` and no forward silently *overrides* its ancestor's lifecycle.
  See [RFC-0019](docs/rfcs/rfc-0019.md).
- **Kernel surface deleted.** `__fire_enter_cascade` and `__fire_exit_cascade`
  are removed from every backend. `__process_transition_loop` now dispatches
  `<$` to the current leaf and `$>` to the new leaf — no chain walk. Erlang's
  gen_statem `enter` callback runs only the leaf's `$>` body; its
  `frame_exit_dispatch__` runs only `frame_exit__<leaf>`.
- **Construction-context push** (resolves RFC-0018 / F1). `_frame_init` /
  `__frame_init` now pushes a `FrameContext` for the start `$>` so the
  context-stack invariant (*every event handler runs in a context*) holds
  during construction. `@@:self.method()` inside a start `$>` no longer
  crashes on the post-call self-call guard.
- **`=> $^` inside `$>` / `<$` is now meaningful and supported on every
  backend.** In dynamic / typed backends it routes the lifecycle event to the
  parent's compartment dispatcher synchronously. In Erlang, `=> $^` in a `$>`
  body lowers to `frame_enter__<P>(Data)` and in a `<$` body to
  `frame_exit__<P>(Data)`, threaded through `Data`. Documented residual: a
  transition inside an ancestor's `$>` reached via `=> $^` on Erlang doesn't
  fire (`state_timeout` only works in the leaf's own `enter` clause).
- **Migration.** The cascade-asserting matrix HSM fixtures per backend
  (`40_hsm_parent_state_vars`, `42_hsm_three_levels`, `46_hsm_enter_parent_only`,
  `47_hsm_enter_both`, `48_hsm_exit_handlers`, `51_hsm_persist`) gained explicit
  `=> $^` forwards to keep their ancestor-lifecycle assertions green. User
  code with HSM cascades needs the same treatment: walk each substate, decide
  whether the parent's lifecycle should still run, and add `=> $^` if so.
  Matrix is 17/17 clean post-migration.

### Added — RFC-0016.1 `@@[no_persist]` honored end-to-end

- **`@@[no_persist]` now works on every backend.** The per-field opt-out
  attribute was parsed and validated (E801) since RFC-0012's persist-stress
  wave, but **no backend's codegen actually skipped the field** — it
  round-tripped just like every other domain field. As of this release, all
  17 backends honor it: the generated `save` body omits the tagged field; the
  `load` body leaves it at its `domain:` default (the value the constructor /
  `@@!Foo()` no-init allocation sets it to). New matrix fixture
  `100_no_persist_field.f*` covers every backend.
- **Python pickle → JSON migration** (deferred from RFC-0012). Python persist
  is now field-by-field UTF-8 JSON (the same wire shape the other dynamic
  backends already use), not whole-object pickle.
- **GDScript: native fidelity preserved (Godot binary Variant).** A brief
  JSON-for-all migration shipped on the morning of 2026-05-13 was reverted
  the same day after a user-reported fidelity bug: Godot's
  `JSON.parse_string` returns every JSON number as `float`, so a persisted
  `int`-typed domain field or list element came back as `float`, and
  `Array.has(typed_int)` after restore returned false even when the value
  was present. The fix is to keep GDScript on `var_to_bytes` /
  `bytes_to_var` — Godot's native binary Variant format, which round-trips
  every Variant type (int / float / string / array / dictionary /
  boolean / null) exactly. Wire-format **shape** still matches every
  other backend (a `PackedByteArray`); the **encoding** inside is Godot
  binary, not JSON. New matrix fixture `101_persist_int_fidelity.fgd`
  locks the regression. GDScript matrix 283/283 clean post-revert.
- **Lua: native fidelity preserved (serpent textual table-literal).** Lua
  has the same class of bug as GDScript: lua-cjson decodes every JSON
  number as `lua_Number` (Lua's float type), erasing the Lua 5.3+ integer
  subtype. Most user code is unaffected (Lua's `==` is numeric-equal
  across int and float) but `math.type()` queries and bitwise operations
  on persisted ints break. Lua persist now uses the **serpent** library
  ([github.com/pkulchenko/serpent](https://github.com/pkulchenko/serpent))
  — a single ~700-line pure-Lua file that dumps each value as a Lua
  table literal serpent.load can read back as the same type. Integers
  stay integers, floats stay floats, nested tables / strings / booleans
  / nil all round-trip exactly. As a side benefit, the previous
  type-aware `math.floor` int-coercion workaround in framec's Lua
  codegen (a type-ignorant boundary violation) was removed. Wire-format
  **shape** still matches every other backend (a Lua `string`); the
  **encoding** inside is a Lua table literal, not JSON. New fixture
  `101_persist_int_fidelity.flua` locks the regression. Lua matrix
  280/280 clean (1 pre-existing async skip).
- **Net wire-format inventory.** **14 backends share JSON**
  (Python, JS, TS, Ruby, PHP, Dart, Java, Kotlin, Swift, C#, Rust, Go,
  C, C++). Three documented native-fidelity exceptions:
  **Erlang** uses ETF (`term_to_binary`), **GDScript** uses Godot
  binary Variant (`var_to_bytes`), **Lua** uses serpent textual
  table-literal. All three exceptions are driven by the same pattern:
  the language has real types JSON can't represent (atoms, Variant
  int/float distinction, Lua int subtype), and forcing a lossy
  conversion silently breaks idiomatic code. A future opt-in
  `@@[persist_format(...)]` attribute (RFC pending) will give the
  14 JSON backends a typed-binary path (MessagePack / CBOR) for
  cross-language use cases that need int/float fidelity outside Frame.
- **Erlang: native fidelity preserved (Erlang External Term Format).** After
  weighing tagged-JSON marshalling against the cost of forcing Erlang
  programmers to deal with lossy round-trip for atoms, tagged tuples, and
  char-list strings, Erlang's persist wire format is `term_to_binary` /
  `binary_to_term({safe})` — the OTP-standard, zero-dep, fully-lossless
  serialization the rest of the Erlang ecosystem (mnesia, dets, ets,
  distributed Erlang) uses for the same job. Wire-format **shape** still
  matches the other 16 backends (a `binary()`); the **encoding** inside that
  binary is ETF, not JSON. Cross-language consumers who need to inspect the
  payload can use an ETF parser (one exists in every major language). The
  `@@[no_persist]` skip contract is preserved by omitting the field from
  the saved `Persisted` map, so the freshly-constructed `#data{}` on load
  picks up the record's compile-time default. Erlang matrix 275/275 clean
  (9 pre-existing framec-gap skips).
- **Wire-format breaks.**
  - **Python**: pickle blobs written by prior framec releases will not load
    (now JSON).
  - **GDScript**: the wire format is **back on `var_to_bytes`** (the
    pre-yesterday format). Pre-4.2 blobs continue to load. If anyone
    pulled framec between the morning-of-2026-05-13 JSON migration and
    the same-day revert, their JSON blobs from that window will not
    load — but the window was a few hours.
  - **Lua**: cjson JSON → serpent textual table-literal. Pre-4.2 cjson
    blobs will not load.
  - **Erlang**: persist now returns `binary()` instead of `map()`. The 3
    existing test drivers that introspected `Saved` directly
    (`23_persist_basic`, `24_persist_roundtrip`, `25_persist_stack`) were
    updated to call `binary_to_term/2` first. User code that calls
    `save_state` / `load_state` round-trip-only is unaffected.
  Persist work is still `[Unreleased]` — no released-format-compat promise
  yet.
- New spec: [RFC-0016.1](docs/rfcs/rfc-0016-1.md). Complementary inclusion-list
  form `@@[persist_fields([...])]` (RFC-0016) remains deferred.

### Fixed — Erlang `#` comments inside handler bodies

- A `#` (Frame comment) inside an Erlang handler body — `$>` / `<$` / a regular
  event handler — used to leak verbatim into the generated `.erl` as `# ...,`.
  Erlang uses `#` for record / map syntax, not comments, so this was a parse
  error (no fixture exercised it before today). The body processor now
  translates Frame `#` comments to Erlang `%` in its pre-pass, distinguishing
  comment-`#` from `Var#rec{...}` / `Var#rec.field` / `#{...}` / `Map#{...}`
  by neighbour chars. `erlang_smart_join` already drops `%` comment-only lines,
  so handler-body comments don't appear in the output — they just no longer
  break it.

### Changed — RFC-0017 init decoupling (breaking)

- Every system class is now emitted as three artifacts instead of one: a **bare constructor** (framework setup only — state stack, compartment placeholder, domain defaults, no user `$>`), a **`__frame_init(args)` method** (runs the user `$>` body, fires the enter cascade, drains the transition loop), and a **`__create(args)` factory** (bare ctor + `__frame_init` + return). Per-backend spellings: typed backends use `__create` / `__frame_init`; Python/Dart/JS/TS/Ruby/Lua/PHP/GDScript use `_create` / `_frame_init`; Go uses `CreateCounter` / `NewCounter`; C uses `Counter_create` / `Counter_new` / `Counter_frame_init`; Erlang uses `create/N` / `start_link/0` / `frame_init/(N+1)`.
- `@@Counter(7)` now lowers to `Counter.__create(7)` (or per-backend equivalent — see the [RFC-0017 mapping table](docs/rfcs/rfc-0017.md#generated-calls-per-backend)). `@@!Counter()` lowers to the bare constructor in every backend (Erlang: `element(2, counter:start_link())`). D4 invariant from RFC-0015 preserved: `$>` runs exactly once on the factory path, never on `@@!`. Verified end-to-end across all 17 backends — the differential matrix is 17/17 (~4,800 fixture×backend executions, 0 failed).
- **`const` domain fields seeded from a system param** (`const x: int = x`): the assignment moves to the constructor body / `__frame_init` (where the param is in scope). C++ keeps `const T` and seeds it via the member-initializer list — so on C++ the bare ctor takes the system params, threaded through `__create`. On Kotlin/Swift, where a `val`/`let` can't be assigned outside the constructor, the field is emitted as mutable at the target-language level (the Frame-level `const` is still enforced by the validator, E814+); Swift additionally seeds it with the type's zero value so the designated `init()` satisfies definite-initialization.
- C++ `__create` returns `T` **by value** (so driver call sites stay value-semantics: `auto c = @@Counter(7); c.method()`). System-typed *domain fields* are `shared_ptr<T>`; their initializer wraps the factory result — `std::make_shared<Counter>(Counter::__create(7))` — move-constructed from the returned temporary.
- **Removed:** `__skipInitialEnter` static flag (Java/Kotlin/Swift/Dart/GDScript/C++), `kotlin_type_default_expr` / `swift_type_default_expr` helper functions, all `__no_init` / `_no_init` / `Foo_alloc` / `'__no_init'/0` D7 synthesized helpers (8 backends), and 5 legacy `__skipInitialEnter` branches in `interface_gen.rs` restore-state codegen (dead since E814 hard-cut). (Swift's `emit_field` does have a small inline type-default `match` for stripped-initializer fields — that's the const-from-param handling above, a different mechanism than the removed helper.)
- **Erlang specifics:** `callback_mode/0` now returns `[state_functions, state_enter]`; `init([])` always sets `frame_skip_enter__ = true`; the `frame_init` `gen_statem:call` handler clears the flag and uses `{repeat_state, Data1, [{reply, From, ok}]}` (not `next_state`) so `state_enter` re-fires on the same state. The differential test harness now constructs systems via `Mod:create/N` (the factory), not the no-init `start_link/N`.
- **Migration.** Frame source is unchanged. Host code that called the generated constructor *with arguments* directly (`new Counter(7)`, `Counter(7)`, `Counter::new(7)`, `counter:start_link(7)`) must switch to the explicit factory (`Counter.__create(7)`, `Counter._create(7)`, `Counter::__create(7)`, `counter:create(7)`). Bare zero-arg constructor calls still compile but now produce a no-init instance — equivalent to `@@!Counter()`. `scripts/migrate_rfc0017_fixtures.py` does the mechanical rewrite for driver code (it ported the ~1,830-file test corpus).
- See [RFC-0017](docs/rfcs/rfc-0017.md) for the full mapping table, rationale, and rejected alternatives.

### Changed — type-ignorant codegen

- framec emits a domain field's *declared* type, spelled the target way, and lets the target's own tooling do the (de)serialization — no per-type `match` in the codegen. The **C `@@[persist]` domain-var path** moved to the symbol-mangled dispatcher: framec emits `<sys>_persist_pack_field_<mangled>((void*)&self->x)` / `<sys>_persist_unpack_field_<mangled>(json, (void*)&self->x)`; the runtime owns the cJSON typing (matching the state/enter-arg path). The now-dead `is_int_type` / `is_float_type` / `is_bool_type` / `is_string_type` predicates were removed.
- `@@:return` typed-read (`context_return_read_typed`) is type-ignorant on 16 backends — C++/Go/Swift/Kotlin/C# downcast to the spelled declared type uniformly for any `T`; Java keeps a primitive-vs-reference branch the JVM forces; C keeps a `void*`-ABI category branch (`double` bit-pun / string pointer / integer width). Rust handles `int`/`float`/`bool`/`str`; a user-declared-struct return still falls back to the raw `Option<&Box<dyn Any>>`.
- New: [`docs/contributing/type-ignorant-codegen.md`](docs/contributing/type-ignorant-codegen.md) — the architectural boundary (the three legitimate per-type branchings: type spelling, definite-init defaults, type-erased-storage downcasts; everything else is forbidden), linked from `adding-a-backend.md` and the architecture guide.

### Changed — documentation

- New [`docs/glossary.md`](docs/glossary.md) (every non-standard Frame term and symbol, deep-linkable, cross-referenced to the language/runtime docs and the defining RFC) and [`docs/rfcs/STYLE.md`](docs/rfcs/STYLE.md) (RFC style guide, grounded in the Rust RFC process / IETF RFC 2119+7322 / PEP 1+12). `rfc-0015.md` (Factory-Only System Construction) and `rfc-0016.md` (Selective Domain Persist — draft, deferred) rewritten against them: internal phase/wave/decision-code noise stripped, `Blank` → "no-initialization" throughout, validator tables verified against the implementation. Terminology aligned in `rfc-0017.md` and `frame_language.md`.

## [4.1.1] - 2026-05-09

### Changed

- **Repository moved to `frame-lang/framec`** (new GitHub org + rename). The previous canonical location, `frame-lang-old/framepiler`, was renamed to `frame-lang-old/framec` then transferred to `frame-lang/framec` in the new org. GitHub serves auto-redirects from both prior URLs.
- Cargo.toml `repository`, `homepage`, and `documentation` URLs updated to the new location. (Crate metadata for `4.1.0` retains the old `frame-lang/framepiler` URLs — fixed forward in this patch release.)
- README CI badge URL updated to the new repository.
- `.github/CODEOWNERS` owner handle migrated to `@cogiton`.
- Doc + contribution-guide URLs swept to the new repository (CONTRIBUTING, SECURITY, getting-started, adding-a-backend).

No code changes. Pure metadata + URL hygiene release.

## [4.1.0] - 2026-05-08

Headline of 4.1.0: **RFC-0015 — factory-only construction with system-level lifecycle attributes**, the new persist contract that supersedes RFC-0012's operation-attribute form. Hard-cut at this release; legacy form rejected by **E819**. Backed by three-attribute lifecycle (`@@[create]` / `@@[save]` / `@@[load]`), the `scripts/migrate_rfc0015.py` codemod (multi-system + visibility-aware), and end-to-end coverage on all 17 backends.

This release also closes the last gaps from the RFC-0014 `@@[main]` wave 1, the RFC-0013 annotation syntax, and the persist wave 8 closure (nested `@@SystemName` on every backend; E700 quiescent contract).

### Added — RFC-0015 factory-only construction (system-level lifecycle attributes)

- **Three-attribute lifecycle contract** at the system level: `@@[create(<name>)]`, `@@[save(<name>)]`, `@@[load(<name>)]`. Names default to `create_<system>` / `save_<system>` / `load_<system>` when unspecified. Generated factory and persist methods adopt the user-named identifiers across all 17 backends.
- **E815** — lifecycle attribute attached to a non-system attachment position (must be system-level, not operation-level).
- **E817** — invalid lifecycle attribute name (non-identifier).
- **E818** — duplicate `@@[save]` or `@@[load]` on the same system (only one of each per system).
- **E819 — hard-cut.** RFC-0012's op-attribute persist form (`save: () { @@[save] ... }`) is rejected at validation time with a one-line migration message pointing at the RFC-0015 system-level form.
- **`scripts/migrate_rfc0015.py` codemod** — multi-system aware (handles `@@system Inner(seed: int) { … }` headers with parameter lists and superclasses) and visibility-modifier aware (`@@system private`, `public`, `internal`). Migrates RFC-0012 op-attribute fixtures to the new contract; full corpus pass touched 4,734 fixtures.
- **D3 — C cross-system method call rewrite.** Post-pass in the C backend rewrites `self.field.method(args)` to `<Sys>_method(self->field[, args])` for domain fields whose `type_annotation` matches a defined system. Closes the long-standing C cross-system call gap (analogous to the existing Erlang post-pass).
- **D4 — leading-`_` C action symbols** preserved verbatim in `func_name` and given a matching `static` keyword in the forward declaration.
- **D5 — Erlang `@@:(<expr>)` in action bodies** now lowered via `expand_system_state_in_code`. Multi-line case/if-block trailing expressions bind to `__ActionRetVal__` and return `{Data, __ActionRetVal__}`. Leading-`_` user names quoted as `'_name'` atoms across action and operation call sites.
- **Rust default load param type** for the new persist contract is `String` (not `&str`), so `save_<sys>() -> String` and `load_<sys>(s: String)` round-trip cleanly through the common case.
- **Phase 5 fuzz coverage** — `gen_persist_multisys.py` (P1 simple_nested, P2 parameterized_inner — Issue #2 reproducer at scale, P3 chained 3-level) across 16 backends; `gen_async_persist.py` Python canary; negative-case fixtures for E815/E817/E818/E819.
- **Cookbook + per-language guides + spec** all migrated to the system-level form. **Recipe 111 added**: "Init Logic in `$>` — Where Setup Code Lives" (clarifies the canonical home for one-time setup vs. recurring transitions).

### Added — RFC-0016 selective domain persist (deferred design)

- `@@[persist_fields([...])]` form documented for use cases that need a subset of domain fields persisted. Explicitly deferred from 4.0/4.1; tracked as RFC-0016 for a future release.

### Added — W705 transition return-type default warning

- Validator warns on `-> $State` transitions in event handlers whose declared return type's default value might silently leak. Strict `return -> $State` form remains the supported pattern; the warning helps catch the loose form during migration.

### Fixed — multi-line `@@:()` return expressions on indent-sensitive targets

- Multi-line expressions inside `@@:(<expr>)`, `@@:return = <expr>`, and `@@:return(<expr>)` are now re-wrapped in `(...)` when the expanded RHS contains a newline. Without the wrap, GDScript and Python parsed the assignment up to the first newline and rejected the continuation as an `Indent` parse error. Curly-brace targets receive redundant-but-harmless parens. Matrix fixture `92_return_expr_multiline.{fpy,fgd}` covers the regression. Surfaced by `frame-arcade/ch05-pacman`.

### Fixed — additional codegen + runtime fixes

- **Erlang multi-line `@@:(value)`** no longer joins lines with stray commas in the emitted record-update.
- **Rust `rewrite_arg_if_non_copy_field`** byte-slice OOB panic — defensive bounds check on the arg-rewrite walk.
- **Two FRAMEC_BUGS hot-fixes** (Issues #1 and #2 from `frame-arcade/FRAMEC_BUGS.md`) closed end-to-end at the codegen layer, with verification trace in the bug report.

### Added — RFC-0014 `@@[main]` (wave 1)

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
- Repo housekeeping: `CLAUDE.md` removed from version control (project-internal AI agent context, kept local-only via `.gitignore`).
- Author / project email migrated to `mark@frame-lang.org`.

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
