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

## Status

| Backend | Guide |
|---------|-------|
| Erlang  | [erlang.md](erlang.md) — actor / `gen_statem` / functional |
| Rust    | [rust.md](rust.md) — typed `StateContext` / ownership |

## Roadmap

The remaining 15 backends are queued. Priority order: backends whose
idioms diverge furthest from C-family default first.

| Backend     | Why it warrants a guide                       | Status   |
|-------------|-----------------------------------------------|----------|
| **Erlang**  | Actor / `gen_statem` / no `while`             | ✅ done  |
| **Rust**    | Typed StateContext / ownership / `&String`    | ✅ done  |
| GDScript    | Godot integration / engine lifecycle hooks    | pending  |
| C           | Raw pointers / `typedef` for list types       | pending  |
| C++         | `shared_ptr<T>` / `co_await`                  | pending  |
| Lua         | No `class` / metatable patterns               | pending  |
| Go          | Pointer receivers / interface satisfaction    | pending  |
| Swift       | `async` / Result / nil-safety                 | pending  |
| Kotlin      | `suspend fun` / coroutine scopes              | pending  |
| Java        | One class per file / CompletableFuture        | pending  |
| C#          | `Task<T>` / `partial class`                   | pending  |
| Dart        | Future / kernel snapshot vs AOT               | pending  |
| PHP         | `$this` reference / closure use clauses       | pending  |
| Ruby        | `self.` / mixins / no static typing           | pending  |
| Python      | `async def` / dynamic dispatch (baseline)     | low pri  |
| TypeScript  | Type narrowing / generics (baseline)          | low pri  |
| JavaScript  | Prototype chain / no types (baseline)         | low pri  |

Python, TypeScript, and JavaScript are the baseline against which
the other guides are usually contrasted — explicit guides for them
are lower priority because most cookbook recipes already exercise
their patterns.

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
