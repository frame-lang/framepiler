//! Go code generation backend
//!
//! Go is structurally different from other backends:
//! - No classes — uses structs with method receivers
//! - No constructors — uses factory functions (NewXxx)
//! - No `this`/`self` — uses explicit receiver parameter (s *StructName)
//! - No semicolons — newline as statement terminator
//! - Type after name in declarations: `name Type` (not `Type name`)
//! - Visibility via capitalization (uppercase = exported)
//! - No generics in runtime types — uses `any` (interface{})

use crate::frame_c::visitors::TargetLanguage;
use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;

/// Go backend for code generation
pub struct GoBackend;

impl LanguageBackend for GoBackend {
    fn emit(&self, node: &CodegenNode, ctx: &mut EmitContext) -> String {
        let system_name = ctx.system_name.clone().unwrap_or_default();

        match node {
            CodegenNode::Module { imports, items } => {
                let mut result = String::new();
                for import in imports {
                    result.push_str(&self.emit(import, ctx));
                    result.push('\n');
                }
                if !imports.is_empty() && !items.is_empty() { result.push('\n'); }
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { result.push_str("\n\n"); }
                    result.push_str(&self.emit(item, ctx));
                }
                result
            }

            CodegenNode::Import { module, .. } => {
                format!("import \"{}\"", module)
            }

            CodegenNode::Class { name, fields, methods, .. } => {
                // Go: emit struct definition + separate method definitions
                let mut result = String::new();

                // Struct definition
                result.push_str(&format!("{}type {} struct {{\n", ctx.get_indent(), name));
                ctx.push_indent();
                for field in fields {
                    if let Some(ref raw_code) = field.raw_code {
                        result.push_str(&format!("{}{}\n", ctx.get_indent(), raw_code));
                    } else {
                        let type_ann = field.type_annotation.as_ref()
                            .map(|t| self.map_type(t))
                            .unwrap_or_else(|| "any".to_string());
                        // Go: lowercase fields are unexported (private)
                        let field_name = match field.visibility {
                            Visibility::Public => capitalize_first(&field.name),
                            _ => field.name.clone(),
                        };
                        result.push_str(&format!("{}{} {}\n", ctx.get_indent(), field_name, type_ann));
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));

                // Method definitions (outside struct)
                for method in methods {
                    result.push('\n');
                    result.push_str(&self.emit(method, ctx));
                }
                result
            }

            CodegenNode::Enum { name, variants } => {
                // Go: use const iota pattern
                let mut result = format!("{}type {} int\n\n", ctx.get_indent(), name);
                result.push_str(&format!("{}const (\n", ctx.get_indent()));
                ctx.push_indent();
                for (i, variant) in variants.iter().enumerate() {
                    if i == 0 {
                        result.push_str(&format!("{}{}_{} {} = iota\n", ctx.get_indent(), name, variant.name, name));
                    } else {
                        result.push_str(&format!("{}{}_{}\n", ctx.get_indent(), name, variant.name));
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{})\n", ctx.get_indent()));
                result
            }

            CodegenNode::Method { name, params, return_type, body, is_static, visibility, .. } => {
                // Go: static methods become package-level functions
                // Instance methods use receiver: func (s *ClassName) name(...)
                let go_name = match visibility {
                    Visibility::Public => capitalize_first(name),
                    _ => name.clone(),
                };

                let params_str = self.emit_params(params);
                let return_str = if let Some(rt) = return_type {
                    let mapped = self.map_type(rt);
                    if mapped.is_empty() { String::new() } else { format!(" {}", mapped) }
                } else {
                    String::new()
                };

                let mut result = if *is_static || system_name.is_empty() {
                    // Package-level function
                    format!("{}func {}({}){} {{\n", ctx.get_indent(), go_name, params_str, return_str)
                } else {
                    // Method with receiver
                    format!("{}func (s *{}) {}({}){} {{\n",
                        ctx.get_indent(), system_name, go_name, params_str, return_str)
                };

                ctx.push_indent();
                for stmt in body {
                    let stmt_str = self.emit(stmt, ctx);
                    result.push_str(&stmt_str);
                    if !stmt_str.trim().is_empty() {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Constructor { params, body, .. } => {
                // Go: factory function NewClassName() *ClassName
                let class_name = system_name.clone();
                let params_str = self.emit_params(params);

                let mut result = format!("{}func New{}({}) *{} {{\n",
                    ctx.get_indent(), class_name, params_str, class_name);
                ctx.push_indent();
                result.push_str(&format!("{}s := &{}{{}}\n", ctx.get_indent(), class_name));

                for stmt in body {
                    let stmt_str = self.emit(stmt, ctx);
                    result.push_str(&stmt_str);
                    if !stmt_str.trim().is_empty() {
                        result.push('\n');
                    }
                }
                result.push_str(&format!("{}return s\n", ctx.get_indent()));
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::VarDecl { name, type_annotation, init, is_const: _ } => {
                if let Some(init_expr) = init {
                    // Short declaration with init
                    format!("{}{} := {}", ctx.get_indent(), name, self.emit(init_expr, ctx))
                } else if let Some(ref type_ann) = type_annotation {
                    // Declaration without init — use var
                    format!("{}var {} {}", ctx.get_indent(), name, self.map_type(type_ann))
                } else {
                    format!("{}var {} any", ctx.get_indent(), name)
                }
            }

            CodegenNode::Assignment { target, value } => {
                format!("{}{} = {}", ctx.get_indent(), self.emit(target, ctx), self.emit(value, ctx))
            }

            CodegenNode::Return { value } => {
                if let Some(val) = value {
                    format!("{}return {}", ctx.get_indent(), self.emit(val, ctx))
                } else {
                    format!("{}return", ctx.get_indent())
                }
            }

            CodegenNode::If { condition, then_block, else_block } => {
                let mut result = format!("{}if {} {{\n", ctx.get_indent(), self.emit(condition, ctx));
                ctx.push_indent();
                for stmt in then_block {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();

                if let Some(else_stmts) = else_block {
                    result.push_str(&format!("{}}} else {{\n", ctx.get_indent()));
                    ctx.push_indent();
                    for stmt in else_stmts {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::Match { scrutinee, arms } => {
                let mut result = format!("{}switch {} {{\n", ctx.get_indent(), self.emit(scrutinee, ctx));
                for arm in arms {
                    result.push_str(&format!("{}case {}:\n", ctx.get_indent(), self.emit(&arm.pattern, ctx)));
                    ctx.push_indent();
                    for stmt in &arm.body {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::While { condition, body } => {
                let mut result = format!("{}for {} {{\n", ctx.get_indent(), self.emit(condition, ctx));
                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::For { var, iterable, body } => {
                let mut result = format!("{}for _, {} := range {} {{\n", ctx.get_indent(), var, self.emit(iterable, ctx));
                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::Break => format!("{}break", ctx.get_indent()),
            CodegenNode::Continue => format!("{}continue", ctx.get_indent()),
            CodegenNode::ExprStmt(expr) => format!("{}{}", ctx.get_indent(), self.emit(expr, ctx)),
            CodegenNode::Await(expr) => self.emit(expr, ctx),

            CodegenNode::Comment { text, .. } => format!("{}// {}", ctx.get_indent(), text),

            CodegenNode::Empty => String::new(),
            CodegenNode::Ident(name) => name.clone(),
            CodegenNode::Literal(lit) => self.emit_literal(lit, ctx),
            CodegenNode::BinaryOp { op, left, right } => self.emit_binary_op(op, left, right, ctx),
            CodegenNode::UnaryOp { op, operand } => self.emit_unary_op(op, operand, ctx),

            CodegenNode::Call { target, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}({})", self.emit(target, ctx), args_str.join(", "))
            }

            CodegenNode::MethodCall { object, method, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}.{}({})", self.emit(object, ctx), method, args_str.join(", "))
            }

            CodegenNode::FieldAccess { object, field } => {
                format!("{}.{}", self.emit(object, ctx), field)
            }

            CodegenNode::IndexAccess { object, index } => {
                format!("{}[{}]", self.emit(object, ctx), self.emit(index, ctx))
            }

            CodegenNode::SelfRef => "s".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                if elems.is_empty() {
                    "[]any{}".to_string()
                } else {
                    format!("[]any{{{}}}", elems.join(", "))
                }
            }

            CodegenNode::Dict(pairs) => {
                if pairs.is_empty() {
                    "map[string]any{}".to_string()
                } else {
                    let pairs_str: Vec<String> = pairs.iter().map(|(k, v)| {
                        format!("{}: {}", self.emit(k, ctx), self.emit(v, ctx))
                    }).collect();
                    format!("map[string]any{{{}}}", pairs_str.join(", "))
                }
            }

            CodegenNode::Ternary { condition, then_expr, else_expr } => {
                // Go has no ternary — use inline func
                format!("func() any {{ if {} {{ return {} }}; return {} }}()",
                    self.emit(condition, ctx), self.emit(then_expr, ctx), self.emit(else_expr, ctx))
            }

            CodegenNode::Lambda { params, body } => {
                let params_str = params.iter().map(|p| format!("{} any", p.name)).collect::<Vec<_>>().join(", ");
                format!("func({}) any {{ return {} }}", params_str, self.emit(body, ctx))
            }

            CodegenNode::Cast { expr, target_type } => {
                // Go type assertion
                format!("{}.({})", self.emit(expr, ctx), target_type)
            }

            CodegenNode::New { class, args } => {
                // Go: &ClassName{} or NewClassName(args)
                if args.is_empty() {
                    format!("&{}{{}}", class)
                } else {
                    let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                    format!("New{}({})", class, args_str.join(", "))
                }
            }

            // Frame-specific nodes
            CodegenNode::Transition { target_state, indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}s.__transition(&{}Compartment{{state: \"{}\"}})",
                    ind, system_name, target_state)
            }
            CodegenNode::ChangeState { target_state, indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}// change_state to {}", ind, target_state)
            }
            CodegenNode::Forward { indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }
            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}s._state_stack = append(s._state_stack, s.__compartment.copy())", ind)
            }
            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}__popped := s._state_stack[len(s._state_stack)-1]\n{}s._state_stack = s._state_stack[:len(s._state_stack)-1]\n{}s.__transition(__popped)",
                    ind, ind, ind)
            }
            CodegenNode::StateContext { state_name } => format!("s._state_context[\"{}\"]", state_name),

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}s.{}()", ctx.get_indent(), event)
                } else {
                    format!("{}s.{}({})", ctx.get_indent(), event, args_str.join(", "))
                }
            }

            CodegenNode::NativeBlock { code, .. } => {
                let indent = ctx.get_indent();
                code.lines()
                    .map(|line| {
                        if line.trim().is_empty() {
                            String::new()
                        } else {
                            format!("{}{}", indent, line)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            CodegenNode::SplicePoint { id } => format!("// SPLICE_POINT: {}", id),
        }
    }

    fn runtime_imports(&self) -> Vec<String> {
        // Go manages imports per-file; runtime types are emitted inline
        vec![]
    }

    fn class_syntax(&self) -> ClassSyntax { ClassSyntax::go() }
    fn target_language(&self) -> TargetLanguage { TargetLanguage::Go }
    fn null_keyword(&self) -> &'static str { "nil" }
}

impl GoBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params.iter().map(|p| {
            let type_ann = self.map_type(p.type_annotation.as_ref().unwrap_or(&"any".to_string()));
            format!("{} {}", p.name, type_ann)
        }).collect::<Vec<_>>().join(", ")
    }

    fn map_type(&self, t: &str) -> String {
        match t {
            "Any" | "object" | "Object" => "any".to_string(),
            "string" | "String" | "str" => "string".to_string(),
            "int" | "i32" | "i64" | "number" => "int".to_string(),
            "float" | "f64" | "f32" => "float64".to_string(),
            "bool" | "boolean" => "bool".to_string(),
            "void" | "None" => String::new(), // Go uses no return type for void
            other => other.to_string(),
        }
    }
}

/// Capitalize the first letter of a string (for Go export visibility)
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
