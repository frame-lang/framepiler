//! RFC-0015 / RFC-0017: `@@!SystemName()` no-initialization
//! allocation expansion.
//!
//! `@@!Counter()` creates a system instance that has had ONLY the
//! framework-only constructor run — empty compartment, empty
//! context stack, empty modal stack — with the user's `$>` start
//! handler skipped. The intended pairing is
//! `inst.restore_state(blob)` which populates the instance from a
//! saved blob; the no-init form is the empty shell that the
//! restore writes into.
//!
//! Per-target lowering is one line per backend (factory name +
//! argless construction shape) — see RFC-0017 for the
//! per-language rationale. The function is `pub(crate)` because
//! the assembler post-pass (`assembler/mod.rs`) needs to render
//! `@@!Counter()` references in native-code regions (e.g.
//! `if __name__ == "__main__":` blocks) using the same primitive.

use super::super::codegen_utils::to_snake_case;
use crate::frame_c::visitors::TargetLanguage;

/// RFC-0015 D7: render `@@!SystemName()` to the per-language no-initialization
/// primitive. The uninitialized instance has no init code run — `$Start` body and
/// `$>` handler are skipped. The user typically pairs this with
/// `inst.restore_state(data)` to populate the instance from saved bytes.
///
/// `pub(crate)` because both the handler-body codegen path (this file) and
/// the assembler post-pass (`assembler/mod.rs`, used for `@@!` in native code
/// regions like the `if __name__ == "__main__":` block) need to render to
/// the same per-language primitive. Phase 5 of the D7 plan removes the
/// duplicate post-pass entry point and consolidates here.
pub(crate) fn generate_no_initialization(name: &str, lang: TargetLanguage) -> String {
    match lang {
        // ---- Built-in primitives (no backend codegen changes needed) ----
        // Python: RFC-0017 Phase A0 — `@@!Counter()` lowers to bare
        // `Counter()` which calls the framework-only `__init__` without
        // running the user `$>`. The factory `Counter.__create(args)` is
        // the @@Counter(args) path. Replaces the prior `__new__` form
        // which bypassed framework setup entirely.
        TargetLanguage::Python3 => format!("{}()", name),
        TargetLanguage::Ruby => {
            // RFC-0017 Phase A5: bare `Counter.new` runs the
            // framework-only initialize; no user `$>` cascade.
            // Replaces D7's `.allocate` form.
            format!("{}.new", name)
        }
        TargetLanguage::JavaScript | TargetLanguage::TypeScript => {
            // RFC-0017 Phase A5: bare `new Counter()` runs framework
            // only. Replaces D7's `Object.create(Foo.prototype)`.
            format!("new {}()", name)
        }
        TargetLanguage::Lua => {
            // RFC-0017 Phase A5: bare `Counter.new()` is the
            // framework-only ctor. Replaces D7's bare metatable setup.
            format!("{}.new()", name)
        }
        TargetLanguage::Go => {
            // RFC-0017 Phase A2: `@@!Counter()` lowers to `NewCounter()`
            // which runs only framework setup (state stack, bare
            // compartment, etc.). Replaces the prior D7 `&Counter{}`
            // bare struct literal — that form skipped framework setup
            // too, leaving the instance in an invalid state for
            // `restore_state` to populate.
            format!("New{}()", name)
        }
        TargetLanguage::GDScript => {
            // RFC-0017 Phase A4: `@@!Counter()` lowers to bare
            // `Counter.new()` which calls the framework-only `_init()`
            // without invoking `_frame_init` (the user `$>` cascade).
            format!("{}.new()", name)
        }
        TargetLanguage::Php => {
            // RFC-0017 Phase A5: bare `new Counter()` runs only the
            // framework-only `__construct()`. Replaces D7's
            // `ReflectionClass::newInstanceWithoutConstructor()`.
            format!("new {}()", name)
        }
        TargetLanguage::CSharp => {
            // RFC-0017 Phase A2: `@@!Counter()` lowers to bare
            // `new Counter()` which runs only framework setup.
            // Replaces the obsolete D7 `FormatterServices.GetUninitializedObject`.
            format!("new {}()", name)
        }
        TargetLanguage::Cpp => {
            // RFC-0017 Phase A3: `@@!Counter()` lowers to bare `Counter()`
            // which runs framework setup only (no user `$>` cascade).
            format!("{}()", name)
        }
        TargetLanguage::C => {
            // RFC-0017 Phase A3: `@@!Foo()` lowers to bare `Foo_new()`
            // which calloc's the struct and runs framework setup
            // (empty compartment, state stack init) without firing the
            // user `$>` cascade. Replaces the obsolete D7 `Foo_alloc()`
            // (which skipped framework setup entirely).
            format!("{}_new()", name)
        }
        TargetLanguage::Rust => {
            // RFC-0017 Phase A1: `@@!Counter()` lowers to bare
            // `Counter::new()`, which is now the framework-only
            // constructor (does NOT run user `$>`). The `__create`
            // factory is the @@Counter(args) path.
            format!("{}::new()", name)
        }
        TargetLanguage::Dart => {
            // RFC-0017 Phase A4: `@@!Counter()` lowers to bare
            // `Counter()` which runs framework setup only (no user
            // `$>` cascade). Replaces the obsolete D7 `Counter._no_init()`
            // named constructor.
            format!("{}()", name)
        }
        TargetLanguage::Java => {
            // RFC-0017 Phase A1: `@@!Counter()` lowers to the bare
            // constructor `new Counter()` which runs only framework
            // setup (no `$>` cascade). Replaces the obsolete D7
            // `Counter.__no_init()` static factory.
            format!("new {}()", name)
        }
        TargetLanguage::Kotlin => {
            // RFC-0017 Phase A1: `@@!Counter()` lowers to the bare
            // primary constructor `Counter()` which runs the
            // framework-only `init {}` block without invoking
            // `__frame_init` (the user `$>` cascade entry point).
            // Replaces the obsolete D7 `Counter.__no_init()` factory.
            format!("{}()", name)
        }
        TargetLanguage::Swift => {
            // RFC-0017 Phase A2: `@@!Counter()` lowers to bare
            // `Counter()` which runs only framework setup. Replaces
            // the obsolete D7 `Counter.__no_init()` factory.
            format!("{}()", name)
        }
        TargetLanguage::Erlang => {
            // RFC-0017 Phase A6: `@@!Counter()` lowers to bare
            // `module:start_link()` unwrapped. The bare init sets
            // `frame_skip_enter__ = true` so the user `$>` body never
            // fires until `frame_init/(N+1)` is called explicitly.
            // Returns a Pid (matches the @@! call site shape).
            format!("element(2, {}:start_link())", to_snake_case(name))
        }
        _ => format!(
            "/* @@! no-initialization allocation not yet wired for {:?} ({}); see RFC-0015 D7 */",
            lang, name
        ),
    }
}
