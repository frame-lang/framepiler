
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

#[allow(dead_code)]
struct GDScriptMultiSysAssemblerFsmFrameEvent {
    message: String,
    parameters: Vec<Box<dyn std::any::Any>>,
}

impl Clone for GDScriptMultiSysAssemblerFsmFrameEvent {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            parameters: Vec::new(),
        }
    }
}

impl GDScriptMultiSysAssemblerFsmFrameEvent {
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
struct GDScriptMultiSysAssemblerFsmFrameContext {
    event: GDScriptMultiSysAssemblerFsmFrameEvent,
    _return: Option<Box<dyn std::any::Any>>,
    _data: std::collections::HashMap<String, Box<dyn std::any::Any>>,
    _transitioned: bool,
}

impl GDScriptMultiSysAssemblerFsmFrameContext {
    fn new(event: GDScriptMultiSysAssemblerFsmFrameEvent, default_return: Option<Box<dyn std::any::Any>>) -> Self {
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
        let mut this = Self {
            _state_stack: Vec::new(),
            _context_stack: Vec::new(),
            bytes: Vec::new(),
            pos: 0,
            system_name: String::new(),
            out: String::new(),
            __compartment: GDScriptMultiSysAssemblerFsmCompartment::new("Init"),
            __next_compartment: None,
        };
        this.__compartment = this.__prepareEnter("Init", vec![]);
        let __frame_event = GDScriptMultiSysAssemblerFsmFrameEvent::new_with_params("$>", &this.__compartment.enter_args);
        let __ctx = GDScriptMultiSysAssemblerFsmFrameContext::new(__frame_event, None);
        this._context_stack.push(__ctx);
        this.__fire_enter_cascade();
        this.__process_transition_loop();
        this._context_stack.pop();
        this
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

    fn __route_to_state(&mut self, state_name: &str, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match state_name {
            "Init" => self._state_Init(__e),
            "SkipLeading" => self._state_SkipLeading(__e),
            "ReadExtends" => self._state_ReadExtends(__e),
            "IndentBody" => self._state_IndentBody(__e),
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
            let exit_event = GDScriptMultiSysAssemblerFsmFrameEvent::new_with_params("<$", args);
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
            let enter_event = GDScriptMultiSysAssemblerFsmFrameEvent::new_with_params("$>", args);
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
            let _ = GDScriptMultiSysAssemblerFsmFrameEvent::new("$>");  // satisfy unused-import / type checks
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

    fn __router(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match self.__compartment.state.as_str() {
            "Init" => self._state_Init(__e),
            "SkipLeading" => self._state_SkipLeading(__e),
            "ReadExtends" => self._state_ReadExtends(__e),
            "IndentBody" => self._state_IndentBody(__e),
            _ => {}
        }
    }

    fn __transition(&mut self, next_compartment: GDScriptMultiSysAssemblerFsmCompartment) {
        self.__next_compartment = Some(next_compartment);
    }

    pub fn wrap_inner(&mut self, name: String) {
        let mut __e = GDScriptMultiSysAssemblerFsmFrameEvent::new("wrap_inner");
        __e.parameters = vec![Box::new(name.clone()) as Box<dyn std::any::Any>];
        let mut __ctx = GDScriptMultiSysAssemblerFsmFrameContext::new(__e, None);
        self._context_stack.push(__ctx);
        self.__kernel();
        self._context_stack.pop();
    }

    fn _state_Init(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match __e.message.as_str() {
            "wrap_inner" => {
                let __ctx_event = &self._context_stack.last().unwrap().event;
                let name: String = __ctx_event.parameters.get(0).and_then(|v| v.downcast_ref::<String>()).cloned().unwrap_or_default();
                self._s_Init_hdl_user_wrap_inner(__e, name);
            }
            _ => {}
        }
    }

    // Skip leading blank lines until first content. Frame's
    // codegen often emits a blank line before `extends`; the
    // wrapped output should start at the actual content.
    fn _state_SkipLeading(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_SkipLeading_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // Capture the `extends <Base>` line (or default to RefCounted
    // if absent — defensive; codegen always emits one for the
    // module-scope path that reaches this FSM). Emit the wrapper
    // header, then skip any blanks before the body.
    fn _state_ReadExtends(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_ReadExtends_hdl_frame_enter(__e); }
            _ => {}
        }
    }

    // Indent every remaining non-blank line by 4 spaces (one
    // level deeper than the wrapper class). Preserve blank lines
    // verbatim — the assembler relies on visual blank-line
    // separators to keep diffs readable.
    fn _state_IndentBody(&mut self, __e: &GDScriptMultiSysAssemblerFsmFrameEvent) {
        match __e.message.as_str() {
            "$>" => { self._s_IndentBody_hdl_frame_enter(__e); }
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
