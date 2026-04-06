
// Dogfooded body closer â Lua language brace matcher.
// Lua uses -- for line comments, --[[ ]] for block comments,
// "..." and '...' for strings, and [[ ]] for long strings.
//
// State machine flow:
//   $Init.scan() â $Scanning.$>() â $InString/$InLineComment/$InBlockComment

#[allow(dead_code)]
struct LuaBodyCloserFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for LuaBodyCloserFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl LuaBodyCloserFsmFrameEvent {
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
struct LuaBodyCloserFsmFrameContext {
    event: LuaBodyCloserFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl LuaBodyCloserFsmFrameContext {
    fn new(event: LuaBodyCloserFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone)]
enum LuaBodyCloserFsmStateContext {
    Init,
    Scanning,
    InString,
    InLongString,
    InLineComment,
    InBlockComment,
    Empty,
}

impl Default for LuaBodyCloserFsmStateContext {
    fn default() -> Self {
        LuaBodyCloserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct LuaBodyCloserFsmCompartment {
    state: String,
    state_context: LuaBodyCloserFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<LuaBodyCloserFsmFrameEvent>,
    parent_compartment: Option<Box<LuaBodyCloserFsmCompartment>>,
}

impl LuaBodyCloserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => LuaBodyCloserFsmStateContext::Init,
            "Scanning" => LuaBodyCloserFsmStateContext::Scanning,
            "InString" => LuaBodyCloserFsmStateContext::InString,
            "InLongString" => LuaBodyCloserFsmStateContext::InLongString,
            "InLineComment" => LuaBodyCloserFsmStateContext::InLineComment,
            "InBlockComment" => LuaBodyCloserFsmStateContext::InBlockComment,
            _ => LuaBodyCloserFsmStateContext::Empty,
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
pub struct LuaBodyCloserFsm {
    _state_stack: Vec<LuaBodyCloserFsmCompartment>,
    __compartment: LuaBodyCloserFsmCompartment,
    __next_compartment: Option<LuaBodyCloserFsmCompartment>,
    _context_stack: Vec<LuaBodyCloserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub depth: i32,
    pub result: i32,
    pub string_char: u8,
}

#[allow(non_snake_case)]
impl LuaBodyCloserFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            pos: 0,
            depth: 1,
            result: -1,
            string_char: 0,
            __compartment: LuaBodyCloserFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = LuaBodyCloserFsmFrameEvent::new("$>");
        let __ctx = LuaBodyCloserFsmFrameContext::new(__frame_event, None);
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
            let exit_event = LuaBodyCloserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = LuaBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = LuaBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "Scanning" => self._state_Scanning(__e),
            "InString" => self._state_InString(__e),
            "InLongString" => self._state_InLongString(__e),
            "InLineComment" => self._state_InLineComment(__e),
            "InBlockComment" => self._state_InBlockComment(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: LuaBodyCloserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: LuaBodyCloserFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = LuaBodyCloserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = LuaBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = LuaBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn scan(&mut self) {
        let mut __e = LuaBodyCloserFsmFrameEvent::new("scan");
        let mut __ctx = LuaBodyCloserFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "scan" => { self._s_Init_scan(__e); }
            _ => {}
        }
    }

    fn _state_InLongString(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_InLongString_enter(__e); }
            _ => {}
        }
    }

    fn _state_InLineComment(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_InLineComment_enter(__e); }
            _ => {}
        }
    }

    fn _state_InString(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_InString_enter(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_Scanning_enter(__e); }
            _ => {}
        }
    }

    fn _state_InBlockComment(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_InBlockComment_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_scan(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        let mut __compartment = LuaBodyCloserFsmCompartment::new("Scanning");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_InLongString_enter(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos + 1 < n {
            if self.bytes[self.pos] == b']' && self.bytes[self.pos + 1] == b']' {
                self.pos += 2;
                let mut __compartment = LuaBodyCloserFsmCompartment::new("Scanning");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            }
            self.pos += 1;
        }
        self.result = -1;
    }

    fn _s_InLineComment_enter(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
        let mut __compartment = LuaBodyCloserFsmCompartment::new("Scanning");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_InString_enter(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            let b = self.bytes[self.pos];
            if b == b'\\' {
                self.pos += 2;
            } else if b == self.string_char {
                self.pos += 1;
                let mut __compartment = LuaBodyCloserFsmCompartment::new("Scanning");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            } else {
                self.pos += 1;
            }
        }
        self.result = -1;
    }

    fn _s_Scanning_enter(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            let b = self.bytes[self.pos];
            if b == b'\n' {
                self.pos += 1;
            } else if b == b'-' && self.pos + 1 < n && self.bytes[self.pos + 1] == b'-' {
                // Check for block comment --[[ ]]
                if self.pos + 3 < n && self.bytes[self.pos + 2] == b'[' && self.bytes[self.pos + 3] == b'[' {
                    self.pos += 4;
                    let mut __compartment = LuaBodyCloserFsmCompartment::new("InBlockComment");
                    __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                    self.__transition(__compartment);
                    return;
                }
                self.pos += 2;
                let mut __compartment = LuaBodyCloserFsmCompartment::new("InLineComment");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            } else if b == b'"' || b == b'\'' {
                self.string_char = b;
                self.pos += 1;
                let mut __compartment = LuaBodyCloserFsmCompartment::new("InString");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            } else if b == b'[' && self.pos + 1 < n && self.bytes[self.pos + 1] == b'[' {
                // Long string [[ ]]
                self.pos += 2;
                let mut __compartment = LuaBodyCloserFsmCompartment::new("InLongString");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            } else if b == b'{' {
                self.depth += 1;
                self.pos += 1;
            } else if b == b'}' {
                self.depth -= 1;
                if self.depth == 0 {
                    self.result = self.pos as i32;
                    return
                }
                self.pos += 1;
            } else {
                self.pos += 1;
            }
        }
        self.result = -1;
    }

    fn _s_InBlockComment_enter(&mut self, __e: &LuaBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos + 1 < n {
            if self.bytes[self.pos] == b']' && self.bytes[self.pos + 1] == b']' {
                self.pos += 2;
                let mut __compartment = LuaBodyCloserFsmCompartment::new("Scanning");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            }
            self.pos += 1;
        }
        self.result = -1;
    }
}
