
// Kotlin syntax skipper — Frame-generated state machine.
// Delegates to shared helpers; adds Kotlin raw strings """..."""
//
// Helpers used:
//   skip_line_comment, skip_block_comment, skip_simple_string,
//   find_line_end_c_like, balanced_paren_end_c_like
// Inline: """...""" raw strings (checked before skip_simple_string)

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum KotlinSyntaxSkipperFsmFrameEvent {
    DoSkipComment {  },
    DoSkipString {  },
    DoFindLineEnd {  },
    DoBalancedParenEnd {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum KotlinSyntaxSkipperFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl KotlinSyntaxSkipperFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            KotlinSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => "do_skip_comment",
            KotlinSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => "do_skip_string",
            KotlinSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => "do_find_line_end",
            KotlinSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => "do_balanced_paren_end",
            KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => "$>",
            KotlinSyntaxSkipperFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum KotlinSyntaxSkipperFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct KotlinSyntaxSkipperFsmFrameContext {
    event: std::rc::Rc<KotlinSyntaxSkipperFsmFrameEvent>,
    _return: Option<KotlinSyntaxSkipperFsmFrameReturn>,
    _data: std::collections::HashMap<String, KotlinSyntaxSkipperFsmFrameValue>,
    _transitioned: bool,
}

impl KotlinSyntaxSkipperFsmFrameContext {
    fn new(event: std::rc::Rc<KotlinSyntaxSkipperFsmFrameEvent>, default_return: Option<KotlinSyntaxSkipperFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum KotlinSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for KotlinSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        KotlinSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct KotlinSyntaxSkipperFsmCompartment {
    state: String,
    state_context: KotlinSyntaxSkipperFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<KotlinSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<KotlinSyntaxSkipperFsmCompartment>>,
}

impl KotlinSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => KotlinSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => KotlinSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => KotlinSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => KotlinSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => KotlinSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => KotlinSyntaxSkipperFsmStateContext::Empty,
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
pub struct KotlinSyntaxSkipperFsm {
    _state_stack: Vec<KotlinSyntaxSkipperFsmCompartment>,
    __compartment: KotlinSyntaxSkipperFsmCompartment,
    __next_compartment: Option<KotlinSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<KotlinSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl KotlinSyntaxSkipperFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: KotlinSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = KotlinSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
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

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> KotlinSyntaxSkipperFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<KotlinSyntaxSkipperFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = KotlinSyntaxSkipperFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<KotlinSyntaxSkipperFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(KotlinSyntaxSkipperFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<KotlinSyntaxSkipperFsmFrameEvent>) {
        let __ev: &KotlinSyntaxSkipperFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "SkipComment" => self._state_SkipComment(__ev),
            "SkipString" => self._state_SkipString(__ev),
            "FindLineEnd" => self._state_FindLineEnd(__ev),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: KotlinSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_skip_comment(&mut self) {
        let __e = std::rc::Rc::new(KotlinSyntaxSkipperFsmFrameEvent::DoSkipComment {});
        let mut __ctx = KotlinSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let __e = std::rc::Rc::new(KotlinSyntaxSkipperFsmFrameEvent::DoSkipString {});
        let mut __ctx = KotlinSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let __e = std::rc::Rc::new(KotlinSyntaxSkipperFsmFrameEvent::DoFindLineEnd {});
        let mut __ctx = KotlinSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let __e = std::rc::Rc::new(KotlinSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd {});
        let mut __ctx = KotlinSyntaxSkipperFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        match __e {
            KotlinSyntaxSkipperFsmFrameEvent::DoBalancedParenEnd { .. } => { self._s_Init_hdl_user_do_balanced_paren_end(__e); }
            KotlinSyntaxSkipperFsmFrameEvent::DoFindLineEnd { .. } => { self._s_Init_hdl_user_do_find_line_end(__e); }
            KotlinSyntaxSkipperFsmFrameEvent::DoSkipComment { .. } => { self._s_Init_hdl_user_do_skip_comment(__e); }
            KotlinSyntaxSkipperFsmFrameEvent::DoSkipString { .. } => { self._s_Init_hdl_user_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        match __e {
            KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipComment_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        match __e {
            KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_SkipString_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        match __e {
            KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_FindLineEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        match __e {
            KotlinSyntaxSkipperFsmFrameEvent::FrameEnter { .. } => { self._s_BalancedParenEnd_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_balanced_paren_end(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("BalancedParenEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_find_line_end(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("FindLineEnd", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_comment(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipComment", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_hdl_user_do_skip_string(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("SkipString", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_hdl_frame_enter(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
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

    fn _s_SkipString_hdl_frame_enter(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        // Kotlin raw string """...""" (must check before simple string)
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

    fn _s_FindLineEnd_hdl_frame_enter(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        self.result_pos = find_line_end_c_like(&self.bytes, self.pos, self.end);
    }

    fn _s_BalancedParenEnd_hdl_frame_enter(&mut self, __e: &KotlinSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = balanced_paren_end_c_like(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }
}
