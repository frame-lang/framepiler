
// Dogfooded body closer — Python language brace matcher.
// Python-specific: triple quotes (''' and """), # line comments, string prefixes (f/r/b/F/R/B).
//
// State machine flow:
//   $Init.scan() → $Scanning.$>() ↔ $InString/$InTripleString/$InLineComment

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum PythonBodyCloserFsmFrameEvent {
    Scan {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum PythonBodyCloserFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl PythonBodyCloserFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            PythonBodyCloserFsmFrameEvent::Scan { .. } => "scan",
            PythonBodyCloserFsmFrameEvent::FrameEnter { .. } => "$>",
            PythonBodyCloserFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum PythonBodyCloserFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct PythonBodyCloserFsmFrameContext {
    event: std::rc::Rc<PythonBodyCloserFsmFrameEvent>,
    _return: Option<PythonBodyCloserFsmFrameReturn>,
    _data: std::collections::HashMap<String, PythonBodyCloserFsmFrameValue>,
    _transitioned: bool,
}

impl PythonBodyCloserFsmFrameContext {
    fn new(event: std::rc::Rc<PythonBodyCloserFsmFrameEvent>, default_return: Option<PythonBodyCloserFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum PythonBodyCloserFsmStateContext {
    Init,
    Scanning,
    InString,
    InTripleString,
    InLineComment,
    Empty,
}

impl Default for PythonBodyCloserFsmStateContext {
    fn default() -> Self {
        PythonBodyCloserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct PythonBodyCloserFsmCompartment {
    state: String,
    state_context: PythonBodyCloserFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<PythonBodyCloserFsmFrameEvent>,
    parent_compartment: Option<Box<PythonBodyCloserFsmCompartment>>,
}

impl PythonBodyCloserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => PythonBodyCloserFsmStateContext::Init,
            "Scanning" => PythonBodyCloserFsmStateContext::Scanning,
            "InString" => PythonBodyCloserFsmStateContext::InString,
            "InTripleString" => PythonBodyCloserFsmStateContext::InTripleString,
            "InLineComment" => PythonBodyCloserFsmStateContext::InLineComment,
            _ => PythonBodyCloserFsmStateContext::Empty,
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
pub struct PythonBodyCloserFsm {
    _state_stack: Vec<PythonBodyCloserFsmCompartment>,
    __compartment: PythonBodyCloserFsmCompartment,
    __next_compartment: Option<PythonBodyCloserFsmCompartment>,
    _context_stack: Vec<PythonBodyCloserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub depth: i32,
    pub result_pos: usize,
    pub error_kind: usize,
    pub error_msg: String,
    pub quote_char: u8,
}

#[allow(non_snake_case)]
impl PythonBodyCloserFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            depth: 1,
            result_pos: 0,
            error_kind: 0,
            error_msg: String::new(),
            quote_char: 0,
            __compartment: PythonBodyCloserFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(PythonBodyCloserFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = PythonBodyCloserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        c._context_stack.push(__ctx);
        c.__kernel(&__e);
        c._context_stack.pop();
        c
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "Scanning" => &["Scanning"],
            "InString" => &["InString"],
            "InTripleString" => &["InTripleString"],
            "InLineComment" => &["InLineComment"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> PythonBodyCloserFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<PythonBodyCloserFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = PythonBodyCloserFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<PythonBodyCloserFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().unwrap();
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(PythonBodyCloserFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(PythonBodyCloserFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, PythonBodyCloserFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(PythonBodyCloserFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<PythonBodyCloserFsmFrameEvent>) {
        let __ev: &PythonBodyCloserFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "Scanning" => self._state_Scanning(__ev),
            "InString" => self._state_InString(__ev),
            "InTripleString" => self._state_InTripleString(__ev),
            "InLineComment" => self._state_InLineComment(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: PythonBodyCloserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn scan(&mut self) {
        let __e = std::rc::Rc::new(PythonBodyCloserFsmFrameEvent::Scan {});
        let mut __ctx = PythonBodyCloserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        match __e {
            PythonBodyCloserFsmFrameEvent::Scan { .. } => { self._s_Init_hdl_user_scan(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        match __e {
            PythonBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_Scanning_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InString(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        match __e {
            PythonBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InTripleString(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        match __e {
            PythonBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InTripleString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InLineComment(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        match __e {
            PythonBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InLineComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_scan(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Scanning_hdl_frame_enter(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            let b = self.bytes[self.pos];
            if b == b'\n' {
                self.pos += 1;
            } else if b == b'#' {
                self.pos += 1;
                let mut __compartment = self.__prepareEnter("InLineComment", vec![]);
                self.__transition(__compartment);
                return;
            } else if b == b'\'' || b == b'"' {
                let q = b;
                if self.pos + 2 < n && self.bytes[self.pos + 1] == q && self.bytes[self.pos + 2] == q {
                    self.quote_char = q;
                    self.pos += 3;
                    let mut __compartment = self.__prepareEnter("InTripleString", vec![]);
                    self.__transition(__compartment);
                    return;
                } else {
                    self.quote_char = q;
                    self.pos += 1;
                    let mut __compartment = self.__prepareEnter("InString", vec![]);
                    self.__transition(__compartment);
                    return;
                }
            } else if b == b'f' || b == b'F' || b == b'r' || b == b'R' || b == b'b' || b == b'B' {
                // String prefixes like f"..", r'..', etc.
                if self.pos + 1 < n && (self.bytes[self.pos + 1] == b'\'' || self.bytes[self.pos + 1] == b'"') {
                    let q = self.bytes[self.pos + 1];
                    if self.pos + 3 < n && self.bytes[self.pos + 2] == q && self.bytes[self.pos + 3] == q {
                        self.quote_char = q;
                        self.pos += 4;
                        let mut __compartment = self.__prepareEnter("InTripleString", vec![]);
                        self.__transition(__compartment);
                        return;
                    } else {
                        self.quote_char = q;
                        self.pos += 2;
                        let mut __compartment = self.__prepareEnter("InString", vec![]);
                        self.__transition(__compartment);
                        return;
                    }
                } else {
                    self.pos += 1;
                }
            } else if b == b'{' {
                self.depth += 1;
                self.pos += 1;
            } else if b == b'}' {
                self.depth -= 1;
                self.pos += 1;
                if self.depth == 0 {
                    self.result_pos = self.pos - 1;
                    self.error_kind = 0;
                    return
                }
            } else {
                self.pos += 1;
            }
        }
        self.error_kind = 3;
        self.error_msg = "body not closed".to_string();
    }

    fn _s_InString_hdl_frame_enter(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            if self.bytes[self.pos] == b'\\' {
                self.pos += 2;
                continue;
            }
            if self.bytes[self.pos] == self.quote_char {
                self.pos += 1;
                let mut __compartment = self.__prepareEnter("Scanning", vec![]);
                self.__transition(__compartment);
                return;
            }
            self.pos += 1;
        }
        self.error_kind = 1;
        self.error_msg = "unterminated string".to_string();
    }

    fn _s_InTripleString_hdl_frame_enter(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        let q = self.quote_char;
        while self.pos < n {
            if self.bytes[self.pos] == q && self.pos + 2 < n && self.bytes[self.pos + 1] == q && self.bytes[self.pos + 2] == q {
                self.pos += 3;
                let mut __compartment = self.__prepareEnter("Scanning", vec![]);
                self.__transition(__compartment);
                return;
            }
            self.pos += 1;
        }
        self.error_kind = 1;
        self.error_msg = "unterminated string".to_string();
    }

    fn _s_InLineComment_hdl_frame_enter(&mut self, __e: &PythonBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }
}
