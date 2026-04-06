
// Lua syntax skipper â Frame-generated state machine.
//
// Handles:
//   -- line comments
//   --[[ ]] block comments (with [=[ ]=] nesting)
//   "..." and '...' strings
//   [[...]] and [=[...]=] long strings
//
// Helpers used:
//   skip_simple_string (for "..." and '...')

#[allow(dead_code)]
struct LuaSyntaxSkipperFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for LuaSyntaxSkipperFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl LuaSyntaxSkipperFsmFrameEvent {
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
struct LuaSyntaxSkipperFsmFrameContext {
    event: LuaSyntaxSkipperFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl LuaSyntaxSkipperFsmFrameContext {
    fn new(event: LuaSyntaxSkipperFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone)]
enum LuaSyntaxSkipperFsmStateContext {
    Init,
    SkipComment,
    SkipString,
    FindLineEnd,
    BalancedParenEnd,
    Empty,
}

impl Default for LuaSyntaxSkipperFsmStateContext {
    fn default() -> Self {
        LuaSyntaxSkipperFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct LuaSyntaxSkipperFsmCompartment {
    state: String,
    state_context: LuaSyntaxSkipperFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<LuaSyntaxSkipperFsmFrameEvent>,
    parent_compartment: Option<Box<LuaSyntaxSkipperFsmCompartment>>,
}

impl LuaSyntaxSkipperFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => LuaSyntaxSkipperFsmStateContext::Init,
            "SkipComment" => LuaSyntaxSkipperFsmStateContext::SkipComment,
            "SkipString" => LuaSyntaxSkipperFsmStateContext::SkipString,
            "FindLineEnd" => LuaSyntaxSkipperFsmStateContext::FindLineEnd,
            "BalancedParenEnd" => LuaSyntaxSkipperFsmStateContext::BalancedParenEnd,
            _ => LuaSyntaxSkipperFsmStateContext::Empty,
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
pub struct LuaSyntaxSkipperFsm {
    _state_stack: Vec<LuaSyntaxSkipperFsmCompartment>,
    __compartment: LuaSyntaxSkipperFsmCompartment,
    __next_compartment: Option<LuaSyntaxSkipperFsmCompartment>,
    _context_stack: Vec<LuaSyntaxSkipperFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
}

#[allow(non_snake_case)]
impl LuaSyntaxSkipperFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 1,
            __compartment: LuaSyntaxSkipperFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = LuaSyntaxSkipperFsmFrameEvent::new("$>");
        let __ctx = LuaSyntaxSkipperFsmFrameContext::new(__frame_event, None);
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
            let exit_event = LuaSyntaxSkipperFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = LuaSyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = LuaSyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "SkipComment" => self._state_SkipComment(__e),
            "SkipString" => self._state_SkipString(__e),
            "FindLineEnd" => self._state_FindLineEnd(__e),
            "BalancedParenEnd" => self._state_BalancedParenEnd(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: LuaSyntaxSkipperFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: LuaSyntaxSkipperFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = LuaSyntaxSkipperFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = LuaSyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = LuaSyntaxSkipperFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn do_skip_comment(&mut self) {
        let mut __e = LuaSyntaxSkipperFsmFrameEvent::new("do_skip_comment");
        let mut __ctx = LuaSyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_skip_string(&mut self) {
        let mut __e = LuaSyntaxSkipperFsmFrameEvent::new("do_skip_string");
        let mut __ctx = LuaSyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_find_line_end(&mut self) {
        let mut __e = LuaSyntaxSkipperFsmFrameEvent::new("do_find_line_end");
        let mut __ctx = LuaSyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    pub fn do_balanced_paren_end(&mut self) {
        let mut __e = LuaSyntaxSkipperFsmFrameEvent::new("do_balanced_paren_end");
        let mut __ctx = LuaSyntaxSkipperFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "do_balanced_paren_end" => { self._s_Init_do_balanced_paren_end(__e); }
            "do_find_line_end" => { self._s_Init_do_find_line_end(__e); }
            "do_skip_comment" => { self._s_Init_do_skip_comment(__e); }
            "do_skip_string" => { self._s_Init_do_skip_string(__e); }
            _ => {}
        }
    }

    fn _state_SkipComment(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_SkipComment_enter(__e); }
            _ => {}
        }
    }

    fn _state_FindLineEnd(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_FindLineEnd_enter(__e); }
            _ => {}
        }
    }

    fn _state_BalancedParenEnd(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BalancedParenEnd_enter(__e); }
            _ => {}
        }
    }

    fn _state_SkipString(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_SkipString_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_do_skip_comment(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = LuaSyntaxSkipperFsmCompartment::new("SkipComment");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_find_line_end(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = LuaSyntaxSkipperFsmCompartment::new("FindLineEnd");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_balanced_paren_end(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = LuaSyntaxSkipperFsmCompartment::new("BalancedParenEnd");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Init_do_skip_string(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut __compartment = LuaSyntaxSkipperFsmCompartment::new("SkipString");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipComment_enter(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        // Must start with --
        if i + 1 >= end || bytes[i] != b'-' || bytes[i + 1] != b'-' {
            self.success = 0;
            return
        }
        
        // Check for block comment --[[ or --[=[
        if i + 3 < end && bytes[i + 2] == b'[' {
            // Count = signs
            let mut level = 0usize;
            let mut j = i + 3;
            while j < end && bytes[j] == b'=' {
                level += 1;
                j += 1;
            }
            if j < end && bytes[j] == b'[' {
                // Block comment: find matching ]=*]
                j += 1;
                while j < end {
                    if bytes[j] == b']' {
                        let mut k = j + 1;
                        let mut matched = 0usize;
                        while k < end && bytes[k] == b'=' && matched < level {
                            matched += 1;
                            k += 1;
                        }
                        if matched == level && k < end && bytes[k] == b']' {
                            self.result_pos = k + 1;
                            self.success = 1;
                            return
                        }
                    }
                    j += 1;
                }
                // Unterminated block comment
                self.result_pos = end;
                self.success = 1;
                return
            }
        }
        
        // Line comment: skip to end of line
        let mut j = i + 2;
        while j < end && bytes[j] != b'\n' {
            j += 1;
        }
        self.result_pos = j;
        self.success = 1;
    }

    fn _s_FindLineEnd_enter(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let mut j = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        let mut in_string: Option<u8> = None;
        
        while j < end {
            let b = bytes[j];
        
            if b == b'\n' { break; }
        
            if let Some(q) = in_string {
                if b == b'\\' && j + 1 < end {
                    j += 2;
                    continue;
                }
                if b == q { in_string = None; }
                j += 1;
                continue;
            }
        
            // Line comment
            if b == b'-' && j + 1 < end && bytes[j + 1] == b'-' { break; }
            if b == b'\'' || b == b'"' { in_string = Some(b); }
            j += 1;
        }
        self.result_pos = j;
    }

    fn _s_BalancedParenEnd_enter(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        if let Some(j) = balanced_paren_end_c_like(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        self.success = 0;
    }

    fn _s_SkipString_enter(&mut self, __e: &LuaSyntaxSkipperFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        // Long strings: [[ ]] or [=[ ]=]
        if bytes[i] == b'[' {
            let mut level = 0usize;
            let mut j = i + 1;
            while j < end && bytes[j] == b'=' {
                level += 1;
                j += 1;
            }
            if j < end && bytes[j] == b'[' {
                // Long string: find matching ]=*]
                j += 1;
                while j < end {
                    if bytes[j] == b']' {
                        let mut k = j + 1;
                        let mut matched = 0usize;
                        while k < end && bytes[k] == b'=' && matched < level {
                            matched += 1;
                            k += 1;
                        }
                        if matched == level && k < end && bytes[k] == b']' {
                            self.result_pos = k + 1;
                            self.success = 1;
                            return
                        }
                    }
                    j += 1;
                }
                self.result_pos = end;
                self.success = 1;
                return
            }
        }
        
        // Simple strings: "..." or '...'
        if let Some(j) = skip_simple_string(&self.bytes, self.pos, self.end) {
            self.result_pos = j;
            self.success = 1;
            return
        }
        
        self.success = 0;
    }
}
