#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSegmentKind {
    Transition,
    TransitionForward, // -> => $State - transition then forward event
    Forward,
    StackPush,
    StackPop,
    StateVar,       // $.varName (read access)
    StateVarAssign, // $.varName = expr (assignment)
    // System context syntax (@@)
    ContextReturn,       // @@:return - return value slot (assignment or read)
    ContextEvent,        // @@:event - interface event name
    ContextData,         // @@:data[key] - call-scoped data (read)
    ContextDataAssign,   // @@:data[key] = expr - call-scoped data (assignment)
    ContextParams,       // @@:params[key] - explicit parameter access
    ContextReturnExpr,   // @@:(expr) - set context return value (concise form)
    TaggedInstantiation, // @@SystemName() - validated system instantiation
    ReturnCall,          // @@:return(expr) - set return value AND exit handler
    ContextSelfCall,     // @@:self.method(args) - reentrant interface call
    ContextSelf,         // @@:self - bare system instance reference
    ContextSystemState,  // @@:system.state - current state name (read-only)
    ContextSystemBare,   // @@:system without recognized member - error E604
    ReturnStatement,     // return <expr>? - native return keyword in handler body
}

/// Structured content parsed from a Frame segment during scanning.
/// Eliminates the need for downstream stages to re-parse raw segment text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentMetadata {
    /// `-> $State`, `-> pop$`, `-> => $State`, `(exit) -> (enter) $State(state_args)`
    Transition {
        target_state: String,
        exit_args: Option<String>,
        enter_args: Option<String>,
        state_args: Option<String>,
        label: Option<String>,
        is_pop: bool,
        is_forward: bool,
    },
    /// `$.varName` (read) or `$.varName = expr` (assign)
    StateVar { name: String },
    /// `@@:params.key`
    ContextParams { key: String },
    /// `@@:data.key` (read) or `@@:data.key = expr` (assign)
    ContextData {
        key: String,
        assign_expr: Option<String>,
    },
    /// `@@:self.method(args)`
    SelfCall { method: String, args: String },
    /// `@@SystemName(args)`
    TaggedInstantiation { system_name: String, args: String },
    /// `@@:(expr)` — may contain nested Frame segments
    ReturnExpr { expr: String },
    /// `@@:return(expr)` — set return + exit
    ReturnCall { expr: String },
    /// `@@:return = expr` or `@@:return` (bare read)
    ContextReturn { assign_expr: Option<String> },
    /// `push$` optionally followed by `-> $State` on the same line
    StackPush { transition_target: Option<String> },
    /// Segments with no additional parsed content
    None,
}

impl Default for SegmentMetadata {
    fn default() -> Self {
        SegmentMetadata::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Region {
    NativeText {
        span: RegionSpan,
    },
    FrameSegment {
        span: RegionSpan,
        kind: FrameSegmentKind,
        indent: usize,
        metadata: SegmentMetadata,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub close_byte: usize,
    pub regions: Vec<Region>,
}

#[derive(Debug)]
pub enum ScanErrorKind {
    UnterminatedProtected,
    Internal,
}

#[derive(Debug)]
pub struct ScanError {
    pub kind: ScanErrorKind,
    pub message: String,
}

impl ScanError {
    pub fn internal(msg: &str) -> Self {
        Self {
            kind: ScanErrorKind::Internal,
            message: msg.to_string(),
        }
    }
}

pub trait NativeRegionScanner {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError>;
}

// Unified scanner architecture - Frame statement detection is shared,
// only language-specific syntax skipping differs
pub mod unified;

pub mod c;
pub mod cpp;
pub mod csharp;
pub mod dart;
pub mod erlang;
pub mod gdscript;
pub mod go;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod lua;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod swift;
pub mod typescript;

use crate::frame_c::visitors::TargetLanguage;
use unified::SyntaxSkipper;

/// Convert scanner regions to AST Statement nodes.
///
/// This is the bridge between the scanner (which produces `Vec<Region>` with byte spans
/// and `SegmentMetadata`) and the AST (which uses `Vec<Statement>` with typed fields).
/// The scanner does all the hard parsing work — this is a thin mechanical mapping.
pub fn regions_to_statements(
    bytes: &[u8],
    regions: &[Region],
) -> Vec<crate::frame_c::compiler::frame_ast::Statement> {
    use crate::frame_c::compiler::frame_ast::*;

    let mut stmts = Vec::new();
    for region in regions {
        match region {
            Region::NativeText { span } => {
                let text = String::from_utf8_lossy(&bytes[span.start..span.end]).to_string();
                stmts.push(Statement::NativeCode(text));
            }
            Region::FrameSegment {
                span,
                kind,
                indent,
                metadata,
            } => {
                let seg_span = Span::new(span.start, span.end);
                let raw = || String::from_utf8_lossy(&bytes[span.start..span.end]).to_string();

                match kind {
                    FrameSegmentKind::Transition => {
                        if let SegmentMetadata::Transition {
                            target_state,
                            exit_args,
                            enter_args,
                            state_args,
                            label,
                            is_pop,
                            is_forward,
                        } = metadata
                        {
                            stmts.push(Statement::Transition(TransitionAst {
                                target: target_state.clone(),
                                args: vec![],
                                label: label.clone(),
                                span: seg_span,
                                indent: *indent,
                                exit_args: exit_args.clone(),
                                enter_args: enter_args.clone(),
                                state_args: state_args.clone(),
                                is_pop: *is_pop,
                                is_forward: *is_forward,
                            }));
                        } else {
                            // Fallback: store raw text
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::TransitionForward => {
                        if let SegmentMetadata::Transition { target_state, .. } = metadata {
                            stmts.push(Statement::TransitionForward(TransitionForwardAst {
                                target: target_state.clone(),
                                span: seg_span,
                                indent: *indent,
                            }));
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::Forward => {
                        stmts.push(Statement::Forward(ForwardAst {
                            event: String::new(),
                            args: vec![],
                            span: seg_span,
                            indent: *indent,
                        }));
                    }
                    FrameSegmentKind::StackPush => {
                        stmts.push(Statement::StackPush(StackPushAst {
                            span: seg_span,
                            indent: *indent,
                        }));
                    }
                    FrameSegmentKind::StackPop => {
                        stmts.push(Statement::StackPop(StackPopAst {
                            span: seg_span,
                            indent: *indent,
                        }));
                    }
                    FrameSegmentKind::StateVar => {
                        if let SegmentMetadata::StateVar { name } = metadata {
                            stmts.push(Statement::StateVarRead {
                                name: name.clone(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::StateVarAssign => {
                        if let SegmentMetadata::StateVar { name } = metadata {
                            // Extract assignment expression from raw text after "$.name = "
                            let text = raw();
                            let expr = text
                                .find('=')
                                .map(|i| text[i + 1..].trim().to_string())
                                .unwrap_or_default();
                            stmts.push(Statement::StateVarAssign {
                                name: name.clone(),
                                expr,
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::ContextReturn => {
                        if let SegmentMetadata::ContextReturn { assign_expr } = metadata {
                            stmts.push(Statement::ContextReturn {
                                assign_expr: assign_expr.clone(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::ContextReturn {
                                assign_expr: None,
                                span: seg_span,
                            });
                        }
                    }
                    FrameSegmentKind::ContextReturnExpr => {
                        if let SegmentMetadata::ReturnExpr { expr } = metadata {
                            stmts.push(Statement::ContextReturnExpr {
                                expr: expr.clone(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::ReturnCall => {
                        if let SegmentMetadata::ReturnCall { expr } = metadata {
                            stmts.push(Statement::ReturnCall {
                                expr: expr.clone(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::ContextEvent => {
                        stmts.push(Statement::ContextEvent { span: seg_span });
                    }
                    FrameSegmentKind::ContextData => {
                        if let SegmentMetadata::ContextData { key, .. } = metadata {
                            stmts.push(Statement::ContextData {
                                key: key.clone(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::ContextDataAssign => {
                        if let SegmentMetadata::ContextData { key, assign_expr } = metadata {
                            stmts.push(Statement::ContextDataAssign {
                                key: key.clone(),
                                expr: assign_expr.clone().unwrap_or_default(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::ContextParams => {
                        if let SegmentMetadata::ContextParams { key } = metadata {
                            stmts.push(Statement::ContextParams {
                                key: key.clone(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::TaggedInstantiation => {
                        if let SegmentMetadata::TaggedInstantiation { system_name, args } = metadata
                        {
                            stmts.push(Statement::TaggedInstantiation {
                                system_name: system_name.clone(),
                                args: args.clone(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::ContextSelfCall => {
                        if let SegmentMetadata::SelfCall { method, args } = metadata {
                            stmts.push(Statement::ContextSelfCall {
                                method: method.clone(),
                                args: args.clone(),
                                span: seg_span,
                            });
                        } else {
                            stmts.push(Statement::NativeCode(raw()));
                        }
                    }
                    FrameSegmentKind::ContextSelf => {
                        stmts.push(Statement::ContextSelf { span: seg_span });
                    }
                    FrameSegmentKind::ContextSystemState => {
                        stmts.push(Statement::ContextSystemState { span: seg_span });
                    }
                    FrameSegmentKind::ContextSystemBare => {
                        stmts.push(Statement::NativeCode(raw()));
                    }
                    FrameSegmentKind::ReturnStatement => {
                        // Extract return expression if present
                        let text = raw();
                        let value = text
                            .strip_prefix("return")
                            .map(|rest| rest.trim())
                            .filter(|s| !s.is_empty())
                            .map(|s| Expression::NativeExpr(s.to_string()));
                        stmts.push(Statement::Return(ReturnAst {
                            value,
                            span: seg_span,
                        }));
                    }
                }
            }
        }
    }
    stmts
}

/// Create the appropriate SyntaxSkipper for a target language.
/// This is the single source of truth for language → skipper mapping.
pub fn create_skipper(lang: TargetLanguage) -> Box<dyn SyntaxSkipper> {
    match lang {
        TargetLanguage::Python3 => Box::new(python::PythonSkipper),
        TargetLanguage::TypeScript => Box::new(typescript::TypeScriptSkipper),
        TargetLanguage::Rust => Box::new(rust::RustSkipper),
        TargetLanguage::C => Box::new(c::CSkipper),
        TargetLanguage::Cpp => Box::new(cpp::CppSkipper),
        TargetLanguage::Java => Box::new(java::JavaSkipper),
        TargetLanguage::CSharp => Box::new(csharp::CSharpSkipper),
        TargetLanguage::Go => Box::new(go::GoSkipper),
        TargetLanguage::JavaScript => Box::new(javascript::JavaScriptSkipper),
        TargetLanguage::Php => Box::new(php::PhpSkipper),
        TargetLanguage::Kotlin => Box::new(kotlin::KotlinSkipper),
        TargetLanguage::Swift => Box::new(swift::SwiftSkipper),
        TargetLanguage::Ruby => Box::new(ruby::RubySkipper),
        TargetLanguage::Erlang => Box::new(erlang::ErlangSkipper),
        TargetLanguage::Lua => Box::new(lua::LuaSkipper),
        TargetLanguage::Dart => Box::new(dart::DartSkipper),
        TargetLanguage::GDScript => Box::new(gdscript::GDScriptSkipper),
        TargetLanguage::Graphviz => Box::new(python::PythonSkipper),
    }
}
