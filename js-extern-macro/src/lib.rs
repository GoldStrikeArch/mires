//! Declares a JavaScript interface to `rustc_codegen_mires`, so a program can call one without
//! anybody hand writing the encoding that carries it.
//!
//! ```ignore
//! #[js_extern(module = "chart.js")]
//! unsafe extern "C" {
//!     #[js(new = "Chart")]
//!     fn chart_new(canvas: &str, config: &Config) -> Chart;
//!
//!     #[js(method = "update")]
//!     fn chart_update(chart: &Chart);
//!
//!     #[js(get = "width", nullable)]
//!     fn chart_width(chart: &Chart) -> Option<f64>;
//!
//!     // A browser global, reached through no module at all.
//!     #[js(global, new = "EventSource")]
//!     fn event_source(url: &str) -> Source;
//! }
//! ```
//!
//! Each declaration becomes a marker function: an ordinary Rust `fn` whose body never runs and
//! whose [`link_section`] the backend matches on, exactly as `view-abi`'s markers work. The
//! declaration is written as a foreign block because that is the shape a reader expects and the
//! shape the signatures suit; nothing foreign survives the expansion, and it must not, because
//! rustc has announced that `link_section` on a foreign function will become a hard error.
//!
//! See [`descriptor`] for the encoding and for the shapes.

mod descriptor;

use descriptor::{Descriptor, Shape};
use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    ForeignItem, Ident, ItemForeignMod, LitStr, Token,
};

/// Declares the JavaScript interface a foreign block describes. See the crate docs.
#[proc_macro_attribute]
pub fn js_extern(attr: TokenStream, item: TokenStream) -> TokenStream {
    let module = match syn::parse::<ModuleArg>(attr) {
        Ok(arg) => arg.module,
        Err(error) => return error.to_compile_error().into(),
    };
    let block = match syn::parse::<ItemForeignMod>(item) {
        Ok(block) => block,
        Err(error) => return error.to_compile_error().into(),
    };
    match expand(&module, &block) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// The attribute's own argument: `module = "chart.js"`, or nothing for a global.
struct ModuleArg {
    module: String,
}

impl Parse for ModuleArg {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self { module: String::new() });
        }
        let key: Ident = input.parse()?;
        if key != "module" {
            return Err(syn::Error::new(
                key.span(),
                "`js_extern` takes `module = \"..\"`, or nothing for a global",
            ));
        }
        input.parse::<Token![=]>()?;
        let value: LitStr = input.parse()?;
        if !input.is_empty() {
            return Err(input.error("`js_extern` takes one argument"));
        }
        Ok(Self { module: value.value() })
    }
}

/// One declaration's `#[js(..)]`: the shape, the JavaScript name it gives, and its flags.
struct JsAttr {
    shape: Shape,
    name: Option<String>,
    nullable: bool,
    global: bool,
    module: Option<String>,
}

impl Parse for JsAttr {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut shape = None;
        let mut name = None;
        let mut nullable = false;
        let mut global = None;
        let mut module = None;

        for entry in Punctuated::<JsEntry, Token![,]>::parse_terminated(input)? {
            match entry {
                JsEntry::Shape(span, found, value) => {
                    if shape.is_some() {
                        return Err(syn::Error::new(span, "a declaration has one shape"));
                    }
                    shape = Some(found);
                    name = value;
                }
                JsEntry::Nullable => nullable = true,
                JsEntry::Global(span) => global = Some(span),
                JsEntry::Module(span, value) => module = Some((span, value)),
            }
        }

        let shape = shape.ok_or_else(|| {
            input.error(
                "`#[js(..)]` needs a shape: `call`, `new`, `method`, `get`, `set`, `index` or \
                 `index_set`",
            )
        })?;
        // The two name a root each, so a declaration carrying both says two things and means one.
        // `global` on its own is what clears a block's module; see [`marker`].
        if let (Some(span), Some((_, module))) = (global, &module) {
            if !module.is_empty() {
                return Err(syn::Error::new(
                    span,
                    format!("`global` and `module = \"{module}\"` name two different roots"),
                ));
            }
        }
        Ok(Self {
            shape,
            name,
            nullable,
            global: global.is_some(),
            module: module.map(|(_, value)| value),
        })
    }
}

/// One entry inside `#[js(..)]`.
enum JsEntry {
    /// A shape, with the JavaScript name it was given.
    Shape(proc_macro2::Span, Shape, Option<String>),
    Nullable,
    Global(proc_macro2::Span),
    Module(proc_macro2::Span, String),
}

impl Parse for JsEntry {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        let value = match input.peek(Token![=]) {
            true => {
                input.parse::<Token![=]>()?;
                Some(input.parse::<LitStr>()?.value())
            }
            false => None,
        };

        let shape = match key.to_string().as_str() {
            "call" => Shape::Call,
            "new" => Shape::New,
            "method" => Shape::Method,
            "get" => Shape::Get,
            "set" => Shape::Set,
            "index" => Shape::Index,
            "index_set" => Shape::IndexSet,
            "nullable" => {
                if value.is_some() {
                    return Err(syn::Error::new(key.span(), "`nullable` takes no value"));
                }
                return Ok(JsEntry::Nullable);
            }
            "global" => {
                if value.is_some() {
                    return Err(syn::Error::new(key.span(), "`global` takes no value"));
                }
                return Ok(JsEntry::Global(key.span()));
            }
            "module" => {
                let value = value.ok_or_else(|| {
                    syn::Error::new(key.span(), "`module` takes a module specifier")
                })?;
                return Ok(JsEntry::Module(key.span(), value));
            }
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!("`{other}` is not a `#[js(..)]` entry"),
                ));
            }
        };
        Ok(JsEntry::Shape(key.span(), shape, value))
    }
}

/// The marker functions a block expands to.
fn expand(module: &str, block: &ItemForeignMod) -> syn::Result<proc_macro2::TokenStream> {
    let mut out = proc_macro2::TokenStream::new();
    for item in &block.items {
        let ForeignItem::Fn(function) = item else {
            return Err(syn::Error::new(
                item.span(),
                "`js_extern` declares functions and nothing else",
            ));
        };
        out.extend(marker(module, function)?);
    }
    Ok(out)
}

/// One declaration, as the marker function that carries its descriptor.
fn marker(module: &str, function: &syn::ForeignItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let mut js = None;
    let mut kept = Vec::new();
    for attr in &function.attrs {
        if attr.path().is_ident("js") {
            if js.is_some() {
                return Err(syn::Error::new(attr.span(), "a declaration has one `#[js(..)]`"));
            }
            js = Some(attr.parse_args::<JsAttr>()?);
        } else {
            kept.push(attr);
        }
    }
    let js = js.ok_or_else(|| {
        syn::Error::new(
            function.sig.ident.span(),
            "every `js_extern` declaration needs a `#[js(..)]` naming its shape",
        )
    })?;

    let signature = &function.sig;
    let visibility = &function.vis;
    let ident = &signature.ident;

    if let Some(variadic) = &signature.variadic {
        return Err(syn::Error::new(variadic.span(), "a declaration is not variadic"));
    }

    if js.nullable && !js.shape.returns_a_value() {
        return Err(syn::Error::new(
            signature.ident.span(),
            "`nullable` describes a returned value, and this shape returns none",
        ));
    }

    // An index off the receiver itself needs no path; everything else defaults to the Rust name,
    // which is what makes the common declaration a one liner.
    let name = match (js.shape, &js.name) {
        (Shape::Index | Shape::IndexSet, None) => String::new(),
        (_, Some(name)) => name.clone(),
        (_, None) => ident.to_string(),
    };
    if !name.is_empty() && !is_js_name(&name) {
        return Err(syn::Error::new(
            signature.ident.span(),
            format!("`{name}` is not a JavaScript path"),
        ));
    }

    // `global` is a root, so it replaces the block's module rather than sitting beside it: a
    // browser global inside a block that names a package is exactly what the dashboard declares.
    let module = match js.global {
        true => String::new(),
        false => js.module.as_deref().unwrap_or(module).to_owned(),
    };
    let descriptor = Descriptor {
        shape: js.shape,
        nullable: js.nullable,
        global: js.global,
        module,
        name,
    };

    if !descriptor.acts_on_argument() && descriptor.name.is_empty() {
        return Err(syn::Error::new(
            signature.ident.span(),
            "this shape is rooted at a module binding or a global, so it needs a name",
        ));
    }

    // The arity is checked here rather than in the backend alone, so a wrong declaration is a
    // spanned error on the declaration instead of a zombie at the call site.
    let arguments = signature.inputs.len();
    let required = descriptor.required_arguments();
    if arguments < required {
        let takes = match (descriptor.acts_on_argument(), js.shape) {
            (true, Shape::Index) => "the value it indexes and the key",
            (true, Shape::IndexSet) => "the value it indexes, the key and the value to write",
            (true, Shape::Set) => "the receiver and the value to write",
            (true, _) => "the receiver",
            (false, _) => "the value to write",
        };
        return Err(syn::Error::new(
            signature.inputs.span(),
            format!(
                "this shape takes {takes}, so it needs at least {required} argument(s) and has \
                 {arguments}"
            ),
        ));
    }

    let section = descriptor.encode();

    let unreachable = format!(
        "`{ident}` is a `#[js_extern]` declaration, which only rustc_codegen_mires can lower"
    );
    Ok(quote! {
        #(#kept)*
        #[cfg_attr(target_arch = "wasm32", link_section = #section)]
        #[inline(never)]
        // The body names none of the parameters and never can: it is a placeholder for an
        // emission, and the signature is the declaration's own.
        #[allow(unused_variables)]
        #visibility #signature {
            // Never runs: the backend replaces every call by its `link_section`. Reaching this
            // body means the crate was compiled by a backend that does not know the declaration.
            unreachable!(#unreachable)
        }
    })
}

/// Whether `name` can be written as a JavaScript identifier, or as a dotted path of them.
///
/// A path is what a walk needs. `instance.data.datasets` is one property read written twice, and
/// declaring a getter per segment would turn one interface into three for nothing; `Math.max` and
/// `window.crypto` are the same idea rooted at a global instead.
fn is_js_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|segment| {
            let mut characters = segment.chars();
            characters
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
                && characters
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(module: &str, source: &str) -> Result<String, String> {
        let block: ItemForeignMod = syn::parse_str(source).expect("the block parses");
        let function = match &block.items[0] {
            ForeignItem::Fn(function) => function,
            _ => panic!("the fixture declares a function"),
        };
        marker(module, function)
            .map(|tokens| {
                let text = tokens.to_string();
                let start = text.find("rcgjs.ext.").expect("the section is written");
                let end = text[start..].find('"').expect("the section is a literal") + start;
                text[start..end].to_owned()
            })
            .map_err(|error| error.to_string())
    }

    #[test]
    fn each_shape_encodes_to_its_letter() {
        for (attr, letter) in [
            ("call", 'c'),
            ("new", 'n'),
            ("index", 'i'),
            ("index_set", 'j'),
        ] {
            let source = format!(
                r#"unsafe extern "C" {{ #[js({attr})] fn f(a: u32, b: u32, c: u32) -> u32; }}"#
            );
            let encoded = descriptor("m.js", &source).expect("it expands");
            assert!(encoded.starts_with(&format!("rcgjs.ext.2.{letter}")), "{encoded}");
        }
        // The member shapes take a receiver, so they are declared with one.
        for (attr, letter) in [("method", 's'), ("get", 'g'), ("set", 't')] {
            let source =
                format!(r#"unsafe extern "C" {{ #[js({attr})] fn f(a: u32, b: u32) -> u32; }}"#);
            let encoded = descriptor("", &source).expect("it expands");
            assert!(encoded.starts_with(&format!("rcgjs.ext.2.{letter}")), "{encoded}");
        }
    }

    #[test]
    fn the_rust_name_is_the_default_javascript_name() {
        assert_eq!(
            descriptor("chart.js", r#"unsafe extern "C" { #[js(new)] fn Chart(); }"#).unwrap(),
            "rcgjs.ext.2.n-.8,5.chart.jsChart",
        );
        assert_eq!(
            descriptor("chart.js", r#"unsafe extern "C" { #[js(new = "Chart")] fn make(); }"#)
                .unwrap(),
            "rcgjs.ext.2.n-.8,5.chart.jsChart",
        );
    }

    #[test]
    fn nullable_sets_the_flag_and_only_where_a_value_comes_back() {
        assert_eq!(
            descriptor("", r#"unsafe extern "C" { #[js(get = "w", nullable)] fn w(a: u32) -> u32; }"#)
                .unwrap(),
            "rcgjs.ext.2.gn.0,1.w",
        );
        let error =
            descriptor("", r#"unsafe extern "C" { #[js(set = "w", nullable)] fn w(a: u32, b: u32); }"#)
                .unwrap_err();
        assert!(error.contains("returns none"), "{error}");
    }

    #[test]
    fn a_shape_that_needs_a_receiver_says_so_on_the_declaration() {
        for (attr, arity) in [("method", 0), ("get", 0), ("set", 1), ("index_set", 2)] {
            let params: Vec<String> = (0..arity).map(|n| format!("a{n}: u32")).collect();
            let source =
                format!(r#"unsafe extern "C" {{ #[js({attr})] fn f({}); }}"#, params.join(", "));
            let error = descriptor("", &source).unwrap_err();
            assert!(error.contains("at least"), "`{attr}` reported `{error}`");
        }
    }

    #[test]
    fn a_member_shape_with_a_module_is_a_static_and_takes_no_receiver() {
        // `Chart.version`: rooted at the module's export, so no argument at all.
        assert_eq!(
            descriptor(
                "chart.js",
                r#"unsafe extern "C" { #[js(get = "Chart.version")] fn v() -> u32; }"#,
            )
            .unwrap(),
            "rcgjs.ext.2.g-.8,13.chart.jsChart.version",
        );
        // The same shape with no module needs the receiver it acts on.
        let error =
            descriptor("", r#"unsafe extern "C" { #[js(get = "data")] fn f() -> u32; }"#)
                .unwrap_err();
        assert!(error.contains("at least 1"), "{error}");
    }

    #[test]
    fn an_index_takes_a_path_and_may_take_none() {
        // Straight off the receiver.
        assert_eq!(
            descriptor("", r#"unsafe extern "C" { #[js(index)] fn at(a: u32, k: u32) -> u32; }"#)
                .unwrap(),
            "rcgjs.ext.2.i-.0,0.",
        );
        // Or after a walk: `a.data.datasets[k]`, which is one declaration and three operations.
        assert_eq!(
            descriptor(
                "",
                r#"unsafe extern "C" { #[js(index = "data.datasets")] fn at(a: u32, k: u32) -> u32; }"#,
            )
            .unwrap(),
            "rcgjs.ext.2.i-.0,13.data.datasets",
        );
    }

    #[test]
    fn a_shape_rooted_at_a_binding_needs_a_name() {
        // The Rust name is the default, so this only bites when it is emptied deliberately.
        let error = descriptor(
            "chart.js",
            r#"unsafe extern "C" { #[js(call = "")] fn f(); }"#,
        )
        .unwrap_err();
        assert!(error.contains("needs a name"), "{error}");
    }

    #[test]
    fn a_declaration_without_a_shape_is_rejected() {
        let block: ItemForeignMod =
            syn::parse_str(r#"unsafe extern "C" { fn f(); }"#).expect("it parses");
        let ForeignItem::Fn(function) = &block.items[0] else { panic!() };
        let error = marker("m.js", function).unwrap_err().to_string();
        assert!(error.contains("needs a `#[js(..)]`"), "{error}");
    }

    #[test]
    fn a_per_declaration_module_overrides_the_blocks() {
        assert_eq!(
            descriptor(
                "chart.js",
                r#"unsafe extern "C" { #[js(new, module = "")] fn EventSource(u: u32); }"#,
            )
            .unwrap(),
            "rcgjs.ext.2.n-.0,11.EventSource",
        );
    }

    #[test]
    fn global_roots_a_member_shape_that_would_otherwise_take_a_receiver() {
        // The flag is what this buys: without it a `get` with no module reads a property of
        // argument zero, and there is no argument to read one off.
        assert_eq!(
            descriptor(
                "",
                r#"unsafe extern "C" { #[js(global, get = "location.href")] fn href() -> u32; }"#,
            )
            .unwrap(),
            "rcgjs.ext.2.gg.0,13.location.href",
        );
        // The same declaration without it needs the receiver it would have acted on.
        let error = descriptor(
            "",
            r#"unsafe extern "C" { #[js(get = "location.href")] fn href() -> u32; }"#,
        )
        .unwrap_err();
        assert!(error.contains("at least 1"), "{error}");
    }

    #[test]
    fn global_clears_the_blocks_module_and_refuses_to_sit_beside_one() {
        // The dashboard's shape: a browser global declared inside a block that names a package.
        assert_eq!(
            descriptor(
                "chart.js",
                r#"unsafe extern "C" { #[js(global, new = "EventSource")] fn source(u: &str) -> u32; }"#,
            )
            .unwrap(),
            "rcgjs.ext.2.ng.0,11.EventSource",
        );
        let error = descriptor(
            "",
            r#"unsafe extern "C" { #[js(global, module = "chart.js", new = "Chart")] fn f(); }"#,
        )
        .unwrap_err();
        assert!(error.contains("two different roots"), "{error}");
    }

    #[test]
    fn a_call_and_a_new_are_global_without_the_flag_because_they_have_no_receiver() {
        // Unchanged by the flag's arrival: these two never acted on an argument.
        assert_eq!(
            descriptor("", r#"unsafe extern "C" { #[js(new)] fn EventSource(u: &str) -> u32; }"#)
                .unwrap(),
            "rcgjs.ext.2.n-.0,11.EventSource",
        );
        assert_eq!(
            descriptor("", r#"unsafe extern "C" { #[js(call = "Math.max")] fn max(a: u32) -> u32; }"#)
                .unwrap(),
            "rcgjs.ext.2.c-.0,8.Math.max",
        );
    }

    #[test]
    fn a_global_may_be_a_dotted_path_and_not_arbitrary_text() {
        assert!(is_js_name("EventSource"));
        assert!(is_js_name("Math.max"));
        assert!(is_js_name("$"));
        assert!(is_js_name("_a$1"));
        assert!(!is_js_name("1a"));
        assert!(!is_js_name("a-b"));
        assert!(!is_js_name("a..b"));
        assert!(!is_js_name(""));
    }

    #[test]
    fn the_expansion_keeps_the_signature_and_the_docs() {
        let block: ItemForeignMod = syn::parse_str(
            r#"unsafe extern "C" { /// docs
                #[js(new)] pub fn Chart(canvas: &str) -> u32; }"#,
        )
        .expect("it parses");
        let ForeignItem::Fn(function) = &block.items[0] else { panic!() };
        let text = marker("chart.js", function).unwrap().to_string();
        assert!(text.contains("pub fn Chart (canvas : & str) -> u32"), "{text}");
        assert!(text.contains("doc"), "{text}");
        assert!(text.contains("inline (never)"), "{text}");
        assert!(text.contains("target_arch = \"wasm32\""), "{text}");
    }
}
