//! Frame V4 Pipeline Infrastructure
//!
//! This module provides the core compilation pipeline infrastructure:
//! - `traits.rs`: RegionScanner trait and scanner factory
//! - `config.rs`: PipelineConfig and CompileMode enum
//! - `compiler.rs`: Main compilation logic extracted from mod.rs
//!
//! This replaces the duplicated scanner dispatch code that was spread across mod.rs.

pub mod compiler;
pub mod config;
pub mod traits;

pub use compiler::{compile_ast_based, compile_module, CompileError, CompileResult};
pub use config::{CompileMode, PipelineConfig, TrailerConfig, ValidationConfig};
pub use traits::{get_region_scanner, RegionScannerTrait};
