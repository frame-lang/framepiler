
// Dogfooded body closer — C++ language brace matcher.
// Extends C with C++ raw string literals: R"delim(...)delim"
//
// State machine flow:
//   $Init.scan() → $Scanning.$>() ↔ $InString/$InCharLiteral/$InLineComment/$InBlockComment/$InRawString

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum CppBodyCloserFsmFrameEvent {
    Scan {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum CppBodyCloserFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl CppBodyCloserFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            CppBodyCloserFsmFrameEvent::Scan { .. } => "scan",
            CppBodyCloserFsmFrameEvent::FrameEnter { .. } => "$>",
            CppBodyCloserFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum CppBodyCloserFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct CppBodyCloserFsmFrameContext {
    event: std::rc::Rc<CppBodyCloserFsmFrameEvent>,
    _return: Option<CppBodyCloserFsmFrameReturn>,
    _data: std::collections::HashMap<String, CppBodyCloserFsmFrameValue>,
    _transitioned: bool,
}

impl CppBodyCloserFsmFrameContext {
    fn new(event: std::rc::Rc<CppBodyCloserFsmFrameEvent>, default_return: Option<CppBodyCloserFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum CppBodyCloserFsmStateContext {
    Init,
    Scanning,
    InString,
    InCharLiteral,
    InLineComment,
    InBlockComment,
    InRawString,
    Empty,
}

impl Default for CppBodyCloserFsmStateContext {
    fn default() -> Self {
        CppBodyCloserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct CppBodyCloserFsmCompartment {
    state: String,
    state_context: CppBodyCloserFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<CppBodyCloserFsmFrameEvent>,
    parent_compartment: Option<Box<CppBodyCloserFsmCompartment>>,
}

impl CppBodyCloserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => CppBodyCloserFsmStateContext::Init,
            "Scanning" => CppBodyCloserFsmStateContext::Scanning,
            "InString" => CppBodyCloserFsmStateContext::InString,
            "InCharLiteral" => CppBodyCloserFsmStateContext::InCharLiteral,
            "InLineComment" => CppBodyCloserFsmStateContext::InLineComment,
            "InBlockComment" => CppBodyCloserFsmStateContext::InBlockComment,
            "InRawString" => CppBodyCloserFsmStateContext::InRawString,
            _ => CppBodyCloserFsmStateContext::Empty,
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
pub struct CppBodyCloserFsm {
    _state_stack: Vec<CppBodyCloserFsmCompartment>,
    __compartment: CppBodyCloserFsmCompartment,
    __next_compartment: Option<CppBodyCloserFsmCompartment>,
    _context_stack: Vec<CppBodyCloserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub depth: i32,
    pub result_pos: usize,
    pub error_kind: usize,
    pub error_msg: String,
    pub raw_delim: Vec<u8>,
}

#[allow(non_snake_case)]
impl CppBodyCloserFsm {
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
            raw_delim: Vec::new(),
            __compartment: CppBodyCloserFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(CppBodyCloserFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = CppBodyCloserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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
            "InCharLiteral" => &["InCharLiteral"],
            "InLineComment" => &["InLineComment"],
            "InBlockComment" => &["InBlockComment"],
            "InRawString" => &["InRawString"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> CppBodyCloserFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<CppBodyCloserFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = CppBodyCloserFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<CppBodyCloserFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(CppBodyCloserFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(CppBodyCloserFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, CppBodyCloserFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(CppBodyCloserFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<CppBodyCloserFsmFrameEvent>) {
        let __ev: &CppBodyCloserFsmFrameEvent = __e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "Scanning" => self._state_Scanning(__ev),
            "InString" => self._state_InString(__ev),
            "InCharLiteral" => self._state_InCharLiteral(__ev),
            "InLineComment" => self._state_InLineComment(__ev),
            "InBlockComment" => self._state_InBlockComment(__ev),
            "InRawString" => self._state_InRawString(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: CppBodyCloserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn scan(&mut self) {
        let __e = std::rc::Rc::new(CppBodyCloserFsmFrameEvent::Scan {});
        let mut __ctx = CppBodyCloserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        match __e {
            CppBodyCloserFsmFrameEvent::Scan { .. } => { self._s_Init_hdl_user_scan(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        match __e {
            CppBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_Scanning_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InString(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        match __e {
            CppBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InCharLiteral(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        match __e {
            CppBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InCharLiteral_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InLineComment(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        match __e {
            CppBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InLineComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InBlockComment(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        match __e {
            CppBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InBlockComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InRawString(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        match __e {
            CppBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InRawString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_scan(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Scanning_hdl_frame_enter(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            let b = self.bytes[self.pos];
            if b == b'\n' {
                self.pos += 1;
            } else if b == b'/' && self.pos + 1 < n && self.bytes[self.pos + 1] == b'/' {
                self.pos += 2;
                let mut __compartment = self.__prepareEnter("InLineComment", vec![]);
                self.__transition(__compartment);
                return;
            } else if b == b'/' && self.pos + 1 < n && self.bytes[self.pos + 1] == b'*' {
                self.pos += 2;
                let mut __compartment = self.__prepareEnter("InBlockComment", vec![]);
                self.__transition(__compartment);
                return;
            } else if b == b'\'' {
                self.pos += 1;
                let mut __compartment = self.__prepareEnter("InCharLiteral", vec![]);
                self.__transition(__compartment);
                return;
            } else if b == b'"' {
                self.pos += 1;
                let mut __compartment = self.__prepareEnter("InString", vec![]);
                self.__transition(__compartment);
                return;
            } else if b == b'R' && self.pos + 1 < n && self.bytes[self.pos + 1] == b'"' {
                // C++ raw string R"delim(...)delim"
                let mut j = self.pos + 2;
                let mut delim: Vec<u8> = Vec::new();
                while j < n && self.bytes[j] != b'(' {
                    delim.push(self.bytes[j]);
                    j += 1;
                    if delim.len() > 32 { break; }
                }
                if j >= n || self.bytes[j] != b'(' {
                    self.pos += 1;
                    continue;
                }
                j += 1;
                self.raw_delim = delim;
                self.pos = j;
                let mut __compartment = self.__prepareEnter("InRawString", vec![]);
                self.__transition(__compartment);
                return;
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

    fn _s_InString_hdl_frame_enter(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            if self.bytes[self.pos] == b'\\' {
                self.pos += 2;
                continue;
            }
            if self.bytes[self.pos] == b'"' {
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

    fn _s_InCharLiteral_hdl_frame_enter(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            if self.bytes[self.pos] == b'\\' {
                self.pos += 2;
                continue;
            }
            if self.bytes[self.pos] == b'\'' {
                self.pos += 1;
                let mut __compartment = self.__prepareEnter("Scanning", vec![]);
                self.__transition(__compartment);
                return;
            }
            self.pos += 1;
        }
        self.error_kind = 1;
        self.error_msg = "unterminated char".to_string();
    }

    fn _s_InLineComment_hdl_frame_enter(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_InBlockComment_hdl_frame_enter(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos + 1 < n {
            if self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/' {
                self.pos += 2;
                let mut __compartment = self.__prepareEnter("Scanning", vec![]);
                self.__transition(__compartment);
                return;
            }
            self.pos += 1;
        }
        self.error_kind = 2;
        self.error_msg = "unterminated comment".to_string();
    }

    fn _s_InRawString_hdl_frame_enter(&mut self, __e: &CppBodyCloserFsmFrameEvent) {
        // Find closing )delim"
        let n = self.bytes.len();
        loop {
            if self.pos >= n {
                self.error_kind = 4;
                self.error_msg = "unterminated raw".to_string();
                return
            }
            if self.bytes[self.pos] == b')' {
                let mut k = self.pos + 1;
                let mut m: usize = 0;
                while m < self.raw_delim.len() && k < n && self.bytes[k] == self.raw_delim[m] {
                    k += 1;
                    m += 1;
                }
                if m == self.raw_delim.len() && k < n && self.bytes[k] == b'"' {
                    self.pos = k + 1;
                    let mut __compartment = self.__prepareEnter("Scanning", vec![]);
                    self.__transition(__compartment);
                    return;
                }
            }
            self.pos += 1;
        }
    }
}
