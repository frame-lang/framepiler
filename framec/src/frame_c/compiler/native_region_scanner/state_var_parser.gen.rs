
// StateVarParser — FSM for parsing $.varName (read) and $.varName = expr (assignment).
//
// Demonstrates hierarchical composition: $ScanExpr creates an ExprScannerFsm
// sub-machine when it detects an assignment.

include!("expr_scanner.gen.rs");

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum StateVarParserFsmFrameEvent {
    DoParse {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum StateVarParserFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl StateVarParserFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            StateVarParserFsmFrameEvent::DoParse { .. } => "do_parse",
            StateVarParserFsmFrameEvent::FrameEnter { .. } => "$>",
            StateVarParserFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum StateVarParserFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct StateVarParserFsmFrameContext {
    event: std::rc::Rc<StateVarParserFsmFrameEvent>,
    _return: Option<StateVarParserFsmFrameReturn>,
    _data: std::collections::HashMap<String, StateVarParserFsmFrameValue>,
    _transitioned: bool,
}

impl StateVarParserFsmFrameContext {
    fn new(event: std::rc::Rc<StateVarParserFsmFrameEvent>, default_return: Option<StateVarParserFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum StateVarParserFsmStateContext {
    Init,
    ScanIdent,
    CheckAssign,
    ScanExpr,
    Done,
    Empty,
}

impl Default for StateVarParserFsmStateContext {
    fn default() -> Self {
        StateVarParserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct StateVarParserFsmCompartment {
    state: String,
    state_context: StateVarParserFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<StateVarParserFsmFrameEvent>,
    parent_compartment: Option<Box<StateVarParserFsmCompartment>>,
}

impl StateVarParserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => StateVarParserFsmStateContext::Init,
            "ScanIdent" => StateVarParserFsmStateContext::ScanIdent,
            "CheckAssign" => StateVarParserFsmStateContext::CheckAssign,
            "ScanExpr" => StateVarParserFsmStateContext::ScanExpr,
            "Done" => StateVarParserFsmStateContext::Done,
            _ => StateVarParserFsmStateContext::Empty,
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
pub struct StateVarParserFsm {
    _state_stack: Vec<StateVarParserFsmCompartment>,
    __compartment: StateVarParserFsmCompartment,
    __next_compartment: Option<StateVarParserFsmCompartment>,
    _context_stack: Vec<StateVarParserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub ident_end: usize,
    pub result_end: usize,
    pub is_assignment: bool,
}

#[allow(non_snake_case)]
impl StateVarParserFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            ident_end: 0,
            result_end: 0,
            is_assignment: false,
            __compartment: StateVarParserFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(StateVarParserFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = StateVarParserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        c._context_stack.push(__ctx);
        c.__kernel(&__e);
        c._context_stack.pop();
        c
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "ScanIdent" => &["ScanIdent"],
            "CheckAssign" => &["CheckAssign"],
            "ScanExpr" => &["ScanExpr"],
            "Done" => &["Done"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> StateVarParserFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<StateVarParserFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = StateVarParserFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<StateVarParserFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(StateVarParserFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(StateVarParserFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, StateVarParserFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(StateVarParserFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<StateVarParserFsmFrameEvent>) {
        let __ev: &StateVarParserFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "ScanIdent" => self._state_ScanIdent(__ev),
            "CheckAssign" => self._state_CheckAssign(__ev),
            "ScanExpr" => self._state_ScanExpr(__ev),
            "Done" => self._state_Done(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: StateVarParserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_parse(&mut self) {
        let __e = std::rc::Rc::new(StateVarParserFsmFrameEvent::DoParse {});
        let mut __ctx = StateVarParserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &StateVarParserFsmFrameEvent) {
        match __e {
            StateVarParserFsmFrameEvent::DoParse { .. } => { self._s_Init_hdl_user_do_parse(__e); }
            _ => {}
        }
    }

    fn _state_ScanIdent(&mut self, __e: &StateVarParserFsmFrameEvent) {
        match __e {
            StateVarParserFsmFrameEvent::FrameEnter { .. } => { self._s_ScanIdent_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_CheckAssign(&mut self, __e: &StateVarParserFsmFrameEvent) {
        match __e {
            StateVarParserFsmFrameEvent::FrameEnter { .. } => { self._s_CheckAssign_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_ScanExpr(&mut self, __e: &StateVarParserFsmFrameEvent) {
        match __e {
            StateVarParserFsmFrameEvent::FrameEnter { .. } => { self._s_ScanExpr_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_Done(&mut self, __e: &StateVarParserFsmFrameEvent) {
        match __e {
            StateVarParserFsmFrameEvent::FrameEnter { .. } => { self._s_Done_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_parse(&mut self, __e: &StateVarParserFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("ScanIdent", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_ScanIdent_hdl_frame_enter(&mut self, __e: &StateVarParserFsmFrameEvent) {
        // Skip "$." prefix and scan identifier
        let mut i = self.pos + 2; // Skip "$."
        let end = self.end;
        let bytes = &self.bytes;
        
        while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        
        self.ident_end = i;
        let mut __compartment = self.__prepareEnter("CheckAssign", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_CheckAssign_hdl_frame_enter(&mut self, __e: &StateVarParserFsmFrameEvent) {
        // Lookahead: skip whitespace, check for = (but not ==)
        let mut j = self.ident_end;
        let end = self.end;
        let bytes = &self.bytes;
        
        while j < end && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        
        if j < end && bytes[j] == b'='
            && (j + 1 >= end || (bytes[j + 1] != b'=' && bytes[j + 1] != b'<' && bytes[j + 1] != b'>'))
        {
            // Assignment detected
            j += 1; // Skip '='
            self.pos = j;
            self.is_assignment = true;
            let mut __compartment = self.__prepareEnter("ScanExpr", vec![]);
            self.__transition(__compartment);
            return;
        } else {
            // Read-only access
            self.result_end = self.ident_end;
            self.is_assignment = false;
            let mut __compartment = self.__prepareEnter("Done", vec![]);
            self.__transition(__compartment);
            return;
        }
    }

    fn _s_ScanExpr_hdl_frame_enter(&mut self, __e: &StateVarParserFsmFrameEvent) {
        // Create ExprScanner sub-machine (state manager pattern)
        let bytes = &self.bytes;
        let mut expr = ExprScannerFsm::new();
        expr.bytes = bytes.to_vec();
        expr.pos = self.pos;
        expr.end = self.end;
        expr.do_scan();
        self.result_end = expr.result_end;
        // expr is destroyed here
        let mut __compartment = self.__prepareEnter("Done", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Done_hdl_frame_enter(&mut self, __e: &StateVarParserFsmFrameEvent) {
        // Terminal state — results in domain vars;
    }
}
