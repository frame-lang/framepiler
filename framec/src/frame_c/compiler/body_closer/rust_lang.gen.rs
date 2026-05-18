
// Dogfooded body closer — Rust language brace matcher.
// Extends C with nested block comments (/* /* */ */) and raw strings (r#"..."#).
//
// State machine flow:
//   $Init.scan() → $Scanning.$>() ↔ $InString/$InCharLiteral/$InLineComment/$InBlockComment/$InRawString

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum RustBodyCloserFsmFrameEvent {
    Scan {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum RustBodyCloserFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl RustBodyCloserFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            RustBodyCloserFsmFrameEvent::Scan { .. } => "scan",
            RustBodyCloserFsmFrameEvent::FrameEnter { .. } => "$>",
            RustBodyCloserFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum RustBodyCloserFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct RustBodyCloserFsmFrameContext {
    event: std::rc::Rc<RustBodyCloserFsmFrameEvent>,
    _return: Option<RustBodyCloserFsmFrameReturn>,
    _data: std::collections::HashMap<String, RustBodyCloserFsmFrameValue>,
    _transitioned: bool,
}

impl RustBodyCloserFsmFrameContext {
    fn new(event: std::rc::Rc<RustBodyCloserFsmFrameEvent>, default_return: Option<RustBodyCloserFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum RustBodyCloserFsmStateContext {
    Init,
    Scanning,
    InString,
    InCharLiteral,
    InLineComment,
    InBlockComment,
    InRawString,
    Empty,
}

impl Default for RustBodyCloserFsmStateContext {
    fn default() -> Self {
        RustBodyCloserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct RustBodyCloserFsmCompartment {
    state: String,
    state_context: RustBodyCloserFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<RustBodyCloserFsmFrameEvent>,
    parent_compartment: Option<Box<RustBodyCloserFsmCompartment>>,
}

impl RustBodyCloserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => RustBodyCloserFsmStateContext::Init,
            "Scanning" => RustBodyCloserFsmStateContext::Scanning,
            "InString" => RustBodyCloserFsmStateContext::InString,
            "InCharLiteral" => RustBodyCloserFsmStateContext::InCharLiteral,
            "InLineComment" => RustBodyCloserFsmStateContext::InLineComment,
            "InBlockComment" => RustBodyCloserFsmStateContext::InBlockComment,
            "InRawString" => RustBodyCloserFsmStateContext::InRawString,
            _ => RustBodyCloserFsmStateContext::Empty,
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
pub struct RustBodyCloserFsm {
    _state_stack: Vec<RustBodyCloserFsmCompartment>,
    __compartment: RustBodyCloserFsmCompartment,
    __next_compartment: Option<RustBodyCloserFsmCompartment>,
    _context_stack: Vec<RustBodyCloserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub depth: i32,
    pub result_pos: usize,
    pub error_kind: usize,
    pub error_msg: String,
    pub block_comment_nest: i32,
    pub raw_hashes: usize,
}

#[allow(non_snake_case)]
impl RustBodyCloserFsm {
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
            block_comment_nest: 0,
            raw_hashes: 0,
            __compartment: RustBodyCloserFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(RustBodyCloserFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = RustBodyCloserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> RustBodyCloserFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<RustBodyCloserFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = RustBodyCloserFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<RustBodyCloserFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(RustBodyCloserFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(RustBodyCloserFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, RustBodyCloserFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(RustBodyCloserFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<RustBodyCloserFsmFrameEvent>) {
        let __ev: &RustBodyCloserFsmFrameEvent = __e;
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

    fn __transition(&mut self, next_compartment: RustBodyCloserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn scan(&mut self) {
        let __e = std::rc::Rc::new(RustBodyCloserFsmFrameEvent::Scan {});
        let mut __ctx = RustBodyCloserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        match __e {
            RustBodyCloserFsmFrameEvent::Scan { .. } => { self._s_Init_hdl_user_scan(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        match __e {
            RustBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_Scanning_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InString(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        match __e {
            RustBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InCharLiteral(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        match __e {
            RustBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InCharLiteral_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InLineComment(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        match __e {
            RustBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InLineComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InBlockComment(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        match __e {
            RustBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InBlockComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InRawString(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        match __e {
            RustBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InRawString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_scan(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Scanning_hdl_frame_enter(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
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
                self.block_comment_nest = 1;
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
            } else if b == b'r' {
                // Rust raw string r#"..."# or just r"..."
                let mut j = self.pos + 1;
                let mut hashes: usize = 0;
                while j < n && self.bytes[j] == b'#' {
                    hashes += 1;
                    j += 1;
                }
                if j < n && self.bytes[j] == b'"' {
                    self.raw_hashes = hashes;
                    self.pos = j + 1;
                    let mut __compartment = self.__prepareEnter("InRawString", vec![]);
                    self.__transition(__compartment);
                    return;
                } else {
                    self.pos += 1;
                    continue;
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

    fn _s_InString_hdl_frame_enter(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
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

    fn _s_InCharLiteral_hdl_frame_enter(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
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

    fn _s_InLineComment_hdl_frame_enter(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_InBlockComment_hdl_frame_enter(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        // Rust supports nested block comments: /* /* */ */
        let n = self.bytes.len();
        while self.pos + 1 < n {
            if self.bytes[self.pos] == b'/' && self.bytes[self.pos + 1] == b'*' {
                self.block_comment_nest += 1;
                self.pos += 2;
                continue;
            }
            if self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/' {
                self.block_comment_nest -= 1;
                self.pos += 2;
                if self.block_comment_nest == 0 {
                    let mut __compartment = self.__prepareEnter("Scanning", vec![]);
                    self.__transition(__compartment);
                    return;
                }
                continue;
            }
            self.pos += 1;
        }
        self.error_kind = 2;
        self.error_msg = "unterminated comment".to_string();
    }

    fn _s_InRawString_hdl_frame_enter(&mut self, __e: &RustBodyCloserFsmFrameEvent) {
        // Find closing "###
        let n = self.bytes.len();
        loop {
            if self.pos >= n {
                self.error_kind = 4;
                self.error_msg = "unterminated raw".to_string();
                return
            }
            if self.bytes[self.pos] == b'"' {
                let mut k = self.pos + 1;
                let mut m: usize = 0;
                while m < self.raw_hashes && k < n && self.bytes[k] == b'#' {
                    m += 1;
                    k += 1;
                }
                if m == self.raw_hashes {
                    self.pos = k;
                    let mut __compartment = self.__prepareEnter("Scanning", vec![]);
                    self.__transition(__compartment);
                    return;
                }
            }
            self.pos += 1;
        }
    }
}
