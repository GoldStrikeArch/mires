//! Core I/O traits, error types, and in-memory streams (`Cursor`, `BufReader`,
//! `BufWriter`, `Stdout`, `Stderr`, `Stdin`).

mod buffered;
mod cursor;
pub(crate) mod stdio;

use crate::boxed::Box;
use crate::cmp;
use crate::error::Error as StdError;
use crate::fmt;
use crate::ops::{Deref, DerefMut};
use crate::str;
use crate::string::String;
use crate::vec::Vec;

pub use buffered::{BufReader, BufWriter, IntoInnerError, LineWriter};
pub use cursor::Cursor;
pub use stdio::{
    _eprint, _eprintln, _print, _println, stderr, stdin, stdout, Stderr, StderrLock, Stdin,
    StdinLock, Stdout, StdoutLock,
};

/// A specialized [`core::result::Result`] type for I/O operations.
pub type Result<T> = core_crate::result::Result<T, Error>;

/// A list specifying general categories of I/O error.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ErrorKind {
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    HostUnreachable,
    NetworkUnreachable,
    ConnectionAborted,
    NotConnected,
    AddrInUse,
    AddrNotAvailable,
    NetworkDown,
    BrokenPipe,
    AlreadyExists,
    WouldBlock,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    ReadOnlyFilesystem,
    StaleNetworkFileHandle,
    InvalidInput,
    InvalidData,
    TimedOut,
    WriteZero,
    StorageFull,
    NotSeekable,
    QuotaExceeded,
    FileTooLarge,
    ResourceBusy,
    ExecutableFileBusy,
    Deadlock,
    CrossesDevices,
    TooManyLinks,
    InvalidFilename,
    ArgumentListTooLong,
    Interrupted,
    Unsupported,
    UnexpectedEof,
    OutOfMemory,
    Other,
}

impl ErrorKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::NotFound => "entity not found",
            ErrorKind::PermissionDenied => "permission denied",
            ErrorKind::ConnectionRefused => "connection refused",
            ErrorKind::ConnectionReset => "connection reset",
            ErrorKind::HostUnreachable => "host unreachable",
            ErrorKind::NetworkUnreachable => "network unreachable",
            ErrorKind::ConnectionAborted => "connection aborted",
            ErrorKind::NotConnected => "not connected",
            ErrorKind::AddrInUse => "address in use",
            ErrorKind::AddrNotAvailable => "address not available",
            ErrorKind::NetworkDown => "network down",
            ErrorKind::BrokenPipe => "broken pipe",
            ErrorKind::AlreadyExists => "entity already exists",
            ErrorKind::WouldBlock => "operation would block",
            ErrorKind::NotADirectory => "not a directory",
            ErrorKind::IsADirectory => "is a directory",
            ErrorKind::DirectoryNotEmpty => "directory not empty",
            ErrorKind::ReadOnlyFilesystem => "read-only filesystem or storage medium",
            ErrorKind::StaleNetworkFileHandle => "stale network file handle",
            ErrorKind::InvalidInput => "invalid input parameter",
            ErrorKind::InvalidData => "invalid data",
            ErrorKind::TimedOut => "timed out",
            ErrorKind::WriteZero => "write zero",
            ErrorKind::StorageFull => "no storage space",
            ErrorKind::NotSeekable => "seek on unseekable file",
            ErrorKind::QuotaExceeded => "quota exceeded",
            ErrorKind::FileTooLarge => "file too large",
            ErrorKind::ResourceBusy => "resource busy",
            ErrorKind::ExecutableFileBusy => "executable file busy",
            ErrorKind::Deadlock => "deadlock",
            ErrorKind::CrossesDevices => "cross-device link or rename",
            ErrorKind::TooManyLinks => "too many links",
            ErrorKind::InvalidFilename => "invalid filename",
            ErrorKind::ArgumentListTooLong => "argument list too long",
            ErrorKind::Interrupted => "operation interrupted",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::UnexpectedEof => "unexpected end of file",
            ErrorKind::OutOfMemory => "out of memory",
            ErrorKind::Other => "other error",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

enum ErrorRepr {
    Simple(ErrorKind),
    SimpleMessage(ErrorKind, &'static str),
    Custom(ErrorKind, Box<dyn StdError + Send + Sync>),
    Os(i32),
}

/// The error type for I/O operations.
pub struct Error {
    repr: ErrorRepr,
}

impl Error {
    /// Creates a new I/O error from a known kind of error as well as an
    /// arbitrary error payload.
    pub fn new<E>(kind: ErrorKind, error: E) -> Error
    where
        E: Into<Box<dyn StdError + Send + Sync>>,
    {
        Error {
            repr: ErrorRepr::Custom(kind, error.into()),
        }
    }

    /// Creates a new I/O error with [`ErrorKind::Other`].
    pub fn other<E>(error: E) -> Error
    where
        E: Into<Box<dyn StdError + Send + Sync>>,
    {
        Error::new(ErrorKind::Other, error)
    }

    #[inline]
    pub(crate) const fn from_static_message(kind: ErrorKind, msg: &'static str) -> Error {
        Error {
            repr: ErrorRepr::SimpleMessage(kind, msg),
        }
    }

    /// Creates an error representing a raw OS error code.
    #[must_use]
    #[inline]
    pub fn from_raw_os_error(code: i32) -> Error {
        Error {
            repr: ErrorRepr::Os(code),
        }
    }

    /// Returns an error representing the last OS error (unsupported on JS).
    #[must_use]
    #[inline]
    pub fn last_os_error() -> Error {
        Error::from(ErrorKind::Unsupported)
    }

    /// Returns the OS error that this error represents, if any.
    #[must_use]
    #[inline]
    pub fn raw_os_error(&self) -> Option<i32> {
        match self.repr {
            ErrorRepr::Os(code) => Some(code),
            _ => None,
        }
    }

    /// Returns a reference to the inner boxed error, if any.
    #[must_use]
    #[inline]
    pub fn get_ref(&self) -> Option<&(dyn StdError + Send + Sync + 'static)> {
        match &self.repr {
            ErrorRepr::Custom(_, err) => Some(&**err),
            _ => None,
        }
    }

    /// Returns a mutable reference to the inner boxed error, if any.
    #[must_use]
    #[inline]
    pub fn get_mut(&mut self) -> Option<&mut (dyn StdError + Send + Sync + 'static)> {
        match &mut self.repr {
            ErrorRepr::Custom(_, err) => Some(&mut **err),
            _ => None,
        }
    }

    /// Consumes the `Error`, returning its inner boxed error if any.
    #[must_use = "`self` will be dropped if the result is not used"]
    #[inline]
    pub fn into_inner(self) -> Option<Box<dyn StdError + Send + Sync>> {
        match self.repr {
            ErrorRepr::Custom(_, err) => Some(err),
            _ => None,
        }
    }

    /// Attempts to downcast the custom boxed error to `E`.
    pub fn downcast<E>(self) -> core_crate::result::Result<E, Self>
    where
        E: StdError + Send + Sync + 'static,
    {
        match self.repr {
            ErrorRepr::Custom(kind, err) => match err.downcast::<E>() {
                Ok(boxed) => Ok(*boxed),
                Err(err) => Err(Error {
                    repr: ErrorRepr::Custom(kind, err),
                }),
            },
            other => Err(Error { repr: other }),
        }
    }

    /// Returns the corresponding [`ErrorKind`] for this error.
    #[must_use]
    #[inline]
    pub fn kind(&self) -> ErrorKind {
        match &self.repr {
            ErrorRepr::Simple(kind)
            | ErrorRepr::SimpleMessage(kind, _)
            | ErrorRepr::Custom(kind, _) => *kind,
            ErrorRepr::Os(_) => ErrorKind::Other,
        }
    }

    #[inline]
    pub(crate) const fn unsupported(msg: &'static str) -> Error {
        Error::from_static_message(ErrorKind::Unsupported, msg)
    }
}

impl From<ErrorKind> for Error {
    #[inline]
    fn from(kind: ErrorKind) -> Error {
        Error {
            repr: ErrorRepr::Simple(kind),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            ErrorRepr::Simple(kind) => f.debug_tuple("Kind").field(kind).finish(),
            ErrorRepr::SimpleMessage(kind, msg) => f
                .debug_struct("Error")
                .field("kind", kind)
                .field("message", msg)
                .finish(),
            ErrorRepr::Custom(kind, err) => f
                .debug_struct("Custom")
                .field("kind", kind)
                .field("error", err)
                .finish(),
            ErrorRepr::Os(code) => f.debug_struct("Os").field("code", code).finish(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            ErrorRepr::Simple(kind) => f.write_str(kind.as_str()),
            ErrorRepr::SimpleMessage(_, msg) => f.write_str(msg),
            ErrorRepr::Custom(_, err) => fmt::Display::fmt(err, f),
            ErrorRepr::Os(code) => write!(f, "os error {code}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.repr {
            ErrorRepr::Custom(_, err) => err.source(),
            _ => None,
        }
    }
}

/// A buffer type used with [`Write::write_vectored`].
#[derive(Clone, Copy)]
pub struct IoSlice<'a> {
    buf: &'a [u8],
}

impl<'a> IoSlice<'a> {
    /// Creates a new `IoSlice` wrapping a byte slice.
    #[must_use]
    #[inline]
    pub fn new(buf: &'a [u8]) -> IoSlice<'a> {
        IoSlice { buf }
    }

    /// Advances the internal slice by `n` bytes.
    #[inline]
    pub fn advance(&mut self, n: usize) {
        self.buf = &self.buf[n..];
    }
}

impl Deref for IoSlice<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        self.buf
    }
}

impl fmt::Debug for IoSlice<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.buf, f)
    }
}

/// A buffer type used with [`Read::read_vectored`].
pub struct IoSliceMut<'a> {
    buf: &'a mut [u8],
}

impl<'a> IoSliceMut<'a> {
    /// Creates a new `IoSliceMut` wrapping a mutable byte slice.
    #[must_use]
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> IoSliceMut<'a> {
        IoSliceMut { buf }
    }

    /// Advances the internal slice by `n` bytes.
    #[inline]
    pub fn advance(&mut self, n: usize) {
        let slice = crate::mem::take(&mut self.buf);
        self.buf = &mut slice[n..];
    }
}

impl Deref for IoSliceMut<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        self.buf
    }
}

impl DerefMut for IoSliceMut<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        self.buf
    }
}

impl fmt::Debug for IoSliceMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.buf, f)
    }
}

/// Enumeration of possible methods to seek within an I/O object.
#[derive(Copy, PartialEq, Eq, Clone, Debug)]
pub enum SeekFrom {
    /// Sets the offset to the provided number of bytes from the start.
    Start(u64),
    /// Sets the offset to the size of this object plus the specified number of bytes.
    End(i64),
    /// Sets the offset to the current position plus the specified number of bytes.
    Current(i64),
}

/// The `Read` trait allows for reading bytes from a source.
pub trait Read {
    /// Pull some bytes from this source into the specified buffer, returning how many were read.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Like `read`, except that it reads into a slice of buffers.
    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> Result<usize> {
        let buf = bufs
            .iter_mut()
            .find(|b| !b.is_empty())
            .map_or(&mut [][..], |b| &mut **b);
        self.read(buf)
    }

    /// Read all bytes until EOF in this source, placing them into `buf`.
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let start_len = buf.len();
        let mut chunk = [0u8; 1024];
        loop {
            match self.read(&mut chunk) {
                Ok(0) => return Ok(buf.len() - start_len),
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Read all bytes until EOF in this source, appending them to `buf`.
    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_to_end(&mut bytes)?;
        let s = str::from_utf8(&bytes).map_err(|_| {
            Error::from_static_message(ErrorKind::InvalidData, "stream did not contain valid UTF-8")
        })?;
        buf.push_str(s);
        Ok(n)
    }

    /// Read the exact number of bytes required to fill `buf`.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = buf;
                    buf = &mut tmp[n..];
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if !buf.is_empty() {
            Err(Error::from_static_message(
                ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ))
        } else {
            Ok(())
        }
    }

    /// Creates a "by reference" adaptor for this instance of `Read`.
    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }

    /// Transforms this `Read` instance to an `Iterator` over its bytes.
    fn bytes(self) -> Bytes<Self>
    where
        Self: Sized,
    {
        Bytes { inner: self }
    }

    /// Creates an adapter which will chain this stream with another.
    fn chain<R: Read>(self, next: R) -> Chain<Self, R>
    where
        Self: Sized,
    {
        Chain {
            first: self,
            second: next,
            done_first: false,
        }
    }

    /// Creates an adapter which will read at most `limit` bytes from it.
    fn take(self, limit: u64) -> Take<Self>
    where
        Self: Sized,
    {
        Take { inner: self, limit }
    }
}

/// A trait for objects which are byte-oriented sinks.
pub trait Write {
    /// Write a buffer into this writer, returning how many bytes were written.
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    /// Flush this output stream, ensuring all intermediately buffered contents reach their destination.
    fn flush(&mut self) -> Result<()>;

    /// Like `write`, except that it writes from a slice of buffers.
    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> Result<usize> {
        let buf = bufs
            .iter()
            .find(|b| !b.is_empty())
            .map_or(&[][..], |b| &**b);
        self.write(buf)
    }

    /// Attempts to write an entire buffer into this writer.
    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => {
                    return Err(Error::from_static_message(
                        ErrorKind::WriteZero,
                        "failed to write whole buffer",
                    ));
                }
                Ok(n) => buf = &buf[n..],
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Writes a formatted string into this writer.
    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> Result<()> {
        struct Adapter<'a, T: ?Sized + 'a> {
            inner: &'a mut T,
            error: Result<()>,
        }

        impl<T: Write + ?Sized> fmt::Write for Adapter<'_, T> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                match self.inner.write_all(s.as_bytes()) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        self.error = Err(e);
                        Err(fmt::Error)
                    }
                }
            }
        }

        if let Some(s) = fmt.as_str() {
            return self.write_all(s.as_bytes());
        }
        let mut output = Adapter {
            inner: self,
            error: Ok(()),
        };
        match fmt::write(&mut output, fmt) {
            Ok(()) => Ok(()),
            Err(_) => {
                if output.error.is_err() {
                    output.error
                } else {
                    Err(Error::from_static_message(
                        ErrorKind::Other,
                        "formatter error",
                    ))
                }
            }
        }
    }

    /// Creates a "by reference" adapter for this instance of `Write`.
    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }
}

/// The `Seek` trait provides a cursor which can be moved within a stream of bytes.
pub trait Seek {
    /// Seek to an offset, in bytes, in a stream.
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;

    /// Rewind to the beginning of a stream.
    fn rewind(&mut self) -> Result<()> {
        self.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    /// Returns the current seek position from the start of the stream.
    fn stream_position(&mut self) -> Result<u64> {
        self.seek(SeekFrom::Current(0))
    }

    /// Seeks relative to the current position.
    fn seek_relative(&mut self, offset: i64) -> Result<()> {
        self.seek(SeekFrom::Current(offset))?;
        Ok(())
    }
}

/// A `BufRead` is a type of `Read`er which has an internal buffer.
pub trait BufRead: Read {
    /// Returns the contents of the internal buffer, filling it if empty.
    fn fill_buf(&mut self) -> Result<&[u8]>;

    /// Tells this buffer that `amt` bytes have been consumed from the buffer.
    fn consume(&mut self, amt: usize);

    /// Read all bytes into `buf` until the delimiter `byte` or EOF is reached.
    fn read_until(&mut self, byte: u8, buf: &mut Vec<u8>) -> Result<usize> {
        let mut read = 0;
        loop {
            let (done, used) = {
                let available = match self.fill_buf() {
                    Ok(n) => n,
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                if let Some(i) = available.iter().position(|&b| b == byte) {
                    buf.extend_from_slice(&available[..=i]);
                    (true, i + 1)
                } else {
                    buf.extend_from_slice(available);
                    (false, available.len())
                }
            };
            self.consume(used);
            read += used;
            if done || used == 0 {
                return Ok(read);
            }
        }
    }

    /// Skip all bytes until the delimiter `byte` or EOF is reached.
    fn skip_until(&mut self, byte: u8) -> Result<usize> {
        let mut read = 0;
        loop {
            let (done, used) = {
                let available = match self.fill_buf() {
                    Ok(n) => n,
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                if let Some(i) = available.iter().position(|&b| b == byte) {
                    (true, i + 1)
                } else {
                    (false, available.len())
                }
            };
            self.consume(used);
            read += used;
            if done || used == 0 {
                return Ok(read);
            }
        }
    }

    /// Read all bytes until a newline (`0xA`) is reached, and append them to `buf`.
    fn read_line(&mut self, buf: &mut String) -> Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_until(b'\n', &mut bytes)?;
        let s = str::from_utf8(&bytes).map_err(|_| {
            Error::from_static_message(ErrorKind::InvalidData, "stream did not contain valid UTF-8")
        })?;
        buf.push_str(s);
        Ok(n)
    }

    /// Returns an iterator over the contents of this reader split on the byte `byte`.
    fn split(self, byte: u8) -> Split<Self>
    where
        Self: Sized,
    {
        Split {
            buf: self,
            delim: byte,
        }
    }

    /// Returns an iterator over the lines of this reader.
    fn lines(self) -> Lines<Self>
    where
        Self: Sized,
    {
        Lines { buf: self }
    }
}

/// Copies the entire contents of a reader into a writer.
pub fn copy<R: ?Sized, W: ?Sized>(reader: &mut R, writer: &mut W) -> Result<u64>
where
    R: Read,
    W: Write,
{
    let mut buf = [0u8; 4096];
    let mut written = 0u64;
    loop {
        let len = match reader.read(&mut buf) {
            Ok(0) => return Ok(written),
            Ok(len) => len,
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&buf[..len])?;
        written += len as u64;
    }
}

/// A reader which is always at EOF.
#[derive(Clone, Copy, Debug, Default)]
pub struct Empty;

/// Constructs a new handle to an empty reader.
#[must_use]
#[inline]
pub const fn empty() -> Empty {
    Empty
}

impl Read for Empty {
    #[inline]
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl BufRead for Empty {
    #[inline]
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(&[])
    }

    #[inline]
    fn consume(&mut self, _amt: usize) {}
}

impl Seek for Empty {
    #[inline]
    fn seek(&mut self, _pos: SeekFrom) -> Result<u64> {
        Ok(0)
    }
}

/// A writer which will move data into the void.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sink;

/// Creates an instance of a writer which will successfully consume all data.
#[must_use]
#[inline]
pub const fn sink() -> Sink {
    Sink
}

impl Write for Sink {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A reader which yields one byte over and over and over and over.
#[derive(Clone, Copy, Debug)]
pub struct Repeat {
    byte: u8,
}

/// Creates an instance of a reader that infinitely repeats one byte.
#[must_use]
#[inline]
pub const fn repeat(byte: u8) -> Repeat {
    Repeat { byte }
}

impl Read for Repeat {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        buf.fill(self.byte);
        Ok(buf.len())
    }
}

/// An iterator over `u8` values of a reader.
#[derive(Debug)]
pub struct Bytes<R> {
    inner: R,
}

impl<R: Read> Iterator for Bytes<R> {
    type Item = Result<u8>;

    fn next(&mut self) -> Option<Result<u8>> {
        let mut byte = 0u8;
        loop {
            return match self.inner.read(crate::slice::from_mut(&mut byte)) {
                Ok(0) => None,
                Ok(_) => Some(Ok(byte)),
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => Some(Err(e)),
            };
        }
    }
}

/// Adapter to chain together two readers.
#[derive(Debug)]
pub struct Chain<T, U> {
    first: T,
    second: U,
    done_first: bool,
}

impl<T, U> Chain<T, U> {
    /// Consumes the `Chain`, returning the wrapped readers.
    pub fn into_inner(self) -> (T, U) {
        (self.first, self.second)
    }

    /// Gets references to the underlying readers.
    pub fn get_ref(&self) -> (&T, &U) {
        (&self.first, &self.second)
    }

    /// Gets mutable references to the underlying readers.
    pub fn get_mut(&mut self) -> (&mut T, &mut U) {
        (&mut self.first, &mut self.second)
    }
}

impl<T: Read, U: Read> Read for Chain<T, U> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if !self.done_first {
            match self.first.read(buf)? {
                0 if !buf.is_empty() => self.done_first = true,
                n => return Ok(n),
            }
        }
        self.second.read(buf)
    }
}

/// Reader adapter which limits the bytes read from an underlying reader.
#[derive(Debug)]
pub struct Take<T> {
    inner: T,
    limit: u64,
}

impl<T> Take<T> {
    /// Returns the number of bytes that can be read before this instance returns EOF.
    #[inline]
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Sets the number of bytes that can be read before this instance returns EOF.
    #[inline]
    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    /// Consumes the `Take`, returning the wrapped reader.
    #[inline]
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Gets a reference to the underlying reader.
    #[inline]
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Gets a mutable reference to the underlying reader.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: Read> Read for Take<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.limit == 0 {
            return Ok(0);
        }
        let max = cmp::min(buf.len() as u64, self.limit) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        self.limit -= n as u64;
        Ok(n)
    }
}

/// An iterator over the contents of an instance of [`BufRead`] split on a particular byte.
#[derive(Debug)]
pub struct Split<B> {
    buf: B,
    delim: u8,
}

impl<B: BufRead> Iterator for Split<B> {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Result<Vec<u8>>> {
        let mut buf = Vec::new();
        match self.buf.read_until(self.delim, &mut buf) {
            Ok(0) => None,
            Ok(_) => {
                if buf.last() == Some(&self.delim) {
                    buf.pop();
                }
                Some(Ok(buf))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

/// An iterator over the lines of an instance of [`BufRead`].
#[derive(Debug)]
pub struct Lines<B> {
    buf: B,
}

impl<B: BufRead> Iterator for Lines<B> {
    type Item = Result<String>;

    fn next(&mut self) -> Option<Result<String>> {
        let mut buf = String::new();
        match self.buf.read_line(&mut buf) {
            Ok(0) => None,
            Ok(_) => {
                if buf.ends_with('\n') {
                    buf.pop();
                    if buf.ends_with('\r') {
                        buf.pop();
                    }
                }
                Some(Ok(buf))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

// Forwarding and slice/Vec implementations.

impl<R: Read + ?Sized> Read for &mut R {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }

    #[inline]
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        (**self).read_to_end(buf)
    }

    #[inline]
    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        (**self).read_to_string(buf)
    }

    #[inline]
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        (**self).read_exact(buf)
    }
}

impl<R: Read + ?Sized> Read for Box<R> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }
}

impl<B: BufRead + ?Sized> BufRead for &mut B {
    #[inline]
    fn fill_buf(&mut self) -> Result<&[u8]> {
        (**self).fill_buf()
    }

    #[inline]
    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}

impl<B: BufRead + ?Sized> BufRead for Box<B> {
    #[inline]
    fn fill_buf(&mut self) -> Result<&[u8]> {
        (**self).fill_buf()
    }

    #[inline]
    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}

impl<W: Write + ?Sized> Write for &mut W {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        (**self).write(buf)
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        (**self).flush()
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        (**self).write_all(buf)
    }

    #[inline]
    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> Result<()> {
        (**self).write_fmt(fmt)
    }
}

impl<W: Write + ?Sized> Write for Box<W> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        (**self).write(buf)
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        (**self).flush()
    }
}

impl<S: Seek + ?Sized> Seek for &mut S {
    #[inline]
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        (**self).seek(pos)
    }
}

impl<S: Seek + ?Sized> Seek for Box<S> {
    #[inline]
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        (**self).seek(pos)
    }
}

impl Read for &[u8] {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let amt = cmp::min(buf.len(), self.len());
        let (a, b) = self.split_at(amt);
        buf[..amt].copy_from_slice(a);
        *self = b;
        Ok(amt)
    }

    #[inline]
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        if buf.len() > self.len() {
            return Err(Error::from_static_message(
                ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ));
        }
        let (a, b) = self.split_at(buf.len());
        buf.copy_from_slice(a);
        *self = b;
        Ok(())
    }

    #[inline]
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        buf.extend_from_slice(self);
        let len = self.len();
        *self = &self[len..];
        Ok(len)
    }
}

impl BufRead for &[u8] {
    #[inline]
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(*self)
    }

    #[inline]
    fn consume(&mut self, amt: usize) {
        *self = &self[amt..];
    }
}

impl Write for &mut [u8] {
    #[inline]
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        let amt = cmp::min(data.len(), self.len());
        let (a, b) = crate::mem::take(self).split_at_mut(amt);
        a.copy_from_slice(&data[..amt]);
        *self = b;
        Ok(amt)
    }

    #[inline]
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        if self.write(data)? == data.len() {
            Ok(())
        } else {
            Err(Error::from_static_message(
                ErrorKind::WriteZero,
                "failed to write whole buffer",
            ))
        }
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for Vec<u8> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.extend_from_slice(buf);
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The I/O prelude (`Read`, `Write`, `BufRead`, `Seek`).
pub mod prelude {
    pub use super::{BufRead, Read, Seek, Write};
}
