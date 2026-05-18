
// Rust syntax skipper — Frame-generated state machine.
// Delegates to shared helpers where possible; inlines Rust-specific logic.
//
// Helpers used:
//   skip_line_comment, skip_rust_raw_string, skip_rust_string,
//   balanced_paren_end_c_like
// Inline: nested /* */ block comments, find_line_end with raw string awareness

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum RustSyntaxSkipperFsmFrameEvent {
    DoSkipComment {  },
    DoSkipString {  },
    DoFindLineEnd {  },
    DoBalancedParenEnd {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum RustSyntaxSkipperFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl RustSyntaxSkipperFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            RustSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => "do_skip_comment",
            RustSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => "do_skip_string",
            RustSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => "do_find_line_end",
            RustSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => "do_balanced_paren_end",
            RustSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => "$>",
            RustSyntaxSkipperFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum RustSyntaxSkipperFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct RustSyntaxSkipperFsmFrameContext {
    event: std::rc::Rc<RustSyntaxSkipperFsmFrameEvent>,
    _return: Option<RustSyntaxSkipperFsmFrameReturn>,
    _data: std::collections::HashMap<String, RustSyntaxSkipperFsmFrameValue>,
    _transitioned: bool,
}

impl RustSyntaxSkipperFsmFrameContext {
    fn new(event: std::rc::Rc<RustSyntaxSkipperFsmFrameEvent>, default_return: Option<RustSyntaxSkipperFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum RustSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for RustSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        RustSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct RustSyntaxSkipperFsmCompartment {
    state: String,
    state_context: RustSyntaxSkipperFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<RustSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<RustSyntaxSkipperFsmCompartment>>,
}

impl RustSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => RustSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => RustSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => RustSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => RustSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => RustSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => RustSyntaxSkipperFsmStateContext::Empty,
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
pub struct RustSyntaxSkipperFsm {
    _state_stack: Vec<RustSyntaxSkipperFsmCompartment>,
    __compartment: RustSyntaxSkipperFsmCompartment,
    __next_compartment: Option<RustSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<RustSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl RustSyntaxSkipperFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: RustSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(RustSyntaxSkipperFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = RustSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        c._context_stack.push(__ctx);
        c.__kernel(&__e);
        c._context_stack.pop();
        c
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "SkipComment" => &["SkipComment"],
            "SkipString" => &["SkipString"],
            "FindLineEnd" => &["FindLineEnd"],
            "BalancedParenEnd" => &["BalancedParenEnd"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> RustSyntaxSkipperFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<RustSyntaxSkipperFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = RustSyntaxSkipperFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<RustSyntaxSkipperFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(RustSyntaxSkipperFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(RustSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, RustSyntaxSkipperFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(RustSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<RustSyntaxSkipperFsmFrameEvent>) {
        let __ev: &RustSyntaxSkipperFsmFrameEvent = __e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "SkipComment" => self._state_SkipComment(__ev),
            "SkipString" => self._state_SkipString(__ev),
            "FindLineEnd" => self._state_FindLineEnd(__ev),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: RustSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_skip_comment(&mut self) {
        let __e = std::rc::Rc::new(RustSyntaxSkipperFsmFrameEvent::DoSkipComment {});
        let mut __ctx = RustSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let __e = std::rc::Rc::new(RustSyntaxSkipperFsmFrameEvent::DoSkipString {});
        let mut __ctx = RustSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let __e = std::rc::Rc::new(RustSyntaxSkipperFsmFrameEvent::DoFindLineEnd {});
        let mut __ctx = RustSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let __e = std::rc::Rc::new(RustSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd {});
        let mut __ctx = RustSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        match __e {
            RustSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => { self._s_Init_hdl_user_do_balanced_paren_end(__e); }
            RustSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => { self._s_Init_hdl_user_do_find_line_end(__e); }
            RustSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => { self._s_Init_hdl_user_do_skip_comment(__e); }
            RustSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => { self._s_Init_hdl_user_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        match __e {
            RustSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        match __e {
            RustSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        match __e {
            RustSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_FindLineEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        match __e {
            RustSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_BalancedParenEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_balanced_paren_end(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("BalancedParenEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_find_line_end(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("FindLineEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_comment(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipComment", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_string(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipString", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_hdl_frame_enter(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        // Line comment via shared helper
        if let Some(j) = skip_line_comment(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        // Rust nested block comment (different from skip_block_comment — supports nesting)
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        if i + 1 < end && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let mut j = i + 2;
            let mut depth: i32 = 1;
            while j + 1 < end && depth > 0 {
                if bytes[j] == b'/' && bytes[j + 1] == b'*' {
                    depth += 1;
                    j += 2;
                    continue;
                }
                if bytes[j] == b'*' && bytes[j + 1] == b'/' {
                    depth -= 1;
                    j += 2;
                    continue;
                }
                j += 1;
            }
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_SkipString_hdl_frame_enter(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        // Raw string via shared helper (must check before simple string)
        if let Some(j) = skip_rust_raw_string(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        // String/char literal via Rust-specific helper (handles lifetimes)
        if let Some(j) = skip_rust_string(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_FindLineEnd_hdl_frame_enter(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        // Rust-specific: handle raw strings r#"..."# during line scanning
        // Cannot use find_line_end_c_like because it doesn't know about raw strings
        let end = self.end;
        let bytes = &self.bytes;
        let mut j = self.pos;
        let mut in_string: u8 = 0;
        let mut raw_hashes: usize = 0;
        
        while j < end {
            let b = bytes[j];
            if b == b'\n' { break; }
        
            // Inside raw string
            if raw_hashes > 0 {
                if b == b'"' {
                    let mut k = j + 1;
                    let mut matched: usize = 0;
                    while k < end && matched < raw_hashes && bytes[k] == b'#' {
                        matched += 1;
                        k += 1;
                    }
                    if matched == raw_hashes {
                        j = k;
                        raw_hashes = 0;
                        continue;
                    }
                }
                j += 1;
                continue;
            }
        
            // Inside regular string
            if in_string != 0 {
                if b == b'\\' { j += 2; continue; }
                if b == in_string { in_string = 0; }
                j += 1;
                continue;
            }
        
            // Terminators
            if b == b';' || b == b'}' { break; }
            if b == b'/' && j + 1 < end && (bytes[j + 1] == b'/' || bytes[j + 1] == b'*') { break; }
        
            // String starts
            if b == b'\'' || b == b'"' {
                in_string = b;
                j += 1;
                continue;
            }
        
            // Raw string start
            if b == b'r' {
                let mut k = j + 1;
                let mut hashes: usize = 0;
                while k < end && bytes[k] == b'#' {
                    hashes += 1;
                    k += 1;
                }
                if k < end && bytes[k] == b'"' {
                    raw_hashes = hashes;
                    j = k + 1;
                    continue;
                }
            }
        
            j += 1;
        }
        self.result_pos = j;
    }

    fn _s_BalancedParenEnd_hdl_frame_enter(&mut self, __e: &RustSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = balanced_paren_end_c_like(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }
}
