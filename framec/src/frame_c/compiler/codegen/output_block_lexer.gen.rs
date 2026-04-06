
// Output Block Lexer â Frame state machine.
//
// EXHAUSTIVE tokenizer: every byte of input maps to exactly one token.
// No gaps â the parser can reconstruct complete output from the token stream.
//
// Token kinds:
//   1=IF, 2=ELSEIF, 3=ELSE, 4=WHILE, 5=FOR,
//   6=LBRACE, 7=RBRACE, 8=RETURN, 9=END,
//   10=NEWLINE, 11=TEXT, 12=COMMENT, 13=STRING
//
// Invariant: sum of (token_end - token_start) == input length.

#[allow(dead_code)]
struct OutputBlockLexerFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for OutputBlockLexerFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl OutputBlockLexerFsmFrameEvent {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
            parameters: std::collections::HashMap::new(),
        }
    }
    fn new_with_params(message: &str, params: &std::collections::HashMap<String, String>) -> Self {
        Self {
            message: message.to_string(),
            parameters: params.iter().map(|(k, v)| (k.clone(), Box::new(v.clone()) as Box<dyn std::any::Any>)).collect(),
        }
    }
}

#[allow(dead_code)]
struct OutputBlockLexerFsmFrameContext {
    event: OutputBlockLexerFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl OutputBlockLexerFsmFrameContext {
    fn new(event: OutputBlockLexerFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
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
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
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
            enter_args: std::collections::HashMap::new(),
            exit_args: std::collections::HashMap::new(),
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
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            end: 0,
            comment_char: 0,
            comment_double: false,
            token_kinds: Vec::new(),
            token_starts: Vec::new(),
            token_ends: Vec::new(),
            __compartment: OutputBlockLexerFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = OutputBlockLexerFsmFrameEvent::new("$>");
        let __ctx = OutputBlockLexerFsmFrameContext::new(__frame_event, None);
        this._context_stack.push(__ctx);
        this.__kernel();
        this._context_stack.pop();
        this
    }

    fn __kernel(&mut self) {
        // Clone event from context stack (needed for borrow checker)
        let __e = self._context_stack.last().unwrap().event.clone();
        // Route event to current state
        self.__router(&__e);
        // Process any pending transition
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().unwrap();
            // Exit current state (with exit_args from current compartment)
            let exit_event = OutputBlockLexerFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = OutputBlockLexerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = OutputBlockLexerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "Scanning" => self._state_Scanning(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: OutputBlockLexerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: OutputBlockLexerFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = OutputBlockLexerFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = OutputBlockLexerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = OutputBlockLexerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn do_lex(&mut self) {
        let mut __e = OutputBlockLexerFsmFrameEvent::new("do_lex");
        let mut __ctx = OutputBlockLexerFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Scanning(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_Scanning_enter(__e); }
            _ => {}
        }
    }

    fn _state_Init(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
        match __e.message.as_str() {
            "do_lex" => { self._s_Init_do_lex(__e); }
            _ => {}
        }
    }

    fn _s_Scanning_enter(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
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

    fn _s_Init_do_lex(&mut self, __e: &OutputBlockLexerFsmFrameEvent) {
        let mut __compartment = OutputBlockLexerFsmCompartment::new("Scanning");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }
}
