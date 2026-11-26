//! A small JavaScript AST and pretty printer.
//!
//! The module uses only `std` plus the backend's small [`crate::opts::EcmaTarget`] value. Build an
//! [`Expr`]/[`Stmt`] tree with the convenience constructors in this module and render it with
//! [`program_to_string`].
//!
//! The printer indents with two spaces and puts every statement on its own line, because the
//! spike output is read by humans. Parenthesization is driven by a precedence table and errs
//! on the side of extra parentheses, never missing ones.
//!
//! Notes for a MIR lowering pass:
//!
//! * Case bodies of a [`Stmt::Switch`] are printed flat, without an extra block per case, so the
//!   contract's `case N: ...; bb = M; continue L;` shape comes out verbatim. All cases of one
//!   switch therefore share a block scope: declare function locals once at the top of the function
//!   body (with [`let_uninit`]) and only assign inside the cases, otherwise two `let` declarations
//!   of the same name in different cases are a JavaScript syntax error.
//! * Arrows with an expression body print on one line. Prefer them for the accessor objects
//!   described by the contract; [`accessor`] builds one.
//! * [`Expr::Raw`] and [`Stmt::Raw`] are escape hatches. A raw expression is parenthesized whenever
//!   it appears inside another expression, since its precedence is unknown.

mod ast;
mod build;
mod print;
mod visit;

#[cfg(test)]
mod tests;

pub use ast::*;
pub use build::*;
pub use print::*;
pub use visit::*;
