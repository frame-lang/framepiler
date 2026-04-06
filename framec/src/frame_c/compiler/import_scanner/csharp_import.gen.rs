
// C# import scanner â Frame-generated state machine.
// Scans for using directives and preprocessor #directives.
// Handles C# string types: "...", @"...", $"...", $@"...", """..."""
//
// Helpers used: starts_kw (from mod.rs)

#[allow(dead_code)]
struct CSharpImportScannerFsmFrameEvent {
    message: String,
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl Clone for CSharpImportScannerFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: std::collections::HashMap::new(),
        }
    }
}

impl CSharpImportScannerFsmFrameEvent {
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
struct CSharpImportScannerFsmFrameContext {
    event: CSharpImportScannerFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
}

impl CSharpImportScannerFsmFrameContext {
    fn new(event: CSharpImportScannerFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone)]
enum CSharpImportScannerFsmStateContext {
    Init,
    Scanning,
    Empty,
}

impl Default for CSharpImportScannerFsmStateContext {
    fn default() -> Self {
        CSharpImportScannerFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct CSharpImportScannerFsmCompartment {
    state: String,
    state_context: CSharpImportScannerFsmStateContext,
    enter_args: std::collections::HashMap<String, String>,
    exit_args: std::collections::HashMap<String, String>,
    forward_event: Option<CSharpImportScannerFsmFrameEvent>,
    parent_compartment: Option<Box<CSharpImportScannerFsmCompartment>>,
}

impl CSharpImportScannerFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => CSharpImportScannerFsmStateContext::Init,
            "Scanning" => CSharpImportScannerFsmStateContext::Scanning,
            _ => CSharpImportScannerFsmStateContext::Empty,
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
pub struct CSharpImportScannerFsm {
    _state_stack: Vec<CSharpImportScannerFsmCompartment>,
    __compartment: CSharpImportScannerFsmCompartment,
    __next_compartment: Option<CSharpImportScannerFsmCompartment>,
    _context_stack: Vec<CSharpImportScannerFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub start: usize,
    pub spans: Vec<RegionSpan>,
    pub issues: Vec<ValidationIssue>,
}

#[allow(non_snake_case)]
impl CSharpImportScannerFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: vec![],
            _context_stack: vec![],
            bytes: Vec::new(),
            start: 0,
            spans: Vec::new(),
            issues: Vec::new(),
            __compartment: CSharpImportScannerFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        let __frame_event = CSharpImportScannerFsmFrameEvent::new("$>");
        let __ctx = CSharpImportScannerFsmFrameContext::new(__frame_event, None);
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
            let exit_event = CSharpImportScannerFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
            self.__router(&exit_event);
            // Switch to new compartment
            self.__compartment = next_compartment;
            // Enter new state (or forward event)
            if self.__compartment.forward_event.is_none() {
                let enter_event = CSharpImportScannerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
            } else {
                // Forward event to new state
                let forward_event = self.__compartment.forward_event.take().unwrap();
                if forward_event.message == "$>" {
                    // Forwarding enter event - just send it
                    self.__router(&forward_event);
                } else {
                    // Forwarding other event - send $> first, then forward
                    let enter_event = CSharpImportScannerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                    self.__router(&enter_event);
                    self.__router(&forward_event);
                }
            }
        }
    }

    fn __router(&mut self, __e: &CSharpImportScannerFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "Scanning" => self._state_Scanning(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: CSharpImportScannerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    fn __push_transition(&mut self, new_compartment: CSharpImportScannerFsmCompartment) {
        // Exit current state (old compartment still in place for routing)
        let exit_event = CSharpImportScannerFsmFrameEvent::new_with_params("<$", &self.__compartment.exit_args);
        self.__router(&exit_event);
        // Swap: old compartment moves to stack, new takes its place
        let old = std::mem::replace(&mut self.__compartment, new_compartment);
        self._state_stack.push(old);
        // Enter new state (or forward event) — matches kernel logic
        if self.__compartment.forward_event.is_none() {
            let enter_event = CSharpImportScannerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
            self.__router(&enter_event);
        } else {
            let forward_event = self.__compartment.forward_event.take().unwrap();
            if forward_event.message == "$>" {
                self.__router(&forward_event);
            } else {
                let enter_event = CSharpImportScannerFsmFrameEvent::new_with_params("$>", &self.__compartment.enter_args);
                self.__router(&enter_event);
                self.__router(&forward_event);
            }
        }
    }

    pub fn do_scan(&mut self) {
        let mut __e = CSharpImportScannerFsmFrameEvent::new("do_scan");
        let mut __ctx = CSharpImportScannerFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &CSharpImportScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "do_scan" => { self._s_Init_do_scan(__e); }
            _ => {}
        }
    }

    fn _state_Scanning(&mut self, __e: &CSharpImportScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_Scanning_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_do_scan(&mut self, __e: &CSharpImportScannerFsmFrameEvent) {
        let mut __compartment = CSharpImportScannerFsmCompartment::new("Scanning");
        __compartment.parent_compartment = Some(Box::new(self.__compartment.clone()));
        self.__transition(__compartment);
        return;
    }

    fn _s_Scanning_enter(&mut self, __e: &CSharpImportScannerFsmFrameEvent) {
        let n = self.bytes.len();
        let mut i = self.start;
        let mut at_sol = true;
        let mut done = false;
        while i < n && !done {
            if at_sol {
                if self.bytes[i] == b'\n' || self.bytes[i] == b'\r' { i += 1; continue; }
                let line_start = i;
                let mut j = i;
                while j < n && (self.bytes[j] == b' ' || self.bytes[j] == b'\t') { j += 1; }
                // Preprocessor line
                if j < n && self.bytes[j] == b'#' {
                    let mut k = j;
                    while k < n && self.bytes[k] != b'\n' { k += 1; }
                    self.spans.push(RegionSpan { start: line_start, end: k });
                    i = if k < n { k + 1 } else { n };
                    at_sol = true;
                    continue;
                }
                // using directive
                if j < n && starts_kw(&self.bytes, j, b"using") {
                    let stmt_start = line_start;
                    let mut k = j + 5;
                    // in_s: 0=none, 1=regular", 2=verbatim@", 3=interp$", 4=interp-verbatim$@", 5=raw"""
                    let mut in_s: u8 = 0;
                    let mut raw_quotes: usize = 0;
                    let mut interp_brace: i32 = 0;
                    let mut found_semi = false;
                    while k < n {
                        let b = self.bytes[k];
                        match b {
                            b'/' if in_s == 0 && k + 1 < n && self.bytes[k + 1] == b'/' => {
                                k += 2; while k < n && self.bytes[k] != b'\n' { k += 1; }
                            }
                            b'/' if in_s == 0 && k + 1 < n && self.bytes[k + 1] == b'*' => {
                                k += 2;
                                while k + 1 < n { if self.bytes[k] == b'*' && self.bytes[k + 1] == b'/' { k += 2; break; } k += 1; }
                            }
                            b'@' if in_s == 0 && k + 1 < n && self.bytes[k + 1] == b'"' => { in_s = 2; k += 2; continue; }
                            b'$' if in_s == 0 && k + 1 < n && self.bytes[k + 1] == b'"' => { in_s = 3; k += 2; continue; }
                            b'$' if in_s == 0 && k + 2 < n && self.bytes[k + 1] == b'@' && self.bytes[k + 2] == b'"' => { in_s = 4; k += 3; continue; }
                            b'"' if in_s == 0 => {
                                let mut q = 1usize;
                                let mut p = k + 1;
                                while p < n && self.bytes[p] == b'"' { q += 1; p += 1; }
                                if q >= 3 { in_s = 5; raw_quotes = q; k = p; continue; }
                                else { in_s = 1; k += 1; continue; }
                            }
                            b'"' if in_s == 1 => { in_s = 0; k += 1; continue; }
                            b'\\' if in_s == 1 => { k += 2; continue; }
                            b'"' if in_s == 2 => {
                                if k + 1 < n && self.bytes[k + 1] == b'"' { k += 2; continue; }
                                else { in_s = 0; k += 1; continue; }
                            }
                            b'"' if in_s == 3 && interp_brace == 0 => { in_s = 0; k += 1; continue; }
                            b'{' if in_s == 3 || in_s == 4 => { interp_brace += 1; k += 1; continue; }
                            b'}' if (in_s == 3 || in_s == 4) && interp_brace > 0 => { interp_brace -= 1; k += 1; continue; }
                            b'"' if in_s == 4 => {
                                if k + 1 < n && self.bytes[k + 1] == b'"' { k += 2; continue; }
                                else if interp_brace == 0 { in_s = 0; k += 1; continue; }
                                else { k += 1; continue; }
                            }
                            b'"' if in_s == 5 => {
                                let mut q = 1usize;
                                let mut p = k + 1;
                                while p < n && self.bytes[p] == b'"' { q += 1; p += 1; }
                                if q >= raw_quotes { in_s = 0; k = p; continue; }
                                else { k = p; continue; }
                            }
                            b';' if in_s == 0 => {
                                self.spans.push(RegionSpan { start: stmt_start, end: k });
                                found_semi = true;
                                k += 1; i = k; at_sol = true; break;
                            }
                            b'\n' => { k += 1; }
                            _ => { k += 1; }
                        }
                    }
                    if found_semi { continue; }
                    // EOF without semicolon
                    if in_s != 0 || (in_s == 3 || in_s == 4) && interp_brace != 0 || in_s == 5 {
                        self.issues.push(ValidationIssue { message: "E110: unterminated using directive".into() });
                    }
                    self.spans.push(RegionSpan { start: stmt_start, end: n });
                    break;
                }
                // Not a preprocessor/using at SOL => imports block finished
                break;
            } else {
                if self.bytes[i] == b'\n' { at_sol = true; i += 1; } else { i += 1; }
            }
        }
    }
}

