//! Writes a JavaScript expression into a Rust one, so a program can build a value the Rust type
//! system cannot spell without a host function standing in for it.
//!
//! ```ignore
//! let promise: JsValue = js! {
//!     globalThis.fetch(url, {
//!         method: "POST",
//!         headers: { "content-type": kind },
//!         body,
//!     }).then(r => r.text())
//! };
//! ```
//!
//! `url`, `kind` and `body` are Rust bindings in scope at the call, and they are CAPTURED: the
//! expansion passes them by value to a marker function whose [`link_section`] carries the
//! JavaScript, and the backend fills the block's holes with them. Nothing else in that block is a
//! capture: `fetch` is reached through `globalThis`, `method` and `headers` are keys, `r` is bound
//! by the arrow, and `text` is a property.
//!
//! Which is which is decided by a JavaScript lexer, [`lexer`], not a parser. An identifier it
//! cannot classify is reported as FREE, so it becomes a real Rust identifier in the expansion and
//! an unintended one is a spanned "cannot find value" from rustc rather than a capture that quietly
//! went missing.
//!
//! # What is a capture
//!
//! Everything identifier shaped except a reserved word, one of the four host names
//! ([`lexer::HOST_NAMES`]), a property after `.` or `?.`, an object literal key, and a name the
//! block itself binds. The object literal shorthand `{ body }` IS a reference and so is a capture;
//! it is written out as `{ body: <slot> }` so the key keeps its name.
//!
//! A host name is reached through `globalThis`. That is deliberate: a builtins list would go stale,
//! and going through `globalThis` makes every host name a block depends on greppable while keeping
//! the rule "a free identifier is a Rust binding" total.
//!
//! # The two ways to write the block
//!
//! Ordinarily the block is written as tokens and the macro reconstructs the JavaScript from them.
//! Rust's own lexer runs first, so a backtick cannot appear that way; a block that needs one is
//! written as a single RAW string literal instead, whose contents are taken verbatim:
//!
//! ```ignore
//! let greeting: JsValue = js! { r#"`hello ${name}`"# };
//! ```
//!
//! A plain `"..."` is an ordinary JavaScript string expression, not the verbatim form.
//!
//! # What this does not do
//!
//! The block is not validated as JavaScript. There is no parser here and none in the backend, so a
//! malformed block is a syntax error in the browser rather than a spanned error in Rust. What is
//! checked is that the body is one expression rather than a run of statements, that every bracket
//! balances, that no string, template, comment or regular expression is left open, and that no name
//! collides with an emitted capture slot.
//!
//! A name the block BINDS is never a capture, in the region that binds it. Shadowing a Rust binding
//! with a parameter of the same name and expecting to capture it outside the arrow works, but doing
//! it the other way round does not: pick a different name.

mod block;
mod lexer;

use std::collections::BTreeMap;

use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::spanned::Spanned;

/// Writes a JavaScript expression, capturing the Rust bindings it names. See the crate docs.
#[proc_macro]
pub fn js(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match Js::parse(input.into()).and_then(|js| js.expand()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// One `js!{}` block, as written.
pub(crate) struct Js {
    /// The JavaScript, either reconstructed from the tokens or taken from a raw string literal.
    source: String,
    /// Where the block as a whole is, which is where a block wide error is reported.
    span: Span,
    /// Where each identifier the block wrote was written. A capture is emitted at its own span, so
    /// "cannot find value" points at the name rather than at the macro.
    spans: BTreeMap<String, Span>,
}

impl Js {
    /// Reads a block's tokens.
    ///
    /// # Errors
    ///
    /// Returns why the tokens are not a block: an empty body is the only thing rejected here, since
    /// everything about the JavaScript itself is [`Js::expand`]'s to say.
    pub(crate) fn parse(input: TokenStream) -> syn::Result<Self> {
        let span = input.span();
        if input.is_empty() {
            return Err(syn::Error::new(span, "`js!{}` takes one JavaScript expression"));
        }

        // A raw string literal is the verbatim form. A plain one is an ordinary JavaScript string
        // expression, so only the raw spelling switches forms, which is what makes the two
        // unambiguous.
        if let Some(literal) = raw_string(&input) {
            let text: syn::LitStr = syn::parse2(literal.clone())?;
            return Ok(Self { source: text.value(), span: text.span(), spans: BTreeMap::new() });
        }

        let mut source = String::new();
        let mut spans = BTreeMap::new();
        write_tokens(&mut source, &mut spans, input, &mut false);
        Ok(Self { source, span, spans })
    }

    /// The `link_section` the block travels as, and the Rust bindings that fill its slots.
    ///
    /// # Errors
    ///
    /// Returns why the block is not one this macro takes. Every reason is [`lexer::analyze`]'s
    /// except one: a free identifier that is not spellable as a Rust identifier, which cannot be
    /// a capture and so has to be a host name written without `globalThis`.
    pub(crate) fn section(&self) -> syn::Result<(String, Vec<syn::Ident>)> {
        let analysis =
            lexer::analyze(&self.source).map_err(|why| syn::Error::new(self.span, why))?;
        let captures: Vec<syn::Ident> =
            analysis.free.iter().map(|name| self.capture(name)).collect::<syn::Result<_>>()?;
        let block = block::Block { slots: captures.len(), text: analysis.text };
        Ok((block.encode(), captures))
    }

    /// The marker function that carries the block, and the call that fills its slots.
    ///
    /// # Errors
    ///
    /// Returns why the block is not one this macro takes. See [`Js::section`].
    pub(crate) fn expand(&self) -> syn::Result<TokenStream> {
        let (section, captures) = self.section()?;
        let types: Vec<syn::Ident> =
            (0..captures.len()).map(|index| format_ident!("__JsCapture{index}")).collect();
        let parameters: Vec<syn::Ident> =
            (0..captures.len()).map(|index| format_ident!("__js_capture{index}")).collect();

        // No braces in the message: `unreachable!` reads it as a format string.
        let unreachable = "a `js!` block is only something rustc_codegen_mires can lower";

        Ok(quote! {{
            #[cfg_attr(target_arch = "wasm32", link_section = #section)]
            #[inline(never)]
            // The body names no parameter and never can: it stands in for an emission, and the
            // signature exists only to carry the captures across.
            #[allow(unused_variables)]
            fn __mires_js_block<__JsBlock #(, #types)*>(
                #(#parameters: #types),*
            ) -> __JsBlock {
                // Never runs: the backend replaces the call by its `link_section`. Reaching this
                // body means the crate was compiled by a backend that does not know the block.
                ::core::unreachable!(#unreachable)
            }
            __mires_js_block(#(#captures),*)
        }})
    }

    /// The Rust identifier a free name captures, at the span it was written.
    fn capture(&self, name: &str) -> syn::Result<syn::Ident> {
        let span = self.spans.get(name).copied().unwrap_or(self.span);
        // Asked of `syn` rather than of `Ident::new`, which panics on a keyword and on `_`.
        if syn::parse_str::<syn::Ident>(name).is_err() {
            return Err(syn::Error::new(
                span,
                format!(
                    "`{name}` is free in this block and is not a Rust identifier, so nothing can \
                     be captured for it; a name the host supplies is reached through `globalThis`"
                ),
            ));
        }
        Ok(syn::Ident::new(name, span))
    }
}

/// The single raw string literal a verbatim block is written as, if that is what the input is.
fn raw_string(input: &TokenStream) -> Option<TokenStream> {
    let mut tokens = input.clone().into_iter();
    let TokenTree::Literal(literal) = tokens.next()? else { return None };
    if tokens.next().is_some() {
        return None;
    }
    let text = literal.to_string();
    match text.starts_with("r\"") || text.starts_with("r#") {
        true => Some(TokenStream::from(TokenTree::Literal(literal))),
        false => None,
    }
}

/// Writes `tokens` back out as the JavaScript they were written as.
///
/// Rust's lexer has already run, so this is a reconstruction and not the source: comments are gone
/// and a backtick could never have got this far, which is what the raw string form is for. What it
/// does keep is the ADJACENCY, which `Spacing::Joint` records. Two punctuation marks written apart
/// are kept apart and two written together are kept together, so `?.` stays an optional chain and
/// `a - -b` does not become a decrement. Nothing else needs a space, because an identifier or a
/// literal after punctuation can never lex as part of it.
fn write_tokens(
    out: &mut String,
    spans: &mut BTreeMap<String, Span>,
    tokens: TokenStream,
    word: &mut bool,
) {
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    // An invisible group is a fragment another macro substituted, so its ends are
                    // not token boundaries and whatever is inside carries the state through.
                    Delimiter::None => ("", ""),
                };
                out.push_str(open);
                if group.delimiter() != Delimiter::None {
                    *word = false;
                }
                write_tokens(out, spans, group.stream(), word);
                out.push_str(close);
                if group.delimiter() != Delimiter::None {
                    *word = false;
                }
            }
            TokenTree::Ident(ident) => {
                // Two words written together would lex as one, and nothing else would.
                if *word {
                    out.push(' ');
                }
                let name = ident.to_string();
                spans.entry(name.clone()).or_insert_with(|| ident.span());
                out.push_str(&name);
                *word = true;
            }
            TokenTree::Literal(literal) => {
                if *word {
                    out.push(' ');
                }
                out.push_str(&literal.to_string());
                *word = true;
            }
            TokenTree::Punct(punct) => {
                out.push(punct.as_char());
                let apart = punct.spacing() == proc_macro2::Spacing::Alone;
                if apart && matches!(tokens.peek(), Some(TokenTree::Punct(_))) {
                    out.push(' ');
                }
                *word = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block as rustc would hand it over.
    ///
    /// Lexed from text rather than built with `quote!`, because the reconstruction turns on
    /// `Spacing`, and only a lexer records which punctuation marks were written against each other.
    fn tokens(block: &str) -> TokenStream {
        block.parse().expect("the block lexes as Rust tokens")
    }

    /// The JavaScript a block's tokens come back as.
    fn source(block: &str) -> String {
        Js::parse(tokens(block)).expect("it parses").source
    }

    /// The `link_section` a block travels as, which is what the backend reads.
    fn section(block: &str) -> String {
        Js::parse(tokens(block)).expect("it parses").section().expect("it expands").0
    }

    fn error(block: &str) -> String {
        Js::parse(tokens(block))
            .and_then(|js| js.section())
            .expect_err("it is refused")
            .to_string()
    }

    #[test]
    fn the_tokens_come_back_as_the_javascript_they_were_written_as() {
        assert_eq!(source("a.b"), "a.b");
        assert_eq!(source("a?.b"), "a?.b");
        assert_eq!(source("r => r.text()"), "r=>r.text()");
        assert_eq!(source("new X(y)"), "new X(y)");
        assert_eq!(source(r#"{ "content-type": v }"#), r#"{"content-type":v}"#);
        assert_eq!(source("a === b"), "a===b");
        assert_eq!(source("a ?? b"), "a??b");
        // Two marks written apart stay apart, so a subtraction of a negation is not a decrement.
        assert_eq!(source("a - -b"), "a- -b");
        assert_eq!(source("a-- - b"), "a-- -b");
        // An identifier after punctuation needs no space, which is what keeps a `$` name whole.
        assert_eq!(source("a.$x"), "a.$x");
        // Two words do need one.
        assert_eq!(source("typeof x"), "typeof x");
    }

    #[test]
    fn a_raw_string_literal_is_the_verbatim_form_and_a_plain_one_is_not() {
        assert_eq!(source(r##"r#"`a${b}c`"#"##), "`a${b}c`");
        assert_eq!(source(r#"r"a + b""#), "a + b");
        // A plain literal is an ordinary JavaScript string, which is what it looks like.
        assert_eq!(source(r#""a + b""#), "\"a + b\"");
        assert_eq!(section(r#""a + b""#), "rcgjs.js.1.0.\"a + b\"");
        // A comment survives the verbatim form and cannot survive the token one.
        assert_eq!(source(r##"r#"a // why
b"#"##), "a // why\nb");
        assert_eq!(section(r##"r#"a // why
b"#"##), "rcgjs.js.1.2._$js0 // why\\n_$js1");
    }

    #[test]
    fn the_section_carries_the_slots_and_the_text() {
        assert_eq!(section("1 + 1"), "rcgjs.js.1.0.1+1");
        assert_eq!(section("f(a)"), "rcgjs.js.1.2._$js0(_$js1)");
        // The motivating case, end to end through the macro half.
        assert_eq!(
            section(
                r#"globalThis.fetch(url, { method: "POST", headers: { "content-type": kind }, body })"#
            ),
            "rcgjs.js.1.3.globalThis.fetch(_$js0,{method:\"POST\",headers:\
             {\"content-type\":_$js1},body: _$js2})",
        );
    }

    #[test]
    fn a_capture_is_emitted_as_the_identifier_it_names() {
        let text = Js::parse(tokens("f(a, b, a)")).unwrap().expand().unwrap().to_string();
        assert!(text.contains("__mires_js_block (f , a , b)"), "{text}");
        assert!(text.contains("__JsCapture0"), "{text}");
        assert!(text.contains("__JsCapture2"), "{text}");
        assert!(!text.contains("__JsCapture3"), "{text}");
        assert!(text.contains("inline (never)"), "{text}");
        assert!(text.contains("target_arch = \"wasm32\""), "{text}");
    }

    #[test]
    fn a_block_with_no_captures_takes_no_arguments() {
        let text = Js::parse(tokens("1 + 1")).unwrap().expand().unwrap().to_string();
        assert!(text.contains("__mires_js_block ()"), "{text}");
        assert!(!text.contains("__JsCapture0"), "{text}");
    }

    #[test]
    fn an_empty_block_is_refused() {
        assert!(error("").contains("one JavaScript expression"));
    }

    #[test]
    fn a_statement_body_and_a_slot_collision_are_refused() {
        assert!(error("const a = 1").contains("one expression"));
        assert!(error("a; b").contains("one expression"));
        // A slot name is only reachable through the verbatim form: Rust lexes `_$js0` as three
        // tokens and the reconstruction never writes them back as one.
        assert!(error(r##"r#"_$js0 + a"#"##).contains("_$js0"));
    }

    #[test]
    fn a_free_name_that_is_not_a_rust_identifier_says_where_a_host_name_goes() {
        // `$` is an identifier in JavaScript and not in Rust, so it can only be a host name.
        let message = error(r##"r#"$(".x")"#"##);
        assert!(message.contains("globalThis"), "{message}");
        // A Rust keyword cannot be a binding either.
        assert!(error(r##"r#"fn + 1"#"##).contains("globalThis"));
    }
}
