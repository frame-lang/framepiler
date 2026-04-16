//! Rust system code generation.
//!
//! Generates a complete Rust module from a Frame system AST.
//! Uses `Rc<RefCell<Compartment>>` for compartment ownership —
//! push$ clones the Rc (cheap pointer copy), no deep clone needed.
//! Enter/exit args use `Box<dyn Any>` for type-safe parameter passing.
//!
//! This module owns the full Rust codegen pipeline, similar to how
//! `erlang_system.rs` owns Erlang's gen_statem generation.

use super::ast::CodegenNode;
use super::codegen_utils::HandlerContext;
use crate::frame_c::compiler::arcanum::Arcanum;
use crate::frame_c::compiler::frame_ast::SystemAst;

/// Generate the complete Rust system from a Frame AST.
///
/// Called from `system_codegen::generate_system` when target is Rust.
/// Returns a CodegenNode containing the full Rust implementation.
pub fn generate_rust_system(system: &SystemAst, arcanum: &Arcanum, source: &[u8]) -> CodegenNode {
    // Delegates to the shared codegen path. A future refactor could
    // extract Rust-specific pieces (Rc<RefCell<Compartment>>, ownership-
    // aware transitions, borrow()/borrow_mut() accessors) into this
    // module, mirroring how erlang_system.rs owns its full pipeline.

    let lang = crate::frame_c::visitors::TargetLanguage::Rust;
    let backend = super::backend::get_backend(lang);
    let syntax = backend.class_syntax();

    // Fall through to shared codegen (temporary)
    super::system_codegen::generate_system_shared(system, arcanum, lang, source)
}
