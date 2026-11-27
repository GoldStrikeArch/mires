//! Standard I/O streams (`Stdout`, `Stderr`, `Stdin`) backed by `console.log`
//! and `console.error` via `runtime/shim.js`.
//!
//! Because `console.log` and `console.error` append a newline per call, partial
//! writes from `print!` / `eprint!` / `Write::write` are line-buffered until a
//! newline, `println!` / `eprintln!`, `flush()`, or `main()` exit.

use crate::cell::UnsafeCell;
use crate::fmt;
use crate::io::{BufRead, Read, Result, Write};
use crate::string::String;
use crate::vec::Vec;

struct LineBuffer {
    buf: UnsafeCell<Option<Vec<u8>>>,
}

// SAFETY: `mires` targets single-threaded JS (`singlethread = true`).
unsafe impl Sync for LineBuffer {}

impl LineBuffer {
    const fn new() -> Self {
        Self {
            buf: UnsafeCell::new(None),
        }
    }

    fn is_empty(&self) -> bool {
        // SAFETY: Single-threaded execution; borrow does not escape this method.
        unsafe { (*self.buf.get()).as_ref().is_none_or(Vec::is_empty) }
    }

    fn write_bytes(&self, data: &[u8], emit: fn(&str)) {
        // SAFETY: Single-threaded execution; exclusive mutable access to `self.buf`.
        let slot = unsafe { &mut *self.buf.get() };
        let buf = slot.get_or_insert_with(Vec::new);
        buf.extend_from_slice(data);
        while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..nl]);
            emit(&line);
            buf.drain(..=nl);
        }
    }

    fn write_line_str(&self, s: &str, emit: fn(&str)) {
        if self.is_empty() {
            emit(s);
            return;
        }
        // SAFETY: Single-threaded execution; exclusive mutable access to `self.buf`.
        let slot = unsafe { &mut *self.buf.get() };
        let buf = slot.get_or_insert_with(Vec::new);
        buf.extend_from_slice(s.as_bytes());
        let line = String::from_utf8_lossy(buf);
        emit(&line);
        buf.clear();
    }

    fn flush(&self, emit: fn(&str)) {
        if self.is_empty() {
            return;
        }
        // SAFETY: Single-threaded execution; exclusive mutable access to `self.buf`.
        let slot = unsafe { &mut *self.buf.get() };
        if let Some(buf) = slot.as_mut() {
            if !buf.is_empty() {
                let line = String::from_utf8_lossy(buf);
                emit(&line);
                buf.clear();
            }
        }
    }
}

static STDOUT_BUF: LineBuffer = LineBuffer::new();
static STDERR_BUF: LineBuffer = LineBuffer::new();

/// Flushes any buffered partial lines on stdout and stderr (called at `main` exit).
pub(crate) fn flush_stdio() {
    STDOUT_BUF.flush(crate::sys::log_str);
    STDERR_BUF.flush(crate::sys::eprint_str);
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    if let Some(s) = args.as_str() {
        STDOUT_BUF.write_bytes(s.as_bytes(), crate::sys::log_str);
    } else {
        let formatted = alloc_crate::fmt::format(args);
        STDOUT_BUF.write_bytes(formatted.as_bytes(), crate::sys::log_str);
    }
}

#[doc(hidden)]
pub fn _println(args: fmt::Arguments<'_>) {
    if let Some(s) = args.as_str() {
        STDOUT_BUF.write_line_str(s, crate::sys::log_str);
    } else {
        let formatted = alloc_crate::fmt::format(args);
        STDOUT_BUF.write_line_str(&formatted, crate::sys::log_str);
    }
}

#[doc(hidden)]
pub fn _eprint(args: fmt::Arguments<'_>) {
    if let Some(s) = args.as_str() {
        STDERR_BUF.write_bytes(s.as_bytes(), crate::sys::eprint_str);
    } else {
        let formatted = alloc_crate::fmt::format(args);
        STDERR_BUF.write_bytes(formatted.as_bytes(), crate::sys::eprint_str);
    }
}

#[doc(hidden)]
pub fn _eprintln(args: fmt::Arguments<'_>) {
    if let Some(s) = args.as_str() {
        STDERR_BUF.write_line_str(s, crate::sys::eprint_str);
    } else {
        let formatted = alloc_crate::fmt::format(args);
        STDERR_BUF.write_line_str(&formatted, crate::sys::eprint_str);
    }
}

/// A handle to the global standard output stream.
#[derive(Debug)]
pub struct Stdout {
    _private: (),
}

/// A locked reference to the [`Stdout`] handle.
#[derive(Debug)]
pub struct StdoutLock<'a> {
    _marker: crate::marker::PhantomData<&'a Stdout>,
}

/// Constructs a new handle to the standard output of the current process.
#[must_use]
#[inline]
pub fn stdout() -> Stdout {
    Stdout { _private: () }
}

impl Stdout {
    /// Locks this handle to the standard output stream.
    #[must_use]
    #[inline]
    pub fn lock(&self) -> StdoutLock<'static> {
        StdoutLock {
            _marker: crate::marker::PhantomData,
        }
    }
}

impl Write for Stdout {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        STDOUT_BUF.write_bytes(buf, crate::sys::log_str);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        STDOUT_BUF.flush(crate::sys::log_str);
        Ok(())
    }
}

impl Write for &Stdout {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        STDOUT_BUF.write_bytes(buf, crate::sys::log_str);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        STDOUT_BUF.flush(crate::sys::log_str);
        Ok(())
    }
}

impl Write for StdoutLock<'_> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        STDOUT_BUF.write_bytes(buf, crate::sys::log_str);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        STDOUT_BUF.flush(crate::sys::log_str);
        Ok(())
    }
}

/// A handle to the standard error stream of a process.
#[derive(Debug)]
pub struct Stderr {
    _private: (),
}

/// A locked reference to the [`Stderr`] handle.
#[derive(Debug)]
pub struct StderrLock<'a> {
    _marker: crate::marker::PhantomData<&'a Stderr>,
}

/// Constructs a new handle to the standard error of the current process.
#[must_use]
#[inline]
pub fn stderr() -> Stderr {
    Stderr { _private: () }
}

impl Stderr {
    /// Locks this handle to the standard error stream.
    #[must_use]
    #[inline]
    pub fn lock(&self) -> StderrLock<'static> {
        StderrLock {
            _marker: crate::marker::PhantomData,
        }
    }
}

impl Write for Stderr {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        STDERR_BUF.write_bytes(buf, crate::sys::eprint_str);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        STDERR_BUF.flush(crate::sys::eprint_str);
        Ok(())
    }
}

impl Write for &Stderr {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        STDERR_BUF.write_bytes(buf, crate::sys::eprint_str);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        STDERR_BUF.flush(crate::sys::eprint_str);
        Ok(())
    }
}

impl Write for StderrLock<'_> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        STDERR_BUF.write_bytes(buf, crate::sys::eprint_str);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        STDERR_BUF.flush(crate::sys::eprint_str);
        Ok(())
    }
}

/// A handle to the standard input stream (EOF/unsupported stub on JS).
#[derive(Debug)]
pub struct Stdin {
    _private: (),
}

/// A locked reference to the [`Stdin`] handle.
#[derive(Debug)]
pub struct StdinLock<'a> {
    _marker: crate::marker::PhantomData<&'a Stdin>,
}

/// Constructs a new handle to the standard input of the current process.
#[must_use]
#[inline]
pub fn stdin() -> Stdin {
    Stdin { _private: () }
}

impl Stdin {
    /// Locks this handle to the standard input stream.
    #[must_use]
    #[inline]
    pub fn lock(&self) -> StdinLock<'static> {
        StdinLock {
            _marker: crate::marker::PhantomData,
        }
    }

    /// Reads a line from stdin (always returns `Ok(0)` EOF on JS hosts).
    #[inline]
    pub fn read_line(&self, _buf: &mut String) -> Result<usize> {
        Ok(0)
    }
}

impl Read for Stdin {
    #[inline]
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl Read for &Stdin {
    #[inline]
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl Read for StdinLock<'_> {
    #[inline]
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl BufRead for StdinLock<'_> {
    #[inline]
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(&[])
    }

    #[inline]
    fn consume(&mut self, _amt: usize) {}
}
