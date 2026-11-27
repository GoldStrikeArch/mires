//! Buffered I/O wrappers: `BufReader`, `BufWriter`, `LineWriter`, and `IntoInnerError`.

use crate::cmp;
use crate::error::Error as StdError;
use crate::fmt;
use crate::io::{BufRead, Error, ErrorKind, Read, Result, Seek, SeekFrom, Write};
use crate::vec::Vec;

const DEFAULT_BUF_SIZE: usize = 8 * 1024;

/// Adds buffering to any reader.
#[derive(Debug)]
pub struct BufReader<R: ?Sized> {
    buf: Vec<u8>,
    pos: usize,
    filled: usize,
    inner: R,
}

impl<R: Read> BufReader<R> {
    /// Creates a new `BufReader<R>` with a default buffer capacity.
    #[must_use]
    pub fn new(inner: R) -> BufReader<R> {
        BufReader::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    /// Creates a new `BufReader<R>` with the specified buffer capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize, inner: R) -> BufReader<R> {
        let mut buf = Vec::with_capacity(capacity);
        buf.resize(capacity, 0);
        BufReader {
            buf,
            pos: 0,
            filled: 0,
            inner,
        }
    }
}

impl<R: ?Sized> BufReader<R> {
    /// Gets a reference to the underlying reader.
    #[inline]
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Gets a mutable reference to the underlying reader.
    #[inline]
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Returns a reference to the internally buffered data.
    #[inline]
    pub fn buffer(&self) -> &[u8] {
        &self.buf[self.pos..self.filled]
    }

    /// Returns the number of bytes the internal buffer can hold at once.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Unwraps this `BufReader<R>`, returning the underlying reader.
    #[inline]
    pub fn into_inner(self) -> R
    where
        R: Sized,
    {
        self.inner
    }

    fn discard_buffer(&mut self) {
        self.pos = 0;
        self.filled = 0;
    }
}

impl<R: Read + ?Sized> Read for BufReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.pos == self.filled && buf.len() >= self.buf.len() {
            self.discard_buffer();
            return self.inner.read(buf);
        }
        let nread = {
            let mut rem = self.fill_buf()?;
            rem.read(buf)?
        };
        self.consume(nread);
        Ok(nread)
    }
}

impl<R: Read + ?Sized> BufRead for BufReader<R> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.pos >= self.filled {
            self.pos = 0;
            self.filled = self.inner.read(&mut self.buf)?;
        }
        Ok(&self.buf[self.pos..self.filled])
    }

    #[inline]
    fn consume(&mut self, amt: usize) {
        self.pos = cmp::min(self.pos + amt, self.filled);
    }
}

impl<R: Seek + ?Sized> Seek for BufReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let result = if let SeekFrom::Current(n) = pos {
            let remainder = (self.filled - self.pos) as i64;
            if let Some(offset) = n.checked_sub(remainder) {
                self.inner.seek(SeekFrom::Current(offset))?
            } else {
                self.inner.seek(SeekFrom::Current(-remainder))?;
                self.discard_buffer();
                self.inner.seek(SeekFrom::Current(n))?
            }
        } else {
            self.inner.seek(pos)?
        };
        self.discard_buffer();
        Ok(result)
    }

    fn stream_position(&mut self) -> Result<u64> {
        let remainder = (self.filled - self.pos) as u64;
        self.inner
            .stream_position()
            .map(|pos| pos.saturating_sub(remainder))
    }
}

/// Error returned by [`BufWriter::into_inner`].
#[derive(Debug)]
pub struct IntoInnerError<W>(W, Error);

impl<W> IntoInnerError<W> {
    /// Returns the error which caused the call to `into_inner` to fail.
    #[inline]
    pub fn error(&self) -> &Error {
        &self.1
    }

    /// Returns the buffered writer instance which generated the error.
    #[inline]
    pub fn into_inner(self) -> W {
        self.0
    }

    /// Consumes the error and returns the underlying [`Error`].
    #[inline]
    pub fn into_error(self) -> Error {
        self.1
    }

    /// Consumes the error and returns both the writer and the [`Error`].
    #[inline]
    pub fn into_parts(self) -> (Error, W) {
        (self.1, self.0)
    }
}

impl<W> fmt::Display for IntoInnerError<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.1, f)
    }
}

impl<W: fmt::Debug> StdError for IntoInnerError<W> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.1)
    }
}

impl<W> From<IntoInnerError<W>> for Error {
    fn from(err: IntoInnerError<W>) -> Error {
        err.1
    }
}

/// Wraps a writer and buffers its output.
#[derive(Debug)]
pub struct BufWriter<W: ?Sized + Write> {
    buf: Vec<u8>,
    capacity: usize,
    inner: W,
}

impl<W: Write> BufWriter<W> {
    /// Creates a new `BufWriter<W>` with a default buffer capacity.
    #[must_use]
    pub fn new(inner: W) -> BufWriter<W> {
        BufWriter::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    /// Creates a new `BufWriter<W>` with at least the specified buffer capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize, inner: W) -> BufWriter<W> {
        BufWriter {
            buf: Vec::with_capacity(capacity),
            capacity,
            inner,
        }
    }

    /// Unwraps this `BufWriter<W>`, returning the underlying writer after flushing.
    pub fn into_inner(mut self) -> core_crate::result::Result<W, IntoInnerError<BufWriter<W>>> {
        match self.flush_buf() {
            Err(e) => Err(IntoInnerError(self, e)),
            Ok(()) => {
                // SAFETY: We move `inner` out without running `BufWriter::drop` (already flushed).
                let inner = unsafe { crate::ptr::read(&self.inner) };
                let buf = unsafe { crate::ptr::read(&self.buf) };
                crate::mem::forget(self);
                drop(buf);
                Ok(inner)
            }
        }
    }
}

impl<W: ?Sized + Write> BufWriter<W> {
    /// Gets a reference to the underlying writer.
    #[inline]
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Gets a mutable reference to the underlying writer.
    #[inline]
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    /// Returns a reference to the internally buffered data.
    #[inline]
    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    /// Returns the number of bytes the internal buffer can hold without flushing.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn flush_buf(&mut self) -> Result<()> {
        let mut written = 0;
        let len = self.buf.len();
        let mut ret = Ok(());
        while written < len {
            match self.inner.write(&self.buf[written..]) {
                Ok(0) => {
                    ret = Err(Error::from_static_message(
                        ErrorKind::WriteZero,
                        "failed to write the buffered data",
                    ));
                    break;
                }
                Ok(n) => written += n,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => {
                    ret = Err(e);
                    break;
                }
            }
        }
        if written > 0 {
            self.buf.drain(..written);
        }
        ret
    }
}

impl<W: ?Sized + Write> Write for BufWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if self.buf.len() + buf.len() > self.capacity {
            self.flush_buf()?;
        }
        if buf.len() >= self.capacity {
            self.inner.write(buf)
        } else {
            self.buf.extend_from_slice(buf);
            Ok(buf.len())
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        if self.buf.len() + buf.len() > self.capacity {
            self.flush_buf()?;
        }
        if buf.len() >= self.capacity {
            self.inner.write_all(buf)
        } else {
            self.buf.extend_from_slice(buf);
            Ok(())
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.flush_buf().and_then(|()| self.inner.flush())
    }
}

impl<W: ?Sized + Write + Seek> Seek for BufWriter<W> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.flush_buf()?;
        self.inner.seek(pos)
    }
}

impl<W: ?Sized + Write> Drop for BufWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush_buf();
    }
}

/// Wraps a writer and buffers output to it, flushing whenever a newline (`0x0A`) is detected.
#[derive(Debug)]
pub struct LineWriter<W: ?Sized + Write> {
    inner: BufWriter<W>,
}

impl<W: Write> LineWriter<W> {
    /// Creates a new `LineWriter`.
    #[must_use]
    pub fn new(inner: W) -> LineWriter<W> {
        LineWriter::with_capacity(1024, inner)
    }

    /// Creates a new `LineWriter` with the specified buffer capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize, inner: W) -> LineWriter<W> {
        LineWriter {
            inner: BufWriter::with_capacity(capacity, inner),
        }
    }

    /// Unwraps this `LineWriter`, returning the underlying writer after flushing.
    pub fn into_inner(self) -> core_crate::result::Result<W, IntoInnerError<LineWriter<W>>> {
        self.inner
            .into_inner()
            .map_err(|err| IntoInnerError(LineWriter { inner: err.0 }, err.1))
    }
}

impl<W: ?Sized + Write> LineWriter<W> {
    /// Gets a reference to the underlying writer.
    #[inline]
    pub fn get_ref(&self) -> &W {
        self.inner.get_ref()
    }

    /// Gets a mutable reference to the underlying writer.
    #[inline]
    pub fn get_mut(&mut self) -> &mut W {
        self.inner.get_mut()
    }
}

impl<W: ?Sized + Write> Write for LineWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if let Some(newline_idx) = buf.iter().rposition(|&b| b == b'\n') {
            self.inner.write_all(&buf[..=newline_idx])?;
            self.inner.flush()?;
            self.inner.write_all(&buf[newline_idx + 1..])?;
            Ok(buf.len())
        } else {
            self.inner.write(buf)
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.inner.flush()
    }
}
