
// Output Block Parser â Frame state machine.
//
// Consumes exhaustive token stream from OutputBlockLexer.
// Every token is either emitted as-is or transformed.
//
// Since the lexer covers every byte, the parser outputs exactly
// the same text if no transformations apply.
//
// Lua mode (mode=1):
//   IF TEXT LBRACE â "if" TEXT "then"
//   RBRACE ELSE LBRACE â "else"
//   RBRACE ELSE LBRACE IF TEXT LBRACE â "elseif" TEXT "then"
//   WHILE TEXT LBRACE â "while" TEXT "do"
//   RBRACE (block close) â "end"
//   RETURN â emit + mark terminal (skip subsequent non-comment tokens)

#[allow(dead_code)]
struct OutputBlockParserFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for OutputBlockParserFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl OutputBlockParserFsmFrameEvent {
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
struct OutputBlockParserFsmFrameContext {
    event: OutputBlockParserFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl OutputBlockParserFsmFrameContext {
    fn new(event: OutputBlockParserFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
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
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
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
            enter_args: std::collections::HashMap::new(),
            exit_args: std::collections::HashMap::new(),
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
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            mode: 1,
            token_kinds: Vec::new(),
            token_starts: Vec::new(),
            token_ends: Vec::new(),
            result: String::new(),
            __compartment: OutputBlockParserFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = OutputBlockParserFsmFrameEvent::new("$>");
        let __ctx = OutputBlockParserFsmFrameContext::new(__frame_event, None);
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
            let exit_event = OutputBlockParserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = OutputBlockParserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = OutputBlockParserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "Parsing" => self._state_Parsing(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: OutputBlockParserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: OutputBlockParserFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = OutputBlockParserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = OutputBlockParserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = OutputBlockParserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn do_parse(&mut self) {
        let mut __e = OutputBlockParserFsmFrameEvent::new("do_parse");
        let mut __ctx = OutputBlockParserFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Parsing(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_Parsing_enter(__e); }
            _ => {}
        }
    }

    fn _state_Init(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
        match __e.message.as_str() {
            "do_parse" => { self._s_Init_do_parse(__e); }
            _ => {}
        }
    }

    fn _s_Parsing_enter(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
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

    fn _s_Init_do_parse(&mut self, __e: &OutputBlockParserFsmFrameEvent) {
        let mut __compartment = OutputBlockParserFsmCompartment::new("Parsing");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }
}
