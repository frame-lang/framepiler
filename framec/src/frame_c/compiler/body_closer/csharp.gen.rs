
// Dogfooded body closer — C# language brace matcher.
// C#-specific: verbatim strings (@"..."), interpolated ($"..."), raw ($"""..."""),
// combined (@$"..." / $@"..."), preprocessor directives (#region etc.), char literals.
//
// State machine flow:
//   $Init.scan() → $Scanning.$>() ↔ $InString/$InCharLiteral/$InVerbatimString/$InRawString
//                                    /$InLineComment/$InBlockComment/$InPreprocessor

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum CsBodyCloserFsmFrameEvent {
    Scan {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum CsBodyCloserFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl CsBodyCloserFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            CsBodyCloserFsmFrameEvent::Scan { .. } => "scan",
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => "$>",
            CsBodyCloserFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum CsBodyCloserFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct CsBodyCloserFsmFrameContext {
    event: std::rc::Rc<CsBodyCloserFsmFrameEvent>,
    _return: Option<CsBodyCloserFsmFrameReturn>,
    _data: std::collections::HashMap<String, CsBodyCloserFsmFrameValue>,
    _transitioned: bool,
}

impl CsBodyCloserFsmFrameContext {
    fn new(event: std::rc::Rc<CsBodyCloserFsmFrameEvent>, default_return: Option<CsBodyCloserFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum CsBodyCloserFsmStateContext {
    Init,
    Scanning,
    InString,
    InCharLiteral,
    InVerbatimString,
    InRawString,
    InLineComment,
    InBlockComment,
    InPreprocessor,
    Empty,
}

impl Default for CsBodyCloserFsmStateContext {
    fn default() -> Self {
        CsBodyCloserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct CsBodyCloserFsmCompartment {
    state: String,
    state_context: CsBodyCloserFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<CsBodyCloserFsmFrameEvent>,
    parent_compartment: Option<Box<CsBodyCloserFsmCompartment>>,
}

impl CsBodyCloserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => CsBodyCloserFsmStateContext::Init,
            "Scanning" => CsBodyCloserFsmStateContext::Scanning,
            "InString" => CsBodyCloserFsmStateContext::InString,
            "InCharLiteral" => CsBodyCloserFsmStateContext::InCharLiteral,
            "InVerbatimString" => CsBodyCloserFsmStateContext::InVerbatimString,
            "InRawString" => CsBodyCloserFsmStateContext::InRawString,
            "InLineComment" => CsBodyCloserFsmStateContext::InLineComment,
            "InBlockComment" => CsBodyCloserFsmStateContext::InBlockComment,
            "InPreprocessor" => CsBodyCloserFsmStateContext::InPreprocessor,
            _ => CsBodyCloserFsmStateContext::Empty,
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
pub struct CsBodyCloserFsm {
    _state_stack: Vec<CsBodyCloserFsmCompartment>,
    __compartment: CsBodyCloserFsmCompartment,
    __next_compartment: Option<CsBodyCloserFsmCompartment>,
    _context_stack: Vec<CsBodyCloserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub depth: i32,
    pub result_pos: usize,
    pub error_kind: usize,
    pub error_msg: String,
    pub raw_quotes: usize,
}

#[allow(non_snake_case)]
impl CsBodyCloserFsm {
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
            raw_quotes: 0,
            __compartment: CsBodyCloserFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(CsBodyCloserFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = CsBodyCloserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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
            "InVerbatimString" => &["InVerbatimString"],
            "InRawString" => &["InRawString"],
            "InLineComment" => &["InLineComment"],
            "InBlockComment" => &["InBlockComment"],
            "InPreprocessor" => &["InPreprocessor"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> CsBodyCloserFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<CsBodyCloserFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = CsBodyCloserFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<CsBodyCloserFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().unwrap();
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(CsBodyCloserFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(CsBodyCloserFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, CsBodyCloserFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(CsBodyCloserFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<CsBodyCloserFsmFrameEvent>) {
        let __ev: &CsBodyCloserFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "Scanning" => self._state_Scanning(__ev),
            "InString" => self._state_InString(__ev),
            "InCharLiteral" => self._state_InCharLiteral(__ev),
            "InVerbatimString" => self._state_InVerbatimString(__ev),
            "InRawString" => self._state_InRawString(__ev),
            "InLineComment" => self._state_InLineComment(__ev),
            "InBlockComment" => self._state_InBlockComment(__ev),
            "InPreprocessor" => self._state_InPreprocessor(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: CsBodyCloserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn scan(&mut self) {
        let __e = std::rc::Rc::new(CsBodyCloserFsmFrameEvent::Scan {});
        let mut __ctx = CsBodyCloserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::Scan { .. } => { self._s_Init_hdl_user_scan(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_Scanning_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InString(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InCharLiteral(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InCharLiteral_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InVerbatimString(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InVerbatimString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InRawString(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InRawString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InLineComment(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InLineComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InBlockComment(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InBlockComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_InPreprocessor(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        match __e {
            CsBodyCloserFsmFrameEvent::FrameEnter { .. } => { self._s_InPreprocessor_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_scan(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Scanning_hdl_frame_enter(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
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
            } else if b == b'#' {
                self.pos += 1;
                let mut __compartment = self.__prepareEnter("InPreprocessor", vec![]);
                self.__transition(__compartment);
                return;
            } else if b == b'\'' {
                self.pos += 1;
                let mut __compartment = self.__prepareEnter("InCharLiteral", vec![]);
                self.__transition(__compartment);
                return;
            } else if b == b'@' {
                // @"verbatim" or @$"verbatim interp"
                if self.pos + 1 < n && self.bytes[self.pos + 1] == b'"' {
                    self.pos += 2;
                    let mut __compartment = self.__prepareEnter("InVerbatimString", vec![]);
                    self.__transition(__compartment);
                    return;
                } else if self.pos + 2 < n && self.bytes[self.pos + 1] == b'$' && self.bytes[self.pos + 2] == b'"' {
                    self.pos += 3;
                    let mut __compartment = self.__prepareEnter("InVerbatimString", vec![]);
                    self.__transition(__compartment);
                    return;
                } else {
                    self.pos += 1;
                }
            } else if b == b'$' {
                // $"interp" or $"""raw""" or $$"..." etc.
                let mut j = self.pos;
                let mut _dollars: usize = 0;
                while j < n && self.bytes[j] == b'$' { _dollars += 1; j += 1; }
                // Check for @$"..." → verbatim interp (handled above via @)
                let mut k = j;
                let mut quotes: usize = 0;
                while k < n && self.bytes[k] == b'"' { quotes += 1; k += 1; }
                if quotes >= 3 {
                    // Raw string $"""..."""
                    self.raw_quotes = quotes;
                    self.pos = k;
                    let mut __compartment = self.__prepareEnter("InRawString", vec![]);
                    self.__transition(__compartment);
                    return;
                } else if j < n && self.bytes[j] == b'"' {
                    // $"interpolated"
                    self.pos = j + 1;
                    let mut __compartment = self.__prepareEnter("InString", vec![]);
                    self.__transition(__compartment);
                    return;
                } else {
                    self.pos += 1;
                }
            } else if b == b'"' {
                self.pos += 1;
                let mut __compartment = self.__prepareEnter("InString", vec![]);
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

    fn _s_InString_hdl_frame_enter(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
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

    fn _s_InCharLiteral_hdl_frame_enter(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
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

    fn _s_InVerbatimString_hdl_frame_enter(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        // Verbatim: "" is escape for literal quote, single " ends
        let n = self.bytes.len();
        while self.pos < n {
            if self.pos + 1 < n && self.bytes[self.pos] == b'"' && self.bytes[self.pos + 1] == b'"' {
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
        self.error_msg = "unterminated verbatim string".to_string();
    }

    fn _s_InRawString_hdl_frame_enter(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        // Raw string: close when we see N consecutive quotes (where N = raw_quotes)
        let n = self.bytes.len();
        while self.pos < n {
            if self.bytes[self.pos] == b'"' {
                let mut q: usize = 0;
                let mut p = self.pos;
                while p < n && self.bytes[p] == b'"' { q += 1; p += 1; }
                if q >= self.raw_quotes {
                    self.pos = p;
                    let mut __compartment = self.__prepareEnter("Scanning", vec![]);
                    self.__transition(__compartment);
                    return;
                }
                self.pos = p;
            } else {
                self.pos += 1;
            }
        }
        self.error_kind = 4;
        self.error_msg = "unterminated raw".to_string();
    }

    fn _s_InLineComment_hdl_frame_enter(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_InBlockComment_hdl_frame_enter(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
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

    fn _s_InPreprocessor_hdl_frame_enter(&mut self, __e: &CsBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }
}
