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
/// `main_system` — RFC-0014 primary system. For multi-system GDScript files this
///   is the system whose code emits at script-module scope; every other system
///   wraps as an inner class. None for single-system files (no special handling
///   needed) or for non-GDScript targets where the attribute is metadata-only.
pub fn assemble(
    source_map: &SourceMap,
    generated_systems: &[(String, String)],
    system_params: &[(String, Vec<SystemParam>)],
    lang: TargetLanguage,
    runtime_imports: &[String],
    main_system: Option<&str>,
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

    // GDScript multi-system: the system whose name matches
    // `main_system` (RFC-0014's `@@[main]`) emits at script-module
    // scope; every other system wraps as a sibling inner class.
    // `main_system` is None for single-system files (no special
    // handling needed — the lone system is implicitly primary) and
    // for non-GDScript targets.
    //
    // GDScript additionally requires the script-level `extends Base`
    // directive to appear before any other declaration (after
    // optional `class_name`). The main system's per-system emission
    // begins with its `extends Base` line, but in source order the
    // main system is typically NOT first (frame-arcade convention:
    // primitives first, composer last). We hoist the main system's
    // `extends` line to the top of the file and strip it from the
    // main system's emission during the walk so the script parses
    // cleanly.
    let main_extends_line: Option<String> =
        if matches!(lang, TargetLanguage::GDScript) {
            main_system.and_then(|m| {
                generated_systems
                    .iter()
                    .find(|(name, _)| name == m)
                    .and_then(|(_, code)| extract_leading_extends_line(code))
            })
        } else {
            None
        };
    if let Some(ref ext_line) = main_extends_line {
        output.push_str(ext_line);
        output.push_str("\n\n");
    }

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
                    // Codegen passes handler-body native regions through
                    // verbatim (Oceans Model). If a handler body contains
                    // `@@OtherSystem($(arg))` that sigil form isn't
                    // expanded by codegen — the handler-body path only
                    // sees the surrounding text as a NativeBlock and
                    // doesn't run the tagged-instantiation expansion.
                    // Re-running the same expansion over the emitted
                    // system code catches those leaks. Idempotent: if
                    // codegen already stripped every `@@Name(...)`, this
                    // pass finds nothing to rewrite.
                    let expanded = expand_tagged_instantiations(
                        code,
                        &defined_system_names,
                        &params_by_name,
                        lang,
                    )?;
                    // GDScript multi-system: every system after the first
                    // must be wrapped as an inner class. GDScript files
                    // accept at most one script-level `extends` directive
                    // and one set of script-level `var`/`func` declarations.
                    // The first system stays as-is at script scope; systems
                    // 2..N strip their leading `extends Base` line and wrap
                    // the rest as `class <Name> extends Base:` with every
                    // subsequent line indented one level.
                    if matches!(lang, TargetLanguage::GDScript) {
                        let is_main = main_system
                            .map(|m| m == name.as_str())
                            .unwrap_or(false);
                        // When the main system's `extends Base` line
                        // was hoisted to the top of the file, strip
                        // it from the in-line emission to avoid a
                        // duplicate `extends`.
                        let rewritten = rewrite_gdscript_per_system(
                            name,
                            &expanded,
                            is_main,
                            is_main && main_extends_line.is_some(),
                        );
                        output.push_str(&rewritten);
                    } else {
                        output.push_str(&expanded);
                    }
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

    // Erlang attribute hoist. Frame source typically has a native
    // prolog with helper functions BEFORE the `@@system` block:
    //
    //   helper() -> ok.            <-- native, emitted first
    //
    //   @@system Foo { ... }       <-- system code:
    //                                  -module(foo). -behaviour(gen_statem).
    //                                  -export([...]). callbacks() ...
    //
    // Erlang requires `-module`/`-behaviour`/`-export` to precede ANY
    // function definition in the source file — otherwise erlc rejects
    // with "no module definition" + "attribute X after function
    // definitions". Walk the assembled output and pull every leading
    // `-` attribute line up to the top of the file, preserving their
    // relative order. Other lines (comments, helper functions,
    // generated callbacks) keep their original sequence in the
    // remainder.
    if lang == TargetLanguage::Erlang {
        // Multi-line attributes (e.g., `-record(data, { ... }).`) are
        // detected by an opening `-attr(` line whose closing `).` is
        // on a later line — track paren depth across lines and
        // keep the whole block together.
        let lines: Vec<&str> = output.lines().collect();
        let mut attrs: Vec<&str> = Vec::new();
        let mut other: Vec<&str> = Vec::new();
        let mut idx = 0;
        while idx < lines.len() {
            let line = lines[idx];
            let t = line.trim_start();
            let is_attr_start = t.starts_with("-module(")
                || t.starts_with("-behaviour(")
                || t.starts_with("-behavior(")
                || t.starts_with("-export(")
                || t.starts_with("-record(")
                || t.starts_with("-define(")
                || t.starts_with("-include(")
                || t.starts_with("-include_lib(");
            if !is_attr_start {
                other.push(line);
                idx += 1;
                continue;
            }
            // Collect this line and any continuation lines until paren
            // depth reaches zero AND we've seen the terminating `).`.
            let mut depth: i32 = 0;
            let mut closed = false;
            let start = idx;
            while idx < lines.len() {
                let l = lines[idx];
                for c in l.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
                idx += 1;
                if depth <= 0 && l.trim_end().ends_with(").") {
                    closed = true;
                    break;
                }
            }
            for l in &lines[start..idx] {
                attrs.push(l);
            }
            // If the block didn't close cleanly (defensive — shouldn't
            // happen in well-formed Erlang), the bytes end up in
            // `attrs` and we move on.
            let _ = closed;
        }
        if !attrs.is_empty() {
            let mut hoisted = String::new();
            for a in &attrs {
                hoisted.push_str(a);
                hoisted.push('\n');
            }
            hoisted.push('\n');
            for o in &other {
                hoisted.push_str(o);
                hoisted.push('\n');
            }
            output = hoisted;
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

/// Did the GDScript codegen emit this system at script-module scope?
///
/// Module-scope emission starts with a leading `extends <Base>` line
/// (after any blank lines) and lays the system's fields/methods flat
/// at indent 0. Systems without a declared base are wrapped by codegen
/// as `class <Name>:` inner classes from the start — those don't need
/// the assembler-level wrap (it would double-nest the class).
fn per_system_emits_at_module_scope(code: &str) -> bool {
    code.lines()
        .map(|l| l.trim_end())
        .find(|l| !l.is_empty())
        .map(|l| l.starts_with("extends "))
        .unwrap_or(false)
}

/// Rewrite a single per-system GDScript emission according to whether
/// it's the file's `@@[main]` system (RFC-0014):
///
/// * **Main system** (`is_main == true`): pass through unchanged.
///   Its per-system codegen owns the script-level `extends` directive
///   and any `var` / `func` declarations. From here, references to
///   non-main systems resolve as `Inner.new()` — sibling inner
///   classes are visible from the script's own `_init` and method
///   bodies.
///
/// * **Non-main systems** whose codegen emitted at script-module
///   scope (leading `extends <Base>`): wrap as `class <name> extends
///   <Base>:` with the body indented one level. Sibling inner classes
///   in GDScript can reference each other by bare name.
///
/// * **Non-main systems** whose codegen already produced inner-class
///   form (`class <name>:`, no declared base): pass through unchanged.
///   Wrapping again would double-nest the class.
///
/// All wrapping/indenting work delegates to the Frame state machine
/// in `compiler/gdscript_multisys/multisys_assembler.frs`.
fn rewrite_gdscript_per_system(
    name: &str,
    code: &str,
    is_main: bool,
    strip_leading_extends: bool,
) -> String {
    use crate::frame_c::compiler::gdscript_multisys;

    if is_main {
        if strip_leading_extends {
            strip_leading_extends_line(code)
        } else {
            code.to_string()
        }
    } else if per_system_emits_at_module_scope(code) {
        gdscript_multisys::wrap_inner(name, code)
    } else {
        code.to_string()
    }
}

/// Find the leading `extends <Base>` line in a per-system GDScript
/// emission and return it (without the trailing newline). Used by the
/// main-system hoist so the script-level `extends` directive lands at
/// the very top of the file, before any inner-class declarations from
/// non-main systems.
///
/// Returns None when the system's emission doesn't begin with an
/// `extends` directive — typically because codegen wrapped it as
/// inner-class form (no declared base in source). In that case the
/// hoist is a no-op.
fn extract_leading_extends_line(code: &str) -> Option<String> {
    for line in code.lines() {
        let t = line.trim_start();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix("extends ") {
            return Some(format!("extends {}", rest.trim_end()));
        }
        return None;
    }
    None
}

/// Drop the first `extends <Base>` line (and any blank lines
/// immediately preceding it) from a per-system emission. Mirrors the
/// inverse of `extract_leading_extends_line`.
fn strip_leading_extends_line(code: &str) -> String {
    let mut lines = code.lines().peekable();
    let mut leading_blanks: Vec<&str> = Vec::new();
    while let Some(&l) = lines.peek() {
        if l.trim().is_empty() {
            leading_blanks.push(l);
            lines.next();
        } else {
            break;
        }
    }
    let stripped = match lines.peek() {
        Some(l) if l.trim_start().starts_with("extends ") => {
            lines.next();
            true
        }
        _ => false,
    };
    if !stripped {
        return code.to_string();
    }
    let mut out = String::with_capacity(code.len());
    for l in leading_blanks {
        out.push_str(l);
        out.push('\n');
    }
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    out
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
        let result = assemble(&map, &[], &[], TargetLanguage::Python3, &[], None).unwrap();
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
        let result = assemble(&map, &generated, &[], TargetLanguage::Python3, &[], None).unwrap();
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
        let result = assemble(&map, &[], &[], TargetLanguage::Python3, &[], None).unwrap();
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
        let result = assemble(&map, &generated, &[], TargetLanguage::Python3, &[], None).unwrap();
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
        let result = assemble(&map, &[], &[], TargetLanguage::Python3, &[], None);
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
        let result = assemble(&map, &generated, &[], TargetLanguage::Python3, &[], None).unwrap();
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
            None,
        )
        .unwrap();
        // Runtime imports should come first, then the native prolog, then system
        assert!(result.starts_with("from typing import Any\n"));
        assert!(result.contains("\nimport json\n"));
        assert!(result.contains("class Foo:"));
    }
}
