
// ContextParser â FSM for parsing all @@ context constructs.
//
// Dispatches on the character after @@ to parse:
//   @@:return [= expr] â ContextReturn (kind=2)
//   @@:event           â ContextEvent (kind=3)
//   @@:data.key [= e]  â ContextData (kind=4) or ContextDataAssign (kind=5)
//   @@:params.key      â ContextParams (kind=6)
//   @@SystemName()     â TaggedInstantiation (kind=7)
//   @@:(expr)          â ContextReturnExpr (kind=8)
//   @@:return(expr)    â ReturnCall (kind=9)
//   @@:self.method()   â ContextSelfCall (kind=10)
//   @@:self            â ContextSelf (kind=11)
//   @@:system.state    â ContextSystemState (kind=12)
//   other              â no match (has_result=false)
//
// Demonstrates hierarchical composition: $ParseReturn and $ParseData
// create ExprScannerFsm sub-machines when they detect assignment `=`.

include!("expr_scanner.gen.rs");

#[allow(dead_code)]
struct ContextParserFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for ContextParserFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl ContextParserFsmFrameEvent {
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
struct ContextParserFsmFrameContext {
    event: ContextParserFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
    _transitioned: bool,
}

impl ContextParserFsmFrameContext {
    fn new(event: ContextParserFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum ContextParserFsmStateContext {
    Init,
    Dispatching,
    DispatchColon,
    ParseReturn,
    ParseContextReturnExpr,
    ParseData,
    ParseParams,
    ParseSelf,
    ParseSystem,
    ParseInstantiation,
    Done,
    Empty,
}

impl Default for ContextParserFsmStateContext {
    fn default() -> Self {
        ContextParserFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct ContextParserFsmCompartment {
    state: String,
    state_context: ContextParserFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<ContextParserFsmFrameEvent>,
    parent_compartment: Option<Box<ContextParserFsmCompartment>>,
}

impl ContextParserFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => ContextParserFsmStateContext::Init,
            "Dispatching" => ContextParserFsmStateContext::Dispatching,
            "DispatchColon" => ContextParserFsmStateContext::DispatchColon,
            "ParseReturn" => ContextParserFsmStateContext::ParseReturn,
            "ParseContextReturnExpr" => ContextParserFsmStateContext::ParseContextReturnExpr,
            "ParseData" => ContextParserFsmStateContext::ParseData,
            "ParseParams" => ContextParserFsmStateContext::ParseParams,
            "ParseSelf" => ContextParserFsmStateContext::ParseSelf,
            "ParseSystem" => ContextParserFsmStateContext::ParseSystem,
            "ParseInstantiation" => ContextParserFsmStateContext::ParseInstantiation,
            "Done" => ContextParserFsmStateContext::Done,
            _ => ContextParserFsmStateContext::Empty,
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
pub struct ContextParserFsm {
    _state_stack: Vec<ContextParserFsmCompartment>,
    __compartment: ContextParserFsmCompartment,
    __next_compartment: Option<ContextParserFsmCompartment>,
    _context_stack: Vec<ContextParserFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub end: usize,
    pub result_end: usize,
    pub result_kind: usize,
    pub has_result: bool,
    pub paren_end: usize,
}

#[allow(non_snake_case)]
impl ContextParserFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_end: 0,
            result_kind: 0,
            has_result: false,
            paren_end: 0,
            __compartment: ContextParserFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = ContextParserFsmFrameEvent::new("$>");
        let __ctx = ContextParserFsmFrameContext::new(__frame_event, None);
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
            let exit_event = ContextParserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = ContextParserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = ContextParserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
            // Mark all stacked contexts as transitioned
            for ctx in self._context_stack.iter_mut() {
                ctx._transitioned = true;
            }
        }
    }

    fn __router(&mut self, __e: &ContextParserFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "Dispatching" => self._state_Dispatching(__e),
            "DispatchColon" => self._state_DispatchColon(__e),
            "ParseReturn" => self._state_ParseReturn(__e),
            "ParseContextReturnExpr" => self._state_ParseContextReturnExpr(__e),
            "ParseData" => self._state_ParseData(__e),
            "ParseParams" => self._state_ParseParams(__e),
            "ParseSelf" => self._state_ParseSelf(__e),
            "ParseSystem" => self._state_ParseSystem(__e),
            "ParseInstantiation" => self._state_ParseInstantiation(__e),
            "Done" => self._state_Done(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: ContextParserFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: ContextParserFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = ContextParserFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = ContextParserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = ContextParserFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn do_parse(&mut self) {
        let mut __e = ContextParserFsmFrameEvent::new("do_parse");
        let mut __ctx = ContextParserFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_ParseSelf(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ParseSelf_enter(__e); }
            _ => {}
        }
    }

    fn _state_ParseInstantiation(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ParseInstantiation_enter(__e); }
            _ => {}
        }
    }

    fn _state_Done(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_Done_enter(__e); }
            _ => {}
        }
    }

    fn _state_ParseContextReturnExpr(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ParseContextReturnExpr_enter(__e); }
            _ => {}
        }
    }

    fn _state_DispatchColon(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_DispatchColon_enter(__e); }
            _ => {}
        }
    }

    fn _state_ParseData(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ParseData_enter(__e); }
            _ => {}
        }
    }

    fn _state_Dispatching(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_Dispatching_enter(__e); }
            _ => {}
        }
    }

    fn _state_ParseParams(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ParseParams_enter(__e); }
            _ => {}
        }
    }

    fn _state_ParseReturn(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ParseReturn_enter(__e); }
            _ => {}
        }
    }

    fn _state_Init(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "do_parse" => { self._s_Init_do_parse(__e); }
            _ => {}
        }
    }

    fn _state_ParseSystem(&mut self, __e: &ContextParserFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ParseSystem_enter(__e); }
            _ => {}
        }
    }

    fn _s_ParseSelf_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // @@:self — bare reference or @@:self.method(args) call
        let mut i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        if i < end && bytes[i] == b'.' {
            i += 1; // Skip '.'
            // Scan identifier (method or property name)
            let name_start = i;
            while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i < end && bytes[i] == b'(' {
                // @@:self.method(args) — scan balanced parens
                let mut depth: usize = 1;
                i += 1; // Skip '('
                while i < end && depth > 0 {
                    if bytes[i] == b'(' { depth += 1; }
                    if bytes[i] == b')' { depth -= 1; }
                    if bytes[i] == b'"' || bytes[i] == b'\'' {
                        let q = bytes[i];
                        i += 1;
                        while i < end {
                            if bytes[i] == b'\\' && i + 1 < end { i += 2; continue; }
                            if bytes[i] == q { break; }
                            i += 1;
                        }
                    }
                    if depth > 0 { i += 1; }
                }
                if depth == 0 { i += 1; } // Skip closing ')'
                self.result_end = i;
                self.result_kind = 10; // ContextSelfCall
                self.has_result = true;
            } else {
                // @@:self.property — bare accessor
                self.result_end = i;
                self.result_kind = 11; // ContextSelf
                self.has_result = true;
            }
        } else {
            // bare @@:self
            self.result_end = i;
            self.result_kind = 11; // ContextSelf
            self.has_result = true;
        }
        
        let mut __compartment = ContextParserFsmCompartment::new("Done");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_ParseInstantiation_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // @@SystemName() — scan name, find balanced parens
        let mut i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        // Scan identifier
        while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        
        // Must be followed by (
        if i < end && bytes[i] == b'(' {
            // Use the pre-computed paren_end if available
            if self.paren_end > 0 {
                i = self.paren_end;
                self.result_end = i;
                self.result_kind = 7; // TaggedInstantiation
                self.has_result = true;
            } else {
                // No paren_end provided — caller must handle
                self.result_end = i;
                self.has_result = false;
            }
        } else {
            // @@SomeName without () — treat as native
            self.result_end = i;
            self.has_result = false;
        }
        
        let mut __compartment = ContextParserFsmCompartment::new("Done");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Done_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // Terminal state — results are in domain vars;
    }

    fn _s_ParseContextReturnExpr_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // @@:(expr) — scan balanced parens to find matching ')'
        let mut i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        if i < end && bytes[i] == b'(' {
            let mut depth: usize = 1;
            i += 1; // Skip opening '('
            while i < end && depth > 0 {
                let b = bytes[i];
                if b == b'(' {
                    depth += 1;
                } else if b == b')' {
                    depth -= 1;
                } else if b == b'"' || b == b'\'' {
                    // Skip string literals
                    let q = b;
                    i += 1;
                    while i < end {
                        if bytes[i] == b'\\' && i + 1 < end {
                            i += 2;
                            continue;
                        }
                        if bytes[i] == q {
                            break;
                        }
                        i += 1;
                    }
                }
                i += 1;
            }
        }
        
        self.result_end = i;
        self.result_kind = 8; // ContextReturnExpr
        self.has_result = true;
        let mut __compartment = ContextParserFsmCompartment::new("Done");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_DispatchColon_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // @@: — dispatch on the keyword after ':'
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        if i + 5 < end && &bytes[i..i + 6] == b"return" {
            self.pos = i + 6;
            let mut __compartment = ContextParserFsmCompartment::new("ParseReturn");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else if i + 4 < end && &bytes[i..i + 5] == b"event" {
            self.result_end = i + 5;
            self.result_kind = 3; // ContextEvent
            self.has_result = true;
            let mut __compartment = ContextParserFsmCompartment::new("Done");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else if i + 3 < end && &bytes[i..i + 4] == b"data" {
            self.pos = i + 4;
            let mut __compartment = ContextParserFsmCompartment::new("ParseData");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else if i + 5 < end && &bytes[i..i + 6] == b"params" {
            self.pos = i + 6;
            let mut __compartment = ContextParserFsmCompartment::new("ParseParams");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else if i + 3 < end && &bytes[i..i + 4] == b"self" {
            self.pos = i + 4;
            let mut __compartment = ContextParserFsmCompartment::new("ParseSelf");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else if i + 5 < end && &bytes[i..i + 6] == b"system" {
            self.pos = i + 6;
            let mut __compartment = ContextParserFsmCompartment::new("ParseSystem");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else if i < end && bytes[i] == b'(' {
            // @@:(expr) — context return expression
            self.pos = i;
            let mut __compartment = ContextParserFsmCompartment::new("ParseContextReturnExpr");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else {
            // Unknown @@: variant
            self.result_end = i;
            self.has_result = false;
            let mut __compartment = ContextParserFsmCompartment::new("Done");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        }
    }

    fn _s_ParseData_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // @@:data.key or @@:data.key = expr
        let mut i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        // Scan .key (dot + identifier)
        if i < end && bytes[i] == b'.' {
            i += 1; // Skip '.'
            while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        }
        
        // Check for assignment
        let mut j = i;
        while j < end && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        
        if j < end && bytes[j] == b'=' && (j + 1 >= end || bytes[j + 1] != b'=') {
            // @@:data[key] = expr — create ExprScanner sub-machine
            j += 1; // Skip '='
            let mut expr = ExprScannerFsm::new();
            expr.bytes = bytes.to_vec();
            expr.pos = j;
            expr.end = end;
            expr.do_scan();
            self.result_end = expr.result_end;
            // expr is destroyed here (state manager pattern)
            self.result_kind = 5; // ContextDataAssign
        } else {
            self.result_end = i;
            self.result_kind = 4; // ContextData
        }
        
        self.has_result = true;
        let mut __compartment = ContextParserFsmCompartment::new("Done");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Dispatching_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        if i >= end {
            self.has_result = false;
            let mut __compartment = ContextParserFsmCompartment::new("Done");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        }
        
        let b = bytes[i];
        
        if b == b':' {
            self.pos = i + 1;
            let mut __compartment = ContextParserFsmCompartment::new("DispatchColon");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else if b.is_ascii_uppercase() {
            // @@SystemName — pos stays at start of name
            let mut __compartment = ContextParserFsmCompartment::new("ParseInstantiation");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else {
            // Just @@ without . or : or uppercase
            self.result_end = i;
            self.has_result = false;
            let mut __compartment = ContextParserFsmCompartment::new("Done");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        }
    }

    fn _s_ParseParams_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // @@:params.key — dot-accessor for interface parameter
        let mut i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        if i < end && bytes[i] == b'.' {
            i += 1; // Skip '.'
            // Scan identifier
            while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        }
        
        self.result_end = i;
        self.result_kind = 6; // ContextParams
        self.has_result = true;
        let mut __compartment = ContextParserFsmCompartment::new("Done");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_ParseReturn_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // @@:return — check for assignment, call form, or bare read
        let mut i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        // Skip whitespace
        while i < end && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        
        if i < end && bytes[i] == b'(' {
            // @@:return(expr) — set return value AND exit handler.
            // Scan balanced parens to find matching ')'.
            let mut depth: usize = 1;
            i += 1; // Skip opening '('
            while i < end && depth > 0 {
                if bytes[i] == b'(' { depth += 1; }
                if bytes[i] == b')' { depth -= 1; }
                if depth > 0 { i += 1; }
            }
            if depth == 0 { i += 1; } // Skip closing ')'
            self.result_end = i;
            self.result_kind = 9; // ReturnCall
            self.has_result = true;
            let mut __compartment = ContextParserFsmCompartment::new("Done");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else if i < end && bytes[i] == b'=' && (i + 1 >= end || bytes[i + 1] != b'=') {
            // @@:return = <expr> — create ExprScanner sub-machine
            i += 1; // Skip '='
            let mut expr = ExprScannerFsm::new();
            expr.bytes = bytes.to_vec();
            expr.pos = i;
            expr.end = end;
            expr.do_scan();
            i = expr.result_end;
            // expr is destroyed here (state manager pattern)
            self.result_end = i;
            self.result_kind = 2; // ContextReturn
            self.has_result = true;
            let mut __compartment = ContextParserFsmCompartment::new("Done");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        } else {
            // @@:return (bare read) — rvalue access to return slot
            self.result_end = i;
            self.result_kind = 2; // ContextReturn (read mode)
            self.has_result = true;
            let mut __compartment = ContextParserFsmCompartment::new("Done");
            __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
            self.__transition(__compartment);
            return;
        }
    }

    fn _s_Init_do_parse(&mut self, __e: &ContextParserFsmFrameEvent) {
        let mut __compartment = ContextParserFsmCompartment::new("Dispatching");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_ParseSystem_enter(&mut self, __e: &ContextParserFsmFrameEvent) {
        // @@:system — currently only .state is supported
        let i = self.pos;
        let end = self.end;
        let bytes = &self.bytes;
        
        if i + 5 < end && &bytes[i..i + 6] == b".state"
            && (i + 6 >= end || !(bytes[i + 6].is_ascii_alphanumeric() || bytes[i + 6] == b'_'))
        {
            // @@:system.state — read-only state name accessor
            self.result_end = i + 6;
            self.result_kind = 12; // ContextSystemState
            self.has_result = true;
        } else {
            // Bare @@:system or unknown variant — emit for validation
            self.result_end = i;
            self.result_kind = 13; // ContextSystemBare
            self.has_result = true;
        }
        
        let mut __compartment = ContextParserFsmCompartment::new("Done");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }
}
