//! Per-language handler-method emitters.
//!
//! Each submodule emits one `_s_<State>_hdl_<kind>_<event>(...)`
//! method for a single backend. The parent module's
//! `generate_per_handler_method_for_lang` dispatcher matches on
//! `TargetLanguage` and calls the appropriate
//! `generate_<lang>_handler_method` here. All 14 emitters share
//! the same signature; the bodies differ by per-target syntax
//! (parameter binding, state-var init guards, type casts, etc.).

pub(super) mod c;
pub(super) mod cpp;
pub(super) mod csharp;
pub(super) mod dart;
pub(super) mod gdscript;
pub(super) mod go;
pub(super) mod java;
pub(super) mod kotlin;
pub(super) mod lua;
pub(super) mod php;
pub(super) mod python;
pub(super) mod ruby;
pub(super) mod swift;
pub(super) mod typescript;
