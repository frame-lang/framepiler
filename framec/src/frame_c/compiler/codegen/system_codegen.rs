//! System Code Generation from Frame AST
//!
//! This module transforms Frame AST (SystemAst) into CodegenNode for emission
//! by language-specific backends.
//!
//! Uses the "oceans model" - native code is preserved exactly, Frame segments
//! are replaced with generated code using the splicer.

mod async_wrap;
mod expand_system;
mod factory;
mod fields;
mod word_util;

pub(crate) use async_wrap::make_system_async;
pub(crate) use expand_system::expand_system_instantiation_in_domain;
pub(crate) use fields::generate_fields;
pub(crate) use word_util::init_references_param;
use async_wrap::make_java_interface_async;
use factory::{
    generate_js_static_factory_alias, generate_python_factory_alias,
    generate_static_factory_alias, params_arg_list,
};
use word_util::prefix_php_vars;

use super::ast::*;
use super::backend::get_backend;
use super::codegen_utils::{
    convert_expression, convert_literal, cpp_map_type, cpp_wrap_any_arg, csharp_map_type,
    expression_to_string, go_map_type, java_map_type, kotlin_map_type, state_var_init_value,
    swift_map_type, to_snake_case, type_to_cpp_string, type_to_string, HandlerContext,
};
use super::frame_expansion::{generate_frame_expansion, get_native_scanner, normalize_indentation};
use super::interface_gen::{
    generate_action, generate_interface_wrappers, generate_operation, generate_persistence_methods,
};
use super::state_dispatch::{generate_handler_from_arcanum, generate_state_handlers_via_arcanum};
use crate::frame_c::compiler::arcanum::{Arcanum, HandlerEntry};
use crate::frame_c::compiler::frame_ast::{
    ActionAst, BinaryOp, Expression, InterfaceMethod, Literal, MachineAst, MethodParam,
    OperationAst, Span, StateVarAst, SystemAst, Type, UnaryOp,
};
use crate::frame_c::compiler::native_region_scanner::{
    c::NativeRegionScannerC, cpp::NativeRegionScannerCpp, csharp::NativeRegionScannerCs,
    dart::NativeRegionScannerDart, erlang::NativeRegionScannerErlang,
    gdscript::NativeRegionScannerGDScript, go::NativeRegionScannerGo,
    java::NativeRegionScannerJava, javascript::NativeRegionScannerJs,
    kotlin::NativeRegionScannerKotlin, lua::NativeRegionScannerLua, php::NativeRegionScannerPhp,
    python::NativeRegionScannerPy, ruby::NativeRegionScannerRuby, rust::NativeRegionScannerRust,
    swift::NativeRegionScannerSwift, typescript::NativeRegionScannerTs, FrameSegmentKind,
    NativeRegionScanner, Region,
};
use crate::frame_c::visitors::TargetLanguage;

/// Generate a complete CodegenNode for a Frame system
///
/// # Arguments
/// * `system` - The parsed Frame system AST
/// * `arcanum` - Symbol table for the system (used for handler info and validation)
/// * `lang` - Target language for code generation
/// * `source` - Original source bytes (used to extract native code via spans)
pub fn generate_system(
    system: &SystemAst,
    arcanum: &Arcanum,
    lang: TargetLanguage,
    source: &[u8],
) -> CodegenNode {
    // Erlang: gen_statem native — completely different codegen path
    if lang == TargetLanguage::Erlang {
        return super::erlang_system::generate_erlang_system(system, arcanum, source);
    }
    // Rust: dedicated codegen with Rc<RefCell<Compartment>> ownership
    if lang == TargetLanguage::Rust {
        return super::rust_system::generate_rust_system(system, arcanum, source);
    }

    generate_system_shared(system, arcanum, lang, source)
}

/// Shared system codegen for class-based languages (15 of 17).
/// Rust and Erlang have dedicated pipelines in `rust_system.rs` and
/// `erlang_system.rs`; they never reach this function.
pub fn generate_system_shared(
    system: &SystemAst,
    arcanum: &Arcanum,
    lang: TargetLanguage,
    source: &[u8],
) -> CodegenNode {
    debug_assert!(
        !matches!(lang, TargetLanguage::Rust | TargetLanguage::Erlang),
        "Rust and Erlang have dedicated pipelines — should not reach generate_system_shared"
    );
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

    // RFC-0015 phase 1.1d: `@@[create(<name>)]` factory rename.
    // Each backend renders the factory in its idiomatic shape:
    // Python uses a `@classmethod`; JS/TS use `static` methods;
    // other backends will follow in subsequent phases.
    if let Some(factory_name) = system.create_op_name() {
        match lang {
            TargetLanguage::Python3 => {
                methods.push(generate_python_factory_alias(system, factory_name));
            }
            TargetLanguage::JavaScript | TargetLanguage::TypeScript => {
                methods.push(generate_js_static_factory_alias(system, factory_name));
            }
            TargetLanguage::GDScript => {
                methods.push(generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!(
                        "return {}._create({})",
                        system.name,
                        params_arg_list(system)
                    ),
                ));
            }
            TargetLanguage::Lua => {
                methods.push(generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!(
                        "return {}._create({})",
                        system.name,
                        params_arg_list(system)
                    ),
                ));
            }
            TargetLanguage::Ruby => {
                methods.push(generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!("_create({})", params_arg_list(system)),
                ));
            }
            TargetLanguage::Php => {
                let php_args = system
                    .params
                    .iter()
                    .map(|p| format!("${}", p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                methods.push(generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!("return self::_create({});", php_args),
                ));
            }
            TargetLanguage::Dart => {
                methods.push(generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!(
                        "return {}._create({});",
                        system.name,
                        params_arg_list(system)
                    ),
                ));
            }
            TargetLanguage::Java | TargetLanguage::CSharp => {
                methods.push(generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!(
                        "return {}.__create({});",
                        system.name,
                        params_arg_list(system)
                    ),
                ));
            }
            TargetLanguage::Cpp => {
                // C++ `__create` returns `System` by value; the renamed
                // alias does too (the `generate_static_factory_alias`
                // default), so no return-type override.
                methods.push(generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!(
                        "return {}::__create({});",
                        system.name,
                        params_arg_list(system)
                    ),
                ));
            }
            TargetLanguage::Kotlin | TargetLanguage::Swift => {
                methods.push(generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!(
                        "return {}.__create({})",
                        system.name,
                        params_arg_list(system)
                    ),
                ));
            }
            TargetLanguage::C => {
                // C: factory is a free function that delegates to
                // the existing `<System>_new(...)` constructor.
                // The C backend prefixes static-method names with
                // `<System>_` so `make` becomes `Counter_make`.
                // Return type is `<System>*` (pointer to allocated
                // struct), matching `<System>_new`.
                let mut method = generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!(
                        "return {}_create({});",
                        system.name,
                        params_arg_list(system)
                    ),
                );
                if let CodegenNode::Method {
                    ref mut return_type,
                    ..
                } = method
                {
                    *return_type = Some(format!("{}*", system.name));
                }
                methods.push(method);
            }
            TargetLanguage::Go => {
                // Go: factory is a package-level function. The Go
                // backend renders `is_static: true` methods as
                // `func Name(...)` (capitalized for public visibility).
                // Default constructor is `New<System>(...)` returning
                // `*<System>` — the factory delegates to it.
                let mut method = generate_static_factory_alias(
                    system,
                    factory_name,
                    &format!("return Create{}({})", system.name, params_arg_list(system)),
                );
                if let CodegenNode::Method {
                    ref mut return_type,
                    ..
                } = method
                {
                    *return_type = Some(format!("*{}", system.name));
                }
                methods.push(method);
            }
            _ => {
                // Other backends (Erlang): follow in subsequent
                // phases. Rust uses its own codegen path in
                // `rust_system.rs` (covered in Phase 1.5).
            }
        }
    }

    // Frame machinery (transition, state management)
    methods.extend(generate_frame_machinery(system, &syntax, lang));

    // Interface wrappers
    methods.extend(generate_interface_wrappers(system, &syntax));

    // Check if system has states with state variables (for Rust compartment-based push/pop)
    let has_state_vars = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().any(|s| !s.state_vars.is_empty()))
        .unwrap_or(false);

    // State handlers - use enhanced Arcanum for clean iteration
    if let Some(ref machine) = system.machine {
        methods.extend(generate_state_handlers_via_arcanum(
            &system.name,
            machine,
            arcanum,
            source,
            lang,
            has_state_vars,
        ));
    }

    // Actions - extract native code from source using spans.
    // `generate_action` returns a `Vec<CodegenNode>` so trivia
    // (action `leading_comments`) can prepend the method node.
    for action in &system.actions {
        methods.extend(generate_action(action, &syntax, source));
    }

    // Operations - same pattern as actions.
    //
    // RFC-0012 amendment: operations marked `@@[save]` / `@@[load]`
    // are framework-managed — their bodies are emitted by
    // `generate_persistence_methods` below, NOT by the per-target
    // operation codegen. We skip them here to avoid emitting the
    // user's empty placeholder method alongside the framework one.
    for operation in &system.operations {
        let is_framework_managed = operation
            .attributes
            .iter()
            .any(|a| a.name == "save" || a.name == "load");
        if is_framework_managed {
            continue;
        }
        methods.extend(generate_operation(operation, &syntax, source));
    }

    // Persistence methods (when @@persist is present)
    if system.persist_attr.is_some() {
        methods.extend(generate_persistence_methods(system, &syntax));
    }

    let mut class_node = CodegenNode::Class {
        name: system.name.clone(),
        fields,
        methods,
        base_classes: system.bases.clone(),
        is_abstract: false,
        derives: vec![], // Derives not used - we manually build JSON
        visibility: if system.visibility.as_deref() == Some("private") {
            Visibility::Private
        } else {
            Visibility::Public
        },
    };

    // Post-process: make dispatch chain async if any interface method is async.
    // Java is excluded — it has no native async/await. Instead, Java
    // interface methods return CompletableFuture<T> (see `make_java_interface_async`)
    // while the internal dispatch chain stays synchronous.
    if needs_async {
        if matches!(lang, TargetLanguage::Java) {
            make_java_interface_async(&mut class_node, system);
        } else {
            make_system_async(&mut class_node, &system.name, lang);
        }
    }

    class_node
}



/// Element type carried by one of the kernel's two stacks. C# and Go
/// format the element type into the init expression; every other language
/// either uses a default-constructed empty container or doesn't care
/// about the element type at all.
enum StackElementKind {
    /// `_state_stack` — Compartment values; pointer-typed in Go.
    Compartment,
    /// `_context_stack` — FrameContext values; never pointer-typed.
    FrameContext,
}

/// Emit the per-language initializer for a kernel stack field
/// (`_state_stack` or `_context_stack`). Returns `None` for languages
/// where no init is needed (C++: vectors default-construct empty;
/// Graphviz: unreachable in this code path).
fn init_collection_stack(
    field_name: &str,
    element_kind: StackElementKind,
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
) -> Option<CodegenNode> {
    match syntax.language {
        TargetLanguage::C => Some(CodegenNode::assign(
            CodegenNode::field(CodegenNode::self_ref(), field_name),
            CodegenNode::Ident(format!("{}_FrameVec_new()", system.name)),
        )),
        // C++ vectors default-construct as empty; no init needed.
        TargetLanguage::Cpp => None,
        TargetLanguage::Java => Some(CodegenNode::NativeBlock {
            code: format!("{} = new ArrayList<>();", field_name),
            span: None,
        }),
        TargetLanguage::Kotlin => Some(CodegenNode::NativeBlock {
            code: format!("{} = mutableListOf()", field_name),
            span: None,
        }),
        TargetLanguage::Swift => Some(CodegenNode::NativeBlock {
            code: format!("{} = []", field_name),
            span: None,
        }),
        TargetLanguage::CSharp => {
            let elem = match element_kind {
                StackElementKind::Compartment => format!("{}Compartment", system.name),
                StackElementKind::FrameContext => format!("{}FrameContext", system.name),
            };
            Some(CodegenNode::NativeBlock {
                code: format!("{} = new List<{}>();", field_name, elem),
                span: None,
            })
        }
        TargetLanguage::Go => {
            // Go state-stack stores Compartment pointers; context-stack stores values.
            let elem = match element_kind {
                StackElementKind::Compartment => format!("*{}Compartment", system.name),
                StackElementKind::FrameContext => format!("{}FrameContext", system.name),
            };
            Some(CodegenNode::NativeBlock {
                code: format!("s.{} = make([]{}, 0)", field_name, elem),
                span: None,
            })
        }
        // Dynamic / array-literal languages.
        TargetLanguage::Python3
        | TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Rust
        | TargetLanguage::Lua
        | TargetLanguage::Dart
        | TargetLanguage::GDScript => Some(CodegenNode::assign(
            CodegenNode::field(CodegenNode::self_ref(), field_name),
            CodegenNode::Array(vec![]),
        )),
        TargetLanguage::Graphviz => unreachable!(),
    }
}

/// Decide whether the constructor body should emit a domain-field init
/// statement, given the language and the field's properties.
///
/// Three groups:
/// - **Always emit** (C, Go, Python, Ruby, Lua, Rust): no field-level
///   init is available (C/Go) or none is generated (dynamic langs go
///   straight to the constructor body).
/// - **Emit only on collision** (Cpp, Java, Swift, C#, Dart, GDScript,
///   TS, JS, PHP): the field has a literal init at declaration scope,
///   except when that init references a system param — then the init
///   moves into the constructor body to avoid the name-collision.
/// - **Kotlin**: same as the OO group. (Const fields whose init refers
///   to a system param are emitted as `var` at the Kotlin level so the
///   constructor-body assignment compiles — see `build_system_fields`.)
/// - **Erlang / Graphviz**: never go through this code path.
fn should_emit_constructor_body_init(
    lang: TargetLanguage,
    _is_const: bool,
    init_refs_param: bool,
    init_has_tagged: bool,
) -> bool {
    use TargetLanguage::*;
    match lang {
        C | Go | Python3 | Ruby | Lua | Rust => true,
        // PHP rejects non-const class-field defaults at parse time, so
        // any `@@<System>()` initializer has to move to the constructor
        // body — same flag the field-emission path uses to strip the
        // inline init.
        Php => init_refs_param || init_has_tagged,
        Cpp | Java | Swift | CSharp | Dart | GDScript | TypeScript | JavaScript | Kotlin => {
            init_refs_param
        }
        // (Kotlin previously skipped const fields here on the assumption
        //  they'd be primary-constructor params; RFC-0017 made the system
        //  ctor parameterless, so the const field must take its value via
        //  the constructor-body assignment like every other backend.)
        Erlang | Graphviz => false,
    }
}

/// True when the target emits domain fields as constructor-body
/// assignments (so leading comments belong inline with the body) vs
/// as class-level field declarations (so leading comments belong on
/// the per-backend `Field` IR's `emit_field` path). Rust is in the
/// constructor-body camp because its emission uses
/// `CodegenNode::assign`, but framec also emits Rust struct field
/// declarations whose `Field` IR carries the comments — the
/// duplication is acceptable for now since Rust's struct-decl path
/// is the user-visible one.
fn uses_constructor_body_init(lang: TargetLanguage) -> bool {
    use TargetLanguage::*;
    matches!(lang, Python3 | Ruby | Lua | Php | GDScript | Go | C | Rust)
}

/// Format `field = init_value` using the per-language self-access form
/// and statement terminator. Used to build the constructor body's
/// domain-field init lines for every language EXCEPT Rust (which uses
/// the structured `CodegenNode::assign` instead) and the C++ const-init
/// case (which uses a member initializer list).
///
/// `field_type` is the user-declared type from the Frame source (an
/// opaque string per Frame's "no type system" rule). Most language
/// arms ignore it; **C** uses it to disambiguate brace-initialized
/// arrays/structs (`{0}`, `{1, 2}`) from scalar inits — array
/// assignment is illegal in C, so a brace init has to be emitted as
/// a `memcpy` from a compound literal rather than a plain `=`.
fn format_field_assignment(
    lang: TargetLanguage,
    field_name: &str,
    init_value: &str,
    field_type: &str,
) -> String {
    use TargetLanguage::*;
    match lang {
        C => {
            // C arrays aren't assignable: `arr = {0};` is a syntax
            // error even though `<Type> arr = {0};` is fine in
            // declaration position. Detect brace-initializers and
            // emit the equivalent via a typed compound literal +
            // memcpy. Works for arrays AND structs; scalar inits
            // (the common case) keep the simple `=` form.
            if init_value.trim_start().starts_with('{') {
                format!(
                    "{{ {ty} __init_{name} = {init}; \
                     memcpy(&self->{name}, &__init_{name}, sizeof(self->{name})); }}",
                    ty = field_type,
                    name = field_name,
                    init = init_value,
                )
            } else {
                format!("self->{} = {};", field_name, init_value)
            }
        }
        Cpp => format!("this->{} = {};", field_name, init_value),
        Go => format!("s.{} = {}", field_name, init_value),
        Java | CSharp | Dart | TypeScript | JavaScript => {
            format!("this.{} = {};", field_name, init_value)
        }
        Swift | GDScript | Python3 | Lua => format!("self.{} = {}", field_name, init_value),
        Kotlin => format!("this.{} = {}", field_name, init_value),
        Php => format!("$this->{} = {};", field_name, init_value),
        Ruby => format!("@{} = {}", field_name, init_value),
        Rust | Erlang | Graphviz => unreachable!(
            "format_field_assignment called for {:?} (Rust uses structured assign; \
             Erlang/Graphviz never reach this code path)",
            lang
        ),
    }
}

/// Generate the constructor
pub(crate) fn generate_constructor(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
) -> CodegenNode {
    let mut body = Vec::new();
    // C++ member initializer list entries: `field(value), field2(value2)`
    // Collected during domain init loop for const fields whose init was stripped.
    let mut cpp_init_list: Vec<String> = Vec::new();

    // Initialize state stack and context stack — same shape per language,
    // only the field name and element-type formatting differ.
    if let Some(node) = init_collection_stack(
        "_state_stack",
        StackElementKind::Compartment,
        system,
        syntax,
    ) {
        body.push(node);
    }
    if let Some(node) = init_collection_stack(
        "_context_stack",
        StackElementKind::FrameContext,
        system,
        syntax,
    ) {
        body.push(node);
    }

    // Initialize domain variables.
    //
    // Three concerns interleave per language:
    //   1. WHETHER to emit a body init at all (see
    //      `should_emit_constructor_body_init`).
    //   2. HOW to spell `self.field = value` for the target
    //      (see `format_field_assignment`).
    //   3. Per-language adjustments to the init expression itself
    //      (PHP `$`-prefix on param refs, Lua `[]` → `{}`, Rust
    //      Domain-param override, C++ const → member init list).
    //
    // Rust is the structural odd-one-out: it uses `CodegenNode::assign`
    // instead of a `NativeBlock` and must always init every field
    // (including the no-init case, handled up front).
    let sys_param_names_for_init: Vec<String> =
        system.params.iter().map(|p| p.name.clone()).collect();
    for domain_var in &system.domain {
        // Emit any source-level leading comments first, so they
        // sit immediately above the field-assignment line in the
        // generated constructor. Skipped for languages that emit
        // domain fields as class-level declarations (Cpp, Java,
        // CSharp, Swift, Kotlin, TypeScript, JavaScript, Dart) —
        // there the comments live on the `Field` IR's
        // `leading_comments` and the per-backend `emit_field`
        // prepends them. The constructor-body path (Python, Ruby,
        // Lua, PHP, GDScript, Go, C, Rust) does not have an
        // emit_field hook for domain fields, so the comments
        // attach here.
        if !domain_var.leading_comments.is_empty() && uses_constructor_body_init(syntax.language) {
            for comment in &domain_var.leading_comments {
                body.push(CodegenNode::NativeBlock {
                    code: comment.clone(),
                    span: None,
                });
            }
        }
        // Rust requires all fields initialized; handle the no-init
        // case up front before the regular path.
        if matches!(syntax.language, TargetLanguage::Rust) && domain_var.initializer_text.is_none()
        {
            let rust_init = if system.params.iter().any(|p| {
                p.name == domain_var.name
                    && matches!(
                        p.kind,
                        crate::frame_c::compiler::frame_ast::ParamKind::Domain
                    )
            }) {
                domain_var.name.clone()
            } else {
                "Default::default()".to_string()
            };
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                CodegenNode::Ident(rust_init),
            ));
            continue;
        }

        let init_text = match &domain_var.initializer_text {
            Some(t) => t,
            None => continue,
        };
        let init_refs_param = init_references_param(init_text, &sys_param_names_for_init);
        let init_has_tagged = init_text.contains("@@");

        if !should_emit_constructor_body_init(
            syntax.language,
            domain_var.is_const,
            init_refs_param,
            init_has_tagged,
        ) {
            continue;
        }

        let init_expanded = expand_system_instantiation_in_domain(init_text, syntax.language);

        // C++ const fields whose init references a constructor param
        // must use the member initializer list — `const T x;` cannot
        // be reassigned in the body. The list is rendered before the
        // constructor body as `: x(value)`.
        if matches!(syntax.language, TargetLanguage::Cpp) && domain_var.is_const {
            cpp_init_list.push(format!("{}({})", domain_var.name, init_expanded));
            continue;
        }

        // Per-language init-expression adjustments.
        let final_init = match syntax.language {
            // PHP rejects bare names in init expressions; system params
            // need a `$` prefix to be valid PHP.
            TargetLanguage::Php => prefix_php_vars(&init_expanded, &sys_param_names_for_init),
            // Lua uses `{}` for empty tables, not `[]`.
            TargetLanguage::Lua if init_expanded.trim() == "[]" => "{}".to_string(),
            // Rust: a Domain-kind system param of the same name overrides
            // the literal default.
            TargetLanguage::Rust
                if system.params.iter().any(|p| {
                    p.name == domain_var.name
                        && matches!(
                            p.kind,
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain
                        )
                }) =>
            {
                domain_var.name.clone()
            }
            _ => init_expanded,
        };

        if matches!(syntax.language, TargetLanguage::Rust) {
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                CodegenNode::Ident(final_init),
            ));
        } else {
            // The user-declared type is opaque (`Type::Custom(..)` for
            // most cases, `Type::Unknown` for bare-form fields in
            // dynamic targets). Render to a string so the C arm can
            // recognize array/struct types when the init is a brace
            // initializer.
            let type_str = match &domain_var.var_type {
                crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
                _ => String::new(),
            };
            body.push(CodegenNode::NativeBlock {
                code: format_field_assignment(
                    syntax.language,
                    &domain_var.name,
                    &final_init,
                    &type_str,
                ),
                span: None,
            });
        }
    }

    // Domain-kind system params override domain field defaults.
    // The domain init uses the literal default (e.g., `self.inventory = {}`).
    // We must then assign the constructor arg: `self.inventory = inventory`.
    // Note: the param name always matches the domain field name for Domain params.
    for p in &system.params {
        if matches!(
            p.kind,
            crate::frame_c::compiler::frame_ast::ParamKind::Domain
        ) {
            // Check that this param name actually matches a domain field
            let matching_field = system.domain.iter().find(|d| d.name == p.name);
            if matching_field.is_none() {
                continue; // Skip — no matching domain field
            }
            // Skip if the domain field's init already references this param
            // (avoids double assignment, which breaks final/readonly/const)
            let field_init = matching_field
                .unwrap()
                .initializer_text
                .as_deref()
                .unwrap_or("");
            if field_init.trim() == p.name {
                continue; // Domain init already assigns from this param
            }
            let assign_code = match syntax.language {
                TargetLanguage::Python3 | TargetLanguage::GDScript | TargetLanguage::Lua => {
                    format!("self.{} = {}", p.name, p.name)
                }
                TargetLanguage::Ruby => {
                    format!("@{} = {}", p.name, p.name)
                }
                TargetLanguage::Php => {
                    format!("$this->{} = ${};", p.name, p.name)
                }
                TargetLanguage::C => {
                    format!("self->{} = {};", p.name, p.name)
                }
                TargetLanguage::Go => {
                    format!("s.{} = {}", p.name, p.name)
                }
                TargetLanguage::Rust => {
                    // Rust handles this in the struct literal — skip here
                    continue;
                }
                TargetLanguage::Cpp => {
                    format!("this->{} = {};", p.name, p.name)
                }
                TargetLanguage::Swift => {
                    format!("self.{} = {}", p.name, p.name)
                }
                TargetLanguage::Erlang => {
                    continue; // Erlang handles domain differently
                }
                _ => {
                    // Java, C#, Kotlin, Dart, TypeScript, JavaScript
                    format!("this.{} = {};", p.name, p.name)
                }
            };
            body.push(CodegenNode::NativeBlock {
                code: assign_code,
                span: None,
            });
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
                        CodegenNode::field(CodegenNode::self_ref(), &format!("__sys_{}", p.name)),
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
            let mut ancestor_chain: Vec<&crate::frame_c::compiler::frame_ast::StateAst> =
                Vec::new();
            if has_hsm_parent {
                let mut current_parent = first_state.parent.as_ref();
                while let Some(parent_name) = current_parent {
                    if let Some(parent_state) =
                        machine.states.iter().find(|s| &s.name == parent_name)
                    {
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
                            CodegenNode::Ident(format!(
                                "{}Compartment::new(\"{}\")",
                                system.name, first_state.name
                            )),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::Ident("None".to_string()),
                        ));
                    }
                }
                TargetLanguage::C => {
                    // C: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // state_args / enter_args flow through the helper.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let mut init_code = String::new();
                    // Build temporary FrameVecs for state_args and enter_args.
                    if state_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __sa = NULL;\n",
                            sys = system.name
                        ));
                    } else {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __sa = {sys}_FrameVec_new();\n",
                            sys = system.name
                        ));
                        for n in &state_args_vec {
                            init_code.push_str(&format!(
                                "{sys}_FrameVec_push(__sa, (void*)(intptr_t)({n}));\n",
                                sys = system.name,
                                n = n
                            ));
                        }
                    }
                    if enter_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __ea = NULL;\n",
                            sys = system.name
                        ));
                    } else {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __ea = {sys}_FrameVec_new();\n",
                            sys = system.name
                        ));
                        for n in &enter_args_vec {
                            init_code.push_str(&format!(
                                "{sys}_FrameVec_push(__ea, (void*)(intptr_t)({n}));\n",
                                sys = system.name,
                                n = n
                            ));
                        }
                    }
                    init_code.push_str(&format!(
                        "self->__compartment = {sys}_prepareEnter(self, \"{leaf}\", __sa, __ea);\n",
                        sys = system.name,
                        leaf = first_state.name
                    ));
                    init_code.push_str("self->__next_compartment = NULL;\n");
                    if !state_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec_destroy(__sa);\n",
                            sys = system.name
                        ));
                    }
                    if !enter_args_vec.is_empty() {
                        init_code
                            .push_str(&format!("{sys}_FrameVec_destroy(__ea);", sys = system.name));
                    }
                    body.push(CodegenNode::NativeBlock {
                        code: init_code,
                        span: None,
                    });
                }
                TargetLanguage::Python3 => {
                    // Python: build the start state's compartment chain
                    // via __prepareEnter, the same helper used by all
                    // transitions. System header params flow into the
                    // start state's state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = None",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    // TS/JS: build start chain via __prepareEnter, the same
                    // helper used by all transitions. System header params
                    // flow into state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = this.__prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Php => {
                    // PHP: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| format!("${}", p.name))
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| format!("${}", p.name))
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "$this->__compartment = $this->__prepareEnter(\"{}\", {}, {});\n$this->__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Ruby => {
                    // Ruby: build start chain via __prepareEnter, the same
                    // helper used by all transitions. System header params
                    // flow into state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "@__compartment = __prepareEnter(\"{}\", {}, {})\n@__next_compartment = nil",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Cpp => {
                    // C++: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // state_args / enter_args flow through the helper.
                    // Values are wrapped in std::any so the dispatch
                    // reader (which uses std::any_cast<Type>) round-trips
                    // them correctly.
                    let _ = compartment_class;
                    let state_args_wrapped: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| format!("std::any({})", cpp_wrap_any_arg(&p.name)))
                        .collect();
                    let enter_args_wrapped: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| format!("std::any({})", cpp_wrap_any_arg(&p.name)))
                        .collect();
                    let state_arg =
                        format!("std::vector<std::any>{{{}}}", state_args_wrapped.join(", "));
                    let enter_arg =
                        format!("std::vector<std::any>{{{}}}", enter_args_wrapped.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "__compartment = __prepareEnter(\"{}\", {}, {});",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Java => {
                    // Java: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "new ArrayList<>()".to_string()
                    } else {
                        format!(
                            "new ArrayList<>(java.util.Arrays.asList({}))",
                            state_args_vec.join(", ")
                        )
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "new ArrayList<>()".to_string()
                    } else {
                        format!(
                            "new ArrayList<>(java.util.Arrays.asList({}))",
                            enter_args_vec.join(", ")
                        )
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Kotlin => {
                    // Kotlin: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "mutableListOf<Any?>()".to_string()
                    } else {
                        format!("mutableListOf<Any?>({})", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "mutableListOf<Any?>()".to_string()
                    } else {
                        format!("mutableListOf<Any?>({})", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {})\nthis.__next_compartment = null",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Swift => {
                    // Swift: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = {}.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = nil",
                            system.name, first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::CSharp => {
                    // C#: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "new List<object>()".to_string()
                    } else {
                        format!("new List<object> {{ {} }}", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "new List<object>()".to_string()
                    } else {
                        format!("new List<object> {{ {} }}", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Go => {
                    // Go: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "[]any{}".to_string()
                    } else {
                        format!("[]any{{{}}}", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "[]any{}".to_string()
                    } else {
                        format!("[]any{{{}}}", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "s.__compartment = s.__prepareEnter(\"{}\", {}, {})\ns.__next_compartment = nil",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Dart => {
                    // Dart: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = syntax;
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = this.__prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::GDScript => {
                    // GDScript: build start chain via __prepareEnter, the
                    // same helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = null",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                // Lua: build start chain via __prepareEnter, the same
                // helper used by all transitions. System header params
                // flow into state_args / enter_args channels. Uses
                // table.pack(...) / nil instead of `{}` literals
                // (block-transformer workaround).
                TargetLanguage::Lua => {
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "nil".to_string()
                    } else {
                        format!("table.pack({})", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "nil".to_string()
                    } else {
                        format!("table.pack({})", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self:__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = nil",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                // Dynamic languages and remaining: New expression
                // (Erlang, Kotlin — all routed here)
                TargetLanguage::Python3
                | TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Php
                | TargetLanguage::Ruby
                | TargetLanguage::Erlang
                | TargetLanguage::Kotlin => {
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
                }
                TargetLanguage::Graphviz => unreachable!(),
            }

            // Send $> (enter) event via __kernel - language-specific.
            // The enter_args of the start state's compartment carry any
            // header-declared enter params; pass them through so the
            // start state's $>(name: type) handler can read them.
            //
            // C optimization: if the start state has no declared enter
            // params, pass NULL for the event's `_parameters` (the
            // dispatch generates no `FrameDict_get` calls in that case).
            let start_state_has_enter_params = first_state
                .enter
                .as_ref()
                .map(|e| !e.params.is_empty())
                .unwrap_or(false);
            let init_event_code = match syntax.language {
                TargetLanguage::Python3 => format!(
                    r#"__e = {ec}("$>", self.__compartment.enter_args)
__ctx = {sys}FrameContext(__e, None)
self._context_stack.append(__ctx)
self.__router(__e)
self.__process_transition_loop()
self._context_stack.pop()"#,
                    ec = event_class,
                    sys = system.name
                ),
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    let _ = event_class;
                    format!(
                        r#"const __e = new {sys}FrameEvent("$>", this.__compartment.enter_args);
const __ctx = new {sys}FrameContext(__e, null);
this._context_stack.push(__ctx);
this.__router(__e);
this.__process_transition_loop();
this._context_stack.pop();"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Rust => format!(
                    r#"let __frame_event = {}::new("$>");
let __ctx = {}FrameContext::new(__frame_event, None);
self._context_stack.push(__ctx);
self.__kernel();
self._context_stack.pop();"#,
                    event_class, system.name
                ),
                TargetLanguage::C => {
                    let _ = start_state_has_enter_params;
                    format!(
                        r#"{sys}_FrameEvent* __e = {sys}_FrameEvent_new("$>", self->__compartment->enter_args, 0);
{sys}_FrameContext* __ctx = {sys}_FrameContext_new(__e, NULL);
{sys}_FrameVec_push(self->_context_stack, __ctx);
{sys}_router(self, __e);
{sys}_process_transition_loop(self);
{sys}_FrameContext* __init_ctx = ({sys}_FrameContext*){sys}_FrameVec_pop(self->_context_stack);
{sys}_FrameContext_destroy(__init_ctx);
{sys}_FrameEvent_destroy(__e);"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Cpp => format!(
                    r#"{sys}FrameEvent __e("$>", __compartment->enter_args);
{sys}FrameContext __ctx(std::move(__e));
_context_stack.push_back(std::move(__ctx));
__router(_context_stack.back()._event);
__process_transition_loop();
_context_stack.pop_back();"#,
                    sys = system.name
                ),
                TargetLanguage::Java => {
                    let _ = event_class;
                    format!(
                        r#"{sys}FrameEvent __e = new {sys}FrameEvent("$>", __compartment.enter_args);
{sys}FrameContext __ctx = new {sys}FrameContext(__e, null);
_context_stack.add(__ctx);
__router(__e);
__process_transition_loop();
_context_stack.remove(_context_stack.size() - 1);"#,
                        sys = system.name
                    )
                }
                TargetLanguage::CSharp => {
                    let _ = event_class;
                    format!(
                        r#"{sys}FrameEvent __e = new {sys}FrameEvent("$>", __compartment.enter_args);
{sys}FrameContext __ctx = new {sys}FrameContext(__e, null);
_context_stack.Add(__ctx);
__router(__e);
__process_transition_loop();
_context_stack.RemoveAt(_context_stack.Count - 1);"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Go => {
                    let _ = event_class;
                    format!(
                        r#"__e := {sys}FrameEvent{{_message: "$>", _parameters: s.__compartment.enterArgs}}
__ctx := {sys}FrameContext{{_event: __e, _data: make(map[string]any)}}
s._context_stack = append(s._context_stack, __ctx)
s.__router(&s._context_stack[len(s._context_stack)-1]._event)
s.__process_transition_loop()
s._context_stack = s._context_stack[:len(s._context_stack)-1]"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Kotlin => {
                    let _ = event_class;
                    format!(
                        r#"val __e = {sys}FrameEvent("$>", __compartment.enter_args)
val __ctx = {sys}FrameContext(__e, null)
_context_stack.add(__ctx)
__router(__e)
__process_transition_loop()
_context_stack.removeAt(_context_stack.size - 1)"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Swift => {
                    let _ = event_class;
                    format!(
                        r#"let __e = {sys}FrameEvent(message: "$>", parameters: __compartment.enter_args)
let __ctx = {sys}FrameContext(event: __e)
_context_stack.append(__ctx)
__router(__e)
__process_transition_loop()
_context_stack.removeLast()"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Php => {
                    let _ = event_class;
                    format!(
                        r#"$__e = new {sys}FrameEvent("$>", $this->__compartment->enter_args);
$__ctx = new {sys}FrameContext($__e, null);
$this->_context_stack[] = $__ctx;
$this->__router($__e);
$this->__process_transition_loop();
array_pop($this->_context_stack);"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Ruby => format!(
                    r#"__e = {ec}.new("$>", @__compartment.enter_args)
__ctx = {sys}FrameContext.new(__e, nil)
@_context_stack.push(__ctx)
__router(__e)
__process_transition_loop
@_context_stack.pop"#,
                    ec = event_class,
                    sys = system.name
                ),
                TargetLanguage::Lua => format!(
                    r#"local __e = {ec}.new("$>", self.__compartment.enter_args)
local __ctx = {sys}FrameContext.new(__e, nil)
self._context_stack[#self._context_stack + 1] = __ctx
self:__router(__e)
self:__process_transition_loop()
table.remove(self._context_stack)"#,
                    ec = event_class,
                    sys = system.name
                ),
                TargetLanguage::Dart => {
                    let _ = event_class;
                    format!(
                        r#"final __e = {sys}FrameEvent("\$>", __compartment.enter_args);
final __ctx = {sys}FrameContext(__e, null);
_context_stack.add(__ctx);
__router(__e);
__process_transition_loop();
_context_stack.removeLast();"#,
                        sys = system.name
                    )
                }
                TargetLanguage::GDScript => {
                    let _ = event_class;
                    format!(
                        r#"var __e = {sys}FrameEvent.new("$>", self.__compartment.enter_args)
var __ctx = {sys}FrameContext.new(__e, null)
self._context_stack.append(__ctx)
self.__router(__e)
self.__process_transition_loop()
self._context_stack.pop_back()"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Erlang => String::new(), // gen_statem: handled natively by erlang_system.rs
                TargetLanguage::Graphviz => unreachable!(),
            };
            body.push(CodegenNode::NativeBlock {
                code: init_event_code,
                span: None,
            });
        }
    }

    // Params from system params
    let params: Vec<Param> = system
        .params
        .iter()
        .map(|p| {
            let type_str = type_to_string(&p.param_type);
            let mut param = Param::new(&p.name).with_type(&type_str);
            if let Some(ref def) = p.default {
                param = param.with_default(CodegenNode::Ident(def.clone()));
            }
            param
        })
        .collect();

    // For C++: emit const fields via member initializer list.
    // The C++ backend formats super_call as ` : {expr}` before the body.
    let super_call = if !cpp_init_list.is_empty() && matches!(syntax.language, TargetLanguage::Cpp)
    {
        Some(Box::new(CodegenNode::Ident(cpp_init_list.join(", "))))
    } else {
        None
    };

    CodegenNode::Constructor {
        params,
        body,
        super_call,
    }
}


/// Compute the HSM topology chain for each state in the machine.
///
/// Returns a list of (leaf_state_name, ancestor_chain) pairs, where
/// `ancestor_chain` is ordered root-to-leaf (with the leaf as the last
/// entry). For flat states, the chain has length 1 — just the state
/// itself. For HSM children, the chain walks up via each state's
/// declared `parent` field until a state with no parent is reached.
///
/// This is the source of truth for `_HSM_CHAIN` table emission and for
/// `__prepareEnter` / persistence-restore chain reconstruction.
pub(crate) fn compute_hsm_chains(system: &SystemAst) -> Vec<(String, Vec<String>)> {
    let machine = match &system.machine {
        Some(m) => m,
        None => return Vec::new(),
    };
    let parent_lookup: std::collections::HashMap<String, Option<String>> = machine
        .states
        .iter()
        .map(|s| (s.name.clone(), s.parent.clone()))
        .collect();
    let mut result = Vec::new();
    for state in &machine.states {
        let mut chain: Vec<String> = Vec::new();
        let mut cursor = Some(state.name.clone());
        while let Some(name) = cursor {
            chain.push(name.clone());
            cursor = parent_lookup.get(&name).cloned().flatten();
        }
        chain.reverse();
        result.push((state.name.clone(), chain));
    }
    result
}

/// Generate Frame machinery methods (__kernel, __router, __transition,
/// __prepareEnter, __prepareExit, __fire_enter_cascade,
/// __fire_exit_cascade, __route_to_state) for all target languages.
pub(crate) fn generate_frame_machinery(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
    lang: TargetLanguage,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let compartment_class = format!("{}Compartment", system.name);
    let event_class = format!("{}FrameEvent", system.name);

    match lang {
        TargetLanguage::Python3 => {
            // 4.2 plan §7.1.P1: Python migrated to the MachineryGenerator
            // trait (canary). Output is byte-canonical with the previous
            // `generate_python3_machinery` fn; matrix gates the safety.
            // The legacy fn below is retained until §7.1.P4 (after all
            // backends migrate) — at which point the entire match falls
            // away and `super::machinery::dispatch(lang)` becomes the
            // single entry point.
            use super::machinery::{generate_machinery, python::PythonMachinery};
            methods.extend(generate_machinery(
                &PythonMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            // 4.2 plan §7.1.P3: JS/TS migrated to MachineryGenerator.
            use super::machinery::{generate_machinery, javascript::JavaScriptMachinery};
            let m = JavaScriptMachinery {
                is_ts: matches!(lang, TargetLanguage::TypeScript),
            };
            methods.extend(generate_machinery(&m, system, &event_class, &compartment_class));
        }
        TargetLanguage::Php => {
            // 4.2 plan §7.1.P3: PHP migrated to MachineryGenerator.
            use super::machinery::{generate_machinery, php::PhpMachinery};
            methods.extend(generate_machinery(&PhpMachinery, system, &event_class, &compartment_class));
        }
        TargetLanguage::Ruby => {
            use super::machinery::{generate_machinery, ruby::RubyMachinery};
            methods.extend(generate_machinery(&RubyMachinery, system, &event_class, &compartment_class));
        }
        TargetLanguage::Rust => {
            // 4.2 plan §7.1.P2: Rust migrated to the MachineryGenerator trait.
            use super::machinery::{generate_machinery, rust::RustMachinery};
            methods.extend(generate_machinery(
                &RustMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::C => {
            use super::machinery::{c::CMachinery, generate_machinery};
            methods.extend(generate_machinery(
                &CMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Cpp => {
            use super::machinery::{cpp::CppMachinery, generate_machinery};
            methods.extend(generate_machinery(
                &CppMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Java => {
            // 4.2 plan §7.1.P2: Java migrated to the MachineryGenerator trait.
            use super::machinery::{generate_machinery, java::JavaMachinery};
            methods.extend(generate_machinery(
                &JavaMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Kotlin => {
            use super::machinery::{generate_machinery, kotlin::KotlinMachinery};
            methods.extend(generate_machinery(
                &KotlinMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Swift => {
            use super::machinery::{generate_machinery, swift::SwiftMachinery};
            methods.extend(generate_machinery(
                &SwiftMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::CSharp => {
            use super::machinery::{csharp::CSharpMachinery, generate_machinery};
            methods.extend(generate_machinery(
                &CSharpMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Go => {
            use super::machinery::{generate_machinery, go::GoMachinery};
            methods.extend(generate_machinery(
                &GoMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Erlang => {
            // gen_statem: kernel/router/transition are built into OTP — no custom methods needed
        }
        TargetLanguage::Lua => {
            use super::machinery::{generate_machinery, lua::LuaMachinery};
            methods.extend(generate_machinery(
                &LuaMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Dart => {
            use super::machinery::{dart::DartMachinery, generate_machinery};
            methods.extend(generate_machinery(
                &DartMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::GDScript => {
            // 4.2 plan §7.1.P2: GDScript migrated to the MachineryGenerator trait.
            use super::machinery::{generate_machinery, gdscript::GDScriptMachinery};
            methods.extend(generate_machinery(
                &GDScriptMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Graphviz => unreachable!(),
    }

    methods
}

// =====================================================================
// Per-language frame-machinery helpers
// =====================================================================
//
// Each `generate_<lang>_machinery` returns the kernel/router/transition
// methods for one target language. They were previously inlined as
// match arms inside `generate_frame_machinery`, which had grown to
// ~1374 lines (one giant match across 17 backends). Splitting them
// out gives each language an isolated, navigable function while
// keeping codegen entirely in this file.




#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::codegen::codegen_utils::{
        convert_expression, convert_literal, cpp_map_type, cpp_wrap_any_arg, csharp_map_type,
        expression_to_string, go_map_type, java_map_type, kotlin_map_type, state_var_init_value,
        swift_map_type, to_snake_case, type_to_cpp_string, type_to_string, HandlerContext,
    };
    use crate::frame_c::compiler::frame_ast::{
        DomainVar, Expression, Literal, Span, SystemAst, Type,
    };
    use crate::frame_c::visitors::TargetLanguage;

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
            is_const: false,
            leading_comments: Vec::new(),
            attributes: Vec::new(),
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

    #[test]
    fn test_init_references_param_exact_match() {
        assert!(init_references_param("balance", &["balance".into()]));
    }

    #[test]
    fn test_init_references_param_in_expression() {
        assert!(init_references_param("balance + 10", &["balance".into()]));
    }

    #[test]
    fn test_init_references_param_no_match() {
        assert!(!init_references_param("0", &["balance".into()]));
        assert!(!init_references_param(
            "mutableListOf<>()",
            &["balance".into()]
        ));
    }

    #[test]
    fn test_init_references_param_member_access_not_matched() {
        // Defaults.count should NOT match param "count" — it's a member access
        assert!(!init_references_param("Defaults.count", &["count".into()]));
        assert!(!init_references_param("obj.balance", &["balance".into()]));
        assert!(!init_references_param(
            "Config.DEFAULT_VALUE",
            &["DEFAULT_VALUE".into()]
        ));
    }

    #[test]
    fn test_init_references_param_method_on_param() {
        // count.toString() SHOULD match — it's a method call on the param
        assert!(init_references_param("count.toString()", &["count".into()]));
    }

    #[test]
    fn test_init_references_param_substring_not_matched() {
        // "rebalance" should NOT match param "balance"
        assert!(!init_references_param("rebalance", &["balance".into()]));
        assert!(!init_references_param("balanced_tree", &["balance".into()]));
    }

    #[test]
    fn test_init_references_param_empty() {
        assert!(!init_references_param("", &["balance".into()]));
        assert!(!init_references_param("balance", &[]));
    }

}
