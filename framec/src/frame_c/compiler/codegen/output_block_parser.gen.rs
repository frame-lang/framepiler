
// Output Block Parser — Frame state machine.
//
// Consumes exhaustive token stream from OutputBlockLexer.
// Every token is either emitted as-is or transformed.
//
// Since the lexer covers every byte, the parser outputs exactly
// the same text if no transformations apply.
//
// Lua mode (mode=1):
//   IF TEXT LBRACE → "if" TEXT "then"
//   RBRACE ELSE LBRACE → "else"
//   RBRACE ELSE LBRACE IF TEXT LBRACE → "elseif" TEXT "then"
//   WHILE TEXT LBRACE → "while" TEXT "do"
//   RBRACE (block close) → "end"
//   RETURN → emit + mark terminal (skip subsequent non-comment tokens)

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum OutputBlockParserFsmFrameEvent {
    DoParse {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum OutputBlockParserFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl OutputBlockParserFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            OutputBlockParserFsmFrameEvent::DoParse { .. } => "do_parse",
            OutputBlockParserFsmFrameEvent::FrameEnter { .. } => "$>",
            OutputBlockParserFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum OutputBlockParserFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct OutputBlockParserFsmFrameContext {
    event: std::rc::Rc<OutputBlockParserFsmFrameEvent>,
    _return: Option<OutputBlockParserFsmFrameReturn>,
    _data: std::collections::HashMap<String, OutputBlockParserFsmFrameValue>,
    _transitioned: bool,
}

impl OutputBlockParserFsmFrameContext {
    fn new(event: std::rc::Rc<OutputBlockParserFsmFrameEvent>, default_return: Option<OutputBlockParserFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum OutputBlockParserFsmStateContext {
    Init,
    Parsing,
    Empty,
}

impl Default for OutputBlockParserFsmStateContext {
    fn default() -> Self {
        OutputBlockParserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct OutputBlockParserFsmCompartment {
    state: String,
    state_context: OutputBlockParserFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<OutputBlockParserFsmFrameEvent>,
    parent_compartment: Option<Box<OutputBlockParserFsmCompartment>>,
}

impl OutputBlockParserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => OutputBlockParserFsmStateContext::Init,
            "Parsing" => OutputBlockParserFsmStateContext::Parsing,
            _ => OutputBlockParserFsmStateContext::Empty,
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
pub struct OutputBlockParserFsm {
    _state_stack: Vec<OutputBlockParserFsmCompartment>,
    __compartment: OutputBlockParserFsmCompartment,
    __next_compartment: Option<OutputBlockParserFsmCompartment>,
    _context_stack: Vec<OutputBlockParserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub mode: usize,
    pub token_kinds: Vec<usize>,
    pub token_starts: Vec<usize>,
    pub token_ends: Vec<usize>,
    pub result: String,
}

#[allow(non_snake_case)]
impl OutputBlockParserFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            mode: 1,
            token_kinds: Vec::new(),
            token_starts: Vec::new(),
            token_ends: Vec::new(),
            result: String::new(),
            __compartment: OutputBlockParserFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(OutputBlockParserFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = OutputBlockParserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        c._context_stack.push(__ctx);
        c.__kernel(&__e);
        c._context_stack.pop();
        c
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "Parsing" => &["Parsing"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> OutputBlockParserFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<OutputBlockParserFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = OutputBlockParserFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<OutputBlockParserFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(OutputBlockParserFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(OutputBlockParserFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, OutputBlockParserFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(OutputBlockParserFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<OutputBlockParserFsmFrameEvent>) {
        let __ev: &OutputBlockParserFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "Parsing" => self._state_Parsing(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: OutputBlockParserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_parse(&mut self) {
        let __e = std::rc::Rc::new(OutputBlockParserFsmFrameEvent::DoParse {});
        let mut __ctx = OutputBlockParserFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
        match __e {
            OutputBlockParserFsmFrameEvent::DoParse { .. } => { self._s_Init_hdl_user_do_parse(__e); }
            _ => {}
        }
    }

    fn _state_Parsing(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
        match __e {
            OutputBlockParserFsmFrameEvent::FrameEnter { .. } => { self._s_Parsing_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_parse(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("Parsing", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Parsing_hdl_frame_enter(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
        let bytes = &self.bytes;
        let n = self.token_kinds.len();
        let mut ti: usize = 0;
        let mut block_depth: i32 = 0;
        let mut after_return = false;
        
        while ti < n {
            let kind = self.token_kinds[ti];
            let start = self.token_starts[ti];
            let end = self.token_ends[ti];
            let text = String::from_utf8_lossy(&bytes[start..end]).to_string();
        
            // ---- RBRACE: check for else/elseif patterns ----
            if kind == 7 && block_depth > 0 {
                // Look ahead past whitespace/newline for ELSE
                let mut scan = ti + 1;
                while scan < n && (self.token_kinds[scan] == 10 || self.token_kinds[scan] == 11) {
                    let st = &bytes[self.token_starts[scan]..self.token_ends[scan]];
                    if st.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\n') {
                        scan += 1;
                    } else {
                        break;
                    }
                }
        
                if scan < n && self.token_kinds[scan] == 2 {
                    // RBRACE ... ELSEIF — look for condition then LBRACE
                    let elseif_ti = scan;
                    scan += 1;
                    let mut cond = String::new();
                    while scan < n && self.token_kinds[scan] != 6 && self.token_kinds[scan] != 10 {
                        let s = self.token_starts[scan];
                        let e = self.token_ends[scan];
                        cond.push_str(&String::from_utf8_lossy(&bytes[s..e]));
                        scan += 1;
                    }
                    if scan < n && self.token_kinds[scan] == 6 {
                        if self.mode == 1 {
                            self.result.push_str("elseif");
                            self.result.push_str(cond.trim_end());
                            self.result.push_str(" then");
                        }
                        ti = scan + 1;
                        after_return = false;
                        continue;
                    }
                }
        
                if scan < n && self.token_kinds[scan] == 3 {
                    // RBRACE ... ELSE — look for LBRACE after else
                    let else_ti = scan;
                    scan += 1;
                    while scan < n && (self.token_kinds[scan] == 10 || self.token_kinds[scan] == 11) {
                        let st = &bytes[self.token_starts[scan]..self.token_ends[scan]];
                        if st.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\n') { scan += 1; } else { break; }
                    }
        
                    if scan < n && self.token_kinds[scan] == 6 {
                        let lbrace_ti = scan;
                        // Check for IF after LBRACE (elseif pattern)
                        scan += 1;
                        while scan < n && (self.token_kinds[scan] == 10 || self.token_kinds[scan] == 11) {
                            let st = &bytes[self.token_starts[scan]..self.token_ends[scan]];
                            if st.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\n') { scan += 1; } else { break; }
                        }
        
                        if scan < n && self.token_kinds[scan] == 1 {
                            // RBRACE ELSE LBRACE IF ... LBRACE → elseif
                            let if_ti = scan;
                            // Collect condition tokens until next LBRACE
                            scan += 1;
                            let mut cond = String::new();
                            while scan < n && self.token_kinds[scan] != 6 {
                                let s = self.token_starts[scan];
                                let e = self.token_ends[scan];
                                cond.push_str(&String::from_utf8_lossy(&bytes[s..e]));
                                scan += 1;
                            }
                            if scan < n && self.token_kinds[scan] == 6 {
                                // Found the full elseif pattern
                                if self.mode == 1 {
                                    self.result.push_str("elseif");
                                    self.result.push_str(cond.trim_end());
                                    self.result.push_str(" then");
                                }
                                ti = scan + 1;
                                after_return = false;
                                continue;
                            }
                        }
        
                        // RBRACE ELSE LBRACE → else
                        if self.mode == 1 {
                            self.result.push_str("else");
                        }
                        ti = lbrace_ti + 1;
                        after_return = false;
                        continue;
                    }
                }
        
                // Plain RBRACE → end
                block_depth -= 1;
                if after_return { after_return = false; }
                if self.mode == 1 {
                    self.result.push_str("end");
                }
                ti += 1;
                continue;
            }
        
            // ---- IF: look for LBRACE pattern ----
            if kind == 1 && !after_return {
                // Collect tokens until LBRACE or NEWLINE
                let mut scan = ti + 1;
                let mut cond = String::new();
                let mut found_brace = false;
                while scan < n {
                    let sk = self.token_kinds[scan];
                    if sk == 6 {
                        // IF ... LBRACE
                        if self.mode == 1 {
                            self.result.push_str("if");
                            self.result.push_str(cond.trim_end());
                            self.result.push_str(" then");
                        }
                        block_depth += 1;
                        ti = scan + 1;
                        found_brace = true;
                        break;
                    }
                    if sk == 10 { break; } // Newline before brace — not our pattern
                    let s = self.token_starts[scan];
                    let e = self.token_ends[scan];
                    cond.push_str(&String::from_utf8_lossy(&bytes[s..e]));
                    scan += 1;
                }
                if found_brace { continue; }
                // Not our pattern — emit as-is
            }
        
            // ---- WHILE: look for LBRACE pattern ----
            if kind == 4 && !after_return {
                let mut scan = ti + 1;
                let mut cond = String::new();
                let mut found_brace = false;
                while scan < n {
                    let sk = self.token_kinds[scan];
                    if sk == 6 {
                        if self.mode == 1 {
                            self.result.push_str("while");
                            self.result.push_str(cond.trim_end());
                            self.result.push_str(" do");
                        }
                        block_depth += 1;
                        ti = scan + 1;
                        found_brace = true;
                        break;
                    }
                    if sk == 10 { break; }
                    let s = self.token_starts[scan];
                    let e = self.token_ends[scan];
                    cond.push_str(&String::from_utf8_lossy(&bytes[s..e]));
                    scan += 1;
                }
                if found_brace { continue; }
            }
        
            // ---- RETURN: emit the return keyword, then continue
            // emitting all subsequent tokens on the same line. We mark
            // `after_return` only after the newline so any expression
            // tokens that follow `return` (e.g. `return self.value`)
            // make it through. Without this, `return X` collapsed to
            // bare `return` and the X token was stripped — Lua tests
            // that returned a value silently returned nothing.
            if kind == 8 && !after_return {
                self.result.push_str(&text);
                // Walk subsequent tokens until newline or block close.
                let mut look = ti + 1;
                while look < n {
                    let lk = self.token_kinds[look];
                    if lk == 10 {
                        // Newline — emit it and now mark after_return.
                        self.result.push_str(
                            &String::from_utf8_lossy(
                                &self.bytes[self.token_starts[look]..self.token_ends[look]],
                            ),
                        );
                        look += 1;
                        break;
                    }
                    if lk == 7 || lk == 9 {
                        // RBRACE / END — let the outer loop handle it
                        // so block_depth is decremented correctly.
                        break;
                    }
                    // Anything else (text, whitespace, identifiers) —
                    // emit as-is so the return expression survives.
                    self.result.push_str(
                        &String::from_utf8_lossy(
                            &self.bytes[self.token_starts[look]..self.token_ends[look]],
                        ),
                    );
                    look += 1;
                }
                after_return = true;
                ti = look;
                continue;
            }
        
            // ---- After return: skip non-comment, non-structural tokens ----
            if after_return {
                // Allow through: COMMENT, NEWLINE (to preserve formatting),
                // RBRACE/END (block closers reset terminal state)
                if kind == 12 {
                    // Comment — emit
                    self.result.push_str(&text);
                    ti += 1;
                    continue;
                }
                if kind == 10 {
                    // Newline — emit (keeps line structure)
                    self.result.push_str(&text);
                    ti += 1;
                    continue;
                }
                if kind == 7 {
                    // RBRACE — end of block, reset after_return
                    after_return = false;
                    block_depth -= 1;
                    if self.mode == 1 { self.result.push_str("end"); }
                    ti += 1;
                    continue;
                }
                if kind == 9 || kind == 2 || kind == 3 {
                    // END/ELSEIF/ELSE — structural boundary, reset
                    after_return = false;
                    self.result.push_str(&text);
                    ti += 1;
                    continue;
                }
                // Skip everything else (unreachable code)
                ti += 1;
                continue;
            }
        
            // ---- Default: emit token text unchanged ----
            self.result.push_str(&text);
            ti += 1;
        }
    }
}
