
// Erlang body closer â Frame-generated FSM for brace matching.
// Erlang has % line comments, "..." strings, '...' quoted atoms.
// No block comments.

#[allow(dead_code)]
struct ErlangBodyCloserFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for ErlangBodyCloserFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl ErlangBodyCloserFsmFrameEvent {
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
struct ErlangBodyCloserFsmFrameContext {
    event: ErlangBodyCloserFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl ErlangBodyCloserFsmFrameContext {
    fn new(event: ErlangBodyCloserFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone)]
enum ErlangBodyCloserFsmStateContext {
    Init,
    Scanning,
    Empty,
}

impl Default for ErlangBodyCloserFsmStateContext {
    fn default() -> Self {
        ErlangBodyCloserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct ErlangBodyCloserFsmCompartment {
    state: String,
    state_context: ErlangBodyCloserFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<ErlangBodyCloserFsmFrameEvent>,
    parent_compartment: Option<Box<ErlangBodyCloserFsmCompartment>>,
}

impl ErlangBodyCloserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => ErlangBodyCloserFsmStateContext::Init,
            "Scanning" => ErlangBodyCloserFsmStateContext::Scanning,
            _ => ErlangBodyCloserFsmStateContext::Empty,
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
pub struct ErlangBodyCloserFsm {
    _state_stack: Vec<ErlangBodyCloserFsmCompartment>,
    __compartment: ErlangBodyCloserFsmCompartment,
    __next_compartment: Option<ErlangBodyCloserFsmCompartment>,
    _context_stack: Vec<ErlangBodyCloserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub depth: i32,
    pub success: usize,
    pub error_kind: usize,
}

#[allow(non_snake_case)]
impl ErlangBodyCloserFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            depth: 1,
            success: 1,
            error_kind: 0,
            __compartment: ErlangBodyCloserFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = ErlangBodyCloserFsmFrameEvent::new("$>");
        let __ctx = ErlangBodyCloserFsmFrameContext::new(__frame_event, None);
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
            let exit_event = ErlangBodyCloserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = ErlangBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = ErlangBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &ErlangBodyCloserFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "Scanning" => self._state_Scanning(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: ErlangBodyCloserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: ErlangBodyCloserFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = ErlangBodyCloserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = ErlangBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = ErlangBodyCloserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn scan(&mut self) {
        let mut __e = ErlangBodyCloserFsmFrameEvent::new("scan");
        let mut __ctx = ErlangBodyCloserFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Scanning(&mut self, __e: &ErlangBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_Scanning_enter(__e); }
            _ => {}
        }
    }

    fn _state_Init(&mut self, __e: &ErlangBodyCloserFsmFrameEvent) {
        match __e.message.as_str() {
            "scan" => { self._s_Init_scan(__e); }
            _ => {}
        }
    }

    fn _s_Scanning_enter(&mut self, __e: &ErlangBodyCloserFsmFrameEvent) {
        let bytes = &self.bytes;
        let end = self.end;
        let mut i = self.pos;
        let mut depth = self.depth;
        
        while i < end {
            let b = bytes[i];
            match b {
                b'{' => { depth += 1; i += 1; }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pos = i;
                        self.depth = 0;
                        self.success = 1;
                        return
                    }
                    i += 1;
                }
                b'%' => {
                    // Line comment — skip to newline
                    i += 1;
                    while i < end && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'"' => {
                    // String — skip to closing quote
                    i += 1;
                    while i < end {
                        if bytes[i] == b'\\' { i += 2; continue; }
                        if bytes[i] == b'"' { i += 1; break; }
                        i += 1;
                    }
                }
                b'\'' => {
                    // Quoted atom — skip to closing quote
                    i += 1;
                    while i < end {
                        if bytes[i] == b'\\' { i += 2; continue; }
                        if bytes[i] == b'\'' { i += 1; break; }
                        i += 1;
                    }
                }
                _ => { i += 1; }
            }
        }
        // Reached end without matching — unmatched brace
        self.pos = i;
        self.depth = depth;
        self.error_kind = 3;
        self.success = 0;
    }

    fn _s_Init_scan(&mut self, __e: &ErlangBodyCloserFsmFrameEvent) {
        let mut __compartment = ErlangBodyCloserFsmCompartment::new("Scanning");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }
}

