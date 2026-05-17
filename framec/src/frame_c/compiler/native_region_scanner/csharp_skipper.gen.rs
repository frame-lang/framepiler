
// C# syntax skipper — Frame-generated state machine.
// Delegates to shared helpers where possible; inlines C#-specific string forms.
//
// Helpers used:
//   skip_hash_comment (preprocessor), skip_line_comment, skip_block_comment,
//   skip_simple_string, find_line_end_c_like, balanced_paren_end_c_like
// Inline: @"..." verbatim strings, $"..." interpolated strings, $"""...""" raw strings

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum CSharpSyntaxSkipperFsmFrameEvent {
    DoSkipComment {  },
    DoSkipString {  },
    DoFindLineEnd {  },
    DoBalancedParenEnd {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum CSharpSyntaxSkipperFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl CSharpSyntaxSkipperFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            CSharpSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => "do_skip_comment",
            CSharpSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => "do_skip_string",
            CSharpSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => "do_find_line_end",
            CSharpSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => "do_balanced_paren_end",
            CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => "$>",
            CSharpSyntaxSkipperFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum CSharpSyntaxSkipperFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct CSharpSyntaxSkipperFsmFrameContext {
    event: std::rc::Rc<CSharpSyntaxSkipperFsmFrameEvent>,
    _return: Option<CSharpSyntaxSkipperFsmFrameReturn>,
    _data: std::collections::HashMap<String, CSharpSyntaxSkipperFsmFrameValue>,
    _transitioned: bool,
}

impl CSharpSyntaxSkipperFsmFrameContext {
    fn new(event: std::rc::Rc<CSharpSyntaxSkipperFsmFrameEvent>, default_return: Option<CSharpSyntaxSkipperFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum CSharpSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for CSharpSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        CSharpSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct CSharpSyntaxSkipperFsmCompartment {
    state: String,
    state_context: CSharpSyntaxSkipperFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<CSharpSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<CSharpSyntaxSkipperFsmCompartment>>,
}

impl CSharpSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => CSharpSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => CSharpSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => CSharpSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => CSharpSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => CSharpSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => CSharpSyntaxSkipperFsmStateContext::Empty,
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
pub struct CSharpSyntaxSkipperFsm {
    _state_stack: Vec<CSharpSyntaxSkipperFsmCompartment>,
    __compartment: CSharpSyntaxSkipperFsmCompartment,
    __next_compartment: Option<CSharpSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<CSharpSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl CSharpSyntaxSkipperFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: CSharpSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = CSharpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> CSharpSyntaxSkipperFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<CSharpSyntaxSkipperFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = CSharpSyntaxSkipperFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<CSharpSyntaxSkipperFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(CSharpSyntaxSkipperFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<CSharpSyntaxSkipperFsmFrameEvent>) {
        let __ev: &CSharpSyntaxSkipperFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "SkipComment" => self._state_SkipComment(__ev),
            "SkipString" => self._state_SkipString(__ev),
            "FindLineEnd" => self._state_FindLineEnd(__ev),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: CSharpSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_skip_comment(&mut self) {
        let __e = std::rc::Rc::new(CSharpSyntaxSkipperFsmFrameEvent::DoSkipComment {});
        let mut __ctx = CSharpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let __e = std::rc::Rc::new(CSharpSyntaxSkipperFsmFrameEvent::DoSkipString {});
        let mut __ctx = CSharpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let __e = std::rc::Rc::new(CSharpSyntaxSkipperFsmFrameEvent::DoFindLineEnd {});
        let mut __ctx = CSharpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let __e = std::rc::Rc::new(CSharpSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd {});
        let mut __ctx = CSharpSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        match __e {
            CSharpSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => { self._s_Init_hdl_user_do_balanced_paren_end(__e); }
            CSharpSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => { self._s_Init_hdl_user_do_find_line_end(__e); }
            CSharpSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => { self._s_Init_hdl_user_do_skip_comment(__e); }
            CSharpSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => { self._s_Init_hdl_user_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        match __e {
            CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        match __e {
            CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        match __e {
            CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_FindLineEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        match __e {
            CSharpSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_BalancedParenEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_balanced_paren_end(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("BalancedParenEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_find_line_end(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("FindLineEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_comment(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipComment", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_string(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipString", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_hdl_frame_enter(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        // Preprocessor directive (uses skip_hash_comment)
        if let Some(j) = skip_hash_comment(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        // Line comment
        if let Some(j) = skip_line_comment(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        // Block comment
        if let Some(j) = skip_block_comment(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_SkipString_hdl_frame_enter(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        let b0 = bytes[i];
        
        // Verbatim string @"..." (doubled quotes for escape)
        if b0 == b'@' && i + 1 < end && bytes[i + 1] == b'"' {
            let mut j = i + 2;
            while j < end {
                if bytes[j] == b'"' {
                    if j + 1 < end && bytes[j + 1] == b'"' {
                        j += 2; // escaped quote
                        continue;
                    }
                    self.result_pos = j + 1;
                    self.success = 1;
                    return
                }
                j += 1;
            }
            self.result_pos = end;
            self.success = 1;
            return
        }
        
        // Interpolated string $"..." or $@"..." or raw $"""..."""
        if b0 == b'$' {
            let mut j = i + 1;
            // Skip additional $
            while j < end && bytes[j] == b'$' { j += 1; }
            // Check for @
            if j < end && bytes[j] == b'@' { j += 1; }
            // Count opening quotes
            let mut quotes: usize = 0;
            while j < end && bytes[j] == b'"' {
                quotes += 1;
                j += 1;
            }
            if quotes == 0 {
                self.success = 0;
                return
            }
            // Raw string (3+ quotes)
            if quotes >= 3 {
                while j < end {
                    if bytes[j] == b'"' {
                        let mut q: usize = 0;
                        let mut p = j;
                        while p < end && bytes[p] == b'"' {
                            q += 1;
                            p += 1;
                        }
                        if q >= quotes {
                            self.result_pos = p;
                            self.success = 1;
                            return
                        }
                        j = p;
                        continue;
                    }
                    j += 1;
                }
                self.result_pos = end;
                self.success = 1;
                return
            }
            // Normal interpolated string
            while j < end {
                if bytes[j] == b'\\' { j += 2; continue; }
                if bytes[j] == b'"' {
                    self.result_pos = j + 1;
                    self.success = 1;
                    return
                }
                j += 1;
            }
            self.result_pos = end;
            self.success = 1;
            return
        }
        
        // Simple string via shared helper
        if let Some(j) = skip_simple_string(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_FindLineEnd_hdl_frame_enter(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        self.result_pos = find_line_end_c_like(&self.bytes, self.pos, self.end);
    }

    fn _s_BalancedParenEnd_hdl_frame_enter(&mut self, __e: &CSharpSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = balanced_paren_end_c_like(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }
}
