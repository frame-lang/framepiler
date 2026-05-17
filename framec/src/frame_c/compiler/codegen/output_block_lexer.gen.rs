
// Output Block Lexer — Frame state machine.
//
// EXHAUSTIVE tokenizer: every byte of input maps to exactly one token.
// No gaps — the parser can reconstruct complete output from the token stream.
//
// Token kinds:
//   1=IF, 2=ELSEIF, 3=ELSE, 4=WHILE, 5=FOR,
//   6=LBRACE, 7=RBRACE, 8=RETURN, 9=END,
//   10=NEWLINE, 11=TEXT, 12=COMMENT, 13=STRING
//
// Invariant: sum of (token_end - token_start) == input length.

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum OutputBlockLexerFsmFrameEvent {
    DoLex {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum OutputBlockLexerFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl OutputBlockLexerFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            OutputBlockLexerFsmFrameEvent::DoLex { .. } => "do_lex",
            OutputBlockLexerFsmFrameEvent::FrameEnter { .. } => "$>",
            OutputBlockLexerFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum OutputBlockLexerFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct OutputBlockLexerFsmFrameContext {
    event: std::rc::Rc<OutputBlockLexerFsmFrameEvent>,
    _return: Option<OutputBlockLexerFsmFrameReturn>,
    _data: std::collections::HashMap<String, OutputBlockLexerFsmFrameValue>,
    _transitioned: bool,
}

impl OutputBlockLexerFsmFrameContext {
    fn new(event: std::rc::Rc<OutputBlockLexerFsmFrameEvent>, default_return: Option<OutputBlockLexerFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum OutputBlockLexerFsmStateContext {
    Init,
    Scanning,
    Empty,
}

impl Default for OutputBlockLexerFsmStateContext {
    fn default() -> Self {
        OutputBlockLexerFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct OutputBlockLexerFsmCompartment {
    state: String,
    state_context: OutputBlockLexerFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<OutputBlockLexerFsmFrameEvent>,
    parent_compartment: Option<Box<OutputBlockLexerFsmCompartment>>,
}

impl OutputBlockLexerFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => OutputBlockLexerFsmStateContext::Init,
            "Scanning" => OutputBlockLexerFsmStateContext::Scanning,
            _ => OutputBlockLexerFsmStateContext::Empty,
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
pub struct OutputBlockLexerFsm {
    _state_stack: Vec<OutputBlockLexerFsmCompartment>,
    __compartment: OutputBlockLexerFsmCompartment,
    __next_compartment: Option<OutputBlockLexerFsmCompartment>,
    _context_stack: Vec<OutputBlockLexerFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub end: usize,
    pub comment_char: u8,
    pub comment_double: bool,
    pub token_kinds: Vec<usize>,
    pub token_starts: Vec<usize>,
    pub token_ends: Vec<usize>,
}

#[allow(non_snake_case)]
impl OutputBlockLexerFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            end: 0,
            comment_char: 0,
            comment_double: false,
            token_kinds: Vec::new(),
            token_starts: Vec::new(),
            token_ends: Vec::new(),
            __compartment: OutputBlockLexerFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(OutputBlockLexerFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = OutputBlockLexerFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        c._context_stack.push(__ctx);
        c.__kernel(&__e);
        c._context_stack.pop();
        c
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "Scanning" => &["Scanning"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> OutputBlockLexerFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<OutputBlockLexerFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = OutputBlockLexerFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<OutputBlockLexerFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(OutputBlockLexerFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(OutputBlockLexerFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, OutputBlockLexerFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(OutputBlockLexerFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<OutputBlockLexerFsmFrameEvent>) {
        let __ev: &OutputBlockLexerFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "Scanning" => self._state_Scanning(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: OutputBlockLexerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_lex(&mut self) {
        let __e = std::rc::Rc::new(OutputBlockLexerFsmFrameEvent::DoLex {});
        let mut __ctx = OutputBlockLexerFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
        match __e {
            OutputBlockLexerFsmFrameEvent::DoLex { .. } => { self._s_Init_hdl_user_do_lex(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
        match __e {
            OutputBlockLexerFsmFrameEvent::FrameEnter { .. } => { self._s_Scanning_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_lex(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("Scanning", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_Scanning_hdl_frame_enter(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
        let bytes = &self.bytes;
        let n = self.end;
        let mut i: usize = 0;
        let mut text_start: i64 = -1;
        
        while i < n {
            let b = bytes[i];
        
            // Comment detection (configured per-language)
            let is_comment = if self.comment_double {
                i + 1 < n && bytes[i] == self.comment_char && bytes[i + 1] == self.comment_char
            } else {
                self.comment_char != 0 && bytes[i] == self.comment_char
            };
        
            if is_comment {
                if text_start >= 0 {
                    self.token_kinds.push(11); self.token_starts.push(text_start as usize); self.token_ends.push(i);
                    text_start = -1;
                }
                let start = i;
                while i < n && bytes[i] != b'\n' { i += 1; }
                self.token_kinds.push(12); self.token_starts.push(start); self.token_ends.push(i);
                continue;
            }
        
            // String literals
            if b == b'"' || b == b'\'' {
                if text_start >= 0 {
                    self.token_kinds.push(11); self.token_starts.push(text_start as usize); self.token_ends.push(i);
                    text_start = -1;
                }
                let q = b;
                let start = i;
                i += 1;
                while i < n {
                    if bytes[i] == b'\\' && i + 1 < n { i += 2; continue; }
                    if bytes[i] == q { i += 1; break; }
                    i += 1;
                }
                self.token_kinds.push(13); self.token_starts.push(start); self.token_ends.push(i);
                continue;
            }
        
            // Newline
            if b == b'\n' {
                if text_start >= 0 {
                    self.token_kinds.push(11); self.token_starts.push(text_start as usize); self.token_ends.push(i);
                    text_start = -1;
                }
                self.token_kinds.push(10); self.token_starts.push(i); self.token_ends.push(i + 1);
                i += 1;
                continue;
            }
        
            // Braces
            if b == b'{' || b == b'}' {
                if text_start >= 0 {
                    self.token_kinds.push(11); self.token_starts.push(text_start as usize); self.token_ends.push(i);
                    text_start = -1;
                }
                let kind = if b == b'{' { 6 } else { 7 };
                self.token_kinds.push(kind); self.token_starts.push(i); self.token_ends.push(i + 1);
                i += 1;
                continue;
            }
        
            // Keyword detection
            if b.is_ascii_alphabetic() || b == b'_' {
                let at_boundary = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
                if at_boundary {
                    let ws = i;
                    let mut j = i;
                    while j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') { j += 1; }
                    let end_boundary = j >= n || !(bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_');
                    if end_boundary {
                        let word = &bytes[i..j];
                        let kind = if word == b"if" { 1 }
                            else if word == b"elseif" { 2 }
                            else if word == b"else" { 3 }
                            else if word == b"while" { 4 }
                            else if word == b"for" { 5 }
                            else if word == b"return" { 8 }
                            else if word == b"end" { 9 }
                            else { 0 };
                        if kind > 0 {
                            if text_start >= 0 {
                                self.token_kinds.push(11); self.token_starts.push(text_start as usize); self.token_ends.push(i);
                                text_start = -1;
                            }
                            self.token_kinds.push(kind); self.token_starts.push(ws); self.token_ends.push(j);
                            i = j;
                            continue;
                        }
                    }
                }
            }
        
            // Accumulate as TEXT
            if text_start < 0 { text_start = i as i64; }
            i += 1;
        }
        
        // Flush remaining text
        if text_start >= 0 {
            self.token_kinds.push(11); self.token_starts.push(text_start as usize); self.token_ends.push(n);
        }
    }
}
