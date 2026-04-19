//! Output Assembler (Stage 7 of the V4 Pipeline)
//!
//! Takes the `SourceMap` from the Segmenter (Stage 0) and generated code from
//! the Codegen/Emit stages (Stages 5-6), and produces the final output file.
//!
//! Algorithm:
//! 1. Walk `SourceMap.segments` in order:
//!    - `Segment::Native` → extract text from source bytes at span, append to output
//!    - `Segment::Pragma` → skip (consumed by earlier stages)
//!    - `Segment::System` → look up system name in generated_systems, append generated code
//! 2. Post-process: expand `@@SystemName()` tagged instantiations in native regions
//! 3. Return final assembled output

use crate::frame_c::compiler::frame_ast::SystemParam;
use crate::frame_c::compiler::native_region_scanner::create_skipper;
use crate::frame_c::compiler::native_region_scanner::unified::SyntaxSkipper;
use crate::frame_c::compiler::pipeline_parser::call_args::{
    parse_call_args, resolve_call, CallArgsError,
};
use crate::frame_c::compiler::segmenter::{Segment, SourceMap};
use crate::frame_c::visitors::TargetLanguage;
use std::collections::HashMap;
use std::collections::HashSet;

// ============================================================================
// Assembly Error
// ============================================================================

#[derive(Debug, Clone)]
pub struct AssemblyError {
    pub message: String,
}

impl std::fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Assembly error: {}", self.message)
    }
}

impl std::error::Error for AssemblyError {}

// ============================================================================
// Public API
// ============================================================================

/// Assemble the final output from source map and generated system code.
///
/// `source_map` — the segmented source from Stage 0
/// `generated_systems` — Vec of (system_name, generated_code) from Stages 5-6
/// `system_params` — Vec of (system_name, declared params) so the assembler
///   can resolve `@@SystemName(args)` call sites against the declared shape
///   (sigil checks, named lookup, default substitution).
/// `lang` — target language for tagged instantiation expansion
/// `runtime_imports` — imports required by generated code (emitted before any native code)
pub fn assemble(
    source_map: &SourceMap,
    generated_systems: &[(String, String)],
    system_params: &[(String, Vec<SystemParam>)],
    lang: TargetLanguage,
    runtime_imports: &[String],
) -> Result<String, AssemblyError> {
    let source = &source_map.source;
    let mut output = String::new();

    // Emit runtime imports first (before any native prolog code)
    // This ensures imports like "from typing import ..." come before user code
    for import in runtime_imports {
        output.push_str(import);
        output.push('\n');
    }
    if !runtime_imports.is_empty() {
        output.push('\n');
    }

    // Build lookup for generated systems
    let system_code: HashMap<&str, &str> = generated_systems
        .iter()
        .map(|(name, code)| (name.as_str(), code.as_str()))
        .collect();

    let defined_system_names: HashSet<String> = generated_systems
        .iter()
        .map(|(name, _)| name.clone())
        .collect();

    // Build name → declared-params lookup for call-site resolution
    let params_by_name: HashMap<&str, &[SystemParam]> = system_params
        .iter()
        .map(|(name, params)| (name.as_str(), params.as_slice()))
        .collect();

    // Walk segments in order
    for segment in &source_map.segments {
        match segment {
            Segment::Native { span } => {
                // Extract native text from source bytes
                let text = extract_text(source, span.start, span.end);
                // Expand tagged instantiations (@@SystemName(args)) in native code
                let expanded = expand_tagged_instantiations(
                    &text,
                    &defined_system_names,
                    &params_by_name,
                    lang,
                )?;
                output.push_str(&expanded);
            }

            Segment::Pragma { .. } => {
                // Pragmas are consumed by earlier stages — skip them
                // (they don't appear in the output)
            }

            Segment::System { name, .. } => {
                // Look up generated code for this system
                if let Some(code) = system_code.get(name.as_str()) {
                    output.push_str(code);
                } else {
                    return Err(AssemblyError {
                        message: format!(
                            "No generated code for system '{}'. Available: {:?}",
                            name,
                            system_code.keys().collect::<Vec<_>>()
                        ),
                    });
                }
            }
        }
    }

    Ok(output)
}

// ============================================================================
// Internal: Text Extraction
// ============================================================================

/// Extract text from source bytes at the given byte range.
fn extract_text(source: &[u8], start: usize, end: usize) -> String {
    let end = end.min(source.len());
    let start = start.min(end);
    String::from_utf8_lossy(&source[start..end]).into_owned()
}

/// Render a `CallArgsError` as a human-readable assembly diagnostic.
fn format_call_args_error(err: &CallArgsError) -> String {
    match err {
        CallArgsError::ParseError { message, position } => {
            format!("parse error at {}: {}", position, message)
        }
        CallArgsError::MixedForms { message } => message.clone(),
        CallArgsError::SigilsRequired { message } => message.clone(),
        CallArgsError::PositionalMismatch { message } => message.clone(),
        CallArgsError::UnknownNamedArg { name } => {
            format!("unknown named argument '{}'", name)
        }
        CallArgsError::MissingArg { name } => {
            format!(
                "required parameter '{}' has no argument and no default",
                name
            )
        }
        CallArgsError::ExtraArgs { count } => {
            format!("{} extra argument(s) supplied", count)
        }
        CallArgsError::DuplicateNamedArg { name } => {
            format!("duplicate named argument '{}'", name)
        }
    }
}

// ============================================================================
// Internal: Tagged Instantiation Expansion
// ============================================================================

/// Expand `@@SystemName(args)` tagged instantiations in native code.
///
/// In native code regions, users write `@@SystemName(args)` which gets expanded
/// to the appropriate constructor syntax for the target language:
/// - Python: `SystemName(args)`
/// - TypeScript: `new SystemName(args)`
/// - Rust: `SystemName::new(args)`
/// - C: `SystemName_new(args)`
/// - C++/Java/C#: `new SystemName(args)`
fn expand_tagged_instantiations(
    text: &str,
    defined_systems: &HashSet<String>,
    params_by_name: &HashMap<&str, &[SystemParam]>,
    lang: TargetLanguage,
) -> Result<String, AssemblyError> {
    let bytes = text.as_bytes();
    let end = bytes.len();
    let mut result = String::new();
    let mut i = 0;

    // Use the language's SyntaxSkipper for comment/string detection —
    // no duplicated logic, proper handling of triple-quotes, raw strings, etc.
    let skipper = create_skipper(lang);

    while i < end {
        // Delegate comment skipping to the language's SyntaxSkipper
        if let Some(after) = skipper.skip_comment(bytes, i, end) {
            result.push_str(&String::from_utf8_lossy(&bytes[i..after]));
            i = after;
            continue;
        }

        // Delegate string skipping to the language's SyntaxSkipper
        if let Some(after) = skipper.skip_string(bytes, i, end) {
            result.push_str(&String::from_utf8_lossy(&bytes[i..after]));
            i = after;
            continue;
        }

        // Look for @@ pattern
        if i + 2 < end && bytes[i] == b'@' && bytes[i + 1] == b'@' {
            let start = i;
            i += 2;

            // Check for uppercase letter (system name start)
            if i < end && bytes[i].is_ascii_uppercase() {
                let name_start = i;
                while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let name = std::str::from_utf8(&bytes[name_start..i]).unwrap_or("");

                // Use SyntaxSkipper's balanced_paren_end for argument extraction
                if i < end && bytes[i] == b'(' {
                    if let Some(close) = skipper.balanced_paren_end(bytes, i, end) {
                        let args_text = std::str::from_utf8(&bytes[i + 1..close - 1]).unwrap_or("");

                        if defined_systems.contains(name) {
                            let resolved_args = match params_by_name.get(name) {
                                Some(params) if !params.is_empty() => {
                                    let parsed =
                                        parse_call_args(args_text).map_err(|e| AssemblyError {
                                            message: format!(
                                                "@@{}({}): {}",
                                                name,
                                                args_text,
                                                format_call_args_error(&e)
                                            ),
                                        })?;
                                    let values = resolve_call(&parsed, params).map_err(|e| {
                                        AssemblyError {
                                            message: format!(
                                                "@@{}({}): {}",
                                                name,
                                                args_text,
                                                format_call_args_error(&e)
                                            ),
                                        }
                                    })?;
                                    values.join(", ")
                                }
                                _ => args_text.to_string(),
                            };
                            let constructor = generate_constructor(name, &resolved_args, lang);
                            result.push_str(&constructor);
                            i = close;
                            continue;
                        } else {
                            return Err(AssemblyError {
                                message: format!(
                                    "Undefined system '{}' in tagged instantiation. Available: {:?}",
                                    name, defined_systems
                                ),
                            });
                        }
                    }
                }
            }

            // Not a valid tagged instantiation — copy original @@ chars
            for b in &bytes[start..i] {
                result.push(*b as char);
            }
            continue;
        }

        // Regular character — copy through
        result.push(bytes[i] as char);
        i += 1;
    }

    Ok(result)
}

/// Generate the language-appropriate constructor call for a system.
fn generate_constructor(name: &str, args: &str, lang: TargetLanguage) -> String {
    match lang {
        TargetLanguage::Python3 => {
            format!("{}({})", name, args)
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            format!("new {}({})", name, args)
        }
        TargetLanguage::Rust => {
            if args.trim().is_empty() {
                format!("{}::new()", name)
            } else {
                format!("{}::new({})", name, args)
            }
        }
        TargetLanguage::C => {
            if args.trim().is_empty() {
                format!("{}_new()", name)
            } else {
                format!("{}_new({})", name, args)
            }
        }
        TargetLanguage::Java | TargetLanguage::CSharp | TargetLanguage::Php => {
            format!("new {}({})", name, args)
        }
        TargetLanguage::Kotlin => {
            // Kotlin: no new keyword
            format!("{}({})", name, args)
        }
        TargetLanguage::Cpp => {
            // C++: stack allocation (classes use unique_ptr internally)
            format!("{}({})", name, args)
        }
        TargetLanguage::Go => {
            // Go: @@System() becomes NewSystem()
            if args.trim().is_empty() {
                format!("New{}()", name)
            } else {
                format!("New{}({})", name, args)
            }
        }
        TargetLanguage::Ruby => {
            // Ruby: @@System() becomes System.new()
            if args.trim().is_empty() {
                format!("{}.new", name)
            } else {
                format!("{}.new({})", name, args)
            }
        }
        TargetLanguage::Swift => {
            // Swift: no new keyword
            format!("{}({})", name, args)
        }
        TargetLanguage::Erlang => {
            // Erlang: module:start_link()
            // Use snake_case for Erlang module names (CamelCase -> snake_case)
            let module_name = {
                let mut result = String::new();
                for (i, c) in name.chars().enumerate() {
                    if c.is_uppercase() && i > 0 {
                        result.push('_');
                    }
                    if let Some(lc) = c.to_lowercase().next() {
                        result.push(lc);
                    }
                }
                result
            };
            if args.trim().is_empty() {
                format!("{}:start_link()", module_name)
            } else {
                format!("{}:start_link({})", module_name, args)
            }
        }
        TargetLanguage::Lua => {
            // Lua: System.new()
            if args.trim().is_empty() {
                format!("{}.new()", name)
            } else {
                format!("{}.new({})", name, args)
            }
        }
        TargetLanguage::Dart => {
            // Dart: no new keyword
            format!("{}({})", name, args)
        }
        TargetLanguage::GDScript => {
            // GDScript: ClassName.new()
            if args.trim().is_empty() {
                format!("{}.new()", name)
            } else {
                format!("{}.new({})", name, args)
            }
        }
        // Non-V4 targets should never reach the assembler.
        // No _ => arm: compiler enforces new TargetLanguage variants are added here.
        TargetLanguage::Graphviz => {
            unreachable!("Assembler called for non-V4 target {:?}", lang)
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::segmenter::Span;

    /// Helper: make a SourceMap manually for testing
    fn make_source_map(source: &str, segments: Vec<Segment>) -> SourceMap {
        SourceMap {
            segments,
            source: source.as_bytes().to_vec(),
            target: Some(TargetLanguage::Python3),
        }
    }

    #[test]
    fn test_native_only() {
        let src = "import math\nprint('hello')\n";
        let map = make_source_map(
            src,
            vec![Segment::Native {
                span: Span {
                    start: 0,
                    end: src.len(),
                },
            }],
        );
        let result = assemble(&map, &[], &[], TargetLanguage::Python3, &[]).unwrap();
        assert_eq!(result, src);
    }

    #[test]
    fn test_system_replacement() {
        let src = "prolog\n@@system Foo {\n  machine:\n    $A { }\n}\nepilogue\n";
        let prolog_end = 7;
        let system_start = 7;
        let system_end = 46;
        let epilog_start = 46;
        let map = make_source_map(
            src,
            vec![
                Segment::Native {
                    span: Span {
                        start: 0,
                        end: prolog_end,
                    },
                },
                Segment::System {
                    outer_span: Span {
                        start: system_start,
                        end: system_end,
                    },
                    body_span: Span {
                        start: system_start + 16,
                        end: system_end - 1,
                    },
                    header_params_span: None,
                    name: "Foo".to_string(),
                    bases: vec![],
                    visibility: None,
                },
                Segment::Native {
                    span: Span {
                        start: epilog_start,
                        end: src.len(),
                    },
                },
            ],
        );
        let generated = vec![("Foo".to_string(), "class Foo:\n  pass\n".to_string())];
        let result = assemble(&map, &generated, &[], TargetLanguage::Python3, &[]).unwrap();
        assert_eq!(result, "prolog\nclass Foo:\n  pass\nepilogue\n");
    }

    #[test]
    fn test_pragma_skipped() {
        let src = "@@target python_3\nimport os\n";
        let map = make_source_map(
            src,
            vec![
                Segment::Pragma {
                    kind: crate::frame_c::compiler::segmenter::PragmaKind::Target,
                    span: Span { start: 0, end: 18 },
                    value: Some("python_3".to_string()),
                },
                Segment::Native {
                    span: Span {
                        start: 18,
                        end: src.len(),
                    },
                },
            ],
        );
        let result = assemble(&map, &[], &[], TargetLanguage::Python3, &[]).unwrap();
        assert_eq!(result, "import os\n");
    }

    /// Helper that builds an empty params lookup. Most of these tests
    /// exercise zero-arg or raw passthrough flows where the system has no
    /// declared params and the assembler is expected to forward the
    /// args text unchanged.
    fn empty_params() -> HashMap<&'static str, &'static [SystemParam]> {
        HashMap::new()
    }

    #[test]
    fn test_tagged_instantiation_python() {
        let src = "s = @@Foo()\n";
        let systems: HashSet<String> = vec!["Foo".to_string()].into_iter().collect();
        let params = empty_params();
        let result =
            expand_tagged_instantiations(src, &systems, &params, TargetLanguage::Python3).unwrap();
        assert_eq!(result, "s = Foo()\n");
    }

    #[test]
    fn test_tagged_instantiation_typescript() {
        let src = "let s = @@Foo()\n";
        let systems: HashSet<String> = vec!["Foo".to_string()].into_iter().collect();
        let params = empty_params();
        let result =
            expand_tagged_instantiations(src, &systems, &params, TargetLanguage::TypeScript)
                .unwrap();
        assert_eq!(result, "let s = new Foo()\n");
    }

    #[test]
    fn test_tagged_instantiation_rust() {
        let src = "let s = @@Foo();\n";
        let systems: HashSet<String> = vec!["Foo".to_string()].into_iter().collect();
        let params = empty_params();
        let result =
            expand_tagged_instantiations(src, &systems, &params, TargetLanguage::Rust).unwrap();
        assert_eq!(result, "let s = Foo::new();\n");
    }

    #[test]
    fn test_tagged_instantiation_c() {
        let src = "struct Foo* s = @@Foo();\n";
        let systems: HashSet<String> = vec!["Foo".to_string()].into_iter().collect();
        let params = empty_params();
        let result =
            expand_tagged_instantiations(src, &systems, &params, TargetLanguage::C).unwrap();
        assert_eq!(result, "struct Foo* s = Foo_new();\n");
    }

    #[test]
    fn test_tagged_instantiation_with_args() {
        let src = "s = @@Foo(1, \"hello\")\n";
        let systems: HashSet<String> = vec!["Foo".to_string()].into_iter().collect();
        let params = empty_params();
        let result =
            expand_tagged_instantiations(src, &systems, &params, TargetLanguage::Python3).unwrap();
        assert_eq!(result, "s = Foo(1, \"hello\")\n");
    }

    #[test]
    fn test_tagged_instantiation_in_comment_not_expanded() {
        let src = "# s = @@Foo()\n";
        let systems: HashSet<String> = vec!["Foo".to_string()].into_iter().collect();
        let params = empty_params();
        let result =
            expand_tagged_instantiations(src, &systems, &params, TargetLanguage::Python3).unwrap();
        assert_eq!(result, "# s = @@Foo()\n");
    }

    #[test]
    fn test_tagged_instantiation_in_string_not_expanded() {
        let src = "s = \"@@Foo()\"\n";
        let systems: HashSet<String> = vec!["Foo".to_string()].into_iter().collect();
        let params = empty_params();
        let result =
            expand_tagged_instantiations(src, &systems, &params, TargetLanguage::Python3).unwrap();
        assert_eq!(result, "s = \"@@Foo()\"\n");
    }

    #[test]
    fn test_multiple_systems() {
        // source with prolog, two systems, interstitial native, epilog
        let src = "prolog\n__SYS1__\nnative_between\n__SYS2__\nepilogue\n";
        let s1_start = 7;
        let s1_end = 16;
        let between_start = 16;
        let between_end = 31;
        let s2_start = 31;
        let s2_end = 40;
        let epilog_start = 40;

        let map = make_source_map(
            src,
            vec![
                Segment::Native {
                    span: Span {
                        start: 0,
                        end: s1_start,
                    },
                },
                Segment::System {
                    outer_span: Span {
                        start: s1_start,
                        end: s1_end,
                    },
                    body_span: Span {
                        start: s1_start + 2,
                        end: s1_end - 2,
                    },
                    header_params_span: None,
                    name: "Alpha".to_string(),
                    bases: vec![],
                    visibility: None,
                },
                Segment::Native {
                    span: Span {
                        start: between_start,
                        end: between_end,
                    },
                },
                Segment::System {
                    outer_span: Span {
                        start: s2_start,
                        end: s2_end,
                    },
                    body_span: Span {
                        start: s2_start + 2,
                        end: s2_end - 2,
                    },
                    header_params_span: None,
                    name: "Beta".to_string(),
                    bases: vec![],
                    visibility: None,
                },
                Segment::Native {
                    span: Span {
                        start: epilog_start,
                        end: src.len(),
                    },
                },
            ],
        );

        let generated = vec![
            ("Alpha".to_string(), "class Alpha: pass\n".to_string()),
            ("Beta".to_string(), "class Beta: pass\n".to_string()),
        ];
        let result = assemble(&map, &generated, &[], TargetLanguage::Python3, &[]).unwrap();
        assert!(result.contains("prolog\n"));
        assert!(result.contains("class Alpha: pass\n"));
        assert!(result.contains("\nnative_between\n"));
        assert!(result.contains("class Beta: pass\n"));
        assert!(result.contains("epilogue\n"));
    }

    #[test]
    fn test_missing_system_code_errors() {
        let src = "@@system Foo { }";
        let map = make_source_map(
            src,
            vec![Segment::System {
                outer_span: Span {
                    start: 0,
                    end: src.len(),
                },
                body_span: Span { start: 14, end: 15 },
                header_params_span: None,
                name: "Foo".to_string(),
                bases: vec![],
                visibility: None,
            }],
        );
        let result = assemble(&map, &[], &[], TargetLanguage::Python3, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Foo"));
    }

    #[test]
    fn test_undefined_tagged_instantiation_errors() {
        let src = "s = @@Unknown()\n";
        let systems: HashSet<String> = HashSet::new();
        let params: HashMap<&str, &[SystemParam]> = HashMap::new();
        let result = expand_tagged_instantiations(src, &systems, &params, TargetLanguage::Python3);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_assembly_with_tagged_instantiation() {
        // prolog creates an instance using tagged instantiation
        let src_native = "s = @@MySystem()\n";
        let src_system = "@@system MySystem { machine: $A { } }";
        let full_src = format!("{}{}", src_native, src_system);
        let native_end = src_native.len();

        let map = make_source_map(
            &full_src,
            vec![
                Segment::Native {
                    span: Span {
                        start: 0,
                        end: native_end,
                    },
                },
                Segment::System {
                    outer_span: Span {
                        start: native_end,
                        end: full_src.len(),
                    },
                    body_span: Span {
                        start: native_end + 20,
                        end: full_src.len() - 1,
                    },
                    header_params_span: None,
                    name: "MySystem".to_string(),
                    bases: vec![],
                    visibility: None,
                },
            ],
        );

        let generated = vec![(
            "MySystem".to_string(),
            "class MySystem:\n  pass\n".to_string(),
        )];
        let result = assemble(&map, &generated, &[], TargetLanguage::Python3, &[]).unwrap();
        assert_eq!(result, "s = MySystem()\nclass MySystem:\n  pass\n");
    }

    #[test]
    fn test_runtime_imports_before_prolog() {
        // Test that runtime imports are emitted before native prolog code
        let src = "import json\n@@system Foo { machine: $A { } }";
        let prolog_end = 12;
        let map = make_source_map(
            src,
            vec![
                Segment::Native {
                    span: Span {
                        start: 0,
                        end: prolog_end,
                    },
                },
                Segment::System {
                    outer_span: Span {
                        start: prolog_end,
                        end: src.len(),
                    },
                    body_span: Span {
                        start: prolog_end + 16,
                        end: src.len() - 1,
                    },
                    header_params_span: None,
                    name: "Foo".to_string(),
                    bases: vec![],
                    visibility: None,
                },
            ],
        );
        let generated = vec![("Foo".to_string(), "class Foo:\n  pass\n".to_string())];
        let runtime_imports = vec!["from typing import Any".to_string()];
        let result = assemble(
            &map,
            &generated,
            &[],
            TargetLanguage::Python3,
            &runtime_imports,
        )
        .unwrap();
        // Runtime imports should come first, then the native prolog, then system
        assert!(result.starts_with("from typing import Any\n"));
        assert!(result.contains("\nimport json\n"));
        assert!(result.contains("class Foo:"));
    }
}
