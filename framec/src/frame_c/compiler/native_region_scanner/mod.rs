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
    /// `-> $State`, `-> pop$`, `(exit) -> (enter) $State(state_args)`
    Transition {
        target_state: String,
        exit_args: Option<String>,
        enter_args: Option<String>,
        state_args: Option<String>,
        label: Option<String>,
        is_pop: bool,
    },
    /// `$.varName` (read) or `$.varName = expr` (assign)
    StateVar {
        name: String,
    },
    /// `@@:params.key`
    ContextParams {
        key: String,
    },
    /// `@@:data.key` (read) or `@@:data.key = expr` (assign)
    ContextData {
        key: String,
        assign_expr: Option<String>,
    },
    /// `@@:self.method(args)`
    SelfCall {
        method: String,
        args: String,
    },
    /// `@@SystemName(args)`
    TaggedInstantiation {
        system_name: String,
        args: String,
    },
    /// `@@:(expr)` — may contain nested Frame segments
    ReturnExpr {
        expr: String,
    },
    /// `@@:return(expr)` — set return + exit
    ReturnCall {
        expr: String,
    },
    /// `@@:return = expr` or `@@:return` (bare read)
    ContextReturn {
        assign_expr: Option<String>,
    },
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
