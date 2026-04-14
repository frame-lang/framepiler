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
pub fn generate_rust_system(
    system: &SystemAst,
    arcanum: &Arcanum,
    source: &[u8],
) -> CodegenNode {
    // For now, delegate to the existing shared codegen path.
    // This will be incrementally replaced with Rust-specific generation
    // that uses Rc<RefCell<Compartment>> throughout.
    //
    // TODO: Move these Rust-specific pieces here:
    //   1. Runtime types (FrameEvent, FrameContext, Compartment with Rc<RefCell<>>)
    //   2. System struct fields (__compartment as Rc<RefCell<>>)
    //   3. Constructor (struct literal with Rc::new(RefCell::new(...)))
    //   4. Kernel (__kernel with borrow()/borrow_mut() access)
    //   5. Router (__router with state matching)
    //   6. State dispatch (match-based, already in state_dispatch.rs)
    //   7. Transition (__transition with Rc assignment)
    //   8. Push/Pop (Rc::clone instead of Compartment::clone)
    //   9. Interface wrappers
    //  10. Actions/Operations
    //
    // Each piece will be extracted from the shared path and adapted
    // for Rust's ownership model.

    let lang = crate::frame_c::visitors::TargetLanguage::Rust;
    let backend = super::backend::get_backend(lang);
    let syntax = backend.class_syntax();

    // Fall through to shared codegen (temporary)
    super::system_codegen::generate_system_shared(system, arcanum, lang, source)
}
