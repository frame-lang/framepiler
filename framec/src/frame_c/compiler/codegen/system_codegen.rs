//! System Code Generation from Frame AST
//!
//! This module transforms Frame AST (SystemAst) into CodegenNode for emission
//! by language-specific backends.
//!
//! Uses the "oceans model" - native code is preserved exactly, Frame segments
//! are replaced with generated code using the splicer.

use crate::frame_c::visitors::TargetLanguage;
use super::codegen_utils::{
    HandlerContext, expression_to_string, type_to_string, state_var_init_value,
    convert_expression, convert_literal, extract_type_from_raw_domain,
    is_int_type, is_float_type, is_bool_type, is_string_type,
    to_snake_case, cpp_map_type, cpp_wrap_any_arg, java_map_type,
    kotlin_map_type, swift_map_type, csharp_map_type, go_map_type, type_to_cpp_string,
};
use super::interface_gen::{
    generate_interface_wrappers, generate_action, generate_operation,
    generate_persistence_methods,
};
use super::state_dispatch::{
    generate_state_handlers_via_arcanum, generate_handler_from_arcanum,
};
use super::frame_expansion::{
    splice_handler_body_from_span, generate_frame_expansion,
    get_native_scanner, normalize_indentation,
};
use crate::frame_c::compiler::frame_ast::{
    SystemAst, MachineAst,
    ActionAst, OperationAst, Type,
    Expression, Literal, BinaryOp, UnaryOp, StateVarAst,
    InterfaceMethod, MethodParam, Span,
};
use crate::frame_c::compiler::arcanum::{Arcanum, HandlerEntry};
use crate::frame_c::compiler::splice::Splicer;
use crate::frame_c::compiler::native_region_scanner::{
    NativeRegionScanner, Region, FrameSegmentKind,
    python::NativeRegionScannerPy,
    typescript::NativeRegionScannerTs,
    rust::NativeRegionScannerRust,
    csharp::NativeRegionScannerCs,
    c::NativeRegionScannerC,
    cpp::NativeRegionScannerCpp,
    java::NativeRegionScannerJava,
    go::NativeRegionScannerGo,
    javascript::NativeRegionScannerJs,
    php::NativeRegionScannerPhp,
    kotlin::NativeRegionScannerKotlin,
    swift::NativeRegionScannerSwift,
    ruby::NativeRegionScannerRuby,
    erlang::NativeRegionScannerErlang,
    lua::NativeRegionScannerLua,
    dart::NativeRegionScannerDart,
    gdscript::NativeRegionScannerGDScript,
};
use super::ast::*;
use super::backend::get_backend;

/// True iff the init expression text contains any of the supplied param
/// names as a whole word (identifier-boundary match). Used to detect the
/// name-collision case `balance: int = balance` where a domain field
/// initializer references a constructor parameter — only then do we
/// move init out of the field declaration into the constructor body for
/// the strict-init OO backends. Literal initializers (`mutableListOf<>()`,
/// `0`, `-1`) stay at field-decl scope so type inference still works.
fn init_references_param(init_text: &str, params: &[String]) -> bool {
    if params.is_empty() || init_text.is_empty() {
        return false;
    }
    let bytes = init_text.as_bytes();
    for p in params {
        let pb = p.as_bytes();
        if pb.is_empty() {
            continue;
        }
        let mut i = 0usize;
        while i + pb.len() <= bytes.len() {
            if let Some(found) = bytes[i..]
                .windows(pb.len())
                .position(|w| w == pb)
            {
                let start = i + found;
                let end = start + pb.len();
                let prev_ok = start == 0
                    || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
                let next_ok = end == bytes.len()
                    || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
                if prev_ok && next_ok {
                    return true;
                }
                i = end;
            } else {
                break;
            }
        }
    }
    false
}

/// Generate a complete CodegenNode for a Frame system
///
/// # Arguments
/// * `system` - The parsed Frame system AST
/// * `arcanum` - Symbol table for the system (used for handler info and validation)
/// * `lang` - Target language for code generation
/// * `source` - Original source bytes (used to extract native code via spans)
pub fn generate_system(system: &SystemAst, arcanum: &Arcanum, lang: TargetLanguage, source: &[u8]) -> CodegenNode {
    // Erlang: gen_statem native — completely different codegen path
    if lang == TargetLanguage::Erlang {
        return super::erlang_system::generate_erlang_system(system, arcanum, source);
    }

    let backend = get_backend(lang);
    let syntax = backend.class_syntax();

    // Determine if this system needs async dispatch chain
    let needs_async = system.interface.iter().any(|m| m.is_async);

    // Generate fields
    let fields = generate_fields(system, &syntax);

    // Generate methods
    let mut methods = Vec::new();

    // Constructor (for async systems, skips $> kernel call)
    methods.push(generate_constructor(system, &syntax));

    // Frame machinery (transition, state management)
    methods.extend(generate_frame_machinery(system, &syntax, lang));

    // Interface wrappers
    methods.extend(generate_interface_wrappers(system, &syntax));

    // Check if system has states with state variables (for Rust compartment-based push/pop)
    let has_state_vars = system.machine.as_ref()
        .map(|m| m.states.iter().any(|s| !s.state_vars.is_empty()))
        .unwrap_or(false);

    // State handlers - use enhanced Arcanum for clean iteration
    if let Some(ref machine) = system.machine {
        methods.extend(generate_state_handlers_via_arcanum(&system.name, machine, arcanum, source, lang, has_state_vars));
    }

    // Actions - extract native code from source using spans
    for action in &system.actions {
        methods.push(generate_action(action, &syntax, source));
    }

    // Operations - extract native code from source using spans
    for operation in &system.operations {
        methods.push(generate_operation(operation, &syntax, source));
    }

    // Persistence methods (when @@persist is present)
    if system.persist_attr.is_some() {
        methods.extend(generate_persistence_methods(system, &syntax));
    }

    let mut class_node = CodegenNode::Class {
        name: system.name.clone(),
        fields,
        methods,
        base_classes: vec![],
        is_abstract: false,
        derives: vec![],  // Derives not used - we manually build JSON
    };

    // Post-process: make dispatch chain async if any interface method is async
    if needs_async {
        make_system_async(&mut class_node, &system.name, lang);
    }

    class_node
}

/// Transform a generated system class to use async dispatch.
///
/// When any interface method is declared `async`, the entire dispatch chain
/// (interface methods, kernel, router, state dispatch, handlers) must be async.
/// This post-processes the CodegenNode tree to:
/// 1. Set `is_async: true` on all non-static, non-constructor methods
/// 2. The backends handle `is_async` to emit `async def` / `async` / etc.
///
/// Note: `await` on internal dispatch calls is handled by the backends
/// recognizing Await nodes in the method bodies, or via NativeBlock code
/// that already contains the dispatch calls.
fn make_system_async(class_node: &mut CodegenNode, _system_name: &str, lang: TargetLanguage) {
    if let CodegenNode::Class { ref mut methods, ref name, .. } = class_node {
        let system_name = name.clone();
        for method in methods.iter_mut() {
            if let CodegenNode::Method { is_async, is_static, name, body, .. } = method {
                // Skip static methods and constructors
                if *is_static || name == "__init__" || name == "new" {
                    continue;
                }
                // Skip __transition (synchronous compartment swap, no dispatch)
                // But NOT __push_transition (has dispatch calls that need await)
                if name == "__transition" {
                    continue;
                }
                *is_async = true;
                // Add `await` to internal dispatch calls in NativeBlock strings
                add_await_to_dispatch_calls(body, lang);
            }
            if let CodegenNode::Constructor { body, .. } = method {
                // Constructor stays sync — remove kernel call for async systems
                // (user calls `await system.init()` instead)
                remove_kernel_call_from_body(body);
            }
        }

        // Add async init() method — fires $> enter event
        let init_code = match lang {
            TargetLanguage::Python3 => format!(
                r#"__e = {s}FrameEvent("$>", None)
__ctx = {s}FrameContext(__e, None)
self._context_stack.append(__ctx)
await self.__kernel(__e)
self._context_stack.pop()"#,
                s = system_name
            ),
            TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!(
                r#"const __e = new {s}FrameEvent("$>", null);
const __ctx = new {s}FrameContext(__e, null);
this._context_stack.push(__ctx);
await this.__kernel(__e);
this._context_stack.pop();"#,
                s = system_name
            ),
            TargetLanguage::Rust => format!(
                r#"let __frame_event = {s}FrameEvent::new("$>");
let __ctx = {s}FrameContext::new(__frame_event, None);
self._context_stack.push(__ctx);
self.__kernel().await;
self._context_stack.pop();"#,
                s = system_name
            ),
            // Async not supported for these targets — emit a comment placeholder
            TargetLanguage::C | TargetLanguage::Cpp => format!("// async not supported for C/C++"),
            // Languages with async that haven't been implemented yet
            TargetLanguage::Java | TargetLanguage::Kotlin | TargetLanguage::Swift
                | TargetLanguage::CSharp | TargetLanguage::Go | TargetLanguage::Php
                | TargetLanguage::Ruby | TargetLanguage::Lua | TargetLanguage::Dart | TargetLanguage::GDScript => {
                format!("// async init not yet implemented for {:?}", lang)
            }
            TargetLanguage::Erlang => String::new(), // TODO: Erlang gen_statem codegen
            TargetLanguage::Graphviz => unreachable!(),
        };
        let init_body = vec![
            CodegenNode::NativeBlock {
                code: init_code,
                span: None,
            },
        ];

        methods.push(CodegenNode::Method {
            name: "init".to_string(),
            params: vec![],
            return_type: None,
            body: init_body,
            is_async: true,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        });
    }
}

/// Remove kernel call from constructor body (for async two-phase init).
fn remove_kernel_call_from_body(body: &mut Vec<CodegenNode>) {
    body.retain(|node| {
        if let CodegenNode::NativeBlock { code, .. } = node {
            // Remove lines that call __kernel in constructor
            !code.contains("__kernel(") && !code.contains("_kernel(self,")
        } else {
            true
        }
    });
}

/// Walk method body and add `await` to dispatch calls in NativeBlock strings.
/// Dispatch calls are: self.__kernel(...), self.__router(...), self._state_*(...),
/// self._s_*(...) — all internal Frame dispatch methods.
fn add_await_to_dispatch_calls(body: &mut Vec<CodegenNode>, lang: TargetLanguage) {
    for node in body.iter_mut() {
        match node {
            CodegenNode::NativeBlock { code, .. } => {
                *code = add_await_to_string(code, lang);
            }
            CodegenNode::If { then_block, else_block, .. } => {
                add_await_to_dispatch_calls(then_block, lang);
                if let Some(els) = else_block {
                    add_await_to_dispatch_calls(els, lang);
                }
            }
            CodegenNode::While { body: while_body, .. } => {
                add_await_to_dispatch_calls(while_body, lang);
            }
            _ => {}
        }
    }
}

/// Add `await` to Frame dispatch calls in a string.
/// Python/TypeScript: prefix `await` (e.g., `await self.__kernel()`)
/// Rust: postfix `.await` (e.g., `self.__kernel().await`)
fn add_await_to_string(code: &str, lang: TargetLanguage) -> String {
    let mut result = String::with_capacity(code.len() + 100);
    for line in code.lines() {
        let trimmed = line.trim();
        // Match dispatch call patterns that need await
        let needs_await = trimmed.starts_with("self.__kernel(")
            || trimmed.starts_with("self.__router(")
            || (trimmed.starts_with("self._state_") && !trimmed.starts_with("self._state_stack"))
            || trimmed.starts_with("self._s_")
            || trimmed.starts_with("handler(")         // Python dynamic dispatch
            || trimmed.starts_with("handler.call(")   // TypeScript dynamic dispatch
            || trimmed.starts_with("this.__kernel(")
            || trimmed.starts_with("this.__router(")
            || trimmed.starts_with("this._state_")
            || trimmed.starts_with("this._s_")
            || trimmed.starts_with("this.#kernel(")
            || trimmed.starts_with("this.#router(")
            || trimmed.starts_with("$this->__kernel(")
            || trimmed.starts_with("$this->__router(")
            || trimmed.starts_with("$this->_state_")
            // Rust match arms and braced dispatch:
            // "StateName" => self._state_X(__e),
            // { self._s_Ready_fetch(__e, key); }
            || (trimmed.contains("self._state_") && !trimmed.contains("_state_stack") && !trimmed.starts_with("fn ") && !trimmed.starts_with("async fn "))
            || (trimmed.contains("self._s_") && !trimmed.starts_with("fn ") && !trimmed.starts_with("async fn "));
        if needs_await {
            let indent = &line[..line.len() - trimmed.len()];
            match lang {
                TargetLanguage::Rust => {
                    // Rust: postfix .await — insert before ); or ); } or ), etc.
                    if !trimmed.contains(".await") {
                        result.push_str(indent);
                        // Find the call's closing paren `)` and insert `.await` after it
                        let modified = insert_rust_await(trimmed);
                        result.push_str(&modified);
                    } else {
                        result.push_str(line);
                    }
                }
                // All other async-capable languages use prefix `await`
                TargetLanguage::Python3 | TargetLanguage::TypeScript | TargetLanguage::JavaScript
                    | TargetLanguage::CSharp | TargetLanguage::Kotlin | TargetLanguage::Swift
                    | TargetLanguage::Java | TargetLanguage::Go | TargetLanguage::C
                    | TargetLanguage::Cpp | TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang
                    | TargetLanguage::Lua | TargetLanguage::Dart | TargetLanguage::GDScript => {
                    // Python/TypeScript/C#/etc: prefix await
                    if !trimmed.starts_with("await ") {
                        result.push_str(indent);
                        result.push_str("await ");
                        result.push_str(trimmed);
                    } else {
                        result.push_str(line);
                    }
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    // Remove trailing newline if original didn't have one
    if !code.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

/// Insert `.await` after the closing paren of a Rust function call.
/// Handles patterns like:
///   `self.__kernel();`          → `self.__kernel().await;`
///   `self._state_Ready(__e),`   → `self._state_Ready(__e).await,`
///   `{ self._s_X(__e, k); }`   → `{ self._s_X(__e, k).await; }`
fn insert_rust_await(line: &str) -> String {
    // Find the last `)` that's part of a function call, insert `.await` after it
    let bytes = line.as_bytes();
    let mut last_close_paren = None;
    let mut depth = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'(' { depth += 1; }
        if b == b')' {
            depth -= 1;
            if depth == 0 {
                last_close_paren = Some(i);
            }
        }
    }
    if let Some(pos) = last_close_paren {
        let mut result = String::with_capacity(line.len() + 6);
        result.push_str(&line[..pos + 1]);
        result.push_str(".await");
        result.push_str(&line[pos + 1..]);
        result
    } else {
        // No paren found — just append .await before semicolon
        let trimmed_semi = line.trim_end_matches(';');
        format!("{}.await;", trimmed_semi)
    }
}

/// Generate class fields for the system
fn generate_fields(system: &SystemAst, syntax: &super::backend::ClassSyntax) -> Vec<Field> {
    let mut fields = Vec::new();
    let compartment_type = format!("{}Compartment", system.name);

    // State stack - for push/pop state operations
    let stack_type = match syntax.language {
        TargetLanguage::Rust => format!("Vec<{}Compartment>", system.name),
        TargetLanguage::Cpp => format!("std::vector<std::unique_ptr<{}Compartment>>", system.name),
        TargetLanguage::Java => format!("ArrayList<{}Compartment>", system.name),
        TargetLanguage::Kotlin => format!("MutableList<{}Compartment>", system.name),
        TargetLanguage::Dart => format!("List<{}Compartment>", system.name),
        TargetLanguage::Swift => format!("[{}Compartment]", system.name),
        TargetLanguage::CSharp => format!("List<{}Compartment>", system.name),
        TargetLanguage::Go => format!("[]*{}Compartment", system.name),
        // Dynamic languages: untyped lists — type annotation is for documentation only
        TargetLanguage::Python3 | TargetLanguage::TypeScript | TargetLanguage::JavaScript
            | TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang | TargetLanguage::Lua
            | TargetLanguage::GDScript => "List".to_string(),
        TargetLanguage::C => "List".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(Field::new("_state_stack")
        .with_visibility(Visibility::Private)
        .with_type(&stack_type));

    // Compartment field - canonical compartment architecture for ALL languages
    let (comp_field_type, nullable_comp_type) = match syntax.language {
        TargetLanguage::Rust => (
            compartment_type.clone(),
            format!("Option<{}>", compartment_type),
        ),
        TargetLanguage::Cpp => (
            format!("std::unique_ptr<{}>", compartment_type),
            format!("std::unique_ptr<{}>", compartment_type),
        ),
        TargetLanguage::Java | TargetLanguage::CSharp => (
            compartment_type.clone(),
            compartment_type.clone(),
        ),
        TargetLanguage::Kotlin | TargetLanguage::Swift | TargetLanguage::Dart => (
            compartment_type.clone(),
            format!("{}?", compartment_type),
        ),
        TargetLanguage::Go => (
            format!("*{}", compartment_type),
            format!("*{}", compartment_type),
        ),
        // Dynamic languages: nullable via language convention (None/null/nil)
        TargetLanguage::Python3 | TargetLanguage::Ruby | TargetLanguage::Erlang | TargetLanguage::Lua
            | TargetLanguage::GDScript => (
            compartment_type.clone(),
            compartment_type.clone(),
        ),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => (
            compartment_type.clone(),
            format!("{} | null", compartment_type),
        ),
        TargetLanguage::Php => (
            compartment_type.clone(),
            format!("?{}", compartment_type),
        ),
        TargetLanguage::C => (
            format!("{}*", compartment_type),
            format!("{}*", compartment_type),
        ),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(Field::new("__compartment")
        .with_visibility(Visibility::Private)
        .with_type(&comp_field_type));

    // Next compartment field - for deferred transition caching in __kernel
    fields.push(Field::new("__next_compartment")
        .with_visibility(Visibility::Private)
        .with_type(&nullable_comp_type));

    // Context stack for reentrancy - holds FrameContext objects
    let context_stack_type = match syntax.language {
        TargetLanguage::Rust => format!("Vec<{}FrameContext>", system.name),
        TargetLanguage::Cpp => format!("std::vector<{}FrameContext>", system.name),
        TargetLanguage::Java => format!("ArrayList<{}FrameContext>", system.name),
        TargetLanguage::Kotlin => format!("MutableList<{}FrameContext>", system.name),
        TargetLanguage::Dart => format!("List<{}FrameContext>", system.name),
        TargetLanguage::Swift => format!("[{}FrameContext]", system.name),
        TargetLanguage::CSharp => format!("List<{}FrameContext>", system.name),
        TargetLanguage::Go => format!("[]{}FrameContext", system.name),
        // Dynamic languages: untyped lists
        TargetLanguage::Python3 | TargetLanguage::TypeScript | TargetLanguage::JavaScript
            | TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang | TargetLanguage::Lua
            | TargetLanguage::GDScript => "List".to_string(),
        TargetLanguage::C => "List".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(Field::new("_context_stack")
        .with_visibility(Visibility::Private)
        .with_type(&context_stack_type));

    /// Render a `DomainVar`'s structured fields back to a single source
    /// line in whichever shape the target language's emit_field expects.
    /// Used as a transitional bridge while per-backend emitters still
    /// consume raw_code rather than the structured slots directly.
    fn synthesize_field_raw(
        var: &crate::frame_c::compiler::frame_ast::DomainVar,
        lang: TargetLanguage,
        sys_param_names: &[String],
    ) -> String {
        let type_text = match &var.var_type {
            Type::Custom(s) => s.clone(),
            Type::Unknown => String::new(),
        };
        // Go and C never permit field-level initializers on struct
        // declarations, so any domain field init MUST move to the
        // factory function body. Strip unconditionally for those.
        //
        // For the OO languages with constructors (C++, Java, Swift,
        // Kotlin, C#, Dart, TypeScript), only strip the field-level
        // init when the init expression references a system parameter —
        // that's the name-collision case `name: string = name;` where
        // the RHS at field-declaration scope resolves to the field
        // itself rather than the constructor parameter. (TypeScript
        // explicitly rejects this with TS2301 at compile time.) For
        // literal initializers (`var log = mutableListOf<String>()`,
        // `int count = 0`), we leave the init at field-declaration
        // scope so type inference still works.
        let init_text = var.initializer_text.as_deref().unwrap_or("");
        let strip_unconditionally = matches!(lang, TargetLanguage::Go | TargetLanguage::C);
        let strip_collision = matches!(
            lang,
            TargetLanguage::Cpp
                | TargetLanguage::Java
                | TargetLanguage::Swift
                | TargetLanguage::Kotlin
                | TargetLanguage::CSharp
                | TargetLanguage::Dart
                | TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Php
        ) && init_references_param(init_text, sys_param_names);
        let strip_init = strip_unconditionally || strip_collision;

        let init_suffix = if strip_init {
            String::new()
        } else {
            match &var.initializer_text {
                Some(t) => format!(" = {}", t),
                None => String::new(),
            }
        };
        // GoStyle: `<name> <type>[ = <init>]`. Go's emit_field expects
        // this exact shape — name first, then type, no colon.
        if matches!(lang, TargetLanguage::Go) {
            if type_text.is_empty() {
                return format!("{}{}", var.name, init_suffix);
            }
            return format!("{} {}{}", var.name, type_text, init_suffix);
        }
        // TypeFirst-shaped languages: `<type> <name>[ = <init>]`
        let type_first = matches!(
            lang,
            TargetLanguage::C
                | TargetLanguage::Cpp
                | TargetLanguage::Java
                | TargetLanguage::CSharp
                | TargetLanguage::Dart
        );
        if type_first && !type_text.is_empty() {
            format!("{} {}{}", type_text, var.name, init_suffix)
        } else if !type_text.is_empty() {
            // AnnotatedName: `<name>: <type>[ = <init>]`
            format!("{}: {}{}", var.name, type_text, init_suffix)
        } else {
            // BareName / unknown type: `<name>[ = <init>]`
            format!("{}{}", var.name, init_suffix)
        }
    }

    // Domain variables.
    //
    // The pipeline parser's domain_native module produces structured
    // (name, var_type, initializer_text) tuples for every field. We
    // build a Field codegen node from those structured slots — no more
    // string surgery on raw_code.
    //
    // For each backend's `emit_field` we currently still pass raw_code
    // for backwards compat (existing emitters fall back to it). The raw
    // code is now SYNTHESISED from the structured fields rather than
    // taken from the source line, so the per-backend emitter sees a
    // canonical form. Future backend cleanups can drop raw_code in
    // favor of the structured slots.
    for domain_var in &system.domain {
        let type_str = type_to_string(&domain_var.var_type);

        // Build a synthesised raw_code line from the structured fields
        // for the backends that still consume raw_code at the field
        // declaration site. The shape varies per backend:
        //   - C / C++ / Java / etc. (TypeFirst):  `<type> <name> = <init>`
        //   - Python / TS / Rust / etc.:           `<name>: <type> = <init>`
        //   - Erlang (BareName):                   `<name> = <init>`
        // For now we hand back the AnnotatedName form (`<name>: <type> = <init>`)
        // for languages that historically used it, and TypeFirst for the
        // C-family backends. Init is omitted entirely if not present, so
        // raw_code remains a faithful representation of what the user
        // would have written.
        let sys_param_names: Vec<String> = system.params.iter().map(|p| p.name.clone()).collect();
        let synthesised_raw = synthesize_field_raw(domain_var, syntax.language, &sys_param_names);
        let expanded_code = expand_tagged_in_domain(&synthesised_raw, syntax.language);

        let mut field = Field::new(&domain_var.name)
            .with_visibility(Visibility::Public)
            .with_type(&type_str)
            .with_raw_code(&expanded_code);

        // Also populate the structured initializer slot when the field
        // has an init expression and the legacy Expression form is
        // present (always None today; reserved for a future Frame
        // expression parser that handles domain init expressions
        // structurally).
        if let Some(ref init) = &domain_var.initializer {
            field = field.with_initializer(convert_expression(init));
        }

        fields.push(field);
    }

    // Rust state vars now live on compartment.state_context (StateContext enum)
    // No _sv_* struct fields needed

    // Rust system header state/enter params: stash on the system struct
    // as `__sys_<name>` typed fields. The constructor receives the
    // system params via its signature (system_codegen.rs:1752 — params
    // from system.params), assigns them into these synthetic fields,
    // and the per-state dispatch reads them into bare locals via the
    // binding preamble inserted by `generate_handler_from_arcanum`.
    //
    // Domain params are handled by the existing domain field path
    // (the domain field IS the storage), so we skip them here.
    //
    // This is the Rust equivalent of the HashMap<String, Any>
    // `state_args`/`enter_args` dict approach used by the dynamic
    // backends — typed fields keep idiomatic Rust without inventing
    // a new typed-enum variant per state.
    if matches!(syntax.language, TargetLanguage::Rust) {
        for p in &system.params {
            match p.kind {
                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                | crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                    let type_str = type_to_string(&p.param_type);
                    fields.push(
                        Field::new(&format!("__sys_{}", p.name))
                            .with_visibility(Visibility::Private)
                            .with_type(&type_str),
                    );
                }
                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
            }
        }
    }

    fields
}

/// Generate the constructor
fn generate_constructor(system: &SystemAst, syntax: &super::backend::ClassSyntax) -> CodegenNode {
    let mut body = Vec::new();

    // Initialize state stack - language specific
    match syntax.language {
        TargetLanguage::C => {
            // C: Use FrameVec_new()
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_state_stack"),
                CodegenNode::Ident(format!("{}_FrameVec_new()", system.name)),
            ));
        }
        TargetLanguage::Cpp => {
            // C++: vectors default-construct as empty, no init needed
        }
        TargetLanguage::Java => {
            // Java: ArrayList fields are null by default, must init
            body.push(CodegenNode::NativeBlock {
                code: format!("_state_stack = new ArrayList<>();"),
                span: None,
            });
        }
        TargetLanguage::Kotlin => {
            body.push(CodegenNode::NativeBlock {
                code: format!("_state_stack = mutableListOf()"),
                span: None,
            });
        }
        TargetLanguage::Swift => {
            body.push(CodegenNode::NativeBlock {
                code: format!("_state_stack = []"),
                span: None,
            });
        }
        TargetLanguage::CSharp => {
            // C#: List fields must be initialized
            body.push(CodegenNode::NativeBlock {
                code: format!("_state_stack = new List<{}Compartment>();", system.name),
                span: None,
            });
        }
        TargetLanguage::Go => {
            // Go: slices are nil by default, initialize as empty
            body.push(CodegenNode::NativeBlock {
                code: format!("s._state_stack = make([]*{}Compartment, 0)", system.name),
                span: None,
            });
        }
        // Dynamic languages: empty array literal
        TargetLanguage::Python3 | TargetLanguage::TypeScript | TargetLanguage::JavaScript
            | TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang | TargetLanguage::Rust
            | TargetLanguage::Lua | TargetLanguage::Dart | TargetLanguage::GDScript => {
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_state_stack"),
                CodegenNode::Array(vec![]),
            ));
        }
        TargetLanguage::Graphviz => unreachable!(),
    }

    // Initialize context stack (for reentrancy support) - language specific
    match syntax.language {
        TargetLanguage::C => {
            // C: Use FrameVec_new()
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_context_stack"),
                CodegenNode::Ident(format!("{}_FrameVec_new()", system.name)),
            ));
        }
        TargetLanguage::Cpp => {
            // C++: vectors default-construct as empty, no init needed
        }
        TargetLanguage::Java => {
            body.push(CodegenNode::NativeBlock {
                code: format!("_context_stack = new ArrayList<>();"),
                span: None,
            });
        }
        TargetLanguage::Kotlin => {
            body.push(CodegenNode::NativeBlock {
                code: format!("_context_stack = mutableListOf()"),
                span: None,
            });
        }
        TargetLanguage::Swift => {
            body.push(CodegenNode::NativeBlock {
                code: format!("_context_stack = []"),
                span: None,
            });
        }
        TargetLanguage::CSharp => {
            body.push(CodegenNode::NativeBlock {
                code: format!("_context_stack = new List<{}FrameContext>();", system.name),
                span: None,
            });
        }
        TargetLanguage::Go => {
            body.push(CodegenNode::NativeBlock {
                code: format!("s._context_stack = make([]{}FrameContext, 0)", system.name),
                span: None,
            });
        }
        // Dynamic languages: empty array literal
        TargetLanguage::Python3 | TargetLanguage::TypeScript | TargetLanguage::JavaScript
            | TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang | TargetLanguage::Rust
            | TargetLanguage::Lua | TargetLanguage::Dart | TargetLanguage::GDScript => {
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_context_stack"),
                CodegenNode::Array(vec![]),
            ));
        }
        TargetLanguage::Graphviz => unreachable!(),
    }

    // Initialize domain variables
    let sys_param_names_for_init: Vec<String> =
        system.params.iter().map(|p| p.name.clone()).collect();
    for domain_var in &system.domain {
        // For the strict-init OO backends (C++, Java, Swift, Kotlin, C#,
        // Dart), `synthesize_field_raw` only strips the field-level init
        // when the init expression references a system parameter. We
        // mirror that decision here: emit the constructor-body assignment
        // ONLY when the init was stripped, otherwise the field-decl init
        // already handles initialization and a duplicate would either be
        // redundant (literal init) or invalid (some langs reject double
        // assignment of `val`/`final`/etc).
        //
        // C and Go are different — neither has any field-level init at
        // all (C: no struct field defaults; Go: no struct field defaults
        // outside literals), so they always emit constructor-body init.
        let init_text_opt = domain_var.initializer_text.clone();
        let init_refs_param = init_text_opt
            .as_deref()
            .map(|t| init_references_param(t, &sys_param_names_for_init))
            .unwrap_or(false);
        // C: factory-function body init. Always emit when there's an init.
        if matches!(syntax.language, TargetLanguage::C) {
            if let Some(ref init_text) = init_text_opt {
                let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::C);
                body.push(CodegenNode::NativeBlock {
                    code: format!("self->{} = {};", domain_var.name, init_expanded),
                    span: None,
                });
            }
            continue;
        }
        // C++ — only emit constructor-body init when the field-level init
        // was stripped (i.e., the init references a constructor param).
        if matches!(syntax.language, TargetLanguage::Cpp) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::Cpp);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("this->{} = {};", domain_var.name, init_expanded),
                        span: None,
                    });
                }
            }
            continue;
        }
        // Go: factory-function body init. Always emit (no field-level init).
        if matches!(syntax.language, TargetLanguage::Go) {
            if let Some(ref init_text) = init_text_opt {
                let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::Go);
                body.push(CodegenNode::NativeBlock {
                    code: format!("s.{} = {}", domain_var.name, init_expanded),
                    span: None,
                });
            }
            continue;
        }
        // Java — only emit constructor-body init when the field-level init
        // was stripped.
        if matches!(syntax.language, TargetLanguage::Java) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::Java);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("this.{} = {};", domain_var.name, init_expanded),
                        span: None,
                    });
                }
            }
            continue;
        }
        // Swift — only emit constructor-body init when stripped.
        if matches!(syntax.language, TargetLanguage::Swift) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::Swift);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("self.{} = {}", domain_var.name, init_expanded),
                        span: None,
                    });
                }
            }
            continue;
        }
        // Kotlin — only emit constructor-body init when stripped.
        if matches!(syntax.language, TargetLanguage::Kotlin) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::Kotlin);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("this.{} = {}", domain_var.name, init_expanded),
                        span: None,
                    });
                }
            }
            continue;
        }
        // C# — only emit constructor-body init when stripped.
        if matches!(syntax.language, TargetLanguage::CSharp) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::CSharp);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("this.{} = {};", domain_var.name, init_expanded),
                        span: None,
                    });
                }
            }
            continue;
        }
        // Dart — only emit constructor-body init when stripped.
        if matches!(syntax.language, TargetLanguage::Dart) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::Dart);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("this.{} = {};", domain_var.name, init_expanded),
                        span: None,
                    });
                }
            }
            continue;
        }
        // TypeScript — only emit constructor-body init when stripped.
        // TypeScript would otherwise reject `name: string = name` at class
        // scope with TS2301 ("Initializer of instance member variable
        // 'name' cannot reference identifier 'name' declared in the
        // constructor.").
        if matches!(syntax.language, TargetLanguage::TypeScript) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::TypeScript);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("this.{} = {};", domain_var.name, init_expanded),
                        span: None,
                    });
                }
            }
            continue;
        }
        // JavaScript — same name-collision rule as TypeScript. JS class
        // field initializers can't see constructor parameters either, so
        // `name = name` at field scope reads the undeclared identifier.
        if matches!(syntax.language, TargetLanguage::JavaScript) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::JavaScript);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("this.{} = {};", domain_var.name, init_expanded),
                        span: None,
                    });
                }
            }
            continue;
        }
        // PHP — same collision rule. PHP rejects field initializers that
        // reference variables ("Constant expression contains invalid
        // operations"), so any init referencing a constructor param
        // moves into the __construct body as `$this->name = $param;`.
        if matches!(syntax.language, TargetLanguage::Php) {
            if init_refs_param {
                if let Some(ref init_text) = init_text_opt {
                    let init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::Php);
                    body.push(CodegenNode::NativeBlock {
                        code: format!("$this->{} = {};", domain_var.name, init_expanded),
                        span: None,
                    });
                }
                continue;
            }
            // No collision — fall through to legacy raw_code branch.
        }
        if let Some(ref raw_code) = domain_var.raw_code {
            // V4: Native code pass-through
            // Expand @@SystemName() tagged instantiations in domain initializers
            let raw_code = &expand_tagged_in_domain(raw_code, syntax.language);
            // Python: emit as self.<raw_code> in __init__
            // TypeScript: already in class fields, skip constructor init
            // C: struct is zeroed by calloc
            // Rust: need explicit init in struct literal
            if matches!(syntax.language, TargetLanguage::Python3) {
                body.push(CodegenNode::NativeBlock {
                    code: format!("self.{}", raw_code),
                    span: None,
                });
            } else if matches!(syntax.language, TargetLanguage::GDScript) {
                // GDScript: strip type annotations and var keyword from self.field assignments
                // "var name = value" -> "name = value"
                // "name: type = value" -> "name = value"
                let raw_code_stripped = if raw_code.starts_with("var ") {
                    &raw_code[4..]
                } else {
                    raw_code
                };
                let gd_code = if let Some(colon_pos) = raw_code_stripped.find(':') {
                    if let Some(eq_pos) = raw_code_stripped.find('=') {
                        if colon_pos < eq_pos {
                            let name_part = raw_code_stripped[..colon_pos].trim();
                            let value_part = raw_code_stripped[eq_pos..].trim();
                            format!("{} {}", name_part, value_part)
                        } else {
                            raw_code_stripped.to_string()
                        }
                    } else {
                        // "name: type" with no initializer -> "name = null"
                        let name_part = raw_code_stripped[..colon_pos].trim();
                        format!("{} = null", name_part)
                    }
                } else {
                    raw_code_stripped.to_string()
                };
                body.push(CodegenNode::NativeBlock {
                    code: format!("self.{}", gd_code),
                    span: None,
                });
            } else if matches!(syntax.language, TargetLanguage::Php) {
                // PHP: emit as $this->field = value in __construct
                body.push(CodegenNode::NativeBlock {
                    code: format!("$this->{};", raw_code),
                    span: None,
                });
            } else if matches!(syntax.language, TargetLanguage::Ruby) {
                // Ruby: emit as @field = value in initialize
                // Strip type annotation (e.g., "name: type = value" -> "name = value")
                let ruby_code = if let Some(colon_pos) = raw_code.find(':') {
                    if let Some(eq_pos) = raw_code.find('=') {
                        if colon_pos < eq_pos {
                            // Has type annotation: "name: type = value" -> "name = value"
                            let name_part = raw_code[..colon_pos].trim();
                            let value_part = raw_code[eq_pos..].trim();
                            format!("{} {}", name_part, value_part)
                        } else {
                            raw_code.to_string()
                        }
                    } else {
                        // "name: type" with no initializer -> "name = nil"
                        let name_part = raw_code[..colon_pos].trim();
                        format!("{} = nil", name_part)
                    }
                } else {
                    raw_code.to_string()
                };
                body.push(CodegenNode::NativeBlock {
                    code: format!("@{}", ruby_code),
                    span: None,
                });
            } else if matches!(syntax.language, TargetLanguage::Lua) {
                // Lua: emit `self.<name> = <init>` in the constructor
                // body. Lua tables don't have field declarations at all,
                // so domain init MUST happen at construction time.
                // Translate Frame's empty-list `[]` literal to Lua's
                // `{}` table literal so domains like `log: list = []`
                // produce valid Lua.
                if let Some(ref init_text) = domain_var.initializer_text {
                    let mut init_expanded = expand_tagged_in_domain(init_text, TargetLanguage::Lua);
                    if init_expanded.trim() == "[]" {
                        init_expanded = "{}".to_string();
                    }
                    body.push(CodegenNode::NativeBlock {
                        code: format!("self.{} = {}", domain_var.name, init_expanded),
                        span: None,
                    });
                }
                continue;
            } else if matches!(syntax.language, TargetLanguage::Rust) {
                // For Rust, extract initializer from raw_code (after '=')
                let init_expr = raw_code.split_once('=')
                    .map(|(_, v)| v.trim().to_string())
                    .unwrap_or_else(|| "Default::default()".to_string());
                body.push(CodegenNode::assign(
                    CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                    CodegenNode::Ident(init_expr),
                ));
            }
        } else if let Some(ref init) = &domain_var.initializer {
            // Construct from parsed components
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                convert_expression(init),
            ));
        } else if matches!(syntax.language, TargetLanguage::Rust) {
            // Rust requires all struct fields to be initialized.
            // Domain vars with no initializer get Default::default().
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                CodegenNode::Ident("Default::default()".to_string()),
            ));
        }
    }

    // Rust state vars now live on compartment.state_context — no _sv_ field init needed.
    // State vars are initialized when compartments are created (in transition codegen).

    // Rust system header state/enter param fields: assign each
    // synthetic `__sys_<name>` field from the constructor parameter
    // of the same name. The Rust constructor emitter (rust.rs) sees
    // these as `self.field = value` assignments and folds them into
    // the struct literal.
    if matches!(syntax.language, TargetLanguage::Rust) {
        for p in &system.params {
            match p.kind {
                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                | crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                    body.push(CodegenNode::assign(
                        CodegenNode::field(
                            CodegenNode::self_ref(),
                            &format!("__sys_{}", p.name),
                        ),
                        CodegenNode::Ident(p.name.clone()),
                    ));
                }
                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
            }
        }
    }

    // Set initial state (first state in machine)
    // All languages now use the kernel/router/compartment pattern
    if let Some(ref machine) = system.machine {
        if let Some(first_state) = machine.states.first() {
            let compartment_class = format!("{}Compartment", system.name);
            let event_class = format!("{}FrameEvent", system.name);

            // HSM: Build ancestor chain if start state has a parent
            // We need to create compartments for all ancestors and link them via parent_compartment
            let has_hsm_parent = first_state.parent.is_some();

            // Build ancestor chain from root to leaf (reversed order for creation)
            let mut ancestor_chain: Vec<&crate::frame_c::compiler::frame_ast::StateAst> = Vec::new();
            if has_hsm_parent {
                let mut current_parent = first_state.parent.as_ref();
                while let Some(parent_name) = current_parent {
                    if let Some(parent_state) = machine.states.iter().find(|s| &s.name == parent_name) {
                        ancestor_chain.push(parent_state);
                        current_parent = parent_state.parent.as_ref();
                    } else {
                        break;
                    }
                }
                // Reverse so we start from root (topmost parent)
                ancestor_chain.reverse();
            }

            // Initialize __compartment with initial state
            match syntax.language {
                TargetLanguage::Rust => {
                    // Rust: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        // For Rust, use a block expression inside struct literal
                        // This creates parent chain and returns the child compartment
                        let mut block_expr = String::new();
                        block_expr.push_str("{\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "None".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            block_expr.push_str(&format!(
                                "let mut {} = {}Compartment::new(\"{}\");\n",
                                comp_var, system.name, ancestor.name
                            ));
                            block_expr.push_str(&format!(
                                "{}.parent_compartment = {};\n",
                                comp_var, prev_comp_var
                            ));
                            // state_context is auto-set by Compartment::new()
                            prev_comp_var = format!("Some(Box::new({}))", comp_var);
                        }
                        // Create the start state compartment with parent link
                        // state_context is auto-set by Compartment::new()
                        block_expr.push_str(&format!(
                            "let mut __child = {}Compartment::new(\"{}\");\n",
                            system.name, first_state.name
                        ));
                        block_expr.push_str(&format!(
                            "__child.parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        block_expr.push_str("__child\n}");

                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::Ident(block_expr),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::Ident("None".to_string()),
                        ));
                    } else {
                        // No HSM parent - simple compartment creation
                        // state_context is auto-set by Compartment::new()
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::Ident(format!("{}Compartment::new(\"{}\")", system.name, first_state.name)),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::Ident("None".to_string()),
                        ));
                    }
                }
                TargetLanguage::C => {
                    // C: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "NULL".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{}_Compartment* {} = {}_Compartment_new(\"{}\");\n",
                                system.name, comp_var, system.name, ancestor.name
                            ));
                            hsm_init_code.push_str(&format!(
                                "{}->parent_compartment = {};\n",
                                comp_var, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::C)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::C)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}_FrameDict_set({}->state_vars, \"{}\", (void*)(intptr_t){});\n",
                                    system.name, comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "self->__compartment = {}_Compartment_new(\"{}\");\n",
                            system.name, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "self->__compartment->parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("self->__next_compartment = NULL;");

                        // System state and enter params: bind into start state's
                        // state_args / enter_args. Mirrors the Python branch — every
                        // ParamKind::StateArg lands in `state_args[name]` and every
                        // ParamKind::EnterArg lands in `enter_args[name]`. The cast
                        // `(void*)(intptr_t)(name)` matches the rest of the C
                        // codegen's intptr-tagged void-pointer convention so the
                        // dispatch reader (also intptr-cast) round-trips correctly.
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n{}_FrameDict_set(self->__compartment->state_args, \"{}\", (void*)(intptr_t)({}));",
                                        system.name, p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n{}_FrameDict_set(self->__compartment->enter_args, \"{}\", (void*)(intptr_t)({}));",
                                        system.name, p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::Ident(format!("{}_Compartment_new(\"{}\")", system.name, first_state.name)),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "{}_FrameDict_set(self->__compartment->state_args, \"{}\", (void*)(intptr_t)({}));",
                                        system.name, p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "{}_FrameDict_set(self->__compartment->enter_args, \"{}\", (void*)(intptr_t)({}));",
                                        system.name, p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Python3 => {
                    // Python: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("# HSM: Create parent compartment chain\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "None".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} = {}(\"{}\", parent_compartment={})\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Python3)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Python3)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "self.__compartment = {}(\"{}\", parent_compartment={})\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("self.__next_compartment = None");

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));
                        // System state and enter params: bind into start state's
                        // state_args / enter_args. The constructor receives params
                        // via system.params; each StateArg/EnterArg entry writes
                        // into the corresponding compartment dict so the state's
                        // dispatch (state_args) and enter handler (enter_args) can
                        // read them. Domain params are handled by the domain field
                        // initializer loop above and aren't touched here.
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    // TypeScript: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "const {} = new {}(\"{}\", {});\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, syntax.language)
                                } else {
                                    state_var_init_value(&var.var_type, syntax.language)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {};\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = new {}(\"{}\", {});\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "this.__compartment.state_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "this.__compartment.enter_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Php => {
                    // PHP: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("$__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} = new {}(\"{}\", {});\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Php)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Php)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}->state_vars[\"{}\"] = {};\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "$this->__compartment = new {}(\"{}\", {});\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("$this->__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n$this->__compartment->state_args[\"{}\"] = ${};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n$this->__compartment->enter_args[\"{}\"] = ${};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "$this->__compartment->state_args[\"{}\"] = ${};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "$this->__compartment->enter_args[\"{}\"] = ${};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Ruby => {
                    // Ruby: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("# HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "nil".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} = {}.new(\"{}\", {})\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Ruby)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Ruby)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "@__compartment = {}.new(\"{}\", {})\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("@__next_compartment = nil");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n@__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n@__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "@__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "@__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Cpp => {
                    // C++: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_expr = "nullptr".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "auto {} = std::make_unique<{}>(\"{}\");\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_expr != "nullptr" {
                                hsm_init_code.push_str(&format!(
                                    "{}->parent_compartment = std::move({});\n",
                                    comp_var, prev_comp_expr
                                ));
                            }
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    cpp_wrap_any_arg(&expression_to_string(init, TargetLanguage::Cpp))
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Cpp)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}->state_vars[\"{}\"] = std::any({});\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_expr = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "__compartment = std::make_unique<{}>(\"{}\");\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "__compartment->parent_compartment = std::move({});\n",
                            prev_comp_expr
                        ));

                        // System state and enter params: bind into the start
                        // state's state_args / enter_args dicts. Mirrors the
                        // Python and C constructor populations — every
                        // ParamKind::StateArg lands in `state_args[name]`,
                        // every ParamKind::EnterArg in `enter_args[name]`.
                        // Values are wrapped in std::any so the dispatch
                        // reader (which uses std::any_cast<Type>) round-trips
                        // them correctly. cpp_wrap_any_arg promotes string
                        // literals to std::string so any_cast<std::string>
                        // works on the read side.
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    let wrapped = cpp_wrap_any_arg(&p.name);
                                    hsm_init_code.push_str(&format!(
                                        "__compartment->state_args[\"{}\"] = std::any({});\n",
                                        p.name, wrapped
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    let wrapped = cpp_wrap_any_arg(&p.name);
                                    hsm_init_code.push_str(&format!(
                                        "__compartment->enter_args[\"{}\"] = std::any({});\n",
                                        p.name, wrapped
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = std::make_unique<{}>(\"{}\");",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    let wrapped = cpp_wrap_any_arg(&p.name);
                                    compartment_inits.push(format!(
                                        "__compartment->state_args[\"{}\"] = std::any({});",
                                        p.name, wrapped
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    let wrapped = cpp_wrap_any_arg(&p.name);
                                    compartment_inits.push(format!(
                                        "__compartment->enter_args[\"{}\"] = std::any({});",
                                        p.name, wrapped
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Java => {
                    // Java: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "var {} = new {}(\"{}\");\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_var != "null" {
                                hsm_init_code.push_str(&format!(
                                    "{}.parent_compartment = {};\n",
                                    comp_var, prev_comp_var
                                ));
                            }
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Java)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Java)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars.put(\"{}\", {});\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = new {}(\"{}\");\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "this.__compartment.parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args.put(\"{}\", {});",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args.put(\"{}\", {});",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = new {}(\"{}\");\n__next_compartment = null;",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.state_args.put(\"{}\", {});",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.enter_args.put(\"{}\", {});",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Kotlin => {
                    // Kotlin: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "val {} = {}(\"{}\")\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_var != "null" {
                                hsm_init_code.push_str(&format!(
                                    "{}.parent_compartment = {}\n",
                                    comp_var, prev_comp_var
                                ));
                            }
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Kotlin)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Kotlin)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}[\"{}\"] = {}\n",
                                    format!("{}.state_vars", comp_var), var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = {}(\"{}\")\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "this.__compartment.parent_compartment = {}\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = {}(\"{}\")\n__next_compartment = null",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Swift => {
                    // Swift: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "nil".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "let {} = {}(state: \"{}\")\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_var != "nil" {
                                hsm_init_code.push_str(&format!(
                                    "{}.parent_compartment = {}\n",
                                    comp_var, prev_comp_var
                                ));
                            }
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Swift)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Swift)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "self.__compartment = {}(state: \"{}\")\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "self.__compartment.parent_compartment = {}\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("self.__next_compartment = nil");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nself.__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nself.__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = {}(state: \"{}\")\n__next_compartment = nil",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::CSharp => {
                    // C#: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "var {} = new {}(\"{}\");\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_var != "null" {
                                hsm_init_code.push_str(&format!(
                                    "{}.parent_compartment = {};\n",
                                    comp_var, prev_comp_var
                                ));
                            }
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::CSharp)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::CSharp)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {};\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = new {}(\"{}\");\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "this.__compartment.parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = new {}(\"{}\");\n__next_compartment = null;",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.state_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.enter_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Go => {
                    // Go: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "nil".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} := new{}Compartment(\"{}\")\n",
                                comp_var, system.name, ancestor.name
                            ));
                            hsm_init_code.push_str(&format!(
                                "{}.parentCompartment = {}\n",
                                comp_var, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Go)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Go)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.stateVars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "s.__compartment = new{}Compartment(\"{}\")\n",
                            system.name, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "s.__compartment.parentCompartment = {}\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("s.__next_compartment = nil");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\ns.__compartment.stateArgs[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\ns.__compartment.enterArgs[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "s.__compartment = new{}Compartment(\"{}\")\ns.__next_compartment = nil",
                                system.name, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "s.__compartment.stateArgs[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "s.__compartment.enterArgs[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Dart => {
                    // Dart: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "final {} = {}(\"{}\", {});\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, syntax.language)
                                } else {
                                    state_var_init_value(&var.var_type, syntax.language)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {};\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = {}(\"{}\", {});\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "this.__compartment.state_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "this.__compartment.enter_args[\"{}\"] = {};",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::GDScript => {
                    // GDScript: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("# HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "var {} = {}.new(\"{}\", {})\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::GDScript)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::GDScript)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "self.__compartment = {}.new(\"{}\", {})\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("self.__next_compartment = null");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nself.__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nself.__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                // Dynamic languages and remaining: New expression
                // (Lua, Erlang, Kotlin — all routed here)
                TargetLanguage::Python3 | TargetLanguage::TypeScript | TargetLanguage::JavaScript
                    | TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang | TargetLanguage::Kotlin
                    | TargetLanguage::Lua => {
                    body.push(CodegenNode::assign(
                        CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                        CodegenNode::New {
                            class: compartment_class.clone(),
                            args: vec![CodegenNode::string(&first_state.name)],
                        },
                    ));
                    body.push(CodegenNode::assign(
                        CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                        CodegenNode::null(),
                    ));

                    // System state and enter params (Lua path — Lua's
                    // compartment has state_args/enter_args dicts the same
                    // way Python does). Erlang and Kotlin have their own
                    // dispatch routes upstream (Erlang via gen_statem,
                    // Kotlin via the explicit branch above), so for these
                    // the population is a no-op.
                    if matches!(syntax.language, TargetLanguage::Lua) {
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.state_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.enter_args[\"{}\"] = {}",
                                        p.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Graphviz => unreachable!(),
            }

            // Send $> (enter) event via __kernel - language-specific.
            // The enter_args of the start state's compartment carry any
            // header-declared enter params; pass them through so the
            // start state's $>(name: type) handler can read them.
            //
            // C optimization (TODO #41): if the start state has no
            // declared enter params (`$>` handler with empty `params`),
            // skip the indirection and pass NULL for the event's
            // `_parameters`. The dispatch generates no `FrameDict_get`
            // calls in that case, so the NULL is never dereferenced.
            // For systems WITH enter params, behavior is unchanged.
            let start_state_has_enter_params = first_state
                .enter
                .as_ref()
                .map(|e| !e.params.is_empty())
                .unwrap_or(false);
            let init_event_code = match syntax.language {
                TargetLanguage::Python3 => format!(
                    r#"__frame_event = {}("$>", self.__compartment.enter_args)
self.__kernel(__frame_event)"#,
                    event_class
                ),
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!(
                    r#"const __frame_event = new {}("$>", this.__compartment.enter_args);
this.__kernel(__frame_event);"#,
                    event_class
                ),
                TargetLanguage::Rust => format!(
                    r#"let __frame_event = {}::new("$>");
let __ctx = {}FrameContext::new(__frame_event, None);
self._context_stack.push(__ctx);
self.__kernel();
self._context_stack.pop();"#,
                    event_class, system.name
                ),
                TargetLanguage::C => {
                    // Pass the start state's enter_args dict as the event
                    // _parameters so the start state's `$>(name: type)`
                    // enter handler can read header-declared enter params
                    // by name. For systems WITHOUT declared enter params,
                    // pass NULL — the dispatch generates no FrameDict_get
                    // calls so the NULL is never dereferenced, and the
                    // generated code is one indirection lighter at start
                    // time. Behavior is unchanged for systems with enter
                    // params.
                    let parameters_arg = if start_state_has_enter_params {
                        "self->__compartment->enter_args"
                    } else {
                        "NULL"
                    };
                    format!(
                        r#"{}_FrameEvent* __frame_event = {}_FrameEvent_new("$>", {});
{}_kernel(self, __frame_event);
{}_FrameEvent_destroy(__frame_event);"#,
                        system.name, system.name, parameters_arg, system.name, system.name
                    )
                },
                TargetLanguage::Cpp => format!(
                    r#"{}FrameEvent __frame_event("$>");
{}FrameContext __ctx(std::move(__frame_event));
_context_stack.push_back(std::move(__ctx));
__kernel(_context_stack.back()._event);
_context_stack.pop_back();"#,
                    system.name, system.name
                ),
                TargetLanguage::Java => format!(
                    r#"{}FrameEvent __frame_event = new {}FrameEvent("$>");
{}FrameContext __ctx = new {}FrameContext(__frame_event, null);
_context_stack.add(__ctx);
__kernel(_context_stack.get(_context_stack.size() - 1)._event);
_context_stack.remove(_context_stack.size() - 1);"#,
                    system.name, system.name, system.name, system.name
                ),
                TargetLanguage::CSharp => format!(
                    r#"{}FrameEvent __frame_event = new {}FrameEvent("$>");
{}FrameContext __ctx = new {}FrameContext(__frame_event, null);
_context_stack.Add(__ctx);
__kernel(_context_stack[_context_stack.Count - 1]._event);
_context_stack.RemoveAt(_context_stack.Count - 1);"#,
                    system.name, system.name, system.name, system.name
                ),
                TargetLanguage::Go => format!(
                    r#"__frame_event := {}FrameEvent{{_message: "$>", _parameters: nil}}
__ctx := {}FrameContext{{_event: __frame_event, _data: make(map[string]any)}}
s._context_stack = append(s._context_stack, __ctx)
s.__kernel(&s._context_stack[len(s._context_stack)-1]._event)
s._context_stack = s._context_stack[:len(s._context_stack)-1]"#,
                    system.name, system.name
                ),
                TargetLanguage::Kotlin => format!(
                    r#"val __frame_event = {}FrameEvent("$>")
val __ctx = {}FrameContext(__frame_event, null)
_context_stack.add(__ctx)
__kernel(_context_stack[_context_stack.size - 1]._event)
_context_stack.removeAt(_context_stack.size - 1)"#,
                    system.name, system.name
                ),
                TargetLanguage::Swift => format!(
                    r#"let __frame_event = {}FrameEvent(message: "$>")
let __ctx = {}FrameContext(event: __frame_event)
_context_stack.append(__ctx)
__kernel(_context_stack[_context_stack.count - 1]._event)
_context_stack.removeLast()"#,
                    system.name, system.name
                ),
                TargetLanguage::Php => format!(
                    r#"$__frame_event = new {}("$>", $this->__compartment->enter_args);
$__ctx = new {}FrameContext($__frame_event, null);
$this->_context_stack[] = $__ctx;
$this->__kernel($this->_context_stack[count($this->_context_stack) - 1]->_event);
array_pop($this->_context_stack);"#,
                    event_class, system.name
                ),
                TargetLanguage::Ruby => format!(
                    r#"__frame_event = {}.new("$>", @__compartment.enter_args)
__ctx = {}FrameContext.new(__frame_event, nil)
@_context_stack.push(__ctx)
__kernel(@_context_stack[@_context_stack.length - 1]._event)
@_context_stack.pop"#,
                    event_class, system.name
                ),
                TargetLanguage::Lua => format!(
                    r#"local __frame_event = {}.new("$>", self.__compartment.enter_args)
local __ctx = {}FrameContext.new(__frame_event, nil)
self._context_stack[#self._context_stack + 1] = __ctx
self:__kernel(self._context_stack[#self._context_stack]._event)
self._context_stack[#self._context_stack] = nil"#,
                    event_class, system.name
                ),
                TargetLanguage::Dart => format!(
                    "final __frame_event = {}(\"\\$>\", this.__compartment.enter_args);\nfinal __ctx = {}FrameContext(__frame_event, null);\n_context_stack.add(__ctx);\n__kernel(_context_stack[_context_stack.length - 1].event);\n_context_stack.removeLast();",
                    event_class, system.name
                ),
                TargetLanguage::GDScript => format!(
                    r#"var __frame_event = {}.new("$>", self.__compartment.enter_args)
var __ctx = {}FrameContext.new(__frame_event, null)
self._context_stack.append(__ctx)
self.__kernel(self._context_stack[self._context_stack.size() - 1].event)
self._context_stack.pop_back()"#,
                    event_class, system.name
                ),
                TargetLanguage::Erlang => String::new(), // TODO: Erlang gen_statem codegen
                TargetLanguage::Graphviz => unreachable!(),
            };
            body.push(CodegenNode::NativeBlock {
                code: init_event_code,
                span: None,
            });
        }
    }

    // Params from system params
    let params: Vec<Param> = system.params.iter().map(|p| {
        let type_str = type_to_string(&p.param_type);
        let mut param = Param::new(&p.name).with_type(&type_str);
        if let Some(ref def) = p.default {
            param = param.with_default(CodegenNode::Ident(def.clone()));
        }
        param
    }).collect();

    CodegenNode::Constructor {
        params,
        body,
        super_call: None,
    }
}

/// Generate Frame machinery methods
///
/// For Python/TypeScript: Proper Frame runtime with __kernel, __router, __transition
/// For Rust: Simplified implementation (proper runtime in future task)
fn generate_frame_machinery(system: &SystemAst, syntax: &super::backend::ClassSyntax, lang: TargetLanguage) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let compartment_class = format!("{}Compartment", system.name);
    let event_class = format!("{}FrameEvent", system.name);

    match lang {
        TargetLanguage::Python3 => {
            // __kernel method - the main event processing loop
            // Routes event to current state, then processes any pending transition
            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"# Route event to current state
self.__router(__e)
# Process any pending transition
while self.__next_compartment is not None:
    next_compartment = self.__next_compartment
    self.__next_compartment = None
    # Exit current state
    exit_event = {}("<$", self.__compartment.exit_args)
    self.__router(exit_event)
    # Switch to new compartment
    self.__compartment = next_compartment
    # Enter new state (or forward event)
    if next_compartment.forward_event is None:
        enter_event = {}("$>", self.__compartment.enter_args)
        self.__router(enter_event)
    else:
        # Forward event to new state
        forward_event = next_compartment.forward_event
        next_compartment.forward_event = None
        if forward_event._message == "$>":
            # Forwarding enter event - just send it
            self.__router(forward_event)
        else:
            # Forwarding other event - send $> first, then forward
            enter_event = {}("$>", self.__compartment.enter_args)
            self.__router(enter_event)
            self.__router(forward_event)"#,
                        event_class, event_class, event_class
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router method - dispatches events to state methods
            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: r#"state_name = self.__compartment.state
handler_name = f"_state_{state_name}"
handler = getattr(self, handler_name, None)
if handler:
    handler(__e)"#.to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition method - caches next compartment (deferred transition)
            // Does NOT execute transition - __kernel does that after handler returns
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next_compartment")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "self.__next_compartment = next_compartment".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            // __kernel method - the main event processing loop
            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"// Route event to current state
this.__router(__e);
// Process any pending transition
while (this.__next_compartment !== null) {{
    const next_compartment = this.__next_compartment;
    this.__next_compartment = null;
    // Exit current state
    const exit_event = new {}("<$", this.__compartment.exit_args);
    this.__router(exit_event);
    // Switch to new compartment
    this.__compartment = next_compartment;
    // Enter new state (or forward event)
    if (next_compartment.forward_event === null) {{
        const enter_event = new {}("$>", this.__compartment.enter_args);
        this.__router(enter_event);
    }} else {{
        // Forward event to new state
        const forward_event = next_compartment.forward_event;
        next_compartment.forward_event = null;
        if (forward_event._message === "$>") {{
            // Forwarding enter event - just send it
            this.__router(forward_event);
        }} else {{
            // Forwarding other event - send $> first, then forward
            const enter_event = new {}("$>", this.__compartment.enter_args);
            this.__router(enter_event);
            this.__router(forward_event);
        }}
    }}
}}"#,
                        event_class, event_class, event_class
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router method - dispatches events to state methods
            // TypeScript uses `(this as any)` for dynamic dispatch; JS uses `this` directly
            let router_code = if matches!(syntax.language, TargetLanguage::TypeScript) {
                r#"const state_name = this.__compartment.state;
const handler_name = `_state_${state_name}`;
const handler = (this as any)[handler_name];
if (handler) {
    handler.call(this, __e);
}"#.to_string()
            } else {
                r#"const state_name = this.__compartment.state;
const handler_name = `_state_${state_name}`;
const handler = this[handler_name];
if (handler) {
    handler.call(this, __e);
}"#.to_string()
            };
            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: router_code,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition method - caches next compartment (deferred transition)
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next_compartment").with_type(&compartment_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "this.__next_compartment = next_compartment;".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Php => {
            // PHP: __kernel method - the main event processing loop
            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"// Route event to current state
$this->__router($__e);
// Process any pending transition
while ($this->__next_compartment !== null) {{
    $next_compartment = $this->__next_compartment;
    $this->__next_compartment = null;
    // Exit current state
    $exit_event = new {}("<$", $this->__compartment->exit_args);
    $this->__router($exit_event);
    // Switch to new compartment
    $this->__compartment = $next_compartment;
    // Enter new state (or forward event)
    if ($next_compartment->forward_event === null) {{
        $enter_event = new {}("$>", $this->__compartment->enter_args);
        $this->__router($enter_event);
    }} else {{
        // Forward event to new state
        $forward_event = $next_compartment->forward_event;
        $next_compartment->forward_event = null;
        if ($forward_event->_message === "$>") {{
            // Forwarding enter event - just send it
            $this->__router($forward_event);
        }} else {{
            // Forwarding other event - send $> first, then forward
            $enter_event = new {}("$>", $this->__compartment->enter_args);
            $this->__router($enter_event);
            $this->__router($forward_event);
        }}
    }}
}}"#,
                        event_class, event_class, event_class
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // PHP: __router method - dispatches events to state methods
            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: r#"$state_name = $this->__compartment->state;
$handler_name = "_state_" . $state_name;
if (method_exists($this, $handler_name)) {
    $this->$handler_name($__e);
}"#.to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // PHP: __transition method - caches next compartment (deferred transition)
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next_compartment")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "$this->__next_compartment = $next_compartment;".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Ruby => {
            methods.push(CodegenNode::Method { name: "__kernel".to_string(), params: vec![Param::new("__e")], return_type: None, body: vec![CodegenNode::NativeBlock { code: format!("# Route event to current state\n__router(__e)\nwhile @__next_compartment != nil\n    next_compartment = @__next_compartment\n    @__next_compartment = nil\n    exit_event = {0}.new(\"<$\", @__compartment.exit_args)\n    __router(exit_event)\n    @__compartment = next_compartment\n    if next_compartment.forward_event == nil\n        enter_event = {0}.new(\"$>\", @__compartment.enter_args)\n        __router(enter_event)\n    else\n        forward_event = next_compartment.forward_event\n        next_compartment.forward_event = nil\n        if forward_event._message == \"$>\"\n            __router(forward_event)\n        else\n            enter_event = {0}.new(\"$>\", @__compartment.enter_args)\n            __router(enter_event)\n            __router(forward_event)\n        end\n    end\nend", event_class), span: None }], is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![] });
            methods.push(CodegenNode::Method { name: "__router".to_string(), params: vec![Param::new("__e")], return_type: None, body: vec![CodegenNode::NativeBlock { code: "state_name = @__compartment.state\nhandler_name = \"_state_#{state_name}\"\nif respond_to?(handler_name, true)\n    send(handler_name, __e)\nend".to_string(), span: None }], is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![] });
            methods.push(CodegenNode::Method { name: "__transition".to_string(), params: vec![Param::new("next_compartment")], return_type: None, body: vec![CodegenNode::NativeBlock { code: "@__next_compartment = next_compartment".to_string(), span: None }], is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![] });
        }
        TargetLanguage::Rust => {
            // Rust: Full kernel/router/transition pattern matching Python/TypeScript

            // __kernel method - the main event processing loop with deferred transitions
            // Gets event from context stack (no parameter needed)
            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"// Clone event from context stack (needed for borrow checker)
let __e = self._context_stack.last().unwrap().event.clone();
// Route event to current state
self.__router(&__e);
// Process any pending transition
while self.__next_compartment.is_some() {{
    let next_compartment = self.__next_compartment.take().unwrap();
    // Exit current state (with exit_args from current compartment)
    let exit_event = {}::new_with_params("<$", &self.__compartment.exit_args);
    self.__router(&exit_event);
    // Switch to new compartment
    self.__compartment = next_compartment;
    // Enter new state (or forward event)
    if self.__compartment.forward_event.is_none() {{
        let enter_event = {}::new_with_params("$>", &self.__compartment.enter_args);
        self.__router(&enter_event);
    }} else {{
        // Forward event to new state
        let forward_event = self.__compartment.forward_event.take().unwrap();
        if forward_event.message == "$>" {{
            // Forwarding enter event - just send it
            self.__router(&forward_event);
        }} else {{
            // Forwarding other event - send $> first, then forward
            let enter_event = {}::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
            self.__router(&forward_event);
        }}
    }}
}}"#,
                        event_class, event_class, event_class
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router method - dispatches events to state dispatch methods
            let router_code = generate_rust_router_dispatch(system);
            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&format!("&{}", event_class))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: router_code,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition method - caches next compartment (deferred transition)
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next_compartment").with_type(&compartment_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "self.__next_compartment = Some(next_compartment);".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::C => {
            // C: Full kernel/router/transition pattern with string comparison dispatch
            let sys = &system.name;

            // __kernel method - the main event processing loop
            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"// Route event to current state
{sys}_router(self, __e);
// Process any pending transition
while (self->__next_compartment != NULL) {{
    {sys}_Compartment* next_compartment = self->__next_compartment;
    self->__next_compartment = NULL;
    // Exit current state (with exit_args from current compartment)
    {sys}_FrameEvent* exit_event = {sys}_FrameEvent_new("<$", self->__compartment->exit_args);
    {sys}_router(self, exit_event);
    {sys}_FrameEvent_destroy(exit_event);
    // Switch to new compartment
    {sys}_Compartment_destroy(self->__compartment);
    self->__compartment = next_compartment;
    // Enter new state (or forward event)
    if (next_compartment->forward_event == NULL) {{
        {sys}_FrameEvent* enter_event = {sys}_FrameEvent_new("$>", self->__compartment->enter_args);
        {sys}_router(self, enter_event);
        {sys}_FrameEvent_destroy(enter_event);
    }} else {{
        // Forward event to new state
        // Note: forward_event is a borrowed pointer to the caller's __e, do NOT destroy it
        {sys}_FrameEvent* forward_event = next_compartment->forward_event;
        next_compartment->forward_event = NULL;
        if (strcmp(forward_event->_message, "$>") == 0) {{
            // Forwarding enter event - just send it
            {sys}_router(self, forward_event);
        }} else {{
            // Forwarding other event - send $> first, then forward
            {sys}_FrameEvent* enter_event = {sys}_FrameEvent_new("$>", self->__compartment->enter_args);
            {sys}_router(self, enter_event);
            {sys}_FrameEvent_destroy(enter_event);
            {sys}_router(self, forward_event);
        }}
        // Do NOT destroy forward_event - it's owned by the interface method caller
    }}
}}"#,
                        sys = sys
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router method - dispatches events to state handler functions
            let router_code = generate_c_router_dispatch(system);
            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: router_code,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition method - caches next compartment (deferred transition)
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next_compartment").with_type(&format!("{}_Compartment*", sys))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "self->__next_compartment = next_compartment;".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // destroy method - cleanup system resources
            methods.push(CodegenNode::Method {
                name: "destroy".to_string(),
                params: vec![],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"if (self->__compartment) {sys}_Compartment_destroy(self->__compartment);
if (self->_state_stack) {sys}_FrameVec_destroy(self->_state_stack);
if (self->_context_stack) {sys}_FrameVec_destroy(self->_context_stack);
free(self);"#,
                        sys = sys
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Cpp => {
            let states: Vec<&str> = system.machine.as_ref()
                .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
                .unwrap_or_default();

            // __kernel
            let mut kernel_code = String::new();
            kernel_code.push_str("__router(__e);\n");
            kernel_code.push_str("while (__next_compartment) {\n");
            kernel_code.push_str("    auto next_compartment = std::move(__next_compartment);\n");
            kernel_code.push_str(&format!("    {} exit_event(\"<$\");\n", event_class));
            kernel_code.push_str("    __router(exit_event);\n");
            kernel_code.push_str("    __compartment = std::move(next_compartment);\n");
            kernel_code.push_str("    if (!__compartment->forward_event) {\n");
            kernel_code.push_str(&format!("        {} enter_event(\"$>\");\n", event_class));
            kernel_code.push_str("        __router(enter_event);\n");
            kernel_code.push_str("    } else {\n");
            kernel_code.push_str("        auto forward_event = std::move(__compartment->forward_event);\n");
            kernel_code.push_str("        if (forward_event->_message == \"$>\") {\n");
            kernel_code.push_str("            __router(*forward_event);\n");
            kernel_code.push_str("        } else {\n");
            kernel_code.push_str(&format!("            {} enter_event(\"$>\");\n", event_class));
            kernel_code.push_str("            __router(enter_event);\n");
            kernel_code.push_str("            __router(*forward_event);\n");
            kernel_code.push_str("        }\n");
            kernel_code.push_str("    }\n");
            kernel_code.push_str("}");

            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: kernel_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router
            let mut router_code = String::new();
            router_code.push_str("const auto& state_name = __compartment->state;\n");
            for (i, state) in states.iter().enumerate() {
                let prefix = if i == 0 { "if" } else { "} else if" };
                router_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
                router_code.push_str(&format!("    _state_{}(__e);\n", state));
            }
            if !states.is_empty() {
                router_code.push_str("}");
            }

            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: router_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next").with_type(&format!("std::unique_ptr<{}>", compartment_class))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "__next_compartment = std::move(next);".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Java => {
            let states: Vec<&str> = system.machine.as_ref()
                .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
                .unwrap_or_default();

            // __kernel
            let mut kernel_code = String::new();
            kernel_code.push_str("__router(__e);\n");
            kernel_code.push_str("while (__next_compartment != null) {\n");
            kernel_code.push_str("    var next_compartment = __next_compartment;\n");
            kernel_code.push_str("    __next_compartment = null;\n");
            kernel_code.push_str(&format!("    {} exit_event = new {}(\"<$\");\n", event_class, event_class));
            kernel_code.push_str("    __router(exit_event);\n");
            kernel_code.push_str("    __compartment = next_compartment;\n");
            kernel_code.push_str("    if (__compartment.forward_event == null) {\n");
            kernel_code.push_str(&format!("        {} enter_event = new {}(\"$>\");\n", event_class, event_class));
            kernel_code.push_str("        __router(enter_event);\n");
            kernel_code.push_str("    } else {\n");
            kernel_code.push_str("        var forward_event = __compartment.forward_event;\n");
            kernel_code.push_str("        __compartment.forward_event = null;\n");
            kernel_code.push_str("        if (forward_event._message.equals(\"$>\")) {\n");
            kernel_code.push_str("            __router(forward_event);\n");
            kernel_code.push_str("        } else {\n");
            kernel_code.push_str(&format!("            {} enter_event = new {}(\"$>\");\n", event_class, event_class));
            kernel_code.push_str("            __router(enter_event);\n");
            kernel_code.push_str("            __router(forward_event);\n");
            kernel_code.push_str("        }\n");
            kernel_code.push_str("    }\n");
            kernel_code.push_str("}");

            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: kernel_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router - if/else if chain with .equals()
            let mut router_code = String::new();
            router_code.push_str("String state_name = __compartment.state;\n");
            for (i, state) in states.iter().enumerate() {
                let prefix = if i == 0 { "if" } else { "} else if" };
                router_code.push_str(&format!("{} (state_name.equals(\"{}\")) {{\n", prefix, state));
                router_code.push_str(&format!("    _state_{}(__e);\n", state));
            }
            if !states.is_empty() {
                router_code.push_str("}");
            }

            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: router_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next").with_type(&compartment_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "__next_compartment = next;".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Kotlin => {
            let states: Vec<&str> = system.machine.as_ref()
                .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
                .unwrap_or_default();

            // __kernel — Kotlin: no semicolons, no `new`, `==` instead of `.equals()`
            let mut kernel_code = String::new();
            kernel_code.push_str("__router(__e)\n");
            kernel_code.push_str("while (__next_compartment != null) {\n");
            kernel_code.push_str("    val next_compartment = __next_compartment!!\n");
            kernel_code.push_str("    __next_compartment = null\n");
            kernel_code.push_str(&format!("    val exit_event = {}(\"<$\")\n", event_class));
            kernel_code.push_str("    __router(exit_event)\n");
            kernel_code.push_str("    __compartment = next_compartment\n");
            kernel_code.push_str("    if (__compartment.forward_event == null) {\n");
            kernel_code.push_str(&format!("        val enter_event = {}(\"$>\")\n", event_class));
            kernel_code.push_str("        __router(enter_event)\n");
            kernel_code.push_str("    } else {\n");
            kernel_code.push_str("        val forward_event = __compartment.forward_event!!\n");
            kernel_code.push_str("        __compartment.forward_event = null\n");
            kernel_code.push_str("        if (forward_event._message == \"$>\") {\n");
            kernel_code.push_str("            __router(forward_event)\n");
            kernel_code.push_str("        } else {\n");
            kernel_code.push_str(&format!("            val enter_event = {}(\"$>\")\n", event_class));
            kernel_code.push_str("            __router(enter_event)\n");
            kernel_code.push_str("            __router(forward_event)\n");
            kernel_code.push_str("        }\n");
            kernel_code.push_str("    }\n");
            kernel_code.push_str("}");

            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: kernel_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router — Kotlin: when expression with == comparison
            let mut router_code = String::new();
            router_code.push_str("val state_name = __compartment.state\n");
            for (i, state) in states.iter().enumerate() {
                let prefix = if i == 0 { "if" } else { "} else if" };
                router_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
                router_code.push_str(&format!("    _state_{}(__e)\n", state));
            }
            if !states.is_empty() {
                router_code.push_str("}");
            }

            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: router_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next").with_type(&compartment_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "__next_compartment = next".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Swift => {
            let states: Vec<&str> = system.machine.as_ref()
                .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
                .unwrap_or_default();

            // __kernel — Swift: no semicolons, no `new`, `!=` for comparison
            let mut kernel_code = String::new();
            kernel_code.push_str("__router(__e)\n");
            kernel_code.push_str("while __next_compartment != nil {\n");
            kernel_code.push_str("    let next_compartment = __next_compartment!\n");
            kernel_code.push_str("    __next_compartment = nil\n");
            kernel_code.push_str(&format!("    let exit_event = {}(message: \"<$\")\n", event_class));
            kernel_code.push_str("    __router(exit_event)\n");
            kernel_code.push_str("    __compartment = next_compartment\n");
            kernel_code.push_str("    if __compartment.forward_event == nil {\n");
            kernel_code.push_str(&format!("        let enter_event = {}(message: \"$>\")\n", event_class));
            kernel_code.push_str("        __router(enter_event)\n");
            kernel_code.push_str("    } else {\n");
            kernel_code.push_str("        let forward_event = __compartment.forward_event!\n");
            kernel_code.push_str("        __compartment.forward_event = nil\n");
            kernel_code.push_str("        if forward_event._message == \"$>\" {\n");
            kernel_code.push_str("            __router(forward_event)\n");
            kernel_code.push_str("        } else {\n");
            kernel_code.push_str(&format!("            let enter_event = {}(message: \"$>\")\n", event_class));
            kernel_code.push_str("            __router(enter_event)\n");
            kernel_code.push_str("            __router(forward_event)\n");
            kernel_code.push_str("        }\n");
            kernel_code.push_str("    }\n");
            kernel_code.push_str("}");

            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: kernel_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router — Swift: if/else chain with == comparison
            let mut router_code = String::new();
            router_code.push_str("let state_name = __compartment.state\n");
            for (i, state) in states.iter().enumerate() {
                let prefix = if i == 0 { "if" } else { "} else if" };
                router_code.push_str(&format!("{} state_name == \"{}\" {{\n", prefix, state));
                router_code.push_str(&format!("    _state_{}(__e)\n", state));
            }
            if !states.is_empty() {
                router_code.push_str("}");
            }

            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: router_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next").with_type(&compartment_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "__next_compartment = next".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::CSharp => {
            let states: Vec<&str> = system.machine.as_ref()
                .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
                .unwrap_or_default();

            // __kernel
            let mut kernel_code = String::new();
            kernel_code.push_str("__router(__e);\n");
            kernel_code.push_str("while (__next_compartment != null) {\n");
            kernel_code.push_str("    var next_compartment = __next_compartment;\n");
            kernel_code.push_str("    __next_compartment = null;\n");
            kernel_code.push_str(&format!("    {} exit_event = new {}(\"<$\");\n", event_class, event_class));
            kernel_code.push_str("    __router(exit_event);\n");
            kernel_code.push_str("    __compartment = next_compartment;\n");
            kernel_code.push_str("    if (__compartment.forward_event == null) {\n");
            kernel_code.push_str(&format!("        {} enter_event = new {}(\"$>\");\n", event_class, event_class));
            kernel_code.push_str("        __router(enter_event);\n");
            kernel_code.push_str("    } else {\n");
            kernel_code.push_str("        var forward_event = __compartment.forward_event;\n");
            kernel_code.push_str("        __compartment.forward_event = null;\n");
            kernel_code.push_str("        if (forward_event._message == \"$>\") {\n");
            kernel_code.push_str("            __router(forward_event);\n");
            kernel_code.push_str("        } else {\n");
            kernel_code.push_str(&format!("            {} enter_event = new {}(\"$>\");\n", event_class, event_class));
            kernel_code.push_str("            __router(enter_event);\n");
            kernel_code.push_str("            __router(forward_event);\n");
            kernel_code.push_str("        }\n");
            kernel_code.push_str("    }\n");
            kernel_code.push_str("}");

            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: kernel_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router - if/else if chain with ==
            let mut router_code = String::new();
            router_code.push_str("string state_name = __compartment.state;\n");
            for (i, state) in states.iter().enumerate() {
                let prefix = if i == 0 { "if" } else { "} else if" };
                router_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
                router_code.push_str(&format!("    _state_{}(__e);\n", state));
            }
            if !states.is_empty() {
                router_code.push_str("}");
            }

            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: router_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next").with_type(&compartment_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "__next_compartment = next;".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Go => {
            let states: Vec<&str> = system.machine.as_ref()
                .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
                .unwrap_or_default();

            // __kernel
            let mut kernel_code = String::new();
            kernel_code.push_str("s.__router(__e)\n");
            kernel_code.push_str("for s.__next_compartment != nil {\n");
            kernel_code.push_str("    next_compartment := s.__next_compartment\n");
            kernel_code.push_str("    s.__next_compartment = nil\n");
            kernel_code.push_str(&format!("    exit_event := &{}FrameEvent{{_message: \"<$\", _parameters: s.__compartment.exitArgs}}\n", system.name));
            kernel_code.push_str("    s.__router(exit_event)\n");
            kernel_code.push_str("    s.__compartment = next_compartment\n");
            kernel_code.push_str("    if s.__compartment.forwardEvent == nil {\n");
            kernel_code.push_str(&format!("        enter_event := &{}FrameEvent{{_message: \"$>\", _parameters: s.__compartment.enterArgs}}\n", system.name));
            kernel_code.push_str("        s.__router(enter_event)\n");
            kernel_code.push_str("    } else {\n");
            kernel_code.push_str("        forward_event := s.__compartment.forwardEvent\n");
            kernel_code.push_str("        s.__compartment.forwardEvent = nil\n");
            kernel_code.push_str("        if forward_event._message == \"$>\" {\n");
            kernel_code.push_str("            s.__router(forward_event)\n");
            kernel_code.push_str("        } else {\n");
            kernel_code.push_str(&format!("            enter_event := &{}FrameEvent{{_message: \"$>\", _parameters: s.__compartment.enterArgs}}\n", system.name));
            kernel_code.push_str("            s.__router(enter_event)\n");
            kernel_code.push_str("            s.__router(forward_event)\n");
            kernel_code.push_str("        }\n");
            kernel_code.push_str("    }\n");
            kernel_code.push_str("}");

            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&format!("*{}FrameEvent", system.name))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: kernel_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router - switch on state string
            let mut router_code = String::new();
            router_code.push_str("switch s.__compartment.state {\n");
            for state in &states {
                router_code.push_str(&format!("case \"{}\":\n", state));
                router_code.push_str(&format!("    s._state_{}(__e)\n", state));
            }
            router_code.push_str("}");

            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e").with_type(&format!("*{}FrameEvent", system.name))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: router_code, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next").with_type(&format!("*{}Compartment", system.name))],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "s.__next_compartment = next".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Erlang => {
            // gen_statem: kernel/router/transition are built into OTP — no custom methods needed
        }
        TargetLanguage::Lua => {
            // __kernel method
            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"-- Route event to current state
self:__router(__e)
-- Process any pending transition
while self.__next_compartment ~= nil do
    local next_compartment = self.__next_compartment
    self.__next_compartment = nil
    -- Exit current state
    local exit_event = {}.new("<$", self.__compartment.exit_args)
    self:__router(exit_event)
    -- Switch to new compartment
    self.__compartment = next_compartment
    -- Enter new state (or forward event)
    if next_compartment.forward_event == nil then
        local enter_event = {}.new("$>", self.__compartment.enter_args)
        self:__router(enter_event)
    else
        local forward_event = next_compartment.forward_event
        next_compartment.forward_event = nil
        if forward_event._message == "$>" then
            self:__router(forward_event)
        else
            local enter_event = {}.new("$>", self.__compartment.enter_args)
            self:__router(enter_event)
            self:__router(forward_event)
        end
    end
end"#,
                        event_class, event_class, event_class
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router method — dispatches to state handler methods
            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: r#"local state_name = self.__compartment.state
local handler = self["_state_" .. state_name]
if handler then
    handler(self, __e)
end"#.to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition method — caches next compartment (deferred)
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next_compartment")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "self.__next_compartment = next_compartment".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Dart => {
            // __kernel method
            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e").with_type(&event_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"// Route event to current state
__router(__e);
// Process any pending transition
while (__next_compartment != null) {{
    final next_compartment = __next_compartment!;
    __next_compartment = null;
    // Exit current state
    final exit_event = {}("<\$", __compartment.exit_args);
    __router(exit_event);
    // Switch to new compartment
    __compartment = next_compartment;
    // Enter new state (or forward event)
    if (next_compartment.forward_event == null) {{
        final enter_event = {}("\$>", __compartment.enter_args);
        __router(enter_event);
    }} else {{
        final forward_event = next_compartment.forward_event!;
        next_compartment.forward_event = null;
        if (forward_event._message == "\$>") {{
            __router(forward_event);
        }} else {{
            final enter_event = {}("\$>", __compartment.enter_args);
            __router(enter_event);
            __router(forward_event);
        }}
    }}
}}"#,
                        event_class, event_class, event_class
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router method — use switch dispatch on state name
            {
                let mut router_code = String::from("switch (__compartment.state) {\n");
                if let Some(ref machine) = system.machine {
                    for state in &machine.states {
                        router_code.push_str(&format!("    case \"{}\":\n", state.name));
                        router_code.push_str(&format!("        _state_{}(__e);\n", state.name));
                        router_code.push_str("        break;\n");
                    }
                }
                router_code.push_str("}");
                methods.push(CodegenNode::Method {
                    name: "__router".to_string(),
                    params: vec![Param::new("__e").with_type(&event_class)],
                    return_type: None,
                    body: vec![CodegenNode::NativeBlock {
                        code: router_code,
                        span: None,
                    }],
                    is_async: false,
                    is_static: false,
                    visibility: Visibility::Private,
                    decorators: vec![],
                });
            }

            // __transition method
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next_compartment").with_type(&compartment_class)],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "__next_compartment = next_compartment;".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::GDScript => {
            // __kernel method
            methods.push(CodegenNode::Method {
                name: "__kernel".to_string(),
                params: vec![Param::new("__e")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: format!(
                        r#"# Route event to current state
self.__router(__e)
# Process any pending transition
while self.__next_compartment != null:
    var next_compartment = self.__next_compartment
    self.__next_compartment = null
    # Exit current state
    var exit_event = {}.new("<$", self.__compartment.exit_args)
    self.__router(exit_event)
    # Switch to new compartment
    self.__compartment = next_compartment
    # Enter new state (or forward event)
    if next_compartment.forward_event == null:
        var enter_event = {}.new("$>", self.__compartment.enter_args)
        self.__router(enter_event)
    else:
        # Forward event to new state
        var forward_event = next_compartment.forward_event
        next_compartment.forward_event = null
        if forward_event._message == "$>":
            self.__router(forward_event)
        else:
            var enter_event = {}.new("$>", self.__compartment.enter_args)
            self.__router(enter_event)
            self.__router(forward_event)"#,
                        event_class, event_class, event_class
                    ),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __router method
            methods.push(CodegenNode::Method {
                name: "__router".to_string(),
                params: vec![Param::new("__e")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: r#"var state_name = self.__compartment.state
var handler_name = "_state_" + state_name
if self.has_method(handler_name):
    self.call(handler_name, __e)"#.to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __transition method
            methods.push(CodegenNode::Method {
                name: "__transition".to_string(),
                params: vec![Param::new("next_compartment")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: "self.__next_compartment = next_compartment".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });
        }
        TargetLanguage::Graphviz => unreachable!(),
    }

    // NOTE: _change_state method removed (->> operator is not supported)
    // The ->> syntax should not compile in V4

    // NOTE: Rust now uses kernel pattern like Python/TypeScript
    // $>/$< events are handled via _state_X dispatch methods, not _enter/_exit dispatchers

    // Generate __push_transition method for Rust when there's a machine
    // Uses mem::replace to move the current compartment to the stack (no clone)
    if matches!(lang, TargetLanguage::Rust) && system.machine.is_some() {
        methods.push(generate_rust_push_transition(system));
    }

    methods
}
fn generate_rust_push_transition(system: &SystemAst) -> CodegenNode {
    let system_name = &system.name;
    let event_class = format!("{}FrameEvent", system_name);
    let compartment_class = format!("{}Compartment", system_name);

    let code = format!(
        r#"// Exit current state (old compartment still in place for routing)
let exit_event = {event_class}::new_with_params("<$", &self.__compartment.exit_args);
self.__router(&exit_event);
// Swap: old compartment moves to stack, new takes its place
let old = std::mem::replace(&mut self.__compartment, new_compartment);
self._state_stack.push(old);
// Enter new state (or forward event) — matches kernel logic
if self.__compartment.forward_event.is_none() {{
    let enter_event = {event_class}::new_with_params("$>", &self.__compartment.enter_args);
    self.__router(&enter_event);
}} else {{
    let forward_event = self.__compartment.forward_event.take().unwrap();
    if forward_event.message == "$>" {{
        self.__router(&forward_event);
    }} else {{
        let enter_event = {event_class}::new_with_params("$>", &self.__compartment.enter_args);
        self.__router(&enter_event);
        self.__router(&forward_event);
    }}
}}"#,
        event_class = event_class
    );

    CodegenNode::Method {
        name: "__push_transition".to_string(),
        params: vec![Param::new("new_compartment").with_type(&compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock { code, span: None }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Generate Rust router dispatch match statement
///
/// Routes events to _state_X methods based on current compartment state
fn generate_rust_router_dispatch(system: &SystemAst) -> String {
    let mut code = String::new();
    code.push_str("match self.__compartment.state.as_str() {\n");

    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            code.push_str(&format!(
                "    \"{}\" => self._state_{}(__e),\n",
                state.name, state.name
            ));
        }
    }

    code.push_str("    _ => {}\n");
    code.push_str("}");
    code
}

/// Generate C router dispatch using if-else chain with strcmp
fn generate_c_router_dispatch(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();
    code.push_str("const char* state_name = self->__compartment->state;\n");

    if let Some(ref machine) = system.machine {
        for (i, state) in machine.states.iter().enumerate() {
            let cond = if i == 0 { "if" } else { "} else if" };
            code.push_str(&format!(
                "{} (strcmp(state_name, \"{}\") == 0) {{\n    {}_state_{}(self, __e);\n",
                cond, state.name, sys, state.name
            ));
        }
        if !machine.states.is_empty() {
            code.push_str("}");
        }
    }

    code
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::frame_ast::{SystemAst, DomainVar, Type, Expression, Literal, Span};
    use crate::frame_c::visitors::TargetLanguage;
use crate::frame_c::compiler::codegen::codegen_utils::{
    HandlerContext, expression_to_string, type_to_string, state_var_init_value,
    convert_expression, convert_literal, extract_type_from_raw_domain,
    is_int_type, is_float_type, is_bool_type, is_string_type,
    to_snake_case, cpp_map_type, cpp_wrap_any_arg, java_map_type,
    kotlin_map_type, swift_map_type, csharp_map_type, go_map_type, type_to_cpp_string,
};

    fn create_test_system() -> SystemAst {
        SystemAst::new("TestSystem".to_string(), Span::new(0, 0))
    }

    #[test]
    fn test_generate_simple_system() {
        let system = create_test_system();
        let arcanum = Arcanum::new();
        // Empty source since test system has no actions/operations with native code
        let source = b"";
        let node = generate_system(&system, &arcanum, TargetLanguage::Python3, source);

        match node {
            CodegenNode::Class { name, .. } => {
                assert_eq!(name, "TestSystem");
            }
            _ => panic!("Expected Class node"),
        }
    }

    #[test]
    fn test_generate_fields() {
        let mut system = create_test_system();
        system.domain.push(DomainVar {
            name: "counter".to_string(),
            var_type: Type::Custom("int".into()),
            initializer_text: Some("0".to_string()),
            initializer: Some(Expression::Literal(Literal::Int(0))),
            is_frame: false,
            raw_code: None,
            span: Span::new(0, 0),
        });

        let syntax = super::super::backend::ClassSyntax::python();
        let fields = generate_fields(&system, &syntax);

        // Should have _state, _state_stack, _state_context, and counter
        assert!(fields.len() >= 4);
        assert!(fields.iter().any(|f| f.name == "counter"));
    }

    #[test]
    fn test_generate_constructor() {
        let system = create_test_system();
        let syntax = super::super::backend::ClassSyntax::python();
        let constructor = generate_constructor(&system, &syntax);

        match constructor {
            CodegenNode::Constructor { body, .. } => {
                assert!(!body.is_empty());
            }
            _ => panic!("Expected Constructor node"),
        }
    }
}

/// Expand `@@SystemName(args)` in domain variable initializers to native constructors.
/// This handles the same pattern as the assembler's tagged instantiation expansion,
/// but for domain code that is emitted during codegen (before the assembler runs).
fn expand_tagged_in_domain(raw_code: &str, lang: TargetLanguage) -> String {
    let bytes = raw_code.as_bytes();
    let mut result = String::new();
    let mut i = 0;

    while i < bytes.len() {
        // Look for @@ followed by uppercase letter
        if i + 2 < bytes.len() && bytes[i] == b'@' && bytes[i + 1] == b'@'
            && i + 2 < bytes.len() && bytes[i + 2].is_ascii_uppercase()
        {
            let start = i;
            i += 2;
            let name_start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = std::str::from_utf8(&bytes[name_start..i]).unwrap_or("");

            if i < bytes.len() && bytes[i] == b'(' {
                // Find matching close paren
                let args_start = i + 1;
                let mut depth = 1;
                i += 1;
                while i < bytes.len() && depth > 0 {
                    match bytes[i] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        b'"' => {
                            i += 1;
                            while i < bytes.len() && bytes[i] != b'"' {
                                if bytes[i] == b'\\' { i += 1; }
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    if depth > 0 { i += 1; }
                }
                if depth == 0 {
                    let args = std::str::from_utf8(&bytes[args_start..i]).unwrap_or("");
                    i += 1; // skip closing )
                    // Generate native constructor
                    let constructor = match lang {
                        TargetLanguage::Python3 | TargetLanguage::GDScript => format!("{}({})", name, args),
                        TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Cpp
                            | TargetLanguage::Java | TargetLanguage::CSharp | TargetLanguage::Dart
                            | TargetLanguage::Kotlin | TargetLanguage::Php => format!("new {}({})", name, args),
                        TargetLanguage::Rust => format!("{}::new({})", name, args),
                        TargetLanguage::C => format!("{}_new({})", name, args),
                        TargetLanguage::Go => format!("New{}({})", name, args),
                        TargetLanguage::Swift => format!("{}({})", name, args),
                        TargetLanguage::Ruby => format!("{}.new({})", name, args),
                        TargetLanguage::Lua => format!("{}.new({})", name, args),
                        TargetLanguage::Erlang => format!("{}:start_link({})", to_snake_case_simple(name), args),
                        TargetLanguage::Graphviz => name.to_string(),
                    };
                    result.push_str(&constructor);
                    continue;
                }
            }
            // Not a valid pattern — copy original
            for b in &bytes[start..i] {
                result.push(*b as char);
            }
            continue;
        }

        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Simple snake_case conversion for Erlang module names
fn to_snake_case_simple(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 { result.push('_'); }
            result.push(c.to_lowercase().next().unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}
