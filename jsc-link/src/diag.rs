//! The diagnostics sink.
//!
//! The link step used to report through the compiler's `DiagCtxt`. It runs in its own process now,
//! so it reports the same messages on stderr and hands the count back through the exit status; the
//! dylib streams the text through unchanged and turns a non-zero status into one rustc error. The
//! rendering is deliberately plain — `error: …` / `warning: …`, one indented `= note:` per note —
//! because every test that reads these messages greps for a substring of the message itself.

use std::cell::Cell;
use std::io::Write;

/// Where the link step's errors and warnings go.
#[derive(Default)]
pub struct Sink {
    errors: Cell<u32>,
}

impl Sink {
    pub fn new() -> Sink {
        Sink::default()
    }

    /// How many errors have been reported. A non-zero count fails the link.
    pub fn errors(&self) -> u32 {
        self.errors.get()
    }

    pub fn err(&self, message: impl Into<String>) {
        self.struct_err(message).emit();
    }

    pub fn warn(&self, message: impl Into<String>) {
        emit("warning", &message.into(), &[]);
    }

    /// An error a caller attaches notes to before emitting, so that the note lines stay with their
    /// message when several links write to one stderr.
    pub fn struct_err(&self, message: impl Into<String>) -> Diag<'_> {
        Diag {
            sink: self,
            message: message.into(),
            notes: Vec::new(),
        }
    }
}

/// A `rcgjs_object::opts::Diagnostics` sink, so the option parser and the lane warning report the
/// same way here as they do inside the compiler.
impl rcgjs_object::opts::Diagnostics for Sink {
    fn err(&self, message: &str) {
        Sink::err(self, message);
    }

    fn warn(&self, message: &str) {
        Sink::warn(self, message);
    }
}

/// An error under construction.
#[must_use = "a diagnostic is only reported once `emit` is called"]
pub struct Diag<'a> {
    sink: &'a Sink,
    message: String,
    notes: Vec<String>,
}

impl Diag<'_> {
    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    pub fn emit(self) {
        self.sink.errors.set(self.sink.errors.get() + 1);
        emit("error", &self.message, &self.notes);
    }
}

fn emit(level: &str, message: &str, notes: &[String]) {
    let mut out = format!("{level}: {message}\n");
    for note in notes {
        out.push_str(&format!("  = note: {note}\n"));
    }
    let _ = std::io::stderr().write_all(out.as_bytes());
}
