
// PHP syntax skipper — Frame-generated state machine.
// PHP has // and # line comments, /* */ block comments,
// "..." and '...' strings, and heredoc/nowdoc (<<<EOT...EOT;).
//
// Helpers used:
//   skip_line_comment, skip_block_comment, skip_simple_string, skip_php_heredoc
// Inline: find_line_end and balanced_paren_end

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum PhpSyntaxSkipperFsmFrameEvent {
    DoSkipComment {  },
    DoSkipString {  },
    DoFindLineEnd {  },
    DoBalancedParenEnd {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum PhpSyntaxSkipperFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl PhpSyntaxSkipperFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            PhpSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => "do_skip_comment",
            PhpSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => "do_skip_string",
            PhpSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => "do_find_line_end",
            PhpSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => "do_balanced_paren_end",
            PhpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => "$>",
            PhpSyntaxSkipperFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum PhpSyntaxSkipperFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct PhpSyntaxSkipperFsmFrameContext {
    event: std::rc::Rc<PhpSyntaxSkipperFsmFrameEvent>,
    _return: Option<PhpSyntaxSkipperFsmFrameReturn>,
    _data: std::collections::HashMap<String, PhpSyntaxSkipperFsmFrameValue>,
    _transitioned: bool,
}

impl PhpSyntaxSkipperFsmFrameContext {
    fn new(event: std::rc::Rc<PhpSyntaxSkipperFsmFrameEvent>, default_return: Option<PhpSyntaxSkipperFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum PhpSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for PhpSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        PhpSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct PhpSyntaxSkipperFsmCompartment {
    state: String,
    state_context: PhpSyntaxSkipperFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<PhpSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<PhpSyntaxSkipperFsmCompartment>>,
}

impl PhpSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => PhpSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => PhpSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => PhpSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => PhpSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => PhpSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => PhpSyntaxSkipperFsmStateContext::Empty,
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
pub struct PhpSyntaxSkipperFsm {
    _state_stack: Vec<PhpSyntaxSkipperFsmCompartment>,
    __compartment: PhpSyntaxSkipperFsmCompartment,
    __next_compartment: Option<PhpSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<PhpSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl PhpSyntaxSkipperFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: PhpSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(PhpSyntaxSkipperFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = PhpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> PhpSyntaxSkipperFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<PhpSyntaxSkipperFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = PhpSyntaxSkipperFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<PhpSyntaxSkipperFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(PhpSyntaxSkipperFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(PhpSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, PhpSyntaxSkipperFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(PhpSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<PhpSyntaxSkipperFsmFrameEvent>) {
        let __ev: &PhpSyntaxSkipperFsmFrameEvent = __e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "SkipComment" => self._state_SkipComment(__ev),
            "SkipString" => self._state_SkipString(__ev),
            "FindLineEnd" => self._state_FindLineEnd(__ev),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: PhpSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_skip_comment(&mut self) {
        let __e = std::rc::Rc::new(PhpSyntaxSkipperFsmFrameEvent::DoSkipComment {});
        let mut __ctx = PhpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let __e = std::rc::Rc::new(PhpSyntaxSkipperFsmFrameEvent::DoSkipString {});
        let mut __ctx = PhpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let __e = std::rc::Rc::new(PhpSyntaxSkipperFsmFrameEvent::DoFindLineEnd {});
        let mut __ctx = PhpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let __e = std::rc::Rc::new(PhpSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd {});
        let mut __ctx = PhpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        match __e {
            PhpSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => { self._s_Init_hdl_user_do_balanced_paren_end(__e); }
            PhpSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => { self._s_Init_hdl_user_do_find_line_end(__e); }
            PhpSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => { self._s_Init_hdl_user_do_skip_comment(__e); }
            PhpSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => { self._s_Init_hdl_user_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        match __e {
            PhpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        match __e {
            PhpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        match __e {
            PhpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_FindLineEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        match __e {
            PhpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_BalancedParenEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_balanced_paren_end(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("BalancedParenEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_find_line_end(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("FindLineEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_comment(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipComment", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_string(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipString", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_hdl_frame_enter(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        // PHP: //, #, or /* */
        if let Some(j) = skip_line_comment(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        // # comment
        if self.pos < self.end && self.bytes[self.pos] == b'#' {
            let mut j = self.pos + 1;
            while j < self.end && self.bytes[j] != b'\n' {
                j += 1;
            }
            self.result_pos = j;
            self.success = 1;
            return
        }
        if let Some(j) = skip_block_comment(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_SkipString_hdl_frame_enter(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        // Heredoc/nowdoc (must check before simple strings — <<< prefix)
        if let Some(j) = skip_php_heredoc(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        // Simple string via shared helper (handles both ' and ")
        if let Some(j) = skip_simple_string(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_FindLineEnd_hdl_frame_enter(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        let end = self.end;
        let bytes = &self.bytes;
        let mut j = self.pos;
        let mut in_string: u8 = 0;
        
        while j < end {
            let b = bytes[j];
            if b == b'\n' { break; }
        
            // Inside string
            if in_string != 0 {
                if b == b'\\' { j += 2; continue; }
                if b == in_string { in_string = 0; }
                j += 1;
                continue;
            }
        
            // Terminators
            if b == b';' || b == b'}' { break; }
            if b == b'/' && j + 1 < end && (bytes[j + 1] == b'/' || bytes[j + 1] == b'*') { break; }
            if b == b'#' { break; }
        
            // String starts
            if b == b'\'' || b == b'"' {
                in_string = b;
                j += 1;
                continue;
            }
        
            j += 1;
        }
        self.result_pos = j;
    }

    fn _s_BalancedParenEnd_hdl_frame_enter(&mut self, __e: &PhpSyntaxSkipperFsmFrameEvent) {
        let end = self.end;
        let bytes = &self.bytes;
        let mut i = self.pos;
        if i >= end || bytes[i] != b'(' {
            self.success = 0;
            return
        }
        let mut depth: i32 = 0;
        let mut in_string: u8 = 0;
        
        while i < end {
            let b = bytes[i];
        
            // Inside string
            if in_string != 0 {
                if b == b'\\' { i += 2; continue; }
                if b == in_string { in_string = 0; }
                i += 1;
                continue;
            }
        
            if b == b'\'' || b == b'"' { in_string = b; i += 1; }
            else if b == b'(' { depth += 1; i += 1; }
            else if b == b')' {
                depth -= 1; i += 1;
                if depth == 0 { self.result_pos = i; self.success = 1;                        return }
            } else { i += 1; }
        }
        self.success = 0;
    }
}
