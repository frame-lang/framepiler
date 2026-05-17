
// Java syntax skipper — Frame-generated state machine.
// Delegates to shared helpers; adds Java 15+ text blocks """..."""
//
// Helpers used:
//   skip_line_comment, skip_block_comment, skip_simple_string,
//   find_line_end_c_like, balanced_paren_end_c_like
// Inline: """...""" text blocks (checked before skip_simple_string)

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum JavaSyntaxSkipperFsmFrameEvent {
    DoSkipComment {  },
    DoSkipString {  },
    DoFindLineEnd {  },
    DoBalancedParenEnd {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum JavaSyntaxSkipperFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl JavaSyntaxSkipperFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            JavaSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => "do_skip_comment",
            JavaSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => "do_skip_string",
            JavaSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => "do_find_line_end",
            JavaSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => "do_balanced_paren_end",
            JavaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => "$>",
            JavaSyntaxSkipperFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum JavaSyntaxSkipperFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct JavaSyntaxSkipperFsmFrameContext {
    event: std::rc::Rc<JavaSyntaxSkipperFsmFrameEvent>,
    _return: Option<JavaSyntaxSkipperFsmFrameReturn>,
    _data: std::collections::HashMap<String, JavaSyntaxSkipperFsmFrameValue>,
    _transitioned: bool,
}

impl JavaSyntaxSkipperFsmFrameContext {
    fn new(event: std::rc::Rc<JavaSyntaxSkipperFsmFrameEvent>, default_return: Option<JavaSyntaxSkipperFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum JavaSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for JavaSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        JavaSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct JavaSyntaxSkipperFsmCompartment {
    state: String,
    state_context: JavaSyntaxSkipperFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<JavaSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<JavaSyntaxSkipperFsmCompartment>>,
}

impl JavaSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => JavaSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => JavaSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => JavaSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => JavaSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => JavaSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => JavaSyntaxSkipperFsmStateContext::Empty,
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
pub struct JavaSyntaxSkipperFsm {
    _state_stack: Vec<JavaSyntaxSkipperFsmCompartment>,
    __compartment: JavaSyntaxSkipperFsmCompartment,
    __next_compartment: Option<JavaSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<JavaSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl JavaSyntaxSkipperFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: JavaSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(JavaSyntaxSkipperFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = JavaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> JavaSyntaxSkipperFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<JavaSyntaxSkipperFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = JavaSyntaxSkipperFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<JavaSyntaxSkipperFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(JavaSyntaxSkipperFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(JavaSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, JavaSyntaxSkipperFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(JavaSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<JavaSyntaxSkipperFsmFrameEvent>) {
        let __ev: &JavaSyntaxSkipperFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "SkipComment" => self._state_SkipComment(__ev),
            "SkipString" => self._state_SkipString(__ev),
            "FindLineEnd" => self._state_FindLineEnd(__ev),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: JavaSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_skip_comment(&mut self) {
        let __e = std::rc::Rc::new(JavaSyntaxSkipperFsmFrameEvent::DoSkipComment {});
        let mut __ctx = JavaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let __e = std::rc::Rc::new(JavaSyntaxSkipperFsmFrameEvent::DoSkipString {});
        let mut __ctx = JavaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let __e = std::rc::Rc::new(JavaSyntaxSkipperFsmFrameEvent::DoFindLineEnd {});
        let mut __ctx = JavaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let __e = std::rc::Rc::new(JavaSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd {});
        let mut __ctx = JavaSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        match __e {
            JavaSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => { self._s_Init_hdl_user_do_balanced_paren_end(__e); }
            JavaSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => { self._s_Init_hdl_user_do_find_line_end(__e); }
            JavaSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => { self._s_Init_hdl_user_do_skip_comment(__e); }
            JavaSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => { self._s_Init_hdl_user_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        match __e {
            JavaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        match __e {
            JavaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        match __e {
            JavaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_FindLineEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        match __e {
            JavaSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_BalancedParenEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_balanced_paren_end(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("BalancedParenEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_find_line_end(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("FindLineEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_comment(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipComment", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_string(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipString", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_hdl_frame_enter(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = skip_line_comment(&self.bytes, self.pos, self.end) {
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

    fn _s_SkipString_hdl_frame_enter(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        // Java text block """...""" (must check before simple string)
        if i + 2 < end && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
            let mut j = i + 3;
            while j + 2 < end {
                if bytes[j] == b'"' && bytes[j + 1] == b'"' && bytes[j + 2] == b'"' {
                    self.result_pos = j + 3;
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

    fn _s_FindLineEnd_hdl_frame_enter(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        self.result_pos = find_line_end_c_like(&self.bytes, self.pos, self.end);
    }

    fn _s_BalancedParenEnd_hdl_frame_enter(&mut self, __e: &JavaSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = balanced_paren_end_c_like(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }
}
