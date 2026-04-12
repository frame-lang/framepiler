#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSegmentKind {
    Transition,
    TransitionForward, // -> => $State - transition then forward event
    Forward,
    StackPush,
    StackPop,
    StateVar,          // $.varName (read access)
    StateVarAssign,    // $.varName = expr (assignment)
    // System context syntax (@@)
    ContextReturn,         // @@:return - return value slot (assignment or read)
    ContextEvent,          // @@:event - interface event name
    ContextData,           // @@:data[key] - call-scoped data (read)
    ContextDataAssign,     // @@:data[key] = expr - call-scoped data (assignment)
    ContextParams,         // @@:params[key] - explicit parameter access
    ContextReturnExpr,     // @@:(expr) - set context return value (concise form)
    TaggedInstantiation,   // @@SystemName() - validated system instantiation
    ReturnCall,            // @@:return(expr) - set return value AND exit handler
    ContextSelfCall,       // @@:self.method(args) - reentrant interface call
    ContextSelf,           // @@:self - bare system instance reference
    ContextSystemState,    // @@:system.state - current state name (read-only)
    ReturnStatement,       // return <expr>? - native return keyword in handler body
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionSpan { pub start: usize, pub end: usize }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Region {
    NativeText { span: RegionSpan },
    FrameSegment { span: RegionSpan, kind: FrameSegmentKind, indent: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult { pub close_byte: usize, pub regions: Vec<Region> }

#[derive(Debug)]
pub enum ScanErrorKind { UnterminatedProtected, Internal }

#[derive(Debug)]
pub struct ScanError { pub kind: ScanErrorKind, pub message: String }

impl ScanError { pub fn internal(msg: &str) -> Self { Self{ kind: ScanErrorKind::Internal, message: msg.to_string() } } }

pub trait NativeRegionScanner {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError>;
}

// Unified scanner architecture - Frame statement detection is shared,
// only language-specific syntax skipping differs
pub mod unified;

pub mod python;
pub mod typescript;
pub mod csharp;
pub mod c;
pub mod cpp;
pub mod java;
pub mod rust;
pub mod go;
pub mod javascript;
pub mod php;
pub mod kotlin;
pub mod swift;
pub mod ruby;
pub mod erlang;
pub mod lua;
pub mod dart;
pub mod gdscript;

