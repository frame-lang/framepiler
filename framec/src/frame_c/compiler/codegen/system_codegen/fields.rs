//! Domain-and-runtime field emission for the generated system class.
//!
//! Every generated system carries four runtime-machinery fields plus
//! one structured `Field` per user-declared domain variable:
//!
//! 1. `_state_stack` — stack of `Compartment` values for `push$`/`pop$`.
//! 2. `__compartment` — the current leaf compartment.
//! 3. `__next_compartment` — deferred-transition slot drained by the
//!    transition loop.
//! 4. `_context_stack` — stack of `FrameContext` values, one per
//!    re-entrant dispatch.
//!
//! Field types vary per backend (`Vec<Compartment>` in Rust, untyped
//! `List` in dynamic languages, `*Compartment` in Go and C, etc.); the
//! mapping is laid out inline in `generate_fields` rather than spread
//! across the backends so the per-language storage shape is reviewable
//! in one place.
//!
//! Domain variables are emitted as structured `Field`s — backends read
//! the structured slots (`name`, `type_annotation`, `initializer`,
//! `is_const`) directly instead of re-parsing a synthesized
//! declaration string. Three special cases:
//!
//! - **Go / C** strip every domain initializer from the declaration —
//!   the constructor assigns instead. Go has no in-struct initializer
//!   syntax; C requires `static const` for inline init, which doesn't
//!   match Frame's mutable-domain semantics.
//! - **Initializer references a system param**: assignment moves to
//!   the constructor body / `__frame_init`. If the field was `const`
//!   the target-language `val` / `let` / `final` keyword is dropped
//!   (a body-assignment to it would not compile); Frame-level `const`
//!   stays enforced via validator E814+. C++ is the exception — its
//!   member-initializer list seeds the const, so `const T` stays.
//! - **PHP non-const expression**: `public $inner = new Counter();` is
//!   a parse error in PHP. Tagged-system-instantiation defaults
//!   (detected by `@@` in init text) get stripped on PHP only.
//!
//! Rust gets one extra emission: state-arg / enter-arg params on the
//! system header materialize as `__sys_<name>` typed fields. This is
//! the Rust equivalent of the dynamic backends' `state_args` /
//! `enter_args` dict — typed fields rather than a typed-enum variant
//! per state.

use super::{
    expand_system_instantiation_in_domain, init_references_param, type_to_string, CodegenNode,
    Field, SystemAst, TargetLanguage, Type, Visibility,
};

pub(crate) fn generate_fields(
    system: &SystemAst,
    syntax: &super::super::backend::ClassSyntax,
) -> Vec<Field> {
    let mut fields = Vec::new();
    let compartment_type = format!("{}Compartment", system.name);

    // State stack - for push/pop state operations
    let stack_type = match syntax.language {
        TargetLanguage::Rust => format!("Vec<{}Compartment>", system.name),
        TargetLanguage::Cpp => format!("std::vector<std::shared_ptr<{}Compartment>>", system.name),
        TargetLanguage::Java => format!("java.util.ArrayList<{}Compartment>", system.name),
        TargetLanguage::Kotlin => format!("MutableList<{}Compartment>", system.name),
        TargetLanguage::Dart => format!("List<{}Compartment>", system.name),
        TargetLanguage::Swift => format!("[{}Compartment]", system.name),
        TargetLanguage::CSharp => format!("List<{}Compartment>", system.name),
        TargetLanguage::Go => format!("[]*{}Compartment", system.name),
        // Dynamic languages: untyped lists — type annotation is for documentation only
        TargetLanguage::Python3
        | TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Lua
        | TargetLanguage::GDScript => "List".to_string(),
        TargetLanguage::C => "List".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(
        Field::new("_state_stack")
            .with_visibility(Visibility::Private)
            .with_type(&stack_type),
    );

    // Compartment field - canonical compartment architecture for ALL languages
    let (comp_field_type, nullable_comp_type) = match syntax.language {
        TargetLanguage::Rust => (
            compartment_type.clone(),
            format!("Option<{}>", compartment_type),
        ),
        TargetLanguage::Cpp => (
            format!("std::shared_ptr<{}>", compartment_type),
            format!("std::shared_ptr<{}>", compartment_type),
        ),
        TargetLanguage::Java => (compartment_type.clone(), compartment_type.clone()),
        TargetLanguage::CSharp => (compartment_type.clone(), format!("{}?", compartment_type)),
        TargetLanguage::Kotlin | TargetLanguage::Swift | TargetLanguage::Dart => {
            (compartment_type.clone(), format!("{}?", compartment_type))
        }
        TargetLanguage::Go => (
            format!("*{}", compartment_type),
            format!("*{}", compartment_type),
        ),
        // Dynamic languages: nullable via language convention (None/null/nil)
        TargetLanguage::Python3
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Lua
        | TargetLanguage::GDScript => (compartment_type.clone(), compartment_type.clone()),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => (
            compartment_type.clone(),
            format!("{} | null", compartment_type),
        ),
        TargetLanguage::Php => (compartment_type.clone(), format!("?{}", compartment_type)),
        TargetLanguage::C => (
            format!("{}*", compartment_type),
            format!("{}*", compartment_type),
        ),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(
        Field::new("__compartment")
            .with_visibility(Visibility::Private)
            .with_type(&comp_field_type),
    );

    // Next compartment field - for deferred transition caching in __kernel
    fields.push(
        Field::new("__next_compartment")
            .with_visibility(Visibility::Private)
            .with_type(&nullable_comp_type),
    );

    // Context stack for reentrancy - holds FrameContext objects
    let context_stack_type = match syntax.language {
        TargetLanguage::Rust => format!("Vec<{}FrameContext>", system.name),
        TargetLanguage::Cpp => format!("std::vector<{}FrameContext>", system.name),
        TargetLanguage::Java => format!("java.util.ArrayList<{}FrameContext>", system.name),
        TargetLanguage::Kotlin => format!("MutableList<{}FrameContext>", system.name),
        TargetLanguage::Dart => format!("List<{}FrameContext>", system.name),
        TargetLanguage::Swift => format!("[{}FrameContext]", system.name),
        TargetLanguage::CSharp => format!("List<{}FrameContext>", system.name),
        TargetLanguage::Go => format!("[]{}FrameContext", system.name),
        TargetLanguage::Python3
        | TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Lua
        | TargetLanguage::GDScript => "List".to_string(),
        TargetLanguage::C => "List".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(
        Field::new("_context_stack")
            .with_visibility(Visibility::Private)
            .with_type(&context_stack_type),
    );

    // Domain variables — build a structured `Field` for each. Backends
    // consume the structured slots (name, type_annotation, initializer,
    // is_const) directly via their own `emit_field` helpers; nothing
    // re-parses a synthesized declaration string anymore.
    for domain_var in &system.domain {
        let type_str_opt = match &domain_var.var_type {
            Type::Custom(s) => Some(s.clone()),
            Type::Unknown => None,
        };
        let sys_param_names: Vec<String> = system.params.iter().map(|p| p.name.clone()).collect();

        let mut field = Field::new(&domain_var.name)
            .with_visibility(Visibility::Public)
            .with_leading_comments(domain_var.leading_comments.clone());
        if let Some(ref t) = type_str_opt {
            field = field.with_type(t);
        }
        field.is_const = domain_var.is_const;

        // Populate the structured initializer slot from init text —
        // but ONLY when the init wasn't stripped from the field declaration.
        // If the init references a system param, the assignment belongs in
        // the constructor body, not the field declaration.
        // Apply expand_system_instantiation_in_domain so backends consuming this slot get
        // the fully-expanded init text (@@SystemName → native constructor).
        let init_text_str = domain_var.initializer_text.as_deref().unwrap_or("");
        let strip_unconditionally =
            matches!(syntax.language, TargetLanguage::Go | TargetLanguage::C);
        let strip_collision = init_references_param(init_text_str, &sys_param_names);

        // When a const field's init references a system param the
        // assignment moves to the constructor body / `__frame_init`
        // (see `should_emit_constructor_body_init`). A method-body
        // assignment to a `val` / `let` / `final` won't compile, so
        // emit those fields as mutable at the target-language level —
        // the Frame-level `const` is enforced by the validator (E814+).
        // C++ is the exception: it seeds the const field via the
        // member-initializer list, so `const T` is fine there.
        if field.is_const && strip_collision && !matches!(syntax.language, TargetLanguage::Cpp) {
            field.is_const = false;
        }
        // PHP rejects non-const expressions in class-field defaults:
        // `public $inner = new Counter();` is a parse error
        // ("New expressions are not supported in this context"). Strip
        // any tagged-system-instantiation default and let the
        // constructor body initialize it instead. Detected by the
        // presence of `@@` in the initializer text — only sibling-
        // system instantiations use that token in domain init exprs.
        let strip_php_non_const =
            matches!(syntax.language, TargetLanguage::Php) && init_text_str.contains("@@");
        if !(strip_unconditionally || strip_collision || strip_php_non_const) {
            if let Some(ref init_text) = &domain_var.initializer_text {
                let expanded_init =
                    expand_system_instantiation_in_domain(init_text, syntax.language);
                field = field.with_initializer(CodegenNode::Ident(expanded_init));
            }
        }

        fields.push(field);
    }

    // Rust system header state/enter params: stash on the system struct
    // as `__sys_<name>` typed fields. The constructor receives the
    // system params via its signature, assigns them into these synthetic
    // fields, and the per-state dispatch reads them into bare locals via
    // the binding preamble inserted by `generate_handler_from_arcanum`.
    //
    // Domain params are handled by the existing domain field path
    // (the domain field IS the storage), so we skip them here.
    //
    // This is the Rust equivalent of the HashMap<String, Any>
    // `state_args`/`enter_args` dict approach used by the dynamic
    // backends — typed fields keep idiomatic Rust without inventing
    // a new typed-enum variant per state.
    if matches!(syntax.language, TargetLanguage::Rust) {
        for p in &system.params {
            match p.kind {
                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                | crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                    let type_str = type_to_string(&p.param_type);
                    fields.push(
                        Field::new(&format!("__sys_{}", p.name))
                            .with_visibility(Visibility::Private)
                            .with_type(&type_str),
                    );
                }
                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
            }
        }
    }

    fields
}
