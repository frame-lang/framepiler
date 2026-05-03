# Per-Language Guides

Frame's "Oceans Model" — native code passes through to the target —
gives each of the 17 backends its own idiomatic surface. Some Frame
patterns work universally (state-machine semantics, transitions,
`@@:return`); others are target-specific in non-obvious ways.

These guides document each backend's idiomatic Frame patterns,
common gotchas, and the framec / harness behaviour you need to know.
They complement (rather than replace):

- `docs/frame_runtime.md` — the canonical v4 runtime spec.
- `framepiler_test_env/docs/runtime-capability-matrix.md` — the
  per-backend capability table.

> **Persist contract update (2026-05-02)**: The new four-attribute
> contract from RFC-0012 amendment (`@@[persist]` + `@@[save]` /
> `@@[load]` / `@@[no_persist]`) ships across all 17 backends.
> See [frame_runtime.md "Naming the save/load methods"](../frame_runtime.md)
> and [RFC-0012](../rfcs/rfc-0012.md) for the canonical pattern.
> The bare `@@[persist]` form remains available for backwards
> compatibility — both contracts work today.

## Status

| Backend  | Guide |
|----------|-------|
| Erlang   | [erlang.md](erlang.md) — actor / `gen_statem` / functional |
| Rust     | [rust.md](rust.md) — typed `StateContext` / ownership |
| GDScript | [gdscript.md](gdscript.md) — Godot engine / E501 reserved names |
| C        | [c.md](c.md) — pointer-based handle / typedef list workaround |
| C++      | [cpp.md](cpp.md) — `shared_ptr<T>` / `co_await` / RAII |
| Java     | [java.md](java.md) — one class per file / `CompletableFuture<T>` async |
| Go       | [go.md](go.md) — pointer receivers / capitalized exports / no async |
| Kotlin   | [kotlin.md](kotlin.md) — `suspend fun` / type inference / companion object |
| Swift    | [swift.md](swift.md) — `async throws` / optionals / `init()` |
| C#       | [csharp.md](csharp.md) — `Task<T>` / async / `using` directives |
| Dart     | [dart.md](dart.md) — `Future<T>` / null safety / `${expr}` |
| Lua      | [lua.md](lua.md) — metatable class / `:` method dispatch / 1-indexed |
| PHP      | [php.md](php.md) — `$this->`, `.` for concat, `<?php` prolog |
| Ruby     | [ruby.md](ruby.md) — `@field` instance vars, `#{...}` interpolation |
| Python   | [python.md](python.md) — class baseline, `async def` / `await`, f-strings |
| TypeScript | [typescript.md](typescript.md) — typed class, `Promise<T>` async |
| JavaScript | [javascript.md](javascript.md) — untyped class, `async`/`await` |

## Roadmap

The remaining 15 backends are queued. Priority order: backends whose
idioms diverge furthest from C-family default first.

| Backend     | Why it warrants a guide                       | Status   |
|-------------|-----------------------------------------------|----------|
| **Erlang**  | Actor / `gen_statem` / no `while`             | ✅ done  |
| **Rust**    | Typed StateContext / ownership / `&String`    | ✅ done  |
| **GDScript**| Godot integration / E501 reserved names       | ✅ done  |
| **C**       | Raw pointers / `typedef` for list types       | ✅ done  |
| **C++**     | `shared_ptr<T>` / `co_await`                  | ✅ done  |
| **Lua**     | No `class` / metatable patterns               | ✅ done  |
| **Go**      | Pointer receivers / interface satisfaction    | ✅ done  |
| **Swift**   | `async throws` / optionals / nil-safety       | ✅ done  |
| **Kotlin**  | `suspend fun` / coroutine scopes              | ✅ done  |
| **Java**    | One class per file / CompletableFuture        | ✅ done  |
| **C#**      | `Task<T>` / `partial class`                   | ✅ done  |
| **Dart**    | `Future<T>` / null safety / AOT vs JIT        | ✅ done  |
| **PHP**     | `$this->`, `.` for concat, `<?php` prolog     | ✅ done  |
| **Ruby**    | `@field` instance vars, `#{...}` interpolation | ✅ done  |
| **Python**  | `async def` / dynamic dispatch (baseline)     | ✅ done  |
| **TypeScript**| Type narrowing / `Promise<T>` (baseline)    | ✅ done  |
| **JavaScript**| Prototype chain / no types (baseline)       | ✅ done  |

All 17 backends now have per-language guides. Python, TypeScript,
and JavaScript are deliberately shorter than the more-divergent
guides — they're the baseline against which the others are
contrasted, and most cookbook recipes already exercise their
patterns.

## Authoring a new guide

Each guide should cover, where relevant:

1. **Foundation** — runtime shape (struct, class, gen_statem,
   pointers, coroutine scope).
2. **Type system contract** — how Frame's `: type` annotations
   map to the target's type system. Strongly-typed targets enforce
   at compile time; dynamic targets pass through.
3. **State machine specifics** — how transitions, HSM, state args,
   and the dispatch loop manifest in the generated code.
4. **Loop idioms** — does the target have `while`? Is recursion
   the natural iteration shape?
5. **Async** — language-native primitive, runtime expectation,
   and the position of `await` (prefix vs postfix, etc).
6. **Multi-system per file** — any structural constraint?
7. **Cross-system embedding** — how `var x: Other = @@Other()`
   lowers (direct construction, `Rc<RefCell<>>`, `Pid` wiring,
   etc).
8. **Comments and Oceans** — what the native comment leader is.
9. **Idiomatic patterns and common gotchas** — short list of
   "if you write Frame source for this target, watch for…".
10. **Cross-references** — pointers to fixtures, codegen modules,
    and capability-matrix footnotes.

Use Erlang and Rust as templates — the section structure is
deliberate; keeping them parallel makes the set easier to compare.
