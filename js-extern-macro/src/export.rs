//! `#[js_export]` expansion for sync functions (including `HashMap`/`BTreeMap` parameter
//! adapters), `async fn` (`TypedPromise<T>`), structs, and stateful inherent `impl` classes.

use std::collections::BTreeSet;

use mires_marker_abi::export::{
    encode_export_section, is_valid_chunk_name, ChunkRole, EXPORT_MARSHAL_SECTION,
    EXPORT_OUT_SECTION,
};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Attribute, FnArg, GenericArgument, GenericParam, Generics, Ident, ImplItem, ImplItemFn, Item,
    ItemFn, ItemImpl, ItemStruct, LitStr, Pat, Path, PathArguments, ReturnType, Token, Type,
    Visibility,
};

use crate::is_js_ident;

/// ECMAScript strict-mode reserved words (sorted for binary search).
pub(crate) const JS_RESERVED_WORDS: &[&str] = &[
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Case-conversion rule accepted by `#[js_export(rename_all = "..")]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenameRule {
    CamelCase,
}

impl RenameRule {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "camelCase" => Some(Self::CamelCase),
            _ => None,
        }
    }

    fn apply(self, ident: &str) -> String {
        match self {
            Self::CamelCase => to_camel_case(ident),
        }
    }
}

/// Converts a `snake_case` Rust identifier to `camelCase`, preserving leading and trailing
/// underscores (so `_internal_fn` becomes `_internalFn` and keyword escapes like `default_` keep
/// their suffix).
fn to_camel_case(ident: &str) -> String {
    let stripped = ident.strip_prefix("r#").unwrap_or(ident);
    let leading = stripped.bytes().take_while(|&b| b == b'_').count();
    let rest = &stripped[leading..];
    if rest.is_empty() {
        return stripped.to_owned();
    }
    let trailing = rest.bytes().rev().take_while(|&b| b == b'_').count();
    let core = &rest[..rest.len() - trailing];

    let mut out = String::with_capacity(stripped.len());
    out.push_str(&stripped[..leading]);
    let mut first_segment = true;
    for part in core.split('_') {
        if part.is_empty() {
            continue;
        }
        if first_segment {
            out.push_str(part);
            first_segment = false;
        } else {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    out.push_str(&rest[rest.len() - trailing..]);
    out
}

/// Arguments parsed from top-level `#[js_export(..)]`.
struct ExportAttr {
    name: Option<String>,
    raw: Option<Span>,
    crate_path: Option<Path>,
    chunk: Option<(Span, String)>,
    role: Option<(Span, ChunkRole)>,
    rename_all: Option<(Span, RenameRule)>,
}

impl ExportAttr {
    fn chunk_spec(&self) -> Option<(&str, ChunkRole)> {
        self.chunk
            .as_ref()
            .map(|(_, name)| (name.as_str(), self.role.map_or(ChunkRole::Main, |(_, r)| r)))
    }

    fn export_link_section(&self) -> Option<String> {
        encode_export_section(self.raw.is_none(), self.chunk_spec())
    }
}

/// One entry inside `#[js_export(..)]`.
enum ExportEntry {
    Name(Span, String),
    Raw(Span),
    Crate(Span, Path),
    Chunk(Span, String),
    Role(Span, String),
    RenameAll(Span, String),
    Skip(Span),
}

impl Parse for ExportEntry {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Token![crate]) {
            let kw = input.parse::<Token![crate]>()?;
            if !input.peek(Token![=]) {
                return Err(syn::Error::new(
                    kw.span(),
                    "`crate` in `#[js_export(..)]` requires a crate path, \
                     e.g. `#[js_export(crate = mires_js)]`",
                ));
            }
            input.parse::<Token![=]>()?;
            let path: Path = input.parse()?;
            return Ok(Self::Crate(kw.span(), path));
        }

        let key: Ident = input.parse()?;
        match key.to_string().as_str() {
            "name" => {
                if !input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        key.span(),
                        "`name` in `#[js_export(..)]` requires a JavaScript identifier string, \
                         e.g. `#[js_export(name = \"myFunc\")]`",
                    ));
                }
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                Ok(Self::Name(lit.span(), lit.value()))
            }
            "raw" => {
                if input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        key.span(),
                        "`raw` in `#[js_export(..)]` takes no value",
                    ));
                }
                Ok(Self::Raw(key.span()))
            }
            "chunk" => {
                if !input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        key.span(),
                        "`chunk` in `#[js_export(..)]` requires a chunk name string, \
                         e.g. `#[js_export(chunk = \"counter\")]`",
                    ));
                }
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                Ok(Self::Chunk(lit.span(), lit.value()))
            }
            "role" => {
                if !input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        key.span(),
                        "`role` in `#[js_export(..)]` requires a role string, \
                         e.g. `#[js_export(chunk = \"counter\", role = \"island\")]`",
                    ));
                }
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                Ok(Self::Role(lit.span(), lit.value()))
            }
            "rename_all" => {
                if !input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        key.span(),
                        "`rename_all` in `#[js_export(..)]` requires a convention string, \
                         e.g. `#[js_export(rename_all = \"camelCase\")]`",
                    ));
                }
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                Ok(Self::RenameAll(lit.span(), lit.value()))
            }
            "skip" => {
                if input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        key.span(),
                        "`skip` in `#[js_export(..)]` takes no value",
                    ));
                }
                Ok(Self::Skip(key.span()))
            }
            other => Err(syn::Error::new(
                key.span(),
                format!(
                    "unknown `#[js_export]` argument `{other}`; expected `name = \"..\"`, `raw`, \
                     `chunk = \"..\"`, `role = \"..\"`, `rename_all = \"camelCase\"`, `skip`, or \
                     `crate = <path>`"
                ),
            )),
        }
    }
}

impl Parse for ExportAttr {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut name = None;
        let mut raw = None;
        let mut crate_path = None;
        let mut chunk = None;
        let mut role = None;
        let mut rename_all = None;

        for entry in Punctuated::<ExportEntry, Token![,]>::parse_terminated(input)? {
            match entry {
                ExportEntry::Name(span, value) => {
                    if name.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `name` in `#[js_export(..)]`",
                        ));
                    }
                    validate_js_export_ident(&value, span)?;
                    name = Some(value);
                }
                ExportEntry::Raw(span) => {
                    if raw.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `raw` in `#[js_export(..)]`",
                        ));
                    }
                    raw = Some(span);
                }
                ExportEntry::Crate(span, path) => {
                    if crate_path.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `crate` in `#[js_export(..)]`",
                        ));
                    }
                    crate_path = Some(path);
                }
                ExportEntry::Chunk(span, value) => {
                    if chunk.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `chunk` in `#[js_export(..)]`",
                        ));
                    }
                    if !is_valid_chunk_name(&value) {
                        return Err(syn::Error::new(
                            span,
                            format!(
                                "`{value}` is not a valid chunk name: must be non-empty and \
                                 contain only ASCII alphanumerics, `_`, `-`, or `$`"
                            ),
                        ));
                    }
                    chunk = Some((span, value));
                }
                ExportEntry::Role(span, value) => {
                    if role.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `role` in `#[js_export(..)]`",
                        ));
                    }
                    let parsed = ChunkRole::parse(&value).ok_or_else(|| {
                        syn::Error::new(
                            span,
                            format!(
                                "unknown `#[js_export]` role `{value}`; expected `\"main\"`, \
                                 `\"island\"`, `\"worker\"`, or `\"procedure\"`"
                            ),
                        )
                    })?;
                    role = Some((span, parsed));
                }
                ExportEntry::RenameAll(span, value) => {
                    if rename_all.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `rename_all` in `#[js_export(..)]`",
                        ));
                    }
                    let parsed = RenameRule::parse(&value).ok_or_else(|| {
                        syn::Error::new(
                            span,
                            format!(
                                "unknown `rename_all` convention `{value}`; expected \
                                 `\"camelCase\"`"
                            ),
                        )
                    })?;
                    rename_all = Some((span, parsed));
                }
                ExportEntry::Skip(span) => {
                    return Err(syn::Error::new(
                        span,
                        "`skip` can only be used on methods inside a `#[js_export] impl` block",
                    ));
                }
            }
        }

        if let (None, Some((role_span, _))) = (&chunk, &role) {
            return Err(syn::Error::new(
                *role_span,
                "`role` in `#[js_export(..)]` requires `chunk = \"..\"`",
            ));
        }

        Ok(Self {
            name,
            raw,
            crate_path,
            chunk,
            role,
            rename_all,
        })
    }
}

/// Method-level `#[js_export(..)]` inside a `#[js_export] impl` block.
struct MethodExportAttr {
    name: Option<String>,
    skip: Option<Span>,
}

impl Parse for MethodExportAttr {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut name = None;
        let mut skip = None;
        for entry in Punctuated::<ExportEntry, Token![,]>::parse_terminated(input)? {
            match entry {
                ExportEntry::Name(span, value) => {
                    if name.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `name` in `#[js_export(..)]`",
                        ));
                    }
                    validate_js_method_ident(&value, span)?;
                    name = Some(value);
                }
                ExportEntry::Skip(span) => {
                    if skip.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `skip` in `#[js_export(..)]`",
                        ));
                    }
                    skip = Some(span);
                }
                ExportEntry::Raw(span) => {
                    return Err(syn::Error::new(
                        span,
                        "`raw` is not supported on methods inside `#[js_export] impl`",
                    ));
                }
                ExportEntry::Crate(span, _) => {
                    return Err(syn::Error::new(
                        span,
                        "`crate` must be placed on the `#[js_export] impl` block rather than an \
                         individual method",
                    ));
                }
                ExportEntry::Chunk(span, _) => {
                    return Err(syn::Error::new(
                        span,
                        "`chunk` is not supported on methods inside `#[js_export] impl`",
                    ));
                }
                ExportEntry::Role(span, _) => {
                    return Err(syn::Error::new(
                        span,
                        "`role` is not supported on methods inside `#[js_export] impl`",
                    ));
                }
                ExportEntry::RenameAll(span, _) => {
                    return Err(syn::Error::new(
                        span,
                        "`rename_all` must be placed on the `#[js_export] impl` block rather \
                         than an individual method",
                    ));
                }
            }
        }
        if let (Some(_), Some(skip_span)) = (&name, skip) {
            return Err(syn::Error::new(
                skip_span,
                "`skip` cannot be combined with `name` in `#[js_export(..)]`",
            ));
        }
        Ok(Self { name, skip })
    }
}

/// Validates that `name` is a legal JavaScript top-level identifier (for exports and class names).
fn validate_js_export_ident(name: &str, span: Span) -> syn::Result<()> {
    if name.is_empty() {
        return Err(syn::Error::new(
            span,
            "`name` in `#[js_export(name = \"..\")]` must not be empty",
        ));
    }
    if !is_js_ident(name) {
        return Err(syn::Error::new(
            span,
            format!(
                "`{name}` is not a valid JavaScript identifier: it must start with an ASCII \
                 letter, `_`, or `$`, and contain only ASCII alphanumerics, `_`, or `$`"
            ),
        ));
    }
    if is_js_reserved_word(name) {
        return Err(syn::Error::new(
            span,
            format!(
                "`{name}` is a JavaScript reserved word and cannot be used as a `#[js_export]` name"
            ),
        ));
    }
    Ok(())
}

/// Validates that `name` is a legal JavaScript class method identifier.
fn validate_js_method_ident(name: &str, span: Span) -> syn::Result<()> {
    if name.is_empty() {
        return Err(syn::Error::new(
            span,
            "`name` in `#[js_export(name = \"..\")]` must not be empty",
        ));
    }
    if !is_js_ident(name) {
        return Err(syn::Error::new(
            span,
            format!(
                "`{name}` is not a valid JavaScript method identifier: it must start with an \
                 ASCII letter, `_`, or `$`, and contain only ASCII alphanumerics, `_`, or `$`"
            ),
        ));
    }
    if matches!(name, "constructor" | "__drop") {
        return Err(syn::Error::new(
            span,
            format!(
                "`{name}` is reserved for class lifecycle handling and cannot be used as a \
                 `#[js_export]` method name"
            ),
        ));
    }
    Ok(())
}

/// Whether `name` is an ECMAScript strict-mode reserved word.
fn is_js_reserved_word(name: &str) -> bool {
    JS_RESERVED_WORDS.binary_search(&name).is_ok()
}

/// Entry point for `#[js_export]` supporting `fn`, `async fn`, `struct`, and inherent `impl`.
pub(crate) fn expand_export(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args: ExportAttr = syn::parse2(attr)?;
    let parsed: Item = syn::parse2(item)?;
    match parsed {
        Item::Fn(function) => expand_export_fn(&args, function),
        Item::Struct(item_struct) => expand_export_struct(&args, item_struct),
        Item::Impl(item_impl) => expand_export_impl(&args, item_impl),
        other => Err(syn::Error::new(
            other.span(),
            "`#[js_export]` can only be applied to functions, structs, or inherent `impl` blocks",
        )),
    }
}

/// Resolves the hidden `__export` runtime support module path.
fn rt_support_path(crate_path: Option<&Path>) -> TokenStream {
    match crate_path {
        Some(path) => quote! { #path::__export },
        None => quote! { ::mires::__export },
    }
}

/// Rejects type and const generic parameters on wrapped exports (lifetimes are allowed).
fn reject_non_lifetime_generics(generics: &Generics, context: &str) -> syn::Result<()> {
    for param in &generics.params {
        if !matches!(param, GenericParam::Lifetime(_)) {
            return Err(syn::Error::new(
                param.span(),
                format!("`#[js_export]` does not support generic {context}"),
            ));
        }
    }
    Ok(())
}

/// Map collection kinds recognized syntactically on top-level exported parameters.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MapKind {
    HashMap,
    BTreeMap,
}

/// Conversion mode when building wrapper parameter lists.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParamMode {
    Raw,
    Sync,
    Async,
}

/// Unwraps any invisible proc-macro `Type::Group` delimiters around `ty`.
fn unwrap_type_group(mut ty: &Type) -> &Type {
    while let Type::Group(group) = ty {
        ty = &group.elem;
    }
    ty
}

/// Matches top-level `HashMap<K, V>` or `BTreeMap<K, V>` by last path segment and 2 type args.
fn classify_map_param(ty: &Type) -> Option<(MapKind, &Type, &Type)> {
    let Type::Path(type_path) = unwrap_type_group(ty) else {
        return None;
    };
    if type_path.qself.is_some() {
        return None;
    }
    let last = type_path.path.segments.last()?;
    let kind = match last.ident.to_string().as_str() {
        "HashMap" => MapKind::HashMap,
        "BTreeMap" => MapKind::BTreeMap,
        _ => return None,
    };
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    if args.args.len() != 2 {
        return None;
    }
    let mut iter = args.args.iter();
    let (Some(GenericArgument::Type(key_ty)), Some(GenericArgument::Type(val_ty))) =
        (iter.next(), iter.next())
    else {
        return None;
    };
    Some((kind, key_ty, val_ty))
}

/// Whether `ty` is a non-`'static` immutable `&str` reference that needs owning for `async move`.
fn is_non_static_str_ref(ty: &Type) -> bool {
    let Type::Reference(reference) = unwrap_type_group(ty) else {
        return false;
    };
    if reference.mutability.is_some() {
        return false;
    }
    if reference
        .lifetime
        .as_ref()
        .is_some_and(|lt| lt.ident == "static")
    {
        return false;
    }
    let Type::Path(elem_path) = unwrap_type_group(&reference.elem) else {
        return false;
    };
    elem_path.qself.is_none() && elem_path.path.is_ident("str")
}

/// Prepared parameter list and forwarded call expressions for a generated `mires` export wrapper.
struct PreparedParams {
    wrapper_params: Vec<TokenStream>,
    call_args: Vec<TokenStream>,
}

/// Builds wrapper parameters and forwarding expressions from typed function parameters.
fn prepare_wrapper_params<'a>(
    inputs: impl Iterator<Item = &'a FnArg>,
    rt: &TokenStream,
    mode: ParamMode,
    self_ty_sub: Option<&Type>,
) -> syn::Result<PreparedParams> {
    let mut wrapper_params = Vec::new();
    let mut call_args = Vec::new();

    for (idx, arg) in inputs.enumerate() {
        let FnArg::Typed(pat_type) = arg else {
            return Err(syn::Error::new(
                arg.span(),
                "unexpected `self` receiver in exported function",
            ));
        };
        let param_ident = match pat_type.pat.as_ref() {
            Pat::Ident(pat_ident)
                if pat_ident.subpat.is_none()
                    && pat_ident.ident != "__mires_self"
                    && pat_ident.ident != "__mires_owned"
                    && pat_ident.ident != "__mires_export_out" =>
            {
                pat_ident.ident.clone()
            }
            _ => format_ident!("__mires_arg_{idx}"),
        };
        let param_ty = match self_ty_sub {
            Some(self_ty) => substitute_self_type(&pat_type.ty, self_ty),
            None => (*pat_type.ty).clone(),
        };

        if mode != ParamMode::Raw {
            if let Some((kind, key_ty, val_ty)) = classify_map_param(&param_ty) {
                let carrier = match kind {
                    MapKind::HashMap => quote! { #rt::JsMapEntries<#key_ty, #val_ty> },
                    MapKind::BTreeMap => quote! { #rt::JsBTreeMapEntries<#key_ty, #val_ty> },
                };
                wrapper_params.push(quote! { #param_ident: #carrier });
                call_args.push(quote! { ::core::iter::FromIterator::from_iter(#param_ident.0) });
                continue;
            }
        }
        if mode == ParamMode::Async && is_non_static_str_ref(&param_ty) {
            wrapper_params.push(quote! { #param_ident: #rt::String });
            call_args.push(quote! { #param_ident.as_str() });
        } else {
            wrapper_params.push(quote! { #param_ident: #param_ty });
            call_args.push(quote! { #param_ident });
        }
    }

    Ok(PreparedParams {
        wrapper_params,
        call_args,
    })
}

/// Resolves the JavaScript export name for a free function.
fn resolve_fn_export_name(args: &ExportAttr, fn_ident: &Ident) -> syn::Result<String> {
    if let (Some(_), Some((rename_span, _))) = (&args.name, &args.rename_all) {
        return Err(syn::Error::new(
            *rename_span,
            "`rename_all` cannot be combined with explicit `name` on a free function",
        ));
    }
    if let Some(name) = &args.name {
        return Ok(name.clone());
    }
    let raw_name = fn_ident.to_string();
    if let Some((_, rule)) = &args.rename_all {
        let converted = rule.apply(&raw_name);
        validate_js_export_ident(&converted, fn_ident.span())?;
        return Ok(converted);
    }
    Ok(raw_name)
}

/// Extracts `#[doc = ...]` attributes so generated `#[js_export]` wrappers forward Rust doc
/// comments to `backend/src/dts.rs` for `.d.ts` JSDoc synthesis.
fn doc_attrs(attrs: &[Attribute]) -> Vec<&Attribute> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .collect()
}

/// Expands `#[js_export]` on a synchronous or `async` free function.
fn expand_export_fn(args: &ExportAttr, function: ItemFn) -> syn::Result<TokenStream> {
    if function.sig.asyncness.is_some() {
        return expand_export_async_fn(args, function);
    }

    reject_non_lifetime_generics(&function.sig.generics, "functions")?;
    let js_name = resolve_fn_export_name(args, &function.sig.ident)?;
    let rt = rt_support_path(args.crate_path.as_ref());
    let mode = if args.raw.is_some() {
        ParamMode::Raw
    } else {
        ParamMode::Sync
    };
    let prepared = prepare_wrapper_params(function.sig.inputs.iter(), &rt, mode, None)?;
    let fn_ident = &function.sig.ident;
    let generics = &function.sig.generics;
    let where_clause = &function.sig.generics.where_clause;
    let ret_output = &function.sig.output;
    let wrapper_params = &prepared.wrapper_params;
    let call_args = &prepared.call_args;
    let docs = doc_attrs(&function.attrs);
    let section_attr = match args.export_link_section() {
        Some(section) => quote! { #[unsafe(link_section = #section)] },
        None => quote! {},
    };

    Ok(quote! {
        #function

        #[allow(unexpected_cfgs, reason = "mires target cfg is supplied by cargo-mires")]
        const _: () = {
            #[cfg(mires)]
            const _: () = {
                #(#docs)*
                #[unsafe(export_name = #js_name)]
                #section_attr
                fn __mires_export #generics (#(#wrapper_params),*) #ret_output #where_clause {
                    #fn_ident(#(#call_args),*)
                }
            };
        };
    })
}

/// Expands `#[js_export]` on an `async fn`, returning `TypedPromise<R>` on `cfg(mires)`.
fn expand_export_async_fn(args: &ExportAttr, function: ItemFn) -> syn::Result<TokenStream> {
    if let Some(raw_span) = args.raw {
        return Err(syn::Error::new(
            raw_span,
            "`#[js_export(raw)]` cannot be used on `async fn`",
        ));
    }
    reject_non_lifetime_generics(&function.sig.generics, "`async fn` exports")?;

    let js_name = resolve_fn_export_name(args, &function.sig.ident)?;
    let export_section = args
        .export_link_section()
        .unwrap_or_else(|| EXPORT_MARSHAL_SECTION.to_owned());
    let rt = rt_support_path(args.crate_path.as_ref());
    let prepared = prepare_wrapper_params(function.sig.inputs.iter(), &rt, ParamMode::Async, None)?;
    let fn_ident = &function.sig.ident;
    let ret_ty: Type = match &function.sig.output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, ty) => (**ty).clone(),
    };
    let wrapper_params = &prepared.wrapper_params;
    let call_args = &prepared.call_args;
    let docs = doc_attrs(&function.attrs);

    Ok(quote! {
        #function

        #[allow(unexpected_cfgs, reason = "mires target cfg is supplied by cargo-mires")]
        const _: () = {
            #[cfg(mires)]
            const _: () = {
                #[unsafe(link_section = #EXPORT_OUT_SECTION)]
                #[inline(never)]
                fn __mires_export_out(value: #ret_ty) -> #rt::JsValue {
                    let _ = ::core::mem::ManuallyDrop::new(value);
                    #rt::JsValue::undefined()
                }

                #(#docs)*
                #[unsafe(export_name = #js_name)]
                #[unsafe(link_section = #export_section)]
                fn __mires_export(#(#wrapper_params),*) -> #rt::TypedPromise<#ret_ty> {
                    #rt::TypedPromise::from_future(async move {
                        ::core::result::Result::Ok(__mires_export_out(
                            #fn_ident(#(#call_args),*).await,
                        ))
                    })
                }
            };
        };
    })
}

/// Expands `#[js_export]` on a `struct` definition (pass-through marker for class symmetry).
fn expand_export_struct(args: &ExportAttr, item_struct: ItemStruct) -> syn::Result<TokenStream> {
    if let Some(raw_span) = args.raw {
        return Err(syn::Error::new(
            raw_span,
            "`#[js_export(raw)]` cannot be used on `struct` definitions",
        ));
    }
    if let Some((chunk_span, _)) = &args.chunk {
        return Err(syn::Error::new(
            *chunk_span,
            "`chunk` is not supported on `#[js_export]` structs",
        ));
    }
    if let Some((rename_span, _)) = &args.rename_all {
        return Err(syn::Error::new(
            *rename_span,
            "`rename_all` is not supported on `#[js_export]` structs",
        ));
    }
    if args.name.is_none() {
        validate_js_export_ident(&item_struct.ident.to_string(), item_struct.ident.span())?;
    }
    Ok(quote! { #item_struct })
}

/// How an exported `impl` method returns a newly boxed class handle, if at all.
enum CtorReturn {
    Direct,
    Fallible(Box<Type>),
}

/// Classification of an exported `pub fn` inside a `#[js_export] impl` block.
enum ClassMethodKind {
    Ctor(CtorReturn),
    Ref,
    Mut,
    Owned,
    AsyncOwned,
    StaticCtor(CtorReturn),
    Static,
    AsyncStatic,
}

/// Shared class-level context threaded through method wrapper generation.
struct ClassContext<'a> {
    class_name: &'a str,
    self_ident: &'a Ident,
    self_ty: &'a Type,
    rt: &'a TokenStream,
    rename_all: Option<RenameRule>,
}

/// Expands `#[js_export]` on an inherent `impl` block into a JS class wrapper table on `cfg(mires)`.
fn expand_export_impl(args: &ExportAttr, mut item_impl: ItemImpl) -> syn::Result<TokenStream> {
    if let Some(raw_span) = args.raw {
        return Err(syn::Error::new(
            raw_span,
            "`#[js_export(raw)]` cannot be used on `impl` blocks",
        ));
    }
    if let Some((chunk_span, _)) = &args.chunk {
        return Err(syn::Error::new(
            *chunk_span,
            "`chunk` is not supported on `#[js_export] impl` blocks",
        ));
    }
    if item_impl.trait_.is_some() {
        return Err(syn::Error::new(
            item_impl.span(),
            "`#[js_export]` only supports inherent `impl` blocks, not trait implementations",
        ));
    }
    if !item_impl.generics.params.is_empty() || item_impl.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            item_impl.generics.span(),
            "`#[js_export]` does not support generic `impl` blocks",
        ));
    }

    let self_ty = item_impl.self_ty.as_ref().clone();
    let self_ident = extract_impl_self_ident(&self_ty)?;
    let class_name = match &args.name {
        Some(name) => name.clone(),
        None => {
            let name = self_ident.to_string();
            validate_js_export_ident(&name, self_ident.span())?;
            name
        }
    };

    let rt = rt_support_path(args.crate_path.as_ref());
    let ctx = ClassContext {
        class_name: &class_name,
        self_ident: &self_ident,
        self_ty: &self_ty,
        rt: &rt,
        rename_all: args.rename_all.map(|(_, rule)| rule),
    };
    let mut seen_exports = BTreeSet::new();
    let drop_export_name = format!("__mires_cls${class_name}$__drop");
    seen_exports.insert(drop_export_name.clone());

    let mut method_wrappers = Vec::new();
    for (idx, item) in item_impl.items.iter_mut().enumerate() {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        let method_attr = take_method_export_attr(&mut method.attrs)?;
        if !matches!(method.vis, Visibility::Public(_)) {
            if let Some((span, _)) = method_attr {
                return Err(syn::Error::new(
                    span,
                    "cannot apply `#[js_export]` to a non-`pub` method; mark the method `pub` or \
                     remove `#[js_export]`",
                ));
            }
            continue;
        }
        if method_attr
            .as_ref()
            .is_some_and(|(_, attr)| attr.skip.is_some())
        {
            continue;
        }
        let wrapper = build_class_method_wrapper(
            idx,
            method,
            method_attr.and_then(|(_, attr)| attr.name),
            &ctx,
            &mut seen_exports,
        )?;
        method_wrappers.push(wrapper);
    }

    Ok(quote! {
        #item_impl

        #[allow(unexpected_cfgs, reason = "mires target cfg is supplied by cargo-mires")]
        const _: () = {
            #[cfg(mires)]
            const _: () = {
                #[unsafe(export_name = #drop_export_name)]
                #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
                fn __mires_cls_drop(__mires_self: *mut #self_ty) {
                    #rt::class_handle::drop_raw(__mires_self);
                }

                #(#method_wrappers)*
            };
        };
    })
}

/// Extracts the concrete struct identifier from `impl MyType`.
fn extract_impl_self_ident(self_ty: &Type) -> syn::Result<Ident> {
    let Type::Path(type_path) = unwrap_type_group(self_ty) else {
        return Err(syn::Error::new(
            self_ty.span(),
            "`#[js_export]` requires an inherent `impl` on a concrete named struct type",
        ));
    };
    if type_path.qself.is_some() {
        return Err(syn::Error::new(
            self_ty.span(),
            "`#[js_export]` requires an inherent `impl` on a concrete named struct type",
        ));
    }
    let Some(last) = type_path.path.segments.last() else {
        return Err(syn::Error::new(
            self_ty.span(),
            "`#[js_export]` requires an inherent `impl` on a concrete named struct type",
        ));
    };
    if !matches!(last.arguments, PathArguments::None) {
        return Err(syn::Error::new(
            last.arguments.span(),
            "`#[js_export]` does not support generic struct types on `impl` blocks",
        ));
    }
    Ok(last.ident.clone())
}

/// Removes and parses an optional method-level `#[js_export(..)]` attribute from `attrs`.
fn take_method_export_attr(
    attrs: &mut Vec<Attribute>,
) -> syn::Result<Option<(Span, MethodExportAttr)>> {
    let mut found: Option<(Span, MethodExportAttr)> = None;
    let mut kept = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        let is_js_export = attr
            .path()
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "js_export");
        if !is_js_export {
            kept.push(attr);
            continue;
        }
        if found.is_some() {
            return Err(syn::Error::new(
                attr.span(),
                "duplicate `#[js_export]` attribute on method",
            ));
        }
        let parsed = match &attr.meta {
            syn::Meta::Path(_) => MethodExportAttr {
                name: None,
                skip: None,
            },
            _ => attr.parse_args::<MethodExportAttr>()?,
        };
        found = Some((attr.span(), parsed));
    }
    *attrs = kept;
    Ok(found)
}

/// Classifies a class method by its receiver, `async` modifier, and return type.
fn classify_class_method(
    method: &ImplItemFn,
    js_method: &str,
    ctx: &ClassContext<'_>,
) -> syn::Result<(ClassMethodKind, usize)> {
    match method.sig.inputs.first() {
        Some(FnArg::Receiver(recv)) => {
            if recv.colon_token.is_some() {
                return Err(syn::Error::new(
                    recv.span(),
                    "`#[js_export]` class methods only support `&self`, `&mut self`, or `self` \
                     receivers",
                ));
            }
            let kind = match (&recv.reference, &recv.mutability, method.sig.asyncness) {
                (Some(_), _, Some(async_kw)) => {
                    return Err(syn::Error::new(
                        async_kw.span(),
                        "`async` methods taking `&self` or `&mut self` inside `#[js_export] impl` \
                         are not supported because a borrowed receiver cannot safely cross an \
                         `.await` while JavaScript may call `.free()`; take `self` by value or \
                         use a static `async fn` instead",
                    ));
                }
                (Some(_), None, None) => ClassMethodKind::Ref,
                (Some(_), Some(_), None) => ClassMethodKind::Mut,
                (None, _, None) => ClassMethodKind::Owned,
                (None, _, Some(_)) => ClassMethodKind::AsyncOwned,
            };
            Ok((kind, 1usize))
        }
        _ => {
            if let Some(async_kw) = method.sig.asyncness {
                if js_method == "new" {
                    return Err(syn::Error::new(
                        async_kw.span(),
                        "`async` constructor `new` inside `#[js_export] impl` is not supported; \
                         use a static `async fn` factory method instead",
                    ));
                }
                return Ok((ClassMethodKind::AsyncStatic, 0usize));
            }
            let ctor_ret = classify_ctor_return(&method.sig.output, ctx.self_ident, ctx.self_ty);
            let kind = match (ctor_ret, js_method) {
                (Some(ctor), "new") => ClassMethodKind::Ctor(ctor),
                (Some(ctor), _) => ClassMethodKind::StaticCtor(ctor),
                (None, _) => ClassMethodKind::Static,
            };
            Ok((kind, 0usize))
        }
    }
}

/// Builds one `cfg(mires)` exported wrapper function for a `pub fn` in a `#[js_export] impl` block.
fn build_class_method_wrapper(
    idx: usize,
    method: &ImplItemFn,
    js_name_override: Option<String>,
    ctx: &ClassContext<'_>,
    seen_exports: &mut BTreeSet<String>,
) -> syn::Result<TokenStream> {
    reject_non_lifetime_generics(&method.sig.generics, "methods inside `#[js_export] impl`")?;

    let method_ident = &method.sig.ident;
    let js_method = match js_name_override {
        Some(name) => name,
        None => {
            let raw_name = method_ident.to_string();
            let name = match ctx.rename_all {
                Some(rule) if raw_name != "new" => rule.apply(&raw_name),
                _ => raw_name,
            };
            validate_js_method_ident(&name, method_ident.span())?;
            name
        }
    };

    let (kind, params_iter_skip) = classify_class_method(method, &js_method, ctx)?;
    let class_name = ctx.class_name;
    let export_name = match &kind {
        ClassMethodKind::Ctor(_) => format!("__mires_cls${class_name}$new"),
        ClassMethodKind::Ref | ClassMethodKind::Mut => {
            format!("__mires_cls${class_name}${js_method}")
        }
        ClassMethodKind::Owned | ClassMethodKind::AsyncOwned => {
            format!("__mires_cls${class_name}$__take${js_method}")
        }
        ClassMethodKind::StaticCtor(_) => {
            format!("__mires_cls${class_name}$__static_ctor${js_method}")
        }
        ClassMethodKind::Static | ClassMethodKind::AsyncStatic => {
            format!("__mires_cls${class_name}$__static${js_method}")
        }
    };
    if !seen_exports.insert(export_name.clone()) {
        return Err(syn::Error::new(
            method_ident.span(),
            format!("duplicate exported JS method `{js_method}` on class `{class_name}`"),
        ));
    }

    let param_mode = if matches!(
        kind,
        ClassMethodKind::AsyncOwned | ClassMethodKind::AsyncStatic
    ) {
        ParamMode::Async
    } else {
        ParamMode::Sync
    };
    let prepared = prepare_wrapper_params(
        method.sig.inputs.iter().skip(params_iter_skip),
        ctx.rt,
        param_mode,
        Some(ctx.self_ty),
    )?;
    let wrapper_ident = format_ident!("__mires_cls_m_{idx}");
    let self_ty = ctx.self_ty;
    let rt = ctx.rt;
    let generics = &method.sig.generics;
    let where_clause = &method.sig.generics.where_clause;
    let wrapper_params = &prepared.wrapper_params;
    let call_args = &prepared.call_args;
    let sub_output = substitute_self_return(&method.sig.output, self_ty);
    let docs = doc_attrs(&method.attrs);

    let tokens = match kind {
        ClassMethodKind::Ctor(CtorReturn::Direct)
        | ClassMethodKind::StaticCtor(CtorReturn::Direct) => quote! {
            #(#docs)*
            #[unsafe(export_name = #export_name)]
            #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
            fn #wrapper_ident #generics (#(#wrapper_params),*) -> *mut #self_ty #where_clause {
                #rt::class_handle::into_raw(<#self_ty>::#method_ident(#(#call_args),*))
            }
        },
        ClassMethodKind::Ctor(CtorReturn::Fallible(ret_ty))
        | ClassMethodKind::StaticCtor(CtorReturn::Fallible(ret_ty)) => quote! {
            #(#docs)*
            #[unsafe(export_name = #export_name)]
            #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
            fn #wrapper_ident #generics (#(#wrapper_params),*) -> #ret_ty #where_clause {
                ::core::result::Result::map(
                    <#self_ty>::#method_ident(#(#call_args),*),
                    #rt::class_handle::into_raw
                )
            }
        },
        ClassMethodKind::Ref => quote! {
            #(#docs)*
            #[unsafe(export_name = #export_name)]
            #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
            fn #wrapper_ident #generics (
                __mires_self: *mut #self_ty #(, #wrapper_params)*
            ) #sub_output #where_clause {
                #rt::class_handle::with_ref(__mires_self, |__mires_this| {
                    <#self_ty>::#method_ident(__mires_this #(, #call_args)*)
                })
            }
        },
        ClassMethodKind::Mut => quote! {
            #(#docs)*
            #[unsafe(export_name = #export_name)]
            #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
            fn #wrapper_ident #generics (
                __mires_self: *mut #self_ty #(, #wrapper_params)*
            ) #sub_output #where_clause {
                #rt::class_handle::with_mut(__mires_self, |__mires_this| {
                    <#self_ty>::#method_ident(__mires_this #(, #call_args)*)
                })
            }
        },
        ClassMethodKind::Owned => quote! {
            #(#docs)*
            #[unsafe(export_name = #export_name)]
            #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
            fn #wrapper_ident #generics (
                __mires_self: *mut #self_ty #(, #wrapper_params)*
            ) #sub_output #where_clause {
                <#self_ty>::#method_ident(
                    #rt::class_handle::into_owned(__mires_self) #(, #call_args)*
                )
            }
        },
        ClassMethodKind::Static => quote! {
            #(#docs)*
            #[unsafe(export_name = #export_name)]
            #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
            fn #wrapper_ident #generics (#(#wrapper_params),*) #sub_output #where_clause {
                <#self_ty>::#method_ident(#(#call_args),*)
            }
        },
        ClassMethodKind::AsyncOwned | ClassMethodKind::AsyncStatic => {
            let out_ident = format_ident!("__mires_cls_m_{idx}_out");
            let ret_ty: Type = match &method.sig.output {
                ReturnType::Default => syn::parse_quote!(()),
                ReturnType::Type(_, ty) => substitute_self_type(ty, self_ty),
            };
            if matches!(kind, ClassMethodKind::AsyncOwned) {
                quote! {
                    #[unsafe(link_section = #EXPORT_OUT_SECTION)]
                    #[inline(never)]
                    fn #out_ident(value: #ret_ty) -> #rt::JsValue {
                        let _ = ::core::mem::ManuallyDrop::new(value);
                        #rt::JsValue::undefined()
                    }

                    #(#docs)*
                    #[unsafe(export_name = #export_name)]
                    #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
                    fn #wrapper_ident(
                        __mires_self: *mut #self_ty #(, #wrapper_params)*
                    ) -> #rt::TypedPromise<#ret_ty> {
                        let __mires_owned = #rt::class_handle::into_owned(__mires_self);
                        #rt::TypedPromise::from_future(async move {
                            ::core::result::Result::Ok(#out_ident(
                                <#self_ty>::#method_ident(__mires_owned #(, #call_args)*).await,
                            ))
                        })
                    }
                }
            } else {
                quote! {
                    #[unsafe(link_section = #EXPORT_OUT_SECTION)]
                    #[inline(never)]
                    fn #out_ident(value: #ret_ty) -> #rt::JsValue {
                        let _ = ::core::mem::ManuallyDrop::new(value);
                        #rt::JsValue::undefined()
                    }

                    #(#docs)*
                    #[unsafe(export_name = #export_name)]
                    #[unsafe(link_section = #EXPORT_MARSHAL_SECTION)]
                    fn #wrapper_ident(#(#wrapper_params),*) -> #rt::TypedPromise<#ret_ty> {
                        #rt::TypedPromise::from_future(async move {
                            ::core::result::Result::Ok(#out_ident(
                                <#self_ty>::#method_ident(#(#call_args),*).await,
                            ))
                        })
                    }
                }
            }
        }
    };
    Ok(tokens)
}

/// Whether `ty` names `Self` or the concrete `impl` target struct `self_ident`.
fn is_self_type(ty: &Type, self_ident: &Ident) -> bool {
    let Type::Path(type_path) = unwrap_type_group(ty) else {
        return false;
    };
    if type_path.qself.is_some() {
        return false;
    }
    let Some(last) = type_path.path.segments.last() else {
        return false;
    };
    if !matches!(last.arguments, PathArguments::None) {
        return false;
    }
    (type_path.path.segments.len() == 1 && last.ident == "Self") || last.ident == *self_ident
}

/// Recognizes `Self` / `#self_ty` or `Result<Self / #self_ty, E>` return types for constructors.
fn classify_ctor_return(
    output: &ReturnType,
    self_ident: &Ident,
    self_ty: &Type,
) -> Option<CtorReturn> {
    let ReturnType::Type(_, ty) = output else {
        return None;
    };
    if is_self_type(ty, self_ident) {
        return Some(CtorReturn::Direct);
    }
    let Type::Path(type_path) = unwrap_type_group(ty) else {
        return None;
    };
    if type_path.qself.is_some() {
        return None;
    }
    let last = type_path.path.segments.last()?;
    if last.ident != "Result" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    let mut type_args = args.args.iter().filter_map(|arg| match arg {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    });
    let (Some(ok_ty), Some(_err_ty), None) = (type_args.next(), type_args.next(), type_args.next())
    else {
        return None;
    };
    if !is_self_type(ok_ty, self_ident) {
        return None;
    }
    let Type::Path(mut boxed_result) = substitute_self_type(ty, self_ty) else {
        return None;
    };
    let last_mut = boxed_result.path.segments.last_mut()?;
    let PathArguments::AngleBracketed(args_mut) = &mut last_mut.arguments else {
        return None;
    };
    for arg in &mut args_mut.args {
        if let GenericArgument::Type(ok_slot) = arg {
            *ok_slot = syn::parse_quote!(*mut #self_ty);
            break;
        }
    }
    Some(CtorReturn::Fallible(Box::new(Type::Path(boxed_result))))
}

/// Replaces occurrences of `Self` in a return type with `self_ty`.
fn substitute_self_return(output: &ReturnType, self_ty: &Type) -> ReturnType {
    match output {
        ReturnType::Default => ReturnType::Default,
        ReturnType::Type(arrow, ty) => {
            ReturnType::Type(*arrow, Box::new(substitute_self_type(ty, self_ty)))
        }
    }
}

/// Recursively substitutes standalone `Self` type paths with `self_ty` for out-of-`impl` wrappers.
fn substitute_self_type(ty: &Type, self_ty: &Type) -> Type {
    match ty {
        Type::Path(type_path) => {
            if type_path.qself.is_none() && type_path.path.is_ident("Self") {
                return self_ty.clone();
            }
            let mut cloned = type_path.clone();
            if let Some(qself) = &mut cloned.qself {
                *qself.ty = substitute_self_type(&qself.ty, self_ty);
            }
            for segment in &mut cloned.path.segments {
                match &mut segment.arguments {
                    PathArguments::None => {}
                    PathArguments::AngleBracketed(ab) => {
                        for arg in &mut ab.args {
                            match arg {
                                GenericArgument::Type(inner) => {
                                    *inner = substitute_self_type(inner, self_ty);
                                }
                                GenericArgument::AssocType(assoc) => {
                                    assoc.ty = substitute_self_type(&assoc.ty, self_ty);
                                }
                                _ => {}
                            }
                        }
                    }
                    PathArguments::Parenthesized(paren) => {
                        for input in &mut paren.inputs {
                            *input = substitute_self_type(input, self_ty);
                        }
                        paren.output = substitute_self_return(&paren.output, self_ty);
                    }
                }
            }
            Type::Path(cloned)
        }
        Type::Reference(reference) => {
            let mut cloned = reference.clone();
            *cloned.elem = substitute_self_type(&cloned.elem, self_ty);
            Type::Reference(cloned)
        }
        Type::Ptr(ptr) => {
            let mut cloned = ptr.clone();
            *cloned.elem = substitute_self_type(&cloned.elem, self_ty);
            Type::Ptr(cloned)
        }
        Type::Slice(slice) => {
            let mut cloned = slice.clone();
            *cloned.elem = substitute_self_type(&cloned.elem, self_ty);
            Type::Slice(cloned)
        }
        Type::Array(array) => {
            let mut cloned = array.clone();
            *cloned.elem = substitute_self_type(&cloned.elem, self_ty);
            Type::Array(cloned)
        }
        Type::Tuple(tuple) => {
            let mut cloned = tuple.clone();
            for elem in &mut cloned.elems {
                *elem = substitute_self_type(elem, self_ty);
            }
            Type::Tuple(cloned)
        }
        Type::Paren(paren) => {
            let mut cloned = paren.clone();
            *cloned.elem = substitute_self_type(&cloned.elem, self_ty);
            Type::Paren(cloned)
        }
        Type::Group(group) => {
            let mut cloned = group.clone();
            *cloned.elem = substitute_self_type(&cloned.elem, self_ty);
            Type::Group(cloned)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_str(attr: &str, item: &str) -> Result<String, String> {
        let attr_ts: TokenStream = attr.parse().expect("attr tokens parse");
        let item_ts: TokenStream = item.parse().expect("item tokens parse");
        expand_export(attr_ts, item_ts)
            .map(|tokens| tokens.to_string())
            .map_err(|err| err.to_string())
    }

    #[test]
    fn async_fn_emits_typed_promise_and_out_section() {
        let expanded = expand_str(
            "",
            "pub async fn fetch_count(url: &str, retries: u32) -> u32 { let _ = (url, retries); 1 }",
        )
        .unwrap();
        assert!(expanded.contains("pub async fn fetch_count"), "{expanded}");
        assert!(
            expanded.contains(r#"unsafe (link_section = "rcgjs.export.out.v1")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains(r#"unsafe (export_name = "fetch_count")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains(r#"unsafe (link_section = "rcgjs.export.v1")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("url : :: mires :: __export :: String"),
            "{expanded}"
        );
        assert!(expanded.contains("url . as_str ()"), "{expanded}");
        assert!(
            expanded.contains("-> :: mires :: __export :: TypedPromise < u32 >"),
            "{expanded}"
        );
        assert!(
            expanded.contains(":: mires :: __export :: TypedPromise :: from_future"),
            "{expanded}"
        );
        assert!(
            expanded.contains(":: core :: mem :: ManuallyDrop :: new (value)"),
            "{expanded}"
        );
    }

    #[test]
    fn async_fn_handles_unit_result_static_str_name_and_crate_override() {
        let unit_expanded = expand_str(r#"name = "pingNow""#, "pub async fn ping() {}").unwrap();
        assert!(
            unit_expanded.contains(r#"unsafe (export_name = "pingNow")"#),
            "{unit_expanded}"
        );
        assert!(
            unit_expanded.contains("TypedPromise < () >"),
            "{unit_expanded}"
        );

        let res_expanded = expand_str(
            r#"name = "loadUser", crate = mires_js"#,
            "pub async fn load(tag: &'static str, scores: HashMap<String, i32>) -> Result<String, JsError> { todo!() }",
        )
        .unwrap();
        assert!(
            res_expanded.contains("tag : & 'static str"),
            "{res_expanded}"
        );
        assert!(
            res_expanded.contains("scores : mires_js :: __export :: JsMapEntries < String , i32 >"),
            "{res_expanded}"
        );
        assert!(
            res_expanded.contains(
                "-> mires_js :: __export :: TypedPromise < Result < String , JsError > >"
            ),
            "{res_expanded}"
        );
    }

    #[test]
    fn async_fn_rejects_raw_and_type_generics() {
        let raw_err = expand_str("raw", "pub async fn fetch() -> u32 { 0 }").unwrap_err();
        assert!(
            raw_err.contains("cannot be used on `async fn`"),
            "{raw_err}"
        );

        let gen_err = expand_str("", "pub async fn fetch<T>(x: T) -> T { x }").unwrap_err();
        assert!(gen_err.contains("generic"), "{gen_err}");
    }

    #[test]
    fn sync_fn_with_hashmap_and_btreemap_emits_entry_carriers() {
        let expanded = expand_str(
            r#"name = "mergeMaps""#,
            "pub fn merge(a: std::collections::HashMap<String, i32>, b: alloc::collections::BTreeMap<String, u64>, scale: i32) -> i32 { 0 }",
        )
        .unwrap();
        assert!(expanded.contains("pub fn merge"), "{expanded}");
        assert!(
            expanded.contains(r#"unsafe (export_name = "mergeMaps")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("a : :: mires :: __export :: JsMapEntries < String , i32 >"),
            "{expanded}"
        );
        assert!(
            expanded.contains("b : :: mires :: __export :: JsBTreeMapEntries < String , u64 >"),
            "{expanded}"
        );
        assert!(expanded.contains("scale : i32"), "{expanded}");
        assert!(
            expanded.contains(":: core :: iter :: FromIterator :: from_iter (a . 0)"),
            "{expanded}"
        );
        assert!(
            expanded.contains(":: core :: iter :: FromIterator :: from_iter (b . 0)"),
            "{expanded}"
        );
    }

    #[test]
    fn sync_fn_raw_or_three_arg_hashmap_skips_map_entry_carrier() {
        let raw = expand_str("raw", "pub fn take_raw(m: HashMap<String, i32>) {}").unwrap();
        assert!(!raw.contains("JsMapEntries"), "{raw}");
        assert!(raw.contains("m : HashMap < String , i32 >"), "{raw}");

        let custom = expand_str("", "pub fn custom(m: HashMap<String, i32, MyHasher>) {}").unwrap();
        assert!(!custom.contains("JsMapEntries"), "{custom}");
        assert!(
            custom.contains("m : HashMap < String , i32 , MyHasher >"),
            "{custom}"
        );
    }

    #[test]
    fn struct_export_passes_through_and_rejects_raw() {
        let plain = expand_str("", "pub struct Counter { value: i32 }").unwrap();
        assert_eq!(plain, "pub struct Counter { value : i32 }");

        let renamed =
            expand_str(r#"name = "JsCounter""#, "pub struct Counter { value: i32 }").unwrap();
        assert_eq!(renamed, "pub struct Counter { value : i32 }");

        let err = expand_str("raw", "pub struct Counter;").unwrap_err();
        assert!(err.contains("cannot be used on `struct`"), "{err}");
    }

    #[test]
    fn impl_export_emits_drop_ctor_methods_static_and_skips_private() {
        let expanded = expand_str(
            r#"name = "JsCounter", crate = mires_js"#,
            r#"
            impl Counter {
                pub fn new(start: i32) -> Self { Self { value: start } }
                pub fn from_checked(start: i32) -> Result<Self, JsError> { Ok(Self { value: start }) }
                #[js_export(name = "getValue")]
                pub fn value(&self) -> i32 { self.value }
                pub fn add(&mut self, delta: i32, weights: HashMap<String, i32>) -> i32 { self.value }
                pub fn into_inner(self) -> i32 { self.value }
                pub fn version() -> u32 { 1 }
                pub fn duplicate(&self) -> Self { Self { value: self.value } }
                fn private_helper(&self) -> i32 { self.value }
                pub(crate) fn crate_helper(&self) -> i32 { self.value }
            }
            "#,
        )
        .unwrap();

        // Method-level #[js_export] must be stripped from the emitted original impl.
        assert!(!expanded.contains("# [js_export"), "{expanded}");

        // Destructor.
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$JsCounter$__drop")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("mires_js :: __export :: class_handle :: drop_raw (__mires_self)"),
            "{expanded}"
        );

        // Constructor `new` -> direct `into_raw`.
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$JsCounter$new")"#),
            "{expanded}"
        );
        assert!(expanded.contains("-> * mut Counter"), "{expanded}");
        assert!(
            expanded.contains(
                "mires_js :: __export :: class_handle :: into_raw (< Counter > :: new (start))"
            ),
            "{expanded}"
        );

        // Static fallible constructor `from_checked`.
        assert!(
            expanded.contains(
                r#"unsafe (export_name = "__mires_cls$JsCounter$__static_ctor$from_checked")"#
            ),
            "{expanded}"
        );
        assert!(
            expanded.contains("-> Result < * mut Counter , JsError >"),
            "{expanded}"
        );
        assert!(
            expanded.contains(":: core :: result :: Result :: map (< Counter > :: from_checked (start) , mires_js :: __export :: class_handle :: into_raw)"),
            "{expanded}"
        );

        // Renamed `&self` method.
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$JsCounter$getValue")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("mires_js :: __export :: class_handle :: with_ref"),
            "{expanded}"
        );

        // `&mut self` method with `HashMap` param.
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$JsCounter$add")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("weights : mires_js :: __export :: JsMapEntries < String , i32 >"),
            "{expanded}"
        );
        assert!(
            expanded.contains("mires_js :: __export :: class_handle :: with_mut"),
            "{expanded}"
        );

        // Consuming `self` method.
        assert!(
            expanded
                .contains(r#"unsafe (export_name = "__mires_cls$JsCounter$__take$into_inner")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("mires_js :: __export :: class_handle :: into_owned (__mires_self)"),
            "{expanded}"
        );

        // Static non-ctor method.
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$JsCounter$__static$version")"#),
            "{expanded}"
        );

        // Instance method returning `Self` substitutes `Counter` out-of-impl.
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$JsCounter$duplicate")"#),
            "{expanded}"
        );
        assert!(expanded.contains(") -> Counter {"), "{expanded}");

        // Private helpers are NOT exported.
        assert!(!expanded.contains("JsCounter$private_helper"), "{expanded}");
        assert!(!expanded.contains("JsCounter$crate_helper"), "{expanded}");
    }

    #[test]
    fn impl_export_fallible_new_constructor() {
        let expanded = expand_str(
            "",
            "impl Parser { pub fn new(src: &str) -> Result<Self, JsError> { todo!() } }",
        )
        .unwrap();
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$Parser$new")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("-> Result < * mut Parser , JsError >"),
            "{expanded}"
        );
    }

    #[test]
    fn impl_export_rejects_invalid_forms() {
        for (attr, item, expected) in [
            ("raw", "impl Foo {}", "cannot be used on `impl`"),
            (
                "",
                "impl Default for Foo { fn default() -> Self { todo!() } }",
                "inherent `impl` blocks",
            ),
            ("", "impl<T> Foo<T> {}", "generic `impl` blocks"),
            ("", "impl Foo<i32> {}", "generic struct types"),
            (
                "",
                "impl Foo { #[js_export(name = \"x\")] fn hidden(&self) {} }",
                "non-`pub` method",
            ),
            (
                "",
                "impl Foo { pub async fn run(&self) {} }",
                "borrowed receiver cannot safely cross an `.await` while JavaScript may call `.free()`",
            ),
            (
                "",
                "impl Foo { pub async fn run_mut(&mut self) {} }",
                "borrowed receiver cannot safely cross an `.await` while JavaScript may call `.free()`",
            ),
            (
                "",
                "impl Foo { pub async fn new() -> Self { todo!() } }",
                "`async` constructor `new`",
            ),
            (
                r#"chunk = "island""#,
                "pub struct Foo;",
                "`chunk` is not supported on `#[js_export]` structs",
            ),
            (
                r#"chunk = "island""#,
                "impl Foo {}",
                "`chunk` is not supported on `#[js_export] impl` blocks",
            ),
            (
                r#"rename_all = "camelCase""#,
                "pub struct Foo;",
                "`rename_all` is not supported on `#[js_export]` structs",
            ),
            (
                "skip",
                "pub fn f() {}",
                "`skip` can only be used on methods inside a `#[js_export] impl` block",
            ),
            (
                "",
                r#"impl Foo { #[js_export(skip, name = "x")] pub fn a(&self) {} }"#,
                "`skip` cannot be combined with `name`",
            ),
            (
                "",
                "impl Foo { #[js_export(skip)] fn private_skip(&self) {} }",
                "non-`pub` method",
            ),
            (
                "",
                r#"impl Foo { #[js_export(chunk = "x")] pub fn a(&self) {} }"#,
                "`chunk` is not supported on methods",
            ),
            (
                "",
                r#"impl Foo { #[js_export(rename_all = "camelCase")] pub fn a(&self) {} }"#,
                "`rename_all` must be placed on the `#[js_export] impl` block",
            ),
            (
                "",
                "impl Foo { pub fn a(&self) {} #[js_export(name = \"a\")] pub fn b(&self) {} }",
                "duplicate exported JS method",
            ),
            (
                "",
                "impl Foo { pub fn boxed(self: Box<Self>) {} }",
                "only support `&self`, `&mut self`, or `self`",
            ),
            (
                "",
                "pub enum Kind { A, B }",
                "only be applied to functions, structs, or inherent `impl`",
            ),
        ] {
            let err = expand_str(attr, item).unwrap_err();
            assert!(
                err.contains(expected),
                "expected `{expected}` for `{item}`, got `{err}`"
            );
        }
    }

    #[test]
    fn chunk_and_role_encode_sections_on_sync_async_map_and_raw_fns() {
        let default_role = expand_str(r#"chunk = "counter""#, "pub fn render() {}").unwrap();
        assert!(
            default_role
                .contains(r#"unsafe (link_section = "rcgjs.export.v1.chunk.main.counter")"#),
            "{default_role}"
        );

        let island = expand_str(
            r#"chunk = "counter-1", role = "island", rename_all = "camelCase""#,
            "pub fn render_counter() {}",
        )
        .unwrap();
        assert!(
            island.contains(r#"unsafe (export_name = "renderCounter")"#),
            "{island}"
        );
        assert!(
            island.contains(r#"unsafe (link_section = "rcgjs.export.v1.chunk.island.counter-1")"#),
            "{island}"
        );

        let raw_worker = expand_str(
            r#"raw, chunk = "worker_a", role = "worker""#,
            "pub fn handle_msg() {}",
        )
        .unwrap();
        assert!(
            raw_worker.contains(r#"unsafe (link_section = "rcgjs.chunk.v1.worker.worker_a")"#),
            "{raw_worker}"
        );
        assert!(!raw_worker.contains("rcgjs.export.v1"), "{raw_worker}");

        let async_proc = expand_str(
            r#"chunk = "rpc$1", role = "procedure", rename_all = "camelCase""#,
            "pub async fn fetch_user(id: u32) -> String { String::new() }",
        )
        .unwrap();
        assert!(
            async_proc.contains(r#"unsafe (link_section = "rcgjs.export.out.v1")"#),
            "{async_proc}"
        );
        assert!(
            async_proc.contains(r#"unsafe (export_name = "fetchUser")"#),
            "{async_proc}"
        );
        assert!(
            async_proc
                .contains(r#"unsafe (link_section = "rcgjs.export.v1.chunk.procedure.rpc$1")"#),
            "{async_proc}"
        );

        let map_chunk = expand_str(
            r#"chunk = "scores", role = "island", rename_all = "camelCase""#,
            "pub fn sum_scores(scores: BTreeMap<String, i32>) -> i32 { 0 }",
        )
        .unwrap();
        assert!(
            map_chunk.contains(r#"unsafe (export_name = "sumScores")"#),
            "{map_chunk}"
        );
        assert!(
            map_chunk.contains(r#"unsafe (link_section = "rcgjs.export.v1.chunk.island.scores")"#),
            "{map_chunk}"
        );
    }

    #[test]
    fn chunk_role_and_rename_all_reject_invalid_forms() {
        for (attr, item, expected) in [
            (r#"role = "island""#, "pub fn f() {}", "requires `chunk"),
            (r#"chunk = """#, "pub fn f() {}", "not a valid chunk name"),
            (
                r#"chunk = "a.b""#,
                "pub fn f() {}",
                "not a valid chunk name",
            ),
            (
                r#"chunk = "a/b""#,
                "pub fn f() {}",
                "not a valid chunk name",
            ),
            (
                r#"chunk = "a", role = "bogus""#,
                "pub fn f() {}",
                "unknown `#[js_export]` role `bogus`",
            ),
            (
                r#"chunk = "a", chunk = "b""#,
                "pub fn f() {}",
                "duplicate `chunk`",
            ),
            (
                r#"chunk = "a", role = "main", role = "island""#,
                "pub fn f() {}",
                "duplicate `role`",
            ),
            (
                r#"rename_all = "PascalCase""#,
                "pub fn f() {}",
                "unknown `rename_all` convention `PascalCase`",
            ),
            (
                r#"rename_all = "camelCase", name = "foo""#,
                "pub fn render_it() {}",
                "cannot be combined with explicit `name` on a free function",
            ),
            (
                r#"raw, chunk = "x""#,
                "pub async fn f() {}",
                "cannot be used on `async fn`",
            ),
        ] {
            let err = expand_str(attr, item).unwrap_err();
            assert!(
                err.contains(expected),
                "expected `{expected}` for `#[js_export({attr})]`, got `{err}`"
            );
        }
    }

    #[test]
    fn to_camel_case_handles_snake_leading_and_trailing_underscores() {
        assert_eq!(to_camel_case("render_markdown"), "renderMarkdown");
        assert_eq!(to_camel_case("get_2d_context"), "get2dContext");
        assert_eq!(to_camel_case("value"), "value");
        assert_eq!(to_camel_case("new"), "new");
        assert_eq!(to_camel_case("foo__bar"), "fooBar");
        assert_eq!(to_camel_case("_private_helper"), "_privateHelper");
        assert_eq!(to_camel_case("default_"), "default_");
        assert_eq!(to_camel_case("___"), "___");
    }

    #[test]
    fn impl_export_supports_rename_all_skip_and_safe_async_methods() {
        let expanded = expand_str(
            r#"rename_all = "camelCase", crate = mires_js"#,
            r#"
            impl Session {
                pub fn new(id: u32) -> Self { Self { id } }
                pub fn from_token(tok: &str) -> Result<Self, JsError> { todo!() }
                pub fn current_id(&self) -> u32 { self.id }
                #[js_export(name = "raw_reset")]
                pub fn reset_all(&mut self) {}
                #[js_export(skip)]
                pub fn internal_generic<T>(&self, x: T) -> T { x }
                #[js_export(skip)]
                pub async fn internal_borrowed_async(&self) -> u32 { self.id }
                pub async fn fetch_remote(url: &str) -> Result<String, JsError> { todo!() }
                pub async fn flush_and_close(self, reason: &str) -> u32 { let _ = reason; self.id }
            }
            "#,
        )
        .unwrap();

        assert!(expanded.contains("allow (unexpected_cfgs"), "{expanded}");
        assert!(expanded.contains("# [cfg (mires)]"), "{expanded}");
        assert!(!expanded.contains("# [js_export"), "{expanded}");
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$Session$new")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains(
                r#"unsafe (export_name = "__mires_cls$Session$__static_ctor$fromToken")"#
            ),
            "{expanded}"
        );
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$Session$currentId")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains(r#"unsafe (export_name = "__mires_cls$Session$raw_reset")"#),
            "{expanded}"
        );
        assert!(!expanded.contains("internalGeneric"), "{expanded}");
        assert!(expanded.contains("pub fn internal_generic"), "{expanded}");
        assert!(!expanded.contains("Session$internal"), "{expanded}");
        assert!(!expanded.contains("internalBorrowedAsync"), "{expanded}");

        // Static async method.
        assert!(
            expanded
                .contains(r#"unsafe (export_name = "__mires_cls$Session$__static$fetchRemote")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains(r#"unsafe (link_section = "rcgjs.export.out.v1")"#),
            "{expanded}"
        );
        assert!(
            expanded.contains(
                "-> mires_js :: __export :: TypedPromise < Result < String , JsError > >"
            ),
            "{expanded}"
        );

        // Consuming `self` async method takes ownership BEFORE `async move`.
        assert!(
            expanded
                .contains(r#"unsafe (export_name = "__mires_cls$Session$__take$flushAndClose")"#),
            "{expanded}"
        );
        let into_owned_pos = expanded
            .find("let __mires_owned = mires_js :: __export :: class_handle :: into_owned (__mires_self) ;")
            .expect("into_owned precedes async move");
        let async_move_pos = expanded[into_owned_pos..]
            .find("async move")
            .expect("async move follows into_owned");
        assert!(async_move_pos > 0);
        assert!(
            expanded.contains(
                "< Session > :: flush_and_close (__mires_owned , reason . as_str ()) . await"
            ),
            "{expanded}"
        );
    }
}
