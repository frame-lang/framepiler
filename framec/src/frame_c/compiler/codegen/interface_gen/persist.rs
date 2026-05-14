//! Per-language persist codegen.
//!
//! Wire format and contract details vary per backend:
//!
//! - **Field-by-field JSON** (Python, JS/TS, Ruby, Lua, PHP, Dart,
//!   GDScript) — uniform shape, `@@[no_persist]` per-field skip,
//!   nested-system fields round-trip via the child's `save_state` /
//!   `restore_state`.
//! - **Whole-object** (Rust serde, Java/Kotlin Jackson, C++ nlohmann,
//!   C# `System.Text.Json`) — relies on the language's reflective
//!   deserialization; `@@[no_persist]` becomes a per-language ignore
//!   annotation.
//! - **gen_statem state-passing** (Erlang) — no save/restore methods;
//!   handled inside `erlang_system.rs`.
//! - **Bespoke** (C — manual JSON; Go — encoding/json with struct
//!   tags; Swift — Codable).
//!
//! Each backend lives in its own submodule and exposes a single
//! `generate(system) -> Vec<CodegenNode>` function. The dispatcher
//! in `interface_gen::generate_persistence_methods` matches on
//! `TargetLanguage` and delegates.
//!
//! Rust is handled out-of-tree by
//! `rust_system::generate_rust_persistence_methods`. Erlang is a
//! no-op here (gen_statem owns persist natively).

pub(super) mod javascript;
pub(super) mod lua;
pub(super) mod python;
