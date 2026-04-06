
// Dogfooded body closer — Ruby language brace matcher.
// Ruby has:
//   - # line comments (like Python)
//   - "..." double-quoted strings with \ escapes and #{expr} interpolation
//   - '...' single-quoted strings (only \\ and \' escapes)
//   - { } braces for hashes, blocks
//
// State machine flow:
//   $Init.scan() → $Scanning.$>() ↔ $InString/$InLineComment

#[allow(dead_code)]
struct RubyBodyCloserFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for RubyBodyCloserFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl RubyBodyCloserFsmFrameEvent {
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
struct RubyBodyCloserFsmFrameContext {
    event: RubyBodyCloserFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl RubyBodyCloserFsmFrameContext {
    fn new(event: RubyBodyCloserFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone)]
enum RubyBodyCloserFsmStateContext {
    Init,
    Scanning,
    InString,
    InLineComment,
    Empty,
}

impl Default for RubyBodyCloserFsmStateContext {
    fn default() -> Self {
        RubyBodyCloserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct RubyBodyCloserFsmCompartment {
    state: String,
    state_context: RubyBodyCloserFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<RubyBodyCloserFsmFrameEvent>,
    parent_compartment: Option<Box<RubyBodyCloserFsmCompartment>>,
}

impl RubyBodyCloserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => RubyBodyCloserFsmStateContext::Init,
            "Scanning" => RubyBodyCloserFsmStateContext::Scanning,
            "InString" => RubyBodyCloserFsmStateContext::InString,
            "InLineComment" => RubyBodyCloserFsmStateContext::InLineComment,
            _ => RubyBodyCloserFsmStateContext::Empty,
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
pub struct RubyBodyCloserFsm {
    _state_stack: Vec<RubyBodyCloserFsmCompartment>,
    __compartment: RubyBodyCloserFsmCompartment,
    __next_compartment: Option<RubyBodyCloserFsmCompartment>,
    _context_stack: Vec<RubyBodyCloserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub depth: i32,
    pub result_pos: usize,
    pub error_kind: usize,
    pub error_msg: String,
    pub quote_char: u8,
}

#[allow(non_snake_case)]
impl RubyBodyCloserFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            pos: 0,
            depth: 1,
            result_pos: 0,
            error_kind: 0,
            error_msg: String::new(),
            quote_char: 0,
            __compartment: RubyBodyCloserFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = RubyBodyCloserFsmFrameEvent::new("$>");
        let __ctx = RubyBodyCloserFsmFrameContext::new(__frame_event, None);
        this._context_stack.push(__ctx);
        this.__kernel();
        this._context_stack.pop();
        this
    }

    fn __kernel(&mut self) {
        let __e = self._context_stack.last().unwrap().event.clone();
        self.__router(&__e);
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().unwrap();
            let exit_event = RubyBodyCloserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            self.__compartment = next_compartment;
            if self.__compartment.forward_event.is_none() {
                let enter_event = RubyBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    self.__router(&forward_event);
                } else {
                    let enter_event = RubyBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "Scanning" => self._state_Scanning(__e),
            "InString" => self._state_InString(__e),
            "InLineComment" => self._state_InLineComment(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: RubyBodyCloserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn scan(&mut self) {
        let mut __e = RubyBodyCloserFsmFrameEvent::new("scan");
        let mut __ctx = RubyBodyCloserFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "scan" => { self._s_Init_scan(__e); }
            _ => {}
        }
    }

    fn _state_InLineComment(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_InLineComment_enter(__e); }
            _ => {}
        }
    }

    fn _state_InString(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_InString_enter(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_Scanning_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_scan(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        let mut __compartment = RubyBodyCloserFsmCompartment::new("Scanning");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_InLineComment_enter(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
        let mut __compartment = RubyBodyCloserFsmCompartment::new("Scanning");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_InString_enter(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            if self.bytes[self.pos] == b'\\' {
                self.pos += 2;
                continue;
            }
            if self.bytes[self.pos] == self.quote_char {
                self.pos += 1;
                let mut __compartment = RubyBodyCloserFsmCompartment::new("Scanning");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            }
            self.pos += 1;
        }
        self.error_kind = 1;
        self.error_msg = "unterminated string".to_string();
    }

    fn _s_Scanning_enter(&mut self, __e: &RubyBodyCloserFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n {
            let b = self.bytes[self.pos];
            if b == b'\n' {
                self.pos += 1;
            } else if b == b'#' {
                self.pos += 1;
                let mut __compartment = RubyBodyCloserFsmCompartment::new("InLineComment");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            } else if b == b'\'' || b == b'"' {
                self.quote_char = b;
                self.pos += 1;
                let mut __compartment = RubyBodyCloserFsmCompartment::new("InString");
                __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
                self.__transition(__compartment);
                return;
            } else if b == b'{' {
                self.depth += 1;
                self.pos += 1;
            } else if b == b'}' {
                self.depth -= 1;
                self.pos += 1;
                if self.depth == 0 {
                    self.result_pos = self.pos - 1;
                    self.error_kind = 0;
                    return
                }
            } else {
                self.pos += 1;
            }
        }
        self.error_kind = 3;
        self.error_msg = "body not closed".to_string();
    }
}
