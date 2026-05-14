//! Factory-alias generation for the RFC-0015 `@@[create(<name>)]`
//! contract.
//!
//! When a system declares an explicit factory name (e.g.
//! `@@[create(make)]`), the codegen emits a static / classmethod
//! alias named `<name>` that delegates to the canonical `_create`
//! (Python / GDScript / etc.) or `__create` (JS / TS) factory. The
//! alias mirrors the constructor's parameter list, forwards
//! verbatim, and returns the constructed instance.
//!
//! Three emission paths, by family:
//!
//! - **Python** — `@classmethod` with `cls`, returns `'<system>'`
//!   (string-form forward reference, so the annotation evaluates
//!   lazily — at class-definition time the class name is not yet
//!   bound).
//! - **JS / TS** — `static` method, returns `<system>`. Body
//!   templates without param types.
//! - **GDScript / Lua / Ruby / PHP / Dart** — `static` method,
//!   shared body-template generator. Each backend supplies its own
//!   raw target-language snippet via `generate_static_factory_alias`.

use super::{type_to_string, CodegenNode, Param, SystemAst, Visibility};

pub(super) fn generate_python_factory_alias(
    system: &SystemAst,
    factory_name: &str,
) -> CodegenNode {
    let params: Vec<Param> = system
        .params
        .iter()
        .map(|p| {
            let type_str = type_to_string(&p.param_type);
            Param::new(&p.name).with_type(&type_str)
        })
        .collect();

    let arg_list = system
        .params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let body = vec![CodegenNode::NativeBlock {
        code: format!("return cls._create({})", arg_list),
        span: None,
    }];

    // Quote the return type as a string-form forward reference so
    // the annotation is evaluated lazily — at class-definition time
    // the class name itself is not yet bound.
    CodegenNode::Method {
        name: factory_name.to_string(),
        params,
        return_type: Some(format!("'{}'", system.name)),
        body,
        is_async: false,
        is_static: false,
        visibility: Visibility::Public,
        decorators: vec!["classmethod".to_string()],
    }
}

/// RFC-0015 phase 1.1d: factory alias for JS / TS.
///
/// When `@@[create(<name>)]` is set on a system, emit a `static`
/// method named `<name>` that delegates to the constructor via
/// `new ClassName(...)`. The signature mirrors the constructor.
/// The existing `new ClassName(seed)` call site is unaffected;
/// the rename is additive.
pub(super) fn generate_js_static_factory_alias(
    system: &SystemAst,
    factory_name: &str,
) -> CodegenNode {
    let params: Vec<Param> = system
        .params
        .iter()
        .map(|p| {
            let type_str = type_to_string(&p.param_type);
            Param::new(&p.name).with_type(&type_str)
        })
        .collect();

    let arg_list = system
        .params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let body = vec![CodegenNode::NativeBlock {
        code: format!("return {}._create({});", system.name, arg_list),
        span: None,
    }];

    CodegenNode::Method {
        name: factory_name.to_string(),
        params,
        return_type: Some(system.name.clone()),
        body,
        is_async: false,
        is_static: true,
        visibility: Visibility::Public,
        decorators: vec![],
    }
}

/// RFC-0015: param-name list for a system, comma-separated.
/// Used by factory aliases to forward arguments to the constructor.
pub(super) fn params_arg_list(system: &SystemAst) -> String {
    system
        .params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// RFC-0015 phase 1.1d: shared static-factory alias generator for
/// backends whose `is_static: true` Method renders cleanly as a
/// static method on the class. The body is supplied as a raw
/// target-language snippet that constructs and returns an instance.
///
/// Used by GDScript, Lua, Ruby, PHP, and Dart. Python uses its own
/// path (classmethod with `cls`); JS/TS use their own path (no
/// param types in body).
pub(super) fn generate_static_factory_alias(
    system: &SystemAst,
    factory_name: &str,
    body_code: &str,
) -> CodegenNode {
    let params: Vec<Param> = system
        .params
        .iter()
        .map(|p| {
            let type_str = type_to_string(&p.param_type);
            Param::new(&p.name).with_type(&type_str)
        })
        .collect();

    let body = vec![CodegenNode::NativeBlock {
        code: body_code.to_string(),
        span: None,
    }];

    CodegenNode::Method {
        name: factory_name.to_string(),
        params,
        return_type: Some(system.name.clone()),
        body,
        is_async: false,
        is_static: true,
        visibility: Visibility::Public,
        decorators: vec![],
    }
}
