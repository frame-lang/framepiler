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
// and expand_system_instantiations have been removed — their responsibilities are now
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
/// 3. Assemble final output: native pass-through + generated systems + system instantiations
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
            // RFC-0024: @@import removed entirely. Cross-file
            // dependencies are expressed in the target language's
            // native syntax (`from .x import Y` for Python, `use
            // crate::x::Y;` for Rust, `const Y = preload(...)` for
            // GDScript, etc.) written as Oceans Model pass-through.
            if trimmed.starts_with("@@import") {
                let after = &trimmed[8..];
                let next = after.chars().next();
                let is_import_directive = match next {
                    None => true,
                    Some(c) => c.is_whitespace() || c == '"',
                };
                if is_import_directive {
                    return Ok(CompileResult {
                        code: String::new(),
                        errors: vec![CompileError::new(
                            "E823",
                            "`@@import \"<path>\"` is no longer accepted (RFC-0024). \
                             Write the target language's native import syntax outside \
                             any `@@system` block instead — `from .other import X` for \
                             Python, `use crate::other::X;` for Rust, `const X = \
                             preload(\"res://other.gd\")` for GDScript, `#include \
                             \"other.h\"` for C/C++, etc. framec passes the line \
                             through unchanged (Oceans Model). See RFC-0024 for the \
                             full per-target migration table.",
                        )],
                        warnings: vec![],
                        source_map: None,
                    });
                }
            }
        }
    }

    // Pass 1: Parse all systems into ASTs.
    //
    // Walk segments in source order so module-level lifecycle pragmas
    // (RFC-0014 `@@[main]` and RFC-0015 `@@[create]` / `@@[save]` /
    // `@@[load]`) attach to the *next* `@@system` declaration they
    // precede. The buffers reset after attachment so a stray pragma
    // followed by native code-then-system doesn't bleed into a later
    // system.
    let mut system_asts: Vec<crate::frame_c::compiler::frame_ast::SystemAst> = Vec::new();
    // RFC-0022: `@@import "path"` module-scope directives accumulate here
    // before being attached to the ModuleAst. Phase 1 stores the raw
    // (quote-stripped) path; symbols + alias remain empty (lax mode).
    let mut module_imports: Vec<crate::frame_c::compiler::frame_ast::Import> = Vec::new();
    // RFC-0022 strict-mode import errors collected during the segment
    // walk. Surfaced as compile errors at the end of the pass so the
    // user sees every unresolved import in one shot.
    let mut strict_import_errors: Vec<CompileError> = Vec::new();
    // RFC-0022 + FRAMEC_BUGS.md Issue #8: imported `@@system` names
    // that carry an `@@[save]` / `@@[load]` attribute (i.e. use the
    // new persist contract). Merged into `NEW_CONTRACT_SYSTEMS` so the
    // parent's domain-field restore codegen picks the instance-method
    // shape (`Type.new()` + `inst.restore_state(bytes)`) instead of
    // the legacy static-factory shape (`Type.restore_state(bytes)`)
    // when the child lives in an imported file.
    let mut imported_new_contract_names: Vec<String> = Vec::new();
    let mut pending_main_attr_span: Option<crate::frame_c::compiler::frame_ast::Span> = None;
    // Vec (not Option) so multiple occurrences of the same lifecycle
    // pragma — `@@[create(a)]` followed by `@@[create(b)]` — all
    // arrive at the validator and trigger E818 (at most one per
    // system). Option-based capture would silently coalesce.
    let mut pending_create_attrs: Vec<(Option<String>, crate::frame_c::compiler::frame_ast::Span)> =
        Vec::new();
    let mut pending_save_attrs: Vec<(Option<String>, crate::frame_c::compiler::frame_ast::Span)> =
        Vec::new();
    let mut pending_load_attrs: Vec<(Option<String>, crate::frame_c::compiler::frame_ast::Span)> =
        Vec::new();
    // Strip `(arg)` wrapper from the captured pragma value.
    // Returns None for absent / empty / whitespace-only args.
    fn strip_paren_arg(value: &Option<String>) -> Option<String> {
        let raw = value.as_deref()?;
        let trimmed = raw.trim();
        let inner = trimmed.strip_prefix('(')?.strip_suffix(')')?.trim();
        if inner.is_empty() {
            None
        } else {
            Some(inner.to_string())
        }
    }
    for segment in &source_map.segments {
        if let Segment::Pragma {
            kind: crate::frame_c::compiler::segmenter::PragmaKind::Main,
            span,
            ..
        } = segment
        {
            pending_main_attr_span = Some(crate::frame_c::compiler::frame_ast::Span::new(
                span.start, span.end,
            ));
            continue;
        }
        if let Segment::Pragma {
            kind: crate::frame_c::compiler::segmenter::PragmaKind::Create,
            span,
            value,
        } = segment
        {
            pending_create_attrs.push((
                strip_paren_arg(value),
                crate::frame_c::compiler::frame_ast::Span::new(span.start, span.end),
            ));
            continue;
        }
        if let Segment::Pragma {
            kind: crate::frame_c::compiler::segmenter::PragmaKind::Save,
            span,
            value,
        } = segment
        {
            pending_save_attrs.push((
                strip_paren_arg(value),
                crate::frame_c::compiler::frame_ast::Span::new(span.start, span.end),
            ));
            continue;
        }
        if let Segment::Pragma {
            kind: crate::frame_c::compiler::segmenter::PragmaKind::Load,
            span,
            value,
        } = segment
        {
            pending_load_attrs.push((
                strip_paren_arg(value),
                crate::frame_c::compiler::frame_ast::Span::new(span.start, span.end),
            ));
            continue;
        }
        if let Segment::Pragma {
            kind: crate::frame_c::compiler::segmenter::PragmaKind::Import,
            span,
            value,
        } = segment
        {
            // RFC-0022: `@@import "path"` — strip surrounding quotes,
            // store raw path. Phase 1 is lax (no cross-file resolution
            // *errors*); we still do a best-effort peek on the imported
            // file to discover its `@@system` names so per-target hooks
            // can bind a const to the right identifier. Phase 2 strict
            // mode replaces the peek with full parsing + per-symbol
            // resolution.
            if let Some(raw) = value {
                let stripped = raw
                    .trim()
                    .trim_start_matches('"')
                    .trim_end_matches('"')
                    .to_string();
                if !stripped.is_empty() {
                    let peek = match peek_imported_system_names(
                        &stripped,
                        config.source_path.as_deref(),
                    ) {
                        Ok(data) => {
                            if data.names.is_empty() && config.strict_imports {
                                strict_import_errors.push(CompileError::new(
                                    "E822",
                                    &format!(
                                        "@@import \"{}\" — imported file declares no \
                                         `@@system`. With --import-mode strict every \
                                         import must surface at least one system.",
                                        stripped
                                    ),
                                ));
                            }
                            data
                        }
                        Err(msg) => {
                            if config.strict_imports {
                                strict_import_errors.push(CompileError::new(
                                    "E821",
                                    &format!(
                                        "@@import \"{}\" — {}. \
                                         --import-mode strict requires every imported \
                                         file to be readable.",
                                        stripped, msg
                                    ),
                                ));
                            }
                            PeekData::default()
                        }
                    };
                    // Bug #8: imported sub-systems that use the new persist
                    // contract need their names in the cross-system registry
                    // so the parent's restore codegen emits `Type.new()` +
                    // instance `restore_state(...)` instead of the legacy
                    // static-factory call.
                    for n in &peek.new_contract {
                        if !imported_new_contract_names.iter().any(|x| x == n) {
                            imported_new_contract_names.push(n.clone());
                        }
                    }
                    module_imports.push(crate::frame_c::compiler::frame_ast::Import {
                        module: stripped,
                        symbols: peek.names,
                        alias: None,
                        span: crate::frame_c::compiler::frame_ast::Span::new(
                            span.start, span.end,
                        ),
                    });
                }
            }
            continue;
        }
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

            // RFC-0014: attach pending `@@[main]` attribute (if any)
            // to this system. The attribute resets after attachment so
            // it can't bleed onto a later system.
            if let Some(main_span) = pending_main_attr_span.take() {
                system_ast
                    .attributes
                    .push(crate::frame_c::compiler::frame_ast::Attribute {
                        name: "main".to_string(),
                        args: None,
                        span: main_span,
                    });
            }

            // RFC-0015: attach pending lifecycle attributes (`@@[create]`,
            // `@@[save]`, `@@[load]`) to this system. All occurrences
            // are attached so the validator (E818) can detect
            // duplicates. The buffers drain to empty after
            // attachment so a stray pragma followed by native
            // code-then-system can't bleed onto a later system.
            for (arg, span) in pending_create_attrs.drain(..) {
                system_ast
                    .attributes
                    .push(crate::frame_c::compiler::frame_ast::Attribute {
                        name: "create".to_string(),
                        args: arg,
                        span,
                    });
            }
            for (arg, span) in pending_save_attrs.drain(..) {
                system_ast
                    .attributes
                    .push(crate::frame_c::compiler::frame_ast::Attribute {
                        name: "save".to_string(),
                        args: arg,
                        span,
                    });
            }
            for (arg, span) in pending_load_attrs.drain(..) {
                system_ast
                    .attributes
                    .push(crate::frame_c::compiler::frame_ast::Attribute {
                        name: "load".to_string(),
                        args: arg,
                        span,
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

            // RFC-0013 wave 2 phase 2: per-target filter runs LATER
            // (after the validator), so that E800/E801/E802 can fire
            // on attributes whose items would be filtered away.
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
        imports: module_imports.clone(),
        span: AstSpan::new(0, 0),
    });
    let arcanum = build_arcanum_from_frame_ast(&module_ast);

    // RFC-0014 module-level pass: enforce exactly one `@@[main]` in
    // multi-system files (E805 zero, E806 multiple). Runs once per
    // module, before either codegen path forks. Single-system files
    // are exempt.
    {
        let mut module_validator = FrameValidator::new();
        if let Err(errs) = module_validator.validate_module_main_attr(&module_ast) {
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
    }

    // GraphViz target: bypass CodegenNode pipeline, use graph IR → DOT emitter
    if matches!(config.target, TargetLanguage::Graphviz) {
        use crate::frame_c::compiler::graphviz;

        let mut dot_systems: Vec<(String, String)> = Vec::new();

        for system_ast in &mut system_asts {
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
            // RFC-0015 D7: validate `@@SystemName(args)` and `@@!SystemName()`
            // call sites (E820 zero-arg no-initialization, E821 undefined system).
            if let Err(errs) =
                validator.validate_system_instantiations(&frame_ast, source, config.target)
            {
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

            // Filter per `@@[target("X")]` (after validation)
            filter_by_target_attribute(system_ast, config.target);

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

    // RFC-0012 amendment: register which systems use the new persist
    // contract (have `@@[save]` / `@@[load]` ops) so nested-system
    // restore emission can pick the right shape (instance method vs
    // legacy static factory). RFC-0022 / FRAMEC_BUGS.md Issue #8:
    // imported systems that carry the same attributes also belong in
    // the registry — without them, a parent referencing an imported
    // sub-system through `@@[persist]` emits the wrong restore form.
    {
        let mut new_contract: std::collections::HashSet<String> = system_asts
            .iter()
            .filter(|s| s.uses_new_persist_contract())
            .map(|s| s.name.clone())
            .collect();
        for n in &imported_new_contract_names {
            new_contract.insert(n.clone());
        }
        crate::frame_c::compiler::codegen::interface_gen::set_new_contract_systems(new_contract);

        // FRAMEC_BUGS Issue #17: register the full set of local
        // `@@system` names so `nested_uses_new_contract` can
        // distinguish "local legacy system" from "cross-file
        // reference" when a name misses the new-contract set.
        // Cross-file references default to new contract; local
        // legacy references default to the legacy emit.
        let local: std::collections::HashSet<String> = system_asts
            .iter()
            .map(|s| s.name.clone())
            .collect();
        crate::frame_c::compiler::codegen::interface_gen::set_local_systems(local);
    }

    // FRAMEC_BUGS.md Issue #2 hot-fix (pre-RFC-0015): register each
    // system's Domain-kind params (bare `@@system Inner(seed: int)`
    // params) so nested-system restore can extract their values from
    // the child's saved JSON and pass them to the constructor instead
    // of `Inner.new()` with zero args. RFC-0015 supersedes this with
    // a uniform factory-only model.
    {
        use crate::frame_c::compiler::frame_ast::ParamKind;
        let mut map: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for s in &system_asts {
            let domain_params: Vec<(String, String)> = s
                .params
                .iter()
                .filter(|p| p.kind == ParamKind::Domain)
                .map(|p| {
                    let type_str = match &p.param_type {
                        crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
                        _ => String::new(),
                    };
                    (p.name.clone(), type_str)
                })
                .collect();
            if !domain_params.is_empty() {
                map.insert(s.name.clone(), domain_params);
            }
        }
        crate::frame_c::compiler::codegen::interface_gen::set_nested_system_domain_params(map);
    }

    for system_ast in &mut system_asts {
        // Validate with shared arcanum (all sibling systems visible).
        // Validation runs on the *unfiltered* AST so attribute-shape
        // errors (E800/E801/E802) surface even on items that the
        // per-target filter would later prune.
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

        // RFC-0013 wave 2 phase 2: prune items whose `@@[target("X")]`
        // attributes don't match the current target. Runs after all
        // validators so attribute-shape errors fire first.
        filter_by_target_attribute(system_ast, config.target);

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
            let compartment_types = generate_rust_compartment_types(system_ast, Some(&arcanum));
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

    // Stage 7: Assemble final output (native pass-through + system substitution + system instantiations)
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
    // RFC-0014: identify the file's primary system. Multi-system files
    // require exactly one `@@[main]` (already validated by E805/E806);
    // single-system files take their lone system as implicit primary.
    let main_system: Option<String> = if system_asts.len() == 1 {
        Some(system_asts[0].name.clone())
    } else {
        system_asts
            .iter()
            .find(|s| s.is_main())
            .map(|s| s.name.clone())
    };
    // RFC-0022: ask the backend to translate `@@import` directives into
    // its native form. Default impl returns empty (no emission); per-
    // backend overrides translate per target.
    let module_imports_emitted = backend.emit_module_imports(&module_imports);
    // Imported `@@system` names — surfaced by the Phase 1 peek. The
    // assembler accepts these as resolvable targets for `@@SystemName()`
    // call sites in handler bodies and module-scope native code.
    let imported_system_names: Vec<String> = module_imports
        .iter()
        .flat_map(|imp| imp.symbols.iter().cloned())
        .collect();
    let code = match assembler::assemble(
        &source_map,
        &generated_systems,
        &system_params,
        config.target,
        &runtime_imports,
        &module_imports_emitted,
        &imported_system_names,
        main_system.as_deref(),
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

    // RFC-0022 strict-mode errors collected during import resolution
    // surface here. They don't abort earlier passes (the rest of the
    // module still compiles), so the user sees both the missing-import
    // error AND any downstream issues in one shot.
    Ok(CompileResult {
        code: if strict_import_errors.is_empty() {
            code
        } else {
            String::new()
        },
        errors: strict_import_errors,
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
/// Data surfaced by an `@@import` peek.
///
/// `names` — every `@@system <Name>` declaration in source order.
/// `new_contract` — the subset of `names` that carry an
/// `@@[save(...)]` and/or `@@[load(...)]` attribute on the line(s)
/// immediately preceding the system declaration. The persist
/// codegen branches on this to pick the cross-file restore shape
/// (instance method on the new contract, legacy static factory
/// otherwise).
#[derive(Debug, Default, Clone)]
struct PeekData {
    names: Vec<String>,
    new_contract: Vec<String>,
}

/// Outcome of an `@@import` peek.
///
/// `Ok(data)` — the imported file was readable; `data.names` lists the
/// surfaced systems (possibly empty, which strict mode treats as E822 —
/// nothing to import).
///
/// `Err(message)` — the imported file couldn't be read (missing,
/// permission denied, IO error). Lax mode swallows this and treats
/// it as `Ok(PeekData::default())`; strict mode surfaces E821 with
/// this message.
type PeekResult = Result<PeekData, String>;

/// RFC-0022 import peek. Resolve `import_path` relative to the
/// importer's directory (or CWD when unknown), read the file, and pull
/// out every `@@system <Name>` declaration. Returns the discovered
/// system names in source order, or an error if the file is unreadable.
///
/// This is a regex-grade scan, not a full parse — bracket-form
/// attributes, line comments, and multi-line `@@system` blocks
/// surrounding the declaration are all handled by anchoring on the
/// `@@system` keyword and reading the next identifier. Strict mode
/// (RFC-0022 `--import-mode strict`) surfaces unreadable files / empty
/// imports as compile errors; lax mode treats them as empty and lets
/// per-target hooks fall back to filename-derived bindings.
fn peek_imported_system_names(
    import_path: &str,
    importer_path: Option<&std::path::Path>,
) -> PeekResult {
    let import_buf = std::path::PathBuf::from(import_path);
    let resolved = if import_buf.is_absolute() {
        import_buf
    } else if let Some(importer) = importer_path.and_then(|p| p.parent()) {
        importer.join(&import_buf)
    } else {
        import_buf
    };
    let content = match std::fs::read_to_string(&resolved) {
        Ok(s) => s,
        Err(e) => {
            return Err(format!(
                "cannot read imported file '{}' (resolved to {}): {}",
                import_path,
                resolved.display(),
                e
            ));
        }
    };
    let mut names: Vec<String> = Vec::new();
    let mut new_contract: Vec<String> = Vec::new();
    // RFC-0012 amendment: `@@[save(...)]` / `@@[load(...)]` attributes
    // attach to the *next* `@@system` declaration. Track whether either
    // has been seen since the last `@@system` consumption; if so, the
    // next system is registered as new-contract.
    let mut pending_save_or_load = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        // Skip comment lines so commented-out `@@system` declarations
        // don't pollute the peek.
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        // Pre-system attribute detection: a bracket-form pragma that
        // names `save` or `load` flips the pending flag. Match against
        // the trimmed line because attributes may be wrapped in `@@[`.
        if trimmed.starts_with("@@[save") || trimmed.starts_with("@@[load") {
            pending_save_or_load = true;
            continue;
        }
        let rest = match trimmed.strip_prefix("@@system") {
            Some(r) => r,
            None => continue,
        };
        // Require a separator after `@@system` so `@@systemd` (a
        // hypothetical future keyword / typo) doesn't false-match.
        let next = rest.chars().next();
        if !matches!(next, Some(c) if c.is_whitespace()) {
            continue;
        }
        // RFC-0014 visibility marker (`@@system private Name`) sits
        // between the keyword and the name; skip it if present.
        let mut tokens = rest.split_whitespace();
        let first = match tokens.next() {
            Some(t) => t,
            None => continue,
        };
        let name_token = if first == "private" || first == "public" {
            match tokens.next() {
                Some(t) => t,
                None => continue,
            }
        } else {
            first
        };
        // Trim trailing punctuation that can attach to the name token
        // when there's no separating whitespace (e.g. `Counter:` for a
        // base-class declaration, `Counter{` for an inlined body).
        let clean: String = name_token
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if clean.is_empty() {
            continue;
        }
        if !names.iter().any(|n| n == &clean) {
            names.push(clean.clone());
        }
        // If `@@[save]` and/or `@@[load]` preceded this system, the
        // system uses the new persist contract. Record it and reset
        // the flag so attributes on the next system are tracked
        // independently.
        if pending_save_or_load && !new_contract.iter().any(|n| n == &clean) {
            new_contract.push(clean);
        }
        pending_save_or_load = false;
    }
    Ok(PeekData { names, new_contract })
}

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

    system_ast
        .domain
        .retain(|d| should_emit(&d.attributes, current));

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

    /// RFC-0017 regression: a parameterized `@@[persist]` child held by
    /// an orchestrator must (a) be field-initialized via the factory
    /// (`Counter._create(7)`) — never the bare-ctor spelling — and
    /// (b) be rehydrated in the orchestrator's `restore_state` with the
    /// bare no-arg ctor (`Counter.new()`), not `Counter.new(<saved
    /// arg>)` which after init-decoupling overflows the parameterless
    /// `_init()`. GDScript-specific: the other typed backends already
    /// emitted the bare-`new` form in the restore path.
    #[test]
    fn test_gdscript_persist_param_child_call_sites() {
        let source = br#"@@[target("gdscript")]
@@[persist(PackedByteArray)]
@@[save(save_state)]
@@[load(restore_state)]
@@system Counter(seed: int) {
    interface:
        bump()
    machine:
        $S { $> { self.n = self.seed } bump() { self.n = self.n + 1 } }
    domain:
        seed: int = seed
        n: int = 0
}
@@[main]
@@[persist(PackedByteArray)]
@@[save(save_state)]
@@[load(restore_state)]
@@system World {
    interface:
        bump_c()
    machine:
        $S { bump_c() { self.c.bump() } }
    domain:
        c = @@Counter(7)
}
"#;
        let config = PipelineConfig::production(TargetLanguage::GDScript);
        let output = compile_module(source, &config).expect("pipeline error");
        assert!(
            output.errors.is_empty(),
            "compile errors: {:?}",
            output.errors
        );
        let code = &output.code;
        // (a) field initializer uses the factory
        assert!(
            code.contains("Counter._create(7)"),
            "domain-init should use the RFC-0017 factory; got:\n{}",
            code
        );
        // (b) restore_state rehydrates via the bare no-arg ctor
        assert!(
            code.contains("self.c = Counter.new()"),
            "restore_state should rehydrate via bare `Counter.new()`; got:\n{}",
            code
        );
        // and never passes the saved ctor arg to the parameterless ctor
        assert!(
            !code.contains("Counter.new(__raw"),
            "restore_state must not pass saved args to the no-arg bare ctor; got:\n{}",
            code
        );
    }

    /// RFC-0017 regression: a single `@@system Foo : RefCounted` emits at
    /// GDScript script-module scope (no `class Foo:` wrapper — the file
    /// IS Foo). The init-decouple `_create()` body references the script
    /// by name (`Foo.new()`), which has no referent at module scope
    /// without a `class_name` declaration → Godot "Identifier not found:
    /// Foo". The assembler must prepend `class_name Foo` (before
    /// `extends`).
    #[test]
    fn test_gdscript_module_scope_system_has_class_name() {
        let source = br#"@@[target("gdscript")]
@@system Adventure : RefCounted {
    interface:
        bump()
        get_value(): int
    machine:
        $S {
            bump() { self.n = self.n + 1 }
            get_value(): int { @@:(self.n) }
        }
    domain:
        n: int = 0
}
"#;
        let config = PipelineConfig::production(TargetLanguage::GDScript);
        let output = compile_module(source, &config).expect("pipeline error");
        assert!(
            output.errors.is_empty(),
            "compile errors: {:?}",
            output.errors
        );
        let code = &output.code;
        // The module-scope system's `_create` references `Adventure.new()`.
        assert!(
            code.contains("Adventure.new()"),
            "expected the module-scope `_create` to reference `Adventure.new()`; got:\n{}",
            code
        );
        // ...so the script must declare `class_name Adventure` (before
        // `extends`) for that identifier to resolve.
        let class_name_at = code.find("class_name Adventure");
        let extends_at = code.find("extends RefCounted");
        assert!(
            class_name_at.is_some(),
            "module-scope GDScript system must emit `class_name Adventure`; got:\n{}",
            code
        );
        assert!(
            class_name_at < extends_at,
            "`class_name` must precede `extends`; got:\n{}",
            code
        );
    }
}
