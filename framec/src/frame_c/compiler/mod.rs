use crate::frame_c::compiler::validator::ValidationResult;
use crate::frame_c::utils::{frame_exitcode, RunError};
pub use crate::frame_c::visitors::TargetLanguage;

pub mod arcanum;
pub mod ast;
pub mod body_closer;
pub mod frame_statement_parser;
pub mod mir;
pub mod native_region_scanner;
pub mod pragma_scanner;
pub mod prolog_scanner;
pub mod splice;
pub mod validator;

pub mod assembler;
pub mod attribute_scanner;
pub mod codegen;
pub mod frame_ast;
pub mod frame_validator;
pub mod gdscript_multisys;
pub mod graphviz;
pub mod lexer;
pub mod model;
pub mod pipeline;
pub mod pipeline_parser;
pub mod segmenter;

pub use codegen::{generate_system, get_backend, CodegenNode, LanguageBackend};
pub use pipeline::{compile_ast_based, CompileError, CompileMode, CompileResult, PipelineConfig};

#[cfg(test)]
#[cfg(test)]
mod arcanum_tests;
#[cfg(test)]
mod compile_tests;
// future: pub mod import_validator;

/// Main module compiler for `@@target` files.
///
/// All Frame code is processed through the pipeline: parse -> validate -> codegen.
pub fn compile_module(content_str: &str, lang: TargetLanguage) -> Result<String, RunError> {
    use crate::frame_c::compiler::pipeline::compiler;
    use crate::frame_c::compiler::pipeline::config::PipelineConfig;

    // Create config from environment, falling back to production defaults
    let config = PipelineConfig::from_env(lang);

    // AST-based compilation
    match compiler::compile_ast_based(content_str.as_bytes(), &config) {
        Ok(result) if result.errors.is_empty() => {
            // Surface non-fatal warnings (W-prefixed codes) to stderr
            // so the user sees them without polluting stdout, which
            // typically holds the generated code path or the code
            // itself when piped.
            for w in &result.warnings {
                eprintln!("Warning: {}: {}", w.code, w.message);
            }
            Ok(result.code)
        }
        Ok(result) => {
            // Even on a failed build, surface any warnings collected
            // before the error stopped compilation.
            for w in &result.warnings {
                eprintln!("Warning: {}: {}", w.code, w.message);
            }
            // Return validation/compilation errors
            let error_msgs: Vec<String> = result
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.code, e.message))
                .collect();
            Err(RunError::new(
                frame_exitcode::CONFIG_ERR,
                &format!("Compilation failed:\n{}", error_msgs.join("\n")),
            ))
        }
        Err(e) => Err(e),
    }
}

pub fn validate_module(
    content_str: &str,
    lang: TargetLanguage,
) -> Result<ValidationResult, RunError> {
    validate_module_with_mode(content_str, lang, false)
}

pub fn validate_module_with_mode(
    content_str: &str,
    lang: TargetLanguage,
    _strict_native: bool,
) -> Result<ValidationResult, RunError> {
    use crate::frame_c::compiler::pipeline::compile_ast_based;
    use crate::frame_c::compiler::pipeline::config::PipelineConfig;
    use crate::frame_c::compiler::validator::ValidationIssue;

    // Run the full V4 pipeline and extract validation errors.
    // The `_strict_native` flag is unused — native-syntax checks
    // run as part of every backend's codegen stage.
    let config = PipelineConfig::production(lang);
    let result = compile_ast_based(content_str.as_bytes(), &config)?;

    let issues: Vec<ValidationIssue> = result
        .errors
        .iter()
        .map(|e| ValidationIssue {
            message: format!("{}: {}", e.code, e.message),
        })
        .collect();

    Ok(ValidationResult {
        ok: issues.is_empty(),
        issues,
    })
}

// SOL-anchored scan for `system <Ident> {` ignoring common comments
pub fn find_system_name(bytes: &[u8], start: usize) -> Option<String> {
    let n = bytes.len();
    let mut i = start;
    while i < n {
        // skip whitespace
        while i < n
            && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r' || bytes[i] == b'\n')
        {
            i += 1;
        }
        if i >= n {
            break;
        }
        // skip line comments
        if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'#' {
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // skip block comments
        if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < n {
                i += 2;
            }
            continue;
        }

        // Check for @@system
        if i + 8 < n && &bytes[i..i + 8] == b"@@system" {
            i += 8;
            // skip whitespace
            while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            // read system name
            let name_start = i;
            while i < n && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i > name_start {
                return Some(String::from_utf8_lossy(&bytes[name_start..i]).to_string());
            }
        }

        // read ident (for compatibility)
        let mut j = i;
        while j < n && ((bytes[j] as char).is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        if j > i {
            let kw = String::from_utf8_lossy(&bytes[i..j]).to_ascii_lowercase();
            if kw == "system" {
                while j < n && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                let name_start = j;
                while j < n && ((bytes[j] as char).is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                if j > name_start {
                    return Some(String::from_utf8_lossy(&bytes[name_start..j]).to_string());
                }
            }
            // Continue scanning the rest of the line so we can catch `system`
            // after leading annotations (e.g., `@@persist @@system Foo {`).
            i = j;
            continue;
        }
        // Non-identifier character: advance one byte and keep scanning
        i += 1;
    }
    None
}

// Compiler interface
pub struct FrameCompiler {
    target: TargetLanguage,
}

impl FrameCompiler {
    pub fn new(target: TargetLanguage) -> Self {
        Self { target }
    }

    /// Compile to semantic JSON model (--format model).
    /// Runs segmenter + parser but skips codegen.
    pub fn compile_to_model(
        &self,
        source: &str,
        _file_path: &str,
        target_ext: &str,
    ) -> Result<String, String> {
        use crate::frame_c::compiler::model::{build_system_model, emit_model_json};
        use crate::frame_c::compiler::pipeline::compiler;
        use crate::frame_c::compiler::pipeline::config::PipelineConfig;

        let config = PipelineConfig::from_env(self.target);
        let bytes = source.as_bytes();

        // Stage 0: Segment
        let source_map = match segmenter::segment_source(bytes, config.target) {
            Ok(sm) => sm,
            Err(e) => return Err(format!("Segmentation error: {}", e)),
        };

        // Pass 1: Parse all systems into ASTs
        let mut models = Vec::new();
        for segment in &source_map.segments {
            if let segmenter::Segment::System {
                name, body_span, ..
            } = segment
            {
                let ast_span = frame_ast::Span::new(body_span.start, body_span.end);
                let system_ast = match pipeline_parser::parse_system(
                    &source_map.source,
                    name.clone(),
                    ast_span,
                    config.target,
                ) {
                    Ok(ast) => ast,
                    Err(e) => return Err(format!("Parse error in system '{}': {}", name, e)),
                };
                models.push(build_system_model(&system_ast, target_ext, bytes));
            }
        }

        Ok(emit_model_json(models))
    }

    pub fn compile(&self, source: &str, _file_path: &str) -> FrameResult {
        match compile_module(source, self.target) {
            Ok(code) => FrameResult::Ok(FrameOutput {
                code,
                warnings: Vec::new(),
                source_map: None,
            }),
            Err(e) => FrameResult::Err(ErrorsAcc {
                errors: vec![e.error],
            }),
        }
    }
}

// Result types
pub enum FrameResult {
    Ok(FrameOutput),
    Err(ErrorsAcc),
}

pub struct FrameOutput {
    pub code: String,
    pub warnings: Vec<String>,
    pub source_map: Option<String>,
}

pub struct ErrorsAcc {
    pub errors: Vec<String>,
}

impl ErrorsAcc {
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    pub fn push_error(&mut self, error: String) {
        self.errors.push(error);
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }
}
