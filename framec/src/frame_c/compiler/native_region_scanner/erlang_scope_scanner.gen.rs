
// Erlang scope scanner — detects `fun(...) -> ... end` closures.
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
// Does NOT match `fun Module:Function/Arity` (function references —
// no closure scope, safe for Frame statements).

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum ErlangScopeScannerFsmFrameEvent {
    DoScan {  },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum ErlangScopeScannerFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl ErlangScopeScannerFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            ErlangScopeScannerFsmFrameEvent::DoScan { .. } => "do_scan",
            ErlangScopeScannerFsmFrameEvent::FrameEnter { .. } => "$>",
            ErlangScopeScannerFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum ErlangScopeScannerFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct ErlangScopeScannerFsmFrameContext {
    event: std::rc::Rc<ErlangScopeScannerFsmFrameEvent>,
    _return: Option<ErlangScopeScannerFsmFrameReturn>,
    _data: std::collections::HashMap<String, ErlangScopeScannerFsmFrameValue>,
    _transitioned: bool,
}

impl ErlangScopeScannerFsmFrameContext {
    fn new(event: std::rc::Rc<ErlangScopeScannerFsmFrameEvent>, default_return: Option<ErlangScopeScannerFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
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
    enter_args: Vec<String>,
    exit_args: Vec<String>,
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
            enter_args: Vec::new(),
            exit_args: Vec::new(),
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
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            end: 0,
            result_pos: 0,
            success: 0,
            depth: 0,
            __compartment: ErlangScopeScannerFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(ErlangScopeScannerFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = ErlangScopeScannerFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        c._context_stack.push(__ctx);
        c.__kernel(&__e);
        c._context_stack.pop();
        c
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "CheckFun" => &["CheckFun"],
            "ScanBody" => &["ScanBody"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> ErlangScopeScannerFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<ErlangScopeScannerFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = ErlangScopeScannerFsmCompartment::new(name);
            new_comp.enter_args = enter_args.clone();
            if let Some(parent) = comp.take() {
                new_comp.parent_compartment = Some(Box::new(parent));
            }
            comp = Some(new_comp);
        }
        comp.expect("chain must contain at least the leaf state")
    }

    fn __prepareExit(&mut self, exit_args: Vec<String>) {
        self.__compartment.exit_args = exit_args.clone();
        let mut cursor = self.__compartment.parent_compartment.as_deref_mut();
        while let Some(c) = cursor {
            c.exit_args = exit_args.clone();
            cursor = c.parent_compartment.as_deref_mut();
        }
    }

    fn __kernel(&mut self, __e: &std::rc::Rc<ErlangScopeScannerFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(ErlangScopeScannerFsmFrameEvent::FrameExit { args: exit_args });
            self.__router(&exit_event);
            // Switch to the new compartment.
            self.__compartment = next_compartment;
            // Three-branch forward-event handling (RFC-0025 Track B.1: forward
            // event is matched on enum variant; $> recognition is now a
            // structural match, not a string compare).
            match self.__compartment.forward_event.take() {
                None => {
                    // No forwarded event — synthesize a fresh $>.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(ErlangScopeScannerFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, ErlangScopeScannerFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(ErlangScopeScannerFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
            }
            for ctx in self._context_stack.iter_mut() {
                ctx._transitioned = true;
            }
        }
    }

    fn __router(&mut self, __e: &std::rc::Rc<ErlangScopeScannerFsmFrameEvent>) {
        let __ev: &ErlangScopeScannerFsmFrameEvent = __e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "CheckFun" => self._state_CheckFun(__ev),
            "ScanBody" => self._state_ScanBody(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: ErlangScopeScannerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn do_scan(&mut self) {
        let __e = std::rc::Rc::new(ErlangScopeScannerFsmFrameEvent::DoScan {});
        let mut __ctx = ErlangScopeScannerFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        match __e {
            ErlangScopeScannerFsmFrameEvent::DoScan { .. } => { self._s_Init_hdl_user_do_scan(__e); }
            _ => {}
        }
    }

    fn _state_CheckFun(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        match __e {
            ErlangScopeScannerFsmFrameEvent::FrameEnter { .. } => { self._s_CheckFun_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _state_ScanBody(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        match __e {
            ErlangScopeScannerFsmFrameEvent::FrameEnter { .. } => { self._s_ScanBody_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_do_scan(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
        let mut __compartment = self.__prepareEnter("CheckFun", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_CheckFun_hdl_frame_enter(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
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
        let mut __compartment = self.__prepareEnter("ScanBody", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_ScanBody_hdl_frame_enter(&mut self, __e: &ErlangScopeScannerFsmFrameEvent) {
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
}
