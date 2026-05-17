
// GDScript multi-system assembler — Frame-generated state machine.
//
// Source:    multisys_assembler.frs (Frame specification)
// Generated: multisys_assembler.gen.rs (via framec --target rust)
//
// Wraps a sequence of per-system GDScript emissions into a valid
// multi-system .gd file. The first system stays at script-module
// scope (its `extends` is the script-level extends, its fields/funcs
// are script-level). Subsequent systems get wrapped as inner classes
// (`class <Name> extends <Base>:` with body indented one level).
//
// When the file holds 2+ systems, the first system also gets a
// `class_name <Name>` line so the inner-class systems can resolve
// cross-references via bare identifier (`var sub = First.new()`).
//
// To regenerate:
//   ./target/release/framec compile -l rust \
//     framec/src/frame_c/compiler/gdscript_multisys/multisys_assembler.frs \
//     -o framec/src/frame_c/compiler/gdscript_multisys/
//
// State machine flow:
//
//   $Init.wrap_inner() → $SkipLeading → $ReadExtends → $IndentBody → done
//
// `class_name` injection lives in the assembler's file-level post-
// pass (`prepend_gdscript_class_name`), not in this FSM — it touches
// the assembled output after both the native prolog and the first
// system have been written, and GDScript requires `class_name` to
// come before any `extends` in the file.
//
// Helpers used (from the wrapper module's helpers section):
//   line_is_blank, next_line_start, read_line_text, line_starts_with,
//   strip_extends_prefix

#[derive(Clone, Debug)]
#[allow(dead_code, non_camel_case_types)]
enum GDScriptMultiSysAssemblerFsmFrameEvent {
    WrapInner { name: String },
    FrameEnter { args: Vec<String> },
    FrameExit { args: Vec<String> },
}

#[derive(Clone)]
#[allow(dead_code, non_camel_case_types)]
enum GDScriptMultiSysAssemblerFsmFrameReturn {
    _Lifecycle(std::rc::Rc<dyn std::any::Any>),
}

#[allow(dead_code)]
impl GDScriptMultiSysAssemblerFsmFrameEvent {
    fn name(&self) -> &'static str {
        match self {
            GDScriptMultiSysAssemblerFsmFrameEvent::WrapInner { .. } => "wrap_inner",
            GDScriptMultiSysAssemblerFsmFrameEvent::FrameEnter { .. } => "$>",
            GDScriptMultiSysAssemblerFsmFrameEvent::FrameExit { .. } => "<$",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum GDScriptMultiSysAssemblerFsmFrameValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    List(Vec<Self>),
    Dict(std::collections::HashMap<String, Self>),
}

#[allow(dead_code)]
struct GDScriptMultiSysAssemblerFsmFrameContext {
    event: std::rc::Rc<GDScriptMultiSysAssemblerFsmFrameEvent>,
    _return: Option<GDScriptMultiSysAssemblerFsmFrameReturn>,
    _data: std::collections::HashMap<String, GDScriptMultiSysAssemblerFsmFrameValue>,
    _transitioned: bool,
}

impl GDScriptMultiSysAssemblerFsmFrameContext {
    fn new(event: std::rc::Rc<GDScriptMultiSysAssemblerFsmFrameEvent>, default_return: Option<GDScriptMultiSysAssemblerFsmFrameReturn>) -> Self {
        Self {
            event,
            _return: default_return,
            _data: std::collections::HashMap::new(),
            _transitioned: false,
        }
    }
}

#[derive(Clone)]
enum GDScriptMultiSysAssemblerFsmStateContext {
    Init,
    SkipLeading,
    ReadExtends,
    IndentBody,
    Empty,
}

impl Default for GDScriptMultiSysAssemblerFsmStateContext {
    fn default() -> Self {
        GDScriptMultiSysAssemblerFsmStateContext::Init
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct GDScriptMultiSysAssemblerFsmCompartment {
    state: String,
    state_context: GDScriptMultiSysAssemblerFsmStateContext,
    enter_args: Vec<String>,
    exit_args: Vec<String>,
    forward_event: Option<GDScriptMultiSysAssemblerFsmFrameEvent>,
    parent_compartment: Option<Box<GDScriptMultiSysAssemblerFsmCompartment>>,
}

impl GDScriptMultiSysAssemblerFsmCompartment {
    fn new(state: &str) -> Self {
        let state_context = match state {
            "Init" => GDScriptMultiSysAssemblerFsmStateContext::Init,
            "SkipLeading" => GDScriptMultiSysAssemblerFsmStateContext::SkipLeading,
            "ReadExtends" => GDScriptMultiSysAssemblerFsmStateContext::ReadExtends,
            "IndentBody" => GDScriptMultiSysAssemblerFsmStateContext::IndentBody,
            _ => GDScriptMultiSysAssemblerFsmStateContext::Empty,
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
pub struct GDScriptMultiSysAssemblerFsm {
    _state_stack: Vec<GDScriptMultiSysAssemblerFsmCompartment>,
    __compartment: GDScriptMultiSysAssemblerFsmCompartment,
    __next_compartment: Option<GDScriptMultiSysAssemblerFsmCompartment>,
    _context_stack: Vec<GDScriptMultiSysAssemblerFsmFrameContext>,
    pub bytes: Vec<u8>,
    pub pos: usize,
    pub system_name: String,
    pub out: String,
}

#[allow(non_snake_case)]
impl GDScriptMultiSysAssemblerFsm {
    pub fn new() -> Self {
        Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            system_name: String::new(),
            out: String::new(),
            __compartment: GDScriptMultiSysAssemblerFsmCompartment::new("Init"),
            __next_compartment: None,
        }
    }

    pub fn __create() -> Self {
        let mut c = Self::new();
        c.__compartment = c.__prepareEnter("Init", vec![]);
        let __e = std::rc::Rc::new(GDScriptMultiSysAssemblerFsmFrameEvent::FrameEnter { args: c.__compartment.enter_args.clone() });
        let __ctx = GDScriptMultiSysAssemblerFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        c._context_stack.push(__ctx);
        c.__kernel(&__e);
        c._context_stack.pop();
        c
    }

    fn __hsm_chain(&mut self, leaf: &str) -> &'static [&'static str] {
        match leaf {
            "Init" => &["Init"],
            "SkipLeading" => &["SkipLeading"],
            "ReadExtends" => &["ReadExtends"],
            "IndentBody" => &["IndentBody"],
            _ => &[],
        }
    }

    fn __prepareEnter(&mut self, leaf: &str, enter_args: Vec<String>) -> GDScriptMultiSysAssemblerFsmCompartment {
        let chain = self.__hsm_chain(leaf);
        let mut comp: Option<GDScriptMultiSysAssemblerFsmCompartment> = None;
        for name in chain.iter() {
            let mut new_comp = GDScriptMultiSysAssemblerFsmCompartment::new(name);
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

    fn __kernel(&mut self, __e: &std::rc::Rc<GDScriptMultiSysAssemblerFsmFrameEvent>) {
        // Route event to current state.
        self.__router(__e);
        // Drain any transitions queued by the handler.
        while self.__next_compartment.is_some() {
            let next_compartment = self.__next_compartment.take().expect("invariant: while-loop guard checked is_some()");
            // Exit the current (leaf) state.
            let exit_args = self.__compartment.exit_args.clone();
            let exit_event = std::rc::Rc::new(GDScriptMultiSysAssemblerFsmFrameEvent::FrameExit { args: exit_args });
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
                    let enter_event = std::rc::Rc::new(GDScriptMultiSysAssemblerFsmFrameEvent::FrameEnter { args: enter_args });
                    self.__router(&enter_event);
                }
                Some(fwd) if matches!(fwd, GDScriptMultiSysAssemblerFsmFrameEvent::FrameEnter { .. }) => {
                    // Forwarded event IS $> — dispatch directly so the
                    // destination's $> handler receives the caller's payload.
                    let fwd_rc = std::rc::Rc::new(fwd);
                    self.__router(&fwd_rc);
                }
                Some(fwd) => {
                    // Forwarded event is not $> — initialize the destination
                    // with a fresh $>, then dispatch the forward.
                    let enter_args = self.__compartment.enter_args.clone();
                    let enter_event = std::rc::Rc::new(GDScriptMultiSysAssemblerFsmFrameEvent::FrameEnter { args: enter_args });
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

    fn __router(&mut self, __e: &std::rc::Rc<GDScriptMultiSysAssemblerFsmFrameEvent>) {
        let __ev: &GDScriptMultiSysAssemblerFsmFrameEvent = &**__e;
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__ev),
            "SkipLeading" => self._state_SkipLeading(__ev),
            "ReadExtends" => self._state_ReadExtends(__ev),
            "IndentBody" => self._state_IndentBody(__ev),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: GDScriptMultiSysAssemblerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn wrap_inner(&mut self, name: String) {
        let __e = std::rc::Rc::new(GDScriptMultiSysAssemblerFsmFrameEvent::WrapInner { name: name.clone() });
        let mut __ctx = GDScriptMultiSysAssemblerFsmFrameContext::new(std::rc::Rc::clone(&__e), None);
        self._context_stack.push(__ctx);
        self.__kernel(&__e);
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match __e {
            GDScriptMultiSysAssemblerFsmFrameEvent::WrapInner { name, .. } => {
                self._s_Init_hdl_user_wrap_inner(__e, name.clone());
            }
            _ => {}
        }
    }

    // Skip leading blank lines until first content. Frame's
    // codegen often emits a blank line before `extends`; the
    // wrapped output should start at the actual content.
    fn _state_SkipLeading(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match __e {
            GDScriptMultiSysAssemblerFsmFrameEvent::FrameEnter { .. } => { self._s_SkipLeading_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // Capture the `extends <Base>` line (or default to RefCounted
    // if absent — defensive; codegen always emits one for the
    // module-scope path that reaches this FSM). Emit the wrapper
    // header, then skip any blanks before the body.
    fn _state_ReadExtends(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match __e {
            GDScriptMultiSysAssemblerFsmFrameEvent::FrameEnter { .. } => { self._s_ReadExtends_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // Indent every remaining non-blank line by 4 spaces (one
    // level deeper than the wrapper class). Preserve blank lines
    // verbatim — the assembler relies on visual blank-line
    // separators to keep diffs readable.
    fn _state_IndentBody(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match __e {
            GDScriptMultiSysAssemblerFsmFrameEvent::FrameEnter { .. } => { self._s_IndentBody_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    fn _s_Init_hdl_user_wrap_inner(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent, name: String) {
        self.system_name = name;
        let mut __compartment = self.__prepareEnter("SkipLeading", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_SkipLeading_hdl_frame_enter(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        while self.pos < self.bytes.len() && line_is_blank(&self.bytes, self.pos) {
            self.pos = next_line_start(&self.bytes, self.pos);
        }
        let mut __compartment = self.__prepareEnter("ReadExtends", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_ReadExtends_hdl_frame_enter(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        let n = self.bytes.len();
        let base = if self.pos < n && line_starts_with(&self.bytes, self.pos, b"extends ") {
            let line = read_line_text(&self.bytes, self.pos);
            self.pos = next_line_start(&self.bytes, self.pos);
            strip_extends_prefix(&line)
        } else {
            "RefCounted".to_string()
        };
        self.out.push_str("\n\n");
        self.out.push_str("class ");
        self.out.push_str(&self.system_name);
        self.out.push_str(" extends ");
        self.out.push_str(&base);
        self.out.push_str(":\n");
        while self.pos < self.bytes.len() && line_is_blank(&self.bytes, self.pos) {
            self.pos = next_line_start(&self.bytes, self.pos);
        }
        let mut __compartment = self.__prepareEnter("IndentBody", vec![]);
        self.__transition(__compartment);
        return;
    }

    fn _s_IndentBody_hdl_frame_enter(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        let n = self.bytes.len();
        let mut prev_blank: bool = true;
        while self.pos < n {
            let blank = line_is_blank(&self.bytes, self.pos);
            let line = read_line_text(&self.bytes, self.pos);
            self.pos = next_line_start(&self.bytes, self.pos);
            if blank {
                if !prev_blank {
                    self.out.push('\n');
                }
                prev_blank = true;
            } else {
                self.out.push_str("    ");
                self.out.push_str(&line);
                self.out.push('\n');
                prev_blank = false;
            }
        }
    }
}
