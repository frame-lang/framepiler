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
mod constructor;

pub(crate) use constructor::generate_constructor;

use async_wrap::make_java_interface_async;
use factory::{
    generate_js_static_factory_alias, generate_python_factory_alias, generate_static_factory_alias,
    params_arg_list,
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
            methods.extend(generate_machinery(
                &m,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Php => {
            // 4.2 plan §7.1.P3: PHP migrated to MachineryGenerator.
            use super::machinery::{generate_machinery, php::PhpMachinery};
            methods.extend(generate_machinery(
                &PhpMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
        }
        TargetLanguage::Ruby => {
            use super::machinery::{generate_machinery, ruby::RubyMachinery};
            methods.extend(generate_machinery(
                &RubyMachinery,
                system,
                &event_class,
                &compartment_class,
            ));
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
            use super::machinery::{gdscript::GDScriptMachinery, generate_machinery};
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
