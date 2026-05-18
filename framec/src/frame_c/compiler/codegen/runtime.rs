//! Runtime type generation for Frame systems.
//!
//! Generates FrameEvent, FrameContext, Compartment classes and supporting
//! runtime types for all target languages. These are the infrastructure
//! classes that every Frame system needs at runtime.

mod c;

use super::ast::{CodegenNode, Field, Param, Visibility};
use super::codegen_utils::{expression_to_string, state_var_init_value, type_to_string};
use crate::frame_c::compiler::frame_ast::{Expression, SystemAst, Type};
use crate::frame_c::visitors::TargetLanguage;
pub use c::generate_c_compartment_types;

/// Convert a Frame snake_case identifier to a PascalCase variant name
/// for use in the per-system FrameEvent enum (RFC-0025 Track B.1).
/// Examples: `bool_return` → `BoolReturn`, `tick` → `Tick`,
/// `get_status` → `GetStatus`. Exported pub(crate) so siblings
/// (rust_system.rs, system_codegen/*.rs, machinery/rust.rs) emit
/// matching variant names at construction and dispatch sites.
pub(crate) fn pascal_case_variant(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// Map a Frame `Type` to a Rust-native type spelling for use inside
/// generated structs (e.g. the per-state `XContext`). Mirrors the
/// conversions in `RustBackend::convert_type`, but lives here so that
/// raw-text codegen in `runtime.rs` doesn't have to round-trip through
/// the AST/CodegenNode pipeline. Untyped (`Type::Unknown`) state params
/// fall back to `String`, matching the dynamic backends' loosely-typed
/// `state_args` HashMap.
fn frame_type_to_rust_type(t: &Type) -> String {
    match t {
        Type::Custom(name) => match name.as_str() {
            "int" => "i64".to_string(),
            "float" => "f64".to_string(),
            "str" | "string" | "String" => "String".to_string(),
            "bool" => "bool".to_string(),
            "Any" => "String".to_string(),
            other => other.to_string(),
        },
        Type::Unknown => "String".to_string(),
    }
}

/// Default value (right-hand side of `Self { name: <init> }`) for a
/// Frame state-param `Type` in Rust. Mirrors `frame_type_to_rust_type`
/// for the type column. The real value is overwritten at the
/// transition site, so these are just neutral placeholders.
fn frame_type_to_rust_default(t: &Type) -> String {
    match t {
        Type::Custom(name) => match name.as_str() {
            "int" | "i32" | "i64" | "u32" | "u64" => "0".to_string(),
            "float" | "f32" | "f64" => "0.0".to_string(),
            "bool" => "false".to_string(),
            "str" | "string" | "String" | "Any" => "String::new()".to_string(),
            _ => "Default::default()".to_string(),
        },
        Type::Unknown => "String::new()".to_string(),
    }
}

/// Generate Rust runtime types for a system
///
/// This is the public entry point for generating the Frame runtime infrastructure
/// for Rust that matches the Python/TypeScript kernel/router/transition pattern.
///
/// Returns the Rust code for:
/// - FrameEvent struct with message field
/// - Compartment struct with state, state_vars, forward_event fields
/// - Context structs for states with state variables (for typed push/pop)
/// - StateContext enum for typed state variable storage
pub fn generate_rust_compartment_types(
    system: &SystemAst,
    arcanum: Option<&crate::frame_c::compiler::arcanum::Arcanum>,
) -> String {
    generate_rust_runtime_types(system, arcanum)
}

/// Generate FrameEvent class for Python/TypeScript
///
/// The FrameEvent class is a lean routing object:
/// - _message: string - Event name (e.g., "$>", "<$", "start")
/// - _parameters: dict - Event parameters (positional args as indexed dict)
///
/// Note: _return is NOT on FrameEvent - it's on FrameContext for proper reentrancy
///
/// Returns None for Rust (which uses a different pattern)
pub fn generate_frame_event_class(system: &SystemAst, lang: TargetLanguage) -> Option<CodegenNode> {
    // Rust uses a different pattern - return None
    if matches!(lang, TargetLanguage::Rust) {
        return None;
    }

    let class_name = format!("{}FrameEvent", system.name);

    // Constructor parameters: message and parameters
    let constructor_params = match lang {
        TargetLanguage::Python3 => vec![
            Param::new("message").with_type("str"),
            Param::new("parameters"),
        ],
        TargetLanguage::GDScript => vec![
            Param::new("message").with_type("String"),
            Param::new("parameters"),
        ],
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => vec![
            Param::new("message").with_type("string"),
            Param::new("parameters").with_type("any[]"),
        ],
        TargetLanguage::Dart => vec![
            Param::new("message").with_type("String"),
            Param::new("parameters").with_type("List<dynamic>"),
        ],
        TargetLanguage::Php => vec![
            Param::new("message"),
            Param::new("parameters").with_default(CodegenNode::null()),
        ],
        TargetLanguage::Ruby => vec![
            Param::new("message"),
            Param::new("parameters").with_default(CodegenNode::Array(vec![])),
        ],
        TargetLanguage::Lua => vec![Param::new("message"), Param::new("parameters")],
        // Static-typed languages generate FrameEvent as NativeBlock in generate_*_compartment_types()
        TargetLanguage::C
        | TargetLanguage::Cpp
        | TargetLanguage::Java
        | TargetLanguage::Kotlin
        | TargetLanguage::Swift
        | TargetLanguage::CSharp
        | TargetLanguage::Go => vec![],
        TargetLanguage::Rust => vec![], // Rust returns None earlier, but be explicit
        TargetLanguage::Erlang => vec![], // gen_statem: handled natively by erlang_system.rs
        TargetLanguage::Graphviz => unreachable!(),
    };

    // Constructor body: initialize fields (no _return - that's on FrameContext)
    let constructor_body = match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_message"),
                CodegenNode::ident("message"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_parameters"),
                CodegenNode::ident("parameters"),
            ),
        ],
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_message"),
                CodegenNode::ident("message"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_parameters"),
                CodegenNode::ident("parameters"),
            ),
        ],
        TargetLanguage::Dart => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_message"),
                CodegenNode::ident("message"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_parameters"),
                CodegenNode::ident("parameters"),
            ),
        ],
        TargetLanguage::Php => vec![CodegenNode::NativeBlock {
            code: "$this->_message = $message;\n$this->_parameters = $parameters ?? [];"
                .to_string(),
            span: None,
        }],
        TargetLanguage::Ruby => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_message"),
                CodegenNode::ident("message"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_parameters"),
                CodegenNode::ident("parameters"),
            ),
        ],
        TargetLanguage::Lua => vec![CodegenNode::NativeBlock {
            code: "self._message = message\nself._parameters = parameters or {}".to_string(),
            span: None,
        }],
        // Static-typed languages generate FrameEvent body as NativeBlock
        TargetLanguage::C
        | TargetLanguage::Cpp
        | TargetLanguage::Java
        | TargetLanguage::Kotlin
        | TargetLanguage::Swift
        | TargetLanguage::CSharp
        | TargetLanguage::Go => vec![],
        TargetLanguage::Rust => vec![],
        TargetLanguage::Erlang => vec![], // gen_statem: handled natively by erlang_system.rs
        TargetLanguage::Graphviz => unreachable!(),
    };

    // Fields for TypeScript/Dart (Python doesn't need field declarations)
    // Note: no _return field - that's on FrameContext for proper reentrancy
    let fields = if matches!(
        lang,
        TargetLanguage::TypeScript | TargetLanguage::JavaScript
    ) {
        vec![
            Field::new("_message")
                .with_type("string")
                .with_visibility(Visibility::Public),
            Field::new("_parameters")
                .with_type("any[]")
                .with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::Dart) {
        vec![
            Field::new("_message")
                .with_type("String")
                .with_visibility(Visibility::Public),
            Field::new("_parameters")
                .with_type("List<dynamic>")
                .with_visibility(Visibility::Public),
        ]
    } else if matches!(
        lang,
        TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang
    ) {
        vec![
            Field::new("_message").with_visibility(Visibility::Public),
            Field::new("_parameters").with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::GDScript) {
        vec![
            Field::new("_message").with_visibility(Visibility::Public),
            Field::new("_parameters").with_visibility(Visibility::Public),
        ]
    } else {
        vec![]
    };

    Some(CodegenNode::Class {
        name: class_name,
        fields,
        methods: vec![CodegenNode::Constructor {
            params: constructor_params,
            body: constructor_body,
            super_call: None,
        }],
        base_classes: vec![],
        is_abstract: false,
        derives: vec![],
        visibility: Visibility::Private,
    })
}

/// Generate FrameContext class for Python/TypeScript
///
/// The FrameContext class holds the call context for reentrancy support:
/// - event: FrameEvent - Reference to the interface event (message + parameters)
/// - _return: any - Return value slot (default or None)
/// - _data: dict - Call-scoped data (empty by default)
///
/// Context is pushed when interface is called, popped when it returns.
/// Lifecycle events ($>, <$) use the existing context without push/pop.
///
/// Returns None for Rust (which uses a different pattern)
pub fn generate_frame_context_class(
    system: &SystemAst,
    lang: TargetLanguage,
) -> Option<CodegenNode> {
    // Rust uses a different pattern - return None
    if matches!(lang, TargetLanguage::Rust) {
        return None;
    }

    let class_name = format!("{}FrameContext", system.name);
    let event_class = format!("{}FrameEvent", system.name);

    // Constructor parameters: event and optional default_return
    let constructor_params = match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => vec![
            Param::new("event").with_type(&event_class),
            Param::new("default_return"),
        ],
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => vec![
            Param::new("event").with_type(&event_class),
            Param::new("default_return").with_type("any"),
        ],
        TargetLanguage::Dart => vec![
            Param::new("event").with_type(&event_class),
            Param::new("default_return").with_type("dynamic"),
        ],
        TargetLanguage::Php => vec![
            Param::new("event"),
            Param::new("defaultReturn").with_default(CodegenNode::null()),
        ],
        TargetLanguage::Ruby => vec![
            Param::new("event"),
            Param::new("default_return").with_default(CodegenNode::null()),
        ],
        TargetLanguage::Lua => vec![Param::new("event"), Param::new("default_return")],
        // Static-typed languages generate FrameContext as NativeBlock
        TargetLanguage::C
        | TargetLanguage::Cpp
        | TargetLanguage::Java
        | TargetLanguage::Kotlin
        | TargetLanguage::Swift
        | TargetLanguage::CSharp
        | TargetLanguage::Go => vec![],
        TargetLanguage::Rust => vec![],
        TargetLanguage::Erlang => vec![], // gen_statem: handled natively by erlang_system.rs
        TargetLanguage::Graphviz => unreachable!(),
    };

    // Constructor body: initialize fields
    let constructor_body = match lang {
        TargetLanguage::Python3 => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "event"),
                CodegenNode::ident("event"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_return"),
                CodegenNode::ident("default_return"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_data"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_transitioned"),
                CodegenNode::ident("False"),
            ),
        ],
        TargetLanguage::GDScript => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "event"),
                CodegenNode::ident("event"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_return"),
                CodegenNode::ident("default_return"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_data"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_transitioned"),
                CodegenNode::ident("false"),
            ),
        ],
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "event"),
                CodegenNode::ident("event"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_return"),
                CodegenNode::ident("default_return"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_data"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_transitioned"),
                CodegenNode::ident("false"),
            ),
        ],
        TargetLanguage::Dart => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "event"),
                CodegenNode::ident("event"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_return"),
                CodegenNode::ident("default_return"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_data"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_transitioned"),
                CodegenNode::ident("false"),
            ),
        ],
        TargetLanguage::Php => vec![
            CodegenNode::NativeBlock {
                code: "$this->_event = $event;\n$this->_return = $defaultReturn;\n$this->_data = [];\n$this->_transitioned = false;".to_string(),
                span: None,
            },
        ],
        TargetLanguage::Ruby => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_event"),
                CodegenNode::ident("event"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_return"),
                CodegenNode::ident("default_return"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_data"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "_transitioned"),
                CodegenNode::ident("false"),
            ),
        ],
        TargetLanguage::Lua => vec![
            CodegenNode::NativeBlock {
                code: "self._event = event\nself._return = default_return\nself._data = {}\nself._transitioned = false".to_string(),
                span: None,
            },
        ],
        // Static-typed languages generate FrameContext body as NativeBlock
        TargetLanguage::C | TargetLanguage::Cpp | TargetLanguage::Java | TargetLanguage::Kotlin
            | TargetLanguage::Swift | TargetLanguage::CSharp | TargetLanguage::Go => vec![],
        TargetLanguage::Rust => vec![],
        TargetLanguage::Erlang => vec![], // gen_statem: handled natively by erlang_system.rs
        TargetLanguage::Graphviz => unreachable!(),
    };

    // Fields for TypeScript/JavaScript/Dart (Python doesn't need field declarations)
    let fields = if matches!(
        lang,
        TargetLanguage::TypeScript | TargetLanguage::JavaScript
    ) {
        vec![
            Field::new("event")
                .with_type(&event_class)
                .with_visibility(Visibility::Public),
            Field::new("_return")
                .with_type("any")
                .with_visibility(Visibility::Public),
            Field::new("_data")
                .with_type("Record<string, any>")
                .with_visibility(Visibility::Public),
            Field::new("_transitioned")
                .with_type("boolean")
                .with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::Dart) {
        vec![
            Field::new("event")
                .with_type(&event_class)
                .with_visibility(Visibility::Public),
            Field::new("_return")
                .with_type("dynamic")
                .with_visibility(Visibility::Public),
            Field::new("_data")
                .with_type("Map<String, dynamic>")
                .with_visibility(Visibility::Public),
            Field::new("_transitioned")
                .with_type("bool")
                .with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::Php) {
        vec![
            Field::new("_event").with_visibility(Visibility::Public),
            Field::new("_return").with_visibility(Visibility::Public),
            Field::new("_data").with_visibility(Visibility::Public),
            Field::new("_transitioned").with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::Ruby) {
        vec![
            Field::new("_event").with_visibility(Visibility::Public),
            Field::new("_return").with_visibility(Visibility::Public),
            Field::new("_data").with_visibility(Visibility::Public),
            Field::new("_transitioned").with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::GDScript) {
        vec![
            Field::new("event").with_visibility(Visibility::Public),
            Field::new("_return").with_visibility(Visibility::Public),
            Field::new("_data").with_visibility(Visibility::Public),
            Field::new("_transitioned").with_visibility(Visibility::Public),
        ]
    } else {
        vec![]
    };

    Some(CodegenNode::Class {
        name: class_name,
        fields,
        methods: vec![CodegenNode::Constructor {
            params: constructor_params,
            body: constructor_body,
            super_call: None,
        }],
        base_classes: vec![],
        is_abstract: false,
        derives: vec![],
        visibility: Visibility::Private,
    })
}

/// Generate Compartment class for Python/TypeScript
///
/// The Compartment class encapsulates all state-related data following the canonical 7-field model:
/// - state: string - Current state identifier
/// - state_args: dict - State parameters ($State(args))
/// - state_vars: dict - State variables ($.varName)
/// - enter_args: dict - Enter transition args (-> (args) $State)
/// - exit_args: dict - Exit transition args ((args) -> $State)
/// - forward_event: Event? - For event forwarding (-> =>)
/// - parent_compartment: Compartment? - For HSM parent state reference
///
/// Returns None for Rust (which uses the specialized enum-of-structs pattern)
pub fn generate_compartment_class(system: &SystemAst, lang: TargetLanguage) -> Option<CodegenNode> {
    // Rust uses a different pattern - return None
    if matches!(lang, TargetLanguage::Rust) {
        return None;
    }

    let class_name = format!("{}Compartment", system.name);

    // Constructor parameters: state and optional parent_compartment
    let constructor_params = match lang {
        TargetLanguage::Python3 => vec![
            Param::new("state").with_type("str"),
            Param::new("parent_compartment").with_default(CodegenNode::null()),
        ],
        TargetLanguage::GDScript => vec![
            Param::new("state").with_type("String"),
            Param::new("parent_compartment").with_default(CodegenNode::null()),
        ],
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => vec![
            Param::new("state").with_type("string"),
            Param::new("parent_compartment")
                .with_type(&format!("{} | null", class_name))
                .with_default(CodegenNode::null()),
        ],
        TargetLanguage::Dart => vec![
            Param::new("state").with_type("String"),
            Param::new("parent_compartment")
                .with_type(&format!("{}?", class_name))
                .with_default(CodegenNode::null()),
        ],
        TargetLanguage::Php => vec![
            Param::new("state"),
            Param::new("parent_compartment").with_default(CodegenNode::null()),
        ],
        TargetLanguage::Ruby => vec![
            Param::new("state"),
            Param::new("parent_compartment").with_default(CodegenNode::null()),
        ],
        TargetLanguage::Lua => vec![Param::new("state"), Param::new("parent_compartment")],
        // Static-typed languages generate Compartment as NativeBlock
        TargetLanguage::C
        | TargetLanguage::Cpp
        | TargetLanguage::Java
        | TargetLanguage::Kotlin
        | TargetLanguage::Swift
        | TargetLanguage::CSharp
        | TargetLanguage::Go => vec![Param::new("state").with_type("str")],
        TargetLanguage::Rust => vec![Param::new("state").with_type("str")],
        TargetLanguage::Erlang => vec![], // gen_statem: handled natively by erlang_system.rs
        TargetLanguage::Graphviz => unreachable!(),
    };

    // Constructor body: initialize all 7 fields
    let constructor_body = match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state"),
                CodegenNode::ident("state"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state_vars"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "enter_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "exit_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "forward_event"),
                CodegenNode::null(),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "parent_compartment"),
                CodegenNode::ident("parent_compartment"),
            ),
        ],
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state"),
                CodegenNode::ident("state"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state_vars"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "enter_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "exit_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "forward_event"),
                CodegenNode::null(),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "parent_compartment"),
                CodegenNode::ident("parent_compartment"),
            ),
        ],
        TargetLanguage::Dart => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state"),
                CodegenNode::ident("state"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state_vars"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "enter_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "exit_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "forward_event"),
                CodegenNode::null(),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "parent_compartment"),
                CodegenNode::ident("parent_compartment"),
            ),
        ],
        TargetLanguage::Php => vec![
            CodegenNode::NativeBlock {
                code: "$this->state = $state;\n$this->state_args = [];\n$this->state_vars = [];\n$this->enter_args = [];\n$this->exit_args = [];\n$this->forward_event = null;\n$this->parent_compartment = $parent_compartment;".to_string(),
                span: None,
            },
        ],
        TargetLanguage::Ruby => vec![
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state"),
                CodegenNode::ident("state"),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "state_vars"),
                CodegenNode::Dict(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "enter_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "exit_args"),
                CodegenNode::Array(vec![]),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "forward_event"),
                CodegenNode::null(),
            ),
            CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), "parent_compartment"),
                CodegenNode::ident("parent_compartment"),
            ),
        ],
        TargetLanguage::Lua => vec![
            CodegenNode::NativeBlock {
                code: "self.state = state\nself.state_args = {}\nself.state_vars = {}\nself.enter_args = {}\nself.exit_args = {}\nself.forward_event = nil\nself.parent_compartment = parent_compartment".to_string(),
                span: None,
            },
        ],
        // Static-typed languages generate Compartment body as NativeBlock
        TargetLanguage::C | TargetLanguage::Cpp | TargetLanguage::Java | TargetLanguage::Kotlin
            | TargetLanguage::Swift | TargetLanguage::CSharp | TargetLanguage::Go => vec![],
        TargetLanguage::Rust => vec![],
        TargetLanguage::Erlang => vec![], // gen_statem: handled natively by erlang_system.rs
        TargetLanguage::Graphviz => unreachable!(),
    };

    // Generate copy() method
    let copy_method = generate_compartment_copy_method(&class_name, lang);

    // Build the class
    let methods = vec![
        CodegenNode::Constructor {
            params: constructor_params,
            body: constructor_body,
            super_call: None,
        },
        copy_method,
    ];

    // Fields for TypeScript/JavaScript/Dart (Python doesn't need field declarations)
    let fields = if matches!(
        lang,
        TargetLanguage::TypeScript | TargetLanguage::JavaScript
    ) {
        vec![
            Field::new("state")
                .with_type("string")
                .with_visibility(Visibility::Public),
            // state_args / enter_args / exit_args are arrays (Frame passes
            // positional values); initialized as `[]` in the constructor and
            // consumed by FrameEvent which expects `any[]`. Declaring them as
            // Record<string, any> here produced a TS2345 type error under
            // strict type checking when the initializer `[]` (any[]) didn't
            // match. state_vars IS a map (name → value) so stays Record.
            Field::new("state_args")
                .with_type("any[]")
                .with_visibility(Visibility::Public),
            Field::new("state_vars")
                .with_type("Record<string, any>")
                .with_visibility(Visibility::Public),
            Field::new("enter_args")
                .with_type("any[]")
                .with_visibility(Visibility::Public),
            Field::new("exit_args")
                .with_type("any[]")
                .with_visibility(Visibility::Public),
            Field::new("forward_event")
                .with_type("any")
                .with_visibility(Visibility::Public),
            Field::new("parent_compartment")
                .with_type(&format!("{} | null", class_name))
                .with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::Dart) {
        vec![
            Field::new("state")
                .with_type("String")
                .with_visibility(Visibility::Public),
            Field::new("state_args")
                .with_type("List<dynamic>")
                .with_visibility(Visibility::Public),
            Field::new("state_vars")
                .with_type("Map<String, dynamic>")
                .with_visibility(Visibility::Public),
            Field::new("enter_args")
                .with_type("List<dynamic>")
                .with_visibility(Visibility::Public),
            Field::new("exit_args")
                .with_type("List<dynamic>")
                .with_visibility(Visibility::Public),
            Field::new("forward_event")
                .with_type("dynamic")
                .with_visibility(Visibility::Public),
            Field::new("parent_compartment")
                .with_type(&format!("{}?", class_name))
                .with_visibility(Visibility::Public),
        ]
    } else if matches!(
        lang,
        TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang
    ) {
        vec![
            Field::new("state").with_visibility(Visibility::Public),
            Field::new("state_args").with_visibility(Visibility::Public),
            Field::new("state_vars").with_visibility(Visibility::Public),
            Field::new("enter_args").with_visibility(Visibility::Public),
            Field::new("exit_args").with_visibility(Visibility::Public),
            Field::new("forward_event").with_visibility(Visibility::Public),
            Field::new("parent_compartment").with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::GDScript) {
        vec![
            Field::new("state").with_visibility(Visibility::Public),
            Field::new("state_args").with_visibility(Visibility::Public),
            Field::new("state_vars").with_visibility(Visibility::Public),
            Field::new("enter_args").with_visibility(Visibility::Public),
            Field::new("exit_args").with_visibility(Visibility::Public),
            Field::new("forward_event").with_visibility(Visibility::Public),
            Field::new("parent_compartment").with_visibility(Visibility::Public),
        ]
    } else if matches!(lang, TargetLanguage::CSharp) {
        vec![
            Field::new("state")
                .with_type("string")
                .with_visibility(Visibility::Public),
            Field::new("state_args")
                .with_type("List<object>")
                .with_visibility(Visibility::Public),
            Field::new("state_vars")
                .with_type("Dictionary<string, object>")
                .with_visibility(Visibility::Public),
            Field::new("enter_args")
                .with_type("List<object>")
                .with_visibility(Visibility::Public),
            Field::new("exit_args")
                .with_type("List<object>")
                .with_visibility(Visibility::Public),
            Field::new("forward_event")
                .with_type(&format!("{}FrameEvent?", system.name))
                .with_visibility(Visibility::Public),
            Field::new("parent_compartment")
                .with_type(&format!("{}?", &class_name))
                .with_visibility(Visibility::Public),
        ]
    } else {
        vec![]
    };

    Some(CodegenNode::Class {
        name: class_name,
        fields,
        methods,
        base_classes: vec![],
        is_abstract: false,
        derives: vec![],
        visibility: Visibility::Private,
    })
}

/// Generate the copy() method for Compartment class
fn generate_compartment_copy_method(class_name: &str, lang: TargetLanguage) -> CodegenNode {
    let copy_body = match lang {
        TargetLanguage::Python3 => {
            // Python: c = {Class}Compartment(self.state, self.parent_compartment); c.state_args = self.state_args.copy(); ...
            vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"c = {}(self.state, self.parent_compartment)
c.state_args = self.state_args.copy()
c.state_vars = self.state_vars.copy()
c.enter_args = self.enter_args.copy()
c.exit_args = self.exit_args.copy()
c.forward_event = self.forward_event
return c"#,
                    class_name
                ),
                span: None,
            }]
        }
        TargetLanguage::GDScript => {
            // GDScript: c = {Class}.new(self.state, self.parent_compartment); c.state_args = self.state_args.duplicate(); ...
            vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"var c = {}.new(self.state, self.parent_compartment)
c.state_args = self.state_args.duplicate()
c.state_vars = self.state_vars.duplicate()
c.enter_args = self.enter_args.duplicate()
c.exit_args = self.exit_args.duplicate()
c.forward_event = self.forward_event
return c"#,
                    class_name
                ),
                span: None,
            }]
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            // TypeScript/JavaScript: const c = new {Class}(this.state, this.parent_compartment); c.state_args = {...this.state_args}; ...
            vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"const c = new {}(this.state, this.parent_compartment);
c.state_args = {{...this.state_args}};
c.state_vars = {{...this.state_vars}};
c.enter_args = {{...this.enter_args}};
c.exit_args = {{...this.exit_args}};
c.forward_event = this.forward_event;
return c;"#,
                    class_name
                ),
                span: None,
            }]
        }
        TargetLanguage::Dart => {
            // Dart: final c = {Class}(this.state, this.parent_compartment); c.state_args = Map.from(this.state_args); ...
            vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"final c = {}(this.state, this.parent_compartment);
c.state_args = List<dynamic>.from(this.state_args);
c.state_vars = Map<String, dynamic>.from(this.state_vars);
c.enter_args = List<dynamic>.from(this.enter_args);
c.exit_args = List<dynamic>.from(this.exit_args);
c.forward_event = this.forward_event;
return c;"#,
                    class_name
                ),
                span: None,
            }]
        }
        TargetLanguage::Php => {
            vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"$c = new {}($this->state, $this->parent_compartment);
$c->state_args = $this->state_args;
$c->state_vars = $this->state_vars;
$c->enter_args = $this->enter_args;
$c->exit_args = $this->exit_args;
$c->forward_event = $this->forward_event;
return $c;"#,
                    class_name
                ),
                span: None,
            }]
        }
        TargetLanguage::Ruby => {
            vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"c = {}.new(@state, @parent_compartment)
c.state_args = @state_args.dup
c.state_vars = @state_vars.dup
c.enter_args = @enter_args.dup
c.exit_args = @exit_args.dup
c.forward_event = @forward_event
c"#,
                    class_name
                ),
                span: None,
            }]
        }
        TargetLanguage::Lua => {
            vec![CodegenNode::NativeBlock {
                code: format!(
                    r#"local c = {}.new(self.state, self.parent_compartment)
c.state_args = {{}}
for k, v in pairs(self.state_args) do c.state_args[k] = v end
c.state_vars = {{}}
for k, v in pairs(self.state_vars) do c.state_vars[k] = v end
c.enter_args = {{}}
for k, v in pairs(self.enter_args) do c.enter_args[k] = v end
c.exit_args = {{}}
for k, v in pairs(self.exit_args) do c.exit_args[k] = v end
c.forward_event = self.forward_event
return c"#,
                    class_name
                ),
                span: None,
            }]
        }
        // Static-typed languages generate copy() as NativeBlock in their own functions
        TargetLanguage::C
        | TargetLanguage::Cpp
        | TargetLanguage::Java
        | TargetLanguage::Kotlin
        | TargetLanguage::Swift
        | TargetLanguage::CSharp
        | TargetLanguage::Go => {
            vec![CodegenNode::comment(
                "copy() generated in language-specific compartment types",
            )]
        }
        TargetLanguage::Rust => vec![],
        TargetLanguage::Erlang => vec![], // gen_statem: handled natively by erlang_system.rs
        TargetLanguage::Graphviz => unreachable!(),
    };

    // Use string annotation for Python to avoid forward reference issues
    let return_type = match lang {
        TargetLanguage::Python3 => format!("'{}'", class_name), // 'ClassName' forward reference
        TargetLanguage::GDScript => class_name.to_string(), // GDScript doesn't use string forward references
        TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Php
        | TargetLanguage::Dart
        | TargetLanguage::Ruby
        | TargetLanguage::Java
        | TargetLanguage::Kotlin
        | TargetLanguage::Swift
        | TargetLanguage::CSharp
        | TargetLanguage::Go
        | TargetLanguage::C
        | TargetLanguage::Cpp
        | TargetLanguage::Rust
        | TargetLanguage::Lua => class_name.to_string(),
        TargetLanguage::Erlang => String::new(), // gen_statem: handled natively by erlang_system.rs
        TargetLanguage::Graphviz => unreachable!(),
    };

    CodegenNode::Method {
        name: "copy".to_string(),
        params: vec![],
        return_type: Some(return_type),
        body: copy_body,
        is_async: false,
        is_static: false,
        visibility: Visibility::Public,
        decorators: vec![],
    }
}

/// Generate Rust runtime types (FrameEvent and Compartment structs)
///
/// Generates the standard Frame runtime infrastructure for Rust:
/// - FooFrameEvent struct with message field
/// - FooCompartment struct with state and state_vars fields
/// - Context structs for states with state variables (for typed push/pop)
fn generate_rust_runtime_types(
    system: &SystemAst,
    arcanum: Option<&crate::frame_c::compiler::arcanum::Arcanum>,
) -> String {
    let system_name = &system.name;
    let mut code = String::new();

    // Build the effective event set: union of `system.interface`,
    // every (non-lifecycle) handler in `system.machine.states`, AND
    // every handler tracked by arcanum (when available).
    //
    // The arcanum walk is the critical piece for `@@[target(...)]`
    // attribute filtering — `filter_by_target_attribute` retains
    // only matching methods/handlers in the AST, but arcanum was
    // built BEFORE the filter ran, so it still has the
    // target-filtered handlers. The state dispatcher walks arcanum
    // (not the filtered AST), so the enum must enumerate the same
    // set or arcanum-emitted dispatch arms reference variants that
    // don't exist.
    let mut effective_events: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for method in &system.interface {
        seen.insert(method.name.clone());
        let fields: Vec<(String, String)> = method
            .params
            .iter()
            .map(|p| (p.name.clone(), frame_type_to_rust_type(&p.param_type)))
            .collect();
        effective_events.push((method.name.clone(), fields));
    }
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            for handler in &state.handlers {
                // Skip lifecycle events — they get FrameEnter/FrameExit
                // variants below, not per-method variants.
                if handler.event == "$>" || handler.event == "$<" {
                    continue;
                }
                if seen.insert(handler.event.clone()) {
                    let fields: Vec<(String, String)> = handler
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), frame_type_to_rust_type(&p.param_type)))
                        .collect();
                    effective_events.push((handler.event.clone(), fields));
                }
            }
        }
    }
    if let Some(arc) = arcanum {
        for state_entry in arc.get_enhanced_states(system_name) {
            for (_event_key, handler_entry) in &state_entry.handlers {
                if handler_entry.event == "$>" || handler_entry.event == "$<" {
                    continue;
                }
                if seen.insert(handler_entry.event.clone()) {
                    // Map FrameSymbol's symbol_type (String) to Rust type.
                    let fields: Vec<(String, String)> = handler_entry
                        .params
                        .iter()
                        .map(|p| {
                            let ty = p
                                .symbol_type
                                .as_deref()
                                .map(|s| match s {
                                    "int" => "i64".to_string(),
                                    "float" => "f64".to_string(),
                                    "str" | "string" => "String".to_string(),
                                    "bool" => "bool".to_string(),
                                    other => other.to_string(),
                                })
                                .unwrap_or_else(|| "String".to_string());
                            (p.name.clone(), ty)
                        })
                        .collect();
                    effective_events.push((handler_entry.event.clone(), fields));
                }
            }
        }
    }

    // Generate FrameEvent enum (RFC-0025 Track B.1).
    //
    // One variant per effective interface event, carrying that
    // event's typed parameters as named fields. Plus two lifecycle
    // variants (FrameEnter / FrameExit) that carry their args as
    // `Vec<String>` — lifecycle args round-trip through persist as
    // strings, so we keep the stringly path for those; user-facing
    // event parameters are now fully typed, eliminating the
    // Box<dyn Any> downcast lies that the old struct+Vec<Box<dyn Any>>
    // shape required.
    code.push_str("#[derive(Clone, Debug)]\n");
    code.push_str("#[allow(dead_code, non_camel_case_types)]\n");
    code.push_str(&format!("enum {}FrameEvent {{\n", system_name));
    for (event_name, fields) in &effective_events {
        let variant = pascal_case_variant(event_name);
        // Always emit struct-style variants (even for no-param events
        // emit `Tick {}` rather than `Tick`) so dispatch and the name()
        // method can consistently use the `{ .. }` pattern. This
        // matters when the same event name is declared with params in
        // one state and without in another (Frame allows it; the
        // no-param handler ignores the carried fields).
        let field_strs: Vec<String> = fields
            .iter()
            .map(|(n, ty)| format!("{}: {}", n, ty))
            .collect();
        code.push_str(&format!(
            "    {} {{ {} }},\n",
            variant,
            field_strs.join(", ")
        ));
    }
    // Lifecycle event variants — carry Vec<String> args, parsed at
    // dispatch via .parse::<T>(). Names chosen to not collide with
    // any reasonable Frame interface method name (PascalCase
    // `FrameEnter` / `FrameExit` would only collide with user
    // methods literally named `frame_enter` / `frame_exit`).
    code.push_str("    FrameEnter { args: Vec<String> },\n");
    code.push_str("    FrameExit { args: Vec<String> },\n");
    code.push_str("}\n\n");

    // Build return-type lookup keyed by event name, merging
    // system.interface, machine.state.handlers, AND arcanum
    // handlers (when present). Arcanum captures target-filtered
    // entries that AST walks miss (see effective_events comment).
    let mut return_types: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for method in &system.interface {
        if let Some(ref rt) = method.return_type {
            return_types.insert(method.name.clone(), frame_type_to_rust_type(rt));
        }
    }
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            for handler in &state.handlers {
                if handler.event == "$>" || handler.event == "$<" {
                    continue;
                }
                if let Some(ref rt) = handler.return_type {
                    return_types
                        .entry(handler.event.clone())
                        .or_insert_with(|| frame_type_to_rust_type(rt));
                }
            }
        }
    }
    if let Some(arc) = arcanum {
        for state_entry in arc.get_enhanced_states(system_name) {
            for (_event_key, handler_entry) in &state_entry.handlers {
                if handler_entry.event == "$>" || handler_entry.event == "$<" {
                    continue;
                }
                if let Some(ref rt) = handler_entry.return_type {
                    return_types
                        .entry(handler_entry.event.clone())
                        .or_insert_with(|| match rt.as_str() {
                            "int" => "i64".to_string(),
                            "float" => "f64".to_string(),
                            "str" | "string" => "String".to_string(),
                            "bool" => "bool".to_string(),
                            other => other.to_string(),
                        });
                }
            }
        }
    }

    // Generate FrameReturn enum (RFC-0025 Track B.2).
    //
    // One variant per interface method with a non-void return type,
    // carrying the declared return value as a single positional
    // payload. Replaces the old `Option<Box<dyn Any>>` _return slot
    // with type discipline at every interface-method @@:return write
    // site.
    //
    // The `_Lifecycle(Rc<dyn std::any::Any>)` variant is the typed-
    // dispatch escape hatch: lifecycle handlers ($>/$<) can also
    // write `@@:return` (for the in-flight interface method that
    // triggered the transition), but at codegen time framec doesn't
    // statically know which interface method that is. We keep these
    // specific writes type-erased and unwrap at the interface read.
    code.push_str("#[derive(Clone)]\n");
    code.push_str("#[allow(dead_code, non_camel_case_types)]\n");
    code.push_str(&format!("enum {}FrameReturn {{\n", system_name));
    let mut return_event_names: Vec<&String> = return_types.keys().collect();
    return_event_names.sort();
    for event_name in &return_event_names {
        let variant = pascal_case_variant(event_name);
        let return_ty = &return_types[*event_name];
        code.push_str(&format!("    {}({}),\n", variant, return_ty));
    }
    code.push_str("    _Lifecycle(std::rc::Rc<dyn std::any::Any>),\n");
    code.push_str("}\n\n");

    // Generate name() method — Frame source spelling of the event,
    // returned by `@@:event` reads. User events: the interface method
    // name verbatim. Lifecycle: `$>` / `<$` per Frame syntax
    // (note: `$<` is the event KEY, `<$` is the message spelling —
    // historical asymmetry preserved for caller compatibility).
    code.push_str("#[allow(dead_code)]\n");
    code.push_str(&format!("impl {}FrameEvent {{\n", system_name));
    code.push_str("    fn name(&self) -> &'static str {\n");
    code.push_str("        match self {\n");
    for (event_name, _fields) in &effective_events {
        let variant = pascal_case_variant(event_name);
        // Always use `{ .. }` since all variants are struct-shaped.
        code.push_str(&format!(
            "            {}FrameEvent::{} {{ .. }} => \"{}\",\n",
            system_name, variant, event_name
        ));
    }
    code.push_str(&format!(
        "            {}FrameEvent::FrameEnter {{ .. }} => \"$>\",\n",
        system_name
    ));
    code.push_str(&format!(
        "            {}FrameEvent::FrameExit {{ .. }} => \"<$\",\n",
        system_name
    ));
    code.push_str("        }\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // Generate FrameValue enum (RFC-0025 Track B.3).
    //
    // Closed value enum for the call-scoped `@@:data` map. This is
    // the *one* place runtime variance is genuinely needed — keys
    // are dynamic and types are determined at the assignment site.
    // The 6 variants cover the Frame v4 base type set; List and
    // Dict recurse for nested values.
    code.push_str("#[derive(Clone, Debug)]\n");
    code.push_str("#[allow(dead_code, non_camel_case_types)]\n");
    code.push_str(&format!("enum {}FrameValue {{\n", system_name));
    code.push_str("    Int(i64),\n");
    code.push_str("    Float(f64),\n");
    code.push_str("    Bool(bool),\n");
    code.push_str("    Str(String),\n");
    code.push_str("    List(Vec<Self>),\n");
    code.push_str("    Dict(std::collections::HashMap<String, Self>),\n");
    code.push_str("}\n\n");

    // Generate FrameContext struct (call context for reentrancy).
    // RFC-0020: `event` is `Rc<FrameEvent>` so the wrapper can pass
    // `&Rc<FrameEvent>` to the kernel without aliasing through `self`.
    // RFC-0025 Track B.2: `_return` is `Option<<System>FrameReturn>`
    // (typed enum) instead of `Option<Box<dyn Any>>`.
    // RFC-0025 Track B.3: `_data` is `HashMap<String, <System>FrameValue>`
    // (value enum) instead of `HashMap<String, Box<dyn Any>>`.
    code.push_str("#[allow(dead_code, non_camel_case_types)]\n");
    code.push_str(&format!("struct {}FrameContext {{\n", system_name));
    code.push_str(&format!(
        "    event: std::rc::Rc<{}FrameEvent>,\n",
        system_name
    ));
    code.push_str(&format!(
        "    _return: Option<{}FrameReturn>,\n",
        system_name
    ));
    code.push_str(&format!(
        "    _data: std::collections::HashMap<String, {}FrameValue>,\n",
        system_name
    ));
    code.push_str("    _transitioned: bool,\n");
    code.push_str("}\n\n");

    // Generate FrameContext impl with new()
    code.push_str(&format!("impl {}FrameContext {{\n", system_name));
    code.push_str(&format!(
        "    fn new(event: std::rc::Rc<{}FrameEvent>, default_return: Option<{}FrameReturn>) -> Self {{\n",
        system_name, system_name
    ));
    code.push_str("        Self {\n");
    code.push_str("            event,\n");
    code.push_str("            _return: default_return,\n");
    code.push_str("            _data: std::collections::HashMap::new(),\n");
    code.push_str("            _transitioned: false,\n");
    code.push_str("        }\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // Generate state context types (must come before Compartment which references them)
    // Context structs for states with state variables OR state params.
    // The XContext struct holds BOTH state vars (declared with `$.name`)
    // AND state params (declared on the state header like `$Counter(initial: int)`)
    // so that transitions of the form `-> $Counter(42)` can populate the
    // state's params via the typed enum-of-structs StateContext.
    if let Some(ref machine) = system.machine {
        let states_with_storage: Vec<_> = machine
            .states
            .iter()
            .filter(|s| !s.state_vars.is_empty() || !s.params.is_empty())
            .collect();

        for state in &states_with_storage {
            code.push_str(&format!(
                "#[derive(Clone)]\nstruct {}Context {{\n",
                state.name
            ));
            // State params first (they come from transitions or system header).
            // Rust requires concrete types, so we map Frame's portable
            // type names (`int`/`str`/`bool`) to Rust-native spellings
            // and fall back to `String` for untyped params.
            for p in &state.params {
                let type_str = frame_type_to_rust_type(&p.param_type);
                code.push_str(&format!("    {}: {},\n", p.name, type_str));
            }
            for var in &state.state_vars {
                // Use the Rust-aware type mapping so Frame's portable
                // types (str, int, bool) produce valid Rust struct
                // field types (String, i64, bool).
                let type_str = frame_type_to_rust_type(&var.var_type);
                code.push_str(&format!("    {}: {},\n", var.name, type_str));
            }
            code.push_str("}\n\n");

            // Manual Default impl with declared initializers (state vars)
            // and neutral defaults (state params — the real values come
            // from the transition site). Both state params and state
            // vars use the Rust-typed default helper so String fields
            // get `String::new()`, not `""` (`&str`).
            code.push_str(&format!("impl Default for {}Context {{\n", state.name));
            code.push_str("    fn default() -> Self {\n");
            code.push_str("        Self {\n");
            for p in &state.params {
                let init_val = frame_type_to_rust_default(&p.param_type);
                code.push_str(&format!("            {}: {},\n", p.name, init_val));
            }
            for var in &state.state_vars {
                let init_val = if let Some(ref init) = var.init {
                    super::codegen_utils::typed_init_expr(init, &var.var_type, TargetLanguage::Rust)
                } else {
                    frame_type_to_rust_default(&var.var_type)
                };
                code.push_str(&format!("            {}: {},\n", var.name, init_val));
            }
            code.push_str("        }\n");
            code.push_str("    }\n");
            code.push_str("}\n\n");
        }
    }

    // StateContext enum — typed state variable storage on the compartment.
    // A state has a context variant if it declares EITHER state vars or
    // state params. The variant carries the state's `XContext` struct.
    code.push_str(&format!(
        "#[allow(dead_code, non_camel_case_types)]\n#[derive(Clone)]\nenum {}StateContext {{\n",
        system_name
    ));
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            if state.state_vars.is_empty() && state.params.is_empty() {
                code.push_str(&format!("    {},\n", state.name));
            } else {
                code.push_str(&format!("    {}({}Context),\n", state.name, state.name));
            }
        }
    }
    code.push_str("    Empty,\n");
    code.push_str("}\n\n");

    // Default impl for StateContext
    if let Some(ref machine) = system.machine {
        if let Some(first_state) = machine.states.first() {
            code.push_str(&format!(
                "impl Default for {}StateContext {{\n",
                system_name
            ));
            code.push_str("    fn default() -> Self {\n");
            if first_state.state_vars.is_empty() && first_state.params.is_empty() {
                code.push_str(&format!(
                    "        {}StateContext::{}\n",
                    system_name, first_state.name
                ));
            } else {
                code.push_str(&format!(
                    "        {}StateContext::{}({}Context::default())\n",
                    system_name, first_state.name, first_state.name
                ));
            }
            code.push_str("    }\n");
            code.push_str("}\n\n");
        }
    }

    // Generate Compartment struct
    code.push_str(&format!(
        "#[allow(dead_code, non_camel_case_types)]\n#[derive(Clone)]\nstruct {}Compartment {{\n",
        system_name
    ));
    code.push_str("    state: String,\n");
    code.push_str(&format!(
        "    state_context: {}StateContext,\n",
        system_name
    ));
    code.push_str("    enter_args: Vec<String>,\n");
    code.push_str("    exit_args: Vec<String>,\n");
    code.push_str(&format!(
        "    forward_event: Option<{}FrameEvent>,\n",
        system_name
    ));
    code.push_str(&format!(
        "    parent_compartment: Option<Box<{}Compartment>>,\n",
        system_name
    ));
    code.push_str("}\n\n");

    // Generate Compartment impl with new()
    // new() automatically sets state_context to the correct variant with defaults.
    // A state has a context variant if it declares EITHER state vars or
    // state params (the same condition used by the StateContext enum).
    code.push_str(&format!("impl {}Compartment {{\n", system_name));
    code.push_str("    fn new(state: &str) -> Self {\n");
    code.push_str(&format!("        let state_context = match state {{\n"));
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            if state.state_vars.is_empty() && state.params.is_empty() {
                code.push_str(&format!(
                    "            \"{}\" => {}StateContext::{},\n",
                    state.name, system_name, state.name
                ));
            } else {
                code.push_str(&format!(
                    "            \"{}\" => {}StateContext::{}({}Context::default()),\n",
                    state.name, system_name, state.name, state.name
                ));
            }
        }
    }
    code.push_str(&format!(
        "            _ => {}StateContext::Empty,\n",
        system_name
    ));
    code.push_str("        };\n");
    code.push_str("        Self {\n");
    code.push_str("            state: state.to_string(),\n");
    code.push_str("            state_context,\n");
    code.push_str("            enter_args: Vec::new(),\n");
    code.push_str("            exit_args: Vec::new(),\n");
    code.push_str("            forward_event: None,\n");
    code.push_str("            parent_compartment: None,\n");
    code.push_str("        }\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    code
}

/// Generate C++17 runtime types (FrameEvent, FrameContext, Compartment classes)
pub fn generate_cpp_compartment_types(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();

    // FrameEvent class
    code.push_str(&format!("class {sys}FrameEvent {{\n"));
    code.push_str("public:\n");
    code.push_str("    std::string _message;\n");
    code.push_str("    std::vector<std::any> _parameters;\n");
    code.push_str(&format!(
        "\n    {sys}FrameEvent(const std::string& message, std::vector<std::any> params = {{}})\n"
    ));
    code.push_str("        : _message(message), _parameters(std::move(params)) {}\n");
    code.push_str("};\n\n");

    // FrameContext class
    code.push_str(&format!("class {sys}FrameContext {{\n"));
    code.push_str("public:\n");
    code.push_str(&format!("    {sys}FrameEvent _event;\n"));
    code.push_str("    std::any _return;\n");
    code.push_str("    std::unordered_map<std::string, std::any> _data;\n");
    code.push_str("    bool _transitioned = false;\n");
    code.push_str(&format!(
        "\n    {sys}FrameContext({sys}FrameEvent event, std::any default_return = {{}})\n"
    ));
    code.push_str("        : _event(std::move(event)), _return(std::move(default_return)) {}\n");
    code.push_str("};\n\n");

    // Compartment class
    code.push_str(&format!("class {sys}Compartment {{\n"));
    code.push_str("public:\n");
    code.push_str("    std::string state;\n");
    code.push_str("    std::vector<std::any> state_args;\n");
    code.push_str("    std::unordered_map<std::string, std::any> state_vars;\n");
    code.push_str("    std::vector<std::any> enter_args;\n");
    code.push_str("    std::vector<std::any> exit_args;\n");
    code.push_str(&format!(
        "    std::unique_ptr<{sys}FrameEvent> forward_event;\n"
    ));
    // shared_ptr: parent_compartment is shared across HSM siblings
    // and state stack entries. shared_ptr ref counting handles cleanup.
    code.push_str(&format!(
        "    std::shared_ptr<{sys}Compartment> parent_compartment;\n"
    ));
    code.push_str(&format!(
        "\n    explicit {sys}Compartment(const std::string& state) : state(state) {{}}\n"
    ));
    code.push_str("};\n\n");

    code
}

/// Generate Java runtime types (FrameEvent, FrameContext, Compartment classes)
pub fn generate_java_compartment_types(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();

    // FrameEvent class. Fully-qualified `java.util.*` types: framec
    // emits no `import java.util.*;` so the generated code stays
    // self-contained (user prolog passthrough lands first; Java's
    // strict "`package` before imports" rule survives Oceans Model).
    code.push_str(&format!("class {sys}FrameEvent {{\n"));
    code.push_str("    String _message;\n");
    code.push_str("    java.util.ArrayList<Object> _parameters;\n");
    code.push_str(&format!("\n    {sys}FrameEvent(String message) {{\n"));
    code.push_str("        this._message = message;\n");
    code.push_str("        this._parameters = new java.util.ArrayList<>();\n");
    code.push_str("    }\n\n");
    code.push_str(&format!(
        "    {sys}FrameEvent(String message, java.util.ArrayList<Object> parameters) {{\n"
    ));
    code.push_str("        this._message = message;\n");
    code.push_str("        this._parameters = parameters;\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // FrameContext class
    code.push_str(&format!("class {sys}FrameContext {{\n"));
    code.push_str(&format!("    {sys}FrameEvent _event;\n"));
    code.push_str("    Object _return;\n");
    code.push_str("    java.util.HashMap<String, Object> _data;\n");
    code.push_str("    boolean _transitioned = false;\n");
    code.push_str(&format!(
        "\n    {sys}FrameContext({sys}FrameEvent event, Object defaultReturn) {{\n"
    ));
    code.push_str("        this._event = event;\n");
    code.push_str("        this._return = defaultReturn;\n");
    code.push_str("        this._data = new java.util.HashMap<>();\n");
    code.push_str("        this._transitioned = false;\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // Compartment class
    code.push_str(&format!("class {sys}Compartment {{\n"));
    code.push_str("    String state;\n");
    code.push_str("    java.util.ArrayList<Object> state_args;\n");
    code.push_str("    java.util.HashMap<String, Object> state_vars;\n");
    code.push_str("    java.util.ArrayList<Object> enter_args;\n");
    code.push_str("    java.util.ArrayList<Object> exit_args;\n");
    code.push_str(&format!("    {sys}FrameEvent forward_event;\n"));
    code.push_str(&format!("    {sys}Compartment parent_compartment;\n"));
    code.push_str(&format!("\n    {sys}Compartment(String state) {{\n"));
    code.push_str("        this.state = state;\n");
    code.push_str("        this.state_args = new java.util.ArrayList<>();\n");
    code.push_str("        this.state_vars = new java.util.HashMap<>();\n");
    code.push_str("        this.enter_args = new java.util.ArrayList<>();\n");
    code.push_str("        this.exit_args = new java.util.ArrayList<>();\n");
    code.push_str("        this.forward_event = null;\n");
    code.push_str("        this.parent_compartment = null;\n");
    code.push_str("    }\n\n");
    code.push_str(&format!("    {sys}Compartment copy() {{\n"));
    code.push_str(&format!(
        "        {sys}Compartment c = new {sys}Compartment(this.state);\n"
    ));
    code.push_str("        c.state_args = new java.util.ArrayList<>(this.state_args);\n");
    code.push_str("        c.state_vars = new java.util.HashMap<>(this.state_vars);\n");
    code.push_str("        c.enter_args = new java.util.ArrayList<>(this.enter_args);\n");
    code.push_str("        c.exit_args = new java.util.ArrayList<>(this.exit_args);\n");
    code.push_str("        c.forward_event = this.forward_event;\n");
    code.push_str("        c.parent_compartment = this.parent_compartment;\n");
    code.push_str("        return c;\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    code
}

/// Generate Kotlin runtime types (FrameEvent, FrameContext, Compartment classes)
pub fn generate_kotlin_compartment_types(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();

    // FrameEvent class — Kotlin: no semicolons, no `new`, `fun` keyword
    code.push_str(&format!("class {sys}FrameEvent(val _message: String, val _parameters: MutableList<Any?> = mutableListOf())\n\n"));

    // FrameContext class
    code.push_str(&format!(
        "class {sys}FrameContext(val _event: {sys}FrameEvent, var _return: Any? = null) {{\n"
    ));
    code.push_str("    val _data: MutableMap<String, Any?> = mutableMapOf()\n");
    code.push_str("    var _transitioned: Boolean = false\n");
    code.push_str("}\n\n");

    // Compartment class
    code.push_str(&format!("class {sys}Compartment(val state: String) {{\n"));
    code.push_str("    val state_args: MutableList<Any?> = mutableListOf()\n");
    code.push_str("    val state_vars: MutableMap<String, Any?> = mutableMapOf()\n");
    code.push_str("    val enter_args: MutableList<Any?> = mutableListOf()\n");
    code.push_str("    val exit_args: MutableList<Any?> = mutableListOf()\n");
    code.push_str(&format!("    var forward_event: {sys}FrameEvent? = null\n"));
    code.push_str(&format!(
        "    var parent_compartment: {sys}Compartment? = null\n"
    ));
    code.push_str(&format!("\n    fun copy(): {sys}Compartment {{\n"));
    code.push_str(&format!("        val c = {sys}Compartment(this.state)\n"));
    code.push_str("        c.state_args.addAll(this.state_args)\n");
    code.push_str("        c.state_vars.putAll(this.state_vars)\n");
    code.push_str("        c.enter_args.addAll(this.enter_args)\n");
    code.push_str("        c.exit_args.addAll(this.exit_args)\n");
    code.push_str("        c.forward_event = this.forward_event\n");
    code.push_str("        c.parent_compartment = this.parent_compartment\n");
    code.push_str("        return c\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    code
}

/// Generate Swift runtime types (FrameEvent, FrameContext, Compartment classes)
pub fn generate_swift_compartment_types(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();

    // FrameEvent class — Swift: no semicolons, no `new`, `func` keyword
    code.push_str(&format!("class {sys}FrameEvent {{\n"));
    code.push_str("    var _message: String\n");
    code.push_str("    var _parameters: [Any]\n\n");
    code.push_str(&format!(
        "    init(message: String, parameters: [Any] = []) {{\n"
    ));
    code.push_str("        self._message = message\n");
    code.push_str("        self._parameters = parameters\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // FrameContext class
    code.push_str(&format!("class {sys}FrameContext {{\n"));
    code.push_str(&format!("    var _event: {sys}FrameEvent\n"));
    code.push_str("    var _return: Any?\n");
    code.push_str("    var _data: [String: Any] = [:]\n");
    code.push_str("    var _transitioned: Bool = false\n\n");
    code.push_str(&format!(
        "    init(event: {sys}FrameEvent, defaultReturn: Any? = nil) {{\n"
    ));
    code.push_str("        self._event = event\n");
    code.push_str("        self._return = defaultReturn\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // Compartment class
    code.push_str(&format!("class {sys}Compartment {{\n"));
    code.push_str("    var state: String\n");
    code.push_str("    var state_args: [Any] = []\n");
    code.push_str("    var state_vars: [String: Any] = [:]\n");
    code.push_str("    var enter_args: [Any] = []\n");
    code.push_str("    var exit_args: [Any] = []\n");
    code.push_str(&format!("    var forward_event: {sys}FrameEvent?\n"));
    code.push_str(&format!(
        "    var parent_compartment: {sys}Compartment?\n\n"
    ));
    code.push_str("    init(state: String) {\n");
    code.push_str("        self.state = state\n");
    code.push_str("    }\n\n");
    code.push_str(&format!("    func copy() -> {sys}Compartment {{\n"));
    code.push_str(&format!(
        "        let c = {sys}Compartment(state: self.state)\n"
    ));
    code.push_str("        c.state_args = self.state_args\n");
    code.push_str("        c.state_vars = self.state_vars\n");
    code.push_str("        c.enter_args = self.enter_args\n");
    code.push_str("        c.exit_args = self.exit_args\n");
    code.push_str("        c.forward_event = self.forward_event\n");
    code.push_str("        c.parent_compartment = self.parent_compartment\n");
    code.push_str("        return c\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    code
}

/// Generate C# runtime types (FrameEvent, FrameContext, Compartment classes)
pub fn generate_csharp_compartment_types(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();

    // FrameEvent class
    code.push_str(&format!("class {sys}FrameEvent {{\n"));
    code.push_str("    public string _message;\n");
    code.push_str("    public List<object> _parameters;\n");
    code.push_str(&format!(
        "\n    public {sys}FrameEvent(string message) {{\n"
    ));
    code.push_str("        this._message = message;\n");
    code.push_str("        this._parameters = new List<object>();\n");
    code.push_str("    }\n\n");
    code.push_str(&format!(
        "    public {sys}FrameEvent(string message, List<object> parameters) {{\n"
    ));
    code.push_str("        this._message = message;\n");
    code.push_str("        this._parameters = parameters;\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // FrameContext class
    code.push_str(&format!("class {sys}FrameContext {{\n"));
    code.push_str(&format!("    public {sys}FrameEvent _event;\n"));
    code.push_str("    public object? _return;\n");
    code.push_str("    public Dictionary<string, object> _data;\n");
    code.push_str("    public bool _transitioned = false;\n");
    code.push_str(&format!(
        "\n    public {sys}FrameContext({sys}FrameEvent ev, object? defaultReturn = null) {{\n"
    ));
    code.push_str("        this._event = ev;\n");
    code.push_str("        this._return = defaultReturn;\n");
    code.push_str("        this._data = new Dictionary<string, object>();\n");
    code.push_str("        this._transitioned = false;\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // Compartment class
    code.push_str(&format!("class {sys}Compartment {{\n"));
    code.push_str("    public string state;\n");
    code.push_str("    public List<object> state_args;\n");
    code.push_str("    public Dictionary<string, object> state_vars;\n");
    code.push_str("    public List<object> enter_args;\n");
    code.push_str("    public List<object> exit_args;\n");
    code.push_str(&format!("    public {sys}FrameEvent? forward_event;\n"));
    code.push_str(&format!(
        "    public {sys}Compartment? parent_compartment;\n"
    ));
    code.push_str(&format!("\n    public {sys}Compartment(string state) {{\n"));
    code.push_str("        this.state = state;\n");
    code.push_str("        this.state_args = new List<object>();\n");
    code.push_str("        this.state_vars = new Dictionary<string, object>();\n");
    code.push_str("        this.enter_args = new List<object>();\n");
    code.push_str("        this.exit_args = new List<object>();\n");
    code.push_str("        this.forward_event = null;\n");
    code.push_str("        this.parent_compartment = null;\n");
    code.push_str("    }\n\n");
    code.push_str(&format!("    public {sys}Compartment Copy() {{\n"));
    code.push_str(&format!(
        "        {sys}Compartment c = new {sys}Compartment(this.state);\n"
    ));
    code.push_str("        c.state_args = new List<object>(this.state_args);\n");
    code.push_str("        c.state_vars = new Dictionary<string, object>(this.state_vars);\n");
    code.push_str("        c.enter_args = new List<object>(this.enter_args);\n");
    code.push_str("        c.exit_args = new List<object>(this.exit_args);\n");
    code.push_str("        c.forward_event = this.forward_event;\n");
    code.push_str("        c.parent_compartment = this.parent_compartment;\n");
    code.push_str("        return c;\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    code
}

/// Generate Go runtime types (FrameEvent, FrameContext, Compartment structs)
pub fn generate_go_compartment_types(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();

    // FrameEvent struct
    code.push_str(&format!("type {}FrameEvent struct {{\n", sys));
    code.push_str("    _message    string\n");
    code.push_str("    _parameters []any\n");
    code.push_str("}\n\n");

    // FrameContext struct
    code.push_str(&format!("type {}FrameContext struct {{\n", sys));
    code.push_str(&format!("    _event  {}FrameEvent\n", sys));
    code.push_str("    _return       any\n");
    code.push_str("    _data         map[string]any\n");
    code.push_str("    _transitioned bool\n");
    code.push_str("}\n\n");

    // Compartment struct
    code.push_str(&format!("type {}Compartment struct {{\n", sys));
    code.push_str("    state            string\n");
    code.push_str("    stateArgs        []any\n");
    code.push_str("    stateVars        map[string]any\n");
    code.push_str("    enterArgs        []any\n");
    code.push_str("    exitArgs         []any\n");
    code.push_str(&format!("    forwardEvent     *{}FrameEvent\n", sys));
    code.push_str(&format!("    parentCompartment *{}Compartment\n", sys));
    code.push_str("}\n\n");

    // Compartment constructor helper
    code.push_str(&format!(
        "func new{}Compartment(state string) *{}Compartment {{\n",
        sys, sys
    ));
    code.push_str(&format!("    return &{}Compartment{{\n", sys));
    code.push_str("        state:    state,\n");
    code.push_str("        stateArgs: []any{},\n");
    code.push_str("        stateVars: make(map[string]any),\n");
    code.push_str("        enterArgs: []any{},\n");
    code.push_str("        exitArgs:  []any{},\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");

    // Compartment copy method
    code.push_str(&format!(
        "func (c *{}Compartment) copy() *{}Compartment {{\n",
        sys, sys
    ));
    code.push_str(&format!("    nc := &{}Compartment{{\n", sys));
    code.push_str("        state: c.state,\n");
    code.push_str("        stateArgs: append([]any{}, c.stateArgs...),\n");
    code.push_str("        stateVars: make(map[string]any),\n");
    code.push_str("        enterArgs: append([]any{}, c.enterArgs...),\n");
    code.push_str("        exitArgs:  append([]any{}, c.exitArgs...),\n");
    code.push_str("        forwardEvent:     c.forwardEvent,\n");
    code.push_str("        parentCompartment: c.parentCompartment,\n");
    code.push_str("    }\n");
    code.push_str("    for k, v := range c.stateVars { nc.stateVars[k] = v }\n");
    code.push_str("    return nc\n");
    code.push_str("}\n\n");

    code
}
