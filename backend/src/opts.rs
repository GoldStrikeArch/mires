//! The `-Cllvm-args` options, as this compilation parsed them.
//!
//! The options themselves — the [`Opts`] struct, the enums, and the parser — live in
//! [`rcgjs_object::opts`], because `jsc-link` runs the whole link step off the very same values and
//! must not depend on rustc to read them. What is left here is the seam to the compiler: turning a
//! [`Session`] into a [`Diagnostics`] sink, parsing `sess.opts.cg.llvm_args` once at
//! `CodegenBackend::init`, and the one warning that needs the session's diagnostic context.

use rustc_session::Session;

pub(crate) use rcgjs_object::opts::*;

/// A [`Session`] as a [`Diagnostics`] sink. A newtype because neither the trait nor `Session` is
/// this crate's.
pub(crate) struct SessionDiagnostics<'a>(pub(crate) &'a Session);

impl Diagnostics for SessionDiagnostics<'_> {
    fn err(&self, message: &str) {
        self.0.dcx().err(message.to_owned());
    }

    fn warn(&self, message: &str) {
        self.0.dcx().warn(message.to_owned());
    }
}

/// Parses `-Cllvm-args` and installs the result. Reports bad options on `sess`.
pub(crate) fn init(sess: &Session) {
    set(parse_args(
        &sess.opts.cg.llvm_args,
        &SessionDiagnostics(sess),
    ));
}
