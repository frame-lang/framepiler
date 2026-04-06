
// Swift syntax skipper â Frame-generated state machine.
// Delegates to shared helpers; adds Swift multi-line strings """..."""
// and nestable block comments /* /* ... */ */
//
// Helpers used:
//   skip_line_comment, skip_simple_string,
//   find_line_end_c_like, balanced_paren_end_c_like
// Inline: """...""" multi-line strings (checked before skip_simple_string)
// Inline: nestable /* */ block comments

#[allow(dead_code)]
struct SwiftSyntaxSkipperFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for SwiftSyntaxSkipperFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl SwiftSyntaxSkipperFsmFrameEvent {
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
struct SwiftSyntaxSkipperFsmFrameContext {
    event: SwiftSyntaxSkipperFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl SwiftSyntaxSkipperFsmFrameContext {
    fn new(event: SwiftSyntaxSkipperFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone)]
enum SwiftSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for SwiftSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        SwiftSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct SwiftSyntaxSkipperFsmCompartment {
    state: String,
    state_context: SwiftSyntaxSkipperFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<SwiftSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<SwiftSyntaxSkipperFsmCompartment>>,
}

impl SwiftSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => SwiftSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => SwiftSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => SwiftSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => SwiftSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => SwiftSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => SwiftSyntaxSkipperFsmStateContext::Empty,
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
pub struct SwiftSyntaxSkipperFsm {
    _state_stack: Vec<SwiftSyntaxSkipperFsmCompartment>,
    __compartment: SwiftSyntaxSkipperFsmCompartment,
    __next_compartment: Option<SwiftSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<SwiftSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl SwiftSyntaxSkipperFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: SwiftSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = SwiftSyntaxSkipperFsmFrameEvent::new("$>");
        let __ctx = SwiftSyntaxSkipperFsmFrameContext::new(__frame_event, None);
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
            let exit_event = SwiftSyntaxSkipperFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = SwiftSyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = SwiftSyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "SkipComment" => self._state_SkipComment(__e),
            "SkipString" => self._state_SkipString(__e),
            "FindLineEnd" => self._state_FindLineEnd(__e),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: SwiftSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: SwiftSyntaxSkipperFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = SwiftSyntaxSkipperFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = SwiftSyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = SwiftSyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn do_skip_comment(&mut self) {
        let mut __e = SwiftSyntaxSkipperFsmFrameEvent::new("do_skip_comment");
        let mut __ctx = SwiftSyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let mut __e = SwiftSyntaxSkipperFsmFrameEvent::new("do_skip_string");
        let mut __ctx = SwiftSyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let mut __e = SwiftSyntaxSkipperFsmFrameEvent::new("do_find_line_end");
        let mut __ctx = SwiftSyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let mut __e = SwiftSyntaxSkipperFsmFrameEvent::new("do_balanced_paren_end");
        let mut __ctx = SwiftSyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_SkipComment(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_SkipComment_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_FindLineEnd_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_SkipString_enter(__e); }
            _ => {}
        }
    }

    fn _state_Init(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "do_balanced_paren_end" => { self._s_Init_do_balanced_paren_end(__e); }
            "do_find_line_end" => { self._s_Init_do_find_line_end(__e); }
            "do_skip_comment" => { self._s_Init_do_skip_comment(__e); }
            "do_skip_string" => { self._s_Init_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BalancedParenEnd_enter(__e); }
            _ => {}
        }
    }

    fn _s_SkipComment_enter(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = skip_line_comment(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        // Nestable block comments for Swift
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        if i + 1 < end && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let mut j = i + 2;
            let mut depth: i32 = 1;
            while j + 1 < end && depth > 0 {
                if bytes[j] == b'/' && bytes[j + 1] == b'*' {
                    depth += 1;
                    j += 2;
                } else if bytes[j] == b'*' && bytes[j + 1] == b'/' {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_FindLineEnd_enter(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        self.result_pos = find_line_end_c_like(&self.bytes, self.pos, self.end);
    }

    fn _s_SkipString_enter(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        // Swift multi-line string """...""" (must check before simple string)
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

    fn _s_Init_do_find_line_end(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = SwiftSyntaxSkipperFsmCompartment::new("FindLineEnd");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_balanced_paren_end(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = SwiftSyntaxSkipperFsmCompartment::new("BalancedParenEnd");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_skip_comment(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = SwiftSyntaxSkipperFsmCompartment::new("SkipComment");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_skip_string(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = SwiftSyntaxSkipperFsmCompartment::new("SkipString");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_BalancedParenEnd_enter(&mut self, __e: &SwiftSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = balanced_paren_end_c_like(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }
}

