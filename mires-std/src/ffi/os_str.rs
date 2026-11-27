//! UTF-8-backed `OsStr` and `OsString` for JavaScript targets.

use crate::borrow::{Borrow, Cow, ToOwned};
use crate::boxed::Box;
use crate::cmp::Ordering;
use crate::convert::Infallible;
use crate::fmt;
use crate::hash::{Hash, Hasher};
use crate::ops::{Deref, DerefMut};
use crate::str::FromStr;
use crate::string::String;

/// Borrowed reference to an OS string (UTF-8-backed on JS targets).
#[repr(transparent)]
pub struct OsStr {
    inner: str,
}

/// Owned, mutable OS string (UTF-8-backed on JS targets).
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OsString {
    inner: String,
}

/// Helper struct for safely printing [`OsStr`] values with [`fmt::Display`].
pub struct Display<'a> {
    inner: &'a OsStr,
}

impl fmt::Debug for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner.inner, f)
    }
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner.inner, f)
    }
}

impl OsStr {
    /// Coerces into an `OsStr` slice.
    #[must_use]
    #[inline]
    pub fn new<S: AsRef<OsStr> + ?Sized>(s: &S) -> &OsStr {
        s.as_ref()
    }

    #[inline]
    pub(crate) fn from_str(s: &str) -> &OsStr {
        // SAFETY: `OsStr` is `#[repr(transparent)]` over `str`.
        unsafe { &*(s as *const str as *const OsStr) }
    }

    #[inline]
    fn from_str_mut(s: &mut str) -> &mut OsStr {
        // SAFETY: `OsStr` is `#[repr(transparent)]` over `str`.
        unsafe { &mut *(s as *mut str as *mut OsStr) }
    }

    /// Yields a `&str` slice if the `OsStr` is valid Unicode (always `Some` on JS).
    #[must_use]
    #[inline]
    pub fn to_str(&self) -> Option<&str> {
        Some(&self.inner)
    }

    /// Converts an `OsStr` to a [`Cow<str>`].
    #[must_use]
    #[inline]
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.inner)
    }

    /// Copies the slice into an owned [`OsString`].
    #[must_use]
    #[inline]
    pub fn to_os_string(&self) -> OsString {
        OsString {
            inner: String::from(&self.inner),
        }
    }

    /// Checks whether the `OsStr` is empty.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the length of this `OsStr` in bytes.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns the underlying byte representation.
    #[must_use]
    #[inline]
    pub fn as_encoded_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Converts a byte slice obtained from [`OsStr::as_encoded_bytes`] back into an `&OsStr`.
    ///
    /// # Safety
    /// The bytes must be valid UTF-8 (encoded by [`OsStr::as_encoded_bytes`]).
    #[must_use]
    #[inline]
    pub unsafe fn from_encoded_bytes_unchecked(bytes: &[u8]) -> &OsStr {
        // SAFETY: Caller guarantees `bytes` is valid UTF-8 from `as_encoded_bytes`.
        let s = unsafe { crate::str::from_utf8_unchecked(bytes) };
        OsStr::from_str(s)
    }

    /// Returns an object that implements [`fmt::Display`] for printing `OsStr`.
    #[must_use]
    #[inline]
    pub fn display(&self) -> Display<'_> {
        Display { inner: self }
    }

    /// Checks that two strings are an ASCII case-insensitive match.
    #[must_use]
    #[inline]
    pub fn eq_ignore_ascii_case<S: AsRef<OsStr>>(&self, other: S) -> bool {
        self.inner.eq_ignore_ascii_case(&other.as_ref().inner)
    }

    /// Converts this string to its ASCII lower case equivalent in-place.
    #[inline]
    pub fn make_ascii_lowercase(&mut self) {
        self.inner.make_ascii_lowercase();
    }

    /// Converts this string to its ASCII upper case equivalent in-place.
    #[inline]
    pub fn make_ascii_uppercase(&mut self) {
        self.inner.make_ascii_uppercase();
    }

    /// Returns a copy of this string where each ASCII uppercase letter is mapped to lowercase.
    #[must_use]
    #[inline]
    pub fn to_ascii_lowercase(&self) -> OsString {
        OsString {
            inner: self.inner.to_ascii_lowercase(),
        }
    }

    /// Returns a copy of this string where each ASCII lowercase letter is mapped to uppercase.
    #[must_use]
    #[inline]
    pub fn to_ascii_uppercase(&self) -> OsString {
        OsString {
            inner: self.inner.to_ascii_uppercase(),
        }
    }
}

impl OsString {
    /// Constructs a new empty `OsString`.
    #[must_use]
    #[inline]
    pub const fn new() -> OsString {
        OsString {
            inner: String::new(),
        }
    }

    /// Creates a new `OsString` with at least the specified capacity.
    #[must_use]
    #[inline]
    pub fn with_capacity(capacity: usize) -> OsString {
        OsString {
            inner: String::with_capacity(capacity),
        }
    }

    /// Converts to an [`OsStr`] slice.
    #[must_use]
    #[inline]
    pub fn as_os_str(&self) -> &OsStr {
        OsStr::from_str(&self.inner)
    }

    /// Converts the `OsString` into a [`String`] if it contains valid Unicode data.
    #[inline]
    pub fn into_string(self) -> Result<String, OsString> {
        Ok(self.inner)
    }

    /// Extends the string with the given `&OsStr` slice.
    #[inline]
    pub fn push<T: AsRef<OsStr>>(&mut self, s: T) {
        self.inner.push_str(&s.as_ref().inner);
    }

    /// Truncates the `OsString` to zero length.
    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Returns the capacity this `OsString` can hold without reallocating.
    #[must_use]
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserves capacity for at least `additional` more bytes.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Reserves the minimum capacity for at least `additional` more bytes.
    #[inline]
    pub fn reserve_exact(&mut self, additional: usize) {
        self.inner.reserve_exact(additional);
    }

    /// Shrinks the capacity of the `OsString` to match its length.
    #[inline]
    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Consumes the `OsString` and returns the underlying bytes.
    #[must_use]
    #[inline]
    pub fn into_encoded_bytes(self) -> crate::vec::Vec<u8> {
        self.inner.into_bytes()
    }

    /// Converts an owned `OsString` into a `Box<OsStr>`.
    #[must_use]
    #[inline]
    pub fn into_boxed_os_str(self) -> Box<OsStr> {
        let boxed: Box<str> = self.inner.into_boxed_str();
        let raw: *mut str = Box::into_raw(boxed);
        // SAFETY: `OsStr` is `#[repr(transparent)]` over `str`, so layout and metadata match.
        unsafe { Box::from_raw(raw as *mut OsStr) }
    }
}

impl Deref for OsString {
    type Target = OsStr;

    #[inline]
    fn deref(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl DerefMut for OsString {
    #[inline]
    fn deref_mut(&mut self) -> &mut OsStr {
        OsStr::from_str_mut(&mut self.inner)
    }
}

impl Borrow<OsStr> for OsString {
    #[inline]
    fn borrow(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl ToOwned for OsStr {
    type Owned = OsString;

    #[inline]
    fn to_owned(&self) -> OsString {
        self.to_os_string()
    }
}

impl AsRef<OsStr> for str {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        OsStr::from_str(self)
    }
}

impl AsRef<OsStr> for String {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        OsStr::from_str(self.as_str())
    }
}

impl AsRef<OsStr> for OsStr {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        self
    }
}

impl AsRef<OsStr> for OsString {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl From<String> for OsString {
    #[inline]
    fn from(s: String) -> OsString {
        OsString { inner: s }
    }
}

impl<T: ?Sized + AsRef<OsStr>> From<&T> for OsString {
    #[inline]
    fn from(s: &T) -> OsString {
        s.as_ref().to_os_string()
    }
}

impl FromStr for OsString {
    type Err = Infallible;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(OsString::from(s))
    }
}

impl fmt::Debug for OsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl fmt::Debug for OsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl PartialEq for OsStr {
    #[inline]
    fn eq(&self, other: &OsStr) -> bool {
        self.inner == other.inner
    }
}

impl Eq for OsStr {}

impl PartialOrd for OsStr {
    #[inline]
    fn partial_cmp(&self, other: &OsStr) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OsStr {
    #[inline]
    fn cmp(&self, other: &OsStr) -> Ordering {
        self.inner.cmp(&other.inner)
    }
}

impl Hash for OsStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl PartialEq<str> for OsStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        &self.inner == other
    }
}

impl PartialEq<OsStr> for str {
    #[inline]
    fn eq(&self, other: &OsStr) -> bool {
        self == &other.inner
    }
}

impl PartialEq<str> for OsString {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.inner == other
    }
}

impl PartialEq<&str> for OsString {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.inner == *other
    }
}

impl PartialEq<OsStr> for OsString {
    #[inline]
    fn eq(&self, other: &OsStr) -> bool {
        self.as_os_str() == other
    }
}

impl PartialEq<&OsStr> for OsString {
    #[inline]
    fn eq(&self, other: &&OsStr) -> bool {
        self.as_os_str() == *other
    }
}

impl<P: AsRef<OsStr>> Extend<P> for OsString {
    fn extend<T: IntoIterator<Item = P>>(&mut self, iter: T) {
        for s in iter {
            self.push(s);
        }
    }
}

impl<P: AsRef<OsStr>> FromIterator<P> for OsString {
    fn from_iter<T: IntoIterator<Item = P>>(iter: T) -> OsString {
        let mut s = OsString::new();
        s.extend(iter);
        s
    }
}
