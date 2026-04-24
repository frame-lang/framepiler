//! Shared codegen utilities.
//!
//! Functions and types used across multiple codegen modules:
//! system_codegen, frame_expansion, runtime, erlang_system.

use super::ast::CodegenNode;
use crate::frame_c::compiler::frame_ast::{BinaryOp, Expression, Literal, Type, UnaryOp};
use crate::frame_c::visitors::TargetLanguage;

#[derive(Clone, Default)]
pub(crate) struct HandlerContext {
    pub system_name: String,
    pub state_name: String,
    pub event_name: String,
    pub parent_state: Option<String>,
    /// Set of defined system names in the module (for @@System() validation)
    pub defined_systems: std::collections::HashSet<String>,
    /// True if we're in a state handler that has __sv_comp available for HSM state var access
    pub use_sv_comp: bool,
    /// True when emitting the body of a per-handler method (new architecture
    /// — see docs/frame_runtime.md § "Dispatch Model"). In this mode, the
    /// handler receives a `compartment` parameter that already points at its
    /// own state's compartment (HSM forwards pre-shift it via
    /// `compartment.parent_compartment` at each `=> $^` site). State-var
    /// access therefore emits `compartment.state_vars[…]` directly — no
    /// `__sv_comp` local, no HSM walk preamble. When false, legacy
    /// monolithic dispatch semantics apply (the dispatcher computes
    /// `__sv_comp` once and handler bodies reference it).
    pub per_handler: bool,
    /// State variable types for type-aware expansion (e.g., C++ std::any_cast<Type>)
    pub state_var_types: std::collections::HashMap<String, String>,
    /// Map from state name to its declared param names (in declaration order).
    /// Used by transition codegen to convert positional state args
    /// (`-> $S(42)`) into named writes (`state_args["the_param_name"] = 42`),
    /// matching the named convention used by the state dispatch reader.
    pub state_param_names: std::collections::HashMap<String, Vec<String>>,
    /// Map from state name to its enter handler's declared param names.
    /// Used by transition codegen to convert positional enter args
    /// (`-> "1, 2" $S`) into named writes (`enter_args["the_param_name"] = 1`),
    /// matching the named convention used by enter-handler binding code.
    pub state_enter_param_names: std::collections::HashMap<String, Vec<String>>,
    /// Map from state name to its exit handler's declared param names.
    /// Used by transition codegen to convert positional exit args
    /// (`("a", b) -> $S`) into named writes
    /// (`exit_args["the_param_name"] = ...`), matching the named
    /// convention the dispatch reader uses for exit handlers.
    pub state_exit_param_names: std::collections::HashMap<String, Vec<String>>,
    /// Map from event name to its interface method's declared param names.
    /// Used by @@:params.name to resolve named parameter to positional index.
    pub event_param_names: std::collections::HashMap<String, Vec<String>>,
    /// Declared return type of the handler currently being expanded.
    /// Used by the C backend to branch on `float`/`double` when emitting
    /// `@@:(expr)` so doubles survive the `void*` return slot.
    pub current_return_type: Option<String>,
}

/// Get default initialization value for a type
pub(crate) fn state_var_init_value(var_type: &Type, lang: TargetLanguage) -> String {
    match var_type {
        Type::Custom(name) => {
            match name.to_lowercase().as_str() {
                "int" | "i32" | "i64" | "u32" | "u64" | "number" => "0".to_string(),
                "float" | "f32" | "f64" => "0.0".to_string(),
                "bool" | "boolean" => match lang {
                    TargetLanguage::Python3 => "False".to_string(),
                    TargetLanguage::GDScript
                    | TargetLanguage::TypeScript
                    | TargetLanguage::JavaScript
                    | TargetLanguage::Rust
                    | TargetLanguage::C
                    | TargetLanguage::Cpp
                    | TargetLanguage::Java
                    | TargetLanguage::Kotlin
                    | TargetLanguage::Swift
                    | TargetLanguage::CSharp
                    | TargetLanguage::Go
                    | TargetLanguage::Php
                    | TargetLanguage::Ruby
                    | TargetLanguage::Erlang
                    | TargetLanguage::Lua
                    | TargetLanguage::Dart => "false".to_string(),
                    TargetLanguage::Graphviz => unreachable!(),
                },
                "str" | "string" => match lang {
                    // Rust: `""` is `&str`, not `String`. The Default impl
                    // for typed XContext structs needs a `String` value.
                    TargetLanguage::Rust => "String::new()".to_string(),
                    // C++: `""` is `const char*`, not `std::string`. Values
                    // stored in `std::any("")` fail `std::any_cast<std::string>`.
                    TargetLanguage::Cpp => "std::string()".to_string(),
                    _ => "\"\"".to_string(),
                },
                "list" | "array" => match lang {
                    TargetLanguage::Python3 | TargetLanguage::GDScript => "[]".to_string(),
                    TargetLanguage::Rust => "Vec::new()".to_string(),
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => "[]".to_string(),
                    TargetLanguage::Java => "new ArrayList<>()".to_string(),
                    TargetLanguage::Kotlin => "mutableListOf()".to_string(),
                    TargetLanguage::Swift => "[]".to_string(),
                    TargetLanguage::CSharp => "new List<object>()".to_string(),
                    TargetLanguage::Cpp => "std::vector<std::any>()".to_string(),
                    TargetLanguage::Go => "[]interface{}{}".to_string(),
                    TargetLanguage::Php => "[]".to_string(),
                    TargetLanguage::Ruby | TargetLanguage::Lua => "{}".to_string(),
                    TargetLanguage::C => "NULL".to_string(),
                    TargetLanguage::Erlang => "[]".to_string(),
                    TargetLanguage::Graphviz => unreachable!(),
                },
                "dict" | "dictionary" | "map" => match lang {
                    TargetLanguage::Python3 => "{}".to_string(),
                    TargetLanguage::GDScript => "{}".to_string(),
                    TargetLanguage::Rust => "HashMap::new()".to_string(),
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript => "{}".to_string(),
                    TargetLanguage::Java => "new HashMap<>()".to_string(),
                    TargetLanguage::Kotlin => "mutableMapOf()".to_string(),
                    TargetLanguage::Swift => "[:]".to_string(),
                    TargetLanguage::CSharp => "new Dictionary<string, object>()".to_string(),
                    TargetLanguage::Cpp => "std::unordered_map<std::string, std::any>()".to_string(),
                    TargetLanguage::Go => "map[string]interface{}{}".to_string(),
                    TargetLanguage::Php => "[]".to_string(),
                    TargetLanguage::Ruby => "{}".to_string(),
                    TargetLanguage::Lua => "{}".to_string(),
                    TargetLanguage::Dart => "{}".to_string(),
                    TargetLanguage::C => "NULL".to_string(),
                    TargetLanguage::Erlang => "#{}".to_string(),
                    TargetLanguage::Graphviz => unreachable!(),
                },
                "set" => match lang {
                    TargetLanguage::Python3 => "set()".to_string(),
                    TargetLanguage::GDScript => "{}".to_string(),
                    TargetLanguage::Rust => "HashSet::new()".to_string(),
                    TargetLanguage::TypeScript | TargetLanguage::JavaScript => "new Set()".to_string(),
                    TargetLanguage::Java => "new HashSet<>()".to_string(),
                    TargetLanguage::Kotlin => "mutableSetOf()".to_string(),
                    TargetLanguage::Swift => "Set<AnyHashable>()".to_string(),
                    TargetLanguage::CSharp => "new HashSet<object>()".to_string(),
                    TargetLanguage::Dart => "<dynamic>{}".to_string(),
                    _ => "null".to_string(),
                },
                _ => match lang {
                    TargetLanguage::Python3 | TargetLanguage::Rust => "None".to_string(),
                    TargetLanguage::Cpp => "nullptr".to_string(),
                    TargetLanguage::Go
                    | TargetLanguage::Swift
                    | TargetLanguage::Ruby
                    | TargetLanguage::Lua => "nil".to_string(),
                    TargetLanguage::C => "NULL".to_string(),
                    TargetLanguage::Erlang => "undefined".to_string(),
                    TargetLanguage::GDScript
                    | TargetLanguage::Dart
                    | TargetLanguage::TypeScript
                    | TargetLanguage::JavaScript
                    | TargetLanguage::Java
                    | TargetLanguage::Kotlin
                    | TargetLanguage::CSharp
                    | TargetLanguage::Php => "null".to_string(),
                    TargetLanguage::Graphviz => unreachable!(),
                },
            }
        }
        Type::Unknown => match lang {
            TargetLanguage::Python3 | TargetLanguage::Rust => "None".to_string(),
            TargetLanguage::Cpp => "nullptr".to_string(),
            TargetLanguage::Go
            | TargetLanguage::Swift
            | TargetLanguage::Ruby
            | TargetLanguage::Lua => "nil".to_string(),
            TargetLanguage::C => "NULL".to_string(),
            TargetLanguage::Erlang => "undefined".to_string(),
            TargetLanguage::GDScript
            | TargetLanguage::Dart
            | TargetLanguage::TypeScript
            | TargetLanguage::JavaScript
            | TargetLanguage::Java
            | TargetLanguage::Kotlin
            | TargetLanguage::CSharp
            | TargetLanguage::Php => "null".to_string(),
            TargetLanguage::Graphviz => unreachable!(),
        },
    }
}

/// Convert a state var init expression to a type-correct value for the
/// target language. Frame source uses portable expressions (`""` for
/// empty string, `0` for integer, `false` for bool). The target language
/// may need wrapping — e.g. Rust's struct fields are `String` not `&str`,
/// so a string literal `""` becomes `String::from("")`.
///
/// This is the canonical way to emit state var init values. It delegates
/// to `expression_to_string` for the base serialization, then wraps
/// based on declared type + target language.
pub(crate) fn typed_init_expr(expr: &Expression, var_type: &Type, lang: TargetLanguage) -> String {
    let raw = expression_to_string(expr, lang);
    let is_string_type = match var_type {
        Type::Custom(s) => matches!(s.to_lowercase().as_str(), "str" | "string"),
        Type::Unknown => false,
    };
    let is_string_literal = matches!(expr, Expression::Literal(Literal::String(_)));
    let is_int_literal = matches!(expr, Expression::Literal(Literal::Int(_)));

    match lang {
        // Rust: struct field is `String`, literal `""` is `&str` → wrap
        TargetLanguage::Rust if is_string_type && is_string_literal => {
            format!("String::from({})", raw)
        }
        // Rust: parser fallback — String field got Integer(0) because it
        // couldn't parse a Rust-specific constructor. Substitute default.
        TargetLanguage::Rust if is_string_type && is_int_literal => "String::new()".to_string(),
        // C++: std::any storage needs std::string, not const char*
        TargetLanguage::Cpp if is_string_type && is_string_literal => {
            format!("std::string({})", raw)
        }
        // All other languages: portable expression works as-is
        _ => raw,
    }
}

/// Convert an Expression to a string representation for inline code
pub(crate) fn expression_to_string(expr: &Expression, lang: TargetLanguage) -> String {
    match expr {
        Expression::Literal(lit) => match lit {
            Literal::Int(n) => n.to_string(),
            Literal::Float(f) => f.to_string(),
            Literal::String(s) => format!("\"{}\"", s),
            Literal::Bool(b) => match lang {
                TargetLanguage::Python3 => {
                    if *b {
                        "True".to_string()
                    } else {
                        "False".to_string()
                    }
                }
                TargetLanguage::GDScript
                | TargetLanguage::Dart
                | TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Rust
                | TargetLanguage::C
                | TargetLanguage::Cpp
                | TargetLanguage::Java
                | TargetLanguage::Kotlin
                | TargetLanguage::Swift
                | TargetLanguage::CSharp
                | TargetLanguage::Go
                | TargetLanguage::Php
                | TargetLanguage::Ruby
                | TargetLanguage::Erlang
                | TargetLanguage::Lua => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                TargetLanguage::Graphviz => unreachable!(),
            },
            Literal::Null => match lang {
                TargetLanguage::Python3 | TargetLanguage::Rust => "None".to_string(),
                TargetLanguage::Cpp => "nullptr".to_string(),
                TargetLanguage::Go
                | TargetLanguage::Swift
                | TargetLanguage::Ruby
                | TargetLanguage::Lua => "nil".to_string(),
                TargetLanguage::C => "NULL".to_string(),
                TargetLanguage::Erlang => "undefined".to_string(),
                TargetLanguage::GDScript
                | TargetLanguage::Dart
                | TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Java
                | TargetLanguage::Kotlin
                | TargetLanguage::CSharp
                | TargetLanguage::Php => "null".to_string(),
                TargetLanguage::Graphviz => unreachable!(),
            },
        },
        Expression::Var(name) => name.clone(),
        Expression::Binary { left, op, right } => {
            let op_str = match op {
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                BinaryOp::Mul => "*",
                BinaryOp::Div => "/",
                BinaryOp::Mod => "%",
                BinaryOp::Eq => "==",
                BinaryOp::Ne => "!=",
                BinaryOp::Lt => "<",
                BinaryOp::Le => "<=",
                BinaryOp::Gt => ">",
                BinaryOp::Ge => ">=",
                BinaryOp::And => "&&",
                BinaryOp::Or => "||",
                BinaryOp::BitAnd => "&",
                BinaryOp::BitOr => "|",
                BinaryOp::BitXor => "^",
            };
            format!(
                "{} {} {}",
                expression_to_string(left, lang),
                op_str,
                expression_to_string(right, lang)
            )
        }
        Expression::Unary { op, expr } => {
            let op_str = match op {
                UnaryOp::Not => "!",
                UnaryOp::Neg => "-",
                UnaryOp::BitNot => "~",
            };
            format!("{}{}", op_str, expression_to_string(expr, lang))
        }
        // NativeExpr carries the source verbatim — used for target-specific
        // literals (list/map/closure forms) that the Frame parser doesn't
        // structurally model. State-var inits use this path for things like
        // `$.items: list = [1, 2, 3]` — the bytes between `=` and the next
        // separator are captured as-is and emitted unchanged. The previous
        // fallback silently substituted `"0"` for anything non-primitive,
        // which turned `[1, 2, 3]` into an int and looked like a typo in
        // the generated code.
        Expression::NativeExpr(code) => code.clone(),
        _ => "0".to_string(), // Fallback for complex expressions (Call/Member/Index/Assign)
    }
}

/// Convert Type enum to string representation
pub(crate) fn type_to_string(t: &Type) -> String {
    match t {
        Type::Custom(name) => name.clone(),
        Type::Unknown => "Any".to_string(),
    }
}

/// Convert Expression AST to CodegenNode
pub(crate) fn convert_expression(expr: &Expression) -> CodegenNode {
    match expr {
        Expression::Var(name) => CodegenNode::ident(name),
        Expression::Literal(lit) => convert_literal(lit),
        Expression::Binary { left, op, right } => {
            let codegen_op = match op {
                BinaryOp::Add => crate::frame_c::compiler::codegen::ast::BinaryOp::Add,
                BinaryOp::Sub => crate::frame_c::compiler::codegen::ast::BinaryOp::Sub,
                BinaryOp::Mul => crate::frame_c::compiler::codegen::ast::BinaryOp::Mul,
                BinaryOp::Div => crate::frame_c::compiler::codegen::ast::BinaryOp::Div,
                BinaryOp::Mod => crate::frame_c::compiler::codegen::ast::BinaryOp::Mod,
                BinaryOp::Eq => crate::frame_c::compiler::codegen::ast::BinaryOp::Eq,
                BinaryOp::Ne => crate::frame_c::compiler::codegen::ast::BinaryOp::Ne,
                BinaryOp::Lt => crate::frame_c::compiler::codegen::ast::BinaryOp::Lt,
                BinaryOp::Le => crate::frame_c::compiler::codegen::ast::BinaryOp::Le,
                BinaryOp::Gt => crate::frame_c::compiler::codegen::ast::BinaryOp::Gt,
                BinaryOp::Ge => crate::frame_c::compiler::codegen::ast::BinaryOp::Ge,
                BinaryOp::And => crate::frame_c::compiler::codegen::ast::BinaryOp::And,
                BinaryOp::Or => crate::frame_c::compiler::codegen::ast::BinaryOp::Or,
                BinaryOp::BitAnd => crate::frame_c::compiler::codegen::ast::BinaryOp::BitAnd,
                BinaryOp::BitOr => crate::frame_c::compiler::codegen::ast::BinaryOp::BitOr,
                BinaryOp::BitXor => crate::frame_c::compiler::codegen::ast::BinaryOp::BitXor,
            };
            CodegenNode::BinaryOp {
                op: codegen_op,
                left: Box::new(convert_expression(left)),
                right: Box::new(convert_expression(right)),
            }
        }
        Expression::Unary { op, expr } => {
            let codegen_op = match op {
                UnaryOp::Neg => crate::frame_c::compiler::codegen::ast::UnaryOp::Neg,
                UnaryOp::Not => crate::frame_c::compiler::codegen::ast::UnaryOp::Not,
                UnaryOp::BitNot => crate::frame_c::compiler::codegen::ast::UnaryOp::BitNot,
            };
            CodegenNode::UnaryOp {
                op: codegen_op,
                operand: Box::new(convert_expression(expr)),
            }
        }
        Expression::Call { func, args } => CodegenNode::Call {
            target: Box::new(CodegenNode::ident(func)),
            args: args.iter().map(convert_expression).collect(),
        },
        Expression::Index { object, index } => CodegenNode::IndexAccess {
            object: Box::new(convert_expression(object)),
            index: Box::new(convert_expression(index)),
        },
        Expression::Member { object, field } => CodegenNode::FieldAccess {
            object: Box::new(convert_expression(object)),
            field: field.clone(),
        },
        Expression::Assign { target, value } => {
            CodegenNode::assign(convert_expression(target), convert_expression(value))
        }
        Expression::NativeExpr(code) => {
            // Pass through native expression verbatim
            CodegenNode::native(code)
        }
    }
}

/// Convert Literal to CodegenNode
pub(crate) fn convert_literal(lit: &Literal) -> CodegenNode {
    match lit {
        Literal::Int(n) => CodegenNode::int(*n),
        Literal::Float(f) => CodegenNode::float(*f),
        Literal::String(s) => CodegenNode::string(s),
        Literal::Bool(b) => CodegenNode::bool(*b),
        Literal::Null => CodegenNode::null(),
    }
}

/// Check if type string represents an integer type
pub(crate) fn is_int_type(type_str: &str) -> bool {
    matches!(
        type_str,
        "int"
            | "i32"
            | "i64"
            | "i8"
            | "i16"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
    )
}

/// Check if type string represents a float type
pub(crate) fn is_float_type(type_str: &str) -> bool {
    matches!(type_str, "float" | "double" | "f32" | "f64")
}

/// Check if type string represents a boolean type
pub(crate) fn is_bool_type(type_str: &str) -> bool {
    matches!(type_str, "bool" | "boolean" | "_Bool")
}

/// Check if type string represents a string type
pub(crate) fn is_string_type(type_str: &str) -> bool {
    matches!(type_str, "str" | "string" | "String" | "char*" | "&str")
}

/// Map a Frame type string to C# type for (Type) cast
pub(crate) fn csharp_map_type(t: &str) -> String {
    match t {
        "Any" => "object".to_string(),
        "str" | "string" | "String" => "string".to_string(),
        "int" | "i32" | "i64" | "number" => "int".to_string(),
        "float" | "f64" | "f32" => "double".to_string(),
        "bool" | "boolean" => "bool".to_string(),
        "void" => "void".to_string(),
        other => other.to_string(),
    }
}

/// Map a Frame type string to Java type for (Type) cast
pub(crate) fn java_map_type(t: &str) -> String {
    match t {
        "Any" => "Object".to_string(),
        "str" | "string" | "String" => "String".to_string(),
        "int" | "i32" | "i64" | "number" => "int".to_string(),
        "float" | "f64" | "f32" => "double".to_string(),
        "bool" | "boolean" => "boolean".to_string(),
        "void" => "void".to_string(),
        other => other.to_string(),
    }
}

/// Map a Frame type string to Kotlin type for cast
pub(crate) fn kotlin_map_type(t: &str) -> String {
    match t {
        "Any" | "Object" | "object" => "Any?".to_string(),
        "str" | "string" | "String" => "String".to_string(),
        "int" | "i32" | "i64" | "number" => "Int".to_string(),
        "float" | "f64" | "f32" | "double" => "Double".to_string(),
        "bool" | "boolean" => "Boolean".to_string(),
        "void" => "Unit".to_string(),
        other => other.to_string(),
    }
}

/// Map a Frame type string to Swift type for cast
pub(crate) fn swift_map_type(t: &str) -> String {
    let t = t.trim();
    // Handle nullable types: "Type | nil" -> "Type?"
    if let Some(pipe_pos) = t.find('|') {
        let base = t[..pipe_pos].trim();
        let suffix = t[pipe_pos + 1..].trim();
        if suffix == "nil" || suffix == "null" || suffix == "None" {
            return format!("{}?", swift_map_type(base));
        }
    }
    // Handle array types: "string[]" -> "[String]"
    if let Some(base) = t.strip_suffix("[]") {
        return format!("[{}]", swift_map_type(base));
    }
    match t {
        "Any" | "Object" | "object" => "Any".to_string(),
        "str" | "string" | "String" => "String".to_string(),
        "int" | "i32" | "i64" | "number" => "Int".to_string(),
        "float" | "f64" | "f32" | "double" => "Double".to_string(),
        "bool" | "boolean" | "Boolean" => "Bool".to_string(),
        "void" => "Void".to_string(),
        other => other.to_string(),
    }
}

/// Map a Frame type string to Go type for type assertion
pub(crate) fn go_map_type(t: &str) -> String {
    match t {
        "Any" | "object" | "Object" => "any".to_string(),
        "str" | "string" | "String" => "string".to_string(),
        "int" | "i32" | "i64" | "number" => "int".to_string(),
        "float" | "f64" | "f32" => "float64".to_string(),
        "bool" | "boolean" => "bool".to_string(),
        "void" | "None" => String::new(),
        other => other.to_string(),
    }
}

/// Map a Frame type string to C++ type for std::any_cast<T>
pub(crate) fn cpp_map_type(t: &str) -> String {
    match t {
        "Any" => "std::any".to_string(),
        "str" | "string" | "String" => "std::string".to_string(),
        "int" | "i32" | "i64" | "number" => "int".to_string(),
        "float" | "f64" | "f32" => "double".to_string(),
        "bool" | "boolean" => "bool".to_string(),
        "void" => "void".to_string(),
        other => other.to_string(), // Pass through C++ native types like std::string, std::vector<int>
    }
}

/// Wrap a C++ argument value for std::any storage.
/// String literals ("...") must be wrapped in std::string() because
/// std::any("literal") stores const char*, not std::string.
pub(crate) fn cpp_wrap_any_arg(arg: &str) -> String {
    let trimmed = arg.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        format!("std::string({})", trimmed)
    } else {
        trimmed.to_string()
    }
}

/// Convert Frame Type to C++ type string
pub(crate) fn type_to_cpp_string(t: &crate::frame_c::compiler::frame_ast::Type) -> String {
    match t {
        crate::frame_c::compiler::frame_ast::Type::Unknown => "void".to_string(),
        crate::frame_c::compiler::frame_ast::Type::Custom(s) => match s.as_str() {
            "str" | "string" | "String" => "std::string".to_string(),
            "int" | "i32" | "i64" => "int".to_string(),
            "float" | "f64" | "f32" => "double".to_string(),
            "bool" => "bool".to_string(),
            "void" => "void".to_string(),
            other => other.to_string(),
        },
    }
}

/// Convert CamelCase to snake_case for Erlang naming

/// Convert CamelCase to snake_case for Erlang naming
pub(crate) fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        if let Some(lc) = c.to_lowercase().next() {
            result.push(lc);
        }
    }
    result
}

// ─── String-aware literal replace ────────────────────────────────────
//
// Used by codegen branches that need to rewrite tokens inside generated
// native code (e.g. `self.` → `s.` for Go, `self.` → `Data#data.` for
// Erlang). A naive `str::replace` would false-match inside string and
// comment literals; this walker delegates string/comment skipping to
// the target language's `SyntaxSkipper` and only performs replacements
// in code positions.

/// Replace each `(needle, replacement)` pair in `code`, skipping over
/// string literals and comments using the language's `SyntaxSkipper`.
///
/// Matches are literal substrings (not regex); the first-matching rule
/// at each byte position wins, so order rules with longer / more-
/// specific needles first if overlapping.
///
/// Replacement is safe against:
///   - single- and double-quoted string contents (per language rules)
///   - single-line and block comments (per language rules)
///   - backslash escapes inside strings (Rust/JS/etc.)
///
/// This is the preferred alternative to `str::replace` in any codegen
/// branch operating on already-emitted native code.
pub fn replace_outside_strings_and_comments(
    code: &str,
    lang: crate::frame_c::visitors::TargetLanguage,
    replacements: &[(&str, &str)],
) -> String {
    let skipper = crate::frame_c::compiler::native_region_scanner::create_skipper(lang);
    let bytes = code.as_bytes();
    let end = bytes.len();
    let mut out = String::with_capacity(code.len());
    let mut i = 0;
    while i < end {
        // Delegate string-literal skipping to the language skipper.
        if let Some(next) = skipper.skip_string(bytes, i, end) {
            out.push_str(&code[i..next]);
            i = next;
            continue;
        }
        // Delegate comment skipping.
        if let Some(next) = skipper.skip_comment(bytes, i, end) {
            out.push_str(&code[i..next]);
            i = next;
            continue;
        }
        // Try each replacement rule; first match wins.
        let mut replaced = false;
        for (needle, replacement) in replacements {
            let nb = needle.as_bytes();
            if i + nb.len() <= end && &bytes[i..i + nb.len()] == nb {
                out.push_str(replacement);
                i += nb.len();
                replaced = true;
                break;
            }
        }
        if replaced {
            continue;
        }
        // Plain character — copy through. Advance by the full UTF-8
        // width so a multibyte sequence in an identifier or unquoted
        // literal isn't split across push calls.
        let width = utf8_char_len(bytes[i]);
        let next = (i + width).min(end);
        out.push_str(&code[i..next]);
        i = next;
    }
    out
}

/// Byte width of the UTF-8 character that starts with `first_byte`.
/// Returns 1 for ASCII and any unexpected continuation byte (which
/// should never appear at an iteration boundary since we always
/// advance by full character widths).
fn utf8_char_len(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else if first_byte < 0xC0 {
        1 // continuation byte — defensive
    } else if first_byte < 0xE0 {
        2
    } else if first_byte < 0xF0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::frame_ast::{Expression, Literal, Type};
    use crate::frame_c::visitors::TargetLanguage;

    // =========================================================
    // state_var_init_value — type-correct defaults per language
    // =========================================================

    #[test]
    fn test_state_var_init_string_rust() {
        assert_eq!(
            state_var_init_value(&Type::Custom("str".into()), TargetLanguage::Rust),
            "String::new()"
        );
        assert_eq!(
            state_var_init_value(&Type::Custom("string".into()), TargetLanguage::Rust),
            "String::new()"
        );
    }

    #[test]
    fn test_state_var_init_string_cpp() {
        assert_eq!(
            state_var_init_value(&Type::Custom("str".into()), TargetLanguage::Cpp),
            "std::string()"
        );
        assert_eq!(
            state_var_init_value(&Type::Custom("string".into()), TargetLanguage::Cpp),
            "std::string()"
        );
    }

    #[test]
    fn test_state_var_init_string_python() {
        assert_eq!(
            state_var_init_value(&Type::Custom("str".into()), TargetLanguage::Python3),
            "\"\""
        );
    }

    #[test]
    fn test_state_var_init_int() {
        assert_eq!(
            state_var_init_value(&Type::Custom("int".into()), TargetLanguage::Rust),
            "0"
        );
        assert_eq!(
            state_var_init_value(&Type::Custom("i64".into()), TargetLanguage::Cpp),
            "0"
        );
        assert_eq!(
            state_var_init_value(&Type::Custom("number".into()), TargetLanguage::Python3),
            "0"
        );
    }

    #[test]
    fn test_state_var_init_bool_python() {
        assert_eq!(
            state_var_init_value(&Type::Custom("bool".into()), TargetLanguage::Python3),
            "False"
        );
    }

    #[test]
    fn test_state_var_init_bool_rust() {
        assert_eq!(
            state_var_init_value(&Type::Custom("bool".into()), TargetLanguage::Rust),
            "false"
        );
    }

    #[test]
    fn test_state_var_init_unknown_rust() {
        assert_eq!(
            state_var_init_value(&Type::Unknown, TargetLanguage::Rust),
            "None"
        );
    }

    #[test]
    fn test_state_var_init_unknown_python() {
        assert_eq!(
            state_var_init_value(&Type::Unknown, TargetLanguage::Python3),
            "None"
        );
    }

    // =========================================================
    // typed_init_expr — type-aware wrapping for init expressions
    // =========================================================

    #[test]
    fn test_typed_init_expr_rust_string_literal() {
        let expr = Expression::Literal(Literal::String("hello".into()));
        let result = typed_init_expr(&expr, &Type::Custom("str".into()), TargetLanguage::Rust);
        assert_eq!(result, "String::from(\"hello\")");
    }

    #[test]
    fn test_typed_init_expr_cpp_string_literal() {
        let expr = Expression::Literal(Literal::String("hello".into()));
        let result = typed_init_expr(&expr, &Type::Custom("str".into()), TargetLanguage::Cpp);
        assert_eq!(result, "std::string(\"hello\")");
    }

    #[test]
    fn test_typed_init_expr_rust_int_fallback_for_string() {
        // Parser produced Integer(0) for unparseable String::new()
        let expr = Expression::Literal(Literal::Int(0));
        let result = typed_init_expr(&expr, &Type::Custom("str".into()), TargetLanguage::Rust);
        assert_eq!(result, "String::new()");
    }

    #[test]
    fn test_typed_init_expr_python_string_no_wrap() {
        let expr = Expression::Literal(Literal::String("hello".into()));
        let result = typed_init_expr(&expr, &Type::Custom("str".into()), TargetLanguage::Python3);
        assert_eq!(
            result, "\"hello\"",
            "Python should not wrap string literals"
        );
    }

    #[test]
    fn test_typed_init_expr_rust_int_for_int_no_wrap() {
        let expr = Expression::Literal(Literal::Int(42));
        let result = typed_init_expr(&expr, &Type::Custom("int".into()), TargetLanguage::Rust);
        assert_eq!(result, "42", "Int-typed int literal should not be wrapped");
    }

    #[test]
    fn test_typed_init_expr_rust_bool_no_wrap() {
        let expr = Expression::Literal(Literal::Bool(true));
        let result = typed_init_expr(&expr, &Type::Custom("bool".into()), TargetLanguage::Rust);
        assert_eq!(result, "true");
    }

    #[test]
    fn test_typed_init_expr_rust_empty_string() {
        let expr = Expression::Literal(Literal::String("".into()));
        let result = typed_init_expr(&expr, &Type::Custom("str".into()), TargetLanguage::Rust);
        assert_eq!(result, "String::from(\"\")");
    }

    #[test]
    fn test_typed_init_expr_cpp_empty_string() {
        let expr = Expression::Literal(Literal::String("".into()));
        let result = typed_init_expr(&expr, &Type::Custom("str".into()), TargetLanguage::Cpp);
        assert_eq!(result, "std::string(\"\")");
    }

    // =========================================================
    // cpp_wrap_any_arg — C++ std::any wrapping for string literals
    // =========================================================

    #[test]
    fn test_cpp_wrap_any_arg_string_literal() {
        assert_eq!(cpp_wrap_any_arg("\"hello\""), "std::string(\"hello\")");
    }

    #[test]
    fn test_cpp_wrap_any_arg_integer() {
        assert_eq!(cpp_wrap_any_arg("42"), "42");
    }

    #[test]
    fn test_cpp_wrap_any_arg_variable() {
        assert_eq!(cpp_wrap_any_arg("my_var"), "my_var");
    }

    #[test]
    fn test_cpp_wrap_any_arg_empty_string() {
        assert_eq!(cpp_wrap_any_arg("\"\""), "std::string(\"\")");
    }

    #[test]
    fn test_cpp_wrap_any_arg_with_whitespace() {
        assert_eq!(cpp_wrap_any_arg("  \"hello\"  "), "std::string(\"hello\")");
    }

    // =========================================================
    // replace_outside_strings_and_comments
    // =========================================================

    #[test]
    fn replace_outside_strings_basic_match() {
        // No strings or comments — straightforward replace.
        let out = replace_outside_strings_and_comments(
            "let x = self.y",
            TargetLanguage::Rust,
            &[("self.", "s.")],
        );
        assert_eq!(out, "let x = s.y");
    }

    #[test]
    fn replace_outside_strings_spares_string_literals() {
        // `self.` inside a double-quoted string must survive.
        let out = replace_outside_strings_and_comments(
            r#"let msg = "self.x is untouched"; self.y = 1;"#,
            TargetLanguage::Rust,
            &[("self.", "s.")],
        );
        assert_eq!(out, r#"let msg = "self.x is untouched"; s.y = 1;"#);
    }

    #[test]
    fn replace_outside_strings_spares_line_comments_rust() {
        let out = replace_outside_strings_and_comments(
            "// self.should stay\nself.y = 1;",
            TargetLanguage::Rust,
            &[("self.", "s.")],
        );
        assert_eq!(out, "// self.should stay\ns.y = 1;");
    }

    #[test]
    fn replace_outside_strings_handles_escapes() {
        // `\"` inside a string shouldn't terminate it early.
        let out = replace_outside_strings_and_comments(
            r#"let s = "outer \"self.inner\" still in string"; self.done = 1;"#,
            TargetLanguage::Rust,
            &[("self.", "s.")],
        );
        assert_eq!(
            out,
            r#"let s = "outer \"self.inner\" still in string"; s.done = 1;"#
        );
    }

    #[test]
    fn replace_outside_strings_multiple_rules() {
        // Multiple rules — first match at position wins.
        let out = replace_outside_strings_and_comments(
            "True False true",
            TargetLanguage::Rust,
            &[("True", "true"), ("False", "false")],
        );
        assert_eq!(out, "true false true");
    }

    #[test]
    fn replace_outside_strings_utf8_passthrough() {
        // Non-ASCII identifiers advance by full UTF-8 width.
        let out = replace_outside_strings_and_comments(
            "let café = self.x",
            TargetLanguage::Rust,
            &[("self.", "s.")],
        );
        assert_eq!(out, "let café = s.x");
    }

    #[test]
    fn replace_outside_strings_works_for_go() {
        // Go line comments use `//`, same as Rust.
        let out = replace_outside_strings_and_comments(
            "self.x = 1 // self.inside_comment\nself.y = 2",
            TargetLanguage::Go,
            &[("self.", "s.")],
        );
        assert_eq!(out, "s.x = 1 // self.inside_comment\ns.y = 2");
    }

    #[test]
    fn replace_outside_strings_works_for_erlang() {
        // Erlang uses `%` line comments — verify skipper respects language.
        let out = replace_outside_strings_and_comments(
            "X = self.a, % self.in_comment\nY = self.b.",
            TargetLanguage::Erlang,
            &[("self.", "Data#data.")],
        );
        assert_eq!(
            out,
            "X = Data#data.a, % self.in_comment\nY = Data#data.b."
        );
    }
}
