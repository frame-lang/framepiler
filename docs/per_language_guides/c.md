# Per-Language Guide: C

C is the most low-level Frame target. Where every other backend has
some form of object/struct with method dispatch, C has plain structs
and free-standing functions: `Counter_bump(Counter* self, int n)`
instead of `counter.bump(n)`. The Frame source you write reads
similarly to C-family targets, but the generated output is materially
different — and a few language idioms (no `class`, no built-in
strings, no async, no list type) require deliberate workarounds.

This guide documents the C-specific patterns. It assumes you are
already familiar with Frame's core syntax and basic C (pointers,
`typedef`, `#include`, `printf`/`sprintf`).

For the canonical capability table, see
`framepiler_test_env/docs/runtime-capability-matrix.md`. C is fully
spec-conformant on the runtime; the only language-natural skips are
`async` (no language-level async/await) and a handful of footnoted
type-system workarounds.

---

## Foundation: pointer-based system handle, free-standing functions

A Frame system targeting C generates one `.c` (and one `.h`) module
containing:

- A `struct Counter` with the domain fields and a state-tag enum.
- A `Counter* Counter_new(...)` constructor that returns a heap-
  allocated pointer.
- One `Counter_<event>(Counter* self, ...)` free-standing function
  per interface method.
- Internal `_state_<S>(Counter* self, FrameEvent* e, ...)` and
  `_s_<S>_hdl_<kind>_<event>(...)` helpers for dispatch.

```c
typedef struct Counter Counter;

Counter* Counter_new(int seed);
void Counter_destroy(Counter* self);

int Counter_get(Counter* self);
void Counter_bump(Counter* self, int by);
```

Frame's `self.field` lowers to `self->field` (pointer dereference),
and method calls are `Counter_method(self, args)` — no implicit
`this`. The generated header makes the system handle opaque if the
struct definition stays in the `.c` file, or transparent if you
include the struct in the header for inlining.

---

## Domain fields: typedef workaround for array / list types

C's declarator syntax is interleaved (the `[8]` follows the variable
name in `char* pending[8]`), which does not fit Frame's
`name : type = init` declaration shape. Frame's `: list` type
doesn't have a native C primitive — there is no `List<T>` or `[]int`.

The Oceans Model workaround is to declare a `typedef` in the prolog
that flattens the array shape to a single-token type:

```frame
@@target c

#include <stdio.h>
#include <string.h>

// Flatten the C array declarator into a single typedef so the Frame
// domain syntax can use it as a normal type name.
typedef char* JobSlots[8];

@@system WorkerPool {
    domain:
        pending: JobSlots = {0}
        pending_count: int = 0

    machine:
        $Idle {
            submit(task: char*) {
                self->pending[self->pending_count] = task;
                self->pending_count += 1;
            }
        }
}
```

The generated C uses standard array indexing on the field:

```c
struct WorkerPool {
    JobSlots pending;        // expands to char* pending[8]
    int pending_count;
    // ...
};

void WorkerPool_submit(WorkerPool* self, char* task) {
    self->pending[self->pending_count] = task;
    self->pending_count += 1;
}
```

This pattern lets you express any array, struct, or function-pointer
shape as a Frame domain field: `typedef` first, declare the field
with the typedef'd name. See `tests/common/positive/demos/21_worker_pool.fc`
for the canonical example.

**Frame `: list` is not auto-converted on C.** There is no Frame
runtime-provided list helper for C — the user is responsible for the
underlying type. Capability-matrix footnote `[l]` documents this.

### Persist + custom typed lists (D12 extension hook)

The `@@persist` C runtime emits a symbol-mangle dispatcher:
`<sys>_persist_pack_<mangled>(value)` and a matching `unpack_`
twin. The runtime supplies built-in pack/unpack functions for
`int`, `double`, `str`, `bool`, `list`, `dict`. The default
`pack_list` packs each element as `int`; if you have a list of
`str` or `float` you'll lose precision/data on round-trip.

Two approaches to typed lists:

**Approach A — declare a custom type, supply your own pack/unpack:**

```c
// Frame:
//   domain:
//       names : <sys>_StrList* = NULL
//
// Then in your C prolog:

typedef <sys>_FrameVec MySys_StrList;  // alias for clarity

static cJSON* MySys_persist_pack_StrList(void* v) {
    MySys_StrList* vec = (MySys_StrList*)v;
    cJSON* arr = cJSON_CreateArray();
    if (vec) {
        for (int i = 0; i < vec->size; i++) {
            cJSON_AddItemToArray(arr,
                cJSON_CreateString((const char*)vec->items[i]));
        }
    }
    return arr;
}

static void* MySys_persist_unpack_StrList(cJSON* j) {
    if (!cJSON_IsArray(j)) return NULL;
    MySys_StrList* vec = MySys_FrameVec_new();
    cJSON* item;
    cJSON_ArrayForEach(item, j) {
        MySys_FrameVec_push(vec,
            (void*)strdup(item->valuestring ? item->valuestring : ""));
    }
    return vec;
}
```

The mangled symbol matches the user-declared type string verbatim
(only `*` becomes `P`, non-identifier chars become `_`). Framec
emits the call as `MySys_persist_pack_StrList(value)`; the linker
finds your symbol.

**Approach B — accept the int default and round-trip raw bytes.**
For homogeneous int lists this works out of the box. For other
types, prefer Approach A.

---

## Strings: `char*` and `sprintf`/`snprintf`

C has no built-in string type. Frame's `: str` annotation maps to
`char*` for C, but ownership is the user's responsibility. The Frame
runtime does not allocate, copy, or free strings on your behalf.

For string concatenation and interpolation, use `sprintf` /
`snprintf` into a buffer that you own:

```frame
$Ready {
    greet(name: char*): char* {
        self->call_count += 1;
        // Static buffer survives across function calls (single-
        // threaded use only — for multi-threaded, use a thread-local
        // or pass an out-buffer).
        static char buffer[256];
        sprintf(buffer, "Hello, %s!", name);
        @@:(buffer)
        return;
    }
}
```

The `static char buffer[256]` shape is common in Frame fixtures
because the wrapper signature returns by value (`char*` is a
pointer to a buffer the user owns) and the caller is expected to
read it before the next call clobbers the buffer. For thread-safe
or longer-lived strings, allocate via `malloc`/`strdup` and let the
caller `free` — Frame doesn't track ownership.

**Avoid `strcat` on overlapping buffers.** Standard C-string
hazard, not Frame-specific. If your handler builds up a string by
appending, prefer `snprintf(buf, sizeof(buf), "%s%s", buf, suffix)`
or `strncat` with explicit bounds.

---

## State variables: `$.var` becomes part of the compartment

State-scoped variables (`$.x`) live on the state's compartment
struct. The compartment is itself a typed struct with one field per
state-var, allocated when the state is entered:

```c
typedef struct {
    int count;
    char* name;
} _StateContext_Counting;

typedef struct {
    const char* state_name;
    _StateContext_Counting state_args;
    bool _transitioned;
} Compartment;
```

Reads and writes use `compartment->state_args.count` internally;
Frame source uses the standard `$.count` syntax which lowers to
that access path. There is no special syntax — you write
`$.count = $.count + 1` and framec emits the typed access.

State-args (declared as `$Counting(seed: int)`) and state-vars
(`$.x: int = 0`) both compile to the same compartment struct.

---

## Loop idioms — both work; idiom 1 is natural

C has `for`, `while`, and `do-while` loops. Frame's idiom 1
(`while cond { ... }`) compiles to a native C `while` block via
passthrough. Idiom 2 (state-flow loop) also works the same way it
does on every other backend.

```frame
$Counting {
    tick() {
        int i = 0;
        while (i < 10) {
            i++;
        }
    }
}
```

C-native braces and `i++` work inside the Frame `while` block — no
escaping needed.

---

## No async — language-natural skip

C has no language-level async/await. The matrix capability table
shows C's async row as 🚫 (language-natural skip). If you need
asynchronous behavior, you express it via:

- `pthread_create` for OS threads (manual synchronization).
- POSIX async I/O (`aio_*`) for I/O-bound work.
- libuv / libev / epoll-based event loops for cooperative
  multitasking.

These are user concerns, not Frame's. Write the prolog `#include`s
for whichever pattern you need, and call into them from handler
bodies via Oceans Model passthrough. The state machine itself is
synchronous and single-threaded.

---

## Multi-system per file: works as you'd expect

A `.fc` source containing multiple `@@system` blocks compiles to a
single `.c` + `.h` pair with multiple struct definitions and their
free-standing function families:

```frame
@@system Producer { ... }
@@system Consumer { ... }
```

Both `Producer_*` and `Consumer_*` function families end up in the
same `.c` file. There's no per-file structural constraint in C
(unlike Java's one-public-class rule).

---

## Cross-system fields: pointer to embedded struct

`var counter: Counter = @@Counter()` lowers to a `Counter*`
pointer field on the embedding struct, allocated via
`Counter_new(...)` in the embedder's constructor:

```c
struct Embedding {
    Counter* counter;   // pointer to a heap-allocated Counter
    // ...
};

Embedding* Embedding_new(...) {
    Embedding* self = malloc(sizeof(Embedding));
    self->counter = Counter_new(...);
    return self;
}
```

Calls to `self.counter.bump(n)` lower to `Counter_bump(self->counter, n)` —
the embedded system's own free-standing function family, called
through the pointer. Lifecycle (free/destroy) is the user's
responsibility; framec emits `_new` constructors but does not
auto-emit `_destroy` for cross-system pointers.

This matches the Phase 7 multi-system fuzz coverage (commit memory
`phase7_multisys_2026_04_27.md`); the C codegen for cross-system
pointer fields was one of the gaps closed in that round.

---

## Comments and the Oceans Model

Frame's "Oceans Model" applies to C the same way it applies to every
other backend. The comment leaders are `//` (C99 line) and
`/* ... */` (block).

```frame
@@target c

// Module-prolog block — passes through as C source.
#include <stdio.h>

@@system Counter {
    machine:
        // Section-level comments are preserved into the generated
        // .c file as native // comment blocks.
        $Counting {
            tick() { ... }
        }
}
```

Section-level leading comments are preserved into the generated
output as native `//` comment blocks attached to the corresponding
generated declaration. Linux-kernel-style multi-line `/* ... */`
comments also pass through verbatim if you write them in the
prolog.

A trailing comment after the last section in a class is handled
correctly — framec ensures a newline before the closing `}` of
the generated struct (this was a bug fix landed in commit
`08ed071`; see `memory/section_comments_complete_2026_04_27.md`).

---

## Idiomatic patterns and common gotchas

**`@@<System>(args)` returns a `System*`, not a `System`.** The
constructor returns a heap-allocated pointer — there is no stack
allocation for Frame systems on C. Always treat the returned
handle as a pointer:

```c
Counter* c = Counter_new(0);
Counter_bump(c, 5);
// ... when done:
// Counter_destroy(c);  // user responsibility
```

**`self->field`, not `self.field`.** Inside handler bodies, Frame's
`self.x` lowers to `self->x` because `self` is a pointer. If you
write native C inside a handler that uses `self`, you must use
`self->`.

**`#include`s in the prolog, not inline.** Frame's prolog (above
`@@system`) is the natural home for `#include` directives. Inside
handler bodies you can't add `#include` — those are file-scope
directives that must precede any `@@system` block.

**Initialize pointers to `NULL`.** C struct fields are
uninitialized by default if you `malloc` without `calloc`. Framec's
constructor initializes Frame's runtime fields (state, compartment,
etc) but leaves user domain fields with whatever `malloc` returned
unless you provide an explicit initializer (`name : char* = NULL`).
For cross-target portability, always provide an initializer.

**Use `static` for buffer-returning string-format helpers.** A
handler that returns a `char*` typically writes into a buffer and
returns the pointer. The buffer must outlive the function call —
`static char buf[256]` is the simplest approach (single-threaded);
heap allocation (`malloc + return`) gives the caller ownership but
requires explicit `free`.

**Brace-init for non-primitive defaults works via memcpy.** Frame's
domain field initialization for non-scalar types (e.g.
`pending: JobSlots = {0}`) emits a typed compound literal +
`memcpy` shape:

```c
{ JobSlots __init_pending = {0}; memcpy(&self->pending, &__init_pending, sizeof(self->pending)); }
```

This is C-correct (compound-literal-with-typed-storage, copied to
the field). You do not need to write the memcpy explicitly — Frame
generates it from the brace-initializer in the source. See
`memory/section_comments_complete_2026_04_27.md` for context on
the codegen fix that landed this.

**No `printf` from inside handler returns.** A handler that returns
a value via `@@:(...)` should not also `printf` *after* the
`@@:return` — terminal-statement validation (E400) prevents it.
Print before the `@@:return`, then return.

---

## Cross-references

- `docs/runtime-capability-matrix.md` — per-backend capability
  table; C shows ✅ on every row except `async` (🚫 — language-
  natural skip) and `list` domain field type (`[l]` —
  typedef-required workaround).
- `tests/common/positive/demos/21_worker_pool.fc` — canonical
  example of the `typedef` pattern for array-shaped domain
  fields.
- `tests/common/positive/primary/02_interface.fc` — canonical
  interface-method shape (`char*` strings, `static char buffer`
  return).
- `tests/common/positive/linux/06_oom_killer.fc` — multi-section
  comment preservation regression fixture.
- `framec/src/frame_c/compiler/codegen/system_codegen.rs` — the C
  domain-field constructor-body emit (incl. the brace-init memcpy
  pattern).
