# Type-Ignorant Codegen

This is the rule framec's code generator follows about Frame's primitive
types, and the (small, bounded) set of exceptions to it. Read it before
adding a `match` on a type name.

## The rule

> **framec emits the user's *declared* type, spelled the target way, and
> lets the target language's own tooling do the type work.**

A Frame source declares types on domain fields, interface parameters,
state/enter parameters, and state vars (`balance: int`, `name: str`,
`items: list`, `Robot($(x: int))`, …). framec's job is to translate Frame
*structure* — states, events, transitions, the dispatch kernel — into 17
target languages. It is **not** framec's job to know that an `int`
serializes to JSON differently from a `str`, or that `==` works on Java
`int` but not Java `String`. The target language already knows that;
framec emits the declared type verbatim (after a one-step *spelling*
translation — see below) and hands the type work to:

- **serialization / deserialization** → the target's serializer:
  `serde_json` (Rust), `Codable` (Swift), `nlohmann::json` (C++),
  Jackson `TypeReference<…>` (Java/Kotlin), `JsonSerializer.Deserialize<…>`
  (C#), `encoding/json` (Go), `JSON` (GDScript), `json`/`pickle`-style
  native paths (Python/JS/TS/Ruby/Lua/PHP), the symbol-mangled `_pack_/_unpack_`
  dispatcher (C). framec never branches on element types here — see
  [`type_ignorant_persist`](../rfcs/rfc-0012.md).
- **equality / formatting / arithmetic** → the target's own operators.
- **defaults** → see exception (2) below.

The poster child is `rust_system.rs::rust_json_extract`: it takes a
`_var_type` argument and **doesn't use it** — `serde_json::from_value`
infers the target from the struct field's declared type. That's the goal
state for every (de)serialization path, and it has been reached in every
backend (the C *domain-var* path is the one residual exception — see the
follow-ups).

## What per-type branching *is* allowed

Three categories, and only three. If your new branch isn't one of these,
you're doing the target language's job for it — stop and emit the type
verbatim instead.

### 1. Type *spelling* — `map_type` and friends

Translating a Frame type *name* to the target's spelling is the
transpiler's core job: `str` → `String` (Java) / `string` (Go) /
`std::string` (C++) / `String` (Swift), `int` → `i64` (Rust) / `Integer`
(when a generic argument needs boxing, Java) / `int` (most), `list` →
`Vec<…>` / `Array` / `[]interface{}` / a `<sys>_FrameVec*` (C), and so on.
The canonical functions:

| function | location |
|---|---|
| `*Backend::map_type` / `convert_type` / `convert_type_to_c` | each `backends/<lang>.rs` |
| `csharp_map_type` / `java_map_type` / `kotlin_map_type` / `swift_map_type` / `go_map_type` / `cpp_map_type` / `type_to_cpp_string` | `codegen_utils.rs` |
| `frame_type_to_rust_type` (local copy of `RustBackend::convert_type`) | `runtime.rs` |
| `type_to_string` (`Type::Custom(s) => s`, `Type::Unknown => "Any"`) | `codegen_utils.rs` |
| `c_mangle_type` (alias-normalise → C-identifier suffix for the `_pack_` dispatcher) | `interface_gen.rs` |

Every one of these has a **verbatim pass-through arm** (`other =>
other.to_string()`). That arm *is* the type-ignorant principle in code:
the table only normalises *known aliases* (`i32`→`int`, `boolean`→`bool`,
`float`→`double`, `Array<any>`→`list`, …); a type the table doesn't
recognise is the user's, and it flows through untouched. **Never add a
non-pass-through arm to one of these for a type you don't recognise.**

A type-spelling helper is the *only* place a new "what target type does
Frame type `X` become" decision belongs. Don't hand-roll a four-entry
`match { "int"|"number" => "int", "double"|"float" => "double", … }` at
the call site — call the backend's helper. (Today the Dart state-var-read
and handler-param-unpack paths in `frame_expansion.rs` / `state_dispatch.rs`
do hand-roll one; that's a code-quality nit to consolidate, not new
licence to copy.)

### 2. Definite-init defaults — a *value*, not *logic*

Some targets refuse a stored property / struct field that isn't
initialised by the end of the constructor (Swift's designated `init()`,
Kotlin properties, Rust `Self { … }`, C++ members, Go zero-values are
implicit but framec still emits them). And a state var declared `$.n: int`
with **no** initializer still needs *something* emitted at the synthetic
`$>` arm. So framec computes the type's *zero value*:

| function | location | what |
|---|---|---|
| `state_var_init_value` | `codegen_utils.rs` | `int→0`, `bool→false`/`False`, `str→""`/`String::new()`/`std::string()`, `list→[]`/`Vec::new()`/…, `dict→{}`/`HashMap::new()`/…, unknown→`null`/`None`/`nil` |
| `frame_type_to_rust_default` | `runtime.rs` | the `XContext` struct's field placeholders |
| `frame_return_default` | `interface_gen.rs` | a `: int`-returning interface method whose body was suppressed by a transition guard must still hand a non-`null` value back to the wrapper |
| `kotlin_default_for_type` / Swift `emit_field`'s stripped-initializer arm | `backends/{kotlin,swift}.rs` | a domain field whose initializer was moved into `__frame_init` (because it references a system param — RFC-0017) needs a placeholder in the declaration |

These are *unavoidable*: the default genuinely **depends on the type**, and
there's no native tooling that supplies it (the target doesn't know "the
zero value of Frame's `int`"). A few entries are specifically about
*spelling* the zero value (`String::new()` not `""` for a Rust `String`
field, `std::string()` not `""` for a C++ `std::any` slot) — same
category. If a future Frame revision required every state var / domain
field to carry an explicit initializer, this category would shrink; today
it's load-bearing.

### 3. Downcasts out of type-erased runtime collections (and the C `void*` ABI, and `Box<dyn Any>` exactness)

The dispatch kernel stores **per-call / per-state** data in collections
that are shared by every state in the system, so they can't be
statically typed:

- `state_args` / `enter_args` / `exit_args` — `List<Object>` (Java),
  `[Any]` (Swift), `[]interface{}` (Go), `Vec<Box<dyn Any>>` (Rust),
  `<sys>_FrameVec*` of `void*` (C), `List<dynamic>` (Dart), …
- `__e._parameters` (the event payload) — same shape.
- the per-call context `_return` slot — `Object` / `Any` / `std::any` /
  `void*` / `Box<dyn Any>` / `interface{}`.
- `state_vars` — `Map<String, Object>` / `FrameDict` / …

To bind a `: int` handler parameter, read `$.sv` typed, or pull a typed
`@@:return` back out, framec **must** downcast from the erased element
type to the declared type. And the downcast *syntax* sometimes differs by
type *category* (not by individual type):

- **JVM / C#**: a value that came back from a `@@persist` JSON round-trip
  is a boxed `Double` / `Long` / `BigDecimal`, which **cannot be cast
  directly** to a primitive `int` / `float` — hence `((Number)x).intValue()`
  / `System.Convert.ToInt32(x)` for primitive receivers, `(T)x` for
  reference receivers. That's a *primitive-vs-reference* branch, the
  minimum the JVM forces.
- **C**: primitives ride in a `void*` via `(intptr_t)`; a `double`
  doesn't fit and needs the memcpy bit-pun (`<sys>_pack_double` /
  `<sys>_unpack_double`); a `char*` / `<sys>_FrameVec*` / `<sys>_FrameDict*`
  is already a pointer. Three forms, dictated by the C ABI.
- **Rust**: a `Box<dyn Any>` downcast *must name the exact type stored*
  (`downcast_ref::<i64>()`, not `<i32>`), and Rust integer literals
  default to `i32` while interface signatures are `i64` — so
  `rust_wrap_for_boxing` casts `(expr) as i64` / wraps `"…"` as
  `String::from(…)` on the way *in* to the box. Exactness, not logic.

The `*_map_type(declared)` portion of all of this is category 1. The
*downcast itself* is category 3 — structurally forced by the runtime data
model. (If `state_args` were ever made a typed-per-state structure — as it
already is for Rust, via the `__sys_<name>: <type>` fields and the
`StateContext` enum — this category would shrink for that backend. It's an
architectural question about the runtime, not a codegen-cleanliness one.)

## What's *not* allowed

Anything that does the target language's type work *for* it where the
language could do it itself:

- Per-type **serialization** / **deserialization** logic that bypasses the
  native serializer — `if int { cJSON_AddNumberToObject } else if str {
  cJSON_AddStringToObject } …` instead of `serializer.encode(value)` /
  the `_pack_<mangled>` dispatcher. (Persist already follows this rule;
  the C *domain-var* sub-path is the one place that still has the old
  ladder — see follow-ups.)
- Per-type **formatting** — `if str { x } else { x.toString() }` instead
  of the target's own `"" + x` / `str(x)` / `format!("{x}")`.
- Per-type **equality** — `if str { a.equals(b) } else { a == b }`
  instead of letting the target's `==` (which already DTRT).
- A downcast wired for `int` / `bool` / `str` *only* with a wrong-but-
  compiling fallback for everything else (e.g. `context_return_read_typed`
  falling back to `std::string` for a `: float` return). Use
  `std::any_cast<cpp_map_type(T)>` / `x.(go_map_type(T))` and let it work
  for every spelled `T`.

When you're about to add a `match` on a Frame type name, ask:

1. **Is it spelling?** → it belongs in the backend's `map_type`, not at
   the call site, and it has a verbatim pass-through arm.
2. **Is it a definite-init default value?** → category 2; keep it minimal
   and data-driven; the `_ =>` arm is `null`/`None`/the language's zero.
3. **Is it a downcast out of a type-erased kernel collection / the C
   `void*` ABI / `Box<dyn Any>` exactness?** → category 3; the per-type
   *category* is forced; route the *spelling* through `map_type`.
4. **None of the above?** → you're doing the target's job. Emit the type
   verbatim (`type_to_string` / `map_type`) and call the native tooling.

## Where each backend stands

The audit behind this doc (2026‑05) found the codegen is **substantially
type-ignorant already** — the historical accumulation point, the persist
(de)serialization paths, was migrated backend-by-backend:

| backend | persist mechanism | type-ignorant? |
|---|---|---|
| Rust | `serde_json::from_value` into the declared field type | ✅ (`_var_type` literally unused) |
| C++ | `nlohmann::json::get<T>()` / `std::any_cast<T>`, `T` = `cpp_map_type(declared)` | ✅ |
| Java / Kotlin | Jackson `new TypeReference<USER_TYPE>(){}` round-trip | ✅ (Java *spells* `int` as `Integer` for the generic arg — that's category 1) |
| C# | `JsonSerializer.Deserialize<USER_TYPE>(…)` | ✅ |
| Go | `json.Marshal`/`Unmarshal` into `var __t USER_TYPE` | ✅ |
| Swift | dict pass-through (`as? [Any]`) | ✅ |
| Dart | comprehension codegen over `parse_dart_type` (a structural `List<…>`/`Map<…>` tree, not a type enumeration) | ✅ (leaf casts `int`/`double`/`String`/`bool` are category 3) |
| C | symbol-mangled dispatcher: framec emits `<sys>_persist_pack_<mangled>(value)`; the runtime owns the type knowledge | ✅ for state/enter args; **the *domain-var* sub-path still uses the `is_*_type` → `cJSON_Add{Number,Bool,String}ToObject` ladder** — see follow-ups |
| Python / JS / TS / Ruby / Lua / PHP / GDScript | native dynamic round-trip | ✅ (Lua's domain-var restore re-integerises `: int` via `math.floor` — see follow-ups) |

Everything else that branches on a type name is category 1, 2, or 3.

## Known soft-spots / follow-ups

Not blockers; documented so a future pass can mop them up.

1. **C persist *domain-var* (de)serialization** (`interface_gen.rs`, the
   `is_int_type`/`is_float_type`/`is_bool_type`/`is_string_type` ladder
   around `cJSON_Add{Number,Bool,String}ToObject` on save and
   `->valuedouble` / `cJSON_IsTrue` / `strdup` on restore). The C
   *state/enter-arg* path 200 lines above already went type-ignorant via
   `c_mangle_type` + `<sys>_persist_pack_<mangled>(void* v)`. The
   domain-var path is harder because a domain field is *statically typed*
   (`int n;`), not a `void*`, so the existing `void*`-passing dispatcher
   doesn't fit directly. The clean fix is a *pointer-passing* variant:
   `<sys>_persist_pack_field_<mangled>(&self->n)` /
   `<sys>_persist_unpack_field_<mangled>(json, &self->n)`, with the runtime
   defining `_persist_pack_field_int(int* p)` etc. (a fixed set of blessed
   types, same extension model). Until then, this is the one place framec
   still calls `is_*_type` outside the `typed_init_expr` parser-fallback.

2. **`context_return_read_typed` / `rust_context_return_read_typed`**
   (`frame_expansion.rs` / `rust_system.rs`) — the typed `@@:return` read
   hard-codes `int` / `bool` / `str` and falls back to a raw access (C++
   even falls back to `std::string`, latently wrong for a `: float`
   return). For C++ and Go this can be made fully type-ignorant
   (`std::any_cast<cpp_map_type(T)>` / `x.(go_map_type(T))`); the JVM arms
   should reuse the `Number`-ladder from `state_dispatch.rs`. Only bites a
   typed-return handler that reads its *own* `@@:return` for an unusual
   type — no test in the corpus hits it.

3. **Rust `.clone()` elision** — `rust_system.rs::rust_expand_state_var_read`'s
   `is_copy` check and `rust_type_is_copy` decide whether to *skip* a
   `.clone()` for `Copy` types. Cloning unconditionally is valid Rust;
   these are a pure idiom/perf nicety (and dropping them would trade
   `i64.clone()` for `clippy::clone_on_copy` lints in generated code, so
   it's a wash). Listed for completeness; not a violation.

4. **Hand-rolled Dart type maps** — `frame_expansion.rs` (state-var read)
   and `state_dispatch.rs` (handler-param unpack) inline a four-entry
   `match { "int"|"number" => "int", "double"|"float" => "double", … }`
   instead of calling a shared Dart spelling helper. Consolidate into one
   `dart_map_type` and reuse it. Category-1 nit, not new licence.

## See also

- [`docs/rfcs/rfc-0012.md`](../rfcs/rfc-0012.md) — the `@@[persist(<type>)]`
  contract and the type-ignorant persist migration.
- [`docs/contributing/adding-a-backend.md`](adding-a-backend.md) — Step 4
  (Backend Codegen) and Step 7 (system-init parameters).
- [`docs/contributing/framepiler_architecture_guide.md`](framepiler_architecture_guide.md) —
  the dispatch model and where state/enter args live.
