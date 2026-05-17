
// Python syntax skipper — Frame-generated state machine.
// Delegates to shared helpers for all scanning logic.
//
// Helpers used:
//   skip_hash_comment, skip_triple_string, skip_simple_string,
//   find_line_end_python, balanced_paren_end_c_like

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum PythonSyntaxSkipperFsmFrameEvent {
    DoSkipComment {  },
    DoSkipString {  },
    DoFindLineEnd {  },
    DoBalancedParenEnd {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum PythonSyntaxSkipperFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl PythonSyntaxSkipperFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            PythonSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => "do_skip_comment",
            PythonSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => "do_skip_string",
            PythonSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => "do_find_line_end",
            PythonSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => "do_balanced_paren_end",
            PythonSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => "$>",
            PythonSyntaxSkipperFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum PythonSyntaxSkipperFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct PythonSyntaxSkipperFsmFrameContext {
    event: std::rc::Rc<PythonSyntaxSkipperFsmFrameEvent>,
    _return: Option<PythonSyntaxSkipperFsmFrameReturn>,
    _data: std::collections::HashMap<String, PythonSyntaxSkipperFsmFrameValue>,
    _transitioned: bool,
}

impl PythonSyntaxSkipperFsmFrameContext {
    fn new(event: std::rc::Rc<PythonSyntaxSkipperFsmFrameEvent>, default_return: Option<PythonSyntaxSkipperFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum PythonSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for PythonSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        PythonSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct PythonSyntaxSkipperFsmCompartment {
    state: String,
    state_context: PythonSyntaxSkipperFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<PythonSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<PythonSyntaxSkipperFsmCompartment>>,
}

impl PythonSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => PythonSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => PythonSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => PythonSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => PythonSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => PythonSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => PythonSyntaxSkipperFsmStateContext::Empty,
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
pub struct PythonSyntaxSkipperFsm {
    _state_stack: Vec<PythonSyntaxSkipperFsmCompartment>,
    __compartment: PythonSyntaxSkipperFsmCompartment,
    __next_compartment: Option<PythonSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<PythonSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl PythonSyntaxSkipperFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: PythonSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(PythonSyntaxSkipperFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = PythonSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> PythonSyntaxSkipperFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<PythonSyntaxSkipperFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = PythonSyntaxSkipperFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<PythonSyntaxSkipperFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().unwrap();
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(PythonSyntaxSkipperFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(PythonSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, PythonSyntaxSkipperFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(PythonSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<PythonSyntaxSkipperFsmFrameEvent>) {
        let __ev: &PythonSyntaxSkipperFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "SkipComment" => self._state_SkipComment(__ev),
            "SkipString" => self._state_SkipString(__ev),
            "FindLineEnd" => self._state_FindLineEnd(__ev),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: PythonSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_skip_comment(&mut self) {
        let __e = std::rc::Rc::new(PythonSyntaxSkipperFsmFrameEvent::DoSkipComment {});
        let mut __ctx = PythonSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let __e = std::rc::Rc::new(PythonSyntaxSkipperFsmFrameEvent::DoSkipString {});
        let mut __ctx = PythonSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let __e = std::rc::Rc::new(PythonSyntaxSkipperFsmFrameEvent::DoFindLineEnd {});
        let mut __ctx = PythonSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let __e = std::rc::Rc::new(PythonSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd {});
        let mut __ctx = PythonSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        match __e {
            PythonSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => { self._s_Init_hdl_user_do_balanced_paren_end(__e); }
            PythonSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => { self._s_Init_hdl_user_do_find_line_end(__e); }
            PythonSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => { self._s_Init_hdl_user_do_skip_comment(__e); }
            PythonSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => { self._s_Init_hdl_user_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        match __e {
            PythonSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        match __e {
            PythonSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        match __e {
            PythonSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_FindLineEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        match __e {
            PythonSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_BalancedParenEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_balanced_paren_end(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("BalancedParenEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_find_line_end(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("FindLineEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_comment(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipComment", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_string(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipString", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_hdl_frame_enter(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = skip_hash_comment(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_SkipString_hdl_frame_enter(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        // Triple-quoted strings (must check before simple)
        if let Some(j) = skip_triple_string(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        // Simple string
        if let Some(j) = skip_simple_string(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_FindLineEnd_hdl_frame_enter(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        self.result_pos = find_line_end_python(&self.bytes, self.pos, self.end);
    }

    fn _s_BalancedParenEnd_hdl_frame_enter(&mut self, __e: &PythonSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = balanced_paren_end_c_like(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }
}
