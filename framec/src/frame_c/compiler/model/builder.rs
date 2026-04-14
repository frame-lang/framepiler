//! Builds a SystemModel from SystemAst — semantic projection for visualization.

use crate::frame_c::compiler::frame_ast::*;
use serde::Serialize;

/// Root model output — may contain multiple systems from one file.
#[derive(Debug, Clone, Serialize)]
pub struct ModelOutput {
    pub format: String,
    pub version: String,
    pub systems: Vec<SystemModel>,
}

/// Complete semantic model of one Frame system.
#[derive(Debug, Clone, Serialize)]
pub struct SystemModel {
    pub name: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persist: Option<bool>,
    pub interface: Vec<InterfaceMethodModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine: Option<MachineModel>,
    pub actions: Vec<ActionModel>,
    pub operations: Vec<OperationModel>,
    pub domain: Vec<DomainVarModel>,
}

/// Machine (state machine portion).
#[derive(Debug, Clone, Serialize)]
pub struct MachineModel {
    pub states: Vec<StateModel>,
    pub transitions: Vec<TransitionModel>,
}

/// A state in the machine.
#[derive(Debug, Clone, Serialize)]
pub struct StateModel {
    pub name: String,
    #[serde(rename = "isStart")]
    pub is_start: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub children: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "defaultForward")]
    pub default_forward: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", rename = "stateVars")]
    pub state_vars: Vec<StateVarModel>,
    #[serde(skip_serializing_if = "Vec::is_empty", rename = "stateParams")]
    pub state_params: Vec<ParamModel>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "enterHandler")]
    pub enter_handler: Option<HandlerModel>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "exitHandler")]
    pub exit_handler: Option<HandlerModel>,
    pub handlers: Vec<HandlerModel>,
}

/// A handler (event handler, enter, exit).
#[derive(Debug, Clone, Serialize)]
pub struct HandlerModel {
    pub event: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamModel>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "returnType")]
    pub return_type: Option<String>,
    /// Handler body as Frame source lines (not transpiled code).
    pub body: Vec<String>,
    /// Transitions originating from this handler.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<TransitionModel>,
}

/// A transition edge.
#[derive(Debug, Clone, Serialize)]
pub struct TransitionModel {
    pub from: String,
    pub to: String,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub kind: String, // "transition", "changeState", "forward", "stackPush", "stackPop"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
}

/// Interface method.
#[derive(Debug, Clone, Serialize)]
pub struct InterfaceMethodModel {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamModel>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "returnType")]
    pub return_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "returnInit")]
    pub return_init: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isAsync")]
    pub is_async: Option<bool>,
}

/// Action method.
#[derive(Debug, Clone, Serialize)]
pub struct ActionModel {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamModel>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isAsync")]
    pub is_async: Option<bool>,
}

/// Operation method.
#[derive(Debug, Clone, Serialize)]
pub struct OperationModel {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamModel>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "returnType")]
    pub return_type: Option<String>,
    #[serde(rename = "isStatic")]
    pub is_static: bool,
    #[serde(skip_serializing_if = "Option::is_none", rename = "isAsync")]
    pub is_async: Option<bool>,
}

/// Domain variable.
#[derive(Debug, Clone, Serialize)]
pub struct DomainVarModel {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub var_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// State variable.
#[derive(Debug, Clone, Serialize)]
pub struct StateVarModel {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub var_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init: Option<String>,
}

/// Parameter (reused across interface, actions, operations, handlers).
#[derive(Debug, Clone, Serialize)]
pub struct ParamModel {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub param_type: Option<String>,
}

// ============================================================================
// Builder
// ============================================================================

/// Build a semantic model from a parsed Frame system.
pub fn build_system_model(system: &SystemAst, target: &str, source: &[u8]) -> SystemModel {
    // Interface
    let interface: Vec<InterfaceMethodModel> = system
        .interface
        .iter()
        .map(|m| InterfaceMethodModel {
            name: m.name.clone(),
            params: m
                .params
                .iter()
                .map(|p| ParamModel {
                    name: p.name.clone(),
                    param_type: format_type_opt(&p.param_type),
                })
                .collect(),
            return_type: m.return_type.as_ref().and_then(|t| format_type_opt(t)),
            return_init: m.return_init.clone(),
            is_async: if m.is_async { Some(true) } else { None },
        })
        .collect();

    // Machine
    let machine = system.machine.as_ref().map(|m| {
        let mut all_transitions = Vec::new();
        let states: Vec<StateModel> = m
            .states
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let children: Vec<String> = m
                    .states
                    .iter()
                    .filter(|other| other.parent.as_deref() == Some(&s.name))
                    .map(|other| other.name.clone())
                    .collect();

                let mut handler_models = Vec::new();
                for handler in &s.handlers {
                    let mut transitions = Vec::new();
                    extract_transitions(
                        &handler.body.statements,
                        &s.name,
                        &handler.event,
                        None,
                        &mut transitions,
                    );
                    all_transitions.extend(transitions.clone());

                    handler_models.push(HandlerModel {
                        event: handler.event.clone(),
                        params: handler
                            .params
                            .iter()
                            .map(|p| ParamModel {
                                name: p.name.clone(),
                                param_type: format_type_opt(&p.param_type),
                            })
                            .collect(),
                        return_type: handler
                            .return_type
                            .as_ref()
                            .and_then(|t| format_type_opt(t)),
                        body: extract_body_lines(source, &handler.body.span),
                        transitions,
                    });
                }

                let enter_handler = s.enter.as_ref().map(|e| {
                    let mut transitions = Vec::new();
                    extract_transitions(&e.body.statements, &s.name, "$>", None, &mut transitions);
                    all_transitions.extend(transitions.clone());
                    HandlerModel {
                        event: "$>".to_string(),
                        params: e
                            .params
                            .iter()
                            .map(|p| ParamModel {
                                name: p.name.clone(),
                                param_type: format_type_opt(&p.param_type),
                            })
                            .collect(),
                        return_type: None,
                        body: extract_body_lines(source, &e.body.span),
                        transitions,
                    }
                });

                let exit_handler = s.exit.as_ref().map(|e| {
                    let mut transitions = Vec::new();
                    extract_transitions(&e.body.statements, &s.name, "<$", None, &mut transitions);
                    all_transitions.extend(transitions.clone());
                    HandlerModel {
                        event: "<$".to_string(),
                        params: e
                            .params
                            .iter()
                            .map(|p| ParamModel {
                                name: p.name.clone(),
                                param_type: format_type_opt(&p.param_type),
                            })
                            .collect(),
                        return_type: None,
                        body: extract_body_lines(source, &e.body.span),
                        transitions,
                    }
                });

                StateModel {
                    name: s.name.clone(),
                    is_start: i == 0,
                    parent: s.parent.clone(),
                    children,
                    default_forward: if s.default_forward { Some(true) } else { None },
                    state_vars: s
                        .state_vars
                        .iter()
                        .map(|sv| StateVarModel {
                            name: sv.name.clone(),
                            var_type: format_type_opt(&sv.var_type),
                            init: sv.init.as_ref().map(|e| format_expr(e)),
                        })
                        .collect(),
                    state_params: s
                        .params
                        .iter()
                        .map(|p| ParamModel {
                            name: p.name.clone(),
                            param_type: format_type_opt(&p.param_type),
                        })
                        .collect(),
                    enter_handler,
                    exit_handler,
                    handlers: handler_models,
                }
            })
            .collect();

        MachineModel {
            states,
            transitions: all_transitions,
        }
    });

    // Actions
    let actions: Vec<ActionModel> = system
        .actions
        .iter()
        .map(|a| ActionModel {
            name: a.name.clone(),
            params: a
                .params
                .iter()
                .map(|p| ParamModel {
                    name: p.name.clone(),
                    param_type: format_type_opt(&p.param_type),
                })
                .collect(),
            is_async: if a.is_async { Some(true) } else { None },
        })
        .collect();

    // Operations
    let operations: Vec<OperationModel> = system
        .operations
        .iter()
        .map(|o| OperationModel {
            name: o.name.clone(),
            params: o
                .params
                .iter()
                .map(|p| ParamModel {
                    name: p.name.clone(),
                    param_type: format_type_opt(&p.param_type),
                })
                .collect(),
            return_type: format_type_opt(&o.return_type),
            is_static: o.is_static,
            is_async: if o.is_async { Some(true) } else { None },
        })
        .collect();

    // Domain
    let domain: Vec<DomainVarModel> = system
        .domain
        .iter()
        .map(|d| DomainVarModel {
            name: d.name.clone(),
            var_type: format_type_opt(&d.var_type),
            default: d.initializer_text.clone(),
        })
        .collect();

    SystemModel {
        name: system.name.clone(),
        target: target.to_string(),
        persist: if system.persist_attr.is_some() {
            Some(true)
        } else {
            None
        },
        interface,
        machine,
        actions,
        operations,
        domain,
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn format_type_opt(t: &Type) -> Option<String> {
    match t {
        Type::Unknown => None,
        Type::Custom(s) => Some(s.clone()),
    }
}

fn format_expr(expr: &Expression) -> String {
    match expr {
        Expression::Literal(lit) => match lit {
            Literal::Int(n) => n.to_string(),
            Literal::Float(f) => f.to_string(),
            Literal::String(s) => format!("\"{}\"", s),
            Literal::Bool(b) => b.to_string(),
            Literal::Null => "null".to_string(),
        },
        Expression::Var(name) => name.clone(),
        Expression::Binary { op, left, right } => {
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
            format!("{} {} {}", format_expr(left), op_str, format_expr(right))
        }
        Expression::Unary { op, expr: operand } => {
            let op_str = match op {
                UnaryOp::Neg => "-",
                UnaryOp::Not => "!",
                UnaryOp::BitNot => "~",
            };
            format!("{}{}", op_str, format_expr(operand))
        }
        Expression::Call { func, args } => {
            let args_str: Vec<String> = args.iter().map(|a| format_expr(a)).collect();
            format!("{}({})", func, args_str.join(", "))
        }
        Expression::Member { object, field } => {
            format!("{}.{}", format_expr(object), field)
        }
        _ => "...".to_string(),
    }
}

/// Extract handler body as Frame source lines from the raw source bytes.
fn extract_body_lines(source: &[u8], span: &Span) -> Vec<String> {
    if span.start >= span.end || span.end > source.len() {
        return vec![];
    }
    let body_text = String::from_utf8_lossy(&source[span.start..span.end]);
    // Strip outer braces and normalize
    let trimmed = body_text.trim();
    let inner = if trimmed.starts_with('{') && trimmed.ends_with('}') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    inner
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Extract transitions from statements recursively.
fn extract_transitions(
    stmts: &[Statement],
    state_name: &str,
    event: &str,
    guard: Option<&str>,
    out: &mut Vec<TransitionModel>,
) {
    for stmt in stmts {
        match stmt {
            Statement::Transition(t) => {
                out.push(TransitionModel {
                    from: state_name.to_string(),
                    to: t.target.clone(),
                    event: event.to_string(),
                    label: t.label.clone(),
                    kind: "transition".to_string(),
                    guard: guard.map(|s| s.to_string()),
                });
            }
            Statement::TransitionForward(tf) => {
                out.push(TransitionModel {
                    from: state_name.to_string(),
                    to: tf.target.clone(),
                    event: event.to_string(),
                    label: None,
                    kind: "transitionForward".to_string(),
                    guard: guard.map(|s| s.to_string()),
                });
            }
            Statement::Forward(f) => {
                out.push(TransitionModel {
                    from: state_name.to_string(),
                    to: f.event.clone(),
                    event: event.to_string(),
                    label: None,
                    kind: "forward".to_string(),
                    guard: guard.map(|s| s.to_string()),
                });
            }
            Statement::StackPush(_) => {
                out.push(TransitionModel {
                    from: state_name.to_string(),
                    to: "$$[+]".to_string(),
                    event: event.to_string(),
                    label: None,
                    kind: "stackPush".to_string(),
                    guard: guard.map(|s| s.to_string()),
                });
            }
            Statement::StackPop(_) => {
                out.push(TransitionModel {
                    from: state_name.to_string(),
                    to: "$$[-]".to_string(),
                    event: event.to_string(),
                    label: None,
                    kind: "stackPop".to_string(),
                    guard: guard.map(|s| s.to_string()),
                });
            }
            Statement::If(if_ast) => {
                let guard_text = format_expr(&if_ast.condition);
                extract_transitions(
                    &[*if_ast.then_branch.clone()],
                    state_name,
                    event,
                    Some(&guard_text),
                    out,
                );
                if let Some(ref else_branch) = if_ast.else_branch {
                    let else_guard = format!("!{}", guard_text);
                    extract_transitions(
                        &[*else_branch.clone()],
                        state_name,
                        event,
                        Some(&else_guard),
                        out,
                    );
                }
            }
            _ => {}
        }
    }
}
