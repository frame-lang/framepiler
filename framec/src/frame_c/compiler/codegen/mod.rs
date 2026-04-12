//! Frame V4 Code Generation Infrastructure
//!
//! This module provides a proper AST-based code generation architecture:
//! - `ast.rs`: CodegenNode - Language-agnostic intermediate representation
//! - `backend.rs`: LanguageBackend trait for language-specific emission
//! - `system_codegen.rs`: System code generation from Frame AST (class-based backends)
//! - `erlang_system.rs`: Erlang gen_statem code generation (bypasses class pipeline)
//! - `backends/`: Language-specific backend implementations
//!
//! This replaces the string-template code generation with proper AST traversal.

pub mod ast;
pub mod backend;
pub mod backends;
pub mod block_transform;
pub mod codegen_utils;
pub mod erlang_system;
pub mod frame_expansion;
pub mod interface_gen;
pub mod runtime;
pub mod state_dispatch;
pub mod system_codegen;

pub use ast::CodegenNode;
pub use backend::{get_backend, ClassSyntax, EmitContext, LanguageBackend};
pub use runtime::{
    generate_c_compartment_types, generate_compartment_class, generate_cpp_compartment_types,
    generate_csharp_compartment_types, generate_frame_context_class, generate_frame_event_class,
    generate_go_compartment_types, generate_java_compartment_types,
    generate_kotlin_compartment_types, generate_rust_compartment_types,
    generate_swift_compartment_types,
};
pub use system_codegen::generate_system;
