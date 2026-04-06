
// Ruby syntax skipper â Frame-generated state machine.
// Ruby has # line comments, "..." / '...' strings,
// and %Q{} / %q[] / %w() / etc. percent literals.
//
// Helpers used:
//   skip_simple_string, skip_ruby_percent_literal
// Inline: skip_comment, find_line_end, balanced_paren_end

#[allow(dead_code)]
struct RubySyntaxSkipperFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for RubySyntaxSkipperFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl RubySyntaxSkipperFsmFrameEvent {
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
struct RubySyntaxSkipperFsmFrameContext {
    event: RubySyntaxSkipperFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl RubySyntaxSkipperFsmFrameContext {
    fn new(event: RubySyntaxSkipperFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone)]
enum RubySyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for RubySyntaxSkipperFsmStateContext {
    fn default() -> Self {
        RubySyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct RubySyntaxSkipperFsmCompartment {
    state: String,
    state_context: RubySyntaxSkipperFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<RubySyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<RubySyntaxSkipperFsmCompartment>>,
}

impl RubySyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => RubySyntaxSkipperFsmStateContext::Init,
            "SkipComment" => RubySyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => RubySyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => RubySyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => RubySyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => RubySyntaxSkipperFsmStateContext::Empty,
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
pub struct RubySyntaxSkipperFsm {
    _state_stack: Vec<RubySyntaxSkipperFsmCompartment>,
    __compartment: RubySyntaxSkipperFsmCompartment,
    __next_compartment: Option<RubySyntaxSkipperFsmCompartment>,
    _context_stack: Vec<RubySyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl RubySyntaxSkipperFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: RubySyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = RubySyntaxSkipperFsmFrameEvent::new("$>");
        let __ctx = RubySyntaxSkipperFsmFrameContext::new(__frame_event, None);
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
            let exit_event = RubySyntaxSkipperFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = RubySyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = RubySyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "SkipComment" => self._state_SkipComment(__e),
            "SkipString" => self._state_SkipString(__e),
            "FindLineEnd" => self._state_FindLineEnd(__e),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: RubySyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: RubySyntaxSkipperFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = RubySyntaxSkipperFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = RubySyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = RubySyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn do_skip_comment(&mut self) {
        let mut __e = RubySyntaxSkipperFsmFrameEvent::new("do_skip_comment");
        let mut __ctx = RubySyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let mut __e = RubySyntaxSkipperFsmFrameEvent::new("do_skip_string");
        let mut __ctx = RubySyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let mut __e = RubySyntaxSkipperFsmFrameEvent::new("do_find_line_end");
        let mut __ctx = RubySyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let mut __e = RubySyntaxSkipperFsmFrameEvent::new("do_balanced_paren_end");
        let mut __ctx = RubySyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_SkipString(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_SkipString_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_FindLineEnd_enter(__e); }
            _ => {}
        }
    }

    fn _state_Init(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "do_balanced_paren_end" => { self._s_Init_do_balanced_paren_end(__e); }
            "do_find_line_end" => { self._s_Init_do_find_line_end(__e); }
            "do_skip_comment" => { self._s_Init_do_skip_comment(__e); }
            "do_skip_string" => { self._s_Init_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_SkipComment_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BalancedParenEnd_enter(__e); }
            _ => {}
        }
    }

    fn _s_SkipString_enter(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        // Percent literals (must check before simple strings — % prefix)
        if let Some(j) = skip_ruby_percent_literal(&self.bytes, self.pos, self.end) {
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

    fn _s_FindLineEnd_enter(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
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

    fn _s_Init_do_skip_string(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        let mut __compartment = RubySyntaxSkipperFsmCompartment::new("SkipString");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_find_line_end(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        let mut __compartment = RubySyntaxSkipperFsmCompartment::new("FindLineEnd");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_skip_comment(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        let mut __compartment = RubySyntaxSkipperFsmCompartment::new("SkipComment");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_balanced_paren_end(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        let mut __compartment = RubySyntaxSkipperFsmCompartment::new("BalancedParenEnd");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_enter(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
        // Ruby: # line comment only
        if self.pos < self.end && self.bytes[self.pos] == b'#' {
            let mut j = self.pos + 1;
            while j < self.end && self.bytes[j] != b'\n' {
                j += 1;
            }
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_BalancedParenEnd_enter(&mut self, __e: &RubySyntaxSkipperFsmFrameEvent) {
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
                if depth == 0 { self.result_pos = i; self.success = 1; return }
            } else { i += 1; }
        }
        self.success = 0;
    }
}

