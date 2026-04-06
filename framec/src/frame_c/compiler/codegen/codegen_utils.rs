//! Shared codegen utilities.
//!
//! Functions and types used across multiple codegen modules:
//! system_codegen, frame_expansion, runtime, erlang_system.

use crate::frame_c::visitors::TargetLanguage;
use crate::frame_c::compiler::frame_ast::{Type, Expression, Literal, BinaryOp, UnaryOp};
use super::ast::CodegenNode;

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
    /// State variable types for type-aware expansion (e.g., C++ std::any_cast<Type>)
    pub state_var_types: std::collections::HashMap<String, String>,
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
                    TargetLanguage::GDScript | TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Rust
                        | TargetLanguage::C | TargetLanguage::Cpp | TargetLanguage::Java
                        | TargetLanguage::Kotlin | TargetLanguage::Swift | TargetLanguage::CSharp
                        | TargetLanguage::Go | TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang
                    | TargetLanguage::Lua | TargetLanguage::Dart => "false".to_string(),
                    TargetLanguage::Graphviz => unreachable!(),
                },
                "str" | "string" => "\"\"".to_string(),
                _ => match lang {
                    TargetLanguage::Python3 | TargetLanguage::Rust => "None".to_string(),
                    TargetLanguage::Cpp => "nullptr".to_string(),
                    TargetLanguage::Go | TargetLanguage::Swift | TargetLanguage::Ruby | TargetLanguage::Lua => "nil".to_string(),
                    TargetLanguage::C => "NULL".to_string(),
                    TargetLanguage::Erlang => "undefined".to_string(),
                    TargetLanguage::GDScript | TargetLanguage::Dart | TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Java
                        | TargetLanguage::Kotlin | TargetLanguage::CSharp | TargetLanguage::Php => "null".to_string(),
                    TargetLanguage::Graphviz => unreachable!(),
                },
            }
        }
        Type::Unknown => match lang {
            TargetLanguage::Python3 | TargetLanguage::Rust => "None".to_string(),
            TargetLanguage::Cpp => "nullptr".to_string(),
            TargetLanguage::Go | TargetLanguage::Swift | TargetLanguage::Ruby | TargetLanguage::Lua => "nil".to_string(),
            TargetLanguage::C => "NULL".to_string(),
            TargetLanguage::Erlang => "undefined".to_string(),
            TargetLanguage::GDScript | TargetLanguage::Dart | TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Java
                | TargetLanguage::Kotlin | TargetLanguage::CSharp | TargetLanguage::Php => "null".to_string(),
            TargetLanguage::Graphviz => unreachable!(),
        },
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
                TargetLanguage::Python3 => if *b { "True".to_string() } else { "False".to_string() },
                TargetLanguage::GDScript | TargetLanguage::Dart | TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Rust
                    | TargetLanguage::C | TargetLanguage::Cpp | TargetLanguage::Java
                    | TargetLanguage::Kotlin | TargetLanguage::Swift | TargetLanguage::CSharp
                    | TargetLanguage::Go | TargetLanguage::Php | TargetLanguage::Ruby | TargetLanguage::Erlang
                    | TargetLanguage::Lua => {
                    if *b { "true".to_string() } else { "false".to_string() }
                }
                TargetLanguage::Graphviz => unreachable!(),
            },
            Literal::Null => match lang {
                TargetLanguage::Python3 | TargetLanguage::Rust => "None".to_string(),
                TargetLanguage::Cpp => "nullptr".to_string(),
                TargetLanguage::Go | TargetLanguage::Swift | TargetLanguage::Ruby | TargetLanguage::Lua => "nil".to_string(),
                TargetLanguage::C => "NULL".to_string(),
                TargetLanguage::Erlang => "undefined".to_string(),
                TargetLanguage::GDScript | TargetLanguage::Dart | TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Java
                    | TargetLanguage::Kotlin | TargetLanguage::CSharp | TargetLanguage::Php => "null".to_string(),
                TargetLanguage::Graphviz => unreachable!(),
            },
        },
        Expression::Var(name) => name.clone(),
        Expression::Binary { left, op, right } => {
            let op_str = match op {
                BinaryOp::Add => "+", BinaryOp::Sub => "-", BinaryOp::Mul => "*",
                BinaryOp::Div => "/", BinaryOp::Mod => "%",
                BinaryOp::Eq => "==", BinaryOp::Ne => "!=",
                BinaryOp::Lt => "<", BinaryOp::Le => "<=",
                BinaryOp::Gt => ">", BinaryOp::Ge => ">=",
                BinaryOp::And => "&&", BinaryOp::Or => "||",
                BinaryOp::BitAnd => "&", BinaryOp::BitOr => "|", BinaryOp::BitXor => "^",
            };
            format!("{} {} {}",
                expression_to_string(left, lang),
                op_str,
                expression_to_string(right, lang))
        }
        Expression::Unary { op, expr } => {
            let op_str = match op {
                UnaryOp::Not => "!", UnaryOp::Neg => "-", UnaryOp::BitNot => "~",
            };
            format!("{}{}", op_str, expression_to_string(expr, lang))
        }
        _ => "0".to_string(), // Fallback for complex expressions
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
        Expression::Call { func, args } => {
            CodegenNode::Call {
                target: Box::new(CodegenNode::ident(func)),
                args: args.iter().map(convert_expression).collect(),
            }
        }
        Expression::Index { object, index } => {
            CodegenNode::IndexAccess {
                object: Box::new(convert_expression(object)),
                index: Box::new(convert_expression(index)),
            }
        }
        Expression::Member { object, field } => {
            CodegenNode::FieldAccess {
                object: Box::new(convert_expression(object)),
                field: field.clone(),
            }
        }
        Expression::Assign { target, value } => {
            CodegenNode::assign(
                convert_expression(target),
                convert_expression(value),
            )
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


/// Extract type from raw domain declaration
/// Handles formats: "name: type = init" (Frame) or "type name = init" (C-style)
pub(crate) fn extract_type_from_raw_domain(raw_code: &Option<String>, name: &str) -> String {
    match raw_code {
        Some(code) => {
            let code = code.trim();

            // Try Frame-style: "name: type = init" or "name: type"
            if let Some(colon_pos) = code.find(':') {
                let before_colon = &code[..colon_pos].trim();
                // Verify it's the variable name before the colon
                if before_colon.ends_with(name) || *before_colon == name {
                    let after_colon = &code[colon_pos + 1..];
                    // Extract type until '=' or end of line
                    let type_end = after_colon.find('=').unwrap_or(after_colon.len());
                    return after_colon[..type_end].trim().to_string();
                }
            }

            // Try C-style: "type name = init" - first word is type
            let first_word = code.split_whitespace().next().unwrap_or("");
            first_word.to_string()
        }
        None => String::new(),
    }
}


/// Check if type string represents an integer type
pub(crate) fn is_int_type(type_str: &str) -> bool {
    matches!(type_str, "int" | "i32" | "i64" | "i8" | "i16" | "u8" | "u16" | "u32" | "u64"
             | "int8_t" | "int16_t" | "int32_t" | "int64_t"
             | "uint8_t" | "uint16_t" | "uint32_t" | "uint64_t")
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
        crate::frame_c::compiler::frame_ast::Type::Custom(s) => {
            match s.as_str() {
                "str" | "string" | "String" => "std::string".to_string(),
                "int" | "i32" | "i64" => "int".to_string(),
                "float" | "f64" | "f32" => "double".to_string(),
                "bool" => "bool".to_string(),
                "void" => "void".to_string(),
                other => other.to_string(),
            }
        }
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

