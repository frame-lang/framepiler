
// Erlang scope scanner â detects `fun(...) -> ... end` closures.
//
// Erlang reuses `end` to close: fun, case, if, receive, begin, try.
// This FSM tracks a depth counter for all block-opening keywords so that
// only the `end` matching our `fun` terminates the scan.
//
// Skips % line comments and "..." / '...' string/atom literals.
//
// Usage: set bytes/pos/end, call do_scan(). On success, result_pos
// points to the byte after the matching `end`.
//
// Does NOT match `fun Module:Function/Arity` (function references â
// no closure scope, safe for Frame statements).

#[allow(dead_code)]
struct ErlangScopeScannerFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for ErlangScopeScannerFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl ErlangScopeScannerFsmFrameEvent {
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
struct ErlangScopeScannerFsmFrameContext {
    event: ErlangScopeScannerFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl ErlangScopeScannerFsmFrameContext {
    fn new(event: ErlangScopeScannerFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone)]
enum ErlangScopeScannerFsmStateContext {
    Init,
    CheckFun,
    ScanBody,
    Empty,
}

impl Default for ErlangScopeScannerFsmStateContext {
    fn default() -> Self {
        ErlangScopeScannerFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct ErlangScopeScannerFsmCompartment {
    state: String,
    state_context: ErlangScopeScannerFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<ErlangScopeScannerFsmFrameEvent>,
    parent_compartment: Option<Box<ErlangScopeScannerFsmCompartment>>,
}

impl ErlangScopeScannerFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => ErlangScopeScannerFsmStateContext::Init,
            "CheckFun" => ErlangScopeScannerFsmStateContext::CheckFun,
            "ScanBody" => ErlangScopeScannerFsmStateContext::ScanBody,
            _ => ErlangScopeScannerFsmStateContext::Empty,
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
pub struct ErlangScopeScannerFsm {
    _state_stack: Vec<ErlangScopeScannerFsmCompartment>,
    __compartment: ErlangScopeScannerFsmCompartment,
    __next_compartment: Option<ErlangScopeScannerFsmCompartment>,
    _context_stack: Vec<ErlangScopeScannerFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_pos: usize,
    pub success: usize,
    pub depth: i32,
}

#[allow(non_snake_case)]
impl ErlangScopeScannerFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 0,
            depth: 0,
            __compartment: ErlangScopeScannerFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = ErlangScopeScannerFsmFrameEvent::new("$>");
        let __ctx = ErlangScopeScannerFsmFrameContext::new(__frame_event, None);
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
            let exit_event = ErlangScopeScannerFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = ErlangScopeScannerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = ErlangScopeScannerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "CheckFun" => self._state_CheckFun(__e),
            "ScanBody" => self._state_ScanBody(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: ErlangScopeScannerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: ErlangScopeScannerFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = ErlangScopeScannerFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = ErlangScopeScannerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = ErlangScopeScannerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn do_scan(&mut self) {
        let mut __e = ErlangScopeScannerFsmFrameEvent::new("do_scan");
        let mut __ctx = ErlangScopeScannerFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_ScanBody(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ScanBody_enter(__e); }
            _ => {}
        }
    }

    fn _state_Init(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "do_scan" => { self._s_Init_do_scan(__e); }
            _ => {}
        }
    }

    fn _state_CheckFun(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_CheckFun_enter(__e); }
            _ => {}
        }
    }

    fn _s_ScanBody_enter(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        let mut i = self.pos;
        while i < self.end {
            let b = self.bytes[i];
        
            // Skip % line comments
            if b == b'%' {
                while i < self.end && self.bytes[i] != b'\n' {
                    i += 1;
                }
                continue
            }
        
            // Skip "..." strings
            if b == b'"' {
                i += 1;
                while i < self.end {
                    if self.bytes[i] == b'\\' { i += 2; continue }
                    if self.bytes[i] == b'"' { i += 1; break }
                    i += 1;
                }
                continue
            }
        
            // Skip '...' quoted atoms
            if b == b'\'' {
                i += 1;
                while i < self.end {
                    if self.bytes[i] == b'\\' { i += 2; continue }
                    if self.bytes[i] == b'\'' { i += 1; break }
                    i += 1;
                }
                continue
            }
        
            // Check for block-opening keywords (increase depth)
            // fun, case, if, receive, begin, try
            if b == b'f' && i + 3 <= self.end
                && self.bytes[i + 1] == b'u' && self.bytes[i + 2] == b'n'
                && (i + 3 >= self.end || !self.bytes[i + 3].is_ascii_alphanumeric() && self.bytes[i + 3] != b'_') {
                self.depth += 1;
                i += 3;
                continue
            }
            if b == b'c' && i + 4 <= self.end
                && self.bytes[i + 1] == b'a' && self.bytes[i + 2] == b's' && self.bytes[i + 3] == b'e'
                && (i + 4 >= self.end || !self.bytes[i + 4].is_ascii_alphanumeric() && self.bytes[i + 4] != b'_') {
                self.depth += 1;
                i += 4;
                continue
            }
            if b == b'i' && i + 2 <= self.end
                && self.bytes[i + 1] == b'f'
                && (i + 2 >= self.end || !self.bytes[i + 2].is_ascii_alphanumeric() && self.bytes[i + 2] != b'_') {
                self.depth += 1;
                i += 2;
                continue
            }
            if b == b'r' && i + 7 <= self.end
                && self.bytes[i + 1] == b'e' && self.bytes[i + 2] == b'c'
                && self.bytes[i + 3] == b'e' && self.bytes[i + 4] == b'i'
                && self.bytes[i + 5] == b'v' && self.bytes[i + 6] == b'e'
                && (i + 7 >= self.end || !self.bytes[i + 7].is_ascii_alphanumeric() && self.bytes[i + 7] != b'_') {
                self.depth += 1;
                i += 7;
                continue
            }
            if b == b'b' && i + 5 <= self.end
                && self.bytes[i + 1] == b'e' && self.bytes[i + 2] == b'g'
                && self.bytes[i + 3] == b'i' && self.bytes[i + 4] == b'n'
                && (i + 5 >= self.end || !self.bytes[i + 5].is_ascii_alphanumeric() && self.bytes[i + 5] != b'_') {
                self.depth += 1;
                i += 5;
                continue
            }
            if b == b't' && i + 3 <= self.end
                && self.bytes[i + 1] == b'r' && self.bytes[i + 2] == b'y'
                && (i + 3 >= self.end || !self.bytes[i + 3].is_ascii_alphanumeric() && self.bytes[i + 3] != b'_') {
                self.depth += 1;
                i += 3;
                continue
            }
        
            // Check for `end` keyword (decrease depth)
            if b == b'e' && i + 3 <= self.end
                && self.bytes[i + 1] == b'n' && self.bytes[i + 2] == b'd'
                && (i + 3 >= self.end || !self.bytes[i + 3].is_ascii_alphanumeric() && self.bytes[i + 3] != b'_') {
                self.depth -= 1;
                if self.depth == 0 {
                    // Found the matching `end` for our `fun`
                    self.result_pos = i + 3;
                    self.success = 1;
                    return
                }
                i += 3;
                continue
            }
        
            // Skip identifiers (avoid false keyword matches mid-word)
            if b.is_ascii_alphabetic() || b == b'_' {
                while i < self.end && (self.bytes[i].is_ascii_alphanumeric() || self.bytes[i] == b'_') {
                    i += 1;
                }
                continue
            }
        
            i += 1;
        }
        // Ran out of bytes without finding matching `end`
        self.success = 0;
    }

    fn _s_Init_do_scan(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        let mut __compartment = ErlangScopeScannerFsmCompartment::new("CheckFun");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_CheckFun_enter(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        // Must start with `fun` keyword followed by ( or whitespace
        if self.pos + 3 > self.end {
            self.success = 0;
            return
        }
        if self.bytes[self.pos] != b'f'
            || self.bytes[self.pos + 1] != b'u'
            || self.bytes[self.pos + 2] != b'n' {
            self.success = 0;
            return
        }
        // Check that `fun` is a word boundary (not part of `function` etc.)
        let after = self.pos + 3;
        if after < self.end && (self.bytes[after].is_ascii_alphanumeric() || self.bytes[after] == b'_') {
            self.success = 0;
            return
        }
        // Skip whitespace after `fun`
        let mut j = after;
        while j < self.end && (self.bytes[j] == b' ' || self.bytes[j] == b'\t' || self.bytes[j] == b'\n') {
            j += 1;
        }
        // Check for Module:Function/Arity pattern (function reference, not closure)
        // Function references have an uppercase letter or atom after `fun`
        // followed by `:` — e.g., `fun io:format/2`
        if j < self.end && self.bytes[j].is_ascii_uppercase() {
            // Could be a function reference — check for `:` after the module name
            let mut k = j;
            while k < self.end && (self.bytes[k].is_ascii_alphanumeric() || self.bytes[k] == b'_') {
                k += 1;
            }
            if k < self.end && self.bytes[k] == b':' {
                // This is `fun Module:Function/Arity` — not a closure
                self.success = 0;
                return
            }
        }
        // It's a closure: fun(...) -> ... end
        self.depth = 1;
        self.pos = after;
        let mut __compartment = ErlangScopeScannerFsmCompartment::new("ScanBody");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }
}
