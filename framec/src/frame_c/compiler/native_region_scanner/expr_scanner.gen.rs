
// ExprScanner — PDA (pushdown automaton) for scanning assignment RHS expressions.
//
// Scans from `pos` (after the `=`) to a terminator (`;` or `\n`) at depth 0,
// respecting nested `()[]{}` and string literals with escape handling.
//
// This replaces 3 duplicated inline expression scanners in unified.rs.

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum ExprScannerFsmFrameEvent {
    DoScan {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum ExprScannerFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl ExprScannerFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            ExprScannerFsmFrameEvent::DoScan { .. } => "do_scan",
            ExprScannerFsmFrameEvent::FrameEnter { .. } => "$>",
            ExprScannerFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum ExprScannerFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct ExprScannerFsmFrameContext {
    event: std::rc::Rc<ExprScannerFsmFrameEvent>,
    _return: Option<ExprScannerFsmFrameReturn>,
    _data: std::collections::HashMap<String, ExprScannerFsmFrameValue>,
    _transitioned: bool,
}

impl ExprScannerFsmFrameContext {
    fn new(event: std::rc::Rc<ExprScannerFsmFrameEvent>, default_return: Option<ExprScannerFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum ExprScannerFsmStateContext {
    Init,
    Scanning,
    Empty,
}

impl Default for ExprScannerFsmStateContext {
    fn default() -> Self {
        ExprScannerFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct ExprScannerFsmCompartment {
    state: String,
    state_context: ExprScannerFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<ExprScannerFsmFrameEvent>,
    parent_compartment: Option<Box<ExprScannerFsmCompartment>>,
}

impl ExprScannerFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => ExprScannerFsmStateContext::Init,
            "Scanning" => ExprScannerFsmStateContext::Scanning,
            _ => ExprScannerFsmStateContext::Empty,
        };
        Self {
            state: state.to_string(),
            state_context,
            enter_args: Vec::new(),
            exit_args: Vec::new(),
            forward_event: None,
            parent_compartment: None,
        }
    }
}

#[allow(dead_code)]
pub struct ExprScannerFsm {
    _state_stack: Vec<ExprScannerFsmCompartment>,
    __compartment: ExprScannerFsmCompartment,
    __next_compartment: Option<ExprScannerFsmCompartment>,
    _context_stack: Vec<ExprScannerFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_end: usize,
}

#[allow(non_snake_case)]
impl ExprScannerFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_end: 0,
            __compartment: ExprScannerFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(ExprScannerFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = ExprScannerFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        c._context_stack.push(__ctx);
        c.__kernel(&__e);
        c._context_stack.pop();
        c
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "Scanning" => &["Scanning"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> ExprScannerFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<ExprScannerFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = ExprScannerFsmCompartment::new(name);
            new_comp.enter_args = enter_args.clone();
            if let Some(parent) = comp.take() {
                new_comp.parent_compartment = Some(Box::new(parent));
            }
            comp = Some(new_comp);
        }
        comp.expect("chain must contain at least the leaf state")
    }

    fn __prepareExit(&mut self, exit_args: Vec<String>) {
        self.__compartment.exit_args = exit_args.clone();
        let mut cursor = self.__compartment.parent_compartment.as_deref_mut();
        while let Some(c) = cursor {
            c.exit_args = exit_args.clone();
            cursor = c.parent_compartment.as_deref_mut();
        }
    }

    fn __kernel(&mut self, __e: &std::rc::Rc<ExprScannerFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(ExprScannerFsmFrameEvent::FrameExit { args: exit_args });
            self.__router(&exit_event);
            // Switch to the new compartment.
            self.__compartment = next_compartment;
            // Three-branch forward-event handling (RFC-0025 Track B.1: forward
            // event is matched on enum variant; $> recognition is now a
            // structural match, not a string compare).
            match self.__compartment.forward_event.take() {
                None => {
                    // No forwarded event — synthesize a fresh $>.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(ExprScannerFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, ExprScannerFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(ExprScannerFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
            }
            for ctx in self._context_stack.iter_mut() {
                ctx._transitioned = true;
            }
        }
    }

    fn __router(&mut self, __e: &std::rc::Rc<ExprScannerFsmFrameEvent>) {
        let __ev: &ExprScannerFsmFrameEvent = __e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "Scanning" => self._state_Scanning(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: ExprScannerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_scan(&mut self) {
        let __e = std::rc::Rc::new(ExprScannerFsmFrameEvent::DoScan {});
        let mut __ctx = ExprScannerFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &ExprScannerFsmFrameEvent) {
        match __e {
            ExprScannerFsmFrameEvent::DoScan { .. } => { self._s_Init_hdl_user_do_scan(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &ExprScannerFsmFrameEvent) {
        match __e {
            ExprScannerFsmFrameEvent::FrameEnter { .. } => { self._s_Scanning_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_scan(&mut self, __e: &ExprScannerFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Scanning_hdl_frame_enter(&mut self, __e: &ExprScannerFsmFrameEvent) {
        let mut i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        let mut depth: i32 = 0;
        let mut in_string: Option<u8> = None;
        
        while i < end {
            let b = bytes[i];
        
            // Handle string literals
            if let Some(q) = in_string {
                if b == b'\\' && i + 1 < end {
                    i += 2;
                    continue;
                }
                if b == q {
                    in_string = None;
                }
                i += 1;
                continue;
            }
        
            // Enter string literal
            if b == b'"' || b == b'\'' {
                in_string = Some(b);
                i += 1;
                continue;
            }
        
            // Track nesting depth (PDA stack via counter)
            match b {
                b'(' | b'[' | b'{' => { depth += 1; }
                b')' | b']' | b'}' => { depth = (depth - 1).max(0); }
                b';' if depth == 0 => {
                    i += 1; // Include the semicolon
                    break;
                }
                b'\n' if depth == 0 => {
                    break; // Don't include the newline
                }
                _ => {}
            }
            i += 1;
        }
        
        self.result_end = i;
    }
}
