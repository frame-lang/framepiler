
// Lua syntax skipper — Frame-generated state machine.
//
// Handles:
//   -- line comments
//   --[[ ]] block comments (with [=[ ]=] nesting)
//   "..." and '...' strings
//   [[...]] and [=[...]=] long strings
//
// Helpers used:
//   skip_simple_string (for "..." and '...')

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum LuaSyntaxSkipperFsmFrameEvent {
    DoSkipComment {  },
    DoSkipString {  },
    DoFindLineEnd {  },
    DoBalancedParenEnd {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum LuaSyntaxSkipperFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl LuaSyntaxSkipperFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            LuaSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => "do_skip_comment",
            LuaSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => "do_skip_string",
            LuaSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => "do_find_line_end",
            LuaSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => "do_balanced_paren_end",
            LuaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => "$>",
            LuaSyntaxSkipperFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum LuaSyntaxSkipperFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct LuaSyntaxSkipperFsmFrameContext {
    event: std::rc::Rc<LuaSyntaxSkipperFsmFrameEvent>,
    _return: Option<LuaSyntaxSkipperFsmFrameReturn>,
    _data: std::collections::HashMap<String, LuaSyntaxSkipperFsmFrameValue>,
    _transitioned: bool,
}

impl LuaSyntaxSkipperFsmFrameContext {
    fn new(event: std::rc::Rc<LuaSyntaxSkipperFsmFrameEvent>, default_return: Option<LuaSyntaxSkipperFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum LuaSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for LuaSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        LuaSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct LuaSyntaxSkipperFsmCompartment {
    state: String,
    state_context: LuaSyntaxSkipperFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<LuaSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<LuaSyntaxSkipperFsmCompartment>>,
}

impl LuaSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => LuaSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => LuaSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => LuaSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => LuaSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => LuaSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => LuaSyntaxSkipperFsmStateContext::Empty,
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
pub struct LuaSyntaxSkipperFsm {
    _state_stack: Vec<LuaSyntaxSkipperFsmCompartment>,
    __compartment: LuaSyntaxSkipperFsmCompartment,
    __next_compartment: Option<LuaSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<LuaSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl LuaSyntaxSkipperFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: LuaSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(LuaSyntaxSkipperFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = LuaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> LuaSyntaxSkipperFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<LuaSyntaxSkipperFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = LuaSyntaxSkipperFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<LuaSyntaxSkipperFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().unwrap();
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(LuaSyntaxSkipperFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(LuaSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, LuaSyntaxSkipperFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(LuaSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<LuaSyntaxSkipperFsmFrameEvent>) {
        let __ev: &LuaSyntaxSkipperFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "SkipComment" => self._state_SkipComment(__ev),
            "SkipString" => self._state_SkipString(__ev),
            "FindLineEnd" => self._state_FindLineEnd(__ev),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: LuaSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_skip_comment(&mut self) {
        let __e = std::rc::Rc::new(LuaSyntaxSkipperFsmFrameEvent::DoSkipComment {});
        let mut __ctx = LuaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let __e = std::rc::Rc::new(LuaSyntaxSkipperFsmFrameEvent::DoSkipString {});
        let mut __ctx = LuaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let __e = std::rc::Rc::new(LuaSyntaxSkipperFsmFrameEvent::DoFindLineEnd {});
        let mut __ctx = LuaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let __e = std::rc::Rc::new(LuaSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd {});
        let mut __ctx = LuaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e {
            LuaSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => { self._s_Init_hdl_user_do_balanced_paren_end(__e); }
            LuaSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => { self._s_Init_hdl_user_do_find_line_end(__e); }
            LuaSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => { self._s_Init_hdl_user_do_skip_comment(__e); }
            LuaSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => { self._s_Init_hdl_user_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e {
            LuaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e {
            LuaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e {
            LuaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_FindLineEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e {
            LuaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_BalancedParenEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_balanced_paren_end(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("BalancedParenEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_find_line_end(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("FindLineEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_comment(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipComment", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_string(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipString", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_hdl_frame_enter(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        // Must start with --
        if i + 1 >= end || bytes[i] != b'-' || bytes[i + 1] != b'-' {
            self.success = 0;
            return
        }
        
        // Check for block comment --[[ or --[=[
        if i + 3 < end && bytes[i + 2] == b'[' {
            // Count = signs
            let mut level = 0usize;
            let mut j = i + 3;
            while j < end && bytes[j] == b'=' {
                level += 1;
                j += 1;
            }
            if j < end && bytes[j] == b'[' {
                // Block comment: find matching ]=*]
                j += 1;
                while j < end {
                    if bytes[j] == b']' {
                        let mut k = j + 1;
                        let mut matched = 0usize;
                        while k < end && bytes[k] == b'=' && matched < level {
                            matched += 1;
                            k += 1;
                        }
                        if matched == level && k < end && bytes[k] == b']' {
                            self.result_pos = k + 1;
                            self.success = 1;
                            return
                        }
                    }
                    j += 1;
                }
                // Unterminated block comment
                self.result_pos = end;
                self.success = 1;
                return
            }
        }
        
        // Line comment: skip to end of line
        let mut j = i + 2;
        while j < end && bytes[j] != b'\n' {
            j += 1;
        }
        self.result_pos = j;
        self.success = 1;
    }

    fn _s_SkipString_hdl_frame_enter(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        // Long strings: [[ ]] or [=[ ]=]
        if bytes[i] == b'[' {
            let mut level = 0usize;
            let mut j = i + 1;
            while j < end && bytes[j] == b'=' {
                level += 1;
                j += 1;
            }
            if j < end && bytes[j] == b'[' {
                // Long string: find matching ]=*]
                j += 1;
                while j < end {
                    if bytes[j] == b']' {
                        let mut k = j + 1;
                        let mut matched = 0usize;
                        while k < end && bytes[k] == b'=' && matched < level {
                            matched += 1;
                            k += 1;
                        }
                        if matched == level && k < end && bytes[k] == b']' {
                            self.result_pos = k + 1;
                            self.success = 1;
                            return
                        }
                    }
                    j += 1;
                }
                self.result_pos = end;
                self.success = 1;
                return
            }
        }
        
        // Simple strings: "..." or '...'
        if let Some(j) = skip_simple_string(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        
        self.success = 0;
    }

    fn _s_FindLineEnd_hdl_frame_enter(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut j = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        let mut in_string: Option<u8> = None;
        
        while j < end {
            let b = bytes[j];
        
            if b == b'\n' { break; }
        
            if let Some(q) = in_string {
                if b == b'\\' && j + 1 < end {
                    j += 2;
                    continue;
                }
                if b == q { in_string = None; }
                j += 1;
                continue;
            }
        
            // Line comment
            if b == b'-' && j + 1 < end && bytes[j + 1] == b'-' { break; }
            if b == b'\'' || b == b'"' { in_string = Some(b); }
            j += 1;
        }
        self.result_pos = j;
    }

    fn _s_BalancedParenEnd_hdl_frame_enter(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = balanced_paren_end_c_like(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }
}
