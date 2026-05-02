
// Attribute scanner — Frame-generated state machine.
//
// Source:    attribute_scanner.frs (Frame specification)
// Generated: attribute_scanner.gen.rs (via framec --target rust)
//
// Recognizes Frame's two pragma surface forms:
//
//   1. Bracket form (RFC-0013): `@@[name]` or `@@[name(args)]`
//      Examples: `@@[persist]`, `@@[target("python_3")]`, `@@[main]`,
//                `@@[save]`, `@@[load]`, `@@[no_persist]`,
//                `@@[migrate(from=1, to=2)]`
//
//   2. Bare form (legacy + still-supported keywords): `@@<name> <value?>`
//      Examples: `@@codegen { ... }`, `@@run-expect 42`, `@@skip-if windows`
//      RFC-0013 hard-cut: bare `@@persist` and `@@target` now error
//      (E803 / E804) — they pass through this scanner as Other and the
//      pipeline reports the migration error downstream.
//
// The scanner returns: name span (`name_start`/`name_end`), optional
// args span (bracket form only — `args_start`/`args_end`), optional
// value span (bare form only — `value_start`/`value_end` post-trim),
// and `is_bracket_form`. Translation from name string to `PragmaKind`
// happens in the wrapper module — keeps the FSM decoupled from the
// growing list of recognized attribute names.
//
// State machine flow:
//
//   $Init.scan(start) → $AfterAt
//                          ├→ $BracketName → $BracketAfterName
//                          │                    ├→ $BracketArgs → $BracketBeforeClose → done
//                          │                    └→ $BracketBeforeClose → done
//                          └→ $BareName → $BareValue → done
//
// Why FSM: bounded, single-pass byte walk over a small lookahead
// window. Same shape as `body_closer/*.frs` and `multisys_assembler.frs`.
// Replacing the existing Rust `identify_pragma` puts attribute parsing
// on the same Frame-native footing as the rest of the scanner work.

#[allow(dead_code)]
struct AttributeScannerFsmFrameEvent {
    message: String,
    parameters: Vec<Box<dyn std::any::Any>>,
}

impl Clone for AttributeScannerFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: Vec::new(),
        }
    }
}

impl AttributeScannerFsmFrameEvent {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
            parameters: Vec::new(),
        }
    }
    fn new_with_params(message: &str, params: &[String]) -> Self {
        Self {
            message: message.to_string(),
            parameters: params.iter().map(|v| Box::new(v.clone()) as Box<dyn std::any::Any>).collect(),
        }
    }
}

#[allow(dead_code)]
struct AttributeScannerFsmFrameContext {
    event: AttributeScannerFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
    _transitioned: bool,
}

impl AttributeScannerFsmFrameContext {
    fn new(event: AttributeScannerFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum AttributeScannerFsmStateContext {
    Init,
    BracketName,
    BracketAfterName,
    BracketArgs,
    BracketBeforeClose,
    BareName,
    BareValue,
    Empty,
}

impl Default for AttributeScannerFsmStateContext {
    fn default() -> Self {
        AttributeScannerFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct AttributeScannerFsmCompartment {
    state: String,
    state_context: AttributeScannerFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<AttributeScannerFsmFrameEvent>,
    parent_compartment: Option<Box<AttributeScannerFsmCompartment>>,
}

impl AttributeScannerFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => AttributeScannerFsmStateContext::Init,
            "BracketName" => AttributeScannerFsmStateContext::BracketName,
            "BracketAfterName" => AttributeScannerFsmStateContext::BracketAfterName,
            "BracketArgs" => AttributeScannerFsmStateContext::BracketArgs,
            "BracketBeforeClose" => AttributeScannerFsmStateContext::BracketBeforeClose,
            "BareName" => AttributeScannerFsmStateContext::BareName,
            "BareValue" => AttributeScannerFsmStateContext::BareValue,
            _ => AttributeScannerFsmStateContext::Empty,
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
pub struct AttributeScannerFsm {
    _state_stack: Vec<AttributeScannerFsmCompartment>,
    __compartment: AttributeScannerFsmCompartment,
    __next_compartment: Option<AttributeScannerFsmCompartment>,
    _context_stack: Vec<AttributeScannerFsmFrameContext>,
    // Input: the source bytes; the scan starting position is
    // passed via the `scan(start)` interface call.
    pub bytes: Vec<u8>,
    // Cursor — advances through the bytes during the walk;
    // accessible after the FSM finishes via `pos()`.
    pub pos: usize,
    // Surface form: true for `@@[name]`, false for bare `@@name`.
    pub is_bracket_form: bool,
    // Name span — always set after a successful scan.
    pub name_start: usize,
    pub name_end: usize,
    // Args span — only valid when `has_args` is true (bracket
    // form with `(...)`). The span covers the parens and their
    // contents, e.g. for `@@[target("python_3")]` it points at
    // `("python_3")` inclusive.
    pub has_args: bool,
    pub args_start: usize,
    pub args_end: usize,
    pub paren_depth: i32,
    // Value span — only valid when `has_value` is true (bare
    // form with non-empty trailing text). Points at the trimmed
    // rest-of-line.
    pub has_value: bool,
    pub value_start: usize,
    pub value_end: usize,
}

#[allow(non_snake_case)]
impl AttributeScannerFsm {
    pub fn new() -> Self {
        let mut this = Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            is_bracket_form: false,
            name_start: 0,
            name_end: 0,
            has_args: false,
            args_start: 0,
            args_end: 0,
            paren_depth: 0,
            has_value: false,
            value_start: 0,
            value_end: 0,
            __compartment: AttributeScannerFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        this.__compartment = this.__prepareEnter("Init", vec![]);
        let __frame_event = AttributeScannerFsmFrameEvent::new_with_params("$>", &this.__compartment.enter_args);
        let __ctx = AttributeScannerFsmFrameContext::new(__frame_event, None);
        this._context_stack.push(__ctx);
        this.__fire_enter_cascade();
        this.__process_transition_loop();
        this._context_stack.pop();
        this
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "BracketName" => &["BracketName"],
            "BracketAfterName" => &["BracketAfterName"],
            "BracketArgs" => &["BracketArgs"],
            "BracketBeforeClose" => &["BracketBeforeClose"],
            "BareName" => &["BareName"],
            "BareValue" => &["BareValue"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> AttributeScannerFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<AttributeScannerFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = AttributeScannerFsmCompartment::new(name);
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

    fn __route_to_state(&mut self, state_name: &str, __e: &AttributeScannerFsmFrameEvent) {
        match state_name {
            "Init" => self._state_Init(__e),
            "BracketName" => self._state_BracketName(__e),
            "BracketAfterName" => self._state_BracketAfterName(__e),
            "BracketArgs" => self._state_BracketArgs(__e),
            "BracketBeforeClose" => self._state_BracketBeforeClose(__e),
            "BareName" => self._state_BareName(__e),
            "BareValue" => self._state_BareValue(__e),
            _ => {}
        }
    }

    fn __fire_exit_cascade(&mut self) {
        let mut layers: Vec<(String, Vec<String>)> = Vec::new();
        {
            let mut cursor = Some(&self.__compartment);
            while let Some(c) = cursor {
                layers.push((c.state.clone(), c.exit_args.clone()));
                cursor = c.parent_compartment.as_deref();
            }
        }
        for (state_name, args) in &layers {
            let exit_event = AttributeScannerFsmFrameEvent::new_with_params("<$", args);
            self.__route_to_state(state_name, &exit_event);
        }
    }

    fn __fire_enter_cascade(&mut self) {
        let mut layers: Vec<(String, Vec<String>)> = Vec::new();
        {
            let mut cursor = Some(&self.__compartment);
            while let Some(c) = cursor {
                layers.push((c.state.clone(), c.enter_args.clone()));
                cursor = c.parent_compartment.as_deref();
            }
        }
        for (state_name, args) in layers.iter().rev() {
            let enter_event = AttributeScannerFsmFrameEvent::new_with_params("$>", args);
            self.__route_to_state(state_name, &enter_event);
        }
    }

    fn __process_transition_loop(&mut self) {
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().unwrap();
            self.__fire_exit_cascade();
            self.__compartment = next_compartment;
            if self.__compartment.forward_event.is_none() {
                self.__fire_enter_cascade();
            } else {
                let forward_event = self.__compartment.forward_event.take().unwrap();
                self.__fire_enter_cascade();
                if forward_event.message != "$>" {
                    self.__router(&forward_event);
                }
            }
            let _ = AttributeScannerFsmFrameEvent::new("$>");  // satisfy unused-import / type checks
            for ctx in self._context_stack.iter_mut() {
                ctx._transitioned = true;
            }
        }
    }

    fn __kernel(&mut self) {
        let __e = self._context_stack.last().unwrap().event.clone();
        self.__router(&__e);
        self.__process_transition_loop();
    }

    fn __router(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "BracketName" => self._state_BracketName(__e),
            "BracketAfterName" => self._state_BracketAfterName(__e),
            "BracketArgs" => self._state_BracketArgs(__e),
            "BracketBeforeClose" => self._state_BracketBeforeClose(__e),
            "BareName" => self._state_BareName(__e),
            "BareValue" => self._state_BareValue(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: AttributeScannerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn scan(&mut self, start: usize) {
        let mut __e = AttributeScannerFsmFrameEvent::new("scan");
        __e.parameters = vec![Box::new(start.clone()) as Box<dyn std::any::Any>];
        let mut __ctx = AttributeScannerFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "scan" => {
                let __ctx_event = &self._context_stack.last().unwrap().event;
                let start: usize = __ctx_event.parameters.get(0).and_then(|v| v.downcast_ref::<usize>()).cloned().unwrap_or_default();
                self._s_Init_hdl_user_scan(__e, start);
            }
            _ => {}
        }
    }

    // --- Bracket form: @@[name] / @@[name(args)] ----------------
    // Read the attribute name. Name chars: alphanumeric, `_`, `-`.
    // Stop at the first non-name char (`(`, `]`, ws).
    fn _state_BracketName(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BracketName_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // After the name: either `(args)` follows, or the closing `]`
    // (with possible intervening whitespace).
    fn _state_BracketAfterName(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BracketAfterName_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // Inside `(args)` — track paren depth so nested calls within
    // arg expressions don't terminate the args block early.
    // E.g. `@@[migrate(from=foo(1), to=2)]` has nested parens that
    // depth-tracking keeps as a single args span.
    fn _state_BracketArgs(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BracketArgs_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // Skip whitespace, expect closing `]`. Tolerates whitespace
    // between `)` and `]` (`@@[name(args) ]`) for source-style
    // flexibility.
    fn _state_BracketBeforeClose(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BracketBeforeClose_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // --- Bare form: @@<name> <value?> ---------------------------
    // Read the bare keyword name (same char class as bracket).
    fn _state_BareName(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BareName_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // Skip whitespace after the keyword, then read the rest of
    // the line as the value. Trim trailing ws / `\r` so CRLF
    // sources round-trip clean.
    fn _state_BareValue(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_BareValue_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_scan(&mut self, __e: &AttributeScannerFsmFrameEvent, start: usize) {
        self.pos = start + 2;  // skip @@
        if self.pos < self.bytes.len() && self.bytes[self.pos] == 0x5B {
            self.is_bracket_form = true;
            self.pos = self.pos + 1;
            let mut __compartment = self.__prepareEnter("BracketName", vec![]);
            self.__transition(__compartment);
            return;
        } else {
            self.is_bracket_form = false;
            let mut __compartment = self.__prepareEnter("BareName", vec![]);
            self.__transition(__compartment);
            return;
        }
    }

    fn _s_BracketName_hdl_frame_enter(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        self.name_start = self.pos;
        let n = self.bytes.len();
        while self.pos < n && is_attr_name_char(self.bytes[self.pos]) {
            self.pos = self.pos + 1;
        }
        self.name_end = self.pos;
        let mut __compartment = self.__prepareEnter("BracketAfterName", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_BracketAfterName_hdl_frame_enter(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        let n = self.bytes.len();
        if self.pos < n && self.bytes[self.pos] == 0x28 {
            self.has_args = true;
            self.args_start = self.pos;
            self.paren_depth = 1;
            self.pos = self.pos + 1;
            let mut __compartment = self.__prepareEnter("BracketArgs", vec![]);
            self.__transition(__compartment);
            return;
        } else {
            let mut __compartment = self.__prepareEnter("BracketBeforeClose", vec![]);
            self.__transition(__compartment);
            return;
        }
    }

    fn _s_BracketArgs_hdl_frame_enter(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && self.paren_depth > 0 {
            let b = self.bytes[self.pos];
            if b == 0x28 {
                self.paren_depth = self.paren_depth + 1;
            } else if b == 0x29 {
                self.paren_depth = self.paren_depth - 1;
            }
            self.pos = self.pos + 1;
        }
        self.args_end = self.pos;
        let mut __compartment = self.__prepareEnter("BracketBeforeClose", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_BracketBeforeClose_hdl_frame_enter(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && (self.bytes[self.pos] == 0x20 || self.bytes[self.pos] == 0x09) {
            self.pos = self.pos + 1;
        }
        if self.pos < n && self.bytes[self.pos] == 0x5D {
            self.pos = self.pos + 1;
        }
    }

    fn _s_BareName_hdl_frame_enter(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        self.name_start = self.pos;
        let n = self.bytes.len();
        while self.pos < n && is_attr_name_char(self.bytes[self.pos]) {
            self.pos = self.pos + 1;
        }
        self.name_end = self.pos;
        let mut __compartment = self.__prepareEnter("BareValue", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_BareValue_hdl_frame_enter(&mut self, __e: &AttributeScannerFsmFrameEvent) {
        let n = self.bytes.len();
        while self.pos < n && (self.bytes[self.pos] == 0x20 || self.bytes[self.pos] == 0x09) {
            self.pos = self.pos + 1;
        }
        let vs = self.pos;
        while self.pos < n && self.bytes[self.pos] != 0x0A {
            self.pos = self.pos + 1;
        }
        let mut ve = self.pos;
        while ve > vs && (
            self.bytes[ve - 1] == 0x20 ||
            self.bytes[ve - 1] == 0x09 ||
            self.bytes[ve - 1] == 0x0D
        ) {
            ve = ve - 1;
        }
        if ve > vs {
            self.has_value = true;
            self.value_start = vs;
            self.value_end = ve;
        }
    }
}
