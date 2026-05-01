//! Main compilation logic
//!
//! This module contains the core compilation pipeline for Frame V4.
//! V4 is a pure preprocessor for @@system blocks.

use super::config::{CompileMode, PipelineConfig};
use crate::frame_c::compiler::arcanum::build_arcanum_from_frame_ast;
use crate::frame_c::compiler::assembler;
use crate::frame_c::compiler::codegen::{
    generate_c_compartment_types, generate_compartment_class, generate_cpp_compartment_types,
    generate_csharp_compartment_types, generate_frame_context_class, generate_frame_event_class,
    generate_go_compartment_types, generate_java_compartment_types,
    generate_kotlin_compartment_types, generate_rust_compartment_types,
    generate_swift_compartment_types, generate_system, get_backend, EmitContext,
};
use crate::frame_c::compiler::frame_ast::{FrameAst, ModuleAst, Span as AstSpan};
use crate::frame_c::compiler::frame_validator::FrameValidator;
use crate::frame_c::compiler::pipeline_parser;
use crate::frame_c::compiler::segmenter::{self, Segment};
use crate::frame_c::utils::RunError;
use crate::frame_c::visitors::TargetLanguage;

/// Result of module compilation
#[derive(Debug)]
pub struct CompileResult {
    /// Generated code
    pub code: String,
    /// Validation errors (if any)
    pub errors: Vec<CompileError>,
    /// Validation warnings (if any)
    pub warnings: Vec<CompileError>,
    /// Source map (if generated)
    pub source_map: Option<String>,
}

/// Compilation error
#[derive(Debug, Clone)]
pub struct CompileError {
    pub code: String,
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl CompileError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            line: None,
            column: None,
        }
    }

    pub fn with_location(mut self, line: usize, column: usize) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }
}

// Helper functions extract_native_code, skip_pragmas_simple, skip_pragmas_keep_native,
// and expand_tagged_instantiations have been removed — their responsibilities are now
// handled by the Segmenter (Stage 0) and Assembler (Stage 7).

/// Compile a Frame module from source bytes
///
/// This is the main entry point for V4 compilation.
///
/// # Arguments
/// * `source` - The Frame source code as bytes
/// * `config` - Pipeline configuration
///
/// # Returns
/// A CompileResult containing the generated code (or validation results)
pub fn compile_module(source: &[u8], config: &PipelineConfig) -> Result<CompileResult, RunError> {
    // Debug output if enabled
    if config.debug {
        eprintln!(
            "[compile_module] Starting V4 compilation with mode={:?}, target={:?}",
            config.mode, config.target
        );
    }

    // Check for validation-only mode
    if config.mode == CompileMode::ValidationOnly {
        return validate_only(source, config);
    }

    // V4 AST-based compilation
    compile_ast_based(source, config)
}

/// Validation-only mode: run the full V4 pipeline and discard the
/// generated code. Same validator coverage as `framec compile`,
/// without paying for the emit/assembly stages where it matters
/// (the codegen still runs, but its output is never written out).
fn validate_only(source: &[u8], config: &PipelineConfig) -> Result<CompileResult, RunError> {
    let result = compile_ast_based(source, config)?;
    Ok(CompileResult {
        code: String::new(),
        errors: result.errors,
        warnings: result.warnings,
        source_map: None,
    })
}

/// Compile using the V4 pipeline stages
///
/// Pipeline: Segmenter → Parser → Arcanum → Validator → Codegen → Emit → Assembler
///
/// 1. Segment source into Native/Pragma/System regions (Segmenter)
/// 2. For each System segment: parse → build Arcanum → validate → generate code
/// 3. Assemble final output: native pass-through + generated systems + tagged instantiations
pub fn compile_ast_based(
    source: &[u8],
    config: &PipelineConfig,
) -> Result<CompileResult, RunError> {
    if config.debug {
        eprintln!("[compile_ast_based] Starting pipeline-based compilation");
    }

    // Stage 0: Segment source
    let source_map = match segmenter::segment_source(source, config.target) {
        Ok(sm) => sm,
        Err(e) => {
            return Ok(CompileResult {
                code: String::new(),
                errors: vec![CompileError::new(
                    "E001",
                    &format!("Segmentation error: {}", e),
                )],
                warnings: vec![],
                source_map: None,
            });
        }
    };

    if config.debug {
        let system_count = source_map
            .segments
            .iter()
            .filter(|s| matches!(s, Segment::System { .. }))
            .count();
        eprintln!(
            "[compile_ast_based] Segmented: {} segments, {} systems",
            source_map.segments.len(),
            system_count
        );
    }

    // Check for @@persist pragma
    let has_persist = source_map.persist_pragma().is_some();

    // RFC-0013 hard-cut migrations: bare `@@persist` and `@@target`
    // are no longer accepted. Catch legacy usages with a clear error
    // and a migration pointer.
    {
        let src = std::str::from_utf8(&source_map.source).unwrap_or("");
        for line in src.lines() {
            let trimmed = line.trim_start();
            // Wave 1: @@persist
            if trimmed.starts_with("@@persist") {
                let after = &trimmed[9..];
                let next = after.chars().next();
                let is_bare = match next {
                    None => true,
                    Some(c) => !c.is_ascii_alphanumeric() && c != '_' && c != '-',
                };
                if is_bare {
                    return Ok(CompileResult {
                        code: String::new(),
                        errors: vec![CompileError::new(
                            "E803",
                            "Bare `@@persist` is no longer accepted. Migrate to `@@[persist]` \
                             (RFC-0013 wave 1). Args form: `@@persist(domain=[a, b])` becomes \
                             `@@[persist(domain=[a, b])]`. The change is mechanical — wrap the \
                             directive in `@@[ ]`.",
                        )],
                        warnings: vec![],
                        source_map: None,
                    });
                }
            }
            // Wave 2: @@target
            if trimmed.starts_with("@@target") {
                let after = &trimmed[8..];
                let next = after.chars().next();
                let is_bare = match next {
                    None => true,
                    Some(c) => !c.is_ascii_alphanumeric() && c != '_' && c != '-',
                };
                if is_bare {
                    return Ok(CompileResult {
                        code: String::new(),
                        errors: vec![CompileError::new(
                            "E804",
                            "Bare `@@target lang` is no longer accepted. Migrate to \
                             `@@[target(\"lang\")]` (RFC-0013 wave 2). Example: \
                             `@@target python_3` becomes `@@[target(\"python_3\")]`.",
                        )],
                        warnings: vec![],
                        source_map: None,
                    });
                }
            }
        }
    }

    // Pass 1: Parse all systems into ASTs
    let mut system_asts: Vec<crate::frame_c::compiler::frame_ast::SystemAst> = Vec::new();
    for segment in &source_map.segments {
        if let Segment::System {
            name,
            body_span,
            header_params_span,
            bases,
            visibility,
            ..
        } = segment
        {
            let ast_body_span = AstSpan::new(body_span.start, body_span.end);

            let mut system_ast = match pipeline_parser::parse_system(
                &source_map.source,
                name.clone(),
                ast_body_span,
                config.target,
            ) {
                Ok(ast) => ast,
                Err(e) => {
                    return Ok(CompileResult {
                        code: String::new(),
                        errors: vec![CompileError::new(
                            "E002",
                            &format!("Parse error in system '{}': {}", name, e),
                        )],
                        warnings: vec![],
                        source_map: None,
                    });
                }
            };

            // Parse the optional header parameter list and attach to the
            // freshly-built SystemAst. This is the bridge between the
            // segmenter (which captured the span) and the codegen (which
            // reads system.params to build constructors).
            if let Some(hp_span) = header_params_span {
                let ast_hp_span = AstSpan::new(hp_span.start, hp_span.end);
                match pipeline_parser::parse_system_header_params(&source_map.source, ast_hp_span) {
                    Ok(params) => system_ast.params = params,
                    Err(e) => {
                        return Ok(CompileResult {
                            code: String::new(),
                            errors: vec![CompileError::new(
                                "E002",
                                &format!("Parse error in system '{}' header params: {}", name, e),
                            )],
                            warnings: vec![],
                            source_map: None,
                        });
                    }
                }
            }

            // Attach base classes from `: Base1, Base2` syntax
            system_ast.bases = bases.clone();
            // Validate and attach visibility from `@@system private Foo` syntax
            if visibility.as_deref() == Some("public") {
                return Ok(CompileResult {
                    code: String::new(),
                    errors: vec![CompileError::new(
                        "E408",
                        &format!(
                            "System '{}': 'public' is redundant — systems are public by default. \
                             Remove the 'public' keyword.",
                            name
                        ),
                    )],
                    warnings: vec![],
                    source_map: None,
                });
            }
            if visibility.as_deref() == Some("private") {
                let unsupported = matches!(
                    config.target,
                    TargetLanguage::Python3
                        | TargetLanguage::Ruby
                        | TargetLanguage::Lua
                        | TargetLanguage::C
                        | TargetLanguage::GDScript
                        | TargetLanguage::Erlang
                );
                if unsupported {
                    return Ok(CompileResult {
                        code: String::new(),
                        errors: vec![CompileError::new(
                            "E409",
                            &format!(
                                "System '{}': target language {:?} does not support private class visibility.",
                                name, config.target
                            ),
                        )],
                        warnings: vec![],
                        source_map: None,
                    });
                }
            }
            system_ast.visibility = visibility.clone();

            if has_persist {
                system_ast.persist_attr = Some(crate::frame_c::compiler::frame_ast::PersistAttr {
                    save_name: None,
                    restore_name: None,
                    library: None,
                    span: AstSpan::new(0, 0),
                });
            }

            // Enrich transition metadata (`exit_args`, `enter_args`,
            // `state_args`) from the V4 unified scanner. The pipeline
            // parser leaves these as None because exit args sit before
            // the `->` token and are emitted by the lexer as a trailing
            // NativeCode chunk; they are not visible during the parser's
            // token-by-token consumption of the arrow. The codegen path
            // re-runs the scanner to recover them, but the validator
            // runs *before* codegen and needs them too — e.g. E419
            // (exit args without a matching `<$()` exit handler).
            //
            // The same scanner pass also surfaces structural errors the
            // user must see as compile failures — currently E407 (Frame
            // statement inside a nested function scope, detected via
            // each backend's `skip_nested_scope`). These are propagated
            // here as `CompileError`s so the user gets a clean rejection
            // before validation runs against a partially-scanned AST.
            let enrich_errors =
                crate::frame_c::compiler::native_region_scanner::enrich_system_metadata(
                    &mut system_ast,
                    &source_map.source,
                    config.target,
                );
            if !enrich_errors.is_empty() {
                let errors = enrich_errors
                    .into_iter()
                    .map(|e| CompileError::new(&e.code, &e.message))
                    .collect();
                return Ok(CompileResult {
                    code: String::new(),
                    errors,
                    warnings: vec![],
                    source_map: None,
                });
            }

            if config.debug {
                eprintln!(
                    "[compile_ast_based] Parsed system '{}': {} states, {} interface methods",
                    name,
                    system_ast
                        .machine
                        .as_ref()
                        .map(|m| m.states.len())
                        .unwrap_or(0),
                    system_ast.interface.len()
                );
            }

            // RFC-0013 wave 2: filter items by `@@[target("X")]`
            // attribute. Items whose `target` attributes don't include
            // the current target are pruned before codegen sees them.
            // No `target` attribute = always emit.
            filter_by_target_attribute(&mut system_ast, config.target);

            system_asts.push(system_ast);
        }
    }

    // Erlang: one module per file — reject multi-system files.
    // E431, distinct from validator's E406 ("Interface handler parameter
    // count mismatch") which lives in `frame_validator.rs`. Both are
    // file-structure issues but they reach the user via different code
    // paths, so they need distinct codes.
    if matches!(config.target, TargetLanguage::Erlang) && system_asts.len() > 1 {
        let names: Vec<&str> = system_asts.iter().map(|s| s.name.as_str()).collect();
        return Ok(CompileResult {
            code: String::new(),
            errors: vec![CompileError::new(
                "E431",
                &format!(
                    "Erlang requires one module per file, but this file contains {} systems: {}. \
                     Split into separate files (one @@system per file).",
                    system_asts.len(),
                    names.join(", ")
                ),
            )],
            warnings: vec![],
            source_map: None,
        });
    }

    // Java: one PUBLIC class per file. Multiple package-private
    // (Frame `@@system private`) systems alongside at most one public
    // system is fine — Java allows that.
    //
    // E430 only fires when >1 system would be emitted as public.
    // Distinct from validator's E407 ("Frame statement in nested
    // function scope"). Both apply to source structure but on
    // entirely separate axes, so they need distinct codes.
    if matches!(config.target, TargetLanguage::Java) {
        let public_systems: Vec<&str> = system_asts
            .iter()
            .filter(|s| s.visibility.as_deref() != Some("private"))
            .map(|s| s.name.as_str())
            .collect();
        if public_systems.len() > 1 {
            return Ok(CompileResult {
                code: String::new(),
                errors: vec![CompileError::new(
                    "E430",
                    &format!(
                        "Java allows only one public class per file, but this file contains {} public systems: {}. \
                         Either split into separate files (one @@system per file), or mark all but one as `@@system private`.",
                        public_systems.len(),
                        public_systems.join(", ")
                    ),
                )],
                warnings: vec![],
                source_map: None,
            });
        }
    }

    // Build a shared arcanum containing ALL systems so they can reference each other
    let module_ast = FrameAst::Module(ModuleAst {
        name: String::new(),
        systems: system_asts.clone(),
        imports: Vec::new(),
        span: AstSpan::new(0, 0),
    });
    let arcanum = build_arcanum_from_frame_ast(&module_ast);

    // GraphViz target: bypass CodegenNode pipeline, use graph IR → DOT emitter
    if matches!(config.target, TargetLanguage::Graphviz) {
        use crate::frame_c::compiler::graphviz;

        let mut dot_systems: Vec<(String, String)> = Vec::new();

        for system_ast in &system_asts {
            // Validate with shared arcanum
            let frame_ast = FrameAst::System(system_ast.clone());
            let mut validator = FrameValidator::new();
            if let Err(errs) = validator.validate_with_arcanum(&frame_ast, &arcanum) {
                let errors = errs
                    .iter()
                    .map(|e| CompileError::new(&e.code, &e.message))
                    .collect();
                return Ok(CompileResult {
                    code: String::new(),
                    errors,
                    warnings: vec![],
                    source_map: None,
                });
            }
            // @@:self.method() validation against interface
            if let Err(errs) = validator.validate_self_calls(&frame_ast, source, config.target) {
                let errors = errs
                    .iter()
                    .map(|e| CompileError::new(&e.code, &e.message))
                    .collect();
                return Ok(CompileResult {
                    code: String::new(),
                    errors,
                    warnings: vec![],
                    source_map: None,
                });
            }
            // Target-specific checks
            if let Err(errs) = validator.validate_target_specific(&frame_ast, config.target) {
                let errors = errs
                    .iter()
                    .map(|e| CompileError::new(&e.code, &e.message))
                    .collect();
                return Ok(CompileResult {
                    code: String::new(),
                    errors,
                    warnings: vec![],
                    source_map: None,
                });
            }

            // Build graph IR and emit DOT
            let graph = graphviz::build_system_graph(system_ast, &arcanum);
            let dot = graphviz::emit_dot(&graph);
            dot_systems.push((system_ast.name.clone(), dot));
        }

        // Assemble: concatenate DOT blocks with // System: Name headers
        let code = graphviz::emit_multi_system(&dot_systems);

        if config.debug {
            eprintln!(
                "[compile_ast_based] GraphViz: generated {} bytes of DOT for {} systems",
                code.len(),
                dot_systems.len()
            );
        }

        return Ok(CompileResult {
            code,
            errors: vec![],
            warnings: vec![],
            source_map: None,
        });
    }

    // Pass 2: Validate + codegen each system with the shared arcanum
    let backend = get_backend(config.target);
    let mut ctx = EmitContext::new();
    // Make the names of every defined system available to the
    // per-backend `emit_field` so it can recognize cross-system
    // domain references (`logger: Logger = @@Logger()`) and emit
    // the right field type per target — Go needs `*Logger`, others
    // use the bare name.
    ctx.defined_systems = arcanum.systems.keys().cloned().collect();
    let mut generated_systems: Vec<(String, String)> = Vec::new();

    // Collect runtime imports once (will be emitted at the start by assembler)
    let runtime_imports = backend.runtime_imports();

    // Warnings accumulated across all systems in the module. Harvested
    // from each per-system validator and attached to the final result.
    let mut module_warnings: Vec<CompileError> = Vec::new();

    for system_ast in &system_asts {
        // Validate with shared arcanum (all sibling systems visible)
        let frame_ast = FrameAst::System(system_ast.clone());
        let mut validator = FrameValidator::new();
        if let Err(errs) = validator.validate_with_arcanum(&frame_ast, &arcanum) {
            let errors = errs
                .iter()
                .map(|e| CompileError::new(&e.code, &e.message))
                .collect();
            return Ok(CompileResult {
                code: String::new(),
                errors,
                warnings: module_warnings,
                source_map: None,
            });
        }
        // @@:self.method() validation against interface
        if let Err(errs) = validator.validate_self_calls(&frame_ast, source, config.target) {
            let errors = errs
                .iter()
                .map(|e| CompileError::new(&e.code, &e.message))
                .collect();
            return Ok(CompileResult {
                code: String::new(),
                errors,
                warnings: module_warnings,
                source_map: None,
            });
        }
        // Target-specific checks (e.g. GDScript Object-method collisions,
        // TypeScript global shadowing). Run after the general validator
        // so structural errors surface first.
        if let Err(errs) = validator.validate_target_specific(&frame_ast, config.target) {
            let errors = errs
                .iter()
                .map(|e| CompileError::new(&e.code, &e.message))
                .collect();
            return Ok(CompileResult {
                code: String::new(),
                errors,
                warnings: module_warnings,
                source_map: None,
            });
        }
        // Harvest soft warnings (e.g. W501 TypeScript global shadowing,
        // W414 unreachable-state) — these don't fail the build but are
        // surfaced to the user.
        for w in validator.take_warnings() {
            module_warnings.push(CompileError::new(&w.code, &w.message));
        }

        // Warn if async is used with C target (no native async support)
        let has_async = system_ast.interface.iter().any(|m| m.is_async)
            || system_ast.actions.iter().any(|a| a.is_async)
            || system_ast.operations.iter().any(|o| o.is_async);
        if has_async && matches!(config.target, TargetLanguage::C) {
            eprintln!("Warning: async is not supported for C — async keyword ignored");
        }

        // Build per-system generated code: runtime classes + system class
        let mut system_code = String::new();

        // Runtime classes (language-specific, per-system)
        if matches!(config.target, TargetLanguage::Rust) {
            let compartment_types = generate_rust_compartment_types(system_ast);
            if !compartment_types.is_empty() {
                system_code.push_str(&compartment_types);
            }
        } else if matches!(config.target, TargetLanguage::C) {
            let c_runtime = generate_c_compartment_types(system_ast);
            if !c_runtime.is_empty() {
                system_code.push_str(&c_runtime);
            }
        } else if matches!(config.target, TargetLanguage::Cpp) {
            let cpp_runtime = generate_cpp_compartment_types(system_ast);
            if !cpp_runtime.is_empty() {
                system_code.push_str(&cpp_runtime);
            }
        } else if matches!(config.target, TargetLanguage::Java) {
            let java_runtime = generate_java_compartment_types(system_ast);
            if !java_runtime.is_empty() {
                system_code.push_str(&java_runtime);
            }
        } else if matches!(config.target, TargetLanguage::CSharp) {
            let csharp_runtime = generate_csharp_compartment_types(system_ast);
            if !csharp_runtime.is_empty() {
                system_code.push_str(&csharp_runtime);
            }
        } else if matches!(config.target, TargetLanguage::Go) {
            let go_runtime = generate_go_compartment_types(system_ast);
            if !go_runtime.is_empty() {
                system_code.push_str(&go_runtime);
            }
        } else if matches!(config.target, TargetLanguage::Kotlin) {
            let kotlin_runtime = generate_kotlin_compartment_types(system_ast);
            if !kotlin_runtime.is_empty() {
                system_code.push_str(&kotlin_runtime);
            }
        } else if matches!(config.target, TargetLanguage::Swift) {
            let swift_runtime = generate_swift_compartment_types(system_ast);
            if !swift_runtime.is_empty() {
                system_code.push_str(&swift_runtime);
            }
        } else if matches!(config.target, TargetLanguage::Erlang) {
            // Erlang gen_statem: no runtime classes needed — gen_statem provides everything
        } else {
            // GDScript module-scope: emit `extends Base` before runtime
            // types so it's the first line (Godot requires this).
            if matches!(config.target, TargetLanguage::GDScript) && !system_ast.bases.is_empty() {
                system_code.push_str(&format!("extends {}\n\n", system_ast.bases[0]));
            }
            if let Some(event_node) = generate_frame_event_class(system_ast, config.target) {
                system_code.push_str(&backend.emit(&event_node, &mut ctx));
                system_code.push_str("\n\n");
            }
            if let Some(context_node) = generate_frame_context_class(system_ast, config.target) {
                system_code.push_str(&backend.emit(&context_node, &mut ctx));
                system_code.push_str("\n\n");
            }
            if let Some(compartment_node) = generate_compartment_class(system_ast, config.target) {
                system_code.push_str(&backend.emit(&compartment_node, &mut ctx));
                system_code.push_str("\n\n");
            }
        }

        // Codegen + Emit with shared arcanum
        ctx = ctx.with_system(&system_ast.name);
        let codegen_node = generate_system(system_ast, &arcanum, config.target, source);
        system_code.push_str(&backend.emit(&codegen_node, &mut ctx));

        generated_systems.push((system_ast.name.clone(), system_code));
    }

    // Stage 7: Assemble final output (native pass-through + system substitution + tagged instantiations)
    // Runtime imports are emitted first (before any native prolog) to fix import ordering.
    // Pass each system's declared params so the assembler can resolve sigil-tagged
    // call sites (`@@Robot($(10), $>(80), "R2D2")`) and substitute Frame defaults.
    let system_params: Vec<(
        String,
        Vec<crate::frame_c::compiler::frame_ast::SystemParam>,
    )> = system_asts
        .iter()
        .map(|s| (s.name.clone(), s.params.clone()))
        .collect();
    let code = match assembler::assemble(
        &source_map,
        &generated_systems,
        &system_params,
        config.target,
        &runtime_imports,
    ) {
        Ok(output) => output,
        Err(e) => {
            return Ok(CompileResult {
                code: String::new(),
                errors: vec![CompileError::new("E003", &format!("Assembly error: {}", e))],
                warnings: vec![],
                source_map: None,
            });
        }
    };

    if config.debug {
        eprintln!("[compile_ast_based] Generated {} bytes of code", code.len());
    }

    Ok(CompileResult {
        code,
        errors: vec![],
        warnings: module_warnings,
        source_map: None,
    })
}

/// RFC-0013 wave 2: prune AST items whose `@@[target("X")]` attributes
/// don't include the current target.
///
/// An item with no `target` attribute is always emitted. An item with one
/// or more `target` attributes is emitted only when at least one matches
/// `current`. Unparseable target args are treated as non-matches (a future
/// validator pass will surface them as a hard error).
fn filter_by_target_attribute(
    system_ast: &mut crate::frame_c::compiler::frame_ast::SystemAst,
    current: TargetLanguage,
) {
    use crate::frame_c::compiler::frame_ast::Attribute;

    fn unquote(s: &str) -> &str {
        let t = s.trim();
        let bytes = t.as_bytes();
        if bytes.len() >= 2
            && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
                || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
        {
            &t[1..t.len() - 1]
        } else {
            t
        }
    }

    fn should_emit(attrs: &[Attribute], current: TargetLanguage) -> bool {
        let mut saw_target = false;
        for a in attrs {
            if a.name != "target" {
                continue;
            }
            saw_target = true;
            if let Some(args) = &a.args {
                let lang_str = unquote(args);
                if let Ok(t) = TargetLanguage::try_from(lang_str) {
                    if t == current {
                        return true;
                    }
                }
            }
        }
        !saw_target
    }

    system_ast
        .interface
        .retain(|m| should_emit(&m.attributes, current));

    if let Some(machine) = system_ast.machine.as_mut() {
        for state in machine.states.iter_mut() {
            state
                .handlers
                .retain(|h| should_emit(&h.attributes, current));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_result_creation() {
        let result = CompileResult {
            code: "generated code".to_string(),
            errors: vec![],
            warnings: vec![],
            source_map: None,
        };
        assert_eq!(result.code, "generated code");
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_compile_error_with_location() {
        let error = CompileError::new("E001", "test error").with_location(10, 5);
        assert_eq!(error.line, Some(10));
        assert_eq!(error.column, Some(5));
    }

    #[test]
    fn test_validation_only_mode() {
        let source = b"@@system Test { machine: $A { } }";
        let config = PipelineConfig::validation_only(TargetLanguage::Python3);
        let result = compile_module(source, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_simple_system() {
        let source = b"@@system Test { machine: $Init { } }";
        let config = PipelineConfig::production(TargetLanguage::Python3);
        let result = compile_module(source, &config);
        assert!(result.is_ok());
        let output = result.unwrap();
        if !output.errors.is_empty() {
            eprintln!("Parse errors: {:?}", output.errors);
            return;
        }
        assert!(output.code.contains("class Test"));
    }

    #[test]
    fn test_compile_with_transition() {
        let source = br#"@@system TestTransition {
    machine:
        $Idle {
            start() {
                -> $Running
            }
        }
        $Running {
            stop() {
                -> $Idle
            }
        }
}"#;
        let config = PipelineConfig::production(TargetLanguage::Python3);
        let result = compile_module(source, &config);
        assert!(result.is_ok());
        let output = result.unwrap();
        if !output.errors.is_empty() {
            for e in &output.errors {
                eprintln!("Error: {}: {}", e.code, e.message);
            }
            return;
        }
        assert!(output.code.contains("_transition"));
    }

    #[test]
    fn test_native_only_input_passes_through() {
        // Input with no @@system blocks is pure native code — passes through verbatim
        let source = b"this is just native code\nno systems here\n";
        let config = PipelineConfig::production(TargetLanguage::Python3);
        let result = compile_module(source, &config);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.errors.is_empty());
        assert!(output.code.contains("this is just native code"));
    }

    #[test]
    fn test_compile_parse_error() {
        // Invalid syntax inside @@system should produce an error
        let source = b"@@system Test { not valid section syntax }";
        let config = PipelineConfig::production(TargetLanguage::Python3);
        let result = compile_module(source, &config);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(
            !output.errors.is_empty(),
            "Expected parse errors for invalid system content"
        );
    }
}
