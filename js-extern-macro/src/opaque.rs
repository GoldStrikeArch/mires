//! `#[js_opaque]` expansion for framework-extensible opaque host handle wrappers.

use mires_marker_abi::export::OPAQUE_TS_CONST_NAME;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Attribute, Fields, Ident, Item, LitStr, Path, PathArguments, Token, Type,
};

/// Arguments parsed from `#[js_opaque(..)]`.
struct OpaqueAttr {
    ts: Option<String>,
    crate_path: Option<Path>,
}

/// One entry inside `#[js_opaque(..)]`.
enum OpaqueEntry {
    Ts(Span, String),
    Crate(Span, Path),
}

impl Parse for OpaqueEntry {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Token![crate]) {
            let kw = input.parse::<Token![crate]>()?;
            if !input.peek(Token![=]) {
                return Err(syn::Error::new(
                    kw.span(),
                    "`crate` in `#[js_opaque(..)]` requires a crate path, \
                     e.g. `#[js_opaque(crate = mires_js)]`",
                ));
            }
            input.parse::<Token![=]>()?;
            let path: Path = input.parse()?;
            return Ok(Self::Crate(kw.span(), path));
        }

        let key: Ident = input.parse()?;
        match key.to_string().as_str() {
            "ts" => {
                if !input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        key.span(),
                        "`ts` in `#[js_opaque(..)]` requires a TypeScript type string, \
                         e.g. `#[js_opaque(ts = \"JSX.Element\")]`",
                    ));
                }
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                Ok(Self::Ts(lit.span(), lit.value()))
            }
            other => Err(syn::Error::new(
                key.span(),
                format!(
                    "unknown `#[js_opaque]` argument `{other}`; expected `ts = \"..\"` or \
                     `crate = <path>`"
                ),
            )),
        }
    }
}

impl Parse for OpaqueAttr {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut ts = None;
        let mut crate_path = None;

        for entry in Punctuated::<OpaqueEntry, Token![,]>::parse_terminated(input)? {
            match entry {
                OpaqueEntry::Ts(span, value) => {
                    if ts.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `ts` in `#[js_opaque(..)]`",
                        ));
                    }
                    if value.trim().is_empty() {
                        return Err(syn::Error::new(
                            span,
                            "`ts` in `#[js_opaque(ts = \"..\")]` must not be empty; omit `ts` \
                             for a distinct branded opaque type",
                        ));
                    }
                    ts = Some(value);
                }
                OpaqueEntry::Crate(span, path) => {
                    if crate_path.is_some() {
                        return Err(syn::Error::new(
                            span,
                            "duplicate `crate` in `#[js_opaque(..)]`",
                        ));
                    }
                    crate_path = Some(path);
                }
            }
        }

        Ok(Self { ts, crate_path })
    }
}

/// Unwraps any invisible proc-macro `Type::Group` delimiters around `ty`.
fn unwrap_type_group(mut ty: &Type) -> &Type {
    while let Type::Group(group) = ty {
        ty = &group.elem;
    }
    ty
}

/// Whether `ty` syntactically names `HostHandle` (possibly qualified, with no type arguments).
fn is_host_handle_type(ty: &Type) -> bool {
    let Type::Path(type_path) = unwrap_type_group(ty) else {
        return false;
    };
    if type_path.qself.is_some() {
        return false;
    }
    type_path.path.segments.last().is_some_and(|seg| {
        seg.ident == "HostHandle" && matches!(seg.arguments, PathArguments::None)
    })
}

/// Whether `ty` syntactically names `PhantomData<..>` (possibly qualified).
fn is_phantom_data_type(ty: &Type) -> bool {
    let Type::Path(type_path) = unwrap_type_group(ty) else {
        return false;
    };
    if type_path.qself.is_some() {
        return false;
    }
    type_path
        .path
        .segments
        .last()
        .is_some_and(|seg| seg.ident == "PhantomData")
}

/// Inspects existing `#[repr(..)]` attributes on the struct, rejecting conflicting reprs and
/// returning whether `#[repr(transparent)]` is already present.
fn check_existing_repr(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut has_transparent = false;
    for attr in attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("transparent") {
                has_transparent = true;
                Ok(())
            } else {
                Err(syn::Error::new(
                    meta.path.span(),
                    "`#[js_opaque]` requires `#[repr(transparent)]` and conflicts with existing \
                     `#[repr(..)]`",
                ))
            }
        })?;
    }
    Ok(has_transparent)
}

/// Resolves the hidden `__export` runtime support module path.
fn rt_support_path(crate_path: Option<&Path>) -> TokenStream {
    match crate_path {
        Some(path) => quote! { #path::__export },
        None => quote! { ::mires::__export },
    }
}

/// Entry point for `#[js_opaque]`.
pub(crate) fn expand_opaque(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args: OpaqueAttr = syn::parse2(attr)?;
    let parsed: Item = syn::parse2(item)?;
    let Item::Struct(item_struct) = parsed else {
        return Err(syn::Error::new(
            parsed.span(),
            "`#[js_opaque]` can only be applied to tuple structs wrapping `HostHandle`",
        ));
    };

    let Fields::Unnamed(fields) = &item_struct.fields else {
        return Err(syn::Error::new(
            item_struct.fields.span(),
            "`#[js_opaque]` requires a tuple struct whose first field is `HostHandle`, \
             e.g. `pub struct Node(HostHandle);`",
        ));
    };
    let Some(first_field) = fields.unnamed.first() else {
        return Err(syn::Error::new(
            fields.span(),
            "`#[js_opaque]` requires a tuple struct whose first field is `HostHandle`, \
             e.g. `pub struct Node(HostHandle);`",
        ));
    };
    if !is_host_handle_type(&first_field.ty) {
        return Err(syn::Error::new(
            first_field.ty.span(),
            "`#[js_opaque]` requires the first field to be `HostHandle`",
        ));
    }

    let has_transparent = check_existing_repr(&item_struct.attrs)?;
    let repr_attr = if has_transparent {
        quote! {}
    } else {
        quote! { #[repr(transparent)] }
    };

    let trailing_count = fields.unnamed.len() - 1;
    let all_trailing_phantom = fields
        .unnamed
        .iter()
        .skip(1)
        .all(|field| is_phantom_data_type(&field.ty));

    let rt = rt_support_path(args.crate_path.as_ref());
    let ident = &item_struct.ident;
    let (impl_generics, ty_generics, where_clause) = item_struct.generics.split_for_impl();
    let ts_const_ident = Ident::new(OPAQUE_TS_CONST_NAME, Span::call_site());
    let ts_literal = args.ts.as_deref().unwrap_or("");

    let from_value_unchecked = if all_trailing_phantom {
        let phantom_inits = (0..trailing_count).map(|_| quote! { ::core::marker::PhantomData });
        quote! {
            /// Wraps a [`JsValue`] as this opaque host handle without runtime type checks.
            #[must_use]
            pub fn from_value_unchecked(value: #rt::JsValue) -> Self {
                Self(#rt::HostHandle::from_value(value) #(, #phantom_inits)*)
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        #repr_attr
        #item_struct

        impl #impl_generics #ident #ty_generics #where_clause {
            #[doc(hidden)]
            pub const #ts_const_ident: &'static str = #ts_literal;

            /// Erases this opaque host handle to a [`JsValue`].
            #[must_use]
            pub fn as_value(self) -> #rt::JsValue {
                self.0.as_value()
            }

            #from_value_unchecked
        }

        impl #impl_generics ::core::convert::From<#ident #ty_generics> for #rt::JsValue #where_clause {
            fn from(value: #ident #ty_generics) -> Self {
                value.as_value()
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_str(attr: &str, item: &str) -> Result<String, String> {
        let attr_ts: TokenStream = attr.parse().expect("attr tokens parse");
        let item_ts: TokenStream = item.parse().expect("item tokens parse");
        expand_opaque(attr_ts, item_ts)
            .map(|tokens| tokens.to_string())
            .map_err(|err| err.to_string())
    }

    #[test]
    fn bare_js_opaque_adds_repr_transparent_and_empty_ts_const() {
        let expanded = expand_str("", "pub struct Node(HostHandle);").unwrap();
        assert!(expanded.contains("# [repr (transparent)]"), "{expanded}");
        assert!(
            expanded.contains(r#"pub const __MIRES_TS_TYPE : & 'static str = "" ;"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("pub fn as_value (self) -> :: mires :: __export :: JsValue"),
            "{expanded}"
        );
        assert!(
            expanded.contains(
                "pub fn from_value_unchecked (value : :: mires :: __export :: JsValue) -> Self"
            ),
            "{expanded}"
        );
        assert!(
            expanded.contains("Self (:: mires :: __export :: HostHandle :: from_value (value))"),
            "{expanded}"
        );
        assert!(
            expanded.contains(
                ":: core :: convert :: From < Node > for :: mires :: __export :: JsValue"
            ),
            "{expanded}"
        );
    }

    #[test]
    fn js_opaque_with_ts_generics_phantom_and_crate_override() {
        let expanded = expand_str(
            r#"ts = "Signal<T>", crate = mires_js"#,
            "#[repr(transparent)] #[derive(Clone, Copy)] pub struct Signal<T>(mires_js::HostHandle, PhantomData<T>) where T: 'static;",
        )
        .unwrap();
        // Existing #[repr(transparent)] must not be duplicated.
        assert_eq!(
            expanded.matches("repr (transparent)").count(),
            1,
            "{expanded}"
        );
        assert!(
            expanded.contains(r#"pub const __MIRES_TS_TYPE : & 'static str = "Signal<T>" ;"#),
            "{expanded}"
        );
        assert!(
            expanded.contains("impl < T > Signal < T > where T : 'static"),
            "{expanded}"
        );
        assert!(
            expanded.contains("mires_js :: __export :: HostHandle :: from_value (value) , :: core :: marker :: PhantomData"),
            "{expanded}"
        );
        assert!(
            expanded.contains(
                "impl < T > :: core :: convert :: From < Signal < T > > for mires_js :: __export :: JsValue where T : 'static"
            ),
            "{expanded}"
        );
    }

    #[test]
    fn js_opaque_omits_from_value_unchecked_for_custom_trailing_zst() {
        let expanded = expand_str(
            r#"ts = "Element""#,
            "pub struct Element(HostHandle, CustomMarker);",
        )
        .unwrap();
        assert!(expanded.contains("pub fn as_value"), "{expanded}");
        assert!(!expanded.contains("from_value_unchecked"), "{expanded}");
        assert!(expanded.contains("From < Element >"), "{expanded}");
    }

    #[test]
    fn js_opaque_rejects_invalid_items_and_arguments() {
        for (attr, item, expected) in [
            ("", "pub fn foo() {}", "only be applied to tuple structs"),
            (
                "",
                "pub enum Kind { A }",
                "only be applied to tuple structs",
            ),
            (
                "",
                "pub struct Named { h: HostHandle }",
                "requires a tuple struct",
            ),
            ("", "pub struct Unit;", "requires a tuple struct"),
            ("", "pub struct Empty();", "requires a tuple struct"),
            (
                "",
                "pub struct Raw(u32);",
                "requires the first field to be `HostHandle`",
            ),
            (
                "",
                "#[repr(C)] pub struct BadRepr(HostHandle);",
                "conflicts with existing `#[repr(..)]`",
            ),
            (
                r#"ts = """#,
                "pub struct Node(HostHandle);",
                "must not be empty",
            ),
            (
                r#"ts = "   ""#,
                "pub struct Node(HostHandle);",
                "must not be empty",
            ),
            (
                "ts",
                "pub struct Node(HostHandle);",
                "requires a TypeScript type",
            ),
            (
                r#"ts = "A", ts = "B""#,
                "pub struct Node(HostHandle);",
                "duplicate `ts`",
            ),
            (
                "crate = a, crate = b",
                "pub struct Node(HostHandle);",
                "duplicate `crate`",
            ),
            (
                "foo = \"bar\"",
                "pub struct Node(HostHandle);",
                "unknown `#[js_opaque]` argument `foo`",
            ),
        ] {
            let err = expand_str(attr, item).unwrap_err();
            assert!(
                err.contains(expected),
                "expected `{expected}` for `#[js_opaque({attr})] {item}`, got `{err}`"
            );
        }
    }
}
