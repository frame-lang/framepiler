//! System constructor emission across all backends.
//!
//! `generate_constructor` is the per-target system-class
//! constructor emitter — the largest single function in the
//! codegen tree before this split. It initializes the two
//! kernel stacks (`_state_stack`, `_context_stack`), threads
//! system params into the compartment, runs user-authored
//! domain field initializers, and (when the language requires
//! it) sets up the framework's `__compartment` and
//! `__frame_init` cascade trigger.
//!
//! Three internal helpers move with it:
//! - `init_collection_stack` — emits the per-language empty
//!   stack initializer (with C#/Go element-type formatting).
//! - `should_emit_constructor_body_init` / `uses_constructor_body_init`
//!   — gate logic for languages that init via constructor body
//!   vs. those that init via field defaults.
//! - `format_field_assignment` — small per-target field=value
//!   shape formatter.

use super::super::ast::{CodegenNode, Field, Param, Visibility};
use super::super::backend::ClassSyntax;
use super::super::codegen_utils::{
    cpp_map_type, csharp_map_type, go_map_type, java_map_type, kotlin_map_type, swift_map_type,
    state_var_init_value, to_snake_case, type_to_cpp_string, type_to_string,
    expression_to_string,
};
use crate::frame_c::compiler::frame_ast::{
    Expression, ParamKind, StateAst, SystemAst, Type,
};
use crate::frame_c::visitors::TargetLanguage;

/// Element type carried by one of the kernel's two stacks. C# and Go
/// format the element type into the init expression; every other language
/// either uses a default-constructed empty container or doesn't care
/// about the element type at all.
enum StackElementKind {
    /// `_state_stack` — Compartment values; pointer-typed in Go.
    Compartment,
    /// `_context_stack` — FrameContext values; never pointer-typed.
    FrameContext,
}

/// Emit the per-language initializer for a kernel stack field
/// (`_state_stack` or `_context_stack`). Returns `None` for languages
/// where no init is needed (C++: vectors default-construct empty;
/// Graphviz: unreachable in this code path).
fn init_collection_stack(
    field_name: &str,
    element_kind: StackElementKind,
    system: &SystemAst,
    syntax: &ClassSyntax,
) -> Option<CodegenNode> {
    match syntax.language {
        TargetLanguage::C => Some(CodegenNode::assign(
            CodegenNode::field(CodegenNode::self_ref(), field_name),
            CodegenNode::Ident(format!("{}_FrameVec_new()", system.name)),
        )),
        // C++ vectors default-construct as empty; no init needed.
        TargetLanguage::Cpp => None,
        TargetLanguage::Java => Some(CodegenNode::NativeBlock {
            code: format!("{} = new java.util.ArrayList<>();", field_name),
            span: None,
        }),
        TargetLanguage::Kotlin => Some(CodegenNode::NativeBlock {
            code: format!("{} = mutableListOf()", field_name),
            span: None,
        }),
        TargetLanguage::Swift => Some(CodegenNode::NativeBlock {
            code: format!("{} = []", field_name),
            span: None,
        }),
        TargetLanguage::CSharp => {
            let elem = match element_kind {
                StackElementKind::Compartment => format!("{}Compartment", system.name),
                StackElementKind::FrameContext => format!("{}FrameContext", system.name),
            };
            Some(CodegenNode::NativeBlock {
                code: format!("{} = new List<{}>();", field_name, elem),
                span: None,
            })
        }
        TargetLanguage::Go => {
            // Go state-stack stores Compartment pointers; context-stack stores values.
            let elem = match element_kind {
                StackElementKind::Compartment => format!("*{}Compartment", system.name),
                StackElementKind::FrameContext => format!("{}FrameContext", system.name),
            };
            Some(CodegenNode::NativeBlock {
                code: format!("s.{} = make([]{}, 0)", field_name, elem),
                span: None,
            })
        }
        // Dynamic / array-literal languages.
        TargetLanguage::Python3
        | TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Rust
        | TargetLanguage::Lua
        | TargetLanguage::Dart
        | TargetLanguage::GDScript => Some(CodegenNode::assign(
            CodegenNode::field(CodegenNode::self_ref(), field_name),
            CodegenNode::Array(vec![]),
        )),
        TargetLanguage::Graphviz => unreachable!(),
    }
}

/// Decide whether the constructor body should emit a domain-field init
/// statement, given the language and the field's properties.
///
/// Three groups:
/// - **Always emit** (C, Go, Python, Ruby, Lua, Rust): no field-level
///   init is available (C/Go) or none is generated (dynamic langs go
///   straight to the constructor body).
/// - **Emit only on collision** (Cpp, Java, Swift, C#, Dart, GDScript,
///   TS, JS, PHP): the field has a literal init at declaration scope,
///   except when that init references a system param — then the init
///   moves into the constructor body to avoid the name-collision.
/// - **Kotlin**: same as the OO group. (Const fields whose init refers
///   to a system param are emitted as `var` at the Kotlin level so the
///   constructor-body assignment compiles — see `build_system_fields`.)
/// - **Erlang / Graphviz**: never go through this code path.
fn should_emit_constructor_body_init(
    lang: TargetLanguage,
    _is_const: bool,
    init_refs_param: bool,
    init_has_tagged: bool,
) -> bool {
    use TargetLanguage::*;
    match lang {
        C | Go | Python3 | Ruby | Lua | Rust => true,
        // PHP rejects non-const class-field defaults at parse time, so
        // any `@@<System>()` initializer has to move to the constructor
        // body — same flag the field-emission path uses to strip the
        // inline init.
        Php => init_refs_param || init_has_tagged,
        Cpp | Java | Swift | CSharp | Dart | GDScript | TypeScript | JavaScript | Kotlin => {
            init_refs_param
        }
        // (Kotlin previously skipped const fields here on the assumption
        //  they'd be primary-constructor params; RFC-0017 made the system
        //  ctor parameterless, so the const field must take its value via
        //  the constructor-body assignment like every other backend.)
        Erlang | Graphviz => false,
    }
}

/// True when the target emits domain fields as constructor-body
/// assignments (so leading comments belong inline with the body) vs
/// as class-level field declarations (so leading comments belong on
/// the per-backend `Field` IR's `emit_field` path). Rust is in the
/// constructor-body camp because its emission uses
/// `CodegenNode::assign`, but framec also emits Rust struct field
/// declarations whose `Field` IR carries the comments — the
/// duplication is acceptable for now since Rust's struct-decl path
/// is the user-visible one.
fn uses_constructor_body_init(lang: TargetLanguage) -> bool {
    use TargetLanguage::*;
    matches!(lang, Python3 | Ruby | Lua | Php | GDScript | Go | C | Rust)
}

/// Format `field = init_value` using the per-language self-access form
/// and statement terminator. Used to build the constructor body's
/// domain-field init lines for every language EXCEPT Rust (which uses
/// the structured `CodegenNode::assign` instead) and the C++ const-init
/// case (which uses a member initializer list).
///
/// `field_type` is the user-declared type from the Frame source (an
/// opaque string per Frame's "no type system" rule). Most language
/// arms ignore it; **C** uses it to disambiguate brace-initialized
/// arrays/structs (`{0}`, `{1, 2}`) from scalar inits — array
/// assignment is illegal in C, so a brace init has to be emitted as
/// a `memcpy` from a compound literal rather than a plain `=`.
fn format_field_assignment(
    lang: TargetLanguage,
    field_name: &str,
    init_value: &str,
    field_type: &str,
) -> String {
    use TargetLanguage::*;
    match lang {
        C => {
            // C arrays aren't assignable: `arr = {0};` is a syntax
            // error even though `<Type> arr = {0};` is fine in
            // declaration position. Detect brace-initializers and
            // emit the equivalent via a typed compound literal +
            // memcpy. Works for arrays AND structs; scalar inits
            // (the common case) keep the simple `=` form.
            if init_value.trim_start().starts_with('{') {
                format!(
                    "{{ {ty} __init_{name} = {init}; \
                     memcpy(&self->{name}, &__init_{name}, sizeof(self->{name})); }}",
                    ty = field_type,
                    name = field_name,
                    init = init_value,
                )
            } else {
                format!("self->{} = {};", field_name, init_value)
            }
        }
        Cpp => format!("this->{} = {};", field_name, init_value),
        Go => format!("s.{} = {}", field_name, init_value),
        Java | CSharp | Dart | TypeScript | JavaScript => {
            format!("this.{} = {};", field_name, init_value)
        }
        Swift | GDScript | Python3 | Lua => format!("self.{} = {}", field_name, init_value),
        Kotlin => format!("this.{} = {}", field_name, init_value),
        Php => format!("$this->{} = {};", field_name, init_value),
        Ruby => format!("@{} = {}", field_name, init_value),
        Rust | Erlang | Graphviz => unreachable!(
            "format_field_assignment called for {:?} (Rust uses structured assign; \
             Erlang/Graphviz never reach this code path)",
            lang
        ),
    }
}

/// Generate the constructor
pub(crate) fn generate_constructor(
    system: &SystemAst,
    syntax: &ClassSyntax,
) -> CodegenNode {
    let mut body = Vec::new();
    // C++ member initializer list entries: `field(value), field2(value2)`
    // Collected during domain init loop for const fields whose init was stripped.
    let mut cpp_init_list: Vec<String> = Vec::new();

    // Initialize state stack and context stack — same shape per language,
    // only the field name and element-type formatting differ.
    if let Some(node) = init_collection_stack(
        "_state_stack",
        StackElementKind::Compartment,
        system,
        syntax,
    ) {
        body.push(node);
    }
    if let Some(node) = init_collection_stack(
        "_context_stack",
        StackElementKind::FrameContext,
        system,
        syntax,
    ) {
        body.push(node);
    }

    // Initialize domain variables.
    //
    // Three concerns interleave per language:
    //   1. WHETHER to emit a body init at all (see
    //      `should_emit_constructor_body_init`).
    //   2. HOW to spell `self.field = value` for the target
    //      (see `format_field_assignment`).
    //   3. Per-language adjustments to the init expression itself
    //      (PHP `$`-prefix on param refs, Lua `[]` → `{}`, Rust
    //      Domain-param override, C++ const → member init list).
    //
    // Rust is the structural odd-one-out: it uses `CodegenNode::assign`
    // instead of a `NativeBlock` and must always init every field
    // (including the no-init case, handled up front).
    let sys_param_names_for_init: Vec<String> =
        system.params.iter().map(|p| p.name.clone()).collect();
    for domain_var in &system.domain {
        // Emit any source-level leading comments first, so they
        // sit immediately above the field-assignment line in the
        // generated constructor. Skipped for languages that emit
        // domain fields as class-level declarations (Cpp, Java,
        // CSharp, Swift, Kotlin, TypeScript, JavaScript, Dart) —
        // there the comments live on the `Field` IR's
        // `leading_comments` and the per-backend `emit_field`
        // prepends them. The constructor-body path (Python, Ruby,
        // Lua, PHP, GDScript, Go, C, Rust) does not have an
        // emit_field hook for domain fields, so the comments
        // attach here.
        if !domain_var.leading_comments.is_empty() && uses_constructor_body_init(syntax.language) {
            for comment in &domain_var.leading_comments {
                body.push(CodegenNode::NativeBlock {
                    code: comment.clone(),
                    span: None,
                });
            }
        }
        // Rust requires all fields initialized; handle the no-init
        // case up front before the regular path.
        if matches!(syntax.language, TargetLanguage::Rust) && domain_var.initializer_text.is_none()
        {
            let rust_init = if system.params.iter().any(|p| {
                p.name == domain_var.name
                    && matches!(
                        p.kind,
                        crate::frame_c::compiler::frame_ast::ParamKind::Domain
                    )
            }) {
                domain_var.name.clone()
            } else {
                "Default::default()".to_string()
            };
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                CodegenNode::Ident(rust_init),
            ));
            continue;
        }

        let init_text = match &domain_var.initializer_text {
            Some(t) => t,
            None => continue,
        };
        let init_refs_param = super::word_util::init_references_param(init_text, &sys_param_names_for_init);
        let init_has_tagged = init_text.contains("@@");

        if !should_emit_constructor_body_init(
            syntax.language,
            domain_var.is_const,
            init_refs_param,
            init_has_tagged,
        ) {
            continue;
        }

        let init_expanded = super::expand_system::expand_system_instantiation_in_domain(init_text, syntax.language);

        // C++ const fields whose init references a constructor param
        // must use the member initializer list — `const T x;` cannot
        // be reassigned in the body. The list is rendered before the
        // constructor body as `: x(value)`.
        if matches!(syntax.language, TargetLanguage::Cpp) && domain_var.is_const {
            cpp_init_list.push(format!("{}({})", domain_var.name, init_expanded));
            continue;
        }

        // Per-language init-expression adjustments.
        let final_init = match syntax.language {
            // PHP rejects bare names in init expressions; system params
            // need a `$` prefix to be valid PHP.
            TargetLanguage::Php => super::word_util::prefix_php_vars(&init_expanded, &sys_param_names_for_init),
            // Lua uses `{}` for empty tables, not `[]`.
            TargetLanguage::Lua if init_expanded.trim() == "[]" => "{}".to_string(),
            // Rust: a Domain-kind system param of the same name overrides
            // the literal default.
            TargetLanguage::Rust
                if system.params.iter().any(|p| {
                    p.name == domain_var.name
                        && matches!(
                            p.kind,
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain
                        )
                }) =>
            {
                domain_var.name.clone()
            }
            _ => init_expanded,
        };

        if matches!(syntax.language, TargetLanguage::Rust) {
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                CodegenNode::Ident(final_init),
            ));
        } else {
            // The user-declared type is opaque (`Type::Custom(..)` for
            // most cases, `Type::Unknown` for bare-form fields in
            // dynamic targets). Render to a string so the C arm can
            // recognize array/struct types when the init is a brace
            // initializer.
            let type_str = match &domain_var.var_type {
                crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
                _ => String::new(),
            };
            body.push(CodegenNode::NativeBlock {
                code: format_field_assignment(
                    syntax.language,
                    &domain_var.name,
                    &final_init,
                    &type_str,
                ),
                span: None,
            });
        }
    }

    // Domain-kind system params override domain field defaults.
    // The domain init uses the literal default (e.g., `self.inventory = {}`).
    // We must then assign the constructor arg: `self.inventory = inventory`.
    // Note: the param name always matches the domain field name for Domain params.
    for p in &system.params {
        if matches!(
            p.kind,
            crate::frame_c::compiler::frame_ast::ParamKind::Domain
        ) {
            // Check that this param name actually matches a domain field
            let matching_field = system.domain.iter().find(|d| d.name == p.name);
            if matching_field.is_none() {
                continue; // Skip — no matching domain field
            }
            // Skip if the domain field's init already references this param
            // (avoids double assignment, which breaks final/readonly/const)
            let field_init = matching_field
                .unwrap()
                .initializer_text
                .as_deref()
                .unwrap_or("");
            if field_init.trim() == p.name {
                continue; // Domain init already assigns from this param
            }
            let assign_code = match syntax.language {
                TargetLanguage::Python3 | TargetLanguage::GDScript | TargetLanguage::Lua => {
                    format!("self.{} = {}", p.name, p.name)
                }
                TargetLanguage::Ruby => {
                    format!("@{} = {}", p.name, p.name)
                }
                TargetLanguage::Php => {
                    format!("$this->{} = ${};", p.name, p.name)
                }
                TargetLanguage::C => {
                    format!("self->{} = {};", p.name, p.name)
                }
                TargetLanguage::Go => {
                    format!("s.{} = {}", p.name, p.name)
                }
                TargetLanguage::Rust => {
                    // Rust handles this in the struct literal — skip here
                    continue;
                }
                TargetLanguage::Cpp => {
                    format!("this->{} = {};", p.name, p.name)
                }
                TargetLanguage::Swift => {
                    format!("self.{} = {}", p.name, p.name)
                }
                TargetLanguage::Erlang => {
                    continue; // Erlang handles domain differently
                }
                _ => {
                    // Java, C#, Kotlin, Dart, TypeScript, JavaScript
                    format!("this.{} = {};", p.name, p.name)
                }
            };
            body.push(CodegenNode::NativeBlock {
                code: assign_code,
                span: None,
            });
        }
    }

    // Rust state vars now live on compartment.state_context — no _sv_ field init needed.
    // State vars are initialized when compartments are created (in transition codegen).

    // Rust system header state/enter param fields: assign each
    // synthetic `__sys_<name>` field from the constructor parameter
    // of the same name. The Rust constructor emitter (rust.rs) sees
    // these as `self.field = value` assignments and folds them into
    // the struct literal.
    if matches!(syntax.language, TargetLanguage::Rust) {
        for p in &system.params {
            match p.kind {
                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                | crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                    body.push(CodegenNode::assign(
                        CodegenNode::field(CodegenNode::self_ref(), &format!("__sys_{}", p.name)),
                        CodegenNode::Ident(p.name.clone()),
                    ));
                }
                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
            }
        }
    }

    // Set initial state (first state in machine)
    // All languages now use the kernel/router/compartment pattern
    if let Some(ref machine) = system.machine {
        if let Some(first_state) = machine.states.first() {
            let compartment_class = format!("{}Compartment", system.name);
            let event_class = format!("{}FrameEvent", system.name);

            // HSM: Build ancestor chain if start state has a parent
            // We need to create compartments for all ancestors and link them via parent_compartment
            let has_hsm_parent = first_state.parent.is_some();

            // Build ancestor chain from root to leaf (reversed order for creation)
            let mut ancestor_chain: Vec<&crate::frame_c::compiler::frame_ast::StateAst> =
                Vec::new();
            if has_hsm_parent {
                let mut current_parent = first_state.parent.as_ref();
                while let Some(parent_name) = current_parent {
                    if let Some(parent_state) =
                        machine.states.iter().find(|s| &s.name == parent_name)
                    {
                        ancestor_chain.push(parent_state);
                        current_parent = parent_state.parent.as_ref();
                    } else {
                        break;
                    }
                }
                // Reverse so we start from root (topmost parent)
                ancestor_chain.reverse();
            }

            // Initialize __compartment with initial state
            match syntax.language {
                TargetLanguage::Rust => {
                    // Rust: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        // For Rust, use a block expression inside struct literal
                        // This creates parent chain and returns the child compartment
                        let mut block_expr = String::new();
                        block_expr.push_str("{\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "None".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            block_expr.push_str(&format!(
                                "let mut {} = {}Compartment::new(\"{}\");\n",
                                comp_var, system.name, ancestor.name
                            ));
                            block_expr.push_str(&format!(
                                "{}.parent_compartment = {};\n",
                                comp_var, prev_comp_var
                            ));
                            // state_context is auto-set by Compartment::new()
                            prev_comp_var = format!("Some(Box::new({}))", comp_var);
                        }
                        // Create the start state compartment with parent link
                        // state_context is auto-set by Compartment::new()
                        block_expr.push_str(&format!(
                            "let mut __child = {}Compartment::new(\"{}\");\n",
                            system.name, first_state.name
                        ));
                        block_expr.push_str(&format!(
                            "__child.parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        block_expr.push_str("__child\n}");

                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::Ident(block_expr),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::Ident("None".to_string()),
                        ));
                    } else {
                        // No HSM parent - simple compartment creation
                        // state_context is auto-set by Compartment::new()
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::Ident(format!(
                                "{}Compartment::new(\"{}\")",
                                system.name, first_state.name
                            )),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::Ident("None".to_string()),
                        ));
                    }
                }
                TargetLanguage::C => {
                    // C: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // state_args / enter_args flow through the helper.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let mut init_code = String::new();
                    // Build temporary FrameVecs for state_args and enter_args.
                    if state_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __sa = NULL;\n",
                            sys = system.name
                        ));
                    } else {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __sa = {sys}_FrameVec_new();\n",
                            sys = system.name
                        ));
                        for n in &state_args_vec {
                            init_code.push_str(&format!(
                                "{sys}_FrameVec_push(__sa, (void*)(intptr_t)({n}));\n",
                                sys = system.name,
                                n = n
                            ));
                        }
                    }
                    if enter_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __ea = NULL;\n",
                            sys = system.name
                        ));
                    } else {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __ea = {sys}_FrameVec_new();\n",
                            sys = system.name
                        ));
                        for n in &enter_args_vec {
                            init_code.push_str(&format!(
                                "{sys}_FrameVec_push(__ea, (void*)(intptr_t)({n}));\n",
                                sys = system.name,
                                n = n
                            ));
                        }
                    }
                    init_code.push_str(&format!(
                        "self->__compartment = {sys}_prepareEnter(self, \"{leaf}\", __sa, __ea);\n",
                        sys = system.name,
                        leaf = first_state.name
                    ));
                    init_code.push_str("self->__next_compartment = NULL;\n");
                    if !state_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec_destroy(__sa);\n",
                            sys = system.name
                        ));
                    }
                    if !enter_args_vec.is_empty() {
                        init_code
                            .push_str(&format!("{sys}_FrameVec_destroy(__ea);", sys = system.name));
                    }
                    body.push(CodegenNode::NativeBlock {
                        code: init_code,
                        span: None,
                    });
                }
                TargetLanguage::Python3 => {
                    // Python: build the start state's compartment chain
                    // via __prepareEnter, the same helper used by all
                    // transitions. System header params flow into the
                    // start state's state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = None",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    // TS/JS: build start chain via __prepareEnter, the same
                    // helper used by all transitions. System header params
                    // flow into state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = this.__prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Php => {
                    // PHP: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| format!("${}", p.name))
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| format!("${}", p.name))
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "$this->__compartment = $this->__prepareEnter(\"{}\", {}, {});\n$this->__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Ruby => {
                    // Ruby: build start chain via __prepareEnter, the same
                    // helper used by all transitions. System header params
                    // flow into state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "@__compartment = __prepareEnter(\"{}\", {}, {})\n@__next_compartment = nil",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Cpp => {
                    // C++: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // state_args / enter_args flow through the helper.
                    // Values are wrapped in std::any so the dispatch
                    // reader (which uses std::any_cast<Type>) round-trips
                    // them correctly.
                    let _ = compartment_class;
                    let state_args_wrapped: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| format!("std::any({})", super::super::codegen_utils::cpp_wrap_any_arg(&p.name)))
                        .collect();
                    let enter_args_wrapped: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| format!("std::any({})", super::super::codegen_utils::cpp_wrap_any_arg(&p.name)))
                        .collect();
                    let state_arg =
                        format!("std::vector<std::any>{{{}}}", state_args_wrapped.join(", "));
                    let enter_arg =
                        format!("std::vector<std::any>{{{}}}", enter_args_wrapped.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "__compartment = __prepareEnter(\"{}\", {}, {});",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Java => {
                    // Java: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "new java.util.ArrayList<>()".to_string()
                    } else {
                        format!(
                            "new java.util.ArrayList<>(java.util.Arrays.asList({}))",
                            state_args_vec.join(", ")
                        )
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "new java.util.ArrayList<>()".to_string()
                    } else {
                        format!(
                            "new java.util.ArrayList<>(java.util.Arrays.asList({}))",
                            enter_args_vec.join(", ")
                        )
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Kotlin => {
                    // Kotlin: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "mutableListOf<Any?>()".to_string()
                    } else {
                        format!("mutableListOf<Any?>({})", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "mutableListOf<Any?>()".to_string()
                    } else {
                        format!("mutableListOf<Any?>({})", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {})\nthis.__next_compartment = null",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Swift => {
                    // Swift: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = {}.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = nil",
                            system.name, first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::CSharp => {
                    // C#: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "new List<object>()".to_string()
                    } else {
                        format!("new List<object> {{ {} }}", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "new List<object>()".to_string()
                    } else {
                        format!("new List<object> {{ {} }}", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Go => {
                    // Go: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "[]any{}".to_string()
                    } else {
                        format!("[]any{{{}}}", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "[]any{}".to_string()
                    } else {
                        format!("[]any{{{}}}", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "s.__compartment = s.__prepareEnter(\"{}\", {}, {})\ns.__next_compartment = nil",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Dart => {
                    // Dart: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = syntax;
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = this.__prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::GDScript => {
                    // GDScript: build start chain via __prepareEnter, the
                    // same helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = null",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                // Lua: build start chain via __prepareEnter, the same
                // helper used by all transitions. System header params
                // flow into state_args / enter_args channels. Uses
                // table.pack(...) / nil instead of `{}` literals
                // (block-transformer workaround).
                TargetLanguage::Lua => {
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "nil".to_string()
                    } else {
                        format!("table.pack({})", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "nil".to_string()
                    } else {
                        format!("table.pack({})", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self:__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = nil",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                // Dynamic languages and remaining: New expression
                // (Erlang, Kotlin — all routed here)
                TargetLanguage::Python3
                | TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Php
                | TargetLanguage::Ruby
                | TargetLanguage::Erlang
                | TargetLanguage::Kotlin => {
                    body.push(CodegenNode::assign(
                        CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                        CodegenNode::New {
                            class: compartment_class.clone(),
                            args: vec![CodegenNode::string(&first_state.name)],
                        },
                    ));
                    body.push(CodegenNode::assign(
                        CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                        CodegenNode::null(),
                    ));
                }
                TargetLanguage::Graphviz => unreachable!(),
            }

            // Send $> (enter) event via __kernel - language-specific.
            // The enter_args of the start state's compartment carry any
            // header-declared enter params; pass them through so the
            // start state's $>(name: type) handler can read them.
            //
            // C optimization: if the start state has no declared enter
            // params, pass NULL for the event's `_parameters` (the
            // dispatch generates no `FrameDict_get` calls in that case).
            let start_state_has_enter_params = first_state
                .enter
                .as_ref()
                .map(|e| !e.params.is_empty())
                .unwrap_or(false);
            let init_event_code = match syntax.language {
                TargetLanguage::Python3 => format!(
                    r#"__e = {ec}("$>", self.__compartment.enter_args)
__ctx = {sys}FrameContext(__e, None)
self._context_stack.append(__ctx)
self.__kernel(__e)
self._context_stack.pop()"#,
                    ec = event_class,
                    sys = system.name
                ),
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    let _ = event_class;
                    // RFC-0020: drain inlined into __kernel.
                    format!(
                        r#"const __e = new {sys}FrameEvent("$>", this.__compartment.enter_args);
const __ctx = new {sys}FrameContext(__e, null);
this._context_stack.push(__ctx);
this.__kernel(__e);
this._context_stack.pop();"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Rust => format!(
                    r#"let __e = std::rc::Rc::new({}::FrameEnter {{ args: self.__compartment.enter_args.clone() }});
let __ctx = {}FrameContext::new(std::rc::Rc::clone(&__e), None);
self._context_stack.push(__ctx);
self.__kernel(&__e);
self._context_stack.pop();"#,
                    event_class, system.name
                ),
                TargetLanguage::C => {
                    let _ = start_state_has_enter_params;
                    format!(
                        r#"{sys}_FrameEvent* __e = {sys}_FrameEvent_new("$>", self->__compartment->enter_args, 0);
{sys}_FrameContext* __ctx = {sys}_FrameContext_new(__e, NULL);
{sys}_FrameVec_push(self->_context_stack, __ctx);
{sys}_kernel(self, __e);
{sys}_FrameContext* __init_ctx = ({sys}_FrameContext*){sys}_FrameVec_pop(self->_context_stack);
{sys}_FrameContext_destroy(__init_ctx);
{sys}_FrameEvent_destroy(__e);"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Cpp => format!(
                    // RFC-0020: drain loop is inlined into __kernel; the
                    // factory just fires the start $> through __kernel.
                    r#"{sys}FrameEvent __e("$>", __compartment->enter_args);
{sys}FrameContext __ctx(std::move(__e));
_context_stack.push_back(std::move(__ctx));
__kernel(_context_stack.back()._event);
_context_stack.pop_back();"#,
                    sys = system.name
                ),
                TargetLanguage::Java => {
                    let _ = event_class;
                    // RFC-0020: drain loop is inlined into __kernel; the
                    // factory just fires the start $> through __kernel.
                    format!(
                        r#"{sys}FrameEvent __e = new {sys}FrameEvent("$>", __compartment.enter_args);
{sys}FrameContext __ctx = new {sys}FrameContext(__e, null);
_context_stack.add(__ctx);
__kernel(__e);
_context_stack.remove(_context_stack.size() - 1);"#,
                        sys = system.name
                    )
                }
                TargetLanguage::CSharp => {
                    let _ = event_class;
                    // RFC-0020: drain loop is inlined into __kernel; the
                    // factory just fires the start $> through __kernel.
                    format!(
                        r#"{sys}FrameEvent __e = new {sys}FrameEvent("$>", __compartment.enter_args);
{sys}FrameContext __ctx = new {sys}FrameContext(__e, null);
_context_stack.Add(__ctx);
__kernel(__e);
_context_stack.RemoveAt(_context_stack.Count - 1);"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Go => {
                    let _ = event_class;
                    // RFC-0020: drain loop is inlined into __kernel; the
                    // factory just fires the start $> through __kernel.
                    format!(
                        r#"__e := {sys}FrameEvent{{_message: "$>", _parameters: s.__compartment.enterArgs}}
__ctx := {sys}FrameContext{{_event: __e, _data: make(map[string]any)}}
s._context_stack = append(s._context_stack, __ctx)
s.__kernel(&s._context_stack[len(s._context_stack)-1]._event)
s._context_stack = s._context_stack[:len(s._context_stack)-1]"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Kotlin => {
                    let _ = event_class;
                    // RFC-0020: drain loop is inlined into __kernel; the
                    // factory just fires the start $> through __kernel.
                    format!(
                        r#"val __e = {sys}FrameEvent("$>", __compartment.enter_args)
val __ctx = {sys}FrameContext(__e, null)
_context_stack.add(__ctx)
__kernel(__e)
_context_stack.removeAt(_context_stack.size - 1)"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Swift => {
                    let _ = event_class;
                    // RFC-0020: drain loop is inlined into __kernel; the
                    // factory just fires the start $> through __kernel.
                    format!(
                        r#"let __e = {sys}FrameEvent(message: "$>", parameters: __compartment.enter_args)
let __ctx = {sys}FrameContext(event: __e)
_context_stack.append(__ctx)
__kernel(__e)
_context_stack.removeLast()"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Php => {
                    let _ = event_class;
                    // RFC-0020: drain inlined into __kernel.
                    format!(
                        r#"$__e = new {sys}FrameEvent("$>", $this->__compartment->enter_args);
$__ctx = new {sys}FrameContext($__e, null);
$this->_context_stack[] = $__ctx;
$this->__kernel($__e);
array_pop($this->_context_stack);"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Ruby => format!(
                    // RFC-0020: drain inlined into __kernel.
                    r#"__e = {ec}.new("$>", @__compartment.enter_args)
__ctx = {sys}FrameContext.new(__e, nil)
@_context_stack.push(__ctx)
__kernel(__e)
@_context_stack.pop"#,
                    ec = event_class,
                    sys = system.name
                ),
                TargetLanguage::Lua => format!(
                    // RFC-0020: drain inlined into __kernel.
                    r#"local __e = {ec}.new("$>", self.__compartment.enter_args)
local __ctx = {sys}FrameContext.new(__e, nil)
self._context_stack[#self._context_stack + 1] = __ctx
self:__kernel(__e)
table.remove(self._context_stack)"#,
                    ec = event_class,
                    sys = system.name
                ),
                TargetLanguage::Dart => {
                    let _ = event_class;
                    // RFC-0020: drain loop is inlined into __kernel; the
                    // factory just fires the start $> through __kernel.
                    format!(
                        r#"final __e = {sys}FrameEvent("\$>", __compartment.enter_args);
final __ctx = {sys}FrameContext(__e, null);
_context_stack.add(__ctx);
__kernel(__e);
_context_stack.removeLast();"#,
                        sys = system.name
                    )
                }
                TargetLanguage::GDScript => {
                    let _ = event_class;
                    format!(
                        r#"var __e = {sys}FrameEvent.new("$>", self.__compartment.enter_args)
var __ctx = {sys}FrameContext.new(__e, null)
self._context_stack.append(__ctx)
self.__kernel(__e)
self._context_stack.pop_back()"#,
                        sys = system.name
                    )
                }
                TargetLanguage::Erlang => String::new(), // gen_statem: handled natively by erlang_system.rs
                TargetLanguage::Graphviz => unreachable!(),
            };
            body.push(CodegenNode::NativeBlock {
                code: init_event_code,
                span: None,
            });
        }
    }

    // Params from system params
    let params: Vec<Param> = system
        .params
        .iter()
        .map(|p| {
            let type_str = type_to_string(&p.param_type);
            let mut param = Param::new(&p.name).with_type(&type_str);
            if let Some(ref def) = p.default {
                param = param.with_default(CodegenNode::Ident(def.clone()));
            }
            param
        })
        .collect();

    // For C++: emit const fields via member initializer list.
    // The C++ backend formats super_call as ` : {expr}` before the body.
    let super_call = if !cpp_init_list.is_empty() && matches!(syntax.language, TargetLanguage::Cpp)
    {
        Some(Box::new(CodegenNode::Ident(cpp_init_list.join(", "))))
    } else {
        None
    };

    CodegenNode::Constructor {
        params,
        body,
        super_call,
    }
}
